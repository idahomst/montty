use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};

use super::Mods;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum KeyName {
    /// Always lowercase.
    Char(char),
    Named(NamedKey),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct KeyCombo {
    pub mods: Mods,
    pub key: KeyName,
}

impl KeyCombo {
    /// Build a combo from a winit key event (key without modifiers applied) and the active modifiers.
    pub fn from_event(key_without_mods: &Key, mods: Mods) -> Option<KeyCombo> {
        let key = match key_without_mods {
            Key::Character(s) => {
                let mut chars = s.chars();
                let c = chars.next()?;
                if chars.next().is_some() {
                    return None;
                }
                KeyName::Char(c.to_lowercase().next()?)
            }
            Key::Named(n) => KeyName::Named(*n),
            _ => return None,
        };
        Some(KeyCombo { mods, key })
    }
}

fn parse_named(name: &str) -> Option<NamedKey> {
    use NamedKey::*;
    Some(match name {
        "up" => ArrowUp,
        "down" => ArrowDown,
        "left" => ArrowLeft,
        "right" => ArrowRight,
        "pageup" | "page_up" => PageUp,
        "pagedown" | "page_down" => PageDown,
        "insert" => Insert,
        "delete" => Delete,
        "home" => Home,
        "end" => End,
        "tab" => Tab,
        "escape" => Escape,
        "f1" => F1,
        "f2" => F2,
        "f3" => F3,
        "f4" => F4,
        "f5" => F5,
        "f6" => F6,
        "f7" => F7,
        "f8" => F8,
        "f9" => F9,
        "f10" => F10,
        "f11" => F11,
        "f12" => F12,
        _ => return None,
    })
}

/// The display name for a named key; the inverse of `parse_named` (modulo aliases).
fn named_label(key: NamedKey) -> Option<&'static str> {
    use NamedKey::*;
    Some(match key {
        ArrowUp => "Up",
        ArrowDown => "Down",
        ArrowLeft => "Left",
        ArrowRight => "Right",
        PageUp => "PageUp",
        PageDown => "PageDown",
        Insert => "Insert",
        Delete => "Delete",
        Home => "Home",
        End => "End",
        Tab => "Tab",
        Escape => "Escape",
        F1 => "F1",
        F2 => "F2",
        F3 => "F3",
        F4 => "F4",
        F5 => "F5",
        F6 => "F6",
        F7 => "F7",
        F8 => "F8",
        F9 => "F9",
        F10 => "F10",
        F11 => "F11",
        F12 => "F12",
        _ => return None,
    })
}

/// `'0'..='9'` for a digit-row key (by physical position).
fn physical_digit(physical: PhysicalKey) -> Option<char> {
    let PhysicalKey::Code(code) = physical else {
        return None;
    };
    Some(match code {
        KeyCode::Digit0 => '0',
        KeyCode::Digit1 => '1',
        KeyCode::Digit2 => '2',
        KeyCode::Digit3 => '3',
        KeyCode::Digit4 => '4',
        KeyCode::Digit5 => '5',
        KeyCode::Digit6 => '6',
        KeyCode::Digit7 => '7',
        KeyCode::Digit8 => '8',
        KeyCode::Digit9 => '9',
        _ => return None,
    })
}

/// "Ctrl+Alt+R", "Shift+PageUp", "Ctrl+plus": parses back to the same combo.
impl fmt::Display for KeyCombo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (on, name) in [
            (self.mods.ctrl, "Ctrl"),
            (self.mods.alt, "Alt"),
            (self.mods.shift, "Shift"),
            (self.mods.logo, "Super"),
        ] {
            if on {
                write!(f, "{name}+")?;
            }
        }
        match &self.key {
            KeyName::Char('+') => f.write_str("plus"),
            KeyName::Char('-') => f.write_str("minus"),
            // Uppercase only when it parses back to `c` ('ß' would become "SS").
            KeyName::Char(c) => {
                let mut up = c.to_uppercase();
                match (up.next(), up.next()) {
                    (Some(u), None) if u.to_lowercase().eq([*c]) => write!(f, "{u}"),
                    _ => write!(f, "{c}"),
                }
            }
            // Every key `parse_named` accepts has a label; the debug form is a fallback only.
            KeyName::Named(n) => match named_label(*n) {
                Some(s) => f.write_str(s),
                None => write!(f, "{n:?}"),
            },
        }
    }
}

