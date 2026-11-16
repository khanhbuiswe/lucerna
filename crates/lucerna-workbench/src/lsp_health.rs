//! The language server Health tab (`lucerna.lsp.health`, spec §7): every
//! (language, server) pair of the server table and whether it is
//! installed, grouped by language under file-header-like cards, with the
//! install command and Copy for a server not found, and the `lsp`
//! setting's invalid entries under the toolbar. One tab, never saved.
//!
//! The check (`which`, the probe) runs in the background; its rows are
//! kept until refresh or until the table changes.

use std::collections::BTreeSet;
use std::path::PathBuf;

use gpui::{
    App, Context, Entity, FocusHandle, Focusable, IntoElement, MouseButton, Render, SharedString,
    Subscription, Task, Window, div, prelude::*, px,
};
use lucerna_lsp::health::{HealthRow, HealthStatus, current_os, health, health_in, status_text};

use crate::language_servers::{LanguageServers, LanguageServersEvent};
use crate::layout::color;
use crate::tasks::guarded;
use crate::ui::{
    GRID, UI_FONT_SIZE_CAPTION, UI_FONT_SIZE_SMALL, bar_icon_button, copy_command_row,
    search_bar_row, search_toolbar, subheader_card, tooltip, warning_row,
};
use crate::{AppState, Workbench, icons};

/// The body of the Health tab.
pub struct LspHealthView {
    servers: Entity<LanguageServers>,
    /// Where probes run once the folder is trusted: the open folder's
    /// root; `None` (`$HOME`) in single-file mode.
    probe_dir: Option<PathBuf>,
    focus: FocusHandle,
    /// The last check's rows; `None` before the first one finished.
    rows: Option<Vec<HealthRow>>,
    /// Bumped per check, so an older check's rows are dropped.
    generation: u64,
    /// The check in flight.
    task: Option<Task<()>>,
    _subscription: Subscription,
}

impl LspHealthView {
    pub fn new(
        servers: Entity<LanguageServers>,
        probe_dir: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) -> Self {
        // A new table (settings changed) or a trust change is checked
        // again once the tab has been shown; the table's errors are drawn
        // as they change.
        let subscription = cx.subscribe(&servers, |view, _, ev, cx| match ev {
            LanguageServersEvent::TableChanged | LanguageServersEvent::TrustChanged
                if view.rows.is_some() || view.task.is_some() =>
            {
                view.refresh(cx);
            }
            LanguageServersEvent::StatusChanged => cx.notify(),
            _ => {}
        });
        LspHealthView {
            servers,
            probe_dir,
            focus: cx.focus_handle(),
            rows: None,
            generation: 0,
            task: None,
            _subscription: subscription,
        }
    }

    /// The tab is shown: checks the servers the first time.
    pub fn shown(&mut self, cx: &mut Context<Self>) {
        if self.rows.is_none() && self.task.is_none() {
            self.refresh(cx);
        }
    }

