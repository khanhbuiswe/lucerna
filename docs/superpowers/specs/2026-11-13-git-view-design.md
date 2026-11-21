# View-only Git (phase 5)

Status: approved by the user on 2026-11-14. The user's decisions: do it like Zed, call the `git` command, the default base is HEAD, a single column, git turns on the trust dialog.

VS Code and Zed research: `docs/research/2026-11-13-git.md`.

## 1. Purpose

Lucerna users let agents (Claude Code, Codex) write the code, and **the agent commits by itself**. After each run, the user needs to see three things quickly:
- which files the agent changed;
- where in the file it changed them;
- what exactly it changed.

Lucerna currently has only the branch name in the title bar (`git_branch.rs`). There is no git gutter and no diff yet.

The first version is for **viewing** only: no stage, no commit, no revert. The UI follows Zed. Commands, keys and settings follow VS Code.

### Goals
1. Color the git status of files and folders in the Explorer.
2. Change bars in the editor gutter, against the `git.diffBase` base (default HEAD).
3. Open the diff of a file (`git.openChange`).
4. An **Uncommitted Changes** tab: gathers every uncommitted change into one view.
5. A **Changes since ‹branch›** tab: gathers every change from the branch point (merge-base) to the version on disk, including the agent's commits. The base branch can be changed.
6. Updates itself when the agent writes a file, commits, switches branches or rebases. No need to press refresh.
7. Never blocks the UI. Every `git` command and every diff runs in the background.

### Not in this version
- Stage, unstage, commit, revert hunk, push, pull, switch branch, Git panel.
- Side-by-side diff. This version has only one column, with removed and added lines interleaved.
- Expanding a hunk right in the editor (Zed style) and the peek frame (VS Code style).
- Commit View (the diff of one commit), history, blame.
- Word diff (highlighting individual words within a line).
- Change bars on the minimap and overview ruler.
- Several repos in one window, and submodules. Only the repo that contains the workspace root is used.
- Editing code right in the diff tab. The tab is read-only.

### Anti-goals (if it works like this, it counts as broken, even if it runs)
- A janky UI when the agent writes hundreds of files, or when opening a large repo.
- Triggering macOS's Command Line Tools install dialog on a machine that has no git.
- Running a command specified by the config of an unknown repo, when the folder is not yet trusted.
- Results that differ from the `git status` / `git diff` that the agent and the user see in the terminal.

## 2. References: the matching Zed screens

The source is Zed's source code at revision `b5cd310`. See the research file for details.

- **Gutter.**
  - A bar `floor(0.275 × line_height)` wide at the left edge of the gutter, square corners, colored `version_control.added` or `version_control.modified`.
  - Where a line was deleted: a pill colored `version_control.deleted`, one line tall, placed between two lines and cut in half at the left edge.
- **Project panel.** The file name is colored by `created`, `modified`, `deleted`, `conflict`, `ignored`. A folder takes the color of its most important child. No letters.
- **Uncommitted Changes.**
  - A tab in the editor area, made of files one after another. Each file has a header (icon, name, dimmed path).
  - Under the header, each hunk is pre-expanded, with 2 lines of context. Removed lines have the `editor.diff_hunk.deleted.background` background, added lines have the `editor.diff_hunk.added.background` background.
  - File order: conflict, then tracked files, then new files. Deleted files are collapsed by default.
  - With no changes, it shows the text "No uncommitted changes" in the center.
- **Changes since main.** Same layout. The tab is named after the base branch. The toolbar has a branch picker.

**Where Lucerna differs from Zed, and why:**
- No Stage, Commit or Restore buttons on the toolbar and in the hunk, because this version is view-only.
- No toggle between one column and two columns, because two columns are left for a later version.
- The tab is read-only. Lucerna builds the tab on the excerpt list (shared with Search and References), not on the editor.

