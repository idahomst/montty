//! Mouse: an open modal takes everything (`modal.rs`), then a click on the
//! toast dismisses it, then the top bar (`bar.rs`), then the find bar (`find.rs`; a
//! press on a pane closes it), then the pane overlay's buttons, divider hover/drag,
//! then selection (`selection.rs`, which also focuses the pane) and wheel scrolling.

use std::time::Instant;

use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::term::TermMode;
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, MouseButton, MouseScrollDelta};
use winit::window::CursorIcon;

use super::Window;
use super::bar::ClickTracker;
use super::links::pointer_icon;
use super::pane_drag::{DragEnd, PaneDrag, drag_threshold};
use super::selection::SelectState;
use crate::clipboard::ClipKind;
use crate::input::Mods;
use crate::input::bindings::Action;
use crate::input::mouse::{MouseButtonCode, MouseEventKind, report, reporting};
use crate::layout::{Divider, DividerId, Orientation, PaneId, Rect};
use crate::term::grid_point::point_at;
use crate::ui::edge_px;
use crate::ui::overlay::{Overlay, OverlayTarget, overlay_hit, overlay_layout, overlay_visible};

/// Lines scrolled per wheel notch.
const LINES_PER_NOTCH: f64 = 3.0;

#[derive(Default)]
pub(super) struct MouseState {
    /// Last pointer position, window pixels.
    pos: (i32, i32),
    /// False after the pointer left the window (until it moves in again).
    inside: bool,
    /// Last pointer motion (shows the pane overlay until the next key there).
    last_move: Option<Instant>,
    /// Double-click detection for the top bar.
    pub(super) clicks: ClickTracker,
    /// Double-click detection for dividers (evens out the panes along one).
    divider_clicks: ClickTracker<DividerId>,
    drag: Option<Drag>,
    /// A press on a pane's overlay, or the pane being dragged from it.
    pub(super) pane_drag: Option<PaneDrag>,
    pub(super) select: SelectState,
    icon: CursorIcon,
    /// Fractional wheel lines not scrolled yet.
    wheel: f64,
    /// The button whose press went to a program: its drag motion and release
    /// go to that pane too, wherever the pointer is by then.
    held: Held,
    /// The last screen cell reported for motion (motion only on a change).
    last_cell: Option<(PaneId, (usize, usize))>,
}

impl MouseState {
    /// A left press while a pane drag is armed means its release was lost: the
    /// drag ends. True when it did.
    fn lost_pane_release(&mut self, state: ElementState, button: MouseButton) -> bool {
        let lost = self.pane_drag.is_some()
            && state == ElementState::Pressed
            && button == MouseButton::Left;
        if lost {
            self.pane_drag = None;
        }
        lost
    }

    /// Divider ids are tree paths: a drag, or half a double-click on a divider, must not
    /// outlive a tree change.
    pub(super) fn cancel_drag(&mut self) {
        self.drag = None;
        self.pane_drag = None;
        self.select.drag = None;
        self.held.clear();
        self.divider_clicks.clear();
    }

    /// A divider drag, a pane drag or a selection drag is on.
    pub(super) fn dragging(&self) -> bool {
        self.drag.is_some() || self.pane_drag.is_some() || self.select.drag.is_some()
    }

    pub(super) fn pos(&self) -> (i32, i32) {
        self.pos
    }

    pub(super) fn inside(&self) -> bool {
        self.inside
    }
}

/// The reported buttons that are down, with the pane each press went to: a drag's
/// motion and each release follow those panes, wherever the pointer is by then.
#[derive(Default)]
struct Held(Vec<(PaneId, MouseButtonCode)>);

impl Held {
    fn press(&mut self, id: PaneId, code: MouseButtonCode) {
        self.release(code);
        self.0.push((id, code));
    }

    /// `code` was released: the pane its press went to, if it was down.
    fn release(&mut self, code: MouseButtonCode) -> Option<PaneId> {
        let i = self.0.iter().position(|&(_, c)| c == code)?;
        Some(self.0.remove(i).0)
    }

