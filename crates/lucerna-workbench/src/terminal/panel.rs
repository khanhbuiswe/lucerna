//! The terminal panel's tabs: which terminals are open, in what order, and
//! which one is shown. The panel frame (`layout::panel`) draws the tab bar
//! from [`tab_elements`] and this entity as its body.

use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, IntoElement,
    MouseButton, ParentElement, Render, SharedString, Styled, Window, div, prelude::*, px, svg,
};

use super::view::TerminalView;
use crate::AppState;
use crate::icons;
use crate::layout::color;
use crate::ui::{self, GRID, TAB_PADDING_X, UI_FONT_SIZE_SMALL};

/// The title a new terminal shows until its shell names it.
const DEFAULT_TITLE: &str = "Terminal";
/// Tab titles longer than this many characters end in `…`.
const MAX_TITLE_CHARS: usize = 24;

/// What a tab's title color says about its terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabState {
    Normal,
    /// The terminal rang its bell since it was last shown (`warning`).
    Bell,
    /// The process exited with a non-zero code or a signal (`error`).
    Failed,
}

pub struct TerminalTab {
    pub id: u64,
    pub title: SharedString,
    pub state: TabState,
    pub view: Option<Entity<TerminalView>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelEvent {
    /// The last tab closed.
    Empty,
}

pub struct TerminalPanel {
    tabs: Vec<TerminalTab>,
    active: usize,
    focus: FocusHandle,
    next_id: u64,
    /// The tab under the mouse: it shows its close button.
    hovered: Option<u64>,
}

impl EventEmitter<PanelEvent> for TerminalPanel {}

impl TerminalPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        TerminalPanel {
            tabs: Vec::new(),
            active: 0,
            focus: cx.focus_handle(),
            next_id: 1,
            hovered: None,
        }
    }

    pub fn tabs(&self) -> &[TerminalTab] {
        &self.tabs
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    /// The index of the shown tab; `None` without tabs.
    pub fn active(&self) -> Option<usize> {
        (!self.tabs.is_empty()).then_some(self.active)
    }

    /// The panel's focus handle; the frame tracks it, so it holds the
    /// focus of every terminal view inside.
    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    /// Adds a tab after the others and shows it. Returns its id.
    pub fn add_tab(&mut self, view: Option<Entity<TerminalView>>, cx: &mut Context<Self>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(TerminalTab {
            id,
            title: DEFAULT_TITLE.into(),
            state: TabState::Normal,
            view,
        });
        self.active = self.tabs.len() - 1;
        cx.notify();
        id
    }

    /// Closes the tab `id`, showing its right neighbour (or the new last
    /// tab) when it was shown. Emits [`PanelEvent::Empty`] when it was the
    /// last one.
    pub fn close_tab(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.tabs.iter().position(|t| t.id == id) else {
            return;
        };
        self.tabs.remove(index);
        if self.hovered == Some(id) {
            self.hovered = None;
        }
        if index < self.active {
            self.active -= 1;
        }
        self.active = self.active.min(self.tabs.len().saturating_sub(1));
        cx.notify();
        if self.tabs.is_empty() {
            cx.emit(PanelEvent::Empty);
        }
    }

    pub fn activate(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.tabs.len() && index != self.active {
            self.active = index;
            cx.notify();
        }
    }

    /// Moves the tab at `from` to `to`; the shown tab stays shown.
    pub fn move_tab(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        if from >= self.tabs.len() || from == to {
            return;
        }
        let active_id = self.tabs[self.active].id;
        let tab = self.tabs.remove(from);
        let to = to.min(self.tabs.len());
        self.tabs.insert(to, tab);
        self.active = self
            .tabs
            .iter()
            .position(|t| t.id == active_id)
            .unwrap_or(0);
        cx.notify();
    }

    fn set_hovered(&mut self, id: u64, hovered: bool, cx: &mut Context<Self>) {
        if hovered {
            self.hovered = Some(id);
        } else if self.hovered == Some(id) {
            self.hovered = None;
        }
        cx.notify();
    }
}

