# Mouse scrollbar: implementation plan

> **For agents:** REQUIRED: use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to carry out this plan task by task. Each step has a checkbox (`- [ ]`) for tracking.

**Goal:** Drag the thumb and click the track to scroll in the editor and the Explorer, with the thumb changing color on hover and while dragging, through one shared `Scrollbar` component.

**Architecture:** The component is a `canvas` element laid over the right edge of the content. At prepaint it asks a `ScrollbarSource` for the viewport, content and scroll_top. At paint it draws the thumb and registers mouse listeners at window level, following the minimap pattern. Hover and drag state live in `ScrollbarState` (`Rc<RefCell<..>>`) held by the owning view. There are two scroll sources:
- `CodeView` converts pixels to visible rows, accounting for folded lines;
- `ScrollHandleSource` wraps `gpui::ScrollHandle`, used for the Explorer and later for every scrollable `div`.

**Tech:** Rust, gpui (rev pinned in the root `Cargo.toml`).

**Spec:** `docs/superpowers/specs/2026-11-14-scrollbar-mouse-design.md`. Research: `docs/research/2026-11-14-scrollbar-mouse.md`.

## Global Constraints

- Only `lucerna-workbench` may use gpui. `lucerna-theme` does not depend on gpui.
- Do not copy code from Zed's `ui` crate (GPL).
- The thumb is drawn `SCROLLBAR_WIDTH` = 6 px wide against the right edge. The mouse hit area is `SCROLLBAR_HIT_WIDTH` = 12 px wide.
- Shortest thumb is `MIN_THUMB_HEIGHT` (already exists).
- New theme keys: `scrollbar.thumb.hover_background` and `scrollbar.thumb.active_background`.
- Setting `editor.scrollbar.scrollByPage`: bool, default `false`, applies to the editor only.
- While dragging, each mouse move only changes the scroll position and then notifies.
- Cargo:
  - `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`;
  - `export CARGO_TARGET_DIR=$PWD/target-wt` (a separate target folder for the worktree);
  - run `scripts/prune-target.sh` before every build or test;
  - keep free disk space above ~1.2 GB.
- Commit: stage each file by path, use `git commit -- <paths>`, trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **Dragging the thumb outside the window or past both ends of the track.** The scroll position is clamped to `0..=max`, no panic. Test: `drag_past_track_ends_clamps` (Task 1).
2. **Clicking the scrollbar.** It does not change the selection and does not move the cursor in the editor. Test: `scrollbar_click_keeps_selection` (Task 4).
3. **A file with folded lines.** Dragging the thumb to the bottom puts the last line on top. Clicking the middle of the track lands on the middle visible row, not the middle buffer row. Test: `track_click_with_folds_lands_on_visible_row` (Task 4).
4. **Releasing the mouse outside the scrollbar.** The drag state ends. On the next mouse move, without a button held, there is no more scrolling. Test: `drag_ends_on_mouse_up_anywhere` (Task 3).
5. **The minimap can still be clicked and dragged.** The scrollbar sits right to the left of the minimap, so the 12 px strip must not cover the minimap. Test: `minimap_still_scrolls_next_to_scrollbar` (Task 4).

---

### Task 1: Pure computation functions

**Files:**
- Modify: `crates/lucerna-workbench/src/ui.rs` (next to `thumb_geometry`, tests in `mod tests` at the end of the file)

**Interfaces:**
- Produces:
  - `pub const SCROLLBAR_HIT_WIDTH: f32 = 12.0;`
  - `pub(crate) fn scroll_top_for_thumb(viewport: f32, content: f32, thumb_top: f32) -> f32`: the inverse of `thumb_geometry`. `thumb_top` is clamped to `0..=viewport - height`. Returns 0 when there is no overflow.
  - `pub(crate) fn scroll_top_for_track_click(viewport: f32, content: f32, scroll_top: f32, y: f32, by_page: bool) -> f32`:
    - `y` is measured from the top of the track;
    - when `by_page == false`, the thumb is centered on `y`, meaning `scroll_top_for_thumb(.., y - height / 2)`;
    - when `by_page == true`, add or subtract `viewport` depending on whether `y` is above or below the thumb, then clamp to `0..=content - viewport`.

- [ ] **Step 1: Write the failing tests**

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

- [ ] **Step 2: Run the tests, see them fail**

Run: `cargo test -p lucerna-workbench --lib ui::tests`
Expected: compile error "cannot find function `scroll_top_for_thumb`".

