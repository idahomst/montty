//! Resolves terminal cell colors (named / indexed / truecolor + flags) to RGB.

use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors;
pub use alacritty_terminal::vte::ansi::Rgb;
use alacritty_terminal::vte::ansi::{Color, NamedColor};

#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    pub fg: Rgb,
    pub bg: Rgb,
    pub cursor: Rgb,
    pub ansi: [Rgb; 16],
    /// Selection colors; `None` keeps the renderer's default (inverted cells).
    pub selection_fg: Option<Rgb>,
    pub selection_bg: Option<Rgb>,
}

const fn hex(v: u32) -> Rgb {
    Rgb {
        r: (v >> 16) as u8,
        g: (v >> 8) as u8,
        b: v as u8,
    }
}

/// Search match colors (selection beats the current match beats other matches).
pub const MATCH_FG: Rgb = hex(0x000000);
pub const MATCH_BG: Rgb = hex(0xE5C07B);
pub const CURRENT_FG: Rgb = hex(0x000000);
pub const CURRENT_BG: Rgb = hex(0xFF9E3B);

/// Scales every channel by 2/3, rounded to nearest.
fn dim(c: Rgb) -> Rgb {
    let f = |v: u8| ((u16::from(v) * 2 + 1) / 3) as u8;
    Rgb {
        r: f(c.r),
        g: f(c.g),
        b: f(c.b),
    }
}

pub fn tango_default() -> Palette {
    Palette {
        fg: hex(0xD3D7CF),
        bg: hex(0x1E1E1E),
        cursor: hex(0xD3D7CF),
        ansi: [
            hex(0x000000),
            hex(0xCC0000),
            hex(0x4D9A05),
            hex(0xC3A000),
            hex(0x3464A3),
            hex(0x754F7B),
            hex(0x05979A),
            hex(0xD3D6CF),
            hex(0x545652),
            hex(0xEF2828),
            hex(0x89E234),
            hex(0xFBE84F),
            hex(0x729ECF),
            hex(0xAC7EA8),
            hex(0x34E2E2),
            hex(0xEDEDEB),
        ],
        selection_fg: None,
        selection_bg: None,
    }
}

/// Palette color for an xterm 256-color index (no terminal override applied).
fn indexed(i: u8, p: &Palette) -> Rgb {
    match i {
        0..=15 => p.ansi[usize::from(i)],
        16..=231 => {
            const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
            let n = usize::from(i - 16);
            Rgb {
                r: LEVELS[n / 36],
                g: LEVELS[n / 6 % 6],
                b: LEVELS[n % 6],
            }
        }
        232..=255 => {
            let v = 8 + 10 * (i - 232);
            Rgb { r: v, g: v, b: v }
        }
    }
}

fn named(n: NamedColor, p: &Palette) -> Rgb {
    match n as usize {
        i @ 0..=15 => p.ansi[i],
        256 | 267 => p.fg, // Foreground, BrightForeground
        257 => p.bg,
        258 => p.cursor,
        i @ 259..=266 => dim(p.ansi[i - 259]), // Dim{Black..White}
        268 => dim(p.fg),                      // DimForeground
        _ => p.fg,
    }
}

pub fn resolve(c: Color, p: &Palette, term: &Colors) -> Rgb {
    match c {
        Color::Spec(rgb) => rgb,
        Color::Indexed(i) => term[usize::from(i)].unwrap_or_else(|| indexed(i, p)),
        Color::Named(n) => term[n].unwrap_or_else(|| named(n, p)),
    }
}

