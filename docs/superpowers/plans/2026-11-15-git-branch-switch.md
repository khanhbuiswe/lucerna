# Branch Switching from the Title Bar Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Clicking the branch name in the title bar (or running `git.checkout`) opens a branch picker. The user can switch to a local or remote branch, or create a new branch from HEAD, through `git switch` run in the background.

**Architecture:**
- `lucerna-git` gains:
  - a branch list with dates and remotes;
  - a write-command runner that keeps hooks, has no timeout and never kills git;
  - `switch_branch`;
  - branch-name helpers.
- The existing Compare-mode `BranchPicker` gains a Checkout mode: a current-branch mark, relative times, and a create row.
- The workbench makes the title-bar branch a button. It gates the click on `GitState`, blocks the switch on unsaved or conflicted documents, runs the switch in the background, and shows git's error in a native prompt.

**Tech Stack:** Rust 2024, `std::process` (git CLI), gpui (workbench only). No new external crates.

**Spec:** `docs/superpowers/specs/2026-11-15-git-branch-switch-design.md`. Research: `docs/research/2026-11-15-git-branch-switch.md`. The spec is binding; read §4 before each task.

## Global Constraints

- **Licensing:**
  - MIT OR Apache-2.0.
  - Zed's `git_ui` / `git` crates are GPL: learn the design, never copy code.
  - `cargo deny check` must pass.
- **Crates:** only `lucerna-workbench` uses gpui. The UI thread never runs git or waits on IO.
- **Read commands are unchanged.** `GitBinary::command()` keeps `-c core.fsmonitor=false -c core.hooksPath=/dev/null --no-optional-locks --no-pager`, its 10 s timeout and its `command_is_hardened` test.
- **Write commands** (`git switch` only, in this plan):
  - flags: `-c core.fsmonitor=false --no-pager`;
  - **no** `core.hooksPath` override;
  - **no** `--no-optional-locks`;
  - env `GIT_TERMINAL_PROMPT=0`, `LC_ALL=C`; stdin null; cwd = worktree root;
  - **no timeout, never killed.**
- **Exact copy:**
  - Picker placeholder (Checkout mode): "Switch to a branch or type to create one".
  - Create row: `+ Create branch "‹name›"`. Invalid-name suffix: "Invalid branch name".
  - Unsaved-files message: "Save or revert unsaved files before switching branches".
  - Title bar while running: "Switching…".
  - Error prompt: title "Could not switch branches", detail = git's stderr trimmed, one button "OK".
  - Gate messages: `Off` → "Git is off"; `Locating` → "Git is still loading"; `NoGit` / `NoRepo` → "Git is not available".
  - Command `git.checkout`, title "Git: Checkout to…", no default key.
- **Branch name sanitizing:** trim both ends, then replace each run of whitespace with `-`. Validate with `git check-ref-format --branch <name>`.
- **Remote names** come from `git remote`. Never split `remote/branch` on the first `/`.
- **Builds:** run `scripts/prune-target.sh` before tests (skip it if another cargo is running). Use the SHARED target dir only; never set `CARGO_TARGET_DIR` or `--target-dir`. Stage files by explicit path. Commit trailer: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **A branch name that looks like a flag or path** (`-x`, a name equal to a file in the repo). `git switch` must treat it as a branch name. Test: Task 1 `switch_rejects_option_like_names_safely`.
2. **A remote whose branch contains slashes** (`origin/feature/x`), or two remotes (`origin`, `upstream`). Switching creates the local `feature/x` tracking the correct remote. Test: Task 1 `remote_branch_with_slashes_tracks_right_remote`.
3. **A click on the title-bar branch that wobbles slightly.** It opens the picker and never starts a window drag or a double-click zoom. Test: Task 3 `branch_click_does_not_reach_title_bar`.
4. **Pressing Enter twice quickly, or running `git.checkout` while a switch runs.** Only one `git switch` runs. Test: Task 3 `second_switch_ignored_while_running`.
5. **Typing a name that differs from an existing branch only by surrounding spaces** (`" main "`). No create row; `main` is selected. Test: Task 2 `create_row_compares_sanitized_names`.

---

### Task 1: `lucerna-git`: branch list, write runner, switch, name helpers

**Files:**
- Modify: `crates/lucerna-git/src/command.rs` (write runner), `crates/lucerna-git/src/repo.rs` (branch list, switch, name check), `crates/lucerna-git/src/lib.rs` (exports)
- Create: `crates/lucerna-git/src/branch.rs` (`BranchRef`, `SwitchTarget`, `sanitize_branch_name`) if `repo.rs` would grow by more than about 150 lines
- Test: `crates/lucerna-git/tests/branch.rs`

