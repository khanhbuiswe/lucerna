# Vim V2a (operators) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Vim editing in Lucerna, covering:
- operators with motions and counts;
- registers, including the system clipboard;
- paste, the single-key commands, undo and redo, and `.`;
- text objects;
- operators in Visual and Visual Line mode.

Each command is one undo step.

**Architecture:**
- `lucerna-vim` stays gpui-free. It now depends on `lucerna-editor` and builds `EditPlan`s through a new `ViewInfo::edit_ctx()`.
- The workbench `VimLayer` applies one `feed`'s edits inside a new `CodeView::transact_group`.
- `.` and counted inserts return `Outcome::Replay(steps)`. The workbench runs those steps one at a time, so each step sees the text the previous one produced.
- Typed text in Insert mode is recorded from new `CodeViewEvent::Typed` events and replayed through the editor's typing path.

**Tech Stack:** Rust 2024, `lucerna-text`, `lucerna-editor`, `lucerna-vim`, gpui (workbench only). No new external dependencies.

**Spec:** `docs/superpowers/specs/2026-11-13-vim-editing-design.md`. This plan covers step **V2a** of §12: §3 (without the `:` line), §4, §5, §6.1–§6.3 and §7, plus the settings listed for V2a. V2b (macros, `:`, yank highlight) gets its own plan. Research: `docs/research/2026-11-09-vim-mode.md` §6. Read the spec section each task cites.

## Global Constraints

- **Licensing:**
  - The project is MIT OR Apache-2.0.
  - **Never copy code from Zed's `crates/vim`**, which is GPL-3. Its design may be learned.
  - VSCodeVim test cases (MIT) may be adapted. Each adapted test carries `// Adapted from VSCodeVim <path>: "<test title>"`.
- **Crates:**
  - Only `lucerna-workbench` uses gpui.
  - `lucerna-vim` may depend on `lucerna-text`, `lucerna-editor` and `unicode-segmentation` only.
- **Positions:**
  - `Point.column` and buffer offsets are UTF-8 bytes.
  - Never split a char.
  - Normal mode never rests past the last char.
- **Edits:**
  - Every text change is an `EditPlan`, built with `lucerna-editor` functions or `EditPlan::new`.
  - Edits are in pre-edit offsets; `selections` are in post-edit offsets.
  - A view that is read-only (`edit_ctx() == None`) gets no edit actions; `y` still works.
- **Undo:** one `feed` equals one undo step (`transact_group`). A command that ends in Insert keeps the step open until Escape. One `.` replay is one step.
- **Settings and defaults (spec §9):**

  | Key | Default |
  |---|---|
  | `vim.useSystemClipboard` | `"onYank"` (also accepts `true`/`false`) |
  | `vim.joinspaces` | `true` |
  | `vim.changeWordIncludesWhitespace` | `false` |
  | `vim.tildeop` | `false` |

- **Register semantics:** exactly spec §6.2. The clipboard-change check is the hash rule there.
- **Shared checkout (git):**
  - Before every commit, run `git status`. If it shows a merge in progress, or files staged that are not yours, stop and report BLOCKED.
  - Commit only your paths: `git commit -m "…" -- <paths>`.
  - Never rebase, never `git pull --rebase`, never stash, never amend.
  - Trailer: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- **Build:**
  - Run `scripts/prune-target.sh` before every `cargo test` or build.
  - Keep one profile (delete `target/release` before a debug build or test, unless another cargo is running: `pgrep -x cargo`).
  - Cargo needs `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`.
- **Talk to the user in Vietnamese.** Code and comments are in English.

## Review Focus

1. **Telex/IME during `ciw` and the `.` replay.** A composition (marked text) committed in Insert must record exactly one `Typed::Text(final)`, not the intermediate marked strings. Expected: `.` reproduces the committed word. Test: `Task 8: ime_commit_records_final_text_for_dot`.
2. **Agent writes the file between an operator and `.`.** The replay must use the current text; no stale offsets, no panic. Test: `Task 8: dot_after_external_reload_uses_new_text`.
3. **`dd` / `p` on the last line with no trailing newline, and on a one-line or empty file.** Expected: Vim results, no stray blank line, no panic. Tests: `Task 3: dd_last_line_without_newline`, `Task 5: p_linewise_on_last_line_without_newline`, `Task 3: dd_on_empty_file`.
4. **Two views of one file.** An operator in view A shifts view B's vim cursor and marks, and `u` in B undoes A's change (the document undo is shared). Test: `Task 8: edit_in_one_view_remaps_other_view_and_marks`.
5. **The clipboard changes between `dd` and `p`, by the user copying in another app.** Expected: with `"onYank"`, `p` pastes the new clipboard. With the clipboard unchanged since the `dd`, `p` pastes the deleted line. Test: `Task 2` table rows, plus `Task 8: clipboard_rules_end_to_end`.

---

## File Structure

