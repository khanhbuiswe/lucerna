# Vim V2b (macros and the `:` command line) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish Vim editing in Lucerna:
- macros `q{reg}` … `q`, `[count]@{reg}`, `@@`, stored in a register as key notation;
- the `:` command line through Quick Open (`:w :q :wq :x :wa :qa :q! :w! :N :$ :d :y :> :< :s :noh`, ex-style ranges);
- `vim.gdefault`;
- highlight on yank (`vim.highlightedyank.enable`, `vim.highlightedyank.duration`).

**Architecture:**
- `lucerna-vim` (pure, no gpui) gains a parser and runner for `:` commands (`ex.rs`, `ex_run.rs`, `substitute.rs`) and macros (`macros.rs`). Commands that need the workbench (save, close a tab) come out as `Action::RunEx(ExCommand)`.
- Running a macro is **replaying keys**: `Outcome::Macro(Vec<Key>)`. The workbench feeds each key into `feed` (in Insert, it types through the typing path), and the whole run is one undo group. While a macro runs, `lucerna-vim` itself collects the text after `/`, `?`, `:` up to `<CR>` (without opening any UI), because only at run time is it known which key belongs to which mode.
- Quick Open has `QuickOpenQuery::VimCommand` when Vim is on and the active view is a code view. The command's result shows in the mode indicator until the next key.

**Tech Stack:** Rust 2024, `lucerna-text`, `lucerna-editor`, `lucerna-vim` (+ `regex`, already in the workspace), gpui (only the workbench).

**Spec:** `docs/superpowers/specs/2026-11-13-vim-editing-design.md`, step **V2b** in §12: §6.4 macros, §8 command line, `vim.gdefault`, highlight on yank (§9). Also read §2 (it has a Helix section), §3.1, §10, §11. The V2a plan: `docs/superpowers/plans/2026-11-13-vim-editing-v2a.md` (same test conventions).

## Global Constraints

- **Text in the app is English only**, exactly as in the spec: `E492: Not an editor command: foo`, `E486: Pattern not found: foo`, `N substitutions on M lines`, `-- NORMAL -- recording @a`, `E: file changed, :s cancelled`, `:%s/foo/bar/g — Replace in all lines`, `:w — Save`, `:12 — Go to line 12`, `Not an editor command: foo`. Vietnamese appears only in test data.
- **Crates:** only `lucerna-workbench` uses gpui. `lucerna-vim` may depend on `lucerna-text`, `lucerna-editor`, `unicode-segmentation` and adds `regex` (workspace, MIT/Apache).
- **License:** do not copy Zed code (GPL) or Helix code (MPL), only learn the design. Test cases adapted from VSCodeVim (MIT) carry `// Adapted from VSCodeVim <path>: "<test title>"`.
- **Positions:** `Point.column` and offsets are UTF-8 bytes; never split a character; Normal does not sit after the last character.
- **Editing:** every text change is an `EditPlan` (edits by offset before the change). A read-only view (`edit_ctx() == None`) does not accept `Action::Edit`; `:y` and `y` still run.
- **Undo:** one `feed` (or one `:` command) is one step; a whole macro run (including `5@a`) is one step.
- **New settings (spec §9):**

  | Key | Default |
  |---|---|
  | `vim.gdefault` | `false` |
  | `vim.highlightedyank.enable` | `false` |
  | `vim.highlightedyank.duration` | `200` (ms) |

- **Build:**
  - Cargo needs `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`.
  - Run `scripts/prune-target.sh` before **every** `cargo test` or build.
  - Never set `CARGO_TARGET_DIR` or `--target-dir`; every worktree builds into the shared `target/`. If the cargo lock is held, wait.
  - Keep one profile: delete `target/release` before a test/debug build, unless `pgrep -x cargo` shows another cargo running.
- **Git (shared folder):**
  - Run `git status` before every commit; if a merge is in progress or there are staged files that are not yours, stop and report BLOCKED.
  - Stage and commit by explicit path: `git commit -m "…" -- <paths>`. No rebase, no stash, no amend.
  - Trailer: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- `cargo deny check` must pass.

## Review Focus

1. **A macro with Telex-typed text and auto-close.** Record `qa` + `ciw` + IME commit `chào` + `Esc` + `q`, then `@a` on another word must produce `chào`, in one undo step. Test: `Task 7: macro_with_vietnamese_ime_text_replays_text`.
2. **A recursive macro or a large count does not hang or overflow the stack.** Register `a` containing `@a`, then `@a`: stops at the nesting limit, no panic, changes nothing. Test: `Task 7: self_calling_macro_stops_at_depth_cap`.
3. **`:%s` on a file over 1 MB while an agent edits the file.** The revision changes before the result returns: nothing is applied, and the status bar reports `E: file changed, :s cancelled`. Test: `Task 6: large_file_substitute_cancelled_by_reload`.
4. **`:wq` when the file conflicts with disk.** The conflict prompt shows; choosing Cancel leaves the tab **not** closed and still dirty. `:w!` overwrites without asking. Tests: `Task 6: wq_conflict_cancel_keeps_tab`, `Task 6: w_bang_overwrites_without_prompt`.
5. **`:` with Vim off and with Vim on.** Vim off: `:-1` is still the last line, `:12,5` is line 12 column 5 (T5-4). Vim on: `:-1` goes up one line, `:12,5` is a reversed range; with no code view, `:12` is still the old go-to-line. Test: `Task 6: colon_dispatch_vim_on_and_off`.

---

## Decisions fixed in the plan (the spec is not explicit)

