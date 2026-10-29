//! Saving sessions: the window as a `SessionDoc`, the name prompt's commit, the session
//! list and the config rows of the ≡ menu that touch files.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use winit::dpi::LogicalSize;
use winit::monitor::MonitorHandle;

use super::links::open_detached;
use super::modal::{Modal, PromptPurpose, pane_running, profile_menu_limits, profile_menu_origin};
use super::tabs::Tab;
use super::{PaneStyle, Window};
use crate::layout::{PaneId, Shape, SplitTree};
use crate::session::{
    LeafDoc, SessionDoc, TabDoc, last_file, list_sessions, read_session, session_file, to_json,
    write_atomic,
};
use crate::term::pane::{Pane, next_pane_id};
use crate::ui::menu::{Menu, session_menu_items};
use crate::ui::textfield::TextField;
use crate::ui::toast::{Toast, ToastSource};
use crate::ui::topbar::Target;
use crate::windows::AppRequest;

/// How long after the last layout change the autosave is written.
const AUTOSAVE_DELAY: Duration = Duration::from_secs(1);

const NO_CONFIG_DIR: &str = "no config directory ($HOME unset)";

/// The session document of tabs given as `(title, focused leaf index, shape)`, the active
/// tab's index and the window's logical size.
pub(super) fn session_doc_from(
    tabs: &[(Option<String>, usize, Shape<LeafDoc>)],
    active: usize,
    size: Option<(u32, u32)>,
) -> SessionDoc {
    SessionDoc {
        window: size,
        active_tab: active,
        tabs: tabs
            .iter()
            .map(|(title, focused, root)| TabDoc {
                title: title.clone(),
                focused: *focused,
                root: root.clone(),
            })
            .collect(),
    }
}

/// A leaf as it will be started: a profile that exists, a directory that exists.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct LeafPlan {
    pub stem: String,
    pub cwd: Option<PathBuf>,
    pub command: Option<Vec<String>>,
}

/// A tab as it will be built.
#[derive(Debug, PartialEq)]
pub(super) struct TabPlan {
    pub title: Option<String>,
    pub focused: usize,
    pub root: Shape<LeafPlan>,
}

/// The decisions per leaf of `doc`: a profile that doesn't exist (or an empty stem) becomes
/// `default`, a working directory that isn't an existing directory is dropped, a command
/// is kept. Layout, titles and focus are the document's.
pub(super) fn load_plan(
    doc: &SessionDoc,
    default: &str,
    profile_exists: impl Fn(&str) -> bool,
    dir_exists: impl Fn(&Path) -> bool,
) -> Vec<TabPlan> {
    doc.tabs
        .iter()
        .map(|t| TabPlan {
            title: t.title.clone(),
            focused: t.focused,
            root: t.root.clone().map(&mut |l| LeafPlan {
                stem: if profile_exists(&l.profile) {
                    l.profile
                } else {
                    default.to_string()
                },
                cwd: l.cwd.filter(|d| dir_exists(d)),
                command: l.command,
            }),
        })
        .collect()
}

/// Whether a window's content may be replaced by a session without asking: one tab with
/// one pane that runs nothing (`busy`) and got no input.
pub(super) fn fresh(tabs: usize, panes: usize, busy: bool, got_input: bool) -> bool {
    tabs == 1 && panes == 1 && !busy && !got_input
}

/// What the save prompt's Enter does first: the file (relative to the config dir) for
/// `name`, or why the name is invalid.
pub(super) fn save_commit(name: &str) -> Result<PathBuf, String> {
    session_file(name)
}

/// When the autosave armed by a layout change at `changed_at` is due.
pub(super) fn autosave_after(changed_at: Instant) -> Instant {
    changed_at + AUTOSAVE_DELAY
}

/// Whether a window with `panes` panes is written to `_last.json`.
pub(super) fn should_autosave(enabled: bool, panes: usize) -> bool {
    enabled && panes > 0
}

