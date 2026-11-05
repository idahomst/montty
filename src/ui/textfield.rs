//! Single-line text input state for the rename dialog (pure; no drawing).

use crate::input::Mods;
use winit::keyboard::{Key, NamedKey};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldOutcome {
    /// Nothing changed (a shortcut, or an edit with nothing to do).
    Unchanged,
    /// Only the cursor moved.
    Moved,
    /// The text changed.
    Edited,
    Commit(String),
    Cancel,
}

pub struct TextField {
    text: String,
    /// In chars, not bytes.
    cursor: usize,
}

impl TextField {
    /// Control chars in `initial` are dropped, as typed ones are.
    pub fn new(initial: &str) -> Self {
        let text: String = initial.chars().filter(|c| !c.is_control()).collect();
        Self {
            cursor: text.chars().count(),
            text,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Insert `s` at the cursor. The caller has dropped its control chars.
    pub fn insert(&mut self, s: &str) {
        let at = self.byte_at(self.cursor);
        self.text.insert_str(at, s);
        self.cursor += s.chars().count();
    }

    /// Apply one key press. `text` is the event's typed text (with modifiers applied).
    /// The outcome says what changed, so callers need not compare copies.
    pub fn key(&mut self, key: &Key, text: Option<&str>, mods: Mods) -> FieldOutcome {
        // Ctrl/Alt/Super combos are shortcuts, never text or editing.
        if mods.ctrl || mods.alt || mods.logo {
            return FieldOutcome::Unchanged;
        }
        let len = self.text.chars().count();
        let cursor = self.cursor;
        match key {
            Key::Named(NamedKey::Enter) => FieldOutcome::Commit(self.text.trim().to_string()),
            Key::Named(NamedKey::Escape) => FieldOutcome::Cancel,
            Key::Named(NamedKey::Backspace) if cursor > 0 => {
                self.cursor -= 1;
                let at = self.byte_at(self.cursor);
                self.text.remove(at);
                FieldOutcome::Edited
            }
            Key::Named(NamedKey::Delete) if cursor < len => {
                let at = self.byte_at(cursor);
                self.text.remove(at);
                FieldOutcome::Edited
            }
            Key::Named(NamedKey::Backspace | NamedKey::Delete) => FieldOutcome::Unchanged,
            Key::Named(NamedKey::ArrowLeft) => self.move_to(cursor.saturating_sub(1)),
            Key::Named(NamedKey::ArrowRight) => self.move_to((cursor + 1).min(len)),
            Key::Named(NamedKey::Home) => self.move_to(0),
            Key::Named(NamedKey::End) => self.move_to(len),
            _ => match text.filter(|t| !t.is_empty() && !t.chars().any(char::is_control)) {
                Some(t) => {
                    self.insert(t);
                    FieldOutcome::Edited
                }
                None => FieldOutcome::Unchanged,
            },
        }
    }

    fn move_to(&mut self, cursor: usize) -> FieldOutcome {
        if cursor == self.cursor {
            return FieldOutcome::Unchanged;
        }
        self.cursor = cursor;
        FieldOutcome::Moved
    }

    fn byte_at(&self, char_idx: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_idx)
            .map_or(self.text.len(), |(i, _)| i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(f: &mut TextField, n: NamedKey) -> FieldOutcome {
        f.key(&Key::Named(n), None, Mods::default())
    }
    fn typ(f: &mut TextField, s: &str) -> FieldOutcome {
        f.key(&Key::Character(s.into()), Some(s), Mods::default())
    }

    #[test]
    fn field_edits() {
        let mut f = TextField::new("abc");
        assert_eq!(f.cursor(), 3);
        named(&mut f, NamedKey::ArrowLeft);
        named(&mut f, NamedKey::ArrowLeft);
        typ(&mut f, "X");
        assert_eq!(f.text(), "aXbc");
        named(&mut f, NamedKey::Home);
        named(&mut f, NamedKey::Delete);
        assert_eq!(f.text(), "Xbc");
        named(&mut f, NamedKey::End);
        named(&mut f, NamedKey::Backspace);
        assert_eq!(f.text(), "Xb");
        assert_eq!(
            named(&mut f, NamedKey::Enter),
            FieldOutcome::Commit("Xb".into())
        );
        assert_eq!(named(&mut f, NamedKey::Escape), FieldOutcome::Cancel);
    }

    #[test]
    fn field_unicode_cursor() {
        let mut f = TextField::new("中a");
        named(&mut f, NamedKey::ArrowLeft);
        named(&mut f, NamedKey::Backspace);
        assert_eq!(f.text(), "a");
        assert_eq!(f.cursor(), 0);
    }

    #[test]
    fn key_reports_what_changed() {
        let mut f = TextField::new("ab");
        let named = |f: &mut TextField, k| f.key(&Key::Named(k), None, Mods::default());
        assert_eq!(named(&mut f, NamedKey::End), FieldOutcome::Unchanged);
        assert_eq!(named(&mut f, NamedKey::ArrowLeft), FieldOutcome::Moved);
        assert_eq!(named(&mut f, NamedKey::Delete), FieldOutcome::Edited);
        assert_eq!(named(&mut f, NamedKey::Delete), FieldOutcome::Unchanged);
        assert_eq!(named(&mut f, NamedKey::Home), FieldOutcome::Moved);
        assert_eq!(named(&mut f, NamedKey::Backspace), FieldOutcome::Unchanged);
        let typed = f.key(&Key::Character("x".into()), Some("x"), Mods::default());
        assert_eq!(typed, FieldOutcome::Edited);
        assert_eq!((f.text(), f.cursor()), ("xa", 1));
    }

    #[test]
    fn field_ignores_ctrl() {
        let mut f = TextField::new("a");
        let ctrl = Mods {
            ctrl: true,
            ..Default::default()
        };
        let out = f.key(&Key::Character("a".into()), Some("\u{1}"), ctrl);
        assert_eq!(out, FieldOutcome::Unchanged);
        let out = f.key(&Key::Character("a".into()), Some("a"), ctrl);
        assert_eq!(out, FieldOutcome::Unchanged);
        assert_eq!(f.text(), "a");
    }

    #[test]
    fn field_edges_and_control_text() {
        let mut f = TextField::new("");
        named(&mut f, NamedKey::Backspace);
        named(&mut f, NamedKey::Delete);
        named(&mut f, NamedKey::ArrowLeft);
        named(&mut f, NamedKey::ArrowRight);
        f.key(&Key::Named(NamedKey::Tab), Some("\t"), Mods::default());
        assert_eq!(f.text(), "");
        let shift = Mods {
            shift: true,
            ..Default::default()
        };
        f.key(&Key::Character("A".into()), Some("A"), shift);
        assert_eq!(f.text(), "A");
        typ(&mut f, " ");
        assert_eq!(
            named(&mut f, NamedKey::Enter),
            FieldOutcome::Commit("A".into())
        );
    }

    #[test]
    fn field_new_drops_control_chars() {
        let f = TextField::new("a\x1b[1mb\nc\u{7}");
        assert_eq!(f.text(), "a[1mbc");
        assert_eq!(f.cursor(), 6);
    }
}
