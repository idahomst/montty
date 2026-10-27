//! The panes' background images: one texture per image, shared by every pane whose profile
//! shows it, decoded off the UI thread and dropped once no pane shows it.

use std::collections::VecDeque;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use winit::dpi::PhysicalSize;

use super::{Window, resolved_stem_of, watcher_toast};
use crate::config::watch::{config_watch_dirs, with_files};
use crate::decoder::decode_in_helper;
use crate::render::image::Rgba8;
use crate::term::pane::UserEvent;
use crate::ui::toast::{Toast, ToastSource};

/// Images are downscaled to at most this, or to the largest monitor when that is larger.
const MIN_LIMIT: (u32, u32) = (3840, 2160);

/// A helper process that takes longer than this is killed.
const DECODE_TIMEOUT: Duration = Duration::from_secs(15);
/// At most this many helper processes run at once per window; the rest wait.
const MAX_HELPERS: usize = 2;

/// A file's modification time and size: a reload decodes the image again when either changed.
pub(super) type Stamp = (SystemTime, u64);

/// The size images are downscaled to: 3840×2160, or per axis the largest of `monitors` when
/// larger, within the GL's `max_texture` size.
pub(super) fn image_limit(
    monitors: impl Iterator<Item = PhysicalSize<u32>>,
    max_texture: u32,
) -> (u32, u32) {
    let (w, h) = monitors.fold(MIN_LIMIT, |(w, h), s| (w.max(s.width), h.max(s.height)));
    (w.min(max_texture), h.min(max_texture))
}

/// The stamp of the file at `path`, or why it can't be read.
fn stamp(path: &Path) -> Result<Stamp, String> {
    let m = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if !m.is_file() {
        return Err("not a file".into());
    }
    Ok((m.modified().map_err(|e| e.to_string())?, m.len()))
}

/// One toast for image errors: the first, plus ` (+N more)`.
fn image_toast(errs: &[(PathBuf, String)], now: Instant) -> Option<Toast> {
    let (_, first) = errs.first()?;
    Some(Toast::from_first(
        first,
        errs.len(),
        ToastSource::Image,
        now,
    ))
}

struct Entry<T> {
    /// The profile that names the image (in error messages).
    stem: String,
    /// The file as last seen; None while it is missing.
    stamp: Option<Stamp>,
    /// The file as last decoded (successfully or not): it isn't decoded again until it
    /// changes.
    decoded: Option<Stamp>,
    tex: Option<T>,
    /// The last error reported, with the stem it named: the same one isn't reported again.
    reported: Option<(String, String)>,
}

impl<T> Entry<T> {
    /// The toast text for `err`, unless it was the last one reported for this stem.
    fn report(&mut self, path: &Path, err: &str) -> Option<String> {
        let key = (self.stem.clone(), err.to_string());
        if self.reported.as_ref() == Some(&key) {
            return None;
        }
        self.reported = Some(key);
        Some(format!(
            "profiles/{}.toml: background image {}: {err}",
            self.stem,
            path.display()
        ))
    }
}

/// The decoding slots: at most `MAX_HELPERS` jobs run, the rest wait in order.
pub(super) struct Gate<J> {
    active: usize,
    queue: VecDeque<J>,
}

impl<J> Default for Gate<J> {
    fn default() -> Self {
        Gate {
            active: 0,
            queue: VecDeque::new(),
        }
    }
}

impl<J> Gate<J> {
    /// A new job: `Some(job)` when it may start now, else it waits.
    pub fn submit(&mut self, job: J) -> Option<J> {
        if self.active < MAX_HELPERS {
            self.active += 1;
            return Some(job);
        }
        self.queue.push_back(job);
        None
    }

    /// A running job ended: its slot goes to the job that waited longest, which is returned
    /// to start.
    pub fn finish(&mut self) -> Option<J> {
        match self.queue.pop_front() {
            Some(job) => Some(job),
            None => {
                self.active = self.active.saturating_sub(1);
                None
            }
        }
    }
}

/// A decoding thread: its request id and the file it was started for.
struct Request {
    id: u64,
    stamp: Stamp,
}