/// How the first window starts.
#[derive(Debug, PartialEq)]
pub enum StartupInit {
    /// The default pane (or `-e`/`--session`, which the caller handles).
    Default,
    /// The last session.
    Session(SessionDoc),
    /// The default pane, and an error toast saying why the last session wasn't opened.
    DefaultWithError(String),
}

/// The startup decision for restoring the last session: `last` is the result of reading
/// `_last.json` (None when there is no such file). `-e` or `--session` win over it.
pub fn startup_init(
    restore: bool,
    last: Option<Result<SessionDoc, String>>,
    cli_session: bool,
    cli_command: bool,
) -> StartupInit {
    if !restore || cli_session || cli_command {
        return StartupInit::Default;
    }
    match last {
        None => StartupInit::Default,
        Some(Ok(doc)) => StartupInit::Session(doc),
        Some(Err(e)) => StartupInit::DefaultWithError(e),
    }
}

impl Window {
    /// A layout change (split, closed pane, divider, maximize, tab or profile change):
    /// arms the autosave debounce, unless `restore_last_session` is off.
    pub(super) fn layout_changed(&mut self) {
        if self.store.global().restore_last_session {
            self.autosave_due = Some(autosave_after(Instant::now()));
        }
    }

    /// Whether `_last.json` is kept for this window now.
    fn autosave_wanted(&self) -> bool {
        should_autosave(self.store.global().restore_last_session, self.panes.len())
    }

    /// Write the window to `_last.json`; the error says why it wasn't.
    fn write_last(&self) -> Result<(), String> {
        let dir = self.user_config_dir().ok_or(NO_CONFIG_DIR)?;
        let rel = last_file();
        write_atomic(dir, &rel, &to_json(&self.session_doc()))
            .map_err(|e| format!("{}: {e}", rel.display()))
    }

    /// The autosave deadline, while one is armed.
    pub(super) fn autosave_deadline(&self) -> Option<Instant> {
        self.autosave_due
    }

    /// The autosave fires when due: one write, then no deadline is left.
    pub(super) fn autosave_tick(&mut self, now: Instant) {
        if self.autosave_due.is_none_or(|at| now < at) {
            return;
        }
        self.autosave_due = None;
        if self.autosave_wanted()
            && let Err(e) = self.write_last()
        {
            self.error_toast(&e);
        }
    }

    /// The user closed the last window: write it (when the option is on and it still has
    /// panes).
    pub fn save_on_leave(&self) {
        if self.autosave_wanted()
            && let Err(e) = self.write_last()
        {
            eprintln!("montty: {e}");
        }
    }

    /// The config dir; None when montty runs without one (`$HOME` unset).
    fn user_config_dir(&self) -> Option<&Path> {
        let dir = self.store.dir();
        (!dir.as_os_str().is_empty()).then_some(dir)
    }

    fn info_toast(&mut self, text: &str) {
        self.toast = Some(Toast::new(text, ToastSource::Info, Instant::now()));
        self.schedule();
    }

    /// An error toast that waits for a config, startup or spawn toast on screen.
    fn error_toast(&mut self, text: &str) {
        self.show_error(text);
    }

    /// The window as a session: every tab's layout, titles and focus, a leaf per pane
    /// with its profile, working directory and explicit command; the active tab and the
    /// window's size in logical pixels.
    pub(super) fn session_doc(&self) -> SessionDoc {
        let leaf = |id| {
            let pane = self.panes.get(&id);
            LeafDoc {
                profile: self.pane_stem(id).to_string(),
                cwd: pane.and_then(Pane::cwd),
                command: pane
                    .and_then(Pane::explicit_command)
                    .map(<[String]>::to_vec),
            }
        };
        let tabs: Vec<_> = self
            .tabs
            .iter()
            .map(|t| {
                let shape = t.tree.shape();
                let focused = shape
                    .leaves()
                    .iter()
                    .position(|&&id| id == t.focused)
                    .unwrap_or(0);
                (t.title.clone(), focused, shape.map(&mut |id| leaf(id)))
            })
            .collect();
        let size = self.window.inner_size().to_logical::<f64>(self.scale);
        let size = Some((size.width.round() as u32, size.height.round() as u32));
        session_doc_from(&tabs, self.tabs.active_index(), size)
    }

