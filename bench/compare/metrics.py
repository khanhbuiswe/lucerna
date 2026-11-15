"""The measurements, taken from outside each editor.

Every metric launches the editor in its bench profile, measures, and quits it
in a `finally`. A metric returns None for a failed round (helper error or
timeout, no window, a suspect frame); a missing Screen Recording/Accessibility
permission raises `PermissionDenied` instead, so a run aborts rather than
recording garbage.

Frame times come from `bench-helper` (README "Helper"); its clock is
CLOCK_UPTIME_RAW, read here with `editors.now_ns()`.

Startup is captured on the whole display, started BEFORE the launch (`watch
--display`), so the first frame of the new window is never missed. Every other
launching metric first sets the window to WINDOW_FRAME (`frame`, through AX), so
all three editors are measured at one window size; startup keeps each editor's
default window size (a first frame cannot be resized).
"""

from __future__ import annotations

import json
import os
import queue
import re
import shutil
import subprocess
import sys
import threading
import time
from pathlib import Path

import editors
from env import BenchPaths

TIMEOUT_S = 60
COVERAGE = 0.3              # marker.txt: full blocks, so the centre fills up
TEXT_COVERAGE = 0.02        # big.rs: plain code covers far less of the centre
SETTLE_S = 1.0              # let a picker or a reload settle
CALIBRATE_SETTLE_S = 3.0    # empty file: let the theme paint before sampling
CALIBRATE_MIN_FRAC = 0.9    # an empty editor's centre is almost all background
OPEN_SETTLE_S = 5.0         # medium repo: window up before Quick Open
SCAN_WAIT_S = 30            # large repo: let the file scan finish
IDLE_S = 60
SCROLL_LINES = 3000
SCROLL_RATE = 120
QUICK_OPEN_QUERY = "mod0042"  # gen-bench-repo.sh: directory pkg00/mod0042/
# Fixed window frame in points (top-left origin) for every launching metric but
# startup. macOS may move the window below a taller (notch) menu bar; the size
# is what the helper enforces.
WINDOW_FRAME = {"x": 0, "y": 25, "w": 1280, "h": 800}

# Permissions each bench-helper subcommand needs (README "Helper") and the
# subcommands each metric runs. run.py derives its permission checks from these
# two tables; tests check every metric's calls against METRIC_HELPERS.
HELPER_PERMISSIONS = {
    "now": (), "check": (), "display": (),
    "sample": ("screen",),
    "watch": ("screen",),
    "scroll": ("screen", "accessibility"),
    "key": ("accessibility",),
    "type": ("accessibility",),
    "frame": ("accessibility",),
    "activate": ("accessibility",),
}
_IDLE_HELPERS = ("frame", "key", "type")
METRIC_HELPERS = {
    # sample: background calibration; activate: bring the new window forward
    "startup_ms": ("sample", "watch", "activate"),
    "file_change_ms": ("sample", "frame", "watch"),
    "quick_open_ms": ("frame", "key", "type", "watch"),
    "idle_ram_mb": _IDLE_HELPERS,
    "idle_cpu_pct": _IDLE_HELPERS,
    "scroll_dropped_pct": ("sample", "frame", "watch", "scroll"),
    "app_size_mb": (),
}


def metric_permissions(metric: str) -> set:
    """{"screen", "accessibility"} subset that `metric`'s helper calls need."""
    return {p for cmd in METRIC_HELPERS[metric] for p in HELPER_PERMISSIONS[cmd]}

# Ten real .rs files of ripgrep 14.1.1 (`git ls-tree -r 14.1.1`), opened with
# Quick Open in every editor: `luc` takes a single path and has no way to
# forward more to a running instance, so the CLI cannot open ten tabs there.
IDLE_FILES = (
    "crates/core/main.rs",
    "crates/core/search.rs",
    "crates/core/flags/defs.rs",
    "crates/core/flags/hiargs.rs",
    "crates/globset/src/glob.rs",
    "crates/ignore/src/walk.rs",
    "crates/ignore/src/gitignore.rs",
    "crates/printer/src/standard.rs",
    "crates/regex/src/matcher.rs",
    "crates/searcher/src/lib.rs",
)

