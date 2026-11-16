#!/bin/sh
# Writes Lucerna.app's Info.plist for a channel to stdout, from the template
# crates/lucerna/resources/macos/Info.plist.
#
# Usage: scripts/render-plist.sh <dev|stable> <version> <build>
#   dev     dev.lucerna.Lucerna-Dev, "Lucerna Dev": builds run from the tree
#   stable  dev.lucerna.Lucerna, "Lucerna": installed, DMG and releases
#
# Dev builds get their own bundle ID, as Zed's do: macOS sends Open requests
# (App Exposé's recent folders, the Dock, Open With) by bundle ID, so copies
# that shared one could get each other's requests.
# Spec: docs/superpowers/specs/2026-11-16-dev-bundle-id-design.md
set -eu

[ $# -eq 3 ] || { echo "usage: render-plist.sh <dev|stable> <version> <build>" >&2; exit 2; }
channel=$1
version=$2
build=$3
case $channel in
  dev) id=dev.lucerna.Lucerna-Dev; name="Lucerna Dev" ;;
  stable) id=dev.lucerna.Lucerna; name=Lucerna ;;
  *) echo "render-plist: unknown channel: $channel" >&2; exit 2 ;;
esac

root=$(cd "$(dirname "$0")/.." && pwd)
sed -e "s/@VERSION@/$version/g" -e "s/@BUILD@/$build/g" \
  -e "s/@BUNDLE_ID@/$id/g" -e "s/@BUNDLE_NAME@/$name/g" \
  "$root/crates/lucerna/resources/macos/Info.plist"
