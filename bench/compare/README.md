# bench/compare: compare Lucerna, Zed and VS Code from outside the app

Spec: `docs/superpowers/specs/2026-11-14-editor-benchmark-design.md`. Python stdlib only (`python3 -I`).
Tests: `python3 -m unittest discover -s bench/compare/tests -v` (run from the repo root).

## Permissions needed (once)

System Settings > Privacy & Security, grant to the terminal app that runs bench:
- **Screen Recording** (the helper captures the screen with ScreenCaptureKit).
- **Accessibility** (the helper creates key/scroll events with CGEvent and sets the window size with AX).

Each helper command asks only for the permissions it uses:

| Command | Permission |
|---|---|
| `now`, `check`, `display` | none |
| `sample`, `watch` (including `watch --display`), `record` | Screen Recording |
| `key`, `type`, `frame`, `activate` | Accessibility |
| `scroll` | Screen Recording and Accessibility |

From that, each measurement needs: `idle_ram_mb`, `idle_cpu_pct` Accessibility; `startup_ms`,
`file_change_ms`, `scroll_dropped_pct` both; `app_size_mb`
none (the `METRIC_HELPERS` × `HELPER_PERMISSIONS` tables in `metrics.py`, from which `run.py` derives
this). If a permission is missing, the helper reports an error and exits with code 3, and `run.py` stops before measuring, prints the name of the
missing permission and suggests an `--only` listing the measurements that can run with the permissions you have (for example without
Screen Recording: `--only idle_ram_mb,idle_cpu_pct,app_size_mb`).

The bench window must be on the main display: `watch --display` captures only the main display, and
a window on another display gives a `null` round (timeout).

## The `bench-helper` helper (Swift)

Build it with the Command Line Tools' `swiftc`: `sh bench/compare/helper/build.sh [out]` (default `<bench>/bin/bench-helper`). Each command prints one JSON line to stdout; errors go to stderr, exit code 1 = syntax error/failure, 2 = timeout, 3 = missing permission (no measurement is printed). Arguments are checked before permissions, so a syntax error is always code 1. Permissions per command: see the table above.

`--pid` takes a comma-separated list (pass every pid of the bench session, child processes included); the helper picks the largest visible layer-0 window (≥ 100×100 pt) belonging to one of those pids. The capture size is fixed when the stream starts; if the window is resized afterwards, the content is scaled into that size. `frame` and `scroll` also take `--window-index <n>`: the n-th (0 = oldest) visible layer-0 window of the pids, ordered by `CGWindowID` (macOS hands IDs out in creation order). It picks one window when one process shows several. `frame` then moves the AX window whose frame equals that window's bounds; `scroll` raises that AX window (`kAXRaiseAction`) and requires that very window, not only its pid, to be on top at its centre.

