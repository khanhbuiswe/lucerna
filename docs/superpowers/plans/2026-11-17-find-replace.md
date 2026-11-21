# Find & Replace in a file — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** ⌥⌘F opens the find bar with a Replace row; Replace one match and Replace All follow VS Code, the UI follows Zed.

**Architecture:** The pure replacement logic (parsing the Replace string the VS Code way, Preserve Case, computing edits) lives in the new module `lucerna_project::replace`, with no gpui. `FindWidget` (`find_widget.rs`) gets a Replace input, the `AB` toggle, a Toggle Replace button and two replace buttons, using components from `ui.rs`. Edits go through `CodeView::transact` with an `EditPlan` that has `stop_before` / `stop_after`, so each command is one undo step.

**Tech Stack:** Rust, crate `regex`, GPUI (only in `lucerna-workbench`).

**Spec:** `docs/superpowers/specs/2026-11-17-find-replace-design.md`

## Global Constraints

- App text is English only: placeholder `Replace`; tooltips `Toggle Replace`, `Preserve Case`, `Replace Next Match`, `Replace All Matches`; command labels `Replace`, `Replace One`, `Replace All`, `Toggle Case Sensitive`, `Toggle Whole Word`, `Toggle Use Regular Expression`, `Toggle Preserve Case`.
- Only `lucerna-workbench` uses gpui. `lucerna_project::replace` has no gpui.
- Do not copy VS Code / Zed / Helix code. The comment at the top of `replace.rs` states the behavior source: VS Code `replacePattern.ts` and `search.ts` (MIT).
- `keys.rs`, `default-keybindings.json`, `app_menu.rs`, `CHANGELOG.md`, `docs/keyboard-shortcuts.md`: only add lines, grouped into one block, and do not edit old lines (the ⌘N session is editing these files).
- Cargo: `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`; run `scripts/prune-target.sh` before every build or test; do not set `CARGO_TARGET_DIR`; if another cargo holds the lock, wait.
- Commit: stage each file by path; trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **Clicking Replace while the background search has not finished, or the file just changed.** It must replace nothing, and never replace at an old range. Test in Task 3: `replace_one_ignores_stale_match`.
2. **A replacement string containing Unicode, or a match sitting next to multi-byte characters** (Vietnamese, emoji). Ranges must fall on character boundaries. Preserve Case uses `char`, not bytes. Tests in Task 1: `preserve_case_unicode`, `replace_all_multibyte`.
3. **A regex that matches the empty string** (`^`, `x*`). Replace All does not insert at every position; it skips empty matches like the find bar. Test in Task 1: `replace_all_skips_empty_matches`.
4. **Enter in the Find input while the Replace row is open.** It is still "next match", not replace. Test in Task 3: `enter_in_find_input_still_steps`.
5. **Replace All, then keep typing.** One undo restores everything, not merged with the typing before or after. Test in Task 3: `replace_all_is_one_undo_step`, typing one character before Replace All.

---

### Task 1: The `lucerna_project::replace` module

**Files:**
- Create: `crates/lucerna-project/src/replace.rs` (unit tests in `#[cfg(test)] mod tests` in the same file)
- Modify: `crates/lucerna-project/src/lib.rs` (add `pub mod replace;`)

**Interfaces:**
- Produces:
  - `pub struct ReplacePattern` (`Debug, Clone, PartialEq`)
  - `ReplacePattern::parse(text: &str, is_regex: bool) -> ReplacePattern`
  - `ReplacePattern::is_static(&self) -> bool`
  - `ReplacePattern::expand(&self, caps: &regex::Captures<'_>) -> String`
  - `pub fn preserve_case(matched: &str, replacement: &str) -> String`
  - `pub fn replacement_for(pattern: &ReplacePattern, caps: &regex::Captures<'_>, preserve: bool) -> String`: expand, then, if `preserve`, apply `preserve_case` with `caps[0]`.
  - `pub fn replace_all(text: &str, regex: &regex::Regex, pattern: &ReplacePattern, preserve: bool) -> Vec<(std::ops::Range<usize>, String)>`
  - `pub fn replacement_at(text: &str, regex: &regex::Regex, range: std::ops::Range<usize>, pattern: &ReplacePattern, preserve: bool) -> Option<String>`: uses `regex.captures_at(text, range.start)`; returns `None` if the match found is not exactly `range`.

