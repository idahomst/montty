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

# iconutil wants name@2x pairs; there is no 1024 px source, so no icon_512x512@2x.
iconset=$stage/montty.iconset
mkdir -p "$iconset"
png() { echo "assets/icon/hicolor/$1x$1/apps/montty.png"; }
cp "$(png 16)" "$iconset/icon_16x16.png"
cp "$(png 32)" "$iconset/icon_16x16@2x.png"
cp "$(png 32)" "$iconset/icon_32x32.png"
cp "$(png 64)" "$iconset/icon_32x32@2x.png"
cp "$(png 128)" "$iconset/icon_128x128.png"
cp "$(png 256)" "$iconset/icon_128x128@2x.png"
cp "$(png 256)" "$iconset/icon_256x256.png"
cp "$(png 512)" "$iconset/icon_256x256@2x.png"
cp "$(png 512)" "$iconset/icon_512x512.png"
iconutil -c icns -o "$app/Contents/Resources/montty.icns" "$iconset"

# Ad-hoc: no Developer ID yet (see docs/install.md for the first launch).
codesign --force --sign - "$app"

ln -s /Applications "$stage/dmg/Applications"
mkdir -p "$out_dir"
dmg=$out_dir/montty-$version-macos.dmg
hdiutil create -volname montty -srcfolder "$stage/dmg" -ov -format UDZO "$dmg" >/dev/null
echo "$dmg"
