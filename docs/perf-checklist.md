# Lucerna performance checklist (Phase 1)

Targets from the spec, how each is measured, and the latest numbers. The
automated rows run in CI (`bench` job) and are gated at +15% against `main`.
The manual rows need a release build of `luc` on real hardware and have **not
been run yet**.

## Setup

```sh
scripts/gen-bench-repo.sh /tmp/lucerna-bench     # 80,000 files + 10 MB big.rs, deterministic
export LUCERNA_BENCH_REPO=/tmp/lucerna-bench
cargo build --release -p lucerna                 # for the manual rows
```

## Automated (criterion)

Run per crate so the gpui-dependent crates are not built:

```sh
cargo bench -p lucerna-project --bench scan --bench fuzzy
cargo bench -p lucerna-syntax --bench highlight
cargo bench -p lucerna-text --bench reload
```

Compare against a saved baseline (what CI does on pull requests):

```sh
cargo bench -p lucerna-project --bench fuzzy -- --save-baseline main
# ...change code...
cargo bench -p lucerna-project --bench fuzzy -- --baseline main
scripts/bench-gate.sh target/criterion 15        # needs jq; exit 1 if any bench is >15% slower
```

| Target | Bench group | Limit | Measured (Apple Silicon dev machine, 2026-11-09) |
|---|---|---|---|
| Tree usable, scan done | `scan_80k` (`Worktree::start` to `ScanComplete`) | < 5 s | 887 ms (median 877-899 ms) |
| Quick Open | `fuzzy_80k` (one `match_files` call, limit 50, 80,000 files) | < 16 ms | 2.04 ms (`item07`), 1.80 ms (`p12m1234i05ts`) |
| Highlight a viewport of a 10 MB file | `highlight_10mb_viewport` (60 rows, tree pre-parsed) | informational | 96 us |
| Reload a 10 MB buffer | `reload_10mb` (tail changed) | informational | 51 ms |

The benches build the 10 MB text in memory; `big.rs` from the generator is
for manual opening in `luc`.

## Manual (release build, not yet run)

| Target | How to measure | Result |
|---|---|---|
| First frame | `target/release/luc --trace-startup /tmp/lucerna-bench` prints `startup_ms=<n>` on stderr (also logged as `first frame painted`). Run once cold (after `sudo purge`) and a few times warm. | pending |
| Tree usable, scan done | The app does not log `ScanProgress`/`ScanComplete` yet. Use `scan_80k` above as the scan figure; for the end-to-end figure add `tracing::info!` lines where the workbench consumes those events, then read them with `LUCERNA_LOG=info` in `~/Library/Logs/Lucerna/lucerna.*.log`. | pending (needs logging) |
| Quick Open | `fuzzy_80k` bench, per call. | see table above |
| Scrolling 120 fps | Open `/tmp/lucerna-bench/big.rs`, then Instruments > Core Animation (FPS), attach to `luc` and hold a trackpad scroll or Page Down. Expect a steady 120 fps on a ProMotion display. | pending |
| Idle RAM | Open 10 tabs from `/tmp/lucerna-bench`, wait 60 s idle, read the "Memory" column for `luc` in Activity Monitor. | pending |
| Disk to screen | `LUCERNA_LOG=debug target/release/luc /tmp/lucerna-bench`, open a file, then `echo x >> <file>`. The `document reloaded` line in `~/Library/Logs/Lucerna/lucerna.*.log` carries a timestamp; subtract the write time (`date +%s.%N` before the write). Repaint is not logged, so add a log at repaint for the full figure. | pending |

### Debug-build smoke numbers (Task 20, for reference only)

These are from a debug build and say nothing about the release targets:

- `startup_ms=510` cold, 148 warm.
- A reload was logged about 69 ms after a write.

### Follow-ups required before Phase 1 is declared done

- Run the manual rows on a release build and fill in the results above.
- Any manual row that misses its target gets a follow-up task in the plan.
- Add scan and repaint log lines (see the two rows marked "needs logging").
