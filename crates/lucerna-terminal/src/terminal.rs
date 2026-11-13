//! One terminal session: a PTY, alacritty's parser on its own thread, and the
//! calls the UI makes.

use std::borrow::Cow;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg, State};
use alacritty_terminal::grid::{Dimensions, Scroll as AlacScroll};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config, Osc52, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{ClearMode, Handler, Rgb};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};

use crate::events::TerminalEvent;
use crate::options::{MAX_SCROLLBACK, TerminalOptions};
use crate::snapshot::{self, Scroll, Snapshot};

/// A running terminal. Dropping it hangs up the PTY, which sends SIGHUP to
/// the shell and its foreground job.
pub struct Terminal {
    term: Arc<FairMutex<Term<Listener>>>,
    shared: Arc<Shared>,
    /// UI operations applied under the lock `snapshot` takes anyway.
    pending: Mutex<Vec<Op>>,
    child: Option<Child>,
    #[cfg(feature = "test-support")]
    parser: Mutex<alacritty_terminal::vte::ansi::Processor>,
    /// A fake terminal's event channel: `feed` sends a `Wakeup` on it.
    #[cfg(feature = "test-support")]
    fake_events: Option<UnboundedSender<TerminalEvent>>,
}

enum Op {
    Resize { rows: u16, cols: u16 },
    Scroll(Scroll),
    Clear,
}

struct Child {
    /// Unknown only if Windows could not report it.
    pid: Option<u32>,
    #[cfg(unix)]
    fd: std::os::fd::RawFd,
}

/// Where bytes for the child go.
enum Sink {
    Pty(EventLoopSender),
    #[cfg(feature = "test-support")]
    Fake(Mutex<Vec<u8>>),
}

/// State the UI side and the listener (on the I/O thread) both touch.
struct Shared {
    /// Set once the event loop exists; the listener is built before it.
    sink: OnceLock<Sink>,
    /// Last size given to `resize`, also used to answer size queries.
    size: Mutex<WindowSize>,
    /// The child has exited and may have been reaped: its pid is stale.
    exited: AtomicBool,
    /// The I/O thread still owns an open PTY. Cleared, under this lock,
    /// before the PTY is dropped; `hang_up` holds the lock while it uses the
    /// fd, so it never touches a closed or reused one.
    pty_open: Mutex<bool>,
    /// Exit code from `ChildExit`, reported once the final output is parsed.
    exit_code: Mutex<Option<i32>>,
}

impl Shared {
    fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        let bytes = bytes.into();
        if bytes.is_empty() {
            return;
        }
        match self.sink.get() {
            Some(Sink::Pty(sender)) => {
                let _ = sender.send(Msg::Input(bytes));
            }
            #[cfg(feature = "test-support")]
            Some(Sink::Fake(buf)) => lock(buf).extend_from_slice(&bytes),
            None => {}
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Turns alacritty's events into [`TerminalEvent`]s and answers the queries
/// alacritty leaves to the embedder. Runs on the I/O thread with the terminal
/// locked, so it must never lock the terminal or block.
struct Listener {
    tx: UnboundedSender<TerminalEvent>,
    shared: Arc<Shared>,
}

impl Listener {
    fn send(&self, ev: TerminalEvent) {
        let _ = self.tx.unbounded_send(ev);
    }
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        match event {
            Event::Wakeup => self.send(TerminalEvent::Wakeup),
            Event::Title(title) => self.send(TerminalEvent::Title(Some(title))),
            Event::ResetTitle => self.send(TerminalEvent::Title(None)),
            Event::Bell => self.send(TerminalEvent::Bell),
            Event::ClipboardStore(_, text) => self.send(TerminalEvent::ClipboardStore(text)),
            Event::PtyWrite(text) => self.shared.write(text.into_bytes()),
            Event::TextAreaSizeRequest(format) => {
                let size = *lock(&self.shared.size);
                self.shared.write(format(size).into_bytes());
            }
            Event::ColorRequest(index, format) => {
                if let Some(rgb) = default_color(index) {
                    self.shared.write(format(rgb).into_bytes());
                }
            }
            Event::ChildExit(status) => {
                self.shared.exited.store(true, Ordering::SeqCst);
                *lock(&self.shared.exit_code) = status.code();
            }
            // Sent after the last output is parsed, so the UI sees it all.
            Event::Exit => {
                self.shared.exited.store(true, Ordering::SeqCst);
                let code = lock(&self.shared.exit_code).take();
                self.send(TerminalEvent::Exited(code));
            }
            // OSC 52 reads are refused: apps may not read the clipboard (spec §5.1).
            Event::ClipboardLoad(..) | Event::MouseCursorDirty | Event::CursorBlinkingChange => {}
        }
    }
}

/// Grid size for alacritty.
struct GridSize {
    rows: usize,
    cols: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.cols
    }
}

