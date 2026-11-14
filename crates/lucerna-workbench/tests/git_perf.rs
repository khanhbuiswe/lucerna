//! Headless performance check for `GitStore` against a real repository
//! (spec 2026-11-13-git-view-design §5). Ignored by default: run with
//! `LUCERNA_GIT_BENCH_REPO=<path> cargo test -p lucerna-workbench --test git_perf -- --ignored --nocapture`.
//! The repo is modified: it commits on a throwaway branch and writes 500 files
//! (and the diff tab tests rewrite 200 files, then restore them), so each test
//! refuses to start unless `git status` is clean. Use a throwaway clone.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use lucerna_git::GitBinary;
use lucerna_project::WorktreeEvent;
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_text::BufferSnapshot;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{AppState, DiffView, GitStore, OpenMode, Workbench};

#[cfg(windows)]
const NULL: &str = "NUL";
#[cfg(not(windows))]
const NULL: &str = "/dev/null";

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

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", NULL)
        .env("GIT_CONFIG_SYSTEM", NULL)
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Refuses to run on a repository with uncommitted or untracked changes.
fn ensure_clean(dir: &Path) {
    let status = git(dir, &["status", "--porcelain"]);
    assert!(
        status.trim().is_empty(),
        "refusing to run: {} has uncommitted changes (use a throwaway clone):\n{status}",
        dir.display()
    );
}

fn p(s: &str) -> Arc<Path> {
    Arc::from(Path::new(s))
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

#[gpui::test]
#[ignore = "needs LUCERNA_GIT_BENCH_REPO"]
fn git_store_timings(cx: &mut TestAppContext) {
    let Some(root) = std::env::var_os("LUCERNA_GIT_BENCH_REPO") else {
        eprintln!("LUCERNA_GIT_BENCH_REPO not set; skipping");
        return;
    };
    let dir = PathBuf::from(root).canonicalize().unwrap();
    ensure_clean(&dir);
    init(cx, None);
    git(&dir, &["checkout", "-q", "-B", "lucerna-bench"]);

    // (a) first rescan -> first snapshot.
    let t = Instant::now();
    let store: Entity<GitStore> = {
        let root = dir.clone();
        let store = cx.new(|cx| {
            let mut s = GitStore::new(root, cx);
            s.use_git_for_test(
                GitBinary::new(PathBuf::from("git"))
                    .with_env("GIT_CONFIG_GLOBAL", NULL)
                    .with_env("GIT_CONFIG_SYSTEM", NULL),
            );
            s
        });
        store.update(cx, |s, cx| s.set_trusted(true, cx));
        cx.run_until_parked();
        store
    };
    eprintln!(
        "(a) first rescan -> snapshot: {:.1} ms (generation {})",
        ms(t),
        store.read_with(cx, |s, _| s.snapshot().generation)
    );

    // (b) one modified file.
    let target = "crates/editor/src/editor.rs";
    let original = std::fs::read_to_string(dir.join(target)).unwrap();
    std::fs::write(dir.join(target), format!("// bench\n{original}")).unwrap();
    let t = Instant::now();
    store.update(cx, |s, cx| s.paths_changed(vec![p(target)], cx));
    cx.run_until_parked();
    let n = store.read_with(cx, |s, _| s.snapshot().statuses.len());
    eprintln!(
        "(b) paths_changed(1 file) -> snapshot: {:.1} ms ({n} statuses)",
        ms(t)
    );

    // (c) 500 files written at once.
    let before = store.read_with(cx, |s, _| s.jobs_run());
    let t = Instant::now();
    for i in 0..500 {
        let name = format!("crates/bench_f{i}.txt");
        std::fs::write(dir.join(&name), "x\n").unwrap();
        store.update(cx, |s, cx| s.paths_changed(vec![p(&name)], cx));
    }
    cx.run_until_parked();
    let ran = store.read_with(cx, |s, _| s.jobs_run()) - before;
    let n = store.read_with(cx, |s, _| s.snapshot().statuses.len());
    eprintln!(
        "(c) 500 files -> settled: {:.1} ms, {ran} jobs, {n} statuses",
        ms(t)
    );

    // (d) commit then rescan.
    git(&dir, &["add", "-A"]);
    git(
        &dir,
        &[
            "-c",
            "user.name=Bench",
            "-c",
            "user.email=b@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "bench",
            "--no-verify",
        ],
    );
    let t = Instant::now();
    store.update(cx, |s, cx| s.rescan(cx));
    cx.run_until_parked();
    let n = store.read_with(cx, |s, _| s.snapshot().statuses.len());
    eprintln!(
        "(d) commit -> rescan -> snapshot: {:.1} ms ({n} statuses)",
        ms(t)
    );

    // (e) track a ~13k-line buffer: base blob read + diff.
    let lines = original.lines().count();
    let edited = format!("// bench\n{original}");
    git(&dir, &["reset", "-q", "--hard", "HEAD~1"]);
    store.update(cx, |s, cx| s.rescan(cx));
    cx.run_until_parked();
    let t = Instant::now();
    store.update(cx, |s, cx| {
        s.track_buffer(p(target), BufferSnapshot::from_text(&edited), cx)
    });
    cx.executor().advance_clock(Duration::from_millis(50));
    cx.run_until_parked();
    let hunks = store.read_with(cx, |s, _| s.hunks(Path::new(target)));
    eprintln!(
        "(e) track_buffer {lines} lines -> hunks: {:.1} ms real (+50 ms debounce on the fake clock), {:?} hunks",
        ms(t),
        hunks.map(|h| h.len())
    );
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

/// A workbench on `dir` with git trusted and the first scan done.
fn open_workbench<'a>(
    dir: &Path,
    cx: &'a mut TestAppContext,
) -> (Entity<Workbench>, &'a mut VisualTestContext) {
    let root = dir.to_path_buf();
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root, window, cx));
    wb.update(cx, |wb, cx| {
        wb.git().update(cx, |git, cx| {
            git.use_git_for_test(
                GitBinary::new(PathBuf::from("git"))
                    .with_env("GIT_CONFIG_GLOBAL", NULL)
                    .with_env("GIT_CONFIG_SYSTEM", NULL),
            );
            git.set_trusted(true, cx);
        });
    });
    cx.run_until_parked();
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::ScanComplete, cx)
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(60));
    cx.run_until_parked();
    (wb, cx)
}

