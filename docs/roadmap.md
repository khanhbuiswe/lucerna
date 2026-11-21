# Roadmap after 0.1.1

Goal: a user switching from VS Code (a dev who lets agents write the code) can replace VS Code for
everyday work. Each item must remove one reason for them to go back to VS Code.

The older phase-based roadmap (phases 1–5, V1–V2) is in §1.3 of
`superpowers/specs/2026-11-08-lucerna-phase1-design.md`.

## Tier 1: blocks new users

Without these, new users can't use or install Lucerna on day one.

**In progress**, in order (one spec per item):

- [x] Zoom (⌘+ / ⌘− / ⌘0)
- [x] New file (⌘N) and Save As
- [x] Find & Replace in file (⌥⌘F)
- [x] Replace in project (Search)
- [x] Image preview (png, jpg, gif, webp, svg…, reloads automatically when the file changes)
- [ ] Format document (⇧⌥F) and format on save (half of step 4.6)

**Later:**

- [x] Multi-cursor (step 4.3)
- [x] Reopen the tab you just closed (⇧⌘T), Close Others
- [ ] Build for Intel Mac
- [ ] Linux (separate project)

## Tier 2: missing VS Code habits

Usable, but annoying every day.

- [ ] Completion and snippets (step 4.5)
- [ ] Rename (step 4.6)
- [ ] Problems tab
- [ ] Settings UI
- [ ] Windows

## Tier 3: better than VS Code when working with agents

The reason for users to stay. Not brainstormed yet.