    /// The button a drag reports (xterm: the lowest held) and its pane.
    fn drag(&self) -> Option<(PaneId, MouseButtonCode)> {
        self.0.iter().copied().min_by_key(|&(_, c)| c)
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn clear(&mut self) {
        self.0.clear();
    }
}

struct Drag {
    id: DividerId,
    orientation: Orientation,
    /// Divider line position minus the pointer position at press, so the line doesn't jump.
    offset: i32,
}

/// Coordinate along a split's axis (x for side-by-side, y for stacked).
fn along(o: Orientation, (x, y): (i32, i32)) -> i32 {
    match o {
        Orientation::Horizontal => x,
        Orientation::Vertical => y,
    }
}

impl Window {
    pub(super) fn on_cursor_moved(&mut self, p: PhysicalPosition<f64>) {
        self.mouse.pos = (p.x.floor() as i32, p.y.floor() as i32);
        self.mouse.inside = true;
        self.mouse.last_move = Some(Instant::now());
        let area = self.area();
        if let Some(mut d) = self.mouse.pane_drag {
            let layout = self.tabs.active().tree.layout(area);
            let (pos, inside) = (self.mouse.pos, self.mouse.inside);
            let changed = d.motion(pos, inside, drag_threshold(self.scale), &layout);
            self.mouse.pane_drag = Some(d);
            if matches!(d, PaneDrag::Dragging { .. }) {
                self.set_cursor_icon(CursorIcon::Grabbing);
            }
            if changed {
                self.schedule();
            }
            return;
        }
        if self.mouse.select.drag.is_some() {
            self.selection_motion();
            return;
        }
        // Ctrl over a link is for montty: the program gets no motion for it (unless a
        // reported button is held).
        self.update_link_hover();
        if (!self.mouse.held.is_empty() || self.links.hover.is_none()) && self.report_motion() {
            return;
        }
        if let Some(d) = &self.mouse.drag {
            let pos = along(d.orientation, self.mouse.pos) + d.offset;
            self.tabs.active_mut().tree.set_divider(&d.id, pos, area);
            let ids = self.relayout();
            self.label_resized(ids);
            return;
        }
        if self.modal_open() {
            self.modal_pointer_moved();
        } else if !self.bar_pointer_moved() {
            let (x, y) = self.mouse.pos;
            let divider =
                divider_at(&self.tabs.active().tree.dividers(area), x, y).map(|d| d.orientation);
            let hovered = self.links.hover.is_some();
            self.set_cursor_icon(pointer_icon(hovered, divider));
        }
        self.refresh_overlay();
        self.update_mark_hover();
    }

    pub(super) fn on_cursor_left(&mut self) {
        self.mouse.inside = false;
        if let Some(PaneDrag::Dragging { over, .. }) = &mut self.mouse.pane_drag
            && over.take().is_some()
        {
            self.schedule();
        }
        self.update_link_hover();
        self.mouse.last_cell = None;
        self.bar_pointer_left();
        self.refresh_overlay();
        self.update_mark_hover();
    }

    /// Show `icon` as the pointer shape (the OS is told only on a change).
    pub(super) fn set_cursor_icon(&mut self, icon: CursorIcon) {
        if icon != self.mouse.icon {
            self.mouse.icon = icon;
            self.window.set_cursor(icon);
        }
    }