impl FromStr for KeyCombo {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let parts: Vec<&str> = s.split('+').map(str::trim).collect();
        let (key_part, mod_parts) = parts
            .split_last()
            .ok_or_else(|| format!("empty key combo: {s:?}"))?;
        let mut mods = Mods::default();
        for m in mod_parts {
            match m.to_lowercase().as_str() {
                "ctrl" | "control" => mods.ctrl = true,
                "alt" => mods.alt = true,
                "shift" => mods.shift = true,
                "super" | "logo" | "meta" => mods.logo = true,
                other => return Err(format!("unknown modifier {other:?} in {s:?}")),
            }
        }
        let lower = key_part.to_lowercase();
        let key = if lower.is_empty() {
            return Err(format!("missing key in {s:?}"));
        } else if let Some(n) = parse_named(&lower) {
            KeyName::Named(n)
        } else if lower == "plus" {
            KeyName::Char('+')
        } else if lower == "minus" {
            KeyName::Char('-')
        } else {
            let mut chars = lower.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => KeyName::Char(c),
                _ => return Err(format!("unknown key {key_part:?} in {s:?}")),
            }
        };
        Ok(KeyCombo { mods, key })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    // Tiling
    SplitRight,
    SplitDown,
    SplitLeft,
    SplitUp,
    SplitAuto,
    FocusUp,
    FocusDown,
    FocusLeft,
    FocusRight,
    /// 1..=10
    FocusPane(u8),
    NextPane,
    PrevPane,
    ResizeUp,
    ResizeDown,
    ResizeLeft,
    ResizeRight,
    MaximizePane,
    ClosePane,
    // Tabs
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    MoveTabLeft,
    MoveTabRight,
    /// 1..=10
    GoToTab(u8),
    // Session and clipboard
    SaveSession,
    OpenSession,
    NewWindow,
    Copy,
    Paste,
    PastePrimary,
    SelectAll,
    // Find and scrolling
    Find,
    FindNext,
    FindPrev,
    ScrollLineUp,
    ScrollLineDown,
    ScrollPageUp,
    ScrollPageDown,
    // View
    ZoomIn,
    ZoomOut,
    ZoomReset,
    Fullscreen,
    ShowSizes,
}

/// Every action without a number: the variant, its `[keybindings]` name and its label in the
/// shortcuts list, in the order of the docs. The numbered families are handled in the methods.
const TABLE: &[(Action, &str, &str)] = {
    use Action::*;
    &[
        (SplitRight, "split_right", "Split right"),
        (SplitDown, "split_down", "Split down"),
        (SplitLeft, "split_left", "Split left"),
        (SplitUp, "split_up", "Split up"),
        (SplitAuto, "split_auto", "Split automatically"),
        (FocusUp, "focus_up", "Focus pane above"),
        (FocusDown, "focus_down", "Focus pane below"),
        (FocusLeft, "focus_left", "Focus pane left"),
        (FocusRight, "focus_right", "Focus pane right"),
        (NextPane, "next_pane", "Next pane"),
        (PrevPane, "prev_pane", "Previous pane"),
        (ResizeUp, "resize_up", "Resize pane up"),
        (ResizeDown, "resize_down", "Resize pane down"),
        (ResizeLeft, "resize_left", "Resize pane left"),
        (ResizeRight, "resize_right", "Resize pane right"),
        (MaximizePane, "maximize_pane", "Maximize pane"),
        (ClosePane, "close_pane", "Close pane"),
        (NewTab, "new_tab", "New tab"),
        (CloseTab, "close_tab", "Close tab"),
        (NextTab, "next_tab", "Next tab"),
        (PrevTab, "prev_tab", "Previous tab"),
        (MoveTabLeft, "move_tab_left", "Move tab left"),
        (MoveTabRight, "move_tab_right", "Move tab right"),
        (SaveSession, "save_session", "Save session"),
        (OpenSession, "open_session", "Open session"),
        (NewWindow, "new_window", "New window"),
        (Copy, "copy", "Copy"),
        (Paste, "paste", "Paste"),
        (PastePrimary, "paste_primary", "Paste selection"),
        (SelectAll, "select_all", "Select all"),
        (Find, "find", "Find"),
        (FindNext, "find_next", "Find next"),
        (FindPrev, "find_prev", "Find previous"),
        (ScrollLineUp, "scroll_line_up", "Scroll line up"),
        (ScrollLineDown, "scroll_line_down", "Scroll line down"),
        (ScrollPageUp, "scroll_page_up", "Scroll page up"),
        (ScrollPageDown, "scroll_page_down", "Scroll page down"),
        (ZoomIn, "zoom_in", "Zoom in"),
        (ZoomOut, "zoom_out", "Zoom out"),
        (ZoomReset, "zoom_reset", "Reset zoom"),
        (Fullscreen, "fullscreen", "Toggle fullscreen"),
        (ShowSizes, "show_sizes", "Show pane sizes"),
    ]
};

