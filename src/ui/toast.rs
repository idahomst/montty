//! The toast: a one-line message at the bottom of the window (config errors, startup
//! problems), gone after 8 s or a click. Pure text, geometry and the rule for when a reload shows one.

use std::time::{Duration, Instant};

use super::text::{display_width, sanitize, truncate};
use super::topbar::BarMetrics;
use crate::config::ConfigError;
use crate::layout::Rect;

/// How long a toast stays up.
pub const TOAST_DURATION: Duration = Duration::from_secs(8);

/// Where a toast came from: a reload that fixes every config error hides only a
/// `ConfigReload` toast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastSource {
    /// Config file errors (at startup or after a reload).
    ConfigReload,
    /// A startup problem that a reload doesn't fix (can't write the default profile, no
    /// config dir).
    Startup,
    /// The config watcher couldn't start.
    Watcher,
    /// A profile's font family isn't installed (the pane uses monospace).
    Font,
    /// A profile's shell or command couldn't start (the pane runs a fallback shell), or no
    /// shell would start for a split or new tab. A reload doesn't fix it, so it doesn't hide it.
    Spawn,
    /// A profile's background image is missing or can't be decoded (the pane draws without).
    Image,
    /// Something that worked (`saved session 'x'`): neutral colors, not an error.
    Info,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    /// The message. Cut with an ellipsis when the toast is narrower than `text` + `tail`.
    pub text: String,
    /// A suffix that is never cut (` (+2 more)`, ` (using /bin/sh)`); empty for most toasts.
    pub tail: String,
    pub until: Instant,
    pub source: ToastSource,
}

/// ` (+N more)` for `total` messages shown as one (the first), or nothing for 0 or 1.
/// The one place that builds this suffix, for config and image errors.
pub fn more_suffix(total: usize) -> String {
    match total.saturating_sub(1) {
        0 => String::new(),
        more => format!(" (+{more} more)"),
    }
}

/// The toast's one-line label in `cols` columns: `text` cut with an ellipsis so that `tail`
/// stays whole. When `tail` alone is wider than `cols`, only the tail shows, without its
/// leading space and cut with an ellipsis.
pub fn label(text: &str, tail: &str, cols: usize) -> String {
    let tail_w = display_width(tail);
    if display_width(text) + tail_w <= cols {
        return format!("{text}{tail}");
    }
    if tail_w >= cols {
        return truncate(tail.trim_start(), cols);
    }
    format!("{}{tail}", truncate(text, cols - tail_w))
}

/// What a config reload does to the toast.
#[derive(Debug, PartialEq)]
pub enum ToastUpdate {
    Keep,
    Show(Toast),
    /// Every error is fixed: a config-error toast still up goes away.
    Hide,
}

impl Toast {
    /// `text` on one line, shown from `now` for `TOAST_DURATION`.
    pub fn new(text: &str, source: ToastSource, now: Instant) -> Toast {
        Toast::with_tail(text, "", source, now)
    }

    /// `text` plus a `tail` that truncation never cuts, on one line.
    pub fn with_tail(text: &str, tail: &str, source: ToastSource, now: Instant) -> Toast {
        let one_line = |s: &str| sanitize(&s.replace('\n', " "));
        Toast {
            text: one_line(text),
            tail: one_line(tail),
            until: now + TOAST_DURATION,
            source,
        }
    }

    /// `first` of `total` messages, plus ` (+N more)` for the rest.
    pub fn from_first(first: &str, total: usize, source: ToastSource, now: Instant) -> Toast {
        Toast::with_tail(first, &more_suffix(total), source, now)
    }

    /// The first error plus ` (+N more)`; None without errors.
    pub fn from_errors(errs: &[ConfigError], source: ToastSource, now: Instant) -> Option<Toast> {
        let first = errs.first()?;
        Some(Toast::from_first(
            &first.to_string(),
            errs.len(),
            source,
            now,
        ))
    }

