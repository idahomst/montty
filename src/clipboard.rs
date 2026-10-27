//! The system CLIPBOARD and PRIMARY selection (copypasta: X11 or Wayland).

use copypasta::ClipboardProvider;
use copypasta::wayland_clipboard;
use copypasta::x11_clipboard::{Clipboard, Primary, X11ClipboardContext};
use raw_window_handle::RawDisplayHandle;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

/// How long a paste waits for the selection owner.
pub const LOAD_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipKind {
    Clipboard,
    Primary,
}

pub trait Clipboards {
    fn store(&mut self, kind: ClipKind, text: String);
    fn load(&mut self, kind: ClipKind) -> Option<String>;
}

/// An in-memory pair: the tests' clipboards and the fallback.
#[derive(Default)]
pub struct MemClipboards {
    clipboard: Option<String>,
    primary: Option<String>,
}

impl Clipboards for MemClipboards {
    fn store(&mut self, kind: ClipKind, text: String) {
        match kind {
            ClipKind::Clipboard => self.clipboard = Some(text),
            ClipKind::Primary => self.primary = Some(text),
        }
    }

    fn load(&mut self, kind: ClipKind) -> Option<String> {
        match kind {
            ClipKind::Clipboard => self.clipboard.clone(),
            ClipKind::Primary => self.primary.clone(),
        }
    }
}

/// The system's clipboards over copypasta providers. A failure never propagates:
/// a store is dropped and the first one prints to stderr; a load is `None`, silently.
struct System {
    clipboard: Box<dyn ClipboardProvider + Send>,
    primary: Box<dyn ClipboardProvider + Send>,
    reported: bool,
}

impl System {
    fn report(&mut self, e: &dyn std::fmt::Display) {
        if !std::mem::replace(&mut self.reported, true) {
            eprintln!("montty: clipboard: {e}");
        }
    }
}

impl Clipboards for System {
    fn store(&mut self, kind: ClipKind, text: String) {
        let p = match kind {
            ClipKind::Clipboard => &mut self.clipboard,
            ClipKind::Primary => &mut self.primary,
        };
        if let Err(e) = p.set_contents(text) {
            self.report(&e);
        }
    }

    fn load(&mut self, kind: ClipKind) -> Option<String> {
        let p = match kind {
            ClipKind::Clipboard => &mut self.clipboard,
            ClipKind::Primary => &mut self.primary,
        };
        // An empty or unowned selection is an error on X11 but normal: not reported,
        // so it can't use up the one report a real failure needs.
        p.get_contents().ok()
    }
}

enum Request {
    Store(ClipKind, String),
    Load(u64, ClipKind),
}

/// Clipboards served by a worker thread (named "clipboard"): stores and loads
/// run there in order; `load` waits at most `timeout`, then returns None and
/// prints a note. A late answer is dropped by its request number.
pub struct Threaded {
    tx: Sender<Request>,
    rx: Receiver<(u64, Option<String>)>,
    next_id: u64,
    timeout: Duration,
    /// Closed when the worker thread ends (its `inner` dropped by then).
    done: Receiver<()>,
}

impl Threaded {
    pub fn spawn(mut inner: Box<dyn Clipboards + Send>, timeout: Duration) -> Threaded {
        let (tx, requests) = mpsc::channel();
        let (replies, rx) = mpsc::channel();
        let (finished, done) = mpsc::channel::<()>();
        let worker = std::thread::Builder::new()
            .name("clipboard".into())
            .spawn(move || {
                let _finished = finished;
                while let Ok(request) = requests.recv() {
                    match request {
                        Request::Store(kind, text) => inner.store(kind, text),
                        Request::Load(id, kind) => {
                            if replies.send((id, inner.load(kind))).is_err() {
                                break;
                            }
                        }
                    }
                }
                drop(inner);
            });
        if let Err(e) = worker {
            eprintln!("montty: clipboard: {e}");
        }
        Threaded {
            tx,
            rx,
            next_id: 0,
            timeout,
            done,
        }
    }
}

impl Drop for Threaded {
    /// Close the request channel, then wait for the worker to drop its clipboards
    /// (the Wayland one joins its own thread, which must end before the display goes).
    /// A worker stuck in a load is given `timeout`, then left behind.
    fn drop(&mut self) {
        let (closed, _) = mpsc::channel();
        drop(std::mem::replace(&mut self.tx, closed));
        let _ = self.done.recv_timeout(self.timeout);
    }
}

impl Clipboards for Threaded {
    fn store(&mut self, kind: ClipKind, text: String) {
        let _ = self.tx.send(Request::Store(kind, text));
    }

    fn load(&mut self, kind: ClipKind) -> Option<String> {
        self.next_id += 1;
        let id = self.next_id;
        self.tx.send(Request::Load(id, kind)).ok()?;
        let deadline = Instant::now() + self.timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok((got, text)) if got == id => return text,
                // A reply to an earlier load that gave up.
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return None,
            }
        }
        let t = self.timeout;
        let waited = if t.as_millis().is_multiple_of(1000) {
            format!("{} s", t.as_secs())
        } else {
            format!("{} ms", t.as_millis())
        };
        eprintln!("montty: clipboard: no answer in {waited}, nothing pasted");
        None
    }
}