_MAIN_BINARY = editors.MAIN_BINARY  # window owner; launchers carry no window


class PermissionDenied(RuntimeError):
    """bench-helper exit 3: Screen Recording or Accessibility not granted."""


def _log(message: str) -> None:
    print(f"metrics: {message}", file=sys.stderr, flush=True)


def helper_path(paths: BenchPaths) -> str:
    return os.environ.get("BENCH_HELPER") or str(paths.root / "bin" / "bench-helper")


def _result(rc: int, stdout: str, stderr: str, args) -> dict | None:
    if rc == 3:
        raise PermissionDenied(f"bench-helper {args[0]}: {stderr.strip()}")
    if rc != 0:
        _log(f"helper {args[0]} exit {rc}: {stderr.strip()}")
        return None
    lines = stdout.strip().splitlines()
    try:
        return json.loads(lines[-1])
    except (IndexError, ValueError):
        _log(f"helper {args[0]}: no JSON on stdout")
        return None


def _helper(paths: BenchPaths, args: list[str], timeout: float = TIMEOUT_S) -> dict | None:
    """Run one helper command; None on any failure but a missing permission."""
    try:
        r = subprocess.run([helper_path(paths), *args], capture_output=True,
                           text=True, timeout=timeout + 10)
    except subprocess.TimeoutExpired:
        _log(f"helper {args[0]} hung past {timeout} s")
        return None
    return _result(r.returncode, r.stdout, r.stderr, args)


class _Watch:
    def __init__(self, popen, args, errors):
        self._p, self._args, self._errors = popen, args, errors

    def result(self, timeout: float = TIMEOUT_S) -> dict | None:
        # stderr belongs to the pump thread; stdout is one short JSON line.
        try:
            self._p.wait(timeout=timeout + 10)
        except subprocess.TimeoutExpired:
            self._p.kill()
            self._p.wait()
            _log("helper watch hung")
            return None
        finally:
            out = "" if self._p.stdout.closed else self._p.stdout.read()
            self._p.stdout.close()
        return _result(self._p.returncode, out, "".join(self._errors), self._args)

    def close(self) -> None:
        """Kill the watch if it is still running (a metric bailed out early)."""
        if self._p.poll() is None:
            self._p.kill()
            self._p.wait()
        self._p.stdout.close()


def _start_watch(paths: BenchPaths, args: list[str], timeout: float = TIMEOUT_S):
    """Start `watch` and return once it printed `ready` (reference stored)."""
    p = subprocess.Popen([helper_path(paths), *args], stdout=subprocess.PIPE,
                         stderr=subprocess.PIPE, text=True)
    errors: list[str] = []
    ready: queue.Queue = queue.Queue()

    def pump():
        for line in p.stderr:
            errors.append(line)
            if line.strip() == "ready":
                ready.put(True)
        p.stderr.close()
        ready.put(False)

    threading.Thread(target=pump, daemon=True).start()
    try:
        ok = ready.get(timeout=timeout)
    except queue.Empty:
        ok = False
    if ok:
        return _Watch(p, args, errors)
    if p.poll() is None:
        p.kill()
    p.wait()
    p.stdout.close()
    _result(p.returncode if p.returncode not in (None, 0) else 1, "", "".join(errors), args)
    return None


def _csv(pids) -> str:
    return ",".join(str(p) for p in pids)


def _left(deadline: float) -> int:
    return max(1, int(deadline - time.monotonic()))


