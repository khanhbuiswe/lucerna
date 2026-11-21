# Lucerna Phase 2 (Zed Extension Host) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Install Zed extensions from the `zed-industries/extensions` repo, building them locally, and use their themes, icon themes, languages (WASM tree-sitter grammars, queries, injections) and snippets (validated only) without restarting Lucerna.

**Architecture:**
- A new pure-Rust crate, `lucerna-extension`, fetches and builds extensions with the system `git`, `curl`, `tar` and wasi-sdk's clang. It writes `installed/<id>/` directories and an `index.json`, and returns **data** only.
- `lucerna-syntax` gains WASM grammars (tree-sitter `wasm` feature on wasmtime), layered language lookup, `files.associations`, and general injections.
- `lucerna-theme` gains layered theme loading and Zed icon themes.
- `lucerna-workbench` turns the index into registries, swaps them at runtime, and adds the Extensions panel. `luc ext …` exposes the same store from the CLI.

**Tech Stack:**
- Rust, edition 2024.
- `tree-sitter` 0.27 with feature `wasm` (pulls `wasmtime-c-api-impl` 48).
- New dependencies: `toml` (manifests), `globset` (associations), `libc` (`statvfs`).
- Existing dependencies: `sha2`, `tempfile`, `notify`, `nucleo-matcher`, `gpui`.
- System tools at run time: `git`, `curl`, `tar`, and wasi-sdk 34.

**Spec:** `docs/superpowers/specs/2026-11-09-lucerna-phase2-extensions-design.md`. Background research: `docs/research/2026-11-09-extensions.md`. Read both before starting a task.

## Global Constraints

- **Licensing:**
  - Lucerna is licensed MIT OR Apache-2.0.
  - **No GPL code from Zed.** The Zed crates `extension`, `extension_host`, `language`, `theme` and `file_icons` may be read to understand data formats, but never copied.
  - `deny.toml` additionally allows `Apache-2.0 WITH LLVM-exception`. No other license changes.
- **Phase 1 rulings still hold:**
  - R3: panics in background work are caught with `catch_unwind` and logged; the UI keeps running.
  - R21: a single worktree event pump. The extensions watcher is separate because it watches a directory outside the worktree.
  - R26: object settings merge across layers.
- **Crate boundaries:**
  - Only `lucerna-workbench` depends on `gpui`.
  - `lucerna-extension` depends on neither `lucerna-syntax` nor `lucerna-theme`.
- **Paths** (base: `~/Library/Application Support/Lucerna/extensions/`):
  - `registry/`, `registry.json`
  - `toolchain/wasi-sdk-34/`
  - `work/<id>/src/`, `work/<id>/grammars/<g>/`
  - `staging/<id>-<random>/`
  - `installed/<id>/`
  - `index.json`
  - `logs/<id>.log`
  - Tests use a temporary directory passed as the store root, never the real one.
- **Registry repository:** `https://github.com/zed-industries/extensions`, branch `main`.
- **Pinned toolchain:**
  - wasi-sdk **34**.
  - URL: `https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-34/wasi-sdk-34.0-<arm64|x86_64>-macos.tar.gz`.
  - Its `VERSION` file reads `34.0`.
  - SHA-256 constants are recorded in Task 1.
  - `WASI_SDK_PATH` overrides the toolchain when `$WASI_SDK_PATH/bin/clang` exists.
- **Grammar compile command,** exactly:

  ```
  <wasi-sdk>/bin/clang -fPIC -shared -Os -Wl,--export=tree_sitter_<name> -Wl,--max-memory=4294967296 -o <out>/<name>.wasm -I <src> <src>/parser.c [<src>/scanner.c]
  ```
- **Limits:**
  - Each `git` or `curl` call has a 120 s timeout.
  - Each grammar compile has a 300 s timeout.
  - Free disk must be at least 1.2 GB (1_288_490_189 bytes) before a fetch or build.
  - Registry refresh interval: 24 h.
  - Watcher debounce on `installed/`: 200 ms.
  - Injection depth: at most 4.
  - At most 1000 child layers per document.
  - `first_line_pattern` is matched against at most the first 256 bytes.
- **Every `git` call** runs with `GIT_TERMINAL_PROMPT=0` and `GIT_ASKPASS=` (empty), and with `-c protocol.file.allow=always`, so the `file://` fixtures used in tests work.
- **Accepted manifests:** `schema_version` ≤ 1. Grammar names must match `^[a-z0-9_]+$`.
- **Query files recognised:** `highlights`, `injections`, `brackets`, `outline`, `indents`, `textobjects`, `overrides`, `redactions`, `runnables`, `debugger` (each with `.scm`).
- **Settings keys:**

  | Key | Default |
  |---|---|
  | `workbench.iconTheme` | `"Lucide"` |
  | `files.associations` | `{}` |
  | `extensions.autoCheckUpdates` | `true` |

- **Commands:**
  - `workbench.view.extensions` (Cmd+Shift+X)
  - `workbench.extensions.action.installExtension`
  - `workbench.extensions.action.checkForUpdates`
  - `workbench.extensions.action.updateAllExtensions`
  - `lucerna.extensions.clearBuildCache`
  - `workbench.action.selectIconTheme`
  - New context key: `extensionsViewletFocus`.
- **Development constraints:**
  - Disk is tight. Never run `cargo build --release`.
  - Before each build, check that `df -g .` shows at least 2 GB free.
  - Cargo needs `PATH="/opt/homebrew/opt/rustup/bin:$PATH"`.
  - Commit on `main`, and stage only the files you changed.
- **Before every commit:** run `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace`. All three must be green.

## Review Focus

1. **A git host asks for credentials.** If a submodule URL is private, gone, or uses SSH (`git@github.com:o/r`), the install must fail within the timeout with a readable error, and must never hang waiting on a password prompt. SSH-form GitHub URLs are rewritten to `https://github.com/o/r`. Test owner: Task 8.
2. **The CLI and the app touch the same id at once.** `luc ext install x` while the window is installing `x` must not interleave writes. A per-id lock file (`flock` on `extensions/locks/<id>.lock`) makes the second caller fail with `Busy`. Test owner: Task 10.
3. **Uninstalling an extension whose language is open in a tab.** The tab falls back to bundled or plain text with no crash, and the already-loaded grammar is dropped only when no state uses it. Test owner: Task 13.
4. **A grammar fails to load or traps at parse.** The tab must still show its text. It must not stay in `Loading` or render blank. Test owner: Task 2 (syntax) and Task 13 (workbench).
5. **Case in file names.** `Dockerfile` vs `dockerfile` in icon-theme `file_stems`, and `.MD` vs `md` in `path_suffixes`. Lookups try the exact name first, then the ASCII-lowercased one. Test owners: Task 3 (languages) and Task 6 (icons).

---

### Task 1: Spike: WASM grammars on tree-sitter 0.27, license gate, fixture grammar

This task proves the riskiest assumption in the spec (§11). **If any step fails, stop and report. Do not work around it.**

