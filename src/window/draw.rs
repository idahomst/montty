//! Frame drawing: the top bar, the panes, the tiling chrome, the pane hover
//! overlay, then an open modal on top.

use std::sync::Arc;
use std::time::Instant;

use alacritty_terminal::grid::Dimensions;
use glutin::surface::GlSurface;

use super::bar::{BarPaint, Label};
use super::find::visible_matches;
use super::links::{label_rect, label_text};
use super::modal::framed;
use super::pane_drag::{PaneDrag, paint_drop_target};
use super::sizes::{size_label_rect, size_text};
use super::{Window, scrollbar_visible};
use crate::config::GlobalConfig;
use crate::config::store::Resolved;
use crate::layout::{Divider, PaneId, Rect};
use crate::render::image;
use crate::render::{PaneDraw, RectInstance, ShapeInstance};
use crate::term::colors::Rgb;
use crate::term::snapshot::Highlights;
use crate::ui::icons::{Icon, icon_shapes};
use crate::ui::overlay::{Overlay, OverlayTarget};
use crate::ui::scrollbar::scrollbar_thumb;
use crate::ui::text::display_width;
use crate::ui::theme;
use crate::ui::toast::{self, Toast, ToastSource};
use crate::ui::topbar::BarMetrics;

/// Focused-pane border width.
const BORDER_PX: i32 = 2;

/// The tiling chrome's colors, from `config.toml`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ChromeColors {
    /// The focused pane's border (default `#4a90d9`).
    pub border: [f32; 4],
    /// Divider lines (default `#555555`).
    pub divider: [f32; 4],
    /// Unfocused panes are dimmed by black at `dim_amount` (default 0.25).
    pub dim: [f32; 4],
}

impl ChromeColors {
    pub(super) fn of(g: &GlobalConfig) -> ChromeColors {
        ChromeColors {
            border: theme::from_rgb(g.focus_border),
            divider: theme::from_rgb(g.divider),
            dim: [0.0, 0.0, 0.0, g.dim],
        }
    }
}

/// What every pane of a frame shares.
struct PaneFrame {
    began: Instant,
    focused_id: PaneId,
    translucent: bool,
    clear_bg: Rgb,
}

impl Window {
    pub fn redraw(&mut self) {
        // The frame cadence counts from here, not from after the swap.
        let began = Instant::now();
        if self.size.width > 0 && self.size.height > 0 && self.ensure_current() {
            self.draw_frame(began);
        }
        self.scheduler.frame_drawn(began);
        if std::mem::take(&mut self.resize_redraw) {
            self.schedule();
        }
    }

    fn draw_frame(&mut self, began: Instant) {
        let focused_id = self.tabs.active().focused;
        let area = self.area();
        let layout = self.tabs.active().tree.layout(area);
        // Each visible pane's profile, looked up once per frame.
        let visible: Vec<(PaneId, Rect, Arc<Resolved>)> = (layout.iter())
            .filter(|(id, _)| self.panes.contains_key(id))
            .map(|&(id, rect)| (id, rect, self.pane_profile(id)))
            .collect();
        let translucent = visible.iter().any(|(_, _, p)| p.profile.opacity < 1.0);
        // An opaque window's background is the focused pane's; other panes fill their
        // own. A translucent window clears to transparent and every pane fills its own.
        let clear_bg = match visible.iter().find(|(id, _, _)| *id == focused_id) {
            Some((_, _, p)) => p.palette.bg,
            None => self.pane_profile(focused_id).palette.bg,
        };
        self.renderer
            .begin_frame(frame_clear(translucent, clear_bg));
        self.draw_bar();
        let frame = PaneFrame {
            began,
            focused_id,
            translucent,
            clear_bg,
        };
        for (id, rect, profile) in visible {
            self.draw_one_pane(id, rect, &profile, &frame);
        }
        let colors = ChromeColors::of(self.store.global());
        let dividers = self.tabs.active().tree.dividers(area);
        let rects = chrome(&layout, &dividers, focused_id, &colors);
        self.renderer.draw_rects(&rects);
        if let Some(PaneDrag::Dragging { over: Some(t), .. }) = self.mouse.pane_drag
            && let Some(&(_, rect)) = layout.iter().find(|&&(id, _)| id == t)
        {
            let rects = paint_drop_target(rect, self.chrome_stroke());
            self.renderer.draw_rects(&rects);
        }
        self.draw_pane_dots(&layout);
        self.draw_find();
        self.draw_overlay();
        self.draw_size_labels();
        self.draw_link_label();
        self.draw_mark_hover();
        self.draw_toast();
        // Last, over everything.
        self.draw_modal();
        self.window.pre_present_notify();
        if let Err(e) = self.gl_surface.swap_buffers(&self.gl_context) {
            eprintln!("montty: swap_buffers: {e}");
        }
    }

