# Third-party license notices — research

Date: 2026-11-13.

## Why

MIT, Apache-2.0 (§4(a), §4(d)), BSD, ISC, Zlib and the SIL OFL all require
the copyright notice and license text to travel with copies of the
software, binaries included. Lucerna.app links about 500 crates and embeds
fonts (Lilex, IBM Plex: OFL-1.1), icons (Lucide: ISC) and the One themes
(MIT). Before the first public release the app must carry these notices.

## VS Code

- Ships `ThirdPartyNotices.txt` inside the app (`Contents/Resources/app/`),
  generated from component manifests at build time.
- Help ▸ View License opens the product license URL; the notices file sits
  in the bundle for anyone who looks.

## Zed

- `script/generate-licenses` runs `cargo about` with a template and writes
  `assets/licenses.md`, built into the app at release time (not committed).
- The command `zed: open licenses` opens that file in an editor tab.
- Theme and icon licenses are appended from the asset folders.

## Choice for Lucerna

- **Generator:** `scripts/gen-licenses.py` reads `cargo metadata` (filtered
  to `aarch64-apple-darwin`, normal dependencies reachable from the
  `lucerna` crate) and the license files inside each crate's source, groups
  crates by identical text, and appends the embedded asset licenses.
  Behaviour follows Zed's generated `licenses.md`; it does not use
  `cargo about`, so building needs only cargo and python3 (installing a new
  tool was not possible here, and fewer tools on CI is simpler).
- Crates that publish no license file are listed with their SPDX license
  and repository; all are MIT or Apache-2.0, so the standard texts of both
  follow. The script stops if a crate without a file uses any other license.
- **Where it ships:** `scripts/bundle-app.sh` writes it to
  `Lucerna.app/Contents/Resources/licenses.md` (about 720 KB).
- **How to see it:** Help ▸ Third-Party Licenses
  (`workbench.action.openThirdPartyLicenses`) opens it in a tab, as Zed
  does; the About dialog has a Licenses button (opens with the system,
  since the Welcome screen has no editor).
- Linux and Windows: the command also looks next to the binary, so those
  packages can put `licenses.md` beside `luc`.
