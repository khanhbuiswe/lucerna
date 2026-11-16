# Scan Depth Limit Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Outside git repos, folders `files.scanDepth` (default 5) levels deep are listed but not scanned until the user expands them or opens a file inside, as in Zed.

**Architecture:** The scan in `lucerna-project` stops descending at the depth limit when the folder is not in a git repo, and records the folders it stopped at in the snapshot's set of unloaded folders. `Worktree::load` reads them on demand. Search skips them. Lookups of parent `.gitignore` files and of the git repo stop at `$HOME`. The workbench reads the setting, loads folders when the Explorer expands them or a file inside is opened, and shows a status message once.

**Tech Stack:** Rust, `ignore::WalkBuilder`, `notify`, GPUI (workbench only).

**Spec:** `docs/superpowers/specs/2026-11-16-scan-depth-design.md`

## Global Constraints

- Setting: `files.scanDepth`, default `5`; `0` or negative means no limit.
- Status message, exact copy: `Some folders deeper than {n} levels load when expanded`, where `{n}` is the setting value. English only.
- No platform-specific code; everything here works on macOS, Linux and Windows.
- Only `lucerna-workbench` may use gpui.
- Before cargo: `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"` and `scripts/prune-target.sh`.
- Never set `CARGO_TARGET_DIR` or `--target-dir`.
- Commit with the trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` and `git commit -- <paths>`.

**Representation (planner's call, same behaviour as the spec):** the spec names a new `EntryKind::UnloadedDir`. The plan keeps such a folder as `EntryKind::Dir` and records it in a set on the snapshot (`WorktreeSnapshot::is_unloaded`). An unloaded folder must look and act like a folder in every one of the roughly 15 `kind == EntryKind::Dir` checks in the Explorer, and a separate set keeps all of them unchanged.

**Depth definition:** a folder whose root-relative path has `n` components, with `n >= scan_depth`, is unloaded: it is an entry, but nothing below it is. With depth 5, `a/b/c/d/e` is listed and its children are not; files in `a/b/c/d` are listed.

## Review Focus

1. **Root inside a repo** (`luc ~/code/repo/src/deep`): the root is inside a git repo even though `.git` sits above it, so nothing is unloaded. Test in Task 1.
2. **Nested repo below the limit** (`~/work/a/b/c/d/repo/.git`): `repo` is at depth 5 and contains `.git`, so it is scanned fully. Test in Task 1.
3. **A file appears inside an unloaded folder** (the agent writes `a/b/c/d/e/new.rs`): it must not show up as an orphan entry without its parents. It stays hidden until `e` is loaded. Test in Task 2.
4. **Rescan after `files.exclude` changes** while the user has a deep folder expanded: the folder stays loaded. Test in Task 2.
5. **Unloaded folder deleted on disk:** it disappears from the tree like any other folder. Test in Task 2.

---

### Task 1: Depth limit in the scan

**Files:**
- Modify: `crates/lucerna-project/src/worktree.rs` (`ScanOptions`, `Worker::scan`, a new scan-only walker)
- Modify: `crates/lucerna-project/src/snapshot.rs` (unloaded set)
- Test: `crates/lucerna-project/tests/scan_depth.rs` (new)

**Interfaces:**
- Produces:
  - `ScanOptions { exclude: Vec<String>, scan_depth: Option<usize>, home: Option<PathBuf> }`. `Default` gives `None, None`, meaning no limit and no ceiling, so existing tests keep today's behaviour. `scan_depth: Some(0)` is never built; the workbench maps `<= 0` to `None`.
  - `WorktreeSnapshot::is_unloaded(&self, dir: &Path) -> bool` and `WorktreeSnapshot::unloaded(&self) -> &HashSet<Arc<Path>>`.
  - `WorktreeSnapshot::build(entries, complete)` keeps its signature. A new `pub(crate) fn with_unloaded(self, set: HashSet<Arc<Path>>) -> Self` attaches the set.
  - `pub(crate) fn root_in_repo(root: &Path, home: Option<&Path>) -> bool`: true if `.git` exists in `root` or in any ancestor, stopping after `home` when `root` is under `home`.

- [ ] **Step 1: Write the failing tests** in `tests/scan_depth.rs`, using a `tempfile` tree and a helper `wait_complete(&Worktree)` like the one in `tests/scan.rs`. Each test passes `home: Some(tmp)` so the machine's own repos never count.
  - `deep_folders_outside_repo_are_unloaded`: tree `a/b/c/d/e/f/g.txt`, plus `a/b/c/d/x.txt`, with `scan_depth: Some(5)`. Assert `contains("a/b/c/d/x.txt")`, `contains("a/b/c/d/e")`, `is_unloaded("a/b/c/d/e")`, `!contains("a/b/c/d/e/f")`, and that `unloaded().len() == 1`.
  - `repo_root_scans_fully`: the same tree plus a `.git/` folder at the root. Assert `contains("a/b/c/d/e/f/g.txt")` and that `unloaded()` is empty.
  - `root_below_repo_scans_fully`: create `.git/` in the temp folder and start the worktree at `tmp/sub`. Assert the deep file is present.
  - `nested_repo_below_limit_scans_fully`: create `a/b/c/d/repo/.git/` and `a/b/c/d/repo/x/y/z.txt`. Assert `contains("a/b/c/d/repo/x/y/z.txt")`, `!is_unloaded("a/b/c/d/repo")`.
  - `no_limit_scans_fully`: `scan_depth: None`. The deep file is present.
- [ ] **Step 2: Run them and see them fail.** Run `cargo test -p lucerna-project --test scan_depth`. Expected: it does not compile yet (the new fields don't exist).
- [ ] **Step 3: Implement.**
  - **Before the walk:** `Worker::scan` computes `in_repo = root_in_repo(&self.root, opts.home.as_deref())`. If `in_repo` is true or `scan_depth` is `None`, use today's walk unchanged.
  - **Otherwise,** build the walker with `walk_builder(&self.root, opts)`, then call `filter_entry` again. The new filter replaces the old one, so it must also keep the `always_skipped` check. It returns `false` for a folder whose root-relative path has `>= depth` components unless that folder or one of its ancestors at depth `< depth` (excluding the root) contains `.git`.
  - **Rejected folders:** push them into an `Arc<Mutex<Vec<PathBuf>>>` that the closure captures. After the walk, add each one as an `EntryKind::Dir` entry and as a member of the unloaded set.
  - **Publish:** `publish` attaches the set with `with_unloaded`. Store the set on `Worker` (`RefCell<HashSet<Arc<Path>>>`), the same way `root_status` is stored.
  - **Content search** keeps using plain `walk_builder`; Task 3 handles it.
- [ ] **Step 4: Run the tests and see them pass.** Run `cargo test -p lucerna-project`. Expected: everything passes, including the existing `scan.rs`, `watch.rs` and the unit tests.
- [ ] **Step 5: Commit** `feat(project): leave folders past files.scanDepth unloaded outside git repos`.

### Task 2: Loading on demand, watcher and rescan

**Files:**
- Modify: `crates/lucerna-project/src/worktree.rs` (`Command::Load`, `Worktree::load`, `apply_fs_events`, rescan)
- Test: `crates/lucerna-project/tests/scan_depth.rs`

**Interfaces:**
- Consumes: Task 1's unloaded set.
- Produces: `Worktree::load(&self, rel: &Path)`, where `rel` is root-relative. It loads every unloaded folder that is `rel` or an ancestor of `rel`. Afterwards it publishes and sends `WorktreeEvent::Changed` with `Added` for the new entries. If nothing on the path is unloaded, it does nothing.

- [ ] **Step 1: Write the failing tests.**
  - `load_reads_one_level`: tree `a/b/c/d/e/f/g/h.txt` with depth 5. Call `load("a/b/c/d/e")` and wait for `Changed`. Assert `contains("a/b/c/d/e/f")`, `is_unloaded("a/b/c/d/e/f")`, `!is_unloaded("a/b/c/d/e")`.
  - `load_path_of_file_loads_ancestors`: call `load("a/b/c/d/e/f/g/h.txt")`. Assert `contains("a/b/c/d/e/f/g/h.txt")` and that no ancestor of it is unloaded.
  - `new_file_inside_unloaded_folder_stays_hidden`: after `ScanComplete`, write `a/b/c/d/e/new.txt`, then create `a/top.txt` as a marker and wait for its `Changed`. Assert `!contains("a/b/c/d/e/new.txt")`.
  - `loaded_folder_survives_rescan`: call `load("a/b/c/d/e")`, then `set_options` with a new `exclude`, and wait for `Rescanned`. Assert `contains("a/b/c/d/e/f")`.
  - `deleting_unloaded_folder_removes_it`: run `fs::remove_dir_all("a/b/c/d/e")` and wait for `Changed`. Assert `!contains("a/b/c/d/e")` and that `unloaded()` is empty.
- [ ] **Step 2: Run them and see them fail.** Run `cargo test -p lucerna-project --test scan_depth`. Expected: it does not compile (`load` is missing).
- [ ] **Step 3: Implement.**
  - **New command:** add `Command::Load(Arc<Path>)` and handle it in `Worker::run`. Also handle it in the `SetOptions` coalescing match and in `apply_queued_fs_events`, which both push it to the backlog.
  - **Loading one folder:** read the folder one level with `fs::read_dir`, filtered through `IgnoreChecker`. The child folders that come out are unloaded unless they contain `.git`. A child that contains `.git` gets `collect_subtree`.
  - **Rescan:** the worker keeps `loaded: RefCell<HashSet<Arc<Path>>>`. After every full scan, it re-applies `Load` for each path that still exists, in order of path length, before publishing `Rescanned`.
  - **In `apply_fs_events`:** skip a change whose path has a proper ancestor in the unloaded set. A change to the unloaded folder itself goes through the normal path. Removal already drops descendants. It also removes the folder from the set and from `loaded`.
- [ ] **Step 4: Run the tests and see them pass.** Run `cargo test -p lucerna-project`. Expected: everything passes.
- [ ] **Step 5: Commit** `feat(project): load unloaded folders on demand`.

### Task 3: Search skips unloaded folders

**Files:**
- Modify: `crates/lucerna-project/src/search.rs` (`search`)
- Modify: the `search(` call site in `crates/lucerna-workbench/src/search_view/` (run `grep -rn "search(" crates/lucerna-workbench/src/search_view`)
- Test: `crates/lucerna-project/tests/search.rs` (existing, or wherever the `search` tests live)

**Interfaces:**
- Consumes: `WorktreeSnapshot::unloaded()`.
- Produces: `search(root, query, settings_exclude, skip_dirs: &HashSet<Arc<Path>>, buffers, sink, cancel)`. `skip_dirs` is root-relative. The walk does not descend into those folders, and the folders themselves yield nothing.

- [ ] **Step 1: Write the failing test** `search_skips_unloaded_dirs`. Make a tree with `deep/needle.txt` and `top.txt`, both containing `needle`. Pass `skip_dirs = {"deep"}` and assert that only `top.txt` matches. Pass an empty set and assert that both match.
- [ ] **Step 2: Run it and see it fail.** Run `cargo test -p lucerna-project search_skips_unloaded_dirs`. Expected: it does not compile.
- [ ] **Step 3: Implement.** Add the parameter. The walker gets a second `filter_entry` that keeps `always_skipped` and rejects any entry whose root-relative path is in `skip_dirs`. The workbench passes `worktree.snapshot().unloaded().clone()`.
- [ ] **Step 4: Run the tests and see them pass.** Run `cargo test -p lucerna-project && cargo test -p lucerna-workbench search`. Expected: everything passes.
- [ ] **Step 5: Commit** `feat(search): skip folders not loaded yet`.

### Task 4: Parent lookups stop at `$HOME`

**Files:**
- Modify: `crates/lucerna-project/src/worktree.rs` (`walk_builder_with_includes`)
- Modify: `crates/lucerna-project/src/ignored.rs` (`IgnoreChecker`)
- Modify: `crates/lucerna-git/src/repo.rs` (`Repo::discover`)
- Test: `crates/lucerna-project/tests/scan_depth.rs`, `crates/lucerna-git/tests/` (the existing repo tests file)

**Interfaces:**
- Consumes: `ScanOptions::home` from Task 1.
- Produces: `Repo::discover_with_ceiling(git: GitBinary, workspace_root: &Path, home: Option<&Path>)`. When `workspace_root` is under `home`, it runs `rev-parse` with `GIT_CEILING_DIRECTORIES` set to `home`'s parent, so git looks in `home` but no higher. `discover` stays as a wrapper that passes `None`.

- [ ] **Step 1: Write the failing tests.**
  - `gitignore_above_home_is_ignored`: use the layout `tmp/.gitignore` containing `*.log`, with `home = tmp/home` and `root = tmp/home/proj`. Put `a.log` in `root`. Assert that `a.log` is present in the snapshot. Do the same for a watcher-reported `b.log` created after `ScanComplete`, which exercises `IgnoreChecker`.
  - `gitignore_in_home_still_applies`: `tmp/home/.gitignore` containing `*.log`. Assert `a.log` is absent.
  - In `lucerna-git`: `discover_stops_at_home`. Run `git init` in `tmp`, then call `discover_with_ceiling(git, tmp/home/proj, Some(tmp/home))`. Assert the result is `Ok(None)`.
- [ ] **Step 2: Run them and see them fail.** Run `cargo test -p lucerna-project --test scan_depth gitignore && cargo test -p lucerna-git discover_stops_at_home`. Expected: the first and third tests fail.
- [ ] **Step 3: Implement.**
  - **Walker:** when `opts.home` is `Some` and the root is under it, set `.parents(false)` and add the ignore files of each ancestor from the root's parent up to `home` inclusive, so they apply exactly as `parents(true)` would. Verify this against the `ignore` crate docs and the test. If `add_ignore` does not anchor patterns the same way, write a small custom parent loader instead.
  - **`IgnoreChecker`:** stop its ancestor walk at the same point. Update its module doc, which today says "from the filesystem root".
  - **Workbench:** pass `home` to `discover_with_ceiling` where the workbench discovers the repo (run `grep -rn "Repo::discover" crates/lucerna-workbench`).
- [ ] **Step 4: Run the tests and see them pass.** Run `cargo test -p lucerna-project && cargo test -p lucerna-git`. Expected: everything passes.
- [ ] **Step 5: Commit** `fix(project): parent .gitignore and repo lookup stop at $HOME`.

### Task 5: Setting, Explorer, opening files, status message

**Files:**
- Modify: `crates/lucerna-settings/assets/default-settings.json`: add `"files.scanDepth": 5` after `files.exclude`
- Modify: `docs/settings.md`: add a row for `files.scanDepth` (default `5`). Text: "Outside a git repository, folders this many levels deep load when you expand them. `0` scans everything."
- Modify: `docs/features.md`: one sentence in the section on how folders open
- Modify: `crates/lucerna-workbench/src/workbench.rs`:
  - where `ScanOptions` is built (around line 585): read `files.scanDepth` and `dirs`/`home_dir` the way the crate already gets home (`grep -rn home_dir crates/lucerna-workbench/src`);
  - `excludes_changed` (around line 1135): also compare `scan_depth`;
  - on `ScanComplete`: show the status message once per window when `!snapshot.unloaded().is_empty()`
- Modify: `crates/lucerna-workbench/src/explorer/view.rs`: in the folder click near line 1282, after `toggle`, if the folder is now expanded and `snapshot.is_unloaded(&path)`, call `worktree.load(&path)`. Do the same in `reveal` for a path the model doesn't contain (lines ~1084 and ~1101): call `worktree.load(path)` and keep the reveal pending until `Changed` brings the path in.
- Test: `crates/lucerna-workbench/tests/` (the existing explorer and workbench gpui test files)

**Interfaces:**
- Consumes: `ScanOptions` (Task 1), `Worktree::load` (Task 2), `WorktreeSnapshot::is_unloaded` and `unloaded` (Task 1).
- Produces: nothing new for other tasks.

- [ ] **Step 1: Write the failing tests** with gpui `TestAppContext`, following the existing explorer tests.
  - `expanding_unloaded_folder_loads_it`: open a temp folder (no `.git`) with `files.scanDepth: 2` and the tree `a/b/c.txt`. Click `a/b`, then run until parked. Assert that the row `a/b/c.txt` is visible.
  - `revealing_file_in_unloaded_folder_loads_it`: open `a/b/c.txt` through the workbench's open-file path. Assert that the Explorer selects it.
  - `status_message_once`: after the scan, the status message text equals `Some folders deeper than 2 levels load when expanded`. A second `ScanComplete` does not show it again.
  - `scan_depth_zero_scans_all`: with `files.scanDepth: 0`, `c.txt` is in the snapshot without any expand.
- [ ] **Step 2: Run them and see them fail.** Run `cargo test -p lucerna-workbench unloaded`, plus `status_message_once` and `scan_depth_zero`. Expected: they fail.
- [ ] **Step 3: Implement** the changes listed under Files. Use `status_message::show(window_id, text, cx)` for the message.
- [ ] **Step 4: Run the full suite.** Run `scripts/prune-target.sh && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`. Expected: everything passes. If `cargo fmt --check` fails only on files this plan did not touch, leave them and say so.
- [ ] **Step 5: Commit** `feat: files.scanDepth, load deep folders on expand (as in Zed)`.
