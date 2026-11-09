use lucerna_extension::builder::{BuildError, Stage};
use lucerna_extension::index::{ExtensionIndex, Source, load_or_rebuild};
use lucerna_extension::lock::IdLock;
use lucerna_extension::process::CancelToken;
use lucerna_extension::store::{InstallProgress, Store, StoreError, StoreEvent};
use lucerna_extension::test_support::{
    FakeEntry, FakeValidator, add_commit, fake_registry, fake_repo, fake_wasi_sdk, file_url,
    registry_update,
};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const THEME: &str = r#"{
  "name": "X", "author": "t",
  "themes": [{ "name": "X Dark", "appearance": "dark", "style": {} }]
}"#;

fn wasm_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../lucerna-syntax/tests/fixtures/wasm/json.wasm")
}

fn manifest(id: &str, version: &str, extra: &str) -> String {
    format!(
        "id = \"{id}\"\nname = \"Ext {id}\"\ndescription = \"About {id}\"\nversion = \"{version}\"\nschema_version = 1\n{extra}"
    )
}

fn theme_files(id: &str, version: &str) -> Vec<(String, String)> {
    vec![
        ("extension.toml".into(), manifest(id, version, "")),
        ("themes/x.json".into(), THEME.into()),
    ]
}

fn as_refs(files: &[(String, String)]) -> Vec<(&str, &str)> {
    files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect()
}

struct Env {
    tmp: tempfile::TempDir,
    root: PathBuf,
    registry: PathBuf,
    store: Store,
}

