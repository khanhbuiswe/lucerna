# Vim V2b (macro và dòng lệnh `:`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Hoàn tất Vim sửa trong Lucerna:
- macro `q{reg}` … `q`, `[count]@{reg}`, `@@`, lưu trong register dưới dạng ký hiệu phím;
- dòng lệnh `:` qua Quick Open (`:w :q :wq :x :wa :qa :q! :w! :N :$ :d :y :> :< :s :noh`, range kiểu ex);
- `vim.gdefault`;
- highlight khi yank (`vim.highlightedyank.enable`, `vim.highlightedyank.duration`).

**Architecture:**
- `lucerna-vim` (thuần, không gpui) có thêm parser và bộ chạy lệnh `:` (`ex.rs`, `ex_run.rs`, `substitute.rs`) và macro (`macros.rs`). Lệnh cần workbench (lưu, đóng tab) đi ra thành `Action::RunEx(ExCommand)`.
- Chạy macro là **phát lại phím**: `Outcome::Macro(Vec<Key>)`. Workbench đưa từng phím vào `feed` (trong Insert thì gõ qua đường gõ), cả lần chạy là một nhóm undo. Khi đang chạy macro, `lucerna-vim` tự gom chữ sau `/`, `?`, `:` tới `<CR>` (không mở giao diện), vì chỉ lúc chạy mới biết phím nào thuộc mode nào.
- Quick Open có `QuickOpenQuery::VimCommand` khi Vim bật và đang ở code view. Kết quả lệnh hiện ở chỗ chỉ báo mode cho tới phím kế tiếp.

**Tech Stack:** Rust 2024, `lucerna-text`, `lucerna-editor`, `lucerna-vim` (+ `regex`, đã có trong workspace), gpui (chỉ workbench).

**Spec:** `docs/superpowers/specs/2026-11-13-vim-editing-design.md`, bước **V2b** ở §12: §6.4 macro, §8 dòng lệnh, `vim.gdefault`, highlight khi yank (§9). Đọc thêm §2 (có mục Helix), §3.1, §10, §11. Plan V2a: `docs/superpowers/plans/2026-11-13-vim-editing-v2a.md` (cùng quy ước test).

## Global Constraints

- **Chữ trong app chỉ tiếng Anh**, đúng nguyên văn spec: `E492: Not an editor command: foo`, `E486: Pattern not found: foo`, `N substitutions on M lines`, `-- NORMAL -- recording @a`, `E: file changed, :s cancelled`, `:%s/foo/bar/g — Replace in all lines`, `:w — Save`, `:12 — Go to line 12`, `Not an editor command: foo`. Tiếng Việt chỉ có trong dữ liệu test.
- **Crate:** chỉ `lucerna-workbench` dùng gpui. `lucerna-vim` được phụ thuộc `lucerna-text`, `lucerna-editor`, `unicode-segmentation` và thêm `regex` (workspace, MIT/Apache).
- **Giấy phép:** không chép code Zed (GPL) hay Helix (MPL), chỉ học thiết kế. Ca test chuyển thể từ VSCodeVim (MIT) ghi `// Adapted from VSCodeVim <path>: "<test title>"`.
- **Vị trí:** `Point.column` và offset là byte UTF-8; không cắt đôi ký tự; Normal không đứng sau ký tự cuối.
- **Sửa:** mọi thay đổi văn bản là một `EditPlan` (edit theo offset trước khi sửa). View chỉ đọc (`edit_ctx() == None`) không nhận `Action::Edit`; `:y` và `y` vẫn chạy.
- **Undo:** một lần `feed` (hay một lệnh `:`) là một bước; cả một lần chạy macro (kể cả `5@a`) là một bước.
- **Settings mới (spec §9):**

  | Khóa | Mặc định |
  |---|---|
  | `vim.gdefault` | `false` |
  | `vim.highlightedyank.enable` | `false` |
  | `vim.highlightedyank.duration` | `200` (ms) |

- **Build:**
  - Cargo cần `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`.
  - Chạy `scripts/prune-target.sh` trước **mọi** lần `cargo test` hay build.
  - Không bao giờ đặt `CARGO_TARGET_DIR` hay `--target-dir`; mọi worktree build vào `target/` chung. Cargo lock đang bị giữ thì chờ.
  - Giữ một profile: xóa `target/release` trước khi test/debug build, trừ khi `pgrep -x cargo` cho thấy cargo khác đang chạy.
- **Git (thư mục dùng chung):**
  - Trước mỗi commit chạy `git status`; có merge đang dở hay file staged không phải của mình thì dừng, báo BLOCKED.
  - Stage và commit theo đường dẫn tường minh: `git commit -m "…" -- <paths>`. Không rebase, không stash, không amend.
  - Trailer: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- `cargo deny check` phải qua.

## Review Focus

1. **Macro có chữ gõ Telex và auto-close.** Ghi `qa` + `ciw` + IME commit `chào` + `Esc` + `q`, rồi `@a` trên từ khác phải ra `chào`, một bước undo. Test: `Task 7: macro_with_vietnamese_ime_text_replays_text`.
2. **Macro đệ quy hoặc count lớn không treo, không tràn stack.** Register `a` chứa `@a` rồi `@a`: dừng ở giới hạn lồng nhau, không panic, không sửa gì. Test: `Task 7: self_calling_macro_stops_at_depth_cap`.
3. **`:%s` trên file trên 1 MB trong lúc agent sửa file.** Revision đổi trước khi kết quả về: không áp gì, status bar báo `E: file changed, :s cancelled`. Test: `Task 6: large_file_substitute_cancelled_by_reload`.
4. **`:wq` khi file đang xung đột với đĩa.** Prompt xung đột hiện; chọn Cancel thì tab **không** đóng và vẫn bẩn. `:w!` ghi đè không hỏi. Test: `Task 6: wq_conflict_cancel_keeps_tab`, `Task 6: w_bang_overwrites_without_prompt`.
5. **`:` khi Vim tắt và khi Vim bật.** Vim tắt: `:-1` vẫn là dòng cuối, `:12,5` là dòng 12 cột 5 (T5-4). Vim bật: `:-1` lên một dòng, `:12,5` là range đảo; không có code view thì `:12` vẫn là go-to-line cũ. Test: `Task 6: colon_dispatch_vim_on_and_off`.

---

## Quyết định chốt trong plan (spec chưa nói rõ)

