use crate::blobs::BlobReader;
use crate::status::{FileStatus, parse_status_v2};
use crate::{GitBinary, GitError};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct Repo {
    /// The worktree root (`--show-toplevel`), canonical.
    pub workdir: PathBuf,
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
    /// The workspace root relative to `workdir` (empty when equal).
    pub prefix: PathBuf,
    pub git: GitBinary,
    blobs: Arc<Mutex<BlobReader>>,
}

fn path_from_line(line: &[u8]) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        PathBuf::from(OsStr::from_bytes(line))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(String::from_utf8_lossy(line).into_owned())
    }
}

fn is_not_a_repo(e: &GitError) -> bool {
    matches!(e, GitError::Failed { stderr, .. } if stderr.contains("not a git repository"))
}

/// `Failed` with this exit code means "no result" for `--verify --quiet` style calls.
fn exit_one_is_none(r: Result<Vec<u8>, GitError>) -> Result<Option<Vec<u8>>, GitError> {
    match r {
        Ok(out) => Ok(Some(out)),
        Err(GitError::Failed { code: Some(1), .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

fn trimmed(out: Vec<u8>) -> String {
    String::from_utf8_lossy(&out).trim().to_string()
}

impl Repo {
    pub fn discover(git: GitBinary, workspace_root: &Path) -> Result<Option<Repo>, GitError> {
        let args = [
            "rev-parse",
            "--show-toplevel",
            "--absolute-git-dir",
            "--git-common-dir",
        ];
        let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
        let out = match git.run(workspace_root, &args) {
            Ok(out) => out,
            Err(e) if is_not_a_repo(&e) => return Ok(None),
            // A missing workspace directory cannot be a repo.
            Err(GitError::Io(_) | GitError::NotFound) if !workspace_root.is_dir() => {
                return Ok(None);
            }
            Err(e) => return Err(e),
        };
        let mut lines = out.split(|&b| b == b'\n').filter(|l| !l.is_empty());
        let (Some(top), Some(gd), Some(cd)) = (lines.next(), lines.next(), lines.next()) else {
            return Err(GitError::Parse("unexpected rev-parse output".into()));
        };
        let canon = |p: PathBuf| p.canonicalize().unwrap_or(p);
        let workdir = canon(path_from_line(top));
        let git_dir = canon(path_from_line(gd));
        let cd = path_from_line(cd);
        let common_dir = canon(if cd.is_absolute() {
            cd
        } else {
            workspace_root.join(cd)
        });
        let root = canon(workspace_root.to_path_buf());
        let prefix = root
            .strip_prefix(&workdir)
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let blobs = Arc::new(Mutex::new(BlobReader::new(git.clone(), workdir.clone())));
        Ok(Some(Repo {
            workdir,
            git_dir,
            common_dir,
            prefix,
            git,
            blobs,
        }))
    }

    fn run(&self, args: &[&str]) -> Result<Vec<u8>, GitError> {
        let args: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
        self.git.run(&self.workdir, &args)
    }

    fn pathspec(path: &Path) -> String {
        format!(":(literal){}", path.to_string_lossy())
    }

    /// Repo-relative entries to workspace-relative; entries outside the prefix are dropped.
    fn strip(&self, entries: Vec<(PathBuf, FileStatus)>) -> Vec<(PathBuf, FileStatus)> {
        entries
            .into_iter()
            .filter_map(|(p, s)| {
                p.strip_prefix(&self.prefix)
                    .ok()
                    .map(|r| (r.to_path_buf(), s))
            })
            .collect()
    }

    /// `paths` are workspace-relative; `None` means the whole workspace.
    pub fn status(
        &self,
        paths: Option<&[PathBuf]>,
    ) -> Result<Vec<(PathBuf, FileStatus)>, GitError> {
        let mut args: Vec<String> = [
            "status",
            "--porcelain=v2",
            "-z",
            "--untracked-files=all",
            "--no-renames",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        match paths {
            Some(paths) if !paths.is_empty() => {
                args.push("--".into());
                args.extend(paths.iter().map(|p| Self::pathspec(&self.prefix.join(p))));
            }
            Some(_) => return Ok(Vec::new()),
            None if !self.prefix.as_os_str().is_empty() => {
                args.push("--".into());
                args.push(Self::pathspec(&self.prefix));
            }
            None => {}
        }
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = self.run(&args)?;
        Ok(self.strip(parse_status_v2(&out)?))
    }

    /// `None` when there is no commit yet.
    pub fn head_oid(&self) -> Result<Option<String>, GitError> {
        let r = self.run(&["rev-parse", "--verify", "--quiet", "HEAD"]);
        Ok(exit_one_is_none(r)?.map(trimmed))
    }

    fn ref_exists(&self, name: &str) -> Result<bool, GitError> {
        let r = self.run(&["rev-parse", "--verify", "--quiet", name]);
        Ok(exit_one_is_none(r)?.is_some())
    }

    /// `upstream/HEAD`, then `origin/HEAD`, then local `main`, then `master`.
    pub fn default_branch(&self) -> Result<Option<String>, GitError> {
        for remote in ["upstream", "origin"] {
            let full = format!("refs/remotes/{remote}/HEAD");
            if self.ref_exists(&full)? {
                let r = self.run(&["symbolic-ref", "--short", &full]);
                if let Some(out) = exit_one_is_none(r)? {
                    return Ok(Some(trimmed(out)));
                }
                // Not symbolic: fall back to the remote name's HEAD ref itself.
                return Ok(Some(format!("{remote}/HEAD")));
            }
        }
        for local in ["main", "master"] {
            if self.ref_exists(&format!("refs/heads/{local}"))? {
                return Ok(Some(local.to_string()));
            }
        }
        Ok(None)
    }

    pub fn merge_base(&self, rev: &str) -> Result<Option<String>, GitError> {
        let r = self.run(&["merge-base", "HEAD", rev]);
        Ok(exit_one_is_none(r)?.map(trimmed))
    }

    /// Files changed in the worktree relative to `base`, plus untracked files.
    pub fn changed_since(&self, base: &str) -> Result<Vec<(PathBuf, FileStatus)>, GitError> {
        let mut args: Vec<String> = ["diff", "--name-status", "-z", "--no-renames", base]
            .iter()
            .map(|s| s.to_string())
            .collect();
        if !self.prefix.as_os_str().is_empty() {
            args.push("--".into());
            args.push(Self::pathspec(&self.prefix));
        }
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = self.run(&argv)?;
        let mut entries = Vec::new();
        let mut recs = out.split(|&b| b == 0).filter(|r| !r.is_empty());
        while let Some(code) = recs.next() {
            let path = recs
                .next()
                .ok_or_else(|| GitError::Parse("diff --name-status: missing path".into()))?;
            let status = match code.first() {
                Some(b'A') => FileStatus::Added,
                Some(b'D') => FileStatus::Deleted,
                Some(b'T') => FileStatus::TypeChanged,
                Some(b'U') => FileStatus::Conflict,
                _ => FileStatus::Modified,
            };
            entries.push((path_from_line(path), status));
        }
        let mut result = self.strip(entries);
        let tracked: std::collections::HashSet<PathBuf> =
            result.iter().map(|(p, _)| p.clone()).collect();
        for (p, s) in self.status(None)? {
            if s == FileStatus::Untracked && !tracked.contains(&p) {
                result.push((p, s));
            }
        }
        Ok(result)
    }

    /// Local and remote branch names, without `*/HEAD`.
    pub fn branches(&self) -> Result<Vec<String>, GitError> {
        let out = self.run(&[
            "for-each-ref",
            "--format=%(refname)",
            "refs/heads",
            "refs/remotes",
        ])?;
        Ok(String::from_utf8_lossy(&out)
            .lines()
            .filter(|l| !(l.starts_with("refs/remotes/") && l.ends_with("/HEAD")))
            .filter_map(|l| {
                l.strip_prefix("refs/heads/")
                    .or_else(|| l.strip_prefix("refs/remotes/"))
            })
            .map(str::to_string)
            .collect())
    }

    /// Contents of `path` (workspace-relative) at `rev`. `None` when it does not
    /// exist there or is not a blob.
    pub fn read_blob(&self, rev: &str, path: &Path) -> Result<Option<Vec<u8>>, GitError> {
        let rel = self.prefix.join(path);
        let rel = rel.to_string_lossy().replace('\\', "/");
        let spec = format!("{rev}:{rel}");
        self.blobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .read(&spec)
    }

    #[doc(hidden)]
    pub fn set_blob_limits_for_test(&self, max_bytes: u64, timeout: std::time::Duration) {
        self.blobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .set_limits_for_test(max_bytes, timeout);
    }

    #[doc(hidden)]
    pub fn kill_blob_process_for_test(&self) {
        self.blobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .kill_for_test();
    }
}
