//! The "cols × rows" labels shown on panes while they are resized.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use crate::layout::{PaneId, Rect};
use crate::ui::topbar::BarMetrics;

/// How long a label stays after the last resize that touched its pane.
pub(super) const RESIZE_LINGER: Duration = Duration::from_secs(1);
/// How long the labels of a "show all sizes" request stay.
pub(super) const SHOW_ALL_FOR: Duration = Duration::from_secs(2);

/// The panes whose size label is up, and when they all go away again.
#[derive(Debug, Default)]
pub(super) struct SizeLabels {
    panes: HashSet<PaneId>,
    until: Option<Instant>,
}

impl SizeLabels {
    /// Label `ids` and extend the linger; an empty slice changes nothing.
    pub(super) fn resized(&mut self, ids: &[PaneId], now: Instant) {
        if ids.is_empty() {
            return;
        }
        self.panes.extend(ids);
        self.extend(now + RESIZE_LINGER);
    }

    /// Label exactly `visible`, for at least `SHOW_ALL_FOR`.
    pub(super) fn show_all(&mut self, visible: &[PaneId], now: Instant) {
        self.panes = visible.iter().copied().collect();
        self.extend(now + SHOW_ALL_FOR);
    }

    fn extend(&mut self, to: Instant) {
        self.until = Some(self.until.map_or(to, |u| u.max(to)));
    }

    pub(super) fn showing(&self) -> bool {
        !self.panes.is_empty()
    }

    pub(super) fn contains(&self, id: PaneId) -> bool {
        self.panes.contains(&id)
    }

    pub(super) fn clear(&mut self) {
        self.panes.clear();
        self.until = None;
    }

    pub(super) fn deadline(&self) -> Option<Instant> {
        self.until
    }

    /// Hide the labels once their time is up; true when a frame is needed.
    pub(super) fn tick(&mut self, now: Instant) -> bool {
        if self.until.is_some_and(|u| now >= u) {
            self.clear();
            return true;
        }
        false
    }
}

/// The resized panes that are in `layout` (the active tab's visible panes).
pub(super) fn visible_resized(resized: &[PaneId], layout: &[(PaneId, Rect)]) -> Vec<PaneId> {
    resized
        .iter()
        .copied()
        .filter(|id| layout.iter().any(|&(p, _)| p == *id))
        .collect()
}

/// How long after a resize montty asked for itself (session open, scale change) the
/// window's `Resized` events stay unlabelled.
pub(super) const QUIET_RESIZE: Duration = Duration::from_millis(500);

/// A window resize labels panes only once the user has pressed a key or button, so the
/// window's first sizing and a tiling manager's settling stay quiet; so does a resize
/// while a modal is open or within `QUIET_RESIZE` of one montty requested.
pub(super) fn window_resize_labels(had_input: bool, modal_open: bool, quiet: bool) -> bool {
    had_input && !modal_open && !quiet
}

pub(super) fn size_text(cols: u16, rows: u16) -> String {
    format!("{cols} \u{d7} {rows}")
}

/// The label's box, centred in `pane`; `None` when the pane cannot hold it.
pub(super) fn size_label_rect(pane: Rect, text_cols: usize, bm: &BarMetrics) -> Option<Rect> {
    let w = text_cols as i32 * bm.cell_w + 2 * bm.pad;
    let h = bm.cell_h + 2 * bm.pad;
    if w > pane.w || h > pane.h {
        return None;
    }
    Some(Rect {
        x: pane.x + (pane.w - w) / 2,
        y: pane.y + (pane.h - h) / 2,
        w,
        h,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: PaneId = 1;
    const B: PaneId = 2;
    const HIDDEN: PaneId = 9;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    fn bm() -> BarMetrics {
        BarMetrics {
            height: 28,
            pad: 6,
            cell_w: 8,
            cell_h: 16,
        }
    }

    #[test]
    fn resized_adds_and_extends() {
        let t0 = Instant::now();
        let mut s = SizeLabels::default();
        s.resized(&[A], t0);
        s.resized(&[B], t0 + ms(500));
        assert!(s.contains(A) && s.contains(B));
        assert_eq!(s.deadline(), Some(t0 + ms(1500)));
        s.resized(&[], t0 + ms(900)); // nothing changed: no extension
        assert_eq!(s.deadline(), Some(t0 + ms(1500)));
    }

    #[test]
    fn tick_clears_and_leaves_no_deadline() {
        let t0 = Instant::now();
        let mut s = SizeLabels::default();
        s.resized(&[A], t0);
        assert!(!s.tick(t0 + ms(999)));
        assert!(s.tick(t0 + ms(1000)));
        assert!(!s.showing());
        assert_eq!(s.deadline(), None);
        assert!(!s.tick(t0 + ms(5000)));
    }

    #[test]
    fn resize_during_show_all_keeps_later_deadline() {
        let t0 = Instant::now();
        let mut s = SizeLabels::default();
        s.show_all(&[A, B], t0);
        s.resized(&[A], t0 + ms(100));
        assert_eq!(s.deadline(), Some(t0 + ms(2000)));
        s.resized(&[A], t0 + ms(1500));
        assert_eq!(s.deadline(), Some(t0 + ms(2500)));
    }

    #[test]
    fn only_visible_resized_panes_are_labelled() {
        let layout = [(A, r(0, 0, 400, 300)), (B, r(400, 0, 400, 300))];
        assert_eq!(visible_resized(&[A, HIDDEN], &layout), vec![A]);
    }

    #[test]
    fn window_resize_labels_need_input() {
        assert!(!window_resize_labels(false, false, false));
        assert!(!window_resize_labels(true, true, false));
        assert!(window_resize_labels(true, false, false));
    }

    #[test]
    fn window_resize_labels_quiet_suppresses() {
        assert!(!window_resize_labels(true, false, true));
    }

    #[test]
    fn size_text_uses_times_sign() {
        assert_eq!(size_text(120, 40), "120 × 40");
    }

    #[test]
    fn label_rect_is_centred() {
        // "120 × 40" is 8 cols: w = 64 + 12 = 76, h = 16 + 12 = 28.
        assert_eq!(
            size_label_rect(r(0, 0, 400, 300), 8, &bm()),
            Some(r(162, 136, 76, 28))
        );
    }

    #[test]
    fn label_rect_none_when_pane_too_small() {
        assert_eq!(size_label_rect(r(0, 0, 75, 300), 8, &bm()), None);
        assert_eq!(size_label_rect(r(0, 0, 400, 27), 8, &bm()), None);
        assert!(size_label_rect(r(0, 0, 76, 28), 8, &bm()).is_some());
    }
}
