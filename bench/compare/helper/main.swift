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
// --pid takes a comma-separated list: Python passes every pid of the bench
// session (helpers included) and the helper picks the largest on-screen
// layer-0 window owned by any of them. `frame` and `scroll` also take
// --window-index <n>: the n-th (0 = oldest) on-screen layer-0 window of the
// pids, ordered by CGWindowID, which macOS hands out in creation order. It
// picks one window when one process shows several (the one-process app).
//
// `watch --display` is for startup: it starts capturing the main display
// BEFORE the editor is launched, so the first frame of the new window is
// never missed. The window is found on every frame by its owner's
// executable path (proc_pidpath), since its pid is not known yet.

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
         [--min-change <0..1>] [--dump-dir <dir>]     (the last two: change only)
  watch  --display --exe <absolute path> [--exclude-pids <pids>] --since <t0_ns|0>
         --until coverage:<0..1> --background RRGGBB [--timeout <s>] [--hold-ms <ms>]
         [--allow-resize]                              (needs --hold-ms > 0)
  frame  --pid <pids> --x <pt> --y <pt> --w <pt> --h <pt> [--timeout <s>]
         [--window-index <n>]
  activate --pid <pids> [--timeout <s>]
  display
  scroll --pid <pids> --lines <n> [--rate <events/s>] [--timeout <s>]
         [--unit line|pixel] [--pixels-per-event <n>] [--dump-dir <dir>]
         [--window-index <n>]
  record --pid <pids> --seconds <s> [--dump-dir <dir>] [--timeout <s>]
  type   --pid <pids> --text <s> [--interval-ms <ms>] [--timeout <s>]
  key    --pid <pids> --combo <cmd+p> [--timeout <s>]
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

/// Number in (0, 1] (nil when absent).
func fractionOption(_ name: String) -> Double? {
    guard let raw = option(name) else { return nil }
    guard let v = Double(raw), v.isFinite, v > 0, v <= 1 else {
        fail("--\(name) must be a number in (0, 1]", code: 1)
    }
    return v
}

/// --dump-dir: a non-empty path (created when the measurement writes to it).
func dumpDirOption() -> String? {
    guard let raw = option("dump-dir") else {
        if flag("dump-dir") { fail("--dump-dir needs a path", code: 1) }
        return nil
    }
    guard !raw.isEmpty, !raw.hasPrefix("--") else { fail("--dump-dir needs a path", code: 1) }
    return raw
}

func pidList(_ name: String, _ raw: String) -> Set<pid_t> {
    let parts = raw.split(separator: ",", omittingEmptySubsequences: false)
    var pids = Set<pid_t>()
    for p in parts {
        guard let v = Int32(p.trimmingCharacters(in: .whitespaces)), v > 0 else {
            fail("--\(name) must be a comma-separated list of positive pids", code: 1)
        }
        pids.insert(v)
    }
    return pids
}

func pidsOption() -> Set<pid_t> { pidList("pid", required("pid")) }

func flag(_ name: String) -> Bool { argv.contains("--" + name) }

/// Deadline from --timeout (default 60 s; 10 s for key/type window lookup).
func deadlineOption(default def: Double = 60) -> (seconds: Double, deadline: UInt64) {
    let s = positiveOption("timeout") ?? def
    return (s, nowNs() + UInt64(s * 1e9))
}

// MARK: - Permissions

/// Preflight only: never prompts (no Request variants). TCC attributes the
/// grant to the responsible app (the terminal or app that launched us).
/// BENCH_HELPER_FAKE_DENY (for tests): "1" denies both, "screen" denies only
/// Screen Recording, "accessibility" denies only Accessibility.
func permissions() -> (screen: Bool, ax: Bool) {
    let fake = ProcessInfo.processInfo.environment["BENCH_HELPER_FAKE_DENY"]
    if fake == "1" { return (false, false) }
    let screen = fake == "screen" ? false : CGPreflightScreenCaptureAccess()
    let ax = fake == "accessibility" ? false : AXIsProcessTrusted()
    return (screen, ax)
}

/// Each measuring command checks only the grants it uses, and refuses rather
/// than report zeros:
///   watch, sample, record  Screen Recording (ScreenCaptureKit capture)
///   scroll         Screen Recording + Accessibility (capture + CGEvent)
///   key, type      Accessibility (CGEvent; the window is found with
///                  CGWindowList, which needs no Screen Recording)
///   frame          Accessibility (AX window position and size)
///   activate       Accessibility (brings the window's app frontmost, like key)
///   now, display   nothing
func requirePermissions(screen needScreen: Bool, accessibility needAx: Bool) {
    let p = permissions()
    var missing: [String] = []
    if needScreen && !p.screen { missing.append("Screen Recording") }
    if needAx && !p.ax { missing.append("Accessibility") }
    if !missing.isEmpty {
        fail("missing permission: \(missing.joined(separator: ", ")) — grant it to the app "
             + "that runs the benchmark (System Settings > Privacy & Security)", code: 3)
    }
}

// MARK: - Window lookup

