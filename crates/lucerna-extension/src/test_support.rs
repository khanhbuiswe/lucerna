//! Helpers for tests (also used by the workbench's tests): real git
//! repositories and registries in temp dirs, reachable through `file://` URLs.
//! Directories are intentionally leaked into the OS temp dir.

use std::path::{Path, PathBuf};
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args([
            "-c",
            "protocol.file.allow=always",
            "-c",
            "commit.gpgsign=false",
        ])
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write_files(dir: &Path, files: &[(&str, &str)]) {
    for (rel, content) in files {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }
}

fn new_dir() -> PathBuf {
    tempfile::tempdir().unwrap().keep()
}

/// `file://` URL for a local path.
pub fn file_url(path: &Path) -> String {
    format!("file://{}", path.display())
}

/// A git repository (branch `main`) with one commit holding `files`.
pub fn fake_repo(files: &[(&str, &str)]) -> (PathBuf, String) {
    let dir = new_dir().join("repo");
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    write_files(&dir, files);
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "initial"]);
    let commit = git(&dir, &["rev-parse", "HEAD"]);
    (dir, commit)
}

/// Adds a commit with `files` to an existing repo and returns its sha.
pub fn add_commit(repo: &Path, files: &[(&str, &str)]) -> String {
    write_files(repo, files);
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-q", "-m", "update"]);
    git(repo, &["rev-parse", "HEAD"])
}

/// One extension in a fake registry.
pub struct FakeEntry {
    pub id: String,
    pub version: String,
    pub repo: PathBuf,
    pub path: Option<String>,
}

impl FakeEntry {
    pub fn new(id: &str, version: &str, repo: &Path) -> Self {
        Self {
            id: id.into(),
            version: version.into(),
            repo: repo.into(),
            path: None,
        }
    }

    pub fn path(mut self, path: &str) -> Self {
        self.path = Some(path.into());
        self
    }
}

/// A registry repository (branch `main`) with `extensions.toml`,
/// `.gitmodules` and a real gitlink per entry, pinned to each repo's HEAD.
pub fn fake_registry(entries: &[FakeEntry]) -> PathBuf {
    let dir = new_dir().join("registry");
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    let mut toml = String::new();
    for e in entries {
        let sub = format!("extensions/{}", e.id);
        git(&dir, &["submodule", "add", "-q", &file_url(&e.repo), &sub]);
        toml.push_str(&format!(
            "[{}]\nsubmodule = \"{sub}\"\nversion = \"{}\"\n",
            e.id, e.version
        ));
        if let Some(p) = &e.path {
            toml.push_str(&format!("path = \"{p}\"\n"));
        }
        toml.push('\n');
    }
    write_files(&dir, &[("extensions.toml", &toml)]);
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "registry"]);
    dir
}

/// Moves extension `id` to its repo's current HEAD and sets `version`.
pub fn registry_update(registry: &Path, id: &str, version: &str) {
    let sub = registry.join("extensions").join(id);
    git(&sub, &["pull", "-q", "origin", "main"]);
    let path = registry.join("extensions.toml");
    let mut table: toml::Table = std::fs::read_to_string(&path).unwrap().parse().unwrap();
    table
        .get_mut(id)
        .and_then(|v| v.as_table_mut())
        .expect("id in extensions.toml")
        .insert("version".into(), toml::Value::String(version.into()));
    std::fs::write(&path, toml::to_string(&table).unwrap()).unwrap();
    git(registry, &["add", "-A"]);
    git(registry, &["commit", "-q", "-m", "bump"]);
}
