use std::path::{Path, PathBuf};

use assert_cmd::Command;
use lucerna_extension::test_support::{FakeEntry, fake_registry, fake_repo, file_url};

const THEME: &str = r#"{
  "name": "X", "author": "t",
  "themes": [{ "name": "X Dark", "appearance": "dark", "style": {} }]
}"#;

fn manifest(id: &str, version: &str) -> String {
    format!(
        "id = \"{id}\"\nname = \"Ext {id}\"\ndescription = \"About {id}\"\nversion = \"{version}\"\nschema_version = 1\n"
    )
}

fn theme_ext(id: &str, version: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("extension.toml"), manifest(id, version)).unwrap();
    std::fs::create_dir_all(dir.path().join("themes")).unwrap();
    std::fs::write(dir.path().join("themes/x.json"), THEME).unwrap();
    dir
}

fn luc(root: &Path, home: &Path) -> Command {
    let mut cmd = Command::cargo_bin("luc").unwrap();
    cmd.env("HOME", home)
        .env("LUCERNA_EXTENSIONS_DIR", root)
        .env_remove("WASI_SDK_PATH");
    cmd
}

struct Env {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("extensions");
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    Env {
        _tmp: tmp,
        root,
        home,
    }
}

impl Env {
    fn luc(&self) -> Command {
        luc(&self.root, &self.home)
    }
}

fn stdout(cmd: &mut Command) -> String {
    let out = cmd.assert().success().get_output().stdout.clone();
    String::from_utf8(out).unwrap()
}

#[test]
fn ext_list_installed_empty() {
    let e = env();
    e.luc()
        .args(["ext", "list", "--installed"])
        .assert()
        .success()
        .stdout("");
}

#[test]
fn ext_install_dev_theme_then_list() {
    let e = env();
    let ext = theme_ext("sunset", "1.2.3");
    e.luc()
        .args(["ext", "install", "--dev"])
        .arg(ext.path())
        .assert()
        .success();
    let out = stdout(e.luc().args(["ext", "list", "--installed"]));
    assert!(out.contains("sunset"), "{out}");
    assert!(out.contains("1.2.3"), "{out}");
}

#[test]
fn ext_install_unknown_id_fails() {
    let e = env();
    let (repo, _) = fake_repo(&[
        ("extension.toml", &manifest("other", "1.0.0")),
        ("themes/x.json", THEME),
    ]);
    let registry = fake_registry(&[FakeEntry::new("other", "1.0.0", &repo)]);
    fails_mentioning(
        e.luc()
            .args(["ext", "--registry-url", &file_url(&registry)])
            .args(["install", "nope-ext"]),
        "nope-ext",
    );
}

#[test]
fn ext_remove_then_list_empty() {
    let e = env();
    let ext = theme_ext("sunset", "1.0.0");
    e.luc()
        .args(["ext", "install", "--dev"])
        .arg(ext.path())
        .assert()
        .success();
    e.luc().args(["ext", "remove", "sunset"]).assert().success();
    e.luc()
        .args(["ext", "list", "--installed"])
        .assert()
        .success()
        .stdout("");
}

#[test]
fn ext_remove_unknown_fails() {
    let e = env();
    fails_mentioning(e.luc().args(["ext", "remove", "ghost"]), "ghost");
}

#[test]
fn ext_install_from_registry_reports_progress_and_updates() {
    let e = env();
    let (repo, _) = fake_repo(&[
        ("extension.toml", &manifest("sunset", "1.0.0")),
        ("themes/x.json", THEME),
    ]);
    let registry = fake_registry(&[FakeEntry::new("sunset", "1.0.0", &repo)]);
    let url = file_url(&registry);
    let run = |args: &[&str]| {
        let mut c = e.luc();
        c.args(["ext", "--registry-url", &url]).args(args);
        c
    };
    let assert = run(&["install", "sunset"]).assert().success();
    let err = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(err.contains("sunset:"), "{err}");
    let list = stdout(&mut run(&["list", "--installed"]));
    assert!(list.contains("sunset"), "{list}");
    let updates = stdout(&mut run(&["list", "--updates"]));
    assert_eq!(updates, "");
    run(&["update"]).assert().success();
}

#[test]
fn ext_search_matches_by_id() {
    let e = env();
    let (repo, _) = fake_repo(&[
        ("extension.toml", &manifest("sunset", "1.0.0")),
        ("themes/x.json", THEME),
    ]);
    let (repo2, _) = fake_repo(&[
        ("extension.toml", &manifest("zebra", "2.0.0")),
        ("themes/x.json", THEME),
    ]);
    let registry = fake_registry(&[
        FakeEntry::new("sunset", "1.0.0", &repo),
        FakeEntry::new("zebra", "2.0.0", &repo2),
    ]);
    let out = stdout(
        e.luc()
            .args(["ext", "--registry-url", &file_url(&registry)])
            .args(["search", "sunse"]),
    );
    assert!(out.starts_with("sunset 1.0.0"), "{out}");
    assert!(!out.contains("zebra"), "{out}");
}

#[test]
fn plain_path_still_opens() {
    let e = env();
    let out = stdout(e.luc().arg("--help"));
    assert!(out.contains("ext"), "{out}");
    assert!(out.contains("[PATH]"), "{out}");
    assert!(out.contains("./ext"), "{out}");
}

/// Asserts exit code 1 and that stderr mentions `needle`.
fn fails_mentioning(cmd: &mut Command, needle: &str) {
    let out = cmd.assert().code(1).get_output().stderr.clone();
    let err = String::from_utf8(out).unwrap();
    assert!(err.contains(needle), "{err}");
}
