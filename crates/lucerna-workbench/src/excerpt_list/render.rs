//! Drawing an [`ExcerptList`]: file headers (with a sticky copy pinned over
//! the list), code lines with their gutter, excerpt separators and file
//! notes. Diff lines get the diff backgrounds and a change bar at the
//! gutter's left edge, as wide as the editor's.

use std::ops::Range;

use gpui::{
    AnyElement, App, Context, Div, ElementId, FontStyle, FontWeight, HighlightStyle, IntoElement,
    MouseButton, ParentElement, Pixels, Render, SharedString, Stateful, Styled, StyledText, Window,
    div, font, list, prelude::*, px, relative, svg,
};

use lucerna_project::LineKind;

use super::ExcerptList;
use super::highlight::LineSpans;
use super::model::{LineRef, Row, expand_tabs, thousands};
use crate::AppState;
use crate::code_view::hunk_bar_width;
use crate::icons::ARROW_UP_RIGHT;
use crate::layout::color;
use crate::ui::{
    CHEVRON_DOWN_ICON, CHEVRON_RIGHT_ICON, FILE_ICON_SIZE, GRID, UI_FONT_SIZE_CAPTION,
    file_icon_sized, subheader_card,
};

/// A file header is this many editor lines tall, as in Zed.
const FILE_HEADER_LINES: f32 = 2.;

/// Padding around a file header's card (Zed's `BUFFER_HEADER_PADDING`).
const FILE_HEADER_PADDING: f32 = GRID;

/// How rows are drawn, read from the `editor.*` settings each frame.
#[derive(Clone)]
struct RowStyle {
    font_family: SharedString,
    font_size: Pixels,
    line_height: Pixels,
    gutter: Pixels,
    tab_size: u32,
}

impl ExcerptList {
    /// Fonts and sizes of the rows: the editor's font, every row one
    /// editor line tall (as `CodeView` computes it), and a gutter as wide as
    /// the largest line number.
    fn row_style(&mut self, window: &Window, cx: &App) -> RowStyle {
        self.scale_factor = window.scale_factor();
        let settings = &cx.global::<AppState>().settings;
        let font_family = crate::fonts::editor_font_family(None, cx);
        let font_size = px(editor_font_size(cx));
        let text_system = window.text_system();
        let font_id = text_system.resolve_font(&font(font_family.clone()));
        let char_width = text_system
            .em_advance(font_id, font_size)
            .ok()
            .filter(|w| *w > px(0.))
            .unwrap_or(font_size * 0.6);
        let digits = self.gutter_digits();
        RowStyle {
            font_family,
            font_size,
            line_height: editor_line_height(self.scale_factor, cx),
            gutter: char_width * digits as f32 + px(4. * GRID),
            tab_size: settings
                .get::<u32>("editor.tabSize", None)
                .unwrap_or(4)
                .max(1),
        }
    }

    /// Row `ix` of the list.
    /// `selected` is the selected line, resolved once for the frame.
    fn render_row(
        &mut self,
        ix: usize,
        style: &RowStyle,
        selected: Option<LineRef>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(row) = self.results.rows().get(ix).copied() else {
            return div().into_any_element();
        };
        match row {
            Row::Header(file) => self.render_header(file, false, style, selected, cx),
            Row::Line(r) => self.render_line(ix, r, style, selected == Some(r), cx),
            Row::Separator { file } => {
                // The excerpt it comes before.
                let excerpt = match self.results.rows().get(ix + 1) {
                    Some(Row::Line(r)) => r.excerpt,
                    _ => 0,
                };
                self.render_separator(ix, file, excerpt, style, cx)
            }
            Row::Note { file } => self.render_note(ix, file, style, cx),
        }
    }

