# Mouse-operated scrollbar

## Purpose

Today Lucerna's scrollbar is display-only. The thumb in the editor (`code_view/element.rs`) and in the Explorer (`ScrollbarThumb` in `explorer/view.rs`) is drawn, but no code handles the mouse on it. People reading a long file want to drag the thumb to scroll quickly, or click the track to jump to a spot in the file, the way they do in VS Code.

## Goals

- **Drag the thumb to scroll.** The grab point on the thumb always stays under the pointer.
- **Click the track** (the area outside the thumb) to jump to the clicked spot, with the thumb centered on the click point. Holding the mouse after the click keeps dragging.
- **The `editor.scrollbar.scrollByPage` setting** (bool, default `false`), as in VS Code. When on, clicking the track in the editor pages up or down instead of jumping. The setting applies only to the editor.
- **The thumb changes color** on hover and while being dragged. Its size does not change.
- **A 12 px mouse hit area** at the right edge, while the thumb is still drawn 6 px wide against the edge.
- **Correct with folded lines:** the editor's thumb is computed over display lines (each unfolded buffer line is one unit, as today). Dragging to a position lands on exactly the display line at that position.
- **One shared component** for the editor and the Explorer, placed in `ui.rs`. Later lists can reuse it.

## Non-goals

- Auto-hiding the scrollbar or a fade effect (Zed has it, Lucerna does not yet).
- A horizontal scrollbar.
- Inverting the behavior when Shift is held (current VS Code does not have it).
- Snapping back to the old position when the drag strays more than 140 px (only VS Code on Windows has it).
- Adding scrollbars to the pickers. The component must be able to do that later, but attaching it to pickers is not part of this change.

## Must not happen

- **Dragging that stutters or lags.** Each mouse move may only change the scroll position and notify. No re-shaping text, no rebuilding the marks on the scrollbar: the `scrollbar_marks` cache does not depend on the scroll position.
- **Clicking the scrollbar changing the selection** or moving the cursor in the editor.
- **The minimap** or the drag-to-select area ceasing to work.

## Constraints

- Only `lucerna-workbench` may use gpui.
- Do not copy code from Zed's `ui` crate (GPL license); learn the behavior only. See `docs/research/2026-11-14-scrollbar-mouse.md`.
- There must be GPUI tests for thumb dragging and track clicking, including the case with folded lines.

## Approaches considered

- **A. Share the computation, wire the mouse separately in each place.** The pure functions live in `ui.rs`; the editor handles the mouse in `register_mouse`, the Explorer handles it in a decoration. No new element, but the mouse wiring has to be written twice.
- **B. One shared scrollbar component, laid over the content (chosen).** One element handles the hitbox, the hover and drag state, and drawing the thumb itself, like Zed's `Scrollbar` component. There is only one place for the code. In exchange, the editor needs a layer converting between pixels and its line-based scrolling, and has to watch the hitbox order.

The user chose B.

## Design

### The `ui::Scrollbar` component

- **Scroll source.** The element takes a scroll source, a trait (provisionally called `ScrollbarSource`) providing:
  - `metrics()`: returns `viewport`, `content` and `scroll_top`, in the same pixel units as the parameters of `thumb_geometry`;
  - `set_scroll_top(px)`: sets the new scroll position, and notifies the owning view.
- **State.** Hover and drag (including the grab point within the thumb) live in a small `ScrollbarState`, held by the owning view across frames. The concrete mechanism (`Rc<Cell>` or element state) is left to the implementer.
- **Position.** The element covers a 12 px wide strip at the right edge of the content area. This strip's hitbox blocks the mouse from the parts underneath, so the editor does not receive the click.
  - Clicking on the thumb starts a drag.
  - Clicking on the track jumps to the clicked spot (or pages if the source requests it), then continues dragging from the middle of the thumb.
  - While dragging, the element receives mouse move and mouse up anywhere, until the button is released.
- **Color.** The track draws nothing. The thumb uses `scrollbar.thumb.background`, `scrollbar.thumb.hover_background` on hover, and `scrollbar.thumb.active_background` while dragging.
  - The two new keys are named after Zed's theme keys.
  - If a theme lacks these two keys, use the normal thumb color made darker (the implementer picks the amount).
- **Computation functions.** The functions computing "click spot to scroll position" and "drag distance to scroll position" are pure functions next to `thumb_geometry`, with unit tests.

### Editor

- **`CodeView`'s scroll source:**
  - `content` = (number of display lines − 1) × line height + viewport;
  - `scroll_top` = number of display lines above × line height + `scroll.offset`.
  - These two formulas stay exactly as in the current code.
- **`set_scroll_top(px)`:** convert `px` to the nth display line, then through `folds` to a buffer line used as `top_row`. The fractional part becomes the `offset` within that line, taking wrapped lines into account. Finally call `report_scroll` and notify.
- **Replacement.** Remove the thumb currently drawn in `paint.rs`. The diagnostic marks are still drawn as before, beneath the thumb.
- **Tests.** `CodeView::scrollbar_thumb()` still returns the thumb's bounds for tests to use.

### Explorer

- Remove the `ScrollbarThumb` decoration.
- Lay `ui::Scrollbar` over the list. The scroll source reads directly from `UniformListScrollHandle`:
  - `offset` and `max_offset` come from `base_handle`;
  - the scroll position is set with `set_offset`.
- This `ScrollHandle`-based scroll source can also be used for any `div` with `overflow_y_scroll` later.

### Settings

- Add `editor.scrollbar.scrollByPage` (bool, default `false`), read through the same mechanism as the existing `editor.*` settings.

## Left to the implementer

- How to hold `ScrollbarState` across frames.
- The amount of darkening for the fallback color.
- The exact names of types and functions.
- Debug selectors for tests: keep `explorer-scrollbar-thumb`, add a selector for the editor.

## References

See `docs/research/2026-11-14-scrollbar-mouse.md`.
