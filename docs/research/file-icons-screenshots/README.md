# Colored file icons and tab icons: before and after screenshots

Taken on 2026-11-15, window 1470×923. All four images use the same sample folder (`src/app.ts`, `src/app.test.ts`, `src/main.rs`, `Dockerfile`, `docker-compose.yml`, `.env`, `.env.example`, `.gitignore`, `package.json`, `tsconfig.json`, `README.md`, `LICENSE`, `Cargo.toml`, `notes.txt`, `logo.png`, `Makefile`), the same 7 pre-opened tabs (restored from the workspace state file), a temporary HOME and no extensions.

| | Dark | Light |
|---|---|---|
| Before (Lucide icon theme, `target/release/luc` built from main) | `before-dark.png` | `before-light.png` |
| After (Seti by default, branch `claude/file-icons`) | `after-dark.png` | `after-light.png` |

- The theme is selected with `workbench.colorTheme` and `"window.autoDetectColorScheme": false`.
- File tabs have an icon at the same 14 px size as the icons in the Explorer. Folders still use the Lucide folder icon.
- On the light theme, colors below 3:1 contrast (yellow, gray) were darkened: see `package.json`, `LICENSE`, `notes.txt`.
- The "Restricted Mode" strip is present because the sample folder is not yet trusted.