- **Quick Open chia việc thế nào.** Khi Vim bật **và** editor đang active là code view Vim nhận phím, mọi text sau `:` (cắt khoảng trắng đầu) là `QuickOpenQuery::VimCommand`. Parser ex của V2b hiểu một số đứng riêng (`12`) là đi tới dòng, nên `:12` vẫn ra `:12 — Go to line 12` như spec §8.1, chỉ khác đường chạy. `resolve_line` và nghĩa VS Code của `:-1`, `:12,5` chỉ còn dùng khi Vim tắt hoặc không có code view. Giống Helix: số đứng riêng là `:goto`.
- **`:N` khi Vim bật** chạy như `NG`: nhảy (đặt mark `''`), con trỏ về ký tự không trắng đầu tiên theo `vim.startofline`, dòng vượt quá thì kẹp về dòng cuối.
- **`:s` khớp theo từng dòng** (text của dòng, không có ký tự xuống dòng): `^`/`$` là đầu/cuối dòng; match rỗng được (`:s/^/# /`); mẫu không vượt qua dòng. Mặc định phân biệt hoa thường; `i` thì không, `I` thì có (không đọc `vim.ignorecase`, vì setting đó chỉ cho `*` và `#`).
- **Range trên `:w :q :wq :x :wa :qa :noh`** bị bỏ qua. `!` chỉ được trên `w q wq x`; chỗ khác: `E477: No ! allowed`. `:wq!` và `:x!` lưu ghi đè rồi đóng.
- **Thông báo lỗi khác** (theo Vim): `E16: Invalid range`, `E20: Mark not set`, `E35: No previous regular expression`, `E383: Invalid search string: {pat}`, `E488: Trailing characters: {rest}`, `E354: Invalid register name: '{c}'`.
- **Motion thất bại** báo bằng `Action::Failed` trong `Outcome::Handled` (spec §3.1 "mỗi `Outcome` cho biết motion có thất bại không"). Workbench chỉ dùng nó để dừng macro.
- **Trong lúc chạy macro**, xóa nhiều dòng vẫn ghi `"` và `1` nhưng **không dịch** `1`–`9`, còn `.` và register `.` không bị ghi đè (spec §6.4, đọc đúng chữ).
- **Giới hạn lồng macro:** `MAX_MACRO_DEPTH = 100` (Vim cho macro đệ quy, Helix chặn hẳn). Vượt quá thì dừng cả lần chạy, không báo lỗi.
- **Màu highlight khi yank:** `theme.ui.search_match_background` (khóa Zed `search.match_background`), tương đương `editor.findMatchHighlightBackground` mà spec §9 nói.

---

## File Structure

```
crates/lucerna-vim/
  Cargo.toml          + regex.workspace = true
  src/lib.rs          Action::{Failed, RunEx, OpenCommandLine, Message, Yanked}; Outcome::{Macro, Substitute};
                      ExCommand; Settings.{gdefault, highlighted_yank}; re-exports
  src/ex.rs           NEW  parse lệnh `:`, range, viết tắt, describe_command, ExError
  src/ex_run.rs       NEW  Vim::ex_command: đi tới dòng, :d :y :> :<, lệnh workbench, `:` register
  src/substitute.rs   NEW  :s — dịch mẫu, phần thay, cờ, SubstituteJob
  src/macros.rs       NEW  ký hiệu phím, ghi macro, `@`, dòng lệnh nội bộ khi chạy macro
  src/register.rs     set_last_command, set_macro, hold_ring
  src/state.rs        phím `:` `q` `@`; Action::Failed; hook ghi macro; không ghi `.` khi chạy macro
  src/test_util.rs    parse_keys → macros::parse_notation; mô phỏng Quick Open `:`; Outcome::Macro/Substitute
crates/lucerna-workbench/src/
  quick_open/query.rs   QuickOpenQuery::VimCommand, parse_query_vim
  quick_open/view.rs    Sources.vim_command_line, Row::VimCommand, QuickOpenEvent::VimCommand
  quick_open/mod.rs     show_quick_open điền vim_command_line; event → run_vim_command
  vim.rs                Redo::Ex; chạy ex; message; ExCommand queue; macro loop; yank flash; settings mới
  vim_ex.rs             NEW  Workbench::{run_vim_command, run_vim_requests, run_ex_command}
  vim_indicator.rs      vẽ Indicator (recording, message)
  workbench.rs          gọi run_vim_requests sau dispatch phím
  save.rs, documents.rs SaveOptions.overwrite
  editor_groups/mod.rs, editor_groups/view.rs   workbench.action.closeAllEditors, request_close_all
  code_view/mod.rs, code_view/element.rs        flash_ranges
crates/lucerna-settings/assets/default-settings.json   ba khóa mới
crates/lucerna-workbench/tests/vim_command_line.rs     NEW
crates/lucerna-workbench/tests/vim_editing.rs          + macro, yank highlight
crates/lucerna-workbench/tests/quick_open_query.rs     + parse_query_vim
```

---

### Task 1: Parser lệnh `:` (`ex.rs`)

Spec: §8.2 (bảng lệnh, abbreviation, range), §8.1 (dòng mô tả).

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

Luật parse:
- **Address:** `N`, `.`, `$`, `'x` (`a`–`z`, `<`, `>`), rồi các offset `+N`/`-N`; `+`/`-` không số là 1; `.++1` = `.+2`; offset không có base thì base là `.`. Range: `%`, `a,b`, `a`. Range đảo thì đổi chỗ (không hỏi).
- **Tên lệnh** là dãy chữ ASCII ngay sau range (hoặc dãy `>`/`<`), rồi `!` tùy chọn. Bảng viết tắt `[viết tắt, đầy đủ]`: `w write`, `wa wall`, `q quit`, `qa qall`, `wq wq`, `x xit`, `d delete`, `y yank`, `s substitute`, `noh nohlsearch`. Tên `n` khớp khi `full.starts_with(n) && n.len() >= abbr.len()`.
- **`:d`/`:y`:** khoảng trắng, register tùy chọn (không phải chữ số), khoảng trắng, count tùy chọn (`:y3`, `:ya3`, `:d x 2`). Còn dư thì `E488`.
- **`:s`:** dấu phân cách là ký tự đầu tiên sau `s`, không phải chữ/số/khoảng trắng/`\`/`"`/`|`. `\` + dấu phân cách là dấu phân cách nguyên văn. Dấu phân cách cuối có thể bỏ. Sau đó là cờ `g i I n`, khoảng trắng, count. Cờ lạ thì `E488`.
- Dòng rỗng (chỉ khoảng trắng) parse ra `Goto` với `Implicit`, và `ex_command` coi đó là không làm gì.

