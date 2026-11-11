//! Talks to a real `rust-analyzer` from `PATH`. Ignored by default:
//! `cargo test -p lucerna-lsp -- --ignored rust_analyzer`.

use std::fs;
use std::time::{Duration, Instant};

use futures::executor::block_on;
use lsp_types::notification::DidOpenTextDocument;
use lsp_types::request::{GotoDefinition, HoverRequest};
use lsp_types::{
    DidOpenTextDocumentParams, GotoDefinitionParams, GotoDefinitionResponse, HoverContents,
    HoverParams, Position, TextDocumentIdentifier, TextDocumentItem, TextDocumentPositionParams,
};
use lucerna_lsp::position::path_to_uri;
use lucerna_lsp::transport::LaunchSpec;
use lucerna_lsp::{
    LanguageServer, Launcher, LspError, ProcessLauncher, ServerEvent, initialize_params,
};

const SOURCE: &str = "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\npub fn total() -> i32 {\n    add(1, 2)\n}\n";
const INDEXING_WAIT: Duration = Duration::from_secs(60);
const RESULT_WAIT: Duration = Duration::from_secs(15);

#[test]
#[ignore = "needs a working rust-analyzer on PATH"]
fn rust_analyzer_definition_and_hover() {
    let launcher = ProcessLauncher::new();
    let Some(program) = launcher.which("rust-analyzer") else {
        eprintln!("skipped: rust-analyzer not found on PATH");
        return;
    };
    if !launcher.probe(&program, &["--version"]) {
        eprintln!("skipped: `rust-analyzer --version` fails (component not installed?)");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::create_dir(root.join("src")).unwrap();
    let file = root.join("src/lib.rs");
    fs::write(&file, SOURCE).unwrap();

    let spec = LaunchSpec {
        server: "rust-analyzer".into(),
        program,
        args: Vec::new(),
        env: launcher.env(),
        cwd: root.clone(),
    };
    let (server, mut events) = LanguageServer::start(
        "rust-analyzer",
        |inbox| launcher.launch(&spec, inbox),
        serde_json::json!({}),
    )
    .unwrap();
    block_on(server.initialize(initialize_params(&root, None))).unwrap();

    let uri = path_to_uri(&file);
    server.notify::<DidOpenTextDocument>(DidOpenTextDocumentParams {
        text_document: TextDocumentItem {
            uri: uri.clone(),
            language_id: "rust".into(),
            version: 1,
            text: SOURCE.into(),
        },
    });
    wait_for_indexing(&mut events);

    // `add` in `add(1, 2)`, on the sixth line.
    let call = Position::new(5, 5);
    let position = TextDocumentPositionParams {
        text_document: TextDocumentIdentifier { uri: uri.clone() },
        position: call,
    };

    let definition = retry(|| {
        let response = block_on(server.request::<GotoDefinition>(GotoDefinitionParams {
            text_document_position_params: position.clone(),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        }))?;
        Ok(definition_line(response, &uri))
    });
    assert_eq!(definition, Some(0), "definition resolves to `fn add`");

    let hover = retry(|| {
        let hover = block_on(server.request::<HoverRequest>(HoverParams {
            text_document_position_params: position.clone(),
            work_done_progress_params: Default::default(),
        }))?;
        Ok(hover.map(|h| hover_text(h.contents)))
    });
    let hover = hover.expect("hover answered");
    assert!(
        hover.contains("fn add(a: i32, b: i32) -> i32"),
        "hover shows the signature, got: {hover}"
    );

    let _ = server.shutdown().join();
}

/// Waits for the end of rust-analyzer's indexing, or 60 s.
fn wait_for_indexing(events: &mut futures::channel::mpsc::UnboundedReceiver<ServerEvent>) {
    let deadline = Instant::now() + INDEXING_WAIT;
    while Instant::now() < deadline {
        match events.try_recv() {
            Ok(ServerEvent::Progress {
                token, done: true, ..
            }) if token.contains("Indexing") || token.contains("cachePriming") => {
                return;
            }
            Ok(_) => {}
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    eprintln!("indexing did not report an end within 60 s; trying anyway");
}

/// Re-asks until the answer is `Some`, since a server may answer `null`
/// while it is still loading.
fn retry<T>(mut ask: impl FnMut() -> Result<Option<T>, LspError>) -> Option<T> {
    let deadline = Instant::now() + RESULT_WAIT;
    loop {
        match ask() {
            Ok(Some(value)) => return Some(value),
            Ok(None) | Err(LspError::Response { .. }) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(250));
            }
            Ok(None) => return None,
            Err(err) => panic!("request failed: {err}"),
        }
    }
}

/// The 0-based start line of the first target inside `uri`.
fn definition_line(response: Option<GotoDefinitionResponse>, uri: &lsp_types::Uri) -> Option<u32> {
    let (target_uri, line) = match response? {
        GotoDefinitionResponse::Scalar(l) => (l.uri, l.range.start.line),
        GotoDefinitionResponse::Array(ls) => {
            let l = ls.into_iter().next()?;
            (l.uri, l.range.start.line)
        }
        GotoDefinitionResponse::Link(ls) => {
            let l = ls.into_iter().next()?;
            (l.target_uri, l.target_selection_range.start.line)
        }
    };
    (target_uri == *uri).then_some(line)
}

fn hover_text(contents: HoverContents) -> String {
    match contents {
        HoverContents::Markup(m) => m.value,
        HoverContents::Scalar(s) => marked(s),
        HoverContents::Array(items) => items.into_iter().map(marked).collect::<Vec<_>>().join("\n"),
    }
}

fn marked(s: lsp_types::MarkedString) -> String {
    match s {
        lsp_types::MarkedString::String(s) => s,
        lsp_types::MarkedString::LanguageString(l) => l.value,
    }
}
