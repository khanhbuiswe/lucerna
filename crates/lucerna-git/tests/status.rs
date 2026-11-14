mod support;

use lucerna_git::{FileStatus, GitBinary, GitError, parse_status_v2, run_with_timeout};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use support::{bin, git, repo};

fn status(dir: &Path, pathspecs: &[&str]) -> Vec<(PathBuf, FileStatus)> {
    let bin = bin();
    let mut args: Vec<&OsStr> = [
        "status",
        "--porcelain=v2",
        "-z",
        "--untracked-files=all",
        "--no-renames",
    ]
    .iter()
    .map(OsStr::new)
    .collect();
    if !pathspecs.is_empty() {
        args.push(OsStr::new("--"));
        args.extend(pathspecs.iter().map(OsStr::new));
    }
    let out = bin.run(dir, &args).unwrap();
    let mut v = parse_status_v2(&out).unwrap();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

fn p(s: &str) -> PathBuf {
    PathBuf::from(s)
}

#[test]
fn parses_modified_added_deleted_untracked() {
    let (_t, d) = repo();
    std::fs::write(d.join("del.txt"), "x\n").unwrap();
    std::fs::write(d.join("mod.txt"), "x\n").unwrap();
    git(&d, &["add", "."]);
    git(&d, &["commit", "-q", "-m", "more"]);
    std::fs::remove_file(d.join("del.txt")).unwrap();
    std::fs::write(d.join("mod.txt"), "y\n").unwrap();
    std::fs::write(d.join("added.txt"), "n\n").unwrap();
    git(&d, &["add", "added.txt"]);
    std::fs::write(d.join("untracked.txt"), "u\n").unwrap();
    assert_eq!(
        status(&d, &[]),
        vec![
            (p("added.txt"), FileStatus::Added),
            (p("del.txt"), FileStatus::Deleted),
            (p("mod.txt"), FileStatus::Modified),
            (p("untracked.txt"), FileStatus::Untracked),
        ]
    );
}

#[test]
fn parses_conflict() {
    let (_t, d) = repo();
    git(&d, &["checkout", "-q", "-b", "other"]);
    std::fs::write(d.join("a.txt"), "other\n").unwrap();
    git(&d, &["commit", "-q", "-am", "other"]);
    git(&d, &["checkout", "-q", "main"]);
    std::fs::write(d.join("a.txt"), "main\n").unwrap();
    git(&d, &["commit", "-q", "-am", "main"]);
    git(&d, &["merge", "other"]);
    assert_eq!(status(&d, &[]), vec![(p("a.txt"), FileStatus::Conflict)]);
}

#[test]
fn parses_unicode_and_space_paths() {
    let (_t, d) = repo();
    std::fs::create_dir_all(d.join("dir with space")).unwrap();
    std::fs::write(d.join("dir with space/é ü.txt"), "x").unwrap();
    std::fs::write(d.join("tab\tname.txt"), "x").unwrap();
    assert_eq!(
        status(&d, &[]),
        vec![
            (p("dir with space/é ü.txt"), FileStatus::Untracked),
            (p("tab\tname.txt"), FileStatus::Untracked),
        ]
    );
}

#[test]
fn pathspec_limits_output() {
    let (_t, d) = repo();
    std::fs::create_dir_all(d.join("sub")).unwrap();
    std::fs::write(d.join("sub/x.txt"), "x").unwrap();
    std::fs::write(d.join("other.txt"), "x").unwrap();
    assert_eq!(
        status(&d, &["sub/"]),
        vec![(p("sub/x.txt"), FileStatus::Untracked)]
    );
}

#[cfg(unix)]
#[test]
fn hooks_and_fsmonitor_never_run() {
    let (_t, d) = repo();
    let ran = d.join("ran.txt");
    let hooks = d.join("myhooks");
    std::fs::create_dir_all(&hooks).unwrap();
    let script = |name: &str| {
        let path = hooks.join(name);
        std::fs::write(&path, format!("#!/bin/sh\ntouch '{}'\n", ran.display())).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    };
    let fsm = script("fsm.sh");
    script("post-index-change");
    git(&d, &["config", "core.fsmonitor", fsm.to_str().unwrap()]);
    git(&d, &["config", "core.hooksPath", hooks.to_str().unwrap()]);
    std::fs::write(d.join("a.txt"), "changed\n").unwrap();
    let _ = status(&d, &[]);
    assert!(!ran.exists(), "a hook or fsmonitor ran");
}

#[cfg(unix)]
#[test]
fn timeout_kills_process() {
    let start = Instant::now();
    let mut cmd = Command::new("sleep");
    cmd.arg("30");
    let r = run_with_timeout(cmd, Duration::from_millis(200));
    assert!(matches!(r, Err(GitError::Timeout)), "{r:?}");
    assert!(start.elapsed() < Duration::from_secs(1));
}

#[test]
fn nonzero_exit_is_failed_and_missing_binary_is_not_found() {
    let (_t, d) = repo();
    let bin = bin();
    let r = bin.run(&d, &[OsStr::new("nonsense-subcommand")]);
    assert!(matches!(r, Err(GitError::Failed { .. })), "{r:?}");
    let r = GitBinary::new(PathBuf::from("/nonexistent/git")).run(&d, &[OsStr::new("status")]);
    assert!(matches!(r, Err(GitError::NotFound)), "{r:?}");
}
