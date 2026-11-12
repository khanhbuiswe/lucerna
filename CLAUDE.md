# Lucerna — instructions for Claude

Lucerna is a Rust/GPUI code editor with a minimal UI: a slimmer VS Code for people who let AI agents write the code, whether the agent runs in a CLI (Claude Code, Codex) or a desktop app (Claude, Codex). It ships on macOS first, and Linux and Windows are planned, so don't add macOS-only assumptions outside platform-specific code. Talk to the user in Vietnamese.

In public-facing text (README, site), mention Zed only for GPUI, the extension ecosystem, and the "why not Zed" positioning (factual and neutral: Zed is AI-first with a built-in agent; Lucerna has no AI and stays minimal; never disparage). Don't describe the UI as "Zed-style"; call it a minimal UI.

## Before planning any feature
Read `docs/design-principles.md` first. It requires that before you brainstorm, spec or plan any feature, you **research how VS Code and Zed implement it** (docs and source) and write the findings to `docs/research/` or to the spec's "Tham khảo" section.

When the two differ:
- Behaviour, settings, commands and keys follow VS Code.
- UI and performance follow Zed.

Design the UI after Zed from the first spec, not as a VS Code-style layout fixed later. Reuse shared components from `ui.rs`, such as the search input box and the bar layout, so features look consistent. See `docs/design-principles.md` §1.1.

Zed's crates are mostly GPL. Learn their design, but never copy their code.

## Project facts
- Specs live in `docs/superpowers/specs/` and plans in `docs/superpowers/plans/`. The roadmap is §1.3 of `2026-11-08-lucerna-phase1-design.md`.
- Crates are `lucerna-{text,syntax,project,settings,theme,vim,workbench}` plus `lucerna`, which builds the `luc` binary. Only `lucerna-workbench` may use gpui.
- Cargo needs `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`. Disk is tight: never let free space drop below about 1.2 GB.
- Keep only one build profile on disk. Before a release build, delete `target/debug`; before a debug build or `cargo test`, delete `target/release`. `scripts/bundle-app.sh` does this itself. Never delete a profile while another session's cargo is running (`pgrep -x cargo`).
- Run `scripts/prune-target.sh` before every `cargo test` or build. It keeps each profile under 5 GB: first it drops old test binaries, then, if still over, the whole profile folder.
- The license is MIT OR Apache-2.0. `cargo deny check` must pass.
- Commit trailer: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Stage files by explicit path, because other sessions may work in this folder.
