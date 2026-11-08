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

## Additional licenses

Licenses outside the base list that were added to `deny.toml` are recorded
here with the crate that required them.


- `notify` (file watching in `lucerna-project`): CC0-1.0, a public-domain
  dedication. Allowed for this crate only through a `[licenses.exceptions]`
  entry in `deny.toml`; the global allow list is unchanged.
