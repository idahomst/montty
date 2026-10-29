//! One OS window: GL surface, panes, input and frame scheduling.

mod actions;
mod bar;
mod blink;
mod draw;
pub mod find;
mod gl;
mod icon;
mod images;
mod links;
mod modal;
mod mouse;
mod pane_drag;
mod selection;
mod sessions;
mod sizes;
pub use sessions::{StartupInit, startup_init};
mod tabs;

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::mem::ManuallyDrop;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, WindowSize};
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::term::ClipboardType;
use alacritty_terminal::vte::ansi::{Color, CursorShape, CursorStyle, NamedColor};
use glutin::context::{PossiblyCurrentContext, PossiblyCurrentGlContext};
use glutin::surface::{GlSurface, Surface, WindowSurface};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawWindowHandle};
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::window::{CursorIcon, WindowId};

use self::blink::Blink;
use crate::clipboard::{ClipKind, Clipboards, MemClipboards, system_clipboards};
use crate::config::profile::{CursorShapeCfg, Profile};
use crate::config::store::{ReloadReport, Resolved, Store};
use crate::config::watch::{Debounce, Watcher, config_watch_dirs};
use crate::config::{ConfigError, config_dir};
use crate::font::{FontCache, Metrics, system_db};
use crate::frame::{FrameScheduler, Schedule};
use crate::input::Mods;
use crate::input::keys::{KeyInput, encode_key};
use crate::layout::{PaneId, Rect};
use crate::render::Renderer;
use crate::render::image::ImageTexture;
use crate::term::colors::resolve;
use crate::term::osc_scan::Signal;
use crate::term::pane::{EventSink, Pane, PaneOptions, UserEvent, next_pane_id};
use crate::term::snapshot::Snapshot;
use crate::ui::confirm::CloseTarget;
use crate::ui::overlay::OverlayTarget;
use crate::ui::toast::{Toast, ToastSource, ToastUpdate, reload_toast};
use crate::ui::topbar::{BarMetrics, Target};
use crate::windows::{AppRequest, WindowInit};
use sizes::{QUIET_RESIZE, SizeLabels, visible_resized, window_resize_labels};
use tabs::{Tab, Tabs};

const ATLAS_SIZE: u32 = 2048;
/// A pane's font size (profile size plus zoom) stays within these points.
const MIN_PT: f32 = 4.0;
const MAX_PT: f32 = 200.0;
const TITLE: &str = "montty";
const NO_CONFIG_DIR: &str = "no config directory ($HOME unset): using defaults";

/// Terminal grid (cols, lines) that fits in `rect` after `padding` on every side; at least 1x1.
pub fn grid_size(rect: Rect, m: &Metrics, padding: i32) -> (u16, u16) {
    let fit = |extent: i32, cell: u32| {
        let n = (extent - 2 * padding).max(0) / (cell.max(1) as i32);
        n.clamp(1, i32::from(u16::MAX)) as u16
    };
    (fit(rect.w, m.cell_width), fit(rect.h, m.cell_height))
}

/// Smallest pane (width, height) that still shows one whole cell inside `padding`.
pub fn pane_min_extent(m: &Metrics, padding: i32) -> (i32, i32) {
    let px = |cell: u32| (cell.min(i32::MAX as u32 / 2) as i32).saturating_add(2 * padding);
    (px(m.cell_width), px(m.cell_height))
}

/// A tab's minimum pane extent: the largest `pane_min_extent` of its panes on each axis
/// (one tree minimum must fit every pane's cell). No panes: (0, 0).
pub(super) fn tab_min_extent(extents: impl Iterator<Item = (i32, i32)>) -> (i32, i32) {
    extents.fold((0, 0), |(w, h), (pw, ph)| (w.max(pw), h.max(ph)))
}

/// A pane's profile (by stem) and its own zoom.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct PaneStyle {
    pub stem: String,
    /// Points added to the profile's font size (zoom keys on this pane).
    pub zoom_pt: f32,
}

/// A pane's font as resolved now: the set's key in the `FontCache`, its cells and the
/// padding in px.
#[derive(Clone, Copy, Debug)]
pub(super) struct PaneFont {
    pub key: u32,
    pub metrics: Metrics,
    pub padding: i32,
}

/// The font keys in use: the UI font and each pane's.
fn live_keys(ui: u32, panes: impl Iterator<Item = u32>) -> HashSet<u32> {
    panes.chain(std::iter::once(ui)).collect()
}

/// A profile's font size plus a pane's zoom, within `MIN_PT..=MAX_PT`.
pub(super) fn effective_size(base_pt: f32, zoom_pt: f32) -> f32 {
    (base_pt + zoom_pt).clamp(MIN_PT, MAX_PT)
}

/// The zoom after a zoom key: `step` points more (kept where the size is still in range),
/// or 0 for a reset (`None`).
pub(super) fn next_zoom(base_pt: f32, zoom_pt: f32, step: Option<f32>) -> f32 {
    match step {
        Some(d) => (zoom_pt + d).clamp(MIN_PT - base_pt, MAX_PT - base_pt),
        None => 0.0,
    }
}

/// The set for `family` at `size_pt` and `scale` in `cache`, else `monospace` at that size
/// together with the first error. Err only when monospace can't be loaded either.
pub(super) fn load_font_or_fallback(
    cache: &mut FontCache,
    family: &str,
    size_pt: f32,
    scale: f64,
) -> Result<(u32, Option<Arc<str>>), Arc<str>> {
    match cache.get(family, size_pt, scale) {
        Ok(set) => Ok((set.key(), None)),
        Err(e) => cache
            .get("monospace", size_pt, scale)
            .map(|set| (set.key(), Some(e))),
    }
}

/// The font of a pane under profile `stem` zoomed by `zoom_pt`, at `scale`: the profile's
/// family at its size plus the zoom (see `load_font_or_fallback`), and its padding scaled.
fn resolve_font(
    fonts: &mut FontCache,
    store: &Store,
    stem: &str,
    zoom_pt: f32,
    scale: f64,
) -> Result<(PaneFont, Option<Arc<str>>), Arc<str>> {
    let r = store.profile(stem);
    let p = &r.profile;
    let size = effective_size(p.font_size, zoom_pt);
    let (key, err) = load_font_or_fallback(fonts, &p.font_family, size, scale)?;
    let metrics = fonts
        .by_key(key)
        .ok_or("font set not in the cache")?
        .metrics();
    let padding = (f64::from(p.padding) * scale).round() as i32;
    Ok((
        PaneFont {
            key,
            metrics,
            padding,
        },
        err,
    ))
}

/// The missing fonts a toast reported: profile stem → families. Looked up by `&str`, so a
/// pane drawn with a missing family allocates nothing per frame.
#[derive(Debug, Default)]
pub(super) struct ReportedFonts(HashMap<String, HashSet<String>>);

impl ReportedFonts {
    pub fn contains(&self, stem: &str, family: &str) -> bool {
        self.0.get(stem).is_some_and(|f| f.contains(family))
    }

    pub fn insert(&mut self, stem: &str, family: &str) {
        self.0
            .entry(stem.to_string())
            .or_default()
            .insert(family.to_string());
    }
}

/// The panes whose profile may resolve differently after reload `r`: their stem changed,
/// went away (they fall back to the default; the stem stays recorded) or came back, or it
/// resolves to the default (the default's own stem, or one `known` doesn't have) while the
/// default moved: another stem (`default_changed`; `default_stem` is the new one) or its
/// file changed. Sorted.
pub(super) fn affected_panes(
    styles: &HashMap<PaneId, PaneStyle>,
    r: &ReloadReport,
    default_changed: bool,
    default_stem: &str,
    known: impl Fn(&str) -> bool,
) -> Vec<PaneId> {
    let listed = |stem: &str| {
        [&r.changed, &r.removed, &r.added]
            .iter()
            .any(|v| v.iter().any(|s| s == stem))
    };
    let default_moved = default_changed || listed(default_stem);
    let mut ids: Vec<PaneId> = styles
        .iter()
        .filter(|(_, st)| {
            let stem = st.stem.as_str();
            listed(stem) || (default_moved && (stem == default_stem || !known(stem)))
        })
        .map(|(&id, _)| id)
        .collect();
    ids.sort_unstable();
    ids
}

/// What a pane redoes when its profile resolves differently: draw again (colors), load
/// another font (family, size, padding: other cells, so min extents, layout and PTY sizes)
/// or update its terminal (scrollback, cursor). The command and shell only apply to new
/// panes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct ProfileDelta {
    pub redraw: bool,
    pub font: bool,
    pub term: bool,
}

impl ProfileDelta {
    fn or(self, o: ProfileDelta) -> ProfileDelta {
        ProfileDelta {
            redraw: self.redraw || o.redraw,
            font: self.font || o.font,
            term: self.term || o.term,
        }
    }
}

pub(super) fn profile_delta(old: &Resolved, new: &Resolved) -> ProfileDelta {
    let (o, n) = (&old.profile, &new.profile);
    ProfileDelta {
        // Drawn from the profile each frame.
        redraw: old.palette != new.palette
            || o.bold_is_bright != n.bold_is_bright
            || o.opacity != n.opacity
            || o.background_image != n.background_image,
        font: o.font_family != n.font_family
            || o.font_size != n.font_size
            || o.padding != n.padding,
        term: o.scrollback != n.scrollback
            || o.cursor_shape != n.cursor_shape
            || o.cursor_blink != n.cursor_blink,
    }
}

/// The cursor a profile starts its terminals with.
fn cursor_style(p: &Profile) -> CursorStyle {
    let shape = match p.cursor_shape {
        CursorShapeCfg::Block => CursorShape::Block,
        CursorShapeCfg::Beam => CursorShape::Beam,
        CursorShapeCfg::Underline => CursorShape::Underline,
    };
    CursorStyle {
        shape,
        blinking: p.cursor_blink,
    }
}

/// How a pane with profile `p` is spawned: `command` (`-e`) if given, else the profile's
/// command or shell (`env_shell` is `$SHELL`), starting in `cwd` (the process's own
/// directory when `None`).
fn pane_options(
    p: &Profile,
    command: Option<Vec<String>>,
    env_shell: Option<&str>,
    window_id: Option<u64>,
    cwd: Option<PathBuf>,
) -> PaneOptions {
    let runs_command = command.is_some() || p.command.as_ref().is_some_and(|c| !c.is_empty());
    PaneOptions {
        command: Some(command.unwrap_or_else(|| p.argv(env_shell))),
        runs_command,
        working_dir: cwd,
        scrollback: p.scrollback,
        cursor: cursor_style(p),
        window_id,
    }
}

/// A program that couldn't start, and the one that did instead.
#[derive(Debug, PartialEq)]
pub(super) struct Fallback {
    /// The program that failed (`argv[0]`).
    failed: String,
    /// Why it failed.
    err: String,
    /// The program used instead.
    used: String,
}

