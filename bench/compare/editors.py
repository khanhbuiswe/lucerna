"""Launch, identify and clean up each editor inside its own bench profile.

Safety: the user's own Zed / VS Code may be running. Only processes that carry
the bench profile path (and their descendants), or `luc` with the bench HOME,
are ever returned by `bench_pids`, and `quit` signals nothing else.
"""

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
ZED_CLI = ZED_APP / "Contents/MacOS/cli"
QUIT_GRACE_S = 5.0


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
        cmd = [str(ZED_CLI), "--user-data-dir", _zed_dir(paths), "--new", str(target)]
    elif editor == "lucerna":
        cmd = [_lucerna_bin(), str(target)]
        env = {**os.environ, "HOME": str(paths.lucerna_home)}
    else:
        raise ValueError(f"unknown editor: {editor}")
    t0 = time.monotonic_ns()
    popen = subprocess.Popen(cmd, env=env, stdout=subprocess.DEVNULL,
                             stderr=subprocess.DEVNULL)
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


def _ps_env(pid: int) -> str:
    """Command line followed by the environment (`ps eww`)."""
    return subprocess.run(["ps", "eww", "-o", "command=", "-p", str(pid)],
                          capture_output=True, text=True).stdout


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
        home = str(paths.lucerna_home)
        pat = rf"(?:^|\s)HOME={re.escape(home)}(?=\s|$)"
        roots = set()
        for pid, _, cmd in table:
            if re.match(r"\S*/luc(?:\s|$)", cmd) and re.search(pat, _ps_env(pid)):
                roots.add(pid)
    else:
        raise ValueError(f"unknown editor: {editor}")
    return sorted(_descendants(roots, table) - {me})


def _alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def quit(editor: str, paths: BenchPaths) -> None:
    pids = bench_pids(editor, paths)
    for pid in pids:
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    deadline = time.monotonic() + QUIT_GRACE_S
    while time.monotonic() < deadline and any(_alive(p) for p in pids):
        time.sleep(0.1)
    # Re-derive the list so a recycled PID is never killed by mistake.
    for pid in set(bench_pids(editor, paths)) & set(pids):
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


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
