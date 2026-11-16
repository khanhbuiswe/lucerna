#!/bin/sh
# Fails unless the app at $1 is the stable channel (dev.lucerna.Lucerna), so a
# DMG or release never ships a dev build ("Lucerna Dev"), whose bundle ID every
# build in the tree shares.
# Usage: scripts/check-stable-app.sh <path/to/Lucerna.app>
# Spec: docs/superpowers/specs/2026-11-16-dev-bundle-id-design.md
set -eu

[ $# -eq 1 ] || { echo "usage: check-stable-app.sh <app>" >&2; exit 2; }
plist="$1/Contents/Info.plist"
[ -f "$plist" ] || { echo "check-stable-app: no $plist" >&2; exit 1; }
# The value on the line after the key, as render-plist.sh writes it (XML).
id=$(sed -n '/<key>CFBundleIdentifier<\/key>/{n;s/.*<string>\(.*\)<\/string>.*/\1/p;}' "$plist")
if [ "$id" != dev.lucerna.Lucerna ]; then
  echo "check-stable-app: $1 is ${id:-unknown}, not dev.lucerna.Lucerna; build it with scripts/bundle-app.sh --stable" >&2
  exit 1
fi