/// The window's default profile: `--profile`, else the store's default.
fn window_stem_of<'a>(profile_arg: Option<&'a str>, default: &'a str) -> &'a str {
    profile_arg.unwrap_or(default)
}

/// The stem a lookup of `stem` resolves to: itself when that profile exists (`known`),
/// else the default's.
pub(super) fn resolved_stem_of<'a>(known: bool, stem: &'a str, default: &'a str) -> &'a str {
    if known { stem } else { default }
}

/// The startup toast, as (body, tail), for `--profile` naming a profile that doesn't exist
/// (the window runs on the default's).
fn profile_arg_problem(arg: Option<&str>, known: bool, default: &str) -> Option<(String, String)> {
    let arg = arg.filter(|_| !known)?;
    Some((
        format!("--profile: no profile '{arg}'"),
        format!(" (using '{default}')"),
    ))
}

/// Whether an error toast may replace the toast showing now (`existing`'s source): not
/// another problem's (the first one stays; later ones are on stderr), only nothing or an
/// info toast.
fn error_replaces(existing: Option<ToastSource>) -> bool {
    matches!(existing, None | Some(ToastSource::Info))
}

/// The toast after the current one goes: a waiting error (shown for a full
/// TOAST_DURATION from `now`), else the waiting image errors.
fn next_toast(
    pending_error: Option<String>,
    images: impl FnOnce() -> Option<Toast>,
    now: Instant,
) -> Option<Toast> {
    match pending_error {
        Some(text) => Some(Toast::new(&text, ToastSource::Spawn, now)),
        None => images(),
    }
}

/// The toast for a fallback, as (body, tail): what couldn't start and what runs instead.
/// Without an explicit command the failing program is the profile `stem`'s, and the toast
/// says so; a command (a session leaf's) isn't any profile file's fault. The tail is never
/// truncated.
fn fallback_text(stem: Option<&str>, f: &Fallback) -> (String, String) {
    let what = match stem {
        Some(stem) => format!("profiles/{stem}.toml: can't start"),
        None => "can't start".to_string(),
    };
    (
        format!("{what} '{}': {}", f.failed, f.err),
        format!(" (using {})", f.used),
    )
}

/// Spawn `argv` through `try_spawn` (its flag: whether it is a fallback shell). When that
/// fails and `fallback` is allowed (not for `-e`), try `$SHELL` (`env_shell`), then
/// `/bin/sh`, skipping programs already tried. Err is the first error when nothing starts.
fn spawn_with_fallback<P>(
    argv: &[String],
    fallback: bool,
    env_shell: Option<&str>,
    mut try_spawn: impl FnMut(&[String], bool) -> std::io::Result<P>,
) -> std::io::Result<(P, Option<Fallback>)> {
    let first = match try_spawn(argv, false) {
        Ok(p) => return Ok((p, None)),
        Err(e) if !fallback => return Err(e),
        Err(e) => e,
    };
    let failed = argv.first().cloned().unwrap_or_default();
    let mut tried = vec![failed.as_str()];
    for shell in env_shell
        .filter(|s| !s.is_empty())
        .into_iter()
        .chain(["/bin/sh"])
    {
        if tried.contains(&shell) {
            continue;
        }
        tried.push(shell);
        if let Ok(p) = try_spawn(&[shell.to_string()], true) {
            // The error already names the command ("Failed to spawn command '...': ...").
            let err = first.to_string();
            let err = err
                .strip_prefix(&format!("Failed to spawn command '{failed}': "))
                .unwrap_or(&err)
                .to_string();
            let used = shell.to_string();
            return Ok((p, Some(Fallback { failed, err, used })));
        }
    }
    Err(first)
}

/// Start the process for `opts` as pane `id` (see `spawn_with_fallback`).
pub(super) fn spawn_pane_process(
    id: PaneId,
    opts: &PaneOptions,
    size: WindowSize,
    sink: &Arc<dyn EventSink>,
    fallback: bool,
    env_shell: Option<&str>,
) -> std::io::Result<(Pane, Option<Fallback>)> {
    let argv = opts.command.clone().unwrap_or_default();
    spawn_with_fallback(&argv, fallback, env_shell, |argv, is_fallback| {
        let tried = PaneOptions {
            command: Some(argv.to_vec()),
            // A fallback shell is a shell, not a command.
            runs_command: opts.runs_command && !is_fallback,
            ..opts.clone()
        };
        Pane::spawn(id, &tried, size, sink.clone())
    })
}

/// Whether `pane`'s title labels a tab: it is the focused pane of a tab the
/// user hasn't renamed.
fn labels_a_tab(tabs: &Tabs, pane: PaneId) -> bool {
    tabs.iter().any(|t| t.focused == pane && t.title.is_none())
}

/// Store `new` in a pane's title `slot`; true when it changed (a shell
/// re-sending the same OSC 2 at every prompt changes nothing).
fn replace_title(slot: &mut Option<String>, new: Option<String>) -> bool {
    if *slot == new {
        return false;
    }
    *slot = new;
    true
}

/// Every pane's rect in every tab (hidden tabs included) when tiled into `area`.
fn all_layouts(tabs: &Tabs, area: Rect) -> Vec<(PaneId, Rect)> {
    tabs.iter().flat_map(|t| t.tree.layout(area)).collect()
}

/// Whether a keyboard event reaches bindings and the PTY: real presses only.
/// winit sends synthetic presses for keys already held when the window gains
/// focus (launching montty from a Ctrl+Alt+T hotkey would type ESC ^T), so
/// they are ignored, as Alacritty does.
fn handles_key(state: ElementState, is_synthetic: bool) -> bool {
    state == ElementState::Pressed && !is_synthetic
}

/// How long a pane flashes after its bell.
const BELL_FLASH: Duration = Duration::from_millis(150);
/// How long a pane's scrollbar stays after its last scroll.
const SCROLLBAR_LINGER: Duration = Duration::from_secs(1);

/// Whether a pane's scrollbar shows: while scrolled back, and for `SCROLLBAR_LINGER` after
/// its last scroll.
fn scrollbar_visible(display_offset: usize, last_scroll: Option<Instant>, now: Instant) -> bool {
    display_offset > 0 || last_scroll.is_some_and(|t| now < t + SCROLLBAR_LINGER)
}

pub struct Window {
    // Drop order: GL objects before the surface/context, the OS window last.
    // Not dropped when the context can't be made current (see `Drop`): its GL calls would
    // hit another window's context.
    renderer: ManuallyDrop<Renderer>,
    gl_surface: Surface<WindowSurface>,
    gl_context: PossiblyCurrentContext,
    /// Whether a failed `make_current` was already reported (once per window).
    gl_error_logged: Cell<bool>,
    window: winit::window::Window,
    size: PhysicalSize<u32>,
    /// A resize happened since the last frame: draw one more after it. Mesa's software
    /// rasterizer (llvmpipe, e.g. in Xephyr or a VM) takes a new window size only at a
    /// swap, so the first frame after a resize goes to the old-size buffer.
    resize_redraw: bool,
    scale: f64,
    /// Every font in use: the panes' and the UI's. Panes look theirs up each frame.
    fonts: FontCache,
    /// The UI text font (bar, menus, panels, toast): the window default profile's font
    /// at its size, unzoomed.
    ui_font: u32,
    ui_metrics: Metrics,
    /// `--profile`: the window's default profile; None follows the store's default.
    profile_arg: Option<String>,
    /// Each pane's profile and zoom.
    pane_profiles: HashMap<PaneId, PaneStyle>,
    /// Missing fonts a toast reported.
    fonts_reported: ReportedFonts,
    /// The config files; key bindings and chrome colors are read from it as needed.
    store: Store,
    /// Watches the config dir; None without one or when watching failed.
    watcher: Option<Watcher>,
    /// Coalesces the watcher's events into one reload.
    debounce: Debounce,
    toast: Option<Toast>,
    /// Image errors (path, text) that came while another toast was up: shown when that
    /// one goes.
    pending_images: Vec<(PathBuf, String)>,
    /// The newest error that came while a config, startup or spawn toast was up: shown
    /// when that one goes.
    pending_error: Option<String>,
    /// The panes' background images, by path as their profile names it.
    images: images::Images<ImageTexture>,
    /// Bounds the image helper processes (see `images::Gate`).
    decode_gate: images::Gate<(PathBuf, u64)>,
    /// Images are downscaled to fit this (see `images::image_limit`).
    image_limit: (u32, u32),
    /// The config errors the last toast showed (empty after a clean reload).
    toast_errors: Vec<ConfigError>,
    modifiers: ModifiersState,
    links: links::LinkState,
    /// The open find bar and its search; closed by a click on a pane, a tab switch,
    /// the pane's close or focus moving off it.
    find: Option<find::FindState>,
    window_focused: bool,
    /// The pane flashing from its bell, and when the flash ends.
    bell: Option<(PaneId, Instant)>,
    /// When each pane last scrolled by a user action (its scrollbar lingers a second).
    scrolled: HashMap<PaneId, Instant>,
    /// The "cols × rows" labels shown while panes are resized.
    size_labels: SizeLabels,
    /// Whether a key or mouse button was pressed yet (a window resize labels only then).
    had_input: bool,
    /// Until when a `Resized` is the echo of a resize montty asked for (no labels).
    quiet_resize_until: Option<Instant>,
    /// The focused pane's cursor blink cycle.
    blink: Blink,
    /// The focused pane's effective cursor style blinks (read at each frame).
    blink_on: bool,
    /// Whether the last frame drew the cursor (the blink phase it showed).
    blink_shown: bool,
    panes: HashMap<PaneId, Pane>,
    /// Last size sent to each pane's PTY.
    sizes: HashMap<PaneId, WindowSize>,
    /// Each tab owns a split tree and its focused pane; panes live in `panes`.
    tabs: Tabs,
    /// Exported to every pane's child as `WINDOWID`; None on Wayland.
    x11_window_id: Option<u64>,
    sink: Arc<dyn EventSink>,
    mouse: mouse::MouseState,
    /// The bar item under the pointer (drawn highlighted); None off the bar.
    hover: Option<Target>,
    /// The open menu, panel, rename field or close prompt; it takes every
    /// key and press.
    modal: modal::Modal,
    /// The pane whose hover overlay is shown and its hovered button, as last
    /// computed (on pointer motion and every frame).
    overlay: Option<(PaneId, Option<OverlayTarget>)>,
    /// The last key press that reached a PTY or a binding: the overlay hides
    /// until the pointer moves again.
    last_key: Option<Instant>,
    /// Last known OS maximized state (no edge resize while maximized, the
    /// Restore icon): refreshed on resize (a maximize comes with one) and
    /// after our own `set_maximized`, never per frame (an X11 round trip).
    maximized: bool,
    /// Set by an action that closed the last pane.
    close_requested: bool,
    /// The session name this window last saved or opened: the save prompt's pre-fill.
    last_session: Option<String>,
    /// When the debounced autosave of `_last.json` fires; None while nothing is pending.
    autosave_due: Option<Instant>,
    snapshot: Snapshot,
    scheduler: FrameScheduler,
    /// CLIPBOARD and PRIMARY (selection.rs). The display they use belongs to the event
    /// loop, so they may outlive `window`.
    clipboards: Box<dyn Clipboards>,
    /// What this window asked of the app (a new window, ...); drained by `take_requests`.
    requests: Vec<AppRequest>,
}