**Interfaces:**
- Produces:
  - `pub struct BranchRef { pub name: String, pub remote: Option<String>, pub committed: i64, pub is_head: bool }`
    - `name` is the short form: `main`, `origin/feature/x`.
    - `remote` is `Some("origin")` for remote-tracking refs.
    - `committed` is the committer date as a unix timestamp.
  - `impl BranchRef { pub fn branch_part(&self) -> &str }`: the name without the `remote/` prefix (`feature/x`). For a local branch it is `name`.
  - `Repo::branch_list(&self) -> Result<Vec<BranchRef>, GitError>`
    - Runs `for-each-ref --sort=-committerdate --format=%(refname)%00%(committerdate:unix)%00%(HEAD) refs/heads refs/remotes`.
    - Drops `refs/remotes/*/HEAD`.
    - The remote is the longest remote from `git remote` whose `refs/remotes/<remote>/` is a prefix of the ref.
  - `pub enum SwitchTarget { Local(String), Remote { remote: String, branch: String }, Create(String) }`
  - `Repo::switch_branch(&self, target: &SwitchTarget) -> Result<(), GitError>`:
    - `Local(n)`: `switch <n>`.
    - `Remote{remote, branch}`: if `refs/heads/<branch>` exists (`show-ref --verify --quiet`), `switch <branch>`; else `switch --track <remote>/<branch>`.
    - `Create(n)`: `switch -c <n>`.
    - Before running, every name passes `check_branch_name`. A failing name returns `GitError::Failed { code: None, stderr: "Invalid branch name" }` without running git.
    - Runs through the write runner.
  - `Repo::check_branch_name(&self, name: &str) -> bool`: `check-ref-format --branch <name>` exits 0. Names starting with `-` are rejected before git runs.
  - `pub fn sanitize_branch_name(input: &str) -> String` (pure).
  - `GitBinary::run_write(&self, cwd: &Path, args: &[&OsStr]) -> Result<Vec<u8>, GitError>`, built by `GitBinary::command_for_write(&self, cwd: &Path) -> Command` with the Global Constraints write flags, waiting with no timeout.

- [ ] **Step 1: Write the failing tests** in `tests/branch.rs`. Use the existing `tests/support` helpers with isolated git config; build remotes as local bare repos plus `git clone`.
  - `branch_list_orders_and_marks_head`: three branches committed at increasing dates. Assert the order is newest first, `is_head` is set only on the current branch, and `origin/HEAD` is absent.
  - `branch_list_splits_remotes`: two remotes, `origin` and `upstream`. Assert `remote` is correct for each, and that `branch_part` of `origin/feature/x` is `feature/x`.
  - `switch_local`, `switch_create`.
  - `remote_branch_with_slashes_tracks_right_remote`: `origin/feature/x` with no local branch → local `feature/x` exists, and its `branch.feature/x.remote` is `origin`.
  - `remote_branch_with_existing_local_switches_to_it`.
  - `conflicting_changes_fail_and_keep_repo`: a modified file differs between the branches. Assert `Err(GitError::Failed{stderr})` with stderr containing "would be overwritten", and that HEAD and the file content are unchanged.
  - `non_conflicting_changes_carry_over`.
  - `switch_runs_post_checkout_hook_but_reads_do_not` (unix): `.git/hooks/post-checkout` touches `ran.txt`. After `status` it is absent; after `switch_branch` it exists.
  - `switch_rejects_option_like_names_safely`: `Create("-x")` and `Local("--orphan")` return `Err` and run no git (HEAD unchanged, no branch created).
  - `sanitize_and_check`:
    - `sanitize_branch_name("  my  new branch ") == "my-new-branch"`;
    - `check_branch_name` is false for `""`, `"a..b"`, `"-x"`, `"a b"`, and true for `"feature/x"`.
  - `write_command_is_not_hook_disabled`: `command_for_write` args lack `core.hooksPath` and `--no-optional-locks`, but include `core.fsmonitor=false` and `--no-pager`.
