mod support;

use lucerna_git::{GitError, Repo, SwitchTarget, is_valid_branch_name, sanitize_branch_name};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use support::{bin, git, repo};
use tempfile::TempDir;

fn discover(dir: &Path) -> Repo {
    Repo::discover(bin(), dir).unwrap().expect("a repo")
}

fn head(dir: &Path) -> String {
    git(dir, &["symbolic-ref", "--short", "HEAD"])
        .trim()
        .to_string()
}

/// Commits an empty change with a fixed committer date (unix seconds).
fn commit_at(dir: &Path, msg: &str, unix: i64) {
    let date = format!("@{unix} +0000");
    let out = std::process::Command::new("git")
        .current_dir(dir)
        .args(["commit", "-q", "--allow-empty", "-m", msg])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_COMMITTER_DATE", &date)
        .env("GIT_AUTHOR_DATE", &date)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A bare remote cloned from `src`, plus a clone of that remote.
fn clone_of(src: &Path) -> (TempDir, PathBuf, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let bare = root.join("remote.git");
    let work = root.join("work");
    git(
        &root,
        &[
            "clone",
            "-q",
            "--bare",
            src.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );
    git(
        &root,
        &[
            "clone",
            "-q",
            bare.to_str().unwrap(),
            work.to_str().unwrap(),
        ],
    );
    git(&work, &["config", "user.name", "Test"]);
    git(&work, &["config", "user.email", "test@example.com"]);
    git(&work, &["config", "commit.gpgsign", "false"]);
    (tmp, bare, work)
}

fn local_branches(dir: &Path) -> Vec<String> {
    git(
        dir,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
    )
    .lines()
    .map(str::to_string)
    .collect()
}

#[test]
fn branch_list_orders_and_marks_head() {
    let (_t, src) = repo();
    commit_at(&src, "m", 1_000_000);
    git(&src, &["branch", "old"]);
    git(&src, &["switch", "-q", "-c", "mid"]);
    commit_at(&src, "mid", 2_000_000);
    git(&src, &["switch", "-q", "-c", "new"]);
    commit_at(&src, "new", 3_000_000);
    git(&src, &["switch", "-q", "main"]);
    commit_at(&src, "main", 1_500_000);
    git(&src, &["switch", "-q", "mid"]);
    let (_c, _bare, work) = clone_of(&src);
    git(&work, &["branch", "old", "origin/old"]);
    git(&work, &["branch", "new", "origin/new"]);

    let list = discover(&work).branch_list().unwrap();
    let names: Vec<&str> = list.iter().map(|b| b.name.as_str()).collect();
    assert!(!names.iter().any(|n| n.ends_with("/HEAD")), "{names:?}");
    let locals: Vec<&str> = list
        .iter()
        .filter(|b| b.remote.is_none())
        .map(|b| b.name.as_str())
        .collect();
    // The clone checks out the remote HEAD (mid).
    assert_eq!(locals, ["new", "mid", "old"]);
    let committed: Vec<i64> = list.iter().map(|b| b.committed).collect();
    let mut sorted = committed.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(committed, sorted);
    assert_eq!(
        list.iter().find(|b| b.name == "new").unwrap().committed,
        3_000_000
    );
    let heads: Vec<&str> = list
        .iter()
        .filter(|b| b.is_head)
        .map(|b| b.name.as_str())
        .collect();
    assert_eq!(heads, ["mid"]);
    assert!(names.contains(&"origin/main"));
}

#[test]
fn branch_list_splits_remotes() {
    let (_t, src) = repo();
    git(&src, &["branch", "feature/x"]);
    let (_c, _bare, work) = clone_of(&src);
    // A second remote with the same branches.
    let (_c2, bare2, _w2) = clone_of(&src);
    git(
        &work,
        &["remote", "add", "upstream", bare2.to_str().unwrap()],
    );
    git(&work, &["fetch", "-q", "upstream"]);

    let list = discover(&work).branch_list().unwrap();
    let ox = list.iter().find(|b| b.name == "origin/feature/x").unwrap();
    assert_eq!(ox.remote.as_deref(), Some("origin"));
    assert_eq!(ox.branch_part(), "feature/x");
    let ux = list
        .iter()
        .find(|b| b.name == "upstream/feature/x")
        .unwrap();
    assert_eq!(ux.remote.as_deref(), Some("upstream"));
    assert_eq!(ux.branch_part(), "feature/x");
    let main = list.iter().find(|b| b.name == "main").unwrap();
    assert_eq!(main.remote, None);
    assert_eq!(main.branch_part(), "main");
    assert!(main.is_head);
}

#[test]
fn branch_list_uses_longest_matching_remote() {
    let (_t, src) = repo();
    git(&src, &["branch", "x"]);
    let (_c, bare, work) = clone_of(&src);
    // Remote `origin/sub` makes `refs/remotes/origin/sub/x` ambiguous with a
    // branch `sub/x` of `origin`; the longest remote wins.
    git(
        &work,
        &["remote", "add", "origin/sub", bare.to_str().unwrap()],
    );
    git(&work, &["fetch", "-q", "origin/sub"]);
    let list = discover(&work).branch_list().unwrap();
    let b = list.iter().find(|b| b.name == "origin/sub/x").unwrap();
    assert_eq!(b.remote.as_deref(), Some("origin/sub"));
    assert_eq!(b.branch_part(), "x");
}

#[test]
fn switch_local() {
    let (_t, d) = repo();
    git(&d, &["branch", "feat"]);
    discover(&d)
        .switch_branch(&SwitchTarget::Local("feat".into()))
        .unwrap();
    assert_eq!(head(&d), "feat");
}

#[test]
fn switch_create() {
    let (_t, d) = repo();
    discover(&d)
        .switch_branch(&SwitchTarget::Create("new/one".into()))
        .unwrap();
    assert_eq!(head(&d), "new/one");
}

#[test]
fn remote_branch_with_slashes_tracks_right_remote() {
    let (_t, src) = repo();
    git(&src, &["branch", "feature/x"]);
    let (_c, _bare, work) = clone_of(&src);
    assert!(!local_branches(&work).contains(&"feature/x".to_string()));
    discover(&work)
        .switch_branch(&SwitchTarget::Remote {
            remote: "origin".into(),
            branch: "feature/x".into(),
        })
        .unwrap();
    assert_eq!(head(&work), "feature/x");
    assert_eq!(
        git(&work, &["config", "branch.feature/x.remote"]).trim(),
        "origin"
    );
}

#[test]
fn remote_branch_with_existing_local_switches_to_it() {
    let (_t, src) = repo();
    git(&src, &["branch", "feat"]);
    let (_c, _bare, work) = clone_of(&src);
    git(&work, &["branch", "feat", "origin/feat"]);
    git(&work, &["switch", "-q", "feat"]);
    commit_at(&work, "local only", 2_000_000);
    let local_tip = git(&work, &["rev-parse", "feat"]);
    git(&work, &["switch", "-q", "main"]);
    discover(&work)
        .switch_branch(&SwitchTarget::Remote {
            remote: "origin".into(),
            branch: "feat".into(),
        })
        .unwrap();
    assert_eq!(head(&work), "feat");
    assert_eq!(git(&work, &["rev-parse", "HEAD"]), local_tip);
}

#[test]
fn conflicting_changes_fail_and_keep_repo() {
    let (_t, d) = repo();
    git(&d, &["switch", "-q", "-c", "other"]);
    std::fs::write(d.join("a.txt"), "other\n").unwrap();
    git(&d, &["commit", "-q", "-am", "other"]);
    git(&d, &["switch", "-q", "main"]);
    std::fs::write(d.join("a.txt"), "dirty\n").unwrap();
    let r = discover(&d).switch_branch(&SwitchTarget::Local("other".into()));
    match r {
        Err(GitError::Failed { stderr, .. }) => {
            assert!(stderr.contains("would be overwritten"), "{stderr}")
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    assert_eq!(head(&d), "main");
    assert_eq!(std::fs::read_to_string(d.join("a.txt")).unwrap(), "dirty\n");
}

#[test]
fn non_conflicting_changes_carry_over() {
    let (_t, d) = repo();
    git(&d, &["branch", "other"]);
    std::fs::write(d.join("a.txt"), "dirty\n").unwrap();
    discover(&d)
        .switch_branch(&SwitchTarget::Local("other".into()))
        .unwrap();
    assert_eq!(head(&d), "other");
    assert_eq!(std::fs::read_to_string(d.join("a.txt")).unwrap(), "dirty\n");
}

#[cfg(unix)]
#[test]
fn switch_runs_post_checkout_hook_but_reads_do_not() {
    use std::os::unix::fs::PermissionsExt;
    let (_t, d) = repo();
    git(&d, &["branch", "other"]);
    let hook = d.join(".git/hooks/post-checkout");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(&hook, "#!/bin/sh\ntouch ran.txt\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let r = discover(&d);
    r.status(None).unwrap();
    assert!(!d.join("ran.txt").exists());
    r.switch_branch(&SwitchTarget::Local("other".into()))
        .unwrap();
    assert!(d.join("ran.txt").exists());
}

#[test]
fn switch_rejects_option_like_names_safely() {
    let (_t, d) = repo();
    let r = discover(&d);
    let before = local_branches(&d);
    for target in [
        SwitchTarget::Create("-x".into()),
        SwitchTarget::Local("--orphan".into()),
        SwitchTarget::Local("-".into()),
        SwitchTarget::Remote {
            remote: "origin".into(),
            branch: "--detach".into(),
        },
        SwitchTarget::Remote {
            remote: "-x".into(),
            branch: "y".into(),
        },
    ] {
        match r.switch_branch(&target) {
            Err(GitError::Failed { code: None, stderr }) => {
                assert_eq!(stderr, "Invalid branch name")
            }
            other => panic!("{target:?}: expected invalid name, got {other:?}"),
        }
    }
    assert_eq!(head(&d), "main");
    assert_eq!(local_branches(&d), before);
}

#[test]
fn sanitize_and_check() {
    assert_eq!(sanitize_branch_name("  my  new branch "), "my-new-branch");
    assert_eq!(sanitize_branch_name("a\t\nb"), "a-b");
    assert_eq!(sanitize_branch_name("   "), "");
    let (_t, d) = repo();
    let r = discover(&d);
    for bad in ["", "a..b", "-x", "a b", "@{-1}", "@"] {
        assert!(!r.check_branch_name(bad), "{bad:?} should be invalid");
    }
    for good in ["feature/x", "main", "fix-1.2"] {
        assert!(r.check_branch_name(good), "{good:?} should be valid");
    }
}

#[test]
fn pure_name_check() {
    for bad in [
        "", "a b", "a\tb", "a..b", "a~b", "a^b", "a:b", "a?b", "a*b", "a[b", "a\\b", "a\u{7f}b",
        "a\u{1}b", "-x", "/x", "x/", "x.lock", "x.", "a@{b", "a//b", "@",
    ] {
        assert!(!is_valid_branch_name(bad), "{bad:?} should be invalid");
    }
    for good in ["main", "feature/x", "fix-1.2", "a@b", "@x", "v1.0", "ünï"] {
        assert!(is_valid_branch_name(good), "{good:?} should be valid");
    }
}

#[test]
fn write_command_is_not_hook_disabled() {
    let cmd = bin().command_for_write(Path::new("."));
    let args: Vec<&OsStr> = cmd.get_args().collect();
    let has = |s: &str| args.contains(&OsStr::new(s));
    assert!(
        !args
            .iter()
            .any(|a| a.to_string_lossy().contains("core.hooksPath"))
    );
    assert!(!has("--no-optional-locks"));
    assert!(has("core.fsmonitor=false"));
    assert!(has("--no-pager"));
    let envs: Vec<_> = cmd.get_envs().collect();
    assert!(envs.contains(&(OsStr::new("GIT_TERMINAL_PROMPT"), Some(OsStr::new("0")))));
    assert!(envs.contains(&(OsStr::new("LC_ALL"), Some(OsStr::new("C")))));
}

#[test]
fn run_write_reports_failure() {
    let (_t, d) = repo();
    let r = bin().run_write(&d, &[OsStr::new("switch"), OsStr::new("nope")]);
    assert!(
        matches!(r, Err(GitError::Failed { code: Some(_), .. })),
        "{r:?}"
    );
    let out = bin()
        .run_write(
            &d,
            &[
                OsStr::new("rev-parse"),
                OsStr::new("--abbrev-ref"),
                OsStr::new("HEAD"),
            ],
        )
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out).trim(), "main");
}