- [ ] **Step 1: Write the failing tests**

Test helper: `fn exp(pat: &str, re: &str, hay: &str) -> String`, which parses `pat` with `is_regex = true`, takes `Regex::new(re).captures(hay)` and then calls `expand`.

```rust
#[test] fn literal_mode_is_verbatim() {
    let p = ReplacePattern::parse(r"$1\n", false);
    assert!(p.is_static());
    assert_eq!(replace_all("ab", &Regex::new("a").unwrap(), &p, false), vec![(0..1, r"$1\n".to_string())]);
}
#[test] fn dollar_forms() {
    assert_eq!(exp("[$0][$&]", "b+", "abbc"), "[bb][bb]");
    assert_eq!(exp("$2-$1", "(a)(b)", "ab"), "b-a");
    assert_eq!(exp("$$1", "(a)", "a"), "$1");
    assert_eq!(exp("$1a", "(x)", "x"), "xa");          // not a group named "1a"
    assert_eq!(exp("$10", "(x)", "x"), "x0");          // fallback: group 1 + "0"
    assert_eq!(exp("$5", "(a)(b)", "ab"), "$5");       // missing group stays literal
    assert_eq!(exp("$1", "(a)|(b)", "b"), "");         // non-participating group
    assert_eq!(exp("a$", "x", "x"), "a$");
    assert_eq!(exp("$x", "x", "x"), "$x");
}
#[test] fn escapes() {
    assert_eq!(exp(r"a\nb\tc\\d", "x", "x"), "a\nb\tc\\d");
    assert_eq!(exp(r"\q", "x", "x"), r"\q");
    assert_eq!(exp(r"a\", "x", "x"), r"a\");
}
#[test] fn case_ops() {
    assert_eq!(exp(r"\u$1", "(foo)", "foo"), "Foo");
    assert_eq!(exp(r"\U$1-$1", "(foo)", "foo"), "FOO-foo"); // applies to the next piece only
    assert_eq!(exp(r"\l$1", "(FOO)", "FOO"), "fOO");
    assert_eq!(exp(r"\L$0", "FOO", "FOO"), "foo");
    assert_eq!(exp(r"\u\L$1", "(fOO)", "fOO"), "Foo");
    assert_eq!(exp(r"\u", "x", "x"), "");                 // ops alone produce nothing
}
#[test] fn preserve_case_rules() {
    assert_eq!(preserve_case("FOO", "bar"), "BAR");
    assert_eq!(preserve_case("foo", "BaR"), "bar");
    assert_eq!(preserve_case("Foo", "bar"), "Bar");
    assert_eq!(preserve_case("fOO", "Bar"), "bar");        // first lower → first lowered
    assert_eq!(preserve_case("Foo-Bar", "baz-qux"), "Baz-Qux");
    assert_eq!(preserve_case("FOO_bar", "baz_qux"), "BAZ_qux");
    assert_eq!(preserve_case("Foo-Bar", "baz"), "Baz");     // segment counts differ
    assert_eq!(preserve_case("123", "bar"), "BAR");         // "123".to_uppercase() == "123" → rule 2
    assert_eq!(preserve_case("", "bar"), "bar");
}
#[test] fn preserve_case_unicode() {
    assert_eq!(preserve_case("Đường", "phố"), "Phố");
    assert_eq!(preserve_case("ĐƯỜNG", "phố"), "PHỐ");
}
#[test] fn replace_all_skips_empty_matches() {
    let p = ReplacePattern::parse("-", true);
    assert_eq!(replace_all("ab", &Regex::new("x*").unwrap(), &p, false), vec![]);
}
#[test] fn replace_all_multibyte() {
    let p = ReplacePattern::parse("[$0]", true);
    let edits = replace_all("ê a ê", &Regex::new("ê").unwrap(), &p, false);
    assert_eq!(edits, vec![(0..2, "[ê]".into()), (5..7, "[ê]".into())]);
}
#[test] fn replace_all_with_preserve_case() {
    let p = ReplacePattern::parse("bar", false);
    let re = RegexBuilder::new("foo").case_insensitive(true).build().unwrap();
    let edits = replace_all("foo Foo FOO", &re, &p, true);
    assert_eq!(edits.iter().map(|e| e.1.as_str()).collect::<Vec<_>>(), ["bar", "Bar", "BAR"]);
}
#[test] fn replacement_at_checks_range() {
    let p = ReplacePattern::parse("<$1>", true);
    let re = Regex::new("(o+)").unwrap();
    assert_eq!(replacement_at("foo boo", &re, 5..7, &p, false), Some("<oo>".into()));
    assert_eq!(replacement_at("foo boo", &re, 4..7, &p, false), None);
}
```

