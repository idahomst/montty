//! The modals' content: pure logic for menus, panels, prompts and the close confirmation.

use std::collections::HashMap;

use unicode_width::UnicodeWidthChar;
use winit::keyboard::{Key, NamedKey};

use crate::input::bindings::{Action, Bindings, KeyCombo};
use crate::layout::{PaneId, Rect};
use crate::ui::confirm::{CloseTarget, confirm_message, panes_message};
use crate::ui::menu::{MIN_COLS, Menu, MenuAction, MenuItem};
use crate::ui::panel::InfoPanel;
use crate::ui::topbar::BarMetrics;
use crate::window::tabs::Tabs;

/// What a profile menu switches: `pane`, to one of `stems` (`Store::stems` when
/// it opened, so a reload meanwhile can't shift the rows' indices).
pub(in crate::window) struct ProfileMenu {
    pub pane: PaneId,
    pub stems: Vec<(String, String)>,
}

/// The top-left corner of a `w` × `h` menu opened from `button` in a `win_w` ×
/// `win_h` window: under the button, its left edge on the button's; moved left
/// to end inside the window (but not past x = 0), and above the button when it
/// would overflow the bottom (from y = 0 when it fits neither way).
pub(in crate::window) fn profile_menu_origin(
    button: Rect,
    w: i32,
    h: i32,
    win_w: i32,
    win_h: i32,
) -> (i32, i32) {
    let x = button.x.min(win_w - w).max(0);
    let below = button.y + button.h;
    let y = if below + h <= win_h {
        below
    } else {
        (button.y - h).max(0)
    };
    (x, y)
}

/// How many rows and columns a profile menu opened from `button` may have in a `win_w` ×
/// `win_h` window: the larger of the room below and above the button, and the window's
/// width less the padding (at least one row and the menu's minimum width).
pub(in crate::window) fn profile_menu_limits(
    button: Rect,
    win_w: i32,
    win_h: i32,
    bm: &BarMetrics,
) -> (usize, usize) {
    let room = (win_h - (button.y + button.h)).max(button.y).max(0);
    let rows = (room / bm.height.max(1)).max(1) as usize;
    let cols = ((win_w - 2 * bm.pad) / bm.cell_w.max(1)).max(MIN_COLS);
    (rows, cols as usize)
}

/// The stem profile menu row `i` stands for, unless that profile is gone (`known` says
/// whether one exists): the menu keeps the list it opened with, so a reload meanwhile can
/// delete it.
pub(in crate::window) fn profile_choice(
    stems: &[(String, String)],
    i: usize,
    known: impl Fn(&str) -> bool,
) -> Option<&str> {
    let (stem, _) = stems.get(i)?;
    known(stem).then_some(stem.as_str())
}

/// Closing `target` closes the window: `Window`, or `Tab` of the only tab.
pub(in crate::window) fn closes_window(target: &CloseTarget, tabs: &Tabs) -> bool {
    match target {
        CloseTarget::Window => true,
        CloseTarget::Tab(_) => tabs.len() == 1,
        CloseTarget::Pane(_) => false,
    }
}

/// The one prompt for closing `target`, or None to close at once: a running
/// process asks about it; else closing the window asks about its several panes
/// (when `confirm_window`). Never both.
pub(in crate::window) fn close_prompt(
    target: &CloseTarget,
    tabs: &Tabs,
    running: usize,
    confirm_window: bool,
) -> Option<String> {
    if running > 0 {
        return Some(confirm_message(running));
    }
    if !(confirm_window && closes_window(target, tabs)) {
        return None;
    }
    let total: usize = tabs.iter().map(|t| t.tree.panes().len()).sum();
    (total > 1).then(|| panes_message(total))
}

/// Whether a close prompt's target still exists: the pane (in a tab), the
/// tab holding the pane, or the window (any tab left).
pub(in crate::window) fn target_alive<P>(
    target: &CloseTarget,
    tabs: &Tabs,
    panes: &HashMap<PaneId, P>,
) -> bool {
    match *target {
        CloseTarget::Pane(id) => panes.contains_key(&id) && tabs.tab_of(id).is_some(),
        CloseTarget::Tab(id) => tabs.tab_of(id).is_some(),
        CloseTarget::Window => !tabs.is_empty(),
    }
}

