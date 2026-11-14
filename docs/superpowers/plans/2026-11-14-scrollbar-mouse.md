# Thanh cuộn dùng chuột: plan triển khai

> **Dành cho agent:** BẮT BUỘC dùng superpowers:subagent-driven-development (khuyên dùng) hoặc superpowers:executing-plans để làm plan này theo từng task. Mỗi bước có checkbox (`- [ ]`) để theo dõi.

**Mục tiêu:** Kéo thumb và bấm rãnh để cuộn trong editor và Explorer, thumb đổi màu khi rê chuột và khi kéo, qua một component `Scrollbar` dùng chung.

**Kiến trúc:** Component là một element `canvas` đặt chồng lên mép phải của nội dung. Lúc prepaint, nó hỏi một `ScrollbarSource` để biết viewport, content và scroll_top. Lúc paint, nó vẽ thumb và đăng ký listener chuột ở cấp window, theo mẫu minimap. Trạng thái hover và kéo nằm trong `ScrollbarState` (`Rc<RefCell<..>>`) do view sở hữu giữ. Có hai nguồn cuộn:
- `CodeView` quy đổi pixel sang dòng hiển thị, có tính dòng bị gập;
- `ScrollHandleSource` bọc `gpui::ScrollHandle`, dùng cho Explorer và về sau cho mọi `div` cuộn được.

**Công nghệ:** Rust, gpui (rev pin trong `Cargo.toml` ở thư mục gốc).

**Spec:** `docs/superpowers/specs/2026-11-14-scrollbar-mouse-design.md`. Research: `docs/research/2026-11-14-scrollbar-mouse.md`.

## Ràng buộc chung

- Chỉ `lucerna-workbench` được dùng gpui. `lucerna-theme` không phụ thuộc gpui.
- Không chép code từ crate `ui` của Zed (GPL).
- Thumb vẽ rộng `SCROLLBAR_WIDTH` = 6 px sát mép phải. Vùng nhận chuột rộng `SCROLLBAR_HIT_WIDTH` = 12 px.
- Thumb ngắn nhất `MIN_THUMB_HEIGHT` (đã có).
- Key theme mới: `scrollbar.thumb.hover_background` và `scrollbar.thumb.active_background`.
- Setting `editor.scrollbar.scrollByPage`: bool, mặc định `false`, chỉ áp dụng cho editor.
- Khi kéo, mỗi lần chuột di chuyển chỉ đổi vị trí cuộn rồi notify.
- Cargo:
  - `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`;
  - `export CARGO_TARGET_DIR=$PWD/target-wt` (thư mục target riêng cho worktree);
  - chạy `scripts/prune-target.sh` trước mỗi lần build hoặc test;
  - giữ dung lượng trống trên ~1.2 GB.
- Commit: stage từng file theo đường dẫn, dùng `git commit -- <paths>`, trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **Kéo thumb ra ngoài cửa sổ hoặc quá hai đầu rãnh.** Vị trí cuộn bị kẹp trong `0..=max`, không panic. Test: `drag_past_track_ends_clamps` (Task 1).
2. **Bấm vào thanh cuộn.** Không đổi vùng chọn và không di chuyển con trỏ trong editor. Test: `scrollbar_click_keeps_selection` (Task 4).
3. **File có dòng bị gập.** Kéo thumb xuống đáy thì dòng cuối nằm trên cùng. Bấm giữa rãnh thì ra dòng hiển thị ở giữa, không phải dòng buffer ở giữa. Test: `track_click_with_folds_lands_on_visible_row` (Task 4).
4. **Thả chuột ngoài thanh cuộn.** Trạng thái kéo kết thúc. Lần di chuột sau, không giữ nút, không cuộn nữa. Test: `drag_ends_on_mouse_up_anywhere` (Task 3).
5. **Minimap vẫn bấm và kéo được.** Thanh cuộn nằm ngay bên trái minimap, nên dải 12 px không được che minimap. Test: `minimap_still_scrolls_next_to_scrollbar` (Task 4).

---

### Task 1: Các hàm tính toán thuần

**Files:**
- Sửa: `crates/lucerna-workbench/src/ui.rs` (cạnh `thumb_geometry`, test ở `mod tests` cuối file)

