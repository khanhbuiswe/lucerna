"""Launch, identify and clean up each editor inside its own bench profile.

Safety: the user's own Zed / VS Code may be running. Only processes that carry
the bench profile path (and their descendants), or the `luc` recorded in the
bench pidfile (and its descendants), are ever returned by `bench_pids`, and
`quit` signals nothing else.

Lucerna is found by pidfile, not by its HOME: macOS `ps` does not print another
process's environment, so a HOME match can never work. `launch` records the pid,
its start time and the exact binary path; a reused pid has a different start
time and is ignored, and the command must start with that exact path (paths
with spaces work).

The Lucerna binary is resolved ONCE per run (`resolve_lucerna`, which may call
`cargo metadata` through scripts/target-dir.sh) and cached, so the measured
binary is the one recorded in the result meta and nothing runs cargo during
measurement.
"""

from __future__ import annotations

import json
import os
import plistlib
import re
import shutil
import signal
import subprocess
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

from env import PIDFILE_NAME, REPO_ROOT, BenchPaths, profile_lsp, vscode_ext_dir

EDITORS = ("lucerna", "zed", "vscode")
VSCODE_APP = Path("/Applications/Visual Studio Code.app")
ZED_APP = Path("/Applications/Zed.app")
# Both are launched through their main binary, which owns the window: VS
# Code's `bin/code` adds a node/CLI hop the other editors don't pay, and Zed's
# `cli` hands off through LaunchServices and drops --user-data-dir.
VSCODE_BIN = VSCODE_APP / "Contents/MacOS/Code"
ZED_BIN = ZED_APP / "Contents/MacOS/zed"
MAIN_BINARY = {
    "vscode": str(VSCODE_BIN),
    "zed": str(ZED_BIN),
}
# The user's own instance, wherever the app bundle lives: matched on the
# executable part of the command line (see `_exe_part`).
_MAIN_PATTERN = {
    "zed": re.compile(r"(^|/)Zed\.app/Contents/MacOS/zed(\s|$)"),
    "vscode": re.compile(r"(^|/)Visual Studio Code\.app/Contents/MacOS/Code(\s|$)"),
}
QUIT_GRACE_S = 5.0
VSCODE_DROP_ENV = ("ELECTRON_RUN_AS_NODE", "ELECTRON_NO_ATTACH_CONSOLE")


def now_ns() -> int:
    """bench-helper's clock (CLOCK_UPTIME_RAW). Not `time.monotonic_ns()`:
    Python 3.9 (the macOS system python3) counts that from process start."""
    return time.clock_gettime_ns(time.CLOCK_UPTIME_RAW)


class ForeignInstance(RuntimeError):
    """The user's own instance blocks a bench launch. Never signalled."""


@dataclass
class Launched:
    t0_ns: int
    popen: object


class BinaryMissing(RuntimeError):
    """The Lucerna binary is missing or not executable: the run must stop."""


def _vscode_dirs(paths: BenchPaths, lsp: bool = False) -> tuple[str, str]:
    return str(paths.profiles / "vscode"), str(vscode_ext_dir(paths, lsp))


def _zed_dir(paths: BenchPaths) -> str:
    return str(paths.profiles / "zed")


_LUCERNA: dict = {}   # cached resolution: {"path", "mtime", "version"}


def _find_lucerna() -> str:
    override = os.environ.get("LUCERNA_BIN")
    if override:
        return override
    try:
        out = subprocess.run([str(REPO_ROOT / "scripts" / "target-dir.sh")],
                             capture_output=True, text=True, timeout=30)
        bundled = Path(out.stdout.strip()) / "release/Lucerna.app/Contents/MacOS/luc"
        if out.stdout.strip() and bundled.exists():
            return str(bundled)
    except (OSError, subprocess.SubprocessError):
        pass
    found = shutil.which("luc")
    if not found:
        raise BinaryMissing("không tìm thấy `luc`: đặt LUCERNA_BIN hoặc dựng bản release "
                            "bằng scripts/bundle-app.sh")
    return found


def _check_executable(path: str) -> None:
    if not (os.path.isfile(path) and os.access(path, os.X_OK)):
        raise BinaryMissing(f"binary Lucerna {path} không tồn tại hoặc không chạy được "
                            "(bị xoá hay đang dựng lại?); dừng.")


