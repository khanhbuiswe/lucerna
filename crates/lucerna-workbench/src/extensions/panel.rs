//! The Extensions panel in the sidebar (spec §5.2): a search box with `@`
//! filters, the Installed group and the registry's Available extensions
//! in one virtualised list of 24 px rows. Clicking a row expands it in
//! place (description, repository, commit, warnings); a running install
//! shows its stage with Cancel, a failure its error with "Show Log".
//! Also the extension commands (spec §5.3).

use std::collections::HashSet;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use gpui::{
    AnyElement, App, Context, Div, Entity, IntoElement, MouseButton, ParentElement, Render,
    SharedString, Stateful, Styled, Subscription, UniformListScrollHandle, WeakEntity, Window, div,
    prelude::*, px, svg, uniform_list,
};

use super::ExtensionService;
use super::panel_model::{PanelRows, Row, RowState, rows, stage_label};
use crate::layout::color;
use crate::text_input::TextInput;
use crate::ui::{
    CHEVRON_DOWN_ICON, CHEVRON_RIGHT_ICON, GRID, TREE_ROW_HEIGHT, UI_FONT_SIZE_CAPTION,
    UI_FONT_SIZE_SMALL,
};
use crate::{AppState, CommandRegistry, OpenMode, SidebarView, Workbench};

const LOADING: &str = "Loading extension list…";
const GIT_MISSING: &str =
    "git was not found. Install the Xcode Command Line Tools: xcode-select --install";
const FETCH_FAILED: &str = "Could not fetch the extension list.";
const LSP_NOTE: &str = "Language server: Phase 3";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    Installed,
    Available,
}

/// A line under a row, as tall as the row.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Line {
    /// What an installed extension holds.
    Summary(String),
    LspNote,
    /// Why an installed extension is unusable, with Reinstall.
    Broken(String),
    /// The running install's stage, with Cancel.
    Progress(String),
    /// The last failure, with "Show Log".
    Error(String),
    Detail(String),
    Warning(String),
}

/// One 24 px item of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Item {
    Header(Group),
    Row(Group, usize),
    Line(Group, usize, Line),
}

pub struct ExtensionsPanel {
    workbench: WeakEntity<Workbench>,
    query: Entity<TextInput>,
    rows: PanelRows,
    items: Vec<Item>,
    /// Ids of the expanded rows.
    expanded: HashSet<String>,
    installed_collapsed: bool,
    /// Ids whose name and description were asked for.
    described: HashSet<String>,
    /// The panel was shown before `registry.json` was read: fetch the
    /// registry if it turns out to be missing.
    fetch_when_loaded: bool,
    scroll: UniformListScrollHandle,
    _service: Option<Subscription>,
}

impl ExtensionsPanel {
    pub(crate) fn new(workbench: WeakEntity<Workbench>, cx: &mut Context<Self>) -> Self {
        let this = cx.weak_entity();
        let query = cx.new(|cx| {
            let mut input = TextInput::new(cx);
            input.set_placeholder("Search extensions (@installed, @updates, @themes…)");
            input.on_change(move |_, _, cx| {
                this.update(cx, |panel, cx| panel.refresh(cx)).ok();
            });
            input
        });
        let service = cx
            .has_global::<ExtensionService>()
            .then(|| cx.observe_global::<ExtensionService>(Self::service_changed));
        let mut panel = ExtensionsPanel {
            workbench,
            query,
            rows: PanelRows::default(),
            items: Vec::new(),
            expanded: HashSet::new(),
            installed_collapsed: false,
            described: HashSet::new(),
            fetch_when_loaded: false,
            scroll: UniformListScrollHandle::new(),
            _service: service,
        };
        panel.refresh(cx);
        panel
    }

    /// The rows shown for the current query, with what runs on them.
    pub fn rows(&self) -> &PanelRows {
        &self.rows
    }

    pub fn query_input(&self) -> &Entity<TextInput> {
        &self.query
    }

    /// The panel became visible: fetches the registry when there is none
    /// and no fetch ran or failed yet.
    pub(crate) fn shown(&mut self, cx: &mut Context<Self>) {
        let Some(service) = cx.try_global::<ExtensionService>() else {
            return;
        };
        if !service.registry_loaded() {
            self.fetch_when_loaded = true;
        } else if service.registry().is_none()
            && !service.is_refreshing()
            && service.registry_error().is_none()
        {
            ExtensionService::refresh_registry(cx);
        }
    }

