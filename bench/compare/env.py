"""Clean profiles, benchmark data and machine preflight for the comparison."""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path

DEFAULT_ROOT = Path.home() / "LucernaBench.noindex"
FONT_SIZE = 14
RIPGREP_URL = "https://github.com/BurntSushi/ripgrep.git"
RIPGREP_TAG = "14.1.1"
REPO_ROOT = Path(__file__).resolve().parents[2]
GEN_SCRIPT = REPO_ROOT / "scripts" / "gen-bench-repo.sh"
VSCODE_RUST_ANALYZER = "rust-lang.rust-analyzer"
VSCODE_CLI = "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code"
LOAD_LIMIT = 2.0
PIDFILE_NAME = ".bench-pids.json"     # editors.py: the bench `luc` (in lucerna-home)
PROFILE_MARKER = "bench-profile.json"  # which config the profiles were written for
# Kept across configs: installing rust-analyzer needs the network.
KEEP_PROFILES = ("vscode-ext-lsp",)


@dataclass(frozen=True)
class BenchPaths:
    root: Path

    @property
    def profiles(self) -> Path:
        return self.root / "profiles"

    @property
    def data(self) -> Path:
        return self.root / "data"

    @property
    def results(self) -> Path:
        return self.root / "results"

    @property
    def large_repo(self) -> Path:
        return self.data / "large"

    @property
    def medium_repo(self) -> Path:
        return self.data / "medium"

    @property
    def big_file(self) -> Path:
        # scripts/gen-bench-repo.sh writes ~10 MB `big.rs` at the repo root.
        return self.large_repo / "big.rs"

    @property
    def marker_file(self) -> Path:
        return self.data / "marker.txt"

    @property
    def lucerna_home(self) -> Path:
        return self.profiles / "lucerna-home"


def prepare_data(paths: BenchPaths) -> None:
    """Large generated repo, medium ripgrep clone and the marker file."""
    paths.data.mkdir(parents=True, exist_ok=True)
    if not paths.big_file.exists():
        subprocess.run([str(GEN_SCRIPT), str(paths.large_repo)], check=True)
    if not paths.medium_repo.exists():
        subprocess.run(
            ["git", "clone", "--depth", "1", "--branch", RIPGREP_TAG,
             RIPGREP_URL, str(paths.medium_repo)],
            check=True,
        )
    paths.marker_file.write_text(("█" * 80 + "\n") * 200, encoding="utf-8")


def _write_json(path: Path, value) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def vscode_ext_dir(paths: BenchPaths, lsp: bool) -> Path:
    """VS Code extensions: `vscode-ext` stays empty (no-LSP config, also
    launched with --disable-extensions); `vscode-ext-lsp` holds rust-analyzer."""
    return paths.profiles / ("vscode-ext-lsp" if lsp else "vscode-ext")


def install_vscode_rust_analyzer(paths: BenchPaths) -> None:
    """Needs the network. Not called by tests; run once before `lsp=True` runs.
    The CLI (`bin/code`) is fine here: this is setup, not a measurement."""
    subprocess.run(
        [VSCODE_CLI, "--user-data-dir", str(paths.profiles / "vscode"),
         "--extensions-dir", str(vscode_ext_dir(paths, True)),
         "--install-extension", VSCODE_RUST_ANALYZER],
        check=True,
    )


def vscode_rust_analyzer_installed(paths: BenchPaths) -> bool:
    d = vscode_ext_dir(paths, True)
    return d.is_dir() and any(p.is_dir() for p in d.glob(VSCODE_RUST_ANALYZER + "*"))


def reset_profiles(paths: BenchPaths) -> list[Path]:
    """Delete every editor profile under <bench>/profiles except
    KEEP_PROFILES, and everything in lucerna-home but the bench pidfile, so
    each config starts from a clean profile. Refuses to delete anything that
    does not resolve inside <bench>/profiles. Returns what was removed."""
    root = paths.profiles.resolve()
    if not paths.profiles.is_dir():
        return []
    victims = []
    for entry in sorted(paths.profiles.iterdir()):
        if entry.name in KEEP_PROFILES:
            continue
        if entry.name == paths.lucerna_home.name and entry.is_dir() and not entry.is_symlink():
            victims += [c for c in sorted(entry.iterdir()) if c.name != PIDFILE_NAME]
        else:
            victims.append(entry)
    for v in victims:
        # A symlink is removed itself, never followed.
        parent = v.parent.resolve()
        if not (parent == root or root in parent.parents) or v.name in ("", ".", ".."):
            raise RuntimeError(f"refusing to delete {v}: not under {root}")
        if v.is_dir() and not v.is_symlink():
            shutil.rmtree(v)
        else:
            v.unlink()
    return victims


