//! Breadcrumbs (spec §7.6): the bar under each group's tab bar shows the
//! active file's path split by folder, then the symbols containing the
//! cursor (`crates › lucerna-lsp › src › server.rs › impl Server › start`).
//! The whole row is one button opening Quick Open `@`.
//!
//! The symbol part follows the cursor in 50 ms batches, read from the
//! tree [`SymbolSource`](super::SymbolSource) caches: a cursor move never
//! asks a server anything.

use std::collections::HashMap;
use std::path::{Path, is_separator};
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, Context, Div, IntoElement, ParentElement, SharedString, Stateful, Styled, Task, Window,
    div, prelude::*, px, relative, svg,
};
use lucerna_syntax::{Symbol, SymbolKind, SymbolTree};
use lucerna_theme::UiColors;

use crate::code_view::CodeView;
use crate::editor_groups::{TabId, TabItem};
use crate::layout::color;
use crate::ui::{GRID, TAB_PADDING_X};
use crate::{AppState, SymbolState, Workbench, fonts};

use super::kind_icon;

/// How long cursor moves and symbol changes gather before the rows are
/// computed again.
pub const BREADCRUMBS_BATCH: Duration = Duration::from_millis(50);
/// More segments than this collapse into the first and last
/// [`KEPT_EACH_SIDE`] around an [`Segment::Ellipsis`].
const MAX_SEGMENTS: usize = 12;
const KEPT_EACH_SIDE: usize = 6;
const ICON_SIZE: f32 = 12.;
const SEPARATOR: &str = "›";
const ELLIPSIS: &str = "⋯";

/// `breadcrumbs.filePath` / `breadcrumbs.symbolPath`: all of the path,
/// none of it, or only its last part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreadcrumbMode {
    On,
    Off,
    Last,
}

pub type FilePathMode = BreadcrumbMode;
pub type SymbolPathMode = BreadcrumbMode;

impl BreadcrumbMode {
    /// `"off"` and `"last"`; anything else is `On`, the default.
    fn parse(value: Option<&str>) -> BreadcrumbMode {
        match value {
            Some("off") => BreadcrumbMode::Off,
            Some("last") => BreadcrumbMode::Last,
            _ => BreadcrumbMode::On,
        }
    }

    /// The part of `items` this mode keeps.
    fn keep<T>(self, items: &[T]) -> &[T] {
        match self {
            BreadcrumbMode::On => items,
            BreadcrumbMode::Off => &[],
            BreadcrumbMode::Last => &items[items.len().saturating_sub(1)..],
        }
    }
}

/// One part of a breadcrumbs row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Dir(String),
    File(String),
    Symbol {
        kind: SymbolKind,
        name: String,
    },
    /// Stands for the segments left out of a long row.
    Ellipsis,
}

impl Segment {
    pub fn text(&self) -> &str {
        match self {
            Segment::Dir(name) | Segment::File(name) | Segment::Symbol { name, .. } => name,
            Segment::Ellipsis => ELLIPSIS,
        }
    }
}

/// The segments of a row: `path_text` (as the plain path bar shows it)
/// split by folder, then `symbols` (outermost first), each kept as
/// `file_path` and `symbol_path` say. More than 12 segments keep the
/// first and last 6 around an [`Segment::Ellipsis`].
pub fn breadcrumb_segments(
    path_text: &str,
    symbols: &[&Symbol],
    file_path: FilePathMode,
    symbol_path: SymbolPathMode,
) -> Vec<Segment> {
    let parts: Vec<&str> = path_text
        .split(is_separator)
        .filter(|p| !p.is_empty())
        .collect();
    let last = parts.len().saturating_sub(1);
    let path = parts.iter().enumerate().map(|(i, part)| {
        if i == last {
            Segment::File(part.to_string())
        } else {
            Segment::Dir(part.to_string())
        }
    });
    let path: Vec<Segment> = path.collect();
    let mut segments: Vec<Segment> = file_path.keep(&path).to_vec();
    segments.extend(symbol_path.keep(symbols).iter().map(|s| Segment::Symbol {
        kind: s.kind,
        name: s.name.clone(),
    }));
    if segments.len() > MAX_SEGMENTS {
        let tail = segments.split_off(segments.len() - KEPT_EACH_SIDE);
        segments.truncate(KEPT_EACH_SIDE);
        segments.push(Segment::Ellipsis);
        segments.extend(tail);
    }
    segments
}

/// `segments` as text, joined by ` › `.
pub(crate) fn segments_text(segments: &[Segment]) -> String {
    let texts: Vec<&str> = segments.iter().map(Segment::text).collect();
    texts.join(&format!(" {SEPARATOR} "))
}