fn window_size(rows: u16, cols: u16, cell_width: u16, cell_height: u16) -> WindowSize {
    WindowSize {
        num_lines: rows.max(1),
        num_cols: cols.max(1),
        cell_width,
        cell_height,
    }
}

fn same_size(a: &WindowSize, b: &WindowSize) -> bool {
    (a.num_lines, a.num_cols, a.cell_width, a.cell_height)
        == (b.num_lines, b.num_cols, b.cell_width, b.cell_height)
}

/// Builds the shared state, the listener and the terminal grid.
fn new_term(
    size: WindowSize,
    scrollback: usize,
    tx: UnboundedSender<TerminalEvent>,
) -> (Arc<FairMutex<Term<Listener>>>, Arc<Shared>) {
    let shared = Arc::new(Shared {
        sink: OnceLock::new(),
        size: Mutex::new(size),
        exited: AtomicBool::new(false),
        pty_open: Mutex::new(true),
        exit_code: Mutex::new(None),
    });
    let config = Config {
        scrolling_history: scrollback.min(MAX_SCROLLBACK),
        osc52: Osc52::OnlyCopy,
        ..Config::default()
    };
    let grid = GridSize {
        rows: size.num_lines as usize,
        cols: size.num_cols as usize,
    };
    let listener = Listener {
        tx,
        shared: shared.clone(),
    };
    let term = Arc::new(FairMutex::new(Term::new(config, &grid, listener)));
    (term, shared)
}

/// The command line to run. Unix removes variables with `env -u`, because
/// alacritty can only add to the inherited environment.
fn shell_command(opts: &TerminalOptions) -> tty::Shell {
    let program = opts.program.to_string_lossy().into_owned();
    #[cfg(unix)]
    {
        let mut removed: Vec<&String> = opts
            .env
            .iter()
            .filter(|(_, v)| v.is_none())
            .map(|(k, _)| k)
            .collect();
        if !removed.is_empty() {
            removed.sort();
            let mut args = Vec::with_capacity(removed.len() * 2 + 1 + opts.args.len());
            for key in removed {
                args.push("-u".to_string());
                args.push(key.clone());
            }
            args.push(program);
            args.extend(opts.args.iter().cloned());
            return tty::Shell::new("/usr/bin/env".to_string(), args);
        }
    }
    tty::Shell::new(program, opts.args.clone())
}

