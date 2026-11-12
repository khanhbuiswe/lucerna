#!/bin/sh
# Builds Lucerna.app, so macOS shows "Lucerna" and its icon in the menu bar
# and Dock instead of the bare `luc` process name.
#
# Usage: scripts/bundle-app.sh [--debug] [--install]
#   --debug    bundle the dev build (default: release)
#   --install  copy the app to /Applications and put a `luc` shim on PATH
#              ($LUC_BIN_DIR, default ~/.local/bin)
#
# Uses only tools that ship with macOS: sips renders the SVG icon, iconutil
# packs the .icns, plutil checks the plist, codesign signs ad hoc.
# Research and choices: docs/research/2026-11-09-app-bundle.md.
set -eu

profile=release
install=0
for arg in "$@"; do
  case $arg in
    --debug) profile=debug ;;
    --install) install=1 ;;
    -h | --help) sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "bundle-app: unknown option: $arg" >&2; exit 2 ;;
  esac
done

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

# Disk is tight: a release build removes the debug build and vice versa, so
# only one profile's artifacts are ever on disk. Skipped while another cargo
# runs (another session may be building or testing in this folder).
case $profile in
  release) other=debug ;;
  *) other=release ;;
esac
if [ -d "target/$other" ]; then
  if pgrep -x cargo >/dev/null 2>&1; then
    echo "bundle-app: another cargo is running; keeping target/$other" >&2
  else
    echo "bundle-app: removing the old $other build (target/$other)"
    rm -rf "target/$other"
  fi
fi

if [ "$profile" = release ]; then
  cargo build --release --bin luc
else
  cargo build --bin luc
fi

version=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)
[ -n "$version" ] || { echo "bundle-app: no version in Cargo.toml" >&2; exit 1; }

app="target/$profile/Lucerna.app"
contents="$app/Contents"
rm -rf "$app"
mkdir -p "$contents/MacOS" "$contents/Resources"

# CFBundleVersion is the build time, as Zed does: LaunchServices and Finder
# then treat the latest build as the newest copy.
build=$(date +%Y%m%d.%H%M%S)
sed -e "s/@VERSION@/$version/g" -e "s/@BUILD@/$build/g" \
  crates/lucerna/resources/macos/Info.plist \
  > "$contents/Info.plist"
plutil -lint -s "$contents/Info.plist"
printf 'APPL????' > "$contents/PkgInfo"
cp "target/$profile/luc" "$contents/MacOS/luc"

# The icon: the 1024 px tile shrunk to Apple's 824 px icon grid, centred on
# a transparent 1024 px canvas, then every size iconutil expects.
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
sips -s format png -z 824 824 assets/brand/app-icon.svg --out "$work/tile.png" >/dev/null
sips -p 1024 1024 "$work/tile.png" --out "$work/icon-1024.png" >/dev/null
iconset="$work/Lucerna.iconset"
mkdir "$iconset"
for size in 16 32 128 256 512; do
  double=$((size * 2))
  sips -z "$size" "$size" "$work/icon-1024.png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
  sips -z "$double" "$double" "$work/icon-1024.png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$contents/Resources/Lucerna.icns"

codesign --force --sign - "$app"
echo "bundle-app: built $app ($version, build $build, $profile)"

if [ "$install" -eq 1 ]; then
  dest=/Applications/Lucerna.app
  rm -rf "$dest"
  cp -R "$app" "$dest"
  bin_dir=${LUC_BIN_DIR:-"$HOME/.local/bin"}
  mkdir -p "$bin_dir"
  # A shim, not a symlink or `open -a`: running the bundled binary keeps the
  # caller's cwd, stdio and exit code, as `luc` does today.
  cat > "$bin_dir/luc" <<EOF
#!/bin/sh
exec "$dest/Contents/MacOS/luc" "\$@"
EOF
  chmod +x "$bin_dir/luc"
  # LaunchServices registers every copy it sees and may pick the build copy
  # under target/ for "Open With" and the default app, since all share one
  # bundle ID. Keep only the installed one registered.
  lsregister=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
  for copy in target/release/Lucerna.app target/debug/Lucerna.app; do
    [ -d "$copy" ] && "$lsregister" -u "$copy" 2>/dev/null || true
  done
  "$lsregister" -f "$dest"
  echo "bundle-app: installed $dest and $bin_dir/luc"
  case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *) echo "bundle-app: add $bin_dir to PATH to use \`luc\`" >&2 ;;
  esac
fi
