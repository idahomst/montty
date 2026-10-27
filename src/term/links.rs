//! Link detection over a terminal grid: an OSC 8 hyperlink under a point, else a URL
//! found by regex on the (wrap-joined) line. Pure: no window state.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Boundary, Column, Direction, Line, Point};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};

/// Alacritty's default hint regex, plus `www.` (opened as `https://`; `trim` checks it
/// starts a word).
pub const URL_REGEX: &str = "(ipfs:|ipns:|magnet:|mailto:|gemini://|gopher://|https://|http://|news:|file:|git://|ssh:|ftp://)[^\u{0000}-\u{001F}\u{007F}-\u{009F}<>\"\\s{-}\\^⟨⟩`\\\\]+|(?i:www)\\.[^\u{0000}-\u{001F}\u{007F}-\u{009F}<>\"\\s{-}\\^⟨⟩`\\\\]+";

/// A link under the pointer: where it goes and the cells it covers (grid coordinates).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// Where it goes: for a `www.` link, with `https://` added.
    pub uri: String,
    /// The visible characters, joined across wrapped rows.
    pub text: String,
    pub range: Match,
}

/// The schemes `openable` allows (those `URL_REGEX` finds, plus OSC 8's `http` family).
const SCHEMES: &[&str] = &[
    "http", "https", "ftp", "file", "mailto", "gemini", "gopher", "git", "ssh", "news", "magnet",
    "ipfs", "ipns",
];

/// How far a scan follows a wrapped line from the point, in rows (Alacritty's
/// `MAX_SEARCH_LINES`): a huge unbroken line must not be walked under the grid lock.
pub const MAX_SEARCH_LINES: usize = 100;

/// `Term::line_search_left`, stopping at most `MAX_SEARCH_LINES` rows above `point`.
pub fn line_search_left_capped<T>(term: &Term<T>, point: Point) -> Point {
    let last = term.last_column();
    let wraps = |line: Line| term.grid()[line][last].flags.contains(Flags::WRAPLINE);
    let mut line: Line = point.line;
    for _ in 0..MAX_SEARCH_LINES {
        let wraps_in = line > term.topmost_line() && wraps(line - 1);
        if !wraps_in {
            break;
        }
        line -= 1;
    }
    Point::new(line, Column(0))
}

/// `Term::line_search_right`, stopping at most `MAX_SEARCH_LINES` rows below `point`.
pub fn line_search_right_capped<T>(term: &Term<T>, point: Point) -> Point {
    let last = term.last_column();
    let wraps = |line: Line| term.grid()[line][last].flags.contains(Flags::WRAPLINE);
    let mut line: Line = point.line;
    for _ in 0..MAX_SEARCH_LINES {
        let wraps_out = line < term.bottommost_line() && wraps(line);
        if !wraps_out {
            break;
        }
        line += 1;
    }
    Point::new(line, last)
}

/// The link at `point` (grid coordinates): an OSC 8 hyperlink on the cell wins, else the
/// URL the regex finds on the cell's line (across wrapped rows).
pub fn link_at<T>(term: &Term<T>, regex: &mut RegexSearch, point: Point) -> Option<Link> {
    if point.line < term.topmost_line() || point.line > term.bottommost_line() {
        return None;
    }
    if point.column >= term.columns() {
        return None;
    }
    match osc8_at(term, point) {
        // An OSC 8 link wins even when it can't be opened: no hover, no fallback to
        // a URL in its text.
        Some(link) => openable(&link.uri).then_some(link),
        None => url_at(term, regex, point),
    }
}

/// Whether `uri`'s scheme is on the allowlist (compared case-insensitively).
pub fn openable(uri: &str) -> bool {
    uri.split_once(':')
        .is_some_and(|(scheme, _)| SCHEMES.iter().any(|s| scheme.eq_ignore_ascii_case(s)))
}

fn osc8_at<T>(term: &Term<T>, point: Point) -> Option<Link> {
    let link = term.grid()[point].hyperlink()?;
    let same = |p: Point| term.grid()[p].hyperlink().as_ref() == Some(&link);
    let last = term.last_column();
    let wraps = |line: Line| term.grid()[line][last].flags.contains(Flags::WRAPLINE);
    let mut start = point;
    loop {
        let prev = if start.column.0 > 0 {
            Point::new(start.line, start.column - 1)
        } else if start.line > term.topmost_line() && wraps(start.line - 1) {
            Point::new(start.line - 1, last)
        } else {
            break;
        };
        if !same(prev) {
            break;
        }
        start = prev;
    }
    let mut end = point;
    loop {
        let next = if end.column < last {
            Point::new(end.line, end.column + 1)
        } else if end.line < term.bottommost_line() && wraps(end.line) {
            Point::new(end.line + 1, Column(0))
        } else {
            break;
        };
        if !same(next) {
            break;
        }
        end = next;
    }
    let range = start..=end;
    Some(Link {
        uri: link.uri().to_string(),
        text: chars_in(term, &range).into_iter().map(|(_, c)| c).collect(),
        range,
    })
}