def _owner_pids(editor: str, paths: BenchPaths, deadline: float) -> list[int] | None:
    """All bench pids, once the process that owns the window (the main binary,
    not a `--type=` helper) exists."""
    while time.monotonic() < deadline:
        pids = editors.bench_pids(editor, paths)
        if pids and editor == "lucerna":
            return pids
        if pids:
            out = subprocess.run(["ps", "-o", "command=", "-p", _csv(pids)],
                                 capture_output=True, text=True).stdout
            main = _MAIN_BINARY[editor]
            if any(c.startswith(main) and "--type=" not in c
                   for c in out.splitlines()):
                return editors.bench_pids(editor, paths)
        time.sleep(0.05)
    _log(f"{editor}: no window process within {TIMEOUT_S} s")
    if editor == "zed":
        stray = editors.foreign_instances("zed", paths)
        if stray:
            _log(f"ERROR: new Zed pid {', '.join(map(str, stray))} appeared without "
                 "--user-data-dir: it uses your REAL profile, not the bench profile. "
                 "Not closing it automatically; close it yourself.")
    return None


# The frame `frame` actually set, per editor (last one); run.py records it.
LAST_FRAME: dict = {}


def _set_frame(editor: str, paths: BenchPaths, deadline: float) -> bool:
    """WINDOW_FRAME through AX, once the window is up; False fails the round
    (a measurement at another size would not be comparable)."""
    f = WINDOW_FRAME
    out = _helper(paths, ["frame", "--pid", _csv(editors.bench_pids(editor, paths)),
                          "--x", str(f["x"]), "--y", str(f["y"]),
                          "--w", str(f["w"]), "--h", str(f["h"]),
                          "--timeout", str(_left(deadline))])
    if out is None:
        return False
    LAST_FRAME[editor] = {k: out[k] for k in ("x", "y", "w", "h") if k in out}
    return True


class _Activator:
    """Brings the new window's app forward from a background thread, once its
    owner process exists, so a window opened behind another one is uncovered
    without delaying the display watch. A PermissionDenied is re-raised by
    `finish`; other failures only cost the round its chance to be uncovered
    (an occluded window delays the measurement, it cannot fake it)."""

    def __init__(self, editor: str, paths: BenchPaths, deadline: float):
        self.error = None
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._run, args=(editor, paths, deadline),
                                        daemon=True)
        self._thread.start()

    def _run(self, editor, paths, deadline):
        try:
            pids = _owner_pids(editor, paths, deadline)
            if pids and not self._stop.is_set():
                _helper(paths, ["activate", "--pid", _csv(pids), "--timeout", "10"])
        except Exception as exc:  # reported by finish()
            self.error = exc

    def finish(self) -> None:
        self._stop.set()
        self._thread.join(timeout=TIMEOUT_S + 15)
        if isinstance(self.error, PermissionDenied):
            raise self.error


def _shown(editor, paths, bg, since_ns, coverage, deadline) -> bool:
    """Wait until the window's centre shows content (coverage of non-background).
    `bg` is fetched before launch: calibrating mid-session would quit it."""
    out = _helper(paths, ["watch", "--pid", _csv(editors.bench_pids(editor, paths)),
                          "--since", str(since_ns), "--until", f"coverage:{coverage}",
                          "--background", bg, "--timeout", str(_left(deadline))])
    return out is not None


# Background calibration ---------------------------------------------------

def _calibration_file(paths: BenchPaths) -> Path:
    return paths.root / "calibration.json"


def load_calibration(paths: BenchPaths) -> dict:
    """{"<editor> <version>": "RRGGBB"}; Task 5 puts it in the result meta."""
    try:
        return json.loads(_calibration_file(paths).read_text())
    except (OSError, ValueError):
        return {}


def calibrate(editor: str, paths: BenchPaths) -> str | None:
    """The editor background: centre dominant colour on an empty file."""
    empty = paths.data / "empty.txt"
    empty.write_text("")
    editors.launch(editor, paths, empty)
    try:
        deadline = time.monotonic() + TIMEOUT_S
        if _owner_pids(editor, paths, deadline) is None:
            return None
        time.sleep(CALIBRATE_SETTLE_S)
        out = _helper(paths, ["sample", "--pid", _csv(editors.bench_pids(editor, paths)),
                              "--timeout", str(_left(deadline))])
        if not out:
            return None
        frac = out.get("center_dominant_frac", 0)
        if frac < CALIBRATE_MIN_FRAC:
            # Something other than the empty editor (dialog, welcome page,
            # half-painted window) fills the centre: not a background colour.
            _log(f"{editor}: calibration centre only {frac:.2f} one colour; not cached")
            return None
        return out["center_dominant"]
    finally:
        editors.quit(editor, paths)