def resolve_lucerna(refresh: bool = False) -> dict:
    """Resolve the Lucerna binary once and cache it: {"path", "mtime" (ISO,
    UTC), "version" (`luc --version`)}. Raises BinaryMissing."""
    if _LUCERNA and not refresh:
        return dict(_LUCERNA)
    path = os.path.realpath(_find_lucerna())
    _check_executable(path)
    mtime = datetime.fromtimestamp(os.stat(path).st_mtime, timezone.utc)
    try:
        v = subprocess.run([path, "--version"], capture_output=True, text=True,
                           timeout=30).stdout.strip()
    except (OSError, subprocess.SubprocessError):
        v = ""
    _LUCERNA.clear()
    _LUCERNA.update({"path": path, "mtime": mtime.isoformat(timespec="seconds"),
                     "version": v})
    return dict(_LUCERNA)


def _lucerna_bin() -> str:
    return resolve_lucerna()["path"]


def main_executable(editor: str) -> str:
    """Absolute path of the binary that owns the editor's window."""
    if editor == "lucerna":
        return _lucerna_bin()
    try:
        return MAIN_BINARY[editor]
    except KeyError:
        raise ValueError(f"unknown editor: {editor}") from None


def launch(editor: str, paths: BenchPaths, target: Path) -> Launched:
    env = None
    if editor == "vscode":
        # The extensions dir follows the profile written for this config:
        # empty plus --disable-extensions without LSP, the rust-analyzer one
        # with it.
        lsp = profile_lsp(paths)
        user, ext = _vscode_dirs(paths, lsp)
        cmd = [str(VSCODE_BIN), "--user-data-dir", user, "--extensions-dir", ext]
        if not lsp:
            cmd.append("--disable-extensions")
        cmd += ["--new-window", str(target)]
        # Run as the app, never as plain node, whatever the calling shell set.
        env = {k: v for k, v in os.environ.items() if k not in VSCODE_DROP_ENV}
    elif editor == "zed":
        # Run the main binary itself: `cli` hands off through LaunchServices
        # and drops --user-data-dir, so Zed would open the user's real profile.
        foreign = foreign_instances("zed", paths)
        if foreign:
            raise ForeignInstance(foreign_message("zed", foreign))
        cmd = [str(ZED_BIN), "--user-data-dir", _zed_dir(paths), str(target)]
    elif editor == "lucerna":
        exe = _lucerna_bin()
        _check_executable(exe)   # vanished since the run started (rebuild, clean)
        cmd = [exe, str(target)]
        env = {**os.environ, "HOME": str(paths.lucerna_home)}
    else:
        raise ValueError(f"unknown editor: {editor}")
    t0 = now_ns()
    popen = subprocess.Popen(cmd, env=env, stdout=subprocess.DEVNULL,
                             stderr=subprocess.DEVNULL)
    if editor == "lucerna":
        pidfile = paths.lucerna_home / PIDFILE_NAME
        pidfile.parent.mkdir(parents=True, exist_ok=True)
        pidfile.write_text(json.dumps({"pid": popen.pid, "lstart": _lstart(popen.pid),
                                       "exe": cmd[0]}))
    return Launched(t0_ns=t0, popen=popen)


def _ps() -> list[tuple[int, int, str]]:
    out = subprocess.run(["ps", "-axo", "pid=,ppid=,command="],
                         capture_output=True, text=True).stdout
    rows = []
    for line in out.splitlines():
        parts = line.split(None, 2)
        if len(parts) == 3 and parts[0].isdigit() and parts[1].isdigit():
            rows.append((int(parts[0]), int(parts[1]), parts[2]))
    return rows


def _exe_part(command: str) -> str:
    """The executable part of a `ps` command line: up to the first argument
    that starts with "-" or "/" (so an app path with spaces stays whole)."""
    return re.split(r"\s(?=[-/])", command, maxsplit=1)[0]


def foreign_instances(editor: str, paths: BenchPaths) -> list[int]:
    """Main app processes of `editor` that are not the bench session's,
    wherever the app bundle lives."""
    pattern = _MAIN_PATTERN.get(editor)
    if pattern is None:
        return []
    mine = set(bench_pids(editor, paths))
    found = []
    for pid, _, cmd in _ps():
        if pid in mine or not pattern.search(_exe_part(cmd)):
            continue
        if "--crash-handler" in cmd or "--type=" in cmd:
            continue
        found.append(pid)
    return sorted(found)


def foreign_message(editor: str, pids: list[int]) -> str:
    listed = ", ".join(str(p) for p in pids)
    if editor == "zed":
        return (f"Zed của bạn đang chạy (pid {listed}). Zed chỉ cho một phiên mỗi "
                "kênh phát hành, nên bench không thể mở Zed riêng. "
                "Hãy tự đóng Zed rồi chạy lại.")
    return f"{editor} của bạn đang chạy (pid {listed})."


