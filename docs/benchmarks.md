# Performance comparison: Lucerna, Zed and VS Code

This page describes how the measurements are made, so that others can rerun them
and understand what the numbers do and do not mean. Design:
[`docs/superpowers/specs/2026-11-14-editor-benchmark-design.md`](superpowers/specs/2026-11-14-editor-benchmark-design.md).
Measurement code: [`bench/compare/`](../bench/compare/) (see also `bench/compare/README.md`).

## Purpose

Measure three editors on the same machine, in a clean environment, with the same
outside-the-app method, then publish the comparison table together with the
script and the raw data. The goal is reproducible numbers, not a ranking.
Lucerna is a minimal editor with no AI and fewer features than the other two
(see [Known limits](#known-limits)).

## Why measure from outside

Each editor has its own instrumentation (VS Code: `--prof-startup` and
Developer: Startup Performance; Zed: its developer profiling tools; Lucerna: the
measurements in `docs/perf-checklist.md`). Each one marks a different moment,
so the numbers cannot be compared with each other. This toolkit uses a single
shared mark for all three: the moment the window **shows content on screen**,
captured with ScreenCaptureKit. Lucerna's internal numbers are not to be
compared with another editor's outside measurement.

## Environment

Everything lives under the bench directory (default `~/LucernaBench.noindex`,
changed with `--bench-dir`); the user's real data is never touched. The
`.noindex` suffix makes Spotlight skip it.

**A separate profile per editor** (`<bench>/profiles/`). At the start of each
configuration (no LSP / with rust-analyzer), bench deletes every profile under
`<bench>/profiles` and writes the settings again, so no configuration carries
over the cache, history or extensions of the previous one. Only `vscode-ext-lsp`
(VS Code's rust-analyzer extension, which needs a network connection to install)
and Lucerna's pid file are kept. The deletion touches only known profile
entries inside `<bench>/profiles`; unknown entries are kept with a warning, and
bench refuses to delete anything if `<bench>/profiles` is a symlink.

All three editors are launched through their **main binary** (the binary that
owns the window), not through a launcher:

| Editor | How it is launched | Main settings |
|---|---|---|
| VS Code | `/Applications/Visual Studio Code.app/Contents/MacOS/Code --user-data-dir <bench>/profiles/vscode --extensions-dir <bench>/profiles/vscode-ext --disable-extensions --new-window <target>`; the LSP configuration uses `--extensions-dir <bench>/profiles/vscode-ext-lsp` and has no `--disable-extensions` | telemetry, updates, Workspace Trust, welcome page, reopening previous windows and minimap off; `editor.fontSize` 14; preview tabs off when opening from Quick Open |
| Zed | `/Applications/Zed.app/Contents/MacOS/zed --user-data-dir <bench>/profiles/zed <target>` | telemetry, updates, AI (`disable_ai`), session restore, minimap, auto-install of the `html` extension (`auto_install_extensions`) and extension suggestions (`suggest_extensions`) off; `buffer_font_size` 14; preview tabs off from the file finder |
| Lucerna | the `luc` binary (resolved once per session) with `HOME=<bench>/profiles/lucerna-home` | `editor.fontSize` 14, `workbench.startupEditor: "none"`, data folder marked trusted in advance (no Restricted Mode), preview tabs off |

VS Code is not launched through `bin/code`: that command starts a node/CLI step
before calling the app, a cost the other two editors do not have in their
startup time. Zed is not launched through `cli`: `cli` goes through
LaunchServices and drops `--user-data-dir`. Zed writes more of its own data to
`profiles/zed/config` and `profiles/zed/data` while running; the folder is
deleted at the start of each configuration.

Each editor uses its default theme. A configuration with a language server
records the path and version of `rust-analyzer` in the result; the no-LSP
configuration turns the LSP off in all three (Lucerna `lucerna.lsp.enabled:
false`, Zed with the Rust language server removed, VS Code with the empty
extension folder `vscode-ext` plus `--disable-extensions`). In the LSP
configuration VS Code uses `vscode-ext-lsp`; if the
`rust-lang.rust-analyzer` extension is missing there, bench stops right at the
start.

**Window size.** Every measurement that opens an editor, except startup, sets
the window to 1280×800 pt (requested at x=0, y=25) through Accessibility
as soon as the window appears, so the three editors are measured at the same
size. macOS keeps the window below the menu bar, so on a display with a notch
the window may sit lower; the size is checked again, and if it is wrong the
round is discarded. The real frame (after macOS adjusts the position) is
recorded in the result, once per editor per configuration
(`meta.window_frames`). The startup measurement uses **each editor's default
window size** (the first frame cannot be resized). The size, scale and refresh
rate of the main display are recorded in the result.

**Fixed data** in `<bench>/data/`:
- The large repo, generated by `scripts/gen-bench-repo.sh`: 80,000 files and a
  `big.rs` of about 10 MB.
- The medium repo: ripgrep `14.1.1` (shallow clone), used for idle RAM and CPU.
- `marker.txt`: 200 lines of 80 `█` characters each (solid blocks, so the middle
  of the window changes colour clearly).

**Machine check** (`env.preflight`), run **before every round**, not only at the
start of the session: no `cargo`/`rustc` running, 1-minute CPU load ≤ 2.0,
`CPU_Speed_Limit` = 100 (`pmset -g therm`), on AC power, Low Power Mode off. If
it fails, bench waits 30 s and tries again (up to 10 times), then stops and
saves the results so far. The person running it is responsible for closing
other apps.

**Permissions needed:** Screen Recording and Accessibility for the terminal
running bench (permissions per helper command and per measurement:
`bench/compare/README.md`). If a permission is missing, the helper reports a
clear error and exits with a non-zero code; it never returns a 0 value.

## Measurements

Each measurement opens the editor in the bench profile, measures, then closes
the editor. Only processes carrying the bench profile path (and their children),
or the `luc` recorded in bench's pid file, are closed; a Zed or VS Code that
the user already has running is left alone. Lucerna is identified through the
pid file (pid, start time and exact binary path) because `ps` on macOS does not
show the environment variables of other processes.

The Lucerna binary is resolved **once** at the start of the session and recorded
in the result (`meta.lucerna_binary`: path, file modification time,
`luc --version`), next to `lucerna_commit` (the checkout's HEAD, which may
differ from the binary if it has not been rebuilt). VS Code runs with
`ELECTRON_RUN_AS_NODE` and `ELECTRON_NO_ATTACH_CONSOLE` removed from its
environment (if the shell that invokes bench sets them), so that `Code` always
runs as an app. If there is no Lucerna binary, bench stops immediately; if the
binary disappears mid-session (being rebuilt), bench stops before the next
Lucerna launch.

Clock: every timestamp is in nanoseconds of `CLOCK_UPTIME_RAW`. The time of a
frame comes from `SCStreamFrameInfo.displayTime` (mach time), which
ScreenCaptureKit attaches to the frame, so it does not depend on the delay in
delivering the frame to the measuring process. The `bench-helper` helper (Swift,
built with the Command Line Tools' `swiftc`; no Xcode or `xctrace`) captures the
editor's largest visible window at the display's refresh rate.

**Background colour.** For each editor and version, bench opens an empty file
and takes the most common colour in the middle of the window as the background
colour; the calibration is discarded if the most common colour covers less than
90% of the middle region (it may be a dialog or a window that has not finished
drawing). The colour is stored in `<bench>/calibration.json` and recorded in the
result.

| Metric | How it is measured |
|---|---|
| Startup to first frame with content | Open `marker.txt`. The helper captures **the whole main display** and starts capturing **before** the editor is opened (bench waits for the helper to report ready, then runs the open command), so the first frame of the new window is not missed. On each frame the helper finds the editor binary's largest window (by the binary path of the owning process; processes of the same binary that were already running, such as your own VS Code, are excluded), crops the frame to that window, and stops at the first frame in which ≥ 30% of the pixels in the middle region (60% × 60% of the window) differ from the calibrated background colour. Time 0 is just before the open command; the helper reports the absolute time of the frame and bench subtracts time 0. Because the whole display is captured, another window lying over the editor would be measured instead: a frame counts only if the topmost window at the centre and the four corners of the middle region belongs to the editor; an occluded frame cannot meet the condition, so an occluded window only makes the measurement later, never falsely earlier (the number of occluded frames is recorded). So that the new window is not behind another window, as soon as the editor's process appears bench brings that app to the front from a background thread (which does not slow down the capture). If the window already has enough content in the very first captured frame (impossible for a window opened afterwards), the round is discarded (recorded as `null`). |
| File changed on disk → screen updated | Open a copy of `marker.txt`, wait for the content to show plus 1 s to settle, start capturing, then the script writes an empty file. Time 0 is just before the write command; stops at the first frame that changes by more than 0.5% relative to the reference frame (a threshold that ignores a blinking cursor). |
| Quick Open per keystroke (80,000-file repo) | Open the large repo, wait 30 s for the file scan, ⌘P, type `mod004`, then start capturing and type the last key `2` (`mod0042` is a folder that really exists in the generated repo). Time 0 is the moment the last key is sent; stops at the first frame that changes. Keys are sent with `CGEvent.postToPid` to the exact process, after bringing the app to the front. |
| Idle RAM | Open the medium repo, open 10 `.rs` files of ripgrep through Quick Open (the same way for all three, because `luc` accepts only one path on the command line), idle for 60 s, then sum the `phys_footprint` of every process of the app (`footprint`). Unit: MiB (1024). |
| Idle CPU | Same session as RAM (one session per editor per round: idle 60 s, read RAM, then measure CPU for another 60 s, about 2 minutes in total): cumulative CPU time (`ps -o time=`) of every process of the app, read every second with the process list refreshed each time; CPU used over the 60 s divided by wall-clock time, ×100 (100% = one core). A process that appears later counts from 0, and a process that exits midway counts up to its last reading. The `%cpu` column of `ps` is not used (a decaying average, not instantaneous). The report shows two decimal places; when Lucerna's median is below 0.1% no ratio is shown (a ratio of two values that small is mostly noise). |
| Dropped frames scrolling `big.rs` | Open `big.rs`, wait for the content to show, bring the editor to the front and check that the topmost window at the window centre is the editor's (if another window covers it, the round is discarded without sending any event), then send 3000 one-line scroll events at 120 events/second (CGEvent), with the pointer at the window centre. A dropped frame is a gap between two new frames > 1.5 × max(display refresh period, event period) while events are still being sent. The reported value is the dropped rate / number of frames (%). The middle of a code file covers fewer pixels, so the "has content" threshold for this measurement is 2%, not 30%. |
| App size | `du -sk` of the `.app` bundle, MiB. One value per configuration, not repeated per round. |

## Rounds and statistics

- **10 rounds** per measurement (`--rounds`, default 10 per the spec). The order
  of the three editors rotates every round so that no editor always runs first
  or last. With 10 rounds, Lucerna runs first 4 times and Zed and VS Code 3
  times each; so choose a number of rounds that is a multiple of 3 (for example
  9 or 12) for each editor to run first an equal number of times.
- **Round 0 is dropped:** at the start of each configuration, bench runs one full
  round of every selected measurement (startup included) for each editor and
  discards the result, so that the first measured round is not the first time
  anything has been opened (operating system and app caches). Each measurement
  that opens an editor therefore runs `rounds + 1` times per editor.
- **A 20 s pause** between two editor launches lets the machine cool. Preflight
  runs before every round.
- Reported are the **median** and **p90** (nearest rank: the ⌈0.9·n⌉-th value
  after sorting).
- A failed round (helper error, 60 s timeout, no window found, suspicious frame)
  is recorded as `null`; the statistics skip it and the report writes "n/N
  rounds failed". One failed round does not fail the whole session; the result
  is saved after every round. A file-change or Quick Open round is discarded
  (`null`) if the frame changed before the triggering event.
- **Ratio** vs Lucerna = the other editor's median / Lucerna's median, computed
  per editor column and rounded to one decimal place. A ratio > 1 only means
  that the other editor's number is larger than Lucerna's; a larger number is
  not good or bad in itself, it depends on the metric. No winner/loser
  colouring.
- Each result comes with: the versions of the three editors, the Lucerna commit,
  the Lucerna binary (path, build time, `--version`), the path and build time of
  the helper, macOS, chip, RAM, the main display (size, scale, refresh rate),
  the window frame that was set, the measurement date, `rust-analyzer` (if any),
  and the calibrated background colours.
- Two modes: **warm** (default) and **cold** (`--cold`: before every startup,
  round 0 included, bench pauses and waits for you to run `sudo purge`). Only
  the startup measurement is cold; the other measurements in the same
  configuration run as in warm mode. The report names this configuration "cold
  (startup only)".

## Known limits

- **Lucerna has fewer features.** No AI, no debugger, no VS Code-style extension
  host. The same measurement run on three programs covers different scope; bench
  does not measure features Lucerna does not have.
- **Different architectures**: VS Code runs on Electron (multiple processes),
  while Zed and Lucerna are native apps drawn on the GPU. The RAM figure sums
  every process of the app; this adds up all processes, but does not say which
  part belongs to which process, nor does it reflect memory shared between
  processes.
- **Outside measurement includes system latency**: the compositor, the display
  and screen capture add to the measurement equally for all three editors, so
  the numbers are not the time inside each app and cannot be compared with their
  internal numbers. ScreenCaptureKit's capture latency makes startup and response
  latency slightly larger than the real value; time resolution is limited by the
  display's refresh rate.
- **Quick Open and file opening differ between editors**: each editor indexes
  and ranks results in its own way and may scan in the background after a folder
  is opened. Bench waits 30 s before typing but does not verify that the scan
  has finished. Files are opened through Quick Open in all three so they take
  the same path; preview tabs are off in all three so that ten files become ten
  tabs. Lucerna has no separate setting for Quick Open, so it uses
  `workbench.editor.enablePreview: false`.
- **Settings choices affect the numbers**: minimap off, Zed's AI features off,
  telemetry off, font size 14 and an empty profile (no extensions) make the
  numbers differ from a machine used every day. The rust-analyzer configuration
  also measures the language server; all three editors share one
  `rust-analyzer` binary (VS Code through the `rust-lang.rust-analyzer`
  extension, installed once and pointed at the same binary with
  `rust-analyzer.server.path`).
- **The ten files of the idle RAM/CPU measurement are not verified to be
  open.** Bench opens them through Quick Open (type the path, Enter) and waits,
  but does not check that each editor really has ten tabs; a failed open makes
  the RAM number lower without the round being discarded.
- **Main display only.** The startup measurement captures the main display (the
  one with the menu bar). If an editor opens its window on another display, the
  round times out and is recorded as `null`. Run with one display, or let the
  editors open on the main display.
- **Startup at each editor's default window size**, the other measurements at
  1280×800 pt. Default sizes differ between editors, so the startup number
  includes the difference in the area that has to be drawn.
- **The startup measurement depends on `marker.txt`** (middle region covered
  ≥ 30%) and on the calibrated background colour; a different theme can give
  different numbers. A round where the content is already visible in the very
  first frame is discarded instead of recording a possibly wrong number.
- **One machine, one run.** The results speak only for the machine and versions
  recorded with them.
- The Zed setup (`profiles/zed/config/settings.json`) is based on reading Zed's
  help and binary; if another Zed version uses different paths,
  `env.write_profiles` needs adjusting.

## Rerunning

From the repo root. Requirements: macOS, the Command Line Tools (`swiftc`),
`python3`, VS Code and Zed in `/Applications`, a `luc` binary (a release build
from `scripts/bundle-app.sh`, or set `LUCERNA_BIN`), and a network connection
for the first run (cloning ripgrep).

1. Plug in the charger, turn Low Power Mode off, close other apps, stop any
   `cargo`/`rustc`. **Quit your own Zed first** and do not open Zed while the
   run is in progress: Zed allows only one instance per release channel, so when
   your Zed is running (anywhere) bench stops immediately (or midway, before the
   next Zed launch) and saves the partial results. Your VS Code can stay open:
   bench runs its own session and ignores your window. Do not rebuild Lucerna
   during the run.
2. Grant **Screen Recording** and **Accessibility** to the terminal app in
   System Settings > Privacy & Security.
3. Build the helper:
   ```sh
   sh bench/compare/helper/build.sh        # writes <bench>/bin/bench-helper
   ```
   If you use `--bench-dir D`, build with `BENCH_DIR=D sh bench/compare/helper/build.sh`
   (or pass the output path as an argument), or set `BENCH_HELPER` to point at
   the helper.
4. (Only for the LSP configuration) `rust-analyzer` must be on PATH; install the
   VS Code extension once into `<bench>/profiles/vscode-ext-lsp` with
   `env.install_vscode_rust_analyzer` (see `bench/compare/README.md`). If the
   extension is missing, `--lsp on`/`both` stops right at the start.
5. Run. Each configuration takes quite a while (each round includes several
   launches and, for each editor, an idle session of about 2 minutes: 60 s idle,
   then 60 s of CPU sampling):
   ```sh
   python3 -I bench/compare/run.py --rounds 10 --lsp off     # warm, no LSP
   python3 -I bench/compare/run.py --rounds 10 --lsp on      # warm, rust-analyzer
   python3 -I bench/compare/run.py --rounds 10 --lsp off --cold
   ```
   With `--cold`, the script pauses and prompts "run `sudo purge`, then press
   Enter" before **every** measured startup (round 0 included); run `sudo purge`
   in another terminal, then press Enter.
   If Screen Recording is missing, bench stops before measuring and suggests
   `--only` with the measurements that can run (for example
   `--only idle_ram_mb,idle_cpu_pct,app_size_mb`).
   Options: `--lsp both`, `--only startup_ms,scroll_dropped_pct`,
   `--bench-dir D`, `--out FILE`, `--skip-prepare`. By default the result is
   written to `bench/compare/results/<date>-<commit>.json`.
6. Generate the tables and charts:
   ```sh
   python3 -I bench/compare/report.py bench/compare/results/<file>.json --out <dir>
   ```
   which writes `results.md` and one SVG per metric and configuration.

The toolkit's tests: `python3 -m unittest discover -s bench/compare/tests -v`.

## Results

Pending: waiting for the official measurement run.
