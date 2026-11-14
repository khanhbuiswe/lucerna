// bench-helper: the system-API half of bench/compare.
//
// Python orchestrates; this tool captures a window with ScreenCaptureKit to
// find when its frames change, and posts keyboard/scroll input with CGEvent.
// Every result is one JSON line on stdout. Errors go to stderr with a
// non-zero exit code and nothing on stdout:
//   1 = usage / runtime error, 2 = timeout, 3 = missing permission.
// Arguments are validated before permissions, so a usage error is exit 1
// even on a machine without the grants.
//
// Clock: every timestamp is nanoseconds of CLOCK_UPTIME_RAW, the clock behind
// mach_absolute_time(); Python reads it with
// time.clock_gettime_ns(time.CLOCK_UPTIME_RAW) (`editors.now_ns()`), not
// time.monotonic_ns(), which Python 3.9 counts from process start. Capture
// frames carry SCStreamFrameInfo.displayTime, which is in mach absolute time
// units; it is converted to ns with mach_timebase_info, so frame times and
// Python's t0 share one time base.
//
// --pid takes a comma-separated list: launchers such as VS Code's `code` or
// Zed's `cli` do not own the window, so Python passes every pid of the bench
// session and the helper picks the largest on-screen layer-0 window owned by
// any of them.

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

func note(_ message: String) {
    FileHandle.standardError.write((message + "\n").data(using: .utf8)!)
}

func fail(_ message: String, code: Int32) -> Never {
    note("bench-helper: " + message)
    exit(code)
}

