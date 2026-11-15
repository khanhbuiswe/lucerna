//! LSP hover (spec §7.2): one [`HoverController`] per [`CodeView`] times
//! the request and the popover, which the code view draws over its text.
//!
//! The mouse resting on a char asks the server after half of
//! `editor.hover.delay` and shows the answer once the whole delay has
//! passed (or when it arrives, if later). Moving to another word first
//! drops the request, which cancels it. A shown hover stays while the mouse
//! is in its range or, with `editor.hover.sticky`, in the popover; leaving
//! hides it after `editor.hover.hidingDelay`. Diagnostics under the point
//! are listed above the answer.

use std::ops::Range;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyElement, App, AvailableSpace, Bounds, Context, Entity, InteractiveElement, IntoElement,
    ParentElement, Pixels, Styled, Task, WeakEntity, Window, div, point, prelude::*, px, size,
};
use lucerna_lsp::diagnostics::{Diagnostic, Severity};
use lucerna_settings::SettingsView;
use lucerna_text::{BufferSnapshot, Point};

use crate::code_view::{CodeView, word_at};
use crate::language_servers::{HoverInfo, LanguageServers};
use crate::layout::color;
use crate::markdown::{
    LinkHandler, MarkdownBlocks, open_markdown_link, parse_markdown, render_markdown_with,
};
use crate::tasks::guarded;
use crate::ui::{UI_FONT_FAMILY, UI_FONT_SIZE_SMALL, popover};
use crate::{AppState, CommandRegistry, Workbench};

/// The popover's padding, and its distance from the editor's right edge.
const PADDING: f32 = 8.;
/// Between the diagnostic blocks and the answer.
const BLOCK_GAP: f32 = 6.;
/// Between the popover and the hovered line.
const LINE_GAP: f32 = 4.;
const MIN_WIDTH_CHARS: f32 = 20.;
const MAX_WIDTH_CHARS: f32 = 120.;
const MAX_HEIGHT_LINES: f32 = 16.;

/// Shows the hover at the cursor; the one key command that keeps it.
pub(crate) const SHOW_HOVER: &str = "editor.action.showHover";

/// The `editor.hover.*` settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HoverConfig {
    pub enabled: bool,
    pub delay: Duration,
    pub sticky: bool,
    pub hiding_delay: Duration,
}

impl HoverConfig {
    /// The settings for a view of `language`.
    pub fn read(settings: SettingsView<'_>, language: Option<&str>) -> HoverConfig {
        let ms = |key: &str, default: u64| {
            Duration::from_millis(settings.get::<u64>(key, language).unwrap_or(default))
        };
        HoverConfig {
            enabled: settings
                .get("editor.hover.enabled", language)
                .unwrap_or(true),
            delay: ms("editor.hover.delay", 300),
            sticky: settings
                .get("editor.hover.sticky", language)
                .unwrap_or(true),
            hiding_delay: ms("editor.hover.hidingDelay", 300),
        }
    }
}

/// A hover waiting for its delay or its answer.
struct Pending {
    point: Point,
    /// The word under `point`, once the request is sent.
    word: Option<Range<Point>>,
    _task: Task<()>,
}

/// The hover on screen (or, from [`HoverController::show_now`], waiting
/// for content).
struct Shown {
    point: Point,
    /// Keeps a mouse hover open while the mouse is in it; painted with
    /// `element.hover`.
    range: Range<Point>,
    info: Option<HoverInfo>,
    blocks: Option<Arc<MarkdownBlocks>>,
    /// The diagnostics containing `point`, narrowest first.
    diagnostics: Vec<Diagnostic>,
    /// Shown by the mouse: leaving its range hides it.
    by_mouse: bool,
    /// The request of a hover shown at once.
    _request: Option<Task<()>>,
}

pub struct HoverController {
    path: Arc<Path>,
    config: HoverConfig,
    snapshot: BufferSnapshot,
    file_diagnostics: Arc<[Diagnostic]>,
    servers: Option<WeakEntity<LanguageServers>>,
    workbench: Option<WeakEntity<Workbench>>,
    pending: Option<Pending>,
    shown: Option<Shown>,
    hide_task: Option<Task<()>>,
    /// The popover as last drawn, in window coordinates.
    popover_bounds: Option<Bounds<Pixels>>,
}

impl HoverController {
    pub(crate) fn new(path: Arc<Path>, config: HoverConfig, snapshot: BufferSnapshot) -> Self {
        HoverController {
            path,
            config,
            snapshot,
            file_diagnostics: Arc::from([]),
            servers: None,
            workbench: None,
            pending: None,
            shown: None,
            hide_task: None,
            popover_bounds: None,
        }
    }

