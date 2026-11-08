//! The root view: shell layout, view slots and VS Code–style key dispatch.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyView, App, Context, DragMoveEvent, Entity, FocusHandle, Focusable, IntoElement,
    KeyDownEvent, ParentElement, Pixels, Render, Styled, Window, div, prelude::*,
};
use lucerna_settings::{Context as WhenContext, ContextValue, Keystroke, Resolution};
use serde_json::Value;

use crate::editor_groups::{self, EditorGroups, EditorGroupsView, OpenMode};
use crate::layout::sidebar::SidebarSash;
use crate::layout::{
    ACTIVITY_BAR_WIDTH, SIDEBAR_DEFAULT_WIDTH, SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH, StatusBarSide,
    activity_bar, color, sidebar, status_bar,
};
use crate::{AppState, CommandRegistry, keys};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarView {
    Explorer,
    Search,
}

pub struct Workbench {
    root: PathBuf,
    commands: Rc<CommandRegistry>,
    focus: FocusHandle,
    sidebar_focus: FocusHandle,
    editor_focus: FocusHandle,
    sidebar_visible: bool,
    active_sidebar: SidebarView,
    sidebar_width: Pixels,
    explorer_view: Option<AnyView>,
    search_view: Option<AnyView>,
    editor_groups: Entity<EditorGroupsView>,
    editor_area: Option<AnyView>,
    status_left: Vec<AnyView>,
    status_right: Vec<AnyView>,
    /// Keystrokes of a chord in progress, e.g. `[cmd+k]`.
    pending_chord: Vec<Keystroke>,
}

impl Workbench {
    /// Focuses the editor area, so key dispatch and `editorFocus`
    /// bindings work from the start.
    ///
    /// # Panics
    /// If the [`AppState`] global is not set: rendering and key dispatch
    /// read the theme and keymap from it.
    pub fn new(root: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        assert!(
            cx.has_global::<AppState>(),
            "set the AppState global before creating the Workbench"
        );
        let focus = cx.focus_handle();
        let editor_focus = cx.focus_handle();
        window.focus(&editor_focus, cx);
        let mut commands = CommandRegistry::default();
        register_shell_commands(&mut commands);
        editor_groups::register_commands(&mut commands);
        let editor_groups = cx.new(|_| EditorGroupsView::default());
        Workbench {
            root,
            commands: Rc::new(commands),
            focus,
            sidebar_focus: cx.focus_handle(),
            editor_focus,
            sidebar_visible: true,
            active_sidebar: SidebarView::Explorer,
            sidebar_width: SIDEBAR_DEFAULT_WIDTH,
            explorer_view: None,
            search_view: None,
            editor_area: Some(editor_groups.clone().into()),
            editor_groups,
            status_left: Vec::new(),
            status_right: Vec::new(),
            pending_chord: Vec::new(),
        }
    }

    /// The opened folder.
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn commands(&self) -> &CommandRegistry {
        &self.commands
    }

    /// For other modules to register their commands at startup.
    pub fn commands_mut(&mut self) -> &mut CommandRegistry {
        Rc::make_mut(&mut self.commands)
    }

    /// Runs a registered command. Returns `false` when `id` is unknown.
    pub fn run_command(
        &mut self,
        id: &str,
        args: Option<&Value>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        // The registry is shared so a handler may borrow `self` mutably.
        let commands = Rc::clone(&self.commands);
        commands.run(id, args, self, window, cx)
    }

    /// The editor groups view, installed in the editor area by [`Workbench::new`].
    pub fn editor_groups(&self) -> &Entity<EditorGroupsView> {
        &self.editor_groups
    }

