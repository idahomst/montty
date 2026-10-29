//! Attention marks: which panes need the user, and how urgently.
//!
//! Pure state, no timers. Blocked follows the pane's root OSC 7501 record; done, error and
//! one-shot signals (bell, OSC 9/777) are "not seen yet" marks that only `look` clears.

use std::collections::HashMap;

use crate::term::PaneId;
use crate::term::status::{Records, State, display_text, label, parse_report};

/// Ord = urgency.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Done,
    Error,
    Blocked,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dot {
    pub level: Level,
    pub label: String,
}

#[derive(Default)]
pub struct Effect {
    pub redraw: bool,
    /// Label for a desktop notification.
    pub notify: Option<String>,
}

#[derive(Default)]
struct PaneAttention {
    records: Records,
    /// The not-seen mark; a newer one replaces it.
    mark: Option<Dot>,
}

#[derive(Default)]
pub struct Attention {
    panes: HashMap<PaneId, PaneAttention>,
}

fn blocked_dot(p: &PaneAttention) -> Option<Dot> {
    let r = p.records.root().filter(|r| r.state == State::Blocked)?;
    Some(Dot {
        level: Level::Blocked,
        label: label(r),
    })
}

fn shown(p: &PaneAttention) -> Option<Dot> {
    match (blocked_dot(p), p.mark.clone()) {
        (Some(b), Some(m)) => Some(if m.level > b.level { m } else { b }),
        (b, m) => b.or(m),
    }
}

impl Attention {
    pub fn status(&mut self, pane: PaneId, payload: &str, looking: bool) -> Effect {
        let Some(report) = parse_report(payload) else {
            return Effect::default();
        };
        let p = self.panes.entry(pane).or_default();
        let before = shown(p);
        let was = p.records.root().map(|r| r.state);
        p.records.apply(report);
        let now = p.records.root().map(|r| r.state);
        let mut effect = Effect::default();
        if !looking && was != now {
            let level = match now {
                Some(State::Done) => Some(Level::Done),
                Some(State::Error) => Some(Level::Error),
                _ => None,
            };
            if let (Some(level), Some(r)) = (level, p.records.root()) {
                p.mark = Some(Dot {
                    level,
                    label: label(r),
                });
            }
            if now == Some(State::Blocked) {
                effect.notify = blocked_dot(p).map(|d| d.label);
            }
        }
        effect.redraw = shown(p) != before;
        effect
    }

    pub fn prompt_start(&mut self, pane: PaneId) -> Effect {
        let Some(p) = self.panes.get_mut(&pane) else {
            return Effect::default();
        };
        let before = shown(p);
        p.records.prompt_start();
        Effect {
            redraw: shown(p) != before,
            notify: None,
        }
    }

    pub fn reset(&mut self, pane: PaneId) -> Effect {
        let Some(p) = self.panes.remove(&pane) else {
            return Effect::default();
        };
        Effect {
            redraw: shown(&p).is_some(),
            notify: None,
        }
    }

    /// Bell (`notify` false) or OSC 9 / 777 (`notify` true): an amber not-seen mark.
    pub fn signal(&mut self, pane: PaneId, label: &str, notify: bool, looking: bool) -> Effect {
        if looking {
            return Effect::default();
        }
        let text = display_text(label);
        let p = self.panes.entry(pane).or_default();
        let before = shown(p);
        p.mark = Some(Dot {
            level: Level::Blocked,
            label: text.clone(),
        });
        Effect {
            redraw: shown(p) != before,
            notify: notify.then_some(text),
        }
    }

    pub fn look(&mut self, pane: PaneId) -> bool {
        self.panes
            .get_mut(&pane)
            .is_some_and(|p| p.mark.take().is_some())
    }

    pub fn remove(&mut self, pane: PaneId) {
        self.panes.remove(&pane);
    }

    pub fn pane_dot(&self, pane: PaneId, looking: bool) -> Option<Dot> {
        if looking {
            return None;
        }
        self.panes.get(&pane).and_then(shown)
    }

    /// The most urgent dot among `panes`; the pane being looked at shows none.
    pub fn tab_dot(&self, panes: &[PaneId], looking: Option<PaneId>) -> Option<Dot> {
        panes
            .iter()
            .filter_map(|&id| self.pane_dot(id, looking == Some(id)))
            .max_by_key(|d| d.level)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: PaneId = 1;
    const BLOCKED: &str = "state=blocked:kind=permission";

    #[test]
    fn blocked_shows_when_not_looking_and_returns_after_glance() {
        let mut a = Attention::default();
        a.status(P, BLOCKED, true);
        assert_eq!(a.pane_dot(P, true), None);
        assert_eq!(a.pane_dot(P, false).unwrap().level, Level::Blocked);
        a.look(P);
        assert_eq!(a.pane_dot(P, false).unwrap().level, Level::Blocked);
        a.status(P, "state=working", false);
        assert_eq!(a.pane_dot(P, false), None);
    }

    #[test]
    fn done_not_seen_survives_next_turn() {
        let mut a = Attention::default();
        a.status(P, "state=done", false);
        assert_eq!(a.pane_dot(P, false).unwrap().level, Level::Done);
        a.status(P, "state=working", false);
        assert_eq!(a.pane_dot(P, false).unwrap().level, Level::Done);
        assert!(a.look(P));
        assert_eq!(a.pane_dot(P, false), None);
    }

    #[test]
    fn done_while_looking_leaves_no_mark() {
        let mut a = Attention::default();
        a.status(P, "state=done", true);
        assert_eq!(a.pane_dot(P, false), None);
    }

    #[test]
    fn tab_dot_picks_most_urgent() {
        let mut a = Attention::default();
        a.status(1, "state=done", false);
        a.status(2, BLOCKED, false);
        let d = a.tab_dot(&[1, 2], None).unwrap();
        assert_eq!(d.level, Level::Blocked);
        assert_eq!(d.label, "Needs permission");
        assert_eq!(a.tab_dot(&[1, 2], Some(2)).unwrap().level, Level::Done);
    }

    #[test]
    fn removed_pane_leaves_no_tab_dot() {
        let mut a = Attention::default();
        a.status(1, "state=done", false);
        a.remove(1);
        assert_eq!(a.tab_dot(&[1], None), None);
    }

    #[test]
    fn notify_only_on_entering_blocked_or_notify_signal() {
        let mut a = Attention::default();
        assert!(a.status(P, BLOCKED, false).notify.is_some());
        assert!(a.status(P, BLOCKED, false).notify.is_none());
        assert!(a.signal(P, "Bell", false, false).notify.is_none());
        assert_eq!(a.signal(P, "hi", true, false).notify.as_deref(), Some("hi"));
        assert!(a.signal(P, "hi", true, true).notify.is_none());
    }

    #[test]
    fn prompt_start_and_reset_clear_blocked() {
        let mut a = Attention::default();
        a.status(P, BLOCKED, false);
        a.status(2, "state=done", false);
        a.prompt_start(P);
        assert_eq!(a.pane_dot(P, false), None);
        a.prompt_start(2);
        assert!(a.pane_dot(2, false).is_some());
        a.reset(2);
        assert_eq!(a.pane_dot(2, false), None);
    }
}
