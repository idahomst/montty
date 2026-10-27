//! Find bar state and layout (pure; the search itself is `window/find.rs`).

use super::textfield::{FieldOutcome, TextField};
use super::topbar::BarMetrics;
use crate::input::Mods;
use crate::layout::Rect;
use winit::keyboard::{Key, NamedKey};

/// Escaped in the query so the search stays literal.
const META: &str = r"\.+*?()|[]{}^$#&-~";

pub struct FindBar {
    pub field: TextField,
    pub match_case: bool,
    pub no_match: bool,
    /// The field's whole text is selected: the next edit replaces it.
    pub select_all: bool,
}

/// What a key did to the bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindKey {
    Nothing,
    /// Only the cursor or the selection changed.
    Redraw,
    /// The query text changed: search again.
    Edited,
    Next,
    Prev,
    Close,
}

impl Default for FindBar {
    fn default() -> Self {
        Self::new()
    }
}

impl FindBar {
    pub fn new() -> Self {
        Self {
            field: TextField::new(""),
            match_case: false,
            no_match: false,
            select_all: false,
        }
    }

    /// The regex for the query: `None` when it is empty, else the query with every
    /// metacharacter escaped (the search is literal), under `(?i)` or `(?-i)`.
    pub fn pattern(&self) -> Option<String> {
        let query = self.field.text();
        if query.is_empty() {
            return None;
        }
        let mut out = String::from(if self.match_case { "(?-i)" } else { "(?i)" });
        for c in query.chars() {
            if META.contains(c) {
                out.push('\\');
            }
            out.push(c);
        }
        Some(out)
    }

    /// Insert pasted text at the cursor (replacing a selected query): the first line
    /// only, control characters dropped. True when the field's text changed.
    pub fn paste(&mut self, text: &str) -> bool {
        let line = text.split(['\r', '\n']).next().unwrap_or("");
        let line: String = line.chars().filter(|c| !c.is_control()).collect();
        if line.is_empty() {
            return false;
        }
        if self.select_all {
            self.field = TextField::new("");
            self.select_all = false;
        }
        self.field.insert(&line);
        true
    }

    /// Apply one key press to the field. Enter is next (Shift+Enter previous) and
    /// Escape closes; with the text selected, an edit replaces all of it.
    pub fn key(&mut self, key: &Key, text: Option<&str>, mods: Mods) -> FindKey {
        let replaces = self.select_all && replaces_selection(key, text, mods);
        let before = (self.field.text().to_string(), self.field.cursor());
        if replaces {
            self.field = TextField::new("");
        }
        let out = self.field.key(key, text, mods);
        match out {
            FieldOutcome::Commit(_) if mods.shift => return FindKey::Prev,
            FieldOutcome::Commit(_) => return FindKey::Next,
            FieldOutcome::Cancel => return FindKey::Close,
            FieldOutcome::Editing => {}
        }
        let moves = matches!(
            key,
            Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowRight | NamedKey::Home | NamedKey::End)
        ) && !(mods.ctrl || mods.alt || mods.logo);
        let selection_dropped = self.select_all && (replaces || moves);
        if selection_dropped {
            self.select_all = false;
        }
        if self.field.text() != before.0 {
            FindKey::Edited
        } else if selection_dropped || self.field.cursor() != before.1 {
            FindKey::Redraw
        } else {
            FindKey::Nothing
        }
    }
}

