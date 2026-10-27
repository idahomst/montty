//! Pixel to grid mapping for the mouse, and per-cell multi-click counting.

use std::time::Instant;

use alacritty_terminal::index::{Column, Line, Point, Side};

use crate::layout::Rect;
use crate::ui::DOUBLE_CLICK;

/// The cell under window pixel (x, y) and the half of it the pixel is in.
/// `content` is the pane rect less its padding; x and y are clamped into the
/// grid. `Point.line` counts from the viewport top less `display_offset`
/// (negative = history).
#[allow(clippy::too_many_arguments)]
pub fn point_at(
    content: Rect,
    cell_w: u32,
    cell_h: u32,
    cols: usize,
    lines: usize,
    display_offset: usize,
    x: i32,
    y: i32,
) -> (Point, Side) {
    let (cw, ch) = (i64::from(cell_w.max(1)), i64::from(cell_h.max(1)));
    let dx = i64::from(x) - i64::from(content.x);
    let dy = i64::from(y) - i64::from(content.y);
    let last_col = cols.saturating_sub(1) as i64;
    let last_row = lines.saturating_sub(1) as i64;
    let col = dx.div_euclid(cw).clamp(0, last_col);
    let row = dy.div_euclid(ch).clamp(0, last_row);
    // Outside the grid the side is the edge's: Left before it, Right past it.
    let side = if dx < 0 || (dx < cols as i64 * cw && dx.rem_euclid(cw) * 2 < cw) {
        Side::Left
    } else {
        Side::Right
    };
    (
        Point::new(
            Line(row as i32 - display_offset as i32),
            Column(col as usize),
        ),
        side,
    )
}

/// Counts presses on the same cell within the double-click time.
#[derive(Default)]
pub struct MultiClick {
    last: Option<(Instant, Point, u8)>,
}

impl MultiClick {
    /// Registers a press on `p` at `now`: 1 (single), 2 (double) or 3 (triple);
    /// a fourth press starts over at 1.
    pub fn press(&mut self, p: Point, now: Instant) -> u8 {
        let n = match self.last {
            Some((at, last, n))
                if last == p && now.saturating_duration_since(at) < DOUBLE_CLICK && n < 3 =>
            {
                n + 1
            }
            _ => 1,
        };
        self.last = Some((now, p, n));
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const CONTENT: Rect = Rect {
        x: 4,
        y: 30,
        w: 80,
        h: 64,
    };

    fn at(x: i32, y: i32, offset: usize) -> (Point, Side) {
        point_at(CONTENT, 8, 16, 10, 4, offset, x, y)
    }

    fn pt(line: i32, col: usize) -> Point {
        Point::new(Line(line), Column(col))
    }

    #[test]
    fn point_at_maps_cells_and_sides() {
        assert_eq!(at(4, 30, 0), (pt(0, 0), Side::Left));
        assert_eq!(at(11, 30, 0), (pt(0, 0), Side::Right));
        assert_eq!(at(12, 46, 0), (pt(1, 1), Side::Left));
    }

    #[test]
    fn point_at_clamps_outside() {
        assert_eq!(at(5000, 5000, 0), (pt(3, 9), Side::Right));
        assert_eq!(at(-50, -50, 0), (pt(0, 0), Side::Left));
    }

    #[test]
    fn point_at_uses_display_offset() {
        assert_eq!(at(4, 30, 3).0, pt(-3, 0));
        assert_eq!(at(4, 46, 3).0, pt(-2, 0));
    }

    #[test]
    fn multi_click_counts_same_cell() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let mut c = MultiClick::default();
        assert_eq!(c.press(pt(0, 0), ms(0)), 1);
        assert_eq!(c.press(pt(0, 0), ms(100)), 2);
        assert_eq!(c.press(pt(0, 0), ms(200)), 3);
        assert_eq!(c.press(pt(0, 0), ms(300)), 1);
        // Another cell starts over.
        assert_eq!(c.press(pt(0, 1), ms(350)), 1);
        // Too slow.
        assert_eq!(c.press(pt(0, 1), ms(750)), 1);
        assert_eq!(c.press(pt(0, 1), ms(800)), 2);
    }
}
