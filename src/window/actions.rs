//! Key-binding actions: tiling, tabs, scrolling, zoom, fullscreen, clipboard and find.

use std::path::PathBuf;
use std::time::Instant;

use alacritty_terminal::event::WindowSize;
use alacritty_terminal::grid::Scroll;
use winit::window::Fullscreen;

use super::tabs::{Tab, Tabs};
use super::{
    PaneStyle, Window, fallback_text, grid_size, next_zoom, pane_min_extent, pane_options,
    spawn_pane_process, tab_min_extent, window_size,
};
use crate::clipboard::ClipKind;
use crate::font::Metrics;
use crate::input::bindings::Action;
use crate::layout::{Direction, Orientation, PaneId, Rect, RemoveResult, SplitTree};
use crate::term::pane::{Pane, PaneOptions, next_pane_id};
use crate::ui::confirm::CloseTarget;
use crate::windows::AppRequest;

impl Window {
    pub fn perform(&mut self, a: Action) {
        match a {
            Action::SplitRight => self.split(Orientation::Horizontal, false),
            Action::SplitDown => self.split(Orientation::Vertical, false),
            Action::SplitLeft => self.split(Orientation::Horizontal, true),
            Action::SplitUp => self.split(Orientation::Vertical, true),
            Action::SplitAuto => {
                let area = self.area();
                let tab = self.tabs.active_mut();
                let was_maximized = tab.tree.maximized().is_some();
                let o = split_auto_orientation(&mut tab.tree, tab.focused, area);
                if was_maximized {
                    // Keep the restored layout even if the split is refused.
                    self.relayout();
                }
                self.split(o, false);
            }
            Action::FocusUp => self.focus_dir(Direction::Up),
            Action::FocusDown => self.focus_dir(Direction::Down),
            Action::FocusLeft => self.focus_dir(Direction::Left),
            Action::FocusRight => self.focus_dir(Direction::Right),
            Action::FocusPane(n) => self.focus_with(|panes, _| nth_pane(panes, n)),
            Action::NextPane => self.focus_with(|panes, cur| cycle_pane(panes, cur, true)),
            Action::PrevPane => self.focus_with(|panes, cur| cycle_pane(panes, cur, false)),
            Action::ResizeUp => self.resize_focused(Direction::Up),
            Action::ResizeDown => self.resize_focused(Direction::Down),
            Action::ResizeLeft => self.resize_focused(Direction::Left),
            Action::ResizeRight => self.resize_focused(Direction::Right),
            Action::MaximizePane => {
                self.mouse.cancel_drag();
                self.size_labels.clear();
                let tab = self.tabs.active_mut();
                tab.tree.toggle_maximize(tab.focused);
                self.relayout();
                self.layout_changed();
            }
            Action::ClosePane => self.request_close(CloseTarget::Pane(self.tabs.active().focused)),
            Action::ScrollLineUp => self.scroll_focused(Scroll::Delta(1)),
            Action::ScrollLineDown => self.scroll_focused(Scroll::Delta(-1)),
            Action::ScrollPageUp => self.scroll_focused(Scroll::PageUp),
            Action::ScrollPageDown => self.scroll_focused(Scroll::PageDown),
            Action::ZoomIn => self.zoom(Some(1.0)),
            Action::ZoomOut => self.zoom(Some(-1.0)),
            Action::ZoomReset => self.zoom(None),
            Action::Fullscreen => {
                let next = match self.window.fullscreen() {
                    Some(_) => None,
                    None => Some(Fullscreen::Borderless(None)),
                };
                self.window.set_fullscreen(next);
            }
            Action::ShowSizes => {
                if self.size_labels.showing() {
                    self.size_labels.clear();
                } else {
                    let ids: Vec<PaneId> = self
                        .tabs
                        .active()
                        .tree
                        .layout(self.area())
                        .iter()
                        .map(|&(id, _)| id)
                        .collect();
                    self.size_labels.show_all(&ids, Instant::now());
                }
                self.schedule();
            }
            Action::NewTab => self.new_tab(),
            Action::CloseTab => self.request_close_tab(self.tabs.active_index()),
            Action::NextTab => self.switch_tab(Tabs::next),
            Action::PrevTab => self.switch_tab(Tabs::prev),
            Action::MoveTabLeft => {
                self.switch_tab(|t| {
                    t.move_active(-1);
                });
                self.layout_changed();
            }
            Action::MoveTabRight => {
                self.switch_tab(|t| {
                    t.move_active(1);
                });
                self.layout_changed();
            }
            Action::GoToTab(n) => self.switch_tab(|t| {
                t.go_to(n);
            }),
            Action::Copy => self.copy(),
            Action::Paste => self.paste_into_focused(ClipKind::Clipboard),
            Action::PastePrimary => self.paste_into_focused(ClipKind::Primary),
            Action::SelectAll => self.select_all(),
            Action::Find => self.open_find(),
            Action::FindNext => self.find_next(),
            Action::FindPrev => self.find_prev(),
            Action::NewWindow => self.requests.push(AppRequest::NewWindow {
                profile: self.profile_arg.clone(),
            }),
            Action::SaveSession => self.open_save_prompt(),
            Action::OpenSession => self.open_session_menu(),
        }
    }

