//! Tilix color schemes (JSON): parsing and the built-in set embedded from `assets/schemes`.

use std::path::Path;

use serde::Deserialize;

use super::{ConfigError, line_of, parse_color};
use crate::term::colors::{Palette, tango_default};

/// A named palette.
#[derive(Clone, Debug, PartialEq)]
pub struct Scheme {
    pub name: String,
    pub palette: Palette,
}

/// The keys of a Tilix scheme file that montty reads; the rest are ignored.
#[derive(Deserialize)]
struct Raw {
    /// Anything but a string counts as missing.
    name: Option<serde_json::Value>,
    #[serde(rename = "foreground-color")]
    foreground: Option<String>,
    #[serde(rename = "background-color")]
    background: Option<String>,
    palette: Option<Vec<String>>,
    /// Real-world files write these as strings or null; anything but `true` counts as false.
    #[serde(rename = "use-cursor-color", default)]
    use_cursor: serde_json::Value,
    #[serde(rename = "cursor-background-color")]
    cursor_background: Option<String>,
    #[serde(rename = "use-highlight-color", default)]
    use_highlight: serde_json::Value,
    #[serde(rename = "highlight-foreground-color")]
    highlight_foreground: Option<String>,
    #[serde(rename = "highlight-background-color")]
    highlight_background: Option<String>,
}

impl Scheme {
    /// Parses a scheme file's text. `file` is used only in error messages.
    pub fn parse(src: &str, file: &Path) -> Result<Scheme, ConfigError> {
        let err = |line: Option<usize>, msg: String| ConfigError {
            file: file.to_path_buf(),
            line,
            msg,
        };
        let raw: Raw = serde_json::from_str(src)
            .map_err(|e| err(Some(e.line()).filter(|&l| l > 0), e.to_string()))?;
        let color = |key: &str, v: &str| {
            parse_color(v).ok_or_else(|| {
                let line = src.find(&format!("\"{v}\"")).map(|at| line_of(src, at));
                err(line, format!("invalid color '{v}' for '{key}'"))
            })
        };
        let opt = |key: &str, v: &Option<String>| v.as_deref().map(|v| color(key, v)).transpose();

        let mut palette = tango_default();
        if let Some(list) = &raw.palette {
            if list.len() != 16 {
                let line = src.find("\"palette\"").map(|at| line_of(src, at));
                return Err(err(
                    line,
                    format!("'palette' must have 16 entries, found {}", list.len()),
                ));
            }
            for (slot, c) in palette.ansi.iter_mut().zip(list) {
                *slot = color("palette", c)?;
            }
        }
        if let Some(c) = opt("foreground-color", &raw.foreground)? {
            palette.fg = c;
        }
        if let Some(c) = opt("background-color", &raw.background)? {
            palette.bg = c;
        }
        // The cursor follows the scheme's foreground unless the scheme sets its own.
        palette.cursor = palette.fg;
        if raw.use_cursor == true
            && let Some(c) = opt("cursor-background-color", &raw.cursor_background)?
        {
            palette.cursor = c;
        }
        if raw.use_highlight == true {
            palette.selection_fg = opt("highlight-foreground-color", &raw.highlight_foreground)?;
            palette.selection_bg = opt("highlight-background-color", &raw.highlight_background)?;
        }
        Ok(Scheme {
            name: match raw.name {
                Some(serde_json::Value::String(n)) => n,
                _ => file
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            },
            palette,
        })
    }
}

