//! Dragging a pane by its overlay onto another pane to swap them: the pure
//! state machine, the drop target and its highlight. `mouse.rs` drives it.

use super::draw::border_rects;
use crate::layout::{PaneId, Rect};
use crate::render::RectInstance;
use crate::ui::overlay::OverlayTarget;
use crate::ui::theme;

/// Pointer travel (px at scale 1, per axis) a press must exceed to become a drag.
pub(super) const DRAG_THRESHOLD_PX: f64 = 6.0;

/// round(DRAG_THRESHOLD_PX * scale), at least 1.
pub(super) fn drag_threshold(scale: f64) -> i32 {
    ((DRAG_THRESHOLD_PX * scale).round() as i32).max(1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PaneDrag {
    /// Pressed on the overlay; still a click.
    Pressed {
        pane: PaneId,
        target: OverlayTarget,
        button: Rect,
        start: (i32, i32),
    },
    /// Moving `pane`; `over` is the drop target under the pointer.
    Dragging { pane: PaneId, over: Option<PaneId> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DragEnd {
    Click {
        pane: PaneId,
        target: OverlayTarget,
        button: Rect,
    },
    Swap {
        pane: PaneId,
        with: PaneId,
    },
    Cancel,
}

impl PaneDrag {
    /// Pointer at `pos`: Pressed becomes Dragging once |dx| or |dy| > threshold;
    /// Dragging updates `over`. True when anything changed (needs a frame).
    pub(super) fn motion(
        &mut self,
        pos: (i32, i32),
        inside: bool,
        threshold: i32,
        layout: &[(PaneId, Rect)],
    ) -> bool {
        let (pane, started) = match *self {
            PaneDrag::Pressed { pane, start, .. } => {
                let past =
                    (pos.0 - start.0).abs() > threshold || (pos.1 - start.1).abs() > threshold;
                if !past {
                    return false;
                }
                (pane, true)
            }
            PaneDrag::Dragging { pane, .. } => (pane, false),
        };
        let now = drop_target(layout, pane, pos, inside);
        let changed = started || !matches!(*self, PaneDrag::Dragging { over, .. } if over == now);
        *self = PaneDrag::Dragging { pane, over: now };
        changed
    }

    /// Left button released at `pos`, `layout` being the layout now.
    pub(super) fn release(
        self,
        pos: (i32, i32),
        inside: bool,
        layout: &[(PaneId, Rect)],
    ) -> DragEnd {
        match self {
            PaneDrag::Pressed {
                pane,
                target,
                button,
                ..
            } => DragEnd::Click {
                pane,
                target,
                button,
            },
            PaneDrag::Dragging { pane, .. } => match drop_target(layout, pane, pos, inside) {
                Some(with) => DragEnd::Swap { pane, with },
                None => DragEnd::Cancel,
            },
        }
    }
}

/// The pane under `pos` that `source` may be dropped on: another pane of `layout`,
/// None outside the window, on a divider, on the bar or on `source` itself.
pub(super) fn drop_target(
    layout: &[(PaneId, Rect)],
    source: PaneId,
    pos: (i32, i32),
    inside: bool,
) -> Option<PaneId> {
    if !inside {
        return None;
    }
    layout
        .iter()
        .find(|(_, r)| r.contains(pos.0, pos.1))
        .map(|&(id, _)| id)
        .filter(|&id| id != source)
}

/// The drop-target highlight: ACCENT at alpha 0.2 over `rect`, plus a 2×`stroke`
/// ACCENT border (`border_rects`).
pub(super) fn paint_drop_target(rect: Rect, stroke: i32) -> Vec<RectInstance> {
    let mut fill = theme::ACCENT;
    fill[3] = 0.2;
    let mut out = vec![RectInstance { rect, color: fill }];
    out.extend(
        border_rects(rect, 2 * stroke)
            .into_iter()
            .map(|rect| RectInstance {
                rect,
                color: theme::ACCENT,
            }),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const R1: Rect = Rect {
        x: 0,
        y: 30,
        w: 100,
        h: 100,
    };
    const R2: Rect = Rect {
        x: 101,
        y: 30,
        w: 100,
        h: 100,
    };
    const LAYOUT: [(PaneId, Rect); 2] = [(1, R1), (2, R2)];
    const BTN: Rect = Rect {
        x: 60,
        y: 34,
        w: 20,
        h: 20,
    };

    fn pressed(target: OverlayTarget) -> PaneDrag {
        PaneDrag::Pressed {
            pane: 1,
            target,
            button: BTN,
            start: (10, 40),
        }
    }

    fn dragging(over: Option<PaneId>) -> PaneDrag {
        PaneDrag::Dragging { pane: 1, over }
    }

    #[test]
    fn threshold_scales() {
        assert_eq!(drag_threshold(1.0), 6);
        assert_eq!(drag_threshold(2.0), 12);
        assert_eq!(drag_threshold(0.1), 1);
    }

    #[test]
    fn hold_still_stays_a_click() {
        let mut d = pressed(OverlayTarget::Close);
        assert!(!d.motion((10, 40), true, 6, &LAYOUT));
        // |d| == threshold is not past it.
        assert!(!d.motion((16, 46), true, 6, &LAYOUT));
        assert_eq!(d, pressed(OverlayTarget::Close));
        assert_eq!(
            d.release((16, 46), true, &LAYOUT),
            DragEnd::Click {
                pane: 1,
                target: OverlayTarget::Close,
                button: BTN
            }
        );
    }

    #[test]
    fn small_move_release_clicks_the_pressed_button() {
        let mut d = pressed(OverlayTarget::Maximize);
        assert!(!d.motion((12, 41), true, 6, &LAYOUT));
        // Released over another button (or anywhere): still the pressed one.
        let end = d.release((70, 44), true, &LAYOUT);
        assert!(matches!(
            end,
            DragEnd::Click {
                target: OverlayTarget::Maximize,
                ..
            }
        ));
    }

    #[test]
    fn past_threshold_starts_dragging() {
        let mut d = pressed(OverlayTarget::Grip);
        assert!(d.motion((17, 40), true, 6, &LAYOUT));
        assert_eq!(d, dragging(None));
        assert!(d.motion((150, 60), true, 6, &LAYOUT));
        assert_eq!(d, dragging(Some(2)));
        assert!(!d.motion((160, 70), true, 6, &LAYOUT));
        assert_eq!(d, dragging(Some(2)));
    }

    #[test]
    fn drop_on_other_pane_swaps() {
        assert_eq!(
            dragging(None).release((150, 60), true, &LAYOUT),
            DragEnd::Swap { pane: 1, with: 2 }
        );
    }

    #[test]
    fn drop_elsewhere_cancels() {
        let d = dragging(Some(2));
        assert_eq!(d.release((50, 60), true, &LAYOUT), DragEnd::Cancel);
        assert_eq!(d.release((100, 60), true, &LAYOUT), DragEnd::Cancel);
        assert_eq!(d.release((150, 10), true, &LAYOUT), DragEnd::Cancel);
        assert_eq!(d.release((150, 60), false, &LAYOUT), DragEnd::Cancel);
    }

    #[test]
    fn release_recomputes_target_from_current_layout() {
        let only_1 = [(1, R1)];
        assert_eq!(
            dragging(Some(2)).release((150, 60), true, &only_1),
            DragEnd::Cancel
        );
    }

    #[test]
    fn drop_target_highlight() {
        let out = paint_drop_target(R2, 1);
        let mut fill = theme::ACCENT;
        fill[3] = 0.2;
        assert_eq!(
            out[0],
            RectInstance {
                rect: R2,
                color: fill
            }
        );
        assert!(out.len() > 1);
        for b in &out[1..] {
            assert_eq!(b.color, theme::ACCENT);
            assert!(
                b.rect.x >= R2.x
                    && b.rect.y >= R2.y
                    && b.rect.x + b.rect.w <= R2.x + R2.w
                    && b.rect.y + b.rect.h <= R2.y + R2.h
            );
        }
    }
}