- [ ] **Step 2: Run the tests, see them fail**

Run: `scripts/prune-target.sh && cargo test -p lucerna-project replace`
Expected: compile error (`replace` does not exist yet).

- [ ] **Step 3: Implement `replace.rs`**

- Representation: `enum Piece { Text(String), Group { index: usize, ops: Vec<CaseOp> } }`, where `index` 0 is the whole match. Use `Vec<Piece>`; `is_static` when there is no `Group`. Case ops accumulate and attach to the next `Group` piece, even when literal text sits in between (`\Uab$1` uppercases `$1`, `ab` stays as is), like VS Code. Once attached, they are cleared, to match the test `\U$1-$1`. An op with no `Group` after it is dropped.
- `$NN`: store the two-digit index when there are two digits. At `expand` time, if `NN >= caps.len()`, use group `N` and then append the second digit. If even `N` is missing (`N >= caps.len()`), output the `$` and the digits verbatim.
- `\u`/`\l` change the first character (by `char`; `to_uppercase()` may yield several chars), then the next op applies to the rest. `\U`/`\L` apply to the rest.
- Preserve Case: the `-`/`_` splitting rule comes first, then all uppercase (`s.to_uppercase() == s`), all lowercase, then the first character. Compare the "upper / lower" of the first character with `char::is_uppercase` / `is_lowercase`.
- `replace_all` uses `regex.captures_iter` when `!is_static() || preserve` needs the matched text; with a static pattern and no preserve it uses `find_iter`.

- [ ] **Step 4: Run the tests, see them pass**

