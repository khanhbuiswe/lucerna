//! The Markdown preview's commands (spec §4.2), which editor a file opens
//! in (`workbench.editorAssociations`, spec §4.1), the dynamic preview
//! following the active markdown file, a double-click on a block opening
//! its source and what a clicked link opens (spec §4.4, §4.5, §5.6).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use gpui::{App, Context, Entity, EntityId, WeakEntity, Window};
use lucerna_settings::SettingsView;
use lucerna_syntax::Associations;
use lucerna_text::Point;

use super::{LinkAction, MarkdownPreview};
use crate::code_view::CodeView;
use crate::editor_groups::{FileView, OpenMode, TabId, TabItem};
use crate::{CommandRegistry, WindowSettings, Workbench};

/// The editor id of the Markdown preview in `workbench.editorAssociations`.
pub const PREVIEW_EDITOR_ID: &str = "vscode.markdown.preview.editor";

/// The setting naming the editor each glob opens in.
pub(crate) const EDITOR_ASSOCIATIONS: &str = "workbench.editorAssociations";

/// The setting that lets a double-click on a block open the source.
const DOUBLE_CLICK_TO_SWITCH: &str = "markdown.preview.doubleClickToSwitchToEditor";

/// Where a link to another markdown file opens: `"inPreview"` or
/// `"inEditor"`.
const OPEN_MARKDOWN_LINKS: &str = "markdown.preview.openMarkdownLinks";

/// The language id of markdown files.
const MARKDOWN: &str = "markdown";

pub const TOGGLE_PREVIEW: &str = "markdown.togglePreview";
pub const SHOW_PREVIEW: &str = "markdown.showPreview";
pub const SHOW_PREVIEW_TO_SIDE: &str = "markdown.showPreviewToSide";
pub const SHOW_SOURCE: &str = "markdown.showSource";
pub const REFRESH_PREVIEW: &str = "markdown.preview.refresh";

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register(
        TOGGLE_PREVIEW,
        "Markdown: Toggle Preview",
        |wb, _, window, cx| match active(wb, cx) {
            Some(Active::Source { tab, path }) => preview_in_place(wb, tab, path, window, cx),
            Some(Active::Static { tab, path }) => source_in_place(wb, tab, path, window, cx),
            Some(Active::Dynamic { path, line, .. }) => source_here(wb, path, line, window, cx),
            None => {}
        },
    );
    commands.register(
        SHOW_PREVIEW,
        "Markdown: Open Preview",
        |wb, args, window, cx| {
            // From the Explorer's menu: the row's file, whatever is open.
            if let Some(rel) = args.and_then(|a| a.get("path")).and_then(|p| p.as_str()) {
                let path = wb.absolute_path(Path::new(rel));
                if is_markdown(wb.root(), &path, wb.settings(), cx) {
                    wb.open_markdown_preview(path, cx);
                    wb.focus_editor_area(window, cx);
                }
                return;
            }
            if let Some(Active::Source { tab, path }) = active(wb, cx) {
                preview_in_place(wb, tab, path, window, cx);
            }
        },
    );
    commands.register(
        SHOW_PREVIEW_TO_SIDE,
        "Markdown: Open Preview to the Side",
        |wb, _, window, cx| {
            let Some(active) = active(wb, cx) else {
                return;
            };
            let (path, line) = match active {
                Active::Source { tab, path } => {
                    let row = code_view_of(wb, tab, cx).map_or(0, |v| v.read(cx).scroll_top_row());
                    (path, row as f32)
                }
                Active::Static { tab, path } => {
                    let line = preview_of(wb, tab, cx).map_or(0., |p| p.read(cx).top_line());
                    (path, line)
                }
                Active::Dynamic { .. } => return,
            };
            let id = wb.update_editor_groups(cx, |m| m.open_dynamic_preview(path.clone()));
            show_dynamic(wb, id, path, Some(line), window, cx);
        },
    );
    commands.register(
        SHOW_SOURCE,
        "Markdown: Show Source",
        |wb, _, window, cx| match active(wb, cx) {
            Some(Active::Static { tab, path }) => source_in_place(wb, tab, path, window, cx),
            Some(Active::Dynamic { path, line, .. }) => source_here(wb, path, line, window, cx),
            _ => {}
        },
    );
    commands.register(
        REFRESH_PREVIEW,
        "Markdown: Refresh Preview",
        |wb, _, _, cx| {
            for preview in previews(wb, cx) {
                preview.update(cx, |p, cx| p.refresh(cx));
            }
        },
    );
}

