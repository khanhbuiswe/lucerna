mod support;

use lucerna_git::{FileStatus, GitError, Repo};
use std::path::{Path, PathBuf};
use support::{bin, git, repo};
use tempfile::TempDir;

fn p(s: &str) -> PathBuf {
    PathBuf::from(s)
}

fn discover(dir: &Path) -> Repo {
    Repo::discover(bin(), dir).unwrap().expect("a repo")
}

fn sorted(mut v: Vec<(PathBuf, FileStatus)>) -> Vec<(PathBuf, FileStatus)> {
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

#[test]
fn discover_none_outside_repo() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().canonicalize().unwrap();
    assert!(Repo::discover(bin(), &dir).unwrap().is_none());
    assert!(matches!(
        Repo::discover(bin(), &dir.join("missing")),
        Ok(None)
    ));
}

#[test]
fn discover_missing_git_is_not_found() {
    let (_t, d) = repo();
    let r = Repo::discover(
        lucerna_git::GitBinary::new(PathBuf::from("/nonexistent/git")),
        &d,
    );
    assert!(matches!(r, Err(GitError::NotFound)));
}

#[test]
fn discover_in_linked_worktree() {
    let (_t, d) = repo();
    let wt_tmp = TempDir::new().unwrap();
    let wt = wt_tmp.path().canonicalize().unwrap().join("wt");
    git(
        &d,
        &["worktree", "add", "-q", "-b", "feat", wt.to_str().unwrap()],
    );
    let r = discover(&wt);
    assert_ne!(r.git_dir, r.common_dir);
    assert_eq!(r.workdir, wt);
    assert_eq!(r.common_dir, d.join(".git"));
    let main = discover(&d);
    assert_eq!(main.git_dir, main.common_dir);
    assert_eq!(main.prefix, PathBuf::new());
}

#[test]
fn workspace_in_subfolder_filters_and_strips_prefix() {
    let (_t, d) = repo();
    std::fs::create_dir_all(d.join("sub")).unwrap();
    std::fs::write(d.join("sub/x.txt"), "x\n").unwrap();
    std::fs::write(d.join("sub/y.txt"), "y\n").unwrap();
    git(&d, &["add", "."]);
    git(&d, &["commit", "-q", "-m", "sub"]);
    std::fs::write(d.join("sub/x.txt"), "x2\n").unwrap();
    std::fs::write(d.join("a.txt"), "outside\n").unwrap();
    std::fs::write(d.join("sub/new.txt"), "n\n").unwrap();

    let r = discover(&d.join("sub"));
    assert_eq!(r.prefix, p("sub"));
    assert_eq!(
        sorted(r.status(None).unwrap()),
        vec![
            (p("new.txt"), FileStatus::Untracked),
            (p("x.txt"), FileStatus::Modified)
        ]
    );
    assert_eq!(
        r.status(Some(&[p("x.txt")])).unwrap(),
        vec![(p("x.txt"), FileStatus::Modified)]
    );
    // Root workspace sees everything.
    assert_eq!(discover(&d).status(None).unwrap().len(), 3);
}

#[test]
fn default_branch_order() {
    let (_t, d) = repo();
    let r = discover(&d);
    assert_eq!(r.default_branch().unwrap().as_deref(), Some("main"));

    let (_t2, d2) = repo();
    git(&d2, &["branch", "-m", "main", "trunk"]);
    assert_eq!(discover(&d2).default_branch().unwrap(), None);
    git(&d2, &["branch", "master"]);
    assert_eq!(
        discover(&d2).default_branch().unwrap().as_deref(),
        Some("master")
    );

    // origin/HEAD wins over local main.
    let (_t3, d3) = repo();
    let clone_tmp = TempDir::new().unwrap();
    let c = clone_tmp.path().canonicalize().unwrap().join("c");
    git(
        &d3,
        &["clone", "-q", d3.to_str().unwrap(), c.to_str().unwrap()],
    );
    assert_eq!(
        discover(&c).default_branch().unwrap().as_deref(),
        Some("origin/main")
    );
}

#[test]
fn merge_base_of_feature_branch() {
    let (_t, d) = repo();
    let base = git(&d, &["rev-parse", "HEAD"]).trim().to_string();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    std::fs::write(d.join("f.txt"), "f\n").unwrap();
    git(&d, &["add", "."]);
    git(&d, &["commit", "-q", "-m", "feat"]);
    let r = discover(&d);
    assert_eq!(r.merge_base("main").unwrap(), Some(base.clone()));
    assert!(matches!(
        r.merge_base("no-such-branch"),
        Err(GitError::Failed { .. })
    ));
    assert_ne!(r.head_oid().unwrap(), Some(base));
}

#[test]
fn head_oid_none_without_commits() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().canonicalize().unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    assert_eq!(discover(&dir).head_oid().unwrap(), None);
}

#[test]
fn changed_since_includes_commits_and_untracked() {
    let (_t, d) = repo();
    let base = git(&d, &["rev-parse", "HEAD"]).trim().to_string();
    git(&d, &["checkout", "-q", "-b", "feat"]);
    std::fs::write(d.join("f.txt"), "f\n").unwrap();
    git(&d, &["add", "."]);
    git(&d, &["commit", "-q", "-m", "feat"]);
    std::fs::write(d.join("a.txt"), "changed\n").unwrap();
    std::fs::write(d.join("u.txt"), "u\n").unwrap();
    let r = discover(&d);
    assert_eq!(
        sorted(r.changed_since(&base).unwrap()),
        vec![
            (p("a.txt"), FileStatus::Modified),
            (p("f.txt"), FileStatus::Added),
            (p("u.txt"), FileStatus::Untracked),
        ]
    );
}

