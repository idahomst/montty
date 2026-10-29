//! Kitty keyboard protocol key encoding, written from
//! <https://sw.kovidgoyal.net/kitty/keyboard-protocol/>. The flags the program pushed
//! come from the terminal's mode; with none set nothing here applies and keys encode as
//! before (`keys::encode_key`).

use alacritty_terminal::term::TermMode;
use winit::keyboard::{Key, NamedKey};

use super::Mods;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEventKind {
    Press,
    Repeat,
    Release,
}

pub struct KittyKey<'a> {
    /// The key without modifiers applied.
    pub key: &'a Key,
    /// The character the logical key produced when Shift changed it (flag 4).
    pub shifted: Option<char>,
    /// Text reported by winit for this event (flag 16).
    pub text: Option<&'a str>,
    pub mods: Mods,
    pub kind: KeyEventKind,
    /// A modifier key on the right side of the keyboard (`KeyLocation::Right`).
    pub right: bool,
}

/// How a key is written once it is escape-coded.
enum Form {
    /// `CSI code u`.
    U(u32),
    /// `CSI 1 ; mods final` (`CSI final` when bare).
    Letter(u8),
    /// `CSI n ; mods ~` (`CSI n ~` when bare).
    Tilde(u8),
}

/// Whether `mode` has any kitty flag set (the caller then wants releases and repeats too).
pub fn active(mode: TermMode) -> bool {
    mode.intersects(TermMode::KITTY_KEYBOARD_PROTOCOL)
}

/// `None` = not a kitty case: the caller falls back to the legacy encoding (press) or
/// sends nothing (release, repeat).
pub fn encode_kitty(key: &KittyKey<'_>, mode: TermMode) -> Option<Vec<u8>> {
    if !active(mode) {
        return None;
    }
    let all = mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC);
    let events = mode.contains(TermMode::REPORT_EVENT_TYPES);
    let mods = key.mods;
    // Without flag 2 a repeat is reported as a press (only releases are dropped).
    let kind = match key.kind {
        KeyEventKind::Repeat if !events => KeyEventKind::Press,
        k => k,
    };
    let mut bits = u32::from(mods.shift)
        | (2 * u32::from(mods.alt))
        | (4 * u32::from(mods.ctrl))
        | (8 * u32::from(mods.logo));
    // Releases are only reported when asked for.
    if kind == KeyEventKind::Release && !events {
        return None;
    }
    let non_shift = mods.alt || mods.ctrl || mods.logo;

    let (form, text_key) = match key.key {
        Key::Named(n) => match named_form(*n, key.right) {
            Some(NamedForm::Modifier(code, bit)) => {
                if !all {
                    return None;
                }
                // A modifier key sets its own bit while held.
                if kind == KeyEventKind::Release {
                    bits &= !bit;
                } else {
                    bits |= bit;
                }
                (Form::U(code), false)
            }
            Some(NamedForm::Plain(f)) => (f, false),
            None if *n == NamedKey::Space => (Form::U(32), true),
            None => return None,
        },
        Key::Character(s) => {
            let c = s.chars().next()?;
            (Form::U(lower(c) as u32), true)
        }
        _ => return None,
    };

    let escaped = match &form {
        // Enter, Tab, Backspace keep their legacy bytes unless modified or `all`.
        Form::U(9 | 13 | 127) => {
            // Their releases need `all`, even with modifiers.
            if kind == KeyEventKind::Release && !all {
                return None;
            }
            all || bits != 0
        }
        Form::U(27) => true,
        // Text keys are text unless modified; without flag 8 they have no release or repeat
        // events (a repeat is text again, the caller's legacy fallback).
        Form::U(_) if text_key => all || non_shift,
        Form::U(_) => true,
        // Plain functional keys stay legacy unless events or `all` ask for them.
        Form::Letter(_) | Form::Tilde(_) => all || bits != 0 || kind != KeyEventKind::Press,
    };
    if !escaped {
        return None;
    }

    let param = 1 + bits;
    let ev = match kind {
        KeyEventKind::Press => None,
        KeyEventKind::Repeat => Some(2),
        KeyEventKind::Release => Some(3),
    };
    // `mods[:event]`; the modifier is explicit whenever an event follows it.
    let mods_field = |blank_if_default: bool| match ev {
        Some(e) => format!("{param}:{e}"),
        None if param == 1 && blank_if_default => String::new(),
        None => param.to_string(),
    };

    let out = match form {
        Form::U(code) => {
            let mut s = format!("\x1b[{code}");
            if mode.contains(TermMode::REPORT_ALTERNATE_KEYS)
                && mods.shift
                && let Some(sh) = key.shifted
                && text_key
            {
                s.push_str(&format!(":{}", sh as u32));
            }
            let text = associated_text(key, mode, all);
            if let Some(t) = text {
                // The spec's own example leaves the modifier blank here (`CSI 0 ; ; 229 u`).
                s.push_str(&format!(";{};{t}u", mods_field(true)));
            } else if param == 1 && ev.is_none() {
                s.push('u');
            } else {
                s.push_str(&format!(";{}u", mods_field(false)));
            }
            s
        }
        Form::Letter(f) => {
            if param == 1 && ev.is_none() {
                format!("\x1b[{}", f as char)
            } else {
                format!("\x1b[1;{}{}", mods_field(false), f as char)
            }
        }
        Form::Tilde(n) => {
            if param == 1 && ev.is_none() {
                format!("\x1b[{n}~")
            } else {
                format!("\x1b[{n};{}~", mods_field(false))
            }
        }
    };
    Some(out.into_bytes())
}