impl Drop for Window {
    fn drop(&mut self) {
        // The GL objects (fields) drop after this: they need their own context current.
        // Without it the renderer's objects are leaked rather than deleted in a wrong context.
        if self.ensure_current() {
            // SAFETY: dropped here once; the field is not used again.
            unsafe { ManuallyDrop::drop(&mut self.renderer) };
        }
    }
}

impl Window {
    pub fn new(
        event_loop: &ActiveEventLoop,
        init: WindowInit,
        sink: Arc<dyn EventSink>,
    ) -> Result<Window, String> {
        // A session window spawns the session's panes first (see `install_session`); its
        // tab list starts with a placeholder tab (no pane) that they replace. Only when none
        // of them starts does it get one default pane.
        let (profile_arg, command, session) = match init {
            WindowInit::Default { profile, command } => (profile, command, None),
            WindowInit::Session { doc, name, profile } => (profile, None, Some((doc, name))),
        };
        let now = Instant::now();
        let dir = config_dir();
        let (store, toast_errors, mut toast) = load_store(dir.as_deref(), now);
        // The first pane runs under the window's default profile, which also sets the
        // UI font and the initial size.
        let first = PaneStyle {
            stem: (profile_arg.as_deref())
                .unwrap_or(store.default_stem())
                .to_string(),
            zoom_pt: 0.0,
        };
        let mut fonts = FontCache::new(system_db());
        let (f1, _) =
            resolve_font(&mut fonts, &store, &first.stem, 0.0, 1.0).map_err(|e| e.to_string())?;
        let (m, padding) = (f1.metrics, f1.padding as u32);
        let (cols, lines) = (80, 24);

        // The window stays hidden until the first pane has spawned, so a bad command
        // fails without a window flash while the child still gets the real WINDOWID.
        // (Wayland ignores `visible`, but shows nothing before the first frame.)
        // montty draws its own top bar: no system title bar or frame.
        let attrs = winit::window::Window::default_attributes()
            .with_title(TITLE)
            .with_window_icon(icon::window_icon())
            .with_decorations(false)
            .with_transparent(true)
            .with_visible(false)
            .with_inner_size(LogicalSize::new(
                cols as u32 * m.cell_width + 2 * padding,
                lines as u32 * m.cell_height + 2 * padding + BarMetrics::new(&m, 1.0).height as u32,
            ));
        // The app id / WM_CLASS; both backends define `with_name`, so call them qualified.
        let attrs =
            winit::platform::x11::WindowAttributesExtX11::with_name(attrs, "montty", "montty");
        let attrs = winit::platform::wayland::WindowAttributesExtWayland::with_name(
            attrs, "montty", "montty",
        );
        let (window, gl_surface, gl_context, glow) = gl::create(event_loop, attrs)?;
        let watcher = dir.and_then(|d| {
            let (w, errs) = start_watcher(&d, window.id(), sink.clone());
            for e in &errs {
                eprintln!("montty: config watcher: {e}");
            }
            // One toast at a time: the config's own errors come first.
            if let Some(t) = watcher_toast(&errs, now) {
                toast.get_or_insert(t);
            }
            w
        });
        if let Some((body, tail)) = profile_arg_problem(
            profile_arg.as_deref(),
            profile_arg.as_deref().is_some_and(|p| store.contains(p)),
            store.default_stem(),
        ) {
            eprintln!("montty: {body}{tail}");
            // One toast at a time: the config's own errors come first.
            toast.get_or_insert_with(|| Toast::with_tail(&body, &tail, ToastSource::Startup, now));
        }
        let scale = window.scale_factor();
        // Unzoomed, the first pane's font is the UI font.
        let (font, _) =
            resolve_font(&mut fonts, &store, &first.stem, 0.0, scale).map_err(|e| e.to_string())?;

        let x11_window_id = x11_window_id(&window);
        let shell = std::env::var("SHELL").ok();
        let id = next_pane_id();
        let bar_h = BarMetrics::new(&font.metrics, scale).height;
        let (cols, lines) = grid_size(
            bar::pane_area(window.inner_size(), bar_h),
            &font.metrics,
            font.padding,
        );
        let size = window_size(cols, lines, &font.metrics);
        // The error already names the command ("Failed to spawn command '...': ...").
        // A profile's program that can't start falls back to a shell; `-e` doesn't.
        let mut first_pane = None;
        if session.is_none() {
            let opts = pane_options(
                &store.profile(&first.stem).profile,
                command.clone(),
                shell.as_deref(),
                x11_window_id,
                None,
            );
            let (mut pane, fallback) =
                spawn_pane_process(id, &opts, size, &sink, command.is_none(), shell.as_deref())
                    .map_err(|e| e.to_string())?;
            pane.set_explicit_command(command.clone());
            if let Some(f) = fallback {
                let stem = resolved_stem_of(
                    store.contains(&first.stem),
                    &first.stem,
                    store.default_stem(),
                );
                let (body, tail) = fallback_text(Some(stem), &f);
                eprintln!("montty: {body}{tail}");
                // One toast at a time: the config's own errors come first.
                toast
                    .get_or_insert_with(|| Toast::with_tail(&body, &tail, ToastSource::Spawn, now));
            }
            first_pane = Some(pane);
        }

        let renderer = Renderer::new(glow, ATLAS_SIZE)?;
        let image_limit = images::image_limit(
            event_loop.available_monitors().map(|m| m.size()),
            renderer.max_texture_size(),
        );

        let tabs = Tabs::new(Tab::new(id, pane_min_extent(&font.metrics, font.padding)));
        let refresh = window
            .current_monitor()
            .and_then(|mon| mon.refresh_rate_millihertz());

        let clipboards = match window.display_handle() {
            Ok(h) => system_clipboards(h.as_raw()),
            Err(_) => Box::new(MemClipboards::default()),
        };
        let mut w = Window {
            renderer: ManuallyDrop::new(renderer),
            gl_surface,
            gl_context,
            gl_error_logged: Cell::new(false),
            size: PhysicalSize::new(0, 0),
            resize_redraw: false,
            window,
            scale,
            fonts,
            ui_font: font.key,
            ui_metrics: font.metrics,
            profile_arg: profile_arg.clone(),
            pane_profiles: HashMap::from([(id, first)]),
            fonts_reported: ReportedFonts::default(),
            store,
            watcher,
            debounce: Debounce::default(),
            toast,
            pending_images: Vec::new(),
            pending_error: None,
            images: images::Images::default(),
            decode_gate: images::Gate::default(),
            image_limit,
            toast_errors,
            modifiers: ModifiersState::empty(),
            links: links::LinkState::new(),
            find: None,
            window_focused: true,
            bell: None,
            scrolled: HashMap::new(),
            size_labels: SizeLabels::default(),
            had_input: false,
            quiet_resize_until: None,
            blink: Blink::new(now),
            blink_on: false,
            blink_shown: true,
            panes: first_pane.map(|p| (id, p)).into_iter().collect(),
            sizes: HashMap::from([(id, size)]),
            tabs,
            x11_window_id,
            sink,
            mouse: mouse::MouseState::default(),
            hover: None,
            modal: modal::Modal::None,
            overlay: None,
            last_key: None,
            maximized: false,
            close_requested: false,
            last_session: None,
            autosave_due: None,
            snapshot: Snapshot::default(),
            scheduler: FrameScheduler::new(refresh),
            clipboards,
            requests: Vec::new(),
        };
        w.on_resize(w.window.inner_size(), false);
        if let Some((doc, name)) = session
            && !w.install_session(&doc, name)
        {
            // Nothing in the session started (the toast says why): one default pane in
            // the placeholder tab.
            let rect = w.area();
            let Some((pane, size)) = w.spawn_pane(id, rect, None, None) else {
                return Err(w
                    .toast_text()
                    .unwrap_or_else(|| "can't start a shell".into()));
            };
            w.panes.insert(id, pane);
            w.sizes.insert(id, size);
        }
        w.sync_images(false);
        w.window.set_visible(true);
        Ok(w)
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    pub fn has_pane(&self, id: PaneId) -> bool {
        self.panes.contains_key(&id)
    }

    /// The requests made since the last call.
    pub fn take_requests(&mut self) -> Vec<AppRequest> {
        std::mem::take(&mut self.requests)
    }

    /// Show `text` as an error toast (a window the app failed to open for this one). A
    /// config, startup or spawn toast on screen is not replaced; the error shows when
    /// that toast goes (the newest one, the rest are on stderr).
    pub fn show_error(&mut self, text: &str) {
        if !error_replaces(self.toast.as_ref().map(|t| t.source)) {
            self.pending_error = Some(text.to_string());
            return;
        }
        self.toast = Some(Toast::new(text, ToastSource::Spawn, Instant::now()));
        self.schedule();
    }

    /// The text (with tail) of the toast showing now.
    fn toast_text(&self) -> Option<String> {
        self.toast.as_ref().map(|t| format!("{}{}", t.text, t.tail))
    }

    /// Make this window's GL context current. Every window has its own, so each GL use
    /// (drawing, resizing, uploads, dropping GL objects) must come after this.
    /// False when it can't be (reported once per window): callers about to do GL work
    /// skip it.
    fn ensure_current(&self) -> bool {
        if self.gl_context.is_current() {
            return true;
        }
        match self.gl_context.make_current(&self.gl_surface) {
            Ok(()) => true,
            Err(e) => {
                if !self.gl_error_logged.replace(true) {
                    eprintln!("montty: warning: cannot make the GL context current: {e}");
                }
                false
            }
        }
    }

    /// Returns false when the window should close.
    pub fn handle_window_event(&mut self, ev: WindowEvent, _el: &ActiveEventLoop) -> bool {
        self.ensure_current();
        match ev {
            // Asks first when a pane runs a process (also while another
            // pane's or tab's prompt is open: it becomes the window's).
            WindowEvent::CloseRequested => self.request_close(CloseTarget::Window),
            WindowEvent::Resized(size) => {
                let modal = self.modal_open();
                self.close_menu();
                self.on_resize(size, modal);
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.close_menu();
                self.set_scale(scale_factor);
            }
            WindowEvent::Focused(f) => {
                self.window_focused = f;
                self.blink_reset();
                // The menu is a popup: it closes when the window loses focus.
                if !f {
                    self.close_menu();
                    if self.mouse.pane_drag.is_some() {
                        self.mouse.cancel_drag();
                        self.set_cursor_icon(CursorIcon::Default);
                    }
                }
                self.schedule();
            }
            WindowEvent::ModifiersChanged(m) => {
                self.modifiers = m.state();
                self.update_link_hover();
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } if handles_key(event.state, is_synthetic) => self.on_key(&event),
            WindowEvent::CursorMoved { position, .. } => self.on_cursor_moved(position),
            WindowEvent::CursorLeft { .. } => self.on_cursor_left(),
            WindowEvent::MouseInput { state, button, .. } => self.on_mouse_input(state, button),
            WindowEvent::MouseWheel { delta, .. } => self.on_wheel(delta),
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
        !self.close_requested
    }

    /// A status or notification sequence from a pane's output. Returns false when the
    /// window should close (as `handle_term_event`). The reply to a feature query is
    /// written from here (the UI thread), never from the PTY thread.
    pub fn handle_signal(&mut self, pane: PaneId, signal: Signal) -> bool {
        if let (Signal::StatusQuery, Some(p)) = (&signal, self.panes.get(&pane)) {
            p.write(b"\x1b]7501;?\x1b\\".to_vec());
        }
        true
    }

    /// Returns false when the window should close (its last pane exited).
    pub fn handle_term_event(&mut self, pane: PaneId, ev: Event) -> bool {
        let Some(p) = self.panes.get_mut(&pane) else {
            return true;
        };
        match ev {
            // Output in a hidden pane (inactive tab, behind a maximized pane) draws nothing.
            Event::Wakeup if tabs::visible(&self.tabs, pane) => {
                if pane == self.tabs.active().focused {
                    self.blink_reset();
                }
                // Output shifts grid coordinates under the find bar's current match.
                if let Some(f) = self.find.as_mut().filter(|f| f.pane == pane)
                    && let Some(p) = self.panes.get(&pane)
                {
                    f.revalidate(&p.term.lock());
                }
                // Output may have moved the link under a still pointer.
                if self.links.hover.as_ref().is_some_and(|(id, _)| *id == pane) {
                    self.update_link_hover();
                }
                self.schedule();
            }
            Event::Wakeup => {}
            Event::PtyWrite(s) => p.write(s.into_bytes()),
            Event::Title(t) => {
                if replace_title(&mut p.title, Some(t)) {
                    self.title_changed(pane);
                }
            }
            Event::ResetTitle => {
                if replace_title(&mut p.title, None) {
                    self.title_changed(pane);
                }
            }
            Event::ColorRequest(idx, fmt) => {
                let profile = self.pane_profile(pane);
                let Some(p) = self.panes.get(&pane) else {
                    return true;
                };
                let rgb = resolve(
                    color_for_index(idx),
                    &profile.palette,
                    p.term.lock().colors(),
                );
                p.write(fmt(rgb).into_bytes());
            }
            Event::TextAreaSizeRequest(fmt) => {
                if let Some(size) = self.sizes.get(&pane) {
                    p.write(fmt(*size).into_bytes());
                }
            }
            Event::ChildExit(_) | Event::Exit => {
                self.ensure_current();
                return self.close_pane(pane);
            }
            Event::ClipboardStore(ty, text) => {
                let kind = match ty {
                    ClipboardType::Clipboard => ClipKind::Clipboard,
                    ClipboardType::Selection => ClipKind::Primary,
                };
                self.clipboards.store(kind, text);
            }
            // OSC 52 policy: programs may store, never read (nothing is written back).
            Event::ClipboardLoad(..) => {}
            Event::Bell => self.ring_bell(pane),
            // Mouse cursor and blink are not handled yet.
            Event::MouseCursorDirty | Event::CursorBlinkingChange => {}
        }
        true
    }

    /// The earliest of: the pending frame's deadline (when the scheduler deferred it to the
    /// next refresh), the config reload's, the toast's expiry and the next cursor toggle.
    /// `now` is the same instant `on_timers` ran with.
    pub fn next_deadline(&self, now: Instant) -> Option<Instant> {
        [
            self.scheduler.deadline(),
            self.debounce.deadline(),
            self.autosave_deadline(),
            self.toast.as_ref().map(|t| t.until),
            self.blink_deadline(now),
            self.bell.map(|(_, until)| until),
            self.scrolled.values().map(|t| *t + SCROLLBAR_LINGER).min(),
            self.size_labels.deadline(),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// The next cursor toggle, only while the focused pane of a focused window blinks.
    fn blink_deadline(&self, now: Instant) -> Option<Instant> {
        if self.window_focused && self.blink_on {
            self.blink.next_toggle(now)
        } else {
            None
        }
    }

    /// Restart the blink cycle (cursor visible). Only an Instant store, plus a frame when
    /// the cursor was hidden.
    fn blink_reset(&mut self) {
        let now = Instant::now();
        let hidden = self.blink_on && self.window_focused && !self.blink.visible(now);
        self.blink.reset(now);
        if hidden {
            self.schedule();
        }
    }

    /// The watcher saw a change in the config dir: reload once the burst settles.
    pub fn config_changed(&mut self, now: Instant) {
        self.debounce.poke(now);
    }

    /// Runs the timers due at `now`: the debounced config reload and the toast's expiry.
    pub fn on_timers(&mut self, now: Instant) {
        if self.debounce.take_due(now) {
            self.reload_config(now);
        }
        self.autosave_tick(now);
        if self.toast.as_ref().is_some_and(|t| now >= t.until) {
            self.dismiss_toast();
        }
        // A flash or a scrollbar linger ended: clear it, one frame for both.
        let flash_over = self.bell.is_some_and(|(_, until)| now >= until);
        if flash_over {
            self.bell = None;
        }
        let before = self.scrolled.len();
        self.scrolled.retain(|_, t| now < *t + SCROLLBAR_LINGER);
        if flash_over || self.scrolled.len() != before {
            self.schedule();
        }
        if self.size_labels.tick(now) {
            self.schedule();
        }
        // A cursor toggle is due: a frame only when the visible state changed.
        if self.window_focused && self.blink_on && self.blink.tick(now, self.blink_shown).redraw {
            self.schedule();
        }
    }

    /// A bell rang in `pane`: a visible pane flashes (a new bell restarts it) unless
    /// `visual_bell` is off; an unfocused window is also asked for attention.
    fn ring_bell(&mut self, pane: PaneId) {
        if self.store.global().visual_bell && tabs::visible(&self.tabs, pane) {
            self.bell = Some((pane, Instant::now() + BELL_FLASH));
            self.schedule();
        }
        if !self.window_focused {
            self.window
                .request_user_attention(Some(winit::window::UserAttentionType::Informational));
        }
    }

    /// `pane`'s view moved by a user action: its scrollbar shows for a while. The frame
    /// is the caller's.
    fn mark_scrolled(&mut self, pane: PaneId) {
        self.scrolled.insert(pane, Instant::now());
    }

    /// Hide the toast (expired or clicked), showing the error or image errors that
    /// waited for it: one frame.
    fn dismiss_toast(&mut self) {
        if self.toast.take().is_some() {
            let pending_error = self.pending_error.take();
            self.toast = next_toast(
                pending_error,
                || self.take_pending_image_toast(),
                Instant::now(),
            );
            self.schedule();
        }
    }

    /// Re-read the config dir. Bindings and chrome colors are read from the store as
    /// needed, so they apply from the next key or frame.
    fn reload_config(&mut self, now: Instant) {
        self.ensure_current();
        let report = self.store.reload();
        let changed = report.global_changed
            || !report.changed.is_empty()
            || !report.added.is_empty()
            || !report.removed.is_empty();
        if report.global_changed {
            self.schedule();
        }
        match reload_toast(
            self.toast.as_ref().map(|t| t.source),
            &self.toast_errors,
            &report.errors,
            // A save of a file that still doesn't parse changes no profile, but deserves
            // the toast again.
            changed || report.edited,
            now,
        ) {
            ToastUpdate::Keep => {}
            ToastUpdate::Show(t) => {
                log_errors(&report.errors);
                // A config-error toast replaces whatever toast is up: one toast at a time,
                // and the errors are what the user just caused. (Image toasts queue instead,
                // because they arrive asynchronously, whenever a decode finishes.)
                self.toast = Some(t);
                self.schedule();
            }
            ToastUpdate::Hide => self.dismiss_toast(),
        }
        self.apply_profile_changes(&report);
        self.toast_errors = report.errors;
        // Images whose file changed load again. The watched dirs follow: dirs created since
        // (profiles/, schemes/, a new symlink's target dir), dirs no longer needed (a removed
        // link's target dir, the config dir's parent) and the images' dirs.
        self.sync_images(true);
    }

    /// Profiles changed, appeared or went away (`r`; the store still has what they were
    /// before). Each pane whose profile resolves differently now does only what changed
    /// (see `profile_delta`); the UI font follows the window default's. One frame at most.
    fn apply_profile_changes(&mut self, r: &ReloadReport) {
        let store = &self.store;
        let default_changed = store.previous_default_stem() != store.default_stem();
        let affected = affected_panes(
            &self.pane_profiles,
            r,
            default_changed,
            store.default_stem(),
            |s| store.contains(s),
        );
        let mut all = ProfileDelta::default();
        for id in affected {
            let stem = self.pane_stem(id);
            let (old, new) = (self.store.previous_profile(stem), self.store.profile(stem));
            all = all.or(self.restyle_pane(id, &old, &new));
        }
        let ui_before = window_stem_of(
            self.profile_arg.as_deref(),
            self.store.previous_default_stem(),
        );
        let (old, new) = (
            self.store.previous_profile(ui_before),
            self.store.profile(self.window_stem()),
        );
        let ui_changed = old.profile.font_family != new.profile.font_family
            || old.profile.font_size != new.profile.font_size;
        if ui_changed {
            self.load_ui_font();
        }
        // Another bar height is laid out as other cells are.
        self.settle_restyle(ProfileDelta {
            font: all.font || ui_changed,
            ..all
        });
        self.evict_fonts();
    }

    /// Switch pane `id` to profile `stem`, keeping its zoom; it redoes only what
    /// resolves differently (see `profile_delta`).
    pub(super) fn set_pane_profile(&mut self, id: PaneId, stem: &str) {
        let old = self.pane_profile(id);
        let Some(style) = self.pane_profiles.get_mut(&id) else {
            return;
        };
        if style.stem == stem {
            return;
        }
        style.stem = stem.to_string();
        self.layout_changed();
        let new = self.pane_profile(id);
        let d = self.restyle_pane(id, &old, &new);
        self.settle_restyle(d);
        self.evict_fonts();
        self.sync_images(false);
    }

    /// Pane `id`'s profile resolves to `new` instead of `old`: its terminal takes the new
    /// scrollback and cursor when those differ. What else it needs is returned, for
    /// `settle_restyle` once every restyled pane is done.
    fn restyle_pane(&self, id: PaneId, old: &Resolved, new: &Resolved) -> ProfileDelta {
        let d = profile_delta(old, new);
        if d.term
            && let Some(pane) = self.panes.get(&id)
        {
            pane.set_options(new.profile.scrollback, cursor_style(&new.profile));
        }
        d
    }

    /// After panes were restyled (`d`, their deltas combined): other cells mean every
    /// tab's minimum and PTY sizes again (only PTYs whose size changed are told); else one
    /// frame when anything shows differently.
    fn settle_restyle(&mut self, d: ProfileDelta) {
        if d.font {
            self.refresh_min_extents();
            self.relayout_all();
        } else if d.redraw || d.term {
            self.schedule();
        }
    }

    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }

    /// Ask for a frame; draws right away when idle, else at the next refresh deadline.
    fn schedule(&mut self) {
        if self.scheduler.request(Instant::now()) == Schedule::Now {
            self.window.request_redraw();
        }
    }

    /// Key routing: an open modal takes the key and nothing else does; else
    /// a binding runs; else the key goes to the focused pane's PTY. A key
    /// that runs a binding or reaches the PTY is stamped (hides the overlay).
    fn on_key(&mut self, event: &KeyEvent) {
        self.had_input = true;
        if self.mouse.pane_drag.is_some() && event.logical_key == Key::Named(NamedKey::Escape) {
            self.mouse.cancel_drag();
            self.set_cursor_icon(CursorIcon::Default);
            self.refresh_overlay();
            self.schedule();
            return;
        }
        if self.modal_open() {
            self.modal_key(event);
            return;
        }
        if self.find.is_some() {
            self.find_key(event);
            return;
        }
        let mods = Mods::from(self.modifiers);
        let key = event.key_without_modifiers();
        let focused = self.tabs.active().focused;
        if let Some(action) = self
            .store
            .global()
            .bindings
            .resolve(&key, event.physical_key, mods)
        {
            self.stamp_key();
            self.perform(action);
            return;
        }
        let Some(pane) = self.panes.get(&focused) else {
            return;
        };
        let mut term = pane.term.lock();
        let input = KeyInput {
            key: &key,
            text: event.text.as_deref(),
            mods,
        };
        let Some(bytes) = encode_key(&input, *term.mode()) else {
            return;
        };
        let scrolled = term.grid().display_offset() != 0;
        if scrolled {
            term.scroll_display(Scroll::Bottom);
        }
        drop(term);
        pane.mark_input();
        pane.write(bytes);
        self.stamp_key();
        if scrolled {
            self.mark_scrolled(focused);
            self.schedule();
        }
    }

    /// A key went to a PTY or a binding: the overlay hides until the pointer
    /// moves again. Costs a frame only when an overlay is shown right now.
    fn stamp_key(&mut self) {
        let now = Instant::now();
        self.last_key = Some(now);
        self.blink_reset();
        if self.overlay.is_some() {
            self.overlay = None;
            self.schedule();
        }
    }

    /// The window below the top bar, in pixels; the split tree tiles it.
    fn area(&self) -> Rect {
        bar::pane_area(self.size, self.bar_metrics().height)
    }

    fn on_resize(&mut self, size: PhysicalSize<u32>, modal_was_open: bool) {
        self.size = size;
        self.resize_redraw = true;
        self.maximized = self.window.is_maximized();
        if let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) {
            self.gl_surface.resize(&self.gl_context, w, h);
        }
        if self.ensure_current() {
            self.renderer.resize(size.width, size.height);
        }
        let ids = self.relayout_all();
        let quiet = self.quiet_resize_until.is_some_and(|t| Instant::now() < t);
        if window_resize_labels(self.had_input, modal_was_open || self.modal_open(), quiet) {
            self.label_resized(ids);
        }
    }

    /// Resize the active tab's panes to their rects, then redraw. Returns the panes whose
    /// size changed.
    fn relayout(&mut self) -> Vec<PaneId> {
        let layout = self.tabs.active().tree.layout(self.area());
        let ids = self.resize_panes(layout);
        self.schedule();
        ids
    }

    /// Like `relayout`, for every tab: hidden tabs keep correct PTY sizes.
    fn relayout_all(&mut self) -> Vec<PaneId> {
        let layout = all_layouts(&self.tabs, self.area());
        let ids = self.resize_panes(layout);
        self.schedule();
        ids
    }

    /// Show the size label on those of `ids` that are in the active tab (none while a
    /// modal is open).
    fn label_resized(&mut self, ids: Vec<PaneId>) {
        if self.modal_open() {
            return;
        }
        let layout = self.tabs.active().tree.layout(self.area());
        let ids = visible_resized(&ids, &layout);
        self.size_labels.resized(&ids, Instant::now());
        self.schedule();
    }

    /// Resize each pane's grid to its rect; only PTYs whose size changed are told.
    /// Returns those panes.
    fn resize_panes(&mut self, layout: Vec<(PaneId, Rect)>) -> Vec<PaneId> {
        let mut resized = Vec::new();
        for (id, rect) in layout {
            let font = self.pane_font(id);
            let (cols, lines) = grid_size(rect, &font.metrics, font.padding);
            let size = window_size(cols, lines, &font.metrics);
            let unchanged = self.sizes.get(&id).is_some_and(|s| same_size(s, &size));
            if let Some(pane) = self.panes.get_mut(&id).filter(|_| !unchanged) {
                pane.resize(size);
                self.sizes.insert(id, size);
                resized.push(id);
            }
        }
        for &id in &resized {
            self.find_resized(id);
        }
        resized
    }

    /// The window moved to a monitor with another scale: every font is loaded again at it.
    fn set_scale(&mut self, scale: f64) {
        self.scale = scale;
        self.load_ui_font();
        // Glyphs at the old scale would only take atlas space.
        self.renderer.clear_glyphs();
        self.refresh_min_extents();
        self.relayout_all();
        self.evict_fonts();
        self.quiet_resize_until = Some(Instant::now() + QUIET_RESIZE);
    }

    /// The UI font: the window default profile's font at its size, unzoomed. When even
    /// monospace fails, the old one stays.
    fn load_ui_font(&mut self) {
        let stem = window_stem_of(self.profile_arg.as_deref(), self.store.default_stem());
        match resolve_font(&mut self.fonts, &self.store, stem, 0.0, self.scale) {
            Ok((font, _)) => {
                self.ui_font = font.key;
                self.ui_metrics = font.metrics;
            }
            Err(e) => eprintln!("montty: {e}"),
        }
    }

    /// The window's default profile: `--profile`, else the store's default. The first pane
    /// and new tabs use it.
    pub(super) fn window_stem(&self) -> &str {
        window_stem_of(self.profile_arg.as_deref(), self.store.default_stem())
    }

    /// A new pane under the window's default profile.
    pub(super) fn default_style(&self) -> PaneStyle {
        PaneStyle {
            stem: self.window_stem().to_string(),
            zoom_pt: 0.0,
        }
    }

    /// The profile stem pane `id` records (an unknown pane: the window default).
    pub(super) fn pane_stem(&self, id: PaneId) -> &str {
        match self.pane_profiles.get(&id) {
            Some(s) => &s.stem,
            None => self.window_stem(),
        }
    }

    /// The stem of the profile pane `id` runs under: its recorded stem, or the default's
    /// when that profile is gone.
    pub(super) fn resolved_stem(&self, id: PaneId) -> &str {
        let stem = self.pane_stem(id);
        resolved_stem_of(self.store.contains(stem), stem, self.store.default_stem())
    }

    /// Show `text` plus an uncut `tail` (a spawn problem) as a toast, and print it.
    pub(super) fn spawn_toast(&mut self, text: &str, tail: &str) {
        eprintln!("montty: {text}{tail}");
        self.toast = Some(Toast::with_tail(
            text,
            tail,
            ToastSource::Spawn,
            Instant::now(),
        ));
        self.schedule();
    }

    /// The profile pane `id` runs under (see `pane_stem`).
    pub(super) fn pane_profile(&self, id: PaneId) -> Arc<Resolved> {
        self.store.profile(self.pane_stem(id))
    }

    /// Pane `id`'s font, metrics and padding now (see `pane_stem`; a new pane's style is
    /// recorded before it spawns). Lookups only, once the font is loaded, even for a
    /// missing family.
    pub(super) fn pane_font(&mut self, id: PaneId) -> PaneFont {
        let (stem, zoom) = match self.pane_profiles.get(&id) {
            Some(s) => (s.stem.as_str(), s.zoom_pt),
            None => (
                window_stem_of(self.profile_arg.as_deref(), self.store.default_stem()),
                0.0,
            ),
        };
        let (font, err) = match resolve_font(&mut self.fonts, &self.store, stem, zoom, self.scale) {
            Ok(found) => found,
            Err(e) => (self.ui_pane_font(), Some(e)),
        };
        if let Some(e) = err
            && self.toast.is_none()
        {
            // The file the font came from: an unknown stem uses the default profile's.
            let stem = resolved_stem_of(self.store.contains(stem), stem, self.store.default_stem());
            let r = self.store.profile(stem);
            if !self.fonts_reported.contains(stem, &r.profile.font_family) {
                let stem = stem.to_string();
                self.font_missing(&stem, &r.profile.font_family, &e);
            }
        }
        font
    }

    /// Last resort when not even monospace loads: the UI font, which did at startup.
    fn ui_pane_font(&self) -> PaneFont {
        PaneFont {
            key: self.ui_font,
            metrics: self.ui_metrics,
            padding: 0,
        }
    }

    /// A toast for profile `stem`'s missing `family`, once per (stem, family). While
    /// another toast is up it waits (the caller checks): a later lookup shows it.
    fn font_missing(&mut self, stem: &str, family: &str, err: &str) {
        let text = format!("profiles/{stem}.toml: {err}");
        eprintln!("montty: {text}");
        self.toast = Some(Toast::new(&text, ToastSource::Font, Instant::now()));
        self.fonts_reported.insert(stem, family);
        self.schedule();
    }

    /// The font keys the UI and every pane in every tab draw with. Resolves each pane's
    /// font, so a set a pane needs is loaded.
    fn live_font_keys(&mut self) -> HashSet<u32> {
        let ids: Vec<PaneId> = self.tabs.iter().flat_map(|t| t.tree.panes()).collect();
        let keys: Vec<u32> = ids.into_iter().map(|id| self.pane_font(id).key).collect();
        live_keys(self.ui_font, keys.into_iter())
    }

    /// Drop the font sets nothing draws with. After zoom, profile, config and scale
    /// changes and pane close, never per frame. Atlas entries of dropped keys stay until
    /// the atlas next clears; keys are never reused, so they can't alias.
    pub(super) fn evict_fonts(&mut self) {
        let live = self.live_font_keys();
        self.fonts.retain(&live);
    }

    /// Each tab's tree minimum: the largest pane minimum among its panes. After a pane's
    /// font or padding changed, and when panes come or go.
    fn refresh_min_extents(&mut self) {
        let trees: Vec<Vec<PaneId>> = self.tabs.iter().map(|t| t.tree.panes()).collect();
        let mins: Vec<(i32, i32)> = trees
            .into_iter()
            .map(|panes| {
                tab_min_extent(panes.into_iter().map(|id| {
                    let font = self.pane_font(id);
                    pane_min_extent(&font.metrics, font.padding)
                }))
            })
            .collect();
        for (tab, (w, h)) in self.tabs.iter_mut().zip(mins) {
            tab.tree.set_min_extent(w, h);
        }
    }

    /// Focus `id` in the active tab; the window title follows the tab's label.
    fn set_focus(&mut self, id: PaneId) {
        if self.find.as_ref().is_some_and(|f| f.pane != id) {
            self.find = None;
        }
        self.tabs.active_mut().focused = id;
        self.blink_reset();
        self.update_title();
        self.schedule();
    }

    /// After the active tab changed: lay out its panes, show its label as the
    /// window title and redraw. Panes of other tabs keep running.
    fn show_active(&mut self) {
        self.size_labels.clear();
        self.set_focus(self.tabs.active().focused);
        self.relayout();
    }

    /// `pane`'s title changed: it shows only as the label of a tab it labels
    /// (see `labels_a_tab`), and the active tab's label is the window title.
    fn title_changed(&mut self, pane: PaneId) {
        if !labels_a_tab(&self.tabs, pane) {
            return;
        }
        if pane == self.tabs.active().focused {
            self.update_title();
        }
        self.schedule();
    }

    /// Drop a pane (its PTY shuts down) and remove it from its tab, in any tab;
    /// focus there moves to the pane that took its place, and a tab left empty
    /// closes. Returns false when no tabs are left.
    fn close_pane(&mut self, id: PaneId) -> bool {
        let in_active = self.tabs.tab_of(id) == Some(self.tabs.active_index());
        let sibling = self.rename_sibling(id);
        let area = self.area();
        let removed = actions::remove_pane(&mut self.tabs, id, area);
        self.panes.remove(&id);
        self.sizes.remove(&id);
        self.pane_profiles.remove(&id);
        if self.find.as_ref().is_some_and(|f| f.pane == id) {
            self.find = None;
        }
        if self.tabs.is_empty() {
            return false;
        }
        self.sync_images(false);
        self.reanchor_rename(sibling);
        self.dismiss_stale_modal();
        if in_active {
            // Divider ids are paths in the active tree, which just changed.
            self.mouse.cancel_drag();
        }
        self.layout_changed();
        if removed {
            // Its tab's tree minimum may only have held for this pane.
            self.refresh_min_extents();
            // Every tab, not just the active one: in a background tab the pane
            // that took the closed one's place needs its new PTY size now.
            self.set_focus(self.tabs.active().focused);
            self.relayout_all();
        }
        self.evict_fonts();
        true
    }

    fn scroll_focused(&mut self, s: Scroll) {
        let focused = self.tabs.active().focused;
        if let Some(pane) = self.panes.get(&focused) {
            pane.scroll(s);
            self.mark_scrolled(focused);
            self.schedule();
        }
    }
}

/// Print every config error: the toast shows only the first.
fn log_errors(errors: &[ConfigError]) {
    for e in errors {
        eprintln!("montty: {e}");
    }
}

/// The config store, its files' errors and the startup toast. The files' errors make a
/// `ConfigReload` toast, which a reload that fixes them hides. A failed first-run write
/// makes it a `Startup` toast, which stays. Without a config dir (`$HOME` unset) montty
/// runs on built-in defaults and says so.
fn load_store(dir: Option<&Path>, now: Instant) -> (Store, Vec<ConfigError>, Option<Toast>) {
    let Some(dir) = dir else {
        return (
            Store::builtin(),
            Vec::new(),
            Some(Toast::new(NO_CONFIG_DIR, ToastSource::Startup, now)),
        );
    };
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|h| h.is_absolute());
    let (store, first_run, errors) = Store::load_split(dir, home.as_deref());
    log_errors(first_run.as_slice());
    log_errors(&errors);
    let toast = match first_run {
        Some(e) => {
            let all: Vec<ConfigError> = std::iter::once(e).chain(errors.clone()).collect();
            Toast::from_errors(&all, ToastSource::Startup, now)
        }
        None => Toast::from_errors(&errors, ToastSource::ConfigReload, now),
    };
    (store, errors, toast)
}

/// Watch the config dir `dir`; each change posts `UserEvent::ConfigChanged` from the
/// watcher's thread. The errors are everything that couldn't be watched (no watcher at
/// all, or each dir, with a message): config edits there need a restart.
fn start_watcher(
    dir: &Path,
    id: WindowId,
    sink: Arc<dyn EventSink>,
) -> (Option<Watcher>, Vec<String>) {
    let on_change = move || sink.send(UserEvent::ConfigChanged(id));
    match Watcher::start(on_change) {
        Ok(mut w) => {
            let errs = w.set_watched(&config_watch_dirs(dir));
            (Some(w), errs)
        }
        Err(e) => (None, vec![e]),
    }
}

/// The toast for watcher failures `errs` (the first, plus ` (+N more)`); the hint comes
/// first, where truncation can't hide it. None without errors.
fn watcher_toast(errs: &[String], now: Instant) -> Option<Toast> {
    let first = errs.first()?;
    Some(Toast::from_first(
        &format!("config watcher unavailable (edits need a restart): {first}"),
        errs.len(),
        ToastSource::Watcher,
        now,
    ))
}

/// The X11 window id (for the child's `WINDOWID`); None on Wayland.
#[allow(clippy::useless_conversion)] // Xlib's c_ulong is only u64 on 64-bit targets.
fn x11_window_id(w: &winit::window::Window) -> Option<u64> {
    match w.window_handle().ok()?.as_raw() {
        RawWindowHandle::Xlib(h) => Some(u64::from(h.window)),
        RawWindowHandle::Xcb(h) => Some(u64::from(h.window.get())),
        _ => None,
    }
}

fn window_size(cols: u16, lines: u16, m: &Metrics) -> WindowSize {
    WindowSize {
        num_cols: cols,
        num_lines: lines,
        cell_width: m.cell_width.min(u16::MAX as u32) as u16,
        cell_height: m.cell_height.min(u16::MAX as u32) as u16,
    }
}

fn same_size(a: &WindowSize, b: &WindowSize) -> bool {
    (a.num_cols, a.num_lines, a.cell_width, a.cell_height)
        == (b.num_cols, b.num_lines, b.cell_width, b.cell_height)
}

/// OSC 4/10/11/12 color index to a terminal color (0..=255 indexed, then fg/bg/cursor).
fn color_for_index(idx: usize) -> Color {
    match idx {
        0..=255 => Color::Indexed(idx as u8),
        257 => Color::Named(NamedColor::Background),
        258 => Color::Named(NamedColor::Cursor),
        _ => Color::Named(NamedColor::Foreground),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::SplitTree;

    fn metrics() -> Metrics {
        Metrics {
            cell_width: 8,
            cell_height: 16,
            baseline: 12,
        }
    }

    #[test]
    fn scrollbar_visible_rules() {
        let now = Instant::now();
        // Scrolled back: always.
        assert!(scrollbar_visible(3, None, now));
        // At the bottom: only within a second of the last scroll.
        assert!(!scrollbar_visible(0, None, now));
        assert!(scrollbar_visible(0, Some(now), now));
        assert!(scrollbar_visible(
            0,
            Some(now),
            now + Duration::from_millis(999)
        ));
        assert!(!scrollbar_visible(0, Some(now), now + SCROLLBAR_LINGER));
    }

    #[test]
    fn grid_size_fits_cells_inside_padding() {
        let r = Rect {
            x: 0,
            y: 0,
            w: 808,
            h: 408,
        };
        assert_eq!(grid_size(r, &metrics(), 4), (100, 25));
    }

    #[test]
    fn pane_min_is_one_cell_plus_padding() {
        let m = Metrics {
            cell_width: 20,
            cell_height: 40,
            baseline: 30,
        };
        assert_eq!(pane_min_extent(&m, 4), (28, 48));
        // Tiny cells: below MIN_PANE_PX, which `SplitTree::set_min_extent` raises them to.
        assert_eq!(pane_min_extent(&metrics(), 4), (16, 24));
    }

    #[test]
    fn split_refused_below_two_cells_plus_padding() {
        let m = Metrics {
            cell_width: 20,
            cell_height: 40,
            baseline: 30,
        };
        let (w, h) = pane_min_extent(&m, 4);
        let mut t = SplitTree::new(1);
        t.set_min_extent(w, h);
        let area = |w, h| Rect { x: 0, y: 0, w, h };
        // 20 + 8 is below the 32px floor, so width uses the floor.
        let o = crate::layout::Orientation::Horizontal;
        assert!(!t.can_split(1, o, area(2 * 32, 500)));
        assert!(t.can_split(1, o, area(2 * 32 + 1, 500)));
        let o = crate::layout::Orientation::Vertical;
        assert!(!t.can_split(1, o, area(500, 2 * (40 + 8))));
        assert!(t.can_split(1, o, area(500, 2 * (40 + 8) + 1)));
        t.split(1, o, 2);
        for (_, r) in t.layout(area(500, 2 * (40 + 8) + 1)) {
            assert_eq!(grid_size(r, &m, 4).1, 1);
            assert!(r.h >= 40 + 8, "{r:?}");
        }
    }

    #[test]
    fn tab_min_extent_is_max_over_panes() {
        assert_eq!(tab_min_extent([(28, 48)].into_iter()), (28, 48));
        // Per axis: the widest cell and the tallest cell may come from different panes.
        assert_eq!(
            tab_min_extent([(28, 48), (40, 30), (10, 10)].into_iter()),
            (40, 48)
        );
        // No panes: nothing required (the tree's MIN_PANE_PX floor applies).
        assert_eq!(tab_min_extent(std::iter::empty()), (0, 0));
    }

    #[test]
    fn live_font_keys_cover_every_tab_and_the_ui() {
        let keys = live_keys(1, [5, 7, 5].into_iter());
        assert_eq!(keys, HashSet::from([1, 5, 7]));
        assert_eq!(live_keys(3, std::iter::empty()), HashSet::from([3]));
    }

    #[test]
    fn missing_font_falls_back() {
        let mut cache = FontCache::new(system_db());
        let mono = cache.get("monospace", 13.0, 1.0).unwrap().key();
        let (key, err) = load_font_or_fallback(&mut cache, "NoSuchFont", 13.0, 1.0).unwrap();
        assert_eq!(key, mono, "monospace at the profile's size");
        assert!(err.unwrap().contains("'NoSuchFont'"));
        let (key, err) = load_font_or_fallback(&mut cache, "monospace", 13.0, 1.0).unwrap();
        assert_eq!((key, err), (mono, None));
    }

    #[test]
    fn zoom_stays_within_size_limits() {
        assert_eq!(next_zoom(11.0, 0.0, Some(1.0)), 1.0);
        assert_eq!(next_zoom(11.0, 3.0, None), 0.0);
        // 11 - 7 = 4 pt is the floor: zooming out further changes nothing.
        assert_eq!(next_zoom(11.0, -7.0, Some(-1.0)), -7.0);
        assert_eq!(next_zoom(199.5, 0.0, Some(1.0)), 0.5);
        assert_eq!(effective_size(11.0, -20.0), 4.0);
        assert_eq!(effective_size(150.0, 60.0), 200.0);
        assert_eq!(effective_size(11.0, 2.0), 13.0);
    }

    fn argv(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    /// A fake spawner: programs in `ok` start, the rest fail with their name; records tries.
    fn fake<'a>(
        ok: &'a [&str],
        tried: &'a mut Vec<(Vec<String>, bool)>,
    ) -> impl FnMut(&[String], bool) -> std::io::Result<String> + 'a {
        move |a, fb| {
            tried.push((a.to_vec(), fb));
            match ok.contains(&a[0].as_str()) {
                true => Ok(a[0].clone()),
                false => Err(std::io::Error::other(format!("no {}", a[0]))),
            }
        }
    }

    #[test]
    fn spawn_works_first_time_without_fallback() {
        let mut tried = Vec::new();
        let r = spawn_with_fallback(
            &argv(&["zsh", "-l"]),
            true,
            Some("/bin/bash"),
            fake(&["zsh"], &mut tried),
        );
        assert_eq!(r.unwrap(), ("zsh".to_string(), None));
        assert_eq!(tried, [(argv(&["zsh", "-l"]), false)]);
    }

    #[test]
    fn spawn_falls_back_to_env_shell_then_sh() {
        let mut tried = Vec::new();
        let r = spawn_with_fallback(
            &argv(&["zhs", "-l"]),
            true,
            Some("/bin/bash"),
            fake(&["/bin/bash", "/bin/sh"], &mut tried),
        );
        let (p, fb) = r.unwrap();
        assert_eq!(p, "/bin/bash");
        assert_eq!(
            fb,
            Some(Fallback {
                failed: "zhs".into(),
                err: "no zhs".into(),
                used: "/bin/bash".into()
            })
        );
        // The fallback is a plain shell: no `-l`, no arguments.
        assert_eq!(
            tried,
            [(argv(&["zhs", "-l"]), false), (argv(&["/bin/bash"]), true)]
        );
        // $SHELL broken too: /bin/sh.
        let mut tried = Vec::new();
        let r = spawn_with_fallback(
            &argv(&["zhs"]),
            true,
            Some("/bin/bash"),
            fake(&["/bin/sh"], &mut tried),
        );
        assert_eq!(r.unwrap().1.unwrap().used, "/bin/sh");
        assert_eq!(tried.len(), 3);
        // No $SHELL (or an empty one): straight to /bin/sh.
        for env in [None, Some("")] {
            let mut tried = Vec::new();
            let r = spawn_with_fallback(&argv(&["zhs"]), true, env, fake(&["/bin/sh"], &mut tried));
            assert_eq!(r.unwrap().1.unwrap().used, "/bin/sh");
            assert_eq!(tried.len(), 2, "{env:?}");
        }
    }

    #[test]
    fn spawn_fallback_skips_what_failed() {
        // $SHELL is what just failed (a profile `shell` equal to it): not tried again.
        let mut tried = Vec::new();
        let r = spawn_with_fallback(
            &argv(&["/bin/zsh", "-l"]),
            true,
            Some("/bin/zsh"),
            fake(&["/bin/sh"], &mut tried),
        );
        assert_eq!(r.unwrap().1.unwrap().used, "/bin/sh");
        assert_eq!(
            tried,
            [
                (argv(&["/bin/zsh", "-l"]), false),
                (argv(&["/bin/sh"]), true)
            ]
        );
        // The failed program is /bin/sh, with $SHELL = /bin/sh too: tried once only.
        let mut tried = Vec::new();
        let r = spawn_with_fallback(
            &argv(&["/bin/sh"]),
            true,
            Some("/bin/sh"),
            fake(&[], &mut tried),
        );
        assert!(r.is_err());
        assert_eq!(tried.len(), 1);
        // $SHELL = /bin/sh and the program failed: /bin/sh once.
        let mut tried = Vec::new();
        let r = spawn_with_fallback(&argv(&["x"]), true, Some("/bin/sh"), fake(&[], &mut tried));
        assert!(r.is_err());
        assert_eq!(tried.len(), 2);
    }

    #[test]
    fn spawn_all_failed_reports_the_first_error() {
        let mut tried = Vec::new();
        let r = spawn_with_fallback(
            &argv(&["zhs"]),
            true,
            Some("/bin/bash"),
            fake(&[], &mut tried),
        );
        assert_eq!(r.unwrap_err().to_string(), "no zhs");
        assert_eq!(tried.len(), 3);
    }

    #[test]
    fn spawn_dash_e_never_falls_back() {
        let mut tried = Vec::new();
        let r = spawn_with_fallback(
            &argv(&["htop"]),
            false,
            Some("/bin/bash"),
            fake(&["/bin/bash", "/bin/sh"], &mut tried),
        );
        assert_eq!(r.unwrap_err().to_string(), "no htop");
        assert_eq!(tried.len(), 1);
    }

    #[test]
    fn watcher_toast_leads_with_the_hint() {
        let now = Instant::now();
        assert_eq!(watcher_toast(&[], now), None);
        let t = watcher_toast(&["/a: no such directory".into()], now).unwrap();
        assert_eq!(
            t.full(),
            "config watcher unavailable (edits need a restart): /a: no such directory"
        );
        assert_eq!(t.source, ToastSource::Watcher);
        let t = watcher_toast(&["/a: x".into(), "/b: y".into(), "/c: z".into()], now).unwrap();
        assert_eq!(t.tail, " (+2 more)");
        assert!(
            t.text
                .starts_with("config watcher unavailable (edits need a restart): /a")
        );
    }

    #[test]
    fn window_stem_is_the_arg_else_the_default() {
        assert_eq!(window_stem_of(Some("work"), "default"), "work");
        assert_eq!(window_stem_of(None, "default"), "default");
    }

    #[test]
    fn unknown_stem_resolves_to_the_default() {
        // The font toast names the file the lookup resolved to, not the recorded stem.
        assert_eq!(resolved_stem_of(true, "work", "default"), "work");
        assert_eq!(resolved_stem_of(false, "nope", "default"), "default");
    }

    #[test]
    fn missing_profile_arg_is_reported() {
        assert_eq!(profile_arg_problem(None, false, "default"), None);
        assert_eq!(profile_arg_problem(Some("work"), true, "default"), None);
        let (body, tail) = profile_arg_problem(Some("wrok"), false, "default").unwrap();
        assert_eq!(
            format!("{body}{tail}"),
            "--profile: no profile 'wrok' (using 'default')"
        );
        assert_eq!(tail, " (using 'default')");
    }

    #[test]
    fn waiting_error_shows_next() {
        let now = Instant::now();
        let image = Toast::new("image", ToastSource::Info, now);
        let called = Cell::new(false);
        let t = next_toast(
            Some("boom".into()),
            || {
                called.set(true);
                Some(image)
            },
            now,
        )
        .unwrap();
        assert_eq!(t.text, "boom");
        assert_eq!(t.source, ToastSource::Spawn);
        assert_eq!(t.until, now + crate::ui::toast::TOAST_DURATION);
        assert!(!called.get());
    }

    #[test]
    fn no_waiting_error_shows_images() {
        let now = Instant::now();
        let t = Toast::new("image", ToastSource::Info, now);
        assert_eq!(next_toast(None, || Some(t.clone()), now), Some(t));
    }

    #[test]
    fn error_toast_keeps_the_first_problem() {
        assert!(error_replaces(None));
        assert!(error_replaces(Some(ToastSource::Info)));
        assert!(!error_replaces(Some(ToastSource::ConfigReload)));
        assert!(!error_replaces(Some(ToastSource::Startup)));
        assert!(!error_replaces(Some(ToastSource::Spawn)));
    }

    #[test]
    fn fallback_toast_text() {
        let f = Fallback {
            failed: "/usr/bin/zhs".into(),
            err: "No such file or directory (os error 2)".into(),
            used: "/bin/sh".into(),
        };
        let (body, tail) = fallback_text(Some("work"), &f);
        assert_eq!(
            body,
            "profiles/work.toml: can't start '/usr/bin/zhs': No such file or directory (os error 2)"
        );
        assert_eq!(tail, " (using /bin/sh)");
        // A command isn't a profile file's doing.
        let (body, _) = fallback_text(None, &f);
        assert_eq!(
            body,
            "can't start '/usr/bin/zhs': No such file or directory (os error 2)"
        );
    }

    #[test]
    fn nonexistent_profile_shell_runs_bin_sh() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let p = Profile {
            shell: Some("/nonexistent/zhs".into()),
            login_shell: true,
            ..Profile::default()
        };
        let opts = pane_options(&p, None, None, None, None);
        let size = WindowSize {
            num_cols: 20,
            num_lines: 5,
            cell_width: 8,
            cell_height: 16,
        };
        let sink: Arc<dyn EventSink> = Arc::new(tx);
        let (pane, fb) = spawn_pane_process(7, &opts, size, &sink, true, None).unwrap();
        let fb = fb.expect("fell back");
        assert_eq!(
            (fb.failed.as_str(), fb.used.as_str()),
            ("/nonexistent/zhs", "/bin/sh")
        );
        assert!(fb.err.contains("No such file"), "{}", fb.err);
        assert!(!pane.runs_command(), "a fallback shell is a shell");
        let cmdline = std::fs::read(format!("/proc/{}/cmdline", pane.shell_pid())).unwrap();
        assert!(cmdline.starts_with(b"/bin/sh\0"), "{cmdline:?}");
        // -e: no fallback.
        let e = pane_options(&p, Some(argv(&["/nonexistent/prog"])), None, None, None);
        assert!(spawn_pane_process(8, &e, size, &sink, false, None).is_err());
    }

