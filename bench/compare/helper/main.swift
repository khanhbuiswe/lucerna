// bench-helper: the system-API half of bench/compare.
//
// Python orchestrates; this tool captures a window with ScreenCaptureKit to
// find when its frames change, and posts keyboard/scroll input with CGEvent.
// Every result is one JSON line on stdout. Errors go to stderr with a
// non-zero exit code and nothing on stdout:
//   1 = usage / runtime error, 2 = timeout, 3 = missing permission.
//
// Clock: every timestamp is nanoseconds of CLOCK_UPTIME_RAW, the clock behind
// mach_absolute_time() and Python's time.monotonic_ns() on macOS. Capture
// frames carry SCStreamFrameInfo.displayTime, which is in mach absolute time
// units; it is converted to ns with mach_timebase_info, so frame times and
// Python's t0 share one time base.

import AppKit
import CoreGraphics
import CoreMedia
import CoreVideo
import Foundation
import ImageIO
import ScreenCaptureKit

// MARK: - Output and errors

setvbuf(stdout, nil, _IOLBF, 0)

func emit(_ object: [String: Any]) {
    let data = try! JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
    print(String(data: data, encoding: .utf8)!)
}

func fail(_ message: String, code: Int32) -> Never {
    FileHandle.standardError.write(("bench-helper: " + message + "\n").data(using: .utf8)!)
    exit(code)
}

let usage = """
usage: bench-helper <command> [options]
  now
  check
  watch  --pid <pid> --since <t0_ns> --until coverage:<0..1>|change
         [--timeout <s>] [--baseline <png>] [--background RRGGBB]
  scroll --pid <pid> --lines <n> --rate <events/s> [--timeout <s>]
  type   --pid <pid> --text <s> [--interval-ms <ms>]
  key    --pid <pid> --combo <cmd+p>
"""

// MARK: - Clock

func nowNs() -> UInt64 { clock_gettime_nsec_np(CLOCK_UPTIME_RAW) }

let timebase: mach_timebase_info_data_t = {
    var info = mach_timebase_info_data_t()
    mach_timebase_info(&info)
    return info
}()

func machToNs(_ t: UInt64) -> UInt64 {
    t / UInt64(timebase.denom) * UInt64(timebase.numer)
        + (t % UInt64(timebase.denom)) * UInt64(timebase.numer) / UInt64(timebase.denom)
}

// MARK: - Arguments

let argv = Array(CommandLine.arguments.dropFirst())
guard let command = argv.first else { fail("missing command\n" + usage, code: 1) }

func option(_ name: String) -> String? {
    guard let i = argv.firstIndex(of: "--" + name), i + 1 < argv.count else { return nil }
    return argv[i + 1]
}

func required(_ name: String) -> String {
    guard let v = option(name) else { fail("missing --\(name)\n" + usage, code: 1) }
    return v
}

func intOption(_ name: String, default def: Int? = nil) -> Int {
    guard let raw = option(name) else {
        if let def { return def }
        fail("missing --\(name)\n" + usage, code: 1)
    }
    guard let v = Int(raw) else { fail("--\(name) must be an integer", code: 1) }
    return v
}

func doubleOption(_ name: String, default def: Double) -> Double {
    guard let raw = option(name) else { return def }
    guard let v = Double(raw) else { fail("--\(name) must be a number", code: 1) }
    return v
}

// MARK: - Permissions

/// Preflight only: never prompts (no Request variants). TCC attributes the
/// grant to the responsible app (the terminal or app that launched us).
func permissions() -> (screen: Bool, ax: Bool) {
    if ProcessInfo.processInfo.environment["BENCH_HELPER_FAKE_DENY"] == "1" {
        return (false, false)
    }
    return (CGPreflightScreenCaptureAccess(), AXIsProcessTrusted())
}

/// Every measuring command needs both grants; refuse rather than measure
/// nothing and report zeros.
func requirePermissions() {
    let p = permissions()
    var missing: [String] = []
    if !p.screen { missing.append("Screen Recording") }
    if !p.ax { missing.append("Accessibility") }
    if !missing.isEmpty {
        fail("missing permission: \(missing.joined(separator: ", ")) — grant it to the app "
             + "that runs the benchmark (System Settings > Privacy & Security)", code: 3)
    }
}