**Interfaces:**
- Produces:
  - `pub const SCROLLBAR_HIT_WIDTH: f32 = 12.0;`
  - `pub(crate) fn scroll_top_for_thumb(viewport: f32, content: f32, thumb_top: f32) -> f32`: hàm ngược của `thumb_geometry`. `thumb_top` bị kẹp trong `0..=viewport - height`. Trả về 0 khi không tràn.
  - `pub(crate) fn scroll_top_for_track_click(viewport: f32, content: f32, scroll_top: f32, y: f32, by_page: bool) -> f32`:
    - `y` tính từ đỉnh rãnh;
    - khi `by_page == false`, thumb căn giữa vào `y`, nghĩa là `scroll_top_for_thumb(.., y - height / 2)`;
    - khi `by_page == true`, cộng hoặc trừ `viewport` tùy `y` nằm trên hay dưới thumb, rồi kẹp trong `0..=content - viewport`.

- [ ] **Bước 1: Viết test thất bại**

```rust
#[test]
fn thumb_position_maps_back_to_scroll() {
    // 200 tall, 400 content: thumb 100 tall, 100 px of travel for 200 px of scroll.
    assert_eq!(scroll_top_for_thumb(200., 400., 0.), 0.);
    assert_eq!(scroll_top_for_thumb(200., 400., 50.), 100.);
    assert_eq!(scroll_top_for_thumb(200., 400., 100.), 200.);
    // Round trip with thumb_geometry.
    assert_eq!(thumb_geometry(200., 400., 80.), Some((40., 100.)));
    assert_eq!(scroll_top_for_thumb(200., 400., 40.), 80.);
    assert_eq!(scroll_top_for_thumb(100., 50., 10.), 0.);
}

#[test]
fn drag_past_track_ends_clamps() {
    assert_eq!(scroll_top_for_thumb(200., 400., -30.), 0.);
    assert_eq!(scroll_top_for_thumb(200., 400., 500.), 200.);
}

#[test]
fn track_click_centers_the_thumb_or_pages() {
    // Click at y=150: thumb (100 tall) centered there, so its top is 100: the end.
    assert_eq!(scroll_top_for_track_click(200., 400., 0., 150., false), 200.);
    // Click at y=60 with the thumb at 0..100 is on the thumb's span; jump mode still centers: top 10.
    assert_eq!(scroll_top_for_track_click(200., 400., 0., 60., false), 20.);
    // Paged: below the thumb, one viewport down, clamped; above it, one up.
    assert_eq!(scroll_top_for_track_click(200., 1000., 0., 190., true), 200.);
    assert_eq!(scroll_top_for_track_click(200., 1000., 700., 5., true), 500.);
    assert_eq!(scroll_top_for_track_click(200., 1000., 700., 199., true), 800.);
}
```

- [ ] **Bước 2: Chạy test, thấy thất bại**

Chạy: `cargo test -p lucerna-workbench --lib ui::tests`
Kết quả mong đợi: lỗi biên dịch "cannot find function `scroll_top_for_thumb`".

- [ ] **Bước 3: Viết `SCROLLBAR_HIT_WIDTH`, `scroll_top_for_thumb`, `scroll_top_for_track_click`** theo đúng chữ ký ở trên.

- [ ] **Bước 4: Chạy lại, thấy test qua**

Chạy: `cargo test -p lucerna-workbench --lib ui::tests`. Kết quả mong đợi: PASS.

- [ ] **Bước 5: Commit** `feat(ui): scrollbar thumb to scroll position math`

---

### Task 2: Màu thumb khi rê chuột và khi kéo

**Files:**
- Sửa: `crates/lucerna-theme/src/lib.rs`: thêm trường vào `UiColors` và vào `fallback`
- Sửa: `crates/lucerna-theme/src/schema.rs:117`
- Test: `crates/lucerna-theme/tests/theme.rs`, cạnh `scrollbar_thumb_from_theme_or_fallback`

**Interfaces:**
- Produces:
  - `UiColors.scrollbar_thumb_hover: Rgba`, đọc từ `scrollbar.thumb.hover_background`;
  - `UiColors.scrollbar_thumb_active: Rgba`, đọc từ `scrollbar.thumb.active_background`.
  - Khi theme thiếu key, fallback là màu `scrollbar_thumb` thực tế của theme đó (lấy từ theme nếu có, không thì fallback) với alpha nhân 1.6 cho hover và 2.2 cho active, kẹp ≤ 1.0.

