//! winit application handler: owns the windows and routes events to them.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::window::WindowId;

use crate::cli::Args;
use crate::config::shared::{SharedConfig, Startup};
use crate::config::{ConfigError, config_dir};
use crate::events::{EventSink, UserEvent};
use crate::session::{last_file, read_file, read_session};
use crate::term::PaneId;
use crate::window::{StartupInit, Window, WindowConfig, startup_init};
use crate::windows::{AppRequest, WindowInit, Windows};

/// Print every config error: a window's toast shows only the first.
fn log_errors(errors: &[ConfigError]) {
    for e in errors {
        eprintln!("montty: {e}");
    }
}

fn log_watcher(errors: &[String]) {
    for e in errors {
        eprintln!("montty: config watcher: {e}");
    }
}

pub struct App {
    args: Args,
    proxy: EventLoopProxy<UserEvent>,
    windows: Windows<Window>,
    /// The config files, their watcher and the reload debounce, for every window.
    config: SharedConfig,
    /// What loading the config at startup found, until the first window shows it.
    startup: Startup,
    /// A window with background images came or went: the watched dirs follow, once per
    /// turn of the loop (see `about_to_wait`).
    watch_dirty: bool,
    started: bool,
    /// Set when startup failed; the caller reports it and exits 1.
    pub error: Option<String>,
}

impl App {
    /// What the first window opens with: `--session`'s session, else the last session
    /// (see `startup_init`), else the default pane; plus a problem for its error toast
    /// (already printed).
    fn first_init(&self) -> (WindowInit, Option<String>) {
        let profile = self.args.profile.clone();
        let default = |command| WindowInit::Default {
            profile: profile.clone(),
            command,
        };
        let store = self.config.store();
        let dir = Some(store.dir()).filter(|d| !d.as_os_str().is_empty());
        if let Some(name) = &self.args.session {
            return match read_session(dir, name) {
                Ok(doc) => (
                    WindowInit::Session {
                        doc,
                        name: Some(name.clone()),
                        profile: profile.clone(),
                    },
                    None,
                ),
                Err(e) => {
                    // The window starts with the default pane and says why.
                    eprintln!("montty: {e}");
                    (default(None), Some(e))
                }
            };
        }
        // Off when `config.toml` is missing or invalid (the window shows that error itself).
        let restore = store.global().restore_last_session;
        let last = (restore && dir.is_some_and(|d| d.join(last_file()).exists()))
            .then(|| read_file(dir, &last_file()));
        let command = self.args.command.clone();
        match startup_init(restore, last, command.is_some()) {
            StartupInit::Session(doc) => (
                WindowInit::Session {
                    doc,
                    name: None,
                    profile: profile.clone(),
                },
                None,
            ),
            StartupInit::Default => (default(command), None),
            StartupInit::DefaultWithError(e) => {
                eprintln!("montty: {e}");
                (default(command), Some(e))
            }
        }
    }

    /// The window holding `pane`; None once the pane or its window is gone (its late
    /// events are dropped).
    fn pane_owner(&self, pane: PaneId) -> Option<WindowId> {
        self.windows.owner_of(pane, |w, p| w.has_pane(p))
    }