    pub(crate) fn set_config(&mut self, config: HoverConfig) {
        self.config = config;
    }

    /// Where answers come from and where links in them go.
    pub(crate) fn set_sources(
        &mut self,
        workbench: WeakEntity<Workbench>,
        servers: WeakEntity<LanguageServers>,
    ) {
        self.workbench = Some(workbench);
        self.servers = Some(servers);
    }

    /// The view shows `snapshot`: a visible hover is about the old text
    /// and hides; one still waiting for content keeps its point (a jump
    /// into a file that was loading).
    pub(crate) fn set_snapshot(&mut self, snapshot: BufferSnapshot, cx: &mut Context<CodeView>) {
        self.snapshot = snapshot;
        self.pending = None;
        if self.is_visible() {
            self.hide(cx);
        }
        if let Some(shown) = &mut self.shown
            && shown.info.is_none()
        {
            shown.range = word_at(&self.snapshot, shown.point);
        }
    }

    /// The file was renamed: requests go to its new path; an answer about
    /// the old one hides.
    pub(crate) fn set_path(&mut self, path: Arc<Path>, cx: &mut Context<CodeView>) {
        self.path = path;
        self.hide(cx);
    }

    /// The view's diagnostics changed: the shown blocks follow.
    pub(crate) fn set_diagnostics(&mut self, diagnostics: Arc<[Diagnostic]>) {
        self.file_diagnostics = diagnostics;
        if let Some(point) = self.shown.as_ref().map(|s| s.point) {
            let diagnostics = self.diagnostics_at(point);
            if let Some(shown) = &mut self.shown {
                shown.diagnostics = diagnostics;
            }
        }
    }

    /// The mouse moved to `mouse` (window coordinates); `point` is the
    /// char with text under it, if any.
    pub fn mouse_moved(
        &mut self,
        point: Option<Point>,
        mouse: gpui::Point<Pixels>,
        cx: &mut Context<CodeView>,
    ) {
        let visible = self.is_visible();
        let in_popover = self.popover_bounds.is_some_and(|b| b.contains(&mouse));
        let in_range = point.is_some_and(|p| {
            self.shown
                .as_ref()
                .is_some_and(|s| visible && char_in(&s.range, p))
        });
        if visible && (in_range || (self.config.sticky && in_popover)) {
            // A word crossed on the way (the strip between the line and
            // the popover is the next row's) must not replace this one.
            self.pending = None;
            self.hide_task = None;
            return;
        }
        if self.shown.as_ref().is_some_and(|s| s.by_mouse) {
            self.begin_hide(cx);
        }
        if !self.config.enabled {
            self.pending = None;
            return;
        }
        let Some(p) = point else {
            self.pending = None;
            return;
        };
        if let Some(pending) = &self.pending
            && (pending.point == p || pending.word.as_ref().is_some_and(|w| char_in(w, p)))
        {
            return;
        }
        let task = self.schedule(p, cx);
        self.pending = Some(Pending {
            point: p,
            word: None,
            _task: task,
        });
    }

    /// Shows the hover at `point` now: the diagnostics there at once, the
    /// server's answer when it arrives (`editor.action.showHover`, F8).
    pub fn show_now(&mut self, point: Point, cx: &mut Context<CodeView>) {
        self.pending = None;
        self.hide_task = None;
        let request = self.request(point, cx);
        let task = cx.spawn(async move |view, cx| {
            let answer = answer(request, cx).await;
            view.update(cx, |v, cx| {
                let hover = v.hover_mut();
                if let Some(shown) = hover.shown.as_mut().filter(|s| s.point == point) {
                    if let Some((info, blocks)) = answer {
                        if let Some(range) = info.range.clone() {
                            shown.range = range;
                        }
                        shown.info = Some(info);
                        shown.blocks = Some(blocks);
                    }
                    shown._request = None;
                    cx.notify();
                }
            })
            .ok();
        });
        self.shown = Some(Shown {
            point,
            range: word_at(&self.snapshot, point),
            info: None,
            blocks: None,
            diagnostics: self.diagnostics_at(point),
            by_mouse: false,
            _request: Some(task),
        });
        cx.notify();
    }

    /// Hides the popover and drops any request.
    pub fn hide(&mut self, cx: &mut Context<CodeView>) {
        self.pending = None;
        self.hide_task = None;
        self.popover_bounds = None;
        if self.shown.take().is_some() {
            cx.notify();
        }
    }

