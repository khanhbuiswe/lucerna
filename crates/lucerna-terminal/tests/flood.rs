//! A flood of output through a real PTY (spec §9.2, review focus 5): how
//! long the parser takes to the child's exit, and that scrollback and the
//! process's memory stay bounded by the scrollback cap. No UI drains the
//! terminal here, only the event channel, so this is the PTY and parser
//! side of the `cat` threshold. Measurements, ignored by default:
//!
//! `cargo test --release -p lucerna-terminal --test flood -- --ignored --nocapture --test-threads=1`
#![cfg(unix)]

use std::collections::HashMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::executor::block_on;
use lucerna_terminal::{Terminal, TerminalEvent, TerminalOptions};

const SCROLLBACK: usize = 10_000;
const TIMEOUT: Duration = Duration::from_secs(60);

fn opts(script: &str) -> TerminalOptions {
    TerminalOptions {
        program: PathBuf::from("/bin/sh"),
        args: vec!["-c".into(), script.into()],
        cwd: std::env::temp_dir(),
        env: HashMap::new(),
        rows: 60,
        cols: 200,
        cell_width: 8,
        cell_height: 16,
        scrollback: SCROLLBACK,
    }
}

/// Peak resident memory of this process, in MB.
fn max_rss_mb() -> f64 {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    // Bytes on macOS, kilobytes on Linux.
    let bytes = if cfg!(target_os = "macos") {
        usage.ru_maxrss as f64
    } else {
        usage.ru_maxrss as f64 * 1024.
    };
    bytes / (1024. * 1024.)
}

/// Runs `script`, draining events and taking a snapshot every few
/// wakeups as a UI would; returns the time to `Exited`.
fn flood(name: &str, script: &str) -> Duration {
    let rss_before = max_rss_mb();
    let start = Instant::now();
    let (terminal, mut rx) = Terminal::spawn(opts(script)).expect("spawn");
    let (mut wakeups, mut snapshots, mut max_history) = (0usize, 0usize, 0usize);
    let exit = block_on(async {
        while let Some(ev) = rx.next().await {
            assert!(start.elapsed() < TIMEOUT, "{name}: no exit in {TIMEOUT:?}");
            match ev {
                TerminalEvent::Wakeup => {
                    wakeups += 1;
                    if wakeups % 64 == 0 {
                        let snap = terminal.snapshot();
                        max_history = max_history.max(snap.history_len);
                        snapshots += 1;
                    }
                }
                TerminalEvent::Exited(code) => return code,
                _ => {}
            }
        }
        panic!("{name}: channel closed before Exited");
    });
    let took = start.elapsed();
    let last = terminal.snapshot();
    max_history = max_history.max(last.history_len);
    println!(
        "{name}: exit {exit:?} after {:.2} s, {wakeups} wakeups, {snapshots} snapshots, \
         scrollback max {max_history} lines (cap {SCROLLBACK}), \
         peak RSS {:.1} MB (before {rss_before:.1} MB)",
        took.as_secs_f64(),
        max_rss_mb(),
    );
    assert_eq!(exit, Some(0), "{name}");
    assert!(max_history <= SCROLLBACK, "{name}: scrollback over the cap");
    drop(terminal);
    took
}

#[test]
#[ignore = "a measurement: run in a release build with --ignored --nocapture"]
fn flood_yes_50_mb() {
    // 25 million two-byte lines: the most scrolling per byte.
    flood("yes | head -c 50 MB", "yes | head -c 50000000");
}

#[test]
#[ignore = "a measurement: run in a release build with --ignored --nocapture"]
fn flood_cat_100_mb_file() {
    // Spec §9.2: `cat` of a 100 MB file finishes in ≤ 3 s.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.txt");
    let mut file = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
    let line: String = "the quick brown fox jumps over the lazy dog 0123456789 "
        .repeat(2)
        .chars()
        .take(99)
        .collect();
    let mut written = 0usize;
    let mut i = 0usize;
    while written < 100_000_000 {
        let l = format!("{i:08} {line}\n");
        file.write_all(l.as_bytes()).unwrap();
        written += l.len();
        i += 1;
    }
    file.flush().unwrap();
    drop(file);
    let took = flood(
        "cat 100 MB file (108-byte lines)",
        &format!("cat '{}'", path.display()),
    );
    println!("threshold 3 s: {:.2} s", took.as_secs_f64());
}
