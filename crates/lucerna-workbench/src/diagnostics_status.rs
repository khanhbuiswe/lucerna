//! The diagnostics status bar item (spec §7.1): `⊗ 3  ⚠ 12` (a dim check
//! when there are none), then the first line of the message under the
//! cursor; and the F8 marker commands.

use std::cmp::Reverse;
use std::path::Path;
use std::sync::Arc;

use gpui::{
    AnyElement, Context, Entity, IntoElement, Render, Subscription, WeakEntity, Window, div,
    prelude::*, px, svg,
};
use lucerna_lsp::diagnostics::{Diagnostic, Scope};
use lucerna_text::Point;

use crate::code_view::CodeView;
use crate::editor_groups::EditorGroupsView;
use crate::icons;
use crate::language_servers::{LanguageServers, LanguageServersEvent};
use crate::layout::color;
use crate::ui::GRID;
use crate::{AppState, CommandRegistry, OpenMode, Workbench};

const ICON_SIZE: f32 = 12.;
/// The message under the cursor is cut at this many characters.
const MESSAGE_CHARS: usize = 80;

const NEXT_IN_FILES: &str = "editor.action.marker.nextInFiles";
const NEXT: &str = "editor.action.marker.next";

/// The first line of `message`, cut at 80 characters.
fn first_line(message: &str) -> String {
    let line = message.lines().next().unwrap_or_default();
    match line.char_indices().nth(MESSAGE_CHARS) {
        Some((end, _)) => format!("{}…", &line[..end]),
        None => line.to_owned(),
    }
}

/// The diagnostic at `cursor`: its range contains the cursor (the end
/// included, like the cursor after a word). The most severe wins, then the
/// narrowest.
fn at_cursor(diagnostics: &[Diagnostic], cursor: Point) -> Option<&Diagnostic> {
    let key = |p: Point| (p.row, p.column);
    diagnostics
        .iter()
        .filter(|d| key(d.range.start) <= key(cursor) && key(cursor) <= key(d.range.end))
        .min_by_key(|d| (d.severity, Reverse(key(d.range.start)), key(d.range.end)))
}

/// The status bar item: counts and the message under the cursor.
pub struct DiagnosticsStatusItem {
    workbench: WeakEntity<Workbench>,
    servers: Entity<LanguageServers>,
    groups: Entity<EditorGroupsView>,
    _subscriptions: [Subscription; 2],
}

impl DiagnosticsStatusItem {
    pub(crate) fn new(
        workbench: WeakEntity<Workbench>,
        servers: Entity<LanguageServers>,
        groups: Entity<EditorGroupsView>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = [
            // One event per 50 ms batch of diagnostics, or a slot change.
            cx.subscribe(&servers, |_, _, ev, cx| match ev {
                LanguageServersEvent::DiagnosticsChanged(_)
                | LanguageServersEvent::StatusChanged => cx.notify(),
                _ => {}
            }),
            cx.observe(&groups, |_, _, cx| cx.notify()),
        ];
        DiagnosticsStatusItem {
            workbench,
            servers,
            groups,
            _subscriptions: subscriptions,
        }
    }

    /// `(errors, warnings)` over every file; `None` while hidden, which is
    /// until a server was selected for some file.
    pub fn counts(&self, cx: &gpui::App) -> Option<(usize, usize)> {
        let servers = self.servers.read(cx);
        (!servers.statuses().is_empty()).then(|| servers.diagnostics().summary())
    }

    /// The first line of the message of the diagnostic under the active
    /// editor's cursor, cut at 80 characters.
    pub fn message(&self, cx: &gpui::App) -> Option<String> {
        self.counts(cx)?;
        let view = self
            .groups
            .read(cx)
            .active_body()?
            .downcast::<CodeView>()
            .ok()?;
        let view = view.read(cx);
        let diagnostic = at_cursor(view.diagnostics(), view.cursor())?;
        Some(first_line(&diagnostic.message))
    }

    fn run(&self, command: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        self.workbench
            .update(cx, |wb, cx| wb.run_command(command, None, window, cx))
            .ok();
    }
}

fn count(icon: &'static [u8], tint: gpui::Rgba, n: usize) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(GRID))
        .child(svg().data(icon).size(px(ICON_SIZE)).text_color(tint))
        .child(n.to_string())
}