    fn service_changed(&mut self, cx: &mut Context<Self>) {
        if self.fetch_when_loaded
            && cx
                .try_global::<ExtensionService>()
                .is_some_and(ExtensionService::registry_loaded)
        {
            self.fetch_when_loaded = false;
            self.shown(cx);
        }
        self.refresh(cx);
    }

    /// Recomputes the rows from the service and the query.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(service) = cx.try_global::<ExtensionService>() else {
            return;
        };
        let query = self.query.read(cx).text().to_string();
        let mut rows = rows(
            service.registry().map(|r| r.as_ref()),
            service.index(),
            service.updates(),
            &query,
        );
        rows.overlay(|row| {
            if service.is_installing(&row.id) {
                let stage = service
                    .progress(&row.id)
                    .map(|p| p.stage.clone())
                    .unwrap_or(lucerna_extension::builder::Stage::Fetching);
                Some(RowState::Installing(stage))
            } else {
                service
                    .last_error(&row.id)
                    .map(|e| RowState::Failed(e.to_string()))
            }
        });
        self.rows = rows;
        self.rebuild_items(cx);
        cx.notify();
    }

    fn rebuild_items(&mut self, cx: &App) {
        let service = cx.try_global::<ExtensionService>();
        let mut items = Vec::new();
        let groups = [
            (Group::Installed, &self.rows.installed),
            (Group::Available, &self.rows.available),
        ];
        for (group, rows) in groups {
            if rows.is_empty() {
                continue;
            }
            items.push(Item::Header(group));
            if group == Group::Installed && self.installed_collapsed {
                continue;
            }
            for (index, row) in rows.iter().enumerate() {
                items.push(Item::Row(group, index));
                let expanded = self.expanded.contains(&row.id);
                let warnings = service.map_or(&[][..], |s| s.warnings(&row.id));
                for line in lines(group, row, expanded, warnings) {
                    items.push(Item::Line(group, index, line));
                }
            }
        }
        self.items = items;
    }

    fn row(&self, group: Group, index: usize) -> Option<&Row> {
        match group {
            Group::Installed => self.rows.installed.get(index),
            Group::Available => self.rows.available.get(index),
        }
    }

    /// Expands or folds a row. Expanding an uninstalled one whose
    /// description is unknown asks the service to describe it.
    fn toggle_expanded(&mut self, group: Group, index: usize, cx: &mut Context<Self>) {
        let Some(row) = self.row(group, index) else {
            return;
        };
        let id = row.id.clone();
        let unknown = row.description.is_none() && !matches!(row.state, RowState::Installing(_));
        if self.expanded.remove(&id) {
            self.rebuild_items(cx);
            cx.notify();
            return;
        }
        let describe = group == Group::Available && unknown && self.described.insert(id.clone());
        self.expanded.insert(id.clone());
        if describe && cx.has_global::<ExtensionService>() {
            ExtensionService::describe(id, cx);
        }
        self.rebuild_items(cx);
        cx.notify();
    }

    fn toggle_installed_group(&mut self, cx: &mut Context<Self>) {
        self.installed_collapsed = !self.installed_collapsed;
        self.rebuild_items(cx);
        cx.notify();
    }

    fn install(&mut self, id: String, cx: &mut Context<Self>) {
        self.expanded.insert(id.clone());
        ExtensionService::install(id, cx);
    }

    fn show_log(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(service) = cx.try_global::<ExtensionService>() else {
            return;
        };
        let path: Arc<Path> = service
            .store()
            .root()
            .join("logs")
            .join(format!("{id}.log"))
            .into();
        if let Some(workbench) = self.workbench.upgrade() {
            workbench.update(cx, |wb, cx| wb.open_path(path, OpenMode::Preview, cx));
        }
    }

    fn render_items(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        range
            .filter_map(|i| {
                let item = self.items.get(i)?.clone();
                Some(match item {
                    Item::Header(group) => self.render_header(group, cx),
                    Item::Row(group, index) => self.render_row(group, index, cx)?,
                    Item::Line(group, index, line) => self.render_line(group, index, line, cx)?,
                })
            })
            .collect()
    }

    fn render_header(&self, group: Group, cx: &mut Context<Self>) -> AnyElement {
        let ui = &cx.global::<AppState>().theme.ui;
        let (label, count) = match group {
            Group::Installed => ("Installed", self.rows.installed.len()),
            Group::Available => ("Available", self.rows.available.len()),
        };
        let chevron = match group {
            Group::Installed if self.installed_collapsed => Some(CHEVRON_RIGHT_ICON),
            Group::Installed => Some(CHEVRON_DOWN_ICON),
            Group::Available => None,
        };
        let id = match group {
            Group::Installed => "extensions-installed-header",
            Group::Available => "extensions-available-header",
        };
        item_row(id, id.to_string(), cx)
            .pl(px(2. * GRID))
            .text_size(px(UI_FONT_SIZE_CAPTION))
            .text_color(color(ui.text_muted))
            .when(group == Group::Installed, |row| {
                row.cursor_pointer()
                    .on_click(cx.listener(|panel, _, _, cx| panel.toggle_installed_group(cx)))
            })
            .child(match chevron {
                Some(icon) => svg()
                    .data(icon)
                    .flex_none()
                    .size(px(12.))
                    .text_color(color(ui.icon_muted))
                    .into_any_element(),
                None => div().flex_none().w(px(12.)).into_any_element(),
            })
            .child(label)
            .child(count.to_string())
            .into_any_element()
    }

    fn render_row(&self, group: Group, index: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let row = self.row(group, index)?;
        let icon_muted = color(cx.global::<AppState>().theme.ui.icon_muted);
        let text_muted = color(cx.global::<AppState>().theme.ui.text_muted);
        let id = row.id.clone();
        let has_update = cx
            .try_global::<ExtensionService>()
            .is_some_and(|s| s.updates().contains(&id));
        let running = matches!(row.state, RowState::Installing(_));
        let mut buttons: Vec<AnyElement> = Vec::new();
        let button = |kind: &'static str, label: &'static str, cx: &mut Context<Self>| {
            text_button(format!("extension-{kind}-{id}"), label, cx)
        };
        match group {
            Group::Available if !running => {
                let install_id = id.clone();
                buttons.push(
                    button("install", "Install", cx)
                        .on_click(cx.listener(move |panel, _, _, cx| {
                            cx.stop_propagation();
                            panel.install(install_id.clone(), cx);
                        }))
                        .into_any_element(),
                );
            }
            Group::Installed if !running => {
                if has_update {
                    let update_id = id.clone();
                    buttons.push(
                        button("update", "Update", cx)
                            .on_click(cx.listener(move |panel, _, _, cx| {
                                cx.stop_propagation();
                                panel.install(update_id.clone(), cx);
                            }))
                            .into_any_element(),
                    );
                }
                let uninstall_id = id.clone();
                buttons.push(
                    button("uninstall", "Uninstall", cx)
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.stop_propagation();
                            ExtensionService::uninstall(uninstall_id.clone(), cx);
                        }))
                        .into_any_element(),
                );
            }
            _ => {}
        }
        let chevron = if self.expanded.contains(&id) {
            CHEVRON_DOWN_ICON
        } else {
            CHEVRON_RIGHT_ICON
        };
        let element = item_row(
            SharedString::from(format!("extension-row-{id}")),
            format!("extension-row-{id}"),
            cx,
        )
        .pl(px(2. * GRID))
        .cursor_pointer()
        .on_click(cx.listener(move |panel, _, _, cx| panel.toggle_expanded(group, index, cx)))
        .child(
            svg()
                .data(chevron)
                .flex_none()
                .size(px(12.))
                .text_color(icon_muted),
        )
        .child(
            div()
                .flex_none()
                .max_w(px(40. * GRID))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                // Available rows show the registry id (spec §5.2); the
                // name, once known, is in the expanded lines.
                .child(match group {
                    Group::Installed => row.title().to_string(),
                    Group::Available => row.id.clone(),
                }),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(UI_FONT_SIZE_CAPTION))
                .text_color(text_muted)
                .child(row.version.clone()),
        )
        .children(buttons)
        .into_any_element();
        Some(element)
    }

    fn render_line(
        &self,
        group: Group,
        index: usize,
        line: Line,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let id = self.row(group, index)?.id.clone();
        let ui = &cx.global::<AppState>().theme.ui;
        let (muted, error) = (color(ui.text_muted), color(ui.error));
        let base = item_row(
            SharedString::from(format!("extension-line-{id}-{}", line_kind(&line))),
            format!("extension-{}-{id}", line_kind(&line)),
            cx,
        )
        .pl(px(7. * GRID))
        .text_size(px(UI_FONT_SIZE_CAPTION))
        .text_color(muted);
        let element = match line {
            Line::Summary(text) | Line::Detail(text) => base.child(text_cell(text)),
            Line::LspNote => base.child(text_cell(LSP_NOTE.into())),
            Line::Warning(text) => base.child(text_cell(format!("⚠ {text}"))),
            Line::Broken(reason) => {
                let reinstall_id = id.clone();
                base.child(text_cell(reason).text_color(error)).child(
                    text_button(format!("extension-reinstall-{id}"), "Reinstall", cx).on_click(
                        cx.listener(move |panel, _, _, cx| {
                            cx.stop_propagation();
                            panel.install(reinstall_id.clone(), cx);
                        }),
                    ),
                )
            }
            Line::Progress(stage) => {
                let cancel_id = id.clone();
                base.child(text_cell(stage)).child(
                    text_button(format!("extension-cancel-{id}"), "Cancel", cx).on_click(
                        cx.listener(move |_, _, _, cx| {
                            cx.stop_propagation();
                            ExtensionService::cancel(cancel_id.clone(), cx);
                        }),
                    ),
                )
            }
            Line::Error(message) => {
                let log_id = id.clone();
                base.child(text_cell(message).text_color(error)).child(
                    text_button(format!("extension-show-log-{id}"), "Show Log", cx).on_click(
                        cx.listener(move |panel, _, _, cx| {
                            cx.stop_propagation();
                            panel.show_log(&log_id, cx);
                        }),
                    ),
                )
            }
        };
        Some(element.into_any_element())
    }

    /// The message above the list: the registry is loading, git is
    /// missing, or the fetch failed (with Retry).
    fn render_message(&self, cx: &mut Context<Self>) -> Option<Div> {
        let service = cx.try_global::<ExtensionService>()?;
        let ui = &cx.global::<AppState>().theme.ui;
        let message = |selector: &'static str, text: &'static str| {
            div()
                .debug_selector(move || selector.into())
                .flex()
                .items_center()
                .gap(px(2. * GRID))
                .px(px(3. * GRID))
                .py(px(GRID))
                .text_size(px(UI_FONT_SIZE_SMALL))
                .text_color(color(ui.text_muted))
                .child(div().flex_1().min_w_0().child(text))
        };
        if service.is_refreshing() && service.registry().is_none() {
            Some(message("extensions-loading", LOADING))
        } else if service.git_missing() {
            Some(message("extensions-git-missing", GIT_MISSING))
        } else if service.registry_error().is_some() {
            Some(
                message("extensions-registry-error", FETCH_FAILED).child(
                    text_button("extensions-retry".to_string(), "Retry", cx)
                        .on_click(|_, _, cx| ExtensionService::refresh_registry(cx)),
                ),
            )
        } else if service.registry().is_none() {
            Some(message("extensions-loading", LOADING))
        } else {
            None
        }
    }
}