impl Terminal {
    /// Starts `opts.program` on a new PTY. Events arrive on the returned
    /// channel, which closes after the `Terminal` is dropped and its I/O
    /// thread ends. A program that cannot be started returns `Err`.
    pub fn spawn(
        opts: TerminalOptions,
    ) -> io::Result<(Terminal, UnboundedReceiver<TerminalEvent>)> {
        let size = window_size(opts.rows, opts.cols, opts.cell_width, opts.cell_height);
        let tty_options = tty::Options {
            shell: Some(shell_command(&opts)),
            working_directory: Some(opts.cwd.clone()),
            drain_on_exit: true,
            env: opts
                .env
                .iter()
                .filter_map(|(k, v)| v.as_ref().map(|v| (k.clone(), v.clone())))
                .collect(),
            #[cfg(windows)]
            escape_args: true,
        };
        let pty = tty::new(&tty_options, size, 0)?;
        #[cfg(unix)]
        let pid = Some(pty.child().id());
        #[cfg(windows)]
        let pid = pty.child_watcher().pid().map(std::num::NonZeroU32::get);
        #[cfg(unix)]
        let fd = std::os::fd::AsRawFd::as_raw_fd(pty.file());

        let (tx, rx) = unbounded();
        let (term, shared) = new_term(size, opts.scrollback, tx.clone());
        let event_proxy = Listener {
            tx,
            shared: shared.clone(),
        };
        let event_loop = EventLoop::new(term.clone(), event_proxy, pty, true, false)?;
        let _ = shared.sink.set(Sink::Pty(event_loop.channel()));
        let io_thread = event_loop.spawn();
        spawn_reaper(io_thread, shared.clone())?;

        let terminal = Terminal {
            term,
            shared,
            pending: Mutex::new(Vec::new()),
            child: Some(Child {
                pid,
                #[cfg(unix)]
                fd,
            }),
            #[cfg(feature = "test-support")]
            parser: Mutex::default(),
            #[cfg(feature = "test-support")]
            fake_events: None,
        };
        Ok((terminal, rx))
    }

    /// A terminal with no PTY: `feed` stands in for the child's output and
    /// `written` returns what would have gone to it.
    #[cfg(feature = "test-support")]
    pub fn fake(rows: u16, cols: u16, scrollback: usize) -> Terminal {
        Self::fake_with_events(rows, cols, scrollback).0
    }

    /// [`Terminal::fake`] with its event channel, as `spawn` returns it:
    /// `feed` sends a `Wakeup` on the calling thread, and titles set by fed
    /// bytes arrive too.
    #[cfg(feature = "test-support")]
    pub fn fake_with_events(
        rows: u16,
        cols: u16,
        scrollback: usize,
    ) -> (Terminal, UnboundedReceiver<TerminalEvent>) {
        let (tx, rx) = unbounded();
        let (term, shared) = new_term(window_size(rows, cols, 8, 16), scrollback, tx.clone());
        let _ = shared.sink.set(Sink::Fake(Mutex::default()));
        let terminal = Terminal {
            term,
            shared,
            pending: Mutex::new(Vec::new()),
            child: None,
            parser: Mutex::default(),
            fake_events: Some(tx),
        };
        (terminal, rx)
    }

    /// Parses `bytes` as if the child wrote them.
    #[cfg(feature = "test-support")]
    pub fn feed(&self, bytes: &[u8]) {
        {
            let mut term = self.term.lock();
            self.apply_pending(&mut term);
            lock(&self.parser).advance(&mut *term, bytes);
        }
        if let Some(tx) = &self.fake_events {
            let _ = tx.unbounded_send(TerminalEvent::Wakeup);
        }
    }

    /// The PTY has been closed and the child reaped (or never existed).
    #[cfg(feature = "test-support")]
    pub fn pty_closed(&self) -> bool {
        !*lock(&self.shared.pty_open) || self.child.is_none()
    }

    /// Everything sent to the PTY so far (fake terminals only).
    #[cfg(feature = "test-support")]
    pub fn written(&self) -> Vec<u8> {
        match self.shared.sink.get() {
            Some(Sink::Fake(buf)) => lock(buf).clone(),
            _ => Vec::new(),
        }
    }

    /// Sends raw bytes (already encoded keys) to the child.
    pub fn input(&self, bytes: &[u8]) {
        self.shared.write(bytes.to_vec());
    }