```
crates/lucerna-editor/src/
  case.rs            NEW  change_case(ctx, ranges, Case) -> EditPlan
  edit.rs            NEW  replace_ranges(ctx, edits, cursor) -> EditPlan (generic helper)
  plan.rs            EditPlan derives PartialEq
crates/lucerna-vim/src/
  lib.rs             Action/Outcome additions, ViewInfo::edit_ctx, Settings fields, re-exports
  operator.rs        NEW  Operator, MotionKind, Range+kind computation, Vim exclusive rules
  object.rs          NEW  text objects
  edit.rs            NEW  operator/command → EditPlan + register writes
  register.rs        NEW  Registers, ClipboardMode, write/read rules, clipboard fingerprint
  repeat.rs          NEW  LastChange, Typed, ReplayStep
  state.rs           pending-operator states, new keys, record_typed, replay outcomes
  test_util.rs       TestView::edit_ctx; run_edit() applies Edit actions between keys
crates/lucerna-workbench/src/
  code_view/transact.rs   transact_group; Typed events from type_text / delete_left / tab / IME commit
  code_view/mod.rs        CodeViewEvent::Typed
  vim.rs                  apply Edit/Undo/Redo/WriteClipboard/Replay; clipboard in; marks on Edited; settings
crates/lucerna-settings/assets/default-settings.json   the four vim.* keys
crates/lucerna-workbench/tests/vim_editing.rs          NEW integration tests
```

---

### Task 1: Foundations — editor helpers, `edit_ctx`, `Action::Edit`, test harness

Spec: §3 and §3.1.

**Files:**
- Create: `crates/lucerna-editor/src/case.rs`, `crates/lucerna-editor/src/edit.rs`.
- Modify:
  - `crates/lucerna-editor/src/lib.rs` (`pub mod case; pub mod edit;`);
  - `crates/lucerna-editor/src/plan.rs` (`#[derive(Debug, Clone, PartialEq)]` on `EditPlan`);
  - `crates/lucerna-vim/Cargo.toml` (add `lucerna-editor.workspace = true`, and dev-dep `lucerna-editor` with feature `test-util`);
  - `crates/lucerna-vim/src/lib.rs`, `src/test_util.rs`;
  - `crates/lucerna-workbench/src/vim.rs` (the `Info` adapter implements `edit_ctx`; the match on `Action` compiles with the new variants, which can be `todo!()`-free no-ops until Task 8).

**Interfaces (produced):**

```rust
// lucerna-editor
pub enum Case { Lower, Upper, Toggle }
pub fn case::change_case(ctx: Ctx, ranges: &[Range<usize>], case: Case, cursor: usize) -> EditPlan;
pub fn edit::replace_ranges(ctx: Ctx, edits: Vec<(Range<usize>, String)>, cursor: usize, kind: EditKind) -> EditPlan;
// cursor = post-edit offset of the single resulting cursor; stop_before = stop_after = true

// lucerna-vim
pub trait ViewInfo { …existing…; fn edit_ctx(&self) -> Option<lucerna_editor::Ctx<'_>>; }
pub enum Action { …existing…, Edit(lucerna_editor::EditPlan), Undo(u32), Redo(u32),
                  WriteClipboard { text: String, linewise: bool } }
// Action and Outcome drop `Eq` (EditPlan is PartialEq only). Remove `Action::Yank`: V1's
// visual `y` now emits WriteClipboard{linewise: mode == VisualLine} until Task 2 routes it via registers.
// test_util
fn run_edit(start: &str, keys: &str) -> (String /*text with | cursor*/, Vec<Action>);
fn assert_edit(start: &str, keys: &str, end: &str);              // end text with | cursor
fn assert_edit_with(settings: Settings, setup: impl FnOnce(&mut TestView), start, keys, end);
```

`run_edit` behaviour:
- It feeds one key at a time.
- After each feed, every `Action::Edit(plan)` is applied to the TestView's snapshot. Use `lucerna-text` (`BufferSnapshot::from_text`, then apply edits in reverse order).
- `Vim::cursor()` is clamped against the new snapshot before the next key, mirroring the workbench.
- `Undo` and `Redo` work on a simple snapshot history kept in `TestView`.
- Key notation as in V1, plus `<BS>` and `<lt>`.

- [ ] **Step 1: Write failing tests.**
  - `case::tests`:
    - `change_case(Upper)` on `"abc déf"` over `0..7` → `"ABC DÉF"`.
    - `Toggle` on `"aB"` → `"Ab"`.
    - Cursor at the given offset.
  - `edit::tests`: `replace_ranges` with two ranges applies both, and the cursor is placed in post-edit offsets.
  - `lucerna-vim` `test_util` self-test: a dummy `Vim` without operators keeps its V1 tests green.
  - Visual `vjy` now emits `WriteClipboard { linewise: false }`, and `Vjy` emits `linewise: true`.
- [ ] **Step 2:** Run `cargo test -p lucerna-editor case edit`, then `cargo test -p lucerna-vim`. Expected: FAIL (missing items).
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p lucerna-editor`, `cargo test -p lucerna-vim`, `cargo test -p lucerna-workbench --test vim`. Expected: all PASS (V1 behaviour unchanged).
- [ ] **Step 5:** Commit with message `feat(vim): edit plans from vim; case and replace helpers` (explicit paths).

---

### Task 2: Registers and clipboard rules

Spec: §6.2.

**Files:**
- Create: `crates/lucerna-vim/src/register.rs`.
- Modify: `lib.rs` (`Settings.use_system_clipboard: ClipboardMode`, default `OnYank`), `state.rs` (`VimGlobals.registers`, `"x` prefix).