/// The editor `workbench.editorAssociations` (`map`) opens a file in: the
/// first value of the longest glob matching `rel` (the path relative to the
/// root, or the file name for a glob without `/`), else of one matching the
/// absolute `path` (a glob such as `/notes/**/*.md`). The preview for
/// [`PREVIEW_EDITOR_ID`]; the source for `"default"`, any other id and no
/// match.
pub fn editor_for(path: &Path, rel: &Path, map: &BTreeMap<String, String>) -> FileView {
    if map.is_empty() {
        return FileView::Source;
    }
    let associations = Associations::new(map);
    let editor = associations
        .matches(rel)
        .first()
        .copied()
        .or_else(|| associations.matches(path).first().copied());
    match editor {
        Some(PREVIEW_EDITOR_ID) => FileView::Preview,
        _ => FileView::Source,
    }
}

/// The editor `path` (absolute) opens in, by the user's associations.
pub(crate) fn associated_editor(root: &Path, path: &Path, settings: SettingsView<'_>) -> FileView {
    let map = settings
        .get::<BTreeMap<String, String>>(EDITOR_ASSOCIATIONS, None)
        .unwrap_or_default();
    let rel = path.strip_prefix(root).unwrap_or(path);
    editor_for(path, rel, &map)
}

/// Whether `path` (absolute) is a markdown file, by its language
/// (`files.associations` included). `settings` are the window's: its
/// folder may add associations.
pub(crate) fn is_markdown(
    root: &Path,
    path: &Path,
    settings: &Entity<WindowSettings>,
    cx: &App,
) -> bool {
    let rel = path.strip_prefix(root).unwrap_or(path);
    settings
        .read(cx)
        .languages(cx)
        .language_for_file(rel, "")
        .is_some_and(|lang| &*lang.id == MARKDOWN)
}

/// What the active tab shows, for the commands; `None` when it is not a
/// markdown file or a preview.
enum Active {
    /// The source of a markdown file.
    Source { tab: TabId, path: Arc<Path> },
    /// A static preview.
    Static { tab: TabId, path: Arc<Path> },
    /// The dynamic preview, with the line at its top.
    Dynamic { path: Arc<Path>, line: f32 },
}

fn active(wb: &Workbench, cx: &App) -> Option<Active> {
    let tab = wb.editor_groups().read(cx).model().active_tab()?.clone();
    match tab.item {
        TabItem::File(path) if is_markdown(wb.root(), &path, wb.settings(), cx) => {
            Some(Active::Source { tab: tab.id, path })
        }
        TabItem::MarkdownPreview(path) => Some(Active::Static { tab: tab.id, path }),
        TabItem::MarkdownPreviewDynamic(path) => {
            let line = preview_of(wb, tab.id, cx).map_or(0., |p| p.read(cx).top_line());
            Some(Active::Dynamic { path, line })
        }
        _ => None,
    }
}

/// The code view of tab `id`, once it has been made.
fn code_view_of(wb: &Workbench, id: TabId, cx: &App) -> Option<Entity<CodeView>> {
    wb.editor_groups().read(cx).body_of(id)?.downcast().ok()
}

/// The preview of tab `id`, once it has been made.
fn preview_of(wb: &Workbench, id: TabId, cx: &App) -> Option<Entity<MarkdownPreview>> {
    wb.editor_groups().read(cx).body_of(id)?.downcast().ok()
}

/// Every preview made so far, static or dynamic.
fn previews(wb: &Workbench, cx: &App) -> Vec<Entity<MarkdownPreview>> {
    let groups = wb.editor_groups().read(cx);
    groups
        .model()
        .groups()
        .iter()
        .flat_map(|g| &g.tabs)
        .filter(|t| t.previewed_path().is_some())
        .filter_map(|t| groups.body_of(t.id)?.downcast().ok())
        .collect()
}