/// The Tilix schemes shipped with montty, as `(file stem, scheme)`.
pub fn builtin_schemes() -> Vec<(&'static str, Scheme)> {
    macro_rules! schemes {
        ($($stem:literal),* $(,)?) => {
            [$(($stem, include_str!(concat!("../../assets/schemes/", $stem, ".json")))),*]
        };
    }
    schemes![
        "base16-twilight-dark",
        "linux",
        "material",
        "monokai",
        "orchis",
        "solarized-dark",
        "solarized-light",
        "tango",
        "yaru",
    ]
    .into_iter()
    .map(|(stem, src)| {
        let file = Path::new("schemes").join(format!("{stem}.json"));
        let scheme = Scheme::parse(src, &file).expect("built-in scheme is valid");
        (stem, scheme)
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::colors::Rgb;

    fn rgb(r: u8, g: u8, b: u8) -> Rgb {
        Rgb { r, g, b }
    }

    fn p(src: &str) -> Result<Scheme, ConfigError> {
        Scheme::parse(src, Path::new("schemes/x.json"))
    }

    fn zeros(n: usize) -> String {
        vec!["\"#000000\""; n].join(",")
    }

    #[test]
    fn parses_all_builtin() {
        let all = builtin_schemes();
        assert_eq!(all.len(), 9);
        for (stem, s) in &all {
            assert!(!s.name.is_empty(), "{stem}");
        }
        let tango = &all.iter().find(|(s, _)| *s == "tango").unwrap().1;
        // tango.json has no foreground-color; the Tango fallback is #d3d7cf.
        assert_eq!(tango.palette.fg, rgb(0xd3, 0xd7, 0xcf));
        assert_eq!(tango.palette.ansi[1], rgb(0xcc, 0x00, 0x00));
    }

    #[test]
    fn missing_fields_fall_back() {
        let s = p(&format!(r#"{{"name":"x","palette":[{}]}}"#, zeros(16))).unwrap();
        let t = tango_default();
        assert_eq!(s.palette.fg, t.fg);
        assert_eq!(s.palette.bg, t.bg);
        assert_eq!(s.palette.cursor, t.cursor);
        assert_eq!(s.palette.ansi[1], rgb(0, 0, 0));
    }

    #[test]
    fn bad_palette_len_is_error() {
        let e = p(&format!(r#"{{"name":"x","palette":[{}]}}"#, zeros(15))).unwrap_err();
        assert!(e.msg.contains("palette"), "{e}");
        assert_eq!(e.file, Path::new("schemes/x.json"));
    }

    #[test]
    fn bad_json_has_line() {
        let e = p("{\n\"name\": 1,,\n}").unwrap_err();
        assert_eq!(e.line, Some(2));
    }

    #[test]
    fn cursor_color_only_when_used() {
        let base = |flag: &str| {
            format!(
                r##"{{"name":"x","foreground-color":"#102030","use-cursor-color":{flag},
                "cursor-background-color":"#ff0000","palette":[{}]}}"##,
                zeros(16)
            )
        };
        assert_eq!(
            p(&base("false")).unwrap().palette.cursor,
            rgb(0x10, 0x20, 0x30)
        );
        assert_eq!(p(&base("true")).unwrap().palette.cursor, rgb(0xff, 0, 0));
    }

    #[test]
    fn highlight_colors_only_when_used() {
        let base = |flag: &str| {
            format!(
                r##"{{"name":"x","use-highlight-color":{flag},
                "highlight-foreground-color":"#111111","highlight-background-color":"#222222",
                "palette":[{}]}}"##,
                zeros(16)
            )
        };
        let off = p(&base("false")).unwrap().palette;
        assert_eq!((off.selection_fg, off.selection_bg), (None, None));
        let on = p(&base("true")).unwrap().palette;
        assert_eq!(on.selection_fg, Some(rgb(0x11, 0x11, 0x11)));
        assert_eq!(on.selection_bg, Some(rgb(0x22, 0x22, 0x22)));
    }

    #[test]
    fn lenient_use_flags() {
        for v in [r#""true""#, "null", "1", "[]"] {
            let src = format!(
                r##"{{"name":5,"foreground-color":"#102030","use-cursor-color":{v},
                "cursor-background-color":"#ff0000","use-highlight-color":{v},
                "highlight-background-color":"#222222","palette":[{}]}}"##,
                zeros(16)
            );
            let s = p(&src).unwrap();
            assert_eq!(s.palette.cursor, rgb(0x10, 0x20, 0x30), "{v}");
            assert_eq!(s.palette.selection_bg, None, "{v}");
            assert_eq!(s.name, "x", "{v}");
        }
    }

    #[test]
    fn bad_palette_len_has_line() {
        let src = format!("{{\n\"name\":\"x\",\n\"palette\":[{}]}}", zeros(15));
        assert_eq!(p(&src).unwrap_err().line, Some(3));
    }

    #[test]
    fn bad_color_has_line() {
        let e = p("{\n\"name\":\"x\",\n\"foreground-color\":\"nope\"}").unwrap_err();
        assert_eq!(e.line, Some(3));
    }

    #[test]
    fn bad_color_names_key() {
        let e = p(r##"{"name":"x","foreground-color":"nope"}"##).unwrap_err();
        assert!(e.msg.contains("foreground-color"), "{e}");
    }
}
