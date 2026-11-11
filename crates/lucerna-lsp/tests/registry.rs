use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lucerna_lsp::test_support::{FakeLauncher, FakeServer};
use lucerna_lsp::{
    Adapter, Registry, ServerKey, ServerSettings, ServersSettings, Slot, StartError,
    builtin_adapters, resolve_launch, select_servers,
};
use serde_json::json;

fn names(list: &[&str], language: &str) -> Vec<&'static str> {
    let list: Vec<String> = list.iter().map(|s| (*s).to_owned()).collect();
    select_servers(&list, &builtin_adapters(), language)
        .iter()
        .map(|a| a.name())
        .collect()
}

fn adapter(name: &str) -> Arc<dyn Adapter> {
    builtin_adapters()
        .into_iter()
        .find(|a| a.name() == name)
        .unwrap()
}

fn settings(json: serde_json::Value) -> ServerSettings {
    serde_json::from_value(json).unwrap()
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
fn select_bang_removes_from_dots() {
    assert_eq!(
        names(&["!basedpyright", "..."], "python"),
        Vec::<&str>::new()
    );
    assert_eq!(names(&["..."], "rust"), ["rust-analyzer"]);
    assert_eq!(
        names(&["!rust-analyzer", "..."], "rust"),
        Vec::<&str>::new()
    );
}

#[test]
fn resolve_uses_binary_path_without_probe() {
    // The launcher knows no binaries, so a probe or `which` would fail.
    let launcher = FakeLauncher::new();
    let s = settings(json!({ "binary": { "path": "/opt/ra", "arguments": ["--x"] } }));
    let spec = resolve_launch(
        &*adapter("rust-analyzer"),
        Some(&s),
        Path::new("/ws"),
        &launcher,
    )
    .unwrap();
    assert_eq!(spec.server, "rust-analyzer");
    assert_eq!(spec.program, PathBuf::from("/opt/ra"));
    assert_eq!(spec.args, ["--x"]);
    assert_eq!(spec.cwd, PathBuf::from("/ws"));

    let s = settings(json!({ "binary": { "path": "/opt/bp" } }));
    let spec = resolve_launch(
        &*adapter("basedpyright"),
        Some(&s),
        Path::new("/ws"),
        &launcher,
    )
    .unwrap();
    assert_eq!(spec.args, ["--stdio"], "adapter args when none are set");
}

#[test]
fn resolve_finds_binary_on_path() {
    let launcher = FakeLauncher::new().with_server("gopls", FakeServer::new());
    let spec = resolve_launch(&*adapter("gopls"), None, Path::new("/ws"), &launcher).unwrap();
    assert_eq!(spec.program, PathBuf::from("/fake/bin/gopls"));
}

#[test]
fn resolve_not_found_message_exact() {
    let launcher = FakeLauncher::new();
    let Err(StartError::NotFound { message }) =
        resolve_launch(&*adapter("gopls"), None, Path::new("/ws"), &launcher)
    else {
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
        fn probe(&self, _: &Path, _: &[&str]) -> bool {
            false
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
    let result = resolve_launch(
        &*adapter("rust-analyzer"),
        None,
        Path::new("/ws"),
        &launcher,
    );
    assert!(matches!(result, Err(StartError::NotFound { .. })));
    // An adapter with empty probe_args is not probed.
    let launcher =
        NoProbe(FakeLauncher::new().with_server("basedpyright-langserver", FakeServer::new()));
    assert!(resolve_launch(&*adapter("basedpyright"), None, Path::new("/ws"), &launcher).is_ok());
}

#[test]
fn binary_env_overlays_login_env() {
    let launcher = FakeLauncher::new().with_server("gopls", FakeServer::new());
    let s = settings(json!({ "binary": { "env": { "GOFLAGS": "-mod=mod", "PATH": "/custom" } } }));
    let spec = resolve_launch(&*adapter("gopls"), Some(&s), Path::new("/ws"), &launcher).unwrap();
    assert_eq!(spec.env["GOFLAGS"], "-mod=mod");
    assert_eq!(spec.env["PATH"], "/custom", "binary.env wins");
    let spec = resolve_launch(&*adapter("gopls"), None, Path::new("/ws"), &launcher).unwrap();
    assert_eq!(spec.env["PATH"], "/fake/bin", "login env otherwise");
}

fn key(root: &str, server: &'static str) -> ServerKey {
    ServerKey {
        root: Arc::from(Path::new(root)),
        server,
    }
}

#[test]
fn settings_diff_restart_vs_reconfigure() {
    let mut reg = Registry::new(builtin_adapters());
    let ra = key("/ws", "rust-analyzer");
    let gopls = key("/ws", "gopls");
    let clangd = key("/ws", "clangd");
    let ts = key("/ws", "typescript-language-server");
    let initial: ServersSettings = serde_json::from_value(json!({
        "rust-analyzer": { "settings": { "a": 1 } },
        "gopls": { "settings": { "a": 1 } },
        "clangd": { "initialization_options": { "x": 1 } },
        "typescript-language-server": { "initialization_options": { "t": 1 } },
    }))
    .unwrap();
    reg.set_settings(initial);
    for k in [&ra, &gopls, &clangd, &ts] {
        reg.set_slot(k.clone(), Slot::Starting);
    }

    let new: ServersSettings = serde_json::from_value(json!({
        "rust-analyzer": { "binary": { "path": "/x" }, "settings": { "a": 1 } },
        "gopls": { "settings": { "a": 2 } },
        "clangd": { "initialization_options": { "x": 1 } },
    }))
    .unwrap();
    let diff = reg.set_settings(new);
    let mut restart = diff.restart.clone();
    restart.sort_by_key(|k| k.server);
    // ts was removed from the map: back to defaults, which differ.
    assert_eq!(restart, [ra, ts]);
    assert_eq!(diff.reconfigure, [(gopls, json!({ "a": 2 }))]);
    assert!(reg.settings("clangd").is_some());
    assert!(reg.settings("typescript-language-server").is_none());

    // A removed `settings` is reported as null.
    let diff = reg.set_settings(ServersSettings::new());
    assert!(diff.reconfigure.iter().any(|(_, v)| v.is_null()));
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
    let reg = Registry::new(builtin_adapters());
    let list = vec!["...".to_owned()];
    let ka = reg.keys_for("rust", &ws.join("crates/a/src/lib.rs"), ws, &list);
    let kb = reg.keys_for("rust", &ws.join("crates/b/src/lib.rs"), ws, &list);
    assert_eq!(ka.len(), 1);
    assert_eq!(ka, kb);
    assert_eq!(&*ka[0].root, ws);
    assert_eq!(ka[0].server, "rust-analyzer");
}

#[test]
fn slots_and_remove() {
    let mut reg = Registry::new(builtin_adapters());
    let k = key("/ws", "gopls");
    assert!(reg.slot(&k).is_none());
    reg.set_slot(k.clone(), Slot::Stopped);
    assert!(matches!(reg.slot(&k), Some(Slot::Stopped)));
    assert_eq!(reg.keys().count(), 1);
    assert_eq!(reg.running().count(), 0);
    assert!(reg.adapter("gopls").is_some());
    assert!(reg.adapter("nope").is_none());
    assert!(matches!(reg.remove(&k), Some(Slot::Stopped)));
    assert!(reg.slot(&k).is_none());
}
