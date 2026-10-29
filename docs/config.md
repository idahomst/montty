# montty configuration

montty reads plain files from one folder. Everything applies live: save a file and the window updates, no restart.
With no files at all, montty behaves like a stock Tilix-style terminal (font `monospace` 11 pt, Tango colors, 4 px
padding, 10 000 lines of scrollback, `$SHELL`, the default keys below).

## Where the files live

```
$XDG_CONFIG_HOME/montty/        (or ~/.config/montty/ when XDG_CONFIG_HOME is unset, empty or not an absolute path)
  config.toml                   window-wide settings and key bindings (optional)
  profiles/<name>.toml          one file per profile; the file name (without .toml) is the profile's id
  schemes/<name>.json           color schemes in Tilix's JSON format (optional)
  sessions/<name>.json          saved sessions (created on the first save)
```

- Only regular files whose name ends in exactly `.toml` (in `profiles/`) or `.json` (in `schemes/`) count. Ignored:
  names starting with `.` (hidden files, `.foo.toml.swp`) or `#` (`#foo.toml#`), anything with another ending
  (`foo.toml~`, `foo.toml.bak`, `foo.toml.swp`), directories, and files elsewhere in the folder.
- Symlinked files work, for example dotfiles managed in a git repository. montty also watches the symlink targets.
- If `$HOME` is unset and so is `XDG_CONFIG_HOME`, montty runs on built-in defaults and shows a toast.

### First run

If `profiles/` contains no `*.toml` file, montty creates it and writes a commented `profiles/default.toml`
with every setting at its default. Edit that file to change how all panes look. If the folder can't be written,
montty runs on the built-in defaults and shows a toast.

## config.toml

Every key is optional. Unknown keys are errors (a typo shows up in the toast instead of being ignored).

```toml
# The profile used by the first pane and by new tabs. Default: "default".
# If it doesn't exist: "default", else the alphabetically first profile, else built-in defaults;
# a toast says so (`no profile 'x' (using 'default')`) until the name or the profile is fixed.
default_profile = "default"

# Colors of the focused-pane border and the dividers between panes. Default: "#4a90d9" and "#555555".
focus_border_color = "#4a90d9"
divider_color = "#555555"

# How strongly unfocused panes are dimmed, 0.0 (not at all) to 1.0 (black). Default: 0.25.
dim_amount = 0.25

# Reopen the last window's layout at startup (see "Sessions and windows"). Default: false.
restore_last_session = false

# Flash the pane when a program rings the bell (e.g. Backspace on an empty prompt). Default: true.
# A background window still asks for attention either way.
visual_bell = true

# Drag a pane by its overlay (grip) onto another pane to swap them. Default: true.
pane_drag = true

# Ask before closing a window (or its last tab) that holds more than one pane. A running process always asks. Default: true.
confirm_close_window = true

# Dots on tabs and panes that need you (see "Attention marks"), with hover labels. Default: true.
# The state is tracked either way.
attention_marks = true

# Key bindings. Only list what you want to change.
[keybindings]
split_right = "Ctrl+Alt+E"                    # one combo
copy = ["Ctrl+Shift+C", "Ctrl+Insert"]        # several combos for one action
fullscreen = "none"                           # no key for this action
zoom_in = ["Ctrl+plus", "Ctrl+="]
```

### Key bindings

- The key is an action name from the table below; the value is a combo string, an array of combo strings, or `"none"`.
- A combo is modifiers and one key joined with `+`: `Ctrl+Alt+R`, `Shift+PageUp`, `F11`. Case doesn't matter.
  Modifiers: `Ctrl` (or `Control`), `Alt`, `Shift`, `Super` (or `Logo`, `Meta`).
- Keys: a single character (`r`, `0`, `=`, `[`), or a name: `Up`, `Down`, `Left`, `Right`, `PageUp`, `PageDown`
  (`Page_Up` too), `Insert`, `Delete`, `Home`, `End`, `Tab`, `Escape`, `F1` to `F12`.