/// Largest on-screen layer-0 window owned by any of `pids`, polled until
/// `deadline` because a freshly launched app may not have a window yet.
func findWindow(pids: Set<pid_t>, deadline: UInt64) async -> SCWindow {
    if let index = windowIndex {
        while true {
            if let chosen = indexedWindow(pids: pids, index: index),
               let content = try? await SCShareableContent.excludingDesktopWindows(
                false, onScreenWindowsOnly: true),
               let win = content.windows.first(where: { $0.windowID == chosen.id }) {
                return win
            }
            if nowNs() >= deadline {
                fail("timeout: no window #\(index) for pid \(pids.sorted())", code: 2)
            }
            try? await Task.sleep(nanoseconds: 50_000_000)
        }
    }
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
    private var stopGeneration = 0
    private var stream: SCStream?
    private let onFrame: (Frame) -> Bool
    let width: Int
    let height: Int
    let hz: Int

    var firstFrameSeen: Bool { lock.withLock { sawFirst } }

    convenience init(window: SCWindow, hz: Int, onFrame: @escaping (Frame) -> Bool) {
        // Capture at point size (scale 1): plenty for pixel statistics and
        // cheap enough to analyse every frame at 120 Hz.
        self.init(filter: SCContentFilter(desktopIndependentWindow: window),
                  width: Int(window.frame.width), height: Int(window.frame.height),
                  hz: hz, onFrame: onFrame)
    }

    init(filter: SCContentFilter, width w: Int, height h: Int, hz: Int,
         onFrame: @escaping (Frame) -> Bool) {
        self.onFrame = onFrame
        self.hz = hz
        width = max(1, w)
        height = max(1, h)
        super.init()
        let config = SCStreamConfiguration()
        config.width = width
        config.height = height
        config.pixelFormat = kCVPixelFormatType_32BGRA
        config.minimumFrameInterval = CMTime(value: 1, timescale: CMTimeScale(hz))
        config.queueDepth = 6
        config.showsCursor = false
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

    /// Stops the capture (as a hit) `ms` milliseconds from now, even when
    /// no new frame arrives: an unchanged screen sends only idle frames.
    /// A later call replaces the earlier one.
    func stop(afterMs ms: Int) {
        let gen: Int = lock.withLock { stopGeneration += 1; return stopGeneration }
        queue.asyncAfter(deadline: .now() + .milliseconds(ms)) { [weak self] in
            guard let self, self.lock.withLock({ self.stopGeneration == gen }) else { return }
            self.finish(true)
        }
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

    /// A black bitmap of the given size.
    init(width: Int, height: Int) {
        self.width = width
        self.height = height
        bytes = [UInt8](repeating: 0, count: width * height * 4)
    }

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

// MARK: - Diagnostics output (--dump-dir)

/// Creates `dir` (and parents); exit 1 when that fails.
func makeDumpDir(_ dir: String) {
    do {
        try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
    } catch {
        fail("cannot create --dump-dir \(dir): \(error.localizedDescription)", code: 1)
    }
}

/// Writes a BGRA bitmap as PNG (alpha ignored); notes on stderr if it fails.
func writePNG(_ b: Bitmap, to path: String) {
    let data = Data(b.bytes) as CFData
    guard let provider = CGDataProvider(data: data),
          let image = CGImage(
            width: b.width, height: b.height, bitsPerComponent: 8, bitsPerPixel: 32,
            bytesPerRow: b.width * 4, space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.noneSkipFirst.rawValue
                | CGBitmapInfo.byteOrder32Little.rawValue),
            provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent),
          let dest = CGImageDestinationCreateWithURL(
            URL(fileURLWithPath: path) as CFURL, "public.png" as CFString, 1, nil) else {
        note("bench-helper: warning: cannot write \(path)")
        return
    }
    CGImageDestinationAddImage(dest, image, nil)
    if !CGImageDestinationFinalize(dest) { note("bench-helper: warning: cannot write \(path)") }
}

/// Writes `lines` (one per line) to `path`; notes on stderr if it fails.
func writeLines(_ lines: [String], to path: String) {
    let text = lines.isEmpty ? "" : lines.joined(separator: "\n") + "\n"
    do {
        try text.write(toFile: path, atomically: true, encoding: .utf8)
    } catch {
        note("bench-helper: warning: cannot write \(path): \(error.localizedDescription)")
    }
}

/// Linear-interpolated percentile of an ascending array (p in 0...1).
func percentile(_ sorted: [Double], _ p: Double) -> Double {
    guard !sorted.isEmpty else { return 0 }
    let pos = p * Double(sorted.count - 1)
    let lo = Int(pos.rounded(.down)), hi = min(lo + 1, sorted.count - 1)
    return sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - Double(lo))
}

/// Gaps (ms) between consecutive frame times and their p50 / p90 / max; the
/// statistics are null when there are fewer than two frames.
func intervalStats(_ gapsMs: [Double]) -> [String: Any] {
    let sorted = gapsMs.sorted()
    func value(_ v: Double) -> Any { sorted.isEmpty ? NSNull() : (v * 1000).rounded() / 1000 }
    return ["interval_ms_p50": value(percentile(sorted, 0.5)),
            "interval_ms_p90": value(percentile(sorted, 0.9)),
            "interval_ms_max": value(sorted.last ?? 0)]
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

/// Owned BGRA copy of a rectangle of a frame (the editor window cropped
/// from a display frame).
func cropBitmap(_ f: Frame, x xs: Range<Int>, y ys: Range<Int>) -> Bitmap {
    var b = Bitmap(width: xs.count, height: ys.count)
    b.bytes.withUnsafeMutableBufferPointer { dst in
        for (row, y) in ys.enumerated() {
            memcpy(dst.baseAddress! + row * xs.count * 4, f.pixel(xs.lowerBound, y), xs.count * 4)
        }
    }
    return b
}

/// Fraction of sampled pixels of the rectangle at (x0, y0) of `f`, the size
/// of `ref`, that differ from `ref` (same tolerance as changedFraction). A
/// rectangle that does not fit in the frame counts as a full change.
func changedFraction(_ f: Frame, x0: Int, y0: Int, _ ref: Bitmap) -> Double {
    guard x0 >= 0, y0 >= 0, x0 + ref.width <= f.width, y0 + ref.height <= f.height else {
        return 1
    }
    var changed = 0, total = 0
    ref.bytes.withUnsafeBufferPointer { r in
        for y in stride(from: 0, to: ref.height, by: sampleStep) {
            for x in stride(from: 0, to: ref.width, by: sampleStep) {
                total += 1
                if differs(f.pixel(x0 + x, y0 + y), r.baseAddress! + (y * ref.width + x) * 4) {
                    changed += 1
                }
            }
        }
    }
    return total == 0 ? 0 : Double(changed) / Double(total)
}

/// The central 60%×60% region that coverage measures.
func centerRegion(_ f: Frame) -> (x: Range<Int>, y: Range<Int>) {
    centerRegion(x: 0..<f.width, y: 0..<f.height)
}

/// The central 60%×60% of a sub-rectangle (a window cropped from a display frame).
func centerRegion(x xs: Range<Int>, y ys: Range<Int>) -> (x: Range<Int>, y: Range<Int>) {
    (xs.lowerBound + xs.count / 5 ..< xs.upperBound - xs.count / 5,
     ys.lowerBound + ys.count / 5 ..< ys.upperBound - ys.count / 5)
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
    coverage(f, region: centerRegion(f), background: bg)
}

/// Fraction of `region`'s pixels that differ from `bg`.
func coverage(_ f: Frame, region: (x: Range<Int>, y: Range<Int>), background bg: [UInt8]) -> Double {
    let (xs, ys) = region
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
///   change      more than --min-change (default 0.005 = 0.5%) of sampled
///               pixels differ from the reference frame. The reference is
///               --baseline <png>, else the last frame before --since, else
///               the first frame captured. The 0.5% floor ignores a blinking
///               caret. Start watch and wait for "ready" on stderr before
///               triggering the change.
/// "first_frame_hit" is true when the very first complete frame met the
/// condition: the measurement is then suspect (content may predate capture).
///
/// Change diagnostics: every frame compared after --since updates
/// "max_change" (highest fraction seen) and "small_changes" (frames with a
/// fraction > 0 but not above the threshold). On success the JSON adds
/// "change" (the hit frame's fraction) and "max_change"; on timeout the
/// stderr message carries max_change and small_changes. --dump-dir writes
/// ref.png (the reference), max.png (the frame with the highest change) and
/// changes.txt ("t_ns fraction" per compared frame), on success and timeout.
func watch(pids: Set<pid_t>, since: UInt64, mode: WatchMode, until: String,
           background bgOption: [UInt8]?, baseline: String?, minChange: Double,
           dumpDir: String?, timeout: (seconds: Double, deadline: UInt64)) async {
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

    if let dumpDir { makeDumpDir(dumpDir) }
    var frames = 0
    var hitNs: UInt64 = 0
    var hitChange = 0.0, maxChange = 0.0, smallChanges = 0
    var maxFrame: Bitmap? = nil
    var changeLog: [String] = []
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
            let fraction = changedFraction(f, reference!)
            if dumpDir != nil {
                changeLog.append("\(f.tNs) \(fraction)")
                if maxFrame == nil || fraction > maxChange { maxFrame = Bitmap(f) }
            }
            maxChange = max(maxChange, fraction)
            if fraction > minChange {
                hitNs = f.tNs
                hitChange = fraction
                return true
            }
            if fraction > 0 { smallChanges += 1 }
        }
        return false
    }
    let hit = await capture.start(deadline: timeout.deadline)
    if let dumpDir, case .change = mode {
        if let reference { writePNG(reference, to: dumpDir + "/ref.png") }
        if let maxFrame { writePNG(maxFrame, to: dumpDir + "/max.png") }
        writeLines(changeLog, to: dumpDir + "/changes.txt")
    }
    guard hit else {
        var extra = ""
        if case .change = mode {
            extra = " max_change=\(maxChange) small_changes=\(smallChanges)"
        }
        fail("timeout after \(timeout.seconds) s waiting for \(until)" + extra, code: 2)
    }
    var result: [String: Any] = ["t_ns": hitNs, "frames": frames, "first_frame_hit": frames == 1]
    if case .change = mode {
        result["change"] = hitChange
        result["max_change"] = maxChange
    }
    emit(result)
}

/// The main display (the one with the menu bar) as ScreenCaptureKit sees it.
func mainDisplay(deadline: UInt64) async -> SCDisplay {
    let id = CGMainDisplayID()
    while true {
        if let content = try? await SCShareableContent.excludingDesktopWindows(
            false, onScreenWindowsOnly: true),
           let display = content.displays.first(where: { $0.displayID == id }) {
            return display
        }
        if nowNs() >= deadline { fail("timeout: main display not available", code: 2) }
        try? await Task.sleep(nanoseconds: 50_000_000)
    }
}

/// Maximum refresh rate of a display (falls back to 60).
func refreshHz(display id: CGDirectDisplayID) -> Int {
    let key = NSDeviceDescriptionKey("NSScreenNumber")
    let screen = NSScreen.screens.first {
        ($0.deviceDescription[key] as? NSNumber)?.uint32Value == id
    }
    let hz = screen?.maximumFramesPerSecond ?? 60
    return hz > 0 ? hz : 60
}

/// `watch --display`: captures the whole main display, started BEFORE the
/// editor is launched. On every complete frame it looks up (CGWindowList)
/// the largest on-screen layer-0 window whose owner matches `matcher`,
/// crops the frame to that window (points → capture pixels) and applies the
/// coverage test to the crop's central 60%×60%. Frames without such a window
/// never hit.
///
/// The pixels are the composited display, so another window over the editor
/// would be measured instead of it. A frame is only tested when the topmost
/// window (same window list, front to back) at the region's centre and at its
/// four corners belongs to a matched pid; otherwise it counts as occluded
/// (`occluded_frames`) and cannot hit. A covered window therefore delays the
/// measured time, it cannot fake an early one.
///
/// `since` 0 means "report t_ns only": Python records t0 after this helper
/// has printed "ready" and subtracts it itself. `first_frame_hit` means the
/// window was already present and covered on the very first frame, which
/// cannot happen for a window launched after "ready": suspect.
///
/// `holdMs` > 0: after the hit, capture goes on for that long and every new
/// frame's centre region (the region coverage measures) is compared with
/// the hit frame's (same tolerance as changedFraction), on frames where the
/// window is on top at its centre. `post_change` is the largest fraction
/// that differs: a dialog or overlay appearing, or a hit taken before the
/// real content, shows up as a large later change. A window whose size
/// changed counts as 1.0. `post_frames` counts the frames compared.
///
/// `allowResize` (needs `holdMs` > 0): a window that animates to its final
/// size after the hit (macOS zooming a new window of an app already running)
/// is not a change. Each size change re-takes the reference at the new size
/// and restarts the hold; `settled_ns` is the first frame at the final size
/// (`t_ns` when the size never changed).
func watchDisplay(matcher: ExecutableMatcher, since: UInt64, threshold: Double,
                  background bg: [UInt8], holdMs: Int, allowResize: Bool = false,
                  timeout: (seconds: Double, deadline: UInt64)) async {
    let display = await mainDisplay(deadline: timeout.deadline)
    let origin = CGDisplayBounds(display.displayID).origin
    let pointsWide = Double(display.width), pointsHigh = Double(display.height)
    var frames = 0, occluded = 0, framesAtHit = 0
    var hitNs: UInt64 = 0
    var hitWindow: CGRect = .zero
    var hitCrop: Bitmap? = nil
    var postChange = 0.0, postFrames = 0
    var settledNs: UInt64 = 0, resizes = 0
    var captureRef: Capture? = nil
    /// The window's rectangle in capture pixels, nil when too small.
    func crop(_ f: Frame, _ bounds: CGRect) -> (x: Range<Int>, y: Range<Int>)? {
        let sx = Double(f.width) / pointsWide, sy = Double(f.height) / pointsHigh
        let x0 = max(0, Int((Double(bounds.minX - origin.x) * sx).rounded()))
        let y0 = max(0, Int((Double(bounds.minY - origin.y) * sy).rounded()))
        let x1 = min(f.width, Int((Double(bounds.maxX - origin.x) * sx).rounded()))
        let y1 = min(f.height, Int((Double(bounds.maxY - origin.y) * sy).rounded()))
        guard x1 - x0 >= 10, y1 - y0 >= 10 else { return nil }
        return (x0..<x1, y0..<y1)
    }
    let capture = Capture(filter: SCContentFilter(display: display, excludingWindows: []),
                          width: display.width, height: display.height,
                          hz: refreshHz(display: display.displayID)) { f in
        frames += 1
        guard f.tNs >= since else { return false }
        let list = onScreenWindows()
        if let ref = hitCrop {
            // Hold phase: how far does the window move away from the hit frame?
            guard f.tNs > hitNs else { return false }
            if let win = largestWindow(in: list, where: { matcher.matches($0) }) {
                // Only the measured centre region counts, and only while the
                // window is on top: chrome painted later around the text (a
                // sidebar, a status bar) or another window passing over it
                // is not a change of the content that was measured.
                let b = win.bounds
                let onTop = topmostWindowOwner(in: list, at: CGPoint(x: b.midX, y: b.midY))
                    .map { matcher.matches($0) } ?? false
                if onTop && allowResize && b.size != hitWindow.size,
                   let r = crop(f, b) {
                    let c = centerRegion(x: r.x, y: r.y)
                    hitWindow = b
                    hitCrop = cropBitmap(f, x: c.x, y: c.y)
                    settledNs = f.tNs
                    resizes += 1
                    captureRef?.stop(afterMs: holdMs)
                    return false
                }
                if onTop {
                    let fraction: Double
                    if b.size != hitWindow.size {
                        fraction = 1
                    } else if let r = crop(f, b) {
                        let c = centerRegion(x: r.x, y: r.y)
                        fraction = changedFraction(f, x0: c.x.lowerBound, y0: c.y.lowerBound, ref)
                    } else {
                        fraction = 1
                    }
                    postFrames += 1
                    postChange = max(postChange, fraction)
                }
            }
            return f.tNs >= max(hitNs, settledNs) + UInt64(holdMs) * 1_000_000
        }
        guard let win = largestWindow(in: list, where: { matcher.matches($0) }) else {
            return false
        }
        let b = win.bounds
        let probes = [CGPoint(x: b.midX, y: b.midY),
                      CGPoint(x: b.minX + b.width * 0.2 + 1, y: b.minY + b.height * 0.2 + 1),
                      CGPoint(x: b.maxX - b.width * 0.2 - 1, y: b.minY + b.height * 0.2 + 1),
                      CGPoint(x: b.minX + b.width * 0.2 + 1, y: b.maxY - b.height * 0.2 - 1),
                      CGPoint(x: b.maxX - b.width * 0.2 - 1, y: b.maxY - b.height * 0.2 - 1)]
        let visible = probes.allSatisfy { p in
            topmostWindowOwner(in: list, at: p).map { matcher.matches($0) } ?? false
        }
        guard visible else {
            occluded += 1
            return false
        }
        guard let r = crop(f, win.bounds) else { return false }
        let region = centerRegion(x: r.x, y: r.y)
        if coverage(f, region: region, background: bg) >= threshold {
            hitNs = f.tNs
            hitWindow = win.bounds
            framesAtHit = frames
            if holdMs <= 0 { return true }
            hitCrop = cropBitmap(f, x: region.x, y: region.y)
            captureRef?.stop(afterMs: holdMs)
        }
        return false
    }
    captureRef = capture
    // A hold cut short by the deadline still reports the hit.
    let stopped = await capture.start(deadline: timeout.deadline)
    guard stopped || hitNs != 0 else {
        fail("timeout after \(timeout.seconds) s waiting for coverage:\(threshold) "
             + "in a window of \(matcher.exe)", code: 2)
    }
    var result: [String: Any] = [
        "t_ns": hitNs, "frames": framesAtHit, "first_frame_hit": framesAtHit == 1,
        "occluded_frames": occluded,
        "window": [hitWindow.minX, hitWindow.minY, hitWindow.width, hitWindow.height]]
    if holdMs > 0 {
        result["post_change"] = postChange
        result["post_frames"] = postFrames
    }
    if allowResize {
        result["settled_ns"] = max(hitNs, settledNs)
        result["resizes"] = resizes
    }
    emit(result)
}

// MARK: - frame

/// Position and size of an AX window, nil if unreadable.
func axFrame(_ window: AXUIElement) -> CGRect? {
    var posRef: CFTypeRef?, sizeRef: CFTypeRef?
    guard AXUIElementCopyAttributeValue(window, kAXPositionAttribute as CFString, &posRef)
            == .success,
          AXUIElementCopyAttributeValue(window, kAXSizeAttribute as CFString, &sizeRef)
            == .success,
          let posRef, let sizeRef,
          CFGetTypeID(posRef) == AXValueGetTypeID(), CFGetTypeID(sizeRef) == AXValueGetTypeID()
    else { return nil }
    var pos = CGPoint.zero, size = CGSize.zero
    // swiftlint:disable:next force_cast
    guard AXValueGetValue(posRef as! AXValue, .cgPoint, &pos),
          AXValueGetValue(sizeRef as! AXValue, .cgSize, &size) else { return nil }
    return CGRect(origin: pos, size: size)
}

/// Sets the largest AX window of the app that owns the chosen window (same
/// choice as key/type) to `rect` (global top-left points) and prints the frame
/// read back. Retries until the read-back SIZE matches within 1 pt, else exits
/// 2 at the deadline (the app refused or the screen is too small). The origin
/// is requested but not enforced: macOS keeps windows below the menu bar, which
/// is taller on displays with a camera notch, so it may move the window down;
/// the printed x/y are where it actually is.
func setFrame(pids: Set<pid_t>, rect: CGRect, deadline: UInt64) async {
    let owner = await findWindowOwner(pids: pids, deadline: deadline)
    let app = AXUIElementCreateApplication(owner)
    var last: CGRect? = nil
    // With --window-index the AX window is the one whose frame is the chosen
    // window's bounds; chosen once, then followed as it moves.
    var picked: AXUIElement? = nil
    while true {
        var ref: CFTypeRef?
        if AXUIElementCopyAttributeValue(app, kAXWindowsAttribute as CFString, &ref) == .success,
           let windows = ref as? [AXUIElement] {
            let sized = windows.compactMap { w in axFrame(w).map { (w, $0) } }
            if windowIndex != nil && picked == nil {
                picked = axWindow(pids: pids, in: sized)
            }
            let target: AXUIElement?
            if windowIndex != nil {
                target = picked
            } else {
                target = sized.max(by: {
                    $0.1.width * $0.1.height < $1.1.width * $1.1.height
                })?.0
            }
            if let window = target {
                var origin = rect.origin, size = rect.size
                if let pv = AXValueCreate(.cgPoint, &origin), let sv = AXValueCreate(.cgSize, &size) {
                    // Position, size, position again: an app may clamp the
                    // size against the old position.
                    AXUIElementSetAttributeValue(window, kAXPositionAttribute as CFString, pv)
                    AXUIElementSetAttributeValue(window, kAXSizeAttribute as CFString, sv)
                    AXUIElementSetAttributeValue(window, kAXPositionAttribute as CFString, pv)
                }
                try? await Task.sleep(nanoseconds: 150_000_000)
                last = axFrame(window)
                if let got = last, abs(got.width - rect.width) <= 1,
                   abs(got.height - rect.height) <= 1 {
                    emit(["x": got.minX, "y": got.minY, "w": got.width, "h": got.height,
                          "pid": owner])
                    return
                }
            }
        }
        if nowNs() >= deadline {
            let got = last.map { "\($0)" } ?? "unreadable"
            fail("timeout: could not set window of pid \(owner) to \(rect) (last \(got))",
                 code: 2)
        }
        try? await Task.sleep(nanoseconds: 100_000_000)
    }
}

// MARK: - display

/// Main display geometry and refresh rate; needs no permission.
func displayInfo() {
    let id = CGMainDisplayID()
    let bounds = CGDisplayBounds(id)
    let mode = CGDisplayCopyDisplayMode(id)
    let pixelWidth = mode?.pixelWidth ?? Int(bounds.width)
    let pixelHeight = mode?.pixelHeight ?? Int(bounds.height)
    emit(["width": bounds.width, "height": bounds.height,
          "pixel_width": pixelWidth, "pixel_height": pixelHeight,
          "scale": bounds.width > 0 ? Double(pixelWidth) / Double(bounds.width) : 0,
          "refresh_hz": refreshHz(display: id),
          "mode_refresh_hz": mode?.refreshRate ?? 0])
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

/// Owner pid of the largest on-screen layer-0 window (>= 100x100 pt) owned
/// by any of `pids` — the same choice as `findWindow`, but read from
/// CGWindowListCopyWindowInfo, whose owner pid, layer and bounds need no
/// Screen Recording (only window titles do, and they are not used).
func findWindowOwner(pids: Set<pid_t>, deadline: UInt64) async -> pid_t {
    while true {
        if let best = largestWindow(where: { pids.contains($0) }) { return best.pid }
        if nowNs() >= deadline {
            fail("timeout: no on-screen window for pid \(pids.sorted())", code: 2)
        }
        try? await Task.sleep(nanoseconds: 50_000_000)
    }
}

/// On-screen windows, front to back (CGWindowListCopyWindowInfo order), as
/// (owner pid, layer, alpha, bounds in global top-left points).
func onScreenWindows() -> [WindowInfo] {
    let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements],
                                          kCGNullWindowID) as? [[String: Any]] ?? []
    return list.compactMap { info in
        guard let owner = (info[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value,
              let layer = (info[kCGWindowLayer as String] as? NSNumber)?.intValue,
              let boundsDict = info[kCGWindowBounds as String] as? NSDictionary,
              let bounds = CGRect(dictionaryRepresentation: boundsDict as CFDictionary)
        else { return nil }
        let alpha = (info[kCGWindowAlpha as String] as? NSNumber)?.doubleValue ?? 1
        return (owner, layer, alpha, bounds)
    }
}

typealias WindowInfo = (pid: pid_t, layer: Int, alpha: Double, bounds: CGRect)

/// Largest on-screen layer-0 window (>= 100x100 pt) whose owner passes `accept`.
func largestWindow(where accept: (pid_t) -> Bool) -> (pid: pid_t, bounds: CGRect)? {
    largestWindow(in: onScreenWindows(), where: accept)
}

func largestWindow(in list: [WindowInfo],
                   where accept: (pid_t) -> Bool) -> (pid: pid_t, bounds: CGRect)? {
    var best: (pid: pid_t, bounds: CGRect)? = nil
    for w in list where w.layer == 0 && w.bounds.width >= 100
        && w.bounds.height >= 100 && accept(w.pid) {
        if best == nil || w.bounds.width * w.bounds.height
            > best!.bounds.width * best!.bounds.height {
            best = (w.pid, w.bounds)
        }
    }
    return best
}

/// --window-index, parsed once (frame and scroll only).
let windowIndex: Int? = option("window-index").map { _ in intOption("window-index", min: 0) }

/// The `index`-th on-screen layer-0 window (>= 100x100 pt) of `pids`,
/// oldest first by CGWindowID; nil while there are not that many.
func indexedWindow(pids: Set<pid_t>, index: Int)
    -> (id: CGWindowID, pid: pid_t, bounds: CGRect)? {
    let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements],
                                          kCGNullWindowID) as? [[String: Any]] ?? []
    let mine: [(id: CGWindowID, pid: pid_t, bounds: CGRect)] = list.compactMap { info in
        guard let owner = (info[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value,
              pids.contains(owner),
              let layer = (info[kCGWindowLayer as String] as? NSNumber)?.intValue, layer == 0,
              let id = (info[kCGWindowNumber as String] as? NSNumber)?.uint32Value,
              let boundsDict = info[kCGWindowBounds as String] as? NSDictionary,
              let bounds = CGRect(dictionaryRepresentation: boundsDict as CFDictionary),
              bounds.width >= 100, bounds.height >= 100
        else { return nil }
        return (id, owner, bounds)
    }.sorted { $0.id < $1.id }
    return index < mine.count ? mine[index] : nil
}

/// AX windows of `pid` with their frames (empty when unreadable).
func axWindows(pid: pid_t?) -> [(AXUIElement, CGRect)] {
    guard let pid else { return [] }
    var ref: CFTypeRef?
    guard AXUIElementCopyAttributeValue(AXUIElementCreateApplication(pid),
                                        kAXWindowsAttribute as CFString, &ref) == .success,
          let windows = ref as? [AXUIElement] else { return [] }
    return windows.compactMap { w in axFrame(w).map { (w, $0) } }
}

/// The AX window among `sized` whose frame is the bounds of window
/// #`windowIndex` of `pids` (within 2 pt), nil if none matches.
func axWindow(pids: Set<pid_t>, in sized: [(AXUIElement, CGRect)]) -> AXUIElement? {
    guard let index = windowIndex, let chosen = indexedWindow(pids: pids, index: index) else {
        return nil
    }
    let b = chosen.bounds
    return sized.first(where: {
        abs($0.1.minX - b.minX) <= 2 && abs($0.1.minY - b.minY) <= 2
            && abs($0.1.width - b.width) <= 2 && abs($0.1.height - b.height) <= 2
    })?.0
}

/// CGWindowID of the topmost visible window containing `point`, with the
/// same skips as `topmostWindowOwner`.
func topmostWindowID(at point: CGPoint) -> CGWindowID? {
    let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements],
                                          kCGNullWindowID) as? [[String: Any]] ?? []
    let menuLevel = Int(CGWindowLevelForKey(.mainMenuWindow))
    for info in list {
        guard let layer = (info[kCGWindowLayer as String] as? NSNumber)?.intValue,
              let id = (info[kCGWindowNumber as String] as? NSNumber)?.uint32Value,
              let boundsDict = info[kCGWindowBounds as String] as? NSDictionary,
              let bounds = CGRect(dictionaryRepresentation: boundsDict as CFDictionary)
        else { continue }
        let alpha = (info[kCGWindowAlpha as String] as? NSNumber)?.doubleValue ?? 1
        if alpha > 0 && layer < menuLevel && bounds.contains(point) { return id }
    }
    return nil
}