/// The `breadcrumbs.*` settings.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BreadcrumbSettings {
    pub enabled: bool,
    pub file_path: FilePathMode,
    pub symbol_path: SymbolPathMode,
    pub icons: bool,
}

impl BreadcrumbSettings {
    pub(crate) fn read(cx: &App) -> BreadcrumbSettings {
        let settings = &cx.global::<AppState>().settings;
        let mode = |key| BreadcrumbMode::parse(settings.get::<String>(key, None).as_deref());
        BreadcrumbSettings {
            enabled: settings.get("breadcrumbs.enabled", None).unwrap_or(true),
            file_path: mode("breadcrumbs.filePath"),
            symbol_path: mode("breadcrumbs.symbolPath"),
            icons: settings.get("breadcrumbs.icons", None).unwrap_or(true),
        }
    }
}

/// The row of a file tab: its segments and the code font of its language.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Crumbs {
    pub segments: Vec<Segment>,
    pub font: SharedString,
}

/// The workbench's breadcrumbs state.
#[derive(Default)]
pub(crate) struct Breadcrumbs {
    /// The batch waiting to run.
    refresh: Option<Task<()>>,
    /// The last symbols of each shown file, used while its symbols for a
    /// newer revision load.
    trees: HashMap<Arc<Path>, Arc<SymbolTree>>,
    /// The active tab of each group when the rows were last computed.
    active: Vec<TabId>,
}

impl Workbench {
    /// The cursor, a tab or the symbols changed: computes the rows again
    /// after [`BREADCRUMBS_BATCH`], once for all changes until then.
    pub(crate) fn breadcrumbs_changed(&mut self, cx: &mut Context<Self>) {
        if self.breadcrumbs.refresh.is_some() {
            return;
        }
        self.breadcrumbs.refresh = Some(cx.spawn(async move |wb, cx| {
            cx.background_executor().timer(BREADCRUMBS_BATCH).await;
            wb.update(cx, |wb, cx| {
                wb.breadcrumbs.refresh = None;
                wb.refresh_breadcrumbs(cx);
            })
            .ok();
        }));
    }

