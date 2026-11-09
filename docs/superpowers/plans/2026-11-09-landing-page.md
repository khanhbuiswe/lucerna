# Lucerna Landing Page Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the approved Lucerna landing page as a static site in `site/`, deployed to GitHub Pages.

**Architecture:**
- Plain HTML, CSS and a tiny script, with no build step and no third-party requests. Fonts are self-hosted.
- A stdlib-only Python checker, `scripts/check-site.py`, is the test suite. It checks head tags, anchors, local files and external links, and the benchmark numbers. CI runs it before every deploy.
- The approved mockup is the source of truth for markup, copy and CSS.

**Tech Stack:** HTML5, CSS (custom properties, `color-mix`), vanilla JS, Python 3 stdlib (`html.parser`), GitHub Actions (`actions/deploy-pages`).

**Spec:** `docs/superpowers/specs/2026-11-09-landing-page-design.md`
**Mockup (source of truth for markup, copy and CSS):** `docs/superpowers/specs/2026-11-09-landing-page-mockup.html`

## Global Constraints

- Language: English only. Copy comes verbatim from the mockup, except the changes listed in spec §6.
- Palette: Cobalt, dark only. Copy the `:root` tokens from the mockup unchanged: `--ground #080b12`, `--ground-2 #0e131d`, `--line #1b2232`, `--ink #e6eaf2`, `--ink-2 #98a2b5`, `--ink-3 #5f6a7e`, `--accent #4f8cff`, `--accent-dim rgba(79,140,255,.14)`, `--glow 79 140 255`, `--on-accent #ffffff`, `--eyebrow #7aa8ff`, plus the One Dark editor tokens.
- No request may leave the page's own origin. No Google Fonts, CDN, analytics or iframes. External `<a href>` links may only start with `https://github.com/lucerna-dev/lucerna`.
- Every internal link and asset path is relative (no leading `/`), so the page works both at `https://lucerna.dev/` and at `https://lucerna-dev.github.io/lucerna/`.
- Site origin constant: `https://lucerna.dev`. It is used only in `link rel=canonical`, `og:url`, `og:image` and the hero `curl` command.
- Fonts: Bricolage Grotesque, Instrument Sans and JetBrains Mono. One latin variable `woff2` per family, each OFL-1.1 with its license file next to it.
- No JS framework, no npm, no build step. The page must read correctly with JS disabled.
- Benchmark figures on the page are exactly `887 ms`, `2.04 ms`, `96 µs` and `51 ms`, measured 9 November 2026 (source: `docs/perf-checklist.md`).
- Breakpoints are `960px` and `640px`, as in the mockup.

## Review Focus

1. **Served from a sub-path.** On `https://lucerna-dev.github.io/lucerna/`, a leading-slash path such as `/styles.css` 404s. The checker fails on any asset or href that starts with `/` (Task 1, `test_root_relative_paths_fail`).
2. **A stray external load.** A leftover Google Fonts `<link>` or `@import` from the mockup silently breaks the "no third parties" promise. The checker scans HTML and CSS `url()`/`@import` for absolute URLs (Task 1, `test_external_stylesheet_fails`).
3. **A broken in-page anchor.** Nav items point at `#speed`, `#features`, `#roadmap` and `#download`; a renamed id leaves a dead link. The checker resolves every `href="#…"` (Task 1, `test_dead_anchor_fails`).
4. **A missing asset after a rename.** A font or the favicon is renamed but still referenced. The checker requires every local `src`, `href` and CSS `url()` to exist on disk (Task 1, `test_missing_local_file_fails`).
5. **Long commands on a phone.** The `git clone … && cargo build --release` line must scroll inside its own box at 375 px, not widen the page. A static checker cannot see this, so Task 5 runs it as a manual check.

---

## File map

