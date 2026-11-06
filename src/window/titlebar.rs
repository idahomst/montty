//! macOS: montty's bar sits over the window's (invisible) native title bar. AppKit treats a
//! press on a view that "can move the window" as a title bar press, so two quick presses on
//! a bar button zoomed the window. winit's view keeps NSView's default (yes, for a view that
//! is not opaque), so montty teaches the class to say no. Moving the window and double-click
//! to maximize stay montty's own (`drag_window`, the bar's double-click).

use std::ffi::c_void;

/// `-[WinitView mouseDownCanMoveWindow]`: NO. A one-byte BOOL either way (`signed char` on
/// x86_64, `bool` on arm64).
extern "C" fn cannot_move_window(_this: *mut c_void, _cmd: *const c_void) -> bool {
    false
}

#[cfg(target_arch = "aarch64")]
const TYPES: &std::ffi::CStr = c"B@:";
#[cfg(not(target_arch = "aarch64"))]
const TYPES: &std::ffi::CStr = c"c@:";

/// Call once a window exists (winit registers its view class with the first window).
/// Later calls do nothing: the class already has the method.
pub(super) fn keep_bar_presses() {
    let imp = cannot_move_window as extern "C" fn(*mut c_void, *const c_void) -> bool;
    // SAFETY: `cannot_move_window` matches TYPES (BOOL return, self and _cmd).
    let added = unsafe {
        crate::objc::add_method(
            c"WinitView",
            c"mouseDownCanMoveWindow",
            imp as *const c_void,
            TYPES,
        )
    };
    // False on every window after the first (the class has the method by then); only a missing
    // class is worth a line, and add_method can't tell the two apart, so stay quiet.
    let _ = added;
}
