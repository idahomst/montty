//! Links under the pointer: while Ctrl is held the link there is underlined (the
//! pointer becomes a hand) and a Ctrl+left press opens it with `xdg-open`.

use std::ffi::OsStr;
use std::process::{Command, Stdio};

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::search::RegexSearch;
use winit::window::CursorIcon;

use super::Window;
use super::mouse::{divider_at, resize_icon};
use crate::layout::{Orientation, PaneId, Rect};
use crate::term::links::{Link, URL_REGEX, link_at};
use crate::ui::text::sanitize;
use crate::ui::topbar::BarMetrics;

pub(super) struct LinkState {
    /// Built once: compiling the URL regex is the costly part.
    regex: RegexSearch,
    /// The link under the pointer; only ever `Some` while Ctrl is held.
    pub(super) hover: Option<(PaneId, Link)>,
}

impl LinkState {
    pub(super) fn new() -> Self {
        Self {
            regex: RegexSearch::new(URL_REGEX).expect("URL_REGEX is a valid pattern"),
            hover: None,
        }
    }
}

/// Links are hovered only with Ctrl held and no drag (a divider's or a selection's) on:
/// a hand while dragging would promise a click that isn't coming.
fn hover_wanted(ctrl: bool, dragging: bool) -> bool {
    ctrl && !dragging
}

impl Window {
    /// The link under the pointer, when it is over a pane and no modal is open.
    fn link_under_pointer(&mut self) -> Option<(PaneId, Link)> {
        if self.modal_open() || !self.mouse.inside() {
            return None;
        }
        let (x, y) = self.mouse.pos();
        let (id, rect) = self.pane_at(x, y)?;
        let (point, _, _) = self.pane_point(id, rect, false)?;
        let font = self.pane_font(id);
        let content = rect.inset(font.padding);
        let (cw, ch) = (font.metrics.cell_width, font.metrics.cell_height);
        let term = self.panes.get(&id)?.term.lock();
        // The edge cell stands in for the padding and leftover strip: not a hit.
        if !in_grid(content, cw, ch, term.columns(), term.screen_lines(), x, y) {
            return None;
        }
        let link = link_at(&term, &mut self.links.regex, point)?;
        Some((id, link))
    }

    /// Recompute the hovered link: on pointer motion and when the modifiers change.
    /// Nothing is searched unless Ctrl is held.
    pub(super) fn update_link_hover(&mut self) {
        let dragging = self.mouse.dragging();
        let hover = if hover_wanted(self.modifiers.control_key(), dragging) {
            self.link_under_pointer()
        } else {
            None
        };
        let changed = hover != self.links.hover;
        let had = self.links.hover.is_some();
        self.links.hover = hover;
        if self.links.hover.is_some() {
            self.set_cursor_icon(CursorIcon::Pointer);
        } else if had {
            let (x, y) = self.mouse.pos();
            let area = self.area();
            let divider =
                divider_at(&self.tabs.active().tree.dividers(area), x, y).map(|d| d.orientation);
            self.set_cursor_icon(pointer_icon(false, divider));
        }
        if changed {
            self.schedule();
        }
    }

    /// A Ctrl+left press on a link whose scheme is allowed opens it and is consumed:
    /// no selection starts. Any other press is left to the normal handling.
    pub(super) fn link_press(&mut self) -> bool {
        if !self.modifiers.control_key() {
            return false;
        }
        // `link_at` only finds links whose scheme may open.
        let Some((_, link)) = self.link_under_pointer() else {
            return false;
        };
        open_detached(OsStr::new(&link.uri));
        true
    }
}

/// The address to show for `link`: its uri, sanitized (OSC 8 uris come from programs),
/// when that differs from the text on screen. None for a plain URL. Invisible format
/// characters (bidi overrides, zero-width) are dropped too: they could make the label
/// read as a different address.
pub(super) fn label_text(link: &Link) -> Option<String> {
    (link.uri != link.text)
        .then(|| {
            sanitize(&link.uri)
                .chars()
                .filter(|&c| !is_invisible_format(c))
                .collect::<String>()
        })
        .filter(|t| !t.is_empty())
}

/// Bidi controls, zero-width characters and the BOM.
fn is_invisible_format(c: char) -> bool {
    matches!(c,
        '\u{061C}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206F}' | '\u{FEFF}')
}

/// The address label's box, in window px: its left edge at `first`'s (the link's first
/// visible cell), just below `last` (its last visible cell), or above `first` when
/// that would pass the bottom of `pane`. Shifted left to stay inside `pane`, and at most
/// as wide as it. None when `pane` can't hold one text row plus padding.
pub(super) fn label_rect(
    first: Rect,
    last: Rect,
    pane: Rect,
    text_cols: usize,
    bm: &BarMetrics,
) -> Option<Rect> {
    let h = bm.cell_h + 2 * bm.pad;
    if text_cols == 0 || pane.w < bm.cell_w + 2 * bm.pad || pane.h < h {
        return None;
    }
    let w = (text_cols as i32 * bm.cell_w + 2 * bm.pad).min(pane.w);
    let mut y = last.y + last.h;
    if y + h > pane.y + pane.h {
        y = (first.y - h).max(pane.y);
    }
    let x = first.x.min(pane.x + pane.w - w).max(pane.x);
    Some(Rect { x, y, w, h })
}

/// The pointer shape outside the bar and modals: the hand over a hovered link, else
/// the divider's resize arrows, else the default.
pub(super) fn pointer_icon(link: bool, divider: Option<Orientation>) -> CursorIcon {
    if link {
        CursorIcon::Pointer
    } else {
        divider.map_or(CursorIcon::Default, resize_icon)
    }
}

