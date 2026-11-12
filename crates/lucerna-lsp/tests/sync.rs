use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::executor::block_on;
use lsp_types::{
    FileChangeType, FileSystemWatcher, GlobPattern, OneOf, RelativePattern, ServerCapabilities,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncOptions, WatchKind,
    WorkspaceFolder,
};
use lucerna_lsp::sync::{DocumentSync, WatchedFiles, too_large_for_lsp};
use lucerna_lsp::test_support::FakeServer;
use lucerna_lsp::{LanguageServer, ServerKey, initialize_params};
use lucerna_text::{Buffer, BufferSnapshot, Patch, PatchEdit};
use serde_json::{Value, json};

fn snap(text: &str) -> BufferSnapshot {
    Buffer::from_bytes(text.as_bytes(), false)
        .unwrap()
        .snapshot()
}

fn key(root: &str) -> ServerKey {
    ServerKey {
        root: Path::new(root).into(),
        server: "fake".into(),
    }
}

fn sync_caps(sync: TextDocumentSyncCapability) -> ServerCapabilities {
    ServerCapabilities {
        position_encoding: Some(lsp_types::PositionEncodingKind::UTF16),
        text_document_sync: Some(sync),
        ..Default::default()
    }
}

fn started(fake: &FakeServer) -> Vec<(ServerKey, Arc<LanguageServer>)> {
    let (server, _events) =
        LanguageServer::start("fake", |inbox| fake.launch(inbox), json!({})).unwrap();
    block_on(server.initialize(initialize_params(Path::new("/proj"), None))).unwrap();
    vec![(key("/proj"), server)]
}

fn notes(fake: &FakeServer) -> Vec<(String, Value)> {
    fake.received()
        .into_iter()
        .filter(|(m, _)| m.starts_with("textDocument/"))
        .collect()
}

const FILE: &str = "/proj/a.rs";

#[test]
fn open_once_per_key() {
    let fake = FakeServer::new();
    let servers = started(&fake);
    let mut sync = DocumentSync::default();
    let path = Path::new(FILE);
    sync.open(path, "rust", &snap("fn a() {}\n"), &servers);
    sync.open(path, "rust", &snap("fn a() {}\n"), &servers);

    let got = notes(&fake);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].0, "textDocument/didOpen");
    let doc = &got[0].1["textDocument"];
    assert_eq!(doc["uri"], "file:///proj/a.rs");
    assert_eq!(doc["languageId"], "rust");
    assert_eq!(doc["version"], 0);
    assert_eq!(doc["text"], "fn a() {}\n");
    assert_eq!(sync.version(path, &servers[0].0), Some(0));
    assert!(sync.is_open(path, &servers[0].0));
    assert!(!sync.is_open(path, &key("/other")));

    // A second server key opens independently.
    let other = (key("/other"), Arc::clone(&servers[0].1));
    sync.open(
        path,
        "rust",
        &snap("fn a() {}\n"),
        std::slice::from_ref(&other),
    );
    assert_eq!(notes(&fake).len(), 2);
}

#[test]
fn change_bumps_version_full_and_incremental() {
    let full = FakeServer::new();
    let full_servers = started(&full);
    let mut sync = DocumentSync::default();
    let path = Path::new(FILE);
    sync.open(path, "rust", &snap("one\n"), &full_servers);
    sync.change(path, &snap("two\n"), None, &full_servers);
    sync.change(path, &snap("three\n"), None, &full_servers);
    assert_eq!(sync.version(path, &full_servers[0].0), Some(2));
    let got = notes(&full);
    assert_eq!(got[1].0, "textDocument/didChange");
    assert_eq!(got[1].1["textDocument"]["version"], 1);
    assert_eq!(
        got[1].1["contentChanges"],
        json!([{ "text": "two\n" }]),
        "full sync sends the whole text, no range"
    );
    assert_eq!(got[2].1["textDocument"]["version"], 2);

    // Incremental: one change replacing 0:0..end of the OLD text. The old
    // text ends inside an emoji line; UTF-16 counts the surrogate pair.
    let inc = FakeServer::new().capabilities(sync_caps(TextDocumentSyncCapability::Kind(
        TextDocumentSyncKind::INCREMENTAL,
    )));
    let inc_servers = started(&inc);
    let mut sync = DocumentSync::default();
    sync.open(path, "rust", &snap("ab\nc😀"), &inc_servers);
    sync.change(path, &snap("new text"), None, &inc_servers);
    let got = notes(&inc);
    assert_eq!(
        got[1].1["contentChanges"],
        json!([{
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 1, "character": 3 },
            },
            "text": "new text",
        }])
    );
    // The next change is relative to the previously sent snapshot.
    sync.change(path, &snap("x"), None, &inc_servers);
    let got = notes(&inc);
    assert_eq!(
        got[2].1["contentChanges"][0]["range"]["end"],
        json!({ "line": 0, "character": 8 })
    );

    // Options form.
    let opts = FakeServer::new().capabilities(sync_caps(TextDocumentSyncCapability::Options(
        TextDocumentSyncOptions {
            change: Some(TextDocumentSyncKind::INCREMENTAL),
            ..Default::default()
        },
    )));
    let servers = started(&opts);
    let mut sync = DocumentSync::default();
    sync.open(path, "rust", &snap("a"), &servers);
    sync.change(path, &snap("b"), None, &servers);
    assert!(notes(&opts)[1].1["contentChanges"][0]["range"].is_object());

    // Close sends didClose once and forgets the file.
    sync.close(path, &servers);
    sync.close(path, &servers);
    let closes = notes(&opts)
        .into_iter()
        .filter(|(m, _)| m == "textDocument/didClose")
        .count();
    assert_eq!(closes, 1);
    assert_eq!(sync.version(path, &servers[0].0), None);
    // Changes to a closed file are ignored.
    sync.change(path, &snap("c"), None, &servers);
    assert_eq!(notes(&opts).len(), 3);
}

