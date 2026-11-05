//! Pure layout and hit-testing for the self-drawn top bar (tabs, buttons, drag area).

use super::text::{display_width, sanitize, truncate};
use crate::font::Metrics;
use crate::layout::Rect;
use winit::window::ResizeDirection;

/// Fewest / most label columns a tab is sized for.
const MIN_COLS: usize = 6;
const MAX_COLS: usize = 24;

/// Pixel metrics of the bar, derived from the terminal font and display scale.
#[derive(Clone, Copy, Debug)]
pub struct BarMetrics {
    pub height: i32,
    pub pad: i32,
    pub cell_w: i32,
    pub cell_h: i32,
}

impl BarMetrics {
    /// Fixed metrics shared by the unit tests of every module that draws chrome.
    #[cfg(test)]
    pub fn test() -> Self {
        Self {
            height: 28,
            pad: 6,
            cell_w: 8,
            cell_h: 16,
        }
    }

    /// Diameter of an attention dot: 8 logical px (`pad` is 6 logical px), at least 4.
    pub fn dot(&self) -> i32 {
        ((self.pad as f32 * 4.0 / 3.0).round() as i32).max(4)
    }

    pub fn new(m: &Metrics, scale: f64) -> Self {
        let pad = (6.0 * scale).round() as i32;
        let cell_h = m.cell_height as i32;
        Self {
            height: cell_h + 2 * pad,
            pad,
            cell_w: m.cell_width as i32,
            cell_h,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Tab(usize),
    TabClose(usize),
    NewTab,
    SplitRight,
    SplitDown,
    Menu,
    Find,
    Minimize,
    Maximize,
    Close,
    Drag,
}

#[derive(Clone, Copy, Debug)]
pub struct Item {
    pub rect: Rect,
    pub target: Target,
}

/// Where and what to draw for one visible tab's label.
#[derive(Clone, Debug)]
pub struct TabText {
    pub index: usize,
    pub text_rect: Rect,
    pub label: String,
    /// The attention dot, before the label (which is shifted right to make room).
    pub dot_rect: Option<Rect>,
}

#[derive(Clone, Debug)]
pub struct Bar {
    pub height: i32,
    pub items: Vec<Item>,
    pub tabs: Vec<TabText>,
}

/// (index, width) of each tab that fits in `available` px, in order. `naturals` are the
/// preferred widths. Tabs share the room when they can, shrink to the minimum, then drop off
/// the end; the active tab always stays visible.
fn visible_tabs(
    bm: &BarMetrics,
    naturals: &[i32],
    available: i32,
    active: usize,
) -> Vec<(usize, i32)> {
    let n = naturals.len();
    let min_w = MIN_COLS as i32 * bm.cell_w + 3 * bm.pad + bm.cell_h;
    let mut visible: Vec<(usize, i32)> = Vec::new();
    // The active tab is always shown: below one full minimum tab it takes all the
    // room (label truncated), as long as one column of label still fits.
    let squeeze_w = 3 * bm.pad + bm.cell_h + bm.cell_w;
    if n > 0 && active < n && available < min_w && available >= squeeze_w {
        visible.push((active, available));
    } else if n > 0 && available >= min_w {
        if naturals.iter().sum::<i32>() <= available {
            visible.extend(naturals.iter().copied().enumerate());
        } else if n as i32 * min_w <= available {
            let w = (available / n as i32).max(min_w);
            visible.extend((0..n).map(|i| (i, w)));
        } else {
            let k = (available / min_w) as usize;
            visible.extend((0..k).map(|i| (i, min_w)));
            if active < n && active >= k {
                visible[k - 1].0 = active;
            }
        }
    }
    visible
}

pub fn layout(width: i32, bm: &BarMetrics, labels: &[String], active: usize, dots: &[bool]) -> Bar {
    let h = bm.height;
    let mut items = Vec::new();
    let mut tabs = Vec::new();

    // Right cluster, laid right-to-left; stop at the first button that doesn't fit.
    // `None` marks the gap between the window buttons and the rest.
    let order = [
        Some(Target::Close),
        Some(Target::Maximize),
        Some(Target::Minimize),
        None,
        Some(Target::Menu),
        Some(Target::Find),
        Some(Target::SplitDown),
        Some(Target::SplitRight),
    ];
    let mut x = width;
    let mut cluster_left = width;
    for slot in order {
        match slot {
            None => x -= bm.pad,
            Some(target) => {
                if x - h < 0 {
                    break;
                }
                x -= h;
                cluster_left = x;
                items.push(Item {
                    rect: Rect { x, y: 0, w: h, h },
                    target,
                });
            }
        }
    }

    let available = cluster_left - h; // room for tabs once the NewTab button is reserved
    let naturals: Vec<i32> = labels
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let cols = display_width(&sanitize(l)).clamp(MIN_COLS, MAX_COLS) as i32;
            let dot = if dots.get(i).copied().unwrap_or(false) {
                bm.dot() + bm.pad / 2
            } else {
                0
            };
            cols * bm.cell_w + 3 * bm.pad + bm.cell_h + dot
        })
        .collect();
    let visible = visible_tabs(bm, &naturals, available, active);