def background(editor: str, paths: BenchPaths) -> str | None:
    key = f"{editor} {editors.version(editor)}"
    cache = load_calibration(paths)
    if key not in cache:
        bg = calibrate(editor, paths)
        if bg is None:
            return None
        cache[key] = bg
        _calibration_file(paths).write_text(json.dumps(cache, indent=2) + "\n")
    return cache[key]


# Metrics ------------------------------------------------------------------

def startup_ms(editor: str, paths: BenchPaths) -> float | None:
    """Launch call → first frame whose window centre is ≥ 30 % marker text.

    The display capture starts (and prints `ready`) BEFORE the launch, and
    finds the window by its executable, minus instances already running. t0 is
    unknown when the helper starts, so it gets `--since 0` and reports the
    absolute frame time; the launch time is subtracted here."""
    bg = background(editor, paths)
    if bg is None:
        return None
    exe = editors.main_executable(editor)
    args = ["watch", "--display", "--exe", exe]
    exclude = editors.running_pids(exe)
    if exclude:
        args += ["--exclude-pids", _csv(exclude)]
    args += ["--since", "0", "--until", f"coverage:{COVERAGE}", "--background", bg,
             "--timeout", str(TIMEOUT_S)]
    watch = _start_watch(paths, args, TIMEOUT_S)
    if watch is None:
        return None
    activator = None
    try:
        launched = editors.launch(editor, paths, paths.marker_file)
        activator = _Activator(editor, paths, time.monotonic() + TIMEOUT_S)
        out = watch.result(TIMEOUT_S)
        activator.finish()
        if out is None:
            return None
        if out.get("occluded_frames"):
            _log(f"{editor}: window covered on {out['occluded_frames']} frame(s) during startup")
        if out.get("first_frame_hit") or out["t_ns"] < launched.t0_ns:
            # The window was there before the launch: not this launch's frame.
            _log(f"{editor}: startup content already on the first frame; round dropped")
            return None
        return (out["t_ns"] - launched.t0_ns) / 1e6
    finally:
        watch.close()
        if activator is not None:
            activator._stop.set()
        editors.quit(editor, paths)


def _overwrite_empty(path: Path) -> int:
    t_w = editors.now_ns()
    path.write_text("")
    return t_w


def file_change_ms(editor: str, paths: BenchPaths) -> float | None:
    """File emptied on disk → first changed frame. Uses a per-run copy of the
    marker (`reload.txt`) so the shared marker is never destroyed."""
    bg = background(editor, paths)
    if bg is None:
        return None
    reload = paths.data / "reload.txt"
    shutil.copyfile(paths.marker_file, reload)
    launched = editors.launch(editor, paths, reload)
    watch = None
    try:
        deadline = time.monotonic() + TIMEOUT_S
        if _owner_pids(editor, paths, deadline) is None:
            return None
        if not _set_frame(editor, paths, deadline):
            return None
        if not _shown(editor, paths, bg, launched.t0_ns, COVERAGE, deadline):
            return None
        time.sleep(SETTLE_S)
        watch = _start_watch(paths, ["watch", "--pid", _csv(editors.bench_pids(editor, paths)),
                                     "--since", str(editors.now_ns()), "--until", "change",
                                     "--timeout", str(_left(deadline))], _left(deadline))
        if watch is None:
            return None
        t_w = _overwrite_empty(reload)
        out = watch.result(_left(deadline))
        if out is None:
            return None
        if out["t_ns"] < t_w:
            _log(f"{editor}: frame changed before the file was written; round dropped")
            return None
        return (out["t_ns"] - t_w) / 1e6
    finally:
        if watch:
            watch.close()
        editors.quit(editor, paths)
        shutil.copyfile(paths.marker_file, reload)


