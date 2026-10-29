//! The self-drawn top bar in the window: drawing, hover, clicks, window move
//! and edge resize. Layout and hit-testing are pure, in `ui::topbar`.

use std::time::Instant;

use winit::dpi::PhysicalSize;
use winit::event::MouseButton;
use winit::window::{CursorIcon, ResizeDirection};

use super::Window;
use super::attention::Level;
use super::draw::ChromeColors;
use super::marks::{disc, level_color};
use super::modal::{Modal, PromptPurpose, editing_label, resolve_rename};
use super::tabs::Tabs;
use crate::input::bindings::Action;
use crate::layout::{PaneId, Rect};
use crate::render::RectInstance;
use crate::term::colors::Rgb;
use crate::ui::confirm::CloseTarget;
use crate::ui::icons::{Icon, icon_rects};
use crate::ui::theme;
use crate::ui::topbar::{Bar, BarMetrics, Target, hit, layout, resize_edge, tab_label};
use crate::ui::{DOUBLE_CLICK, edge_px};

/// The pane area: the window below a bar `bar_h` px tall.
pub(super) fn pane_area(size: PhysicalSize<u32>, bar_h: i32) -> Rect {
    let clamp = |v: u32| v.min(i32::MAX as u32) as i32;
    let bar_h = bar_h.max(0);
    Rect {
        x: 0,
        y: bar_h,
        w: clamp(size.width),
        h: (clamp(size.height) - bar_h).max(0),
    }
}

/// Each tab's label: its rename, else its focused pane's title, else "montty".
pub(super) fn tab_labels<'a>(
    tabs: &Tabs,
    title_of: impl Fn(PaneId) -> Option<&'a str>,
) -> Vec<String> {
    tabs.iter()
        .map(|t| tab_label(t.title.as_deref(), title_of(t.focused)))
        .collect()
}

/// Detects double-clicks: two left presses on the same target within
/// `DOUBLE_CLICK`, with no tab added, removed or moved in between (a tab
/// index then names another tab). Bar targets by default; dividers too.
pub(super) struct ClickTracker<T = Target> {
    last: Option<(Instant, T, u64)>,
}

impl<T> Default for ClickTracker<T> {
    fn default() -> Self {
        Self { last: None }
    }
}

impl<T: PartialEq> ClickTracker<T> {
    /// Records a left press on `t` at `now` while the tabs are at `generation`
    /// (`Tabs::generation`); true when it completes a double-click. A
    /// double-click starts over, so a third press begins a new pair.
    pub(super) fn press(&mut self, t: T, generation: u64, now: Instant) -> bool {
        let double = self.last.as_ref().is_some_and(|(at, last, g)| {
            *last == t && *g == generation && now.saturating_duration_since(*at) < DOUBLE_CLICK
        });
        self.last = if double {
            None
        } else {
            Some((now, t, generation))
        };
        double
    }

    /// A press elsewhere (a pane, another button) breaks a pending pair.
    pub(super) fn clear(&mut self) {
        self.last = None;
    }
}

/// What a press on a bar target does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BarAction {
    SelectTab(usize),
    CloseTab(usize),
    /// Double-click on a tab: the rename field.
    RenameTab(usize),
    Perform(Action),
    /// The ≡ menu.
    OpenMenu,
    Minimize,
    ToggleMaximize,
    Close,
    DragWindow,
}