/// What `Images::sync` leaves the window to do.
pub(super) struct SyncOut<T> {
    /// Images (by profile path) to decode on a new thread, with the request id.
    pub load: Vec<(PathBuf, u64)>,
    /// Textures no pane shows anymore (or whose file went away): delete them.
    pub dropped: Vec<T>,
    /// Errors not reported yet: (profile path, toast text).
    pub failed: Vec<(PathBuf, String)>,
    /// Paths no pane shows anymore.
    pub gone: Vec<PathBuf>,
}

/// What to do with a decoding thread's result (see `Images::loaded`).
#[derive(Debug, PartialEq)]
pub(super) enum Loaded {
    /// Not the latest request, no pane shows the image anymore, or its file went away.
    Ignore,
    /// The file changed meanwhile: decode it again, as this request.
    Load(u64),
    /// Use it: `succeeded` with its texture, or `failed`.
    Apply,
}

/// The images by their path as the profile names it (`~` expanded, relative to the config
/// dir): one entry per path, generic over the texture so it is testable without GL.
pub(super) struct Images<T> {
    entries: HashMap<PathBuf, Entry<T>>,
    /// The decoding request in flight per path (running or queued); at most one. It outlives
    /// its entry: a path wanted again meanwhile waits for it instead of starting another.
    running: HashMap<PathBuf, Request>,
    last_id: u64,
}

impl<T> Default for Images<T> {
    fn default() -> Self {
        Images {
            entries: HashMap::new(),
            running: HashMap::new(),
            last_id: 0,
        }
    }
}

impl<T> Images<T> {
    /// The texture for `path`, once loaded. Looked up by `&Path`: no allocation per frame.
    pub fn texture(&self, path: &Path) -> Option<&T> {
        self.entries.get(path)?.tex.as_ref()
    }

    /// Keep entries for exactly the images in `wanted` (path → a stem that shows it). A new
    /// image is stamped (`stat`) and loaded; with `recheck` (a reload), every image is
    /// stamped again and loaded again when its file changed. A file that can't be stamped
    /// (missing, not a file) is drawn without, and reported unless it already was.
    pub fn sync(
        &mut self,
        wanted: &BTreeMap<PathBuf, String>,
        recheck: bool,
        mut stat: impl FnMut(&Path) -> Result<Stamp, String>,
    ) -> SyncOut<T> {
        let mut out = SyncOut {
            load: Vec::new(),
            dropped: Vec::new(),
            failed: Vec::new(),
            gone: Vec::new(),
        };
        let mut gone: Vec<PathBuf> = (self.entries.keys())
            .filter(|p| !wanted.contains_key(*p))
            .cloned()
            .collect();
        gone.sort();
        for p in &gone {
            out.dropped
                .extend(self.entries.remove(p).and_then(|e| e.tex));
        }
        out.gone = gone;
        for (path, stem) in wanted {
            let is_new = !self.entries.contains_key(path);
            let e = self.entries.entry(path.clone()).or_insert_with(|| Entry {
                stem: String::new(),
                stamp: None,
                decoded: None,
                tex: None,
                reported: None,
            });
            e.stem.clone_from(stem);
            if !is_new && !recheck {
                continue;
            }
            match stat(path) {
                Ok(s) => e.stamp = Some(s),
                Err(err) => {
                    e.stamp = None;
                    e.decoded = None;
                    out.dropped.extend(e.tex.take());
                    out.failed
                        .extend(e.report(path, &err).map(|t| (path.clone(), t)));
                }
            }
            out.load
                .extend(self.start(path).map(|id| (path.clone(), id)));
        }
        out
    }

    /// Whether request `id` for `path` is still the one to decode: its image is still wanted.
    pub fn is_current(&self, path: &Path, id: u64) -> bool {
        self.entries.contains_key(path) && self.running.get(path).is_some_and(|r| r.id == id)
    }

    /// A new request for `path` when its file isn't decoded yet and no thread runs for it.
    fn start(&mut self, path: &Path) -> Option<u64> {
        let e = self.entries.get(path)?;
        let stamp = e.stamp?;
        if e.decoded == Some(stamp) || self.running.contains_key(path) {
            return None;
        }
        self.last_id += 1;
        let id = self.last_id;
        self.running
            .insert(path.to_path_buf(), Request { id, stamp });
        Some(id)
    }

