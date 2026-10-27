//! Selections against a real `Term` fed by `Processor`.

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

fn term(cols: usize, lines: usize, scrollback: usize) -> Term<VoidListener> {
    let config = Config {
        scrolling_history: scrollback,
        ..Config::default()
    };
    Term::new(config, &TermSize::new(cols, lines), VoidListener)
}

fn feed(term: &mut Term<VoidListener>, bytes: &[u8]) {
    Processor::<StdSyncHandler>::new().advance(term, bytes);
}

fn pt(line: i32, col: usize) -> Point {
    Point::new(Line(line), Column(col))
}

#[test]
fn semantic_selection_selects_word() {
    let mut t = term(20, 3, 10);
    feed(&mut t, b"foo bar.baz qux");
    t.selection = Some(Selection::new(
        SelectionType::Semantic,
        pt(0, 5),
        Side::Left,
    ));
    assert_eq!(t.selection_to_string().as_deref(), Some("bar.baz"));
}

#[test]
fn line_selection_selects_wrapped_line() {
    let mut t = term(5, 3, 10);
    feed(&mut t, b"abcdefgh");
    t.selection = Some(Selection::new(SelectionType::Lines, pt(1, 0), Side::Left));
    // No newline inside the wrapped line; Lines selections end with one.
    assert_eq!(t.selection_to_string().as_deref(), Some("abcdefgh\n"));
}

#[test]
fn selection_survives_output_scroll() {
    let mut t = term(10, 3, 10);
    feed(&mut t, b"one\r\ntwo\r\nthree");
    let mut sel = Selection::new(SelectionType::Simple, pt(1, 0), Side::Left);
    sel.update(pt(1, 2), Side::Right);
    t.selection = Some(sel);
    let before = t.selection_to_string();
    assert_eq!(before.as_deref(), Some("two"));
    feed(&mut t, b"\r\n4\r\n5\r\n6\r\n7\r\n8");
    assert_eq!(t.selection_to_string(), before);
}

#[test]
fn selection_dropped_from_history_is_none() {
    let mut t = term(10, 3, 10);
    feed(&mut t, b"top");
    let mut sel = Selection::new(SelectionType::Simple, pt(0, 0), Side::Left);
    sel.update(pt(0, 2), Side::Right);
    t.selection = Some(sel);
    for i in 0..50 {
        feed(&mut t, format!("\r\nline {i}").as_bytes());
    }
    let s = t.selection_to_string();
    assert!(s.as_deref().is_none_or(str::is_empty), "{s:?}");
}
