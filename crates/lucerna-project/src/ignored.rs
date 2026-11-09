//! Single-path ignore check with the same policy as the scan walker.
//!
//! The walker (`worktree::walk_builder`) only decides about paths it
//! discovers while descending. For a path reported by the watcher we need the
//! same answer without a walk, so this module consults the same sources:
//! `.ignore`, `.gitignore` and `.git/info/exclude` in every directory from the
//! filesystem root down to the path's parent (the walker's `parents(true)`),
//! deepest directory first, plus the `files.exclude` overrides. A path is
//! ignored when any of its ancestors (or itself) is, because the walker never
//! descends into an ignored directory.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ignore::Match;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use ignore::overrides::{Override, OverrideBuilder};

use crate::worktree::{GIT_DIR, ScanOptions};

/// Per-directory matchers in the walker's precedence order.
struct DirRules(Vec<Gitignore>);

/// Caches parsed ignore files, so create one per batch of paths.
pub(crate) struct IgnoreChecker {
    root: PathBuf,
    overrides: Option<Override>,
    dirs: HashMap<PathBuf, DirRules>,
}

impl IgnoreChecker {
    pub(crate) fn new(root: &Path, opts: &ScanOptions) -> Self {
        let mut builder = OverrideBuilder::new(root);
        for glob in &opts.exclude {
            // Invalid globs were already reported when the scan built them.
            let _ = builder.add(&format!("!{glob}"));
        }
        Self {
            root: root.to_path_buf(),
            overrides: builder.build().ok(),
            dirs: HashMap::new(),
        }
    }

    /// Whether the root-relative `rel` is excluded from the worktree.
    /// Anything named `.git`, and everything below it, always is (see
    /// [`crate::worktree::walk_builder`]).
    pub(crate) fn is_ignored(&mut self, rel: &Path, is_dir: bool) -> bool {
        if rel.components().any(|c| c.as_os_str() == GIT_DIR) {
            return true;
        }
        let total = rel.components().count();
        let mut prefix = PathBuf::new();
        for (i, component) in rel.components().enumerate() {
            prefix.push(component);
            let dir = is_dir || i + 1 < total;
            if self.single(&prefix, dir) {
                return true;
            }
        }
        false
    }

    fn single(&mut self, rel: &Path, is_dir: bool) -> bool {
        if let Some(overrides) = &self.overrides
            && overrides.matched(rel, is_dir).is_ignore()
        {
            return true;
        }
        let abs = self.root.join(rel);
        let mut dir = abs.parent();
        while let Some(d) = dir {
            let rules = self
                .dirs
                .entry(d.to_path_buf())
                .or_insert_with(|| load_rules(d));
            for matcher in &rules.0 {
                match matcher.matched(&abs, is_dir) {
                    Match::Ignore(_) => return true,
                    Match::Whitelist(_) => return false,
                    Match::None => {}
                }
            }
            dir = d.parent();
        }
        false
    }
}

fn load_rules(dir: &Path) -> DirRules {
    let sources = [".ignore", ".gitignore", ".git/info/exclude"];
    let matchers = sources
        .iter()
        .filter_map(|name| {
            let file = dir.join(name);
            if !file.is_file() {
                return None;
            }
            let mut builder = GitignoreBuilder::new(dir);
            // A malformed line is skipped by the builder, as in the walker.
            let _ = builder.add(file);
            builder.build().ok()
        })
        .collect();
    DirRules(matchers)
}

/// Files whose change alters what the walker would include.
pub(crate) fn is_ignore_source(rel: &Path) -> bool {
    let name = rel.file_name().and_then(|n| n.to_str());
    matches!(name, Some(".gitignore" | ".ignore")) || rel.ends_with(".git/info/exclude")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktree::walk_builder;
    use std::collections::HashSet;
    use std::fs;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }

    fn all_paths(dir: &Path, root: &Path, out: &mut Vec<(PathBuf, bool)>) {
        for e in fs::read_dir(dir).unwrap() {
            let e = e.unwrap();
            let is_dir = e.file_type().unwrap().is_dir();
            out.push((e.path().strip_prefix(root).unwrap().to_path_buf(), is_dir));
            if is_dir {
                all_paths(&e.path(), root, out);
            }
        }
    }

    /// The checker must agree with the walker on every path of a tree that
    /// exercises nested files, negation, a parent .gitignore, `.ignore`,
    /// `.git/info/exclude` and exclude globs.
    #[test]
    fn agrees_with_the_walker() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("proj");
        write(tmp.path(), ".gitignore", "gen/\n");
        write(&root, ".gitignore", "out/\n*.log\n!keep.log\n/rooted.txt\n");
        write(&root, ".ignore", "scratch/\n");
        write(&root, ".git/info/exclude", "local.cfg\n");
        write(&root, "sub/.gitignore", "!*.log\nonly_here/\n");
        for f in [
            "a.rs",
            "out/x",
            "out/deep/y",
            "t.log",
            "keep.log",
            "rooted.txt",
            "sub/rooted.txt",
            "sub/t.log",
            "sub/only_here/z",
            "only_here/z",
            "scratch/s",
            "local.cfg",
            "sub/local.cfg",
            "gen/g",
            "src/node_modules/m",
            "src/lib.rs",
            ".git/config",
            "docs/a.md",
        ] {
            write(&root, f, "x");
        }
        let opts = ScanOptions {
            exclude: vec![
                "**/node_modules".into(),
                "**/.git".into(),
                "docs/*.md".into(),
            ],
        };
        let walked: HashSet<PathBuf> = walk_builder(&root, &opts)
            .build()
            .filter_map(Result::ok)
            .filter_map(|e| e.path().strip_prefix(&root).ok().map(Path::to_path_buf))
            .collect();
        let mut on_disk = Vec::new();
        all_paths(&root, &root, &mut on_disk);
        let mut checker = IgnoreChecker::new(&root, &opts);
        for (rel, is_dir) in on_disk {
            assert_eq!(
                checker.is_ignored(&rel, is_dir),
                !walked.contains(&rel),
                "disagreement on {}",
                rel.display()
            );
        }
    }

    /// `.git` is out of the tree whatever the exclude globs say.
    #[test]
    fn git_dir_always_ignored() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut checker = IgnoreChecker::new(tmp.path(), &ScanOptions::default());
        assert!(checker.is_ignored(Path::new(".git"), true));
        assert!(checker.is_ignored(Path::new(".git/config"), false));
        assert!(checker.is_ignored(Path::new("sub/.git/HEAD"), false));
        assert!(!checker.is_ignored(Path::new("a.git"), false));
        assert!(!checker.is_ignored(Path::new("src/main.rs"), false));
    }

    #[test]
    fn ignore_sources_recognised() {
        assert!(is_ignore_source(Path::new(".gitignore")));
        assert!(is_ignore_source(Path::new("a/.ignore")));
        assert!(is_ignore_source(Path::new(".git/info/exclude")));
        assert!(!is_ignore_source(Path::new("a/b.rs")));
    }
}
