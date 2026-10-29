//! Button icons as pixel rects (no font or texture needed).

use crate::layout::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    SplitRight,
    SplitDown,
    Menu,
    Plus,
    Minimize,
    Maximize,
    Restore,
    Close,
    Profile,
    Find,
    Grip,
}

/// A filled disc of `diameter` px centred on (`cx`, `cy`) as one horizontal span per row, each
/// centred, so the shape is mirror-symmetric and stays inside its bounding square.
pub fn disc_rects(cx: i32, cy: i32, diameter: i32) -> Vec<Rect> {
    let d = diameter.max(0);
    let r = d as f32 / 2.0;
    (0..d)
        .map(|i| {
            let dy = i as f32 + 0.5 - r;
            let half = (r * r - dy * dy).max(0.0).sqrt();
            // Same parity as `d` so the span can be centred on whole pixels.
            let w = if d % 2 == 0 {
                2 * half.round() as i32
            } else {
                2 * (half - 0.5).round() as i32 + 1
            }
            .clamp(1, d);
            Rect {
                x: cx - d / 2 + (d - w) / 2,
                y: cy - d / 2 + i,
                w,
                h: 1,
            }
        })
        .collect()
}

/// Rects forming `icon`, centered in `bounds` inside a square of side
/// `min(w, h) / 2` rounded to even, drawn with lines `stroke` px thick.
pub fn icon_rects(icon: Icon, bounds: Rect, stroke: i32) -> Vec<Rect> {
    let side = (((bounds.w.min(bounds.h).max(0) as f32 * 0.5) / 2.0).round() as i32) * 2;
    if side == 0 {
        return Vec::new();
    }
    let s = stroke.clamp(1, (side / 2).max(1));
    let g = Rect {
        x: bounds.x + (bounds.w - side) / 2,
        y: bounds.y + (bounds.h - side) / 2,
        w: side,
        h: side,
    };
    let rect = |dx: i32, dy: i32, w: i32, h: i32| Rect {
        x: g.x + dx,
        y: g.y + dy,
        w,
        h,
    };
    let mid = (side - s) / 2;
    let mut out = Vec::new();
    match icon {
        Icon::SplitRight => {
            outline(&mut out, g, s);
            out.push(rect(mid, 0, s, side));
        }
        Icon::SplitDown => {
            outline(&mut out, g, s);
            out.push(rect(0, mid, side, s));
        }
        Icon::Menu => {
            let step = mid;
            let top = (side - (2 * step + s)) / 2;
            for i in 0..3 {
                out.push(rect(0, top + i * step, side, s));
            }
        }
        Icon::Plus => {
            out.push(rect(0, mid, side, s));
            out.push(rect(mid, 0, s, side));
        }
        Icon::Minimize => out.push(rect(0, side - s, side, s)),
        Icon::Maximize => outline(&mut out, g, s),
        Icon::Restore => {
            // Front square lower-left; the back one shows only its top and right edges.
            let d = (side / 4).max(s + 1).min(side - 1);
            let f = side - d;
            outline(&mut out, rect(0, d, f, f), s);
            out.push(rect(d, 0, side - d, s));
            out.push(rect(side - s, s, s, f - s));
        }
        Icon::Profile => {
            // A palette as a small bar chart: three bars, bottoms aligned, each
            // shorter than the one to its left; at least `s` wide, spread over the side.
            let bw = s.max(side / 5).min(side / 3);
            let gap = (side - 3 * bw) / 2;
            let x0 = (side - 3 * bw - 2 * gap) / 2;
            for i in 0..3 {
                let h = side * (3 - i) / 3;
                out.push(rect(x0 + i * (bw + gap), side - h, bw, h));
            }
        }
        Icon::Find => {
            // A magnifier: a hollow square ring of ⅔ of the side at the top-left and a
            // two-step diagonal handle running to the bottom-right corner.
            let ring = (side * 2 / 3).max(1);
            outline(&mut out, rect(0, 0, ring, ring), s);
            let d = side - ring;
            let a = (d / 2).max(1).min(d);
            out.push(rect(ring, ring, a, a));
            out.push(rect(ring + a, ring + a, d - a, d - a));
        }
        Icon::Grip => {
            // Six dots in two columns of three, centered. Dots are `s` square,
            // spread over the side when it has room for gaps.
            let d = s.min(side / 4).max(1);
            let gx = (side - 2 * d) / 3 + d;
            let gy = (side - 3 * d) / 2 + d;
            let (x0, y0) = ((side - gx - d) / 2, (side - 2 * gy - d) / 2);
            for col in 0..2 {
                for row in 0..3 {
                    out.push(rect(x0 + col * gx, y0 + row * gy, d, d));
                }
            }
            // A glyph too small for six dots keeps those that fit.
            out.retain(|r| {
                r.x >= g.x && r.y >= g.y && r.x + r.w <= g.x + side && r.y + r.h <= g.y + side
            });
        }
        Icon::Close => {
            for r in 0..=(side - s) {
                out.push(rect(r, r, s, s));
                out.push(rect(side - s - r, r, s, s));
            }
        }
    }
    out.retain(|r| r.w > 0 && r.h > 0);
    out
}

