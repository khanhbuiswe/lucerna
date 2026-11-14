//! Turns a path found in terminal output into an existing file or directory
//! (spec §6.7). Blocking: call it off the UI thread.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy)]
pub struct ResolveCtx<'a> {
    /// The foreground process's current directory, when known.
    pub cwd_now: Option<&'a Path>,
    /// The directory the terminal was opened in.
    pub cwd_start: &'a Path,
    pub root: &'a Path,
    pub home: Option<&'a Path>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    File {
        path: PathBuf,
        line: Option<u32>,
        col: Option<u32>,
    },
    Dir(PathBuf),
}

/// Resolution order (spec §6.7): absolute or `~/`, then relative to the
/// foreground cwd, the terminal's start cwd and the workspace root; only if all
/// fail, retry with a git `a/` or `b/` prefix removed.
pub fn resolve_path(
    raw: &str,
    line: Option<u32>,
    col: Option<u32>,
    ctx: &ResolveCtx,
) -> Option<Resolved> {
    if raw.is_empty() {
        return None;
    }
    let found = |p: PathBuf| {
        let meta = std::fs::metadata(&p).ok()?;
        Some(if meta.is_dir() {
            Resolved::Dir(p)
        } else {
            Resolved::File { path: p, line, col }
        })
    };

    if raw == "~" || raw.starts_with("~/") {
        let rest = raw[1..].trim_start_matches('/');
        return found(ctx.home?.join(rest));
    }
    if Path::new(raw).is_absolute() {
        return found(PathBuf::from(raw));
    }

    let bases = [ctx.cwd_now, Some(ctx.cwd_start), Some(ctx.root)];
    let relative = |rel: &str| {
        let mut tried: Vec<&Path> = Vec::new();
        for base in bases.into_iter().flatten() {
            if tried.contains(&base) {
                continue;
            }
            tried.push(base);
            if let Some(r) = found(base.join(rel)) {
                return Some(r);
            }
        }
        None
    };

    relative(raw).or_else(|| {
        let stripped = raw.strip_prefix("a/").or_else(|| raw.strip_prefix("b/"))?;
        if stripped.is_empty() {
            return None;
        }
        relative(stripped)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Fx {
        _t: tempfile::TempDir,
        root: PathBuf,
        now: PathBuf,
        start: PathBuf,
        home: PathBuf,
    }

    fn fx() -> Fx {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().canonicalize().unwrap();
        let [root, now, start, home] = ["root", "now", "start", "home"].map(|n| base.join(n));
        for d in [&root, &now, &start, &home] {
            fs::create_dir_all(d).unwrap();
        }
        Fx {
            _t: t,
            root,
            now,
            start,
            home,
        }
    }

    impl Fx {
        fn ctx(&self) -> ResolveCtx<'_> {
            ResolveCtx {
                cwd_now: Some(&self.now),
                cwd_start: &self.start,
                root: &self.root,
                home: Some(&self.home),
            }
        }
    }

    fn file(path: PathBuf, line: Option<u32>, col: Option<u32>) -> Option<Resolved> {
        Some(Resolved::File { path, line, col })
    }

    #[test]
    fn absolute_path() {
        let f = fx();
        let p = f.root.join("a.rs");
        fs::write(&p, "").unwrap();
        let got = resolve_path(p.to_str().unwrap(), Some(3), Some(4), &f.ctx());
        assert_eq!(got, file(p, Some(3), Some(4)));
    }

    #[test]
    fn tilde_expands() {
        let f = fx();
        let p = f.home.join("n.txt");
        fs::write(&p, "").unwrap();
        assert_eq!(
            resolve_path("~/n.txt", None, None, &f.ctx()),
            file(p, None, None)
        );
        assert_eq!(
            resolve_path("~", None, None, &f.ctx()),
            Some(Resolved::Dir(f.home.clone()))
        );
        assert_eq!(resolve_path("~x/n.txt", None, None, &f.ctx()), None);
        let no_home = ResolveCtx {
            home: None,
            ..f.ctx()
        };
        assert_eq!(resolve_path("~/n.txt", None, None, &no_home), None);
    }

    #[test]
    fn relative_to_cwd_now_first() {
        let f = fx();
        for d in [&f.now, &f.start, &f.root] {
            fs::write(d.join("x.rs"), "").unwrap();
        }
        let got = resolve_path("x.rs", Some(1), None, &f.ctx());
        assert_eq!(got, file(f.now.join("x.rs"), Some(1), None));
    }

    #[test]
    fn falls_back_to_cwd_start_then_root() {
        let f = fx();
        fs::write(f.start.join("s.rs"), "").unwrap();
        fs::write(f.root.join("r.rs"), "").unwrap();
        assert_eq!(
            resolve_path("s.rs", None, None, &f.ctx()),
            file(f.start.join("s.rs"), None, None)
        );
        assert_eq!(
            resolve_path("r.rs", None, None, &f.ctx()),
            file(f.root.join("r.rs"), None, None)
        );
    }

    #[test]
    fn git_ab_prefix_stripped() {
        let f = fx();
        fs::create_dir_all(f.root.join("src")).unwrap();
        fs::write(f.root.join("src/x.rs"), "").unwrap();
        assert_eq!(
            resolve_path("b/src/x.rs", None, None, &f.ctx()),
            file(f.root.join("src/x.rs"), None, None)
        );
        assert_eq!(
            resolve_path("a/src/x.rs", None, None, &f.ctx()),
            file(f.root.join("src/x.rs"), None, None)
        );
    }

    #[test]
    fn plain_attempt_beats_prefix_strip() {
        let f = fx();
        fs::create_dir_all(f.root.join("a/src")).unwrap();
        fs::create_dir_all(f.root.join("src")).unwrap();
        fs::write(f.root.join("a/src/x.rs"), "").unwrap();
        fs::write(f.root.join("src/x.rs"), "").unwrap();
        assert_eq!(
            resolve_path("a/src/x.rs", None, None, &f.ctx()),
            file(f.root.join("a/src/x.rs"), None, None)
        );
    }

    #[test]
    fn directory_resolves_to_dir() {
        let f = fx();
        fs::create_dir_all(f.root.join("pkg/sub")).unwrap();
        assert_eq!(
            resolve_path("pkg/sub/", None, None, &f.ctx()),
            Some(Resolved::Dir(f.root.join("pkg/sub/")))
        );
    }

    #[test]
    fn missing_returns_none() {
        let f = fx();
        assert_eq!(resolve_path("nope/x.rs", None, None, &f.ctx()), None);
        assert_eq!(resolve_path("", None, None, &f.ctx()), None);
        assert_eq!(resolve_path("b/", None, None, &f.ctx()), None);
    }

    #[test]
    fn cwd_now_none_is_skipped() {
        let f = fx();
        fs::write(f.start.join("s.rs"), "").unwrap();
        let ctx = ResolveCtx {
            cwd_now: None,
            ..f.ctx()
        };
        assert_eq!(
            resolve_path("s.rs", None, None, &ctx),
            file(f.start.join("s.rs"), None, None)
        );
    }
}
