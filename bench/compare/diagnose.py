#!/usr/bin/env python3
"""Diagnostics for the Quick Open and scrolling measurements.

Not a benchmark: it records what the helper sees so a person can check
whether a measurement method is sound before its numbers are published.
Needs Screen Recording and Accessibility for the terminal that runs it.

    python3 -I bench/compare/diagnose.py [--editors lucerna,zed,vscode]
        [--bench-dir D] [--out DIR] [--skip quick_open|scroll|manual]

For each editor it:
  quick_open  opens the large repo, waits for the scan, opens Quick Open,
              types `mod004`, then records how much of the window changes
              after `2` and after a non-matching `x` (min-change 0.01 %,
              PNG dumps of the reference and the most-changed frame);
  scroll      opens big.rs and scrolls with line events at 120/s and 60/s
              and with pixel (trackpad-like) events at 60/s, printing the
              frame-interval statistics of each;
  manual      (Zed and Lucerna by default) asks you to scroll big.rs by
              hand for 5 s on the trackpad while `record` measures the
              frame intervals, for comparison with the synthetic scroll.

Everything goes to --out (default <bench>/diagnose-<time>/), with a
summary.json; paste that file's contents back for analysis.
"""
from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import editors  # noqa: E402
import env  # noqa: E402
import metrics  # noqa: E402

LOG: list[str] = []


def log(msg: str) -> None:
    print(msg, file=sys.stderr, flush=True)
    LOG.append(msg)


def helper_run(paths, args, timeout=metrics.TIMEOUT_S):
    """Like metrics._helper, but keeps stderr (the diagnostics) on failure."""
    import subprocess
    r = subprocess.run([metrics.helper_path(paths), *args], capture_output=True,
                       text=True, timeout=timeout + 15)
    out = None
    if r.returncode == 0 and r.stdout.strip():
        try:
            out = json.loads(r.stdout.strip().splitlines()[-1])
        except ValueError:
            pass
    return {"rc": r.returncode, "out": out, "stderr": r.stderr.strip()[-800:]}


def start(editor, paths, target):
    launched = editors.launch(editor, paths, target)
    deadline = time.monotonic() + metrics.TIMEOUT_S
    if metrics._owner_pids(editor, paths, deadline) is None:
        raise RuntimeError(f"{editor}: no window process")
    if not metrics._set_frame(editor, paths, deadline):
        raise RuntimeError(f"{editor}: could not set the window frame")
    return launched


def pids(editor, paths) -> str:
    return metrics._csv(editors.bench_pids(editor, paths))


def watch_after_key(editor, paths, key_text, dump: Path, min_change="0.002"):
    """Start a change watch, type `key_text`, return watch result + key time.

    `--since` lies 1 s ahead, so the reference is the last frame just before
    the key and changes before it (a blinking caret, a late repaint) are
    not counted. 0.2 % is above a caret blink (about 0.1 % of the window)."""
    dump.mkdir(parents=True, exist_ok=True)
    p = pids(editor, paths)
    since = editors.now_ns() + 1_000_000_000
    w = metrics._start_watch(paths, ["watch", "--pid", p, "--since", str(since),
                                     "--until", "change", "--min-change", min_change,
                                     "--dump-dir", str(dump), "--timeout", "15"], 20)
    if w is None:
        return {"error": "watch did not start"}
    time.sleep(max(0.0, (since - editors.now_ns()) / 1e9) + 0.05)
    typed = helper_run(paths, ["type", "--pid", p, "--text", key_text])
    out = w.result(20)
    w.close()
    res = {"typed": typed, "watch": out, "watch_stderr": "".join(w._errors)[-800:]}
    if out and typed["out"]:
        res["latency_ms"] = (out["t_ns"] - typed["out"]["t_ns"][-1]) / 1e6
    changes = dump / "changes.txt"
    if changes.exists():
        rows = [ln.split() for ln in changes.read_text().splitlines() if ln.strip()]
        res["changes_after_key_ms"] = [
            [round((int(t) - typed["out"]["t_ns"][-1]) / 1e6, 1), float(f)]
            for t, f in rows] if typed["out"] else rows
    return res


