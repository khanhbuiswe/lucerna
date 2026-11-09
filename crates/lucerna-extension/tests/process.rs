use lucerna_extension::git::{fetch_commit, normalize_url};
use lucerna_extension::process::{CancelToken, ProcessError, run};
use lucerna_extension::test_support::fake_repo;
use std::process::Command;
use std::time::{Duration, Instant};

fn pid_alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

#[test]
fn run_times_out_and_kills() {
    let dir = tempfile::tempdir().unwrap();
    let pidfile = dir.path().join("pid");
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(format!("echo $$ > {}; sleep 5", pidfile.display()));
    let started = Instant::now();
    let mut log = Vec::new();
    let err = run(
        cmd,
        Duration::from_millis(200),
        &CancelToken::default(),
        &mut log,
    )
    .unwrap_err();
    assert!(matches!(err, ProcessError::TimedOut(_)), "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(3));
    let pid: i32 = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(!pid_alive(pid));
}

#[test]
fn run_cancel_kills() {
    let cancel = CancelToken::default();
    let c2 = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        c2.cancel();
    });
    let mut cmd = Command::new("sleep");
    cmd.arg("5");
    let started = Instant::now();
    let err = run(cmd, Duration::from_secs(30), &cancel, &mut Vec::new()).unwrap_err();
    assert!(matches!(err, ProcessError::Cancelled), "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn run_logs_command_and_output_and_reports_failure() {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg("echo out; echo err >&2; exit 3");
    let mut log = Vec::new();
    let err = run(
        cmd,
        Duration::from_secs(5),
        &CancelToken::default(),
        &mut log,
    )
    .unwrap_err();
    match err {
        ProcessError::Failed { status, stderr, .. } => {
            assert_eq!(status, 3);
            assert!(stderr.contains("err"));
        }
        e => panic!("{e:?}"),
    }
    let log = String::from_utf8(log).unwrap();
    assert!(log.contains("sh -c"), "{log}");
    assert!(log.contains("out") && log.contains("err"), "{log}");
}

#[test]
fn run_missing_program_is_not_found() {
    let cmd = Command::new("definitely-not-a-program-xyz");
    let err = run(
        cmd,
        Duration::from_secs(5),
        &CancelToken::default(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(matches!(err, ProcessError::NotFound(_)), "{err:?}");
}

#[test]
fn normalize_ssh_github_url() {
    assert_eq!(
        normalize_url("git@github.com:owner/repo.git"),
        "https://github.com/owner/repo"
    );
    assert_eq!(
        normalize_url("git@github.com:owner/repo"),
        "https://github.com/owner/repo"
    );
    assert_eq!(
        normalize_url("https://github.com/o/r.git"),
        "https://github.com/o/r.git"
    );
}

#[test]
fn fetch_commit_idempotent() {
    let (repo, commit) = fake_repo(&[("extension.toml", "id = \"x\"\n"), ("a/b.txt", "hi")]);
    let url = format!("file://{}", repo.display());
    let dest = tempfile::tempdir().unwrap().keep().join("src");
    let cancel = CancelToken::default();
    let mut log = Vec::new();
    fetch_commit(&url, &commit, &dest, &cancel, &mut log).unwrap();
    assert_eq!(std::fs::read_to_string(dest.join("a/b.txt")).unwrap(), "hi");
    // Second call is a no-op: works even when the remote is gone.
    std::fs::remove_dir_all(&repo).unwrap();
    fetch_commit(&url, &commit, &dest, &cancel, &mut log).unwrap();
    assert!(dest.join("extension.toml").exists());
}

#[test]
fn fetch_nonexistent_url_fails_fast_without_prompt() {
    let dest = tempfile::tempdir().unwrap();
    let started = Instant::now();
    let err = fetch_commit(
        "https://127.0.0.1:9/none.git",
        "0123456789012345678901234567890123456789",
        &dest.path().join("src"),
        &CancelToken::default(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(
        matches!(err, ProcessError::Failed { .. } | ProcessError::TimedOut(_)),
        "{err:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(30));
}
