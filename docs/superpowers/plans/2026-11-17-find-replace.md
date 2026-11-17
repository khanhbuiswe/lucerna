# Find & Replace trong file — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** ⌥⌘F mở thanh tìm kèm hàng Replace; Replace một match và Replace All theo VS Code, giao diện theo Zed.

**Architecture:** Logic thay thế thuần (đọc chuỗi Replace kiểu VS Code, Preserve Case, tính edit) nằm ở module mới `lucerna_project::replace`, không gpui. `FindWidget` (`find_widget.rs`) thêm ô Replace, toggle `AB`, nút Toggle Replace và hai nút replace, dùng component trong `ui.rs`. Edit đi qua `CodeView::transact` với một `EditPlan` có `stop_before` / `stop_after`, nên mỗi lệnh là một bước undo.

**Tech Stack:** Rust, crate `regex`, GPUI (chỉ trong `lucerna-workbench`).

**Spec:** `docs/superpowers/specs/2026-11-17-find-replace-design.md`

## Global Constraints

- Chữ trong app chỉ tiếng Anh: placeholder `Replace`; tooltip `Toggle Replace`, `Preserve Case`, `Replace Next Match`, `Replace All Matches`; nhãn lệnh `Replace`, `Replace One`, `Replace All`, `Toggle Case Sensitive`, `Toggle Whole Word`, `Toggle Use Regular Expression`, `Toggle Preserve Case`.
- Chỉ `lucerna-workbench` dùng gpui. `lucerna_project::replace` không có gpui.
- Không chép code VS Code / Zed / Helix. Comment đầu `replace.rs` ghi nguồn hành vi: VS Code `replacePattern.ts` và `search.ts` (MIT).
- `keys.rs`, `default-keybindings.json`, `app_menu.rs`, `CHANGELOG.md`, `docs/keyboard-shortcuts.md`: chỉ thêm dòng, gom thành một khối, không sửa dòng cũ (session ⌘N đang sửa các file này).
- Cargo: `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`; chạy `scripts/prune-target.sh` trước mỗi lần build hoặc test; không đặt `CARGO_TARGET_DIR`; nếu cargo khác giữ lock thì chờ.
- Commit: stage từng file theo đường dẫn; trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **Bấm Replace khi tìm nền chưa xong, hoặc file vừa đổi.** Phải không thay gì, không bao giờ thay ở range cũ. Test ở Task 3: `replace_one_ignores_stale_match`.
2. **Chuỗi thay chứa Unicode, hoặc match nằm giữa ký tự nhiều byte** (tiếng Việt, emoji). Range phải đúng ranh giới ký tự. Preserve Case dùng `char`, không dùng byte. Test ở Task 1: `preserve_case_unicode`, `replace_all_multibyte`.
3. **Regex khớp chuỗi rỗng** (`^`, `x*`). Replace All không chèn vào mọi vị trí, bỏ match rỗng như thanh tìm. Test ở Task 1: `replace_all_skips_empty_matches`.
4. **Enter trong ô Find khi hàng Replace đang mở.** Vẫn là "next match", không phải replace. Test ở Task 3: `enter_in_find_input_still_steps`.
5. **Replace All rồi gõ tiếp.** Undo một lần trả lại toàn bộ, không gộp với lần gõ trước hay sau. Test ở Task 3: `replace_all_is_one_undo_step`, gõ một ký tự trước khi Replace All.

---

### Task 1: Module `lucerna_project::replace`

**Files:**
- Create: `crates/lucerna-project/src/replace.rs` (unit test trong `#[cfg(test)] mod tests` cùng file)
- Modify: `crates/lucerna-project/src/lib.rs` (thêm `pub mod replace;`)

**Interfaces:**
- Produces:
  - `pub struct ReplacePattern` (`Debug, Clone, PartialEq`)
  - `ReplacePattern::parse(text: &str, is_regex: bool) -> ReplacePattern`
  - `ReplacePattern::is_static(&self) -> bool`
  - `ReplacePattern::expand(&self, caps: &regex::Captures<'_>) -> String`
  - `pub fn preserve_case(matched: &str, replacement: &str) -> String`
  - `pub fn replacement_for(pattern: &ReplacePattern, caps: &regex::Captures<'_>, preserve: bool) -> String`: expand rồi, nếu `preserve`, áp `preserve_case` với `caps[0]`.
  - `pub fn replace_all(text: &str, regex: &regex::Regex, pattern: &ReplacePattern, preserve: bool) -> Vec<(std::ops::Range<usize>, String)>`
  - `pub fn replacement_at(text: &str, regex: &regex::Regex, range: std::ops::Range<usize>, pattern: &ReplacePattern, preserve: bool) -> Option<String>`: dùng `regex.captures_at(text, range.start)`; trả `None` nếu match tìm được không đúng `range`.

