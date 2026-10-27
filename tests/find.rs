//! Find bar search against a real `Term` fed by `Processor`.

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Direction, Line, Point};
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use montty::ui::findbar::FindBar;
use montty::ui::textfield::TextField;
use montty::window::find::{FindState, visible_matches};

const LINES: usize = 10;

/// 10 screen lines and 200 lines of history, line `n` reading `line n` (`needle` marks
/// the ones in `needles`, `NEEDLE` those in `upper`); the last line is on the screen.
fn term_with(needles: &[usize], upper: &[usize]) -> Term<VoidListener> {
    let config = Config {
        scrolling_history: 1000,
        ..Config::default()
    };
    let mut term = Term::new(config, &TermSize::new(40, LINES), VoidListener);
    let mut p = Processor::<StdSyncHandler>::new();
    for n in 0..(200 + LINES) {
        let word = if needles.contains(&n) {
            "needle"
        } else if upper.contains(&n) {
            "NEEDLE"
        } else {
            "hay"
        };
        p.advance(&mut term, format!("line {n} {word}\r\n").as_bytes());
    }
    term
}

fn state(query: &str, term: &mut Term<VoidListener>) -> FindState {
    let mut s = FindState::new(1);
    s.bar = FindBar {
        field: TextField::new(query),
        ..FindBar::new()
    };
    s.query_changed(term);
    s
}

fn row_of(term: &Term<VoidListener>, line: Line) -> String {
    (0..term.columns())
        .map(|c| term.grid()[Point::new(line, Column(c))].c)
        .collect::<String>()
}

fn current_line(s: &FindState, term: &Term<VoidListener>) -> usize {
    let m = s.current.as_ref().expect("a current match");
    let text = row_of(term, m.start().line);
    text.split_whitespace().nth(1).unwrap().parse().unwrap()
}

#[test]
fn find_newest_first() {
    let mut term = term_with(&[10, 150], &[]);
    let s = state("needle", &mut term);
    assert!(!s.bar.no_match);
    assert_eq!(current_line(&s, &term), 150);
    // It is scrolled into view.
    let off = term.grid().display_offset() as i32;
    let line = s.current.as_ref().unwrap().start().line.0;
    assert!(line >= -off && line < -off + LINES as i32);
}

#[test]
fn find_wraps_around() {
    let mut term = term_with(&[10, 150], &[]);
    let mut s = state("needle", &mut term);
    assert_eq!(current_line(&s, &term), 150);
    s.search(&mut term, Direction::Left);
    assert_eq!(current_line(&s, &term), 10);
    // Up from the oldest wraps to the newest.
    s.search(&mut term, Direction::Left);
    assert_eq!(current_line(&s, &term), 150);
    // Down from the newest wraps to the oldest.
    s.search(&mut term, Direction::Right);
    assert_eq!(current_line(&s, &term), 10);
    s.search(&mut term, Direction::Right);
    assert_eq!(current_line(&s, &term), 150);
}

#[test]
fn find_case_toggle() {
    let mut term = term_with(&[50], &[100]);
    let mut s = state("needle", &mut term);
    // Case-insensitive: both match, the newest first.
    assert_eq!(current_line(&s, &term), 100);
    s.search(&mut term, Direction::Left);
    assert_eq!(current_line(&s, &term), 50);
    // Case-sensitive: only the lowercase one.
    s.bar.match_case = true;
    s.query_changed(&mut term);
    assert_eq!(current_line(&s, &term), 50);
    s.search(&mut term, Direction::Left);
    assert_eq!(current_line(&s, &term), 50);
    s.bar.field = TextField::new("NEEDLE");
    s.query_changed(&mut term);
    assert_eq!(current_line(&s, &term), 100);
}

#[test]
fn find_regex_metachars_are_literal() {
    let config = Config::default();
    let mut term = Term::new(config, &TermSize::new(40, LINES), VoidListener);
    Processor::<StdSyncHandler>::new().advance(&mut term, b"axb\r\na.b\r\n(a)\r\n");
    let s = state("a.b", &mut term);
    let m = s.current.as_ref().expect("a.b matches");
    assert_eq!(row_of(&term, m.start().line).trim_end(), "a.b");
    let s = state("(a)", &mut term);
    assert!(s.current.is_some());
    let s = state("a.*b", &mut term);
    assert!(s.current.is_none() && s.bar.no_match);
}

