//! Non-interactive git helpers.

use crate::process::{CancelToken, ProcessError, run};
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

/// Timeout for each git call.
pub const GIT_TIMEOUT: Duration = Duration::from_secs(120);

/// A `git` command that can never prompt and may use `file://` remotes.
pub fn git_command() -> Command {
    let mut cmd = Command::new("git");
    // The user's gitconfig is kept (proxy settings matter) but credential
    // helpers and hooks are disabled. GIT_SSH_COMMAND is always overridden
    // (even if the user set one) so ssh can never prompt.
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("SSH_ASKPASS", "")
        .env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes")
        .args([
            "-c",
            "protocol.file.allow=always",
            "-c",
            "credential.helper=",
            "-c",
            "core.hooksPath=/dev/null",
        ]);
    cmd
}

/// Runs git with `args` (optionally in `dir`) under the standard timeout.
pub fn run_git(
    dir: Option<&Path>,
    args: &[&str],
    cancel: &CancelToken,
    log: &mut dyn Write,
) -> Result<std::process::Output, ProcessError> {
    let mut cmd = git_command();
    if let Some(d) = dir {
        cmd.arg("-C").arg(d);
    }
    cmd.args(args);
    run(cmd, GIT_TIMEOUT, cancel, log)
}

pub fn git_available() -> bool {
    let mut cmd = git_command();
    cmd.arg("--version");
    run(
        cmd,
        Duration::from_secs(10),
        &CancelToken::default(),
        &mut std::io::sink(),
    )
    .is_ok()
}

/// `git@github.com:o/r(.git)` becomes `https://github.com/o/r`; others unchanged.
pub fn normalize_url(url: &str) -> String {
    match url.strip_prefix("git@github.com:") {
        Some(rest) => {
            let rest = rest.strip_suffix(".git").unwrap_or(rest);
            format!("https://github.com/{rest}")
        }
        None => url.to_string(),
    }
}

fn head_is(dest: &Path, commit: &str, cancel: &CancelToken) -> bool {
    if !dest.join(".git").exists() {
        return false;
    }
    run_git(
        Some(dest),
        &["rev-parse", "HEAD"],
        cancel,
        &mut std::io::sink(),
    )
    .map(|o| String::from_utf8_lossy(&o.stdout).trim() == commit)
    .unwrap_or(false)
}

/// Shallow-fetches `commit` of `url` into `dest` and checks it out.
/// No-op when `dest` is already at `commit`. A broken `dest` is wiped and
/// retried once from scratch.
pub fn fetch_commit(
    url: &str,
    commit: &str,
    dest: &Path,
    cancel: &CancelToken,
    log: &mut dyn Write,
) -> Result<(), ProcessError> {
    if head_is(dest, commit, cancel) {
        let _ = writeln!(log, "{} already at {commit}", dest.display());
        return Ok(());
    }
    match fetch_inner(url, commit, dest, cancel, log) {
        Err(ProcessError::Failed { .. }) if dest.join(".git").exists() => {
            let _ = writeln!(log, "retrying from scratch in {}", dest.display());
            let _ = std::fs::remove_dir_all(dest);
            fetch_inner(url, commit, dest, cancel, log)
        }
        r => r,
    }
}

fn fetch_inner(
    url: &str,
    commit: &str,
    dest: &Path,
    cancel: &CancelToken,
    log: &mut dyn Write,
) -> Result<(), ProcessError> {
    std::fs::create_dir_all(dest).map_err(|e| ProcessError::Failed {
        cmd: format!("mkdir {}", dest.display()),
        status: -1,
        stderr: e.to_string(),
    })?;
    let url = normalize_url(url);
    run_git(Some(dest), &["init", "-q"], cancel, log)?;
    run_git(
        Some(dest),
        &["fetch", "--depth", "1", &url, commit],
        cancel,
        log,
    )?;
    run_git(
        Some(dest),
        &["checkout", "--force", "-q", "FETCH_HEAD"],
        cancel,
        log,
    )?;
    // Drop files left over from a previously checked-out commit.
    run_git(Some(dest), &["clean", "-fdx", "-q"], cancel, log)?;
    Ok(())
}
