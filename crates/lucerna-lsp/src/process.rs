//! Stdio process transport. Platform specifics (kill, wait) live here.

use std::io::{self, BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, unbounded};

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

fn kill_child(child: &SharedChild) {
    let _ = child.lock().unwrap_or_else(PoisonError::into_inner).kill();
}

/// Starts the server process and its reader, writer and stderr threads.
///
/// # Errors
/// When the process or a thread cannot be started.
pub fn spawn_process(spec: &LaunchSpec, inbox: Arc<dyn Inbox>) -> io::Result<Arc<dyn Transport>> {
    let mut child = Command::new(&spec.program)
        .args(&spec.args)
        .env_clear()
        .envs(&spec.env)
        .current_dir(&spec.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
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
    let stderr_thread = spawn_named(&name, child, closer, {
        let inbox = Arc::clone(&closer.inbox);
        move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                inbox.stderr(line);
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
    spawn_named(&name, child, closer, {
        let child = Arc::clone(child);
        let closer = Arc::clone(closer);
        move || {
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
            // Let the stderr tail finish so its lines precede `closed`.
            let _ = stderr_thread.join();
            closer.close(wait_exit_code(&child));
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