    /// Checks every server of the table again, in the background. The
    /// table is the one servers start from: without the folder's own
    /// settings while it is untrusted. Probes run in the folder only once
    /// it is trusted: a rustup, asdf or mise shim reads the folder's
    /// toolchain files, so an untrusted folder would pick what runs.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let servers = self.servers.read(cx);
        let (table, launcher) = (servers.table().clone(), servers.launcher());
        let probe_dir = self.probe_dir.clone().filter(|_| servers.is_trusted());
        self.generation += 1;
        let generation = self.generation;
        let work = cx.background_spawn(async move {
            guarded("lsp-health", move || match probe_dir {
                Some(dir) => health_in(&table, &*launcher, current_os(), &dir),
                None => health(&table, &*launcher, current_os()),
            })
            .unwrap_or_default()
        });
        self.task = Some(cx.spawn(async move |view, cx| {
            let rows = work.await;
            view.update(cx, |view, cx| {
                if view.generation == generation {
                    view.rows = Some(rows);
                    view.task = None;
                    cx.notify();
                }
            })
            .ok();
        }));
        cx.notify();
    }

    /// The last check's rows; `None` before the first one finished.
    pub fn rows(&self) -> Option<&[HealthRow]> {
        self.rows.as_deref()
    }

    /// The `lsp` setting's invalid entries, one line each, as in the ⚡
    /// popover.
    pub fn table_error_lines(&self, cx: &App) -> Vec<String> {
        self.servers.read(cx).table_error_lines()
    }

    /// Whether a check is running.
    pub fn checking(&self) -> bool {
        self.task.is_some()
    }

    /// "{found} of {total} servers found", counting each server once
    /// however many languages it has; `None` before the first check
    /// finished.
    pub fn summary(&self) -> Option<String> {
        let rows = self.rows.as_ref()?;
        let total: BTreeSet<&str> = rows.iter().map(|r| &*r.server).collect();
        let found: BTreeSet<&str> = rows
            .iter()
            .filter(|r| matches!(r.status, HealthStatus::Found(_)))
            .map(|r| &*r.server)
            .collect();
        Some(format!("{} of {} servers found", found.len(), total.len()))
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = &cx.global::<AppState>().theme.ui;
        let summary = self
            .summary()
            .unwrap_or_else(|| "Checking language servers…".to_owned());
        search_toolbar(cx).child(
            search_bar_row()
                .child(
                    div()
                        .debug_selector(|| "lsp-health-summary".into())
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(UI_FONT_SIZE_SMALL))
                        .text_color(color(ui.text_muted))
                        .child(summary),
                )
                .child(
                    bar_icon_button("lsp-health-refresh", icons::REFRESH_CW, false, cx)
                        .tooltip(tooltip("Refresh"))
                        .on_click(cx.listener(|view, _, _, cx| view.refresh(cx))),
                ),
        )
    }

    /// The language's card, as a file header of a results list.
    fn render_header(language: &str, cx: &App) -> impl IntoElement {
        let ui = &cx.global::<AppState>().theme.ui;
        subheader_card(cx)
            .flex_none()
            .h(px(7. * GRID))
            .mt(px(2. * GRID))
            .px(px(2. * GRID))
            .text_color(color(ui.text))
            .child(language.to_owned())
    }

    fn render_row(
        i: usize,
        row: &HealthRow,
        code_font: SharedString,
        cx: &App,
    ) -> impl IntoElement {
        let ui = &cx.global::<AppState>().theme.ui;
        let muted = color(ui.text_muted);
        let status_color = match row.status {
            HealthStatus::NotFound => color(ui.warning),
            HealthStatus::Found(_) | HealthStatus::Disabled => muted,
        };
        let found = matches!(row.status, HealthStatus::Found(_));
        let install = row.install.clone().filter(|_| !found).map(|command| {
            copy_command_row(
                command,
                format!("lsp-health-install-{i}"),
                format!("lsp-health-copy-{i}"),
                code_font,
                cx,
            )
            .flex_none()
            .max_w(px(100. * GRID))
        });
        let note = row.note.clone().map(|note| {
            div()
                .text_size(px(UI_FONT_SIZE_CAPTION))
                .text_color(muted)
                .child(note)
        });
        div()
            .debug_selector(move || format!("lsp-health-row-{i}"))
            .flex()
            .flex_col()
            .px(px(2. * GRID))
            .py(px(GRID))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(3. * GRID))
                    .min_h(px(6. * GRID))
                    .child(
                        div()
                            .flex_none()
                            .w(px(48. * GRID))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_color(color(ui.text))
                            .child(row.server.to_string()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_color(status_color)
                            .child(status_text(&row.status)),
                    )
                    .children(install),
            )
            .children(note)
    }
}

impl Focusable for LspHealthView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for LspHealthView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = &cx.global::<AppState>().theme.ui;
        let background = color(ui.editor_background);
        let code_font = {
            let settings = self.servers.read(cx).settings().read(cx);
            crate::fonts::editor_font_family(settings.view(cx), None)
        };
        // The `lsp` setting's invalid entries, above the languages.
        let errors = self.table_error_lines(cx);
        let errors =
            (!errors.is_empty()).then(|| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(GRID))
                    .pt(px(2. * GRID))
                    .children(errors.into_iter().enumerate().map(|(i, line)| {
                        warning_row(format!("lsp-health-table-error-{i}"), line, cx)
                    }))
            });
        let mut list = Vec::new();
        let mut language: Option<&str> = None;
        for (i, row) in self.rows.iter().flatten().enumerate() {
            if language != Some(row.language.as_str()) {
                language = Some(&row.language);
                list.push(Self::render_header(&row.language, cx).into_any_element());
            }
            list.push(Self::render_row(i, row, code_font.clone(), cx).into_any_element());
        }
        div()
            .id("lsp-health")
            .debug_selector(|| "lsp-health".into())
            .track_focus(&self.focus)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, window, cx| window.focus(&view.focus, cx)),
            )
            .flex()
            .flex_col()
            .size_full()
            .bg(background)
            .text_size(px(UI_FONT_SIZE_SMALL))
            .child(self.render_toolbar(cx))
            .child(
                div()
                    .id("lsp-health-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(3. * GRID))
                    .pb(px(3. * GRID))
                    .children(errors)
                    .children(list),
            )
    }
}

impl Workbench {
    /// Opens (or focuses) the Health tab: every server of the table and
    /// whether it is installed, checked the first time it is shown.
    pub fn show_lsp_health(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.update_editor_groups(cx, |m| {
            m.open_lsp_health();
        });
        self.lsp_health.update(cx, |view, cx| {
            view.shown(cx);
            window.focus(&view.focus, cx);
        });
        cx.notify();
    }

    /// The Health tab's body.
    pub fn lsp_health(&self) -> &Entity<LspHealthView> {
        &self.lsp_health
    }
}