    /// Pane `id` at `rect`: its background, image, cells, scrollbar and bell flash.
    fn draw_one_pane(&mut self, id: PaneId, rect: Rect, profile: &Resolved, frame: &PaneFrame) {
        let font = self.pane_font(id);
        let palette = &profile.palette;
        let Some((lines, history)) = self.capture_snapshot(id, profile) else {
            return;
        };
        // The effective default bg (an OSC 11 override, else the palette's) at the
        // profile's opacity.
        let bg = self.snapshot.bg;
        let opacity = profile.profile.opacity;
        if frame.translucent || bg != frame.clear_bg {
            let mut color = theme::from_rgb(bg);
            color[3] = opacity;
            self.renderer.draw_rects(&[RectInstance { rect, color }]);
        }
        // Then its background image, if loaded, at the image's opacity.
        let image = (profile.profile.background_image.as_ref())
            .and_then(|img| Some((img, self.images.texture(&img.path)?)));
        if let Some((img, tex)) = image
            && let Some((dest, uv)) = image::fit_rect(img.fit, (tex.w, tex.h), rect)
        {
            let repeat = image::repeats(img.fit);
            self.renderer.draw_image(tex, dest, uv, img.opacity, repeat);
        }
        // Default-bg cells of a translucent pane let its bg rect show through, and
        // those over an image let the image show.
        let default_bg_alpha = if opacity < 1.0 || image.is_some() {
            0
        } else {
            255
        };
        self.renderer.set_cursor_color(palette.cursor);
        let Some(fonts) = self.fonts.by_key(font.key) else {
            return;
        };
        // Unfocused panes (and every pane of an unfocused window) get a hollow cursor.
        let focused = self.window_focused && id == frame.focused_id;
        // Only the focused pane blinks; the hidden phase draws no cursor.
        if focused {
            self.blink_on = self.snapshot.blinking;
            self.blink_shown = !self.blink_on || self.blink.visible(frame.began);
            if !self.blink_shown {
                self.snapshot.cursor = None;
            }
        }
        let pane = PaneDraw {
            clip: rect,
            content: rect.inset(font.padding),
            metrics: &font.metrics,
            focused,
            default_bg_alpha,
        };
        self.renderer.draw_pane(&pane, &self.snapshot, fonts);
        let last_scroll = self.scrolled.get(&id).copied();
        if scrollbar_visible(self.snapshot.display_offset, last_scroll, frame.began)
            && let Some(thumb) = scrollbar_thumb(
                rect,
                lines,
                history,
                self.snapshot.display_offset,
                self.scale,
            )
        {
            let mut color = theme::TEXT_DIM;
            color[3] = 0.6;
            self.renderer
                .draw_rects(&[RectInstance { rect: thumb, color }]);
        }
        // The bell's flash over the whole pane.
        if self.bell.is_some_and(|(b, _)| b == id) {
            let mut color = theme::TEXT;
            color[3] = 0.25;
            self.renderer.draw_rects(&[RectInstance { rect, color }]);
        }
    }