    /// Split the focused pane; the new pane runs under its profile (unzoomed) and takes
    /// focus. The new pane goes left or above when `before`. A refused split or a failed
    /// spawn leaves the layout as it was.
    fn split(&mut self, o: Orientation, before: bool) {
        let id = next_pane_id();
        let style = PaneStyle {
            stem: self.pane_stem(self.tabs.active().focused).to_string(),
            zoom_pt: 0.0,
        };
        let cwd = self
            .panes
            .get(&self.tabs.active().focused)
            .and_then(Pane::cwd);
        // Recorded first: its font decides whether there is room.
        self.pane_profiles.insert(id, style);
        if !self.split_into(o, before, id, cwd) {
            self.pane_profiles.remove(&id);
        }
        self.sync_images(false);
    }

    /// `split` once the new pane `id`'s style is recorded; false when refused or failed.
    fn split_into(
        &mut self,
        o: Orientation,
        before: bool,
        id: PaneId,
        cwd: Option<PathBuf>,
    ) -> bool {
        let area = self.area();
        let font = self.pane_font(id);
        let tab = self.tabs.active();
        // Room for the new pane's own cell too: the focused pane may be zoomed out.
        let new_min = pane_min_extent(&font.metrics, font.padding);
        if !can_split_with(&tab.tree, tab.focused, o, area, new_min) {
            return false;
        }
        let mut tree = tab.tree.clone();
        let split = if before {
            tree.split_before(tab.focused, o, id)
        } else {
            tree.split(tab.focused, o, id)
        };
        if !split {
            return false;
        }
        let Some(&(_, rect)) = tree.layout(area).iter().find(|(p, _)| *p == id) else {
            return false;
        };
        let Some((pane, size)) = self.spawn_pane(id, rect, cwd, None) else {
            return false;
        };
        self.tabs.active_mut().tree = tree;
        self.mouse.cancel_drag();
        self.panes.insert(id, pane);
        self.sizes.insert(id, size);
        self.refresh_min_extents();
        self.set_focus(id);
        self.relayout();
        self.layout_changed();
        true
    }

    /// Swap panes `pane` and `with` (places and sizes): the programs keep running, and
    /// `pane`, the dragged one, takes the focus.
    pub(super) fn swap_panes(&mut self, pane: PaneId, with: PaneId) {
        // A drag outlived by a maximize: `with` may be a hidden slot.
        if self.tabs.active().tree.maximized().is_some()
            || !self.tabs.active_mut().tree.swap(pane, with)
        {
            return;
        }
        self.mouse.cancel_drag();
        self.set_focus(pane);
        self.relayout();
        self.layout_changed();
    }

