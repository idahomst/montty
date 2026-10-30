//! Dropdown menus (behind the ≡ button, a pane overlay's Profile button, and a
//! right-click in a pane):
//! geometry, hit-testing and keyboard navigation (pure).

use super::text::{display_width, sanitize, truncate};
use super::topbar::BarMetrics;
use crate::layout::Rect;
use winit::keyboard::{Key, NamedKey};

/// Narrowest menu, in columns.
pub const MIN_COLS: i32 = 12;

/// Longest profile name in the profile menu, in columns (the mark's two columns included).
pub const MAX_NAME_COLS: usize = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    NewTab,
    NewWindow,
    SaveSession,
    /// Opens the session list (a second menu).
    OpenSessionMenu,
    EditConfig,
    OpenConfigFolder,
    ReloadConfig,
    Shortcuts,
    About,
    /// An index into the `(stem, display name)` list the profile menu was opened with.
    SetProfile(usize),
    /// An index into the sorted session names the session list was opened with.
    OpenSession(usize),
    SplitRight,
    SplitDown,
    SplitLeft,
    SplitUp,
    Copy,
    Paste,
    /// A thin line between groups of rows: never chosen, skipped by the arrow keys.
    Separator,
    /// Opens the profile menu's pane's profile file in an editor.
    EditProfile,
    /// Asks for a name and creates a profile from the profile menu's pane's.
    NewProfile,
    /// A row that only informs (`… N more`): never chosen, skipped by the arrow keys.
    Disabled,
}

/// The prefix marking the current row of the profile menu (the painter draws
/// the mark in ACCENT); the other rows get as many spaces.
pub const CHECK: &str = "✓ ";

/// A profile's display name (user file text) as one menu line: control characters gone,
/// newlines and runs of blanks one space; `fallback` when nothing is left.
fn menu_name(name: &str, fallback: &str) -> String {
    let one_line = sanitize(&name.replace(['\n', '\r', '\t'], " "));
    let words: Vec<&str> = one_line.split_whitespace().collect();
    match words.is_empty() {
        true => sanitize(fallback),
        false => words.join(" "),
    }
}

/// The profile menu's rows: each profile's display name, in `stems` order; the one whose
/// stem is `current` marked with `CHECK`. Names are one line and at most `max_cols`
/// columns (and `MAX_NAME_COLS`) with the mark. With more profiles than `max_rows`, the
/// first `max_rows - 1` show and a disabled `… N more` row ends the list.
pub fn profile_menu_items(
    stems: &[(String, String)],
    current: &str,
    max_rows: usize,
    max_cols: usize,
) -> Vec<MenuItem> {
    let cols = max_cols.min(MAX_NAME_COLS);
    let name_cols = cols.saturating_sub(display_width(CHECK));
    let shown = match stems.len() > max_rows {
        true => max_rows.saturating_sub(1),
        false => stems.len(),
    };
    let mut items: Vec<MenuItem> = stems
        .iter()
        .enumerate()
        .take(shown)
        .map(|(i, (stem, name))| MenuItem {
            label: format!(
                "{}{}",
                if stem == current { CHECK } else { "  " },
                truncate(&menu_name(name, stem), name_cols)
            ),
            shortcut: None,
            action: MenuAction::SetProfile(i),
        })
        .collect();
    if shown < stems.len() {
        items.push(MenuItem {
            label: format!("  … {} more", stems.len() - shown),
            shortcut: None,
            action: MenuAction::Disabled,
        });
    }
    items
}

/// The profile menu's and session list's title rows.
pub const PROFILES_TITLE: &str = "Profiles";
pub const SESSIONS_TITLE: &str = "Sessions";

/// `title` as a dim row and a separator, above `items`.
fn titled(title: &str, items: Vec<MenuItem>) -> Vec<MenuItem> {
    let row = |label: &str, action| MenuItem {
        label: label.into(),
        shortcut: None,
        action,
    };
    let mut out = vec![
        row(title, MenuAction::Disabled),
        row("", MenuAction::Separator),
    ];
    out.extend(items);
    out
}

/// Rows a title takes: its own and a separator (counted whole, so the menu still fits).
const TITLE_ROWS: usize = 2;

