//! One terminal's view: holds a `lucerna_terminal::Terminal`, turns its
//! events into redraws (batched over 4 ms, spec §5.1) and draws it with
//! [`TerminalElement`](super::element::TerminalElement).

use std::io;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc::UnboundedReceiver;
use futures::future::{Either, select};
use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, Global, IntoElement,
    ParentElement, Render, SharedString, Styled, Subscription, Task, Window, div, prelude::*,
};
use lucerna_terminal::{Batch, Terminal, TerminalEvent, TerminalOptions, TitleState};

use super::element::{Frame, TerminalElement};
use super::settings::spawn_options;

/// How long events are gathered after the first one before the view
/// handles them together (spec §5.1).
const BATCH_WINDOW: Duration = Duration::from_millis(4);

/// Starts a terminal. The workbench holds one; the default runs
/// [`Terminal::spawn`], tests install a fake.
pub type TerminalSpawner =
    Rc<dyn Fn(TerminalOptions) -> io::Result<(Terminal, UnboundedReceiver<TerminalEvent>)>>;

/// The spawner new workbenches start with, when set; see
/// [`set_default_terminal_spawner`].
struct DefaultTerminalSpawner(TerminalSpawner);

impl Global for DefaultTerminalSpawner {}

/// Makes `spawner` the one every workbench created afterwards starts with,
/// instead of [`Terminal::spawn`]. For tests, where a terminal restored
/// with the window opens before [`crate::Workbench::set_terminal_spawner`]
/// can be called.
pub fn set_default_terminal_spawner(spawner: TerminalSpawner, cx: &mut App) {
    cx.set_global(DefaultTerminalSpawner(spawner));
}

/// The spawner a new workbench starts with.
pub(crate) fn default_spawner(cx: &App) -> TerminalSpawner {
    match cx.try_global::<DefaultTerminalSpawner>() {
        Some(default) => default.0.clone(),
        None => Rc::new(Terminal::spawn),
    }
}

/// Starts a terminal in `cwd` with `spawner` and makes its view; the error
/// text when it could not start.
pub(crate) fn spawn_view(
    spawner: &TerminalSpawner,
    cwd: &Path,
    window: &mut Window,
    cx: &mut App,
) -> Result<Entity<TerminalView>, SharedString> {
    match spawner(spawn_options(cwd)) {
        Ok((terminal, events)) => Ok(cx.new(|cx| TerminalView::new(terminal, events, window, cx))),
        Err(error) => Err(format!("The terminal could not start: {error}").into()),
    }
}

/// What a terminal view tells its tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalViewEvent {
    /// The tab's title changed to this.
    TitleChanged(SharedString),
}

pub struct TerminalView {
    terminal: Terminal,
    pub(super) focus: FocusHandle,
    title_state: TitleState,
    /// Bells rung since the view was made; Task 12 acts on them.
    bells: u64,
    /// The exit code once the process has exited (`Some(None)`: killed by
    /// a signal).
    exited: Option<Option<i32>>,
    /// Rows and columns last sent to the terminal.
    pub(super) grid: (u16, u16),
    pub(super) frames_drawn: u64,
    /// The last laid-out frame, kept for tests to read what was drawn.
    pub(super) last_frame: Option<Rc<Frame>>,
    /// The palette and the theme generation it was built for.
    pub(super) palette: Option<(u64, Rc<lucerna_terminal::TerminalPalette>)>,
    _events: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TerminalViewEvent> for TerminalView {}

impl TerminalView {
    pub fn new(
        terminal: Terminal,
        events: UnboundedReceiver<TerminalEvent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        let snapshot = terminal.snapshot();
        let subscriptions = vec![
            cx.on_focus(&focus, window, |_, _, cx| cx.notify()),
            cx.on_blur(&focus, window, |_, _, cx| cx.notify()),
        ];
        TerminalView {
            terminal,
            focus,
            title_state: TitleState::default(),
            bells: 0,
            exited: None,
            grid: (snapshot.rows, snapshot.cols),
            frames_drawn: 0,
            last_frame: None,
            palette: None,
            _events: Self::pump_events(events, cx),
            _subscriptions: subscriptions,
        }
    }