    #[test]
    fn pane_options_sets_working_dir() {
        let p = Profile::default();
        let o = pane_options(&p, None, None, None, Some(PathBuf::from("/tmp")));
        assert_eq!(o.working_dir, Some(PathBuf::from("/tmp")));
        assert_eq!(pane_options(&p, None, None, None, None).working_dir, None);
    }

    #[test]
    fn split_without_cwd_uses_default() {
        let o = pane_options(&Profile::default(), None, Some("/bin/zsh"), Some(1), None);
        assert_eq!(o.working_dir, None);
    }

    #[test]
    fn pane_options_from_profile() {
        let mut p = Profile {
            scrollback: 500,
            cursor_shape: CursorShapeCfg::Beam,
            cursor_blink: true,
            login_shell: true,
            ..Profile::default()
        };
        let o = pane_options(&p, None, Some("/bin/zsh"), Some(7), None);
        assert_eq!(o.command, Some(vec!["/bin/zsh".into(), "-l".into()]));
        assert!(!o.runs_command, "a shell");
        assert_eq!((o.scrollback, o.window_id), (500, Some(7)));
        assert_eq!(
            (o.cursor.shape, o.cursor.blinking),
            (CursorShape::Beam, true)
        );
        // -e wins over the profile.
        let o = pane_options(&p, Some(vec!["htop".into()]), Some("/bin/zsh"), None, None);
        assert_eq!(o.command, Some(vec!["htop".into()]));
        assert!(o.runs_command);
        // A profile command is a command too.
        p.command = Some(vec!["top".into(), "-d1".into()]);
        let o = pane_options(&p, None, None, None, None);
        assert_eq!(o.command, Some(vec!["top".into(), "-d1".into()]));
        assert!(o.runs_command);
        // Plan 2 defaults: $SHELL, block cursor without blink, 10 000 lines.
        let o = pane_options(&Profile::default(), None, None, None, None);
        assert_eq!(o.command, Some(vec!["/bin/sh".into()]));
        assert_eq!(o.scrollback, 10_000);
        assert_eq!(
            (o.cursor.shape, o.cursor.blinking),
            (CursorShape::Block, false)
        );
    }

