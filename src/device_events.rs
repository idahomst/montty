//! montty never uses winit device events (raw input from every device, wherever
//! the pointer is). Receiving them wakes the event loop for every mouse motion
//! anywhere on screen, so an idle window would not be idle.

use std::os::raw::c_int;

use raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
use winit::event_loop::{DeviceEvents, EventLoop};
use x11_dl::xinput2::{XIAllMasterDevices, XIEventMask, XInput2};
use x11_dl::xlib::{Display, Xlib};

/// Stop winit from delivering device events. Call before `run_app`.
pub fn ignore<T>(el: &EventLoop<T>) {
    el.listen_device_events(DeviceEvents::Never);
    // winit 0.30 (X11) selects raw XInput2 events on the root window when the
    // event loop is created and re-applies `listen_device_events` only on a
    // focus change, so a window that is never focused keeps receiving them.
    // Clear that selection now, on winit's own connection.
    if let Ok(handle) = el.display_handle()
        && let RawDisplayHandle::Xlib(h) = handle.as_raw()
        && let Some(display) = h.display
        && let Err(e) = clear_x11_raw_events(display.as_ptr().cast(), h.screen)
    {
        eprintln!("montty: cannot deselect X11 device events: {e}");
    }
}

/// Replace this client's XInput2 selection on the root window for all master
/// devices with an empty mask, as winit itself does on a focus change.
fn clear_x11_raw_events(display: *mut Display, screen: c_int) -> Result<(), String> {
    let xlib = Xlib::open().map_err(|e| e.to_string())?;
    let xi = XInput2::open().map_err(|e| e.to_string())?;
    let mut bits = [0u8; 4];
    let mut mask = XIEventMask {
        deviceid: XIAllMasterDevices,
        mask_len: bits.len() as c_int,
        mask: bits.as_mut_ptr(),
    };
    // SAFETY: `display` is winit's live Xlib connection (the event loop outlives
    // this call), used on the event loop's thread; `mask` points to `bits`.
    unsafe {
        let root = (xlib.XRootWindow)(display, screen);
        (xi.XISelectEvents)(display, root, &mut mask, 1);
        (xlib.XFlush)(display);
    }
    Ok(())
}