/// Owner of the topmost visible window containing `point` (global top-left
/// points). Windows that are fully transparent (alpha 0) and system overlays
/// at or above the main-menu level (menu bar, status items, cursor) are
/// skipped: they do not take wheel events over an editor's centre.
func topmostWindowOwner(at point: CGPoint) -> pid_t? {
    topmostWindowOwner(in: onScreenWindows(), at: point)
}

/// Same, over an already fetched front-to-back window list.
func topmostWindowOwner(in list: [WindowInfo], at point: CGPoint) -> pid_t? {
    let menuLevel = Int(CGWindowLevelForKey(.mainMenuWindow))
    for w in list where w.alpha > 0 && w.layer < menuLevel {
        if w.bounds.contains(point) { return w.pid }
    }
    return nil
}

/// Executable path of `pid` (proc_pidpath), nil when it is gone.
func executablePath(_ pid: pid_t) -> String? {
    var buf = [CChar](repeating: 0, count: 4096)
    guard proc_pidpath(pid, &buf, UInt32(buf.count)) > 0 else { return nil }
    return String(cString: buf)
}

/// Matches window owners by executable: `exe` itself, or any process inside
/// the same app bundle's Contents/ (helper processes), minus `excluded`
/// (instances that were running before the launch, e.g. the user's own).
final class ExecutableMatcher {
    let exe: String
    let contents: String?
    let excluded: Set<pid_t>
    private var cache: [pid_t: Bool] = [:]