#[test]
fn none_sync_sends_nothing() {
    let fake = FakeServer::new().capabilities(sync_caps(TextDocumentSyncCapability::Kind(
        TextDocumentSyncKind::NONE,
    )));
    let servers = started(&fake);
    let mut sync = DocumentSync::default();
    let path = Path::new(FILE);
    sync.open(path, "rust", &snap("a"), &servers);
    sync.change(path, &snap("b"), None, &servers);
    let got = notes(&fake);
    assert_eq!(got.len(), 1, "only didOpen: {got:?}");
    assert_eq!(sync.version(path, &servers[0].0), Some(0));
}

#[test]
fn forget_server_drops_state_without_notifying() {
    let fake = FakeServer::new();
    let servers = started(&fake);
    let mut sync = DocumentSync::default();
    let path = Path::new(FILE);
    sync.open(path, "rust", &snap("a"), &servers);
    sync.forget_server(&servers[0].0);
    assert!(!sync.is_open(path, &servers[0].0));
    assert_eq!(notes(&fake).len(), 1);
}

#[test]
fn large_file_and_long_line_detected() {
    let small = snap("a\nb\nc\n");
    assert!(!too_large_for_lsp(&small, 100, 10));
    assert!(too_large_for_lsp(&small, 4, 10), "bytes");
    assert!(too_large_for_lsp(&small, 100, 2), "lines");
    let long = snap(&"x".repeat(20_001));
    assert!(too_large_for_lsp(&long, u64::MAX, u32::MAX));
    let ok = snap(&"x".repeat(20_000));
    assert!(!too_large_for_lsp(&ok, u64::MAX, u32::MAX));
    // The limit counts chars, not bytes.
    let wide = snap(&"é".repeat(15_000));
    assert!(!too_large_for_lsp(&wide, u64::MAX, u32::MAX));
}

fn watcher(pattern: GlobPattern, kind: Option<WatchKind>) -> FileSystemWatcher {
    FileSystemWatcher {
        glob_pattern: pattern,
        kind,
    }
}

