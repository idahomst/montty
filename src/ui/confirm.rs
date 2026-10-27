//! Close confirmation prompt: a message and Close / Cancel buttons, centered
//! (pure geometry, keys and clicks).

use winit::keyboard::{Key, NamedKey};

use super::text::display_width;
use super::topbar::BarMetrics;
use crate::layout::{PaneId, Rect};

pub const CLOSE_LABEL: &str = "Close";
pub const CANCEL_LABEL: &str = "Cancel";

/// What a confirmed close closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseTarget {
    Pane(PaneId),
    /// The tab holding this pane (its focused pane when asked).
    Tab(PaneId),
    Window,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmOutcome {
    None,
    Yes,
    No,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmButton {
    Close,
    Cancel,
}

pub struct Confirm {
    pub target: CloseTarget,
    pub message: String,
    /// The button under the pointer (drawn in HOVER).
    pub hover: Option<ConfirmButton>,
}

/// The prompt's text for `running` panes with a foreground job.
pub fn confirm_message(running: usize) -> String {
    match running {
        1 => "A process is still running. Close anyway?".into(),
        n => format!("{n} processes are still running. Close anyway?"),
    }
}

/// The prompt's text for closing a window that holds `panes` panes.
pub fn panes_message(panes: usize) -> String {
    format!("This window has {panes} panes. Close anyway?")
}

/// Both buttons are as wide as the longer label plus padding, one bar tall.
fn button_size(bm: &BarMetrics) -> (i32, i32) {
    let cols = display_width(CLOSE_LABEL).max(display_width(CANCEL_LABEL)) as i32;
    (cols * bm.cell_w + 2 * bm.pad, bm.height)
}

/// Space between the prompt's border and its contents, and between the
/// message and the buttons.
fn margin(bm: &BarMetrics) -> i32 {
    2 * bm.pad
}

impl Confirm {
    pub fn new(target: CloseTarget, message: String) -> Self {
        Self {
            target,
            message,
            hover: None,
        }
    }

    /// Centered: the message on top, the buttons in the bottom-right corner.
    /// At most 90% of the window each way (the message is then truncated).
    pub fn rect(&self, win_w: i32, win_h: i32, bm: &BarMetrics) -> Rect {
        let m = margin(bm);
        let (bw, bh) = button_size(bm);
        let text_w = display_width(&self.message) as i32 * bm.cell_w;
        let want_w = text_w.max(2 * bw + bm.pad) + 2 * m;
        let want_h = bm.cell_h + bh + 3 * m;
        let w = want_w.min(win_w * 9 / 10).max(0);
        let h = want_h.min(win_h * 9 / 10).max(0);
        Rect {
            x: ((win_w - w) / 2).max(0),
            y: ((win_h - h) / 2).max(0),
            w,
            h,
        }
    }

    /// (Close, Cancel): Close in the bottom-right corner (the default, as on
    /// GNOME), Cancel to its left.
    pub fn button_rects(&self, win_w: i32, win_h: i32, bm: &BarMetrics) -> (Rect, Rect) {
        let r = self.rect(win_w, win_h, bm);
        let m = margin(bm);
        let (w, h) = button_size(bm);
        let close = Rect {
            x: r.x + r.w - m - w,
            y: r.y + r.h - m - h,
            w,
            h,
        };
        let cancel = Rect {
            x: close.x - bm.pad - w,
            ..close
        };
        (close, cancel)
    }

    pub fn button_at(
        &self,
        x: i32,
        y: i32,
        win_w: i32,
        win_h: i32,
        bm: &BarMetrics,
    ) -> Option<ConfirmButton> {
        let (close, cancel) = self.button_rects(win_w, win_h, bm);
        if close.contains(x, y) {
            Some(ConfirmButton::Close)
        } else if cancel.contains(x, y) {
            Some(ConfirmButton::Cancel)
        } else {
            None
        }
    }

    /// Pointer moved: the button under it is hovered. True when that changed.
    pub fn hover(&mut self, x: i32, y: i32, win_w: i32, win_h: i32, bm: &BarMetrics) -> bool {
        let now = self.button_at(x, y, win_w, win_h, bm);
        let changed = now != self.hover;
        self.hover = now;
        changed
    }

    /// Enter or y closes; Esc or n cancels.
    pub fn key(&self, key: &Key) -> ConfirmOutcome {
        match key {
            Key::Named(NamedKey::Enter) => ConfirmOutcome::Yes,
            Key::Named(NamedKey::Escape) => ConfirmOutcome::No,
            Key::Character(c) if c.eq_ignore_ascii_case("y") => ConfirmOutcome::Yes,
            Key::Character(c) if c.eq_ignore_ascii_case("n") => ConfirmOutcome::No,
            _ => ConfirmOutcome::None,
        }
    }

    /// A press on Close is Yes, on Cancel No; anywhere else (inside the
    /// prompt or outside it) does nothing: the prompt stays.
    pub fn click(&self, x: i32, y: i32, win_w: i32, win_h: i32, bm: &BarMetrics) -> ConfirmOutcome {
        match self.button_at(x, y, win_w, win_h, bm) {
            Some(ConfirmButton::Close) => ConfirmOutcome::Yes,
            Some(ConfirmButton::Cancel) => ConfirmOutcome::No,
            None => ConfirmOutcome::None,
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

    fn confirm() -> Confirm {
        Confirm::new(CloseTarget::Pane(1), confirm_message(1))
    }

    fn center(r: Rect) -> (i32, i32) {
        (r.x + r.w / 2, r.y + r.h / 2)
    }

    fn inside(outer: Rect, r: Rect) -> bool {
        r.x >= outer.x
            && r.y >= outer.y
            && r.x + r.w <= outer.x + outer.w
            && r.y + r.h <= outer.y + outer.h
    }

    #[test]
    fn confirm_keys() {
        let c = confirm();
        let ch = |s: &str| Key::Character(s.into());
        assert_eq!(c.key(&Key::Named(NamedKey::Enter)), ConfirmOutcome::Yes);
        assert_eq!(c.key(&ch("y")), ConfirmOutcome::Yes);
        assert_eq!(c.key(&Key::Named(NamedKey::Escape)), ConfirmOutcome::No);
        assert_eq!(c.key(&ch("n")), ConfirmOutcome::No);
        // Shifted letters count too; anything else does nothing.
        assert_eq!(c.key(&ch("Y")), ConfirmOutcome::Yes);
        assert_eq!(c.key(&ch("N")), ConfirmOutcome::No);
        assert_eq!(c.key(&ch("W")), ConfirmOutcome::None);
        assert_eq!(c.key(&Key::Named(NamedKey::Space)), ConfirmOutcome::None);
    }

    #[test]
    fn confirm_message_plural() {
        assert_eq!(
            confirm_message(1),
            "A process is still running. Close anyway?"
        );
        assert_eq!(
            confirm_message(2),
            "2 processes are still running. Close anyway?"
        );
        assert_eq!(
            confirm_message(12),
            "12 processes are still running. Close anyway?"
        );
    }

    #[test]
    fn panes_message_counts_panes() {
        assert_eq!(panes_message(3), "This window has 3 panes. Close anyway?");
    }

    #[test]
    fn confirm_centered_with_buttons_inside() {
        let bm = bm();
        let c = confirm();
        let r = c.rect(800, 600, &bm);
        assert_eq!((r.x, r.y), ((800 - r.w) / 2, (600 - r.h) / 2));
        // The message plus margins sets the width.
        assert_eq!(r.w, 41 * 8 + 4 * 6);
        let (close, cancel) = c.button_rects(800, 600, &bm);
        assert!(
            inside(r, close) && inside(r, cancel),
            "{close:?} {cancel:?}"
        );
        // Same size, one bar tall, Cancel left of Close with a gap, both below the message.
        assert_eq!((close.w, close.h), (cancel.w, cancel.h));
        assert_eq!(close.h, bm.height);
        assert!(cancel.x + cancel.w < close.x);
        assert_eq!(close.y, cancel.y);
        assert!(close.y >= r.y + 2 * bm.pad + bm.cell_h);
        // Wide enough for the longer label.
        assert!(cancel.w >= 6 * bm.cell_w + 2 * bm.pad);
    }

    #[test]
    fn confirm_clicks() {
        let bm = bm();
        let c = confirm();
        let r = c.rect(800, 600, &bm);
        let (close, cancel) = c.button_rects(800, 600, &bm);
        let click = |(x, y): (i32, i32)| c.click(x, y, 800, 600, &bm);
        assert_eq!(click(center(close)), ConfirmOutcome::Yes);
        assert_eq!(click(center(cancel)), ConfirmOutcome::No);
        // Elsewhere in the prompt or outside it: nothing (it stays open).
        assert_eq!(click((r.x + 2, r.y + 2)), ConfirmOutcome::None);
        assert_eq!(click((1, 1)), ConfirmOutcome::None);
    }

    #[test]
    fn confirm_hover_reports_changes() {
        let bm = bm();
        let mut c = confirm();
        let (close, cancel) = c.button_rects(800, 600, &bm);
        let (x, y) = center(close);
        assert!(c.hover(x, y, 800, 600, &bm));
        assert_eq!(c.hover, Some(ConfirmButton::Close));
        assert!(!c.hover(x + 1, y, 800, 600, &bm));
        let (x, y) = center(cancel);
        assert!(c.hover(x, y, 800, 600, &bm));
        assert_eq!(c.hover, Some(ConfirmButton::Cancel));
        assert!(c.hover(1, 1, 800, 600, &bm));
        assert_eq!(c.hover, None);
    }

    #[test]
    fn confirm_never_negative() {
        let bm = bm();
        let c = Confirm::new(CloseTarget::Window, confirm_message(3));
        for (w, h) in [(0, 0), (1, 1), (50, 20), (120, 400)] {
            let r = c.rect(w, h, &bm);
            assert!(r.x >= 0 && r.y >= 0 && r.w >= 0 && r.h >= 0, "{r:?}");
            assert!(r.w <= w && r.h <= h, "{r:?} in {w}x{h}");
            let _ = c.button_rects(w, h, &bm);
            let _ = c.click(0, 0, w, h, &bm);
        }
    }
}
