# montty design

montty is a tiling terminal emulator in the spirit of [Tilix](https://gnunn1.github.io/tilix-web/), written in Rust,
with no GTK or Qt dependency. This document describes how it works and why. User-facing settings and key bindings are
in [config.md](config.md), measurements in [bench.md](bench.md).

## 1. Intent

Tilix's way of working (splitting panes, icons in the top bar, profiles) without GTK/Qt, and small, fast and light on
memory. Priority order: **responsiveness > simplicity > features**.

## 2. Goals

- Tiling: split the focused pane right, down, left or up, to any depth; resize by dragging dividers or by keyboard.
  Swap two panes by dragging.
- Tabs: each tab holds its own split tree.
- Profiles: named settings bundles. Each pane uses one profile; editing a profile file updates every pane using it
  **live**. A running pane can switch profile from its overlay.
- Color schemes, per-profile background image, window transparency, font choice, scrollback length, shell or command.
- Save and load sessions (tab layout, per-pane profile and working directory).
- Find in the scrollback, clickable URLs (Ctrl+click), mouse selection, clipboard and primary selection.
- Tilix's default shortcuts, all rebindable in `config.toml`.
- Linux, X11 and Wayland.

## 3. Non-goals

- Quake / drop-down mode, bookmarks, triggers, password manager, sidebar, sync input to several panes, badges,
  silence notifications, encodings other than UTF-8.
- A preferences GUI: settings are files only.
- Restoring running processes or scrollback when loading a session.
- Inline images (sixel, kitty graphics) and font ligatures.
- Moving a pane to another tab or window.
- macOS and Windows (the design does not preclude a macOS port).

## 4. Anti-goals

Failures even if everything else works:

- Slow typing or laggy rendering. Every design choice yields to this.
- One busy pane (a `cat` of a huge log) making other panes or typing stutter.
- Burning CPU or GPU while idle.
- Pulling in GTK, Qt, or a large GUI toolkit or web runtime.

## 5. Constraints and targets

- Rust, stable (edition 2024, so 1.85 or newer).
- No GTK/Qt. Runtime libraries on Linux: libxkbcommon, libwayland-client / libX11 (both loaded at run time by winit),
  libEGL / libGL. No fontconfig library and no freetype: the font stack is pure Rust.
- A small dependency tree of mostly pure-Rust crates; each direct dependency is justified in `Cargo.toml`.
- Release builds use fat LTO, `codegen-units = 1`, `panic = "abort"`, strip, `opt-level = 3`.
- Gates (they guard the anti-goal): key-to-screen latency within one frame of the reference terminal; `cat` of a
  100 MB file in at most about 1.5x Alacritty's time with other panes responsive meanwhile; idle CPU at 0 % with no
  redraws. Targets measured and reported: 10 idle panes under about 100 MB RSS, stripped binary under about 8 MB.
  The procedure is `scripts/bench.sh`, documented with results in [bench.md](bench.md).

## 6. Technical approach

| Area | Decision |
|---|---|
| Terminal core | `alacritty_terminal`, pinned to one exact version: VT parsing, grid, scrollback and PTY spawn. montty does not reimplement VT parsing. Small layers on top: `tcgetpgrp` on the PTY fd to tell whether a pane runs a foreground process, `/proc/<pid>/cwd` for a pane's working directory, URL detection by regex on the visible grid. |
| Windowing and input | `winit`. Windows are created **without system decorations**; montty draws its own top bar on every backend, so the look is the same everywhere and GNOME Wayland (no server-side decorations) works. Moving uses `drag_window()`, resizing `drag_resize_window()` with an edge hit zone, so compositor features (snapping, maximize) keep working. |
| Rendering | OpenGL 3.3 core, falling back to GLES 3, via `glutin` and `glow`. `wgpu` was rejected for binary size, CPU rendering for the cost of background images and transparency at 4K. |
| Fonts | `fontdb` for discovery, `swash` for rasterization including color emoji. The default family `monospace` is resolved by running `fc-match` when it exists (no linking to fontconfig), else the first installed of DejaVu Sans Mono, Noto Sans Mono, Liberation Mono, else any monospace face. A missing glyph is looked up by coverage across installed fonts and cached. Color emoji, CJK double-width cells and combining marks are supported; complex shaping and ligatures are not (plain codepoint-to-glyph mapping). |
| Split layout | A hand-written binary tree: a split node is an orientation, a ratio and two children; a leaf is a pane. A layout library would be overkill. |
| Config formats | TOML for `config.toml` and profiles, Tilix's JSON scheme format for schemes (Tilix schemes drop in), JSON for sessions. |
| File watching | `notify` (inotify on Linux) for live reload. |
| Clipboard | `copypasta` on X11 and on Wayland (using the window's own connection, so it works on GNOME, which lacks the data-control protocol). |

## 7. Window and UI

```
┌──────────────────────────────────────────────────────────────────────┐
│ [build] [montty ✕] [servers] [+]          [⇥▯] [⬓] │ [🔍] [≡]  _ □ ✕ │  top bar (drawn by montty)
├───────────────────────────────┬──────────────────────────────────────┤
│ $ cargo build  [⠿ Default▾ ⛶ ✕]│ htop (dimmed)                        │  overlay on hover
│ ...  (focused: colored border) │                                      │
│                               ├──────────────────────────────────────┤  1 px gray dividers
│                               │ ssh prod-01 (dimmed)                 │
└───────────────────────────────┴──────────────────────────────────────┘
```

**Top bar**
- Left: one tab per session (label and ✕), then `+` for a new tab. The active tab has a 3 px underline in the
  focus-border color along its bottom edge; nothing else about the tab changes.
- Right: split right, split down, find, the ≡ menu, and the window buttons (minimize, maximize, close). With Alt held,
  the split buttons split left and up.
- Dragging empty bar space moves the window; double-clicking it toggles maximize.
- A tab's label is the focused pane's title (OSC 0/2) unless the tab was renamed (double-click it).
- Icons are drawn by montty as antialiased strokes (2 px at scale 1, round caps, Lucide-like shapes on a 24-unit
  grid, straight edges snapped to whole pixels); there is no icon theme lookup and no SVG library.

**Panes**
- No per-pane title bars. Panes are separated by 1 px gray dividers with a drag zone a few pixels wider; the pointer
  changes over it. Double-clicking a divider equalizes the neighbouring panes.
- The focused pane has a colored border; unfocused panes are slightly dimmed. Both are global settings.
- A hover overlay in the top-right corner of the hovered pane holds a grip, `profile ▾` (switch profile live),
  maximize/restore and close. It hides while the pointer is outside and while typing in that pane.
- Maximizing makes one pane fill the tab; restoring returns the previous layout.
- A new split inherits the focused pane's profile and working directory, and goes before or after the focused pane
  depending on the direction (left/up or right/down). Splitting is refused when the pane is too small.
- **Swapping panes:** press on the overlay and drag past a small threshold onto another pane of the same tab; that pane
  is highlighted as the target. On drop the two panes trade slots: the split tree keeps its shape, ratios and
  dividers, each terminal is resized to its new slot (a PTY resize, not a restart), and focus follows the dragged
  pane. Escape, losing window focus, or dropping anywhere else cancels. A click that doesn't move, or a press-and-hold,
  never starts a drag and still fires the button under it. No drag while a pane is maximized.
- **Size labels:** while a divider is dragged, the window is resized or a pane is resized from the keyboard, each pane
  whose grid size changed shows `columns × rows` centred in a small framed box. They disappear one second after the
  last change. Splits, pane maximize, font zoom, startup and window-manager placement of a new window never show
  them. `show_sizes` (`Ctrl+Alt+S`) shows every visible pane's size for two seconds, and hides them if pressed
  again. The hide is a single timer wake-up, so an idle window stays idle.
- Closing the last pane closes the tab, and closing the last tab closes the window.
- Closing a pane or window whose shell has a running foreground child asks for confirmation in a small in-window prompt.
  Closing a window (or its last tab) that holds more than one pane in total asks once as well
  (`confirm_close_window`); the running-process prompt takes precedence, never both.

**Menu (≡):** New tab, New window, Save session…, Open session ▸, Edit config, Open config folder, Reload config,
Keyboard shortcuts, About.

**Find bar:** a small text field with next/previous and a case toggle, at the top right of the focused pane.

**Own widgets:** tab strip, icon button, dropdown menu, single-line text field (find, tab rename, session name), the
confirm prompt, info panels and a transient error toast. There is no general widget toolkit; their state and geometry
are pure data, tested without a window.

**Multiple windows:** one process, several winit windows (≡ menu or `Ctrl+Shift+N`). Each window has its own GL
context and glyph atlas, which avoids context sharing.

**Application identity:** the app id / `WM_CLASS` is `montty`. The window icon (the author's portrait of Monty, the dog
the project is named after) is embedded at 256 px for X11; on Wayland the compositor takes the icon from the
`.desktop` file matched by app id (`assets/montty.desktop`, icons in `assets/icon/hicolor`).

## 8. Profiles, config, live reload

Files in `$XDG_CONFIG_HOME/montty/` (default `~/.config/montty/`):

```
config.toml             global: default profile, keybindings, focus border color, dim amount, divider color,
                        restore_last_session, pane_drag, confirm_close_window, visual_bell
profiles/<name>.toml    one profile per file; the file stem is the profile's identity
schemes/<name>.json     color schemes in Tilix format (built-in schemes are embedded in the binary)
sessions/<name>.json    saved sessions
```

**Profile fields:** display name; font family and size; color scheme and explicit color overrides; bold-is-bright;
background image path, fit mode (stretch, fill, center, tile) and opacity; window opacity; cursor shape and blink;
scrollback lines; shell, login-shell flag or custom command; padding.

**Default profile:** written on first run if `profiles/` has no profile. `monospace` 11 pt, the Tango palette,
no background image, opaque, 10 000 lines of scrollback, command `$SHELL` (else `/bin/sh`), not a login shell.

**Color schemes:** every Tilix scheme file parses. montty uses the name, foreground and background, the 16-color
palette, and the cursor and highlight colors, and ignores other fields. Nine Tilix schemes are embedded (a few KB).

**Background image:** per profile, drawn behind each pane that uses it, fitted to the pane by the profile's fit mode.
It is decoded once per profile, in a child process (`montty --decode-image`, so a decoder crash or hang cannot take
the terminal down; release builds abort on panic), and uploaded once per window as a texture shared by all panes
using it. Images larger than the biggest monitor are downscaled at load. It is reloaded when the profile or the image
file changes, and dropped when no pane shows it.

**Live reload**
- Panes share a reference to their profile rather than copying it.
- On a file change montty re-parses; if valid, it swaps the new profile in and redraws every pane using it. A font
  change recomputes the cell size, reflows the grid and sends the new size to the PTY. Scrollback length changes
  apply to existing panes.
- An invalid file keeps its last good version and shows a toast naming the file and line.
- `config.toml`, including key bindings, reloads the same way.
- A session or pane naming a profile that no longer exists falls back to the default profile.

**Startup:** one window, one tab, one pane with the default profile. Command line: `--session <name>`,
`--profile <name>`, `-e <command>`.

## 9. Sessions

- Save: a name prompt in the window writes `sessions/<name>.json` (atomically). There is no file chooser.
- Content: window size; tabs; per tab its split tree, with split nodes (orientation, ratio, two children) and leaves
  (profile name, working directory, optional command, optional tab title). The format is montty's own; it is not
  compatible with Tilix sessions.
- Load: opens into the current window, replacing it if it holds only one fresh pane (a single pane with no foreground
  child that has not received keyboard input), else into a new window. The window size is best-effort, since Wayland
  compositors may ignore it.
- With `restore_last_session` on, the layout is autosaved to `sessions/_last.json` (debounced on every layout change,
  including a pane swap, and on clean exit), and loaded at startup instead of the single default pane.

## 10. Keyboard shortcuts

Tilix's defaults, rebindable in `config.toml` with plain combo strings such as `split_right = "Ctrl+Alt+R"`
(`"none"` unbinds). A montty binding wins over the terminal application; unbind it to pass the key through. The
complete list of actions and defaults is in [config.md](config.md#actions-and-their-default-keys). Splitting left and
up default to the right/down keys plus Shift (`Ctrl+Alt+Shift+R`, `Ctrl+Alt+Shift+D`).

## 11. Terminal behaviour

- Whatever `alacritty_terminal` supports (truecolor, bracketed paste, mouse reporting, alt screen, OSC 8 hyperlinks,
  OSC 52 writes) is exposed; montty adds no emulation of its own.
- Mouse: drag selects, double-click selects a word, triple-click a line, Alt+drag a block, Shift bypasses a program's
  mouse reporting, selecting copies to PRIMARY, middle click pastes it, the wheel scrolls (alternate scroll on the alt
  screen).
- Ctrl+click on a URL or OSC 8 link opens it with `xdg-open`.
- Kitty keyboard protocol (`kitty_keyboard`, default on): `alacritty_terminal` tracks and answers the flags;
  `input/kitty.rs` encodes keys for all five, so Shift+Enter and other modified keys reach agents distinctly (Claude
  Code >= 2.1.269 uses it). With no flags pushed keys encode as before. Releases and repeats reach
  the PTY only when the flags ask; bindings win, and the release of a key a binding consumed is dropped.
- Agent status and attention marks: people run coding agents (Claude Code and others) in several panes and tabs and
  look elsewhere meanwhile, so montty shows which pane needs them, quietly. `alacritty_terminal`'s parser drops OSC
  sequences it doesn't know, so the PTY reader passes every read to a small scanner (one `memchr` for ESC outside a
  sequence) and the bytes reach the parser unchanged. The scanner recognises OSC 7501 (the Program Status Protocol:
  `working`, `blocked`, `done`, `error`, `idle`, per-pane records; montty answers its feature query, and Claude Code
  >= 2.1.295 reports this way), OSC 9 and OSC 777 `notify` (one-shot notifications), and OSC 133 `A` (a new prompt,
  which ends a blocked state). Content is untrusted: limits are checked before any record changes and text is
  sanitised before display.
- A pane that needs the user while they are not looking at it (not the focused pane of the active tab in a focused
  window) gets a dot: amber (blocked, notification or bell), red (error) or green (done), on its tab before the title
  and in the pane's top-right corner when it is visible; a tab shows the most urgent of its panes. Hovering shows the
  reason. A blocked dot follows the agent's state and goes away when it moves on; the others stay until the pane is
  looked at. Nothing is shown while an agent is working. The state is pure data with no timers, so an idle window
  still wakes for nothing. `attention_marks = false` hides the dots.
- Desktop notifications (`desktop_notifications`, default off) additionally run `notify-send` when a pane turns amber
  from OSC 7501, OSC 9 or OSC 777, at most once per pane per 10 seconds. It uses the command, not a crate, to keep
  the dependency tree small; whether it pops up or sounds is the desktop's choice.
- Bell: a visual flash, plus the window's urgency hint when it is not focused.
- Scrollbar: a thin indicator shown while scrolled back.

## 12. Performance architecture

- Each pane's PTY is read on its own thread (`alacritty_terminal`'s event loop). The UI thread never blocks on PTY I/O.
- Key presses are written to the PTY immediately on the input event, not deferred to a frame.
- Damage-driven redraw: nothing is drawn when nothing changed. Under heavy output, redraws coalesce to at most one per
  display refresh, drawing the latest state.
- The UI thread holds a pane's grid lock only long enough to copy out the visible cells; all GPU work happens after
  the lock is released, so the PTY thread is never stalled by rendering.
- An idle window has no timers: the event loop sleeps until the next deadline any window reports, and no deadline
  means it sleeps indefinitely. Cursor blink stops 10 s after the last key or output, and winit's raw device events
  are switched off, since they would wake the loop on every mouse movement anywhere on screen.
- Glyphs are rasterized once into an atlas; cells are drawn with instanced rendering, one draw per pane plus a few for
  chrome, background image and overlay.
- Scrollback memory is bounded by the per-profile setting.
- Clipboard reads on Wayland run off the UI thread with a short timeout, so a stuck selection owner cannot freeze a
  window.
