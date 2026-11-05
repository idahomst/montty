//! The window manager's window menu (always on top, workspaces, move, resize...),
//! which a WM normally shows on a right click on its own title bar. montty draws
//! its own bar, so it asks the WM to show the menu, as GTK does for Tilix.

use winit::dpi::PhysicalPosition;
use winit::window::Window;

/// Show the WM's window menu at `pos` (window pixels), from the handler of the
/// button press that asked for it. Nothing happens when the WM has no such menu.
pub fn show(window: &Window, pos: PhysicalPosition<i32>) {
    #[cfg(not(target_os = "macos"))]
    x11_wayland::show(window, pos);
    // macOS has no window manager menu to ask for.
    #[cfg(target_os = "macos")]
    let _ = (window, pos);
}

#[cfg(not(target_os = "macos"))]
mod x11_wayland {
    use std::ffi::CStr;
    use std::os::raw::{c_int, c_long, c_uchar, c_ulong};
    use std::ptr;

    use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
    use winit::dpi::PhysicalPosition;
    use winit::window::Window;
    use x11_dl::xinput2::XInput2;
    use x11_dl::xlib::{
        self, ClientMessage, Display, SubstructureNotifyMask, SubstructureRedirectMask, XA_ATOM,
        XClientMessageEvent, XEvent, Xlib,
    };

    pub(super) fn show(window: &Window, pos: PhysicalPosition<i32>) {
        let (Ok(dh), Ok(wh)) = (window.display_handle(), window.window_handle()) else {
            return;
        };
        match (dh.as_raw(), wh.as_raw()) {
            (RawDisplayHandle::Xlib(d), RawWindowHandle::Xlib(w)) => {
                let Some(display) = d.display else {
                    return;
                };
                if let Err(e) = show_x11(display.as_ptr().cast(), d.screen, w.window, pos) {
                    eprintln!("montty: window menu: {e}");
                }
            }
            // xdg_toplevel.show_window_menu; winit's docs still call it unsupported.
            (RawDisplayHandle::Wayland(_), _) => window.show_window_menu(pos),
            _ => {}
        }
    }

    const SHOW_WINDOW_MENU: &CStr = c"_GTK_SHOW_WINDOW_MENU";

    /// GTK's protocol, understood by Muffin, Mutter, Metacity and KWin: drop the
    /// implicit grab of the press (the WM must grab the pointer for its menu), then
    /// send `_GTK_SHOW_WINDOW_MENU` with the device and the root coordinates. Only
    /// when the WM lists the atom in `_NET_SUPPORTED`; the others would ignore it.
    fn show_x11(
        display: *mut Display,
        screen: c_int,
        window: c_ulong,
        pos: PhysicalPosition<i32>,
    ) -> Result<(), String> {
        let xlib = Xlib::open().map_err(|e| e.to_string())?;
        let xi = XInput2::open().map_err(|e| e.to_string())?;
        // SAFETY: `display` is winit's live Xlib connection, used on the event loop's
        // thread; every out pointer is a local, and the property data is freed once read.
        unsafe {
            let root = (xlib.XRootWindow)(display, screen);
            let atom = (xlib.XInternAtom)(display, SHOW_WINDOW_MENU.as_ptr(), xlib::False);
            if !wm_supports(&xlib, display, root, atom) {
                return Ok(());
            }
            let (mut x, mut y, mut child) = (0, 0, 0);
            (xlib.XTranslateCoordinates)(
                display, window, root, pos.x, pos.y, &mut x, &mut y, &mut child,
            );
            // winit selects XInput2 button events, so the press grabbed the
            // client pointer through XI2; the WM ignores the device id we send.
            let mut device: c_int = 0;
            (xi.XIGetClientPointer)(display, 0, &mut device);
            (xi.XIUngrabDevice)(display, device, xlib::CurrentTime);
            (xlib.XUngrabPointer)(display, xlib::CurrentTime);
            let mut msg = XClientMessageEvent {
                type_: ClientMessage,
                serial: 0,
                send_event: xlib::True,
                display,
                window,
                message_type: atom,
                format: 32,
                data: Default::default(),
            };
            let longs = msg.data.as_longs_mut();
            longs[0] = c_long::from(device);
            longs[1] = c_long::from(x);
            longs[2] = c_long::from(y);
            let mut ev = XEvent::from(msg);
            (xlib.XSendEvent)(
                display,
                root,
                xlib::False,
                SubstructureRedirectMask | SubstructureNotifyMask,
                &mut ev,
            );
            (xlib.XFlush)(display);
        }
        Ok(())
    }

    /// Whether the root's `_NET_SUPPORTED` list holds `atom`.
    ///
    /// # Safety
    /// `display` must be a live Xlib connection and `root` its root window.
    unsafe fn wm_supports(
        xlib: &Xlib,
        display: *mut Display,
        root: c_ulong,
        atom: c_ulong,
    ) -> bool {
        let (mut kind, mut format, mut count, mut after) = (0, 0, 0, 0);
        let mut data: *mut c_uchar = ptr::null_mut();
        // SAFETY: per the contract; out pointers are locals.
        let ok = unsafe {
            let supported = (xlib.XInternAtom)(display, c"_NET_SUPPORTED".as_ptr(), xlib::False);
            (xlib.XGetWindowProperty)(
                display,
                root,
                supported,
                0,
                c_long::MAX / 4,
                xlib::False,
                XA_ATOM,
                &mut kind,
                &mut format,
                &mut count,
                &mut after,
                &mut data,
            )
        } == xlib::Success as c_int;
        if data.is_null() {
            return false;
        }
        // Format 32 properties come back as an array of C longs.
        let found = ok && kind == XA_ATOM && format == 32 && {
            // SAFETY: Xlib returned `count` atoms at `data`.
            let atoms =
                unsafe { std::slice::from_raw_parts(data.cast::<c_ulong>(), count as usize) };
            atoms.contains(&atom)
        };
        // SAFETY: `data` was allocated by Xlib.
        unsafe { (xlib.XFree)(data.cast()) };
        found
    }
}
