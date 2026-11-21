# Lucerna theme: before and after screenshots

Same window (1470×923), same file `crates/lucerna-theme/src/registry.rs`, taken on 2026-11-14.

| | Dark | Light |
|---|---|---|
| Before (One, `/Applications/Lucerna.app` from 2026-11-13) | `before-dark.png` | `before-light.png` |
| After (Lucerna, branch `claude/lucerna-theme`) | `after-dark.png` | `after-light.png` |

- `after-dark.png` sets `window.autoDetectColorScheme: false` and `workbench.colorTheme: "Lucerna Dark"`, because the capture machine was in light mode.
- `after-light.png` uses the default settings: the theme is chosen by the system's light mode.
- All four images ran with a temporary HOME and no extensions.
- The "Restricted Mode" strip in the title bar is present because the project is not yet trusted.