    init(exe raw: String, excluded: Set<pid_t>) {
        exe = URL(fileURLWithPath: raw).resolvingSymlinksInPath().path
        if let r = exe.range(of: ".app/Contents/", options: .backwards) {
            contents = String(exe[..<r.upperBound])
        } else {
            contents = nil
        }
        self.excluded = excluded
    }

    func matches(_ pid: pid_t) -> Bool {
        if excluded.contains(pid) { return false }
        if let hit = cache[pid] { return hit }
        var hit = false
        if let path = executablePath(pid) {
            hit = path == exe || (contents.map { path.hasPrefix($0) } ?? false)
        }
        cache[pid] = hit
        return hit
    }
}

/// Finds the window among `pids`, brings its owning app frontmost
/// (NSRunningApplication.activate; key events posted to a pid go to its key
/// window, which only takes input when the app is active) and returns the
/// owner pid to post to.
func activateOwner(pids: Set<pid_t>, deadline: UInt64) async -> pid_t {
    let owner = await findWindowOwner(pids: pids, deadline: deadline)
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
func typeText(pids: Set<pid_t>, keys: [KeyStroke], intervalMs: Int,
              deadline: UInt64) async {
    let pid = await activateOwner(pids: pids, deadline: deadline)
    var times: [UInt64] = []
    for (i, k) in keys.enumerated() {
        if i > 0 { usleep(UInt32(intervalMs) * 1000) }
        times.append(nowNs())
        post(k, to: pid)
    }
    emit(["t_ns": times])
}

func keyCombo(pids: Set<pid_t>, stroke: KeyStroke, deadline: UInt64) async {
    let pid = await activateOwner(pids: pids, deadline: deadline)
    let t = nowNs()
    post(stroke, to: pid)
    emit(["t_ns": t])
}

// MARK: - scroll

enum ScrollUnit { case line, pixel(Int32) }

/// Sends --lines scroll-down events at --rate per second (default: the
/// display refresh rate) to the window's center while capturing it. The
/// pointer is moved to the window center first because the window server
/// routes wheel events to the window under the pointer.
///
/// --unit line (default): each event is one wheel line
/// (kCGScrollEventUnitLine), like a notched mouse wheel.
/// --unit pixel: --lines counts events; each is a continuous (trackpad-like)
/// pixel event of --pixels-per-event (default 40) with
/// kCGScrollWheelEventIsContinuous = 1 and kCGScrollWheelEventScrollPhase
/// began on the first event, changed on the rest, and one extra zero-delta
/// ended event after the last (no momentum phase).
///
/// frames = new frames from the first event to 250 ms after the last.
/// dropped = gaps between consecutive new frames, starting while events are
/// still being sent, longer than 1.5 × max(refresh period, event period).
/// The brief says 1.5 × refresh period; with --rate below the refresh rate
/// every gap would count, so the event period is the floor.
/// interval_ms_p50/p90/max are those same gaps (the ones `dropped` tests);
/// fps = new frames inside the send window / its length; events_sent counts
/// the scroll events posted (the ended event excluded). --dump-dir writes
/// intervals.txt (one gap in ms per line).
func scroll(pids: Set<pid_t>, lines: Int, rateOption: Double?, unit: ScrollUnit,
            dumpDir: String?, timeout: (seconds: Double, deadline: UInt64)) async {
    let window = await findWindow(pids: pids, deadline: timeout.deadline)
    let hz = refreshHz(for: window)
    let rate = rateOption ?? Double(hz)
    let center = CGPoint(x: window.frame.midX, y: window.frame.midY)
    // Wheel events go to the window under the pointer: bring the editor
    // frontmost, then refuse unless its window is the topmost one there.
    _ = await activateOwner(pids: pids, deadline: timeout.deadline)
    if windowIndex != nil {
        // One process, several windows: raise the chosen one, then require
        // that very window (not just its pid) on top at its centre.
        if let ax = axWindow(pids: pids, in: axWindows(pid: window.owningApplication?.processID)) {
            AXUIElementPerformAction(ax, kAXRaiseAction as CFString)
            try? await Task.sleep(nanoseconds: 200_000_000)
        }
        guard topmostWindowID(at: center) == window.windowID else {
            fail("another window is on top at the centre \(center) of window #"
                 + "\(windowIndex!) of pid \(pids.sorted()); no scroll events sent", code: 1)
        }
    }
    guard let top = topmostWindowOwner(at: center), pids.contains(top) else {
        let who = topmostWindowOwner(at: center).map { "pid \($0)" } ?? "no window"
        fail("another window (\(who)) is on top at the centre \(center) of the window of "
             + "pid \(pids.sorted()); no scroll events sent", code: 1)
    }
    if let dumpDir { makeDumpDir(dumpDir) }
    let lock = NSLock()
    var stamps: [UInt64] = []
    var sent = 0
    let capture = Capture(window: window, hz: hz) { f in
        lock.withLock { stamps.append(f.tNs) }
        return false
    }
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
        var posted = 0
        for i in 0..<lines {
            let target = start + UInt64(i) * periodNs
            while nowNs() < target { usleep(200) }
            let e: CGEvent?
            switch unit {
            case .line:
                e = CGEvent(scrollWheelEvent2Source: nil, units: .line, wheelCount: 1,
                            wheel1: -1, wheel2: 0, wheel3: 0)
            case .pixel(let px):
                e = CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 1,
                            wheel1: -px, wheel2: 0, wheel3: 0)
                e?.setIntegerValueField(.scrollWheelEventIsContinuous, value: 1)
                e?.setIntegerValueField(.scrollWheelEventScrollPhase,
                                        value: Int64(i == 0 ? CGScrollPhase.began.rawValue
                                                            : CGScrollPhase.changed.rawValue))
            }
            if let e {
                e.location = center
                e.post(tap: .cghidEventTap)
                posted += 1
            }
        }
        if case .pixel = unit {
            usleep(UInt32(periodNs / 1000))
            if let e = CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 1,
                               wheel1: 0, wheel2: 0, wheel3: 0) {
                e.setIntegerValueField(.scrollWheelEventIsContinuous, value: 1)
                e.setIntegerValueField(.scrollWheelEventScrollPhase,
                                       value: Int64(CGScrollPhase.ended.rawValue))
                e.location = center
                e.post(tap: .cghidEventTap)
            }
        }
        lock.withLock { sendStart = start; sendEnd = nowNs(); sent = posted }
    }
    sender.start()
    let stopAt = nowNs() + UInt64((sendDuration + 3.5) * 1e9)
    _ = await capture.start(deadline: min(max(stopAt, nowNs()), max(timeout.deadline, nowNs())))
    while !lock.withLock({ sendEnd != 0 }) {
        if nowNs() >= timeout.deadline { fail("timeout while sending scroll events", code: 2) }
        try? await Task.sleep(nanoseconds: 10_000_000)
    }

    let (window0, window1, all, eventsSent) = lock.withLock {
        (sendStart, sendEnd, stamps.sorted(), sent)
    }
    let counted = all.filter { $0 >= window0 && $0 <= window1 + 250_000_000 }
    let limit = 1.5 * max(1e9 / Double(hz), 1e9 / rate)
    var dropped = 0
    var gapsMs: [Double] = []
    for (a, b) in zip(counted, counted.dropFirst()) where a <= window1 {
        if Double(b - a) > limit { dropped += 1 }
        gapsMs.append(Double(b - a) / 1e6)
    }
    let inSend = counted.filter { $0 <= window1 }.count
    let sendSeconds = Double(window1 - window0) / 1e9
    var result: [String: Any] = ["frames": counted.count, "dropped": dropped, "refresh_hz": hz,
                                 "rate": rate, "events_sent": eventsSent,
                                 "fps": sendSeconds > 0 ? Double(inSend) / sendSeconds : 0]
    result.merge(intervalStats(gapsMs)) { a, _ in a }
    if let dumpDir { writeLines(gapsMs.map { "\($0)" }, to: dumpDir + "/intervals.txt") }
    emit(result)
}

