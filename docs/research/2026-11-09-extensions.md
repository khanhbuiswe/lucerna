# Research: the extension systems of Zed and VS Code

- **Date:** 2026-11-09
- **Status:** research notes, groundwork for the Phase 2 spec.
- **Sources:**
  - Zed: source at rev `b5cd310` (the crates `extension`, `extension_host`, `extension_cli`, `language`, `theme`, `file_icons`, `snippet_provider`) and `docs/src/extensions/`. These crates are GPL-3: learn the design and data formats only, don't copy code.
  - VS Code: `microsoft/vscode` (MIT) at commit `142e5584`, the docs at `code.visualstudio.com/api`, the Open VSX wiki.
- Items not verified directly in the source are marked **[unverified]**.

## 1. Overall architecture

| | Zed | VS Code |
|---|---|---|
| Extension code | WASM component (`extension.wasm`, WIT `since_v*`), runs in wasmtime, in the same process | JavaScript in a separate Node process (the extension host) |
| Declarative part (no code runs) | themes, icon themes, languages, grammars, snippets | `contributes.themes`, `iconThemes`, `languages`, `grammars` (TextMate), `snippets`, `configurationDefaults`… |
| Data-only extension | `extension.wasm` is not loaded | Never activated, can be added and removed while running, always allowed even in an untrusted workspace |

**Conclusion for Lucerna:** all four things in Phase 2's scope are data. Phase 2 doesn't need to run extension code. The WASM host is left to Phase 3, together with LSP.

## 2. Distribution and registry

