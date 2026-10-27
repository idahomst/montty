//! Live reload: a directory watcher (inotify through `notify`) and the debounce that turns a
//! burst of file events into one reload.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use notify::event::EventKind;
use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

/// Quiet time after the last event of a burst before the reload.
const SETTLE: Duration = Duration::from_millis(150);
/// The longest a burst can postpone its reload.
const MAX_DELAY: Duration = Duration::from_secs(1);

/// Which events in a watched dir count.
#[derive(Clone, Debug, PartialEq)]
pub enum Scope {
    All,
    /// Only events naming one of these paths (in the dir).
    Only(Vec<PathBuf>),
}

/// The scope of every watched dir, shared with notify's thread.
type Filter = Arc<Mutex<HashMap<PathBuf, Scope>>>;

/// Watches directories, not files: an editor that saves by writing a temp file and renaming
/// it over the original still triggers a change. Dropping it stops watching.
pub struct Watcher {
    inner: RecommendedWatcher,
    filter: Filter,
    /// Dirs from the last `set_watched`.
    set: Vec<PathBuf>,
    /// Dirs whose failure was already reported, until they work again.
    failing: HashSet<PathBuf>,
}

/// Whether an event changes what a reload would read. Opens and closes are not changes;
/// the reload itself opens every file and lists the dirs. A rescan request (inotify's queue
/// overflowed, so events were lost) counts: what changed is unknown.
fn is_change(ev: &notify::Event) -> bool {
    ev.need_rescan()
        || matches!(
            ev.kind,
            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
        )
}

/// The messages (`dir: error`) of the `failed` dirs that weren't failing at the last call,
/// so a dir that keeps failing is reported once. `seen` becomes the dirs failing now.
fn newly_failed(seen: &mut HashSet<PathBuf>, failed: &[(PathBuf, String)]) -> Vec<String> {
    let out = failed
        .iter()
        .filter(|(dir, _)| !seen.contains(dir))
        .map(|(dir, e)| format!("{}: {e}", dir.display()))
        .collect();
    *seen = failed.iter().map(|(dir, _)| dir.clone()).collect();
    out
}

/// Whether an event naming `path` counts: the path is in a dir watched with `All` (or is
/// that dir), or is one of the paths a `Only` dir watches for.
fn in_scope(filter: &HashMap<PathBuf, Scope>, path: &Path) -> bool {
    if filter.get(path) == Some(&Scope::All) {
        return true;
    }
    match path.parent().and_then(|dir| filter.get(dir)) {
        Some(Scope::All) => true,
        Some(Scope::Only(paths)) => paths.iter().any(|p| p == path),
        None => false,
    }
}

/// A notify error without its path list (the caller names the dir).
fn describe(e: &notify::Error) -> String {
    match &e.kind {
        notify::ErrorKind::Generic(s) => s.clone(),
        notify::ErrorKind::Io(io) => io.to_string(),
        notify::ErrorKind::PathNotFound => "no such directory".into(),
        notify::ErrorKind::MaxFilesWatch => "inotify watch limit reached".into(),
        _ => e.to_string(),
    }
}

impl Watcher {
    /// A watcher that calls `on_change`, on notify's thread, for any create, modify, remove
    /// or rename that is in scope in the dirs `set_watched` names (none yet).
    pub fn start(on_change: impl Fn() + Send + 'static) -> Result<Watcher, String> {
        let filter = Filter::default();
        let scopes = filter.clone();
        let handler = move |res: notify::Result<notify::Event>| {
            let Ok(ev) = res else {
                return;
            };
            if !is_change(&ev) {
                return;
            }
            let scopes = scopes.lock().unwrap_or_else(PoisonError::into_inner);
            // An event without a path can't be placed: count it.
            if ev.paths.is_empty() || ev.paths.iter().any(|p| in_scope(&scopes, p)) {
                drop(scopes);
                on_change();
            }
        };
        let inner = notify::recommended_watcher(handler).map_err(|e| describe(&e))?;
        Ok(Watcher {
            inner,
            filter,
            set: Vec::new(),
            failing: HashSet::new(),
        })
    }