// MARK: - Window lookup

/// Largest on-screen window owned by `pid`, polled until `deadline` because a
/// freshly launched app may not have a window yet.
func findWindow(pid: pid_t, deadline: UInt64) async -> SCWindow {
    while true {
        if let content = try? await SCShareableContent.excludingDesktopWindows(
            false, onScreenWindowsOnly: true) {
            let mine = content.windows.filter {
                $0.owningApplication?.processID == pid && $0.windowLayer == 0
                    && $0.frame.width >= 100 && $0.frame.height >= 100
            }
            if let best = mine.max(by: {
                $0.frame.width * $0.frame.height < $1.frame.width * $1.frame.height
            }) {
                return best
            }
        }
        if nowNs() >= deadline { fail("timeout: no on-screen window for pid \(pid)", code: 2) }
        try? await Task.sleep(nanoseconds: 50_000_000)
    }
}

/// Maximum refresh rate of the screen holding the window's center.
func refreshHz(for window: SCWindow) -> Int {
    // SCWindow.frame uses top-left global coordinates; NSScreen uses
    // bottom-left with the primary screen's height as reference.
    let primaryHeight = NSScreen.screens.first?.frame.height ?? 0
    let center = CGPoint(x: window.frame.midX, y: primaryHeight - window.frame.midY)
    let screen = NSScreen.screens.first { $0.frame.contains(center) } ?? NSScreen.main
    let hz = screen?.maximumFramesPerSecond ?? 60
    return hz > 0 ? hz : 60
}

// MARK: - Capture

struct Frame {
    let tNs: UInt64
    let base: UnsafePointer<UInt8>
    let width: Int
    let height: Int
    let bytesPerRow: Int

    func pixel(_ x: Int, _ y: Int) -> UnsafePointer<UInt8> { base + y * bytesPerRow + x * 4 }
}

/// Streams one window at the display refresh rate and hands every frame with
/// new content (SCFrameStatus.complete; idle repeats are skipped) to
/// `onFrame` on the capture queue. `onFrame` returns true to stop.
final class Capture: NSObject, SCStreamOutput, SCStreamDelegate, @unchecked Sendable {
    private let queue = DispatchQueue(label: "bench-helper.capture", qos: .userInteractive)
    private let lock = NSLock()
    private var continuation: CheckedContinuation<Bool, Never>?
    private var finished = false
    private var stream: SCStream?
    private let onFrame: (Frame) -> Bool
    let width: Int
    let height: Int
    let hz: Int

    init(window: SCWindow, hz: Int, onFrame: @escaping (Frame) -> Bool) {
        self.onFrame = onFrame
        self.hz = hz
        // Capture at point size (scale 1): plenty for pixel statistics and
        // cheap enough to analyse every frame at 120 Hz.
        width = max(1, Int(window.frame.width))
        height = max(1, Int(window.frame.height))
        super.init()
        let config = SCStreamConfiguration()
        config.width = width
        config.height = height
        config.pixelFormat = kCVPixelFormatType_32BGRA
        config.minimumFrameInterval = CMTime(value: 1, timescale: CMTimeScale(hz))
        config.queueDepth = 6
        config.showsCursor = false
        let filter = SCContentFilter(desktopIndependentWindow: window)
        stream = SCStream(filter: filter, configuration: config, delegate: self)
    }

    /// Starts capturing; resolves true when `onFrame` asked to stop, false
    /// at `deadline`.
    func start(deadline: UInt64) async -> Bool {
        guard let stream else { fail("could not create capture stream", code: 1) }
        do {
            try stream.addStreamOutput(self, type: .screen, sampleHandlerQueue: queue)
            try await stream.startCapture()
        } catch {
            fail("capture failed to start: \(error.localizedDescription)", code: 1)
        }
        FileHandle.standardError.write("ready\n".data(using: .utf8)!)
        let delay = deadline > nowNs() ? deadline - nowNs() : 0
        queue.asyncAfter(deadline: .now() + .nanoseconds(Int(min(delay, UInt64(Int.max))))) {
            [weak self] in self?.finish(false)
        }
        let result = await withCheckedContinuation { (c: CheckedContinuation<Bool, Never>) in
            lock.lock()
            if finished {
                lock.unlock()
                c.resume(returning: true)
            } else {
                continuation = c
                lock.unlock()
            }
        }
        try? await stream.stopCapture()
        return result
    }

