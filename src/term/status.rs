//! OSC 7501 agent status reports: parsing, per-pane records, display text.
//!
//! Pure data. The PTY scanner (`osc_scan`) hands over the raw payload after `7501;`; content is
//! untrusted, so every limit is checked before any record changes and text is sanitised for display.

use base64::Engine;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};

/// Records kept per pane (the spec's minimum); the least recently updated is evicted.
pub const MAX_RECORDS: usize = 64;

const MAX_KEY: usize = 16;
const MAX_MSG_ENCODED: usize = 2732;
const MAX_MSG: usize = 2048;
const MAX_TITLE_ENCODED: usize = 256;
const MAX_TITLE: usize = 192;
const MAX_APP: usize = 32;
const MAX_ID: usize = 128;
const MAX_SEGMENT: usize = 32;
const MAX_LEVELS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Working,
    Done,
    Blocked,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Permission,
    Question,
    Auth,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub state: State,
    pub kind: Option<Kind>,
    pub app: Option<String>,
    pub msg: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Report {
    Set { id: Option<String>, record: Record },
    Clear { id: Option<String> },
}

fn is_segment_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '+' | '-')
}

fn valid_app(s: &str) -> bool {
    (1..=MAX_APP).contains(&s.len()) && s.chars().all(is_segment_char)
}

fn valid_id(s: &str) -> bool {
    s.len() <= MAX_ID
        && s.split('/').count() <= MAX_LEVELS
        && s.split('/')
            .all(|seg| (1..=MAX_SEGMENT).contains(&seg.len()) && seg.chars().all(is_segment_char))
}

/// Standard base64, padding optional.
fn decode_text(s: &str, max_encoded: usize, max: usize) -> Option<String> {
    const ENGINE: GeneralPurpose = GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
    );
    if s.len() > max_encoded {
        return None;
    }
    let bytes = ENGINE.decode(s).ok()?;
    if bytes.len() > max {
        return None;
    }
    let text = String::from_utf8(bytes).ok()?;
    if text.chars().any(char::is_control) {
        return None;
    }
    Some(text)
}

/// Parses the payload after `7501;`. `None` discards the whole report.
pub fn parse_report(payload: &str) -> Option<Report> {
    let mut state: Option<&str> = None;
    let mut id: Option<String> = None;
    let mut kind: Option<&str> = None;
    let mut app: Option<&str> = None;
    let mut msg: Option<String> = None;
    for pair in payload.split(':') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        if key.is_empty() || !key.bytes().all(|b| b.is_ascii_lowercase()) {
            continue;
        }
        // Every pair is validated as it is seen, so a bad earlier duplicate still discards the report.
        if key.len() > MAX_KEY {
            return None;
        }
        match key {
            "state" => state = Some(value),
            "id" => {
                if !valid_id(value) {
                    return None;
                }
                id = Some(value.to_owned());
            }
            "kind" => kind = Some(value),
            "app" => {
                if !valid_app(value) {
                    return None;
                }
                app = Some(value);
            }
            "msg" => {
                msg = Some(decode_text(value, MAX_MSG_ENCODED, MAX_MSG)?).filter(|m| !m.is_empty())
            }
            "title" => {
                decode_text(value, MAX_TITLE_ENCODED, MAX_TITLE)?;
            }
            _ => {}
        }
    }
    let state = match state? {
        "clear" => return Some(Report::Clear { id }),
        "idle" => State::Idle,
        "working" => State::Working,
        "done" => State::Done,
        "blocked" => State::Blocked,
        "error" => State::Error,
        _ => return None,
    };
    let kind = match (state, kind) {
        (State::Blocked, Some("permission")) => Some(Kind::Permission),
        (State::Blocked, Some("question")) => Some(Kind::Question),
        (State::Blocked, Some("auth")) => Some(Kind::Auth),
        _ => None,
    };
    Some(Report::Set {
        id,
        record: Record {
            state,
            kind,
            app: app.map(str::to_owned),
            msg,
        },
    })
}

/// A pane's records, oldest update first.
#[derive(Default)]
pub struct Records {
    entries: Vec<(Option<String>, Record)>,
}

impl Records {
    pub fn apply(&mut self, report: Report) {
        match report {
            Report::Set { id, mut record } => {
                if record.app.is_none() {
                    record.app = self.inherited_app(id.as_deref());
                }
                self.entries.retain(|(k, _)| *k != id);
                self.entries.push((id, record));
                if self.entries.len() > MAX_RECORDS {
                    self.entries.remove(0);
                }
            }
            Report::Clear { id: None } => self.entries.clear(),
            Report::Clear { id: Some(id) } => {
                let prefix = format!("{id}/");
                self.entries.retain(|(k, _)| match k {
                    Some(k) => *k != id && !k.starts_with(&prefix),
                    None => true,
                });
            }
        }
    }