Run: `cargo test -p lucerna-project replace`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/lucerna-project/src/replace.rs crates/lucerna-project/src/lib.rs
git commit -m "feat(project): VS Code replace pattern, preserve case and replace-all edits" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>" -- crates/lucerna-project/src/replace.rs crates/lucerna-project/src/lib.rs
```

---

### Task 2: The Replace row on the find bar and ⌥⌘F

**Files:**
- Create: `crates/lucerna-workbench/assets/icons/replace.svg`, `replace-all.svg`, and an icon for Replace Next (suggested `arrow-right-to-line.svg`). Unmodified Lucide files, taken from `lucide-icons/lucide` `icons/<name>.svg`.
- Modify: `crates/lucerna-workbench/src/icons.rs` (new const + the `ALL` array), `src/ui.rs` (icon const, in the style of `SEARCH_ICON`), `src/find_widget.rs`, `src/workbench.rs` (the `replaceInputFocussed` flag), `crates/lucerna-settings/assets/default-keybindings.json`, `src/app_menu.rs`
- Test: `crates/lucerna-workbench/tests/find_widget.rs`

**Interfaces:**
- Produces (on `FindWidget`):
  - `pub fn replace_input(&self) -> &Entity<TextInput>`
  - `pub fn replace_visible(&self) -> bool`
  - `pub fn preserve_case(&self) -> bool`
  - `pub fn toggle_replace(&mut self, window: &mut Window, cx: &mut Context<Self>)` (toggles, then focuses the matching input)
  - `pub fn toggle_preserve_case(&mut self, cx: &mut Context<Self>)`
  - `pub(crate) fn replace_input_focused(&self, window: &Window, cx: &App) -> bool`
- The debug selectors: `find-toggle-replace`, `find-replace-row`, `find-replace-input`, `find-toggle-preserve-case`, `find-replace-next`, `find-replace-all`.
- The command `editor.action.startFindReplaceAction` (label `Replace`).

- [ ] **Step 1: Write the failing tests** (in `tests/find_widget.rs`, using the existing helpers)

```rust
fn find(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> Entity<FindWidget> {
    widget(view, cx).expect("the find widget is shown")
}

#[gpui::test]
fn alt_cmd_f_shows_replace_row(cx: &mut TestAppContext) {
    init(cx);
    let (_dir, _wb, view, cx) = open_file("a.txt", "foo bar\n", cx);
    keys("alt-cmd-f", cx);
    let f = find(&view, cx);
    assert!(f.read_with(cx, |f, _| f.replace_visible()));
    assert!(cx.debug_bounds("find-replace-row").is_some());
    assert!(cx.debug_bounds("find-replace-next").is_some());
    assert!(cx.debug_bounds("find-replace-all").is_some());
    // No selection: focus is in the find input.
    type_text("foo", cx);
    assert_eq!(counter(&view, cx), "1 of 1");
    keys("escape", cx);
    keys("cmd-f", cx);
    assert!(!find(&view, cx).read_with(cx, |f, _| f.replace_visible()), "cmd-f on a hidden bar has no replace row");
    assert!(cx.debug_bounds("find-replace-row").is_none());
}

#[gpui::test]
fn cmd_f_keeps_open_replace_row(cx: &mut TestAppContext) {
    // alt-cmd-f, then cmd-f while open: replace_visible stays true.
}

#[gpui::test]
fn alt_cmd_f_with_selection_focuses_replace(cx: &mut TestAppContext) {
    // select "bar" (view.select(pt(0,4)..pt(0,7))), alt-cmd-f: find input holds "bar",
    // typing "x" goes to the replace input (replace_input text == "x").
}

#[gpui::test]
fn alt_cmd_f_again_moves_focus_to_replace(cx: &mut TestAppContext) {
    // alt-cmd-f, type "foo" (find input), alt-cmd-f, type "x": replace input == "x".
}

#[gpui::test]
fn toggle_replace_button_and_tab(cx: &mut TestAppContext) {
    // cmd-f, click "find-toggle-replace": row shows, typing goes to replace input.
    // shift-tab: typing goes to find input. tab: back to replace input.
    // click toggle again: row hidden, typing goes to find input.
}

#[gpui::test]
fn replace_text_survives_close(cx: &mut TestAppContext) {
    // alt-cmd-f, focus replace, type "x", escape, alt-cmd-f: replace_input text still "x".
}

#[gpui::test]
fn readonly_view_has_no_replace(cx: &mut TestAppContext) {
    // view.update(|v,_| v.set_passive(true)); alt-cmd-f: bar shows, replace_visible false,
    // debug_bounds("find-toggle-replace") is None.
}

#[gpui::test]
fn vim_search_hides_replace(cx: &mut TestAppContext) {
    // Follow the existing vim `/` test setup (tests/vim.rs): with replace_visible set
    // by alt-cmd-f then escape, `/` shows no "find-replace-row" and no "find-toggle-replace".
}
```

Write out the full bodies of the tests that have comments, with exactly the asserts stated in the comments.

- [ ] **Step 2: Run the tests, see them fail**

Run: `scripts/prune-target.sh && cargo test -p lucerna-workbench --test find_widget`
Expected: the new tests FAIL (no command / selector).

- [ ] **Step 3: Implement**

- `FindWidget::new`: create `replace_input` (placeholder `Replace`, no `on_change`). Add `replace_visible: false`, `preserve_case: false`. Add `can_replace(&self, cx) -> bool`: the view is still alive, not `is_readonly()`, and `!self.vim`.
- `render`:
  - After `field` in row 1, when `can_replace`, add `bar_icon_button("find-toggle-replace", REPLACE_ICON, self.replace_visible, cx)` with the tooltip `Toggle Replace`; a click calls `toggle_replace`.
  - When `replace_visible && can_replace`, add row 2 as a `search_bar_row()` containing:
    - `search_query_column(input_box(&self.replace_input, cx))` with the debug selector `find-replace-input`, holding `toggle_button("find-toggle-preserve-case", "AB", …)` with the tooltip `Preserve Case`;
    - `bar_separator`;
    - two `bar_icon_button`s: `find-replace-next` (tooltip `Replace Next Match`) and `find-replace-all` (`Replace All Matches`). A click calls the Task 3 functions; in this task leave them as a no-op or a `todo` through an empty `FindWidget::replace_one`. When `matches` is empty or there is an `error`, the button is dimmed (`text_color(ui.icon_disabled)` or an equivalent already in the theme) and gets no `on_click`.
  - Row 2 starts aligned with the query input, because the toggle button sits *after* the query input.
- `open(view, window, cx)` takes an extra `replace: bool`:
  - `actions.find`: if the widget is hidden, set `replace_visible = false`; if it is shown, leave it as is.
  - `editor.action.startFindReplaceAction`: set `replace_visible = can_replace`. Focus the Replace input if `can_replace` and (the Find input already had focus before the command ran, or it was just filled from the selection); otherwise focus the Find input.
- Tab / Shift+Tab: in `key_down`, `tab` when the Find input has focus and the Replace row is shown focuses the Replace input; `shift-tab` when the Replace input has focus focuses the Find input. Together with `stop_propagation`.
- The existing `key_down`: only handles Enter when the Find input has focus (Enter in the Replace input is left to the keymap in Task 3).
- `Workbench` (`workbench.rs`, next to `findWidgetVisible`): `flag("replaceInputFocussed", …)`, true when the active view's widget is shown and `replace_input_focused`.
- Keybinding (add at the end of the file, opening the replace block):
  `{ "key": "alt+cmd+f", "command": "editor.action.startFindReplaceAction", "when": "editorFocus" }`
- Edit menu: `command("Replace", "editor.action.startFindReplaceAction")` right after the `Find` line.

- [ ] **Step 4: Run the tests, see them pass**

Run: `cargo test -p lucerna-workbench --test find_widget && cargo test -p lucerna-workbench --test app_menu`
Expected: PASS. If `app_menu` has a test that lists the menu items, update it to include `Replace`.

- [ ] **Step 5: Commit** the files listed above, by path. Message: `feat: replace row on the find bar, alt-cmd-f`.

---

### Task 3: Replace one match and Replace All

**Files:**
- Modify: `crates/lucerna-workbench/src/find_widget.rs`, `crates/lucerna-editor/src/commands.rs` (make `mk_patch` `pub` and re-export it in `lib.rs` as `lucerna_editor::mk_patch`), `crates/lucerna-editor/src/lib.rs`, `crates/lucerna-settings/assets/default-keybindings.json`
- Test: `crates/lucerna-workbench/tests/find_widget.rs`

**Interfaces:**
- Consumes: Task 1 (`ReplacePattern`, `replace_all`, `replacement_at`); Task 2 (`replace_input`, `replace_visible`, `preserve_case`).
- Produces:
  - `pub fn replace_one(&mut self, cx: &mut Context<Self>)`
  - `pub fn replace_all(&mut self, cx: &mut Context<Self>)`
  - `pub fn mk_patch(edits: &[(Range<usize>, Arc<str>)]) -> lucerna_text::Patch` in `lucerna_editor`
  - The commands `editor.action.replaceOne` (`Replace One`) and `editor.action.replaceAll` (`Replace All`).

- [ ] **Step 1: Write the failing tests**

```rust
fn replace_text(view: &Entity<CodeView>, text: &str, cx: &mut VisualTestContext) {
    // alt-cmd-f, then (find input focused) tab, then type `text` into the replace input.
}
fn text(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| { let s = view.read(cx).snapshot(); s.text_for_range(0..s.len_bytes()).to_string() })
}

