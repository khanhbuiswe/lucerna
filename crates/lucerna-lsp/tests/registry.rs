use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lucerna_lsp::table::{ServerDef, ServerTable};
use lucerna_lsp::test_support::{FakeLauncher, FakeServer};
use lucerna_lsp::vars::Vars;
use lucerna_lsp::{Registry, ServerKey, Slot, StartError, resolve_launch, select_servers};
use serde_json::{Value, json};

fn names_for(
    list: &[&str],
    language: Option<&str>,
    path: &str,
    table: &ServerTable,
) -> Vec<String> {
    let list: Vec<String> = list.iter().map(|s| (*s).to_owned()).collect();
    select_servers(&list, table, language, Path::new(path))
        .iter()
        .map(|d| d.name.to_string())
        .collect()
}

fn names(list: &[&str], language: &str) -> Vec<String> {
    names_for(list, Some(language), "/ws/x", &ServerTable::build(None))
}

fn table(lsp: Value) -> ServerTable {
    let table = ServerTable::build(Some(&lsp));
    assert!(table.errors.is_empty(), "{:?}", table.errors);
    table
}

fn adapter(name: &str) -> ServerDef {
    ServerTable::build(None).get(name).unwrap().clone()
}

/// `name` with `lsp` overrides from the User settings applied.
fn def_with(name: &str, lsp: Value) -> ServerDef {
    table(lsp).get(name).unwrap().clone()
}

fn vars() -> Vars {
    Vars {
        root: PathBuf::from("/ws"),
        data_dir: PathBuf::from("/data/lsp/s/0123456789ab"),
        npm_root: None,
    }
}

fn resolve(
    def: &ServerDef,
    launcher: &dyn lucerna_lsp::Launcher,
) -> Result<lucerna_lsp::transport::LaunchSpec, StartError> {
    resolve_launch(def, Path::new("/ws"), launcher, &vars())
}

#[test]
fn select_default_python_list() {
    assert_eq!(
        names(&["basedpyright", "!pyright", "..."], "python"),
        ["basedpyright"]
    );
    assert_eq!(names(&["..."], "python"), ["basedpyright"]);
}

#[test]
fn select_explicit_pyright_first() {
    assert_eq!(
        names(&["pyright", "..."], "python"),
        ["pyright", "basedpyright"]
    );
    // Unknown names and servers of other languages are ignored.
    assert_eq!(names(&["nope", "gopls", "pyright"], "python"), ["pyright"]);
}

#[test]
fn select_by_file_type_without_language() {
    let t = table(json!({
        "fake-rb": { "command": "fake-rb", "file_types": ["rb", "Gemfile"] },
        "off-rb": { "command": "off-rb", "file_types": ["rb"], "enabled": false },
    }));
    assert_eq!(names_for(&["..."], None, "/ws/a.rb", &t), ["fake-rb"]);
    assert_eq!(names_for(&["..."], None, "/ws/A.RB", &t), ["fake-rb"]);
    assert_eq!(names_for(&["..."], None, "/ws/Gemfile", &t), ["fake-rb"]);
    assert!(names_for(&["..."], None, "/ws/gemfile", &t).is_empty());
    assert!(names_for(&["..."], None, "/ws/a.py", &t).is_empty());
    // A file with a language is matched by language only.
    assert!(names_for(&["..."], Some("python"), "/ws/a.rb", &t) == ["basedpyright"]);
    // Named explicitly, a disabled server is used.
    assert_eq!(
        names_for(&["off-rb", "..."], None, "/ws/a.rb", &t),
        ["off-rb", "fake-rb"]
    );
}

#[test]
fn select_dots_is_in_name_order() {
    let t = table(json!({
        "b-ls": { "command": "b", "languages": ["x"] },
        "a-ls": { "command": "a", "languages": ["x"] },
    }));
    assert_eq!(
        names_for(&["..."], Some("x"), "/ws/f", &t),
        ["a-ls", "b-ls"]
    );
}

#[test]
fn select_bang_removes_from_dots() {
    assert_eq!(
        names(&["!basedpyright", "..."], "python"),
        Vec::<String>::new()
    );
    assert_eq!(names(&["..."], "rust"), ["rust-analyzer"]);
    assert_eq!(
        names(&["!rust-analyzer", "..."], "rust"),
        Vec::<String>::new()
    );
}

