//! A workbench whose language servers are in-process fakes
//! (`lucerna_lsp::test_support`), in a folder trusted through
//! `session.trustAllWorktrees`.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Entity, TestAppContext, VisualTestContext};
use lucerna_lsp::test_support::{FakeLauncher, FakeServer};
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{AppState, LanguageServers, OpenMode, Workbench};
use serde_json::Value;

/// The user settings every LSP test starts from.
pub const TRUSTED: &str = r#"{ "session.trustAllWorktrees": true }"#;

/// Sets the [`AppState`] global with `user_settings` as the User layer.
pub fn init_with(user_settings: &str, cx: &mut TestAppContext) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "keymap error: {error:?}");
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("Lucerna Dark");
    let mut settings = SettingsStore::with_defaults();
    settings
        .set_layer(Layer::User, user_settings)
        .expect("valid user settings");
    let (icon_themes, _) = lucerna_theme::IconThemeRegistry::build(&[]);
    let icons = icon_themes.resolve("Lucide");
    cx.set_global(AppState {
        settings,
        keymap,
        themes,
        theme,
        theme_generation: 0,
        languages: Arc::new(LanguageRegistry::bundled()),
        languages_generation: 0,
        icon_themes,
        icons,
    });
}

/// A workbench on `root`, trusted, whose servers come from `fake`.
pub fn lsp_workbench<'a>(
    root: &Path,
    fake: FakeLauncher,
    cx: &'a mut TestAppContext,
) -> (Entity<Workbench>, &'a mut VisualTestContext) {
    lsp_workbench_with(root, Arc::new(fake), TRUSTED, cx)
}

/// Like [`lsp_workbench`], keeping the launcher (for `launches()`) and
/// with `user_settings` as the User layer.
pub fn lsp_workbench_with<'a>(
    root: &Path,
    launcher: Arc<FakeLauncher>,
    user_settings: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<Workbench>, &'a mut VisualTestContext) {
    init_with(user_settings, cx);
    let root = root.to_path_buf();
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root, window, cx));
    let servers = language_servers(&wb, cx);
    servers.update(cx, |servers, _| servers.set_launcher(launcher));
    (wb, cx)
}

pub fn language_servers(
    wb: &Entity<Workbench>,
    cx: &mut VisualTestContext,
) -> Entity<LanguageServers> {
    cx.update(|_, cx| wb.read(cx).language_servers().clone())
}

pub fn p(name: &str) -> Arc<Path> {
    Arc::from(Path::new(name))
}

pub fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

/// Draws, lets background work and server traffic finish, runs the
/// diagnostics (50 ms) and watched-file (100 ms) batches, and draws again.
pub fn settle(cx: &mut VisualTestContext) {
    draw(cx);
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    draw(cx);
    cx.run_until_parked();
}

pub fn active_view(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<CodeView> {
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

/// Opens `path` (relative to the root) as a pinned tab and settles.
pub fn open(wb: &Entity<Workbench>, path: &str, cx: &mut VisualTestContext) -> Entity<CodeView> {
    wb.update(cx, |wb, cx| wb.open_path(p(path), OpenMode::Pinned, cx));
    settle(cx);
    active_view(wb, cx)
}

/// Closes the active tab and settles.
pub fn close_active_tab(wb: &Entity<Workbench>, cx: &mut VisualTestContext) {
    wb.update(cx, |wb, cx| {
        wb.update_editor_groups(cx, |m| {
            let id = m.active_tab().expect("an active tab").id;
            m.close(id);
        })
    });
    settle(cx);
}

/// The params of every notification or request `method` the server got.
pub fn sent(fake: &FakeServer, method: &str) -> Vec<Value> {
    fake.received()
        .into_iter()
        .filter(|(m, _)| m == method)
        .map(|(_, params)| params)
        .collect()
}

/// The methods the server got, in order.
pub fn methods(fake: &FakeServer) -> Vec<String> {
    fake.received().into_iter().map(|(m, _)| m).collect()
}

/// Waits (at most 5 s of real time) until `done` holds: for effects of the
/// language servers' own shutdown threads, which the GPUI scheduler does
/// not drive.
pub fn wait_until(what: &str, done: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(std::time::Instant::now() < deadline, "timed out: {what}");
        std::thread::yield_now();
    }
}

/// How many times the server got `method`.
pub fn count(fake: &FakeServer, method: &str) -> usize {
    sent(fake, method).len()
}

/// Gives `view` keyboard focus, as clicking into it does.
pub fn focus_view(view: &Entity<CodeView>, cx: &mut VisualTestContext) {
    let handle = cx.update(|_, cx| gpui::Focusable::focus_handle(view.read(cx), cx));
    cx.update(|window, cx| window.focus(&handle, cx));
    settle(cx);
}

/// Whether editor keys work: `editorTextFocus` holds.
pub fn editor_text_focus(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> bool {
    let ctx = cx.update(|window, cx| wb.read(cx).context_keys(window, cx));
    lucerna_settings::parse_when("editorTextFocus && editorFocus")
        .unwrap()
        .eval(&ctx)
}
