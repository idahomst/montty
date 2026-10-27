//! Key press -> xterm byte sequence written to the PTY.

use alacritty_terminal::term::TermMode;
use winit::keyboard::{Key, NamedKey};

use super::Mods;

pub struct KeyInput<'a> {
    /// The key without modifiers applied.
    pub key: &'a Key,
    /// Text reported by winit for this press (may be absent or a control char).
    pub text: Option<&'a str>,
    pub mods: Mods,
}

pub fn encode_key(input: &KeyInput<'_>, mode: TermMode) -> Option<Vec<u8>> {
    let mods = input.mods;
    if mods.logo {
        return None;
    }
    // xterm modifier parameter: 1 + shift + 2*alt + 4*ctrl.
    let param = 1 + u8::from(mods.shift) + 2 * u8::from(mods.alt) + 4 * u8::from(mods.ctrl);

    match input.key {
        Key::Named(named) => encode_named(*named, mods, param, mode),
        Key::Character(s) => encode_char(s.chars().next()?, input.text, mods),
        _ => None,
    }
}

fn alt_prefixed(mods: Mods, mut bytes: Vec<u8>) -> Vec<u8> {
    if mods.alt {
        bytes.insert(0, 0x1b);
    }
    bytes
}

/// `ESC [ 1 ; m <final>` with modifiers, otherwise `ESC [ <final>` or `ESC O <final>`.
fn csi_or_ss3(final_byte: u8, param: u8, ss3: bool) -> Vec<u8> {
    if param > 1 {
        format!("\x1b[1;{param}{}", final_byte as char).into_bytes()
    } else if ss3 {
        vec![0x1b, b'O', final_byte]
    } else {
        vec![0x1b, b'[', final_byte]
    }
}

/// `ESC [ <code> ~` with an optional `; m` modifier parameter.
fn tilde(code: u8, param: u8) -> Vec<u8> {
    if param > 1 {
        format!("\x1b[{code};{param}~").into_bytes()
    } else {
        format!("\x1b[{code}~").into_bytes()
    }
}

fn encode_named(key: NamedKey, mods: Mods, param: u8, mode: TermMode) -> Option<Vec<u8>> {
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    let bytes = match key {
        NamedKey::Enter => alt_prefixed(mods, vec![b'\r']),
        NamedKey::Backspace => alt_prefixed(mods, vec![if mods.ctrl { 0x08 } else { 0x7f }]),
        NamedKey::Tab if mods.shift => b"\x1b[Z".to_vec(),
        NamedKey::Tab => alt_prefixed(mods, vec![b'\t']),
        NamedKey::Escape => alt_prefixed(mods, vec![0x1b]),
        NamedKey::Space => return encode_char(' ', None, mods),
        NamedKey::ArrowUp => csi_or_ss3(b'A', param, app_cursor),
        NamedKey::ArrowDown => csi_or_ss3(b'B', param, app_cursor),
        NamedKey::ArrowRight => csi_or_ss3(b'C', param, app_cursor),
        NamedKey::ArrowLeft => csi_or_ss3(b'D', param, app_cursor),
        NamedKey::Home => csi_or_ss3(b'H', param, app_cursor),
        NamedKey::End => csi_or_ss3(b'F', param, app_cursor),
        NamedKey::Insert => tilde(2, param),
        NamedKey::Delete => tilde(3, param),
        NamedKey::PageUp => tilde(5, param),
        NamedKey::PageDown => tilde(6, param),
        NamedKey::F1 => csi_or_ss3(b'P', param, true),
        NamedKey::F2 => csi_or_ss3(b'Q', param, true),
        NamedKey::F3 => csi_or_ss3(b'R', param, true),
        NamedKey::F4 => csi_or_ss3(b'S', param, true),
        NamedKey::F5 => tilde(15, param),
        NamedKey::F6 => tilde(17, param),
        NamedKey::F7 => tilde(18, param),
        NamedKey::F8 => tilde(19, param),
        NamedKey::F9 => tilde(20, param),
        NamedKey::F10 => tilde(21, param),
        NamedKey::F11 => tilde(23, param),
        NamedKey::F12 => tilde(24, param),
        NamedKey::F13 => tilde(25, param),
        NamedKey::F14 => tilde(26, param),
        NamedKey::F15 => tilde(28, param),
        NamedKey::F16 => tilde(29, param),
        NamedKey::F17 => tilde(31, param),
        NamedKey::F18 => tilde(32, param),
        NamedKey::F19 => tilde(33, param),
        NamedKey::F20 => tilde(34, param),
        _ => return None,
    };
    Some(bytes)
}

