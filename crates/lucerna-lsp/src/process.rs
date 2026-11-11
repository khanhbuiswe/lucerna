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

type SharedChild = Arc<Mutex<Child>>;

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
fn kill_child(child: &SharedChild) {
    let mut child = child.lock().unwrap_or_else(PoisonError::into_inner);
    kill_group(&child);
    let _ = child.kill();
}

/// The child leads its own process group (see `spawn_process`), so a group
/// kill reaches descendants that would otherwise keep the pipes open.
#[cfg(unix)]
fn kill_group(child: &Child) {
    if let Ok(pgid) = libc::pid_t::try_from(child.id()) {
        // SAFETY: plain syscall with integer arguments; a stale group id
        // makes it fail with ESRCH, which is ignored.
        unsafe {
            libc::killpg(pgid, libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
fn kill_group(_child: &Child) {}

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
    let child: SharedChild = Arc::new(Mutex::new(child));
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
    spawn_named(&name, child, closer, {
        let child = Arc::clone(child);
        let closer = Arc::clone(closer);
        move || {
            let _done = reader_done;
            let mut reader = BufReader::new(stdout);
            loop {
                match read_message(&mut reader) {
                    Ok(Some(body)) => closer.inbox.message(body),
                    Ok(None) => break,
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
            // Reap orphaned descendants so they do not outlive the server.
            kill_child(&child);
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

/// Waits for the child without holding the lock, so `kill` stays possible.
fn wait_exit_code(child: &SharedChild) -> Option<i32> {
    loop {
        let status = child
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .try_wait();
        match status {
            Ok(Some(status)) => return status.code(),
            Ok(None) => thread::sleep(Duration::from_millis(5)),
            Err(_) => return None,
        }
    }
}
