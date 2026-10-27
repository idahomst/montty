//! Single-line text input state for the rename dialog (pure; no drawing).

use crate::input::Mods;
use winit::keyboard::{Key, NamedKey};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldOutcome {
    Editing,
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
    pub fn key(&mut self, key: &Key, text: Option<&str>, mods: Mods) -> FieldOutcome {
        // Ctrl/Alt/Super combos are shortcuts, never text or editing.
        if mods.ctrl || mods.alt || mods.logo {
            return FieldOutcome::Editing;
        }
        let len = self.text.chars().count();
        match key {
            Key::Named(NamedKey::Enter) => {
                return FieldOutcome::Commit(self.text.trim().to_string());
            }
            Key::Named(NamedKey::Escape) => return FieldOutcome::Cancel,
            Key::Named(NamedKey::Backspace) => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    let at = self.byte_at(self.cursor);
                    self.text.remove(at);
                }
            }
            Key::Named(NamedKey::Delete) => {
                if self.cursor < len {
                    let at = self.byte_at(self.cursor);
                    self.text.remove(at);
                }
            }
            Key::Named(NamedKey::ArrowLeft) => self.cursor = self.cursor.saturating_sub(1),
            Key::Named(NamedKey::ArrowRight) => self.cursor = (self.cursor + 1).min(len),
            Key::Named(NamedKey::Home) => self.cursor = 0,
            Key::Named(NamedKey::End) => self.cursor = len,
            _ => {
                if let Some(t) = text.filter(|t| !t.is_empty() && !t.chars().any(char::is_control))
                {
                    let at = self.byte_at(self.cursor);
                    self.text.insert_str(at, t);
                    self.cursor += t.chars().count();
                }
            }
        }
        FieldOutcome::Editing
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
    fn field_ignores_ctrl() {
        let mut f = TextField::new("a");
        let ctrl = Mods {
            ctrl: true,
            ..Default::default()
        };
        let out = f.key(&Key::Character("a".into()), Some("\u{1}"), ctrl);
        assert_eq!(out, FieldOutcome::Editing);
        let out = f.key(&Key::Character("a".into()), Some("a"), ctrl);
        assert_eq!(out, FieldOutcome::Editing);
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