`describe_command` (chữ chính xác):

| Dòng | Kết quả |
|---|---|
| `12` | `Ok("Go to line 12")` |
| `$` | `Ok("Go to last line")` |
| range khác không lệnh | `Ok("Go to line")` |
| `w`, `w!`, `wa` | `Save`, `Save, overwriting the file on disk`, `Save all` |
| `q`, `q!`, `qa` | `Close editor`, `Revert and close editor`, `Close all editors` |
| `wq`, `x` | `Save and close editor`, `Save if modified and close editor` |
| `d`, `y`, `>`, `<` | `Delete` / `Yank` / `Indent` / `Outdent` + ` current line` / ` all lines` (`%`) / ` selected lines` (`'<,'>`) / ` lines in range` |
| `s…` | `Replace in current line` / `Replace in all lines` / `Replace in selection` / `Replace in range` |
| `noh` | `Clear search highlight` |
| `foo` | `Err("Not an editor command: foo")` |

- [ ] **Step 1: Viết test hỏng** trong `ex.rs`:
  - `parse("%s/foo/bar/g")` → `All` + `Substitute(New{pattern:"foo", replacement:"bar", flags.global:true})`.
  - `parse("'<,'>d")`, `parse(".+3yan2")` → `Yank{count:Some(2)}` với `One(Address{Current, 3})`; `parse(".++1s/a/b/")` → offset 2; `parse("+2s/a/b/")` → base Current offset 2.
  - `parse("s/\\/\\/f/z/g")` → pattern `//f`. *Adapted from VSCodeVim `test/cmd_line/substitute.test.ts`: "Substitute with escaped delimiter"*.
  - `parse("s g")` → `Repeat{flags.global:true}`; `parse("s/a/b/ 3")` → count 3.
  - `parse("wq!")` → `WriteQuit{bang:true}`; `parse("wa!")` → `Err(NoBangAllowed)`; `parse("no")` → `Err(NotACommand("no"))`; `parse("d x 2 z")` → `Err(TrailingCharacters("z"))`.
  - `resolve`: `3,1` → `0..=2`; `'a` chưa đặt → `MarkNotSet`; `20` trên file 5 dòng → `InvalidRange`, còn với `clamp` → `4..=4`.
  - `ExError::NotACommand("foo").message() == "E492: Not an editor command: foo"`, `PatternNotFound("foo").message() == "E486: Pattern not found: foo"`.
  - Bảng `describe_command` ở trên, từng dòng.
- [ ] **Step 2:** `scripts/prune-target.sh && cargo test -p lucerna-vim ex::` → FAIL (chưa có module).
- [ ] **Step 3:** Cài đặt `ex.rs`.
- [ ] **Step 4:** `cargo test -p lucerna-vim` → PASS.
- [ ] **Step 5:** `git commit -m "feat(vim): ex command line parser" -- crates/lucerna-vim/Cargo.toml crates/lucerna-vim/src/ex.rs crates/lucerna-vim/src/lib.rs Cargo.lock`

---

### Task 2: Chạy lệnh `:` trong `lucerna-vim` (trừ `:s`)

Spec: §3.1 (các `Action` mới), §7 (`:` từ Visual), §8.1, §8.2, §6.2 (register `:`).

**Files:**
- Create: `crates/lucerna-vim/src/ex_run.rs`
- Modify: `src/lib.rs`, `src/state.rs` (phím `:`), `src/register.rs`, `src/test_util.rs`, `crates/lucerna-workbench/src/vim.rs` (match `Action` biên dịch được: biến thể mới là no-op tới Task 6).

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

Hành vi:
- **Phím `:`** trong Normal: `OpenCommandLine { prefill: "" }`; với count `n`: `"."` khi `n == 1`, `".,.+{n-1}"` khi `n > 1`. Trong Visual/Visual Line: kết thúc Visual (mark `<` `>`), về Normal, `prefill: "'<,'>"`. Ghi `prefill` vào `Vim.last_prefill: Option<String>` (Task 4 dùng).
- **`Goto`:** như `{N}G` (nhảy, mark `''`, `startofline`), dòng kẹp.
- **`:d`:** ghi register như `dd` (`WriteKind::Delete`, Linewise), con trỏ ở ký tự không trắng đầu tiên của dòng đứng vào chỗ dòng bị xóa. **`:y`:** như `yy`, con trỏ không đổi. **`:>`/`:<`:** `depth` mức, con trỏ ở ký tự không trắng đầu tiên của dòng cuối range. Count: range mới bắt đầu ở dòng cuối của range, dài `count` dòng, kẹp ở cuối file.
- **Lệnh workbench** trả `Handled(vec![RunEx(…)])`; range bị bỏ qua.
- **Register `:`** ghi khi lệnh chạy xong (kể cả lỗi), trừ dòng rỗng.

Test harness: khi `run_edit` gặp `Action::OpenCommandLine { prefill }`, các phím sau tới `<CR>` gõ vào một dòng lệnh giả bắt đầu bằng `prefill` (`<BS>` xóa, `<Esc>` hủy); `<CR>` gọi `vim.ex_command(line, …)` rồi áp outcome, như mô phỏng `/` đã có.

- [ ] **Step 1: Viết test hỏng** (`assert_edit`):

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

  Ca riêng của Lucerna:
  - `:5<CR>` trên file 10 dòng có thụt lề → con trỏ ở ký tự không trắng đầu tiên dòng 5; rồi `''` quay về.
  - `:999<CR>` → dòng cuối; `:$<CR>` → dòng cuối.
  - `Vj:` phát `OpenCommandLine{prefill:"'<,'>"}` và mode Normal; `3:` phát `prefill: ".,.+2"`.
  - `Vj:>` + `<CR>` thụt hai dòng; `:'a,'b>` khi chưa có mark `a` → actions `[Message("E20: Mark not set"), Failed]`, văn bản không đổi.
  - `:foo<CR>` → `[Message("E492: Not an editor command: foo"), Failed]`.
  - `:w<CR>` → `[RunEx(Save{overwrite:false})]`; `:x!<CR>` → `RunEx(SaveAndClose{only_if_dirty:true, overwrite:true})`; `:noh<CR>` → `RunEx(NoHighlight)`.
  - `:d<CR>` trong view chỉ đọc → không có `Edit`.
  - Sau `:d 2<CR>`, `":p` dán `d 2`.