    fn scopes(&self) -> MutexGuard<'_, HashMap<PathBuf, Scope>> {
        self.filter.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The OS watch on `dir`. Watching a dir again is harmless (and re-arms a dir that was
    /// deleted and created again).
    fn add(&mut self, dir: &Path) -> Result<(), String> {
        self.inner
            .watch(dir, RecursiveMode::NonRecursive)
            .map_err(|e| describe(&e))
    }

    /// Watch exactly `dirs`, each with its scope; dirs from the previous call that aren't in
    /// `dirs` are unwatched. Returns a message for each dir that couldn't be watched and
    /// wasn't failing at the previous call (one that keeps failing is reported once).
    pub fn set_watched(&mut self, dirs: &[(PathBuf, Scope)]) -> Vec<String> {
        let dropped: Vec<PathBuf> = self
            .set
            .iter()
            .filter(|d| !dirs.iter().any(|(n, _)| n == *d))
            .cloned()
            .collect();
        for d in &dropped {
            // Fails when the OS already dropped it (the dir is gone): nothing to undo.
            let _ = self.inner.unwatch(d);
            self.scopes().remove(d);
        }
        let mut failed = Vec::new();
        self.set.clear();
        for (dir, scope) in dirs {
            // The scope goes in first: an event between the OS watch and the scope would
            // otherwise be dropped.
            let old = self.scopes().insert(dir.clone(), scope.clone());
            if let Err(e) = self.add(dir) {
                match old {
                    Some(old) => self.scopes().insert(dir.clone(), old),
                    None => self.scopes().remove(dir),
                };
                failed.push((dir.clone(), e));
                continue;
            }
            self.set.push(dir.clone());
        }
        newly_failed(&mut self.failing, &failed)
    }
}

/// The dirs to watch for the config dir `dir`, with their scopes:
/// - the dir itself, `profiles/` and `schemes/` where they exist: every event counts;
/// - the real parent dir of each config file that is a symlink to a file elsewhere
///   (dotfile setups): only events naming the link targets count, so a busy dir such as
///   `$HOME` costs nothing;
/// - without the config dir, its parent, for the config dir's own name only: creating
///   the dir is seen, and a reload replaces this with the above.
pub fn config_watch_dirs(dir: &Path) -> Vec<(PathBuf, Scope)> {
    if !dir.is_dir() {
        return dir
            .parent()
            .filter(|p| p.is_dir())
            .map(|p| (p.to_path_buf(), Scope::Only(vec![dir.to_path_buf()])))
            .into_iter()
            .collect();
    }
    let mut out = vec![(dir.to_path_buf(), Scope::All)];
    out.extend(
        ["profiles", "schemes"]
            .into_iter()
            .map(|sub| dir.join(sub))
            .filter(|d| d.is_dir())
            .map(|d| (d, Scope::All)),
    );
    // What the watches above really cover (any of them may be a link itself).
    let covered: Vec<PathBuf> = out
        .iter()
        .filter_map(|(d, _)| d.canonicalize().ok())
        .collect();
    let mut files = vec![dir.join("config.toml")];
    for (sub, _) in &out[1..] {
        if let Ok(rd) = fs::read_dir(sub) {
            files.extend(rd.filter_map(|e| e.ok()).map(|e| e.path()));
        }
    }
    files.sort();
    for f in files {
        let is_link = fs::symlink_metadata(&f).is_ok_and(|m| m.file_type().is_symlink());
        let Some(target) = is_link.then(|| f.canonicalize().ok()).flatten() else {
            continue;
        };
        let Some(parent) = target.parent().filter(|p| !covered.iter().any(|c| c == p)) else {
            continue;
        };
        match out.iter_mut().find(|(d, _)| d == parent) {
            Some((_, Scope::Only(targets))) => {
                if !targets.contains(&target) {
                    targets.push(target);
                }
            }
            Some((_, Scope::All)) => {}
            None => out.push((parent.to_path_buf(), Scope::Only(vec![target]))),
        }
    }
    out
}

/// `dirs` (see `config_watch_dirs`) plus the dir of each of `files` (background images),
/// seeing only events that name that file: a dir watched with `All` already sees it, a dir
/// watched for other files sees it too. A file that is a symlink also gets its target's dir,
/// for the target. Dirs that don't exist are left out.
pub fn with_files(mut dirs: Vec<(PathBuf, Scope)>, files: &[PathBuf]) -> Vec<(PathBuf, Scope)> {
    for f in files {
        let target = f.canonicalize().ok().filter(|t| t != f);
        for file in std::iter::once(f.clone()).chain(target) {
            let Some(parent) = file.parent().filter(|p| p.is_dir()) else {
                continue;
            };
            match dirs.iter_mut().find(|(d, _)| d == parent) {
                Some((_, Scope::All)) => {}
                Some((_, Scope::Only(paths))) => {
                    if !paths.contains(&file) {
                        paths.push(file);
                    }
                }
                None => dirs.push((parent.to_path_buf(), Scope::Only(vec![file]))),
            }
        }
    }
    dirs
}

/// Trailing debounce with a cap: the reload runs 150 ms after the last event of a burst, but
/// no later than 1 s after its first, so a steady stream of events still reloads.
#[derive(Debug, Default)]
pub struct Debounce {
    /// The first and the last event of the pending burst.
    burst: Option<(Instant, Instant)>,
}

impl Debounce {
    pub fn poke(&mut self, now: Instant) {
        let first = self.burst.map_or(now, |(first, _)| first);
        self.burst = Some((first, now));
    }