    /// The editor groups changed: when another tab became active (a tab
    /// switch, a split, a close), computes the rows now rather than after
    /// the batch, so the symbols show at once. Makes the bodies of the
    /// active tabs first, as drawing would.
    pub(crate) fn active_tabs_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let groups = self.editor_groups().clone();
        let active: Vec<TabId> = groups
            .read(cx)
            .model()
            .groups()
            .iter()
            .filter_map(|g| g.active)
            .collect();
        if active == self.breadcrumbs.active {
            return;
        }
        if BreadcrumbSettings::read(cx).enabled {
            groups.update(cx, |view, cx| view.ensure_active_bodies(window, cx));
        }
        self.refresh_breadcrumbs(cx);
    }

    /// Hands each group's file tab its row.
    fn refresh_breadcrumbs(&mut self, cx: &mut Context<Self>) {
        let settings = BreadcrumbSettings::read(cx);
        let groups = self.editor_groups().clone();
        self.breadcrumbs.active = groups
            .read(cx)
            .model()
            .groups()
            .iter()
            .filter_map(|g| g.active)
            .collect();
        let shown: Vec<(TabId, String, CodeViewRef)> = if settings.enabled {
            let view = groups.read(cx);
            (0..view.model().groups().len())
                .filter_map(|gi| {
                    let group = &view.model().groups()[gi];
                    let tab = group.tabs.iter().find(|t| Some(t.id) == group.active)?;
                    let TabItem::File(_) = tab.item else {
                        return None;
                    };
                    let text = view.path_bar_text(gi)?;
                    let editor = view
                        .body_of(tab.id)
                        .and_then(|body| body.downcast::<CodeView>().ok());
                    Some((tab.id, text, editor))
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut rows = HashMap::new();
        let mut used = Vec::new();
        for (id, text, editor) in shown {
            let (tree, cursor, font) = match &editor {
                Some(editor) => {
                    let (path, cursor, language) = {
                        let e = editor.read(cx);
                        (
                            self.absolute_path(e.path()),
                            e.cursor(),
                            e.language_id().map(str::to_owned),
                        )
                    };
                    let tree = self.symbol_tree(&path, cx);
                    used.push(path);
                    (
                        tree,
                        Some(cursor),
                        fonts::editor_font_family(language.as_deref(), cx),
                    )
                }
                None => (None, None, fonts::editor_font_family(None, cx)),
            };
            let symbols = match (&tree, cursor) {
                (Some(tree), Some(cursor)) => tree.path_at(cursor),
                _ => Vec::new(),
            };
            let segments =
                breadcrumb_segments(&text, &symbols, settings.file_path, settings.symbol_path);
            rows.insert(id, Crumbs { segments, font });
        }
        self.breadcrumbs.trees.retain(|path, _| used.contains(path));
        groups.update(cx, |view, cx| {
            if settings.enabled {
                view.set_breadcrumbs(rows, cx);
            } else {
                view.clear_breadcrumbs(cx);
            }
        });
    }

    /// The symbols of `path` as cached: the last ones known while newer
    /// ones load.
    fn symbol_tree(&mut self, path: &Arc<Path>, cx: &mut Context<Self>) -> Option<Arc<SymbolTree>> {
        match self.symbols.current(path, cx) {
            SymbolState::Ready(tree) => {
                self.breadcrumbs.trees.insert(path.clone(), tree.clone());
                Some(tree)
            }
            SymbolState::Loading => self.breadcrumbs.trees.get(path).cloned(),
            SymbolState::Unavailable => {
                self.breadcrumbs.trees.remove(path);
                None
            }
        }
    }

    /// A click on the row of group `group`: focuses its editor, then
    /// opens Quick Open `@` for it.
    pub(crate) fn open_breadcrumbs(
        &mut self,
        group: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.update_editor_groups(cx, |m| m.focus_group(group));
        self.show_document_symbols(window, cx);
    }
}

type CodeViewRef = Option<gpui::Entity<CodeView>>;

/// The row of group `group` showing `crumbs`, filling the path bar; the
/// caller makes it clickable. Overflow is cut on the left, so the symbol
/// at the cursor stays in view.
pub(crate) fn breadcrumbs_row(
    group: usize,
    crumbs: &Crumbs,
    icons: bool,
    ui: &UiColors,
) -> Stateful<Div> {
    let mut separator = ui.text_muted;
    separator.a *= 0.6;
    let mut items = Vec::new();
    for (i, segment) in crumbs.segments.iter().enumerate() {
        if i > 0 {
            items.push(
                div()
                    .flex_none()
                    .text_color(color(separator))
                    .child(SEPARATOR)
                    .into_any_element(),
            );
        }
        let item = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(GRID))
            .whitespace_nowrap();
        let item = match segment {
            Segment::Dir(name) => item.child(name.clone()),
            Segment::File(name) => item.text_color(color(ui.text)).child(name.clone()),
            Segment::Symbol { kind, name } => item
                .when(icons, |d| {
                    d.child(
                        svg()
                            .debug_selector(move || format!("breadcrumb-icon-{group}-{i}"))
                            .data(kind_icon(*kind))
                            .flex_none()
                            .size(px(ICON_SIZE))
                            .text_color(color(ui.icon_muted)),
                    )
                })
                .child(div().font_family(crumbs.font.clone()).child(name.clone())),
            Segment::Ellipsis => item.child(ELLIPSIS),
        };
        items.push(item.into_any_element());
    }
    let hover = color(ui.ghost_element_hover);
    div()
        .id(("breadcrumbs", group))
        .debug_selector(move || format!("breadcrumbs-{group}"))
        .flex()
        .size_full()
        .min_w_0()
        .px(px(TAB_PADDING_X))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(
            // Content wider than the row is aligned to its end: the
            // start overflows and is clipped.
            div()
                .flex()
                .flex_row()
                .justify_end()
                .size_full()
                .overflow_hidden()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_none()
                        .items_center()
                        .gap(px(1.5 * GRID))
                        .min_w(relative(1.))
                        .children(items),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_parse_with_on_as_default() {
        assert_eq!(BreadcrumbMode::parse(Some("off")), BreadcrumbMode::Off);
        assert_eq!(BreadcrumbMode::parse(Some("last")), BreadcrumbMode::Last);
        assert_eq!(BreadcrumbMode::parse(Some("on")), BreadcrumbMode::On);
        assert_eq!(BreadcrumbMode::parse(Some("weird")), BreadcrumbMode::On);
        assert_eq!(BreadcrumbMode::parse(None), BreadcrumbMode::On);
    }

    #[test]
    fn text_joins_with_separators() {
        let segments = [
            Segment::Dir("src".into()),
            Segment::Ellipsis,
            Segment::File("main.rs".into()),
        ];
        assert_eq!(segments_text(&segments), "src › ⋯ › main.rs");
        assert_eq!(segments_text(&[]), "");
    }
}