**Interfaces (produced):**

```rust
pub enum ClipboardMode { OnYank, Always, Never }            // "onYank" | true | false
pub enum RegKind { Charwise, Linewise }
pub struct RegContent { pub text: String, pub kind: RegKind }
pub struct Clipboard { pub text: String, pub whole_line: bool } // what the workbench read
pub(crate) enum WriteKind { Yank, Delete }                  // Delete covers d c x X s S C D
impl Registers {
    pub(crate) fn write(&mut self, name: Option<char>, content: RegContent, kind: WriteKind,
                        mode: ClipboardMode, clipboard: Option<&Clipboard>) -> Option<Action>; // WriteClipboard when due
    pub(crate) fn read(&self, name: Option<char>, mode: ClipboardMode,
                       clipboard: Option<&Clipboard>) -> Option<RegContent>;
}
impl VimGlobals { pub fn set_clipboard(&mut self, clip: Option<Clipboard>); } // workbench calls before feed
```

Rules (verbatim from spec §6.2):
- **Yank** writes `"` and `0`; with a named register it writes that register and `"`, not `0`.
- **Delete or change** writes `"`. If the text has a `\n` or is linewise, shift `1`–`9` and write `1`; otherwise write `-`. A named register writes that register and `"` only.
- **`A`–`Z`** append: the result is linewise if the appended part is. Text = `old + ("\n" if linewise) + new`.
- **`_`** writes nothing, not even `"`.
- **`+` and `*`** read and write the clipboard.
- **Read-only registers:** `.` is the last inserted text (set by Task 7), `%` is the file path, `/` is the last search pattern. `:` stays empty until V2b.
- **Fingerprint:** each write to `"` stores a hash of the current clipboard text (for a yank in `OnYank` or `Always` mode, the yanked text itself).
  - `"` read in `OnYank` mode: use the clipboard if its hash ≠ the stored fingerprint.
  - `"` read in `Always` mode: always use the clipboard.
  - `"` read in `Never` mode: never use the clipboard.
- **Clipboard kind:** `Linewise` if `whole_line`, or if the text equals the last linewise text written; otherwise `Charwise`.

- [ ] **Step 1: Write failing table tests in `register.rs`.**

  | # | Setup | Expected |
  |---|---|---|
  | 1 | yank `"abc"` (OnYank) | `"` = `0` = `abc`; returns `WriteClipboard{abc}` |
  | 2 | `yy` A, then clipboard set to B externally, `dd` C, `read(None)` | C |
  | 3 | `yy` A, then clipboard B, `read(None)` | B (Charwise) |
  | 4 | Never mode, yank | no `WriteClipboard`; `read` ignores the clipboard |
  | 5 | Always mode, delete `"x"` | returns `WriteClipboard{x}` |
  | 6 | three linewise deletes | `1`/`2`/`3` hold the last three, newest in `1` |
  | 7 | delete `"ab"` charwise | `-` = `ab`; `1` unchanged |
  | 8 | `"a` yank `tes` (charwise) then `"A` yank `test2\n` (linewise) | `a` = `tes\ntest2\n` Linewise. *Adapted from VSCodeVim `test/register/register.test.ts`: "A appends linewise text to "a"* |
  | 9 | `"_` delete | `"` unchanged |
  | 10 | named `"x` delete | `-` and `1` unchanged. *Adapted from VSCodeVim `test/register/register.test.ts`: "Small delete register is not set when another register is specified"* |
  | 11 | clipboard `{text:"foo\n", whole_line:true}` and `read(None)` in OnYank with the fingerprint differing | Linewise |

- [ ] **Step 2:** Run `cargo test -p lucerna-vim register`. Expected: FAIL.
- [ ] **Step 3:** Implement.
  - `"x` before a command sets the register for that command. `"` followed by an invalid name cancels.
  - Route Visual `y` through `Registers::write(Yank)`.
  - Use `std::hash::DefaultHasher` for the fingerprint.
- [ ] **Step 4:** Run `cargo test -p lucerna-vim`. Expected: PASS.
- [ ] **Step 5:** Commit with message `feat(vim): registers with clipboard rules`.

---

### Task 3: Operators with motions and counts

Spec: §4.1 and §4.2.

**Files:**
- Create: `crates/lucerna-vim/src/operator.rs`, `crates/lucerna-vim/src/edit.rs`.
- Modify: `state.rs` (operator-pending state, count multiplication, doubled keys).

**Interfaces (produced):**

```rust
pub(crate) enum Operator { Delete, Change, Yank, Indent, Outdent, Lower, Upper, ToggleCase }
pub(crate) enum MotionKind { Exclusive, Inclusive, Linewise }
pub(crate) struct OpRange { pub range: Range<usize>, pub kind: MotionKind }
/// The range a motion covers from `from`, after Vim's exclusive/linewise rules (§4.2).
pub(crate) fn motion_range(s: &BufferSnapshot, from: Point, to: Point, kind: MotionKind,
                           force: Option<MotionKind>) -> Option<OpRange>;
/// Applies `op` to `r`: edits, register writes, new cursor, next mode.
pub(crate) fn apply_operator(op: Operator, r: OpRange, ctx: Ctx, view: &dyn ViewInfo,
                             g: &mut VimGlobals, s: &Settings, register: Option<char>)
                             -> (Vec<Action>, Point, Mode);
```

