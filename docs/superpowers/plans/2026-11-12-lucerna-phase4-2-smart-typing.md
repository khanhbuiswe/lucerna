# Lucerna Phase 4.2 (Smart Typing) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Typing behaves like VS Code. Brackets and quotes auto-close; typing a closer the editor inserted types over it; Enter indents by brackets and `indents.scm`. Each file's indentation is detected and shown in the status bar. Line and block comments toggle, the line commands work, and smart select expands and shrinks by syntax node.

**Architecture:**
- **Language data.** `lucerna-syntax` gains `LanguageEditing`: bracket pairs, `autoclose_before`, line and block comment tokens, and the `indents.scm` source.
  - It comes from a built-in table for bundled languages and from `config.toml` for extension languages.
  - `lucerna-editor` now depends on `lucerna-syntax` (spec §3.2).
- **Editing logic.** New pure modules in `lucerna-editor` (`autoclose`, `indent`, `indentation`, `comment`, `lines`, `smart_select`) return `EditPlan`s or `SelectionSet`s. They follow the existing `commands.rs` pattern.
- **Workbench.** `lucerna-workbench` wires the commands, keys, settings, the `AutocloseRegion` state in `CodeView`, the detected indentation in `Documents`, and the status-bar segment.

**Tech Stack:** Rust 2024; tree-sitter (through `lucerna-syntax`); gpui (workbench only). No new external dependencies.

**Spec:** `docs/superpowers/specs/2026-11-11-lucerna-phase4-editing-design.md`, step **4.2** in §17. Background: §3.2, §3.3, §3.4, §5 (the 4.2 bullets), §14.1 and §14.2 (rows marked 4.2). Research: `docs/research/2026-11-11-editing.md` §4. Phase 4.1 is shipped; read `crates/lucerna-editor/src/commands.rs` for the conventions every new command follows.

## Global Constraints

- **Licensing:**
  - MIT OR Apache-2.0, and `cargo deny check` must pass.
  - Never copy code or `.scm` files from Zed (GPL-3). The format of `config.toml` is data and may be read.
  - The built-in bracket and comment table is written by hand from language facts.
  - Code adapted from VS Code (MIT) carries `// Adapted from microsoft/vscode <path> (MIT)`. Task 9 lists it in `THIRD_PARTY_NOTICES.md`.
- **Crate boundaries:**
  - Only `lucerna-workbench` uses gpui.
  - `lucerna-editor` may now depend on `lucerna-syntax` as well as `lucerna-text`, `unicode-segmentation` and std.
- **Offsets and steps:** UTF-8 byte offsets into LF text. Never split a grapheme cluster.
- **Undo:** every command in this plan is one undo step. Set `stop_before = stop_after = true`, except auto-close and overtype while typing, which keep the typing kind so they group with the surrounding typing as VS Code does.
- **Behaviour, keys and settings follow VS Code; UI follows Zed** (`docs/design-principles.md`).
- **Settings added (exact keys and defaults):**

  | Key | Default |
  |---|---|
  | `editor.autoIndent` | `"full"` (`none`, `keep`, `brackets`, `advanced`, `full`) |
  | `editor.detectIndentation` | `true` |
  | `editor.autoClosingBrackets` | `"languageDefined"` (`always`, `languageDefined`, `beforeWhitespace`, `never`) |
  | `editor.autoClosingQuotes` | `"languageDefined"` (same values) |
  | `editor.autoClosingOvertype` | `"auto"` (`always`, `auto`, `never`) |
  | `editor.autoClosingDelete` | `"auto"` (`always`, `auto`, `never`) |
  | `editor.autoSurround` | `"languageDefined"` (`languageDefined`, `quotes`, `brackets`, `never`) |