    /// Start pane `id` under its recorded profile (its command or shell), sized for `rect`;
    /// a program that can't start falls back to `$SHELL`, then `/bin/sh`, with a toast, and
    /// with none left a toast says so. `-e` applies only to the first pane. The pane starts
    /// in `cwd`, or in montty's own directory when `None`. A session leaf's `command` runs
    /// instead of the profile's program (and is recorded on the pane, for saving).
    pub(super) fn spawn_pane(
        &mut self,
        id: PaneId,
        rect: Rect,
        cwd: Option<PathBuf>,
        command: Option<Vec<String>>,
    ) -> Option<(Pane, WindowSize)> {
        let font = self.pane_font(id);
        let (cols, lines) = grid_size(rect, &font.metrics, font.padding);
        let size = window_size(cols, lines, &font.metrics);
        let shell = std::env::var("SHELL").ok();
        let profile = &self.pane_profile(id).profile;
        let opts = PaneOptions {
            kitty_keyboard: self.store.global().kitty_keyboard,
            ..pane_options(
                profile,
                command.clone(),
                shell.as_deref(),
                self.x11_window_id,
                cwd,
            )
        };
        let result = spawn_pane_process(id, &opts, size, &self.sink, true, shell.as_deref());
        match result {
            Ok((mut p, fallback)) => {
                let explicit = command.is_some();
                p.set_explicit_command(command);
                if let Some(f) = fallback {
                    let stem = (!explicit).then(|| self.resolved_stem(id));
                    let (body, tail) = fallback_text(stem, &f);
                    self.spawn_toast(&body, &tail);
                }
                Some((p, size))
            }
            Err(e) => {
                self.spawn_toast(&format!("can't start a shell: {e}"), "");
                None
            }
        }
    }

    /// Append a tab with one pane under the window's default profile and switch to it.
    /// A failed spawn leaves the tabs as they were.
    pub(super) fn new_tab(&mut self) {
        self.new_tab_with(None, None);
    }

    /// A new tab under the window's default profile, started in `cwd` and running `command`
    /// (None: the profile's shell or command).
    pub(super) fn new_tab_with(&mut self, cwd: Option<PathBuf>, command: Option<Vec<String>>) {
        let id = next_pane_id();
        self.pane_profiles.insert(id, self.default_style());
        let Some((pane, size)) = self.spawn_pane(id, self.area(), cwd, command) else {
            self.pane_profiles.remove(&id);
            return;
        };
        let font = self.pane_font(id);
        self.panes.insert(id, pane);
        self.sizes.insert(id, size);
        self.tabs
            .push(Tab::new(id, pane_min_extent(&font.metrics, font.padding)));
        self.mouse.cancel_drag();
        self.sync_images(false);
        self.show_active();
        self.layout_changed();
    }

    /// Close tab `i`, asking first when any of its panes runs a process.
    pub(super) fn request_close_tab(&mut self, i: usize) {
        if let Some(tab) = self.tabs.get(i) {
            self.request_close(CloseTarget::Tab(tab.focused));
        }
    }

    /// Close `target` without asking: the pane (the window with its last
    /// one), the tab holding the pane, or the window.
    pub(super) fn close_target(&mut self, target: CloseTarget) {
        match target {
            CloseTarget::Pane(id) => {
                if !self.close_pane(id) {
                    self.close_requested = true;
                }
            }
            CloseTarget::Tab(id) => {
                if let Some(i) = self.tabs.tab_of(id) {
                    self.close_tab(i);
                }
            }
            CloseTarget::Window => self.close_requested = true,
        }
    }

    /// Close tab `i` and every pane in it (their PTYs shut down); closing the
    /// last tab closes the window.
    pub(super) fn close_tab(&mut self, i: usize) {
        let Some(tab) = self.tabs.remove(i) else {
            return;
        };
        for id in tab.tree.panes() {
            self.attention.remove(id);
            self.panes.remove(&id);
            self.sizes.remove(&id);
            self.pane_profiles.remove(&id);
        }
        self.evict_fonts();
        if self.tabs.is_empty() {
            self.close_requested = true;
            return;
        }
        self.sync_images(false);
        self.dismiss_stale_modal();
        self.mouse.cancel_drag();
        self.show_active();
        self.layout_changed();
    }

    /// Change the active tab (or its place) with `f`, then show the active tab.
    pub(super) fn switch_tab(&mut self, f: impl FnOnce(&mut Tabs)) {
        f(&mut self.tabs);
        self.mouse.cancel_drag();
        self.show_active();
    }

