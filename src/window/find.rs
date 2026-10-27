//! The find bar's search glue: literal scrollback search, highlights, and the window's
//! open/close, key and press handling and drawing.

use winit::event::{KeyEvent, MouseButton};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

use super::Window;
use super::bar::{BarPaint, Label};
use super::draw::border_rects;
use super::modal::{field_view, framed};
use crate::clipboard::ClipKind;
use crate::input::Mods;
use crate::input::bindings::Action;
use crate::layout::Rect;
use crate::render::RectInstance;
use crate::ui::findbar::{FindKey, FindLayout, FindTarget, findbar_hit, findbar_layout};
use crate::ui::icons::{Icon, icon_rects};
use crate::ui::text::display_width;
use crate::ui::theme;
use crate::ui::topbar::BarMetrics;
use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Boundary, Column, Direction, Line, Point, Side};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};

use crate::layout::PaneId;
use crate::term::links::line_search_left_capped;
use crate::ui::findbar::FindBar;

pub struct FindState {
    pub pane: PaneId,
    pub bar: FindBar,
    pub regex: Option<RegexSearch>,
    pub current: Option<Match>,
}

impl FindState {
    pub fn new(pane: PaneId) -> Self {
        Self {
            pane,
            bar: FindBar::new(),
            regex: None,
            current: None,
        }
    }

    /// The query or the case flag changed: compile it and search from the bottom of the
    /// scrollback upward, so the nearest match is the current one.
    pub fn query_changed<T: EventListener>(&mut self, term: &mut Term<T>) {
        self.regex = self.bar.pattern().and_then(|p| RegexSearch::new(&p).ok());
        self.current = None;
        self.bar.no_match = false;
        self.search(term, Direction::Left);
    }

    /// Move to the match past the current one: `Right` toward newer text, `Left` toward
    /// older, both wrapping. With no current match it starts from the bottom (`Left`) or
    /// the top (`Right`). A found match is scrolled into view; none sets `no_match` and
    /// leaves the scroll position alone.
    pub fn search<T: EventListener>(&mut self, term: &mut Term<T>, dir: Direction) {
        // A query that didn't compile (too big) finds nothing.
        let Some(regex) = self.regex.as_mut() else {
            self.current = None;
            self.bar.no_match = !self.bar.field.text().is_empty();
            return;
        };
        // The text moved under a stale match (history trimmed, the grid resized): start
        // afresh from the bottom rather than from a point that may not exist.
        let mut dir = dir;
        if self.current.as_ref().is_some_and(|m| !on_grid(term, m)) {
            self.current = None;
            dir = Direction::Left;
        }
        let (origin, side) = match (&self.current, dir) {
            (Some(m), Direction::Right) => (m.end().add(term, Boundary::None, 1), Side::Left),
            (Some(m), Direction::Left) => (m.start().sub(term, Boundary::None, 1), Side::Right),
            (None, Direction::Left) => (
                Point::new(term.bottommost_line(), term.last_column()),
                Side::Right,
            ),
            (None, Direction::Right) => (Point::new(term.topmost_line(), Column(0)), Side::Left),
        };
        self.current = term.search_next(regex, origin, dir, side, None);
        self.bar.no_match = self.current.is_none();
        if let Some(m) = &self.current {
            term.scroll_to_point(*m.start());
        }
    }

    /// Output arrived: grid coordinates shift as lines scroll, so drop the current match
    /// unless the query still matches exactly there. The next step then starts afresh
    /// from the bottom instead of from a point that now holds other text.
    pub fn revalidate<T>(&mut self, term: &Term<T>) {
        let (Some(regex), Some(m)) = (self.regex.as_mut(), self.current.as_ref()) else {
            return;
        };
        let same = on_grid(term, m)
            && RegexIter::new(*m.start(), *m.end(), Direction::Right, term, regex).next()
                == Some(m.clone());
        if !same {
            self.current = None;
        }
    }
}