/// Flag 16: the text as `:`-separated code points. Only with flag 8 (the spec leaves it
/// undefined otherwise), never on release, never control characters.
fn associated_text(key: &KittyKey<'_>, mode: TermMode, all: bool) -> Option<String> {
    if !all || !mode.contains(TermMode::REPORT_ASSOCIATED_TEXT) || key.kind == KeyEventKind::Release
    {
        return None;
    }
    let t = key
        .text
        .filter(|t| !t.is_empty() && !t.chars().any(char::is_control))?;
    let cps: Vec<String> = t.chars().map(|c| (c as u32).to_string()).collect();
    Some(cps.join(":"))
}

fn lower(c: char) -> char {
    let mut it = c.to_lowercase();
    match (it.next(), it.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

enum NamedForm {
    Plain(Form),
    /// Code and the modifier bit the key itself sets.
    Modifier(u32, u32),
}

/// The right-hand modifier codes (57447..) follow the left ones (57441..) by six.
fn side(right: bool) -> u32 {
    if right { 6 } else { 0 }
}

fn named_form(n: NamedKey, right: bool) -> Option<NamedForm> {
    use Form::{Letter, Tilde, U};
    let f = match n {
        NamedKey::Enter => U(13),
        NamedKey::Tab => U(9),
        NamedKey::Backspace => U(127),
        NamedKey::Escape => U(27),
        NamedKey::ArrowUp => Letter(b'A'),
        NamedKey::ArrowDown => Letter(b'B'),
        NamedKey::ArrowRight => Letter(b'C'),
        NamedKey::ArrowLeft => Letter(b'D'),
        NamedKey::Home => Letter(b'H'),
        NamedKey::End => Letter(b'F'),
        NamedKey::Insert => Tilde(2),
        NamedKey::Delete => Tilde(3),
        NamedKey::PageUp => Tilde(5),
        NamedKey::PageDown => Tilde(6),
        NamedKey::F1 => Letter(b'P'),
        NamedKey::F2 => Letter(b'Q'),
        // Not `CSI R`: the kitty spec dropped it, it clashes with the cursor position report.
        NamedKey::F3 => Tilde(13),
        NamedKey::F4 => Letter(b'S'),
        NamedKey::F5 => Tilde(15),
        NamedKey::F6 => Tilde(17),
        NamedKey::F7 => Tilde(18),
        NamedKey::F8 => Tilde(19),
        NamedKey::F9 => Tilde(20),
        NamedKey::F10 => Tilde(21),
        NamedKey::F11 => Tilde(23),
        NamedKey::F12 => Tilde(24),
        NamedKey::F13 => U(57376),
        NamedKey::F14 => U(57377),
        NamedKey::F15 => U(57378),
        NamedKey::F16 => U(57379),
        NamedKey::F17 => U(57380),
        NamedKey::F18 => U(57381),
        NamedKey::F19 => U(57382),
        NamedKey::F20 => U(57383),
        NamedKey::Shift => return Some(NamedForm::Modifier(57441 + side(right), 1)),
        NamedKey::Control => return Some(NamedForm::Modifier(57442 + side(right), 4)),
        NamedKey::Alt => return Some(NamedForm::Modifier(57443 + side(right), 2)),
        NamedKey::Super => return Some(NamedForm::Modifier(57444 + side(right), 8)),
        _ => return None,
    };
    Some(NamedForm::Plain(f))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Mods = Mods {
        ctrl: false,
        alt: false,
        shift: false,
        logo: false,
    };
    const CTRL: Mods = Mods { ctrl: true, ..NONE };
    const ALT: Mods = Mods { alt: true, ..NONE };
    const SHIFT: Mods = Mods {
        shift: true,
        ..NONE
    };

    const D: TermMode = TermMode::DISAMBIGUATE_ESC_CODES;
    const E: TermMode = TermMode::REPORT_EVENT_TYPES;
    const A: TermMode = TermMode::REPORT_ALL_KEYS_AS_ESC;

    fn enc(
        key: Key,
        shifted: Option<char>,
        text: Option<&str>,
        mods: Mods,
        kind: KeyEventKind,
        mode: TermMode,
    ) -> Option<String> {
        encode_kitty(
            &KittyKey {
                key: &key,
                shifted,
                text,
                mods,
                kind,
                right: false,
            },
            mode,
        )
        .map(|b| String::from_utf8(b).unwrap())
    }
    fn press(key: Key, mods: Mods, mode: TermMode) -> Option<String> {
        enc(key, None, None, mods, KeyEventKind::Press, mode)
    }
    fn ch(c: &str) -> Key {
        Key::Character(c.into())
    }
    fn nk(n: NamedKey) -> Key {
        Key::Named(n)
    }
    fn s(x: &str) -> Option<String> {
        Some(x.to_string())
    }

    #[test]
    fn no_flags_is_legacy() {
        let m = TermMode::empty();
        assert_eq!(press(nk(NamedKey::Enter), SHIFT, m), None);
        assert_eq!(press(ch("a"), NONE, m), None);
        assert_eq!(press(ch("c"), CTRL, m), None);
    }

    #[test]
    fn disambiguate() {
        assert_eq!(press(nk(NamedKey::Enter), SHIFT, D), s("\x1b[13;2u"));
        assert_eq!(press(nk(NamedKey::Enter), NONE, D), None);
        assert_eq!(press(nk(NamedKey::Escape), NONE, D), s("\x1b[27u"));
        assert_eq!(press(ch("c"), CTRL, D), s("\x1b[99;5u"));
        assert_eq!(press(ch("x"), ALT, D), s("\x1b[120;3u"));
        assert_eq!(press(ch("a"), NONE, D), None);
        assert_eq!(press(ch("a"), SHIFT, D), None);
        assert_eq!(press(nk(NamedKey::ArrowUp), CTRL, D), s("\x1b[1;5A"));
        assert_eq!(press(nk(NamedKey::F5), NONE, D), None);
        assert_eq!(press(nk(NamedKey::F5), CTRL, D), s("\x1b[15;5~"));
        assert_eq!(press(nk(NamedKey::Tab), SHIFT, D), s("\x1b[9;2u"));
    }

    #[test]
    fn event_types() {
        let m = D | E;
        // A text key has no release or repeat events without flag 8.
        assert_eq!(
            enc(ch("a"), None, None, NONE, KeyEventKind::Release, m),
            None
        );
        assert_eq!(
            enc(ch("a"), None, None, NONE, KeyEventKind::Repeat, m),
            None
        );
        assert_eq!(
            enc(ch("a"), None, None, NONE, KeyEventKind::Release, m | A),
            s("\x1b[97;1:3u")
        );
        assert_eq!(
            enc(
                nk(NamedKey::Enter),
                None,
                None,
                NONE,
                KeyEventKind::Release,
                m
            ),
            None
        );
        assert_eq!(
            enc(
                nk(NamedKey::Enter),
                None,
                None,
                NONE,
                KeyEventKind::Release,
                m | A
            ),
            s("\x1b[13;1:3u")
        );
        assert_eq!(
            enc(ch("c"), None, None, CTRL, KeyEventKind::Repeat, m),
            s("\x1b[99;5:2u")
        );
        // Without flag 2 releases are not sent and a repeat is reported as a press.
        assert_eq!(
            enc(ch("c"), None, None, CTRL, KeyEventKind::Release, D),
            None
        );
        assert_eq!(
            enc(ch("c"), None, None, CTRL, KeyEventKind::Repeat, D),
            s("\x1b[99;5u")
        );
        assert_eq!(
            enc(
                nk(NamedKey::Enter),
                None,
                None,
                SHIFT,
                KeyEventKind::Repeat,
                D
            ),
            s("\x1b[13;2u")
        );
        assert_eq!(
            enc(
                nk(NamedKey::ArrowUp),
                None,
                None,
                NONE,
                KeyEventKind::Release,
                m
            ),
            s("\x1b[1;1:3A")
        );
    }

    #[test]
    fn all_keys_as_escapes() {
        assert_eq!(press(ch("a"), NONE, A), s("\x1b[97u"));
        assert_eq!(press(nk(NamedKey::Enter), NONE, A), s("\x1b[13u"));
        assert_eq!(press(nk(NamedKey::Shift), SHIFT, A), s("\x1b[57441;2u"));
        assert_eq!(press(nk(NamedKey::F1), NONE, A), s("\x1b[P"));
        // A modifier key alone is nothing without flag 8.
        assert_eq!(press(nk(NamedKey::Shift), SHIFT, D), None);
    }

    #[test]
    fn f3_is_tilde_13() {
        assert_eq!(press(nk(NamedKey::F3), NONE, A), s("\x1b[13~"));
        assert_eq!(press(nk(NamedKey::F3), CTRL, D), s("\x1b[13;5~"));
    }

    #[test]
    fn right_hand_modifiers() {
        let side = |n: NamedKey, right: bool, mods: Mods| {
            encode_kitty(
                &KittyKey {
                    key: &Key::Named(n),
                    shifted: None,
                    text: None,
                    mods,
                    kind: KeyEventKind::Press,
                    right,
                },
                A,
            )
            .map(|b| String::from_utf8(b).unwrap())
        };
        assert_eq!(side(NamedKey::Shift, false, NONE), s("\x1b[57441;2u"));
        assert_eq!(side(NamedKey::Shift, true, NONE), s("\x1b[57447;2u"));
        assert_eq!(side(NamedKey::Control, true, NONE), s("\x1b[57448;5u"));
        assert_eq!(side(NamedKey::Alt, true, NONE), s("\x1b[57449;3u"));
        assert_eq!(side(NamedKey::Super, true, NONE), s("\x1b[57450;9u"));
        assert_eq!(side(NamedKey::Control, false, CTRL), s("\x1b[57442;5u"));
    }

    #[test]
    fn alternate_keys() {
        let m = D | TermMode::REPORT_ALTERNATE_KEYS | A;
        assert_eq!(
            enc(ch("a"), Some('A'), None, SHIFT, KeyEventKind::Press, m),
            s("\x1b[97:65;2u")
        );
    }

    #[test]
    fn associated_text() {
        let m = A | TermMode::REPORT_ASSOCIATED_TEXT;
        // The spec's own example for no modifiers is `CSI 0 ; ; 229 u`: the field is blank.
        assert_eq!(
            enc(ch("a"), None, Some("a"), NONE, KeyEventKind::Press, m),
            s("\x1b[97;;97u")
        );
        assert_eq!(
            enc(ch("a"), Some('A'), Some("A"), SHIFT, KeyEventKind::Press, m),
            s("\x1b[97;2;65u")
        );
    }
}
