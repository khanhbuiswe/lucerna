# Mouse-driven scrollbar: VS Code and Zed

Looked up on 2026-11-14. VS Code at branch `main` (MIT). Zed at branch `main`, crate `ui` (GPL): only the behavior is studied, no code is copied.

## VS Code

Sources: `src/vs/base/browser/ui/scrollbar/abstractScrollbar.ts`, `scrollbarState.ts`, `src/vs/editor/common/config/editorOptions.ts`.

- **Dragging the thumb (slider).** On mouse down, VS Code stores the mouse position and a copy of the scrollbar state. On every mouse move, the offset from the press point is added to the original slider position (`getDesiredScrollPositionFromDelta`). This keeps the grab point always under the cursor, and the error does not accumulate.
  - Windows only: dragging sideways more than 140 px (`POINTER_DRAG_RESET_DISTANCE`) scrolls back to the original spot. macOS and Linux do not have this.
- **Clicking the track.**
  - `editor.scrollbar.scrollByPage` defaults to `false`: jump to the click spot, with the slider centered on the click point (`getDesiredScrollPositionFromOffset` subtracts half the slider height).
  - When set to `true`: clicking above the slider goes up one page, below the slider goes down one page (`...FromOffsetPaged`).
  - After a track click, if the mouse is held you can keep dragging, because `_sliderPointerDown` is called right away.
  - There is **no** inverted behavior when holding Shift in the current code.
- **Hover.** Only the color changes, via CSS (`scrollbarSlider.hoverBackground`; while dragging, `scrollbarSlider.activeBackground`, through the `active` class). The size does not change.
- The slider is at least 20 px (`MINIMUM_SLIDER_SIZE`).

## Zed

Source: `crates/ui/src/components/scrollbar.rs`.

- **Three thumb states:** Inactive, Hover, Dragging. Hover and Dragging only change the color, using separate hover and active colors; the thumb does not widen.
- **Clicking the track:** the thumb is centered on the click point, clamped between the two ends of the track. No page flipping.
- **Dragging:** stores the grab point inside the thumb, then keeps that spot under the cursor.
- **While dragging:** only notifies the scrollbar's state entity (and the parent entity if one is registered). The mouse is captured the whole time, the cursor is an arrow.
- **Size:** the normal thumb is 6 px wide, with 4 px of padding on each side; the editor thumb is 15 px wide. The shortest thumb is 25 px.
- **Auto-hide:** hides after 1 second of no use, 400 ms fade. Lucerna has not done this part yet.

## What Lucerna chooses

- **Behavior follows VS Code:**
  - dragging keeps the grab point;
  - clicking the track jumps to the click spot, with the setting `editor.scrollbar.scrollByPage`;
  - holding the mouse after a track click lets you keep dragging;
  - no Shift behavior;
  - no 140 px reset, because macOS is the primary platform and Zed does not have it either.
- **Look follows Zed:**
  - 6 px thumb as now;
  - 12 px wide mouse hit area;
  - color change on hover and on drag, using Zed's theme keys `scrollbar.thumb.hover_background` and `scrollbar.thumb.active_background`;
  - one scrollbar component shared between the editor and lists, like Zed's `Scrollbar` component.
