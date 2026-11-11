//! `luc lsp health`: the table the library function prints.

use lucerna::lsp_cli::run_health;
use lucerna_lsp::ServerTable;
use lucerna_lsp::test_support::{FakeLauncher, FakeServer};
use serde_json::json;

#[test]
fn lsp_health_snapshot() {
    let setting = json!({
        "gopls": { "enabled": false },
        "mine": { "command": "mine-ls", "languages": ["mine"] }
    });
    let mut table = ServerTable::build(Some(&setting));
    // Servers whose install command is the same on every OS, so the
    // snapshot does not depend on the machine.
    table.servers.retain(|name, _| {
        [
            "rust-analyzer",
            "gopls",
            "basedpyright",
            "pyright",
            "typescript-language-server",
            "mine",
        ]
        .contains(&&**name)
    });
    let launcher = FakeLauncher::new()
        .with_server("rust-analyzer", FakeServer::new())
        .with_server("basedpyright-langserver", FakeServer::new());
    let mut out = Vec::new();
    let code = run_health(&table, &launcher, &mut out);
    assert_eq!(code, 0);
    insta::assert_snapshot!(String::from_utf8(out).unwrap());
}

#[test]
fn lsp_health_reports_table_errors() {
    let setting = json!({ "broken": { "languages": ["x"] } });
    let table = ServerTable::build(Some(&setting));
    let mut out = Vec::new();
    assert_eq!(run_health(&table, &FakeLauncher::new(), &mut out), 0);
    let out = String::from_utf8(out).unwrap();
    assert!(
        out.contains("Invalid `lsp.broken`: missing `command`"),
        "{out}"
    );
}
