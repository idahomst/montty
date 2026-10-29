# AGENTS.md

Guide for AI agents working on montty (Claude Code reads it through `CLAUDE.md`).

## What montty is

A tiling terminal emulator inspired by Tilix, in Rust: winit + glutin + glow (OpenGL 3.3 / GLES 3) for the window and
drawing, `alacritty_terminal` for the VT core and PTY, no GTK/Qt. Supported: Linux, X11 and Wayland. macOS and
Windows are not supported yet. Priority order: responsiveness > simplicity > features. Rust edition 2024.

## Commands

```bash
cargo build --release          # target/release/montty
cargo test                     # headless; tests that need an X display skip themselves
cargo clippy --all-targets     # must be warning-free
cargo fmt --check
```

## Architecture

Top-level modules (`src/`):

- `main.rs`: entry point. `--decode-image` runs the decoder helper with no event loop; otherwise parse args and run `App`.
- `cli.rs`: command-line parsing (`--profile`, `--session`, `-e`, `--help`).
- `app.rs`: the winit `ApplicationHandler`; owns all windows, routes events, computes the event loop's control flow.
- `windows.rs`: the collection of windows (`Windows<W>`) and the requests a window makes of the app (`AppRequest`: new window, open session).
- `window/`: one OS window (see below).
- `layout/`: pure data. `geom.rs` (rects, constants) and `tree.rs` (binary split tree: split, close, swap, resize, hit-test).
- `term/`: `pane.rs` (a pane = shell on a PTY + its `alacritty_terminal` grid, PTY reader thread, user events),
  `snapshot.rs` (copy of the visible cells taken under a brief lock), `colors.rs`, `links.rs`, `grid_point.rs`.
  Signals: `osc_scan.rs` (cheap scanner for OSC 7501 / 9 / 777 / 133 in the PTY byte stream, which alacritty's parser
  drops), `pty_tap.rs` (PTY reader wrapper that feeds the scanner and sends each hit as `UserEvent::Signal`),
  `status.rs` (OSC 7501 agent status parsing, per-pane records, display text; pure, input is untrusted).
- `render/`: the OpenGL renderer. `atlas.rs` (shelf-packed glyph atlas), `cells.rs` (per-cell instances),
  `rects.rs` (solid rects for chrome and cursor), `image.rs` (background image decode, fit math, quad), `shaders/`.
- `font/`: font discovery (fontdb), rasterization (swash), fallback, `fc-match` lookup of `monospace`.
- `ui/`: pure chrome state and geometry, no GL: top bar, menu, panel, confirm, find bar, text field, toast, overlay,
  scrollbar, icons (pixel rects), theme, text helpers.
- `input/`: key to xterm bytes (`keys.rs`), kitty keyboard protocol encoding (`kitty.rs`), key-binding parsing and the
  action table (`bindings.rs`), mouse reporting, paste.
- `notify.rs`: optional desktop notifications by spawning `notify-send` (rate-limited per pane).
- `config/`: `global.rs` (`config.toml`), `profile.rs`, `scheme.rs` (Tilix JSON, built-ins from `assets/schemes`),
  `store.rs` (all files loaded from one directory, last good version kept), `watch.rs` (inotify watcher + debounce).
- `session.rs`: saved-session JSON format, names, atomic writes.
- `clipboard.rs`: CLIPBOARD and PRIMARY (copypasta, X11 or Wayland, read with a timeout off the UI thread).
- `decoder.rs`: background image decoding in a child process (`montty --decode-image <path> <w> <h>`); release builds
  abort on panic, so a decoder crash or hang must only kill the child.
- `frame.rs`: refresh-capped frame scheduler. `device_events.rs`: turns off winit's raw device events.

`src/window/` files (a `Window` is split across them by concern):

- `mod.rs`: `Window` struct, event handling, frame scheduling, `next_deadline` / `on_timers`, config reload.
- `gl.rs`: OS window, GL context and surface creation. `icon.rs`: embedded window icon.
- `draw.rs`: frame drawing (bar, panes, chrome, overlay, size labels, link label, toast, modal on top).
- `actions.rs`: key-binding actions (tiling, tabs, scroll, zoom, fullscreen, clipboard, find).
- `tabs.rs`: pure tab collection; each tab owns a split tree.
- `bar.rs`: top bar drawing and clicks. `mouse.rs`: mouse routing (modal, toast, bar, find, overlay, dividers, selection).
- `selection.rs`: mouse selection. `links.rs`: Ctrl+click links. `find.rs`: scrollback search and find bar glue.
- `modal.rs`: menus, panels, prompts, close confirmation (at most one open; it takes all input).
- `pane_drag.rs`: drag-to-swap state machine. `sizes.rs`: size labels shown while resizing. `blink.rs`: cursor blink.
- `images.rs`: background image textures shared per image. `sessions.rs`: saving sessions and the session list.
- `attention.rs`: pure state of which panes need the user (blocked / error / done) and how urgently; no timers.
  `marks.rs`: attention dots on tabs and panes (placement, hover label).