    /// Loads the config (once, for every window) and starts watching it.
    pub fn new(args: Args, proxy: EventLoopProxy<UserEvent>) -> App {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|h| h.is_absolute());
        let waker = proxy.clone();
        let on_change = move || waker.send(UserEvent::ConfigChanged);
        let (config, startup) =
            SharedConfig::load(config_dir().as_deref(), home.as_deref(), on_change);
        log_errors(startup.first_run.as_slice());
        log_errors(config.errors());
        log_watcher(&startup.watcher);
        App {
            args,
            proxy,
            windows: Windows::default(),
            config,
            startup,
            watch_dirty: false,
            started: false,
            error: None,
        }
    }

    /// A new window on the config as it is now; the first one also gets what the load
    /// at startup found.
    fn open(&mut self, el: &ActiveEventLoop, init: WindowInit) -> Result<Window, String> {
        let config = WindowConfig {
            store: self.config.store(),
            errors: self.config.errors().to_vec(),
            startup: std::mem::take(&mut self.startup),
        };
        Window::new(el, init, Arc::new(self.proxy.clone()), config)
    }

    /// Re-read the config dir (the watcher's burst settled, `Reload config`, a new
    /// profile) and hand every window the new store with what changed.
    fn reload_config(&mut self, now: Instant) {
        let (report, news) = self.config.reload();
        if news {
            log_errors(&report.errors);
        }
        let store = self.config.store();
        for (_, w) in self.windows.iter_mut() {
            w.config_reloaded(store.clone(), &report, now);
        }
        // Dirs created since (profiles/, schemes/, a new symlink's target dir) and dirs no
        // longer needed (a removed link's target dir, the config dir's parent).
        self.watch_dirty = true;
    }

    /// Watch the config dirs and every window's background image files. A dir that newly
    /// fails is reported in one window (the focused one, else any).
    fn update_watched(&mut self) {
        let files: Vec<PathBuf> = self
            .windows
            .iter_mut()
            .flat_map(|(_, w)| w.image_files())
            .collect();
        let errs = self.config.watch(&files);
        log_watcher(&errs);
        if !errs.is_empty()
            && let Some(id) = self.windows.pick(|w| w.is_focused())
            && let Some(w) = self.windows.get_mut(id)
        {
            w.watch_failed(&errs);
        }
    }

    /// Run `f` on window `id` (a window that is gone gets nothing), then serve what it
    /// asked for. `f` returns false when the window should close; `user_close` says that
    /// is the user's doing (a close request), so the last window is saved on its way out.
    /// Exits when the last window is gone.
    fn with_window(
        &mut self,
        el: &ActiveEventLoop,
        id: WindowId,
        user_close: bool,
        f: impl FnOnce(&mut Window) -> bool,
    ) {
        let Some(w) = self.windows.get_mut(id) else {
            return;
        };
        let keep = f(w);
        let requests = w.take_requests();
        if !keep {
            // Dropping it makes its GL context current to free its GL objects.
            let w = self.windows.remove(id);
            if let Some(w) = w {
                if user_close && self.windows.is_empty() {
                    w.save_on_leave();
                }
                drop(w);
                // Its images' dirs need no watch anymore.
                self.watch_dirty = true;
            }
        }
        for r in requests {
            self.serve(el, id, r);
        }
        if self.windows.is_empty() {
            el.exit();
        }
    }

    /// Quit: with a process running in any window (and not `confirmed`), window `from` asks
    /// first; else every window closes and the loop exits. `from` is the one saved for
    /// `restore_last_session`, as the last window closed would be.
    pub(crate) fn quit(&mut self, el: &ActiveEventLoop, from: WindowId, confirmed: bool) {
        let running: usize = self
            .windows
            .iter_mut()
            .map(|(_, w)| w.running_count())
            .sum();
        if !confirmed && running > 0 {
            if let Some(w) = self.windows.get_mut(from) {
                w.ask_quit(running);
            }
            return;
        }
        if let Some(w) = self.windows.get_mut(from) {
            w.save_on_leave();
        }
        // Dropping each makes its GL context current to free its GL objects.
        self.windows.clear();
        el.exit();
    }

    /// A request from window `from`. A window that fails to open is reported in `from`
    /// (the process lives on).
    fn serve(&mut self, el: &ActiveEventLoop, from: WindowId, r: AppRequest) {
        let init = match r {
            AppRequest::Quit { confirmed } => return self.quit(el, from, confirmed),
            AppRequest::ReloadConfig => return self.reload_config(Instant::now()),
            AppRequest::NewWindow { profile } => WindowInit::Default {
                profile,
                command: None,
            },
            AppRequest::OpenSession { doc, name, profile } => WindowInit::Session {
                doc,
                name: Some(name),
                profile,
            },
        };
        match self.open(el, init) {
            Ok(w) => self.windows.insert(w.id(), w),
            Err(e) => {
                eprintln!("montty: {e}");
                if let Some(w) = self.windows.get_mut(from) {
                    w.show_error(&e);
                }
            }
        }
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.started {
            return;
        }
        self.started = true;
        let (init, problem) = self.first_init();
        match self.open(el, init) {
            Ok(mut w) => {
                if let Some(e) = problem {
                    w.show_error(&e);
                }
                self.windows.insert(w.id(), w);
            }
            Err(e) => {
                self.error = Some(e);
                el.exit();
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, ev: WindowEvent) {
        self.with_window(el, id, true, |w| w.handle_window_event(ev));
    }

    fn user_event(&mut self, el: &ActiveEventLoop, ev: UserEvent) {
        match ev {
            UserEvent::Term(pane, ev) => {
                if let Some(id) = self.pane_owner(pane) {
                    self.with_window(el, id, false, |w| w.handle_term_event(pane, ev));
                }
            }
            UserEvent::Signals(pane) => {
                if let Some(id) = self.pane_owner(pane) {
                    self.with_window(el, id, false, |w| w.handle_signals(pane));
                }
            }
            UserEvent::ConfigChanged => self.config.poke(Instant::now()),
            UserEvent::ImageLoaded(id, path, n, img) => self.with_window(el, id, false, |w| {
                w.image_loaded(path, n, img);
                true
            }),
            UserEvent::Quit => {
                if let Some(id) = self.windows.pick(|w| w.is_focused()) {
                    self.quit(el, id, false);
                }
            }
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        // Due timers run first, so the only deadline that can be due below is a frame's
        // (one `now` for both: a timer is never seen due without having run).
        let now = Instant::now();
        if self.config.take_due(now) {
            self.reload_config(now);
        }
        let mut watch = std::mem::take(&mut self.watch_dirty);
        for (_, w) in self.windows.iter_mut() {
            watch |= w.take_watch_dirty();
        }
        if watch {
            self.update_watched();
        }
        // The app's own timer (the config reload's debounce), then every window's.
        let mut next: Option<Instant> = self.config.deadline();
        let mut due = false;
        for (_, w) in self.windows.iter_mut() {
            w.on_timers(now);
            match w.next_deadline(now) {
                Some(at) if at <= now => {
                    w.request_redraw();
                    due = true;
                }
                Some(at) => next = Some(next.map_or(at, |n| n.min(at))),
                None => {}
            }
        }
        match next {
            Some(at) if !due => el.set_control_flow(ControlFlow::WaitUntil(at)),
            _ => el.set_control_flow(ControlFlow::Wait),
        }
    }
}