/// The lines shown under a row: always the installed summary, language
/// server note and broken reason, a running install's stage and the last
/// failure; when expanded, the description, repository, commit and the
/// last install's warnings.
fn lines(group: Group, row: &Row, expanded: bool, warnings: &[String]) -> Vec<Line> {
    let mut lines = Vec::new();
    if group == Group::Installed && !row.summary.is_empty() {
        lines.push(Line::Summary(row.summary.clone()));
    }
    if row.lsp_note {
        lines.push(Line::LspNote);
    }
    match &row.state {
        RowState::Broken(reason) => lines.push(Line::Broken(reason.clone())),
        RowState::Installing(stage) => lines.push(Line::Progress(stage_label(stage))),
        RowState::Failed(error) => lines.push(Line::Error(error.clone())),
        _ => {}
    }
    if expanded {
        if group == Group::Available
            && let Some(name) = &row.name
        {
            lines.push(Line::Detail(name.clone()));
        }
        lines.push(Line::Detail(
            row.description
                .clone()
                .unwrap_or_else(|| "No description".into()),
        ));
        if let Some(repository) = &row.repository {
            lines.push(Line::Detail(repository.clone()));
        }
        if let Some(commit) = &row.commit {
            lines.push(Line::Detail(commit.chars().take(7).collect()));
        }
        lines.extend(warnings.iter().cloned().map(Line::Warning));
    }
    lines
}