    let mut tx = 0;
    for &(index, w) in &visible {
        let rect = Rect { x: tx, y: 0, w, h };
        let close = Rect {
            x: tx + w - bm.pad - bm.cell_h,
            y: (h - bm.cell_h) / 2,
            w: bm.cell_h,
            h: bm.cell_h,
        };
        let mut text_x = tx + bm.pad;
        let dot_rect = dots.get(index).copied().unwrap_or(false).then(|| {
            let d = bm.dot();
            let r = Rect {
                x: text_x,
                y: (h - d) / 2,
                w: d,
                h: d,
            };
            text_x += d + bm.pad / 2;
            r
        });
        let text_w = (close.x - bm.pad - text_x).max(0);
        let text_rect = Rect {
            x: text_x,
            y: (h - bm.cell_h) / 2,
            w: text_w,
            h: bm.cell_h,
        };
        let cols = (text_w / bm.cell_w.max(1)) as usize;
        tabs.push(TabText {
            index,
            text_rect,
            label: truncate(&sanitize(&labels[index]), cols),
            dot_rect,
        });
        items.push(Item {
            rect,
            target: Target::Tab(index),
        });
        items.push(Item {
            rect: close,
            target: Target::TabClose(index),
        });
        tx += w;
    }
    if tx + h <= cluster_left {
        items.push(Item {
            rect: Rect {
                x: tx,
                y: 0,
                w: h,
                h,
            },
            target: Target::NewTab,
        });
    }

    Bar {
        height: h,
        items,
        tabs,
    }
}

/// What a click at (x, y) hits: `None` below the bar, `Drag` on empty bar space.
pub fn hit(bar: &Bar, x: i32, y: i32) -> Option<Target> {
    if y < 0 || y >= bar.height {
        return None;
    }
    let found = |want_close: bool| {
        bar.items
            .iter()
            .find(|i| {
                i.rect.contains(x, y) && matches!(i.target, Target::TabClose(_)) == want_close
            })
            .map(|i| i.target)
    };
    // The close box lies inside its tab, so it must be tested first.
    found(true).or_else(|| found(false)).or(Some(Target::Drag))
}

/// Resize direction for a point within `border` px of a window edge; corners win.
pub fn resize_edge(w: i32, h: i32, x: i32, y: i32, border: i32) -> Option<ResizeDirection> {
    let left = x < border;
    let right = x >= w - border;
    let top = y < border;
    let bottom = y >= h - border;
    match (left, right, top, bottom) {
        (true, _, true, _) => Some(ResizeDirection::NorthWest),
        (_, true, true, _) => Some(ResizeDirection::NorthEast),
        (true, _, _, true) => Some(ResizeDirection::SouthWest),
        (_, true, _, true) => Some(ResizeDirection::SouthEast),
        (_, _, true, _) => Some(ResizeDirection::North),
        (_, _, _, true) => Some(ResizeDirection::South),
        (true, ..) => Some(ResizeDirection::West),
        (_, true, ..) => Some(ResizeDirection::East),
        _ => None,
    }
}