#[gpui::test]
fn replace_one_replaces_active_and_moves_on(cx: &mut TestAppContext) {
    // "foo\nbar foo\nfoo\n": alt-cmd-f, type "foo", tab, type "X", enter.
    assert_eq!(text(&view, cx), "X\nbar foo\nfoo\n");
    assert_eq!(counter(&view, cx), "1 of 2");
    assert_eq!(view.read_with(cx, |v, _| v.selection()), Some(pt(1, 4)..pt(1, 7)));
    // Undo once restores; one step.
    keys("cmd-z", cx); // focus is in the replace input: use the view's undo directly if cmd-z goes to the input
}

#[gpui::test]
fn replace_one_wraps_to_first(cx: &mut TestAppContext) {
    // "foo a foo": active is the 2nd match (cursor placed after the first before alt-cmd-f),
    // replace -> text "foo a X", active match is 1 of 1 at 0..3.
}

#[gpui::test]
fn replace_one_only_selects_when_selection_moved(cx: &mut TestAppContext) {
    // after finding "foo", move cursor with view.go_to(pt(0,1)) (selection no longer == match),
    // shift-cmd-1: text unchanged, the next match at/after the cursor is selected.
}

#[gpui::test]
fn replace_one_ignores_stale_match(cx: &mut TestAppContext) {
    // find "foo", then edit the buffer via view.update(|v,cx| v.type_text(..)) before the
    // search re-runs is impossible synchronously; instead edit at 0 so the active range is stale,
    // call find.update(|f,cx| f.replace_one(cx)) in the same update as the edit: no text replaced
    // at the old range (replacement_at returns None).
}

