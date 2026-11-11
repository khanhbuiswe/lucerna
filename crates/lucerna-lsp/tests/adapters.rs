use std::fs;
use std::path::Path;

use lucerna_lsp::adapters::{Adapter, builtin_adapters, find_root};
use lucerna_lsp::env::{shell_env_from, which};

fn adapter(name: &str) -> std::sync::Arc<dyn Adapter> {
    builtin_adapters()
        .into_iter()
        .find(|a| a.name() == name)
        .unwrap_or_else(|| panic!("no adapter {name}"))
}

fn touch(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "").unwrap();
}

#[test]
fn rust_root_is_outermost_cargo_toml() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    touch(&root.join("Cargo.toml"));
    touch(&root.join("crates/a/Cargo.toml"));
    let file = root.join("crates/a/src/lib.rs");
    touch(&file);
    assert_eq!(
        find_root(&*adapter("rust-analyzer"), &file, root).unwrap(),
        root
    );
}

#[test]
fn go_prefers_go_work_over_go_mod() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    touch(&root.join("go.work"));
    touch(&root.join("sub/go.mod"));
    let file = root.join("sub/main.go");
    touch(&file);
    assert_eq!(find_root(&*adapter("gopls"), &file, root).unwrap(), root);
}

#[test]
fn go_mod_when_no_go_work() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    touch(&root.join("sub/go.mod"));
    let file = root.join("sub/pkg/main.go");
    touch(&file);
    assert_eq!(
        find_root(&*adapter("gopls"), &file, root).unwrap(),
        root.join("sub")
    );
}

#[test]
fn ts_nearest_package_json() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    touch(&root.join("package.json"));
    touch(&root.join("packages/web/package.json"));
    let file = root.join("packages/web/src/index.ts");
    touch(&file);
    assert_eq!(
        find_root(&*adapter("typescript-language-server"), &file, root).unwrap(),
        root.join("packages/web")
    );
}

#[test]
fn root_never_escapes_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let outer = dir.path();
    let root = outer.join("ws");
    touch(&outer.join("Cargo.toml"));
    let file = root.join("src/lib.rs");
    touch(&file);
    // Marker only above the workspace: falls back to the workspace root.
    assert_eq!(
        find_root(&*adapter("rust-analyzer"), &file, &root).unwrap(),
        root
    );
}

#[test]
fn file_outside_workspace_has_no_root() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("ws");
    fs::create_dir_all(&root).unwrap();
    let file = dir.path().join("other/x.rs");
    touch(&file);
    touch(&dir.path().join("other/Cargo.toml"));
    assert_eq!(find_root(&*adapter("rust-analyzer"), &file, &root), None);
}

#[test]
fn no_marker_falls_back_to_workspace_root() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let file = root.join("a/b/main.go");
    touch(&file);
    assert_eq!(find_root(&*adapter("gopls"), &file, root).unwrap(), root);
}

#[cfg(unix)]
#[test]
fn which_finds_executable_only() {
    use std::os::unix::fs::PermissionsExt;
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let plain = a.path().join("tool");
    fs::write(&plain, "").unwrap();
    fs::set_permissions(&plain, fs::Permissions::from_mode(0o644)).unwrap();
    let exe = b.path().join("tool");
    fs::write(&exe, "").unwrap();
    fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
    let path_var = format!("{}:{}", a.path().display(), b.path().display());
    assert_eq!(which("tool", &path_var), Some(exe));
    assert_eq!(which("missing", &path_var), None);
}

#[test]
fn login_env_falls_back_on_bad_shell() {
    assert!(shell_env_from(Path::new("/nonexistent/shell")).is_none());
}

#[cfg(unix)]
#[test]
fn shell_env_parses_marker_output() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let shell = dir.path().join("fakesh");
    // Ignores its arguments; prints noise, the marker, then NUL-separated env.
    fs::write(
        &shell,
        "#!/bin/sh\nprintf 'banner noise'\nprintf '\\0LUCERNA_ENV\\0'\nprintf 'A=1\\0PATH=/x:/y\\0'\n",
    )
    .unwrap();
    fs::set_permissions(&shell, fs::Permissions::from_mode(0o755)).unwrap();
    let env = shell_env_from(&shell).expect("env");
    assert_eq!(env.get("A").map(String::as_str), Some("1"));
    assert_eq!(env.get("PATH").map(String::as_str), Some("/x:/y"));
    assert_eq!(env.len(), 2);
}

#[cfg(unix)]
fn fake_shell(dir: &Path, body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let shell = dir.join("fakesh");
    fs::write(&shell, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&shell, fs::Permissions::from_mode(0o755)).unwrap();
    shell
}

