//! Adapts the syntax crate's WASM checks to the extension store.

use lucerna_extension::validate::GrammarValidator;

/// Validates grammars and queries by loading them with the real runtime.
pub struct WasmGrammarValidator;

impl GrammarValidator for WasmGrammarValidator {
    fn load(&self, name: &str, wasm: &[u8]) -> Result<(), String> {
        lucerna_syntax::validate_wasm_grammar(name, wasm)
    }

    fn check_query(&self, name: &str, wasm: &[u8], kind: &str, source: &str) -> Result<(), String> {
        lucerna_syntax::validate_query(name, wasm, source).map_err(|e| format!("{kind}.scm: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON_WASM: &[u8] =
        include_bytes!("../../../lucerna-syntax/tests/fixtures/wasm/json.wasm");

    #[test]
    fn loads_a_real_grammar() {
        WasmGrammarValidator.load("json", JSON_WASM).unwrap();
    }

    #[test]
    fn rejects_garbage_bytes() {
        assert!(WasmGrammarValidator.load("json", b"nope").is_err());
    }

    #[test]
    fn checks_queries_and_names_the_kind() {
        let v = WasmGrammarValidator;
        v.check_query("json", JSON_WASM, "highlights", "(string) @string")
            .unwrap();
        let err = v
            .check_query("json", JSON_WASM, "highlights", "(no_such_node) @x")
            .unwrap_err();
        assert!(err.starts_with("highlights.scm: "), "{err}");
    }
}
