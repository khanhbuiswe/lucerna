//! The language servers' status (spec §7.7): the ⚡ button at the status
//! bar's right with its dot, the progress text beside it, the popover
//! listing the servers, and the `lucerna.lsp.*` commands. The log tab is
//! in `lsp_log`.

use std::sync::Arc;

use gpui::{
    Anchor, App, Context, Entity, FocusHandle, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, Pixels, Point, Render, Subscription, WeakEntity, Window, anchored, deferred,
    div, point, prelude::*, px, svg,
};
use lucerna_lsp::ServerKey;
use lucerna_theme::Rgba;
use serde_json::Value;

use crate::editor_groups::path_bar_text;
use crate::icons;
use crate::language_servers::{LanguageServers, LanguageServersEvent, ServerStatus, SlotState};
use crate::layout::color;
use crate::ui::{
    GRID, STATUS_BAR_HEIGHT, TITLE_BAR_HEIGHT, UI_FONT_SIZE_CAPTION, UI_FONT_SIZE_SMALL,
    icon_button, popover, text_button, tooltip,
};
use crate::{AppState, CommandRegistry, Workbench};

const POPOVER_WIDTH: f32 = 420.;
const DOT_SIZE: f32 = 6.;
/// Stderr lines a failed server shows in the popover; the log has all.
const POPOVER_STDERR_LINES: usize = 5;

/// The dot on the ⚡ button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusDot {
    /// A server is not found, or the folder is in Restricted Mode.
    Warning,
    /// A server failed.
    Error,
}

/// A server as the popover lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerRow {
    pub key: ServerKey,
    pub state: SlotState,
    /// Shown under the row: the not-found or failure message (§4) with the
    /// last 5 lines of stderr, then `window/showMessage` errors.
    pub lines: Vec<String>,
    /// Stderr lines left out of `lines` (they are in the log).
    pub more_lines: usize,
}

impl ServerRow {
    fn new(status: ServerStatus) -> ServerRow {
        let mut more_lines = 0;
        let mut lines = match &status.state {
            SlotState::NotFound { message } => vec![message.clone()],
            SlotState::Failed {
                message,
                stderr_tail,
            } => {
                more_lines = stderr_tail.len().saturating_sub(POPOVER_STDERR_LINES);
                std::iter::once(message.clone())
                    .chain(stderr_tail[more_lines..].iter().cloned())
                    .collect()
            }
            SlotState::Starting | SlotState::Running | SlotState::Stopped => Vec::new(),
        };
        lines.extend(status.errors);
        ServerRow {
            key: status.key,
            state: status.state,
            lines,
            more_lines,
        }
    }
}

/// The ⚡ status bar item: the button, the progress text and the popover.
pub struct LspStatusItem {
    workbench: WeakEntity<Workbench>,
    servers: Entity<LanguageServers>,
    open: bool,
    focus: FocusHandle,
    focus_before: Option<FocusHandle>,
    /// Where the press that closed the popover from outside was: a press
    /// on the button itself closes it without opening it again.
    closed_at: Option<Point<Pixels>>,
    /// The current press on the button closed the popover.
    press_closed: bool,
    _subscription: Subscription,
}

impl LspStatusItem {
    pub fn new(
        workbench: WeakEntity<Workbench>,
        servers: Entity<LanguageServers>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.subscribe(&servers, |_, _, ev, cx| {
            if *ev == LanguageServersEvent::StatusChanged {
                cx.notify();
            }
        });
        LspStatusItem {
            workbench,
            servers,
            open: false,
            focus: cx.focus_handle(),
            focus_before: None,
            closed_at: None,
            press_closed: false,
            _subscription: subscription,
        }
    }

    /// Red when a server failed; yellow when one is not found or the
    /// folder is in Restricted Mode; `None` when all is well.
    pub fn dot(&self, cx: &App) -> Option<StatusDot> {
        let servers = self.servers.read(cx);
        let statuses = servers.statuses();
        if statuses
            .iter()
            .any(|s| matches!(s.state, SlotState::Failed { .. }))
        {
            Some(StatusDot::Error)
        } else if servers.shows_restricted_mode()
            || statuses
                .iter()
                .any(|s| matches!(s.state, SlotState::NotFound { .. }))
        {
            Some(StatusDot::Warning)
        } else {
            None
        }
    }

