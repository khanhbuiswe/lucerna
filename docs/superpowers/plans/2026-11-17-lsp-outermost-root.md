# LSP Outermost Root Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A language server's root defaults to the outermost directory with a root marker, so jumping into `vendor/`, `node_modules/` and similar no longer starts a second server.

**Architecture:** `RootSearch`'s serde default flips from `Nearest` to `Outermost`. `find_root` already implements all three modes; no algorithm changes. `servers.json` keeps gopls on `"ordered"` and rust-analyzer's explicit `"outermost"`. Users can still opt back with `lsp.<server>.root_search: "nearest"`, because the `lsp` setting is merged into the built-in table.

**Tech Stack:** Rust, serde, `tempfile` in tests.

**Spec:** `docs/superpowers/specs/2026-11-17-lsp-outermost-root-design.md`

## Global Constraints

- Cargo needs `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`.
- Run `scripts/prune-target.sh` before every `cargo test`. Delete `target/release` before a debug build or `cargo test`, unless another cargo is running (`pgrep -x cargo`). Never set `CARGO_TARGET_DIR`.
- Stage files by explicit path; commit trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Code comments and test names in English.

## Review Focus

1. A Laravel app without `.git` at the workspace root: `apps/api/composer.json` must still win over `apps/api/vendor/<pkg>/composer.json` (outermost is computed among markers found, not "workspace root"). Covered by `php_vendor_file_shares_app_root` (no-git case).
2. A user setting that only changes `root_search` must merge with the built-in entry (keep `command`, `languages`, `root_markers`). Covered by `nearest_can_be_restored_by_setting`.
3. gopls still prefers the nearest `go.work`, then `go.mod`: existing tests `go_prefers_go_work_over_go_mod` and `go_mod_when_no_go_work` must stay green unchanged.
4. A marker only above the workspace still falls back to the workspace root: existing `root_never_escapes_workspace` must stay green unchanged.
5. A custom server defined only in settings, with no `root_search`, now gets outermost: covered by the updated `glob_marker_finds_sln`.

---

### Task 1: Default root search to outermost

**Files:**
- Modify: `crates/lucerna-lsp/src/adapters.rs:11-24` (the `RootSearch` enum)
- Test: `crates/lucerna-lsp/tests/adapters.rs`
- Modify: `docs/research/2026-11-11-lsp-server-table.md` (one note under the Helix section, near line 52)

**Interfaces:**
- Consumes: `find_root(def: &ServerDef, file: &Path, workspace_root: &Path) -> Option<PathBuf>`, `ServerTable::build(Option<&Value>)`, test helpers `adapter(name)` and `touch(path)` already in `adapters.rs`.
- Produces: `RootSearch::default() == RootSearch::Outermost`.

- [ ] **Step 1: Update existing tests and add new ones in `crates/lucerna-lsp/tests/adapters.rs`**

Rename `ts_nearest_package_json` to `ts_monorepo_uses_outermost_package_json`; same fixture, expected root becomes `root` (not `root.join("packages/web")`).

In `glob_marker_finds_sln`, the second assertion (after touching `src/App/App.csproj`) now expects `root`; change its comment to `// The outermost directory with a matching entry wins.`

Leave `glob_marker_question_mark_and_no_match` as is (only one marker directory).

Add:

```rust
#[test]
fn php_vendor_file_shares_app_root() {
    for with_git in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        if with_git {
            fs::create_dir_all(root.join(".git")).unwrap();
        }
        touch(&root.join("apps/api/composer.json"));
        touch(&root.join("apps/api/vendor/laravel/framework/composer.json"));
        let app = root.join("apps/api/app/User.php");
        let lib = root.join("apps/api/vendor/laravel/framework/src/Model.php");
        touch(&app);
        touch(&lib);
        let def = adapter("intelephense");
        let expected = if with_git { root.to_path_buf() } else { root.join("apps/api") };
        assert_eq!(find_root(&def, &app, root).unwrap(), expected);
        assert_eq!(find_root(&def, &lib, root).unwrap(), expected);
    }
}

#[test]
fn node_modules_file_shares_project_root() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    touch(&root.join("web/package.json"));
    touch(&root.join("web/node_modules/react/package.json"));
    let app = root.join("web/src/main.ts");
    let lib = root.join("web/node_modules/react/index.d.ts");
    touch(&app);
    touch(&lib);
    let def = adapter("typescript-language-server");
    assert_eq!(find_root(&def, &app, root).unwrap(), root.join("web"));
    assert_eq!(find_root(&def, &lib, root).unwrap(), root.join("web"));
}

#[test]
fn nearest_can_be_restored_by_setting() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    touch(&root.join("package.json"));
    touch(&root.join("packages/web/package.json"));
    let file = root.join("packages/web/src/index.ts");
    touch(&file);
    let table = ServerTable::build(Some(&json!({
        "typescript-language-server": { "root_search": "nearest" }
    })));
    let def = table.get("typescript-language-server").unwrap();
    assert_eq!(find_root(def, &file, root).unwrap(), root.join("packages/web"));
}
```

- [ ] **Step 2: Run the tests and confirm the new/changed ones fail**

Run: `scripts/prune-target.sh && cargo test -p lucerna-lsp --test adapters`
Expected: FAIL in `ts_monorepo_uses_outermost_package_json`, `glob_marker_finds_sln`, `php_vendor_file_shares_app_root`, `node_modules_file_shares_project_root`; `nearest_can_be_restored_by_setting` already passes; all other tests pass.

- [ ] **Step 3: Move `#[default]` from `Nearest` to `Outermost` in `RootSearch` (`crates/lucerna-lsp/src/adapters.rs`)**

Update the enum's doc comment to say the outermost marker directory is the default (matches Helix and Zed; libraries inside the project, such as `vendor/` or `node_modules/`, carry their own markers). Leave `servers.json` unchanged.

- [ ] **Step 4: Run the whole crate's tests**

Run: `scripts/prune-target.sh && cargo test -p lucerna-lsp`
Expected: all pass, including `tests/table.rs` and `tests/registry.rs`.

- [ ] **Step 5: Run the workbench LSP tests**

Run: `cargo test -p lucerna-workbench --test lsp_sync --test lsp_vue --test diagnostics`
Expected: all pass.

- [ ] **Step 6: Add the Helix note to `docs/research/2026-11-11-lsp-server-table.md`**

Under the Helix `roots` row/section (near line 52), one line in Vietnamese: Helix picks the **outermost** directory with a marker (`find_lsp_workspace` overwrites on each match going up, stops at the workspace or a `workspace-lsp-roots` entry); Lucerna follows this since 2026-11-17, see `docs/superpowers/specs/2026-11-17-lsp-outermost-root-design.md`.

- [ ] **Step 7: Commit**

```bash
git add crates/lucerna-lsp/src/adapters.rs crates/lucerna-lsp/tests/adapters.rs docs/research/2026-11-11-lsp-server-table.md
git commit -m "fix(lsp): default the server root to the outermost marker directory

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>" -- crates/lucerna-lsp/src/adapters.rs crates/lucerna-lsp/tests/adapters.rs docs/research/2026-11-11-lsp-server-table.md
```