- [ ] **Step 3: Write `SCROLLBAR_HIT_WIDTH`, `scroll_top_for_thumb`, `scroll_top_for_track_click`** with exactly the signatures above.

- [ ] **Step 4: Run again, see the tests pass**

Run: `cargo test -p lucerna-workbench --lib ui::tests`. Expected: PASS.

- [ ] **Step 5: Commit** `feat(ui): scrollbar thumb to scroll position math`

---

### Task 2: Thumb color on hover and while dragging

**Files:**
- Modify: `crates/lucerna-theme/src/lib.rs`: add fields to `UiColors` and to `fallback`
- Modify: `crates/lucerna-theme/src/schema.rs:117`
- Test: `crates/lucerna-theme/tests/theme.rs`, next to `scrollbar_thumb_from_theme_or_fallback`

**Interfaces:**
- Produces:
  - `UiColors.scrollbar_thumb_hover: Rgba`, read from `scrollbar.thumb.hover_background`;
  - `UiColors.scrollbar_thumb_active: Rgba`, read from `scrollbar.thumb.active_background`.
  - When the theme lacks a key, the fallback is that theme's actual `scrollbar_thumb` color (taken from the theme if present, otherwise the fallback) with alpha multiplied by 1.6 for hover and 2.2 for active, clamped to ≤ 1.0.

- [ ] **Step 1: Write the failing test** `scrollbar_thumb_hover_and_active_from_theme_or_derived`. It checks:
  - a theme declaring both keys: gets exactly those values;
  - a theme with only `scrollbar.thumb.background` = `#80808040`: `hover.a` ≈ 0x40/255 × 1.6, `active.a` ≈ 0x40/255 × 2.2, `r`, `g`, `b` unchanged;
  - a theme with `scrollbar.thumb.background` = `#808080cc`: `active.a == 1.0`.
- [ ] **Step 2: Run, see it fail.** Run `cargo test -p lucerna-theme scrollbar_thumb`. Expected: compile error, the new fields do not exist yet.
- [ ] **Step 3: Add the two fields and read the keys** in `schema.rs`, compute the derived colors after `scrollbar_thumb` is available.
- [ ] **Step 4: Run again, see it pass.** Run `cargo test -p lucerna-theme`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(theme): scrollbar thumb hover and active colors`

---

### Task 3: The `Scrollbar` component and attaching it to the Explorer

**Files:**
- Create: `crates/lucerna-workbench/src/scrollbar.rs`. Declare `mod scrollbar;` in `lib.rs`, then `pub use crate::scrollbar::{Scrollbar, ScrollbarSource, ScrollbarState, ScrollHandleSource, ScrollMetrics};` in `ui.rs`.
- Modify: `crates/lucerna-workbench/src/explorer/view.rs`:
  - remove `struct ScrollbarThumb` (lines ~1339–1385) and `.with_decoration(..)`;
  - add the field `scrollbar: ScrollbarState`;
  - add `pub fn scrollbar_thumb(&self) -> Option<Bounds<Pixels>>`.
- Test: `crates/lucerna-workbench/tests/explorer_view.rs`. Change `scrollbar_thumb_only_when_list_overflows` to read `ExplorerView::scrollbar_thumb()` instead of the debug selector `explorer-scrollbar-thumb`.

**Interfaces:**
- Consumes:
  - `thumb_geometry`, `scroll_top_for_thumb`, `scroll_top_for_track_click`, `SCROLLBAR_WIDTH`, `SCROLLBAR_HIT_WIDTH` (Task 1);
  - `ui.scrollbar_thumb`, `ui.scrollbar_thumb_hover`, `ui.scrollbar_thumb_active` (Task 2).
- Produces:

```rust
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollMetrics { pub viewport: f32, pub content: f32, pub scroll_top: f32 }

pub trait ScrollbarSource: 'static {
    /// Read at prepaint, after the content has been laid out in this frame.
    fn metrics(&self, cx: &mut App) -> Option<ScrollMetrics>;
    /// Set the new scroll position (already clamped) and notify the owning view.
    fn set_scroll_top(&self, scroll_top: f32, cx: &mut App);
    /// Clicking the track flips pages instead of jumping.
    fn track_click_pages(&self, _cx: &App) -> bool { false }
}

/// Hover, drag, and the thumb as last painted; cheap to clone, kept in the view.
#[derive(Clone, Default)]
pub struct ScrollbarState(/* Rc<RefCell<..>> */);
impl ScrollbarState {
    pub fn thumb(&self) -> Option<Bounds<Pixels>>;
    pub fn is_hovered(&self) -> bool;
    pub fn is_dragging(&self) -> bool;
}

