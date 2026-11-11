use std::path::{Path, PathBuf};

use lsp_types::{GotoDefinitionResponse, Location, LocationLink, Position, Range};
use lucerna_lsp::locations::{ResolvedLocation, locations_from_response, resolve_locations};
use lucerna_lsp::position::{PositionEncoding::Utf16, path_to_uri};
use lucerna_text::{Buffer, BufferSnapshot, Point};

fn snap(text: &str) -> BufferSnapshot {
    Buffer::from_bytes(text.as_bytes(), false)
        .unwrap()
        .snapshot()
}

fn range(sl: u32, sc: u32, el: u32, ec: u32) -> Range {
    Range::new(Position::new(sl, sc), Position::new(el, ec))
}

fn pt(row: u32, column: u32) -> Point {
    Point { row, column }
}

#[test]
fn location_link_uses_target_selection() {
    let path = Path::new("/proj/a.rs");
    let link = LocationLink {
        origin_selection_range: Some(range(9, 9, 9, 9)),
        target_uri: path_to_uri(path),
        target_range: range(0, 0, 5, 0),
        target_selection_range: range(1, 4, 1, 7),
    };
    assert_eq!(
        locations_from_response(Some(GotoDefinitionResponse::Link(vec![link]))),
        vec![(path.to_path_buf(), range(1, 4, 1, 7))]
    );
    let loc = Location {
        uri: path_to_uri(path),
        range: range(2, 0, 2, 1),
    };
    assert_eq!(
        locations_from_response(Some(GotoDefinitionResponse::Scalar(loc.clone()))),
        vec![(path.to_path_buf(), range(2, 0, 2, 1))]
    );
    // Non-file URIs are dropped; None is empty.
    let remote = Location {
        uri: "https://example.com/x".parse().unwrap(),
        range: range(0, 0, 0, 1),
    };
    assert_eq!(
        locations_from_response(Some(GotoDefinitionResponse::Array(vec![remote, loc]))),
        vec![(path.to_path_buf(), range(2, 0, 2, 1))]
    );
    assert!(locations_from_response(None).is_empty());
}

#[test]
fn unopened_file_read_from_disk() {
    let dir = tempfile::tempdir().unwrap();
    let on_disk = dir.path().join("disk.rs");
    std::fs::write(&on_disk, "first\na😀b = 1;\n").unwrap();
    let open_path = PathBuf::from("/proj/open.rs");
    let open_snap = snap("let open = 1;\n");
    let locs = vec![
        (on_disk.clone(), range(1, 3, 1, 4)),
        (open_path.clone(), range(0, 4, 0, 8)),
        (dir.path().join("missing.rs"), range(0, 0, 0, 1)),
    ];
    let got = resolve_locations(locs, Utf16, &|p| {
        (p == open_path).then(|| open_snap.clone())
    });
    let mut want = vec![
        ResolvedLocation {
            path: on_disk.as_path().into(),
            range: pt(1, 5)..pt(1, 6),
            line_text: "a😀b = 1;".into(),
        },
        ResolvedLocation {
            path: open_path.as_path().into(),
            range: pt(0, 4)..pt(0, 8),
            line_text: "let open = 1;".into(),
        },
    ];
    // Sorted by path; the missing file is dropped.
    want.sort_by(|a, b| a.path.cmp(&b.path));
    assert_eq!(got, want);
}

#[test]
fn dedup_and_sort() {
    let a = PathBuf::from("/proj/a.rs");
    let b = PathBuf::from("/proj/b.rs");
    let text = snap("one\ntwo\nthree\n");
    let locs = vec![
        (b.clone(), range(0, 0, 0, 1)),
        (a.clone(), range(2, 0, 2, 1)),
        (a.clone(), range(1, 2, 1, 3)),
        (a.clone(), range(2, 0, 2, 1)),
        (a.clone(), range(1, 0, 1, 1)),
    ];
    let got = resolve_locations(locs, Utf16, &|_| Some(text.clone()));
    let flat: Vec<_> = got
        .iter()
        .map(|l| {
            (
                l.path.to_path_buf(),
                l.range.start.row,
                l.range.start.column,
                l.line_text.clone(),
            )
        })
        .collect();
    assert_eq!(
        flat,
        vec![
            (a.clone(), 1, 0, "two".to_owned()),
            (a.clone(), 1, 2, "two".to_owned()),
            (a, 2, 0, "three".to_owned()),
            (b, 0, 0, "one".to_owned()),
        ]
    );
}