/// Square outline of thickness `s` (a solid fill when too small to have a hollow).
fn outline(out: &mut Vec<Rect>, r: Rect, s: i32) {
    if r.w <= 2 * s || r.h <= 2 * s {
        out.push(r);
        return;
    }
    out.push(Rect { h: s, ..r });
    out.push(Rect {
        y: r.y + r.h - s,
        h: s,
        ..r
    });
    out.push(Rect {
        y: r.y + s,
        w: s,
        h: r.h - 2 * s,
        ..r
    });
    out.push(Rect {
        x: r.x + r.w - s,
        y: r.y + s,
        w: s,
        h: r.h - 2 * s,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Icon; 11] = [
        Icon::SplitRight,
        Icon::SplitDown,
        Icon::Menu,
        Icon::Plus,
        Icon::Minimize,
        Icon::Maximize,
        Icon::Restore,
        Icon::Close,
        Icon::Profile,
        Icon::Find,
        Icon::Grip,
    ];
    const B: Rect = Rect {
        x: 10,
        y: 20,
        w: 24,
        h: 24,
    };

    fn inside(r: Rect, b: Rect) -> bool {
        r.x >= b.x && r.y >= b.y && r.x + r.w <= b.x + b.w && r.y + r.h <= b.y + b.h
    }

    #[test]
    fn icons_stay_inside_bounds() {
        for icon in ALL {
            for stroke in [1, 2] {
                let rects = icon_rects(icon, B, stroke);
                assert!(!rects.is_empty(), "{icon:?}");
                for r in rects {
                    assert!(r.w > 0 && r.h > 0, "{icon:?} s{stroke} empty {r:?}");
                    assert!(inside(r, B), "{icon:?} s{stroke} outside {r:?}");
                }
            }
        }
    }

    #[test]
    fn tiny_bounds_do_not_panic_or_overflow() {
        for icon in ALL {
            for side in [0, 1, 2, 5] {
                let b = Rect {
                    x: 3,
                    y: 3,
                    w: side,
                    h: side,
                };
                for r in icon_rects(icon, b, 2) {
                    assert!(r.w > 0 && r.h > 0 && inside(r, b), "{icon:?} {side} {r:?}");
                }
            }
        }
    }

    #[test]
    fn split_right_has_outline_and_vertical_bar() {
        // side 12, glyph at (16, 26)..(28, 38); center x = 16 + (12 - 1) / 2.
        let rects = icon_rects(Icon::SplitRight, B, 1);
        assert!(
            rects
                .iter()
                .any(|r| r.w == 1 && r.h == 12 && r.x == 16 + 5 && r.y == 26)
        );
    }

    #[test]
    fn split_down_has_horizontal_bar() {
        let rects = icon_rects(Icon::SplitDown, B, 1);
        assert!(
            rects
                .iter()
                .any(|r| r.h == 1 && r.w == 12 && r.y == 26 + 5 && r.x == 16)
        );
    }

    #[test]
    fn close_is_symmetric_x() {
        for stroke in [1, 2] {
            let rects = icon_rects(Icon::Close, B, stroke);
            let mirror = |r: &Rect| Rect {
                x: 2 * 16 + 12 - r.x - r.w,
                ..*r
            };
            let mut a: Vec<_> = rects.iter().map(|r| (r.x, r.y, r.w, r.h)).collect();
            let mut b: Vec<_> = rects
                .iter()
                .map(mirror)
                .map(|r| (r.x, r.y, r.w, r.h))
                .collect();
            a.sort();
            b.sort();
            assert_eq!(a, b);
        }
    }

    #[test]
    fn menu_has_three_bars() {
        let rects = icon_rects(Icon::Menu, B, 1);
        assert_eq!(rects.len(), 3);
        assert!(rects.iter().all(|r| r.w == 12 && r.h == 1));
        let gap = |i: usize| rects[i + 1].y - rects[i].y;
        assert_eq!(gap(0), gap(1));
    }

    #[test]
    fn plus_crosses_at_center() {
        let rects = icon_rects(Icon::Plus, B, 1);
        assert_eq!(rects.len(), 2);
    }

    #[test]
    fn minimize_sits_at_bottom() {
        let rects = icon_rects(Icon::Minimize, B, 2);
        assert_eq!(
            rects,
            vec![Rect {
                x: 16,
                y: 26 + 12 - 2,
                w: 12,
                h: 2
            }]
        );
    }

    #[test]
    fn find_is_a_ring_with_a_two_step_handle() {
        // side 12 at (16, 26): the ring is 8 wide, the handle two 2x2 steps from its
        // bottom-right corner to the glyph's.
        let rects = icon_rects(Icon::Find, B, 1);
        let at = |x: i32, y: i32, w: i32| rects.contains(&Rect { x, y, w, h: w });
        assert!(at(16 + 8, 26 + 8, 2), "{rects:?}");
        assert!(at(16 + 10, 26 + 10, 2), "{rects:?}");
        // Four ring edges and the two steps.
        assert_eq!(rects.len(), 6);
        // The ring is hollow: no rect covers its middle.
        assert!(!rects.iter().any(|r| r.contains(16 + 4, 26 + 4)));
        let right = rects.iter().map(|r| r.x + r.w).max().unwrap();
        let bottom = rects.iter().map(|r| r.y + r.h).max().unwrap();
        assert_eq!((right, bottom), (16 + 12, 26 + 12));
    }

    #[test]
    fn grip_is_six_dots() {
        let rects = icon_rects(Icon::Grip, B, 1);
        assert_eq!(rects.len(), 6);
        assert!(rects.iter().all(|r| r.w == rects[0].w && r.h == rects[0].w));
        for (i, a) in rects.iter().enumerate() {
            assert!(inside(*a, B), "{a:?}");
            for b in &rects[i + 1..] {
                let apart =
                    a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y;
                assert!(apart, "{a:?} {b:?}");
            }
        }
    }

    #[test]
    fn icon_profile_within_bounds() {
        // The overlay's button side at scales 0.5..3 (cell_h + edge_px), and odd sizes.
        for side in [6, 10, 13, 20, 24, 29, 40, 61] {
            let b = Rect {
                x: 7,
                y: 9,
                w: side,
                h: side,
            };
            for stroke in [1, 2, 3, 4] {
                let rects = icon_rects(Icon::Profile, b, stroke);
                assert_eq!(rects.len(), 3, "{side} s{stroke}");
                for r in &rects {
                    assert!(
                        r.w > 0 && r.h > 0 && inside(*r, b),
                        "{side} s{stroke} {r:?}"
                    );
                }
                // Left to right, not overlapping, bottoms aligned, heights decreasing.
                for w in rects.windows(2) {
                    assert!(w[0].x + w[0].w <= w[1].x, "{side} s{stroke} {rects:?}");
                    assert_eq!(w[0].y + w[0].h, w[1].y + w[1].h);
                    assert!(w[0].h > w[1].h, "{side} s{stroke} {rects:?}");
                }
            }
        }
    }

    #[test]
    fn disc_is_symmetric_and_bounded() {
        for d in [4, 8, 16] {
            let (cx, cy) = (50, 30);
            let rects = disc_rects(cx, cy, d);
            let square = Rect {
                x: cx - d / 2,
                y: cy - d / 2,
                w: d,
                h: d,
            };
            assert_eq!(rects.len() as i32, d, "one span per row");
            for r in &rects {
                assert!(r.w > 0 && r.h > 0 && inside(*r, square), "d{d} {r:?}");
                // Mirror-symmetric left/right around the centre.
                assert_eq!(r.x - square.x, square.x + d - (r.x + r.w), "d{d} {r:?}");
            }
            let widths: Vec<i32> = rects.iter().map(|r| r.w).collect();
            let mid = widths.iter().copied().max().unwrap();
            assert!(
                widths[0] < mid && widths[widths.len() - 1] < mid,
                "{widths:?}"
            );
            let mut flipped = widths.clone();
            flipped.reverse();
            assert_eq!(widths, flipped, "top/bottom mirror");
        }
    }
}