    fn styles(stems: &[(PaneId, &str)]) -> HashMap<PaneId, PaneStyle> {
        stems
            .iter()
            .map(|&(id, stem)| {
                let style = PaneStyle {
                    stem: stem.to_string(),
                    zoom_pt: 0.0,
                };
                (id, style)
            })
            .collect()
    }

    fn report(changed: &[&str], removed: &[&str], added: &[&str]) -> ReloadReport {
        let v = |s: &[&str]| s.iter().map(|s| s.to_string()).collect();
        ReloadReport {
            changed: v(changed),
            removed: v(removed),
            added: v(added),
            ..ReloadReport::default()
        }
    }

    const PANES: [(PaneId, &str); 5] = [(1, "a"), (2, "b"), (3, "default"), (4, "a"), (5, "b")];

    /// `a`, `b` and `default` exist.
    fn known(stem: &str) -> bool {
        ["a", "b", "default"].contains(&stem)
    }

    #[test]
    fn affected_by_change() {
        let r = report(&["a"], &[], &[]);
        assert_eq!(
            affected_panes(&styles(&PANES), &r, false, "default", known),
            [1, 4]
        );
        let r = report(&[], &[], &[]);
        assert!(affected_panes(&styles(&PANES), &r, false, "default", known).is_empty());
    }

