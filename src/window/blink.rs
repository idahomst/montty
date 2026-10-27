//! The focused pane's cursor blink: 600 ms visible, 600 ms hidden, restarting at each key
//! press or output, and stopping (cursor visible) 10 s after the last of them so an idle
//! window has no timer.

use std::time::{Duration, Instant};

/// Length of one visible or hidden phase.
const PHASE: Duration = Duration::from_millis(600);
/// How long after the last activity the cursor keeps blinking.
const STOP_AFTER: Duration = Duration::from_secs(10);

/// What a wake-up at one instant needs: a frame when the visible state differs from the one
/// last drawn, and when to wake next.
#[derive(Debug, PartialEq)]
pub(super) struct Tick {
    pub redraw: bool,
    pub next: Option<Instant>,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Blink {
    pub last_activity: Instant,
}

impl Blink {
    pub fn new(now: Instant) -> Blink {
        Blink { last_activity: now }
    }

    /// Restart the cycle (visible phase) at `now`.
    pub fn reset(&mut self, now: Instant) {
        self.last_activity = now;
    }

    fn elapsed(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.last_activity)
    }

    /// Whether the cursor is drawn at `now`; always, once blinking has stopped.
    pub fn visible(&self, now: Instant) -> bool {
        let e = self.elapsed(now);
        e >= STOP_AFTER || (e.as_millis() / PHASE.as_millis()).is_multiple_of(2)
    }

    /// The next time the visible state changes after `now`; None once blinking has stopped.
    pub fn next_toggle(&self, now: Instant) -> Option<Instant> {
        let e = self.elapsed(now);
        if e >= STOP_AFTER {
            return None;
        }
        let next = (e.as_millis() / PHASE.as_millis() + 1) * PHASE.as_millis();
        let next = Duration::from_millis(next as u64);
        // No toggle at the stop itself: the phase it falls in is a visible one (see
        // `stop_falls_in_a_visible_phase`), so the cursor is already shown then.
        (next < STOP_AFTER).then_some(self.last_activity + next)
    }

    /// The wake-up at `now`, given whether the last frame drew the cursor (`shown`). One
    /// `now` for both answers: a toggle between two separate clock reads would be missed
    /// by the first and scheduled a whole phase late by the second.
    pub fn tick(&self, now: Instant, shown: bool) -> Tick {
        Tick {
            redraw: self.visible(now) != shown,
            next: self.next_toggle(now),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn blink_phases() {
        let t0 = Instant::now();
        let b = Blink::new(t0);
        assert!(b.visible(t0));
        assert!(b.visible(t0 + ms(599)));
        assert!(!b.visible(t0 + ms(600)));
        assert!(!b.visible(t0 + ms(1199)));
        assert!(b.visible(t0 + ms(1200)));
        assert_eq!(b.next_toggle(t0), Some(t0 + ms(600)));
        assert_eq!(b.next_toggle(t0 + ms(600)), Some(t0 + ms(1200)));
        assert_eq!(b.next_toggle(t0 + ms(1000)), Some(t0 + ms(1200)));
    }

    #[test]
    fn stop_falls_in_a_visible_phase() {
        // Pins the constants: blinking stops during an even (visible) phase, so no toggle is
        // needed at the stop and the last one before it is into the visible phase.
        assert_eq!(PHASE, ms(600));
        assert_eq!(STOP_AFTER, Duration::from_secs(10));
        let phase_at = |d: Duration| d.as_millis() / PHASE.as_millis();
        assert_eq!(phase_at(STOP_AFTER - ms(1)) % 2, 0);
        assert_eq!(phase_at(STOP_AFTER) % 2, 0);
    }

    #[test]
    fn one_now_gives_a_consistent_tick() {
        let t0 = Instant::now();
        let b = Blink::new(t0);
        // Just before the boundary: nothing to draw yet, the toggle is next.
        assert_eq!(
            b.tick(t0 + ms(599), true),
            Tick {
                redraw: false,
                next: Some(t0 + ms(600))
            }
        );
        // On it: the toggle is due now (the cursor drawn is stale) and the one after that
        // is a full phase away. A `now` read after the boundary for the deadline but before
        // it for the check would lose this frame.
        assert_eq!(
            b.tick(t0 + ms(600), true),
            Tick {
                redraw: true,
                next: Some(t0 + ms(1200))
            }
        );
        assert!(!b.tick(t0 + ms(600), false).redraw);
        // After the stop: shown, and nothing to wake for.
        assert_eq!(
            b.tick(t0 + ms(10_500), true),
            Tick {
                redraw: false,
                next: None
            }
        );
    }

    #[test]
    fn blink_stops_after_10s() {
        let t0 = Instant::now();
        let b = Blink::new(t0);
        assert!(b.visible(t0 + ms(10_001)));
        assert_eq!(b.next_toggle(t0 + ms(10_001)), None);
        // 9.9 s is in a visible phase (index 16) and nothing changes before the stop.
        assert!(b.visible(t0 + ms(9_900)));
        assert_eq!(b.next_toggle(t0 + ms(9_900)), None);
        // The last toggle that fires is the one into the visible phase at 9.6 s.
        assert_eq!(b.next_toggle(t0 + ms(9_000)), Some(t0 + ms(9_600)));
    }

    #[test]
    fn activity_resets_phase() {
        let t0 = Instant::now();
        let mut b = Blink::new(t0);
        assert!(!b.visible(t0 + ms(700)));
        b.reset(t0 + ms(700));
        assert!(b.visible(t0 + ms(700)));
        assert!(b.visible(t0 + ms(1299)));
        assert!(!b.visible(t0 + ms(1300)));
        // After the stop, activity restarts blinking.
        assert_eq!(b.next_toggle(t0 + ms(20_000)), None);
        b.reset(t0 + ms(20_000));
        assert_eq!(b.next_toggle(t0 + ms(20_000)), Some(t0 + ms(20_600)));
    }
}