#[test]
fn find_no_match_keeps_scroll() {
    let mut term = term_with(&[10], &[]);
    term.scroll_display(Scroll::Delta(30));
    let before = term.grid().display_offset();
    assert_eq!(before, 30);
    let mut s = state("absent", &mut term);
    assert!(s.bar.no_match);
    assert!(s.current.is_none());
    assert_eq!(term.grid().display_offset(), before);
    s.search(&mut term, Direction::Right);
    assert!(s.bar.no_match);
    assert_eq!(term.grid().display_offset(), before);
}

#[test]
fn empty_query_clears_everything() {
    let mut term = term_with(&[10], &[]);
    let mut s = state("needle", &mut term);
    assert!(s.current.is_some());
    s.bar.field = TextField::new("");
    s.query_changed(&mut term);
    assert!(s.current.is_none() && !s.bar.no_match && s.regex.is_none());
}

#[test]
fn visible_matches_are_sorted_and_only_the_screen() {
    let mut term = term_with(&[10, 205, 208], &[]);
    let mut s = state("needle", &mut term);
    assert_eq!(current_line(&s, &term), 208);
    let matches = visible_matches(&term, s.regex.as_mut().unwrap());
    let lines: Vec<String> = matches
        .iter()
        .map(|m| row_of(&term, m.start().line))
        .collect();
    assert_eq!(matches.len(), 2, "{lines:?}");
    assert!(matches[0].start() < matches[1].start());
    // Scrolled up to line 10's screen: just that one.
    s.search(&mut term, Direction::Left);
    s.search(&mut term, Direction::Left);
    assert_eq!(current_line(&s, &term), 10);
    let matches = visible_matches(&term, s.regex.as_mut().unwrap());
    assert_eq!(matches.len(), 1);
}

#[test]
fn a_query_that_does_not_compile_is_no_match() {
    let mut term = term_with(&[10], &[]);
    // Far beyond the regex engine's size limit.
    let s = state(&"a".repeat(2_000_000), &mut term);
    assert!(s.regex.is_none(), "expected the compile to fail");
    assert!(s.current.is_none() && s.bar.no_match);
}

#[test]
fn a_match_wrapping_into_the_top_line_is_visible() {
    let mut term = Term::new(Config::default(), &TermSize::new(40, LINES), VoidListener);
    let mut p = Processor::<StdSyncHandler>::new();
    // "needle" starts at column 38: "ne" ends the row, "edle" wraps onto the next.
    p.advance(
        &mut term,
        format!("{}needle\r\n", "x".repeat(38)).as_bytes(),
    );
    for _ in 0..8 {
        p.advance(&mut term, b"L\r\n");
    }
    // The wrapped line's first row is in history, its second row is the top of the screen.
    assert_eq!(row_of(&term, Line(0)).trim_end(), "edle");
    let mut s = state("needle", &mut term);
    s.current = None;
    let matches = visible_matches(&term, s.regex.as_mut().unwrap());
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].start().line, Line(-1));
    assert_eq!(matches[0].end().line, Line(0));
}

#[test]
fn a_stale_current_match_does_not_break_the_next_step() {
    let config = Config {
        scrolling_history: 1000,
        ..Config::default()
    };
    let mut term = Term::new(config, &TermSize::new(40, LINES), VoidListener);
    let mut p = Processor::<StdSyncHandler>::new();
    for n in 0..300 {
        let word = if n == 5 { "needle" } else { "hay" };
        p.advance(&mut term, format!("line {n} {word}\r\n").as_bytes());
    }
    let mut s = state("needle", &mut term);
    assert_eq!(current_line(&s, &term), 5);
    // The history is cut to almost nothing: the match is gone from the grid.
    term.set_options(Config {
        scrolling_history: 2,
        ..Config::default()
    });
    for dir in [Direction::Right, Direction::Left] {
        s.search(&mut term, dir);
        assert!(s.current.is_none() && s.bar.no_match);
    }
    // New output with a needle: a fresh search finds it.
    p.advance(&mut term, b"needle again\r\n");
    s.query_changed(&mut term);
    assert!(s.current.is_some());
}

#[test]
fn a_match_evicted_by_new_output_is_survived() {
    let config = Config {
        scrolling_history: 20,
        ..Config::default()
    };
    let mut term = Term::new(config, &TermSize::new(40, LINES), VoidListener);
    let mut p = Processor::<StdSyncHandler>::new();
    p.advance(&mut term, b"needle\r\n");
    for _ in 0..15 {
        p.advance(&mut term, b"hay\r\n");
    }
    let mut s = state("needle", &mut term);
    assert!(s.current.is_some());
    for _ in 0..100 {
        p.advance(&mut term, b"hay\r\n");
    }
    s.search(&mut term, Direction::Right);
    assert!(s.current.is_none() && s.bar.no_match);
}
