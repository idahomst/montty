# macOS release: design

Date: 2026-11-05. Status: draft for review.

## Goal

A macOS build that other people can download and use: Mac-style shortcuts, correct text size, sensible behaviour
when started from Finder, notifications, and a `montty.app` in the GitHub release. Linux behaviour does not change,
except for the new optional `quit` action and the shell fallback in item 4.

Today montty builds and passes its tests on macOS (CI runs both OSes), runs from `cargo run`, resizes natively,
starts a login shell, and opens files with `open`.

**Out of scope:**

- a native menu bar beyond the app menu winit already provides
- macOS-native window tabs
- code signing with a Developer ID, and notarization (a later change; nothing here blocks it)
- a Homebrew cask (a follow-up once a `.dmg` exists)
- the divider-drag issue seen once on the first run (only if it comes back)

## Approach

There are six small PRs on `main`, in the order below, each switched on per OS where needed
(`cfg(target_os = "macos")` next to the code it replaces, as AGENTS.md describes). CI checks Linux and macOS on every
PR. Each PR also gets a short manual check on a Mac. The release job comes last, so `main` stays releasable for Linux
throughout.

## 1. Mac-style shortcuts and quitting

**Default bindings on macOS** replace Ctrl+Shift with Cmd (`Super` in montty's key syntax), following Terminal.app and
iTerm2 where they agree. Ctrl stays free for the shell. Linux defaults are unchanged. On macOS, key names show as
`Cmd` and `Option` (and parse that way too); the `docs/config.md` table gets a Linux column and a macOS column.

| Action | macOS default |
|---|---|
| copy / paste / select_all | Cmd+C / Cmd+V / Cmd+A |
| find / find_next / find_prev | Cmd+F / Cmd+G / Cmd+Shift+G |
| new_tab / new_window | Cmd+T / Cmd+N |
| close_pane / close_tab | Cmd+W / Cmd+Shift+W |
| quit (new) | Cmd+Q, through the app menu (below) |
| next_tab / prev_tab | Cmd+Shift+] / Cmd+Shift+[ |
| go_to_tab_N | Cmd+1 … Cmd+9 (tab 10 unbound, as in Terminal.app and iTerm2) |
| move_tab_left / move_tab_right | Cmd+Shift+Left / Cmd+Shift+Right |
| split_right / split_down | Cmd+D / Cmd+Shift+D |
| split_left / split_up / split_auto | Cmd+Alt+D / Cmd+Alt+Shift+D / Cmd+Shift+A |
| focus_up/down/left/right | Cmd+Alt+Arrows |
| focus_pane_N | Cmd+Alt+1 … Cmd+Alt+0 |
| next_pane / prev_pane | Cmd+] / Cmd+[ |
| resize_up/down/left/right | Cmd+Ctrl+Arrows |
| maximize_pane | Cmd+Shift+Enter |
| save_session / open_session | Cmd+S / Cmd+O |
| zoom_in / zoom_out / zoom_reset | Cmd+= and Cmd+plus / Cmd+minus / Cmd+0 |
| fullscreen | Cmd+Ctrl+F |
| show_sizes | Cmd+Alt+S |
| scroll keys | as on Linux (Shift+PageUp/PageDown; Fn+Up/Down on a Mac keyboard), plus Cmd+Up / Cmd+Down for one line |
| paste_primary | unbound (Macs have no Insert key); middle click still pastes |

**Links:** Ctrl+click opens a link on Linux. On macOS it becomes Cmd+click, since Ctrl+click is a right click there.
The hover hint follows the same modifier.

**The new `quit` action** closes every window at once. It asks only when a process is running somewhere: one
confirmation counts them across all windows, as Terminal.app does (`confirm_close_window`'s "several panes" prompt
is for closing one window). It reuses the existing confirm prompt. The session autosave runs as it
does when the last window closes. The action is unbound on Linux, but users can bind it.

**The app menu's Quit, Cmd+Q and the Dock's Quit** all send `terminate:`, and AppKit would end the process with no
confirmation or autosave. montty adds `applicationShouldTerminate:` to winit's app delegate class
(`WinitApplicationDelegate`) at runtime, the same technique as `window/titlebar.rs`. The method returns
`NSTerminateCancel` and posts a quit request (`UserEvent::Quit`) to montty's event loop through a stored proxy. montty
then runs `quit` and exits the event loop itself. winit's default menu (About, Hide, Hide Others, Show All, Quit) stays.

## 2. Option as Alt

A new `config.toml` setting, `option_as_alt`, takes `"left"` (the default on macOS), `"right"`, `"both"` or `"none"`.
It maps to winit's `OptionAsAlt` (`OnlyLeft`, `OnlyRight`, `Both`, `None`): `with_option_as_alt` at window creation,
and `set_option_as_alt` on every window when the config reloads. Linux ignores the setting; it appears in
`docs/config.md` as macOS-only.

## 3. Text size in points

`FontSet::load` computes pixels as `pt × 96/72 × scale`. On macOS the logical unit is already a point, so the factor
there is 1: `pt × scale`. An 11 pt font then matches other Mac apps, where today it is about a third too large. One
constant per OS, `PX_PER_PT`, with a unit test.

## 4. Starting from Finder

A Finder or Dock launch has a minimal environment, unlike a launch from a shell:

- **Shell:** when `$SHELL` is unset or empty, use the account's shell (`getpwuid_r` → `pw_shell`), then `/bin/sh`.
  This applies on both OSes: it is correct everywhere and cheap.
- **Locale (macOS):** when none of `LC_ALL`, `LC_CTYPE` and `LANG` is set, panes get `LC_CTYPE=UTF-8`, as
  Terminal.app does. Without it, bash and vim treat input as ASCII.
- **Working directory (macOS):** when montty's own working directory is `/` (a bundle launch), a pane with no
  explicit directory starts in `$HOME`.

All three come from one pure function of (environment, process cwd), which returns the overrides. It has unit tests;
`Pane::spawn` and `pane_options` apply its result.

## 5. Notifications

On macOS, `notify.rs` runs
`osascript -e 'on run argv' -e 'display notification (item 2 of argv) with title (item 1 of argv)' -e 'end run' <title> <body>`
instead of `notify-send`. The title and body are passed as arguments, never spliced into the script, so text from a
pane cannot inject AppleScript. The per-pane rate limiting stays as it is. These notifications appear as coming from
Script Editor; native ones need a signed bundle (out of scope).

## 6. App bundle and release

- **Bundle files:** `packaging/macos/Info.plist` (template) and `packaging/macos/build-app.sh <version> <binary>
  <out-dir>`, mirroring `build-deb.sh`. The script builds:
  - `montty.app/Contents/MacOS/montty`.
  - `Contents/Resources/montty.icns`, made with `iconutil` from `assets/icon/hicolor/*` (16-512 px; there is no
    1024 px source, so no 512@2x).
  - `Contents/Info.plist` with:
    - `CFBundleIdentifier = io.github.idahomst.montty`
    - the version
    - `NSHighResolutionCapable`
    - `LSMinimumSystemVersion = 11.0`
- **Signing and packaging:** ad-hoc signing (`codesign --sign -`), then `hdiutil` packs a `montty-<version>-macos.dmg`
  with the app and an Applications link.
- **Release job:** a new macOS job in `release.yml` builds `aarch64-apple-darwin` and `x86_64-apple-darwin`, joins
  them with `lipo` into a universal binary, runs `build-app.sh`, and uploads the `.dmg`. The `release` job waits for
  it, like the deb and Arch jobs. `workflow_dispatch` lets the job run without a tag.
- **Docs:** `docs/install.md` gets a macOS section: download the `.dmg`, drag the app to Applications. On first launch,
  right-click → Open, or run `xattr -dr com.apple.quarantine /Applications/montty.app`. Building from source is
  `cargo build --release` plus `build-app.sh`. README's "macOS is not supported yet" line changes.

## Testing

- **Unit tests** (they run on the macOS CI job):
  - the macOS default binding table, including that no two actions share a key
  - `option_as_alt` parsing
  - `PX_PER_PT`
  - the launch-environment function
  - the osascript argument list
  - the quit confirmation's process list across windows
- **Linux:** the existing tests must pass unchanged; the Linux defaults have their own table test.
- **Manual check on a Mac, per PR:**
  - the shortcuts from the table
  - Cmd+Q and Dock → Quit, with and without a running process
  - Option+B / Option+F in bash; right Option typing `@`
  - text size next to Terminal.app at the same point size
  - a launch from Finder: prompt, `locale`, `pwd`
  - a notification from `printf '\e]9;hi\a'`
  - the `.dmg` from a `workflow_dispatch` run, installed on the Mac
- **Docs** in the same PR as each change: `docs/config.md` (macOS bindings table, `option_as_alt`, `quit`),
  `docs/design.md` (Cmd+click, quit flow), AGENTS.md (the new files).
