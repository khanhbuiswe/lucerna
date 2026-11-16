use lucerna_project::{Entry, EntryKind, FileMatch, WorktreeSnapshot, match_files};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

fn snapshot(files: &[&str]) -> WorktreeSnapshot {
    let mut entries = Vec::new();
    for f in files {
        let mut dir = Path::new(f).parent();
        while let Some(d) = dir.filter(|d| !d.as_os_str().is_empty()) {
            let e = Entry {
                path: Arc::from(d),
                kind: EntryKind::Dir,
            };
            if !entries.contains(&e) {
                entries.push(e);
            }
            dir = d.parent();
        }
        entries.push(Entry {
            path: Arc::from(Path::new(f)),
            kind: EntryKind::File,
        });
    }
    WorktreeSnapshot::from_entries(entries)
}

fn run(s: &WorktreeSnapshot, q: &str, recent: &[&str], limit: usize) -> Vec<FileMatch> {
    let recent: Vec<Arc<Path>> = recent.iter().map(|p| Arc::from(Path::new(p))).collect();
    match_files(s, q, &recent, limit, &AtomicBool::new(false))
}

fn names(m: &[FileMatch]) -> Vec<String> {
    m.iter()
        .map(|m| m.path.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn matches_subsequence() {
    let s = snapshot(&["crates/workbench/src/main.rs", "README.md", "src/lib.rs"]);
    assert_eq!(
        names(&run(&s, "wbmain", &[], 10)),
        ["crates/workbench/src/main.rs"]
    );
}

#[test]
fn recent_first() {
    let s = snapshot(&["src/main.rs", "src/domain.rs", "src/other/main.rs"]);
    let m = run(&s, "main", &["src/other/main.rs", "src/domain.rs"], 10);
    assert_eq!(&names(&m)[..2], ["src/other/main.rs", "src/domain.rs"]);
    assert_eq!(names(&m)[2], "src/main.rs");
}

#[test]
fn shorter_path_breaks_tie() {
    let s = snapshot(&["a/b/c/foo.rs", "foo.rs", "a/foo.rs"]);
    let m = run(&s, "foo.rs", &[], 10);
    assert_eq!(names(&m), ["foo.rs", "a/foo.rs", "a/b/c/foo.rs"]);
}

#[test]
fn empty_query_recent_then_alpha() {
    let s = snapshot(&["b.rs", "a.rs", "c.rs", "d.rs"]);
    let m = run(&s, "", &["c.rs", "gone.rs", "b.rs"], 10);
    assert_eq!(names(&m), ["c.rs", "b.rs", "a.rs", "d.rs"]);
}

#[test]
fn limit_respected() {
    let files: Vec<String> = (0..100).map(|i| format!("src/file{i}.rs")).collect();
    let refs: Vec<&str> = files.iter().map(String::as_str).collect();
    let s = snapshot(&refs);
    assert_eq!(run(&s, "file", &[], 7).len(), 7);
    assert_eq!(run(&s, "", &[], 7).len(), 7);
}

#[test]
fn cancel_returns_empty() {
    let s = snapshot(&["a.rs", "b.rs"]);
    let cancel = AtomicBool::new(true);
    cancel.store(true, Ordering::Relaxed);
    assert!(match_files(&s, "a", &[], 10, &cancel).is_empty());
    assert!(match_files(&s, "", &[], 10, &cancel).is_empty());
}

#[test]
fn directories_are_not_candidates() {
    let s = snapshot(&["srcdir/x.txt"]);
    assert_eq!(names(&run(&s, "srcdir", &[], 10)), ["srcdir/x.txt"]);
    assert!(
        run(&s, "srcdir", &[], 10)
            .iter()
            .all(|m| &*m.path != Path::new("srcdir"))
    );
}

#[test]
fn positions_point_at_matched_chars() {
    let s = snapshot(&["src/main.rs"]);
    let m = run(&s, "smr", &[], 10);
    assert_eq!(m.len(), 1);
    let chars: Vec<char> = m[0].path.to_string_lossy().chars().collect();
    let p = &m[0].positions;
    assert!(p.windows(2).all(|w| w[0] < w[1]));
    let picked: String = p
        .iter()
        .map(|&i| chars[i as usize].to_ascii_lowercase())
        .collect();
    assert_eq!(picked, "smr");
}

/// Run with `cargo test -p lucerna-project --release --test fuzzy -- --ignored --nocapture`.
#[test]
#[ignore = "timing check, release only"]
fn eighty_thousand_files_under_budget() {
    let files: Vec<String> = (0..80_000)
        .map(|i| format!("crates/c{}/src/mod{}/file{}.rs", i % 40, i % 997, i))
        .collect();
    let refs: Vec<&str> = files.iter().map(String::as_str).collect();
    let s = snapshot(&refs);
    let start = std::time::Instant::now();
    let m = run(&s, "c1modfile7", &[], 50);
    let took = start.elapsed();
    println!("80k files: {took:?}, {} matches", m.len());
    assert!(!m.is_empty());
}

#[test]
fn nfc_query_matches_nfd_name_with_valid_char_positions() {
    let name = "ghi-chu\u{301}.md"; // NFD
    let s = snapshot(&[name]);
    let m = run(&s, "chú", &[], 10); // NFC query
    assert_eq!(m.len(), 1);
    let chars = name.chars().count() as u32;
    assert!(m[0].positions.iter().all(|&p| p < chars));
    // "chu" + combining acute are chars 4..=7 of the displayed string.
    assert_eq!(m[0].positions, vec![4, 5, 6, 7]);
}

#[test]
fn nfd_query_matches_nfc_name() {
    let s = snapshot(&["ghi-chú.md"]);
    let m = run(&s, "chu\u{301}", &[], 10);
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].positions, vec![4, 5, 6]);
}

#[test]
fn emoji_names_do_not_panic_and_positions_are_valid() {
    let name = "\u{2764}\u{fe0f}-chú.md";
    let s = snapshot(&[name]);
    let m = run(&s, "chú", &[], 10);
    assert_eq!(m.len(), 1);
    let chars = name.chars().count() as u32;
    assert!(m[0].positions.iter().all(|&p| p < chars));
    assert_eq!(m[0].positions, vec![3, 4, 5]);
}
