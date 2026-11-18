//! Drawing an [`ExcerptList`]: file headers (with a sticky copy pinned over
//! the list), code lines with their gutter, excerpt separators and file
//! notes. Diff lines get the diff backgrounds and a change bar at the
//! gutter's left edge, as wide as the editor's. Under a replace preview a
//! match is drawn struck out, followed by its replacement.

use std::borrow::Cow;
use std::ops::Range;
use std::sync::Arc;

use gpui::{
    AnyElement, App, Context, Div, ElementId, FontStyle, FontWeight, HighlightStyle, IntoElement,
    MouseButton, ParentElement, Pixels, Render, SharedString, Stateful, StrikethroughStyle, Styled,
    StyledText, Window, div, font, list, prelude::*, px, relative, svg,
};

use lucerna_project::LineKind;
use lucerna_settings::SettingsView;

use super::highlight::LineSpans;
use super::model::{LineRef, Row, expand_tabs, thousands};
use super::{ExcerptList, ExcerptListEvent, ListSelection};
use crate::AppState;
use crate::code_view::hunk_bar_width;
use crate::icons::{ARROW_UP_RIGHT, REPLACE, REPLACE_ALL, X};
use crate::layout::color;
use crate::ui::{
    CHEVRON_DOWN_ICON, CHEVRON_RIGHT_ICON, FILE_ICON_SIZE, GRID, UI_FONT_SIZE_CAPTION,
    file_icon_sized, subheader_card, tooltip,
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
        let settings = self.settings.read(cx).view(cx);
        let font_family = crate::fonts::editor_font_family(settings, None);
        let font_size = px(editor_font_size(settings, cx));
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
            line_height: editor_line_height(self.scale_factor, settings, cx),
            gutter: char_width * digits as f32 + px(4. * GRID),
            tab_size: settings
                .get::<u32>("editor.tabSize", None)
                .unwrap_or(4)
                .max(1),
        }
    }

    /// Row `ix` of the list.
    /// `selected` is the selected line or header, resolved once for the frame.
    fn render_row(
        &mut self,
        ix: usize,
        style: &RowStyle,
        selected: Option<ListSelection>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(row) = self.results.rows().get(ix).copied() else {
            return div().into_any_element();
        };
        match row {
            Row::Header(file) => self.render_header(file, false, style, selected, cx),
            Row::Line(r) => {
                let preview = self.preview_of(r);
                let selected = selected == Some(ListSelection::Line(r));
                self.render_line(ix, r, style, selected, preview, cx)
            }
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
        selected: Option<ListSelection>,
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
        let header_selected = selected == Some(ListSelection::Header(index));
        let selected_here =
            header_selected || matches!(selected, Some(ListSelection::Line(r)) if r.file == index);
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
        // Replace All and Dismiss, before Open File (see `RowActions`).
        let actions = self.row_actions.map(|actions| {
            let id = |name: &str| -> (ElementId, String) {
                if sticky {
                    (
                        format!("sticky-{name}").into(),
                        format!("{prefix}-sticky-{name}"),
                    )
                } else {
                    (
                        (SharedString::from(format!("file-{name}")), index).into(),
                        format!("{prefix}-file-{name}-{index}"),
                    )
                }
            };
            let (replace_path, dismiss_path) = (path.clone(), path.clone());
            let replace = actions.replace.then(|| {
                let (id, selector) = id("replace");
                row_button(
                    id,
                    selector,
                    REPLACE_ALL,
                    "Replace All",
                    &group,
                    selected_here,
                    ui,
                )
                .h(px(5. * GRID))
                .on_click(cx.listener(move |list, _, _, cx| {
                    cx.stop_propagation();
                    list.emit_file(&replace_path, ExcerptListEvent::ReplaceFile, cx);
                }))
            });
            let (id, selector) = id("dismiss");
            let dismiss = row_button(id, selector, X, "Dismiss", &group, selected_here, ui)
                .h(px(5. * GRID))
                .on_click(cx.listener(move |list, _, _, cx| {
                    cx.stop_propagation();
                    list.emit_file(&dismiss_path, ExcerptListEvent::DismissFile, cx);
                }));
            div()
                .flex()
                .flex_none()
                .items_center()
                .children(replace)
                .child(dismiss)
        });
        let card = subheader_card(cx)
            .id(card_id)
            .debug_selector(move || selector)
            .group(group)
            .gap(px(1.5 * GRID))
            .size_full()
            .pl(px(GRID))
            .pr(px(GRID))
            .when(header_selected, |d| {
                d.bg(color(ui.editor_active_line_background))
            })
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
                state.theme.appearance,
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
            .children(actions)
            .child(open);
        div()
            .w_full()
            .h(header_height(style.line_height))
            .p(px(FILE_HEADER_PADDING))
            .child(card)
            .into_any_element()
    }

    /// A code line. `preview` holds the replacement of each of its ranges
    /// when a replace preview is on.
    fn render_line(
        &self,
        ix: usize,
        r: LineRef,
        style: &RowStyle,
        selected: bool,
        preview: Option<Arc<[Option<String>]>>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = cx.global::<AppState>();
        let (ui, theme) = (&state.theme.ui, &state.theme);
        let Some(line) = self.results.line(r) else {
            return div().into_any_element();
        };
        let mark = HighlightStyle {
            background_color: Some(color(ui.search_match_background).into()),
            ..Default::default()
        };
        let spans = self
            .results
            .files()
            .get(r.file)
            .and_then(|f| self.highlight_cache.get(&f.path, line));
        let (shown, spans) = match &preview {
            Some(replacements) => (
                preview_line(&line.line_text, &line.ranges, replacements),
                spans.map(|s| {
                    Cow::Owned(shift_spans(s, &line.line_text, &line.ranges, replacements))
                }),
            ),
            None => (
                PreviewLine {
                    text: Cow::Borrowed(&line.line_text),
                    old: line.ranges.clone(),
                    new: Vec::new(),
                },
                spans.map(Cow::Borrowed),
            ),
        };
        // `CodeView` expands tabs itself; gpui would draw them raw.
        let expanded = expand_tabs(&shown.text, style.tab_size);
        let map = |r: &Range<usize>| expanded.map(r.start)..expanded.map(r.end);
        let marks: Vec<(Range<usize>, HighlightStyle)> = if preview.is_some() {
            let removed = HighlightStyle {
                background_color: Some(color(ui.diff_deleted_background).into()),
                strikethrough: Some(StrikethroughStyle {
                    thickness: px(1.),
                    color: None,
                }),
                ..Default::default()
            };
            let inserted = HighlightStyle {
                background_color: Some(color(ui.diff_added_background).into()),
                ..Default::default()
            };
            let old = shown.old.iter().map(|r| (map(r), removed));
            let new = shown.new.iter().map(|r| (map(r), inserted));
            old.chain(new).collect()
        } else {
            shown.old.iter().map(|r| (map(r), mark)).collect()
        };
        let spans: Option<LineSpans> =
            spans.map(|s| s.iter().map(|(r, c)| (map(r), c.clone())).collect());
        let highlights = line_highlights(&expanded.text, &marks, spans.as_ref(), theme);
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
        let group: SharedString = format!("{selector}-group").into();
        // Replace and Dismiss on a match line (see `RowActions`); a
        // context line has none.
        let actions = self
            .row_actions
            .filter(|_| !line.ranges.is_empty())
            .map(|actions| {
                let name = |what: &str| {
                    format!(
                        "{}-line-{what}-{}-{}-{}",
                        self.id_prefix, r.file, r.excerpt, r.line
                    )
                };
                let button = |what: &str, icon, label| {
                    let id = (SharedString::from(format!("line-{what}")), ix);
                    row_button(id.into(), name(what), icon, label, &group, selected, ui).h_full()
                };
                let replace = actions.replace.then(|| {
                    let anchor = anchor.clone();
                    button("replace", REPLACE, "Replace").on_click(cx.listener(
                        move |list, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(anchor) = &anchor {
                                list.emit_line(anchor, ExcerptListEvent::ReplaceLine, cx);
                            }
                        },
                    ))
                });
                let dismiss_anchor = anchor.clone();
                let dismiss =
                    button("dismiss", X, "Dismiss").on_click(cx.listener(move |list, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(anchor) = &dismiss_anchor {
                            list.emit_line(anchor, ExcerptListEvent::DismissLine, cx);
                        }
                    }));
                div()
                    .flex()
                    .flex_none()
                    .h_full()
                    .items_center()
                    .children(replace)
                    .child(dismiss)
            });
        let mut el = row(ix, selector, style).relative().group(group.clone());
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
        let previewed = preview.is_some();
        el.on_click(cx.listener(move |list, _, _, cx| {
            if let Some(anchor) = &anchor {
                list.open_anchor(anchor, cx);
            }
        }))
        .child(gutter)
        .child(
            div()
                .when(previewed, |d| {
                    d.debug_selector(move || format!("{}-preview-{}-{}-{}", id.0, id.1, id.2, id.3))
                })
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .font_family(style.font_family.clone())
                .text_size(style.font_size)
                .text_color(color(ui.editor_foreground))
                .child(styled),
        )
        .children(actions)
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
        let selected = self.selection();
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