    /// Copy pane `id`'s visible cells into `self.snapshot` under one brief lock, with the
    /// hovered link and the find matches; returns its (screen lines, history size).
    fn capture_snapshot(&mut self, id: PaneId, profile: &Resolved) -> Option<(usize, usize)> {
        let term = self.panes.get(&id)?.term.lock();
        let link = (self.links.hover.as_ref())
            .filter(|(pane, _)| *pane == id)
            .map(|(_, l)| &l.range);
        // The find pane's visible matches, found under this same lock.
        let find = self.find.as_mut().filter(|f| f.pane == id);
        let (matches, current) = match find {
            Some(f) => (
                (f.regex.as_mut())
                    .map(|re| visible_matches(&term, re))
                    .unwrap_or_default(),
                f.current.clone(),
            ),
            None => (Vec::new(), None),
        };
        let hl = Highlights {
            matches: &matches,
            current: current.as_ref(),
            link,
        };
        let bright = profile.profile.bold_is_bright;
        self.snapshot.capture(&term, &profile.palette, bright, &hl);
        Some((term.screen_lines(), term.grid().history_size()))
    }

    /// The hover overlay of the pane under the pointer (none while a modal
    /// is open). Recomputed here, so it is never stale after a layout change.
    fn draw_overlay(&mut self) {
        let overlay = self.overlay_at_pointer();
        self.overlay = overlay.as_ref().map(|(id, _, hover)| (*id, *hover));
        if let Some((id, o, hover)) = overlay {
            let maximized = self.tabs.active().tree.maximized() == Some(id);
            let (rects, icons) =
                paint_overlay(&o, hover, maximized, self.chrome_stroke(), self.icon_line());
            self.renderer.draw_rects(&rects);
            self.renderer.draw_shapes(&icons);
        }
    }

    /// The "cols × rows" label centred on each pane that was just resized. The numbers are
    /// read now, so a zoom while the labels are up shows the new size.
    fn draw_size_labels(&mut self) {
        if !self.size_labels.showing() {
            return;
        }
        let bm = self.bar_metrics();
        let stroke = self.chrome_stroke();
        for (id, pane) in self.tabs.active().tree.layout(self.area()) {
            let Some(size) = self
                .sizes
                .get(&id)
                .filter(|_| self.size_labels.contains(id))
            else {
                continue;
            };
            let text = size_text(size.num_cols, size.num_lines);
            if let Some(rect) = size_label_rect(pane, display_width(&text), &bm) {
                self.draw_paint(&paint_label_box(rect, &text, &bm, stroke));
            }
        }
    }

    /// The hovered link's address, when it differs from its text: a label below the
    /// link's last visible row (above its first when there is no room), over the panes
    /// and the overlay, under the toast.
    fn draw_link_label(&mut self) {
        let Some((id, link)) = self.links.hover.clone() else {
            return;
        };
        let Some(text) = label_text(&link) else {
            return;
        };
        let Some(&(_, pane)) = self
            .tabs
            .active()
            .tree
            .layout(self.area())
            .iter()
            .find(|&&(p, _)| p == id)
        else {
            return;
        };
        let font = self.pane_font(id);
        let content = pane.inset(font.padding);
        let (cw, ch) = (
            font.metrics.cell_width as i32,
            font.metrics.cell_height as i32,
        );
        let Some(term) = self.panes.get(&id).map(|p| p.term.lock()) else {
            return;
        };
        let (lines, offset) = (
            term.screen_lines() as i32,
            term.grid().display_offset() as i32,
        );
        drop(term);
        // Screen rows of the link's ends; a link scrolled partly off uses its visible rows.
        let (start, end) = (link.range.start(), link.range.end());
        let (r0, r1) = (start.line.0 + offset, end.line.0 + offset);
        if r1 < 0 || r0 >= lines {
            return;
        }
        let cell = |row: i32, col: usize| Rect {
            x: content.x + col as i32 * cw,
            y: content.y + row * ch,
            w: cw,
            h: ch,
        };
        let first = if r0 < 0 {
            cell(0, 0)
        } else {
            cell(r0, start.column.0)
        };
        let last = cell(r1.min(lines - 1), end.column.0);
        let bm = self.bar_metrics();
        let cols = display_width(&text);
        let Some(rect) = label_rect(first, last, pane, cols, &bm) else {
            return;
        };
        let p = paint_label_box(rect, &text, &bm, self.chrome_stroke());
        self.draw_paint(&p);
    }