#[test]
fn resolve_uses_binary_path_without_probe() {
    // The launcher knows no binaries, so a probe or `which` would fail.
    let launcher = FakeLauncher::new();
    let def = def_with(
        "rust-analyzer",
        json!({ "rust-analyzer": { "binary": { "path": "/opt/ra", "arguments": ["--x"] } } }),
    );
    let spec = resolve(&def, &launcher).unwrap();
    assert_eq!(spec.server, "rust-analyzer");
    assert_eq!(spec.program, PathBuf::from("/opt/ra"));
    assert_eq!(spec.args, ["--x"]);
    assert_eq!(spec.cwd, PathBuf::from("/ws"));

    let def = def_with(
        "basedpyright",
        json!({ "basedpyright": { "binary": { "path": "/opt/bp" } } }),
    );
    let spec = resolve(&def, &launcher).unwrap();
    assert_eq!(spec.args, ["--stdio"], "adapter args when none are set");
}

#[test]
fn resolve_finds_binary_on_path() {
    let launcher = FakeLauncher::new().with_server("gopls", FakeServer::new());
    let spec = resolve(&adapter("gopls"), &launcher).unwrap();
    assert_eq!(spec.program, PathBuf::from("/fake/bin/gopls"));
}

#[test]
fn resolve_not_found_message_exact() {
    let launcher = FakeLauncher::new();
    let Err(StartError::NotFound { message }) = resolve(&adapter("gopls"), &launcher) else {
        panic!("expected NotFound");
    };
    assert_eq!(
        message,
        r#"gopls not found on PATH. Install it or set "lsp": { "gopls": { "binary": { "path": … } } }"#
    );
}

#[test]
fn resolve_probe_failure_is_not_found() {
    // `which` finds the binary but the probe fails: a launcher that knows
    // the name only for `which`.
    struct NoProbe(FakeLauncher);
    impl lucerna_lsp::Launcher for NoProbe {
        fn env(&self) -> HashMap<String, String> {
            self.0.env()
        }
        fn which(&self, name: &str) -> Option<PathBuf> {
            self.0.which(name)
        }
        fn probe(&self, _: &Path, _: &[&str], _: &Path) -> bool {
            false
        }
        fn output(&self, program: &str, args: &[&str]) -> Option<String> {
            self.0.output(program, args)
        }
        fn launch(
            &self,
            spec: &lucerna_lsp::transport::LaunchSpec,
            inbox: Arc<dyn lucerna_lsp::transport::Inbox>,
        ) -> std::io::Result<Arc<dyn lucerna_lsp::transport::Transport>> {
            self.0.launch(spec, inbox)
        }
    }
    let launcher = NoProbe(FakeLauncher::new().with_server("rust-analyzer", FakeServer::new()));
    let result = resolve(&adapter("rust-analyzer"), &launcher);
    assert!(matches!(result, Err(StartError::NotFound { .. })));
    // An adapter with empty probe_args is not probed.
    let launcher =
        NoProbe(FakeLauncher::new().with_server("basedpyright-langserver", FakeServer::new()));
    assert!(resolve(&adapter("basedpyright"), &launcher).is_ok());
}

#[test]
fn binary_env_overlays_login_env() {
    let launcher = FakeLauncher::new().with_server("gopls", FakeServer::new());
    let def = def_with(
        "gopls",
        json!({ "gopls": { "binary": { "env": { "GOFLAGS": "-mod=mod", "PATH": "/custom" } } } }),
    );
    let spec = resolve(&def, &launcher).unwrap();
    assert_eq!(spec.env["GOFLAGS"], "-mod=mod");
    assert_eq!(spec.env["PATH"], "/custom", "binary.env wins");
    let spec = resolve(&adapter("gopls"), &launcher).unwrap();
    assert_eq!(spec.env["PATH"], "/fake/bin", "login env otherwise");
    // `env` lies between the login env and `binary.env`; both expand.
    let def = def_with(
        "gopls",
        json!({ "gopls": {
            "env": { "A": "{root}/a", "B": "env" },
            "binary": { "env": { "B": "{data_dir}" } }
        } }),
    );
    let spec = resolve(&def, &launcher).unwrap();
    assert_eq!(spec.env["A"], "/ws/a");
    assert_eq!(spec.env["B"], "/data/lsp/s/0123456789ab");
    assert_eq!(spec.env["PATH"], "/fake/bin");
}

