//! Mouse reporting: encode pointer events for programs that asked for them
//! (xterm modes 1000/1002/1003 with X10, UTF-8 or SGR coordinates).

use alacritty_terminal::term::TermMode;

use super::Mods;

/// Ordered as xterm numbers them (Left lowest), which picks the button a drag reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MouseButtonCode {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseEventKind {
    Press,
    Release,
    Motion,
}

/// Whether pointer events go to the program: it asked for them and Shift is
/// not held (Shift bypasses, so text can still be selected).
pub fn reporting(mode: TermMode, shift: bool) -> bool {
    mode.intersects(TermMode::MOUSE_MODE) && !shift
}

/// The bytes reporting this event, or None when `mode` doesn't want it.
/// `button` is the button held (None: no button, for plain motion); `col` and
/// `line` are 0-based screen cells.
pub fn report(
    button: Option<MouseButtonCode>,
    kind: MouseEventKind,
    col: usize,
    line: usize,
    mods: Mods,
    mode: TermMode,
) -> Option<Vec<u8>> {
    if !mode.intersects(TermMode::MOUSE_MODE) {
        return None;
    }
    if kind == MouseEventKind::Motion
        && !mode.contains(TermMode::MOUSE_MOTION)
        && !(button.is_some() && mode.contains(TermMode::MOUSE_DRAG))
    {
        return None;
    }
    let sgr = mode.contains(TermMode::SGR_MOUSE);
    let mut code = match button {
        Some(MouseButtonCode::Left) => 0,
        Some(MouseButtonCode::Middle) => 1,
        Some(MouseButtonCode::Right) => 2,
        Some(MouseButtonCode::WheelUp) => 64,
        Some(MouseButtonCode::WheelDown) => 65,
        None => 3,
    };
    match kind {
        MouseEventKind::Release => match button {
            // A wheel notch has no release.
            Some(MouseButtonCode::WheelUp | MouseButtonCode::WheelDown) => return None,
            _ if !sgr => code = 3,
            _ => {}
        },
        MouseEventKind::Motion => code += 32,
        MouseEventKind::Press => {}
    }
    code += 4 * u32::from(mods.shift) + 8 * u32::from(mods.alt) + 16 * u32::from(mods.ctrl);
    if sgr {
        let end = if kind == MouseEventKind::Release {
            'm'
        } else {
            'M'
        };
        return Some(format!("\x1b[<{code};{};{}{end}", col + 1, line + 1).into_bytes());
    }
    let mut out = b"\x1b[M".to_vec();
    out.push(32 + code as u8);
    if mode.contains(TermMode::UTF8_MOUSE) {
        // Two-byte UTF-8 holds values up to 2047.
        for v in [col, line] {
            let c = char::from_u32((32 + v + 1).min(2047) as u32).unwrap_or(' ');
            let mut buf = [0; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        }
    } else {
        out.push(32 + (col + 1).min(223) as u8);
        out.push(32 + (line + 1).min(223) as u8);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use MouseButtonCode::*;
    use MouseEventKind::*;

    const NONE: Mods = Mods {
        ctrl: false,
        alt: false,
        shift: false,
        logo: false,
    };
    const CLICK: TermMode = TermMode::MOUSE_REPORT_CLICK;
    const SGR: TermMode = TermMode::MOUSE_REPORT_CLICK.union(TermMode::SGR_MOUSE);

    #[test]
    fn report_sgr_press_release() {
        assert_eq!(
            report(Some(Left), Press, 4, 9, NONE, SGR),
            Some(b"\x1b[<0;5;10M".to_vec())
        );
        assert_eq!(
            report(Some(Right), Release, 4, 9, NONE, SGR),
            Some(b"\x1b[<2;5;10m".to_vec())
        );
    }

    #[test]
    fn report_x10_clamps_past_223() {
        assert_eq!(
            report(Some(Left), Press, 0, 1, NONE, CLICK),
            Some(vec![0x1b, b'[', b'M', 32, 33, 34])
        );
        // Release is button 3 in the legacy encodings.
        assert_eq!(
            report(Some(Left), Release, 0, 0, NONE, CLICK),
            Some(vec![0x1b, b'[', b'M', 35, 33, 33])
        );
        assert_eq!(
            report(Some(Left), Press, 500, 223, NONE, CLICK),
            Some(vec![0x1b, b'[', b'M', 32, 255, 255])
        );
    }

    #[test]
    fn report_utf8_large_coords() {
        let mode = CLICK | TermMode::UTF8_MOUSE;
        let out = report(Some(Left), Press, 300, 0, NONE, mode).unwrap();
        let mut want = b"\x1b[M ".to_vec();
        want.extend_from_slice(char::from_u32(32 + 301).unwrap().to_string().as_bytes());
        want.push(33);
        assert_eq!(out, want);
    }

    #[test]
    fn wheel_reports_buttons_64_65() {
        assert_eq!(
            report(Some(WheelUp), Press, 0, 0, NONE, SGR),
            Some(b"\x1b[<64;1;1M".to_vec())
        );
        assert_eq!(
            report(Some(WheelDown), Press, 0, 0, NONE, SGR),
            Some(b"\x1b[<65;1;1M".to_vec())
        );
        assert_eq!(report(Some(WheelUp), Release, 0, 0, NONE, SGR), None);
    }

    #[test]
    fn motion_needs_mode() {
        assert_eq!(report(Some(Left), Motion, 1, 1, NONE, SGR), None);
        let drag = SGR | TermMode::MOUSE_DRAG;
        assert_eq!(
            report(Some(Left), Motion, 1, 1, NONE, drag),
            Some(b"\x1b[<32;2;2M".to_vec())
        );
        // No button held: only any-motion mode reports.
        assert_eq!(report(None, Motion, 1, 1, NONE, drag), None);
        let any = SGR | TermMode::MOUSE_MOTION;
        assert_eq!(
            report(None, Motion, 1, 1, NONE, any),
            Some(b"\x1b[<35;2;2M".to_vec())
        );
        assert_eq!(
            report(Some(Left), Press, 0, 0, NONE, TermMode::empty()),
            None
        );
    }

    #[test]
    fn shift_bypasses_reporting() {
        assert!(reporting(CLICK, false));
        assert!(!reporting(CLICK, true));
        assert!(!reporting(TermMode::empty(), false));
        assert!(!reporting(TermMode::SGR_MOUSE, false));
    }

    #[test]
    fn modifier_bits() {
        let m = Mods {
            ctrl: true,
            alt: true,
            shift: true,
            logo: false,
        };
        // 0 + 4 shift + 8 alt + 16 ctrl
        assert_eq!(
            report(Some(Left), Press, 0, 0, m, SGR),
            Some(b"\x1b[<28;1;1M".to_vec())
        );
    }

    #[test]
    fn utf8_encodes_the_line_too_and_caps_at_2047() {
        let mode = CLICK | TermMode::UTF8_MOUSE;
        let out = report(Some(Left), Press, 0, 300, NONE, mode).unwrap();
        let mut want = b"\x1b[M !".to_vec();
        want.extend_from_slice(char::from_u32(333).unwrap().to_string().as_bytes());
        assert_eq!(out, want);
        let out = report(Some(Left), Press, 5000, 5000, NONE, mode).unwrap();
        let cap = char::from_u32(2047).unwrap().to_string();
        let mut want = b"\x1b[M ".to_vec();
        want.extend_from_slice(cap.as_bytes());
        want.extend_from_slice(cap.as_bytes());
        assert_eq!(out, want);
    }

    #[test]
    fn modifiers_apply_to_x10_and_motion_codes() {
        let shift = Mods {
            shift: true,
            ..NONE
        };
        assert_eq!(
            report(Some(Left), Press, 0, 0, shift, CLICK),
            Some(vec![0x1b, b'[', b'M', 36, 33, 33])
        );
        let ctrl = Mods { ctrl: true, ..NONE };
        let drag = SGR | TermMode::MOUSE_DRAG;
        assert_eq!(
            report(Some(Left), Motion, 0, 0, ctrl, drag),
            Some(b"\x1b[<48;1;1M".to_vec())
        );
    }
}
