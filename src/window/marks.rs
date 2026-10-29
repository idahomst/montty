//! Attention dots on tabs and panes: which are shown, where, their hover labels. Pure state
//! lives in `attention`; nothing here has a timer.

use super::Window;
use super::attention::{Dot, Level};
use super::draw::paint_link_label;
use crate::layout::{PaneId, Rect};
use crate::render::RectInstance;
use crate::term::status::display_text;
use crate::ui::icons::disc_rects;
use crate::ui::text::display_width;
use crate::ui::theme;
use crate::ui::topbar::BarMetrics;

/// The hover target is the dot grown by this many px per side.
const HOVER_SLOP: i32 = 2;
/// Gap between a dot and its label.
const LABEL_GAP: i32 = 4;

pub(super) fn level_color(level: Level) -> [f32; 4] {
    match level {
        Level::Blocked => theme::MARK_BLOCKED,
        Level::Done => theme::MARK_DONE,
        Level::Error => theme::MARK_ERROR,
    }
}

/// A dot's disc as filled rects.
pub(super) fn disc(dot: Rect, color: [f32; 4]) -> Vec<RectInstance> {
    disc_rects(dot.x + dot.w / 2, dot.y + dot.h / 2, dot.w)
        .into_iter()
        .map(|rect| RectInstance { rect, color })
        .collect()
}

/// A pane's dot: the pane's top-right corner, inset `bm.pad` (6 logical px). None when the
/// pane can't hold it.
pub(super) fn pane_dot_rect(pane: Rect, bm: &BarMetrics) -> Option<Rect> {
    let d = bm.dot();
    let inset = bm.pad;
    (pane.w >= d + 2 * inset && pane.h >= d + 2 * inset).then_some(Rect {
        x: pane.x + pane.w - inset - d,
        y: pane.y + inset,
        w: d,
        h: d,
    })
}

/// Where a pane's dot is drawn and hit: the top-right corner, or, while the pane's hover
/// overlay (`overlay_bg`) is shown, just left of the overlay, centred on its row, so the
/// overlay never covers it. None when there is no room.
pub(super) fn pane_dot_at(pane: Rect, overlay_bg: Option<Rect>, bm: &BarMetrics) -> Option<Rect> {
    let Some(bg) = overlay_bg else {
        return pane_dot_rect(pane, bm);
    };
    let d = bm.dot();
    let r = Rect {
        x: bg.x - bm.pad / 2 - d,
        y: bg.y + (bg.h - d) / 2,
        w: d,
        h: d,
    };
    (r.x >= pane.x).then_some(r)
}

/// The label box for `dot`: below it, left edges aligned, or above it when it wouldn't fit
/// below; clamped to the window.
pub(super) fn dot_label_rect(
    dot: Rect,
    text_cols: usize,
    win: (i32, i32),
    bm: &BarMetrics,
) -> Option<Rect> {
    let h = bm.cell_h + 2 * bm.pad;
    if text_cols == 0 || win.1 < h || win.0 < bm.cell_w + 2 * bm.pad {
        return None;
    }
    let w = (text_cols as i32 * bm.cell_w + 2 * bm.pad).min(win.0);
    let mut y = dot.y + dot.h + LABEL_GAP;
    if y + h > win.1 {
        y = dot.y - LABEL_GAP - h;
    }
    Some(Rect {
        x: dot.x.min(win.0 - w).max(0),
        y: y.clamp(0, win.1 - h),
        w,
        h,
    })
}

fn grown(r: Rect) -> Rect {
    Rect {
        x: r.x - HOVER_SLOP,
        y: r.y - HOVER_SLOP,
        w: r.w + 2 * HOVER_SLOP,
        h: r.h + 2 * HOVER_SLOP,
    }
}

impl Window {
    /// The pane being looked at: the focused pane of the active tab while the window has focus.
    pub(super) fn looking_at(&self) -> Option<PaneId> {
        (self.window_focused && !self.tabs.is_empty()).then(|| self.tabs.active().focused)
    }

    /// Seeing a pane clears its not-seen mark. Called after every event the window handles.
    pub(super) fn refresh_looking(&mut self) {
        if let Some(p) = self.looking_at()
            && self.attention.look(p)
        {
            self.schedule();
        }
    }

    /// Per tab (by index) its most urgent dot's level.
    pub(super) fn tab_marks(&self) -> Vec<Option<Level>> {
        if !self.store.global().attention_marks {
            return vec![None; self.tabs.iter().count()];
        }
        let looking = self.looking_at();
        self.tabs
            .iter()
            .map(|t| {
                self.attention
                    .tab_dot(&t.tree.panes(), looking)
                    .map(|d| d.level)
            })
            .collect()
    }

    fn tab_dot_label(&self, index: usize) -> Option<Dot> {
        let tab = self.tabs.iter().nth(index)?;
        self.attention.tab_dot(&tab.tree.panes(), self.looking_at())
    }

