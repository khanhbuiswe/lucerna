//! The extension service: registries built from the store's index at
//! startup, swapped at runtime when the index changes, and open documents
//! following the swap (Review Focus 3 and 4).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use gpui::{Entity, TestAppContext, VisualTestContext};
use lucerna_extension::index::reindex;
use lucerna_extension::store::StoreEvent;
use lucerna_settings::parse_when;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::extensions::ExtensionService;
use lucerna_workbench::{
    AppState, ConfigPaths, DocState, Documents, OpenMode, Workbench, reload_config, watch_config,
};
use tempfile::TempDir;

const JSON_WASM: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../lucerna-syntax/tests/fixtures/wasm/json.wasm"
);

/// The WASM grammar cache is process-wide; tests that count or fill it
/// run one at a time.
fn serial() -> MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

/// A project, a user config directory and an extension store root.
struct Dirs {
    project: TempDir,
    user: TempDir,
    store: TempDir,
}

impl Dirs {
    fn new() -> Dirs {
        Dirs {
            project: tempfile::tempdir().unwrap(),
            user: tempfile::tempdir().unwrap(),
            store: tempfile::tempdir().unwrap(),
        }
    }

    fn installed(&self, id: &str) -> PathBuf {
        self.store.path().join("installed").join(id)
    }

    fn settings(&self) -> PathBuf {
        self.user.path().join("settings.json")
    }
}

fn write(path: &Path, text: impl AsRef<[u8]>) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Writes `installed/<id>` by hand: a manifest, install info and, per
/// `(name, suffix, capture)`, a language using the `json.wasm` fixture
/// whose highlights capture strings as `capture`.
fn install_language_ext(dirs: &Dirs, id: &str, languages: &[(&str, &str, &str)]) {
    let dir = dirs.installed(id);
    write(
        &dir.join("extension.toml"),
        format!("id = \"{id}\"\nname = \"{id}\"\nversion = \"0.1.0\"\nschema_version = 1\n"),
    );
    write(
        &dir.join("lucerna.json"),
        r#"{"commit":"","version":"0.1.0","installed_at":0,"source":"dev"}"#,
    );
    write(
        &dir.join("grammars/json.wasm"),
        std::fs::read(JSON_WASM).unwrap(),
    );
    for (name, suffix, capture) in languages {
        let ldir = dir.join("languages").join(name.to_lowercase());
        write(
            &ldir.join("config.toml"),
            format!("name = \"{name}\"\ngrammar = \"json\"\npath_suffixes = [\"{suffix}\"]\n"),
        );
        write(
            &ldir.join("highlights.scm"),
            format!("(string) @{capture}\n"),
        );
    }
}

fn install_theme_ext(dirs: &Dirs, id: &str, theme: &str) {
    let dir = dirs.installed(id);
    write(
        &dir.join("extension.toml"),
        format!("id = \"{id}\"\nname = \"{id}\"\nversion = \"0.1.0\"\nschema_version = 1\n"),
    );
    write(
        &dir.join("lucerna.json"),
        r#"{"commit":"","version":"0.1.0","installed_at":0,"source":"dev"}"#,
    );
    write(
        &dir.join("themes/ext.json"),
        format!(
            r##"{{"name":"Ext","author":"t","themes":[{{"name":"{theme}","appearance":"dark","style":{{"background":"#102030"}}}}]}}"##
        ),
    );
}

/// What the store's watcher would deliver after `installed/` changed.
fn index_changed(dirs: &Dirs, cx: &mut VisualTestContext) {
    let index = Arc::new(reindex(dirs.store.path()));
    cx.update(|_, cx| ExtensionService::handle_event(StoreEvent::Changed(index), cx));
    settle(cx);
}

fn init(dirs: &Dirs, cx: &mut TestAppContext) {
    let root = dirs.store.path().to_path_buf();
    cx.update(|cx| lucerna_workbench::app::set_app_state(root, cx));
}

