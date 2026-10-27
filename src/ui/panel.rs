//! Centered info panel (shortcuts list, about): geometry and scrolling (pure).

use super::text::display_width;
use super::topbar::BarMetrics;
use crate::layout::Rect;

pub struct InfoPanel {
    pub title: String,
    pub lines: Vec<String>,
    /// First body line shown; the title never scrolls.
    pub scroll: usize,
}

/// One body row as drawn: a line, or a marker standing in for the rows
/// scrolled out above or below (it replaces the first / last visible line).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelRow<'a> {
    Line(&'a str),
    MoreAbove(usize),
    MoreBelow(usize),
}

/// A marker row's text ("↑ 3 more" / "↓ 3 more"); None for a line.
pub fn more_text(row: PanelRow<'_>) -> Option<String> {
    match row {
        PanelRow::Line(_) => None,
        PanelRow::MoreAbove(n) => Some(format!("↑ {n} more")),
        PanelRow::MoreBelow(n) => Some(format!("↓ {n} more")),
    }
}

/// Fewest visible rows that get "more" markers; smaller panels show lines only.
const MIN_ROWS_FOR_MARKERS: usize = 3;

impl InfoPanel {
    pub fn new(title: impl Into<String>, lines: Vec<String>) -> Self {
        Self {
            title: title.into(),
            lines,
            scroll: 0,
        }
    }

    /// Whole body rows that fit in `rect` (as returned by `rect`) below the title.
    pub fn visible_lines(&self, rect: Rect, bm: &BarMetrics) -> usize {
        let rows = (rect.h - 3 * bm.pad - bm.cell_h).max(0) / bm.cell_h.max(1);
        (rows as usize).min(self.lines.len())
    }

    fn max_scroll(&self, visible: usize) -> usize {
        self.lines.len().saturating_sub(visible)
    }

    /// Scroll by `delta` lines with `visible` rows shown, clamped so the last
    /// line can reach the bottom row. True when the position changed.
    pub fn scroll_by(&mut self, delta: isize, visible: usize) -> bool {
        let max = self.max_scroll(visible);
        let from = self.scroll.min(max) as isize;
        let to = from.saturating_add(delta).clamp(0, max as isize) as usize;
        let changed = to != self.scroll;
        self.scroll = to;
        changed
    }