/// The action for a press of `button` on `target`; `double` when it completes a
/// double-click; `alt` when Alt is held, which turns the split buttons into split left / up.
/// Middle-click closes a tab; other buttons do nothing.
pub(super) fn press_action(
    target: Target,
    button: MouseButton,
    double: bool,
    alt: bool,
) -> Option<BarAction> {
    use BarAction as A;
    match (button, target) {
        (MouseButton::Middle, Target::Tab(i) | Target::TabClose(i)) => Some(A::CloseTab(i)),
        (MouseButton::Left, _) => Some(match target {
            Target::Drag if double => A::ToggleMaximize,
            Target::Tab(i) if double => A::RenameTab(i),
            Target::Tab(i) => A::SelectTab(i),
            Target::TabClose(i) => A::CloseTab(i),
            Target::NewTab => A::Perform(Action::NewTab),
            Target::SplitRight if alt => A::Perform(Action::SplitLeft),
            Target::SplitDown if alt => A::Perform(Action::SplitUp),
            Target::SplitRight => A::Perform(Action::SplitRight),
            Target::SplitDown => A::Perform(Action::SplitDown),
            Target::Menu => A::OpenMenu,
            Target::Find => A::Perform(Action::Find),
            Target::Minimize => A::Minimize,
            Target::Maximize => A::ToggleMaximize,
            Target::Close => A::Close,
            Target::Drag => A::DragWindow,
        }),
        _ => None,
    }
}

/// Pointer shape over a window edge zone.
pub(super) fn edge_icon(dir: ResizeDirection) -> CursorIcon {
    match dir {
        ResizeDirection::North => CursorIcon::NResize,
        ResizeDirection::NorthEast => CursorIcon::NeResize,
        ResizeDirection::East => CursorIcon::EResize,
        ResizeDirection::SouthEast => CursorIcon::SeResize,
        ResizeDirection::South => CursorIcon::SResize,
        ResizeDirection::SouthWest => CursorIcon::SwResize,
        ResizeDirection::West => CursorIcon::WResize,
        ResizeDirection::NorthWest => CursorIcon::NwResize,
    }
}

/// The edge-resize direction at `pos`, if the window is resizable from there
/// (not maximized or fullscreen).
pub(super) fn window_edge(
    size: PhysicalSize<u32>,
    pos: (i32, i32),
    border: i32,
    resizable: bool,
) -> Option<ResizeDirection> {
    if !resizable {
        return None;
    }
    let clamp = |v: u32| v.min(i32::MAX as u32) as i32;
    resize_edge(clamp(size.width), clamp(size.height), pos.0, pos.1, border)
}

/// The hover state for a pointer hit: `Drag` (empty bar) draws no hover.
pub(super) fn hover_of(t: Option<Target>) -> Option<Target> {
    t.filter(|&t| t != Target::Drag)
}

/// A tab label to draw with `draw_text`.
#[derive(Debug, PartialEq)]
pub(super) struct Label {
    pub x: i32,
    pub y: i32,
    pub text: String,
    pub color: Rgb,
    pub clip: Rect,
}

/// Everything the bar draws, in order: backgrounds, labels, icons.
#[derive(Debug, Default)]
pub(super) struct BarPaint {
    pub under: Vec<RectInstance>,
    pub labels: Vec<Label>,
    pub icons: Vec<RectInstance>,
}

