//! Pure tab collection: each tab owns a split tree. No GL, no PTYs.

use crate::layout::{PaneId, SplitTree};

/// One tab: a split tree, the focused pane within it and an optional
/// user-chosen title.
pub struct Tab {
    pub tree: SplitTree,
    pub focused: PaneId,
    pub title: Option<String>,
}

impl Tab {
    /// A tab holding a single pane; `min` is the tree's minimum pane extent.
    pub fn new(pane: PaneId, min: (i32, i32)) -> Tab {
        let mut tree = SplitTree::new(pane);
        tree.set_min_extent(min.0, min.1);
        Tab {
            tree,
            focused: pane,
            title: None,
        }
    }
}

/// Ordered tabs plus the active index.
///
/// Invariant: `active < tabs.len()` whenever the collection is non-empty.
/// `remove` may leave it empty; the window closes before using it again, so
/// `active()`/`active_mut()` panic if called when empty.
pub struct Tabs {
    tabs: Vec<Tab>,
    active: usize,
    /// Bumped whenever tabs are added, removed or reordered.
    generation: u64,
}

impl Tabs {
    pub fn new(first: Tab) -> Tabs {
        Tabs {
            tabs: vec![first],
            active: 0,
            generation: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    /// The active tab. Panics if empty.
    pub fn active(&self) -> &Tab {
        &self.tabs[self.active]
    }

    /// The active tab. Panics if empty.
    pub fn active_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    /// Changes whenever tabs are added, removed or reordered (not when the
    /// active tab changes): a tab index taken before may now name another tab.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn get(&self, i: usize) -> Option<&Tab> {
        self.tabs.get(i)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Tab> {
        self.tabs.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Tab> {
        self.tabs.iter_mut()
    }

    /// Appends and activates; returns the new index.
    pub fn push(&mut self, t: Tab) -> usize {
        self.tabs.push(t);
        self.generation += 1;
        self.active = self.tabs.len() - 1;
        self.active
    }

    /// Removes tab `i`. Active tab removed: the right neighbour (now at `i`)
    /// becomes active, else the left one. A tab before the active one was
    /// removed: the same tab stays active.
    pub fn remove(&mut self, i: usize) -> Option<Tab> {
        if i >= self.tabs.len() {
            return None;
        }
        let t = self.tabs.remove(i);
        self.generation += 1;
        if i < self.active {
            self.active -= 1;
        } else if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        }
        Some(t)
    }

    pub fn select(&mut self, i: usize) -> bool {
        if i < self.tabs.len() {
            self.active = i;
            true
        } else {
            false
        }
    }

    pub fn next(&mut self) {
        if !self.tabs.is_empty() {
            self.active = (self.active + 1) % self.tabs.len();
        }
    }

    pub fn prev(&mut self) {
        if !self.tabs.is_empty() {
            self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
        }
    }

    /// Moves the active tab by `delta` places without wrapping; false at an edge.
    pub fn move_active(&mut self, delta: isize) -> bool {
        let to = self.active as isize + delta;
        if self.tabs.is_empty() || to < 0 || to >= self.tabs.len() as isize {
            return false;
        }
        let t = self.tabs.remove(self.active);
        self.tabs.insert(to as usize, t);
        self.generation += 1;
        self.active = to as usize;
        true
    }

    /// Activates the 1-based tab `n`.
    pub fn go_to(&mut self, n: u8) -> bool {
        n >= 1 && self.select(n as usize - 1)
    }

    pub fn tab_of(&self, pane: PaneId) -> Option<usize> {
        self.tabs
            .iter()
            .position(|t| t.tree.panes().contains(&pane))
    }
}

/// True when `pane` is in the active tab and not hidden behind another
/// maximized pane.
pub fn visible(tabs: &Tabs, pane: PaneId) -> bool {
    if tabs.is_empty() || !tabs.active().tree.panes().contains(&pane) {
        return false;
    }
    tabs.active().tree.maximized().is_none_or(|m| m == pane)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Orientation;

    fn tab(id: PaneId) -> Tab {
        Tab::new(id, (32, 32))
    }

    /// First pane of each tab, in order.
    fn order(t: &Tabs) -> Vec<PaneId> {
        t.iter().map(|x| x.tree.panes()[0]).collect()
    }

    fn three() -> Tabs {
        let mut t = Tabs::new(tab(1));
        t.push(tab(2));
        t.push(tab(3));
        t
    }

    #[test]
    fn push_appends_and_activates() {
        let t = three();
        assert_eq!(order(&t), [1, 2, 3]);
        assert_eq!(t.active_index(), 2);
        assert_eq!(t.len(), 3);
    }

    #[test]
    fn remove_active_picks_right_then_left() {
        let mut t = three();
        t.select(1);
        assert!(t.remove(1).is_some());
        assert_eq!(t.active().tree.panes(), [3]);
        assert_eq!(t.active_index(), 1);
        assert!(t.remove(1).is_some());
        assert_eq!(t.active_index(), 0);
        assert_eq!(order(&t), [1]);
    }

    #[test]
    fn remove_before_active_keeps_same_tab() {
        let mut t = three();
        t.remove(0);
        assert_eq!(t.active_index(), 1);
        assert_eq!(t.active().tree.panes(), [3]);
    }

    #[test]
    fn remove_last_leaves_empty() {
        let mut t = Tabs::new(tab(1));
        assert!(t.remove(0).is_some());
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
        assert!(!t.select(0));
        assert!(t.tab_of(1).is_none());
    }

    #[test]
    fn remove_out_of_range_is_none() {
        let mut t = three();
        assert!(t.remove(3).is_none());
        assert_eq!(t.len(), 3);
        assert_eq!(t.active_index(), 2);
    }

    #[test]
    fn next_prev_wrap() {
        let mut t = three();
        t.select(0);
        t.prev();
        assert_eq!(t.active_index(), 2);
        t.next();
        assert_eq!(t.active_index(), 0);
    }

    #[test]
    fn move_active_no_wrap() {
        let mut t = three();
        t.select(0);
        assert!(!t.move_active(-1));
        assert!(t.move_active(1));
        assert_eq!(order(&t), [2, 1, 3]);
        assert_eq!(t.active_index(), 1);
        assert_eq!(t.active().tree.panes(), [1]);
        assert!(t.move_active(1));
        assert!(!t.move_active(1));
        assert_eq!(order(&t), [2, 3, 1]);
    }

    #[test]
    fn go_to_is_one_based() {
        let mut t = three();
        assert!(t.go_to(2));
        assert_eq!(t.active_index(), 1);
        assert!(!t.go_to(9));
        assert!(!t.go_to(0));
        assert_eq!(t.active_index(), 1);
        let mut t = Tabs::new(tab(1));
        for id in 2..=10 {
            t.push(tab(id));
        }
        t.select(0);
        assert!(t.go_to(10));
        assert_eq!(t.active_index(), 9);
    }

    #[test]
    fn tab_of_finds_pane() {
        let mut t = three();
        assert!(
            t.iter_mut()
                .nth(1)
                .unwrap()
                .tree
                .split(2, Orientation::Horizontal, 20)
        );
        assert_eq!(t.tab_of(20), Some(1));
        assert_eq!(t.tab_of(3), Some(2));
        assert_eq!(t.tab_of(99), None);
        assert_eq!(t.get(1).unwrap().tree.panes().len(), 2);
        assert!(t.get(3).is_none());
    }

    #[test]
    fn pane_visibility_rules() {
        let mut t = three();
        t.select(0);
        assert!(visible(&t, 1));
        assert!(!visible(&t, 2), "inactive tab");
        assert!(t.active_mut().tree.split(1, Orientation::Horizontal, 10));
        assert!(visible(&t, 10));
        t.active_mut().tree.toggle_maximize(10);
        assert!(visible(&t, 10));
        assert!(!visible(&t, 1), "hidden behind maximized pane");
        assert!(!visible(&t, 99), "unknown pane");
    }

    #[test]
    fn generation_changes_on_structure_only() {
        let mut t = three();
        let g = t.generation();
        t.select(0);
        t.next();
        t.prev();
        t.go_to(2);
        assert!(!t.move_active(-5), "refused move");
        assert!(t.remove(7).is_none(), "out of range");
        assert_eq!(t.generation(), g, "switching tabs keeps indices");
        let mut seen = vec![g];
        t.push(tab(4));
        seen.push(t.generation());
        assert!(t.move_active(-1));
        seen.push(t.generation());
        t.remove(0);
        seen.push(t.generation());
        seen.dedup();
        assert_eq!(seen.len(), 4, "push, move and remove each change it");
    }

    #[test]
    fn tab_new_sets_min_extent_and_focus() {
        let t = tab(7);
        assert_eq!(t.focused, 7);
        assert!(t.title.is_none());
    }
}
