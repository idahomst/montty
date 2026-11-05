//! Events posted to the UI thread's event loop: from the PTY reader threads, the config
//! watcher and the image decoders, each for a pane or a window.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Sender;

use alacritty_terminal::event::Event;
use winit::window::WindowId;

use crate::render::image::Rgba8;
use crate::term::PaneId;
use crate::term::osc_scan::Signal;

pub enum UserEvent {
    Term(PaneId, Event),
    /// A status or notification sequence the PTY tap found in the pane's output.
    Signal(PaneId, Signal),
    /// A file in the config dir changed (sent from the watcher's thread), for the window
    /// whose watcher saw it.
    ConfigChanged(WindowId),
    /// A background image finished loading (sent from its decoding thread) for a window:
    /// the image's path as its profile names it, the request id, and the image, downscaled, or why it
    /// failed.
    ImageLoaded(WindowId, PathBuf, u64, Result<Arc<Rgba8>, String>),
}

pub trait EventSink: Send + Sync + 'static {
    fn send(&self, ev: UserEvent);
}

impl EventSink for winit::event_loop::EventLoopProxy<UserEvent> {
    fn send(&self, ev: UserEvent) {
        let _ = self.send_event(ev);
    }
}

/// The integration tests (`tests/`) collect a pane's events through a channel.
impl EventSink for Sender<UserEvent> {
    fn send(&self, ev: UserEvent) {
        let _ = Sender::send(self, ev);
    }
}
