//! The Explorer colors files and folders by git status and can show badge
//! letters, both controlled by settings.

mod support;

use std::path::Path;
use std::sync::Arc;

use gpui::{Entity, TestAppContext, VisualTestContext};
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::{AppState, Workbench};
use support::git::{git, isolated_git, repo};
use tempfile::TempDir;

fn init(cx: &mut TestAppContext, user_settings: Option<&str>) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "keymap error: {error:?}");
    let mut settings = SettingsStore::with_defaults();
    if let Some(json) = user_settings {
        settings.set_layer(Layer::User, json).unwrap();
    }
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("One Dark");
    cx.set_global(AppState {
        settings,
        keymap,
        themes,
        theme,
        theme_generation: 0,
        languages: Arc::new(LanguageRegistry::bundled()),
        languages_generation: 0,
        icon_themes: lucerna_theme::IconThemeRegistry::build(&[]).0,
        icons: lucerna_theme::IconThemeRegistry::build(&[])
            .0
            .resolve("Lucide"),
    });
}

fn settle(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

/// A repo with committed `a.txt`, `b.txt` and `src/x.rs`, `src/y.rs`;
/// `a.txt` is modified and `new.txt` untracked. The workbench is trusted
/// and has scanned.
fn workbench<'a>(
    settings: Option<&str>,
    cx: &'a mut TestAppContext,
) -> (TempDir, Entity<Workbench>, &'a mut VisualTestContext) {
    init(cx, settings);
    let (tmp, dir) = repo();
    std::fs::create_dir(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/x.rs"), "x\n").unwrap();
    std::fs::write(dir.join("src/y.rs"), "y\n").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "src"]);
    std::fs::write(dir.join("a.txt"), "changed\n").unwrap();
    std::fs::write(dir.join("new.txt"), "n\n").unwrap();
    let root = dir.clone();
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root, window, cx));
    wb.update(cx, |wb, cx| {
        wb.git().update(cx, |git, cx| {
            git.use_git_for_test(isolated_git());
            git.set_trusted(true, cx);
        });
    });
    settle(cx);
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&lucerna_project::WorktreeEvent::ScanComplete, cx)
    });
    settle(cx);
    (tmp, wb, cx)
}

fn color(wb: &Entity<Workbench>, path: &str, cx: &mut VisualTestContext) -> Option<gpui::Hsla> {
    cx.update(|_, cx| {
        wb.read(cx)
            .explorer()
            .read(cx)
            .row_text_color(Path::new(path), cx)
    })
}

fn badge(wb: &Entity<Workbench>, path: &str, cx: &mut VisualTestContext) -> Option<&'static str> {
    cx.update(|_, cx| {
        wb.read(cx)
            .explorer()
            .read(cx)
            .row_badge(Path::new(path), cx)
    })
}

fn ui(cx: &mut VisualTestContext) -> lucerna_theme::UiColors {
    cx.update(|_, cx| cx.global::<AppState>().theme.ui.clone())
}

fn hsla(c: lucerna_theme::Rgba) -> gpui::Hsla {
    gpui::Rgba {
        r: c.r,
        g: c.g,
        b: c.b,
        a: c.a,
    }
    .into()
}

/// Expands the `src` folder by clicking its row.
fn expand_src(cx: &mut VisualTestContext) {
    settle(cx);
    let bounds = cx.debug_bounds("explorer-row-src").expect("src row");
    cx.simulate_click(bounds.center(), gpui::Modifiers::none());
    settle(cx);
}

fn rescan(wb: &Entity<Workbench>, cx: &mut VisualTestContext) {
    wb.update(cx, |wb, cx| wb.git().update(cx, |g, cx| g.rescan(cx)));
    settle(cx);
}

