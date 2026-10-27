#!/usr/bin/env bash
# Builds montty_<version>_amd64.deb from an already built release binary.
# Run from the repo root: packaging/deb/build-deb.sh <version> <path-to-binary> <out-dir>
set -euo pipefail

if [ "$#" -ne 3 ]; then
    echo "usage: $0 <version> <path-to-binary> <out-dir>" >&2
    exit 2
fi
version=$1
binary=$2
out_dir=$3

[ -f "$binary" ] || { echo "no such binary: $binary" >&2; exit 1; }
[ -f assets/montty.desktop ] || { echo "run from the repo root" >&2; exit 1; }

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
root=$stage/montty_${version}_amd64

install -Dm755 "$binary" "$root/usr/bin/montty"
install -Dm644 assets/montty.desktop "$root/usr/share/applications/montty.desktop"
for icon in assets/icon/hicolor/*/apps/montty.png; do
    size=$(basename "$(dirname "$(dirname "$icon")")")
    install -Dm644 "$icon" "$root/usr/share/icons/hicolor/$size/apps/montty.png"
done
install -Dm644 README.md "$root/usr/share/doc/montty/README.md"

mkdir -p "$root/usr/share/doc/montty"
{
    cat <<'COPYRIGHT'
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: montty
Upstream-Contact: Martin Stepanek <38214485+idahomst@users.noreply.github.com>
Source: https://github.com/idahomst/montty

Files: *
Copyright: 2026 Martin Stepanek
License: MIT

Files: assets/schemes/*
Copyright: Tilix authors
License: MPL-2.0
Comment: Color schemes copied from Tilix (https://github.com/gnunn1/tilix).
 The Mozilla Public License 2.0 is at https://www.mozilla.org/MPL/2.0/

License: MIT
COPYRIGHT
    # The license text, indented as a Debian copyright continuation.
    sed -e '1,2d' -e 's/^$/./' -e 's/^/ /' LICENSE
} > "$root/usr/share/doc/montty/copyright"
chmod 644 "$root/usr/share/doc/montty/copyright"

installed_size=$(du -sk "$root" | cut -f1)

mkdir -p "$root/DEBIAN"
cat > "$root/DEBIAN/control" <<CONTROL
Package: montty
Version: $version
Section: x11
Priority: optional
Architecture: amd64
Maintainer: Martin Stepanek <38214485+idahomst@users.noreply.github.com>
Homepage: https://github.com/idahomst/montty
Installed-Size: $installed_size
Depends: libc6, libgcc-s1, libgl1, libegl1, libx11-6, libx11-xcb1, libxcursor1, libxi6, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libwayland-cursor0, libwayland-egl1
Recommends: fonts-dejavu-core
Provides: x-terminal-emulator
Description: tiling terminal emulator inspired by Tilix, without GTK
 montty splits a window into tiled panes, with tabs, profiles, saved
 sessions, find in the scrollback, clickable links and background images.
 It draws with OpenGL, runs on X11 and Wayland, and needs neither GTK nor
 Qt. Profiles and key bindings are plain files that apply live.
CONTROL

mkdir -p "$out_dir"
dpkg-deb --build --root-owner-group "$root" "$out_dir/montty_${version}_amd64.deb"