/// Whether both ends of `m` are points of `term`'s grid.
fn on_grid<T>(term: &Term<T>, m: &Match) -> bool {
    let ok = |p: &Point| {
        p.line >= term.topmost_line()
            && p.line <= term.bottommost_line()
            && p.column <= term.last_column()
    };
    ok(m.start()) && ok(m.end())
}

/// The matches on the screen as it is now scrolled, in grid order and not overlapping
/// (what `Snapshot::capture` needs).
pub fn visible_matches<T>(term: &Term<T>, regex: &mut RegexSearch) -> Vec<Match> {
    let offset = term.grid().display_offset() as i32;
    let top = Point::new(Line(-offset), Column(0));
    // From the start of the top line's wrapped line, so a match that begins above the
    // screen and runs into it is found; matches wholly above are dropped.
    let from = Some(line_search_left_capped(term, top))
        .filter(|p| p.line <= top.line)
        .unwrap_or(top);
    let bottom = Point::new(
        Line(-offset + term.screen_lines() as i32 - 1),
        term.last_column(),
    );
    RegexIter::new(from, bottom, Direction::Right, term, regex)
        .filter(|m| *m.end() >= top)
        .collect()
}

/// The no-matches label shown in the field.
const NO_MATCH: &str = "no matches";

/// Bindings that still run while the bar is open; every other key goes to the field.
fn runs_from_bar(a: Action) -> bool {
    matches!(
        a,
        Action::Find
            | Action::FindNext
            | Action::FindPrev
            | Action::NewTab
            | Action::CloseTab
            | Action::NextTab
            | Action::PrevTab
            // Paste goes into the field; copy still copies the pane's selection.
            | Action::Copy
            | Action::Paste
            | Action::PastePrimary
    )
}

/// The bar's rects and labels: the overlay's panel, the field like the rename field
/// (border, text, beam; the text highlighted while it is all selected; `no matches`
/// at its right end when it fits) and the four buttons.
pub(super) fn paint_find(l: &FindLayout, bar: &FindBar, bm: &BarMetrics, stroke: i32) -> BarPaint {
    let mut out = BarPaint::default();
    let r = l.rect;
    if r.w <= 2 * stroke || r.h <= 2 * stroke {
        return out;
    }
    let fill = |rect: Rect, color: [f32; 4]| RectInstance { rect, color };
    out.under.push(fill(r.inset(stroke), theme::OVERLAY_BG));
    // Edges, not a filled rect under the bg: the bg is translucent.
    out.under.extend(
        border_rects(r, stroke)
            .into_iter()
            .map(|e| fill(e, theme::OVERLAY_BORDER)),
    );
    let half = (bm.pad / 2).max(stroke);
    let cw = bm.cell_w.max(1);
    let text_y = r.y + (r.h - bm.cell_h) / 2;
    let fb = l.field.inset(half + stroke);
    let inner = fb.inset(stroke);
    if inner.w > 2 * half {
        out.under
            .extend(framed(fb, stroke, theme::TAB_ACTIVE, theme::ACCENT));
        let mut text = Rect {
            x: inner.x + half,
            y: text_y,
            w: inner.w - 2 * half,
            h: bm.cell_h,
        };
        // The label sits at the right end, with a column of room before it, when the
        // query keeps at least four columns.
        let label_w = (display_width(NO_MATCH) as i32 + 1) * cw;
        if bar.no_match && text.w - label_w >= 4 * cw {
            text.w -= label_w;
            out.labels.push(Label {
                x: text.x + text.w + cw,
                y: text_y,
                text: NO_MATCH.into(),
                color: theme::to_rgb(theme::CLOSE_HOVER),
                clip: Rect {
                    x: text.x + text.w,
                    w: label_w,
                    ..text
                },
            });
        }
        let cols = (text.w / cw).max(0) as usize;
        let (shown, col) = field_view(bar.field.text(), bar.field.cursor(), cols);
        if bar.select_all && !shown.is_empty() {
            let w = (display_width(&shown) as i32 * cw).min(text.w);
            out.under.push(fill(Rect { w, ..text }, theme::ACCENT));
        }
        out.labels.push(Label {
            x: text.x,
            y: text_y,
            text: shown,
            color: theme::to_rgb(theme::TEXT),
            clip: text,
        });
        if !bar.select_all {
            out.icons.push(fill(
                Rect {
                    x: text.x + col as i32 * cw,
                    y: text_y,
                    w: stroke,
                    h: bm.cell_h,
                },
                theme::TEXT,
            ));
        }
    }
    for &(b, target) in &l.buttons {
        let glyph = match target {
            FindTarget::Prev => "\u{2191}",
            FindTarget::Next => "\u{2193}",
            FindTarget::Case => "Aa",
            FindTarget::Field | FindTarget::Close => "",
        };
        if target == FindTarget::Close {
            out.icons.extend(
                icon_rects(Icon::Close, b, stroke)
                    .into_iter()
                    .map(|i| fill(i, theme::ICON)),
            );
        }
        if target == FindTarget::Case && bar.match_case {
            out.under.push(fill(b.inset(half), theme::ACCENT));
        }
        if !glyph.is_empty() {
            out.labels.push(Label {
                x: b.x + (b.w - display_width(glyph) as i32 * cw) / 2,
                y: b.y + (b.h - bm.cell_h) / 2,
                text: glyph.into(),
                color: theme::to_rgb(theme::TEXT),
                clip: b,
            });
        }
    }
    out
}