- `+` and `-` are written `plus` and `minus` (`Ctrl+plus`, `Ctrl+minus`), because `+` separates the parts.
- Listing an action replaces all of its default combos (an array lists them all; `"none"` removes them). Actions you
  don't list keep their defaults.
- An action with several default combos, such as `zoom_in` (`Ctrl+=`, `Ctrl+Shift+plus`, `Ctrl+plus`), loses all of
  them when you list it. To rebind one and keep the others, list all of them again:
  `zoom_in = ["Ctrl+plus", "Ctrl+Shift+plus", "Ctrl+=", "Ctrl+Alt+Z"]`.
- One combo, one action. Using the same combo for two actions in the file is an error
  (`` `Ctrl+Alt+E` is bound to both `split_right` and `split_down` ``) and the whole file is rejected. If you give an
  action a combo that a default of a different, unlisted action uses, the listed action gets it and the other action
  loses that default.
- An unknown action name is an error: `` unknown action `splt_right` in [keybindings] ``.
- Digit keys also match by position on the number row, so `Alt+1` works on layouts (such as Czech) where the
  unshifted digit row types other characters.
- The mouse gestures (selection, middle click paste, Ctrl+click links; see "Mouse and clipboard") aren't configurable.

### Actions and their default keys

**Tiling**

| Action | Default |
|---|---|
| `split_right` | `Ctrl+Alt+R` |
| `split_down` | `Ctrl+Alt+D` |
| `split_left` | `Ctrl+Alt+Shift+R` |
| `split_up` | `Ctrl+Alt+Shift+D` |
| `split_auto` | `Ctrl+Alt+A` |
| `focus_up` | `Alt+Up` |
| `focus_down` | `Alt+Down` |
| `focus_left` | `Alt+Left` |
| `focus_right` | `Alt+Right` |
| `next_pane` | `Ctrl+Tab` |
| `prev_pane` | `Ctrl+Shift+Tab` |
| `resize_up` | `Alt+Shift+Up` |
| `resize_down` | `Alt+Shift+Down` |
| `resize_left` | `Alt+Shift+Left` |
| `resize_right` | `Alt+Shift+Right` |
| `maximize_pane` | `Ctrl+Shift+X` |
| `close_pane` | `Ctrl+Shift+W` |

**Tabs**

| Action | Default |
|---|---|
| `new_tab` | `Ctrl+Shift+T` |
| `close_tab` | `Ctrl+Shift+Q` |
| `next_tab` | `Ctrl+PageDown` |
| `prev_tab` | `Ctrl+PageUp` |
| `move_tab_left` | `Ctrl+Shift+PageUp` |
| `move_tab_right` | `Ctrl+Shift+PageDown` |

**Session and clipboard**

| Action | Default |
|---|---|
| `save_session` | `Ctrl+Shift+S` |
| `open_session` | `Ctrl+Shift+O` |
| `new_window` | `Ctrl+Shift+N` |
| `copy` | `Ctrl+Shift+C` |
| `paste` | `Ctrl+Shift+V` |
| `paste_primary` | `Shift+Insert` |
| `select_all` | `Ctrl+Shift+A` |

**Find and scrolling**

| Action | Default |
|---|---|
| `find` | `Ctrl+Shift+F` |
| `find_next` | `Ctrl+Shift+G` |
| `find_prev` | `Ctrl+Shift+H` |
| `scroll_line_up` | `Ctrl+Shift+Up` |
| `scroll_line_down` | `Ctrl+Shift+Down` |
| `scroll_page_up` | `Shift+PageUp` |
| `scroll_page_down` | `Shift+PageDown` |

**View**

| Action | Default |
|---|---|
| `zoom_in` | `Ctrl+=`, `Ctrl+Shift+plus`, `Ctrl+plus` |
| `zoom_out` | `Ctrl+minus` |
| `zoom_reset` | `Ctrl+0` |
| `fullscreen` | `F11` |
| `show_sizes` | `Ctrl+Alt+S` |

