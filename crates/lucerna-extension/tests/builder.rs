use lucerna_extension::builder::{BuildError, BuildOutput, Stage, build};
use lucerna_extension::process::CancelToken;
use lucerna_extension::test_support::{
    FakeValidator, add_commit, fake_clang_calls, fake_repo, fake_wasi_sdk, file_url,
};
use std::cell::RefCell;
use std::path::{Path, PathBuf};

const THEME: &str = r#"{
  // lenient JSON
  "name": "X", "author": "t",
  "themes": [{ "name": "X Dark", "appearance": "dark", "style": {}, }],
}"#;

const SNIPPETS: &str = r#"{ "p": { "prefix": "p", "body": "print($1)" } }"#;

fn wasm_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../lucerna-syntax/tests/fixtures/wasm/json.wasm")
}

fn write_tree(dir: &Path, files: &[(&str, &str)]) {
    for (rel, content) in files {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }
}

struct Env {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    src: PathBuf,
}

impl Env {
    fn new(files: &[(&str, &str)]) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("store");
        let src = tmp.path().join("ext");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&src).unwrap();
        write_tree(&src, files);
        Self {
            _tmp: tmp,
            root,
            src,
        }
    }

    fn with_sdk(self) -> Self {
        fake_wasi_sdk(&self.sdk(), &wasm_fixture());
        self
    }

    fn sdk(&self) -> PathBuf {
        self.root.join("toolchain/wasi-sdk-34")
    }

    fn build(&self, expected: Option<&str>) -> (Result<BuildOutput, BuildError>, Vec<Stage>) {
        let stages = RefCell::new(Vec::new());
        let r = build(
            &self.src,
            &self.root,
            "ext",
            expected,
            &FakeValidator,
            &CancelToken::default(),
            &mut std::io::sink(),
            &|s| stages.borrow_mut().push(s),
        );
        (r, stages.into_inner())
    }

    fn staging_entries(&self) -> Vec<PathBuf> {
        std::fs::read_dir(self.root.join("staging"))
            .map(|rd| rd.map(|e| e.unwrap().path()).collect())
            .unwrap_or_default()
    }
}

fn manifest(extra: &str) -> String {
    format!("id = \"ext\"\nname = \"Ext\"\nversion = \"0.1.0\"\nschema_version = 1\n{extra}")
}

/// A grammar repo with `files` and the `[grammars.<name>]` manifest entry.
fn grammar(name: &str, files: &[(&str, &str)]) -> String {
    let (repo, commit) = fake_repo(files);
    format!(
        "[grammars.{name}]\nrepository = \"{}\"\nrev = \"{commit}\"\n",
        file_url(&repo)
    )
}

fn lang_config(name: &str, grammar: &str) -> String {
    format!("name = \"{name}\"\ngrammar = \"{grammar}\"\npath_suffixes = [\"{grammar}\"]\n")
}

const PARSER: (&str, &str) = ("src/parser.c", "/* parser */");

#[test]
fn build_theme_only_extension() {
    let env = Env::new(&[("extension.toml", &manifest("")), ("themes/x.json", THEME)]);
    let (r, stages) = env.build(Some("0.1.0"));
    let out = r.unwrap();
    assert!(out.staging.starts_with(env.root.join("staging")));
    assert!(out.staging.join("extension.toml").is_file());
    assert!(out.staging.join("themes/x.json").is_file());
    assert_eq!(out.manifest.themes, [PathBuf::from("themes/x.json")]);
    assert!(!env.root.join("toolchain").exists());
    assert_eq!(
        stages,
        [Stage::ReadingManifest, Stage::Validating, Stage::Installing]
    );
    // The written manifest round-trips.
    let back = lucerna_extension::load_manifest(&out.staging).unwrap();
    assert_eq!(back.id, "ext");
    assert_eq!(back.themes, [PathBuf::from("themes/x.json")]);
}

