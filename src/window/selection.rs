//! Mouse selection in panes: drag (simple), double-click (word), triple-click
//! (line), Alt (block) and Shift+click (extend). Output scrolling and history
//! trimming are the terminal's job: it moves and drops the selection itself.

use std::time::Instant;

use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::{Term, TermMode};

use super::Window;
use crate::clipboard::ClipKind;
use crate::input::paste::paste_bytes;
use crate::layout::{PaneId, Rect};
use crate::term::grid_point::{MultiClick, point_at};

/// A selection being dragged in `pane`.
pub(super) struct SelectDrag {
    pane: PaneId,
    /// The pointer left the cell it pressed on (or the press extended a selection);
    /// a click that never did clears the selection on release.
    moved: bool,
    start: Point,
}

#[derive(Default)]
pub(super) struct SelectState {
    pub(super) drag: Option<SelectDrag>,
    pub(super) clicks: MultiClick,
}

/// What a left press does to the pane's selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Press {
    /// Move the end of the existing selection (Shift+click).
    Extend,
    Start(SelectionType),
}

/// `clicks` is the multi-click count (1..=3). Shift extends an existing selection
/// (with none, it starts one) unless the program has mouse reporting on: there
/// Shift is the bypass, so it selects like a plain click. Alt makes it a block,
/// whatever the count.
fn press_kind(clicks: u8, shift: bool, alt: bool, has_selection: bool, mouse_mode: bool) -> Press {
    if shift && has_selection && !mouse_mode {
        return Press::Extend;
    }
    Press::Start(match (alt, clicks) {
        (true, _) => SelectionType::Block,
        (_, 2) => SelectionType::Semantic,
        (_, 3) => SelectionType::Lines,
        _ => SelectionType::Simple,
    })
}

/// A press-release that never moved (a plain single click) clears the selection.
fn clears_on_release(moved: bool) -> bool {
    !moved
}

/// The whole grid, history included, as one Lines selection.
fn select_all_selection<T>(term: &Term<T>) -> Selection {
    let top = Point::new(term.topmost_line(), Column(0));
    let bottom = Point::new(term.bottommost_line(), term.last_column());
    let mut sel = Selection::new(SelectionType::Lines, top, Side::Left);
    sel.update(bottom, Side::Right);
    sel.include_all();
    sel
}

impl Window {
    /// The text of `pane`'s selection, if any.
    pub fn selection_text(&self, pane: PaneId) -> Option<String> {
        self.panes.get(&pane)?.term.lock().selection_to_string()
    }

    /// A selection drag in `pane` ended with a non-empty selection.
    pub fn on_selection_done(&mut self, pane: PaneId) {
        if let Some(text) = self.selection_text(pane).filter(|t| !t.is_empty()) {
            self.clipboards.store(ClipKind::Primary, text);
        }
    }

    /// Copy the focused pane's selection to CLIPBOARD.
    pub(super) fn copy(&mut self) {
        let focused = self.tabs.active().focused;
        if let Some(text) = self.selection_text(focused).filter(|t| !t.is_empty()) {
            self.clipboards.store(ClipKind::Clipboard, text);
        }
    }

    /// Select the pane's whole grid, history included, and offer it as PRIMARY.
    pub(super) fn select_all(&mut self) {
        let focused = self.tabs.active().focused;
        if let Some(pane) = self.panes.get(&focused) {
            let mut term = pane.term.lock();
            term.selection = Some(select_all_selection(&term));
        }
        self.on_selection_done(focused);
        self.schedule();
    }

    pub(super) fn paste_into_focused(&mut self, kind: ClipKind) {
        // With the find bar open, a paste goes into its field.
        if self.find.is_some() {
            self.find_paste(kind);
            return;
        }
        let focused = self.tabs.active().focused;
        self.paste_into(focused, kind);
    }

    /// Paste `kind` into pane `id`: scrolled to the bottom, bracketed when the
    /// program asked for it. Nothing happens when the clipboard is empty.
    pub(super) fn paste_into(&mut self, id: PaneId, kind: ClipKind) {
        let Some(text) = self.clipboards.load(kind) else {
            return;
        };
        let Some(pane) = self.panes.get(&id) else {
            return;
        };
        let mut term = pane.term.lock();
        let bytes = paste_bytes(&text, term.mode().contains(TermMode::BRACKETED_PASTE));
        if bytes.is_empty() {
            return;
        }
        let scrolled = term.grid().display_offset() != 0;
        if scrolled {
            term.scroll_display(Scroll::Bottom);
        }
        drop(term);
        pane.mark_input();
        pane.write(bytes);
        if scrolled {
            self.mark_scrolled(id);
            self.schedule();
        }
    }

    /// The grid cell and half under the pointer in pane `id` (at `rect`), with
    /// whether the pane scrolled. With `autoscroll`, a pointer above the grid or
    /// at/below its real bottom (not the padding or leftover strip) first scrolls
    /// the pane a line, so each motion event moves the view one line.
    pub(super) fn pane_point(
        &mut self,
        id: PaneId,
        rect: Rect,
        autoscroll: bool,
    ) -> Option<(Point, Side, bool)> {
        let font = self.pane_font(id);
        let content = rect.inset(font.padding);
        let (cw, ch) = (font.metrics.cell_width, font.metrics.cell_height);
        let (x, y) = self.mouse.pos();
        let mut term = self.panes.get(&id)?.term.lock();
        let (cols, lines) = (term.columns(), term.screen_lines());
        let bottom = content.y + (lines as u32 * ch) as i32;
        let mut scrolled = false;
        if autoscroll && (y < content.y || y >= bottom) {
            term.scroll_display(Scroll::Delta(if y < content.y { 1 } else { -1 }));
            scrolled = true;
        }
        let offset = term.grid().display_offset();
        let (p, side) = point_at(content, cw, ch, cols, lines, offset, x, y);
        drop(term);
        if scrolled {
            self.mark_scrolled(id);
        }
        Some((p, side, scrolled))
    }

