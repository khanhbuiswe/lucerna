# Code review of the basic flows before the v0.1.0 release

## Purpose

Before making the repo public and releasing v0.1.0, find bugs in the actions every new user takes in their first 10 minutes. A bug in a side feature is still acceptable. A bug in a basic flow, especially one that loses data, is not.

## Scope

The flows, ordered by the consequence if they break:

1. **No data loss.** Edit and save: atomic writes, keeping encoding, BOM and line endings; `files.autoSave`, trim and final newline. Undo and redo. A file changed on disk while there are unsaved changes: conflict dot, ask before overwriting. Closing a tab or quitting with unsaved files. Explorer: create, rename (an open tab follows), delete to Trash or delete permanently.
2. **Opens correctly.** `luc .`, `luc <file>`, opening from Finder or the Dock, Open With, workspace restore, the Welcome page, Open Recent, Close Folder. Large files, binary files, files in other encodings.
3. **Following the agent.** Reload when a file on disk changes (atomic write, delete then recreate, `git checkout` of thousands of files), keeping scroll position and folds, markers on the directory tree, git colors in Explorer, the gutter.
4. **Daily navigation.** Quick Open (`>`, `:`), search in project, search in file, split, tabs and preview tabs, terminal, Vietnamese IME.

**Out of scope:** Vim, LSP, extensions, Markdown preview, diff view, performance, UI, docs, license, and cleanup before making the repo public.

## Chosen approach

**A. Read the code flow by flow.** Each group of flows goes to one agent that reads the code and changes nothing. The agent follows the code's path from the user's action (key, click, file system event) to the end result (disk write, view update) and looks for:
- unhandled edge cases,
- race conditions between the watcher, file writes and user actions,
- `unwrap`, `expect` and indexing that can panic on real data,
- swallowed errors (`let _ =`, `.ok()`) that should have been reported to the user,
- states that disagree with each other (tab, buffer, directory tree).

For each bug found, another agent that does not know the first agent's conclusion verifies it by reading the code. A bug that does not hold up is dropped.

### Approaches considered

| Approach | Trade-off | Result |
|---|---|---|
| A. Read the code by flow | Fast, broad, no build needed. Cannot catch bugs that only show when actually running (focus, IME, wrong drawing, timing) | **Chosen** |
| B. Run the real app against a checklist | Sees exactly what the user sees, but slow, each flow only gets a few cases tried, and a build costs disk | Not chosen |
| C. A + tests for group 1 + a partial real run | Most thorough, but the most expensive | Not chosen |

## Output

A report at `docs/superpowers/reviews/2026-11-15-release-flow-review.md`. For each bug:
- severity,
- the affected flow,
- `file:line`,
- a concrete way to reproduce it,
- a suggested fix direction.

Severity:
- **P0:** data loss or corruption, crash.
- **P1:** a basic flow cannot be done.
- **P2:** wrong but there is another way to do it.
- **P3:** minor.

The report states plainly what approach A cannot check, so you can try those by hand before release.

**No code is changed in this step.** Once the report exists, you pick the bugs that must be fixed before release. The fixes go through a separate spec or plan.

## Constraints

- Agents only read code: no build, no `cargo test`, no worktrees. So it costs no disk.
- Many sessions work in this repo at once, so everything (report, spec) is committed by specific path.
- Excerpts taken from code into the report must be short, just enough to point out the bug.

## Left to the implementer (my decisions, you can change them)

- Splitting the agents: one agent per group of flows (4 agents). Group 1 is the largest and most important, so split it in two: saving files and conflicts is one agent, Explorer and closing or quitting is the other. In total 5 review agents, plus the verification agents.
- Work out for yourself which files belong to which flow, starting from `lucerna-workbench`, `lucerna-text`, `lucerna-project`, `lucerna-editor`, `lucerna-git`, `lucerna-terminal`.
- If a bug belongs to an out-of-scope area (for example Vim) but breaks a basic flow, it still goes in the report.