    pub(super) fn on_mouse_input(&mut self, state: ElementState, button: MouseButton) {
        if state == ElementState::Pressed {
            self.had_input = true;
        }
        if self.mouse.pane_drag.is_some() && self.mouse.lost_pane_release(state, button) {
            // The release never came (a compositor grab): this press is a fresh one.
            self.set_cursor_icon(CursorIcon::Default);
            self.refresh_overlay();
        }
        if let Some(d) = self.mouse.pane_drag {
            if state == ElementState::Released && button == MouseButton::Left {
                self.mouse.pane_drag = None;
                let layout = self.tabs.active().tree.layout(self.area());
                match d.release(self.mouse.pos, self.mouse.inside, &layout) {
                    DragEnd::Click {
                        pane,
                        target,
                        button,
                    } => self.overlay_click(pane, target, button),
                    DragEnd::Swap { pane, with } => self.swap_panes(pane, with),
                    DragEnd::Cancel => {}
                }
                self.set_cursor_icon(CursorIcon::Default);
                self.refresh_overlay();
            }
            return;
        }
        if state == ElementState::Pressed && self.modal_open() {
            self.modal_press(button);
            return;
        }
        if state == ElementState::Pressed && self.mouse.drag.is_none() && self.toast_press() {
            return;
        }
        if state == ElementState::Pressed && self.mouse.drag.is_none() && self.bar_press(button) {
            return;
        }
        if state == ElementState::Pressed
            && self.mouse.drag.is_none()
            && self.find.is_some()
            && self.find_press(button)
        {
            return;
        }
        if state == ElementState::Pressed && self.mouse.drag.is_none() && self.overlay_press(button)
        {
            return;
        }
        // Ahead of mouse reporting: Ctrl+click opens a link even for a program that
        // asked for mouse events. A Ctrl press off a link falls through unchanged.
        if state == ElementState::Pressed
            && button == MouseButton::Left
            && self.mouse.drag.is_none()
            && self.link_press()
        {
            return;
        }
        if state == ElementState::Released && self.report_release(button) {
            return;
        }
        if state == ElementState::Pressed && self.mouse.drag.is_none() && self.report_press(button)
        {
            return;
        }
        if button == MouseButton::Middle && state == ElementState::Pressed {
            self.middle_press();
            return;
        }
        if button != MouseButton::Left {
            return;
        }
        if state == ElementState::Released {
            if self.mouse.drag.take().is_some() {
                self.layout_changed();
            }
            self.selection_release();
            self.refresh_overlay();
            return;
        }
        let area = self.area();
        let (x, y) = self.mouse.pos;
        if let Some(d) = divider_at(&self.tabs.active().tree.dividers(area), x, y) {
            self.mouse.clicks.clear();
            if self
                .mouse
                .divider_clicks
                .press(d.id.clone(), 0, Instant::now())
            {
                let id = d.id.clone();
                self.tabs.active_mut().tree.equalize(&id);
                let ids = self.relayout();
                self.label_resized(ids);
                self.layout_changed();
                return;
            }
            let line = (d.line.x, d.line.y);
            self.mouse.drag = Some(Drag {
                id: d.id.clone(),
                orientation: d.orientation,
                offset: along(d.orientation, line) - along(d.orientation, self.mouse.pos),
            });
            // No overlay while dragging.
            self.refresh_overlay();
        } else if let Some((id, rect)) = self.pane_at(x, y) {
            self.selection_press(id, rect);
        }
    }

    /// A press in a pane whose program asked for mouse reports (and no Shift):
    /// focus the pane and report the press, so no selection or paste happens.
    /// A left press on a divider is left to the divider. True when consumed.
    fn report_press(&mut self, button: MouseButton) -> bool {
        let code = match button {
            MouseButton::Left => MouseButtonCode::Left,
            MouseButton::Middle => MouseButtonCode::Middle,
            MouseButton::Right => MouseButtonCode::Right,
            _ => return false,
        };
        let (x, y) = self.mouse.pos;
        let Some((id, rect)) = self.pane_at(x, y) else {
            return false;
        };
        let Some(mode) = self.pane_mode(id) else {
            return false;
        };
        if !reporting(mode, self.modifiers.shift_key()) {
            return false;
        }
        if button == MouseButton::Left
            && divider_at(&self.tabs.active().tree.dividers(self.area()), x, y).is_some()
        {
            return false;
        }
        self.mouse.clicks.clear();
        if id != self.tabs.active().focused {
            self.set_focus(id);
        }
        self.mouse.held.press(id, code);
        self.send_report(id, rect, Some(code), MouseEventKind::Press);
        true
    }

