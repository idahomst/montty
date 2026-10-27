//! Basic geometry types and layout constants.

pub type PaneId = u64;

/// Smallest allowed pane extent (px) along a split axis.
pub const MIN_PANE_PX: i32 = 32;
/// Thickness of the divider line between two panes.
pub const DIVIDER_PX: i32 = 1;
/// Drag hit zone extends this many px on each side of the divider line.
pub const DIVIDER_HIT_PX: i32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }

    /// Shrink by `by` on every side, clamped at zero size.
    pub fn inset(&self, by: i32) -> Rect {
        Rect {
            x: self.x + by,
            y: self.y + by,
            w: (self.w - 2 * by).max(0),
            h: (self.h - 2 * by).max(0),
        }
    }
}

/// Horizontal = side by side ("add right"); Vertical = stacked ("add down").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}