    /// Request `id` for `path` finished. Its result applies when it is the running request
    /// and the file is still the one it decoded; for a file changed meanwhile, another
    /// request starts.
    pub fn loaded(&mut self, path: &Path, id: u64) -> Loaded {
        if self.running.get(path).is_none_or(|r| r.id != id) {
            return Loaded::Ignore;
        }
        let Some(req) = self.running.remove(path) else {
            return Loaded::Ignore;
        };
        let Some(e) = self.entries.get_mut(path) else {
            return Loaded::Ignore;
        };
        if e.stamp == Some(req.stamp) {
            e.decoded = Some(req.stamp);
            return Loaded::Apply;
        }
        self.start(path).map_or(Loaded::Ignore, Loaded::Load)
    }

    /// `path` loaded as `tex`; returns the texture it replaces.
    pub fn succeeded(&mut self, path: &Path, tex: T) -> Option<T> {
        match self.entries.get_mut(path) {
            Some(e) => {
                e.reported = None;
                e.tex.replace(tex)
            }
            None => Some(tex),
        }
    }

    /// `path` failed to decode or upload: the last good texture stays. The toast text,
    /// unless this error was just reported.
    pub fn failed(&mut self, path: &Path, err: &str) -> Option<String> {
        self.entries.get_mut(path)?.report(path, err)
    }

    /// Every image path, sorted.
    pub fn paths(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = self.entries.keys().cloned().collect();
        v.sort();
        v
    }
}

impl Window {
    /// The images the panes' profiles show (in any tab): path → the first stem showing it.
    fn wanted_images(&self) -> BTreeMap<PathBuf, String> {
        let mut out: BTreeMap<PathBuf, String> = BTreeMap::new();
        for style in self.pane_profiles.values() {
            let Some(img) = &self.store.profile(&style.stem).profile.background_image else {
                continue;
            };
            // An unknown stem shows the default profile's image.
            let stem = resolved_stem_of(
                self.store.contains(&style.stem),
                &style.stem,
                self.store.default_stem(),
            );
            out.entry(img.path.clone())
                .and_modify(|s| {
                    if stem < s.as_str() {
                        *s = stem.to_string();
                    }
                })
                .or_insert_with(|| stem.to_string());
        }
        out
    }

    /// Load the images panes now show and drop those none does. `recheck` (after a reload)
    /// stamps every file again: changed ones are decoded again, missing ones reported.
    pub(super) fn sync_images(&mut self, recheck: bool) {
        let wanted = self.wanted_images();
        let before = self.images.paths();
        let dir = self.store.dir().to_path_buf();
        let out = self.images.sync(&wanted, recheck, |p| stamp(&dir.join(p)));
        let redraw = !out.dropped.is_empty();
        for t in out.dropped {
            self.renderer.delete_image(t);
        }
        for (p, id) in out.load {
            self.load_image(p, id);
        }
        // Errors of images no pane shows anymore don't wait for a toast.
        self.pending_images.retain(|(p, _)| !out.gone.contains(p));
        self.image_errors(out.failed);
        if recheck || self.images.paths() != before {
            self.update_watched();
        }
        if redraw {
            self.schedule();
        }
    }

    /// Watch the config dirs and the images' dirs (for the image files only).
    pub(super) fn update_watched(&mut self) {
        let Some(w) = &mut self.watcher else {
            return;
        };
        let dir = self.store.dir();
        let files: Vec<PathBuf> = self.images.paths().iter().map(|p| dir.join(p)).collect();
        let errs = w.set_watched(&with_files(config_watch_dirs(dir), &files));
        for e in &errs {
            eprintln!("montty: config watcher: {e}");
        }
        // Only dirs that newly fail come back, so a dir that keeps failing doesn't show a
        // toast on every reload. Config errors, startup and spawn problems stay up: the
        // watcher's toast would hide them (the stderr line remains).
        let busy = self.toast.as_ref().is_some_and(|t| {
            !matches!(
                t.source,
                ToastSource::Image | ToastSource::Font | ToastSource::Watcher
            )
        });
        if !busy && let Some(t) = watcher_toast(&errs, Instant::now()) {
            self.toast = Some(t);
            self.schedule();
        }
    }

