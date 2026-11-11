//! Go to definition, type definition, implementation, declaration and
//! references (spec §7.3, §7.5 first bullet).
//!
//! A command asks the language servers about the cursor of the active
//! editor, then by the number of results and the
//! `editor.gotoLocation.multiple*` setting jumps, shows the locations, or
//! says "No definition found for 'word'" in a small popover at the cursor
//! for 3 s. A Definition that is the place the cursor is already at runs
//! Go to References instead, as VS Code's `alternativeDefinitionCommand`
//! does.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, Context, Entity, InteractiveElement, IntoElement, ParentElement, SharedString, Styled,
    Task, Window, px,
};
use lucerna_lsp::locations::ResolvedLocation;
use lucerna_text::Point;

use crate::code_view::{CodeView, word_at};
use crate::hover::{PopoverAnchor, place_popover};
use crate::locations_picker::LocationsPicker;
use crate::ui::{UI_FONT_FAMILY, UI_FONT_SIZE_SMALL, popover};
use crate::{AppState, CommandHandler, CommandRegistry, OpenMode, Workbench};

/// How long the "No … found" message stays.
const MESSAGE_DURATION: Duration = Duration::from_secs(3);

/// What is looked up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GotoKind {
    Definition,
    TypeDefinition,
    Implementation,
    Declaration,
    References,
}

impl GotoKind {
    /// The `editor.gotoLocation.*` setting for several results.
    fn multiple_setting(self) -> &'static str {
        match self {
            GotoKind::Definition => "editor.gotoLocation.multipleDefinitions",
            GotoKind::TypeDefinition => "editor.gotoLocation.multipleTypeDefinitions",
            GotoKind::Implementation => "editor.gotoLocation.multipleImplementations",
            GotoKind::Declaration => "editor.gotoLocation.multipleDeclarations",
            GotoKind::References => "editor.gotoLocation.multipleReferences",
        }
    }

    /// What was not found, as the message names it.
    fn noun(self) -> &'static str {
        match self {
            GotoKind::Definition => "definition",
            GotoKind::TypeDefinition => "type definition",
            GotoKind::Implementation => "implementation",
            GotoKind::Declaration => "declaration",
            GotoKind::References => "references",
        }
    }
}

/// How the results are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GotoMode {
    /// One result is jumped to; several follow the
    /// `editor.gotoLocation.multiple*` setting.
    Reveal,
    /// The locations are always listed, even one.
    Peek,
    /// Like `Reveal`, opening in the group to the right.
    Aside,
}

/// What started the lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GotoSource {
    /// A command: an empty answer says so at the cursor.
    Command,
    /// ⌘-click: an empty answer is silent.
    Click,
}

/// `editor.gotoLocation.multiple*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Multiple {
    Peek,
    GotoAndPeek,
    Goto,
}

impl Multiple {
    fn read(kind: GotoKind, language: Option<&str>, cx: &App) -> Multiple {
        let value = cx
            .global::<AppState>()
            .settings
            .get::<String>(kind.multiple_setting(), language);
        match value.as_deref() {
            Some("goto") => Multiple::Goto,
            Some("gotoAndPeek") => Multiple::GotoAndPeek,
            _ => Multiple::Peek,
        }
    }
}

/// Looks up `kind` at the cursor of the active editor and shows the
/// results by `mode`.
pub fn run_goto(
    wb: &mut Workbench,
    kind: GotoKind,
    mode: GotoMode,
    window: &mut Window,
    cx: &mut Context<Workbench>,
) {
    let Some(view) = wb.active_code_view(cx) else {
        return;
    };
    let point = view.read(cx).cursor();
    goto_at(
        wb,
        &view,
        point,
        kind,
        mode,
        GotoSource::Command,
        window,
        cx,
    );
}

/// Looks up `kind` at `point` of `view`. The latest lookup wins: an
/// earlier one still waiting is dropped, which cancels its requests. An
/// answer is dropped, as VS Code does, when it arrives after `view` stopped
/// being the active editor, its cursor left `point` or its text changed.
#[allow(clippy::too_many_arguments)]
pub fn goto_at(
    wb: &mut Workbench,
    view: &Entity<CodeView>,
    point: Point,
    kind: GotoKind,
    mode: GotoMode,
    source: GotoSource,
    window: &mut Window,
    cx: &mut Context<Workbench>,
) {
    let path = wb.absolute_path(view.read(cx).path());
    let revision = view.read(cx).revision();
    let request = wb
        .language_servers()
        .clone()
        .update(cx, |servers, cx| servers.locations(&path, point, kind, cx));
    let view = view.downgrade();
    wb.goto_task = Some(cx.spawn_in(window, async move |wb, cx| {
        let locations = request.await;
        wb.update_in(cx, |wb, window, cx| {
            wb.goto_task = None;
            let Some(view) = view.upgrade() else {
                return;
            };
            let active = wb.active_code_view(cx).as_ref() == Some(&view);
            let v = view.read(cx);
            if !active || v.cursor() != point || v.revision() != revision {
                tracing::debug!(?kind, "goto answer dropped: the editor moved on");
                return;
            }
            let lookup = Lookup {
                view,
                point,
                kind,
                mode,
                source,
            };
            show_results(wb, lookup, locations, window, cx);
        })
        .ok();
    }));
}

