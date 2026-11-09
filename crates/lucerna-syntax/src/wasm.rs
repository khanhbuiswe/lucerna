//! WASM grammars: one shared engine, a process-wide grammar cache, and the
//! validation helpers an installer uses before accepting a grammar.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use tree_sitter::{Query, WasmStore, wasmtime};

use crate::registry::{GrammarError, GrammarKey};

/// The wasmtime engine every grammar and parser store shares.
pub fn engine() -> &'static wasmtime::Engine {
    static ENGINE: OnceLock<wasmtime::Engine> = OnceLock::new();
    ENGINE.get_or_init(wasmtime::Engine::default)
}

/// A fresh store for one parser. Languages loaded by another store of the
/// same engine work in it.
pub(crate) fn new_store() -> Result<WasmStore, GrammarError> {
    WasmStore::new(engine()).map_err(|e| GrammarError(format!("wasm store: {e}")))
}

struct Cache {
    languages: HashMap<GrammarKey, Result<tree_sitter::Language, GrammarError>>,
    /// The stores that loaded the languages, kept alive with them.
    stores: Vec<WasmStore>,
}

/// Loads (once per process) the grammar at `path`. A failure is cached too,
/// so a broken grammar is not re-read on every document.
pub(crate) fn load_wasm_grammar(
    key: &GrammarKey,
    path: &Path,
) -> Result<tree_sitter::Language, GrammarError> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| {
        Mutex::new(Cache {
            languages: HashMap::new(),
            stores: Vec::new(),
        })
    });
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(cached) = cache.languages.get(key) {
        return cached.clone();
    }
    let loaded = load(&key.name, path);
    let result = match loaded {
        Ok((store, language)) => {
            cache.stores.push(store);
            Ok(language)
        }
        Err(e) => Err(e),
    };
    cache.languages.insert(key.clone(), result.clone());
    result
}

fn load(name: &str, path: &Path) -> Result<(WasmStore, tree_sitter::Language), GrammarError> {
    let bytes = std::fs::read(path)
        .map_err(|e| GrammarError(format!("reading {}: {e}", path.display())))?;
    let mut store = new_store()?;
    let language = store
        .load_language(name, &bytes)
        .map_err(|e| GrammarError(format!("loading grammar {name}: {e}")))?;
    Ok((store, language))
}

/// Checks that `bytes` is a loadable tree-sitter grammar named `name`.
pub fn validate_wasm_grammar(name: &str, bytes: &[u8]) -> Result<(), String> {
    load_bytes(name, bytes).map(|_| ())
}

/// Checks that `source` compiles against the WASM grammar `wasm`.
pub fn validate_query(name: &str, wasm: &[u8], source: &str) -> Result<(), String> {
    let (_store, language) = load_bytes(name, wasm)?;
    Query::new(&language, source)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn load_bytes(name: &str, bytes: &[u8]) -> Result<(WasmStore, tree_sitter::Language), String> {
    let mut store = new_store().map_err(|e| e.0)?;
    let language = store
        .load_language(name, bytes)
        .map_err(|e| format!("loading grammar {name}: {e}"))?;
    Ok((store, language))
}
