# Lucerna — instructions for Claude

Lucerna is a Rust/GPUI code viewer for macOS: a slimmer VS Code with Zed-style UI, built for people who write code through CLI agents. Talk to the user in Vietnamese.

## Before planning any feature
Read `docs/design-principles.md` first. It requires that before you brainstorm, spec or plan any feature, you **research how VS Code and Zed implement it** (docs and source) and write the findings to `docs/research/` or to the spec's "Tham khảo" section.

When the two differ:
- Behaviour, settings, commands and keys follow VS Code.
- UI and performance follow Zed.

Zed's crates are mostly GPL. Learn their design, but never copy their code.

## Project facts
- Specs live in `docs/superpowers/specs/` and plans in `docs/superpowers/plans/`. The roadmap is §1.3 of `2026-11-08-lucerna-phase1-design.md`.
- Crates are `lucerna-{text,syntax,project,settings,theme,workbench}` plus `lucerna`, which builds the `luc` binary. Only `lucerna-workbench` may use gpui.
- Cargo needs `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`. Disk is tight: never let free space drop below about 1.2 GB.
- The license is MIT OR Apache-2.0. `cargo deny check` must pass.
- Commit trailer: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Stage files by explicit path, because other sessions may work in this folder.