// MARK: - record

/// Manual-scroll baseline: captures the largest window of --pid for
/// --seconds while a person interacts with it (nothing is sent), then
/// prints the gaps between consecutive new-content frames: frames,
/// refresh_hz, seconds, fps (new frames / seconds) and
/// interval_ms_p50/p90/max. "ready" goes to stderr when the first frame has
/// arrived (start scrolling then) and "done" when the capture ends. Pauses
/// show up as long gaps, so keep scrolling for the whole recording.
/// --dump-dir writes intervals.txt (one gap in ms per line).
func record(pids: Set<pid_t>, seconds: Double, dumpDir: String?,
            timeout: (seconds: Double, deadline: UInt64)) async {
    let window = await findWindow(pids: pids, deadline: timeout.deadline)
    let hz = refreshHz(for: window)
    if let dumpDir { makeDumpDir(dumpDir) }
    let lock = NSLock()
    var stamps: [UInt64] = []
    let capture = Capture(window: window, hz: hz) { f in
        lock.withLock { stamps.append(f.tNs) }
        return false
    }
    _ = await capture.start(deadline: nowNs() + UInt64(seconds * 1e9))
    note("done")
    let all = lock.withLock { stamps.sorted() }
    let gapsMs = zip(all, all.dropFirst()).map { Double($1 - $0) / 1e6 }
    var result: [String: Any] = ["frames": all.count, "refresh_hz": hz, "seconds": seconds,
                                 "fps": Double(all.count) / seconds]
    result.merge(intervalStats(gapsMs)) { a, _ in a }
    if let dumpDir { writeLines(gapsMs.map { "\($0)" }, to: dumpDir + "/intervals.txt") }
    emit(result)
}

