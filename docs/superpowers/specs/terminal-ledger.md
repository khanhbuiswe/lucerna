# Integrated terminal ledger: performance and manual checks

Date: 2026-11-13. Spec: `2026-11-13-terminal-design.md` (§9.2 thresholds, §11 testing). Plan: `../plans/2026-11-13-terminal.md`, Task 13.

Measuring machine: Apple M4, 10 cores, 16 GB RAM, macOS 26 (Darwin 25.6). The spec sets thresholds on M1; M4 is faster than M1, so the numbers below are a lower bound for slower machines.

## 1. Measurements against the §9.2 thresholds

| Measure | Threshold | Result | Status |
|---|---|---|---|
| Prepaint + paint of one 200 × 60 terminal full of colored text (style changes every 3 cells, 4020 runs), release build | ≤ 4 ms | The whole window draw (render, layout, prepaint, paint): p95 **2.35 ms** when the screen is unchanged, **2.46 ms** when every row changes each frame (p50 2.22 / 2.25 ms), idle machine. Rerun after fixing the descent sign (§1.3), machine under heavy load: p95 4.04 / 3.41 ms (p50 3.24 / 2.88 ms) | Passes on an idle machine; needs re-measuring when the machine is idle |
| `cat` of a 100 MB file, PTY and parser side | terminal done ≤ 3 s | **0.71 s** (108-byte lines, 3000 wakeups) | Passes |
| `cat` of a 100 MB file, editor frame next to it ≤ 16 ms | ≤ 16 ms | not measured, needs the real app | Waiting for user check |
| Idle terminal | 0 frames per second | 0 frames, 0 `notify` in 10 s simulated | Passes |
| Panel hidden, terminal flushing output | no drawing | 0 frames over 500 lines of output; output still parsed | Passes |
| Scrollback RAM | bounded by `scrollback` (10,000 lines) | scrollback max 10,000 lines; peak RSS of the test process 55.5 MB at 200 columns, unchanged between `cat` 100 MB and `yes` 50 MB | Passes |
| Whole-app RAM with 1 terminal running Claude Code | grows ≤ 50 MB | not measured, needs the real app | Waiting for user check |
| Release binary `luc` | grows ≤ 1.5 MB | **+1.09 MiB** (+1,147,760 bytes) unstripped; +0.88 MiB after `strip -x`; `__TEXT` +0.80 MiB | Passes |

### 1.1 How it was measured