- [ ] **Step 2:** `cargo test -p lucerna-vim ex_run` → FAIL.
- [ ] **Step 3:** Cài đặt.
- [ ] **Step 4:** `cargo test -p lucerna-vim` và `cargo test -p lucerna-workbench --test vim --test vim_editing` → PASS.
- [ ] **Step 5:** `git commit -m "feat(vim): run ex commands: go to line, :d :y :> :<, workbench commands" -- <các file trên>`

---

### Task 3: `:s` và `vim.gdefault`

Spec: §8.2 mục `:s`, §9 (`vim.gdefault`), §10 (`:%s` trên file lớn).

**Files:**
- Create: `crates/lucerna-vim/src/substitute.rs`
- Modify: `src/lib.rs` (`Settings.gdefault: bool`, mặc định `false`; `Outcome::Substitute`), `src/ex_run.rs`, `src/state.rs` (`VimGlobals.last_substitute`), `src/test_util.rs`, `crates/lucerna-workbench/src/vim.rs` (tạm thời chạy `Outcome::Substitute` đồng bộ: `job.run` rồi `finish_substitute`; Task 6 thêm luồng nền).

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

Luật:
- **Mẫu rỗng** dùng mẫu tìm cuối (`g.last_search`): `is_regex == false` thì `regex::escape`, `whole_word` thì bọc `\b…\b`. Không có thì `E35`. **`:s` lặp lại** (`Repeat`) dùng mẫu và phần thay của `g.last_substitute` với cờ mới; không có thì `E35`.
- **Cờ:** `global = flags.global ^ s.gdefault`. Phân biệt hoa thường theo "Quyết định chốt".
- **Mẫu không biên dịch được** → `E383: Invalid search string: {pat}`.
- **Khớp theo dòng** trong range (mặc định dòng hiện tại); không `g` thì match đầu tiên mỗi dòng; match rỗng được, sau match rỗng tìm tiếp từ ký tự sau.
- **`ex_command`** chạy đồng bộ nếu `snapshot.len_bytes() <= BACKGROUND_SUBSTITUTE_BYTES` hoặc `g.macro_running()` (Task 4; tới lúc đó coi là `false`); ngược lại trả `Outcome::Substitute(job)`, không đổi trạng thái.
- **Sau khi áp** (`finish_substitute`, cũng dùng cho đường đồng bộ): một `Edit` dựng bằng `lucerna_editor::edit::replace_ranges(ctx, edits, cursor, EditKind::Other)`; con trỏ ở cột 0 của dòng chứa cuối phần thay cuối cùng (sau sửa); `g.last_search = (SearchQuery{pattern: dịch rồi, is_regex: true, case_sensitive, whole_word: false}, false)` và `Action::Highlight` như `/`; `g.last_substitute` cập nhật; `Message("{n} substitution{s} on {m} line{s}")` (số ít khi bằng 1). Cờ `n`: không sửa, `Message("{n} match{es} on {m} line{s}")`. Không match: `[Message("E486: Pattern not found: {pat}"), Failed]`, `{pat}` là mẫu như người dùng gõ (hoặc mẫu tìm cuối khi rỗng).
- View chỉ đọc: không `Edit`, vẫn báo số (như cờ `n`).

- [ ] **Step 1: Viết test hỏng.**
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

    Với `gdefault = true` (*cùng file, suite "Effects of gdefault=true"*): `\|aba` + `:%s/a/d<CR>` → `\|dbd` ("Replace all matches in the line"); `:%s/a/d/g<CR>` → `\|dba` ("Replace with `g` flag inverts global flag").
  - Ca riêng của Lucerna:
    - `:%s/foo/bar/g` trên `foo foo\nfoo` → message `3 substitutions on 2 lines`; một `Edit` duy nhất.
    - `:s/x/y/` không match → `[Message("E486: Pattern not found: x"), Failed]`.
    - `:s/(/x/` → `E383: Invalid search string: (`, không panic.
    - `:%s/^/# /` thêm `# ` vào đầu mọi dòng (match rỗng).
    - `:s/chào/\\U&/` trên `xin chào` → `xin CHÀO` (Unicode).
    - Sau `:s/foo/bar/`, `n` tìm `foo` tiếp theo.
    - `Outcome::Substitute` được trả khi `TestView` có text trên `BACKGROUND_SUBSTITUTE_BYTES`; `job.run(&snapshot)` rồi `finish_substitute` cho cùng kết quả như đường đồng bộ.
- [ ] **Step 2:** `cargo test -p lucerna-vim substitute` → FAIL.
- [ ] **Step 3:** Cài đặt. Test harness xử lý `Outcome::Substitute(job)` bằng `job.run` rồi `finish_substitute` ngay.
- [ ] **Step 4:** `cargo test -p lucerna-vim` → PASS.
- [ ] **Step 5:** `git commit -m "feat(vim): :s substitute and vim.gdefault" -- <các file trên>`

---

### Task 4: Macro trong `lucerna-vim`

Spec: §6.4, §3.1 (motion thất bại), §1.2 mục 4.

**Files:**
- Create: `crates/lucerna-vim/src/macros.rs`
- Modify: `src/lib.rs`, `src/state.rs`, `src/register.rs`, `src/ex_run.rs`, `src/motion.rs`/`src/state.rs` (phát `Failed`), `src/test_util.rs`, `crates/lucerna-workbench/src/vim.rs` (lời gọi `record_typed` mới; `Outcome::Macro` tạm thời chạy như `Replay` các `Key` — Task 7 làm đầy đủ).

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

`Outcome::Macro(keys)`: "chạy các phím theo thứ tự, cả lần chạy là một nhóm undo, giữa lúc đó `VimGlobals::set_macro_running(true)`. Phím đến khi Vim ở Insert (trừ `Escape`) thì gõ: `Char('\t')` → `Typed::Tab`, `Char(c)` → `Typed::Text(c)`, `Enter` → `Newline`, `Backspace` → `DeleteLeft`, `Ctrl(_)` bỏ qua. Dừng cả lần chạy khi một phím trả `Action::Failed`. `Macro` lồng (gặp `@` bên trong) chạy tại chỗ."

