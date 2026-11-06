//! macOS: the app menu's Quit (Cmd+Q), the Dock's Quit and logout send `terminate:`, which would
//! end the process with no close confirmation and no session autosave. montty answers the app
//! delegate's `applicationShouldTerminate:` with "cancel" and runs its own `quit` instead
//! (`UserEvent::Quit`). A logout is therefore cancelled once; the user logs out again after
//! montty has quit.

use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};

use winit::event_loop::EventLoopProxy;

use crate::events::UserEvent;

static PROXY: OnceLock<Mutex<EventLoopProxy<UserEvent>>> = OnceLock::new();

/// `NSTerminateCancel`.
const TERMINATE_CANCEL: usize = 0;

/// `-[WinitApplicationDelegate applicationShouldTerminate:]`.
extern "C" fn should_terminate(
    _this: *mut c_void,
    _cmd: *const c_void,
    _app: *mut c_void,
) -> usize {
    if let Some(p) = PROXY.get()
        && let Ok(p) = p.lock()
    {
        let _ = p.send_event(UserEvent::Quit);
    }
    TERMINATE_CANCEL
}

/// Route `terminate:` to montty's quit. Call once, after the event loop is built (winit
/// registers its delegate class then).
pub fn install(proxy: EventLoopProxy<UserEvent>) {
    let _ = PROXY.set(Mutex::new(proxy));
    let imp = should_terminate as extern "C" fn(*mut c_void, *const c_void, *mut c_void) -> usize;
    // SAFETY: `should_terminate` matches "Q@:@" (NSUInteger return; self, _cmd, sender).
    let added = unsafe {
        crate::objc::add_method(
            c"WinitApplicationDelegate",
            c"applicationShouldTerminate:",
            imp as *const c_void,
            c"Q@:@",
        )
    };
    if !added {
        eprintln!("montty: cannot hook the app menu's Quit; Cmd+Q quits without asking");
    }
}