`show_sizes` shows every visible pane's size as `columns × rows` for 2 seconds; press it again to hide the labels.

**Numbered**

| Action | Default |
|---|---|
| `focus_pane_1` | `Alt+1` |
| `focus_pane_2` | `Alt+2` |
| `focus_pane_3` | `Alt+3` |
| `focus_pane_4` | `Alt+4` |
| `focus_pane_5` | `Alt+5` |
| `focus_pane_6` | `Alt+6` |
| `focus_pane_7` | `Alt+7` |
| `focus_pane_8` | `Alt+8` |
| `focus_pane_9` | `Alt+9` |
| `focus_pane_10` | `Alt+0` |
| `go_to_tab_1` | `Ctrl+Alt+1` |
| `go_to_tab_2` | `Ctrl+Alt+2` |
| `go_to_tab_3` | `Ctrl+Alt+3` |
| `go_to_tab_4` | `Ctrl+Alt+4` |
| `go_to_tab_5` | `Ctrl+Alt+5` |
| `go_to_tab_6` | `Ctrl+Alt+6` |
| `go_to_tab_7` | `Ctrl+Alt+7` |
| `go_to_tab_8` | `Ctrl+Alt+8` |
| `go_to_tab_9` | `Ctrl+Alt+9` |
| `go_to_tab_10` | `Ctrl+Alt+0` |

`Ctrl+plus`, `Ctrl+Shift+plus` and `Ctrl+=` all zoom in by default.

## Mouse and clipboard

### Selecting

- Drag with the left button to select text. Double-click selects a word, triple-click a line.
  Clicks count as a double or triple click when they follow each other within 400 ms on the same cell; a fourth
  click starts over as a single click.
- Alt+drag selects a rectangular block.
- Shift+click extends the existing selection to the clicked cell; with no selection it starts one (in a program that
  uses the mouse, see below, it always starts a new one).
- Releasing a selection copies it to the primary selection (select to primary). Middle click pastes the primary
  selection into the pane under the pointer.
- `copy` (`Ctrl+Shift+C`) puts the focused pane's selection into the clipboard; with no selection it does nothing.
  `paste` (`Ctrl+Shift+V`) pastes the clipboard and `paste_primary` (`Shift+Insert`) the primary selection into the
  focused pane. `select_all` (`Ctrl+Shift+A`) selects the whole scrollback and screen of the focused pane and also
  copies it to the primary selection.
- A paste scrolls the pane to the bottom, like typing. In bracketed-paste mode the text is wrapped in the
  bracketed-paste markers and any ESC and Ctrl+C characters in it are removed; otherwise line breaks become `\r`.
- If the clipboard can't be opened, copy and paste do nothing, and `montty: clipboard: <error>` is printed once.

### Panes

Alt+click on the Split right / Split down buttons splits left / up instead.

Press on a pane's overlay (the grip, left of the buttons, is made for it) and drag at least a few pixels onto another
pane of the same tab: the two swap places and sizes, both programs keep running, and the dragged pane takes the
focus. Escape, the window losing focus, or dropping anywhere else cancels. A plain click on a button works as
before. There is no dragging while a pane is maximized or when the tab has a single pane. `pane_drag = false` turns it off: no grip, and the buttons
act on press.

Closing a window, or its last tab, that holds more than one pane asks once ("This window has 3 panes. Close anyway?");
a running process asks instead, never both. `confirm_close_window = false` turns the pane prompt off.

### Programs that use the mouse

When a program asks for mouse reporting (htop, vim with `mouse=a`, tmux), clicks, drags, motion and the wheel go to
the program instead. Hold Shift to bypass that: Shift+drag selects, Shift+middle click pastes and Shift+wheel scrolls
montty's scrollback, as without reporting. With Shift held, clicks select as usual: Shift can't also extend a
selection there, so each Shift+click or Shift+drag starts a new one.