    /// `app` of the nearest ancestor that has one, fixed when the record is stored.
    fn inherited_app(&self, id: Option<&str>) -> Option<String> {
        let mut path = id?;
        loop {
            path = path.rsplit_once('/').map_or("", |(p, _)| p);
            let key = (!path.is_empty()).then_some(path);
            if let Some((_, r)) = self.entries.iter().find(|(k, _)| k.as_deref() == key)
                && r.app.is_some()
            {
                return r.app.clone();
            }
            key?;
        }
    }

    /// OSC 133 `A` or process exit: the agent is no longer working or waiting.
    pub fn prompt_start(&mut self) {
        self.entries
            .retain(|(_, r)| !matches!(r.state, State::Working | State::Blocked));
    }

    /// RIS.
    pub fn reset(&mut self) {
        self.entries.clear();
    }

    /// The terminal-wide record (no `id`).
    pub fn root(&self) -> Option<&Record> {
        self.entries
            .iter()
            .find(|(k, _)| k.is_none())
            .map(|(_, r)| r)
    }
}

/// Text for the mark: `<app>: <msg or fixed text>`.
pub fn label(record: &Record) -> String {
    let fixed = match (record.state, record.kind) {
        (State::Blocked, Some(Kind::Permission)) => "Needs permission",
        (State::Blocked, Some(Kind::Question)) => "Has a question",
        (State::Blocked, Some(Kind::Auth)) => "Needs login",
        (State::Blocked, None) => "Needs you",
        (State::Done, _) => "Done",
        (State::Error, _) => "Failed",
        (State::Working, _) => "Working",
        (State::Idle, _) => "Idle",
    };
    let text = record
        .msg
        .as_deref()
        .map_or_else(|| fixed.to_owned(), display_text);
    match record.app.as_deref() {
        Some(app) => format!("{}: {text}", display_text(app)),
        None => text,
    }
}