    /// Pastes text: bracketed when the app asked for it, with ESC removed so
    /// the text cannot end the bracket early; otherwise newlines become CR.
    pub fn paste(&self, text: &str) {
        let bracketed = self.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        let bytes = if bracketed {
            let mut out = String::with_capacity(text.len() + 12);
            out.push_str("\x1b[200~");
            out.extend(text.chars().filter(|&c| c != '\x1b'));
            out.push_str("\x1b[201~");
            out
        } else {
            text.replace("\r\n", "\r").replace('\n', "\r")
        };
        self.shared.write(bytes.into_bytes());
    }

    /// Resizes to at least one cell. Only a change reaches the PTY; the grid
    /// follows at the next `snapshot`.
    pub fn resize(&self, rows: u16, cols: u16, cell_w: u16, cell_h: u16) {
        let new = window_size(rows, cols, cell_w, cell_h);
        {
            let mut size = lock(&self.shared.size);
            if same_size(&size, &new) {
                return;
            }
            *size = new;
        }
        if let Some(Sink::Pty(sender)) = self.shared.sink.get() {
            let _ = sender.send(Msg::Resize(new));
        }
        let mut pending = lock(&self.pending);
        pending.retain(|op| !matches!(op, Op::Resize { .. }));
        pending.push(Op::Resize {
            rows: new.num_lines,
            cols: new.num_cols,
        });
    }

    pub fn scroll(&self, s: Scroll) {
        lock(&self.pending).push(Op::Scroll(s));
    }

    /// Clears the scrollback and the screen but keeps the cursor line, which
    /// moves to the top (VS Code's "Clear").
    pub fn clear(&self) {
        lock(&self.pending).push(Op::Clear);
    }

    /// The terminal's modes (alt screen, app cursor…), without copying the
    /// screen.
    pub fn mode(&self) -> crate::snapshot::Modes {
        snapshot::modes(*self.term.lock().mode())
    }

    /// Whether some text is selected.
    pub fn has_selection(&self) -> bool {
        self.term
            .lock()
            .selection
            .as_ref()
            .is_some_and(|s| !s.is_empty())
    }

    /// Copies the visible screen under one short lock, applying queued UI
    /// operations first.
    pub fn snapshot(&self) -> Snapshot {
        let mut term = self.term.lock_unfair();
        self.apply_pending(&mut term);
        snapshot::take(&term)
    }

    /// The PTY's controller side, while the terminal has one.
    #[cfg(unix)]
    pub fn pty_fd(&self) -> Option<std::os::fd::RawFd> {
        let child = self.child.as_ref()?;
        let open = !self.shared.exited.load(Ordering::SeqCst) && *lock(&self.shared.pty_open);
        open.then_some(child.fd)
    }

    pub fn child_pid(&self) -> Option<u32> {
        self.child.as_ref().and_then(|c| c.pid)
    }

    fn apply_pending(&self, term: &mut Term<Listener>) {
        let ops = std::mem::take(&mut *lock(&self.pending));
        for op in ops {
            match op {
                Op::Resize { rows, cols } => term.resize(GridSize {
                    rows: rows as usize,
                    cols: cols as usize,
                }),
                Op::Scroll(s) => term.scroll_display(match s {
                    Scroll::Lines(n) => AlacScroll::Delta(n),
                    Scroll::PageUp => AlacScroll::PageUp,
                    Scroll::PageDown => AlacScroll::PageDown,
                    Scroll::Top => AlacScroll::Top,
                    Scroll::Bottom => AlacScroll::Bottom,
                }),
                Op::Clear => clear_keeping_cursor_line(term),
            }
        }
    }
}

fn clear_keeping_cursor_line(term: &mut Term<Listener>) {
    term.scroll_display(AlacScroll::Bottom);
    let cursor = term.grid().cursor.point;
    // Push the lines above the cursor into history, then drop the history.
    term.scroll_up(cursor.line.0.max(0) as usize);
    term.goto(0, cursor.column.0);
    term.clear_screen(ClearMode::Saved);
}

/// How long a hung-up child gets to exit before the I/O thread stops reading.
const HANGUP_GRACE: Duration = Duration::from_secs(1);

