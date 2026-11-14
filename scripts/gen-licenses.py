#!/usr/bin/env python3
"""Writes the third-party license notices shipped in Lucerna.app.

Usage: scripts/gen-licenses.py OUTPUT.md

MIT, Apache-2.0, BSD and the other licenses `deny.toml` allows require the
copyright notice and license text to travel with the binary. This lists
every crate the `luc` binary links (normal dependencies of the `lucerna`
crate, for macOS on Apple silicon), grouped by identical license text, and
the fonts, icons and theme compiled into it. It reads `cargo metadata` and
the license files inside each crate's published source, so it needs no tool
beyond cargo and python3. Research: docs/research/2026-11-13-third-party-licenses.md.
"""

import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TARGET = "aarch64-apple-darwin"
LICENSE_FILE = re.compile(r"^(LICEN[CS]E|COPYING|COPYRIGHT|NOTICE|UNLICENSE)", re.I)

# Assets compiled into the binary, with the license file that covers each.
ASSETS = [
    ("Lilex font", "crates/lucerna-workbench/assets/fonts/LICENSE-Lilex"),
    ("IBM Plex fonts", "crates/lucerna-workbench/assets/fonts/LICENSE-IBM-Plex"),
    ("Lucide icons", "crates/lucerna-workbench/assets/icons/LICENSE-lucide"),
    ("GitHub Primer colors (Lucerna themes)", "crates/lucerna-theme/assets/LICENSE-primer"),
]


def metadata():
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked",
         "--filter-platform", TARGET],
        cwd=ROOT, check=True, capture_output=True, text=True,
    ).stdout
    return json.loads(out)


def linked_packages(meta):
    """The packages reachable from `lucerna` through normal dependencies,
    without the workspace's own crates."""
    packages = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    workspace = set(meta["workspace_members"])
    start = next(i for i in workspace if packages[i]["name"] == "lucerna")
    seen, stack = set(), [start]
    while stack:
        node = nodes[stack.pop()]
        for dep in node["deps"]:
            if not any(k["kind"] is None for k in dep["dep_kinds"]):
                continue
            if dep["pkg"] not in seen:
                seen.add(dep["pkg"])
                stack.append(dep["pkg"])
    return sorted(
        (packages[i] for i in seen if i not in workspace),
        key=lambda p: (p["name"], p["version"]),
    )


def license_texts(package):
    """The text of each license file at the top of the crate, by file name."""
    folder = Path(package["manifest_path"]).parent
    texts = []
    for entry in sorted(os.listdir(folder)):
        path = folder / entry
        if LICENSE_FILE.match(entry) and path.is_file():
            try:
                text = path.read_text(encoding="utf-8", errors="replace").strip()
            except OSError:
                continue
            if text:
                texts.append(text)
    return texts


def fence(text):
    """A code fence longer than any backtick run in `text`."""
    longest = max((len(m) for m in re.findall(r"`+", text)), default=0)
    return "`" * max(3, longest + 1)


def render(packages, root):
    groups = {}  # license text -> crates using it
    bare = []  # crates whose published source has no license file
    for p in packages:
        label = f"{p['name']} {p['version']}"
        texts = license_texts(p)
        if not texts:
            bare.append(p)
        for text in texts:
            groups.setdefault(text, []).append(label)

    lines = [
        "# Third-party licenses",
        "",
        "Lucerna is licensed under MIT OR Apache-2.0. It includes the software",
        "below, whose licenses require these notices to accompany it.",
        "",
        "## Fonts, icons and themes",
        "",
    ]
    for title, rel in ASSETS:
        text = (root / rel).read_text(encoding="utf-8").strip()
        f = fence(text)
        lines += [f"### {title}", "", f, text, f, ""]

    lines += ["## Rust crates", ""]
    ordered = sorted(groups.items(), key=lambda kv: (-len(kv[1]), kv[1][0]))
    for text, crates in ordered:
        f = fence(text)
        lines += [f"### Used by: {', '.join(crates)}", "", f, text, f, ""]

    if bare:
        lines += [
            "## Crates without a license file",
            "",
            "These crates declare their license in Cargo.toml but publish no",
            "license file; their license and source are:",
            "",
        ]
        for p in bare:
            where = p.get("repository") or "crates.io"
            lines.append(f"- {p['name']} {p['version']}: {p.get('license') or 'unknown'} ({where})")
        lines.append("")
        # Each of them is offered under MIT or Apache-2.0 (checked below), so
        # the standard texts of both follow, copyright held by the authors
        # named in each crate's repository.
        lines += ["### Standard license texts", ""]
        for title, text in standard_texts(root):
            f = fence(text)
            lines += [f"#### {title}", "", f, text, f, ""]
    return "\n".join(lines)


def standard_texts(root):
    """Apache-2.0 from the repository's own LICENSE-APACHE (the unmodified
    text), and MIT from LICENSE-MIT without its copyright line."""
    apache = (root / "LICENSE-APACHE").read_text(encoding="utf-8").strip()
    mit = (root / "LICENSE-MIT").read_text(encoding="utf-8").strip().splitlines()
    mit = "\n".join(line for line in mit if not line.startswith("Copyright")).strip()
    return [("MIT", mit), ("Apache-2.0", apache)]


def main():
    if len(sys.argv) != 2:
        sys.exit("usage: scripts/gen-licenses.py OUTPUT.md")
    out = Path(sys.argv[1])
    packages = linked_packages(metadata())
    for p in packages:
        spdx = p.get("license") or ""
        if not license_texts(p) and "MIT" not in spdx and "Apache-2.0" not in spdx:
            sys.exit(f"gen-licenses: {p['name']} {p['version']} ships no license file "
                     f"and is not MIT or Apache-2.0 ({spdx or 'no license'}); add its text")
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(render(packages, ROOT), encoding="utf-8")
    print(f"gen-licenses: {len(packages)} crates -> {out}")


if __name__ == "__main__":
    main()
