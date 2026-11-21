# Review of the basic flows before release: plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produce a verified bug report, ranked by severity, for Lucerna's basic flows before v0.1.0.

**Architecture:** Five read-only review agents run in parallel, one flow area each (Tasks 1–5). Each bug they report is re-read by an independent verification agent (Task 6). Bugs that hold up are merged into one report and committed (Task 7). No product code is changed.

**Tech Stack:** Rust, GPUI. Tools: Agent tool (`general-purpose` subagent), Read, Grep, Bash for reading only.

**Spec:** `docs/superpowers/specs/2026-11-15-release-flow-review-design.md`

## Global Constraints

- Read code only: no build, no `cargo test`, no worktree, no edits to files in `crates/`.
- The scope is the four flow groups in the spec. Vim, LSP, extensions, Markdown preview, diff view, performance, UI, docs and license are out of scope, **unless** they break a basic flow.
- Severity: **P0** data loss or corruption, crash. **P1** a basic flow cannot be done. **P2** wrong but there is a workaround. **P3** minor.
- Keep code quotes in the report very short, just enough to point at the bug.
- Commit by specific path (`git commit -- <paths>`), with the trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Report: `docs/superpowers/reviews/2026-11-15-release-flow-review.md`.

## Review Focus

The five kinds of input most likely to hurt users. Each line is assigned to the checklist of the task in charge:

1. **An agent writes a file while the buffer has unsaved changes.** The user expects to lose neither side, and no overwrite without asking. (Task 1)
2. **Files with CRLF, a BOM, not UTF-8, or no final newline.** Expected: saved back exactly as before, except for what the user edited. (Task 1)
3. **Paths with spaces, Unicode characters (Vietnamese, NFD on macOS), symlinks, or hidden files.** Expected: open, rename, delete and save all work correctly. (Tasks 2, 3)
4. **Quitting the app, or closing a window, while a save or a file operation is half done.** Expected: no data lost and no temp file left behind. (Task 2)
5. **The open folder or open file is deleted, renamed or remounted from outside.** Expected: no crash, and the tab shows its state clearly. (Tasks 3, 4)

---

## Finding Format (shared by every review task)

Each review agent returns a list. Each item has all of these fields:

```
- id: <T{task}-{n}>, for example T1-3
  severity: P0 | P1 | P2 | P3
  flow: <flow name in the spec>
  location: <crate/src/file.rs:line>
  summary: <one sentence saying what the bug is>
  repro: <concrete user steps, or a chain of events, that lead to the bug>
  evidence: <the code path that proves it, with file:line>
  fix_hint: <one sentence>
  confidence: high | medium
```

Do not report `low` items. Do not report style, naming or performance issues. Add a `not_checked:` item listing what reading the code cannot verify (for example: focus, IME, paint order).

## Shared brief for the review agents

Each of Tasks 1–5 sends one `general-purpose` subagent. The brief consists of:
- this section,
- the Global Constraints,
- the Finding Format,
- the "Flow" and "Start reading from" parts of that task.

> You are reviewing the code of Lucerna, a Rust/GPUI code editor at `/Users/duykhanh/Projects.nosync/lucerna`, before its first release. Read code only: do not edit, build or run tests. For each flow below, follow the code from the user's action or the file system event to the final result (disk write, view update, tree update). Look for these kinds of bugs:
> - unhandled edge cases,
> - race conditions between the watcher, file writes and user actions,
> - `unwrap`, `expect` and index/slice operations that can panic on real data,
> - errors that are swallowed (`let _ =`, `.ok()`, `unwrap_or_default`) but should be reported to the user,
> - tab, buffer and tree state that drift apart.
>
> Also read the existing tests to see what is already checked. Report only bugs you can show with a concrete path through the code. The documentation of expected behavior is in `docs/features.md` and `docs/settings.md`.

---

### Task 1: Saving files and conflicts with changes on disk

