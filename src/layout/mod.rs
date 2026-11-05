//! Pure-data tiling layout: geometry and the split tree.

pub mod geom;
pub mod tree;

pub use geom::{
    DIVIDER_HIT_PX, DIVIDER_PX, Direction, MIN_PANE_PX, Orientation, PaneId, Rect, px_i32,
};
pub use tree::{Divider, DividerId, RemoveResult, Shape, SplitTree};