/// Files of `view` whose hunks are in the list.
fn loaded_files(view: &Entity<DiffView>, cx: &mut VisualTestContext) -> usize {
    cx.update(|_, cx| {
        view.read(cx)
            .excerpt_list()
            .read(cx)
            .results()
            .files()
            .iter()
            .filter(|f| !f.excerpts.is_empty())
            .count()
    })
}

/// Spec §5: the Uncommitted Changes tab with 200 modified files. Times, on
/// the real clock, `git.viewChanges` until (1) all headers are listed,
/// (2) the first 10 files (what a window shows) and the first 30 have their
/// hunks, and (3) all 200 are loaded. The test executor runs background
/// work on this thread one task at a time, so loads do not overlap as they
/// do in the app: the numbers are an upper bound.
#[gpui::test]
#[ignore = "needs LUCERNA_GIT_BENCH_REPO"]
fn diff_tab_200_files(cx: &mut TestAppContext) {
    let Some(root) = std::env::var_os("LUCERNA_GIT_BENCH_REPO") else {
        eprintln!("LUCERNA_GIT_BENCH_REPO not set; skipping");
        return;
    };
    let dir = PathBuf::from(root).canonicalize().unwrap();
    ensure_clean(&dir);
    init(cx, None);
    let listing = git(&dir, &["ls-files", "*.rs"]);
    let files: Vec<&str> = listing.lines().take(200).collect();
    assert_eq!(files.len(), 200);
    for f in &files {
        let text = std::fs::read_to_string(dir.join(f)).unwrap();
        std::fs::write(dir.join(f), format!("// bench\n{text}")).unwrap();
    }

    let (wb, cx) = open_workbench(&dir, cx);
    let t = Instant::now();
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            wb.run_command("git.viewChanges", None, window, cx)
        })
    });
    let view = cx
        .update(|_, cx| wb.read(cx).git_changes_view(cx))
        .expect("the tab is open");
    let (mut headers, mut first10, mut first, mut all) = (None, None, None, None);
    let mut ticks = 0usize;
    let mut longest = (0.0f64, 0usize);
    loop {
        let tick = Instant::now();
        let more = cx.executor().tick();
        let d = ms(tick);
        ticks += 1;
        if d > longest.0 {
            longest = (d, ticks);
        }
        if headers.is_none() && cx.update(|_, cx| view.read(cx).totals().0) == 200 {
            headers = Some(ms(t));
        }
        if headers.is_some() && all.is_none() {
            let n = loaded_files(&view, cx);
            if first10.is_none() && n >= 10 {
                first10 = Some(ms(t));
            }
            if first.is_none() && n >= 30 {
                first = Some(ms(t));
            }
            if n >= 200 {
                all = Some(ms(t));
            }
        }
        if !more {
            // Idle: let the debounce timers fire, then stop when parked.
            cx.executor().advance_clock(Duration::from_millis(60));
            if !cx.executor().tick() {
                break;
            }
        }
    }
    draw(cx);
    let (listed, added, removed) = cx.update(|_, cx| view.read(cx).totals());
    eprintln!(
        "diff tab, 200 files: headers listed {:?} ms, first 10 files {:?} ms, first 30 files' hunks {:?} ms, all 200 {:?} ms ({listed} files +{added} -{removed}; {ticks} ticks, longest tick {:.1} ms; peak jobs {}, loads {})",
        headers,
        first10,
        first,
        all,
        longest.0,
        cx.update(|_, cx| view.read(cx).peak_jobs()),
        cx.update(|_, cx| view.read(cx).loads_started()),
    );
    let t = Instant::now();
    draw(cx);
    eprintln!("diff tab, one more draw: {:.1} ms", ms(t));
    drop(view);
    git(&dir, &["checkout", "-q", "--", "."]);
}