#[test]
fn build_language_with_grammar() {
    let env = Env::new(&[
        ("extension.toml", &manifest(&grammar("json", &[PARSER]))),
        ("languages/json/config.toml", &lang_config("JSON", "json")),
        ("languages/json/highlights.scm", "(string) @string"),
    ])
    .with_sdk();
    let (r, stages) = env.build(Some("0.1.0"));
    let out = r.unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let s = &out.staging;
    assert_eq!(
        std::fs::read(s.join("grammars/json.wasm")).unwrap(),
        std::fs::read(wasm_fixture()).unwrap()
    );
    assert!(s.join("languages/json/config.toml").is_file());
    assert!(s.join("languages/json/highlights.scm").is_file());
    assert!(out.manifest.grammars.contains_key("json"));
    let back = lucerna_extension::load_manifest(s).unwrap();
    assert_eq!(back.grammars, out.manifest.grammars);
    assert_eq!(back.languages, [PathBuf::from("languages/json")]);
    assert_eq!(
        stages,
        [
            Stage::ReadingManifest,
            Stage::PreparingToolchain,
            Stage::BuildingGrammar {
                name: "json".into(),
                index: 0,
                total: 1
            },
            Stage::Validating,
            Stage::Installing,
        ]
    );
    // The exact compile command.
    let calls = std::fs::read_to_string(env.sdk().join("calls")).unwrap();
    let src = env.root.join("work/ext/grammars/json/src");
    let src = src.canonicalize().unwrap();
    let out_dir = env.root.join("work/ext/grammars").canonicalize().unwrap();
    assert_eq!(
        calls.trim(),
        format!(
            "-fPIC -shared -Os -Wl,--export=tree_sitter_json -Wl,--max-memory=4294967296 -o {}/json.wasm -I {} {}/parser.c",
            out_dir.display(),
            src.display(),
            src.display()
        )
    );
}

#[test]
fn grammar_cache_reused_on_second_build() {
    let env = Env::new(&[
        ("extension.toml", &manifest(&grammar("json", &[PARSER]))),
        ("languages/json/config.toml", &lang_config("JSON", "json")),
    ])
    .with_sdk();
    let first = env.build(None).0.unwrap();
    let second = env.build(None).0.unwrap();
    assert_eq!(fake_clang_calls(&env.sdk()), 1);
    assert!(second.staging.join("grammars/json.wasm").is_file());
    assert_ne!(first.staging, second.staging);
    assert!(env.root.join("work/ext/grammars/json.wasm").is_file());
    assert!(env.root.join("work/ext/grammars/json.rev").is_file());
}

#[test]
fn cpp_scanner_drops_language_with_warning() {
    let extra = format!(
        "{}{}",
        grammar("cpp", &[PARSER, ("src/scanner.cc", "// c++")]),
        grammar("json", &[PARSER])
    );
    let env = Env::new(&[
        ("extension.toml", &manifest(&extra)),
        ("languages/cpp/config.toml", &lang_config("Cpp", "cpp")),
        ("languages/json/config.toml", &lang_config("JSON", "json")),
    ])
    .with_sdk();
    let out = env.build(None).0.unwrap();
    assert!(
        out.warnings.iter().any(|w| w.contains("C++ scanner")),
        "{:?}",
        out.warnings
    );
    assert_eq!(out.manifest.languages, [PathBuf::from("languages/json")]);
    assert!(!out.manifest.grammars.contains_key("cpp"));
    assert!(!out.staging.join("languages/cpp").exists());
    assert!(!out.staging.join("grammars/cpp.wasm").exists());
    assert!(out.staging.join("languages/json/config.toml").is_file());
    assert_eq!(fake_clang_calls(&env.sdk()), 1);
}

#[test]
fn missing_parser_drops_language() {
    let extra = format!(
        "{}{}",
        grammar("empty", &[("README", "no parser")]),
        grammar("json", &[PARSER])
    );
    let env = Env::new(&[
        ("extension.toml", &manifest(&extra)),
        (
            "languages/empty/config.toml",
            &lang_config("Empty", "empty"),
        ),
        ("languages/json/config.toml", &lang_config("JSON", "json")),
    ])
    .with_sdk();
    let out = env.build(None).0.unwrap();
    assert!(
        out.warnings.iter().any(|w| w.contains("parser.c")),
        "{:?}",
        out.warnings
    );
    assert_eq!(out.manifest.languages, [PathBuf::from("languages/json")]);
}

#[test]
fn grammar_failing_to_load_drops_language() {
    let extra = format!(
        "{}{}",
        grammar("bad", &[PARSER]),
        grammar("json", &[PARSER])
    );
    let env = Env::new(&[
        ("extension.toml", &manifest(&extra)),
        ("languages/bad/config.toml", &lang_config("Bad", "bad")),
        ("languages/json/config.toml", &lang_config("JSON", "json")),
    ])
    .with_sdk();
    let out = env.build(None).0.unwrap();
    assert_eq!(out.manifest.languages, [PathBuf::from("languages/json")]);
    assert!(!out.staging.join("grammars/bad.wasm").exists());
    assert!(out.warnings.iter().any(|w| w.contains("bad")));
}