fn init_with_config(dirs: &Dirs, cx: &mut TestAppContext) {
    init(dirs, cx);
    let paths = ConfigPaths::user_only_with_user_dir(dirs.user.path());
    cx.update(|cx| watch_config(paths, cx));
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn settle(cx: &mut VisualTestContext) {
    draw(cx);
    cx.run_until_parked();
    draw(cx);
    cx.run_until_parked();
    draw(cx);
}

fn workbench<'a>(
    dirs: &Dirs,
    cx: &'a mut TestAppContext,
) -> (Entity<Workbench>, &'a mut VisualTestContext) {
    let root = dirs.project.path().to_path_buf();
    cx.add_window_view(|window, cx| Workbench::new(root, window, cx))
}

fn documents(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<Documents> {
    cx.update(|_, cx| wb.read(cx).documents().clone())
}

fn open(wb: &Entity<Workbench>, path: &str, cx: &mut VisualTestContext) -> Entity<CodeView> {
    let path: Arc<Path> = Arc::from(Path::new(path));
    wb.update(cx, |wb, cx| wb.open_path(path, OpenMode::Pinned, cx));
    settle(cx);
    cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .active_body()
            .expect("the active tab has a body")
            .downcast::<CodeView>()
            .expect("tab bodies are code views")
    })
}

fn language(wb: &Entity<Workbench>, view: &Entity<CodeView>, cx: &mut VisualTestContext) -> String {
    let docs = documents(wb, cx);
    cx.update(|_, cx| {
        docs.read(cx)
            .status(view.read(cx))
            .expect("the document has text")
            .language
            .to_string()
    })
}

/// Capture names highlighted on the first line.
fn captures(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |v, _| {
        v.syntax().map_or_else(Vec::new, |syntax| {
            syntax
                .highlights(v.snapshot(), 0..1, &|_| true)
                .into_iter()
                .map(|s| s.capture.to_string())
                .collect()
        })
    })
}

fn languages_generation(cx: &mut VisualTestContext) -> u64 {
    cx.update(|_, cx| cx.global::<AppState>().languages_generation)
}

fn editor_lang_id_is(wb: &Entity<Workbench>, id: &str, cx: &mut VisualTestContext) -> bool {
    let ctx = cx.update(|window, cx| wb.read(cx).context_keys(window, cx));
    parse_when(&format!("editorLangId == '{id}'"))
        .unwrap()
        .eval(&ctx)
}

fn theme_name(cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| cx.global::<AppState>().theme.name.clone())
}

#[gpui::test]
fn startup_reads_index_without_loading_grammar(cx: &mut TestAppContext) {
    let _serial = serial();
    let dirs = Dirs::new();
    install_language_ext(&dirs, "zz", &[("ZZ", "zz", "string")]);
    reindex(dirs.store.path());
    let before = lucerna_syntax::wasm_cache_len();

    init(&dirs, cx);

    cx.update(|cx| {
        let languages = &cx.global::<AppState>().languages;
        let zz = languages
            .language_for_path(Path::new("a.zz"))
            .expect("the extension language is registered");
        assert_eq!(&*zz.name, "ZZ");
        assert!(!zz.grammar_ready(), "its grammar is not loaded");
    });
    assert_eq!(
        lucerna_syntax::wasm_cache_len(),
        before,
        "startup loads no WASM grammar"
    );
}

#[gpui::test]
fn install_event_recolors_open_tab(cx: &mut TestAppContext) {
    let _serial = serial();
    let dirs = Dirs::new();
    write(&dirs.project.path().join("a.zz"), "\"hello\"\n");
    init(&dirs, cx);
    let (wb, cx) = workbench(&dirs, cx);
    let view = open(&wb, "a.zz", cx);
    assert_eq!(language(&wb, &view, cx), "Plain Text");
    assert!(captures(&view, cx).is_empty());

    let generation = languages_generation(cx);
    install_language_ext(&dirs, "zz", &[("ZZ", "zz", "string")]);
    index_changed(&dirs, cx);

    assert_eq!(languages_generation(cx), generation + 1);
    assert_eq!(language(&wb, &view, cx), "ZZ");
    assert!(editor_lang_id_is(&wb, "zz", cx));
    assert!(
        captures(&view, cx).iter().any(|c| c == "string"),
        "the string is highlighted: {:?}",
        captures(&view, cx)
    );
}