/// What a press on the find bar does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BarClick {
    Step(Direction),
    ToggleCase,
    Close,
    PastePrimary,
}

/// A left click works the bar's buttons; a middle click anywhere on the bar pastes the
/// primary selection into the field, as in other text fields.
fn bar_click(target: FindTarget, button: MouseButton) -> Option<BarClick> {
    match (button, target) {
        (MouseButton::Middle, _) => Some(BarClick::PastePrimary),
        (MouseButton::Left, FindTarget::Prev) => Some(BarClick::Step(Direction::Left)),
        (MouseButton::Left, FindTarget::Next) => Some(BarClick::Step(Direction::Right)),
        (MouseButton::Left, FindTarget::Case) => Some(BarClick::ToggleCase),
        (MouseButton::Left, FindTarget::Close) => Some(BarClick::Close),
        _ => None,
    }
}

impl Window {
    /// The bar's layout in its pane; None with no bar (or its pane out of view).
    pub(super) fn find_layout(&self) -> Option<FindLayout> {
        let pane = self.find.as_ref()?.pane;
        let rect = self.pane_rect(pane)?;
        Some(findbar_layout(
            rect,
            &self.bar_metrics(),
            self.overlay_inset(),
        ))
    }

    /// Ctrl+Shift+F or the top bar's button: the bar opens on the focused pane; when
    /// it is open there already, the field takes the focus with its text selected.
    pub(super) fn open_find(&mut self) {
        let focused = self.tabs.active().focused;
        // No room for the bar (it would only swallow keys).
        let fits = self.pane_rect(focused).is_some_and(|r| {
            findbar_layout(r, &self.bar_metrics(), self.overlay_inset())
                .rect
                .w
                > 0
        });
        if !fits {
            return;
        }
        match &mut self.find {
            Some(f) if f.pane == focused => f.bar.select_all = !f.bar.field.text().is_empty(),
            _ => self.find = Some(FindState::new(focused)),
        }
        self.schedule();
    }

    /// Paste `kind` into the field (its first line) and search for it.
    pub(super) fn find_paste(&mut self, kind: ClipKind) {
        let Some(text) = self.clipboards.load(kind) else {
            return;
        };
        if self.find.as_mut().is_some_and(|f| f.bar.paste(&text)) {
            self.find_requery();
        }
    }

