# macOS Release Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make montty a macOS app that other people can download and use. That means Mac-style shortcuts, a confirmed
quit, Option as Alt, text sized in points, sane defaults when started from Finder, notifications, and a `montty.app`
in a `.dmg` in the GitHub release.

**Architecture:** There are six small PRs on `main` (tasks 1-4, 5, 6, 7, 8, 9-10). Each switches behaviour per OS with
`cfg!(target_os = "macos")` or `#[cfg(...)]` next to the code it changes, as AGENTS.md describes. Pure logic gets
unit tests that run on both OSes whenever possible. For example, both default binding tables are plain functions that
are tested everywhere, and only `Bindings::defaults()` picks one. macOS-only glue (the Objective-C runtime hooks, winit
macOS extension traits) stays thin.

**Tech Stack:** Rust 2024 (rust-version 1.88), winit 0.30.13, glutin, alacritty_terminal 0.26 (vendored), libc. macOS
needs no new crates: Objective-C runtime calls are declared by hand against `libobjc`, as in
`src/window/titlebar.rs`.

**Spec:** `docs/superpowers/specs/2026-11-05-macos-release-design.md`

## Global Constraints

- Linux behaviour must not change, except for the new optional `quit` action (unbound on Linux) and the `$SHELL`
  fallback to the account's shell (task 7).
- No new crates (AGENTS.md: "No new crates without a strong reason").
- Every PR must pass `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`,
  `cargo clippy --all-targets --locked --target x86_64-unknown-linux-gnu -- -D warnings` (run on the Mac) and
  `cargo test --locked`. CI runs Linux and macOS.
- Bundle identifier: `io.github.idahomst.montty`. Minimum macOS: `11.0`.
- macOS default for `option_as_alt`: `"left"`.
- Comments explain why, not what, and match the surrounding density (AGENTS.md).
- Tests never touch the real `~/.config/montty`.
- GUI behaviour can't be checked by an agent. Each PR ends with a **manual check** that the human runs on the Mac
  before merging.
- Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

---

## PR 1: Mac-style shortcuts and quitting (tasks 1-4)

Branch: `macos-shortcuts`.

### Task 1: macOS default key bindings and Mac key names

**Files:**
- Modify: `src/input/bindings.rs`: the `NAMED` table, `KeyCombo` Display and parsing, `Bindings::defaults`, and tests
- Modify: `docs/config.md`: the "Actions and their default keys" tables get a macOS column, plus the key-syntax notes

**Interfaces:**
- Produces: `Bindings::linux_defaults() -> Bindings`, `Bindings::macos_defaults() -> Bindings`, and
  `Bindings::defaults()`, which picks one by OS.
- Produces: `pub enum KeyLabels { Linux, Mac }` with `KeyLabels::NATIVE`, and
  `KeyCombo::label(&self, KeyLabels) -> String`. `Display` uses `KeyLabels::NATIVE`.
- Produces: the named key `enter` (`NamedKey::Enter`, label `Enter`).

- [ ] **Step 1: Write failing tests** in `src/input/bindings.rs`'s `mod tests`. Rename `tilix_defaults` to
  `linux_defaults_are_tilix_like`, and change its first line to `let b = Bindings::linux_defaults();`. Then add:

```rust
    #[test]
    fn macos_defaults() {
        let b = Bindings::macos_defaults();
        let cases = [
            ("Super+C", Action::Copy),
            ("Super+V", Action::Paste),
            ("Super+A", Action::SelectAll),
            ("Super+F", Action::Find),
            ("Super+G", Action::FindNext),
            ("Super+Shift+G", Action::FindPrev),
            ("Super+T", Action::NewTab),
            ("Super+N", Action::NewWindow),
            ("Super+W", Action::ClosePane),
            ("Super+Shift+W", Action::CloseTab),
            ("Super+Shift+]", Action::NextTab),
            ("Super+Shift+[", Action::PrevTab),
            ("Super+1", Action::GoToTab(1)),
            ("Super+9", Action::GoToTab(9)),
            ("Super+Shift+Left", Action::MoveTabLeft),
            ("Super+Shift+Right", Action::MoveTabRight),
            ("Super+D", Action::SplitRight),
            ("Super+Shift+D", Action::SplitDown),
            ("Super+Alt+D", Action::SplitLeft),
            ("Super+Alt+Shift+D", Action::SplitUp),
            ("Super+Shift+A", Action::SplitAuto),
            ("Super+Alt+Up", Action::FocusUp),
            ("Super+Alt+Left", Action::FocusLeft),
            ("Super+Alt+1", Action::FocusPane(1)),
            ("Super+Alt+0", Action::FocusPane(10)),
            ("Super+]", Action::NextPane),
            ("Super+[", Action::PrevPane),
            ("Super+Ctrl+Up", Action::ResizeUp),
            ("Super+Ctrl+Right", Action::ResizeRight),
            ("Super+Shift+Enter", Action::MaximizePane),
            ("Super+S", Action::SaveSession),
            ("Super+O", Action::OpenSession),
            ("Super+=", Action::ZoomIn),
            ("Super+plus", Action::ZoomIn),
            ("Super+minus", Action::ZoomOut),
            ("Super+0", Action::ZoomReset),
            ("Super+Ctrl+F", Action::Fullscreen),
            ("Super+Alt+S", Action::ShowSizes),
            ("Super+Up", Action::ScrollLineUp),
            ("Super+Down", Action::ScrollLineDown),
            ("Shift+PageUp", Action::ScrollPageUp),
            ("Shift+PageDown", Action::ScrollPageDown),
        ];
        for (combo, action) in cases {
            assert_eq!(b.lookup(&k(combo)), Some(action), "{combo}");
        }
        // Ctrl stays the shell's: no macOS default uses Ctrl without Cmd.
        for (c, a) in b.entries() {
            assert!(!c.mods.ctrl || c.mods.logo, "{c} ({a:?}) takes Ctrl from the shell");
        }
        // Macs have no Insert key; tab 10 is unbound as in Terminal.app.
        assert!(b.entries().iter().all(|(_, a)| *a != Action::PastePrimary));
        assert!(b.entries().iter().all(|(_, a)| *a != Action::GoToTab(10)));
    }

    #[test]
    fn defaults_follow_the_os() {
        let want = if cfg!(target_os = "macos") {
            Bindings::macos_defaults()
        } else {
            Bindings::linux_defaults()
        };
        assert_eq!(Bindings::defaults(), want);
    }

    #[test]
    fn mac_key_names() {
        let c = k("super+alt+shift+d");
        assert_eq!(c.label(KeyLabels::Mac), "Option+Shift+Cmd+D");
        assert_eq!(c.label(KeyLabels::Linux), "Alt+Shift+Super+D");
        // Mac names parse too.
        assert_eq!(k("Cmd+Option+D"), k("Super+Alt+D"));
        assert_eq!(k("command+opt+d"), k("Super+Alt+D"));
        assert_eq!(k("Super+Shift+Enter").label(KeyLabels::Mac), "Shift+Cmd+Enter");
        assert_eq!(k("Super+Shift+Return"), k("Super+Shift+Enter"));
    }
```

  In `keycombo_display_roundtrip`, change the last assertion to use explicit labels, and make the roundtrip cover
  both tables:

```rust
    #[test]
    fn keycombo_display_roundtrip() {
        for b in [Bindings::linux_defaults(), Bindings::macos_defaults()] {
            for (combo, _) in b.entries() {
                for labels in [KeyLabels::Linux, KeyLabels::Mac] {
                    let s = combo.label(labels);
                    assert_eq!(s.parse::<KeyCombo>().as_ref(), Ok(&combo), "{s}");
                }
            }
        }
        assert_eq!(k("ctrl+alt+r").label(KeyLabels::Linux), "Ctrl+Alt+R");
        assert_eq!(k("Shift+Page_Up").label(KeyLabels::Linux), "Shift+PageUp");
        assert_eq!(k("Ctrl+plus").label(KeyLabels::Linux), "Ctrl+plus");
        assert_eq!(
            k("super+shift+ctrl+alt+minus").label(KeyLabels::Linux),
            "Ctrl+Alt+Shift+Super+minus"
        );
    }
```

  Replace `config_md_lists_all_actions` so it checks both columns of the docs table:

