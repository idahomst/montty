//! Configuration files: the config directory, error type and color strings shared by
//! `config.toml`, profiles and schemes.

pub mod global;
pub mod profile;
pub mod scheme;
pub mod store;
pub mod watch;

use std::fmt;
use std::path::{Path, PathBuf};

use crate::term::colors::Rgb;

pub use global::GlobalConfig;

/// `$XDG_CONFIG_HOME/montty`, or `$HOME/.config/montty` when the variable is unset, empty
/// or not an absolute path (the XDG spec says to ignore relative values). A relative
/// `HOME` is unusable too.
pub fn config_dir_from(xdg: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    let absolute = |s: &&str| Path::new(s).is_absolute();
    match xdg.filter(absolute) {
        Some(x) => Some(PathBuf::from(x).join("montty")),
        None => home
            .filter(absolute)
            .map(|h| PathBuf::from(h).join(".config").join("montty")),
    }
}

/// The config directory for the current environment.
pub fn config_dir() -> Option<PathBuf> {
    config_dir_from(
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

/// A problem in one config file.
#[derive(Clone, Debug, PartialEq)]
pub struct ConfigError {
    /// Relative to the config directory.
    pub file: PathBuf,
    pub line: Option<usize>,
    pub msg: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "{}:{line}: {}", self.file.display(), self.msg),
            None => write!(f, "{}: {}", self.file.display(), self.msg),
        }
    }
}

impl std::error::Error for ConfigError {}

/// `#rgb`, `#rrggbb`, `#rrggbbaa` (alpha ignored) or `rgb(r,g,b)`, case-insensitive,
/// surrounding whitespace allowed.
pub fn parse_color(s: &str) -> Option<Rgb> {
    let s = s.trim().to_ascii_lowercase();
    if let Some(hex) = s.strip_prefix('#') {
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let digit = |i: usize| u8::from_str_radix(&hex[i..=i], 16).ok();
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        return match hex.len() {
            3 => Some(Rgb {
                r: digit(0)? * 17,
                g: digit(1)? * 17,
                b: digit(2)? * 17,
            }),
            6 | 8 => Some(Rgb {
                r: byte(0)?,
                g: byte(2)?,
                b: byte(4)?,
            }),
            _ => None,
        };
    }
    let inner = s.strip_prefix("rgb(")?.strip_suffix(')')?;
    let mut parts = inner.split(',').map(|p| p.trim().parse::<u8>().ok());
    let (r, g, b) = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(Rgb { r, g, b })
}

/// serde's ``unknown field `x`, expected one of …`` as ``unknown key `x` ``: the docs list the
/// keys, and the long list would push the rest of the toast off the screen. Other messages
/// pass through.
pub(crate) fn short_unknown_key(msg: &str) -> String {
    msg.strip_prefix("unknown field `")
        .and_then(|rest| rest.split_once('`'))
        .map_or_else(
            || msg.to_string(),
            |(key, _)| format!("unknown key `{key}`"),
        )
}

/// The 1-based line containing byte offset `byte` of `src`.
pub(crate) fn line_of(src: &str, byte: usize) -> usize {
    let end = byte.min(src.len());
    src.as_bytes()[..end]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
        + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(r: u8, g: u8, b: u8) -> Rgb {
        Rgb { r, g, b }
    }

    #[test]
    fn config_dir_rules() {
        assert_eq!(
            config_dir_from(Some("/x"), Some("/h")),
            Some(PathBuf::from("/x/montty"))
        );
        assert_eq!(
            config_dir_from(Some(""), Some("/h")),
            Some(PathBuf::from("/h/.config/montty"))
        );
        assert_eq!(
            config_dir_from(None, Some("/h")),
            Some(PathBuf::from("/h/.config/montty"))
        );
        assert_eq!(config_dir_from(None, None), None);
        // XDG: a relative XDG_CONFIG_HOME is ignored; a relative HOME is unusable.
        assert_eq!(
            config_dir_from(Some("rel/x"), Some("/h")),
            Some(PathBuf::from("/h/.config/montty"))
        );
        assert_eq!(config_dir_from(None, Some("rel")), None);
    }

    #[test]
    fn parse_color_forms() {
        for s in [
            "#4a90d9",
            "#4A90D9",
            " #4a90d9 ",
            "#4a90d9ff",
            "rgb(74,144,217)",
            "RGB( 74, 144 ,217 )",
        ] {
            assert_eq!(parse_color(s), Some(rgb(74, 144, 217)), "{s}");
        }
        assert_eq!(parse_color("#fff"), Some(rgb(255, 255, 255)));
        for s in [
            "blue",
            "#12345",
            "rgb(300,0,0)",
            "#ggg",
            "rgb(1,2)",
            "rgb(1,2,3,4)",
            "#+12",
            "",
            "#",
            "#é12",
        ] {
            assert_eq!(parse_color(s), None, "{s}");
        }
    }

    #[test]
    fn line_of_counts_newlines() {
        assert_eq!(line_of("a\nb\nc", 0), 1);
        assert_eq!(line_of("a\nb\nc", 2), 2);
        assert_eq!(line_of("a\nb\nc", 4), 3);
        assert_eq!(line_of("a", 99), 1);
    }

    #[test]
    fn error_display() {
        let mut e = ConfigError {
            file: "profiles/a.toml".into(),
            line: Some(12),
            msg: "x".into(),
        };
        assert_eq!(e.to_string(), "profiles/a.toml:12: x");
        e.line = None;
        assert_eq!(e.to_string(), "profiles/a.toml: x");
    }
}
