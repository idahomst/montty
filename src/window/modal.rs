//! Modal widgets in the window: the ≡ menu, a pane's profile menu, the info
//! panels (keyboard shortcuts, about), the text prompts (tab rename, session name) and
//! the close confirmation. At most one is open; while it is, it takes every key and
//! pointer press. The widgets' state and geometry are pure
//! (`ui::{menu,panel,textfield,confirm}`); this file opens, routes and draws them.

use std::collections::HashMap;
use std::time::Instant;

use unicode_width::UnicodeWidthChar;
use winit::event::{KeyEvent, MouseButton};
use winit::keyboard::{Key, NamedKey};
use winit::window::CursorIcon;

use super::Window;
use super::bar::{BarPaint, Label};
use super::tabs::Tabs;
use crate::clipboard::ClipKind;
use crate::input::Mods;
use crate::input::bindings::{Action, Bindings, KeyCombo};
use crate::layout::{PaneId, Rect};
use crate::render::RectInstance;
use crate::term::pane::Pane;
use crate::ui::confirm::{
    CANCEL_LABEL, CLOSE_LABEL, CloseTarget, Confirm, ConfirmButton, ConfirmOutcome,
    confirm_message, panes_message,
};
use crate::ui::menu::{CHECK, MIN_COLS, Menu, MenuAction, MenuItem, MenuOutcome, profile_menu};
use crate::ui::panel::{InfoPanel, PanelRow, more_text};
use crate::ui::text::{display_width, truncate};
use crate::ui::textfield::{FieldOutcome, TextField};
use crate::ui::theme;
use crate::ui::topbar::{BarMetrics, Target, tab_label};

/// The save prompt's title and its width in columns.
pub(super) const SAVE_PROMPT_TITLE: &str = "Save session";
/// The new profile prompt's title.
pub(super) const NEW_PROFILE_TITLE: &str = "New profile";
const SAVE_PROMPT_COLS: i32 = 40;

/// The open modal widget, if any. Opening one replaces the current one.
pub(super) enum Modal {
    None,
    /// The ≡ menu; with `profiles` a pane's profile menu; with `sessions` the session
    /// list (the names it opened with, so a change on disk can't shift its rows); with
    /// `pane` the menu a right-click in that pane opened.
    Menu {
        menu: Menu,
        profiles: Option<ProfileMenu>,
        sessions: Option<Vec<String>>,
        pane: Option<PaneId>,
    },
    Panel(InfoPanel),
    /// A single-line text field for `purpose`.
    Prompt {
        purpose: PromptPurpose,
        field: TextField,
    },
    /// Asks before closing a pane, tab or window with running processes.
    Confirm(Confirm),
}

/// What a text prompt is for; Enter commits `field`'s text to it.
pub(super) enum PromptPurpose {
    /// Renames the tab holding `pane` (its focused pane when opened), so the
    /// rename survives tab index shifts. Edited in place over the tab's label.
    /// `original` is the tab's user title when opened and `seed` the label
    /// the field started with.
    RenameTab {
        pane: PaneId,
        original: Option<String>,
        seed: String,
    },
    /// The name to save the session under: a titled panel over the window.
    SaveSession,
    /// The name of a new profile copied from `pane`'s, which then switches to it: a
    /// titled panel like `SaveSession`.
    NewProfile { pane: PaneId },
}

/// The title of `modal` when it is a prompt shown as a titled panel.
pub(super) fn prompt_title(modal: &Modal) -> Option<&'static str> {
    match modal {
        Modal::Prompt {
            purpose: PromptPurpose::SaveSession,
            ..
        } => Some(SAVE_PROMPT_TITLE),
        Modal::Prompt {
            purpose: PromptPurpose::NewProfile { .. },
            ..
        } => Some(NEW_PROFILE_TITLE),
        _ => None,
    }
}

/// What a profile menu switches: `pane`, to one of `stems` (`Store::stems` when
/// it opened, so a reload meanwhile can't shift the rows' indices).
pub(super) struct ProfileMenu {
    pub pane: PaneId,
    pub stems: Vec<(String, String)>,
}