fn line_kind(line: &Line) -> &'static str {
    match line {
        Line::Summary(_) => "summary",
        Line::LspNote => "lsp",
        Line::Broken(_) => "broken",
        Line::Progress(_) => "progress",
        Line::Error(_) => "error",
        Line::Detail(_) => "detail",
        Line::Warning(_) => "warning",
    }
}

/// A 24 px list item with hover; add `on_click` and the content.
fn item_row(id: impl Into<gpui::ElementId>, selector: String, cx: &App) -> Stateful<Div> {
    let hover = color(cx.global::<AppState>().theme.ui.ghost_element_hover);
    div()
        .id(id)
        .debug_selector(move || selector)
        .flex()
        .items_center()
        .gap(px(1.5 * GRID))
        .h(px(TREE_ROW_HEIGHT))
        .w_full()
        .pr(px(2. * GRID))
        .hover(move |s| s.bg(hover))
}

/// One line of text that ends in an ellipsis when too long.
fn text_cell(text: String) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .child(text)
}

/// A small bordered text button, 20 px tall; its id and debug selector are
/// `id`. Clicking it leaves focus where it was.
fn text_button(id: String, label: &'static str, cx: &App) -> Stateful<Div> {
    let ui = &cx.global::<AppState>().theme.ui;
    let hover = color(ui.ghost_element_hover);
    let selector = id.clone();
    div()
        .id(SharedString::from(id))
        .debug_selector(move || selector)
        .flex()
        .flex_none()
        .items_center()
        .h(px(5. * GRID))
        .px(px(1.5 * GRID))
        .rounded(px(GRID))
        .border_1()
        .border_color(color(ui.border_variant))
        .text_size(px(UI_FONT_SIZE_CAPTION))
        .text_color(color(ui.text))
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .hover(move |s| s.bg(hover))
        .child(label)
}