/// A pane's profile menu: the `PROFILES_TITLE`, the profiles (see `profile_menu_items`),
/// and with `files` (there is a config folder) `Edit profile "<current>"` (when `current`
/// is one of `stems`) and `New profile…` below a separator. At most `max_rows` rows of
/// `max_cols` columns.
pub fn profile_menu(
    stems: &[(String, String)],
    current: &str,
    files: bool,
    max_rows: usize,
    max_cols: usize,
) -> Vec<MenuItem> {
    let edit = stems
        .iter()
        .find(|(stem, _)| stem == current)
        .filter(|_| files)
        .map(|(stem, name)| {
            let frame = display_width("Edit profile \"\"");
            let name = truncate(
                &menu_name(name, stem),
                max_cols.saturating_sub(frame).min(MAX_NAME_COLS),
            );
            MenuItem {
                label: format!("Edit profile \"{name}\""),
                shortcut: None,
                action: MenuAction::EditProfile,
            }
        });
    let mut tail: Vec<MenuItem> = edit.into_iter().collect();
    if files {
        tail.push(MenuItem {
            label: "New profile…".into(),
            shortcut: None,
            action: MenuAction::NewProfile,
        });
    }
    // The tail's separator and the title's together take less than a row.
    let reserved = TITLE_ROWS + tail.len();
    let mut items = match stems.is_empty() {
        true => Vec::new(),
        false => profile_menu_items(
            stems,
            current,
            max_rows.saturating_sub(reserved).max(1),
            max_cols,
        ),
    };
    if !items.is_empty() && !tail.is_empty() {
        items.push(MenuItem {
            label: String::new(),
            shortcut: None,
            action: MenuAction::Separator,
        });
    }
    items.extend(tail);
    titled(PROFILES_TITLE, items)
}

/// Shown (disabled) when no session is saved.
pub const NO_SESSIONS: &str = "no saved sessions";

/// The session list's rows below the `SESSIONS_TITLE`: each name (sanitized to one line,
/// cut to `max_cols` columns) in `names` order; with more than fit in `max_rows`, the last
/// row is a disabled `… N more`. No names: one disabled row.
pub fn session_menu_items(names: &[String], max_rows: usize, max_cols: usize) -> Vec<MenuItem> {
    titled(
        SESSIONS_TITLE,
        session_rows(names, max_rows.saturating_sub(TITLE_ROWS).max(1), max_cols),
    )
}

fn session_rows(names: &[String], max_rows: usize, max_cols: usize) -> Vec<MenuItem> {
    if names.is_empty() {
        return vec![MenuItem {
            label: NO_SESSIONS.into(),
            shortcut: None,
            action: MenuAction::Disabled,
        }];
    }
    let cols = max_cols.min(MAX_NAME_COLS);
    let shown = match names.len() > max_rows {
        true => max_rows.saturating_sub(1),
        false => names.len(),
    };
    let mut items: Vec<MenuItem> = names
        .iter()
        .enumerate()
        .take(shown)
        .map(|(i, name)| MenuItem {
            label: truncate(&menu_name(name, name), cols),
            shortcut: None,
            action: MenuAction::OpenSession(i),
        })
        .collect();
    if shown < names.len() {
        items.push(MenuItem {
            label: format!("… {} more", names.len() - shown),
            shortcut: None,
            action: MenuAction::Disabled,
        });
    }
    items
}

/// A separator row's height: a third of a row.
pub fn separator_height(bm: &BarMetrics) -> i32 {
    (bm.height / 3).max(1)
}

fn row_height(item: &MenuItem, bm: &BarMetrics) -> i32 {
    match item.action {
        MenuAction::Separator => separator_height(bm),
        _ => bm.height,
    }
}

