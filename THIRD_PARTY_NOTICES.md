# Third-party notices

Lucerna is licensed under MIT OR Apache-2.0. It depends on third-party crates,
whose licenses are restricted by `deny.toml` to: MIT, Apache-2.0,
BSD-2-Clause, BSD-3-Clause, ISC, Zlib, Unicode-3.0, MPL-2.0.

## GPUI

The UI framework `gpui` (and `gpui_platform` and its platform crates) comes
from the Zed repository (https://github.com/zed-industries/zed), pinned to a
single commit in the workspace `Cargo.toml`. Zed's `zed-font-kit` fork (https://github.com/zed-industries/font-kit) is also
pulled in transitively for macOS glyph rasterization. Only the Apache-2.0 licensed
GPUI crates are used; no GPL-licensed crate from that repository is a
dependency.

## Tree-sitter grammars

`lucerna-syntax` bundles these grammars. Each crate compiles its grammar's C
sources into the binary, and every highlights query is the one the crate
itself ships (`HIGHLIGHTS_QUERY` or equivalent); no query file is vendored.
All are MIT licensed.

| Crate | Version | License | Upstream |
|---|---|---|---|
| `tree-sitter` (core library) | 0.27.2 | MIT | https://github.com/tree-sitter/tree-sitter |
| `tree-sitter-language` | 0.1.9 | MIT | https://github.com/tree-sitter/tree-sitter |
| `tree-sitter-rust` | 0.24.2 | MIT | https://github.com/tree-sitter/tree-sitter-rust |
| `tree-sitter-typescript` (TypeScript, TSX) | 0.23.2 | MIT | https://github.com/tree-sitter/tree-sitter-typescript |
| `tree-sitter-javascript` | 0.25.0 | MIT | https://github.com/tree-sitter/tree-sitter-javascript |
| `tree-sitter-python` | 0.25.0 | MIT | https://github.com/tree-sitter/tree-sitter-python |
| `tree-sitter-go` | 0.25.0 | MIT | https://github.com/tree-sitter/tree-sitter-go |
| `tree-sitter-json` | 0.24.8 | MIT | https://github.com/tree-sitter/tree-sitter-json |
| `tree-sitter-md` (block + inline) | 0.5.4 | MIT | https://github.com/tree-sitter-grammars/tree-sitter-markdown |
| `tree-sitter-toml-ng` | 0.7.0 | MIT | https://github.com/tree-sitter-grammars/tree-sitter-toml |
| `tree-sitter-yaml` | 0.7.2 | MIT | https://github.com/tree-sitter-grammars/tree-sitter-yaml |
| `tree-sitter-bash` | 0.25.1 | MIT | https://github.com/tree-sitter/tree-sitter-bash |
| `tree-sitter-html` | 0.23.2 | MIT | https://github.com/tree-sitter/tree-sitter-html |
| `tree-sitter-css` | 0.25.0 | MIT | https://github.com/tree-sitter/tree-sitter-css |

## Website fonts

The landing page in `site/` self-hosts these fonts. Each file is the latin
variable `woff2` build from Fontsource. License texts ship next to the files in `site/fonts/`.

| Font | License | Upstream |
|---|---|---|
| Bricolage Grotesque | OFL-1.1 | https://github.com/ateliertriay/bricolage |
| Instrument Sans | OFL-1.1 | https://github.com/Instrument/instrument-sans |
| JetBrains Mono | OFL-1.1 | https://github.com/JetBrains/JetBrainsMono |

## Additional licenses

Licenses outside the base list that were added to `deny.toml` are recorded
here with the crate that required them.


- `notify` (file watching in `lucerna-project`): CC0-1.0, a public-domain
  dedication. Allowed for this crate only through a `[licenses.exceptions]`
  entry in `deny.toml`; the global allow list is unchanged.

## Themes

`lucerna-theme` bundles the "One" theme family (One Dark, One Light) as a data
file: `assets/themes/one/one.json` from the Zed repository
(https://github.com/zed-industries/zed) at revision
`b5cd310b3b54dfc659fcc06453e6358843dfe216`, stored as
`crates/lucerna-theme/assets/one.json`. It is MIT licensed (Copyright (c) 2014
GitHub Inc.); the license text is `crates/lucerna-theme/assets/LICENSE-one`.