/// The bar's rects and labels for a window `width` px wide. The active tab has
/// the terminal background; a hovered inactive tab or button gets the hover bg
/// (the Close button its own red); a hovered ✕ highlights only its box. The
/// active tab ends in an `underline` px line in `accent`, drawn after every
/// background.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint(
    bar: &Bar,
    width: i32,
    active: usize,
    hover: Option<Target>,
    maximized: bool,
    stroke: i32,
    underline: i32,
    accent: [f32; 4],
    marks: &[Option<Level>],
) -> BarPaint {
    let fill = |rect: Rect, color: [f32; 4]| RectInstance { rect, color };
    let mut out = BarPaint::default();
    out.under.push(fill(
        Rect {
            x: 0,
            y: 0,
            w: width,
            h: bar.height,
        },
        theme::BAR_BG,
    ));
    for item in &bar.items {
        let hovered = hover == Some(item.target);
        let (bg, icon) = match item.target {
            Target::Tab(i) if i == active => (Some(theme::TAB_ACTIVE), None),
            Target::Tab(_) if hovered => (Some(theme::HOVER), None),
            Target::Tab(_) => (Some(theme::TAB_INACTIVE), None),
            Target::TabClose(_) => (hovered.then_some(theme::HOVER), Some(Icon::Close)),
            Target::NewTab => (hovered.then_some(theme::HOVER), Some(Icon::Plus)),
            Target::SplitRight => (hovered.then_some(theme::HOVER), Some(Icon::SplitRight)),
            Target::SplitDown => (hovered.then_some(theme::HOVER), Some(Icon::SplitDown)),
            Target::Menu => (hovered.then_some(theme::HOVER), Some(Icon::Menu)),
            Target::Find => (hovered.then_some(theme::HOVER), Some(Icon::Find)),
            Target::Minimize => (hovered.then_some(theme::HOVER), Some(Icon::Minimize)),
            Target::Maximize => {
                let icon = if maximized {
                    Icon::Restore
                } else {
                    Icon::Maximize
                };
                (hovered.then_some(theme::HOVER), Some(icon))
            }
            Target::Close => (hovered.then_some(theme::CLOSE_HOVER), Some(Icon::Close)),
            Target::Drag => (None, None),
        };
        if let Some(bg) = bg {
            out.under.push(fill(item.rect, bg));
        }
        if let Some(icon) = icon {
            out.icons.extend(
                icon_rects(icon, item.rect, stroke)
                    .into_iter()
                    .map(|r| fill(r, theme::ICON)),
            );
        }
    }
    if let Some(item) = bar.items.iter().find(|i| i.target == Target::Tab(active)) {
        let r = item.rect;
        out.under.push(fill(
            Rect {
                x: r.x,
                y: r.y + r.h - underline,
                w: r.w,
                h: underline,
            },
            accent,
        ));
    }
    for t in &bar.tabs {
        if let (Some(dot), Some(Some(level))) = (t.dot_rect, marks.get(t.index)) {
            out.icons.extend(disc(dot, level_color(*level)));
        }
    }
    out.labels = bar
        .tabs
        .iter()
        .map(|t| Label {
            x: t.text_rect.x,
            y: t.text_rect.y,
            text: t.label.clone(),
            color: theme::to_rgb(if t.index == active {
                theme::TEXT
            } else {
                theme::TEXT_DIM
            }),
            clip: t.text_rect,
        })
        .collect();
    out
}

/// The active-tab underline thickness: 3 px at scale 1.
pub(super) fn underline_px(scale: f64) -> i32 {
    ((3.0 * scale).round() as i32).max(1)
}

impl Window {
    /// Bar metrics for the current font and scale.
    pub(super) fn bar_metrics(&self) -> BarMetrics {
        BarMetrics::new(&self.ui_metrics, self.scale)
    }

    /// The bar's layout for the current width and tabs. A tab being renamed
    /// is sized for the field's text.
    pub(super) fn bar(&self) -> Bar {
        let mut labels = tab_labels(&self.tabs, |id| {
            self.panes.get(&id).and_then(|p| p.title.as_deref())
        });
        if let Modal::Prompt {
            purpose: PromptPurpose::RenameTab { pane, .. },
            field,
        } = &self.modal
            && let Some(label) = resolve_rename(&self.tabs, *pane).and_then(|i| labels.get_mut(i))
        {
            *label = editing_label(field.text());
        }
        let width = self.size.width.min(i32::MAX as u32) as i32;
        let dots: Vec<bool> = self.tab_marks().iter().map(Option::is_some).collect();
        layout(
            width,
            &self.bar_metrics(),
            &labels,
            self.tabs.active_index(),
            &dots,
        )
    }

    /// The label the bar shows for the tab holding `pane`, sanitised for a notification.
    pub(super) fn tab_title_of(&self, pane: PaneId) -> String {
        let labels = tab_labels(&self.tabs, |id| {
            self.panes.get(&id).and_then(|p| p.title.as_deref())
        });
        let label = self.tabs.tab_of(pane).and_then(|i| labels.get(i));
        crate::term::status::display_text(label.map_or("montty", String::as_str))
    }

    /// The OS window title follows the active tab's label.
    pub(super) fn update_title(&self) {
        let tab = self.tabs.active();
        let title = self
            .panes
            .get(&tab.focused)
            .and_then(|p| p.title.as_deref());
        self.window
            .set_title(&tab_label(tab.title.as_deref(), title));
    }