fn x11() -> Result<System, Box<dyn std::error::Error + Send + Sync>> {
    Ok(System {
        clipboard: Box::new(X11ClipboardContext::<Clipboard>::new()?),
        primary: Box::new(X11ClipboardContext::<Primary>::new()?),
        reported: false,
    })
}

/// The clipboards of the window's display: Wayland's, else X11's.
pub fn system_clipboards(display: RawDisplayHandle) -> Box<dyn Clipboards> {
    if let RawDisplayHandle::Wayland(h) = display {
        // SAFETY: winit's display pointer stays valid while the window (which owns
        // these clipboards) lives: dropping `Threaded` waits (up to `LOAD_TIMEOUT`) for
        // the worker to drop them, which joins smithay-clipboard's thread.
        let (primary, clipboard) =
            unsafe { wayland_clipboard::create_clipboards_from_external(h.display.as_ptr()) };
        let system = System {
            clipboard: Box::new(clipboard),
            primary: Box::new(primary),
            reported: false,
        };
        return Box::new(Threaded::spawn(Box::new(system), LOAD_TIMEOUT));
    }
    match x11() {
        Ok(s) => Box::new(s),
        Err(e) => {
            eprintln!("montty: clipboard: {e}");
            Box::new(MemClipboards::default())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::time::Instant;

    /// Each load sleeps for the next delay, then answers with the next text.
    struct Scripted(VecDeque<(Duration, &'static str)>);

    impl Clipboards for Scripted {
        fn store(&mut self, _: ClipKind, _: String) {}

        fn load(&mut self, _: ClipKind) -> Option<String> {
            let (delay, text) = self.0.pop_front()?;
            std::thread::sleep(delay);
            Some(text.to_string())
        }
    }

    fn scripted(steps: &[(u64, &'static str)]) -> Box<Scripted> {
        Box::new(Scripted(
            steps
                .iter()
                .map(|&(ms, t)| (Duration::from_millis(ms), t))
                .collect(),
        ))
    }

    #[test]
    fn threaded_passes_store_and_load_through() {
        let mut c = Threaded::spawn(Box::new(MemClipboards::default()), Duration::from_secs(1));
        c.store(ClipKind::Clipboard, "x".into());
        assert_eq!(c.load(ClipKind::Clipboard).as_deref(), Some("x"));
        assert_eq!(c.load(ClipKind::Primary), None);
    }

    #[test]
    fn hung_load_times_out() {
        let mut c = Threaded::spawn(scripted(&[(5000, "never")]), Duration::from_millis(50));
        let start = Instant::now();
        assert_eq!(c.load(ClipKind::Clipboard), None);
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn late_reply_is_not_used_by_the_next_load() {
        let mut c = Threaded::spawn(
            scripted(&[(150, "first"), (0, "second")]),
            Duration::from_millis(50),
        );
        assert_eq!(c.load(ClipKind::Clipboard), None);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(c.load(ClipKind::Clipboard).as_deref(), Some("second"));
    }

    /// Records its own drop.
    struct DropFlag(std::sync::Arc<std::sync::atomic::AtomicBool>, Duration);

    impl Clipboards for DropFlag {
        fn store(&mut self, _: ClipKind, _: String) {}

        fn load(&mut self, _: ClipKind) -> Option<String> {
            std::thread::sleep(self.1);
            None
        }
    }

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[test]
    fn dropping_threaded_drops_the_clipboards_first() {
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let c = Threaded::spawn(
            Box::new(DropFlag(flag.clone(), Duration::ZERO)),
            Duration::from_secs(5),
        );
        drop(c);
        assert!(flag.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn dropping_threaded_with_a_stuck_worker_is_bounded() {
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut c = Threaded::spawn(
            Box::new(DropFlag(flag, Duration::from_secs(5))),
            Duration::from_millis(100),
        );
        assert_eq!(c.load(ClipKind::Clipboard), None);
        let start = Instant::now();
        drop(c);
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn load_after_the_worker_is_gone_is_none() {
        let mut c = Threaded::spawn(Box::new(MemClipboards::default()), Duration::from_secs(5));
        // Stand in for a dead worker: swap in channels nobody serves.
        let (tx, _) = mpsc::channel();
        c.tx = tx;
        let (_, rx) = mpsc::channel();
        c.rx = rx;
        let start = Instant::now();
        assert_eq!(c.load(ClipKind::Clipboard), None);
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn mem_clipboards_keep_kinds_apart() {
        let mut c = MemClipboards::default();
        assert_eq!(c.load(ClipKind::Clipboard), None);
        c.store(ClipKind::Clipboard, "clip".into());
        c.store(ClipKind::Primary, "prim".into());
        assert_eq!(c.load(ClipKind::Clipboard).as_deref(), Some("clip"));
        assert_eq!(c.load(ClipKind::Primary).as_deref(), Some("prim"));
    }
}