**Ghi:**
- `q{a-zA-Z}` trong Normal (không pending, không đang chạy macro) bắt đầu ghi; `q` trong Normal hoặc Visual khi đang ghi thì dừng và gọi `set_macro`. `q` với tên khác thì hủy. `q` khi đang chạy macro không làm gì.
- Trong `Vim::feed`, khi đang ghi và không chạy macro: phím được nối vào bằng `push_key`, trừ khi outcome là `Unhandled`, hoặc có `OpenSearch`/`OpenCommandLine` (`/`, `?`, `:` được ghi lúc chạy xong), hoặc là phím `q` dừng ghi.
- `record_typed` nối `push_typed` khi đang ghi và không chạy macro (vẫn ghi cho `.` như V2a).
- `search_submitted` nối `/` (hoặc `?`) + mẫu (thoát `<`) + `<CR>`. Find widget bị đóng bằng Esc thì không ghi gì.
- `ex_command` nối `:` + phần dòng sau `last_prefill` + `<CR>` nếu dòng bắt đầu bằng `last_prefill.take()`; ngược lại (hoặc mở bằng ⌃G) là `:<C-u>` + dòng + `<CR>`.
- Thứ tự: phím `"` + tên register trước `q` không phải là một phần của `q` (giữ như V2a).

**Chạy:**
- `[count]@{r}`: `r` thuộc `a-z A-Z 0-9 " - + * .` (chữ hoa đọc register thường). Đọc bằng `VimGlobals::read_register`, `parse_notation`, lặp `count` lần → `Outcome::Macro`. `g.last_macro = r`. `@@` chạy `last_macro`.
- Register rỗng, hoặc `@@` khi chưa có → `Handled(vec![Failed])`. Tên khác → `[Message("E354: Invalid register name: '{c}'"), Failed]`.
- **Dòng lệnh nội bộ khi chạy macro:** `/`, `?`, `:` không phát `OpenSearch`/`OpenCommandLine` mà mở `Vim.cmdline: Option<CmdLine>` (`CmdKind::Search{backward}` hoặc `Ex`, text bắt đầu bằng prefill). Rồi `Char` thêm chữ, `Backspace` xóa (rỗng thì hủy), `Ctrl('u')` xóa hết, `Escape` hủy (`search_dismissed`), `Enter` chạy: tìm bằng `search_submitted` với query lấy cờ của `g.last_search` (không có thì `is_regex/case_sensitive/whole_word = false`), từ con trỏ; lệnh bằng `ex_command`. `Retry` giữ nguyên `cmdline` (vì `feed` khôi phục trạng thái), nên workbench đưa lại `Enter` là đủ.
- **Khi chạy macro:** `record`/`save_change` không ghi `g.last_change` và register `.`; `Registers.hold_ring` làm lần xóa nhiều dòng ghi `"` và `1` mà không dịch `1`–`9`.

**`Action::Failed`** được phát (cùng các action khác của phím) khi:
- `h l j k w W b B e E ge gE { } %` không di chuyển được con trỏ;
- `f F t T ; ,` không tìm thấy ký tự;
- `n N * #` không có match, hoặc không còn match khi `wrapscan` tắt;
- toán tử bị hủy vì motion hay text object thất bại (`dfz`, `ci"` không có cặp);
- `.` khi chưa có thay đổi nào; lỗi của `ex_command`.
Các test V1/V2a đang chờ `Handled(vec![])` cho các ca này được sửa thành `Handled(vec![Action::Failed])`.

Test harness: `parse_keys` gọi `macros::parse_notation`; `Run::outcome` xử lý `Outcome::Macro(keys)`: `set_macro_running(true)`, `self.key(k, true)` từng phím, dừng khi phím vừa chạy có `Failed`, lồng tối đa 100, cuối cùng `set_macro_running(false)` và `vim.macro_ended()`.

- [ ] **Step 1: Viết test hỏng.**
  - `macros::tests`: `parse_notation("a<lt>b<Esc><C-r>\n<tab>")` → `[Char('a'), Char('<'), Char('b'), Escape, Ctrl('r'), Enter, Char('\t')]`; `push_typed(Text("chào <b>"))` → `"chào <lt>b>"`; với mọi chuỗi đã ghi, `parse_notation` trả lại đúng các phím.
  - *Adapted from VSCodeVim `test/macro.test.ts`* (start, keys, end, title):

    | Start | Keys | End | Title |
    |---|---|---|---|
    | `\|foo = 1\nbar = 'a'\nfoobar = foo + bar` | `qaA;<Esc>Ivar <Esc>qj@a` | `var foo = 1;\nvar\| bar = 'a';\nfoobar = foo + bar` | "Can record and execute" |
    | `\|foo = 1\nbar = 'a'\nfoobar = foo + bar` | `qaA;<Esc>Ivar <Esc>qj@aj@@` | `var foo = 1;\nvar bar = 'a';\nvar\| foobar = foo + bar;` | "Can repeat last invoked macro" |
    | `\|"("+a+","+b+","+c+","+d+","+e+")"` | `f+s + <Esc>qq;.q8@q` | `"(" + a + "," + b + "," + c + "," + d + "," + e +\| ")"` | "Can play back with count" |
    | (như trên) | `f+s + <Esc>qq;.q22@q` | (như trên) | "Can play back with count, abort when a motion fails" |
    | `1. \|one\n2. two\n3. three\n4. four` | `qa0f.r)w~jq3@a` | `1) One\n2) Two\n3) Three\n4) F\|our` | "Repeat change on contiguous lines" |
    | `1. \|one\n2. two\n3. three\n4. four` | `qa0f.r)qqAw~jq3@a` | (như trên) | "Append command to a macro" |
    | `1. \|one\n2. two\n3. three\n4. four` | `qB0f.r)w~jq3@b` | (như trên) | "Append command to a not yet created register creates a new register" |
    | `1. \|one\n2. two\n3. three\n4. four` | `qa0f.r)w~jq3@A` | (như trên) | "Can handle calling an uppercase register" |
    | `1. \|one\n2. two` | `@x` | unchanged | "Can handle calling a non existing macro" |
    | `\|test` ×7 dòng | `qadd.q@a@a` | `\|test` | "Can execute macros with dot commands properly" |
    | `\|one two three` ×3 dòng | `/two<CR>0qqnea XXX<Esc>q5@q` (wrapscan off) | `one two XXX three\none two XXX three\none two XX\|X three` | "Failed `n` stops macro from repeating" |
    | `\|` | `qBione two<Esc>qo<Esc>@b` | `one two\none tw\|o` | "q[A-Z] (action) Creates new register, accessible by [a-z]" |
    | `one t\|wo three` | `@~` | unchanged; message `E354: Invalid register name: '~'` | "Invalid register throws E354" |

  - Ca riêng của Lucerna:
    - `macro_stops_when_motion_fails`: `qajxq` trên 3 dòng, `10@a` dừng ở dòng cuối (vì `j` thất bại), không panic.
    - `macro_with_vietnamese_text`: `qaciwchào<Esc>q` (chữ gõ trong Insert của harness), `w@a` → từ kế tiếp thành `chào`; register `a` = `ciwchào<Esc>`.
    - `"ap` sau khi ghi dán text của macro (Charwise).
    - Chạy text bất kỳ: dòng `dd` được yank vào `a` bằng `"ay$`, rồi `j@a` xóa dòng kế tiếp.
    - `qa/foo<CR>xq`: register `a` = `/foo<CR>x`; `@a` tìm và xóa ở match kế tiếp mà không phát `OpenSearch`.
    - `qa/foo<Esc>xq` (dismiss) → register `a` = `x`.
    - `qa:s/a/b/<CR>jq@a` thay ở dòng kế; register `a` = `:s/a/b/<CR>j`. `qaVj:d<CR>q` → register `a` = `Vj:d<CR>` (prefill `'<,'>` không nằm trong bản ghi; lúc chạy, phím `:` dựng lại nó).
    - `dd` ×3 trong một lần chạy macro không làm `"2`, `"3` đổi; `.` sau macro vẫn lặp thay đổi trước macro.
    - Trong lúc ghi, `VimGlobals::recording() == Some('a')`; sau `q` là `None`.