    #[test]
    fn pane_falls_back_when_profile_removed() {
        let gone = |s: &str| s != "b" && known(s);
        let r = report(&[], &["b"], &[]);
        assert_eq!(
            affected_panes(&styles(&PANES), &r, false, "default", gone),
            [2, 5]
        );
        // The pane keeps its stem: the file coming back brings it back.
        let r = report(&[], &[], &["b"]);
        assert_eq!(
            affected_panes(&styles(&PANES), &r, false, "default", known),
            [2, 5]
        );
        // Falling back, `b` panes follow edits to the default.
        let r = report(&["default"], &[], &[]);
        assert_eq!(
            affected_panes(&styles(&PANES), &r, false, "default", gone),
            [2, 3, 5]
        );
    }

    #[test]
    fn default_switch_affects_default_users() {
        // config.toml's default_profile went from `default` to `a`: the panes whose
        // profile is unknown (`x`) resolve to the new default, as do `a`'s.
        let panes = [(1, "a"), (2, "b"), (3, "default"), (6, "x")];
        let r = ReloadReport {
            global_changed: true,
            ..ReloadReport::default()
        };
        assert_eq!(
            affected_panes(&styles(&panes), &r, true, "a", known),
            [1, 6]
        );
        // Another global change (a color) moves no pane.
        assert!(affected_panes(&styles(&panes), &r, false, "default", known).is_empty());
    }

