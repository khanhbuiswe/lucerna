use std::path::Path;

use lucerna_lsp::RootSearch;
use lucerna_lsp::table::{Install, LanguageIdMap, ServerDef, ServerTable, merge_json};
use serde_json::json;

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn defaults_parse_without_errors() {
    assert!(ServerTable::defaults().is_object());
    let table = ServerTable::build(None);
    assert!(table.errors.is_empty(), "{:?}", table.errors);
}

#[test]
fn defaults_match_phase3_values() {
    let table = ServerTable::build(None);
    let py = [
        "pyproject.toml",
        "setup.py",
        "setup.cfg",
        "pyrightconfig.json",
    ];
    let ts_langs = [
        "typescript",
        "typescriptreact",
        "javascript",
        "javascriptreact",
    ];
    // (name, command, args, languages, markers, search, probe, enabled)
    #[allow(clippy::type_complexity)]
    let expected: Vec<(
        &str,
        &str,
        &[&str],
        &[&str],
        &[&str],
        RootSearch,
        Option<&[&str]>,
        bool,
    )> = vec![
        (
            "rust-analyzer",
            "rust-analyzer",
            &[],
            &["rust"],
            &["Cargo.toml"],
            RootSearch::Outermost,
            Some(&["--version"]),
            true,
        ),
        (
            "gopls",
            "gopls",
            &[],
            &["go"],
            &["go.work", "go.mod"],
            RootSearch::NearestByMarker,
            Some(&["version"]),
            true,
        ),
        (
            "basedpyright",
            "basedpyright-langserver",
            &["--stdio"],
            &["python"],
            &py,
            RootSearch::Outermost,
            None,
            true,
        ),
        (
            "pyright",
            "pyright-langserver",
            &["--stdio"],
            &["python"],
            &py,
            RootSearch::Outermost,
            None,
            false,
        ),
        (
            "typescript-language-server",
            "typescript-language-server",
            &["--stdio"],
            &ts_langs,
            &["tsconfig.json", "jsconfig.json", "package.json"],
            RootSearch::Outermost,
            Some(&["--version"]),
            true,
        ),
        (
            "clangd",
            "clangd",
            &[],
            &["c", "c++", "objective-c"],
            &["compile_commands.json", ".clangd"],
            RootSearch::Outermost,
            Some(&["--version"]),
            true,
        ),
    ];
    for (name, command, args, langs, markers, search, probe, enabled) in expected {
        let d = table.get(name).unwrap_or_else(|| panic!("{name} missing"));
        assert_eq!(d.command.as_deref(), Some(command), "{name}");
        assert_eq!(d.args, strs(args), "{name}");
        assert_eq!(d.languages, strs(langs), "{name}");
        assert_eq!(d.root_markers, strs(markers), "{name}");
        assert_eq!(d.root_search, search, "{name}");
        assert_eq!(d.probe, probe.map(strs), "{name}");
        assert_eq!(d.enabled, enabled, "{name}");
        assert!(d.env.is_empty() && d.relay.is_none() && d.binary.is_none());
    }
}

#[test]
fn override_settings_only_keeps_default_launch() {
    let t = ServerTable::build(Some(&json!({"rust-analyzer": {"settings": {"a": 1}}})));
    let d = t.get("rust-analyzer").unwrap();
    assert_eq!(d.command.as_deref(), Some("rust-analyzer"));
    assert!(d.args.is_empty());
    assert_eq!(d.settings, Some(json!({"a": 1})));
    assert!(t.errors.is_empty());
}

#[test]
fn user_adds_new_server() {
    let t = ServerTable::build(Some(
        &json!({"mylang-ls": {"command": "mylang-ls", "file_types": ["ml"]}}),
    ));
    let d = t.get("mylang-ls").unwrap();
    assert!(d.enabled);
    assert_eq!(d.probe, Some(strs(&["--version"])));
    assert_eq!(d.root_search, RootSearch::Outermost);
    assert!(t.errors.is_empty());
}

#[test]
fn arrays_replace_objects_merge() {
    let mut base = json!({"args": ["a", "b"], "env": {"A": "1"}, "n": {"x": 1}});
    merge_json(
        &mut base,
        &json!({"args": ["x"], "env": {"B": "2"}, "n": 5}),
    );
    assert_eq!(
        base,
        json!({"args": ["x"], "env": {"A": "1", "B": "2"}, "n": 5})
    );

    let t = ServerTable::build(Some(&json!({
        "basedpyright": {"args": ["x"], "env": {"K": "v"}}
    })));
    let d = t.get("basedpyright").unwrap();
    assert_eq!(d.args, strs(&["x"]));
    assert_eq!(d.env.get("K").map(String::as_str), Some("v"));
}