    private func finish(_ value: Bool) {
        lock.lock()
        if finished { lock.unlock(); return }
        finished = true
        let c = continuation
        continuation = nil
        lock.unlock()
        c?.resume(returning: value)
    }

    func stream(_ stream: SCStream, didOutputSampleBuffer sb: CMSampleBuffer,
                of type: SCStreamOutputType) {
        guard type == .screen, sb.isValid else { return }
        lock.lock()
        let done = finished
        lock.unlock()
        if done { return }
        guard let attachments = CMSampleBufferGetSampleAttachmentsArray(
                sb, createIfNecessary: false) as? [[SCStreamFrameInfo: Any]],
              let info = attachments.first,
              let rawStatus = info[.status] as? Int,
              SCFrameStatus(rawValue: rawStatus) == .complete,
              let pixels = CMSampleBufferGetImageBuffer(sb) else { return }
        // displayTime is mach absolute time; fall back to the presentation
        // timestamp (host clock, also mach based) if it is missing.
        let tNs: UInt64
        if let display = info[.displayTime] as? UInt64 {
            tNs = machToNs(display)
        } else {
            tNs = UInt64(max(0, CMSampleBufferGetPresentationTimeStamp(sb).seconds * 1e9))
        }
        CVPixelBufferLockBaseAddress(pixels, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(pixels, .readOnly) }
        guard let base = CVPixelBufferGetBaseAddress(pixels) else { return }
        let frame = Frame(tNs: tNs,
                          base: UnsafePointer(base.assumingMemoryBound(to: UInt8.self)),
                          width: CVPixelBufferGetWidth(pixels),
                          height: CVPixelBufferGetHeight(pixels),
                          bytesPerRow: CVPixelBufferGetBytesPerRow(pixels))
        if onFrame(frame) { finish(true) }
    }

    func stream(_ stream: SCStream, didStopWithError error: Error) {
        fail("capture stopped: \(error.localizedDescription)", code: 1)
    }
}

// MARK: - Pixel analysis

/// Pixels are sampled on a 2×2 grid: a quarter of the work, same statistics.
let sampleStep = 2
/// Per-channel tolerance: absorbs compression-free but subpixel/AA noise.
let channelTolerance = 24

@inline(__always)
func differs(_ a: UnsafePointer<UInt8>, _ b: UnsafePointer<UInt8>) -> Bool {
    abs(Int(a[0]) - Int(b[0])) > channelTolerance
        || abs(Int(a[1]) - Int(b[1])) > channelTolerance
        || abs(Int(a[2]) - Int(b[2])) > channelTolerance
}

/// Owned BGRA copy (reference frames outlive the capture buffer).
struct Bitmap {
    var bytes: [UInt8]
    let width: Int
    let height: Int

    init(_ f: Frame) {
        width = f.width
        height = f.height
        bytes = [UInt8](repeating: 0, count: width * height * 4)
        bytes.withUnsafeMutableBufferPointer { dst in
            for y in 0..<height {
                memcpy(dst.baseAddress! + y * width * 4, f.base + y * f.bytesPerRow, width * 4)
            }
        }
    }

    /// Loads a PNG and scales it to the capture size.
    init?(png path: String, width: Int, height: Int) {
        guard let src = CGImageSourceCreateWithURL(URL(fileURLWithPath: path) as CFURL, nil),
              let image = CGImageSourceCreateImageAtIndex(src, 0, nil) else { return nil }
        self.width = width
        self.height = height
        bytes = [UInt8](repeating: 0, count: width * height * 4)
        let ok = bytes.withUnsafeMutableBytes { buf -> Bool in
            guard let ctx = CGContext(
                data: buf.baseAddress, width: width, height: height, bitsPerComponent: 8,
                bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue
                    | CGBitmapInfo.byteOrder32Little.rawValue) else { return false }
            ctx.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
            return true
        }
        if !ok { return nil }
    }
}

