//! Desktop notifications through the `notify-send` command (no extra crate).

use crate::layout::PaneId;
use std::collections::HashMap;
use std::io::ErrorKind;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// At most one notification per pane in this span; later ones are dropped, not queued.
pub const NOTIFY_INTERVAL: Duration = Duration::from_secs(10);

pub struct Notifier {
    last: HashMap<PaneId, Instant>,
    disabled: bool,
}

impl Default for Notifier {
    fn default() -> Self {
        Self::new()
    }
}

impl Notifier {
    pub fn new() -> Self {
        Notifier {
            last: HashMap::new(),
            disabled: false,
        }
    }

    /// Whether `pane` may notify at `now`; records the send when it may.
    pub fn allow(&mut self, pane: PaneId, now: Instant) -> bool {
        if self.disabled {
            return false;
        }
        if let Some(&t) = self.last.get(&pane)
            && now.saturating_duration_since(t) < NOTIFY_INTERVAL
        {
            return false;
        }
        self.last.insert(pane, now);
        true
    }

    /// Spawns `notify-send` and returns at once; a detached thread reaps the child. A missing
    /// binary disables the notifier after one stderr line.
    pub fn send(&mut self, title: &str, body: &str) {
        if self.disabled {
            return;
        }
        let spawned = Command::new("notify-send")
            .args(["-a", "montty", "-i", "montty", "--", title, body])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match spawned {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {
                eprintln!("montty: notify-send not found; desktop notifications disabled");
                self.disabled = true;
            }
            Err(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_rate_limits_per_pane() {
        let mut n = Notifier::new();
        let t0 = Instant::now();
        let s = Duration::from_secs;
        assert!(n.allow(1, t0));
        assert!(!n.allow(1, t0 + s(9)));
        assert!(n.allow(1, t0 + s(10)));
        assert!(n.allow(2, t0 + s(1)));
    }
}
