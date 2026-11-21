# Switching branches: how VS Code and Zed do it

Research for the feature of clicking the branch name in the title bar to switch branches. Spec: `docs/superpowers/specs/2026-11-15-git-branch-switch-design.md`.

Sources:
- VS Code: `microsoft/vscode` branch `main`, MIT license. Read `extensions/git/src/{commands,statusbar,repository,git}.ts`, `package.json` and `package.nls.json`.
- Zed: revision `b5cd310`, which Lucerna pins, GPL license. Only the design is studied, no code is copied. Read `crates/git_ui/src/branch_picker.rs` and `crates/git/src/repository.rs`.

## 1. How it opens

| | VS Code | Zed |
|---|---|---|
| From where | clicking the branch entry on the status bar, or the command `git.checkout` "Git: Checkout to..." | clicking the branch name button on the title bar; the command `git::Branch` |
| Form | a Quick Pick at the top center | a popover attached right under the branch name button (`GitBranchSelector`) |
| Default key | none | none for the open command |

## 2. The list

- **VS Code:**
  - The placeholder is "Select a branch or tag to checkout".
  - While the filter box is empty, the top three entries are: "+ Create new branch...", "+ Create new branch from...", "Checkout detached...".
  - After that come the groups "branches", "remote branches", "tags", in the order of `git.checkoutType`.
  - Sorted by commit date descending (`git.branchSortOrder`).
  - Each row has: a label; a description `N↓ M↑` with a relative date; a detail line `author • hash • subject` (`git.showReferenceDetails`).
  - Fuzzy filtering on the label and description. `origin/HEAD` is not shown.
- **Zed:**
  - The placeholder is "Switch or type to create a branch…".
  - Lists local and remote branches. Has an All / Local / Remote filter.
  - The current branch is prioritized at the top. Each row shows the branch name, the commit subject and the time.
  - Typing a name that does not exist yet shows a create-branch row.
  - Holding a modifier key deletes a branch.

## 3. Switching branches with uncommitted changes

- **VS Code:**
  - Runs `git checkout -q <ref>`. Changes that do not conflict are carried over to the new branch by git itself.
  - If stderr contains "commit your changes or stash them", a dialog "Your local changes would be overwritten by checkout." appears with three buttons:
    - **Stash & Checkout**: stash, including untracked files;
    - **Migrate Changes**: stash, switch branch, then pop;
    - **Force Checkout**: discard everything, then switch.
  - `git.autoStash` does not apply to checkout.
- **Zed:** `change_branch` runs `git checkout <name>`. The error is shown verbatim in a "Failed to change branch" dialog. No stash.

## 4. Remote branches

Both do the same thing:
- A local branch with the same name exists: switch to it. Zed also resets the upstream.
- Does not exist: create a local branch tracking the remote branch. VS Code uses `checkout --track`, Zed uses `branch --track` then `checkout`.

## 5. Creating a branch

- **VS Code:**
  - The input box has the prompt "Please provide a new branch name". The initial value comes from `git.branchPrefix`.
  - Whitespace and invalid characters are replaced with `git.branchWhitespaceChar` (default `-`).
  - Reports "Branch "x" already exists" on a duplicate name. Validated per `git.branchValidationRegex`.
  - After creating, it switches right away (`checkout -b`).
- **Zed:** typing a name then Enter runs `git switch -c <name>` from HEAD. The secondary key creates from the default branch.

## 6. Restricted Mode and hooks

- VS Code disables the whole git extension when the workspace is untrusted.
- Zed runs git, but disables hooks in an untrusted directory. In a trusted directory hooks run normally.

## 7. What Lucerna chooses

Approved by the user on 2026-11-15:
- **Opening and display:** a popover under the branch name on the title bar, following Zed. Commands and IDs follow VS Code (`git.checkout`).
- **Uncommitted changes:** let git decide, like Zed. Lucerna only blocks beforehand when there are still unsaved files.
- **Creating a branch:** yes, from HEAD.
- **Remote branches:** create a local branch tracking it.
- **Scope of this version:** no tags, detached HEAD, stash or branch deletion.