Rules:
- **Kinds** follow the §4.2 table.
- **Exclusive rule:** if the end is at column 0 of a later row, it moves to the end of the previous line and the motion becomes inclusive; if the start is at or before the first non-blank, it becomes linewise instead.
- **`dw` and `cw`:** `dw` stops at the line end. `cw` and `cW` on a non-blank use `e`/`E` (unless `changeWordIncludesWhitespace`). `dw` on an empty line is linewise; `cw` on an empty line does nothing.
- **`c`:** keeps the indent and the final newline for linewise ranges, then enters Insert. It emits `Action::Edit` followed by `Action::EnterInsert(InsertAt::Before)`.
- **Linewise delete on the last line** takes the preceding `\n`.
- **Cursor after:**
  - `d` linewise → first non-blank (with `startofline`).
  - `y` → range start, unless the motion went forward (then the cursor stays).
  - `>` and `<` → first non-blank of the first line; they use `lines::indent_lines`/`outdent_lines` over a `SelectionSet` spanning the rows.
  - `gu`, `gU`, `g~` → range start; they use `case::change_case`.
- **Doubled operators** (`dd cc yy >> << guu gugu gUU gUgU g~~ g~g~`) cover `count` lines.
- **Count:** pre × post count. `v` or `V` after an operator forces the kind (spec table).
- **Cancel:** Esc or an invalid key cancels; a failed motion cancels with no edit.

- [ ] **Step 1: Write failing tests** with `assert_edit`. Unless noted, cases are adapted from VSCodeVim `test/mode/modeNormal.test.ts`, title in the comment:

  | Start | Keys | End | Title / note |
  |---|---|---|---|
  | `one \|two\n  three` | `dw` | `one\| \n  three` | "Can handle dw across lines (1)" |
  | `\|const a = 1;` | `cw` | `\| a = 1;` | "Can handle 'cw' without deleting following white spaces"; ends in Insert |
  | `\|const a = 1;` | `c2w` | `\| = 1;` | "Can handle 'c2w'" |
  | `\| const a = 1;` | `0cw` | `\|const a = 1;` | "Can handle 'cw' on white space" |
  | `\|text;\ntext` | `llllcw` | `text\|\ntext` | "Can handle 'cw' without removing EOL" |
  | `text tex\|t` | `^de` | `\| text` | "Can handle 'de'" |
  | `\|11\n22\n\t33\n44\n55\n66` | `dj` | `\t\|33\n44\n55\n66` | "Can handle 'dj'" |
  | `\|foo\nbar\n\nfun` | `d}` | `\|\nfun` | "Can handle d} at beginning of line" |
  | `one\ntwo\nthree\n\|four\nfive` | `3dd` | `one\ntwo\n\|three` | "Can handle 3dd off end of document" |
  | `one\ntwo\nthr\|ee\nfour` | `d2gg` | `one\n\|four` | "can delete linewise with d2gg backwards" |
  | `bla\|h` | `$dldldl` | `\|b` | "Can handle 'dl' at end of line" |
  | `\|ABC DEF` | `guw` | `\|abc DEF` | "Can handle guw" |
  | `oNe\nTw\|O\ntHrEe` | `g~~` | `oNe\n\|tWo\ntHrEe` | "Can handle 'g~~'" |

  Lucerna-only cases:
  - `dd_last_line_without_newline`: `a\n\|b` + `dd` → `\|a`.
  - `dd_on_empty_file`: `\|` + `dd` → `\|`, with no panic.
  - `2d3w` deletes 6 words.
  - `dvj` is charwise.
  - `>>` with tab size 4 and spaces → 4 spaces.
  - `yiw`-free `yw` leaves the cursor in place and register `0` = `"one "`.
  - `ciw` on `Xin ch\|ào` → `Xin \|`.
  - A failed motion `dfz` makes no edit.
  - In a read-only view (`edit_ctx` None), `dd` emits no `Edit`.
