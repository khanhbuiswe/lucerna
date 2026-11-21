# Packaging Lucerna as a macOS app bundle

Problem: running `luc` as a bare executable makes the menu bar show "luc" and the Dock show a default icon. macOS takes the app name from `CFBundleName` in the `Info.plist` of the bundle that contains the executable; `cx.set_menus` can't change this name.

## VS Code (see the install at `/Applications/Visual Studio Code.app`)

- `Info.plist`: `CFBundleName`/`CFBundleDisplayName` = Code, `CFBundleExecutable` = Code, `CFBundleIconFile` = Code.icns, `CFBundleIdentifier` = com.microsoft.VSCode, `LSMinimumSystemVersion` = 12.0, `NSHighResolutionCapable`, `LSApplicationCategoryType` = public.app-category.developer-tools.
- The `code` command is the shell script `Contents/Resources/app/bin/code`. The script follows symlinks to find the `.app`, then runs the Electron binary in `Contents/MacOS` itself (in Node mode) with `cli.js`; `cli.js` opens the app and passes the arguments over IPC to the running instance.
- The command "Shell Command: Install 'code' command in PATH" creates the symlink `/usr/local/bin/code` → the script above.

## Zed (see the install at `/Applications/Zed.app`)

- `Info.plist`: `CFBundleName`/`CFBundleDisplayName` = Zed, `CFBundleExecutable` = zed, `CFBundleIconFile` = Zed.icns, `CFBundleIdentifier` = dev.zed.Zed, `LSMinimumSystemVersion` = 12.0, `NSHighResolutionCapable`, category developer-tools.
- `Contents/MacOS` has three binaries: `zed` (the app), `cli` and `git`. The `zed` command is the `cli` binary: it finds the bundle (`Contents/MacOS/zed`), opens the app and sends the paths over IPC; the app runs as a single instance. The "cli: install" command creates a symlink to `cli`.
- Zed builds the bundle with `cargo-bundle` (the `[package.metadata.bundle]` metadata in `crates/zed/Cargo.toml`) in `script/bundle-mac`, then signs and notarizes it.

## Similarities and differences

- Similar: both ship an `.app` with the name and icon in `Info.plist`, and both install a CLI command into PATH via a symlink to a file inside the bundle.
- Different: VS Code uses a script and the app binary itself; Zed uses a separate CLI binary. Both pass arguments to a single app instance over IPC.
- Both icons follow Apple's standard grid: an 824 px tile centered in a 1024 px canvas.

## What Lucerna chooses

- **Build the bundle with `scripts/bundle-app.sh`**, not `cargo-bundle`: no new dependency, only `sips`, `iconutil`, `plutil` and `codesign`, which ship with macOS. The icon is rendered from `assets/brand/app-icon.svg` and scaled to an 824/1024 tile like VS Code and Zed.
- **`Info.plist`** uses the same set of keys as the other two. The bundle ID is `dev.lucerna.Lucerna`, in Zed's style. `LSMinimumSystemVersion` is 13.0, the level Lucerna supports. `CFBundleVersion` is the build time (`YYYYMMDD.HHMMSS`), like Zed (`20261104.154313`). When a machine has several copies with the same bundle ID (the installed one, one in `target/`, one on a mounted DMG), LaunchServices picks the copy with the highest version, which is the newest. `CFBundleShortVersionString` stays the Cargo version. `NSPrincipalClass` = `NSApplication` is added too: without this key, when running inside a bundle AppKit logs the error `getApplicationProperty: called with invalid property` at startup.
- **Two channels, like Zed** (`dev.zed.Zed`, `dev.zed.Zed-Preview`, `dev.zed.Zed-Dev`). By default `bundle-app.sh` builds the dev channel: `dev.lucerna.Lucerna-Dev`, named "Lucerna Dev". `--stable` (and `--install`, `make-dmg.sh`, the release workflow) gives `dev.lucerna.Lucerna`, named "Lucerna". Reason: macOS sends open requests (recent folders in App Exposé, the Dock, Open With) by bundle ID, so when the build in `target/` and the installed copy share an ID, a request sometimes reaches the wrong copy and the folder doesn't open. Spec: `docs/superpowers/specs/2026-11-16-dev-bundle-id-design.md`.
- **The `luc` command** is a VS Code-style shim: a shell script that `exec`s the `Contents/MacOS/luc` binary directly. The binary lives inside the bundle, so macOS uses the bundle's name and icon. Running the binary directly preserves cwd, stderr, the exit code, `luc ext …` and `--trace-startup`. It doesn't use `open -a`, because that drops stdout/stderr and the exit code.
- **Departure from both:** there is no single-instance app over IPC yet. Each `luc` is still its own process, as before. This is later work.
- **Opening from Finder or the Dock** (no path, cwd is `/`): shows a folder picker dialog, like VS Code's "Open Folder"; pressing Cancel quits. It does not open `/`, because scanning the whole disk is both slow and useless.
- **`cargo run`** stays as before: no bundle, the menu bar shows "luc".

