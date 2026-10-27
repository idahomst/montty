//! Profiles (`profiles/<stem>.toml`): fonts, colors, cursor, shell and background image.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use toml::Spanned;

use super::{ConfigError, line_of, parse_color, short_unknown_key};
use crate::term::colors::{Palette, Rgb};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    Stretch,
    Fill,
    Center,
    Tile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShapeCfg {
    Block,
    Beam,
    Underline,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BackgroundImage {
    /// `~` already expanded; a relative path is resolved against the config dir by the window.
    pub path: PathBuf,
    pub fit: Fit,
    pub opacity: f32,
}

/// Per-profile color overrides applied on top of the scheme's palette.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColorOverrides {
    pub foreground: Option<Rgb>,
    pub background: Option<Rgb>,
    pub cursor: Option<Rgb>,
    pub selection_foreground: Option<Rgb>,
    pub selection_background: Option<Rgb>,
    pub palette: Option<[Rgb; 16]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    /// Display name; empty means "use the file stem".
    pub name: String,
    pub font_family: String,
    /// 4.0..=200.0
    pub font_size: f32,
    /// Scheme stem or name.
    pub scheme: String,
    pub colors: ColorOverrides,
    pub bold_is_bright: bool,
    /// Window opacity, 0.0..=1.0.
    pub opacity: f32,
    /// Pixels, 0..=100.
    pub padding: i32,
    /// Lines, 0..=1_000_000.
    pub scrollback: usize,
    pub cursor_shape: CursorShapeCfg,
    pub cursor_blink: bool,
    pub shell: Option<String>,
    pub login_shell: bool,
    pub command: Option<Vec<String>>,
    pub background_image: Option<BackgroundImage>,
}