- **Keys (macOS; `when` = `editorTextFocus && !editorReadonly`, except smart select, which is `editorTextFocus`):**

  | Command | Key |
  |---|---|
  | `editor.action.commentLine` | `cmd+/` |
  | `editor.action.addCommentLine` | `cmd+k cmd+c` |
  | `editor.action.removeCommentLine` | `cmd+k cmd+u` |
  | `editor.action.blockComment` | `shift+alt+a` |
  | `editor.action.moveLinesUpAction` / `moveLinesDownAction` | `alt+up` / `alt+down` |
  | `editor.action.copyLinesUpAction` / `copyLinesDownAction` | `shift+alt+up` / `shift+alt+down` |
  | `editor.action.deleteLines` | `cmd+shift+k` |
  | `editor.action.insertLineAfter` / `insertLineBefore` | `cmd+enter` / `cmd+shift+enter` |
  | `editor.action.indentLines` / `outdentLines` | `cmd+]` / `cmd+[` |
  | `editor.action.joinLines` | `ctrl+j` |
  | `editor.action.trimTrailingWhitespace` | `cmd+k cmd+x` |
  | `editor.action.smartSelect.expand` / `shrink` | `ctrl+shift+cmd+right` / `ctrl+shift+cmd+left` |

- **Status bar copy (exact):** `Spaces: {n}` when indenting with spaces, `Tab Size: {n}` when indenting with tabs. Display only.
- **Do not touch the 4.4 area** (another session is working on it on its own branch):
  - `lucerna-editor`'s `merge` module;
  - the conflict bar;
  - `save.rs`;
  - `files.saveConflictResolution`.

  If a change there is unavoidable, say so in the commit message. Add new modules instead of growing shared files, and keep edits to `lucerna-editor/src/lib.rs`, `default-settings.json` and `default-keybindings.json` to appended lines, so merges stay trivial.
- **Process:**
  - Run `git pull --rebase` before each task. If it fails over SSH, use `git -c credential.helper='!gh auth git-credential' pull --rebase https://github.com/khanhbuiswe/lucerna.git main`.
  - Run `scripts/prune-target.sh` before every cargo build or test, and keep one build profile on disk.
  - Stage by explicit path and commit on `main`. Trailer, exactly: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
  - Run `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"` before cargo.
  - Each task ends green: `cargo test` on the touched crates, `cargo clippy -- -D warnings`, `cargo fmt --check`.
- **Language:** talk to the user in Vietnamese. Code, comments and commits are in English.

## Review Focus

1. **Quote auto-close inside strings and comments, and after word characters.** Typing `"` inside a string, or `'` in `don't`, must not insert a pair. Test: Task 3 `quotes_do_not_close_in_strings_or_after_words`.
2. **Overtype only types over closers the editor inserted (`auto`).** After the cursor leaves the pair and comes back, typing `)` inserts a new `)`. Test: Task 3 `overtype_forgets_region_after_cursor_leaves`.
3. **Comment toggle over mixed lines with several cursors.** Lines that are blank or already commented in a mixed block are handled as VS Code does: comment all if any non-blank line is uncommented, insert at the minimum indent, and skip blank lines. Test: Task 6 `toggle_mixed_block_comments_all_at_min_indent`.
4. **Move and copy lines at the buffer edges and on a last line with no trailing newline.** These must not lose or merge lines. Test: Task 7 `move_down_last_line_without_newline` and `move_up_first_line_is_noop`.
5. **Indentation detection on a file with no indentation or a mix of tabs and spaces.** It must fall back to the settings and never pick a tab size of 0 or 1 from noise. Test: Task 5 `guess_falls_back_when_unclear`.

---

### Task 1: Language editing data

Spec §3.4 and §3.3 (`indents.scm`).

**Files:**
- Create: `crates/lucerna-syntax/src/editing.rs`
- Modify: `crates/lucerna-syntax/src/{lib.rs,registry.rs}`
- Modify: `crates/lucerna-extension/src/{language_config.rs,index.rs}` and whatever builds `ExtensionLanguage` from `LanguageEntry` (grep `ExtensionLanguage {`)
- Test: unit tests in `editing.rs` and `language_config.rs`