fn url_at<T>(term: &Term<T>, regex: &mut RegexSearch, point: Point) -> Option<Link> {
    let start = line_search_left_capped(term, point);
    let end = line_search_right_capped(term, point);
    let found = RegexIter::new(start, end, Direction::Right, term, regex)
        .take_while(|m| *m.start() <= point)
        .find(|m| m.contains(&point))?;
    let link = trim(term, found)?;
    link.range.contains(&point).then_some(link)
}

/// The match's text and range without trailing punctuation or an unbalanced `)`.
fn trim<T>(term: &Term<T>, found: Match) -> Option<Link> {
    let mut cells = chars_in(term, &found);
    let www = starts_www(&cells);
    if www && !at_word_start(term, *found.start()) {
        return None;
    }
    let count = |cells: &[(Point, char)], c| cells.iter().filter(|(_, x)| *x == c).count();
    while let Some(&(_, c)) = cells.last() {
        let trim = matches!(c, '.' | ',' | ':' | ';' | '!' | '?' | '\'' | '"')
            || (c == ')' && count(&cells, ')') > count(&cells, '('));
        if !trim {
            break;
        }
        cells.pop();
    }
    let &(last, _) = cells.last()?;
    let end = if term.grid()[last].flags.contains(Flags::WIDE_CHAR) {
        last.add(term, Boundary::None, 1)
    } else {
        last
    };
    let text: String = cells.iter().map(|(_, c)| c).collect();
    let uri = if www {
        // Trimmed down to `www` (`www..`): no link.
        if !is_www(&text) {
            return None;
        }
        format!("https://{text}")
    } else {
        text.clone()
    };
    Some(Link {
        uri,
        text,
        range: *found.start()..=end,
    })
}

const WWW: &str = "www.";

fn is_www(text: &str) -> bool {
    text.get(..WWW.len())
        .is_some_and(|p| p.eq_ignore_ascii_case(WWW))
}

/// Whether the match is a `www.` one (not a scheme URL, which never starts with `www.`).
fn starts_www(cells: &[(Point, char)]) -> bool {
    let text: String = cells.iter().take(WWW.len()).map(|(_, c)| c).collect();
    is_www(&text)
}

/// Whether a `www.` at `start` begins a word: the character before it (on the same row,
/// or the last cell of the row above when that wraps) is not alphanumeric or one of `.-_/@`.
fn at_word_start<T>(term: &Term<T>, start: Point) -> bool {
    let last = term.last_column();
    let above: Line = start.line - 1;
    let prev = if start.column.0 > 0 {
        Point::new(start.line, start.column - 1)
    } else if start.line > term.topmost_line()
        && term.grid()[above][last].flags.contains(Flags::WRAPLINE)
    {
        Point::new(above, last)
    } else {
        return true;
    };
    let c = term.grid()[prev].c;
    !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '/' | '@'))
}