pub fn cell_colors(
    fg: Color,
    bg: Color,
    flags: Flags,
    p: &Palette,
    term: &Colors,
    bold_is_bright: bool,
) -> (Rgb, Rgb) {
    let fg = match fg {
        Color::Named(n) if bold_is_bright && flags.contains(Flags::BOLD) && (n as usize) < 8 => {
            Color::Indexed(n as u8 + 8)
        }
        Color::Indexed(i) if bold_is_bright && flags.contains(Flags::BOLD) && i < 8 => {
            Color::Indexed(i + 8)
        }
        other => other,
    };
    let mut fg = resolve(fg, p, term);
    let mut bg = resolve(bg, p, term);
    if flags.contains(Flags::DIM) {
        fg = dim(fg);
    }
    if flags.contains(Flags::INVERSE) {
        std::mem::swap(&mut fg, &mut bg);
    }
    if flags.contains(Flags::HIDDEN) {
        fg = bg;
    }
    (fg, bg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(r: u8, g: u8, b: u8) -> Rgb {
        Rgb { r, g, b }
    }

    #[test]
    fn named_red_is_ansi_1() {
        let p = tango_default();
        let t = Colors::default();
        assert_eq!(resolve(Color::Named(NamedColor::Red), &p, &t), p.ansi[1]);
        assert_eq!(
            resolve(Color::Named(NamedColor::BrightRed), &p, &t),
            p.ansi[9]
        );
    }

    #[test]
    fn tango_values() {
        let p = tango_default();
        assert_eq!(p.ansi[1], rgb(0xCC, 0, 0));
        assert_eq!(p.ansi[15], rgb(0xED, 0xED, 0xEB));
        assert_eq!(p.fg, rgb(0xD3, 0xD7, 0xCF));
        assert_eq!(p.bg, rgb(0x1E, 0x1E, 0x1E));
        assert_eq!(p.cursor, rgb(0xD3, 0xD7, 0xCF));
    }

    #[test]
    fn indexed_cube_and_gray() {
        let p = tango_default();
        let t = Colors::default();
        assert_eq!(resolve(Color::Indexed(196), &p, &t), rgb(255, 0, 0));
        assert_eq!(resolve(Color::Indexed(16), &p, &t), rgb(0, 0, 0));
        assert_eq!(resolve(Color::Indexed(232), &p, &t), rgb(8, 8, 8));
        assert_eq!(resolve(Color::Indexed(255), &p, &t), rgb(238, 238, 238));
        assert_eq!(resolve(Color::Indexed(9), &p, &t), p.ansi[9]);
    }

    #[test]
    fn spec_passes_through() {
        let p = tango_default();
        let t = Colors::default();
        assert_eq!(resolve(Color::Spec(rgb(1, 2, 3)), &p, &t), rgb(1, 2, 3));
    }

    #[test]
    fn term_override_wins() {
        let p = tango_default();
        let mut t = Colors::default();
        t[1] = Some(rgb(9, 9, 9));
        assert_eq!(resolve(Color::Indexed(1), &p, &t), rgb(9, 9, 9));
        assert_eq!(resolve(Color::Named(NamedColor::Red), &p, &t), rgb(9, 9, 9));
        t[NamedColor::Foreground] = Some(rgb(7, 7, 7));
        assert_eq!(
            resolve(Color::Named(NamedColor::Foreground), &p, &t),
            rgb(7, 7, 7)
        );
    }

    #[test]
    fn special_named_use_palette() {
        let p = tango_default();
        let t = Colors::default();
        assert_eq!(resolve(Color::Named(NamedColor::Foreground), &p, &t), p.fg);
        assert_eq!(resolve(Color::Named(NamedColor::Background), &p, &t), p.bg);
        assert_eq!(resolve(Color::Named(NamedColor::Cursor), &p, &t), p.cursor);
    }

    #[test]
    fn dim_named_is_two_thirds() {
        let p = tango_default();
        let t = Colors::default();
        // 0xCC = 204 -> 136
        assert_eq!(
            resolve(Color::Named(NamedColor::DimRed), &p, &t),
            rgb(136, 0, 0)
        );
    }

    #[test]
    fn bold_is_bright() {
        let p = tango_default();
        let t = Colors::default();
        let red = Color::Named(NamedColor::Red);
        let bg = Color::Named(NamedColor::Background);
        assert_eq!(cell_colors(red, bg, Flags::BOLD, &p, &t, true).0, p.ansi[9]);
        assert_eq!(
            cell_colors(red, bg, Flags::BOLD, &p, &t, false).0,
            p.ansi[1]
        );
        assert_eq!(
            cell_colors(Color::Indexed(1), bg, Flags::BOLD, &p, &t, true).0,
            p.ansi[9]
        );
        // not promoted: bright, or no bold
        assert_eq!(
            cell_colors(Color::Indexed(9), bg, Flags::BOLD, &p, &t, true).0,
            p.ansi[9]
        );
        assert_eq!(
            cell_colors(red, bg, Flags::empty(), &p, &t, true).0,
            p.ansi[1]
        );
    }

    #[test]
    fn inverse_swaps() {
        let p = tango_default();
        let t = Colors::default();
        let fg = Color::Spec(rgb(10, 20, 30));
        let bg = Color::Spec(rgb(40, 50, 60));
        assert_eq!(
            cell_colors(fg, bg, Flags::INVERSE, &p, &t, true),
            (rgb(40, 50, 60), rgb(10, 20, 30))
        );
    }

    #[test]
    fn dim_scales_fg() {
        let p = tango_default();
        let t = Colors::default();
        let fg = Color::Spec(rgb(255, 100, 1));
        let bg = Color::Spec(rgb(0, 0, 0));
        // 255*2/3=170, 100*2/3=66.67->67, 1*2/3=0.67->1
        assert_eq!(
            cell_colors(fg, bg, Flags::DIM, &p, &t, true).0,
            rgb(170, 67, 1)
        );
    }

    #[test]
    fn hidden_fg_equals_bg() {
        let p = tango_default();
        let t = Colors::default();
        let fg = Color::Spec(rgb(10, 20, 30));
        let bg = Color::Spec(rgb(40, 50, 60));
        let (f, b) = cell_colors(fg, bg, Flags::HIDDEN, &p, &t, true);
        assert_eq!(f, b);
        assert_eq!(b, rgb(40, 50, 60));
        let (f, b) = cell_colors(fg, bg, Flags::HIDDEN | Flags::INVERSE, &p, &t, true);
        assert_eq!((f, b), (rgb(10, 20, 30), rgb(10, 20, 30)));
    }
}