**Files:**
- Modify: `Cargo.toml` (workspace: `tree-sitter = { version = "0.27", features = ["wasm"] }`; add `toml = "0.8"`, `globset = "0.4"`, `libc = "0.2"`)
- Modify: `deny.toml` (allow `Apache-2.0 WITH LLVM-exception`)
- Create: `crates/lucerna-syntax/tests/fixtures/wasm/json.wasm` (built from the `tree-sitter-json` crate's own `src/parser.c` at the version in `Cargo.lock`)
- Create: `crates/lucerna-syntax/tests/fixtures/wasm/README.md` (the exact build command, the grammar version, and the SHA-256 of both wasi-sdk 34 macOS tarballs)
- Create: `crates/lucerna-syntax/tests/fixtures/wasm/LICENSE-tree-sitter-json` (copied from the crate)
- Test: `crates/lucerna-syntax/tests/wasm_spike.rs`

**Interfaces:**
- Produces:
  - The fixture `json.wasm`, which exports `tree_sitter_json`.
  - The two SHA-256 constants that Task 9 pastes into `toolchain.rs`.

- [ ] **Step 1: Download wasi-sdk 34** for this machine (arm64) into `~/Library/Application Support/Lucerna/extensions/toolchain/` and extract it there. Record the arm64 tarball's `shasum -a 256`. Read the x86_64 digest from `gh api repos/WebAssembly/wasi-sdk/releases/tags/wasi-sdk-34` (the asset `digest` field). If that field is missing, download the x86_64 tarball to the scratchpad, hash it, and delete it.
- [ ] **Step 2: Build the fixture** with the Global Constraints command, using `name = json` and the `tree-sitter-json` crate's `src/`. Write the command, the crate version and both hashes to `README.md`.
- [ ] **Step 3: Write the tests** in `wasm_spike.rs`:
  - `wasm_json_loads_and_parses`:
    - `WasmStore::new(&tree_sitter::wasmtime::Engine::default())`, then `load_language("json", bytes)`.
    - A `Parser` with `set_wasm_store` and `set_language` parses `{"a": [1, true]}`.
    - The root node kind is `document` and `!has_error()`.
  - `wasm_language_shared_across_stores`: a language loaded in store A parses correctly in a parser that owns store B, created from the same `Engine`.
  - `wasm_matches_native_highlights`: running `tree_sitter_json::HIGHLIGHTS_QUERY` on the WASM tree gives the same `(byte_range, capture_name)` list as the native grammar for `tests/fixtures/sample.json`.
  - `wasm_load_garbage_errors`: `load_language("json", b"not wasm")` returns `Err`. It must not panic.
- [ ] **Step 4: Run** `cargo test -p lucerna-syntax --test wasm_spike`. Expected: PASS. Then run `cargo deny check licenses`. Expected: ok.
  - If `wasm_language_shared_across_stores` fails, stop and report. Task 2's design depends on it.
- [ ] **Step 5: Measure.** Record in `README.md` how much the `luc` debug binary grew and how much `target/` grew (`du -sh target`).
- [ ] **Step 6: Commit** with message `build(syntax): enable tree-sitter wasm; json.wasm fixture; allow LLVM-exception`.

### Task 2: WASM grammars and owned language definitions (`lucerna-syntax`)

**Files:**
- Modify: `crates/lucerna-syntax/src/registry.rs` (owned fields, `GrammarSource`)
- Create: `crates/lucerna-syntax/src/wasm.rs` (engine, grammar cache, store helpers, validation functions)
- Modify: `crates/lucerna-syntax/src/syntax_state.rs` (`new` returns `Result`; `highlights` takes a style filter)
- Modify: `crates/lucerna-syntax/src/lib.rs`
- Modify: callers in `crates/lucerna-workbench/src/documents.rs`, `code_view/mod.rs` and `workbench.rs` (the `.id` and `.name` types, `SyntaxState::new` errors, and the `styled` argument)
- Test: `crates/lucerna-syntax/tests/wasm_grammar.rs`. Existing tests are updated for the new signatures.

**Interfaces:**
- Consumes: `json.wasm` (Task 1).
- Produces:
  - `Language` keeps its behaviour, but its fields become owned:
    - `pub id: Arc<str>`, `pub name: Arc<str>`
    - `pub fn grammar(&self) -> Result<tree_sitter::Language, GrammarError>`, loaded lazily and cached.
    - `pub fn origin(&self) -> &LanguageOrigin`, where `pub enum LanguageOrigin { Bundled, Extension { extension_id: Arc<str> } }`.
    - `pub fn is_failed(&self) -> bool`.
    - `pub fn hidden(&self) -> bool`.
  - `pub(crate) enum GrammarSource { Native(tree_sitter::Language), Wasm { key: GrammarKey, path: PathBuf } }`, where `pub struct GrammarKey { pub extension_id: Arc<str>, pub name: Arc<str> }`.
  - `pub struct GrammarError(pub String)`, which implements `Display`.
  - `SyntaxState::new(language: Arc<Language>) -> Result<SyntaxState, GrammarError>`. It attaches a `WasmStore` from `wasm::new_store()` only when the grammar is WASM.
  - `SyntaxState::highlights(&self, snapshot, rows, styled: &dyn Fn(&str) -> bool) -> Vec<HighlightSpan>`. A capture for which `styled(name)` is false is skipped before ranking.
  - In `wasm.rs`:
    - `pub fn engine() -> &'static tree_sitter::wasmtime::Engine`, a lazily created `OnceLock`.
    - `pub(crate) fn new_store() -> Result<WasmStore, GrammarError>`.
    - `pub(crate) fn load_wasm_grammar(key: &GrammarKey, path: &Path) -> Result<tree_sitter::Language, GrammarError>`. It is cached in a global `Mutex<HashMap<GrammarKey, Result<tree_sitter::Language, GrammarError>>>`, so a failure is remembered for the rest of the process.
    - `pub fn validate_wasm_grammar(name: &str, bytes: &[u8]) -> Result<(), String>`.
    - `pub fn validate_query(name: &str, wasm: &[u8], source: &str) -> Result<(), String>`. Compiles `source` against the grammar.

  `lib.rs` re-exports `GrammarError`, `LanguageOrigin`, `validate_wasm_grammar`, `validate_query` and `engine`.

- [ ] **Step 1: Write failing tests** in `wasm_grammar.rs`:
  - `extension_language_from_wasm_highlights_json`: a `Language` built through the crate-internal test constructor `Language::from_wasm_for_test(id, path_to_json_wasm, highlights_src)` highlights `{"a": 1}` with a span starting with `"string"` over `"a"` (use `|_| true` as the filter).
  - `missing_wasm_file_is_grammar_error`: `SyntaxState::new` returns `Err`, and `language.is_failed()` is `true` afterwards.
  - `failure_is_cached`: a second `SyntaxState::new` on the same language returns `Err` without reading the file again. Rename the file into place between the two calls; it must still be `Err`.
  - `styled_filter_drops_unstyled_captures`: with `LastPatternWins` and a query `((pair key: (_) @a) (pair key: (_) @b))`, `styled = |c| c == "a"` gives capture `a` on the key, and `|_| true` gives `b`.
  - `validate_wasm_grammar_rejects_garbage` and `validate_query_reports_bad_node`, the latter using the query `(no_such_node) @x`.
- [ ] **Step 2: Run** `cargo test -p lucerna-syntax`. Expected: the new tests FAIL to compile.
- [ ] **Step 3: Implement.**
  - Bundled languages build `GrammarSource::Native`. `bundled()` keeps its table, with `&'static str` converted to `Arc<str>` and `Vec<String>`.
  - `Language::queries()` now compiles against `grammar()?`. A highlights query that fails to compile for a bundled language still panics (a bundled bug). For an extension language it marks the language failed and yields no spans.
  - Workbench callers:
    - Pass `&|c| theme.style_for(c).is_some()` as the filter.
    - When `SyntaxState::new` returns `Err`, show the file as plain text (the existing no-syntax path) and log a `warn`. The document must not stay in `Loading`.
- [ ] **Step 4: Run** `cargo test --workspace`. Expected: PASS, with every Phase 1 snapshot unchanged.
- [ ] **Step 5: Commit** with message `feat(syntax): lazily loaded WASM grammars and owned language definitions`.

### Task 3: Layered language registry, `files.associations`, extension queries (`lucerna-syntax`)

**Files:**
- Modify: `crates/lucerna-syntax/src/registry.rs`
- Create: `crates/lucerna-syntax/src/associations.rs`
- Modify: `crates/lucerna-syntax/src/brackets.rs` (query-driven pairs)
- Modify: `crates/lucerna-syntax/Cargo.toml` (add `globset`, `regex`)
- Test: `crates/lucerna-syntax/tests/registry_layers.rs`, plus extensions to `tests/brackets.rs`

**Interfaces:**
- Consumes: Task 2.
- Produces:
  - The extension language input. `lucerna-syntax` does not depend on `lucerna-extension`; the workbench converts `LanguageEntry` into this struct:

    ```rust
    pub struct ExtensionLanguage {
        pub extension_id: String,
        pub name: String,
        pub dir: PathBuf,                 // holds config.toml and *.scm
        pub grammar: Option<(String, PathBuf)>, // (grammar name, .wasm path)
        pub path_suffixes: Vec<String>,
        pub first_line_pattern: Option<String>,
        pub hidden: bool,
    }
    ```
  - `pub struct Associations`, with `Associations::new(map: &BTreeMap<String, String>) -> Associations`. Invalid globs are skipped with a `warn`.
  - `LanguageRegistry::build(extensions: &[ExtensionLanguage], associations: Associations) -> LanguageRegistry`. Bundled languages are always included. `LanguageRegistry::bundled()` equals `build(&[], Associations::default())`.
  - `language_for_path(&self, path: &Path) -> Option<Arc<Language>>`. Unchanged; it does not use the first line.
  - `language_for_file(&self, path: &Path, first_line: &str) -> Option<Arc<Language>>`. The full lookup order from spec §4.2.
  - `language_by_name(&self, name: &str) -> Option<Arc<Language>>`. Case-insensitive match on name, then id, then path suffix. Takes the part after the last `.`. Includes hidden languages. Task 4 uses it for injections.
  - Language ids: bundled ids are unchanged. For an extension language the id is `name.to_lowercase()` with spaces replaced by `-`. An extension id equal to a bundled id **replaces** the bundled language.
  - `pub(crate) struct LanguageQueries` gains `injections: Option<Query>` and `brackets: Option<Query>`. Both are read from `dir` for extension languages. For bundled languages, `markdown` and `html` get the crate's `INJECTION_QUERY_BLOCK` / `INJECTIONS_QUERY`; no other bundled language gets an injections query.
  - Extension languages use `Precedence::LastPatternWins`.
  - `.scm` files other than `highlights`, `injections` and `brackets` are ignored here.
  - **Predicates** (spec §4.3). The tree-sitter crate already applies text predicates (`#eq?`, `#match?`, `#any-of?` and their `not-` forms). For every match, `highlights` also checks `query.general_predicates(pattern_index)`:
    - `#has-parent? @cap type…` and `#not-has-parent? @cap type…` test the parent node's kind of the captured node.
    - Any other general predicate (except `#set!`, which tree-sitter keeps as a property) makes the match count as **not matched**, and logs one `warn` per language per predicate name.

- [ ] **Step 1: Write failing tests** in `registry_layers.rs`. They use the `json.wasm` fixture and temp directories that hold a `config.toml`-free layout; the language data comes from `ExtensionLanguage` directly.
  - `extension_suffix_beats_bundled`: an extension language with suffix `json` wins for `a.json`.
  - `exact_filename_beats_suffix`: an extension `path_suffixes = ["Cargo.lock"]` wins over the bundled `toml` suffix rule for `Cargo.lock`.
  - `longest_suffix_wins`: suffixes `ts` and `d.ts` in two extensions; `x.d.ts` picks `d.ts`.
  - `tie_breaks_on_extension_id`: ext `a` and ext `b` both claim `foo`; `x.foo` picks `b`.
  - `associations_win_over_everything`: `{"*.json": "markdown"}` makes `a.json` Markdown. A pattern containing `/`, such as `docs/*.txt`, matches the relative path. The longer glob wins.
  - `association_by_extension_language_name`: `{"*.zz": "Zed Json"}` resolves to an extension language named `Zed Json`.
  - `first_line_pattern_only_when_nothing_else`: `^#!.*\bzz\b` matches a file with no extension. A file with a known suffix ignores the pattern.
  - `hidden_language_not_matched_by_path_but_by_name`.
  - `suffix_matching_is_case_insensitive`: `README.MD` is Markdown, and `X.JSON` is JSON.
  - `extension_id_replaces_bundled`: an extension named `JSON` (id `json`) is the only `json` language in `languages()`.
  - `has_parent_predicate`: `((identifier) @x (#has-parent? @x pair))` captures only identifiers whose parent is `pair`. `#not-has-parent?` captures the inverse.
  - `unknown_predicate_disables_pattern`: `((string) @x (#zed-only? @x))` yields no `x` span and no panic.
  - In `tests/brackets.rs`, `brackets_scm_pairs_used_when_present`: a query `("[" @open "]" @close)` makes `matching_bracket` pair `[`/`]` but not `{`/`}` for that language.
- [ ] **Step 2: Run** `cargo test -p lucerna-syntax`. Expected: FAIL.
- [ ] **Step 3: Implement.**
  - Suffix matching tries the whole file name, then each `.`-suffix from longest to shortest, first exactly and then ASCII-lowercased.
  - `Associations` uses `globset` with `literal_separator(false)`. A pattern without `/` matches the file name; a pattern with `/` matches the root-relative path.
  - `SyntaxState::matching_bracket` uses the language's `brackets` query when it has one: find the `@open`/`@close` pair whose either end is at `offset`. Otherwise it keeps the Phase 1 behaviour.
- [ ] **Step 4: Run** `cargo test -p lucerna-syntax`. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat(syntax): layered language registry, files.associations, extension queries`.

### Task 4: General injections (`lucerna-syntax`)

**Files:**
- Create: `crates/lucerna-syntax/src/injection.rs` (query capture extraction, layer tree)
- Modify: `crates/lucerna-syntax/src/syntax_state.rs` (layers; remove the hand-written `inline` path)
- Modify: `crates/lucerna-syntax/src/registry.rs`: a bundled hidden language `markdown_inline` (grammar `tree_sitter_md::INLINE_LANGUAGE`, highlights `HIGHLIGHT_QUERY_INLINE`, injections `INJECTION_QUERY_INLINE`). Remove `InlineGrammar`.
- Modify: `THIRD_PARTY_NOTICES.md`, if the injection queries add a source not already listed.
- Test: `crates/lucerna-syntax/tests/injection.rs`, plus snapshots.

**Interfaces:**
- Consumes: `LanguageRegistry::language_by_name` (Task 3).
- Produces:
  - `SyntaxState::new(language, registry: Arc<LanguageRegistry>)`. The registry is needed to resolve injected names; update callers.
  - `parse` builds the root tree, then the child layers, recursively.
  - `highlights` merges spans from every layer that intersects the rows. When spans tie on range, the deeper layer wins.
  - `pub fn layer_count(&self) -> usize`, for tests.
  - `edited()` edits the root tree only and drops child layers, which are rebuilt at the next `parse`.
- Injection rules:
  - Captures: `@injection.content` or `@content`; `@injection.language` or `@language` (the node's text).
  - Properties: `#set! injection.language "<name>"` and `#set! injection.combined`.
  - `injection.include-children` and every other property are ignored.
  - Combined: every content node from the same pattern with the same language becomes one layer with several `included_ranges`.
  - Language names resolve through `language_by_name`. Unknown names produce no layer.
  - Depth: the root is depth 0; layers are created up to depth 4.
  - At most 1000 child layers in total per document. Stop creating layers past that and log a `debug`.
  - Child parses share the caller's time budget. A `TimedOut` child keeps the layers already built and returns `TimedOut`.

- [ ] **Step 1: Write failing tests** in `injection.rs`:
  - `html_script_and_style_injected`: in `<script>let x = 1;</script><style>a{color:red}</style>`, `let` has a capture starting with `keyword`, and `color` has a `property` capture. `layer_count() == 2`.
  - `markdown_inline_still_highlighted`: the Phase 1 test, unchanged. `**bold**` gets an inline span.
  - `markdown_code_fence_injects_rust`: in "```rust\nfn main() {}\n```", `fn` gets `keyword`.
  - `markdown_inline_snapshot_matches_phase1`: run `insta::assert_debug_snapshot!` over `tests/fixtures/sample.md` and **review the diff**. Spans for inline text must be unchanged. New spans (code fences, front matter) may be added. Accept the snapshot only after checking this.
  - `unknown_injected_language_is_skipped`: a code fence tagged `nosuchlang` creates no layer and no panic.
  - `combined_injection_makes_one_layer`. The fixture is an extension language whose `injections.scm` marks several `(string)` nodes `@injection.content` with `#set! injection.language "json"` and `#set! injection.combined`. It must yield `layer_count() == 1`.
  - `depth_limit_stops_at_four`: a language that injects itself recursively stops with `layer_count() <= 4` levels deep.
  - `layer_cap_1000`: 1500 code fences give `layer_count() == 1000`.
  - `edited_drops_children_until_reparse`.
- [ ] **Step 2: Run** `cargo test -p lucerna-syntax --test injection`. Expected: FAIL.
- [ ] **Step 3: Implement** `injection.rs` and the layer tree in `SyntaxState`.
  - Each child layer owns a `Parser`, with a store only if its grammar is WASM, and calls `set_included_ranges` before parsing.
  - `highlights` keeps the per-row segmenting from Phase 1. Each span is tagged with its layer depth, and the sort key becomes `(start, Reverse(end), Reverse(depth), rank)` before dedup.
- [ ] **Step 4: Run** `cargo test -p lucerna-syntax`. Expected: PASS. Then run `cargo bench -p lucerna-syntax --bench highlight -- --quick` and check the Markdown highlight benchmark is within 15% of `main`'s.
- [ ] **Step 5: Commit** with message `feat(syntax): general tree-sitter injections; markdown via injections.scm`.

### Task 5: Layered themes and lenient JSON (`lucerna-theme`)

**Files:**
- Modify: `crates/lucerna-theme/src/registry.rs`, `src/lib.rs`
- Modify: `crates/lucerna-theme/Cargo.toml` (add `jsonc-parser` with its `serde` feature)
- Modify: `crates/lucerna-workbench/src/config_files.rs` (the `ConfigFile::Themes` branch uses `build`)
- Test: `crates/lucerna-theme/tests/layers.rs`

**Interfaces:**
- Produces:
  - `pub struct ThemeFile { pub path: PathBuf, pub source: ThemeSource }`, where `pub enum ThemeSource { Extension(String /* id */), User }`.
  - `ThemeRegistry::build(extension_files: &[(String, PathBuf)], user_dir: Option<&Path>) -> (ThemeRegistry, Vec<(PathBuf, String)>)`. The load order is built-in, then extension files sorted by extension id, then user dir. A later layer wins on name. A name clash between two extensions is logged once as a `warn`.
  - `ThemeRegistry::themes_with_appearance(&self) -> Vec<(String, Appearance)>`, for the picker.
  - `parse_family` accepts JSONC: comments and trailing commas.

- [ ] **Step 1: Write failing tests.**
  - `user_beats_extension_beats_builtin`: an extension theme named "One Dark" replaces the built-in one, and a user theme of the same name replaces the extension's.
  - `extension_clash_later_id_wins`.
  - `jsonc_theme_parses`: a theme file with `//` comments and a trailing comma loads.
  - `broken_file_reported_others_load`.
- [ ] **Step 2: Run** `cargo test -p lucerna-theme`. Expected: FAIL.
- [ ] **Step 3: Implement.** `builtin()` stays as `build(&[], None).0`. Keep `load_dir` only if something still calls it; otherwise delete it.
- [ ] **Step 4: Run** `cargo test --workspace`. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat(theme): layered theme registry; JSONC theme files`.

### Task 6: Icon themes and the built-in "Lucide" theme (`lucerna-theme`, `lucerna-workbench`)

**Files:**
- Create: `crates/lucerna-theme/src/icon_theme.rs`
- Modify: `crates/lucerna-theme/src/lib.rs`
- Modify: `crates/lucerna-workbench/src/icons.rs`: `icon_for_path`'s mapping moves to the `lucide_icon_theme()` data. `icons.rs` keeps the SVG bytes and maps a Lucide name to bytes with `pub fn lucide_svg(name: &str) -> Option<&'static [u8]>`.
- Test: `crates/lucerna-theme/tests/icon_theme.rs`. Move the cases from `icons.rs`'s tests here as Lucide-theme cases.

**Interfaces:**
- Produces:
  - `pub enum IconSource { Builtin(&'static str), File(PathBuf) }`. A `Builtin` value is a Lucide name such as `"file-code"`.
  - `pub struct IconTheme { pub name: String, pub appearance: Appearance, … }`, with:
    - `fn file_icon(&self, path: &Path) -> Option<IconSource>`
    - `fn directory_icon(&self, name: &str, expanded: bool) -> Option<IconSource>`
    - `fn chevron(&self, expanded: bool) -> Option<IconSource>`
  - `pub fn lucide_icon_theme() -> IconTheme`. It holds the Phase 1 table from spec §13 as data. Chevrons are `chevron-right` / `chevron-down`; folders are `folder` / `folder-open`.
  - `pub struct IconThemeRegistry`, with:
    - `IconThemeRegistry::build(files: &[(String /*ext id*/, PathBuf /*json*/, PathBuf /*icons root*/)]) -> (IconThemeRegistry, Vec<(PathBuf, String)>)`.
    - `names(&self) -> Vec<String>`. "Lucide" comes first.
    - `resolve(&self, name: &str) -> ResolvedIcons`. Falls back to Lucide for an unknown name.
  - `pub struct ResolvedIcons`, with `file(&self, path) -> IconSource`, `directory(&self, name, expanded) -> IconSource` and `chevron(&self, expanded) -> IconSource`. Each tries the active theme, then Lucide, then `Builtin("file")`, `Builtin("folder")` or `Builtin("chevron-right")` respectively.
  - `ResolvedIcons::name(&self) -> &str`.
- Schema (Zed v0.3.0):

  ```
  { name, author, themes: [{ name, appearance,
    directory_icons{collapsed?,expanded?},
    named_directory_icons{<dir>:{collapsed?,expanded?}},
    chevron_icons{collapsed?,expanded?},
    file_stems{<stem>:<type>}, file_suffixes{<suffix>:<type>},
    file_icons{<type>:{path}} }] }
  ```

  Unknown keys are ignored.
- Paths are resolved against the icons root and canonicalised. A path that escapes the root, or an absolute path, makes that one icon unavailable and adds a warning. It does not fail the theme.
- File lookup:
  1. the full name;
  2. each `.`-suffix after the first dot, from longest to shortest;
  3. the part before the first dot;
  4. type `default`.

  At each step, try `file_stems` and then `file_suffixes`, first with the exact key and then with the ASCII-lowercased one. The resulting type maps to `file_icons[type]`.

- [ ] **Step 1: Write failing tests.**
  - `lucide_matches_phase1_table`: the moved `icons.rs` cases now go through `lucide_icon_theme()`.
  - `stem_then_suffix_order`: `file_stems {"Dockerfile": "docker"}` and `file_suffixes {"d.ts": "tsdef", "ts": "ts"}`. Then `Dockerfile` maps to docker, `dockerfile` maps to docker (lowercase fallback), `x.d.ts` maps to tsdef, and `x.ts` maps to ts.
  - `missing_type_falls_back_to_lucide`.
  - `named_directory_then_generic_then_lucide`.
  - `escaping_icon_path_rejected`: both `../../etc/x.svg` and `/abs.svg` are rejected and produce warnings, while the rest of the theme loads.
  - `unknown_theme_name_resolves_to_lucide`.
- [ ] **Step 2: Run** `cargo test -p lucerna-theme`. Expected: FAIL.
- [ ] **Step 3: Implement.** Explorer rendering is not changed in this task; Task 14 does that. `icon_for_path` is kept temporarily as a thin wrapper over `lucide_icon_theme()` so that the workbench still compiles.
- [ ] **Step 4: Run** `cargo test --workspace`. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat(theme): Zed icon themes; Lucide as the built-in icon theme`.

### Task 7: `lucerna-extension` crate: manifests, language configs, snippets

**Files:**
- Create: `crates/lucerna-extension/Cargo.toml` (deps: `serde`, `serde_json`, `toml`, `jsonc-parser`, `regex`, `tracing`; dev-deps: `tempfile`)
- Create: `crates/lucerna-extension/src/lib.rs`, `src/manifest.rs`, `src/language_config.rs`, `src/snippets.rs`
- Modify: root `Cargo.toml` (`lucerna-extension` path dependency)
- Test: `crates/lucerna-extension/tests/manifest.rs`, `tests/snippets.rs`, with fixtures under `tests/fixtures/manifests/`

**Interfaces:**
- Produces:
  - `pub struct ExtensionManifest`, which is `Serialize + Deserialize`. Its fields:
    - `id: String`, `name: String`, `version: String`, `schema_version: u32`
    - `description: Option<String>`, `repository: Option<String>`, `authors: Vec<String>`
    - `lib: Option<toml::Value>`
    - `themes: Vec<PathBuf>`, `icon_themes: Vec<PathBuf>`, `languages: Vec<PathBuf>`
    - `grammars: BTreeMap<String, GrammarManifest>`
    - `snippets: Vec<PathBuf>`
    - `language_servers: BTreeMap<String, toml::Value>`
    - `other: BTreeMap<String, toml::Value>`, collecting every remaining key (`context_servers`, `slash_commands`, `capabilities`…).
  - `pub struct GrammarManifest { pub repository: String, pub rev: String /* alias "commit" */, pub path: Option<String> }`.
  - `pub fn load_manifest(dir: &Path) -> Result<ExtensionManifest, ManifestError>`. It reads `extension.toml`, or `extension.json` as a fallback, then calls `populate_defaults`. It errors on `schema_version > 1`, on a grammar name not matching `^[a-z0-9_]+$`, and when neither file exists.
  - `fn populate_defaults(&mut self, dir: &Path)` follows spec §3.2:
    - Schema 0 clears `languages`, `grammars` and `themes`, and reads `grammars/*.toml`.
    - It discovers `languages/*/config.toml`, `themes/*.json`, `icon_themes/*.json`, and `snippets.json`.
    - Explicit entries come first; duplicates are dropped.
  - `ExtensionManifest::unsupported(&self) -> Vec<&'static str>` returns, for example, `["language_servers", "lib"]` for the panel.
  - `pub struct LanguageConfig { pub name: String, pub grammar: Option<String>, pub path_suffixes: Vec<String>, pub first_line_pattern: Option<String>, pub line_comments: Vec<String>, pub hidden: bool, pub tab_size: Option<u32> }`. Every other field is ignored.
  - `pub fn load_language_config(dir: &Path) -> Result<LanguageConfig, ManifestError>`.
  - `pub const QUERY_KINDS: [&str; 10]`, the spec list.
  - `pub fn query_files(dir: &Path) -> (Vec<(String /*kind*/, PathBuf)>, Vec<String> /*warnings for unknown .scm*/)`.
  - `pub struct SnippetFile { pub language: Option<String> /* None = global */, pub valid: usize, pub errors: Vec<String> }`.
  - `pub fn check_snippets(path: &Path) -> Result<SnippetFile, String>`.
    - The language comes from the lowercased file stem; `snippets` means global.
    - Each entry is `{prefix?: str|[str], body: str|[str], description?: str|[str]}`.
    - The body is checked by `fn check_snippet_body(&str) -> Result<(), String>`, which accepts `$n`, `${n}`, `${n:…}` (nested), `${n|a,b|}`, `$VAR`, `${VAR}`, `${VAR:…}` and `\$`, `\}`, `\\` escapes. It rejects an unclosed `${` and an unterminated choice.
  - `pub enum ManifestError { Io(PathBuf, String), Parse(PathBuf, String), Invalid(String) }`, which implements `Display`.

- [ ] **Step 1: Write failing tests.**
  - `schema1_full_manifest`: a fixture with every section.
  - `schema0_reads_grammars_toml_and_discovers_dirs`.
  - `legacy_extension_json_uses_dir_name_as_id`.
  - `defaults_discovered_and_deduplicated`.
  - `schema_version_2_rejected`.
  - `bad_grammar_name_rejected` (`Foo-Bar`).
  - `commit_alias_for_rev`.
  - `unsupported_lists_lsp_and_lib`.
  - `language_config_ignores_unknown_fields`.
  - `query_files_warn_on_folds_scm`: a `folds.scm` produces a warning and is not listed.
  - `snippets_language_from_stem`.
  - `snippet_body_accepts_textmate_forms`.
  - `snippet_body_rejects_unclosed`.
  - `snippet_array_body_joined`.
- [ ] **Step 2: Run** `cargo test -p lucerna-extension`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p lucerna-extension` and `cargo deny check`. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat(extension): manifest, language config and snippet parsing`.

### Task 8: Processes, git and the registry (`lucerna-extension`)

**Files:**
- Create: `crates/lucerna-extension/src/process.rs`, `src/git.rs`, `src/registry.rs`, `src/disk.rs`
- Create: `crates/lucerna-extension/src/test_support.rs`, behind a cargo feature `test-support` (the crate's own tests enable it through a dev-dependency on itself with that feature). Helpers build `file://` git repos in a temp dir: `fake_repo(files) -> (PathBuf, commit)`, and `fake_registry(entries) -> PathBuf`, which writes `extensions.toml`, `.gitmodules` and real submodule gitlinks via `git submodule add`. Tasks 9 and 10 add `fake_wasi_sdk` and `FakeValidator` here. Task 15 uses this module from the workbench's tests.
- Test: `crates/lucerna-extension/tests/registry.rs`, `tests/process.rs`

**Interfaces:**
- Produces:
  - `pub struct CancelToken`, which is `Clone`, with `cancel()` and `is_cancelled()`.
  - `pub fn run(cmd: Command, timeout: Duration, cancel: &CancelToken, log: &mut dyn Write) -> Result<Output, ProcessError>`. It polls the child every 50 ms, kills it on timeout or cancel, and appends the command line, stdout and stderr to `log`.
  - `pub enum ProcessError { NotFound(String), Failed { cmd: String, status: i32, stderr: String }, TimedOut(String), Cancelled }`.
  - In `git.rs`:
    - `pub fn git_available() -> bool`.
    - `pub fn normalize_url(url: &str) -> String`: `git@github.com:o/r(.git)` becomes `https://github.com/o/r`; other URLs are unchanged.
    - `pub fn fetch_commit(url, commit, dest: &Path, cancel, log) -> Result<(), ProcessError>`. If `dest` is already at `commit`, it is a no-op. Otherwise: `git init`, `fetch --depth 1 <url> <commit>`, `checkout --force FETCH_HEAD`.
    - Every call applies the Global Constraints environment and `-c` flags.
  - In `disk.rs`: `pub fn free_bytes(path: &Path) -> io::Result<u64>` (via `statvfs`) and `pub const MIN_FREE_BYTES: u64 = 1_288_490_189`.
  - In `registry.rs`:
    - `pub struct RegistryEntry { pub id: String, pub version: String, pub url: String, pub commit: String, pub path: Option<String>, pub name: Option<String>, pub description: Option<String> }`, which is `Serialize + Deserialize`.
    - `pub struct Registry { pub fetched_at: SystemTime, pub entries: Vec<RegistryEntry> }`, sorted by id.
    - `pub fn refresh(root: &Path, remote: &str, cancel, log) -> Result<(Registry, Vec<String> /*warnings*/), RegistryError>`. It runs the spec §3.4 steps and writes `registry.json` atomically (temp file plus `rename`). Previously known `name` and `description` values are kept when an entry's commit is unchanged.
    - `pub fn load_cached(root: &Path) -> Option<Registry>`. It deletes a corrupt `registry.json` and returns `None`.
    - `pub fn is_stale(reg: &Registry, now: SystemTime) -> bool`, true after 24 h.
    - `pub fn parse_parts(extensions_toml: &str, gitmodules: &str, ls_tree: &str) -> (Vec<RegistryEntry>, Vec<String>)`. A pure function.

- [ ] **Step 1: Write failing tests.**
  - `parse_parts_joins_three_sources`. Use a trimmed copy of the real `extensions.toml`, `.gitmodules` and `ls-tree` output saved as `tests/fixtures/registry/`, with about 5 entries including one with `path`.
  - `parse_parts_skips_entry_without_commit_with_warning`.
  - `normalize_ssh_github_url`.
  - `refresh_from_file_registry`: `fake_registry` holds 2 extensions. `refresh` returns both, with the right commits, and writes `registry.json`.
  - `refresh_twice_fetches_updates`: after adding a commit to the fake registry, the second `refresh` sees the new version.
  - `fetch_commit_idempotent`.
  - `fetch_nonexistent_url_fails_fast_without_prompt`: `https://127.0.0.1:9/none.git` returns `Err(Failed|TimedOut)` within 120 s. Assert it returns in under 30 s in practice, because the connection is refused.
  - `run_times_out_and_kills`: `sleep 5` with a 200 ms timeout returns `TimedOut`, and the process is gone.
  - `run_cancel_kills`.
  - `corrupt_registry_json_discarded`.
  - `stale_after_24h`.
- [ ] **Step 2: Run** `cargo test -p lucerna-extension`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p lucerna-extension`. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat(extension): process runner, git fetch and registry refresh`.

### Task 9: Toolchain and builder (`lucerna-extension`)

**Files:**
- Create: `crates/lucerna-extension/src/toolchain.rs`, `src/builder.rs`, `src/validate.rs`
- Modify: `crates/lucerna-extension/Cargo.toml` (add `sha2`)
- Test: `crates/lucerna-extension/tests/builder.rs`, `tests/toolchain.rs`. `test_support` gains `FakeValidator` (accepts everything unless the grammar or query name is `bad`) and `fake_wasi_sdk(dir, wasm_fixture)`, which writes a `bin/clang` shell script that copies the fixture to the `-o` path, plus a `VERSION` file with `34.0`.

**Interfaces:**
- Consumes: Task 7 (manifest, config, snippets) and Task 8 (`run`, `fetch_commit`, `CancelToken`).
- Produces:
  - In `toolchain.rs`:
    - `pub const WASI_SDK_VERSION: &str = "34.0"`.
    - `const SHA256_ARM64: &str` and `const SHA256_X86_64: &str`, both from Task 1's `README.md`.
    - `pub fn ensure_wasi_sdk(root: &Path, cancel, log, progress: &dyn Fn()) -> Result<PathBuf, BuildError>`. It prefers `WASI_SDK_PATH`, then an existing `toolchain/wasi-sdk-34/` whose `VERSION` reads `34.0`. Otherwise it downloads with `curl -fL --max-time 120 -o`, verifies the SHA-256, and extracts with `tar -xzf` into a temp dir that is renamed into place. On a hash mismatch it deletes the file and returns `BuildError::Checksum`.
  - In `validate.rs`:

    ```rust
    pub trait GrammarValidator: Send + Sync {
        fn load(&self, name: &str, wasm: &[u8]) -> Result<(), String>;
        fn check_query(&self, name: &str, wasm: &[u8], kind: &str, source: &str) -> Result<(), String>;
    }
    ```

    Also `pub fn check_theme_json(path) -> Result<Vec<(String /*name*/, String /*appearance*/)>, String>`, which parses only the outer family shape (lenient JSON), and `pub fn check_icon_theme_json(path, icons_root) -> Result<(Vec<(String, String)>, Vec<String> /*warnings*/), String>`.
  - In `builder.rs`:
    - `pub struct BuildOutput { pub staging: PathBuf, pub manifest: ExtensionManifest, pub warnings: Vec<String> }`.
    - `pub fn build(source_dir: &Path, root: &Path, id: &str, expected_version: Option<&str>, validator: &dyn GrammarValidator, cancel, log, progress: &dyn Fn(Stage)) -> Result<BuildOutput, BuildError>`. `expected_version` is `None` for dev installs. It performs spec §3.5 steps 2–6, up to and including writing into `staging/`; it does not rename into `installed/`.
    - `pub enum Stage { Fetching, ReadingManifest, PreparingToolchain, BuildingGrammar { name: String, index: usize, total: usize }, Validating, Installing }`.
    - `pub enum BuildError { Process(ProcessError), Manifest(ManifestError), VersionMismatch { registry: String, manifest: String }, Checksum, CppScanner(String), NoParser(String), NothingUsable(Vec<String>), DiskFull(u64), Io(String), Cancelled }`. It implements `Display` with short, user-readable messages.
  - Grammar build cache:
    - Key file: `work/<id>/grammars/<name>.wasm`, with `<name>.rev` next to it.
    - Reuse when the rev matches and the `.wasm` is newer than both `parser.c` and `scanner.c`.
  - Pruning:
    - A language whose grammar failed is dropped with a warning.
    - A failing query file is dropped with a warning. If it is `highlights.scm`, the language stays.
    - A theme or icon theme that fails to parse is dropped.
    - An icon path that escapes the root produces a warning.
    - Bad snippets are counted, not fatal.
  - Staging layout: the spec §3.1 `installed/<id>` layout, plus `extension.toml` written back from the populated manifest with `toml::to_string`.

- [ ] **Step 1: Write failing tests**, using a fake validator that accepts everything unless the name is `bad`.
  - `ensure_wasi_sdk_prefers_env`.
  - `ensure_wasi_sdk_rejects_bad_checksum`: point `curl` at a `file://` tarball whose bytes do not match. The function is made testable with `ensure_wasi_sdk_from(url, sha, …)`.
  - `build_theme_only_extension`: the staging dir holds `extension.toml` and `themes/x.json`. No toolchain is touched.
  - `build_language_with_grammar`: a fake grammar repo plus the fake wasi-sdk. The result has `grammars/<g>.wasm` and `languages/<lang>/{config.toml,highlights.scm}`, and the stages are emitted in order.
  - `grammar_cache_reused_on_second_build`: the fake clang appends to a counter file; the count stays at 1 after two builds.
  - `cpp_scanner_drops_language_with_warning`: a grammar repo with only `src/scanner.cc` drops its language. Another language in the same extension survives.
  - `bad_query_dropped_language_kept`.
  - `nothing_usable_fails`.
  - `version_mismatch_fails`: `expected_version = Some("9.9.9")` against a manifest at `0.1.0`.
  - `cancel_during_grammar_build_cleans_staging`.
- [ ] **Step 2: Run** `cargo test -p lucerna-extension`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p lucerna-extension`. Expected: PASS. Then run the real build once, ignored by default: `WASI_SDK_PATH=… cargo test -p lucerna-extension -- --ignored real_clang_builds_json_grammar`. It builds `tree-sitter-json` from a local clone. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat(extension): wasi-sdk toolchain and extension builder`.

### Task 10: Store, index, install/update/uninstall (`lucerna-extension`)

**Files:**
- Create: `crates/lucerna-extension/src/store.rs`, `src/index.rs`, `src/lock.rs`
- Modify: `crates/lucerna-extension/Cargo.toml` (add `libc`, `notify`, `crossbeam-channel`)
- Test: `crates/lucerna-extension/tests/store.rs`

**Interfaces:**
- Consumes: Tasks 7–9.
- Produces:
  - In `index.rs`:
    - `pub struct ExtensionIndex { pub extensions: BTreeMap<String, IndexedExtension> }`, which is `Serialize + Deserialize`.
    - `pub struct IndexedExtension`, with fields:
      - `manifest: ExtensionManifest`
      - `info: InstallInfo { commit, version, installed_at, source: Source /* Registry | Dev */ }`
      - `languages: Vec<LanguageEntry>`
      - `themes: Vec<ThemeEntry>`, `icon_themes: Vec<IconThemeEntry>`
      - `snippets: BTreeMap<String /* language or "*" */, usize>`
      - `broken: Option<String>`
    - `pub struct LanguageEntry { pub name, pub dir: PathBuf, pub grammar: Option<GrammarRef { name, wasm_path }>, pub path_suffixes, pub first_line_pattern, pub hidden, pub queries: Vec<String> }`.
    - `pub struct ThemeEntry { pub name, pub appearance, pub path }`.
    - `pub struct IconThemeEntry { pub name, pub appearance, pub path, pub icons_root }`.
    - `pub fn rebuild(installed: &Path) -> ExtensionIndex`. It skips dot-entries and marks a broken dir instead of failing.
    - `pub fn load_or_rebuild(root: &Path) -> ExtensionIndex`. It rebuilds when the file is missing or corrupt, or when the mtime of `installed/` is newer than the index.
  - `lock.rs`: `pub struct IdLock`, acquired by `IdLock::try_acquire(root, id) -> Result<IdLock, StoreError::Busy>`, using `flock(LOCK_EX | LOCK_NB)` on `locks/<id>.lock`. It is released on `Drop`.
  - `store.rs`:
    - `pub struct Store`, which is `Clone` (it holds an `Arc` inside).
    - `Store::open(root: PathBuf, validator: Arc<dyn GrammarValidator>) -> Store`. At open it removes leftover `installed/.old-*`, `installed/.trash-*` and `staging/*`, and loads the index.
    - `Store::with_remote(self, url: String) -> Store`, used by tests. The default remote is the registry repository URL.
    - `index(&self) -> Arc<ExtensionIndex>`.
    - `registry(&self) -> Option<Registry>`.
    - `refresh_registry(&self, cancel) -> Result<Vec<String>, StoreError>`.
    - `describe(&self, id, cancel) -> Result<RegistryEntry, StoreError>`. It fetches the source, reads the manifest, and persists `name` and `description` into `registry.json`.
    - `install(&self, id, progress: &dyn Fn(InstallProgress), cancel) -> Result<Vec<String> /*warnings*/, StoreError>`. Install and update share this one entry point.
    - `install_dev(&self, dir: &Path, progress, cancel) -> Result<Vec<String>, StoreError>`.
    - `uninstall(&self, id) -> Result<(), StoreError>`.
    - `updates(&self) -> Vec<String>`. An id is in the list when the registry `version` or `commit` differs from the installed one; dev installs are never included.
    - `clear_build_cache(&self) -> Result<(), StoreError>`.
    - `subscribe(&self) -> crossbeam_channel::Receiver<StoreEvent>`.
    - `watch(&self) -> notify::Result<()>`. It starts a 200 ms-debounced watcher on `installed/`, which rebuilds the index and emits `Changed` for changes made by another process.
  - `pub struct InstallProgress { pub id: String, pub stage: Stage }`.
  - `pub enum StoreEvent { Changed(Arc<ExtensionIndex>), RegistryRefreshed }`.
  - `pub enum StoreError { UnknownId(String), Busy(String), GitMissing, Build(BuildError), Registry(String), Io(String) }`.
  - Every install, update and uninstall runs inside `catch_unwind` (R3). A panic becomes `StoreError::Io("internal error")`, plus an `error` log line.
  - `logs/<id>.log` is truncated at the start of each install.

- [ ] **Step 1: Write failing tests.** They use the fake registry, the fake wasi-sdk and the fake validator.
  - `install_theme_extension_end_to_end`: `installed/<id>/` exists, `index().extensions[id].themes.len() == 1`, `lucerna.json` has the commit, and a `Changed` event is received.
  - `install_language_extension_indexes_language`: `wasm_path` points inside `installed/`.
  - `failed_install_keeps_previous_version`: install v1, publish a broken v2 in the fake registry, refresh, install. The result is `Err`, v1 remains, and `staging/` is empty.
  - `update_detected_and_applied`.
  - `uninstall_removes_and_emits`.
  - `second_install_same_id_is_busy`: hold an `IdLock` for the id in the test, then call `install`, which returns `Busy`.
  - `startup_cleans_leftovers`.
  - `broken_installed_dir_marked`.
  - `index_rebuilt_when_corrupt`.
  - `describe_persists_name_and_description`.
  - `install_dev_copies_not_symlinks`.
  - `unknown_id_error`.
  - `git_missing_reported`. Run with `PATH` set to an empty dir through `Store::with_git_program("/nonexistent/git")`, a test hook.
  - `watcher_sees_external_uninstall`: rename `installed/x` away from outside the store. A `Changed` event arrives within 2 s.
- [ ] **Step 2: Run** `cargo test -p lucerna-extension`. Expected: FAIL.
- [ ] **Step 3: Implement.** The disk check (`MIN_FREE_BYTES`) runs before fetching and before building. Install steps follow spec §3.5 steps 1–7; uninstall follows §3.6.
- [ ] **Step 4: Run** `cargo test -p lucerna-extension`. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat(extension): store with atomic install, update, uninstall and index`.

### Task 11: Grammar validator adapter and `luc ext` CLI

**Files:**
- Create: `crates/lucerna-workbench/src/extensions/mod.rs` and `extensions/validator.rs`. The validator lives in the workbench because both the app and the CLI depend on it, and `lucerna-syntax` must not depend on `lucerna-extension`.
- Modify: `crates/lucerna-workbench/Cargo.toml` (add `lucerna-extension`)
- Create: `crates/lucerna/src/ext_cli.rs`
- Modify: `crates/lucerna/src/main.rs`. Add `#[command(subcommand)] command: Option<Command>` and `args_conflicts_with_subcommands = true`. Paths keep working; a folder literally named `ext` must be opened as `luc ./ext`, which the help text says.
- Modify: `crates/lucerna/Cargo.toml` (add `lucerna-extension`)
- Test: `crates/lucerna/tests/ext_cli.rs`

**Interfaces:**
- Consumes: `Store` (Task 10), and `validate_wasm_grammar` / `validate_query` (Task 2).
- Produces:
  - `pub struct WasmGrammarValidator`, which implements `GrammarValidator` by calling the `lucerna-syntax` functions.
  - `pub fn extensions_root() -> PathBuf`, which returns `~/Library/Application Support/Lucerna/extensions`. The environment variable `LUCERNA_EXTENSIONS_DIR` overrides it; this is used by tests.
  - The CLI:

    ```
    luc ext search <query>
    luc ext list [--installed] [--updates]
    luc ext install <id>... | --dev <dir>
    luc ext remove <id>...
    luc ext update [<id>...]
    ```

    - Progress goes to stderr, one line per `Stage`, for example `zig: building grammar 1/1 (zig)`.
    - Warnings are prefixed `warning:`.
    - The exit code is 1 when any id fails.
    - `search` and `list` refresh the registry first when it is stale.
    - `search` uses `nucleo-matcher` on the id, plus the name and description when known, and prints `id version  description`.

- [ ] **Step 1: Write failing tests** with `assert_cmd`, setting `LUCERNA_EXTENSIONS_DIR` to a temp dir:
  - `ext_list_installed_empty`: exit 0, empty output.
  - `ext_install_dev_theme_then_list`: `install --dev <fixture theme ext>`, then `list --installed`, which shows its id and version.
  - `ext_install_unknown_id_fails`: exit 1, and stderr mentions the id. Use a local `file://` fake registry via the hidden `--registry-url` flag.
  - `ext_remove_then_list_empty`.
  - `plain_path_still_opens`: `luc --help` lists both the `ext` subcommand and the `[PATH]` argument.
- [ ] **Step 2: Run** `cargo test -p lucerna --test ext_cli`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p lucerna`. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat(cli): luc ext search/list/install/remove/update`.

### Task 12: Settings keys

**Files:**
- Modify: `crates/lucerna-settings/assets/default-settings.json` (the three keys from Global Constraints)
- Modify: `crates/lucerna-settings/src/when.rs` or the context-key list, wherever Phase 1 declares known keys (add `extensionsViewletFocus`)
- Test: the existing settings tests, extended

- [ ] **Step 1: Write failing tests.**
  - `defaults_include_phase2_keys`: `get::<String>("workbench.iconTheme") == "Lucide"`, `get::<bool>("extensions.autoCheckUpdates") == true`, and `get::<BTreeMap<String, String>>("files.associations")` is empty.
  - `files_associations_merge_across_layers`: user `{"*.a": "x"}` plus workspace `{"*.b": "y"}` gives both (R26).
- [ ] **Step 2: Run** `cargo test -p lucerna-settings`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run.** Expected: PASS.
- [ ] **Step 5: Commit** with message `feat(settings): workbench.iconTheme, files.associations, extensions.autoCheckUpdates`.

### Task 13: Extension service and registry swap (`lucerna-workbench`)

**Files:**
- Create: `crates/lucerna-workbench/src/extensions/service.rs`
- Modify: `crates/lucerna-workbench/src/app_state.rs`. Add `icon_themes: IconThemeRegistry` and `icons: ResolvedIcons`. `languages` stays an `Arc<LanguageRegistry>`, but is now replaced at runtime. Add `languages_generation: u64`.
- Modify: `crates/lucerna-workbench/src/app.rs`: open the `Store`, start the service, and set the initial registries from the index **synchronously from `index.json`** without loading any grammar.
- Modify: `crates/lucerna-workbench/src/config_files.rs`. Theme loading goes through `ThemeRegistry::build` with the extension theme list. Settings changes to `files.associations` or `workbench.iconTheme` trigger a rebuild or re-resolve. `apply_configured_theme` is unchanged in behaviour: it keeps the setting when the theme is missing.
- Modify: `crates/lucerna-workbench/src/documents.rs`, adding `fn languages_changed(&mut self, cx)`.
- Test: `crates/lucerna-workbench/tests/extensions_service.rs`

**Interfaces:**
- Consumes: Tasks 3, 5, 6, 10 and 11.
- Produces:
  - `pub struct ExtensionService`, a GPUI global, with:
    - `store() -> &Store`
    - `progress(&self, id) -> Option<&InstallProgress>`
    - `last_error(&self, id) -> Option<&str>`
    - `warnings(&self, id) -> &[String]`
    - `updates(&self) -> &[String]`
  - `ExtensionService::install(id: String, cx: &mut App)`, `uninstall(id, cx)`, `update_all(cx)`, `cancel(id, cx)`, `refresh_registry(cx)` and `describe(id, cx)`. Each runs on the background executor and pushes state back with `cx.update`.
  - `pub fn rebuild_registries(index: &ExtensionIndex, associations: &BTreeMap<String, String>, user_themes: &Path) -> Registries`. It runs on the background thread. `Registries { languages, themes, icon_themes, warnings }`.
  - `pub(crate) fn apply_registries(r: Registries, cx: &mut App)`. It swaps the `AppState` fields, bumps `languages_generation`, calls `Documents::languages_changed`, re-applies `workbench.colorTheme` and `workbench.iconTheme`, and calls `cx.refresh_windows()`.
  - `Documents::languages_changed`: for every open document, compute `language_for_file(path, first_line)` again. When the result differs by `Arc::ptr_eq`, create a new `SyntaxState`, or set it to `None`, and reparse.
  - Converting `IndexedExtension.languages` into `ExtensionLanguage` happens here.
  - Startup refresh: when `extensions.autoCheckUpdates` is on and the registry is stale (or missing), call `refresh_registry` 5 s after the first frame.

- [ ] **Step 1: Write failing tests** with `TestAppContext`, a temp store root, and a hand-written `installed/<id>` that uses the `json.wasm` fixture:
  - `startup_reads_index_without_loading_grammar`: after `set_app_state`, the extension language is in the registry, and `wasm` grammar cache size is 0. Expose `lucerna_syntax::wasm_cache_len()` for this test (`#[doc(hidden)] pub`).
  - `install_event_recolors_open_tab`: open `a.zz` (plain) → write installed ext claiming `zz` + emit `Changed` → document's language is the ext language and highlights non-empty after parse.
  - `uninstall_falls_back_without_crash` (Review Focus 3): open a `.json` file under an extension language, uninstall it. The document falls back to bundled JSON and still renders.
  - `failed_grammar_tab_shows_text` (Review Focus 4): the extension's `json.wasm` is replaced with garbage before the tab opens. The doc state becomes `Ready` with no syntax; it does not stay `Loading`.
  - `color_theme_applied_when_extension_appears`: the setting names "Ext Dark", which starts out missing, so One Dark is active and the setting is unchanged. After the install event, "Ext Dark" is active.
  - `files_associations_change_rebuilds`.
- [ ] **Step 2: Run** `cargo test -p lucerna-workbench --test extensions_service`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test --workspace`. Expected: PASS.
- [ ] **Step 5: Commit** with message `feat(workbench): extension service; swap language/theme/icon registries at runtime`.

### Task 14: Icon theme rendering and picker (`lucerna-workbench`)

**Files:**
- Modify: `crates/lucerna-workbench/src/explorer/view.rs`, plus the tab and title-bar code wherever file or folder icons are drawn. They read `AppState.icons`.
- Modify: `crates/lucerna-workbench/src/ui.rs`. Add `pub fn file_icon(source: &IconSource, color: Hsla) -> AnyElement`:
  - `Builtin(name)` renders with `svg()` from `lucide_svg(name)`, tinted with `color`.
  - `File(path)` renders with `img(path)`, at the same size, with no tint.
- Modify: `crates/lucerna-workbench/src/theme_picker.rs`. Generalise it so the same picker serves `workbench.action.selectIconTheme`, with preview on move and revert on escape, writing `workbench.iconTheme` to user settings.
- Modify: `crates/lucerna-workbench/src/icons.rs`. Delete the temporary `icon_for_path` wrapper from Task 6.
- Test: extend `tests/explorer_view.rs`, and create `tests/icon_theme_picker.rs`.

- [ ] **Step 1: Verify `img()` with SVG files.** Write a test that renders `img("<tmp>/x.svg")` in a `TestAppContext` window and asserts the image loads without error. Check the GPUI image cache API at the pinned rev.
  - If GPUI cannot load SVG from a path, rasterise once with the `resvg` already in GPUI's dependency tree into an `Arc<RenderImage>`, cached by path in `ui.rs`. Record which path was taken in the commit message.
- [ ] **Step 2: Write failing tests.**
  - `explorer_uses_extension_icon`: with an icon theme mapping `json` to `json.svg`, the row for `a.json` has `IconSource::File(…json.svg)`. Expose the chosen source through the explorer row's debug selector or through a model accessor.
  - `explorer_falls_back_to_lucide`.
  - `icon_theme_picker_previews_and_persists`.
  - `unknown_icon_theme_setting_kept`.
- [ ] **Step 3: Run** `cargo test -p lucerna-workbench`. Expected: FAIL.
- [ ] **Step 4: Implement.**
- [ ] **Step 5: Run** `cargo test --workspace`. Expected: PASS.
- [ ] **Step 6: Commit** with message `feat(workbench): render icon themes; icon theme picker`.

### Task 15: Extensions panel, status bar button, commands (`lucerna-workbench`)

**Files:**
- Create: `crates/lucerna-workbench/src/extensions/panel.rs` (the view) and `extensions/panel_model.rs` (the pure filter, sort and row model, testable without a window)
- Create: `crates/lucerna-workbench/src/extensions/toast.rs`, a minimal bottom-right notification with one action button that closes after 10 s
- Modify: `crates/lucerna-workbench/src/workbench.rs` (`SidebarView::Extensions`; restoring the saved sidebar state accepts `"extensions"`)
- Modify: `crates/lucerna-workbench/src/layout/panel_toggles.rs`: a third button with the Lucide `blocks` icon (add `blocks.svg` to `assets/icons/` and to `ALL`). It shows an update-count badge and a spinner while any install runs.
- Modify: `crates/lucerna-workbench/src/commands.rs` and the default keymap (the Global Constraints commands; `cmd+shift+x`)
- Test: `crates/lucerna-workbench/tests/extensions_panel.rs`, plus unit tests in `panel_model.rs`

**Interfaces:**
- Consumes: `ExtensionService` (Task 13).
- Produces:
  - `panel_model::rows(registry: Option<&Registry>, index: &ExtensionIndex, updates: &[String], query: &str) -> PanelRows { installed: Vec<Row>, available: Vec<Row> }`.
  - Filters: `@installed`, `@updates`, `@themes`, `@icons`, `@languages`. A filter token and free text combine.
  - Free text uses fuzzy matching with `nucleo-matcher` on id, name and description.
  - `Row` holds:
    - `id`, `version`, `name: Option`, `description: Option`
    - `summary: String`, for example `"2 themes · 1 language · 14 snippets (Phase 4)"`
    - `lsp_note: bool`
    - `state: RowState`, one of `NotInstalled`, `Installed`, `UpdateAvailable`, `Installing(Stage)`, `Failed(String)` or `Broken(String)`.
  - Panel copy, exactly:
    - `"Loading extension list…"`
    - `"git was not found. Install the Xcode Command Line Tools: xcode-select --install"`
    - `"Could not fetch the extension list."` with a `Retry` button
    - `"Language server: Phase 3"`
    - `"Show Log"`, `"Cancel"`, `"Install"`, `"Uninstall"`, `"Update"`, `"Reinstall"`
    - Toast: `"Use theme “{name}”?"` with the button `"Set Theme"`
  - Expanding an uninstalled row calls `ExtensionService::describe` when its description is unknown.
  - "Show Log" opens `logs/<id>.log` in a preview tab.
  - The install command (`workbench.extensions.action.installExtension`) opens Quick Open in a mode that lists registry ids. Enter installs the selected id. Reuse the Quick Open picker with a new provider.

- [ ] **Step 1: Write failing tests.**
  - In `panel_model`:
    - `filter_installed_and_updates`
    - `fuzzy_matches_name_when_known`
    - `summary_text`
    - `lsp_note_from_manifest`
    - `installing_state_shows_stage`
  - In `extensions_panel.rs`:
    - `status_bar_button_toggles_panel`
    - `cmd_shift_x_opens_panel`
    - `install_button_runs_and_shows_progress`. The service runs on a store rooted in a temp dir, built with `lucerna-extension`'s `test-support` helpers: a fake registry, a fake wasi-sdk, and `FakeValidator`.
    - `failed_install_shows_error_and_log_button`
    - `theme_install_offers_set_theme_toast`
    - `update_badge_count`
    - `panel_state_restored_from_workspace_state`
- [ ] **Step 2: Run** `cargo test -p lucerna-workbench`. Expected: FAIL.
- [ ] **Step 3: Implement.** The available list uses `uniform_list`, with rows 24 px tall when collapsed. Styling follows spec §12–§13 of Phase 1.
- [ ] **Step 4: Run** `cargo test --workspace`. Expected: PASS.
- [ ] **Step 5: Run the app** (`cargo run -p lucerna -- .`) and check that the panel opens, the list loads from the real registry, and the layout matches the other panels. Take a screenshot for the review.
- [ ] **Step 6: Commit** with message `feat(workbench): Extensions panel, status bar button and commands`.

### Task 15b: Extensions as an editor-tab page (replaces the sidebar panel view)

The user chose (2026-11-09) a Zed-style dedicated page in an editor tab instead of the sidebar panel. Spec §5.1–§5.2 were rewritten accordingly; read them. Task 15's `ExtensionService` additions, `panel_model`, the Quick Open install provider, the toast and the commands stay; only the view and how it is opened change. Task 15's review findings are folded in here.

**Files:**
- Create: `crates/lucerna-workbench/src/extensions/page.rs` (the page view), and move command registration out of `panel.rs` into `extensions/commands.rs`
- Delete: `crates/lucerna-workbench/src/extensions/panel.rs` once nothing uses it; remove `SidebarView::Extensions` (workspace state that says `"extensions"` for the sidebar now restores to Explorer)
- Modify: `crates/lucerna-workbench/src/editor_groups/` so a group can hold a non-file tab kind (`TabItem::Extensions` or equivalent), with tab title "Extensions", the `blocks` icon, never preview, closable with Cmd+W
- Modify: `crates/lucerna-workbench/src/workspace_state.rs` (persist and restore the Extensions tab in its group and position)
- Modify: `layout/panel_toggles.rs` (the `blocks` button opens/focuses the tab; badge and spinner unchanged), `workbench.rs` (`extensionsViewletFocus` = the Extensions tab is focused)
- Modify: `crates/lucerna-workbench/src/extensions/service.rs` (fix 1 below), `quick_open/` (fix 2 below)
- Test: replace `tests/extensions_panel.rs` with `tests/extensions_page.rs`; keep `panel_model` unit tests

**Interfaces:**
- Consumes: `ExtensionService` and `panel_model::rows` (Task 15).
- Produces: `Workbench::open_extensions(&mut self, window, cx)`, which focuses the existing Extensions tab in any group or opens one in the active group, then focuses the search input.

**Behaviour (from spec §5.2):**
- Header: a search input plus filter buttons `All`, `Installed`, `Updates`, `Themes`, `Icon Themes`, `Languages`. A button sets the matching `@` token; typed `@` tokens still work.
- Cards: a virtualised GPUI `list` (variable heights), with installed extensions first and then available ones.
  - Card lines and buttons as in spec §5.2.
  - The description of an uninstalled card is fetched with `ExtensionService::describe` the first time the card is laid out. Show `"Loading description…"` while it loads. A failed describe may be retried the next time the card appears after 60 s.
  - Card style: 1 px `border.variant`, 6 px radius, `ghost_element.hover` on hover, a 4 px grid.
- Copy strings are unchanged from Task 15, plus `"Loading description…"`. Filter button labels are exactly as above.

**Fixes carried from Task 15's review:**
1. **Install while describe runs:** an Install (or Update) requested while a describe runs for the same id is queued and starts as soon as the describe ends. The card shows the installing state immediately. Never drop it silently.
2. **Install Quick Open:** cap the rows at `QUICK_OPEN_LIMIT`, or render that mode with `uniform_list`. When the registry is not loaded yet, show `"Loading extension list…"` and fill the list when it arrives.
3. **Search focus:** opening or focusing the tab (Cmd+Shift+X, the status bar button, the command) focuses the search input.
4. **Card clicks capture the extension id,** not a row index.
5. **Clear Build Cache** refuses with a visible message (toast `"Can't clear the build cache while an extension is installing."`) while any operation runs.
6. **`offer_installed_themes`** skips (rather than `break`s on) a fresh install whose index entry is missing.

- [ ] **Step 1: Write failing tests** in `tests/extensions_page.rs`:
  - `cmd_shift_x_opens_single_tab_and_focuses_search`: pressing it twice leaves one Extensions tab. Typed text goes into the search.
  - `status_bar_button_focuses_existing_tab_in_other_group`
  - `filter_buttons_set_query_tokens`
  - `install_button_runs_and_shows_progress`
  - `install_during_describe_is_queued`
  - `failed_install_shows_error_and_log_button`
  - `theme_install_offers_set_theme_toast`
  - `update_badge_count`
  - `extensions_tab_restored_from_workspace_state`
  - `old_sidebar_state_extensions_restores_explorer`
  - `extensions_viewlet_focus_when_tab_focused`
  - `install_quick_open_is_capped_and_shows_loading`
  - `clear_build_cache_refused_while_installing`

  Reuse the fake store setup from Task 15's tests.
- [ ] **Step 2: Run** `cargo test -p lucerna-workbench --test extensions_page`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test --workspace`. Expected: PASS.
- [ ] **Step 5: Run the app** (debug build) if a screenshot is possible; otherwise say so. Never edit the user's workspace state or settings files outside the repo.
- [ ] **Step 6: Commit** with message `feat(workbench): Extensions page in an editor tab (Zed-style)`.

### Task 16: Benchmarks, notices, manual acceptance

**Files:**
- Modify: `crates/lucerna-syntax/benches/highlight.rs`. Add `wasm_json_vs_native` (record-only) and `html_large_script_injection` / `markdown_many_fences` (with the 15% gate).
- Modify: `THIRD_PARTY_NOTICES.md` (wasmtime and cranelift as Apache-2.0 WITH LLVM-exception, the `json.wasm` fixture, and the injection query sources)
- Modify: `docs/superpowers/specs/2026-11-09-lucerna-phase2-extensions-design.md`, setting the status to "Implemented" and adding a short "Measurement results" section
- Modify: `README.md`, adding a short "Extensions" section that covers `luc ext`, the panel, and the Xcode CLT requirement

- [ ] **Step 1: Run the benchmarks** with `cargo bench -p lucerna-syntax` and record the numbers in the spec.
- [ ] **Step 2: Check performance in a debug build.** Run `luc --trace-startup .` and confirm `startup_ms` has not regressed by more than 15% against `main` before Phase 2, measured in a debug build on the same machine. The release-build target is checked only by the user.
- [ ] **Step 3: Manual acceptance against spec §1.2**, on the real registry:
  - `luc ext install catppuccin` (theme), then set it as the theme from the toast.
  - Install an icon theme (for example `material-icon-theme`) and select it with Preferences: File Icon Theme.
  - Install `zig` and open a `.zig` file, which should be coloured.
  - Install `vue` and open a `.vue` file. The `<script>` and `<style>` blocks should be coloured.
  - Uninstall each of them; none needs a restart.

  Record the results, including any extension that failed and why, in the spec's "Measurement results" section.
- [ ] **Step 4: Run** `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` and `cargo deny check`. Expected: all green.
- [ ] **Step 5: Commit** with message `docs: Phase 2 measurements, notices and README`.