## Drag-and-drop installer DMG

- **VS Code** ([`build/darwin/create-dmg.ts`](https://github.com/microsoft/vscode/blob/main/build/darwin/create-dmg.ts), `dmg-settings.py.template`) uses `dmgbuild` (Python, MIT) in its own venv. `dmgbuild` writes `.DS_Store` directly, with no Finder needed. Window 480×352, 80 px icons, the app at (120, 160), the `Applications` symlink at (360, 160), a `.tiff` background image, and the toolbar, sidebar, status bar and path bar hidden.
- **Zed** (`script/bundle-mac`) keeps it minimal: a folder containing `Zed.app` and a `/Applications` symlink, then `hdiutil create -format UDZO`. No background image or layout; the DMG is signed and notarized.
- **Lucerna chooses `scripts/make-dmg.sh`**: `hdiutil` plus AppleScript for Finder, like the `create-dmg` tool, but without adding Python or Node.
  - Layout in VS Code's style: window 540×380, 96 px icons, the app at (140, 170), Applications at (400, 170).
  - The background image is drawn from `crates/lucerna/resources/macos/dmg-background.svg`, with 1x and 2x versions combined with `tiffutil`.
  - A light background, because Finder draws icon labels in black over the background image, even in dark mode.
  - On the first run, macOS asks whether the terminal may control Finder. If the layout step fails, the DMG is still usable, like Zed's DMG.
  - If we later need to build on a CI machine without a display, switch to `dmgbuild` like VS Code.

## Opening files from Finder and other apps ("Open With", default app)

When macOS opens a file with an app (Finder "Open With", the default app, "Default app" in the "Open in" menu of Claude Desktop), the path is not in `argv`. It arrives as an "open URLs" event (the `odoc` Apple Event). If the app is already running, the event is delivered to that process.

- **VS Code.**
  - `Info.plist` has 65 `CFBundleDocumentTypes` entries, one per group of file extensions, with the `Editor` role.
  - Electron receives the `open-file` event. Events that arrive before the app is ready are collected and handled in one pass.
  - A file inside a folder that is open in a window opens as a tab in that window; otherwise it opens a new window.
- **Zed.**
  - `CFBundleDocumentTypes` has two entries. The first is `public.folder`, `public.text`, `public.plain-text`, `public.utf8-plain-text`, with the `Editor` role and the `Alternate` rank. The second is "Zed Text Document" for 28 file extensions, with the `Default` rank.
  - GPUI's `on_open_urls` pushes URLs through a channel; the processing loop converts `file://` into a path. A path inside an open workspace opens there, otherwise it opens a new window.
- **The "Open in" menu of Claude Desktop** comes from a fixed list of bundle IDs in Claude.app (`com.microsoft.VSCode`, `dev.zed.Zed`…). Other apps can't add themselves. The menu's "Default app" entry opens the file with the macOS default app.

**Lucerna chooses:**
- Declare one `CFBundleDocumentTypes` entry like Zed's first one, with the `Editor` role and the `Alternate` rank, for folders, text and source code. Lucerna appears in "Open With" and can be chosen as the default app, but doesn't grab default status from other apps on its own.
- Each Lucerna process still has one window, because the workspace configuration (`watch_config`) is a global tied to a single root folder. When it receives an open request:
  - if there is no window yet, it opens a window by the same rules as `luc <path>`;
  - if the file is under the root folder of the open window, it opens as a tab there, the VS Code way;
  - otherwise it runs a new `luc <path>` process, as if you typed `luc` again.
- When the app starts with no path, it waits 300 ms for an open-file event before showing the folder picker dialog.
