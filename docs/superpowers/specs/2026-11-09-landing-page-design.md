# Lucerna — Landing page

- **Date:** 2026-11-09
- **Status:** Draft, awaiting approval
- **Approved mockup:** [`2026-11-09-landing-page-mockup.html`](2026-11-09-landing-page-mockup.html) (online version: https://claude.ai/artifact/Xh8wbLS9o9Q2c67mk8yuVx)

## 1. Goal

A page that introduces Lucerna, as professional as Wu's page (wu.dstrml.org), but with its own identity and saying only what Lucerna can actually do.

The audience is developers who write code mostly through an agent in the terminal (Claude Code, Codex…). Within the first 10 seconds, they must understand three things:

1. Lucerna is a native, fast code viewer written in Rust, made for reading the code an agent just changed.
2. Lucerna has no AI inside, and keeps the same shortcuts and `settings.json` as VS Code.
3. How to install: the release is coming soon, for now build from source.

**Success criteria:**

- The page can be deployed to GitHub Pages from the `site/` folder.
- It displays correctly at 1280 px and 375 px wide, with no horizontal scrolling.
- Every number and every claim on the page can be verified in the repo, see section 5.
- Lighthouse (mobile) scores ≥ 95 for Performance, Accessibility, Best Practices and SEO.

## 2. What was decided

| Question | Decision |
|---|---|
| Where it lives and how it is hosted | Plain HTML/CSS in the `site/` folder of this repo, no build step, deployed with GitHub Pages |
| Primary CTA | A pre-designed download area (a `curl` command, per-platform buttons), labeled "Soon" until there is a release; with the build-from-source command alongside |
| Language | English only |
| Editor image in the hero | Built with HTML/CSS in One Dark colors, no screenshot |
| Performance figures | Only use numbers measured in `docs/perf-checklist.md`; unmeasured numbers are written as targets; no comparison with VS Code or Zed |
| Look | Dark theme only (Lucerna defaults to One Dark), **Cobalt** palette: blue-black background `#080b12`, electric blue accent `#4f8cff`, blue-tinged white text. All color tokens live in the mockup's `:root`. Cobalt was chosen over amber because that palette did not look "tech" enough |

**Out of scope:** a docs page, a blog, a Vietnamese version, analytics, a newsletter sign-up form, a changelog page, a real `install.sh` script.

## 3. File structure

```
site/
  index.html          # all the content, semantic HTML
  styles.css          # color/type tokens in :root, then each section
  main.js             # only the Copy button (~20 lines), the page still works with JS off
  favicon.svg         # small version of the J2 logo, same as the logo on the nav bar
  og.png              # 1200×630 image for link sharing (captured from the hero)
  fonts/              # self-hosted woff2, with the OFL license files
    BricolageGrotesque-*.woff2
    InstrumentSans-*.woff2
    JetBrainsMono-*.woff2
    OFL-*.txt
.github/workflows/pages.yml
```

- **Self-host fonts, no calls to Google Fonts.** The page sends no request to any third party, matching the "no telemetry" line in the FAQ. Subset to Latin only, at most two weights per font. All three fonts use the OFL 1.1 license, which is also added to `THIRD_PARTY_NOTICES.md`.
- `<head>` contains: `title`, `meta description`, Open Graph and Twitter card, `theme-color`, `link rel=canonical`, a preload for the main display font.
- No framework or JS library, and no build step.

## 4. Sections

The order and content follow the mockup. The text in the mockup is the official text, changed only where noted in section 6.

| # | Section | Main content |
|---|---|---|
| 1 | Nav (sticky) | Logo + "Lucerna"; links Speed, Features, Roadmap, GitHub; a "Download · Soon" button leading to `#download` |
| 2 | Hero | Eyebrow "Open source · Written in Rust · macOS"; H1 "A quiet place to *read* the code your agents write."; a description paragraph; a `curl` command + Copy button + "First release coming soon" label + a build-from-source link; a mock editor frame and an agent terminal pane |
| 3 | Speed (`#speed`) | 4 real measurements: 887 ms, 2.04 ms, 96 µs, 51 ms; bars against the budget for the first two; a note on how they were measured |
| 4 | Features (`#features`) | 6 Phase 1 features on the right; a list of 6 shortcuts on the left (sticky on desktop) |
| 5 | Principles | No AI inside · Native · Open source |
| 6 | Roadmap (`#roadmap`) | 5 phases per the Phase 1 spec; Phase 1 is highlighted |
| 7 | Download (`#download`) | Build-from-source command + Copy; macOS Apple Silicon and Intel labeled "Soon"; Linux and Windows dimmed, labeled "after Phase 1" |
| 8 | FAQ | 5 questions using `<details>`, the first one open |
| 9 | Footer | "Latin for lamp", license, links to GitHub, Roadmap, Report a problem |

**Mock editor frame in the hero:**

- Built in HTML, with `role="img"` and an `aria-label` describing the content.
- The code inside comes from `crates/lucerna-text/src/buffer.rs`: lines 59–82, including the `reload()` function.
- Lines 65–77 are highlighted blue in a "just changed" style. When the page loads, these lines flash gently once, and do not flash if the user has `prefers-reduced-motion` on.
- The explorer marks `buffer.rs` and `reload.rs` as modified externally. The `reload.rs` tab is in italics (preview tab). The status bar shows "Reloaded from disk".
- The agent terminal pane is decorative only (`aria-hidden`) and is hidden when the screen is narrower than 960 px.
- If the code of `buffer.rs` changes, the code in the mock does **not** need to be updated to match. It is only an illustration.

**Responsive:**

- At 960 px and below: hide the terminal pane, Features becomes one column, the roadmap becomes two columns.
- At 640 px and below: hide the nav links, hide the mock's explorer and minimap, every grid becomes one column.
- Long commands scroll horizontally inside their own box, without making the whole page scroll horizontally.

## 5. Source of every claim

| Claim on the page | Source in the repo |
|---|---|
| 887 ms to scan 80,000 files; 2.04 ms Quick Open; 96 µs highlight; 51 ms reload | `docs/perf-checklist.md`, the Automated table, measured on 2026-11-09 |
| Budgets of 5 s and 16 ms; targets of 200 ms, 150 MB, 120 fps | Phase 1 spec, section 6 |
| CI blocks when a benchmark is more than 15% slower | `scripts/bench-gate.sh`, the `bench` job in `ci.yml` |
| No GPL dependencies, checked with `cargo deny` | `deny.toml`, `ci.yml` |
| Search uses the ripgrep engine | the `grep-searcher` and `grep-regex` crates in `Cargo.toml` |
| No telemetry; logs are at `~/Library/Logs/Lucerna` | no crate makes network calls (no reqwest/hyper/ureq); `crates/lucerna/src/logging.rs` |
| macOS 13 and later only; Linux and Windows come after Phase 1 | Phase 1 spec, line 92 |
| Features, shortcuts, 5-phase roadmap | Phase 1 spec, sections 1.3, 2.1 and 5.4 |

When one of these sources changes (for example the benchmarks are re-measured or the first release ships), the page must be updated in the same PR. This line will be added to `docs/perf-checklist.md`.

## 6. Differences from the mockup

1. The mockup uses Google Fonts; the real page self-hosts the fonts (section 3).
2. The mockup omits `<!doctype>` and `<head>` because the artifact platform adds them; the real page must have them in full.
3. **Domain name:** there is none yet. The `curl` command and the canonical tag use a single constant, temporarily set to `https://lucerna.dev`. If there is still no domain at deploy time, use the default GitHub Pages address (`https://lucerna-dev.github.io/lucerna/`). All internal links are relative paths, so the page works at both addresses.
4. **GitHub repo:** the links use `https://github.com/lucerna-dev/lucerna`, taken from `Cargo.toml`. The repo has no remote yet, so the links will be broken until the repo is pushed.
5. The "Download" button on the nav points to `#download` and does not download any file.

## 7. Deploy

The file `.github/workflows/pages.yml`:

- Runs on a push to `main` with changes in `site/**` or in this workflow file itself; can be run manually with `workflow_dispatch`.
- Uses `actions/configure-pages`, `actions/upload-pages-artifact` (path `site`) and `actions/deploy-pages`.
- Minimal permissions: `pages: write`, `id-token: write`, `contents: read`; `concurrency: pages`.
- Does not affect the existing `ci.yml`.

Enabling GitHub Pages (Source: GitHub Actions) in the repo settings is something the author does after the remote exists.

## 8. Verification

This is a static page, so no unit tests are written. Verify with these steps:

1. Open `site/index.html` directly from disk and through `python3 -m http.server`, at 1280 px and 375 px wide: no horizontal scrolling, fonts load correctly, no errors in the console.
2. With JS off: the page is still readable, and the command can still be selected for copying.
3. With `prefers-reduced-motion` on: no animation.
4. With the keyboard: Tab goes through all the links, buttons and `<details>` in the correct order, and focus is always visible.
5. Lighthouse mobile ≥ 95 in all four categories.
6. The W3C HTML validator reports no errors.
7. The Network tab shows only requests to the page's own origin.
8. Check each row of the table in section 5 against the content on the page.