/// Wraps `gpui::ScrollHandle`; `view` is notified after scrolling.
pub struct ScrollHandleSource { pub handle: gpui::ScrollHandle, pub view: gpui::EntityId }

pub struct Scrollbar;
impl Scrollbar {
    /// A strip SCROLLBAR_HIT_WIDTH wide, as tall as the parent, placed absolute top_0 right_0.
    /// The parent must be `.relative()`.
    pub fn new(state: ScrollbarState, source: impl ScrollbarSource, owner: gpui::EntityId) -> impl IntoElement;
}
```

- **Mouse handling.** In the canvas, the hitbox uses `HitboxBehavior::BlockMouse` and the cursor `CursorStyle::Arrow`. Mouse listeners are registered at window level at paint, like `code_view/minimap.rs`.
  - **Left MouseDown on the hitbox:**
    - hits the thumb: save `grab = y − thumb.top`;
    - hits the track: call `set_scroll_top(scroll_top_for_track_click(..))`, then save `grab = thumb_height / 2` (when flipping pages, dragging does not start yet);
    - after that call `stop_propagation`.
  - **MouseMove while dragging**, mouse anywhere: `set_scroll_top(scroll_top_for_thumb(.., y − track_top − grab))`.
  - **MouseUp anywhere:** end the drag, notify `owner`.
  - **MouseMove in general:** turn hover on or off depending on the position relative to the 12 px strip, and notify `owner` only when the state really changes.
- **Thumb color:** while dragging use the active color; while hovered use the hover color; otherwise use the normal color.
- **Explorer scroll source:** `ScrollHandleSource` is taken from `self.scroll.0.borrow().base_handle.clone()`:
  - `viewport = handle.bounds().size.height`;
  - `content = viewport + max_offset.y`;
  - `scroll_top = −offset.y`.
- **Position in the layout:** wrap `uniform_list` in a `.relative()` div, with `Scrollbar::new(..)` as the second child, after the list.

- [ ] **Step 1: Write the failing tests** in `explorer_view.rs`. Use a folder with 300 files `f{i:03}.rs`, like the existing test.
  - `explorer_scrollbar_drag_scrolls`: press the mouse in the middle of the thumb, drag down exactly `list.height − thumb.height` px, release.
    - The thumb now has `bottom()` ≈ `list.bottom()` (±0.5 px).
    - `explorer-row-f299.rs` has debug bounds.
    - `explorer-row-f000.rs` does not.
  - `explorer_track_click_jumps`: click at `(list.right() − 3, list.bottom() − 2)`. The thumb moves to the bottom, `bottom()` ≈ `list.bottom()`.
  - `drag_ends_on_mouse_up_anywhere`:
    - press on the thumb;
    - drag 20 px;
    - release the mouse at `list.left() + 5`, outside the scrollbar strip;
    - move the mouse without a button held another 100 px.
    - The thumb top does not change after the last move.
  - `scrollbar_hover_tracks_mouse`:
    - move the mouse to `(list.right() − 10, thumb.center().y)`: `explorer.read(cx).scrollbar_hovered()` is `true` (add the accessor `pub fn scrollbar_hovered(&self) -> bool`, forwarding to `ScrollbarState::is_hovered`);
    - move out to `list.left() + 5`: it is `false`.
  - Change `scrollbar_thumb_only_when_list_overflows`: use `scrollbar_thumb()`, keep the asserts about the 6 px width and the right edge.
- [ ] **Step 2: Run, see it fail.** Run `cargo test -p lucerna-workbench --test explorer_view scrollbar`. Expected: compile error or FAIL.
- [ ] **Step 3: Write `scrollbar.rs`** with exactly the interfaces above, then attach it to the Explorer.
- [ ] **Step 4: Run again, see it pass.** Run `cargo test -p lucerna-workbench --test explorer_view`. Expected: PASS, including the old Explorer tests such as row click and context menu.
- [ ] **Step 5: Commit** `feat(ui): shared mouse scrollbar, used by the Explorer`

---

### Task 4: The editor scrollbar

**Files:**
- Modify: `crates/lucerna-workbench/src/code_view/layout.rs`: add `Folds::nth_visible`, with a unit test.
- Modify: `crates/lucerna-workbench/src/code_view/mod.rs`:
  - add `Config.scroll_by_page`, read from `editor.scrollbar.scrollByPage`, default `false`;
  - add the field `scrollbar: ScrollbarState`;
  - render `Scrollbar::new` in the `code-text-area` div (already `.relative()`), after `CodeElement`;
  - `scrollbar_thumb()` returns `self.scrollbar.thumb()`.
- Modify: `crates/lucerna-workbench/src/code_view/element.rs:681-703` and `element/paint.rs:185-188`: remove computing and drawing the thumb. Remove `Frame.thumb` if it is no longer used. Keep the marker ticks as they are.
- Modify: `crates/lucerna-settings/assets/default-settings.json`: `"editor.scrollbar.scrollByPage": false`.
- Test: `crates/lucerna-workbench/tests/code_view.rs`.

**Interfaces:**
- Consumes: `Scrollbar`, `ScrollbarState`, `ScrollbarSource`, `ScrollMetrics` (Task 3).
- Produces:
  - `Folds::nth_visible(&self, n: u32) -> u32`: the buffer row of the `n`-th visible row, the opposite of `row − hidden_before(row)`.
  - `CodeViewScroll(WeakEntity<CodeView>)`: implements `ScrollbarSource`.
    - `metrics`: the same formula as `element.rs:681-694` today, with `viewport` being the height of the text area as last laid out, taken from `frame.text_bounds` or `frame.hitbox`.
    - `set_scroll_top(px)`:
      - `n = floor(px / lh)`, `top_row = folds.nth_visible(n)`;
      - `offset = (px / lh − n) × wrap_count(top_row) × lh`;
      - call `scroll_by(0.)` to clamp;
      - call `report_scroll`, then `cx.notify()`.
    - `track_click_pages`: returns `config.scroll_by_page`.

- [ ] **Step 1: Write the failing tests**
  - Unit test in `layout.rs`, `nth_visible_skips_folds`: with a closed fold hiding rows 3..10 (header 2), check
    - `nth_visible(0) == 0`;
    - `nth_visible(2) == 2`;
    - `nth_visible(3) == 10`;
    - `nth_visible(5) == 12`.
  - In `tests/code_view.rs`, a 500-line file `line {i}`:
    - `editor_scrollbar_drag_scrolls`: drag the thumb from its middle down `viewport.height − thumb.height` px. Result: `scroll_top_row() == 499`, the thumb touches the bottom (±0.01 px), like the existing test `scrollbar_thumb_tracks_scroll`.
    - `editor_track_click_jumps`: click at `(viewport.width − 3, viewport.height / 2)`. Result: `scroll_top_row()` is within `249 ± 1`.
    - `scroll_by_page_setting_pages`: use `init(cx, Some(r#"{ "editor.scrollbar.scrollByPage": true }"#))`, click near the bottom of the track. Result: `scroll_top_row() == visible_rows()`, ±1 row.
    - `track_click_with_folds_lands_on_visible_row`:
      - a file with a large function to fold, or call `fold_at` on a region hiding about 200 lines at the top;
      - click the middle of the track;
      - the top row is the middle visible row, that is `top_row − hidden_before(top_row)` ≈ (number of visible rows − 1) / 2, ±1;
      - dragging to the bottom gives `scroll_top_row() == 499`.
    - `scrollbar_click_keeps_selection`: put the cursor at (0,0), click then drag the thumb. Result: the primary selection is still `(0,0)..(0,0)`.
    - `minimap_still_scrolls_next_to_scrollbar`: enable the minimap then click at `minimap.left() + 2`. Result: it still scrolls like the existing minimap test, that is `scroll_top_row()` changes.
  - Keep the two tests `scrollbar_thumb_tracks_scroll` and `scrollbar_thumb_is_left_of_the_minimap`: these two must pass without changes.
- [ ] **Step 2: Run, see it fail.** Run `cargo test -p lucerna-workbench --test code_view scrollbar` and `cargo test -p lucerna-workbench --lib nth_visible`. Expected: FAIL or compile error.
- [ ] **Step 3: Write the code** following the interfaces above.
- [ ] **Step 4: Run again, see it pass.** Run `cargo test -p lucerna-workbench`. Expected: all PASS.
- [ ] **Step 5: Check everything.**
  - `cargo clippy -p lucerna-workbench -p lucerna-theme --all-targets -- -D warnings`
  - `cargo deny check`
  - Expected: clean.
- [ ] **Step 6: Commit** `feat(editor): drag the scrollbar thumb and click the track`