impl Render for ExtensionsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = &cx.global::<AppState>().theme.ui;
        let input = div()
            .flex()
            .items_center()
            .h(px(7. * GRID))
            .px(px(2. * GRID))
            .rounded(px(GRID))
            .border_1()
            .border_color(color(ui.border_variant))
            .bg(color(ui.editor_background))
            .child(div().flex_1().min_w_0().child(self.query.clone()));
        let message = self.render_message(cx);
        div()
            .debug_selector(|| "extensions-panel".into())
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex_none()
                    .px(px(2. * GRID))
                    .pb(px(2. * GRID))
                    .child(input),
            )
            .children(message)
            .child(
                div().flex_1().min_h_0().child(
                    uniform_list(
                        "extensions-list",
                        self.items.len(),
                        cx.processor(|panel, range, _, cx| panel.render_items(range, cx)),
                    )
                    .size_full()
                    .track_scroll(&self.scroll),
                ),
            )
    }
}

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register(
        "workbench.view.extensions",
        "View: Show Extensions",
        |wb, _, window, cx| wb.show_sidebar_view(SidebarView::Extensions, window, cx),
    );
    commands.register(
        "workbench.extensions.action.installExtension",
        "Extensions: Install Extension…",
        |wb, _, window, cx| {
            let Some(service) = cx.try_global::<ExtensionService>() else {
                return;
            };
            let installed = &service.index().extensions;
            let ids: Vec<String> = service
                .registry()
                .map(|r| {
                    r.entries
                        .iter()
                        .filter(|e| !installed.contains_key(&e.id))
                        .map(|e| e.id.clone())
                        .collect()
                })
                .unwrap_or_default();
            let fetch = service.registry().is_none() && !service.is_refreshing();
            if fetch {
                ExtensionService::refresh_registry(cx);
            }
            wb.show_extension_ids(ids, window, cx);
        },
    );
    commands.register(
        "workbench.extensions.action.checkForUpdates",
        "Extensions: Check for Updates",
        |_, _, _, cx| {
            if cx.has_global::<ExtensionService>() {
                ExtensionService::refresh_registry(cx);
            }
        },
    );
    commands.register(
        "workbench.extensions.action.updateAllExtensions",
        "Extensions: Update All",
        |_, _, _, cx| {
            if cx.has_global::<ExtensionService>() {
                ExtensionService::update_all(cx);
            }
        },
    );
    commands.register(
        "lucerna.extensions.clearBuildCache",
        "Extensions: Clear Build Cache",
        |_, _, _, cx| {
            if cx.has_global::<ExtensionService>() {
                ExtensionService::clear_build_cache(cx);
            }
        },
    );
}
