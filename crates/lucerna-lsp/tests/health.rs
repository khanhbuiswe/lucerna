//! Health: which servers in the table are installed (spec §7).

use std::path::PathBuf;

use lucerna_lsp::ServerTable;
use lucerna_lsp::health::{HealthRow, HealthStatus, current_os, format_table, health, health_in};
use lucerna_lsp::test_support::{FakeLauncher, FakeServer};
use serde_json::json;

fn launcher(binaries: &[&str]) -> FakeLauncher {
    binaries.iter().fold(FakeLauncher::new(), |l, name| {
        l.with_server(name, FakeServer::new())
    })
}

fn row<'a>(rows: &'a [HealthRow], language: &str, server: &str) -> &'a HealthRow {
    rows.iter()
        .find(|r| r.language == language && &*r.server == server)
        .unwrap_or_else(|| panic!("no row ({language}, {server}) in {rows:#?}"))
}

#[test]
fn found_not_found_disabled() {
    let setting = json!({
        "gopls": { "enabled": false },
        "mine": { "command": "/nowhere/mine-ls", "languages": ["mine"] },
        "pinned": {
            "command": "pinned-ls",
            "languages": ["pinned"],
            "binary": { "path": "/definitely/missing/pinned-ls" }
        }
    });
    let table = ServerTable::build(Some(&setting));
    let rows = health(&table, &launcher(&["rust-analyzer"]), "macos");

    let rust = row(&rows, "rust", "rust-analyzer");
    assert_eq!(
        rust.status,
        HealthStatus::Found(PathBuf::from("/fake/bin/rust-analyzer"))
    );
    assert_eq!(
        rust.install.as_deref(),
        Some("rustup component add rust-analyzer")
    );

    let ts = row(&rows, "typescript", "typescript-language-server");
    assert_eq!(ts.status, HealthStatus::NotFound);
    assert_eq!(
        ts.install.as_deref(),
        Some("npm i -g typescript typescript-language-server")
    );

    assert_eq!(row(&rows, "go", "gopls").status, HealthStatus::Disabled);
    assert_eq!(
        row(&rows, "python", "pyright").status,
        HealthStatus::Disabled
    );

    let mine = row(&rows, "mine", "mine");
    assert_eq!(mine.status, HealthStatus::NotFound);
    assert_eq!(mine.install, None);

    // A `binary.path` that does not exist is not found either.
    assert_eq!(
        row(&rows, "pinned", "pinned").status,
        HealthStatus::NotFound
    );

    // Install commands follow the OS; notes come from the table.
    let clangd = row(&rows, "c", "clangd");
    assert_eq!(clangd.install.as_deref(), Some("xcode-select --install"));
    assert!(clangd.note.is_some());
    let linux = health(&table, &launcher(&[]), "linux");
    let kotlin = row(&linux, "kotlin", "kotlin-lsp");
    assert_eq!(kotlin.install, None, "kotlin-lsp has no Linux command");
}

#[test]
fn binary_path_that_exists_is_found() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pinned-ls");
    std::fs::write(&path, "").unwrap();
    let setting = json!({
        "pinned": {
            "command": "pinned-ls",
            "languages": ["pinned"],
            "binary": { "path": path }
        }
    });
    let table = ServerTable::build(Some(&setting));
    let rows = health(&table, &launcher(&[]), "macos");
    assert_eq!(
        row(&rows, "pinned", "pinned").status,
        HealthStatus::Found(path)
    );
}