    /// The release of the button whose press was reported goes to that pane,
    /// wherever the pointer is. True when consumed.
    fn report_release(&mut self, button: MouseButton) -> bool {
        let code = match button {
            MouseButton::Left => MouseButtonCode::Left,
            MouseButton::Middle => MouseButtonCode::Middle,
            MouseButton::Right => MouseButtonCode::Right,
            _ => return false,
        };
        let Some(id) = self.mouse.held.release(code) else {
            return false;
        };
        self.mouse.last_cell = None;
        if let Some(rect) = self.pane_rect(id) {
            self.send_report(id, rect, Some(code), MouseEventKind::Release);
        }
        true
    }

    /// Pointer motion for a reporting program: while a reported button is held
    /// (in that pane), else over a pane in any-motion mode. Sent only when the
    /// cell changed. True when the motion belongs to the held drag (nothing else
    /// reacts to it).
    fn report_motion(&mut self) -> bool {
        if let Some((id, code)) = self.mouse.held.drag() {
            // A modal opened mid-hold ends it.
            if self.modal_open() {
                self.mouse.held.clear();
                self.mouse.last_cell = None;
                return false;
            }
            if let Some(rect) = self.pane_rect(id) {
                self.send_report(id, rect, Some(code), MouseEventKind::Motion);
            }
            return true;
        }
        let over_overlay_button = matches!(self.overlay_at_pointer(), Some((_, _, Some(_))));
        if self.modal_open() || self.mouse.drag.is_some() || over_overlay_button {
            return false;
        }
        let (x, y) = self.mouse.pos;
        if let Some((id, rect)) = self.pane_at(x, y)
            && let Some(mode) = self.pane_mode(id)
            && mode.contains(TermMode::MOUSE_MOTION)
            && reporting(mode, self.modifiers.shift_key())
        {
            self.send_report(id, rect, None, MouseEventKind::Motion);
        }
        false
    }

    /// The program's mouse mode in pane `id`.
    fn pane_mode(&self, id: PaneId) -> Option<TermMode> {
        Some(*self.panes.get(&id)?.term.lock().mode())
    }

    pub(super) fn pane_rect(&self, id: PaneId) -> Option<Rect> {
        self.tabs
            .active()
            .tree
            .layout(self.area())
            .into_iter()
            .find_map(|(p, r)| (p == id).then_some(r))
    }

    /// The screen cell (0-based, ignoring scrollback) under the pointer in pane
    /// `id` at `rect`, clamped into the grid.
    fn screen_cell(&mut self, id: PaneId, rect: Rect) -> Option<(usize, usize)> {
        let font = self.pane_font(id);
        let content = rect.inset(font.padding);
        let (cw, ch) = (font.metrics.cell_width, font.metrics.cell_height);
        let (x, y) = self.mouse.pos;
        let term = self.panes.get(&id)?.term.lock();
        let (cols, lines) = (term.columns(), term.screen_lines());
        drop(term);
        let (p, _) = point_at(content, cw, ch, cols, lines, 0, x, y);
        Some((p.column.0, p.line.0.max(0) as usize))
    }

    /// Send the program in pane `id` this pointer event at the pointer's cell,
    /// as far as its mode wants it. Motion repeating the last cell is dropped.
    fn send_report(
        &mut self,
        id: PaneId,
        rect: Rect,
        button: Option<MouseButtonCode>,
        kind: MouseEventKind,
    ) {
        let Some((col, line)) = self.screen_cell(id, rect) else {
            return;
        };
        if kind == MouseEventKind::Motion && self.mouse.last_cell == Some((id, (col, line))) {
            return;
        }
        self.mouse.last_cell = Some((id, (col, line)));
        let Some(mode) = self.pane_mode(id) else {
            return;
        };
        let mods = Mods::from(self.modifiers);
        if let Some(bytes) = report(button, kind, col, line, mods, mode)
            && let Some(pane) = self.panes.get(&id)
        {
            pane.write(bytes);
        }
    }

    /// Middle press: paste PRIMARY into the pane under the pointer (focus stays).
    /// (Mouse reporting, with Shift as the bypass, takes this press first.)
    fn middle_press(&mut self) {
        if self.mouse.drag.is_some() || self.mouse.select.drag.is_some() {
            return;
        }
        let (x, y) = self.mouse.pos;
        if let Some((id, _)) = self.pane_at(x, y) {
            self.paste_into(id, ClipKind::Primary);
        }
    }

