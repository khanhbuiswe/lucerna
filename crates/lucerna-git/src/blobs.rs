use crate::{GitBinary, GitError};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Stdio};

struct Session {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A long-lived `git cat-file --batch`, started lazily and restarted once if it dies.
pub(crate) struct BlobReader {
    git: GitBinary,
    cwd: PathBuf,
    session: Option<Session>,
}

impl BlobReader {
    pub(crate) fn new(git: GitBinary, cwd: PathBuf) -> BlobReader {
        BlobReader {
            git,
            cwd,
            session: None,
        }
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
            child,
            stdin,
            stdout,
        })
    }

    pub(crate) fn kill_for_test(&mut self) {
        if let Some(s) = &mut self.session {
            let _ = s.child.kill();
            let _ = s.child.wait();
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
            match Self::query(self.session.as_mut().expect("session"), spec) {
                Ok(r) => return Ok(r),
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

    fn query(s: &mut Session, spec: &str) -> Result<Option<Vec<u8>>, GitError> {
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
        let mut buf = vec![0u8; size + 1]; // content plus trailing LF
        s.stdout.read_exact(&mut buf)?;
        if kind != "blob" {
            return Ok(None);
        }
        buf.truncate(size);
        Ok(Some(buf))
    }
}