**Flow:**
- Edit and save: atomic write; keep encoding, BOM and line ending style; `files.trimTrailingWhitespace`, `files.insertFinalNewline`, `files.trimFinalNewlines`, `files.eol`, `files.autoSave`.
- Save All, Save Without Formatting, Revert.
- Undo and redo (including cursor undo).
- The file changes on disk while the buffer has unsaved changes: conflict dot, conflict bar, asking before overwriting.
- Review Focus 1 and 2.

**Start reading from:**
- `lucerna-text/src/{write,load,reload,buffer,history,patch,diff}.rs`
- `lucerna-workbench/src/{save,documents,conflict_bar,editing}.rs`
- `lucerna-workbench/src/code_view/transact.rs`
- `lucerna-editor/src/merge.rs`
- `lucerna-text/tests/`

- [ ] **Step 1:** Send the brief to a subagent, with the description "Review save and conflict flows".
- [ ] **Step 2:** Check that the returned result follows the Finding Format and that every `location` points to a real file (`test -f`). For any item missing a field, send it back to the same agent (SendMessage) to complete.

### Task 2: Explorer, closing tabs and quitting the app

**Flow:**
- Explorer: New File, New Folder, Rename (an open tab follows), Delete to Trash, Delete Permanently, copy and paste files.
- Closing a tab, closing a group, closing a window and quitting the app with unsaved files (the dialog that asks; the buttons Save, Don't Save, Cancel).
- Close Folder with unsaved files.
- Review Focus 3 (the file operations part) and 4.

**Start reading from:**
- `lucerna-workbench/src/explorer/{commands,clipboard,model,view}.rs`
- `lucerna-project/src/fs_ops.rs`
- `lucerna-workbench/src/editor_groups/{model,view}.rs`
- `lucerna-workbench/src/{app,app_menu,workbench}.rs` (the quit/close parts)
- `lucerna-project/tests/fs_ops.rs`

- [ ] **Step 1:** Send the brief to a subagent, with the description "Review explorer and close flows".
- [ ] **Step 2:** Check the format and the paths as in Task 1, Step 2.

### Task 3: Opening a folder, opening a file and restoring the workspace

**Flow:**
- `luc .`, `luc <file>`, `luc <nonexistent path>`.
- Opening from Finder or the Dock, Open With (a file inside the open folder opens as a tab, any other opens a new window).
- Workspace restore (`window.restoreWindows`), Welcome page, Open Recent, Close Folder (Cmd+K F).
- Large files (over 20 MB or over 300 thousand lines), a 5 MB long line, binary files, UTF-16, Latin-1.
- A saved workspace that points to a folder or file that no longer exists.
- Review Focus 3 (the opening part) and 5 (the vanished folder part).

**Start reading from:**
- `lucerna/src/{main,target}.rs`
- `lucerna-workbench/src/{startup,open_commands,open_files,open_requests,recent,recent_picker,welcome,workspace_state,app_state,app_state_file}.rs`
- `lucerna-text/src/load.rs`
- `lucerna-workbench/src/documents.rs` (the open part)

- [ ] **Step 1:** Send the brief to a subagent, with the description "Review open and restore flows".
- [ ] **Step 2:** Check the format and the paths as in Task 1, Step 2.

### Task 4: Following the agent's changes

**Flow:**
- Reload when the file on disk changes: atomic write (rename over), delete then recreate, `git checkout` of thousands of files.
- Keep scroll position, syntax colors and folds after a reload.
- Mark a just-changed file in the tree (10 seconds).
- Git colors in the Explorer and the gutter update with `git` commands.
- The tree updates itself, respecting `.gitignore` and `files.exclude`.
- Review Focus 5 (the open file vanishing part).

**Start reading from:**
- `lucerna-project/src/{watch,worktree,coalesce,snapshot,ignored}.rs`
- `lucerna-git/src/{watch,status,repo,hunks}.rs`
- `lucerna-workbench/src/git/{mod,wiring,job,rollup}.rs`
- `lucerna-text/src/reload.rs`
- `lucerna-workbench/src/documents.rs` (the reload part)
- `lucerna-project/tests/`, `lucerna-git/tests/`

- [ ] **Step 1:** Send the brief to a subagent, with the description "Review agent-follow flows".
- [ ] **Step 2:** Check the format and the paths as in Task 1, Step 2.

### Task 5: Everyday navigation

**Flow:**
- Quick Open: file names, `>` for commands, `:` for go to line (including a line past the end of the file).
- Search in the project (Cmd+Shift+F), including a result in a file that was just deleted.
- Find in file (Cmd+F), including an invalid regex.
- Split editor (Cmd+\), tabs and preview tabs.
- Terminal: open, close, several tabs, the shell exits.
- Typing Vietnamese with an IME in the editor and the input boxes.

**Start reading from:**
- `lucerna-workbench/src/quick_open/`, `lucerna-workbench/src/search_view/`, `lucerna-project/src/{search,fuzzy}.rs`
- `lucerna-workbench/src/{find_widget,goto,text_input}.rs`
- `lucerna-workbench/src/editor_groups/`
- `lucerna-workbench/src/terminal/{panel,view,commands}.rs`, `lucerna-terminal/src/{terminal,process}.rs`
- `lucerna-workbench/src/code_view/input.rs`

- [ ] **Step 1:** Send the brief to a subagent, with the description "Review navigation flows".
- [ ] **Step 2:** Check the format and the paths as in Task 1, Step 2.

> Tasks 1–5 are independent of each other: send all five in the same message so they run in parallel.

### Task 6: Verify each finding

**Interfaces:**
- Consumes: the list of findings from Tasks 1–5, in the Finding Format.
- Produces: for each `id`, a result `verdict: confirmed | rejected | downgraded`, with the final `severity` and a one-sentence reason.

- [ ] **Step 1: Merge and deduplicate.** Two findings with the same root cause (the same `location` or the same code path) are merged into one, keeping the smaller `id` and the higher severity.
- [ ] **Step 2: Send the verification agents.** For each review task that has findings, send one `general-purpose` subagent, running in parallel. The verification agent's brief contains only the `summary`, `location` and `repro` of each item, and **not** the `evidence` or `confidence`, so the agent is not anchored on the earlier conclusion. Ask the agent to read the code and answer three questions:
  - With the current code, do the repro steps lead to this bug?
  - Is there anywhere else in the code that already prevents the bug?
  - What is the correct severity?
- [ ] **Step 3: Reconcile.** `rejected` items are dropped. `downgraded` items take the new severity. If the review agent and the verification agent disagree about a P0 or P1 item, read that `location` yourself to decide, and write down the reason.

### Task 7: Write the report and commit

**Files:**
- Create: `docs/superpowers/reviews/2026-11-15-release-flow-review.md`

- [ ] **Step 1: Write the report**, with these sections:
  1. **Summary:** the number of bugs by severity, and the P0 and P1 bugs one line each.
  2. **List of bugs:** each bug as one `### <id> [P?] <summary>` entry, with the flow, the `location` (as a relative markdown link), how to reproduce it, and a suggested fix. Sorted from P0 to P3, and within the same severity by flow group order.
  3. **Rejected:** one line per `rejected` item with the reason, so the next review does not have to look again.
  4. **To try by hand before release:** the `not_checked` items of Tasks 1–5 merged into one checklist with checkboxes.
- [ ] **Step 2: Check the links.** Every `location` in the report points to a real file:
  `grep -o '](\.\./\.\./\.\./crates/[^):]*' docs/superpowers/reviews/2026-11-15-release-flow-review.md | sed 's/](\.\.\/\.\.\/\.\.\///' | sort -u | while read f; do test -f "$f" || echo "MISSING $f"; done`
  Expected result: prints nothing.
- [ ] **Step 3: Commit.**

```bash
git add -- docs/superpowers/reviews/2026-11-15-release-flow-review.md docs/superpowers/plans/2026-11-15-release-flow-review.md
git commit -m "docs(review): basic-flow review before v0.1.0" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>" -- docs/superpowers/reviews/2026-11-15-release-flow-review.md docs/superpowers/plans/2026-11-15-release-flow-review.md
```

- [ ] **Step 4:** Send the user the summary and the link to the report. Ask them which bugs must be fixed before release.