/// `title` cut to [`MAX_TITLE_CHARS`] characters, ending in `…` when cut.
pub fn tab_title(title: &str) -> String {
    if title.chars().count() <= MAX_TITLE_CHARS {
        return title.to_string();
    }
    let mut cut: String = title.chars().take(MAX_TITLE_CHARS - 1).collect();
    cut.push('…');
    cut
}

/// Drag payload while a tab is dragged to a new place.
struct TabDrag {
    index: usize,
    title: SharedString,
}

/// What follows the cursor while a tab is dragged: its title on a tab.
struct TabGhost {
    title: SharedString,
}

impl Render for TabGhost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = &cx.global::<AppState>().theme.ui;
        div()
            .px(px(TAB_PADDING_X))
            .py(px(GRID))
            .rounded(px(GRID))
            .border_1()
            .border_color(color(ui.border))
            .bg(color(ui.editor_background))
            .text_size(px(UI_FONT_SIZE_SMALL))
            .text_color(color(ui.text))
            .child(self.title.clone())
    }
}

/// The tabs of `panel` for its tab bar (debug selectors `terminal-tab-{i}`
/// and `terminal-tab-close-{i}`): click shows a tab, middle-click or ×
/// closes it, dragging one onto another moves it there.
pub(crate) fn tab_elements(panel: &Entity<TerminalPanel>, cx: &App) -> Vec<AnyElement> {
    let ui = &cx.global::<AppState>().theme.ui;
    let state = panel.read(cx);
    let drop_bg = color(ui.ghost_element_hover);
    state
        .tabs
        .iter()
        .enumerate()
        .map(|(i, tab)| {
            let id = tab.id;
            let active = i == state.active;
            let title: SharedString = tab_title(&tab.title).into();
            let title_color = match tab.state {
                TabState::Bell => ui.warning,
                TabState::Failed => ui.error,
                TabState::Normal if active => ui.text,
                TabState::Normal => ui.text_muted,
            };
            let close = (active || state.hovered == Some(id)).then(|| {
                let panel = panel.clone();
                ui::TabClose {
                    id: ("terminal-tab-close", id).into(),
                    selector: format!("terminal-tab-close-{i}"),
                    on_click: Box::new(move |_: &ClickEvent, _, cx: &mut App| {
                        panel.update(cx, |p, cx| p.close_tab(id, cx));
                    }),
                }
            });
            let icon = svg()
                .data(icons::TERMINAL)
                .flex_none()
                .size(px(14.))
                .text_color(color(ui.icon_muted))
                .into_any_element();
            let drag = TabDrag {
                index: i,
                title: title.clone(),
            };
            let (on_hover, on_click, on_middle, on_drop) =
                (panel.clone(), panel.clone(), panel.clone(), panel.clone());
            ui::tab(
                ui::TabProps {
                    id: ("terminal-tab", id).into(),
                    selector: format!("terminal-tab-{i}"),
                    active,
                    title,
                    title_color,
                    icon: Some(icon),
                    italic: false,
                    strike: None,
                    close,
                    dot: None,
                },
                cx,
            )
            .on_hover(move |hovered: &bool, _, cx| {
                on_hover.update(cx, |p, cx| p.set_hovered(id, *hovered, cx));
            })
            .on_click(move |_, _, cx| {
                on_click.update(cx, |p, cx| p.activate(i, cx));
            })
            .on_mouse_up(MouseButton::Middle, move |_, _, cx| {
                on_middle.update(cx, |p, cx| p.close_tab(id, cx));
            })
            .on_drag(drag, |drag: &TabDrag, _, _, cx: &mut App| {
                let title = drag.title.clone();
                cx.new(|_| TabGhost { title })
            })
            .drag_over::<TabDrag>(move |s, _, _, _| s.bg(drop_bg))
            .on_drop(move |drag: &TabDrag, _, cx| {
                on_drop.update(cx, |p, cx| p.move_tab(drag.index, i, cx));
            })
            .into_any_element()
        })
        .collect()
}

impl Render for TerminalPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let background = color(cx.global::<AppState>().theme.terminal.background);
        let view = self.tabs.get(self.active).and_then(|tab| tab.view.clone());
        div()
            .debug_selector(|| "terminal-body".into())
            .size_full()
            .bg(background)
            .children(view)
    }
}
