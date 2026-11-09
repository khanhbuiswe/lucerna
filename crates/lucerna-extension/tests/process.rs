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
    let sleeppid = dir.path().join("sleeppid");
    cmd.arg("-c").arg(format!(
        "echo $$ > {}; sleep 5 & echo $! > {}; wait",
        pidfile.display(),
        sleeppid.display()
    ));
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
    let sp: i32 = std::fs::read_to_string(&sleeppid)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert!(!pid_alive(sp), "grandchild survived");
}

#[test]
fn run_returns_promptly_despite_background_grandchild() {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg("sleep 30 & echo hi");
    let started = Instant::now();
    let out = run(
        cmd,
        Duration::from_secs(60),
        &CancelToken::default(),
        &mut Vec::new(),
    )
    .unwrap();
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hi");
}

#[test]
fn fetch_commit_wipes_stale_files_on_version_change() {
    let (repo, a) = fake_repo(&[("x.txt", "x"), ("keep.txt", "k")]);
    let url = format!("file://{}", repo.display());
    let dest = tempfile::tempdir().unwrap().keep().join("src");
    let cancel = CancelToken::default();
    fetch_commit(&url, &a, &dest, true, &cancel, &mut Vec::new()).unwrap();
    std::fs::write(dest.join("untracked.bin"), "u").unwrap();
    std::fs::remove_file(repo.join("x.txt")).unwrap();
    let b = lucerna_extension::test_support::add_commit(&repo, &[("new.txt", "n")]);
    // add_commit uses `add -A`, so x.txt is removed in commit b.
    fetch_commit(&url, &b, &dest, true, &cancel, &mut Vec::new()).unwrap();
    assert!(!dest.join("x.txt").exists());
    assert!(!dest.join("untracked.bin").exists());
    assert!(dest.join("new.txt").exists());
}

#[test]
fn fetch_commit_heals_broken_git_dir() {
    let (repo, c) = fake_repo(&[("f.txt", "f")]);
    let url = format!("file://{}", repo.display());
    let dest = tempfile::tempdir().unwrap().keep().join("src");
    std::fs::create_dir_all(dest.join(".git")).unwrap();
    std::fs::write(dest.join(".git/HEAD"), "").unwrap();
    fetch_commit(
        &url,
        &c,
        &dest,
        true,
        &CancelToken::default(),
        &mut Vec::new(),
    )
    .unwrap();
    assert!(dest.join("f.txt").exists());
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
    fetch_commit(&url, &commit, &dest, true, &cancel, &mut log).unwrap();
    assert_eq!(std::fs::read_to_string(dest.join("a/b.txt")).unwrap(), "hi");
    // Second call is a no-op: works even when the remote is gone.
    std::fs::remove_dir_all(&repo).unwrap();
    fetch_commit(&url, &commit, &dest, true, &cancel, &mut log).unwrap();
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
        false,
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

#[test]
fn fetch_commit_rejects_option_like_commit() {
    let (repo, _) = fake_repo(&[("f.txt", "f")]);
    let url = format!("file://{}", repo.display());
    let tmp = tempfile::tempdir().unwrap();
    let marker = tmp.path().join("PWNED");
    let commit = format!("--upload-pack=touch {}; git-upload-pack", marker.display());
    let r = fetch_commit(
        &url,
        &commit,
        &tmp.path().join("src"),
        true,
        &CancelToken::default(),
        &mut Vec::new(),
    );
    assert!(r.is_err(), "{r:?}");
    assert!(!marker.exists(), "the injected command ran");
}

#[test]
fn fetch_commit_rejects_option_like_url() {
    let tmp = tempfile::tempdir().unwrap();
    let marker = tmp.path().join("PWNED");
    let url = format!("--upload-pack=touch {}; git-upload-pack", marker.display());
    let r = fetch_commit(
        &url,
        "0123456789012345678901234567890123456789",
        &tmp.path().join("src"),
        true,
        &CancelToken::default(),
        &mut Vec::new(),
    );
    assert!(r.is_err(), "{r:?}");
    assert!(!marker.exists(), "the injected command ran");
}

#[test]
fn fetch_commit_rejects_file_url_without_opt_in() {
    let (repo, commit) = fake_repo(&[("f.txt", "f")]);
    let dest = tempfile::tempdir().unwrap().keep().join("src");
    for url in [
        format!("file://{}", repo.display()),
        repo.display().to_string(),
    ] {
        let r = fetch_commit(
            &url,
            &commit,
            &dest,
            false,
            &CancelToken::default(),
            &mut Vec::new(),
        );
        assert!(r.is_err(), "{url}: {r:?}");
        assert!(!dest.join("f.txt").exists(), "{url}");
    }
}

#[test]
fn fetch_commit_uppercase_commit_is_idempotent() {
    let (repo, commit) = fake_repo(&[("f.txt", "f")]);
    let url = format!("file://{}", repo.display());
    let dest = tempfile::tempdir().unwrap().keep().join("src");
    let upper = commit.to_ascii_uppercase();
    let cancel = CancelToken::default();
    fetch_commit(&url, &upper, &dest, true, &cancel, &mut Vec::new()).unwrap();
    // Already there: no fetch, so it works with the remote gone.
    std::fs::remove_dir_all(&repo).unwrap();
    fetch_commit(&url, &upper, &dest, true, &cancel, &mut Vec::new()).unwrap();
    assert!(dest.join("f.txt").exists());
}