#[test]
fn changed_since_in_limits_to_paths() {
    let (_t, d) = repo();
    let base = git(&d, &["rev-parse", "HEAD"]).trim().to_string();
    std::fs::write(d.join("a.txt"), "changed\n").unwrap();
    std::fs::write(d.join("u.txt"), "u\n").unwrap();
    std::fs::write(d.join("v.txt"), "v\n").unwrap();
    let r = discover(&d);
    let only = [p("a.txt"), p("u.txt")];
    assert_eq!(
        sorted(r.changed_since_in(&base, Some(&only)).unwrap()),
        vec![
            (p("a.txt"), FileStatus::Modified),
            (p("u.txt"), FileStatus::Untracked),
        ]
    );
    assert!(r.changed_since_in(&base, Some(&[])).unwrap().is_empty());
}

#[test]
fn branches_lists_heads_and_remotes_without_head() {
    let (_t, d) = repo();
    git(&d, &["branch", "other"]);
    let mut b = discover(&d).branches().unwrap();
    b.sort();
    assert_eq!(b, vec!["main".to_string(), "other".to_string()]);
}

#[test]
fn read_blob_head_and_missing() {
    let (_t, d) = repo();
    std::fs::create_dir_all(d.join("sub")).unwrap();
    std::fs::write(d.join("sub/x.txt"), "x\r\n").unwrap();
    git(&d, &["add", "."]);
    git(&d, &["commit", "-q", "-m", "sub"]);
    let r = discover(&d);
    assert_eq!(
        r.read_blob("HEAD", Path::new("a.txt")).unwrap(),
        Some(b"one\n".to_vec())
    );
    assert_eq!(r.read_blob("HEAD", Path::new("nope.txt")).unwrap(), None);
    assert_eq!(r.read_blob("nosuchrev", Path::new("a.txt")).unwrap(), None);
    // A tree is not a blob.
    assert_eq!(r.read_blob("HEAD", Path::new("sub")).unwrap(), None);
    // Workspace-relative paths get the prefix.
    let sub = discover(&d.join("sub"));
    assert_eq!(
        sub.read_blob("HEAD", Path::new("x.txt")).unwrap(),
        Some(b"x\r\n".to_vec())
    );
    // Several reads reuse the process, including a large blob.
    let big = "z".repeat(300_000);
    std::fs::write(d.join("big.txt"), &big).unwrap();
    git(&d, &["add", "."]);
    git(&d, &["commit", "-q", "-m", "big"]);
    assert_eq!(
        r.read_blob("HEAD", Path::new("big.txt"))
            .unwrap()
            .unwrap()
            .len(),
        big.len()
    );
    assert_eq!(
        r.read_blob("HEAD", Path::new("a.txt")).unwrap(),
        Some(b"one\n".to_vec())
    );
}

#[test]
fn blob_reader_restarts_after_kill() {
    let (_t, d) = repo();
    let r = discover(&d);
    assert!(r.read_blob("HEAD", Path::new("a.txt")).unwrap().is_some());
    r.kill_blob_process_for_test();
    assert_eq!(
        r.read_blob("HEAD", Path::new("a.txt")).unwrap(),
        Some(b"one\n".to_vec())
    );
}

#[test]
fn repo_is_clone_send_sync() {
    fn assert<T: Clone + Send + Sync>() {}
    assert::<Repo>();
}

#[test]
fn branches_in_clone_skips_origin_head() {
    let (_t, d) = repo();
    git(&d, &["branch", "other"]);
    let clone_tmp = TempDir::new().unwrap();
    let c = clone_tmp.path().canonicalize().unwrap().join("c");
    git(
        &d,
        &["clone", "-q", d.to_str().unwrap(), c.to_str().unwrap()],
    );
    let mut b = discover(&c).branches().unwrap();
    b.sort();
    assert_eq!(
        b,
        vec!["main", "origin/main", "origin/other"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
    );
}

#[test]
fn oversized_blob_is_none_and_reader_still_works() {
    let (_t, d) = repo();
    std::fs::write(d.join("big.txt"), "x".repeat(5000)).unwrap();
    git(&d, &["add", "."]);
    git(&d, &["commit", "-q", "-m", "big"]);
    let r = discover(&d);
    r.set_blob_limits_for_test(1000, std::time::Duration::from_secs(10));
    assert_eq!(r.read_blob("HEAD", Path::new("big.txt")).unwrap(), None);
    assert_eq!(
        r.read_blob("HEAD", Path::new("a.txt")).unwrap(),
        Some(b"one\n".to_vec())
    );
}

#[cfg(unix)]
#[test]
fn hung_cat_file_times_out_and_next_read_restarts() {
    use std::os::unix::fs::PermissionsExt;
    let (_t, d) = repo();
    let script_dir = TempDir::new().unwrap();
    let marker = script_dir.path().join("hang");
    let script = script_dir.path().join("fakegit.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nfor a in \"$@\"; do\n  if [ \"$a\" = cat-file ] && [ -e '{}' ]; then exec sleep 30; fi\ndone\nexec git \"$@\"\n",
            marker.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let bin = lucerna_git::GitBinary::new(script)
        .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
        .with_env("GIT_CONFIG_SYSTEM", "/dev/null");
    let r = Repo::discover(bin, &d).unwrap().unwrap();
    r.set_blob_limits_for_test(1 << 20, std::time::Duration::from_millis(300));
    std::fs::write(&marker, "").unwrap();
    let start = std::time::Instant::now();
    let res = r.read_blob("HEAD", Path::new("a.txt"));
    assert!(matches!(res, Err(GitError::Timeout)), "{res:?}");
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
    std::fs::remove_file(&marker).unwrap();
    assert_eq!(
        r.read_blob("HEAD", Path::new("a.txt")).unwrap(),
        Some(b"one\n".to_vec())
    );
}
