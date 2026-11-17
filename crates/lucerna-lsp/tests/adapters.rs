use std::fs;
use std::path::Path;

use lucerna_lsp::adapters::find_root;
use lucerna_lsp::env::{shell_env_from, which};
use lucerna_lsp::table::{ServerDef, ServerTable};
use serde_json::json;

fn adapter(name: &str) -> ServerDef {
    ServerTable::build(None)
        .get(name)
        .cloned()
        .unwrap_or_else(|| panic!("no server {name}"))
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
        find_root(&adapter("rust-analyzer"), &file, root).unwrap(),
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
    assert_eq!(find_root(&adapter("gopls"), &file, root).unwrap(), root);
}

#[test]
fn go_mod_when_no_go_work() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    touch(&root.join("sub/go.mod"));
    let file = root.join("sub/pkg/main.go");
    touch(&file);
    assert_eq!(
        find_root(&adapter("gopls"), &file, root).unwrap(),
        root.join("sub")
    );
}

#[test]
fn ts_monorepo_uses_outermost_package_json() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    touch(&root.join("package.json"));
    touch(&root.join("packages/web/package.json"));
    let file = root.join("packages/web/src/index.ts");
    touch(&file);
    assert_eq!(
        find_root(&adapter("typescript-language-server"), &file, root).unwrap(),
        root
    );
}

#[test]
fn php_vendor_file_shares_app_root() {
    for with_git in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        if with_git {
            fs::create_dir_all(root.join(".git")).unwrap();
        }
        touch(&root.join("apps/api/composer.json"));
        touch(&root.join("apps/api/vendor/laravel/framework/composer.json"));
        let app = root.join("apps/api/app/User.php");
        let lib = root.join("apps/api/vendor/laravel/framework/src/Model.php");
        touch(&app);
        touch(&lib);
        let def = adapter("intelephense");
        // A `.git` at the repo root must not outrank the app's composer.json.
        let expected = root.join("apps/api");
        assert_eq!(find_root(&def, &app, root).unwrap(), expected);
        assert_eq!(find_root(&def, &lib, root).unwrap(), expected);
    }
}

#[test]
fn project_marker_wins_over_repo_git() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join(".git")).unwrap();
    touch(&root.join("apps/rails/Gemfile"));
    let file = root.join("apps/rails/app/models/user.rb");
    touch(&file);
    assert_eq!(
        find_root(&adapter("ruby-lsp"), &file, root).unwrap(),
        root.join("apps/rails")
    );
}

#[test]
fn git_is_only_ever_a_sole_root_marker() {
    // With outermost search a `.git` beside other markers would always win
    // over them, so it may only be a server's sole marker.
    let table = ServerTable::build(None);
    for def in table.servers.values() {
        if def.root_markers.len() > 1 {
            assert!(
                !def.root_markers.iter().any(|m| m == ".git"),
                "{} lists .git with other markers",
                def.name
            );
        }
    }
}

#[test]
fn node_modules_file_shares_project_root() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    touch(&root.join("web/package.json"));
    touch(&root.join("web/node_modules/react/package.json"));
    let app = root.join("web/src/main.ts");
    let lib = root.join("web/node_modules/react/index.d.ts");
    touch(&app);
    touch(&lib);
    let def = adapter("typescript-language-server");
    assert_eq!(find_root(&def, &app, root).unwrap(), root.join("web"));
    assert_eq!(find_root(&def, &lib, root).unwrap(), root.join("web"));
}

#[test]
fn nearest_can_be_restored_by_setting() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    touch(&root.join("package.json"));
    touch(&root.join("packages/web/package.json"));
    let file = root.join("packages/web/src/index.ts");
    touch(&file);
    let table = ServerTable::build(Some(&json!({
        "typescript-language-server": { "root_search": "nearest" }
    })));
    let def = table.get("typescript-language-server").unwrap();
    assert_eq!(find_root(def, &file, root).unwrap(), root.join("packages/web"));
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
        find_root(&adapter("rust-analyzer"), &file, &root).unwrap(),
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
    assert_eq!(find_root(&adapter("rust-analyzer"), &file, &root), None);
}

#[test]
fn no_marker_falls_back_to_workspace_root() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let file = root.join("a/b/main.go");
    touch(&file);
    assert_eq!(find_root(&adapter("gopls"), &file, root).unwrap(), root);
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

#[test]
fn glob_marker_finds_sln() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    touch(&root.join("a.sln"));
    let file = root.join("src/App/Program.cs");
    touch(&file);
    touch(&root.join("src/notes.txt"));
    let table = ServerTable::build(Some(&json!({
        "csharp-ls": { "command": "csharp-ls", "languages": ["csharp"],
                       "root_markers": ["*.sln", "*.csproj"] }
    })));
    let def = table.get("csharp-ls").unwrap();
    assert_eq!(find_root(def, &file, root).unwrap(), root);
    // The outermost directory with a matching entry wins.
    touch(&root.join("src/App/App.csproj"));
    assert_eq!(find_root(def, &file, root).unwrap(), root);
}

#[test]
fn glob_marker_question_mark_and_no_match() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    touch(&root.join("sub/pkg.cabal"));
    let file = root.join("sub/src/Main.hs");
    touch(&file);
    let table = ServerTable::build(Some(&json!({
        "hls": { "command": "hls", "languages": ["haskell"],
                 "root_markers": ["???.cabal"] },
        "none": { "command": "none", "languages": ["haskell"],
                  "root_markers": ["*.nothing"] }
    })));
    assert_eq!(
        find_root(table.get("hls").unwrap(), &file, root).unwrap(),
        root.join("sub")
    );
    assert_eq!(
        find_root(table.get("none").unwrap(), &file, root).unwrap(),
        root
    );
}

#[test]
fn fake_launcher_output_is_registered_per_command() {
    use lucerna_lsp::Launcher;
    use lucerna_lsp::test_support::FakeLauncher;
    let launcher = FakeLauncher::new().with_output("npm", &["root", "-g"], "/n\n");
    assert_eq!(
        launcher.output("npm", &["root", "-g"]).as_deref(),
        Some("/n\n")
    );
    assert_eq!(launcher.output("npm", &["-v"]), None);
    assert_eq!(launcher.output("node", &[]), None);
}

#[cfg(unix)]
#[test]
fn process_launcher_output_returns_stdout_on_success() {
    use lucerna_lsp::{Launcher, ProcessLauncher};
    let launcher = ProcessLauncher::new();
    assert_eq!(
        launcher.output("echo", &["hello"]).as_deref(),
        Some("hello\n")
    );
    assert_eq!(launcher.output("false", &[]), None);
    assert_eq!(launcher.output("lucerna-no-such-binary", &[]), None);
}

/// Glob markers ignore case where file names do (Windows): `App.SLN` is a
/// `*.sln` there, not on Linux or macOS.
#[test]
fn glob_marker_case_follows_platform() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    touch(&root.join("sub/App.SLN"));
    let file = root.join("sub/Program.cs");
    touch(&file);
    let table = ServerTable::build(Some(&json!({
        "cs": { "command": "cs", "languages": ["csharp"], "root_markers": ["*.sln"] }
    })));
    let expected = if cfg!(windows) {
        root.join("sub")
    } else {
        root.to_path_buf()
    };
    assert_eq!(
        find_root(table.get("cs").unwrap(), &file, root).unwrap(),
        expected
    );
}