/// Fraction of sampled pixels that differ from `ref` (sizes must match;
/// a size change counts as a full change).
func changedFraction(_ f: Frame, _ ref: Bitmap) -> Double {
    guard f.width == ref.width, f.height == ref.height else { return 1 }
    var changed = 0, total = 0
    ref.bytes.withUnsafeBufferPointer { r in
        for y in stride(from: 0, to: f.height, by: sampleStep) {
            for x in stride(from: 0, to: f.width, by: sampleStep) {
                total += 1
                if differs(f.pixel(x, y), r.baseAddress! + (y * ref.width + x) * 4) {
                    changed += 1
                }
            }
        }
    }
    return total == 0 ? 0 : Double(changed) / Double(total)
}

/// Most common colour of the whole frame (5 bits per channel buckets),
/// returned as BGR bytes.
func dominantColor(_ f: Frame) -> [UInt8] {
    var counts = [Int](repeating: 0, count: 1 << 15)
    for y in stride(from: 0, to: f.height, by: sampleStep) {
        for x in stride(from: 0, to: f.width, by: sampleStep) {
            let p = f.pixel(x, y)
            counts[(Int(p[0]) >> 3) | (Int(p[1]) >> 3) << 5 | (Int(p[2]) >> 3) << 10] += 1
        }
    }
    let key = counts.indices.max { counts[$0] < counts[$1] }!
    return [UInt8((key & 31) << 3 | 4), UInt8((key >> 5 & 31) << 3 | 4),
            UInt8((key >> 10 & 31) << 3 | 4)]
}

/// Fraction of the central 60%×60% region whose pixels differ from `bg`.
func coverage(_ f: Frame, background bg: [UInt8]) -> Double {
    let x0 = f.width / 5, x1 = f.width - f.width / 5
    let y0 = f.height / 5, y1 = f.height - f.height / 5
    var hit = 0, total = 0
    bg.withUnsafeBufferPointer { b in
        for y in stride(from: y0, to: y1, by: sampleStep) {
            for x in stride(from: x0, to: x1, by: sampleStep) {
                total += 1
                if differs(f.pixel(x, y), b.baseAddress!) { hit += 1 }
            }
        }
    }
    return total == 0 ? 0 : Double(hit) / Double(total)
}

// MARK: - watch

/// Prints the time of the first frame at or after --since that meets
/// --until:
///   coverage:x  the central region differs from the background colour on at
///               least x of its pixels. Background = --background RRGGBB, else
///               the dominant colour of the first captured frame (a new window
///               paints its clear colour before content).
///   change      more than 0.5% of pixels differ from the reference frame. The
///               reference is --baseline <png>, else the last frame before
///               --since, else the first frame captured. The 0.5% floor
///               ignores a blinking caret. Start watch before triggering the
///               change: it writes "ready" to stderr once capture runs.
func watch() async {
    let pid = pid_t(intOption("pid"))
    let since = UInt64(max(0, intOption("since")))
    let until = required("until")
    let timeout = doubleOption("timeout", default: 60)
    let deadline = nowNs() + UInt64(timeout * 1e9)

    enum Mode { case coverage(Double), change }
    let mode: Mode
    if until == "change" {
        mode = .change
    } else if until.hasPrefix("coverage:"), let x = Double(until.dropFirst(9)), x >= 0, x <= 1 {
        mode = .coverage(x)
    } else {
        fail("--until must be coverage:<0..1> or change", code: 1)
    }
    var background: [UInt8]? = nil
    if let hex = option("background") {
        guard hex.count == 6, let v = UInt32(hex, radix: 16) else {
            fail("--background must be RRGGBB", code: 1)
        }
        background = [UInt8(v & 0xff), UInt8(v >> 8 & 0xff), UInt8(v >> 16)]
    }

    let window = await findWindow(pid: pid, deadline: deadline)
    let hz = refreshHz(for: window)
    let w = max(1, Int(window.frame.width)), h = max(1, Int(window.frame.height))
    var reference: Bitmap? = nil
    let hasBaseline = option("baseline") != nil
    if let png = option("baseline") {
        guard let b = Bitmap(png: png, width: w, height: h) else {
            fail("cannot read baseline \(png)", code: 1)
        }
        reference = b
    }

    var frames = 0
    var hitNs: UInt64 = 0
    let capture = Capture(window: window, hz: hz) { f in
        frames += 1
        switch mode {
        case .coverage(let x):
            if background == nil { background = dominantColor(f) }
            if f.tNs >= since && coverage(f, background: background!) >= x {
                hitNs = f.tNs
                return true
            }
        case .change:
            if f.tNs < since {
                if !hasBaseline { reference = Bitmap(f) }
                return false
            }
            if reference == nil {
                reference = Bitmap(f)
                return false
            }
            if changedFraction(f, reference!) > 0.005 {
                hitNs = f.tNs
                return true
            }
        }
        return false
    }
    guard await capture.start(deadline: deadline) else {
        fail("timeout after \(timeout) s waiting for \(until)", code: 2)
    }
    emit(["t_ns": hitNs, "frames": frames])
}

