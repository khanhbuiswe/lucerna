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

#[test]
fn ext_update_up_to_date_does_not_rebuild() {
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
        c.assert().success().get_output().stderr.clone()
    };
    run(&["install", "sunset"]);
    let err = String::from_utf8(run(&["update", "sunset"])).unwrap();
    assert!(err.contains("sunset: already up to date"), "{err}");
    assert!(!err.contains("installing"), "{err}");
}

#[test]
fn ext_list_survives_closed_pipe() {
    let e = env();
    let ext = theme_ext("sunset", "1.0.0");
    e.luc()
        .args(["ext", "install", "--dev"])
        .arg(ext.path())
        .assert()
        .success();
    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("luc"))
        .args(["ext", "list", "--installed"])
        .env("HOME", &e.home)
        .env("LUCERNA_EXTENSIONS_DIR", &e.root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let out = child.wait_with_output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!err.contains("panicked"), "{err}");
    assert!(out.status.success(), "{:?} {err}", out.status);
}

fn pid_alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    unsafe { libc::kill(pid, 0) == 0 }
}

#[test]
fn sigint_cancels_build_and_kills_children() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    let e = env();
    // A fake wasi-sdk whose clang records its pid and sleeps.
    let sdk = e._tmp.path().join("sdk");
    std::fs::create_dir_all(sdk.join("bin")).unwrap();
    std::fs::write(sdk.join("VERSION"), "34.0\n").unwrap();
    let pid_file = sdk.join("clang.pid");
    let clang = sdk.join("bin/clang");
    std::fs::write(
        &clang,
        format!(
            "#!/bin/sh\necho $$ > '{}.tmp'\nmv '{0}.tmp' '{0}'\nexec sleep 30\n",
            pid_file.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&clang, std::fs::Permissions::from_mode(0o755)).unwrap();

    let (grammar_repo, rev) = fake_repo(&[("src/parser.c", "/* parser */")]);
    let ext = tempfile::tempdir().unwrap();
    std::fs::write(
        ext.path().join("extension.toml"),
        format!(
            "{}[grammars.json]\nrepository = \"{}\"\nrev = \"{rev}\"\n",
            manifest("lang", "0.1.0"),
            file_url(&grammar_repo)
        ),
    )
    .unwrap();
    std::fs::create_dir_all(ext.path().join("languages/json")).unwrap();
    std::fs::write(
        ext.path().join("languages/json/config.toml"),
        "name = \"JSON\"\ngrammar = \"json\"\npath_suffixes = [\"json\"]\n",
    )
    .unwrap();

    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("luc"))
        .args(["ext", "--wasi-sdk"])
        .arg(&sdk)
        .args(["install", "--dev"])
        .arg(ext.path())
        .env("HOME", &e.home)
        .env("LUCERNA_EXTENSIONS_DIR", &e.root)
        .env_remove("WASI_SDK_PATH")
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(20);
    let clang_pid = loop {
        if let Ok(text) = std::fs::read_to_string(&pid_file)
            && let Ok(pid) = text.trim().parse::<i32>()
        {
            break pid;
        }
        assert!(Instant::now() < deadline, "clang never started");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(pid_alive(clang_pid));

    let sent = Instant::now();
    // SAFETY: signalling our own child.
    unsafe { libc::kill(child.id() as i32, libc::SIGINT) };
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(130));
    assert!(sent.elapsed() < Duration::from_secs(10), "exit was slow");

    let gone = Instant::now() + Duration::from_secs(3);
    while pid_alive(clang_pid) && Instant::now() < gone {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!pid_alive(clang_pid), "clang survived luc");
    // Nothing half-installed.
    let list = stdout(e.luc().args(["ext", "list", "--installed"]));
    assert_eq!(list, "");
}

fn registry_env(e: &Env) -> (String, impl Fn(&[&str]) -> Command + '_) {
    let (repo, _) = fake_repo(&[
        ("extension.toml", &manifest("sunset", "1.0.0")),
        ("themes/x.json", THEME),
    ]);
    let registry = fake_registry(&[FakeEntry::new("sunset", "1.0.0", &repo)]);
    let url = file_url(&registry);
    let u = url.clone();
    (url, move |args: &[&str]| {
        let mut c = e.luc();
        c.args(["ext", "--registry-url", &u]).args(args);
        c
    })
}

#[test]
fn ext_update_reinstalls_broken_extension() {
    let e = env();
    let (_, run) = registry_env(&e);
    run(&["install", "sunset"]).assert().success();
    std::fs::write(e.root.join("installed/sunset/extension.toml"), "not toml [").unwrap();
    let _ = std::fs::remove_file(e.root.join("index.json"));
    let list = stdout(&mut run(&["list", "--installed"]));
    assert!(list.contains("broken"), "{list}");
    let err = run(&["update", "sunset"])
        .assert()
        .success()
        .get_output()
        .stderr
        .clone();
    let err = String::from_utf8(err).unwrap();
    assert!(err.contains("installing"), "{err}");
    let list = stdout(&mut run(&["list", "--installed"]));
    assert!(!list.contains("broken"), "{list}");
}

#[test]
fn ext_update_replaces_dev_install_with_registry_version() {
    let e = env();
    let (_, run) = registry_env(&e);
    let dev = theme_ext("sunset", "9.9.9");
    e.luc()
        .args(["ext", "install", "--dev"])
        .arg(dev.path())
        .assert()
        .success();
    assert!(stdout(&mut run(&["list", "--installed"])).contains("9.9.9"));
    run(&["update", "sunset"]).assert().success();
    let list = stdout(&mut run(&["list", "--installed"]));
    assert!(list.contains("sunset 1.0.0"), "{list}");
}