    /// The window size in px, as i32.
    pub(super) fn size_px(&self) -> (i32, i32) {
        let clamp = |v: u32| v.min(i32::MAX as u32) as i32;
        (clamp(self.size.width), clamp(self.size.height))
    }

    /// Icon strokes and chrome borders: `max(1, round(scale))` px.
    pub(super) fn chrome_stroke(&self) -> i32 {
        (self.scale.round() as i32).max(1)
    }

    /// Draw the bar. The hover is recomputed from the last pointer position
    /// first, so it is never stale after tabs or titles changed.
    pub(super) fn draw_bar(&mut self) {
        let bar = self.bar();
        self.hover = self.pointer_hover(&bar);
        let p = paint(
            &bar,
            self.size_px().0,
            self.tabs.active_index(),
            self.hover,
            self.maximized,
            self.chrome_stroke(),
            underline_px(self.scale),
            ChromeColors::of(self.store.global()).border,
            &self.tab_marks(),
        );
        self.draw_paint(&p);
    }

    /// Draw `p`'s layers in order: backgrounds, labels, then the rects on top.
    pub(super) fn draw_paint(&mut self, p: &BarPaint) {
        self.renderer.draw_rects(&p.under);
        if let Some(fonts) = self.fonts.by_key(self.ui_font) {
            for l in &p.labels {
                let m = &self.ui_metrics;
                self.renderer
                    .draw_text(l.x, l.y, &l.text, l.color, l.clip, fonts, m);
            }
        }
        self.renderer.draw_rects(&p.icons);
    }

    /// Edge-resize zone width: `edge_px(scale)` (4 px at scale 1), the
    /// same as the pane overlay's inset.
    fn edge_border(&self) -> i32 {
        edge_px(self.scale)
    }

    /// Whether the window can be resized from its edges now.
    fn edges_resizable(&self, maximized: bool) -> bool {
        !maximized && self.window.fullscreen().is_none()
    }

    /// The bar item the pointer is over (`None` in the edge zone, below the bar,
    /// outside the window or while a modal is open), as drawn.
    fn pointer_hover(&self, bar: &Bar) -> Option<Target> {
        let pos = self.mouse.pos();
        if !self.mouse.inside() || pos.1 >= bar.height || self.modal_open() {
            return None;
        }
        let resizable = self.edges_resizable(self.maximized);
        if window_edge(self.size, pos, self.edge_border(), resizable).is_some() {
            return None;
        }
        hover_of(hit(bar, pos.0, pos.1))
    }

    fn set_hover(&mut self, hover: Option<Target>) {
        if hover != self.hover {
            self.hover = hover;
            self.schedule();
        }
    }

    /// Pointer moved (no divider drag): updates the hover and, over the edge
    /// zone or the bar, the cursor shape. True when the pointer is there, so
    /// the panes don't handle it.
    pub(super) fn bar_pointer_moved(&mut self) -> bool {
        let pos = self.mouse.pos();
        let resizable = self.edges_resizable(self.maximized);
        if let Some(dir) = window_edge(self.size, pos, self.edge_border(), resizable) {
            self.set_hover(None);
            self.set_cursor_icon(edge_icon(dir));
            return true;
        }
        if pos.1 >= self.bar_metrics().height {
            self.set_hover(None);
            return false;
        }
        let bar = self.bar();
        self.set_hover(hover_of(hit(&bar, pos.0, pos.1)));
        self.set_cursor_icon(CursorIcon::Default);
        true
    }

    /// The pointer left the window: nothing is hovered.
    pub(super) fn bar_pointer_left(&mut self) {
        self.set_hover(None);
    }