/// Drops control characters, bidi overrides/isolates and invisible formatting characters.
pub fn display_text(s: &str) -> String {
    s.chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(*c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
                    | '\u{200B}'..='\u{200F}' | '\u{2060}'..='\u{2064}' | '\u{FEFF}')
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(s: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(s)
    }

    fn rec(state: State) -> Record {
        Record {
            state,
            kind: None,
            app: None,
            msg: None,
        }
    }

    fn set(id: Option<&str>, state: State) -> Report {
        Report::Set {
            id: id.map(str::to_owned),
            record: rec(state),
        }
    }

    fn state_of(p: &str) -> State {
        match parse_report(p) {
            Some(Report::Set { record, .. }) => record.state,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parses_blocked_with_kind_and_msg() {
        let p = format!(
            "state=blocked:kind=permission:app=claude-code:msg={}",
            b64("Allow edit?")
        );
        assert_eq!(
            parse_report(&p),
            Some(Report::Set {
                id: None,
                record: Record {
                    state: State::Blocked,
                    kind: Some(Kind::Permission),
                    app: Some("claude-code".into()),
                    msg: Some("Allow edit?".into()),
                }
            })
        );
    }

    #[test]
    fn unknown_state_or_bad_id_discards() {
        assert_eq!(parse_report("state=busy"), None);
        assert_eq!(parse_report("state=done:id=a b"), None);
        assert_eq!(parse_report("state=done:id=x/y/z/a/b/c/d/e/f"), None);
    }

    #[test]
    fn malformed_pairs_skipped_unknown_keys_ignored_last_wins() {
        assert_eq!(state_of("junk:state=working:foo=1:state=done"), State::Done);
        assert_eq!(state_of(" state = done "), State::Done);
    }

    #[test]
    fn limits_discard() {
        let p = format!("state=done:msg={}", b64(&"a".repeat(2049)));
        assert_eq!(parse_report(&p), None);
        let p = format!("state=done:msg={}", b64(&"a".repeat(2048)));
        assert!(parse_report(&p).is_some());
        let p = format!("state=done:app={}", "a".repeat(33));
        assert_eq!(parse_report(&p), None);
    }

    #[test]
    fn bad_earlier_duplicate_discards() {
        let big = b64(&"a".repeat(3000));
        assert_eq!(
            parse_report(&format!("state=done:msg={big}:msg={}", b64("ok"))),
            None
        );
        assert_eq!(parse_report("state=done:id=a b:id=a"), None);
        assert_eq!(
            parse_report(&format!("state=done:app={}:app=x", "a".repeat(33))),
            None
        );
    }

    #[test]
    fn unpadded_base64_is_accepted() {
        // "Allow edit?" is 11 bytes, so the padded form ends in "=".
        let padded = b64("Allow edit?");
        assert!(padded.ends_with('='));
        let p = format!("state=done:msg={}", padded.trim_end_matches('='));
        match parse_report(&p) {
            Some(Report::Set { record, .. }) => {
                assert_eq!(record.msg.as_deref(), Some("Allow edit?"))
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn control_chars_drop_report() {
        let p = format!("state=done:msg={}", b64("a\nb"));
        assert_eq!(parse_report(&p), None);
        assert_eq!(parse_report("state=done:msg=@@@"), None);
    }

    #[test]
    fn kind_only_with_blocked() {
        let kind = |p| match parse_report(p) {
            Some(Report::Set { record, .. }) => record.kind,
            other => panic!("{other:?}"),
        };
        assert_eq!(kind("state=done:kind=question"), None);
        assert_eq!(kind("state=blocked:kind=nope"), None);
        assert_eq!(kind("state=blocked:kind=auth"), Some(Kind::Auth));
    }

    #[test]
    fn report_replaces_record_and_clear_removes_subtree() {
        let mut r = Records::default();
        r.apply(set(None, State::Working));
        r.apply(set(Some("a"), State::Working));
        r.apply(set(Some("a/b"), State::Working));
        r.apply(set(None, State::Done));
        assert_eq!(r.root().unwrap().state, State::Done);
        r.apply(Report::Clear {
            id: Some("a".into()),
        });
        assert_eq!(r.entries.len(), 1);
        assert!(r.root().is_some());
        r.apply(set(Some("ab"), State::Done));
        r.apply(Report::Clear {
            id: Some("a".into()),
        });
        assert_eq!(r.entries.len(), 2, "sibling with a shared prefix survives");
        r.apply(Report::Clear { id: None });
        assert!(r.entries.is_empty());
        assert!(r.root().is_none());
    }

    #[test]
    fn app_is_inherited_from_nearest_ancestor() {
        let mut r = Records::default();
        let with_app = Record {
            app: Some("x".into()),
            ..rec(State::Working)
        };
        r.apply(Report::Set {
            id: None,
            record: with_app,
        });
        r.apply(set(Some("a/b"), State::Done));
        assert_eq!(r.entries[1].1.app.as_deref(), Some("x"));
    }

    #[test]
    fn prompt_start_drops_working_and_blocked_only() {
        let mut r = Records::default();
        r.apply(set(None, State::Blocked));
        r.apply(set(Some("w"), State::Working));
        r.apply(set(Some("d"), State::Done));
        r.prompt_start();
        assert!(r.root().is_none());
        assert_eq!(r.entries.len(), 1);
        r.apply(set(None, State::Done));
        r.prompt_start();
        assert_eq!(r.root().unwrap().state, State::Done);
        r.reset();
        assert!(r.root().is_none());
    }

    #[test]
    fn evicts_least_recently_updated_beyond_max() {
        let mut r = Records::default();
        for i in 0..=MAX_RECORDS {
            r.apply(set(Some(&format!("i{i}")), State::Done));
        }
        assert_eq!(r.entries.len(), MAX_RECORDS);
        assert!(r.entries.iter().all(|(k, _)| k.as_deref() != Some("i0")));
        assert!(r.entries.iter().any(|(k, _)| k.as_deref() == Some("i1")));
    }

    #[test]
    fn labels() {
        let blocked = Record {
            kind: Some(Kind::Permission),
            ..rec(State::Blocked)
        };
        assert_eq!(label(&blocked), "Needs permission");
        let r = Record {
            app: Some("claude-code".into()),
            ..rec(State::Blocked)
        };
        assert_eq!(label(&r), "claude-code: Needs you");
        let r = Record {
            app: Some("claude-code".into()),
            msg: Some("Tests pass".into()),
            ..rec(State::Done)
        };
        assert_eq!(label(&r), "claude-code: Tests pass");
        assert_eq!(label(&rec(State::Error)), "Failed");
    }

    #[test]
    fn bidi_is_stripped_for_display() {
        assert_eq!(display_text("a\u{202E}b\u{200B}c"), "abc");
    }
}
