//! The thin scrollbar shown over a pane while it is scrolled back (pure geometry).

use crate::layout::Rect;

/// Thumb width, right inset and minimum height at scale 1.
const WIDTH_PX: f64 = 4.0;
const INSET_PX: f64 = 2.0;
const MIN_HEIGHT_PX: f64 = 16.0;

/// The thumb of a pane showing `lines` rows over `history` rows of scrollback, scrolled
/// back by `display_offset`. None without history. Offset 0 puts the thumb at the bottom
/// of the pane, offset == history at the top.
pub fn scrollbar_thumb(
    pane: Rect,
    lines: usize,
    history: usize,
    display_offset: usize,
    scale: f64,
) -> Option<Rect> {
    if history == 0 || lines == 0 {
        return None;
    }
    let px = |v: f64| (v * scale).round() as i32;
    let w = px(WIDTH_PX);
    let total = (lines + history) as f64;
    let h = ((f64::from(pane.h) * lines as f64 / total).round() as i32)
        .max(px(MIN_HEIGHT_PX))
        .min(pane.h);
    let offset = display_offset.min(history);
    let travel = f64::from(pane.h - h);
    let from_top = (travel * (history - offset) as f64 / history as f64).round() as i32;
    Some(Rect {
        x: pane.x + pane.w - px(INSET_PX) - w,
        y: pane.y + from_top,
        w,
        h,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANE: Rect = Rect {
        x: 100,
        y: 50,
        w: 400,
        h: 300,
    };

    #[test]
    fn thumb_none_without_history() {
        assert_eq!(scrollbar_thumb(PANE, 25, 0, 0, 1.0), None);
    }

    #[test]
    fn thumb_at_bottom_when_not_scrolled() {
        let t = scrollbar_thumb(PANE, 25, 75, 0, 1.0).unwrap();
        // 300 × 25 / 100 = 75 px tall, flush with the bottom.
        assert_eq!((t.w, t.h), (4, 75));
        assert_eq!(t.y + t.h, PANE.y + PANE.h);
        // 2 px inset from the right edge.
        assert_eq!(t.x + t.w, PANE.x + PANE.w - 2);
    }

    #[test]
    fn thumb_at_top_when_fully_scrolled() {
        let t = scrollbar_thumb(PANE, 25, 75, 75, 1.0).unwrap();
        assert_eq!(t.y, PANE.y);
    }

    #[test]
    fn thumb_mid_range() {
        // 75 px thumb, 225 px of travel; a third of the history back is a third from the
        // bottom, so two thirds down from the top: 150 px.
        let t = scrollbar_thumb(PANE, 25, 75, 25, 1.0).unwrap();
        assert_eq!((t.y, t.h), (PANE.y + 150, 75));
    }

    #[test]
    fn thumb_min_height() {
        let t = scrollbar_thumb(PANE, 25, 1_000_000, 0, 1.0).unwrap();
        assert_eq!(t.h, 16);
        let t = scrollbar_thumb(PANE, 25, 1_000_000, 0, 1.5).unwrap();
        assert_eq!((t.w, t.h), (6, 24));
    }
}
