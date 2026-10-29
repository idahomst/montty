//! Chrome colors. Values are what `RectInstance.color` takes: sRGB bytes / 255 as-is,
//! straight alpha, no gamma conversion (as the configured chrome colors in draw.rs).

use crate::term::colors::Rgb;

/// `0xRRGGBB` to an opaque `RectInstance` color.
pub const fn rgb(hex: u32) -> [f32; 4] {
    rgba(hex, 1.0)
}

const fn rgba(hex: u32, a: f32) -> [f32; 4] {
    [
        ((hex >> 16) & 0xff) as f32 / 255.0,
        ((hex >> 8) & 0xff) as f32 / 255.0,
        (hex & 0xff) as f32 / 255.0,
        a,
    ]
}

/// An 8-bit color (from the config) to an opaque `RectInstance` color.
pub fn from_rgb(c: Rgb) -> [f32; 4] {
    rgb(u32::from(c.r) << 16 | u32::from(c.g) << 8 | u32::from(c.b))
}

/// Rect color back to the 8-bit color text is drawn in (alpha ignored).
pub fn to_rgb(c: [f32; 4]) -> Rgb {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgb {
        r: b(c[0]),
        g: b(c[1]),
        b: b(c[2]),
    }
}

pub const BAR_BG: [f32; 4] = rgb(0x2b2d31);
pub const TAB_INACTIVE: [f32; 4] = rgb(0x232428);
/// Same as the terminal background.
pub const TAB_ACTIVE: [f32; 4] = rgb(0x1e1e1e);
pub const HOVER: [f32; 4] = rgb(0x3a3d42);
pub const TEXT: [f32; 4] = rgb(0xdddddd);
pub const TEXT_DIM: [f32; 4] = rgb(0xaaaaaa);
pub const ICON: [f32; 4] = rgb(0xcccccc);
pub const CLOSE_HOVER: [f32; 4] = rgb(0xd9534f);
pub const PANEL_BG: [f32; 4] = rgb(0x2b2d31);
pub const PANEL_BORDER: [f32; 4] = rgb(0x444444);
/// `rgba(40,44,52,0.92)`.
/// Attention dots: needs you (amber), finished, failed.
pub const MARK_BLOCKED: [f32; 4] = rgb(0xe5a50a);
pub const MARK_DONE: [f32; 4] = rgb(0x3fb36b);
pub const MARK_ERROR: [f32; 4] = rgb(0xe05252);
pub const OVERLAY_BG: [f32; 4] = rgba(0x282c34, 0.92);
pub const OVERLAY_BORDER: [f32; 4] = rgb(0x3d4350);
pub const ACCENT: [f32; 4] = rgb(0x4a90d9);
/// Every toast reports a problem: dark red with a bright red border, so it stands out
/// from the terminals under it.
pub const TOAST_BG: [f32; 4] = rgb(0x5c1f1f);
pub const TOAST_BORDER: [f32; 4] = CLOSE_HOVER;
pub const TOAST_TEXT: [f32; 4] = rgb(0xffffff);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_matches_draw_constants() {
        assert_eq!(
            rgb(0x555555),
            [
                0x55 as f32 / 255.0,
                0x55 as f32 / 255.0,
                0x55 as f32 / 255.0,
                1.0
            ]
        );
        assert_eq!(
            ACCENT,
            [
                0x4a as f32 / 255.0,
                0x90 as f32 / 255.0,
                0xd9 as f32 / 255.0,
                1.0
            ]
        );
    }

    #[test]
    fn to_rgb_round_trips() {
        let c = to_rgb(rgb(0x4a90d9));
        assert_eq!((c.r, c.g, c.b), (0x4a, 0x90, 0xd9));
        assert_eq!(from_rgb(c), rgb(0x4a90d9));
    }

    #[test]
    fn overlay_has_alpha() {
        assert_eq!(OVERLAY_BG[3], 0.92);
    }
}