- [ ] **Step 2:** `cargo test -p lucerna-vim macros` → FAIL.
- [ ] **Step 3:** Cài đặt.
- [ ] **Step 4:** `cargo test -p lucerna-vim` và `cargo test -p lucerna-workbench --test vim --test vim_editing` → PASS.
- [ ] **Step 5:** `git commit -m "feat(vim): macros: record, replay, notation, failed motions" -- <các file trên>`

---

### Task 5: Highlight khi yank

Spec: §9 (`vim.highlightedyank.*`, màu theo theme).

**Files:**
- Modify: `crates/lucerna-vim/src/lib.rs` (`Settings.highlighted_yank: bool`, mặc định `false`; `Action::Yanked(Vec<Range<usize>>)`), `src/edit.rs`, `src/state.rs` (Visual `y`/`Y`), `src/ex_run.rs` (`:y`);
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

- Linewise: từ đầu dòng đầu tới cuối dòng cuối (gồm `\n`); block: `block_ranges`.
- `VimLayer` đọc `vim.highlightedyank.enable` vào `Settings.highlighted_yank` và `vim.highlightedyank.duration` (ms, `u64`, mặc định 200) vào `VimLayer.yank_duration`. `Action::Yanked` → `flash_ranges`. Hẹn giờ bằng `cx.background_executor().timer(duration)`, chỉ xóa khi `flash_generation` chưa đổi.
- `default-settings.json` thêm `"vim.gdefault": false`, `"vim.highlightedyank.enable": false`, `"vim.highlightedyank.duration": 200` (cạnh các khóa `vim.*` ở dòng 139–146).

- [ ] **Step 1: Viết test hỏng.**
  - `lucerna-vim`: với `highlighted_yank`, `yiw` trên `one t|wo` phát `Yanked(vec![4..7])`; `yy` trên `a\nbc` dòng 2 phát `Yanked(vec![2..4])`; tắt thì không phát; `dd` không bao giờ phát.
  - Workbench `yank_flash_follows_settings`: bật `"vim.highlightedyank.enable": true`, `yiw` → `flashed_ranges() == [word]`; `cx.executor().advance_clock(Duration::from_millis(200))` → rỗng. Với `duration: 1000`, sau 200 ms vẫn còn. Mặc định (tắt) → luôn rỗng.
  - Workbench `yank_flash_cleared_by_edit`: `yiw` rồi `x` → rỗng ngay.
- [ ] **Step 2:** `cargo test -p lucerna-vim yanked` và `scripts/prune-target.sh && cargo test -p lucerna-workbench --test vim_editing yank_flash` → FAIL.
- [ ] **Step 3:** Cài đặt.
- [ ] **Step 4:** Chạy lại hai lệnh trên → PASS.
- [ ] **Step 5:** `git commit -m "feat(vim): highlighted yank" -- <các file trên>`

---

### Task 6: Workbench — dòng lệnh `:` qua Quick Open

Spec: §8.1, §8.2, §10 (`:%s` file lớn), §11.2 (dòng lệnh), §13 (xung đột phím `:`).

**Files:**
- Create: `crates/lucerna-workbench/src/vim_ex.rs`, `crates/lucerna-workbench/tests/vim_command_line.rs`
- Modify:
  - `src/quick_open/query.rs`, `src/quick_open/view.rs`, `src/quick_open/mod.rs`;
  - `src/vim.rs`, `src/vim_indicator.rs`, `src/workbench.rs` (dòng ~3004: thay `take_command` bằng `run_vim_requests`), `src/lib.rs` (`mod vim_ex;`);
  - `src/save.rs` (`SaveOptions.overwrite: bool`, 5 chỗ dựng thêm `overwrite: false`), `src/documents.rs` (`save`: `opts.overwrite` bỏ `asks` và gọi `start_write(…, overwrite: true)`);
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
- `show_quick_open` đặt `vim_command_line = vim.enabled() && active_code_view().is_some_and(|v| takes_keys(v.read(cx)))`. `query_changed` dùng `parse_query_vim` khi cờ bật.
- `VimCommand(line)`: rỗng → `Row::Message("Type a Vim command…")`; còn lại một `Row::VimCommand` với `label` = `":{line} — {desc}"` hoặc chữ lỗi của `describe_command` (`valid: false`, vẽ màu `ui.text_muted`). Enter trên cả hai → `QuickOpenEvent::VimCommand(line)` (dòng lỗi cũng chạy, để status bar báo `E492`).
- `on_quick_open_event` → `run_vim_command(line)`: lấy code view active, `vim.run_command_line(&view, line)`, rồi `run_vim_requests`.

