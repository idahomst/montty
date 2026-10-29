//! Button icons as antialiased strokes (no font or texture needed), and the attention dot.

use crate::layout::Rect;
use crate::render::{Shape, ShapeInstance};

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

/// Antialiased strokes forming `icon` in `color`, `line` px wide, centred in `bounds` inside
/// a square of side `min(w, h) / 2` rounded to even. Shapes follow a 24-unit grid whose
/// span 3..21 maps onto the square (Lucide's geometry); straight edges are snapped so the
/// stroke covers whole pixels and stays sharp.
pub fn icon_shapes(icon: Icon, bounds: Rect, line: f32, color: [f32; 4]) -> Vec<ShapeInstance> {
    let side = (((bounds.w.min(bounds.h).max(0) as f32 * 0.5) / 2.0).round() as i32) * 2;
    if side == 0 {
        return Vec::new();
    }
    // Very small icons (the pane overlay's at scale 1) get thinner lines, so they stay open.
    let w = line.min(side as f32 / 6.0).max(1.0);
    let k = side as f32 / 18.0;
    let (gx, gy) = (
        (bounds.x + (bounds.w - side) / 2) as f32,
        (bounds.y + (bounds.h - side) / 2) as f32,
    );
    // Grid units to px. The + for a new tab is drawn at three quarters of the others' size.
    let scale = if icon == Icon::Plus { 0.75 } else { 1.0 };
    let (cx, cy) = (gx + side as f32 / 2.0, gy + side as f32 / 2.0);
    let px = |u: f32, v: f32| [cx + (u - 12.0) * k * scale, cy + (v - 12.0) * k * scale];
    let snap = |v: f32| (v - w / 2.0).round() + w / 2.0;
    let sp = |u: f32, v: f32| {
        let [x, y] = px(u, v);
        [snap(x), snap(y)]
    };
    let st = |shape: Shape, width: f32| ShapeInstance {
        shape,
        width,
        color,
    };
    let ln = |a: [f32; 2], b: [f32; 2]| st(Shape::Line { a, b }, w);
    let mut out = Vec::new();
    let boxed = |u0: f32, v0: f32, u1: f32, v1: f32| {
        let ([x0, y0], [x1, y1]) = (sp(u0, v0), sp(u1, v1));
        Shape::RoundRect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
            r: 2.0 * k,
        }
    };
    match icon {
        Icon::SplitRight => {
            out.push(ln(sp(12.0, 3.0), sp(12.0, 21.0)));
            out.push(st(boxed(3.0, 3.0, 21.0, 21.0), w));
        }
        Icon::SplitDown => {
            out.push(ln(sp(3.0, 12.0), sp(21.0, 12.0)));
            out.push(st(boxed(3.0, 3.0, 21.0, 21.0), w));
        }
        Icon::Menu => {
            for v in [6.0, 12.0, 18.0] {
                out.push(ln(sp(4.0, v), sp(20.0, v)));
            }
        }
        Icon::Plus => {
            out.push(ln(sp(5.0, 12.0), sp(19.0, 12.0)));
            out.push(ln(sp(12.0, 5.0), sp(12.0, 19.0)));
        }
        Icon::Minimize => out.push(ln(sp(5.0, 18.0), sp(19.0, 18.0))),
        Icon::Maximize => out.push(st(boxed(4.0, 4.0, 20.0, 20.0), w)),
        Icon::Restore => {
            // A front square at the lower left; of the one behind it only the top and right
            // edges show.
            out.push(st(boxed(3.0, 9.0, 15.0, 21.0), w));
            out.push(ln(sp(9.0, 3.0), sp(21.0, 3.0)));
            out.push(ln(sp(21.0, 3.0), sp(21.0, 15.0)));
        }
        Icon::Profile => {
            // A palette as a small bar chart: three bars, bottoms aligned, each shorter.
            for (u, top) in [(6.0, 4.0), (12.0, 10.0), (18.0, 15.0)] {
                out.push(ln(sp(u, top), sp(u, 20.0)));
            }
        }
        Icon::Find => {
            // A round lens and a handle at 45 degrees.
            let [x, y] = px(11.0, 11.0);
            out.push(st(
                Shape::Ring {
                    c: [x, y],
                    r: 7.5 * k,
                },
                w,
            ));
            out.push(ln(px(16.3, 16.3), px(20.5, 20.5)));
        }
        Icon::Grip => {
            for u in [9.0, 15.0] {
                for v in [6.0, 12.0, 18.0] {
                    let p = px(u, v);
                    out.push(st(Shape::Line { a: p, b: p }, (w * 1.5).min(3.6 * k)));
                }
            }
        }
        Icon::Close => {
            out.push(ln(px(5.0, 5.0), px(19.0, 19.0)));
            out.push(ln(px(19.0, 5.0), px(5.0, 19.0)));
        }
    }
    out
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

    const C: [f32; 4] = [0.8, 0.8, 0.8, 1.0];

    /// The centre-line extent of a shape: (x0, y0, x1, y1).
    fn extent(s: &Shape) -> (f32, f32, f32, f32) {
        match *s {
            Shape::Line { a, b } => (
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[0].max(b[0]),
                a[1].max(b[1]),
            ),
            Shape::RoundRect { x, y, w, h, .. } => (x, y, x + w, y + h),
            Shape::Ring { c, r } => (c[0] - r, c[1] - r, c[0] + r, c[1] + r),
        }
    }

    fn union(shapes: &[ShapeInstance]) -> (f32, f32, f32, f32) {
        shapes.iter().map(|s| extent(&s.shape)).fold(
            (f32::MAX, f32::MAX, f32::MIN, f32::MIN),
            |(a, b, c, d), (x0, y0, x1, y1)| (a.min(x0), b.min(y0), c.max(x1), d.max(y1)),
        )
    }

    #[test]
    fn icons_stay_inside_bounds() {
        for icon in ALL {
            for side in [16, 24, 32, 41] {
                let b = Rect {
                    w: side,
                    h: side,
                    ..B
                };
                for line in [1.0, 2.0, 3.0, 4.0] {
                    let shapes = icon_shapes(icon, b, line, C);
                    assert!(!shapes.is_empty(), "{icon:?}");
                    for s in shapes {
                        assert!(inside(s.bounds(), b), "{icon:?} {side} {line} {s:?}");
                        assert_eq!(s.color, C);
                    }
                }
            }
        }
    }

    #[test]
    fn tiny_bounds_do_not_panic() {
        for icon in ALL {
            for side in [0, 1, 2, 5] {
                let b = Rect {
                    x: 3,
                    y: 3,
                    w: side,
                    h: side,
                };
                for s in icon_shapes(icon, b, 2.0, C) {
                    let (x0, y0, x1, y1) = extent(&s.shape);
                    assert!(
                        [x0, y0, x1, y1].iter().all(|v| v.is_finite()),
                        "{icon:?} {side}"
                    );
                }
            }
        }
        assert!(icon_shapes(Icon::Close, Rect { w: 0, ..B }, 2.0, C).is_empty());
    }

    #[test]
    fn straight_strokes_cover_whole_pixels() {
        // A stroke `line` px wide centred on its line ends on pixel edges, so it stays sharp.
        for line in [1.0f32, 2.0, 3.0] {
            for icon in [
                Icon::SplitRight,
                Icon::SplitDown,
                Icon::Menu,
                Icon::Plus,
                Icon::Minimize,
                Icon::Maximize,
                Icon::Restore,
                Icon::Profile,
            ] {
                for s in icon_shapes(icon, B, line, C) {
                    let (x0, y0, x1, y1) = extent(&s.shape);
                    for v in [x0, y0, x1, y1] {
                        let edge = v - s.width / 2.0;
                        assert!((edge - edge.round()).abs() < 1e-4, "{icon:?} {line} {s:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn small_icons_get_thinner_lines() {
        // Side 12 caps the line at 2 px; side 16 keeps the requested 2.
        assert!(
            icon_shapes(Icon::Close, B, 3.0, C)
                .iter()
                .all(|s| s.width == 2.0)
        );
        let b16 = Rect { w: 32, h: 32, ..B };
        assert!(
            icon_shapes(Icon::Close, b16, 2.0, C)
                .iter()
                .all(|s| s.width == 2.0)
        );
    }

    #[test]
    fn square_icons_are_centred() {
        let (cx, cy) = (B.x as f32 + 12.0, B.y as f32 + 12.0);
        for icon in [
            Icon::SplitRight,
            Icon::SplitDown,
            Icon::Menu,
            Icon::Plus,
            Icon::Maximize,
            Icon::Close,
            Icon::Grip,
        ] {
            for line in [1.0, 2.0] {
                let (x0, y0, x1, y1) = union(&icon_shapes(icon, B, line, C));
                assert!(((x0 + x1) / 2.0 - cx).abs() <= 0.5, "{icon:?} {line}");
                assert!(((y0 + y1) / 2.0 - cy).abs() <= 0.5, "{icon:?} {line}");
            }
        }
    }

    #[test]
    fn split_icons_divide_their_frame() {
        let right = icon_shapes(Icon::SplitRight, B, 2.0, C);
        let down = icon_shapes(Icon::SplitDown, B, 2.0, C);
        assert!(matches!(right[0].shape, Shape::Line { a, b } if a[0] == b[0]));
        assert!(matches!(down[0].shape, Shape::Line { a, b } if a[1] == b[1]));
        for s in [&right, &down] {
            assert!(matches!(s[1].shape, Shape::RoundRect { r, .. } if r > 0.0));
        }
    }

    #[test]
    fn plus_is_smaller_than_the_other_icons() {
        let width = |icon| {
            let (x0, _, x1, _) = union(&icon_shapes(icon, B, 2.0, C));
            x1 - x0
        };
        assert!(
            width(Icon::Plus) < 0.8 * width(Icon::Menu),
            "{} {}",
            width(Icon::Plus),
            width(Icon::Menu)
        );
    }

    #[test]
    fn menu_has_three_evenly_spaced_bars() {
        let ys: Vec<f32> = icon_shapes(Icon::Menu, B, 2.0, C)
            .iter()
            .map(|s| extent(&s.shape).1)
            .collect();
        assert_eq!(ys.len(), 3);
        assert_eq!(ys[1] - ys[0], ys[2] - ys[1]);
    }

    #[test]
    fn minimize_sits_low() {
        let (_, y0, _, _) = union(&icon_shapes(Icon::Minimize, B, 2.0, C));
        assert!(y0 > B.y as f32 + 12.0 + 3.0, "{y0}");
    }

    #[test]
    fn find_is_a_ring_with_a_diagonal_handle() {
        let s = icon_shapes(Icon::Find, B, 2.0, C);
        assert_eq!(s.len(), 2);
        let Shape::Ring { c, r } = s[0].shape else {
            panic!("{s:?}")
        };
        let Shape::Line { a, b } = s[1].shape else {
            panic!("{s:?}")
        };
        // The handle leaves the lens at its lower right and runs at 45 degrees.
        assert!(((b[0] - a[0]) - (b[1] - a[1])).abs() < 1e-3 && b[0] > a[0]);
        let gap = ((a[0] - c[0]).powi(2) + (a[1] - c[1]).powi(2)).sqrt() - r;
        assert!(gap.abs() < 0.1, "{gap}");
    }

    #[test]
    fn restore_is_a_front_frame_and_two_back_edges() {
        let s = icon_shapes(Icon::Restore, B, 2.0, C);
        assert_eq!(s.len(), 3);
        assert!(matches!(s[0].shape, Shape::RoundRect { .. }));
    }

    #[test]
    fn grip_is_six_separate_dots() {
        let s = icon_shapes(Icon::Grip, B, 2.0, C);
        assert_eq!(s.len(), 6);
        let dots: Vec<[f32; 2]> = s
            .iter()
            .map(|d| match d.shape {
                Shape::Line { a, b } if a == b => a,
                _ => panic!("{d:?}"),
            })
            .collect();
        for (i, p) in dots.iter().enumerate() {
            for q in &dots[i + 1..] {
                let dist = ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt();
                assert!(dist > s[0].width + 1.5, "{p:?} {q:?}");
            }
        }
    }

    #[test]
    fn profile_bars_are_bottom_aligned_and_shrink() {
        let s = icon_shapes(Icon::Profile, B, 2.0, C);
        assert_eq!(s.len(), 3);
        let e: Vec<_> = s.iter().map(|b| extent(&b.shape)).collect();
        for w in e.windows(2) {
            assert!(w[0].0 < w[1].0);
            assert_eq!(w[0].3, w[1].3);
            assert!(w[0].3 - w[0].1 > w[1].3 - w[1].1, "{e:?}");
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