    /// The whole message, tail included.
    pub fn full(&self) -> String {
        format!("{}{}", self.text, self.tail)
    }

    /// Bottom-centered, 2×pad above the window's bottom edge, one text row plus padding
    /// tall and at most 90 % of the window wide (the text is then truncated).
    pub fn rect(&self, win_w: i32, win_h: i32, bm: &BarMetrics) -> Rect {
        let text_w = (display_width(&self.text) + display_width(&self.tail)) as i32 * bm.cell_w;
        let w = (text_w + 2 * bm.pad).min(win_w * 9 / 10).max(0);
        let h = bm.cell_h + 2 * bm.pad;
        Rect {
            x: (win_w - w) / 2,
            y: win_h - 2 * bm.pad - h,
            w,
            h,
        }
    }
}

/// The toast after a reload that found `errors`, given the source of the toast up now
/// (`current`) and the errors `last` shown. A toast shows only when the errors differ from
/// the last shown ones, or when something changed or a file was saved (`changed`) and
/// errors remain: saving the same broken file again re-shows it, while an unrelated event
/// doesn't. With every error fixed
/// a config-error toast hides; other toasts stay until they expire or are clicked.
pub fn reload_toast(
    current: Option<ToastSource>,
    last: &[ConfigError],
    errors: &[ConfigError],
    changed: bool,
    now: Instant,
) -> ToastUpdate {
    if errors.is_empty() {
        return if current == Some(ToastSource::ConfigReload) {
            ToastUpdate::Hide
        } else {
            ToastUpdate::Keep
        };
    }
    if errors != last || changed {
        Toast::from_errors(errors, ToastSource::ConfigReload, now)
            .map_or(ToastUpdate::Keep, ToastUpdate::Show)
    } else {
        ToastUpdate::Keep
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(file: &str, line: Option<usize>, msg: &str) -> ConfigError {
        ConfigError {
            file: file.into(),
            line,
            msg: msg.into(),
        }
    }

    fn bm() -> BarMetrics {
        BarMetrics {
            height: 28,
            pad: 6,
            cell_w: 8,
            cell_h: 16,
        }
    }

    #[test]
    fn toast_text_and_expiry() {
        let now = Instant::now();
        assert_eq!(
            Toast::from_errors(&[], ToastSource::ConfigReload, now),
            None
        );
        let errs = [
            err(
                "config.toml",
                Some(1),
                "dim_amount must be between 0.0 and 1.0, got 9",
            ),
            err("profiles/a.toml", None, "x"),
            err("schemes/b.json", Some(3), "y"),
        ];
        let t = Toast::from_errors(&errs, ToastSource::ConfigReload, now).unwrap();
        assert_eq!(
            t.full(),
            "config.toml:1: dim_amount must be between 0.0 and 1.0, got 9 (+2 more)"
        );
        assert_eq!(t.tail, " (+2 more)");
        assert_eq!(t.until, now + Duration::from_secs(8));
        let t = Toast::from_errors(&errs[1..2], ToastSource::Startup, now).unwrap();
        assert_eq!(t.full(), "profiles/a.toml: x");
        assert_eq!(t.source, ToastSource::Startup);
        // One line: newlines in a message become spaces, other controls go.
        let t =
            Toast::from_errors(&[err("c", None, "a\nb\x1b")], ToastSource::Startup, now).unwrap();
        assert_eq!(t.full(), "c: a b");
    }

    #[test]
    fn label_keeps_the_tail_whole() {
        let body = "config.toml:1: unknown key `bogus`";
        // Fits: nothing is cut.
        assert_eq!(label(body, " (+2 more)", 80), format!("{body} (+2 more)"));
        assert_eq!(label(body, "", body.len()), body);
        // Too narrow: only the body is cut, the tail stays whole.
        let l = label(body, " (+2 more)", 30);
        assert_eq!(l.chars().count(), 30);
        assert!(l.ends_with("… (+2 more)"), "{l}");
        assert!(l.starts_with("config.toml"), "{l}");
        // No tail: the body alone is cut.
        let l = label(body, "", 10);
        assert_eq!(l.chars().count(), 10);
        assert!(l.ends_with('…'));
        // The tail alone doesn't fit: it is shown without its leading space, cut with an ellipsis.
        assert_eq!(label(body, " (+2 more)", 10), "(+2 more)");
        assert_eq!(label(body, " (+2 more)", 6), "(+2 m…");
        assert_eq!(label(body, " (+2 more)", 0), "");
        // The spawn toast's tail is protected too.
        let l = label(
            "profiles/a.toml: can't start 'zhs': No such file",
            " (using /bin/sh)",
            40,
        );
        assert!(l.ends_with("… (using /bin/sh)"), "{l}");
    }

    #[test]
    fn more_suffix_counts_the_rest() {
        assert_eq!(more_suffix(0), "");
        assert_eq!(more_suffix(1), "");
        assert_eq!(more_suffix(3), " (+2 more)");
        let t = Toast::from_first("e0", 3, ToastSource::Image, Instant::now());
        assert_eq!(t.full(), "e0 (+2 more)");
    }

    #[test]
    fn toast_rect_bottom_centered_clamped() {
        let now = Instant::now();
        // 10 columns: 80 px of text plus 6 px padding each side.
        let t = Toast::new("0123456789", ToastSource::Watcher, now);
        let r = t.rect(800, 600, &bm());
        assert_eq!(
            r,
            Rect {
                x: (800 - 92) / 2,
                y: 600 - 12 - 28,
                w: 92,
                h: 28
            }
        );
        // Too long: 90 % of the window.
        let t = Toast::new(&"x".repeat(200), ToastSource::Watcher, now);
        let r = t.rect(800, 600, &bm());
        assert_eq!((r.x, r.w), (40, 720));
        assert_eq!(r.y + r.h, 600 - 12);
        // A tiny window never gets a negative size.
        let r = t.rect(0, 0, &bm());
        assert!(r.w >= 0 && r.h >= 0);
    }

    #[test]
    fn reload_toast_rules() {
        use ToastSource::*;
        let now = Instant::now();
        let a = [err("config.toml", Some(1), "a")];
        let b = [err("config.toml", Some(2), "b")];
        let shown = |u| matches!(u, ToastUpdate::Show(_));
        // New errors show; the same errors again only when something changed.
        assert!(shown(reload_toast(None, &[], &a, false, now)));
        assert!(shown(reload_toast(Some(ConfigReload), &a, &b, false, now)));
        assert_eq!(
            reload_toast(Some(ConfigReload), &a, &a, false, now),
            ToastUpdate::Keep
        );
        assert!(shown(reload_toast(None, &a, &a, true, now)));
        // Fixed: a config-error toast hides.
        assert_eq!(
            reload_toast(Some(ConfigReload), &a, &[], true, now),
            ToastUpdate::Hide
        );
        assert_eq!(
            reload_toast(Some(ConfigReload), &a, &[], false, now),
            ToastUpdate::Hide
        );
        // Other toasts (can't write profiles/default.toml, no watcher) stay.
        for other in [Startup, Watcher] {
            assert_eq!(
                reload_toast(Some(other), &a, &[], true, now),
                ToastUpdate::Keep
            );
        }
        assert_eq!(reload_toast(None, &a, &[], true, now), ToastUpdate::Keep);
        // Nothing wrong before or after.
        assert_eq!(reload_toast(None, &[], &[], true, now), ToastUpdate::Keep);
        // A reload's toast is a ConfigReload toast.
        assert_eq!(
            reload_toast(Some(Startup), &[], &a, false, now),
            ToastUpdate::Show(Toast::from_errors(&a, ConfigReload, now).unwrap())
        );
    }
}