- [ ] **Step 2:** Run `cargo test -p lucerna-vim operator`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p lucerna-vim`. Expected: PASS.
- [ ] **Step 5:** Commit with message `feat(vim): operators with motions and counts`.

---

### Task 4: Text objects

Spec: §5.

**Files:** create `crates/lucerna-vim/src/object.rs`; modify `state.rs` (`i`/`a` after an operator and in Visual), `lib.rs`, `test_util.rs` (`TestView.nodes: Vec<Range<usize>>`), and `crates/lucerna-workbench/src/vim.rs` (`Info::enclosing_nodes` calls the view's `SyntaxState::enclosing_nodes`, or returns empty without a tree).

**Interfaces:**
- `pub(crate) fn object_range(s: &BufferSnapshot, cursor: Point, obj: Object, around: bool, count: u32, view: &dyn ViewInfo) -> Option<OpRange>`.
- `pub(crate) enum Object { Word, BigWord, Paren, Brace, Bracket, Angle, DoubleQuote, SingleQuote, BackQuote, Tag, Paragraph, Sentence }`.
- `ViewInfo` gains `fn enclosing_nodes(&self, range: Range<usize>) -> Vec<Range<usize>>`. It is empty without a syntax tree. The workbench implements it via the view's `SyntaxState::enclosing_nodes`.

Rules: spec §5 table, verbatim.
- Brackets search forward when the cursor is not inside a pair. When nodes are available, skip brackets inside string and comment nodes.
- `it`/`at` use `enclosing_nodes` when available, else a text scan for matching tag names.
- Paragraph objects are linewise. All others are charwise.

- [ ] **Step 1: Write failing tests** (`assert_edit`). Cases adapted from VSCodeVim `test/mode/modeNormal.test.ts`, title in the comment:

  | Start | Keys | End | Title / note |
  |---|---|---|---|
  | `one (two) th\|ree (four) five` | `cib` | `one (two) three (\|) five` | "Can handle 'cib' between sets of parentheses" |
  | ` b(l(baz(f\|oo)baz)a)h ` | `c3i)` | ` b(\|)h ` | "Can handle count prefixed 'ci)'" |
  | `\|one "two"` | `ci"` | `one "\|"` | "…starting behind the quoted word" |
  | `one "t\|wo" three` | `ca"` | `one \|three` | |
  | `one "t\|wo" \nthree` | `c2i"` | `one \| \nthree` | |
  | `one two th\|ree, four ` | `daw` | `one two\|, four ` | "Can handle 'daw' on word with leading spaces" |
  | `one two three, fo\|ur \nfive six` | `d2aw` | `one two three, \|six` | |
  | `<blink>he\|llo</blink>` | `cit` | `<blink>\|</blink>` | |
  | `one <blink>he\|llo</blink> two` | `cat` | `one \| two` | |
  | `foo\n\n\|\n\nfun` | `dip` | `foo\n\|fun` | |
  | `foo\n\nbar baz\nbar \|baz\n\nfun` | `dap` | `foo\n\n\|fun` | |
  | `That's my sec\|ret, Captain. I'm always angry.` | `das` | `\|I'm always angry.` | |

  Lucerna-only cases:
  - `diw` on whitespace removes the whole run.
  - `di(` on a line with `"(" + (a)`: with the TestView supplying a string node over `"("`, it picks `(a)`.
  - `ci"` after the last pair fails with no edit.
  - `viw` in Visual extends the selection.
- [ ] **Step 2:** Run `cargo test -p lucerna-vim object`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p lucerna-vim`. Expected: PASS.
- [ ] **Step 5:** Commit with message `feat(vim): text objects`.

---

### Task 5: Single-key commands and paste

Spec: §4.3 and §6.3.

**Files:** modify `state.rs` and `edit.rs`.

**Commands:**
- `x X s S C D Y r{c} ~ J gJ p P gp gP`, with counts on `i a I A o O`.
- `vim.joinspaces` and `vim.tildeop` are read from `Settings` (add `joinspaces: bool` and `tildeop: bool`, defaults true and false).

**Counted insert** (`3ifoo<Esc>`):
- At Escape, `Vim` returns `Outcome::Replay(steps)`, where the steps repeat the recorded `Typed` events `count-1` times, followed by `Key::Escape`.
- This depends on Task 7's `Typed` recording. In this task, implement only the count bookkeeping; the test lives in Task 7.

- [ ] **Step 1: Write failing tests.**
  - Adapted from VSCodeVim `test/mode/modeNormal.test.ts` (title in comment):

    | Start | Keys | End | Title / note |
    |---|---|---|---|
    | `o\|ne\ntwo\nthree\nfour` | `3D` | `\|o\nfour` | |
    | `123\|456\n789` | `2ra` | `123a\|a6\n789` | |
    | `123\|456\n789` | `4ra` | unchanged | |
    | `abc\|defg\n12345` | `3r<CR>` | `abc\n\|g\n12345` | |
    | `t\|ext` | `2xo<Esc>p` | `tt\ne\|x` | |
    | `one t\|wo\nthree four` | `1000x` | `one \|t\nthree four` | |
    | `\|teXt` | `4~` | `TEx\|T` | |
    | ` one\n tw\|o\n three` | `2S` | ` one\n \|` | |

  - Adapted from VSCodeVim `normalModeTests/commands.test.ts`:

    | Start | Keys | End |
    |---|---|---|
    | `\|one\ntwo\nthree\nfour` | `3J` | `one two\| three\nfour` |
    | `one.\ntw\|o` | `kJ` | `one.\|  two` (two spaces) |
    | `one(\n)tw\|o` | `kJ` | `one(\|)two` |
    | `on\|e\n ` | `J` | `on\|e` |
    | `\|one\n two \nthree\nfour` | `3gJ` | `one two \|three\nfour` |

  - Adapted from VSCodeVim `test/register/register.test.ts`:

    | Start | Keys | End |
    |---|---|---|
    | `\|test1\ntest2\ntest3\n` | `dd dd dd "1p "2p "3p` | `\ntest3\ntest2\n\|test1` |
    | `\|test1\ntest2\ntest3` | `2x j "-p` | `st1\ntt\|eest2\ntest3` |
    | `\|test1\ntest2\ntest3` | `yy j yy gg dd "0P` | `\|test2\ntest2\ntest3` |
    | `\|asdf\nqwer` | `yyj"_ddkp` | `asdf\n\|asdf` |

  - Adapted from VSCodeVim `test/operator/put.test.ts`:

    | Start | Keys | End |
    |---|---|---|
    | `\|XYZone two three` | `d3l w 2p` | `one tXYZXY\|Zwo three` |
    | `\|abc\none\ntwo\nthree` | `dd j 2gp` | `one\ntwo\nabc\nabc\n\|three` |

  - Lucerna-only: `p_linewise_on_last_line_without_newline`: `a\n\|b` + `yyp` → `a\nb\n\|b`, with no trailing blank line.
