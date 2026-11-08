//! The pure Explorer model: visible rows, expansion, reveal and the
//! changed-file decoration. No GPUI.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lucerna_project::{Entry, EntryKind, WorktreeSnapshot};
use lucerna_workbench::{ExplorerModel, Row};

fn p(path: &str) -> Arc<Path> {
    Arc::from(Path::new(path))
}

fn snapshot(entries: &[(&str, EntryKind)]) -> WorktreeSnapshot {
    WorktreeSnapshot::from_entries(
        entries
            .iter()
            .map(|(path, kind)| Entry {
                path: p(path),
                kind: *kind,
            })
            .collect(),
    )
}

fn project() -> WorktreeSnapshot {
    use EntryKind::*;
    snapshot(&[
        ("src", Dir),
        ("src/main.rs", File),
        ("src/ui", Dir),
        ("src/ui/view.rs", File),
        ("tests", Dir),
        ("tests/it.rs", File),
        ("Cargo.toml", File),
    ])
}

/// `"  name/"`-style outline: two spaces per depth, `/` after expanded dirs.
fn outline(model: &ExplorerModel) -> Vec<String> {
    model
        .visible_rows()
        .iter()
        .map(|row| {
            let name = row.path.file_name().unwrap().to_string_lossy();
            let mark = if row.expanded { "/" } else { "" };
            format!("{}{name}{mark}", "  ".repeat(row.depth as usize))
        })
        .collect()
}

#[test]
fn root_children_visible_collapsed() {
    let model = ExplorerModel::new(project());
    assert_eq!(outline(&model), ["src", "tests", "Cargo.toml"]);
    assert_eq!(
        model.visible_rows()[0],
        Row {
            path: p("src"),
            depth: 0,
            kind: EntryKind::Dir,
            expanded: false,
        }
    );
    assert_eq!(model.visible_rows()[2].kind, EntryKind::File);
}

#[test]
fn toggle_expands() {
    let mut model = ExplorerModel::new(project());
    model.toggle(Path::new("src"));
    assert_eq!(
        outline(&model),
        ["src/", "  ui", "  main.rs", "tests", "Cargo.toml"]
    );
    model.toggle(Path::new("src/ui"));
    assert_eq!(
        outline(&model),
        [
            "src/",
            "  ui/",
            "    view.rs",
            "  main.rs",
            "tests",
            "Cargo.toml"
        ]
    );
    model.toggle(Path::new("src"));
    assert_eq!(outline(&model), ["src", "tests", "Cargo.toml"]);
    // Toggling a file does nothing.
    model.toggle(Path::new("Cargo.toml"));
    assert_eq!(outline(&model), ["src", "tests", "Cargo.toml"]);
}

#[test]
fn expansion_survives_snapshot_update() {
    use EntryKind::*;
    let mut model = ExplorerModel::new(project());
    model.toggle(Path::new("src"));
    model.set_snapshot(snapshot(&[
        ("src", Dir),
        ("src/lib.rs", File),
        ("src/main.rs", File),
        ("Cargo.toml", File),
        ("README.md", File),
    ]));
    assert_eq!(
        outline(&model),
        ["src/", "  lib.rs", "  main.rs", "Cargo.toml", "README.md"]
    );
}

#[test]
fn reveal_expands_ancestors() {
    let mut model = ExplorerModel::new(project());
    assert_eq!(model.reveal(Path::new("src/ui/view.rs")), Some(2));
    assert_eq!(
        outline(&model),
        [
            "src/",
            "  ui/",
            "    view.rs",
            "  main.rs",
            "tests",
            "Cargo.toml"
        ]
    );
    assert_eq!(model.reveal(Path::new("Cargo.toml")), Some(5));
    assert_eq!(model.reveal(Path::new("src/missing.rs")), None);
}

#[test]
fn changed_decoration_expires_after_10s() {
    let mut model = ExplorerModel::new(project());
    let t0 = Instant::now();
    let main = Path::new("src/main.rs");
    assert!(!model.is_recently_changed(main, t0));
    model.mark_changed(&[p("src/main.rs")], t0);
    assert!(model.is_recently_changed(main, t0));
    assert!(model.is_recently_changed(main, t0 + Duration::from_millis(9_999)));
    assert!(!model.is_recently_changed(main, t0 + Duration::from_secs(10)));
    assert!(!model.is_recently_changed(Path::new("Cargo.toml"), t0));

    // A later change restarts the 10 s.
    model.mark_changed(&[p("src/main.rs")], t0 + Duration::from_secs(8));
    assert!(model.is_recently_changed(main, t0 + Duration::from_secs(17)));
    assert!(!model.is_recently_changed(main, t0 + Duration::from_secs(18)));
}

#[test]
fn clear_on_open() {
    let mut model = ExplorerModel::new(project());
    let t0 = Instant::now();
    model.mark_changed(&[p("src/main.rs"), p("Cargo.toml")], t0);
    model.clear_changed(Path::new("src/main.rs"));
    assert!(!model.is_recently_changed(Path::new("src/main.rs"), t0));
    assert!(model.is_recently_changed(Path::new("Cargo.toml"), t0));
}

#[test]
fn next_expiry_drops_expired_and_reports_the_soonest() {
    let mut model = ExplorerModel::new(project());
    let t0 = Instant::now();
    model.mark_changed(&[p("src/main.rs")], t0);
    model.mark_changed(&[p("Cargo.toml")], t0 + Duration::from_secs(3));
    let ten = Duration::from_secs(10);
    assert_eq!(model.next_expiry(t0), Some(t0 + ten));
    assert_eq!(
        model.next_expiry(t0 + ten),
        Some(t0 + Duration::from_secs(13))
    );
    assert_eq!(model.next_expiry(t0 + Duration::from_secs(13)), None);
}

#[test]
fn removed_dir_drops_expansion() {
    use EntryKind::*;
    let mut model = ExplorerModel::new(project());
    model.toggle(Path::new("tests"));
    model.set_snapshot(snapshot(&[("src", Dir), ("Cargo.toml", File)]));
    assert_eq!(outline(&model), ["src", "Cargo.toml"]);
    // The directory comes back collapsed.
    model.set_snapshot(project());
    assert_eq!(outline(&model), ["src", "tests", "Cargo.toml"]);
}
