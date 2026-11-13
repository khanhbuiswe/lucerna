//! One terminal's view: holds a `lucerna_terminal::Terminal`, turns its
//! events into redraws (batched over 4 ms, spec §5.1) and draws it with
//! [`TerminalElement`](super::element::TerminalElement).

use std::io;
use std::ops::Range;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc::UnboundedReceiver;
use futures::future::{Either, select};
use gpui::{
    App, AppContext, Bounds, ClipboardItem, Context, Entity, EntityInputHandler, EventEmitter,
    FocusHandle, Focusable, Global, IntoElement, KeyDownEvent, ParentElement, Pixels, Point,
    Render, SharedString, Styled, Subscription, Task, UTF16Selection, Window, div, prelude::*,
};
use lucerna_terminal::{Batch, Scroll, Terminal, TerminalEvent, TerminalOptions, TitleState};

use super::element::{Frame, TerminalElement};
use super::mouse::MouseState;
use super::settings::spawn_options;
use crate::ui;

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
    pub(super) terminal: Terminal,
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
    /// An IME composition in progress: drawn at the cursor, not yet sent.
    pub(super) marked: Option<String>,
    /// The last paint gave the window this view's input handler, so typed
    /// text arrives through it rather than as key-downs.
    pub(super) input_handler_installed: bool,
    pub(super) mouse: MouseState,
    /// A paste of several lines waiting for the user to confirm it.
    pub(super) pending_paste: Option<String>,
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
            marked: None,
            input_handler_installed: false,
            mouse: MouseState::default(),
            pending_paste: None,
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
                // OSC 52 writes (reads are refused in the terminal crate).
                TerminalEvent::ClipboardStore(text) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }
        }
        if redraw {
            cx.notify();
        }
    }

    pub fn terminal(&self) -> &Terminal {
        &self.terminal
    }

    /// Sends typed bytes to the shell and follows the output again
    /// (spec §6.5: typing jumps to the bottom).
    pub fn write_input(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        if bytes.is_empty() {
            return;
        }
        self.terminal.input(bytes);
        self.terminal.scroll(Scroll::Bottom);
        cx.notify();
    }

    /// Pastes `text` (bracketed when the app asked for it) and follows the
    /// output again.
    pub fn paste_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if text.is_empty() {
            return;
        }
        self.terminal.paste(text);
        self.terminal.scroll(Scroll::Bottom);
        cx.notify();
    }

    /// The IME composition in progress, if any.
    pub fn marked_text(&self) -> Option<&str> {
        self.marked.as_deref()
    }

    /// Whether `ev` is left to the input handler: during an IME
    /// composition every key is, and otherwise a key that types text
    /// (GPUI's `key_char`, without ctrl or cmd, and without alt when alt is
    /// meta). Only once a paint installed the handler; before that the
    /// workbench encodes the key itself.
    pub(crate) fn input_handler_takes(&self, ev: &KeyDownEvent, option_is_meta: bool) -> bool {
        if !self.input_handler_installed {
            return false;
        }
        if self.marked.is_some() {
            return true;
        }
        let m = &ev.keystroke.modifiers;
        if m.control || m.platform || m.function || (m.alt && option_is_meta) {
            return false;
        }
        ev.keystroke
            .key_char
            .as_deref()
            .is_some_and(|t| !t.is_empty() && !t.chars().any(char::is_control))
    }

    /// The UTF-16 length of the marked text.
    fn marked_len_utf16(&self) -> usize {
        self.marked
            .as_deref()
            .map_or(0, |t| t.encode_utf16().count())
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

    /// Where screen cell `row`/`col` was drawn in the last frame.
    #[doc(hidden)]
    pub fn cell_bounds(&self, row: u16, col: u16) -> Option<Bounds<Pixels>> {
        Some(self.last_frame.as_ref()?.geometry.cells(row, col, 1))
    }

    /// How many selection rectangles the last frame drew.
    #[doc(hidden)]
    pub fn painted_selection_rects(&self) -> usize {
        self.last_frame
            .as_ref()
            .map_or(0, |f| f.selection_rect_count())
    }

    /// The background the last frame was filled with.
    #[doc(hidden)]
    pub fn painted_background(&self) -> Option<gpui::Rgba> {
        Some(self.last_frame.as_ref()?.background)
    }
}

/// The input handler sees only the composition: the shell owns the line,
/// so there is no text before or after the cursor to report.
impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let marked: Vec<u16> = self.marked.as_deref()?.encode_utf16().collect();
        let start = range_utf16.start.min(marked.len());
        let end = range_utf16.end.clamp(start, marked.len());
        *adjusted_range = Some(start..end);
        Some(String::from_utf16_lossy(&marked[start..end]))
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let end = self.marked_len_utf16();
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|_| 0..self.marked_len_utf16())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.marked.take().is_some() {
            cx.notify();
        }
    }

    /// Typed text and the end of a composition go to the shell as UTF-8.
    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.marked.take().is_some() {
            cx.notify();
        }
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        self.write_input(text.as_bytes(), cx);
    }

    /// A composition in progress is only drawn; nothing reaches the shell.
    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        new_text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = (!new_text.is_empty()).then(|| new_text.to_string());
        cx.notify();
    }

    fn paste(&mut self, item: ClipboardItem, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = item.text() {
            self.request_paste(text, cx);
        }
    }

    /// The cursor cell, so the IME's candidate window opens at it.
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.last_frame.as_ref()?.cursor_cell
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TerminalView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let warning = self
            .pending_paste
            .as_deref()
            .map(|text| paste_warning(text, cx));
        div()
            .id("terminal-view")
            .debug_selector(|| "terminal-view".into())
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .child(TerminalElement::new(cx.entity()))
            .children(warning)
    }
}

/// The multi-line paste warning: a small popover at the top right with
/// the line count and Paste / Cancel.
fn paste_warning(text: &str, cx: &mut Context<TerminalView>) -> gpui::AnyElement {
    let lines = text.lines().count().max(1);
    let ui_colors = &cx.global::<crate::AppState>().theme.ui;
    let muted = crate::layout::color(ui_colors.text_muted);
    let card = ui::popover(cx)
        .debug_selector(|| "terminal-paste-warning".into())
        .w(gpui::px(280.))
        .p(gpui::px(3. * ui::GRID))
        .gap(gpui::px(2. * ui::GRID))
        .occlude()
        .child(
            div()
                .text_size(gpui::px(ui::UI_FONT_SIZE_SMALL))
                .child(format!("Paste {lines} lines into the terminal?")),
        )
        .child(
            div()
                .text_size(gpui::px(ui::UI_FONT_SIZE_SMALL))
                .text_color(muted)
                .child("Each line may run as a command."),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .justify_end()
                .gap(gpui::px(2. * ui::GRID))
                .child(
                    ui::text_button("terminal-paste-cancel", "Cancel", cx)
                        .on_click(cx.listener(|view, _, _, cx| view.resolve_paste(false, cx))),
                )
                .child(
                    ui::primary_text_button("terminal-paste-confirm", "Paste", cx)
                        .on_click(cx.listener(|view, _, _, cx| view.resolve_paste(true, cx))),
                ),
        );
    div()
        .absolute()
        .top(gpui::px(2. * ui::GRID))
        .right(gpui::px(2. * ui::GRID))
        .child(card)
        .into_any_element()
}
