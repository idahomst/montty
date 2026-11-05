//! Painting the modals: menus, panels, prompts and the close confirmation.

use super::*;
use crate::layout::Rect;
use crate::render::RectInstance;
use crate::ui::confirm::{CANCEL_LABEL, CLOSE_LABEL, Confirm, ConfirmButton};
use crate::ui::menu::{CHECK, Menu, MenuAction};
use crate::ui::panel::{InfoPanel, PanelRow, more_text};
use crate::ui::text::{display_width, truncate};
use crate::ui::textfield::TextField;
use crate::ui::theme;
use crate::ui::topbar::BarMetrics;
use crate::window::bar::{BarPaint, Label};

/// `r` in `border`, `stroke` px wide, filled with `bg`: two rects, border first.
pub(in crate::window) fn framed(
    r: Rect,
    stroke: i32,
    bg: [f32; 4],
    border: [f32; 4],
) -> [RectInstance; 2] {
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

/// The menu in a `stroke` px frame: the selected row highlighted, labels left and
/// shortcuts right-aligned. A label's `CHECK` mark is drawn in the accent color.
pub(in crate::window) fn paint_menu(menu: &Menu, bm: &BarMetrics, stroke: i32) -> BarPaint {
    let dx = menu_shift(menu, bm);
    let shift = |r: Rect| Rect { x: r.x + dx, ..r };
    let r = shift(menu.rect(bm));
    let inner = r.inset(stroke);
    let mut out = BarPaint::default();
    out.under
        .extend(framed(r, stroke, theme::PANEL_BG, theme::PANEL_BORDER));
    let rows: Vec<Rect> = menu.item_rects(bm).into_iter().map(shift).collect();
    if let Some(row) = menu.selected().and_then(|i| rows.get(i)) {
        // The row's full width within the border.
        let full = Rect {
            x: inner.x,
            w: inner.w,
            ..*row
        };
        if let Some(rect) = full.intersect(inner) {
            let color = theme::HOVER;
            out.under.push(RectInstance { rect, color });
        }
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
/// panel's width, with "↑ N more" / "↓ N more" markers.
pub(in crate::window) fn paint_panel(
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

/// The close prompt centered in a `win_w` × `win_h` window, in a `stroke` px frame: the
/// message (truncated to the prompt), then Cancel and Close. Close is the default (Enter),
/// so it is the accented one; the hovered button is highlighted.
pub(in crate::window) fn paint_confirm(
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
pub(in crate::window) fn field_rect(text: Rect, bm: &BarMetrics, stroke: i32) -> Rect {
    let o = (bm.pad / 2).max(stroke);
    Rect {
        x: text.x - o,
        y: text.y - o,
        w: text.w + 2 * o,
        h: text.h + 2 * o,
    }
}

/// The rename field over a tab's label rect `text`: a framed field with the text and
/// a beam cursor (`stroke` px, no blink).
pub(in crate::window) fn paint_rename(
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
pub(in crate::window) fn titled_prompt_layout(
    win_w: i32,
    win_h: i32,
    bm: &BarMetrics,
    stroke: i32,
) -> (Rect, Rect, Rect) {
    let o = (bm.pad / 2).max(stroke);
    let w = (TITLED_PROMPT_COLS * bm.cell_w + 2 * bm.pad)
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
pub(in crate::window) fn paint_titled_prompt(
    title_text: &str,
    field: &TextField,
    win_w: i32,
    win_h: i32,
    bm: &BarMetrics,
    stroke: i32,
) -> BarPaint {
    let (panel, title, text) = titled_prompt_layout(win_w, win_h, bm, stroke);
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
    out.append(paint_rename(field, text, bm, stroke));
    out
}

#[cfg(test)]
mod tests {
    use crate::input::bindings::Bindings;
    use crate::ui::confirm::confirm_message;

    use super::*;
    use crate::ui::theme::{
        ACCENT, HOVER, PANEL_BG, PANEL_BORDER, TAB_ACTIVE, TEXT, TEXT_DIM, to_rgb,
    };

    use winit::keyboard::{Key, NamedKey};

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
    fn paint_confirm_frame_buttons_and_text() {
        let bm = BarMetrics::test();
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
        let bm = BarMetrics::test();
        let c = Confirm::new(CloseTarget::Window, confirm_message(1));
        let r = c.rect(200, 300, &bm);
        let p = paint_confirm(&c, 200, 300, &bm, 1);
        let cols = ((r.w - 4 * bm.pad) / bm.cell_w) as usize;
        assert_eq!(p.labels[0].text.chars().count(), cols);
        assert!(p.labels[0].text.ends_with('…'));
    }

    #[test]
    fn save_prompt_is_a_titled_field_inside_its_panel() {
        let bm = BarMetrics::test();
        let (panel, title, field) = titled_prompt_layout(800, 600, &bm, 1);
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
    fn paint_menu_check_mark_in_accent() {
        let bm = BarMetrics::test();
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
        let bm = BarMetrics::test();
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
    fn paint_menu_separator_is_a_line() {
        let bm = BarMetrics::test();
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
        let bm = BarMetrics::test();
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
        let bm = BarMetrics::test();
        let panel = InfoPanel::new("T", vec!["x".repeat(500)]);
        let r = panel.rect(400, 300, &bm);
        let p = paint_panel(&panel, 400, 300, &bm, 1);
        let cols = ((r.w - 2 * bm.pad) / bm.cell_w) as usize;
        assert_eq!(p.labels[1].text.chars().count(), cols);
        assert!(p.labels[1].text.ends_with('…'));
    }

    #[test]
    fn paint_rename_border_text_and_beam() {
        let bm = BarMetrics::test();
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
}