    /// Decode image `path` (as its profile names it) as request `id`: now, or once one of
    /// the `MAX_HELPERS` helpers is free.
    fn load_image(&mut self, path: PathBuf, id: u64) {
        if let Some(job) = self.decode_gate.submit((path, id)) {
            self.start_helper(job);
        }
    }

    /// Start a helper process for `(path, id)` on a new thread, which posts
    /// `UserEvent::ImageLoaded`.
    fn start_helper(&self, (path, id): (PathBuf, u64)) {
        let file = self.store.dir().join(&path);
        let max = self.image_limit;
        let sink = self.sink.clone();
        let wid = self.window.id();
        let key = path.clone();
        let job = move || {
            // Not `current_exe()`: once the binary is replaced (an upgrade) that path reads
            // "… (deleted)", while /proc/self/exe still executes the running image.
            let exe = Path::new("/proc/self/exe");
            let img = decode_in_helper(exe, &file, max, DECODE_TIMEOUT).map(Arc::new);
            sink.send(UserEvent::ImageLoaded(wid, key, id, img));
        };
        let spawned = std::thread::Builder::new()
            .name("montty image".into())
            .spawn(job);
        if let Err(e) = spawned {
            self.sink.send(UserEvent::ImageLoaded(
                self.window.id(),
                path,
                id,
                Err(e.to_string()),
            ));
        }
    }

    /// A helper finished: start the next queued request still wanted. Those that went
    /// stale while they waited are skipped, so they cost no helper.
    fn helper_done(&mut self) {
        let mut next = self.decode_gate.finish();
        while let Some((path, id)) = next {
            if self.images.is_current(&path, id) {
                return self.start_helper((path, id));
            }
            next = self.decode_gate.finish();
            // Clears the stale request; a file that changed meanwhile is requested again.
            if let Loaded::Load(id) = self.images.loaded(&path, id) {
                self.load_image(path, id);
            }
        }
    }

    /// Request `id` for image `path` finished: upload the image, replacing the old texture.
    /// On failure the toast says why and the last good texture stays (none on a first
    /// load). Stale results are dropped (see `Images::loaded`).
    pub fn image_loaded(&mut self, path: PathBuf, id: u64, img: Result<Arc<Rgba8>, String>) {
        let gl_ok = self.ensure_current();
        self.helper_done();
        if !gl_ok {
            return;
        }
        match self.images.loaded(&path, id) {
            Loaded::Ignore => return,
            Loaded::Load(id) => return self.load_image(path, id),
            Loaded::Apply => {}
        }
        match img.and_then(|img| self.renderer.upload_image(&img)) {
            Ok(tex) => {
                if let Some(old) = self.images.succeeded(&path, tex) {
                    self.renderer.delete_image(old);
                }
                self.pending_images.retain(|(p, _)| *p != path);
                self.schedule();
            }
            Err(e) => {
                let text = self.images.failed(&path, &e);
                self.image_errors(text.map(|t| (path, t)).into_iter().collect());
            }
        }
    }

    /// Show image errors as one toast. While another kind of toast is up, they wait until
    /// that one goes away (see `dismiss_toast`).
    fn image_errors(&mut self, errs: Vec<(PathBuf, String)>) {
        for (_, text) in &errs {
            eprintln!("montty: {text}");
        }
        match &self.toast {
            Some(t) if t.source != ToastSource::Image => {
                for (path, text) in errs {
                    self.pending_images.retain(|(p, _)| *p != path);
                    self.pending_images.push((path, text));
                }
            }
            _ => {
                if let Some(t) = image_toast(&errs, Instant::now()) {
                    self.toast = Some(t);
                    self.schedule();
                }
            }
        }
    }

