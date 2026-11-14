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
from datetime import date
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
           "file_change_ms", "quick_open_ms", "app_size_mb")
IDLE = ("idle_ram_mb", "idle_cpu_pct")
NEEDS_SCREEN = ("startup_ms", "scroll_dropped_pct", "file_change_ms", "quick_open_ms")
NEEDS_AX = ("quick_open_ms", "idle_ram_mb", "idle_cpu_pct")
SIMPLE = ("startup_ms", "scroll_dropped_pct", "file_change_ms", "quick_open_ms")


class Abort(Exception):
    """Stop the run; partial results are saved first."""


def _sleep(seconds: float) -> None:
    time.sleep(seconds)


def _log(message: str) -> None:
    print(message, file=sys.stderr, flush=True)


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


def collect_meta(lsp_modes) -> dict:
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
    }


def helper_permissions(paths: BenchPaths) -> dict | None:
    return metrics._helper(paths, ["check"])


def check_permissions(paths: BenchPaths, selected: list[str]) -> None:
    need_screen = [m for m in selected if m in NEEDS_SCREEN]
    need_ax = [m for m in selected if m in NEEDS_AX]
    if not (need_screen or need_ax):
        return
    perms = helper_permissions(paths)
    if perms is None:
        raise Abort("bench-helper `check` không chạy được; dựng helper bằng "
                    "`sh bench/compare/helper/build.sh` (xem README).")
    missing = []
    if need_screen and not perms.get("screen_recording"):
        missing.append(("Screen Recording", need_screen))
    if need_ax and not perms.get("accessibility"):
        missing.append(("Accessibility", need_ax))
    if not missing:
        return
    lines = [f"Thiếu quyền {name}: cần cho {', '.join(ms)}." for name, ms in missing]
    blocked = {m for _, ms in missing for m in ms}
    runnable = [m for m in selected if m not in blocked]
    if runnable:
        lines.append("Có thể chạy trước các phép đo còn lại: --only " + ",".join(runnable))
    lines.append("Cấp quyền trong System Settings > Privacy & Security rồi chạy lại.")
    raise Abort("\n".join(lines))


def wait_quiet() -> None:
    """Preflight; on failure wait and retry, then give up."""
    for attempt in range(PREFLIGHT_RETRIES + 1):
        reasons = env.preflight()
        if not reasons:
            return
        _log("máy chưa yên: " + "; ".join(reasons))
        if attempt == PREFLIGHT_RETRIES:
            break
        _log(f"chờ {PREFLIGHT_WAIT_S} s rồi kiểm tra lại ({attempt + 1}/{PREFLIGHT_RETRIES})")
        _sleep(PREFLIGHT_WAIT_S)
    raise Abort(f"máy vẫn chưa yên sau {PREFLIGHT_RETRIES} lần thử; dừng (đã lưu kết quả dở).")


def _write_atomic(path: Path, data: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + ".tmp")
    tmp.write_text(json.dumps(data, indent=2) + "\n")
    os.replace(tmp, path)


