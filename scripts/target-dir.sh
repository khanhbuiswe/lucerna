#!/bin/sh
# Prints cargo's target directory for this workspace: where builds really
# land, which is not always ./target. CARGO_TARGET_DIR or a `build.target-dir`
# in a .cargo/config.toml (in this folder or any parent) can move it, for
# example so every git worktree under .claude/worktrees/ shares one build
# folder instead of each growing its own.
#
# Usage: target=$(scripts/target-dir.sh)
# Falls back to <workspace>/target when cargo is missing or fails.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
dir=
if command -v cargo >/dev/null 2>&1; then
  dir=$(cd "$root" && cargo metadata --format-version 1 --no-deps --offline 2>/dev/null |
    sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p') || dir=
fi
echo "${dir:-$root/target}"