    pub(super) fn on_wheel(&mut self, delta: MouseScrollDelta) {
        // An open modal: only the info panel scrolls, under the pointer.
        if self.modal_open() {
            if self.pointer_over_panel() {
                let cell_h = self.ui_metrics.cell_height;
                let lines = wheel_lines(delta, &mut self.mouse.wheel, cell_h);
                self.scroll_panel(-(lines as isize));
            }
            return;
        }
        let (x, y) = self.mouse.pos;
        let Some((id, rect)) = self.pane_at(x, y) else {
            return;
        };
        // The pane's own line height: panes may use different fonts.
        let cell_h = self.pane_font(id).metrics.cell_height;
        let lines = wheel_lines(delta, &mut self.mouse.wheel, cell_h);
        let Some(pane) = self.panes.get(&id).filter(|_| lines != 0) else {
            return;
        };
        let mode = *pane.term.lock().mode();
        if reporting(mode, self.modifiers.shift_key()) {
            let code = if lines > 0 {
                MouseButtonCode::WheelUp
            } else {
                MouseButtonCode::WheelDown
            };
            for _ in 0..lines.unsigned_abs() {
                self.send_report(id, rect, Some(code), MouseEventKind::Press);
            }
        } else if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
            pane.write(alt_scroll_bytes(lines, mode.contains(TermMode::APP_CURSOR)));
        } else {
            pane.scroll(Scroll::Delta(lines));
            self.mark_scrolled(id);
            self.schedule();
        }
    }

    /// Whether the active tab offers the pane drag: `pane_drag` is on, no pane is
    /// maximized and there is another pane to drop on.
    fn pane_drag_allowed(&self) -> bool {
        let tree = &self.tabs.active().tree;
        pane_drag_offered(
            self.store.global().pane_drag,
            tree.maximized().is_some(),
            tree.panes().len(),
        )
    }

    /// Pane overlay inset: `edge_px(scale)` (4 px at scale 1), so HiDPI keeps
    /// the same look; it also pads the buttons over a cell. It equals the
    /// edge-resize zone's width, so the overlay never overlaps that zone.
    pub(super) fn overlay_inset(&self) -> i32 {
        edge_px(self.scale)
    }

    /// The overlay of the pane under the pointer, with that pane and the
    /// hovered button. None while a modal is open or a divider is dragged,
    /// off the panes (over the bar, outside the window), after a key press
    /// until the pointer moves again, or in a pane too small for it.
    /// (No tabs left: the overlay's Close just closed the last pane.)
    pub(super) fn overlay_at_pointer(&self) -> Option<(PaneId, Overlay, Option<OverlayTarget>)> {
        if self.tabs.is_empty()
            || self.modal_open()
            || self.mouse.drag.is_some()
            || matches!(self.mouse.pane_drag, Some(PaneDrag::Dragging { .. }))
        {
            return None;
        }
        let last_move = self.mouse.last_move?;
        let (x, y) = self.mouse.pos;
        let pane = self.pane_at(x, y).filter(|_| self.mouse.inside);
        if !overlay_visible(pane.is_some(), last_move, self.last_key) {
            return None;
        }
        let (id, rect) = pane?;
        // The find bar sits where the overlay would.
        if self.find.as_ref().is_some_and(|f| f.pane == id) {
            return None;
        }
        let grip = self.pane_drag_allowed();
        let o = overlay_layout(rect, &self.bar_metrics(), self.overlay_inset(), grip);
        if o.items.is_empty() {
            return None;
        }
        let hover = overlay_hit(&o, x, y);
        Some((id, o, hover))
    }

    /// Recompute the overlay from the last pointer position; a redraw only
    /// when its pane, visibility or hovered button changed.
    pub(super) fn refresh_overlay(&mut self) {
        let now = self.overlay_at_pointer().map(|(id, _, hover)| (id, hover));
        if now != self.overlay {
            self.overlay = now;
            self.schedule();
        }
    }