- [ ] **Step 2:** Run `cargo test -p lucerna-vim`. Expected: the new tests FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p lucerna-vim`. Expected: PASS.
- [ ] **Step 5:** Commit with message `feat(vim): single-key editing commands and paste`.

---

### Task 6: Visual and Visual Line operators

Spec: §7.

**Files:** modify `state.rs` and `edit.rs`.

**Keys:**
- `d x D X c s C S R y Y > < ~ u U r{c} J gJ p P` in Visual and Visual Line.
- `<C-v>` keeps V1 behaviour: only `y` and `Esc`; other operators are a no-op, out of scope until Phase 4.3.

**After the operator:**
- The cursor goes to the selection start, and the mode goes to Normal (Insert for `c`).
- The `<` and `>` marks are updated.
- Visual `p` writes the old selection to `"`; Visual `P` does not.

- [ ] **Step 1: Write failing tests.**
  - Adapted from VSCodeVim `modeVisualLine.test.ts`:
    - `\|hello\nworld` + `ddVpP` → `\|world\nhello` ("Vp updates register content").
    - `begin\n\| middle\nend` + `yyjVp` → `begin\n middle\n \|middle` ("Vp places the cursor on first non-whitespace character on line").
  - Lucerna-only:
    - `vjd` charwise across lines.
    - `Vj>` indents two lines.
    - `veU` uppercases a word.
    - `vlr*` replaces two chars.
    - `VjJ` joins.
    - `vec` then typed text is one undo step; this is checked in Task 8.
    - `<C-v>jd` makes no edit.