#[gpui::test]
fn uninstall_falls_back_without_crash(cx: &mut TestAppContext) {
    let _serial = serial();
    let dirs = Dirs::new();
    write(&dirs.project.path().join("a.json"), "\"hello\"\n");
    install_language_ext(&dirs, "extjson", &[("JSON", "json", "ext.string")]);
    reindex(dirs.store.path());
    init(&dirs, cx);
    let (wb, cx) = workbench(&dirs, cx);
    let view = open(&wb, "a.json", cx);
    assert!(
        captures(&view, cx).iter().any(|c| c == "ext.string"),
        "the extension grammar colours the file: {:?}",
        captures(&view, cx)
    );

    let cached = lucerna_syntax::wasm_cache_len();
    assert!(cached > 0);

    cx.update(|_, cx| ExtensionService::uninstall("extjson".into(), cx));
    settle(cx);

    assert!(
        lucerna_syntax::wasm_cache_len() < cached,
        "the uninstalled grammar is dropped from the cache"
    );

    assert!(!dirs.installed("extjson").exists());
    let docs = documents(&wb, cx);
    let state = cx.update(|_, cx| docs.read(cx).state(Path::new("a.json")).cloned());
    assert_eq!(state, Some(DocState::Ready));
    assert_eq!(language(&wb, &view, cx), "JSON");
    let spans = captures(&view, cx);
    assert!(
        spans.iter().any(|c| c.starts_with("string")) && !spans.iter().any(|c| c == "ext.string"),
        "bundled JSON colours the file now: {spans:?}"
    );
    cx.update(|_, cx| {
        assert_eq!(ExtensionService::global(cx).last_error("extjson"), None);
    });
}

#[gpui::test]
fn failed_grammar_tab_shows_text(cx: &mut TestAppContext) {
    let _serial = serial();
    let dirs = Dirs::new();
    write(&dirs.project.path().join("a.yy"), "\"hello\"\n");
    install_language_ext(&dirs, "yy", &[("YY", "yy", "string")]);
    write(
        &dirs.installed("yy").join("grammars/json.wasm"),
        b"garbage, not wasm",
    );
    reindex(dirs.store.path());
    init(&dirs, cx);
    let (wb, cx) = workbench(&dirs, cx);
    let view = open(&wb, "a.yy", cx);

    let docs = documents(&wb, cx);
    let state = cx.update(|_, cx| docs.read(cx).state(Path::new("a.yy")).cloned());
    assert_eq!(state, Some(DocState::Ready), "it does not stay Loading");
    view.read_with(cx, |v, _| {
        assert!(v.syntax().is_none(), "no syntax from a broken grammar");
        let s = v.snapshot();
        assert_eq!(s.text_for_range(0..s.len_bytes()), "\"hello\"\n");
    });
}

#[gpui::test]
fn color_theme_applied_when_extension_appears(cx: &mut TestAppContext) {
    let _serial = serial();
    let dirs = Dirs::new();
    write(
        &dirs.settings(),
        r#"{ "window.autoDetectColorScheme": false, "workbench.colorTheme": "Ext Dark" }"#,
    );
    init_with_config(&dirs, cx);
    let (_wb, cx) = workbench(&dirs, cx);
    settle(cx);
    assert_eq!(theme_name(cx), "Lucerna Dark");
    let setting: Option<String> = cx.update(|_, cx| {
        cx.global::<AppState>()
            .settings
            .get("workbench.colorTheme", None)
    });
    assert_eq!(setting.as_deref(), Some("Ext Dark"), "the setting is kept");

    let generation = languages_generation(cx);
    install_theme_ext(&dirs, "exttheme", "Ext Dark");
    index_changed(&dirs, cx);

    assert_eq!(theme_name(cx), "Ext Dark");
    assert_eq!(
        languages_generation(cx),
        generation,
        "a theme-only change keeps the language registry"
    );
}

