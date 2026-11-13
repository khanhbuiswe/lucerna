//! One terminal's view: holds a `lucerna_terminal::Terminal`, turns its
//! events into redraws (batched over 4 ms, spec §5.1) and draws it with
//! [`TerminalElement`](super::element::TerminalElement).

use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::channel::mpsc::UnboundedReceiver;
use futures::future::{Either, select};
use gpui::{
    App, AppContext, Bounds, ClipboardItem, Context, Entity, EntityInputHandler, EventEmitter,
    FocusHandle, Focusable, Global, IntoElement, KeyDownEvent, ParentElement, Pixels, Point,
    Render, SharedString, Styled, Subscription, Task, UTF16Selection, Window, div, prelude::*,
};
use lucerna_terminal::{
    Batch, ProcessInfo, Scroll, Terminal, TerminalEvent, TerminalOptions, TitleState, display_name,
    foreground_process,
};

use super::element::{Frame, TerminalElement};
use super::links::{LinkState, LinkTarget};
use super::mouse::MouseState;
use super::settings::{bell_enabled, cursor_style, mouse_settings, scrollback, spawn_plan};
use crate::ui;

/// How long events are gathered after the first one before the view
/// handles them together (spec §5.1).
const BATCH_WINDOW: Duration = Duration::from_millis(4);
/// The foreground process is asked for at most this often (spec §6.6).
const PROCESS_POLL: Duration = Duration::from_millis(500);

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

/// Starts a terminal for the workspace at `root` with `spawner`, as the
/// settings say (their Workspace layer skipped when `restricted`), and
/// makes its view; the error text when it could not start. Links resolve
/// against `root` last.
pub(crate) fn spawn_view(
    spawner: &TerminalSpawner,
    root: &Path,
    restricted: bool,
    window: &mut Window,
    cx: &mut App,
) -> Result<Entity<TerminalView>, SharedString> {
    let plan = spawn_plan(root, restricted, cx);
    let cwd = plan.options.cwd.clone();
    match spawner(plan.options) {
        Ok((terminal, events)) => Ok(cx.new(|cx| {
            let mut view = TerminalView::new(terminal, events, window, cx);
            view.cwd_start = cwd;
            view.root = root.to_path_buf();
            if let Some(warning) = plan.warning {
                view.write_notice(&warning, true, cx);
            }
            view
        })),
        Err(error) => Err(format!("The terminal could not start: {error}").into()),
    }
}

/// The line written when the process exits with a non-zero code, or with
/// none (killed by a signal), as spec §6.1 words it.
fn exit_line(code: Option<i32>) -> String {
    match code {
        Some(code) => format!("[Tiến trình kết thúc với mã {code}]"),
        None => "[Tiến trình kết thúc]".to_string(),
    }
}

/// What a terminal view tells its tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalViewEvent {
    /// The tab's title changed to this.
    TitleChanged(SharedString),
    /// A ⌘+clicked file or folder link, for the workbench to open.
    OpenLink(LinkTarget),
    /// The process exited with this code (`None`: a signal killed it).
    Exited(Option<i32>),
    /// The bell rang, with `terminal.integrated.enableBell` on.
    Bell,
    /// The foreground process or its folder changed (the tab's tooltip).
    ProcessChanged,
}