- [ ] **Bước 1: Viết test thất bại** `scrollbar_thumb_hover_and_active_from_theme_or_derived`. Kiểm tra:
  - một theme khai báo đủ hai key: lấy đúng giá trị;
  - một theme chỉ có `scrollbar.thumb.background` = `#80808040`: `hover.a` ≈ 0x40/255 × 1.6, `active.a` ≈ 0x40/255 × 2.2, giữ nguyên `r`, `g`, `b`;
  - một theme có `scrollbar.thumb.background` = `#808080cc`: `active.a == 1.0`.
- [ ] **Bước 2: Chạy, thấy thất bại.** Chạy `cargo test -p lucerna-theme scrollbar_thumb`. Kết quả mong đợi: lỗi biên dịch, chưa có trường mới.
- [ ] **Bước 3: Thêm hai trường và đọc key** trong `schema.rs`, tính màu dẫn xuất sau khi đã có `scrollbar_thumb`.
- [ ] **Bước 4: Chạy lại, thấy qua.** Chạy `cargo test -p lucerna-theme`. Kết quả mong đợi: PASS.
- [ ] **Bước 5: Commit** `feat(theme): scrollbar thumb hover and active colors`

---

### Task 3: Component `Scrollbar` và gắn vào Explorer

**Files:**
- Tạo: `crates/lucerna-workbench/src/scrollbar.rs`. Khai báo `mod scrollbar;` trong `lib.rs`, rồi `pub use crate::scrollbar::{Scrollbar, ScrollbarSource, ScrollbarState, ScrollHandleSource, ScrollMetrics};` trong `ui.rs`.
- Sửa: `crates/lucerna-workbench/src/explorer/view.rs`:
  - bỏ `struct ScrollbarThumb` (dòng ~1339–1385) và `.with_decoration(..)`;
  - thêm trường `scrollbar: ScrollbarState`;
  - thêm `pub fn scrollbar_thumb(&self) -> Option<Bounds<Pixels>>`.
- Test: `crates/lucerna-workbench/tests/explorer_view.rs`. Sửa `scrollbar_thumb_only_when_list_overflows` để đọc `ExplorerView::scrollbar_thumb()` thay vì debug selector `explorer-scrollbar-thumb`.

**Interfaces:**
- Consumes:
  - `thumb_geometry`, `scroll_top_for_thumb`, `scroll_top_for_track_click`, `SCROLLBAR_WIDTH`, `SCROLLBAR_HIT_WIDTH` (Task 1);
  - `ui.scrollbar_thumb`, `ui.scrollbar_thumb_hover`, `ui.scrollbar_thumb_active` (Task 2).
- Produces:

```rust
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollMetrics { pub viewport: f32, pub content: f32, pub scroll_top: f32 }

pub trait ScrollbarSource: 'static {
    /// Đọc lúc prepaint, sau khi nội dung đã layout trong frame này.
    fn metrics(&self, cx: &mut App) -> Option<ScrollMetrics>;
    /// Đặt vị trí cuộn mới (đã kẹp) và notify view sở hữu.
    fn set_scroll_top(&self, scroll_top: f32, cx: &mut App);
    /// Bấm rãnh thì lật trang thay vì nhảy.
    fn track_click_pages(&self, _cx: &App) -> bool { false }
}

/// Hover, kéo, và thumb đã vẽ lần cuối; clone rẻ, giữ trong view.
#[derive(Clone, Default)]
pub struct ScrollbarState(/* Rc<RefCell<..>> */);
impl ScrollbarState {
    pub fn thumb(&self) -> Option<Bounds<Pixels>>;
    pub fn is_hovered(&self) -> bool;
    pub fn is_dragging(&self) -> bool;
}

/// Bọc `gpui::ScrollHandle`; `view` được notify sau khi cuộn.
pub struct ScrollHandleSource { pub handle: gpui::ScrollHandle, pub view: gpui::EntityId }

pub struct Scrollbar;
impl Scrollbar {
    /// Một dải SCROLLBAR_HIT_WIDTH, cao hết cha, đặt absolute top_0 right_0.
    /// Cha phải là `.relative()`.
    pub fn new(state: ScrollbarState, source: impl ScrollbarSource, owner: gpui::EntityId) -> impl IntoElement;
}
```