    /// Changes the editor groups model and redraws it.
    pub fn update_editor_groups<R>(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut EditorGroups) -> R,
    ) -> R {
        self.editor_groups
            .update(cx, |view, cx| view.update_model(cx, f))
    }

    /// Opens `path` in the editor area. `Preview` honours the
    /// `workbench.editor.enablePreview` setting.
    pub fn open_path(&mut self, path: Arc<Path>, mode: OpenMode, cx: &mut Context<Self>) {
        let preview_enabled = cx
            .global::<AppState>()
            .settings
            .get::<bool>("workbench.editor.enablePreview", None)
            .unwrap_or(true);
        self.update_editor_groups(cx, |m| m.open(path, mode, preview_enabled));
    }

    /// The `when`-clause context for key bindings.
    pub fn context_keys(&self, window: &Window, cx: &App) -> WhenContext {
        let sidebar_focus = self.sidebar_visible && self.sidebar_focus.contains_focused(window, cx);
        let mut ctx = WhenContext::default();
        let mut flag = |key: &str, on: bool| ctx.set(key, ContextValue::Bool(on));
        flag(
            "editorFocus",
            self.editor_focus.contains_focused(window, cx),
        );
        flag("sideBarVisible", self.sidebar_visible);
        flag("sideBarFocus", sidebar_focus);
        flag(
            "explorerViewletFocus",
            sidebar_focus && self.active_sidebar == SidebarView::Explorer,
        );
        flag(
            "searchViewletFocus",
            sidebar_focus && self.active_sidebar == SidebarView::Search,
        );
        // Owned by Quick Open and the find widget once they exist.
        flag("inQuickOpen", false);
        flag("findWidgetVisible", false);
        let active = self.editor_groups.read(cx).model().active_tab();
        let path = active.map(|tab| &tab.path);
        let extname = path
            .and_then(|p| p.extension())
            .map(|ext| format!(".{}", ext.to_string_lossy()))
            .unwrap_or_default();
        let lang_id = path
            .and_then(|p| cx.global::<AppState>().languages.language_for_path(p))
            .map(|lang| lang.id.to_string())
            .unwrap_or_default();
        ctx.set("resourceExtname", ContextValue::Str(extname));
        ctx.set("editorLangId", ContextValue::Str(lang_id));
        ctx
    }

    pub fn sidebar_visible(&self) -> bool {
        self.sidebar_visible
    }

    pub fn active_sidebar(&self) -> SidebarView {
        self.active_sidebar
    }

    pub fn sidebar_width(&self) -> Pixels {
        self.sidebar_width
    }

    /// Sets the sidebar width, clamped to 160–600 px.
    pub fn set_sidebar_width(&mut self, width: Pixels, cx: &mut Context<Self>) {
        self.sidebar_width = width.clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);
        cx.notify();
    }

    /// Shows or hides the sidebar. Hiding it moves focus out of it.
    pub fn toggle_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sidebar_visible && self.sidebar_focus.contains_focused(window, cx) {
            self.focus_editor_area(window, cx);
        }
        self.sidebar_visible = !self.sidebar_visible;
        cx.notify();
    }

    /// Shows the sidebar with `view` active and focuses it.
    pub fn show_sidebar_view(
        &mut self,
        view: SidebarView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sidebar_visible = true;
        self.active_sidebar = view;
        self.focus_sidebar(window, cx);
        cx.notify();
    }

    /// Activity bar click: clicking the active view's icon hides the sidebar.
    pub(crate) fn activity_clicked(
        &mut self,
        view: SidebarView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sidebar_visible && self.active_sidebar == view {
            self.toggle_sidebar(window, cx);
        } else {
            self.show_sidebar_view(view, window, cx);
        }
    }

    pub fn focus_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.sidebar_focus, cx);
        cx.notify();
    }

    pub fn focus_editor_area(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.editor_focus, cx);
        cx.notify();
    }

    /// Puts `view` in the sidebar slot for `kind`; shown while `kind` is active.
    pub fn set_sidebar_view(&mut self, kind: SidebarView, view: AnyView, cx: &mut Context<Self>) {
        match kind {
            SidebarView::Explorer => self.explorer_view = Some(view),
            SidebarView::Search => self.search_view = Some(view),
        }
        cx.notify();
    }

    /// Replaces the editor area placeholder.
    pub fn set_editor_area(&mut self, view: AnyView, cx: &mut Context<Self>) {
        self.editor_area = Some(view);
        cx.notify();
    }

    /// The view in the sidebar slot for `kind`, if one was set.
    pub fn sidebar_view(&self, kind: SidebarView) -> Option<&AnyView> {
        match kind {
            SidebarView::Explorer => self.explorer_view.as_ref(),
            SidebarView::Search => self.search_view.as_ref(),
        }
    }

    /// The view in the editor area, if one replaced the placeholder.
    pub fn editor_area(&self) -> Option<&AnyView> {
        self.editor_area.as_ref()
    }

    /// The status bar items on `side`, in order.
    pub fn status_items(&self, side: StatusBarSide) -> &[AnyView] {
        match side {
            StatusBarSide::Left => &self.status_left,
            StatusBarSide::Right => &self.status_right,
        }
    }

    /// Appends an item to one end of the status bar.
    pub fn add_status_item(&mut self, side: StatusBarSide, view: AnyView, cx: &mut Context<Self>) {
        match side {
            StatusBarSide::Left => self.status_left.push(view),
            StatusBarSide::Right => self.status_right.push(view),
        }
        cx.notify();
    }

    /// The chord typed so far, e.g. `"⌘K"`, while waiting for its next key.
    pub fn pending_chord_label(&self) -> Option<String> {
        if self.pending_chord.is_empty() {
            return None;
        }
        Some(
            self.pending_chord
                .iter()
                .map(Keystroke::label)
                .collect::<Vec<_>>()
                .join(" "),
        )
    }

    /// Resolves the key against the user keymap (GPUI's own keymap is not
    /// used). Runs in the capture phase so bindings win over child views;
    /// unbound keys, and keys bound to commands nobody registered,
    /// propagate to them.
    ///
    /// Because this runs before any text input sees the key, a binding for
    /// a plain character (no cmd/ctrl/alt) must carry a `when` clause, or it
    /// swallows that character in every text input.
    fn dispatch_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let stroke = keys::to_keystroke(ev);
        let ctx = self.context_keys(window, cx);
        let had_pending = !self.pending_chord.is_empty();
        let mut chord = std::mem::take(&mut self.pending_chord);
        chord.push(stroke.clone());
        let keymap = &cx.global::<AppState>().keymap;
        let mut resolution = keymap.resolve(&chord, &ctx);
        if had_pending && resolution == Resolution::NoMatch {
            // VS Code: a key that breaks a chord is tried again on its own.
            chord = vec![stroke];
            resolution = keymap.resolve(&chord, &ctx);
        }
        match resolution {
            Resolution::Command { command, args } => {
                if self.run_command(&command, args.as_ref(), window, cx) {
                    cx.stop_propagation();
                } else {
                    tracing::debug!(command, "key bound to an unregistered command");
                }
            }
            Resolution::Pending => {
                cx.stop_propagation();
                self.pending_chord = chord;
            }
            Resolution::NoMatch => {}
        }
        if had_pending || !self.pending_chord.is_empty() {
            cx.notify();
        }
    }
}

