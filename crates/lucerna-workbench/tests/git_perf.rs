//! Headless performance check for `GitStore` against a real repository
//! (spec 2026-11-13-git-view-design §5). Ignored by default: run with
//! `LUCERNA_GIT_BENCH_REPO=<path> cargo test -p lucerna-workbench --test git_perf -- --ignored --nocapture`.
//! The repo is modified: it commits on a throwaway branch and writes 500 files.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{AppContext as _, Entity, TestAppContext};
use lucerna_git::GitBinary;
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_text::BufferSnapshot;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::{AppState, GitStore};

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