**Review at 5.2 (Task 6).** I read the tab-drawing code (`diff_view/render.rs`, `excerpt_list/render.rs`) against the description above: header (icon, name, dimmed path), added/removed line background by `diff_*_background`, change bar at the gutter edge, removed lines with no line number, the toolbar built with `search_toolbar` / `search_bar_row` from `ui.rs`, a centered empty message ("No uncommitted changes"), deleted files collapsed by default. I found nothing that differs. Putting it next to Zed's `git: diff` on the same repo for a visual comparison (header height, colors, toolbar spacing) was not possible because this session cannot drive two GUI apps. **Left for the user to compare by eye**; wherever it differs, fix it per `docs/design-principles.md` §1.1 or record it here with the reason.

## 3. Approaches considered

### 3.1 The base to compare against
The agent commits by itself, so if we compare only with HEAD, the gutter and the tab would be empty right after the agent commits.

- **A. Compare with the branch's merge-base** (Zed `git::DiffBranch`). Shows the whole task when the agent works on its own branch. **Chosen**, as a separate tab.
- **B. View commit by commit** (Zed's Commit View). Left for a later version.
- **C. A "reviewed up to here" base.** Lucerna saves a commit by itself as the base. Neither VS Code nor Zed has this. **Dropped**: the user chose to do it like Zed.

The gutter, the Explorer colors and `git.openChange` follow Zed's `git.diffBase` setting. The default is `"head"`, **the user's choice**.

### 3.2 Reading git
- **Call the `git` command, like Zed. Chosen.** The binary does not grow, and the result always matches the agent's git. Needs a machine with git.
- `gix`: fastest (~60 ms on 4,400 files), but the binary grows 3.3 MB and pulls in ~440 crates. The API is still 0.x, and filters and submodules are not yet complete.
- `git2`: grows 0.8 MB but is slowest, needs a C build, and libgit2 behaves differently from git in some places.

All three pass `cargo deny`. Detailed measurements are in the research file, section 1.

### 3.3 Showing the diff
- **One column, built on the excerpt list. Chosen.** Reuses the view that Search and References use.
- Two columns like the default in Zed and VS Code: needs a new view, with the lines on the two sides staggered. Left for a later version.

## 4. Design

### 4.1 Crate split
- **`lucerna-git` (a new crate, no gpui dependency).** Everything that touches git and computes diffs:
  - finding the repo and finding the `git` binary;
  - running `git` commands;
  - parsing `status` output;
  - the `cat-file --batch` process;
  - finding the default branch and the merge-base;
  - line diff into a list of hunks.

  Can be tested without UI.
- **`lucerna-workbench`.** The part wired into the app:
  - `GitStore`: one entity per window;
  - the gutter;
  - Explorer colors;
  - the diff tabs;
  - commands, settings and theme.

### 4.2 Finding git and the repo
- **Finding the `git` binary** (the job of `lucerna-workbench`; `lucerna-git` only takes a path to the `git` binary, so it does not depend on `lucerna-lsp`):
  1. The `git.path` setting (from VS Code), if set.
  2. If not set, look for `git` in the login shell's PATH: `lucerna_lsp::env::login_shell_env()` then `which`. This function can wait up to 5 s, so it runs in the background and runs once per session. The git features turn on when it finishes.
  3. On macOS, if the result is `/usr/bin/git`, check whether the Command Line Tools are installed (`xcode-select -p` succeeds) before running it. If not installed, treat it as no git.
  4. No git: turn off every git feature, and the status bar reports once "Git not found" with a hint to install it.
- **Finding the repo:** reuse what `git_branch.rs` does today (look for `.git` from the root folder upward, follow the worktree's `gitdir:`), then ask `git rev-parse --show-toplevel --git-dir --git-common-dir` to get:
  - the worktree root;
  - the worktree's own git directory;
  - the common git directory (containing `refs`).

  No repo: turn the features off, say nothing.
- **Restricted Mode:** an untrusted folder (`trust.rs`) **does not run git**, like VS Code. Reason: a repo's local config (`core.fsmonitor`, `filter.*.clean`) can run arbitrary commands even when only reading the status. The branch name in the title bar still shows, because it reads the file directly. When the user trusts the folder, git turns on immediately, no reopening needed.
  - **Git also triggers the trust dialog** (the user's choice). Opening a workspace inside a repo whose folder is not yet trusted makes the trust dialog from `trust.rs` appear automatically, as it does when opening a file with a language server. The dialog appears automatically only once per session, as it does today. The "Restricted Mode" button in the title bar can still reopen it.
  - The dialog text must mention Git, not only language servers, for example "Language servers and Git run only in trusted folders".
  - If the user dismisses the dialog: the gutter and Explorer show nothing, and the diff tab shows one line of explanation with a button that opens the trust dialog.

### 4.3 Running `git` commands
- Every command has `-c core.fsmonitor=false -c core.hooksPath=/dev/null --no-optional-locks --no-pager` and `GIT_TERMINAL_PROMPT=0`, like Zed. `--no-optional-locks` keeps Lucerna from fighting the agent for the `index.lock` lock.
- **Work queue:** each repo has one queue running sequentially on a background thread. A keyed job ("scan everything", "refresh paths") replaces a waiting job with the same key. This follows Zed.
- **Status:** `git status --porcelain=v2 -z --untracked-files=all --no-renames [-- <paths>]`. A rename is shown as a delete plus an add, like Zed.
- **File contents at the base:** one `git cat-file --batch` process alive for the whole session. Send `‹rev›:‹path›`, receive the blob. If the process dies, restart it when needed.
- **Default branch:** try in turn `refs/remotes/upstream/HEAD`, `refs/remotes/origin/HEAD`, then the local branch `main`, then `master`. If none exists, treat it as "no default branch".
  - Because remotes are tried first, when the agent commits straight to `main` the base is `origin/main`. The tab can still show the unpushed commits.
  - Only when the repo has no remote and the agent commits straight to `main` is the merge-base HEAD, and the tab is identical to Uncommitted Changes. Zed does the same. A "reviewed up to here" base (approach C) is for a later version, if needed.
- **Merge-base:** `git merge-base ‹ref› HEAD`. The list of files changed against the merge-base is `git diff --name-status -z --no-renames ‹merge-base›` (compares the merge-base's tree with the version on disk), plus the untracked files taken from status.
- **Errors:** if a command fails or runs longer than 10 s, keep the old state, log it, and the status bar reports once. An error never removes the colors or bars currently shown, unless the repo is gone.

### 4.4 The comparison base: `git.diffBase`
- `"head"` (default, like Zed): the base is HEAD.
- `"defaultBranch"`: the base is the merge-base of HEAD with the default branch. If no default branch is found, use HEAD.
- This base applies to the gutter, the Explorer colors and `git.openChange`.
- The two tabs have their own bases: Uncommitted Changes always compares with HEAD, Changes since ‹branch› always compares with the merge-base of the selected branch.
- When HEAD or the default branch changes (agent commit, rebase, fetch), recompute the merge-base.

### 4.5 Explorer
- Color the file name by its status against the `git.diffBase` base, in Zed's priority order: conflict, deleted, modified, created (including untracked files), ignored. A file counts as changed whether or not it is staged.
- A folder takes the color of its most important child, accumulated through every level as in Zed. Each time the status changes, this aggregate is recomputed on a background thread.
- Zed's theme color keys: `created`, `modified`, `deleted`, `conflict`, `ignored`. If the theme lacks a key, use VS Code's default `gitDecoration.*` color (see the research file, section 4).
- No letters shown. `explorer.decorations.badges` defaults to `false`, **a deviation from VS Code** so the UI looks like Zed. When on, it shows VS Code-style letters at the right edge of the row: `M`, `U`, `D`, `!`.
- The existing "agent just wrote" mark (`explorer.decorations.externalChanges`) stays: bold text and a dot. The git color only changes the text color, so the two can show together.
- The Explorer always hides gitignored files (`worktree.rs`, `.git_ignore(true)`), so this version does not use the `ignored` color. It still reads this key for later use.
- The list of colored files comes from `git status`, that is, the contents on disk. A file edited in Lucerna but not yet saved has a bar in the gutter but does not change color in the Explorer until saved. I accept this discrepancy, because Lucerna users mainly read changes the agent wrote to disk. The diff tab is the same: the file list follows the disk, the content follows the buffer.

### 4.6 Gutter
- **Base and when it is computed:**
  - Compare the content **currently in the buffer**, including the unsaved part, with the blob at the `git.diffBase` base. Same as both Zed and VS Code.
  - Re-diff the whole file in a background task, debounced 50 ms after the last edit.
  - The blob at the base is reloaded only when the base changes, or when that file itself changes status.
  - The result is a list of line hunks: added, modified, removed. Keep the old result until a new one arrives, so the bars do not flicker.
- **Diff:** uses `similar` by line (a crate already in the workspace). Ignore differences that lie only in the line-ending characters (CRLF/LF), so a file changed by `core.autocrlf` is not highlighted entirely.
- **When there is no bar:**
  - an untracked file, or a file outside the repo (like Zed `tracked_files` and VS Code);
  - exception: with `"defaultBranch"`, a file tracked at HEAD but not yet present at the merge-base (the agent created it and committed on the branch) is compared against empty content, so every line is an added line, and the Explorer colors it `created`. Same as Zed;
  - a binary file;
  - a file larger than 8 MB (a threshold I set, see §6).
- **Drawing** follows Zed:
  - The bar at the left edge of the gutter, to the left of the diagnostics icon column, `floor(0.275 × line_height)` wide.
  - An added line is colored `version_control.added`, a modified line is colored `version_control.modified`.
  - A removed line: a pill colored `version_control.deleted` at the boundary between two lines.
  - The color is blended onto `editor.background`, as in Zed. If the theme lacks a key, use VS Code's default `editorGutter.*Background` color.
- **Clicking the bar:** opens the file's diff (§4.8), scrolled to that hunk. The mouse cursor becomes a hand when hovering over the bar.
- **Settings** (VS Code):
  - `scm.diffDecorations`: `"all"` (default) or `"gutter"` shows the bar, `"none"` turns it off. `"overview"` and `"minimap"` are accepted but draw nothing yet, because this version has no bar on the minimap.
  - `scm.diffDecorationsGutterAction`: `"diff"` (default) or `"none"` (clicking the bar does nothing).

### 4.7 Diff tabs (Uncommitted Changes and Changes since ‹branch›)
- **Two new kinds of tab in `TabItem`, at most one tab of each kind in every group.** They are like Search and References: not preview, cannot be split. They are saved in the workspace (like Search) to reopen at startup. The branch tab also saves the selected branch name.
- **Tab title:**
  - "Uncommitted Changes";
  - "Changes since ‹branch name›", for example "Changes since main".
- **The top bar.** Uses `search_toolbar` / `search_bar_row` in `ui.rs`, like References:
  - on the left: the number of files and the total lines `+N −M`;
  - on the right: previous-hunk and next-hunk buttons, collapse all / expand all, refresh;
  - the branch tab also has a branch-name button. Clicking it opens a branch picker (local and remote, taken from `git for-each-ref`), using the existing picker component.
- **Content: the excerpt list, extended for diffs.**
  - Each file has a header like Search, plus `+N −M`. A new file and a deleted file have a small "new" / "deleted" label.
  - Each hunk is an excerpt with 2 lines of context on each side, as in Zed. Hunks that are close together are merged.
  - Each line is one of three kinds: context, added, removed.
    - An added line: `editor.diff_hunk.added.background` background, plus a `version_control.added` bar in the gutter.
    - A removed line: `editor.diff_hunk.deleted.background` background, plus a `version_control.deleted` bar. The gutter has no line number, like Zed.
    - If the theme lacks a key, use VS Code's default `diffEditor.insertedLineBackground` / `removedLineBackground`.
  - Syntax highlighting for removed lines too: taken from the blob content at the base, not from the file on disk.
  - File order: conflict, then tracked files, then new files, as in Zed. Within each group, sort by path.
  - A deleted file is collapsed by default. A binary or too-large file (§6) has only a header, with the line "Binary file" or "File too large to diff".
- **Behavior:**
  - Click a line, or press `Enter`: opens the file at that line. For a removed line, it opens at the line right after where it was removed.
  - The "open file" button on the header: opens the file. For a deleted file the button is disabled.
  - The keyboard moves between lines as in Search.
  - Alt+F5 / Shift+Alt+F5 jump to the next / previous hunk (§4.9).
- **Content source:**
  - A file open in Lucerna: use the buffer, including the unsaved part.
  - A file not open: read the file on disk.
  - Everything runs in the background, at most 16 files at a time as in Zed. The file list shows immediately, and each file's hunks pour in gradually, as Search does with results.
- **Auto-update:**
  - Repo state changes: rebuild the file list.
  - A file changes (on disk or in the buffer): recompute only that file.
  - The selected line and the scroll position are kept, anchored by path and line number as the excerpt list does.
- **Empty states**, text in the center as in Zed:
  - "No uncommitted changes";
  - "No changes since ‹branch›";
  - No default branch: "No default branch found", with a branch-picker button.
  - No repo / no git / Restricted Mode: state the reason in one line.

### 4.8 The diff of one file (`git.openChange`)
- Opens a tab "‹file name› (Changes)", using the same view as §4.7 but with only one file and **showing the whole file**, like Zed's `git.file_diff.show_full_file`. Each file has at most one tab of this kind, and it is not a preview.
- The base is `git.diffBase`.
- Untracked file: every line is an added line. A file with no changes: the tab shows the whole file, with no colored lines.
- The top bar has previous/next hunk and an "Open File" button (`git.openFile`).
- Can be opened from:
  - the Command Palette, for the file in the active tab;
  - clicking the bar in the gutter;
  - the `git.openChange` command attached to the Explorer's right-click menu, once that menu exists.

### 4.9 Commands and keys

| Command | Title | Default key | Notes |
|---|---|---|---|
| `git.viewChanges` | Git: Open Changes (Uncommitted) | none | VS Code ID |
| `lucerna.git.viewBranchChanges` | Git: Open Changes Since Branch | none | VS Code has no matching command. Equivalent to Zed `git::DiffBranch` |
| `git.openChange` | Git: Open Changes | none | VS Code |
| `git.openFile` | Git: Open File | none | VS Code. Used in the diff tab |
| `git.refresh` | Git: Refresh | none | VS Code. Rescan everything |
| `workbench.action.editor.nextChange` / `previousChange` | Go to Next/Previous Change | Alt+F5 / Shift+Alt+F5 | VS Code. In the editor: moves the cursor to the start of the next hunk, wrapping around past the end of the file. In the diff tab: jumps between hunks across all files |
| Vim `]c` / `[c` | | | Like Vim and Zed. Share the command above |

### 4.10 New settings (`default-settings.json`)

| Key | Default | Source |
|---|---|---|
| `git.enabled` | `true` | VS Code |
| `git.path` | `null` | VS Code |
| `git.decorations.enabled` | `true` | VS Code. Turns off Explorer colors |
| `git.diffBase` | `"head"` | Zed `git.diff_base` (`"head"` / `"defaultBranch"`) |
| `git.statusLimit` | `10000` | VS Code |
| `explorer.decorations.colors` | `true` | VS Code |
| `explorer.decorations.badges` | `false` | VS Code. The default differs, see §4.5 |
| `scm.diffDecorations` | `"all"` | VS Code |
| `scm.diffDecorationsGutterAction` | `"diff"` | VS Code |

A setting change applies immediately, with no restart needed.

### 4.11 Theme
Add Zed theme's keys to `UiColors`. Each key has a fallback color for dark and light, taken from VS Code's defaults (see the research file), placed in `UiColors::fallback`:
- `version_control.added`, `version_control.modified`, `version_control.deleted`;
- `created`, `modified`, `deleted`, `conflict`, `ignored`;
- `editor.diff_hunk.added.background`, `editor.diff_hunk.deleted.background`.

### 4.12 Refreshing
Lucerna refreshes like Zed (fast), not like VS Code (waits 1 s and waits for the window to be focused). Reason: users want to see it immediately when the agent writes a file.

- **A file in the worktree changes** (the `lucerna-project` watcher already batches every 100 ms): queue a "refresh paths" job that runs `git status -- <paths>`, then recompute the gutter diff of those open files among them.
- **The git directory changes** (`index`, `HEAD`, `refs/**`, `packed-refs`, `MERGE_HEAD`): queue a "scan everything" job. It recomputes the status, the base and the merge-base, then reloads the blobs at the base for the open files.
  - Ignore `objects`, `hooks`, `lfs`, `logs`, `rebase-*`, `sequencer`, `fsmonitor--daemon`, `*.lock`, `COMMIT_EDITMSG`, `FETCH_HEAD`, `ORIG_HEAD`, `gc.pid`.
  - The watcher currently ignores everything in `.git`. The worktree's git directory and the common git directory can lie outside the root folder. So `GitStore` needs its own watcher (using `notify` like `lucerna-project::watch`) on those two directories, not recursing into `objects`.
- **Agent commit:** `index` and `refs/heads/...` change, which leads to a full scan, and the bars in the gutter disappear (with a HEAD base). That is as intended.
- **Too many entries:** if status has more than `git.statusLimit` entries:
  - stop auto-refreshing for that repo, keep the last result;
  - the status bar reports once "Too many changes in this repository; automatic Git updates are off. Run Git: Refresh to update.";
  - the diff tab can still be opened, but updates only when `git.refresh` is run.

  Same as VS Code `isRepositoryHuge`.

## 5. Performance

| Task | Threshold |
|---|---|
| `git` commands and diffs on the UI thread | never. The UI thread only receives results that are already computed |
| Opening a workspace: Explorer colors appear | ≤ 500 ms after the tree finishes scanning, for a repo of 10,000 files and 100 changed files. The Explorer shows immediately, the colors come later |
| Agent writes 1 file → gutter bar and Explorer color update | ≤ 300 ms after the watcher event |
| Agent writes 500 files in 1 s | No dropped UI frames. Refreshes coalesce, with at most one "refresh paths" job pending |
| Agent commits → gutter and Explorer update | ≤ 1 s, for a repo of 10,000 files |
| Typing in a 10,000-line file | gutter diff each time ≤ 20 ms on the background thread (measured ~6 ms with 13,000 lines), debounced 50 ms. No frame lag |
| Opening the Uncommitted Changes tab with 200 changed files | headers appear ≤ 200 ms, hunks of the files in the viewport ≤ 500 ms |
| Memory | the blob at the base is kept only for open files and for files shown in a diff tab. Released on close |

Benchmark on the Zed repo at the pinned revision (4,401 files, 98 MB of source) and the Lucerna repo. Record the results in the research file.

## 6. Edge cases
- **Large or binary files.**
  - A file larger than 8 MB is not diffed. The threshold is mine: VS Code uses 50 MB, Zed has no limit, but 8 MB is enough to keep the diff under a few tens of ms.
  - A file is considered binary if it has a NUL byte in the first 8 KB, the same way git guesses.
  - These two kinds have no bar in the gutter. In the diff tab they have only a header.
- **Unsaved buffer:** the gutter and diff tab use the buffer content. Clicking "Revert" in Lucerna returns the bars to match the version on disk.
- **Merge conflicts** (status `UU`, `AA`, ...):
  - The Explorer colors it `conflict`.
  - The diff tab puts that file first and shows the diff against HEAD (the part with `<<<<<<<` markers becomes added lines).
  - There is no conflict-resolution UI.
- **Detached HEAD, a repo with no commits yet:** with no HEAD every file is new. The gutter has no bars, because the files are all untracked.
- **Switching branches, rebase:** full scan (§4.12). The diff tab keeps its position if the file is still in the list.
- **Symlink:** follows git: the symlink's blob is the target path, the content is not diffed, only the header shows.
- **The root folder is a subfolder of the repo:** only files under the root folder are shown. Paths convert between "relative to the repo" and "relative to the workspace".
- **Single-file mode** (`2026-11-13-single-file-open-design.md`): the gutter is still there if the file is in the repo and the folder is trusted. No Explorer, so no colors.
- **Encoding and line endings:** blobs are decoded with `lucerna_text::decode_file`, the same path Lucerna uses to open files (BOM, UTF-8, then chardetng). This function also turns CRLF into LF. Because the buffer is already in LF form, the CRLF-ignoring rule in §4.6 only needs to normalize the blob side.

## 7. Tests
- **`lucerna-git`** (uses a temp repo, created with the `git` command in the test):
  - parsing `status --porcelain=v2 -z`, including conflicts, new files, deletions, paths with spaces and Unicode characters;
  - `cat-file --batch`: reading many blobs, a blob that does not exist, the process dying and restarting;
  - finding the default branch in the right order, the merge-base, the list of files changed against the merge-base;
  - hunks of the right added / modified / removed kind, CRLF not counted as a change, binary detection;
  - with no git it reports a clean error, no panic;
  - no hooks and no fsmonitor run (the test repo installs a hook that writes a file, check that file does not appear).
- **`lucerna-workbench/tests/`:**
  - Explorer: file and folder colors by status; turned off by `git.decorations.enabled` and `explorer.decorations.colors`; letters when `explorer.decorations.badges` is on;
  - gutter: editing the buffer changes the bars; saving, then committing from outside, makes the bars disappear; with `git.diffBase` `"defaultBranch"` the bars remain after the commit;
  - Alt+F5 / Shift+Alt+F5 wrap around between hunks; `]c` / `[c` when Vim is on;
  - the Uncommitted Changes tab: file order, removed and added lines, clicking a line opens the right position, the empty state, keeping the selected line when a file changes;
  - the Changes since tab: includes both the commits on the branch and the uncommitted part; changing branch through the picker; no default branch;
  - Restricted Mode: no `git` command runs; opening a workspace in an untrusted repo makes the trust dialog appear automatically; trusting the folder turns the features on;
  - exceeding `git.statusLimit`: auto-refresh stops, `git.refresh` still runs.
- **Benchmark** (section 5), run by hand, not put into CI.

## 8. Left for the plan author to decide
- How `GitStore` hands state to the Explorer and CodeView: an immutable snapshot with a generation number, following the principle "the UI thread only reads snapshots".
- The data structure for per-path status and the aggregate for folders (a summarized tree like Zed, or a map plus a precomputed aggregate), as long as it meets §5.
- How to extend the excerpt list for removed lines and a highlight source taken from the blob, so Search and References are not affected.
- How to split the work into mergeable steps. Suggested order:
  1. `lucerna-git` and `GitStore`;
  2. Explorer colors;
  3. gutter;
  4. the Uncommitted Changes tab;
  5. the diff of one file;
  6. the Changes since tab.

## 9. Decisions I made (the user has not approved each item)
- `explorer.decorations.badges` defaults to `false`.
- The 8 MB threshold for diffs. The performance thresholds in section 5.
- Clicking a gutter bar opens the file's diff tab, instead of expanding a hunk or opening a peek frame.
- Diff tabs are saved in the workspace.
- The custom command name `lucerna.git.viewBranchChanges`.