- **Cách xử lý chuột.** Trong canvas, hitbox dùng `HitboxBehavior::BlockMouse` và con trỏ `CursorStyle::Arrow`. Listener chuột đăng ký ở cấp window lúc paint, như `code_view/minimap.rs`.
  - **MouseDown trái trên hitbox:**
    - trúng thumb: lưu `grab = y − thumb.top`;
    - trúng rãnh: gọi `set_scroll_top(scroll_top_for_track_click(..))`, rồi lưu `grab = thumb_height / 2` (khi lật trang thì chưa bắt đầu kéo);
    - sau đó gọi `stop_propagation`.
  - **MouseMove khi đang kéo**, chuột ở bất kỳ đâu: `set_scroll_top(scroll_top_for_thumb(.., y − track_top − grab))`.
  - **MouseUp ở bất kỳ đâu:** kết thúc kéo, notify `owner`.
  - **MouseMove nói chung:** bật hoặc tắt hover theo vị trí so với dải 12 px, và chỉ notify `owner` khi trạng thái thật sự đổi.
- **Màu thumb:** đang kéo thì dùng màu active; đang rê chuột thì dùng màu hover; còn lại dùng màu thường.
- **Nguồn cuộn của Explorer:** `ScrollHandleSource` lấy từ `self.scroll.0.borrow().base_handle.clone()`:
  - `viewport = handle.bounds().size.height`;
  - `content = viewport + max_offset.y`;
  - `scroll_top = −offset.y`.
- **Vị trí trong layout:** bọc `uniform_list` trong một div `.relative()`, với `Scrollbar::new(..)` là con thứ hai, đứng sau list.

- [ ] **Bước 1: Viết test thất bại** trong `explorer_view.rs`. Dùng thư mục có 300 file `f{i:03}.rs`, như test hiện có.
  - `explorer_scrollbar_drag_scrolls`: nhấn chuột ở giữa thumb, kéo xuống đúng `list.height − thumb.height` px, thả.
    - Thumb lúc này có `bottom()` ≈ `list.bottom()` (±0.5 px).
    - `explorer-row-f299.rs` có debug bounds.
    - `explorer-row-f000.rs` thì không.
  - `explorer_track_click_jumps`: bấm ở `(list.right() − 3, list.bottom() − 2)`. Thumb dịch tới đáy, `bottom()` ≈ `list.bottom()`.
  - `drag_ends_on_mouse_up_anywhere`:
    - nhấn trên thumb;
    - kéo 20 px;
    - thả chuột ở `list.left() + 5`, ngoài dải thanh cuộn;
    - di chuột không giữ nút thêm 100 px.
    - Thumb top không đổi sau lần di chuột cuối.
  - `scrollbar_hover_tracks_mouse`:
    - di chuột vào `(list.right() − 10, thumb.center().y)`: `explorer.read(cx).scrollbar_hovered()` là `true` (thêm accessor `pub fn scrollbar_hovered(&self) -> bool`, chuyển tiếp tới `ScrollbarState::is_hovered`);
    - di ra `list.left() + 5`: là `false`.
  - Sửa `scrollbar_thumb_only_when_list_overflows`: dùng `scrollbar_thumb()`, giữ nguyên các assert về chiều rộng 6 px và mép phải.
- [ ] **Bước 2: Chạy, thấy thất bại.** Chạy `cargo test -p lucerna-workbench --test explorer_view scrollbar`. Kết quả mong đợi: lỗi biên dịch hoặc FAIL.
- [ ] **Bước 3: Viết `scrollbar.rs`** đúng các interface ở trên, rồi gắn vào Explorer.
- [ ] **Bước 4: Chạy lại, thấy qua.** Chạy `cargo test -p lucerna-workbench --test explorer_view`. Kết quả mong đợi: PASS, kể cả các test Explorer cũ như click hàng và context menu.
- [ ] **Bước 5: Commit** `feat(ui): shared mouse scrollbar, used by the Explorer`

---

### Task 4: Thanh cuộn của editor

**Files:**
- Sửa: `crates/lucerna-workbench/src/code_view/layout.rs`: thêm `Folds::nth_visible`, kèm unit test.
- Sửa: `crates/lucerna-workbench/src/code_view/mod.rs`:
  - thêm `Config.scroll_by_page`, đọc từ `editor.scrollbar.scrollByPage`, mặc định `false`;
  - thêm trường `scrollbar: ScrollbarState`;
  - render `Scrollbar::new` trong div `code-text-area` (đã là `.relative()`), sau `CodeElement`;
  - `scrollbar_thumb()` trả về `self.scrollbar.thumb()`.
- Sửa: `crates/lucerna-workbench/src/code_view/element.rs:681-703` và `element/paint.rs:185-188`: bỏ việc tính và vẽ thumb. Bỏ `Frame.thumb` nếu không còn dùng. Giữ nguyên các vạch đánh dấu.
- Sửa: `crates/lucerna-settings/assets/default-settings.json`: `"editor.scrollbar.scrollByPage": false`.
- Test: `crates/lucerna-workbench/tests/code_view.rs`.