fn register_shell_commands(commands: &mut CommandRegistry) {
    commands.register(
        "workbench.action.toggleSidebarVisibility",
        "View: Toggle Primary Side Bar Visibility",
        |wb, _, window, cx| wb.toggle_sidebar(window, cx),
    );
    commands.register(
        "workbench.view.explorer",
        "View: Show Explorer",
        |wb, _, window, cx| wb.show_sidebar_view(SidebarView::Explorer, window, cx),
    );
    commands.register(
        "workbench.view.search",
        "View: Show Search",
        |wb, _, window, cx| wb.show_sidebar_view(SidebarView::Search, window, cx),
    );
}

impl Focusable for Workbench {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Workbench {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<AppState>().theme.clone();
        let ui = &theme.ui;
        let sidebar_slot = match self.active_sidebar {
            SidebarView::Explorer => self.explorer_view.clone(),
            SidebarView::Search => self.search_view.clone(),
        };
        let editor_area = div()
            .id("editor-area")
            .track_focus(&self.editor_focus)
            .flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(color(ui.editor_background))
            .children(self.editor_area.clone())
            .when(self.editor_area.is_none(), |area| {
                area.justify_center()
                    .items_center()
                    .text_color(color(ui.text_muted))
                    .child("Lucerna")
            });

        div()
            .id("workbench")
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(Self::dispatch_key))
            .on_drag_move(cx.listener(|wb, ev: &DragMoveEvent<SidebarSash>, _, cx| {
                wb.set_sidebar_width(ev.event.position.x - ACTIVITY_BAR_WIDTH, cx)
            }))
            .flex()
            .flex_col()
            .size_full()
            .bg(color(ui.background))
            .text_color(color(ui.text))
            .text_sm()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(activity_bar::render(
                        self.active_sidebar,
                        self.sidebar_visible,
                        ui,
                        cx,
                    ))
                    .when(self.sidebar_visible, |row| {
                        row.child(sidebar::render(
                            self.sidebar_width,
                            self.active_sidebar,
                            sidebar_slot,
                            &self.sidebar_focus,
                            ui,
                        ))
                    })
                    .child(editor_area),
            )
            .child(status_bar::render(
                &self.status_left,
                self.pending_chord_label(),
                &self.status_right,
                ui,
            ))
    }
}
