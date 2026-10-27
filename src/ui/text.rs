//! Pure text helpers for chrome labels (display width, truncation, cell layout).

use unicode_width::UnicodeWidthChar;

/// Drop control characters (escape sequences, newlines, ...).
pub fn sanitize(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

/// Terminal columns `s` occupies: the sum of per-char widths, the same model
/// `truncate` and `layout_cells` use (so a label measured here is exactly what is drawn).
/// Callers should `sanitize` first; control chars count as 0 and are not drawn.
pub fn display_width(s: &str) -> usize {
    s.chars().map(|c| c.width().unwrap_or(0)).sum()
}

/// Cut `s` to at most `max_cols` columns, ending in `…` when anything was cut.
pub fn truncate(s: &str, max_cols: usize) -> String {
    if display_width(s) <= max_cols {
        return s.to_string();
    }
    if max_cols == 0 {
        return String::new();
    }
    // Leave one column for the ellipsis.
    let budget = max_cols - 1;
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > budget {
            break;
        }
        used += w;
        out.push(c);
    }
    out.push('…');
    out
}

/// One (column, char, wide) per drawn char; zero-width chars are skipped.
pub fn layout_cells(s: &str) -> Vec<(u16, char, bool)> {
    let mut col = 0usize;
    let mut out = Vec::new();
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if w == 0 {
            continue;
        }
        out.push((col.min(u16::MAX as usize) as u16, c, w == 2));
        col += w;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_by_display_width() {
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("abc", 4), "abc");
        assert_eq!(truncate("中文字", 4), "中…");
        assert_eq!(truncate("x", 0), "");
        assert!(display_width(&truncate("😀😀😀😀", 5)) <= 5);
    }

    #[test]
    fn sanitize_drops_controls() {
        assert_eq!(sanitize("a\x1b[1mb\n"), "a[1mb");
    }

    #[test]
    fn layout_cells_wide() {
        assert_eq!(layout_cells("中a"), vec![(0, '中', true), (2, 'a', false)]);
    }

    #[test]
    fn display_width_counts_wide() {
        assert_eq!(display_width("中a"), 3);
    }

    const TRICKY: [&str; 4] = [
        "\u{2764}\u{fe0f}\u{2764}\u{fe0f}\u{2764}\u{fe0f}\u{2764}\u{fe0f}",
        "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}",
        "ab\u{4e2d}\u{6587}cd\u{5b57}",
        "\u{1b}x\u{1f600}",
    ];

    #[test]
    fn truncate_never_exceeds_max_cols() {
        for s in TRICKY {
            for n in 0..8 {
                let t = truncate(s, n);
                assert!(display_width(&t) <= n, "{s:?} n={n} -> {t:?}");
            }
        }
    }

    #[test]
    fn display_width_matches_layout_cells() {
        for s in TRICKY.into_iter().chain(["", "abc", "\u{4e2d}a"]) {
            let end = layout_cells(s)
                .last()
                .map_or(0, |&(col, _, wide)| col as usize + if wide { 2 } else { 1 });
            assert_eq!(display_width(s), end, "{s:?}");
        }
    }
}