/// Panes of `target` for which `running` holds (a foreground job): the
/// pane, every pane of its tab, or every pane of every tab.
pub(in crate::window) fn running_in<P>(
    target: &CloseTarget,
    tabs: &Tabs,
    panes: &HashMap<PaneId, P>,
    running: impl Fn(&P) -> bool,
) -> usize {
    let ids = match *target {
        CloseTarget::Pane(id) => vec![id],
        CloseTarget::Tab(id) => tabs
            .tab_of(id)
            .and_then(|i| tabs.get(i))
            .map(|t| t.tree.panes())
            .unwrap_or_default(),
        CloseTarget::Window => tabs.iter().flat_map(|t| t.tree.panes()).collect(),
    };
    ids.iter()
        .filter_map(|id| panes.get(id))
        .filter(|p| running(p))
        .count()
}

/// The first key combination bound to `a`, as shown in menus.
fn keys_of(bindings: &Bindings, a: Action) -> Option<String> {
    bindings
        .entries()
        .into_iter()
        .find(|&(_, b)| b == a)
        .map(|(c, _)| c.to_string())
}

/// The ≡ menu's rows; the rows with a key binding show it.
pub(in crate::window) fn menu_items(bindings: &Bindings) -> Vec<MenuItem> {
    let keys_of = |a: Action| keys_of(bindings, a);
    let item = |label: &str, shortcut, action| MenuItem {
        label: label.into(),
        shortcut,
        action,
    };
    vec![
        item("New tab", keys_of(Action::NewTab), MenuAction::NewTab),
        item(
            "New window",
            keys_of(Action::NewWindow),
            MenuAction::NewWindow,
        ),
        item(
            "Save session…",
            keys_of(Action::SaveSession),
            MenuAction::SaveSession,
        ),
        item(
            "Open session ▸",
            keys_of(Action::OpenSession),
            MenuAction::OpenSessionMenu,
        ),
        item("Edit config", None, MenuAction::EditConfig),
        item("Open config folder", None, MenuAction::OpenConfigFolder),
        item("Reload config", None, MenuAction::ReloadConfig),
        item("Keyboard shortcuts", None, MenuAction::Shortcuts),
        item("About montty", None, MenuAction::About),
    ]
}

/// The menu a right-click in a pane opens: the four splits, then the clipboard. Copy is
/// greyed out when the pane has no selection.
pub(in crate::window) fn pane_menu_items(
    bindings: &Bindings,
    has_selection: bool,
) -> Vec<MenuItem> {
    let item = |label: &str, a: Action, action| MenuItem {
        label: label.into(),
        shortcut: keys_of(bindings, a),
        action,
    };
    let copy = match has_selection {
        true => MenuAction::Copy,
        false => MenuAction::Disabled,
    };
    vec![
        item("Split right", Action::SplitRight, MenuAction::SplitRight),
        item("Split down", Action::SplitDown, MenuAction::SplitDown),
        item("Split left", Action::SplitLeft, MenuAction::SplitLeft),
        item("Split up", Action::SplitUp, MenuAction::SplitUp),
        MenuItem {
            label: String::new(),
            shortcut: None,
            action: MenuAction::Separator,
        },
        item("Copy", Action::Copy, copy),
        item("Paste", Action::Paste, MenuAction::Paste),
    ]
}

/// One row per key binding (numbered families collapsed, see `shortcut_rows`):
/// the label padded to 28 columns, then the keys.
pub(in crate::window) fn shortcuts_panel(bindings: &Bindings) -> InfoPanel {
    InfoPanel::new(
        "Keyboard shortcuts",
        shortcut_rows(&bindings.entries())
            .into_iter()
            .map(|(label, keys)| format!("{label:<28}{keys}"))
            .collect(),
    )
}

pub(in crate::window) fn about_panel() -> InfoPanel {
    InfoPanel::new(
        "About montty",
        vec![
            format!("montty {}", env!("CARGO_PKG_VERSION")),
            "A tiling terminal without GTK/Qt".into(),
        ],
    )
}

/// The tab's user title after a rename commits `text` (trimmed). Empty text
/// clears it (the label falls back to the pane title). A tab without a user
/// title whose automatic label (`seed`) is committed unchanged keeps none, so
/// it goes on following its pane's title.
pub(in crate::window) fn renamed_title(
    original: Option<&str>,
    seed: &str,
    text: &str,
) -> Option<String> {
    let text = text.trim();
    if text.is_empty() || (original.is_none() && text == seed.trim()) {
        return None;
    }
    Some(text.to_string())
}

/// The family name and number of a numbered action.
fn family(a: Action) -> Option<(&'static str, u8)> {
    match a {
        Action::FocusPane(n) => Some(("Focus pane", n)),
        Action::GoToTab(n) => Some(("Go to tab", n)),
        _ => None,
    }
}