pub struct MenuItem {
    pub label: String,
    pub shortcut: Option<String>,
    pub action: MenuAction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuOutcome {
    None,
    Chosen(MenuAction),
    Close,
}

pub struct Menu {
    pub items: Vec<MenuItem>,
    anchor_right: i32,
    top: i32,
    selected: Option<usize>,
}

impl Menu {
    /// `anchor_right` is the right edge of the ≡ button; the menu hangs below `top`.
    pub fn new(items: Vec<MenuItem>, anchor_right: i32, top: i32) -> Self {
        Self {
            items,
            anchor_right,
            top,
            selected: None,
        }
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Wide enough for the longest "label  shortcut" row; right edge on the anchor.
    pub fn rect(&self, bm: &BarMetrics) -> Rect {
        let cols = self
            .items
            .iter()
            .map(|i| {
                let sc = i.shortcut.as_deref().map_or(0, |s| 2 + display_width(s));
                (display_width(&i.label) + sc) as i32
            })
            .max()
            .unwrap_or(0)
            .max(MIN_COLS);
        let w = cols * bm.cell_w + 2 * bm.pad;
        Rect {
            x: self.anchor_right - w,
            y: self.top,
            w,
            h: self.items.iter().map(|i| row_height(i, bm)).sum(),
        }
    }

    pub fn item_rects(&self, bm: &BarMetrics) -> Vec<Rect> {
        let r = self.rect(bm);
        let mut y = r.y;
        self.items
            .iter()
            .map(|i| {
                let h = row_height(i, bm);
                y += h;
                Rect {
                    x: r.x,
                    y: y - h,
                    w: r.w,
                    h,
                }
            })
            .collect()
    }

    fn item_at(&self, x: i32, y: i32, bm: &BarMetrics) -> Option<usize> {
        self.item_rects(bm).iter().position(|r| r.contains(x, y))
    }

    fn enabled(&self, i: usize) -> bool {
        !matches!(
            self.items[i].action,
            MenuAction::Disabled | MenuAction::Separator
        )
    }

    /// The next enabled row from `from` going `step` (1 or n - 1) rows at a time, wrapping;
    /// None when every row is disabled.
    fn step_from(&self, from: usize, step: usize) -> Option<usize> {
        let n = self.items.len();
        let mut i = from;
        for _ in 0..n {
            i = (i + step) % n;
            if self.enabled(i) {
                return Some(i);
            }
        }
        None
    }

    /// Pointer moved: the item under it becomes selected. Off the rows the
    /// selection stays (it may have come from the keyboard).
    pub fn hover(&mut self, x: i32, y: i32, bm: &BarMetrics) {
        if let Some(i) = self.item_at(x, y, bm).filter(|&i| self.enabled(i)) {
            self.selected = Some(i);
        }
    }

    pub fn click(&self, x: i32, y: i32, bm: &BarMetrics) -> MenuOutcome {
        if let Some(i) = self.item_at(x, y, bm) {
            match self.enabled(i) {
                true => MenuOutcome::Chosen(self.items[i].action),
                false => MenuOutcome::None,
            }
        } else if self.rect(bm).contains(x, y) {
            MenuOutcome::None
        } else {
            MenuOutcome::Close
        }
    }

    pub fn key(&mut self, key: &Key) -> MenuOutcome {
        let n = self.items.len();
        match key {
            Key::Named(NamedKey::Escape) => return MenuOutcome::Close,
            // From nothing, step from just before the first row / just after the last.
            Key::Named(NamedKey::ArrowDown) if n > 0 => {
                self.selected = self
                    .step_from(self.selected.unwrap_or(n - 1), 1)
                    .or(self.selected);
            }
            Key::Named(NamedKey::ArrowUp) if n > 0 => {
                self.selected = self
                    .step_from(self.selected.unwrap_or(0), n - 1)
                    .or(self.selected);
            }
            Key::Named(NamedKey::Enter) => {
                if let Some(item) = self.selected.and_then(|i| self.items.get(i)) {
                    return MenuOutcome::Chosen(item.action);
                }
            }
            _ => {}
        }
        MenuOutcome::None
    }
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

    fn menu() -> Menu {
        let item = |l: &str, s: Option<&str>, a| MenuItem {
            label: l.into(),
            shortcut: s.map(Into::into),
            action: a,
        };
        Menu::new(
            vec![
                item("New tab", Some("Ctrl+Shift+T"), MenuAction::NewTab),
                item("Keyboard shortcuts", None, MenuAction::Shortcuts),
                item("About", None, MenuAction::About),
            ],
            400,
            28,
        )
    }

    fn down() -> Key {
        Key::Named(NamedKey::ArrowDown)
    }
    fn up() -> Key {
        Key::Named(NamedKey::ArrowUp)
    }

    #[test]
    fn menu_keyboard_nav_wraps() {
        let mut m = menu();
        assert_eq!(m.key(&down()), MenuOutcome::None);
        assert_eq!(m.selected(), Some(0));
        m.key(&up());
        assert_eq!(m.selected(), Some(2));
        assert_eq!(
            m.key(&Key::Named(NamedKey::Enter)),
            MenuOutcome::Chosen(MenuAction::About)
        );
        m.key(&down());
        assert_eq!(m.selected(), Some(0));
        assert_eq!(m.key(&Key::Named(NamedKey::Escape)), MenuOutcome::Close);
    }

    #[test]
    fn menu_up_from_nothing_selects_last_and_enter_needs_selection() {
        let mut m = menu();
        assert_eq!(m.key(&Key::Named(NamedKey::Enter)), MenuOutcome::None);
        m.key(&up());
        assert_eq!(m.selected(), Some(2));
    }

    #[test]
    fn menu_click_inside_and_outside() {
        let m = menu();
        let bm = bm();
        let rs = m.item_rects(&bm);
        assert_eq!(rs.len(), 3);
        let r = rs[1];
        assert_eq!(
            m.click(r.x + 2, r.y + 2, &bm),
            MenuOutcome::Chosen(MenuAction::Shortcuts)
        );
        assert_eq!(m.click(0, 500, &bm), MenuOutcome::Close);
        let outer = m.rect(&bm);
        assert_eq!(m.click(outer.x - 1, outer.y + 2, &bm), MenuOutcome::Close);
    }

    #[test]
    fn menu_hover_selects() {
        let mut m = menu();
        let bm = bm();
        let r = m.item_rects(&bm)[2];
        m.hover(r.x + 1, r.y + 1, &bm);
        assert_eq!(m.selected(), Some(2));
        // Off the rows the (keyboard) selection stays.
        m.hover(0, 0, &bm);
        assert_eq!(m.selected(), Some(2));
        m.key(&up());
        m.hover(0, 500, &bm);
        assert_eq!(m.selected(), Some(1));
    }

    #[test]
    fn menu_aligned_to_anchor() {
        let m = menu();
        let bm = bm();
        let r = m.rect(&bm);
        assert_eq!(r.x + r.w, 400);
        assert_eq!(r.y, 28);
        assert_eq!(r.h, 3 * bm.height);
        // "Keyboard shortcuts" = 18 cols; "New tab" + 2 + "Ctrl+Shift+T" = 21 cols.
        assert_eq!(r.w, 21 * bm.cell_w + 2 * bm.pad);
    }

    #[test]
    fn menu_enter_with_stale_selection_is_none() {
        let mut m = menu();
        m.key(&up());
        assert_eq!(m.selected(), Some(2));
        m.items.pop();
        assert_eq!(m.key(&Key::Named(NamedKey::Enter)), MenuOutcome::None);
    }

    #[test]
    fn menu_has_minimum_width() {
        let m = Menu::new(
            vec![MenuItem {
                label: "A".into(),
                shortcut: None,
                action: MenuAction::About,
            }],
            100,
            0,
        );
        assert_eq!(m.rect(&bm()).w, 12 * 8 + 12);
    }

    #[test]
    fn profile_menu_marks_current() {
        let stems = [
            ("dark".to_string(), "Dark".to_string()),
            ("default".to_string(), "Default".to_string()),
            ("x".to_string(), "Solarized light".to_string()),
        ];
        let items = profile_menu_items(&stems, "default", 99, 40);
        let rows: Vec<(&str, Option<&str>, MenuAction)> = items
            .iter()
            .map(|i| (i.label.as_str(), i.shortcut.as_deref(), i.action))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("  Dark", None, MenuAction::SetProfile(0)),
                ("✓ Default", None, MenuAction::SetProfile(1)),
                ("  Solarized light", None, MenuAction::SetProfile(2)),
            ]
        );
        assert!(items[1].label.starts_with(CHECK));
        // An unknown current stem marks nothing.
        let items = profile_menu_items(&stems, "gone", 99, 40);
        assert!(items.iter().all(|i| i.label.starts_with("  ")));
        assert!(profile_menu_items(&[], "default", 99, 40).is_empty());
    }

