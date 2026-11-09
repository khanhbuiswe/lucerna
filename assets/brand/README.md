# Lucerna brand files

The Lucerna mark is a light rising from a line of text. Spec: [`docs/superpowers/specs/2026-11-09-logo-design.md`](../../docs/superpowers/specs/2026-11-09-logo-design.md).

| File | Use it for |
|---|---|
| `mark.svg` | The mark at 24 px and up, on dark backgrounds: README, social images, docs |
| `mark-small.svg` | The mark below 24 px: favicons, tabs, the site nav. Two halo layers and a thicker line, so it stays legible |
| `mark-mono.svg` | Single-colour uses (menu bar, engraving, anywhere transparency is unavailable). Draws in `currentColor` |
| `lockup-dark.svg` | Mark and "Lucerna" wordmark on dark backgrounds |
| `lockup-light.svg` | Mark and wordmark on light backgrounds |
| `app-icon.svg` | 1024×1024 macOS app icon source |

## Colours

| Token | Hex | Where |
|---|---|---|
| Accent | `#4f8cff` | The halo, always |
| Ink | `#e6eaf2` | Line and wordmark on dark |
| Ground | `#080b12` | Line and wordmark on light |
| Tile | `#0f1420`, border `#1b2232` | App icon background |

## Rules

- Keep clear space around the mark equal to the core radius: 7 units on the 64-unit grid.
- The bottom of the line sits on the wordmark's baseline.
- Don't recolour the halo, add a glow or shadow, or rotate the mark.
- Don't put `lockup-light.svg` on a dark background or `lockup-dark.svg` on a light one.
- Below 24 px use `mark-small.svg`, not a scaled-down `mark.svg`.

## Regenerating the lockups

The wordmark is Bricolage Grotesque (OFL, `site/fonts/`) at weight 650, outlined to paths by `scripts/gen-lockup.py`. Setup and usage are in that script's docstring.