    /// A file header, two editor lines tall as in Zed: a card with a 1px
    /// border on `editor.subheader.background` holding the fold chevron,
    /// the file icon, the name, the folder (muted, cut with "…"), the
    /// match count and an "open file" button. Clicking the card folds or
    /// unfolds the file. `sticky` is the copy pinned over the list. A file
    /// with diff meta shows its label as a small tag after the name and
    /// `+added −removed` in place of the match count; an unopenable one
    /// has its button disabled.
    fn render_header(
        &self,
        index: usize,
        sticky: bool,
        style: &RowStyle,
        selected: Option<LineRef>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = cx.global::<AppState>();
        let ui = &state.theme.ui;
        let file = &self.results.files()[index];
        let name = file
            .path
            .file_name()
            .unwrap_or(file.path.as_os_str())
            .to_string_lossy()
            .into_owned();
        let dir = file
            .path
            .parent()
            .map(|d| d.to_string_lossy().into_owned())
            .unwrap_or_default();
        let chevron = if file.collapsed {
            CHEVRON_RIGHT_ICON
        } else {
            CHEVRON_DOWN_ICON
        };
        let icon = state.icons.file(&file.path);
        let path = file.path.clone();
        let hover = color(ui.ghost_element_hover);
        let prefix = self.id_prefix;
        let (card_id, selector, open_id, open_selector): (ElementId, String, ElementId, String) =
            if sticky {
                (
                    "sticky-header".into(),
                    format!("{prefix}-sticky-card"),
                    "sticky-open-file".into(),
                    format!("{prefix}-sticky-open-file"),
                )
            } else {
                (
                    ("file", index).into(),
                    format!("{prefix}-file-{index}"),
                    ("open-file", index).into(),
                    format!("{prefix}-open-file-{index}"),
                )
            };
        let group: SharedString = format!("{selector}-group").into();
        let selected_here = selected.is_some_and(|r| r.file == index);
        let meta = self.results.meta(&file.path).copied();
        let openable = meta.is_none_or(|m| m.openable);
        let open_path = path.clone();
        let open = div()
            .id(open_id)
            .debug_selector(move || open_selector)
            .flex()
            .flex_none()
            .justify_center()
            .items_center()
            .w(px(6. * GRID))
            .h(px(5. * GRID))
            .rounded(px(GRID))
            .when(openable, |d| d.cursor_pointer().hover(move |s| s.bg(hover)))
            .when(!selected_here, |d| {
                d.opacity(0.).group_hover(group.clone(), move |s| {
                    s.opacity(if openable { 1. } else { 0.4 })
                })
            })
            .when(selected_here && !openable, |d| d.opacity(0.4))
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            // Disabled, it still swallows the click so the card does not fold.
            .on_click(cx.listener(move |list, _, _, cx| {
                cx.stop_propagation();
                if openable {
                    list.open_file_at_first_match(&open_path, cx);
                }
            }))
            .child(
                svg()
                    .data(ARROW_UP_RIGHT)
                    .size(px(14.))
                    .text_color(color(ui.icon_muted)),
            );
        let card = subheader_card(cx)
            .id(card_id)
            .debug_selector(move || selector)
            .group(group)
            .gap(px(1.5 * GRID))
            .size_full()
            .pl(px(GRID))
            .pr(px(GRID))
            .hover(move |s| s.bg(hover))
            .when(sticky, |d| d.shadow_md())
            .cursor_pointer()
            .on_click(cx.listener(move |list, _, _, cx| {
                if sticky {
                    list.toggle_sticky(&path, cx);
                } else {
                    list.toggle_path(&path, cx);
                }
            }))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .justify_center()
                    .items_center()
                    .w(px(5. * GRID))
                    .child(
                        svg()
                            .data(chevron)
                            .size(px(12.))
                            .text_color(color(ui.icon_muted)),
                    ),
            )
            .child(file_icon_sized(
                &icon,
                px(FILE_ICON_SIZE),
                color(ui.icon_muted).into(),
            ))
            .child(
                div()
                    .flex_none()
                    .max_w(relative(0.6))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(color(ui.text))
                    .child(name),
            )
            .children(meta.and_then(|m| m.label).map(|label| {
                div()
                    .when(!sticky, |d| {
                        d.debug_selector(move || format!("{prefix}-label-{index}"))
                    })
                    .flex_none()
                    .px(px(GRID))
                    .rounded(px(GRID))
                    .border_1()
                    .border_color(color(ui.border_variant))
                    .text_size(px(UI_FONT_SIZE_CAPTION))
                    .text_color(color(ui.text_muted))
                    .child(label)
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(color(ui.text_muted))
                    .child(dir),
            )
            .child(match meta {
                Some(m) => div()
                    .when(!sticky, |d| {
                        d.debug_selector(move || format!("{prefix}-stats-{index}"))
                    })
                    .flex()
                    .flex_none()
                    .gap(px(GRID))
                    .child(
                        div()
                            .text_color(color(ui.vc_added))
                            .child(format!("+{}", thousands(m.added as usize))),
                    )
                    .child(
                        div()
                            .text_color(color(ui.vc_deleted))
                            .child(format!("\u{2212}{}", thousands(m.removed as usize))),
                    ),
                None => div()
                    .flex_none()
                    .text_color(color(ui.text_muted))
                    .child(thousands(file.match_count())),
            })
            .child(open);
        div()
            .w_full()
            .h(header_height(style.line_height))
            .p(px(FILE_HEADER_PADDING))
            .child(card)
            .into_any_element()
    }

    fn render_line(
        &self,
        ix: usize,
        r: LineRef,
        style: &RowStyle,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = cx.global::<AppState>();
        let (ui, theme) = (&state.theme.ui, &state.theme);
        let Some(line) = self.results.line(r) else {
            return div().into_any_element();
        };
        let text = &line.line_text;
        let mark = HighlightStyle {
            background_color: Some(color(ui.search_match_background).into()),
            ..Default::default()
        };
        let spans = self
            .results
            .files()
            .get(r.file)
            .and_then(|f| self.highlight_cache.get(&f.path, line));
        // `CodeView` expands tabs itself; gpui would draw them raw.
        let expanded = expand_tabs(text, style.tab_size);
        let ranges: Vec<Range<usize>> = line
            .ranges
            .iter()
            .map(|r| expanded.map(r.start)..expanded.map(r.end))
            .collect();
        let spans: Option<LineSpans> = spans.map(|s| {
            s.iter()
                .map(|(r, c)| (expanded.map(r.start)..expanded.map(r.end), c.clone()))
                .collect()
        });
        let highlights = line_highlights(&expanded.text, &ranges, spans.as_ref(), theme, mark);
        let styled = StyledText::new(expanded.text.clone()).with_highlights(highlights);
        let selector = format!(
            "{}-line-{}-{}-{}",
            self.id_prefix, r.file, r.excerpt, r.line
        );
        let anchor = self.results.anchor_of(r);
        // Code lines read like editor lines (as in Zed's multibuffer): no
        // hover; the selected line gets the editor's active-line colors,
        // over the diff color of an added or deleted line.
        let bar = match line.kind {
            LineKind::Normal => None,
            LineKind::Added => Some(ui.vc_added),
            LineKind::Deleted { .. } => Some(ui.vc_deleted),
        };
        let mut el = row(ix, selector, style).relative();
        if let Some(bg) = self.background_of(line.kind, selected, cx) {
            el = el.bg(bg);
        }
        if selected && bar.is_some() {
            el = el.child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(color(ui.editor_active_line_background)),
            );
        }
        let number = if selected {
            color(ui.editor_active_line_number)
        } else {
            color(ui.text_muted)
        };
        let id = (self.id_prefix, r.file, r.excerpt, r.line);
        let mut gutter = gutter(style, number).relative();
        if let Some(bar) = bar {
            gutter = gutter.child(
                div()
                    .debug_selector(move || format!("{}-bar-{}-{}-{}", id.0, id.1, id.2, id.3))
                    .absolute()
                    .left_0()
                    .top_0()
                    .h_full()
                    .w(hunk_bar_width(style.line_height))
                    .bg(color(bar)),
            );
        }
        // A deleted line is not in the current text: no number, as in Zed.
        if !matches!(line.kind, LineKind::Deleted { .. }) {
            gutter = gutter.child(
                div()
                    .debug_selector(move || format!("{}-number-{}-{}-{}", id.0, id.1, id.2, id.3))
                    .child(line.line_number.to_string()),
            );
        }
        el.on_click(cx.listener(move |list, _, _, cx| {
            if let Some(anchor) = &anchor {
                list.open_anchor(anchor, cx);
            }
        }))
        .child(gutter)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .font_family(style.font_family.clone())
                .text_size(style.font_size)
                .text_color(color(ui.editor_foreground))
                .child(styled),
        )
        .into_any_element()
    }

    /// The gap between two excerpts of a file: one editor line with a thin
    /// `border.variant` rule across the code (Zed draws excerpt
    /// boundaries as a line, not a glyph in the gutter).
    fn render_separator(
        &self,
        ix: usize,
        file: usize,
        excerpt: usize,
        style: &RowStyle,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rule = color(cx.global::<AppState>().theme.ui.border_variant);
        let selector = format!("{}-separator-{file}-{excerpt}", self.id_prefix);
        div()
            .id(("row", ix))
            .debug_selector(move || selector)
            .flex()
            .items_center()
            .h(style.line_height)
            .w_full()
            .pr(px(2. * GRID))
            .child(div().flex_none().w(style.gutter))
            .child(div().flex_1().h(px(1.)).bg(rule))
            .into_any_element()
    }
}

