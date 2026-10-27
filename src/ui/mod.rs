//! Self-drawn UI chrome primitives: theme, icons and text layout.

use std::time::Duration;

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