| Path | Responsibility |
|---|---|
| `scripts/check-site.py` | Static checks over `site/`; exit 0 on pass, 1 with one line per failure |
| `scripts/test_check_site.py` | `unittest` tests for the checker, using temporary fixture sites |
| `site/index.html` | The page: full `<head>` plus the mockup body |
| `site/styles.css` | Mockup CSS minus the font import, plus `@font-face` |
| `site/main.js` | Copy buttons only |
| `site/favicon.svg` | Lamp mark in Cobalt |
| `site/og.png` | 1200×630 share image |
| `site/fonts/*.woff2`, `site/fonts/OFL-*.txt` | Self-hosted fonts and their licenses |
| `.github/workflows/pages.yml` | Run the checker, then deploy `site/` |
| `THIRD_PARTY_NOTICES.md` | New "Website fonts" section |
| `docs/perf-checklist.md` | Note that `site/index.html` quotes these numbers |

---

### Task 1: Site checker

**Files:**
- Create: `scripts/check-site.py`
- Test: `scripts/test_check_site.py`

**Interfaces:**
- Produces: `check_site(root: pathlib.Path) -> list[str]`, which returns failure messages (empty means pass). The CLI is `python3 scripts/check-site.py [site_dir]` (default `site`). It prints each failure on its own line and exits 1 if there are any. The tests import the module with `importlib` because its file name contains a hyphen.

**Checks, in order (each failure is one message naming the file and the value):**
1. `index.html` exists and begins with `<!doctype html>` (case-insensitive).
2. `<html lang="en">`.
3. Head has: a non-empty `<title>`; `meta[name=description]`; `meta[property=og:title]`, `og:description`, `og:image`, `og:url`; `meta[name=twitter:card]` = `summary_large_image`; `meta[name=theme-color]` = `#080b12`; `link[rel=canonical]`; `link[rel=icon]` pointing to `favicon.svg`; `meta[name=viewport]`; and `og.png` exists at the root, because `og:image` points at it by absolute URL.
4. Every element id that a `href="#x"` targets exists (`#top` included).
5. Every local `src` and `href` (on `link`, `script` and `img`) and every CSS `url(...)` in `site/**/*.css` is relative, does not start with `/`, and resolves to an existing file under the root.
6. No `http(s)://` or protocol-relative URL in `src`, in `link[href]`, or in CSS `url()`/`@import`. The only exceptions are the `og:image`, `og:url` and canonical values, which must start with `https://lucerna.dev/`.
7. Every `<a href>` that starts with `http` starts with `https://github.com/lucerna-dev/lucerna`.
8. The text of the elements with `class="v"` (the metric values), in document order, is exactly `887ms`, `2.04ms`, `96µs`, `51ms`. Whitespace is stripped and the `<small>` unit is concatenated.

- [ ] **Step 1: Write the failing tests**

`scripts/test_check_site.py` builds each fixture in a `tempfile.TemporaryDirectory` from a minimal valid page that is a module-level constant (`VALID_HTML`, with all head tags, the four metrics, `#top` and one anchor). Tests:

