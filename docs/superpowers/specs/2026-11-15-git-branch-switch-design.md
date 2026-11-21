# Switching branches from the title bar

Status: approved by the user on 2026-11-15.

Research: `docs/research/2026-11-15-git-branch-switch.md`. Builds on Git 5.1 and 5.2 (`2026-11-13-git-view-design.md`), both already on `main`.

## 1. Purpose

The user wants to click the branch name on the title bar to see the list of branches and switch to another one right inside Lucerna, like Zed. Right now the branch name is just text, and clicking it does nothing.

This is the **first operation in Lucerna that changes the repo**. Every earlier Git feature was read-only. So the design prioritizes: never lose any change, and let git decide as it does when typing commands in a terminal.

### Goals
1. Clicking the branch name on the title bar opens the branch picker, placed directly under the button.
2. The command `git.checkout` "Git: Checkout to…" opens the same picker, placed where Quick Open appears.
3. The picker lists local and remote branches, with typing to filter. Choosing a branch switches to it.
4. Typing a name that does not exist yet shows a "Create branch ‹name›" row. Choosing that row creates a branch from HEAD and switches to it immediately.
5. Choosing a remote branch that has no local copy creates a local branch tracking it, then switches to it.
6. After switching, the title bar, Explorer, gutter and diff tabs update on their own.

### Not in this version
- Tags and detached HEAD.
- "Create branch from…" (picking a starting point other than HEAD).
- Stash, Migrate Changes, Force Checkout.
- Deleting branches, renaming branches, setting upstream.
- Pull or fetch before switching.
- `git.branchPrefix`, `git.branchRandomName`.

### Anti-goals
- Losing uncommitted changes, or overwriting a file being edited in Lucerna that has not been saved.
- Running git in an untrusted folder.
- Blocking the UI thread while git runs.
- Behavior different from `git switch` typed in a terminal.

## 2. Zed reference: the corresponding screen

Zed (revision `b5cd310`, `git_ui/src/branch_picker.rs`):
- The branch name button on the title bar opens a popover anchored directly under it.
- The popover has an input box and a list: branch name, commit title, time. The current branch comes first.
- Typing a name that does not exist adds a create-branch row at the end of the list. An error while switching branches is shown in the "Failed to change branch" dialog.

Where Lucerna differs from Zed:
- No All/Local/Remote filter. Instead, local branches and remote branches are two groups in order.
- No branch deletion and no adding remotes, since they are out of scope.
- Each row does not show the commit title, to keep the list compact. It can be added later.

Sketch:

```
lucerna  ⎇ main ▾                       ← click the branch name
  ┌─────────────────────────────────────┐
  │ fix-gu│                             │
  │ ✓ main                   2 hours ago│
  │   fix-gutter-click         yesterday│
  │   origin/fix-gutter-docs  3 days ago│
  │ + Create branch "fix-gu"            │
  └─────────────────────────────────────┘
```

## 3. Approaches considered

