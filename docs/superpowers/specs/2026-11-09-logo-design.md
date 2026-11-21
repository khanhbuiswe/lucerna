# Lucerna — Logo

- **Date:** 2026-11-09
- **Status:** Draft, awaiting approval
- **Chosen direction:** J2 "Light over a line of text, minimal", picked from 15 directions and 8 sketched variants in the brainstorm session

## 1. Goal

Replace the temporary logo (a flame on a lamp bowl) with a logo that has its own identity and works from the 1024 px app icon down to the 16 px favicon.

Meaning: a glow rising from a line of text, like lamplight falling on a page of code. The shape keeps the spirit of the name "Lucerna" (Latin for lamp) and the line "A quiet place to read", but does not draw a lamp.

**Scope:** the logo only. The Cobalt palette, fonts and tone of voice stay as in the landing page spec.

**Success criteria:**

- At 16 px it still reads as a glow sitting on a line, not a smudge.
- On the site, the favicon, the nav logo and `og.png` all use the new logo. No place still uses the old lamp logo.
- `assets/brand/` has all the source files needed for the README, the app icon and later releases.
- `python3 scripts/check-site.py` still passes.

**Out of scope:** creating the `.icns` and packaging the `.app` (the repo has no bundle step yet), changing the palette, changing fonts, making a brand guideline for other channels.

## 2. Construction

Grid `viewBox="0 0 64 64"`. The glow's center is at (32, 42).

| Part | Shape | Color |
|---|---|---|
| Outer glow | Upper half circle, r = 22 | accent `#4f8cff`, opacity 0.18 |
| Middle glow | Upper half circle, r = 14 | accent, opacity 0.45 |
| Core | Upper half circle, r = 7 | accent, opacity 1 |
| Line | `M4 44H60`, stroke width 5, round caps | ink `#e6eaf2` |

Base version (use at 24 px and above):

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">
  <path d="M10 42a22 22 0 0 1 44 0Z" fill="#4f8cff" opacity=".18"/>
  <path d="M18 42a14 14 0 0 1 28 0Z" fill="#4f8cff" opacity=".45"/>
  <path d="M25 42a7 7 0 0 1 14 0Z" fill="#4f8cff"/>
  <path d="M4 44H60" stroke="#e6eaf2" stroke-width="5" stroke-linecap="round"/>
</svg>
```

All the half circles have their base at y = 42. The line, 5 thick, covers y = 41.5 to 46.5, so it fully hides the base edge of the glow and leaves no gap.

## 3. Variants

| Variant | Difference from the base | Where used |
|---|---|---|
| **Base** (`mark.svg`) | as in section 2 | 24 px and above: README, og, app icon |
| **Small** (`mark-small.svg`) | only 2 layers: outer layer r = 22 opacity 0.3, core r = 10 opacity 1; line width 6 | below 24 px: favicon, nav logo |
| **Monochrome** (`mark-mono.svg`) | one color, no transparency: solid core r = 8, arc r = 15 stroke 3.5 unfilled, the line; all use `currentColor` | menu bar icon, engraving, places that cannot show opacity |
| **Light background** | line and text change to `#080b12`; the glow stays the same | README on GitHub in light mode, print |

## 4. Lockup with text

- The word "Lucerna" uses Bricolage Grotesque, weight 650, letter-spacing −0.02em. This is the heading font already used on the site.
- The mark sits to the left of the text. The mark uses a viewBox cropped tight to `1 18 62 29` (leaving room for the round caps of the line, which run from x = 1.5 to 62.5), which drops the empty grid above and below.
- **Main rule:** the bottom edge of the line in the mark sits on the text baseline, so the glow looks like it rises from the line of text.
- The mark's height (per the cropped viewBox) is about 1.25 times the cap height. The gap between the mark and the text is 0.3 times the mark's height.
- In the source file, the text is converted to paths, so it does not depend on the fonts installed on the machine. The paths come from `site/fonts/BricolageGrotesque-latin-wght.woff2` at weight 650, using fonttools. The font is licensed under the OFL, which allows this.

## 5. macOS app icon

- Canvas 1024×1024, rounded tile rx ≈ 230 (the 22.5% ratio of macOS icons), background `#0f1420`, 1.5% border in `#1b2232`.
- Place the base mark in the middle, at the proportions in the approved mockup: the whole mark (the line counted with its round caps) is about 60% of the canvas wide, i.e. a scale of 10 grid units. The shape (x 1.5–62.5, y 20–46.5) is centered in the tile: `translate(192 179.5) scale(10)`.
- Only make the source file `app-icon.svg`. Exporting the `.icns` is left for release packaging.

## 6. Files changed

```
assets/brand/                 # new
  mark.svg                    # base version
  mark-small.svg              # below 24 px
  mark-mono.svg               # currentColor
  lockup-dark.svg             # mark with text on a dark background (text #e6eaf2)
  lockup-light.svg            # mark with text on a light background (text #080b12)
  app-icon.svg                # 1024×1024
  README.md                   # short table: which file is used where, colors, minimum clear space

site/favicon.svg              # replaced by mark-small.svg (hard-coded colors); the line is #080b12, switched to #e6eaf2
                              # via @media (prefers-color-scheme: dark) inside the SVG, so it does not vanish on a light tab
site/index.html               # the SVG logo in .brand switches to the small version, using var(--accent) / var(--ink)
site/styles.css               # adjust .brand svg for the new proportions and align to the text baseline
site/og.png                   # re-capture the hero after the logo change, still 1200×630
docs/superpowers/specs/2026-11-09-landing-page-design.md   # fix the "lamp flame logo" line in section 3
```

The minimum clear space around the mark equals the core radius (7 grid units).

## 7. Verification

- `python3 scripts/check-site.py` and `python3 scripts/test_check_site.py` still pass.
- Open the site at 1x and 2x and check by eye that the mark's line sits on the baseline of the "Lucerna" text in the nav.
- Look at the favicon in a browser tab at 16 px, on both light and dark browser tabs. The glow sitting on the line must be recognizable.
- Open each file in `assets/brand/` on a light and a dark background. For `mark-mono.svg` specifically, check it with an arbitrary `color` value.
- Search `site/` to confirm no path of the old logo remains (`M12 2.5c2.6 3`).