    /// Whether opening a session may replace this window (see `fresh`).
    pub fn is_fresh(&self) -> bool {
        let ids: Vec<PaneId> = self.tabs.iter().flat_map(|t| t.tree.panes()).collect();
        let panes = ids.iter().filter_map(|id| self.panes.get(id));
        let busy = panes
            .clone()
            .any(|p| pane_running(p.runs_command(), || p.has_foreground_child()));
        let typed = panes.clone().any(Pane::got_input);
        fresh(self.tabs.iter().count(), ids.len(), busy, typed)
    }

    /// Open the saved session `name`: in this window when it is fresh, else in a new one.
    /// A name or file that can't be opened shows an error toast and changes nothing.
    pub(super) fn open_session(&mut self, name: &str) {
        let doc = match read_session(self.user_config_dir(), name) {
            Ok(doc) => doc,
            Err(e) => {
                self.error_toast(&e);
                return;
            }
        };
        if self.is_fresh() {
            self.install_session(&doc, Some(name.to_string()));
        } else {
            self.requests.push(AppRequest::OpenSession {
                doc,
                name: name.to_string(),
                profile: self.profile_arg.clone(),
            });
        }
    }

    /// Start one tab of a session: a pane per leaf, as laid out. When a leaf can't start at
    /// all (its fallbacks failed too) the tab becomes one default pane instead; None when
    /// even that can't start.
    fn spawn_tab(&mut self, plan: TabPlan) -> Option<Tab> {
        let area = self.area();
        let mut leaves = Vec::new();
        let shape = plan.root.map(&mut |leaf| {
            let id = next_pane_id();
            leaves.push((id, leaf));
            id
        });
        let mut started = Vec::new();
        let mut all_ok = true;
        for (id, leaf) in &leaves {
            let style = PaneStyle {
                stem: leaf.stem.clone(),
                zoom_pt: 0.0,
            };
            self.pane_profiles.insert(*id, style);
            let Some((pane, size)) =
                self.spawn_pane(*id, area, leaf.cwd.clone(), leaf.command.clone())
            else {
                all_ok = false;
                break;
            };
            self.panes.insert(*id, pane);
            self.sizes.insert(*id, size);
            started.push(*id);
        }
        let (tree, focused) = if all_ok {
            let focused = leaves[plan.focused.min(leaves.len() - 1)].0;
            (SplitTree::from_shape(shape), focused)
        } else {
            for (id, _) in &leaves {
                self.panes.remove(id);
                self.sizes.remove(id);
                self.pane_profiles.remove(id);
            }
            let id = next_pane_id();
            self.pane_profiles.insert(id, self.default_style());
            let Some((pane, size)) = self.spawn_pane(id, area, None, None) else {
                self.pane_profiles.remove(&id);
                return None;
            };
            self.panes.insert(id, pane);
            self.sizes.insert(id, size);
            (SplitTree::new(id), id)
        };
        Some(Tab {
            tree,
            focused,
            title: plan.title,
        })
    }