/// One entry per character in `range`: wide characters' spacer cells carry none.
fn chars_in<T>(term: &Term<T>, range: &Match) -> Vec<(Point, char)> {
    let mut cells = Vec::new();
    let mut p = *range.start();
    loop {
        let cell = &term.grid()[p];
        if !cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            cells.push((p, cell.c));
        }
        if p >= *range.end() {
            break;
        }
        p = p.add(term, Boundary::None, 1);
    }
    cells
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::index::{Column, Line};
    use alacritty_terminal::term::Config;
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

    fn term(cols: usize, bytes: &[u8]) -> Term<VoidListener> {
        let mut t = Term::new(Config::default(), &TermSize::new(cols, 4), VoidListener);
        Processor::<StdSyncHandler>::new().advance(&mut t, bytes);
        t
    }

    fn pt(line: i32, col: usize) -> Point {
        Point::new(Line(line), Column(col))
    }

    fn at(t: &Term<VoidListener>, p: Point) -> Option<Link> {
        link_at(t, &mut RegexSearch::new(URL_REGEX).unwrap(), p)
    }

    #[test]
    fn url_found_under_point() {
        let t = term(40, b"go https://a.b/c now");
        let l = at(&t, pt(0, 8)).unwrap();
        assert_eq!(l.uri, "https://a.b/c");
        assert_eq!(l.range, pt(0, 3)..=pt(0, 15));
        assert_eq!(at(&t, pt(0, 1)), None);
        assert_eq!(at(&t, pt(0, 17)), None);
    }

    #[test]
    fn url_across_wrapped_lines() {
        let t = term(10, b"x https://example.com/path end");
        // "x https://" / "example.co" / "m/path end": the point is on the second row.
        let l = at(&t, pt(1, 3)).unwrap();
        assert_eq!(l.uri, "https://example.com/path");
        assert_eq!(*l.range.start(), pt(0, 2));
        assert_eq!(*l.range.end(), pt(2, 5));
    }

    #[test]
    fn url_excludes_trailing_punctuation() {
        let t = term(40, b"see https://a.b/c.");
        let l = at(&t, pt(0, 6)).unwrap();
        assert_eq!(l.uri, "https://a.b/c");
        assert_eq!(*l.range.end(), pt(0, 16));
        let t = term(40, b"\"https://a.b/c\", ok?");
        assert_eq!(at(&t, pt(0, 6)).unwrap().uri, "https://a.b/c");
    }

    #[test]
    fn url_in_parens() {
        let t = term(40, b"(https://a.b/c)");
        assert_eq!(at(&t, pt(0, 5)).unwrap().uri, "https://a.b/c");
        let t = term(60, b"https://en.wikipedia.org/wiki/A_(b)");
        assert_eq!(
            at(&t, pt(0, 5)).unwrap().uri,
            "https://en.wikipedia.org/wiki/A_(b)"
        );
        let t = term(60, b"(see https://en.wikipedia.org/wiki/A_(b)).");
        assert_eq!(
            at(&t, pt(0, 8)).unwrap().uri,
            "https://en.wikipedia.org/wiki/A_(b)"
        );
    }

    #[test]
    fn osc8_link_wins() {
        let t = term(
            40,
            b"ab \x1b]8;;https://x.y\x1b\\text\x1b]8;;\x1b\\ https://a.b",
        );
        let l = at(&t, pt(0, 4)).unwrap();
        assert_eq!(l.uri, "https://x.y");
        assert_eq!(l.range, pt(0, 3)..=pt(0, 6));
        // The plain URL after it is still found by regex.
        assert_eq!(at(&t, pt(0, 10)).unwrap().uri, "https://a.b");
    }

    #[test]
    fn osc8_link_with_a_closed_scheme_is_no_link() {
        let t = term(
            40,
            b"ab \x1b]8;;javascript:x\x1b\\text\x1b]8;;\x1b\\ https://a.b",
        );
        assert_eq!(at(&t, pt(0, 4)), None);
        assert_eq!(at(&t, pt(0, 10)).unwrap().uri, "https://a.b");
    }

    /// 10 columns, 3000 characters with no newline: 300 wrapped rows, the URL on the
    /// last ones.
    fn long_line() -> Term<VoidListener> {
        let mut bytes = vec![b'x'; 2980];
        bytes.extend_from_slice(b" https://a.b/c  yyyy");
        term(10, &bytes)
    }

    #[test]
    fn wrapped_line_scans_stop_at_the_cap() {
        let t = long_line();
        let point = pt(2, 0);
        let left = line_search_left_capped(&t, point);
        assert_eq!(
            left,
            Point::new(Line(2 - MAX_SEARCH_LINES as i32), Column(0))
        );
        let right = line_search_right_capped(&t, left);
        assert_eq!(right.line, Line(left.line.0 + MAX_SEARCH_LINES as i32));
        // A short wrapped line is still taken whole.
        let t = term(10, b"x https://example.com/path end");
        assert_eq!(line_search_left_capped(&t, pt(2, 0)), pt(0, 0));
        assert_eq!(line_search_right_capped(&t, pt(0, 0)), pt(2, 9));
    }

    #[test]
    fn url_within_the_cap_is_found_on_a_huge_line() {
        let t = long_line();
        let l = at(&t, pt(2, 2)).unwrap();
        assert_eq!(l.uri, "https://a.b/c");
    }

    #[test]
    fn www_link_opens_as_https() {
        let t = term(40, b"go www.mst.cz now");
        let l = at(&t, pt(0, 5)).unwrap();
        assert_eq!(l.uri, "https://www.mst.cz");
        assert_eq!(l.text, "www.mst.cz");
        assert_eq!(l.range, pt(0, 3)..=pt(0, 12));
        assert_eq!(at(&t, pt(0, 13)), None);
        let t = term(40, b"WWW.Example.org");
        assert_eq!(at(&t, pt(0, 0)).unwrap().uri, "https://WWW.Example.org");
    }

    #[test]
    fn www_link_trims_trailing_punctuation_and_parens() {
        let t = term(40, b"see www.mst.cz.");
        let l = at(&t, pt(0, 5)).unwrap();
        assert_eq!(
            (l.uri.as_str(), l.text.as_str()),
            ("https://www.mst.cz", "www.mst.cz")
        );
        assert_eq!(*l.range.end(), pt(0, 13));
        let t = term(40, b"(www.mst.cz)");
        let l = at(&t, pt(0, 5)).unwrap();
        assert_eq!(l.uri, "https://www.mst.cz");
        assert_eq!(l.range, pt(0, 1)..=pt(0, 10));
        // Nothing after the prefix is no link.
        let t = term(40, b"www.. x");
        assert_eq!(at(&t, pt(0, 1)), None);
    }

    #[test]
    fn www_needs_a_word_start() {
        for text in [
            &b"foo.www.bar"[..],
            b"x/www.y",
            b"a@www.b",
            b"a-www.b",
            b"a_www.b",
            b"1www.b",
        ] {
            let t = term(40, text);
            let col = text.windows(3).position(|w| w == b"www").unwrap();
            assert_eq!(
                at(&t, pt(0, col + 1)),
                None,
                "{}",
                String::from_utf8_lossy(text)
            );
        }
        for text in [
            &b"\"www.b\""[..],
            b"'www.b'",
            b"<www.b>",
            b"[www.b]",
            b"www.b",
        ] {
            let t = term(40, text);
            let col = text.windows(3).position(|w| w == b"www").unwrap();
            assert!(
                at(&t, pt(0, col + 1)).is_some(),
                "{}",
                String::from_utf8_lossy(text)
            );
        }
    }

    #[test]
    fn www_word_start_looks_across_a_wrap() {
        // "abcdefgh" fills row 0 and wraps: the `www.` that starts row 1 follows `h`.
        let t = term(8, b"abcdefghwww.b.cz");
        assert_eq!(at(&t, pt(1, 1)), None);
        // A row that ends in a space wraps too, and a space is a word start.
        let t = term(8, b"abcdefg www.b.cz");
        let l = at(&t, pt(1, 1)).unwrap();
        assert_eq!(l.uri, "https://www.b.cz");
        assert_eq!(l.range, pt(1, 0)..=pt(1, 7));
    }

    #[test]
    fn www_link_across_wrapped_rows() {
        let t = term(10, b"x www.example.com/p end");
        let l = at(&t, pt(1, 3)).unwrap();
        assert_eq!(l.uri, "https://www.example.com/p");
        assert_eq!(l.text, "www.example.com/p");
        assert_eq!(*l.range.start(), pt(0, 2));
    }

    #[test]
    fn https_www_url_is_unchanged() {
        let t = term(40, b"go https://www.mst.cz now");
        let l = at(&t, pt(0, 12)).unwrap();
        assert_eq!(l.uri, "https://www.mst.cz");
        assert_eq!(l.text, "https://www.mst.cz");
        assert_eq!(*l.range.start(), pt(0, 3));
        assert_eq!(*l.range.end(), pt(0, 20));
    }

    #[test]
    fn no_bare_domains() {
        let t = term(40, b"mst.cz");
        assert_eq!(at(&t, pt(0, 1)), None);
    }

    #[test]
    fn osc8_text_is_the_visible_characters() {
        let t = term(
            40,
            b"ab \x1b]8;;https://x.y\x1b\\te\xe4\xb8\xadxt\x1b]8;;\x1b\\ z",
        );
        let l = at(&t, pt(0, 4)).unwrap();
        assert_eq!(l.text, "te\u{4e2d}xt");
        assert_eq!(l.uri, "https://x.y");
        let t = term(40, b"go https://a.b/c now");
        let l = at(&t, pt(0, 8)).unwrap();
        assert_eq!(l.text, l.uri);
    }

    #[test]
    fn unknown_scheme_not_opened() {
        assert!(!openable("javascript:x"));
        assert!(!openable("data:text/html,hi"));
        assert!(openable("https://a"));
        assert!(openable("mailto:a@b.c"));
        assert!(openable("file:///tmp/x"));
    }
}