/// The editor font size in pixels: `editor.fontSize` with the font zoom.
fn editor_font_size(settings: SettingsView<'_>, cx: &App) -> f32 {
    crate::font_zoom::zoomed(crate::font_zoom::base_size(settings, None), cx)
}

/// One editor line, as `CodeView` computes it from `editor.lineHeight`
/// (a factor of the font size below 8, else pixels), rounded to the
/// device pixels of `scale_factor`: layout snaps row heights to that grid,
/// and scrolling to a row sums these heights, so they must match what the
/// list measures.
pub(super) fn editor_line_height(
    scale_factor: f32,
    settings: SettingsView<'_>,
    cx: &App,
) -> Pixels {
    let size = editor_font_size(settings, cx);
    let lh = settings
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

/// The runs of one line: syntax colors from `spans` with the `marks` (the
/// match background, or a preview's old and new text styles) laid over
/// their ranges, which never overlap. Runs never overlap; where a mark and
/// a syntax span meet, the run takes the syntax style plus the mark.
/// Ranges are clamped and anything off a char boundary or empty (a match
/// past the 1,000-char cut) is dropped.
fn line_highlights(
    text: &str,
    marks: &[(Range<usize>, HighlightStyle)],
    spans: Option<&LineSpans>,
    theme: &lucerna_theme::Theme,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let valid = |r: &Range<usize>| {
        r.start < r.end && text.is_char_boundary(r.start) && text.is_char_boundary(r.end)
    };
    let matches: Vec<(Range<usize>, HighlightStyle)> = marks
        .iter()
        .map(|(r, style)| (r.start..r.end.min(text.len()), *style))
        .filter(|(r, _)| valid(r))
        .collect();
    let syntax: Vec<(Range<usize>, &lucerna_theme::HighlightStyle)> = spans
        .into_iter()
        .flatten()
        .filter(|(r, _)| valid(r) && r.end <= text.len())
        .filter_map(|(r, capture)| Some((r.clone(), theme.style_for(capture)?)))
        .collect();
    let mut edges: Vec<usize> = matches
        .iter()
        .flat_map(|(r, _)| [r.start, r.end])
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
                .find(|(r, _)| r.start <= seg.start && seg.end <= r.end)
                .map(|(_, mark)| *mark);
            if style.is_none() && marked.is_none() {
                return None;
            }
            let mut run = marked.unwrap_or_default();
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

/// A line as a replace preview draws it: the text with each replacement
/// put right after its match, the old (matched) and new (inserted) ranges
/// in that text, and the syntax spans moved with it.
struct PreviewLine<'a> {
    text: Cow<'a, str>,
    old: Vec<Range<usize>>,
    new: Vec<Range<usize>>,
}

/// The ranges kept of a line's matches: non-empty, in order, inside
/// `text` and on char boundaries, each with its replacement.
fn preview_ranges<'a>(
    text: &'a str,
    ranges: &'a [Range<usize>],
    replacements: &'a [Option<String>],
) -> impl Iterator<Item = (Range<usize>, Option<&'a str>)> + 'a {
    let mut cursor = 0;
    ranges
        .iter()
        .zip(
            replacements
                .iter()
                .map(Option::as_deref)
                .chain(std::iter::repeat(None)),
        )
        .filter_map(move |(r, new)| {
            let new = new.filter(|s| !s.is_empty());
            let ok = r.start < r.end
                && r.start >= cursor
                && r.end <= text.len()
                && text.is_char_boundary(r.start)
                && text.is_char_boundary(r.end);
            if !ok {
                return None;
            }
            cursor = r.end;
            Some((r.clone(), new))
        })
}

/// `text` with each replacement put right after its match. A range
/// without a replacement is only struck out.
fn preview_line(
    text: &str,
    ranges: &[Range<usize>],
    replacements: &[Option<String>],
) -> PreviewLine<'static> {
    let mut out = String::with_capacity(text.len());
    let (mut old, mut new) = (Vec::new(), Vec::new());
    let mut cursor = 0;
    for (r, replacement) in preview_ranges(text, ranges, replacements) {
        out.push_str(&text[cursor..r.start]);
        let start = out.len();
        out.push_str(&text[r.clone()]);
        old.push(start..out.len());
        if let Some(replacement) = replacement {
            let start = out.len();
            out.push_str(replacement);
            new.push(start..out.len());
        }
        cursor = r.end;
    }
    out.push_str(&text[cursor..]);
    PreviewLine {
        text: Cow::Owned(out),
        old,
        new,
    }
}

