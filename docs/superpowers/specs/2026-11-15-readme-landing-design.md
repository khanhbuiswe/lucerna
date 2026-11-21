# README and landing page refresh

Date: 2026-11-15 · Status: approved design, small change

## Intent

The README grew into a full manual (427 lines) and the landing page in `site/`
still describes Phase 1: a read-only "code viewer", old criterion numbers and an
install script that does not exist. Both should tell a newcomer four things and
nothing more:

1. **Written in Rust**, drawn on the GPU with GPUI.
2. **Fast**: the published numbers in `docs/benchmarks.md`.
3. **Minimal**: a small UI that keeps VS Code's shortcuts and `settings.json`.
4. **No built-in AI**: an agent (Claude Code, Codex) writes the code; Lucerna is
   for reading and reviewing it.

## Goals

- README of about 70–90 lines: what Lucerna is, the four points, install, basic
  usage, links to the detailed docs.
- Nothing is lost: the long tables move to `docs/`.
- Landing page rebuilt around the four points; hero headline is option B:
  "A fast, minimal code editor. Written in Rust. No AI inside."
- `scripts/check-site.py` keeps passing, updated for the new numbers.

## Non-goals and anti-goals

- No full feature list on the site or in the README (a short "what it does"
  line is fine, a 20-bullet list is not).
- Only four metrics: startup 140 ms, idle RAM 169 MB, idle CPU 1.40 %, app size
  50 MB, each with Zed and VS Code. Quick Open and scrolling are unpublished and
  must not appear.
- Zed is named only for GPUI, the extension ecosystem and "why not Zed", in
  neutral, factual words. The UI is never called "Zed-style".
- No new build tooling, frameworks, external hosts or JS beyond the copy button.

## Approaches

- **Edit the existing static site in place (chosen).** Keep `site/` as plain
  HTML/CSS with the current fonts, brand colours and checker; rewrite
  `index.html` and adjust `styles.css`. Cheapest, and CI/Pages need no change.
- Rebuild with a static site generator: more structure than one page needs, and
  it would change the Pages workflow.
- Patch only the text of the current page: keeps the old roadmap, FAQ and
  feature grid that no longer fit the four-point story.

## Design

### README (`README.md`)

Logo and tagline → one paragraph on what Lucerna is and the agent → Lucerna →
review flow → **Why Lucerna** (the four points, two or three lines each; the
fast point carries the four numbers in a small table with Zed and VS Code) →
**Install** (GitHub release, build from source) → **Usage** (`luc .`, five key
shortcuts, where `settings.json` lives) → **Documentation** (links) → status
line → license.

Moved out, unchanged in substance:

| New file | Content from the old README |
|---|---|
| `docs/features.md` | The feature list |
| `docs/keyboard-shortcuts.md` | Shortcut table |
| `docs/settings.md` | Settings files, example, editing settings table |
| `docs/vim.md` | Vim mode: keys, clipboard rule, settings |
| `docs/extensions.md` | Extensions: in-app, CLI, runtime needs |
| `docs/building.md` | Building from source, checks, crate layout, roadmap |

The disk-image install steps move to `docs/building.md` as well.

### Landing page (`site/index.html`, `site/styles.css`)

Sections, top to bottom:

1. **Nav**: logo, links Speed · Why · GitHub, Download button marked Soon.
2. **Hero**: eyebrow "Open source · Rust · macOS, Linux and Windows later";
   H1 option B; lede "Your agent writes the code in Claude Code or Codex.
   Lucerna is where you read and review it."; buttons GitHub and Build from
   source; editor mock updated to show the git gutter and agent-changed files
   next to a terminal running an agent.
3. **Four pillars**: one band, four short cells (Rust, Fast, Minimal, No AI).
4. **Fast** (`#speed`): a benchmark chart (requested by the user). Four small
   charts, one per metric (startup, idle RAM, idle CPU, app size), each a
   horizontal bar chart with three bars, Lucerna, Zed and VS Code, on a linear
   scale from zero. Lucerna's value is the headline number of the card
   (140 ms, 169 MB, 1.40 %, 50 MB).
   - Form: one measure per chart, so no shared axis between metrics (different
     units); every bar is labelled with editor name and median, and the others
     carry their "× Lucerna" ratio.
   - Colour: Lucerna in the brand accent `#4f8cff`, Zed and VS Code in one
     neutral grey; names and values are text, never colour alone.
   - Drawn in plain HTML/CSS (bar width = value / max), no chart library.
     Hovering or focusing a bar shows median and p90; a "Show as table" toggle
     reveals the full table (median, p90, ratio).
   - Footnote: Apple M4, macOS 26, 10 runs, median, 15 November 2026, no LSP,
     warm; link to `docs/benchmarks.md` on GitHub for method and limits.
   - Quick Open and scrolling stay out.
5. **Rust and GPUI**: short text, no Electron, no web view.
6. **Minimal**: VS Code shortcuts block plus a `settings.json` snippet.
7. **No AI inside** (`#why`): the agent → Lucerna → you flow, plus "Why not
   Zed?" in neutral words and "no telemetry, no account".
8. **Download** (`#download`): no release yet; build from source command;
   platform chips.
9. **FAQ**: three questions (can I edit code, relation to Zed, data
   collection).
10. **Footer**.

Roadmap is removed from the page. `site/og.png` is re-captured from the new
hero at 1200×630.

### Checker

`scripts/check-site.py` `METRICS` becomes `140ms, 169MB, 1.40%, 50MB`;
`scripts/test_check_site.py` follows.

## Left to the implementer

Exact wording of body copy, CSS details of the bars, mock contents.