**Interfaces:**
- Produces:
  - `pub struct BracketPair { pub start: String, pub end: String, pub close: bool, pub newline: bool, pub not_in: Vec<String> }`.
  - `pub struct LanguageEditing { pub brackets: Vec<BracketPair>, pub autoclose_before: String, pub line_comments: Vec<String>, pub block_comment: Option<(String, String)> }`.
  - `LanguageEditing::default()`: brackets `()`, `[]`, `{}` with `close: true` and `newline: true`; no quotes; `autoclose_before = ";:.,=}])> \n\t"`; no comments.
  - `Language::editing(&self) -> &LanguageEditing`.
  - `Language::indents_query(&self) -> Option<&str>`: an extension language's `indents.scm` source, read lazily like the other queries. Bundled languages return `None`.
  - `ExtensionLanguage` gains `editing: LanguageEditing`.
  - `LanguageConfig` gains:
    - `brackets: Vec<BracketPairConfig>`, where `BracketPairConfig { start, end, close: bool = true, newline: bool = true, not_in: Vec<String> }`;
    - `autoclose_before: Option<String>`;
    - `block_comment: Option<BlockComment>`, deserialised from either `["/* ", " */"]` or `{ start = "/*", end = "*/", ... }` (only `start` and `end` are used; trim surrounding spaces from both).
- Built-in table (hand-written facts):

  | Language | Line comment | Block comment | Extra pairs (beyond the default brackets) |
  |---|---|---|---|
  | rust | `//` | `/*` `*/` | `"` (`not_in: string`) |
  | typescript, typescriptreact, javascript | `//` | `/*` `*/` | `"`, `'`, `` ` `` (`not_in: string, comment`) |
  | go | `//` | `/*` `*/` | `"`, `` ` `` (`not_in: string, comment`) |
  | python | `#` | — | `"`, `'` (`not_in: string, comment`) |
  | json | — | — | `"` (`not_in: string`) |
  | toml, yaml, bash | `#` | — | `"`, `'` (`not_in: string, comment`) |
  | css | — | `/*` `*/` | `"`, `'` (`not_in: string, comment`) |
  | html | — | `<!--` `-->` | `"`, `'` (`not_in: string, comment`) |
  | markdown | — | `<!--` `-->` | `` ` `` (`not_in: string`) |

  Quote pairs always have `newline: false`.

- [ ] **Step 1: Write failing tests.**
  - `bundled_rust_has_comment_tokens`: `editing().line_comments == ["//"]`, the block comment is `("/*", "*/")`, and the brackets include `"`.
  - `config_toml_brackets_and_block_comment_both_shapes`: parse a TOML with `brackets = [{ start = "{", end = "}", close = true, newline = true }, { start = "\"", end = "\"", close = true, newline = false, not_in = ["string"] }]`, `autoclose_before = ";:.,=}])>"` and `block_comment = ["/* ", " */"]`. The parsed fields match, with the block comment trimmed to `("/*", "*/")`. A second TOML uses the table shape and yields the same block comment.
  - `extension_language_carries_editing`: an `ExtensionLanguage` built through the registry exposes the parsed editing data and its `indents.scm` source when the directory has one.
- [ ] **Step 2: Run them and confirm they fail.**
- [ ] **Step 3: Implement.** Missing `config.toml` keys fall back to `LanguageEditing::default()` for that field.
- [ ] **Step 4: Run `cargo test -p lucerna-syntax -p lucerna-extension -p lucerna-workbench` and confirm it passes.**
- [ ] **Step 5: Commit** `feat(syntax): per-language editing data (brackets, comments, indents)`.

### Task 2: Scope at an offset, and the editor's access to language data

Spec §3.2 (`Ctx`). This task is groundwork for Tasks 3, 4 and 6.

**Files:**
- Modify: `crates/lucerna-syntax/src/syntax_state.rs` (add `scope_at`)
- Modify: `crates/lucerna-editor/{Cargo.toml,src/plan.rs}`
- Modify: `crates/lucerna-workbench/src/code_view/transact.rs` (`edit_config`, and building `Ctx`)

