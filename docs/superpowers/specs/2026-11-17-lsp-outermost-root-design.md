# LSP: the default root is the outermost directory

Small change. Date 2026-11-17.

## Why

Open a Laravel project (`noti.one`, code in `apps/api`) and then go-to-definition into
`apps/api/vendor/laravel/framework/...`, and Lucerna starts a second
intelephense. The reason: the server's root is the **nearest** directory that has a marker, and every package in
`vendor/` has a `composer.json`, so the root becomes `vendor/laravel/framework`.
The second server re-indexes from scratch, does not see the app code, and shows up as a "duplicate" row in the
LSP popover.

It is not only PHP. Every ecosystem that downloads libraries into the project, each package with its own
marker, is hit:

- `node_modules/<package>/package.json`: typescript-language-server, vtsls,
  vue-language-server, vscode-css, vscode-html.
- `deps/<package>/mix.exs`: elixir-ls.
- `.build/checkouts/<package>/Package.swift`: sourcekit-lsp.
- `build/packages/<package>/gleam.toml`: gleam.
- `_build/default/lib/<package>/rebar.config`: elp.

VS Code, Zed and Helix all do not pick "nearest" (see References). Lucerna is
the exception, and the old specs do not record why "nearest" was chosen.

## Goals

- Jumping into library code that lives inside the project does not spawn another server.
- The default root matches Helix: the outermost directory with a marker, from the file up to the
  workspace root.

## Non-goals

- Sharing one process across several workspace folders (`didChangeWorkspaceFolders`,
  like Helix). Bigger, later.
- A setting that splits a monorepo into several roots like Helix's `workspace-lsp-roots`.
- A list of library directories to skip (approach A); no longer needed once outermost is chosen.
- Changing the popover to tell apart two rows with the same server name.

## Approach

Approaches considered:

- **A.** Skip `vendor/`, `node_modules/`, `deps/`... when looking for the root. Fixes every
  server but has to keep a hard-coded directory list.
- **B.** Reuse a running server if its root contains the file. Depends on the order files
  are opened.
- **C.** Only let intelephense/phpactor pick outermost. A one-line data change,
  but JS/TS, Elixir, Swift... are still hit.
- **D.** Change every server's default to outermost, like Helix. One place to change,
  no directory list needed. Trade-off: a JS/Python monorepo runs one server for
  the whole repo (VS Code and Helix do the same). **The user chose this approach.**

Changes:

- `RootSearch` in `crates/lucerna-lsp/src/adapters.rs`: `#[default]` moves
  from `Nearest` to `Outermost`. `Nearest` stays, and users can set it through
  `lsp.<server>.root_search: "nearest"` (the `lsp` setting merges into the default table).
- `servers.json`: gopls keeps `"ordered"` (prefer `go.work`, then `go.mod`, as Go intends).
  rust-analyzer keeps its `"outermost"` line even though it now matches the default,
  because it states the intent (my decision).
- Tests in `crates/lucerna-lsp/tests/adapters.rs`:
  - Fix `ts_nearest_package_json`: a monorepo with `package.json` at the top and
    `packages/web` now gets the top as root. Rename it to match the new behavior.
  - Fix the second half of `glob_marker_finds_sln`: with both `a.sln` at the top and
    `src/App/App.csproj`, it now gives the top.
  - New: intelephense, a file in `apps/api/vendor/laravel/framework/` and a file
    in `apps/api/app/` get the same root, both with and without a `.git` at the top.
  - New: typescript-language-server, a file in
    `node_modules/<package>/` gets the same root as the project's file.
  - New: setting `root_search: "nearest"` through settings still gives the nearest directory.
- My decision: do not edit the old specs (history). Add one line to
  `docs/research/2026-11-11-lsp-server-table.md` saying Helix picks the outermost marker, so
  nobody has to look it up again.

Consequences to know:

- `.git` is only a marker when it is the server's only marker (yaml, json,
  bash). Fifteen servers that had `.git` alongside a project marker (intelephense, phpactor,
  ruby-lsp, jdtls, sourcekit-lsp, lua, css/html...) drop `.git`, because with the outermost
  search, a `.git` at the repo root would always beat the `composer.json`, `Gemfile`,
  `pom.xml` in subdirectories. With no marker at all, the workspace root is still used, as
  before. (Found during review; test `git_is_only_ever_a_sole_root_marker`.)
- An Elixir umbrella takes the umbrella's top `mix.exs`, a multi-module Maven/Gradle takes the parent
  `pom.xml`/`settings.gradle`. That is the root these servers want.
- A server already running with the old root does not change by itself; the user needs to Restart or reopen the
  app. The default table is inside the binary, so the change arrives with the new build.

## References

- **VS Code:** the intelephense extension (`src/extension.ts`) creates one
  `LanguageClient` for the whole window, without passing a separate root; files in `vendor/`
  go into that same server.
- **Zed:** `crates/project/src/manifest_tree.rs`.
  - A server that declares no manifest uses the worktree's root
    (`root_for_path_or_worktree_root`). The PHP extension declares no manifest,
    so each worktree has one intelephense.
  - A server with a manifest picks the **outermost** directory: `CargoManifestProvider`
    (`crates/languages/src/rust.rs`), `PyprojectTomlManifestProvider` (changed to
    outermost in PR #53781 for uv/Poetry workspaces).
  - There is no special rule for `vendor/`/`node_modules/`.
- **Helix:** `find_lsp_workspace` in `helix-lsp/src/lib.rs`.
  - Walks up from the file's directory, each marker met overrides the earlier result, so the
    **highest** directory with a marker wins. Stops at the workspace root or at a
    directory in `workspace-lsp-roots`. With no marker, uses the workspace root.
  - There is no special rule for library directories.
  - `try_add_doc` in `helix-lsp/src/client.rs`: a root equal to a running client is
    reused; a different root where the server supports several workspace folders adds a
    folder through `didChangeWorkspaceFolders` instead of opening a new process.
