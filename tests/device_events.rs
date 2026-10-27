//! An idle montty must not wake up for input aimed at other windows: winit's
//! raw XInput2 device events (every mouse motion anywhere on screen) must not
//! be selected on the X11 root window. Needs an X11 display; skipped without one.

use std::os::raw::c_int;

use raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
use winit::event_loop::EventLoop;
use winit::platform::x11::EventLoopBuilderExtX11;
use x11_dl::xinput2::{
    XI_RawButtonPress, XI_RawButtonRelease, XI_RawKeyPress, XI_RawKeyRelease, XI_RawMotion,
    XIAllMasterDevices, XInput2,
};
use x11_dl::xlib::{Display, Xlib};

/// XI2 event types this client has selected on the root window for all master devices.
fn root_selection(el: &EventLoop<()>) -> Vec<i32> {
    let RawDisplayHandle::Xlib(h) = el.display_handle().unwrap().as_raw() else {
        panic!("not an Xlib display");
    };
    let display = h
        .display
        .expect("Xlib display pointer")
        .as_ptr()
        .cast::<Display>();
    let xlib = Xlib::open().unwrap();
    let xi = XInput2::open().unwrap();
    let mut selected = Vec::new();
    unsafe {
        let root = (xlib.XRootWindow)(display, h.screen);
        let mut n: c_int = 0;
        let masks = (xi.XIGetSelectedEvents)(display, root, &mut n);
        if !masks.is_null() {
            for m in std::slice::from_raw_parts(masks, n as usize) {
                if m.deviceid != XIAllMasterDevices {
                    continue;
                }
                let bytes = std::slice::from_raw_parts(m.mask, m.mask_len as usize);
                for (i, b) in bytes.iter().enumerate() {
                    for bit in 0..8 {
                        if b & (1 << bit) != 0 {
                            selected.push((i * 8 + bit) as i32);
                        }
                    }
                }
            }
            (xlib.XFree)(masks.cast());
        }
    }
    selected
}

#[test]
fn no_raw_device_events_selected_on_x11_root() {
    if std::env::var_os("DISPLAY").is_none() {
        eprintln!("skipped: no X11 DISPLAY");
        return;
    }
    let el = EventLoop::builder()
        .with_x11()
        .with_any_thread(true)
        .build()
        .unwrap();
    montty::device_events::ignore(&el);
    let raw = [
        XI_RawMotion,
        XI_RawButtonPress,
        XI_RawButtonRelease,
        XI_RawKeyPress,
        XI_RawKeyRelease,
    ];
    let selected = root_selection(&el);
    let leaked: Vec<i32> = selected
        .iter()
        .copied()
        .filter(|t| raw.contains(t))
        .collect();
    assert!(
        leaked.is_empty(),
        "raw XI2 events selected on root: {leaked:?} (all: {selected:?})"
    );
}
