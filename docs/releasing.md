# Releasing Lucerna

Releases are built on GitHub by `.github/workflows/release.yml`, so nothing large has to be built
on a local machine.

The repository is public, so the macOS runners the workflow uses are free.

## Cut a release

1. Set the new version in `[workspace.package]` in `Cargo.toml` and commit it on `main`.
2. Tag that commit with the same version and push the tag:

   ```sh
   git tag v0.2.0
   git push origin v0.2.0
   ```

3. The **Release** workflow runs on `macos-15` (Apple silicon). It:
   - checks that the tag matches the version in `Cargo.toml`,
   - builds `Lucerna.app` with `scripts/bundle-app.sh` (release, signed ad hoc),
   - signs with a Developer ID and notarizes, only if the `APPLE_*` secrets are set,
   - packs the signed app into `Lucerna-<version>-macos-aarch64.dmg` with
     `scripts/make-dmg.sh --no-build --strict` (the job fails if Finder cannot lay out the window),
   - packs the same app with `ditto -c -k --keepParent` into `Lucerna-<version>-macos-aarch64.zip`,
   - writes a `.sha256` file beside each, checkable with `shasum -a 256 -c`,
   - creates the GitHub Release with `gh release create` and attaches all four files.

To try the pipeline without publishing, run the workflow by hand (Actions → Release → Run
workflow). A manual run builds and packs only, and keeps the DMG and zip as a workflow artifact for 7 days.

## Opening an unnotarized build

Until the app is notarized, macOS Gatekeeper blocks the first launch of a downloaded copy. Either
right-click `Lucerna.app` and choose **Open**, or remove the quarantine flag:

```sh
xattr -dr com.apple.quarantine /Applications/Lucerna.app
```

## Notarizing (prepared, not enabled)

The "Sign and notarize" step runs only when all of these repository secrets are set:

| Secret | Value |
| --- | --- |
| `APPLE_CERTIFICATE` | The Developer ID Application certificate and key, exported as `.p12`, base64-encoded (`base64 -i cert.p12 \| pbcopy`) |
| `APPLE_CERTIFICATE_PASSWORD` | The password of that `.p12` |
| `APPLE_SIGNING_IDENTITY` | The identity name, e.g. `Developer ID Application: Name (TEAMID)` |
| `APPLE_ID` | The Apple ID email used for notarization |
| `APPLE_TEAM_ID` | The 10-character team ID |
| `APPLE_APP_PASSWORD` | An app-specific password for that Apple ID |

The step imports the certificate into a throwaway keychain, re-signs `luc` and the bundle with the
hardened runtime and a timestamp, submits the app to `notarytool`, and staples the ticket.

If Lucerna later embeds a JIT (for example wasmtime for the WASM extension host), the hardened
runtime will need a `com.apple.security.cs.allow-jit` entitlements file passed to `codesign`.

## Later

- **Universal binary.** Only Apple silicon is built today. For Intel Macs, add the
  `x86_64-apple-darwin` target, build `luc` for both targets, merge them with
  `lipo -create -output luc <arm64> <x86_64>`, and bundle that. This needs a `--target` option (or a
  prebuilt binary path) in `scripts/bundle-app.sh`, which today always builds `target/release/luc`.
- **Linux and Windows** packages come with cross-platform support.
