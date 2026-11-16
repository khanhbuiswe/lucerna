# Markdown Preview follows a rename

Small change (bug fix).

## Problem

Rename `README.md` to `READDME.md` in the Explorer while a Markdown Preview
of it is open, then rename it back: the Explorer refuses with "A file or
folder README.md already exists at this location."

Cause: `Workbench::paths_renamed` moves documents, tab items, CodeViews,
recent files and history, but not the `MarkdownPreview` view entity, whose
`path` stays `README.md`. `Documents::rename` emits `Closed` for the old
path; the preview's `reopen_if_dropped` sees no document there and opens
`README.md` again (a not-found document). The Explorer's name check counts
any open document (`has_document_under`), so the gone file's name is held.
The preview also stops showing the renamed file.

Not a bug: `git status` showing "deleted" + "untracked" for an unstaged
rename. Git only detects renames once both sides are staged.

## Goal

- After a rename (file or a folder above it), every Markdown Preview of a
  moved path shows the file at its new path.
- Renaming back to the old name works.

Non-goals: changing the Explorer's name check; git rename detection.

## Approaches

1. **Chosen:** in `paths_renamed`, call `MarkdownPreview::set_path(moved)`
   on each preview body whose path is at or under `from`, before the
   `Closed` event is delivered, so `reopen_if_dropped` finds the document
   at the new path.
2. Rejected: make the Explorer ignore documents in the deleted/not-found
   state. Hides the symptom; the preview still points at a gone file.

## Tham khảo

VS Code: a Markdown preview follows its resource when the file is renamed
(the preview is retargeted with the editor). Behaviour follows VS Code.

## Test

Open a preview of `a.md`, rename `a.md` → `b.md`: preview path is `b.md`,
no document is left at `a.md`. Rename back to `a.md`: allowed, preview
path is `a.md`. Same for a dynamic preview, and for renaming a folder that
contains the file.