    /// Whether a hover is shown, waiting or hiding.
    pub(crate) fn is_busy(&self) -> bool {
        self.shown.is_some() || self.pending.is_some() || self.hide_task.is_some()
    }

    /// Whether the popover shows: it has an answer or diagnostics.
    pub fn is_visible(&self) -> bool {
        self.shown
            .as_ref()
            .is_some_and(|s| s.info.is_some() || !s.diagnostics.is_empty())
    }

    /// The server's answer shown.
    pub fn info(&self) -> Option<&HoverInfo> {
        self.shown.as_ref().and_then(|s| s.info.as_ref())
    }

    /// The diagnostics shown above the answer, narrowest first.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        self.shown.as_ref().map_or(&[], |s| &s.diagnostics)
    }

    /// The range painted with `element.hover` while the popover shows.
    pub fn highlight_range(&self) -> Option<Range<Point>> {
        self.shown
            .as_ref()
            .filter(|_| self.is_visible())
            .map(|s| s.range.clone())
    }

    /// Where the popover is anchored: the start of the range when on the
    /// hovered row, else the point.
    pub(crate) fn anchor(&self) -> Option<Point> {
        let shown = self.shown.as_ref().filter(|_| self.is_visible())?;
        Some(if shown.range.start.row == shown.point.row {
            shown.range.start
        } else {
            shown.point
        })
    }

    pub(crate) fn popover_bounds(&self) -> Option<Bounds<Pixels>> {
        self.popover_bounds
    }

    pub(crate) fn set_popover_bounds(&mut self, bounds: Option<Bounds<Pixels>>) {
        self.popover_bounds = bounds;
    }

    /// Without `sticky`, hides now; else after the hiding delay, unless
    /// the mouse comes back first.
    fn begin_hide(&mut self, cx: &mut Context<CodeView>) {
        if !self.config.sticky {
            self.hide(cx);
            return;
        }
        if self.hide_task.is_some() {
            return;
        }
        let delay = self.config.hiding_delay;
        self.hide_task = Some(cx.spawn(async move |view, cx| {
            cx.background_executor().timer(delay).await;
            view.update(cx, |v, cx| {
                let hover = v.hover_mut();
                hover.hide_task = None;
                // A new hover may have replaced the one leaving.
                if hover.shown.as_ref().is_some_and(|s| s.by_mouse) {
                    hover.shown = None;
                    hover.popover_bounds = None;
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// Asks at half the delay; shows at the full delay or on arrival.
    fn schedule(&mut self, p: Point, cx: &mut Context<CodeView>) -> Task<()> {
        let delay = self.config.delay;
        cx.spawn(async move |view, cx| {
            cx.background_executor().timer(delay / 2).await;
            let Ok(request) = view.update(cx, |v, cx| {
                let hover = v.hover_mut();
                let word = word_at(&hover.snapshot, p);
                if let Some(pending) = &mut hover.pending {
                    pending.word = Some(word);
                }
                hover.request(p, cx)
            }) else {
                return;
            };
            let show_at = cx.background_executor().timer(delay - delay / 2);
            let answer = answer(request, cx).await;
            show_at.await;
            view.update(cx, |v, cx| v.hover_mut().finish(p, answer, cx))
                .ok();
        })
    }

    /// The pending hover at `p` got `answer`: shows it, with the
    /// diagnostics there; nothing when both are empty.
    fn finish(
        &mut self,
        p: Point,
        answer: Option<(HoverInfo, Arc<MarkdownBlocks>)>,
        cx: &mut Context<CodeView>,
    ) {
        let Some(pending) = self.pending.take_if(|pending| pending.point == p) else {
            return;
        };
        let diagnostics = self.diagnostics_at(p);
        if answer.is_none() && diagnostics.is_empty() {
            return;
        }
        let word = pending.word.unwrap_or_else(|| word_at(&self.snapshot, p));
        let range = answer
            .as_ref()
            .and_then(|(info, _)| info.range.clone())
            .unwrap_or(word);
        let (info, blocks) = answer.unzip();
        self.shown = Some(Shown {
            point: p,
            range,
            info,
            blocks,
            diagnostics,
            by_mouse: true,
            _request: None,
        });
        self.hide_task = None;
        cx.notify();
    }

    /// The hover request for `p`, if a server can answer.
    fn request(&self, p: Point, cx: &mut Context<CodeView>) -> Task<Option<HoverInfo>> {
        let servers = self.servers.as_ref().and_then(WeakEntity::upgrade);
        match servers {
            Some(servers) => servers.update(cx, |s, cx| s.hover(&self.path, p, cx)),
            None => Task::ready(None),
        }
    }

    /// The diagnostics whose range holds `p`, narrowest first.
    fn diagnostics_at(&self, p: Point) -> Vec<Diagnostic> {
        let mut found: Vec<&Diagnostic> = self
            .file_diagnostics
            .iter()
            .filter(|d| char_in(&d.range, p))
            .collect();
        let len = |d: &Diagnostic| {
            let offset = |p: Point| self.snapshot.point_to_offset(p);
            offset(d.range.end).saturating_sub(offset(d.range.start))
        };
        found.sort_by_key(|d| (len(d), d.severity));
        found.into_iter().cloned().collect()
    }
}

/// Whether the char at `p` lies in `range` (an empty range holds the char
/// at its start).
fn char_in(range: &Range<Point>, p: Point) -> bool {
    let key = |p: Point| (p.row, p.column);
    let (start, end, p) = (key(range.start), key(range.end), key(p));
    start <= p && (p < end || p == start)
}

/// Awaits `request` and parses its markdown, off the UI thread.
async fn answer(
    request: Task<Option<HoverInfo>>,
    cx: &mut gpui::AsyncApp,
) -> Option<(HoverInfo, Arc<MarkdownBlocks>)> {
    let info = request.await?;
    let source = info.markdown.clone();
    let language = info.language.clone();
    let blocks = cx
        .background_spawn(async move {
            guarded("hover-markdown", || {
                parse_markdown(&source, language.as_deref())
            })
        })
        .await?;
    Some((info, Arc::new(blocks)))
}

/// Where the code view puts the popover: in window coordinates.
pub(crate) struct PopoverAnchor {
    /// The code view's text and gutter.
    pub editor: Bounds<Pixels>,
    /// The left of the anchor char and the top of its line.
    pub x: Pixels,
    pub line_top: Pixels,
    pub line_height: Pixels,
    pub char_width: Pixels,
}

/// Lays out the popover of `view` and draws it over everything, above the
/// anchor line when it fits, else below; 8 px from the editor's right
/// edge. Call while the code view prepaints.
pub(crate) fn prepaint_popover(
    view: &Entity<CodeView>,
    anchor: Option<PopoverAnchor>,
    window: &mut Window,
    cx: &mut App,
) {
    let element = anchor.and_then(|a| {
        let hover = view.read(cx).hover();
        let max_w = (a.editor.size.width / 2.).min(a.char_width * MAX_WIDTH_CHARS);
        let min_w = (a.char_width * MIN_WIDTH_CHARS).min(max_w);
        let max_h = (a.editor.size.height / 2.).min(a.line_height * MAX_HEIGHT_LINES);
        Some((hover.render_popover(min_w, max_w, max_h, cx)?, a, max_w))
    });
    let Some((mut element, a, max_w)) = element else {
        if view.read(cx).hover().popover_bounds().is_some() {
            view.update(cx, |v, _| v.hover_mut().set_popover_bounds(None));
        }
        return;
    };
    let bounds = place_popover(&mut element, &a, max_w, window, cx);
    view.update(cx, |v, _| v.hover_mut().set_popover_bounds(Some(bounds)));
    window.defer_draw(element, bounds.origin, 1, None);
}

/// Lays out `element` at most `max_width` wide and finds its place by
/// the anchor: above the anchor line when it fits, else below; 8 px from
/// the editor's right edge. Text wraps at the width it is drawn at, so
/// the height (and what is left to scroll) counts the wrapped lines.
pub(crate) fn place_popover(
    element: &mut AnyElement,
    a: &PopoverAnchor,
    max_width: Pixels,
    window: &mut Window,
    cx: &mut App,
) -> Bounds<Pixels> {
    let size = element.layout_as_root(
        size(
            AvailableSpace::Definite(max_width),
            AvailableSpace::Definite(a.editor.size.height),
        ),
        window,
        cx,
    );
    let above = a.line_top - px(LINE_GAP) - size.height;
    let y = if above >= a.editor.top() {
        above
    } else {
        a.line_top + a.line_height + px(LINE_GAP)
    };
    let x =
        a.x.min(a.editor.right() - px(PADDING) - size.width)
            .max(a.editor.left());
    Bounds {
        origin: point(x, y),
        size,
    }
}

impl HoverController {
    /// The popover: diagnostic blocks, then the answer; `None` when
    /// hidden.
    fn render_popover(
        &self,
        min_w: Pixels,
        max_w: Pixels,
        max_h: Pixels,
        cx: &App,
    ) -> Option<AnyElement> {
        let shown = self.shown.as_ref().filter(|_| self.is_visible())?;
        let workbench = self.workbench.clone();
        let on_link: LinkHandler = Rc::new(move |url, _, cx| {
            if let Some(wb) = workbench.as_ref().and_then(WeakEntity::upgrade) {
                wb.update(cx, |wb, cx| open_markdown_link(url, wb, cx));
            }
        });
        let blocks = (!shown.diagnostics.is_empty()).then(|| {
            div().flex().flex_col().gap(px(4.)).children(
                shown
                    .diagnostics
                    .iter()
                    .enumerate()
                    .map(|(i, d)| diagnostic_block(i, d, on_link.clone(), cx)),
            )
        });
        let info = shown.blocks.as_ref().map(|blocks| {
            div()
                .debug_selector(|| "hover-markdown".into())
                .min_w_0()
                .child(render_markdown_with(blocks, on_link.clone(), cx))
        });
        Some(
            popover(cx)
                .id("hover-popover")
                .debug_selector(|| "hover-popover".into())
                .occlude()
                .w_auto()
                .min_w(min_w)
                .max_w(max_w)
                .max_h(max_h)
                .overflow_y_scroll()
                .p(px(PADDING))
                .gap(px(BLOCK_GAP))
                .font_family(UI_FONT_FAMILY)
                .text_size(px(UI_FONT_SIZE_SMALL))
                .children(blocks)
                .children(info)
                .into_any_element(),
        )
    }
}

/// One diagnostic: its message, then a muted `source(code)` line whose
/// code links to its description.
fn diagnostic_block(i: usize, d: &Diagnostic, on_link: LinkHandler, cx: &App) -> AnyElement {
    let ui = &cx.global::<AppState>().theme.ui;
    let (background, border) = match d.severity {
        Severity::Error => (ui.error_background, ui.error_border),
        Severity::Warning => (ui.warning_background, ui.warning_border),
        Severity::Information => (ui.info_background, ui.info_border),
        Severity::Hint => (ui.hint_background, ui.hint_border),
    };
    let code = d.code.clone().map(|code| {
        let link = d.code_href.clone();
        div()
            .id(("hover-diagnostic-code", i))
            .child(code)
            .when_some(link, |el, url| {
                el.underline()
                    .cursor_pointer()
                    .on_click(move |_, window, cx| on_link(&url, window, cx))
            })
    });
    let source = match (&d.source, code) {
        (None, None) => None,
        (source, code) => Some(
            div()
                .debug_selector(move || format!("hover-diagnostic-source-{i}"))
                .flex()
                .flex_row()
                .text_color(color(ui.text_muted))
                .children(source.clone())
                .when_some(code, |el, code| {
                    let wrap = d.source.is_some();
                    el.when(wrap, |el| el.child("("))
                        .child(code)
                        .when(wrap, |el| el.child(")"))
                }),
        ),
    };
    div()
        .debug_selector(move || format!("hover-diagnostic-{i}"))
        .flex()
        .flex_col()
        .gap(px(2.))
        .px(px(8.))
        .py(px(4.))
        .rounded(px(4.))
        .border_1()
        .border_color(color(border))
        .bg(color(background))
        .child(div().child(d.message.clone()))
        .children(source)
        .into_any_element()
}

impl Workbench {
    /// Hides the hover of every code view whose tab is no longer on
    /// screen: switching tabs need not move focus out of the old view.
    pub(crate) fn hide_hidden_hovers(&mut self, cx: &mut Context<Workbench>) {
        let views: Vec<Entity<CodeView>> = self
            .editor_groups()
            .read(cx)
            .hidden_bodies()
            .filter_map(|body| body.clone().downcast::<CodeView>().ok())
            .filter(|view| view.read(cx).hover().is_busy())
            .collect();
        for view in views {
            view.update(cx, |v, cx| v.hover_mut().hide(cx));
        }
    }
}

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register(SHOW_HOVER, "Show or Focus Hover", |wb, _, _, cx| {
        if let Some(view) = wb.active_code_view(cx) {
            view.update(cx, |v, cx| {
                let cursor = v.cursor();
                v.hover_mut().show_now(cursor, cx);
            });
        }
    });
    commands.register("editor.action.hideHover", "Hide Hover", |wb, _, _, cx| {
        if let Some(view) = wb.active_code_view(cx) {
            view.update(cx, |v, cx| v.hover_mut().hide(cx));
        }
    });
}