    /// Replace this window's content with the session `doc` (opened as `name`, if it has
    /// one): every leaf is started with its profile, directory and command, with ratios,
    /// titles, focus and the active tab as saved, and the window's size is asked for
    /// (best effort). The old panes shut down. When nothing can be started the window is
    /// left as it was (the spawn errors are shown) and false is returned.
    pub(super) fn install_session(&mut self, doc: &SessionDoc, name: Option<String>) -> bool {
        let default = self.window_stem().to_string();
        let plans = load_plan(doc, &default, |s| self.store.contains(s), |d| d.is_dir());
        let mut staged: Vec<(usize, Tab)> = Vec::new();
        for (i, plan) in plans.into_iter().enumerate() {
            if let Some(tab) = self.spawn_tab(plan) {
                staged.push((i, tab));
            }
        }
        if staged.is_empty() {
            return false;
        }
        let active = staged
            .iter()
            .position(|(i, _)| *i == doc.active_tab)
            .unwrap_or(0);
        let old: Vec<PaneId> = self.tabs.iter().flat_map(|t| t.tree.panes()).collect();
        let old_tabs = self.tabs.iter().count();
        for (_, tab) in staged {
            self.tabs.push(tab);
        }
        for _ in 0..old_tabs {
            self.tabs.remove(0);
        }
        self.tabs.select(active);
        // The old panes' PTYs shut down as they drop; late events from them find no owner.
        for id in old {
            self.attention.remove(id);
            self.panes.remove(&id);
            self.sizes.remove(&id);
            self.pane_profiles.remove(&id);
            self.scrolled.remove(&id);
        }
        self.evict_fonts();
        self.find = None;
        self.mouse.cancel_drag();
        if let Some(saved) = doc.window {
            // Before the window is mapped, X11 reports a 1×1 placeholder as its monitor.
            let usable = |m: &MonitorHandle| m.size().width >= 100 && m.size().height >= 100;
            let monitor = self
                .window
                .current_monitor()
                .filter(usable)
                .or_else(|| self.window.primary_monitor().filter(usable))
                .or_else(|| self.window.available_monitors().find(usable))
                .map(|m| {
                    let size = m.size().to_logical::<f64>(m.scale_factor());
                    (size.width, size.height)
                });
            let (w, h) = fit_to_monitor(saved, monitor);
            let _ = self.window.request_inner_size(LogicalSize::new(w, h));
            self.quiet_resize_until = Some(Instant::now() + super::sizes::QUIET_RESIZE);
        }
        if name.is_some() {
            self.last_session = name;
        }
        self.refresh_min_extents();
        self.dismiss_stale_modal();
        self.sync_images(false);
        self.show_active();
        self.relayout_all();
        self.layout_changed();
        true
    }

    /// Ctrl+Shift+S or `Save session…`: the name prompt, pre-filled with the name this
    /// window last saved or opened.
    pub(super) fn open_save_prompt(&mut self) {
        let field = TextField::new(self.last_session.as_deref().unwrap_or(""));
        self.set_modal(Modal::Prompt {
            purpose: PromptPurpose::SaveSession,
            field,
        });
    }

    /// Enter in the save prompt with `name`: an invalid name keeps the prompt open and
    /// says why; otherwise the session is written (replacing a file of that name) and
    /// the prompt closes. A write error keeps the prompt open and shows the error.
    pub(super) fn commit_save(&mut self, name: &str) {
        let rel = match save_commit(name) {
            Ok(rel) => rel,
            Err(why) => {
                self.error_toast(&format!("invalid session name: {why}"));
                return;
            }
        };
        let Some(dir) = self.user_config_dir().map(Path::to_path_buf) else {
            self.error_toast(NO_CONFIG_DIR);
            return;
        };
        // The prompt closes only once the file is written: a failure keeps the typed name.
        match write_atomic(&dir, &rel, &to_json(&self.session_doc())) {
            Ok(()) => {
                self.set_modal(Modal::None);
                self.last_session = Some(name.to_string());
                self.info_toast(&format!("saved session '{name}'"));
            }
            Err(e) => self.error_toast(&format!("{}: {e}", rel.display())),
        }
    }

    /// Ctrl+Shift+O or `Open session ▸`: the saved names, sorted, in a menu hanging below
    /// the ≡ button like the main menu.
    pub(super) fn open_session_menu(&mut self) {
        let bar = self.bar();
        let Some(button) = bar.items.iter().find(|i| i.target == Target::Menu) else {
            return;
        };
        let button = button.rect;
        let names = self
            .user_config_dir()
            .map(list_sessions)
            .unwrap_or_default();
        let bm = self.bar_metrics();
        let (w, h) = self.size_px();
        let (max_rows, max_cols) = profile_menu_limits(button, w, h, &bm);
        let items = session_menu_items(&names, max_rows, max_cols);
        let probe = Menu::new(items, 0, 0);
        let r = probe.rect(&bm);
        let (x, y) = profile_menu_origin(button, r.w, r.h, w, h);
        let menu = Menu::new(probe.items, x + r.w, y);
        self.set_modal(Modal::Menu {
            menu,
            profiles: None,
            sessions: Some(names),
            pane: None,
        });
    }

