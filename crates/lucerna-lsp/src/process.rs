//! Stdio process transport. Platform specifics (kill, wait) live here.

use std::io::{self, BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, bounded, unbounded};

use crate::framing::{read_message, write_message};
use crate::guard::catch_panic;
use crate::transport::{Inbox, LaunchSpec, Transport};

/// The server process. The wait thread blocks on it without holding the
/// lock; `exited` is set under the lock right before the child is reaped, so
/// a kill never signals a pid that may have been recycled.
struct ChildProc {
    child: Mutex<Child>,
    pid: u32,
    exited: AtomicBool,
}

type SharedChild = Arc<ChildProc>;

/// Calls `Inbox::closed` at most once, however many threads end or panic.
struct Closer {
    inbox: Arc<dyn Inbox>,
    done: AtomicBool,
}

impl Closer {
    fn close(&self, code: Option<i32>) {
        if !self.done.swap(true, Ordering::AcqRel) {
            self.inbox.closed(code);
        }
    }
}

struct ProcessTransport {
    tx: Sender<String>,
    child: SharedChild,
}

impl Transport for ProcessTransport {
    fn send(&self, body: String) {
        // Fails only once the writer has gone, i.e. the server is closing.
        let _ = self.tx.send(body);
    }

    fn kill(&self) {
        kill_child(&self.child);
    }
}

/// Kills the server and everything it spawned (npx/node shims, `sh -c`).
fn kill_child(proc: &SharedChild) {
    let mut child = proc.child.lock().unwrap_or_else(PoisonError::into_inner);
    if proc.exited.load(Ordering::Acquire) {
        return;
    }
    kill_group(proc.pid);
    let _ = child.kill();
}

