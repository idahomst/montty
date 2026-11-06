# montty

A tiling terminal emulator for Linux, inspired by [Tilix](https://gnunn1.github.io/tilix-web/) but written in Rust
with no GTK or Qt. It keeps what makes Tilix pleasant (splitting panes, tabs, profiles, saved sessions) and aims to be
small, fast and idle-quiet: it is drawn with OpenGL, runs on X11 and Wayland, and uses no CPU while nothing happens.

![montty with three panes: a file tree, a shell with the pane buttons shown, and Claude Code](docs/screenshot.png)

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

Linux (X11, Wayland) and macOS 11+. Windows is not supported yet.

## Install

Download a package from the [latest release](https://github.com/idahomst/montty/releases/latest).

- Debian / Ubuntu: `sudo apt install ./montty_<version>_amd64.deb`
- Arch / CachyOS: `sudo pacman -U montty-<version>-1-x86_64.pkg.tar.zst`
- Any Linux (x86_64): unpack `montty-<version>-x86_64-linux.tar.gz` and copy the files as described in
  [docs/install.md](docs/install.md).
- macOS: open `montty-<version>-macos.dmg` and drag montty.app to Applications; see
  [docs/install.md](docs/install.md#macos) for the first launch.

### Build from source

```bash
cargo build --release      # binary: target/release/montty
```

You need a stable Rust toolchain, 1.88 or newer, and a C compiler for a few build
scripts. X11, Wayland and OpenGL libraries are loaded at run time, so no `-dev` packages are required. At run time
you need OpenGL 3.3 (or GLES 3), `libxkbcommon`, and a monospace font such as DejaVu Sans Mono.
See [docs/install.md](docs/install.md) for installing the result by hand.

## Configuration

Settings live in plain files under `~/.config/montty/`. There is no settings window: edit the files, and montty applies
every change as soon as you save, with no restart. A file with a mistake keeps its last good version and a toast
names the file and line.

```
~/.config/montty/
  config.toml           window-wide settings and key bindings (optional)
  profiles/<name>.toml  one file per profile
  schemes/<name>.json   extra color schemes in Tilix's JSON format (optional)
  sessions/<name>.json  saved sessions
```

The ≡ menu has `Edit config` (opens `config.toml` in `$VISUAL`/`$EDITOR` in a new tab), `Open config folder` and
`Keyboard shortcuts`. The full reference, including every key binding, is in [docs/config.md](docs/config.md).

### Profiles

A profile is how a pane looks and what it runs (font, color scheme, cursor, opacity, padding, scrollback, shell,
background image), one file per profile in `profiles/`. On the first run montty writes `profiles/default.toml` with
every setting and a comment on each. Hover a pane and click **Profile** in its overlay to switch that pane to another
profile, edit its profile, or create a new one. See [Profiles](docs/config.md#profiles),
[Choosing a profile for a pane](docs/config.md#choosing-a-profile-for-a-pane) and
[Color schemes](docs/config.md#color-schemes).

### Sessions

A session is a window's layout: its tabs, splits, and each pane's profile and working directory (shells start fresh,
scrollback isn't saved). Save one with `Ctrl+Shift+S`, open one with `Ctrl+Shift+O` or `montty --session NAME`, and
set `restore_last_session = true` to reopen your last window at the next start. See
[Sessions and windows](docs/config.md#sessions-and-windows).

Other docs: [docs/design.md](docs/design.md) (how montty is built and why),
[docs/bench.md](docs/bench.md) (performance measurements and the benchmark procedure).

## Ideas under consideration

Not built yet; reactions in GitHub issues help decide.

- **`montty --config-check`:** check the config files on purpose (valid TOML, known settings, montty's own key
  conflicts) and also warn about shortcuts the desktop takes before montty sees them, such as Cinnamon's screen
  recording on `Ctrl+Alt+Shift+R` or moving windows with Alt+click. Linux has no common interface for asking which
  shortcuts the desktop holds, and Wayland offers none at all, so each desktop (Cinnamon, GNOME, KDE, Xfce, ...)
  would need its own reader, and some (tiling compositors) could not be checked.

## Why montty?

Named after Monty, the author's dog. The app icon is his portrait, drawn by the author.

## License

MIT, see [LICENSE](LICENSE). The built-in color schemes in `assets/schemes/` come from Tilix and are under the MPL-2.0
(see `assets/schemes/LICENSE-NOTICE`).
