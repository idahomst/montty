//! macOS: montty's bar sits over the window's (invisible) native title bar. AppKit treats a
//! press on a view that "can move the window" as a title bar press, so two quick presses on
//! a bar button zoomed the window. winit's view keeps NSView's default (yes, for a view that
//! is not opaque), so montty teaches the class to say no. Moving the window and double-click
//! to maximize stay montty's own (`drag_window`, the bar's double-click).

use std::ffi::{c_char, c_void};

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> *mut c_void;
    fn sel_registerName(name: *const c_char) -> *const c_void;
    fn class_addMethod(
        class: *mut c_void,
        sel: *const c_void,
        imp: *const c_void,
        types: *const c_char,
    ) -> bool;
}

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
    // SAFETY: plain Objective-C runtime calls with NUL-terminated names; the method's
    // signature matches its type encoding (BOOL return, self and _cmd arguments).
    unsafe {
        let class = objc_getClass(c"WinitView".as_ptr());
        if class.is_null() {
            eprintln!("montty: no WinitView class; bar clicks may zoom the window");
            return;
        }
        let sel = sel_registerName(c"mouseDownCanMoveWindow".as_ptr());
        let imp = cannot_move_window as extern "C" fn(*mut c_void, *const c_void) -> bool;
        class_addMethod(class, sel, imp as *const c_void, TYPES.as_ptr());
    }
}
