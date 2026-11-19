#!/bin/sh
# Tests scripts/bench-run.sh in dry-run mode against a temp tree where the
# lucerna-text reload bench is missing, so it must be skipped.
# Usage: sh scripts/test-bench-run.sh
set -eu

script="$(cd "$(dirname "$0")" && pwd)/bench-run.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

mkdir -p "$tmp/crates/lucerna-project/benches" "$tmp/crates/lucerna-syntax/benches"
touch "$tmp/crates/lucerna-project/benches/scan.rs" \
  "$tmp/crates/lucerna-project/benches/fuzzy.rs" \
  "$tmp/crates/lucerna-syntax/benches/highlight.rs"
cd "$tmp"

fail() {
  echo "FAIL: $1" >&2
  exit 1
}

expect() {
  mode=$1
  flag=$2
  got=$(BENCH_DRY_RUN=1 sh "$script" "$mode") || fail "$mode exited non-zero"
  want="cargo bench -p lucerna-project --bench scan -- $flag main
cargo bench -p lucerna-project --bench fuzzy -- $flag main
cargo bench -p lucerna-syntax --bench highlight -- $flag main
skip lucerna-text/reload: no benches/reload.rs"
  [ "$got" = "$want" ] || fail "$mode output:
$got"
}

expect save --save-baseline
expect compare --baseline-lenient

status=0
sh "$script" bogus 2>/dev/null || status=$?
[ "$status" -eq 2 ] || fail "bogus exited $status, want 2"

echo ok
