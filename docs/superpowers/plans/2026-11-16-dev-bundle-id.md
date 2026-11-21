# Dev Bundle ID Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** App builds from the build tree get the bundle id `dev.lucerna.Lucerna-Dev` and the name "Lucerna Dev"; the installed app, the DMG and GitHub releases keep `dev.lucerna.Lucerna` and "Lucerna".

**Architecture:** `Info.plist` gets `@BUNDLE_ID@` and `@BUNDLE_NAME@` placeholders. A new `scripts/render-plist.sh` fills every placeholder for a channel (`dev` or `stable`). `bundle-app.sh` calls it; `--stable` (implied by `--install`) picks the stable channel. A Python unittest covers the rendered plist.

**Tech Stack:** POSIX sh, `plutil`, Python 3 `unittest` + `plistlib`.

**Spec:** `docs/superpowers/specs/2026-11-16-dev-bundle-id-design.md`

## Global Constraints

- Dev channel: `CFBundleIdentifier` = `dev.lucerna.Lucerna-Dev`; `CFBundleName` and `CFBundleDisplayName` = `Lucerna Dev`.
- Stable channel: `dev.lucerna.Lucerna`, `Lucerna`, as today.
- The default with no flag is dev. `--install` always builds stable.
- `Lucerna.app` (folder name) and `luc` (executable, `CFBundleExecutable`) stay unchanged.
- Data dir, log dir and `luc` socket stay unchanged; no Rust code reads the bundle id.
- The app shows English only; script messages are English.
- Stage files by explicit path; commit trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Before any cargo command: `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`, `scripts/prune-target.sh`, and never touch another session's running cargo (`pgrep -x cargo`).

## Review Focus