### Links

Ctrl+click opens a link with `xdg-open`, even while the program has mouse reporting on. A link is either an OSC 8
hyperlink on the cell or, failing that, a URL found in the text under the pointer (a URL that wraps onto the next
line counts). Text starting with `www.` also counts as a URL (not when it follows a letter, digit, `.`, `-`, `_`,
`/` or `@`) and opens as `https://` plus the text. Only these schemes open: `http`, `https`, `ftp`, `file`,
`mailto`, `gemini`, `gopher`, `git`, `ssh`, `news`, `magnet`, `ipfs`, `ipns`. While Ctrl is held over a link the
pointer becomes a hand and the link is underlined; if the link's address differs from its text (an OSC 8 link, or a
`www.` link), a label shows the address. If `xdg-open` can't be started, `montty: xdg-open: <error>` is printed.

### Programs and the clipboard (OSC 52)

A program may store text into the clipboard or the primary selection with OSC 52. Reading them is denied: a program
asking for the clipboard's contents gets nothing back. This is not configurable.

### Find

`find` (`Ctrl+Shift+F`) or the magnifier button in the top bar opens a find bar at the top right of the focused pane.
If it is already open, the field is focused and its text selected.

- Type to search; the search is literal text (no regular expressions) and ignores case. The `Aa` button switches to
  an exact-case match. The search runs from the bottom of the scrollback upward, so the newest match becomes the
  current one and is scrolled into view. The label shows `no matches` when there is none.
- Matches are highlighted in amber and the current match in orange; a selection is drawn over both.
- `Enter` (and `find_next`, `Ctrl+Shift+G`) or the bar's `↓` button goes to the next match, toward newer text.
  `Shift+Enter` (and `find_prev`, `Ctrl+Shift+H`) or the `↑` button goes to the previous one. Both wrap around.
- `Escape` or the bar's `✕` button closes the bar and clears the highlights; the scroll position stays. Focusing
  another pane (a click, a keyboard focus move) closes it too and gives the keyboard back to the terminal, and so
  does switching tab.
- While the bar is open, typing goes to its field. `paste` and `paste_primary` insert the clipboard's or the primary
  selection's first line into the field and search for it, and so does a middle click on the bar; `copy` still
  copies the pane's selection. Other bindings that aren't text editing (`find_next`, `new_tab`, ...) still work.
- The current match follows the text only while it stays put: when new output moves it, the highlight is dropped and
  the next step searches again from the bottom.

### Bell and scrollbar

- A bell (`\a`) in a visible pane flashes it briefly (`visual_bell = false` in `config.toml` turns that off). When the window isn't focused, the bell also asks the window
  manager for attention (a taskbar highlight); a bell in a hidden pane only does that.
- A thin scrollbar appears at a pane's right edge while the pane is scrolled back and for one second after any scroll,
  then disappears. It is only an indicator: it can't be dragged. A pane with no history never shows it.

### Attention marks

A small dot shows where a program needs you. A tab's dot sits before its title; a pane's dot sits in the pane's
top-right corner. Hovering a dot shows its label. `attention_marks = false` in `config.toml` hides the dots and
labels (the state is still tracked).

- Amber: the program is blocked on you (OSC 7501 `state=blocked`, labelled `Needs permission`, `Has a question`,
  `Needs login` or `Needs you`; with `app` and `msg` set, `<app>: <msg>`), or it sent a notification (OSC 9, OSC 777
  `notify`) or a bell (`Bell`) while you weren't looking at the pane.
- Green: the program finished (`state=done`, `Done`) while you weren't looking. Red: it failed (`state=error`, `Failed`).
- A tab shows the most urgent dot of its panes: amber, then red, then green.
- A blocked dot goes away when the program moves on (`state=working`, a new prompt, a reset). Done, failed, bell and
  notification dots stay until you look at the pane (it is the focused pane of the active tab in a focused window).
  A pane you are already looking at never gets a dot.