/// A finished lookup's question.
struct Lookup {
    view: Entity<CodeView>,
    point: Point,
    kind: GotoKind,
    mode: GotoMode,
    source: GotoSource,
}

/// The decision table of spec §7.3.
fn show_results(
    wb: &mut Workbench,
    lookup: Lookup,
    locations: Vec<ResolvedLocation>,
    window: &mut Window,
    cx: &mut Context<Workbench>,
) {
    let Lookup {
        view,
        point,
        kind,
        mode,
        source,
    } = lookup;
    let Some(first) = locations.first().cloned() else {
        if source == GotoSource::Command {
            let word = message_word(view.read(cx), point);
            show_no_result_at(&view, kind, &word, point, cx);
        }
        return;
    };
    let current = wb.absolute_path(view.read(cx).path());
    let here = locations.len() == 1
        && wb.absolute_path(&first.path) == current
        && contains(&first.range, point);
    if here && kind == GotoKind::Definition && mode != GotoMode::Peek {
        goto_at(
            wb,
            &view,
            point,
            GotoKind::References,
            GotoMode::Reveal,
            source,
            window,
            cx,
        );
        return;
    }
    let aside = mode == GotoMode::Aside;
    if mode != GotoMode::Peek && locations.len() == 1 {
        open_location(wb, &first, aside, window, cx);
        return;
    }
    let multiple = match mode {
        GotoMode::Peek => Multiple::Peek,
        _ => Multiple::read(kind, view.read(cx).language_id(), cx),
    };
    match multiple {
        Multiple::Goto => open_location(wb, &first, aside, window, cx),
        Multiple::GotoAndPeek => {
            open_location(wb, &first, aside, window, cx);
            // Listed from the place jumped to, which is selected first;
            // the picker opens in the aside group, if any, already.
            let jumped = wb.absolute_path(&first.path);
            LocationsPicker::open(wb, kind, locations, jumped, false, window, cx);
        }
        Multiple::Peek => LocationsPicker::open(wb, kind, locations, current, aside, window, cx),
    }
}

/// Opens `loc` as a preview tab with the cursor at its start, recording
/// the jump in the navigation history; `aside` opens it in the group to
/// the right instead, splitting when there is none, and records it there.
pub(crate) fn open_location(
    wb: &mut Workbench,
    loc: &ResolvedLocation,
    aside: bool,
    window: &mut Window,
    cx: &mut Context<Workbench>,
) {
    open_at(wb, loc.path.clone(), loc.range.start, aside, window, cx);
}

/// Opens `path` as a preview tab with the cursor at `point`, as
/// [`open_location`] does.
pub(crate) fn open_at(
    wb: &mut Workbench,
    path: Arc<Path>,
    point: Point,
    aside: bool,
    window: &mut Window,
    cx: &mut Context<Workbench>,
) {
    let path = wb.absolute_path(&path);
    if !aside {
        wb.jump_to(path, point, OpenMode::Preview, cx);
        return;
    }
    wb.update_editor_groups(cx, |m| {
        let next = m.active_group() + 1;
        if !m.focus_group(next) {
            m.split_active();
        }
    });
    // Recorded in the aside group, from the place it showed.
    wb.jump_to(path, point, OpenMode::Preview, cx);
    wb.focus_editor_area(window, cx);
}

/// Whether `p` lies in `range`, ends included: a cursor just after a
/// word is on it.
fn contains(range: &std::ops::Range<Point>, p: Point) -> bool {
    let key = |p: Point| (p.row, p.column);
    key(range.start) <= key(p) && key(p) <= key(range.end)
}

/// The identifier at `point` (or just before it); else the selected text;
/// else nothing.
pub(crate) fn message_word(view: &CodeView, point: Point) -> String {
    let snapshot = view.snapshot();
    let line = snapshot.line(point.row);
    let before = point.column.checked_sub(1).map(|column| Point {
        row: point.row,
        column,
    });
    let word = [Some(point), before].into_iter().flatten().find_map(|p| {
        let range = word_at(snapshot, p);
        let text = line.get(range.start.column as usize..range.end.column as usize)?;
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        (!text.is_empty() && text.chars().all(is_word)).then(|| text.to_owned())
    });
    word.or_else(|| view.selection().and(view.copy_selection()))
        .unwrap_or_default()
}

/// The message of an empty answer: "No definition found for 'word'".
pub(crate) fn no_result_text(kind: GotoKind, word: &str) -> String {
    let noun = kind.noun();
    if word.is_empty() {
        format!("No {noun} found")
    } else {
        format!("No {noun} found for '{word}'")
    }
}