def running_pids(exe: str) -> list[int]:
    """Pids already running `exe` or any process inside its app bundle's
    Contents/ (what `watch --display` would match): excluded from the
    display watch so the user's own instance is never measured."""
    contents = None
    i = exe.rfind(".app/Contents/")
    if i >= 0:
        contents = exe[:i + len(".app/Contents/")]
    me = os.getpid()
    return sorted(pid for pid, _, cmd in _ps()
                  if pid != me and (cmd == exe or cmd.startswith(exe + " ")
                                    or (contents and cmd.startswith(contents))))


def _lstart(pid: int) -> str:
    """Process start time; empty when the pid is gone."""
    return subprocess.run(["ps", "-o", "lstart=", "-p", str(pid)],
                          capture_output=True, text=True).stdout.strip()


def _ancestors(pid: int, table) -> set[int]:
    parent = {p: pp for p, pp, _ in table}
    out = {pid}
    while parent.get(pid, 0) > 1 and parent[pid] not in out:
        pid = parent[pid]
        out.add(pid)
    return out


def _lucerna_root(paths: BenchPaths, table) -> set[int]:
    try:
        rec = json.loads((paths.lucerna_home / PIDFILE_NAME).read_text())
        pid, started, exe = int(rec["pid"]), rec["lstart"], rec["exe"]
    except (OSError, ValueError, KeyError, TypeError):
        return set()
    cmd = next((c for p, _, c in table if p == pid), None)
    if cmd is None or not started or not exe or _lstart(pid) != started:
        return set()
    if not (cmd == exe or cmd.startswith(exe + " ")):
        return set()
    return {pid}


def _has_arg(command: str, flag: str, value: str) -> bool:
    # Exact value: must end at whitespace or end of line, so a sibling such as
    # `<bench>/profiles/zed-other` never matches `<bench>/profiles/zed`.
    pat = rf"(?:^|\s){re.escape(flag)}(?:\s+|=){re.escape(value)}(?=\s|$)"
    return re.search(pat, command) is not None


def _descendants(roots: set[int], table) -> set[int]:
    found = set(roots)
    grew = True
    while grew:
        grew = False
        for pid, ppid, _ in table:
            if ppid in found and pid not in found:
                found.add(pid)
                grew = True
    return found


def bench_pids(editor: str, paths: BenchPaths) -> list[int]:
    table = _ps()
    me = os.getpid()
    if editor == "vscode":
        user, _ = _vscode_dirs(paths)
        roots = {p for p, _, c in table if _has_arg(c, "--user-data-dir", user)}
    elif editor == "zed":
        zed = _zed_dir(paths)
        roots = {p for p, _, c in table if _has_arg(c, "--user-data-dir", zed)}
    elif editor == "lucerna":
        roots = _lucerna_root(paths, table)
    else:
        raise ValueError(f"unknown editor: {editor}")
    return sorted(_descendants(roots - _ancestors(me, table), table)
                  - _ancestors(me, table))


def _reap(pid: int) -> None:
    try:
        os.waitpid(pid, os.WNOHANG)
    except ChildProcessError:
        pass


def quit(editor: str, paths: BenchPaths) -> None:
    # Snapshot (pid -> start time) now: orphans reparented to pid 1 are still
    # killed later, and a reused pid (different start time) never is.
    # A pid without a start time is already gone: never signal it.
    snap = {}
    for pid in bench_pids(editor, paths):
        started = _lstart(pid)
        if started:
            snap[pid] = started

    def alive(pid: int) -> bool:
        return bool(snap[pid]) and _lstart(pid) == snap[pid]

    for pid in snap:
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    deadline = time.monotonic() + QUIT_GRACE_S
    while True:
        for pid in snap:
            _reap(pid)
        if time.monotonic() >= deadline or not any(alive(p) for p in snap):
            break
        time.sleep(0.1)
    for pid in snap:
        if alive(pid):
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            _reap(pid)


def app_bundle(editor: str) -> Path:
    if editor == "vscode":
        return VSCODE_APP
    if editor == "zed":
        return ZED_APP
    if editor == "lucerna":
        binary = Path(_lucerna_bin())
        if binary.parent.name == "MacOS" and binary.parents[1].name == "Contents":
            return binary.parents[2]
        return binary
    raise ValueError(f"unknown editor: {editor}")


def version(editor: str) -> str:
    if editor == "lucerna":
        # Cached with the binary: no subprocess during measurement. The build
        # time keys the background calibration to this exact binary.
        rec = resolve_lucerna()
        return f"{rec['version'] or 'unknown'} (build {rec['mtime']})"
    with open(app_bundle(editor) / "Contents" / "Info.plist", "rb") as f:
        return plistlib.load(f)["CFBundleShortVersionString"]
