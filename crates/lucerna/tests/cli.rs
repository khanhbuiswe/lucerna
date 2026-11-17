use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use lucerna::{Target, prompts_for_folder, resolve_target, resolve_target_with_home};

#[test]
fn version_flag_prints_name_and_version() {
    let home = tempfile::tempdir().unwrap();
    Command::cargo_bin("luc")
        .unwrap()
        .env("HOME", home.path())
        .arg("--version")
        .assert()
        .success()
        .stdout("luc 0.1.0\n");
}

fn canonical(path: impl Into<PathBuf>) -> PathBuf {
    fs::canonicalize(path.into()).unwrap()
}

#[test]
fn resolve_none_is_cwd() {
    let cwd = std::env::current_dir().unwrap();
    assert_eq!(resolve_target(None), Ok(Target::folder(canonical(cwd))));
}

#[test]
fn resolve_dir() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("sub");
    fs::create_dir(&sub).unwrap();
    assert_eq!(
        resolve_target(Some(sub.clone())),
        Ok(Target::folder(canonical(sub)))
    );
}

/// A file opens alone, as in Zed and VS Code: its folder (or repository)
/// is never scanned, and the root is the file's folder.
fn lone(file: &Path) -> Target {
    let file = canonical(file);
    Target {
        root: file.parent().unwrap().to_path_buf(),
        file: Some(file),
        single_file: true,
    }
}

#[test]
fn resolve_file_in_git_repo_opens_alone() {
    let repo = tempfile::tempdir().unwrap();
    fs::create_dir(repo.path().join(".git")).unwrap();
    let nested = repo.path().join("src").join("deep");
    fs::create_dir_all(&nested).unwrap();
    let file = nested.join("main.rs");
    fs::write(&file, "fn main() {}\n").unwrap();
    assert_eq!(resolve_target(Some(file.clone())), Ok(lone(&file)));
}

#[test]
fn resolve_file_in_git_worktree_opens_alone() {
    let outer = tempfile::tempdir().unwrap();
    fs::create_dir(outer.path().join(".git")).unwrap();
    let worktree = outer.path().join("worktree");
    let nested = worktree.join("src");
    fs::create_dir_all(&nested).unwrap();
    fs::write(
        worktree.join(".git"),
        "gitdir: ../.git/worktrees/worktree\n",
    )
    .unwrap();
    let file = nested.join("lib.rs");
    fs::write(&file, "\n").unwrap();
    assert_eq!(resolve_target(Some(file.clone())), Ok(lone(&file)));
}

/// A file in a plain folder such as `~/Downloads` opens alone; the folder
/// is not listed in the sidebar.
#[test]
fn resolve_file_in_plain_folder_opens_alone() {
    let home = tempfile::tempdir().unwrap();
    let downloads = home.path().join("Downloads");
    fs::create_dir(&downloads).unwrap();
    let file = downloads.join("notes.md");
    fs::write(&file, "hi\n").unwrap();
    let target = resolve_target_with_home(Some(file.clone()), Some(home.path())).unwrap();
    assert_eq!(target, lone(&file));
    // A loose file in `$HOME` itself too.
    let in_home = home.path().join("todo.md");
    fs::write(&in_home, "").unwrap();
    let target = resolve_target_with_home(Some(in_home.clone()), Some(home.path())).unwrap();
    assert_eq!(target, lone(&in_home));
}

#[test]
fn resolve_folders_stay_folders() {
    let home = tempfile::tempdir().unwrap();
    let repo = home.path().join("repo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    let sub = home.path().join("sub");
    fs::create_dir(&sub).unwrap();
    for dir in [home.path().to_path_buf(), repo, sub] {
        let target = resolve_target_with_home(Some(dir.clone()), Some(home.path())).unwrap();
        assert_eq!(target, Target::folder(canonical(&dir)));
    }
}

#[test]
fn resolve_missing_errors() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope");
    assert_eq!(
        resolve_target(Some(missing.clone())),
        Err(format!("path not found: {}", missing.display()))
    );
}

/// `luc <path>` and the paths Finder sends the running app resolve the
/// same way.
#[test]
fn resolve_matches_the_router() {
    use lucerna_workbench::router::Launch;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.rs");
    fs::write(&file, "\n").unwrap();
    let missing = dir.path().join("nope");
    for path in [dir.path().to_path_buf(), file, missing] {
        let cli = resolve_target(Some(path.clone()));
        let router = Launch::for_path(path).map(|l| Target {
            root: l.root,
            file: l.file,
            single_file: l.single_file,
        });
        assert_eq!(cli, router);
    }
}

#[test]
fn missing_path_exits_1() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope");
    let home = tempfile::tempdir().unwrap();
    let output = Command::cargo_bin("luc")
        .unwrap()
        .env("HOME", home.path())
        .arg(&missing)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(&format!("path not found: {}", missing.display())),
        "stderr: {stderr}"
    );
}