    /// A focus change first restores a maximized layout.
    fn restore_layout(&mut self) {
        if restore_maximized(&mut self.tabs.active_mut().tree) {
            self.relayout();
        }
    }

    fn focus_dir(&mut self, dir: Direction) {
        self.restore_layout();
        let tab = self.tabs.active();
        if let Some(n) = tab.tree.neighbor(tab.focused, dir, self.area()) {
            self.set_focus(n);
        }
    }

    fn focus_with(&mut self, pick: impl FnOnce(&[PaneId], PaneId) -> Option<PaneId>) {
        self.restore_layout();
        let tab = self.tabs.active();
        if let Some(n) = pick(&tab.tree.panes(), tab.focused) {
            self.set_focus(n);
        }
    }

    fn resize_focused(&mut self, dir: Direction) {
        let font = self.pane_font(self.tabs.active().focused);
        let step = resize_step(dir, &font.metrics);
        let area = self.area();
        let tab = self.tabs.active_mut();
        if tab.tree.resize(tab.focused, dir, step, area) {
            let ids = self.relayout();
            self.label_resized(ids);
            self.layout_changed();
        }
    }

    /// Zoom the focused pane only (as Tilix does): `step` points, or back to the
    /// profile's size (`None`). Its tab's panes are laid out again.
    fn zoom(&mut self, step: Option<f32>) {
        let id = self.tabs.active().focused;
        let base = self.pane_profile(id).profile.font_size;
        let Some(style) = self.pane_profiles.get_mut(&id) else {
            return;
        };
        let zoom = next_zoom(base, style.zoom_pt, step);
        if zoom == style.zoom_pt {
            return;
        }
        style.zoom_pt = zoom;
        self.refresh_min_extents();
        self.relayout();
        self.evict_fonts();
    }
}

/// Whether `tree` can split `target` along `o` in `area` when the new pane needs at least
/// `new_min`: both halves must hold the larger of the tree's minimum and it.
pub(super) fn can_split_with(
    tree: &SplitTree,
    target: PaneId,
    o: Orientation,
    area: Rect,
    new_min: (i32, i32),
) -> bool {
    let mut t = tree.clone();
    let (w, h) = tab_min_extent([tree.min_extent(), new_min].into_iter());
    t.set_min_extent(w, h);
    t.can_split(target, o, area)
}

/// `SplitAuto`: side by side when the pane is wider than tall, else stacked.
pub(super) fn auto_orientation(r: Rect) -> Orientation {
    if r.w > r.h {
        Orientation::Horizontal
    } else {
        Orientation::Vertical
    }
}

/// Leave a maximized layout; true when the tree was maximized.
pub(super) fn restore_maximized(tree: &mut SplitTree) -> bool {
    match tree.maximized() {
        Some(m) => {
            tree.toggle_maximize(m);
            true
        }
        None => false,
    }
}

/// `SplitAuto` for `pane`: like the focus actions, first restore a maximized layout,
/// so the pane's real rect (not the whole window) picks the orientation.
pub(super) fn split_auto_orientation(
    tree: &mut SplitTree,
    pane: PaneId,
    area: Rect,
) -> Orientation {
    restore_maximized(tree);
    let layout = tree.layout(area);
    let rect = layout
        .iter()
        .find(|(id, _)| *id == pane)
        .map_or(area, |&(_, r)| r);
    auto_orientation(rect)
}

/// `FocusPane(n)`: the n-th pane (1-based) in tree order.
pub(super) fn nth_pane(panes: &[PaneId], n: u8) -> Option<PaneId> {
    panes.get(usize::from(n).checked_sub(1)?).copied()
}

/// `NextPane`/`PrevPane`: the pane after/before `current` in tree order, wrapping.
pub(super) fn cycle_pane(panes: &[PaneId], current: PaneId, forward: bool) -> Option<PaneId> {
    let i = panes.iter().position(|&p| p == current)?;
    let n = panes.len();
    let next = if forward {
        (i + 1) % n
    } else {
        (i + n - 1) % n
    };
    Some(panes[next])
}

