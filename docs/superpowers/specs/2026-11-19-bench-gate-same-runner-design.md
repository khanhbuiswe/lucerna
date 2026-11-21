# Bench gate: measure main and the PR on the same runner

## Why
The `bench` job in `.github/workflows/ci.yml` compares a pull request against a
criterion baseline that an earlier push to `main` saved to the Actions cache.
The two runs land on different shared macOS runners at different times. On PR #1
(tests only) every benchmark came out 20–80% slower, far past the 15% gate, so
the gate fails on runner noise rather than on real regressions. The gate should
block merges only for slowdowns the PR causes.

## Goals / non-goals
- Goal: a PR's benchmarks are compared with its base commit measured in the
  same job, on the same machine, minutes apart.
- Goal: show, on every PR, which benchmarks are more than 15% slower than the base (report only; see the update below).
- Non-goal: changing which benchmarks run, or the 15% threshold.
- Non-goal: instruction-count benchmarks (e.g. callgrind); they would need a
  Linux runner and new bench harnesses.
- Anti-goal: a gate that still fails on tests-only or docs-only PRs.

## Approaches
1. **Same job, base then head (picked).** Benchmark the PR's base commit
   with `--save-baseline main`, switch to the PR commit, benchmark with
   `--baseline main`, then run `bench-gate.sh`. Removes cross-machine noise;
   the job takes about twice as long (~5 → ~10 min).
2. Report only: keep the cached baseline but make the gate non-blocking.
   Cheap, but real regressions no longer block a merge.
3. Raise the threshold. Noise seen was up to 80%, so any threshold that
   passes it would miss real regressions.

## Design
- The `bench` job runs only on `pull_request`; the push-to-main baseline steps
  and the `actions/cache` save/restore steps are removed.
- Checkout uses enough history to reach `github.event.pull_request.base.sha`
  (`fetch-depth: 0`, or an explicit fetch of that SHA).
- Generate the bench repo once with the PR's `scripts/gen-bench-repo.sh`, so
  both runs read the same input.
- Check out the base SHA in the same working tree, run the three
  `cargo bench ... -- --save-baseline main` commands; check out the PR commit
  again, run the three `--baseline main` commands; run
  `scripts/bench-gate.sh target/criterion 15`. `target/` is untracked, so the
  criterion data survives the checkouts.

## Left to the builder (Claude's calls)
- A bench target the PR adds does not exist on the base, so the base run must
  not fail on it: run base benches per target and skip ones that are missing.
  `bench-gate.sh` already prints `NEW (no baseline)` for those.
- Whether base or head runs first. Default: base first; revisit if a
  consistent order bias shows up.

## Update after the first run (PR #2)
PR #2 changes no Rust code, yet on one runner the PR run differed from the base
run by -17% to +31% (`fuzzy_80k/p12m1234i05ts` -17.3%, `scan_80k/scan` +31.3%,
`highlight_markdown_viewport` +21.4%, `markdown_many_fences` +16.6%). Same-runner
noise on shared macOS runners is still above the 15% threshold, so the gate
cannot block merges without failing on noise. Decision (user): report only. The
job writes the `bench-gate.sh` table to the job summary and raises a warning
annotation when a bench is over 15% slower; it never fails the PR. A blocking
gate would need a quieter measure, e.g. instruction counts on a Linux runner.