impl ExcerptList {
    /// A file's note (such as "Binary file"), one muted line in place of
    /// its excerpts, starting where the code does.
    fn render_note(
        &self,
        ix: usize,
        file: usize,
        style: &RowStyle,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ui = &cx.global::<AppState>().theme.ui;
        let note = self
            .results
            .files()
            .get(file)
            .and_then(|f| self.results.meta(&f.path))
            .and_then(|m| m.note)
            .unwrap_or_default();
        let selector = format!("{}-note-{file}", self.id_prefix);
        div()
            .id(("row", ix))
            .debug_selector(move || selector)
            .flex()
            .items_center()
            .gap(px(1.5 * GRID))
            .h(style.line_height)
            .w_full()
            .pr(px(2. * GRID))
            .child(div().flex_none().w(style.gutter))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(color(ui.text_muted))
                    .child(note),
            )
            .into_any_element()
    }
}

impl Render for ExcerptList {
    /// The rows, with the sticky header over them. The host tracks
    /// [`ExcerptList::focus_handle`] on the area around it.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = cx.global::<AppState>().theme.ui.clone();
        let style = self.row_style(window, cx);
        let lh = style.line_height;
        // Found once per frame, not per row: resolving it scans its file.
        let selected = self.selected();
        self.selection_resolves += 1;
        // The rows on screen get their syntax colors first.
        let viewport = match self.list.viewport_bounds().size.height {
            h if h > px(0.) => h,
            _ => window.viewport_size().height,
        };
        let mut visible: Vec<usize> = Vec::new();
        for ix in self.visible_rows(viewport, lh) {
            let file = self.results.rows()[ix].file();
            if visible.last() != Some(&file) {
                visible.push(file);
            }
        }
        self.request_highlights(visible, cx);
        // The pinned header: a clip at the list's top, as tall as the part
        // still shown while the next header pushes it up.
        let prefix = self.id_prefix;
        let sticky = self.sticky_header(lh).map(|(file, push)| {
            div()
                .debug_selector(move || format!("{prefix}-sticky-header"))
                .absolute()
                .left_0()
                .right_0()
                .top_0()
                .h(header_height(lh) + push)
                .overflow_hidden()
                // Clicks on the pinned header never reach the rows under it.
                .occlude()
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .top(push)
                        .bg(color(ui.editor_background))
                        .child(self.render_header(file, true, &style, selected, cx)),
                )
        });
        let row_style = style.clone();
        div()
            .relative()
            .size_full()
            // Nothing in here (the pushed-up sticky header) draws over the
            // host's toolbar.
            .overflow_hidden()
            .child(
                list(
                    self.list.clone(),
                    cx.processor(move |list, ix: usize, _, cx| {
                        list.render_row(ix, &row_style, selected, cx)
                    }),
                )
                .size_full(),
            )
            .children(sticky)
    }
}