/// Shows that `kind` found nothing for `word` at the cursor of `view`,
/// for 3 s or until the cursor moves or a key is pressed.
pub fn show_no_result(view: &Entity<CodeView>, kind: GotoKind, word: &str, cx: &mut App) {
    let point = view.read(cx).cursor();
    show_no_result_at(view, kind, word, point, cx);
}

/// Like [`show_no_result`], anchored at `point`.
fn show_no_result_at(
    view: &Entity<CodeView>,
    kind: GotoKind,
    word: &str,
    point: Point,
    cx: &mut App,
) {
    let text = SharedString::from(no_result_text(kind, word));
    view.update(cx, |v, cx| {
        let hide = cx.spawn(async move |view, cx| {
            cx.background_executor().timer(MESSAGE_DURATION).await;
            view.update(cx, |v, cx| v.set_goto_message(None, cx)).ok();
        });
        let message = GotoMessage {
            text,
            point,
            _hide: hide,
        };
        v.set_goto_message(Some(message), cx);
    });
}

/// The "No … found" message of a code view.
pub struct GotoMessage {
    text: SharedString,
    point: Point,
    _hide: Task<()>,
}

impl GotoMessage {
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn point(&self) -> Point {
        self.point
    }
}

/// Draws the message of `view` by `anchor` over everything, placed like
/// the hover. Call while the code view prepaints.
pub(crate) fn prepaint_message(
    view: &Entity<CodeView>,
    anchor: Option<PopoverAnchor>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(anchor) = anchor else {
        return;
    };
    let Some(text) = view.read(cx).goto_message().map(SharedString::new) else {
        return;
    };
    let mut element = popover(cx)
        .id("goto-message")
        .debug_selector(|| "goto-message".into())
        .occlude()
        .w_auto()
        .px(px(8.))
        .py(px(4.))
        .font_family(UI_FONT_FAMILY)
        .text_size(px(UI_FONT_SIZE_SMALL))
        .child(text)
        .into_any_element();
    let bounds = place_popover(&mut element, &anchor, window, cx);
    window.defer_draw(element, bounds.origin, 1, None);
}

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    use GotoKind::*;
    use GotoMode::*;
    let all: [(&'static str, &'static str, CommandHandler); 10] = [
        (
            "editor.action.revealDefinition",
            "Go to Definition",
            |wb, _, w, cx| run_goto(wb, Definition, Reveal, w, cx),
        ),
        (
            "editor.action.peekDefinition",
            "Peek Definition",
            |wb, _, w, cx| run_goto(wb, Definition, Peek, w, cx),
        ),
        (
            "editor.action.revealDefinitionAside",
            "Open Definition to the Side",
            |wb, _, w, cx| run_goto(wb, Definition, Aside, w, cx),
        ),
        (
            "editor.action.goToTypeDefinition",
            "Go to Type Definition",
            |wb, _, w, cx| run_goto(wb, TypeDefinition, Reveal, w, cx),
        ),
        (
            "editor.action.peekTypeDefinition",
            "Peek Type Definition",
            |wb, _, w, cx| run_goto(wb, TypeDefinition, Peek, w, cx),
        ),
        (
            "editor.action.goToImplementation",
            "Go to Implementations",
            |wb, _, w, cx| run_goto(wb, Implementation, Reveal, w, cx),
        ),
        (
            "editor.action.peekImplementation",
            "Peek Implementations",
            |wb, _, w, cx| run_goto(wb, Implementation, Peek, w, cx),
        ),
        (
            "editor.action.revealDeclaration",
            "Go to Declaration",
            |wb, _, w, cx| run_goto(wb, Declaration, Reveal, w, cx),
        ),
        (
            "editor.action.peekDeclaration",
            "Peek Declaration",
            |wb, _, w, cx| run_goto(wb, Declaration, Peek, w, cx),
        ),
        (
            "editor.action.goToReferences",
            "Go to References",
            |wb, _, w, cx| run_goto(wb, References, Reveal, w, cx),
        ),
    ];
    for (id, title, handler) in all {
        commands.register(id, title, handler);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(row: u32, column: u32) -> Point {
        Point { row, column }
    }

    #[test]
    fn message_copy() {
        assert_eq!(
            no_result_text(GotoKind::Definition, "main"),
            "No definition found for 'main'"
        );
        assert_eq!(
            no_result_text(GotoKind::References, "x"),
            "No references found for 'x'"
        );
        assert_eq!(
            no_result_text(GotoKind::TypeDefinition, ""),
            "No type definition found"
        );
    }

    #[test]
    fn contains_includes_both_ends() {
        let r = pt(0, 3)..pt(0, 7);
        assert!(contains(&r, pt(0, 3)));
        assert!(contains(&r, pt(0, 7)));
        assert!(!contains(&r, pt(0, 8)));
        assert!(!contains(&r, pt(1, 4)));
    }
}
