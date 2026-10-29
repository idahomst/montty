//! A PTY wrapper that lets the scanner observe the byte stream on its way to the parser.

use std::fs::File;
use std::io::{self, Read};
use std::sync::Arc;

use alacritty_terminal::event::{OnResize, WindowSize};
use alacritty_terminal::tty::{ChildEvent, EventedPty, EventedReadWrite, Pty};
use polling::{Event, PollMode, Poller};

use super::PaneId;
use super::osc_scan::{OscScanner, Signal};
use super::pane::{EventSink, UserEvent};

/// Reads from `R` unchanged; every successful read is also fed to the scanner and each
/// signal found is sent to the UI as a `UserEvent::Signal`.
pub struct TapReader<R: Read> {
    inner: R,
    scanner: OscScanner,
    pane: PaneId,
    sink: Arc<dyn EventSink>,
    /// Reused across reads so a read without signals allocates nothing.
    signals: Vec<Signal>,
}

impl<R: Read> TapReader<R> {
    pub fn new(inner: R, pane: PaneId, sink: Arc<dyn EventSink>) -> Self {
        TapReader {
            inner,
            scanner: OscScanner::new(),
            pane,
            sink,
            signals: Vec::new(),
        }
    }
}

impl<R: Read> Read for TapReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.signals.clear();
        self.scanner.feed(&buf[..n], &mut self.signals);
        for s in self.signals.drain(..) {
            self.sink.send(UserEvent::Signal(self.pane, s));
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
    pub fn new(inner: Pty, pane: PaneId, sink: Arc<dyn EventSink>) -> io::Result<TapPty> {
        // A dup shares the open file description, so the non-blocking flag and the poll
        // registration made on the original fd apply to it.
        let reader = TapReader::new(inner.file().try_clone()?, pane, sink);
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

    #[test]
    fn reader_passes_bytes_and_reports_signals() {
        let input = b"hi\x1b]7501;?\x07there";
        let (tx, rx) = channel::<UserEvent>();
        let sink: Arc<dyn EventSink> = Arc::new(tx);
        let mut r = TapReader::new(Cursor::new(input.to_vec()), 7, sink);
        let mut got = Vec::new();
        let mut buf = [0u8; 5];
        loop {
            let n = r.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            got.extend_from_slice(&buf[..n]);
        }
        assert_eq!(got, input);
        let evs: Vec<_> = rx.try_iter().collect();
        assert_eq!(evs.len(), 1);
        assert!(matches!(&evs[0], UserEvent::Signal(7, Signal::StatusQuery)));
    }
}
