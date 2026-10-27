//! Refresh-capped frame scheduler: draw immediately when idle, otherwise at
//! most once per monitor refresh interval. The interval runs from when a
//! frame was due (or began), not from when it finished, so the time spent
//! drawing and waking up doesn't stretch the period.
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Schedule {
    Now,
    At(Instant),
    AlreadyPending,
}

pub struct FrameScheduler {
    interval: Duration,
    /// Earliest start of the next frame: one interval after the last frame's slot.
    next_slot: Option<Instant>,
    pending: bool,
    deadline: Option<Instant>,
}

impl FrameScheduler {
    /// `refresh_millihertz` is the monitor rate (60 Hz = 60_000); `None` or 0 means 60 Hz.
    pub fn new(refresh_millihertz: Option<u32>) -> Self {
        let mhz = match refresh_millihertz {
            Some(m) if m > 0 => m,
            _ => 60_000,
        };
        Self {
            interval: Duration::from_nanos(1_000_000_000_000 / u64::from(mhz)),
            next_slot: None,
            pending: false,
            deadline: None,
        }
    }

    pub fn request(&mut self, now: Instant) -> Schedule {
        if self.pending {
            return Schedule::AlreadyPending;
        }
        self.pending = true;
        match self.next_slot {
            Some(at) if now < at => {
                self.deadline = Some(at);
                Schedule::At(at)
            }
            _ => Schedule::Now,
        }
    }

    /// A frame that began drawing at `began` is done (call it after the swap,
    /// with the time taken before drawing). Its slot is the deadline it was
    /// deferred to, or `began` when it came early or undeferred; the next
    /// frame may start one interval after that slot. A frame that started
    /// more than an interval late counts from `began`: no catch-up burst.
    pub fn frame_drawn(&mut self, began: Instant) {
        let slot = self.deadline.map_or(began, |d| d.min(began));
        let next = slot + self.interval;
        self.next_slot = Some(if next > began {
            next
        } else {
            began + self.interval
        });
        self.pending = false;
        self.deadline = None;
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One refresh interval at 60 Hz.
    const I: Duration = Duration::from_nanos(16_666_666);

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn first_request_is_now() {
        let mut s = FrameScheduler::new(None);
        assert_eq!(s.request(Instant::now()), Schedule::Now);
        assert_eq!(s.deadline(), None);
    }

    #[test]
    fn request_within_interval_is_deferred() {
        let t = Instant::now();
        let mut s = FrameScheduler::new(None);
        s.frame_drawn(t);
        let want = t + Duration::from_nanos(1_000_000_000_000 / 60_000);
        assert_eq!(s.request(t + ms(5)), Schedule::At(want));
        assert_eq!(s.deadline(), Some(want));
    }

    #[test]
    fn second_request_while_pending() {
        let t = Instant::now();
        let mut s = FrameScheduler::new(None);
        s.frame_drawn(t);
        s.request(t + ms(5));
        assert_eq!(s.request(t + ms(6)), Schedule::AlreadyPending);
        let mut s2 = FrameScheduler::new(None);
        assert_eq!(s2.request(t), Schedule::Now);
        assert_eq!(s2.request(t), Schedule::AlreadyPending);
    }

    #[test]
    fn request_after_interval_is_now() {
        let t = Instant::now();
        let mut s = FrameScheduler::new(None);
        s.frame_drawn(t);
        assert_eq!(s.request(t + ms(17)), Schedule::Now);
    }

    #[test]
    fn frame_drawn_clears_pending() {
        let t = Instant::now();
        let mut s = FrameScheduler::new(None);
        s.frame_drawn(t);
        s.request(t + ms(5));
        // The frame deferred to t + I began at t + 17 ms: the next slot is
        // t + 2I, on the cadence.
        s.frame_drawn(t + ms(17));
        assert_eq!(s.deadline(), None);
        assert_eq!(s.request(t + ms(20)), Schedule::At(t + 2 * I));
    }

    #[test]
    fn late_frame_keeps_the_cadence() {
        // A frame deferred to D that starts 3 ms late: the next slot is D + I,
        // not 3 ms later (the drawing and wake-up time must not stretch the period).
        let t = Instant::now();
        let mut s = FrameScheduler::new(None);
        s.frame_drawn(t);
        assert_eq!(s.request(t + ms(1)), Schedule::At(t + I));
        s.frame_drawn(t + I + ms(3));
        assert_eq!(s.request(t + I + ms(5)), Schedule::At(t + 2 * I));
        // A steady flood: 60 frames in one second, each starting 3 ms late.
        let mut s = FrameScheduler::new(None);
        let mut began = t;
        let mut frames = 0;
        while began < t + Duration::from_secs(1) {
            s.frame_drawn(began);
            frames += 1;
            began = match s.request(began + ms(4)) {
                Schedule::At(at) => at + ms(3),
                other => panic!("{other:?}"),
            };
        }
        assert_eq!(frames, 60);
    }

    #[test]
    fn frame_more_than_an_interval_late_reanchors() {
        // No burst of catch-up frames: the next slot is one interval after
        // the late frame began.
        let t = Instant::now();
        let mut s = FrameScheduler::new(None);
        s.frame_drawn(t);
        s.request(t + ms(1));
        let began = t + I + ms(20);
        s.frame_drawn(began);
        assert_eq!(s.request(began + ms(1)), Schedule::At(began + I));
    }

    #[test]
    fn after_idle_draws_now_then_one_interval_later() {
        let t = Instant::now();
        let mut s = FrameScheduler::new(None);
        s.frame_drawn(t);
        let later = t + Duration::from_secs(5);
        assert_eq!(s.request(later), Schedule::Now);
        assert_eq!(s.deadline(), None);
        let began = later + ms(1);
        s.frame_drawn(began);
        assert_eq!(s.request(began + ms(2)), Schedule::At(began + I));
    }

    #[test]
    fn early_frame_counts_from_its_start() {
        // A frame drawn before its deadline (an expose): the next slot is one
        // interval after it began, not after the deadline it pre-empted.
        let t = Instant::now();
        let mut s = FrameScheduler::new(None);
        s.frame_drawn(t);
        s.request(t + ms(1));
        s.frame_drawn(t + ms(10));
        assert_eq!(s.request(t + ms(11)), Schedule::At(t + ms(10) + I));
    }

    #[test]
    fn idle_has_no_deadline() {
        let t = Instant::now();
        let mut s = FrameScheduler::new(None);
        s.frame_drawn(t);
        assert_eq!(s.deadline(), None);
        s.request(t + ms(1));
        s.frame_drawn(t + I);
        assert_eq!(s.deadline(), None, "nothing pending: no wake-up");
    }

    #[test]
    fn uses_monitor_rate() {
        let t = Instant::now();
        let mut s = FrameScheduler::new(Some(144_000));
        s.frame_drawn(t);
        let Schedule::At(at) = s.request(t + ms(1)) else {
            panic!("expected At")
        };
        let d = at - t;
        assert!(
            d > Duration::from_micros(6900) && d < Duration::from_micros(6990),
            "{d:?}"
        );
        let mut z = FrameScheduler::new(Some(0));
        z.frame_drawn(t);
        let Schedule::At(at) = z.request(t + ms(1)) else {
            panic!("expected At")
        };
        assert_eq!(at - t, Duration::from_nanos(16_666_666));
    }
}