/// After a rename of `from` to `to` (absolute): every preview of a file
/// at `from` or under it follows to the new path, keeping its place. It
/// must run in the same update as `Documents::rename`: GPUI delivers that
/// rename's `Closed` event for `from` only once the update ends, and by
/// then no preview may still point at `from`, or it would open the gone
/// path again (and that document would hold its name).
pub(crate) fn retarget_previews(wb: &Workbench, from: &Path, to: &Path, cx: &mut App) {
    let moved: Vec<(Entity<MarkdownPreview>, Arc<Path>)> = previews(wb, cx)
        .into_iter()
        .filter_map(|preview| {
            let new = crate::workspace_state::moved_path(preview.read(cx).path(), from, to)?;
            Some((preview, Arc::from(new)))
        })
        .collect();
    for (preview, path) in moved {
        preview.update(cx, |p, cx| p.follow_rename(path, cx));
    }
}

/// Replaces the source tab `tab` with the preview of `path` in its place,
/// showing the editor's top line at the top, and focuses it.
fn preview_in_place(
    wb: &mut Workbench,
    tab: TabId,
    path: Arc<Path>,
    window: &mut Window,
    cx: &mut Context<Workbench>,
) {
    let row = code_view_of(wb, tab, cx).map_or(0, |v| v.read(cx).scroll_top_row());
    if wb
        .update_editor_groups(cx, |m| m.replace_item(tab, TabItem::MarkdownPreview(path)))
        .is_none()
    {
        return;
    }
    let groups = wb.editor_groups().clone();
    let preview = groups
        .update(cx, |view, cx| view.ensure_active_body(window, cx))
        .and_then(|body| body.downcast::<MarkdownPreview>().ok());
    if let Some(preview) = preview {
        preview.update(cx, |p, cx| p.scroll_to_line(row as f32, cx));
    }
    wb.focus_editor_area(window, cx);
}

/// Replaces the static preview `tab` with the source of `path` in its
/// place, showing the preview's top line at the top, and focuses it.
fn source_in_place(
    wb: &mut Workbench,
    tab: TabId,
    path: Arc<Path>,
    window: &mut Window,
    cx: &mut Context<Workbench>,
) {
    let line = preview_of(wb, tab, cx).map_or(0., |p| p.read(cx).top_line());
    let row = line.floor().max(0.) as u32;
    source_in_place_at(wb, tab, path, row, row, window, cx);
}

/// Replaces the static preview `tab` with the source of `path`, the cursor
/// on `cursor_row` and `scroll_row` at the top.
fn source_in_place_at(
    wb: &mut Workbench,
    tab: TabId,
    path: Arc<Path>,
    cursor_row: u32,
    scroll_row: u32,
    window: &mut Window,
    cx: &mut Context<Workbench>,
) {
    if wb
        .update_editor_groups(cx, |m| m.replace_item(tab, TabItem::File(path.clone())))
        .is_none()
    {
        return;
    }
    go_to(wb, &path, cursor_row, Some(scroll_row), cx);
    wb.focus_editor_area(window, cx);
}

/// Opens the source of `path` in the active group, `line` at the top.
fn source_here(
    wb: &mut Workbench,
    path: Arc<Path>,
    line: f32,
    window: &mut Window,
    cx: &mut Context<Workbench>,
) {
    let row = line.floor().max(0.) as u32;
    wb.open_source(path.clone(), OpenMode::Pinned, cx);
    go_to(wb, &path, row, Some(row), cx);
    wb.focus_editor_area(window, cx);
}

