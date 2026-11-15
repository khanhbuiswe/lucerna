use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A `git` command is killed after this long.
const TIMEOUT: Duration = Duration::from_secs(10);

#[cfg(windows)]
const NULL_HOOKS: &str = "core.hooksPath=NUL";
#[cfg(not(windows))]
const NULL_HOOKS: &str = "core.hooksPath=/dev/null";

#[derive(Debug)]
pub enum GitError {
    NotFound,
    Timeout,
    Failed { code: Option<i32>, stderr: String },
    Io(std::io::Error),
    Parse(String),
}

impl GitError {
    /// `git` reported that the directory is not (or no longer) in a
    /// repository.
    pub fn is_not_a_repo(&self) -> bool {
        matches!(self, GitError::Failed { stderr, .. } if stderr.contains("not a git repository"))
    }
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitError::NotFound => write!(f, "git executable not found"),
            GitError::Timeout => write!(f, "git command timed out"),
            GitError::Failed { code, stderr } => {
                write!(f, "git failed (exit {code:?}): {}", stderr.trim())
            }
            GitError::Io(e) => write!(f, "git I/O error: {e}"),
            GitError::Parse(m) => write!(f, "git output parse error: {m}"),
        }
    }
}

impl std::error::Error for GitError {}

impl From<std::io::Error> for GitError {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::NotFound {
            GitError::NotFound
        } else {
            GitError::Io(e)
        }
    }
}

/// Path to a `git` executable. Read commands ([`command`](Self::command)) are
/// hardened: no fsmonitor, no hooks, no optional locks, no pager, no prompts,
/// C locale. Write commands ([`command_for_write`](Self::command_for_write))
/// keep hooks and locks.
#[derive(Debug, Clone)]
pub struct GitBinary {
    path: PathBuf,
    extra_env: Vec<(OsString, OsString)>,
}

impl GitBinary {
    pub fn new(path: PathBuf) -> GitBinary {
        GitBinary {
            path,
            extra_env: Vec::new(),
        }
    }

    /// Adds an environment variable to every command. Meant for tests (e.g. to
    /// isolate from the developer's git config); production never calls it.
    #[doc(hidden)]
    pub fn with_env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> GitBinary {
        self.extra_env.push((key.into(), value.into()));
        self
    }

    pub fn command(&self, cwd: &Path) -> Command {
        let mut cmd = Command::new(&self.path);
        cmd.args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            NULL_HOOKS,
            "--no-optional-locks",
            "--no-pager",
        ])
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .stdin(Stdio::null());
        cmd.envs(self.extra_env.iter().map(|(k, v)| (k, v)));
        cmd
    }

    /// Runs git and returns stdout. Non-zero exit is an error; 10 s timeout.
    pub fn run(&self, cwd: &Path, args: &[&OsStr]) -> Result<Vec<u8>, GitError> {
        let mut cmd = self.command(cwd);
        cmd.args(args);
        run_with_timeout(cmd, TIMEOUT)
    }

    /// A command for git operations that change the repository (`git switch`).
    /// Unlike [`command`](Self::command), hooks run as they would in a
    /// terminal (git-lfs, husky) and optional locks are taken.
    pub fn command_for_write(&self, cwd: &Path) -> Command {
        let mut cmd = Command::new(&self.path);
        cmd.args(["-c", "core.fsmonitor=false", "--no-pager"])
            .current_dir(cwd)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .stdin(Stdio::null());
        cmd.envs(self.extra_env.iter().map(|(k, v)| (k, v)));
        cmd
    }

    /// Runs a write command and returns stdout. Non-zero exit is an error.
    /// There is no timeout and the child is never killed: stopping git
    /// halfway can leave a half-switched tree or a stale `index.lock`.
    /// Call it off the UI thread.
    pub fn run_write(&self, cwd: &Path, args: &[&OsStr]) -> Result<Vec<u8>, GitError> {
        let mut cmd = self.command_for_write(cwd);
        cmd.args(args);
        run_to_completion(cmd)
    }
}

/// A running child and the threads draining its stdout and stderr.
type Drained = (
    std::process::Child,
    std::thread::JoinHandle<Vec<u8>>,
    std::thread::JoinHandle<Vec<u8>>,
);

/// Spawns `cmd` with piped stdout and stderr, each drained on a helper thread
/// so a full pipe cannot deadlock.
fn spawn_drained(mut cmd: Command) -> Result<Drained, GitError> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });
    Ok((child, out_thread, err_thread))
}

fn finish(
    status: std::process::ExitStatus,
    out: Vec<u8>,
    err: Vec<u8>,
) -> Result<Vec<u8>, GitError> {
    if status.success() {
        Ok(out)
    } else {
        Err(GitError::Failed {
            code: status.code(),
            stderr: String::from_utf8_lossy(&err).into_owned(),
        })
    }
}

/// Runs `cmd` until it exits, returning stdout on success. Never kills it.
fn run_to_completion(cmd: Command) -> Result<Vec<u8>, GitError> {
    let (mut child, out_thread, err_thread) = spawn_drained(cmd)?;
    // On a wait error the drain threads are detached rather than joined.
    let status = child.wait()?;
    let out = out_thread.join().unwrap_or_default();
    let err = err_thread.join().unwrap_or_default();
    finish(status, out, err)
}

/// Runs `cmd`, returning stdout on success. Stdout and stderr are drained on
/// helper threads so a full pipe cannot deadlock; the child is killed at the deadline.
#[doc(hidden)]
pub fn run_with_timeout(cmd: Command, timeout: Duration) -> Result<Vec<u8>, GitError> {
    let (mut child, out_thread, err_thread) = spawn_drained(cmd)?;

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e.into());
            }
        }
    };

    let out = out_thread.join().unwrap_or_default();
    let err = err_thread.join().unwrap_or_default();
    match status {
        None => Err(GitError::Timeout),
        Some(s) => finish(s, out, err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_is_hardened() {
        let cmd = GitBinary::new(PathBuf::from("git")).command(Path::new("."));
        let args: Vec<&OsStr> = cmd.get_args().collect();
        let expected = [
            "-c",
            "core.fsmonitor=false",
            "-c",
            NULL_HOOKS,
            "--no-optional-locks",
            "--no-pager",
        ];
        assert_eq!(args, expected.iter().map(OsStr::new).collect::<Vec<_>>());
        let envs: Vec<_> = cmd.get_envs().collect();
        assert!(envs.contains(&(OsStr::new("GIT_TERMINAL_PROMPT"), Some(OsStr::new("0")))));
        assert!(envs.contains(&(OsStr::new("LC_ALL"), Some(OsStr::new("C")))));
    }
}
