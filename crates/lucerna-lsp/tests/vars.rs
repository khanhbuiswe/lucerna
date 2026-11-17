use std::path::{Path, PathBuf};

use lucerna_lsp::table::ServerTable;
use lucerna_lsp::test_support::FakeLauncher;
use lucerna_lsp::vars::{
    VarError, Vars, data_dir_for, expand, expand_value, npm_root, uses_npm_root,
};
use serde_json::json;
use sha2::{Digest, Sha256};

fn vars() -> Vars {
    Vars {
        root: PathBuf::from("/ws"),
        data_dir: PathBuf::from("/data/lsp/x/abc"),
        npm_root: None,
    }
}

#[test]
fn expands_root_and_data_dir() {
    let v = vars();
    assert_eq!(expand("-data", &v).unwrap(), "-data");
    assert_eq!(expand("{data_dir}", &v).unwrap(), "/data/lsp/x/abc");
    assert_eq!(
        expand("--root={root}/sub:{root}", &v).unwrap(),
        "--root=/ws/sub:/ws"
    );
    // Braces that are not a variable name stay as they are.
    assert_eq!(expand(r#"{"a":1}"#, &v).unwrap(), r#"{"a":1}"#);
    assert_eq!(expand("{} {", &v).unwrap(), "{} {");
}

#[test]
fn unknown_variable_errors() {
    assert_eq!(
        expand("x{nope}", &vars()),
        Err(VarError::Unknown("nope".to_owned()))
    );
    assert_eq!(
        VarError::Unknown("nope".to_owned()).to_string(),
        "unknown variable {nope}"
    );
}

#[test]
fn npm_root_missing_errors() {
    assert_eq!(
        expand("{npm_root}/@vue/language-server", &vars()),
        Err(VarError::NpmRootUnavailable)
    );
    assert_eq!(
        VarError::NpmRootUnavailable.to_string(),
        "npm not found; {npm_root} unavailable"
    );
    let v = Vars {
        npm_root: Some(PathBuf::from("/usr/lib/node_modules")),
        ..vars()
    };
    assert_eq!(
        expand("{npm_root}/@vue/language-server", &v).unwrap(),
        "/usr/lib/node_modules/@vue/language-server"
    );
}

#[test]
fn expand_value_walks_nested_strings() {
    let value = json!({
        "a": "{root}",
        "{root}": 1,
        "list": ["{data_dir}", 2, null, { "deep": "{root}/x" }],
        "flag": true,
    });
    let out = expand_value(&value, &vars()).unwrap();
    assert_eq!(
        out,
        json!({
            "a": "/ws",
            "{root}": 1,
            "list": ["/data/lsp/x/abc", 2, null, { "deep": "/ws/x" }],
            "flag": true,
        })
    );
    assert_eq!(
        expand_value(&json!(["{bad}"]), &vars()),
        Err(VarError::Unknown("bad".to_owned()))
    );
}

#[test]
fn data_dir_is_stable_and_server_scoped() {
    let base = Path::new("/base");
    let a = data_dir_for(base, "jdtls", Path::new("/ws/a"), 0);
    assert_eq!(a, data_dir_for(base, "jdtls", Path::new("/ws/a"), 0));
    assert!(a.starts_with("/base/lsp/jdtls"));
    let hash = a.file_name().unwrap().to_str().unwrap();
    assert_eq!(hash.len(), 12);
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(a, data_dir_for(base, "jdtls", Path::new("/ws/b"), 0));
    assert_ne!(
        a.file_name(),
        data_dir_for(base, "jdtls", Path::new("/ws/b"), 0).file_name()
    );
    assert!(data_dir_for(base, "other", Path::new("/ws/a"), 0).starts_with("/base/lsp/other"));

    let legacy: String = Sha256::digest(b"/ws/a")
        .iter()
        .take(6)
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(
        data_dir_for(base, "jdtls", Path::new("/ws/a"), 0),
        base.join("lsp/jdtls").join(legacy)
    );
    let one = data_dir_for(base, "jdtls", Path::new("/ws/a"), 1);
    assert_ne!(one, data_dir_for(base, "jdtls", Path::new("/ws/a"), 0));
    assert_eq!(one, data_dir_for(base, "jdtls", Path::new("/ws/a"), 1));
    assert_ne!(one, data_dir_for(base, "jdtls", Path::new("/ws/a"), 2));
}

#[test]
fn npm_root_reads_launcher_output() {
    let launcher = FakeLauncher::new().with_output("npm", &["root", "-g"], "/opt/node/lib\n");
    assert_eq!(npm_root(&launcher), Some(PathBuf::from("/opt/node/lib")));
    assert_eq!(npm_root(&FakeLauncher::new()), None);
    let blank = FakeLauncher::new().with_output("npm", &["root", "-g"], "  \n");
    assert_eq!(npm_root(&blank), None);
}

#[test]
fn uses_npm_root_scans_launch_and_config_fields() {
    let table = ServerTable::build(Some(&json!({
        "a": { "command": "a", "languages": ["x"], "args": ["{npm_root}/a"] },
        "b": { "command": "b", "languages": ["x"],
               "settings": { "p": [{ "location": "{npm_root}/b" }] } },
        "c": { "command": "c", "languages": ["x"], "env": { "N": "{npm_root}" } },
        "d": { "command": "d", "languages": ["x"], "args": ["{root}"] },
    })));
    assert!(uses_npm_root(table.get("a").unwrap()));
    assert!(uses_npm_root(table.get("b").unwrap()));
    assert!(uses_npm_root(table.get("c").unwrap()));
    assert!(!uses_npm_root(table.get("d").unwrap()));
    assert!(!uses_npm_root(table.get("rust-analyzer").unwrap()));
}