```rust
    /// Every action has a row "| `name` | linux combos | macOS combos |" in docs/config.md
    /// whose combos are each OS's defaults, in the order the shortcuts list shows them.
    #[test]
    fn config_md_lists_all_actions() {
        let doc = include_str!("../../docs/config.md");
        let column = |b: &Bindings, a: Action, labels: KeyLabels| {
            b.entries()
                .iter()
                .filter(|(_, x)| *x == a)
                .map(|(c, _)| format!("`{}`", c.label(labels)))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let (linux, mac) = (Bindings::linux_defaults(), Bindings::macos_defaults());
        for a in Action::all() {
            let name = a.config_name();
            let prefix = format!("| `{name}` | ");
            let row = doc
                .lines()
                .find_map(|l| l.strip_prefix(&prefix))
                .unwrap_or_else(|| panic!("docs/config.md has no table row for `{name}`"));
            let cells: Vec<&str> = row.trim_end().trim_end_matches('|').split('|').map(str::trim).collect();
            assert_eq!(cells.len(), 2, "{name}: want a Linux and a macOS column");
            assert_eq!(cells[0], column(&linux, a, KeyLabels::Linux), "{name} (Linux)");
            assert_eq!(cells[1], column(&mac, a, KeyLabels::Mac), "{name} (macOS)");
        }
    }
```

  Other tests assume Linux keys through `Bindings::defaults()`. Find them with
  `grep -rn "Bindings::defaults()" src --include=*.rs`; today they are in `src/input/bindings.rs`,
  `src/window/modal/content.rs` (lines ~374, 493, 501, 545, 595, 755, 784) and `src/window/modal/paint.rs` (~328,
  504). Change each one inside `#[cfg(test)]` to `Bindings::linux_defaults()`, so it keeps testing what it tested
  before. `content.rs:493` expects the shortcut text `Ctrl+Shift+N`, and a Linux combo without Alt or Super
  displays the same with either label set. Leave the one non-test call, `src/config/global.rs:63`
  (`GlobalConfig::default`), on `Bindings::defaults()`.

- [ ] **Step 2: Run the tests to confirm they fail.**

