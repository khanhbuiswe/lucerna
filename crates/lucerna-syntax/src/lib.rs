//! Bundled tree-sitter languages, viewport highlights, indent folds and
//! bracket matching for Lucerna's read-only code view.

mod associations;
mod brackets;
mod editing;
mod folds;
mod injection;
mod outline;
mod registry;
mod symbols;
mod syntax_state;
mod wasm;

use lucerna_text::{BufferSnapshot, Point};

pub use associations::Associations;
pub use brackets::matching_bracket;
pub use editing::{BracketPair, LanguageEditing};
pub use folds::indent_folds;
pub use registry::{
    ExtensionLanguage, GrammarError, GrammarKey, Language, LanguageOrigin, LanguageRegistry,
};
pub use symbols::{FlatSymbol, Symbol, SymbolKind, SymbolTree};
pub use syntax_state::{HighlightSpan, ParseOutcome, SYNC_PARSE_BUDGET, SyntaxState};

/// The byte offset where `row` starts; rows past the end give the length.
pub(crate) fn row_start(snapshot: &BufferSnapshot, row: u32) -> usize {
    if row >= snapshot.line_count() {
        snapshot.len_bytes()
    } else {
        snapshot.point_to_offset(Point { row, column: 0 })
    }
}
pub use wasm::{engine, retain_grammars, validate_query, validate_wasm_grammar, wasm_cache_len};
