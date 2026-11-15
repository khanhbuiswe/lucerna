//! A language server's log tab (spec §7.7): read-only lines in the
//! editor's font, one tab per server, never saved.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, ClipboardItem, Context, Entity, FocusHandle, Focusable, IntoElement, KeyDownEvent,
    MouseButton, Render, Subscription, UniformListScrollHandle, Window, div, prelude::*, px,
    uniform_list,
};
use lucerna_lsp::LogKind;

use crate::editor_groups::{EditorGroups, TabItem};
use crate::language_servers::{LanguageServers, LanguageServersEvent};
use crate::layout::color;
use crate::ui::GRID;
use crate::{AppState, Workbench};

/// A server's log, the body of its log tab: read-only, in the editor's
/// font, following the end while scrolled to it. ⌘A then ⌘C copies it.
pub struct LspLogView {
    server: Arc<str>,
    servers: Entity<LanguageServers>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    /// ⌘A selected every line.
    all_selected: bool,
    _subscription: Subscription,
}

impl LspLogView {
    pub fn new(server: Arc<str>, servers: Entity<LanguageServers>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&servers, |view, _, ev, cx| {
            if let LanguageServersEvent::LogChanged(server) = ev
                && **server == *view.server
            {
                if view.scroll.is_scrolled_to_end().unwrap_or(true) {
                    view.scroll.scroll_to_bottom();
                }
                cx.notify();
            }
        });
        let scroll = UniformListScrollHandle::new();
        scroll.scroll_to_bottom();
        LspLogView {
            server,
            servers,
            focus: cx.focus_handle(),
            scroll,
            all_selected: false,
            _subscription: subscription,
        }
    }

    pub fn server(&self) -> &Arc<str> {
        &self.server
    }

    /// Every line of the log, oldest first.
    pub fn lines(&self, cx: &App) -> Vec<String> {
        let servers = self.servers.read(cx);
        servers
            .log_range(&self.server, 0..servers.log_len(&self.server))
            .into_iter()
            .map(|line| line.text)
            .collect()
    }

    /// What ⌘C copies: every line after ⌘A, otherwise nothing.
    pub fn selected_text(&self, cx: &App) -> Option<String> {
        self.all_selected.then(|| self.lines(cx).join("\n"))
    }

    fn key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let m = &ev.keystroke.modifiers;
        if ev.keystroke.key == "a" && m.platform && !(m.control || m.alt || m.shift) {
            self.all_selected = true;
            cx.notify();
            cx.stop_propagation();
        }
    }

    fn render_lines(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<gpui::Div> {
        let ui = &cx.global::<AppState>().theme.ui;
        let (text, muted) = (color(ui.text), color(ui.text_muted));
        let selected = self.all_selected.then(|| color(ui.selection));
        self.servers
            .read(cx)
            .log_range(&self.server, range)
            .into_iter()
            .map(|line| {
                div()
                    .px(px(3. * GRID))
                    .whitespace_nowrap()
                    .text_color(if line.kind == LogKind::Lifecycle {
                        muted
                    } else {
                        text
                    })
                    .when_some(selected, |d, bg| d.bg(bg))
                    .child(line.text)
            })
            .collect()
    }
}

impl Focusable for LspLogView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for LspLogView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = &cx.global::<AppState>().settings;
        let font = crate::fonts::editor_font_family(
            cx.global::<crate::AppState>().settings.view(None),
            None,
        );
        let size = settings
            .get::<f32>("editor.fontSize", None)
            .unwrap_or(14.)
            .clamp(6., 40.);
        let count = self.servers.read(cx).log_len(&self.server);
        div()
            .id("lsp-log")
            .debug_selector(|| "lsp-log".into())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, window, cx| {
                    view.all_selected = false;
                    window.focus(&view.focus, cx);
                    cx.notify();
                }),
            )
            .size_full()
            .py(px(GRID))
            .bg(color(cx.global::<AppState>().theme.ui.editor_background))
            .font_family(font)
            .text_size(px(size))
            .line_height(px((size * 1.5).round()))
            .child(
                uniform_list(
                    "lsp-log-lines",
                    count,
                    cx.processor(|view, range, _, cx| view.render_lines(range, cx)),
                )
                .size_full()
                .track_scroll(&self.scroll),
            )
    }
}

/// The log views, one per server, kept while their tabs are closed.
/// Shared by the workbench and the tab body factory.
#[derive(Clone, Default)]
pub(crate) struct LspLogs(Rc<RefCell<HashMap<Arc<str>, Entity<LspLogView>>>>);

impl LspLogs {
    /// The log view of `server`, made on first use.
    pub(crate) fn view(
        &self,
        server: &Arc<str>,
        servers: &Entity<LanguageServers>,
        cx: &mut App,
    ) -> Entity<LspLogView> {
        if let Some(view) = self.0.borrow().get(server) {
            return view.clone();
        }
        let view = cx.new(|cx| LspLogView::new(server.clone(), servers.clone(), cx));
        self.0.borrow_mut().insert(server.clone(), view.clone());
        view
    }

    /// Whether a log view whose tab is no longer shown has focus.
    pub(crate) fn hidden_with_focus(
        &self,
        model: &EditorGroups,
        window: &Window,
        cx: &App,
    ) -> bool {
        self.0.borrow().iter().any(|(server, view)| {
            let shown = model.groups().iter().any(|g| {
                g.tabs
                    .iter()
                    .any(|t| g.active == Some(t.id) && t.item == TabItem::LspLog(server.clone()))
            });
            !shown && view.read(cx).focus.contains_focused(window, cx)
        })
    }
}

/// `editor.action.clipboardCopyAction` in a log tab: copies the log after
/// ⌘A. Returns whether the active tab is a log.
pub(crate) fn copy_from_log(wb: &Workbench, cx: &mut App) -> bool {
    let body = wb.editor_groups().read(cx).active_body();
    let Some(view) = body.and_then(|b| b.downcast::<LspLogView>().ok()) else {
        return false;
    };
    if let Some(text) = view.read(cx).selected_text(cx) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }
    true
}

impl Workbench {
    /// Opens (or focuses) the log tab of `server` and focuses it.
    pub fn show_lsp_log(&mut self, server: Arc<str>, window: &mut Window, cx: &mut Context<Self>) {
        let view = self.lsp_logs.view(&server, self.language_servers(), cx);
        self.update_editor_groups(cx, |m| m.open_lsp_log(server));
        view.update(cx, |view, cx| {
            view.scroll.scroll_to_bottom();
            window.focus(&view.focus, cx);
        });
        cx.notify();
    }
}