## Profiles

A profile is a file `profiles/<name>.toml`. Every key is optional; a missing key uses its default.
A pane uses one profile, and different panes may use different ones. Unknown keys are errors.

```toml
name = "Work"                  # shown in the profile menu; empty or missing: the file name
font = "JetBrains Mono"        # font family
font_size = 12                 # points
scheme = "solarized-dark"      # a built-in scheme, or a file in schemes/
bold_is_bright = false
opacity = 0.9                  # window transparency; needs a compositor
padding = 6
scrollback = 50000
cursor_shape = "beam"
cursor_blink = true
login_shell = true
shell = "/bin/zsh"

[colors]                       # override single colors of the scheme
background = "#101418"
cursor = "rgb(255, 170, 0)"

[background_image]
path = "~/Pictures/wall.jpg"
fit = "fill"
opacity = 0.35
```

### Keys

| Key | Type | Default | Range / values |
|---|---|---|---|
| `name` | string | file name | any text |
| `font` | string | `"monospace"` | a family name as `fc-list : family` prints it (case-insensitive); `monospace` is resolved through fontconfig |
| `font_size` | number | `11` | 4 to 200 (points) |
| `scheme` | string | `"tango"` | a file name (without `.json`) in `schemes/`, a built-in name, or a scheme's display name (case-insensitive) |
| `bold_is_bright` | bool | `false` | `true` draws bold text in the bright palette colors |
| `opacity` | number | `1.0` | 0.0 (invisible) to 1.0 (opaque) |
| `padding` | integer | `4` | 0 to 100 (pixels) |
| `scrollback` | integer | `10000` | 0 to 1 000 000 (lines) |
| `cursor_shape` | string | `"block"` | `"block"`, `"beam"`, `"underline"` |
| `cursor_blink` | bool | `false` | blink the cursor, see below |
| `shell` | string | `$SHELL`, else `/bin/sh` | a program path |
| `login_shell` | bool | `false` | `true` starts the shell with `-l` |
| `command` | array of strings | none | `["htop"]` runs this instead of a shell; no empty arguments; wins over `shell` |
| `[colors]` | table | none | see below |
| `[background_image]` | table | none | see below |

- A font family that isn't installed isn't fatal: the pane uses `monospace` at the profile's size and a toast reads
  `profiles/x.toml: no font found for family 'Foo'`.
- An unknown `scheme` is not fatal either: the profile uses Tango (with its own `[colors]`) and a toast names the
  line. Out-of-range numbers are errors with their line, and the profile keeps its last good version.
- `shell`, `login_shell` and `command` apply to panes started after the change; running shells aren't touched.
  Everything else (font, colors, padding, scrollback, cursor, opacity, image) applies to open panes at once.
- If a profile's `shell` or `command` can't start, the pane runs `$SHELL` (else `/bin/sh`) instead and a toast says why; `montty -e CMD` has no fallback and fails.
- `montty -e CMD ...` runs `CMD` in the first pane instead of the profile's shell or command.
- The bar, menus, panels and toasts use the default profile's font at its base size; zoom doesn't change them.

### Colors

Color strings are `#rgb`, `#rrggbb`, `#rrggbbaa` (the alpha part is ignored) or `rgb(r, g, b)` with 0 to 255.
Case and surrounding spaces don't matter. Color names such as `blue` aren't accepted.

`[colors]` overrides single colors of the scheme; every key is optional:

| Key | Meaning |
|---|---|
| `foreground`, `background` | default text and background |
| `cursor` | cursor color (default: the scheme's, else its foreground) |
| `selection_foreground`, `selection_background` | selected text |
| `palette` | exactly 16 colors, black to bright white (the 8 normal colors, then the 8 bright ones) |

A program that sets the background with OSC 11 changes what a transparent pane shows too.

### Cursor blink

With `cursor_blink = true` the cursor of the focused pane blinks. It stops blinking, and stays visible, 10 seconds
after the last key press or output in that pane, so an idle window costs no CPU. The next key or output starts it
again.

### Transparency

`opacity` below 1.0 makes the window background translucent, but you only see through it when the desktop runs a
compositing window manager (GNOME, KDE, picom, ...). Without a compositor the window just looks opaque or black.
Text stays fully opaque. Opacity belongs to the profile, so two panes of one window may differ.

### Background image

```toml
[background_image]
path = "wall.png"       # required
fit = "fill"            # optional
opacity = 1.0           # optional, 0.0 to 1.0
```

- `path`: a PNG or JPEG file. `~/` is your home folder. A relative path is relative to the config folder
  (`~/.config/montty/`), so `path = "wall.png"` finds `~/.config/montty/wall.png`. Other formats (GIF, WebP, ...)
  are refused with a toast.
- `fit`:
  - `fill` (default): scale to cover the pane, keeping proportions; the overflow is cropped.
  - `stretch`: scale to the pane's size, ignoring proportions.
  - `center`: original size, centered, not scaled.
  - `tile`: original size, repeated.
- `opacity`: how strongly the image shows over the pane background; the text is drawn on top.
- Limits: files over 600 MiB and images over 134 million pixels (2^27) are refused. Larger images are accepted but
  shrunk to at most 3840x2160 (or your largest monitor, if bigger) once loaded.
- Images load in the background, so the window never waits for them. A missing, unreadable or corrupt image shows a
  toast that names the file, and the pane draws without it. While the picture is loading or after a failed reload,
  the pane keeps the last good one. Panes whose profiles use the same image share one copy.
- Images are decoded in a separate process, so a broken file can't crash montty. If that process dies the toast says
  `decoder crashed`, and if it takes longer than 15 seconds it is stopped and the toast says `decoder timed out`.
- Editing or replacing the image file reloads it, the same as editing a config file.

## Color schemes

Schemes use Tilix's JSON format, so Tilix's scheme collection works unchanged. Pick one with `scheme = "<name>"`.

Built in (match by these names): `base16-twilight-dark`, `linux`, `material`, `monokai`, `orchis`,
`solarized-dark`, `solarized-light`, `tango`, `yaru`. They are copied from Tilix (MPL-2.0); see
`assets/schemes/LICENSE-NOTICE`.

To add more, copy a Tilix `.json` file into `~/.config/montty/schemes/` (Tilix keeps its own in
`~/.config/tilix/schemes/`). The file name without `.json` is the scheme's name, and the `name` inside the file works
as well (case-insensitive). A file in `schemes/` with the same name as a built-in one replaces the built-in.

What montty reads from the file; everything else is ignored. A missing `palette`, `foreground-color` or
`background-color` falls back to Tango's value, a missing cursor color follows the scheme's own foreground, and
missing selection colors mean none (the default selection colors). A color that doesn't parse is an error naming
the file:

| Key | Meaning |
|---|---|
| `name` | display name |
| `foreground-color`, `background-color` | default colors |
| `palette` | exactly 16 colors, otherwise the file is an error naming it |
| `use-cursor-color`, `cursor-background-color` | cursor color, used only when `use-cursor-color` is `true` |
| `use-highlight-color`, `highlight-foreground-color`, `highlight-background-color` | selection colors, used only when `use-highlight-color` is `true` |

Colors may be `#rgb`, `#rrggbb` or `rgb(r,g,b)`. A profile's `[colors]` table is applied on top of its scheme.

## Zoom

`zoom_in`, `zoom_out` and `zoom_reset` (`Ctrl+plus`, `Ctrl+minus`, `Ctrl+0`) change the font size of the focused pane
only, in 1 point steps within 4 to 200 pt. Other panes keep their size. Zoom is added to the profile's `font_size`,
isn't saved, and starts at zero in a new pane (a split pane doesn't inherit it).

## Choosing a profile for a pane