- [ ] **Step 2: Run** `cargo test -p lucerna-git --test branch`. Expected: compile failure (missing items).
- [ ] **Step 3: Implement.** The write runner reads stdout and stderr on helper threads, like `run_with_timeout`, but waits with `wait()` and never kills.
- [ ] **Step 4: Run** `cargo test -p lucerna-git` (all suites, including `command_is_hardened`), `cargo clippy -p lucerna-git --all-targets -- -D warnings`, and `cargo deny check`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(git): branch list, write runner and switch_branch`.

### Task 2: Checkout mode in the branch picker

**Files:**
- Modify: `crates/lucerna-workbench/src/diff_view/branch_picker.rs`. Move it to `crates/lucerna-workbench/src/git/branch_picker.rs` (or a shared spot) if both modes make that clearer; update imports.
- Modify: `crates/lucerna-workbench/src/ui.rs`: add `pub fn relative_time(then_unix: i64, now_unix: i64) -> String`, unless an equivalent helper already exists.
- Test: `crates/lucerna-workbench/tests/branch_picker.rs`, plus unit tests for `relative_time` in `ui.rs`.

**Interfaces:**
- Consumes: `BranchRef`, `sanitize_branch_name` (Task 1). `check_branch_name` results are passed in as a validity callback or precomputed, so the picker never runs git on the UI thread.
- Produces:
  - `pub enum BranchPickerMode { Compare, Checkout }`
  - `BranchPicker::new_checkout(branches: Vec<BranchRef>, is_valid_name: Arc<dyn Fn(&str) -> bool + Send + Sync>, cx) -> Self`
    - `is_valid_name` is a pure check mirroring git's ref-format rules. Implement it in `lucerna-git` as `pub fn is_valid_branch_name(name: &str) -> bool`. It mirrors `check-ref-format --branch` for the common rules: no space, `..`, `~^:?*[\`, control chars, leading `-` or `/`, trailing `/` or `.lock`, `@{`, or empty.
    - Task 1's `check_branch_name` stays the authority right before switching.
  - `BranchPickerEvent::Checkout(SwitchTarget)` and `BranchPickerEvent::Dismiss`. Compare mode keeps its existing `Pick(String)`.
  - Test hooks, `#[doc(hidden)] pub`: `row_texts()` and `selected_index()` (exist), plus `create_row(&self) -> Option<(String, bool)>` giving the name and whether it is valid.
- **Behaviour (spec §4.2):**
  - **Rows in Checkout mode:**
    1. the current branch with `✓`;
    2. the other local branches, newest first;
    3. remote branches, newest first.
  - **Each row:** branch icon, name, and `relative_time` right-aligned and muted.
  - **Compare mode:** its rows and order are unchanged.
  - **Create row:**
    - shown when `sanitize_branch_name(query)` is non-empty, equals no local name, and equals no remote `branch_part`;
    - always last;
    - if `!is_valid_name`, the row is dimmed with "Invalid branch name", and Enter or a click does nothing.
  - **Enter or click:**
    - on the current branch → `Dismiss`;
    - on a local branch → `Checkout(Local)`;
    - on a remote branch → `Checkout(Remote{remote, branch: branch_part})`;
    - on the create row → `Checkout(Create(sanitized))`.
- **`relative_time` buckets:**
  - under 60 s → "just now";
  - under 1 h → "N minutes ago" ("1 minute ago");
  - under 24 h → "N hours ago";
  - under 48 h → "yesterday";
  - under 30 days → "N days ago";
  - under 365 days → "N months ago";
  - otherwise "N years ago".

- [ ] **Step 1: Write the failing tests:**
  - `checkout_rows_order_and_mark`: rows are `["✓ main …", "feature …", "origin/docs …"]`, compared by name prefix.
  - `create_row_for_new_name`: query `"new thing"` → `create_row() == Some(("new-thing", true))`, and it is the last row.
  - `create_row_compares_sanitized_names`: query `" main "` → `None`, and the selected row is `main`.
  - `create_row_hidden_when_remote_has_it`: only `origin/foo` exists; query `"foo"` → `None`.
  - `invalid_name_row_is_inert`: query `"a..b"` → `Some(("a..b", false))`, and Enter emits nothing.
  - `enter_emits_targets`: emits each of the four events above.
  - `compare_mode_unchanged`: the existing Compare tests still pass.
  - `relative_time` unit tests at each bucket boundary.
  - `is_valid_branch_name` unit tests (in `lucerna-git`) for each rule listed above.
- [ ] **Step 2: Run** `cargo test -p lucerna-workbench --test branch_picker` and `cargo test -p lucerna-git`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the same commands plus `cargo test -p lucerna-workbench --test git_diff_view` (Compare mode users). Expected: PASS.
- [ ] **Step 5: Commit** `feat(git): checkout mode in the branch picker`.

### Task 3: Title-bar button, `git.checkout`, and the switch flow

**Files:**
- Modify: `crates/lucerna-workbench/src/layout/title_bar.rs`. The branch becomes a button: hand cursor, its own mouse handlers that stop propagation, a dimmed look with "Switching…" while a switch runs, and a debug selector `title-bar-branch-button`.
- Modify: the workbench module that hosts the picker (`show_branch_picker` / `open_branch_picker`). Add `show_checkout_picker(anchor: CheckoutAnchor, window, cx)` with `enum CheckoutAnchor { TitleBar, Center }`.
- Modify: `crates/lucerna-workbench/src/git/wiring.rs`: register the command `git.checkout`, and run the switch.
- Test: `crates/lucerna-workbench/tests/git_checkout.rs`

**Interfaces:**
- Consumes: `Repo::{branch_list, switch_branch}` and `is_valid_branch_name` (Task 1); `BranchPicker::new_checkout` and `BranchPickerEvent::Checkout` (Task 2); `GitStore::{state, repo, rescan}`; `Documents::{dirty_paths, has_conflict, open_paths}`; `git_branch::read_branch`; `status_message::show`; `window.prompt`.
- Produces:
  - `Workbench::show_checkout_picker(&mut self, anchor: CheckoutAnchor, window, cx)`
  - `Workbench::is_switching_branch(&self) -> bool`
- **Behaviour (spec §4.3–§4.5):**
  - **Click on the branch, or `git.checkout`, gated on `GitState`:**
    - `Restricted` → `show_trust_modal`;
    - `Off` → "Git is off";
    - `Locating` → "Git is still loading";
    - `NoGit` / `NoRepo` → "Git is not available";
    - `Ready` / `TooManyChanges` → load `branch_list` in the background, then open the picker.
  - **Anchor:** title-bar clicks use `TitleBar`, anchored under the button and left-aligned to it. The command uses `Center` (the Quick Open position). The existing overlay rules apply: `close_modal_overlays`, focus restore, `inQuickOpen` context.
  - **On `Checkout(target)`:**
    1. If any open document is dirty or has a conflict: `status_message::show("Save or revert unsaved files before switching branches")` and stop.
    2. Set `switching = true` and notify, so the title shows "Switching…".
    3. Run `repo.switch_branch(&target)` on the background executor.
    4. On `Ok`: `switching = false`, `GitStore::rescan()`, re-read `git_branch` via `read_branch(root)`, notify.
    5. On `Err`: `switching = false`, then `window.prompt(PromptLevel::Critical, "Could not switch branches", Some(stderr.trim()), &["OK"])`.
  - **While `switching`:** clicks and `git.checkout` are ignored.

- [ ] **Step 1: Write the failing tests** in `tests/git_checkout.rs`. Use a trusted workbench on a temp repo, with `run_until_parked`.
  - `click_opens_checkout_picker_with_current_first`
  - `picking_branch_switches_and_updates_title`: HEAD changes, `git_branch()` shows the new name, and `GitStore` statuses refresh.
  - `create_branch_from_picker`
  - `unsaved_file_blocks_switch`: status message equals the exact copy, and HEAD is unchanged.
  - `conflicted_document_blocks_switch`
  - `git_refusal_shows_prompt`: use the conflict setup from Task 1. Assert a prompt appears, read it with `cx.simulate_prompt_answer("OK")` and check the prompt text via the test platform's pending-prompt API, and HEAD is unchanged.
  - `restricted_click_opens_trust_modal_and_runs_no_git`
  - `gate_messages`: for `git.enabled: false` → "Git is off"; for a non-repo → no branch shown and the command shows "Git is not available".
  - `branch_click_does_not_reach_title_bar`: mouse down, a 2-px move, mouse up on `title-bar-branch-button`. The picker opens and `title_bar_pressed` stays false. A double-click on the button does not toggle zoom.
  - `second_switch_ignored_while_running`: two `Checkout` events before parking → exactly one switch. Count with a test hook on `Workbench`, e.g. `switches_started()`.
  - `checkout_command_registered`: the command palette has `git.checkout` with title "Git: Checkout to…".
- [ ] **Step 2: Run** `cargo test -p lucerna-workbench --test git_checkout`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p lucerna-workbench` (full), `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo deny check`. Expected: PASS.
- [ ] **Step 5: Update docs.**
  - In `docs/superpowers/specs/2026-11-08-lucerna-phase1-design.md` §1.3, roadmap row 5: add "branch switching from the title bar (done on the branch, awaiting merge)".
  - Commit `feat(git): switch branches from the title bar`.

---

## Self-review notes

- **Spec coverage:**
  - §4.1: Task 1.
  - §4.2: Task 2.
  - §4.3, §4.4, §4.5: Task 3.
  - §4.6: no settings.
  - §5 tests are distributed across the three tasks.
- **The pure `is_valid_branch_name`** exists so the picker never runs git on the UI thread. `check_branch_name` (git itself) stays the authority inside `switch_branch`.