impl Render for DiagnosticsStatusItem {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let counts = self.counts(cx);
        let message = self.message(cx);
        let ui = cx.global::<AppState>().theme.ui.clone();
        let counts_row: Option<AnyElement> = counts.map(|(errors, warnings)| {
            let body = if (errors, warnings) == (0, 0) {
                div()
                    .debug_selector(|| "diagnostics-check".into())
                    .opacity(0.6)
                    .child(
                        svg()
                            .data(icons::CHECK)
                            .size(px(ICON_SIZE))
                            .text_color(color(ui.text_muted)),
                    )
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .flex_row()
                    .gap(px(2. * GRID))
                    .child(count(icons::CIRCLE_X, color(ui.error), errors))
                    .child(count(icons::TRIANGLE_ALERT, color(ui.warning), warnings))
                    .into_any_element()
            };
            div()
                .id("diagnostics-counts")
                .debug_selector(|| "diagnostics-counts".into())
                .cursor_pointer()
                .child(body)
                .on_click(cx.listener(|item, _, window, cx| item.run(NEXT_IN_FILES, window, cx)))
                .into_any_element()
        });
        // Hidden, not just empty, so it adds no gap to the status bar.
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(2. * GRID))
            .when(counts_row.is_none(), |d| d.hidden())
            .children(counts_row)
            .children(message.map(|text| {
                div()
                    .id("diagnostics-message")
                    .debug_selector(|| "diagnostics-message".into())
                    .cursor_pointer()
                    .whitespace_nowrap()
                    .opacity(0.7)
                    .child(text)
                    .on_click(cx.listener(|item, _, window, cx| item.run(NEXT, window, cx)))
            }))
    }
}

impl Workbench {
    /// The diagnostics status bar item.
    pub fn diagnostics_status(&self) -> &Entity<DiagnosticsStatusItem> {
        &self.diagnostics_status
    }

    /// Moves to the next (or previous) diagnostic that is not a hint, in
    /// the active file or, for [`Scope::Files`], across files in path
    /// order. Opens the file as a preview tab when needed, puts the cursor
    /// at the start of the range and records the place in the navigation
    /// history. Unopened files may need reading, so `Files` runs off the
    /// UI thread; the latest request wins.
    pub(crate) fn go_to_marker(&mut self, forward: bool, scope: Scope, cx: &mut Context<Self>) {
        let Some(view) = self.active_code_view(cx) else {
            return;
        };
        let (path, cursor) = {
            let view = view.read(cx);
            (self.absolute_path(view.path()), view.cursor())
        };
        let store = self.language_servers().read(cx).diagnostics();
        match scope {
            Scope::File => {
                self.marker_task = None;
                let hit = store.next((&path, cursor), forward, scope);
                self.show_marker(hit, cx);
            }
            Scope::Files => {
                let store = store.clone();
                let task = cx.background_spawn(async move {
                    crate::tasks::guarded("diagnostics-next", || {
                        store.next((&path, cursor), forward, scope)
                    })
                    .flatten()
                });
                self.marker_task = Some(cx.spawn(async move |wb, cx| {
                    let hit = task.await;
                    wb.update(cx, |wb, cx| {
                        wb.marker_task = None;
                        wb.show_marker(hit, cx);
                    })
                    .ok();
                }));
            }
        }
    }

    fn show_marker(
        &mut self,
        hit: Option<(std::path::PathBuf, Diagnostic)>,
        cx: &mut Context<Self>,
    ) {
        let Some((path, diagnostic)) = hit else {
            return;
        };
        // Task 15 opens the diagnostic hover here, without delay.
        self.jump_to(
            Arc::from(Path::new(&path)),
            diagnostic.range.start,
            OpenMode::Preview,
            cx,
        );
    }
}

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register(
        NEXT_IN_FILES,
        "Go to Next Problem (Error, Warning, Info)",
        |wb, _, _, cx| wb.go_to_marker(true, Scope::Files, cx),
    );
    commands.register(
        "editor.action.marker.prevInFiles",
        "Go to Previous Problem (Error, Warning, Info)",
        |wb, _, _, cx| wb.go_to_marker(false, Scope::Files, cx),
    );
    commands.register(
        NEXT,
        "Go to Next Problem in Current File (Error, Warning, Info)",
        |wb, _, _, cx| wb.go_to_marker(true, Scope::File, cx),
    );
    commands.register(
        "editor.action.marker.prev",
        "Go to Previous Problem in Current File (Error, Warning, Info)",
        |wb, _, _, cx| wb.go_to_marker(false, Scope::File, cx),
    );
}
