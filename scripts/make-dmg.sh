#!/bin/sh
# Packs Lucerna.app into a drag-to-install Lucerna.dmg: opening it shows a
# fixed-size Finder window with the app, an arrow and an Applications link.
#
# Usage: scripts/make-dmg.sh [--debug]
#   --debug  pack the dev build (default: release)
# Builds Lucerna.app first (scripts/bundle-app.sh), so the image always holds
# the current build, and ejects a Lucerna volume left mounted from an
# earlier image.
#
# Uses only tools that ship with macOS: hdiutil builds the image, Finder
# (through osascript) lays out the window, sips and tiffutil render the
# background. The first run asks to let this terminal control Finder. If
# the layout step fails, the DMG still works but opens unarranged.
# Research and choices: docs/research/2026-11-09-app-bundle.md.
set -eu

profile=release
for arg in "$@"; do
  case $arg in
    --debug) profile=debug ;;
    -h | --help) sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "make-dmg: unknown option: $arg" >&2; exit 2 ;;
  esac
done

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
# Where cargo puts builds: not always ./target (see scripts/target-dir.sh).
target=$(scripts/target-dir.sh)

app="$target/$profile/Lucerna.app"
dmg="$target/$profile/Lucerna.dmg"
volume=Lucerna
lsregister=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister

# Finder registers the app on any mounted Lucerna volume with LaunchServices,
# and the entry can outlive the volume. Stale copies make "Open With" and the
# default app pick an old build, so drop the entry before and after ejecting.
forget_volume_app() {
  "$lsregister" -u "/Volumes/$volume/Lucerna.app" 2>/dev/null || true
}

if [ "$profile" = debug ]; then
  scripts/bundle-app.sh --debug
else
  scripts/bundle-app.sh
fi

# An earlier image still mounted: its old app would stay registered with
# LaunchServices, and its volume name would clash with the new one.
if [ -e "/Volumes/$volume" ]; then
  forget_volume_app
  hdiutil detach "/Volumes/$volume" -quiet || {
    echo "make-dmg: could not eject /Volumes/$volume; close what uses it and retry" >&2
    exit 1
  }
  forget_volume_app
  echo "make-dmg: ejected the earlier /Volumes/$volume"
fi

# Window content size, matching dmg-background.svg, and icon centres.
width=540
height=380
icon_y=170

work=$(mktemp -d)
device=
cleanup() {
  [ -z "$device" ] || hdiutil detach "$device" -quiet -force || true
  rm -rf "$work"
}
trap cleanup EXIT

# Background at 1x and 2x in one TIFF, so Retina displays get the sharp one.
svg=crates/lucerna/resources/macos/dmg-background.svg
sips -s format png "$svg" --out "$work/bg.png" >/dev/null
sips -s format png -z $((height * 2)) $((width * 2)) "$svg" --out "$work/bg@2x.png" >/dev/null
stage="$work/stage"
mkdir -p "$stage/.background"
tiffutil -cathidpicheck "$work/bg.png" "$work/bg@2x.png" \
  -out "$stage/.background/background.tiff" 2>/dev/null
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"

hdiutil create -srcfolder "$stage" -volname "$volume" -fs HFS+ \
  -format UDRW -ov "$work/rw.dmg" -quiet
device=$(hdiutil attach -readwrite -noverify -noautoopen "$work/rw.dmg" |
  awk '/Apple_HFS/ { print $1 }')
mount="/Volumes/$volume"

# The window's bounds include its ~28 pt title bar.
if osascript - "$volume" "$width" "$height" "$icon_y" <<'EOF'
on run argv
  set volumeName to item 1 of argv
  set w to (item 2 of argv) as integer
  set h to (item 3 of argv) as integer
  set iconY to (item 4 of argv) as integer
  tell application "Finder"
    tell disk volumeName
      open
      set current view of container window to icon view
      set toolbar visible of container window to false
      set statusbar visible of container window to false
      set pathbar visible of container window to false
      set the bounds of container window to {200, 120, 200 + w, 120 + h + 28}
      set opts to the icon view options of container window
      set arrangement of opts to not arranged
      set icon size of opts to 96
      set text size of opts to 13
      set background picture of opts to file ".background:background.tiff"
      set position of item "Lucerna.app" of container window to {140, iconY}
      set position of item "Applications" of container window to {400, iconY}
      close
      open
      update without registering applications
      delay 1
      close
    end tell
  end tell
end run
EOF
then
  # Finder writes .DS_Store lazily; wait for it before detaching.
  tries=0
  while [ ! -f "$mount/.DS_Store" ] && [ $tries -lt 20 ]; do
    sleep 0.5
    tries=$((tries + 1))
  done
  [ -f "$mount/.DS_Store" ] || echo "make-dmg: warning: Finder saved no layout" >&2
else
  echo "make-dmg: warning: Finder layout failed; the DMG opens unarranged" >&2
fi

chmod -Rf go-w "$mount" || true
sync
forget_volume_app
hdiutil detach "$device" -quiet
device=
forget_volume_app

rm -f "$dmg"
hdiutil convert "$work/rw.dmg" -format UDZO -imagekey zlib-level=9 -o "$dmg" -quiet
codesign --force --sign - "$dmg"
echo "make-dmg: built $dmg ($(du -h "$dmg" | cut -f1 | tr -d ' '))"