#[test]
fn bad_entry_is_reported_and_skipped() {
    let t = ServerTable::build(Some(&json!({
        "bad": {"args": "--stdio"},
        "empty": {"command": "x"}
    })));
    assert_eq!(t.errors.len(), 2, "{:?}", t.errors);
    assert!(t.get("bad").is_none() && t.get("empty").is_none());
    assert!(t.get("rust-analyzer").is_some());
    let empty = t.errors.iter().find(|e| e.server == "empty").unwrap();
    assert_eq!(empty.message, "missing `languages` or `file_types`");
    assert!(t.errors.iter().any(|e| e.server == "bad"));

    let t = ServerTable::build(Some(&json!({"nocmd": {"languages": ["x"]}})));
    assert_eq!(t.errors[0].message, "missing `command`");
}

fn def(languages: &[&str], file_types: &[&str], map: Option<LanguageIdMap>) -> ServerDef {
    let mut t = ServerTable::build(Some(&json!({"t": {
        "command": "t", "languages": languages, "file_types": file_types
    }})));
    let mut d = t.servers.remove("t").unwrap().clone();
    d.language_id = map;
    d
}

#[test]
fn language_id_rules() {
    let one = def(&["a", "b"], &[], Some(LanguageIdMap::One("x".into())));
    assert_eq!(one.lsp_language_id(Some("a")), "x");
    assert_eq!(one.lsp_language_id(None), "x");

    let map = LanguageIdMap::Map([("vue.js".to_owned(), "vue".to_owned())].into());
    let m = def(&["vue.js", "b"], &[], Some(map));
    assert_eq!(m.lsp_language_id(Some("vue.js")), "vue");
    assert_eq!(m.lsp_language_id(Some("b")), "b");
    assert_eq!(m.lsp_language_id(None), "vue.js");

    let none = def(&["first", "b"], &[], None);
    assert_eq!(none.lsp_language_id(Some("b")), "b");
    assert_eq!(none.lsp_language_id(None), "first");
}

#[test]
fn matches_by_language_and_file_type() {
    let d = def(&["ruby"], &["rb", "Gemfile"], None);
    assert!(d.matches(Some("ruby"), Path::new("/x/a.txt")));
    assert!(
        !d.matches(Some("python"), Path::new("/x/a.rb")),
        "language present skips file_types"
    );
    assert!(d.matches(None, Path::new("/x/a.rb")));
    assert!(d.matches(None, Path::new("/x/a.RB")));
    assert!(d.matches(None, Path::new("/x/Gemfile")));
    assert!(!d.matches(None, Path::new("/x/gemfile")));
    assert!(!d.matches(None, Path::new("/x/a.py")));
}

#[test]
fn root_search_serde() {
    let r: RootSearch = serde_json::from_value(json!("ordered")).unwrap();
    assert_eq!(r, RootSearch::NearestByMarker);
    let r: RootSearch = serde_json::from_value(json!("outermost")).unwrap();
    assert_eq!(r, RootSearch::Outermost);
}

#[test]
fn install_for_os() {
    let one = Install::One("cmd".into());
    let mut d = def(&["a"], &[], None);
    d.install = Some(one);
    assert_eq!(d.install_for("macos"), Some("cmd"));
    d.install = Some(Install::PerOs(
        [("linux".to_owned(), "apt".to_owned())].into(),
    ));
    assert_eq!(d.install_for("linux"), Some("apt"));
    assert_eq!(d.install_for("macos"), None);
    d.install = None;
    assert_eq!(d.install_for("linux"), None);
}

#[test]
fn launch_fields_eq_ignores_settings() {
    let a = ServerTable::build(None);
    let b = ServerTable::build(Some(
        &json!({"clangd": {"settings": {"q": 1}, "note": "n"}}),
    ));
    let (x, y) = (a.get("clangd").unwrap(), b.get("clangd").unwrap());
    assert!(x.launch_fields_eq(y));
    let c = ServerTable::build(Some(&json!({"clangd": {"args": ["--x"]}})));
    assert!(!x.launch_fields_eq(c.get("clangd").unwrap()));
}

#[test]
fn bad_override_of_default_keeps_the_default() {
    let t = ServerTable::build(Some(&json!({"rust-analyzer": {"args": "x"}})));
    assert_eq!(t.errors.len(), 1, "{:?}", t.errors);
    assert_eq!(t.errors[0].server, "rust-analyzer");
    let ra = t.get("rust-analyzer").expect("default kept");
    let default = ServerTable::build(None);
    assert_eq!(ra, default.get("rust-analyzer").unwrap());
    assert!(ra.args.is_empty());
}

#[test]
fn launch_fields_include_probe() {
    let a = ServerTable::build(None);
    let b = ServerTable::build(Some(&json!({"gopls": {"probe": ["-v"]}})));
    let c = ServerTable::build(Some(&json!({"gopls": {"probe": false}})));
    let gopls = |t: &ServerTable| t.get("gopls").unwrap().clone();
    assert!(gopls(&a).launch_fields_eq(&gopls(&a)));
    assert!(!gopls(&a).launch_fields_eq(&gopls(&b)));
    assert!(!gopls(&a).launch_fields_eq(&gopls(&c)));
}