/// The child leads its own process group (see `spawn_process`), so a group
/// kill reaches descendants that would otherwise keep the pipes open.
#[cfg(unix)]
fn kill_group(pid: u32) {
    if let Ok(pgid) = libc::pid_t::try_from(pid) {
        // SAFETY: plain syscall with integer arguments; a stale group id
        // makes it fail with ESRCH, which is ignored.
        unsafe {
            libc::killpg(pgid, libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
fn kill_group(_pid: u32) {}

/// How long `closed` waits for the pipe readers once the server is gone.
const PIPE_GRACE: Duration = Duration::from_millis(500);

/// Starts the server process and its reader, writer and stderr threads.
///
/// # Errors
/// When the process or a thread cannot be started.
pub fn spawn_process(spec: &LaunchSpec, inbox: Arc<dyn Inbox>) -> io::Result<Arc<dyn Transport>> {
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .env_clear()
        .envs(&spec.env)
        .current_dir(&spec.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command.spawn()?;
    let stdin = child.stdin.take().expect("stdin is piped");
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let pid = child.id();
    let child: SharedChild = Arc::new(ChildProc {
        child: Mutex::new(child),
        pid,
        exited: AtomicBool::new(false),
    });
    let closer = Arc::new(Closer {
        inbox,
        done: AtomicBool::new(false),
    });
    let (tx, rx) = unbounded::<String>();

    let started = start_threads(spec, &child, &closer, rx, stdin, stdout, stderr);
    if let Err(err) = started {
        kill_child(&child);
        return Err(err);
    }
    Ok(Arc::new(ProcessTransport { tx, child }))
}

fn start_threads(
    spec: &LaunchSpec,
    child: &SharedChild,
    closer: &Arc<Closer>,
    rx: Receiver<String>,
    mut stdin: std::process::ChildStdin,
    stdout: std::process::ChildStdout,
    stderr: std::process::ChildStderr,
) -> io::Result<()> {
    let server = &spec.server;

    let name = format!("lsp-stderr-{server}");
    let (stderr_done, stderr_finished) = bounded::<()>(0);
    spawn_named(&name, child, closer, {
        let inbox = Arc::clone(&closer.inbox);
        move || {
            let _done = stderr_done;
            let mut reader = BufReader::new(stderr);
            let mut buf = Vec::new();
            loop {
                buf.clear();
                match reader.read_until(b'\n', &mut buf) {
                    Ok(0) => break,
                    Ok(_) => {
                        let line = String::from_utf8_lossy(&buf);
                        inbox.stderr(line.trim_end_matches(['\r', '\n']).to_owned());
                    }
                    Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
        }
    })?;

    let name = format!("lsp-write-{server}");
    spawn_named(&name, child, closer, move || {
        for body in rx {
            if write_message(&mut stdin, &body).is_err() {
                break;
            }
        }
        // Dropping stdin closes the pipe so a well-behaved server exits.
    })?;

    let name = format!("lsp-read-{server}");
    let (reader_done, reader_finished) = bounded::<()>(0);
    // Disconnected by the wait thread once the process has exited.
    let (process_exited, exit_seen) = bounded::<()>(0);
    spawn_named(&name, child, closer, {
        let child = Arc::clone(child);
        let closer = Arc::clone(closer);
        move || {
            let _done = reader_done;
            let mut reader = BufReader::new(stdout);
            loop {
                match read_message(&mut reader) {
                    Ok(Some(body)) => closer.inbox.message(body),
                    Ok(None) => {
                        // stdout closed: a server that keeps running without
                        // it can never answer again, so end it.
                        if exit_seen
                            .recv_timeout(PIPE_GRACE)
                            .is_err_and(|e| e.is_timeout())
                        {
                            tracing::warn!("language server closed stdout but kept running");
                            kill_child(&child);
                        }
                        break;
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "language server stream ended abnormally");
                        kill_child(&child);
                        break;
                    }
                }
            }
            // Let the stderr tail finish so its lines precede `closed`, but
            // never wait on a pipe some descendant still holds.
            let _ = stderr_finished.recv_timeout(PIPE_GRACE);
        }
    })?;

    // `closed` hangs off the process exit, not pipe EOF.
    let name = format!("lsp-wait-{server}");
    spawn_named(&name, child, closer, {
        let child = Arc::clone(child);
        let closer = Arc::clone(closer);
        move || {
            let code = wait_exit_code(&child);
            drop(process_exited);
            let _ = reader_finished.recv_timeout(PIPE_GRACE);
            closer.close(code);
        }
    })?;
    Ok(())
}

/// Spawns a thread whose panic is logged, kills the child and reports
/// `closed(None)`.
fn spawn_named(
    name: &str,
    child: &SharedChild,
    closer: &Arc<Closer>,
    f: impl FnOnce() + Send + 'static,
) -> io::Result<JoinHandle<()>> {
    let child = Arc::clone(child);
    let closer = Arc::clone(closer);
    let task = name.to_owned();
    thread::Builder::new().name(name.to_owned()).spawn(move || {
        if catch_panic(&task, f).is_err() {
            kill_child(&child);
            closer.close(None);
        }
    })
}

/// Blocks until the child exits, then reaps it. Orphaned descendants are
/// killed while the zombie still holds the pid, so the group id cannot have
/// been recycled.
fn wait_exit_code(proc: &ChildProc) -> Option<i32> {
    if !wait_without_reaping(proc.pid) {
        return poll_exit_code(proc);
    }
    let mut child = proc.child.lock().unwrap_or_else(PoisonError::into_inner);
    kill_group(proc.pid);
    proc.exited.store(true, Ordering::Release);
    child.wait().ok().and_then(|status| status.code())
}

/// Blocks until `pid` has exited, leaving it a zombie. `false` when the wait
/// is not possible and the caller has to poll.
#[cfg(unix)]
fn wait_without_reaping(pid: u32) -> bool {
    let id = libc::id_t::from(pid);
    loop {
        // SAFETY: `info` is a plain C struct the call fills in; WNOWAIT leaves
        // the child for `Child::wait` to reap.
        let rc = unsafe {
            let mut info: libc::siginfo_t = std::mem::zeroed();
            libc::waitid(
                libc::P_PID,
                id,
                &raw mut info,
                libc::WEXITED | libc::WNOWAIT,
            )
        };
        if rc == 0 {
            return true;
        }
        if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            return false;
        }
    }
}

#[cfg(not(unix))]
fn wait_without_reaping(_pid: u32) -> bool {
    false
}

/// Fallback: polls the exit status, without holding the lock between polls
/// so `kill` stays possible.
fn poll_exit_code(proc: &ChildProc) -> Option<i32> {
    loop {
        {
            let mut child = proc.child.lock().unwrap_or_else(PoisonError::into_inner);
            match child.try_wait() {
                Ok(Some(status)) => {
                    proc.exited.store(true, Ordering::Release);
                    return status.code();
                }
                Ok(None) => {}
                Err(_) => {
                    proc.exited.store(true, Ordering::Release);
                    return None;
                }
            }
        }
        thread::sleep(Duration::from_millis(250));
    }
}