#[test]
fn jdtls_args_expand_data_dir() {
    let launcher = FakeLauncher::new().with_server("jdtls", FakeServer::new());
    let def = def_with(
        "jdtls",
        json!({ "jdtls": { "command": "jdtls", "args": ["-data", "{data_dir}"], "languages": ["java"] } }),
    );
    let spec = resolve(&def, &launcher).unwrap();
    assert_eq!(spec.server, "jdtls");
    assert_eq!(spec.args, ["-data", "/data/lsp/s/0123456789ab"]);
    // `binary.arguments` replace `args` and expand too.
    let def = def_with(
        "jdtls",
        json!({ "jdtls": { "command": "jdtls", "languages": ["java"],
                           "binary": { "arguments": ["--root={root}"] } } }),
    );
    assert_eq!(resolve(&def, &launcher).unwrap().args, ["--root=/ws"]);
}

#[test]
fn resolve_variable_errors_fail_with_the_variable() {
    let launcher = FakeLauncher::new().with_server("x", FakeServer::new());
    let def = def_with(
        "x",
        json!({ "x": { "command": "x", "args": ["{nope}"], "languages": ["x"] } }),
    );
    let Err(StartError::Failed { message, .. }) = resolve(&def, &launcher) else {
        panic!("expected Failed");
    };
    assert_eq!(message, "unknown variable {nope}");
    let def = def_with(
        "x",
        json!({ "x": { "command": "x", "args": ["{npm_root}/p"], "languages": ["x"] } }),
    );
    let Err(StartError::Failed { message, .. }) = resolve(&def, &launcher) else {
        panic!("expected Failed");
    };
    assert_eq!(message, "npm not found; {npm_root} unavailable");
}

#[test]
fn resolve_user_command_without_probe_flag() {
    let launcher = FakeLauncher::new().with_server("mine", FakeServer::new());
    let def = def_with(
        "mine",
        json!({ "mine": { "command": "mine", "file_types": ["m"], "probe": false } }),
    );
    let spec = resolve(&def, &launcher).unwrap();
    assert_eq!(spec.program, PathBuf::from("/fake/bin/mine"));
    assert!(spec.args.is_empty());
}

fn key(root: &str, server: &str) -> ServerKey {
    ServerKey {
        root: Arc::from(Path::new(root)),
        server: Arc::from(server),
    }
}

#[test]
fn settings_diff_restart_vs_reconfigure() {
    let mut reg = Registry::new(table(json!({
        "rust-analyzer": { "settings": { "a": 1 } },
        "gopls": { "settings": { "a": 1 } },
        "clangd": { "initialization_options": { "x": 1 } },
        "typescript-language-server": { "initialization_options": { "t": 1 } },
    })));
    let ra = key("/ws", "rust-analyzer");
    let gopls = key("/ws", "gopls");
    let clangd = key("/ws", "clangd");
    let ts = key("/ws", "typescript-language-server");
    for k in [&ra, &gopls, &clangd, &ts] {
        reg.set_slot(k.clone(), Slot::Starting);
    }

    let diff = reg.set_table(table(json!({
        "rust-analyzer": { "binary": { "path": "/x" }, "settings": { "a": 1 } },
        "gopls": { "settings": { "a": 2 } },
        "clangd": { "initialization_options": { "x": 1 } },
    })));
    let mut restart = diff.restart.clone();
    restart.sort_by(|a, b| a.server.cmp(&b.server));
    // ts was removed from the map: back to defaults, which differ.
    assert_eq!(restart, [ra, ts]);
    assert_eq!(diff.reconfigure, [(gopls, json!({ "a": 2 }))]);
    assert!(diff.stop.is_empty());
    assert!(
        reg.table()
            .get("clangd")
            .unwrap()
            .initialization_options
            .is_some()
    );
    assert!(
        reg.table()
            .get("typescript-language-server")
            .unwrap()
            .initialization_options
            .is_none()
    );

    // A removed `settings` is reported as null.
    let diff = reg.set_table(ServerTable::build(None));
    assert!(diff.reconfigure.iter().any(|(_, v)| v.is_null()));
}