**Interfaces:**
- Produces:
  - `pub enum Scope { Code, String, Comment }`.
  - `SyntaxState::scope_at(&self, snapshot: &BufferSnapshot, offset: usize) -> Scope`. The innermost highlight capture covering `offset` decides it: a name starting with `string` gives `String`, a name starting with `comment` gives `Comment`, anything else gives `Code`. No tree also gives `Code`.
  - `Ctx` gains `language: Option<&'a LanguageEditing>` and `syntax: Option<&'a SyntaxState>`.
  - `EditConfig` gains `auto_indent: AutoIndent`, `auto_closing_brackets: AutoClosing`, `auto_closing_quotes: AutoClosing`, `auto_closing_overtype: AutoClosingEdit`, `auto_closing_delete: AutoClosingEdit` and `auto_surround: AutoSurround`. It replaces `keep_indent`, which becomes `auto_indent >= AutoIndent::Keep`. The enums are:
    - `AutoIndent { None, Keep, Brackets, Advanced, Full }` (ordered);
    - `AutoClosing { Always, LanguageDefined, BeforeWhitespace, Never }`;
    - `AutoClosingEdit { Always, Auto, Never }`;
    - `AutoSurround { LanguageDefined, Quotes, Brackets, Never }`.
  - Each enum has `parse(&str) -> Option<Self>`. `edit_config` reads the settings from Global Constraints with the language override and falls back to the defaults on unknown values.
  - Existing tests that build `EditConfig` or `Ctx` must be updated. Add a `test_util::test_config()` default with `AutoIndent::Full` and the other defaults.

- [ ] **Step 1: Write failing tests.**
  - `scope_at_string_and_comment`: parse Rust `let s = "ab"; // c`. The offset inside `ab` gives `String`, the offset inside `c` gives `Comment`, and the offset at `let` gives `Code`.
  - `edit_config_reads_auto_settings`: with user settings `{"editor.autoIndent": "keep", "editor.autoClosingBrackets": "never"}`, `edit_config` reflects both values.
- [ ] **Step 2: Run them and confirm they fail.**
- [ ] **Step 3: Implement.** The workbench passes `doc` language editing data and the view's `SyntaxState` into `Ctx` wherever it builds one.
- [ ] **Step 4: Run `cargo test -p lucerna-syntax -p lucerna-editor -p lucerna-workbench` and confirm it passes.**
- [ ] **Step 5: Commit** `feat(editor): language data and scope in the edit context; autoIndent and autoClosing settings`.

### Task 3: Auto-close, overtype, auto-delete, surround

Spec §5 (auto-close bullet) and §14.2 rows. Review Focus 1 and 2.

**Files:**
- Create: `crates/lucerna-editor/src/autoclose.rs`
- Modify: `crates/lucerna-editor/src/{commands.rs (insert, delete_left), plan.rs, lib.rs}`
- Modify: `crates/lucerna-workbench/src/code_view/{transact.rs,mod.rs}` (keep, remap and clear the regions), `default-settings.json`
- Test: unit tests in `autoclose.rs`; `crates/lucerna-workbench/tests/editing.rs`