- `now`: `{"t_ns"}` from `CLOCK_UPTIME_RAW`, the same clock as `editors.now_ns()` (`time.clock_gettime_ns(time.CLOCK_UPTIME_RAW)`; `time.monotonic_ns()` is not used because macOS's python3 3.9 counts it from process start).
- `check`: `{"screen_recording", "accessibility"}`, preflight only, never shows a permission prompt. `BENCH_HELPER_FAKE_DENY=1` fakes both permissions missing; `=screen` or `=accessibility` fakes one missing.
- `sample --pid [--timeout]`: calibration; grabs one frame and prints `{"center_dominant": "RRGGBB", "center_dominant_frac", "window_dominant", "width", "height"}` (the middle region is the region `coverage` measures). Python uses it to pass a per-editor `--background`.
- `watch --pid --since --until coverage:x|change [--timeout 60] [--baseline png] [--background RRGGBB]`: captures at the display's refresh rate; the frame time comes from `SCStreamFrameInfo.displayTime` (mach time → ns). Prints `{"t_ns", "frames", "first_frame_hit"}`; `first_frame_hit: true` means the first frame already met the condition, so the measurement is suspect. `coverage`: the fraction of pixels in the middle 60%×60% region that differ from the background colour; without `--background` it takes the most common colour of the first frame and warns on stderr. `change`: compared with `--baseline`, or the last frame before `--since`, or the first frame; a 0.5% threshold ignores a blinking cursor (`--min-change <fraction>` overrides it; default 0.005). The helper writes `ready` to stderr once the first frame has been saved as the reference; wait for this line before causing the change. Change diagnostics: on success the JSON also has `"change"` (the hit frame's fraction) and `"max_change"` (the highest fraction seen after `--since`); on timeout the stderr message ends with `max_change=<f> small_changes=<n>`, where `small_changes` counts frames whose fraction was above 0 but not above the threshold. `--dump-dir <dir>` (change only) writes `ref.png` (the reference), `max.png` (the frame with the highest change) and `changes.txt` (one `t_ns fraction` line per frame compared after `--since`), on success and on timeout.
- `watch --display --exe <absolute path> [--exclude-pids a,b] --since 0 --until coverage:x --background RRGGBB [--timeout s]`: used for startup. Captures **the whole main display** (`SCContentFilter(display:excludingWindows: [])`), starting **before** the editor is opened. On every complete frame the helper uses `CGWindowListCopyWindowInfo` to find the largest visible layer-0 window (≥ 100×100 pt) whose owning process has a `proc_pidpath` equal to `--exe` or inside the `Contents/` of the same `.app` bundle (child processes), and is not in `--exclude-pids`; it crops the frame to the window region (points → pixels by the frame/display ratio) and measures `coverage` on the middle 60%×60% region of the crop. It writes `ready` to stderr after the first complete frame. Python does not yet know t0 when the helper starts, so it passes `--since 0` (the helper reports only an absolute `t_ns`) and subtracts the t0 of the launch itself. `first_frame_hit: true` in this mode means the window already existed and had enough content in the first frame, which cannot happen for a window opened after `ready`: it is still treated as suspect and the round is discarded. The pixels are the composited display, so another window covering the editor would be measured instead: the helper tries `coverage` only on a frame where the topmost window (same window list, front-to-back order) at the centre and the four corners of the middle region belongs to a matching pid; other frames count as occluded (`occluded_frames`) and cannot hit. An occluded window therefore makes the measurement later, not falsely earlier. Accepts only `coverage`, requires `--background`, does not accept `--pid`/`--baseline`. Also prints `"occluded_frames"` and `"window": [x, y, w, h]` of the window at the hit. `--hold-ms <n>` (0 to 60000, default 0, display mode only): after the hit, keeps capturing for n ms and compares the window crop of every new frame with the hit frame's crop (same per-channel tolerance as `change`); prints `"post_change"`, the largest fraction that differs (a window whose size changed counts as 1.0), and `"post_frames"`, the number of frames compared. A hold cut short by `--timeout` still reports the hit. `--allow-resize` (needs `--hold-ms` > 0): a size change after the hit is not counted as a change; the reference is re-taken at the new size and the hold restarts. It is for a window that macOS animates to its zoomed size (a new window of an app that is already running). Prints `"settled_ns"` (the first frame at the final size, `t_ns` when the size never changed) and `"resizes"`.
- `activate --pid [--timeout 10]`: picks the window like `key` and brings the owning app to the front (`NSRunningApplication.activate`); prints `{"pid", "active"}`.
- `frame --pid --x --y --w --h [--timeout 10]`: picks the window like `key`, takes the owning app's largest AX window (`AXUIElementCreateApplication` → `kAXWindowsAttribute`), sets `kAXPositionAttribute`/`kAXSizeAttribute`, reads them back; prints `{"x", "y", "w", "h", "pid"}`. Only the **size** is enforced (error ≤ 1 pt): macOS keeps the window below the menu bar, which is taller on a display with a notch, so the real `y` can be larger than `--y`. If it is not reached before the deadline → code 2.
- `display`: `{"width", "height"}` (pt), `pixel_width`, `pixel_height`, `scale`, `refresh_hz` (`NSScreen.maximumFramesPerSecond`), `mode_refresh_hz` of the main display; needs no permission.
- `scroll --pid --lines [--rate]`: brings the owning app's window to the front, then checks with `CGWindowListCopyWindowInfo` (front-to-back order) that the topmost window at the window centre belongs to the given pids (ignoring fully transparent alpha-0 windows and system layers from the menu bar level up); if not, code 1 and no event is sent. Then it moves the mouse pointer to the window centre and sends one-line scroll events, by default at a rate equal to the display's refresh rate; prints `{"frames", "dropped", "refresh_hz", "rate", "events_sent", "fps", "interval_ms_p50", "interval_ms_p90", "interval_ms_max"}`. A "dropped" frame is a gap between two new frames > 1.5 × max(display period, event period) (a deviation from the definition in the brief: when `rate` < the display refresh rate, the original definition counts every frame as dropped). The `interval_ms_*` statistics cover the same gaps that `dropped` tests (consecutive new frames, the first of the pair inside the send window; `null` with fewer than two frames); `fps` = new frames inside the send window / its length. `--unit line` (default) sends one-line wheel events (`kCGScrollEventUnitLine`), like a notched mouse wheel. `--unit pixel` sends continuous, trackpad-like pixel events of `--pixels-per-event` (default 40; `--lines` then counts events): `kCGScrollWheelEventIsContinuous` = 1 and `kCGScrollWheelEventScrollPhase` = began on the first event, changed on the rest, then one extra zero-delta ended event (not counted in `events_sent`); there is no momentum phase. `--dump-dir <dir>` writes `intervals.txt` (one gap in ms per line).
- `record --pid --seconds [--dump-dir] [--timeout 10]`: a baseline for scrolling by hand. Captures the chosen window for `--seconds` without sending any input; writes `ready` to stderr when the first frame arrives (start scrolling then) and `done` when the capture ends; prints `{"frames", "refresh_hz", "seconds", "fps", "interval_ms_p50", "interval_ms_p90", "interval_ms_max"}` over the gaps between all consecutive new frames. A pause shows up as a long gap, so keep scrolling for the whole recording. `--dump-dir` writes `intervals.txt`. `--timeout` bounds only the window lookup.
- `type --pid --text [--interval-ms 100] [--timeout 10]`, `key --pid --combo cmd+p [--timeout 10]`: find the window with `CGWindowListCopyWindowInfo` (owner pid, layer and frame need no Screen Recording; the same "largest layer-0 window" choice), bring the owning app to the front (`NSRunningApplication.activate`), then send keys with `CGEvent.postToPid` to that pid; print the time of each key. `type` accepts only ASCII letters, digits, space, basic punctuation and `\n` (Return); any other character → code 1.