- Hover a pane: its overlay shows a **Profile** button (the first of the buttons after the grip in the pane's top-right corner). It opens
  a menu of all profiles; pick one and that pane switches at once. The shell keeps running; only the look changes.
  A pane's profile isn't stored anywhere, so it is gone when the window closes.
- A split pane starts with the profile of the pane that was split. A new tab, and the first pane, use the window's
  default profile.
- The window's default profile is `default_profile` in `config.toml`, or `--profile` on the command line.

```
montty [--profile NAME] [--session NAME] [-e COMMAND [ARGS...]]

  --profile NAME  the window's default profile (profiles/NAME.toml): the first pane
                  and new tabs use it
  --session NAME  open the saved session sessions/NAME.json (not with -e)
  -e, --command   run COMMAND (everything after it) instead of the default shell
  -h, --help      print this help
```

`NAME` is the file name without `.toml`. An unknown name falls back as described under `default_profile`, and a toast says so at startup (`--profile: no profile 'wrok' (using 'default')`).
If a pane's profile is deleted or renamed while it is open, the pane switches to the default profile.

## Sessions and windows

A session is a window's layout: its tabs, their split trees, each pane's profile and working directory. Processes
and scrollback are not saved; opening a session starts a fresh shell in every pane.

### Windows

- `new_window` (`Ctrl+Shift+N`, or `New window` in the ≡ menu) opens another window in the same process, with one
  pane. The pane uses the requesting window's default profile (its `--profile` if it has one) and no `-e` command.
  `-e` applies to the first window only.
- Closing the last window exits montty. Windows are independent otherwise (tabs, panes, zoom).

### Working directory of a new pane

A split starts in the focused pane's working directory. A new tab or window starts where montty itself was started.
montty reads the directory from `/proc/<shell pid>/cwd` when it needs it; it doesn't poll. It doesn't read OSC 7 (the
escape sequence some shells use to report their directory), because the terminal library montty uses drops it before
montty sees it. So the directory is the shell's own, wherever it is at that moment.

### Saving

`save_session` (`Ctrl+Shift+S`, or `Save session…` in the ≡ menu) opens a `Save session` name prompt in the window.
The field starts with the name this window last saved or opened. Enter saves, replacing a file of that name; Esc
cancels. An invalid name keeps the prompt open and shows the error toast `invalid session name: <why>`. On success a
neutral (not red) toast says `saved session '<name>'`.

A name is 1 to 64 characters, with no `/`, no NUL and no control characters, doesn't start with `.`, and isn't `_last`
(reserved for the autosave). The file is `sessions/<name>.json`; it is written to `<name>.json.tmp` and renamed over
the target, so a crash never leaves half a file. The listing of saved sessions ignores everything that isn't
`*.json`, and `_last.json`.

### Opening

`open_session` (`Ctrl+Shift+O`) or `Open session ▸` in the ≡ menu lists the saved names, sorted (a disabled
`no saved sessions` row when there are none). Pick one:

- If the window is *fresh*, the session replaces its content. Fresh means: exactly one tab with one pane, the pane's
  shell has no foreground child process, the pane wasn't started with an explicit command (`-e` or a session's
  `command`) or running a profile's `command`, and it has received no typing or paste.
- Otherwise the session opens in a new window.

`montty --session NAME` opens `sessions/NAME.json` in the first window at startup. It can't be combined with `-e`
(`--session and -e can't be combined`). A missing or invalid session shows the error toast and the window starts with
the default pane. A session with errors opens nothing.

### Restoring the last session

With `restore_last_session = true` in `config.toml` (default `false`), montty keeps `sessions/_last.json` up to date:

- A layout change arms a 1 second debounce for that window; when it fires, the window is written to `_last.json`.
  Layout changes are: split, swap, close pane, divider drag or equalize, maximize, new, close, move or rename tab,
  profile switch, and opening a session. Nothing runs while the window is idle.