    /// The `visible` body rows at the current scroll position. With at least
    /// three rows, hidden lines above / below are counted by a marker in the
    /// first / last row (the line it covers counts as hidden).
    pub fn body_rows(&self, visible: usize) -> Vec<PanelRow<'_>> {
        let len = self.lines.len();
        let visible = visible.min(len);
        // A scroll past the end (the panel grew) shows the end.
        let start = self.scroll.min(self.max_scroll(visible));
        let mut rows: Vec<PanelRow<'_>> = self.lines[start..start + visible]
            .iter()
            .map(|l| PanelRow::Line(l))
            .collect();
        if visible >= MIN_ROWS_FOR_MARKERS {
            if start > 0 {
                rows[0] = PanelRow::MoreAbove(start + 1);
            }
            let below = len - start - visible;
            if below > 0 {
                rows[visible - 1] = PanelRow::MoreBelow(below + 1);
            }
        }
        rows
    }

    /// Centered; at most 90% of the window in each direction (lines past that
    /// scroll). A panel that scrolls is wide enough for its "more" markers.
    pub fn rect(&self, win_w: i32, win_h: i32, bm: &BarMetrics) -> Rect {
        // Title row, a pad gap, then the body lines.
        let want_h = (self.lines.len() as i32 + 1) * bm.cell_h + 3 * bm.pad;
        let h = want_h.min(win_h * 9 / 10).max(0);
        let scrolls = h < want_h;
        let marker = more_text(PanelRow::MoreBelow(self.lines.len())).filter(|_| scrolls);
        let cols = self
            .lines
            .iter()
            .chain([&self.title])
            .chain(marker.as_ref())
            .map(|l| display_width(l))
            .max()
            .unwrap_or(0) as i32;
        let want_w = cols * bm.cell_w + 2 * bm.pad;
        let w = want_w.min(win_w * 9 / 10).max(0);
        Rect {
            x: ((win_w - w) / 2).max(0),
            y: ((win_h - h) / 2).max(0),
            w,
            h,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bm() -> BarMetrics {
        BarMetrics {
            height: 28,
            pad: 6,
            cell_w: 8,
            cell_h: 16,
        }
    }

    fn panel(n: usize) -> InfoPanel {
        InfoPanel::new(
            "Shortcuts",
            (0..n).map(|i| format!("line {i} of text")).collect(),
        )
    }

    #[test]
    fn panel_centered_and_clamped() {
        let p = panel(3);
        let r = p.rect(800, 600, &bm());
        assert_eq!(r.w, 14 * 8 + 12);
        assert_eq!(r.x, (800 - r.w) / 2);
        assert_eq!(r.y, (600 - r.h) / 2);
        // Many lines clamp to 90% of the window height.
        let r = panel(200).rect(800, 600, &bm());
        assert_eq!(r.h, 540);
        assert_eq!(r.y, 30);
        // A long line clamps to 90% of the width.
        let wide = InfoPanel::new("t", vec!["x".repeat(500)]);
        assert_eq!(wide.rect(800, 600, &bm()).w, 720);
    }

    #[test]
    fn scrolling_panel_fits_its_marker() {
        let bm = bm();
        let short = |n: usize| InfoPanel::new("T", (0..n).map(|i| i.to_string()).collect());
        // "↓ 100 more" is 10 columns, wider than any line.
        assert_eq!(short(100).rect(800, 600, &bm).w, 10 * 8 + 12);
        // No marker when everything fits.
        assert_eq!(short(5).rect(800, 600, &bm).w, 8 + 12);
    }

    #[test]
    fn panel_never_negative() {
        for (w, h) in [(0, 0), (1, 1), (5, 3)] {
            let r = panel(5).rect(w, h, &bm());
            assert!(r.x >= 0 && r.y >= 0 && r.w >= 0 && r.h >= 0, "{r:?}");
        }
    }

    #[test]
    fn visible_lines_fit_below_title() {
        let bm = bm();
        let p = panel(200);
        let r = p.rect(800, 600, &bm);
        // 540 px: pad, title, pad, rows, pad.
        assert_eq!(p.visible_lines(r, &bm), ((540 - 3 * 6 - 16) / 16) as usize);
        let p = panel(3);
        assert_eq!(p.visible_lines(p.rect(800, 600, &bm), &bm), 3);
        assert_eq!(p.visible_lines(p.rect(5, 5, &bm), &bm), 0);
    }

    #[test]
    fn scroll_clamps_to_content() {
        let mut p = panel(10);
        assert!(!p.scroll_by(-1, 4));
        assert!(p.scroll_by(3, 4));
        assert_eq!(p.scroll, 3);
        assert!(p.scroll_by(100, 4));
        assert_eq!(p.scroll, 6);
        assert!(!p.scroll_by(1, 4));
        assert!(p.scroll_by(-100, 4));
        assert_eq!(p.scroll, 0);
        // Everything fits: nothing to scroll.
        assert!(!p.scroll_by(5, 10));
        assert!(!p.scroll_by(5, 20));
        assert_eq!(p.scroll, 0);
    }

    #[test]
    fn body_rows_mark_hidden_lines() {
        use PanelRow::*;
        let mut p = panel(10);
        let l = |i: usize| format!("line {i} of text");
        assert_eq!(
            p.body_rows(4),
            [Line(&l(0)), Line(&l(1)), Line(&l(2)), MoreBelow(7)]
        );
        p.scroll_by(2, 4);
        assert_eq!(
            p.body_rows(4),
            [MoreAbove(3), Line(&l(3)), Line(&l(4)), MoreBelow(5)]
        );
        p.scroll_by(100, 4);
        assert_eq!(
            p.body_rows(4),
            [MoreAbove(7), Line(&l(7)), Line(&l(8)), Line(&l(9))]
        );
        // Too few rows for markers; everything fitting needs none.
        assert_eq!(p.body_rows(2), [Line(&l(6)), Line(&l(7))]);
        // Scrolled past the end (the window grew): shown as the end.
        p.scroll = 100;
        assert_eq!(p.body_rows(4)[3], Line(&l(9)));
        let p = panel(2);
        assert_eq!(p.body_rows(5), [Line(&l(0)), Line(&l(1))]);
    }
}