## Measurements (`metrics.py`)

Each function opens the editor in the bench profile, measures, then always closes the editor; it returns `None` when the round failed (helper error/60 s timeout, no window found, suspicious frame). A missing permission (helper code 3) raises `PermissionDenied` to stop the whole session.

- Background calibration: `calibrate` opens `data/empty.txt` and `sample` takes the colour at the window centre; stored in `<bench>/calibration.json` under `"<editor> <version> <settings hash>"` (`load_calibration`), where the hash is the first 12 hex digits of the SHA-256 of the settings.json the profile writes for that editor (`env.settings_file`); any profile change therefore recalibrates.
- Window size: every measurement that opens an editor, except startup, sets the window to `WINDOW_FRAME` (1280×800 pt, requested at x=0, y=25) with `frame` as soon as the window appears; if `frame` fails, that round is `None`. Startup uses each editor's default size (the first frame cannot be resized).
- `startup_ms`: computes the list of pids already running the same binary (for example your own VS Code) as `--exclude-pids`, runs `watch --display --exe <main binary> --since 0 --until coverage:0.3 --background <background>`, waits for `ready`, **then** opens `marker.txt`; result = `t_ns` − t0. Right after the open command, a background thread waits for the window-owning process to appear and then calls `activate` to bring the new window to the front (without slowing down `watch`); a failed `activate` is only logged, a missing permission stops the session. `first_frame_hit` or `t_ns` < t0 → `None`. The watch runs with `--hold-ms 1500`: if `post_change` > `POST_CHANGE_MAX` (0.05), the window changed after the first content frame (a dialog, an overlay, or a hit before the real content), the round is logged and `None`; a missing `post_change` (an older helper) is also `None`.
- `file_change_ms`: opens `data/reload.txt` (a per-round copy of `marker.txt`), sets the window, waits for it to appear, `watch --until change`, writes an empty file; time 0 is just before the write.
- `idle_session` → (`idle_ram_mb`, `idle_cpu_pct`): the medium repo, sets the window, opens 10 `.rs` files of ripgrep 14.1.1 through Quick Open (the same way for all three editors; `luc` accepts only one path), idles for 60 s; RAM = the sum of `phys_footprint` (`footprint -f bytes --noCategories -p …`). CPU = cumulative CPU time (`ps -o pid=,time=`) of every bench process over 60 s: sampled every second, with the pid list refreshed each time; a process present in the first sample counts from its value then, a process that appears later counts from 0, a process that exits midway counts up to its last sample; % = CPU used / wall-clock time × 100 (100% = one core). The `%cpu` of `ps` is not used (a decaying average). `report.py` writes CPU with two decimal places and writes no ratio when Lucerna's median is below `CPU_RATIO_FLOOR` (0.1 %), with a note under the table. The profile turns preview tabs off when opening from Quick Open so that 10 files are 10 tabs; bench does not verify that 10 tabs are open (a known limit).
- `scroll_dropped_pct`: `big.rs`, sets the window, `scroll --unit pixel --lines 600 --pixels-per-event 40` (one continuous event per refresh), `dropped / frames × 100`.
- `app_size_mb`: `du -sk` of the bundle, MiB (1024).