// MARK: - Dispatch (parse, then check permissions, then measure)

if windowIndex != nil && command != "frame" && command != "scroll" {
    fail("--window-index is for frame and scroll only", code: 1)
}

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
    requirePermissions(screen: true, accessibility: false)
    await sample(pids: pids, timeout: timeout)
case "watch":
    let display = flag("display")
    // --display finds the window by executable; --pid by owner pid.
    if display && option("pid") != nil { fail("--display and --pid are exclusive", code: 1) }
    let pids = display ? [] : pidsOption()
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
    if display {
        let exe = required("exe")
        guard exe.hasPrefix("/") else { fail("--exe must be an absolute path", code: 1) }
        guard case .coverage(let threshold) = mode else {
            fail("watch --display supports only --until coverage:<0..1>", code: 1)
        }
        guard let bg = background else { fail("watch --display needs --background", code: 1) }
        if option("baseline") != nil { fail("watch --display takes no --baseline", code: 1) }
        if option("min-change") != nil || option("dump-dir") != nil {
            fail("watch --display takes no --min-change / --dump-dir", code: 1)
        }
        if flag("hold-ms") && option("hold-ms") == nil {
            fail("--hold-ms needs a number of milliseconds", code: 1)
        }
        let holdMs = intOption("hold-ms", default: 0, min: 0)
        if holdMs > 60_000 { fail("--hold-ms must be at most 60000", code: 1) }
        let allowResize = flag("allow-resize")
        if allowResize && holdMs <= 0 { fail("--allow-resize needs --hold-ms > 0", code: 1) }
        let excluded = option("exclude-pids").map { pidList("exclude-pids", $0) } ?? []
        let timeout = deadlineOption()
        requirePermissions(screen: true, accessibility: false)
        await watchDisplay(matcher: ExecutableMatcher(exe: exe, excluded: excluded),
                           since: since, threshold: threshold, background: bg,
                           holdMs: holdMs, allowResize: allowResize, timeout: timeout)
    } else {
        if flag("hold-ms") { fail("--hold-ms needs watch --display", code: 1) }
        if flag("allow-resize") { fail("--allow-resize needs watch --display", code: 1) }
        let minChange = fractionOption("min-change") ?? 0.005
        let dumpDir = dumpDirOption()
        if case .coverage = mode, option("min-change") != nil || dumpDir != nil {
            fail("--min-change and --dump-dir need --until change", code: 1)
        }
        let timeout = deadlineOption()
        requirePermissions(screen: true, accessibility: false)
        await watch(pids: pids, since: since, mode: mode, until: until, background: background,
                    baseline: option("baseline"), minChange: minChange, dumpDir: dumpDir,
                    timeout: timeout)
    }