- [ ] **Step 1: Viết test hỏng**

Helper trong test: `fn exp(pat: &str, re: &str, hay: &str) -> String`, parse `pat` với `is_regex = true`, lấy `Regex::new(re).captures(hay)` rồi `expand`.

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

- [ ] **Step 2: Chạy test, thấy hỏng**

Run: `scripts/prune-target.sh && cargo test -p lucerna-project replace`
Expected: lỗi biên dịch (`replace` chưa có).

- [ ] **Step 3: Cài đặt `replace.rs`**

- Biểu diễn: `enum Piece { Text(String), Group { index: usize, ops: Vec<CaseOp> } }`, với `index` 0 là cả match. Dùng `Vec<Piece>`; `is_static` khi không có `Group`. Case op dồn lại và gắn vào mảnh `Group` kế tiếp, kể cả khi có chữ thường đứng giữa (`\Uab$1` viết hoa `$1`, `ab` giữ nguyên), như VS Code. Gắn xong thì xóa, cho khớp test `\U$1-$1`. Op không có `Group` nào theo sau thì bỏ.
- `$NN`: lưu chỉ số hai chữ số khi có hai chữ số. Lúc `expand`, nếu `NN >= caps.len()` thì dùng nhóm `N` rồi thêm chữ số thứ hai. Nếu ngay cả `N` cũng không có (`N >= caps.len()`) thì xuất lại nguyên văn `$` và các chữ số.
- `\u`/`\l` đổi ký tự đầu (theo `char`, dùng `to_uppercase()` có thể ra nhiều char) rồi sang op kế tiếp áp lên phần còn lại. `\U`/`\L` áp lên phần còn lại.
- Preserve Case: quy tắc tách `-`/`_` trước, sau đó toàn hoa (`s.to_uppercase() == s`), toàn thường, rồi ký tự đầu. So "hoa / thường" của ký tự đầu bằng `char::is_uppercase` / `is_lowercase`.
- `replace_all` dùng `regex.captures_iter` khi `!is_static() || preserve` cần text match; với pattern tĩnh và không preserve thì `find_iter`.

- [ ] **Step 4: Chạy test, thấy qua**