    /// A press on an overlay button is consumed (no divider drag or pane
    /// click). With `pane_drag` on (and no pane maximized, and several panes) a left press only starts a
    /// `PaneDrag`: the button acts on release, unless the pointer moved far enough to
    /// drag the pane. Otherwise a left press acts at once (`overlay_click`).
    /// True when consumed.
    fn overlay_press(&mut self, button: MouseButton) -> bool {
        let Some((id, o, Some(target))) = self.overlay_at_pointer() else {
            return false;
        };
        self.mouse.clicks.clear();
        if button == MouseButton::Left
            && let Some(&(rect, _)) = o.items.iter().find(|&&(_, t)| t == target)
        {
            if self.pane_drag_allowed() {
                self.mouse.pane_drag = Some(PaneDrag::Pressed {
                    pane: id,
                    target,
                    button: rect,
                    start: self.mouse.pos,
                });
                return true;
            }
            self.overlay_click(id, target, rect);
        }
        self.refresh_overlay();
        true
    }

    /// A click on `target`, drawn at `button`, in pane `id`'s overlay. Profile opens that
    /// pane's profile menu (the pane takes the focus when a profile is chosen); Maximize
    /// and Close focus the pane, then maximize/restore or close it as the key bindings
    /// do. The grip does nothing.
    fn overlay_click(&mut self, id: PaneId, target: OverlayTarget, button: Rect) {
        let action = match target {
            OverlayTarget::Grip => return,
            OverlayTarget::Profile => {
                self.open_profile_menu(id, button);
                return;
            }
            OverlayTarget::Maximize => Action::MaximizePane,
            OverlayTarget::Close => Action::ClosePane,
        };
        if id != self.tabs.active().focused {
            self.set_focus(id);
        }
        self.perform(action);
    }

    /// A press on the toast dismisses it and is consumed. True when consumed.
    fn toast_press(&mut self) -> bool {
        let (w, h) = self.size_px();
        let (x, y) = self.mouse.pos;
        let hit = self
            .toast
            .as_ref()
            .is_some_and(|t| t.rect(w, h, &self.bar_metrics()).contains(x, y));
        if hit {
            self.mouse.clicks.clear();
            self.dismiss_toast();
        }
        hit
    }

    /// The visible pane under (x, y).
    pub(super) fn pane_at(&self, x: i32, y: i32) -> Option<(PaneId, Rect)> {
        self.tabs
            .active()
            .tree
            .layout(self.area())
            .into_iter()
            .find(|(_, r)| r.contains(x, y))
    }
}

/// The divider whose hit zone contains (x, y); outer splits win where zones overlap.
pub(super) fn divider_at(divs: &[Divider], x: i32, y: i32) -> Option<&Divider> {
    divs.iter().find(|d| d.hit.contains(x, y))
}

/// Pointer shape over a divider of a split with orientation `o`.
pub(super) fn resize_icon(o: Orientation) -> CursorIcon {
    match o {
        Orientation::Horizontal => CursorIcon::ColResize,
        Orientation::Vertical => CursorIcon::RowResize,
    }
}

