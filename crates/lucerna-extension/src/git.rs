//! Non-interactive git helpers.

use crate::process::{CancelToken, ProcessError, run};
use std::ffi::OsStr;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

/// Timeout for each git call.
pub const GIT_TIMEOUT: Duration = Duration::from_secs(120);

/// A `git` command that can never prompt. Local (`file://` or path)
/// remotes are refused unless `allow_file_urls` (a test opt-in) is set.
pub fn git_command(allow_file_urls: bool) -> Command {
    git_command_with(OsStr::new("git"), allow_file_urls)
}

/// [`git_command`] running `program` instead of `git` from `PATH`.
pub fn git_command_with(program: &OsStr, allow_file_urls: bool) -> Command {
    let mut cmd = Command::new(program);
    // The user's gitconfig is kept (proxy settings matter) but credential
    // helpers, hooks and fsmonitor (which runs a configured program) are
    // disabled. GIT_SSH_COMMAND is always overridden (even if the user set
    // one) so ssh can never prompt.
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("SSH_ASKPASS", "")
        .env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes")
        .args([
            "-c",
            if allow_file_urls {
                "protocol.file.allow=always"
            } else {
                "protocol.file.allow=never"
            },
            "-c",
            "credential.helper=",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
        ]);
    cmd
}

/// Runs git with `args` (optionally in `dir`) under the standard timeout.
/// Every argument taken from an extension or the registry must come after
/// a `--` or be validated ([`check_url`], [`is_commit_sha`]).
pub fn run_git(
    dir: Option<&Path>,
    args: &[&str],
    allow_file_urls: bool,
    cancel: &CancelToken,
    log: &mut dyn Write,
) -> Result<std::process::Output, ProcessError> {
    let mut cmd = git_command(allow_file_urls);
    if let Some(d) = dir {
        cmd.arg("-C").arg(d);
    }
    cmd.args(args);
    run(cmd, GIT_TIMEOUT, cancel, log)
}

pub fn git_available() -> bool {
    git_available_with(OsStr::new("git"))
}

/// Whether `program --version` runs (see [`git_command_with`]).
pub fn git_available_with(program: &OsStr) -> bool {
    let mut cmd = git_command_with(program, false);
    cmd.arg("--version");
    run(
        cmd,
        Duration::from_secs(10),
        &CancelToken::default(),
        &mut std::io::sink(),
    )
    .is_ok()
}

/// Whether `s` is a full commit hash: 40 (SHA-1) or 64 (SHA-256) hex digits.
pub fn is_commit_sha(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Normalizes `url` ([`normalize_url`]) and checks that it is safe to hand
/// to git: never option-like, `https://` only, and local `file://` URLs or
/// absolute paths only when `allow_file_urls` (a test opt-in) is set.
pub fn check_url(url: &str, allow_file_urls: bool) -> Result<String, String> {
    let normalized = normalize_url(url);
    let ok = !url.starts_with('-')
        && !normalized.chars().any(|c| c.is_control())
        && (normalized.starts_with("https://")
            || (allow_file_urls
                && (normalized.starts_with("file://") || normalized.starts_with('/'))));
    if ok {
        Ok(normalized)
    } else {
        Err(format!(
            "unsupported repository URL `{url}` (only https:// URLs are allowed)"
        ))
    }
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

/// Removes `*.lock` files that a killed git left in `repo/.git` (top level,
/// e.g. `shallow.lock`, `index.lock`, `HEAD.lock`, and under `refs/`).
/// Call it only while holding the lock that proves no other Lucerna git
/// runs in `repo` (the registry clone lock, or the id lock for `work/<id>`).
pub fn remove_stale_locks(repo: &Path) {
    fn sweep(dir: &Path, recurse: bool) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let Ok(ty) = e.file_type() else { continue };
            let path = e.path();
            if ty.is_dir() {
                if recurse {
                    sweep(&path, true);
                }
            } else if path.extension().is_some_and(|x| x == "lock") {
                match std::fs::remove_file(&path) {
                    Ok(()) => tracing::warn!("removed stale git lock {}", path.display()),
                    Err(e) => tracing::warn!("cannot remove {}: {e}", path.display()),
                }
            }
        }
    }
    let git = repo.join(".git");
    sweep(&git, false);
    sweep(&git.join("refs"), true);
}

/// Whether a git failure is "a lock file exists" (left by a killed git).
pub fn is_lock_error(e: &ProcessError) -> bool {
    matches!(e, ProcessError::Failed { stderr, .. }
        if stderr.contains("Unable to create") && stderr.contains(".lock"))
}

fn head_is(dest: &Path, commit: &str, cancel: &CancelToken) -> bool {
    if !dest.join(".git").exists() {
        return false;
    }
    run_git(
        Some(dest),
        &["rev-parse", "HEAD"],
        false,
        cancel,
        &mut std::io::sink(),
    )
    .map(|o| String::from_utf8_lossy(&o.stdout).trim() == commit)
    .unwrap_or(false)
}

/// Shallow-fetches `commit` of `url` into `dest` and checks it out.
/// No-op when `dest` is already at `commit`. A broken `dest` is wiped and
/// retried once from scratch. `commit` must be a full hash and `url` must
/// pass [`check_url`]; otherwise nothing runs. The caller holds the id lock
/// of the checkout, so stale git lock files in `dest` are removed first.
pub fn fetch_commit(
    url: &str,
    commit: &str,
    dest: &Path,
    allow_file_urls: bool,
    cancel: &CancelToken,
    log: &mut dyn Write,
) -> Result<(), ProcessError> {
    if !is_commit_sha(commit) {
        return Err(ProcessError::Rejected(format!(
            "`{commit}` is not a full commit hash"
        )));
    }
    // git prints and matches object names in lowercase.
    let commit = &commit.to_ascii_lowercase();
    let url = check_url(url, allow_file_urls).map_err(ProcessError::Rejected)?;
    remove_stale_locks(dest);
    if head_is(dest, commit, cancel) {
        let _ = writeln!(log, "{} already at {commit}", dest.display());
        return Ok(());
    }
    match fetch_inner(&url, commit, dest, allow_file_urls, cancel, log) {
        Err(ProcessError::Failed { .. }) if dest.join(".git").exists() => {
            let _ = writeln!(log, "retrying from scratch in {}", dest.display());
            let _ = std::fs::remove_dir_all(dest);
            fetch_inner(&url, commit, dest, allow_file_urls, cancel, log)
        }
        r => r,
    }
}

/// `url` is already checked and `commit` is a full hash.
fn fetch_inner(
    url: &str,
    commit: &str,
    dest: &Path,
    allow_file_urls: bool,
    cancel: &CancelToken,
    log: &mut dyn Write,
) -> Result<(), ProcessError> {
    std::fs::create_dir_all(dest).map_err(|e| ProcessError::Failed {
        cmd: format!("mkdir {}", dest.display()),
        status: -1,
        stderr: e.to_string(),
    })?;
    run_git(Some(dest), &["init", "-q"], false, cancel, log)?;
    run_git(
        Some(dest),
        &["fetch", "--depth", "1", "--", url, commit],
        allow_file_urls,
        cancel,
        log,
    )?;
    run_git(
        Some(dest),
        &["checkout", "--force", "-q", "FETCH_HEAD", "--"],
        false,
        cancel,
        log,
    )?;
    // Drop files left over from a previously checked-out commit.
    run_git(Some(dest), &["clean", "-fdx", "-q"], false, cancel, log)?;
    Ok(())
}