    /// A left press on pane `id` (at `rect`): focus it and start, or with Shift
    /// extend, a selection.
    pub(super) fn selection_press(&mut self, id: PaneId, rect: Rect) {
        if id != self.tabs.active().focused {
            self.set_focus(id);
        }
        let Some((p, side, _)) = self.pane_point(id, rect, false) else {
            return;
        };
        let (shift, alt) = (self.modifiers.shift_key(), self.modifiers.alt_key());
        let clicks = self.mouse.select.clicks.press(p, Instant::now());
        let Some(pane) = self.panes.get(&id) else {
            return;
        };
        let mut term = pane.term.lock();
        let mouse_mode = term.mode().intersects(TermMode::MOUSE_MODE);
        let kind = press_kind(clicks, shift, alt, term.selection.is_some(), mouse_mode);
        match (kind, term.selection.as_mut()) {
            (Press::Extend, Some(sel)) => sel.update(p, side),
            (Press::Extend, None) => {}
            (Press::Start(ty), _) => term.selection = Some(Selection::new(ty, p, side)),
        }
        drop(term);
        self.mouse.select.drag = Some(SelectDrag {
            pane: id,
            moved: kind == Press::Extend || clicks > 1,
            start: p,
        });
        self.schedule();
    }

    /// Pointer motion while a selection drag is on: extend it, autoscrolling
    /// when the pointer is above or below the grid.
    pub(super) fn selection_motion(&mut self) {
        let Some(id) = self.mouse.select.drag.as_ref().map(|d| d.pane) else {
            return;
        };
        let rect = self
            .tabs
            .active()
            .tree
            .layout(self.area())
            .into_iter()
            .find_map(|(p, r)| (p == id).then_some(r));
        let point = rect.and_then(|r| self.pane_point(id, r, true));
        let Some((p, side, scrolled)) = point else {
            self.mouse.select.drag = None;
            return;
        };
        if let Some(pane) = self.panes.get(&id)
            && let Some(sel) = pane.term.lock().selection.as_mut()
        {
            sel.update(p, side);
        }
        if let Some(d) = self.mouse.select.drag.as_mut() {
            d.moved |= scrolled || p != d.start;
        }
        self.schedule();
    }

    /// Left release: end a selection drag. A click that didn't move clears the
    /// selection; a non-empty one is done.
    pub(super) fn selection_release(&mut self) {
        let Some(d) = self.mouse.select.drag.take() else {
            return;
        };
        let Some(pane) = self.panes.get(&d.pane) else {
            return;
        };
        let mut term = pane.term.lock();
        if clears_on_release(d.moved) {
            term.selection = None;
        }
        let done = term.selection.as_ref().is_some_and(|s| !s.is_empty());
        drop(term);
        if done {
            self.on_selection_done(d.pane);
        }
        self.schedule();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicks_pick_the_selection_type() {
        let start = |c| press_kind(c, false, false, true, false);
        assert_eq!(start(1), Press::Start(SelectionType::Simple));
        assert_eq!(start(2), Press::Start(SelectionType::Semantic));
        assert_eq!(start(3), Press::Start(SelectionType::Lines));
    }

    #[test]
    fn alt_makes_a_block_at_any_click_count() {
        for c in 1..=3 {
            assert_eq!(
                press_kind(c, false, true, false, false),
                Press::Start(SelectionType::Block)
            );
        }
    }

    #[test]
    fn shift_extends_only_an_existing_selection() {
        assert_eq!(press_kind(1, true, false, true, false), Press::Extend);
        assert_eq!(
            press_kind(1, true, false, false, false),
            Press::Start(SelectionType::Simple)
        );
        // Shift+Alt on a selection still extends it.
        assert_eq!(press_kind(1, true, true, true, false), Press::Extend);
    }

    #[test]
    fn shift_in_mouse_mode_starts_a_fresh_selection() {
        // Shift is the bypass there, so it can't also mean "extend".
        let press = |clicks| press_kind(clicks, true, false, true, true);
        assert_eq!(press(1), Press::Start(SelectionType::Simple));
        assert_eq!(press(2), Press::Start(SelectionType::Semantic));
        assert_eq!(press(3), Press::Start(SelectionType::Lines));
        assert_eq!(
            press_kind(1, true, true, true, true),
            Press::Start(SelectionType::Block)
        );
        // An unmoved Shift+click there is a plain click: it clears on release.
        let moved = press(1) == Press::Extend;
        assert!(clears_on_release(moved));
    }

    #[test]
    fn select_all_covers_history_through_last_line() {
        use alacritty_terminal::event::VoidListener;
        use alacritty_terminal::term::{Config, test::TermSize};
        use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
        let config = Config {
            scrolling_history: 10,
            ..Config::default()
        };
        let mut t = Term::new(config, &TermSize::new(10, 3), VoidListener);
        Processor::<StdSyncHandler>::new().advance(&mut t, b"one\r\ntwo\r\nthree\r\nfour\r\nfive");
        t.selection = Some(select_all_selection(&t));
        let text = t.selection_to_string().unwrap();
        assert!(text.starts_with("one\ntwo\n"), "{text:?}");
        assert!(text.contains("four\nfive"), "{text:?}");
    }

    #[test]
    fn only_an_unmoved_click_clears() {
        assert!(clears_on_release(false));
        assert!(!clears_on_release(true));
    }
}