#[test]
fn prompts_for_folder_only_without_path_at_filesystem_root() {
    use std::path::Path;
    // Finder and the Dock start the app with no arguments in `/`.
    assert!(prompts_for_folder(None, Path::new("/")));
    // A terminal `luc` with no path opens the current directory.
    assert!(!prompts_for_folder(None, Path::new("/Users/me/project")));
    // An explicit path always wins, even `/`.
    assert!(!prompts_for_folder(Some(Path::new("/")), Path::new("/")));
}

/// `luc /dev/zero` is refused before any window: reading it never ends.
#[cfg(unix)]
#[test]
fn resolve_refuses_a_device() {
    let err = resolve_target(Some(PathBuf::from("/dev/zero"))).unwrap_err();
    assert!(err.contains("Not a regular file"), "{err}");
}

/// Tests with a socket this test holds itself as the app: `luc` must meet
/// it without starting an app (no display needed).
#[cfg(unix)]
mod running_app {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::path::Path;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use assert_cmd::Command;
    use lucerna_workbench::cli_socket::{Bound, Claim, CliRequest, claim, socket_path};

    use super::canonical;

    /// Holds the `luc` socket under `home` as the app does.
    fn held_socket(home: &Path) -> Bound {
        let data_dir = home.join("Library/Application Support/Lucerna");
        match claim(&socket_path(&data_dir)) {
            Claim::Server(bound) => bound,
            other => panic!("the test should bind the socket: {other:?}"),
        }
    }

    /// `luc` with `args`; a `luc` that wrongly runs the app is killed at
    /// the timeout and fails the test.
    fn run_luc(home: &Path, cwd: Option<&Path>, args: &[&Path]) -> std::process::Output {
        let mut command = Command::cargo_bin("luc").unwrap();
        command
            .env("HOME", home)
            .timeout(Duration::from_secs(30))
            .args(args);
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        command
            .output()
            .expect("luc exits by itself instead of running the app")
    }

    /// `luc` refused: exit 1, `stderr`, and nothing was sent to the app.
    fn assert_refused(flag: &str, stderr: &str) {
        let home = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let bound = held_socket(home.path());
        let output = run_luc(home.path(), None, &[Path::new(flag), dir.path()]);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert_eq!(String::from_utf8_lossy(&output.stderr), stderr);
        bound.listener.set_nonblocking(true).unwrap();
        while let Ok((stream, _)) = bound.listener.accept() {
            // A probe may connect and leave; it sends no request line.
            // (macOS refuses socket options on a stream the peer closed.)
            let _ = stream.set_nonblocking(false);
            let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
            let mut line = String::new();
            let _ = BufReader::new(&stream).read_line(&mut line);
            assert_eq!(line, "", "luc sent a request");
        }
    }

    #[test]
    fn trace_startup_refuses_when_an_app_runs() {
        assert_refused(
            "--trace-startup",
            "luc: Lucerna is already running; quit it to measure startup.\n",
        );
    }

    #[test]
    fn foreground_refuses_when_an_app_runs() {
        assert_refused(
            "--foreground",
            "luc: Lucerna is already running; quit it to run in the foreground.\n",
        );
    }

    /// Plays the app on `listener`: answers the first request line sent,
    /// within 30 s.
    fn serve_one(listener: UnixListener) -> mpsc::Receiver<CliRequest> {
        let (sent, got) = mpsc::channel();
        listener.set_nonblocking(true).unwrap();
        std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(30);
            while Instant::now() < deadline {
                let Ok((stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut line = String::new();
                if BufReader::new(&stream).read_line(&mut line).unwrap_or(0) == 0 {
                    continue; // a probe
                }
                let request = serde_json::from_str(&line).unwrap();
                (&stream).write_all(b"{\"ok\":true}\n").unwrap();
                let _ = sent.send(request);
                return;
            }
        });
        got
    }

    /// Review Focus 3: the app `luc` started finds another app came up
    /// meanwhile and hands the path to it.
    #[test]
    fn detached_child_hands_off_to_a_running_app() {
        let home = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let got = serve_one(held_socket(home.path()).listener);
        let output = run_luc(
            home.path(),
            None,
            &[Path::new("--detached-child"), dir.path()],
        );
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let request = got.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(request.paths, vec![canonical(dir.path())]);
    }

    /// Review Focus 4: a launch from Finder (no path, in `/`) comes
    /// forward the running app.
    #[test]
    fn finder_launch_activates_a_running_app() {
        let home = tempfile::tempdir().unwrap();
        let got = serve_one(held_socket(home.path()).listener);
        let output = run_luc(home.path(), Some(Path::new("/")), &[]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let request = got.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(request.activate, "{request:?}");
    }
}