    /// `pane` was resized: its current match no longer lines up, and a pane left with
    /// no room for the bar closes it. (Called after the layout settled.)
    pub(super) fn find_resized(&mut self, pane: PaneId) {
        if let Some(f) = self.find.as_mut().filter(|f| f.pane == pane) {
            f.current = None;
        }
        if self.find.is_some() && self.find_layout().is_none_or(|l| l.rect.w == 0) {
            self.close_find();
        }
    }

    /// Close the bar: the highlights go, the scroll position stays.
    pub(super) fn close_find(&mut self) {
        if self.find.take().is_some() {
            self.schedule();
        }
    }

    /// The next match, toward newer text.
    pub(super) fn find_next(&mut self) {
        self.find_step(Direction::Right);
    }

    /// The previous match, toward older text.
    pub(super) fn find_prev(&mut self) {
        self.find_step(Direction::Left);
    }

    fn find_step(&mut self, dir: Direction) {
        let Some(f) = self.find.as_mut() else {
            return;
        };
        let id = f.pane;
        if let Some(pane) = self.panes.get(&id) {
            let mut term = pane.term.lock();
            let before = term.grid().display_offset();
            f.search(&mut term, dir);
            let moved = term.grid().display_offset() != before;
            drop(term);
            if moved {
                self.mark_scrolled(id);
            }
            self.schedule();
        }
    }

    /// The query or the case flag changed: search again from the bottom.
    fn find_requery(&mut self) {
        let Some(f) = self.find.as_mut() else {
            return;
        };
        let id = f.pane;
        if let Some(pane) = self.panes.get(&id) {
            let mut term = pane.term.lock();
            let before = term.grid().display_offset();
            f.query_changed(&mut term);
            let moved = term.grid().display_offset() != before;
            drop(term);
            if moved {
                self.mark_scrolled(id);
            }
        }
        self.schedule();
    }

    /// A key press while the bar is open: the few bindings that make sense with it run;
    /// everything else goes to the field.
    pub(super) fn find_key(&mut self, event: &KeyEvent) {
        let mods = Mods::from(self.modifiers);
        let bound = self.store.global().bindings.resolve(
            &event.key_without_modifiers(),
            event.physical_key,
            mods,
        );
        if let Some(action) = bound.filter(|&a| runs_from_bar(a)) {
            self.stamp_key();
            self.perform(action);
            return;
        }
        let Some(f) = self.find.as_mut() else {
            return;
        };
        let out = f.bar.key(&event.logical_key, event.text.as_deref(), mods);
        self.stamp_key();
        match out {
            FindKey::Nothing => {}
            FindKey::Redraw => self.schedule(),
            FindKey::Edited => self.find_requery(),
            FindKey::Next => self.find_step(Direction::Right),
            FindKey::Prev => self.find_step(Direction::Left),
            FindKey::Close => self.close_find(),
        }
    }

    /// A press while the bar is open. Its buttons act (left button) and any press on
    /// the bar is consumed, so the pane behind it never sees it; a press on any other
    /// part of a pane closes the bar and carries on. True when consumed.
    pub(super) fn find_press(&mut self, button: MouseButton) -> bool {
        let (x, y) = self.mouse.pos();
        let hit = self.find_layout().and_then(|l| findbar_hit(&l, x, y));
        let Some(target) = hit else {
            if self.find.is_some() && self.pane_at(x, y).is_some() {
                self.close_find();
            }
            return false;
        };
        self.mouse.clicks.clear();
        match bar_click(target, button) {
            Some(BarClick::Step(dir)) => self.find_step(dir),
            Some(BarClick::ToggleCase) => {
                if let Some(f) = self.find.as_mut() {
                    f.bar.match_case = !f.bar.match_case;
                }
                self.find_requery();
            }
            Some(BarClick::Close) => self.close_find(),
            Some(BarClick::PastePrimary) => self.find_paste(ClipKind::Primary),
            None => {}
        }
        true
    }