#[test]
fn bad_query_dropped_language_kept() {
    let env = Env::new(&[
        ("extension.toml", &manifest(&grammar("json", &[PARSER]))),
        ("languages/json/config.toml", &lang_config("JSON", "json")),
        ("languages/json/highlights.scm", "bad"),
        ("languages/json/outline.scm", "bad"),
        ("languages/json/brackets.scm", "(\"[\" @open)"),
    ])
    .with_sdk();
    let out = env.build(None).0.unwrap();
    assert_eq!(out.manifest.languages, [PathBuf::from("languages/json")]);
    let dir = out.staging.join("languages/json");
    assert!(dir.join("config.toml").is_file());
    assert!(dir.join("brackets.scm").is_file());
    assert!(!dir.join("highlights.scm").exists());
    assert!(!dir.join("outline.scm").exists());
    assert_eq!(
        out.warnings
            .iter()
            .filter(|w| w.contains("highlights") || w.contains("outline"))
            .count(),
        2,
        "{:?}",
        out.warnings
    );
}

#[test]
fn nothing_usable_fails() {
    let env = Env::new(&[
        ("extension.toml", &manifest(&grammar("bad", &[PARSER]))),
        ("languages/bad/config.toml", &lang_config("Bad", "bad")),
        ("themes/broken.json", "{ not json"),
    ])
    .with_sdk();
    let (r, _) = env.build(None);
    match r {
        Err(BuildError::NothingUsable(w)) => assert!(w.len() >= 2, "{w:?}"),
        other => panic!("expected NothingUsable, got {:?}", other.map(|o| o.staging)),
    }
    assert!(env.staging_entries().is_empty());
}

#[test]
fn version_mismatch_fails() {
    let env = Env::new(&[("extension.toml", &manifest("")), ("themes/x.json", THEME)]);
    match env.build(Some("9.9.9")).0 {
        Err(BuildError::VersionMismatch { registry, manifest }) => {
            assert_eq!(registry, "9.9.9");
            assert_eq!(manifest, "0.1.0");
        }
        other => panic!(
            "expected VersionMismatch, got {:?}",
            other.map(|o| o.staging)
        ),
    }
    assert!(env.staging_entries().is_empty());
}

#[test]
fn cancel_during_grammar_build_cleans_staging() {
    let env = Env::new(&[
        ("extension.toml", &manifest(&grammar("json", &[PARSER]))),
        ("languages/json/config.toml", &lang_config("JSON", "json")),
        ("themes/x.json", THEME),
    ])
    .with_sdk();
    let cancel = CancelToken::default();
    let r = build(
        &env.src,
        &env.root,
        "ext",
        None,
        &FakeValidator,
        &cancel,
        &mut std::io::sink(),
        &|s| {
            if matches!(s, Stage::BuildingGrammar { .. }) {
                cancel.cancel();
            }
        },
    );
    assert!(
        matches!(r, Err(BuildError::Cancelled)),
        "{:?}",
        r.map(|o| o.staging)
    );
    assert!(env.staging_entries().is_empty());
}

#[test]
fn symlinked_themes_dir_outside_is_dropped() {
    let env = Env::new(&[
        ("extension.toml", &manifest("")),
        ("snippets.json", SNIPPETS),
    ]);
    let outside = tempfile::tempdir().unwrap();
    write_tree(outside.path(), &[("x.json", THEME)]);
    std::os::unix::fs::symlink(outside.path(), env.src.join("themes")).unwrap();
    let out = env.build(None).0.unwrap();
    assert!(out.manifest.themes.is_empty());
    assert!(!out.staging.join("themes").exists());
    assert!(
        out.warnings.iter().any(|w| w.contains("outside")),
        "{:?}",
        out.warnings
    );
    assert!(out.staging.join("snippets.json").is_file());
}

#[test]
fn symlinked_query_file_outside_is_dropped() {
    let env = Env::new(&[
        ("extension.toml", &manifest(&grammar("json", &[PARSER]))),
        ("languages/json/config.toml", &lang_config("JSON", "json")),
    ])
    .with_sdk();
    let outside = tempfile::tempdir().unwrap();
    write_tree(outside.path(), &[("secret.scm", "(x) @x")]);
    std::os::unix::fs::symlink(
        outside.path().join("secret.scm"),
        env.src.join("languages/json/highlights.scm"),
    )
    .unwrap();
    let out = env.build(None).0.unwrap();
    assert!(!out.staging.join("languages/json/highlights.scm").exists());
    assert!(out.warnings.iter().any(|w| w.contains("outside")));
}