/// Control byte for Ctrl + `c` (`c` is the unmodified key), if one is defined.
fn control_byte(c: char) -> Option<u8> {
    match c {
        'a'..='z' => Some(c as u8 - b'a' + 1),
        'A'..='Z' => Some(c as u8 - b'A' + 1),
        ' ' | '@' => Some(0x00),
        '[' => Some(0x1b),
        '\\' => Some(0x1c),
        ']' => Some(0x1d),
        '^' => Some(0x1e),
        '_' | '/' | '-' => Some(0x1f),
        '?' => Some(0x7f),
        // xterm's digit-row controls: Ctrl+2 = NUL, Ctrl+3..7 = ESC..US, Ctrl+8 = DEL.
        '2' => Some(0x00),
        '3'..='7' => Some(c as u8 - b'3' + 0x1b),
        '8' => Some(0x7f),
        _ => None,
    }
}

fn encode_char(key_char: char, text: Option<&str>, mods: Mods) -> Option<Vec<u8>> {
    if mods.ctrl
        && let Some(byte) = control_byte(key_char)
    {
        return Some(alt_prefixed(mods, vec![byte]));
    }
    // Typed text wins unless it is empty or a control char (Ctrl/Alt combos).
    let typed = text.filter(|t| !t.is_empty() && !t.chars().any(char::is_control));
    let bytes = match typed {
        Some(t) => t.as_bytes().to_vec(),
        None => {
            let c = if mods.shift {
                key_char.to_uppercase().next()?
            } else {
                key_char
            };
            c.to_string().into_bytes()
        }
    };
    Some(alt_prefixed(mods, bytes))
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

    fn enc(key: Key, text: Option<&str>, mods: Mods, mode: TermMode) -> Option<Vec<u8>> {
        encode_key(
            &KeyInput {
                key: &key,
                text,
                mods,
            },
            mode,
        )
    }
    fn named(k: NamedKey, mods: Mods) -> Option<Vec<u8>> {
        enc(Key::Named(k), None, mods, TermMode::empty())
    }
    fn ch(c: &str, mods: Mods) -> Option<Vec<u8>> {
        enc(Key::Character(c.into()), None, mods, TermMode::empty())
    }
    fn b(s: &[u8]) -> Option<Vec<u8>> {
        Some(s.to_vec())
    }

    #[test]
    fn text_and_control_keys() {
        assert_eq!(
            enc(
                Key::Character("a".into()),
                Some("a"),
                NONE,
                TermMode::empty()
            ),
            b(b"a")
        );
        assert_eq!(
            enc(
                Key::Character("ž".into()),
                Some("ž"),
                NONE,
                TermMode::empty()
            ),
            b("ž".as_bytes())
        );
        assert_eq!(
            enc(
                Key::Character("a".into()),
                Some("A"),
                SHIFT,
                TermMode::empty()
            ),
            b(b"A")
        );
        assert_eq!(named(NamedKey::Enter, NONE), b(b"\r"));
        assert_eq!(named(NamedKey::Backspace, NONE), b(b"\x7f"));
        assert_eq!(named(NamedKey::Backspace, CTRL), b(b"\x08"));
        assert_eq!(named(NamedKey::Tab, NONE), b(b"\t"));
        assert_eq!(named(NamedKey::Tab, SHIFT), b(b"\x1b[Z"));
        assert_eq!(named(NamedKey::Escape, NONE), b(b"\x1b"));
        assert_eq!(named(NamedKey::Space, NONE), b(b" "));
    }

    #[test]
    fn arrows() {
        assert_eq!(named(NamedKey::ArrowUp, NONE), b(b"\x1b[A"));
        assert_eq!(
            enc(
                Key::Named(NamedKey::ArrowUp),
                None,
                NONE,
                TermMode::APP_CURSOR
            ),
            b(b"\x1bOA")
        );
        assert_eq!(named(NamedKey::ArrowUp, CTRL), b(b"\x1b[1;5A"));
        assert_eq!(
            enc(
                Key::Named(NamedKey::ArrowUp),
                None,
                CTRL,
                TermMode::APP_CURSOR
            ),
            b(b"\x1b[1;5A")
        );
        let sa = Mods {
            shift: true,
            alt: true,
            ..NONE
        };
        assert_eq!(named(NamedKey::ArrowRight, sa), b(b"\x1b[1;4C"));
        assert_eq!(named(NamedKey::ArrowDown, NONE), b(b"\x1b[B"));
        assert_eq!(named(NamedKey::ArrowLeft, NONE), b(b"\x1b[D"));
    }

    #[test]
    fn navigation() {
        assert_eq!(named(NamedKey::Home, NONE), b(b"\x1b[H"));
        assert_eq!(
            enc(Key::Named(NamedKey::Home), None, NONE, TermMode::APP_CURSOR),
            b(b"\x1bOH")
        );
        assert_eq!(named(NamedKey::End, NONE), b(b"\x1b[F"));
        assert_eq!(named(NamedKey::PageUp, NONE), b(b"\x1b[5~"));
        assert_eq!(named(NamedKey::PageDown, NONE), b(b"\x1b[6~"));
        assert_eq!(named(NamedKey::Insert, NONE), b(b"\x1b[2~"));
        assert_eq!(named(NamedKey::Delete, NONE), b(b"\x1b[3~"));
        assert_eq!(named(NamedKey::Delete, CTRL), b(b"\x1b[3;5~"));
    }

    #[test]
    fn function_keys() {
        assert_eq!(named(NamedKey::F1, NONE), b(b"\x1bOP"));
        assert_eq!(named(NamedKey::F4, NONE), b(b"\x1bOS"));
        assert_eq!(named(NamedKey::F5, NONE), b(b"\x1b[15~"));
        assert_eq!(named(NamedKey::F12, NONE), b(b"\x1b[24~"));
        assert_eq!(named(NamedKey::F5, CTRL), b(b"\x1b[15;5~"));
        assert_eq!(named(NamedKey::F1, CTRL), b(b"\x1b[1;5P"));
    }

    #[test]
    fn control_characters() {
        assert_eq!(ch("c", CTRL), b(&[0x03]));
        assert_eq!(ch(" ", CTRL), b(&[0x00]));
        assert_eq!(ch("[", CTRL), b(&[0x1b]));
        assert_eq!(ch("\\", CTRL), b(&[0x1c]));
        assert_eq!(ch("]", CTRL), b(&[0x1d]));
        // Text is a control char already; the key still decides.
        assert_eq!(
            enc(
                Key::Character("c".into()),
                Some("\u{3}"),
                CTRL,
                TermMode::empty()
            ),
            b(&[0x03])
        );
    }

    #[test]
    fn xterm_control_bytes_for_digits_and_punctuation() {
        assert_eq!(ch("/", CTRL), b(&[0x1f]));
        assert_eq!(ch("-", CTRL), b(&[0x1f]));
        assert_eq!(ch("2", CTRL), b(&[0x00]));
        assert_eq!(ch("3", CTRL), b(&[0x1b]));
        assert_eq!(ch("4", CTRL), b(&[0x1c]));
        assert_eq!(ch("5", CTRL), b(&[0x1d]));
        assert_eq!(ch("6", CTRL), b(&[0x1e]));
        assert_eq!(ch("7", CTRL), b(&[0x1f]));
        assert_eq!(ch("8", CTRL), b(&[0x7f]));
        // Ctrl+Shift+- (Ctrl+_) is 0x1f too, not a literal '-'.
        let cs = Mods {
            ctrl: true,
            shift: true,
            ..NONE
        };
        assert_eq!(ch("-", cs), b(&[0x1f]));
        // Digits without a control byte stay digits.
        assert_eq!(ch("1", CTRL), b(b"1"));
        assert_eq!(ch("9", CTRL), b(b"9"));
    }

    #[test]
    fn alt_prefix() {
        assert_eq!(ch("x", ALT), b(b"\x1bx"));
        let ca = Mods {
            ctrl: true,
            alt: true,
            ..NONE
        };
        assert_eq!(ch("c", ca), b(b"\x1b\x03"));
        assert_eq!(named(NamedKey::Enter, ALT), b(b"\x1b\r"));
    }

    #[test]
    fn no_encoding() {
        assert_eq!(named(NamedKey::Shift, SHIFT), None);
        assert_eq!(named(NamedKey::CapsLock, NONE), None);
        let logo = Mods { logo: true, ..NONE };
        assert_eq!(ch("a", logo), None);
        assert_eq!(named(NamedKey::ArrowUp, logo), None);
    }
}