/// After removing a pane, the first pane (tree order) of the sibling subtree that
/// took its place: the first pane in `after` whose rect grew, else the first pane.
pub(super) fn focus_after_remove(
    before: &[(PaneId, Rect)],
    after: &[(PaneId, Rect)],
) -> Option<PaneId> {
    // A subtree's panes are contiguous in tree order and only they change rect.
    let changed = |&&(id, r): &&(PaneId, Rect)| !before.contains(&(id, r));
    after
        .iter()
        .find(changed)
        .or(after.first())
        .map(|&(id, _)| id)
}

/// Remove `pane` from whichever tab holds it; focus within that tab follows
/// `focus_after_remove` (a focused maximized pane first restores the layout).
/// A tab left empty is removed, which may leave `tabs` empty. False when no tab
/// holds `pane`.
pub(super) fn remove_pane(tabs: &mut Tabs, pane: PaneId, area: Rect) -> bool {
    let Some(i) = tabs.tab_of(pane) else {
        return false;
    };
    let Some(tab) = tabs.iter_mut().nth(i) else {
        return false;
    };
    let was_focused = pane == tab.focused;
    if was_focused && tab.tree.maximized() == Some(pane) {
        tab.tree.toggle_maximize(pane);
    }
    let before = tab.tree.layout(area);
    match tab.tree.remove(pane) {
        RemoveResult::NotFound => return false,
        RemoveResult::Empty => {
            tabs.remove(i);
        }
        RemoveResult::Removed => {
            if was_focused && let Some(next) = focus_after_remove(&before, &tab.tree.layout(area)) {
                tab.focused = next;
            }
        }
    }
    true
}

