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

`site/index.html` quotes the four numbers above and the date they were measured. When you re-measure, update the Speed section of the landing page in the same PR.

The benches build the 10 MB text in memory; `big.rs` from the generator is
for manual opening in `luc`.

## Manual (release build, not yet run)

| Target | How to measure | Result |
|---|---|---|
| First frame | `target/release/luc --trace-startup /tmp/lucerna-bench` prints `startup_ms=<n>` on stderr (also logged as `first frame painted`). Run once cold (after `sudo purge`) and a few times warm. | **147 ms** warm (145–160 ms over 4 runs); 704 ms on the very first launch right after the build (cold disk cache). 2026-11-09, release build. |
| Tree usable, scan done | See "Commands" below: `first scan progress` is the tree usable, `scan complete` is scan done; both carry `scan_ms`, the ms since the worktree started (first scan only). | **58 ms** to first tree batch, **1215 ms** scan complete on the 80,001-file bench repo. 2026-11-09, release build. |
| Quick Open | `fuzzy_80k` bench, per call. | see table above |
| Scrolling 120 fps | Open `/tmp/lucerna-bench/big.rs`, then Instruments > Core Animation (FPS), attach to `luc` and hold a trackpad scroll or Page Down. Expect a steady 120 fps on a ProMotion display. | pending |
| Idle RAM | Open 10 tabs from `/tmp/lucerna-bench`, wait 60 s idle, read the "Memory" column for `luc` in Activity Monitor. | **113 MB** RSS idle on the 80k-file bench repo (no tabs); 100 MB on this repo. Not yet measured with 10 tabs. 2026-11-09. |
| Disk to screen | See "Commands" below: the time from the `document reloaded` log line to the next `first render of revision` line for the same file, both stamped by the same clock. | **10–16 ms** from `document reloaded` to `first render of revision` (3 writes). Watcher latency before the reload (50 ms debounce + FSEvents) not separately measured. 2026-11-09. |

### Commands

Tree usable / scan done (default `info` level; prints `scan_ms=<n>`):

```sh
target/release/luc /tmp/lucerna-bench
grep -hE 'first scan progress|scan complete' ~/Library/Logs/Lucerna/lucerna.*.log | tail -2
```

Disk to screen. Use a reload-only flow: `first render of revision` also fires
for the first load and other revision bumps, so open the file first, let it
render, then write to it once:

```sh
LUCERNA_LOG=debug target/release/luc /tmp/lucerna-bench
# in luc: open pkg00/mod0000/item00.rs; then in another shell:
echo '// x' >> /tmp/lucerna-bench/pkg00/mod0000/item00.rs
grep -hE 'document reloaded|first render of revision' ~/Library/Logs/Lucerna/lucerna.*.log | tail -2
```

The two lines carry RFC 3339 UTC timestamps with microseconds. Disk to screen
is the second timestamp minus the first (reload finished to first frame). To
include the watcher latency, take the write time from the same clock:

```sh
python3 -c 'import datetime; print(datetime.datetime.now(datetime.timezone.utc).isoformat())'
```

Run it immediately before the `echo` and subtract it from the
`first render of revision` timestamp.

### Debug-build smoke numbers (Task 20, for reference only)

These are from a debug build and say nothing about the release targets:

- `startup_ms=510` cold, 148 warm.
- A reload was logged about 69 ms after a write.

### Follow-ups required before Phase 1 is declared done

- Run the manual rows on a release build and fill in the results above.
- Any manual row that misses its target gets a follow-up task in the plan.
