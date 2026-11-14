//! The excerpt list as a diff view: added, deleted and context lines,
//! syntax colors from given texts, per-file meta and change navigation.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{Entity, Hsla, Modifiers, TestAppContext, VisualTestContext};
use lucerna_project::{Excerpt, LineKind, LineMatch};
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::{
    AppState, ExcerptList, ExcerptListEvent, FileMeta, SearchLineRef as LineRef,
    SearchResults as Results, SearchRow as Row, TextSource,
};
use tempfile::TempDir;

fn init(cx: &mut TestAppContext) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "keymap error: {error:?}");
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("Lucerna Dark");
    cx.set_global(AppState {
        settings: SettingsStore::with_defaults(),
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

fn line(n: u32, text: &str, kind: LineKind) -> LineMatch {
    LineMatch {
        line_number: n,
        line_text: text.into(),
        ranges: Vec::new(),
        kind,
    }
}

fn ctx(n: u32, text: &str) -> LineMatch {
    line(n, text, LineKind::Normal)
}

fn add(n: u32, text: &str) -> LineMatch {
    line(n, text, LineKind::Added)
}

/// A deleted base line `base_line`, shown before current row `n`.
fn del(n: u32, base_line: u32, text: &str) -> LineMatch {
    line(n, text, LineKind::Deleted { base_line })
}

fn path(s: &str) -> Arc<Path> {
    Arc::from(Path::new(s))
}

fn at(file: usize, excerpt: usize, line: usize) -> LineRef {
    LineRef {
        file,
        excerpt,
        line,
    }
}

/// An empty project folder and a window showing a diff list over it.
fn diff_list(cx: &mut TestAppContext) -> (TempDir, Entity<ExcerptList>, &mut VisualTestContext) {
    init(cx);
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    let (list, cx) = cx.add_window_view(move |_, cx| ExcerptList::new(root, "diff", cx));
    (dir, list, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn set_files(
    list: &Entity<ExcerptList>,
    files: Vec<(&str, Vec<Vec<LineMatch>>)>,
    cx: &mut VisualTestContext,
) {
    let files = files
        .into_iter()
        .map(|(p, excerpts)| {
            (
                path(p),
                excerpts
                    .into_iter()
                    .map(|lines| Excerpt { lines })
                    .collect(),
            )
        })
        .collect();
    list.update(cx, |l, cx| l.set_files(files, cx));
    draw(cx);
}

fn click(selector: &'static str, cx: &mut VisualTestContext) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    cx.simulate_click(bounds.center(), Modifiers::none());
    draw(cx);
}

/// The lines the list asks to open, in order.
fn activations(
    list: &Entity<ExcerptList>,
    cx: &mut VisualTestContext,
) -> Rc<RefCell<Vec<LineRef>>> {
    let events = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        let events = events.clone();
        cx.subscribe(list, move |_, ev: &ExcerptListEvent, _| {
            let ExcerptListEvent::Activate(r) = ev;
            events.borrow_mut().push(*r);
        })
        .detach();
    });
    events
}

fn ui_color(
    cx: &mut VisualTestContext,
    pick: fn(&lucerna_theme::UiColors) -> lucerna_theme::Rgba,
) -> Hsla {
    cx.update(|_, cx| {
        let c = pick(&cx.global::<AppState>().theme.ui);
        gpui::Rgba {
            r: c.r,
            g: c.g,
            b: c.b,
            a: c.a,
        }
        .into()
    })
}

#[gpui::test]
fn deleted_rows_have_no_number_and_backgrounds(cx: &mut TestAppContext) {
    let (_dir, list, cx) = diff_list(cx);
    set_files(
        &list,
        vec![(
            "a.rs",
            vec![vec![
                ctx(3, "a"),
                del(4, 4, "old"),
                add(4, "new"),
                ctx(5, "b"),
            ]],
        )],
        cx,
    );
    for row in ["diff-line-0-0-0", "diff-line-0-0-1", "diff-line-0-0-2"] {
        assert!(cx.debug_bounds(row).is_some(), "{row}");
    }
    assert!(cx.debug_bounds("diff-number-0-0-0").is_some());
    assert!(
        cx.debug_bounds("diff-number-0-0-1").is_none(),
        "a deleted row has no number"
    );
    assert!(cx.debug_bounds("diff-number-0-0-2").is_some());
    assert!(
        cx.debug_bounds("diff-bar-0-0-0").is_none(),
        "context: no bar"
    );
    let row = cx.debug_bounds("diff-line-0-0-1").unwrap();
    for bar in ["diff-bar-0-0-1", "diff-bar-0-0-2"] {
        let b = cx
            .debug_bounds(bar)
            .unwrap_or_else(|| panic!("{bar} is drawn"));
        assert_eq!(b.size.width, (row.size.height * 0.275).floor(), "{bar}");
        assert_eq!(b.origin.x, row.origin.x, "{bar} at the gutter's left edge");
    }
    let added = ui_color(cx, |ui| ui.diff_added_background);
    let deleted = ui_color(cx, |ui| ui.diff_deleted_background);
    list.read_with(cx, |l, cx| {
        assert_eq!(l.row_background(at(0, 0, 1), cx), Some(deleted));
        assert_eq!(l.row_background(at(0, 0, 2), cx), Some(added));
        assert_eq!(l.row_background(at(0, 0, 0), cx), None);
    });
}

#[gpui::test]
fn gutter_ignores_deleted_line_numbers(cx: &mut TestAppContext) {
    let (_dir, list, cx) = diff_list(cx);
    set_files(
        &list,
        vec![("a.rs", vec![vec![del(123_456, 7, "gone")]])],
        cx,
    );
    list.read_with(cx, |l, _| {
        assert_eq!(l.results().max_line_number(), 0);
        assert_eq!(l.gutter_digits(), 4);
    });
}

#[gpui::test]
fn deleted_lines_resolve_to_following_row(cx: &mut TestAppContext) {
    let (_dir, list, cx) = diff_list(cx);
    set_files(
        &list,
        vec![(
            "a.rs",
            vec![vec![
                ctx(3, "c"),
                ctx(4, "d"),
                del(5, 5, "x"),
                del(5, 6, "y"),
                ctx(5, "z"),
            ]],
        )],
        cx,
    );
    let events = activations(&list, cx);
    click("diff-line-0-0-2", cx);
    click("diff-line-0-0-3", cx);
    let got = events.borrow().clone();
    assert_eq!(got, vec![at(0, 0, 2), at(0, 0, 3)]);
    list.read_with(cx, |l, _| {
        for r in &got {
            assert_eq!(l.results().line(*r).unwrap().line_number, 5);
        }
        assert_eq!(
            l.selected(),
            Some(at(0, 0, 3)),
            "the second deleted line stays selected"
        );
    });
    click("diff-line-0-0-4", cx);
    list.read_with(cx, |l, _| assert_eq!(l.selected(), Some(at(0, 0, 4))));
}

#[gpui::test]
fn text_source_colors_deleted_lines_from_base(cx: &mut TestAppContext) {
    let (_dir, list, cx) = diff_list(cx);
    // Nothing on disk: the colors can only come from the given texts.
    set_files(
        &list,
        vec![(
            "a.rs",
            vec![vec![del(1, 1, "fn old() {}"), add(1, "fn new() {}")]],
        )],
        cx,
    );
    list.update(cx, |l, cx| {
        l.set_text_source(
            path("a.rs"),
            TextSource {
                current: Some(Arc::from("fn new() {}\n")),
                base: Some(Arc::from("fn old() {}\n")),
            },
            cx,
        )
    });
    draw(cx);
    list.read_with(cx, |l, _| {
        for r in [at(0, 0, 0), at(0, 0, 1)] {
            let spans = l.line_spans(r).expect("colored");
            assert!(
                spans
                    .iter()
                    .any(|(range, c)| *range == (0..2) && c.starts_with("keyword")),
                "{r:?}: {spans:?}"
            );
        }
    });
}

#[gpui::test]
fn next_change_skips_context_and_wraps(cx: &mut TestAppContext) {
    let mut r = Results::default();
    r.add_file(
        path("a"),
        vec![Excerpt {
            lines: vec![
                ctx(1, "1"),
                add(2, "2"),
                add(3, "3"),
                ctx(4, "4"),
                del(5, 5, "x"),
                ctx(5, "5"),
            ],
        }],
    );
    r.add_file(
        path("b"),
        vec![Excerpt {
            lines: vec![ctx(1, "1"), del(2, 2, "x"), ctx(2, "2")],
        }],
    );
    let (a1, a4, b1) = (at(0, 0, 1), at(0, 0, 4), at(1, 0, 1));
    assert_eq!(r.next_change(None, true), Some(a1));
    assert_eq!(r.next_change(None, false), Some(b1));
    assert_eq!(r.next_change(Some(a1), true), Some(a4));
    assert_eq!(
        r.next_change(Some(at(0, 0, 2)), true),
        Some(a4),
        "inside a run"
    );
    assert_eq!(
        r.next_change(Some(at(0, 0, 3)), true),
        Some(a4),
        "from context"
    );
    assert_eq!(r.next_change(Some(a4), true), Some(b1));
    assert_eq!(r.next_change(Some(b1), true), Some(a1), "wraps");
    assert_eq!(r.next_change(Some(a1), false), Some(b1), "wraps back");
    assert_eq!(r.next_change(Some(at(0, 0, 2)), false), Some(a1));
    assert_eq!(r.next_change(Some(b1), false), Some(a4));
    assert_eq!(Results::default().next_change(None, true), None);

    // The list selects the change, unfolding its file.
    let (_dir, list, cx) = diff_list(cx);
    set_files(
        &list,
        vec![
            ("a", vec![vec![ctx(1, "1"), add(2, "2")]]),
            ("b", vec![vec![ctx(1, "1"), del(2, 2, "x"), ctx(2, "2")]]),
        ],
        cx,
    );
    list.update(cx, |l, cx| l.set_all_collapsed(true, cx));
    let first = list.update(cx, |l, cx| l.step_change(true, cx));
    assert_eq!(first, Some(at(0, 0, 1)));
    let second = list.update(cx, |l, cx| l.step_change(true, cx));
    assert_eq!(second, Some(at(1, 0, 1)));
    draw(cx);
    list.read_with(cx, |l, _| {
        assert_eq!(l.selected(), Some(at(1, 0, 1)));
        assert!(!l.results().files()[1].collapsed);
    });
    assert!(cx.debug_bounds("diff-line-1-0-1").is_some());
}

#[gpui::test]
fn file_meta_header_and_note(cx: &mut TestAppContext) {
    let (_dir, list, cx) = diff_list(cx);
    list.update(cx, |l, cx| {
        l.set_file_meta(
            path("a.rs"),
            FileMeta {
                added: 3,
                removed: 1,
                label: Some("new"),
                note: None,
                openable: true,
                group: 0,
            },
            cx,
        );
        l.set_file_meta(
            path("b.bin"),
            FileMeta {
                note: Some("Binary file"),
                ..FileMeta::default()
            },
            cx,
        );
    });
    set_files(
        &list,
        vec![
            ("a.rs", vec![vec![add(1, "a")]]),
            ("b.bin", vec![vec![ctx(1, "hidden")]]),
        ],
        cx,
    );
    assert!(cx.debug_bounds("diff-stats-0").is_some());
    assert!(cx.debug_bounds("diff-label-0").is_some());
    assert!(cx.debug_bounds("diff-label-1").is_none());
    assert!(cx.debug_bounds("diff-note-1").is_some());
    assert!(cx.debug_bounds("diff-line-0-0-0").is_some());
    assert!(
        cx.debug_bounds("diff-line-1-0-0").is_none(),
        "the note replaces the excerpts"
    );
    list.read_with(cx, |l, _| {
        assert_eq!(
            l.results().rows(),
            &[
                Row::Header(0),
                Row::Line(at(0, 0, 0)),
                Row::Header(1),
                Row::Note { file: 1 },
            ]
        );
    });
    // Folded, the note goes too.
    list.update(cx, |l, cx| l.toggle_collapsed(1, cx));
    draw(cx);
    assert!(cx.debug_bounds("diff-note-1").is_none());
}

#[gpui::test]
fn unopenable_header_button_disabled(cx: &mut TestAppContext) {
    let (_dir, list, cx) = diff_list(cx);
    list.update(cx, |l, cx| {
        l.set_file_meta(
            path("gone.rs"),
            FileMeta {
                removed: 1,
                label: Some("deleted"),
                openable: false,
                ..FileMeta::default()
            },
            cx,
        )
    });
    set_files(
        &list,
        vec![
            ("a.rs", vec![vec![ctx(1, "a"), add(2, "b")]]),
            ("gone.rs", vec![vec![del(1, 1, "x")]]),
        ],
        cx,
    );
    let events = activations(&list, cx);
    click("diff-open-file-1", cx);
    assert!(events.borrow().is_empty(), "disabled");
    list.read_with(cx, |l, _| {
        assert!(!l.results().files()[1].collapsed, "the click does not fold");
    });
    click("diff-open-file-0", cx);
    assert_eq!(
        *events.borrow(),
        vec![at(0, 0, 1)],
        "without matches, the first change opens"
    );
}

#[gpui::test]
fn sources_and_meta_go_with_files_that_leave(cx: &mut TestAppContext) {
    let (_dir, list, cx) = diff_list(cx);
    let source = TextSource {
        current: Some(Arc::from("a\n")),
        base: None,
    };
    list.update(cx, |l, cx| {
        for p in ["a.rs", "b.rs"] {
            l.set_file_meta(path(p), FileMeta::default(), cx);
            l.set_text_source(path(p), source.clone(), cx);
        }
        l.add_files(
            vec![
                (
                    path("a.rs"),
                    vec![Excerpt {
                        lines: vec![add(1, "a")],
                    }],
                ),
                (
                    path("b.rs"),
                    vec![Excerpt {
                        lines: vec![add(1, "a")],
                    }],
                ),
            ],
            cx,
        );
        // A streamed update that no longer reports `a.rs`.
        l.update_results(
            |r| {
                r.begin_update();
                r.update_files(vec![(
                    path("b.rs"),
                    vec![Excerpt {
                        lines: vec![add(1, "b")],
                    }],
                )]);
                r.prune();
            },
            cx,
        );
        // Meta set ahead of a file that has not arrived stays.
        l.set_file_meta(path("later.rs"), FileMeta::default(), cx);
    });
    list.read_with(cx, |l, _| {
        assert_eq!(l.results().files().len(), 1);
        assert!(l.results().meta(Path::new("a.rs")).is_none());
        assert!(!l.has_text_source(Path::new("a.rs")));
        assert!(l.results().meta(Path::new("b.rs")).is_some());
        assert!(l.has_text_source(Path::new("b.rs")));
        assert!(l.results().meta(Path::new("later.rs")).is_some());
    });
}

#[gpui::test]
fn note_only_files_are_not_highlighted(cx: &mut TestAppContext) {
    let (_dir, list, cx) = diff_list(cx);
    list.update(cx, |l, cx| {
        l.set_file_meta(
            path("a.rs"),
            FileMeta {
                note: Some("Binary file"),
                ..FileMeta::default()
            },
            cx,
        );
        l.set_text_source(
            path("a.rs"),
            TextSource {
                current: Some(Arc::from("fn a() {}\n")),
                base: None,
            },
            cx,
        );
    });
    set_files(&list, vec![("a.rs", vec![vec![add(1, "fn a() {}")]])], cx);
    assert!(cx.debug_bounds("diff-note-0").is_some());
    list.read_with(cx, |l, _| assert_eq!(l.highlight_jobs_started(), 0));
}

#[gpui::test]
fn files_sort_by_group_then_path(cx: &mut TestAppContext) {
    let (_dir, list, cx) = diff_list(cx);
    list.update(cx, |l, cx| {
        l.set_file_meta(
            path("z.rs"),
            FileMeta {
                group: 0,
                ..FileMeta::default()
            },
            cx,
        );
        for p in ["a.rs", "b.rs"] {
            l.set_file_meta(
                path(p),
                FileMeta {
                    group: 1,
                    ..FileMeta::default()
                },
                cx,
            );
        }
        l.add_files(
            vec![
                (path("b.rs"), vec![]),
                (path("a.rs"), vec![]),
                (path("z.rs"), vec![]),
                (path("m.rs"), vec![]),
            ],
            cx,
        );
    });
    let order = |list: &Entity<ExcerptList>, cx: &mut VisualTestContext| {
        list.read_with(cx, |l, _| {
            l.results()
                .files()
                .iter()
                .map(|f| f.path.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(order(&list, cx), ["m.rs", "z.rs", "a.rs", "b.rs"]);
    // A new group moves the file; it is still found by path.
    list.update(cx, |l, cx| {
        l.set_file_meta(
            path("m.rs"),
            FileMeta {
                group: 2,
                ..FileMeta::default()
            },
            cx,
        );
        l.set_file(
            path("m.rs"),
            vec![Excerpt {
                lines: vec![add(1, "m")],
            }],
            cx,
        );
        l.remove_files(&[path("a.rs")], cx);
    });
    assert_eq!(order(&list, cx), ["z.rs", "b.rs", "m.rs"]);
    list.read_with(cx, |l, _| {
        assert_eq!(l.results().file_index(Path::new("m.rs")), Some(2));
        assert_eq!(l.results().files()[2].excerpts.len(), 1);
    });
}
