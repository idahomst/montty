#!/usr/bin/env bash
# Builds montty-<version>-macos.dmg (montty.app, ad-hoc signed) from an already built binary
# (universal in the release workflow). macOS only: iconutil, codesign and hdiutil.
# Run from the repo root: packaging/macos/build-app.sh <version> <path-to-binary> <out-dir>
set -euo pipefail

if [ "$#" -ne 3 ]; then
    echo "usage: $0 <version> <path-to-binary> <out-dir>" >&2
    exit 2
fi
version=$1
binary=$2
out_dir=$3

[ -f "$binary" ] || { echo "no such binary: $binary" >&2; exit 1; }
[ -f packaging/macos/Info.plist ] || { echo "run from the repo root" >&2; exit 1; }

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
app=$stage/dmg/montty.app
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

install -m755 "$binary" "$app/Contents/MacOS/montty"
sed "s/@VERSION@/$version/g" packaging/macos/Info.plist > "$app/Contents/Info.plist"

# macOS icons keep a transparent margin (Apple's grid: an 824 px tile on a 1024 px canvas);
# the hicolor tile fills its square and would look a size too big in the Dock. Each entry is
# the 512 px icon shrunk to that share and padded back out. iconutil wants name@2x pairs;
# there is no 1024 px source, so no icon_512x512@2x.
iconset=$stage/montty.iconset
mkdir -p "$iconset"
icon() { # <pixels> <name>
    local tile=$(( $1 * 824 / 1024 ))
    sips -s format png -z "$tile" "$tile" assets/icon/hicolor/512x512/apps/montty.png \
        --out "$stage/tile.png" >/dev/null
    sips -p "$1" "$1" "$stage/tile.png" --out "$iconset/$2.png" >/dev/null
}
icon 16 icon_16x16
icon 32 icon_16x16@2x
icon 32 icon_32x32
icon 64 icon_32x32@2x
icon 128 icon_128x128
icon 256 icon_128x128@2x
icon 256 icon_256x256
icon 512 icon_256x256@2x
icon 512 icon_512x512
iconutil -c icns -o "$app/Contents/Resources/montty.icns" "$iconset"

# Ad-hoc: no Developer ID yet (see docs/install.md for the first launch).
codesign --force --sign - "$app"

ln -s /Applications "$stage/dmg/Applications"
mkdir -p "$out_dir"
dmg=$out_dir/montty-$version-macos.dmg
hdiutil create -volname montty -srcfolder "$stage/dmg" -ov -format UDZO "$dmg" >/dev/null
echo "$dmg"