#[test]
fn language_rows_and_order() {
    let setting = json!({
        "filetype-only": { "command": "ft-ls", "file_types": ["zzz", "yyy"] }
    });
    let table = ServerTable::build(Some(&setting));
    let rows = health(&table, &launcher(&[]), "macos");

    // One row per language of a server.
    let ts: Vec<&str> = rows
        .iter()
        .filter(|r| &*r.server == "typescript-language-server")
        .map(|r| r.language.as_str())
        .collect();
    assert_eq!(
        ts,
        [
            "javascript",
            "javascriptreact",
            "typescript",
            "typescriptreact"
        ]
    );
    // Without languages, the first file type names the row.
    let ft: Vec<&str> = rows
        .iter()
        .filter(|r| &*r.server == "filetype-only")
        .map(|r| r.language.as_str())
        .collect();
    assert_eq!(ft, ["zzz"]);

    // Sorted by language, then enabled before disabled.
    let languages: Vec<&str> = rows.iter().map(|r| r.language.as_str()).collect();
    let mut sorted = languages.clone();
    sorted.sort_unstable();
    assert_eq!(languages, sorted);
    let python: Vec<&str> = rows
        .iter()
        .filter(|r| r.language == "python")
        .map(|r| &*r.server)
        .collect();
    assert_eq!(python, ["basedpyright", "pyright"]);
    // Not alphabetical: the enabled kotlin-lsp comes first.
    let kotlin: Vec<&str> = rows
        .iter()
        .filter(|r| r.language == "kotlin")
        .map(|r| &*r.server)
        .collect();
    assert_eq!(kotlin, ["kotlin-lsp", "kotlin-language-server"]);
}

#[test]
fn format_table_columns() {
    let rows = vec![
        HealthRow {
            language: "rust".into(),
            server: "rust-analyzer".into(),
            status: HealthStatus::Found(PathBuf::from("/bin/ra")),
            install: Some("rustup component add rust-analyzer".into()),
            note: None,
        },
        HealthRow {
            language: "python".into(),
            server: "pyright".into(),
            status: HealthStatus::Disabled,
            install: Some("npm i -g pyright".into()),
            note: None,
        },
        HealthRow {
            language: "go".into(),
            server: "gopls".into(),
            status: HealthStatus::NotFound,
            install: None,
            note: Some("a note".into()),
        },
    ];
    let expected = "\
Language  Server         Status       Install
rust      rust-analyzer  ✓ /bin/ra    rustup component add rust-analyzer
python    pyright        disabled     npm i -g pyright
go        gopls          ✗ not found
";
    assert_eq!(format_table(&rows), expected);
}

#[test]
fn current_os_is_known() {
    assert!(["macos", "linux", "windows"].contains(&current_os()));
    #[cfg(target_os = "macos")]
    assert_eq!(current_os(), "macos");
}

/// Records the folder each probe runs in; every binary is found.
#[derive(Default)]
struct ProbeDirs(std::sync::Mutex<Vec<PathBuf>>);

impl lucerna_lsp::Launcher for ProbeDirs {
    fn env(&self) -> std::collections::HashMap<String, String> {
        std::collections::HashMap::new()
    }
    fn which(&self, name: &str) -> Option<PathBuf> {
        Some(PathBuf::from("/fake/bin").join(name))
    }
    fn probe(&self, _: &std::path::Path, _: &[&str], cwd: &std::path::Path) -> bool {
        self.0.lock().unwrap().push(cwd.to_path_buf());
        true
    }
    fn output(&self, _: &str, _: &[&str]) -> Option<String> {
        None
    }
    fn launch(
        &self,
        _: &lucerna_lsp::transport::LaunchSpec,
        _: std::sync::Arc<dyn lucerna_lsp::transport::Inbox>,
    ) -> std::io::Result<std::sync::Arc<dyn lucerna_lsp::transport::Transport>> {
        Err(std::io::ErrorKind::NotFound.into())
    }
}

#[test]
fn probes_run_in_the_given_folder() {
    let setting = json!({
        "probed": { "command": "probed-ls", "languages": ["probed"], "probe": ["--version"] }
    });
    let table = ServerTable::build(Some(&setting));
    let dir = tempfile::tempdir().unwrap();
    let launcher = ProbeDirs::default();
    let rows = health_in(&table, &launcher, "macos", dir.path());
    assert!(matches!(
        row(&rows, "probed", "probed").status,
        HealthStatus::Found(_)
    ));
    let dirs = launcher.0.lock().unwrap().clone();
    assert!(!dirs.is_empty());
    assert!(dirs.iter().all(|d| d == dir.path()), "{dirs:?}");
}
