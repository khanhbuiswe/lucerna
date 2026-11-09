//! Child-process runner with timeout, cancellation and logging.

use std::fmt;
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

/// Shared cancellation flag.
#[derive(Clone, Default, Debug)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Debug)]
pub enum ProcessError {
    /// The program could not be started (not installed).
    NotFound(String),
    Failed {
        cmd: String,
        status: i32,
        stderr: String,
    },
    TimedOut(String),
    Cancelled,
}

impl fmt::Display for ProcessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(p) => write!(f, "cannot run `{p}`: program not found"),
            Self::Failed {
                cmd,
                status,
                stderr,
            } => write!(f, "`{cmd}` failed (exit {status}): {}", stderr.trim()),
            Self::TimedOut(cmd) => write!(f, "`{cmd}` timed out"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::error::Error for ProcessError {}

fn describe(cmd: &Command) -> String {
    let mut s = cmd.get_program().to_string_lossy().into_owned();
    for a in cmd.get_args() {
        s.push(' ');
        s.push_str(&a.to_string_lossy());
    }
    s
}

/// Grace period for collecting output once the child (group) is gone.
const DRAIN_GRACE: Duration = Duration::from_secs(2);

fn drain<R: Read + Send + 'static>(mut r: R) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = r.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });
    rx
}

/// Bounded wait: a stray grandchild holding the pipe open cannot hang us.
fn collect(rx: Option<mpsc::Receiver<Vec<u8>>>) -> Vec<u8> {
    rx.and_then(|r| r.recv_timeout(DRAIN_GRACE).ok())
        .unwrap_or_default()
}

/// Runs `cmd` to completion. Polls every 50 ms; on timeout or cancellation the
/// whole process group is killed. A non-zero exit is `ProcessError::Failed`.
/// The command line, stdout and stderr are appended to `log`.
pub fn run(
    mut cmd: Command,
    timeout: Duration,
    cancel: &CancelToken,
    log: &mut dyn Write,
) -> Result<Output, ProcessError> {
    let line = describe(&cmd);
    let _ = writeln!(log, "$ {line}");
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = cmd.spawn().map_err(|e| {
        let _ = writeln!(log, "spawn failed: {e}");
        ProcessError::NotFound(cmd.get_program().to_string_lossy().into_owned())
    })?;
    let out_t = child.stdout.take().map(drain);
    let err_t = child.stderr.take().map(drain);
    let pid = child.id() as i32;
    let deadline = Instant::now() + timeout;

    let outcome = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                // Reap stragglers (daemons, background jobs) in the group so
                // they cannot hold our pipes open.
                // SAFETY: signalling the group we created for this child.
                unsafe {
                    libc::killpg(pid, libc::SIGKILL);
                }
                break Ok(status);
            }
            Ok(None) => {}
            Err(_) => break Err(ProcessError::NotFound(line.clone())),
        }
        let stop = if cancel.is_cancelled() {
            Some(ProcessError::Cancelled)
        } else if Instant::now() >= deadline {
            Some(ProcessError::TimedOut(line.clone()))
        } else {
            None
        };
        if let Some(e) = stop {
            // SAFETY: signalling the group we created for this child.
            unsafe {
                libc::killpg(pid, libc::SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
            break Err(e);
        }
        std::thread::sleep(Duration::from_millis(50));
    };

    let stdout = collect(out_t);
    let stderr = collect(err_t);
    let _ = log.write_all(&stdout);
    let _ = log.write_all(&stderr);
    match outcome {
        Err(e) => {
            let _ = writeln!(log, "-> {e}");
            Err(e)
        }
        Ok(status) if status.success() => Ok(Output {
            status,
            stdout,
            stderr,
        }),
        Ok(status) => Err(ProcessError::Failed {
            cmd: line,
            status: status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        }),
    }
}
