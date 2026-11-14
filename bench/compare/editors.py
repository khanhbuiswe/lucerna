"""Launch, identify and clean up each editor inside its own bench profile.

Safety: the user's own Zed / VS Code may be running. Only processes that carry
the bench profile path (and their descendants), or the `luc` recorded in the
bench pidfile (and its descendants), are ever returned by `bench_pids`, and
`quit` signals nothing else.

Lucerna is found by pidfile, not by its HOME: macOS `ps` does not print another
process's environment, so a HOME match can never work. `launch` records the pid
and its start time; a reused pid has a different start time and is ignored.
"""

import json

import os
import plistlib
import re
import shutil
import signal
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path

from env import REPO_ROOT, BenchPaths

EDITORS = ("lucerna", "zed", "vscode")
VSCODE_APP = Path("/Applications/Visual Studio Code.app")
ZED_APP = Path("/Applications/Zed.app")
VSCODE_CLI = VSCODE_APP / "Contents/Resources/app/bin/code"
ZED_BIN = ZED_APP / "Contents/MacOS/zed"
# The process that owns the window. Launchers (`code`, Zed's `cli`) carry no
# window; Zed's `cli` would also start the app WITHOUT --user-data-dir.
MAIN_BINARY = {
    "vscode": str(VSCODE_APP / "Contents/MacOS/Code"),
    "zed": str(ZED_BIN),
}
QUIT_GRACE_S = 5.0
PIDFILE_NAME = ".bench-pids.json"


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


def _vscode_dirs(paths: BenchPaths) -> tuple[str, str]:
    return str(paths.profiles / "vscode"), str(paths.profiles / "vscode-ext")


def _zed_dir(paths: BenchPaths) -> str:
    return str(paths.profiles / "zed")


def _lucerna_bin() -> str:
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
        raise RuntimeError("luc not found: set LUCERNA_BIN or build the release bundle")
    return found


def launch(editor: str, paths: BenchPaths, target: Path) -> Launched:
    env = None
    if editor == "vscode":
        user, ext = _vscode_dirs(paths)
        cmd = [str(VSCODE_CLI), "--user-data-dir", user, "--extensions-dir", ext,
               "--new-window", str(target)]
    elif editor == "zed":
        # Run the main binary itself: `cli` hands off through LaunchServices
        # and drops --user-data-dir, so Zed would open the user's real profile.
        foreign = foreign_instances("zed", paths)
        if foreign:
            raise ForeignInstance(foreign_message("zed", foreign))
        cmd = [str(ZED_BIN), "--user-data-dir", _zed_dir(paths), str(target)]
    elif editor == "lucerna":
        cmd = [_lucerna_bin(), str(target)]
        env = {**os.environ, "HOME": str(paths.lucerna_home)}
    else:
        raise ValueError(f"unknown editor: {editor}")
    t0 = now_ns()
    popen = subprocess.Popen(cmd, env=env, stdout=subprocess.DEVNULL,
                             stderr=subprocess.DEVNULL)
    if editor == "lucerna":
        pidfile = paths.lucerna_home / PIDFILE_NAME
        pidfile.parent.mkdir(parents=True, exist_ok=True)
        pidfile.write_text(json.dumps({"pid": popen.pid, "lstart": _lstart(popen.pid)}))
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


def foreign_instances(editor: str, paths: BenchPaths) -> list[int]:
    """Main app processes of `editor` that are not the bench session's."""
    main = MAIN_BINARY.get(editor)
    if main is None:
        return []
    mine = set(bench_pids(editor, paths))
    found = []
    for pid, _, cmd in _ps():
        if pid in mine or not (cmd == main or cmd.startswith(main + " ")):
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
        pid, started = int(rec["pid"]), rec["lstart"]
    except (OSError, ValueError, KeyError, TypeError):
        return set()
    cmd = next((c for p, _, c in table if p == pid), None)
    if cmd is None or not started or _lstart(pid) != started:
        return set()
    if not re.match(r"\S*/luc(?:\s|$)", cmd):
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
    snap = {pid: _lstart(pid) for pid in bench_pids(editor, paths)}

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
        v = subprocess.run([_lucerna_bin(), "--version"], capture_output=True,
                           text=True).stdout.strip()
        sha = subprocess.run(["git", "rev-parse", "--short", "HEAD"],
                             cwd=REPO_ROOT, capture_output=True,
                             text=True).stdout.strip()
        return f"{v} ({sha})"
    with open(app_bundle(editor) / "Contents" / "Info.plist", "rb") as f:
        return plistlib.load(f)["CFBundleShortVersionString"]