let usage = """
usage: bench-helper <command> [options]      (<pids> = pid[,pid...])
  now
  check
  sample --pid <pids> [--timeout <s>]
  watch  --pid <pids> --since <t0_ns> --until coverage:<0..1>|change
         [--timeout <s>] [--baseline <png>] [--background RRGGBB]
  scroll --pid <pids> --lines <n> [--rate <events/s>] [--timeout <s>]
  type   --pid <pids> --text <s> [--interval-ms <ms>]
  key    --pid <pids> --combo <cmd+p>
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

func intOption(_ name: String, default def: Int? = nil, min lo: Int = Int.min) -> Int {
    guard let raw = option(name) else {
        if let def { return def }
        fail("missing --\(name)\n" + usage, code: 1)
    }
    guard let v = Int(raw), v >= lo else {
        fail("--\(name) must be an integer >= \(lo)", code: 1)
    }
    return v
}

/// Finite number > 0 (or nil when absent and no default).
func positiveOption(_ name: String) -> Double? {
    guard let raw = option(name) else { return nil }
    guard let v = Double(raw), v.isFinite, v > 0, v < 1e7 else {
        fail("--\(name) must be a positive number", code: 1)
    }
    return v
}

func pidsOption() -> Set<pid_t> {
    let parts = required("pid").split(separator: ",", omittingEmptySubsequences: false)
    var pids = Set<pid_t>()
    for p in parts {
        guard let v = Int32(p.trimmingCharacters(in: .whitespaces)), v > 0 else {
            fail("--pid must be a comma-separated list of positive pids", code: 1)
        }
        pids.insert(v)
    }
    return pids
}

/// Deadline from --timeout (default 60 s).
func deadlineOption() -> (seconds: Double, deadline: UInt64) {
    let s = positiveOption("timeout") ?? 60
    return (s, nowNs() + UInt64(s * 1e9))
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

/// Every measuring command needs both grants (window lookup uses
/// ScreenCaptureKit, input uses CGEvent); refuse rather than report zeros.
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

/// Largest on-screen layer-0 window owned by any of `pids`, polled until
/// `deadline` because a freshly launched app may not have a window yet.
func findWindow(pids: Set<pid_t>, deadline: UInt64) async -> SCWindow {
    while true {
        if let content = try? await SCShareableContent.excludingDesktopWindows(
            false, onScreenWindowsOnly: true) {
            let mine = content.windows.filter {
                guard let owner = $0.owningApplication?.processID else { return false }
                return pids.contains(owner) && $0.windowLayer == 0
                    && $0.frame.width >= 100 && $0.frame.height >= 100
            }
            if let best = mine.max(by: {
                $0.frame.width * $0.frame.height < $1.frame.width * $1.frame.height
            }) {
                return best
            }
        }
        if nowNs() >= deadline {
            fail("timeout: no on-screen window for pid \(pids.sorted())", code: 2)
        }
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
///
/// The capture size is fixed at stream start (the window's size then, in
/// points); if the window is resized later ScreenCaptureKit scales its
/// content into that size.
///
/// After the first complete frame has gone through `onFrame` (so a watch has
/// stored its reference), "ready" is written to stderr and `firstFrameSeen`
/// becomes true.
final class Capture: NSObject, SCStreamOutput, SCStreamDelegate, @unchecked Sendable {
    private let queue = DispatchQueue(label: "bench-helper.capture", qos: .userInteractive)
    private let lock = NSLock()
    private var continuation: CheckedContinuation<Bool, Never>?
    private var finished = false
    private var sawFirst = false
    private var stream: SCStream?
    private let onFrame: (Frame) -> Bool
    let width: Int
    let height: Int
    let hz: Int

    var firstFrameSeen: Bool { lock.withLock { sawFirst } }

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
        let delay = deadline > nowNs() ? deadline - nowNs() : 0
        queue.asyncAfter(deadline: .now() + .nanoseconds(Int(min(delay, UInt64(Int.max))))) {
            [weak self] in self?.finish(false)
        }
        let result = await withCheckedContinuation { (c: CheckedContinuation<Bool, Never>) in
            let already: Bool = lock.withLock {
                if finished { return true }
                continuation = c
                return false
            }
            if already { c.resume(returning: true) }
        }
        try? await stream.stopCapture()
        return result
    }

    private func finish(_ value: Bool) {
        let c: CheckedContinuation<Bool, Never>? = lock.withLock {
            if finished { return nil }
            finished = true
            defer { continuation = nil }
            return continuation
        }
        c?.resume(returning: value)
    }

    func stream(_ stream: SCStream, didOutputSampleBuffer sb: CMSampleBuffer,
                of type: SCStreamOutputType) {
        guard type == .screen, sb.isValid, !lock.withLock({ finished }) else { return }
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
        let stop = onFrame(frame)
        let first: Bool = lock.withLock {
            defer { sawFirst = true }
            return !sawFirst
        }
        if first { note("ready") }
        if stop { finish(true) }
    }

    func stream(_ stream: SCStream, didStopWithError error: Error) {
        fail("capture stopped: \(error.localizedDescription)", code: 1)
    }
}

// MARK: - Pixel analysis

/// Pixels are sampled on a 2×2 grid: a quarter of the work, same statistics.
let sampleStep = 2
/// Per-channel tolerance: absorbs anti-aliasing and colour-matching noise.
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

/// The central 60%×60% region that coverage measures.
func centerRegion(_ f: Frame) -> (x: Range<Int>, y: Range<Int>) {
    (f.width / 5 ..< f.width - f.width / 5, f.height / 5 ..< f.height - f.height / 5)
}

/// Most common colour of a region (5-bit-per-channel buckets) as BGR bytes,
/// and the fraction of the region's sampled pixels in that bucket.
func dominantColor(_ f: Frame, x xs: Range<Int>, y ys: Range<Int>) -> ([UInt8], Double) {
    var counts = [Int](repeating: 0, count: 1 << 15)
    var total = 0
    for y in stride(from: ys.lowerBound, to: ys.upperBound, by: sampleStep) {
        for x in stride(from: xs.lowerBound, to: xs.upperBound, by: sampleStep) {
            let p = f.pixel(x, y)
            counts[(Int(p[0]) >> 3) | (Int(p[1]) >> 3) << 5 | (Int(p[2]) >> 3) << 10] += 1
            total += 1
        }
    }
    let key = counts.indices.max { counts[$0] < counts[$1] }!
    let bgr = [UInt8((key & 31) << 3 | 4), UInt8((key >> 5 & 31) << 3 | 4),
               UInt8((key >> 10 & 31) << 3 | 4)]
    return (bgr, total == 0 ? 0 : Double(counts[key]) / Double(total))
}

func hexRGB(_ bgr: [UInt8]) -> String {
    String(format: "%02X%02X%02X", bgr[2], bgr[1], bgr[0])
}

/// Fraction of the central region whose pixels differ from `bg`.
func coverage(_ f: Frame, background bg: [UInt8]) -> Double {
    let (xs, ys) = centerRegion(f)
    var hit = 0, total = 0
    bg.withUnsafeBufferPointer { b in
        for y in stride(from: ys.lowerBound, to: ys.upperBound, by: sampleStep) {
            for x in stride(from: xs.lowerBound, to: xs.upperBound, by: sampleStep) {
                total += 1
                if differs(f.pixel(x, y), b.baseAddress!) { hit += 1 }
            }
        }
    }
    return total == 0 ? 0 : Double(hit) / Double(total)
}

// MARK: - sample

/// Calibration: waits for the window, takes one complete frame and reports
/// the dominant colour of the coverage region and of the whole window, so
/// Python can pass a per-editor --background to watch.
func sample(pids: Set<pid_t>, timeout: (seconds: Double, deadline: UInt64)) async {
    let window = await findWindow(pids: pids, deadline: timeout.deadline)
    var result: [String: Any] = [:]
    let capture = Capture(window: window, hz: refreshHz(for: window)) { f in
        let (cx, cy) = centerRegion(f)
        let (center, frac) = dominantColor(f, x: cx, y: cy)
        let (whole, _) = dominantColor(f, x: 0..<f.width, y: 0..<f.height)
        result = ["center_dominant": hexRGB(center), "center_dominant_frac": frac,
                  "window_dominant": hexRGB(whole), "width": f.width, "height": f.height]
        return true
    }
    guard await capture.start(deadline: timeout.deadline) else {
        fail("timeout after \(timeout.seconds) s waiting for a frame", code: 2)
    }
    emit(result)
}

// MARK: - watch

enum WatchMode { case coverage(Double), change }

/// Prints the time of the first frame at or after --since that meets
/// --until:
///   coverage:x  the central region differs from the background colour on at
///               least x of its pixels. Background = --background RRGGBB
///               (calibrate with `sample`), else — with a warning — the
///               dominant colour of the first captured frame.
///   change      more than 0.5% of pixels differ from the reference frame. The
///               reference is --baseline <png>, else the last frame before
///               --since, else the first frame captured. The 0.5% floor
///               ignores a blinking caret. Start watch and wait for "ready"
///               on stderr before triggering the change.
/// "first_frame_hit" is true when the very first complete frame met the
/// condition: the measurement is then suspect (content may predate capture).
func watch(pids: Set<pid_t>, since: UInt64, mode: WatchMode, until: String,
           background bgOption: [UInt8]?, baseline: String?,
           timeout: (seconds: Double, deadline: UInt64)) async {
    let window = await findWindow(pids: pids, deadline: timeout.deadline)
    let hz = refreshHz(for: window)
    let w = max(1, Int(window.frame.width)), h = max(1, Int(window.frame.height))
    var reference: Bitmap? = nil
    if let baseline {
        guard let b = Bitmap(png: baseline, width: w, height: h) else {
            fail("cannot read baseline \(baseline)", code: 1)
        }
        reference = b
    }
    var background = bgOption
    if case .coverage = mode, background == nil {
        note("bench-helper: warning: no --background; using the first frame's dominant "
             + "colour, wrong if that frame already shows content")
    }

    var frames = 0
    var hitNs: UInt64 = 0
    let capture = Capture(window: window, hz: hz) { f in
        frames += 1
        switch mode {
        case .coverage(let x):
            if background == nil {
                background = dominantColor(f, x: 0..<f.width, y: 0..<f.height).0
            }
            if f.tNs >= since && coverage(f, background: background!) >= x {
                hitNs = f.tNs
                return true
            }
        case .change:
            if f.tNs < since {
                if baseline == nil { reference = Bitmap(f) }
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
    guard await capture.start(deadline: timeout.deadline) else {
        fail("timeout after \(timeout.seconds) s waiting for \(until)", code: 2)
    }
    emit(["t_ns": hitNs, "frames": frames, "first_frame_hit": frames == 1])
}

// MARK: - Input

/// US ANSI virtual key codes. Apps that read the key code (not only the
/// characters) need the real one. `type` supports exactly these characters
/// (ASCII letters, digits, space, basic punctuation) plus "\n" (Return).
let keyCodes: [Character: CGKeyCode] = [
    "a": 0, "s": 1, "d": 2, "f": 3, "h": 4, "g": 5, "z": 6, "x": 7, "c": 8, "v": 9,
    "b": 11, "q": 12, "w": 13, "e": 14, "r": 15, "y": 16, "t": 17, "1": 18, "2": 19,
    "3": 20, "4": 21, "6": 22, "5": 23, "=": 24, "9": 25, "7": 26, "-": 27, "8": 28,
    "0": 29, "]": 30, "o": 31, "u": 32, "[": 33, "i": 34, "p": 35, "l": 37, "j": 38,
    "'": 39, "k": 40, ";": 41, "\\": 42, ",": 43, "/": 44, "n": 45, "m": 46, ".": 47,
    " ": 49, "`": 50, "\n": 36,
]
/// Shifted US ANSI characters → their unshifted key.
let shifted: [Character: Character] = [
    "!": "1", "@": "2", "#": "3", "$": "4", "%": "5", "^": "6", "&": "7", "*": "8",
    "(": "9", ")": "0", "_": "-", "+": "=", "{": "[", "}": "]", "|": "\\", ":": ";",
    "\"": "'", "<": ",", ">": ".", "?": "/", "~": "`",
]
let namedKeys: [String: CGKeyCode] = [
    "return": 36, "enter": 36, "tab": 48, "space": 49, "delete": 51, "backspace": 51,
    "escape": 53, "esc": 53, "left": 123, "right": 124, "down": 125, "up": 126,
]

struct KeyStroke {
    let code: CGKeyCode
    let text: String?
    let flags: CGEventFlags
}

func strokes(for text: String) -> [KeyStroke] {
    text.map { ch -> KeyStroke in
        if let code = keyCodes[ch] {
            return KeyStroke(code: code, text: ch == "\n" ? "\r" : String(ch), flags: [])
        }
        if ch.isASCII, ch.isUppercase, let code = keyCodes[Character(ch.lowercased())] {
            return KeyStroke(code: code, text: String(ch), flags: .maskShift)
        }
        if let base = shifted[ch], let code = keyCodes[base] {
            return KeyStroke(code: code, text: String(ch), flags: .maskShift)
        }
        fail("unsupported character \(ch.debugDescription) (ASCII letters, digits, space, "
             + "punctuation and \\n only)", code: 1)
    }
}

func comboStroke(_ combo: String) -> KeyStroke {
    let parts = combo.lowercased().split(separator: "+").map(String.init)
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
    if let named = namedKeys[keyName] { return KeyStroke(code: named, text: nil, flags: flags) }
    if keyName.count == 1, let c = keyCodes[Character(keyName)] {
        return KeyStroke(code: c, text: keyName, flags: flags)
    }
    fail("unknown key \(keyName)", code: 1)
}

func post(_ k: KeyStroke, to pid: pid_t) {
    for down in [true, false] {
        guard let e = CGEvent(keyboardEventSource: nil, virtualKey: k.code, keyDown: down) else {
            fail("cannot create key event", code: 1)
        }
        e.flags = k.flags
        if let text = k.text {
            let units = Array(text.utf16)
            e.keyboardSetUnicodeString(stringLength: units.count, unicodeString: units)
        }
        e.postToPid(pid)
    }
}

/// Finds the window among `pids`, brings its owning app frontmost
/// (NSRunningApplication.activate; key events posted to a pid go to its key
/// window, which only takes input when the app is active) and returns the
/// owner pid to post to.
func activateOwner(pids: Set<pid_t>) async -> pid_t {
    let window = await findWindow(pids: pids, deadline: nowNs() + 10_000_000_000)
    guard let owner = window.owningApplication?.processID else {
        fail("window has no owning application", code: 1)
    }
    if let app = NSRunningApplication(processIdentifier: owner), !app.isActive {
        app.activate()
        let until = nowNs() + 2_000_000_000
        while !app.isActive && nowNs() < until {
            try? await Task.sleep(nanoseconds: 20_000_000)
        }
        try? await Task.sleep(nanoseconds: 100_000_000)
    }
    return owner
}

/// Posts each character and prints the time each key-down was sent.
func typeText(pids: Set<pid_t>, keys: [KeyStroke], intervalMs: Int) async {
    let pid = await activateOwner(pids: pids)
    var times: [UInt64] = []
    for (i, k) in keys.enumerated() {
        if i > 0 { usleep(UInt32(intervalMs) * 1000) }
        times.append(nowNs())
        post(k, to: pid)
    }
    emit(["t_ns": times])
}

func keyCombo(pids: Set<pid_t>, stroke: KeyStroke) async {
    let pid = await activateOwner(pids: pids)
    let t = nowNs()
    post(stroke, to: pid)
    emit(["t_ns": t])
}

// MARK: - scroll

/// Sends --lines one-line scroll-down events at --rate per second (default:
/// the display refresh rate) to the window's center while capturing it. The
/// pointer is moved to the window center first because the window server
/// routes wheel events to the window under the pointer.
///
/// frames = new frames from the first event to 250 ms after the last.
/// dropped = gaps between consecutive new frames, starting while events are
/// still being sent, longer than 1.5 × max(refresh period, event period).
/// The brief says 1.5 × refresh period; with --rate below the refresh rate
/// every gap would count, so the event period is the floor.
func scroll(pids: Set<pid_t>, lines: Int, rateOption: Double?,
            timeout: (seconds: Double, deadline: UInt64)) async {
    let window = await findWindow(pids: pids, deadline: timeout.deadline)
    let hz = refreshHz(for: window)
    let rate = rateOption ?? Double(hz)
    let lock = NSLock()
    var stamps: [UInt64] = []
    let capture = Capture(window: window, hz: hz) { f in
        lock.withLock { stamps.append(f.tNs) }
        return false
    }
    let center = CGPoint(x: window.frame.midX, y: window.frame.midY)
    var sendStart: UInt64 = 0, sendEnd: UInt64 = 0
    let sendDuration = Double(lines) / rate

    let sender = Thread {
        // Wait for the first frame (at most 3 s; a static window may not
        // send one until something changes).
        let waitUntil = nowNs() + 3_000_000_000
        while !capture.firstFrameSeen && nowNs() < waitUntil { usleep(5_000) }
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
    let stopAt = nowNs() + UInt64((sendDuration + 3.5) * 1e9)
    _ = await capture.start(deadline: min(max(stopAt, nowNs()), max(timeout.deadline, nowNs())))
    while !lock.withLock({ sendEnd != 0 }) {
        if nowNs() >= timeout.deadline { fail("timeout while sending scroll events", code: 2) }
        try? await Task.sleep(nanoseconds: 10_000_000)
    }

    let (window0, window1, all) = lock.withLock { (sendStart, sendEnd, stamps.sorted()) }
    let counted = all.filter { $0 >= window0 && $0 <= window1 + 250_000_000 }
    let limit = 1.5 * max(1e9 / Double(hz), 1e9 / rate)
    var dropped = 0
    for (a, b) in zip(counted, counted.dropFirst()) where a <= window1 {
        if Double(b - a) > limit { dropped += 1 }
    }
    emit(["frames": counted.count, "dropped": dropped, "refresh_hz": hz, "rate": rate])
}

// MARK: - Dispatch (parse, then check permissions, then measure)

switch command {
case "now":
    emit(["t_ns": nowNs()])
case "check":
    let p = permissions()
    emit(["screen_recording": p.screen, "accessibility": p.ax])
    exit(p.screen && p.ax ? 0 : 3)
case "sample":
    let pids = pidsOption()
    let timeout = deadlineOption()
    requirePermissions()
    await sample(pids: pids, timeout: timeout)
case "watch":
    let pids = pidsOption()
    let since = UInt64(intOption("since", min: 0))
    let until = required("until")
    let mode: WatchMode
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
    let timeout = deadlineOption()
    requirePermissions()
    await watch(pids: pids, since: since, mode: mode, until: until, background: background,
                baseline: option("baseline"), timeout: timeout)
case "scroll":
    let pids = pidsOption()
    let lines = intOption("lines", min: 1)
    let rate = positiveOption("rate")
    let timeout = deadlineOption()
    requirePermissions()
    await scroll(pids: pids, lines: lines, rateOption: rate, timeout: timeout)
case "type":
    let pids = pidsOption()
    let keys = strokes(for: required("text"))
    let interval = intOption("interval-ms", default: 100, min: 0)
    requirePermissions()
    await typeText(pids: pids, keys: keys, intervalMs: interval)
case "key":
    let pids = pidsOption()
    let stroke = comboStroke(required("combo"))
    requirePermissions()
    await keyCombo(pids: pids, stroke: stroke)
default:
    fail("unknown command \(command)\n" + usage, code: 1)
}
exit(0)
