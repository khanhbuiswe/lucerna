#!/bin/sh
# Fails if any criterion benchmark is slower than its saved `main` baseline by
# more than <threshold> percent (default 15). Run after
# `cargo bench ... -- --baseline main`, which leaves `new/` next to `main/`.
# Usage: scripts/bench-gate.sh [criterion-dir] [threshold-percent]
set -eu

dir=${1:-"$("$(dirname "$0")"/target-dir.sh)/criterion"}
threshold=${2:-15}
status=0
checked=0

for new in $(find "$dir" -path '*/new/estimates.json' | sort); do
  bench_dir=$(dirname "$(dirname "$new")")
  base="$bench_dir/main/estimates.json"
  name=${bench_dir#"$dir"/}
  if [ ! -f "$base" ]; then
    echo "NEW (no baseline): $name"
    continue
  fi
  old_ns=$(jq '.median.point_estimate' "$base")
  new_ns=$(jq '.median.point_estimate' "$new")
  checked=$((checked + 1))
  verdict=$(awk -v o="$old_ns" -v n="$new_ns" -v t="$threshold" \
    'BEGIN { p = (n - o) / o * 100; printf "%+.1f%% %s", p, (p > t ? "FAIL" : "ok") }')
  echo "$name: $verdict (main ${old_ns} ns, now ${new_ns} ns)"
  case $verdict in *FAIL) status=1 ;; esac
done

if [ "$checked" -eq 0 ]; then
  echo "bench-gate: no benchmarks compared under $dir" >&2
  exit 1
fi
exit $status
