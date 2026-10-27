//! The hover overlay of the pane under the pointer: Profile, Maximize/Restore
//! and Close buttons in the pane's top-right corner, and, when pane dragging is
//! on, a Grip in front of them. Pure layout and visibility.

use std::time::Instant;

use super::topbar::BarMetrics;
use crate::layout::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayTarget {
    Grip,
    Profile,
    Maximize,
    Close,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Overlay {
    pub bg: Rect,
    pub items: Vec<(Rect, OverlayTarget)>,
}

/// The overlay for `pane`, `inset` px in from its top and right edges (the
/// window passes `ui::edge_px(scale)`, the edge-resize zone's width): square
/// buttons of side `cell_h + inset`, Profile, Maximize, then Close, with Grip
/// in front when `grip` is set and the pane has room for four. A pane too
/// small to hold the three with `inset` to spare on each side gets an empty
/// overlay (no items, zero-sized bg): nothing is drawn or hit.
pub fn overlay_layout(pane: Rect, bm: &BarMetrics, inset: i32, grip: bool) -> Overlay {
    let side = bm.cell_h + inset;
    let fits = |n: i32| pane.w >= n * side + 2 * inset && pane.h >= side + 2 * inset;
    let mut targets = vec![
        OverlayTarget::Profile,
        OverlayTarget::Maximize,
        OverlayTarget::Close,
    ];
    if grip && fits(4) {
        targets.insert(0, OverlayTarget::Grip);
    }
    if !fits(3) {
        return Overlay {
            bg: Rect {
                x: pane.x,
                y: pane.y,
                w: 0,
                h: 0,
            },
            items: Vec::new(),
        };
    }
    let (w, h) = (targets.len() as i32 * side, side);
    let bg = Rect {
        x: pane.x + pane.w - inset - w,
        y: pane.y + inset,
        w,
        h,
    };
    Overlay {
        bg,
        items: targets
            .into_iter()
            .enumerate()
            .map(|(i, t)| {
                let r = Rect {
                    x: bg.x + i as i32 * side,
                    y: bg.y,
                    w: side,
                    h: side,
                };
                (r, t)
            })
            .collect(),
    }
}

/// What a press at (x, y) on `o` hits.
pub fn overlay_hit(o: &Overlay, x: i32, y: i32) -> Option<OverlayTarget> {
    o.items
        .iter()
        .find(|(r, _)| r.contains(x, y))
        .map(|&(_, t)| t)
}

/// Shown while the pointer is in the pane and has moved since the last key
/// press (window-wide: a key to any pane or binding hides it). Nothing animates: this changes only on motion, a key or the
/// pointer leaving.
pub fn overlay_visible(
    pointer_in_pane: bool,
    last_move: Instant,
    last_key: Option<Instant>,
) -> bool {
    pointer_in_pane && last_key.is_none_or(|k| last_move > k)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::topbar::resize_edge;
    use crate::ui::{EDGE_PX, edge_px};
    use std::time::Duration;

    fn bm() -> BarMetrics {
        BarMetrics {
            height: 28,
            pad: 6,
            cell_w: 8,
            cell_h: 16,
        }
    }

    const PANE: Rect = Rect {
        x: 100,
        y: 28,
        w: 300,
        h: 200,
    };

    fn inside(r: Rect, outer: Rect) -> bool {
        r.x >= outer.x
            && r.y >= outer.y
            && r.x + r.w <= outer.x + outer.w
            && r.y + r.h <= outer.y + outer.h
    }

    #[test]
    fn overlay_inside_pane_top_right() {
        let o = overlay_layout(PANE, &bm(), EDGE_PX, false);
        assert!(inside(o.bg, PANE), "{o:?}");
        assert_eq!(o.bg.x + o.bg.w, PANE.x + PANE.w - 4);
        assert_eq!(o.bg.y, PANE.y + 4);
        // Three buttons of side cell_h + 4 filling the bg.
        let side = 16 + 4;
        let button = |i: i32| Rect {
            x: PANE.x + PANE.w - 4 - (3 - i) * side,
            y: PANE.y + 4,
            w: side,
            h: side,
        };
        let rects: Vec<Rect> = o.items.iter().map(|&(r, _)| r).collect();
        assert_eq!(rects, vec![button(0), button(1), button(2)]);
        assert_eq!(o.bg.w, 3 * side);
        assert_eq!(o.bg.h, side);
    }

    #[test]
    fn overlay_three_buttons_order() {
        let o = overlay_layout(PANE, &bm(), EDGE_PX, false);
        let targets: Vec<OverlayTarget> = o.items.iter().map(|&(_, t)| t).collect();
        assert_eq!(
            targets,
            vec![
                OverlayTarget::Profile,
                OverlayTarget::Maximize,
                OverlayTarget::Close
            ]
        );
        for w in o.items.windows(2) {
            assert_eq!(w[0].0.x + w[0].0.w, w[1].0.x, "left to right, adjacent");
        }
        for &(r, t) in &o.items {
            assert!(inside(r, PANE), "{t:?} {r:?}");
        }
    }

    #[test]
    fn overlay_grip_is_leftmost() {
        let o = overlay_layout(PANE, &bm(), EDGE_PX, true);
        let targets: Vec<OverlayTarget> = o.items.iter().map(|&(_, t)| t).collect();
        assert_eq!(
            targets,
            vec![
                OverlayTarget::Grip,
                OverlayTarget::Profile,
                OverlayTarget::Maximize,
                OverlayTarget::Close
            ]
        );
        let side = 16 + 4;
        assert_eq!(o.bg.w, 4 * side);
        assert_eq!(o.bg.x + o.bg.w, PANE.x + PANE.w - 4);
        assert_eq!(o.items[0].0.x, o.bg.x);
    }

    #[test]
    fn overlay_drops_grip_before_buttons() {
        let side = 16 + 4;
        let pane = Rect {
            w: 3 * side + 2 * 4,
            h: side + 8,
            ..PANE
        };
        let o = overlay_layout(pane, &bm(), EDGE_PX, true);
        let targets: Vec<OverlayTarget> = o.items.iter().map(|&(_, t)| t).collect();
        assert_eq!(
            targets,
            vec![
                OverlayTarget::Profile,
                OverlayTarget::Maximize,
                OverlayTarget::Close
            ]
        );
        let small = Rect {
            w: 2 * side + 8,
            ..pane
        };
        assert!(overlay_layout(small, &bm(), EDGE_PX, true).items.is_empty());
    }

    #[test]
    fn overlay_scales_with_inset() {
        // HiDPI: the window passes round(4 × 2) = 8.
        let o = overlay_layout(PANE, &bm(), 8, false);
        assert_eq!(o.bg.x + o.bg.w, PANE.x + PANE.w - 8);
        assert_eq!(o.bg.y, PANE.y + 8);
        assert_eq!(o.items[0].0.w, 16 + 8);
    }

    #[test]
    fn overlay_hit_finds_buttons() {
        let o = overlay_layout(PANE, &bm(), EDGE_PX, false);
        let (p, m, c) = (o.items[0].0, o.items[1].0, o.items[2].0);
        assert_eq!(overlay_hit(&o, p.x, p.y), Some(OverlayTarget::Profile));
        assert_eq!(
            overlay_hit(&o, p.x + p.w - 1, p.y),
            Some(OverlayTarget::Profile)
        );
        assert_eq!(overlay_hit(&o, m.x, m.y), Some(OverlayTarget::Maximize));
        assert_eq!(
            overlay_hit(&o, c.x + c.w - 1, c.y + c.h - 1),
            Some(OverlayTarget::Close)
        );
        assert_eq!(overlay_hit(&o, c.x + c.w, c.y), None);
        assert_eq!(overlay_hit(&o, m.x, m.y + m.h), None);
        assert_eq!(overlay_hit(&o, p.x - 1, p.y), None);
        assert_eq!(overlay_hit(&o, PANE.x, PANE.y), None);
    }

    #[test]
    fn overlay_hidden_after_typing() {
        let t0 = Instant::now();
        let t1 = t0 + Duration::from_millis(10);
        let t2 = t1 + Duration::from_millis(10);
        assert!(overlay_visible(true, t0, None), "moved, no key");
        assert!(!overlay_visible(true, t0, Some(t1)), "key after the move");
        assert!(overlay_visible(true, t2, Some(t1)), "moved again");
        assert!(!overlay_visible(false, t2, Some(t1)), "pointer outside");
        assert!(!overlay_visible(false, t0, None), "pointer outside, no key");
    }

    #[test]
    fn overlay_degrades_in_tiny_pane() {
        let side = 16 + 4;
        // Exactly the overlay plus 8 px fits.
        let fits = Rect {
            w: 3 * side + 8,
            h: side + 8,
            ..PANE
        };
        assert_eq!(overlay_layout(fits, &bm(), EDGE_PX, false).items.len(), 3);
        for small in [
            // Room for two buttons is not enough any more.
            Rect {
                w: 2 * side + 8,
                ..fits
            },
            Rect {
                w: 3 * side + 7,
                ..fits
            },
            Rect {
                h: side + 7,
                ..fits
            },
            Rect { w: 0, h: 0, ..fits },
            Rect {
                w: -5,
                h: 1,
                ..fits
            },
        ] {
            let o = overlay_layout(small, &bm(), EDGE_PX, false);
            assert!(o.items.is_empty(), "{small:?}");
            assert_eq!((o.bg.w, o.bg.h), (0, 0));
            assert_eq!(overlay_hit(&o, small.x, small.y), None);
        }
    }

    #[test]
    fn overlay_never_reaches_the_edge_zone() {
        // A pane in the window's top-right corner, below the bar: at any scale
        // the overlay's corners are outside the edge-resize zone, because the
        // inset and the zone width are the same `edge_px`.
        let (w, h, bar_h) = (400, 300, 28);
        let pane = Rect {
            x: 0,
            y: bar_h,
            w,
            h: h - bar_h,
        };
        for scale in [0.5, 1.0, 1.25, 1.5, 1.75, 2.0, 3.0] {
            let e = edge_px(scale);
            let o = overlay_layout(pane, &bm(), e, false);
            assert_eq!(o.items.len(), 3, "{scale}");
            for r in std::iter::once(o.bg).chain(o.items.iter().map(|&(r, _)| r)) {
                for (x, y) in [
                    (r.x, r.y),
                    (r.x + r.w - 1, r.y),
                    (r.x, r.y + r.h - 1),
                    (r.x + r.w - 1, r.y + r.h - 1),
                ] {
                    assert_eq!(resize_edge(w, h, x, y, e), None, "{scale} {r:?}");
                }
            }
            // ...while the pixel just right of it is in the zone.
            assert!(resize_edge(w, h, o.bg.x + o.bg.w, o.bg.y, e).is_some());
        }
    }
}