/// The top-left corner of a `w` × `h` menu opened from `button` in a `win_w` ×
/// `win_h` window: under the button, its left edge on the button's; moved left
/// to end inside the window (but not past x = 0), and above the button when it
/// would overflow the bottom (from y = 0 when it fits neither way).
pub(super) fn profile_menu_origin(
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
pub(super) fn profile_menu_limits(
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
pub(super) fn profile_choice(
    stems: &[(String, String)],
    i: usize,
    known: impl Fn(&str) -> bool,
) -> Option<&str> {
    let (stem, _) = stems.get(i)?;
    known(stem).then_some(stem.as_str())
}

/// Whether keys go to bindings and the focused PTY: only when no modal is open.
pub(super) fn routes_to_pty(modal: &Modal) -> bool {
    matches!(modal, Modal::None)
}

/// The index of the tab a rename targets; None once its pane is gone.
pub(super) fn resolve_rename(tabs: &Tabs, pane: PaneId) -> Option<usize> {
    tabs.tab_of(pane)
}

/// Closing `target` closes the window: `Window`, or `Tab` of the only tab.
pub(super) fn closes_window(target: &CloseTarget, tabs: &Tabs) -> bool {
    match target {
        CloseTarget::Window => true,
        CloseTarget::Tab(_) => tabs.iter().count() == 1,
        CloseTarget::Pane(_) => false,
    }
}

/// The one prompt for closing `target`, or None to close at once: a running
/// process asks about it; else closing the window asks about its several panes
/// (when `confirm_window`). Never both.
pub(super) fn close_prompt(
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
pub(super) fn target_alive<P>(
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
pub(super) fn running_in<P>(
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

/// Whether closing a pane would stop something running: a pane started with
/// an explicit command (`-e`) runs it as long as the pane lives (it leads the
/// session, so the foreground-job check can't see it); any other pane when a
/// foreground job (`foreground_child`) owns its terminal.
pub(super) fn pane_running(runs_command: bool, foreground_child: impl FnOnce() -> bool) -> bool {
    runs_command || foreground_child()
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
pub(super) fn menu_items(bindings: &Bindings) -> Vec<MenuItem> {
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
pub(super) fn pane_menu_items(bindings: &Bindings, has_selection: bool) -> Vec<MenuItem> {
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

/// The top-left corner of a `w` × `h` menu opened by a click at (`x`, `y`): on the
/// pointer, moved to stay inside the window like a profile menu.
pub(super) fn pointer_menu_origin(
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    win_w: i32,
    win_h: i32,
) -> (i32, i32) {
    let at = Rect { x, y, w: 0, h: 0 };
    profile_menu_origin(at, w, h, win_w, win_h)
}

/// One row per key binding (numbered families collapsed, see `shortcut_rows`):
/// the label padded to 28 columns, then the keys.
pub(super) fn shortcuts_panel(bindings: &Bindings) -> InfoPanel {
    InfoPanel::new(
        "Keyboard shortcuts",
        shortcut_rows(&bindings.entries())
            .into_iter()
            .map(|(label, keys)| format!("{label:<28}{keys}"))
            .collect(),
    )
}

pub(super) fn about_panel() -> InfoPanel {
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
pub(super) fn renamed_title(original: Option<&str>, seed: &str, text: &str) -> Option<String> {
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
pub(super) fn shortcut_rows(entries: &[(KeyCombo, Action)]) -> Vec<(String, String)> {
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
pub(super) fn panel_scroll_delta(key: &Key, visible: usize, len: usize) -> Option<isize> {
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
pub(super) fn panel_closes(key: &Key) -> bool {
    matches!(key, Key::Named(NamedKey::Escape | NamedKey::Enter))
}

/// Where a rename anchored on a pane that just closed moves: the focused pane
/// of its tab, found by `sibling` (another pane of that tab before the close);
/// None when the tab closed too.
pub(super) fn reanchor(tabs: &Tabs, sibling: Option<PaneId>) -> Option<PaneId> {
    let i = tabs.tab_of(sibling?)?;
    tabs.get(i).map(|t| t.focused)
}

/// The label the bar lays a tab out with while it is being renamed: the
/// field's text plus a column for the beam, so the tab grows as you type
/// (up to the bar's maximum tab width; past that the field scrolls).
pub(super) fn editing_label(text: &str) -> String {
    format!("{text} ")
}

/// How far right the menu is moved so it starts at x ≥ 0 (a narrow window).
pub(super) fn menu_shift(menu: &Menu, bm: &BarMetrics) -> i32 {
    (-menu.rect(bm).x).max(0)
}

/// The part of `text` shown in a field `cols` columns wide and the cursor's
/// column in it: leading chars scroll out so the cursor stays visible.
pub(super) fn field_view(text: &str, cursor: usize, cols: usize) -> (String, usize) {
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

/// `r` in `border`, `stroke` px wide, filled with `bg`: two rects, border first.
pub(super) fn framed(r: Rect, stroke: i32, bg: [f32; 4], border: [f32; 4]) -> [RectInstance; 2] {
    [
        RectInstance {
            rect: r,
            color: border,
        },
        RectInstance {
            rect: r.inset(stroke),
            color: bg,
        },
    ]
}

/// The menu: MENU_BG inside a `stroke` px MENU_BORDER, the selected row in
/// HOVER, labels left and shortcuts right-aligned. A label's `CHECK` mark is
/// drawn in ACCENT, the rest of it in TEXT.
pub(super) fn paint_menu(menu: &Menu, bm: &BarMetrics, stroke: i32) -> BarPaint {
    let dx = menu_shift(menu, bm);
    let shift = |r: Rect| Rect { x: r.x + dx, ..r };
    let r = shift(menu.rect(bm));
    let inner = r.inset(stroke);
    let mut out = BarPaint::default();
    out.under
        .extend(framed(r, stroke, theme::PANEL_BG, theme::PANEL_BORDER));
    let rows: Vec<Rect> = menu.item_rects(bm).into_iter().map(shift).collect();
    if let Some(row) = menu.selected().and_then(|i| rows.get(i)) {
        // The row within the border.
        let top = row.y.max(inner.y);
        let bottom = (row.y + row.h).min(inner.y + inner.h);
        out.under.push(RectInstance {
            rect: Rect {
                x: inner.x,
                y: top,
                w: inner.w,
                h: (bottom - top).max(0),
            },
            color: theme::HOVER,
        });
    }
    for (item, row) in menu.items.iter().zip(&rows) {
        if item.action == MenuAction::Separator {
            out.under.push(RectInstance {
                rect: Rect {
                    x: inner.x + bm.pad,
                    y: row.y + (row.h - stroke) / 2,
                    w: inner.w - 2 * bm.pad,
                    h: stroke,
                },
                color: theme::PANEL_BORDER,
            });
            continue;
        }
        let y = row.y + bm.pad;
        let mut x = row.x + bm.pad;
        let mut text = item.label.as_str();
        if let Some(rest) = text.strip_prefix(CHECK) {
            out.labels.push(Label {
                x,
                y,
                text: CHECK.trim_end().to_string(),
                color: theme::to_rgb(theme::ACCENT),
                clip: inner,
            });
            x += display_width(CHECK) as i32 * bm.cell_w;
            text = rest;
        }
        let color = match item.action {
            MenuAction::Disabled => theme::TEXT_DIM,
            _ => theme::TEXT,
        };
        out.labels.push(Label {
            x,
            y,
            text: text.to_string(),
            color: theme::to_rgb(color),
            clip: inner,
        });
        if let Some(sc) = &item.shortcut {
            out.labels.push(Label {
                x: row.x + row.w - bm.pad - display_width(sc) as i32 * bm.cell_w,
                y,
                text: sc.clone(),
                color: theme::to_rgb(theme::TEXT_DIM),
                clip: inner,
            });
        }
    }
    out
}

/// A panel centered in a `win_w` × `win_h` window: the fixed title, then the
/// body rows that fit whole from the scroll position, each truncated to the
/// panel's width. Lines are TEXT; "↑ N more" / "↓ N more" markers TEXT_DIM.
pub(super) fn paint_panel(
    panel: &InfoPanel,
    win_w: i32,
    win_h: i32,
    bm: &BarMetrics,
    stroke: i32,
) -> BarPaint {
    let r = panel.rect(win_w, win_h, bm);
    let mut out = BarPaint::default();
    out.under
        .extend(framed(r, stroke, theme::PANEL_BG, theme::PANEL_BORDER));
    let text_w = (r.w - 2 * bm.pad).max(0);
    let cols = (text_w / bm.cell_w.max(1)) as usize;
    let bottom = r.y + r.h - bm.pad;
    let label = |y: i32, text: &str, color: [f32; 4]| Label {
        x: r.x + bm.pad,
        y,
        text: truncate(text, cols),
        color: theme::to_rgb(color),
        clip: Rect {
            x: r.x + bm.pad,
            y,
            w: text_w,
            h: bm.cell_h,
        },
    };
    let title_y = r.y + bm.pad;
    if title_y + bm.cell_h <= bottom {
        out.labels.push(label(title_y, &panel.title, theme::TEXT));
    }
    let first = title_y + bm.cell_h + bm.pad;
    let visible = panel.visible_lines(r, bm);
    for (i, row) in panel.body_rows(visible).into_iter().enumerate() {
        let (text, color) = match (row, more_text(row)) {
            (PanelRow::Line(l), _) => (l.to_string(), theme::TEXT),
            (_, marker) => (marker.unwrap_or_default(), theme::TEXT_DIM),
        };
        out.labels
            .push(label(first + i as i32 * bm.cell_h, &text, color));
    }
    out
}

/// The close prompt centered in a `win_w` × `win_h` window: PANEL_BG inside
/// a `stroke` px PANEL_BORDER, the message (TEXT, truncated to the prompt),
/// then Cancel and Close. Close is the default (Enter): an ACCENT border and
/// fill; Cancel a PANEL_BORDER border. The hovered button is filled with HOVER.
pub(super) fn paint_confirm(
    c: &Confirm,
    win_w: i32,
    win_h: i32,
    bm: &BarMetrics,
    stroke: i32,
) -> BarPaint {
    let r = c.rect(win_w, win_h, bm);
    let (close, cancel) = c.button_rects(win_w, win_h, bm);
    let fill = |b: ConfirmButton, normal: [f32; 4]| {
        if c.hover == Some(b) {
            theme::HOVER
        } else {
            normal
        }
    };
    let mut out = BarPaint::default();
    out.under
        .extend(framed(r, stroke, theme::PANEL_BG, theme::PANEL_BORDER));
    let cancel_bg = fill(ConfirmButton::Cancel, theme::PANEL_BG);
    out.under
        .extend(framed(cancel, stroke, cancel_bg, theme::PANEL_BORDER));
    let close_bg = fill(ConfirmButton::Close, theme::ACCENT);
    out.under
        .extend(framed(close, stroke, close_bg, theme::ACCENT));
    let m = 2 * bm.pad;
    let text = Rect {
        x: r.x + m,
        y: r.y + m,
        w: (r.w - 2 * m).max(0),
        h: bm.cell_h,
    };
    let cols = (text.w / bm.cell_w.max(1)) as usize;
    out.labels.push(Label {
        x: text.x,
        y: text.y,
        text: truncate(&c.message, cols),
        color: theme::to_rgb(theme::TEXT),
        clip: text,
    });
    for (b, label) in [(cancel, CANCEL_LABEL), (close, CLOSE_LABEL)] {
        out.labels.push(Label {
            x: b.x + (b.w - display_width(label) as i32 * bm.cell_w) / 2,
            y: b.y + bm.pad,
            text: label.into(),
            color: theme::to_rgb(theme::TEXT),
            clip: b.inset(stroke),
        });
    }
    out
}

/// The rename field's outer rect (border included) around a tab's label rect
/// `text`: half the bar padding bigger on every side, so the border doesn't
/// touch the text.
fn field_rect(text: Rect, bm: &BarMetrics, stroke: i32) -> Rect {
    let o = (bm.pad / 2).max(stroke);
    Rect {
        x: text.x - o,
        y: text.y - o,
        w: text.w + 2 * o,
        h: text.h + 2 * o,
    }
}

/// The rename field over a tab's label rect `text`: an ACCENT border, the
/// active-tab background, the text and a beam cursor (`stroke` px, no blink).
pub(super) fn paint_rename(
    field: &TextField,
    text: Rect,
    bm: &BarMetrics,
    stroke: i32,
) -> BarPaint {
    let r = field_rect(text, bm, stroke);
    let cols = (text.w / bm.cell_w.max(1)).max(0) as usize;
    let (shown, col) = field_view(field.text(), field.cursor(), cols);
    let mut out = BarPaint::default();
    out.under
        .extend(framed(r, stroke, theme::TAB_ACTIVE, theme::ACCENT));
    out.labels.push(Label {
        x: text.x,
        y: text.y,
        text: shown,
        color: theme::to_rgb(theme::TEXT),
        clip: text,
    });
    out.icons.push(RectInstance {
        rect: Rect {
            x: text.x + col as i32 * bm.cell_w,
            y: text.y,
            w: stroke,
            h: bm.cell_h,
        },
        color: theme::TEXT,
    });
    out
}

/// The save prompt in a `win_w` × `win_h` window: the panel centered, then the title's
/// text rect and the field's text rect (as `paint_rename` takes it) inside it.
pub(super) fn save_prompt_layout(
    win_w: i32,
    win_h: i32,
    bm: &BarMetrics,
    stroke: i32,
) -> (Rect, Rect, Rect) {
    let o = (bm.pad / 2).max(stroke);
    let w = (SAVE_PROMPT_COLS * bm.cell_w + 2 * bm.pad)
        .min(win_w * 9 / 10)
        .max(0);
    let h = 3 * bm.pad + 2 * o + 2 * bm.cell_h + bm.pad;
    let panel = Rect {
        x: (win_w - w) / 2,
        y: ((win_h - h) / 2).max(0),
        w,
        h,
    };
    let title = Rect {
        x: panel.x + bm.pad,
        y: panel.y + bm.pad,
        w: (w - 2 * bm.pad).max(0),
        h: bm.cell_h,
    };
    let field = Rect {
        x: title.x + o,
        y: title.y + bm.cell_h + bm.pad + o,
        w: (title.w - 2 * o).max(0),
        h: bm.cell_h,
    };
    (panel, title, field)
}

/// A titled prompt (save session, new profile): a PANEL_BG panel in PANEL_BORDER with
/// `title_text` (TEXT) over the name field, drawn like the rename field.
pub(super) fn paint_titled_prompt(
    title_text: &str,
    field: &TextField,
    win_w: i32,
    win_h: i32,
    bm: &BarMetrics,
    stroke: i32,
) -> BarPaint {
    let (panel, title, text) = save_prompt_layout(win_w, win_h, bm, stroke);
    let mut out = BarPaint::default();
    out.under
        .extend(framed(panel, stroke, theme::PANEL_BG, theme::PANEL_BORDER));
    let cols = (title.w / bm.cell_w.max(1)).max(0) as usize;
    out.labels.push(Label {
        x: title.x,
        y: title.y,
        text: truncate(title_text, cols),
        color: theme::to_rgb(theme::TEXT),
        clip: title,
    });
    let f = paint_rename(field, text, bm, stroke);
    out.under.extend(f.under);
    out.labels.extend(f.labels);
    out.icons.extend(f.icons);
    out
}

impl Window {
    /// Whether a modal widget is open: it gets every key and press. (The pane
    /// overlay is not drawn meanwhile.)
    pub(super) fn modal_open(&self) -> bool {
        !routes_to_pty(&self.modal)
    }

    /// Replace the open modal (or close it with `Modal::None`) and redraw.
    /// Opening one ends a divider drag (it takes the pointer from now on).
    pub(super) fn set_modal(&mut self, m: Modal) {
        if !routes_to_pty(&m) {
            self.mouse.cancel_drag();
            self.size_labels.clear();
        }
        self.modal = m;
        self.schedule();
    }

    /// ≡ clicked: the menu hangs below the bar, its right edge on the button's.
    pub(super) fn open_menu(&mut self) {
        let bar = self.bar();
        let Some(button) = bar.items.iter().find(|i| i.target == Target::Menu) else {
            return;
        };
        let right = button.rect.x + button.rect.w;
        let menu = Menu::new(menu_items(&self.store.global().bindings), right, bar.height);
        self.set_modal(Modal::Menu {
            menu,
            profiles: None,
            sessions: None,
            pane: None,
        });
    }

    /// The profile pane `pane` runs under: its own, or the default when that is unknown.
    pub(super) fn current_stem(&self, pane: PaneId) -> &str {
        let stem = self.pane_stem(pane);
        match self.store.contains(stem) {
            true => stem,
            false => self.store.default_stem(),
        }
    }

    /// The Profile button (`button`) of pane `pane`'s overlay clicked: a row per
    /// profile, the one the pane resolves to marked, then editing it and creating a new
    /// one, hanging from the button (see `profile_menu_origin`). No profiles and no
    /// config folder: nothing.
    pub(super) fn open_profile_menu(&mut self, pane: PaneId, button: Rect) {
        let stems = self.store.stems();
        let files = self.user_config_dir().is_some();
        if stems.is_empty() && !files {
            return;
        }
        let current = self.current_stem(pane);
        let bm = self.bar_metrics();
        let (w, h) = self.size_px();
        let (max_rows, max_cols) = profile_menu_limits(button, w, h, &bm);
        let items = profile_menu(&stems, current, files, max_rows, max_cols);
        let probe = Menu::new(items, 0, 0);
        let r = probe.rect(&bm);
        let (x, y) = profile_menu_origin(button, r.w, r.h, w, h);
        let menu = Menu::new(probe.items, x + r.w, y);
        let profiles = Some(ProfileMenu { pane, stems });
        self.set_modal(Modal::Menu {
            menu,
            profiles,
            sessions: None,
            pane: None,
        });
    }

    /// Right-click in pane `pane` at (`x`, `y`): its menu opens on the pointer. The
    /// pane takes the focus first, so a split happens next to it.
    pub(super) fn open_pane_menu(&mut self, pane: PaneId, x: i32, y: i32) {
        if pane != self.tabs.active().focused {
            self.set_focus(pane);
        }
        let has_selection = self.selection_text(pane).is_some_and(|t| !t.is_empty());
        let items = pane_menu_items(&self.store.global().bindings, has_selection);
        let bm = self.bar_metrics();
        let (w, h) = self.size_px();
        let probe = Menu::new(items, 0, 0);
        let r = probe.rect(&bm);
        let (x, y) = pointer_menu_origin(x, y, r.w, r.h, w, h);
        self.set_modal(Modal::Menu {
            menu: Menu::new(probe.items, x + r.w, y),
            profiles: None,
            sessions: None,
            pane: Some(pane),
        });
    }

    /// Tab `i` double-clicked: edit its label in place, starting from the
    /// label as shown.
    pub(super) fn open_rename(&mut self, i: usize) {
        let Some(tab) = self.tabs.get(i) else {
            return;
        };
        let title = self
            .panes
            .get(&tab.focused)
            .and_then(|p| p.title.as_deref());
        let seed = tab_label(tab.title.as_deref(), title);
        let modal = Modal::Prompt {
            purpose: PromptPurpose::RenameTab {
                pane: tab.focused,
                original: tab.title.clone(),
                seed: seed.clone(),
            },
            field: TextField::new(&seed),
        };
        self.set_modal(modal);
    }

    /// Before pane `closed` is removed: when it anchors the open rename, another
    /// pane of its tab (to find the tab again afterwards).
    pub(super) fn rename_sibling(&self, closed: PaneId) -> Option<PaneId> {
        let Modal::Prompt {
            purpose: PromptPurpose::RenameTab { pane, .. },
            ..
        } = &self.modal
        else {
            return None;
        };
        let i = resolve_rename(&self.tabs, *pane).filter(|_| *pane == closed)?;
        self.tabs
            .get(i)?
            .tree
            .panes()
            .into_iter()
            .find(|&p| p != closed)
    }

    /// After a pane close: a rename anchored on it moves to its tab's focused
    /// pane when the tab survived (`sibling` from `rename_sibling`).
    pub(super) fn reanchor_rename(&mut self, sibling: Option<PaneId>) {
        if let Some(new) = reanchor(&self.tabs, sibling)
            && let Modal::Prompt {
                purpose: PromptPurpose::RenameTab { pane, .. },
                ..
            } = &mut self.modal
        {
            *pane = new;
        }
    }

    /// Ask before closing `target` when it has running processes (see
    /// `running_in`); with none it closes right away. The prompt replaces any
    /// other modal; a repeat request for the open prompt's own target does
    /// nothing.
    pub(super) fn request_close(&mut self, target: CloseTarget) {
        if matches!(&self.modal, Modal::Confirm(c) if c.target == target) {
            return;
        }
        let running = running_in(&target, &self.tabs, &self.panes, |p: &Pane| {
            pane_running(p.runs_command(), || p.has_foreground_child())
        });
        let confirm_window = self.store.global().confirm_close_window;
        let Some(msg) = close_prompt(&target, &self.tabs, running, confirm_window) else {
            self.close_target(target);
            return;
        };
        let mut c = Confirm::new(target, msg);
        let (w, h) = self.size_px();
        let (x, y) = self.mouse.pos();
        c.hover(x, y, w, h, &self.bar_metrics());
        self.set_modal(Modal::Confirm(c));
    }

    /// The prompt's Close: it closes, then its target does unless that is
    /// already gone.
    fn confirm_yes(&mut self) {
        let Modal::Confirm(c) = std::mem::replace(&mut self.modal, Modal::None) else {
            return;
        };
        self.schedule();
        if target_alive(&c.target, &self.tabs, &self.panes) {
            self.close_target(c.target);
        }
    }

    fn confirm_outcome(&mut self, out: ConfirmOutcome) {
        match out {
            ConfirmOutcome::None => {}
            ConfirmOutcome::Yes => self.confirm_yes(),
            ConfirmOutcome::No => self.set_modal(Modal::None),
        }
    }

    /// A menu hangs off the ≡ button or a pane's overlay, which move with the
    /// size and scale: it closes then.
    pub(super) fn close_menu(&mut self) {
        if matches!(self.modal, Modal::Menu { .. }) {
            self.set_modal(Modal::None);
        }
    }

    /// Body rows `panel` shows in the current window.
    fn panel_visible(&self, panel: &InfoPanel) -> usize {
        let (w, h) = self.size_px();
        let bm = self.bar_metrics();
        panel.visible_lines(panel.rect(w, h, &bm), &bm)
    }

    /// Whether the pointer is over the open info panel.
    pub(super) fn pointer_over_panel(&self) -> bool {
        let Modal::Panel(panel) = &self.modal else {
            return false;
        };
        let (w, h) = self.size_px();
        let (x, y) = self.mouse.pos();
        panel.rect(w, h, &self.bar_metrics()).contains(x, y)
    }

    /// Scroll the open info panel by `delta` lines; a redraw only on a change.
    pub(super) fn scroll_panel(&mut self, delta: isize) {
        let Modal::Panel(panel) = &self.modal else {
            return;
        };
        let visible = self.panel_visible(panel);
        if let Modal::Panel(panel) = &mut self.modal
            && panel.scroll_by(delta, visible)
        {
            self.schedule();
        }
    }

    /// A rename whose tab has closed, or a close prompt or profile menu whose
    /// target has, is dismissed; true when that happened. Called after panes or
    /// tabs are removed, and before each key, press and draw.
    pub(super) fn dismiss_stale_modal(&mut self) -> bool {
        let stale = match &self.modal {
            Modal::Prompt {
                purpose: PromptPurpose::RenameTab { pane, .. },
                ..
            } => resolve_rename(&self.tabs, *pane).is_none(),
            Modal::Confirm(c) => !target_alive(&c.target, &self.tabs, &self.panes),
            Modal::Menu {
                profiles: Some(ProfileMenu { pane, .. }),
                ..
            }
            | Modal::Menu {
                pane: Some(pane), ..
            }
            | Modal::Prompt {
                purpose: PromptPurpose::NewProfile { pane },
                ..
            } => !target_alive(&CloseTarget::Pane(*pane), &self.tabs, &self.panes),
            _ => false,
        };
        if stale {
            self.set_modal(Modal::None);
        }
        stale
    }

    /// Commit the open rename with `text` (see `renamed_title`) and close it.
    fn commit_rename(&mut self, text: &str) {
        let Modal::Prompt {
            purpose:
                PromptPurpose::RenameTab {
                    pane,
                    original,
                    seed,
                },
            ..
        } = &self.modal
        else {
            return;
        };
        let title = renamed_title(original.as_deref(), seed, text);
        if let Some(i) = resolve_rename(&self.tabs, *pane) {
            if let Some(tab) = self.tabs.iter_mut().nth(i) {
                tab.title = title;
            }
            self.layout_changed();
            if i == self.tabs.active_index() {
                self.update_title();
            }
        }
        self.set_modal(Modal::None);
    }

    /// Enter in the open prompt: its text goes to what the prompt is for.
    fn commit_prompt(&mut self, text: &str) {
        match &self.modal {
            Modal::Prompt {
                purpose: PromptPurpose::RenameTab { .. },
                ..
            } => self.commit_rename(text),
            Modal::Prompt {
                purpose: PromptPurpose::SaveSession,
                ..
            } => self.commit_save(text),
            Modal::Prompt {
                purpose: PromptPurpose::NewProfile { pane },
                ..
            } => self.commit_new_profile(*pane, text),
            _ => {}
        }
    }

    /// A menu row was chosen: the menu closes and its action runs.
    fn choose(&mut self, a: MenuAction) {
        match a {
            MenuAction::NewTab => {
                self.set_modal(Modal::None);
                self.perform(Action::NewTab);
            }
            MenuAction::NewWindow => {
                self.set_modal(Modal::None);
                self.perform(Action::NewWindow);
            }
            MenuAction::SaveSession => self.open_save_prompt(),
            MenuAction::OpenSessionMenu => self.open_session_menu(),
            MenuAction::EditConfig => {
                self.set_modal(Modal::None);
                self.edit_config();
            }
            MenuAction::OpenConfigFolder => {
                self.set_modal(Modal::None);
                self.open_config_folder();
            }
            MenuAction::ReloadConfig => {
                self.set_modal(Modal::None);
                self.reload_config(Instant::now());
            }
            MenuAction::Shortcuts => {
                self.set_modal(Modal::Panel(shortcuts_panel(&self.store.global().bindings)))
            }
            MenuAction::About => self.set_modal(Modal::Panel(about_panel())),
            MenuAction::SetProfile(i) => self.choose_profile(i),
            MenuAction::OpenSession(i) => {
                let name = match &self.modal {
                    Modal::Menu {
                        sessions: Some(names),
                        ..
                    } => names.get(i).cloned(),
                    _ => None,
                };
                self.set_modal(Modal::None);
                if let Some(name) = name {
                    self.open_session(&name);
                }
            }
            MenuAction::SplitRight
            | MenuAction::SplitDown
            | MenuAction::SplitLeft
            | MenuAction::SplitUp => {
                self.set_modal(Modal::None);
                self.perform(match a {
                    MenuAction::SplitRight => Action::SplitRight,
                    MenuAction::SplitDown => Action::SplitDown,
                    MenuAction::SplitLeft => Action::SplitLeft,
                    _ => Action::SplitUp,
                });
            }
            MenuAction::Copy | MenuAction::Paste => {
                let pane = match &self.modal {
                    Modal::Menu { pane, .. } => *pane,
                    _ => None,
                };
                self.set_modal(Modal::None);
                // The pane clicked, even with the find bar open (its paste goes to the field).
                match (a, pane) {
                    (MenuAction::Copy, _) => self.copy(),
                    (_, Some(pane)) => self.paste_into(pane, ClipKind::Clipboard),
                    _ => {}
                }
            }
            MenuAction::EditProfile | MenuAction::NewProfile => {
                let pane = match &self.modal {
                    Modal::Menu {
                        profiles: Some(p), ..
                    } => p.pane,
                    _ => return,
                };
                match a {
                    MenuAction::EditProfile => {
                        self.set_modal(Modal::None);
                        self.edit_profile(pane);
                    }
                    _ => self.set_modal(Modal::Prompt {
                        purpose: PromptPurpose::NewProfile { pane },
                        field: TextField::new(""),
                    }),
                }
            }
            MenuAction::Disabled | MenuAction::Separator => {}
        }
    }

    /// Row `i` of the profile menu was chosen: the menu closes, and its pane, when
    /// still there, switches to that profile and takes the focus.
    fn choose_profile(&mut self, i: usize) {
        let Modal::Menu {
            profiles: Some(p), ..
        } = std::mem::replace(&mut self.modal, Modal::None)
        else {
            return;
        };
        self.schedule();
        // A profile deleted while the menu was open: nothing changes.
        let Some(stem) = profile_choice(&p.stems, i, |s| self.store.contains(s)) else {
            return;
        };
        if !target_alive(&CloseTarget::Pane(p.pane), &self.tabs, &self.panes) {
            return;
        }
        self.set_pane_profile(p.pane, stem);
        if self.tabs.tab_of(p.pane) == Some(self.tabs.active_index()) {
            self.set_focus(p.pane);
        }
    }

    /// A key press while a modal is open: it goes to the modal only. Esc closes
    /// any of them (cancelling a rename).
    pub(super) fn modal_key(&mut self, event: &KeyEvent) {
        if self.dismiss_stale_modal() {
            return;
        }
        let key = &event.logical_key;
        let mods = Mods::from(self.modifiers);
        let visible = match &self.modal {
            Modal::Panel(panel) => self.panel_visible(panel),
            _ => 0,
        };
        match &mut self.modal {
            Modal::None => {}
            Modal::Menu { menu, .. } => {
                let before = menu.selected();
                let out = menu.key(key);
                let moved = menu.selected() != before;
                match out {
                    MenuOutcome::None if moved => self.schedule(),
                    MenuOutcome::None => {}
                    MenuOutcome::Chosen(a) => self.choose(a),
                    MenuOutcome::Close => self.set_modal(Modal::None),
                }
            }
            Modal::Panel(panel) => {
                if panel_closes(key) {
                    self.set_modal(Modal::None);
                } else if let Some(d) = panel_scroll_delta(key, visible, panel.lines.len()) {
                    self.scroll_panel(d);
                }
            }
            Modal::Prompt { field, .. } => {
                let before = (field.text().to_string(), field.cursor());
                let out = field.key(key, event.text.as_deref(), mods);
                let changed = (field.text(), field.cursor()) != (before.0.as_str(), before.1);
                match out {
                    FieldOutcome::Editing if changed => self.schedule(),
                    FieldOutcome::Editing => {}
                    FieldOutcome::Commit(text) => self.commit_prompt(&text),
                    FieldOutcome::Cancel => self.set_modal(Modal::None),
                }
            }
            Modal::Confirm(c) => {
                let out = c.key(key);
                self.confirm_outcome(out);
            }
        }
    }

    /// Pointer moved while a modal is open: the menu row or prompt button
    /// under it is highlighted (a redraw only when that changes). Nothing else
    /// reacts.
    pub(super) fn modal_pointer_moved(&mut self) {
        self.set_cursor_icon(CursorIcon::Default);
        let (x, y) = self.mouse.pos();
        let (w, h) = self.size_px();
        let bm = self.bar_metrics();
        let changed = match &mut self.modal {
            Modal::Menu { menu, .. } => {
                let before = menu.selected();
                menu.hover(x - menu_shift(menu, &bm), y, &bm);
                menu.selected() != before
            }
            Modal::Confirm(c) => c.hover(x, y, w, h, &bm),
            _ => false,
        };
        if changed {
            self.schedule();
        }
    }

    /// Where tab `i`'s label is drawn; None while the tab is clipped out of the bar.
    fn tab_text_rect(&self, i: usize) -> Option<Rect> {
        self.bar()
            .tabs
            .into_iter()
            .find(|t| t.index == i)
            .map(|t| t.text_rect)
    }

    /// A button press while a modal is open; always consumed (no tab switch,
    /// pane focus or window drag). A left press on a menu row chooses it; a
    /// press outside the menu or panel closes it. A press outside the rename
    /// field commits the rename, as Enter does (the usual in-place edit
    /// behaviour); inside it does nothing. A left press on a prompt button
    /// answers it; elsewhere the prompt stays (it needs an answer).
    pub(super) fn modal_press(&mut self, button: MouseButton) {
        self.mouse.clicks.clear();
        if self.dismiss_stale_modal() {
            return;
        }
        let (x, y) = self.mouse.pos();
        let bm = self.bar_metrics();
        match &self.modal {
            Modal::None => {}
            Modal::Menu { menu, .. } => match menu.click(x - menu_shift(menu, &bm), y, &bm) {
                MenuOutcome::Chosen(a) if button == MouseButton::Left => self.choose(a),
                MenuOutcome::Chosen(_) | MenuOutcome::None => {}
                MenuOutcome::Close => self.set_modal(Modal::None),
            },
            Modal::Panel(panel) => {
                let (w, h) = self.size_px();
                if !panel.rect(w, h, &bm).contains(x, y) {
                    self.set_modal(Modal::None);
                }
            }
            Modal::Prompt {
                purpose: PromptPurpose::RenameTab { pane, .. },
                field,
            } => {
                let text = field.text().to_string();
                let inside = resolve_rename(&self.tabs, *pane)
                    .and_then(|i| self.tab_text_rect(i))
                    .is_some_and(|r| field_rect(r, &bm, self.chrome_stroke()).contains(x, y));
                if !inside {
                    self.commit_rename(&text);
                }
            }
            Modal::Prompt {
                purpose: PromptPurpose::SaveSession | PromptPurpose::NewProfile { .. },
                ..
            } => {
                let (w, h) = self.size_px();
                let (panel, ..) = save_prompt_layout(w, h, &bm, self.chrome_stroke());
                if !panel.contains(x, y) {
                    self.set_modal(Modal::None);
                }
            }
            Modal::Confirm(c) if button == MouseButton::Left => {
                let (w, h) = self.size_px();
                let out = c.click(x, y, w, h, &bm);
                self.confirm_outcome(out);
            }
            Modal::Confirm(_) => {}
        }
    }

    /// The open modal, over everything else. A rename whose tab is clipped
    /// out of the bar (the window shrank) is not drawn but stays open.
    pub(super) fn draw_modal(&mut self) {
        if self.dismiss_stale_modal() {
            return;
        }
        let bm = self.bar_metrics();
        let stroke = self.chrome_stroke();
        let (w, h) = self.size_px();
        let paint = match &self.modal {
            Modal::None => return,
            Modal::Menu { menu, .. } => paint_menu(menu, &bm, stroke),
            Modal::Panel(panel) => paint_panel(panel, w, h, &bm, stroke),
            Modal::Prompt {
                purpose: PromptPurpose::RenameTab { pane, .. },
                field,
            } => {
                let Some(text) =
                    resolve_rename(&self.tabs, *pane).and_then(|i| self.tab_text_rect(i))
                else {
                    return;
                };
                paint_rename(field, text, &bm, stroke)
            }
            Modal::Prompt {
                purpose: PromptPurpose::SaveSession | PromptPurpose::NewProfile { .. },
                field,
            } => paint_titled_prompt(
                prompt_title(&self.modal).unwrap_or_default(),
                field,
                w,
                h,
                &bm,
                stroke,
            ),
            Modal::Confirm(c) => paint_confirm(c, w, h, &bm, stroke),
        };
        self.draw_paint(&paint);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::{
        ACCENT, HOVER, PANEL_BG, PANEL_BORDER, TAB_ACTIVE, TEXT, TEXT_DIM, to_rgb,
    };
    use crate::window::tabs::Tab;
    use winit::keyboard::{Key, NamedKey};

    fn bm() -> BarMetrics {
        BarMetrics {
            height: 28,
            pad: 6,
            cell_w: 8,
            cell_h: 16,
        }
    }

    fn menu_at(anchor_right: i32) -> Menu {
        Menu::new(menu_items(&Bindings::defaults()), anchor_right, 28)
    }

    fn colored(p: &BarPaint, color: [f32; 4]) -> Vec<Rect> {
        p.under
            .iter()
            .filter(|r| r.color == color)
            .map(|r| r.rect)
            .collect()
    }

    fn inside(outer: Rect, r: Rect) -> bool {
        r.x >= outer.x
            && r.y >= outer.y
            && r.x + r.w <= outer.x + outer.w
            && r.y + r.h <= outer.y + outer.h
    }

    #[test]
    fn modal_swallows_keys() {
        assert!(routes_to_pty(&Modal::None));
        let open = [
            Modal::Menu {
                menu: menu_at(400),
                profiles: None,
                sessions: None,
                pane: None,
            },
            Modal::Menu {
                menu: menu_at(400),
                profiles: Some(ProfileMenu {
                    pane: 1,
                    stems: Vec::new(),
                }),
                sessions: None,
                pane: None,
            },
            Modal::Menu {
                menu: menu_at(400),
                profiles: None,
                sessions: Some(Vec::new()),
                pane: None,
            },
            Modal::Panel(about_panel()),
            Modal::Prompt {
                purpose: PromptPurpose::RenameTab {
                    pane: 1,
                    original: None,
                    seed: "x".into(),
                },
                field: TextField::new("x"),
            },
            Modal::Prompt {
                purpose: PromptPurpose::SaveSession,
                field: TextField::new(""),
            },
            Modal::Confirm(Confirm::new(CloseTarget::Window, confirm_message(1))),
        ];
        for m in &open {
            assert!(!routes_to_pty(m));
        }
    }

    #[test]
    fn rename_of_closed_tab_is_dismissed() {
        let mut tabs = Tabs::new(Tab::new(1, (32, 32)));
        tabs.push(Tab::new(2, (32, 32)));
        tabs.push(Tab::new(3, (32, 32)));
        assert_eq!(resolve_rename(&tabs, 3), Some(2));
        // An earlier tab closes: the rename follows its tab to the new index.
        tabs.remove(0);
        assert_eq!(resolve_rename(&tabs, 3), Some(1));
        // Its own tab closes: nothing to rename.
        tabs.remove(1);
        assert_eq!(resolve_rename(&tabs, 3), None);
        tabs.remove(0);
        assert_eq!(resolve_rename(&tabs, 3), None);
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
        super::super::actions::remove_pane(&mut tabs, 3, area);
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
    fn command_panes_count_as_running() {
        assert!(pane_running(true, || false), "-e vim: the command itself");
        assert!(pane_running(false, || true), "shell with a foreground job");
        assert!(!pane_running(false, || false), "idle shell");
        // The foreground check (a syscall) is skipped for a command pane.
        assert!(pane_running(true, || panic!("not needed")));
        // Counted per target like any running pane.
        let tabs = Tabs::new(Tab::new(1, (32, 32)));
        let panes: HashMap<PaneId, (bool, bool)> = [(1, (true, false))].into();
        let n = running_in(&CloseTarget::Window, &tabs, &panes, |&(cmd, fg)| {
            pane_running(cmd, || fg)
        });
        assert_eq!(n, 1);
    }

    #[test]
    fn paint_confirm_frame_buttons_and_text() {
        let bm = bm();
        let mut c = Confirm::new(CloseTarget::Tab(1), confirm_message(2));
        let r = c.rect(800, 600, &bm);
        let (close, cancel) = c.button_rects(800, 600, &bm);
        let p = paint_confirm(&c, 800, 600, &bm, 1);
        assert_eq!(colored(&p, PANEL_BORDER), [r, cancel]);
        assert_eq!(colored(&p, PANEL_BG), [r.inset(1), cancel.inset(1)]);
        assert_eq!(colored(&p, ACCENT), [close, close.inset(1)]);
        assert!(colored(&p, HOVER).is_empty());
        let texts: Vec<&str> = p.labels.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "2 processes are still running. Close anyway?",
                "Cancel",
                "Close"
            ]
        );
        assert!(p.labels.iter().all(|l| l.color == to_rgb(TEXT)));
        let msg = &p.labels[0];
        assert_eq!((msg.x, msg.y), (r.x + 2 * bm.pad, r.y + 2 * bm.pad));
        assert!(inside(r, msg.clip), "{msg:?}");
        // Button labels centered in their buttons.
        let close_label = &p.labels[2];
        assert_eq!(close_label.x, close.x + (close.w - 5 * bm.cell_w) / 2);
        assert_eq!(close_label.y, close.y + bm.pad);
        assert_eq!(p.labels[1].x, cancel.x + (cancel.w - 6 * bm.cell_w) / 2);
        // A hovered button is filled with HOVER inside its border.
        c.hover = Some(ConfirmButton::Cancel);
        let p = paint_confirm(&c, 800, 600, &bm, 1);
        assert_eq!(colored(&p, HOVER), [cancel.inset(1)]);
        c.hover = Some(ConfirmButton::Close);
        let p = paint_confirm(&c, 800, 600, &bm, 1);
        assert_eq!(colored(&p, HOVER), [close.inset(1)]);
        assert_eq!(colored(&p, ACCENT), [close]);
    }

    #[test]
    fn paint_confirm_truncates_message() {
        let bm = bm();
        let c = Confirm::new(CloseTarget::Window, confirm_message(1));
        let r = c.rect(200, 300, &bm);
        let p = paint_confirm(&c, 200, 300, &bm, 1);
        let cols = ((r.w - 4 * bm.pad) / bm.cell_w) as usize;
        assert_eq!(p.labels[0].text.chars().count(), cols);
        assert!(p.labels[0].text.ends_with('…'));
    }

    #[test]
    fn menu_lists_new_window_with_shortcut() {
        let items = menu_items(&Bindings::defaults());
        let row = items.iter().find(|i| i.label == "New window").unwrap();
        assert_eq!(row.shortcut.as_deref(), Some("Ctrl+Shift+N"));
        assert_eq!(row.action, MenuAction::NewWindow);
    }

    #[test]
    fn menu_rows_in_spec_order() {
        let items = menu_items(&Bindings::defaults());
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
    fn save_prompt_is_a_titled_field_inside_its_panel() {
        let bm = bm();
        let (panel, title, field) = save_prompt_layout(800, 600, &bm, 1);
        assert!(inside(
            Rect {
                x: 0,
                y: 0,
                w: 800,
                h: 600
            },
            panel
        ));
        assert!(inside(panel, title) && inside(panel, field));
        assert!(field.y >= title.y + title.h);
        let p = paint_titled_prompt(SAVE_PROMPT_TITLE, &TextField::new("demo"), 800, 600, &bm, 1);
        assert_eq!(colored(&p, PANEL_BORDER), [panel]);
        let texts: Vec<&str> = p.labels.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["Save session", "demo"]);
        // The beam cursor is drawn.
        assert_eq!(p.icons.len(), 1);
    }

    #[test]
    fn shortcuts_panel_lists_every_binding() {
        let b = Bindings::defaults();
        let p = shortcuts_panel(&b);
        assert_eq!(p.title, "Keyboard shortcuts");
        assert_eq!(p.scroll, 0);
        assert_eq!(p.lines.len(), shortcut_rows(&b.entries()).len());
        let new_tab = format!("{:<28}{}", "New tab", "Ctrl+Shift+T");
        assert!(p.lines.contains(&new_tab), "{:?}", p.lines);
        assert_eq!(&new_tab[..10], "New tab   ");
        assert_eq!(new_tab.find("Ctrl"), Some(28));
        let focus = format!("{:<28}{}", "Focus pane 1–10", "Alt+1 … Alt+0");
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
        let b = Bindings::defaults();
        let rows = shortcut_rows(&b.entries());
        assert!(rows.len() < b.entries().len() - 15, "{}", rows.len());
        let row = |l: &str| rows.iter().find(|(x, _)| x == l).map(|(_, k)| k.as_str());
        assert_eq!(row("Focus pane 1–10"), Some("Alt+1 … Alt+0"));
        assert_eq!(row("Go to tab 1–10"), Some("Ctrl+Alt+1 … Ctrl+Alt+0"));
        assert_eq!(row("Focus pane 2"), None);
        assert_eq!(row("Split right"), Some("Ctrl+Alt+R"));
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
                s("Focus pane 1–5", "Alt+1 … Ctrl+5"),
                s("Go to tab 3", "Ctrl+Alt+3"),
                s("Split right", "Ctrl+Alt+R"),
            ]
        );
    }

    #[test]
    fn menu_clamped_to_window_left_edge() {
        let bm = bm();
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
        let bm = bm();
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
        let bm = bm();
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
        let title = |p: PromptPurpose| {
            prompt_title(&Modal::Prompt {
                purpose: p,
                field: TextField::new(""),
            })
        };
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
    fn paint_menu_check_mark_in_accent() {
        let bm = bm();
        let stems = [
            ("a".to_string(), "Alpha".to_string()),
            ("b".to_string(), "Beta".to_string()),
        ];
        let m = Menu::new(
            crate::ui::menu::profile_menu_items(&stems, "b", 99, 40),
            400,
            28,
        );
        let rows = m.item_rects(&bm);
        let p = paint_menu(&m, &bm, 1);
        let texts: Vec<_> = p
            .labels
            .iter()
            .map(|l| (l.text.as_str(), l.x, l.y, l.color))
            .collect();
        let (x, y0, y1) = (rows[0].x + bm.pad, rows[0].y + bm.pad, rows[1].y + bm.pad);
        assert_eq!(
            texts,
            [
                ("  Alpha", x, y0, to_rgb(TEXT)),
                ("✓", x, y1, to_rgb(ACCENT)),
                ("Beta", x + 2 * bm.cell_w, y1, to_rgb(TEXT)),
            ]
        );
    }

    #[test]
    fn paint_menu_frame_hover_and_text() {
        let bm = bm();
        let mut m = menu_at(400);
        let r = m.rect(&bm);
        let p = paint_menu(&m, &bm, 1);
        assert_eq!(colored(&p, PANEL_BORDER), [r]);
        assert_eq!(colored(&p, PANEL_BG), [r.inset(1)]);
        assert!(colored(&p, HOVER).is_empty());
        // Labels left, the shortcut right-aligned and dim.
        let texts: Vec<&str> = p.labels.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "New tab",
                "Ctrl+Shift+T",
                "New window",
                "Ctrl+Shift+N",
                "Save session…",
                "Ctrl+Shift+S",
                "Open session ▸",
                "Ctrl+Shift+O",
                "Edit config",
                "Open config folder",
                "Reload config",
                "Keyboard shortcuts",
                "About montty"
            ]
        );
        let sc = &p.labels[1];
        assert_eq!(sc.x + 12 * bm.cell_w, r.x + r.w - bm.pad);
        assert_eq!(sc.color, to_rgb(TEXT_DIM));
        assert_eq!(p.labels[0].color, to_rgb(TEXT));
        assert_eq!(
            (p.labels[2].x, p.labels[2].y),
            (r.x + bm.pad, r.y + bm.height + bm.pad)
        );
        // The hovered row, inside the border.
        let row = m.item_rects(&bm)[1];
        m.hover(row.x + 2, row.y + 2, &bm);
        let p = paint_menu(&m, &bm, 1);
        let hover = colored(&p, HOVER);
        assert_eq!(hover.len(), 1);
        assert!(inside(r.inset(1), hover[0]));
        assert_eq!((hover[0].y, hover[0].h), (row.y, row.h));
        // The first and last rows stop at the border.
        let last = *m.item_rects(&bm).last().unwrap();
        m.hover(last.x + 2, last.y + 2, &bm);
        let hover = colored(&paint_menu(&m, &bm, 1), HOVER);
        assert_eq!(hover[0].y + hover[0].h, last.y + last.h - 1);
    }

    #[test]
    fn pane_menu_rows() {
        let rows = |items: Vec<MenuItem>| -> Vec<(String, Option<String>, MenuAction)> {
            items
                .into_iter()
                .map(|i| (i.label, i.shortcut, i.action))
                .collect()
        };
        let row = |l: &str, s: Option<&str>, a| (l.to_string(), s.map(String::from), a);
        let b = Bindings::defaults();
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
            Bindings::defaults(),
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
        assert_eq!(shortcut("Split left"), Some("Ctrl+Alt+E"));
        // Unbound: the row stays, without a shortcut.
        assert_eq!(shortcut("Paste"), None);
    }

    #[test]
    fn pane_menu_opens_at_the_pointer_and_stays_inside() {
        // Room below and right: its top-left corner on the pointer.
        assert_eq!(pointer_menu_origin(100, 50, 120, 90, 800, 600), (100, 50));
        // Near the bottom-right corner: moved left, and above the pointer.
        assert_eq!(pointer_menu_origin(780, 590, 120, 90, 800, 600), (680, 500));
        // A window smaller than the menu: from the top-left corner.
        assert_eq!(pointer_menu_origin(10, 10, 120, 90, 60, 40), (0, 0));
    }

    #[test]
    fn paint_menu_separator_is_a_line() {
        let bm = bm();
        let m = Menu::new(pane_menu_items(&Bindings::defaults(), true), 400, 28);
        let r = m.rect(&bm);
        let sep = m.item_rects(&bm)[4];
        let p = paint_menu(&m, &bm, 1);
        // The frame, then one border-coloured line across the separator row.
        let lines: Vec<Rect> = colored(&p, PANEL_BORDER)
            .into_iter()
            .filter(|l| *l != r)
            .collect();
        assert_eq!(lines.len(), 1);
        let line = lines[0];
        assert_eq!(line.h, 1);
        assert!(inside(sep, line), "{line:?} in {sep:?}");
        assert_eq!(line.x, r.x + 1 + bm.pad);
        assert_eq!(line.x + line.w, r.x + r.w - 1 - bm.pad);
        // No label for it.
        assert!(p.labels.iter().all(|l| !l.text.is_empty()));
        assert!(!p.labels.iter().any(|l| l.y >= sep.y && l.y < sep.y + sep.h));
    }

    #[test]
    fn paint_panel_clips_lines() {
        let bm = bm();
        let mut panel = InfoPanel::new("T", (0..100).map(|i| format!("line {i}")).collect());
        let r = panel.rect(800, 600, &bm);
        let p = paint_panel(&panel, 800, 600, &bm, 1);
        assert_eq!(colored(&p, PANEL_BORDER), [r]);
        assert_eq!(colored(&p, PANEL_BG), [r.inset(1)]);
        assert_eq!(p.labels[0].text, "T");
        assert_eq!((p.labels[0].x, p.labels[0].y), (r.x + bm.pad, r.y + bm.pad));
        assert_eq!(p.labels[1].text, "line 0");
        assert_eq!(p.labels[1].y, r.y + 2 * bm.pad + bm.cell_h);
        // Only whole lines above the bottom padding.
        let shown = p.labels.len() - 1;
        assert_eq!(shown as i32, (r.h - 3 * bm.pad - bm.cell_h) / bm.cell_h);
        assert_eq!(shown, panel.visible_lines(r, &bm));
        assert!(shown < 100);
        for l in &p.labels {
            assert!(l.y + bm.cell_h <= r.y + r.h - bm.pad, "{l:?}");
            assert!(inside(r, l.clip), "{l:?}");
        }
        // Lines in TEXT; the last row says how many more there are, dim.
        assert_eq!(p.labels[1].color, to_rgb(TEXT));
        let last = p.labels.last().unwrap();
        assert_eq!(last.text, format!("↓ {} more", 100 - shown + 1));
        assert_eq!(last.color, to_rgb(TEXT_DIM));
        // Scrolled: the title stays, the first row marks the lines above.
        panel.scroll_by(5, shown);
        let p = paint_panel(&panel, 800, 600, &bm, 1);
        assert_eq!(p.labels[0].text, "T");
        assert_eq!(p.labels[1].text, "↑ 6 more");
        assert_eq!(p.labels[1].color, to_rgb(TEXT_DIM));
        assert_eq!(p.labels[2].text, "line 6");
        // A short panel shows everything.
        let p = paint_panel(&about_panel(), 800, 600, &bm, 1);
        assert_eq!(p.labels.len(), 3);
    }

    #[test]
    fn paint_panel_truncates_wide_lines() {
        let bm = bm();
        let panel = InfoPanel::new("T", vec!["x".repeat(500)]);
        let r = panel.rect(400, 300, &bm);
        let p = paint_panel(&panel, 400, 300, &bm, 1);
        let cols = ((r.w - 2 * bm.pad) / bm.cell_w) as usize;
        assert_eq!(p.labels[1].text.chars().count(), cols);
        assert!(p.labels[1].text.ends_with('…'));
    }

    #[test]
    fn paint_rename_border_text_and_beam() {
        let bm = bm();
        let text = Rect {
            x: 6,
            y: 6,
            w: 80,
            h: 16,
        };
        let mut f = TextField::new("abc");
        let p = paint_rename(&f, text, &bm, 1);
        let field = Rect {
            x: 3,
            y: 3,
            w: 86,
            h: 22,
        };
        assert_eq!(colored(&p, ACCENT), [field]);
        assert_eq!(colored(&p, TAB_ACTIVE), [field.inset(1)]);
        assert_eq!(p.labels.len(), 1);
        assert_eq!(p.labels[0].text, "abc");
        assert_eq!((p.labels[0].x, p.labels[0].y), (6, 6));
        assert_eq!(p.labels[0].clip, text);
        assert_eq!(p.labels[0].color, to_rgb(TEXT));
        // The beam sits after the cursor's column, a cell tall.
        assert_eq!(
            p.icons
                .iter()
                .map(|r| (r.rect, r.color))
                .collect::<Vec<_>>(),
            [(
                Rect {
                    x: 6 + 3 * 8,
                    y: 6,
                    w: 1,
                    h: 16
                },
                TEXT
            )]
        );
        f.key(&Key::Named(NamedKey::Home), None, Default::default());
        assert_eq!(paint_rename(&f, text, &bm, 2).icons[0].rect.x, 6);
        assert_eq!(paint_rename(&f, text, &bm, 2).icons[0].rect.w, 2);
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
        super::super::actions::remove_pane(&mut tabs, 2, area);
        assert_eq!(reanchor(&tabs, Some(3)), Some(3));
        assert_eq!(tabs.get(1).unwrap().focused, 3);
        // The tab's last pane closes: nowhere to go.
        super::super::actions::remove_pane(&mut tabs, 3, area);
        assert_eq!(reanchor(&tabs, Some(3)), None);
        assert_eq!(reanchor(&tabs, None), None);
    }

    #[test]
    fn renamed_tab_grows_with_its_text() {
        let bm = bm();
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