    /// Draw the bar over its pane.
    pub(super) fn draw_find(&mut self) {
        let (Some(l), Some(f)) = (self.find_layout(), self.find.as_ref()) else {
            return;
        };
        let p = paint_find(&l, &f.bar, &self.bar_metrics(), self.chrome_stroke());
        self.draw_paint(&p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::textfield::TextField;
    use crate::ui::theme::{ACCENT, CLOSE_HOVER, OVERLAY_BG, OVERLAY_BORDER, TEXT, to_rgb};
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::Config;
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

    fn bm() -> BarMetrics {
        BarMetrics {
            height: 28,
            pad: 6,
            cell_w: 8,
            cell_h: 16,
        }
    }

    fn pane() -> Rect {
        Rect {
            x: 0,
            y: 28,
            w: 800,
            h: 300,
        }
    }

    fn bar(text: &str) -> FindBar {
        FindBar {
            field: TextField::new(text),
            ..FindBar::new()
        }
    }

    fn paint(b: &FindBar, w: i32) -> BarPaint {
        let l = findbar_layout(Rect { w, ..pane() }, &bm(), 4);
        paint_find(&l, b, &bm(), 1)
    }

    fn fills(p: &BarPaint, color: [f32; 4]) -> usize {
        p.under.iter().filter(|r| r.color == color).count()
    }

    fn term(bytes: &[u8]) -> Term<VoidListener> {
        let mut t = Term::new(Config::default(), &TermSize::new(10, 3), VoidListener);
        Processor::<StdSyncHandler>::new().advance(&mut t, bytes);
        t
    }

    fn found(text: &str, t: &mut Term<VoidListener>) -> FindState {
        let mut f = FindState::new(1);
        f.bar = bar(text);
        f.query_changed(t);
        f
    }

    #[test]
    fn bar_clicks_map_to_actions() {
        use FindTarget::*;
        let left = |t| bar_click(t, MouseButton::Left);
        assert_eq!(left(Prev), Some(BarClick::Step(Direction::Left)));
        assert_eq!(left(Next), Some(BarClick::Step(Direction::Right)));
        assert_eq!(left(Case), Some(BarClick::ToggleCase));
        assert_eq!(left(Close), Some(BarClick::Close));
        assert_eq!(left(Field), None);
        // Middle-click anywhere on the bar pastes the primary selection into the field.
        for t in [Field, Prev, Next, Case, Close] {
            assert_eq!(
                bar_click(t, MouseButton::Middle),
                Some(BarClick::PastePrimary)
            );
        }
        assert_eq!(bar_click(Field, MouseButton::Right), None);
    }

    #[test]
    fn current_match_follows_output_or_is_dropped() {
        let mut t = term(b"foo\r\nbar");
        let mut f = found("foo", &mut t);
        let m = f.current.clone().unwrap();
        // Nothing moved: still current.
        f.revalidate(&t);
        assert_eq!(f.current, Some(m));
        // Two more lines scroll "foo" into history: the old coordinates hold other text.
        Processor::<StdSyncHandler>::new().advance(&mut t, b"\r\nbaz\r\nqux");
        f.revalidate(&t);
        assert_eq!(f.current, None);
        // The next step searches afresh from the bottom and finds it again.
        f.search(&mut t, Direction::Right);
        assert_eq!(f.current.as_ref().unwrap().start().line, Line(-1));
    }

    #[test]
    fn a_match_that_stays_put_survives_output_below() {
        let mut t = term(b"foo");
        let mut f = found("foo", &mut t);
        let m = f.current.clone();
        Processor::<StdSyncHandler>::new().advance(&mut t, b"\r\nx\r\ny");
        f.revalidate(&t);
        assert_eq!(f.current, m);
    }

    #[test]
    fn visible_matches_on_a_huge_wrapped_line_are_bounded() {
        let mut bytes = vec![b'x'; 2990];
        bytes.extend_from_slice(b"foo       ");
        let mut t = term(&bytes);
        let mut re = RegexSearch::new("foo").unwrap();
        assert_eq!(visible_matches(&t, &mut re).len(), 1);
        // Scrolled far up, the top row's wrapped line is capped, and nothing hangs.
        t.scroll_display(alacritty_terminal::grid::Scroll::Top);
        assert!(visible_matches(&t, &mut re).is_empty());
    }

    #[test]
    fn only_find_and_tab_bindings_run_from_the_bar() {
        for a in [
            Action::Find,
            Action::FindNext,
            Action::FindPrev,
            Action::NewTab,
            Action::CloseTab,
            Action::NextTab,
            Action::PrevTab,
            Action::Copy,
            Action::Paste,
            Action::PastePrimary,
        ] {
            assert!(runs_from_bar(a), "{a:?}");
        }
        for a in [Action::SplitRight, Action::SelectAll] {
            assert!(!runs_from_bar(a), "{a:?}");
        }
    }

    #[test]
    fn panel_field_text_and_beam() {
        let p = paint(&bar("abc"), 800);
        assert_eq!(fills(&p, OVERLAY_BG), 1);
        assert_eq!(fills(&p, OVERLAY_BORDER), 4);
        // The field's ACCENT border: one framed rect, no Aa fill.
        assert_eq!(fills(&p, ACCENT), 1);
        let text = p.labels.iter().find(|l| l.text == "abc").unwrap();
        assert_eq!(text.color, to_rgb(TEXT));
        let beam = p.icons.iter().find(|i| i.color == TEXT).unwrap();
        assert_eq!(beam.rect.x, text.x + 3 * 8);
        assert_eq!((beam.rect.y, beam.rect.h), (text.y, 16));
        assert!(p.labels.iter().all(|l| l.text != NO_MATCH));
    }

    #[test]
    fn buttons_have_arrows_case_and_close() {
        let mut b = bar("a");
        let p = paint(&b, 800);
        let texts: Vec<_> = p.labels.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["a", "\u{2191}", "\u{2193}", "Aa"]);
        assert_eq!(fills(&p, ACCENT), 1);
        // The close icon is drawn in ICON.
        assert!(p.icons.iter().any(|i| i.color == theme::ICON));
        // Aa on: filled with ACCENT.
        b.match_case = true;
        assert_eq!(fills(&paint(&b, 800), ACCENT), 2);
    }

    #[test]
    fn no_matches_label_is_red_and_only_when_it_fits() {
        let mut b = bar("zzz");
        b.no_match = true;
        let p = paint(&b, 800);
        let l = p.labels.iter().find(|l| l.text == NO_MATCH).unwrap();
        assert_eq!(l.color, to_rgb(CLOSE_HOVER));
        // The query is clipped before the label.
        let q = p.labels.iter().find(|l| l.text == "zzz").unwrap();
        assert!(q.clip.x + q.clip.w < l.x);
        // Little room: the label is dropped, the query keeps its space.
        let p = paint(&b, 190);
        assert!(p.labels.iter().all(|l| l.text != NO_MATCH));
    }

    #[test]
    fn selected_text_is_highlighted_without_a_beam() {
        let mut b = bar("abc");
        b.select_all = true;
        let p = paint(&b, 800);
        // Border and the selection.
        assert_eq!(fills(&p, ACCENT), 2);
        assert!(p.icons.iter().all(|i| i.color != TEXT));
        let sel = p.under.iter().rfind(|r| r.color == ACCENT).unwrap();
        assert_eq!(sel.rect.w, 3 * 8);
    }

    #[test]
    fn tiny_pane_paints_nothing() {
        let l = findbar_layout(Rect { w: 6, ..pane() }, &bm(), 4);
        let p = paint_find(&l, &bar("x"), &bm(), 1);
        assert!(p.under.is_empty() && p.labels.is_empty() && p.icons.is_empty());
    }
}
