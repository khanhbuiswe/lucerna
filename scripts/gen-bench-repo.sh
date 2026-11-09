#!/bin/sh
# Generates the deterministic benchmark repo: 4,000 dirs x 20 files (80,000
# files, Rust- and TS-like content) plus a ~10 MB `big.rs`.
# Usage: scripts/gen-bench-repo.sh <dir>
set -eu

[ $# -eq 1 ] || { echo "usage: $0 <dir>" >&2; exit 2; }
dir=$1
mkdir -p "$dir"

# Content depends only on the dir/file indices: no randomness, no clock.
# Create all directories with one `mkdir -p` (names come from awk on stdin,
# NUL-separated, so no path character needs escaping).
BENCH_ROOT=$dir awk 'BEGIN {
  root = ENVIRON["BENCH_ROOT"]
  for (d = 0; d < 4000; d++)
    printf "%s/pkg%02d/mod%04d%c", root, int(d / 100), d, 0
}' | xargs -0 mkdir -p

BENCH_ROOT=$dir awk 'BEGIN {
  root = ENVIRON["BENCH_ROOT"]
  for (d = 0; d < 4000; d++) {
    sub_dir = sprintf("%s/pkg%02d/mod%04d", root, int(d / 100), d)
    for (f = 0; f < 20; f++) {
      if (f % 2 == 0) {
        path = sprintf("%s/item%02d.rs", sub_dir, f)
        printf "//! Module %d item %d.\n\npub struct Item%d_%d {\n    pub id: u32,\n    pub name: String,\n}\n\nimpl Item%d_%d {\n    pub fn new(id: u32) -> Self {\n        Self { id, name: format!(\"item-{id}\") }\n    }\n}\n", d, f, d, f, d, f > path
      } else {
        path = sprintf("%s/item%02d.ts", sub_dir, f)
        printf "// Module %d item %d.\nexport interface Item%d_%d {\n  id: number;\n  name: string;\n}\n\nexport function make%d_%d(id: number): Item%d_%d {\n  return { id, name: `item-${id}` };\n}\n", d, f, d, f, d, f, d, f > path
      }
      close(path)
    }
  }
  # ~10 MB of Rust: ~126-byte function blocks, repeated with the index.
  big = root "/big.rs"
  printf "" > big
  i = 0
  while (i < 83000) {
    printf "pub fn generated_%d(a: u64, b: u64) -> u64 {\n    let c = a.wrapping_mul(%d) ^ b; // line %d\n    c.rotate_left(%d)\n}\n\n", i, i * 7 + 1, i, i % 63 >> big
    i++
  }
  close(big)
}'
