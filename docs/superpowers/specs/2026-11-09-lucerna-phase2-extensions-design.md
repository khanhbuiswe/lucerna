# Lucerna — Phase 2: Zed Extension Host

- **Date:** 2026-11-09
- **Status:** Implemented (the visual checks in the Measurement results section are still waiting for the user to confirm)
- **Previous spec:** `2026-11-08-lucerna-phase1-design.md` (roadmap in §1.3, UI in §12–§13)
- **Background research:** `docs/research/2026-11-09-extensions.md` (Zed and VS Code compared)

## 1. Goals

### 1.1 Outcome

Users can install extensions from Zed's registry right inside Lucerna, and what an extension brings is usable immediately, with no restart:

- themes;
- icon themes;
- languages, including tree-sitter grammars and queries;
- snippets: installed and validated, but not used yet. Snippet insertion belongs to Phase 4.

Extensions are built by Lucerna itself from the `zed-industries/extensions` repo. Lucerna does **not** call `api.zed.dev`. Zed has public terms (https://zed.dev/terms): §2.2 only allows access through mechanisms Zed designates, §2.4(b) forbids access to non-public APIs, and the extension endpoint of `api.zed.dev` has no public documentation. Details in `docs/research/2026-11-11-lsp-server-table.md` §5.

### 1.2 Success criteria

1. From the Extensions panel or with `luc ext install`, the following three kinds of extension can be installed and used, with no restart:
   - a theme (Catppuccin);
   - an icon theme (for example `material-icon-theme`);
   - a language with its own grammar (Zig).
2. A language extension that needs injection, such as Vue or Svelte, gets colored for the embedded parts too: `<script>`, `<style>`, expressions in the template.
3. All the Phase 1 performance targets (§6) are still met.
4. A broken extension, whether at install time or at run time, does not crash or hang the editor.

### 1.3 What was decided during brainstorming

| # | Decision | Reason |
|---|---|---|
| D1 | **No WASM component host yet** (WIT, `extension.wasm`, host functions) in this phase. Left for Phase 3b. | The four things in scope are all data. Only language servers need to run the extension's code. |
| D2 | **Grammars from extensions are compiled to WASM** with wasi-sdk and run through tree-sitter's `WasmStore` on wasmtime. Bundled grammars stay native. | Sandboxed: a broken grammar only causes a trap and does not crash the app. Zed does the same. Phase 3 still needs wasmtime. |
| D3 | **An Extensions panel in the app**, plus a `luc ext …` CLI command. | A VS Code-like experience; the CLI is handy for scripts and tests. |
| D4 | **Snippets** are installed and validated, but not used. | Lucerna stays read-only until Phase 4. |
| D5 | **Updates:** check periodically on its own, the user clicks to update. | No silently downloading and building code when the user did not ask. |
| D6 | **General injection** following Zed's conventions. | About a third of language extensions need injection. |
| D7 | **Priority on conflicts:** the user, then extensions, then built-in. When two extensions collide, the extension whose id is greater alphabetically wins, with a warning in the log. | Follows the general direction of Zed and VS Code, but the rule is stated explicitly and deterministic. |
| D8 | **Fetch data with the `git` and `curl` already on macOS.** Add no network or git crate. | Lean. Like Zed's `extension_builder`. Works with any git host. |
| D9 | **The registry has only id and version.** Name and description come from the manifest when the extension is installed or when the user opens that entry. | Avoids fetching more than 1,590 repos. |

### 1.4 Not in Phase 2

- The WASM host and host functions (`download-file`, `process`, `nodejs`, `github`…), capability prompts, language servers, slash commands, context servers. Whatever belongs to Phase 3 is done in Phase 3; AI features are never done.
- Snippet insertion.
- The `outline`, `indents`, `textobjects`, `runnables`, `overrides`, `redactions`, `debugger` queries. They are copied at install time but not used yet.
- Grammars with a scanner written in C++ (`scanner.cc`). Zed does not support them either.
- VS Code extensions, Open VSX, the Microsoft Marketplace.
- Enabling or disabling extensions per workspace, extension recommendations for a workspace.

## 2. Architecture

### 2.1 New crate: `lucerna-extension`

Pure Rust, with no dependency on GPUI, `lucerna-syntax` or `lucerna-theme`. This crate only returns **data**: the paths and descriptions of what is installed. Loading that data into the registries is done by `lucerna-workbench` (or the CLI).

| Module | Responsibility |
|---|---|
| `manifest` | Reads `extension.toml` (schema 0 and 1) and the legacy `extension.json`. Fills in default paths from the directory layout (§3.2). Reads each language's `config.toml` (§3.3). |
| `registry` | Clones and fetches the `zed-industries/extensions` repo. Combines `extensions.toml`, `.gitmodules` and `git ls-tree` into a list of `RegistryEntry`. Caches it to `registry.json`. |
| `process` | Runs child processes (`git`, `curl`, `tar`, clang) with a timeout, cancellable, keeping stderr. |
| `toolchain` | Ensures wasi-sdk 34 is present: downloads it, checks the SHA-256, extracts it. Prefers `WASI_SDK_PATH` if this variable points to a real `bin/clang`. |
| `builder` | Fetches extension code, fetches and compiles grammars, checks resources, builds the directory in `staging/`. |
| `validate` | Validates the JSON of themes, icon themes and snippets; validates the snippet body syntax per TextMate. Grammar loading and query compilation are called through a `GrammarValidator` trait, supplied by the caller (§2.3). |
| `store` | Manages installed extensions: `install`, `uninstall`, `update`, `index()`. Builds `index.json`. Emits events. Each id has only one operation at a time. |

### 2.2 Existing crates that are extended

- **`lucerna-syntax`**
  - Enable tree-sitter's `wasm` feature (which pulls in `wasmtime-c-api-impl` 48).
  - `Language` gains an `Extension` origin.
  - Add two new things: general injection (§4.3) and `files.associations` (§4.2).
  - Provide `WasmGrammarValidator`, an implementation of `GrammarValidator`.
- **`lucerna-theme`**
  - `ThemeRegistry` loads themes in layers (§4.4).
  - Add `IconTheme` and `IconThemeRegistry` (§4.5).
  - The existing Lucide icon table becomes the built-in icon theme named "Lucide". The mapping data moves to `lucerna-theme`; the SVG bytes stay in `lucerna-workbench`.
- **`lucerna-settings`:** add the new keys in §6.
- **`lucerna-workbench`**
  - The Extensions panel and a button on the status bar.
  - Swap the registries on an `ExtensionsChanged` event.
  - Draw icon themes with `img()`.
- **`lucerna` (bin `luc`):** add the `luc ext` subcommand.

### 2.3 Interfaces between crates

- **`extension::Store`**
  - `open(root) -> Store`
  - `index() -> Arc<ExtensionIndex>`
  - `install(id, progress: impl Fn(InstallProgress), cancel: CancelToken) -> Result<InstalledExtension, InstallError>`
  - `uninstall(id)`, `update(id)`
  - `install_dev(dir)`
  - `subscribe() -> Receiver<StoreEvent>`
- **`ExtensionIndex`** holds, for each extension:
  - the fully filled-in manifest;
  - the commit;
  - a list of `LanguageEntry { name, extension_id, dir, grammar: Option<GrammarRef { name, wasm_path }>, matcher: { path_suffixes, first_line_pattern }, hidden, queries: QueryFlags }`;
  - a list of `ThemeEntry { name, appearance, extension_id, path }`;
  - a list of `IconThemeEntry { name, appearance, extension_id, path, icons_root }`;
  - snippets: the count per language.
- **`trait GrammarValidator`**
  - `fn load(&self, name, wasm: &[u8]) -> Result<(), String>`
  - `fn check_query(&self, name, wasm, kind, source) -> Result<(), String>`

  `lucerna-syntax` provides the real implementation. Tests use a fake one.
- **`syntax::LanguageRegistry::build(bundled, extensions: &[LanguageEntry], associations: &Associations) -> LanguageRegistry`**
- **`theme::ThemeRegistry::build(builtin, extension_themes, user_dir)`**, **`theme::IconThemeRegistry::build(builtin_lucide, extension_icon_themes)`**

## 3. Installation

### 3.1 Layout on disk

Everything lives in `~/Library/Application Support/Lucerna/extensions/`:

```
registry/                 shallow clone of zed-industries/extensions
registry.json             cache of the RegistryEntry list + fetch time
toolchain/wasi-sdk-34/    extracted wasi-sdk
work/<id>/src/            extension code at the pinned commit (cache, deletable)
work/<id>/grammars/<g>/   grammar code at the pinned rev (cache, deletable)
staging/<id>-<uuid>/      being built
installed/<id>/
  extension.toml          fully filled-in manifest
  lucerna.json            { commit, version, installed_at, source: "registry" | "dev" }
  grammars/<g>.wasm
  languages/<dir>/{config.toml,*.scm}
  themes/*.json
  icon_themes/*.json
  icons/**
  snippets/…              keeps the relative paths as in the manifest
index.json                ExtensionIndex
logs/<id>.log             log of the most recent install
```

### 3.2 Manifest

- **`extension.toml`**
  - **Required:** `id`, `name`, `version`, `schema_version`.
  - **Optional:**
    - `description`, `repository`, `authors`;
    - `lib`;
    - `themes[]`, `icon_themes[]`, `languages[]`;
    - `grammars.<name> = { repository, rev (or commit), path? }`;
    - `snippets` (a string or an array);
    - `language_servers`, `context_servers`, `slash_commands`, `capabilities`… (kept in raw form for Phase 3).
  - Unrecognized keys are ignored, with no error.
- **Filled in automatically when missing:**
  - `languages/<dir>/` that has a `config.toml`;
  - `themes/*.json`;
  - `icon_themes/*.json`;
  - `snippets.json` at the root.

  Explicitly declared entries are kept as they are; entries found automatically are added if not already present.
- **Schema 0:** delete `languages`, `grammars` and `themes` and rebuild them from the directories. Also read `grammars/*.toml` (`repository`, `commit`, `path?`).
- **Legacy `extension.json`:** the id is taken from the directory name, and the schema is treated as 0.
- **Reject** when `schema_version` is greater than 1, or when a grammar name is not snake_case.

### 3.3 A language's `config.toml`

The fields used:

- `name`;
- `grammar`;
- `path_suffixes`: matches the extension, the full file name, or the `.<suffix>` tail;
- `first_line_pattern`;
- `line_comments`, `block_comment`;
- `brackets`;
- `hidden`;
- `tab_size`.

Other fields are ignored.

**Query files accepted:** `highlights`, `injections`, `brackets`, `outline`, `indents`, `textobjects`, `overrides`, `redactions`, `runnables`, `debugger` (with the `.scm` extension). A `.scm` file not in this list is ignored and a warning is logged.

### 3.4 Refreshing the registry

1. **First time:** `git clone --depth 1 --filter=blob:none --no-checkout https://github.com/zed-industries/extensions registry/`.
   **Later times:** `git -C registry fetch --depth 1 origin main`, then `git -C registry update-ref HEAD FETCH_HEAD`.
2. Read three sources and join them by id:
   - `git show HEAD:extensions.toml` for `[<id>] submodule, version, path?`;
   - `git show HEAD:.gitmodules` for the URL per submodule;
   - `git ls-tree HEAD extensions/` for the commit per submodule.

   An entry missing its URL or commit is skipped and a warning is logged.
3. Write `registry.json` with `fetched_at` and the list of `RegistryEntry { id, version, url, commit, path }`. `name` and `description` are filled in when available (D9).
4. **When it runs:**
   - when the panel opens and `fetched_at` is more than 24 hours old;
   - at startup, if `extensions.autoCheckUpdates` is on and more than 24 hours have passed;
   - when the user runs the "Extensions: Check for Updates" command.

**Getting an entry's description** (D9): when the user expands a row that is not installed, Lucerna shallow-fetches that repo at the commit into `work/<id>/src`. This step is also the first step of installing, so no work is wasted. It then reads `extension.toml` and saves `name` and `description` into `registry.json`.

### 3.5 Installing an extension

**Pre-checks:**
- `<id>` is found in the registry.
- `git --version` runs.
- The disk has more than 1.2 GB free.
- No other operation is running on the same id.

**Steps**, each emitting `InstallProgress`:
1. **`Fetching`:**
   - Run `git init`, `git fetch --depth 1 <url> <commit>` and `git checkout FETCH_HEAD` in `work/<id>/src`.
   - If the directory already exists at the right commit, skip this step.
   - Apply `path`.
2. **`ReadingManifest`:**
   - Read and fill in the manifest.
   - `version` must match the version in the registry.
   - Record the unsupported parts to show on the panel.
3. **`PreparingToolchain`**, runs only when the extension has grammars:
   - Download `https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-34/wasi-sdk-34.0-<arm64|x86_64>-macos.tar.gz` with `curl -fL`.
   - Compare the SHA-256 with the constant pinned in the code.
   - Extract with `tar` into `toolchain/`.
4. **`BuildingGrammar(name, i, n)`:** for each grammar:
   - Run `git init`, `git fetch --depth 1 <repository> <rev>` and `git checkout` into `work/<id>/grammars/<name>`.
   - Apply `path`. If `src/parser.c` is missing, report an error. If `src/scanner.cc` exists without a `scanner.c`, report the error "C++ scanner is not supported".
   - Compile:
     ```
     <wasi-sdk>/bin/clang -fPIC -shared -Os -Wl,--export=tree_sitter_<name> \
       -Wl,--max-memory=4294967296 -o <staging>/grammars/<name>.wasm \
       -I src src/parser.c [src/scanner.c]
     ```
   - There is a cache: if `work/<id>/grammars/<name>.wasm` already exists, with the same rev and newer than the source, copy that file instead of compiling.
5. **`Validating`:**
   - Load each grammar with `GrammarValidator::load`.
   - Compile each `.scm` file with its grammar.
   - Read the JSON of themes and icon themes. Check that every icon `path` is inside the extension directory.
   - Read the snippets.
   - A resource that fails is dropped on its own and recorded in `warnings`. A language whose grammar fails is dropped entirely.
   - If no usable resource is left after filtering, the install fails with a `NothingUsable` error.
6. **`Installing`:**
   - Copy the resources into `staging/<id>-<uuid>/` following the layout in §3.1.
   - Write `extension.toml` (the fully filled-in version) and `lucerna.json`.
   - If an older install exists, rename `installed/<id>` to `installed/.old-<id>-<uuid>`.
   - `rename` staging to `installed/<id>`.
   - Rebuild `index.json` and write it atomically: write to a temp file then `rename`.
   - Delete the `.old-…` directory in the background.
   - Emit `StoreEvent::Changed`.
7. **`Done { warnings }`**, or **`Failed(InstallError)`**. On failure, Lucerna deletes `staging/` and keeps the old install as it was.

**Cancel:** `CancelToken` kills the running child process, deletes `staging/`, then returns `Failed(Cancelled)`.

**Timeout:** each `git` or `curl` process gets 120 seconds; each grammar compile gets 300 seconds.

**Log:** all stdout and stderr of the child processes are written to `logs/<id>.log`, overwritten on each install.

### 3.6 Updating, removing, installing from a local directory

- **An update is available** when the registry has a different `version` or a different `commit` compared with the installed copy's `lucerna.json`. Updating goes through exactly the flow in §3.5.
- **Remove:**
  - Rename `installed/<id>` to `installed/.trash-<id>-<uuid>`.
  - Rebuild the index, emit the event.
  - Delete the `.trash-…` directory in the background.
  - `work/<id>` is kept as a cache.
- **Startup:** delete any leftover `.old-*`, `.trash-*` and `staging/*` directories.
- **`luc ext install --dev <dir>`:** build from `<dir>` per §3.5, skipping step 1 and the version check against the registry, then copy into `installed/<id>` with `source: "dev"`. Lucerna does not use symlinks. To update, run the command again.
- **"Extensions: Clear Build Cache":** deletes `work/`.

### 3.7 Index

- `index.json` is rebuilt from `installed/*`, skipping entries whose names start with `.`.
- For each extension, Lucerna reads `extension.toml`, `lucerna.json`, each language's `config.toml` (taking the matcher, grammar and query flags), the name and `appearance` of themes and icon themes, and the snippet count per language.
- **At startup:**
  - read `index.json`;
  - if the file is missing, corrupt, or the mtime of `installed/` is newer than the file, rebuild it in the background;
  - while waiting, the UI uses the old index or an empty one.
- A broken install directory (missing or faulty `extension.toml`) is marked `broken` in the index, with the reason.
- **The watcher** for `installed/` is a separate `notify` watcher, debounced at 200 ms. It sits outside the worktree, so it does not violate R21. Thanks to the watcher, a `luc ext` command run in another terminal also makes an already-open window update.

## 4. Runtime integration

### 4.1 WASM grammars

- One global `wasmtime::Engine`, created lazily the first time it is needed.
- Each `SyntaxState`, and each parser of an injection layer, uses one `Parser`. A `WasmStore` is only attached to a parser when its grammar is WASM.
- Grammars load lazily. The first time a file of that language is opened, Lucerna reads the `.wasm` file and calls `load_language` on a background thread, then caches the `tree_sitter::Language` in `GrammarCache` keyed by `(extension_id, grammar_name)`.
- **Namespace separation:** a grammar name only has meaning within its own extension. Two extensions that both have a `toml` grammar do not overwrite each other.
- **Errors:**
  - A load error, or a trap during parsing (tree-sitter returns an error or no tree): that language is marked `failed` until the next startup.
  - The file is shown without coloring and a log entry is written.
  - Panics are caught with `catch_unwind` (R3).

### 4.2 Choosing a language for a file

`LanguageRegistry` has three layers: bundled, extension, associations. The order of consideration:

1. **`files.associations`:**
   - Apply VS Code-style globs to the file name. If the pattern contains `/`, apply it to the relative path.
   - The value is a language id (for example `rust`, `typescriptreact`) or an extension language's name, case-insensitive.
   - If several globs match, the longest glob wins.
2. **Exact file name**, with the extension layer considered before the bundled layer.
3. **Longest suffix:** compare the extension and the `.<suffix>` tails by distance. The extension layer is considered before the bundled layer. If two extensions match at the same length, the extension whose id is greater alphabetically wins, and Lucerna logs a warning once.
4. **`first_line_pattern`:** matched against the first line, at most 256 bytes. Considered only when the steps above give no result, and applies only to languages from extensions.

**Hidden languages:** a language with `hidden = true` does not take part in steps 2–4. It can only be used for injection and for `files.associations`.

**Language ids:**
- A language from an extension uses its lowercased name as the id, with spaces replaced by `-`. For example `Zig` becomes `zig`.
- This id is the value of the `editorLangId` context key and of the `"[zig]"` override key.
- If the id collides with a bundled language, the extension's language replaces it (D7).

### 4.3 Highlighting and injection

**Precedence**
- Languages from extensions use `LastPatternWins`.
- A capture only counts if `Theme::style_for` returns a style. This keeps Zed's `@a @b` rule (try `@b` first) and the skipping of captures with a `_` prefix correct.
- Bundled languages keep the current precedence.

**Predicates**
- The standard predicates are handled by tree-sitter.
- Add `#has-parent?` and `#not-has-parent?`.
- Unknown predicate: that pattern is treated as not matching, and Lucerna logs a warning once.

**Injection**
- Read `injections.scm`:
  - `@injection.content` (or `@content`);
  - `@injection.language` (or `@language`);
  - `#set! injection.language "<name>"`;
  - `#set! injection.combined`.
- **Language name lookup:**
  - Case-insensitive, by language name, then id, then path suffix.
  - If the string contains a `.`, take only the part after the last dot. For example `foo.rs` becomes `rs`.
- **Layer tree:**
  - `SyntaxState` holds the root layer and the child layers.
  - Each child layer has a language, byte ranges (`included_ranges`) and its own `Tree`.
  - Several `combined` regions are merged into one layer.
  - At most 4 levels, and at most 1,000 child layers per file. Past the limit, no more injection happens.
- **Parse:**
  - After the parent layer is parsed, Lucerna runs `injections.scm` over the whole parent tree, then parses the child layers.
  - When the file changes, the child layers are rebuilt.
  - All of this is within the same background parse pass as today, and shares the 1 ms synchronous budget.
- **Highlight:**
  - For the lines being displayed, Lucerna runs each layer's `highlights.scm` for the layers that intersect that area.
  - The spans are merged; the deeper layer wins where they overlap.
- **Injected language not yet available:** that region is left blank. When the registry changes, the file is re-parsed.
- **Markdown:**
  - The hand-written inline mechanism from Phase 1 is dropped, replaced by the `injections.scm` of `tree-sitter-md`: inline and code fences.
  - The injection queries of bundled grammars come from the grammar crate if the crate exports them (`INJECTIONS_QUERY`), otherwise from the upstream repo, with a note in `THIRD_PARTY_NOTICES.md`.

**Bracket pairs**
- If a language has `brackets.scm` (requiring `@open` and `@close`), Lucerna uses this query to find bracket pairs.
- If not, Lucerna uses the Phase 1 algorithm.

### 4.4 Themes

- **Load order into `ThemeRegistry`:** built-in (One), then the themes of extensions in ascending id order, then `~/.config/lucerna/themes/*.json`. The layer loaded later wins on a name collision. Lucerna logs a warning when two extensions have the same theme name.
- Theme JSON is read per Zed's v0.2.0 schema, as in Phase 1, in lenient mode: comments and trailing commas are accepted.
- **When `workbench.colorTheme` points to a theme that does not exist yet:**
  - Lucerna uses One Dark but keeps the setting as it is.
  - Every time the registry is rebuilt, Lucerna re-applies the theme in the setting. So right after installing an extension, the theme takes effect.

### 4.5 Icon themes

- **Schema** follows Zed's v0.3.0: `{ name, author, themes: [{ name, appearance, directory_icons, named_directory_icons, chevron_icons, file_stems, file_suffixes, file_icons }] }`.
- **Icon paths** are resolved from `installed/<id>/`. Any path that goes outside that directory is rejected.
- **Lookup order for a file:**
  1. the full file name;
  2. the compound suffixes from longest to shortest (`a.b.c` tries `b.c`, then `c`);
  3. the part before the first dot;
  4. `default`.

  At each step, look up `file_stems` first, then `file_suffixes`. The result is a type, then `file_icons[type].path`. When missing, fall back to the "Lucide" icon theme.
- **Directories:** look up `named_directory_icons[name]`, then `directory_icons`, then Lucide. Chevrons work the same way.
- **Built-in "Lucide":** the current `icon_for_path` table turned into icon theme data. The Explorer only asks `IconThemeRegistry`.
- **Drawing:**
  - Lucide icons use `svg()` and the `icon.muted` color, as today.
  - Icons from extensions use `img(path)` to keep the SVG's original colors, at the same size as the current icons.
- **Setting:** `workbench.iconTheme`, default `"Lucide"`. If the value points to an icon theme that does not exist yet, Lucerna uses Lucide but keeps the setting, as in §4.4.

### 4.6 Snippets

- At install time, Lucerna reads each snippet file, in the format `{ "<name>": { prefix?, body, description? } }` (a string or an array is accepted), and validates the body syntax per TextMate (`$1`, `${1:x}`, `${1|a,b|}`, `$0`, variables).
- The language of a snippet file is taken from the file's stem, lowercased: `snippets` means shared, any other name is a language name.
- `index.json` stores the number of valid snippets per language. Lucerna has no runtime for snippets yet.

### 4.7 Swapping registries

- `workbench` receives `StoreEvent::Changed` through a background task reading from `Store::subscribe()`.
- On a background thread, it rebuilds `LanguageRegistry`, `ThemeRegistry` and `IconThemeRegistry` from the new index, then on the UI thread replaces the `Arc`s in `AppState`.
- After the swap:
  - each open tab re-resolves its language, and re-parses if it differs;
  - the theme and icon theme are re-applied per the settings;
  - the Explorer redraws.
- No restart is needed.

## 5. UI

### 5.1 Status bar

> Change 2026-11-09: the user chose to make Extensions a **dedicated page in an editor tab**, like Zed, instead of a panel in the sidebar. Sections 5.1 and 5.2 below are the corrected version.

- Add a third button at the left corner, with the Lucide `blocks` icon, right after Explorer and Search. Clicking it opens (or focuses) the Extensions tab. This button does not toggle the sidebar.
- The button has a number badge when updates are available, and a spinner while installing.
- Shortcut: Cmd+Shift+X, command `workbench.view.extensions`.

### 5.2 Extensions page

- **A single tab** named "Extensions", opened in the active editor group. If the tab is already open in any group, just focus it. The tab is not a preview tab. The tab is saved in the workspace state and reopened on restore.
- When the tab is opened or focused, the cursor sits in the search box.
- The context key `extensionsViewletFocus` is true when the Extensions tab has focus.
- **Top of the page:**
  - A large search box. Fuzzy match with `nucleo-matcher` over the id, plus the name and description if already known.
  - A row of filter buttons: **All · Installed · Updates · Themes · Icon Themes · Languages**. `@installed`, `@updates`, `@themes`, `@icons`, `@languages` can still be typed. The last three filters only apply to installed extensions, because the registry has no information about what extensions provide.
- **List of cards**, virtualized (GPUI `list`, variable height). Installed extensions come first, then the entries not installed. Each card has:
  - line 1: the name (or the id if the name is not known), version, the id in faint text;
  - line 2: the description. For an entry not installed, the description is fetched per §3.4 the first time the card appears on screen; while fetching it shows "Loading description…";
  - line 3: what the extension provides, for example "2 themes · 1 language · 14 snippets (Phase 4)", with "Language server: Phase 3b" if present; for an entry not installed, the repository and the abbreviated commit;
  - on the right: an Install / Update / Uninstall / Reinstall button, or Cancel while installing;
  - while installing: the current step, for example "Building grammar 2/3: zig";
  - when done with warnings: the list of warnings;
  - on error: a short error in the `status.error` color and a "Show Log" button, which opens `logs/<id>.log` in a preview tab.
- Clicking Install while the description of the same id is being fetched: the install is queued to run right after the description fetch finishes, and the button switches to the installing state.
- **Empty states and general errors:**
  - no registry yet: "Loading list…";
  - `git` missing: instructions to run `xcode-select --install`;
  - registry fetch fails: a message with a Retry button, and the installed extensions are still shown.
- **When an extension with a theme or icon theme finishes installing:** a small notification at the bottom right asks "Use theme X?" with a "Set Theme" button. This button writes to the user settings.
- Zed-like style: 4 px grid, hover background is `ghost_element.hover`, cards have a 1 px `border.variant` border, 6 px rounded corners.

### 5.3 Commands

| Command ID | Display name |
|---|---|
| `workbench.view.extensions` | View: Show Extensions |
| `workbench.extensions.action.installExtension` | Extensions: Install Extension… (Quick Open lists the registry) |
| `workbench.extensions.action.checkForUpdates` | Extensions: Check for Updates |
| `workbench.extensions.action.updateAllExtensions` | Extensions: Update All |
| `lucerna.extensions.clearBuildCache` | Extensions: Clear Build Cache |
| `workbench.action.selectIconTheme` | Preferences: File Icon Theme |

The theme picker (`workbench.action.selectTheme`) also lists themes from extensions. The icon theme picker uses the same widget, with a preview as the selection moves.

### 5.4 CLI

```
luc ext search <query>
luc ext list [--installed] [--updates]
luc ext install <id>... | --dev <dir>
luc ext remove <id>...
luc ext update [<id>...]
```

- The `luc ext` command runs without the GUI, using `lucerna-extension` and `WasmGrammarValidator`.
- Progress is printed to stderr, one line per step.
- The exit code is non-zero on error.

## 6. New settings

| Key | Default | Meaning |
|---|---|---|
| `workbench.iconTheme` | `"Lucide"` | The icon theme name |
| `files.associations` | `{}` | Glob → language id or name, like VS Code |
| `extensions.autoCheckUpdates` | `true` | Check the registry periodically (24 hours) |

New context key for `when` clauses: `extensionsViewletFocus`.

## 7. Error handling

| Situation | Handling |
|---|---|
| Missing `git`, no network, fetch over 120 s | The install fails, `staging/` is deleted, the panel shows the error, the log is at `logs/<id>.log` |
| Disk has less than 1.2 GB free | Blocked before downloading or building |
| wasi-sdk has the wrong SHA-256 | Delete the downloaded file, report an error, never run it |
| Grammar fails to compile or has only `scanner.cc` | Drop the languages that use that grammar, the rest is still installed, the panel shows a warning |
| A query cannot be compiled | Drop that query file, with a warning. If it is `highlights.scm`, the language is still installed but has no color. |
| An icon points outside the extension directory | Drop that icon, with a warning |
| Manifest version differs from the registry, or `schema_version` is greater than 1 | The install fails |
| Grammar fails when loading or parsing | The language is marked `failed`, the file is shown without coloring, a log entry is written |
| `index.json` corrupt | Rebuild from `installed/*` |
| A directory in `installed/` is broken | Mark it `broken` in the index, the panel has a Reinstall button |
| `registry.json` corrupt | Delete it and fetch again |
| Panic in an install or registry-building task | `catch_unwind` (R3), switch to `Failed`, the editor keeps running |
| Two operations on the same id | The later operation is rejected with `Busy` |

## 8. Performance

- **Startup** only reads `index.json`, and opens no `.wasm` or `.scm`. The target of a first frame under 200 ms still holds.
- **The first load of a WASM grammar** (Cranelift compiling) runs in the background. The file shows its text first, color comes later.
- **Wasmtime** is only initialized the first time a WASM grammar is needed. The idle RAM target of under 150 MB still holds.
- **Injection** only computes highlights for the lines being displayed. Add benchmarks for HTML with a large `<script>` and Markdown with many code fences, with the same 15% regression threshold as the other benchmarks.
- **New benchmark:** parse and highlight a WASM grammar, compared with the native version of the same grammar (using a JSON fixture). Recorded only, no threshold set.

## 9. Licensing

- Add `Apache-2.0 WITH LLVM-exception` to `deny.toml` (wasmtime, cranelift and related crates).
- wasi-sdk is downloaded at run time, not bundled. Lucerna does not distribute extensions; users build them on their own machine.
- **No Zed GPL code is used.** Only data formats (`extension.toml`, `config.toml`, `.scm` files, the schemas of themes and icon themes) are read, as an interface. References to Zed's source code in the research documents are only for understanding behavior.
- The injection queries of bundled grammars come from the grammar crates or their upstream repos, recorded in `THIRD_PARTY_NOTICES.md`.
- The `.wasm` file used as a test fixture is built from an MIT grammar, with the license file in `tests/fixtures`.

## 10. Testing

- **Unit tests:**
  - `lucerna-extension`:
    - manifest schema 0 and 1, the legacy `extension.json`, auto-filled paths, invalid grammar names;
    - joining the registry from three sources;
    - comparison to detect updates;
    - validating the JSON of themes, icon themes and snippets;
    - blocking icon paths that go outside the directory;
    - building `index.json`.
  - `lucerna-syntax`:
    - the language selection order, including `files.associations`, file name, longest suffix, tied ids, `first_line_pattern`, `hidden`;
    - `LastPatternWins` combined with skipping captures that have no style;
    - `#has-parent?`.
  - `lucerna-theme`:
    - the override order between theme layers;
    - the icon theme lookup order and fallback to Lucide.
- **`lucerna-extension` integration tests, run offline:**
  - Fake git repos in a temp directory, accessed via `file://` URLs: a registry repo with `extensions.toml`, `.gitmodules` and a submodule; a theme extension; a language extension with a grammar pointing to a fake grammar repo.
  - Clang is replaced by a fake script (via `WASI_SDK_PATH`) that only copies the `.wasm` fixture file.
  - Check: install, update, uninstall, cancelling midway, errors at each step, reinstalling when `broken`, cleaning up `.old` and `.trash` at startup.
- **WASM grammars:**
  - The grammar loading test uses a prebuilt `.wasm` fixture, committed in the repo. This test does not need wasi-sdk.
  - The test that builds with real clang is `#[ignore]` and only runs when `WASI_SDK_PATH` is set.
- **Injection:** snapshot tests with `insta` for:
  - HTML with `<script>` and `<style>`;
  - Markdown with inline and code fences; this case must give the same result as the Phase 1 snapshot for the inline part;
  - one `injection.combined` case;
  - the depth limit.
- **UI** with `TestAppContext`:
  - the panel's filtering and search;
  - the button state for each `InstallProgress`;
  - the update badge;
  - swapping registries makes an open tab recolor;
  - changing the icon theme makes the Explorer redraw;
  - a setting pointing to a theme that does not exist yet is applied when the theme appears.
- **CLI:** tests with `assert_cmd` for `luc ext list`, `luc ext install --dev`, and the error when the id does not exist.
- **Manual**, before closing the phase: the criteria in §1.2, checked against the real registry.

## 11. Risks

| Risk | Mitigation |
|---|---|
| Wasmtime increases build time and the size of `target/` (about 1 GB), while the machine only has about 12 GB left | No release builds during development; `profile.dev` keeps `debug = false` for dependencies; check free space before heavy steps |
| Tree-sitter 0.27 with the `wasm` feature may not support things exactly as Zed uses it (Zed uses a git rev of tree-sitter) | The first task of the plan is a spike: load the `.wasm` fixture with `WasmStore` on tree-sitter 0.27. If it does not work, stop and report |
| The ABI version of a grammar in an extension is outside the range tree-sitter 0.27 supports | `load_language` reports a clear error; the language is dropped with a warning at install time |
| GPUI's `img()` cannot draw SVGs from a file path, or draws them slowly | Check in the plan; if needed, rasterize the SVG once and cache it as an image |
| An extension's queries use Zed-specific predicates or conventions that Lucerna does not support yet | A pattern with an unknown predicate is treated as not matching and a warning is logged; monitor with Catppuccin, Zig, Vue, Svelte |
| The registry changes the `extensions.toml` format | Read leniently, skip faulty entries with a warning, with tests using saved real data as fixtures |
| Injection slows down large files | Limit of 4 levels and 1,000 child layers; turned off entirely above the large-file threshold as in Phase 1 |

## 12. Expected implementation order

1. Spike: tree-sitter `wasm` with a fixture grammar; `deny.toml`.
2. `lucerna-extension`: manifest, then registry, process and toolchain, builder and validate, and finally store and index.
3. `lucerna-syntax`: WASM grammars and the extension language layer, `files.associations`, precedence, then general injection (including moving Markdown to the new mechanism) and `brackets.scm`.
4. `lucerna-theme`: layered themes, icon themes, Lucide as data.
5. `lucerna`: the `luc ext` command.
6. `lucerna-workbench`: registry swapping, icon theme drawing, the Extensions panel, commands and settings.
7. Benchmarks, manual testing per §1.2.

## 13. Measurement results

Measured on 2026-11-09 on the development machine (macOS, arm64). Benchmarks were run with `cargo bench -p lucerna-syntax` (bench profile, optimized); startup was measured with the debug build.

### 13.1 Benchmarks

The numbers are criterion's middle value (`[low  middle  high]`).

| Benchmark | Result | Notes |
|---|---|---|
| `highlight_10mb_viewport/rows_60` | 96.5 µs | Phase 1 (commit `8bd8467`): 97.4 µs. Difference −0.9%, within the 15% threshold. |
| `highlight_markdown_viewport/rows_60` | 82.8 µs | A bench present since Task 4, with no Phase 1 version to compare against (see below). |
| `markdown_many_fences/rows_60` | 59.5 µs | 800 code fences (rust and json), 1 viewport of 60 lines in the middle. |
| `html_large_script_injection/rows_60` | 472.6 µs | An 8000-line `<script>` in HTML, viewport in the middle. |
| `wasm_json_vs_native/native` | 21.5 ms | Parse + highlight the first 60 lines of a 4000-entry JSON, bundled grammar. |
| `wasm_json_vs_native/wasm` | 31.6 ms | Same data, WASM grammar (`json.wasm`). About 1.47 times slower. Recorded only, no threshold. |

On the 15% regression threshold: in Phase 1 `SyntaxState::new` did not yet take a registry, so there was no injection; therefore the three Markdown/HTML benches cannot be compared with the commit before Phase 2. Only `highlight_10mb_viewport` (the path without injection) can be compared, and it passes. The three injection benches set the baseline for later runs.

### 13.2 Startup (debug build, `luc --trace-startup .`)

| Build | `startup_ms` |
|---|---|
| Before Phase 2 (`8bd8467`), first cold run | 553 |
| Before Phase 2, 5 warm runs | 154, 160, 166, 180, 184 (median 166) |
| Phase 2 (main), first cold run | 236 |
| Phase 2, 9 warm runs | 157, 158, 161, 166, 166, 170, 170, 171, 175 (median 166) |

The warm medians are equal (166 ms), no regression. The release build target (under 200 ms) is for the user to check.

### 13.3 Acceptance per §1.2 via the CLI

Run on the real registry, but in an isolated directory (`LUCERNA_EXTENSIONS_DIR` pointing to a temp directory, `WASI_SDK_PATH` pointing to a pre-downloaded wasi-sdk 34), so the user's real extensions directory was not touched.

| Extension | Version | Result | Content in `index.json` |
|---|---|---|---|
| `catppuccin` | 0.2.27 | Installed, no warnings (about 2 s) | 8 themes (4 light/dark per file, including the "No Italics" versions) |
| `material-icon-theme` | 1.3.2 | Installed, no warnings (about 5 s) | 1 icon theme "Material Icon Theme", `icons_root` correct |
| `zig` | 0.4.2 | Installed, no warnings (about 6 s) | the Zig language, suffixes `zig` and `zon`, `grammars/zig.wasm` (695 KB), 8 queries including `injections` |
| `vue` | 0.4.1 | Installed, no warnings (about 5 s) | the Vue.js language, suffix `vue`, `grammars/vue.wasm` (26 KB), 6 queries including `injections` |

- All four: `luc ext install` exits with code 0, `luc ext list --installed` lists all four, `broken` is `null`, and the log in `logs/<id>.log` has no error or warning lines.
- The `installed/<id>/` directory has the full `themes/`, `icon_themes/`, `languages/<name>/*.scm` and `grammars/*.wasm` as in §3.1.
- `luc ext remove` for each id: done, `index.json` returns to `{"extensions": {}}`, and `installed/`, `staging/` and `trash/` are all empty.
- No extension failed, so there were no errors caused by Lucerna to record.

### 13.4 Still waiting for the user to check visually

This working session could not control the GUI (no screen recording permission), so the following points have not been checked:

- choosing "Set Theme" from the toast after installing Catppuccin and seeing the UI change colors;
- choosing Material Icon Theme with *Preferences: File Icon Theme* and seeing the icons in the file tree;
- opening a `.zig` file and seeing it colored;
- opening a `.vue` file and seeing both `<script>` and `<style>` colored;
- removing each extension inside the already-open window, with no restart.

So far it has only been checked via the CLI that the built `.wasm` grammars, `languages/*/injections.scm` and the index are correct. Loading them into on-screen colors is covered by the integration tests in Tasks 2–13 using the `json.wasm` fixture, and has not been run with the real `zig.wasm` or `vue.wasm`.
