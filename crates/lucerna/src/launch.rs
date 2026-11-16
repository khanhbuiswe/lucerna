//! Starting the app apart from the terminal and handing it a request
//! (spec M18, §6.5 of the multi-window design): `luc` in a terminal runs
//! the app as its own session, like `code .`, so closing the terminal or
//! Ctrl+C there does not take every window with it.

use std::ffi::OsString;
use std::io;
use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use lucerna_workbench::cli_socket::{self, CliReply, CliRequest, SendError};

/// How often [`send_with_retry`] tries to connect while the app starts.
const RETRY_EVERY: Duration = Duration::from_millis(50);

/// Runs `exe` with `args` in `cwd` as a new session (`setsid`: no
/// controlling terminal), its stdin, stdout and stderr on `/dev/null`
/// (the app logs to its file). Does not wait for it; a thread reaps it
/// should it exit while this process still runs.
///
/// # Errors
/// When the process cannot be started.
pub fn spawn_detached(exe: &Path, args: &[OsString], cwd: &Path) -> io::Result<()> {
    let mut command = Command::new(exe);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: `setsid` is async-signal-safe and touches no memory of this
    // process, as `pre_exec` requires between `fork` and `exec`.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn()?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// Sends `request` to the app starting at `path` (spec §6.5): tries to
/// connect every 50 ms for up to `connect_within`, then waits up to
/// `reply_within` for the reply, so at most their sum in all.
///
/// # Errors
/// [`SendError::Unreachable`] when no app listened within
/// `connect_within`; otherwise as [`cli_socket::send`].
pub fn send_with_retry(
    path: &Path,
    request: &CliRequest,
    connect_within: Duration,
    reply_within: Duration,
) -> Result<CliReply, SendError> {
    let start = Instant::now();
    loop {
        match std::os::unix::net::UnixStream::connect(path) {
            Ok(stream) => return cli_socket::send(stream, request, reply_within),
            Err(err) if start.elapsed() >= connect_within => {
                return Err(SendError::Unreachable(format!(
                    "could not reach Lucerna: {err}"
                )));
            }
            Err(_) => std::thread::sleep(RETRY_EVERY),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead as _, BufReader, Write as _};
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn send_with_retry_waits_for_the_app() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sock");
        let server_path = path.clone();
        let server = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            let listener = UnixListener::bind(&server_path).unwrap();
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            (&stream).write_all(b"{\"ok\":true}\n").unwrap();
            line
        });
        let request = CliRequest::new(PathBuf::from("/w"), Vec::new());
        let reply = send_with_retry(
            &path,
            &request,
            Duration::from_secs(5),
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(reply, CliReply::ok());
        let sent: CliRequest = serde_json::from_str(&server.join().unwrap()).unwrap();
        assert_eq!(sent, request);
    }

    #[test]
    fn send_with_retry_gives_up() {
        let dir = tempfile::tempdir().unwrap();
        let request = CliRequest::new(PathBuf::from("/w"), Vec::new());
        let start = Instant::now();
        let result = send_with_retry(
            &dir.path().join("none.sock"),
            &request,
            Duration::from_millis(200),
            Duration::from_secs(5),
        );
        assert!(matches!(result, Err(SendError::Unreachable(_))));
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn spawn_detached_runs_in_its_own_session() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("sid");
        // `ps -o sess=` is not portable; compare the process group, which
        // `setsid` also makes new, with this process's.
        let script = format!("ps -o pgid= -p $$ > '{}'", out.display());
        spawn_detached(
            Path::new("/bin/sh"),
            &["-c".into(), script.into()],
            dir.path(),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let pgid = loop {
            if let Ok(text) = std::fs::read_to_string(&out)
                && !text.trim().is_empty()
            {
                break text.trim().parse::<i32>().unwrap();
            }
            assert!(Instant::now() < deadline, "the child ran");
            std::thread::sleep(Duration::from_millis(20));
        };
        // SAFETY: `getpgrp` cannot fail.
        let ours = unsafe { libc::getpgrp() };
        assert_ne!(pgid, ours, "the child leads a group of its own");
    }
}