class Runner:
    def __init__(self, paths, rounds, selected, cold, out, meta):
        self.paths, self.rounds, self.selected = paths, rounds, selected
        self.cold, self.out = cold, out
        self.result = {"meta": meta, "configs": []}
        self.ran_any = False

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
        except metrics.PermissionDenied:
            raise
        except Exception as exc:  # one broken round must not end the session
            _log(f"  {label} {editor}: lỗi {exc!r}, ghi null")
            return None

    def measure(self, cfg_metrics, editor) -> None:
        for name in SIMPLE:
            if name not in self.selected:
                continue
            self._launch_gate()
            if name == "startup_ms" and self.cold:
                input("chạy `sudo purge` rồi nhấn Enter")
            v = self._call(lambda: getattr(metrics, name)(editor, self.paths), editor, name)
            _log(f"  {editor} {name}: {v}")
            cfg_metrics[name][editor].append(v)
        if any(m in self.selected for m in IDLE):
            self._launch_gate()
            r = self._call(lambda: metrics.idle_session(editor, self.paths), editor, "idle")
            ram, cpu = r if r else (None, None)
            _log(f"  {editor} idle: ram {ram} MB, cpu {cpu} %")
            if "idle_ram_mb" in self.selected:
                cfg_metrics["idle_ram_mb"][editor].append(ram)
            if "idle_cpu_pct" in self.selected:
                cfg_metrics["idle_cpu_pct"][editor].append(cpu)

    def warm_up(self, cfg_metrics) -> None:
        """First startup of each editor runs and is discarded."""
        if "startup_ms" not in self.selected:
            return
        for editor in editors.EDITORS:
            wait_quiet()
            _log(f"khởi động thử {editor} (bỏ kết quả)")
            self._launch_gate()
            if self.cold:
                input("chạy `sudo purge` rồi nhấn Enter")
            self._call(lambda: metrics.startup_ms(editor, self.paths), editor, "startup_ms")

    def run_config(self, lsp: bool) -> None:
        env.write_profiles(self.paths, lsp)
        cfg = {"lsp": lsp, "cold": self.cold,
               "metrics": {m: {e: [] for e in editors.EDITORS} for m in self.selected}}
        self.result["configs"].append(cfg)
        cm = cfg["metrics"]
        if "app_size_mb" in self.selected:
            for e in editors.EDITORS:
                cm["app_size_mb"][e] = [self._call(
                    lambda: metrics.app_size_mb(e), e, "app_size_mb")]
        self.save()
        self.warm_up(cm)
        launching = [m for m in self.selected if m != "app_size_mb"]
        if not launching:
            return
        for r in range(self.rounds):
            wait_quiet()
            _log(f"lượt {r + 1}/{self.rounds} (lsp={'on' if lsp else 'off'})")
            for editor in order(r):
                # warm-up already consumed the discarded startup, so measure()
                # here records everything it returns
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
    ap.add_argument("--skip-prepare", action="store_true")
    a = ap.parse_args(argv)
    if a.only is None:
        a.selected = list(METRICS)
    else:
        a.selected = [m.strip() for m in a.only.split(",") if m.strip()]
        bad = [m for m in a.selected if m not in METRICS]
        if bad or not a.selected:
            ap.error(f"--only: tên không hợp lệ {bad}; hợp lệ: {', '.join(METRICS)}")
    if a.rounds < 1:
        ap.error("--rounds phải >= 1")
    return a


def main(argv=None) -> int:
    a = parse_args(sys.argv[1:] if argv is None else argv)
    paths = BenchPaths(Path(a.bench_dir).expanduser())
    lsp_modes = {"off": [False], "on": [True], "both": [False, True]}[a.lsp]
    meta = collect_meta(lsp_modes)
    if a.out:
        out = Path(a.out)
    else:
        out = RESULTS_DIR / f"{meta['date']}-{meta['lucerna_commit']}.json"
    runner = Runner(paths, a.rounds, a.selected, a.cold, out, meta)
    try:
        check_permissions(paths, a.selected)
        if not a.skip_prepare:
            env.prepare_data(paths)
        for lsp in lsp_modes:
            runner.run_config(lsp)
    except metrics.PermissionDenied as exc:
        _log(f"thiếu quyền giữa chừng: {exc}\nĐã lưu kết quả dở tại {out}.")
        runner.save()
        return 1
    except Abort as exc:
        _log(str(exc))
        runner.save()
        return 1
    except EOFError:
        _log(f"stdin đóng khi chờ Enter (--cold); đã lưu kết quả dở tại {out}.")
        runner.save()
        return 1
    except Exception as exc:
        _log(f"lỗi bất ngờ: {exc!r}; đã lưu kết quả dở tại {out}.")
        runner.save()
        return 1
    except KeyboardInterrupt:
        _log(f"bị ngắt; đã lưu kết quả dở tại {out}.")
        runner.save()
        return 130
    runner.save()
    _log(f"xong: {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