case "frame":
    let pids = pidsOption()
    let x = intOption("x"), y = intOption("y")
    let w = intOption("w", min: 100), h = intOption("h", min: 100)
    let timeout = deadlineOption(default: 10)
    requirePermissions(screen: false, accessibility: true)
    await setFrame(pids: pids, rect: CGRect(x: x, y: y, width: w, height: h),
                   deadline: timeout.deadline)
case "display":
    displayInfo()
case "scroll":
    let pids = pidsOption()
    let lines = intOption("lines", min: 1)
    let rate = positiveOption("rate")
    let unit: ScrollUnit
    switch option("unit") ?? "line" {
    case "line":
        if option("pixels-per-event") != nil {
            fail("--pixels-per-event needs --unit pixel", code: 1)
        }
        unit = .line
    case "pixel":
        let px = intOption("pixels-per-event", default: 40, min: 1)
        guard px <= 10_000 else { fail("--pixels-per-event must be at most 10000", code: 1) }
        unit = .pixel(Int32(px))
    default:
        fail("--unit must be line or pixel", code: 1)
    }
    let dumpDir = dumpDirOption()
    let timeout = deadlineOption()
    requirePermissions(screen: true, accessibility: true)
    await scroll(pids: pids, lines: lines, rateOption: rate, unit: unit, dumpDir: dumpDir,
                 timeout: timeout)
