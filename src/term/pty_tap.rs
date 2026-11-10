//! A PTY wrapper that lets the scanner observe the byte stream on its way to the parser.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Read};
use std::sync::{Arc, Mutex, PoisonError};

use alacritty_terminal::event::{OnResize, WindowSize};
use alacritty_terminal::tty::{ChildEvent, EventedPty, EventedReadWrite, Pty};
use polling::{Event, PollMode, Poller};

use super::PaneId;
use super::osc_scan::{OscScanner, Signal};
use crate::events::{EventSink, UserEvent};

/// Signals kept for the UI at once; beyond it the oldest is dropped. A program reports a
/// handful between two UI turns; only a flood (a crafted or binary file sent to the
/// terminal) gets here.
pub const MAX_PENDING: usize = 256;

/// A pane's signals on their way from its PTY reader thread to the UI. Output is
/// untrusted and arrives faster than the UI turns, so the signals wait here, bounded,
/// and the UI is woken once per batch rather than once per signal.
#[derive(Default)]
pub struct SignalQueue {
    pending: Mutex<VecDeque<Signal>>,
}

impl SignalQueue {
    /// Queue `signals` in order. A signal equal to the one before it is dropped, as is a
    /// feature query while one waits (it gets one reply either way). True when the queue
    /// was empty and no longer is: the caller wakes the UI.
    fn push(&self, signals: impl Iterator<Item = Signal>) -> bool {
        let mut q = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let was_empty = q.is_empty();
        for s in signals {
            let repeat = q.back() == Some(&s);
            if repeat || (s == Signal::StatusQuery && q.contains(&s)) {
                continue;
            }
            if q.len() == MAX_PENDING {
                q.pop_front();
            }
            q.push_back(s);
        }
        was_empty && !q.is_empty()
    }

    /// Everything queued, oldest first; the queue is empty after.
    pub fn take(&self) -> Vec<Signal> {
        let mut q = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        q.drain(..).collect()
    }
}

/// Reads from `R` unchanged; every successful read is also fed to the scanner. The signals
/// found go to the pane's `SignalQueue`, and one `UserEvent::Signals` wakes the UI when the
/// queue was empty.
pub struct TapReader<R: Read> {
    inner: R,
    scanner: OscScanner,
    pane: PaneId,
    sink: Arc<dyn EventSink>,
    queue: Arc<SignalQueue>,
    /// Reused across reads so a read without signals allocates nothing.
    signals: Vec<Signal>,
}

impl<R: Read> TapReader<R> {
    pub fn new(inner: R, pane: PaneId, sink: Arc<dyn EventSink>, queue: Arc<SignalQueue>) -> Self {
        TapReader {
            inner,
            scanner: OscScanner::new(),
            pane,
            sink,
            queue,
            signals: Vec::new(),
        }
    }
}

impl<R: Read> Read for TapReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.signals.clear();
        self.scanner.feed(&buf[..n], &mut self.signals);
        // No lock on the common path: most reads hold no signal.
        if !self.signals.is_empty() && self.queue.push(self.signals.drain(..)) {
            self.sink.send(UserEvent::Signals(self.pane));
        }
        Ok(n)
    }
}

/// `Pty` with a tapped reader; everything else is delegated.
pub struct TapPty {
    inner: Pty,
    reader: TapReader<File>,
}

impl TapPty {
    pub fn new(
        inner: Pty,
        pane: PaneId,
        sink: Arc<dyn EventSink>,
        queue: Arc<SignalQueue>,
    ) -> io::Result<TapPty> {
        // A dup shares the open file description, so the non-blocking flag and the poll
        // registration made on the original fd apply to it.
        let reader = TapReader::new(inner.file().try_clone()?, pane, sink, queue);
        Ok(TapPty { inner, reader })
    }
}

impl EventedReadWrite for TapPty {
    type Reader = TapReader<File>;
    type Writer = File;

    unsafe fn register(&mut self, poll: &Arc<Poller>, i: Event, m: PollMode) -> io::Result<()> {
        unsafe { self.inner.register(poll, i, m) }
    }

    fn reregister(&mut self, poll: &Arc<Poller>, i: Event, m: PollMode) -> io::Result<()> {
        self.inner.reregister(poll, i, m)
    }

    fn deregister(&mut self, poll: &Arc<Poller>) -> io::Result<()> {
        self.inner.deregister(poll)
    }

    fn reader(&mut self) -> &mut Self::Reader {
        &mut self.reader
    }

    fn writer(&mut self) -> &mut Self::Writer {
        self.inner.writer()
    }
}

impl EventedPty for TapPty {
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        self.inner.next_child_event()
    }
}

impl OnResize for TapPty {
    fn on_resize(&mut self, size: WindowSize) {
        self.inner.on_resize(size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::mpsc::channel;

    fn read_all(input: &[u8], chunk: usize) -> (Vec<u8>, Vec<UserEvent>, Arc<SignalQueue>) {
        let (tx, rx) = channel::<UserEvent>();
        let sink: Arc<dyn EventSink> = Arc::new(tx);
        let queue = Arc::new(SignalQueue::default());
        let mut r = TapReader::new(Cursor::new(input.to_vec()), 7, sink, queue.clone());
        let mut got = Vec::new();
        let mut buf = vec![0u8; chunk];
        loop {
            let n = r.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            got.extend_from_slice(&buf[..n]);
        }
        (got, rx.try_iter().collect(), queue)
    }

    #[test]
    fn reader_passes_bytes_and_reports_signals() {
        let input = b"hi\x1b]7501;?\x07there";
        let (got, evs, queue) = read_all(input, 5);
        assert_eq!(got, input);
        assert_eq!(evs.len(), 1);
        assert!(matches!(&evs[0], UserEvent::Signals(7)));
        assert_eq!(queue.take(), [Signal::StatusQuery]);
        assert!(queue.take().is_empty());
    }

    #[test]
    fn a_flood_wakes_the_ui_once_and_stays_bounded() {
        let mut input = Vec::new();
        for i in 0..10_000 {
            input.extend_from_slice(format!("\x1b]9;n{i}\x07\x1b]7501;?\x07").as_bytes());
        }
        // Small reads, as a PTY gives them: many reads, each with signals.
        let (_, evs, queue) = read_all(&input, 4096);
        assert_eq!(evs.len(), 1, "one wake while the UI hasn't taken the batch");
        let got = queue.take();
        assert_eq!(got.len(), MAX_PENDING);
        // The newest survive, and the query waits once however often it was asked.
        assert_eq!(got.last(), Some(&Signal::Notify("n9999".into())));
        let queries = got.iter().filter(|s| **s == Signal::StatusQuery).count();
        assert!(queries <= 1, "{queries}");
    }

    #[test]
    fn queue_drops_repeats_and_wakes_again_once_taken() {
        let q = SignalQueue::default();
        let n = |s: &str| Signal::Notify(s.into());
        assert!(q.push([n("a"), n("a"), Signal::PromptStart, Signal::PromptStart].into_iter()));
        assert!(!q.push([Signal::PromptStart, n("a")].into_iter()));
        assert_eq!(q.take(), [n("a"), Signal::PromptStart, n("a")]);
        assert!(q.push([n("b")].into_iter()));
        assert!(!q.push(std::iter::empty()));
    }
}