Run: `cargo test --lib input::bindings`
Expected: compile errors (`linux_defaults`, `macos_defaults`, `KeyLabels` and `label` don't exist).

- [ ] **Step 3: Implement.** In `src/input/bindings.rs`:

  Add the Enter rows to `NAMED`, after `("escape", Escape, "Escape"),`:

```rust
        ("enter", Enter, "Enter"),
        ("return", Enter, "Enter"),
```

  Replace the `impl fmt::Display for KeyCombo` block's modifier loop with labels chosen per style:

```rust
/// Modifier names: Linux's, or the Mac's (Cmd for Super, Option for Alt, in Apple's order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyLabels {
    Linux,
    Mac,
}

impl KeyLabels {
    /// The names shown in this OS's shortcuts list and errors.
    pub const NATIVE: KeyLabels = if cfg!(target_os = "macos") {
        KeyLabels::Mac
    } else {
        KeyLabels::Linux
    };
}

impl KeyCombo {
    /// "Ctrl+Alt+R" (Linux) or "Option+Shift+Cmd+D" (Mac): parses back to the same combo.
    pub fn label(&self, labels: KeyLabels) -> String {
        let m = &self.mods;
        let mods: [(bool, &str); 4] = match labels {
            KeyLabels::Linux => [(m.ctrl, "Ctrl"), (m.alt, "Alt"), (m.shift, "Shift"), (m.logo, "Super")],
            KeyLabels::Mac => [(m.ctrl, "Ctrl"), (m.alt, "Option"), (m.shift, "Shift"), (m.logo, "Cmd")],
        };
        let mut s = String::new();
        for (on, name) in mods {
            if on {
                s.push_str(name);
                s.push('+');
            }
        }
        s.push_str(&key_label(&self.key));
        s
    }
}

fn key_label(key: &KeyName) -> String {
    match key {
        KeyName::Char('+') => "plus".into(),
        KeyName::Char('-') => "minus".into(),
        // Uppercase only when it parses back to `c` ('ß' would become "SS").
        KeyName::Char(c) => {
            let mut up = c.to_uppercase();
            match (up.next(), up.next()) {
                (Some(u), None) if u.to_lowercase().eq([*c]) => u.to_string(),
                _ => c.to_string(),
            }
        }
        // Every key `parse_named` accepts has a label; the debug form is a fallback only.
        KeyName::Named(n) => named_label(*n).map_or_else(|| format!("{n:?}"), str::to_string),
    }
}

impl fmt::Display for KeyCombo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label(KeyLabels::NATIVE))
    }
}
```

  In `FromStr for KeyCombo`, extend the modifier match:

```rust
                "ctrl" | "control" => mods.ctrl = true,
                "alt" | "option" | "opt" => mods.alt = true,
                "shift" => mods.shift = true,
                "super" | "logo" | "meta" | "cmd" | "command" => mods.logo = true,
```

  Rename the existing `pub fn defaults() -> Bindings` to `pub fn linux_defaults() -> Bindings`, with its body
  unchanged. Then add:

```rust
    /// This OS's built-in bindings.
    pub fn defaults() -> Bindings {
        if cfg!(target_os = "macos") {
            Bindings::macos_defaults()
        } else {
            Bindings::linux_defaults()
        }
    }

    /// Mac-style built-ins: Cmd where Linux has Ctrl+Shift, after Terminal.app and iTerm2.
    /// Ctrl without Cmd stays the shell's. Cmd+Q is the app menu's (see `macos_quit`).
    pub fn macos_defaults() -> Bindings {
        use Action::*;
        let mut map = HashMap::new();
        let mut bind = |s: &str, a: Action| {
            let combo: KeyCombo = s.parse().expect("valid built-in key combo");
            map.insert(combo, a);
        };
        bind("Cmd+D", SplitRight);
        bind("Cmd+Shift+D", SplitDown);
        bind("Cmd+Option+D", SplitLeft);
        bind("Cmd+Option+Shift+D", SplitUp);
        bind("Cmd+Shift+A", SplitAuto);
        bind("Cmd+Option+Up", FocusUp);
        bind("Cmd+Option+Down", FocusDown);
        bind("Cmd+Option+Left", FocusLeft);
        bind("Cmd+Option+Right", FocusRight);
        bind("Cmd+]", NextPane);
        bind("Cmd+[", PrevPane);
        bind("Cmd+Ctrl+Up", ResizeUp);
        bind("Cmd+Ctrl+Down", ResizeDown);
        bind("Cmd+Ctrl+Left", ResizeLeft);
        bind("Cmd+Ctrl+Right", ResizeRight);
        bind("Cmd+Shift+Enter", MaximizePane);
        bind("Cmd+W", ClosePane);
        bind("Cmd+T", NewTab);
        bind("Cmd+Shift+W", CloseTab);
        bind("Cmd+Shift+]", NextTab);
        bind("Cmd+Shift+[", PrevTab);
        bind("Cmd+Shift+Left", MoveTabLeft);
        bind("Cmd+Shift+Right", MoveTabRight);
        bind("Cmd+S", SaveSession);
        bind("Cmd+O", OpenSession);
        bind("Cmd+N", NewWindow);
        bind("Cmd+C", Copy);
        bind("Cmd+V", Paste);
        bind("Cmd+A", SelectAll);
        bind("Cmd+F", Find);
        bind("Cmd+G", FindNext);
        bind("Cmd+Shift+G", FindPrev);
        bind("Cmd+Up", ScrollLineUp);
        bind("Cmd+Down", ScrollLineDown);
        bind("Shift+PageUp", ScrollPageUp);
        bind("Shift+PageDown", ScrollPageDown);
        bind("Cmd+plus", ZoomIn);
        bind("Cmd+=", ZoomIn);
        bind("Cmd+minus", ZoomOut);
        bind("Cmd+0", ZoomReset);
        bind("Cmd+Ctrl+F", Fullscreen);
        bind("Cmd+Option+S", ShowSizes);
        for n in 1..=9u8 {
            bind(&format!("Cmd+{n}"), GoToTab(n));
        }
        for n in 0..=9u8 {
            let slot = if n == 0 { 10 } else { n };
            bind(&format!("Cmd+Option+{n}"), FocusPane(slot));
        }
        Bindings { map }
    }
```

  `lookup` already retries `=` as `+` without Shift. Keep both `Cmd+plus` and `Cmd+=`, as Linux does.

- [ ] **Step 4: Update `docs/config.md`.** In "Actions and their default keys", change every table's header to
  `| Action | Linux | macOS |` and `|---|---|---|`. Then give every action row a third cell listing the macOS
  defaults in `KeyLabels::Mac` form (for example `` | `split_right` | `Ctrl+Alt+R` | `Cmd+D` | ``). Leave the cell
  empty (`| |`) where macOS has no default.
  - The test prints every wrong row with the expected text. Run it and copy the expected values.
  - The `focus_pane_N` and `go_to_tab_N` rows need the same treatment.
  - In the key-syntax notes above the tables, add one sentence: "On macOS the names `Cmd` (`Super`) and `Option`
    (`Alt`) work too, and the shortcuts list shows them."

- [ ] **Step 5: Run the tests to confirm they pass.**

Run: `cargo test --lib input::bindings && cargo test --lib window::modal`
Expected: PASS. `window::modal` covers the shortcuts panel, which uses `Display`.

- [ ] **Step 6: Run the gates and commit.**

```bash
cargo fmt && cargo clippy --all-targets --locked -- -D warnings \
  && cargo clippy --all-targets --locked --target x86_64-unknown-linux-gnu -- -D warnings && cargo test --locked
git add src/input/bindings.rs docs/config.md
git commit -m "feat: Mac-style default shortcuts on macOS

Cmd takes the place of Ctrl+Shift (after Terminal.app and iTerm2), so Ctrl stays
the shell's. Both tables are plain functions tested on every OS; key names show
as Cmd and Option on macOS and parse there too.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

### Task 2: Cmd+click opens links on macOS

**Files:**
- Modify: `src/window/links.rs:68-98` (`update_link_hover`, `link_press`, module doc)
- Modify: `src/window/mouse.rs:~261` (comment)
- Modify: `docs/config.md` "Links" section and the mouse-gestures note; `docs/design.md:21,202`

**Interfaces:**
- Produces: `fn link_modifier_held(&self) -> bool` (private, in `impl Window` in links.rs).

- [ ] **Step 1: Implement.** In `src/window/links.rs`, inside `impl Window`, add:

```rust
    /// The modifier that turns a click into "open the link": Ctrl, or Cmd on macOS, where
    /// Ctrl+click is a right click.
    fn link_modifier_held(&self) -> bool {
        if cfg!(target_os = "macos") {
            self.modifiers.super_key()
        } else {
            self.modifiers.control_key()
        }
    }
```

  Replace `self.modifiers.control_key()` with `self.link_modifier_held()` in `update_link_hover` and `link_press`.
  Update the doc comments there: "Nothing is searched unless Ctrl (Cmd on macOS) is held", and "A Ctrl+left press
  (Cmd on macOS)". Update the module doc line 2 the same way.

- [ ] **Step 2: Fix the docs.** In `docs/config.md` "Links", write "Ctrl+click (Cmd+click on macOS) opens a link…".
  In the mouse-gestures bullet, write "Ctrl+click links (Cmd+click on macOS)". In `docs/design.md`, change lines 21
  and 202 the same way.

- [ ] **Step 3: Run the gates and commit.** There's no new unit test: the choice is one `cfg!`, and the behaviour is
  covered by the manual check at the end of PR 1.

```bash
cargo fmt && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked
git add src/window/links.rs src/window/mouse.rs docs/config.md docs/design.md
git commit -m "feat: Cmd+click opens links on macOS

Ctrl+click is a right click on a Mac.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

### Task 3: The `quit` action

**Files:**
- Modify: `src/input/bindings.rs`: the `Quit` variant, its `TABLE` row, `every_plain_action_has_a_row`, and
  `Cmd+Q` in `macos_defaults`
- Modify: `src/ui/confirm.rs`: `CloseTarget::App` and `quit_message`
- Modify: `src/window/modal/content.rs`: `CloseTarget::App` arms in `closes_window`, `target_alive` and `running_in`
- Modify: `src/window/modal/mod.rs`: `pub fn ask_quit`
- Modify: `src/window/actions.rs`: `perform(Action::Quit)` and `close_target(CloseTarget::App)`
- Modify: `src/window/mod.rs`: `pub fn running_count`, `pub fn is_focused`
- Modify: `src/windows.rs`: `AppRequest::Quit { confirmed }`, `Windows::clear` and `Windows::pick`
- Modify: `src/app.rs`: `App::quit` and the `serve` arm
- Modify: `docs/config.md`: the `quit` row (Linux empty, macOS `Cmd+Q`), plus the "Windows" section

**Interfaces:**
- Produces: `Action::Quit` (config name `quit`, label `Quit`).
- Produces: `AppRequest::Quit { confirmed: bool }`.
- Produces: `Window::running_count(&self) -> usize`, `Window::ask_quit(&mut self, running: usize)` and
  `Window::is_focused(&self) -> bool`.
- Produces: `App::quit(&mut self, el: &ActiveEventLoop, from: WindowId, confirmed: bool)`, which task 4 calls.
- Produces: `Windows::pick(&self, prefer: impl Fn(&W) -> bool) -> Option<WindowId>` and `Windows::clear(&mut self)`.

- [ ] **Step 1: Write failing tests.**

  In `src/ui/confirm.rs` tests:

```rust
    #[test]
    fn quit_message_counts_processes() {
        assert_eq!(quit_message(1), "A process is still running. Quit anyway?");
        assert_eq!(quit_message(3), "3 processes are still running. Quit anyway?");
    }
```

  In `src/windows.rs` tests (they already have `Fake`, `has`, `wid` and `two()`):

```rust
    #[test]
    fn pick_prefers_then_falls_back() {
        let ws = two();
        assert_eq!(ws.pick(|w| w.0.contains(&20)), Some(wid(2)));
        // Nothing preferred: any window.
        assert!(matches!(ws.pick(|_| false), Some(id) if id == wid(1) || id == wid(2)));
        assert_eq!(Windows::<Fake>::default().pick(|_| true), None);
        let mut ws = two();
        ws.clear();
        assert!(ws.is_empty());
    }
```

  In `src/window/modal/content.rs` tests, after `running_counts_per_target` (reuse its fixture):

```rust
    #[test]
    fn app_target_counts_like_window() {
        let mut tabs = Tabs::new(Tab::new(1, (32, 32)));
        let mut t = Tab::new(2, (32, 32));
        t.tree.split(2, crate::layout::Orientation::Horizontal, 3);
        tabs.push(t);
        let panes: HashMap<PaneId, bool> = [(1, true), (2, false), (3, true)].into();
        let n = |t: CloseTarget| running_in(&t, &tabs, &panes, |&r| r);
        assert_eq!(n(CloseTarget::App), n(CloseTarget::Window));
        assert_eq!(n(CloseTarget::App), 2);
        assert!(closes_window(&CloseTarget::App, &tabs));
        assert!(target_alive(&CloseTarget::App, &tabs, &panes));
    }
```

  In `src/input/bindings.rs` tests, add `Quit` to the `plain` list in `every_plain_action_has_a_row`, and add to
  `macos_defaults`:

```rust
        assert_eq!(b.lookup(&k("Super+Q")), Some(Action::Quit));
```

  Add to `linux_defaults_are_tilix_like`:

```rust
        assert!(b.entries().iter().all(|(_, a)| *a != Action::Quit), "quit is unbound on Linux");
```

- [ ] **Step 2: Run the tests to confirm they fail.**

Run: `cargo test --lib`
Expected: compile errors (`Quit`, `quit_message`, `CloseTarget::App`, `pick` and `clear` are missing).

- [ ] **Step 3: Implement.**

  `src/input/bindings.rs`:
  - Add `Quit,` after `NewWindow,` in `enum Action`.
  - Add `(Quit, "quit", "Quit"),` after the `NewWindow` row in `TABLE`.
  - Add `bind("Cmd+Q", Quit);` in `macos_defaults`, after `NewWindow`.

  `src/ui/confirm.rs`. Add to `CloseTarget`:

```rust
    /// Every window: quitting montty.
    App,
```

  and next to `confirm_message`:

```rust
/// The prompt's text for quitting with `running` panes (in any window) that have a foreground job.
pub fn quit_message(running: usize) -> String {
    match running {
        1 => "A process is still running. Quit anyway?".into(),
        n => format!("{n} processes are still running. Quit anyway?"),
    }
}
```

  `src/window/modal/content.rs`: add `CloseTarget::App` to the arms that already handle `CloseTarget::Window`:
  - `closes_window`: `CloseTarget::Window | CloseTarget::App => true,`
  - `target_alive`: `CloseTarget::Window | CloseTarget::App => !tabs.is_empty(),`
  - `running_in`: `CloseTarget::Window | CloseTarget::App => tabs.iter().flat_map(|t| t.tree.panes()).collect(),`

  `src/window/mod.rs`, in `impl Window` near `take_requests`:

```rust
    /// Panes in this window with a foreground job (what a quit asks about).
    pub fn running_count(&self) -> usize {
        running_in(&CloseTarget::Window, &self.tabs, &self.panes, |p: &Pane| p.is_running())
    }

    pub fn is_focused(&self) -> bool {
        self.window_focused
    }
```

  `running_in` lives in `modal::content`; import it the way `modal/mod.rs` does.

  `src/window/modal/mod.rs`, next to `request_close`:

```rust
    /// Ask before quitting montty: `running` counts the panes with a foreground job in every
    /// window. The prompt's Close asks the app to quit (see `close_target`). A repeat while it
    /// is open does nothing.
    pub fn ask_quit(&mut self, running: usize) {
        if matches!(&self.modal, Modal::Confirm(c) if c.target == CloseTarget::App) {
            return;
        }
        let mut c = Confirm::new(CloseTarget::App, quit_message(running));
        let (w, h) = self.size_px();
        let (x, y) = self.mouse.pos();
        c.hover(x, y, w, h, &self.bar_metrics());
        self.set_modal(Modal::Confirm(c));
        // A quit from the Dock or the app menu may come while another app is in front.
        self.window.focus_window();
    }
```

  Import `quit_message` from `crate::ui::confirm`.

  `src/window/actions.rs`:
  - In `perform`, after `Action::NewWindow`, add:
    `Action::Quit => self.requests.push(AppRequest::Quit { confirmed: false }),`
  - In `close_target`, add:
    `CloseTarget::App => self.requests.push(AppRequest::Quit { confirmed: true }),`

  `src/windows.rs`. Add to `AppRequest`:

```rust
    /// Close every window and exit: `confirmed` once the user said yes (or nothing runs).
    Quit { confirmed: bool },
```

  and to `impl<W> Windows<W>`:

```rust
    /// The first window for which `prefer` holds, else any window.
    pub fn pick(&self, prefer: impl Fn(&W) -> bool) -> Option<WindowId> {
        self.map
            .iter()
            .find(|(_, w)| prefer(w))
            .or_else(|| self.map.iter().next())
            .map(|(id, _)| *id)
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }
```

  `src/app.rs`. In `serve`, handle `Quit` before building `init`:

```rust
    fn serve(&mut self, el: &ActiveEventLoop, from: WindowId, r: AppRequest) {
        let init = match r {
            AppRequest::Quit { confirmed } => return self.quit(el, from, confirmed),
            AppRequest::NewWindow { profile } => WindowInit::Default {
```

  and add to `impl App`:

```rust
    /// Quit: with a process running in any window (and not `confirmed`), window `from` asks
    /// first; else every window closes and the loop exits. `from` is the one saved for
    /// `restore_last_session`, as the last window closed would be.
    pub(crate) fn quit(&mut self, el: &ActiveEventLoop, from: WindowId, confirmed: bool) {
        let running: usize = self.windows.iter_mut().map(|(_, w)| w.running_count()).sum();
        if !confirmed && running > 0 {
            if let Some(w) = self.windows.get_mut(from) {
                w.ask_quit(running);
            }
            return;
        }
        if let Some(w) = self.windows.get_mut(from) {
            w.save_on_leave();
        }
        // Dropping each makes its GL context current to free its GL objects.
        self.windows.clear();
        el.exit();
    }
```

  `with_window` serves requests after `f`, and then exits when there are no windows left, so it needs no change.

- [ ] **Step 4: Update the docs.** In `docs/config.md`'s table (in the section that has `new_window`), add
  `` | `quit` | | `Cmd+Q` | ``. In "Windows", add: "`quit` closes every window at once; it asks first only when a
  process is still running in one of them (the count covers all windows). On macOS it is Cmd+Q, the app menu's Quit
  and the Dock's Quit."

- [ ] **Step 5: Run the tests to confirm they pass.**

Run: `cargo test --locked`
Expected: PASS, including `config_md_lists_all_actions` with the new row.

- [ ] **Step 6: Run the gates and commit.**

```bash
cargo fmt && cargo clippy --all-targets --locked -- -D warnings \
  && cargo clippy --all-targets --locked --target x86_64-unknown-linux-gnu -- -D warnings && cargo test --locked
git add -A src docs/config.md
git commit -m "feat: a quit action that closes every window, asking once

It asks only when a process runs in some window, counting them all, and saves
the requesting window for restore_last_session. Unbound on Linux; Cmd+Q on
macOS.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

### Task 4: macOS app-menu and Dock Quit go through montty's quit

**Files:**
- Create: `src/objc.rs`: the shared Objective-C runtime helper (macOS only)
- Create: `src/macos_quit.rs`: `applicationShouldTerminate:` on winit's app delegate
- Modify: `src/window/titlebar.rs`: use `crate::objc::add_method`
- Modify: `src/lib.rs`: `#[cfg(target_os = "macos")] pub mod objc; pub mod macos_quit;`
- Modify: `src/events.rs`: `UserEvent::Quit`
- Modify: `src/app.rs`: the `user_event` arm
- Modify: `src/main.rs`: install the hook after building the event loop
- Modify: `AGENTS.md`: the module list

**Interfaces:**
- Consumes: `App::quit(el, from, confirmed)` and `Windows::pick` from task 3.
- Produces: `crate::objc::add_method(class: &CStr, sel: &CStr, imp: *const c_void, types: &CStr) -> bool`, and
  `montty::macos_quit::install(proxy: EventLoopProxy<UserEvent>)`.

- [ ] **Step 1: Create `src/objc.rs`:**

```rust
//! The few Objective-C runtime calls montty needs on macOS to change how winit's classes answer
//! AppKit (see `window/titlebar.rs`, `macos_quit.rs`), declared against libobjc instead of pulling
//! in a binding crate.

use std::ffi::{CStr, c_char, c_void};

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> *mut c_void;
    fn sel_registerName(name: *const c_char) -> *const c_void;
    fn class_addMethod(
        class: *mut c_void,
        sel: *const c_void,
        imp: *const c_void,
        types: *const c_char,
    ) -> bool;
}

/// Add method `sel` (implemented by `imp`, with type encoding `types`) to class `class`. False
/// when the class doesn't exist (yet), or already has its own `sel`.
///
/// # Safety
/// `imp` must be an `extern "C"` function whose signature matches `types`.
pub(crate) unsafe fn add_method(class: &CStr, sel: &CStr, imp: *const c_void, types: &CStr) -> bool {
    // SAFETY: NUL-terminated names; the caller vouches for `imp` and `types`.
    unsafe {
        let class = objc_getClass(class.as_ptr());
        if class.is_null() {
            return false;
        }
        class_addMethod(class, sel_registerName(sel.as_ptr()), imp, types.as_ptr())
    }
}
```

- [ ] **Step 2: Refactor `src/window/titlebar.rs`** to use it. Delete its own `extern` block, and change
  `keep_bar_presses` to:

```rust
pub(super) fn keep_bar_presses() {
    let imp = cannot_move_window as extern "C" fn(*mut c_void, *const c_void) -> bool;
    // SAFETY: `cannot_move_window` matches TYPES (BOOL return, self and _cmd).
    let added = unsafe {
        crate::objc::add_method(c"WinitView", c"mouseDownCanMoveWindow", imp as *const c_void, TYPES)
    };
    // False on every window after the first (the class has the method by then); only a missing
    // class is worth a line, and add_method can't tell the two apart, so stay quiet.
    let _ = added;
}
```

  Keep `use std::ffi::c_void;`. Remove the `c_char` import if it is now unused.

- [ ] **Step 3: Create `src/macos_quit.rs`:**

```rust
//! macOS: the app menu's Quit (Cmd+Q), the Dock's Quit and logout send `terminate:`, which would
//! end the process with no close confirmation and no session autosave. montty answers the app
//! delegate's `applicationShouldTerminate:` with "cancel" and runs its own `quit` instead
//! (`UserEvent::Quit`). A logout is therefore cancelled once; the user logs out again after
//! montty has quit.

use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};

use winit::event_loop::EventLoopProxy;

use crate::events::UserEvent;

static PROXY: OnceLock<Mutex<EventLoopProxy<UserEvent>>> = OnceLock::new();

/// `NSTerminateCancel`.
const TERMINATE_CANCEL: usize = 0;

/// `-[WinitApplicationDelegate applicationShouldTerminate:]`.
extern "C" fn should_terminate(_this: *mut c_void, _cmd: *const c_void, _app: *mut c_void) -> usize {
    if let Some(p) = PROXY.get()
        && let Ok(p) = p.lock()
    {
        let _ = p.send_event(UserEvent::Quit);
    }
    TERMINATE_CANCEL
}

/// Route `terminate:` to montty's quit. Call once, after the event loop is built (winit
/// registers its delegate class then).
pub fn install(proxy: EventLoopProxy<UserEvent>) {
    let _ = PROXY.set(Mutex::new(proxy));
    let imp = should_terminate as extern "C" fn(*mut c_void, *const c_void, *mut c_void) -> usize;
    // SAFETY: `should_terminate` matches "Q@:@" (NSUInteger return; self, _cmd, sender).
    let added = unsafe {
        crate::objc::add_method(
            c"WinitApplicationDelegate",
            c"applicationShouldTerminate:",
            imp as *const c_void,
            c"Q@:@",
        )
    };
    if !added {
        eprintln!("montty: cannot hook the app menu's Quit; Cmd+Q quits without asking");
    }
}
```

- [ ] **Step 4: Wire it up.**
  - `src/lib.rs`: add `#[cfg(target_os = "macos")] pub mod macos_quit;` and `#[cfg(target_os = "macos")] mod objc;`,
    in alphabetical order with the others.
  - `src/events.rs`: add a variant to `UserEvent`:

```rust
    /// macOS: the app menu's or the Dock's Quit (see `macos_quit`).
    Quit,
```

  - `src/app.rs` `user_event`:

```rust
            UserEvent::Quit => {
                if let Some(id) = self.windows.pick(|w| w.is_focused()) {
                    self.quit(el, id, false);
                }
            }
```

  - `src/main.rs`, after `montty::device_events::ignore(&event_loop);`:

```rust
    #[cfg(target_os = "macos")]
    montty::macos_quit::install(event_loop.create_proxy());
```

  - `AGENTS.md` module list: add `` `macos_quit.rs` (macOS): the app menu's / Dock's Quit runs montty's `quit` `` and
    `` `objc.rs` (macOS): the Objective-C runtime calls for that and `window/titlebar.rs` ``.

- [ ] **Step 5: Run the gates.** `UserEvent` must stay `Send` for the `OnceLock<Mutex<EventLoopProxy>>` static. If
  the compiler says `EventLoopProxy<UserEvent>` isn't `Sync`/`Send`, keep the `Mutex` (already there). If it is still
  not `Send`, report back instead of adding `unsafe impl`.

```bash
cargo fmt && cargo clippy --all-targets --locked -- -D warnings \
  && cargo clippy --all-targets --locked --target x86_64-unknown-linux-gnu -- -D warnings && cargo test --locked
```

- [ ] **Step 6: Commit.**

```bash
git add -A src AGENTS.md
git commit -m "feat: macOS Quit from the app menu and Dock asks like montty's quit

terminate: would end the process with no confirmation or autosave; the app
delegate now cancels it and posts UserEvent::Quit. The Objective-C runtime
calls move to src/objc.rs, shared with window/titlebar.rs.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 7: Open PR 1 and run the manual check.** Push `macos-shortcuts`, open the PR against `main` and wait
  for both CI jobs. Then ask the human to run `cargo run --release` on the Mac and check:
  1. Cmd+C / Cmd+V copy and paste. Cmd+T, Cmd+W, Cmd+Shift+W, Cmd+N, Cmd+D and Cmd+Shift+D work. Cmd+1-9 switch
     tabs, and Cmd+= / Cmd+- / Cmd+0 zoom.
  2. Ctrl+C, Ctrl+R and Ctrl+A still reach the shell.
  3. The ≡ menu → Keyboard shortcuts shows `Cmd` and `Option` names.
  4. Cmd+click opens a URL (try `echo https://example.com`), and a plain click doesn't.
  5. Cmd+Q with an idle shell quits at once. Then run `sleep 100` and press Cmd+Q: the prompt says "A process is
     still running. Quit anyway?", Cancel keeps montty, and Close quits. Do the same from the Dock icon's Quit.
  6. With two windows, and `sleep 100` in the second one, Cmd+Q in the first counts 1 process.

  Merge only after the human confirms.

---

## PR 2: Option as Alt (task 5)

Branch: `macos-option-as-alt`.

### Task 5: `option_as_alt` in `config.toml`

**Files:**
- Modify: `src/config/global.rs`: the `OptionAsAlt` enum, the field, the `Raw` field, parsing, and tests
- Modify: `src/window/mod.rs`: the macOS attrs block in `Window::new`, and `reload_config`
- Modify: `docs/config.md`: the `config.toml` options list and its example

**Interfaces:**
- Produces: `pub enum OptionAsAlt { Left, Right, Both, None }` in `config::global`, and
  `GlobalConfig::option_as_alt: OptionAsAlt` (default `Left`).

- [ ] **Step 1: Write failing tests** in `src/config/global.rs` tests, following
  `kitty_keyboard_default_on_and_parse`:

```rust
    #[test]
    fn option_as_alt_defaults_left_and_parses() {
        let p = |s: &str| GlobalConfig::parse(s, Path::new("config.toml"));
        assert_eq!(p("").unwrap().option_as_alt, OptionAsAlt::Left);
        for (v, want) in [
            ("left", OptionAsAlt::Left),
            ("right", OptionAsAlt::Right),
            ("both", OptionAsAlt::Both),
            ("none", OptionAsAlt::None),
        ] {
            assert_eq!(p(&format!("option_as_alt = \"{v}\"")).unwrap().option_as_alt, want);
        }
        let e = p("\noption_as_alt = \"yes\"").unwrap_err();
        assert_eq!(e.line, Some(2));
        assert!(e.msg.contains("\"left\", \"right\", \"both\" or \"none\""), "{}", e.msg);
    }
```

- [ ] **Step 2: Run the test to confirm it fails.**

Run: `cargo test --lib config::global`
Expected: compile errors (`OptionAsAlt` doesn't exist).

- [ ] **Step 3: Implement** in `src/config/global.rs`:

```rust
/// Which Option key (macOS) acts as Alt in the terminal; the other types its special
/// characters. Ignored elsewhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionAsAlt {
    Left,
    Right,
    Both,
    None,
}
```

  - Add `pub option_as_alt: OptionAsAlt,` to `GlobalConfig`, with the doc comment
    `/// macOS: which Option key is Alt (see OptionAsAlt).`
  - Add `option_as_alt: OptionAsAlt::Left,` to `Default`.
  - Add `option_as_alt: Option<Spanned<String>>,` to `Raw`.
  - In `parse`, after `kitty_keyboard`:

```rust
        if let Some(v) = raw.option_as_alt {
            cfg.option_as_alt = match v.get_ref().as_str() {
                "left" => OptionAsAlt::Left,
                "right" => OptionAsAlt::Right,
                "both" => OptionAsAlt::Both,
                "none" => OptionAsAlt::None,
                other => {
                    return Err(err(
                        Some(v.span().start),
                        format!(
                            "option_as_alt must be \"left\", \"right\", \"both\" or \"none\", got {other:?}"
                        ),
                    ));
                }
            };
        }
```

  `src/window/mod.rs`. Add a macOS-only helper near `cursor_style`:

```rust
#[cfg(target_os = "macos")]
fn winit_option_as_alt(o: crate::config::global::OptionAsAlt) -> winit::platform::macos::OptionAsAlt {
    use crate::config::global::OptionAsAlt as C;
    use winit::platform::macos::OptionAsAlt as W;
    match o {
        C::Left => W::OnlyLeft,
        C::Right => W::OnlyRight,
        C::Both => W::Both,
        C::None => W::None,
    }
}
```

  In `Window::new`'s macOS attrs block (the one with `with_fullsize_content_view`), add
  `.with_option_as_alt(winit_option_as_alt(store.global().option_as_alt))`.

  In `reload_config`, inside `if report.global_changed { ... }`, add:

```rust
            #[cfg(target_os = "macos")]
            {
                use winit::platform::macos::WindowExtMacOS;
                self.window
                    .set_option_as_alt(winit_option_as_alt(self.store.global().option_as_alt));
            }
```

- [ ] **Step 4: Update the docs.** In `docs/config.md`'s `config.toml` example and options list, add
  `option_as_alt = "left"   # macOS: "left", "right", "both" or "none": which Option key is Alt` and one sentence of
  explanation: "Alt for the shell (Alt+B / Alt+F, Meta in Emacs); the other Option key types special characters such
  as `@` on a Czech layout. Applies live."

- [ ] **Step 5: Run the gates and commit.**

```bash
cargo fmt && cargo clippy --all-targets --locked -- -D warnings \
  && cargo clippy --all-targets --locked --target x86_64-unknown-linux-gnu -- -D warnings && cargo test --locked
git add src/config/global.rs src/window/mod.rs docs/config.md
git commit -m "feat: option_as_alt chooses which Option key is Alt on macOS

Default left: Alt for the shell on the left, the layout's special characters on
the right, as iTerm2 and kitty offer. Applies live on reload.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 6: Open PR 2 and run the manual check.** Push, open the PR and wait for CI. The human checks on the
  Mac:
  - In bash, left Option+B / Option+F jump by words, and right Option types a special character.
  - With `option_as_alt = "none"` saved in `~/.config/montty/config.toml`, left Option types the character, live.

  Merge after confirmation.

---

## PR 3: Text size in points (task 6)

Branch: `macos-font-points`.

### Task 6: `PX_PER_PT` per OS

**Files:**
- Modify: `src/font/mod.rs:~294` (`FontSet::load`) and tests

**Interfaces:**
- Produces: `fn font_px(size_pt: f32, scale: f64) -> f32` (private to `font`).

- [ ] **Step 1: Write a failing test** in `src/font/mod.rs` tests:

```rust
    #[test]
    fn points_to_pixels_per_os() {
        // Linux: 96 dpi; macOS: a logical pixel is a point.
        let want = if cfg!(target_os = "macos") { 24.0 } else { 32.0 };
        assert_eq!(font_px(12.0, 2.0), want);
        assert_eq!(font_px(12.0, 1.0), want / 2.0);
    }
```

  Check the type of `FontSpec::size_pt` (`grep -n "size_pt" src/font/mod.rs`). If it isn't `f32`, use that type in
  `font_px`'s signature and in the test literal.

- [ ] **Step 2: Run the test to confirm it fails.**

Run: `cargo test --lib font::tests::points_to_pixels_per_os`
Expected: compile error (`font_px` doesn't exist).

- [ ] **Step 3: Implement.** Above `impl FontSet`:

```rust
/// Pixels per point at scale 1: X11 and Wayland lay out at 96 dpi; a macOS logical pixel is
/// already a point, so 11 pt matches other Mac apps.
#[cfg(not(target_os = "macos"))]
const PX_PER_PT: f64 = 96.0 / 72.0;
#[cfg(target_os = "macos")]
const PX_PER_PT: f64 = 1.0;

fn font_px(size_pt: f32, scale: f64) -> f32 {
    (f64::from(size_pt) * PX_PER_PT * scale) as f32
}
```

  In `FontSet::load`, replace `let px = (spec.size_pt as f64 * 96.0 / 72.0 * scale) as f32;` with
  `let px = font_px(spec.size_pt, scale);`.

- [ ] **Step 4: Run all the font tests.** `metrics_sane` expects `cell_width` in `5..=12`. On macOS an 11 pt Menlo
  cell is about 7 px, so it should still pass. If it fails, widen the range in the test for macOS (a smaller font is
  expected there) and add a comment saying why.

Run: `cargo test --lib font`
Expected: PASS.

- [ ] **Step 5: Run the gates and commit.**

```bash
cargo fmt && cargo clippy --all-targets --locked -- -D warnings \
  && cargo clippy --all-targets --locked --target x86_64-unknown-linux-gnu -- -D warnings && cargo test --locked
git add src/font/mod.rs
git commit -m "fix: font sizes are points on macOS

A macOS logical pixel is already a point; the 96/72 factor made text about a
third larger than in other Mac apps.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 6: Open PR 3 and run the manual check.** The human opens Terminal.app and montty side by side, both at
  11 pt Menlo, and checks that the letters are the same height. Merge after confirmation.

---

## PR 4: Starting from Finder (task 7)

Branch: `macos-launch-env`.

### Task 7: Shell, locale and start directory when montty's own environment is thin

**Files:**
- Create: `src/launch.rs`
- Modify: `src/lib.rs` (`pub mod launch;`)
- Modify: `src/window/actions.rs:~215` (`spawn_pane`) and `src/window/mod.rs:~1775` (`FirstPane::spawn`): use
  `launch::env_shell()` and `launch::default_start_dir()`
- Modify: `src/term/pane.rs` (`Pane::spawn` env): the macOS locale override
- Modify: `AGENTS.md` (module list), `docs/config.md` ("Working directory of a new pane", and the profile `shell`
  key's description)

**Interfaces:**
- Produces: `launch::pick_shell(env: Option<String>, account: impl FnOnce() -> Option<String>) -> Option<String>`,
  `launch::env_shell() -> Option<String>`, `launch::account_shell() -> Option<String>`,
  `launch::locale_override(get: impl Fn(&str) -> Option<String>) -> Option<(&'static str, &'static str)>`,
  `launch::start_dir(process_cwd: Option<&Path>, home: Option<PathBuf>) -> Option<PathBuf>` and
  `launch::default_start_dir() -> Option<PathBuf>`.

- [ ] **Step 1: Write `src/launch.rs` with its tests first** (the functions as `todo!()`, so the tests compile and
  fail):

```rust
//! What a pane takes from montty's own environment, which is thin when Finder or the Dock
//! started it: no `$SHELL`, no locale, and `/` as the working directory.

use std::path::{Path, PathBuf};

/// `$SHELL` when set and non-empty, else the account's shell.
pub fn pick_shell(env: Option<String>, account: impl FnOnce() -> Option<String>) -> Option<String> {
    todo!()
}

/// The shell for panes that don't name one (see `pick_shell`).
pub fn env_shell() -> Option<String> {
    pick_shell(std::env::var("SHELL").ok(), account_shell)
}

/// The login shell in the user's account record (`getpwuid_r`).
pub fn account_shell() -> Option<String> {
    todo!()
}

/// `LC_CTYPE=UTF-8` when none of `LC_ALL`, `LC_CTYPE` and `LANG` is set, as Terminal.app
/// does; without it, bash and vim treat input as ASCII. Applied on macOS only.
pub fn locale_override(get: impl Fn(&str) -> Option<String>) -> Option<(&'static str, &'static str)> {
    todo!()
}

/// `home` when montty runs in `/` (a Finder or Dock launch): a pane with no directory of its
/// own starts there rather than in `/`.
pub fn start_dir(process_cwd: Option<&Path>, home: Option<PathBuf>) -> Option<PathBuf> {
    todo!()
}

/// `start_dir` for this process, on macOS; None elsewhere (a desktop launcher starts in $HOME).
pub fn default_start_dir() -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    start_dir(
        std::env::current_dir().ok().as_deref(),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_prefers_env_then_account() {
        let account = || Some("/opt/homebrew/bin/bash".to_string());
        assert_eq!(pick_shell(Some("/bin/zsh".into()), account).as_deref(), Some("/bin/zsh"));
        assert_eq!(pick_shell(Some(String::new()), account).as_deref(), Some("/opt/homebrew/bin/bash"));
        assert_eq!(pick_shell(None, account).as_deref(), Some("/opt/homebrew/bin/bash"));
        assert_eq!(pick_shell(None, || None), None);
    }

    #[test]
    fn account_shell_is_an_absolute_path() {
        // Every test machine's account has a shell.
        let s = account_shell().expect("account shell");
        assert!(s.starts_with('/'), "{s}");
    }

    #[test]
    fn locale_only_when_none_is_set() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
        };
        assert_eq!(locale_override(env(&[])), Some(("LC_CTYPE", "UTF-8")));
        assert_eq!(locale_override(env(&[("LANG", "")])), Some(("LC_CTYPE", "UTF-8")));
        assert_eq!(locale_override(env(&[("LANG", "cs_CZ.UTF-8")])), None);
        assert_eq!(locale_override(env(&[("LC_CTYPE", "UTF-8")])), None);
        assert_eq!(locale_override(env(&[("LC_ALL", "C")])), None);
    }

    #[test]
    fn start_dir_only_from_root() {
        let home = Some(PathBuf::from("/Users/me"));
        assert_eq!(start_dir(Some(Path::new("/")), home.clone()), home);
        assert_eq!(start_dir(Some(Path::new("/tmp")), home.clone()), None);
        assert_eq!(start_dir(None, home), None);
        assert_eq!(start_dir(Some(Path::new("/")), None), None);
    }
}
```

  Add `pub mod launch;` to `src/lib.rs`.

- [ ] **Step 2: Run the tests to confirm they fail.**

Run: `cargo test --lib launch`
Expected: FAIL (panics on `todo!()`).

- [ ] **Step 3: Implement the four functions:**

```rust
pub fn pick_shell(env: Option<String>, account: impl FnOnce() -> Option<String>) -> Option<String> {
    env.filter(|s| !s.is_empty()).or_else(account)
}

pub fn account_shell() -> Option<String> {
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut pw = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut out: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: `pw` and `buf` are valid for writes of their sizes; on success `out` points to
    // `pw`, whose strings point into `buf`, and both outlive the reads below.
    let rc = unsafe {
        libc::getpwuid_r(libc::getuid(), pw.as_mut_ptr(), buf.as_mut_ptr(), buf.len(), &mut out)
    };
    if rc != 0 || out.is_null() {
        return None;
    }
    // SAFETY: as above; `pw_shell` is NUL-terminated (or null for no shell).
    let shell = unsafe {
        let p = (*out).pw_shell;
        if p.is_null() {
            return None;
        }
        std::ffi::CStr::from_ptr(p)
    };
    shell.to_str().ok().filter(|s| !s.is_empty()).map(str::to_string)
}

pub fn locale_override(get: impl Fn(&str) -> Option<String>) -> Option<(&'static str, &'static str)> {
    let set = |k: &str| get(k).is_some_and(|v| !v.is_empty());
    (!["LC_ALL", "LC_CTYPE", "LANG"].into_iter().any(set)).then_some(("LC_CTYPE", "UTF-8"))
}

pub fn start_dir(process_cwd: Option<&Path>, home: Option<PathBuf>) -> Option<PathBuf> {
    if process_cwd == Some(Path::new("/")) { home } else { None }
}
```

- [ ] **Step 4: Run the tests to confirm they pass.**

Run: `cargo test --lib launch`
Expected: PASS.

- [ ] **Step 5: Wire it in.**
  - `src/window/actions.rs` `spawn_pane`: replace `let shell = std::env::var("SHELL").ok();` with
    `let shell = crate::launch::env_shell();`. In the `pane_options(...)` call, pass
    `cwd.or_else(crate::launch::default_start_dir)` instead of `cwd`.
  - `src/window/mod.rs` `FirstPane::spawn`: likewise, `let shell = crate::launch::env_shell();`, and pass
    `crate::launch::default_start_dir()` instead of `None` for `cwd`.
  - `src/term/pane.rs` `Pane::spawn`: after the `env:` collection is built into `pty_opts`, add:

```rust
        let mut pty_opts = pty_opts;
        #[cfg(target_os = "macos")]
        if let Some((k, v)) = crate::launch::locale_override(|k| std::env::var(k).ok()) {
            pty_opts.env.insert(k.to_string(), v.to_string());
        }
```

    (Or make the `let pty_opts` binding `mut` in place, which is cleaner. `tty::Options::env` is a
    `HashMap<String, String>`; check with `grep -n "pub env" vendor/alacritty_terminal/src/tty/mod.rs`.)
  - Update the `PaneOptions::command` doc in `pane.rs` ("`None` runs `$SHELL`…"); it now reads "the account's shell
    when `$SHELL` is unset".
  - `docs/config.md`: in "Working directory of a new pane", add "On macOS, a pane with no directory of its own
    starts in `$HOME` when montty was started from Finder or the Dock." In the profile `shell` key's row, change
    the default to "`$SHELL`, else the account's shell, else `/bin/sh`". Also change the template comment in
    `src/config/profile.rs` (`# shell = "/bin/zsh"        # default: $SHELL, else /bin/sh`) the same way.
  - `AGENTS.md` module list: `` `launch.rs`: what panes take from montty's own environment (shell, macOS locale and
    start dir) ``.

- [ ] **Step 6: Run the gates and commit.**

```bash
cargo fmt && cargo clippy --all-targets --locked -- -D warnings \
  && cargo clippy --all-targets --locked --target x86_64-unknown-linux-gnu -- -D warnings && cargo test --locked
git add -A src AGENTS.md docs/config.md
git commit -m "feat: sane panes when montty starts from Finder or the Dock

No \$SHELL falls back to the account's shell (on every OS), no locale gets
LC_CTYPE=UTF-8 on macOS, and a launch in / starts panes in \$HOME on macOS.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 7: Open PR 4.** The manual check for this PR happens after PR 6, when there is a `montty.app` to start
  from Finder. Until then, the human can test with `env -i HOME="$HOME" PATH=/usr/bin:/bin
  ./target/release/montty` started from `/` (`cd / && …`): the prompt should appear, `locale` should show
  `LC_CTYPE="UTF-8"`, and `pwd` should print the home folder.

---

## PR 5: Notifications (task 8)

Branch: `macos-notifications`.

### Task 8: `osascript` on macOS

**Files:**
- Modify: `src/notify.rs`
- Modify: `src/config/global.rs` (the `desktop_notifications` doc comment mentions `notify-send`) and
  `docs/config.md` (`desktop_notifications`)

**Interfaces:**
- Produces: `fn notifier_command(title: &str, body: &str) -> (&'static str, Vec<String>)` (private).

- [ ] **Step 1: Write failing tests** in `src/notify.rs` tests:

```rust
    #[test]
    fn notifier_command_per_os() {
        let (prog, args) = notifier_command("<b>t</b>", "-x \"q\"");
        if cfg!(target_os = "macos") {
            assert_eq!(prog, "osascript");
            // Title and body are arguments after `--`, never part of the script (no injection,
            // and a leading '-' is not an option).
            let n = args.len();
            assert_eq!(&args[n - 3..], ["--", "<b>t</b>", "-x \"q\""]);
            assert!(args.iter().take(n - 3).all(|a| !a.contains("<b>")));
        } else {
            assert_eq!(prog, "notify-send");
            assert_eq!(args.last().map(String::as_str), Some("-x &quot;q&quot;"));
            assert!(args.contains(&"&lt;b&gt;t&lt;/b&gt;".to_string()));
        }
    }
```

  Check how `escape_markup` escapes `"` (see the `markup_is_escaped` test). If it leaves `"` alone, change the
  Linux expectation to `"-x \"q\""`.

- [ ] **Step 2: Run the test to confirm it fails.**

Run: `cargo test --lib notify`
Expected: compile error (`notifier_command` doesn't exist).

- [ ] **Step 3: Implement.** In `src/notify.rs`:

```rust
/// The program that shows one notification, and its arguments.
fn notifier_command(title: &str, body: &str) -> (&'static str, Vec<String>) {
    if cfg!(target_os = "macos") {
        let script = [
            "-e",
            "on run argv",
            "-e",
            "display notification (item 2 of argv) with title (item 1 of argv)",
            "-e",
            "end run",
            "--",
        ];
        let mut args: Vec<String> = script.iter().map(|s| s.to_string()).collect();
        args.extend([title.to_string(), body.to_string()]);
        ("osascript", args)
    } else {
        let args = ["-a", "montty", "-i", "montty", "--"]
            .iter()
            .map(|s| s.to_string())
            .chain([escape_markup(title), escape_markup(body)])
            .collect();
        ("notify-send", args)
    }
}
```

  In `send`, replace the `Command::new("notify-send").args([...])` with:

```rust
        let (prog, args) = notifier_command(title, body);
        let spawned = Command::new(prog)
            .args(&args)
```

  Change the not-found message to `eprintln!("montty: {prog} not found; desktop notifications disabled");`. Update
  the module doc to "Desktop notifications through `notify-send` (Linux) or `osascript` (macOS), with no extra
  crate", and `send`'s doc to say "the notifier" instead of `notify-send`. If clippy then reports `escape_markup` as
  unused on macOS, note that it is still used in the `else` branch, which compiles on every OS because this uses
  `cfg!`. That is why `cfg!` is used here rather than `#[cfg]`.

- [ ] **Step 4: Update the docs.** In `docs/config.md` `desktop_notifications`, write "…through `notify-send`, or
  `osascript` on macOS (shown as coming from Script Editor)". Change the `GlobalConfig::desktop_notifications` doc
  comment the same way.

- [ ] **Step 5: Run the gates and commit.**

```bash
cargo fmt && cargo clippy --all-targets --locked -- -D warnings \
  && cargo clippy --all-targets --locked --target x86_64-unknown-linux-gnu -- -D warnings && cargo test --locked
git add src/notify.rs src/config/global.rs docs/config.md
git commit -m "feat: desktop notifications on macOS through osascript

Title and body travel as arguments after --, never inside the script, so pane
text can't inject AppleScript.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 6: Open PR 5 and run the manual check.** The human sets `desktop_notifications = true` in
  `config.toml`, focuses another app, and runs `sleep 2; printf '\e]9;hi from montty\a'` in a montty pane (switching
  away during the sleep). A notification should appear. If macOS asks for permission for Script Editor, allow it.
  Merge after confirmation.

---

## PR 6: App bundle and release (tasks 9-10)

Branch: `macos-bundle`.

### Task 9: `packaging/macos/build-app.sh`

**Files:**
- Create: `packaging/macos/Info.plist`
- Create: `packaging/macos/build-app.sh` (executable)

**Interfaces:**
- Produces: `packaging/macos/build-app.sh <version> <binary> <out-dir>`, which writes
  `<out-dir>/montty-<version>-macos.dmg`.

- [ ] **Step 1: Create `packaging/macos/Info.plist`** (`@VERSION@` is replaced by the script):

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>montty</string>
    <key>CFBundleDisplayName</key>
    <string>montty</string>
    <key>CFBundleIdentifier</key>
    <string>io.github.idahomst.montty</string>
    <key>CFBundleExecutable</key>
    <string>montty</string>
    <key>CFBundleIconFile</key>
    <string>montty</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>@VERSION@</string>
    <key>CFBundleVersion</key>
    <string>@VERSION@</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.developer-tools</string>
</dict>
</plist>
```

- [ ] **Step 2: Create `packaging/macos/build-app.sh`:**

```bash
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
```

  Run `chmod +x packaging/macos/build-app.sh`.

- [ ] **Step 3: Test it locally on the Mac.**

```bash
cargo build --release --locked
packaging/macos/build-app.sh "$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)" target/release/montty /tmp/montty-dmg
hdiutil attach -nobrowse -mountpoint /tmp/montty-mnt /tmp/montty-dmg/montty-*-macos.dmg
plutil -lint /tmp/montty-mnt/montty.app/Contents/Info.plist
codesign --verify --verbose /tmp/montty-mnt/montty.app
ls /tmp/montty-mnt/montty.app/Contents/Resources/montty.icns
hdiutil detach /tmp/montty-mnt
```

Expected: `plutil` prints `OK`, `codesign` prints `valid on disk`, and the `.icns` exists.

- [ ] **Step 4: Commit.**

```bash
git add packaging/macos
git commit -m "packaging: montty.app in a .dmg for macOS

Info.plist (io.github.idahomst.montty, macOS 11+), an icns from the hicolor
PNGs, ad-hoc signing, and a dmg with an Applications link.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

### Task 10: Release job, install docs and README

**Files:**
- Modify: `.github/workflows/release.yml`: the new `macos` job, plus the `release` job's `needs` and downloads
- Modify: `docs/install.md`: the macOS section
- Modify: `README.md`: the platform line (`README.md:24`) and the install section
- Modify: `AGENTS.md`: the "Packaging and release" section

**Interfaces:**
- Consumes: `packaging/macos/build-app.sh` (task 9), and the `build` job's `outputs.version` in `release.yml`.

- [ ] **Step 1: Add the job** to `.github/workflows/release.yml`, after the `arch` job:

```yaml
  macos:
    needs: build
    runs-on: macos-15
    env:
      MACOSX_DEPLOYMENT_TARGET: "11.0"
    steps:
      - uses: actions/checkout@v7

      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: aarch64-apple-darwin, x86_64-apple-darwin

      - uses: Swatinem/rust-cache@v2

      - name: Build (Apple Silicon and Intel)
        run: |
          cargo build --release --locked --target aarch64-apple-darwin
          cargo build --release --locked --target x86_64-apple-darwin

      - name: Universal binary
        run: |
          lipo -create -output montty \
            target/aarch64-apple-darwin/release/montty \
            target/x86_64-apple-darwin/release/montty
          lipo -info montty

      - name: App bundle and dmg
        run: packaging/macos/build-app.sh "${{ needs.build.outputs.version }}" montty out

      - uses: actions/upload-artifact@v7
        with:
          name: macos
          path: out/*.dmg
          if-no-files-found: error
```

  In the `release` job, change `needs: [build, arch]` to `needs: [build, arch, macos]`, and add a download step next
  to the `arch` one:

```yaml
      - uses: actions/download-artifact@v8
        with:
          name: macos
          path: files
```

  Check how the `softprops/action-gh-release` step lists its files (for example `files: files/*`), so that the
  `.dmg` is included. Adjust its glob if it lists file types one by one.

- [ ] **Step 2: Update `docs/install.md`.** Add a `## macOS` section before "## From a source build":

```markdown
## macOS

Download `montty-<version>-macos.dmg` from the [latest release](https://github.com/idahomst/montty/releases/latest),
open it and drag `montty.app` to Applications. It runs on Apple Silicon and Intel Macs with macOS 11 or newer.

The app is not signed with an Apple Developer ID yet, so the first launch is blocked: right-click `montty.app` in
Applications, choose **Open**, then **Open** again. Or clear the download flag once:

    xattr -dr com.apple.quarantine /Applications/montty.app

From source: `cargo build --release`, then `packaging/macos/build-app.sh <version> target/release/montty <out-dir>`
builds the same `.dmg`, or run `target/release/montty` directly.
```

  In "From a source build", note that the same `cargo build --release` works on macOS (Xcode Command Line Tools are
  enough).

- [ ] **Step 3: Update README.md and AGENTS.md.** In README.md, replace "macOS and Windows are not supported yet."
  with "Linux (X11, Wayland) and macOS 11+. Windows is not supported yet." Add a macOS line to the install section
  that points to `docs/install.md#macos`. In AGENTS.md:
  - Change the intro's "macOS builds and passes the tests (CI runs both), but is not a supported app yet" to "macOS
    is supported too (CI runs both)".
  - In "Packaging and release", add "… and a macOS `.dmg` (`packaging/macos/build-app.sh`, universal binary, ad-hoc
    signed)".

- [ ] **Step 4: Commit and push.** Then run the release workflow by hand on the branch:

```bash
git add .github/workflows/release.yml docs/install.md README.md AGENTS.md
git commit -m "ci(release): a universal macOS dmg; docs: installing on macOS

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
git push -u origin macos-bundle
gh workflow run release.yml --ref macos-bundle
gh run watch "$(gh run list --workflow release.yml --branch macos-bundle --limit 1 --json databaseId -q '.[0].databaseId')"
gh run download "$(gh run list --workflow release.yml --branch macos-bundle --limit 1 --json databaseId -q '.[0].databaseId')" -n macos -D /tmp/montty-release-dmg
```

Expected: the `build`, `arch` and `macos` jobs pass, the `release` job is skipped (no tag), and the `.dmg` downloads.

- [ ] **Step 5: Open PR 6 and run the manual check.** This is the final check for the whole spec. The human:
  1. Opens the downloaded `.dmg`, drags montty.app to Applications, and goes through the right-click → Open step.
  2. Starts montty from Finder or the Dock. Checks that the prompt works (`create_prompt` defined), that `locale`
     shows UTF-8 and that `pwd` is the home folder. This is also PR 4's check.
  3. Checks the icon in the Dock and the app switcher.
  4. Quits with Cmd+Q and from the Dock icon. With `restore_last_session = true`, the next start restores the window.

  Merge after confirmation. The next `v*` tag publishes the `.dmg` together with the Linux packages.
