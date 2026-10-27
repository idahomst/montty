//! OS window + OpenGL context and surface creation (GL 3.3 core, GLES 3.0 fallback).

use std::num::NonZeroU32;

use glutin::config::{Config, ConfigTemplateBuilder, GlConfig};
use glutin::context::{
    ContextApi, ContextAttributesBuilder, GlProfile, NotCurrentGlContext, PossiblyCurrentContext,
    Version,
};
use glutin::display::{Display, DisplayApiPreference, GetGlDisplay, GlDisplay};
use glutin::surface::{GlSurface, Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowAttributes};

pub(super) type GlWindow = (
    Window,
    Surface<WindowSurface>,
    PossiblyCurrentContext,
    glow::Context,
);

/// Prefer configs with transparency support, then fewer MSAA samples.
fn pick(configs: Box<dyn Iterator<Item = Config> + '_>) -> Option<Config> {
    let key = |c: &Config| {
        (
            c.supports_transparency().unwrap_or(false),
            std::cmp::Reverse(c.num_samples()),
        )
    };
    configs.reduce(|best, c| if key(&c) > key(&best) { c } else { best })
}

pub(super) fn create(el: &ActiveEventLoop, attrs: WindowAttributes) -> Result<GlWindow, String> {
    // Same bootstrap as glutin_winit::DisplayBuilder (GLX, falling back to EGL), but done
    // by hand so an empty config list is an error instead of a panic in the picker.
    let handle = el
        .display_handle()
        .map_err(|e| format!("display handle: {e}"))?
        .as_raw();
    let pref =
        DisplayApiPreference::GlxThenEgl(Box::new(winit::platform::x11::register_xlib_error_hook));
    let display = unsafe { Display::new(handle, pref) }
        .map_err(|e| format!("cannot open OpenGL display: {e}"))?;
    let template = ConfigTemplateBuilder::new()
        .with_alpha_size(8)
        .with_transparency(true)
        .build();
    let configs = unsafe { display.find_configs(template) }
        .map_err(|e| format!("no suitable OpenGL config: {e}"))?;
    let config = pick(configs).ok_or("no suitable OpenGL config (8-bit alpha, transparency)")?;
    let window = glutin_winit::finalize_window(el, attrs, &config)
        .map_err(|e| format!("cannot create window: {e}"))?;
    let raw = window
        .window_handle()
        .map_err(|e| format!("window handle: {e}"))?
        .as_raw();

    let display = config.display();
    let core = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
        .with_profile(GlProfile::Core)
        .build(Some(raw));
    let gles = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::Gles(Some(Version::new(3, 0))))
        .build(Some(raw));
    let context = unsafe {
        display
            .create_context(&config, &core)
            .or_else(|_| display.create_context(&config, &gles))
    }
    .map_err(|e| format!("cannot create OpenGL 3.3 / GLES 3.0 context: {e}"))?;

    let size = window.inner_size();
    let nz = |v: u32| NonZeroU32::new(v.max(1)).expect("max(1) is non-zero");
    let surface_attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(
        raw,
        nz(size.width),
        nz(size.height),
    );
    let surface = unsafe { display.create_window_surface(&config, &surface_attrs) }
        .map_err(|e| format!("cannot create GL surface: {e}"))?;
    let context = context
        .make_current(&surface)
        .map_err(|e| format!("cannot make GL context current: {e}"))?;
    // Frame pacing is ours (FrameScheduler); never block in swap_buffers.
    if let Err(e) = surface.set_swap_interval(&context, SwapInterval::DontWait) {
        eprintln!("montty: warning: cannot disable vsync, swap_buffers may block: {e}");
    }
    let gl = unsafe { glow::Context::from_loader_function_cstr(|s| display.get_proc_address(s)) };
    Ok((window, surface, context, gl))
}
