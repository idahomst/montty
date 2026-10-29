//! `config.toml`: window-wide settings and rebindable keys.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde::Deserialize;
use toml::Spanned;

use super::{ConfigError, line_of, parse_color, short_unknown_key};
use crate::input::bindings::{Action, Bindings, KeyCombo};
use crate::term::colors::Rgb;

#[derive(Clone, Debug, PartialEq)]
pub struct GlobalConfig {
    pub default_profile: String,
    /// The line of `default_profile` in the file; None when the file doesn't set it.
    pub default_profile_line: Option<usize>,
    pub focus_border: Rgb,
    pub divider: Rgb,
    /// Alpha of the dimming overlay on unfocused panes, 0.0..=1.0.
    pub dim: f32,
    pub restore_last_session: bool,
    /// The bell flashes the pane (the urgency hint of a background window doesn't depend
    /// on it).
    pub visual_bell: bool,
    /// Panes can be dragged by the overlay's grip onto another pane to swap them.
    pub pane_drag: bool,
    /// Closing a window (or its last tab) with several panes asks first.
    pub confirm_close_window: bool,
    /// Dots on tabs and panes that need attention, and their hover labels. The state is
    /// tracked either way.
    pub attention_marks: bool,
    pub bindings: Bindings,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        GlobalConfig {
            default_profile: "default".into(),
            default_profile_line: None,
            focus_border: Rgb {
                r: 0x4a,
                g: 0x90,
                b: 0xd9,
            },
            divider: Rgb {
                r: 0x55,
                g: 0x55,
                b: 0x55,
            },
            dim: 0.25,
            restore_last_session: false,
            visual_bell: true,
            pane_drag: true,
            confirm_close_window: true,
            attention_marks: true,
            bindings: Bindings::defaults(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    default_profile: Option<Spanned<String>>,
    focus_border_color: Option<Spanned<String>>,
    divider_color: Option<Spanned<String>>,
    dim_amount: Option<Spanned<f64>>,
    restore_last_session: Option<bool>,
    visual_bell: Option<bool>,
    pane_drag: Option<bool>,
    confirm_close_window: Option<bool>,
    attention_marks: Option<bool>,
    keybindings: Option<BTreeMap<String, Spanned<toml::Value>>>,
}

impl GlobalConfig {
    /// Parse `src`; `file` (relative to the config dir) names it in errors.
    pub fn parse(src: &str, file: &Path) -> Result<GlobalConfig, ConfigError> {
        let err = |byte: Option<usize>, msg: String| ConfigError {
            file: file.to_path_buf(),
            line: byte.map(|b| line_of(src, b)),
            msg,
        };
        let raw: Raw = toml::from_str(src)
            .map_err(|e| err(e.span().map(|s| s.start), short_unknown_key(e.message())))?;
        let mut cfg = GlobalConfig::default();
        if let Some(p) = raw.default_profile {
            cfg.default_profile_line = Some(line_of(src, p.span().start));
            cfg.default_profile = p.into_inner();
        }
        for (field, key, slot) in [
            (
                raw.focus_border_color,
                "focus_border_color",
                &mut cfg.focus_border,
            ),
            (raw.divider_color, "divider_color", &mut cfg.divider),
        ] {
            if let Some(v) = field {
                *slot = parse_color(v.get_ref()).ok_or_else(|| {
                    err(
                        Some(v.span().start),
                        format!("{key}: invalid color {:?}", v.get_ref()),
                    )
                })?;
            }
        }
        if let Some(d) = raw.dim_amount {
            let v = *d.get_ref();
            if !(0.0..=1.0).contains(&v) {
                return Err(err(
                    Some(d.span().start),
                    format!("dim_amount must be between 0.0 and 1.0, got {v}"),
                ));
            }
            cfg.dim = v as f32;
        }
        if let Some(r) = raw.restore_last_session {
            cfg.restore_last_session = r;
        }
        if let Some(v) = raw.visual_bell {
            cfg.visual_bell = v;
        }
        if let Some(v) = raw.pane_drag {
            cfg.pane_drag = v;
        }
        if let Some(v) = raw.confirm_close_window {
            cfg.confirm_close_window = v;
        }
        if let Some(v) = raw.attention_marks {
            cfg.attention_marks = v;
        }
        // Validate in file order so the first error in the file is the one reported.
        let mut entries: Vec<_> = raw.keybindings.into_iter().flatten().collect();
        entries.sort_by_key(|(_, v)| v.span().start);
        let mut overrides = Vec::new();
        // A combo may belong to only one action; repeating it within one action is fine.
        let mut claimed: HashMap<KeyCombo, String> = HashMap::new();
        for (name, value) in entries {
            let at = Some(value.span().start);
            let action = Action::from_config_name(&name)
                .ok_or_else(|| err(at, format!("unknown action `{name}` in [keybindings]")))?;
            let combos =
                key_combos(value.get_ref()).map_err(|m| err(at, format!("{name}: {m}")))?;
            for c in &combos {
                match claimed.get(c) {
                    Some(other) if *other != name => {
                        return Err(err(
                            at,
                            format!("`{c}` is bound to both `{other}` and `{name}`"),
                        ));
                    }
                    _ => {
                        claimed.insert(c.clone(), name.clone());
                    }
                }
            }
            overrides.push((action, combos));
        }
        cfg.bindings = Bindings::with_overrides(cfg.bindings, &overrides);
        Ok(cfg)
    }
}

/// A keybinding value: a combo string, an array of them, or "none" (no combos).
fn key_combos(value: &toml::Value) -> Result<Vec<KeyCombo>, String> {
    let combo = |v: &toml::Value| -> Result<KeyCombo, String> {
        v.as_str()
            .ok_or_else(|| "expected a key combo string".to_string())?
            .parse()
    };
    match value {
        toml::Value::String(s) if s.trim().eq_ignore_ascii_case("none") => Ok(Vec::new()),
        toml::Value::Array(items) => items.iter().map(combo).collect(),
        v => combo(v).map(|c| vec![c]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::bindings::Action;

    fn p() -> &'static Path {
        Path::new("config.toml")
    }
    fn k(s: &str) -> KeyCombo {
        s.parse().unwrap()
    }

    #[test]
    fn default_profile_line_is_recorded() {
        let g = GlobalConfig::parse("dim_amount = 0.3\ndefault_profile = \"w\"\n", p()).unwrap();
        assert_eq!(
            (g.default_profile.as_str(), g.default_profile_line),
            ("w", Some(2))
        );
        assert_eq!(
            GlobalConfig::parse("", p()).unwrap().default_profile_line,
            None
        );
    }

    #[test]
    fn unknown_key_message_is_short() {
        let e = GlobalConfig::parse("dim_amount = 0.3\nbogus = 1\n", p()).unwrap_err();
        assert_eq!(e.line, Some(2));
        assert_eq!(e.msg, "unknown key `bogus`");
        assert_eq!(e.to_string(), "config.toml:2: unknown key `bogus`");
    }

    #[test]
    fn global_defaults_match_plan2() {
        let c = GlobalConfig::parse("", p()).unwrap();
        assert_eq!(c, GlobalConfig::default());
        assert_eq!(
            c.bindings.lookup(&k("Ctrl+Alt+R")),
            Some(Action::SplitRight)
        );
        assert_eq!(c.default_profile, "default");
        assert_eq!(c.dim, 0.25);
    }

    #[test]
    fn global_scalars() {
        let c = GlobalConfig::parse(
            "default_profile = \"work\"\nfocus_border_color = \"#fff\"\ndivider_color = \"rgb(1,2,3)\"\ndim_amount = 1\nrestore_last_session = true",
            p(),
        )
        .unwrap();
        assert_eq!(c.default_profile, "work");
        assert_eq!(
            c.focus_border,
            Rgb {
                r: 255,
                g: 255,
                b: 255
            }
        );
        assert_eq!(c.divider, Rgb { r: 1, g: 2, b: 3 });
        assert_eq!(c.dim, 1.0);
        assert!(c.restore_last_session);
    }

    #[test]
    fn pane_drag_defaults_on_and_can_be_turned_off() {
        assert!(GlobalConfig::default().pane_drag);
        assert!(GlobalConfig::parse("", p()).unwrap().pane_drag);
        let c = GlobalConfig::parse("pane_drag = false", p()).unwrap();
        assert!(!c.pane_drag);
    }

    #[test]
    fn confirm_close_window_defaults_on_and_can_be_turned_off() {
        assert!(GlobalConfig::default().confirm_close_window);
        assert!(GlobalConfig::parse("", p()).unwrap().confirm_close_window);
        let c = GlobalConfig::parse("confirm_close_window = false", p()).unwrap();
        assert!(!c.confirm_close_window);
    }

    #[test]
    fn visual_bell_defaults_on_and_can_be_turned_off() {
        assert!(GlobalConfig::default().visual_bell);
        assert!(GlobalConfig::parse("", p()).unwrap().visual_bell);
        let c = GlobalConfig::parse("visual_bell = false", p()).unwrap();
        assert!(!c.visual_bell);
    }

    #[test]
    fn attention_marks_default_and_parse() {
        assert!(GlobalConfig::default().attention_marks);
        assert!(GlobalConfig::parse("", p()).unwrap().attention_marks);
        let c = GlobalConfig::parse("attention_marks = false", p()).unwrap();
        assert!(!c.attention_marks);
        let e = GlobalConfig::parse("dim_amount = 0.5\nattention_marks = 3", p()).unwrap_err();
        assert_eq!(e.line, Some(2));
    }

    #[test]
    fn global_keybinding_override() {
        let c = GlobalConfig::parse(
            "[keybindings]\nsplit_right = \"Ctrl+Alt+E\"\npaste = \"none\"\ncopy = [\"Ctrl+Shift+C\", \"Ctrl+Insert\"]",
            p(),
        )
        .unwrap();
        let b = &c.bindings;
        assert_eq!(b.lookup(&k("Ctrl+Alt+E")), Some(Action::SplitRight));
        assert_eq!(b.lookup(&k("Ctrl+Alt+R")), None);
        assert_eq!(b.lookup(&k("Ctrl+Shift+V")), None);
        assert_eq!(b.lookup(&k("Ctrl+Insert")), Some(Action::Copy));
        assert_eq!(b.lookup(&k("Ctrl+Shift+C")), Some(Action::Copy));
    }

    #[test]
    fn global_errors_have_lines() {
        let e = GlobalConfig::parse("dim_amount = 2.0", p()).unwrap_err();
        assert_eq!(e.line, Some(1));
        assert!(e.msg.contains("dim_amount"), "{e}");
        let e = GlobalConfig::parse("\n\nbogus = 1", p()).unwrap_err();
        assert_eq!(e.line, Some(3));
        assert!(e.msg.contains("bogus"), "{e}");
        let e = GlobalConfig::parse("[keybindings]\nfly = \"Ctrl+F\"", p()).unwrap_err();
        assert_eq!(e.line, Some(2));
        assert!(e.msg.contains("fly"), "{e}");
        let e = GlobalConfig::parse("[keybindings]\ncopy = \"Ctrl+Hyper+C\"", p()).unwrap_err();
        assert_eq!(e.line, Some(2));
        let e = GlobalConfig::parse("divider_color = \"blue\"", p()).unwrap_err();
        assert_eq!(e.line, Some(1));
        assert_eq!(e.to_string().split(':').next(), Some("config.toml"));
        // Syntax errors carry a line too.
        assert_eq!(
            GlobalConfig::parse("a = \n", p()).unwrap_err().line,
            Some(1)
        );
    }

    #[test]
    fn global_wrong_type_has_line() {
        let e = GlobalConfig::parse("dim_amount = \"x\"", p()).unwrap_err();
        assert_eq!(e.line, Some(1));
        let e = GlobalConfig::parse("[keybindings]\ncopy = 5", p()).unwrap_err();
        assert_eq!(e.line, Some(2));
    }

    #[test]
    fn global_combo_conflict_is_error() {
        let e = GlobalConfig::parse("[keybindings]\ncopy = \"Ctrl+X\"\npaste = \"Ctrl+X\"", p())
            .unwrap_err();
        assert_eq!(e.line, Some(3));
        assert!(
            e.msg
                .contains("`Ctrl+X` is bound to both `copy` and `paste`"),
            "{e}"
        );
        // Duplicates within one action's own array are fine.
        GlobalConfig::parse("[keybindings]\ncopy = [\"Ctrl+X\", \"ctrl+x\"]", p()).unwrap();
    }

    #[test]
    fn global_keybinding_errors_in_file_order() {
        // "a_bad" sorts before "copy" alphabetically but comes later in the file.
        let e = GlobalConfig::parse(
            "[keybindings]\nzoom_in = \"Hyper+1\"\nfly = \"Ctrl+F\"",
            p(),
        )
        .unwrap_err();
        assert_eq!(e.line, Some(2));
        // The conflict is reported on the later line in file order, naming earlier action first.
        let e = GlobalConfig::parse(
            "[keybindings]\nzoom_in = \"Ctrl+X\"\ncopy = \"Ctrl+X\"",
            p(),
        )
        .unwrap_err();
        assert_eq!(e.line, Some(3));
        assert!(e.msg.contains("both `zoom_in` and `copy`"), "{e}");
    }
}