#[test]
fn grammar_path_symlink_outside_checkout_drops_language() {
    let outside = tempfile::tempdir().unwrap();
    write_tree(outside.path(), &[PARSER]);
    let (repo, _) = fake_repo(&[("README", "x")]);
    // A committed symlink pointing outside the checkout.
    std::os::unix::fs::symlink(outside.path(), repo.join("sub")).unwrap();
    let commit = add_commit(&repo, &[("other", "y")]);
    let g = format!(
        "[grammars.json]\nrepository = \"{}\"\nrev = \"{commit}\"\npath = \"sub\"\n",
        file_url(&repo)
    );
    let env = Env::new(&[
        ("extension.toml", &manifest(&g)),
        ("languages/json/config.toml", &lang_config("JSON", "json")),
        ("themes/x.json", THEME),
    ])
    .with_sdk();
    let out = env.build(None).0.unwrap();
    assert!(
        out.manifest.languages.is_empty(),
        "{:?}",
        out.manifest.languages
    );
    assert!(
        out.warnings.iter().any(|w| w.contains("outside")),
        "{:?}",
        out.warnings
    );
    assert_eq!(fake_clang_calls(&env.sdk()), 0);
}

#[test]
fn bad_snippets_counted_not_fatal() {
    let env = Env::new(&[
        ("extension.toml", &manifest("")),
        ("themes/x.json", THEME),
        (
            "snippets.json",
            r#"{ "ok": { "body": "a" }, "bad": { "body": "${1" } }"#,
        ),
    ]);
    let out = env.build(None).0.unwrap();
    assert!(out.staging.join("snippets.json").is_file());
    assert!(
        out.warnings.iter().any(|w| w.contains("1 invalid snippet")),
        "{:?}",
        out.warnings
    );
}

#[test]
fn icon_theme_and_referenced_icons_copied() {
    let icon_theme = r#"{
      "name": "I", "author": "t",
      "themes": [{
        "name": "I Dark", "appearance": "dark",
        "directory_icons": { "collapsed": "./icons/folder.svg", "expanded": "./icons/folder.svg" },
        "file_icons": { "rust": { "path": "./icons/rust.svg" }, "evil": { "path": "../../etc/passwd" } }
      }]
    }"#;
    let env = Env::new(&[
        ("extension.toml", &manifest("")),
        ("icon_themes/i.json", icon_theme),
        ("icons/folder.svg", "<svg/>"),
        ("icons/rust.svg", "<svg/>"),
        ("icons/unused.svg", "<svg/>"),
    ]);
    let out = env.build(None).0.unwrap();
    assert_eq!(
        out.manifest.icon_themes,
        [PathBuf::from("icon_themes/i.json")]
    );
    assert!(out.staging.join("icon_themes/i.json").is_file());
    assert!(out.staging.join("icons/folder.svg").is_file());
    assert!(out.staging.join("icons/rust.svg").is_file());
    assert!(!out.staging.join("icons/unused.svg").exists());
    assert!(
        out.warnings.iter().any(|w| w.contains("passwd")),
        "{:?}",
        out.warnings
    );
}

/// Builds tree-sitter-json 0.24.8 (from the cargo registry) with a real
/// wasi-sdk: `WASI_SDK_PATH=… cargo test -p lucerna-extension -- --ignored`.
#[test]
#[ignore]
fn real_clang_builds_json_grammar() {
    assert!(
        std::env::var_os("WASI_SDK_PATH").is_some(),
        "set WASI_SDK_PATH to a wasi-sdk 34 directory"
    );
    let home = std::env::var("CARGO_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap()).join(".cargo"));
    let crate_dir = std::fs::read_dir(home.join("registry/src"))
        .unwrap()
        .map(|e| e.unwrap().path().join("tree-sitter-json-0.24.8"))
        .find(|p| p.join("src/parser.c").is_file())
        .expect("tree-sitter-json-0.24.8 in the cargo registry");
    let mut files: Vec<(String, String)> = vec![(
        "src/parser.c".into(),
        std::fs::read_to_string(crate_dir.join("src/parser.c")).unwrap(),
    )];
    for e in std::fs::read_dir(crate_dir.join("src/tree_sitter")).unwrap() {
        let p = e.unwrap().path();
        files.push((
            format!(
                "src/tree_sitter/{}",
                p.file_name().unwrap().to_string_lossy()
            ),
            std::fs::read_to_string(&p).unwrap(),
        ));
    }
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let env = Env::new(&[
        ("extension.toml", &manifest(&grammar("json", &refs))),
        ("languages/json/config.toml", &lang_config("JSON", "json")),
    ]);
    let out = env.build(None).0.unwrap();
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let wasm = std::fs::read(out.staging.join("grammars/json.wasm")).unwrap();
    assert_eq!(&wasm[..4], b"\0asm");
    assert!(wasm.windows(16).any(|w| w == b"tree_sitter_json"));
    assert!(!env.root.join("toolchain").exists());
}