```python
def test_valid_site_passes(self): self.assertEqual(check_site(self.write(VALID_HTML)), [])
def test_missing_index_fails(self): ...  # empty dir -> message contains "index.html"
def test_missing_theme_color_fails(self): ...  # drop the meta -> message contains "theme-color"
def test_dead_anchor_fails(self): ...  # add <a href="#nope"> -> message contains "#nope"
def test_root_relative_paths_fail(self): ...  # <link rel="stylesheet" href="/styles.css"> -> contains "/styles.css"
def test_missing_local_file_fails(self): ...  # <script src="main.js"> with no file -> contains "main.js"
def test_external_stylesheet_fails(self): ...  # CSS file with @import url(https://fonts.googleapis.com/x) -> contains "fonts.googleapis.com"
def test_foreign_link_fails(self): ...  # <a href="https://example.com"> -> contains "example.com"
def test_github_link_passes(self): ...  # <a href="https://github.com/lucerna-dev/lucerna/issues"> -> []
def test_missing_metric_fails(self): ...  # change the second .v to "2.40" -> contains "2.04"
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `python3 -m unittest scripts/test_check_site.py -v`
Expected: errors because `scripts/check-site.py` does not exist.

- [ ] **Step 3: Implement `check_site` in `scripts/check-site.py`**

Use `html.parser.HTMLParser` to collect head tags, ids, `href`s and `src`s, and the visible text (skip `script` and `style`). Scan CSS with the regex `url\(\s*['"]?([^'")]+)` plus `@import`. Ignore `data:` URLs. Use only the stdlib.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `python3 -m unittest scripts/test_check_site.py -v`
Expected: 10 tests OK.

- [ ] **Step 5: Commit**

```bash
git add scripts/check-site.py scripts/test_check_site.py
git commit -m "feat(site): static checker for the landing page"
```

---

### Task 2: Port the page from the mockup

**Files:**
- Create: `site/index.html`, `site/styles.css`, `site/main.js`

**Interfaces:**
- Consumes: `python3 scripts/check-site.py site` (Task 1).
- Produces: `styles.css` with a placeholder block `/* @font-face: Task 3 */` at the top. `index.html` references `styles.css`, `main.js` and `favicon.svg` by relative path, and `og:image` points at `https://lucerna.dev/og.png`. Task 3 and Task 4 only add files and never change these references.

- [ ] **Step 1: Run the checker to confirm it fails**

Run: `python3 scripts/check-site.py site`
Expected: exit 1, reporting that `index.html` is missing.

- [ ] **Step 2: Write `site/index.html`**

- Start with `<!doctype html>`, `<html lang="en">`, `<meta charset="utf-8">` and `<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">`.
- Title: `Lucerna · A fast, native code viewer`.
- Description, which is also `og:description`, is the hero lede verbatim: "Lucerna is a fast, native code viewer for people who work with Claude Code, Codex and other terminal agents. Open a repo, follow every change as it lands on disk, and keep the VS Code keys you already know."
- `og:title` = the title. `og:url` and canonical = `https://lucerna.dev/`. `og:image` = `https://lucerna.dev/og.png`. `twitter:card` = `summary_large_image`. `theme-color` = `#080b12`. `link rel=icon type=image/svg+xml href=favicon.svg`.
- Load `styles.css` with `<link rel="stylesheet">`, and `main.js` with `<script src="main.js" defer>` at the end of `<head>`.
- The body is the mockup's `<nav>` … `</footer>`, copied unchanged.
- Drop the mockup's `claude:viewport-fit-sides` meta, its Google Fonts links and its inline `<script>`.

- [ ] **Step 3: Write `site/styles.css`**

The mockup's `<style>` contents, unchanged, except:
- Prepend the `/* @font-face: Task 3 */` placeholder.
- In the `body::before`/`nav` sticky rules, keep the `env(safe-area-inset-*)` usage as it is.

- [ ] **Step 4: Write `site/main.js`**

Copy the mockup's copy-button script as is (the `.copy` click handlers and `selectText`): `navigator.clipboard.writeText`, with a `Range` selection fallback, and the label going "Copied" and back to "Copy" after 1400 ms.

- [ ] **Step 5: Run the checker**

Run: `python3 scripts/check-site.py site`
Expected: the only failures are the missing `favicon.svg` and `og.png`.

- [ ] **Step 6: Commit**

```bash
git add site/index.html site/styles.css site/main.js
git commit -m "feat(site): landing page markup and styles from the approved mockup"
```

---

### Task 3: Self-hosted fonts

**Files:**
- Create: `site/fonts/BricolageGrotesque-latin-wght.woff2`, `site/fonts/InstrumentSans-latin-wght.woff2`, `site/fonts/JetBrainsMono-latin-wght.woff2`, `site/fonts/OFL-BricolageGrotesque.txt`, `site/fonts/OFL-InstrumentSans.txt`, `site/fonts/OFL-JetBrainsMono.txt`
- Modify: `site/styles.css` (replace the placeholder), `site/index.html` (preload), `THIRD_PARTY_NOTICES.md`

**Interfaces:**
- Consumes: the `--display`, `--body` and `--mono` font stacks from Task 2. Their family names stay `"Bricolage Grotesque"`, `"Instrument Sans"` and `"JetBrains Mono"`.

- [ ] **Step 1: Download the fonts and licenses**

```bash
curl -fsSLo site/fonts/BricolageGrotesque-latin-wght.woff2 https://cdn.jsdelivr.net/fontsource/fonts/bricolage-grotesque:vf@latest/latin-wght-normal.woff2
curl -fsSLo site/fonts/InstrumentSans-latin-wght.woff2 https://cdn.jsdelivr.net/fontsource/fonts/instrument-sans:vf@latest/latin-wght-normal.woff2
curl -fsSLo site/fonts/JetBrainsMono-latin-wght.woff2 https://cdn.jsdelivr.net/fontsource/fonts/jetbrains-mono:vf@latest/latin-wght-normal.woff2
curl -fsSLo site/fonts/OFL-BricolageGrotesque.txt https://raw.githubusercontent.com/google/fonts/main/ofl/bricolagegrotesque/OFL.txt
curl -fsSLo site/fonts/OFL-InstrumentSans.txt https://raw.githubusercontent.com/google/fonts/main/ofl/instrumentsans/OFL.txt
curl -fsSLo site/fonts/OFL-JetBrainsMono.txt https://raw.githubusercontent.com/google/fonts/main/ofl/jetbrainsmono/OFL.txt
file site/fonts/*.woff2
```

Expected: `file` reports `Web Open Font Format (Version 2)` for all three. Each `woff2` is under 120 KB (`ls -l`).

- [ ] **Step 2: Replace the placeholder with three `@font-face` rules**

Each rule has `src: url("fonts/<file>.woff2") format("woff2")`, `font-display: swap` and a `font-weight` range: Bricolage `200 800`, Instrument Sans `400 700`, JetBrains Mono `100 800`. Add `font-optical-sizing: auto` to the Bricolage rule so `opsz` follows the size.

- [ ] **Step 3: Preload the display face**

In `index.html`, before the stylesheet, add `<link rel="preload" href="fonts/BricolageGrotesque-latin-wght.woff2" as="font" type="font/woff2" crossorigin>`.

- [ ] **Step 4: Add a "Website fonts" section to `THIRD_PARTY_NOTICES.md`**

Add it before "Additional licenses". It holds a table in the existing format: font, license `OFL-1.1`, upstream (github.com/ateliertriay/bricolage, github.com/Instrument/instrument-sans, github.com/JetBrains/JetBrainsMono), and the line "License texts ship next to the files in `site/fonts/`."

- [ ] **Step 5: Run the checker**

Run: `python3 scripts/check-site.py site`
Expected: the only failures are `favicon.svg` and `og.png`, with no font paths reported.

- [ ] **Step 6: Commit**

```bash
git add site/fonts site/styles.css site/index.html THIRD_PARTY_NOTICES.md
git commit -m "feat(site): self-host the three OFL fonts"
```

---

### Task 4: Favicon and share image

**Files:**
- Create: `site/favicon.svg`, `site/og.png`

- [ ] **Step 1: Write `site/favicon.svg`**

Use the nav logo's two paths at `viewBox="0 0 24 24"`, with literal colors because a favicon cannot read CSS variables: flame `#4f8cff`, lamp base stroke `#e6eaf2`, width 1.6.

- [ ] **Step 2: Render `site/og.png` (1200×630) from the hero**

```bash
python3 -m http.server 8765 --directory site &
"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new --hide-scrollbars --window-size=1200,630 --screenshot="$PWD/site/og.png" http://localhost:8765/
kill %1
file site/og.png
```

Expected: `PNG image data, 1200 x 630`. On a Retina Mac, add `--force-device-scale-factor=1` if the size comes out doubled. Open the image and check that the H1 and the top of the editor mock are visible. Do not change the page to suit the screenshot.

- [ ] **Step 3: Run the checker and the checker tests**

Run: `python3 scripts/check-site.py site && python3 -m unittest scripts/test_check_site.py`
Expected: no output from the checker, exit 0, and 10 tests OK.

- [ ] **Step 4: Commit**

```bash
git add site/favicon.svg site/og.png
git commit -m "feat(site): favicon and share image"
```

---

### Task 5: Deploy workflow, docs note, manual verification

**Files:**
- Create: `.github/workflows/pages.yml`
- Modify: `docs/perf-checklist.md` (one paragraph under the Automated table)

**Interfaces:**
- Consumes: `scripts/check-site.py` and `scripts/test_check_site.py` (Task 1), and the complete `site/` (Tasks 2–4).

- [ ] **Step 1: Write `.github/workflows/pages.yml`**

- **Trigger:** `on.push.branches: [main]` with `paths: ["site/**", "scripts/check-site.py", ".github/workflows/pages.yml"]`, plus `workflow_dispatch`.
- **Permissions and concurrency:** `permissions: { contents: read, pages: write, id-token: write }`, `concurrency: { group: pages, cancel-in-progress: false }`.
- **Job `deploy`:**
  - `runs-on: ubuntu-latest`, `environment: { name: github-pages, url: ${{ steps.deployment.outputs.page_url }} }`.
  - Steps:
    1. `actions/checkout@v4`
    2. `python3 -m unittest scripts/test_check_site.py`
    3. `python3 scripts/check-site.py site`
    4. `actions/configure-pages@v5`
    5. `actions/upload-pages-artifact@v3` with `path: site`
    6. `actions/deploy-pages@v4` with `id: deployment`

- [ ] **Step 2: Validate the workflow syntax**

Run: `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/pages.yml'))" 2>/dev/null || ruby -ryaml -e 'YAML.load_file(".github/workflows/pages.yml")'`
Expected: exit 0. Also confirm that `ci.yml` is unchanged (`git diff --stat .github/workflows/ci.yml` is empty).

- [ ] **Step 3: Add the docs note to `docs/perf-checklist.md`**

Add this right after the Automated table, verbatim: "`site/index.html` quotes the four numbers above and the date they were measured. When you re-measure, update the Speed section of the landing page in the same PR."

- [ ] **Step 4: Run the manual checks (spec §8) against `python3 -m http.server 8765 --directory site`**

Record the result of each check in the PR description:
1. At 1280 px and 375 px: no horizontal scroll (`document.documentElement.scrollWidth <= innerWidth`); fonts render as Bricolage, Instrument Sans and JetBrains Mono (DevTools → Computed → Rendered fonts); the console is empty.
2. With JS disabled, the page reads fully and the commands are still selectable.
3. With `prefers-reduced-motion: reduce` emulated, the added lines do not flash.
4. Tab order runs nav, hero copy buttons, links, `<details>`, footer, and every focus ring is visible.
5. Lighthouse mobile scores ≥ 95 for Performance, Accessibility, Best Practices and SEO.
6. https://validator.w3.org/nu/ on `site/index.html` (file upload) reports zero errors.
7. The Network tab shows only `localhost:8765` requests.
8. Every row of spec §5 matches the page text.
9. At 375 px, the `git clone … && cargo build --release` command scrolls inside its box (Review Focus 5).

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/pages.yml docs/perf-checklist.md
git commit -m "ci(site): deploy the landing page to GitHub Pages"
```

After merge, the author turns on Pages (Settings → Pages → Source: GitHub Actions) once the repository has a remote. This plan does not do that.