    /// The popover's rows, ordered by root then server name.
    pub fn rows(&self, cx: &App) -> Vec<ServerRow> {
        self.servers
            .read(cx)
            .statuses()
            .into_iter()
            .map(ServerRow::new)
            .collect()
    }

    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = true;
        self.focus_before = window.focused(cx);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !std::mem::take(&mut self.open) {
            return;
        }
        let focus_before = self.focus_before.take();
        if self.focus.contains_focused(window, cx) {
            match focus_before {
                Some(focus) => window.focus(&focus, cx),
                None => {
                    let workbench = self.workbench.clone();
                    window.defer(cx, move |window, cx| {
                        workbench
                            .update(cx, |wb, cx| wb.focus_editor_area(window, cx))
                            .ok();
                    });
                }
            }
        }
        cx.notify();
    }

    fn button_pressed(&mut self, ev: &MouseDownEvent) {
        self.press_closed = self.closed_at.take() == Some(ev.position);
    }

    fn button_clicked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.press_closed) {
            return;
        }
        if self.open {
            self.close(window, cx);
        } else {
            self.open(window, cx);
        }
    }

    fn key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let m = &ev.keystroke.modifiers;
        if ev.keystroke.key == "escape" && !(m.control || m.alt || m.shift || m.platform) {
            self.close(window, cx);
            cx.stop_propagation();
        }
    }

    /// Closes the popover, then runs `f` on the workbench.
    fn act(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Workbench, &mut Window, &mut Context<Workbench>) + 'static,
    ) {
        self.close(window, cx);
        let workbench = self.workbench.clone();
        window.defer(cx, move |window, cx| {
            workbench.update(cx, |wb, cx| f(wb, window, cx)).ok();
        });
    }

    /// The popover, at most `max_height` tall: its rows scroll, the
    /// footer stays.
    fn render_popover(
        &self,
        max_height: Pixels,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let ui = cx.global::<AppState>().theme.ui.clone();
        let muted = color(ui.text_muted);
        let home = std::env::home_dir();
        let restricted = self.servers.read(cx).shows_restricted_mode();
        let rows = self.rows(cx);
        let has_rows = !rows.is_empty();
        let rows = rows.into_iter().enumerate().map(|(i, row)| {
            let (key, server) = (row.key.clone(), Arc::<str>::from(row.key.server));
            let root = path_bar_text(&row.key.root, None, home.as_deref());
            let lines = row.lines.iter().enumerate().map(|(j, line)| {
                div()
                    .debug_selector(move || format!("lsp-row-message-{i}-{j}"))
                    .pl(px(DOT_SIZE + 2. * GRID))
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(UI_FONT_SIZE_CAPTION))
                    .text_color(muted)
                    .child(line.clone())
            });
            let more = (row.more_lines > 0).then(|| {
                let server = server.clone();
                div()
                    .id(("lsp-row-more", i))
                    .debug_selector(move || format!("lsp-row-more-{i}"))
                    .ml(px(DOT_SIZE + 2. * GRID))
                    .cursor_pointer()
                    .text_size(px(UI_FONT_SIZE_CAPTION))
                    .text_color(color(ui.text))
                    .hover(|s| s.underline())
                    .child("Show Log")
                    .on_click(cx.listener(move |item, _, window, cx| {
                        let server = server.clone();
                        item.act(window, cx, move |wb, window, cx| {
                            wb.show_lsp_log(server, window, cx)
                        });
                    }))
            });
            let restart_key = key.clone();
            div()
                .debug_selector(move || format!("lsp-row-{i}"))
                .flex()
                .flex_col()
                .gap(px(GRID))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(2. * GRID))
                        .child(dot(state_color(&row.state, &ui)))
                        .child(div().flex_none().child(row.key.server))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(px(UI_FONT_SIZE_CAPTION))
                                .text_color(muted)
                                .child(root),
                        )
                        .child(
                            text_button(format!("lsp-restart-{i}"), "Restart", cx).on_click(
                                cx.listener(move |item, _, window, cx| {
                                    let key = restart_key.clone();
                                    item.close(window, cx);
                                    item.servers.update(cx, |s, cx| s.restart(&key, cx));
                                }),
                            ),
                        )
                        .child(text_button(format!("lsp-stop-{i}"), "Stop", cx).on_click(
                            cx.listener(move |item, _, window, cx| {
                                item.close(window, cx);
                                item.servers.update(cx, |s, cx| s.stop(&key, cx));
                            }),
                        ))
                        .child(text_button(format!("lsp-log-{i}"), "Log", cx).on_click(
                            cx.listener(move |item, _, window, cx| {
                                let server = server.clone();
                                item.act(window, cx, move |wb, window, cx| {
                                    wb.show_lsp_log(server, window, cx)
                                });
                            }),
                        )),
                )
                .children(lines)
                .children(more)
        });
        let hover = color(ui.ghost_element_hover);
        let restricted_row = restricted.then(|| {
            div()
                .id("lsp-restricted")
                .debug_selector(|| "lsp-restricted".into())
                .flex()
                .flex_row()
                .items_center()
                .gap(px(2. * GRID))
                .px(px(GRID))
                .py(px(GRID / 2.))
                .rounded(px(GRID))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(
                    svg()
                        .data(icons::TRIANGLE_ALERT)
                        .flex_none()
                        .size(px(UI_FONT_SIZE_SMALL))
                        .text_color(color(ui.warning)),
                )
                .child("Restricted Mode — Trust this folder…")
                .on_click(cx.listener(|item, _, window, cx| {
                    item.act(window, cx, |wb, window, cx| wb.show_trust_modal(window, cx));
                }))
        });
        let footer = has_rows.then(|| {
            div()
                .flex_none()
                .flex()
                .flex_row()
                .justify_end()
                .gap(px(2. * GRID))
                .px(px(3. * GRID))
                .py(px(2. * GRID))
                .border_t_1()
                .border_color(color(ui.border_variant))
                .child(
                    text_button("lsp-restart-all", "Restart All", cx).on_click(cx.listener(
                        |item, _, window, cx| {
                            item.close(window, cx);
                            item.servers.update(cx, LanguageServers::restart_all);
                        },
                    )),
                )
                .child(
                    text_button("lsp-stop-all", "Stop All", cx).on_click(cx.listener(
                        |item, _, window, cx| {
                            item.close(window, cx);
                            item.servers.update(cx, LanguageServers::stop_all);
                        },
                    )),
                )
        });
        popover(cx)
            .id("lsp-popover")
            .debug_selector(|| "lsp-popover".into())
            .track_focus(&self.focus)
            .w(px(POPOVER_WIDTH))
            .max_h(max_height)
            .occlude()
            .text_size(px(UI_FONT_SIZE_SMALL))
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down_out(cx.listener(|item, ev: &MouseDownEvent, window, cx| {
                item.closed_at = Some(ev.position);
                item.close(window, cx);
            }))
            .child(
                div()
                    .id("lsp-popover-rows")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .gap(px(2. * GRID))
                    .p(px(3. * GRID))
                    .children(restricted_row)
                    .children(rows),
            )
            .children(footer)
    }
}