fn uris(
    events: &std::collections::HashMap<ServerKey, Vec<lsp_types::FileEvent>>,
    k: &ServerKey,
) -> Vec<(String, FileChangeType)> {
    events
        .get(k)
        .map(|v| {
            v.iter()
                .map(|e| (e.uri.as_str().to_owned(), e.typ))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn watched_glob_relative_pattern_and_kind() {
    let k = key("/proj");
    let mut watched = WatchedFiles::default();
    watched.register(
        k.clone(),
        "a".into(),
        vec![
            // Relative to the server root.
            watcher(GlobPattern::String("**/*.rs".into()), None),
            // Absolute string pattern.
            watcher(GlobPattern::String("/etc/hosts".into()), None),
            // Relative pattern against a workspace folder base.
            watcher(
                GlobPattern::Relative(RelativePattern {
                    base_uri: OneOf::Left(WorkspaceFolder {
                        uri: "file:///proj/sub".parse().unwrap(),
                        name: "sub".into(),
                    }),
                    pattern: "*.toml".into(),
                }),
                Some(WatchKind::Create),
            ),
            // Relative pattern against a plain Uri base.
            watcher(
                GlobPattern::Relative(RelativePattern {
                    base_uri: OneOf::Right("file:///proj/gen".parse().unwrap()),
                    pattern: "{a,b}.txt".into(),
                }),
                Some(WatchKind::Delete),
            ),
        ],
    );
    let changes = vec![
        (PathBuf::from("/proj/src/lib.rs"), FileChangeType::CHANGED),
        (PathBuf::from("/proj/README.md"), FileChangeType::CHANGED),
        (PathBuf::from("/etc/hosts"), FileChangeType::DELETED),
        (
            PathBuf::from("/proj/sub/Cargo.toml"),
            FileChangeType::CREATED,
        ),
        // Kind filter: the toml watcher is Create only.
        (
            PathBuf::from("/proj/sub/Cargo.toml"),
            FileChangeType::CHANGED,
        ),
        // `*` does not cross directories.
        (
            PathBuf::from("/proj/sub/deep/Cargo.toml"),
            FileChangeType::CREATED,
        ),
        (PathBuf::from("/proj/gen/a.txt"), FileChangeType::DELETED),
        (PathBuf::from("/proj/gen/a.txt"), FileChangeType::CREATED),
        (PathBuf::from("/proj/gen/c.txt"), FileChangeType::DELETED),
    ];
    let events = watched.events_for(&changes);
    assert_eq!(
        uris(&events, &k),
        vec![
            (
                "file:///proj/src/lib.rs".to_owned(),
                FileChangeType::CHANGED
            ),
            ("file:///etc/hosts".to_owned(), FileChangeType::DELETED),
            (
                "file:///proj/sub/Cargo.toml".to_owned(),
                FileChangeType::CREATED
            ),
            ("file:///proj/gen/a.txt".to_owned(), FileChangeType::DELETED),
        ]
    );

    watched.unregister(&k, "a");
    assert!(watched.events_for(&changes).is_empty());

    watched.register(
        k.clone(),
        "b".into(),
        vec![watcher(GlobPattern::String("*.rs".into()), None)],
    );
    watched.forget_server(&k);
    assert!(watched.events_for(&changes).is_empty());
}

#[test]
fn watched_event_reported_once_across_overlapping_watchers() {
    let k = key("/proj");
    let mut watched = WatchedFiles::default();
    watched.register(
        k.clone(),
        "a".into(),
        vec![
            watcher(GlobPattern::String("**/*.rs".into()), None),
            watcher(GlobPattern::String("src/*.rs".into()), None),
        ],
    );
    let events = watched.events_for(&[(PathBuf::from("/proj/src/a.rs"), FileChangeType::CHANGED)]);
    assert_eq!(events[&k].len(), 1);
}

fn save_caps(save: Option<lsp_types::TextDocumentSyncSaveOptions>) -> ServerCapabilities {
    sync_caps(TextDocumentSyncCapability::Options(
        TextDocumentSyncOptions {
            change: Some(TextDocumentSyncKind::INCREMENTAL),
            save,
            ..Default::default()
        },
    ))
}

#[test]
fn incremental_change_ranges_utf16() {
    let fake = FakeServer::new().capabilities(save_caps(None));
    let servers = started(&fake);
    let mut sync = DocumentSync::default();
    let path = Path::new(FILE);
    // "aé\nb": é is 2 bytes (offsets 1..3), the newline is at 3, b at 4.
    sync.open(path, "rust", &snap("aé\nb"), &servers);
    let patch = Patch::new(vec![
        PatchEdit {
            old: 3..3,
            new: 3..4,
        },
        PatchEdit {
            old: 4..5,
            new: 5..5,
        },
    ]);
    sync.change(path, &snap("aéX\n"), Some(&patch), &servers);
    let got = notes(&fake);
    assert_eq!(got[1].1["textDocument"]["version"], 1);
    assert_eq!(
        got[1].1["contentChanges"],
        json!([
            { "range": { "start": { "line": 1, "character": 0 },
                         "end": { "line": 1, "character": 1 } }, "text": "" },
            { "range": { "start": { "line": 0, "character": 2 },
                         "end": { "line": 0, "character": 2 } }, "text": "X" },
        ])
    );
}

#[test]
fn full_server_gets_full_text() {
    let fake = FakeServer::new();
    let servers = started(&fake);
    let mut sync = DocumentSync::default();
    let path = Path::new(FILE);
    sync.open(path, "rust", &snap("ab"), &servers);
    let patch = Patch::new(vec![PatchEdit {
        old: 1..1,
        new: 1..2,
    }]);
    sync.change(path, &snap("aXb"), Some(&patch), &servers);
    assert_eq!(
        notes(&fake)[1].1["contentChanges"],
        json!([{ "text": "aXb" }])
    );
}

#[test]
fn did_save_respects_include_text() {
    use lsp_types::{SaveOptions, TextDocumentSyncSaveOptions as Save};
    let path = Path::new(FILE);
    let saved = |save: Option<Save>| {
        let fake = FakeServer::new().capabilities(save_caps(save));
        let servers = started(&fake);
        let mut sync = DocumentSync::default();
        sync.open(path, "rust", &snap("a"), &servers);
        sync.did_save(path, &snap("b"), &servers);
        notes(&fake)
            .into_iter()
            .filter(|(m, _)| m == "textDocument/didSave")
            .map(|(_, p)| p)
            .collect::<Vec<_>>()
    };
    assert!(saved(None).is_empty());
    assert!(saved(Some(Save::Supported(false))).is_empty());
    let got = saved(Some(Save::Supported(true)));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0]["textDocument"]["uri"], "file:///proj/a.rs");
    assert!(got[0].get("text").is_none());
    let got = saved(Some(Save::SaveOptions(SaveOptions {
        include_text: Some(true),
    })));
    assert_eq!(got[0]["text"], "b");
    let got = saved(Some(Save::SaveOptions(SaveOptions {
        include_text: Some(false),
    })));
    assert!(got[0].get("text").is_none());
}