VimLayer:
- `Redo::Ex(String)` chạy `vim.ex_command`; đọc clipboard trước khi chạy (như phím có register). `Outcome::Retry` của ex đi đường `search_then_run` như phím.
- `Outcome::Substitute(job)`: nếu đang chạy macro hay replay thì `job.run` ngay; ngược lại chạy `job.run(&snapshot)` ở `background_spawn` (bọc `guarded`), ghi `revision` và `key_generation`; khi xong, revision còn nguyên và không có phím mới thì `finish_substitute` rồi `apply`; revision đã đổi thì `message = "E: file changed, :s cancelled"`.
- `Action::Message(m)` → `self.message = Some(m)`; xóa ở đầu `handle_key`/`feed_keys`. `Action::RunEx(NoHighlight)` → `view.set_match_highlights(Vec::new(), None, cx)`; `RunEx` khác vào `ex_requests`. `Action::OpenCommandLine{prefill}` → `command_line = Some(prefill)`. `Action::Failed` không làm gì ở đây.
- `indicator_text`: có `message` thì chỉ hiện message; ngược lại `{label}`, thêm ` recording @{r}` khi đang ghi, thêm `  {pending}` khi có phím chờ. `vim_indicator.rs` vẽ message cùng font và màu với nhãn mode.
- `read_settings` đọc `vim.gdefault`.

Workbench `run_ex_command`:

| ExCommand | Làm gì |
|---|---|
| `Save{overwrite:false}` | `run_command("workbench.action.files.save")` |
| `Save{overwrite:true}` | lưu file active với `SaveOptions{reason: Explicit, participants: true, overwrite: true}` |
| `SaveAll` | `run_command("workbench.action.files.saveAll")` |
| `Close{revert:false}` | `run_command("workbench.action.closeActiveEditor")` (tab bẩn hiện prompt Phase 4) |
| `Close{revert:true}` | nhớ tab id, chờ `Documents::revert(path)`, rồi đóng tab đó không hỏi |
| `CloseAll` | `run_command("workbench.action.closeAllEditors")` |
| `SaveAndClose{only_if_dirty, overwrite}` | nhớ path và tab id; `only_if_dirty` và file sạch thì đóng ngay; ngược lại chờ `Documents::save(...)`, rồi chỉ đóng khi kết quả `Ok(_)` và file không còn bẩn |