**Interfaces:**
- Consumes: `Scrollbar`, `ScrollbarState`, `ScrollbarSource`, `ScrollMetrics` (Task 3).
- Produces:
  - `Folds::nth_visible(&self, n: u32) -> u32`: dòng buffer của dòng hiển thị thứ `n`, ngược với `row − hidden_before(row)`.
  - `CodeViewScroll(WeakEntity<CodeView>)`: hiện thực `ScrollbarSource`.
    - `metrics`: cùng công thức với `element.rs:681-694` hiện nay, với `viewport` là chiều cao vùng text đã layout lần cuối, lấy từ `frame.text_bounds` hoặc `frame.hitbox`.
    - `set_scroll_top(px)`:
      - `n = floor(px / lh)`, `top_row = folds.nth_visible(n)`;
      - `offset = (px / lh − n) × wrap_count(top_row) × lh`;
      - gọi `scroll_by(0.)` để kẹp;
      - gọi `report_scroll`, rồi `cx.notify()`.
    - `track_click_pages`: trả về `config.scroll_by_page`.

- [ ] **Bước 1: Viết test thất bại**
  - Unit test trong `layout.rs`, `nth_visible_skips_folds`: với fold đóng ẩn các dòng 3..10 (header 2), kiểm tra
    - `nth_visible(0) == 0`;
    - `nth_visible(2) == 2`;
    - `nth_visible(3) == 10`;
    - `nth_visible(5) == 12`.
  - Trong `tests/code_view.rs`, file 500 dòng `line {i}`:
    - `editor_scrollbar_drag_scrolls`: kéo thumb từ giữa thumb xuống `viewport.height − thumb.height` px. Kết quả: `scroll_top_row() == 499`, thumb chạm đáy (±0.01 px), giống test `scrollbar_thumb_tracks_scroll` hiện có.
    - `editor_track_click_jumps`: bấm ở `(viewport.width − 3, viewport.height / 2)`. Kết quả: `scroll_top_row()` nằm trong `249 ± 1`.
    - `scroll_by_page_setting_pages`: dùng `init(cx, Some(r#"{ "editor.scrollbar.scrollByPage": true }"#))`, bấm gần đáy rãnh. Kết quả: `scroll_top_row() == visible_rows()`, ±1 dòng.
    - `track_click_with_folds_lands_on_visible_row`:
      - file có hàm lớn để gập, hoặc gọi `fold_at` một vùng ẩn khoảng 200 dòng ở đầu;
      - bấm giữa rãnh;
      - dòng trên cùng là dòng hiển thị ở giữa, tức `top_row − hidden_before(top_row)` ≈ (số dòng hiển thị − 1) / 2, ±1;
      - kéo xuống đáy thì `scroll_top_row() == 499`.
    - `scrollbar_click_keeps_selection`: đặt con trỏ ở (0,0), bấm rồi kéo thumb. Kết quả: vùng chọn chính vẫn là `(0,0)..(0,0)`.
    - `minimap_still_scrolls_next_to_scrollbar`: bật minimap rồi bấm ở `minimap.left() + 2`. Kết quả: vẫn cuộn như test minimap hiện có, tức `scroll_top_row()` đổi.
  - Giữ hai test `scrollbar_thumb_tracks_scroll` và `scrollbar_thumb_is_left_of_the_minimap`: hai test này phải qua mà không cần sửa.
- [ ] **Bước 2: Chạy, thấy thất bại.** Chạy `cargo test -p lucerna-workbench --test code_view scrollbar` và `cargo test -p lucerna-workbench --lib nth_visible`. Kết quả mong đợi: FAIL hoặc lỗi biên dịch.
- [ ] **Bước 3: Viết code** theo interface ở trên.
- [ ] **Bước 4: Chạy lại, thấy qua.** Chạy `cargo test -p lucerna-workbench`. Kết quả mong đợi: tất cả PASS.
- [ ] **Bước 5: Kiểm tra toàn bộ.**
  - `cargo clippy -p lucerna-workbench -p lucerna-theme --all-targets -- -D warnings`
  - `cargo deny check`
  - Kết quả mong đợi: sạch.
- [ ] **Bước 6: Commit** `feat(editor): drag the scrollbar thumb and click the track`