const SPEC_SERVERS: &[&str] = &[
    "rust-analyzer",
    "gopls",
    "basedpyright",
    "pyright",
    "typescript-language-server",
    "intelephense",
    "phpactor",
    "ruby-lsp",
    "solargraph",
    "jdtls",
    "kotlin-lsp",
    "kotlin-language-server",
    "sourcekit-lsp",
    "lua-language-server",
    "elixir-ls",
    "expert",
    "zls",
    "dart",
    "vue-language-server",
    "vtsls",
    "clangd",
    "csharp-ls",
    "roslyn-language-server",
    "haskell-language-server",
    "ocamllsp",
    "metals",
    "terraform-ls",
    "docker-langserver",
    "yaml-language-server",
    "vscode-json-language-server",
    "vscode-css-language-server",
    "vscode-html-language-server",
    "bash-language-server",
    "taplo",
    "marksman",
    "nil",
    "nixd",
    "gleam",
    "elp",
];

#[test]
fn spec_rows_present() {
    let table = ServerTable::build(None);
    for name in SPEC_SERVERS {
        assert!(table.get(name).is_some(), "{name} missing");
    }
    assert_eq!(table.servers.len(), SPEC_SERVERS.len());
}

#[test]
fn every_default_entry_is_complete() {
    let table = ServerTable::build(None);
    for def in table.servers.values() {
        let n = &def.name;
        assert!(def.command.is_some(), "{n} command");
        assert!(
            !def.languages.is_empty() || !def.file_types.is_empty(),
            "{n} languages"
        );
        assert!(def.install.is_some(), "{n} install");
        assert!(
            def.homepage
                .as_deref()
                .is_some_and(|h| h.starts_with("https://")),
            "{n} homepage"
        );
    }
}

#[test]
fn one_enabled_server_per_language() {
    let table = ServerTable::build(None);
    let mut count: std::collections::BTreeMap<&str, Vec<&str>> = Default::default();
    for def in table.servers.values().filter(|d| d.enabled) {
        for lang in &def.languages {
            count.entry(lang).or_default().push(&def.name);
        }
    }
    for (lang, servers) in &count {
        let allowed = if *lang == "vue.js" { 2 } else { 1 };
        assert_eq!(servers.len(), allowed, "{lang}: {servers:?}");
    }
    for alt in [
        "pyright",
        "phpactor",
        "solargraph",
        "kotlin-language-server",
        "expert",
        "roslyn-language-server",
        "nixd",
    ] {
        assert!(!table.get(alt).unwrap().enabled, "{alt} should be disabled");
    }
}

#[test]
fn install_commands_are_single_copyable_commands() {
    let table = ServerTable::build(None);
    for def in table.servers.values() {
        let values: Vec<&String> = match def.install.as_ref().unwrap() {
            Install::One(s) => vec![s],
            Install::PerOs(m) => m.values().collect(),
        };
        for v in values {
            assert!(
                !v.contains(", or ") && !v.contains(" or "),
                "{}: {v}",
                def.name
            );
            assert!(
                !v.trim().is_empty() && !v.contains('\n'),
                "{}: {v}",
                def.name
            );
        }
    }
}

#[test]
fn vue_entries_follow_the_spec() {
    let table = ServerTable::build(None);
    let vue = table.get("vue-language-server").unwrap();
    assert_eq!(vue.relay, Some(lucerna_lsp::table::Relay::VueTsserver));
    assert_eq!(vue.lsp_language_id(Some("vue.js")), "vue");
    let vtsls = table.get("vtsls").unwrap();
    let location =
        &vtsls.settings.as_ref().unwrap()["vtsls"]["tsserver"]["globalPlugins"][0]["location"];
    assert_eq!(location, "{npm_root}/@vue/language-server");
    let jdtls = table.get("jdtls").unwrap();
    assert_eq!(jdtls.args, strs(&["-data", "{data_dir}"]));
}

#[test]
fn language_id_mappings() {
    let table = ServerTable::build(None);
    assert_eq!(
        table.get("clangd").unwrap().lsp_language_id(Some("c++")),
        "cpp"
    );
    assert_eq!(
        table
            .get("clangd")
            .unwrap()
            .lsp_language_id(Some("objective-c")),
        "objective-c"
    );
    assert_eq!(
        table.get("csharp-ls").unwrap().lsp_language_id(Some("c#")),
        "csharp"
    );
    assert_eq!(
        table
            .get("bash-language-server")
            .unwrap()
            .lsp_language_id(Some("shellscript")),
        "shellscript"
    );
}