    /// Handles the first event at once, then gathers the next ones for
    /// [`BATCH_WINDOW`] (or until a batch is full) and handles them
    /// together, so a flood of output asks for one frame per window.
    fn pump_events(
        mut events: UnboundedReceiver<TerminalEvent>,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn(async move |this, cx| {
            while let Some(first) = events.next().await {
                let mut batch = Batch::default();
                batch.push(first);
                if this.update(cx, |view, cx| view.apply(batch, cx)).is_err() {
                    return;
                }
                let mut batch = Batch::default();
                let mut closed = false;
                let mut window_ends = cx.background_executor().timer(BATCH_WINDOW);
                loop {
                    match select(events.next(), &mut window_ends).await {
                        Either::Left((Some(event), _)) => {
                            if batch.push(event) {
                                break;
                            }
                        }
                        Either::Left((None, _)) => {
                            closed = true;
                            break;
                        }
                        Either::Right(_) => break,
                    }
                }
                if !batch.is_empty() && this.update(cx, |view, cx| view.apply(batch, cx)).is_err() {
                    return;
                }
                if closed {
                    return;
                }
            }
        })
    }

    fn apply(&mut self, batch: Batch, cx: &mut Context<Self>) {
        let mut redraw = batch.wakeup;
        for event in batch.events {
            match event {
                TerminalEvent::Wakeup => redraw = true,
                TerminalEvent::Title(title) => {
                    // The foreground process is polled later (Task 11).
                    self.title_state.on_osc_title(title, None);
                    let title = self.title_state.tab_title(None);
                    cx.emit(TerminalViewEvent::TitleChanged(title.into()));
                }
                TerminalEvent::Bell => self.bells += 1,
                TerminalEvent::Exited(code) => {
                    self.exited = Some(code);
                    redraw = true;
                }
                // OSC 52 writes reach the clipboard with the copy commands.
                TerminalEvent::ClipboardStore(_) => {}
            }
        }
        if redraw {
            cx.notify();
        }
    }

    pub fn terminal(&self) -> &Terminal {
        &self.terminal
    }

    /// Rows and columns last sent to the terminal.
    pub fn grid_size(&self) -> (u16, u16) {
        self.grid
    }

    /// How many times the view has been painted.
    pub fn frames_drawn(&self) -> u64 {
        self.frames_drawn
    }

    /// Bells the terminal has rung.
    pub fn bells(&self) -> u64 {
        self.bells
    }

    /// The exit code once the process exited; `Some(None)` when a signal
    /// killed it.
    pub fn exited(&self) -> Option<Option<i32>> {
        self.exited
    }

    /// The text drawn on screen row `row` in the last frame, each run at
    /// its column.
    #[doc(hidden)]
    pub fn painted_row_text(&self, row: usize) -> String {
        self.last_frame
            .as_ref()
            .map(|f| f.row_text(row))
            .unwrap_or_default()
    }

    /// The cursor drawn in the last frame, by its debug selector name:
    /// `terminal-cursor-{block,bar,underline,outline}`.
    #[doc(hidden)]
    pub fn painted_cursor(&self) -> Option<&'static str> {
        self.last_frame.as_ref()?.cursor_selector()
    }

    /// The background the last frame was filled with.
    #[doc(hidden)]
    pub fn painted_background(&self) -> Option<gpui::Rgba> {
        Some(self.last_frame.as_ref()?.background)
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TerminalView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("terminal-view")
            .debug_selector(|| "terminal-view".into())
            .track_focus(&self.focus)
            .size_full()
            .child(TerminalElement::new(cx.entity()))
    }
}