// MARK: - Input

/// US ANSI virtual key codes. Apps that read the key code (not only the
/// characters) need the real one.
let keyCodes: [Character: CGKeyCode] = [
    "a": 0, "s": 1, "d": 2, "f": 3, "h": 4, "g": 5, "z": 6, "x": 7, "c": 8, "v": 9,
    "b": 11, "q": 12, "w": 13, "e": 14, "r": 15, "y": 16, "t": 17, "1": 18, "2": 19,
    "3": 20, "4": 21, "6": 22, "5": 23, "=": 24, "9": 25, "7": 26, "-": 27, "8": 28,
    "0": 29, "]": 30, "o": 31, "u": 32, "[": 33, "i": 34, "p": 35, "l": 37, "j": 38,
    "'": 39, "k": 40, ";": 41, "\\": 42, ",": 43, "/": 44, "n": 45, "m": 46, ".": 47,
    " ": 49, "`": 50,
]
let namedKeys: [String: CGKeyCode] = [
    "return": 36, "enter": 36, "tab": 48, "space": 49, "delete": 51, "backspace": 51,
    "escape": 53, "esc": 53, "left": 123, "right": 124, "down": 125, "up": 126,
]

func postKey(pid: pid_t, code: CGKeyCode, text: String?, flags: CGEventFlags) {
    for down in [true, false] {
        guard let e = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: down) else {
            fail("cannot create key event", code: 1)
        }
        e.flags = flags
        if let text {
            let units = Array(text.utf16)
            e.keyboardSetUnicodeString(stringLength: units.count, unicodeString: units)
        }
        e.postToPid(pid)
    }
}

/// Posts each character to the pid (it should be frontmost) and prints the
/// time each key-down was sent.
func typeText() {
    let pid = pid_t(intOption("pid"))
    let text = required("text")
    let interval = UInt32(max(0, intOption("interval-ms", default: 100)) * 1000)
    var times: [UInt64] = []
    for (i, ch) in text.enumerated() {
        if i > 0 { usleep(interval) }
        let lower = Character(ch.lowercased())
        let code = keyCodes[lower] ?? 0
        let flags: CGEventFlags = ch.isUppercase ? .maskShift : []
        times.append(nowNs())
        postKey(pid: pid, code: code, text: String(ch), flags: flags)
    }
    emit(["t_ns": times])
}