/// The editor font size in pixels, from `editor.fontSize`.
fn editor_font_size(cx: &App) -> f32 {
    cx.global::<AppState>()
        .settings
        .get::<f32>("editor.fontSize", None)
        .unwrap_or(14.)
        .clamp(6., 40.)
}

/// One editor line, as `CodeView` computes it from `editor.lineHeight`
/// (a factor of the font size below 8, else pixels), rounded to the
/// device pixels of `scale_factor`: layout snaps row heights to that grid,
/// and scrolling to a row sums these heights, so they must match what the
/// list measures.
pub(super) fn editor_line_height(scale_factor: f32, cx: &App) -> Pixels {
    let size = editor_font_size(cx);
    let lh = cx
        .global::<AppState>()
        .settings
        .get::<f32>("editor.lineHeight", None)
        .unwrap_or(1.6);
    let lh = if lh < 8. { lh * size } else { lh };
    let scale = if scale_factor > 0. { scale_factor } else { 1. };
    px((lh.max(1.) * scale).round().max(1.) / scale)
}

/// A file header's height: [`FILE_HEADER_LINES`] editor lines.
pub(super) fn header_height(lh: Pixels) -> Pixels {
    lh * FILE_HEADER_LINES
}

/// The drawn height of `row`: every row but a header is one line.
pub(super) fn row_height(row: &Row, lh: Pixels) -> Pixels {
    match row {
        Row::Header(_) => header_height(lh),
        Row::Line(_) | Row::Separator { .. } | Row::Note { .. } => lh,
    }
}

