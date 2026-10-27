//! The app's windows and the requests a window makes of the app.

use std::collections::HashMap;

use winit::window::WindowId;

use crate::layout::PaneId;
use crate::session::SessionDoc;

/// How a new window starts.
pub enum WindowInit {
    /// One pane under `profile` (None: the store's default), running `command` or the
    /// profile's shell.
    Default {
        profile: Option<String>,
        command: Option<Vec<String>>,
    },
    /// The window's content comes from the session `doc` (saved as `name`, if it has a
    /// name); `profile` is the window's default profile (None: the store's default).
    Session {
        doc: SessionDoc,
        name: Option<String>,
        profile: Option<String>,
    },
}

/// What a window asks of the app; drained by the app after every event it delivers.
pub enum AppRequest {
    NewWindow {
        profile: Option<String>,
    },
    OpenSession {
        doc: SessionDoc,
        name: String,
        profile: Option<String>,
    },
}

/// The open windows by id. Generic so the routing can be tested without a display.
pub struct Windows<W> {
    map: HashMap<WindowId, W>,
}

impl<W> Default for Windows<W> {
    fn default() -> Self {
        Windows {
            map: HashMap::new(),
        }
    }
}

impl<W> Windows<W> {
    pub fn insert(&mut self, id: WindowId, w: W) {
        self.map.insert(id, w);
    }

    pub fn remove(&mut self, id: WindowId) -> Option<W> {
        self.map.remove(&id)
    }

    pub fn get_mut(&mut self, id: WindowId) -> Option<&mut W> {
        self.map.get_mut(&id)
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (WindowId, &mut W)> {
        self.map.iter_mut().map(|(id, w)| (*id, w))
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// The window that owns `pane`; pane ids are unique for the whole process, so at
    /// most one does.
    pub fn owner_of(
        &self,
        pane: PaneId,
        has_pane: impl Fn(&W, PaneId) -> bool,
    ) -> Option<WindowId> {
        self.map
            .iter()
            .find(|(_, w)| has_pane(w, pane))
            .map(|(id, _)| *id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(Vec<PaneId>);

    fn has(w: &Fake, p: PaneId) -> bool {
        w.0.contains(&p)
    }

    fn wid(n: u64) -> WindowId {
        WindowId::from(n)
    }

    fn two() -> Windows<Fake> {
        let mut ws = Windows::default();
        ws.insert(wid(1), Fake(vec![10, 11]));
        ws.insert(wid(2), Fake(vec![20]));
        ws
    }

    #[test]
    fn routes_events_to_owner() {
        let ws = two();
        assert_eq!(ws.owner_of(11, has), Some(wid(1)));
        assert_eq!(ws.owner_of(20, has), Some(wid(2)));
    }

    #[test]
    fn events_for_closed_window_dropped() {
        let mut ws = two();
        assert!(ws.remove(wid(2)).is_some());
        assert_eq!(ws.owner_of(20, has), None);
        assert!(ws.get_mut(wid(2)).is_none());
        assert_eq!(ws.owner_of(10, has), Some(wid(1)));
    }

    #[test]
    fn last_window_close_exits() {
        let mut ws = two();
        ws.remove(wid(1));
        assert!(!ws.is_empty());
        ws.remove(wid(2));
        assert!(ws.is_empty());
    }
}
