//! A pane: one shell on a PTY plus its terminal grid.

use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::CursorStyle;

use winit::window::WindowId;

use super::PaneId;
use crate::render::image::Rgba8;

pub enum UserEvent {
    Term(PaneId, Event),
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

impl EventSink for Sender<UserEvent> {
    fn send(&self, ev: UserEvent) {
        let _ = Sender::send(self, ev);
    }
}

#[derive(Clone)]
pub struct EventProxy {
    pub pane: PaneId,
    pub sink: Arc<dyn EventSink>,
}

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        self.sink.send(UserEvent::Term(self.pane, event));
    }
}

/// A fresh pane id, unique for the whole process (all windows) and never reused,
/// so late events from a closed pane are ignored. Starts at 1.
pub fn next_pane_id() -> PaneId {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone)]
pub struct PaneOptions {
    /// Program and arguments; `None` (or empty) runs `$SHELL`, else `/bin/sh`.
    pub command: Option<Vec<String>>,
    /// `command` is an explicit command (`-e`, a profile's `command`), not a shell.
    pub runs_command: bool,
    pub working_dir: Option<PathBuf>,
    pub scrollback: usize,
    /// The cursor the terminal starts with and resets to.
    pub cursor: CursorStyle,
    /// X11 window id exported as `WINDOWID`; `None` exports it empty.
    pub window_id: Option<u64>,
}

pub struct Pane {
    pub id: PaneId,
    pub term: Arc<FairMutex<Term<EventProxy>>>,
    pub title: Option<String>,
    sender: EventLoopSender,
    /// A duplicate of the PTY master: stays valid (never a reused fd number)
    /// after the event loop thread drops the `Pty` on child exit.
    master: File,
    /// The shell's pid, which is also its process group and session id.
    shell_pid: u32,
    /// Started with an explicit command (`-e`, a profile's `command`) rather than a shell.
    runs_command: bool,
    /// The explicit command the pane was asked to run (`-e`, a session leaf's `command`),
    /// never a profile's `command`; what a saved session records.
    explicit_command: Option<Vec<String>>,
    /// Whether the user has typed or pasted into the pane (a window with such a pane is
    /// not fresh for opening a session).
    got_input: AtomicBool,
}

/// The terminal's options: how much history it keeps and the cursor it resets to. At spawn
/// and when the pane's profile changes.
fn term_config(scrollback: usize, cursor: CursorStyle) -> Config {
    Config {
        scrolling_history: scrollback,
        default_cursor_style: cursor,
        ..Default::default()
    }
}

fn term_size(size: WindowSize) -> TermSize {
    TermSize::new(
        usize::from(size.num_cols).max(1),
        usize::from(size.num_lines).max(1),
    )
}

impl Pane {
    pub fn spawn(
        id: PaneId,
        opts: &PaneOptions,
        size: WindowSize,
        sink: Arc<dyn EventSink>,
    ) -> io::Result<Pane> {
        let runs_command = opts.runs_command && matches!(opts.command.as_deref(), Some([_, ..]));
        let shell = match opts.command.as_deref() {
            Some([prog, args @ ..]) => tty::Shell::new(prog.clone(), args.to_vec()),
            _ => {
                let sh = std::env::var("SHELL")
                    .ok()
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "/bin/sh".to_string());
                tty::Shell::new(sh, Vec::new())
            }
        };
        let pty_opts = tty::Options {
            shell: Some(shell),
            working_directory: opts.working_dir.clone(),
            drain_on_exit: true,
            // tty::new exports the pane id as WINDOWID and ALACRITTY_WINDOW_ID; these
            // entries are applied after it and override both (an empty value is the
            // closest to unset that tty::Options allows).
            env: [
                ("TERM", "xterm-256color".to_string()),
                ("COLORTERM", "truecolor".to_string()),
                (
                    "WINDOWID",
                    opts.window_id.map(|w| w.to_string()).unwrap_or_default(),
                ),
                ("ALACRITTY_WINDOW_ID", String::new()),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
        };
        let pty = tty::new(&pty_opts, size, id)?;
        let master = pty.file().try_clone()?;
        let shell_pid = pty.child().id();

        let proxy = EventProxy { pane: id, sink };
        let term = Arc::new(FairMutex::new(Term::new(
            term_config(opts.scrollback, opts.cursor),
            &term_size(size),
            proxy.clone(),
        )));

        let event_loop = EventLoop::new(term.clone(), proxy, pty, true, false)?;
        let sender = event_loop.channel();
        event_loop.spawn();

        Ok(Pane {
            id,
            term,
            title: None,
            sender,
            master,
            shell_pid,
            runs_command,
            explicit_command: None,
            got_input: AtomicBool::new(false),
        })
    }