/// One key combination such as cmd+p or cmd+shift+p.
func keyCombo() {
    let pid = pid_t(intOption("pid"))
    let parts = required("combo").lowercased().split(separator: "+").map(String.init)
    guard let keyName = parts.last else { fail("empty --combo", code: 1) }
    var flags: CGEventFlags = []
    for m in parts.dropLast() {
        switch m {
        case "cmd", "command": flags.insert(.maskCommand)
        case "shift": flags.insert(.maskShift)
        case "alt", "opt", "option": flags.insert(.maskAlternate)
        case "ctrl", "control": flags.insert(.maskControl)
        default: fail("unknown modifier \(m)", code: 1)
        }
    }
    let code: CGKeyCode
    var text: String? = nil
    if let named = namedKeys[keyName] {
        code = named
    } else if keyName.count == 1, let c = keyCodes[Character(keyName)] {
        code = c
        text = keyName
    } else {
        fail("unknown key \(keyName)", code: 1)
    }
    let t = nowNs()
    postKey(pid: pid, code: code, text: text, flags: flags)
    emit(["t_ns": t])
}

// MARK: - scroll

/// Sends --lines one-line scroll-down events at --rate per second to the
/// window's center while capturing it. The pointer is moved to the window
/// center first because the window server routes wheel events to the window
/// under the pointer. Counts frames with new content from the first event to
/// 250 ms after the last, and "dropped" = gaps between consecutive new frames
/// longer than 1.5 refresh periods while events are still being sent.
func scroll() async {
    let pid = pid_t(intOption("pid"))
    let lines = intOption("lines")
    let rate = doubleOption("rate", default: 60)
    guard lines > 0, rate > 0 else { fail("--lines and --rate must be positive", code: 1) }
    let timeout = doubleOption("timeout", default: 60)
    let deadline = nowNs() + UInt64(timeout * 1e9)

    let window = await findWindow(pid: pid, deadline: deadline)
    let hz = refreshHz(for: window)
    let lock = NSLock()
    var stamps: [UInt64] = []
    let capture = Capture(window: window, hz: hz) { f in
        lock.withLock { stamps.append(f.tNs) }
        return false
    }
    let center = CGPoint(x: window.frame.midX, y: window.frame.midY)
    let sendDuration = Double(lines) / rate
    let stopAt = nowNs() + UInt64((sendDuration + 1.5) * 1e9)
    var sendStart: UInt64 = 0, sendEnd: UInt64 = 0

    let sender = Thread {
        usleep(500_000)  // let capture deliver its first frame
        CGEvent(mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: center,
                mouseButton: .left)?.post(tap: .cghidEventTap)
        usleep(50_000)
        let periodNs = UInt64(1e9 / rate)
        let start = nowNs()
        for i in 0..<lines {
            let target = start + UInt64(i) * periodNs
            while nowNs() < target { usleep(200) }
            if let e = CGEvent(scrollWheelEvent2Source: nil, units: .line, wheelCount: 1,
                               wheel1: -1, wheel2: 0, wheel3: 0) {
                e.location = center
                e.post(tap: .cghidEventTap)
            }
        }
        lock.withLock { sendStart = start; sendEnd = nowNs() }
    }
    sender.start()
    _ = await capture.start(deadline: max(stopAt, nowNs()))
    while true {
        if lock.withLock({ sendEnd != 0 }) { break }
        try? await Task.sleep(nanoseconds: 10_000_000)
    }

    let (window0, window1, all) = lock.withLock { (sendStart, sendEnd, stamps.sorted()) }
    let counted = all.filter { $0 >= window0 && $0 <= window1 + 250_000_000 }
    let limit = 1.5 * 1e9 / Double(hz)
    var dropped = 0
    for (a, b) in zip(counted, counted.dropFirst()) where a <= window1 {
        if Double(b - a) > limit { dropped += 1 }
    }
    emit(["frames": counted.count, "dropped": dropped, "refresh_hz": hz])
}

// MARK: - Dispatch

switch command {
case "now":
    emit(["t_ns": nowNs()])
case "check":
    let p = permissions()
    emit(["screen_recording": p.screen, "accessibility": p.ax])
    exit(p.screen && p.ax ? 0 : 3)
case "watch":
    requirePermissions()
    await watch()
case "scroll":
    requirePermissions()
    await scroll()
case "type":
    requirePermissions()
    typeText()
case "key":
    requirePermissions()
    keyCombo()
default:
    fail("unknown command \(command)\n" + usage, code: 1)
}
exit(0)