- **How Quick Open splits the work.** When Vim is on **and** the active editor is a code view that receives Vim's keys, all text after `:` (leading spaces trimmed) is `QuickOpenQuery::VimCommand`. V2b's ex parser understands a lone number (`12`) as go to that line, so `:12` still gives `:12 — Go to line 12` as in spec §8.1, only the execution path differs. `resolve_line` and the VS Code meaning of `:-1`, `:12,5` are used only when Vim is off or there is no code view. Like Helix: a lone number is `:goto`.
- **`:N` when Vim is on** runs like `NG`: jump (set the `''` mark), the cursor goes to the first non-blank character per `vim.startofline`, and a line past the end is clamped to the last line.
- **`:s` matches line by line** (the line's text, without a newline character): `^`/`$` are the start/end of the line; an empty match is allowed (`:s/^/# /`); the pattern does not cross lines. Case-sensitive by default; `i` makes it insensitive, `I` makes it sensitive (it does not read `vim.ignorecase`, because that setting is only for `*` and `#`).
- **A range on `:w :q :wq :x :wa :qa :noh`** is ignored. `!` is only allowed on `w q wq x`; elsewhere: `E477: No ! allowed`. `:wq!` and `:x!` save overwriting, then close.
- **Other error messages** (following Vim): `E16: Invalid range`, `E20: Mark not set`, `E35: No previous regular expression`, `E383: Invalid search string: {pat}`, `E488: Trailing characters: {rest}`, `E354: Invalid register name: '{c}'`.
- **A failed motion** is reported with `Action::Failed` in `Outcome::Handled` (spec §3.1 "every `Outcome` tells whether the motion failed"). The workbench uses it only to stop a macro.
- **While a macro runs**, deleting multiple lines still writes `"` and `1` but **does not shift** `1`–`9`, and `.` and the `.` register are not overwritten (spec §6.4, read literally).
- **Macro nesting limit:** `MAX_MACRO_DEPTH = 100` (Vim allows recursive macros, Helix blocks them outright). Past it, the whole run stops, with no error.
- **Highlight color on yank:** `theme.ui.search_match_background` (the Zed key `search.match_background`), equivalent to the `editor.findMatchHighlightBackground` that spec §9 mentions.

---

## File Structure

```
crates/lucerna-vim/
  Cargo.toml          + regex.workspace = true
  src/lib.rs          Action::{Failed, RunEx, OpenCommandLine, Message, Yanked}; Outcome::{Macro, Substitute};
                      ExCommand; Settings.{gdefault, highlighted_yank}; re-exports
  src/ex.rs           NEW  parse `:` commands, ranges, abbreviations, describe_command, ExError
  src/ex_run.rs       NEW  Vim::ex_command: go to line, :d :y :> :<, workbench commands, `:` register
  src/substitute.rs   NEW  :s — pattern translation, replacement, flags, SubstituteJob
  src/macros.rs       NEW  key notation, macro recording, `@`, internal command line while running a macro
  src/register.rs     set_last_command, set_macro, hold_ring
  src/state.rs        keys `:` `q` `@`; Action::Failed; macro recording hook; no `.` write while running a macro
  src/test_util.rs    parse_keys → macros::parse_notation; simulate Quick Open `:`; Outcome::Macro/Substitute
crates/lucerna-workbench/src/
  quick_open/query.rs   QuickOpenQuery::VimCommand, parse_query_vim
  quick_open/view.rs    Sources.vim_command_line, Row::VimCommand, QuickOpenEvent::VimCommand
  quick_open/mod.rs     show_quick_open fills vim_command_line; event → run_vim_command
  vim.rs                Redo::Ex; run ex; message; ExCommand queue; macro loop; yank flash; new settings
  vim_ex.rs             NEW  Workbench::{run_vim_command, run_vim_requests, run_ex_command}
  vim_indicator.rs      draws Indicator (recording, message)
  workbench.rs          calls run_vim_requests after dispatching a key
  save.rs, documents.rs SaveOptions.overwrite
  editor_groups/mod.rs, editor_groups/view.rs   workbench.action.closeAllEditors, request_close_all
  code_view/mod.rs, code_view/element.rs        flash_ranges
crates/lucerna-settings/assets/default-settings.json   three new keys
crates/lucerna-workbench/tests/vim_command_line.rs     NEW
crates/lucerna-workbench/tests/vim_editing.rs          + macro, yank highlight
crates/lucerna-workbench/tests/quick_open_query.rs     + parse_query_vim
```

---

### Task 1: The `:` command parser (`ex.rs`)

Spec: §8.2 (command table, abbreviations, ranges), §8.1 (the description line).

**Files:**
- Create: `crates/lucerna-vim/src/ex.rs`
- Modify: `crates/lucerna-vim/src/lib.rs` (`mod ex; pub use ex::describe_command;`), `crates/lucerna-vim/Cargo.toml` (`regex.workspace = true`)

**Interfaces (produced):**

```rust
pub(crate) enum Base { Number(u32 /*1-based*/), Current, Last, Mark(char) }
pub(crate) struct Address { pub base: Base, pub offset: i64 }
pub(crate) enum LineRange { Implicit, All, One(Address), Two(Address, Address) }
pub(crate) struct SubFlags { pub global: bool, pub ignore_case: Option<bool>, pub report_only: bool }
pub(crate) enum SubstituteArgs {
    /// `:s`, `:s g`, `:s 3`: repeat the last substitute with these flags.
    Repeat { flags: SubFlags, count: Option<u32> },
    New { pattern: String, replacement: String, flags: SubFlags, count: Option<u32> },
}
pub(crate) enum Command {
    Goto,                                       // a range alone
    Write { bang: bool }, WriteAll, Quit { bang: bool }, QuitAll,
    WriteQuit { bang: bool }, Exit { bang: bool },   // :wq, :x
    Delete { register: Option<char>, count: Option<u32> },
    Yank { register: Option<char>, count: Option<u32> },
    Shift { right: bool, depth: u32, count: Option<u32> },
    Substitute(SubstituteArgs),
    NoHighlight,
}
pub(crate) struct ExLine { pub range: LineRange, pub command: Command }
pub(crate) enum ExError { NotACommand(String), InvalidRange, MarkNotSet, NoBangAllowed,
    TrailingCharacters(String), PatternNotFound(String), InvalidPattern(String), NoPreviousPattern,
    InvalidRegister(char) }
impl ExError {
    pub(crate) fn message(&self) -> String;   // "E492: Not an editor command: foo"
    pub(crate) fn short(&self) -> String;     // "Not an editor command: foo" (no "Exxx: ")
}
pub(crate) fn parse(line: &str) -> Result<ExLine, ExError>;
/// 0-based inclusive rows. `clamp`: out-of-range rows clamp (Goto) instead of E16.
pub(crate) fn resolve(range: &LineRange, cursor_row: u32, last_row: u32,
                      marks: Option<&FileMarks>, clamp: bool) -> Result<RangeInclusive<u32>, ExError>;
/// The Quick Open row text after `:{line} — `, or the error row text.
pub fn describe_command(line: &str) -> Result<String, String>;
```

Parsing rules:
- **Address:** `N`, `.`, `$`, `'x` (`a`–`z`, `<`, `>`), then the offsets `+N`/`-N`; a `+`/`-` with no number is 1; `.++1` = `.+2`; an offset with no base has base `.`. Range: `%`, `a,b`, `a`. A reversed range is swapped (no question asked).
- **The command name** is a run of ASCII letters right after the range (or a run of `>`/`<`), then an optional `!`. The abbreviation table `[abbreviation, full]`: `w write`, `wa wall`, `q quit`, `qa qall`, `wq wq`, `x xit`, `d delete`, `y yank`, `s substitute`, `noh nohlsearch`. A name `n` matches when `full.starts_with(n) && n.len() >= abbr.len()`.
- **`:d`/`:y`:** whitespace, an optional register (not a digit), whitespace, an optional count (`:y3`, `:ya3`, `:d x 2`). Anything left over gives `E488`.
- **`:s`:** the delimiter is the first character after `s` that is not a letter/digit/whitespace/`\`/`"`/`|`. `\` + the delimiter is the delimiter itself. The final delimiter may be omitted. After that come the flags `g i I n`, whitespace, a count. An unknown flag gives `E488`.
- An empty line (only whitespace) parses to `Goto` with `Implicit`, and `ex_command` treats it as doing nothing.

`describe_command` (exact text):

| Line | Result |
|---|---|
| `12` | `Ok("Go to line 12")` |
| `$` | `Ok("Go to last line")` |
| another range with no command | `Ok("Go to line")` |
| `w`, `w!`, `wa` | `Save`, `Save, overwriting the file on disk`, `Save all` |
| `q`, `q!`, `qa` | `Close editor`, `Revert and close editor`, `Close all editors` |
| `wq`, `x` | `Save and close editor`, `Save if modified and close editor` |
| `d`, `y`, `>`, `<` | `Delete` / `Yank` / `Indent` / `Outdent` + ` current line` / ` all lines` (`%`) / ` selected lines` (`'<,'>`) / ` lines in range` |
| `s…` | `Replace in current line` / `Replace in all lines` / `Replace in selection` / `Replace in range` |
| `noh` | `Clear search highlight` |
| `foo` | `Err("Not an editor command: foo")` |

- [ ] **Step 1: Write the failing tests** in `ex.rs`:
  - `parse("%s/foo/bar/g")` → `All` + `Substitute(New{pattern:"foo", replacement:"bar", flags.global:true})`.
  - `parse("'<,'>d")`, `parse(".+3yan2")` → `Yank{count:Some(2)}` with `One(Address{Current, 3})`; `parse(".++1s/a/b/")` → offset 2; `parse("+2s/a/b/")` → base Current offset 2.
  - `parse("s/\\/\\/f/z/g")` → pattern `//f`. *Adapted from VSCodeVim `test/cmd_line/substitute.test.ts`: "Substitute with escaped delimiter"*.
  - `parse("s g")` → `Repeat{flags.global:true}`; `parse("s/a/b/ 3")` → count 3.
  - `parse("wq!")` → `WriteQuit{bang:true}`; `parse("wa!")` → `Err(NoBangAllowed)`; `parse("no")` → `Err(NotACommand("no"))`; `parse("d x 2 z")` → `Err(TrailingCharacters("z"))`.
  - `resolve`: `3,1` → `0..=2`; `'a` not set → `MarkNotSet`; `20` on a 5-line file → `InvalidRange`, and with `clamp` → `4..=4`.
  - `ExError::NotACommand("foo").message() == "E492: Not an editor command: foo"`, `PatternNotFound("foo").message() == "E486: Pattern not found: foo"`.
  - The `describe_command` table above, line by line.
- [ ] **Step 2:** `scripts/prune-target.sh && cargo test -p lucerna-vim ex::` → FAIL (the module does not exist yet).
- [ ] **Step 3:** Implement `ex.rs`.
- [ ] **Step 4:** `cargo test -p lucerna-vim` → PASS.
- [ ] **Step 5:** `git commit -m "feat(vim): ex command line parser" -- crates/lucerna-vim/Cargo.toml crates/lucerna-vim/src/ex.rs crates/lucerna-vim/src/lib.rs Cargo.lock`

---

### Task 2: Run `:` commands in `lucerna-vim` (except `:s`)

Spec: §3.1 (the new `Action`s), §7 (`:` from Visual), §8.1, §8.2, §6.2 (the `:` register).

**Files:**
- Create: `crates/lucerna-vim/src/ex_run.rs`
- Modify: `src/lib.rs`, `src/state.rs` (the `:` key), `src/register.rs`, `src/test_util.rs`, `crates/lucerna-workbench/src/vim.rs` (make the `Action` match compile: the new variants are no-ops until Task 6).

**Interfaces:**
- Consumes: Task 1 `parse`, `resolve`, `ExError`; V2a `edit::apply_operator(op, OpRange, ctx, view, g, s, register) -> (Vec<Action>, Point, Mode)`.
- Produces:

```rust
/// What the workbench carries out for a `:` command.
pub enum ExCommand {
    Save { overwrite: bool },                          // :w, :w!
    SaveAll,                                           // :wa
    Close { revert: bool },                            // :q, :q!
    CloseAll,                                          // :qa
    SaveAndClose { only_if_dirty: bool, overwrite: bool }, // :wq, :x (and with !)
    NoHighlight,                                       // :noh
}
pub enum Action { …, Failed, RunEx(ExCommand), OpenCommandLine { prefill: String }, Message(String) }
impl Vim {
    /// Runs one command line (the text after `:`). Ends a Visual mode first (setting `'<` `'>`),
    /// drops pending keys and count. Errors return `Handled(vec![Message(err.message()), Failed])`.
    pub fn ex_command(&mut self, line: &str, view: &dyn ViewInfo, g: &mut VimGlobals, s: &Settings) -> Outcome;
}
impl Registers { pub(crate) fn set_last_command(&mut self, line: String); } // read(Some(':')) returns it, Charwise
```

Behavior:
- **The `:` key** in Normal: `OpenCommandLine { prefill: "" }`; with count `n`: `"."` when `n == 1`, `".,.+{n-1}"` when `n > 1`. In Visual/Visual Line: end Visual (marks `<` `>`), go back to Normal, `prefill: "'<,'>"`. Store `prefill` in `Vim.last_prefill: Option<String>` (Task 4 uses it).
- **`Goto`:** like `{N}G` (jump, mark `''`, `startofline`), the line clamped.
- **`:d`:** writes the register like `dd` (`WriteKind::Delete`, Linewise), the cursor on the first non-blank character of the line that takes the place of the deleted line. **`:y`:** like `yy`, the cursor unchanged. **`:>`/`:<`:** `depth` levels, the cursor on the first non-blank character of the last line of the range. Count: the new range starts at the last line of the range and is `count` lines long, clamped at the end of the file.
- **Workbench commands** return `Handled(vec![RunEx(…)])`; the range is ignored.
- **The `:` register** is written when the command finishes (even with an error), except for an empty line.

Test harness: when `run_edit` meets `Action::OpenCommandLine { prefill }`, the following keys up to `<CR>` are typed into a fake command line that starts with `prefill` (`<BS>` deletes, `<Esc>` cancels); `<CR>` calls `vim.ex_command(line, …)` and applies the outcome, like the existing `/` simulation.

- [ ] **Step 1: Write the failing tests** (`assert_edit`):

  *Adapted from VSCodeVim `test/cmd_line/delete.test.ts`:*

  | Start | Keys | End | Title |
  |---|---|---|---|
  | `line 1\nli\|ne 2\nline 3` | `:d<CR>` | `line 1\n\|line 3` | ":d" |
  | `line 1\nli\|ne 2\nline 3` | `:2,3d<CR>` | `\|line 1` | ":2,3d" |
  | `line 1\nli\|ne 2\nline 3` | `:d 2<CR>` | `\|line 1` | ":d 2" |

  *Adapted from VSCodeVim `test/cmd_line/yank.test.ts`:*

  | Start | Keys | End | Title |
  |---|---|---|---|
  | `\|one\ntwo\nthree` | `:yank<CR>p` | `one\n\|one\ntwo\nthree` | ":yank will yank a single line" |
  | `\|one\ntwo\nthree\nfour\nfive` | `:ya3<CR>p` | `one\n\|one\ntwo\nthree\ntwo\nthree\nfour\nfive` | ":yank [x] [cnt] will yank [cnt] lines" |
  | `\|one\ntwo\nthree\nfour\nfive` | `:.+3yan<CR>p` | `one\n\|four\ntwo\nthree\nfour\nfive` | ":[range]yank will yank from the end of the range, if range a line number" |

  Lucerna's own cases:
  - `:5<CR>` on a 10-line file with indentation → the cursor on the first non-blank character of line 5; then `''` goes back.
  - `:999<CR>` → the last line; `:$<CR>` → the last line.
  - `Vj:` emits `OpenCommandLine{prefill:"'<,'>"}` and Normal mode; `3:` emits `prefill: ".,.+2"`.
  - `Vj:>` + `<CR>` indents two lines; `:'a,'b>` when mark `a` is not yet set → actions `[Message("E20: Mark not set"), Failed]`, text unchanged.
  - `:foo<CR>` → `[Message("E492: Not an editor command: foo"), Failed]`.
  - `:w<CR>` → `[RunEx(Save{overwrite:false})]`; `:x!<CR>` → `RunEx(SaveAndClose{only_if_dirty:true, overwrite:true})`; `:noh<CR>` → `RunEx(NoHighlight)`.
  - `:d<CR>` in a read-only view → no `Edit`.
  - After `:d 2<CR>`, `":p` pastes `d 2`.
- [ ] **Step 2:** `cargo test -p lucerna-vim ex_run` → FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** `cargo test -p lucerna-vim` and `cargo test -p lucerna-workbench --test vim --test vim_editing` → PASS.
- [ ] **Step 5:** `git commit -m "feat(vim): run ex commands: go to line, :d :y :> :<, workbench commands" -- <the files above>`

---

### Task 3: `:s` and `vim.gdefault`

Spec: §8.2 item `:s`, §9 (`vim.gdefault`), §10 (`:%s` on a large file).

**Files:**
- Create: `crates/lucerna-vim/src/substitute.rs`
- Modify: `src/lib.rs` (`Settings.gdefault: bool`, default `false`; `Outcome::Substitute`), `src/ex_run.rs`, `src/state.rs` (`VimGlobals.last_substitute`), `src/test_util.rs`, `crates/lucerna-workbench/src/vim.rs` (temporarily run `Outcome::Substitute` synchronously: `job.run` then `finish_substitute`; Task 6 adds the background thread).

**Interfaces (produced):**

```rust
/// Vim's escapes in a Rust-regex pattern: `\(` `\)` `\{` `\}` `\+` `\?` `\|` lose the backslash;
/// `\<` `\>` stay (regex 1.13 reads them as word starts and ends).
pub(crate) fn translate_pattern(vim: &str) -> String;
/// `&` `\0` match, `\1`–`\9` groups, `\n` newline, `\t` tab, `\\` `\&` literal, `~` = `previous`,
/// `\u \U \l \L \e \E` case.
pub(crate) fn expand_replacement(template: &str, caps: &regex::Captures, previous: &str) -> String;
/// A substitute ready to run on a snapshot; Send, so the workbench may run it in the background.
#[derive(Debug, Clone)] pub struct SubstituteJob { /* regex, template, rows, global, report_only, pattern text */ }
#[derive(Debug, Clone)] pub struct SubstituteResult { /* edits, substitutions, lines, last_row */ }
impl SubstituteJob { pub fn run(&self, snapshot: &BufferSnapshot) -> SubstituteResult; }
pub const BACKGROUND_SUBSTITUTE_BYTES: usize = 1024 * 1024;
pub enum Outcome { …, Substitute(SubstituteJob) }
impl Vim {
    /// Applies a job's result computed for the view's current text.
    pub fn finish_substitute(&mut self, job: &SubstituteJob, result: SubstituteResult,
                             view: &dyn ViewInfo, g: &mut VimGlobals, s: &Settings) -> Outcome;
}
```

Rules:
- **An empty pattern** uses the last search pattern (`g.last_search`): `is_regex == false` gets `regex::escape`, `whole_word` wraps it in `\b…\b`. If there is none, `E35`. **A repeated `:s`** (`Repeat`) uses the pattern and replacement of `g.last_substitute` with the new flags; if there is none, `E35`.
- **Flags:** `global = flags.global ^ s.gdefault`. Case sensitivity per "Decisions fixed in the plan".
- **A pattern that does not compile** → `E383: Invalid search string: {pat}`.
- **Match line by line** within the range (the current line by default); without `g`, the first match on each line; an empty match is allowed, and after an empty match, searching continues from the next character.
- **`ex_command`** runs synchronously if `snapshot.len_bytes() <= BACKGROUND_SUBSTITUTE_BYTES` or `g.macro_running()` (Task 4; until then treat it as `false`); otherwise it returns `Outcome::Substitute(job)`, without changing state.
- **After applying** (`finish_substitute`, also used by the synchronous path): one `Edit` built with `lucerna_editor::edit::replace_ranges(ctx, edits, cursor, EditKind::Other)`; the cursor at column 0 of the line that contains the end of the last replacement (after the edit); `g.last_search = (SearchQuery{pattern: translated, is_regex: true, case_sensitive, whole_word: false}, false)` and `Action::Highlight` like `/`; `g.last_substitute` is updated; `Message("{n} substitution{s} on {m} line{s}")` (singular when equal to 1). The `n` flag: no edit, `Message("{n} match{es} on {m} line{s}")`. No match: `[Message("E486: Pattern not found: {pat}"), Failed]`, where `{pat}` is the pattern as the user typed it (or the last search pattern when empty).
- A read-only view: no `Edit`, the count is still reported (like the `n` flag).

- [ ] **Step 1: Write the failing tests.**
  - `translate_pattern("\\(a\\|b\\)\\+")` → `"(a|b)+"`; `translate_pattern("\\<foo\\>")` → `"\\<foo\\>"`.
  - *Adapted from VSCodeVim `test/cmd_line/substitute.test.ts`* (start, keys, end, title):

    | Start | Keys | End | Title |
    |---|---|---|---|
    | `\|aba` | `:%s/a/d<CR>` | `\|dba` | "Replace single word once" |
    | `\|aba` | `:%s/a/d/g<CR>` | `\|dbd` | "Replace with `g` flag" |
    | `\|blah blah\nblah\nblah blah\nblah blah` | `:.s/blah/yay/g 2<CR>` | `yay yay\n\|yay\nblah blah\nblah blah` | "Replace with flags AND count" |
    | `\|aba\nab` | `:%s/a/d/g<CR>` | `dbd\n\|db` | "Replace across all lines" |
    | `blah blah\nbla\|h\nblah blah\nblah blah` | `:.-1,.+1s/blah/yay<CR>` | `yay blah\nyay\n\|yay blah\nblah blah` | "Replace across relative line range using dot, plus, and minus" |
    | `blah blah\n\|blah\nblah blah\nblah blah` | `3:s/blah/yay<CR>` | `blah blah\nyay\nyay blah\n\|yay blah` | "Replace across relative line range using numLines+colon shorthand" |
    | `\|blah blah\nblah\nblah blah\nblah blah` | `:s/blah/yay<CR>j3:s<CR>` | `yay blah\nyay\nyay blah\n\|yay blah` | "Repeat replacement across relative line range" |
    | `one \|two three` | `:s/t/\\n/g<CR>` | `one \nwo \n\|hree` | "Replace with \\n" |
    | `one \|two three thirteen` | `:s/\\bt/x/g<CR>` | `\|one xwo xhree xhirteen` | "Preserve \\b in regular expression" |
    | `\|aba\nab` | `Vj:s/a/d/g<CR>` | `dbd\n\|db` | "Replace text in selection" |
    | `\|aba\naba\nabc` | `majmb:'a,'bs/a/d/g<CR>` | `dbd\n\|dbd\nabc` | "Substitute support marks" |
    | `apple\nban\|ana\ncelery\ndragonfruit` | `:%s/a//gn<CR>` | unchanged | "`n` flag (report count)"; message `5 matches on 3 lines` |
    | `\|foo\nbar\nfoo\nbar` | `*:%s//fighters/<CR>` | `fighters\nbar\n\|fighters\nbar` | "Substitute with previous search using *" |
    | `\|one two three\ntwo three four` | `:s/two/xyz/<CR>j:s/three/~ ~/<CR>` | `one xyz three\n\|two xyz xyz four` | "`~` in replace string uses previous replace string" |
    | `\|fooo` | `:s/o/un/<CR>:s g<CR>` | `\|fununun` | "Substitute repeat previous should accept flags" |
    | `\|she sells seashells by the seashore` | `:s/s\\S*/\\U&x/g<CR>` | `\|SHEX SELLSX SEASHELLSX by the SEASHOREX` | "\\U" |
    | `\|she sells seashells by the seashore` | `:s/s\\S*/\\u&x/g<CR>` | `\|Shex Sellsx Seashellsx by the Seashorex` | "\\u" |
    | `\|SHE SELLS SEASHELLS BY THE SEASHORE` | `:s/S\\S*/\\L&\\EX/g<CR>` | `\|sheX sellsX seashellsX BY THE seashoreX` | "\\L then \\E" |

    With `gdefault = true` (*same file, suite "Effects of gdefault=true"*): `\|aba` + `:%s/a/d<CR>` → `\|dbd` ("Replace all matches in the line"); `:%s/a/d/g<CR>` → `\|dba` ("Replace with `g` flag inverts global flag").
  - Lucerna's own cases:
    - `:%s/foo/bar/g` on `foo foo\nfoo` → message `3 substitutions on 2 lines`; a single `Edit`.
    - `:s/x/y/` with no match → `[Message("E486: Pattern not found: x"), Failed]`.
    - `:s/(/x/` → `E383: Invalid search string: (`, no panic.
    - `:%s/^/# /` adds `# ` to the start of every line (an empty match).
    - `:s/chào/\\U&/` on `xin chào` → `xin CHÀO` (Unicode).
    - After `:s/foo/bar/`, `n` finds the next `foo`.
    - `Outcome::Substitute` is returned when `TestView` has text over `BACKGROUND_SUBSTITUTE_BYTES`; `job.run(&snapshot)` then `finish_substitute` give the same result as the synchronous path.
- [ ] **Step 2:** `cargo test -p lucerna-vim substitute` → FAIL.
- [ ] **Step 3:** Implement. The test harness handles `Outcome::Substitute(job)` by `job.run` then `finish_substitute` right away.
- [ ] **Step 4:** `cargo test -p lucerna-vim` → PASS.
- [ ] **Step 5:** `git commit -m "feat(vim): :s substitute and vim.gdefault" -- <the files above>`

---

### Task 4: Macros in `lucerna-vim`

Spec: §6.4, §3.1 (a failed motion), §1.2 item 4.

**Files:**
- Create: `crates/lucerna-vim/src/macros.rs`
- Modify: `src/lib.rs`, `src/state.rs`, `src/register.rs`, `src/ex_run.rs`, `src/motion.rs`/`src/state.rs` (emit `Failed`), `src/test_util.rs`, `crates/lucerna-workbench/src/vim.rs` (the new `record_typed` call; `Outcome::Macro` temporarily runs like `Replay` of the `Key`s — Task 7 does it fully).

**Interfaces (produced):**

```rust
// macros.rs
/// Vim key notation: plain chars, `<Esc>` `<CR>` (also `<Enter>`, `<Return>`) `<BS>` `<Tab>` `<lt>` `<C-x>`,
/// names case-insensitive; a `\n` is `<CR>`, a `\t` is `<Tab>`; an unknown `<…>` is literal text.
pub fn parse_notation(text: &str) -> Vec<Key>;
pub(crate) fn push_key(out: &mut String, key: Key);      // Char('<') → "<lt>", Char('\t') → "<Tab>"…
pub(crate) fn push_typed(out: &mut String, t: &Typed);   // Text escaped, Newline "<CR>", DeleteLeft "<BS>", Tab "<Tab>"
impl VimGlobals {
    pub fn recording(&self) -> Option<char>;              // the register as typed (`a` or `A`)
    pub fn set_macro_running(&mut self, running: bool);   // also sets Registers.hold_ring
    pub fn macro_running(&self) -> bool;
}
impl Vim {
    pub fn record_typed(&mut self, t: Typed, g: &mut VimGlobals);   // signature change (was (t))
    /// The workbench calls this when a macro run ends: an unfinished command line is dropped.
    pub fn macro_ended(&mut self);
}
pub enum Outcome { …, Macro(Vec<Key>) }
impl Registers {
    pub(crate) fn set_macro(&mut self, name: char, text: String); // a–z replace, A–Z append (creates), Charwise
}
```

`Outcome::Macro(keys)`: "run the keys in order, the whole run being one undo group, with `VimGlobals::set_macro_running(true)` during it. A key that arrives while Vim is in Insert (except `Escape`) is typed: `Char('\t')` → `Typed::Tab`, `Char(c)` → `Typed::Text(c)`, `Enter` → `Newline`, `Backspace` → `DeleteLeft`, `Ctrl(_)` is skipped. Stop the whole run when a key returns `Action::Failed`. A nested `Macro` (meeting `@` inside) runs in place."

**Recording:**
- `q{a-zA-Z}` in Normal (nothing pending, no macro running) starts recording; `q` in Normal or Visual while recording stops and calls `set_macro`. `q` with another name cancels. `q` while a macro is running does nothing.
- In `Vim::feed`, while recording and not running a macro: the key is appended with `push_key`, unless the outcome is `Unhandled`, or has `OpenSearch`/`OpenCommandLine` (`/`, `?`, `:` are recorded when they finish running), or it is the `q` key that stops recording.
- `record_typed` appends `push_typed` while recording and not running a macro (still recording for `.` as in V2a).
- `search_submitted` appends `/` (or `?`) + the pattern (with `<` escaped) + `<CR>`. A Find widget closed with Esc records nothing.
- `ex_command` appends `:` + the part of the line after `last_prefill` + `<CR>` if the line starts with `last_prefill.take()`; otherwise (or when opened with ⌃G) it is `:<C-u>` + the line + `<CR>`.
- Order: the key `"` + a register name before `q` is not part of `q` (kept as in V2a).

**Running:**
- `[count]@{r}`: `r` is in `a-z A-Z 0-9 " - + * .` (an uppercase letter reads the lowercase register). Read with `VimGlobals::read_register`, `parse_notation`, repeat `count` times → `Outcome::Macro`. `g.last_macro = r`. `@@` runs `last_macro`.
- An empty register, or `@@` with none yet → `Handled(vec![Failed])`. Another name → `[Message("E354: Invalid register name: '{c}'"), Failed]`.
- **The internal command line while running a macro:** `/`, `?`, `:` do not emit `OpenSearch`/`OpenCommandLine` but open `Vim.cmdline: Option<CmdLine>` (`CmdKind::Search{backward}` or `Ex`, the text starting with the prefill). Then `Char` adds text, `Backspace` deletes (cancels when empty), `Ctrl('u')` deletes everything, `Escape` cancels (`search_dismissed`), `Enter` runs: a search with `search_submitted` with a query taking the flags of `g.last_search` (if none, `is_regex/case_sensitive/whole_word = false`), from the cursor; a command with `ex_command`. `Retry` keeps `cmdline` as is (because `feed` restores state), so the workbench feeding `Enter` again is enough.
- **While running a macro:** `record`/`save_change` do not write `g.last_change` and the `.` register; `Registers.hold_ring` makes a multi-line delete write `"` and `1` without shifting `1`–`9`.

**`Action::Failed`** is emitted (together with the key's other actions) when:
- `h l j k w W b B e E ge gE { } %` cannot move the cursor;
- `f F t T ; ,` does not find the character;
- `n N * #` has no match, or there is no match left when `wrapscan` is off;
- an operator is cancelled because its motion or text object failed (`dfz`, `ci"` with no pair);
- `.` when there has been no change yet; an `ex_command` error.
The V1/V2a tests that currently expect `Handled(vec![])` for these cases are changed to `Handled(vec![Action::Failed])`.

Test harness: `parse_keys` calls `macros::parse_notation`; `Run::outcome` handles `Outcome::Macro(keys)`: `set_macro_running(true)`, `self.key(k, true)` for each key, stopping when the key just run has `Failed`, nesting at most 100, and finally `set_macro_running(false)` and `vim.macro_ended()`.

- [ ] **Step 1: Write the failing tests.**
  - `macros::tests`: `parse_notation("a<lt>b<Esc><C-r>\n<tab>")` → `[Char('a'), Char('<'), Char('b'), Escape, Ctrl('r'), Enter, Char('\t')]`; `push_typed(Text("chào <b>"))` → `"chào <lt>b>"`; for every recorded string, `parse_notation` returns exactly the keys.
  - *Adapted from VSCodeVim `test/macro.test.ts`* (start, keys, end, title):

    | Start | Keys | End | Title |
    |---|---|---|---|
    | `\|foo = 1\nbar = 'a'\nfoobar = foo + bar` | `qaA;<Esc>Ivar <Esc>qj@a` | `var foo = 1;\nvar\| bar = 'a';\nfoobar = foo + bar` | "Can record and execute" |
    | `\|foo = 1\nbar = 'a'\nfoobar = foo + bar` | `qaA;<Esc>Ivar <Esc>qj@aj@@` | `var foo = 1;\nvar bar = 'a';\nvar\| foobar = foo + bar;` | "Can repeat last invoked macro" |
    | `\|"("+a+","+b+","+c+","+d+","+e+")"` | `f+s + <Esc>qq;.q8@q` | `"(" + a + "," + b + "," + c + "," + d + "," + e +\| ")"` | "Can play back with count" |
    | (as above) | `f+s + <Esc>qq;.q22@q` | (as above) | "Can play back with count, abort when a motion fails" |
    | `1. \|one\n2. two\n3. three\n4. four` | `qa0f.r)w~jq3@a` | `1) One\n2) Two\n3) Three\n4) F\|our` | "Repeat change on contiguous lines" |
    | `1. \|one\n2. two\n3. three\n4. four` | `qa0f.r)qqAw~jq3@a` | (as above) | "Append command to a macro" |
    | `1. \|one\n2. two\n3. three\n4. four` | `qB0f.r)w~jq3@b` | (as above) | "Append command to a not yet created register creates a new register" |
    | `1. \|one\n2. two\n3. three\n4. four` | `qa0f.r)w~jq3@A` | (as above) | "Can handle calling an uppercase register" |
    | `1. \|one\n2. two` | `@x` | unchanged | "Can handle calling a non existing macro" |
    | `\|test` ×7 lines | `qadd.q@a@a` | `\|test` | "Can execute macros with dot commands properly" |
    | `\|one two three` ×3 lines | `/two<CR>0qqnea XXX<Esc>q5@q` (wrapscan off) | `one two XXX three\none two XXX three\none two XX\|X three` | "Failed `n` stops macro from repeating" |
    | `\|` | `qBione two<Esc>qo<Esc>@b` | `one two\none tw\|o` | "q[A-Z] (action) Creates new register, accessible by [a-z]" |
    | `one t\|wo three` | `@~` | unchanged; message `E354: Invalid register name: '~'` | "Invalid register throws E354" |

  - Lucerna's own cases:
    - `macro_stops_when_motion_fails`: `qajxq` on 3 lines, `10@a` stops at the last line (because `j` fails), no panic.
    - `macro_with_vietnamese_text`: `qaciwchào<Esc>q` (text typed in the harness's Insert), `w@a` → the next word becomes `chào`; register `a` = `ciwchào<Esc>`.
    - `"ap` after recording pastes the macro's text (Charwise).
    - Running arbitrary text: a `dd` line is yanked into `a` with `"ay$`, then `j@a` deletes the next line.
    - `qa/foo<CR>xq`: register `a` = `/foo<CR>x`; `@a` searches and deletes at the next match without emitting `OpenSearch`.
    - `qa/foo<Esc>xq` (dismiss) → register `a` = `x`.
    - `qa:s/a/b/<CR>jq@a` replaces on the next line; register `a` = `:s/a/b/<CR>j`. `qaVj:d<CR>q` → register `a` = `Vj:d<CR>` (the prefill `'<,'>` is not in the recording; at run time, the `:` key rebuilds it).
    - `dd` ×3 within one macro run does not change `"2`, `"3`; `.` after the macro still repeats the change from before the macro.
    - While recording, `VimGlobals::recording() == Some('a')`; after `q` it is `None`.
- [ ] **Step 2:** `cargo test -p lucerna-vim macros` → FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** `cargo test -p lucerna-vim` and `cargo test -p lucerna-workbench --test vim --test vim_editing` → PASS.
- [ ] **Step 5:** `git commit -m "feat(vim): macros: record, replay, notation, failed motions" -- <the files above>`

---

### Task 5: Highlight on yank

Spec: §9 (`vim.highlightedyank.*`, color from the theme).

**Files:**
- Modify: `crates/lucerna-vim/src/lib.rs` (`Settings.highlighted_yank: bool`, default `false`; `Action::Yanked(Vec<Range<usize>>)`), `src/edit.rs`, `src/state.rs` (Visual `y`/`Y`), `src/ex_run.rs` (`:y`);
  `crates/lucerna-workbench/src/code_view/mod.rs`, `code_view/element.rs`, `src/vim.rs`;
  `crates/lucerna-settings/assets/default-settings.json`.
- Test: `crates/lucerna-workbench/tests/vim_editing.rs`.

**Interfaces (produced):**

```rust
// lucerna-vim: emitted after every yank (y{motion}, yy, Y, Visual y/Y incl. block, :y) when
// Settings::highlighted_yank; byte ranges of the yanked text in the current text.
Action::Yanked(Vec<Range<usize>>)
// lucerna-workbench
impl CodeView {
    /// Paints `ranges` with `theme.ui.search_match_background` for `duration`, then clears them.
    /// A new call, an edit or a new snapshot clears them first.
    pub fn flash_ranges(&mut self, ranges: Vec<Range<usize>>, duration: Duration, cx: &mut Context<Self>);
    pub fn flashed_ranges(&self) -> &[Range<usize>];
}
```

- Linewise: from the start of the first line to the end of the last line (including `\n`); block: `block_ranges`.
- `VimLayer` reads `vim.highlightedyank.enable` into `Settings.highlighted_yank` and `vim.highlightedyank.duration` (ms, `u64`, default 200) into `VimLayer.yank_duration`. `Action::Yanked` → `flash_ranges`. Schedule the timer with `cx.background_executor().timer(duration)`, clearing only if `flash_generation` has not changed.
- `default-settings.json` adds `"vim.gdefault": false`, `"vim.highlightedyank.enable": false`, `"vim.highlightedyank.duration": 200` (next to the `vim.*` keys at lines 139–146).

- [ ] **Step 1: Write the failing tests.**
  - `lucerna-vim`: with `highlighted_yank`, `yiw` on `one t|wo` emits `Yanked(vec![4..7])`; `yy` on line 2 of `a\nbc` emits `Yanked(vec![2..4])`; off, it does not emit; `dd` never emits.
  - Workbench `yank_flash_follows_settings`: turn on `"vim.highlightedyank.enable": true`, `yiw` → `flashed_ranges() == [word]`; `cx.executor().advance_clock(Duration::from_millis(200))` → empty. With `duration: 1000`, it is still there after 200 ms. By default (off) → always empty.
  - Workbench `yank_flash_cleared_by_edit`: `yiw` then `x` → empty at once.
- [ ] **Step 2:** `cargo test -p lucerna-vim yanked` and `scripts/prune-target.sh && cargo test -p lucerna-workbench --test vim_editing yank_flash` → FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the two commands above again → PASS.
- [ ] **Step 5:** `git commit -m "feat(vim): highlighted yank" -- <the files above>`

---

### Task 6: Workbench — the `:` command line through Quick Open

Spec: §8.1, §8.2, §10 (`:%s` large file), §11.2 (command line), §13 (the `:` key conflict).

**Files:**
- Create: `crates/lucerna-workbench/src/vim_ex.rs`, `crates/lucerna-workbench/tests/vim_command_line.rs`
- Modify:
  - `src/quick_open/query.rs`, `src/quick_open/view.rs`, `src/quick_open/mod.rs`;
  - `src/vim.rs`, `src/vim_indicator.rs`, `src/workbench.rs` (around line 3004: replace `take_command` with `run_vim_requests`), `src/lib.rs` (`mod vim_ex;`);
  - `src/save.rs` (`SaveOptions.overwrite: bool`, 5 construction sites gain `overwrite: false`), `src/documents.rs` (`save`: `opts.overwrite` skips `asks` and calls `start_write(…, overwrite: true)`);
  - `src/editor_groups/mod.rs`, `src/editor_groups/view.rs`;
  - `tests/quick_open_query.rs`.

**Interfaces:**
- Consumes: Task 2 `ExCommand`, `Vim::ex_command`, `describe_command`; Task 3 `Outcome::Substitute`, `SubstituteJob::run`, `Vim::finish_substitute`, `BACKGROUND_SUBSTITUTE_BYTES`, `Settings.gdefault`.
- Produces:

```rust
// quick_open/query.rs
pub enum QuickOpenQuery { …, /// `:` with Vim on: a Vim command line (leading spaces trimmed).
    VimCommand(String) }
/// As `parse_query`, but `:` gives `VimCommand`.
pub fn parse_query_vim(input: &str) -> QuickOpenQuery;
// quick_open/view.rs
pub(crate) struct Sources { …, /// Vim is on and the active editor is a code view it takes keys in.
    pub vim_command_line: bool }
enum Row { …, VimCommand { line: String, label: SharedString, valid: bool } }
pub enum QuickOpenEvent { …, VimCommand(String) }
// vim.rs
pub(crate) struct Indicator<'a> { pub label: &'static str, pub recording: Option<char>,
                                  pub pending: &'a str, pub message: Option<&'a str> }
impl VimLayer {
    pub(crate) fn indicator_parts(&self, view: EntityId) -> Option<Indicator<'_>>; // replaces the tuple
    pub(crate) fn run_command_line(&mut self, view: &Entity<CodeView>, line: &str, cx: &mut Context<Self>);
    pub(crate) fn take_ex(&mut self) -> Vec<ExCommand>;
    pub(crate) fn take_command_line(&mut self) -> Option<String>; // the prefill, without ':'
}
// vim_ex.rs
impl Workbench {
    pub(crate) fn run_vim_command(&mut self, line: &str, window: &mut Window, cx: &mut Context<Self>);
    /// Runs what the last vim key or command asked for: a command id, ex commands, the `:` line.
    pub(crate) fn run_vim_requests(&mut self, window: &mut Window, cx: &mut Context<Self>);
    fn run_ex_command(&mut self, cmd: ExCommand, window: &mut Window, cx: &mut Context<Self>);
}
impl EditorGroupsView { pub fn request_close_all(&mut self, window: &mut Window, cx: &mut Context<Self>); }
// command "workbench.action.closeAllEditors", title "View: Close All Editors"
```

Quick Open:
- `show_quick_open` sets `vim_command_line = vim.enabled() && active_code_view().is_some_and(|v| takes_keys(v.read(cx)))`. `query_changed` uses `parse_query_vim` when the flag is on.
- `VimCommand(line)`: empty → `Row::Message("Type a Vim command…")`; otherwise one `Row::VimCommand` with `label` = `":{line} — {desc}"` or the error text of `describe_command` (`valid: false`, drawn in `ui.text_muted`). Enter on both → `QuickOpenEvent::VimCommand(line)` (an error line also runs, so the status bar reports `E492`).
- `on_quick_open_event` → `run_vim_command(line)`: take the active code view, `vim.run_command_line(&view, line)`, then `run_vim_requests`.

VimLayer:
- `Redo::Ex(String)` runs `vim.ex_command`; it reads the clipboard before running (like a key with a register). The ex `Outcome::Retry` goes through the `search_then_run` path like a key.
- `Outcome::Substitute(job)`: if a macro or a replay is running, `job.run` at once; otherwise run `job.run(&snapshot)` in `background_spawn` (wrapped in `guarded`), recording `revision` and `key_generation`; when done, if the revision is intact and there is no new key, `finish_substitute` then `apply`; if the revision has changed, `message = "E: file changed, :s cancelled"`.
- `Action::Message(m)` → `self.message = Some(m)`; cleared at the start of `handle_key`/`feed_keys`. `Action::RunEx(NoHighlight)` → `view.set_match_highlights(Vec::new(), None, cx)`; any other `RunEx` goes into `ex_requests`. `Action::OpenCommandLine{prefill}` → `command_line = Some(prefill)`. `Action::Failed` does nothing here.
- `indicator_text`: with a `message`, show only the message; otherwise `{label}`, plus ` recording @{r}` while recording, plus `  {pending}` when keys are pending. `vim_indicator.rs` draws the message in the same font and color as the mode label.
- `read_settings` reads `vim.gdefault`.

Workbench `run_ex_command`:

| ExCommand | What it does |
|---|---|
| `Save{overwrite:false}` | `run_command("workbench.action.files.save")` |
| `Save{overwrite:true}` | save the active file with `SaveOptions{reason: Explicit, participants: true, overwrite: true}` |
| `SaveAll` | `run_command("workbench.action.files.saveAll")` |
| `Close{revert:false}` | `run_command("workbench.action.closeActiveEditor")` (a dirty tab shows the Phase 4 prompt) |
| `Close{revert:true}` | remember the tab id, wait for `Documents::revert(path)`, then close that tab without asking |
| `CloseAll` | `run_command("workbench.action.closeAllEditors")` |
| `SaveAndClose{only_if_dirty, overwrite}` | remember the path and tab id; with `only_if_dirty` and a clean file, close at once; otherwise wait for `Documents::save(...)`, then close only when the result is `Ok(_)` and the file is no longer dirty |

`request_close_all`: if no tab is dirty, close everything; otherwise `save::prompt_unsaved_all` (Save All / Don't Save / Cancel): Save All saves then closes everything, Don't Save closes everything as `request_close` does with Don't Save, Cancel does nothing.

- [ ] **Step 1: Write the failing tests.**
  - `tests/quick_open_query.rs`: `parse_query_vim(":%s/a/b/")` → `VimCommand("%s/a/b/")`; `parse_query_vim(":  w")` → `VimCommand("w")`; `parse_query_vim("@x")` == `parse_query("@x")`; `parse_query(":-1")` is still `GotoLine{line: Some(-1), column: None}`.
  - `tests/vim_command_line.rs` (helpers copied from `tests/vim_editing.rs`, and `type_text`/`rows` from `tests/quick_open.rs`):
    - `colon_opens_quick_open_with_rows`: Vim on, `:` → Quick Open query `":"`, row `Type a Vim command…`; type `%s/foo/bar/g` → row `:%s/foo/bar/g — Replace in all lines`; `w` → `:w — Save`; `12` → `:12 — Go to line 12`; `foo` → `Not an editor command: foo`.
    - `colon_dispatch_vim_on_and_off`: Vim off, `⌃G`/`:-1` Enter → the last line; `:12,5` → line 12 column 5. Vim on, cursor on line 10: `:-1` Enter → line 9; `:12,5` → the cursor on line 12 (a reversed range, go to the last line of the range); with no tab open, `:12` shows `Open a file to go to a line`.
    - `visual_colon_prefills_marks`: `Vj:` → query `:'<,'>`; type `d` Enter → two lines deleted, one undo step.
    - `colon_w_saves`, `colon_q_dirty_shows_prompt` (`cx.has_pending_prompt()`), `colon_q_bang_reverts_and_closes` (the file on disk unchanged, the tab closed, no prompt), `colon_wq_saves_and_closes`, `colon_x_clean_closes_without_write` (mtime unchanged), `colon_qa_closes_all`.
    - `w_bang_overwrites_without_prompt`: change the file on disk (helper `change_on_disk`) while the tab is dirty → `:w!` writes the tab's content, no prompt.
    - `wq_conflict_cancel_keeps_tab`: change the file on disk → `:wq` → conflict prompt → `simulate_prompt_answer("Cancel")` → the tab stays, still dirty.
    - `substitute_reports_in_status_bar`: `:%s/a/b/g` on `a a\na` → the text changes, `indicator_text == "3 substitutions on 2 lines"`; the next key `j` → back to `-- NORMAL --`; `u` restores in one step.
    - `unknown_command_message`: `:foo` Enter → `E492: Not an editor command: foo`.
    - `large_file_substitute_cancelled_by_reload`: a file > 1 MB; `:%s/x/y/g` Enter; before running the executor, reload the file from disk (helper `change_on_disk`); `cx.run_until_parked()` → the text is the new content on disk, message `E: file changed, :s cancelled`.
    - `large_file_substitute_runs_in_background`: as above but with no reload → replaced, one undo step.
    - `noh_clears_highlights`: `/foo<CR>` then `:noh` → `match_highlights()` is empty.
    - `gdefault_setting_inverts_g`: `"vim.gdefault": true`, `:s/a/b/` on `aaa` → `bbb`.
- [ ] **Step 2:** `scripts/prune-target.sh && cargo test -p lucerna-workbench --test vim_command_line --test quick_open_query` → FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** `cargo test -p lucerna-workbench` → PASS (including `tests/quick_open.rs`, `tests/save.rs`, `tests/vim.rs`, `tests/vim_editing.rs`).
- [ ] **Step 5:** `git commit -m "feat(vim): command line through Quick Open" -- <the files above>`

---

### Task 7: Workbench — run and record macros

Spec: §6.4, §3.1 (the replay loop in the workbench), §11.2 (macros).

**Files:**
- Modify: `crates/lucerna-workbench/src/vim.rs`, `crates/lucerna-workbench/tests/vim_editing.rs`.

**Interfaces:**
- Consumes: Task 4 `Outcome::Macro`, `Action::Failed`, `VimGlobals::{set_macro_running, macro_running, recording}`, `Vim::{record_typed(t, g), macro_ended}`; Task 6 `Indicator`.
- Produces: `const MAX_MACRO_DEPTH: u32 = 100;` and `fn replay(&mut self, view, steps: Vec<ReplayStep>, run: ReplayKind, cx)` with `enum ReplayKind { Dot, Macro }`.

Behavior:
- `Outcome::Macro(keys)` → `replay(view, keys.map(ReplayStep::Key), ReplayKind::Macro, cx)`. The outermost run: `globals.set_macro_running(true)`, `set_typed_events(false)`, `set_suppress_stops(true)` like `.`; at the end `set_macro_running(false)`, `vim.macro_ended()`, and drop any leftover `open_search` and `command_line`.
- A `Key` step when Vim is in Insert and it is not `Escape` is typed (the table in Task 4) instead of `feed`. This rule is also true for `.` (a `Key` step of `.` in Insert is only `Escape`).
- `run` records `self.last_failed = actions.contains(&Action::Failed)`. In `ReplayKind::Macro`, after any step with `last_failed`, stop the whole run (`replay_stopped`). A `.` `Replay` nested inside a macro runs in place; a `Failed` inside it also stops the macro.
- A nested `Macro` increments `macro_depth`; past `MAX_MACRO_DEPTH`, stop the whole run.
- `CodeViewEvent::Typed(t)` → `state.vim.record_typed(t, &mut self.globals)`.
- `may_use_register` adds `'@'`.

- [ ] **Step 1: Write the failing tests** in `tests/vim_editing.rs`:
  - `macro_record_and_count_is_one_undo_step`: `qaA;<Esc>jq` on 4 lines, `3@a` → 4 lines have `;`; right after `qa`, `indicator_text == "-- NORMAL -- recording @a"`; in Insert it is `"-- INSERT -- recording @a"`; after `q` it is `-- NORMAL --`; one `u` undoes all three runs.
  - `macro_with_vietnamese_ime_text_replays_text`: `qa`, `ciw`, `replace_and_mark_text_in_range("chà")`, `replace_text_in_range("chào")` (helper `commit`), Esc, `q`; `w@a` → the next word becomes `chào`.
  - `macro_types_through_auto_close`: `qaci(foo(<Esc>q` then `@a` on another `(x)` → the result is the same as the recording.
  - `macro_search_runs_without_widget`: `qa/foo<CR>xq` (through the real find widget) then `@a` → no find widget is open, the character at the next match is deleted.
  - `macro_stops_when_motion_fails`: `qajxq`, `100@a` on 5 lines → stops at the last line, no panic.
  - `self_calling_macro_stops_at_depth_cap`: type `i@a<Esc>`, `0"ay$` (register `a` = `@a`), `u`; then `@a` → no panic, text unchanged, `cx.run_until_parked()` finishes.
  - `macro_with_colon_command`: `qa:s/a/b/<CR>jq@a` → line 2 is replaced; the message of the last run shows.
- [ ] **Step 2:** `scripts/prune-target.sh && cargo test -p lucerna-workbench --test vim_editing macro` → FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** `cargo test -p lucerna-workbench` → PASS.
- [ ] **Step 5:** `git commit -m "feat(vim): run and record macros in the workbench" -- crates/lucerna-workbench/src/vim.rs crates/lucerna-workbench/tests/vim_editing.rs`

---

### Task 8: Docs, notices, full check

Spec: §12 (the last paragraph).

**Files:**
- `docs/vim.md`: remove the sentence "Macros and `:` commands are planned (V2b)" and the deviation item "macros (`q`, `@`) and `:` commands are V2b". Add a table **Macros and command line (V2b)**: `q{a-z}` / `q{A-Z}` / `q`, `[count]@{reg}`, `@@`, `:` (Quick Open; `'<,'>` from Visual; `3:`), the §8.2 command list, ranges, the `:s` flags, the messages. Note the deviations: a macro records `Completion` approximately (when Phase 4.5 exists), arrow keys in Insert do not go into a macro, `:s` matches by line and is case-sensitive by default, `:N` when Vim is on follows Vim's meaning (`:-1` is up one line). Add the three settings to the settings table.
- `docs/features.md` lines 96–97: add "macros, `:` commands (`:w`, `:q`, `:%s`…)".
- `docs/superpowers/specs/2026-11-08-lucerna-phase1-design.md` §1.3 row V2: "**V2a and V2b done**" (drop "V2b … continues after the P3 fix round").
- `docs/superpowers/specs/2026-11-13-vim-editing-design.md`: the status line becomes `V2a and V2b implemented`.
- `README.md`: if the Vim section lists features, add macros and `:`.
- `THIRD_PARTY_NOTICES.md` section "VSCodeVim test cases": add `test/macro.test.ts`, `test/cmd_line/substitute.test.ts`, `test/cmd_line/delete.test.ts`, `test/cmd_line/yank.test.ts` (cross-check with `grep -rn "Adapted from VSCodeVim" crates/lucerna-vim`).

Steps:
- [ ] **Step 1:** Fix the docs.
- [ ] **Step 2:** Run each command and compare:

  | Command | Expected result |
  |---|---|
  | `scripts/prune-target.sh` | — |
  | `cargo fmt --all -- --check` | prints nothing |
  | `cargo clippy --workspace --all-targets -- -D warnings` | clean |
  | `cargo test --workspace` | all pass |
  | `cargo deny check` | `advisories ok, bans ok, licenses ok, sources ok` |

- [ ] **Step 3:** `git status`, then `git commit -m "docs: vim V2b macros and command line" -- docs/vim.md docs/features.md docs/superpowers/specs/2026-11-08-lucerna-phase1-design.md docs/superpowers/specs/2026-11-13-vim-editing-design.md README.md THIRD_PARTY_NOTICES.md`
- [ ] **Step 4:** `pgrep -x cargo` shows no other cargo, then `scripts/bundle-app.sh --install`. Result: the last line is `bundle-app: installed /Applications/Lucerna.app`. Check by hand per spec §11.3: `:` versus Zed's command palette; a macro with Telex typing.