Run: `cargo test -p lucerna-project replace`
Expected: tất cả PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/lucerna-project/src/replace.rs crates/lucerna-project/src/lib.rs
git commit -m "feat(project): VS Code replace pattern, preserve case and replace-all edits" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>" -- crates/lucerna-project/src/replace.rs crates/lucerna-project/src/lib.rs
```

---

### Task 2: Hàng Replace trên thanh tìm và ⌥⌘F

**Files:**
- Create: `crates/lucerna-workbench/assets/icons/replace.svg`, `replace-all.svg`, và icon cho Replace Next (gợi ý `arrow-right-to-line.svg`). File Lucide nguyên bản, lấy từ `lucide-icons/lucide` `icons/<name>.svg`.
- Modify: `crates/lucerna-workbench/src/icons.rs` (const mới + mảng `ALL`), `src/ui.rs` (const icon, theo kiểu `SEARCH_ICON`), `src/find_widget.rs`, `src/workbench.rs` (flag `replaceInputFocussed`), `crates/lucerna-settings/assets/default-keybindings.json`, `src/app_menu.rs`
- Test: `crates/lucerna-workbench/tests/find_widget.rs`

**Interfaces:**
- Produces (trên `FindWidget`):
  - `pub fn replace_input(&self) -> &Entity<TextInput>`
  - `pub fn replace_visible(&self) -> bool`
  - `pub fn preserve_case(&self) -> bool`
  - `pub fn toggle_replace(&mut self, window: &mut Window, cx: &mut Context<Self>)` (đảo, rồi focus ô tương ứng)
  - `pub fn toggle_preserve_case(&mut self, cx: &mut Context<Self>)`
  - `pub(crate) fn replace_input_focused(&self, window: &Window, cx: &App) -> bool`
- Các debug selector: `find-toggle-replace`, `find-replace-row`, `find-replace-input`, `find-toggle-preserve-case`, `find-replace-next`, `find-replace-all`.
- Lệnh `editor.action.startFindReplaceAction` (nhãn `Replace`).

- [ ] **Step 1: Viết test hỏng** (trong `tests/find_widget.rs`, dùng helper có sẵn)

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

Viết đủ thân các test có comment theo đúng các assert ghi trong comment.

- [ ] **Step 2: Chạy test, thấy hỏng**

Run: `scripts/prune-target.sh && cargo test -p lucerna-workbench --test find_widget`
Expected: test mới FAIL (không có lệnh / selector).

- [ ] **Step 3: Cài đặt**

- `FindWidget::new`: tạo `replace_input` (placeholder `Replace`, không `on_change`). Thêm `replace_visible: false`, `preserve_case: false`. Thêm `can_replace(&self, cx) -> bool`: view còn sống, không `is_readonly()`, và `!self.vim`.
- `render`:
  - Sau `field` ở hàng 1, khi `can_replace`, thêm `bar_icon_button("find-toggle-replace", REPLACE_ICON, self.replace_visible, cx)` có tooltip `Toggle Replace`, click gọi `toggle_replace`.
  - Khi `replace_visible && can_replace`, thêm hàng 2 là `search_bar_row()` gồm:
    - `search_query_column(input_box(&self.replace_input, cx))` có debug selector `find-replace-input`, chứa `toggle_button("find-toggle-preserve-case", "AB", …)` với tooltip `Preserve Case`;
    - `bar_separator`;
    - hai `bar_icon_button`: `find-replace-next` (tooltip `Replace Next Match`) và `find-replace-all` (`Replace All Matches`). Click gọi hàm của Task 3; ở task này để no-op hoặc `todo` bằng một `FindWidget::replace_one` rỗng. Khi `matches` trống hoặc có `error`, nút mờ (`text_color(ui.icon_disabled)` hoặc tương đương có sẵn trong theme) và không gắn `on_click`.
  - Hàng 2 bắt đầu thẳng hàng với ô query, vì nút toggle nằm *sau* ô query.
- `open(view, window, cx)` nhận thêm `replace: bool`:
  - `actions.find`: nếu widget đang ẩn thì đặt `replace_visible = false`; nếu đang hiện thì giữ nguyên.
  - `editor.action.startFindReplaceAction`: đặt `replace_visible = can_replace`. Focus ô Replace nếu `can_replace` và (ô Find đã có focus trước khi lệnh chạy, hoặc vừa điền từ selection); ngược lại focus ô Find.
- Tab / Shift+Tab: trong `key_down`, `tab` khi ô Find có focus và hàng Replace hiện thì focus ô Replace; `shift-tab` khi ô Replace có focus thì focus ô Find. Kèm `stop_propagation`.
- `key_down` hiện có: chỉ xử lý Enter khi ô Find có focus (Enter trong ô Replace để keymap ở Task 3 lo).
- `Workbench` (`workbench.rs`, cạnh `findWidgetVisible`): `flag("replaceInputFocussed", …)`, true khi widget của view active đang hiện và `replace_input_focused`.
- Keybinding (thêm cuối file, mở khối replace):
  `{ "key": "alt+cmd+f", "command": "editor.action.startFindReplaceAction", "when": "editorFocus" }`
- Menu Edit: `command("Replace", "editor.action.startFindReplaceAction")` ngay sau dòng `Find`.

- [ ] **Step 4: Chạy test, thấy qua**

Run: `cargo test -p lucerna-workbench --test find_widget && cargo test -p lucerna-workbench --test app_menu`
Expected: PASS. Nếu `app_menu` có test liệt kê mục menu thì cập nhật cho có `Replace`.

- [ ] **Step 5: Commit** các file đã liệt kê ở trên, theo đường dẫn. Message: `feat: replace row on the find bar, alt-cmd-f`.

---

### Task 3: Replace một match và Replace All

**Files:**
- Modify: `crates/lucerna-workbench/src/find_widget.rs`, `crates/lucerna-editor/src/commands.rs` (đổi `mk_patch` thành `pub` và re-export trong `lib.rs` là `lucerna_editor::mk_patch`), `crates/lucerna-editor/src/lib.rs`, `crates/lucerna-settings/assets/default-keybindings.json`
- Test: `crates/lucerna-workbench/tests/find_widget.rs`

**Interfaces:**
- Consumes: Task 1 (`ReplacePattern`, `replace_all`, `replacement_at`); Task 2 (`replace_input`, `replace_visible`, `preserve_case`).
- Produces:
  - `pub fn replace_one(&mut self, cx: &mut Context<Self>)`
  - `pub fn replace_all(&mut self, cx: &mut Context<Self>)`
  - `pub fn mk_patch(edits: &[(Range<usize>, Arc<str>)]) -> lucerna_text::Patch` trong `lucerna_editor`
  - Lệnh `editor.action.replaceOne` (`Replace One`) và `editor.action.replaceAll` (`Replace All`).

- [ ] **Step 1: Viết test hỏng**

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

Viết đủ thân các test có comment theo đúng các assert ghi trong comment. Undo trong test thì gọi `view.update(cx, |v, cx| v.undo(true, cx))`, để không phụ thuộc phím nào có focus.

- [ ] **Step 2: Chạy test, thấy hỏng**

Run: `scripts/prune-target.sh && cargo test -p lucerna-workbench --test find_widget replace`
Expected: FAIL.

- [ ] **Step 3: Cài đặt**

- Trường mới `pending_pick_from: Option<usize>`. Trong `apply`, nếu có thì chọn match đầu tiên có `start >= offset` (không có thì 0), reveal nó, rồi xóa. Cái này đi trước nhánh `Pick::Keep` vốn chạy khi revision đổi.
- `replace_one`:
  - Không làm gì nếu `!can_replace`, `!replace_visible`, `pending`, hoặc `revision` khác revision của view.
  - Lấy `range = matches[active]`. Nếu offset của selection hiện tại (`view.selection()` đổi ra offset) khác `range` thì gọi `self.step(0…)`, tức chọn `nearest` rồi reveal, và dừng.
  - Nếu trùng: build regex bằng `build_regex`, tính `replacement_at` trên text của snapshot. `None` thì dừng.
  - Plan: `EditPlan::new(vec![(range, text.into())], SelectionSet::cursor(range.start + text.len()), EditKind::Other)` với `stop_before = stop_after = true`. Gọi `view.update(|v, cx| v.transact(plan, cx))`.
  - Đặt `pending_pick_from = Some(range.start + text.len())`. Lần tìm do revision đổi sẽ chọn match kế.
- `replace_all`:
  - Cùng các điều kiện chặn như `replace_one`, trừ chuyện selection.
  - Nhỏ hơn hoặc bằng `BACKGROUND_SEARCH_BYTES`: tính `replace_all(...)` ngay.
  - Lớn hơn: `cx.background_spawn` qua `guarded("replace all", …)`, rồi áp dụng chỉ khi `revision` của view chưa đổi. Dùng chung `_search` / `generation` để một lần tìm mới hủy nó.
  - Áp dụng: rỗng thì thôi. Không rỗng thì `edits: Vec<(Range, Arc<str>)>`; `selections = view.selections().clone()` rồi `remap(&mk_patch(&edits))`; tạo `EditPlan` với `EditKind::Other`, `stop_before = stop_after = true`, rồi `transact`.
- Lệnh, đăng ký trong `register_commands`: `editor.action.replaceOne` / `editor.action.replaceAll` gọi hàm trên widget của view active, khi widget đang hiện.
- Keybinding, nối tiếp khối của Task 2:
  ```json
  { "key": "shift+cmd+1", "command": "editor.action.replaceOne", "when": "editorFocus && findWidgetVisible" },
  { "key": "alt+cmd+enter", "command": "editor.action.replaceAll", "when": "editorFocus && findWidgetVisible" },
  { "key": "enter", "command": "editor.action.replaceOne", "when": "editorFocus && replaceInputFocussed" },
  { "key": "cmd+enter", "command": "editor.action.replaceAll", "when": "editorFocus && replaceInputFocussed" }
  ```
- Hai nút ở hàng 2 gọi `replace_one` / `replace_all`.

- [ ] **Step 4: Chạy test, thấy qua**

Run: `cargo test -p lucerna-workbench --test find_widget && cargo test -p lucerna-editor`
Expected: PASS.

- [ ] **Step 5: Commit** các file của task, theo đường dẫn. Message: `feat: replace one and replace all in the find bar`.

---

### Task 4: Phím tắt toggle ⌥⌘C/W/R/P

**Files:**
- Modify: `crates/lucerna-workbench/src/find_widget.rs`, `crates/lucerna-settings/assets/default-keybindings.json`
- Test: `crates/lucerna-workbench/tests/find_widget.rs`

**Interfaces:**
- Consumes: Task 2 (`toggle_preserve_case`); các `toggle_*` có sẵn.
- Produces: lệnh `toggleFindCaseSensitive`, `toggleFindWholeWord`, `toggleFindRegex`, `togglePreserveCase`; getter `pub fn case_sensitive(&self) -> bool`, `whole_word()`, `is_regex()`.

- [ ] **Step 1: Viết test hỏng**

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

- [ ] **Step 2: Chạy test, thấy hỏng** — `cargo test -p lucerna-workbench --test find_widget toggle_keys`, FAIL.

- [ ] **Step 3: Cài đặt**

- Thêm `fn widget_for_toggle(view, cx) -> Entity<FindWidget>`: trả widget có sẵn (dù đang ẩn), không thì tạo `FindWidget::new` rồi gắn vào view mà không hiện. Cần thêm `CodeView::set_find_widget_hidden(find)` hoặc tương đương: đặt `self.find = Some(find)` mà không đổi `find_visible`.
- Bốn lệnh gọi `toggle_*` trên widget đó. Widget đang ẩn thì `search` vẫn chạy nhưng không tô sáng: `apply` chỉ gọi `set_match_highlights` khi `view.find_widget().is_some()`. Kiểm tra lại `apply`: nó gọi vô điều kiện, nên cần thêm điều kiện này.
- Keybinding, nối tiếp khối:
  ```json
  { "key": "alt+cmd+c", "command": "toggleFindCaseSensitive", "when": "editorFocus" },
  { "key": "alt+cmd+w", "command": "toggleFindWholeWord", "when": "editorFocus" },
  { "key": "alt+cmd+r", "command": "toggleFindRegex", "when": "editorFocus" },
  { "key": "alt+cmd+p", "command": "togglePreserveCase", "when": "editorFocus" }
  ```

- [ ] **Step 4: Chạy test, thấy qua** — `cargo test -p lucerna-workbench --test find_widget`, PASS.

- [ ] **Step 5: Commit** các file của task. Message: `feat: find toggle shortcuts alt-cmd-c/w/r/p`.

---

### Task 5: Tài liệu và kiểm tra toàn bộ

**Files:**
- Modify: `docs/keyboard-shortcuts.md` (thêm dòng sau dòng `Cmd+F`: `Alt+Cmd+F` Replace in file, `Shift+Cmd+1` Replace one, `Alt+Cmd+Enter` Replace all, `Alt+Cmd+C/W/R/P` Toggle case / whole word / regex / preserve case)
- Modify: `CHANGELOG.md` (dưới `## [Unreleased]`, thêm mục `### Added` riêng của tính năng này, ngay sau tiêu đề `[Unreleased]`, viết theo giọng các mục cũ)
- Modify: `docs/roadmap.md` (đánh `[x]` cho dòng Find & Replace)
- Modify: `docs/design-principles.md` §1.1 (bỏ câu ví dụ "chẳng hạn bỏ Replace vì Lucerna chỉ để xem code")

- [ ] **Step 1:** Sửa bốn file trên.
- [ ] **Step 2:** `scripts/prune-target.sh && cargo test -p lucerna-workbench && cargo test -p lucerna-project -p lucerna-editor -p lucerna-settings`. Expected: tất cả PASS.
- [ ] **Step 3:** `cargo clippy --workspace --all-targets -- -D warnings` và `cargo fmt --all --check`. Expected: sạch.
- [ ] **Step 4:** Commit bốn file docs, theo đường dẫn. Message: `docs: find and replace shortcuts, changelog, roadmap`.