/// One cell along the resize axis.
pub(super) fn resize_step(dir: Direction, m: &Metrics) -> i32 {
    let px = match dir {
        Direction::Left | Direction::Right => m.cell_width,
        Direction::Up | Direction::Down => m.cell_height,
    };
    px.min(i32::MAX as u32) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect {
        x: 0,
        y: 0,
        w: 401,
        h: 201,
    };

    fn rect(w: i32, h: i32) -> Rect {
        Rect { x: 0, y: 0, w, h }
    }

    #[test]
    fn auto_orientation_follows_aspect() {
        assert_eq!(auto_orientation(rect(200, 100)), Orientation::Horizontal);
        assert_eq!(auto_orientation(rect(100, 200)), Orientation::Vertical);
        // Square: not wider than tall, so stacked.
        assert_eq!(auto_orientation(rect(100, 100)), Orientation::Vertical);
    }

    #[test]
    fn split_auto_restores_maximize_and_uses_real_rect() {
        // [1 | 2] in 401x201: pane 1 is 200x201, so stacked; maximized it would
        // fill the 401x201 window and wrongly pick side by side.
        let mut t = SplitTree::new(1);
        t.split(1, Orientation::Horizontal, 2);
        t.toggle_maximize(1);
        assert_eq!(
            split_auto_orientation(&mut t, 1, AREA),
            Orientation::Vertical
        );
        assert_eq!(t.maximized(), None);
    }

    #[test]
    fn nth_pane_is_one_based_and_bounded() {
        let panes = [7, 3, 9];
        assert_eq!(nth_pane(&panes, 1), Some(7));
        assert_eq!(nth_pane(&panes, 3), Some(9));
        assert_eq!(nth_pane(&panes, 4), None);
        assert_eq!(nth_pane(&panes, 0), None);
        assert_eq!(nth_pane(&panes, 10), None);
    }

    #[test]
    fn cycle_pane_wraps_both_ways() {
        let panes = [7, 3, 9];
        assert_eq!(cycle_pane(&panes, 7, true), Some(3));
        assert_eq!(cycle_pane(&panes, 9, true), Some(7));
        assert_eq!(cycle_pane(&panes, 7, false), Some(9));
        assert_eq!(cycle_pane(&panes, 3, false), Some(7));
        assert_eq!(cycle_pane(&[5], 5, true), Some(5));
        assert_eq!(cycle_pane(&panes, 42, true), None);
    }

    /// Remove `gone` from `t` and return the pane chosen for focus.
    fn focus_after(t: &mut SplitTree, gone: PaneId) -> Option<PaneId> {
        let before = t.layout(AREA);
        t.remove(gone);
        focus_after_remove(&before, &t.layout(AREA))
    }

    #[test]
    fn focus_after_remove_picks_promoted_leaf_sibling() {
        // [1 | 2]: removing 2 promotes 1, removing 1 promotes 2.
        let mut t = SplitTree::new(1);
        t.split(1, Orientation::Horizontal, 2);
        assert_eq!(focus_after(&mut t.clone(), 2), Some(1));
        assert_eq!(focus_after(&mut t, 1), Some(2));
    }

    #[test]
    fn focus_after_remove_picks_first_pane_of_promoted_subtree() {
        // [ (1|2) | 3 ]: removing 3 promotes subtree (1|2) -> 1, not the adjacent 2.
        let mut t = SplitTree::new(1);
        t.split(1, Orientation::Horizontal, 3);
        t.split(1, Orientation::Horizontal, 2);
        assert_eq!(t.panes(), vec![1, 2, 3]);
        assert_eq!(focus_after(&mut t, 3), Some(1));
    }

    #[test]
    fn focus_after_remove_ignores_untouched_panes() {
        // [ 1 | (2 over 3) ]: removing 3 promotes 2; 1 keeps its rect.
        let mut t = SplitTree::new(1);
        t.split(1, Orientation::Horizontal, 2);
        t.split(2, Orientation::Vertical, 3);
        assert_eq!(focus_after(&mut t.clone(), 3), Some(2));
        // Removing 2 promotes 3.
        assert_eq!(focus_after(&mut t.clone(), 2), Some(3));
        // Removing 1 promotes the subtree (2 over 3) -> 2.
        assert_eq!(focus_after(&mut t, 1), Some(2));
    }

    #[test]
    fn focus_after_remove_falls_back_to_first_pane() {
        let after = [(4, rect(10, 10)), (5, rect(10, 10))];
        assert_eq!(focus_after_remove(&after, &after), Some(4));
        assert_eq!(focus_after_remove(&[], &[]), None);
    }

    fn tabs_of(trees: Vec<SplitTree>, active: usize) -> Tabs {
        let mut it = trees.into_iter().map(|tree| Tab {
            focused: tree.panes()[0],
            tree,
            title: None,
        });
        let mut tabs = Tabs::new(it.next().unwrap());
        it.for_each(|t| {
            tabs.push(t);
        });
        tabs.select(active);
        tabs
    }

    fn pair(a: PaneId, o: Orientation, b: PaneId) -> SplitTree {
        let mut t = SplitTree::new(a);
        t.split(a, o, b);
        t
    }

    #[test]
    fn remove_pane_in_background_tab_moves_its_focus_only() {
        // [ (1|2), 3 ], tab 2 active; pane 2 focused in tab 1.
        let mut tabs = tabs_of(
            vec![pair(1, Orientation::Horizontal, 2), SplitTree::new(3)],
            1,
        );
        tabs.iter_mut().next().unwrap().focused = 2;
        assert!(remove_pane(&mut tabs, 2, AREA));
        assert_eq!(tabs.get(0).unwrap().tree.panes(), [1]);
        assert_eq!(tabs.get(0).unwrap().focused, 1);
        assert_eq!(tabs.active_index(), 1);
        assert_eq!(tabs.active().focused, 3);
    }

    #[test]
    fn remove_unfocused_pane_keeps_focus() {
        let mut tabs = tabs_of(vec![pair(1, Orientation::Horizontal, 2)], 0);
        assert!(remove_pane(&mut tabs, 2, AREA));
        assert_eq!(tabs.active().focused, 1);
        assert_eq!(tabs.active().tree.panes(), [1]);
    }

    #[test]
    fn remove_focused_maximized_pane_focuses_promoted_sibling() {
        // [ 1 | (2 over 3) ] with 2 maximized: 3 takes 2's place, not 1.
        let mut t = pair(1, Orientation::Horizontal, 2);
        t.split(2, Orientation::Vertical, 3);
        t.toggle_maximize(2);
        let mut tabs = tabs_of(vec![t], 0);
        tabs.active_mut().focused = 2;
        assert!(remove_pane(&mut tabs, 2, AREA));
        assert_eq!(tabs.active().focused, 3);
        assert_eq!(tabs.active().tree.maximized(), None);
    }

    #[test]
    fn remove_last_pane_of_background_tab_closes_it() {
        let trees = vec![SplitTree::new(1), SplitTree::new(2), SplitTree::new(3)];
        let mut tabs = tabs_of(trees, 2);
        assert!(remove_pane(&mut tabs, 1, AREA));
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs.active().tree.panes(), [3], "same tab stays active");
    }

    #[test]
    fn remove_last_pane_of_active_tab_activates_right_neighbour() {
        let trees = vec![SplitTree::new(1), SplitTree::new(2), SplitTree::new(3)];
        let mut tabs = tabs_of(trees, 1);
        assert!(remove_pane(&mut tabs, 2, AREA));
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs.active().tree.panes(), [3]);
    }

    #[test]
    fn remove_last_pane_of_last_tab_leaves_no_tabs() {
        let mut tabs = tabs_of(vec![SplitTree::new(1)], 0);
        assert!(remove_pane(&mut tabs, 1, AREA));
        assert!(tabs.is_empty());
    }

    #[test]
    fn remove_unknown_pane_changes_nothing() {
        let mut tabs = tabs_of(vec![pair(1, Orientation::Horizontal, 2)], 0);
        assert!(!remove_pane(&mut tabs, 9, AREA));
        assert_eq!(tabs.active().tree.panes(), [1, 2]);
    }

    #[test]
    fn split_needs_room_for_the_new_panes_cell() {
        // The focused pane is zoomed out (40 px cells); the new one starts unzoomed with
        // 120 px cells. 201 px fit two 40 px panes but not a 120 px one beside another.
        let mut t = SplitTree::new(1);
        t.set_min_extent(40, 40);
        let o = Orientation::Horizontal;
        assert!(t.can_split(1, o, rect(201, 201)));
        assert!(can_split_with(&t, 1, o, rect(201, 201), (40, 40)));
        assert!(!can_split_with(&t, 1, o, rect(201, 201), (120, 40)));
        assert!(can_split_with(&t, 1, o, rect(250, 201), (120, 40)));
        // The tree's own minimum still counts when the new pane's is smaller.
        t.set_min_extent(120, 40);
        assert!(!can_split_with(&t, 1, o, rect(201, 201), (40, 40)));
        assert_eq!(t.min_extent(), (120, 40));
    }

    #[test]
    fn split_needs_room_on_the_other_axis_too() {
        // The new pane's cells are taller than the focused pane's. Side by side halves only
        // the width, but the new pane still needs `new_min.1` of height: the tab is 100 px
        // tall, so a 120 px minimum can't be met whichever way it splits.
        let mut t = SplitTree::new(1);
        t.set_min_extent(40, 40);
        let (h, v) = (Orientation::Horizontal, Orientation::Vertical);
        assert!(!can_split_with(&t, 1, h, rect(400, 100), (40, 120)));
        assert!(!can_split_with(&t, 1, v, rect(400, 100), (40, 120)));
        // With the height it needs, the same split is fine.
        assert!(can_split_with(&t, 1, h, rect(400, 130), (40, 120)));
        // Stacking halves the height: 130 px can't hold two 120 px panes.
        assert!(!can_split_with(&t, 1, v, rect(400, 130), (40, 120)));
    }

    #[test]
    fn resize_step_is_one_cell() {
        let m = Metrics {
            cell_width: 8,
            cell_height: 16,
            baseline: 12,
        };
        assert_eq!(resize_step(Direction::Left, &m), 8);
        assert_eq!(resize_step(Direction::Right, &m), 8);
        assert_eq!(resize_step(Direction::Up, &m), 16);
        assert_eq!(resize_step(Direction::Down, &m), 16);
    }
}