- **Zed:**
  - The client only calls `api.zed.dev` to get the list, updates, and to download the prebuilt `archive.tar.gz`.
  - The `zed-industries/extensions` repo has:
    - `extensions.toml`, where each entry is `[<id>] submodule = "extensions/<id>"`, `version = "x.y.z"`, `path = "subdir"` (optional);
    - git submodules pinned by commit.
  - That repo's CI builds extensions with `extension_cli`. The repo does not contain prebuilt archives.
  - A note on terms: Zed has public terms (https://zed.dev/terms, updated 2026-03-02). §2.2 only allows access to the Service through "the mechanisms designated by Zed", and §2.4(b) forbids "seek to access non-public APIs associated with the Service". The extension endpoint of `api.zed.dev` has no public documentation, so Lucerna doesn't call it. See `docs/research/2026-11-11-lsp-server-table.md` §5.
  - To avoid `api.zed.dev`, you have to do the steps yourself: read `extensions.toml` and `.gitmodules`, get the submodule commit, fetch it, apply `path`, then build it yourself.
- **VS Code:**
  - A `.vsix` package is a zip file. The gallery is `POST {serviceUrl}/extensionquery`.
  - The OSS build configures no gallery.
  - Microsoft Marketplace's terms only allow use with Microsoft products. VSCodium and third-party editors use **Open VSX** (Eclipse Foundation, with rate limits).
  - Each package has a `.sigzip` signature, and there is a blocklist of malicious extensions.
- **Conclusion:**
  - Lucerna builds from the `zed-industries/extensions` repo itself, exactly as in the roadmap.
  - Don't use Microsoft Marketplace. If VS Code extensions are needed later, the legitimate source is Open VSX.

## 3. On-disk layout, install and uninstall

- **Zed:** the folder `~/Library/Application Support/Zed/extensions/` contains:
  - `installed/<id>/`: the built result, with a full `extension.toml`, `grammars/<name>.wasm`, `themes/`, `icon_themes/`, `icons/`, `languages/<dir>/`, snippets;
  - `build/`: holds wasi-sdk;
  - `staging/`: where things are unpacked before being renamed to `installed/<id>`;
  - `index.json`: an index of themes, icon themes and languages by name, with matchers and the query files present. It is rebuilt when the mtime of `installed/` is newer.

  A dev extension is a symlink. To uninstall an extension, delete the folder and reload.
- **VS Code:**
  - The folder `~/.vscode/extensions/<publisher>.<name>-<version>`, with an `extensions.json` listing installed extensions.
  - Atomic install: unpack into `.<uuid>` then rename.
  - Uninstall is deferred via an `.obsolete` file: skipped during scanning, actually deleted on the next startup. This way running windows can still read the old files.
- **Conclusion:**
  - Use `staging/` then `rename` for atomic install.
  - Keep one index file so startup is fast without re-reading every `config.toml`.
  - Uninstall and update also go through a rename step. Since Lucerna reads grammars into memory when loading, deleting immediately is safe.

## 4. Building grammars

- **Zed (`extension_builder`):**
  - Fetch the grammar source: `git init`, `git fetch --depth 1 origin <rev>`, `git checkout <rev>` into `<ext>/grammars/<name>/`.
  - Compile:

    ```
    <wasi-sdk>/bin/clang -fPIC -shared -Os -Wl,--export=tree_sitter_<name> -Wl,--max-memory=4294967296 \
      -o grammars/<name>.wasm -I src src/parser.c [src/scanner.c]
    ```
  - wasi-sdk **34**, downloaded from `github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-34/wasi-sdk-34.0-<arm64|x86_64>-macos.tar.gz`. The `WASI_SDK_PATH` variable takes priority if set.
  - Only `scanner.c` is supported. **`scanner.cc` (C++) is not supported.**
  - The grammar name must be snake_case.
- **Loading at runtime:**
  - There is one `wasmtime::Engine` shared across the whole process.
  - Each `Parser` in the pool has its own `WasmStore`.
  - Call `store.load_language(name, bytes)` once, then cache the `tree_sitter::Language`.
  - On a name clash, a native grammar always wins over an extension grammar.
- **VS Code:** uses TextMate grammars (JSON/plist), no compilation needed. Not applicable to Lucerna.

## 5. Languages and queries

- **Zed's `config.toml`:** the fields useful for a viewer are:
  - `name`, `grammar`;
  - `path_suffixes` (matches the extension, the full file name, or a `.<suffix>` ending);
  - `first_line_pattern`;
  - `line_comments`, `block_comment`;
  - `brackets`;
  - `hidden` (a language used only for injection);
  - `tab_size`.
- **Valid query files:** `highlights`, `brackets`, `outline`, `indents`, `injections`, `overrides`, `redactions`, `runnables`, `debugger`, `textobjects` (`.scm` suffix). **There is no `folds.scm`.**
- **Language selection order:**
  - Zed: the user's `file_types` globs, then the longest matching path suffix, and only last `first_line_pattern`.
  - VS Code: the user's `files.associations`, then the exact file name, then the longest glob, then the longest extension, and finally `firstLine`. On a tie, the language registered later wins.
- **Highlighting in Zed:**
  - Captures are stacked in iteration order. Only a capture that maps to a style in the theme counts.
  - For the same node, **the last capture with a style wins.** In a pattern `@a @b`, Zed tries `@b` first. Between patterns, the later pattern wins **[unverified: the order of tree-sitter captures]**.
  - A capture name falls back along its dots when looking up a style.
  - A capture with a `_` prefix is ignored.
  - The only custom predicates are `#has-parent?` and `#not-has-parent?`.
- **Injection in Zed:**
  - `@injection.content` (or `@content`), `@injection.language`, `#set! injection.language`, `injection.combined`.
  - Language names are looked up case-insensitively, by name or by path suffix. Multiple levels of nesting work. The injected layer wins over the host layer.
  - Many extensions depend on injection: HTML with JS/CSS, Vue, Svelte, PHP, Markdown with code fences.

## 6. Themes

- **Zed:**
  - Format `{ name, author, themes: [{ name, appearance, style }] }`, schema v0.2.0, read leniently.
  - A theme extension may not provide anything else.
  - On a name clash, the theme loaded later wins.
- **VS Code:**
  - `workbench.colorTheme` stores an id or a label. On a clash, the first theme found while scanning wins.
  - If the theme in settings isn't installed, VS Code uses the default theme but **doesn't overwrite the setting**. When the extension that provides that theme is installed, the theme is applied automatically.
- **Conclusion:**
  - Set an explicit tie-breaking rule.
  - Keep VS Code's behavior: the setting isn't deleted, and the theme is applied automatically when it appears.

## 7. Icon themes

- **Zed (schema v0.3.0):**
  - Format `{ name, author, themes: [{ name, appearance, directory_icons, named_directory_icons, chevron_icons, file_stems, file_suffixes, file_icons{type: {path}} }] }`.
  - Icon paths are relative to the extension's root folder.
  - Icon lookup order:
    1. the full file name;
    2. compound suffixes, from longest to shortest (`a.b.c`: `b.c`, then `c`);
    3. the stem;
    4. the extension;
    5. `default`.

    At each step, look in `file_stems` first and then `file_suffixes`. If an icon is missing, fall back to the default icon theme.
  - **The SVGs of an icon theme are drawn in full color with `img()`**, not tinted a single color like the built-in icons (`svg()` plus `text_color`).
- **VS Code:**
  - `iconDefinitions`, `fileNames`, `fileExtensions`, `folderNames`, `languageIds`, the `light`/`highContrast` sections.
  - An icon can be an SVG or PNG, or a glyph in a font.
  - Lookup order: file name, then the longest extension, then languageId, then the default.
- **Conclusion:**
  - Lucerna reads Zed's schema and draws with `img()`.
  - The current Lucide set becomes the default icon theme, named "Lucide".

## 8. Snippets

- Both use the same JSON format: `{ "<name>": { prefix, body, description } }`, with the body in TextMate syntax.
- **Zed:**
  - The file name decides which language a snippet belongs to: `rust.json` for Rust, `snippets.json` shared by all languages.
  - Bodies are checked with the snippet parser.
  - Snippets are loaded into memory, not copied into the user's snippets folder.

## 9. Updates and compatibility

- **Zed:**
  - `schema_version` ≤ 1.
  - If `lib` is present, `wasm_api_version` must be in the supported range.
  - There is no constraint on the Zed version.
  - Auto-updates when the registry's semver is higher.
  - There is a hardcoded list of blocked ids, for example `snippets`, `ruff`.
  - Only the size of the downloaded file is checked, with no checksum.
- **VS Code:**
  - `engines.vscode` is required.
  - The settings `extensions.autoUpdate`, `extensions.autoCheckUpdates`.
  - Signature verification.
- **Conclusion for Lucerna:**
  - Pin by submodule commit. This is a form of integrity check through the git hash, stronger than Zed's approach.
  - Periodic checks, manual updates, with `extensions.autoCheckUpdates`.
  - An extension with `lib` can still install its data part. The language server part will work from Phase 3.