    /// True once the deadline has passed; the burst is then over.
    pub fn take_due(&mut self, now: Instant) -> bool {
        let due = self.deadline().is_some_and(|at| now >= at);
        if due {
            self.burst = None;
        }
        due
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.burst
            .map(|(first, last)| (last + SETTLE).min(first + MAX_DELAY))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn debounce_coalesces() {
        let t0 = Instant::now();
        let mut d = Debounce::default();
        assert_eq!(d.deadline(), None);
        assert!(!d.take_due(t0));
        d.poke(t0);
        d.poke(t0 + ms(100));
        // Trailing: 150 ms after the last event.
        assert_eq!(d.deadline(), Some(t0 + ms(250)));
        assert!(!d.take_due(t0 + ms(249)));
        assert!(d.take_due(t0 + ms(250)));
        assert_eq!(d.deadline(), None);
        assert!(!d.take_due(t0 + ms(300)));
    }

    #[test]
    fn debounce_is_capped_at_one_second() {
        let t0 = Instant::now();
        let mut d = Debounce::default();
        for i in 0..30 {
            d.poke(t0 + ms(100 * i));
            assert!(d.deadline().unwrap() <= t0 + ms(1000), "poke {i}");
        }
        assert_eq!(d.deadline(), Some(t0 + ms(1000)));
        assert!(d.take_due(t0 + ms(1000)));
        // The next event starts a new burst.
        d.poke(t0 + ms(3000));
        assert_eq!(d.deadline(), Some(t0 + ms(3150)));
    }

    #[test]
    fn rescan_counts_as_a_change() {
        use notify::Event;
        use notify::event::{AccessKind, CreateKind, Flag, ModifyKind};
        let ev = |k| Event::new(k);
        assert!(is_change(&ev(EventKind::Create(CreateKind::File))));
        assert!(is_change(&ev(EventKind::Modify(ModifyKind::Any))));
        assert!(!is_change(&ev(EventKind::Access(AccessKind::Any))));
        assert!(!is_change(&ev(EventKind::Other)));
        // inotify overflowed: events were lost, so reload.
        assert!(is_change(&ev(EventKind::Other).set_flag(Flag::Rescan)));
    }

    #[test]
    fn failing_dirs_are_reported_once() {
        let p = |s: &str| PathBuf::from(s);
        let fail = |dirs: &[&str]| -> Vec<(PathBuf, String)> {
            dirs.iter()
                .map(|d| (p(d), "no such directory".into()))
                .collect()
        };
        let mut seen = HashSet::new();
        // Every failing dir at first (not only one).
        assert_eq!(
            newly_failed(&mut seen, &fail(&["/a", "/b"])),
            ["/a: no such directory", "/b: no such directory"]
        );
        // Still failing: not again. A new one is.
        assert_eq!(
            newly_failed(&mut seen, &fail(&["/a", "/b", "/c"])),
            ["/c: no such directory"]
        );
        assert!(newly_failed(&mut seen, &fail(&["/a", "/b", "/c"])).is_empty());
        // /a recovers, then fails again: reported again.
        assert!(newly_failed(&mut seen, &fail(&["/b", "/c"])).is_empty());
        assert_eq!(
            newly_failed(&mut seen, &fail(&["/a", "/b", "/c"])),
            ["/a: no such directory"]
        );
    }

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    struct Tmp(PathBuf);

    impl Tmp {
        fn new() -> Tmp {
            let p = std::env::temp_dir().join(format!(
                "montty-watch-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn watch_dirs_existing_subdirs() {
        let t = Tmp::new();
        let dir = t.0.join("montty");
        // No config dir yet: its parent, for the dir's own name only.
        assert_eq!(
            config_watch_dirs(&dir),
            vec![(t.0.clone(), Scope::Only(vec![dir.clone()]))]
        );
        // Neither the dir nor its parent: nothing.
        assert!(config_watch_dirs(&dir.join("x").join("y")).is_empty());
        fs::create_dir_all(dir.join("profiles")).unwrap();
        assert_eq!(
            config_watch_dirs(&dir),
            vec![
                (dir.clone(), Scope::All),
                (dir.join("profiles"), Scope::All)
            ]
        );
        fs::create_dir_all(dir.join("schemes")).unwrap();
        assert_eq!(
            config_watch_dirs(&dir),
            vec![
                (dir.clone(), Scope::All),
                (dir.join("profiles"), Scope::All),
                (dir.join("schemes"), Scope::All)
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn watch_dirs_follow_symlinked_files() {
        let t = Tmp::new();
        let dir = t.0.join("montty");
        let dots = t.0.join("dotfiles");
        let other = t.0.join("other");
        fs::create_dir_all(dir.join("profiles")).unwrap();
        fs::create_dir_all(&dots).unwrap();
        fs::create_dir_all(&other).unwrap();
        fs::write(dots.join("config.toml"), "").unwrap();
        fs::write(dots.join("work.toml"), "").unwrap();
        fs::write(other.join("x.json"), "").unwrap();
        fs::write(dir.join("profiles").join("default.toml"), "").unwrap();
        std::os::unix::fs::symlink(dots.join("config.toml"), dir.join("config.toml")).unwrap();
        std::os::unix::fs::symlink(dots.join("work.toml"), dir.join("profiles/work.toml")).unwrap();
        // A link inside the config dir needs no extra watch.
        std::os::unix::fs::symlink(
            dir.join("profiles/default.toml"),
            dir.join("profiles/alias.toml"),
        )
        .unwrap();
        fs::create_dir_all(dir.join("schemes")).unwrap();
        std::os::unix::fs::symlink(other.join("x.json"), dir.join("schemes/x.json")).unwrap();
        // A dangling link has no dir to watch.
        std::os::unix::fs::symlink(t.0.join("gone/y.json"), dir.join("schemes/y.json")).unwrap();
        // Two links into dotfiles/ share one watch that sees only their targets.
        fs::write(dots.join("x.json"), "").unwrap();
        std::os::unix::fs::symlink(dots.join("x.json"), dir.join("schemes/z.json")).unwrap();
        let dots = dots.canonicalize().unwrap();
        let other = other.canonicalize().unwrap();
        assert_eq!(
            config_watch_dirs(&dir),
            vec![
                (dir.clone(), Scope::All),
                (dir.join("profiles"), Scope::All),
                (dir.join("schemes"), Scope::All),
                (
                    dots.clone(),
                    Scope::Only(vec![
                        dots.join("config.toml"),
                        dots.join("work.toml"),
                        dots.join("x.json")
                    ])
                ),
                (other.clone(), Scope::Only(vec![other.join("x.json")])),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn image_files_watched_by_name() {
        let t = Tmp::new();
        let dir = t.0.join("montty");
        let pics = t.0.join("pics");
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(&pics).unwrap();
        fs::write(pics.join("a.png"), "").unwrap();
        let base = config_watch_dirs(&dir);
        assert_eq!(base, vec![(dir.clone(), Scope::All)]);
        // In the config dir: already seen. Two files in pics/ share one watch for both
        // names; one not yet created is watched for too (creating it loads it). A missing
        // dir has nothing to watch.
        let files = [
            dir.join("bg.png"),
            pics.join("a.png"),
            pics.join("b.jpg"),
            t.0.join("gone/c.png"),
        ];
        assert_eq!(
            with_files(base.clone(), &files),
            vec![
                (dir.clone(), Scope::All),
                (
                    pics.clone(),
                    Scope::Only(vec![pics.join("a.png"), pics.join("b.jpg")])
                ),
            ]
        );
        // A symlink: its own dir for the link's name, the target's dir for the target.
        let other = t.0.join("other");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("real.png"), "").unwrap();
        std::os::unix::fs::symlink(other.join("real.png"), pics.join("link.png")).unwrap();
        let other = other.canonicalize().unwrap();
        assert_eq!(
            with_files(base, &[pics.join("link.png")]),
            vec![
                (dir, Scope::All),
                (pics.clone(), Scope::Only(vec![pics.join("link.png")])),
                (other.clone(), Scope::Only(vec![other.join("real.png")])),
            ]
        );
    }
}