case "record":
    let pids = pidsOption()
    guard let seconds = positiveOption("seconds") else { fail("missing --seconds\n" + usage, code: 1) }
    let dumpDir = dumpDirOption()
    let timeout = deadlineOption(default: 10)
    requirePermissions(screen: true, accessibility: false)
    await record(pids: pids, seconds: seconds, dumpDir: dumpDir, timeout: timeout)
case "type":
    let pids = pidsOption()
    let keys = strokes(for: required("text"))
    let interval = intOption("interval-ms", default: 100, min: 0)
    let timeout = deadlineOption(default: 10)
    requirePermissions(screen: false, accessibility: true)
    await typeText(pids: pids, keys: keys, intervalMs: interval, deadline: timeout.deadline)
case "activate":
    let pids = pidsOption()
    let timeout = deadlineOption(default: 10)
    requirePermissions(screen: false, accessibility: true)
    let owner = await activateOwner(pids: pids, deadline: timeout.deadline)
    emit(["pid": owner, "active": NSRunningApplication(processIdentifier: owner)?.isActive ?? false])
case "key":
    let pids = pidsOption()
    let stroke = comboStroke(required("combo"))
    let timeout = deadlineOption(default: 10)
    requirePermissions(screen: false, accessibility: true)
    await keyCombo(pids: pids, stroke: stroke, deadline: timeout.deadline)
default:
    fail("unknown command \(command)\n" + usage, code: 1)
}
exit(0)
