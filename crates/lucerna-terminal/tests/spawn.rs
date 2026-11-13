#![cfg(unix)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::channel::mpsc::UnboundedReceiver;
use futures::channel::oneshot;
use futures::executor::block_on;
use futures::future::{Either, select};
use lucerna_terminal::{Snapshot, Terminal, TerminalEvent, TerminalOptions, build_env};

const TIMEOUT: Duration = Duration::from_secs(5);

fn opts(program: &str, args: &[&str]) -> TerminalOptions {
    TerminalOptions {
        program: PathBuf::from(program),
        args: args.iter().map(|a| a.to_string()).collect(),
        cwd: std::env::temp_dir(),
        env: HashMap::new(),
        rows: 24,
        cols: 200,
        cell_width: 8,
        cell_height: 16,
        scrollback: 1000,
    }
}

/// Next event, or `None` once `timeout` passes or the channel closes.
fn next_event(
    rx: &mut UnboundedReceiver<TerminalEvent>,
    timeout: Duration,
) -> Option<TerminalEvent> {
    let (tx, timer) = oneshot::channel::<()>();
    thread::spawn(move || {
        thread::sleep(timeout);
        let _ = tx.send(());
    });
    block_on(async {
        match select(rx.next(), timer).await {
            Either::Left((ev, _)) => ev,
            Either::Right(_) => None,
        }
    })
}

fn wait_for(
    rx: &mut UnboundedReceiver<TerminalEvent>,
    pred: impl Fn(&TerminalEvent) -> bool,
    timeout: Duration,
) -> Option<TerminalEvent> {
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.checked_duration_since(Instant::now())?;
        let ev = next_event(rx, left)?;
        if pred(&ev) {
            return Some(ev);
        }
    }
}

fn row_text(snap: &Snapshot, row: usize) -> String {
    snap.cells[row].iter().map(|c| c.ch).collect()
}

