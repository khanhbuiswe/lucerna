//! The Uncommitted Changes tab (spec §4.7): files against HEAD in
//! conflict, tracked, new order, streamed in with bounded concurrency,
//! with diff rows, notes, empty and unavailable states, change navigation
//! and workspace save / restore.

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Entity, Focusable, TestAppContext, VisualTestContext};
use lucerna_project::{Change, ChangeKind, LineKind, WorktreeEvent};
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_text::Point;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{
    AppState, DiffKind, DiffView, GIT_CHANGES_TAB_STATE, OpenMode, SearchLineRef as LineRef,
    TabItem, Workbench,
};
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
    let theme = themes.get_or_default("Lucerna Dark");
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

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn settle(cx: &mut VisualTestContext) {
    for _ in 0..3 {
        draw(cx);
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(60));
    }
    draw(cx);
}

/// A repo (committed `a.txt` and `b.txt`, see [`repo`]) that `prepare`
/// changes before a workbench opens on it. `trusted` runs git.
fn workbench_with<'a>(
    settings: Option<&str>,
    trusted: bool,
    prepare: impl FnOnce(&Path),
    cx: &'a mut TestAppContext,
) -> (
    TempDir,
    PathBuf,
    Entity<Workbench>,
    &'a mut VisualTestContext,
) {
    init(cx, settings);
    let (tmp, dir) = repo();
    prepare(&dir);
    let root = dir.clone();
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root, window, cx));
    wb.update(cx, |wb, cx| {
        wb.git().update(cx, |git, cx| {
            git.use_git_for_test(isolated_git());
            git.set_trusted(trusted, cx);
        });
    });
    settle(cx);
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::ScanComplete, cx)
    });
    settle(cx);
    (tmp, dir, wb, cx)
}

fn workbench(
    prepare: impl FnOnce(&Path),
    cx: &mut TestAppContext,
) -> (TempDir, PathBuf, Entity<Workbench>, &mut VisualTestContext) {
    workbench_with(None, true, prepare, cx)
}

/// Runs `git.viewChanges` and returns the tab's view.
fn open_changes(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<DiffView> {
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            wb.run_command("git.viewChanges", None, window, cx)
        })
    });
    settle(cx);
    cx.update(|_, cx| wb.read(cx).git_changes_view(cx))
        .expect("the Uncommitted Changes tab is open")
}

fn file_paths(view: &Entity<DiffView>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|_, cx| {
        view.read(cx)
            .excerpt_list()
            .read(cx)
            .results()
            .files()
            .iter()
            .map(|f| f.path.to_string_lossy().into_owned())
            .collect()
    })
}

/// `(line number, kind, text)` of every line of file `file`.
fn rows(
    view: &Entity<DiffView>,
    file: usize,
    cx: &mut VisualTestContext,
) -> Vec<(u32, LineKind, String)> {
    cx.update(|_, cx| {
        view.read(cx).excerpt_list().read(cx).results().files()[file]
            .excerpts
            .iter()
            .flat_map(|e| &e.lines)
            .map(|l| (l.line_number, l.kind, l.line_text.clone()))
            .collect()
    })
}

fn message(view: &Entity<DiffView>, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| view.read(cx).message(cx))
}

fn selected(view: &Entity<DiffView>, cx: &mut VisualTestContext) -> Option<LineRef> {
    cx.update(|_, cx| view.read(cx).excerpt_list().read(cx).selected())
}

fn write(dir: &Path, name: &str, text: &str) {
    std::fs::write(dir.join(name), text).unwrap();
}

fn changed(dir: &Path, name: &str) -> WorktreeEvent {
    WorktreeEvent::Changed(vec![Change {
        path: Arc::from(dir.join(name)),
        kind: ChangeKind::Modified,
    }])
}

