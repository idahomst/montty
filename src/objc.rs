//! The few Objective-C runtime calls montty needs on macOS to change how winit's classes answer
//! AppKit (see `window/titlebar.rs`, `macos_quit.rs`), declared against libobjc instead of pulling
//! in a binding crate.

use std::ffi::{CStr, c_char, c_void};

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

/// Add method `sel` (implemented by `imp`, with type encoding `types`) to class `class`. False
/// when the class doesn't exist (yet), or already has its own `sel`.
///
/// # Safety
/// `imp` must be an `extern "C"` function whose signature matches `types`.
pub(crate) unsafe fn add_method(
    class: &CStr,
    sel: &CStr,
    imp: *const c_void,
    types: &CStr,
) -> bool {
    // SAFETY: NUL-terminated names; the caller vouches for `imp` and `types`.
    unsafe {
        let class = objc_getClass(class.as_ptr());
        if class.is_null() {
            return false;
        }
        class_addMethod(class, sel_registerName(sel.as_ptr()), imp, types.as_ptr())
    }
}