/// Whether (x, y) lies on the cells of a `cols` x `lines` grid at `content`.
fn in_grid(content: Rect, cw: u32, ch: u32, cols: usize, lines: usize, x: i32, y: i32) -> bool {
    let (dx, dy) = (i64::from(x - content.x), i64::from(y - content.y));
    (0..cols as i64 * i64::from(cw)).contains(&dx)
        && (0..lines as i64 * i64::from(ch)).contains(&dy)
}

/// Hand `arg` (a URI or a path) to the desktop's opener, detached: no stdio, reaped on a
/// thread.
pub(super) fn open_detached(arg: &OsStr) {
    let child = Command::new("xdg-open")
        .arg(arg)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match child {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => eprintln!("montty: xdg-open: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::index::{Column, Line, Point};

    #[test]
    fn no_link_hover_during_a_drag() {
        assert!(hover_wanted(true, false));
        assert!(!hover_wanted(true, true));
        assert!(!hover_wanted(false, false));
        assert!(!hover_wanted(false, true));
    }

    #[test]
    fn hand_over_a_link_else_divider_else_default() {
        assert_eq!(pointer_icon(true, None), CursorIcon::Pointer);
        assert_eq!(
            pointer_icon(true, Some(Orientation::Horizontal)),
            CursorIcon::Pointer
        );
        assert_eq!(pointer_icon(false, None), CursorIcon::Default);
        assert_eq!(
            pointer_icon(false, Some(Orientation::Vertical)),
            resize_icon(Orientation::Vertical)
        );
    }

    fn bm() -> BarMetrics {
        BarMetrics {
            height: 28,
            pad: 6,
            cell_w: 8,
            cell_h: 16,
        }
    }

    fn cell(x: i32, y: i32) -> Rect {
        Rect { x, y, w: 9, h: 18 }
    }

    const PANE: Rect = Rect {
        x: 100,
        y: 50,
        w: 300,
        h: 200,
    };

    #[test]
    fn label_sits_below_the_link() {
        let r = label_rect(cell(120, 70), cell(150, 88), PANE, 10, &bm()).unwrap();
        assert_eq!(
            r,
            Rect {
                x: 120,
                y: 106,
                w: 92,
                h: 28
            }
        );
    }

    #[test]
    fn label_flips_above_at_the_pane_bottom() {
        // The row below would end at 250 + 10: past the pane's bottom (250).
        let r = label_rect(cell(120, 214), cell(120, 232), PANE, 10, &bm()).unwrap();
        assert_eq!((r.y, r.y + r.h), (214 - 28, 214));
        // Fits exactly: stays below.
        let r = label_rect(cell(120, 186), cell(120, 204), PANE, 10, &bm()).unwrap();
        assert_eq!(r.y, 222);
        assert_eq!(r.y + r.h, 250);
    }

    #[test]
    fn label_is_clamped_to_the_pane_edge() {
        let r = label_rect(cell(380, 70), cell(380, 70), PANE, 10, &bm()).unwrap();
        assert_eq!((r.x, r.w), (400 - 92, 92));
        // Left of the pane: pulled in too.
        let r = label_rect(cell(90, 70), cell(90, 70), PANE, 10, &bm()).unwrap();
        assert_eq!(r.x, 100);
    }

    #[test]
    fn label_is_at_most_the_pane_wide() {
        let r = label_rect(cell(200, 70), cell(200, 70), PANE, 500, &bm()).unwrap();
        assert_eq!((r.x, r.w), (100, 300));
    }

    #[test]
    fn no_label_in_a_pane_too_small() {
        let small = |w, h| Rect { w, h, ..PANE };
        let f = |p| label_rect(cell(100, 50), cell(100, 50), p, 5, &bm());
        assert_eq!(f(small(19, 200)), None);
        assert!(f(small(20, 200)).is_some());
        assert_eq!(f(small(300, 27)), None);
        assert!(f(small(300, 28)).is_some());
        assert_eq!(
            label_rect(cell(100, 50), cell(100, 50), PANE, 0, &bm()),
            None
        );
    }

    #[test]
    fn label_only_when_the_address_differs_from_the_text() {
        let link = |uri: &str, text: &str| Link {
            uri: uri.into(),
            text: text.into(),
            range: Point::new(Line(0), Column(0))..=Point::new(Line(0), Column(1)),
        };
        assert_eq!(label_text(&link("https://a.b", "https://a.b")), None);
        assert_eq!(
            label_text(&link("https://a.b", "click")).as_deref(),
            Some("https://a.b")
        );
        assert_eq!(
            label_text(&link("https://www.a.b", "www.a.b")).as_deref(),
            Some("https://www.a.b")
        );
        // Bidi overrides and zero-width characters would disguise the address.
        assert_eq!(
            label_text(&link("https://a\u{202E}b\u{200B}.c\u{2066}d\u{FEFF}", "x")).as_deref(),
            Some("https://ab.cd")
        );
        // Control characters from a program are dropped.
        assert_eq!(
            label_text(&link("https://a\x1b.b", "x")).as_deref(),
            Some("https://a.b")
        );
    }

    #[test]
    fn padding_and_leftover_strip_are_not_on_a_cell() {
        let content = Rect {
            x: 4,
            y: 34,
            w: 100,
            h: 100,
        };
        // 5 cols x 2 lines of 9x18 cells: x 4..49, y 34..70.
        assert!(in_grid(content, 9, 18, 5, 2, 4, 34));
        assert!(in_grid(content, 9, 18, 5, 2, 48, 69));
        assert!(!in_grid(content, 9, 18, 5, 2, 3, 40));
        assert!(!in_grid(content, 9, 18, 5, 2, 49, 40));
        assert!(!in_grid(content, 9, 18, 5, 2, 10, 70));
        assert!(!in_grid(content, 9, 18, 5, 2, 10, 33));
    }
}