/// Syntax `spans` of the original line, moved into [`preview_line`]'s text:
/// an offset shifts by the replacements inserted before it. A span ending
/// at a match's end stops before that match's replacement; the inserted
/// text itself takes no syntax colors.
fn shift_spans(
    spans: &LineSpans,
    text: &str,
    ranges: &[Range<usize>],
    replacements: &[Option<String>],
) -> LineSpans {
    // (offset in the original text, replacement length inserted there)
    let inserts: Vec<(usize, usize)> = preview_ranges(text, ranges, replacements)
        .filter_map(|(r, new)| Some((r.end, new?.len())))
        .filter(|(_, len)| *len > 0)
        .collect();
    let shift = |offset: usize, inclusive: bool| {
        offset
            + inserts
                .iter()
                .filter(|(at, _)| {
                    if inclusive {
                        *at <= offset
                    } else {
                        *at < offset
                    }
                })
                .map(|(_, len)| len)
                .sum::<usize>()
    };
    let mut out = Vec::new();
    for (r, capture) in spans {
        // Split a span around each insertion inside it.
        let mut start = r.start;
        for &(at, _) in inserts
            .iter()
            .filter(|(at, _)| r.start < *at && *at < r.end)
        {
            out.push((shift(start, true)..shift(at, false), capture.clone()));
            start = at;
        }
        out.push((shift(start, true)..shift(r.end, false), capture.clone()));
    }
    out
}