def profile_lsp(paths: BenchPaths) -> bool:
    """The `lsp` the profiles were last written for (False if unknown)."""
    try:
        return bool(json.loads((paths.profiles / PROFILE_MARKER).read_text())["lsp"])
    except (OSError, ValueError, KeyError, TypeError):
        return False


def write_profiles(paths: BenchPaths, lsp: bool) -> None:
    ra = shutil.which("rust-analyzer") if lsp else None
    if lsp and not ra:
        raise RuntimeError("rust-analyzer not found on PATH")

    vscode = {
        "telemetry.telemetryLevel": "off",
        "update.mode": "none",
        "security.workspace.trust.enabled": False,
        "workbench.startupEditor": "none",
        "window.restoreWindows": "none",
        "editor.fontSize": FONT_SIZE,
        "editor.minimap.enabled": False,
        "extensions.autoCheckUpdates": False,
        # Quick Open opens the idle-RAM files: each must stay a real tab.
        "workbench.editor.enablePreviewFromQuickOpen": False,
    }
    if lsp:
        vscode["rust-analyzer.server.path"] = ra
    _write_json(paths.profiles / "vscode" / "User" / "settings.json", vscode)
    vscode_ext_dir(paths, False).mkdir(parents=True, exist_ok=True)

    zed = {
        "telemetry": {"diagnostics": False, "metrics": False},
        "auto_update": False,
        "buffer_font_size": FONT_SIZE,
        "disable_ai": True,
        "restore_on_startup": "none",
        "minimap": {"show": "never"},
        "preview_tabs": {"enable_preview_from_file_finder": False},
        # Zed installs the html extension by default and suggests others:
        # no downloads into the bench profile.
        "auto_install_extensions": {"html": False},
        "suggest_extensions": False,
    }
    if lsp:
        zed["lsp"] = {"rust-analyzer": {"binary": {"path": ra}}}
    else:
        zed["languages"] = {"Rust": {"language_servers": []}}
    _write_json(paths.profiles / "zed" / "config" / "settings.json", zed)

    luc = {
        "editor.fontSize": FONT_SIZE,
        "workbench.startupEditor": "none",
        "lucerna.lsp.enabled": lsp,
        # Lucerna's Quick Open always opens a preview; no narrower key exists.
        "workbench.editor.enablePreview": False,
    }
    if lsp:
        luc["lsp"] = {"rust-analyzer": {"binary": {"path": ra}}}
    home = paths.lucerna_home
    _write_json(home / ".config" / "lucerna" / "settings.json", luc)
    data = str(paths.data)
    _write_json(
        home / "Library" / "Application Support" / "Lucerna" / "trusted.json",
        {"folders": [data], "parents": [data]},
    )
    _write_json(paths.profiles / PROFILE_MARKER, {"lsp": lsp})


def _processes() -> list[str]:
    out = subprocess.run(["ps", "-axo", "comm="], capture_output=True, text=True)
    return [os.path.basename(line.strip()) for line in out.stdout.splitlines()]


def _pmset(args: list[str]) -> str:
    return subprocess.run(["pmset", *args], capture_output=True, text=True).stdout


def preflight() -> list[str]:
    """Reasons the machine is not quiet; empty means ready."""
    reasons = []
    busy = sorted({p for p in _processes() if p in ("cargo", "rustc")})
    if busy:
        reasons.append("build running: " + ", ".join(busy))
    load = os.getloadavg()[0]
    if load > LOAD_LIMIT:
        reasons.append(f"1-minute load average {load:.2f} > {LOAD_LIMIT}")
    m = re.search(r"CPU_Speed_Limit\s*=\s*(\d+)", _pmset(["-g", "therm"]))
    if m and int(m.group(1)) < 100:
        reasons.append(f"CPU_Speed_Limit {m.group(1)} < 100 (thermal throttling)")
    if "AC Power" not in _pmset(["-g", "batt"]):
        reasons.append("not on AC power")
    if re.search(r"^\s*lowpowermode\s+1\b", _pmset(["-g"]), re.M):
        reasons.append("Low Power Mode is on")
    return reasons
