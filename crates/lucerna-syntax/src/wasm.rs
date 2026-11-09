//! WASM grammars: one shared engine, a process-wide grammar cache, and the
//! validation helpers an installer uses before accepting a grammar.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use tree_sitter::{Query, WasmStore, wasmtime};

use crate::registry::{GrammarError, GrammarKey};

/// The wasmtime engine every grammar and parser store shares.
pub fn engine() -> &'static wasmtime::Engine {
    static ENGINE: OnceLock<wasmtime::Engine> = OnceLock::new();
    ENGINE.get_or_init(wasmtime::Engine::default)
}

/// Most idle stores kept for reuse.
const POOL_CAP: usize = 8;

fn pool() -> &'static Mutex<Vec<WasmStore>> {
    static POOL: OnceLock<Mutex<Vec<WasmStore>>> = OnceLock::new();
    POOL.get_or_init(|| Mutex::new(Vec::new()))
}

/// A store for one parser: a pooled one (already holding the grammars it
/// ran before) or a fresh one. Languages loaded by another store of the
/// same engine work in it. Give it back with [`return_store`].
pub(crate) fn take_store() -> Result<WasmStore, GrammarError> {
    if let Some(store) = pool().lock().unwrap_or_else(|e| e.into_inner()).pop() {
        return Ok(store);
    }
    new_store()
}

/// Keeps `store` for reuse, or drops it when the pool is full.
pub(crate) fn return_store(store: WasmStore) {
    let mut pool = pool().lock().unwrap_or_else(|e| e.into_inner());
    if pool.len() < POOL_CAP {
        pool.push(store);
    }
}

fn new_store() -> Result<WasmStore, GrammarError> {
    WasmStore::new(engine()).map_err(|e| GrammarError(format!("wasm store: {e}")))
}

/// A grammar file as it was when loaded: an extension updated in place
/// changes the stamp and so loads afresh.
#[derive(Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    grammar: GrammarKey,
    path: PathBuf,
    stamp: Option<(SystemTime, u64)>,
}

impl CacheKey {
    fn new(grammar: &GrammarKey, path: &Path) -> CacheKey {
        let stamp = std::fs::metadata(path)
            .ok()
            .and_then(|m| Some((m.modified().ok()?, m.len())));
        CacheKey {
            grammar: grammar.clone(),
            path: path.to_path_buf(),
            stamp,
        }
    }
}

type Slot = Arc<OnceLock<Result<tree_sitter::Language, GrammarError>>>;

struct Cache {
    slots: HashMap<CacheKey, Slot>,
    /// The stores that loaded the languages, kept alive with them.
    stores: Vec<WasmStore>,
}

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(Cache {
            slots: HashMap::new(),
            stores: Vec::new(),
        })
    })
}

fn lock() -> std::sync::MutexGuard<'static, Cache> {
    cache().lock().unwrap_or_else(|e| e.into_inner())
}

/// How many grammar files the process-wide cache holds (loaded, loading or
/// failed), for tests that check a grammar was not loaded.
#[doc(hidden)]
pub fn wasm_cache_len() -> usize {
    lock().slots.len()
}

/// Whether [`load_wasm_grammar`] would answer without reading the file.
pub(crate) fn is_loaded(key: &GrammarKey, path: &Path) -> bool {
    let key = CacheKey::new(key, path);
    lock().slots.get(&key).is_some_and(|s| s.get().is_some())
}

/// Loads (once per file version and process) the grammar at `path`. A
/// failure is cached too, so a broken grammar is not re-read on every
/// document. The cache lock is not held while compiling.
pub(crate) fn load_wasm_grammar(
    key: &GrammarKey,
    path: &Path,
) -> Result<tree_sitter::Language, GrammarError> {
    let cache_key = CacheKey::new(key, path);
    let slot = lock().slots.entry(cache_key).or_default().clone();
    slot.get_or_init(|| {
        let (store, language) = load(&key.name, path)?;
        lock().stores.push(store);
        Ok(language)
    })
    .clone()
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