#[gpui::test]
fn lists_files_in_conflict_tracked_new_order(cx: &mut TestAppContext) {
    let (_t, _dir, wb, cx) = workbench(
        |dir| {
            // `zz.txt` conflicts: changed on two branches, then merged.
            write(dir, "zz.txt", "base\n");
            git(dir, &["add", "."]);
            git(dir, &["commit", "-q", "-m", "zz"]);
            git(dir, &["checkout", "-q", "-b", "other"]);
            write(dir, "zz.txt", "other\n");
            git(dir, &["commit", "-q", "-am", "other"]);
            git(dir, &["checkout", "-q", "main"]);
            write(dir, "zz.txt", "main\n");
            git(dir, &["commit", "-q", "-am", "main"]);
            let merge = Command::new("git")
                .current_dir(dir)
                .args(["merge", "-q", "other"])
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .output()
                .unwrap();
            assert!(!merge.status.success(), "the merge conflicts");
            write(dir, "a.txt", "one\nTWO\nthree\n");
            std::fs::remove_file(dir.join("b.txt")).unwrap();
            write(dir, "added.txt", "added\n");
            git(dir, &["add", "added.txt"]);
            write(dir, "0new.txt", "new\n");
        },
        cx,
    );
    let view = open_changes(&wb, cx);
    assert_eq!(
        file_paths(&view, cx),
        ["zz.txt", "a.txt", "added.txt", "b.txt", "0new.txt"],
        "conflicts, then tracked, then new; by path within each"
    );
    let labels: Vec<Option<&str>> = cx.update(|_, cx| {
        let list = view.read(cx).excerpt_list().read(cx);
        list.results()
            .files()
            .iter()
            .map(|f| list.results().meta(&f.path).and_then(|m| m.label))
            .collect()
    });
    assert_eq!(
        labels,
        [None, None, Some("new"), Some("deleted"), Some("new")]
    );
    let title = cx.update(|_, cx| view.read(cx).title());
    assert_eq!(title, "Uncommitted Changes");
    assert_eq!(
        cx.update(|_, cx| view.read(cx).kind().clone()),
        DiffKind::Uncommitted
    );
    assert!(cx.debug_bounds("git-diff-file-0").is_some());
}

#[gpui::test]
fn shows_deleted_and_added_rows_for_modified_file(cx: &mut TestAppContext) {
    let (_t, _dir, wb, cx) = workbench(|dir| write(dir, "a.txt", "one\nTWO\nthree\n"), cx);
    let view = open_changes(&wb, cx);
    assert_eq!(file_paths(&view, cx), ["a.txt"]);
    assert_eq!(
        rows(&view, 0, cx),
        vec![
            (1, LineKind::Normal, "one".into()),
            (2, LineKind::Deleted { base_line: 2 }, "two".into()),
            (2, LineKind::Added, "TWO".into()),
            (3, LineKind::Normal, "three".into()),
        ]
    );
    let (files, added, removed) = cx.update(|_, cx| view.read(cx).totals());
    assert_eq!((files, added, removed), (1, 1, 1));
    let (added_bg, theme_added) = cx.update(|_, cx| {
        let list = view.read(cx).excerpt_list().read(cx);
        let r = LineRef {
            file: 0,
            excerpt: 0,
            line: 2,
        };
        let c = cx.global::<AppState>().theme.ui.diff_added_background;
        let theme = gpui::Rgba {
            r: c.r,
            g: c.g,
            b: c.b,
            a: c.a,
        };
        (list.row_background(r, cx), Some(gpui::Hsla::from(theme)))
    });
    assert_eq!(added_bg, theme_added);
    assert!(cx.debug_bounds("git-diff-line-0-0-1").is_some());
    assert!(cx.debug_bounds("git-diff-summary").is_some());
}

#[gpui::test]
fn unsaved_buffer_text_is_used(cx: &mut TestAppContext) {
    let (_t, _dir, wb, cx) = workbench(|dir| write(dir, "a.txt", "one\nTWO\nthree\n"), cx);
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("a.txt")), OpenMode::Pinned, cx)
    });
    settle(cx);
    let code = cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .active_body()
            .and_then(|b| b.downcast::<CodeView>().ok())
            .expect("a code view")
    });
    cx.update(|window, cx| {
        let focus = code.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    });
    code.update(cx, |v, cx| v.set_cursor(Point { row: 0, column: 0 }, cx));
    cx.simulate_input("X");
    settle(cx);
    let view = open_changes(&wb, cx);
    let lines = rows(&view, 0, cx);
    assert!(
        lines.contains(&(1, LineKind::Added, "Xone".into())),
        "the buffer's unsaved text: {lines:?}"
    );
    assert!(lines.contains(&(1, LineKind::Deleted { base_line: 1 }, "one".into())));
    // An edit while the tab shows reloads the file.
    code.update(cx, |v, cx| {
        v.type_text("Y", cx);
    });
    settle(cx);
    let lines = rows(&view, 0, cx);
    assert!(
        lines.contains(&(1, LineKind::Added, "XYone".into())),
        "the edited text: {lines:?}"
    );
}

#[gpui::test]
fn deleted_file_collapsed_and_not_openable(cx: &mut TestAppContext) {
    let (_t, _dir, wb, cx) = workbench(|dir| std::fs::remove_file(dir.join("b.txt")).unwrap(), cx);
    let view = open_changes(&wb, cx);
    assert_eq!(file_paths(&view, cx), ["b.txt"]);
    let (collapsed, meta) = cx.update(|_, cx| {
        let results = view.read(cx).excerpt_list().read(cx).results();
        let f = &results.files()[0];
        (f.collapsed, results.meta(&f.path).copied())
    });
    assert!(collapsed, "deleted files start collapsed");
    let meta = meta.expect("meta");
    assert!(!meta.openable);
    assert_eq!(meta.label, Some("deleted"));
    assert_eq!((meta.added, meta.removed), (0, 1));
    assert_eq!(
        rows(&view, 0, cx),
        vec![(1, LineKind::Deleted { base_line: 1 }, "b".into())]
    );
    assert!(cx.debug_bounds("git-diff-line-0-0-0").is_none(), "folded");
}