**Interfaces:**
- Produces:
  - `EditPlan` gains `autoclose: Vec<Range<usize>>`, the auto-inserted pair regions (open..close-end) in post-edit offsets. The field is empty by default.
  - `commands::insert` gains a final parameter `regions: &[Range<usize>]`, the view's current regions. It handles a single typed char:
    1. **Overtype.** The char equals the closer right after a cursor whose position is inside a region ending at that closer (`auto`), or any matching closer (`always`): move over it. The plan has no edit for that cursor.
    2. **Surround.** The selection is non-empty, the char is a pair `start`, and `auto_surround` allows it (brackets, quotes or both): wrap the selection and keep it selected inside the pair.
    3. **Auto-close.** All of these hold:
       - the char is a pair `start` with `close: true`;
       - the setting allows it (`languageDefined` uses the language's pairs; `beforeWhitespace` additionally needs the next char to be whitespace or end of line; `always` closes even when the next char is not in `autoclose_before`);
       - the next char is in `autoclose_before`, is whitespace, or is end of line;
       - the scope at the cursor is not in the pair's `not_in`;
       - for a quote (start == end), the previous char is not a word char.

       Then insert `start+end`, put the cursor between them, and record the region.
  - The order of checks is overtype, then surround, then auto-close, then plain insert.
  - `commands::delete_left` gains `regions: &[Range<usize>]`. With `auto_closing_delete` set to `auto`, and an empty cursor between an empty pair that a region covers (or any adjacent matching pair with `always`), it deletes both chars.
  - **Region lifetime in `CodeView`:**
    - Regions are kept in `autoclose_regions: Vec<Range<usize>>` and remapped by `SelectionSet`-style remap on every `apply_edit`.
    - A region is dropped when no cursor is inside it after a selection change, or when its text no longer starts and ends with the pair.
    - Undo clears all regions.

- [ ] **Step 1: Write failing tests** as marked-text rows, using `test_config()` and Rust language data, with a parsed `SyntaxState` where scope matters.
  - `autoclose_bracket`: `"|"` typing `(` gives `"(|)"`, and `"a|b"` typing `(` gives `"a(|b"` (`b` is not in `autoclose_before`).
  - `quotes_do_not_close_in_strings_or_after_words`: `"let s = \"a|\""` typing `"` gives `"let s = \"a\"|\""` (plain insert: no region to overtype, no pair inside a string). `"don|"` typing `'` (TS) gives `"don'|"`.
  - `overtype_closer_in_region`: `"(|)"` with region `0..2`, typing `)`, gives `"()|"`.
  - `overtype_forgets_region_after_cursor_leaves`: in the workbench test, type `(`, press `left`, `right`, `right`, `left`, then type `)`. The text has two `)`.
  - `autodelete_empty_pair`: `"(|)"` with a region, `delete_left` gives `"|"`.
  - `surround_selection`: `"«ab»"` typing `(` gives `"(«ab»)"`, and typing `"` gives `"\"«ab»\""`.
  - `never_setting_disables`: with `auto_closing_brackets = Never`, `"|"` typing `(` gives `"(|"`.
  - `multi_cursor_autoclose`: `"|\n|"` typing `[` gives `"[|]\n[|]"`.
- [ ] **Step 2: Run them and confirm they fail.**
- [ ] **Step 3: Implement.** Auto-close and overtype keep `kind = TypingOther`, so they group with typing. Surround uses `Other` with both stops.
- [ ] **Step 4: Run `cargo test -p lucerna-editor -p lucerna-workbench` and confirm it passes.**
- [ ] **Step 5: Commit** `feat(editor): auto-close, overtype, auto-delete and surround`.

### Task 4: Auto-indent on Enter and on typed closers

Spec §5 (Enter bullet) and §3.3 (`indent_suggestion`).

**Files:**
- Create: `crates/lucerna-editor/src/indent.rs`
- Modify: `crates/lucerna-editor/src/commands.rs` (`newline`, and `insert` for closers)
- Modify: `crates/lucerna-syntax/src/syntax_state.rs` (`indent_suggestion`)
- Test: unit tests in `indent.rs`; one workbench test

**Interfaces:**
- Produces:
  - `pub enum IndentDelta { Same, Indent, Outdent }`.
  - `SyntaxState::indent_suggestion(&self, snapshot: &BufferSnapshot, row: u32) -> Option<IndentDelta>`. It runs the language's `indents.scm`, using captures `@indent` (the node's start row opens a level for rows inside it), `@end` (a row starting with that node closes the level) and `@outdent` (outdent that row). It returns the delta of `row` relative to the previous non-blank row. With no query it returns `None`.
  - `newline` by `auto_indent`:
    - `None`: insert a plain `\n`.
    - `Keep`: as in 4.1 (keep the current line's indent, capped at the cursor column).
    - `Brackets`:
      - When the char before the cursor (ignoring trailing whitespace) is a pair `start` with `newline: true`, indent one unit more.
      - When the cursor is also directly before that pair's `end`, insert two newlines: the cursor goes on the middle line, indented one unit, and the closer goes on the next line at the original indent.
    - `Advanced` and `Full`: as `Brackets`, then, when `indent_suggestion` returns `Some`, apply it to the new row instead of the bracket rule.
  - Typing a pair `end` (with `auto_indent >= Brackets`) on a line that is whitespace-only up to the cursor sets that line's indent to the indent of the line holding the matching `start`. Use `SyntaxState::matching_bracket` when available, else scan backwards counting pairs. This is part of the same typing step.

- [ ] **Step 1: Write failing tests.**
  - `enter_after_open_brace_indents`: `"fn a() {|"` gives `"fn a() {\n    |"`.
  - `enter_between_braces_splits`: `"{|}"` gives `"{\n    |\n}"`.
  - `typing_closer_outdents`: `"{\n    |"` typing `}` gives `"{\n}|"`.
  - `auto_indent_none_and_keep`: with `None`, `"    a|"` gives `"    a\n|"`; with `Keep`, it gives `"    a\n    |"`.
  - `indents_scm_suggestion`: use a tiny test grammar setup (an existing extension test fixture with an `indents.scm`, or `Language::bundled_with_indents_for_test` added in this task). A row inside an `@indent` node is suggested `Indent`.
  - `newline_multi_cursor_brackets`: two cursors after `{` on separate lines each get their own indented line.
- [ ] **Step 2: Run them and confirm they fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run `cargo test -p lucerna-syntax -p lucerna-editor -p lucerna-workbench` and confirm it passes.**
- [ ] **Step 5: Commit** `feat(editor): auto-indent by brackets and indents.scm`.

### Task 5: Detect indentation; status-bar segment

Spec §5 (indentation bullet). Review Focus 5.

**Files:**
- Create: `crates/lucerna-editor/src/indentation.rs`
- Modify: `crates/lucerna-workbench/src/{documents.rs,code_view/transact.rs,layout/status_bar.rs}`, `default-settings.json`
- Test: unit tests in `indentation.rs`; `crates/lucerna-workbench/tests/editing.rs`

**Interfaces:**
- Produces:
  - `pub struct Indentation { pub insert_spaces: bool, pub tab_size: u32 }`.
  - `pub fn guess(snapshot: &BufferSnapshot, default: Indentation) -> Indentation`.
    - Adapted from VS Code `src/vs/editor/common/model/indentationGuesser.ts` (MIT): compare the leading whitespace of consecutive non-blank lines over the first 10 000 lines, count space versus tab indentation, and score the candidate tab sizes 2, 4, 6, 8, 3 and 5 by how often each appears as a space delta.
    - When there are too few indented lines (fewer than 2 deltas), return `default`.
  - `Documents` stores `indentation: Option<Indentation>` per document. It is computed on load and on reload when `editor.detectIndentation` is true for the document's language, and is `None` otherwise.
  - `CodeView::edit_config` uses the detected value over `editor.insertSpaces` and `editor.tabSize` when it is present. Rendering also uses the detected `tab_size`, as VS Code's model options do.
  - Status bar: a segment left of the encoding showing `Spaces: {n}` or `Tab Size: {n}` (exact copy), for the active document's effective indentation. Reuse the existing status-bar segment style.

- [ ] **Step 1: Write failing tests.**
  - `guess_two_spaces`: a file indented by 2 spaces gives `(true, 2)`.
  - `guess_tabs`: a tab-indented file gives `insert_spaces == false`.
  - `guess_falls_back_when_unclear`: a file with no indentation, and a file with a single indented line, return the default. A file mixing one tab-indented line with many 4-space lines gives `(true, 4)`.
  - Workbench `detected_indentation_drives_tab_and_status`: open a 2-space file with `editor.tabSize` 4. `tab` inserts 2 spaces, and the status bar shows `Spaces: 2`. With `editor.detectIndentation: false`, it shows `Spaces: 4`.
- [ ] **Step 2: Run them and confirm they fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run `cargo test -p lucerna-editor -p lucerna-workbench` and confirm it passes.**
- [ ] **Step 5: Commit** `feat(editor): detect indentation; status bar shows it`.

### Task 6: Comments

Spec §5 (comment bullet) and §14.1 rows. Review Focus 3.

**Files:**
- Create: `crates/lucerna-editor/src/comment.rs`
- Modify: `crates/lucerna-workbench/src/editing.rs`, `default-keybindings.json`
- Test: unit tests in `comment.rs`; workbench key test

**Interfaces:**
- Produces (each returns `EditPlan` with kind `Other` and both stops):
  - `comment::toggle_line(ctx, set) -> EditPlan`: uses `line_comments[0]` followed by one space. Over the touched lines (a selection ending at column 0 does not touch its last line):
    - If every non-blank line starts (after its indent) with the token, remove the token and one following space if present.
    - Otherwise add the token and a space at the minimum indent column of the non-blank lines.
    - Blank lines are skipped in both cases.
  - `comment::add_line(ctx, set)` and `comment::remove_line(ctx, set)`: the add-only and remove-only variants.
  - `comment::toggle_block(ctx, set) -> EditPlan`:
    - With a non-empty selection: if it is wrapped by the block tokens (ignoring one space inside each), unwrap it; otherwise wrap it as `start + " " + text + " " + end`.
    - With an empty cursor: insert `start + "  " + end` and put the cursor in the middle.
    - With no block comment, fall back to `toggle_line`, as VS Code does.
  - With no comment data at all, the plan is empty.
  - Selections are kept on the same text after the edit.
  - Commands: `editor.action.commentLine`, `addCommentLine`, `removeCommentLine`, `blockComment`, with the keys from Global Constraints.

- [ ] **Step 1: Write failing tests.**
  - `toggle_comments_and_uncomments`: `"|a\nb"` (Rust) gives `"// |a\nb"`, and toggling again gives `"|a\nb"`.
  - `toggle_mixed_block_comments_all_at_min_indent`: `"«  a\n\n    // b\n  c»"` gives `"«  // a\n\n  //     // b\n  // c»"` (the blank line is untouched).
  - `remove_line_only_removes`, and `add_line_on_commented_adds_again`.
  - `block_comment_wraps_and_unwraps`: `"«ab»"` gives `"/* «ab» */"`, and back.
  - `python_has_no_block_comment_falls_back_to_line`.
  - `multi_cursor_two_regions`: two cursors on separate lines comment both lines.
  - Workbench `cmd_slash_toggles`.
- [ ] **Step 2: Run them and confirm they fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run `cargo test -p lucerna-editor -p lucerna-workbench` and confirm it passes.**
- [ ] **Step 5: Commit** `feat(editor): toggle line and block comments`.

### Task 7: Line commands

Spec §5 (lines bullet) and §14.1 rows. Review Focus 4.

**Files:**
- Create: `crates/lucerna-editor/src/lines.rs`
- Modify: `crates/lucerna-workbench/src/editing.rs`, `default-keybindings.json`
- Test: unit tests in `lines.rs`; workbench key test

**Interfaces:**
- Produces (all `EditPlan`, kind `Other`, both stops). The touched rows of each selection are merged into blocks; a selection ending at column 0 does not touch its last row.
  - `move_lines(ctx, set, up: bool)`: swap each block with the row above or below. At the buffer edge it is a no-op. When the last line has no trailing `\n`, the moved text keeps exactly one line break between rows, and the file still ends without one. Selections move with their text.
  - `copy_lines(ctx, set, up: bool)`: duplicate each block above or below. The selection ends on the copy for `down` and stays on the original for `up`, as in VS Code.
  - `delete_lines(ctx, set)`: delete the whole rows, including their line break. The cursor lands on the following row at the same column, clamped.
  - `insert_line(ctx, set, after: bool)`: insert an empty line after or before the cursor's row, with the indent `newline` would give under the current `auto_indent`, and put the cursor there.
  - `indent_lines(ctx, set)` and `outdent_lines(ctx, set)`: the 4.1 line-mode shift (indent-stop snapping), applied regardless of whether the selection is empty.
  - `join_lines(ctx, set)`:
    - An empty selection joins its row with the next.
    - A multi-row selection joins all of its rows.
    - Each join replaces the line break and the following line's leading whitespace with one space, or with nothing when the line was empty or the next line starts with `)`, `]` or `}`.
    - The cursor goes to the join point.
  - `trim_trailing_whitespace(ctx, set)`: remove trailing spaces and tabs on every line of the document.
  - Commands use the ids and keys from Global Constraints.

- [ ] **Step 1: Write failing tests.**
  - `move_down_swaps`: `"a|\nb\nc"` gives `"b\na|\nc"`.
  - `move_up_first_line_is_noop`.
  - `move_down_last_line_without_newline`: `"a\nb|"` move up gives `"b|\na"`, and `"a|\nb"` move down gives `"b\na|"`. Neither ends with `\n`.
  - `move_block_with_selection`: `"«a\nb»\nc"` gives `"c\n«a\nb»"`.
  - `copy_down_and_up_selection_placement`.
  - `delete_lines_middle_and_last`.
  - `insert_line_after_keeps_indent`: `"    a|b"` gives `"    ab\n    |"`.
  - `insert_line_before`.
  - `join_lines_spaces_and_brackets`: `"a|\n   b"` gives `"a| b"`, and `"f(|\n)"` gives `"f(|)"`.
  - `trim_trailing_whitespace_all_lines`.
  - `two_cursors_same_block_moves_once`: two cursors on the same line move that line once.
  - Workbench `alt_down_moves_line` and `cmd_shift_k_deletes_line`.
- [ ] **Step 2: Run them and confirm they fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run `cargo test -p lucerna-editor -p lucerna-workbench` and confirm it passes.**
- [ ] **Step 5: Commit** `feat(editor): line commands (move, copy, delete, insert, join, indent, trim)`.

### Task 8: Smart select

Spec §5 (smart select bullet) and §3.3 (`enclosing_nodes`).

**Files:**
- Modify: `crates/lucerna-syntax/src/syntax_state.rs` (`enclosing_nodes`)
- Create: `crates/lucerna-editor/src/smart_select.rs`
- Modify: `crates/lucerna-workbench/src/{editing.rs,code_view/mod.rs}`, `default-keybindings.json`
- Test: unit tests; workbench test

**Interfaces:**
- Produces:
  - `SyntaxState::enclosing_nodes(&self, range: Range<usize>) -> Vec<Range<usize>>`: the byte ranges of the named nodes that strictly contain `range` (or equal it when `range` is empty), innermost first, deduplicated. This covers injected layers too.
  - `smart_select::expand(snapshot, syntax: Option<&SyntaxState>, set, word_separators) -> SelectionSet`: each selection grows to the smallest candidate strictly larger than it.
    - Candidates are the word at the cursor, then `enclosing_nodes`, then the current line, then the whole buffer.
    - Without a tree the candidates are word, line, whole buffer.
  - `CodeView` keeps a per-view `smart_select_stack: Vec<SelectionSet>`. `expand` pushes the previous set; `shrink` pops it, or does nothing when the stack is empty. The stack clears on any edit or on any selection change not made by expand or shrink.
  - Commands `editor.action.smartSelect.expand` and `editor.action.smartSelect.shrink`, with the keys from Global Constraints.

- [ ] **Step 1: Write failing tests.**
  - `expand_word_then_node_then_line`: Rust `fn a() { let x = foo(bar); }` with the cursor in `bar`. Expand gives `bar`, then `(bar)` or the call arguments node, then `foo(bar)`, and keeps growing to the whole function and the buffer.
  - `expand_without_tree_uses_word_line_all`.
  - `shrink_restores_previous`.
  - `stack_clears_on_edit`.
- [ ] **Step 2: Run them and confirm they fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run `cargo test -p lucerna-syntax -p lucerna-editor -p lucerna-workbench` and confirm it passes.**
- [ ] **Step 5: Commit** `feat(editor): smart select expand and shrink`.

### Task 9: Docs, notices and the final check

**Files:**
- Modify: `THIRD_PARTY_NOTICES.md` (the indentation guesser, plus anything else marked "Adapted from")
- Modify: `README.md` (4.2 features; add the new settings to the editing settings table)
- Modify: `docs/superpowers/specs/2026-11-11-lucerna-phase4-editing-design.md` status line (`4.1 và 4.2 đã triển khai; 4.3–4.6 chưa làm`)
- Modify: `docs/superpowers/specs/2026-11-08-lucerna-phase1-design.md` roadmap row 4 status

- [ ] **Step 1:** Make the edits.
- [ ] **Step 2:** Run the full check:
  ```bash
  scripts/prune-target.sh
  export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
  cargo test -p lucerna-text -p lucerna-editor -p lucerna-syntax -p lucerna-extension -p lucerna-workbench
  cargo clippy --workspace -- -D warnings
  cargo deny check
  ```
  Expected: everything passes.
- [ ] **Step 3:** Do not claim a manual check that was not run. List the manual checks left for the user: typing brackets and quotes in Rust, TS and Python files; Enter inside `{}`; ⌘/ on a selection; alt+up/down.
- [ ] **Step 4: Commit** `docs: Phase 4.2 smart typing shipped`.