    /// The toast, over the panes and the overlay, under a modal.
    fn draw_toast(&mut self) {
        let Some(t) = &self.toast else {
            return;
        };
        let (w, h) = self.size_px();
        let p = paint_toast(t, w, h, &self.bar_metrics(), self.chrome_stroke());
        self.draw_paint(&p);
    }
}

/// The frame's clear color: transparent when any visible pane is translucent (each
/// pane then fills its own bg at its opacity), else the opaque `bg`.
pub(super) fn frame_clear(any_translucent: bool, bg: Rgb) -> [f32; 4] {
    if any_translucent {
        [0.0; 4]
    } else {
        theme::from_rgb(bg)
    }
}

/// The toast in a `win_w` × `win_h` window, in a `stroke` px frame, its text truncated
/// to the toast. An info toast takes the panel's colors: it is not an error.
pub(super) fn paint_toast(
    t: &Toast,
    win_w: i32,
    win_h: i32,
    bm: &BarMetrics,
    stroke: i32,
) -> BarPaint {
    let r = t.rect(win_w, win_h, bm);
    let mut out = BarPaint::default();
    if r.w <= 2 * stroke || r.h <= 2 * stroke {
        return out;
    }
    let (bg, border, fg) = match t.source {
        ToastSource::Info => (theme::PANEL_BG, theme::PANEL_BORDER, theme::TEXT),
        _ => (theme::TOAST_BG, theme::TOAST_BORDER, theme::TOAST_TEXT),
    };
    out.under.extend(framed(r, stroke, bg, border));
    let text = Rect {
        x: r.x + bm.pad,
        y: r.y + bm.pad,
        w: (r.w - 2 * bm.pad).max(0),
        h: bm.cell_h,
    };
    let cols = (text.w / bm.cell_w.max(1)) as usize;
    out.labels.push(Label {
        x: text.x,
        y: text.y,
        text: toast::label(&t.text, &t.tail, cols),
        color: theme::to_rgb(fg),
        clip: text,
    });
    out
}

/// The link address label in `rect`: an overlay-style box holding `text` in TEXT, cut
/// with an ellipsis when the box is narrower.
pub(super) fn paint_label_box(rect: Rect, text: &str, bm: &BarMetrics, stroke: i32) -> BarPaint {
    let mut out = BarPaint::default();
    if rect.w <= 2 * stroke || rect.h <= 2 * stroke {
        return out;
    }
    out.under.extend(framed(
        rect,
        stroke,
        theme::OVERLAY_BG,
        theme::OVERLAY_BORDER,
    ));
    let inner = Rect {
        x: rect.x + bm.pad,
        y: rect.y + bm.pad,
        w: (rect.w - 2 * bm.pad).max(0),
        h: bm.cell_h,
    };
    let cols = (inner.w / bm.cell_w.max(1)) as usize;
    out.labels.push(Label {
        x: inner.x,
        y: inner.y,
        text: toast::label(text, "", cols),
        color: theme::to_rgb(theme::TEXT),
        clip: inner,
    });
    out
}

/// Chrome over the visible panes: dim overlays on unfocused panes, divider
/// lines, and the focused pane's border when more than one pane is visible.
pub(super) fn chrome(
    layout: &[(PaneId, Rect)],
    dividers: &[Divider],
    focused: PaneId,
    colors: &ChromeColors,
) -> Vec<RectInstance> {
    let mut out: Vec<RectInstance> = layout
        .iter()
        .filter(|&&(id, _)| id != focused)
        .map(|&(_, rect)| RectInstance {
            rect,
            color: colors.dim,
        })
        .collect();
    out.extend(dividers.iter().map(|d| RectInstance {
        rect: d.line,
        color: colors.divider,
    }));
    if layout.len() > 1
        && let Some(&(_, r)) = layout.iter().find(|&&(id, _)| id == focused)
    {
        // Not overlapping, so a translucent border color doesn't blend twice at the corners.
        let b = BORDER_PX.min(r.w).min(r.h);
        out.extend(border_rects(r, b).into_iter().map(|rect| RectInstance {
            rect,
            color: colors.border,
        }));
    }
    out
}