**Drawing** (`crates/lucerna-workbench/tests/terminal_perf.rs`, without the libtest harness because macOS's real text system can only be created on the main thread):

```
cargo test --release -p lucerna-workbench --test terminal_perf -- --ignored
```

- `TestAppContext::build_with_text_system` with the platform's real text system, so text is really shaped and glyphs really rasterized. The bundled Lilex font, 2× scale like a Retina screen, a window just big enough for 200 × 60 cells (cell 8.40 × 17.00 px).
- Screen: 60 rows, every 3 cells switching one of 8 SGR styles (16 color, bold, italic, underline, 256-color background). 4020 text runs in total.
- Measures 200 `window.draw` calls of a window containing only `TerminalView`, after 10 warm-up runs. Two cases: unchanged screen, and each frame feeding a new screen (every row has different text). Does not include the GPU drawing.
- Measured separately: `layout_line` for 60 rows p95 0.55 ms; `snapshot()` 200 × 60 p95 0.07 ms.

The run on an idle machine (before fixing the descent sign in §1.3; the fix does not change the amount of work per frame):

```
cell 8.40×17.00 px at 2×, grid 200×60
4020 text runs on screen
layout_line, 60 rows: p50 0.524 ms, p95 0.548 ms, max 0.572 ms (200 frames)
snapshot, 200×60: p50 0.059 ms, p95 0.065 ms, max 0.151 ms (200 frames)
window draw, unchanged screen: p50 2.218 ms, p95 2.348 ms, max 2.587 ms (200 frames)
window draw, every row new: p50 2.249 ms, p95 2.462 ms, max 3.990 ms (200 frames)
budget 4 ms: unchanged p95 2.348 ms, every row new p95 2.462 ms
```

The last run, after fixing the descent sign, while another session was building (load average 30 on 10 cores). `layout_line` (code unchanged) was also ~30% slower, so the slowdown comes from machine load:

```
terminal_perf: baseline matches shaped text: ok
cell 8.40×17.00 px at 2×, grid 200×60
4020 text runs on screen
layout_line, 60 rows: p50 0.677 ms, p95 0.717 ms, max 0.878 ms (200 frames)
snapshot, 200×60: p50 0.076 ms, p95 0.078 ms, max 0.140 ms (200 frames)
window draw, unchanged screen: p50 3.244 ms, p95 4.040 ms, max 5.873 ms (200 frames)
window draw, every row new: p50 2.878 ms, p95 3.405 ms, max 3.742 ms (200 frames)
budget 4 ms: unchanged p95 4.040 ms, every row new p95 3.405 ms
```

**Flood** (`crates/lucerna-terminal/tests/flood.rs`, real PTY, no UI; the test thread receives events and takes a snapshot every 64 wakeups like the UI):

```
cargo test --release -p lucerna-terminal --test flood -- --ignored --nocapture --test-threads=1
```

```
cat 100 MB file (108-byte lines): exit Some(0) after 0.71 s, 3000 wakeups, 46 snapshots, scrollback max 10000 lines (cap 10000), peak RSS 55.4 MB (before 2.0 MB)
yes | head -c 50 MB: exit Some(0) after 15.77 s, 2951183 wakeups, 46112 snapshots, scrollback max 10000 lines (cap 10000), peak RSS 55.5 MB (before 55.4 MB)
```

`yes` is the worst case by line count: 25 million lines of 2 bytes, each line scrolling the grid once, about 0.63 µs per line in alacritty's parser. `cat` of 100 MB has only ~0.93 million lines. The 55 MB RSS is a full scrollback at 200 columns (10,000 × 200 cells × ~24 bytes ≈ 48 MB); at 120 columns as in the spec's estimate it is about 30 MB.

**Idle and hidden panel** (`crates/lucerna-workbench/tests/terminal_view.rs`, run with the normal test suite): `idle_prompt_draws_nothing_for_ten_seconds`, `hidden_busy_panel_draws_nothing`.

**Binary**: `cargo build --release --bin luc` on this branch (including the drawing change in §1.2) and on the branch point `9f7d77c` (a temporary `git worktree`, its own target, deleted after measuring). Compared with the branch point and not with the current `main`, because `main` has since gained Markdown preview, which is not part of the terminal.

| | `9f7d77c` | terminal branch | Growth |
|---|---|---|---|
| `luc` release | 48,823,568 | 49,971,328 | +1,147,760 (1.09 MiB) |
| after `strip -x` | 40,182,904 | 41,104,288 | +921,384 (0.88 MiB) |
| `__TEXT` | 36,536,320 | 37,371,904 | +835,584 |

### 1.2 Drawing: the first measurement exceeded the threshold, fixed

First measurement (one `shape_line` and one `ShapedLine::paint` per run): unchanged screen p95 5.55 ms, every row changed p95 10.15 ms. Broken down: prepaint 1.5 ms (GPUI's row cache hit) or 4.7–5.2 ms (reshaping), paint 2.8 ms for 4020 single-line paints.

Tried and dropped:

- **Merging a whole row into one multi-run `shape_line`:** unchanged screen dropped to 2.2 ms but every row changed rose to 30 ms. GPUI splits font runs at every color change, and CoreText is much slower on a row with many font runs.
- **Drawing glyph by glyph without a layer:** 7.8 ms for 12,000 glyphs, because each primitive is ordered separately in the scene's bounds tree.

What was kept (`element.rs`):

- A run consisting only of ordinary narrow characters (1 cell wide, no combining marks) is not shaped. Each character's glyph per style (regular, bold, italic) is looked up once with `layout_line` of one character, then cached in `TerminalView` (cleared when the font changes; rebuilt past 8192 characters). Ligatures are off and the grid is fixed, so the result is the same as shaping the whole run. Fallback fonts are still right because the cache keeps the `font_id` that CoreText chose.
- Paint draws those glyphs with `paint_glyph` at the exact cell, on the same baseline that a row shaped with the terminal font has; underline and strikethrough are drawn per run.
- All text is drawn in one `paint_layer`.
- A run with wide characters, combining marks, emoji or characters that do not produce exactly one glyph is still shaped as before (forcing the cell width).

The damage-based shape cache of §9.1 is not done: prepaint of 200 × 60 is now ~1.4 ms even when every row changes (under §9.1's 2 ms mark), and the cost no longer depends on whether the screen changed. The remaining note from Task 8 (prepaint rebuilds the whole grid on every window frame while the terminal is shown) therefore costs only ~1.4 ms per redrawn frame; if M1 measurements exceed 2 ms, do a per-row cache later.

### 1.3 Fix: baseline of text drawn from the glyph cache

The first version of §1.2 took the font's `descent` with its negative sign (font-kit returns descent as a negative number), so the baseline was lower than the shaped row's baseline by about one descent (17.60 px versus 13.39 px in a 17 px tall cell), making text misaligned with wide characters, combining marks, the block cursor and IME text on the same row, and underline and strikethrough offset along with it. Fixed with `abs()`, like GPUI's layout. `terminal_perf` now always checks (even in a normal `cargo test`, without `--ignored`) that the baseline of text drawn from glyphs equals `(cell height − ascent − descent) / 2 + ascent` of `layout_line("m")` using the real text system.

### 1.4 Fixes after the final branch review

Each item has a regression test, written first and seen failing before the fix.

1. Clicking the panel's tab bar (tab, × including on a background tab, middle button, ⤢) left focus on the panel frame and not on the terminal: the input handler (IME, Telex typing) was not installed, and the cursor was drawn empty. `TerminalPanel` now moves the panel's own focus to the shown terminal (`cx.on_focus`), and clicking a tab or closing a tab while focus is inside the panel focuses the terminal that gets shown. Tests: `clicks_in_the_tab_bar_keep_the_terminal_focused`, `middle_click_close_keeps_the_terminal_focused`.
2. OSC 4/10/11/12 color queries were always answered with a fixed dark palette, so Neovim and Codex picked the dark style under Lucerna Light. The palette used to answer now lives in the terminal (`Terminal::set_query_palette`), `TerminalView` sets it at creation and every time `theme_generation` changes. The fixed palette in `lucerna-terminal` is removed; until a palette is set, no answer is given. Tests: `background_query_answers_with_the_theme`, `color_request_is_answered_from_the_query_palette`.
3. A terminal reopened in the same window always ran as Restricted Mode because it started before the trust store finished reading. It now waits for the trust store's answer (`open_restored_terminal`). Test: `restored_terminal_in_a_trusted_folder_uses_workspace_settings`.
4. `foreground_process` could read an fd that had been closed and reused. Now `Terminal::with_pty_fd` holds the `pty_open` lock during the `tcgetpgrp` call. Test: `with_pty_fd_keeps_the_fd_open_while_it_runs`.
5. `env -u` treated a program path containing `=` as a variable assignment. Such a program now runs through `/bin/sh -c 'exec "$0" "$@"'`, because both BSD and GNU `env` still read `a=b` as an assignment even with `--`. `ALACRITTY_WINDOW_ID` and `WINDOWID` (added by alacritty itself) are also removed from the shell's environment. Tests: `program_path_with_equals_sign_runs`, `alacritty_window_variables_do_not_reach_the_child`.
6. ⌘+click on a file that is already the active editor while the panel is maximized showed nothing. A file link now always returns the panel to its normal size. Test: `link_to_the_shown_file_restores_a_maximized_panel`.
7. `PanelState` has `#[serde(default)]`: a state file missing a field can still be read. Test: `panel_state_with_missing_fields_keeps_the_others`.

The manual check items in §2 are still not ticked: waiting for the user's check, especially Telex typing after clicking to switch tabs and OSC 11 under Lucerna Light (Neovim, Codex).

## 2. Manual checks (waiting for user check)

Run the release build (`scripts/bundle-app.sh`), put it next to Zed when comparing the UI (design-principles §1.1).

### 2.1 Performance in the app

- [ ] `cat` of a 100 MB file in the terminal while scrolling the editor next to it: editor frames ≤ 16 ms (GPUI frame time log), `cat` done ≤ 3 s. Waiting for user check.
- [ ] `yes` for 30 s with the panel hidden: the app's RSS does not grow beyond the scrollback level. Waiting for user check.
- [ ] Terminal at a prompt for 10 s: CPU ~0 in Activity Monitor. Waiting for user check.
- [ ] Whole-app RAM with one terminal running Claude Code grows ≤ 50 MB compared with no terminal. Waiting for user check.

### 2.2 Real programs (spec §11)

- [ ] Claude Code: border frames, colors, scrolling while running, ⇧Enter via the `sendSequence` binding, ⌘+click on a path Claude Code prints opens the right file and line. Waiting for user check.
- [ ] Codex: draws and takes input correctly. Waiting for user check.
- [ ] vim: mouse mode, alt screen (quitting vim returns the shell screen). Waiting for user check.
- [ ] htop: drawing, colors, keys. Waiting for user check.
- [ ] Resize the panel while Claude Code is drawing: frames not broken, no panic. Waiting for user check.
- [ ] Typing Vietnamese with Telex in zsh and in Claude Code: the text being typed shows at the cursor, diacritics correct. Waiting for user check.
- [ ] Change theme while the terminal is open: colors change immediately. Waiting for user check.
- [ ] Under Lucerna Light, Neovim 0.10+ and Codex pick the light style (OSC 11). Waiting for user check.
- [ ] Two Claude Code tabs: click to switch tabs then type Telex right away, the text being typed shows at the cursor. Waiting for user check.
- [ ] Text with combining marks, CJK and emoji shows in the right cells after the drawing change in §1.2 (the shape path is still used for them). Waiting for user check.

### 2.3 Compare the UI with Zed (Task 13 step 6)

- [ ] Tab bar height, panel background and border, sash, spacing of the ＋ ⤢ × buttons. Waiting for user check.
- [ ] Empty state, exited-process state (`[Process exited with code N]`), a tab with a bell or non-zero code. Waiting for user check.
- [ ] Empty-outline cursor when focus is lost, left margin of one cell, scrollbar. Waiting for user check.

## 3. Other

- VS Code's default ANSI palette (light | dark) is recorded in research §1.5, matching `TerminalColors::fallback_ansi` in `lucerna-theme`.
- `cargo deny check`: `advisories ok, bans ok, licenses ok, sources ok`.