impl Env {
    /// A store whose registry holds `entries`, already refreshed.
    fn new(entries: &[FakeEntry]) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("extensions");
        let registry = fake_registry(entries);
        let sdk = tmp.path().join("sdk");
        fake_wasi_sdk(&sdk, &wasm_fixture());
        let store = Store::open(root.clone(), Arc::new(FakeValidator))
            .allow_file_urls()
            .with_remote(file_url(&registry))
            .with_wasi_sdk(sdk);
        store.refresh_registry(&CancelToken::default()).unwrap();
        Self {
            tmp,
            root,
            registry,
            store,
        }
    }

    fn reopen(&self) -> Store {
        Store::open(self.root.clone(), Arc::new(FakeValidator))
            .allow_file_urls()
            .with_remote(file_url(&self.registry))
            .with_wasi_sdk(self.tmp.path().join("sdk"))
    }

    fn install(&self, id: &str) -> (Result<Vec<String>, StoreError>, Vec<InstallProgress>) {
        let seen = RefCell::new(Vec::new());
        let r = self
            .store
            .install(id, &|p| seen.borrow_mut().push(p), &CancelToken::default());
        (r, seen.into_inner())
    }

    fn installed(&self, id: &str) -> PathBuf {
        self.root.join("installed").join(id)
    }

    fn entries(&self, dir: &str) -> Vec<String> {
        std::fs::read_dir(self.root.join(dir))
            .map(|rd| {
                rd.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn theme_ext(id: &str) -> (PathBuf, FakeEntry) {
    let (repo, _) = fake_repo(&as_refs(&theme_files(id, "0.1.0")));
    let entry = FakeEntry::new(id, "0.1.0", &repo);
    (repo, entry)
}

fn changed(rx: &crossbeam_channel::Receiver<StoreEvent>) -> Option<Arc<ExtensionIndex>> {
    rx.try_iter().find_map(|e| match e {
        StoreEvent::Changed(i) => Some(i),
        _ => None,
    })
}

#[test]
fn install_theme_extension_end_to_end() {
    let (repo, entry) = theme_ext("x");
    let commit = add_commit(&repo, &[("README.md", "hi")]);
    let env = Env::new(&[entry]);
    let rx = env.store.subscribe();
    let (r, seen) = env.install("x");
    assert!(r.unwrap().is_empty());
    assert!(env.installed("x").join("themes/x.json").is_file());
    let ext = &env.store.index().extensions["x"];
    assert_eq!(ext.themes.len(), 1);
    assert_eq!(ext.themes[0].name, "X Dark");
    assert_eq!(ext.themes[0].appearance, "dark");
    assert!(ext.broken.is_none());
    assert_eq!(ext.info.source, Source::Registry);
    let info: serde_json::Value =
        serde_json::from_slice(&std::fs::read(env.installed("x").join("lucerna.json")).unwrap())
            .unwrap();
    assert_eq!(info["commit"], commit.as_str());
    assert_eq!(info["version"], "0.1.0");
    assert_eq!(info["source"], "registry");
    let idx = changed(&rx).expect("Changed event");
    assert!(idx.extensions.contains_key("x"));
    let stages: Vec<_> = seen.iter().map(|p| p.stage.clone()).collect();
    assert_eq!(stages[0], Stage::Fetching);
    assert!(stages.contains(&Stage::Installing));
    assert!(seen.iter().all(|p| p.id == "x"));
    assert!(env.entries("staging").is_empty());
    assert!(env.root.join("index.json").is_file());
    assert!(env.root.join("logs/x.log").is_file());
}

#[test]
fn install_applies_registry_path() {
    let (repo, _) = fake_repo(&[
        ("zed/extension.toml", &manifest("sub", "0.2.0", "")),
        ("zed/themes/x.json", THEME),
    ]);
    let env = Env::new(&[FakeEntry::new("sub", "0.2.0", &repo).path("zed")]);
    env.install("sub").0.unwrap();
    assert!(env.installed("sub").join("themes/x.json").is_file());
}

#[test]
fn install_language_extension_indexes_language() {
    let (grammar_repo, rev) = fake_repo(&[("src/parser.c", "/* parser */")]);
    let extra = format!(
        "[grammars.json]\nrepository = \"{}\"\nrev = \"{rev}\"\n",
        file_url(&grammar_repo)
    );
    let (repo, _) = fake_repo(&[
        ("extension.toml", &manifest("lang", "0.1.0", &extra)),
        (
            "languages/json/config.toml",
            "name = \"JSON\"\ngrammar = \"json\"\npath_suffixes = [\"json\"]\n",
        ),
        ("languages/json/highlights.scm", "(string) @string"),
    ]);
    let env = Env::new(&[FakeEntry::new("lang", "0.1.0", &repo)]);
    let (r, seen) = env.install("lang");
    r.unwrap();
    let ext = &env.store.index().extensions["lang"];
    assert_eq!(ext.languages.len(), 1);
    let l = &ext.languages[0];
    assert_eq!(l.name, "JSON");
    assert_eq!(l.path_suffixes, ["json"]);
    assert_eq!(l.queries, ["highlights"]);
    let g = l.grammar.as_ref().expect("grammar");
    assert_eq!(g.name, "json");
    assert!(g.wasm_path.is_file());
    let installed = env.root.join("installed").canonicalize().unwrap();
    assert!(
        g.wasm_path.canonicalize().unwrap().starts_with(&installed),
        "{}",
        g.wasm_path.display()
    );
    assert!(l.dir.join("highlights.scm").is_file());
    assert!(
        seen.iter()
            .any(|p| matches!(p.stage, Stage::BuildingGrammar { .. }))
    );
}

#[test]
fn failed_install_keeps_previous_version() {
    let (repo, entry) = theme_ext("x");
    let env = Env::new(&[entry]);
    env.install("x").0.unwrap();
    // v2 has a theme file that is not a valid theme: nothing usable.
    add_commit(
        &repo,
        &[
            ("extension.toml", &manifest("x", "0.2.0", "")),
            ("themes/x.json", "{ \"themes\": 3 }"),
        ],
    );
    registry_update(&env.registry, "x", "0.2.0");
    env.store.refresh_registry(&CancelToken::default()).unwrap();
    let (r, _) = env.install("x");
    assert!(
        matches!(r, Err(StoreError::Build(BuildError::NothingUsable(_)))),
        "{r:?}"
    );
    assert_eq!(env.store.index().extensions["x"].info.version, "0.1.0");
    assert_eq!(
        std::fs::read_to_string(env.installed("x").join("themes/x.json")).unwrap(),
        THEME
    );
    assert!(env.entries("staging").is_empty());
    assert_eq!(env.entries("installed"), ["x"]);
}

#[test]
fn update_detected_and_applied() {
    let (repo, entry) = theme_ext("x");
    let env = Env::new(&[entry]);
    env.install("x").0.unwrap();
    assert!(env.store.updates().is_empty());
    let commit = add_commit(&repo, &as_refs(&theme_files("x", "0.2.0")));
    registry_update(&env.registry, "x", "0.2.0");
    env.store.refresh_registry(&CancelToken::default()).unwrap();
    assert_eq!(env.store.updates(), ["x"]);
    env.install("x").0.unwrap();
    let info = &env.store.index().extensions["x"].info;
    assert_eq!(info.version, "0.2.0");
    assert_eq!(info.commit, commit);
    assert!(env.store.updates().is_empty());
    // The old copy is held in trash/, never in installed/.
    assert_eq!(env.entries("installed"), ["x"]);
}

#[test]
fn uninstall_removes_and_emits() {
    let (_repo, entry) = theme_ext("x");
    let env = Env::new(&[entry]);
    env.install("x").0.unwrap();
    let rx = env.store.subscribe();
    env.store.uninstall("x").unwrap();
    assert!(!env.installed("x").exists());
    let idx = changed(&rx).expect("Changed event");
    assert!(!idx.extensions.contains_key("x"));
    assert!(!env.store.index().extensions.contains_key("x"));
    // work/<id> stays as a cache.
    assert!(env.root.join("work/x/src").is_dir());
    assert!(matches!(
        env.store.uninstall("x"),
        Err(StoreError::UnknownId(_))
    ));
    assert_eq!(env.entries("installed"), Vec::<String>::new());
    // A reopen (even before the background removal ran) never restores an
    // uninstalled extension.
    std::fs::create_dir_all(env.root.join("trash/removed-x-1-2-3/themes")).unwrap();
    let store = env.reopen();
    assert!(!env.installed("x").exists());
    assert!(!store.index().extensions.contains_key("x"));
    assert!(env.entries("trash").is_empty());
}

#[test]
fn second_install_same_id_is_busy() {
    let (_repo, entry) = theme_ext("x");
    let env = Env::new(&[entry]);
    let lock = IdLock::try_acquire(&env.root, "x").unwrap();
    // A second lock on the same id fails too, from this thread.
    assert!(matches!(
        IdLock::try_acquire(&env.root, "x"),
        Err(StoreError::Busy(_))
    ));
    let (r, _) = env.install("x");
    assert!(
        matches!(r, Err(StoreError::Busy(ref id)) if id == "x"),
        "{r:?}"
    );
    assert!(matches!(env.store.uninstall("x"), Err(StoreError::Busy(_))));
    drop(lock);
    env.install("x").0.unwrap();
}

#[test]
fn startup_cleans_leftovers() {
    let (_repo, entry) = theme_ext("x");
    let env = Env::new(&[entry]);
    env.install("x").0.unwrap();
    for d in [
        "trash/old-x-1-2-3",
        "trash/removed-x-1-2-3",
        "trash/junk",
        "staging/x-1-2-3/themes",
    ] {
        std::fs::create_dir_all(env.root.join(d)).unwrap();
    }
    std::fs::write(env.root.join("index.json.tmp-1-2-3"), "{").unwrap();
    let store = env.reopen();
    assert_eq!(env.entries("installed"), ["x"]);
    assert!(env.entries("staging").is_empty());
    assert!(env.entries("trash").is_empty());
    assert!(!env.root.join("index.json.tmp-1-2-3").exists());
    assert!(store.index().extensions.contains_key("x"));
}

#[test]
fn startup_restores_old_copy_after_crash_mid_swap() {
    let (_repo, entry) = theme_ext("x");
    let env = Env::new(&[entry]);
    env.install("x").0.unwrap();
    // Crash between "installed/x -> trash/old-x-..." and "staging -> installed/x".
    let trash = env.root.join("trash");
    std::fs::create_dir_all(&trash).unwrap();
    std::fs::rename(env.installed("x"), trash.join("old-x-ff-1-2")).unwrap();
    std::fs::create_dir_all(trash.join("old-x-a-1-2/themes")).unwrap();
    std::fs::create_dir_all(env.root.join("staging/x-ff-1-3")).unwrap();
    let store = env.reopen();
    assert!(env.installed("x").join("themes/x.json").is_file());
    assert!(env.entries("trash").is_empty());
    assert!(env.entries("staging").is_empty());
    let ext = &store.index().extensions["x"];
    assert!(ext.broken.is_none(), "{:?}", ext.broken);
    assert_eq!(ext.themes.len(), 1);
}

#[test]
fn startup_keeps_staging_of_a_running_install() {
    let (_repo, entry) = theme_ext("x");
    let env = Env::new(&[entry]);
    std::fs::create_dir_all(env.root.join("staging/x-1-2-3")).unwrap();
    let _lock = IdLock::try_acquire(&env.root, "x").unwrap();
    let _store = env.reopen();
    assert_eq!(env.entries("staging"), ["x-1-2-3"]);
}

#[test]
fn broken_installed_dir_marked() {
    let (_repo, entry) = theme_ext("x");
    let env = Env::new(&[entry]);
    env.install("x").0.unwrap();
    std::fs::create_dir_all(env.root.join("installed/junk")).unwrap();
    std::fs::create_dir_all(env.root.join("installed/.hidden")).unwrap();
    std::fs::write(env.installed("x").join("extension.toml"), "not = [toml").unwrap();
    let store = env.reopen();
    let idx = store.index();
    assert!(idx.extensions["junk"].broken.is_some());
    assert!(
        idx.extensions["x"]
            .broken
            .as_deref()
            .is_some_and(|b| b.contains("extension.toml")),
        "{:?}",
        idx.extensions["x"].broken
    );
    assert!(!idx.extensions.contains_key(".hidden"));
}

#[test]
fn index_rebuilt_when_corrupt() {
    let (_repo, entry) = theme_ext("x");
    let env = Env::new(&[entry]);
    env.install("x").0.unwrap();
    let path = env.root.join("index.json");
    std::fs::write(&path, "{ corrupt").unwrap();
    let idx = load_or_rebuild(&env.root);
    assert!(idx.extensions.contains_key("x"));
    // The rebuilt index was written back.
    let back: ExtensionIndex = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(back.extensions.contains_key("x"));
    // A missing file is rebuilt too.
    std::fs::remove_file(&path).unwrap();
    assert!(env.reopen().index().extensions.contains_key("x"));
}

#[test]
fn index_rebuilt_when_installed_is_newer() {
    let (_repo, entry) = theme_ext("x");
    let env = Env::new(&[entry]);
    env.install("x").0.unwrap();
    std::thread::sleep(Duration::from_millis(20));
    // Another process removed it without updating index.json.
    std::fs::rename(env.installed("x"), env.tmp.path().join("gone")).unwrap();
    assert!(!load_or_rebuild(&env.root).extensions.contains_key("x"));
    // An index is not trusted once installed/ itself is gone.
    env.install("x").0.unwrap();
    std::fs::rename(env.root.join("installed"), env.tmp.path().join("gone2")).unwrap();
    assert!(load_or_rebuild(&env.root).extensions.is_empty());
}

#[test]
fn concurrent_installs_of_different_ids_all_indexed() {
    let ids = ["a", "b", "c", "d"];
    let entries: Vec<FakeEntry> = ids.iter().map(|id| theme_ext(id).1).collect();
    let env = Env::new(&entries);
    // Two clones share one store; a second open store stands in for
    // another process.
    let other = env.reopen();
    std::thread::scope(|s| {
        for (i, id) in ids.iter().enumerate() {
            let store = if i % 2 == 0 {
                env.store.clone()
            } else {
                other.clone()
            };
            s.spawn(move || {
                store.install(id, &|_| {}, &CancelToken::default()).unwrap();
            });
        }
    });
    let on_disk: ExtensionIndex =
        serde_json::from_slice(&std::fs::read(env.root.join("index.json")).unwrap()).unwrap();
    for id in ids {
        assert!(on_disk.extensions.contains_key(id), "{id} missing on disk");
    }
    let mem = env.store.index();
    assert!(mem.extensions.contains_key("a") && mem.extensions.contains_key("c"));
    let mem = other.index();
    assert!(mem.extensions.contains_key("b") && mem.extensions.contains_key("d"));
}

#[test]
fn concurrent_describes_of_different_ids_all_persist() {
    let ids = ["a", "b", "c", "d", "e", "f", "g", "h"];
    let entries: Vec<FakeEntry> = ids.iter().map(|id| theme_ext(id).1).collect();
    let env = Env::new(&entries);
    // A second open store stands in for another process.
    let other = env.reopen();
    std::thread::scope(|s| {
        for (i, id) in ids.iter().enumerate() {
            let store = if i % 2 == 0 {
                env.store.clone()
            } else {
                other.clone()
            };
            s.spawn(move || {
                store.describe(id, &CancelToken::default()).unwrap();
            });
        }
    });
    let reg = env.store.registry().unwrap();
    for id in ids {
        let e = reg.entries.iter().find(|e| e.id == id).unwrap();
        assert_eq!(
            e.description.as_deref(),
            Some(format!("About {id}").as_str()),
            "{id} lost its description"
        );
    }
    let leftovers: Vec<String> = std::fs::read_dir(&env.root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("registry.json.tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn describe_persists_name_and_description() {
    let (_repo, entry) = theme_ext("x");
    let env = Env::new(&[entry]);
    let e = env.store.describe("x", &CancelToken::default()).unwrap();
    assert_eq!(e.name.as_deref(), Some("Ext x"));
    assert_eq!(e.description.as_deref(), Some("About x"));
    let reg = env.store.registry().unwrap();
    let cached = reg.entries.iter().find(|e| e.id == "x").unwrap();
    assert_eq!(cached.name.as_deref(), Some("Ext x"));
    assert_eq!(cached.description.as_deref(), Some("About x"));
    assert!(env.root.join("work/x/src/extension.toml").is_file());
    assert!(matches!(
        env.store.describe("nope", &CancelToken::default()),
        Err(StoreError::UnknownId(_))
    ));
}

#[test]
fn install_dev_copies_not_symlinks() {
    let (_repo, entry) = theme_ext("x");
    let env = Env::new(&[entry]);
    let dev = env.tmp.path().join("dev-ext");
    for (rel, content) in theme_files("devx", "0.0.1") {
        let p = dev.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }
    let rx = env.store.subscribe();
    env.store
        .install_dev(&dev, &|_| {}, &CancelToken::default())
        .unwrap();
    let inst = env.installed("devx");
    assert!(!std::fs::symlink_metadata(&inst).unwrap().is_symlink());
    let theme = inst.join("themes/x.json");
    assert!(!std::fs::symlink_metadata(&theme).unwrap().is_symlink());
    std::fs::write(dev.join("themes/x.json"), "changed").unwrap();
    assert_eq!(std::fs::read_to_string(&theme).unwrap(), THEME);
    let ext = &env.store.index().extensions["devx"];
    assert_eq!(ext.info.source, Source::Dev);
    assert!(changed(&rx).is_some());
    // Dev installs are never offered as updates.
    assert!(env.store.updates().is_empty());
}

#[test]
fn unknown_id_error() {
    let (_repo, entry) = theme_ext("x");
    let env = Env::new(&[entry]);
    let (r, _) = env.install("nope");
    assert!(
        matches!(r, Err(StoreError::UnknownId(ref id)) if id == "nope"),
        "{r:?}"
    );
    let (r, _) = env.install("../x");
    assert!(matches!(r, Err(StoreError::UnknownId(_))), "{r:?}");
}

#[test]
fn git_missing_reported() {
    let (_repo, entry) = theme_ext("x");
    let env = Env::new(&[entry]);
    let store = env.reopen().with_git_program("/nonexistent/git");
    let r = store.install("x", &|_| {}, &CancelToken::default());
    assert!(matches!(r, Err(StoreError::GitMissing)), "{r:?}");
    assert!(matches!(
        store.refresh_registry(&CancelToken::default()),
        Err(StoreError::GitMissing)
    ));
    assert!(!env.installed("x").exists());
}

#[test]
fn clear_build_cache_removes_work() {
    let (_repo, entry) = theme_ext("x");
    let env = Env::new(&[entry]);
    env.install("x").0.unwrap();
    assert!(env.root.join("work").is_dir());
    env.store.clear_build_cache().unwrap();
    assert!(!env.root.join("work").exists());
    assert!(env.installed("x").is_dir());
}

#[test]
fn panic_in_progress_becomes_internal_error() {
    let (_repo, entry) = theme_ext("x");
    let env = Env::new(&[entry]);
    let r = env
        .store
        .install("x", &|_| panic!("boom"), &CancelToken::default());
    assert!(
        matches!(r, Err(StoreError::Io(ref m)) if m == "internal error"),
        "{r:?}"
    );
    // The lock was released by unwinding.
    env.install("x").0.unwrap();
}

#[test]
fn watcher_sees_external_uninstall() {
    let (_repo, entry) = theme_ext("x");
    let env = Env::new(&[entry]);
    env.install("x").0.unwrap();
    env.store.watch().unwrap();
    let rx = env.store.subscribe();
    std::thread::sleep(Duration::from_millis(300));
    while rx.try_recv().is_ok() {}
    std::fs::rename(env.installed("x"), env.tmp.path().join("moved")).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match rx.recv_timeout(left) {
            Ok(StoreEvent::Changed(idx)) if !idx.extensions.contains_key("x") => break,
            Ok(_) => {}
            Err(e) => panic!("no Changed event within 2 s: {e}"),
        }
    }
    assert!(!env.store.index().extensions.contains_key("x"));
}

#[test]
fn install_refuses_option_like_grammar_rev_and_never_runs_it() {
    let marker = tempfile::tempdir().unwrap().keep().join("PWNED");
    let (grammar_repo, _) = fake_repo(&[("src/parser.c", "/* parser */")]);
    let extra = format!(
        "[grammars.json]\nrepository = \"{}\"\nrev = \"--upload-pack=touch {}; git-upload-pack\"\n",
        file_url(&grammar_repo),
        marker.display()
    );
    let (repo, _) = fake_repo(&[
        ("extension.toml", &manifest("evil", "0.1.0", &extra)),
        (
            "languages/json/config.toml",
            "name = \"JSON\"\ngrammar = \"json\"\npath_suffixes = [\"json\"]\n",
        ),
    ]);
    let env = Env::new(&[FakeEntry::new("evil", "0.1.0", &repo)]);
    let err = env.install("evil").0.unwrap_err();
    assert!(err.to_string().contains("rev"), "{err}");
    assert!(!marker.exists(), "the injected command ran");
    assert!(!env.installed("evil").exists());
}

#[test]
fn file_urls_refused_without_the_opt_in() {
    let (_, entry) = theme_ext("x");
    let registry = fake_registry(&[entry]);
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(tmp.path().join("extensions"), Arc::new(FakeValidator))
        .with_remote(file_url(&registry));
    let err = store.refresh_registry(&CancelToken::default()).unwrap_err();
    assert!(err.to_string().contains("https://"), "{err}");
    assert!(!tmp.path().join("extensions/registry").exists());

    // A registry entry pointing at a local repository is refused as well.
    let env = Env::new(&[theme_ext("y").1]);
    let plain = Store::open(env.root.clone(), Arc::new(FakeValidator));
    let err = plain
        .install("y", &|_| {}, &CancelToken::default())
        .unwrap_err();
    assert!(err.to_string().contains("https://"), "{err}");
    assert!(!env.installed("y").exists());
}

#[test]
fn startup_skips_busy_ids_without_waiting() {
    let (_repo, entry) = theme_ext("x");
    let env = Env::new(&[entry]);
    env.install("x").0.unwrap();
    std::fs::create_dir_all(env.root.join("staging/busy-1-2-3")).unwrap();
    let _lock = IdLock::try_acquire(&env.root, "busy").unwrap();
    let started = std::time::Instant::now();
    let _store = env.reopen();
    let took = started.elapsed();
    assert!(took < Duration::from_millis(150), "open took {took:?}");
    assert_eq!(env.entries("staging"), ["busy-1-2-3"]);
}

#[test]
fn deferred_open_reads_only_the_index_and_sweeps_in_background() {
    let (_repo, entry) = theme_ext("x");
    let env = Env::new(&[entry]);
    env.install("x").0.unwrap();
    for d in [
        "trash/removed-x-1-2-3/themes",
        "staging/x-1-2-3/themes",
        "staging/busy-1-2-3",
    ] {
        std::fs::create_dir_all(env.root.join(d)).unwrap();
    }
    std::fs::write(env.root.join("index.json.tmp-1-2-3"), "{").unwrap();
    let busy = IdLock::try_acquire(&env.root, "busy").unwrap();
    // A missing index is rebuilt in the background, not on open.
    std::fs::remove_file(env.root.join("index.json")).unwrap();

    let started = std::time::Instant::now();
    let store = Store::open_deferred(env.root.clone(), Arc::new(FakeValidator));
    let took = started.elapsed();
    assert!(took < Duration::from_millis(50), "open took {took:?}");
    assert!(store.index().extensions.is_empty());
    // Nothing was swept yet.
    assert_eq!(env.entries("trash"), ["removed-x-1-2-3"]);
    assert!(env.root.join("index.json.tmp-1-2-3").exists());

    let rx = store.subscribe();
    store.start_maintenance();
    let idx = rx
        .recv_timeout(Duration::from_secs(10))
        .ok()
        .and_then(|e| match e {
            StoreEvent::Changed(i) => Some(i),
            _ => None,
        })
        .expect("Changed after the background rebuild");
    assert!(idx.extensions.contains_key("x"));
    assert!(store.index().extensions.contains_key("x"));
    assert!(env.entries("trash").is_empty());
    assert_eq!(env.entries("staging"), ["busy-1-2-3"]);
    assert!(!env.root.join("index.json.tmp-1-2-3").exists());
    assert!(env.root.join("index.json").is_file());
    drop(busy);
}

#[test]
fn uninstall_after_update_is_never_undone() {
    let (repo, entry) = theme_ext("x");
    let env = Env::new(&[entry]);
    env.install("x").0.unwrap();
    add_commit(&repo, &as_refs(&theme_files("x", "0.2.0")));
    registry_update(&env.registry, "x", "0.2.0");
    env.store.refresh_registry(&CancelToken::default()).unwrap();
    env.install("x").0.unwrap();
    // The update's background removal of the old copy has not run yet.
    let old = env.root.join("trash/old-x-1-2-3");
    std::fs::create_dir_all(old.join("themes")).unwrap();
    for f in ["extension.toml", "themes/x.json", "lucerna.json"] {
        std::fs::copy(env.installed("x").join(f), old.join(f)).unwrap();
    }
    env.store.uninstall("x").unwrap();
    let store = env.reopen();
    assert!(!env.installed("x").exists());
    assert!(!store.index().extensions.contains_key("x"));
    assert!(env.entries("trash").is_empty());
}

#[test]
fn clear_build_cache_skips_busy_ids() {
    let (_r1, e1) = theme_ext("x");
    let (_r2, e2) = theme_ext("y");
    let env = Env::new(&[e1, e2]);
    env.install("x").0.unwrap();
    env.install("y").0.unwrap();
    let lock = IdLock::try_acquire(&env.root, "x").unwrap();
    let busy = env.store.clear_build_cache().unwrap();
    assert_eq!(busy, ["x"]);
    assert!(env.root.join("work/x/src").is_dir());
    assert!(!env.root.join("work/y").exists());
    drop(lock);
    assert!(env.store.clear_build_cache().unwrap().is_empty());
    assert!(!env.root.join("work").exists());
}
