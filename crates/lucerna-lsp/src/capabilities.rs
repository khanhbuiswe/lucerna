//! What Lucerna tells a server in `initialize`: read-only navigation and
//! diagnostics features only (no completion, edits or formatting).

use std::path::Path;

use lsp_types::{
    ClientCapabilities, ClientInfo, DiagnosticTag, DidChangeWatchedFilesClientCapabilities,
    DocumentSymbolClientCapabilities, DynamicRegistrationClientCapabilities,
    GeneralClientCapabilities, GotoCapability, HoverClientCapabilities, InitializeParams,
    MarkupKind, PositionEncodingKind, PublishDiagnosticsClientCapabilities, TagSupport,
    TextDocumentClientCapabilities, TextDocumentSyncClientCapabilities, WindowClientCapabilities,
    WorkspaceClientCapabilities, WorkspaceFolder, WorkspaceSymbolClientCapabilities,
};
use serde_json::Value;

use crate::position::path_to_uri;

/// `initialize` params for a server rooted at `root` (absolute).
pub fn initialize_params(root: &Path, initialization_options: Option<Value>) -> InitializeParams {
    let root_uri = path_to_uri(root);
    let folder = WorkspaceFolder {
        uri: root_uri.clone(),
        name: root.file_name().map_or_else(
            || root.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        ),
    };
    #[allow(deprecated)] // `root_uri` is still read by many servers.
    InitializeParams {
        process_id: Some(std::process::id()),
        root_uri: Some(root_uri),
        workspace_folders: Some(vec![folder]),
        client_info: Some(ClientInfo {
            name: "Lucerna".into(),
            version: Some(env!("CARGO_PKG_VERSION").into()),
        }),
        initialization_options,
        capabilities: client_capabilities(),
        ..Default::default()
    }
}

fn client_capabilities() -> ClientCapabilities {
    let goto = || {
        Some(GotoCapability {
            dynamic_registration: None,
            link_support: Some(true),
        })
    };
    ClientCapabilities {
        general: Some(GeneralClientCapabilities {
            position_encodings: Some(vec![
                PositionEncodingKind::UTF8,
                PositionEncodingKind::UTF16,
            ]),
            ..Default::default()
        }),
        text_document: Some(TextDocumentClientCapabilities {
            synchronization: Some(TextDocumentSyncClientCapabilities::default()),
            definition: goto(),
            type_definition: goto(),
            implementation: goto(),
            declaration: goto(),
            references: Some(DynamicRegistrationClientCapabilities::default()),
            hover: Some(HoverClientCapabilities {
                dynamic_registration: None,
                content_format: Some(vec![MarkupKind::Markdown, MarkupKind::PlainText]),
            }),
            document_symbol: Some(DocumentSymbolClientCapabilities {
                hierarchical_document_symbol_support: Some(true),
                ..Default::default()
            }),
            publish_diagnostics: Some(PublishDiagnosticsClientCapabilities {
                related_information: Some(true),
                version_support: Some(true),
                tag_support: Some(TagSupport {
                    value_set: vec![DiagnosticTag::UNNECESSARY, DiagnosticTag::DEPRECATED],
                }),
                code_description_support: Some(true),
                data_support: None,
            }),
            ..Default::default()
        }),
        workspace: Some(WorkspaceClientCapabilities {
            symbol: Some(WorkspaceSymbolClientCapabilities::default()),
            configuration: Some(true),
            workspace_folders: Some(true),
            did_change_watched_files: Some(DidChangeWatchedFilesClientCapabilities {
                dynamic_registration: Some(true),
                relative_pattern_support: Some(true),
            }),
            ..Default::default()
        }),
        window: Some(WindowClientCapabilities {
            work_done_progress: Some(true),
            show_message: Some(Default::default()),
            show_document: None,
        }),
        ..Default::default()
    }
}