    /// The image errors that waited for another toast, as one toast.
    pub(super) fn take_pending_image_toast(&mut self) -> Option<Toast> {
        let toast = image_toast(&self.pending_images, Instant::now())?;
        self.pending_images.clear();
        Some(toast)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(secs: u64, len: u64) -> Stamp {
        (SystemTime::UNIX_EPOCH + Duration::from_secs(secs), len)
    }

    fn wanted(paths: &[(&str, &str)]) -> BTreeMap<PathBuf, String> {
        paths
            .iter()
            .map(|&(p, s)| (PathBuf::from(p), s.to_string()))
            .collect()
    }

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    const ENOENT: &str = "No such file or directory (os error 2)";

    fn gone(_: &Path) -> Result<Stamp, String> {
        Err(ENOENT.to_string())
    }

    fn text(stem: &str, path: &str, err: &str) -> String {
        format!("profiles/{stem}.toml: background image {path}: {err}")
    }

    #[test]
    fn loads_once_and_shares() {
        let mut im: Images<u32> = Images::default();
        let w = wanted(&[("a.png", "work"), ("b.png", "home")]);
        let out = im.sync(&w, false, |_| Ok(at(1, 10)));
        assert_eq!(out.load, [(p("a.png"), 1), (p("b.png"), 2)]);
        assert!(out.dropped.is_empty() && out.failed.is_empty());
        // Already loading: nothing new, even with a recheck of an unchanged file.
        assert!(im.sync(&w, false, |_| Ok(at(1, 10))).load.is_empty());
        assert!(im.sync(&w, true, |_| Ok(at(1, 10))).load.is_empty());
        assert_eq!(im.loaded(&p("a.png"), 1), Loaded::Apply);
        assert_eq!(im.succeeded(&p("a.png"), 7), None);
        assert_eq!(im.texture(&p("a.png")), Some(&7));
        assert_eq!(im.texture(&p("b.png")), None, "still loading");
        assert_eq!(im.paths(), [p("a.png"), p("b.png")]);
        // A result of no running request (an older one) is ignored.
        assert_eq!(im.loaded(&p("b.png"), 1), Loaded::Ignore);
        assert_eq!(im.loaded(&p("a.png"), 1), Loaded::Ignore, "already applied");
    }

    #[test]
    fn unused_textures_dropped() {
        let mut im: Images<u32> = Images::default();
        im.sync(&wanted(&[("a.png", "x"), ("b.png", "y")]), false, |_| {
            Ok(at(1, 1))
        });
        im.loaded(&p("a.png"), 1);
        im.succeeded(&p("a.png"), 1);
        // No pane shows a.png or b.png anymore: a's texture goes, b's late result is ignored.
        let out = im.sync(&wanted(&[]), false, |_| unreachable!());
        assert_eq!(out.dropped, [1]);
        assert_eq!(out.gone, [p("a.png"), p("b.png")]);
        assert_eq!(im.loaded(&p("b.png"), 2), Loaded::Ignore);
        assert!(im.paths().is_empty());
    }

    #[test]
    fn reload_decodes_changed_files_only() {
        let mut im: Images<u32> = Images::default();
        let w = wanted(&[("a.png", "x")]);
        im.sync(&w, false, |_| Ok(at(1, 1)));
        im.loaded(&p("a.png"), 1);
        im.succeeded(&p("a.png"), 1);
        // Not a reload: no stat at all.
        assert!(im.sync(&w, false, |_| unreachable!()).load.is_empty());
        // Same mtime and size: nothing; another size or mtime: decoded again, the old
        // texture showing until the new one replaces it.
        assert!(im.sync(&w, true, |_| Ok(at(1, 1))).load.is_empty());
        assert_eq!(im.sync(&w, true, |_| Ok(at(1, 2))).load, [(p("a.png"), 2)]);
        assert_eq!(im.texture(&p("a.png")), Some(&1));
        assert_eq!(im.loaded(&p("a.png"), 2), Loaded::Apply);
        assert_eq!(im.succeeded(&p("a.png"), 2), Some(1));
        assert_eq!(im.sync(&w, true, |_| Ok(at(2, 2))).load, [(p("a.png"), 3)]);
    }

    #[test]
    fn change_while_loading_reloads_once_done() {
        let mut im: Images<u32> = Images::default();
        let w = wanted(&[("a.png", "x")]);
        im.sync(&w, false, |_| Ok(at(1, 1)));
        // Changed while the first thread runs: no second thread now; its result is stale.
        assert!(im.sync(&w, true, |_| Ok(at(2, 1))).load.is_empty());
        assert_eq!(im.loaded(&p("a.png"), 1), Loaded::Load(2));
        assert_eq!(im.loaded(&p("a.png"), 2), Loaded::Apply);
    }

    #[test]
    fn toggling_away_and_back_runs_one_thread() {
        let mut im: Images<u32> = Images::default();
        let w = wanted(&[("a.png", "x")]);
        assert_eq!(im.sync(&w, false, |_| Ok(at(1, 1))).load, [(p("a.png"), 1)]);
        // The pane switches to a profile without it, and back, while it loads: the running
        // thread's result is used, no second one starts.
        im.sync(&wanted(&[]), false, |_| unreachable!());
        assert!(im.sync(&w, false, |_| Ok(at(1, 1))).load.is_empty());
        assert_eq!(im.loaded(&p("a.png"), 1), Loaded::Apply);
        im.succeeded(&p("a.png"), 5);
        assert_eq!(im.texture(&p("a.png")), Some(&5));
        // Back to a file changed meanwhile: the running result is dropped and the new file
        // loads once it is done.
        assert_eq!(im.sync(&w, true, |_| Ok(at(2, 1))).load, [(p("a.png"), 2)]);
        im.sync(&wanted(&[]), false, |_| unreachable!());
        assert!(im.sync(&w, false, |_| Ok(at(3, 1))).load.is_empty());
        assert_eq!(im.loaded(&p("a.png"), 2), Loaded::Load(3));
        assert_eq!(im.loaded(&p("a.png"), 3), Loaded::Apply);
    }

    #[test]
    fn failed_redecode_keeps_last_good() {
        let mut im: Images<u32> = Images::default();
        let w = wanted(&[("a.png", "x")]);
        // A first load that fails: no image, one report.
        im.sync(&w, false, |_| Ok(at(1, 1)));
        assert_eq!(im.loaded(&p("a.png"), 1), Loaded::Apply);
        assert_eq!(
            im.failed(&p("a.png"), "bad PNG"),
            Some(text("x", "a.png", "bad PNG"))
        );
        assert_eq!(im.texture(&p("a.png")), None);
        // Fixed: loads.
        im.sync(&w, true, |_| Ok(at(2, 1)));
        im.loaded(&p("a.png"), 2);
        im.succeeded(&p("a.png"), 1);
        // Saved broken (half written): reported, the last good texture stays.
        assert_eq!(im.sync(&w, true, |_| Ok(at(3, 1))).load, [(p("a.png"), 3)]);
        assert_eq!(im.loaded(&p("a.png"), 3), Loaded::Apply);
        assert!(im.failed(&p("a.png"), "bad PNG").is_some());
        assert_eq!(im.texture(&p("a.png")), Some(&1));
        // The same broken file isn't decoded again on an unrelated reload.
        assert!(im.sync(&w, true, |_| Ok(at(3, 1))).load.is_empty());
        // Missing (or not a file): the texture does go.
        let out = im.sync(&w, true, gone);
        assert_eq!(out.dropped, [1]);
        assert_eq!(im.texture(&p("a.png")), None);
    }

    #[test]
    fn errors_reported_once_until_they_change() {
        let mut im: Images<u32> = Images::default();
        let w = wanted(&[("a.png", "x")]);
        let out = im.sync(&w, false, gone);
        assert!(out.load.is_empty());
        assert_eq!(out.failed, [(p("a.png"), text("x", "a.png", ENOENT))]);
        // Unrelated reloads don't report it again.
        assert!(im.sync(&w, false, gone).failed.is_empty());
        assert!(im.sync(&w, true, gone).failed.is_empty());
        // Another error, or another profile naming it, is reported.
        let denied = |_: &Path| Err::<Stamp, _>("Permission denied (os error 13)".to_string());
        assert_eq!(im.sync(&w, true, denied).failed.len(), 1);
        let y = wanted(&[("a.png", "y")]);
        assert_eq!(
            im.sync(&y, true, denied).failed,
            [(
                p("a.png"),
                text("y", "a.png", "Permission denied (os error 13)")
            )]
        );
        // Created: loaded; a later failure after a success is reported again.
        assert_eq!(im.sync(&y, true, |_| Ok(at(1, 1))).load, [(p("a.png"), 1)]);
        im.loaded(&p("a.png"), 1);
        im.succeeded(&p("a.png"), 3);
        let out = im.sync(&y, true, denied);
        assert_eq!((out.dropped, out.failed.len()), (vec![3], 1));
        // Deleted while loading: the result is dropped, nothing reloads.
        im.sync(&y, true, |_| Ok(at(2, 1)));
        im.sync(&y, true, gone);
        assert_eq!(im.loaded(&p("a.png"), 2), Loaded::Ignore);
        assert_eq!(im.sync(&y, true, |_| Ok(at(3, 1))).load, [(p("a.png"), 3)]);
    }

    #[test]
    fn several_errors_make_one_toast() {
        let errs = |n: usize| -> Vec<(PathBuf, String)> {
            (0..n)
                .map(|i| (p(&format!("{i}.png")), format!("e{i}")))
                .collect()
        };
        let now = Instant::now();
        assert_eq!(image_toast(&errs(0), now), None);
        assert_eq!(image_toast(&errs(1), now).unwrap().full(), "e0");
        assert_eq!(image_toast(&errs(3), now).unwrap().full(), "e0 (+2 more)");
    }

    #[test]
    fn gate_runs_two_and_queues_the_rest() {
        let mut g: Gate<u32> = Gate::default();
        assert_eq!(g.submit(1), Some(1));
        assert_eq!(g.submit(2), Some(2));
        assert_eq!(g.submit(3), None);
        assert_eq!(g.submit(4), None);
        // A finished helper hands its slot to the oldest waiting request.
        assert_eq!(g.finish(), Some(3));
        assert_eq!(g.finish(), Some(4));
        assert_eq!(g.finish(), None);
        // Two slots are free again... one finished, so one is.
        assert_eq!(g.submit(5), Some(5));
        assert_eq!(g.submit(6), None);
        assert_eq!(g.finish(), Some(6));
        assert_eq!(g.finish(), None);
        assert_eq!(g.finish(), None);
        assert_eq!(g.submit(7), Some(7));
        assert_eq!(g.submit(8), Some(8));
        assert_eq!(g.submit(9), None);
    }

    #[test]
    fn stale_queued_requests_are_not_current() {
        let mut im: Images<u32> = Images::default();
        let a = PathBuf::from("a.png");
        let b = PathBuf::from("b.png");
        let out = im.sync(&wanted(&[("a.png", "x"), ("b.png", "x")]), false, |_| {
            Ok(at(1, 1))
        });
        let ids: Vec<u64> = out.load.iter().map(|(_, id)| *id).collect();
        assert!(im.is_current(&a, ids[0]) && im.is_current(&b, ids[1]));
        assert!(!im.is_current(&a, ids[1]), "wrong id");
        // b goes unwanted while it waits: not current, but its request still clears.
        im.sync(&wanted(&[("a.png", "x")]), false, |_| Ok(at(1, 1)));
        assert!(!im.is_current(&b, ids[1]));
        assert_eq!(im.loaded(&b, ids[1]), Loaded::Ignore);
        // b wanted again afterwards starts a new request.
        let out = im.sync(&wanted(&[("a.png", "x"), ("b.png", "x")]), false, |_| {
            Ok(at(1, 1))
        });
        assert_eq!(out.load.len(), 1);
        assert!(im.is_current(&b, out.load[0].1));
    }

    #[test]
    fn limit_is_4k_or_largest_monitor() {
        let s = |w, h| PhysicalSize::new(w, h);
        assert_eq!(image_limit(std::iter::empty(), 16384), (3840, 2160));
        assert_eq!(
            image_limit([s(1920, 1080)].into_iter(), 16384),
            (3840, 2160)
        );
        // Per axis: a 5K monitor and a portrait one.
        assert_eq!(
            image_limit([s(5120, 2880), s(2160, 3840)].into_iter(), 16384),
            (5120, 3840)
        );
        // Within the GL's texture size.
        assert_eq!(image_limit([s(5120, 2880)].into_iter(), 4096), (4096, 2880));
    }
}