#[gpui::test]
fn binary_file_shows_note(cx: &mut TestAppContext) {
    let (_t, _dir, wb, cx) = workbench(
        |dir| {
            std::fs::write(dir.join("img.bin"), b"\0\x01\x02").unwrap();
            git(dir, &["add", "."]);
            git(dir, &["commit", "-q", "-m", "bin"]);
            std::fs::write(dir.join("img.bin"), b"\0\x01\x03").unwrap();
        },
        cx,
    );
    let view = open_changes(&wb, cx);
    assert_eq!(file_paths(&view, cx), ["img.bin"]);
    let note = cx.update(|_, cx| {
        let results = view.read(cx).excerpt_list().read(cx).results();
        results.meta(Path::new("img.bin")).and_then(|m| m.note)
    });
    assert_eq!(note, Some("Binary file"));
    assert!(cx.debug_bounds("git-diff-note-0").is_some());
}

#[gpui::test]
fn one_file_change_keeps_selection(cx: &mut TestAppContext) {
    let base: String = (1..=20).map(|n| format!("l{n}\n")).collect();
    let (_t, dir, wb, cx) = workbench(
        |dir| {
            write(dir, "a.rs", &base);
            write(dir, "b.rs", &base);
            git(dir, &["add", "."]);
            git(dir, &["commit", "-q", "-m", "rs"]);
            write(dir, "a.rs", &base.replace("l3\n", "x3\n"));
            write(dir, "b.rs", &base.replace("l10\n", "x10\n"));
        },
        cx,
    );
    let view = open_changes(&wb, cx);
    assert_eq!(file_paths(&view, cx), ["a.rs", "b.rs"]);
    let list = cx.update(|_, cx| view.read(cx).excerpt_list().clone());
    list.update(cx, |l, cx| {
        l.step_change(true, cx);
        l.step_change(true, cx);
    });
    let anchor = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            let l = list.read(cx);
            let r = l.selected().expect("a selection");
            let results = l.results();
            let line = results.line(r).unwrap();
            (
                results.files()[r.file].path.to_path_buf(),
                line.line_number,
                line.kind,
                line.line_text.clone(),
            )
        })
    };
    let before = anchor(cx);
    assert_eq!(before.0, Path::new("b.rs"));
    // a.rs gains a hunk: its rows grow above b.rs.
    write(
        &dir,
        "a.rs",
        &base.replace("l3\n", "x3\n").replace("l15\n", "x15\n"),
    );
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&changed(&dir, "a.rs"), cx)
    });
    settle(cx);
    assert_eq!(rows(&view, 0, cx).len(), 12, "a.rs was rebuilt");
    assert_eq!(anchor(cx), before, "the selection stays on its line");
}

#[gpui::test]
fn streams_with_bounded_concurrency(cx: &mut TestAppContext) {
    let (_t, _dir, wb, cx) = workbench(
        |dir| {
            for i in 0..100 {
                write(dir, &format!("f{i:03}.txt"), "a\nb\nc\n");
            }
            git(dir, &["add", "."]);
            git(dir, &["commit", "-q", "-m", "many"]);
            for i in 0..100 {
                write(dir, &format!("f{i:03}.txt"), "a\nB\nc\n");
            }
        },
        cx,
    );
    let view = open_changes(&wb, cx);
    let (files, peak, loading) = cx.update(|_, cx| {
        let v = view.read(cx);
        let results = v.excerpt_list().read(cx).results();
        let loading = results
            .files()
            .iter()
            .filter(|f| results.meta(&f.path).is_some_and(|m| m.note.is_some()))
            .count();
        (results.files().len(), v.peak_jobs(), loading)
    });
    assert_eq!(files, 100);
    assert_eq!(loading, 0, "every file finished loading");
    assert!(peak <= 16, "at most 16 files load at once, saw {peak}");
    assert!(peak > 1, "files load in parallel, saw {peak}");
}

#[gpui::test]
fn empty_state_when_clean(cx: &mut TestAppContext) {
    let (_t, _dir, wb, cx) = workbench(|_| {}, cx);
    let view = open_changes(&wb, cx);
    assert_eq!(
        message(&view, cx).as_deref(),
        Some("No uncommitted changes")
    );
    assert!(cx.debug_bounds("git-diff-message").is_some());
    assert!(cx.debug_bounds("git-diff-trust").is_none());
}

