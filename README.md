# montty

A tiling terminal emulator for Linux, inspired by [Tilix](https://gnunn1.github.io/tilix-web/) but written in Rust
with no GTK or Qt. It keeps what makes Tilix pleasant (splitting panes, tabs, profiles, saved sessions) and aims to be
small, fast and idle-quiet: it is drawn with OpenGL, runs on X11 and Wayland, and uses no CPU while nothing happens.

## Features

- **Splits:** split right, down, left or up to any depth; drag dividers or use the keyboard to resize; maximize a
  pane; drag a pane's grip onto another pane to swap them.
- **Tabs and windows:** each tab has its own split tree; several windows in one process.
- **Profiles:** named settings (font, colors, cursor, scrollback, shell, opacity, padding). A pane can switch profile
  from its hover overlay, and editing a profile or `config.toml` applies live, without a restart.
- **Color schemes:** Tilix's JSON format, nine schemes built in.
- **Background images:** PNG or JPEG per profile, with fit modes and opacity, plus window transparency.
- **Sessions:** save and reopen a window's tabs, splits, profiles and working directories.
- **Find, links, clipboard:** search the scrollback, Ctrl+click URLs and OSC 8 links, CLIPBOARD and PRIMARY
  selection, bracketed paste, mouse reporting.
- **Tilix key bindings** by default, every action rebindable in `config.toml`.
- Pane size labels while resizing (and on demand with `Ctrl+Alt+S`), a visual bell, color emoji and CJK.

macOS and Windows are not supported yet.

## Install

Download a package from the [latest release](https://github.com/idahomst/montty/releases/latest).

- Debian / Ubuntu: `sudo apt install ./montty_0.1.0_amd64.deb`
- Arch / CachyOS: `sudo pacman -U montty-0.1.0-1-x86_64.pkg.tar.zst`
- Any Linux (x86_64): unpack `montty-0.1.0-x86_64-linux.tar.gz` and copy the files as described in
  [docs/install.md](docs/install.md).

### Build from source

```bash
cargo build --release      # binary: target/release/montty
```

You need a stable Rust toolchain, 1.85 or newer (the crate uses the 2024 edition), and a C compiler for a few build
scripts. X11, Wayland and OpenGL libraries are loaded at run time, so no `-dev` packages are required. At run time
you need OpenGL 3.3 (or GLES 3), `libxkbcommon`, and a monospace font such as DejaVu Sans Mono.
See [docs/install.md](docs/install.md) for installing the result by hand.

## Configuration

Settings live in plain files under `~/.config/montty/` (`config.toml`, `profiles/*.toml`, `schemes/*.json`). With no
files montty behaves like a stock Tilix-style terminal. The full reference, including every key binding, is in
[docs/config.md](docs/config.md).

Other docs: [docs/design.md](docs/design.md) (how montty is built and why),
[docs/bench.md](docs/bench.md) (performance measurements and the benchmark procedure).

## Why montty?

Named after Monty, the author's dog. The app icon is his portrait, drawn by the author.

## License

MIT, see [LICENSE](LICENSE). The built-in color schemes in `assets/schemes/` come from Tilix and are under the MPL-2.0
(see `assets/schemes/LICENSE-NOTICE`).
