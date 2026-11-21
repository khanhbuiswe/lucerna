# A separate bundle id for dev builds — short design

## Why

Opening a recent folder with App Exposé (Ctrl+↓) works sometimes and not
other times. The log shows that on failures no open request reached the
running app. At that time the running app was the `target/release/Lucerna.app`
build. With the `/Applications/Lucerna.app` build, it opened correctly every
time (the user tried about 15 times on 2026-11-16, plus three
`open -b` runs).

Cause: every build carries the bundle id `dev.lucerna.Lucerna`
(`bundle-app.sh` copies `Info.plist` as is). LaunchServices has 6 such builds
registered, including the one in `target/`, `~/LucernaBench.noindex`, an
ejected DMG and another session's temp folder. When macOS opens a document
by bundle id, it may pick a build other than the running one, even one that
has been deleted, so the open request never reaches the app.

## Goals

- A build run from the build tree (not installed) has its own bundle id and
  name, so its Ctrl+↓, Dock and "Open With" do not mix with the main build.
- The build installed in `/Applications`, the DMG and the GitHub release
  build keep `dev.lucerna.Lucerna` / "Lucerna".

## Non-goals

- No data split: the dev build still shares `~/Library/Application Support/Lucerna`,
  the log and the `luc` socket, because these paths are hard-coded, not
  derived from the bundle id. Running two builds at once, `luc` talks to
  only one of them, as it does today.
- No separate icon for the dev build.
- No change to the folder-opening code and window selection (`window_choice`),
  since they are already correct.

## Approaches considered

1. **The dev build has its own bundle id (chosen).** This is how Zed does it:
   `dev.zed.Zed`, `dev.zed.Zed-Preview`, `dev.zed.Zed-Dev`. VS Code also
   splits Insiders into `com.microsoft.VSCodeInsiders`. Limit: dev builds
   still share an id with each other.
2. Only change the workflow: always `--install` and then unregister old
   builds with `lsregister -u`. Dropped because it is easy to regress, as
   other sessions keep building new builds.
3. Both 1 and a step that unregisters old dev builds. Later, if 1 is not enough.

## Design

- `crates/lucerna/resources/macos/Info.plist`: `CFBundleIdentifier` and
  `CFBundleName` (plus `CFBundleDisplayName` if present) become the
  placeholders `@BUNDLE_ID@` and `@BUNDLE_NAME@`.
- `scripts/bundle-app.sh` has two channels, like Zed (dev is the default,
  a release must say so explicitly):
  - default: `dev.lucerna.Lucerna-Dev`, "Lucerna Dev";
  - `--stable`: `dev.lucerna.Lucerna`, "Lucerna". `--install` turns on
    `--stable` by itself.
  - The `.app` folder name and the binary file name stay `Lucerna.app` / `luc`,
    so the scripts and docs that point to them need no change.
- `scripts/make-dmg.sh` and `.github/workflows/release.yml` pass `--stable`.
- Keep the three diagnostic log lines already added in
  `crates/lucerna-workbench/src/app.rs`: the open request and reopen from
  the OS, plus the paths it could read.

## Left to the implementer

- How to check the plist after a build: `plutil`, `defaults read`.
- Update `docs/research/2026-11-09-app-bundle.md` to record the two channels.

## References

- Zed: the release channel (stable, preview, nightly, dev) decides the bundle
  id and app name. The default build is dev.
- VS Code: Stable and Insiders have separate bundle ids so they can be
  installed side by side.
