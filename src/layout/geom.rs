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

    /// The overlap of `self` and `other`; None when they don't overlap.
    pub fn intersect(&self, other: Rect) -> Option<Rect> {
        let (x0, y0) = (self.x.max(other.x), self.y.max(other.y));
        let x1 = (self.x + self.w).min(other.x + other.w);
        let y1 = (self.y + self.h).min(other.y + other.h);
        (x1 > x0 && y1 > y0).then_some(Rect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intersect_clips_and_rejects_disjoint() {
        let a = Rect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
        };
        let b = Rect {
            x: 5,
            y: -5,
            w: 10,
            h: 10,
        };
        assert_eq!(
            a.intersect(b),
            Some(Rect {
                x: 5,
                y: 0,
                w: 5,
                h: 5
            })
        );
        let touching = Rect {
            x: 10,
            y: 0,
            w: 5,
            h: 5,
        };
        assert_eq!(a.intersect(touching), None);
    }
}
