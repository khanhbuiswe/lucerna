"""The measurements, taken from outside each editor.

Every metric launches the editor in its bench profile, measures, and quits it
in a `finally`. A metric returns None for a failed round (helper error or
timeout, no window, a suspect frame); a missing Screen Recording/Accessibility
permission raises `PermissionDenied` instead, so a run aborts rather than
recording garbage.

Frame times come from `bench-helper` (README "Helper"); its clock is
CLOCK_UPTIME_RAW, read here with `editors.now_ns()`.
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
OPEN_SETTLE_S = 5.0         # medium repo: window up before Quick Open
SCAN_WAIT_S = 30            # large repo: let the file scan finish
IDLE_S = 60
SCROLL_LINES = 3000
SCROLL_RATE = 120
QUICK_OPEN_QUERY = "mod0042"  # gen-bench-repo.sh: directory pkg00/mod0042/

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

# The process that owns the window. The `code` / `cli` launchers (and VS Code's
# `cli.js` node process) carry the profile flag too but own no window.
_MAIN_BINARY = {
    "vscode": str(editors.VSCODE_APP / "Contents/MacOS/Code"),
    "zed": str(editors.ZED_APP / "Contents/MacOS/zed"),
}


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
            out = self._p.stdout.read()
            self._p.stdout.close()
        return _result(self._p.returncode, out, "".join(self._errors), self._args)


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
    """All bench pids, once the process that owns the window exists."""
    while time.monotonic() < deadline:
        pids = editors.bench_pids(editor, paths)
        if pids and editor == "lucerna":
            return pids
        if pids:
            out = subprocess.run(["ps", "-o", "command=", "-p", _csv(pids)],
                                 capture_output=True, text=True).stdout
            main = _MAIN_BINARY[editor]
            if any(c.startswith(main) and "cli.js" not in c and "--type=" not in c
                   for c in out.splitlines()):
                return editors.bench_pids(editor, paths)
        time.sleep(0.05)
    _log(f"{editor}: no window process within {TIMEOUT_S} s")
    return None


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
        return out["center_dominant"] if out else None
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
    """Launch call → first frame whose centre is ≥ 30 % marker text."""
    bg = background(editor, paths)
    if bg is None:
        return None
    launched = editors.launch(editor, paths, paths.marker_file)
    try:
        deadline = time.monotonic() + TIMEOUT_S
        pids = _owner_pids(editor, paths, deadline)
        if pids is None:
            return None
        out = _helper(paths, ["watch", "--pid", _csv(pids), "--since", str(launched.t0_ns),
                              "--until", f"coverage:{COVERAGE}", "--background", bg,
                              "--timeout", str(_left(deadline))])
        if out is None:
            return None
        if out.get("first_frame_hit"):
            # Content predates the capture: the number is only an upper bound
            # inflated by capture latency.
            _log(f"{editor}: startup content already on the first frame; round dropped")
            return None
        return (out["t_ns"] - launched.t0_ns) / 1e6
    finally:
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
    try:
        deadline = time.monotonic() + TIMEOUT_S
        if _owner_pids(editor, paths, deadline) is None:
            return None
        if not _shown(editor, paths, bg, launched.t0_ns, COVERAGE, deadline):
            return None
        time.sleep(SETTLE_S)
        watch = _start_watch(paths, ["watch", "--pid", _csv(editors.bench_pids(editor, paths)),
                                     "--since", str(editors.now_ns()), "--until", "change",
                                     "--timeout", str(_left(deadline))])
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
        editors.quit(editor, paths)
        shutil.copyfile(paths.marker_file, reload)


def quick_open_ms(editor: str, paths: BenchPaths) -> float | None:
    """Large repo, ⌘P, type `mod0042`: last key → first changed frame."""
    editors.launch(editor, paths, paths.large_repo)
    try:
        if _owner_pids(editor, paths, time.monotonic() + TIMEOUT_S) is None:
            return None
        time.sleep(SCAN_WAIT_S)
        deadline = time.monotonic() + TIMEOUT_S
        pids = _csv(editors.bench_pids(editor, paths))
        if _helper(paths, ["key", "--pid", pids, "--combo", "cmd+p"]) is None:
            return None
        if _helper(paths, ["type", "--pid", pids, "--text", QUICK_OPEN_QUERY[:-1]]) is None:
            return None
        time.sleep(SETTLE_S)
        watch = _start_watch(paths, ["watch", "--pid", pids, "--since", str(editors.now_ns()),
                                     "--until", "change", "--timeout", str(_left(deadline))])
        if watch is None:
            return None
        typed = _helper(paths, ["type", "--pid", pids, "--text", QUICK_OPEN_QUERY[-1]])
        out = watch.result(_left(deadline))
        if typed is None or out is None:
            return None
        t_k = typed["t_ns"][-1]
        if out["t_ns"] < t_k:
            _log(f"{editor}: Quick Open changed before the last key; round dropped")
            return None
        return (out["t_ns"] - t_k) / 1e6
    finally:
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


def _cpu_pct(pids) -> float | None:
    """Mean over IDLE_S one-second samples of the summed `ps %cpu`."""
    samples = []
    for _ in range(IDLE_S):
        out = subprocess.run(["ps", "-o", "%cpu=", "-p", _csv(pids)],
                             capture_output=True, text=True).stdout
        values = [float(v) for v in out.split()]
        if values:
            samples.append(sum(values))
        time.sleep(1)
    return sum(samples) / len(samples) if samples else None


def _open_with_quick_open(editor, paths, relative: str) -> bool:
    pids = _csv(editors.bench_pids(editor, paths))
    if _helper(paths, ["key", "--pid", pids, "--combo", "cmd+p"]) is None:
        return False
    time.sleep(SETTLE_S / 2)
    if _helper(paths, ["type", "--pid", pids, "--text", relative]) is None:
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
        if _owner_pids(editor, paths, time.monotonic() + TIMEOUT_S) is None:
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
        cpu = _cpu_pct(pids)
        return ram, cpu
    finally:
        editors.quit(editor, paths)


def idle_ram_mb(editor: str, paths: BenchPaths) -> float | None:
    return idle_session(editor, paths)[0]


def idle_cpu_pct(editor: str, paths: BenchPaths) -> float | None:
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
        if not _shown(editor, paths, bg, launched.t0_ns, TEXT_COVERAGE, deadline):
            return None
        time.sleep(SETTLE_S)
        out = _helper(paths, ["scroll", "--pid", _csv(editors.bench_pids(editor, paths)),
                              "--lines", str(SCROLL_LINES), "--rate", str(SCROLL_RATE),
                              "--timeout", str(TIMEOUT_S)])
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