Key flows:

- Events: winit event -> `App` -> the right `Window` -> modal, bar, find, overlay, divider or pane. Keys that match a
  binding run an action, otherwise they become bytes written to the PTY immediately, not deferred to a frame.
- Frame scheduling: the UI never polls. `Window::next_deadline` returns the earliest of every timer (frame, config
  debounce, autosave, toast, blink, bell, scrollbar, size labels); `Window::on_timers` runs the due ones; `App::about_to_wait`
  turns the minimum into `WaitUntil`, or `Wait` when there is none. A new timer must be added to both functions and must
  expire on its own: an idle window must not wake (0 % CPU).
- Rendering: The frame draw (`window/draw.rs`) takes a `Snapshot` of each visible pane under a brief lock, releases it, then does all GPU
  work: one instanced cell draw per pane plus a few for chrome and images. Glyphs go through the per-window atlas.
- Signals: the PTY reader's tap (`pty_tap.rs` + `osc_scan.rs`) sends `UserEvent::Signal` -> `App` -> the owning
  `Window` -> `Attention` (`window/attention.rs`) -> dots (`marks.rs`) and, if enabled, `notify.rs`. Status reports
  are stored per pane in `term/status.rs`. Marks have no timers; focusing a pane clears its "not seen yet" marks.
- Terminal: each pane owns an `alacritty_terminal` `Term` and a PTY reader thread that wakes the UI through a
  `UserEvent`. The UI thread never blocks on PTY I/O.
- Config: `config::store::Store` loads `config.toml`, `profiles/*.toml` and `schemes/*.json`; the watcher debounces file events
  into one reload; panes share their profile, so a reload restyles them live. A bad file keeps its last good version
  and shows a toast.
- Sessions: a window serializes to a `SessionDoc` (tabs, split trees, per-pane profile and cwd); `restore_last_session`
  autosaves to `sessions/_last.json`.
- Images: decoded by a child process, uploaded once per window and shared by panes using the same image.

## Conventions

- Match the surrounding code's comment density and idiom. Comments explain why, not what.
- Keep logic pure and testable next to the code, with `#[cfg(test)]` tests; keep GL and window code thin. Integration
  tests are in `tests/`. Tests must never touch the real `~/.config/montty` (use a temp dir).
- No new crates without a strong reason; the dependency tree is deliberately small and each direct dependency is
  commented in `Cargo.toml`. Prefer pure-Rust crates.
- Performance gates in `docs/bench.md` (`cat` throughput, key latency, idle CPU 0 %, RSS, binary size) must not
  regress. For a hot-path change, re-measure with `scripts/bench.sh` and update the baseline in `docs/bench.md` if it moves.
- When adding an action or option, update `docs/config.md` (bindings table and examples) and, for user-visible
  behaviour, `docs/design.md`.

## Testing GUI changes safely

- Never run GUI tests or benchmarks on the developer's live display without asking.
- Use the nested X server helpers in `scripts/gui-test/` (needs Xephyr; screenshots need xwd and ffmpeg):
  ```bash
  cargo build --release
  scripts/gui-test/session.sh start                  # Xephyr on :5 + montty with a fresh, empty XDG_CONFIG_HOME
  scripts/gui-test/xtest.py focus key ctrl+alt+r type 'ls\n'   # XTest keys and pointer; always :5, never $DISPLAY
  scripts/gui-test/session.sh shot split             # prints the PNG path; look at it
  scripts/gui-test/session.sh stop                   # stops both by recorded PID
  ```
  `xtest.py --help` lists the commands (keys, typing, clicks, drags, wheel, resize). There is no window manager on
  `:5`, so run `focus` before sending keys.
- The real `~/.config/montty` must never be read or written; the helper uses its own temp config.
- Kill test processes by PID, never by name.

## Packaging and release

- Pushing a tag `v*` runs `.github/workflows/release.yml`: it tests, builds the release binary, then builds the `.deb`
  (`packaging/deb/build-deb.sh <version> <binary> <out-dir>`), a tarball, and an Arch package (`packaging/arch/PKGBUILD`
  in an Arch container), and attaches them to a GitHub release.
- `assets/`: `montty.desktop`, `icon/hicolor/<N>x<N>/apps/montty.png`, and the built-in Tilix schemes in `schemes/`.
- Keep the version in `Cargo.toml` and `packaging/arch/PKGBUILD` in step with the tag.

## Docs

- `docs/design.md`: architecture and design rationale.
- `docs/config.md`: user configuration and key bindings (the reference).
- `docs/install.md`: installing from packages and from a source build.
- `docs/bench.md`: benchmark procedure and measurements.
