# README and landing page refresh Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Short README and a rebuilt landing page built around four points: Rust, fast, minimal, no built-in AI.

**Architecture:** Plain Markdown in `README.md` and `docs/`; static `site/index.html` + `styles.css` + `main.js`, checked by `scripts/check-site.py`.

**Tech Stack:** Markdown, HTML, CSS, a few lines of vanilla JS, Python 3 unittest.

**Spec:** `docs/superpowers/specs/2026-11-15-readme-landing-design.md`

## Global Constraints

- Only four metrics, from `docs/benchmarks.md` (median, p90): startup Lucerna 140 (143) ms, Zed 241 (252), VS Code 267 (672); idle RAM 169 (179) MB, 235 (236), 879 (879); idle CPU 1.40 (1.42) %, 2.20 (2.23), 3.01 (3.18); app size 50, 177, 1604 MB. Ratios 1.7×/1.9×, 1.4×/5.2×, 1.6×/2.1×, 3.5×/31.9×.
- Never mention Quick Open or scrolling numbers.
- Zed only for GPUI, extensions and "why not Zed", neutral. Never "Zed-style".
- Site: no external hosts except `https://github.com/lucerna-dev/lucerna…`; no inline `<script>`; `theme-color` `#080b12`; brand accent `#4f8cff`.
- Public text in English.

## Review Focus

- Phone width (375 px): chart labels and the VS Code bar must not overflow; no horizontal page scroll.
- App-size chart: Lucerna's bar at 50/1604 is ~3 % wide; it must still be visible (min width) and labelled.
- Chart readable without hover: values are always printed, hover only adds p90.
- Keyboard: bars focusable for the tooltip; the table toggle is a real `<button>` / `<details>`.
- No content lost from the old README: every table is somewhere under `docs/`.

---

### Task 1: Move long README sections into docs/ and rewrite README

**Files:** Create `docs/features.md`, `docs/keyboard-shortcuts.md`, `docs/settings.md`, `docs/vim.md`, `docs/extensions.md`, `docs/building.md`; rewrite `README.md`.

- [ ] Copy sections verbatim from the old README into the new files (features list; shortcuts table; settings files + example + editing settings; Vim mode; Extensions; disk image + source install, Building from source, Workspace layout, Roadmap). Fix relative links (`docs/…` → sibling paths).
- [ ] Rewrite `README.md` per the spec order, ≤ 90 lines, with a four-row benchmark table and links to every new doc plus `docs/benchmarks.md`.
- [ ] Check: `grep -c '' README.md` ≤ 90; every relative link resolves (`python3` one-liner over `](` targets).
- [ ] Commit by explicit paths.

### Task 2: Checker expects the new metrics

**Files:** `scripts/check-site.py`, `scripts/test_check_site.py`

- [ ] Change `VALID_HTML` metric divs to `140<small>ms</small>`, `169<small>MB</small>`, `1.40<small>%</small>`, `50<small>MB</small>`; run `python3 -m unittest scripts/test_check_site.py` → FAIL.
- [ ] Set `METRICS = ["140ms", "169MB", "1.40%", "50MB"]` → tests PASS.
- [ ] Commit (with Task 3, since the site must pass the checker in the same commit range).

### Task 3: Rebuild the landing page

**Files:** `site/index.html`, `site/styles.css`, `site/main.js`, `site/og.png`

- [ ] Sections per spec: nav, hero (H1 "A fast, minimal code editor. Written in Rust. No AI inside."), four pillars, `#speed` charts, Rust, Minimal, `#why` no-AI, `#download`, FAQ (3), footer.
- [ ] Chart: per metric a `<figure class="chart">` with headline `<div class="v">` and three rows `.bar` (`--w` = value/max as %, min 2 %); Lucerna accent, others `--ink-3`; `tabindex="0"` + `data-tip` for median/p90 tooltip (CSS `:hover`/`:focus`); one `<details>` "Show as table" with the full table.
- [ ] Run `python3 scripts/check-site.py site` → no output, exit 0; `python3 -m unittest scripts/test_check_site.py scripts/test_brand.py` → OK.
- [ ] Preview in the browser pane at desktop and 375 px; fix overflow.
- [ ] Capture the hero at 1200×630 into `site/og.png`.
- [ ] Commit by explicit paths.
