//! Branch listing, switching and branch-name helpers.

use crate::repo::exit_one_is_none;
use crate::{GitError, Repo};
use std::ffi::OsStr;

/// A local branch or a remote-tracking ref.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchRef {
    /// Short form: `main`, `origin/feature/x`.
    pub name: String,
    /// `Some("origin")` for remote-tracking refs.
    pub remote: Option<String>,
    /// Committer date of the tip, as a unix timestamp.
    pub committed: i64,
    /// The branch `HEAD` points at.
    pub is_head: bool,
}

impl BranchRef {
    /// The name without the `remote/` prefix (`feature/x`); `name` for a local
    /// branch.
    pub fn branch_part(&self) -> &str {
        match &self.remote {
            Some(r) => self
                .name
                .strip_prefix(r.as_str())
                .and_then(|s| s.strip_prefix('/'))
                .unwrap_or(&self.name),
            None => &self.name,
        }
    }
}

/// What [`Repo::switch_branch`] switches to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitchTarget {
    /// An existing local branch.
    Local(String),
    /// A remote-tracking branch, e.g. `origin` + `feature/x`. Switches to the
    /// local branch of the same name, creating it to track the remote if needed.
    Remote { remote: String, branch: String },
    /// A new branch at `HEAD`.
    Create(String),
}

/// Trims both ends and replaces each run of whitespace with `-` (VS Code's
/// default `git.branchWhitespaceChar`).
pub fn sanitize_branch_name(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join("-")
}

/// A pure approximation of `git check-ref-format --branch`, for checking a
/// name on every keystroke without running git. It never accepts `@{…}`
/// forms, which git expands instead of rejecting.
pub fn is_valid_branch_name(name: &str) -> bool {
    if name.is_empty() || name == "@" {
        return false;
    }
    if name.starts_with('-') || name.starts_with('/') {
        return false;
    }
    if name.ends_with('/') || name.ends_with('.') || name.ends_with(".lock") {
        return false;
    }
    if name.contains("..") || name.contains("@{") || name.contains("//") {
        return false;
    }
    if name
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c))
    {
        return false;
    }
    // Like git: no component may start with `.` or end with `.lock`.
    name.split('/')
        .all(|c| !c.starts_with('.') && !c.ends_with(".lock"))
}

fn invalid_name() -> GitError {
    GitError::Failed {
        code: None,
        stderr: "Invalid branch name".into(),
    }
}

impl Repo {
    /// Local branches and remote-tracking refs, newest commit first, without
    /// `refs/remotes/*/HEAD`. Remote-tracking refs of a remote that is no
    /// longer configured are left out, since their remote cannot be told apart
    /// from the branch name.
    pub fn branch_list(&self) -> Result<Vec<BranchRef>, GitError> {
        let remotes_out = self.run(&["remote"])?;
        let remotes_out = String::from_utf8_lossy(&remotes_out);
        let remotes: Vec<&str> = remotes_out.lines().filter(|l| !l.is_empty()).collect();
        let out = self.run(&[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname)%00%(committerdate:unix)%00%(HEAD)",
            "refs/heads",
            "refs/remotes",
        ])?;
        let out = String::from_utf8_lossy(&out);
        let mut list = Vec::new();
        for line in out.lines() {
            let mut fields = line.split('\0');
            let (Some(refname), Some(date), Some(head)) =
                (fields.next(), fields.next(), fields.next())
            else {
                return Err(GitError::Parse(format!("for-each-ref: {line:?}")));
            };
            let committed = date.trim().parse().unwrap_or(0);
            let is_head = head == "*";
            if let Some(name) = refname.strip_prefix("refs/heads/") {
                list.push(BranchRef {
                    name: name.to_string(),
                    remote: None,
                    committed,
                    is_head,
                });
            } else if let Some(name) = refname.strip_prefix("refs/remotes/") {
                if name.ends_with("/HEAD") {
                    continue;
                }
                let remote = remotes
                    .iter()
                    .filter(|r| {
                        name.strip_prefix(**r)
                            .is_some_and(|rest| rest.len() > 1 && rest.starts_with('/'))
                    })
                    .max_by_key(|r| r.len());
                let Some(remote) = remote else { continue };
                list.push(BranchRef {
                    name: name.to_string(),
                    remote: Some(remote.to_string()),
                    committed,
                    is_head,
                });
            }
        }
        Ok(list)
    }

    /// `git check-ref-format --branch <name>` succeeds and does not rewrite the
    /// name. Names starting with `-`, and names [`is_valid_branch_name`]
    /// rejects, fail without running git.
    pub fn check_branch_name(&self, name: &str) -> bool {
        if !is_valid_branch_name(name) {
            return false;
        }
        match self.run(&["check-ref-format", "--branch", name]) {
            Ok(out) => String::from_utf8_lossy(&out).trim_end_matches('\n') == name,
            Err(_) => false,
        }
    }

    /// Runs `git switch` with hooks enabled and no timeout; call it off the UI
    /// thread. Every name is checked first: an invalid one returns
    /// `Failed { code: None, stderr: "Invalid branch name" }` without running
    /// git. Names are passed after `--end-of-options` (or as the value of
    /// `-c`), so they can never be read as flags. A remote branch with no
    /// local copy runs `switch -c <branch> --track refs/remotes/<remote>/<branch>`.
    pub fn switch_branch(&self, target: &SwitchTarget) -> Result<(), GitError> {
        let args: Vec<String> = match target {
            SwitchTarget::Local(name) => {
                if !self.check_branch_name(name) {
                    return Err(invalid_name());
                }
                vec!["switch".into(), "--end-of-options".into(), name.clone()]
            }
            SwitchTarget::Remote { remote, branch } => {
                let full = format!("{remote}/{branch}");
                if remote.is_empty()
                    || !self.check_branch_name(branch)
                    || !self.check_branch_name(&full)
                {
                    return Err(invalid_name());
                }
                let local = format!("refs/heads/{branch}");
                let r = self.run(&["show-ref", "--verify", "--quiet", &local]);
                if exit_one_is_none(r)?.is_some() {
                    vec!["switch".into(), "--end-of-options".into(), branch.clone()]
                } else {
                    // Name the local branch and the full ref explicitly: with
                    // `--track` alone git derives the local name by cutting at the
                    // first `/`, which is wrong for remotes like `team/a`.
                    vec![
                        "switch".into(),
                        "-c".into(),
                        branch.clone(),
                        "--track".into(),
                        "--end-of-options".into(),
                        format!("refs/remotes/{full}"),
                    ]
                }
            }
            SwitchTarget::Create(name) => {
                if !self.check_branch_name(name) {
                    return Err(invalid_name());
                }
                vec!["switch".into(), "-c".into(), name.clone()]
            }
        };
        let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
        self.git.run_write(&self.workdir, &args)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize() {
        assert_eq!(sanitize_branch_name("  my  new branch "), "my-new-branch");
        assert_eq!(sanitize_branch_name("x"), "x");
        assert_eq!(sanitize_branch_name(""), "");
    }

    #[test]
    fn components() {
        assert!(!is_valid_branch_name("a/.b"));
        assert!(!is_valid_branch_name("a.lock/b"));
        assert!(is_valid_branch_name("a/b.c"));
    }

    #[test]
    fn branch_part() {
        let b = BranchRef {
            name: "origin/feature/x".into(),
            remote: Some("origin".into()),
            committed: 0,
            is_head: false,
        };
        assert_eq!(b.branch_part(), "feature/x");
    }
}