/// Puts the cursor of the active editor (of `path`) on `row`, with
/// `scroll_row` at the top or the row centred; now if the view exists,
/// else once it opens.
fn go_to(
    wb: &mut Workbench,
    path: &Path,
    row: u32,
    scroll_row: Option<u32>,
    cx: &mut Context<Workbench>,
) {
    let point = Point { row, column: 0 };
    let view = wb.active_code_view(cx);
    wb.documents()
        .clone()
        .update(cx, |docs, cx| match (view, scroll_row) {
            (Some(view), Some(scroll)) => docs.go_to_and_scroll(&view, point, scroll, cx),
            (Some(view), None) => docs.go_to(&view, point, cx),
            (None, Some(scroll)) => docs.go_to_and_scroll_when_opened(path, point, scroll),
            (None, None) => docs.go_to_when_opened(path, point),
        });
}

/// Points the view of the dynamic preview (tab `id`) at `path`, when it
/// has been made, with `line` at the top. The editor groups are told, so
/// the scroll links pair it with the editors of its new file.
fn show_dynamic(
    wb: &mut Workbench,
    id: TabId,
    path: Arc<Path>,
    line: Option<f32>,
    window: &mut Window,
    cx: &mut Context<Workbench>,
) {
    let groups = wb.editor_groups().clone();
    groups.update(cx, |view, cx| view.ensure_active_bodies(window, cx));
    if let Some(preview) = preview_of(wb, id, cx) {
        preview.update(cx, |p, cx| {
            p.set_path(path, cx);
            if let Some(line) = line {
                p.scroll_to_line(line, cx);
            }
        });
    }
    groups.update(cx, |_, cx| cx.notify());
}

/// The dynamic preview follows the active tab of the focused group when
/// that group is not the preview's and the tab shows a markdown file
/// (source or static preview); other tabs leave it as it is. Runs whenever
/// the editor groups change.
pub(crate) fn follow_active(wb: &mut Workbench, window: &mut Window, cx: &mut Context<Workbench>) {
    let groups = wb.editor_groups().read(cx);
    let model = groups.model();
    let Some((dynamic_group, dynamic)) = model.dynamic_preview() else {
        return;
    };
    if model.active_group() == dynamic_group {
        return;
    }
    let Some(tab) = model.active_tab() else {
        return;
    };
    let path = match &tab.item {
        TabItem::File(path) if is_markdown(wb.root(), path, wb.settings(), cx) => path.clone(),
        TabItem::MarkdownPreview(path) => path.clone(),
        _ => return,
    };
    if dynamic.previewed_path() == Some(&path) {
        return;
    }
    let (id, tab) = (dynamic.id, tab.id);
    let line = code_view_of(wb, tab, cx)
        .map(|v| v.read(cx).scroll_top_row() as f32)
        .or_else(|| preview_of(wb, tab, cx).map(|p| p.read(cx).top_line()));
    wb.update_editor_groups(cx, |m| m.retarget_dynamic(path.clone()));
    show_dynamic(wb, id, path, line, window, cx);
}

/// A double-click on the block starting at source line `line` of
/// `preview` (`markdown.preview.doubleClickToSwitchToEditor`): a static
/// preview turns into its source in place, the cursor on that line; the
/// dynamic one activates an editor of its file in another group, or opens
/// one in its own group.
pub(crate) fn double_clicked(
    wb: &mut Workbench,
    preview: EntityId,
    line: u32,
    window: &mut Window,
    cx: &mut Context<Workbench>,
) {
    let enabled = wb
        .settings()
        .read(cx)
        .get::<bool>(DOUBLE_CLICK_TO_SWITCH, None, cx)
        .unwrap_or(true);
    if !enabled {
        return;
    }
    let groups = wb.editor_groups().read(cx);
    let found = groups
        .model()
        .groups()
        .iter()
        .enumerate()
        .flat_map(|(gi, g)| g.tabs.iter().map(move |t| (gi, t)))
        .find(|(_, t)| {
            groups
                .body_of(t.id)
                .is_some_and(|b| b.entity_id() == preview)
        })
        .map(|(gi, t)| (gi, t.clone()));
    let Some((group, tab)) = found else {
        return;
    };
    match tab.item {
        TabItem::MarkdownPreview(path) => {
            let top = preview_of(wb, tab.id, cx).map_or(0., |p| p.read(cx).top_line());
            let scroll = (top.floor().max(0.) as u32).min(line);
            source_in_place_at(wb, tab.id, path, line, scroll, window, cx);
        }
        TabItem::MarkdownPreviewDynamic(path) => {
            let editor = wb
                .editor_groups()
                .read(cx)
                .model()
                .groups()
                .iter()
                .enumerate()
                .filter(|(gi, _)| *gi != group)
                .flat_map(|(_, g)| {
                    // The group's active tab first, then the others.
                    let active = g.tabs.iter().filter(|t| Some(t.id) == g.active);
                    active.chain(g.tabs.iter().filter(|t| Some(t.id) != g.active))
                })
                .find(|t| t.item == TabItem::File(path.clone()))
                .map(|t| t.id);
            match editor {
                Some(id) => {
                    wb.update_editor_groups(cx, |m| m.activate(id));
                }
                None => {
                    wb.update_editor_groups(cx, |m| m.focus_group(group));
                    wb.open_source(path.clone(), OpenMode::Pinned, cx);
                }
            }
            go_to(wb, &path, line, None, cx);
            wb.focus_editor_area(window, cx);
        }
        _ => {}
    }
}