#[gpui::test]
fn replace_all_is_one_undo_step(cx: &mut TestAppContext) {
    // "foo foo foo": type "z" at end via view.type_text first (separate step),
    // alt-cmd-f, "foo", tab, "b", cmd-enter -> "b b bz"? (spell exact result) counter "No results".
    // view.undo(true) once -> back to "foo foo fooz".
}

#[gpui::test]
fn alt_cmd_enter_and_shift_cmd_1_from_editor(cx: &mut TestAppContext) {
    // with the bar open and focus back in the editor text? Focus stays in the find input:
    // alt-cmd-enter replaces all; shift-cmd-1 replaces one.
}

#[gpui::test]
fn enter_in_find_input_still_steps(cx: &mut TestAppContext) {
    // alt-cmd-f, type "foo" (find input), enter: counter "2 of 3", text unchanged.
}

#[gpui::test]
fn replace_regex_groups_and_preserve_case(cx: &mut TestAppContext) {
    // "let a = 1;": click find-toggle-regex, find "(\w+) = (\d)", replace "$2 = $1", cmd-enter
    //   -> "let 1 = a;".
    // New file "Foo foo": find "foo", replace "bar", click find-toggle-preserve-case, cmd-enter
    //   -> "Bar bar".
}

#[gpui::test]
fn readonly_view_does_not_replace(cx: &mut TestAppContext) {
    // cmd-f "foo", set_passive(true), alt-cmd-enter and shift-cmd-1: text unchanged.
}