/// Whole lines to scroll for `delta` (positive = up, into history). Fractions
/// (touchpads, high-resolution wheels) accumulate in `acc`.
pub(super) fn wheel_lines(delta: MouseScrollDelta, acc: &mut f64, cell_height: u32) -> i32 {
    *acc += match delta {
        MouseScrollDelta::LineDelta(_, y) => f64::from(y) * LINES_PER_NOTCH,
        MouseScrollDelta::PixelDelta(p) => p.y / f64::from(cell_height.max(1)),
    };
    let whole = acc.trunc();
    *acc -= whole;
    whole.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

/// Alternate scroll: `lines` Up (positive) or Down (negative) arrow key sequences.
pub(super) fn alt_scroll_bytes(lines: i32, app_cursor: bool) -> Vec<u8> {
    let prefix: &[u8] = if app_cursor { b"\x1bO" } else { b"\x1b[" };
    let key = if lines > 0 { b'A' } else { b'B' };
    let mut out = Vec::new();
    for _ in 0..lines.unsigned_abs() {
        out.extend_from_slice(prefix);
        out.push(key);
    }
    out
}

/// The pane drag is on, nothing is maximized and the tab has a second pane.
fn pane_drag_offered(enabled: bool, maximized: bool, panes: usize) -> bool {
    enabled && !maximized && panes >= 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_left_press_ends_an_armed_pane_drag() {
        let armed = || MouseState {
            pane_drag: Some(PaneDrag::Dragging {
                pane: 1,
                over: None,
            }),
            ..Default::default()
        };
        let mut m = armed();
        assert!(m.lost_pane_release(ElementState::Pressed, MouseButton::Left));
        assert!(m.pane_drag.is_none());
        for (s, b) in [
            (ElementState::Released, MouseButton::Left),
            (ElementState::Pressed, MouseButton::Right),
        ] {
            let mut m = armed();
            assert!(!m.lost_pane_release(s, b));
            assert!(m.pane_drag.is_some());
        }
        let mut idle = MouseState::default();
        assert!(!idle.lost_pane_release(ElementState::Pressed, MouseButton::Left));
    }

    #[test]
    fn pane_drag_needs_a_second_visible_pane() {
        assert!(pane_drag_offered(true, false, 2));
        assert!(!pane_drag_offered(true, false, 1));
        assert!(!pane_drag_offered(true, true, 3));
        assert!(!pane_drag_offered(false, false, 3));
    }
    use crate::layout::SplitTree;
    use winit::dpi::PhysicalPosition;

    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        w: 201,
        h: 101,
    };

    #[test]
    fn cancel_drag_ends_a_pane_drag() {
        let mut m = MouseState {
            pane_drag: Some(PaneDrag::Dragging {
                pane: 1,
                over: None,
            }),
            ..Default::default()
        };
        assert!(m.dragging());
        m.cancel_drag();
        assert!(m.pane_drag.is_none());
        assert!(!m.dragging());
    }

    #[test]
    fn divider_double_click_needs_the_same_divider() {
        let t0 = Instant::now();
        let ms = |n| t0 + std::time::Duration::from_millis(n);
        let mut c = ClickTracker::default();
        assert!(!c.press(DividerId(vec![]), 0, t0));
        assert!(c.press(DividerId(vec![]), 0, ms(200)));
        assert!(!c.press(DividerId(vec![]), 0, ms(1000)));
        assert!(!c.press(DividerId(vec![1]), 0, ms(1100)));
    }

    #[test]
    fn divider_hit_zone_extends_three_px_each_side() {
        let mut t = SplitTree::new(1);
        t.split(1, Orientation::Horizontal, 2);
        let divs = t.dividers(AREA);
        // Line at x=100, hit zone 97..=103.
        assert!(divider_at(&divs, 97, 50).is_some());
        assert!(divider_at(&divs, 100, 0).is_some());
        assert!(divider_at(&divs, 103, 100).is_some());
        assert!(divider_at(&divs, 96, 50).is_none());
        assert!(divider_at(&divs, 104, 50).is_none());
    }

    #[test]
    fn divider_at_reports_orientation_and_id() {
        let mut t = SplitTree::new(1);
        t.split(1, Orientation::Horizontal, 2);
        t.split(2, Orientation::Vertical, 3);
        let divs = t.dividers(AREA);
        // Right half is 101..201; its vertical split's line sits at y=50.
        let d = divider_at(&divs, 150, 51).expect("row divider");
        assert_eq!(d.orientation, Orientation::Vertical);
        assert_eq!(d.id.0, vec![1]);
        // Where the two zones meet, the outer (column) divider wins.
        let d = divider_at(&divs, 102, 50).expect("column divider");
        assert_eq!(d.orientation, Orientation::Horizontal);
        assert!(divider_at(&divs, 50, 50).is_none());
    }

    #[test]
    fn divider_hit_and_drag_honour_area_offset() {
        // Pane area below a 30px top bar.
        let area = Rect {
            x: 0,
            y: 30,
            w: 201,
            h: 101,
        };
        let mut t = SplitTree::new(1);
        t.split(1, Orientation::Vertical, 2);
        let divs = t.dividers(area);
        // Row divider line at y = 30 + 50 = 80, hit zone 77..=83.
        assert!(divider_at(&divs, 50, 80).is_some());
        assert!(divider_at(&divs, 50, 77).is_some());
        assert!(divider_at(&divs, 50, 83).is_some());
        assert!(divider_at(&divs, 50, 50).is_none());
        assert!(divider_at(&divs, 50, 29).is_none());
        // Dragging to absolute y = 60 puts the line 30px into the area.
        let id = divs[0].id.clone();
        t.set_divider(&id, 60, area);
        let moved = t.dividers(area);
        assert!(divider_at(&moved, 50, 60).is_some());
        assert!(divider_at(&moved, 50, 80).is_none());
    }

    #[test]
    fn a_second_button_keeps_the_first_releasable() {
        use MouseButtonCode::{Left, Right};
        let mut h = Held::default();
        h.press(1, Left);
        h.press(1, Right);
        assert_eq!(h.release(Left), Some(1));
        assert_eq!(h.release(Right), Some(1));
        assert!(h.is_empty());
        // A release with nothing down is not ours.
        assert_eq!(h.release(Left), None);
    }

    #[test]
    fn a_drag_reports_the_lowest_held_button() {
        use MouseButtonCode::{Left, Middle, Right};
        let mut h = Held::default();
        h.press(1, Right);
        h.press(1, Left);
        h.press(1, Middle);
        assert_eq!(h.drag(), Some((1, Left)));
        h.release(Left);
        assert_eq!(h.drag(), Some((1, Middle)));
        h.release(Middle);
        assert_eq!(h.drag(), Some((1, Right)));
        h.release(Right);
        assert_eq!(h.drag(), None);
    }

    #[test]
    fn divider_at_none_when_maximized() {
        let mut t = SplitTree::new(1);
        t.split(1, Orientation::Horizontal, 2);
        t.toggle_maximize(1);
        assert!(divider_at(&t.dividers(AREA), 100, 50).is_none());
    }

    #[test]
    fn resize_icon_matches_divider_direction() {
        assert_eq!(resize_icon(Orientation::Horizontal), CursorIcon::ColResize);
        assert_eq!(resize_icon(Orientation::Vertical), CursorIcon::RowResize);
    }

    #[test]
    fn wheel_notch_scrolls_three_lines() {
        let mut acc = 0.0;
        assert_eq!(
            wheel_lines(MouseScrollDelta::LineDelta(0.0, 1.0), &mut acc, 16),
            3
        );
        assert_eq!(
            wheel_lines(MouseScrollDelta::LineDelta(0.0, -2.0), &mut acc, 16),
            -6
        );
        assert_eq!(acc, 0.0);
    }

    #[test]
    fn wheel_fractions_accumulate() {
        let mut acc = 0.0;
        // A third of a notch is one line.
        assert_eq!(
            wheel_lines(MouseScrollDelta::LineDelta(0.0, 0.25), &mut acc, 16),
            0
        );
        assert_eq!(
            wheel_lines(MouseScrollDelta::LineDelta(0.0, 0.25), &mut acc, 16),
            1
        );
        // Pixel deltas count in cell heights.
        let mut acc = 0.0;
        let px = |y| MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, y));
        assert_eq!(wheel_lines(px(10.0), &mut acc, 16), 0);
        assert_eq!(wheel_lines(px(10.0), &mut acc, 16), 1);
        assert_eq!(wheel_lines(px(-36.0), &mut acc, 16), -2);
    }

    #[test]
    fn alt_scroll_sends_arrow_keys() {
        assert_eq!(alt_scroll_bytes(3, false), b"\x1b[A\x1b[A\x1b[A".to_vec());
        assert_eq!(alt_scroll_bytes(-2, false), b"\x1b[B\x1b[B".to_vec());
        assert_eq!(alt_scroll_bytes(1, true), b"\x1bOA".to_vec());
        assert_eq!(alt_scroll_bytes(-1, true), b"\x1bOB".to_vec());
        assert!(alt_scroll_bytes(0, true).is_empty());
    }
}