    /// Each visible pane's dot rect, given the overlay now shown (if any).
    fn pane_dot_rects(&self, layout: &[(PaneId, Rect)]) -> Vec<(PaneId, Rect)> {
        let bm = self.bar_metrics();
        let overlay = self.overlay_at_pointer().map(|(id, o, _)| (id, o.bg));
        layout
            .iter()
            .filter_map(|&(id, pane)| {
                let bg = overlay.filter(|(o, _)| *o == id).map(|(_, bg)| bg);
                Some((id, pane_dot_at(pane, bg, &bm)?))
            })
            .collect()
    }

    /// The dot under the pointer, if any: its rect and label.
    fn dot_under_pointer(&self) -> Option<(Rect, String)> {
        if !self.store.global().attention_marks
            || !self.mouse.inside()
            || self.modal_open()
            || self.tabs.is_empty()
        {
            return None;
        }
        let (x, y) = self.mouse.pos();
        let bar = self.bar();
        if y < bar.height {
            return bar.tabs.iter().find_map(|t| {
                let r = t.dot_rect.filter(|r| grown(*r).contains(x, y))?;
                Some((r, self.tab_dot_label(t.index)?.label))
            });
        }
        let looking = self.looking_at();
        let layout = self.tabs.active().tree.layout(self.area());
        self.pane_dot_rects(&layout)
            .into_iter()
            .find_map(|(id, r)| {
                let r = Some(r).filter(|r| grown(*r).contains(x, y))?;
                Some((r, self.attention.pane_dot(id, looking == Some(id))?.label))
            })
    }

    /// The pointer moved or marks changed: one frame when the hovered dot changed.
    pub(super) fn update_mark_hover(&mut self) {
        let now = self.dot_under_pointer().map(|(r, _)| r);
        if now != self.mark_hover {
            self.mark_hover = now;
            self.schedule();
        }
    }

    /// The corner dots of the visible panes, over their contents.
    pub(super) fn draw_pane_dots(&mut self, layout: &[(PaneId, Rect)]) {
        if !self.store.global().attention_marks {
            return;
        }
        let looking = self.looking_at();
        for (id, r) in self.pane_dot_rects(layout) {
            if let Some(dot) = self.attention.pane_dot(id, looking == Some(id)) {
                self.renderer.draw_rects(&disc(r, level_color(dot.level)));
            }
        }
    }

    /// The hovered dot's label, over the panes, under the toast.
    pub(super) fn draw_mark_hover(&mut self) {
        let hovered = self.dot_under_pointer();
        self.mark_hover = hovered.as_ref().map(|(r, _)| *r);
        let Some((dot, text)) = hovered else {
            return;
        };
        let text = display_text(&text);
        let bm = self.bar_metrics();
        if let Some(rect) = dot_label_rect(dot, display_width(&text), self.size_px(), &bm) {
            let p = paint_link_label(rect, &text, &bm, self.chrome_stroke());
            self.draw_paint(&p);
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

    const PANE: Rect = Rect {
        x: 100,
        y: 50,
        w: 300,
        h: 200,
    };

    #[test]
    fn pane_dot_sits_in_the_top_right_corner() {
        let r = pane_dot_rect(PANE, &bm()).unwrap();
        assert_eq!(
            r,
            Rect {
                x: 386,
                y: 56,
                w: 8,
                h: 8
            }
        );
        assert!(pane_dot_rect(Rect { w: 10, ..PANE }, &bm()).is_none());
    }

    #[test]
    fn pane_dot_moves_left_of_the_overlay() {
        let bg = Rect {
            x: 300,
            y: 56,
            w: 96,
            h: 22,
        };
        let r = pane_dot_at(PANE, Some(bg), &bm()).unwrap();
        assert_eq!(
            r,
            Rect {
                x: 289,
                y: 63,
                w: 8,
                h: 8
            }
        );
        assert_eq!(pane_dot_at(PANE, None, &bm()), pane_dot_rect(PANE, &bm()));
        let tight = Rect { x: 102, ..bg };
        assert!(pane_dot_at(PANE, Some(tight), &bm()).is_none());
    }

    #[test]
    fn label_goes_below_then_above_and_stays_in_the_window() {
        let dot = Rect {
            x: 386,
            y: 56,
            w: 8,
            h: 8,
        };
        let below = dot_label_rect(dot, 10, (400, 300), &bm()).unwrap();
        assert_eq!(below.y, 64 + LABEL_GAP);
        assert_eq!(below.x + below.w, 400, "shifted left to fit");
        let low = Rect { y: 290, ..dot };
        let above = dot_label_rect(low, 10, (400, 300), &bm()).unwrap();
        assert_eq!(above.y + above.h + LABEL_GAP, 290);
        assert!(dot_label_rect(dot, 0, (400, 300), &bm()).is_none());
        let tiny = dot_label_rect(dot, 100, (60, 300), &bm()).unwrap();
        assert_eq!((tiny.x, tiny.w), (0, 60));
    }
}