/// The four edges of `r`, `s` px thick, not overlapping; empty ones (a rect too small
/// for a hollow) are dropped.
pub(super) fn border_rects(r: Rect, s: i32) -> Vec<Rect> {
    [
        Rect { h: s, ..r },
        Rect {
            y: r.y + r.h - s,
            h: s,
            ..r
        },
        Rect {
            y: r.y + s,
            w: s,
            h: r.h - 2 * s,
            ..r
        },
        Rect {
            x: r.x + r.w - s,
            y: r.y + s,
            w: s,
            h: r.h - 2 * s,
        },
    ]
    .into_iter()
    .filter(|e| e.w > 0 && e.h > 0)
    .collect()
}

/// The pane overlay in a `stroke` px frame, the hovered button highlighted within the
/// border (Close in red, as in the bar), then the icons (`line` px strokes): Restore
/// instead of Maximize while the pane is maximized. An empty overlay (tiny pane) paints
/// nothing.
pub(super) fn paint_overlay(
    o: &Overlay,
    hover: Option<OverlayTarget>,
    maximized: bool,
    stroke: i32,
    line: f32,
) -> (Vec<RectInstance>, Vec<ShapeInstance>) {
    if o.items.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let fill = |rect: Rect, color: [f32; 4]| RectInstance { rect, color };
    let r = o.bg;
    let s = stroke.clamp(1, (r.w.min(r.h) / 2).max(1));
    let inner = r.inset(s);
    let mut out = vec![fill(inner, theme::OVERLAY_BG)];
    // Four edges, not a filled rect under the bg: the bg is translucent.
    out.extend(
        border_rects(r, s)
            .into_iter()
            .map(|e| fill(e, theme::OVERLAY_BORDER)),
    );
    for &(rect, target) in &o.items {
        if hover == Some(target) {
            let color = match target {
                OverlayTarget::Grip | OverlayTarget::Profile | OverlayTarget::Maximize => {
                    theme::HOVER
                }
                OverlayTarget::Close => theme::CLOSE_HOVER,
            };
            if let Some(within) = rect.intersect(inner) {
                out.push(fill(within, color));
            }
        }
    }
    let mut icons = Vec::new();
    for &(rect, target) in &o.items {
        let icon = match target {
            OverlayTarget::Grip => Icon::Grip,
            OverlayTarget::Profile => Icon::Profile,
            OverlayTarget::Maximize if maximized => Icon::Restore,
            OverlayTarget::Maximize => Icon::Maximize,
            OverlayTarget::Close => Icon::Close,
        };
        icons.extend(icon_shapes(icon, rect, line, theme::ICON));
    }
    (out, icons)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Orientation, SplitTree};
    use crate::ui::overlay::overlay_layout;
    use crate::ui::toast::ToastSource;
    use crate::ui::topbar::BarMetrics;

    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        w: 201,
        h: 101,
    };

    fn defaults() -> ChromeColors {
        ChromeColors::of(&GlobalConfig::default())
    }

    fn with(rects: &[RectInstance], color: [f32; 4]) -> Vec<Rect> {
        rects
            .iter()
            .filter(|r| r.color == color)
            .map(|r| r.rect)
            .collect()
    }

    #[test]
    fn clear_color_rule() {
        let bg = Rgb {
            r: 0x1e,
            g: 0x1e,
            b: 0x1e,
        };
        assert_eq!(frame_clear(true, bg), [0.0; 4]);
        let v = f32::from(0x1e_u8) / 255.0;
        assert_eq!(frame_clear(false, bg), [v, v, v, 1.0]);
    }

    #[test]
    fn single_pane_has_no_chrome() {
        let t = SplitTree::new(1);
        assert!(chrome(&t.layout(AREA), &t.dividers(AREA), 1, &defaults()).is_empty());
    }

    #[test]
    fn maximized_pane_has_no_chrome() {
        let mut t = SplitTree::new(1);
        t.split(1, Orientation::Horizontal, 2);
        t.toggle_maximize(2);
        assert!(chrome(&t.layout(AREA), &t.dividers(AREA), 2, &defaults()).is_empty());
    }

    #[test]
    fn two_panes_get_divider_dim_and_border() {
        let mut t = SplitTree::new(1);
        t.split(1, Orientation::Horizontal, 2);
        let out = chrome(&t.layout(AREA), &t.dividers(AREA), 2, &defaults());

        let c = defaults();
        let gray = with(&out, c.divider);
        assert_eq!(
            gray,
            vec![Rect {
                x: 100,
                y: 0,
                w: 1,
                h: 101
            }]
        );
        assert_eq!(
            c.divider,
            [
                0x55 as f32 / 255.0,
                0x55 as f32 / 255.0,
                0x55 as f32 / 255.0,
                1.0
            ]
        );

        // Pane 1 (0..100) is unfocused: dimmed by black at 0.25.
        assert_eq!(
            with(&out, [0.0, 0.0, 0.0, 0.25]),
            vec![Rect {
                x: 0,
                y: 0,
                w: 100,
                h: 101
            }]
        );

        // Pane 2 (101..201) gets a 2px border inside its rect.
        let mut border = with(&out, c.border);
        border.sort_by_key(|r| (r.x, r.y, r.w, r.h));
        assert_eq!(
            border,
            // Edges that don't overlap: the sides run between the top and bottom.
            vec![
                Rect {
                    x: 101,
                    y: 0,
                    w: 100,
                    h: 2
                },
                Rect {
                    x: 101,
                    y: 2,
                    w: 2,
                    h: 97
                },
                Rect {
                    x: 101,
                    y: 99,
                    w: 100,
                    h: 2
                },
                Rect {
                    x: 199,
                    y: 2,
                    w: 2,
                    h: 97
                },
            ]
        );
        assert_eq!(
            c.border,
            [
                0x4a as f32 / 255.0,
                0x90 as f32 / 255.0,
                0xd9 as f32 / 255.0,
                1.0
            ]
        );
        assert_eq!(out.len(), 1 + 1 + 4);
    }

    #[test]
    fn dim_goes_under_dividers_and_border() {
        let mut t = SplitTree::new(1);
        t.split(1, Orientation::Vertical, 2);
        let out = chrome(&t.layout(AREA), &t.dividers(AREA), 1, &defaults());
        assert_eq!(out[0].color, defaults().dim);
        assert_eq!(out.last().unwrap().color, defaults().border);
    }

    #[test]
    fn chrome_uses_configured_colors() {
        let g = GlobalConfig::parse(
            "focus_border_color = \"#ff0000\"\ndivider_color = \"#00ff00\"\ndim_amount = 0.5\n",
            std::path::Path::new("config.toml"),
        )
        .unwrap();
        let c = ChromeColors::of(&g);
        assert_eq!(c.border, theme::rgb(0xff0000));
        assert_eq!(c.divider, theme::rgb(0x00ff00));
        assert_eq!(c.dim, [0.0, 0.0, 0.0, 0.5]);
        let mut t = SplitTree::new(1);
        t.split(1, Orientation::Horizontal, 2);
        let out = chrome(&t.layout(AREA), &t.dividers(AREA), 2, &c);
        assert_eq!(with(&out, c.border).len(), 4);
        assert_eq!(with(&out, c.divider).len(), 1);
        assert_eq!(with(&out, c.dim).len(), 1);
    }

    #[test]
    fn paint_toast_frame_and_text() {
        let bm = BarMetrics {
            height: 28,
            pad: 6,
            cell_w: 8,
            cell_h: 16,
        };
        let t = Toast::new(
            "config.toml:1: bad",
            ToastSource::ConfigReload,
            std::time::Instant::now(),
        );
        let r = t.rect(800, 600, &bm);
        let p = paint_toast(&t, 800, 600, &bm, 1);
        assert_eq!(with(&p.under, theme::TOAST_BORDER), vec![r]);
        assert_eq!(with(&p.under, theme::TOAST_BG), vec![r.inset(1)]);
        assert_eq!(p.labels.len(), 1);
        let l = &p.labels[0];
        assert_eq!(
            (l.text.as_str(), l.color),
            ("config.toml:1: bad", theme::to_rgb(theme::TOAST_TEXT))
        );
        assert_eq!((l.x, l.y), (r.x + 6, r.y + 6));
        // Narrow window: truncated to the toast.
        let t = Toast::new(
            &"x".repeat(100),
            ToastSource::ConfigReload,
            std::time::Instant::now(),
        );
        let p = paint_toast(&t, 200, 600, &bm, 1);
        let cols = (t.rect(200, 600, &bm).w - 12) / 8;
        assert_eq!(p.labels[0].text.chars().count() as i32, cols);
        assert!(p.labels[0].text.ends_with('…'));
        // A long first error with more behind it: the suffix stays whole.
        let t = Toast::from_first(
            &"x".repeat(100),
            3,
            ToastSource::ConfigReload,
            std::time::Instant::now(),
        );
        let p = paint_toast(&t, 200, 600, &bm, 1);
        let cols = (t.rect(200, 600, &bm).w - 12) / 8;
        assert_eq!(p.labels[0].text.chars().count() as i32, cols);
        assert!(p.labels[0].text.ends_with("… (+2 more)"));
        // No room: nothing.
        let p = paint_toast(&t, 0, 0, &bm, 1);
        assert!(p.under.is_empty() && p.labels.is_empty());
    }

    #[test]
    fn link_label_frame_and_cut_text() {
        let bm = BarMetrics {
            height: 28,
            pad: 6,
            cell_w: 8,
            cell_h: 16,
        };
        let r = Rect {
            x: 10,
            y: 20,
            w: 100,
            h: 28,
        };
        let p = paint_label_box(r, "https://a.b", &bm, 1);
        assert_eq!(with(&p.under, theme::OVERLAY_BORDER), vec![r]);
        assert_eq!(with(&p.under, theme::OVERLAY_BG), vec![r.inset(1)]);
        let l = &p.labels[0];
        assert_eq!(
            (l.text.as_str(), l.color, l.x, l.y),
            ("https://a.b", theme::to_rgb(theme::TEXT), 16, 26)
        );
        // 11 columns of text room: cut with an ellipsis.
        let p = paint_label_box(r, "https://example.com/long", &bm, 1);
        assert_eq!(p.labels[0].text, "https://ex…");
    }

    #[test]
    fn info_toast_uses_neutral_colors() {
        let bm = BarMetrics {
            height: 28,
            pad: 6,
            cell_w: 8,
            cell_h: 16,
        };
        let t = Toast::new(
            "saved session 'x'",
            ToastSource::Info,
            std::time::Instant::now(),
        );
        let r = t.rect(800, 600, &bm);
        let p = paint_toast(&t, 800, 600, &bm, 1);
        assert_eq!(with(&p.under, theme::PANEL_BORDER), vec![r]);
        assert_eq!(with(&p.under, theme::PANEL_BG), vec![r.inset(1)]);
        assert!(with(&p.under, theme::TOAST_BG).is_empty());
        assert_eq!(p.labels[0].color, theme::to_rgb(theme::TEXT));
    }

    fn overlay() -> Overlay {
        let bm = BarMetrics {
            height: 28,
            pad: 6,
            cell_w: 8,
            cell_h: 16,
        };
        overlay_layout(
            Rect {
                x: 0,
                y: 28,
                w: 300,
                h: 200,
            },
            &bm,
            4,
            false,
        )
    }

    #[test]
    fn paint_overlay_frame_and_icons() {
        let o = overlay();
        let (out, shapes) = paint_overlay(&o, None, false, 1, 2.0);
        // The translucent bg must not cover the border: it fills only the inside.
        assert_eq!(with(&out, theme::OVERLAY_BG), vec![o.bg.inset(1)]);
        let border = with(&out, theme::OVERLAY_BORDER);
        let area: i32 = border.iter().map(|r| r.w * r.h).sum();
        assert_eq!(area, o.bg.w * o.bg.h - o.bg.inset(1).w * o.bg.inset(1).h);
        for r in &border {
            assert!(o.bg.contains(r.x, r.y) && !o.bg.inset(1).contains(r.x, r.y));
        }
        assert!(with(&out, theme::HOVER).is_empty());
        assert!(with(&out, theme::CLOSE_HOVER).is_empty());
        let (pr, m, c) = (o.items[0].0, o.items[1].0, o.items[2].0);
        let mut icons = icon_shapes(Icon::Profile, pr, 2.0, theme::ICON);
        icons.extend(icon_shapes(Icon::Maximize, m, 2.0, theme::ICON));
        icons.extend(icon_shapes(Icon::Close, c, 2.0, theme::ICON));
        assert_eq!(shapes, icons);
        // Maximized: Restore.
        let (_, shapes) = paint_overlay(&o, None, true, 1, 2.0);
        let mut icons = icon_shapes(Icon::Profile, pr, 2.0, theme::ICON);
        icons.extend(icon_shapes(Icon::Restore, m, 2.0, theme::ICON));
        icons.extend(icon_shapes(Icon::Close, c, 2.0, theme::ICON));
        assert_eq!(shapes, icons);
    }

    #[test]
    fn paint_overlay_hover_within_border() {
        let o = overlay();
        let (pr, m, c) = (o.items[0].0, o.items[1].0, o.items[2].0);
        let inner = o.bg.inset(2);
        let (out, _) = paint_overlay(&o, Some(OverlayTarget::Close), false, 2, 4.0);
        assert_eq!(
            with(&out, theme::CLOSE_HOVER),
            vec![Rect {
                x: c.x,
                y: inner.y,
                w: inner.x + inner.w - c.x,
                h: inner.h
            }]
        );
        assert!(with(&out, theme::HOVER).is_empty());
        let (out, _) = paint_overlay(&o, Some(OverlayTarget::Maximize), false, 2, 4.0);
        assert_eq!(
            with(&out, theme::HOVER),
            vec![Rect {
                x: m.x,
                y: inner.y,
                w: m.w,
                h: inner.h
            }]
        );
        let (out, _) = paint_overlay(&o, Some(OverlayTarget::Profile), false, 2, 4.0);
        assert_eq!(
            with(&out, theme::HOVER),
            vec![Rect {
                x: inner.x,
                y: inner.y,
                w: pr.x + pr.w - inner.x,
                h: inner.h
            }]
        );
        // The grip, when shown, hovers like Profile.
        let bm = BarMetrics {
            height: 28,
            pad: 6,
            cell_w: 8,
            cell_h: 16,
        };
        let pane = Rect {
            x: 0,
            y: 28,
            w: 300,
            h: 200,
        };
        let og = overlay_layout(pane, &bm, 4, true);
        assert_eq!(og.items[0].1, OverlayTarget::Grip);
        let (out, icons) = paint_overlay(&og, Some(OverlayTarget::Grip), false, 2, 4.0);
        assert_eq!(with(&out, theme::HOVER).len(), 1);
        // The icons are strokes drawn over the rects, so the hover is under them.
        assert!(icons.len() > 6 && icons.iter().all(|i| i.color == theme::ICON));
    }

    #[test]
    fn empty_overlay_paints_nothing() {
        let o = Overlay {
            bg: Rect {
                x: 0,
                y: 0,
                w: 0,
                h: 0,
            },
            items: Vec::new(),
        };
        let (rects, shapes) = paint_overlay(&o, Some(OverlayTarget::Close), true, 1, 2.0);
        assert!(rects.is_empty() && shapes.is_empty());
    }
}