impl Default for Profile {
    fn default() -> Self {
        Profile {
            name: String::new(),
            font_family: "monospace".into(),
            font_size: 11.0,
            scheme: "tango".into(),
            colors: ColorOverrides::default(),
            bold_is_bright: false,
            opacity: 1.0,
            padding: 4,
            scrollback: 10_000,
            cursor_shape: CursorShapeCfg::Block,
            cursor_blink: false,
            shell: None,
            login_shell: false,
            command: None,
            background_image: None,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawColors {
    foreground: Option<Spanned<String>>,
    background: Option<Spanned<String>>,
    cursor: Option<Spanned<String>>,
    selection_foreground: Option<Spanned<String>>,
    selection_background: Option<Spanned<String>>,
    palette: Option<Spanned<Vec<Spanned<String>>>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawImage {
    path: Option<Spanned<String>>,
    fit: Option<Spanned<String>>,
    opacity: Option<Spanned<f64>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    name: Option<String>,
    font: Option<String>,
    font_size: Option<Spanned<f64>>,
    scheme: Option<String>,
    colors: Option<RawColors>,
    bold_is_bright: Option<bool>,
    opacity: Option<Spanned<f64>>,
    padding: Option<Spanned<i64>>,
    scrollback: Option<Spanned<i64>>,
    cursor_shape: Option<Spanned<String>>,
    cursor_blink: Option<bool>,
    shell: Option<String>,
    login_shell: Option<bool>,
    command: Option<Spanned<Vec<String>>>,
    background_image: Option<Spanned<RawImage>>,
}

/// `~` or `~/rest` expanded with `home`; anything else (or no home) is left as written.
fn expand_home(path: &str, home: Option<&Path>) -> PathBuf {
    match (home, path) {
        (Some(h), "~") => h.to_path_buf(),
        (Some(h), p) if p.starts_with("~/") => h.join(p[2..].trim_start_matches('/')),
        _ => PathBuf::from(path),
    }
}

/// Rewrites toml's type errors to name the key (taken from the error line, before `=`).
fn friendly(src: &str, span: Option<std::ops::Range<usize>>, msg: &str) -> String {
    let msg = &short_unknown_key(msg);
    let Some(span) = span else {
        return msg.to_string();
    };
    let line_start = src[..span.start.min(src.len())]
        .rfind('\n')
        .map_or(0, |i| i + 1);
    let line = src[line_start..].lines().next().unwrap_or("");
    let Some((key, _)) = line.split_once('=') else {
        return msg.to_string();
    };
    let key = key.trim();
    let got = src.get(span.clone()).unwrap_or("").trim();
    if msg.contains("expected i64") {
        format!("{key}: expected an integer, got {got}")
    } else if msg.contains("expected f64") {
        format!("{key}: expected a number")
    } else if msg.contains("too large") || msg.contains("out of range") {
        format!("{key}: expected an integer")
    } else {
        msg.to_string()
    }
}

impl Profile {
    /// Parse `src`; `file` (relative to the config dir) names it in errors. `home` expands `~`.
    pub fn parse(src: &str, file: &Path, home: Option<&Path>) -> Result<Profile, ConfigError> {
        let err = |byte: Option<usize>, msg: String| ConfigError {
            file: file.to_path_buf(),
            line: byte.map(|b| line_of(src, b)),
            msg,
        };
        let raw: Raw = toml::from_str(src).map_err(|e| {
            let msg = friendly(src, e.span(), e.message());
            err(e.span().map(|s| s.start), msg)
        })?;
        let range = |v: &Spanned<f64>, key: &str, lo: f64, hi: f64| -> Result<f32, ConfigError> {
            let x = *v.get_ref();
            if (lo..=hi).contains(&x) {
                Ok(x as f32)
            } else {
                Err(err(
                    Some(v.span().start),
                    format!("{key} must be between {lo} and {hi}, got {x}"),
                ))
            }
        };
        let int_range = |v: &Spanned<i64>, key: &str, hi: i64| -> Result<i64, ConfigError> {
            let x = *v.get_ref();
            if (0..=hi).contains(&x) {
                Ok(x)
            } else {
                Err(err(
                    Some(v.span().start),
                    format!("{key} must be between 0 and {hi}, got {x}"),
                ))
            }
        };
        let color = |v: &Spanned<String>, key: &str| -> Result<Rgb, ConfigError> {
            parse_color(v.get_ref()).ok_or_else(|| {
                err(
                    Some(v.span().start),
                    format!("{key}: invalid color {:?}", v.get_ref()),
                )
            })
        };

        let mut p = Profile::default();
        if let Some(n) = raw.name {
            p.name = n;
        }
        if let Some(f) = raw.font {
            p.font_family = f;
        }
        if let Some(s) = &raw.font_size {
            p.font_size = range(s, "font_size", 4.0, 200.0)?;
        }
        if let Some(s) = raw.scheme {
            p.scheme = s;
        }
        if let Some(c) = &raw.colors {
            let o = &mut p.colors;
            for (field, key, slot) in [
                (&c.foreground, "colors.foreground", &mut o.foreground),
                (&c.background, "colors.background", &mut o.background),
                (&c.cursor, "colors.cursor", &mut o.cursor),
                (
                    &c.selection_foreground,
                    "colors.selection_foreground",
                    &mut o.selection_foreground,
                ),
                (
                    &c.selection_background,
                    "colors.selection_background",
                    &mut o.selection_background,
                ),
            ] {
                if let Some(v) = field {
                    *slot = Some(color(v, key)?);
                }
            }
            if let Some(list) = &c.palette {
                let at = Some(list.span().start);
                let list = list.get_ref();
                if list.len() != 16 {
                    return Err(err(
                        at,
                        format!("colors.palette must have 16 entries, found {}", list.len()),
                    ));
                }
                let mut pal = [Rgb { r: 0, g: 0, b: 0 }; 16];
                for (slot, s) in pal.iter_mut().zip(list) {
                    *slot = color(s, "colors.palette")?;
                }
                o.palette = Some(pal);
            }
        }
        if let Some(b) = raw.bold_is_bright {
            p.bold_is_bright = b;
        }
        if let Some(o) = &raw.opacity {
            p.opacity = range(o, "opacity", 0.0, 1.0)?;
        }
        if let Some(v) = &raw.padding {
            p.padding = int_range(v, "padding", 100)? as i32;
        }
        if let Some(v) = &raw.scrollback {
            p.scrollback = int_range(v, "scrollback", 1_000_000)? as usize;
        }
        if let Some(s) = &raw.cursor_shape {
            p.cursor_shape = match s.get_ref().as_str() {
                "block" => CursorShapeCfg::Block,
                "beam" => CursorShapeCfg::Beam,
                "underline" => CursorShapeCfg::Underline,
                other => {
                    return Err(err(
                        Some(s.span().start),
                        format!(
                            "cursor_shape must be \"block\", \"beam\" or \"underline\", got {other:?}"
                        ),
                    ));
                }
            };
        }
        if let Some(b) = raw.cursor_blink {
            p.cursor_blink = b;
        }
        p.shell = raw.shell;
        if let Some(b) = raw.login_shell {
            p.login_shell = b;
        }
        if let Some(c) = raw.command {
            if c.get_ref().iter().any(|a| a.is_empty()) {
                return Err(err(
                    Some(c.span().start),
                    "command: arguments must not be empty".into(),
                ));
            }
            p.command = Some(c.into_inner());
        }
        if let Some(img) = &raw.background_image {
            let at = Some(img.span().start);
            let i = img.get_ref();
            let path = i
                .path
                .as_ref()
                .ok_or_else(|| err(at, "[background_image] needs a `path`".into()))?;
            if path.get_ref().is_empty() {
                return Err(err(
                    Some(path.span().start),
                    "background_image.path must not be empty".into(),
                ));
            }
            let fit = match i
                .fit
                .as_ref()
                .map(|f| (f.get_ref().as_str(), f.span().start))
            {
                None | Some(("fill", _)) => Fit::Fill,
                Some(("stretch", _)) => Fit::Stretch,
                Some(("center", _)) => Fit::Center,
                Some(("tile", _)) => Fit::Tile,
                Some((other, start)) => {
                    return Err(err(
                        Some(start),
                        format!(
                            "fit must be \"stretch\", \"fill\", \"center\" or \"tile\", got {other:?}"
                        ),
                    ));
                }
            };
            let opacity = match &i.opacity {
                Some(o) => range(o, "background_image.opacity", 0.0, 1.0)?,
                None => 1.0,
            };
            p.background_image = Some(BackgroundImage {
                path: expand_home(path.get_ref(), home),
                fit,
                opacity,
            });
        }
        Ok(p)
    }

    /// The program and arguments to spawn: `command` if set, else the shell (own, then
    /// `env_shell`, then `/bin/sh`) plus `-l` for a login shell.
    pub fn argv(&self, env_shell: Option<&str>) -> Vec<String> {
        if let Some(c) = self.command.as_ref().filter(|c| !c.is_empty()) {
            return c.clone();
        }
        let shell = self
            .shell
            .as_deref()
            .filter(|s| !s.is_empty())
            .or(env_shell.filter(|s| !s.is_empty()))
            .unwrap_or("/bin/sh");
        let mut argv = vec![shell.to_string()];
        if self.login_shell {
            argv.push("-l".into());
        }
        argv
    }

    /// The scheme's palette (looked up by `schemes`) with this profile's color overrides applied.
    pub fn palette(&self, schemes: &dyn Fn(&str) -> Option<Palette>) -> Result<Palette, String> {
        let mut pal = schemes(&self.scheme)
            .ok_or_else(|| format!("unknown color scheme '{}'", self.scheme))?;
        let c = &self.colors;
        if let Some(v) = c.foreground {
            pal.fg = v;
        }
        if let Some(v) = c.background {
            pal.bg = v;
        }
        if let Some(v) = c.cursor {
            pal.cursor = v;
        }
        if let Some(v) = c.selection_foreground {
            pal.selection_fg = Some(v);
        }
        if let Some(v) = c.selection_background {
            pal.selection_bg = Some(v);
        }
        if let Some(a) = c.palette {
            pal.ansi = a;
        }
        Ok(pal)
    }
}

/// Written to `profiles/default.toml` on first run.
pub const DEFAULT_PROFILE_TOML: &str = r##"# montty profile — edit and save; changes apply live. Reference: docs/config.md

name = "Default"            # display name; empty uses the file name
font = "monospace"          # font family
font_size = 11              # points, 4 to 200
scheme = "tango"            # color scheme: a file in schemes/ or a built-in name
bold_is_bright = false      # draw bold text with bright colors
opacity = 1.0               # window opacity, 0.0 to 1.0
padding = 4                 # pixels around the terminal, 0 to 100
scrollback = 10000          # lines of history, 0 to 1000000
cursor_shape = "block"      # "block", "beam" or "underline"
cursor_blink = false        # blink the cursor while typing
login_shell = false         # start the shell as a login shell (-l)
# shell = "/bin/zsh"        # default: $SHELL, else /bin/sh
# command = ["htop"]        # run this instead of a shell

# Override single colors of the scheme: "#rrggbb" or "rgb(r,g,b)".
[colors]
# foreground = "#d3d7cf"
# background = "#1e1e1e"
# cursor = "#d3d7cf"
# selection_foreground = "#000000"
# selection_background = "#729fcf"
# palette = [               # the 16 terminal colors, black to bright white
#   "#000000", "#cc0000", "#4d9a05", "#c3a000", "#3464a3", "#754f7b", "#05979a", "#d3d6cf",
#   "#545652", "#ef2828", "#89e234", "#fbe84f", "#729ecf", "#ac7ea8", "#34e2e2", "#ededeb",
# ]

# A picture behind the text.
# [background_image]
# path = "~/Pictures/bg.png"  # PNG or JPEG; relative paths start in the config folder
# fit = "fill"                # "stretch", "fill", "center" or "tile"
# opacity = 1.0               # 0.0 to 1.0
"##;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::scheme::builtin_schemes;
    use crate::term::colors::tango_default;

    fn p() -> &'static Path {
        Path::new("profiles/t.toml")
    }
    fn parse(s: &str) -> Result<Profile, ConfigError> {
        Profile::parse(s, p(), Some(Path::new("/h")))
    }
    fn rgb(r: u8, g: u8, b: u8) -> Rgb {
        Rgb { r, g, b }
    }

    #[test]
    fn unknown_key_message_is_short() {
        let e = parse("font_size = 12\nbogus = 1\n").unwrap_err();
        assert_eq!(e.line, Some(2));
        assert_eq!(e.msg, "unknown key `bogus`");
        // Also inside a table.
        let e = parse("[colors]\nbogus = 1\n").unwrap_err();
        assert_eq!(e.line, Some(2));
        assert_eq!(e.msg, "unknown key `bogus`");
    }

    #[test]
    fn default_text_parses_to_default() {
        assert_eq!(
            parse(DEFAULT_PROFILE_TOML).unwrap(),
            Profile {
                name: "Default".into(),
                ..Profile::default()
            }
        );
        assert!(DEFAULT_PROFILE_TOML.lines().count() <= 60);
    }

    #[test]
    fn empty_file_is_default() {
        assert_eq!(parse("").unwrap(), Profile::default());
    }

    #[test]
    fn full_profile() {
        let pal: Vec<String> = (0..16).map(|i| format!("\"#0000{i:02x}\"")).collect();
        let src = format!(
            r##"name = "Work"
font = "Fira Code"
font_size = 14
scheme = "nord"
bold_is_bright = true
opacity = 0.8
padding = 10
scrollback = 500
cursor_shape = "beam"
cursor_blink = true
shell = "/bin/fish"
login_shell = true
command = ["htop", "-d", "5"]
[colors]
foreground = "#fff"
background = "rgb(1, 2, 3)"
cursor = "#112233"
selection_foreground = "#000000"
selection_background = "#ff000080"
palette = [{}]
[background_image]
path = "~/p.png"
fit = "tile"
opacity = 0.5
"##,
            pal.join(", ")
        );
        let pr = parse(&src).unwrap();
        let mut ansi = [rgb(0, 0, 0); 16];
        for (i, c) in ansi.iter_mut().enumerate() {
            *c = rgb(0, 0, i as u8);
        }
        assert_eq!(
            pr,
            Profile {
                name: "Work".into(),
                font_family: "Fira Code".into(),
                font_size: 14.0,
                scheme: "nord".into(),
                colors: ColorOverrides {
                    foreground: Some(rgb(255, 255, 255)),
                    background: Some(rgb(1, 2, 3)),
                    cursor: Some(rgb(0x11, 0x22, 0x33)),
                    selection_foreground: Some(rgb(0, 0, 0)),
                    selection_background: Some(rgb(255, 0, 0)),
                    palette: Some(ansi),
                },
                bold_is_bright: true,
                opacity: 0.8,
                padding: 10,
                scrollback: 500,
                cursor_shape: CursorShapeCfg::Beam,
                cursor_blink: true,
                shell: Some("/bin/fish".into()),
                login_shell: true,
                command: Some(vec!["htop".into(), "-d".into(), "5".into()]),
                background_image: Some(BackgroundImage {
                    path: PathBuf::from("/h/p.png"),
                    fit: Fit::Tile,
                    opacity: 0.5,
                }),
            }
        );
    }

    #[test]
    fn home_expansion() {
        let src = |p: &str| format!("[background_image]\npath = \"{p}\"");
        let path = |s: &str, home: Option<&Path>| {
            Profile::parse(&src(s), p(), home)
                .unwrap()
                .background_image
                .unwrap()
                .path
        };
        let h = Some(Path::new("/h"));
        assert_eq!(path("~", h), PathBuf::from("/h"));
        assert_eq!(path("~/x", None), PathBuf::from("~/x"));
        assert_eq!(path("rel/x.png", h), PathBuf::from("rel/x.png"));
        assert_eq!(path("~//x", h), PathBuf::from("/h/x"));
        assert_eq!(path("~user/x", h), PathBuf::from("~user/x"));
        let d = Profile::parse(&src("/a.png"), p(), h).unwrap();
        let img = d.background_image.unwrap();
        assert_eq!((img.fit, img.opacity), (Fit::Fill, 1.0));
    }

    #[test]
    fn validation_ranges() {
        let bad = |src: &str, line: usize, word: &str| {
            let e = parse(src).unwrap_err();
            assert_eq!(e.line, Some(line), "{src}: {e}");
            assert!(e.msg.contains(word), "{src}: {e}");
        };
        bad("\nfont_size = 2.0", 2, "font_size");
        bad("font_size = 201", 1, "font_size");
        bad("opacity = 1.5", 1, "opacity");
        bad("padding = -1", 1, "padding");
        bad("padding = 2.5", 1, "padding");
        bad("scrollback = -5", 1, "scrollback");
        bad("scrollback = 1000001", 1, "scrollback");
        bad(
            "cursor_blink = true\ncursor_shape = \"box\"",
            2,
            "cursor_shape",
        );
        bad("[background_image]\npath = \"a\"\nfit = \"zoom\"", 3, "fit");
        bad(
            "[background_image]\npath = \"a\"\nopacity = 2",
            3,
            "opacity",
        );
        bad("[background_image]\nfit = \"fill\"", 1, "path");
        bad(
            "[colors]\npalette = [\"#000\", \"#111\", \"#222\"]",
            2,
            "palette",
        );
        bad("[colors]\nbackground = \"blue\"", 2, "background");
        bad("\nbogus = 1", 2, "bogus");
        bad("[colors]\nbogus = 1", 2, "bogus");
        bad("[background_image]\npath = \"a\"\nbogus = 1", 3, "bogus");
        bad("font_size = \"big\"", 1, "");
    }

    #[test]
    fn argv_rules() {
        let d = Profile::default();
        assert_eq!(d.argv(Some("/bin/zsh")), ["/bin/zsh"]);
        assert_eq!(d.argv(None), ["/bin/sh"]);
        assert_eq!(d.argv(Some("")), ["/bin/sh"]);
        let login = Profile {
            login_shell: true,
            ..Profile::default()
        };
        assert_eq!(login.argv(Some("/bin/zsh")), ["/bin/zsh", "-l"]);
        let cmd = Profile {
            command: Some(vec!["htop".into()]),
            login_shell: true,
            ..Profile::default()
        };
        assert_eq!(cmd.argv(Some("/bin/zsh")), ["htop"]);
        let empty_cmd = Profile {
            command: Some(vec![]),
            ..Profile::default()
        };
        assert_eq!(empty_cmd.argv(Some("/bin/zsh")), ["/bin/zsh"]);
        let sh = Profile {
            shell: Some("fish".into()),
            ..Profile::default()
        };
        assert_eq!(sh.argv(Some("/bin/zsh")), ["fish"]);
    }

    #[test]
    fn palette_overrides() {
        let schemes = |s: &str| (s == "tango").then(tango_default);
        let pr = parse("[colors]\nbackground = \"#000000\"\nselection_background = \"#123456\"")
            .unwrap();
        let pal = pr.palette(&schemes).unwrap();
        assert_eq!(pal.bg, rgb(0, 0, 0));
        assert_eq!(pal.fg, tango_default().fg);
        assert_eq!(pal.ansi, tango_default().ansi);
        assert_eq!(pal.selection_bg, Some(rgb(0x12, 0x34, 0x56)));
        assert_eq!(pal.selection_fg, tango_default().selection_fg);
        let e = parse("scheme = \"nope\"")
            .unwrap()
            .palette(&schemes)
            .unwrap_err();
        assert!(e.contains("nope"), "{e}");
        // Works against the built-in set too.
        let (_, first) = builtin_schemes().into_iter().next().unwrap();
        let by = |_: &str| Some(first.palette.clone());
        assert_eq!(Profile::default().palette(&by).unwrap(), first.palette);
    }

    #[test]
    fn default_examples_valid_when_uncommented() {
        let ident = |r: &str| {
            let k = r.split(" =").next().unwrap_or("");
            r.contains(" =")
                && !k.is_empty()
                && k.chars().all(|c| c.is_ascii_lowercase() || c == '_')
        };
        let text: String = DEFAULT_PROFILE_TOML
            .lines()
            .map(|l| match l.strip_prefix("# ") {
                Some(r)
                    if ident(r)
                        || r.starts_with("[background_image]")
                        || r.trim_start().starts_with(['"', ']']) =>
                {
                    format!("{r}\n")
                }
                _ => format!("{l}\n"),
            })
            .collect();
        let pr = parse(&text).unwrap();
        assert_eq!(pr.colors.palette, Some(tango_default().ansi));
        assert_eq!(pr.colors.foreground, Some(tango_default().fg));
        assert_eq!(pr.colors.background, Some(tango_default().bg));
        assert!(pr.background_image.is_some() && pr.command.is_some() && pr.shell.is_some());
    }

    #[test]
    fn friendly_type_errors() {
        let msg = |src: &str| {
            let e = parse(src).unwrap_err();
            (e.line, e.msg)
        };
        assert_eq!(
            msg("\npadding = 2.5"),
            (Some(2), "padding: expected an integer, got 2.5".to_string())
        );
        let (l, m) = msg("scrollback = 99999999999999999999");
        assert_eq!(l, Some(1));
        assert!(m.starts_with("scrollback: expected an integer"), "{m}");
        assert_eq!(
            msg("opacity = \"x\""),
            (Some(1), "opacity: expected a number".to_string())
        );
        let (l, m) = msg("[background_image]\npath = \"a\"\nopacity = true");
        assert_eq!(l, Some(3));
        assert!(m.starts_with("opacity: expected a number"), "{m}");
    }

    #[test]
    fn palette_error_on_entry_line() {
        let mut ents: Vec<String> = (0..16).map(|i| format!("  \"#0000{i:02x}\",")).collect();
        ents[5] = "  \"nope\",".into();
        let src = format!("[colors]\npalette = [\n{}\n]", ents.join("\n"));
        let e = parse(&src).unwrap_err();
        assert_eq!(e.line, Some(8), "{e}");
        assert!(e.msg.contains("palette") && e.msg.contains("nope"), "{e}");
        let e = parse("[colors]\npalette = [\n\"#000\",\n]").unwrap_err();
        assert_eq!(e.line, Some(2), "{e}");
    }

    #[test]
    fn empty_values_rejected() {
        let e = parse("\ncommand = [\"ls\", \"\"]").unwrap_err();
        assert_eq!(e.line, Some(2));
        assert!(e.msg.contains("command"), "{e}");
        let e = parse("[background_image]\npath = \"\"").unwrap_err();
        assert_eq!(e.line, Some(2));
        assert!(e.msg.contains("path"), "{e}");
    }
}