/// The dot of a server's state: running is `info` (themes have no green
/// "success" color), starting or not found `warning`, failed `error`,
/// stopped `icon.muted`.
fn state_color(state: &SlotState, ui: &lucerna_theme::UiColors) -> Rgba {
    match state {
        SlotState::Running => ui.info,
        SlotState::Starting | SlotState::NotFound { .. } => ui.warning,
        SlotState::Failed { .. } => ui.error,
        SlotState::Stopped => ui.icon_muted,
    }
}

fn dot(rgba: Rgba) -> gpui::Div {
    div()
        .flex_none()
        .size(px(DOT_SIZE))
        .rounded_full()
        .bg(color(rgba))
}

impl Render for LspStatusItem {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let servers = self.servers.read(cx);
        let visible = !servers.statuses().is_empty() || servers.shows_restricted_mode();
        let progress = servers.progress_text();
        if !visible {
            self.close(window, cx);
            return div().into_any_element();
        }
        let ui = cx.global::<AppState>().theme.ui.clone();
        let badge = self.dot(cx).map(|d| {
            let rgba = match d {
                StatusDot::Warning => ui.warning,
                StatusDot::Error => ui.error,
            };
            dot(rgba)
                .debug_selector(|| "lsp-button-dot".into())
                .absolute()
                .top(px(GRID / 2.))
                .right(px(GRID / 2.))
        });
        let button = icon_button("lsp-button", icons::ZAP, self.open, cx)
            .debug_selector(|| "lsp-button".into())
            .relative()
            .tooltip(tooltip("Language Servers"))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|item, ev, _, _| item.button_pressed(ev)),
            )
            .on_click(cx.listener(|item, _, window, cx| item.button_clicked(window, cx)))
            .children(badge);
        let max_height = popover_max_height(window.viewport_size().height);
        let popover = self.open.then(|| {
            // Above the button, right edges aligned.
            div().absolute().top_0().right_0().child(
                deferred(
                    anchored()
                        .anchor(Anchor::BottomRight)
                        .offset(point(px(0.), px(-GRID)))
                        .snap_to_window_with_margin(px(2. * GRID))
                        .child(self.render_popover(max_height, cx)),
                )
                .with_priority(1),
            )
        });
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(GRID))
            .children(progress.map(|text| {
                div()
                    .debug_selector(|| "lsp-progress".into())
                    .whitespace_nowrap()
                    .text_color(color(ui.text_muted))
                    .child(text)
            }))
            .child(div().relative().child(button).children(popover))
            .into_any_element()
    }
}

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register(
        "lucerna.lsp.restart",
        "Language Servers: Restart",
        |wb, _, _, cx| {
            let keys = wb.active_file_servers(cx);
            wb.language_servers().clone().update(cx, |servers, cx| {
                for key in &keys {
                    servers.restart(key, cx);
                }
            });
        },
    );
    commands.register(
        "lucerna.lsp.restartAll",
        "Language Servers: Restart All",
        |wb, _, _, cx| {
            wb.language_servers()
                .clone()
                .update(cx, LanguageServers::restart_all);
        },
    );
    commands.register(
        "lucerna.lsp.stopAll",
        "Language Servers: Stop All",
        |wb, _, _, cx| {
            wb.language_servers()
                .clone()
                .update(cx, LanguageServers::stop_all);
        },
    );
    commands.register(
        "lucerna.lsp.showLog",
        "Language Servers: Show Log",
        |wb, args, window, cx| {
            let named = args
                .and_then(|a| a.get("server"))
                .and_then(Value::as_str)
                .map(Arc::<str>::from);
            let server = named.or_else(|| {
                let first = wb.active_file_servers(cx).into_iter().next().or_else(|| {
                    let statuses = wb.language_servers().read(cx).statuses();
                    statuses.into_iter().next().map(|s| s.key)
                });
                first.map(|key| Arc::from(key.server))
            });
            if let Some(server) = server {
                wb.show_lsp_log(server, window, cx);
            }
        },
    );
}

impl Workbench {
    /// The ⚡ status bar item.
    pub fn lsp_status(&self) -> &Entity<LspStatusItem> {
        &self.lsp_status
    }

    /// The servers of the active tab's file.
    fn active_file_servers(&self, cx: &App) -> Vec<ServerKey> {
        let model = self.editor_groups().read(cx).model();
        match model.active_tab().and_then(|t| t.path()) {
            Some(path) => self.language_servers().read(cx).servers_for(path),
            None => Vec::new(),
        }
    }
}

/// The popover's height limit: below the title bar, above the status bar,
/// with the margins; never negative in a tiny window.
fn popover_max_height(viewport_height: Pixels) -> Pixels {
    (viewport_height - px(TITLE_BAR_HEIGHT + STATUS_BAR_HEIGHT + 4. * GRID)).max(px(0.))
}

#[cfg(test)]
mod popover_tests {
    use super::*;

    #[test]
    fn popover_height_is_never_negative() {
        assert_eq!(popover_max_height(px(10.)), px(0.));
        let tall = popover_max_height(px(800.));
        assert_eq!(
            tall,
            px(800. - (TITLE_BAR_HEIGHT + STATUS_BAR_HEIGHT + 4. * GRID))
        );
    }
}
