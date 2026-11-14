use crate::{GitBinary, GitError, MAX_DIFF_BYTES};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A query that takes longer than this kills `cat-file` (like every git command).
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);

struct Session {
    child: Arc<Mutex<Child>>,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Drop for Session {
    fn drop(&mut self) {
        kill(&self.child);
    }
}

fn kill(child: &Mutex<Child>) {
    let mut c = child.lock().unwrap_or_else(|e| e.into_inner());
    let _ = c.kill();
    let _ = c.wait();
}

/// A long-lived `git cat-file --batch`, started lazily and restarted once if it dies.
pub(crate) struct BlobReader {
    git: GitBinary,
    cwd: PathBuf,
    session: Option<Session>,
    max_bytes: u64,
    timeout: Duration,
}

impl BlobReader {
    pub(crate) fn new(git: GitBinary, cwd: PathBuf) -> BlobReader {
        BlobReader {
            git,
            cwd,
            session: None,
            max_bytes: MAX_DIFF_BYTES,
            timeout: QUERY_TIMEOUT,
        }
    }

    pub(crate) fn set_limits_for_test(&mut self, max_bytes: u64, timeout: Duration) {
        self.max_bytes = max_bytes;
        self.timeout = timeout;
    }

    fn spawn(&self) -> Result<Session, GitError> {
        let mut cmd = self.git.command(&self.cwd);
        cmd.args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
        Ok(Session {
            child: Arc::new(Mutex::new(child)),
            stdin,
            stdout,
        })
    }

    pub(crate) fn kill_for_test(&mut self) {
        if let Some(s) = &self.session {
            kill(&s.child);
        }
    }

    /// `spec` is `<rev>:<repo-relative path>`. `Ok(None)` when missing or not a blob.
    pub(crate) fn read(&mut self, spec: &str) -> Result<Option<Vec<u8>>, GitError> {
        if spec.contains('\n') {
            return Ok(None);
        }
        for attempt in 0..2 {
            if self.session.is_none() {
                self.session = Some(self.spawn()?);
            }
            let fired = Arc::new(AtomicBool::new(false));
            let session = self.session.as_mut().expect("session");
            let (done_tx, done_rx) = mpsc::channel::<()>();
            let (child, flag, timeout) = (session.child.clone(), fired.clone(), self.timeout);
            let watchdog = std::thread::spawn(move || {
                if let Err(RecvTimeoutError::Timeout) = done_rx.recv_timeout(timeout) {
                    flag.store(true, Ordering::SeqCst);
                    kill(&child);
                }
            });
            let result = Self::query(session, spec, self.max_bytes);
            drop(done_tx);
            let _ = watchdog.join();
            match result {
                Ok(r) if !fired.load(Ordering::SeqCst) => return Ok(r),
                Ok(_) | Err(_) if fired.load(Ordering::SeqCst) => {
                    self.session = None;
                    return Err(GitError::Timeout);
                }
                Ok(_) => unreachable!(),
                Err(e) => {
                    self.session = None;
                    if attempt == 1 {
                        return Err(e);
                    }
                }
            }
        }
        unreachable!()
    }

    fn query(s: &mut Session, spec: &str, max_bytes: u64) -> Result<Option<Vec<u8>>, GitError> {
        s.stdin.write_all(spec.as_bytes())?;
        s.stdin.write_all(b"\n")?;
        s.stdin.flush()?;
        let mut header = String::new();
        if s.stdout.read_line(&mut header)? == 0 {
            return Err(GitError::Io(std::io::ErrorKind::UnexpectedEof.into()));
        }
        let header = header.trim_end();
        // `<oid> <type> <size>` or `<spec> missing` / `<spec> ambiguous`.
        let mut parts = header.rsplitn(3, ' ');
        let last = parts.next().unwrap_or("");
        let Ok(size) = last.parse::<usize>() else {
            return Ok(None);
        };
        let kind = parts.next().unwrap_or("");
        // Content plus trailing LF. Never allocate more than the cap: skip instead.
        if kind != "blob" || size as u64 > max_bytes {
            let want = size as u64 + 1;
            let n = std::io::copy(&mut (&mut s.stdout).take(want), &mut std::io::sink())?;
            if n != want {
                return Err(GitError::Io(std::io::ErrorKind::UnexpectedEof.into()));
            }
            return Ok(None);
        }
        let mut buf = vec![0u8; size + 1];
        s.stdout.read_exact(&mut buf)?;
        if kind != "blob" {
            return Ok(None);
        }
        buf.truncate(size);
        Ok(Some(buf))
    }
}
