//! Picker building blocks shared by Quick Open and the theme picker: the
//! input row, the scrolling list, a row, match highlighting, fuzzy
//! filtering and wrapping selection.

use std::ops::Range;

use gpui::{
    App, Div, Entity, HighlightStyle, Hsla, ParentElement, ScrollHandle, SharedString, Stateful,
    StyledText, div, prelude::*, px,
};
use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::AppState;
use crate::layout::color;
use crate::text_input::TextInput;
use crate::ui::GRID;

/// Height of one picker row.
pub(crate) const PICKER_ROW_HEIGHT: f32 = 7. * GRID;
/// Rows shown before the list scrolls.
pub(crate) const PICKER_VISIBLE_ROWS: f32 = 12.;

/// The items whose text fuzzy-matches `query`, best first, with the
/// matched char indices; every item in order when `query` is empty.
pub(crate) fn fuzzy_filter<'a>(
    items: impl IntoIterator<Item = &'a str>,
    query: &str,
) -> Vec<(usize, Vec<u32>)> {
    let items = items.into_iter().enumerate();
    if query.is_empty() {
        return items.map(|(index, _)| (index, Vec::new())).collect();
    }
    let pattern = Pattern::new(
        query,
        CaseMatching::Smart,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut buf = Vec::new();
    let mut scored: Vec<(u32, usize, Vec<u32>)> = items
        .filter_map(|(index, text)| {
            let mut positions = Vec::new();
            let score =
                pattern.indices(Utf32Str::new(text, &mut buf), &mut matcher, &mut positions)?;
            positions.sort_unstable();
            positions.dedup();
            Some((score, index, positions))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored
        .into_iter()
        .map(|(_, index, positions)| (index, positions))
        .collect()
}

/// `current` moved by `delta` within `0..len`, wrapping at both ends.
pub(crate) fn wrap_index(current: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    (current as isize + delta).rem_euclid(len as isize) as usize
}

/// The color of fuzzy-matched chars in a picker. The theme has no
/// `text.accent`; the active line number is its accent.
pub(crate) fn match_color(cx: &App) -> Hsla {
    color(cx.global::<AppState>().theme.ui.editor_active_line_number).into()
}

/// `text` with the chars at `positions` (char indices) in the accent color.
pub(crate) fn highlighted(text: SharedString, positions: &[u32], cx: &App) -> StyledText {
    let style = HighlightStyle {
        color: Some(match_color(cx)),
        ..Default::default()
    };
    let ranges: Vec<(Range<usize>, HighlightStyle)> = text
        .char_indices()
        .enumerate()
        .filter(|(i, _)| positions.binary_search(&(*i as u32)).is_ok())
        .map(|(_, (byte, ch))| (byte..byte + ch.len_utf8(), style))
        .collect();
    StyledText::new(text).with_highlights(ranges)
}

/// The input row at the top of a picker, with a divider below it.
pub(crate) fn picker_input_row(input: &Entity<TextInput>, cx: &App) -> Div {
    let ui = &cx.global::<AppState>().theme.ui;
    div()
        .flex()
        .items_center()
        .h(px(9. * GRID))
        .px(px(3. * GRID))
        .border_b_1()
        .border_color(color(ui.border_variant))
        .child(input.clone())
}

/// The scrolling list under a picker's input.
pub(crate) fn picker_list(id: &'static str, scroll: &ScrollHandle) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_col()
        .p(px(GRID))
        .max_h(px(PICKER_ROW_HEIGHT * PICKER_VISIBLE_ROWS + 2. * GRID))
        .overflow_y_scroll()
        .track_scroll(scroll)
}

/// One picker row: `element.selected` when selected, `ghost_element.hover`
/// on hover. Its debug selector is `"{id}-{index}"`; add `on_click` and
/// the content.
pub(crate) fn picker_row(
    id: &'static str,
    index: usize,
    selected: bool,
    cx: &App,
) -> Stateful<Div> {
    picker_row_sized(id, index, selected, PICKER_ROW_HEIGHT, cx)
}

/// A [`picker_row`] `height` px tall, for rows of two lines.
pub(crate) fn picker_row_sized(
    id: &'static str,
    index: usize,
    selected: bool,
    height: f32,
    cx: &App,
) -> Stateful<Div> {
    let ui = &cx.global::<AppState>().theme.ui;
    let hover = color(ui.ghost_element_hover);
    div()
        .id((id, index))
        .debug_selector(|| format!("{id}-{index}"))
        .flex()
        .flex_none()
        .items_center()
        .h(px(height))
        .px(px(2. * GRID))
        .rounded(px(GRID))
        .overflow_hidden()
        .cursor_pointer()
        .when(selected, |row| row.bg(color(ui.element_selected)))
        .when(!selected, |row| row.hover(move |s| s.bg(hover)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_index_wraps_both_ways() {
        assert_eq!(wrap_index(0, 3, -1), 2);
        assert_eq!(wrap_index(2, 3, 1), 0);
        assert_eq!(wrap_index(1, 3, 1), 2);
        assert_eq!(wrap_index(0, 0, 1), 0);
    }

    #[test]
    fn fuzzy_filter_best_first_and_all_when_empty() {
        let names = ["Lucerna Dark", "Lucerna Light", "Solarized Light"];
        let indices = |q: &str| -> Vec<usize> {
            fuzzy_filter(names, q).into_iter().map(|(i, _)| i).collect()
        };
        assert_eq!(indices(""), vec![0, 1, 2]);
        assert_eq!(indices("dark"), vec![0]);
        assert_eq!(indices("light").len(), 2);
        assert!(indices("zzz").is_empty());
        assert_eq!(fuzzy_filter(names, "dark")[0].1, vec![8, 9, 10, 11]);
    }
}