Quick Open latency is not measured: no external method gave a comparable result across the three editors (docs/benchmarks.md, "Measurements").

`diagnose.py` (not a benchmark) records what the scroll measurement sees: frame intervals for line and pixel scroll events, plus an optional hand-scrolling comparison (`record`). See docs/benchmarks.md, "Scrolling: why pixel events".

## Multi-window measurement (`multiwindow.py`)

Spec: `docs/superpowers/specs/2026-11-15-multi-window-design.md` section 7.1. Lucerna only.

```sh
python3 -I bench/compare/multiwindow.py --windows 1,2,3 --mode before \
    --out bench/compare/results/multiwindow-before.json --single-window
```

`--mode before` is today's behaviour: every window is its own `luc <repo>` process. `--mode after` is the one-process build: the first window is `luc --foreground <repo1>`, the others are `luc <repoK>` clients (the `--foreground` flag does not exist before that work lands). Options: `--rounds` (default 5), `--only ram_mb,second_window_ms,drop`, `--single-window` (also measures the existing `startup_ms` and `idle_ram_mb` of one window with `metrics.py`), `--ignore-preflight`, `--bench-dir`.

| Metric | What | Permission |
|---|---|---|
| `ram_mb[n]` | summed `footprint` of every `luc` process this script spawned (and their children), after n windows on n different repos settled | none |
| `second_window_ms` | the second `luc` command to the first frame of the new window with content (`watch --display`, as `startup_ms`). Window A is empty and 600x400, so the new, larger window is the one measured in both modes. In the `after` mode macOS animates the new window's zoom (about 0.37 s), so the watch takes `--allow-resize`; `second_window_settled_ms` is the first frame at the final size (equal to `second_window_ms` when the hit came after the animation) | Screen Recording, Accessibility |
| `drop_pct_idle_b`, `drop_pct_while_busy` | dropped frames in % while scrolling A (`big.rs`, 1280x800) with B idle on a small repo, and while B has just been asked to open the large repo. `drop_frames_idle_b` / `drop_frames_while_busy` are the frames captured during each scroll, so a 0 % comes with the count it was taken over | Screen Recording, Accessibility |

The script starts its own processes and signals only those and their children (the pidfile of `editors.py` holds one root, so it is not used). Repos are copies of the ripgrep sources in `<bench>/data/multi/{a,b,c,d}`, and `<bench>/data/multi/scroll/big.rs` is the scroll file (so A does not index the 80,000-file repo).

With two windows of one pid (the `after` mode) the script parks B with `frame --window-index 1` and scrolls A with `scroll --window-index 0`. If B still covers A's centre, `scroll` refuses and that round is `null`. `--single-window` waits for a quiet machine (preflight) before each of its rounds, as the multi-window rounds do.

## Building the environment