    pub fn write(&self, bytes: Vec<u8>) {
        if !bytes.is_empty() {
            let _ = self.sender.send(Msg::Input(bytes.into()));
        }
    }

    /// Apply a changed profile's scrollback and cursor: a shorter history drops its oldest
    /// lines now; the cursor applies until a program sets its own.
    pub fn set_options(&self, scrollback: usize, cursor: CursorStyle) {
        self.term
            .lock()
            .set_options(term_config(scrollback, cursor));
    }

    pub fn resize(&mut self, size: WindowSize) {
        let _ = self.sender.send(Msg::Resize(size));
        self.term.lock().resize(term_size(size));
    }

    /// Whether a job other than the shell owns the terminal (vim, a build…):
    /// the PTY's foreground process group is not the shell's. One syscall;
    /// false when it fails. A shell without job control (`sh -c`) never
    /// hands the terminal over, so this stays false for it.
    pub fn has_foreground_child(&self) -> bool {
        // SAFETY: tcgetpgrp only reads the terminal's foreground group; the
        // fd is owned by `self.master` and open for this call.
        let pgrp = unsafe { libc::tcgetpgrp(self.master.as_raw_fd()) };
        pgrp > 0 && pgrp as u32 != self.shell_pid
    }

    /// Whether the pane runs an explicit command (`-e`, a profile's
    /// `command`) instead of a shell. That command is the session leader, so
    /// `has_foreground_child` can't see it run; the pane lives as long as it.
    pub fn runs_command(&self) -> bool {
        self.runs_command
    }

    /// The explicit command this pane was started with (see `set_explicit_command`).
    pub fn explicit_command(&self) -> Option<&[String]> {
        self.explicit_command.as_deref()
    }

    /// Record the explicit command (`-e`, a session leaf's `command`) the window started
    /// this pane with; a profile's own `command` is never recorded.
    pub fn set_explicit_command(&mut self, command: Option<Vec<String>>) {
        self.explicit_command = command;
    }

    /// Note that keyboard input or a paste went to this pane.
    pub fn mark_input(&self) {
        self.got_input.store(true, Ordering::Relaxed);
    }

    /// Whether `mark_input` was ever called.
    pub fn got_input(&self) -> bool {
        self.got_input.load(Ordering::Relaxed)
    }

    /// The shell's (or command's) process id.
    pub fn shell_pid(&self) -> u32 {
        self.shell_pid
    }

    /// The shell's working directory, read from `/proc` when asked (never polled). `None`
    /// when the link can't be read or isn't a directory: a deleted one reads as
    /// `"<path> (deleted)"`.
    pub fn cwd(&self) -> Option<PathBuf> {
        let dir = std::fs::read_link(format!("/proc/{}/cwd", self.shell_pid)).ok()?;
        dir.is_dir().then_some(dir)
    }

    pub fn scroll(&self, s: Scroll) {
        self.term.lock().scroll_display(s);
    }
}

impl Drop for Pane {
    fn drop(&mut self) {
        let _ = self.sender.send(Msg::Shutdown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_ids_are_unique_per_process() {
        let threads: Vec<_> = (0..4)
            .map(|_| std::thread::spawn(|| (0..100).map(|_| next_pane_id()).collect::<Vec<_>>()))
            .collect();
        let mut ids: Vec<PaneId> = threads
            .into_iter()
            .flat_map(|t| t.join().unwrap())
            .collect();
        assert!(ids.iter().all(|&id| id != 0));
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 400);
    }
}