#[gpui::test]
fn replace_all_large_file_off_main_thread(cx: &mut TestAppContext) {
    // the 1 MB+ file from find_large_file_searches_off_the_main_thread; replace "needle" with "pin"
    // via cmd-enter; after draw, text ends with "the pin\n" and its length shrank by 3.
}
```

Write out the full bodies of the tests that have comments, with exactly the asserts stated in the comments. For undo in tests, call `view.update(cx, |v, cx| v.undo(true, cx))`, so it does not depend on which key has focus.

- [ ] **Step 2: Run the tests, see them fail**

Run: `scripts/prune-target.sh && cargo test -p lucerna-workbench --test find_widget replace`
Expected: FAIL.

- [ ] **Step 3: Implement**

- New field `pending_pick_from: Option<usize>`. In `apply`, if set, pick the first match with `start >= offset` (0 if none), reveal it, then clear it. This goes before the `Pick::Keep` branch that runs when the revision changes.
- `replace_one`:
  - Do nothing if `!can_replace`, `!replace_visible`, `pending`, or `revision` differs from the view's revision.
  - Take `range = matches[active]`. If the offset of the current selection (`view.selection()` converted to offsets) differs from `range`, call `self.step(0…)`, that is, select `nearest` and reveal it, and stop.
  - If it matches: build the regex with `build_regex` and compute `replacement_at` on the snapshot's text. On `None`, stop.
  - Plan: `EditPlan::new(vec![(range, text.into())], SelectionSet::cursor(range.start + text.len()), EditKind::Other)` with `stop_before = stop_after = true`. Call `view.update(|v, cx| v.transact(plan, cx))`.
  - Set `pending_pick_from = Some(range.start + text.len())`. The search triggered by the revision change will pick the next match.
- `replace_all`:
  - The same guard conditions as `replace_one`, except the selection check.
  - Smaller than or equal to `BACKGROUND_SEARCH_BYTES`: compute `replace_all(...)` right away.
  - Larger: `cx.background_spawn` through `guarded("replace all", …)`, then apply only if the view's `revision` has not changed. Share `_search` / `generation` so a new search cancels it.
  - Apply: if empty, stop. If not empty, `edits: Vec<(Range, Arc<str>)>`; `selections = view.selections().clone()` then `remap(&mk_patch(&edits))`; build an `EditPlan` with `EditKind::Other`, `stop_before = stop_after = true`, then `transact`.
- Commands, registered in `register_commands`: `editor.action.replaceOne` / `editor.action.replaceAll` call the function on the active view's widget, when the widget is shown.
- Keybindings, continuing the Task 2 block:
  ```json
  { "key": "shift+cmd+1", "command": "editor.action.replaceOne", "when": "editorFocus && findWidgetVisible" },
  { "key": "alt+cmd+enter", "command": "editor.action.replaceAll", "when": "editorFocus && findWidgetVisible" },
  { "key": "enter", "command": "editor.action.replaceOne", "when": "editorFocus && replaceInputFocussed" },
  { "key": "cmd+enter", "command": "editor.action.replaceAll", "when": "editorFocus && replaceInputFocussed" }
  ```
- The two buttons in row 2 call `replace_one` / `replace_all`.

- [ ] **Step 4: Run the tests, see them pass**

Run: `cargo test -p lucerna-workbench --test find_widget && cargo test -p lucerna-editor`
Expected: PASS.

- [ ] **Step 5: Commit** the task's files, by path. Message: `feat: replace one and replace all in the find bar`.

---

### Task 4: Toggle shortcuts ⌥⌘C/W/R/P

**Files:**
- Modify: `crates/lucerna-workbench/src/find_widget.rs`, `crates/lucerna-settings/assets/default-keybindings.json`
- Test: `crates/lucerna-workbench/tests/find_widget.rs`

**Interfaces:**
- Consumes: Task 2 (`toggle_preserve_case`); the existing `toggle_*` functions.
- Produces: the commands `toggleFindCaseSensitive`, `toggleFindWholeWord`, `toggleFindRegex`, `togglePreserveCase`; getters `pub fn case_sensitive(&self) -> bool`, `whole_word()`, `is_regex()`.

- [ ] **Step 1: Write the failing tests**

```rust
#[gpui::test]
fn toggle_keys_flip_find_options(cx: &mut TestAppContext) {
    // "Foo foo": cmd-f, type "foo" -> "1 of 2"; alt-cmd-c -> counter "1 of 1"; alt-cmd-c again -> "1 of 2".
    // alt-cmd-w, alt-cmd-r, alt-cmd-p each flip the getter on the widget.
}

