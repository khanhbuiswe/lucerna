#!/bin/sh
# Keeps target/ small. Each `cargo test` after a code change links new test
# binaries in target/<profile>/deps and leaves the old ones behind, so the
# folder grows by gigabytes a day. Run this before `cargo test` or a build.
#
# Usage: scripts/prune-target.sh [limit-in-GB]   (default 5)
#
# Over the limit, it deletes old test binaries (and their .dSYM folders),
# keeping the newest one of each name. Still over the limit, it removes the
# whole profile folder (the next build starts from scratch). Does nothing
# while another cargo runs, since another session may be using target/.
set -eu

limit_gb=${1:-5}
root=$(cd "$(dirname "$0")/.." && pwd)
target=$("$root/scripts/target-dir.sh")
limit_kb=$((limit_gb * 1024 * 1024))

size_kb() { du -sk "$1" 2>/dev/null | cut -f1; }

if pgrep -x cargo >/dev/null 2>&1; then
  echo "prune-target: another cargo is running; nothing removed" >&2
  exit 0
fi

for profile in debug release; do
  dir="$target/$profile"
  [ -d "$dir" ] || continue
  before=$(size_kb "$dir")
  [ "$before" -gt "$limit_kb" ] || continue

  deps="$dir/deps"
  if [ -d "$deps" ]; then
    # Test binaries are executables named <name>-<16 hex>, with no extension.
    # Newest first, so the first one seen of each name is kept.
    ls -t "$deps" | grep -E '^[A-Za-z0-9_]+-[0-9a-f]{16}$' | while read -r file; do
      [ -f "$deps/$file" ] && [ -x "$deps/$file" ] || continue
      name=${file%-*}
      if [ -e "$deps/.keep-$name" ]; then
        rm -rf "$deps/$file" "$deps/$file.dSYM"
      else
        : > "$deps/.keep-$name"
      fi
    done || true
    rm -f "$deps"/.keep-*
  fi

  after=$(size_kb "$dir")
  if [ "$after" -gt "$limit_kb" ]; then
    echo "prune-target: $profile still $((after / 1024)) MB after pruning; removing $dir"
    rm -rf "$dir"
    after=0
  fi
  echo "prune-target: $profile $((before / 1024)) MB -> $((after / 1024)) MB (limit ${limit_gb} GB)"
done
