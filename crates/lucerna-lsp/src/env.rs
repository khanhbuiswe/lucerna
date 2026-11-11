//! Process environment as the user's login shell sees it, and PATH lookup.
//! Platform specifics (login shell) live here.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The environment a language server should run in: what the user's login
/// shell exports (so `PATH` includes rustup, nvm, Homebrew ...), falling back
/// to this process's environment when the shell fails or times out.
///
/// Blocks for up to 5 s while the shell starts. Never call it on the UI
/// thread; call it from a background thread and cache the result.
pub fn login_shell_env() -> HashMap<String, String> {
    login_shell_env_impl().unwrap_or_else(|| std::env::vars().collect())
}

#[cfg(unix)]
fn login_shell_env_impl() -> Option<HashMap<String, String>> {
    let shell = std::env::var_os("SHELL")?;
    shell_env_from(Path::new(&shell))
}

#[cfg(not(unix))]
fn login_shell_env_impl() -> Option<HashMap<String, String>> {
    None
}

/// Runs `shell -l -i -c 'printf "\0LUCERNA_ENV\0"; env -0'` and parses the
/// variables printed after the marker (rc files may print anything before
/// it). `None` when the shell cannot start, fails, times out after 5 s or
/// prints no marker. Blocking, like [`login_shell_env`].
pub fn shell_env_from(shell: &Path) -> Option<HashMap<String, String>> {
    shell_env_impl(shell)
}

#[cfg(unix)]
fn shell_env_impl(shell: &Path) -> Option<HashMap<String, String>> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    const TIMEOUT: Duration = Duration::from_secs(5);
    const SCRIPT: &str = r#"printf "\0LUCERNA_ENV\0"; env -0"#;

    let mut child = Command::new(shell)
        .args(["-l", "-i", "-c", SCRIPT])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("lsp-login-env".into())
        .spawn(move || {
            let mut buf = Vec::new();
            let _ = stdout.read_to_end(&mut buf);
            let _ = tx.send(buf);
        })
        .ok()?;

    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    if !status.success() {
        return None;
    }
    let output = rx.recv_timeout(Duration::from_secs(1)).ok()?;
    parse_marked_env(&output)
}

#[cfg(not(unix))]
fn shell_env_impl(_shell: &Path) -> Option<HashMap<String, String>> {
    None
}

#[cfg(unix)]
fn parse_marked_env(output: &[u8]) -> Option<HashMap<String, String>> {
    const MARKER: &[u8] = b"\0LUCERNA_ENV\0";
    let start = output
        .windows(MARKER.len())
        .position(|w| w == MARKER)?
        .checked_add(MARKER.len())?;
    let env: HashMap<String, String> = output[start..]
        .split(|&b| b == 0)
        .filter_map(|entry| {
            let entry = String::from_utf8_lossy(entry);
            let (key, value) = entry.split_once('=')?;
            (!key.is_empty()).then(|| (key.to_owned(), value.to_owned()))
        })
        .collect();
    (!env.is_empty()).then_some(env)
}

/// The first executable file called `name` in the `path_var` directories.
pub fn which(name: &str, path_var: &str) -> Option<PathBuf> {
    std::env::split_paths(path_var)
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}