/// Whether this key edits the field's text (so it replaces a selected query).
fn replaces_selection(key: &Key, text: Option<&str>, mods: Mods) -> bool {
    if mods.ctrl || mods.alt || mods.logo {
        return false;
    }
    match key {
        Key::Named(NamedKey::Backspace | NamedKey::Delete) => true,
        Key::Named(NamedKey::Enter | NamedKey::Escape) => false,
        _ => text.is_some_and(|t| !t.is_empty() && !t.chars().any(char::is_control)),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindTarget {
    Field,
    Prev,
    Next,
    Case,
    Close,
}

/// The field's width in cells when the pane has room.
const FIELD_CELLS: i32 = 40;

#[derive(Clone, Debug, PartialEq)]
pub struct FindLayout {
    pub rect: Rect,
    pub field: Rect,
    pub buttons: Vec<(Rect, FindTarget)>,
}

/// The bar in `pane`: `bm.height` tall at its top-right, `inset` px in from the corner;
/// the field first, then Prev, Next, Case and Close, each a `bm.height` square.
pub fn findbar_layout(pane: Rect, bm: &BarMetrics, inset: i32) -> FindLayout {
    let h = bm.height;
    let w = (pane.w - 2 * inset)
        .min(FIELD_CELLS * bm.cell_w + 4 * h)
        .max(0);
    if w == 0 || pane.h < h + 2 * inset {
        let zero = Rect {
            x: pane.x,
            y: pane.y,
            w: 0,
            h: 0,
        };
        return FindLayout {
            rect: zero,
            field: zero,
            buttons: Vec::new(),
        };
    }
    let rect = Rect {
        x: pane.x + pane.w - inset - w,
        y: pane.y + inset,
        w,
        h,
    };
    let fw = (w - 4 * h).max(0);
    let field = Rect { w: fw, ..rect };
    // In a pane too narrow for all four, the leftmost don't fit and are left out: Close
    // (then Case) stay.
    let buttons = [
        FindTarget::Prev,
        FindTarget::Next,
        FindTarget::Case,
        FindTarget::Close,
    ]
    .into_iter()
    .enumerate()
    .map(|(i, t)| {
        let r = Rect {
            x: rect.x + rect.w - (4 - i as i32) * h,
            w: h,
            ..rect
        };
        (r, t)
    })
    .filter(|(r, _)| r.x >= rect.x)
    .collect();
    FindLayout {
        rect,
        field,
        buttons,
    }
}

/// What a press at (x, y) hits: a button, else the field (any other part of the bar).
pub fn findbar_hit(l: &FindLayout, x: i32, y: i32) -> Option<FindTarget> {
    l.buttons
        .iter()
        .find(|(r, _)| r.contains(x, y))
        .map(|&(_, t)| t)
        .or_else(|| l.rect.contains(x, y).then_some(FindTarget::Field))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bm() -> BarMetrics {
        BarMetrics {
            height: 28,
            pad: 6,
            cell_w: 8,
            cell_h: 16,
        }
    }

    fn pane(w: i32) -> Rect {
        Rect {
            x: 100,
            y: 30,
            w,
            h: 300,
        }
    }

    fn bar_with(text: &str) -> FindBar {
        FindBar {
            field: TextField::new(text),
            ..FindBar::new()
        }
    }

    #[test]
    fn paste_inserts_the_first_line_at_the_cursor() {
        let mut b = bar_with("ac");
        b.field.key(
            &Key::Named(NamedKey::ArrowLeft),
            None,
            crate::input::Mods::default(),
        );
        assert!(b.paste("b\x1b[31mX\r\nsecond\nthird"));
        assert_eq!(b.field.text(), "ab[31mXc");
        assert_eq!(b.field.cursor(), 7);
        assert!(!b.paste("\n\tonly later lines"));
    }

    #[test]
    fn paste_replaces_a_selected_query() {
        let mut b = bar_with("old");
        b.select_all = true;
        assert!(b.paste("new"));
        assert_eq!(b.field.text(), "new");
        assert!(!b.select_all);
        // Nothing usable to paste: the selection stays.
        b.select_all = true;
        assert!(!b.paste("\n"));
        assert!(b.select_all);
        assert_eq!(b.field.text(), "new");
    }

    #[test]
    fn pattern_escapes_metachars() {
        assert_eq!(
            bar_with("a.b(c)").pattern().as_deref(),
            Some(r"(?i)a\.b\(c\)")
        );
        assert_eq!(
            bar_with(r"\.+*?()|[]{}^$#&-~").pattern().as_deref(),
            Some(r"(?i)\\\.\+\*\?\(\)\|\[\]\{\}\^\$\#\&\-\~")
        );
    }

    #[test]
    fn pattern_case_flag() {
        let mut b = bar_with("Ab");
        assert_eq!(b.pattern().as_deref(), Some("(?i)Ab"));
        b.match_case = true;
        assert_eq!(b.pattern().as_deref(), Some("(?-i)Ab"));
    }

    #[test]
    fn pattern_empty_is_none() {
        assert_eq!(bar_with("").pattern(), None);
    }

    #[test]
    fn layout_fits_small_pane() {
        let l = findbar_layout(pane(200), &bm(), 4);
        assert!(l.rect.w <= 200 - 8, "{l:?}");
        assert!(l.field.w >= 0);
        assert_eq!((l.rect.h, l.rect.y), (28, 34));
        assert_eq!(l.rect.x + l.rect.w, 100 + 200 - 4);
        // Narrower than the four buttons: the field is empty, no button sticks out.
        let l = findbar_layout(pane(100), &bm(), 4);
        assert!(l.field.w >= 0 && l.rect.w <= 92);
        for (r, _) in &l.buttons {
            assert!(r.x >= l.rect.x && r.x + r.w <= l.rect.x + l.rect.w);
        }
    }

    #[test]
    fn narrow_pane_drops_buttons_from_the_left() {
        // Bar 3 buttons wide (84 px): Prev goes, the field is empty.
        let l = findbar_layout(pane(84 + 8), &bm(), 4);
        let targets: Vec<_> = l.buttons.iter().map(|b| b.1).collect();
        assert_eq!(
            targets,
            [FindTarget::Next, FindTarget::Case, FindTarget::Close]
        );
        assert_eq!(l.field.w, 0);
        // One button wide: only Close.
        let l = findbar_layout(pane(28 + 8), &bm(), 4);
        let targets: Vec<_> = l.buttons.iter().map(|b| b.1).collect();
        assert_eq!(targets, [FindTarget::Close]);
        let (r, _) = l.buttons[0];
        assert_eq!(r.x + r.w, l.rect.x + l.rect.w);
    }

    #[test]
    fn layout_is_zero_size_when_the_pane_has_no_room() {
        // Too narrow (no width left after the insets) or too short.
        assert_eq!(findbar_layout(pane(8), &bm(), 4).rect.w, 0);
        let short = Rect { h: 30, ..pane(800) };
        let l = findbar_layout(short, &bm(), 4);
        assert_eq!((l.rect.w, l.rect.h), (0, 0));
        assert!(l.buttons.is_empty());
    }

    #[test]
    fn layout_wide_pane_caps_the_width() {
        let l = findbar_layout(pane(2000), &bm(), 4);
        assert_eq!(l.rect.w, 40 * 8 + 4 * 28);
        assert_eq!(l.field.w, 40 * 8);
    }

    #[test]
    fn layout_buttons_in_order() {
        let l = findbar_layout(pane(800), &bm(), 4);
        let targets: Vec<_> = l.buttons.iter().map(|b| b.1).collect();
        assert_eq!(
            targets,
            [
                FindTarget::Prev,
                FindTarget::Next,
                FindTarget::Case,
                FindTarget::Close
            ]
        );
        assert!(
            l.buttons
                .windows(2)
                .all(|w| w[0].0.x + w[0].0.w == w[1].0.x)
        );
        assert_eq!(l.field.x + l.field.w, l.buttons[0].0.x);
        let last = l.buttons[3].0;
        assert_eq!(last.x + last.w, l.rect.x + l.rect.w);
    }

    #[test]
    fn hit_buttons_then_field() {
        let l = findbar_layout(pane(800), &bm(), 4);
        let c = |r: Rect| (r.x + 1, r.y + 1);
        for (r, t) in &l.buttons {
            let (x, y) = c(*r);
            assert_eq!(findbar_hit(&l, x, y), Some(*t));
        }
        let (x, y) = c(l.field);
        assert_eq!(findbar_hit(&l, x, y), Some(FindTarget::Field));
        assert_eq!(findbar_hit(&l, l.rect.x - 1, l.rect.y), None);
        assert_eq!(findbar_hit(&l, l.rect.x, l.rect.y + l.rect.h), None);
    }

    fn typed(b: &mut FindBar, s: &str) -> FindKey {
        b.key(&Key::Character(s.into()), Some(s), Mods::default())
    }

    fn named(b: &mut FindBar, n: NamedKey, mods: Mods) -> FindKey {
        b.key(&Key::Named(n), None, mods)
    }

    #[test]
    fn keys_edit_navigate_and_close() {
        let mut b = FindBar::new();
        assert_eq!(typed(&mut b, "a"), FindKey::Edited);
        assert_eq!(
            named(&mut b, NamedKey::ArrowLeft, Mods::default()),
            FindKey::Redraw
        );
        assert_eq!(
            named(&mut b, NamedKey::ArrowLeft, Mods::default()),
            FindKey::Nothing
        );
        assert_eq!(
            named(&mut b, NamedKey::Enter, Mods::default()),
            FindKey::Next
        );
        let shift = Mods {
            shift: true,
            ..Default::default()
        };
        assert_eq!(named(&mut b, NamedKey::Enter, shift), FindKey::Prev);
        assert_eq!(
            named(&mut b, NamedKey::Escape, Mods::default()),
            FindKey::Close
        );
        assert_eq!(
            named(&mut b, NamedKey::Backspace, Mods::default()),
            FindKey::Nothing
        );
        assert_eq!(
            named(&mut b, NamedKey::Delete, Mods::default()),
            FindKey::Edited
        );
        assert_eq!(b.field.text(), "");
    }

    #[test]
    fn typing_replaces_a_selected_query() {
        let mut b = bar_with("old");
        b.select_all = true;
        assert_eq!(typed(&mut b, "x"), FindKey::Edited);
        assert_eq!(b.field.text(), "x");
        assert!(!b.select_all);

        let mut b = bar_with("old");
        b.select_all = true;
        assert_eq!(
            named(&mut b, NamedKey::Backspace, Mods::default()),
            FindKey::Edited
        );
        assert_eq!(b.field.text(), "");

        // Arrows only drop the selection; Enter keeps it.
        let mut b = bar_with("old");
        b.select_all = true;
        assert_eq!(
            named(&mut b, NamedKey::Enter, Mods::default()),
            FindKey::Next
        );
        assert!(b.select_all);
        assert_eq!(
            named(&mut b, NamedKey::End, Mods::default()),
            FindKey::Redraw
        );
        assert!(!b.select_all);
        assert_eq!(b.field.text(), "old");
    }
}