#[cfg(unix)]
#[test]
fn login_shell_runs_in_its_own_session() {
    let dir = tempfile::tempdir().unwrap();
    // Prints the env only when it leads its own process group (setsid), so
    // an interactive shell cannot grab the terminal's foreground group.
    let shell = fake_shell(
        dir.path(),
        r#"[ "$(ps -o pgid= -p $$ | tr -d ' ')" = "$$" ] || exit 1
printf '\0LUCERNA_ENV\0A=1\0'"#,
    );
    let env = shell_env_from(&shell).expect("shell is a session leader");
    assert_eq!(env.get("A").map(String::as_str), Some("1"));
}

#[cfg(unix)]
#[test]
fn login_shell_timeout_kills_its_children() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("pid");
    let shell = fake_shell(
        dir.path(),
        &format!("sleep 60 &\necho $! > '{}'\nsleep 60", pid_file.display()),
    );
    assert!(shell_env_from(&shell).is_none());
    let pid = fs::read_to_string(&pid_file).unwrap();
    let alive = || {
        std::process::Command::new("kill")
            .args(["-0", pid.trim()])
            .status()
            .is_ok_and(|s| s.success())
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while alive() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(
        !alive(),
        "background child of the timed-out shell was killed"
    );
}

#[cfg(unix)]
#[test]
fn process_launcher_probe_runs_in_the_project_root() {
    use lucerna_lsp::{Launcher, ProcessLauncher};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let launcher = ProcessLauncher::new();
    let script = format!("[ \"$(pwd -P)\" = '{}' ]", root.display());
    assert!(launcher.probe(Path::new("/bin/sh"), &["-c", &script], &root));
}

#[test]
fn builtin_adapter_table() {
    let all = builtin_adapters();
    let names: Vec<_> = all.iter().map(|a| a.name()).collect();
    assert_eq!(
        names,
        [
            "rust-analyzer",
            "gopls",
            "basedpyright",
            "pyright",
            "typescript-language-server",
            "clangd"
        ]
    );
    let ra = adapter("rust-analyzer");
    assert_eq!(ra.language_ids(), ["rust"]);
    assert_eq!(ra.binary(), "rust-analyzer");
    assert!(ra.args().is_empty());
    assert_eq!(ra.probe_args(), ["--version"]);

    let go = adapter("gopls");
    assert_eq!(go.language_ids(), ["go"]);
    assert_eq!(go.probe_args(), ["version"]);
    assert_eq!(go.root_markers(), ["go.work", "go.mod"]);

    let bp = adapter("basedpyright");
    assert_eq!(bp.language_ids(), ["python"]);
    assert_eq!(bp.binary(), "basedpyright-langserver");
    assert_eq!(bp.args(), ["--stdio"]);
    assert!(bp.probe_args().is_empty());
    assert!(bp.enabled_by_default());

    let py = adapter("pyright");
    assert_eq!(py.binary(), "pyright-langserver");
    assert_eq!(py.args(), ["--stdio"]);
    assert!(py.probe_args().is_empty());
    assert!(!py.enabled_by_default());
    assert_eq!(py.root_markers(), bp.root_markers());

    let ts = adapter("typescript-language-server");
    assert_eq!(
        ts.language_ids(),
        [
            "typescript",
            "typescriptreact",
            "javascript",
            "javascriptreact"
        ]
    );
    assert_eq!(ts.binary(), "typescript-language-server");
    assert_eq!(ts.args(), ["--stdio"]);
    assert_eq!(
        ts.root_markers(),
        ["tsconfig.json", "jsconfig.json", "package.json"]
    );

    let cl = adapter("clangd");
    assert_eq!(cl.language_ids(), ["c", "cpp"]);
    assert_eq!(cl.binary(), "clangd");
    assert_eq!(cl.root_markers(), ["compile_commands.json", ".clangd"]);
}

#[test]
fn fake_launcher_resolves_only_registered_binaries() {
    use lucerna_lsp::Launcher;
    use lucerna_lsp::test_support::{FakeLauncher, FakeServer};
    let launcher = FakeLauncher::new().with_server("gopls", FakeServer::new());
    let found = launcher.which("gopls").expect("registered");
    assert!(found.ends_with("gopls"));
    assert!(launcher.which("clangd").is_none());
    assert!(launcher.probe(&found, &["version"], Path::new("/")));
    assert!(launcher.launches().is_empty());
}

#[cfg(unix)]
#[test]
fn process_launcher_probe_reports_exit_status() {
    use lucerna_lsp::{Launcher, ProcessLauncher};
    let launcher = ProcessLauncher::new();
    let root = Path::new("/");
    assert!(launcher.probe(Path::new("/usr/bin/true"), &[], root));
    assert!(!launcher.probe(Path::new("/usr/bin/false"), &[], root));
    assert!(!launcher.probe(Path::new("/nonexistent/x"), &[], root));
}