#[test]
fn settings_diff_launch_fields_restart() {
    let mut reg = Registry::new(table(json!({
        "mine": { "command": "mine", "languages": ["x"], "settings": { "a": 1 } },
    })));
    let k = key("/ws", "mine");
    reg.set_slot(k.clone(), Slot::Starting);
    for change in [
        json!({ "command": "mine2" }),
        json!({ "args": ["--stdio"] }),
        json!({ "env": { "A": "1" } }),
    ] {
        let mut lsp =
            json!({ "mine": { "command": "mine", "languages": ["x"], "settings": { "a": 1 } } });
        lucerna_lsp::table::merge_json(&mut lsp["mine"], &change);
        let diff = reg.set_table(table(lsp));
        assert_eq!(diff.restart, std::slice::from_ref(&k), "{change}");
        // Back to the original for the next change.
        reg.set_table(table(json!({
            "mine": { "command": "mine", "languages": ["x"], "settings": { "a": 1 } },
        })));
    }
    // Fields that do not decide the launch change nothing.
    let diff = reg.set_table(table(json!({
        "mine": { "command": "mine", "languages": ["x", "y"], "file_types": ["x"],
                  "root_markers": ["m"], "settings": { "a": 1 } },
    })));
    assert_eq!(diff, lucerna_lsp::SettingsDiff::default());
}

#[test]
fn set_table_stops_removed_server() {
    let mut reg = Registry::new(table(json!({
        "mine": { "command": "mine", "languages": ["x"] },
        "other": { "command": "other", "languages": ["x"] },
    })));
    let mine = key("/ws", "mine");
    let other = key("/ws", "other");
    let ra = key("/ws", "rust-analyzer");
    for k in [&mine, &other, &ra] {
        reg.set_slot(k.clone(), Slot::Starting);
    }
    let diff = reg.set_table(table(json!({
        "other": { "command": "other", "languages": ["x"] },
        "rust-analyzer": { "enabled": false },
    })));
    let mut stop = diff.stop.clone();
    stop.sort_by(|a, b| a.server.cmp(&b.server));
    assert_eq!(stop, [mine, ra]);
    assert!(diff.restart.is_empty() && diff.reconfigure.is_empty());
    assert!(reg.table().get("mine").is_none());
}

#[test]
fn keys_for_two_crates_share_outer_root() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    std::fs::write(ws.join("Cargo.toml"), "[workspace]").unwrap();
    for c in ["a", "b"] {
        std::fs::create_dir_all(ws.join("crates").join(c).join("src")).unwrap();
        std::fs::write(ws.join("crates").join(c).join("Cargo.toml"), "[package]").unwrap();
    }
    let reg = Registry::new(ServerTable::build(None));
    let list = vec!["...".to_owned()];
    let ka = reg.keys_for(Some("rust"), &ws.join("crates/a/src/lib.rs"), ws, &list);
    let kb = reg.keys_for(Some("rust"), &ws.join("crates/b/src/lib.rs"), ws, &list);
    assert_eq!(ka.len(), 1);
    assert_eq!(ka, kb);
    assert_eq!(&*ka[0].root, ws);
    assert_eq!(&*ka[0].server, "rust-analyzer");
}

#[test]
fn slots_and_remove() {
    let mut reg = Registry::new(ServerTable::build(None));
    let k = key("/ws", "gopls");
    assert!(reg.slot(&k).is_none());
    reg.set_slot(k.clone(), Slot::Stopped);
    assert!(matches!(reg.slot(&k), Some(Slot::Stopped)));
    assert_eq!(reg.keys().count(), 1);
    assert_eq!(reg.running().count(), 0);
    assert!(reg.table().get("gopls").is_some());
    assert!(reg.table().get("nope").is_none());
    assert!(matches!(reg.remove(&k), Some(Slot::Stopped)));
    assert!(reg.slot(&k).is_none());
}

#[test]
fn keys_for_a_file_type_without_language() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    std::fs::write(ws.join("Gemfile"), "").unwrap();
    std::fs::create_dir_all(ws.join("lib")).unwrap();
    let reg = Registry::new(table(json!({
        "fake-rb": { "command": "fake-rb", "file_types": ["rb"], "root_markers": ["Gemfile"] }
    })));
    let list = vec!["...".to_owned()];
    let keys = reg.keys_for(None, &ws.join("lib/x.rb"), ws, &list);
    assert_eq!(keys, [key(ws.to_str().unwrap(), "fake-rb")]);
    assert!(
        reg.keys_for(None, &ws.join("lib/x.txt"), ws, &list)
            .is_empty()
    );
}