def quick_open_ms(editor: str, paths: BenchPaths) -> float | None:
    """Large repo, ⌘P, type `mod0042`: last key → first changed frame."""
    editors.launch(editor, paths, paths.large_repo)
    watch = None
    try:
        deadline = time.monotonic() + TIMEOUT_S
        if _owner_pids(editor, paths, deadline) is None:
            return None
        if not _set_frame(editor, paths, deadline):
            return None
        time.sleep(SCAN_WAIT_S)
        deadline = time.monotonic() + TIMEOUT_S
        pids = _csv(editors.bench_pids(editor, paths))
        if not _quick_open_type(paths, pids, QUICK_OPEN_QUERY[:-1]):
            return None
        time.sleep(SETTLE_S)
        watch = _start_watch(paths, ["watch", "--pid", pids, "--since", str(editors.now_ns()),
                                     "--until", "change", "--timeout", str(_left(deadline))],
                             _left(deadline))
        if watch is None:
            return None
        typed = _helper(paths, ["type", "--pid", pids, "--text", QUICK_OPEN_QUERY[-1]])
        if typed is None:
            return None  # `finally` kills the watch instead of waiting it out
        out = watch.result(_left(deadline))
        if out is None:
            return None
        t_k = typed["t_ns"][-1]
        if out["t_ns"] < t_k:
            _log(f"{editor}: Quick Open changed before the last key; round dropped")
            return None
        return (out["t_ns"] - t_k) / 1e6
    finally:
        if watch:
            watch.close()
        editors.quit(editor, paths)


_UNITS = {"B": 1, "KB": 1024, "MB": 1024 ** 2, "GB": 1024 ** 3}


def parse_footprint(text: str) -> int:
    """Sum of every process's `phys_footprint` in `footprint` output, in bytes
    (handles both `-f bytes` and the default formatted KB/MB/GB, 1024-based)."""
    total = 0
    for value, unit in re.findall(r"phys_footprint:\s+([\d.]+)\s*(B|KB|MB|GB)\b", text):
        total += int(float(value) * _UNITS[unit])
    return total


def _footprint_mb(pids) -> float | None:
    cmd = ["footprint", "-f", "bytes", "--noCategories"]
    for pid in pids:
        cmd += ["-p", str(pid)]
    out = subprocess.run(cmd, capture_output=True, text=True).stdout
    total = parse_footprint(out)
    return total / 1024 ** 2 if total else None


def parse_cputime(text: str) -> float:
    """`ps -o time=` ("[[dd-]hh:]mm:ss.ss", minutes may exceed 59) → seconds."""
    days = 0
    if "-" in text:
        d, text = text.split("-", 1)
        days = int(d)
    total = 0.0
    for part in text.strip().split(":"):
        total = total * 60 + float(part)
    return days * 86400 + total


def _cpu_times(pids) -> dict:
    """{pid: cumulative CPU seconds} of the live ones among `pids`."""
    out = subprocess.run(["ps", "-o", "pid=,time=", "-p", _csv(pids)],
                         capture_output=True, text=True).stdout
    times = {}
    for line in out.splitlines():
        parts = line.split()
        if len(parts) == 2 and parts[0].isdigit():
            try:
                times[int(parts[0])] = parse_cputime(parts[1])
            except ValueError:
                pass
    return times


def _cpu_pct(editor: str, paths: BenchPaths) -> float | None:
    """CPU % over an IDLE_S window from cumulative CPU time (`ps -o time=`),
    not from `%cpu`, which is a decaying average. Sampled every second with the
    bench pids refreshed each time: a process present at the first sample
    counts from its time then, a later one from 0; a process that exits counts
    up to its last sample. 100 % = one core."""
    first: dict = {}
    last: dict = {}
    start = end = None
    for i in range(IDLE_S + 1):
        pids = editors.bench_pids(editor, paths)
        now = time.monotonic()
        times = _cpu_times(pids) if pids else {}
        if start is None:
            start, first = now, dict(times)
        last.update(times)
        end = now
        if i < IDLE_S:
            time.sleep(1)
    elapsed = end - start
    if not last or elapsed <= 0:
        return None
    used = sum(max(0.0, v - first.get(pid, 0.0)) for pid, v in last.items())
    return used / elapsed * 100