/// A row's hover button (Replace, Dismiss): an icon shown while `group`
/// is hovered, or always when its row is `selected`. Swallows the mouse
/// down so the list keeps the focus; add its height and `on_click`.
fn row_button(
    id: ElementId,
    selector: String,
    icon: &'static [u8],
    label: &'static str,
    group: &SharedString,
    selected: bool,
    ui: &lucerna_theme::UiColors,
) -> Stateful<Div> {
    let hover = color(ui.ghost_element_hover);
    let group = group.clone();
    div()
        .id(id)
        .debug_selector(move || selector)
        .flex()
        .flex_none()
        .justify_center()
        .items_center()
        .w(px(6. * GRID))
        .rounded(px(GRID))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .when(!selected, |d| {
            d.opacity(0.).group_hover(group, |s| s.opacity(1.))
        })
        .tooltip(tooltip(label))
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .child(
            svg()
                .data(icon)
                .size(px(14.))
                .text_color(color(ui.icon_muted)),
        )
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
        let marks = [(1..3, mark), (3..5, mark)];
        let runs = line_highlights("fn main", &marks, Some(&spans), &theme);
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
        let mark = HighlightStyle::default();
        let runs = line_highlights(
            "é",
            &[(1..2, mark), (5..9, mark), (0..0, mark)],
            Some(&spans),
            &theme,
        );
        assert!(runs.is_empty(), "{runs:?}");
    }

    #[test]
    fn preview_puts_each_replacement_after_its_match() {
        let text = "a foo b foo c";
        let ranges = [2..5, 8..11];
        let line = preview_line(text, &ranges, &[Some("X".into()), None]);
        assert_eq!(line.text, "a fooX b foo c");
        assert_eq!(line.old, vec![2..5, 9..12]);
        assert_eq!(line.new, vec![5..6]);
        // Empty or bad ranges are skipped; an empty replacement inserts nothing.
        let line = preview_line(text, &[0..0, 2..5, 99..100], &[None, Some(String::new())]);
        assert_eq!(line.text, text);
        assert_eq!(line.old, vec![2..5]);
        assert!(line.new.is_empty());
    }

    #[test]
    fn spans_move_with_the_inserted_text() {
        let text = "let foo = 1";
        let ranges = [4..7, 11..11];
        let replacements = [Some("barbaz".to_string())];
        let spans: LineSpans = vec![
            (0..3, "keyword".into()),
            (4..7, "variable".into()),
            (2..9, "outer".into()),
            (10..11, "number".into()),
        ];
        let shifted = shift_spans(&spans, text, &ranges, &replacements);
        let line = preview_line(text, &ranges, &replacements);
        assert_eq!(line.text, "let foobarbaz = 1");
        let r = |name: &str| -> Vec<Range<usize>> {
            shifted
                .iter()
                .filter(|(_, c)| &**c == name)
                .map(|(r, _)| r.clone())
                .collect()
        };
        assert_eq!(r("keyword"), vec![0..3]);
        assert_eq!(r("variable"), vec![4..7], "the old text keeps its color");
        assert_eq!(r("outer"), vec![2..7, 13..15], "split around the insertion");
        assert_eq!(r("number"), vec![16..17]);
    }
}