impl Drop for Terminal {
    fn drop(&mut self) {
        let Some(child) = self.child.take() else {
            return;
        };
        #[cfg(unix)]
        {
            let open = lock(&self.shared.pty_open);
            if *open && !self.shared.exited.load(Ordering::SeqCst) {
                hang_up(child.pid, child.fd);
            }
        }
        #[cfg(not(unix))]
        let _ = child;
        let Some(Sink::Pty(sender)) = self.shared.sink.get() else {
            return;
        };
        if !*lock(&self.shared.pty_open) {
            return;
        }
        // The I/O thread keeps reading meanwhile: an exiting child can block
        // until its last output is read, and then the loop ends by itself and
        // the reaper closes the PTY. Stop it only if the child lingers.
        let sender = sender.clone();
        let shared = self.shared.clone();
        let _ = thread::Builder::new()
            .name("terminal hangup".into())
            .spawn(move || {
                let deadline = Instant::now() + HANGUP_GRACE;
                while *lock(&shared.pty_open) && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(10));
                }
                if *lock(&shared.pty_open) {
                    let _ = sender.send(Msg::Shutdown);
                }
            });
    }
}

/// Joins the I/O thread as soon as its loop ends, whether the child exited
/// or `Drop` asked it to stop, and drops the `EventLoop`. That closes the PTY
/// and lets alacritty's `Pty::drop` signal and wait for the child right
/// after it was reaped, instead of hours later when the pid may belong to
/// another process.
fn spawn_reaper(
    io_thread: JoinHandle<(EventLoop<tty::Pty, Listener>, State)>,
    shared: Arc<Shared>,
) -> io::Result<()> {
    thread::Builder::new()
        .name("terminal reaper".into())
        .spawn(move || {
            let result = io_thread.join();
            *lock(&shared.pty_open) = false;
            // May block in `wait()` for a child that ignores SIGHUP, so the
            // lock is not held here.
            drop(result);
        })
        .map(drop)
}

/// Sends SIGHUP to the shell's process group and to the terminal's foreground
/// job, as closing a terminal window does. A shell that traps SIGHUP while
/// waiting on a foreground command would otherwise linger until it returns.
#[cfg(unix)]
fn hang_up(pid: Option<u32>, fd: std::os::fd::RawFd) {
    // SAFETY: plain syscalls on a pid we spawned and have not reaped, and a
    // PTY fd kept open while the caller holds `pty_open` (both checked there).
    unsafe {
        let foreground = libc::tcgetpgrp(fd);
        if foreground > 0 {
            libc::kill(-foreground, libc::SIGHUP);
        }
        // The child called setsid, so its pid is its process group.
        if let Some(pid) = pid {
            libc::kill(-(pid as libc::pid_t), libc::SIGHUP);
        }
    }
}

