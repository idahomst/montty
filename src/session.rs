//! Saved sessions: the JSON file format (spec §9), session names and atomic writes.
//! Paths are relative to the config directory.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize, Serializer};

use crate::config::ConfigError;
use crate::layout::{Orientation, Shape};

/// The name reserved for the autosave; only `last_file` reaches it.
pub const LAST: &str = "_last";

const MAX_NAME_CHARS: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub struct LeafDoc {
    /// The profile stem; empty or unknown falls back to the default profile later.
    pub profile: String,
    pub cwd: Option<PathBuf>,
    pub command: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TabDoc {
    pub title: Option<String>,
    /// Depth-first index of the focused leaf.
    pub focused: usize,
    pub root: Shape<LeafDoc>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SessionDoc {
    /// Inner window size in logical pixels.
    pub window: Option<(u32, u32)>,
    pub active_tab: usize,
    pub tabs: Vec<TabDoc>,
}

// ---- JSON representation ----

/// Accepts only `1`.
struct Version;

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match u64::deserialize(d)? {
            1 => Ok(Version),
            v => Err(de::Error::custom(format!(
                "version: {v} is not supported (expected 1)"
            ))),
        }
    }
}

impl Serialize for Version {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u32(1)
    }
}

/// A split ratio, 0.0 to 1.0.
#[derive(Clone, Copy)]
struct Ratio(f32);

impl<'de> Deserialize<'de> for Ratio {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let r = f32::deserialize(d)?;
        if (0.0..=1.0).contains(&r) {
            Ok(Ratio(r))
        } else {
            Err(de::Error::custom(format!(
                "ratio: {r} is out of range (0.0 to 1.0)"
            )))
        }
    }
}

impl Serialize for Ratio {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_f32(self.0)
    }
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Dir {
    Right,
    Down,
}

/// Leaf and split keys together; `TryFrom` sorts out which one it is.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawNode {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    split: Option<Dir>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ratio: Option<Ratio>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    a: Option<Box<Node>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    b: Option<Box<Node>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cwd: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    command: Option<Vec<String>>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(try_from = "RawNode", into = "RawNode")]
struct Node(Shape<LeafDoc>);

impl TryFrom<RawNode> for Node {
    type Error = String;

    fn try_from(r: RawNode) -> Result<Self, String> {
        let Some(dir) = r.split else {
            if r.ratio.is_some() || r.a.is_some() || r.b.is_some() {
                return Err("a leaf can't have ratio, a or b (no \"split\" key)".into());
            }
            return Ok(Node(Shape::Leaf(LeafDoc {
                profile: r.profile.unwrap_or_default(),
                cwd: r.cwd,
                command: r.command,
            })));
        };
        if r.profile.is_some() || r.cwd.is_some() || r.command.is_some() {
            return Err("a split can't have profile, cwd or command".into());
        }
        let (Some(ratio), Some(a), Some(b)) = (r.ratio, r.a, r.b) else {
            return Err("a split needs ratio, a and b".into());
        };
        Ok(Node(Shape::Split {
            orientation: match dir {
                Dir::Right => Orientation::Horizontal,
                Dir::Down => Orientation::Vertical,
            },
            ratio: ratio.0,
            a: Box::new(a.0),
            b: Box::new(b.0),
        }))
    }
}