/// Follows a link clicked in `preview` (spec §5.6): web and `mailto:` links
/// go to the browser, `#slug` scrolls the preview, another markdown file
/// opens as the Explorer opens it (`markdown.preview.openMarkdownLinks`:
/// `"inEditor"` opens its source) or, from the dynamic preview, becomes
/// the file it shows; `#L10` opens the source at that line, and other
/// files open as the Explorer opens them. A link to a folder or to a file
/// that does not exist does nothing.
pub(crate) fn follow_link(
    wb: &mut Workbench,
    preview: WeakEntity<MarkdownPreview>,
    action: LinkAction,
    window: &mut Window,
    cx: &mut Context<Workbench>,
) {
    // One stat, on a click: no tab for a folder or a dead link.
    if let LinkAction::Markdown { path, .. } | LinkAction::File(path) = &action
        && !path.is_file()
    {
        return;
    }
    match action {
        LinkAction::Web(url) => cx.open_url(&url),
        LinkAction::Anchor(slug) => {
            preview.update(cx, |p, cx| p.scroll_to_slug(&slug, cx)).ok();
        }
        LinkAction::Markdown { path, slug } => {
            let path: Arc<Path> = Arc::from(path);
            let in_editor = wb
                .settings()
                .read(cx)
                .get::<String>(OPEN_MARKDOWN_LINKS, None, cx)
                .is_some_and(|v| v == "inEditor");
            if in_editor {
                wb.open_source(path, OpenMode::Preview, cx);
                wb.focus_editor_area(window, cx);
                return;
            }
            let dynamic = preview.upgrade().filter(|p| p.read(cx).is_dynamic());
            let target = match dynamic {
                Some(dynamic) => {
                    let id = wb
                        .editor_groups()
                        .read(cx)
                        .model()
                        .dynamic_preview()
                        .map(|(_, tab)| tab.id);
                    wb.update_editor_groups(cx, |m| m.retarget_dynamic(path.clone()));
                    if let Some(id) = id {
                        show_dynamic(wb, id, path, None, window, cx);
                    }
                    Some(dynamic)
                }
                None => {
                    wb.open_path(path, OpenMode::Preview, cx);
                    let groups = wb.editor_groups().clone();
                    let opened = groups
                        .update(cx, |view, cx| view.ensure_active_body(window, cx))
                        .and_then(|body| body.downcast::<MarkdownPreview>().ok());
                    wb.focus_editor_area(window, cx);
                    opened
                }
            };
            if let (Some(target), Some(slug)) = (target, slug) {
                target.update(cx, |p, cx| p.scroll_to_slug(&slug, cx));
            }
        }
        LinkAction::Source { path, point } => {
            wb.jump_to(Arc::from(path), point, OpenMode::Preview, cx);
            wb.focus_editor_area(window, cx);
        }
        LinkAction::File(path) => {
            wb.open_path(Arc::from(path), OpenMode::Preview, cx);
            wb.focus_editor_area(window, cx);
        }
        LinkAction::None => {}
    }
}