- [ ] **Step 2:** Run `cargo test -p lucerna-vim visual`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p lucerna-vim`. Expected: PASS.
- [ ] **Step 5:** Commit with message `feat(vim): visual and visual-line operators`.

---

### Task 7: `.` repeat, typed-text recording, `u` / `⌃R`

Spec: §3.3 and §6.1.

**Files:** create `crates/lucerna-vim/src/repeat.rs`; modify `state.rs` and `lib.rs`.

**Interfaces (produced):**

```rust
pub enum Typed { Text(String), Newline, DeleteLeft, Tab }
// Completion is added in Phase 4.5; not now.
pub enum ReplayStep { Key(Key), Type(Typed) }
// V2b adds Search/Ex. The spec's operator-with-`/` replay is wired in Task 9.
pub enum Outcome { …, Replay(Vec<ReplayStep>) }
impl Vim {
    pub fn record_typed(&mut self, t: Typed);   // called by the workbench during Insert
}
```

**LastChange** (spec §6.1): register, count, the command's keys, the `Typed` list, and the visual shape.

**Recording:**
- Every command that changes text is recorded, including `x`, `p` and Insert entries.
- Yank, motions and undo are not recorded.
- Escape ends an Insert recording. Arrow-key moves in Insert (the workbench calls `record_typed` only for typing; a cursor move calls `Vim::break_insert_record()`) start a new record.

**`.` and `N.`:**
- `.` returns `Outcome::Replay(steps)` with the count replaced by `N` when given.
- `"1p` followed by `.` → `"2p`.
- A Visual change replays as Visual over the same shape from the cursor.

**`u` and `⌃R`:**
- `u` / `<C-r>` with count `n` emit `Action::Undo(n)` / `Action::Redo(n)`.
- After an undo, the workbench resets vim to the view's selection head (Task 8).

**Register `.`** = the text of the last Insert session's `Typed::Text` pieces joined.

- [ ] **Step 1: Write failing tests.**
  - Replay in tests: extend `run_edit` so `Outcome::Replay` feeds its steps back. Feed a `Key` step through `feed`. Apply a `Type` step with `lucerna_editor::commands::insert`, `newline`, `delete_left` or `tab` on the test snapshot, and call `record_typed` only when not replaying.
  - Adapted from VSCodeVim `normalModeTests/dot.test.ts` and `modeNormal.test.ts`:

    | Start | Keys | End | Note |
    |---|---|---|---|
    | `one \|two three four five` | `dw2.` | `one \|five` | `2.` replaces the count, so `d2w` |
    | `\|one\ntwo\nthree` | `A!<Esc>j.j.` | `one!\ntwo!\nthree\|!` | |
    | `\|\n` | `ivar<Esc>j4.` | `var\nvarvarvarva\|r` | `4.` repeats the insert 4 times, as Vim does: the count multiplies the insert |
    | `\|1 2 3` | `"1daw..` then `"1p..` | `1 2 \|3` | |
    | `on\|e\ntwo` | `Vj>.` | `\t\t\|one\n\t\ttwo` | with `insert_spaces=false` |

  - Adapted from VSCodeVim `undo.test.ts`:

    | Start | Keys | End |
    |---|---|---|
    | `one \|two three four five` | `dwdwu` | `one \|three four five` |
    | `one \|two three four five` | `dwdw2u<C-r>` | `one \|three four five` |
    | `one \|two three four five` | `vwwdu` | `one \|two three four five` |

  - Lucerna-only:
    - `ci(` + typed `foo(` + `bar` + Esc, then `.` on another `( )` → `foo(bar)`, through autoclose in `insert`.
    - `3ifoo<Esc>` → `foofoofo\|o` (the counted insert from Task 5).
- [ ] **Step 2:** Run `cargo test -p lucerna-vim repeat`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p lucerna-vim`. Expected: PASS.
- [ ] **Step 5:** Commit with message `feat(vim): dot repeat, typed-text recording, undo keys`.

---

### Task 8: Workbench wiring — one undo step per command, clipboard, typed events, replay, marks, settings

Spec: §3.2, §3.3, §3.4 and §9 (V2a keys).

**Files:**
- Modify:
  - `crates/lucerna-workbench/src/code_view/transact.rs` (`transact_group`; emit `Typed`);
  - `code_view/mod.rs` (`CodeViewEvent::Typed(lucerna_vim::Typed)`);
  - `code_view/input.rs` (IME commit emits `Typed::Text(final)` once);
  - `vim.rs`;
  - `crates/lucerna-workbench/Cargo.toml` (already depends on `lucerna-vim`);
  - `crates/lucerna-settings/assets/default-settings.json`.
- Create: `crates/lucerna-workbench/tests/vim_editing.rs`.

**Interfaces (produced):**

```rust
impl CodeView {
    /// Runs `f` as one undo step: a stop before, `suppress_stops` while it runs, a stop after
    /// unless `keep_open` (a command that ends in Insert; the Insert session's Escape closes it).
    pub fn transact_group<R>(&mut self, keep_open: bool, cx: &mut Context<Self>,
                             f: impl FnOnce(&mut Self, &mut Context<Self>) -> R) -> R;
}
pub enum CodeViewEvent { Reloaded { reload: Reload }, Typed(lucerna_vim::Typed) }
// Typed is emitted only while `suppress_stops` is set by vim's Insert session (a vim flag on the view).
```

**VimLayer:**
- **Before each `feed`:** set `globals.set_clipboard(...)` from `cx.read_from_clipboard()`.
  - Text, plus `whole_line` from the Phase 4 metadata. Parse it with the same `ClipboardMetadata` serde struct as `transact.rs`; make that struct `pub(crate)`.
  - Read only when the key can touch `"`: any key in operator-pending state, or `p P x X d c s S C D y Y "`. Otherwise pass `None`.
- **Applying one feed's actions:** all `Action::Edit` from one feed go inside one `transact_group(keep_open = mode after feed is Insert)`.
  - Each plan is applied with `CodeView::transact`.
  - After the group, set the vim cursor from vim, as before.
- **`Undo(n)` / `Redo(n)`:** call `CodeView::undo(true/false)` `n` times, then `vim.reset(view.selection_head())`, then `vim.clamp`, then `sync_view`.
- **`WriteClipboard { text, linewise }`:** write a `ClipboardItem` with metadata `{ pieces: None, wholeLine: linewise }`.
- **`Outcome::Replay(steps)`:**
  - Run each step in order inside one `transact_group`.
  - A `Key` step goes to `feed`, with `may_retry = false` and searches computed synchronously regardless of size.
  - A `Type` step goes to `CodeView::type_text` / `delete_left` / `edit(commands::tab)` / `type_text("\n")`, with the view in Insert.
  - Stop at the first failed step: a step whose `Outcome` is `Handled(vec![])` after a motion key counts as failed for V2b's macros. For `.`, run all steps.
- **`CodeViewEvent::Typed(t)`:** call `vim.record_typed(t)` for that view's state.
- **A cursor move during Insert:** call `vim.break_insert_record()` (Task 7).
- **Marks:** subscribe to `Documents`' `DocumentEvent::Edited { path, patch, .. }`. Remap that path's marks with `patch.map_offset(offset, Bias::Left)`. Remap every vim cursor and anchor of other views of the same path. Add `VimGlobals::remap_marks_patch(path, &Patch, &BufferSnapshot)` in `lucerna-vim` (a `marks.rs` helper).
- **Settings:** read the four V2a keys. `vim.useSystemClipboard` accepts `true`/`false`/`"onYank"`; anything else → `OnYank`.

- [ ] **Step 1: Write failing tests in `tests/vim_editing.rs`.** Copy the helpers from `tests/vim.rs`.
  - `three_dd_is_one_undo_step`: `3dd` then `u` restores all three lines; one more `u` does nothing.
  - `ciw_typing_escape_is_one_undo_step`: `ciw`, type `foo` through the input handler (`replace_text_in_range`), Escape, `u` → original.
  - `ime_commit_records_final_text_for_dot`: `ciw`, then `replace_and_mark_text_in_range("chà")`, then `replace_text_in_range("chào")`, then Esc. Then `.` on another word → `chào`.
  - `dot_after_external_reload_uses_new_text`: `dw`; reload the file with a line inserted above; `.` deletes the right word, with no panic.
  - `edit_in_one_view_remaps_other_view_and_marks`: two groups on one file; `ma` in B at row 5; `dd` in A at row 1 → B's mark `a` is row 4 and B's cursor shifts; `u` in B restores.
  - `clipboard_rules_end_to_end`:
    - `yy` writes a `ClipboardItem` with `wholeLine: true`.
    - `dd` leaves the clipboard unchanged.
    - Writing `"zzz"` to the test clipboard, then `p`, pastes `zzz` charwise.
    - With `vim.useSystemClipboard: false`, `yy` does not touch the clipboard.
  - `cmd_c_line_then_p_is_linewise`: with an empty selection, `editor.action.clipboardCopyAction`, then `p` → the line pasted below.
  - `readonly_preview_dd_does_nothing`.
  - `u_moves_cursor_to_change_start`.
- [ ] **Step 2:** Run `scripts/prune-target.sh && cargo test -p lucerna-workbench --test vim_editing`. Expected: FAIL.
- [ ] **Step 3:** Implement. Add the four keys to `default-settings.json` (`"vim.useSystemClipboard": "onYank"`, `"vim.joinspaces": true`, `"vim.changeWordIncludesWhitespace": false`, `"vim.tildeop": false`).
- [ ] **Step 4:** Run `cargo test -p lucerna-workbench`. Expected: PASS, including the existing `tests/vim.rs`.
- [ ] **Step 5:** Commit with message `feat(vim): workbench wiring for vim editing`.

---

### Task 9: Operators with `/` and `?`

Spec: §4.2 (last paragraph) and §6.1 (search records).

**Files:** modify `crates/lucerna-vim/src/state.rs`, `repeat.rs`, `crates/lucerna-workbench/src/vim.rs`.

**Pending operator + `/`:**
- `d/` or `c?` with an operator pending emits `OpenSearch` and keeps the pending operator in `Vim`.
- `search_submitted` completes the operator with the match as an exclusive motion. Esc (`Dismissed`) cancels both.

**Recording:**
- `ReplayStep::Search { pattern: String, backward: bool }` is added.
- `LastChange` stores it, so `.` reruns the search directly, synchronously and with no widget.

- [ ] **Step 1: Write failing tests.**
  - In `lucerna-vim`:
    - `d/foo` + `search_submitted("foo")` on `\|abc foo` → `\|foo`.
    - Then `.` on `x\|yz foo` → `x\|foo`.
    - Esc during the search leaves the text unchanged.
  - In workbench `tests/vim_editing.rs`:
    - `d_slash_through_find_widget`: type `d/foo`, Enter → deleted, one undo step.
    - `c_slash_then_escape_cancels`.
- [ ] **Step 2:** Run `cargo test -p lucerna-vim` and `cargo test -p lucerna-workbench --test vim_editing`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the same commands. Expected: PASS.
- [ ] **Step 5:** Commit with message `feat(vim): operators with search motions`.

---

### Task 10: Docs, notices, full checks, install

Spec: §12, last paragraph.

**Files:**
- `README.md`:
  - In the Vim section, add the V2a keys table and the four settings.
  - Note the `"onYank"` default and how it differs from VSCodeVim.
  - Note that `.` replays typing through auto-close.
- `THIRD_PARTY_NOTICES.md`: add the new VSCodeVim test files used to the existing "VSCodeVim test cases" list (grep `Adapted from VSCodeVim`).
- `docs/superpowers/specs/2026-11-08-lucerna-phase1-design.md` §1.3, V2 row: mark "V2a done".
- `docs/superpowers/specs/2026-11-13-vim-editing-design.md`: set the status line to `V2a implemented`.
- `docs/superpowers/specs/2026-11-11-lucerna-phase4-editing-design.md` §13: note that `transact_group` now exists (signature in Task 8).

Steps:
- [ ] **Step 1:** Edit the docs.
- [ ] **Step 2:** Run each check and confirm its expected output:

  | Command | Expected |
  |---|---|
  | `scripts/prune-target.sh` | — |
  | `cargo fmt --all -- --check` | no output |
  | `cargo clippy --workspace --all-targets -- -D warnings` | clean |
  | `cargo test --workspace` | all pass |
  | `cargo deny check` | `advisories ok, bans ok, licenses ok, sources ok` |

- [ ] **Step 3:** Commit with message `docs: vim editing V2a` (explicit paths, after `git status`).
- [ ] **Step 4:** Check `pgrep -x cargo` shows no other cargo, then run `scripts/bundle-app.sh --install`. Expected: it ends with `bundle-app: installed /Applications/Lucerna.app`.
