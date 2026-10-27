//! Copy of the visible cells of a terminal, taken under a brief lock.

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::cell::Flags;
pub use alacritty_terminal::term::search::Match;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor};

use super::colors::{CURRENT_BG, CURRENT_FG, MATCH_BG, MATCH_FG, Palette, Rgb, cell_colors};

#[derive(Clone, Copy, Debug)]
pub struct SnapCell {
    pub ch: char,
    pub fg: Rgb,
    pub bg: Rgb,
    pub flags: Flags,
    /// The bg is the default background (`NamedColor::Background`, not inverse), i.e.
    /// `Snapshot::bg`: the pane's bg rect already painted it.
    pub default_bg: bool,
    /// Underlined as the Ctrl-hovered link (`Highlights::link`).
    pub link: bool,
}

/// Search matches to color, in grid coordinates (the snapshot maps them to the view).
#[derive(Clone, Copy, Debug)]
pub struct Highlights<'a> {
    pub matches: &'a [Match],
    pub current: Option<&'a Match>,
    /// The Ctrl-hovered link: its cells are underlined.
    pub link: Option<&'a Match>,
}

impl Highlights<'_> {
    pub const NONE: Highlights<'static> = Highlights {
        matches: &[],
        current: None,
        link: None,
    };
}

impl Default for Highlights<'_> {
    fn default() -> Self {
        Self::NONE
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapCursor {
    pub col: usize,
    pub line: usize,
    pub shape: CursorShape,
}

#[derive(Default)]
pub struct Snapshot {
    pub cols: usize,
    pub lines: usize,
    /// Row-major, `lines * cols` entries.
    pub cells: Vec<SnapCell>,
    /// (cell index, combining characters) for cells that have any.
    pub zerowidth: Vec<(usize, Vec<char>)>,
    pub cursor: Option<SnapCursor>,
    /// The effective cursor style (profile default or DECSCUSR) blinks.
    pub blinking: bool,
    pub display_offset: usize,
    pub mode: TermMode,
    /// The effective default background: the OSC 11 override, else the palette's.
    pub bg: Rgb,
}

impl Snapshot {
    pub fn capture<T: EventListener>(
        &mut self,
        term: &Term<T>,
        p: &Palette,
        bold_is_bright: bool,
        hl: &Highlights<'_>,
    ) {
        let rc = term.renderable_content();
        self.cols = term.columns();
        self.lines = term.screen_lines();
        self.display_offset = rc.display_offset;
        self.mode = rc.mode;
        self.blinking = term.cursor_style().blinking;
        self.bg = rc.colors[NamedColor::Background].unwrap_or(p.bg);

        let blank = SnapCell {
            ch: ' ',
            fg: p.fg,
            bg: self.bg,
            flags: Flags::empty(),
            default_bg: true,
            link: false,
        };
        self.cells.clear();
        self.cells.resize(self.cols * self.lines, blank);
        self.zerowidth.clear();

        // Matches arrive sorted and cells are visited in grid order, so skip the matches
        // that ended before the current cell instead of scanning them all per cell.
        let mut next_match = 0;
        for item in rc.display_iter {
            while hl
                .matches
                .get(next_match)
                .is_some_and(|m| *m.end() < item.point)
            {
                next_match += 1;
            }
            let line = item.point.line.0 + rc.display_offset as i32;
            if line < 0 || line as usize >= self.lines || item.point.column.0 >= self.cols {
                continue;
            }
            let idx = line as usize * self.cols + item.point.column.0;
            let cell = item.cell;
            let (mut fg, mut bg) =
                cell_colors(cell.fg, cell.bg, cell.flags, p, rc.colors, bold_is_bright);
            let mut default_bg = cell.bg == Color::Named(NamedColor::Background)
                && !cell.flags.contains(Flags::INVERSE);
            // Priority: selection > current match > other matches.
            let selected = rc
                .selection
                .as_ref()
                .is_some_and(|r| r.contains_cell(&item, rc.cursor.point, rc.cursor.shape));
            if selected {
                // An unset selection color takes the other cell color (the swap).
                (fg, bg) = (p.selection_fg.unwrap_or(bg), p.selection_bg.unwrap_or(fg));
                default_bg = false;
            } else if hl.current.is_some_and(|m| m.contains(&item.point)) {
                (fg, bg, default_bg) = (CURRENT_FG, CURRENT_BG, false);
            } else if next_match < hl.matches.len() && hl.matches[next_match].contains(&item.point)
            {
                (fg, bg, default_bg) = (MATCH_FG, MATCH_BG, false);
            }
            self.cells[idx] = SnapCell {
                ch: cell.c,
                fg,
                bg,
                flags: cell.flags,
                default_bg,
                link: hl.link.is_some_and(|m| m.contains(&item.point)),
            };
            if let Some(zw) = cell.zerowidth() {
                self.zerowidth.push((idx, zw.to_vec()));
            }
        }

        let cur_line = rc.cursor.point.line.0 + rc.display_offset as i32;
        self.cursor = (rc.mode.contains(TermMode::SHOW_CURSOR)
            && cur_line >= 0
            && (cur_line as usize) < self.lines
            && rc.cursor.point.column.0 < self.cols)
            .then_some(SnapCursor {
                col: rc.cursor.point.column.0,
                line: cur_line as usize,
                shape: rc.cursor.shape,
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::colors::tango_default;
    use crate::term::colors::{CURRENT_BG, CURRENT_FG, MATCH_BG, MATCH_FG};
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::grid::Scroll;
    use alacritty_terminal::index::{Column, Line, Point, Side};
    use alacritty_terminal::selection::{Selection, SelectionType};
    use alacritty_terminal::term::Config;
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

    fn snap(bytes: &[u8]) -> (Snapshot, Palette) {
        let p = tango_default();
        let s = snap_with(bytes, &p, None, &Highlights::NONE);
        (s, p)
    }

    /// Captures after `bytes`, with an optional selection (start, end, end side) on Simple.
    fn snap_with(
        bytes: &[u8],
        p: &Palette,
        sel: Option<(Point, Point, Side)>,
        hl: &Highlights<'_>,
    ) -> Snapshot {
        let mut term = Term::new(Config::default(), &TermSize::new(10, 3), VoidListener);
        let mut proc = Processor::<StdSyncHandler>::new();
        proc.advance(&mut term, bytes);
        if let Some((a, b, side)) = sel {
            let mut sel = Selection::new(SelectionType::Simple, a, Side::Left);
            sel.update(b, side);
            term.selection = Some(sel);
        }
        let mut s = Snapshot::default();
        s.capture(&term, p, false, hl);
        s
    }

    fn pt(line: i32, col: usize) -> Point {
        Point::new(Line(line), Column(col))
    }

    #[test]
    fn link_cells_are_flagged_without_recoloring() {
        let p = tango_default();
        let link = pt(0, 1)..=pt(0, 2);
        let hl = Highlights {
            link: Some(&link),
            ..Highlights::NONE
        };
        let s = snap_with(b"abcd", &p, None, &hl);
        let flags: Vec<bool> = s.cells[..4].iter().map(|c| c.link).collect();
        assert_eq!(flags, [false, true, true, false]);
        assert_eq!((s.cells[1].fg, s.cells[1].bg), (p.fg, p.bg));
    }

    #[test]
    fn selected_cells_swap_colors_without_palette_selection() {
        let p = tango_default();
        let sel = Some((pt(0, 0), pt(0, 1), Side::Right));
        let s = snap_with(b"abc", &p, sel, &Highlights::NONE);
        for c in &s.cells[..2] {
            assert_eq!((c.fg, c.bg), (p.bg, p.fg));
        }
        assert_eq!((s.cells[2].fg, s.cells[2].bg), (p.fg, p.bg));
    }

    #[test]
    fn selected_cells_use_palette_selection_colors() {
        let mut p = tango_default();
        let (fg, bg) = (
            Rgb { r: 0, g: 0, b: 0 },
            Rgb {
                r: 0x72,
                g: 0x9f,
                b: 0xcf,
            },
        );
        p.selection_fg = Some(fg);
        p.selection_bg = Some(bg);
        let sel = Some((pt(0, 0), pt(0, 1), Side::Right));
        let s = snap_with(b"abc", &p, sel, &Highlights::NONE);
        assert_eq!((s.cells[0].fg, s.cells[0].bg), (fg, bg));
        assert_eq!((s.cells[1].fg, s.cells[1].bg), (fg, bg));
        assert_eq!((s.cells[2].fg, s.cells[2].bg), (p.fg, p.bg));
    }

    #[test]
    fn half_set_selection_colors_swap_the_unset_one() {
        let black = Rgb { r: 0, g: 0, b: 0 };
        let sel = Some((pt(0, 0), pt(0, 1), Side::Right));
        let mut p = tango_default();
        p.selection_bg = Some(black);
        let s = snap_with(b"abc", &p, sel, &Highlights::NONE);
        assert_eq!((s.cells[0].fg, s.cells[0].bg), (p.bg, black));
        let mut p = tango_default();
        p.selection_fg = Some(black);
        let s = snap_with(b"abc", &p, sel, &Highlights::NONE);
        assert_eq!((s.cells[0].fg, s.cells[0].bg), (black, p.fg));
    }

    #[test]
    fn every_cell_matched_by_adjacent_single_cell_ranges() {
        let p = tango_default();
        let ms: Vec<Match> = (0..3)
            .flat_map(|l| (0..10).map(move |c| pt(l, c)..=pt(l, c)))
            .collect();
        let hl = Highlights {
            matches: &ms,
            current: None,
            ..Highlights::NONE
        };
        let s = snap_with(b"", &p, None, &hl);
        assert!(s.cells.iter().all(|c| c.bg == MATCH_BG && !c.default_bg));
    }

    #[test]
    fn selected_default_bg_cells_are_opaque() {
        let p = tango_default();
        let sel = Some((pt(0, 0), pt(0, 5), Side::Right));
        let s = snap_with(b"abc", &p, sel, &Highlights::NONE);
        assert!(!s.cells[0].default_bg && !s.cells[4].default_bg);
        assert!(s.cells[6].default_bg);
    }

    #[test]
    fn matches_and_current_match_colored() {
        let p = tango_default();
        let ms = [pt(0, 0)..=pt(0, 1), pt(0, 3)..=pt(0, 4)];
        let hl = Highlights {
            matches: &ms,
            current: Some(&ms[1]),
            ..Highlights::NONE
        };
        let s = snap_with(b"ab ab", &p, None, &hl);
        for i in [0, 1] {
            assert_eq!((s.cells[i].fg, s.cells[i].bg), (MATCH_FG, MATCH_BG));
        }
        for i in [3, 4] {
            assert_eq!((s.cells[i].fg, s.cells[i].bg), (CURRENT_FG, CURRENT_BG));
        }
        assert_eq!(s.cells[2].bg, p.bg);
        assert!(!s.cells[0].default_bg && !s.cells[3].default_bg);
        // Selection beats current beats match.
        let sel = Some((pt(0, 1), pt(0, 3), Side::Right));
        let s = snap_with(b"ab ab", &p, sel, &hl);
        assert_eq!(s.cells[0].bg, MATCH_BG);
        for i in [1, 2, 3] {
            assert_eq!((s.cells[i].fg, s.cells[i].bg), (p.bg, p.fg));
        }
        assert_eq!(s.cells[4].bg, CURRENT_BG);
    }

    #[test]
    fn highlights_follow_display_offset() {
        let p = tango_default();
        let mut term = Term::new(Config::default(), &TermSize::new(10, 3), VoidListener);
        Processor::<StdSyncHandler>::new().advance(&mut term, b"one\r\ntwo\r\nthree\r\nfour");
        term.scroll_display(Scroll::Delta(1));
        let ms = [pt(-1, 0)..=pt(-1, 2)];
        let hl = Highlights {
            matches: &ms,
            current: None,
            ..Highlights::NONE
        };
        let mut s = Snapshot::default();
        s.capture(&term, &p, false, &hl);
        assert_eq!(s.display_offset, 1);
        assert_eq!(s.cells[0].ch, 'o');
        assert_eq!(s.cells[0].bg, MATCH_BG);
        assert_eq!(s.cells[2].bg, MATCH_BG);
        assert_eq!(s.cells[3].bg, p.bg);
        assert_eq!(s.cells[10].bg, p.bg);
    }

    #[test]
    fn chars_and_cursor() {
        let (s, _) = snap(b"ab");
        assert_eq!((s.cols, s.lines, s.cells.len()), (10, 3, 30));
        assert_eq!(s.cells[0].ch, 'a');
        assert_eq!(s.cells[1].ch, 'b');
        let c = s.cursor.unwrap();
        assert_eq!((c.col, c.line), (2, 0));
    }

    #[test]
    fn sgr_color_resolved() {
        let (s, p) = snap(b"\x1b[31mX");
        assert_eq!(s.cells[0].fg, p.ansi[1]);
    }

    #[test]
    fn zerowidth_collected() {
        let (s, _) = snap(b"e\xcc\x81");
        assert_eq!(s.zerowidth, vec![(0, vec!['\u{301}'])]);
    }

    #[test]
    fn hidden_cursor_is_none() {
        let (s, _) = snap(b"\x1b[?25l");
        assert!(s.cursor.is_none());
    }

    #[test]
    fn default_bg_flag() {
        // Default bg, indexed bg, named non-default bg, inverse default bg.
        let (s, p) = snap(b"a\x1b[48;5;100mb\x1b[41mc\x1b[0;7md\x1b[0me");
        let flags: Vec<bool> = s.cells[..5].iter().map(|c| c.default_bg).collect();
        assert_eq!(flags, [true, false, false, false, true]);
        // Blank cells past the text are default too.
        assert!(s.cells[9].default_bg && s.cells[29].default_bg);
        assert_eq!(s.bg, p.bg);
        // An OSC 11 override is still the default bg; the pane rect paints it.
        let (s, _) = snap(b"\x1b]11;#ff0000\x07a");
        assert!(s.cells[0].default_bg);
        assert_eq!(s.bg, Rgb { r: 255, g: 0, b: 0 });
    }

    #[test]
    fn capture_reuses_allocation() {
        let mut term = Term::new(Config::default(), &TermSize::new(10, 3), VoidListener);
        Processor::<StdSyncHandler>::new().advance(&mut term, b"hi");
        let p = tango_default();
        let mut s = Snapshot::default();
        s.capture(&term, &p, false, &Highlights::NONE);
        let (cap, ptr) = (s.cells.capacity(), s.cells.as_ptr());
        s.capture(&term, &p, false, &Highlights::NONE);
        assert_eq!(s.cells.capacity(), cap);
        assert_eq!(s.cells.as_ptr(), ptr);
    }
}
