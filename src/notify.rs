//! Desktop notifications through `notify-send` (Linux) or `osascript` (macOS), with no extra
//! crate.

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

    /// Spawns the notifier and returns at once; a detached thread reaps the child. A missing
    /// binary disables the notifier after one stderr line.
    pub fn send(&mut self, title: &str, body: &str) {
        if self.disabled {
            return;
        }
        let (prog, args) = notifier_command(title, body);
        let spawned = Command::new(prog)
            .args(&args)
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
                eprintln!("montty: {prog} not found; desktop notifications disabled");
                self.disabled = true;
            }
            Err(_) => {}
        }
    }
}

/// The program that shows one notification, and its arguments.
fn notifier_command(title: &str, body: &str) -> (&'static str, Vec<String>) {
    if cfg!(target_os = "macos") {
        // Title and body are argv items after `--`, never script text, so OSC 9/777 text
        // can't inject AppleScript.
        let script = [
            "-e",
            "on run argv",
            "-e",
            "display notification (item 2 of argv) with title (item 1 of argv)",
            "-e",
            "end run",
            "--",
        ];
        let mut args: Vec<String> = script.iter().map(|s| s.to_string()).collect();
        args.extend([title.to_string(), body.to_string()]);
        ("osascript", args)
    } else {
        let args = ["-a", "montty", "-i", "montty", "--"]
            .iter()
            .map(|s| s.to_string())
            .chain([escape_markup(title), escape_markup(body)])
            .collect();
        ("notify-send", args)
    }
}

/// Notification servers parse markup in the title and body, and OSC 9/777 text is untrusted.
fn escape_markup(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifier_command_per_os() {
        let (prog, args) = notifier_command("<b>t</b>", "-x \"q\"");
        if cfg!(target_os = "macos") {
            assert_eq!(prog, "osascript");
            // Title and body are arguments after `--`, never part of the script (no injection,
            // and a leading '-' is not an option).
            let n = args.len();
            assert_eq!(&args[n - 3..], ["--", "<b>t</b>", "-x \"q\""]);
            assert!(args.iter().take(n - 3).all(|a| !a.contains("<b>")));
        } else {
            assert_eq!(prog, "notify-send");
            assert_eq!(args.last().map(String::as_str), Some("-x \"q\""));
            assert!(args.contains(&"&lt;b&gt;t&lt;/b&gt;".to_string()));
        }
    }

    #[test]
    fn markup_is_escaped() {
        assert_eq!(
            escape_markup("a & <b>x</b> &amp;"),
            "a &amp; &lt;b&gt;x&lt;/b&gt; &amp;amp;"
        );
        assert_eq!(escape_markup("plain"), "plain");
    }

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