    /// `Edit config`: `config.toml` (created empty when missing) in a terminal editor
    /// (`$VISUAL`, `$EDITOR`, Debian's `editor`) in a new tab; without one, in the desktop's
    /// editor.
    pub(super) fn edit_config(&mut self) {
        let Some(dir) = self.user_config_dir().map(Path::to_path_buf) else {
            self.error_toast(NO_CONFIG_DIR);
            return;
        };
        let file = dir.join("config.toml");
        if !file.exists()
            && let Err(e) = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&file, ""))
        {
            self.error_toast(&format!("{}: {e}", file.display()));
            return;
        }
        let visual = std::env::var("VISUAL").ok();
        let editor = std::env::var("EDITOR").ok();
        let path = std::env::var_os("PATH").unwrap_or_default();
        let candidates = [visual.as_deref(), editor.as_deref(), Some("editor")];
        match editor_command(&candidates, &file, |p| on_path(p, &path)) {
            Some(command) => self.new_tab_with(Some(dir), Some(command)),
            None => open_detached(file.as_os_str()),
        }
    }

    /// `Open config folder`.
    pub(super) fn open_config_folder(&mut self) {
        match self.user_config_dir() {
            Some(dir) => open_detached(OsStr::new(&dir.to_path_buf())),
            None => self.error_toast(NO_CONFIG_DIR),
        }
    }
}

/// The saved window size, or, in each dimension that doesn't fit the monitor (logical
/// pixels), 90 % of the monitor: a session saved on a bigger screen still opens on screen.
pub(super) fn fit_to_monitor(saved: (u32, u32), monitor: Option<(f64, f64)>) -> (u32, u32) {
    let Some((mw, mh)) = monitor else {
        return saved;
    };
    let fit = |v: u32, m: f64| {
        if f64::from(v) > m {
            (m * 0.9) as u32
        } else {
            v
        }
    };
    (fit(saved.0, mw), fit(saved.1, mh))
}

/// The command that edits `file` in a terminal: the first of `candidates` (`$VISUAL`,
/// `$EDITOR`, Debian's `editor`) that is set, non-empty and whose program is `found`. A value
/// may carry arguments (`nvim -p`); it runs through `sh` so it splits like a shell would.
/// None when no candidate qualifies.
pub(super) fn editor_command(
    candidates: &[Option<&str>],
    file: &Path,
    found: impl Fn(&str) -> bool,
) -> Option<Vec<String>> {
    let ed = candidates
        .iter()
        .flatten()
        .map(|e| e.trim())
        .find(|e| e.split_whitespace().next().is_some_and(&found))?;
    Some(vec![
        "sh".into(),
        "-c".into(),
        format!("exec {ed} \"$1\""),
        "sh".into(),
        file.to_string_lossy().into_owned(),
    ])
}

