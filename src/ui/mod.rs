//! Pure chrome state and geometry (no GL).

use std::time::Duration;

use crate::layout::Rect;

pub mod confirm;
pub mod findbar;
pub mod icons;
pub mod menu;
pub mod overlay;
pub mod panel;
pub mod scrollbar;
pub mod text;
pub mod textfield;
pub mod theme;
pub mod toast;
pub mod topbar;

/// Two left presses on the same target within this time are a double-click
/// (the top bar's, and the terminal's multi-click selection).
pub const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// The window's edge-resize zone width and the pane overlay's inset from the
/// pane's top and right edges, at scale 1. One value for both, so the overlay
/// (inset by it) never reaches into the edge zone.
pub const EDGE_PX: i32 = 4;

/// `EDGE_PX` at `scale`: `round(EDGE_PX × scale)`, at least 1 px.
pub fn edge_px(scale: f64) -> i32 {
    ((f64::from(EDGE_PX) * scale).round() as i32).max(1)
}

/// A modal's size along one axis: what it wants, at most 90 % of the window's `win` px.
pub fn modal_extent(want: i32, win: i32) -> i32 {
    want.min(win * 9 / 10).max(0)
}

/// A `w` × `h` box centered in a `win_w` × `win_h` window (from the top-left corner when
/// it doesn't fit).
pub fn centered(w: i32, h: i32, win_w: i32, win_h: i32) -> Rect {
    Rect {
        x: ((win_w - w) / 2).max(0),
        y: ((win_h - h) / 2).max(0),
        w,
        h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_px_scales_and_rounds() {
        assert_eq!(edge_px(1.0), 4);
        assert_eq!(edge_px(1.5), 6);
        assert_eq!(edge_px(2.0), 8);
        assert_eq!(edge_px(1.1), 4);
        // Never zero, however small the scale.
        assert_eq!(edge_px(0.05), 1);
    }
}