/// Shortcut list rows (label, keys) from sorted binding entries. A numbered
/// family (Focus pane n, Go to tab n) with two or more numbers bound becomes
/// one row: "Focus pane 1–10" with "first … last" keys in number order.
pub(in crate::window) fn shortcut_rows(entries: &[(KeyCombo, Action)]) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    let mut done: Vec<&str> = Vec::new();
    for (combo, action) in entries {
        let Some((name, _)) = family(*action) else {
            rows.push((action.label(), combo.to_string()));
            continue;
        };
        // The whole family goes where its first entry is.
        if done.contains(&name) {
            continue;
        }
        done.push(name);
        let mut members: Vec<(u8, String, Action)> = entries
            .iter()
            .filter_map(|(c, a)| match family(*a) {
                Some((f, n)) if f == name => Some((n, c.to_string(), *a)),
                _ => None,
            })
            .collect();
        members.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
        let (first, last) = (&members[0], &members[members.len() - 1]);
        if first.0 == last.0 {
            rows.extend(members.iter().map(|(_, c, a)| (a.label(), c.clone())));
        } else {
            rows.push((
                format!("{name} {}–{}", first.0, last.0),
                format!("{} … {}", first.1, last.1),
            ));
        }
    }
    rows
}

/// The scroll change a key asks of a panel showing `visible` of `len` lines:
/// Up/Down one line, PageUp/PageDown a page, Home/End to the ends. A page is
/// two rows short of `visible`: the "more" markers cover the first and last
/// row, so a full step would skip the lines under them.
pub(in crate::window) fn panel_scroll_delta(
    key: &Key,
    visible: usize,
    len: usize,
) -> Option<isize> {
    let page = visible.saturating_sub(2).max(1) as isize;
    let all = len as isize;
    let Key::Named(named) = key else {
        return None;
    };
    Some(match named {
        NamedKey::ArrowUp => -1,
        NamedKey::ArrowDown => 1,
        NamedKey::PageUp => -page,
        NamedKey::PageDown => page,
        NamedKey::Home => -all,
        NamedKey::End => all,
        _ => return None,
    })
}

/// Whether `key` closes an open info panel: Esc, or Enter (it has nothing to
/// confirm, so Enter dismisses it too).
pub(in crate::window) fn panel_closes(key: &Key) -> bool {
    matches!(key, Key::Named(NamedKey::Escape | NamedKey::Enter))
}

/// Where a rename anchored on a pane that just closed moves: the focused pane
/// of its tab, found by `sibling` (another pane of that tab before the close);
/// None when the tab closed too.
pub(in crate::window) fn reanchor(tabs: &Tabs, sibling: Option<PaneId>) -> Option<PaneId> {
    let i = tabs.tab_of(sibling?)?;
    tabs.get(i).map(|t| t.focused)
}

/// The label the bar lays a tab out with while it is being renamed: the
/// field's text plus a column for the beam, so the tab grows as you type
/// (up to the bar's maximum tab width; past that the field scrolls).
pub(in crate::window) fn editing_label(text: &str) -> String {
    format!("{text} ")
}

/// How far right the menu is moved so it starts at x ≥ 0 (a narrow window).
pub(in crate::window) fn menu_shift(menu: &Menu, bm: &BarMetrics) -> i32 {
    (-menu.rect(bm).x).max(0)
}

