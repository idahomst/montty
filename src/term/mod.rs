//! Terminal-side helpers: color resolution, panes, snapshots.

pub mod colors;
pub mod grid_point;
pub mod links;
pub mod osc_scan;
pub mod pane;
pub mod pty_tap;
pub mod snapshot;
pub mod status;

pub use crate::layout::PaneId;
