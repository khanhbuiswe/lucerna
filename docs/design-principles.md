# Lucerna design principles

The principles below apply to every feature and every phase. They sit above any individual spec.

## 1. Always research VS Code, Zed and Helix before planning

Lucerna aims to be a slimmer VS Code with a minimal UI. It targets people who let AI agents write the code, whether the agent runs as a CLI or a desktop app. Lucerna ships on macOS first, then Linux and Windows. So **before brainstorming, writing a spec or making a plan for any feature**, check how all three implemented that feature.

1. **VS Code.** Read the user docs, the API docs or contribution points, and the source in `microsoft/vscode`. If the feature lives in a popular extension (for example VSCodeVim), read that extension too. You need to know:
   - how users experience the feature;
   - the settings keys, command IDs and keybindings;
   - the edge cases VS Code has already run into.
2. **Zed.** Read the docs and the source in `zed-industries/zed`, preferably at the exact GPUI revision Lucerna pins. You need to know:
   - the architecture;
   - how it is made fast;
   - the UI style.
3. **Helix.** Read the docs and the source in `helix-editor/helix`. Helix is a slim Rust editor configured through data, so it is especially worth reading for LSP, tree-sitter, language configuration and modal editing. You need to know:
   - the algorithms and how it handles edge cases;
   - how it describes features in configuration data instead of code.
4. **Write down the findings** in `docs/research/YYYY-MM-DD-<topic>.md`, or in the "References" section of that feature's spec. The write-up covers:
   - how each of them does it;
   - where they agree and where they differ;
   - which one Lucerna follows, and why.
5. **Priority order when they differ:**
   - Behavior, settings, commands and keybindings: follow **VS Code**, so VS Code users who switch don't have to relearn anything.
   - UI and performance: follow **Zed**.
   - Helix does not override these two rules. It is a third view on internals (algorithms, data structures, config formats) when VS Code and Zed differ, or when one of them lacks the feature.
   - If Lucerna departs from both VS Code and Zed, the spec must state the reason.

### 1.1 UI follows Zed from the first design

A lesson from Search: Lucerna built the Search panel in the sidebar in the VS Code style, then had to redo it as a tab with a Zed-style search bar. To avoid repeating that:

- **Layout and components follow Zed from the very first spec.** Every feature starts with the question "what does this screen look like in Zed": is it a tab, a panel, a toolbar or a modal; which components does it use. Don't build a VS Code-style UI first and adjust it later. Only commands, keys, settings and behavior follow VS Code.
- **Share components across features**, as Zed does. Every search input (the in-file find bar, the Search tab, and later pickers or filters) uses the same query-box frame: a 1 px border, rounded corners, and the `Aa` `ab` `.*` toggles inside the box. They also share the same bar height, the same `‹ ›` buttons and counter, and the same background color. When adding a feature, look for an existing component in `ui.rs` first; if there is none, create it there so later features can reuse it.
- **A spec must include a screenshot or description of the matching Zed screen** (read the source at the pinned revision, or take a screenshot of Zed). The spec also states where Lucerna differs from Zed and why.
- **Review the UI against Zed** before treating a feature as done: run the app, put it next to Zed, and check spacing, heights, background colors, borders, hover and empty states.

## 2. Licenses when researching

| Source | License | What you may do |
|---|---|---|
| Zed: most crates (`editor`, `vim`, `extension_host`, `language`, `theme`…) | GPL-3 / AGPL | **Learn the design only, don't copy code.** Data formats (theme JSON, `extension.toml`, `.scm` queries, keymap) may be used. |
| Zed: `gpui*`, `sum_tree`, `zed_extension_api`, WIT files | Apache-2.0 | May be used as a dependency or reference. |
| Helix (`helix-editor/helix`) | MPL-2.0 | **Learn the design only, don't copy code** (copying it in would bind that file to MPL). Data such as `languages.toml` and `.scm` queries also carries MPL, so compare against it but don't copy it verbatim. |
| VS Code (`microsoft/vscode`) and MIT extensions | MIT | May be referenced or reused, but **the source must be credited** in `THIRD_PARTY_NOTICES.md` or in a comment. |
| Fonts, icons (Lilex, IBM Plex, Lucide…) | OFL / ISC / MIT | Take them from the original repo, together with the license file verbatim. |

Lucerna's source code is licensed MIT OR Apache-2.0. `cargo-deny` blocks dependencies with GPL, AGPL or LGPL licenses.

## 3. Other settled principles

- Lucerna targets people who write code through CLI agents (Claude Code, Codex). **Reading code** is the number one priority. There are no AI features.
- The UI thread never waits on I/O or parsing. Heavy work runs on a background thread over an immutable snapshot.
- Only the `lucerna-workbench` crate may depend on GPUI.
- Settings and keybindings follow VS Code semantics: objects merge across layers, per-language overrides, and the same precedence order as VS Code.
- Every feature has tests, each task is reviewed, and then the whole is reviewed before merging.
- **English only in the repository.** Everything written to the repository is English: code, comments, docs, specs, plans, research, commit messages and PR text, including every string the app shows (menus, commands, labels, messages, dialogs, lines printed to the terminal, tooltips, errors), which are named the way VS Code names things. Vietnamese appears in the repository only as test data in code (typing Vietnamese, IME and Unicode cases). Any text in a spec that describes the UI must give the exact English text that will appear in the app. `scripts/check-english.py scan` enforces this in CI; Vietnamese test data quoted in docs is listed in `scripts/english-allow.txt`.