#[gpui::test]
fn toggle_keys_work_with_bar_hidden(cx: &mut TestAppContext) {
    // focus in the editor, bar never opened: alt-cmd-r; then cmd-f -> widget.is_regex() is true.
}

#[gpui::test]
fn alt_cmd_c_in_explorer_still_copies_path(cx: &mut TestAppContext) {
    // keymap check only: resolve "alt+cmd+c" with filesExplorerFocus && !editorFocus
    // -> copyFilePath; with editorFocus -> toggleFindCaseSensitive. Use Keymap::resolve and
    // lucerna_settings::Context like the existing keymap tests in this crate.
}
```

- [ ] **Step 2: Run the tests, see them fail** — `cargo test -p lucerna-workbench --test find_widget toggle_keys`, FAIL.

- [ ] **Step 3: Implement**

- Add `fn widget_for_toggle(view, cx) -> Entity<FindWidget>`: return the existing widget (even when hidden), otherwise create `FindWidget::new` and attach it to the view without showing it. This needs a `CodeView::set_find_widget_hidden(find)` or equivalent: set `self.find = Some(find)` without changing `find_visible`.
- The four commands call `toggle_*` on that widget. When the widget is hidden, `search` still runs but does not highlight: `apply` only calls `set_match_highlights` when `view.find_widget().is_some()`. Re-check `apply`: it calls it unconditionally, so this condition has to be added.
- Keybindings, continuing the block:
  ```json
  { "key": "alt+cmd+c", "command": "toggleFindCaseSensitive", "when": "editorFocus" },
  { "key": "alt+cmd+w", "command": "toggleFindWholeWord", "when": "editorFocus" },
  { "key": "alt+cmd+r", "command": "toggleFindRegex", "when": "editorFocus" },
  { "key": "alt+cmd+p", "command": "togglePreserveCase", "when": "editorFocus" }
  ```

- [ ] **Step 4: Run the tests, see them pass** — `cargo test -p lucerna-workbench --test find_widget`, PASS.

- [ ] **Step 5: Commit** the task's files. Message: `feat: find toggle shortcuts alt-cmd-c/w/r/p`.

---

### Task 5: Docs and a full check

**Files:**
- Modify: `docs/keyboard-shortcuts.md` (add lines after the `Cmd+F` line: `Alt+Cmd+F` Replace in file, `Shift+Cmd+1` Replace one, `Alt+Cmd+Enter` Replace all, `Alt+Cmd+C/W/R/P` Toggle case / whole word / regex / preserve case)
- Modify: `CHANGELOG.md` (under `## [Unreleased]`, add this feature's own `### Added` entry, right after the `[Unreleased]` heading, written in the voice of the older entries)
- Modify: `docs/roadmap.md` (mark the Find & Replace line `[x]`)
- Modify: `docs/design-principles.md` §1.1 (remove the example sentence "for instance, dropping Replace because Lucerna is only for viewing code")

- [ ] **Step 1:** Edit the four files above.
- [ ] **Step 2:** `scripts/prune-target.sh && cargo test -p lucerna-workbench && cargo test -p lucerna-project -p lucerna-editor -p lucerna-settings`. Expected: all PASS.
- [ ] **Step 3:** `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check`. Expected: clean.
- [ ] **Step 4:** Commit the four docs files, by path. Message: `docs: find and replace shortcuts, changelog, roadmap`.
