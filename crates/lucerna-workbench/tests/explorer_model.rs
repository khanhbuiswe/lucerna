//! The pure Explorer model: visible rows, expansion, reveal and the
//! changed-file decoration. No GPUI.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lucerna_project::{Entry, EntryKind, WorktreeSnapshot};
use lucerna_workbench::{EditKind, EditState, ExplorerModel, Row, SelectedKind, Selection};

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
            edit: false,
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

#[test]
fn selection_starts_at_root() {
    let model = ExplorerModel::new(project());
    assert_eq!(model.selection(), &Selection::Root);
    assert_eq!(model.selected_kind(), SelectedKind::Root);
}

#[test]
fn select_path_reports_kind() {
    let mut model = ExplorerModel::new(project());
    model.select(Selection::Path(p("src")));
    assert_eq!(model.selected_kind(), SelectedKind::Dir);
    model.select(Selection::Path(p("Cargo.toml")));
    assert_eq!(model.selected_kind(), SelectedKind::File);
    model.select(Selection::Root);
    assert_eq!(model.selected_kind(), SelectedKind::Root);
}

/// The kind is kept with the rows: a hidden selection reads `File` until
/// its row shows, and follows a snapshot that changes the entry's kind.
#[test]
fn selected_kind_follows_rows() {
    use EntryKind::*;
    let mut model = ExplorerModel::new(project());
    model.select(Selection::Path(p("src/ui")));
    assert_eq!(model.selected_kind(), SelectedKind::File, "hidden");
    model.reveal(Path::new("src/ui"));
    assert_eq!(model.selected_kind(), SelectedKind::Dir);
    model.select(Selection::Path(p("Cargo.toml")));
    model.set_snapshot(snapshot(&[("src", Dir), ("Cargo.toml", Dir)]));
    assert_eq!(model.selected_kind(), SelectedKind::Dir);
    model.set_snapshot(snapshot(&[("src", Dir)]));
    assert_eq!(model.selection(), &Selection::Root);
    assert_eq!(model.selected_kind(), SelectedKind::Root);
}

#[test]
fn selection_falls_back_to_visible_ancestor() {
    let mut model = ExplorerModel::new(project());
    model.toggle(Path::new("src"));
    model.select(Selection::Path(p("src/main.rs")));
    assert_eq!(model.selection(), &Selection::Path(p("src/main.rs")));
    model.toggle(Path::new("src"));
    assert_eq!(model.selection(), &Selection::Path(p("src")));
    model.set_snapshot(snapshot(&[("Cargo.toml", EntryKind::File)]));
    assert_eq!(model.selection(), &Selection::Root);
}

#[test]
fn select_hidden_path_is_kept_until_tree_changes() {
    let mut model = ExplorerModel::new(project());
    model.select(Selection::Path(p("src/main.rs")));
    assert_eq!(model.selection(), &Selection::Path(p("src/main.rs")));
    model.reveal(Path::new("src/main.rs"));
    assert_eq!(model.selection(), &Selection::Path(p("src/main.rs")));
}

#[test]
fn new_file_edit_row_is_first_child() {
    use EntryKind::*;
    let mut model = ExplorerModel::new(snapshot(&[
        ("src", Dir),
        ("src/main.rs", File),
        ("a.rs", File),
        ("b.rs", File),
    ]));
    model.start_edit(EditState {
        kind: EditKind::NewFile,
        parent: p("src"),
        original: None,
    });
    let rows = model.visible_rows();
    let paths: Vec<_> = rows
        .iter()
        .map(|r| {
            if r.edit {
                "<edit>".to_string()
            } else {
                r.path.to_string_lossy().into_owned()
            }
        })
        .collect();
    assert_eq!(paths, ["src", "<edit>", "src/main.rs", "a.rs", "b.rs"]);
    assert_eq!((rows[1].depth, rows[1].kind), (1, File));
    assert!(rows[0].expanded);
    assert_eq!(model.edit().map(|e| e.kind), Some(EditKind::NewFile));
    model.end_edit();
    assert!(model.edit().is_none());
    assert!(model.visible_rows().iter().all(|r| !r.edit));
}

#[test]
fn new_folder_edit_row_on_root_is_first() {
    let mut model = ExplorerModel::new(project());
    model.start_edit(EditState {
        kind: EditKind::NewFolder,
        parent: p(""),
        original: None,
    });
    let first = &model.visible_rows()[0];
    assert!(first.edit);
    assert_eq!((first.depth, first.kind), (0, EntryKind::Dir));
    assert_eq!(model.visible_rows().len(), 4);
}