fn screen_text(snap: &Snapshot) -> String {
    (0..snap.cells.len())
        .map(|r| row_text(snap, r))
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_exit(ev: &TerminalEvent) -> bool {
    matches!(ev, TerminalEvent::Exited(_))
}

#[test]
fn spawn_runs_shell_and_reports_exit() {
    let (term, mut rx) = Terminal::spawn(opts("/bin/sh", &["-c", "printf hi"])).unwrap();
    assert_eq!(
        wait_for(&mut rx, is_exit, TIMEOUT),
        Some(TerminalEvent::Exited(Some(0)))
    );
    let snap = term.snapshot();
    assert!(
        row_text(&snap, 0).starts_with("hi"),
        "row 0: {:?}",
        row_text(&snap, 0)
    );
}

#[test]
fn spawn_nonzero_exit_code() {
    let (_term, mut rx) = Terminal::spawn(opts("/bin/sh", &["-c", "exit 3"])).unwrap();
    assert_eq!(
        wait_for(&mut rx, is_exit, TIMEOUT),
        Some(TerminalEvent::Exited(Some(3)))
    );
}

#[test]
fn spawn_missing_shell_errors() {
    match Terminal::spawn(opts("/nonexistent/sh", &[])) {
        Err(_) => {}
        Ok((_term, mut rx)) => {
            let ev = wait_for(
                &mut rx,
                |ev| !matches!(ev, TerminalEvent::Wakeup | TerminalEvent::Title(_)),
                TIMEOUT,
            );
            match ev {
                Some(TerminalEvent::Exited(Some(n))) => assert_ne!(n, 0),
                other => panic!("expected a non-zero exit, got {other:?}"),
            }
        }
    }
}

#[test]
fn resize_changes_stty_size() {
    let (term, mut rx) = Terminal::spawn(opts("/bin/sh", &["-c", "sleep 0.2; stty size"])).unwrap();
    term.resize(30, 100, 8, 16);
    assert_eq!(
        wait_for(&mut rx, is_exit, TIMEOUT),
        Some(TerminalEvent::Exited(Some(0)))
    );
    let snap = term.snapshot();
    assert_eq!((snap.rows, snap.cols), (30, 100));
    assert!(
        screen_text(&snap).contains("30 100"),
        "screen: {}",
        screen_text(&snap)
    );
}

#[test]
fn resize_clamps_to_one_cell() {
    let term = Terminal::fake(24, 80, 100);
    term.resize(0, 0, 8, 16);
    let snap = term.snapshot();
    assert_eq!((snap.rows, snap.cols), (1, 1));
    assert_eq!(snap.cells.len(), 1);
    assert_eq!(snap.cells[0].len(), 1);
}

#[test]
fn cwd_and_env_reach_child() {
    let dir = tempfile::tempdir().unwrap();
    let mut o = opts("/bin/sh", &["-c", "pwd; echo $TERM_PROGRAM"]);
    o.cwd = dir.path().to_path_buf();
    o.env = build_env(
        std::env::vars(),
        &HashMap::new(),
        &HashMap::new(),
        "0.1.0",
        true,
    )
    .into_iter()
    .map(|(k, v)| (k, Some(v)))
    .collect();
    let (term, mut rx) = Terminal::spawn(o).unwrap();
    assert_eq!(
        wait_for(&mut rx, is_exit, TIMEOUT),
        Some(TerminalEvent::Exited(Some(0)))
    );
    let snap = term.snapshot();
    let pwd = row_text(&snap, 0);
    let pwd = Path::new(pwd.trim_end());
    let canonical = dir.path().canonicalize().unwrap();
    assert!(pwd == dir.path() || pwd == canonical, "pwd: {pwd:?}");
    assert_eq!(row_text(&snap, 1).trim_end(), "lucerna");
}

#[test]
fn env_none_removes_inherited_variable() {
    // SAFETY: set before any child is spawned by this test; the name is unique to it.
    unsafe { std::env::set_var("LUCERNA_TERMINAL_TEST_REMOVE", "present") };
    let mut o = opts(
        "/bin/sh",
        &["-c", "echo \"[${LUCERNA_TERMINAL_TEST_REMOVE-unset}]\""],
    );
    o.env.insert("LUCERNA_TERMINAL_TEST_REMOVE".into(), None);
    let (term, mut rx) = Terminal::spawn(o).unwrap();
    assert_eq!(
        wait_for(&mut rx, is_exit, TIMEOUT),
        Some(TerminalEvent::Exited(Some(0)))
    );
    assert_eq!(row_text(&term.snapshot(), 0).trim_end(), "[unset]");
}

#[test]
fn drop_hangs_up_child() {
    let (term, _rx) =
        Terminal::spawn(opts("/bin/sh", &["-c", "trap \"exit 7\" HUP; sleep 30"])).unwrap();
    let pid = term.child_pid().expect("a spawned terminal has a child") as libc::pid_t;
    // Let the shell install its trap.
    thread::sleep(Duration::from_millis(200));
    drop(term);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        // SAFETY: signal 0 only checks that the process exists.
        if unsafe { libc::kill(pid, 0) } != 0 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "child {pid} still alive 2 s after drop"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn scrollback_is_capped() {
    let term = Terminal::fake(5, 10, 200_000);
    let bytes = "x\r\n".repeat(150_000);
    term.feed(bytes.as_bytes());
    let snap = term.snapshot();
    assert_eq!(snap.history_len, 100_000);
}

#[test]
fn input_reaches_the_child() {
    let (term, mut rx) =
        Terminal::spawn(opts("/bin/sh", &["-c", "read line; echo \"got $line\""])).unwrap();
    term.input(b"abc\r");
    assert_eq!(
        wait_for(&mut rx, is_exit, TIMEOUT),
        Some(TerminalEvent::Exited(Some(0)))
    );
    assert!(screen_text(&term.snapshot()).contains("got abc"));
}