#[gpui::test]
fn configured_extension_theme_applies_at_startup(cx: &mut TestAppContext) {
    let _serial = serial();
    let dirs = Dirs::new();
    install_theme_ext(&dirs, "exttheme", "Ext Dark");
    reindex(dirs.store.path());
    write(
        &dirs.settings(),
        r#"{ "window.autoDetectColorScheme": false, "workbench.colorTheme": "Ext Dark" }"#,
    );
    init_with_config(&dirs, cx);
    let (_wb, cx) = workbench(&dirs, cx);
    settle(cx);
    assert_eq!(theme_name(cx), "Ext Dark");
}

#[gpui::test]
fn files_associations_change_rebuilds(cx: &mut TestAppContext) {
    let _serial = serial();
    let dirs = Dirs::new();
    write(&dirs.project.path().join("a.qq"), "\"hello\"\n");
    init_with_config(&dirs, cx);
    let (wb, cx) = workbench(&dirs, cx);
    let view = open(&wb, "a.qq", cx);
    assert_eq!(language(&wb, &view, cx), "Plain Text");
    assert!(editor_lang_id_is(&wb, "", cx));

    write(
        &dirs.settings(),
        r#"{ "files.associations": { "*.qq": "json" } }"#,
    );
    let changed = dirs.settings();
    cx.update(|_, cx| reload_config(&[changed], cx));
    settle(cx);

    assert_eq!(language(&wb, &view, cx), "JSON");
    assert!(editor_lang_id_is(&wb, "json", cx));
    assert_eq!(
        view.read_with(cx, |v, _| v.language_id().map(str::to_string)),
        Some("json".to_string()),
        "the `[json]` settings override applies to the view"
    );
    assert!(
        captures(&view, cx).iter().any(|c| c.starts_with("string")),
        "bundled JSON colours it: {:?}",
        captures(&view, cx)
    );
}

fn install_icon_theme_ext(dirs: &Dirs, id: &str, theme: &str) {
    let dir = dirs.installed(id);
    write(
        &dir.join("extension.toml"),
        format!("id = \"{id}\"\nname = \"{id}\"\nversion = \"0.1.0\"\nschema_version = 1\n"),
    );
    write(
        &dir.join("lucerna.json"),
        r#"{"commit":"","version":"0.1.0","installed_at":0,"source":"dev"}"#,
    );
    write(&dir.join("icons/zz.svg"), "<svg/>");
    write(
        &dir.join("icon_themes/icons.json"),
        format!(
            r#"{{"name":"Ext Icons","author":"t","themes":[{{"name":"{theme}","appearance":"dark","file_suffixes":{{"zz":"zz"}},"file_icons":{{"zz":{{"path":"./icons/zz.svg"}}}}}}]}}"#
        ),
    );
}

#[gpui::test]
fn icon_theme_setting_follows_extensions(cx: &mut TestAppContext) {
    let _serial = serial();
    let dirs = Dirs::new();
    install_icon_theme_ext(&dirs, "exticons", "Ext Icons");
    reindex(dirs.store.path());
    write(
        &dirs.settings(),
        r#"{ "workbench.iconTheme": "Ext Icons" }"#,
    );
    init_with_config(&dirs, cx);
    let (_wb, cx) = workbench(&dirs, cx);
    settle(cx);
    let icons = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| cx.global::<AppState>().icons.name().to_string())
    };
    assert_eq!(icons(cx), "Ext Icons");

    // Uninstalled: Seti answers, and the setting is kept.
    cx.update(|_, cx| ExtensionService::uninstall("exticons".into(), cx));
    settle(cx);
    assert_eq!(icons(cx), "Seti");
    let setting: Option<String> = cx.update(|_, cx| {
        cx.global::<AppState>()
            .settings
            .get("workbench.iconTheme", None)
    });
    assert_eq!(setting.as_deref(), Some("Ext Icons"));

    // Changing the setting re-resolves it.
    install_icon_theme_ext(&dirs, "exticons", "Ext Icons");
    index_changed(&dirs, cx);
    assert_eq!(icons(cx), "Ext Icons");
    write(&dirs.settings(), r#"{ "workbench.iconTheme": "Lucide" }"#);
    let changed = dirs.settings();
    cx.update(|_, cx| reload_config(&[changed], cx));
    settle(cx);
    assert_eq!(icons(cx), "Lucide");
}