    fn named(names: &[&str]) -> Vec<(String, String)> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| (format!("s{i}"), n.to_string()))
            .collect()
    }

    fn labels(items: &[MenuItem]) -> Vec<&str> {
        items.iter().map(|i| i.label.as_str()).collect()
    }

    #[test]
    fn profile_names_are_one_clean_line() {
        let stems = named(&[
            "Two\nlines",
            "esc\x1b[31m red",
            "  spaced   out  ",
            "\x07\n",
        ]);
        let items = profile_menu_items(&stems, "none", 99, 40);
        // Controls go, newlines and runs of blanks become one space; an empty name
        // shows the stem.
        assert_eq!(
            labels(&items),
            ["  Two lines", "  esc[31m red", "  spaced out", "  s3"]
        );
    }

    #[test]
    fn profile_names_are_cut_to_the_width() {
        let long = "x".repeat(100);
        let stems = named(&[&long, "short"]);
        let items = profile_menu_items(&stems, "s0", 99, MAX_NAME_COLS);
        assert_eq!(display_width(&items[0].label), MAX_NAME_COLS);
        assert!(items[0].label.starts_with(CHECK) && items[0].label.ends_with('…'));
        assert_eq!(items[1].label, "  short");
        // A narrow window cuts further; the label never exceeds `max_cols`.
        let items = profile_menu_items(&stems, "s0", 99, 12);
        assert_eq!(display_width(&items[0].label), 12);
        // Wide chars count by columns.
        let wide = named(&[&"漢".repeat(30)]);
        let items = profile_menu_items(&wide, "none", 99, 20);
        assert!(display_width(&items[0].label) <= 20);
    }

    #[test]
    fn too_many_profiles_end_in_a_disabled_row() {
        let stems = named(&["a", "b", "c", "d", "e"]);
        // Fits: no extra row.
        assert_eq!(profile_menu_items(&stems, "s0", 5, 40).len(), 5);
        // 3 rows: the first 2 plus "… 3 more".
        let items = profile_menu_items(&stems, "s0", 3, 40);
        assert_eq!(labels(&items), ["✓ a", "  b", "  … 3 more"]);
        assert_eq!(items[1].action, MenuAction::SetProfile(1));
        assert_eq!(items[2].action, MenuAction::Disabled);
        // One row: only the note.
        let items = profile_menu_items(&stems, "s0", 1, 40);
        assert_eq!(labels(&items), ["  … 5 more"]);
    }

    #[test]
    fn disabled_rows_do_nothing() {
        let item = |l: &str, a| MenuItem {
            label: l.into(),
            shortcut: None,
            action: a,
        };
        let mut m = Menu::new(
            vec![
                item("a", MenuAction::SetProfile(0)),
                item("… 2 more", MenuAction::Disabled),
            ],
            400,
            28,
        );
        // Arrow keys skip it, hover doesn't select it, a click keeps the menu open.
        m.key(&down());
        m.key(&down());
        assert_eq!(m.selected(), Some(0));
        m.key(&up());
        assert_eq!(m.selected(), Some(0));
        let rows = m.item_rects(&bm());
        m.hover(rows[1].x + 1, rows[1].y + 1, &bm());
        assert_eq!(m.selected(), Some(0));
        assert_eq!(
            m.click(rows[1].x + 1, rows[1].y + 1, &bm()),
            MenuOutcome::None
        );
    }

    #[test]
    fn separators_are_short_and_inert() {
        let item = |l: &str, a| MenuItem {
            label: l.into(),
            shortcut: None,
            action: a,
        };
        let mut m = Menu::new(
            vec![
                item("a", MenuAction::NewTab),
                item("", MenuAction::Separator),
                item("b", MenuAction::About),
            ],
            400,
            28,
        );
        let bm = bm();
        let rows = m.item_rects(&bm);
        let sep = separator_height(&bm);
        assert!(sep > 0 && sep < bm.height);
        assert_eq!(rows[1].h, sep);
        assert_eq!(rows[1].y, rows[0].y + bm.height);
        assert_eq!(rows[2].y, rows[1].y + sep);
        assert_eq!(m.rect(&bm).h, 2 * bm.height + sep);
        // Arrow keys step over it, hover doesn't select it, a click keeps the menu open.
        m.key(&down());
        m.key(&down());
        assert_eq!(m.selected(), Some(2));
        m.key(&up());
        assert_eq!(m.selected(), Some(0));
        m.hover(rows[1].x + 1, rows[1].y + 1, &bm);
        assert_eq!(m.selected(), Some(0));
        assert_eq!(
            m.click(rows[1].x + 1, rows[1].y + 1, &bm),
            MenuOutcome::None
        );
        assert_eq!(
            m.click(rows[2].x + 1, rows[2].y + 1, &bm),
            MenuOutcome::Chosen(MenuAction::About)
        );
    }

    fn rows(items: &[MenuItem]) -> Vec<(&str, MenuAction)> {
        items.iter().map(|i| (i.label.as_str(), i.action)).collect()
    }

    #[test]
    fn profile_menu_has_a_title_and_edit_rows() {
        let stems = [
            ("default".to_string(), "Default".to_string()),
            ("work".to_string(), "Work".to_string()),
        ];
        assert_eq!(
            rows(&profile_menu(&stems, "work", true, 99, 40)),
            [
                (PROFILES_TITLE, MenuAction::Disabled),
                ("", MenuAction::Separator),
                ("  Default", MenuAction::SetProfile(0)),
                ("✓ Work", MenuAction::SetProfile(1)),
                ("", MenuAction::Separator),
                ("Edit profile \"Work\"", MenuAction::EditProfile),
                ("New profile…", MenuAction::NewProfile),
            ]
        );
        // No config folder: nothing to edit or create.
        assert_eq!(
            rows(&profile_menu(&stems, "work", false, 99, 40)),
            [
                (PROFILES_TITLE, MenuAction::Disabled),
                ("", MenuAction::Separator),
                ("  Default", MenuAction::SetProfile(0)),
                ("✓ Work", MenuAction::SetProfile(1)),
            ]
        );
        // No profile files (built-in defaults only): only New profile.
        assert_eq!(
            rows(&profile_menu(&[], "default", true, 99, 40)),
            [
                (PROFILES_TITLE, MenuAction::Disabled),
                ("", MenuAction::Separator),
                ("New profile…", MenuAction::NewProfile),
            ]
        );
    }

    #[test]
    fn profile_menu_edit_row_fits_and_the_list_shrinks_for_it() {
        let long = "w".repeat(100);
        let stems = named(&[&long, "b", "c", "d", "e", "f", "g", "h"]);
        let items = profile_menu(&stems, "s0", true, 99, 30);
        let edit = items
            .iter()
            .find(|i| i.action == MenuAction::EditProfile)
            .unwrap();
        assert!(edit.label.starts_with("Edit profile \"w"));
        assert!(edit.label.ends_with("…\""), "{}", edit.label);
        assert_eq!(display_width(&edit.label), 30);
        // 8 rows: title, two separators (a row between them), Edit and New leave 4 for
        // the list: 3 profiles and "… 5 more".
        let items = profile_menu(&stems, "s0", true, 8, 40);
        let listed = items
            .iter()
            .filter(|i| matches!(i.action, MenuAction::SetProfile(_)))
            .count();
        assert_eq!(listed, 3);
        assert!(items.iter().any(|i| i.label.contains("5 more")));
    }

    #[test]
    fn session_list_rows() {
        let names: Vec<String> = ["a", "b", "c"].map(String::from).into();
        let rows = |items: &[MenuItem]| -> Vec<(String, MenuAction)> {
            items.iter().map(|i| (i.label.clone(), i.action)).collect()
        };
        let title = [
            (SESSIONS_TITLE.to_string(), MenuAction::Disabled),
            (String::new(), MenuAction::Separator),
        ];
        let titled = |items: Vec<MenuItem>| {
            assert_eq!(rows(&items[..2]), title);
            rows(&items[2..])
        };
        assert_eq!(
            titled(session_menu_items(&names, 99, 40)),
            [
                ("a".to_string(), MenuAction::OpenSession(0)),
                ("b".to_string(), MenuAction::OpenSession(1)),
                ("c".to_string(), MenuAction::OpenSession(2)),
            ]
        );
        // Bounded: the last row says how many are hidden; the title takes two rows.
        assert_eq!(
            titled(session_menu_items(&names, 4, 40)),
            [
                ("a".to_string(), MenuAction::OpenSession(0)),
                ("… 2 more".to_string(), MenuAction::Disabled),
            ]
        );
        // None saved: one disabled row.
        assert_eq!(
            titled(session_menu_items(&[], 5, 40)),
            [(NO_SESSIONS.to_string(), MenuAction::Disabled)]
        );
        // Long names are cut.
        let long = vec!["x".repeat(100)];
        let items = session_menu_items(&long, 5, 20);
        assert_eq!(display_width(&items[2].label), 20);
    }
}
