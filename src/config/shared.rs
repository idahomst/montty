//! The process's one config: the store every window reads, the watcher on its files and
//! the debounce that turns a burst of file events into one reload. Owned by the app, so N
//! windows cost one reload and one watcher thread, and all of them agree.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use super::ConfigError;
use super::store::{ReloadReport, Store};
use super::watch::{Debounce, Watcher, config_watch_dirs, with_files};

/// What the load at startup found; the first window shows it.
#[derive(Debug, Default, PartialEq)]
pub struct Startup {
    /// The first-run write of `profiles/default.toml` failed. A reload can't fix it.
    pub first_run: Option<ConfigError>,
    /// What couldn't be watched (no watcher at all, or each dir): edits there need a restart.
    pub watcher: Vec<String>,
}

pub struct SharedConfig {
    /// Windows hold clones: a snapshot that a reload replaces (see `reload`).
    store: Rc<Store>,
    /// None without a config dir or when watching failed.
    watcher: Option<Watcher>,
    debounce: Debounce,
    /// The errors of the last (re)load: a new window shows them, as a reload would.
    errors: Vec<ConfigError>,
}

impl SharedConfig {
    /// Load the config dir `dir` (None: built-in defaults, nothing read or watched) and
    /// watch it. `on_change` runs on the watcher's thread for each change; the app answers
    /// with `poke`. Never fails: problems are in `errors` and the returned `Startup`.
    pub fn load(
        dir: Option<&Path>,
        home: Option<&Path>,
        on_change: impl Fn() + Send + 'static,
    ) -> (SharedConfig, Startup) {
        let Some(dir) = dir else {
            let config = SharedConfig {
                store: Rc::new(Store::builtin()),
                watcher: None,
                debounce: Debounce::default(),
                errors: Vec::new(),
            };
            return (config, Startup::default());
        };
        let (store, first_run, errors) = Store::load_split(dir, home);
        let (watcher, watcher_errors) = match Watcher::start(on_change) {
            Ok(mut w) => {
                let errs = w.set_watched(&config_watch_dirs(dir));
                (Some(w), errs)
            }
            Err(e) => (None, vec![e]),
        };
        let config = SharedConfig {
            store: Rc::new(store),
            watcher,
            debounce: Debounce::default(),
            errors,
        };
        let startup = Startup {
            first_run,
            watcher: watcher_errors,
        };
        (config, startup)
    }

    /// The store as it is now. Cheap; a window keeps it until the next reload hands it
    /// the new one.
    pub fn store(&self) -> Rc<Store> {
        self.store.clone()
    }

    /// The errors of the last (re)load.
    pub fn errors(&self) -> &[ConfigError] {
        &self.errors
    }

    /// The watcher saw a change: reload once the burst settles.
    pub fn poke(&mut self, now: Instant) {
        self.debounce.poke(now);
    }

    /// When the pending reload is due.
    pub fn deadline(&self) -> Option<Instant> {
        self.debounce.deadline()
    }

    /// True once the pending reload is due; the caller then calls `reload`.
    pub fn take_due(&mut self, now: Instant) -> bool {
        self.debounce.take_due(now)
    }

    /// Re-read the config dir. Windows still hold the store as it was, so the new one is a
    /// copy (profiles are shared, not copied); the caller hands `store()` to each of them
    /// with the report. The bool says whether its errors are news: they differ from the
    /// last load's, or something changed or was saved (see `ui::toast::reload_toast`).
    pub fn reload(&mut self) -> (ReloadReport, bool) {
        let report = Rc::make_mut(&mut self.store).reload();
        let news = report.errors != self.errors || report.edited || report.changed_anything();
        self.errors.clone_from(&report.errors);
        (report, news)
    }

    /// Watch the config dirs and the dirs of `images` (every window's background image
    /// files, for those files only). Returns a message per dir that newly fails.
    pub fn watch(&mut self, images: &[PathBuf]) -> Vec<String> {
        let Some(w) = &mut self.watcher else {
            return Vec::new();
        };
        w.set_watched(&with_files(config_watch_dirs(self.store.dir()), images))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    #[test]
    fn without_a_dir_nothing_is_read_or_watched() {
        let (mut c, startup) = SharedConfig::load(None, None, || {});
        assert_eq!(startup, Startup::default());
        assert!(c.errors().is_empty());
        assert!(c.store().dir().as_os_str().is_empty());
        assert!(c.watch(&[PathBuf::from("/nowhere/a.png")]).is_empty());
        let (report, news) = c.reload();
        assert_eq!(report, ReloadReport::default());
        assert!(!news);
    }

    #[test]
    fn a_reload_leaves_the_windows_store_as_it_was() {
        let t = TempDir::new("shared");
        t.write("profiles/default.toml", "font_size = 11\n");
        let (mut c, startup) = SharedConfig::load(Some(&t.0), None, || {});
        assert_eq!(startup.first_run, None);
        let held = c.store();
        assert_eq!(held.profile("default").profile.font_size, 11.0);

        t.write("profiles/default.toml", "font_size = 20\n");
        let (report, news) = c.reload();
        assert_eq!(report.changed, ["default"]);
        assert!(news);
        // The snapshot a window holds is untouched until it takes the new one.
        assert_eq!(held.profile("default").profile.font_size, 11.0);
        assert_eq!(c.store().profile("default").profile.font_size, 20.0);
        assert_eq!(
            c.store().previous_profile("default").profile.font_size,
            11.0
        );

        // Nothing changed: no news.
        let (_, news) = c.reload();
        assert!(!news);
    }

    #[test]
    fn errors_are_kept_for_new_windows_and_news_once() {
        let t = TempDir::new("shared-err");
        t.write("profiles/default.toml", "");
        t.write("config.toml", "nope = 1\n");
        let (mut c, _) = SharedConfig::load(Some(&t.0), None, || {});
        assert_eq!(c.errors().len(), 1);
        // The same broken file, unsaved: not news.
        let (report, news) = c.reload();
        assert_eq!(report.errors.len(), 1);
        assert!(!news);
        t.write("config.toml", "");
        let (_, news) = c.reload();
        assert!(news);
        assert!(c.errors().is_empty());
    }
}