def quick_open(editor, paths, out: Path):
    res = {}
    start(editor, paths, paths.large_repo)
    try:
        log(f"{editor}: waiting {metrics.SCAN_WAIT_S} s for the scan")
        time.sleep(metrics.SCAN_WAIT_S)
        p = pids(editor, paths)
        res["open"] = helper_run(paths, ["key", "--pid", p, "--combo", "cmd+p"])
        time.sleep(metrics.SETTLE_S / 2)
        res["type_prefix"] = helper_run(paths, ["type", "--pid", p, "--text", "mod004"])
        time.sleep(metrics.SETTLE_S)
        res["key_2"] = watch_after_key(editor, paths, "2", out / f"qo-{editor}-2")
        time.sleep(metrics.SETTLE_S)
        res["key_x"] = watch_after_key(editor, paths, "x", out / f"qo-{editor}-x")
    finally:
        editors.quit(editor, paths)
    return res


def scroll(editor, paths, out: Path):
    res = {}
    variants = [("line-120", ["--lines", "600", "--rate", "120"]),
                ("line-60", ["--lines", "300", "--rate", "60"]),
                ("pixel-60", ["--unit", "pixel", "--lines", "300", "--rate", "60",
                              "--pixels-per-event", "40"])]
    for name, extra in variants:
        start(editor, paths, paths.big_file)
        try:
            time.sleep(3)
            dump = out / f"scroll-{editor}-{name}"
            dump.mkdir(parents=True, exist_ok=True)
            res[name] = helper_run(paths, ["scroll", "--pid", pids(editor, paths), *extra,
                                           "--dump-dir", str(dump), "--timeout", "30"])
            log(f"{editor} {name}: {res[name]['out'] or res[name]['stderr']}")
        finally:
            editors.quit(editor, paths)
        time.sleep(3)
    return res


def manual(editor, paths, out: Path):
    start(editor, paths, paths.big_file)
    try:
        time.sleep(3)
        input(f"\n{editor}: put the pointer over the {editor} window. Press Enter, "
              "then scroll big.rs with the trackpad, steadily, for 5 seconds… ")
        dump = out / f"manual-{editor}"
        dump.mkdir(parents=True, exist_ok=True)
        res = helper_run(paths, ["record", "--pid", pids(editor, paths), "--seconds", "5",
                                 "--dump-dir", str(dump)], timeout=20)
        log(f"{editor} manual: {res['out'] or res['stderr']}")
        return res
    finally:
        editors.quit(editor, paths)


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--bench-dir", type=Path, default=env.DEFAULT_ROOT)
    ap.add_argument("--editors", default="lucerna,zed,vscode")
    ap.add_argument("--manual-editors", default="lucerna,zed")
    ap.add_argument("--skip", default="", help="comma-separated: quick_open,scroll,manual")
    ap.add_argument("--out", type=Path)
    a = ap.parse_args(argv)
    paths = env.BenchPaths(a.bench_dir)
    out = a.out or paths.root / time.strftime("diagnose-%Y%m%d-%H%M%S")
    out.mkdir(parents=True, exist_ok=True)
    skip = set(filter(None, a.skip.split(",")))
    eds = [e for e in a.editors.split(",") if e]
    for e in eds:
        if e not in editors.EDITORS:
            ap.error(f"unknown editor {e}")
    if "zed" in eds and editors.foreign_instances("zed", paths):
        print(editors.foreign_message("zed", editors.foreign_instances("zed", paths)),
              file=sys.stderr)
        return 1
    env.reset_profiles(paths)
    env.write_profiles(paths, lsp=False)
    summary = {"meta": {"versions": {e: editors.version(e) for e in eds},
                        "helper": metrics.helper_path(paths)}}
    try:
        for e in eds:
            summary[e] = {}
            if "quick_open" not in skip:
                summary[e]["quick_open"] = quick_open(e, paths, out)
            if "scroll" not in skip:
                summary[e]["scroll"] = scroll(e, paths, out)
        if "manual" not in skip:
            for e in a.manual_editors.split(","):
                if e in eds:
                    summary[e]["manual"] = manual(e, paths, out)
    finally:
        summary["log"] = LOG
        (out / "summary.json").write_text(json.dumps(summary, indent=1))
        print(f"\nsummary: {out / 'summary.json'}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