/// Tab label: the rename if set, else the pane title, else "montty"; sanitized.
pub fn tab_label(rename: Option<&str>, pane_title: Option<&str>) -> String {
    [rename, pane_title]
        .into_iter()
        .flatten()
        .map(sanitize)
        .find(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "montty".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // With BarMetrics::test(): minimum tab width 82, squeeze threshold 42.
    #[test]
    fn visible_tabs_natural_widths_when_all_fit() {
        let v = visible_tabs(&BarMetrics::test(), &[100, 120, 90], 400, 1);
        assert_eq!(v, vec![(0, 100), (1, 120), (2, 90)]);
    }

    #[test]
    fn visible_tabs_share_room_down_to_the_minimum() {
        let v = visible_tabs(&BarMetrics::test(), &[200, 200, 200], 300, 0);
        assert_eq!(v, vec![(0, 100), (1, 100), (2, 100)]);
    }

    #[test]
    fn visible_tabs_overflow_keeps_the_active_tab() {
        // Room for two minimum tabs of five: a far active tab replaces the last visible one.
        let nat = [100; 5];
        assert_eq!(
            visible_tabs(&BarMetrics::test(), &nat, 170, 0),
            vec![(0, 82), (1, 82)]
        );
        assert_eq!(
            visible_tabs(&BarMetrics::test(), &nat, 170, 4),
            vec![(0, 82), (4, 82)]
        );
    }

    #[test]
    fn visible_tabs_squeezes_only_the_active_tab() {
        assert_eq!(
            visible_tabs(&BarMetrics::test(), &[100, 100], 60, 1),
            vec![(1, 60)]
        );
        assert!(visible_tabs(&BarMetrics::test(), &[100, 100], 41, 1).is_empty());
        assert!(visible_tabs(&BarMetrics::test(), &[], 400, 0).is_empty());
    }

    fn rect_of(bar: &Bar, t: Target) -> Rect {
        bar.items
            .iter()
            .find(|i| i.target == t)
            .unwrap_or_else(|| panic!("{t:?} missing"))
            .rect
    }

    fn overlap(a: Rect, b: Rect) -> bool {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }

    fn assert_sane(bar: &Bar, width: i32) {
        for (i, a) in bar.items.iter().enumerate() {
            assert!(
                a.rect.x >= 0 && a.rect.x + a.rect.w <= width,
                "{a:?} outside {width}"
            );
            for b in &bar.items[i + 1..] {
                let nested = matches!(
                    (a.target, b.target),
                    (Target::Tab(x), Target::TabClose(y)) | (Target::TabClose(x), Target::Tab(y)) if x == y
                );
                assert!(nested || !overlap(a.rect, b.rect), "{a:?} overlaps {b:?}");
            }
        }
    }

    fn tab_rects(bar: &Bar) -> Vec<(usize, Rect)> {
        bar.items
            .iter()
            .filter_map(|i| match i.target {
                Target::Tab(n) => Some((n, i.rect)),
                _ => None,
            })
            .collect()
    }

    fn labels(n: usize, len: usize) -> Vec<String> {
        (0..n).map(|_| "x".repeat(len)).collect()
    }

    #[test]
    fn buttons_right_aligned_in_order() {
        let bar = layout(800, &BarMetrics::test(), &[], 0, &[]);
        let r = |t| rect_of(&bar, t);
        assert_eq!(
            r(Target::Close),
            Rect {
                x: 772,
                y: 0,
                w: 28,
                h: 28
            }
        );
        assert_eq!(r(Target::Maximize).x, 744);
        assert_eq!(r(Target::Minimize).x, 716);
        assert_eq!(r(Target::Menu).x, 716 - 6 - 28);
        assert_eq!(r(Target::Find).x, 716 - 6 - 56);
        assert_eq!(r(Target::SplitDown).x, 716 - 6 - 84);
        assert_eq!(r(Target::SplitRight).x, 716 - 6 - 112);
        for t in [
            Target::Maximize,
            Target::Minimize,
            Target::Menu,
            Target::Find,
            Target::SplitDown,
            Target::SplitRight,
        ] {
            assert_eq!((r(t).w, r(t).h, r(t).y), (28, 28, 0));
        }
    }

    #[test]
    fn tab_width_from_label() {
        let bar = layout(800, &BarMetrics::test(), &["bash".to_string()], 0, &[]);
        let t = rect_of(&bar, Target::Tab(0));
        assert_eq!((t.x, t.w, t.h), (0, 82, 28));
        let c = rect_of(&bar, Target::TabClose(0));
        assert_eq!((c.w, c.h), (16, 16));
        assert_eq!(c.x, t.x + t.w - 6 - 16);
        assert_eq!(c.y, 6);
        assert_eq!(bar.tabs[0].label, "bash");
        let tr = bar.tabs[0].text_rect;
        assert_eq!((tr.x, tr.h, tr.y), (6, 16, 6));
        assert!(tr.x + tr.w <= c.x - 6);
    }

    #[test]
    fn new_tab_button_follows_last_tab() {
        let bar = layout(
            800,
            &BarMetrics::test(),
            &["a".to_string(), "b".to_string()],
            0,
            &[],
        );
        let last = rect_of(&bar, Target::Tab(1));
        let nt = rect_of(&bar, Target::NewTab);
        assert_eq!(nt.x, last.x + last.w);
        assert_eq!((nt.w, nt.h), (28, 28));
        let bar = layout(800, &BarMetrics::test(), &[], 0, &[]);
        assert_eq!(rect_of(&bar, Target::NewTab).x, 0);
    }

    #[test]
    fn tabs_shrink_to_fit() {
        let bar = layout(900, &BarMetrics::test(), &labels(10, 24), 0, &[]);
        let tabs = tab_rects(&bar);
        assert!(!tabs.is_empty());
        let w0 = tabs[0].1.w;
        assert!(w0 >= 82);
        assert!(tabs.iter().all(|(_, r)| r.w == w0));
        assert_sane(&bar, 900);
        // Labels never overflow their text area.
        for t in &bar.tabs {
            assert!(display_width(&t.label) as i32 * 8 <= t.text_rect.w);
        }
    }

    #[test]
    fn many_tabs_keep_active_visible() {
        let bar = layout(600, &BarMetrics::test(), &labels(20, 10), 17, &[]);
        let tabs = tab_rects(&bar);
        assert!(tabs.iter().any(|(i, _)| *i == 17));
        assert!(tabs.len() < 20);
        assert_sane(&bar, 600);
        assert!(bar.tabs.iter().any(|t| t.index == 17));
        // Active tab is last among the visible ones and NewTab follows.
        let a = rect_of(&bar, Target::Tab(17));
        assert_eq!(rect_of(&bar, Target::NewTab).x, a.x + a.w);
    }

    #[test]
    fn narrow_window_keeps_buttons_no_overlap() {
        let bar = layout(100, &BarMetrics::test(), &labels(3, 5), 0, &[]);
        assert!(tab_rects(&bar).is_empty());
        assert_eq!(rect_of(&bar, Target::Close).x, 72);
        assert_sane(&bar, 100);
        for w in [0, 1, 27, 28, 29, 60, 90, 130] {
            let bar = layout(w, &BarMetrics::test(), &labels(3, 5), 2, &[]);
            assert_sane(&bar, w);
        }
    }

    #[test]
    fn hit_maps_points() {
        let bar = layout(
            800,
            &BarMetrics::test(),
            &["bash".to_string(), "zsh".to_string()],
            0,
            &[],
        );
        assert_eq!(hit(&bar, 790, 10), Some(Target::Close));
        let c = rect_of(&bar, Target::TabClose(1));
        assert_eq!(hit(&bar, c.x + 2, c.y + 2), Some(Target::TabClose(1)));
        assert_eq!(hit(&bar, 2, 2), Some(Target::Tab(0)));
        assert_eq!(hit(&bar, 400, 10), Some(Target::Drag));
        assert_eq!(hit(&bar, 790, 28), None);
    }

    #[test]
    fn resize_edge_corners() {
        let (w, h) = (800, 600);
        assert_eq!(resize_edge(w, h, 0, 0, 4), Some(ResizeDirection::NorthWest));
        assert_eq!(
            resize_edge(w, h, w - 1, h - 1, 4),
            Some(ResizeDirection::SouthEast)
        );
        assert_eq!(
            resize_edge(w, h, w - 1, 0, 4),
            Some(ResizeDirection::NorthEast)
        );
        assert_eq!(
            resize_edge(w, h, 0, h - 1, 4),
            Some(ResizeDirection::SouthWest)
        );
        assert_eq!(resize_edge(w, h, w / 2, 0, 4), Some(ResizeDirection::North));
        assert_eq!(
            resize_edge(w, h, w / 2, h - 1, 4),
            Some(ResizeDirection::South)
        );
        assert_eq!(resize_edge(w, h, 0, h / 2, 4), Some(ResizeDirection::West));
        assert_eq!(
            resize_edge(w, h, w - 1, h / 2, 4),
            Some(ResizeDirection::East)
        );
        assert_eq!(resize_edge(w, h, w / 2, h / 2, 4), None);
    }

    #[test]
    fn natural_width_never_truncates() {
        for n in 6..=24 {
            let label = "w".repeat(n);
            let bar = layout(
                2000,
                &BarMetrics::test(),
                std::slice::from_ref(&label),
                0,
                &[],
            );
            assert_eq!(bar.tabs[0].label, label, "n={n}");
        }
    }

    #[test]
    fn layout_sweep_is_sane() {
        let widths = (1..2000).step_by(7).chain([1, 2, 3]);
        for width in widths {
            for n in 0..30usize {
                let ls: Vec<String> = (0..n).map(|i| "y".repeat(3 + i % 25)).collect();
                for active in [0, n / 2, n.saturating_sub(1)] {
                    let bar = layout(width, &BarMetrics::test(), &ls, active, &[]);
                    assert_sane(&bar, width);
                    let tabs = tab_rects(&bar);
                    let b = BarMetrics::test();
                    let available =
                        bar.items.iter().map(|i| i.rect.x).min().unwrap_or(width) - b.height;
                    if n > 0 && available >= 3 * b.pad + b.cell_h + b.cell_w {
                        assert!(
                            tabs.iter().any(|(i, _)| *i == active),
                            "active hidden w={width} n={n} active={active}"
                        );
                    }
                    if !tabs.is_empty() {
                        assert!(
                            tabs.iter().any(|(i, _)| *i == active),
                            "w={width} n={n} active={active}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn narrow_window_shows_active_tab_truncated() {
        let b = BarMetrics::test();
        // Full right cluster is 7 buttons + the gap; 60px is left for tabs.
        let width = 7 * b.height + b.pad + b.height + 60;
        let ls = vec![
            "alpha-long".to_string(),
            "beta-long".into(),
            "gamma-long".into(),
        ];
        let bar = layout(width, &b, &ls, 1, &[]);
        let tabs = tab_rects(&bar);
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0].0, 1);
        assert_eq!(tabs[0].1.w, 60);
        let new_tab = rect_of(&bar, Target::NewTab);
        assert!(tabs[0].1.x + tabs[0].1.w <= new_tab.x);
        for item in &bar.items {
            if !matches!(item.target, Target::Tab(_) | Target::TabClose(_)) {
                assert!(!overlap(tabs[0].1, item.rect), "{item:?}");
            }
        }
        assert!(bar.tabs[0].label.chars().count() < "beta-long".len());
    }

    #[test]
    fn label_fallbacks() {
        assert_eq!(tab_label(None, Some("")), "montty");
        assert_eq!(tab_label(None, None), "montty");
        assert_eq!(tab_label(Some("x"), Some("y")), "x");
        assert_eq!(tab_label(Some(""), Some("y")), "y");
        assert_eq!(tab_label(None, Some("a\x07b")), "ab");
    }

    #[test]
    fn dot_shifts_label() {
        let ls = ["aaaa".to_string(), "bbbb".to_string()];
        let plain = layout(800, &BarMetrics::test(), &ls, 0, &[false, false]);
        let dotted = layout(800, &BarMetrics::test(), &ls, 0, &[false, true]);
        assert!(plain.tabs.iter().all(|t| t.dot_rect.is_none()));
        assert!(dotted.tabs[0].dot_rect.is_none());
        let dot = dotted.tabs[1].dot_rect.expect("tab 1 has a dot");
        let shift = BarMetrics::test().dot() + BarMetrics::test().pad / 2;
        assert_eq!(
            dotted.tabs[1].text_rect.x,
            plain.tabs[1].text_rect.x + shift
        );
        assert_eq!(dotted.tabs[0].text_rect, plain.tabs[0].text_rect);
        assert_eq!((dot.w, dot.h), (8, 8));
        assert_eq!(dot.x, plain.tabs[1].text_rect.x);
        assert_eq!(dot.y, (BarMetrics::test().height - dot.h) / 2);
    }

    #[test]
    fn dot_diameter_scales_and_has_a_floor() {
        assert_eq!(BarMetrics::test().dot(), 8);
        let big = BarMetrics {
            pad: 12,
            ..BarMetrics::test()
        };
        assert_eq!(big.dot(), 16);
        let tiny = BarMetrics {
            pad: 1,
            ..BarMetrics::test()
        };
        assert_eq!(tiny.dot(), 4);
    }
}