impl Action {
    /// The snake_case name used in config.toml's `[keybindings]`, e.g. `split_right`, `focus_pane_3`.
    pub fn config_name(&self) -> String {
        match self {
            Action::FocusPane(n) => format!("focus_pane_{n}"),
            Action::GoToTab(n) => format!("go_to_tab_{n}"),
            _ => self.row().1.into(),
        }
    }

    fn row(&self) -> &'static (Action, &'static str, &'static str) {
        TABLE
            .iter()
            .find(|r| r.0 == *self)
            .unwrap_or_else(|| panic!("{self:?} has no row in TABLE"))
    }

    /// Every action, with `FocusPane` and `GoToTab` for 1..=10, in the order of the docs.
    pub fn all() -> Vec<Action> {
        let mut v: Vec<Action> = TABLE.iter().map(|r| r.0).collect();
        v.extend((1..=10).flat_map(|n| [Action::FocusPane(n), Action::GoToTab(n)]));
        v
    }

    /// The inverse of `config_name`.
    pub fn from_config_name(s: &str) -> Option<Action> {
        let numbered = |prefix: &str| -> Option<u8> {
            let n: u8 = s.strip_prefix(prefix)?.parse().ok()?;
            // Reject "03" and "+3": only the canonical spelling is a name.
            (1..=10)
                .contains(&n)
                .then_some(n)
                .filter(|n| n.to_string() == s[prefix.len()..])
        };
        if let Some(n) = numbered("focus_pane_") {
            return Some(Action::FocusPane(n));
        }
        if let Some(n) = numbered("go_to_tab_") {
            return Some(Action::GoToTab(n));
        }
        TABLE.iter().find(|r| r.1 == s).map(|r| r.0)
    }

    /// Short human-readable name, as shown in the shortcuts list.
    pub fn label(&self) -> String {
        match self {
            Action::FocusPane(n) => format!("Focus pane {n}"),
            Action::GoToTab(n) => format!("Go to tab {n}"),
            _ => self.row().2.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bindings {
    map: HashMap<KeyCombo, Action>,
}

impl Bindings {
    pub fn defaults() -> Bindings {
        use Action::*;
        let mut map = HashMap::new();
        let mut bind = |s: &str, a: Action| {
            let combo: KeyCombo = s.parse().expect("valid built-in key combo");
            map.insert(combo, a);
        };
        bind("Ctrl+Alt+R", SplitRight);
        bind("Ctrl+Alt+D", SplitDown);
        // Not Ctrl+Alt+Shift+R: Cinnamon and GNOME record the screen with it.
        bind("Ctrl+Alt+Shift+L", SplitLeft);
        bind("Ctrl+Alt+Shift+D", SplitUp);
        bind("Ctrl+Alt+A", SplitAuto);
        bind("Alt+Up", FocusUp);
        bind("Alt+Down", FocusDown);
        bind("Alt+Left", FocusLeft);
        bind("Alt+Right", FocusRight);
        bind("Ctrl+Tab", NextPane);
        bind("Ctrl+Shift+Tab", PrevPane);
        bind("Shift+Alt+Up", ResizeUp);
        bind("Shift+Alt+Down", ResizeDown);
        bind("Shift+Alt+Left", ResizeLeft);
        bind("Shift+Alt+Right", ResizeRight);
        bind("Ctrl+Shift+X", MaximizePane);
        bind("Ctrl+Shift+W", ClosePane);
        bind("Ctrl+Shift+T", NewTab);
        bind("Ctrl+Shift+Q", CloseTab);
        bind("Ctrl+PageDown", NextTab);
        bind("Ctrl+PageUp", PrevTab);
        bind("Ctrl+Shift+PageUp", MoveTabLeft);
        bind("Ctrl+Shift+PageDown", MoveTabRight);
        bind("Ctrl+Shift+S", SaveSession);
        bind("Ctrl+Shift+O", OpenSession);
        bind("Ctrl+Shift+N", NewWindow);
        bind("Ctrl+Shift+C", Copy);
        bind("Ctrl+Shift+V", Paste);
        bind("Shift+Insert", PastePrimary);
        bind("Ctrl+Shift+A", SelectAll);
        bind("Ctrl+Shift+F", Find);
        bind("Ctrl+Shift+G", FindNext);
        bind("Ctrl+Shift+H", FindPrev);
        bind("Ctrl+Shift+Up", ScrollLineUp);
        bind("Ctrl+Shift+Down", ScrollLineDown);
        bind("Shift+PageUp", ScrollPageUp);
        bind("Shift+PageDown", ScrollPageDown);
        bind("Ctrl+plus", ZoomIn);
        bind("Ctrl+Shift+plus", ZoomIn);
        bind("Ctrl+=", ZoomIn);
        bind("Ctrl+minus", ZoomOut);
        bind("Ctrl+0", ZoomReset);
        bind("F11", Fullscreen);
        bind("Ctrl+Alt+S", ShowSizes);
        for n in 0..=9u8 {
            let slot = if n == 0 { 10 } else { n };
            bind(&format!("Alt+{n}"), FocusPane(slot));
            bind(&format!("Ctrl+Alt+{n}"), GoToTab(slot));
        }
        Bindings { map }
    }

    /// The action for a key press: the logical key (without modifiers) first; when
    /// that is unbound, a digit-row key retried as its digit by physical position.
    /// Layouts like Czech QWERTZ type `+ ě š č …` on the digit row, which would
    /// otherwise never match `Alt+1`, `Ctrl+Alt+1` or `Ctrl+0`.
    pub fn resolve(&self, logical: &Key, physical: PhysicalKey, mods: Mods) -> Option<Action> {
        if let Some(a) = KeyCombo::from_event(logical, mods).and_then(|c| self.lookup(&c)) {
            return Some(a);
        }
        let key = KeyName::Char(physical_digit(physical)?);
        self.lookup(&KeyCombo { mods, key })
    }

    /// Every binding, sorted by action label and then key text (for the shortcuts list).
    /// A trailing number sorts numerically: "Focus pane 2" before "Focus pane 10".
    pub fn entries(&self) -> Vec<(KeyCombo, Action)> {
        let mut v: Vec<(KeyCombo, Action)> =
            self.map.iter().map(|(c, a)| (c.clone(), *a)).collect();
        v.sort_by_cached_key(|(c, a)| {
            let label = a.label();
            let stem = label.trim_end_matches(|ch: char| ch.is_ascii_digit());
            let n: Option<u64> = label[stem.len()..].parse().ok();
            (stem.to_string(), n, c.to_string())
        });
        v
    }

    /// `base` with each listed action's combos replaced by the given ones (an empty list
    /// unbinds the action). A combo that `base` or an earlier override bound to another
    /// action moves to the listed one.
    pub fn with_overrides(base: Bindings, overrides: &[(Action, Vec<KeyCombo>)]) -> Bindings {
        let mut map = base.map;
        map.retain(|_, a| !overrides.iter().any(|(o, _)| o == a));
        for (action, combos) in overrides {
            for combo in combos {
                map.insert(combo.clone(), *action);
            }
        }
        Bindings { map }
    }

    pub fn lookup(&self, combo: &KeyCombo) -> Option<Action> {
        if let Some(a) = self.map.get(combo) {
            return Some(*a);
        }
        // `=` is the unshifted key that types `+` on most layouts: retry as plain `+`, ignoring shift.
        if combo.key == KeyName::Char('=') {
            let retry = KeyCombo {
                mods: Mods {
                    shift: false,
                    ..combo.mods
                },
                key: KeyName::Char('+'),
            };
            return self.map.get(&retry).copied();
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(s: &str) -> KeyCombo {
        s.parse().unwrap()
    }
    #[test]
    fn parses_modifiers_and_char() {
        assert_eq!(
            k("Ctrl+Alt+R"),
            KeyCombo {
                mods: Mods {
                    ctrl: true,
                    alt: true,
                    ..Default::default()
                },
                key: KeyName::Char('r')
            }
        );
    }
    #[test]
    fn parsing_is_case_insensitive() {
        assert_eq!(k("ctrl+alt+r"), k("Ctrl+Alt+R"));
        assert_eq!(k("T").key, KeyName::Char('t'));
        assert!(!k("T").mods.shift);
    }
    #[test]
    fn parses_named_keys() {
        assert_eq!(k("Shift+Insert").key, KeyName::Named(NamedKey::Insert));
        assert_eq!(k("Ctrl+PageDown"), k("Ctrl+Page_Down"));
        assert_eq!(k("F11").key, KeyName::Named(NamedKey::F11));
        assert_eq!(k("Alt+Up").key, KeyName::Named(NamedKey::ArrowUp));
        assert_eq!(k("Ctrl+plus").key, KeyName::Char('+'));
        assert_eq!(k("Ctrl+minus").key, KeyName::Char('-'));
    }
    #[test]
    fn rejects_garbage() {
        assert!("Ctrl+".parse::<KeyCombo>().is_err());
        assert!("Hyper+x".parse::<KeyCombo>().is_err());
    }
    #[test]
    fn tilix_defaults() {
        let b = Bindings::defaults();
        assert_eq!(b.lookup(&k("Ctrl+Alt+R")), Some(Action::SplitRight));
        assert_eq!(b.lookup(&k("Ctrl+Alt+D")), Some(Action::SplitDown));
        assert_eq!(b.lookup(&k("Ctrl+Alt+A")), Some(Action::SplitAuto));
        // Not Ctrl+Alt+Shift+R: Cinnamon and GNOME record the screen with it.
        assert_eq!(b.lookup(&k("Ctrl+Alt+Shift+L")), Some(Action::SplitLeft));
        assert_eq!(b.lookup(&k("Ctrl+Alt+Shift+R")), None);
        assert_eq!(b.lookup(&k("Ctrl+Alt+Shift+D")), Some(Action::SplitUp));
        assert_eq!(Action::SplitLeft.config_name(), "split_left");
        assert_eq!(Action::SplitUp.config_name(), "split_up");
        assert_eq!(Action::SplitLeft.label(), "Split left");
        assert_eq!(Action::SplitUp.label(), "Split up");
        assert_eq!(b.lookup(&k("Alt+Left")), Some(Action::FocusLeft));
        assert_eq!(b.lookup(&k("Alt+1")), Some(Action::FocusPane(1)));
        assert_eq!(b.lookup(&k("Alt+0")), Some(Action::FocusPane(10)));
        assert_eq!(b.lookup(&k("Shift+Alt+Right")), Some(Action::ResizeRight));
        assert_eq!(b.lookup(&k("Ctrl+Shift+X")), Some(Action::MaximizePane));
        assert_eq!(b.lookup(&k("Ctrl+Shift+W")), Some(Action::ClosePane));
        assert_eq!(b.lookup(&k("Ctrl+Tab")), Some(Action::NextPane));
        assert_eq!(b.lookup(&k("Ctrl+Shift+Tab")), Some(Action::PrevPane));
        assert_eq!(b.lookup(&k("Ctrl+Alt+3")), Some(Action::GoToTab(3)));
        assert_eq!(b.lookup(&k("Ctrl+Alt+0")), Some(Action::GoToTab(10)));
        assert_eq!(b.lookup(&k("Shift+PageUp")), Some(Action::ScrollPageUp));
        assert_eq!(b.lookup(&k("F11")), Some(Action::Fullscreen));
        assert_eq!(b.lookup(&k("Ctrl+Shift+PageUp")), Some(Action::MoveTabLeft));
        assert_eq!(
            b.lookup(&k("Ctrl+Shift+PageDown")),
            Some(Action::MoveTabRight)
        );
        assert_eq!(b.lookup(&k("Ctrl+Shift+Up")), Some(Action::ScrollLineUp));
        assert_eq!(
            b.lookup(&k("Ctrl+Shift+Down")),
            Some(Action::ScrollLineDown)
        );
        assert_eq!(b.lookup(&k("Ctrl+0")), Some(Action::ZoomReset));
    }
    #[test]
    fn zoom_in_accepts_plus_and_equals() {
        let b = Bindings::defaults();
        assert_eq!(b.lookup(&k("Ctrl+plus")), Some(Action::ZoomIn));
        assert_eq!(b.lookup(&k("Ctrl+=")), Some(Action::ZoomIn));
        assert_eq!(b.lookup(&k("Ctrl+Shift+=")), Some(Action::ZoomIn));
    }
    #[test]
    fn from_event_lowercases_and_keeps_shift() {
        let c = KeyCombo::from_event(
            &Key::Character("T".into()),
            Mods {
                ctrl: true,
                shift: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(c, k("Ctrl+Shift+T"));
    }
    #[test]
    fn digit_bindings_fall_back_to_physical_key() {
        // Czech QWERTZ digit row without modifiers: + ě š č ř ž ý á í é.
        let b = Bindings::defaults();
        let alt = Mods {
            alt: true,
            ..Default::default()
        };
        let ctrl = Mods {
            ctrl: true,
            ..Default::default()
        };
        let ctrl_alt = Mods { ctrl: true, ..alt };
        let cz = |s: &str| Key::Character(s.into());
        let code = PhysicalKey::Code;
        assert_eq!(
            b.resolve(&cz("+"), code(KeyCode::Digit1), alt),
            Some(Action::FocusPane(1))
        );
        assert_eq!(
            b.resolve(&cz("ě"), code(KeyCode::Digit2), alt),
            Some(Action::FocusPane(2))
        );
        assert_eq!(
            b.resolve(&cz("é"), code(KeyCode::Digit0), alt),
            Some(Action::FocusPane(10))
        );
        assert_eq!(
            b.resolve(&cz("č"), code(KeyCode::Digit4), ctrl_alt),
            Some(Action::GoToTab(4))
        );
        assert_eq!(
            b.resolve(&cz("é"), code(KeyCode::Digit0), ctrl),
            Some(Action::ZoomReset)
        );
        // The logical key wins when it is bound.
        assert_eq!(
            b.resolve(&cz("r"), code(KeyCode::Digit3), ctrl_alt),
            Some(Action::SplitRight)
        );
        // Ctrl + `+` (Czech Digit1) stays ZoomIn.
        assert_eq!(
            b.resolve(&cz("+"), code(KeyCode::Digit1), ctrl),
            Some(Action::ZoomIn)
        );
        // Only the digit row falls back; unbound combos stay unbound.
        assert_eq!(b.resolve(&cz("ú"), code(KeyCode::BracketLeft), alt), None);
        assert_eq!(
            b.resolve(&cz("+"), code(KeyCode::Digit1), Mods::default()),
            None
        );
    }

    #[test]
    fn keycombo_display_roundtrip() {
        for (combo, _) in Bindings::defaults().entries() {
            let s = combo.to_string();
            assert_eq!(s.parse::<KeyCombo>().as_ref(), Ok(&combo), "{s}");
        }
        assert_eq!(k("ctrl+alt+r").to_string(), "Ctrl+Alt+R");
        assert_eq!(k("Shift+Page_Up").to_string(), "Shift+PageUp");
        assert_eq!(k("Ctrl+plus").to_string(), "Ctrl+plus");
        assert_eq!(
            k("super+shift+ctrl+alt+minus").to_string(),
            "Ctrl+Alt+Shift+Super+minus"
        );
    }

    #[test]
    fn keycombo_display_keeps_multichar_uppercase_unchanged() {
        // 'ß' uppercases to "SS", which would not parse back.
        let c = KeyCombo {
            mods: Mods {
                ctrl: true,
                ..Default::default()
            },
            key: KeyName::Char('ß'),
        };
        assert_eq!(c.to_string(), "Ctrl+ß");
        assert_eq!(c.to_string().parse::<KeyCombo>(), Ok(c));
        assert_eq!(k("Alt+é").to_string(), "Alt+É");
    }

    #[test]
    fn named_label_matches_parse_named() {
        for name in [
            "up", "down", "left", "right", "pageup", "pagedown", "insert", "delete", "home", "end",
            "tab", "escape", "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10", "f11",
            "f12",
        ] {
            let key = parse_named(name).unwrap_or_else(|| panic!("{name} unparsed"));
            let label = named_label(key).unwrap_or_else(|| panic!("{name} unlabeled"));
            assert_eq!(label.to_lowercase(), name);
        }
    }

    #[test]
    fn entries_cover_defaults() {
        let b = Bindings::defaults();
        let e = b.entries();
        assert_eq!(e.len(), b.map.len());
        let labels: Vec<String> = e.iter().map(|(_, a)| a.label()).collect();
        assert!(labels.contains(&"Focus pane 3".to_string()));
        assert!(labels.contains(&"Go to tab 10".to_string()));
    }

    #[test]
    fn entries_sort_numbers_naturally() {
        let labels: Vec<String> = Bindings::defaults()
            .entries()
            .iter()
            .map(|(_, a)| a.label())
            .collect();
        let at = |l: &str| labels.iter().position(|x| x == l).unwrap();
        assert!(at("Focus pane 2") < at("Focus pane 10"));
        assert!(at("Focus pane 9") < at("Focus pane 10"));
        assert!(at("Go to tab 2") < at("Go to tab 10"));
        assert!(at("Close pane") < at("Close tab"));
        // Numbered rows stay together, ahead of "Focus pane above".
        assert_eq!(at("Focus pane 10"), at("Focus pane 1") + 9);
        assert!(at("Focus pane 10") < at("Focus pane above"));
    }

    #[test]
    fn config_names_roundtrip() {
        let mut names = std::collections::HashSet::new();
        for a in Action::all() {
            let name = a.config_name();
            assert_eq!(Action::from_config_name(&name), Some(a), "{name}");
            names.insert(name);
        }
        assert!(names.contains("split_right") && names.contains("go_to_tab_10"));
        assert_eq!(Action::from_config_name("focus_pane_11"), None);
        assert_eq!(Action::from_config_name("focus_pane_03"), None);
        assert_eq!(Action::from_config_name("nope"), None);
    }

    #[test]
    fn all_covers_defaults_with_unique_names() {
        let all = Action::all();
        // The plain rows, plus the two numbered families ten times each.
        assert_eq!(all.len(), TABLE.len() + 20);
        let names: std::collections::HashSet<String> =
            all.iter().map(Action::config_name).collect();
        assert_eq!(names.len(), all.len(), "duplicate config name");
        for (_, a) in Bindings::defaults().entries() {
            assert!(all.contains(&a), "{a:?} missing from TABLE");
        }
    }

    /// Every action has a table row "| `name` | `combo`, `combo` |" in docs/config.md whose
    /// combos are the defaults, in the order the shortcuts list shows them.
    #[test]
    fn config_md_lists_all_actions() {
        let doc = include_str!("../../docs/config.md");
        let entries = Bindings::defaults().entries();
        for a in Action::all() {
            let name = a.config_name();
            let prefix = format!("| `{name}` | ");
            let row = doc
                .lines()
                .find_map(|l| l.strip_prefix(&prefix))
                .unwrap_or_else(|| panic!("docs/config.md has no table row for `{name}`"));
            let want = entries
                .iter()
                .filter(|(_, x)| *x == a)
                .map(|(c, _)| format!("`{c}`"))
                .collect::<Vec<_>>()
                .join(", ");
            assert_eq!(
                row.trim_end().trim_end_matches('|').trim_end(),
                want,
                "{name}"
            );
        }
    }

    #[test]
    fn overrides_move_and_unbind() {
        let b = Bindings::with_overrides(
            Bindings::defaults(),
            &[
                (Action::SplitRight, vec![k("Ctrl+Shift+V")]),
                (Action::Fullscreen, vec![]),
                (Action::ShowSizes, vec![k("Ctrl+Alt+Z")]),
            ],
        );
        assert_eq!(b.lookup(&k("Ctrl+Alt+Z")), Some(Action::ShowSizes));
        assert_eq!(b.lookup(&k("Ctrl+Alt+S")), None);
        assert_eq!(b.lookup(&k("Ctrl+Shift+V")), Some(Action::SplitRight));
        assert_eq!(b.lookup(&k("Ctrl+Alt+R")), None);
        assert_eq!(b.lookup(&k("F11")), None);
        // Paste lost its combo to SplitRight; untouched bindings stay.
        assert_eq!(b.lookup(&k("Ctrl+Shift+C")), Some(Action::Copy));
    }

    #[test]
    fn show_sizes_default_and_name() {
        let b = Bindings::defaults();
        assert_eq!(b.lookup(&k("Ctrl+Alt+S")), Some(Action::ShowSizes));
        assert_eq!(b.lookup(&k("Ctrl+Shift+S")), Some(Action::SaveSession));
        assert_eq!(Action::ShowSizes.config_name(), "show_sizes");
        assert_eq!(Action::ShowSizes.label(), "Show pane sizes");
    }

    #[test]
    fn unbound_key_is_none() {
        assert_eq!(Bindings::defaults().lookup(&k("Ctrl+Q")), None);
    }
}