- A release built by CI or `make-dmg.sh` that comes out as "Lucerna Dev": Task 2 Step 6 greps both callers for `--stable`.
- `--install` without `--stable` installs a dev-id copy into `/Applications`: Task 2 test `test_install_implies_stable` (the script's flag parsing).
- An unknown channel silently renders a dev or broken plist: `test_unknown_channel_fails`.
- A placeholder left unreplaced (e.g. `@BUILD@`) yields an app LaunchServices rejects: `test_no_placeholder_left` on both channels.
- `CFBundleExecutable` renamed with the display name breaks launching: `test_executable_stays_luc`.

---

### Task 1: Commit the diagnostic logs

The three `tracing` lines are already in `crates/lucerna-workbench/src/app.rs` (uncommitted): `the platform asks to open` (with `urls`) and a `warn` when the channel send fails in the `on_open_urls` closure; `the platform asks to reopen` in `on_reopen`; `opening the platform's paths` (with `paths`) in `open_urls`.

**Files:**
- Modify: `crates/lucerna-workbench/src/app.rs` (already edited)

- [ ] **Step 1: Check it builds and lints**

Run: `cargo clippy -p lucerna-workbench --all-targets -- -D warnings`
Expected: no warnings, exit 0.

- [ ] **Step 2: Commit**

```bash
git commit -m "chore(app): log open and reopen requests from the platform" -- crates/lucerna-workbench/src/app.rs
```

### Task 2: Dev and stable channels in the app bundle

**Files:**
- Create: `scripts/render-plist.sh`
- Create: `scripts/test_bundle_plist.py`
- Modify: `crates/lucerna/resources/macos/Info.plist:3-15` (comment, `CFBundleName`, `CFBundleDisplayName`, `CFBundleIdentifier`)
- Modify: `scripts/bundle-app.sh` (usage, flag parsing, the `sed` at lines 65-67, the final `echo`)
- Modify: `scripts/make-dmg.sh:58,60`
- Modify: `.github/workflows/release.yml:68`
- Modify: `.github/workflows/pages.yml` (run the new test, add it to `paths`)
- Modify: `docs/research/2026-11-09-app-bundle.md:26`

**Interfaces:**
- Produces: `scripts/render-plist.sh <dev|stable> <version> <build>` writes the rendered plist to stdout; any other channel prints `render-plist: unknown channel: <x>` to stderr and exits 2. Reads the template at `crates/lucerna/resources/macos/Info.plist` relative to the repo root (found from `$0`).
- Produces: `scripts/bundle-app.sh [--debug] [--stable] [--install]`, with `--install` setting the channel to stable.

- [ ] **Step 1: Write the failing tests** in `scripts/test_bundle_plist.py` (docstring: `Run: python3 -m unittest scripts/test_bundle_plist.py -v`, spec path). A helper runs `render-plist.sh` with `subprocess.run`, then `plistlib.loads` the stdout.

```python
def test_dev_channel(self):
    p = render("dev")
    self.assertEqual(p["CFBundleIdentifier"], "dev.lucerna.Lucerna-Dev")
    self.assertEqual(p["CFBundleName"], "Lucerna Dev")
    self.assertEqual(p["CFBundleDisplayName"], "Lucerna Dev")

def test_stable_channel(self):
    p = render("stable")
    self.assertEqual(p["CFBundleIdentifier"], "dev.lucerna.Lucerna")
    self.assertEqual(p["CFBundleName"], "Lucerna")
    self.assertEqual(p["CFBundleDisplayName"], "Lucerna")

def test_version_and_build(self):  # both channels
    p = render(channel, version="1.2.3", build="20261116.221211")
    self.assertEqual(p["CFBundleShortVersionString"], "1.2.3")
    self.assertEqual(p["CFBundleVersion"], "20261116.221211")

def test_executable_stays_luc(self):  # both channels
    self.assertEqual(render(channel)["CFBundleExecutable"], "luc")

def test_no_placeholder_left(self):  # both channels: raw stdout
    self.assertNotIn("@", raw_output(channel))

def test_unknown_channel_fails(self):
    r = run("beta")
    self.assertEqual(r.returncode, 2)
    self.assertIn("unknown channel: beta", r.stderr)

def test_install_implies_stable(self):
    # bundle-app.sh exposes its parsed channel without building:
    # `LUCERNA_BUNDLE_DRY_RUN=1 scripts/bundle-app.sh --install` prints
    # `channel=stable` and exits 0 before any cargo or file work.
    self.assertIn("channel=stable", dry_run("--install"))
    self.assertIn("channel=dev", dry_run())
    self.assertIn("channel=stable", dry_run("--stable"))
```

- [ ] **Step 2: Run them to see them fail**

Run: `python3 -m unittest scripts/test_bundle_plist.py -v`
Expected: FAIL / ERROR (`render-plist.sh` missing).

- [ ] **Step 3: Implement**

- `Info.plist`: `CFBundleName` and `CFBundleDisplayName` become `@BUNDLE_NAME@`, `CFBundleIdentifier` becomes `@BUNDLE_ID@`; the top comment lists all four placeholders and points at `scripts/render-plist.sh`.
- `scripts/render-plist.sh`: `set -eu`, a `case` on the channel giving the id and name, one `sed` replacing `@VERSION@`, `@BUILD@`, `@BUNDLE_ID@`, `@BUNDLE_NAME@`. Executable bit set.
- `scripts/bundle-app.sh`: `channel=dev`; `--stable` sets `channel=stable`; `--install` sets `install=1` and `channel=stable`. Right after argument parsing, `[ "${LUCERNA_BUNDLE_DRY_RUN:-}" = 1 ] && { echo "channel=$channel"; exit 0; }`. Replace the `sed` with `scripts/render-plist.sh "$channel" "$version" "$build" > "$contents/Info.plist"` (keep `plutil -lint`). Usage text describes `--stable`. The final echo adds the channel: `... ($version, build $build, $profile, $channel)`.

- [ ] **Step 4: Run the tests to see them pass**

Run: `python3 -m unittest scripts/test_bundle_plist.py -v`
Expected: all OK.

- [ ] **Step 5: Lint a rendered plist for each channel**

Run: `for c in dev stable; do scripts/render-plist.sh $c 0.1.0 1 | plutil -lint -; done`
Expected: `<stdin>: OK` twice.

- [ ] **Step 6: Callers and docs**

- `scripts/make-dmg.sh`: both calls gain `--stable` (`scripts/bundle-app.sh --debug --stable`, `scripts/bundle-app.sh --stable`).
- `.github/workflows/release.yml`: `run: scripts/bundle-app.sh --stable`.
- `.github/workflows/pages.yml`: add `scripts/test_bundle_plist.py`, `scripts/render-plist.sh`, `scripts/bundle-app.sh` and `crates/lucerna/resources/macos/Info.plist` to both `paths` lists, and a step `python3 -m unittest scripts/test_bundle_plist.py`.
- `docs/research/2026-11-09-app-bundle.md`: after the Bundle ID sentence, one paragraph: the default build is the dev channel (`dev.lucerna.Lucerna-Dev`, "Lucerna Dev"), and `--stable` or `--install` use the main id, like Zed; the reason is that App Exposé sends open requests by bundle id to the wrong copy (spec 2026-11-16).

Check: `grep -n "bundle-app.sh" scripts/make-dmg.sh .github/workflows/release.yml`
Expected: every call line contains `--stable`.

- [ ] **Step 7: Commit**

```bash
git commit -m "build(bundle): dev builds get their own bundle id, as in Zed" -- scripts/render-plist.sh scripts/test_bundle_plist.py scripts/bundle-app.sh scripts/make-dmg.sh crates/lucerna/resources/macos/Info.plist .github/workflows/release.yml .github/workflows/pages.yml docs/research/2026-11-09-app-bundle.md
```

### Task 3: Check a real build

- [ ] **Step 1: Build the default (dev) bundle**

Run (after `pgrep -x cargo` shows nothing, then `scripts/prune-target.sh`): `scripts/bundle-app.sh`
Expected: last line ends with `release, dev)`.

- [ ] **Step 2: Read the built plist**

Run: `defaults read "$(scripts/target-dir.sh)/release/Lucerna.app/Contents/Info.plist" CFBundleIdentifier`
Expected: `dev.lucerna.Lucerna-Dev`.

- [ ] **Step 3: Hand over to the user** to open that app, press Ctrl+↓ and click recent folders; the log should show `the platform asks to open` for each click. The menu bar reads "Lucerna Dev".