- When the user closes the last window, that window is written too. A window with no panes left is never written.
- With several windows the most recent write wins.
- At startup, a valid `_last.json` opens instead of the default pane. `-e` and `--session` win over it. An invalid one
  shows the error toast and the window starts with the default pane.
- `_last` can't be saved or opened by name; copy the file to another name to keep it.

### The ≡ menu

In order: `New tab`, `New window`, `Save session…`, `Open session ▸`, `Edit config`, `Open config folder`,
`Reload config`, `Keyboard shortcuts`, `About montty`. Shortcuts are shown beside the items that have one.
`Edit config` opens `config.toml` (created empty first if it doesn't exist) in a new tab running the first of
`$VISUAL`, `$EDITOR` and Debian's `editor` (the system default, `update-alternatives --config editor`) that is set
and on `PATH`; a value may carry arguments (`nvim -p`). The tab closes when the editor exits. Without any of them
it runs `xdg-open` on the file instead. The variables are read from montty's own environment, so set them in
`~/.profile` (or your desktop's environment), not only in `~/.bashrc`. `Open config folder` runs `xdg-open` on the
config folder, and `Reload config` re-reads everything now, as if a file had changed.

### The session file

Pretty-printed JSON in montty's own format (not Tilix's):

```json
{
  "version": 1,
  "window": { "width": 1000, "height": 700 },
  "active_tab": 0,
  "tabs": [
    { "title": "build", "focused": 1,
      "root": { "split": "right", "ratio": 0.5,
                "a": { "profile": "default", "cwd": "/home/u/src" },
                "b": { "profile": "work", "cwd": "/tmp", "command": ["htop"] } } }
  ]
}
```

- `version` must be 1. Unknown keys are errors.
- `window` is the window's inner size in logical pixels. Optional, and best effort on load: a dimension larger than
  the monitor becomes 90 % of the monitor, so a session saved on a bigger screen still fits.
- `active_tab` is the index of the tab shown.
- `title` is the tab's rename. Optional.
- `focused` is the depth-first index of the focused pane.
- A split has `split` (`"right"` = side by side, `"down"` = stacked), `ratio` (0.0 to 1.0, the first part's share),
  and the two halves `a` and `b`. A leaf has:
  - `profile`: the profile's file name without `.toml`. An empty or unknown name means the default profile.
  - `cwd`: optional. Dropped on load when it isn't an existing directory.
  - `command`: optional. Recorded only for a pane started with an explicit command (`-e`, or a session leaf's
    `command`), never for a profile's own `command`.
- Errors use the config error format (`sessions/<name>.json:<line>: <message>`) and show as the error toast.

## Live reload

- montty watches the config folder (the folders, not single files). One burst of file changes causes one reload,
  150 ms after the last change (at most 1 s after the first), and every file is re-read.
- Editor save patterns are fine: writing in place, or writing a temp file and renaming it over the original. A file
  caught half-written doesn't stick; when it's complete the next reload picks it up. An empty `config.toml` or
  profile is valid and means all defaults, so a reload that catches a file at the moment an editor truncates it
  would apply defaults for a moment. The 150 ms debounce usually rides over that moment, and the real content
  follows with the next reload.
- A file that doesn't parse never replaces its last good version. The window keeps running on the last good one (or
  on built-in defaults if there never was one) and shows a toast. A fixed file clears the problem on the next save.
- The toast shows the first error, then `(+N more)` if there are more:

  ```
  profiles/work.toml:7: font_size must be between 4 and 200, got 500 (+1 more)
  ```

  The form is `<file relative to the config folder>:<line>: <message>`; errors without a line are
  `<file>: <message>`. A long first error is cut with `…`, never the `(+N more)`. A misspelled key reads
  `` config.toml:1: unknown key `bogus` ``. Every error, not only the first, is also printed to stderr. The toast
  goes away after 8 seconds or on a click.
- A profile that is added shows up in the profile menu; one that is deleted drops out and its panes use the default.
- Key bindings, border and divider colors and `dim_amount` change live too.