### 3.1 When there are uncommitted changes
- **A. Like Zed: run `git switch` and let git decide. Chosen** (the user's choice).
  - If the changes do not conflict, git carries them to the new branch.
  - If the changes conflict, git refuses, and Lucerna shows git's message verbatim.
  - Changes are never lost.
- B. Like VS Code: add Stash & Checkout and Migrate Changes. That is a second write operation, and it can conflict on pop. Rejected.
- C. Block entirely whenever there are any changes. Safest, but it also blocks cases git can handle. Rejected.

### 3.2 Creating a branch
**Yes, create from HEAD** (the user's choice). No "from…".

### 3.3 Picker
- **Extend the compare picker of the Changes since tab (`diff_view/branch_picker.rs`) with a "switch branch" mode. Chosen.** One component for both, in the spirit of sharing in `docs/design-principles.md` §1.1.
- Write a separate picker: duplicated code. Rejected.

## 4. Design

### 4.1 `lucerna-git`
- **`Repo::branch_list(&self) -> Result<Vec<BranchRef>, GitError>`:**
  - Runs `for-each-ref --sort=-committerdate --format=%(refname)%00%(committerdate:unix)%00%(HEAD) refs/heads refs/remotes`.
  - Drops `refs/remotes/*/HEAD`.
  - `BranchRef { name: String, remote: Option<String>, committed: i64 (unix), is_head: bool }`.
    - `name` is the short name: `main`, `origin/feature/x`.
    - `remote` is the remote name, obtained by matching the `refs/remotes/<remote>/` prefix against the `git remote` list. The remote name must never be split at the first `/`, because `origin/feature/x` could be remote `origin` with branch `feature/x`.
  - The existing `Repo::branches()`, used by Compare mode, stays unchanged. It is not required to switch it over to `branch_list`.
- **`Repo::switch_branch(&self, target: SwitchTarget) -> Result<(), GitError>`:**
  - `SwitchTarget::Local(name)` runs `switch <name>`.
  - `SwitchTarget::Remote { remote, branch }` (for example `origin`, `feature/x`):
    - if `refs/heads/<branch>` already exists, run `switch <branch>`;
    - otherwise run `switch --track <remote>/<branch>`.
  - `SwitchTarget::Create(name)` runs `switch -c <name>`.
  - Every name goes after `--` where git allows it, or is checked beforehand, so a name starting with `-` is not read as a flag.
- **Running write commands:**
  - Still uses `--no-pager` and `-c core.fsmonitor=false`.
  - Does **not** set `core.hooksPath=/dev/null`: the `post-checkout` hooks of git-lfs, husky… run as they do in a terminal. Write commands run only in trusted folders (§4.5).
  - Drops `--no-optional-locks`, since this is a write command.
  - Runs with the login shell's environment (overriding the process environment), taken from `login_shell_env()`, which `GitStore` already reads and caches on a background thread; drop `PWD`, `OLDPWD`, `SHLVL`, `_`. When the app is opened from the Dock, `PATH` has only `/usr/bin:/bin:/usr/sbin:/sbin`, so without this step husky's `git-lfs` or `node` cannot run, and a broken `required` filter would leave a half-switched working tree. Tests inject the environment with `GitBinary::with_write_env`.
  - Does **not** set `LC_ALL=C`: hooks run with the user's locale as in a terminal. Only read commands, whose output Lucerna parses, use `LC_ALL=C`.
  - **No timeout and never killed.** Killing git midway can leave a half-switched working tree or a stale `index.lock`. A checkout with LFS or hooks can take a while, so just let it finish.
  - Add a dedicated function on `GitBinary`, something like `command_for_write`. The `command()` function for read commands and the `command_is_hardened` test stay unchanged.
- **`pub fn sanitize_branch_name(input: &str) -> String`:**
  - Trims whitespace at both ends.
  - Replaces each run of whitespace with `-`, as with VS Code's default `git.branchWhitespaceChar`.
- **`Repo::check_branch_name(&self, name: &str) -> bool`:** runs `git check-ref-format --branch <name>`.

### 4.2 Picker
Extend `BranchPicker` with modes `Compare` (as now) and `Checkout`.

**Checkout mode:**
- **Placeholder:** "Switch to a branch or type to create one".
- **Row order when unfiltered:**
  1. the current branch (✓ mark);
  2. the other local branches, newest commit first;
  3. the remote branches, newest commit first.
- **Each row:** a branch icon, the branch name, and on the right the relative time of the last commit, muted ("2 hours ago", "yesterday", "3 days ago"). Use an existing helper if there is one; if not, create it in `ui.rs`.
- **Filtering:** uses `fuzzy_filter` as now.
- **The create-branch row:**
  - Compared by normalized name. The row appears when `sanitize_branch_name(query)` is not empty, does not equal any local branch name, and does not equal the `branch` part of any remote branch. If only `origin/foo` exists, typing "foo" has the user choose the `origin/foo` row to get a branch `foo` tracking it, rather than creating an unrelated `foo` branch. The row sits at the end of the list: "+ Create branch "‹normalized name›"".
  - If the name is not valid per `check_ref_format`, the row still appears but is dimmed, with "Invalid branch name", and choosing it does nothing.
  - Typing exactly the name of an existing branch shows no create row, only that branch.
- **Choosing the current branch:** just closes the picker.
- **Choosing another branch or the create row:** closes the picker, then switches branches (§4.3).

**Position:**
- Opened from the title bar: a popover anchored under the branch name button, left-aligned with the button.
- Opened from the command: Quick Open's position.
- Both use `ui::popover` and parts of `quick_open::picker`.

The branch list is loaded on a background thread as now. The picker appears as soon as loading finishes, keeping the existing behavior.

### 4.3 Switching branches
1. **Check for unsaved files.** If `Documents::dirty_paths()` is not empty, or any document is in a conflict state with disk (`has_conflict`), do not run git. The status bar reports "Save or revert unsaved files before switching branches" and the picker stays closed.
   - Reason: git writes new files to disk. An unsaved buffer would fall into a conflict state with disk. Lucerna already has a way to handle that state (Phase 4.4), but the user did not intend to cause it.
2. **Run `Repo::switch_branch` on a background thread.** While it runs:
   - the branch name on the title bar dims, with the text "Switching…";
   - the `git.checkout` command and clicks on the branch name are ignored;
   - the editor remains editable. If the user edits a file that git is rewriting, Phase 4.4's conflict mechanism handles it as when an agent writes the file. This is a known gap and acceptable, since a branch switch is usually done in under a second.
3. **Success:** nothing more to do. The 5.1 watcher sees `HEAD` and `index` change and rescans. The worktree watcher sees files change so Documents reloads the buffers. The title bar re-reads `HEAD`.
   - To avoid waiting for the watcher, additionally call `GitStore::rescan()`, and re-read the branch name with `git_branch::read_branch`, right after the command finishes. The branch name on the title bar is read from `HEAD` through `git_branch.rs`, not from `GitState`.
4. **Failure** (git returns an error code, or git could not run): still call `GitStore::rescan()` and re-read the branch name as on success, because git may have switched branches and then reported an error (for example a `post-checkout` hook returning a nonzero code). Then show a native dialog `window.prompt(PromptLevel::Critical, <title>, Some(<git's stderr, whitespace trimmed>), ["OK"])`, the way Zed reports errors.
   - If `HEAD` is now the target branch, the title is "Switched branches, but git reported an error".
   - Otherwise the title is "Could not switch branches". When git refuses because of conflicting changes, the repo is left unchanged.

### 4.4 Commands and keys

| Command | Title | Key | Notes |
|---|---|---|---|
| `git.checkout` | Git: Checkout to… | none | VS Code's ID. Opens the picker in Checkout mode |

Clicking the icon or the branch name on the title bar also opens the picker. The mouse cursor becomes a hand on hover. The branch name button must stop mouse events from propagating to the title bar, so the click does not become a window drag or a double-click zoom.

### 4.5 Restricted Mode and git state
The branch name on the title bar can appear even when git is not running, because it reads the `HEAD` file directly. So a click must check `GitState`:
- `Restricted` (untrusted folder): open the trust dialog (`workbench.trust.manage`). The `git.checkout` command does the same.
- `Off`, `Locating`, `NoGit`, `NoRepo`: do not open the picker. The status bar shows one line:
  - `Off`: "Git is off";
  - `Locating`: "Git is still loading";
  - `NoGit` and `NoRepo`: "Git is not available".
- `TooManyChanges` can still switch branches.

### 4.6 Settings
No settings are added.

## 5. Tests

### `lucerna-git` (temporary repos created with the `git` command)
- `branch_list`: correct order by date; marks `is_head`; drops `origin/HEAD`; separates Local and Remote.
- `switch_branch`:
  - switch to a local branch;
  - a remote branch with no local copy creates a tracking branch;
  - a remote branch that already has a local copy switches to the local one;
  - create a new branch;
  - conflicting changes return an error with git's stderr, and the repo is unchanged;
  - non-conflicting changes are carried over.
- The `post-checkout` hook runs when switching branches, while read commands still do not run hooks (a `checkout` run through the read command does not trigger the hook).
- A `required` filter whose program exists only on the write environment's `PATH`: the switch succeeds.
- A remote branch with `/` in its name (`origin/feature/x`) can create the local branch `feature/x`.
- `sanitize_branch_name` and `check_branch_name`: whitespace, a name starting with `-`, `..`, an empty name.

### `lucerna-workbench/tests/`
- Clicking the branch name opens the picker in Checkout mode. The current branch comes first, with ✓.
- Choosing a branch: HEAD changes, the title bar shows the new name, statuses and hunks update.
- Typing a new name: a "Create branch …" row appears. Enter creates the branch and switches to it.
- Typing an invalid name: the create row is dimmed, Enter does nothing.
- With unsaved files: git does not run, the status bar shows the message, HEAD does not change.
- Git refuses: an error dialog appears (using the test's fake prompt), HEAD does not change.
- A `post-checkout` hook fails after the switch: the dialog is titled "Switched branches, but git reported an error", the title bar shows the new branch, statuses are rescanned.
- Restricted Mode: clicking the branch name opens the trust dialog, no git command runs.
- Compare mode of the Changes since tab still works as before.

## 6. Left to the plan writer
- How to organize the modes in `BranchPicker` (an enum mode, or passing in a delegate), and whether to move the picker out of `diff_view/` to a shared location.
- How to get the branch name button's position to place the popover.
- The relative time format.

## 7. Decisions I made (the user approved them together by saying "ok")
- The switch command runs hooks normally. Read commands still have hooks off.
- The write command has no timeout and is never killed.
- Block branch switching while there are unsaved files.
- Report errors with a native dialog.
- Each row does not show the commit title. No Local/Remote filter.
- The create-branch row with an invalid name still appears, but dimmed. No create row when the typed name equals a remote branch.
- Editing a file while the branch switch is in progress is a known gap and is accepted.
- Also block branch switching when a document is in conflict with disk.