/// A File (full-file) tab on a ~13k-line file: one edit, then the debounced
/// reload. Per executor task, the time is printed, so the background load
/// (a long task that does the git diff) is told apart from the tasks on the
/// UI side (the debounce flush and `finish_load`).
#[gpui::test]
#[ignore = "needs LUCERNA_GIT_BENCH_REPO"]
fn file_tab_reload(cx: &mut TestAppContext) {
    let Some(root) = std::env::var_os("LUCERNA_GIT_BENCH_REPO") else {
        eprintln!("LUCERNA_GIT_BENCH_REPO not set; skipping");
        return;
    };
    let dir = PathBuf::from(root).canonicalize().unwrap();
    ensure_clean(&dir);
    init(cx, None);
    let target = "crates/editor/src/editor.rs";
    let original = std::fs::read_to_string(dir.join(target)).unwrap();
    let lines = original.lines().count();
    std::fs::write(dir.join(target), format!("// bench\n{original}")).unwrap();

    let (wb, cx) = open_workbench(&dir, cx);
    wb.update(cx, |wb, cx| wb.open_path(p(target), OpenMode::Pinned, cx));
    cx.run_until_parked();
    let t = Instant::now();
    cx.update(|window, cx| wb.update(cx, |wb, cx| wb.open_file_diff(p(target), None, window, cx)));
    cx.run_until_parked();
    let view = cx
        .update(|_, cx| wb.read(cx).file_diff_view(Path::new(target), cx))
        .expect("the file diff tab is open");
    eprintln!("file tab, {lines} lines: open -> loaded {:.1} ms", ms(t));
    let t = Instant::now();
    draw(cx);
    eprintln!("file tab: first draw {:.1} ms", ms(t));

    // An edit in the open buffer.
    let code = open_code_view(&wb, target, cx);
    let t = Instant::now();
    code.update(cx, |v, cx| {
        v.type_text("Y", cx);
    });
    eprintln!("file tab: type_text (UI thread) {:.1} ms", ms(t));
    let start = Instant::now();
    let mut n = 0;
    let mut slow = Vec::new();
    loop {
        let tick = Instant::now();
        let mut more = cx.executor().tick();
        if !more {
            // Idle: let the debounce timer fire.
            cx.executor().advance_clock(Duration::from_millis(60));
            more = cx.executor().tick();
        }
        let d = ms(tick);
        n += 1;
        if d > 1.0 {
            slow.push((n, d));
        }
        if !more {
            break;
        }
    }
    eprintln!(
        "file tab: edit -> reload settled {:.1} ms real ({n} ticks); ticks over 1 ms (index, ms): {slow:?}",
        ms(start)
    );
    let t = Instant::now();
    draw(cx);
    eprintln!("file tab: draw after reload {:.1} ms", ms(t));
    drop(view);
    git(&dir, &["checkout", "-q", "--", "."]);
}

fn open_code_view(
    wb: &Entity<Workbench>,
    target: &str,
    cx: &mut VisualTestContext,
) -> Entity<CodeView> {
    wb.update(cx, |wb, cx| wb.open_path(p(target), OpenMode::Pinned, cx));
    cx.run_until_parked();
    cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .active_body()
            .and_then(|b| b.downcast::<CodeView>().ok())
            .expect("a code view")
    })
}
