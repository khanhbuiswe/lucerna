"""Multi-window measurement: RAM for n windows, second-window latency, and
frame drops in a scrolling window while another window opens a large repo.

    python3 -I bench/compare/multiwindow.py --windows 1,2,3 --out FILE.json
        [--mode before|after] [--rounds 5] [--bench-dir D] [--only a,b]
        [--single-window]

Spec: docs/superpowers/specs/2026-11-15-multi-window-design.md section 7.1.

`before`: every window is its own `luc <repo>` process (today's behaviour).
`after`: the first is `luc --foreground <repo1>`, the others are `luc <repoK>`
clients of that one process. The `--foreground` flag does not exist before the
one-process work lands, so `after` cannot run on the baseline commit.

Metrics (one value per round, `None` = the round failed):
- `ram_mb[n]`: summed phys_footprint of every `luc` process of the session
  after n windows on n different repos have settled.
- `second_window_ms`: the second `luc` command -> first frame of the new window
  showing content. Window A is empty and small, so the new (larger) window is
  the one the display watch measures, whichever process owns it.
- `drop_pct_idle_b` / `drop_pct_while_busy`: dropped frames while scrolling A
  with B idle (a small repo) / while B opens the large repo.

`--single-window` also records the existing single-window `startup_ms` and
`idle_ram_mb` (metrics.py, Lucerna only), the baseline the one-window threshold
compares against.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import signal
import statistics
import subprocess
import sys
import time
from datetime import date, datetime, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import editors  # noqa: E402
import env  # noqa: E402
import metrics  # noqa: E402
from env import BenchPaths  # noqa: E402

ROUNDS = 5
SETTLE_S = 8.0            # after the last window opened, before reading RAM
WINDOW_UP_S = 5.0         # a window is up and quiet this long after its launch
A_FRAME = metrics.WINDOW_FRAME                       # scrolled window: big
A_SMALL = {"x": 0, "y": 25, "w": 600, "h": 400}      # empty window A in second_window_ms
B_FRAME = {"x": 1030, "y": 600, "w": 420, "h": 320}  # B parked clear of A's centre
SCRATCH = "multi"         # <data>/multi: small repos and the scroll file
REPO_NAMES = ("a", "b", "c", "d")
METRIC_KEYS = ("ram_mb", "second_window_ms", "drop")
QUIT_GRACE_S = 5.0


def _log(message: str) -> None:
    print(f"multiwindow: {message}", file=sys.stderr, flush=True)


# Pure parts ------------------------------------------------------------------

def luc_commands(mode: str, repos, bin: str = "luc") -> list[list[str]]:
    """The `luc` command that opens each repo, in order."""
    repos = [str(r) for r in repos]
    if mode == "before":
        return [[bin, r] for r in repos]
    if mode == "after":
        return [[bin, "--foreground", r] if i == 0 else [bin, r]
                for i, r in enumerate(repos)]
    raise ValueError(f"unknown mode: {mode}")


def _flatten(round_: dict) -> dict:
    flat = {}
    for key, value in round_.items():
        if isinstance(value, dict):
            for sub, v in value.items():
                flat[f"{key}[{sub}]"] = v
        else:
            flat[key] = value
    return flat


def summarize(rounds: list[dict]) -> dict:
    """{metric: {median, min, max, n, failed}}; nested dicts (ram_mb) become
    `ram_mb[<windows>]`. A `None` value is a failed round."""
    series: dict[str, list] = {}
    for r in rounds:
        for key, value in _flatten(r).items():
            series.setdefault(key, []).append(value)
    out = {}
    for key, values in series.items():
        ok = [v for v in values if v is not None]
        out[key] = {"median": statistics.median(ok) if ok else None,
                    "min": min(ok) if ok else None,
                    "max": max(ok) if ok else None,
                    "n": len(ok), "failed": len(values) - len(ok)}
    return out


def parse_args(argv) -> argparse.Namespace:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--windows", default="1,2,3", help="window counts for ram_mb")
    ap.add_argument("--mode", choices=("before", "after"), default="before")
    ap.add_argument("--rounds", type=int, default=ROUNDS)
    ap.add_argument("--out", required=True)
    ap.add_argument("--bench-dir", default=str(env.DEFAULT_ROOT))
    ap.add_argument("--only", default=None,
                    help="comma-separated: " + ",".join(METRIC_KEYS)
                         + " (ram_mb needs no screen permission)")
    ap.add_argument("--single-window", action="store_true",
                    help="also measure startup_ms and idle_ram_mb of one window")
    ap.add_argument("--ignore-preflight", action="store_true")
    a = ap.parse_args(argv)
    try:
        a.windows = [int(x) for x in a.windows.split(",")]
        if not a.windows or min(a.windows) < 1 or max(a.windows) > len(REPO_NAMES):
            raise ValueError
    except ValueError:
        ap.error(f"--windows: counts between 1 and {len(REPO_NAMES)}, e.g. 1,2,3")
    a.selected = list(METRIC_KEYS) if a.only is None else [
        m.strip() for m in a.only.split(",") if m.strip()]
    if not a.selected or [m for m in a.selected if m not in METRIC_KEYS]:
        ap.error(f"--only: valid names are {', '.join(METRIC_KEYS)}")
    if a.rounds < 1:
        ap.error("--rounds must be >= 1")
    return a


# Sessions --------------------------------------------------------------------

class Session:
    """The `luc` processes one bench session started, found by the pids this
    script spawned (the pidfile of editors.py holds only one root, and a
    client of the after mode may exit). Never touches any other `luc`."""

    def __init__(self, paths: BenchPaths, mode: str):
        self.paths, self.mode = paths, mode
        self.roots: list[int] = []
        self.popens: list = []
        self.started: dict[int, str] = {}

    def open(self, repo: Path, index: int) -> int:
        """Run window `index`'s command. Returns the spawned pid."""
        exe = editors.resolve_lucerna()["path"]
        # In the after mode the first window is the one-process server.
        cmds = luc_commands(self.mode, [repo] * (index + 1), exe)
        cmd = cmds[index]
        popen = subprocess.Popen(cmd, env={**os.environ, "HOME": str(self.paths.lucerna_home)},
                                 stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.popens.append(popen)
        self.roots.append(popen.pid)
        self.started[popen.pid] = editors._lstart(popen.pid)
        return popen.pid

    def pids(self, roots=None) -> list[int]:
        table = editors._ps()
        roots = set(self.roots if roots is None else roots)
        live = {p for p, _, _ in table}
        return sorted(editors._descendants(roots & live, table))

    def quit(self) -> None:
        snap = {}
        for pid in self.pids():
            started = self.started.get(pid) or editors._lstart(pid)
            if started:
                snap[pid] = started

        def alive(pid):
            return editors._lstart(pid) == snap[pid]

        for pid in snap:
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        deadline = time.monotonic() + QUIT_GRACE_S
        while time.monotonic() < deadline and any(alive(p) for p in snap):
            for pid in snap:
                editors._reap(pid)
            time.sleep(0.1)
        for pid in snap:
            if alive(pid):
                try:
                    os.kill(pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            editors._reap(pid)
        for p in self.popens:
            try:
                p.wait(timeout=1)
            except subprocess.TimeoutExpired:
                pass


def _csv(pids) -> str:
    return metrics._csv(pids)


def _frame(paths, pids, f: dict, timeout: int = 30) -> bool:
    out = metrics._helper(paths, ["frame", "--pid", _csv(pids), "--x", str(f["x"]),
                                  "--y", str(f["y"]), "--w", str(f["w"]), "--h", str(f["h"]),
                                  "--timeout", str(timeout)])
    return out is not None


def _fresh_profile(paths: BenchPaths) -> None:
    env.reset_profiles(paths)
    env.write_profiles(paths, False)


def repo_dirs(paths: BenchPaths) -> list[Path]:
    return [paths.data / SCRATCH / n for n in REPO_NAMES]


def scroll_dir(paths: BenchPaths) -> Path:
    return paths.data / SCRATCH / "scroll"


def prepare_repos(paths: BenchPaths) -> None:
    """Small distinct repos (a copy of the ripgrep sources each) and a folder
    with only `big.rs`, so scrolling A does not index the 80,000-file repo."""
    for repo in repo_dirs(paths):
        if not repo.exists():
            repo.mkdir(parents=True)
            shutil.copytree(paths.medium_repo / "crates", repo / "crates")
    sd = scroll_dir(paths)
    if not (sd / "big.rs").exists():
        sd.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(paths.big_file, sd / "big.rs")


# Metrics ---------------------------------------------------------------------

def ram_mb(paths: BenchPaths, mode: str, n: int) -> float | None:
    """n windows on n repos, settled; summed footprint of the whole session."""
    _fresh_profile(paths)
    session = Session(paths, mode)
    try:
        for i, repo in enumerate(repo_dirs(paths)[:n]):
            session.open(repo, i)
            time.sleep(WINDOW_UP_S)
        time.sleep(SETTLE_S)
        pids = session.pids()
        if len(session.roots) != n or not pids:
            return None
        return metrics._footprint_mb(pids)
    finally:
        session.quit()


def second_window_ms(paths: BenchPaths, mode: str, bg: str) -> float | None:
    """A (empty file, small) is up; the second `luc` command opens marker.txt;
    time to the first frame of the new window with content."""
    _fresh_profile(paths)
    exe = editors.resolve_lucerna()["path"]
    exclude = editors.running_pids(exe)      # your own luc, never measured
    session = Session(paths, mode)
    watch = None
    try:
        pa = session.open(paths.data / "empty.txt", 0)
        if not _frame(paths, [pa], A_SMALL):
            return None
        time.sleep(WINDOW_UP_S)
        args = ["watch", "--display", "--exe", exe]
        if exclude:
            args += ["--exclude-pids", _csv(exclude)]
        args += ["--since", "0", "--until", f"coverage:{metrics.COVERAGE}",
                 "--background", bg, "--timeout", str(metrics.TIMEOUT_S),
                 "--hold-ms", str(metrics.POST_HOLD_MS)]
        watch = metrics._start_watch(paths, args, metrics.TIMEOUT_S)
        if watch is None:
            return None
        t0 = editors.now_ns()
        session.open(paths.marker_file, 1)
        out = watch.result(metrics.TIMEOUT_S)
        if out is None:
            return None
        if out.get("first_frame_hit") or out["t_ns"] < t0:
            _log("content already on the first frame; round dropped")
            return None
        post = out.get("post_change")
        if post is None or post > metrics.POST_CHANGE_MAX:
            _log(f"window changed after the first content frame ({post}); round dropped")
            return None
        return (out["t_ns"] - t0) / 1e6
    finally:
        if watch is not None:
            watch.close()
        session.quit()


def drop_pct(paths: BenchPaths, mode: str, bg: str, busy: bool) -> float | None:
    """Scroll A (`big.rs`) while B is idle on a small repo (busy=False) or has
    just been asked to open the large repo (busy=True)."""
    _fresh_profile(paths)
    session = Session(paths, mode)
    try:
        deadline = time.monotonic() + metrics.TIMEOUT_S
        t_a = editors.now_ns()
        pa = session.open(scroll_dir(paths) / "big.rs", 0)
        if not _frame(paths, [pa], A_FRAME):
            return None
        out = metrics._helper(paths, [
            "watch", "--pid", _csv(session.pids()), "--since", str(t_a),
            "--until", f"coverage:{metrics.TEXT_COVERAGE}", "--background", bg,
            "--timeout", str(metrics._left(deadline))])
        if out is None:
            return None
        if not busy:
            pb = session.open(repo_dirs(paths)[0], 1)
            _park_b(paths, mode, pb)
            time.sleep(WINDOW_UP_S + SETTLE_S)
        time.sleep(metrics.SETTLE_S)
        if busy:
            pb = session.open(paths.large_repo, 1)
            _park_b(paths, mode, pb)
        out = metrics._helper(paths, [
            "scroll", "--pid", _csv(session.pids([pa])), "--unit", "pixel",
            "--lines", str(metrics.SCROLL_EVENTS),
            "--pixels-per-event", str(metrics.SCROLL_PIXELS_PER_EVENT),
            "--timeout", "30"])
        if not out or not out.get("frames"):
            return None
        return out["dropped"] / out["frames"] * 100
    finally:
        session.quit()


def _park_b(paths, mode: str, pb: int) -> None:
    """Keep B off A's centre. Only the before mode can address B's window by
    its own pid; in the after mode both windows share a pid, B keeps its
    default frame and `scroll` refuses (round = None) if it covers A."""
    if mode == "before":
        _frame(paths, [pb], B_FRAME, timeout=10)


def single_window(paths: BenchPaths, bg_rounds: int) -> dict:
    """Existing single-window metrics for Lucerna only (the one-window
    threshold of spec 7.2 compares against them)."""
    result = {"startup_ms": [], "idle_ram_mb": [], "idle_cpu_pct": []}
    for i in range(bg_rounds + 1):               # round 0 is the discarded warm-up
        _fresh_profile(paths)
        s = metrics.startup_ms("lucerna", paths)
        _fresh_profile(paths)
        ram, cpu = metrics.idle_session("lucerna", paths)
        _log(f"single-window round {i}: startup {s} ms, ram {ram} MB, cpu {cpu} %")
        if i:
            result["startup_ms"].append(s)
            result["idle_ram_mb"].append(ram)
            result["idle_cpu_pct"].append(cpu)
    return result


# Runner ----------------------------------------------------------------------

def _git(*args) -> str:
    try:
        return subprocess.run(["git", "-C", str(env.REPO_ROOT), *args], capture_output=True,
                              text=True).stdout.strip()
    except OSError:
        return ""


def _write(path: Path, data: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + ".tmp")
    tmp.write_text(json.dumps(data, indent=2) + "\n")
    os.replace(tmp, path)


def _wait_quiet(ignore: bool, notes: list) -> None:
    import run   # preflight with retries; imported late, it pulls the whole runner
    run.wait_quiet((lambda reasons: notes.append(list(reasons))) if ignore else None)


def main(argv=None) -> int:
    a = parse_args(sys.argv[1:] if argv is None else argv)
    paths = BenchPaths(Path(a.bench_dir).expanduser())
    out = Path(a.out)
    try:
        binary = editors.resolve_lucerna()
    except editors.BinaryMissing as exc:
        _log(str(exc))
        return 1
    need_screen = [m for m in a.selected if m != "ram_mb"] + (["single"] if a.single_window else [])
    if need_screen:
        import run
        perms = run.helper_permissions(paths)
        if not perms or not (perms.get("screen_recording") and perms.get("accessibility")):
            _log("Screen Recording and Accessibility are needed for "
                 f"{','.join(need_screen)}; run with --only ram_mb or grant them (see README).")
            return 1
    result = {"meta": {
        "date": date.today().isoformat(), "mode": a.mode, "rounds": a.rounds,
        "windows": a.windows, "selected": a.selected,
        "lucerna_commit": _git("rev-parse", "--short", "HEAD"),
        "lucerna_binary": binary,
        "macos": subprocess.run(["sw_vers", "-productVersion"], capture_output=True,
                                text=True).stdout.strip(),
        "chip": subprocess.run(["sysctl", "-n", "machdep.cpu.brand_string"],
                               capture_output=True, text=True).stdout.strip(),
        "preflight_ignored": [],
        "recorded_at": datetime.now(timezone.utc).isoformat(timespec="seconds")},
        "rounds": [], "summary": {}}
    try:
        env.prepare_data(paths)
        prepare_repos(paths)
        _fresh_profile(paths)
        bg = metrics.background("lucerna", paths) if need_screen else None
        if need_screen and bg is None:
            _log("background calibration failed")
            return 1
        for r in range(a.rounds):
            _wait_quiet(a.ignore_preflight, result["meta"]["preflight_ignored"])
            _log(f"round {r + 1}/{a.rounds} ({a.mode})")
            row: dict = {}
            if "ram_mb" in a.selected:
                row["ram_mb"] = {str(n): ram_mb(paths, a.mode, n) for n in a.windows}
                _log(f"  ram_mb {row['ram_mb']}")
            if "second_window_ms" in a.selected:
                row["second_window_ms"] = second_window_ms(paths, a.mode, bg)
                _log(f"  second_window_ms {row['second_window_ms']}")
            if "drop" in a.selected:
                row["drop_pct_idle_b"] = drop_pct(paths, a.mode, bg, busy=False)
                row["drop_pct_while_busy"] = drop_pct(paths, a.mode, bg, busy=True)
                _log(f"  drop idle {row['drop_pct_idle_b']} busy {row['drop_pct_while_busy']}")
            result["rounds"].append(row)
            result["summary"] = summarize(result["rounds"])
            _write(out, result)
        if a.single_window:
            result["single_window"] = single_window(paths, a.rounds)
            result["summary_single_window"] = summarize([
                {k: result["single_window"][k][i] for k in result["single_window"]}
                for i in range(a.rounds)])
        _write(out, result)
    except (metrics.PermissionDenied, editors.BinaryMissing) as exc:
        _log(f"stopped: {exc}; partial results in {out}")
        _write(out, result)
        return 1
    except KeyboardInterrupt:
        _write(out, result)
        return 130
    _log(f"done: {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