/// Answers OSC 4/10/11/12 colour queries. The theme's palette is not known
/// here, so these are xterm's 256 colours on a dark background.
fn default_color(index: usize) -> Option<Rgb> {
    const ANSI: [(u8, u8, u8); 16] = [
        (0x00, 0x00, 0x00),
        (0xcd, 0x31, 0x31),
        (0x0d, 0xbc, 0x79),
        (0xe5, 0xe5, 0x10),
        (0x24, 0x72, 0xc8),
        (0xbc, 0x3f, 0xbc),
        (0x11, 0xa8, 0xcd),
        (0xe5, 0xe5, 0xe5),
        (0x66, 0x66, 0x66),
        (0xf1, 0x4c, 0x4c),
        (0x23, 0xd1, 0x8b),
        (0xf5, 0xf5, 0x43),
        (0x3b, 0x8e, 0xea),
        (0xd6, 0x70, 0xd6),
        (0x29, 0xb8, 0xdb),
        (0xe5, 0xe5, 0xe5),
    ];
    const FOREGROUND: (u8, u8, u8) = (0xcc, 0xcc, 0xcc);
    const BACKGROUND: (u8, u8, u8) = (0x1e, 0x1e, 0x1e);
    let dim = |(r, g, b): (u8, u8, u8)| {
        let d = |c: u8| (c as u16 * 2 / 3) as u8;
        (d(r), d(g), d(b))
    };
    let (r, g, b) = match index {
        0..=15 => ANSI[index],
        16..=231 => {
            let i = index - 16;
            let level = |v: usize| if v == 0 { 0 } else { (55 + v * 40) as u8 };
            (level(i / 36), level(i / 6 % 6), level(i % 6))
        }
        232..=255 => {
            let v = (8 + (index - 232) * 10) as u8;
            (v, v, v)
        }
        256 | 258 | 267 => FOREGROUND,
        257 => BACKGROUND,
        259..=266 => dim(ANSI[index - 259]),
        268 => dim(FOREGROUND),
        _ => return None,
    };
    Some(Rgb { r, g, b })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_palette_covers_xterm_indices() {
        assert_eq!(default_color(16), Some(Rgb { r: 0, g: 0, b: 0 }));
        assert_eq!(
            default_color(231),
            Some(Rgb {
                r: 255,
                g: 255,
                b: 255
            })
        );
        assert_eq!(default_color(232), Some(Rgb { r: 8, g: 8, b: 8 }));
        assert_eq!(
            default_color(255),
            Some(Rgb {
                r: 238,
                g: 238,
                b: 238
            })
        );
        assert!(default_color(257).is_some());
        assert_eq!(default_color(269), None);
    }

    fn listener() -> (Listener, UnboundedReceiver<TerminalEvent>) {
        let (tx, rx) = unbounded();
        let (_, shared) = new_term(window_size(4, 10, 8, 16), 10, tx.clone());
        (Listener { tx, shared }, rx)
    }

    fn drain(rx: &mut UnboundedReceiver<TerminalEvent>) -> Vec<TerminalEvent> {
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    }

    #[test]
    fn listener_maps_title_bell_and_clipboard() {
        let (l, mut rx) = listener();
        l.send_event(Event::Title("vim".into()));
        l.send_event(Event::ResetTitle);
        l.send_event(Event::Bell);
        l.send_event(Event::Wakeup);
        l.send_event(Event::MouseCursorDirty);
        l.send_event(Event::ClipboardStore(
            alacritty_terminal::term::ClipboardType::Clipboard,
            "copied".into(),
        ));
        assert_eq!(
            drain(&mut rx),
            vec![
                TerminalEvent::Title(Some("vim".into())),
                TerminalEvent::Title(None),
                TerminalEvent::Bell,
                TerminalEvent::Wakeup,
                TerminalEvent::ClipboardStore("copied".into()),
            ]
        );
    }

    #[test]
    fn exit_without_child_status_reports_none() {
        let (l, mut rx) = listener();
        l.send_event(Event::Exit);
        assert_eq!(drain(&mut rx), vec![TerminalEvent::Exited(None)]);
    }

    #[cfg(unix)]
    #[test]
    fn child_exit_code_is_reported_at_exit() {
        use std::os::unix::process::ExitStatusExt;
        let (l, mut rx) = listener();
        l.send_event(Event::ChildExit(std::process::ExitStatus::from_raw(5 << 8)));
        assert_eq!(drain(&mut rx), vec![], "held until the output is parsed");
        l.send_event(Event::Exit);
        assert_eq!(drain(&mut rx), vec![TerminalEvent::Exited(Some(5))]);
    }

    #[cfg(unix)]
    #[test]
    fn killed_child_reports_none() {
        use std::os::unix::process::ExitStatusExt;
        let (l, mut rx) = listener();
        l.send_event(Event::ChildExit(std::process::ExitStatus::from_raw(
            libc::SIGKILL,
        )));
        l.send_event(Event::Exit);
        assert_eq!(drain(&mut rx), vec![TerminalEvent::Exited(None)]);
    }
}