By default under `~/LucernaBench.noindex` (change with `--bench-dir`):
1. `env.prepare_data(paths)`: generates the large repo with `scripts/gen-bench-repo.sh` (80,000 files + `large/big.rs` ~10 MB; only `big.rs` is measured, by `scroll_dropped_pct`), shallow-clones ripgrep `14.1.1` into `data/medium` (needs a network connection), writes `data/marker.txt`.
2. At the start of each configuration: `env.reset_profiles(paths)` deletes the known profiles under `<bench>/profiles` (`env.PROFILE_ENTRIES`: `vscode`, `vscode-ext`, `zed`, `lucerna-home`, `bench-profile.json`), except `vscode-ext-lsp` and the Lucerna pid file in `lucerna-home`. Unknown entries are kept with a warning; if `<bench>/profiles` is a symlink it refuses; it deletes only paths inside that folder; then `env.write_profiles(paths, lsp=False|True)` writes clean settings for all three editors and `profiles/bench-profile.json` (`{"lsp": …}`, which `editors.launch` reads to choose VS Code's extensions folder).
3. VS Code extensions: `profiles/vscode-ext` is always empty, used for the no-LSP configuration together with `--disable-extensions`; `profiles/vscode-ext-lsp` holds rust-analyzer, used for `lsp=True`. `lsp=True` needs `rust-analyzer` on PATH and the extension installed once (needs a network connection):
   ```sh
   python3 -I -c 'import sys; sys.path.insert(0, "bench/compare"); import env; env.install_vscode_rust_analyzer(env.BenchPaths(env.DEFAULT_ROOT))'
   ```
   If `rust-lang.rust-analyzer*` is missing from `vscode-ext-lsp`, `run.py --lsp on|both` stops right at the start.
4. Before each measured round, `env.preflight()` must return an empty list (no `cargo`/`rustc`, 1-minute load <= 2.0, `CPU_Speed_Limit` = 100, on AC power, Low Power Mode off).
5. With `--cold`, before every measured startup (round 0 included) the script pauses; the user runs `sudo purge` themselves and presses Enter (the script does not run sudo). Only the startup measurement is cold; the report names the configuration "cold (startup only)".
6. The Lucerna binary is resolved **once** at the start of the session (`LUCERNA_BIN`, then the release bundle per `scripts/target-dir.sh`, then `luc` on PATH) and recorded in `meta.lucerna_binary` (`path`, `mtime`, `version` = `luc --version`); `cargo metadata` is not called during measurement. Missing or unable to run → stop immediately; binary disappears mid-session → stop before the next Lucerna launch (partial results saved).

Command used to launch each editor (all run the main binary directly, and that binary owns the window):

| Editor | Command |
|---|---|
| VS Code | `…/Visual Studio Code.app/Contents/MacOS/Code --user-data-dir <bench>/profiles/vscode --extensions-dir <bench>/profiles/vscode-ext --disable-extensions --new-window <target>` (with LSP: `--extensions-dir …/vscode-ext-lsp`, no `--disable-extensions`). Not through `bin/code`, because it adds a node/CLI step that the other two editors do not have. The child environment drops `ELECTRON_RUN_AS_NODE` and `ELECTRON_NO_ATTACH_CONSOLE` so `Code` always runs as an app. |
| Zed | `…/Zed.app/Contents/MacOS/zed --user-data-dir <bench>/profiles/zed <target>`. Not through `cli`, because `cli` goes through LaunchServices and drops `--user-data-dir`. Zed allows only one instance per channel: if your own Zed is running (anywhere, matching `Zed.app/Contents/MacOS/zed`), bench stops. |
| Lucerna | `<resolved binary> <target>` with `HOME=<bench>/profiles/lucerna-home`; the pid, start time and binary path are written to `lucerna-home/.bench-pids.json`. |

## Verified paths and keys

| Editor | Settings file | Notes |
|---|---|---|
| VS Code | `profiles/vscode/User/settings.json` | Follows `--user-data-dir`. |
| Zed | `profiles/zed/config/settings.json` | Zed 1.28.1: the help for `--user-data-dir` says it is the folder holding the data; the binary has `config` and `data` subfolders (`strings`); `config/settings.json` follows how Zed computes `custom_data_dir`. Zed also writes more into `profiles/zed/config` (and `data`) while running; the profile is deleted at the start of each configuration so it does not carry over. The keys `auto_install_extensions` (`{"html": false}`) and `suggest_extensions` (`false`) follow Zed's documentation (Configuring Zed) and are present in the binary. Zed has not been launched to verify this. |
| Lucerna | `profiles/lucerna-home/.config/lucerna/settings.json` | Lucerna uses `$HOME/.config/lucerna` (`crates/lucerna-workbench/src/config_files.rs:52`), not `Library/Application Support`. |
| Lucerna trust | `profiles/lucerna-home/Library/Application Support/Lucerna/trusted.json` | `{"folders": [...], "parents": [...]}` (`trust.rs:40-47`, file name `trusted.json` at `trust.rs:35`); data folder from `workspace_state.rs:236`. Writes `<data>` into both lists. |

Lucerna keys: `editor.fontSize`, `workbench.startupEditor: "none"` (`startup.rs:14`; "none" asks to pick a folder, Cancel exits), LSP off with `lucerna.lsp.enabled: false` (`default-settings.json:62`). With `lsp=True`: `"lsp": {"rust-analyzer": {"binary": {"path": ...}}}` (Zed's form, per the phase 3 LSP spec). Lucerna has no "do not reopen the previous session" key: the `HOME` profile is empty, so there is no previous session to reopen.
