"""Runner: rounds, rotation, preflight and the result file.

    python3 -I bench/compare/run.py [--bench-dir D] [--rounds 10]
        [--lsp off|on|both] [--cold] [--only metric,...] [--out FILE]

Result schema: see `main`. The file is rewritten (atomically) after every
round, so an aborted run keeps what it measured.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import shutil
import statistics
import subprocess
import sys
import time
import traceback
from datetime import date, datetime, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import editors  # noqa: E402
import env  # noqa: E402
import metrics  # noqa: E402
from env import BenchPaths  # noqa: E402

REST_S = 20            # between two editor runs
PREFLIGHT_WAIT_S = 30
PREFLIGHT_RETRIES = 10
RESULTS_DIR = Path(__file__).resolve().parent / "results"

METRICS = ("startup_ms", "idle_ram_mb", "idle_cpu_pct", "scroll_dropped_pct",
           "file_change_ms", "app_size_mb")
IDLE = ("idle_ram_mb", "idle_cpu_pct")
# Derived from metrics.METRIC_HELPERS x metrics.HELPER_PERMISSIONS: a metric
# needs a permission if any helper subcommand it runs needs it.
NEEDS_SCREEN = tuple(m for m in METRICS if "screen" in metrics.metric_permissions(m))
NEEDS_AX = tuple(m for m in METRICS if "accessibility" in metrics.metric_permissions(m))
SIMPLE = ("startup_ms", "scroll_dropped_pct", "file_change_ms")


class Abort(Exception):
    """Stop the run; partial results are saved first."""


def _sleep(seconds: float) -> None:
    time.sleep(seconds)


def _log(message: str) -> None:
    print(message, file=sys.stderr, flush=True)


def _log_traceback() -> None:
    print(traceback.format_exc(), file=sys.stderr, end="", flush=True)


def order(rnd: int) -> list[str]:
    k = rnd % len(editors.EDITORS)
    return list(editors.EDITORS[k:] + editors.EDITORS[:k])


def stats(values) -> dict:
    ok = sorted(v for v in values if v is not None)
    failed = len(values) - len(ok)
    if not ok:
        return {"median": None, "p90": None, "failed": failed}
    return {"median": statistics.median(ok),
            "p90": ok[math.ceil(0.9 * len(ok)) - 1],
            "failed": failed}


def _cmd(args: list[str]) -> str:
    try:
        return subprocess.run(args, capture_output=True, text=True).stdout.strip()
    except OSError:
        return ""


def _mtime_iso(path) -> str | None:
    try:
        t = os.stat(path).st_mtime
    except OSError:
        return None
    return datetime.fromtimestamp(t, timezone.utc).isoformat(timespec="seconds")


def helper_display(paths: BenchPaths) -> dict | None:
    """`bench-helper display`: main display size, scale and refresh (no
    permission needed)."""
    try:
        r = subprocess.run([metrics.helper_path(paths), "display"],
                           capture_output=True, text=True, timeout=30)
        data = json.loads(r.stdout.strip().splitlines()[-1]) if r.returncode == 0 else None
    except (OSError, subprocess.SubprocessError, IndexError, ValueError):
        return None
    return data if isinstance(data, dict) else None


def collect_meta(lsp_modes, paths: BenchPaths) -> dict:
    """Run once, before measuring. The Lucerna binary must already be
    resolved (`editors.resolve_lucerna`); this reads the cached record."""
    ra = None
    if True in lsp_modes:
        path = shutil.which("rust-analyzer")
        ra = {"path": path,
              "version": _cmd([path, "--version"]) if path else None}
    versions = {}
    for e in editors.EDITORS:
        try:
            versions[e] = editors.version(e)
        except Exception as exc:  # a missing app must not hide the rest
            versions[e] = f"unknown ({exc})"
    mem = _cmd(["sysctl", "-n", "hw.memsize"])
    return {
        "date": date.today().isoformat(),
        "lucerna_commit": _cmd(["git", "-C", str(env.REPO_ROOT), "rev-parse", "--short", "HEAD"]),
        "versions": versions,
        "macos": f"{_cmd(['sw_vers', '-productVersion'])} ({_cmd(['sw_vers', '-buildVersion'])})",
        "chip": _cmd(["sysctl", "-n", "machdep.cpu.brand_string"]),
        "ram_gb": round(int(mem) / 2 ** 30) if mem.isdigit() else None,
        "rust_analyzer": ra,
        # The measured binary itself; lucerna_commit is only the checkout's HEAD.
        "lucerna_binary": editors.resolve_lucerna(),
        "helper": {"path": metrics.helper_path(paths),
                   "mtime": _mtime_iso(metrics.helper_path(paths))},
        "display": helper_display(paths),
        # Every launching metric but startup; startup keeps each editor's
        # default window size.
        "window_frame": dict(metrics.WINDOW_FRAME),
    }


def helper_permissions(paths: BenchPaths) -> dict | None:
    """Run `bench-helper check` ourselves: it exits 3 *with* its JSON on stdout
    when a permission is missing, which metrics._helper would turn into an
    exception without the details."""
    try:
        r = subprocess.run([metrics.helper_path(paths), "check"],
                           capture_output=True, text=True, timeout=30)
    except (OSError, subprocess.SubprocessError):
        return None
    if r.returncode not in (0, 3):
        return None
    try:
        data = json.loads(r.stdout.strip().splitlines()[-1])
    except (IndexError, ValueError):
        return None
    return data if isinstance(data, dict) else None


def check_permissions(paths: BenchPaths, selected: list[str]) -> None:
    need_screen = [m for m in selected if m in NEEDS_SCREEN]
    need_ax = [m for m in selected if m in NEEDS_AX]
    if not (need_screen or need_ax):
        return
    perms = helper_permissions(paths)
    if perms is None:
        raise Abort("bench-helper `check` cannot run; build the helper with "
                    "`sh bench/compare/helper/build.sh` (see README).")
    missing = []
    if need_screen and not perms.get("screen_recording"):
        missing.append(("Screen Recording", need_screen))
    if need_ax and not perms.get("accessibility"):
        missing.append(("Accessibility", need_ax))
    if not missing:
        return
    lines = [f"Missing {name} permission: needed for {', '.join(ms)}." for name, ms in missing]
    blocked = {m for _, ms in missing for m in ms}
    runnable = [m for m in selected if m not in blocked]
    if runnable:
        lines.append("The remaining measurements can run first: --only " + ",".join(runnable))
    lines.append("Grant the permission in System Settings > Privacy & Security and run again.")
    raise Abort("\n".join(lines))


def check_vscode_lsp(paths: BenchPaths, selected: list[str], lsp_modes) -> None:
    """The rust-analyzer config needs the extension in vscode-ext-lsp."""
    if True not in lsp_modes or not [m for m in selected if m != "app_size_mb"]:
        return
    if not env.vscode_rust_analyzer_installed(paths):
        raise Abort(f"Missing extension {env.VSCODE_RUST_ANALYZER} in "
                    f"{env.vscode_ext_dir(paths, True)} (needed for --lsp on/both). Install it once "
                    "with env.install_vscode_rust_analyzer (see bench/compare/README.md).")


def check_foreign_zed(paths: BenchPaths, selected: list[str]) -> None:
    """Zed allows one instance per channel: stop before touching anything."""
    if not [m for m in selected if m != "app_size_mb"]:
        return
    pids = editors.foreign_instances("zed", paths)
    if pids:
        raise Abort(editors.foreign_message("zed", pids))


def wait_quiet(on_ignored=None) -> None:
    """Preflight; on failure wait and retry, then give up. With `on_ignored`
    (--ignore-preflight) a failure is reported to it and the run goes on."""
    for attempt in range(PREFLIGHT_RETRIES + 1):
        reasons = env.preflight()
        if not reasons:
            return
        _log("machine not quiet: " + "; ".join(reasons))
        if on_ignored is not None:
            on_ignored(reasons)
            _log("--ignore-preflight: measuring anyway")
            return
        if attempt == PREFLIGHT_RETRIES:
            break
        _log(f"waiting {PREFLIGHT_WAIT_S} s, then checking again ({attempt + 1}/{PREFLIGHT_RETRIES})")
        _sleep(PREFLIGHT_WAIT_S)
    raise Abort(f"machine still not quiet after {PREFLIGHT_RETRIES} attempts; stopping (partial results saved).")


def _write_atomic(path: Path, data: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + ".tmp")
    tmp.write_text(json.dumps(data, indent=2) + "\n")
    os.replace(tmp, path)


class Runner:
    def __init__(self, paths, rounds, selected, cold, out, meta, ignore_preflight=False):
        self.paths, self.rounds, self.selected = paths, rounds, selected
        self.cold, self.out = cold, out
        self.result = {"meta": meta, "configs": []}
        self.ran_any = False
        self.ignore_preflight = ignore_preflight
        self.round_no = 0   # 0 = warm-up, then 1-based rounds
        if ignore_preflight:
            meta["preflight_ignored"] = []

    def quiet(self) -> None:
        if not self.ignore_preflight:
            wait_quiet()
            return
        wait_quiet(lambda reasons: self.result["meta"]["preflight_ignored"].append(
            {"round": self.round_no, "reasons": list(reasons)}))

    def save(self) -> None:
        self.result["meta"]["rounds"] = self.rounds
        self.result["meta"]["calibration"] = metrics.load_calibration(self.paths)
        _write_atomic(self.out, self.result)

    def _launch_gate(self) -> None:
        """Rest before every editor launch but the first of the session."""
        if self.ran_any:
            _sleep(REST_S)
        self.ran_any = True

    def _call(self, fn, editor, label):
        try:
            return fn()
        except (metrics.PermissionDenied, editors.ForeignInstance, editors.BinaryMissing):
            raise
        except Exception as exc:  # one broken round must not end the session
            _log(f"  {label} {editor}: error {exc!r}, recording null")
            _log_traceback()
            return None

    def measure(self, cfg_metrics, editor) -> None:
        for name in SIMPLE:
            if name not in self.selected:
                continue
            self._launch_gate()
            if name == "startup_ms" and self.cold:
                input("run `sudo purge`, then press Enter")
            v = self._call(lambda: getattr(metrics, name)(editor, self.paths), editor, name)
            _log(f"  {editor} {name}: {v}")
            cfg_metrics[name][editor].append(v)
            self._record_frame(editor)
        if any(m in self.selected for m in IDLE):
            self._launch_gate()
            r = self._call(lambda: metrics.idle_session(editor, self.paths), editor, "idle")
            ram, cpu = r if r else (None, None)
            self._record_frame(editor)
            _log(f"  {editor} idle: ram {ram} MB, cpu {cpu} %")
            if "idle_ram_mb" in self.selected:
                cfg_metrics["idle_ram_mb"][editor].append(ram)
            if "idle_cpu_pct" in self.selected:
                cfg_metrics["idle_cpu_pct"][editor].append(cpu)

    def _record_frame(self, editor: str) -> None:
        """The window frame `frame` actually set, once per editor per config:
        meta.window_frames[editor] = [{"lsp", "cold", "x", "y", "w", "h"}, ...]."""
        actual = metrics.LAST_FRAME.get(editor)
        cfg = self.result["configs"][-1] if self.result["configs"] else None
        if not actual or cfg is None:
            return
        frames = self.result["meta"].setdefault("window_frames", {}).setdefault(editor, [])
        if any(f["lsp"] == cfg["lsp"] and f["cold"] == cfg["cold"] for f in frames):
            return
        frames.append({"lsp": cfg["lsp"], "cold": cfg["cold"], **actual})

    def warm_up(self) -> None:
        """Round 0: one full round of every selected launching metric (startup
        included) for each editor, discarded. Fills OS and app caches so the
        first measured round is not the first launch of anything."""
        launching = [m for m in self.selected if m != "app_size_mb"]
        if not launching:
            return
        self.round_no = 0
        self.quiet()
        _log("round 0: warm-up, result discarded")
        scratch = {m: {e: [] for e in editors.EDITORS} for m in launching}
        for editor in order(0):
            self.measure(scratch, editor)

    def run_config(self, lsp: bool) -> None:
        # Clean profiles for every config (vscode-ext-lsp is kept: installing
        # rust-analyzer needs the network), then the settings for this one.
        env.reset_profiles(self.paths)
        env.write_profiles(self.paths, lsp)
        metrics.LAST_FRAME.clear()
        cfg = {"lsp": lsp, "cold": self.cold,
               "metrics": {m: {e: [] for e in editors.EDITORS} for m in self.selected}}
        self.result["configs"].append(cfg)
        cm = cfg["metrics"]
        if "app_size_mb" in self.selected:
            for e in editors.EDITORS:
                cm["app_size_mb"][e] = [self._call(
                    lambda: metrics.app_size_mb(e), e, "app_size_mb")]
        self.save()
        self.warm_up()
        launching = [m for m in self.selected if m != "app_size_mb"]
        if not launching:
            return
        for r in range(self.rounds):
            self.round_no = r + 1
            self.quiet()
            _log(f"round {r + 1}/{self.rounds} (lsp={'on' if lsp else 'off'})")
            for editor in order(r):
                # round 0 (warm_up) was discarded; everything here is recorded
                self.measure(cm, editor)
            self.save()


def parse_args(argv) -> argparse.Namespace:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--bench-dir", default=str(env.DEFAULT_ROOT))
    ap.add_argument("--rounds", type=int, default=10)
    ap.add_argument("--lsp", choices=("off", "on", "both"), default="off")
    ap.add_argument("--cold", action="store_true")
    ap.add_argument("--only", default=None, help="comma-separated: " + ",".join(METRICS))
    ap.add_argument("--out", default=None)
    ap.add_argument("--ignore-preflight", action="store_true",
                    help="log preflight failures and measure anyway (reasons go to "
                         "meta.preflight_ignored). Results made with this flag are "
                         "NOT for publication.")
    ap.add_argument("--skip-prepare", action="store_true")
    a = ap.parse_args(argv)
    if a.only is None:
        a.selected = list(METRICS)
    else:
        a.selected = [m.strip() for m in a.only.split(",") if m.strip()]
        bad = [m for m in a.selected if m not in METRICS]
        if bad or not a.selected:
            ap.error(f"--only: invalid name {bad}; valid: {', '.join(METRICS)}")
    if a.rounds < 1:
        ap.error("--rounds must be >= 1")
    return a


def main(argv=None) -> int:
    a = parse_args(sys.argv[1:] if argv is None else argv)
    paths = BenchPaths(Path(a.bench_dir).expanduser())
    lsp_modes = {"off": [False], "on": [True], "both": [False, True]}[a.lsp]
    try:
        # Once per run: the binary recorded in meta is the binary measured.
        editors.resolve_lucerna()
    except editors.BinaryMissing as exc:
        _log(str(exc))
        return 1
    meta = collect_meta(lsp_modes, paths)
    if a.out:
        out = Path(a.out)
    else:
        out = RESULTS_DIR / f"{meta['date']}-{meta['lucerna_commit']}.json"
    runner = Runner(paths, a.rounds, a.selected, a.cold, out, meta, a.ignore_preflight)
    try:
        check_permissions(paths, a.selected)
        check_foreign_zed(paths, a.selected)
        check_vscode_lsp(paths, a.selected, lsp_modes)
        if not a.skip_prepare:
            env.prepare_data(paths)
        for lsp in lsp_modes:
            runner.run_config(lsp)
    except metrics.PermissionDenied as exc:
        detail = str(exc).strip()
        if not detail or detail.endswith(":"):
            detail = (detail + " " if detail else "") + (
                "helper reported a missing Screen Recording or Accessibility "
                "permission (no details)")
        _log(f"permission missing midway: {detail}\nPartial results saved to {out}.")
        runner.save()
        return 1
    except (Abort, editors.ForeignInstance, editors.BinaryMissing) as exc:
        _log(str(exc))
        runner.save()
        return 1
    except EOFError:
        _log(f"stdin closed while waiting for Enter (--cold); partial results saved to {out}.")
        runner.save()
        return 1
    except Exception as exc:
        _log_traceback()
        _log(f"unexpected error: {exc!r}; partial results saved to {out}.")
        runner.save()
        return 1
    except KeyboardInterrupt:
        _log(f"interrupted; partial results saved to {out}.")
        runner.save()
        return 130
    runner.save()
    _log(f"done: {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