`request_close_all`: không tab nào bẩn thì đóng hết; có thì `save::prompt_unsaved_all` (Save All / Don't Save / Cancel): Save All lưu rồi đóng hết, Don't Save đóng hết như `request_close` làm với Don't Save, Cancel không làm gì.

- [ ] **Step 1: Viết test hỏng.**
  - `tests/quick_open_query.rs`: `parse_query_vim(":%s/a/b/")` → `VimCommand("%s/a/b/")`; `parse_query_vim(":  w")` → `VimCommand("w")`; `parse_query_vim("@x")` == `parse_query("@x")`; `parse_query(":-1")` vẫn là `GotoLine{line: Some(-1), column: None}`.
  - `tests/vim_command_line.rs` (helper chép từ `tests/vim_editing.rs` và `type_text`/`rows` từ `tests/quick_open.rs`):
    - `colon_opens_quick_open_with_rows`: Vim bật, `:` → Quick Open query `":"`, row `Type a Vim command…`; gõ `%s/foo/bar/g` → row `:%s/foo/bar/g — Replace in all lines`; `w` → `:w — Save`; `12` → `:12 — Go to line 12`; `foo` → `Not an editor command: foo`.
    - `colon_dispatch_vim_on_and_off`: Vim tắt, `⌃G`/`:-1` Enter → dòng cuối; `:12,5` → dòng 12 cột 5. Vim bật, con trỏ dòng 10: `:-1` Enter → dòng 9; `:12,5` → con trỏ dòng 12 (range đảo, đi tới dòng cuối của range); không có tab nào mở thì `:12` hiện `Open a file to go to a line`.
    - `visual_colon_prefills_marks`: `Vj:` → query `:'<,'>`; gõ `d` Enter → hai dòng bị xóa, một bước undo.
    - `colon_w_saves`, `colon_q_dirty_shows_prompt` (`cx.has_pending_prompt()`), `colon_q_bang_reverts_and_closes` (file trên đĩa không đổi, tab đóng, không prompt), `colon_wq_saves_and_closes`, `colon_x_clean_closes_without_write` (mtime không đổi), `colon_qa_closes_all`.
    - `w_bang_overwrites_without_prompt`: đổi file trên đĩa (helper `change_on_disk`) khi tab bẩn → `:w!` ghi nội dung của tab, không prompt.
    - `wq_conflict_cancel_keeps_tab`: đổi file trên đĩa → `:wq` → prompt xung đột → `simulate_prompt_answer("Cancel")` → tab còn, vẫn bẩn.
    - `substitute_reports_in_status_bar`: `:%s/a/b/g` trên `a a\na` → text đổi, `indicator_text == "3 substitutions on 2 lines"`; phím `j` kế tiếp → lại `-- NORMAL --`; `u` khôi phục trong một bước.
    - `unknown_command_message`: `:foo` Enter → `E492: Not an editor command: foo`.
    - `large_file_substitute_cancelled_by_reload`: file > 1 MB; `:%s/x/y/g` Enter; trước khi chạy executor, reload file từ đĩa (helper `change_on_disk`); `cx.run_until_parked()` → text là nội dung mới trên đĩa, message `E: file changed, :s cancelled`.
    - `large_file_substitute_runs_in_background`: như trên nhưng không reload → được thay, một bước undo.
    - `noh_clears_highlights`: `/foo<CR>` rồi `:noh` → `match_highlights()` rỗng.
    - `gdefault_setting_inverts_g`: `"vim.gdefault": true`, `:s/a/b/` trên `aaa` → `bbb`.
- [ ] **Step 2:** `scripts/prune-target.sh && cargo test -p lucerna-workbench --test vim_command_line --test quick_open_query` → FAIL.
- [ ] **Step 3:** Cài đặt.
- [ ] **Step 4:** `cargo test -p lucerna-workbench` → PASS (kể cả `tests/quick_open.rs`, `tests/save.rs`, `tests/vim.rs`, `tests/vim_editing.rs`).
- [ ] **Step 5:** `git commit -m "feat(vim): command line through Quick Open" -- <các file trên>`

---

### Task 7: Workbench — chạy và ghi macro

Spec: §6.4, §3.1 (vòng lặp phát lại ở workbench), §11.2 (macro).

**Files:**
- Modify: `crates/lucerna-workbench/src/vim.rs`, `crates/lucerna-workbench/tests/vim_editing.rs`.

**Interfaces:**
- Consumes: Task 4 `Outcome::Macro`, `Action::Failed`, `VimGlobals::{set_macro_running, macro_running, recording}`, `Vim::{record_typed(t, g), macro_ended}`; Task 6 `Indicator`.
- Produces: `const MAX_MACRO_DEPTH: u32 = 100;` và `fn replay(&mut self, view, steps: Vec<ReplayStep>, run: ReplayKind, cx)` với `enum ReplayKind { Dot, Macro }`.

Hành vi:
- `Outcome::Macro(keys)` → `replay(view, keys.map(ReplayStep::Key), ReplayKind::Macro, cx)`. Lần ngoài cùng: `globals.set_macro_running(true)`, `set_typed_events(false)`, `set_suppress_stops(true)` như `.`; cuối cùng `set_macro_running(false)`, `vim.macro_ended()`, bỏ `open_search` và `command_line` còn sót.
- Một bước `Key` khi Vim đang ở Insert và không phải `Escape` thì gõ (bảng ở Task 4) thay vì `feed`. Luật này cũng đúng cho `.` (bước `Key` của `.` trong Insert chỉ có `Escape`).
- `run` ghi `self.last_failed = actions.contains(&Action::Failed)`. Trong `ReplayKind::Macro`, sau mỗi bước có `last_failed` thì dừng cả lần chạy (`replay_stopped`). `Replay` của `.` lồng bên trong macro chạy tại chỗ; `Failed` bên trong nó cũng dừng macro.
- `Macro` lồng tăng `macro_depth`; vượt `MAX_MACRO_DEPTH` thì dừng cả lần chạy.
- `CodeViewEvent::Typed(t)` → `state.vim.record_typed(t, &mut self.globals)`.
- `may_use_register` thêm `'@'`.

- [ ] **Step 1: Viết test hỏng** trong `tests/vim_editing.rs`:
  - `macro_record_and_count_is_one_undo_step`: `qaA;<Esc>jq` trên 4 dòng, `3@a` → 4 dòng có `;`; ngay sau `qa`, `indicator_text == "-- NORMAL -- recording @a"`; trong Insert là `"-- INSERT -- recording @a"`; sau `q` là `-- NORMAL --`; một `u` gỡ cả ba lần chạy.
  - `macro_with_vietnamese_ime_text_replays_text`: `qa`, `ciw`, `replace_and_mark_text_in_range("chà")`, `replace_text_in_range("chào")` (helper `commit`), Esc, `q`; `w@a` → từ kế tiếp thành `chào`.
  - `macro_types_through_auto_close`: `qaci(foo(<Esc>q` rồi `@a` trên `(x)` khác → kết quả giống lần ghi.
  - `macro_search_runs_without_widget`: `qa/foo<CR>xq` (qua find widget thật) rồi `@a` → không có find widget mở, ký tự ở match kế tiếp bị xóa.
  - `macro_stops_when_motion_fails`: `qajxq`, `100@a` trên 5 dòng → dừng ở dòng cuối, không panic.
  - `self_calling_macro_stops_at_depth_cap`: gõ `i@a<Esc>`, `0"ay$` (register `a` = `@a`), `u`; rồi `@a` → không panic, text không đổi, `cx.run_until_parked()` xong.
  - `macro_with_colon_command`: `qa:s/a/b/<CR>jq@a` → dòng 2 được thay; message của lần chạy cuối hiện.
- [ ] **Step 2:** `scripts/prune-target.sh && cargo test -p lucerna-workbench --test vim_editing macro` → FAIL.
- [ ] **Step 3:** Cài đặt.
- [ ] **Step 4:** `cargo test -p lucerna-workbench` → PASS.
- [ ] **Step 5:** `git commit -m "feat(vim): run and record macros in the workbench" -- crates/lucerna-workbench/src/vim.rs crates/lucerna-workbench/tests/vim_editing.rs`

---

### Task 8: Tài liệu, notices, kiểm tra toàn bộ

Spec: §12 (đoạn cuối).

**Files:**
- `docs/vim.md`: bỏ câu "Macros and `:` commands are planned (V2b)" và mục lệch "macros (`q`, `@`) and `:` commands are V2b". Thêm bảng **Macros and command line (V2b)**: `q{a-z}` / `q{A-Z}` / `q`, `[count]@{reg}`, `@@`, `:` (Quick Open; `'<,'>` từ Visual; `3:`), danh sách lệnh §8.2, range, cờ `:s`, các thông báo. Ghi các chỗ lệch: macro ghi `Completion` gần đúng (khi Phase 4.5 có), phím mũi tên trong Insert không vào macro, `:s` khớp theo dòng và mặc định phân biệt hoa thường, `:N` khi Vim bật theo nghĩa Vim (`:-1` là lên một dòng). Thêm ba setting vào bảng settings.
- `docs/features.md` dòng 96–97: thêm "macros, `:` commands (`:w`, `:q`, `:%s`…)".
- `docs/superpowers/specs/2026-11-08-lucerna-phase1-design.md` §1.3 dòng V2: "**V2a và V2b đã xong**" (bỏ "V2b … làm tiếp sau đợt sửa lỗi P3").
- `docs/superpowers/specs/2026-11-13-vim-editing-design.md`: dòng trạng thái thành `V2a và V2b đã triển khai`.
- `README.md`: nếu mục Vim liệt kê tính năng, thêm macro và `:`.
- `THIRD_PARTY_NOTICES.md` mục "VSCodeVim test cases": thêm `test/macro.test.ts`, `test/cmd_line/substitute.test.ts`, `test/cmd_line/delete.test.ts`, `test/cmd_line/yank.test.ts` (đối chiếu `grep -rn "Adapted from VSCodeVim" crates/lucerna-vim`).

Steps:
- [ ] **Step 1:** Sửa tài liệu.
- [ ] **Step 2:** Chạy từng lệnh và đối chiếu:

  | Lệnh | Kết quả mong đợi |
  |---|---|
  | `scripts/prune-target.sh` | — |
  | `cargo fmt --all -- --check` | không in gì |
  | `cargo clippy --workspace --all-targets -- -D warnings` | sạch |
  | `cargo test --workspace` | tất cả pass |
  | `cargo deny check` | `advisories ok, bans ok, licenses ok, sources ok` |

- [ ] **Step 3:** `git status`, rồi `git commit -m "docs: vim V2b macros and command line" -- docs/vim.md docs/features.md docs/superpowers/specs/2026-11-08-lucerna-phase1-design.md docs/superpowers/specs/2026-11-13-vim-editing-design.md README.md THIRD_PARTY_NOTICES.md`
- [ ] **Step 4:** `pgrep -x cargo` không có cargo khác, rồi `scripts/bundle-app.sh --install`. Kết quả: dòng cuối `bundle-app: installed /Applications/Lucerna.app`. Kiểm tra tay theo spec §11.3: `:` so với command palette của Zed; macro gõ Telex.
