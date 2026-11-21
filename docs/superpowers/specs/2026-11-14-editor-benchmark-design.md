# Lucerna, Zed and VS Code performance comparison — design

Date: 2026-11-14. Status: Approved.

## Intent

Measure Lucerna next to Zed and VS Code on the same machine, in a clean
environment, then **publish** the results (README, site). Because the
results are published, every number must be reproducible: the scripts,
data, versions and machine configuration are all public, and the
measurement method is fair to all three editors.

It also checks the performance targets in §6 of
`2026-11-08-lucerna-phase1-design.md` on the build about to be released.

## Goals

- A toolkit in `bench/compare/`, run with one command, producing result
  files (JSON and a Markdown table).
- Each editor runs with its own empty profile and never touches the user's
  real data.
- The same measurement method, taken from outside the app, for all three
  editors.
- A methodology page (`docs/benchmarks.md`) that describes the setup well
  enough for someone else to rerun it.

## Non-goals

- Measuring features Lucerna does not have (AI, debugging, the VS Code
  extension host).
- Comparing Lucerna's internal measurements with another editor's external
  measurements.
- Optimizing specifically for the benchmark.

## Anti-goals

- A number that cannot be reproduced, or a configuration that disadvantages
  Zed/VS Code without saying so.
- Wording that disparages Zed or VS Code. Per `CLAUDE.md`: state the
  numbers, stay neutral.

## Clean environment

**A separate profile for each editor**, in `<bench>/profiles/`:

| Editor | How it runs |
|---|---|
| VS Code | `code --user-data-dir <bench>/profiles/vscode --extensions-dir <bench>/profiles/vscode-ext`; settings: telemetry, updates, workspace trust and the welcome page off, git auto-scan left at the default |
| Zed | `zed --user-data-dir <bench>/profiles/zed` (check the flag on the installed version); settings: telemetry, updates and AI features off |
| Lucerna | `HOME=<bench>/profiles/lucerna-home luc …` |

Shared settings: the same window size, font size 14, each editor's default
theme, minimap off if there is one (stated explicitly), no restoring the
previous session.

**Fixed data** in `<bench>/data.noindex/` (Spotlight skips the `.noindex`
suffix):
- A repo of 80,000 files and `big.rs` of ~10 MB: `scripts/gen-bench-repo.sh`.
- A medium repo of ~5,000 files at a fixed commit (proposal: ripgrep or a
  Rust repo of similar size, chosen when writing the plan) for idle RAM.

**Machine:**
- Plugged in, Low Power Mode off, other apps closed, every session/cargo
  stopped.
- Check before running: no `cargo`/`rustc` running, low CPU load,
  `pmset -g therm` reports no throttling; if the check fails, the script
  stops.
- Rest between runs so the machine cools down; rotate the editor order
  between runs.
- Two modes: cold (after `sudo purge`, run by the user) and warm (drop the
  first run).

**Two configurations:** no language server; and the same rust-analyzer
(path and version recorded in the results).

## Measurements

Everything is measured from outside, the same way for all three editors. 10
runs per measurement; report the median and p90.

| Metric | How it is measured |
|---|---|
| Startup to the first frame with content (opening `big.rs`) | Screen recording at ≥ 120 fps (ScreenCaptureKit), time 0 is when the command is invoked; the first frame showing the file's text |
| Idle RAM (medium repo, 10 tabs) | Sum of phys_footprint over every process of the app (`footprint`), after 60 s idle |
| Scrolling `big.rs` | The same synthetic scroll events (CGEvent); the `xctrace` Animation Hitches template counts dropped frames |
| File changed on disk → screen updates | A script writes the file; screen recording measures the latency |
| Quick Open per keystroke (80,000-file repo) | A key-typing script (CGEvent); screen recording measures key → result |
| Idle CPU | `top`/`ps` over 60 s |
| App size | `du` of the bundle |

Each result is recorded together with: the three editors' versions, the
Lucerna commit, macOS, chip, RAM, and measurement date.

## Result: three-editor comparison

The main output is a comparison table, one row per metric, the three editors
side by side:

| Metric | Lucerna | Zed | VS Code |
|---|---|---|---|
| Startup to first frame with content (median / p90) | … ms | … ms | … ms |
| Idle RAM | … MB | … MB | … MB |
| … | | | |

- Each cell gives the median and p90. Each row adds a ratio relative to
  Lucerna (for example "Zed 1.4×"), computed from the median; no
  "win/lose" coloring.
- One table per configuration (no LSP / with rust-analyzer; cold / warm).
- A bar chart (SVG) per metric, three bars for the three editors, used on
  the site.
- The toolkit generates the Markdown tables and SVGs from the result JSON
  files, so the numbers on the README/site always match the raw data.

## Publication

- `docs/benchmarks.md`: methodology, configuration, known limitations
  (Lucerna has fewer features; each editor has a different architecture),
  the comparison tables, measurement date.
- Raw results (JSON) committed in `bench/compare/results/`.
- The README and site quote a summary table and link to the methodology
  page.

## References

- VS Code, [Performance Issues](https://github.com/microsoft/vscode/wiki/Performance-Issues):
  `code --prof-startup` writes two CPU profile files for a startup; the
  **Developer: Startup Performance** command (F1 > Startup Performance)
  opens a table of startup times by phase. Both rely on VS Code's own
  `perf marks` (the wiki page "Inventory of performance marks").
- Zed, [Performance](https://zed.dev/docs/performance): uses `samply` for
  CPU flamecharts, Tracy (`ZTRACING=1 cargo r --features tracy --release`)
  for detailed traces, `zed open performance profiler` for async task
  traces, and the slow-function warning
  `zlog::time!(...).warn_if_gt(...)`. These are tools for Zed developers,
  built from source; they are not a common way to measure. We read the
  documentation only and copy no code (most of Zed's crates are GPL).
- Why measure from outside: each editor marks a different moment itself (VS
  Code: marks in the renderer/extension host processes; Zed: in-process
  function timings; Lucerna: the first frame in `perf-checklist.md`). These
  numbers cannot be compared with each other and do not include the time the
  operating system takes to display the frame. Recording the window from
  outside with ScreenCaptureKit gives all three the same "the user sees the
  content" mark.

## Approaches considered

1. **Measure from outside with screen recording and system tools**
   (chosen): fair, applicable to all three.
2. Use each editor's own internal measurements: each editor measures a
   different moment, so they cannot be compared. Rejected.
3. Measure only RAM and startup time with `time`: easy, but it cannot
   measure the first frame with content. Rejected.

## What the user must do

Grant Screen Recording and Accessibility permission to the terminal that
runs the scripts; run the `sudo` commands (`purge`).

## Left to the implementer

The language of the toolkit (proposal: standard Python plus a small Swift
helper for ScreenCaptureKit/CGEvent if needed), the specific medium repo,
how to recognize a "frame with text".