    fn resolved(p: Profile) -> Resolved {
        Resolved {
            profile: p,
            palette: crate::term::colors::tango_default(),
        }
    }

    #[test]
    fn profile_delta_does_only_what_changed() {
        let old = resolved(Profile::default());
        let none = ProfileDelta::default();
        assert_eq!(profile_delta(&old, &old), none);
        let mut colors = resolved(Profile::default());
        colors.palette.bg.r ^= 1;
        let redraw = ProfileDelta {
            redraw: true,
            ..none
        };
        assert_eq!(profile_delta(&old, &colors), redraw);
        let bold = resolved(Profile {
            bold_is_bright: !old.profile.bold_is_bright,
            ..Profile::default()
        });
        assert_eq!(profile_delta(&old, &bold), redraw);
        for p in [
            Profile {
                font_size: 14.0,
                ..Profile::default()
            },
            Profile {
                font_family: "serif".into(),
                ..Profile::default()
            },
            Profile {
                padding: 9,
                ..Profile::default()
            },
        ] {
            let d = profile_delta(&old, &resolved(p));
            assert!(d.font && !d.term, "{d:?}");
        }
        for p in [
            Profile {
                scrollback: 10,
                ..Profile::default()
            },
            Profile {
                cursor_shape: CursorShapeCfg::Beam,
                ..Profile::default()
            },
            Profile {
                cursor_blink: true,
                ..Profile::default()
            },
        ] {
            let d = profile_delta(&old, &resolved(p));
            assert!(d.term && !d.font, "{d:?}");
        }
        // The command and shell only apply to new panes.
        let spawn = resolved(Profile {
            command: Some(vec!["htop".into()]),
            shell: Some("/bin/zsh".into()),
            login_shell: true,
            ..Profile::default()
        });
        assert_eq!(profile_delta(&old, &spawn), none);
    }