    /// A button press: the edge zone starts a window resize, the bar runs its
    /// target's action. True when consumed (the panes don't see it).
    pub(super) fn bar_press(&mut self, button: MouseButton) -> bool {
        let pos = self.mouse.pos();
        if button == MouseButton::Left {
            let resizable = self.edges_resizable(self.maximized);
            if let Some(dir) = window_edge(self.size, pos, self.edge_border(), resizable) {
                self.mouse.clicks.clear();
                if let Err(e) = self.window.drag_resize_window(dir) {
                    eprintln!("montty: drag_resize_window: {e}");
                }
                return true;
            }
        }
        let Some(target) = hit(&self.bar(), pos.0, pos.1) else {
            self.mouse.clicks.clear();
            return false;
        };
        let double = match button {
            MouseButton::Left => {
                let generation = self.tabs.generation();
                self.mouse.clicks.press(target, generation, Instant::now())
            }
            _ => {
                self.mouse.clicks.clear();
                false
            }
        };
        if let Some(action) = press_action(target, button, double, self.modifiers.alt_key()) {
            self.run_bar_action(action);
        }
        true
    }

    fn run_bar_action(&mut self, action: BarAction) {
        match action {
            BarAction::SelectTab(i) if i != self.tabs.active_index() => {
                self.switch_tab(|t| {
                    t.select(i);
                });
            }
            BarAction::SelectTab(_) => {}
            BarAction::CloseTab(i) => self.request_close_tab(i),
            BarAction::RenameTab(i) => self.open_rename(i),
            BarAction::Perform(a) => self.perform(a),
            BarAction::OpenMenu => self.open_menu(),
            BarAction::Minimize => self.window.set_minimized(true),
            BarAction::ToggleMaximize => {
                // Toggles what the button shows (the cached state, no X11
                // round trip); the cache follows our request at once and
                // `on_resize` corrects it if the WM did otherwise.
                let maximize = !self.maximized;
                self.window.set_maximized(maximize);
                self.maximized = maximize;
                self.schedule();
            }
            // Same as the window manager's close request.
            BarAction::Close => self.request_close(CloseTarget::Window),
            BarAction::DragWindow => {
                if let Err(e) = self.window.drag_window() {
                    eprintln!("montty: drag_window: {e}");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::{
        BAR_BG, CLOSE_HOVER, HOVER, ICON, TAB_ACTIVE, TAB_INACTIVE, TEXT, TEXT_DIM, to_rgb,
    };
    use crate::window::tabs::Tab;
    use std::collections::HashMap;
    use std::time::Duration;

    const ACCENT_T: [f32; 4] = [0.29, 0.56, 0.85, 1.0];

    fn bm() -> BarMetrics {
        BarMetrics {
            height: 28,
            pad: 6,
            cell_w: 8,
            cell_h: 16,
        }
    }

    fn rect_of(bar: &Bar, t: Target) -> Rect {
        bar.items.iter().find(|i| i.target == t).unwrap().rect
    }

    fn color_at(p: &BarPaint, r: Rect) -> Vec<[f32; 4]> {
        p.under
            .iter()
            .filter(|i| i.rect == r)
            .map(|i| i.color)
            .collect()
    }

    #[test]
    fn pane_area_is_below_bar() {
        let a = pane_area(PhysicalSize::new(800, 600), 28);
        assert_eq!(
            a,
            Rect {
                x: 0,
                y: 28,
                w: 800,
                h: 572
            }
        );
        // Shorter than the bar: empty, never negative.
        let a = pane_area(PhysicalSize::new(100, 10), 28);
        assert_eq!((a.y, a.w, a.h), (28, 100, 0));
        let a = pane_area(PhysicalSize::new(u32::MAX, u32::MAX), 28);
        assert_eq!((a.w, a.h), (i32::MAX, i32::MAX - 28));
    }

    #[test]
    fn labels_follow_rename_then_focused_title() {
        let mut tabs = Tabs::new(Tab::new(1, (32, 32)));
        let mut t = Tab::new(2, (32, 32));
        t.title = Some("work".into());
        tabs.push(t);
        tabs.push(Tab::new(3, (32, 32)));
        let titles: HashMap<PaneId, String> =
            HashMap::from([(1, "vim".into()), (2, "bash".into())]);
        let labels = tab_labels(&tabs, |id| titles.get(&id).map(String::as_str));
        assert_eq!(labels, ["vim", "work", "montty"]);
    }

    #[test]
    fn double_click_same_target_within_400ms() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let mut c = ClickTracker::default();
        assert!(!c.press(Target::Drag, 0, t0));
        assert!(c.press(Target::Drag, 0, ms(399)));
        // A third press starts a new pair.
        assert!(!c.press(Target::Drag, 0, ms(450)));
        // Too slow.
        assert!(!c.press(Target::Drag, 0, ms(851)));
        // Another target breaks the pair.
        assert!(!c.press(Target::Tab(0), 0, ms(900)));
        assert!(!c.press(Target::Drag, 0, ms(950)));
        assert!(c.press(Target::Drag, 0, ms(1000)));
        // A press elsewhere breaks it too.
        assert!(!c.press(Target::Tab(1), 0, ms(2000)));
        c.clear();
        assert!(!c.press(Target::Tab(1), 0, ms(2100)));
        // A tab closed, opened or moved in between: tab 1 is another tab now.
        assert!(!c.press(Target::Tab(1), 0, ms(3000)));
        assert!(!c.press(Target::Tab(1), 1, ms(3100)));
        assert!(c.press(Target::Tab(1), 1, ms(3200)));
    }

    #[test]
    fn left_press_maps_targets_to_actions() {
        let l = |t| press_action(t, MouseButton::Left, false, false);
        assert_eq!(l(Target::Tab(2)), Some(BarAction::SelectTab(2)));
        assert_eq!(l(Target::TabClose(1)), Some(BarAction::CloseTab(1)));
        assert_eq!(l(Target::NewTab), Some(BarAction::Perform(Action::NewTab)));
        assert_eq!(
            l(Target::SplitRight),
            Some(BarAction::Perform(Action::SplitRight))
        );
        assert_eq!(
            l(Target::SplitDown),
            Some(BarAction::Perform(Action::SplitDown))
        );
        assert_eq!(l(Target::Menu), Some(BarAction::OpenMenu));
        assert_eq!(l(Target::Find), Some(BarAction::Perform(Action::Find)));
        assert_eq!(l(Target::Minimize), Some(BarAction::Minimize));
        assert_eq!(l(Target::Maximize), Some(BarAction::ToggleMaximize));
        assert_eq!(l(Target::Close), Some(BarAction::Close));
        assert_eq!(l(Target::Drag), Some(BarAction::DragWindow));
    }

    #[test]
    fn alt_click_splits_left_or_up() {
        let a = |t| press_action(t, MouseButton::Left, false, true);
        assert_eq!(
            a(Target::SplitRight),
            Some(BarAction::Perform(Action::SplitLeft))
        );
        assert_eq!(
            a(Target::SplitDown),
            Some(BarAction::Perform(Action::SplitUp))
        );
        assert_eq!(a(Target::NewTab), Some(BarAction::Perform(Action::NewTab)));
    }

    #[test]
    fn double_click_maximizes_or_renames() {
        let d = |t| press_action(t, MouseButton::Left, true, false);
        assert_eq!(d(Target::Drag), Some(BarAction::ToggleMaximize));
        assert_eq!(d(Target::Tab(3)), Some(BarAction::RenameTab(3)));
        // Elsewhere a second press is just another press.
        assert_eq!(d(Target::TabClose(0)), Some(BarAction::CloseTab(0)));
        assert_eq!(d(Target::NewTab), Some(BarAction::Perform(Action::NewTab)));
    }

    #[test]
    fn middle_click_closes_tab_only() {
        let m = |t| press_action(t, MouseButton::Middle, false, false);
        assert_eq!(m(Target::Tab(1)), Some(BarAction::CloseTab(1)));
        assert_eq!(m(Target::TabClose(1)), Some(BarAction::CloseTab(1)));
        assert_eq!(m(Target::Drag), None);
        assert_eq!(m(Target::Close), None);
        assert_eq!(
            press_action(Target::Tab(0), MouseButton::Right, false, false),
            None
        );
    }

    #[test]
    fn edge_icons_match_direction() {
        use ResizeDirection as D;
        let pairs = [
            (D::North, CursorIcon::NResize),
            (D::NorthEast, CursorIcon::NeResize),
            (D::East, CursorIcon::EResize),
            (D::SouthEast, CursorIcon::SeResize),
            (D::South, CursorIcon::SResize),
            (D::SouthWest, CursorIcon::SwResize),
            (D::West, CursorIcon::WResize),
            (D::NorthWest, CursorIcon::NwResize),
        ];
        for (d, icon) in pairs {
            assert_eq!(edge_icon(d), icon, "{d:?}");
        }
    }

    #[test]
    fn edge_zone_only_when_resizable() {
        let size = PhysicalSize::new(800, 600);
        assert_eq!(
            window_edge(size, (0, 0), 4, true),
            Some(ResizeDirection::NorthWest)
        );
        assert_eq!(
            window_edge(size, (400, 599), 4, true),
            Some(ResizeDirection::South)
        );
        assert_eq!(window_edge(size, (400, 300), 4, true), None);
        // Maximized or fullscreen: no edge zone.
        assert_eq!(window_edge(size, (0, 0), 4, false), None);
    }

    #[test]
    fn drag_area_draws_no_hover() {
        assert_eq!(hover_of(Some(Target::Drag)), None);
        assert_eq!(hover_of(None), None);
        assert_eq!(hover_of(Some(Target::Tab(1))), Some(Target::Tab(1)));
    }

    fn two_tabs() -> Bar {
        layout(800, &bm(), &["bash".to_string(), "vim".to_string()], 0, &[])
    }

    #[test]
    fn paint_backgrounds_and_labels() {
        let bar = two_tabs();
        let p = paint(&bar, 800, 0, None, false, 1, 3, ACCENT_T, &[]);
        // The bar background comes first and spans the window.
        assert_eq!(
            p.under[0],
            RectInstance {
                rect: Rect {
                    x: 0,
                    y: 0,
                    w: 800,
                    h: 28
                },
                color: BAR_BG
            }
        );
        assert_eq!(color_at(&p, rect_of(&bar, Target::Tab(0))), [TAB_ACTIVE]);
        assert_eq!(color_at(&p, rect_of(&bar, Target::Tab(1))), [TAB_INACTIVE]);
        // Buttons draw only their icons when not hovered.
        assert!(color_at(&p, rect_of(&bar, Target::Close)).is_empty());
        assert_eq!(p.labels.len(), 2);
        let l = &p.labels[0];
        assert_eq!((l.text.as_str(), l.color), ("bash", to_rgb(TEXT)));
        assert_eq!((l.x, l.y, l.clip), (6, 6, bar.tabs[0].text_rect));
        assert_eq!(p.labels[1].color, to_rgb(TEXT_DIM));
        assert!(p.icons.iter().all(|i| i.color == ICON));
    }

    #[test]
    fn paint_icons_for_every_button_and_tab_close() {
        let bar = two_tabs();
        let p = paint(&bar, 800, 0, None, false, 1, 3, ACCENT_T, &[]);
        let mut expected = 0;
        for (t, icon) in [
            (Target::TabClose(0), Icon::Close),
            (Target::TabClose(1), Icon::Close),
            (Target::NewTab, Icon::Plus),
            (Target::SplitRight, Icon::SplitRight),
            (Target::SplitDown, Icon::SplitDown),
            (Target::Menu, Icon::Menu),
            (Target::Find, Icon::Find),
            (Target::Minimize, Icon::Minimize),
            (Target::Maximize, Icon::Maximize),
            (Target::Close, Icon::Close),
        ] {
            let rects = icon_rects(icon, rect_of(&bar, t), 1);
            for r in &rects {
                assert!(p.icons.iter().any(|i| i.rect == *r), "{t:?} {r:?}");
            }
            expected += rects.len();
        }
        assert_eq!(p.icons.len(), expected);
        // Maximized: the Maximize button shows Restore.
        let p = paint(&bar, 800, 0, None, true, 1, 3, ACCENT_T, &[]);
        for r in icon_rects(Icon::Restore, rect_of(&bar, Target::Maximize), 1) {
            assert!(p.icons.iter().any(|i| i.rect == r));
        }
    }

    #[test]
    fn paint_hover() {
        let bar = two_tabs();
        let hovered = |t| paint(&bar, 800, 0, Some(t), false, 1, 3, ACCENT_T, &[]);
        let p = hovered(Target::Tab(1));
        assert_eq!(color_at(&p, rect_of(&bar, Target::Tab(1))), [HOVER]);
        // The active tab keeps the terminal background.
        let p = hovered(Target::Tab(0));
        assert_eq!(color_at(&p, rect_of(&bar, Target::Tab(0))), [TAB_ACTIVE]);
        // A hovered ✕ highlights its box, not the tab.
        let p = hovered(Target::TabClose(1));
        assert_eq!(color_at(&p, rect_of(&bar, Target::Tab(1))), [TAB_INACTIVE]);
        assert_eq!(color_at(&p, rect_of(&bar, Target::TabClose(1))), [HOVER]);
        for t in [Target::NewTab, Target::Minimize, Target::Menu] {
            assert_eq!(color_at(&hovered(t), rect_of(&bar, t)), [HOVER], "{t:?}");
        }
        let p = hovered(Target::Close);
        assert_eq!(color_at(&p, rect_of(&bar, Target::Close)), [CLOSE_HOVER]);
        // Only the hovered item changes.
        assert_eq!(
            p.under.len(),
            paint(&bar, 800, 0, None, false, 1, 3, ACCENT_T, &[])
                .under
                .len()
                + 1
        );
    }

    #[test]
    fn underline_px_scales() {
        assert_eq!(underline_px(1.0), 3);
        assert_eq!(underline_px(2.0), 6);
        assert_eq!(underline_px(1.5), 5);
        assert_eq!(underline_px(0.2), 1);
    }

    #[test]
    fn active_tab_has_underline_last_in_under() {
        for active in [0, 1] {
            let bar = two_tabs();
            let p = paint(
                &bar,
                800,
                active,
                Some(Target::TabClose(active)),
                false,
                1,
                3,
                ACCENT_T,
                &[],
            );
            let t = rect_of(&bar, Target::Tab(active));
            let line = RectInstance {
                rect: Rect {
                    x: t.x,
                    y: t.y + t.h - 3,
                    w: t.w,
                    h: 3,
                },
                color: ACCENT_T,
            };
            // Drawn after every background, including the ✕ hover fill.
            assert_eq!(p.under.last(), Some(&line));
            assert_eq!(p.under.iter().filter(|i| i.color == ACCENT_T).count(), 1);
            // The tab itself looks as before.
            assert_eq!(color_at(&p, t), [TAB_ACTIVE]);
        }
    }

    #[test]
    fn single_tab_is_underlined() {
        let bar = layout(800, &bm(), &["bash".to_string()], 0, &[]);
        let p = paint(&bar, 800, 0, None, false, 1, 3, ACCENT_T, &[]);
        assert_eq!(p.under.last().unwrap().color, ACCENT_T);
    }

    #[test]
    fn marked_tab_paints_a_coloured_disc() {
        let ls = ["bash".to_string(), "vim".to_string()];
        let bar = layout(800, &bm(), &ls, 0, &[false, true]);
        let marks = [None, Some(Level::Blocked)];
        let p = paint(&bar, 800, 0, None, false, 1, 3, ACCENT_T, &marks);
        let dot = bar.tabs[1].dot_rect.unwrap();
        let discs: Vec<_> = p
            .icons
            .iter()
            .filter(|r| r.color == theme::MARK_BLOCKED)
            .collect();
        assert_eq!(discs.len() as i32, dot.h);
        assert!(discs.iter().all(|r| dot.contains(r.rect.x, r.rect.y)));
        let none = paint(&bar, 800, 0, None, false, 1, 3, ACCENT_T, &[]);
        assert!(none.icons.iter().all(|r| r.color != theme::MARK_BLOCKED));
    }
}