#[gpui::test]
fn modified_file_and_parent_dir_colored(cx: &mut TestAppContext) {
    let (t, wb, cx) = workbench(None, cx);
    let ui = ui(cx);
    std::fs::write(t.path().canonicalize().unwrap().join("src/x.rs"), "x2\n").unwrap();
    rescan(&wb, cx);
    expand_src(cx);
    assert_eq!(color(&wb, "a.txt", cx), Some(hsla(ui.status_modified)));
    assert_eq!(color(&wb, "new.txt", cx), Some(hsla(ui.status_created)));
    assert_eq!(color(&wb, "src", cx), Some(hsla(ui.status_modified)));
    assert_eq!(color(&wb, "src/x.rs", cx), Some(hsla(ui.status_modified)));
    assert_eq!(color(&wb, "src/y.rs", cx), None);
    assert_eq!(color(&wb, "b.txt", cx), None);
}

#[gpui::test]
fn conflict_beats_modified_in_dir(cx: &mut TestAppContext) {
    let (t, wb, cx) = workbench(None, cx);
    let ui = ui(cx);
    let dir = t.path().canonicalize().unwrap();
    git(&dir, &["commit", "-q", "-am", "a"]);
    git(&dir, &["checkout", "-q", "-b", "side"]);
    std::fs::write(dir.join("src/x.rs"), "side\n").unwrap();
    git(&dir, &["commit", "-q", "-am", "side x"]);
    git(&dir, &["checkout", "-q", "main"]);
    std::fs::write(dir.join("src/x.rs"), "main\n").unwrap();
    git(&dir, &["commit", "-q", "-am", "main x"]);
    let merge = std::process::Command::new("git")
        .current_dir(&dir)
        .args(["merge", "side"])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .unwrap();
    assert!(!merge.status.success(), "the merge conflicts");
    std::fs::write(dir.join("src/y.rs"), "y2\n").unwrap();
    rescan(&wb, cx);
    expand_src(cx);
    assert_eq!(color(&wb, "src/x.rs", cx), Some(hsla(ui.status_conflict)));
    assert_eq!(color(&wb, "src/y.rs", cx), Some(hsla(ui.status_modified)));
    assert_eq!(color(&wb, "src", cx), Some(hsla(ui.status_conflict)));
}

#[gpui::test]
fn colors_off_by_setting(cx: &mut TestAppContext) {
    let (_t, wb, cx) = workbench(Some(r#"{ "git.decorations.enabled": false }"#), cx);
    assert_eq!(color(&wb, "a.txt", cx), None);
}

#[gpui::test]
fn colors_off_by_explorer_setting(cx: &mut TestAppContext) {
    let (_t, wb, cx) = workbench(Some(r#"{ "explorer.decorations.colors": false }"#), cx);
    assert_eq!(color(&wb, "a.txt", cx), None);
}

#[gpui::test]
fn badges_when_enabled(cx: &mut TestAppContext) {
    let (_t, wb, cx) = workbench(Some(r#"{ "explorer.decorations.badges": true }"#), cx);
    assert_eq!(badge(&wb, "a.txt", cx), Some("M"));
    assert_eq!(badge(&wb, "new.txt", cx), Some("U"));
    assert_eq!(badge(&wb, "b.txt", cx), None);
    assert_eq!(badge(&wb, "src", cx), None);
}

#[gpui::test]
fn badges_off_by_default(cx: &mut TestAppContext) {
    let (_t, wb, cx) = workbench(None, cx);
    assert_eq!(badge(&wb, "a.txt", cx), None);
}

#[gpui::test]
fn external_change_bold_keeps_git_color(cx: &mut TestAppContext) {
    let (_t, wb, cx) = workbench(None, cx);
    let ui = ui(cx);
    let change = lucerna_project::Change {
        path: Arc::from(Path::new("a.txt")),
        kind: lucerna_project::ChangeKind::Modified,
    };
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&lucerna_project::WorktreeEvent::Changed(vec![change]), cx)
    });
    settle(cx);
    let changed = cx.update(|_, cx| {
        wb.read(cx)
            .explorer()
            .read(cx)
            .model()
            .is_recently_changed(Path::new("a.txt"), std::time::Instant::now())
    });
    assert!(changed);
    assert_eq!(color(&wb, "a.txt", cx), Some(hsla(ui.status_modified)));
}