/// Whether `program` is an executable file: a path as given, or a name looked up in `path`
/// (`$PATH`).
fn on_path(program: &str, path: &OsStr) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let executable = |p: &Path| {
        p.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if program.contains('/') {
        return executable(Path::new(program));
    }
    std::env::split_paths(path).any(|dir| executable(&dir.join(program)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Orientation;

    fn leaf(profile: &str, cwd: Option<&str>, command: Option<&[&str]>) -> LeafDoc {
        LeafDoc {
            profile: profile.into(),
            cwd: cwd.map(PathBuf::from),
            command: command.map(|c| c.iter().map(|s| s.to_string()).collect()),
        }
    }

    #[test]
    fn autosave_debounce_is_one_second() {
        let t = Instant::now();
        assert_eq!(autosave_after(t), t + Duration::from_secs(1));
    }

    #[test]
    fn empty_window_not_autosaved() {
        assert!(!should_autosave(true, 0));
        assert!(should_autosave(true, 1));
    }

    #[test]
    fn disabled_never_autosaves() {
        assert!(!should_autosave(false, 3));
    }

    fn doc() -> SessionDoc {
        session_doc_from(
            &[(None, 0, Shape::Leaf(leaf("default", None, None)))],
            0,
            None,
        )
    }

    #[test]
    fn invalid_last_starts_default() {
        let bad = || Some(Err("sessions/_last.json:1: bad".to_string()));
        assert_eq!(
            startup_init(true, bad(), false, false),
            StartupInit::DefaultWithError("sessions/_last.json:1: bad".into())
        );
        assert_eq!(startup_init(true, None, false, false), StartupInit::Default);
        assert_eq!(
            startup_init(true, Some(Ok(doc())), false, false),
            StartupInit::Session(doc())
        );
        // Off, `--session` and `-e` each leave the last session alone.
        assert_eq!(
            startup_init(false, Some(Ok(doc())), false, false),
            StartupInit::Default
        );
        assert_eq!(startup_init(true, bad(), true, false), StartupInit::Default);
        assert_eq!(
            startup_init(true, Some(Ok(doc())), false, true),
            StartupInit::Default
        );
    }

    #[test]
    fn session_doc_records_tabs_focus_and_titles() {
        let split = Shape::Split {
            orientation: Orientation::Horizontal,
            ratio: 0.5,
            a: Box::new(Shape::Leaf(leaf("default", Some("/a"), None))),
            b: Box::new(Shape::Leaf(leaf("dark", None, Some(&["htop"])))),
        };
        let single = Shape::Leaf(leaf("default", None, None));
        let tabs = [
            (Some("build".to_string()), 1, split.clone()),
            (None, 0, single.clone()),
        ];
        let doc = session_doc_from(&tabs, 1, Some((800, 600)));
        assert_eq!(doc.window, Some((800, 600)));
        assert_eq!(doc.active_tab, 1);
        assert_eq!(doc.tabs.len(), 2);
        assert_eq!(doc.tabs[0].title.as_deref(), Some("build"));
        assert_eq!(doc.tabs[0].focused, 1);
        assert_eq!(doc.tabs[0].root, split);
        assert_eq!(doc.tabs[1].title, None);
        assert_eq!(doc.tabs[1].root, single);
        // It survives the file format.
        let back = crate::session::parse(&to_json(&doc), Path::new("x.json")).unwrap();
        assert_eq!(back, doc);
    }

    fn doc_of(leaves: [LeafDoc; 2]) -> SessionDoc {
        let [a, b] = leaves;
        session_doc_from(
            &[(
                Some("t".into()),
                1,
                Shape::Split {
                    orientation: Orientation::Vertical,
                    ratio: 0.3,
                    a: Box::new(Shape::Leaf(a)),
                    b: Box::new(Shape::Leaf(b)),
                },
            )],
            0,
            None,
        )
    }

    #[test]
    fn fresh_rules() {
        assert!(fresh(1, 1, false, false));
        assert!(!fresh(1, 2, false, false));
        assert!(!fresh(2, 1, false, false));
        assert!(!fresh(1, 1, false, true));
        assert!(!fresh(1, 1, true, false));
    }

    #[test]
    fn load_plan_falls_back() {
        let doc = doc_of([
            leaf("gone", Some("/nowhere"), Some(&["htop"])),
            leaf("", Some("/here"), None),
        ]);
        let plan = load_plan(&doc, "def", |s| s == "work", |d| d == Path::new("/here"));
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].title.as_deref(), Some("t"));
        assert_eq!(plan[0].focused, 1);
        let Shape::Split { ratio, .. } = &plan[0].root else {
            panic!("split kept");
        };
        assert_eq!(*ratio, 0.3);
        let leaves = plan[0].root.leaves();
        // Unknown profile: the default; missing cwd: None; the command stays.
        assert_eq!(leaves[0].stem, "def");
        assert_eq!(leaves[0].cwd, None);
        assert_eq!(
            leaves[0].command.as_deref(),
            Some(&["htop".to_string()][..])
        );
        // Empty stem: the default; an existing cwd stays.
        assert_eq!(leaves[1].stem, "def");
        assert_eq!(leaves[1].cwd.as_deref(), Some(Path::new("/here")));
        // A known profile is kept.
        let kept = load_plan(&doc, "def", |_| true, |_| true);
        assert_eq!(kept[0].root.leaves()[0].stem, "gone");
    }

    #[test]
    fn missing_cwd_dropped_on_load() {
        let dir = std::env::temp_dir().join(format!("montty-load-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("f");
        std::fs::write(&file, "").unwrap();
        let doc = doc_of([
            leaf("default", dir.to_str(), None),
            leaf("default", file.to_str(), None),
        ]);
        let plan = load_plan(&doc, "default", |_| true, |d| d.is_dir());
        let leaves = plan[0].root.leaves();
        assert_eq!(leaves[0].cwd.as_deref(), Some(dir.as_path()));
        // A file is not a directory.
        assert_eq!(leaves[1].cwd, None);
        let gone = load_plan(
            &doc_of([
                leaf("default", Some("/montty/no/such/dir"), None),
                leaf("x", None, None),
            ]),
            "default",
            |_| true,
            |d| d.is_dir(),
        );
        assert_eq!(gone[0].root.leaves()[0].cwd, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_name_keeps_prompt() {
        assert_eq!(save_commit("demo"), Ok(PathBuf::from("sessions/demo.json")));
        for bad in ["", "a/b", ".hidden", "_last"] {
            assert!(save_commit(bad).is_err(), "{bad:?}");
        }
        assert!(save_commit("").unwrap_err().contains("empty"));
    }

    #[test]
    fn saved_size_fits_the_monitor() {
        assert_eq!(
            fit_to_monitor((1000, 700), Some((1920.0, 1080.0))),
            (1000, 700)
        );
        assert_eq!(
            fit_to_monitor((2500, 1400), Some((1280.0, 800.0))),
            (1152, 720)
        );
        assert_eq!(
            fit_to_monitor((1000, 1400), Some((1280.0, 800.0))),
            (1000, 720)
        );
        assert_eq!(fit_to_monitor((2500, 1400), None), (2500, 1400));
    }

    #[test]
    fn editor_command_takes_the_first_usable_candidate() {
        let f = Path::new("/c/config.toml");
        let all = |_: &str| true;
        let cmd = editor_command(&[Some("nvim -p"), Some("vi")], f, all).unwrap();
        assert_eq!(
            cmd,
            ["sh", "-c", "exec nvim -p \"$1\"", "sh", "/c/config.toml"]
        );
        let cmd = editor_command(&[Some("  "), None, Some("editor")], f, all).unwrap();
        assert_eq!(cmd[2], "exec editor \"$1\"");
        // An editor that isn't installed is skipped, not run.
        let cmd = editor_command(&[Some("nosuch -x"), Some("vi")], f, |p| p != "nosuch").unwrap();
        assert_eq!(cmd[2], "exec vi \"$1\"");
        assert_eq!(editor_command(&[None, None], f, all), None);
        assert_eq!(editor_command(&[Some("nosuch")], f, |_| false), None);
    }

    #[test]
    fn on_path_finds_executables_only() {
        let dir = std::env::temp_dir().join(format!("montty-onpath-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("ed1");
        std::fs::write(&exe, "").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(dir.join("plain"), "").unwrap();
        let path = dir.clone().into_os_string();
        assert!(on_path("ed1", &path));
        assert!(!on_path("plain", &path));
        assert!(!on_path("missing", &path));
        assert!(on_path(exe.to_str().unwrap(), OsStr::new("")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