def _quick_open_type(paths, pids: str, text: str) -> bool:
    """⌘P, let the picker appear (so no leading key is lost), type `text`.
    Shared by quick_open_ms and idle_session so both open it identically."""
    if _helper(paths, ["key", "--pid", pids, "--combo", "cmd+p"]) is None:
        return False
    time.sleep(SETTLE_S / 2)
    return _helper(paths, ["type", "--pid", pids, "--text", text]) is not None


def _open_with_quick_open(editor, paths, relative: str) -> bool:
    pids = _csv(editors.bench_pids(editor, paths))
    if not _quick_open_type(paths, pids, relative):
        return False
    time.sleep(SETTLE_S)
    if _helper(paths, ["type", "--pid", pids, "--text", "\n"]) is None:
        return False
    time.sleep(SETTLE_S)
    return True


def idle_session(editor: str, paths: BenchPaths) -> tuple[float | None, float | None]:
    """Medium repo with IDLE_FILES open, idle IDLE_S, then (RAM MB, mean CPU %)
    from one launch, so run.py can record both from the same session."""
    editors.launch(editor, paths, paths.medium_repo)
    try:
        deadline = time.monotonic() + TIMEOUT_S
        if _owner_pids(editor, paths, deadline) is None:
            return None, None
        if not _set_frame(editor, paths, deadline):
            return None, None
        time.sleep(OPEN_SETTLE_S)
        for relative in IDLE_FILES:
            if not _open_with_quick_open(editor, paths, relative):
                return None, None
        time.sleep(IDLE_S)
        pids = editors.bench_pids(editor, paths)
        if not pids:
            return None, None
        ram = _footprint_mb(pids)
        cpu = _cpu_pct(editor, paths)
        return ram, cpu
    finally:
        editors.quit(editor, paths)


def idle_ram_mb(editor: str, paths: BenchPaths) -> float | None:
    """Runs a full idle_session (launch, 10 files, 60 s idle + 60 s CPU
    sampling) for RAM alone; run.py should call idle_session once instead."""
    return idle_session(editor, paths)[0]


def idle_cpu_pct(editor: str, paths: BenchPaths) -> float | None:
    """Runs a full idle_session for CPU alone; run.py should call
    idle_session once to get RAM and CPU from the same launch."""
    return idle_session(editor, paths)[1]


def scroll_dropped_pct(editor: str, paths: BenchPaths) -> float | None:
    """big.rs: 3000 one-line scroll events at 120/s; dropped frames in %."""
    bg = background(editor, paths)
    if bg is None:
        return None
    launched = editors.launch(editor, paths, paths.big_file)
    try:
        deadline = time.monotonic() + TIMEOUT_S
        if _owner_pids(editor, paths, deadline) is None:
            return None
        if not _set_frame(editor, paths, deadline):
            return None
        if not _shown(editor, paths, bg, launched.t0_ns, TEXT_COVERAGE, deadline):
            return None
        time.sleep(SETTLE_S)
        out = _helper(paths, ["scroll", "--pid", _csv(editors.bench_pids(editor, paths)),
                              "--lines", str(SCROLL_LINES), "--rate", str(SCROLL_RATE),
                              "--timeout", str(_left(deadline))])
        if not out or not out.get("frames"):
            return None
        return out["dropped"] / out["frames"] * 100
    finally:
        editors.quit(editor, paths)


def app_size_mb(editor: str) -> float:
    """`du -sk` of the app bundle, in MiB (1024-based)."""
    out = subprocess.run(["du", "-sk", str(editors.app_bundle(editor))],
                         capture_output=True, text=True, check=True).stdout
    return int(out.split()[0]) / 1024
