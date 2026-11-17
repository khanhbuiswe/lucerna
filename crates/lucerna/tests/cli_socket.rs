//! `luc` hands its paths to the running app over the socket and leaves
//! the terminal (spec M13, M18, §8.4 of the multi-window design).
//!
//! Real processes with a window, so ignored: run by hand on a machine
//! with a display, `cargo test -p lucerna --test cli_socket -- --ignored`.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::{Duration, Instant};

use assert_cmd::Command;
use assert_cmd::cargo::cargo_bin;
use lucerna_workbench::cli_socket::socket_path;

/// The app processes a test started: `luc --foreground` or the
/// `--detached-child` that `luc` starts, of this build
/// on `dir` (canonical), the folder the test opened first; by pid.
fn app_pids(exe: &Path, dir: &Path) -> Vec<i32> {
    let out = std::process::Command::new("ps")
        .args(["-axo", "pid=,command="])
        .output()
        .unwrap();
    let exe = exe.to_string_lossy();
    let dir = dir.to_string_lossy();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let (pid, command) = line.trim().split_once(' ')?;
            (command.starts_with(&*exe)
                && (command.contains("--foreground") || command.contains("--detached-child"))
                && command.contains(&*dir))
            .then(|| pid.parse().ok())
            .flatten()
        })
        .collect()
}

/// Kills the app processes of a test ([`app_pids`]: executable, first
/// folder) when it ends, passed or not, and removes the socket of its
/// `HOME`, which a killed app leaves (a temporary HOME is long, so the
/// socket is in the temporary directory, not in the HOME).
struct KillApps(PathBuf, PathBuf, PathBuf);

impl Drop for KillApps {
    fn drop(&mut self) {
        for pid in app_pids(&self.0, &self.1) {
            // SAFETY: a plain signal to a process this test started.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
        let data_dir = self
            .2
            .join("Library")
            .join("Application Support")
            .join("Lucerna");
        std::fs::remove_file(socket_path(&data_dir)).ok();
    }
}

fn luc(home: &Path, args: &[&Path]) -> (Output, Duration) {
    let start = Instant::now();
    let output = Command::cargo_bin("luc")
        .unwrap()
        .env("HOME", home)
        .args(args)
        .timeout(Duration::from_secs(30))
        .output()
        .unwrap();
    (output, start.elapsed())
}

/// Waits up to 10 s for `pids` to say what the test expects.
fn wait_for(exe: &Path, dir: &Path, what: &str, ok: impl Fn(&[i32]) -> bool) -> Vec<i32> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let pids = app_pids(exe, dir);
        if ok(&pids) {
            return pids;
        }
        assert!(Instant::now() < deadline, "{what}: {pids:?}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
#[ignore = "starts the app: needs a display"]
fn luc_hands_paths_to_one_detached_app() {
    let exe = cargo_bin("luc");
    let home = tempfile::tempdir().unwrap();
    let (dir1, dir2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let first = std::fs::canonicalize(dir1.path()).unwrap();
    let _kill = KillApps(exe.clone(), first.clone(), home.path().to_path_buf());

    let (output, took) = luc(home.path(), &[dir1.path()]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(took < Duration::from_secs(10), "returned after {took:?}");
    let pids = wait_for(&exe, &first, "one app", |pids| pids.len() == 1);

    let (output, _) = luc(home.path(), &[dir2.path()]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        app_pids(&exe, &first),
        pids,
        "the second path opens in the same app"
    );
    let second = std::fs::canonicalize(dir2.path()).unwrap();
    assert!(app_pids(&exe, &second).is_empty(), "no second app");

    let (output, _) = luc(home.path(), &[Path::new("/nope")]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("path not found"), "{stderr}");
    assert_eq!(app_pids(&exe, &first), pids);
}

#[test]
#[ignore = "starts the app: needs a display"]
fn foreground_runs_the_app_in_the_calling_process() {
    let exe = cargo_bin("luc");
    let home = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let canonical = std::fs::canonicalize(dir.path()).unwrap();
    let _kill = KillApps(exe.clone(), canonical.clone(), home.path().to_path_buf());
    let mut child = std::process::Command::new(&exe)
        .env("HOME", home.path())
        .arg("--foreground")
        .arg(&canonical)
        .spawn()
        .unwrap();
    let pid = i32::try_from(child.id()).unwrap();
    let pids = wait_for(&exe, &canonical, "the app", |pids| pids.contains(&pid));
    assert_eq!(pids, vec![pid], "the app is the process `luc` runs as");
    assert!(child.try_wait().unwrap().is_none(), "still running");
    child.kill().unwrap();
    child.wait().unwrap();
}

/// `luc <file>` with no app: the started app opens the file, `luc` sends
/// it again, and it all ends in one app (the router test
/// `cli_same_file_as_launch_opens_once` checks one window and one tab).
#[test]
#[ignore = "starts the app: needs a display"]
fn luc_file_starts_one_app() {
    let exe = cargo_bin("luc");
    let home = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let file = std::fs::canonicalize(dir.path()).unwrap().join("notes.txt");
    std::fs::write(&file, "hi\n").unwrap();
    let _kill = KillApps(exe.clone(), file.clone(), home.path().to_path_buf());
    let (output, took) = luc(home.path(), &[&file]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(took < Duration::from_secs(10), "returned after {took:?}");
    wait_for(&exe, &file, "one app", |pids| pids.len() == 1);
}