/// The runs of one line: syntax colors from `spans` with `mark` laid over
/// the match `ranges`. Runs never overlap; where a match and a syntax span
/// meet, the run takes the syntax style plus the match background. Ranges
/// are clamped and anything off a char boundary or empty (a match past the
/// 1,000-char cut) is dropped.
fn line_highlights(
    text: &str,
    ranges: &[Range<usize>],
    spans: Option<&LineSpans>,
    theme: &lucerna_theme::Theme,
    mark: HighlightStyle,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let valid = |r: &Range<usize>| {
        r.start < r.end && text.is_char_boundary(r.start) && text.is_char_boundary(r.end)
    };
    let matches: Vec<Range<usize>> = ranges
        .iter()
        .map(|r| r.start..r.end.min(text.len()))
        .filter(valid)
        .collect();
    let syntax: Vec<(Range<usize>, &lucerna_theme::HighlightStyle)> = spans
        .into_iter()
        .flatten()
        .filter(|(r, _)| valid(r) && r.end <= text.len())
        .filter_map(|(r, capture)| Some((r.clone(), theme.style_for(capture)?)))
        .collect();
    let mut edges: Vec<usize> = matches
        .iter()
        .map(|r| r.start)
        .chain(matches.iter().map(|r| r.end))
        .chain(syntax.iter().flat_map(|(r, _)| [r.start, r.end]))
        .collect();
    edges.sort_unstable();
    edges.dedup();
    edges
        .windows(2)
        .filter_map(|w| {
            let seg = w[0]..w[1];
            // The narrowest span covering the segment wins; the first on a tie.
            let style = syntax
                .iter()
                .filter(|(r, _)| r.start <= seg.start && seg.end <= r.end)
                .min_by_key(|(r, _)| r.len())
                .map(|(_, s)| *s);
            let marked = matches
                .iter()
                .any(|r| r.start <= seg.start && seg.end <= r.end);
            if style.is_none() && !marked {
                return None;
            }
            let mut run = if marked {
                mark
            } else {
                HighlightStyle::default()
            };
            if let Some(s) = style {
                run.color = s.color.map(|c| color(c).into());
                if s.bold {
                    run.font_weight = Some(FontWeight::BOLD);
                }
                if s.italic {
                    run.font_style = Some(FontStyle::Italic);
                }
            }
            Some((seg, run))
        })
        .collect()
}

/// A clickable row, one editor line tall; add `on_click` and the content.
fn row(ix: usize, selector: String, style: &RowStyle) -> Stateful<Div> {
    div()
        .id(("row", ix))
        .debug_selector(move || selector)
        .flex()
        .items_center()
        .gap(px(1.5 * GRID))
        .h(style.line_height)
        .w_full()
        .pr(px(2. * GRID))
        .cursor_pointer()
}

/// The line-number column: right-aligned, in the editor font.
fn gutter(style: &RowStyle, muted: gpui::Rgba) -> Div {
    div()
        .flex()
        .flex_none()
        .justify_end()
        .w(style.gutter)
        .pr(px(2. * GRID))
        .font_family(style.font_family.clone())
        .text_size(style.font_size)
        .text_color(muted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lucerna_theme::ThemeRegistry;

    #[test]
    fn match_background_wins_over_syntax_and_runs_do_not_overlap() {
        let theme = ThemeRegistry::builtin().get_or_default("Lucerna Dark");
        let mark = HighlightStyle {
            background_color: Some(gpui::red()),
            ..Default::default()
        };
        // "fn main": keyword over 0..2, function over 3..7, match on 1..5.
        let spans: LineSpans = vec![(0..2, "keyword".into()), (3..7, "function".into())];
        let ranges = [1..3, 3..5];
        let runs = line_highlights("fn main", &ranges, Some(&spans), &theme, mark);
        assert!(runs.windows(2).all(|w| w[0].0.end <= w[1].0.start));
        let covered: Vec<_> = runs
            .iter()
            .filter(|(r, _)| r.start >= 1 && r.end <= 5)
            .collect();
        assert!(!covered.is_empty());
        assert!(covered.iter().all(|(_, s)| s.background_color.is_some()));
        assert!(
            runs.iter()
                .filter(|(r, _)| r.end <= 1 || r.start >= 5)
                .all(|(_, s)| s.background_color.is_none())
        );
        assert!(
            runs.iter().any(|(_, s)| s.color.is_some()),
            "syntax color kept"
        );
    }

    #[test]
    fn bad_ranges_are_dropped() {
        let theme = ThemeRegistry::builtin().get_or_default("Lucerna Dark");
        let spans: LineSpans = vec![(0..1, "keyword".into()), (0..99, "keyword".into())];
        let runs = line_highlights(
            "é",
            &[1..2, 5..9, 0..0],
            Some(&spans),
            &theme,
            Default::default(),
        );
        assert!(runs.is_empty(), "{runs:?}");
    }
}
