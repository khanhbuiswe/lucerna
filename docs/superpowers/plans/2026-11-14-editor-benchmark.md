# Performance comparison of Lucerna, Zed and VS Code — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A `bench/compare/` toolset that measures Lucerna, Zed and VS Code in a clean environment, from outside the app, then generates a comparison table and charts for publication.

**Architecture:** Plain Python (no external libraries) orchestrates: builds empty profiles, checks the machine, runs each editor, measures, writes JSON. A small Swift helper (`bench-helper`) does the parts that need system APIs: screen capture with ScreenCaptureKit to find the moment a frame changes, and key/scroll event generation with CGEvent. `report.py` turns the JSON into a Markdown table and SVG.

**Tech Stack:** python3 (the one bundled with macOS) + `unittest`; Swift via the Command Line Tools' `swiftc`; macOS `footprint`, `ps`, `pmset`, `du`.

**Spec:** `docs/superpowers/specs/2026-11-14-editor-benchmark-design.md`

## Global Constraints

- No Python libraries outside the stdlib; run with `python3 -I`. Tests: `python3 -m unittest discover -s bench/compare/tests -v`.
- No Xcode on the machine (no `xctrace`): every frame measurement uses `bench-helper` (ScreenCaptureKit), not Instruments.
- Never touch real data: every profile lives under `<bench>` (default `~/LucernaBench.noindex`, changed with `--bench-dir`).
- VS Code runs with `/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code --user-data-dir <bench>/profiles/vscode --extensions-dir <bench>/profiles/vscode-ext`; Zed with `/Applications/Zed.app/Contents/MacOS/cli --user-data-dir <bench>/profiles/zed`; Lucerna with `luc` and `HOME=<bench>/profiles/lucerna-home`.
- 10 rounds per measurement, report median and p90; the editor order rotates every round.
- Published wording is neutral, no winner/loser coloring (CLAUDE.md).
- Commit: stage by path, trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`; check `.git/MERGE_HEAD` before committing.

## Review Focus

- An editor is already running (the user's real instance is open): the script must open its own session with the bench profile, not attach to an existing window, and not close the user's session — Task 2 has a test for recognizing processes by `--user-data-dir`/`HOME`.
- The machine is not quiet midway (a session starts cargo): preflight runs again before every round, not just at the start of the session — Task 1 test.
- First-run dialogs (VS Code trust, Zed welcome, Lucerna Restricted Mode) cover the content and skew startup numbers: the pre-made settings must turn them off; the first startup round is always discarded — Task 1 and Task 5 tests.
- Missing Screen Recording/Accessibility permission: the helper must report a clear error and exit with a non-zero code, not return a 0 number — Task 3 test.
- A failed round (timeout): record `null` for that round, the statistics skip it and the number of failed rounds goes into the report, without breaking the whole session — Task 5 and Task 6 tests.

---

### Task 1: Clean environment and preflight

**Files:**
- Create: `bench/compare/env.py`, `bench/compare/tests/test_env.py`, `bench/compare/README.md`

**Interfaces:**
- Produces:
  - `BenchPaths(root: Path)` dataclass with `profiles`, `data`, `results`, `big_file`, `large_repo`, `medium_repo`, `marker_file`.
  - `prepare_data(paths: BenchPaths) -> None`: calls `scripts/gen-bench-repo.sh <data>/large` if it does not exist yet; clones a medium repo (ripgrep, tag `14.1.1`) into `<data>/medium` if it does not exist yet; writes `marker.txt` (200 lines, each line 80 `█` characters).
  - `write_profiles(paths: BenchPaths, lsp: bool) -> None`: settings for the three editors (table below).
  - `preflight() -> list[str]`: the list of reasons it fails (empty = passes).

Settings (same font size 14, no restoring the previous session):

| Editor | File | Keys |
|---|---|---|
| VS Code | `profiles/vscode/User/settings.json` | `telemetry.telemetryLevel: "off"`, `update.mode: "none"`, `security.workspace.trust.enabled: false`, `workbench.startupEditor: "none"`, `window.restoreWindows: "none"`, `editor.fontSize: 14`, `editor.minimap.enabled: false`, `extensions.autoCheckUpdates: false` |
| Zed | `profiles/zed/config/settings.json` (check the sub-path under `--user-data-dir` on the installed version, write it into the README) | `telemetry: {diagnostics: false, metrics: false}`, `auto_update: false`, `buffer_font_size: 14`, `disable_ai: true`, `restore_on_startup: "none"`, `minimap.show: "never"` |
| Lucerna | `profiles/lucerna-home/Library/Application Support/Lucerna/config/settings.json` + `trust.json` trusting `<data>` | `editor.fontSize: 14`, `workbench.startupEditor: "none"` |

With `lsp=False`: VS Code installs no extensions; Zed `languages.Rust.language_servers: []`; Lucerna `lsp.enabled: false` (or the equivalent key present in settings, verify). With `lsp=True`: all three point to the same `rust-analyzer` on PATH (VS Code installs `rust-lang.rust-analyzer` into `vscode-ext` and sets `rust-analyzer.server.path`).

Preflight fails when: a `cargo`/`rustc` process exists; the 1-minute load average > 2.0 (`os.getloadavg`); `pmset -g therm` shows `CPU_Speed_Limit` < 100; `pmset -g batt` is not "AC Power"; Low Power Mode is on (`pmset -g` has `lowpowermode 1`).

- [ ] **Step 1: Tests** `test_write_profiles_vscode_disables_trust_and_telemetry` (read the JSON written, assert the keys above), `test_write_profiles_zed_no_lsp` (`language_servers == []` when `lsp=False`), `test_preflight_flags_cargo_and_thermal` (patch the system-reading functions to return a running `cargo` and `CPU_Speed_Limit = 80` → two reasons), `test_preflight_ok_when_quiet` (→ `[]`).
- [ ] **Step 2: Run, see FAIL** — `python3 -m unittest bench/compare/tests/test_env.py -v`.
- [ ] **Step 3: Write `env.py`.** Wrap the system commands in small functions (`_processes()`, `_pmset(args)`) so tests can patch them.
- [ ] **Step 4: Run, see PASS.**
- [ ] **Step 5: README** records: the permissions to grant (Screen Recording, Accessibility for the terminal), the `sudo purge` command the user runs themselves, how to build the environment. **Commit** `feat(bench): clean profiles and preflight for the editor comparison`.

### Task 2: Launch and clean up each editor

**Files:**
- Create: `bench/compare/editors.py`, `bench/compare/tests/test_editors.py`

**Interfaces:**
- Consumes: `BenchPaths`.
- Produces:
  - `EDITORS = ("lucerna", "zed", "vscode")`.
  - `launch(editor: str, paths: BenchPaths, target: Path) -> Launched` (`Launched(t0_ns: int, popen)`; `t0_ns` = `time.monotonic_ns()` right before `Popen`).
  - `bench_pids(editor: str, paths: BenchPaths) -> list[int]`: every process of the bench session — recognized by a command line containing the bench profile path (`--user-data-dir <bench>…` for VS Code/Zed and all their child helpers through the PPID tree; for Lucerna: a `luc` process whose `HOME` environment variable = the bench profile, read with `ps eww`).
  - `quit(editor: str, paths: BenchPaths) -> None`: SIGTERM the `bench_pids`, wait 5 s, then SIGKILL; never touches a process outside the list.
  - `app_bundle(editor) -> Path` and `version(editor) -> str` (reads `CFBundleShortVersionString`; Lucerna: `luc --version` + `git rev-parse --short HEAD`).

- [ ] **Step 1: Tests** `test_launch_command_vscode_uses_bench_profile` (the command line contains `--user-data-dir <bench>/profiles/vscode` and `--extensions-dir`), `test_bench_pids_ignores_users_own_session` (given a fake `ps` with a user's own Zed without `--user-data-dir` and a bench Zed with its child → returns only the bench PID and its child), `test_lucerna_env_home` (the env of `launch("lucerna")` has `HOME` = the bench profile).
- [ ] **Step 2: FAIL.** **Step 3: Write `editors.py`** (`_ps()` wraps the calls `ps -axo pid,ppid,command` and `ps eww -o command= -p`). **Step 4: PASS.**
- [ ] **Step 5: Commit** `feat(bench): launch and clean up each editor in its own profile`.

### Task 3: Swift helper `bench-helper`

**Files:**
- Create: `bench/compare/helper/main.swift`, `bench/compare/helper/build.sh`, `bench/compare/tests/test_helper.py`

**Interfaces:**
- Produces (CLI, every result prints one JSON line to stdout; errors print to stderr and exit with code ≠ 0):
  - `bench-helper check` → `{"screen_recording": bool, "accessibility": bool}`; exits 3 if either permission is missing.
  - `bench-helper watch --pid <pid> --since <t0_ns> --until coverage:<0..1>|change [--timeout <s>] [--baseline <png>]` → `{"t_ns": <monotonic ns of the first frame meeting the condition>, "frames": n}`. Captures the largest window of `pid` at the display refresh rate. `coverage:x`: the fraction of pixels differing from the background color in the middle of the window ≥ x (used for the `marker.txt` file made entirely of `█`). `change`: the first frame that differs from the previous frame before `t0` by more than a 0.5% pixel threshold.
  - `bench-helper scroll --pid <pid> --lines <n> --rate <events/s>` and measures at the same time: `{"frames": n, "dropped": d, "refresh_hz": r}`; a "dropped" frame is a gap between two frames with new content > 1.5 × the display period while scroll events are still being sent.
  - `bench-helper type --pid <pid> --text <s>` sends keys through CGEvent, prints `{"t_ns": [...]}` with the time of each key.
- Clock: `clock_gettime_nsec_np(CLOCK_UPTIME_RAW)`, matching Python's `time.monotonic_ns()` on macOS (verified in a test).
- `build.sh` compiles `swiftc -O main.swift -o <bench>/bin/bench-helper` (ScreenCaptureKit, CoreGraphics, AppKit).

- [ ] **Step 1: Tests** `test_check_reports_permissions` (run `check`, the JSON has two bool keys), `test_missing_permission_exits_nonzero` (skipped with `skipUnless` when the machine has all permissions; with the variable `BENCH_HELPER_FAKE_DENY=1` the helper pretends a permission is missing → code 3, prints no measurement), `test_clock_matches_python` (`bench-helper now` prints ns; the difference with `time.monotonic_ns()` < 5 ms).
- [ ] **Step 2: FAIL.** **Step 3: Write the helper.** **Step 4: PASS** (and run `watch` by hand on a TextEdit window with `marker.txt` open to see a reasonable `t_ns`).
- [ ] **Step 5: Commit** `feat(bench): screen-capture and input helper`.

### Task 4: Measurements

**Files:**
- Create: `bench/compare/metrics.py`, `bench/compare/tests/test_metrics.py`, `bench/compare/tests/fixtures/footprint.txt`

**Interfaces:**
- Consumes: `launch`, `bench_pids`, `quit`, helper CLI.
- Produces — each function returns `float | None` (None = failed round/60 s timeout):
  - `startup_ms(editor, paths)`: open `marker.txt`, `watch --until coverage:0.3`, return `(t_ns - t0_ns)/1e6`.
  - `idle_ram_mb(editor, paths)`: open the medium repo, open 10 fixed `.rs` files (list in the code), idle 60 s, the sum of `phys_footprint` over `bench_pids` from `footprint -p <pid>…`.
  - `idle_cpu_pct(editor, paths)`: after the state above, the sum of `%cpu` over `bench_pids` sampled every second for 60 s, return the average.
  - `scroll_dropped_pct(editor, paths)`: open `big.rs`, `scroll --lines 3000 --rate 120`, return `dropped/frames*100`.
  - `file_change_ms(editor, paths)`: open `marker.txt`, overwrite it with an empty file, `watch --until change`, return ms.
  - `quick_open_ms(editor, paths)`: open the large repo, wait for the scan to finish (30 s), open Quick Open (⌘P), `type --text mod0042`, `watch --until change` after the last key, return ms from the last key.
  - `app_size_mb(editor)`: `du -sk` of the bundle.
  - `parse_footprint(text) -> int` (bytes).
- [ ] **Step 1: Tests** `test_parse_footprint_sums_processes` (a fixture recorded from the real `footprint` with two processes → correct sum), `test_metric_timeout_returns_none` (a fake helper returns code ≠ 0 → `None`), `test_startup_uses_marker_and_coverage` (patch `launch`/helper, check the arguments sent to the helper).
- [ ] **Step 2: FAIL.** **Step 3: Write `metrics.py`.** **Step 4: PASS.**
- [ ] **Step 5: Commit** `feat(bench): startup, memory, CPU, scroll, reload and Quick Open measurements`.

### Task 5: The runner

**Files:**
- Create: `bench/compare/run.py`, `bench/compare/tests/test_run.py`

**Interfaces:**
- Consumes: Tasks 1–4.
- Produces: `python3 -I bench/compare/run.py [--bench-dir D] [--rounds 10] [--lsp off|on|both] [--cold] [--only metric,…]` writes `bench/compare/results/<yyyy-mm-dd>-<commit>.json`:
  `{"meta": {"date", "lucerna_commit", "versions": {editor: str}, "macos", "chip", "ram_gb", "rust_analyzer"}, "configs": [{"lsp": bool, "cold": bool, "metrics": {metric: {editor: [float|null, …]}}}]}`.
  - `stats(values) -> {"median", "p90", "failed"}` (drops `None`, p90 by nearest-rank).
  - `order(round: int) -> list[str]`: rotates `EDITORS` by round.
- Rules: preflight before every round (if it fails, wait 30 s, retry up to 10 times, then stop with a message); rest 20 s between two editor runs; the first startup round of each editor runs and is discarded; `--cold` pauses before every startup round, prints "run `sudo purge` then press Enter".
- [ ] **Step 1: Tests** `test_order_rotates` (`order(0) != order(1)`, each editor comes first exactly once in three rounds), `test_stats_ignore_failed` (`[10, None, 30, 20]` → median 20, failed 1), `test_first_startup_discarded` (patch the metric, check 11 calls for 10 recorded values), `test_preflight_rechecked_each_round`.
- [ ] **Step 2: FAIL.** **Step 3: Write `run.py`.** **Step 4: PASS.**
- [ ] **Step 5: Commit** `feat(bench): rounds, rotation, preflight and result file`.

### Task 6: Comparison report

**Files:**
- Create: `bench/compare/report.py`, `bench/compare/tests/test_report.py`, `bench/compare/tests/fixtures/result.json`

**Interfaces:**
- Consumes: the JSON file from Task 5.
- Produces: `python3 -I bench/compare/report.py <result.json> --out <dir>` writes `<dir>/results.md` (one table per configuration: columns Metric | Lucerna | Zed | VS Code; cell `trung vị (p90 …)` in the same unit; a ratio line under each metric of the form `Zed 1,4× · VS Code 2,1×` computed from the median versus Lucerna; failed rounds are written as "n/10 rounds failed") and `<dir>/<metric>-<config>.svg` (three bars in one neutral color, number labels on the bars, no winner/loser color).
  - `ratio(other: float, lucerna: float) -> str` (one decimal digit, decimal comma).
- [ ] **Step 1: Tests** `test_table_has_three_editor_columns` (the header line is exactly `| Chỉ số | Lucerna | Zed | VS Code |`), `test_ratio_format` (`ratio(140, 100) == "1,4×"`), `test_failed_rounds_reported`, `test_svg_has_three_bars` (count of the bars' `<rect` = 3).
- [ ] **Step 2: FAIL.** **Step 3: Write `report.py`.** **Step 4: PASS.**
- [ ] **Step 5: Commit** `feat(bench): comparison tables and bar charts`.

### Task 7: Docs, references, trial run

**Files:**
- Create: `docs/benchmarks.md`
- Modify: `docs/superpowers/specs/2026-11-14-editor-benchmark-design.md` (References section), `README.md` (one line linking to `docs/benchmarks.md`)

- [ ] **Step 1: Research** how VS Code (`--prof-startup`, Developer: Startup Performance) and Zed measure performance; write it into the spec's References section, saying why we measure from outside.
- [ ] **Step 2: Write `docs/benchmarks.md`**: purpose, method (environment, each measurement, number of rounds, statistics), known limits (Lucerna has fewer features; different architectures; measuring from outside includes the same system latency for all three editors), how to rerun, where the results table goes (left empty until the official measurement).
- [ ] **Step 3: Trial run** `run.py --rounds 2 --lsp off` on the current Lucerna build (release build with `scripts/bundle-app.sh` when no session is building), then `report.py`. The trial results are **not committed**; record the problems met in the final report.
- [ ] **Step 4: Commit** `docs: benchmark methodology`.

The official measurement (10 rounds, all four configurations) happens after Git and Terminal have merged, when every session is stopped; the user runs it and commits `results/`.