    #[test]
    fn reported_fonts_looked_up_by_borrow() {
        let mut r = ReportedFonts::default();
        assert!(!r.contains("work", "Foo"));
        r.insert("work", "Foo");
        assert!(r.contains("work", "Foo"));
        assert!(!r.contains("work", "Bar"), "another family in that profile");
        assert!(!r.contains("other", "Foo"));
    }

    #[test]
    fn synthetic_presses_are_ignored() {
        assert!(handles_key(ElementState::Pressed, false));
        // Keys held while the window gains focus (e.g. the Ctrl+Alt+T that launched us).
        assert!(!handles_key(ElementState::Pressed, true));
        assert!(!handles_key(ElementState::Released, false));
    }

    fn three_tabs() -> Tabs {
        let mut tabs = Tabs::new(Tab::new(1, (32, 32)));
        let mut t = Tab::new(2, (32, 32));
        t.tree.split(2, crate::layout::Orientation::Horizontal, 3);
        tabs.push(t);
        tabs.push(Tab::new(4, (32, 32)));
        tabs
    }

    #[test]
    fn title_counts_for_any_tabs_focused_pane() {
        let mut tabs = three_tabs();
        assert!(labels_a_tab(&tabs, 1), "background tab's focused pane");
        assert!(labels_a_tab(&tabs, 2));
        assert!(!labels_a_tab(&tabs, 3), "unfocused pane");
        assert!(labels_a_tab(&tabs, 4), "active tab");
        assert!(!labels_a_tab(&tabs, 9));
        // A renamed tab's label ignores its pane's title.
        tabs.active_mut().title = Some("logs".into());
        assert!(!labels_a_tab(&tabs, 4));
    }

    #[test]
    fn same_title_is_no_change() {
        let mut t = None;
        assert!(!replace_title(&mut t, None));
        assert!(replace_title(&mut t, Some("vim".into())));
        assert!(!replace_title(&mut t, Some("vim".into())), "re-sent");
        assert_eq!(t.as_deref(), Some("vim"));
        assert!(replace_title(&mut t, Some("bash".into())));
        assert!(replace_title(&mut t, None), "reset");
        assert!(!replace_title(&mut t, None));
        assert_eq!(t, None);
    }

    #[test]
    fn all_layouts_covers_hidden_tabs() {
        let mut tabs = three_tabs();
        tabs.iter_mut().nth(1).unwrap().tree.toggle_maximize(3);
        let area = Rect {
            x: 0,
            y: 0,
            w: 200,
            h: 100,
        };
        let mut ids: Vec<PaneId> = all_layouts(&tabs, area).iter().map(|&(id, _)| id).collect();
        ids.sort();
        // A maximized tab lays out only its maximized pane, as `relayout` does.
        assert_eq!(ids, [1, 3, 4]);
        assert!(all_layouts(&tabs, area).iter().all(|&(_, r)| r == area));
    }

    #[test]
    fn background_pane_exit_resizes_its_sibling() {
        // Tab 2 is [2 | 3] in the background; pane 3 exits. The active tab's
        // layout alone misses pane 2; `all_layouts` (relayout_all) gives it
        // the whole area.
        let mut tabs = three_tabs();
        let area = Rect {
            x: 0,
            y: 0,
            w: 200,
            h: 100,
        };
        let half = all_layouts(&tabs, area)
            .into_iter()
            .find(|&(id, _)| id == 2)
            .unwrap()
            .1;
        assert!(half.w < area.w);
        assert!(actions::remove_pane(&mut tabs, 3, area));
        let active: Vec<PaneId> = tabs
            .active()
            .tree
            .layout(area)
            .iter()
            .map(|&(id, _)| id)
            .collect();
        assert_eq!(active, [4], "the active tab doesn't hold pane 2");
        assert!(all_layouts(&tabs, area).contains(&(2, area)));
    }

    #[test]
    fn grid_size_never_zero() {
        let r = Rect {
            x: 0,
            y: 0,
            w: 3,
            h: 3,
        };
        assert_eq!(grid_size(r, &metrics(), 4), (1, 1));
    }
}