/// The part of `text` shown in a field `cols` columns wide and the cursor's
/// column in it: leading chars scroll out so the cursor stays visible.
pub(in crate::window) fn field_view(text: &str, cursor: usize, cols: usize) -> (String, usize) {
    let budget = cols.saturating_sub(1);
    let chars: Vec<char> = text.chars().collect();
    let cursor = cursor.min(chars.len());
    let mut start = cursor;
    let mut col = 0;
    while start > 0 {
        let w = chars[start - 1].width().unwrap_or(0);
        if col + w > budget {
            break;
        }
        col += w;
        start -= 1;
    }
    (chars[start..].iter().collect(), col)
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::ui::panel::PanelRow;
    use crate::window::bar::BarPaint;

    use super::*;
    use crate::testutil::native_keys;
    use crate::ui::theme::PANEL_BORDER;
    use crate::window::tabs::Tab;
    use winit::keyboard::{Key, NamedKey};

    fn menu_at(anchor_right: i32) -> Menu {
        Menu::new(menu_items(&Bindings::linux_defaults()), anchor_right, 28)
    }

    fn colored(p: &BarPaint, color: [f32; 4]) -> Vec<Rect> {
        p.under
            .iter()
            .filter(|r| r.color == color)
            .map(|r| r.rect)
            .collect()
    }

    /// Tab 1 holds panes 1 and 2 (split); tab 3 holds pane 3.
    fn two_tabs() -> Tabs {
        use crate::layout::Orientation;
        let mut a = Tab::new(1, (32, 32));
        a.tree.split(1, Orientation::Horizontal, 2);
        let mut tabs = Tabs::new(a);
        tabs.push(Tab::new(3, (32, 32)));
        tabs
    }

    #[test]
    fn close_prompt_rules() {
        let tabs = two_tabs();
        let panes3 = Some(panes_message(3));
        assert_eq!(close_prompt(&CloseTarget::Window, &tabs, 0, true), panes3);
        assert_eq!(
            close_prompt(&CloseTarget::Window, &tabs, 2, true),
            Some(confirm_message(2))
        );
        assert_eq!(close_prompt(&CloseTarget::Window, &tabs, 0, false), None);
        assert_eq!(close_prompt(&CloseTarget::Tab(3), &tabs, 0, true), None);
        assert_eq!(close_prompt(&CloseTarget::Pane(1), &tabs, 0, true), None);
    }

    #[test]
    fn closing_the_last_tab_is_closing_the_window() {
        use crate::layout::Orientation;
        let mut t = Tab::new(1, (32, 32));
        t.tree.split(1, Orientation::Horizontal, 2);
        let tabs = Tabs::new(t);
        assert!(closes_window(&CloseTarget::Tab(1), &tabs));
        assert_eq!(
            close_prompt(&CloseTarget::Tab(1), &tabs, 0, true),
            Some(panes_message(2))
        );
        let single = Tabs::new(Tab::new(1, (32, 32)));
        assert_eq!(close_prompt(&CloseTarget::Window, &single, 0, true), None);
    }

    #[test]
    fn stale_confirm_is_dismissed() {
        use crate::layout::Orientation;
        let area = Rect {
            x: 0,
            y: 0,
            w: 400,
            h: 200,
        };
        let mut tabs = Tabs::new(Tab::new(1, (32, 32)));
        let mut t = Tab::new(2, (32, 32));
        t.tree.split(2, Orientation::Horizontal, 3);
        tabs.push(t);
        let mut panes: HashMap<PaneId, ()> = [1, 2, 3].map(|id| (id, ())).into();
        let alive = |t: CloseTarget, tabs: &Tabs, panes: &HashMap<PaneId, ()>| {
            target_alive(&t, tabs, panes)
        };
        for t in [
            CloseTarget::Pane(3),
            CloseTarget::Tab(2),
            CloseTarget::Tab(3),
            CloseTarget::Window,
        ] {
            assert!(alive(t, &tabs, &panes), "{t:?}");
        }
        assert!(!alive(CloseTarget::Pane(9), &tabs, &panes));
        assert!(!alive(CloseTarget::Tab(9), &tabs, &panes));
        // Pane 3 exits: its prompt is stale; its tab lives on through pane 2.
        crate::window::actions::remove_pane(&mut tabs, 3, area);
        panes.remove(&3);
        assert!(!alive(CloseTarget::Pane(3), &tabs, &panes));
        assert!(alive(CloseTarget::Tab(2), &tabs, &panes));
        assert!(alive(CloseTarget::Pane(2), &tabs, &panes));
        // A pane still in a tree but already dropped counts as gone.
        panes.remove(&2);
        assert!(!alive(CloseTarget::Pane(2), &tabs, &panes));
        // The tab closes: its prompt is stale, the window's is not.
        tabs.remove(1);
        assert!(!alive(CloseTarget::Tab(2), &tabs, &panes));
        assert!(alive(CloseTarget::Window, &tabs, &panes));
        // No tabs left: nothing to close.
        tabs.remove(0);
        assert!(!alive(CloseTarget::Window, &tabs, &panes));
    }

    #[test]
    fn running_counts_per_target() {
        let mut tabs = Tabs::new(Tab::new(1, (32, 32)));
        let mut t = Tab::new(2, (32, 32));
        t.tree.split(2, crate::layout::Orientation::Horizontal, 3);
        t.tree.split(3, crate::layout::Orientation::Vertical, 4);
        tabs.push(t);
        // Pane 4 is in the tree but already gone from the pane map.
        let panes: HashMap<PaneId, bool> = [(1, true), (2, true), (3, true)].into();
        let n = |t: CloseTarget| running_in(&t, &tabs, &panes, |&r| r);
        assert_eq!(n(CloseTarget::Pane(1)), 1);
        assert_eq!(n(CloseTarget::Pane(4)), 0);
        assert_eq!(n(CloseTarget::Tab(3)), 2);
        assert_eq!(n(CloseTarget::Tab(1)), 1);
        assert_eq!(n(CloseTarget::Window), 3);
        assert_eq!(n(CloseTarget::Tab(9)), 0);
        let idle: HashMap<PaneId, bool> = [(1, false), (2, false), (3, true)].into();
        assert_eq!(running_in(&CloseTarget::Window, &tabs, &idle, |&r| r), 1);
        assert_eq!(running_in(&CloseTarget::Tab(2), &tabs, &idle, |&r| r), 1);
        assert_eq!(running_in(&CloseTarget::Pane(2), &tabs, &idle, |&r| r), 0);
    }

    #[test]
    fn menu_lists_new_window_with_shortcut() {
        let items = menu_items(&Bindings::linux_defaults());
        let row = items.iter().find(|i| i.label == "New window").unwrap();
        assert_eq!(row.shortcut.as_deref(), Some("Ctrl+Shift+N"));
        assert_eq!(row.action, MenuAction::NewWindow);
    }

    #[test]
    fn menu_rows_in_spec_order() {
        let items = menu_items(&Bindings::linux_defaults());
        let rows: Vec<(&str, Option<&str>, MenuAction)> = items
            .iter()
            .map(|i| (i.label.as_str(), i.shortcut.as_deref(), i.action))
            .collect();
        assert_eq!(
            rows,
            [
                ("New tab", Some("Ctrl+Shift+T"), MenuAction::NewTab),
                ("New window", Some("Ctrl+Shift+N"), MenuAction::NewWindow),
                (
                    "Save session…",
                    Some("Ctrl+Shift+S"),
                    MenuAction::SaveSession
                ),
                (
                    "Open session ▸",
                    Some("Ctrl+Shift+O"),
                    MenuAction::OpenSessionMenu
                ),
                ("Edit config", None, MenuAction::EditConfig),
                ("Open config folder", None, MenuAction::OpenConfigFolder),
                ("Reload config", None, MenuAction::ReloadConfig),
                ("Keyboard shortcuts", None, MenuAction::Shortcuts),
                ("About montty", None, MenuAction::About),
            ]
        );
    }

    #[test]
    fn prompt_commits_and_cancels() {
        let mods = Mods::default();
        let key = |f: &mut TextField, k: NamedKey| f.key(&Key::Named(k), None, mods);
        let mut f = TextField::new("dem");
        f.insert("o");
        assert_eq!(
            key(&mut f, NamedKey::Enter),
            FieldOutcome::Commit("demo".into())
        );
        assert_eq!(key(&mut f, NamedKey::Escape), FieldOutcome::Cancel);
    }

    #[test]
    fn shortcuts_panel_lists_every_binding() {
        let b = Bindings::linux_defaults();
        let p = shortcuts_panel(&b);
        assert_eq!(p.title, "Keyboard shortcuts");
        assert_eq!(p.scroll, 0);
        assert_eq!(p.lines.len(), shortcut_rows(&b.entries()).len());
        let new_tab = format!("{:<28}{}", "New tab", "Ctrl+Shift+T");
        assert!(p.lines.contains(&new_tab), "{:?}", p.lines);
        assert_eq!(&new_tab[..10], "New tab   ");
        assert_eq!(new_tab.find("Ctrl"), Some(28));
        let focus = format!("{:<28}{}", "Focus pane 1–10", native_keys("Alt+1 … Alt+0"));
        assert!(p.lines.contains(&focus), "{:?}", p.lines);
    }

    #[test]
    fn about_panel_text() {
        let p = about_panel();
        assert_eq!(p.title, "About montty");
        assert_eq!(
            p.lines,
            [
                format!("montty {}", env!("CARGO_PKG_VERSION")),
                "A tiling terminal without GTK/Qt".to_string(),
            ]
        );
    }

    #[test]
    fn rename_commit_rules() {
        // Untouched automatic label: still automatic.
        assert_eq!(renamed_title(None, "bash", "bash"), None);
        assert_eq!(renamed_title(None, " bash ", "bash"), None);
        assert_eq!(renamed_title(None, "bash", " bash  "), None);
        // Edited automatic label.
        assert_eq!(renamed_title(None, "bash", "bash 2"), Some("bash 2".into()));
        // Empty clears.
        assert_eq!(renamed_title(None, "bash", ""), None);
        assert_eq!(renamed_title(Some("build"), "build", "  "), None);
        // A user title kept as is stays.
        assert_eq!(
            renamed_title(Some("build"), "build", "build"),
            Some("build".into())
        );
        assert_eq!(
            renamed_title(Some("build"), "build", " ci "),
            Some("ci".into())
        );
    }

    #[test]
    fn numbered_families_collapse() {
        let b = Bindings::linux_defaults();
        let rows = shortcut_rows(&b.entries());
        assert!(rows.len() < b.entries().len() - 15, "{}", rows.len());
        let row = |l: &str| rows.iter().find(|(x, _)| x == l).map(|(_, k)| k.as_str());
        assert_eq!(
            row("Focus pane 1–10"),
            Some(native_keys("Alt+1 … Alt+0").as_str())
        );
        assert_eq!(
            row("Go to tab 1–10"),
            Some(native_keys("Ctrl+Alt+1 … Ctrl+Alt+0").as_str())
        );
        assert_eq!(row("Focus pane 2"), None);
        assert_eq!(row("Split right"), Some(native_keys("Ctrl+Alt+R").as_str()));
        let at = |l: &str| rows.iter().position(|(x, _)| x == l).unwrap();
        assert!(at("Find previous") < at("Focus pane 1–10"));
        assert!(at("Focus pane 1–10") < at("Focus pane above"));
        assert!(at("Go to tab 1–10") < at("Maximize pane"));
    }

    #[test]
    fn partial_families_collapse_to_bound_ends() {
        let k = |s: &str| s.parse::<KeyCombo>().unwrap();
        // Sorted as `entries()` sorts them.
        let entries = [
            (k("Alt+1"), Action::FocusPane(1)),
            (k("F2"), Action::FocusPane(2)),
            (k("Ctrl+5"), Action::FocusPane(5)),
            (k("Ctrl+Alt+3"), Action::GoToTab(3)),
            (k("Ctrl+Alt+R"), Action::SplitRight),
        ];
        let rows = shortcut_rows(&entries);
        let s = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert_eq!(
            rows,
            [
                s("Focus pane 1–5", &native_keys("Alt+1 … Ctrl+5")),
                s("Go to tab 3", &native_keys("Ctrl+Alt+3")),
                s("Split right", &native_keys("Ctrl+Alt+R")),
            ]
        );
    }

    #[test]
    fn menu_clamped_to_window_left_edge() {
        let bm = BarMetrics::test();
        assert_eq!(menu_shift(&menu_at(400), &bm), 0);
        let m = menu_at(50);
        let w = m.rect(&bm).w;
        assert_eq!(menu_shift(&m, &bm), w - 50);
        let p = paint_menu(&m, &bm, 1);
        assert_eq!(colored(&p, PANEL_BORDER)[0].x, 0);
        assert!(p.labels.iter().all(|l| l.x >= 0));
    }

    #[test]
    fn profile_menu_under_button_clamped() {
        let button = Rect {
            x: 300,
            y: 40,
            w: 20,
            h: 20,
        };
        let (mw, mh) = (120, 3 * 28);
        // Top-left under the button.
        assert_eq!(profile_menu_origin(button, mw, mh, 800, 600), (300, 60));
        // Kept inside the right edge, then the left one.
        assert_eq!(profile_menu_origin(button, mw, mh, 380, 600), (260, 60));
        assert_eq!(profile_menu_origin(button, mw, mh, 100, 600), (0, 60));
        // No room below: above the button.
        let low = Rect { y: 500, ..button };
        assert_eq!(profile_menu_origin(low, mw, mh, 800, 600), (300, 500 - mh));
        // Exactly fits below.
        assert_eq!(profile_menu_origin(button, mw, mh, 800, 60 + mh), (300, 60));
        // Room neither below nor above: from the window's top.
        assert_eq!(profile_menu_origin(button, mw, mh, 800, 70), (300, 0));
    }

    #[test]
    fn profile_menu_limits_follow_the_window() {
        let bm = BarMetrics::test();
        let button = Rect {
            x: 300,
            y: 40,
            w: 20,
            h: 20,
        };
        // Room below the button (600 - 60 = 540 px = 19 rows of 28) beats room above.
        assert_eq!(
            profile_menu_limits(button, 800, 600, &bm),
            (19, (800 - 2 * 6) / 8)
        );
        // The button low in the window: the room above counts.
        let low = Rect { y: 500, ..button };
        assert_eq!(profile_menu_limits(low, 800, 600, &bm).0, 500 / 28);
        // A tiny window still gets one row and the minimum width.
        let (rows, cols) = profile_menu_limits(button, 40, 30, &bm);
        assert_eq!(rows, 1);
        assert!(cols >= 12);
    }

    #[test]
    fn menu_with_many_long_profiles_fits_the_window() {
        let bm = BarMetrics::test();
        let button = Rect {
            x: 90,
            y: 0,
            w: 20,
            h: 28,
        };
        let (win_w, win_h) = (160, 200);
        let long = "n".repeat(80);
        let stems: Vec<_> = (0..30).map(|i| (format!("p{i}"), long.clone())).collect();
        let (rows, cols) = profile_menu_limits(button, win_w, win_h, &bm);
        let items = profile_menu(&stems, "p0", true, rows, cols);
        let probe = Menu::new(items, 0, 0);
        let r = probe.rect(&bm);
        assert!(r.w <= win_w, "{r:?}");
        assert!(r.h <= win_h, "{r:?}");
        let (x, y) = profile_menu_origin(button, r.w, r.h, win_w, win_h);
        assert!(x >= 0 && x + r.w <= win_w && y >= 0 && y + r.h <= win_h);
        assert!(probe.items.iter().any(|i| i.label.contains("more")));
        assert_eq!(probe.items.last().unwrap().action, MenuAction::NewProfile);
    }

    #[test]
    fn prompt_titles() {
        let title = |p: PromptPurpose| p.title();
        assert_eq!(title(PromptPurpose::SaveSession), Some(SAVE_PROMPT_TITLE));
        assert_eq!(
            title(PromptPurpose::NewProfile { pane: 1 }),
            Some(NEW_PROFILE_TITLE)
        );
        // The rename is edited in place over the tab: no titled panel.
        assert_eq!(
            title(PromptPurpose::RenameTab {
                pane: 1,
                original: None,
                seed: String::new(),
            }),
            None
        );
    }

    #[test]
    fn deleted_profile_is_not_chosen() {
        let stems = vec![
            ("a".to_string(), "A".to_string()),
            ("b".to_string(), "B".to_string()),
        ];
        let known = |s: &str| s != "b";
        assert_eq!(profile_choice(&stems, 0, known), Some("a"));
        // Deleted while the menu was open: nothing.
        assert_eq!(profile_choice(&stems, 1, known), None);
        assert_eq!(profile_choice(&stems, 5, known), None);
    }

    #[test]
    fn pane_menu_rows() {
        let rows = |items: Vec<MenuItem>| -> Vec<(String, Option<String>, MenuAction)> {
            items
                .into_iter()
                .map(|i| (i.label, i.shortcut, i.action))
                .collect()
        };
        let row = |l: &str, s: Option<&str>, a| (l.to_string(), s.map(native_keys), a);
        let b = Bindings::linux_defaults();
        assert_eq!(
            rows(pane_menu_items(&b, true)),
            [
                row("Split right", Some("Ctrl+Alt+R"), MenuAction::SplitRight),
                row("Split down", Some("Ctrl+Alt+D"), MenuAction::SplitDown),
                row(
                    "Split left",
                    Some("Ctrl+Alt+Shift+L"),
                    MenuAction::SplitLeft
                ),
                row("Split up", Some("Ctrl+Alt+Shift+D"), MenuAction::SplitUp),
                row("", None, MenuAction::Separator),
                row("Copy", Some("Ctrl+Shift+C"), MenuAction::Copy),
                row("Paste", Some("Ctrl+Shift+V"), MenuAction::Paste),
            ]
        );
        // Nothing selected: Copy shows, greyed out.
        let items = pane_menu_items(&b, false);
        assert_eq!(
            (items[5].label.as_str(), items[5].action),
            ("Copy", MenuAction::Disabled)
        );
    }

    #[test]
    fn pane_menu_shows_rebound_shortcuts() {
        let k = |s: &str| s.parse::<KeyCombo>().unwrap();
        let b = Bindings::with_overrides(
            Bindings::linux_defaults(),
            &[
                (Action::SplitLeft, vec![k("Ctrl+Alt+E")]),
                (Action::Paste, Vec::new()),
            ],
        );
        let items = pane_menu_items(&b, true);
        let shortcut = |l: &str| {
            let i = items.iter().find(|i| i.label == l).unwrap();
            i.shortcut.as_deref()
        };
        assert_eq!(
            shortcut("Split left"),
            Some(native_keys("Ctrl+Alt+E").as_str())
        );
        // Unbound: the row stays, without a shortcut.
        assert_eq!(shortcut("Paste"), None);
    }

    #[test]
    fn panel_keys_scroll() {
        let d = |n: NamedKey| panel_scroll_delta(&Key::Named(n), 10, 50);
        assert_eq!(d(NamedKey::ArrowUp), Some(-1));
        assert_eq!(d(NamedKey::ArrowDown), Some(1));
        // A page is two rows short: the markers cover the first and last row.
        assert_eq!(d(NamedKey::PageUp), Some(-8));
        assert_eq!(d(NamedKey::PageDown), Some(8));
        let page = |v| panel_scroll_delta(&Key::Named(NamedKey::PageDown), v, 50);
        assert_eq!(page(3), Some(1));
        assert_eq!(page(2), Some(1), "never a zero step");
        assert_eq!(page(0), Some(1));
        assert_eq!(d(NamedKey::Home), Some(-50));
        assert_eq!(d(NamedKey::End), Some(50));
        assert_eq!(d(NamedKey::Escape), None);
        assert_eq!(
            panel_scroll_delta(&Key::Character("j".into()), 10, 50),
            None
        );
        // The ends really are the ends.
        let mut p = InfoPanel::new("T", (0..50).map(|i| i.to_string()).collect());
        p.scroll_by(d(NamedKey::End).unwrap(), 10);
        assert_eq!(p.scroll, 40);
        p.scroll_by(d(NamedKey::Home).unwrap(), 10);
        assert_eq!(p.scroll, 0);
    }

    #[test]
    fn panel_paging_skips_no_line() {
        // Every line is shown as a line (not under a marker) on some page,
        // paging down and back up.
        for visible in [3, 4, 10] {
            let len = 37;
            let mut p = InfoPanel::new("T", (0..len).map(|i| i.to_string()).collect());
            let shown = |p: &InfoPanel| -> Vec<usize> {
                p.body_rows(visible)
                    .into_iter()
                    .filter_map(|r| match r {
                        PanelRow::Line(l) => l.parse().ok(),
                        _ => None,
                    })
                    .collect()
            };
            for key in [NamedKey::PageDown, NamedKey::PageUp] {
                let mut seen = shown(&p);
                let d = panel_scroll_delta(&Key::Named(key), visible, len).unwrap();
                while p.scroll_by(d, visible) {
                    seen.extend(shown(&p));
                }
                seen.sort_unstable();
                seen.dedup();
                assert_eq!(seen, (0..len).collect::<Vec<_>>(), "{visible} {key:?}");
            }
        }
    }

    #[test]
    fn enter_and_esc_close_info_panels() {
        assert!(panel_closes(&Key::Named(NamedKey::Escape)));
        assert!(panel_closes(&Key::Named(NamedKey::Enter)));
        assert!(!panel_closes(&Key::Named(NamedKey::PageDown)));
        assert!(!panel_closes(&Key::Character("q".into())));
    }

    #[test]
    fn rename_reanchors_when_its_pane_closes() {
        use crate::layout::Orientation;
        let area = Rect {
            x: 0,
            y: 0,
            w: 400,
            h: 200,
        };
        let mut tabs = Tabs::new(Tab::new(1, (32, 32)));
        let mut t = Tab::new(2, (32, 32));
        t.tree.split(2, Orientation::Horizontal, 3);
        tabs.push(t);
        // Pane 2 (the anchor) closes; its tab lives on with pane 3 focused.
        crate::window::actions::remove_pane(&mut tabs, 2, area);
        assert_eq!(reanchor(&tabs, Some(3)), Some(3));
        assert_eq!(tabs.get(1).unwrap().focused, 3);
        // The tab's last pane closes: nowhere to go.
        crate::window::actions::remove_pane(&mut tabs, 3, area);
        assert_eq!(reanchor(&tabs, Some(3)), None);
        assert_eq!(reanchor(&tabs, None), None);
    }

    #[test]
    fn renamed_tab_grows_with_its_text() {
        let bm = BarMetrics::test();
        let text = "montty dev";
        let labels = [editing_label(text), "other".into()];
        let bar = crate::ui::topbar::layout(800, &bm, &labels, 0, &[]);
        let cols = (bar.tabs[0].text_rect.w / bm.cell_w) as usize;
        // The whole text and the beam after it fit without scrolling.
        assert_eq!(field_view(text, 10, cols), (text.into(), 10));
        assert_eq!(bar.tabs[1].label, "other");
    }

    #[test]
    fn field_view_keeps_cursor_visible() {
        assert_eq!(field_view("abc", 3, 10), ("abc".into(), 3));
        assert_eq!(field_view("abc", 1, 10), ("abc".into(), 1));
        // 10 chars in 5 columns, cursor at the end: the last 4 chars and the beam.
        assert_eq!(field_view("0123456789", 10, 5), ("6789".into(), 4));
        assert_eq!(field_view("0123456789", 2, 5), ("0123456789".into(), 2));
        assert_eq!(field_view("0123456789", 7, 5), ("3456789".into(), 4));
        // Wide chars count two columns.
        assert_eq!(field_view("中文字", 3, 4), ("字".into(), 2));
        assert_eq!(field_view("abc", 3, 0), (String::new(), 0));
    }
}
