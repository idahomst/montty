# Installing montty

## From a release package

Packages are attached to each [GitHub release](https://github.com/idahomst/montty/releases/latest). The Linux packages
are for x86_64; `<version>` below is the release's version.

**Debian, Ubuntu and derivatives**

```bash
sudo apt install ./montty_<version>_amd64.deb
```

**Arch, CachyOS and derivatives**

```bash
sudo pacman -U montty-<version>-1-x86_64.pkg.tar.zst
```

**Tarball (any Linux)**

```bash
tar xzf montty-<version>-x86_64-linux.tar.gz
cd montty-<version>
mkdir -p ~/.local/bin ~/.local/share/icons ~/.local/share/applications
cp bin/montty ~/.local/bin/
cp -r assets/icon/hicolor ~/.local/share/icons/
cp assets/montty.desktop ~/.local/share/applications/
```

The tarball holds `bin/montty`, `assets/icon/hicolor`, `assets/montty.desktop`, `LICENSE`, `LICENSE-SCHEMES`,
`README.md`, `docs/install.md` and `docs/config.md` (the configuration reference). For a system-wide install copy the same files under `/usr/local/bin`, `/usr/local/share/icons` and
`/usr/local/share/applications` instead.

## macOS

Download `montty-<version>-macos.dmg` from the [latest release](https://github.com/idahomst/montty/releases/latest),
open it and drag `montty.app` to Applications. It runs on Apple Silicon and Intel Macs with macOS 11 or newer.

The app is not signed with an Apple Developer ID yet, so the first launch is blocked: right-click `montty.app` in
Applications, choose **Open**, then **Open** again. Or clear the download flag once:

    xattr -dr com.apple.quarantine /Applications/montty.app

From source: `cargo build --release`, then `packaging/macos/build-app.sh <version> target/release/montty <out-dir>`
builds the same `.dmg`, or run `target/release/montty` directly.

## From a source build

Needs Rust 1.88 or newer (on macOS the Xcode Command Line Tools are enough, and the same `cargo build --release`
works). From the repo root, after `cargo build --release`:

```bash
mkdir -p ~/.local/bin ~/.local/share/icons ~/.local/share/applications
cp target/release/montty ~/.local/bin/
cp -r assets/icon/hicolor ~/.local/share/icons/
cp assets/montty.desktop ~/.local/share/applications/
```

## Notes

- `~/.local/bin` must be on `PATH`, because the desktop file uses `Exec=montty`.
- KDE may need `kbuildsycoca6`, or a log out and in, to pick up the icon.
- Wayland compositors take the icon from the `.desktop` file (matched by app_id `montty`), not from the window.
- montty needs OpenGL 3.3 (or GLES 3) and `libxkbcommon` at run time, and uses `fc-match` (fontconfig's command, if
  installed) to find the default monospace font. The packages above depend on or recommend what is needed.