pub struct TerminalView {
    /// Shared with background threads that ask for its foreground process.
    pub(super) terminal: Arc<Terminal>,
    /// The folder the terminal started in, and the workspace root: where
    /// relative link paths are looked for after the current cwd.
    pub(super) cwd_start: PathBuf,
    pub(super) root: PathBuf,
    pub(super) links: LinkState,
    pub(super) focus: FocusHandle,
    title_state: TitleState,
    /// The last OSC title, to tell a new title from alacritty repeating
    /// the current one when its config changes.
    osc_title: Option<String>,
    /// The title last told to the tab.
    title: String,
    /// The foreground process at the last poll.
    foreground: Option<ProcessInfo>,
    /// The poll waiting for its turn or running, and whether a wakeup
    /// came while it ran.
    poll: Option<Task<()>>,
    poll_again: bool,
    last_poll: Option<Instant>,
    /// Bells rung since the view was made.
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
            cx.observe_global::<crate::AppState>(|view, cx| {
                view.apply_settings(cx);
                cx.notify();
            }),
        ];
        // The first hover must not spend its 1 ms compiling the patterns.
        cx.background_spawn(async { lucerna_terminal::warm_link_patterns() })
            .detach();
        let view = TerminalView {
            terminal: Arc::new(terminal),
            cwd_start: PathBuf::new(),
            root: PathBuf::new(),
            links: LinkState::default(),
            focus,
            title_state: TitleState::default(),
            osc_title: None,
            title: String::new(),
            foreground: None,
            poll: None,
            poll_again: false,
            last_poll: None,
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
        };
        view.apply_settings(cx);
        view
    }

    /// Hands the settings an open terminal follows to its terminal
    /// (spec §8.2); the font and colours are read at every layout.
    fn apply_settings(&self, cx: &App) {
        self.terminal.set_scrollback(scrollback(cx));
        self.terminal.set_default_cursor_shape(cursor_style(cx));
        self.terminal
            .set_word_separators(&mouse_settings(cx).word_separators);
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
                    if title != self.osc_title {
                        self.osc_title = title.clone();
                        // Which process set it is learnt at the next poll
                        // (`TitleState::observe`), which this asks for.
                        self.title_state.on_osc_title(title, None);
                        redraw = true;
                    }
                    self.update_title(cx);
                }
                TerminalEvent::Bell => {
                    self.bells += 1;
                    if bell_enabled(cx) {
                        cx.emit(TerminalViewEvent::Bell);
                    }
                }
                TerminalEvent::Exited(code) => {
                    if self.exited.is_none() {
                        self.exited = Some(code);
                        self.poll = None;
                        if code != Some(0) {
                            self.write_notice(&exit_line(code), false, cx);
                            // The cursor of a finished process means nothing.
                            self.terminal.write_output(b"\x1b[?25l");
                        }
                        cx.emit(TerminalViewEvent::Exited(code));
                    }
                    redraw = true;
                }
                // OSC 52 writes (reads are refused in the terminal crate).
                TerminalEvent::ClipboardStore(text) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }
        }
        if redraw {
            self.request_poll(cx);
            cx.notify();
        }
    }

    /// Writes `text` into the grid on a line of its own, in the theme's
    /// `text_muted` colour, followed by a new line when `newline`. For
    /// Lucerna's own lines: the exit line and the `cwd` warning.
    pub(super) fn write_notice(&self, text: &str, newline: bool, cx: &App) {
        let muted = cx.global::<crate::AppState>().theme.ui.text_muted;
        let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
        let at_line_start = self.terminal.snapshot().cursor.is_none_or(|c| c.col == 0);
        let mut out = String::new();
        if !at_line_start {
            out.push_str("\r\n");
        }
        out.push_str(&format!(
            "\x1b[38;2;{};{};{}m{text}\x1b[0m",
            byte(muted.r),
            byte(muted.g),
            byte(muted.b)
        ));
        if newline {
            out.push_str("\r\n");
        }
        self.terminal.write_output(out.as_bytes());
    }

    /// Asks for the foreground process after a wakeup: at once when the
    /// last poll is [`PROCESS_POLL`] old, else when it will be; one poll
    /// at a time, on a background thread (spec §6.6).
    fn request_poll(&mut self, cx: &mut Context<Self>) {
        if self.exited.is_some() {
            return;
        }
        if self.poll.is_some() {
            self.poll_again = true;
            return;
        }
        let executor = cx.background_executor().clone();
        let now = executor.now();
        let wait = self.last_poll.map_or(Duration::ZERO, |at| {
            PROCESS_POLL.saturating_sub(now.saturating_duration_since(at))
        });
        let terminal = self.terminal.clone();
        self.poll = Some(cx.spawn(async move |this, cx| {
            if !wait.is_zero() {
                executor.timer(wait).await;
            }
            let info = executor
                .spawn(async move { foreground_process(&terminal) })
                .await;
            this.update(cx, |view, cx| view.polled(info, cx)).ok();
        }));
    }

    fn polled(&mut self, info: Option<ProcessInfo>, cx: &mut Context<Self>) {
        self.poll = None;
        let now = cx.background_executor().now();
        self.last_poll = Some(now);
        self.foreground_seen(info, now, cx);
        if std::mem::take(&mut self.poll_again) {
            self.request_poll(cx);
        }
    }

    /// What a poll found in the foreground at `now`.
    fn foreground_seen(&mut self, info: Option<ProcessInfo>, now: Instant, cx: &mut Context<Self>) {
        let pid = info.as_ref().map(|i| i.pid);
        // Before `tab_title`: a title set while the pid was unknown belongs
        // to the process seen now (`TitleState`'s contract).
        self.title_state.observe(pid);
        // Another process: the same title set again is new, from it.
        if pid != self.foreground.as_ref().map(|f| f.pid) {
            self.osc_title = None;
        }
        if let Some(cwd) = info.as_ref().and_then(|i| i.cwd.clone()) {
            self.links.note_polled_cwd(cwd, now);
        }
        if self.foreground != info {
            self.foreground = info;
            cx.emit(TerminalViewEvent::ProcessChanged);
        }
        self.update_title(cx);
    }

    /// Tells the tab its title when it changed.
    fn update_title(&mut self, cx: &mut Context<Self>) {
        let title = self.title_state.tab_title(self.foreground.as_ref());
        if title != self.title {
            self.title = title.clone();
            cx.emit(TerminalViewEvent::TitleChanged(title.into()));
        }
    }

    /// The tab's tooltip: the foreground process's name and folder.
    pub fn process_tooltip(&self) -> Option<String> {
        let info = self.foreground.as_ref()?;
        let name = display_name(info);
        Some(match &info.cwd {
            Some(cwd) => format!("{name}\n{}", cwd.display()),
            None => name,
        })
    }

    /// Something other than the shell runs in the foreground (as of the
    /// last poll), for `confirmOnExit: hasChildProcesses`.
    pub fn has_child_process(&self) -> bool {
        self.exited.is_none()
            && self
                .foreground
                .as_ref()
                .is_some_and(|f| Some(f.pid) != self.terminal.child_pid())
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

    /// Stands in for a poll of the foreground process, for tests.
    #[doc(hidden)]
    pub fn set_foreground_process(&mut self, info: Option<ProcessInfo>, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        self.foreground_seen(info, now, cx);
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
            .children(self.link_overlay(cx))
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