impl From<Node> for RawNode {
    fn from(n: Node) -> RawNode {
        let empty = RawNode {
            split: None,
            ratio: None,
            a: None,
            b: None,
            profile: None,
            cwd: None,
            command: None,
        };
        match n.0 {
            Shape::Leaf(l) => RawNode {
                profile: Some(l.profile),
                cwd: l.cwd,
                command: l.command,
                ..empty
            },
            Shape::Split {
                orientation,
                ratio,
                a,
                b,
            } => RawNode {
                split: Some(match orientation {
                    Orientation::Horizontal => Dir::Right,
                    Orientation::Vertical => Dir::Down,
                }),
                ratio: Some(Ratio(ratio)),
                a: Some(Box::new(Node(*a))),
                b: Some(Box::new(Node(*b))),
                ..empty
            },
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawWindow {
    width: u32,
    height: u32,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawTab {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    focused: usize,
    root: Node,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawSession {
    version: Version,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    window: Option<RawWindow>,
    active_tab: usize,
    tabs: Vec<RawTab>,
}

/// Parse and validate a session file. `file` is its path relative to the config dir,
/// used in errors.
pub fn parse(src: &str, file: &Path) -> Result<SessionDoc, ConfigError> {
    let fail = |line: Option<usize>, msg: String| ConfigError {
        file: file.to_path_buf(),
        line,
        msg,
    };
    let raw: RawSession = serde_json::from_str(src).map_err(|e| {
        let full = e.to_string();
        // serde_json appends " at line L column C"; the line is reported separately.
        let msg = full
            .rsplit_once(" at line ")
            .map_or(full.as_str(), |(m, _)| m)
            .to_string();
        fail((e.line() > 0).then(|| e.line()), msg)
    })?;
    if raw.tabs.is_empty() {
        return Err(fail(None, "tabs: at least one tab is required".into()));
    }
    if raw.active_tab >= raw.tabs.len() {
        return Err(fail(
            None,
            format!(
                "active_tab: {} is out of range ({} tabs)",
                raw.active_tab,
                raw.tabs.len()
            ),
        ));
    }
    let mut tabs = Vec::with_capacity(raw.tabs.len());
    for (i, t) in raw.tabs.into_iter().enumerate() {
        let panes = t.root.0.leaves().len();
        if t.focused >= panes {
            return Err(fail(
                None,
                format!(
                    "tabs[{i}].focused: {} is out of range ({panes} panes)",
                    t.focused
                ),
            ));
        }
        tabs.push(TabDoc {
            title: t.title,
            focused: t.focused,
            root: t.root.0,
        });
    }
    Ok(SessionDoc {
        window: raw.window.map(|w| (w.width, w.height)),
        active_tab: raw.active_tab,
        tabs,
    })
}

/// The session as pretty-printed JSON.
pub fn to_json(doc: &SessionDoc) -> String {
    let raw = RawSession {
        version: Version,
        window: doc
            .window
            .map(|(width, height)| RawWindow { width, height }),
        active_tab: doc.active_tab,
        tabs: doc
            .tabs
            .iter()
            .map(|t| RawTab {
                title: t.title.clone(),
                focused: t.focused,
                root: Node(t.root.clone()),
            })
            .collect(),
    };
    let mut s = serde_json::to_string_pretty(&raw).expect("session documents serialize");
    s.push('\n');
    s
}

// ---- names and files ----

fn check_name(name: &str) -> Result<(), String> {
    let n = name.chars().count();
    if n == 0 {
        Err("the name is empty".into())
    } else if n > MAX_NAME_CHARS {
        Err(format!(
            "the name is longer than {MAX_NAME_CHARS} characters"
        ))
    } else if name.contains('/') {
        Err("the name can't contain '/'".into())
    } else if name.chars().any(char::is_control) {
        Err("the name can't contain control characters".into())
    } else if name.starts_with('.') {
        Err("the name can't start with '.'".into())
    } else if name == LAST {
        Err(format!("'{LAST}' is reserved for the automatic save"))
    } else {
        Ok(())
    }
}

/// `sessions/<name>.json` relative to the config dir, or why the name is invalid.
pub fn session_file(name: &str) -> Result<PathBuf, String> {
    check_name(name)?;
    Ok(Path::new("sessions").join(format!("{name}.json")))
}

/// The autosave file, `sessions/_last.json`.
pub fn last_file() -> PathBuf {
    Path::new("sessions").join(format!("{LAST}.json"))
}

/// Saved session names under the config dir `dir`, sorted. Skips `_last`, non-`.json`
/// entries, directories and names that couldn't be saved.
pub fn list_sessions(dir: &Path) -> Vec<String> {
    let Ok(rd) = fs::read_dir(dir.join("sessions")) else {
        return Vec::new();
    };
    let mut names: Vec<String> = rd
        .filter_map(Result::ok)
        .filter(|e| fs::metadata(e.path()).is_ok_and(|m| m.is_file()))
        .filter_map(|e| {
            let p = e.path();
            if p.extension()? != "json" {
                return None;
            }
            let stem = p.file_stem()?.to_str()?.to_string();
            check_name(&stem).ok().map(|()| stem)
        })
        .collect();
    names.sort();
    names
}

/// Read and parse the session file `rel` (relative to the config dir `dir`). Errors are
/// ready to show: the config error format (`sessions/<name>.json:<line>: <msg>`).
pub fn read_file(dir: Option<&Path>, rel: &Path) -> Result<SessionDoc, String> {
    let dir = dir
        .filter(|d| !d.as_os_str().is_empty())
        .ok_or("no config directory ($HOME unset)")?;
    let src = fs::read_to_string(dir.join(rel)).map_err(|e| format!("{}: {e}", rel.display()))?;
    parse(&src, rel).map_err(|e| e.to_string())
}

/// Read the saved session `name`; the error says why it can't be opened (an invalid
/// name, a missing or unreadable file, or a file with errors).
pub fn read_session(dir: Option<&Path>, name: &str) -> Result<SessionDoc, String> {
    let rel = session_file(name).map_err(|why| format!("invalid session name: {why}"))?;
    read_file(dir, &rel)
}

/// Write `contents` to `path` and flush it to disk before returning.
fn write_synced(path: &Path, contents: &str) -> io::Result<()> {
    use std::io::Write;
    let mut f = fs::File::create(path)?;
    f.write_all(contents.as_bytes())?;
    f.sync_all()
}

/// Write `contents` to `dir/rel` through `<file>.tmp` and a rename, creating the parent
/// directory if needed. A failure leaves no temporary file behind.
pub fn write_atomic(dir: &Path, rel: &Path, contents: &str) -> io::Result<()> {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut tmp = path.clone().into_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let result = write_synced(&tmp, contents).and_then(|()| fs::rename(&tmp, &path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Orientation, Shape};

    const EXAMPLE: &str = r#"{
  "version": 1,
  "window": { "width": 1000, "height": 700 },
  "active_tab": 0,
  "tabs": [
    { "title": "build", "focused": 1,
      "root": { "split": "right", "ratio": 0.5,
                "a": { "profile": "default", "cwd": "/home/u/src" },
                "b": { "profile": "work", "cwd": "/tmp", "command": ["htop"] } } }
  ]
}"#;

    fn file() -> &'static Path {
        Path::new("sessions/x.json")
    }

    fn err(src: &str) -> ConfigError {
        parse(src, file()).unwrap_err()
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("montty-session-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn json_round_trip() {
        let doc = parse(EXAMPLE, file()).unwrap();
        assert_eq!(doc.window, Some((1000, 700)));
        assert_eq!(doc.active_tab, 0);
        let tab = &doc.tabs[0];
        assert_eq!(tab.title.as_deref(), Some("build"));
        assert_eq!(tab.focused, 1);
        let Shape::Split {
            orientation,
            ratio,
            a,
            b,
        } = &tab.root
        else {
            panic!("expected a split");
        };
        assert_eq!(*orientation, Orientation::Horizontal);
        assert_eq!(*ratio, 0.5);
        assert_eq!(
            **a,
            Shape::Leaf(LeafDoc {
                profile: "default".into(),
                cwd: Some("/home/u/src".into()),
                command: None
            })
        );
        assert_eq!(
            **b,
            Shape::Leaf(LeafDoc {
                profile: "work".into(),
                cwd: Some("/tmp".into()),
                command: Some(vec!["htop".into()])
            })
        );
        let again = parse(&to_json(&doc), file()).unwrap();
        assert_eq!(again, doc);
    }

    #[test]
    fn round_trip_without_optionals_and_down_split() {
        let doc = SessionDoc {
            window: None,
            active_tab: 1,
            tabs: vec![
                TabDoc {
                    title: None,
                    focused: 0,
                    root: Shape::Leaf(LeafDoc {
                        profile: String::new(),
                        cwd: None,
                        command: None,
                    }),
                },
                TabDoc {
                    title: Some("t".into()),
                    focused: 1,
                    root: Shape::Split {
                        orientation: Orientation::Vertical,
                        ratio: 0.3,
                        a: Box::new(Shape::Leaf(LeafDoc {
                            profile: "p".into(),
                            cwd: None,
                            command: None,
                        })),
                        b: Box::new(Shape::Leaf(LeafDoc {
                            profile: "q".into(),
                            cwd: None,
                            command: None,
                        })),
                    },
                },
            ],
        };
        let json = to_json(&doc);
        assert!(json.contains("\"down\""), "{json}");
        assert_eq!(parse(&json, file()).unwrap(), doc);
    }

    #[test]
    fn parse_errors_name_the_line() {
        let e = err("{\n  \"version\": 1,\n  \"active_tab\": \"zero\",\n  \"tabs\": []\n}");
        assert_eq!(e.file, Path::new("sessions/x.json"));
        assert_eq!(e.line, Some(3));
        assert!(e.to_string().starts_with("sessions/x.json:3: "), "{e}");
        let e = err("{ not json");
        assert_eq!(e.line, Some(1));
    }

    #[test]
    fn ratio_out_of_range_is_error() {
        for r in ["1.5", "-0.1"] {
            let src = format!(
                "{{\"version\":1,\"active_tab\":0,\"tabs\":[{{\"focused\":0,\"root\":\n{{\"split\":\"right\",\"ratio\":{r},\n\"a\":{{\"profile\":\"x\"}},\"b\":{{\"profile\":\"x\"}}}}}}]}}"
            );
            let e = err(&src);
            assert!(e.line.is_some(), "{e}");
            assert!(e.msg.contains("ratio"), "{e}");
        }
    }

    #[test]
    fn unknown_key_is_error() {
        let e = err(
            "{\"version\":1,\"active_tab\":0,\"bogus\":1,\"tabs\":[{\"focused\":0,\"root\":{\"profile\":\"x\"}}]}",
        );
        assert!(e.msg.contains("bogus"), "{e}");
        let e = err(
            "{\"version\":1,\"active_tab\":0,\"tabs\":[{\"focused\":0,\"root\":{\"profile\":\"x\",\"colour\":1}}]}",
        );
        assert!(e.msg.contains("colour"), "{e}");
    }

    #[test]
    fn leaf_and_split_keys_do_not_mix() {
        let leaf = "{\"profile\":\"x\"}";
        let wrap = |root: &str| {
            format!(
                "{{\"version\":1,\"active_tab\":0,\"tabs\":[{{\"focused\":0,\"root\":{root}}}]}}"
            )
        };
        // A leaf with a split key.
        assert!(parse(&wrap("{\"profile\":\"x\",\"ratio\":0.5}"), file()).is_err());
        // A split with a leaf key.
        let s = format!(
            "{{\"split\":\"right\",\"ratio\":0.5,\"a\":{leaf},\"b\":{leaf},\"cwd\":\"/\"}}"
        );
        assert!(parse(&wrap(&s), file()).is_err());
        // A split missing b.
        let s = format!("{{\"split\":\"right\",\"ratio\":0.5,\"a\":{leaf}}}");
        assert!(parse(&wrap(&s), file()).is_err());
        // A split with a bad direction.
        let s = format!("{{\"split\":\"left\",\"ratio\":0.5,\"a\":{leaf},\"b\":{leaf}}}");
        assert!(parse(&wrap(&s), file()).is_err());
    }

    #[test]
    fn version_2_is_error() {
        let e = err(
            "{\"version\":2,\"active_tab\":0,\"tabs\":[{\"focused\":0,\"root\":{\"profile\":\"x\"}}]}",
        );
        assert_eq!(e.line, Some(1));
        assert!(e.msg.contains("version"), "{e}");
    }

    #[test]
    fn no_tabs_and_active_tab_range_are_errors() {
        let e = err("{\"version\":1,\"active_tab\":0,\"tabs\":[]}");
        assert_eq!(e.line, None);
        assert!(e.msg.contains("tabs"), "{e}");
        let e = err(
            "{\"version\":1,\"active_tab\":1,\"tabs\":[{\"focused\":0,\"root\":{\"profile\":\"x\"}}]}",
        );
        assert_eq!(e.line, None);
        assert_eq!(e.msg, "active_tab: 1 is out of range (1 tabs)");
    }

    #[test]
    fn focused_out_of_range_is_error() {
        let leaf = "{\"profile\":\"x\"}";
        let src = format!(
            "{{\"version\":1,\"active_tab\":0,\"tabs\":[{{\"focused\":0,\"root\":{leaf}}},\
             {{\"focused\":4,\"root\":{{\"split\":\"down\",\"ratio\":0.5,\"a\":{leaf},\"b\":{leaf}}}}}]}}"
        );
        let e = err(&src);
        assert_eq!(e.line, None);
        assert_eq!(e.msg, "tabs[1].focused: 4 is out of range (2 panes)");
    }

    #[test]
    fn unknown_profile_is_kept_for_fallback() {
        let doc = parse(
            "{\"version\":1,\"active_tab\":0,\"tabs\":[{\"focused\":0,\"root\":{\"profile\":\"nope\"}}]}",
            file(),
        )
        .unwrap();
        let Shape::Leaf(l) = &doc.tabs[0].root else {
            panic!()
        };
        assert_eq!(l.profile, "nope");
    }

    #[test]
    fn session_names_validated() {
        let long = "a".repeat(65);
        for bad in [
            "",
            "a/b",
            "../x",
            ".h",
            "_last",
            long.as_str(),
            "a\nb",
            "a\0b",
        ] {
            assert!(session_file(bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            session_file("work").unwrap(),
            PathBuf::from("sessions/work.json")
        );
        assert!(session_file(&"a".repeat(64)).is_ok());
        assert_eq!(last_file(), PathBuf::from("sessions/_last.json"));
    }

    #[test]
    fn unicode_name_ok() {
        assert_eq!(
            session_file("práce").unwrap(),
            PathBuf::from("sessions/práce.json")
        );
        // 64 characters, more than 64 bytes.
        assert!(session_file(&"á".repeat(64)).is_ok());
    }

    #[test]
    fn list_skips_last_and_non_json() {
        let d = temp("list");
        assert!(list_sessions(&d).is_empty());
        let s = d.join("sessions");
        std::fs::create_dir_all(&s).unwrap();
        for f in [
            "b.json",
            "a.json",
            "_last.json",
            "c.txt",
            "d.json.tmp",
            ".h.json",
            "práce.json",
        ] {
            std::fs::write(s.join(f), "{}").unwrap();
        }
        std::fs::create_dir(s.join("dir.json")).unwrap();
        assert_eq!(list_sessions(&d), ["a", "b", "práce"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn listing_follows_symlinked_files() {
        let d = temp("symlink");
        let s = d.join("sessions");
        std::fs::create_dir_all(&s).unwrap();
        let store = d.join("dotfiles");
        std::fs::create_dir_all(store.join("adir")).unwrap();
        std::fs::write(store.join("real.json"), "{}").unwrap();
        std::os::unix::fs::symlink(store.join("real.json"), s.join("work.json")).unwrap();
        // A dangling link and a link to a directory aren't sessions.
        std::os::unix::fs::symlink(store.join("gone.json"), s.join("dangling.json")).unwrap();
        std::os::unix::fs::symlink(store.join("adir"), s.join("dirlink.json")).unwrap();
        assert_eq!(list_sessions(&d), ["work"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn write_atomic_replaces_whole_file() {
        let d = temp("atomic");
        let rel = session_file("w").unwrap();
        write_atomic(&d, &rel, &"x".repeat(1000)).unwrap();
        write_atomic(&d, &rel, "short").unwrap();
        assert_eq!(std::fs::read_to_string(d.join(&rel)).unwrap(), "short");
        let names: Vec<_> = std::fs::read_dir(d.join("sessions"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, ["w.json"]);
        let _ = std::fs::remove_dir_all(&d);
    }
}