#[gpui::test]
fn restricted_state_has_trust_button(cx: &mut TestAppContext) {
    let (_t, _dir, wb, cx) = workbench_with(None, false, |_| {}, cx);
    let view = open_changes(&wb, cx);
    assert_eq!(
        message(&view, cx).as_deref(),
        Some("Git is off in Restricted Mode")
    );
    assert!(cx.debug_bounds("git-diff-trust").is_some());
    let button = cx.debug_bounds("git-diff-trust").unwrap();
    cx.simulate_click(button.center(), gpui::Modifiers::none());
    settle(cx);
    assert!(
        cx.update(|_, cx| wb.read(cx).trust_modal().is_some()),
        "the button opens the trust dialog"
    );
}

#[gpui::test]
fn alt_f5_steps_hunks_across_files(cx: &mut TestAppContext) {
    let (_t, _dir, wb, cx) = workbench(
        |dir| {
            write(dir, "a.txt", "one\nTWO\nthree\n");
            write(dir, "b.txt", "B\n");
        },
        cx,
    );
    let view = open_changes(&wb, cx);
    let focused =
        cx.update(|window, cx| view.read(cx).focus_handle(cx).contains_focused(window, cx));
    assert!(focused, "the tab takes the focus");
    let ctx = cx.update(|window, cx| wb.read(cx).context_keys(window, cx));
    assert!(
        lucerna_settings::parse_when("gitDiffViewFocus")
            .unwrap()
            .eval(&ctx)
    );
    assert!(
        !lucerna_settings::parse_when("editorTextFocus")
            .unwrap()
            .eval(&ctx)
    );
    let at = |file, line| LineRef {
        file,
        excerpt: 0,
        line,
    };
    cx.simulate_keystrokes("alt-f5");
    settle(cx);
    assert_eq!(selected(&view, cx), Some(at(0, 1)));
    cx.simulate_keystrokes("alt-f5");
    settle(cx);
    assert_eq!(selected(&view, cx), Some(at(1, 0)));
    cx.simulate_keystrokes("alt-f5");
    settle(cx);
    assert_eq!(selected(&view, cx), Some(at(0, 1)), "wraps around");
    cx.simulate_keystrokes("shift-alt-f5");
    settle(cx);
    assert_eq!(selected(&view, cx), Some(at(1, 0)));
}

#[gpui::test]
fn tab_saved_and_restored(cx: &mut TestAppContext) {
    init(cx, None);
    let (_t, dir) = repo();
    let data = TempDir::new().unwrap();
    let items = |wb: &Entity<Workbench>, cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            wb.read(cx)
                .editor_groups()
                .read(cx)
                .model()
                .groups()
                .iter()
                .flat_map(|g| g.tabs.iter().map(|t| t.item.clone()))
                .collect::<Vec<_>>()
        })
    };
    {
        let (root, data) = (dir.clone(), data.path().to_path_buf());
        let (wb, cx) =
            cx.add_window_view(|window, cx| Workbench::with_data_dir(root, data, window, cx));
        open_changes(&wb, cx);
        assert_eq!(items(&wb, cx), [TabItem::GitChanges]);
        let state = cx.update(|_, cx| wb.read(cx).editor_groups().read(cx).model().state());
        assert_eq!(
            state.groups[0].tabs,
            [(PathBuf::from(GIT_CHANGES_TAB_STATE), true)]
        );
        wb.update(cx, |wb, cx| wb.save_workspace_state(cx));
    }
    let (root, data) = (dir.clone(), data.path().to_path_buf());
    let (wb, cx) =
        cx.add_window_view(|window, cx| Workbench::with_data_dir(root, data, window, cx));
    settle(cx);
    assert_eq!(items(&wb, cx), [TabItem::GitChanges]);
    let title =
        cx.update(|_, cx| wb.read(cx).editor_groups().read(cx).model().groups()[0].tabs[0].title());
    assert_eq!(title, "Uncommitted Changes");
    assert!(
        cx.update(|_, cx| wb.read(cx).git_changes_view(cx))
            .is_some()
    );
}

#[gpui::test]
fn independent_of_diff_base_setting(cx: &mut TestAppContext) {
    let (_t, _dir, wb, cx) = workbench_with(
        Some(r#"{ "git.diffBase": "defaultBranch" }"#),
        true,
        |dir| {
            git(dir, &["checkout", "-q", "-b", "feature"]);
            write(dir, "a.txt", "one\nTWO\nthree\n");
            git(dir, &["commit", "-q", "-am", "work"]);
        },
        cx,
    );
    let against_branch = cx.update(|_, cx| wb.read(cx).git().read(cx).snapshot().statuses.len());
    assert_eq!(against_branch, 1, "the store diffs against main");
    let view = open_changes(&wb, cx);
    assert_eq!(file_paths(&view, cx), Vec::<String>::new());
    assert_eq!(
        message(&view, cx).as_deref(),
        Some("No uncommitted changes")
    );
}
