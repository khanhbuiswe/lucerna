#!/bin/sh
# Runs the benches the CI gate checks. `save` records them as the criterion
# baseline `main`; `compare` measures again against that baseline. A bench
# whose source file is missing (e.g. one a PR adds, run on the PR's base) is
# skipped. Benches are listed per crate so gpui-dependent crates are never
# built. Set BENCH_DRY_RUN=1 to print the commands instead of running them.
# Usage: scripts/bench-run.sh <save|compare>   (from the repo root)
set -eu

case ${1:-} in
  save) flag=--save-baseline ;;
  # Lenient: a bench the base lacks (new or renamed id) is measured without a
  # baseline instead of panicking, and bench-gate.sh reports it as NEW.
  compare) flag=--baseline-lenient ;;
  *)
    echo "usage: $0 <save|compare>" >&2
    exit 2
    ;;
esac

for pair in lucerna-project:scan lucerna-project:fuzzy lucerna-syntax:highlight lucerna-text:reload; do
  crate=${pair%%:*}
  bench=${pair#*:}
  if [ ! -f "crates/$crate/benches/$bench.rs" ]; then
    echo "skip $crate/$bench: no benches/$bench.rs"
    continue
  fi
  if [ "${BENCH_DRY_RUN:-}" = 1 ]; then
    echo "cargo bench -p $crate --bench $bench -- $flag main"
  else
    cargo bench -p "$crate" --bench "$bench" -- "$flag" main
  fi
done
