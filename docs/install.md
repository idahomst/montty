# Installing montty

## From a release package

Packages are attached to each [GitHub release](https://github.com/idahomst/montty/releases). They are for x86_64.

**Debian, Ubuntu and derivatives**

```bash
sudo apt install ./montty_0.1.0_amd64.deb
```

**Arch, CachyOS and derivatives**

```bash
sudo pacman -U montty-0.1.0-1-x86_64.pkg.tar.zst
```

**Tarball (any Linux)**

```bash
tar xzf montty-0.1.0-x86_64-linux.tar.gz
cd montty-0.1.0
mkdir -p ~/.local/bin ~/.local/share/icons ~/.local/share/applications
cp bin/montty ~/.local/bin/
cp -r assets/icon/hicolor ~/.local/share/icons/
cp assets/montty.desktop ~/.local/share/applications/
```

The tarball holds `bin/montty`, `assets/icon/hicolor`, `assets/montty.desktop`, `LICENSE`, `README.md` and
`docs/install.md`. For a system-wide install copy the same files under `/usr/local/bin`, `/usr/local/share/icons` and
`/usr/local/share/applications` instead.

## From a source build

Needs Rust 1.85 or newer. From the repo root, after `cargo build --release`:

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
