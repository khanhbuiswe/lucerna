//! Requests the editor sends to the language servers of a document.

use std::ops::Range;
use std::path::Path;
use std::time::Duration;

use gpui::{AppContext, Context, Task};
use lsp_types::request::HoverRequest;
use lsp_types::{
    HoverContents, HoverParams, HoverProviderCapability, MarkedString, MarkupKind,
    TextDocumentIdentifier, TextDocumentPositionParams,
};
use lucerna_lsp::position::{PositionEncoding, path_to_uri, range_from_lsp, to_lsp};
use lucerna_text::{BufferSnapshot, Point};

use super::LanguageServers;
use crate::workspace_state::absolute_path;

/// `textDocument/hover` gives up after this.
const HOVER_TIMEOUT: Duration = Duration::from_secs(10);

/// A hover answer, as markdown.
#[derive(Debug, Clone, PartialEq)]
pub struct HoverInfo {
    pub markdown: String,
    /// The range the answer is about, in the document's text.
    pub range: Option<Range<Point>>,
    /// The document's language id: the language of code blocks that name
    /// none.
    pub language: Option<String>,
}

impl LanguageServers {
    /// Asks the first running server of `path` that has `hoverProvider`
    /// about `point`, with a 10 s timeout. Resolves to `None` when no
    /// server can answer, on errors and for empty answers. Dropping the
    /// task cancels the request (`$/cancelRequest`).
    pub fn hover(
        &self,
        path: &Path,
        point: Point,
        cx: &mut Context<Self>,
    ) -> Task<Option<HoverInfo>> {
        let path = absolute_path(&self.root, path);
        let Some(doc) = self.docs.get(path.as_path()) else {
            return Task::ready(None);
        };
        let server = self.running_for(&path).into_iter().find_map(|(_, server)| {
            let caps = server.capabilities()?;
            match caps.hover_provider.as_ref()? {
                HoverProviderCapability::Simple(false) => None,
                _ => Some(server),
            }
        });
        let Some(server) = server else {
            return Task::ready(None);
        };
        let snapshot = doc.snapshot.clone();
        let language = doc.language_id.as_deref().map(str::to_owned);
        let encoding = server.encoding();
        let params = HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: path_to_uri(&path),
                },
                position: to_lsp(&snapshot, point, encoding),
            },
            work_done_progress_params: Default::default(),
        };
        let request = server.request_with_timeout::<HoverRequest>(params, HOVER_TIMEOUT);
        cx.background_spawn(async move {
            let hover = match request.await {
                Ok(hover) => hover?,
                Err(err) => {
                    tracing::debug!(%err, "hover failed");
                    return None;
                }
            };
            crate::tasks::guarded("hover", || hover_info(hover, &snapshot, encoding, language))
                .flatten()
        })
    }
}

/// The markdown of `hover`; `None` when it says nothing.
fn hover_info(
    hover: lsp_types::Hover,
    snapshot: &BufferSnapshot,
    encoding: PositionEncoding,
    language: Option<String>,
) -> Option<HoverInfo> {
    let markdown = match hover.contents {
        HoverContents::Markup(markup) => match markup.kind {
            MarkupKind::Markdown => markup.value,
            MarkupKind::PlainText => escape_markdown(&markup.value),
        },
        HoverContents::Scalar(marked) => marked_string(marked),
        HoverContents::Array(items) => items
            .into_iter()
            .map(marked_string)
            .filter(|s| !s.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
    };
    if markdown.trim().is_empty() {
        return None;
    }
    Some(HoverInfo {
        markdown,
        range: hover.range.map(|r| range_from_lsp(snapshot, r, encoding)),
        language,
    })
}

/// A legacy `MarkedString`: a plain string is markdown, a language string
/// a code block in that language.
fn marked_string(marked: MarkedString) -> String {
    match marked {
        MarkedString::String(s) => s,
        MarkedString::LanguageString(ls) => {
            // A fence longer than any backtick run in the code.
            let longest = ls
                .value
                .split(|c| c != '`')
                .map(str::len)
                .max()
                .unwrap_or(0);
            let fence = "`".repeat(longest.max(2) + 1);
            format!("{fence}{}\n{}\n{fence}", ls.language, ls.value)
        }
    }
}

/// Plain text as markdown showing it verbatim: punctuation escaped, line
/// breaks kept.
fn escape_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 8);
    for line in text.lines() {
        if !out.is_empty() {
            // A backslash at the end of a line is a hard break.
            out.push_str("\\\n");
        }
        for c in line.chars() {
            if c.is_ascii_punctuation() {
                out.push('\\');
            }
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_strings_become_fenced_code() {
        let ls = MarkedString::LanguageString(lsp_types::LanguageString {
            language: "rust".into(),
            value: "let s = \"```\";".into(),
        });
        assert_eq!(marked_string(ls), "````rust\nlet s = \"```\";\n````");
        assert_eq!(marked_string(MarkedString::String("*a*".into())), "*a*");
    }

    #[test]
    fn plain_text_is_escaped() {
        assert_eq!(escape_markdown("a *b*\n# c"), "a \\*b\\*\\\n\\# c");
    }
}
