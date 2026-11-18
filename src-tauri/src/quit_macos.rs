//! macOS: route `terminate:` (⌘Q, the Quit menu item, Dock → Quit) through the quit confirmation.
//!
//! tao's application delegate answers `applicationWillTerminate:` but not
//! `applicationShouldTerminate:`, so `NSApp terminate:` ends the process without Tauri ever
//! emitting `RunEvent::ExitRequested`. We add `applicationShouldTerminate:` to the delegate's
//! class at runtime: it cancels the termination and feeds a request into the quit state machine
//! (`quit.rs`), whose `quit` reply (or timeout) exits through `AppHandle::exit`. That path stops
//! tao's loop directly and never calls `terminate:` again, so there is no loop; the "allowed"
//! flag below is a second guard for it.
//!
//! A log out, restart or shut down of the system is never blocked.

use std::ffi::c_char;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use objc2::ffi::class_addMethod;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Sel};
use objc2::{msg_send, sel};
use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager};
use tauri::{AppHandle, Manager};

use crate::quit::Quit;

/// `NSTerminateCancel` / `NSTerminateNow` (`NSApplicationTerminateReply`).
const TERMINATE_CANCEL: usize = 0;
const TERMINATE_NOW: usize = 1;

/// `keyAEQuitReason` (`'why?'`) and the reasons sent for a system log out, restart or shut down
/// (`AERegistry.h`: `kAELogOut`, `kAEReallyLogOut`, `kAERestart`, `kAEShowRestartDialog`,
/// `kAEShutDown`, `kAEShowShutdownDialog`). A plain quit carries `kAEQuitAll` (`'quia'`) or none.
const KEY_AE_QUIT_REASON: u32 = fourcc(b"why?");
const SYSTEM_QUIT_REASONS: [u32; 6] = [
    fourcc(b"logo"),
    fourcc(b"rlgo"),
    fourcc(b"rest"),
    fourcc(b"rrst"),
    fourcc(b"shut"),
    fourcc(b"rsdn"),
];

const fn fourcc(code: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*code)
}

static HANDLE: OnceLock<AppHandle> = OnceLock::new();
static ALLOWED: AtomicBool = AtomicBool::new(false);

/// What `applicationShouldTerminate:` answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Let the process end.
    TerminateNow,
    /// Refuse and ask the user through the quit state machine.
    CancelAndAsk,
}

/// The whole decision, without AppKit. `allowed`: our own confirmed exit is under way.
/// `reason`: the quit reason of the Apple event being handled, if any.
pub fn decide(allowed: bool, reason: Option<u32>) -> Decision {
    if allowed || reason.is_some_and(|r| SYSTEM_QUIT_REASONS.contains(&r)) {
        Decision::TerminateNow
    } else {
        Decision::CancelAndAsk
    }
}

/// Lets the next `terminate:` through; called right before our own exit.
pub fn allow_terminate() {
    ALLOWED.store(true, Ordering::SeqCst);
}

/// The quit reason carried by the Apple event being handled (none for ⌘Q and the menu item).
fn current_quit_reason() -> Option<u32> {
    let event = NSAppleEventManager::sharedAppleEventManager().currentAppleEvent()?;
    // The sender may put the reason among the parameters or among the attributes: look in both,
    // parameters first. Missing it would cancel a system log out, which must never happen.
    quit_reason_of(&event, true).or_else(|| quit_reason_of(&event, false))
}

/// The code found under `keyAEQuitReason` among the event's parameters (`parameters`) or
/// attributes, if any.
fn quit_reason_of(event: &NSAppleEventDescriptor, parameters: bool) -> Option<u32> {
    // SAFETY: both getters take an `AEKeyword` (`u32`) and return a nullable descriptor; the code
    // getters return an `OSType` (`u32`). (The typed bindings need the objc2-core-services crate,
    // not worth a new one.)
    unsafe {
        let descriptor: Option<Retained<NSAppleEventDescriptor>> = if parameters {
            msg_send![event, paramDescriptorForKeyword: KEY_AE_QUIT_REASON]
        } else {
            msg_send![event, attributeDescriptorForKeyword: KEY_AE_QUIT_REASON]
        };
        let descriptor = descriptor?;
        // The reason is an enumeration code or a type code, depending on the sender.
        let enum_code: u32 = msg_send![&*descriptor, enumCodeValue];
        let type_code: u32 = msg_send![&*descriptor, typeCodeValue];
        [enum_code, type_code].into_iter().find(|code| *code != 0)
    }
}

extern "C-unwind" fn should_terminate(
    _this: &AnyObject,
    _sel: Sel,
    _sender: *mut AnyObject,
) -> usize {
    match decide(ALLOWED.load(Ordering::SeqCst), current_quit_reason()) {
        Decision::TerminateNow => TERMINATE_NOW,
        Decision::CancelAndAsk => {
            match HANDLE.get() {
                Some(handle) => {
                    if let Some(quit) = handle.try_state::<Quit>() {
                        quit.request(handle);
                    }
                }
                // Not installed yet: nobody could answer, so do not refuse.
                None => return TERMINATE_NOW,
            }
            TERMINATE_CANCEL
        }
    }
}

/// Adds `applicationShouldTerminate:` to the class of the application delegate. Never panics: on
/// any problem it logs a warning and leaves today's behaviour (quit without asking).
pub fn install(handle: &AppHandle) {
    if HANDLE.set(handle.clone()).is_err() {
        tracing::warn!("the macOS quit hook is already installed");
        return;
    }
    let Some(app_class) = AnyClass::get(c"NSApplication") else {
        tracing::warn!("NSApplication not found; ⌘Q and Dock → Quit will not ask");
        return;
    };
    // SAFETY: `sharedApplication` and `delegate` are plain getters on the main thread's NSApp.
    let delegate: *mut AnyObject = unsafe {
        let app: *mut AnyObject = msg_send![app_class, sharedApplication];
        if app.is_null() {
            std::ptr::null_mut()
        } else {
            msg_send![app, delegate]
        }
    };
    // SAFETY: a non-null delegate is a live Objective-C object.
    let Some(delegate) = (unsafe { delegate.as_ref() }) else {
        tracing::warn!("no application delegate; ⌘Q and Dock → Quit will not ask");
        return;
    };
    let class = delegate.class();
    let selector = sel!(applicationShouldTerminate:);
    if class.responds_to(selector) {
        tracing::warn!("the application delegate already handles applicationShouldTerminate:");
        return;
    }
    if !add_should_terminate(class) {
        tracing::warn!(
            "could not add applicationShouldTerminate:; ⌘Q and Dock → Quit will not ask"
        );
    }
}

/// Adds the method to `class`; false when the runtime refuses.
fn add_should_terminate(class: &AnyClass) -> bool {
    // Method type encoding: returns NSUInteger ('Q'), takes self, _cmd and an object.
    let types: &[u8] = b"Q@:@\0";
    // SAFETY: the implementation matches the encoding above; the class outlives the process.
    let added = unsafe {
        class_addMethod(
            class as *const AnyClass as *mut AnyClass,
            sel!(applicationShouldTerminate:),
            std::mem::transmute::<
                extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject) -> usize,
                unsafe extern "C-unwind" fn(),
            >(should_terminate),
            types.as_ptr().cast::<c_char>(),
        )
    };
    added.as_bool()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_quit_cancels_and_asks() {
        // ⌘Q and the menu item carry no Apple event; Dock → Quit carries kAEQuitAll.
        assert_eq!(decide(false, None), Decision::CancelAndAsk);
        assert_eq!(decide(false, Some(fourcc(b"quia"))), Decision::CancelAndAsk);
    }

    #[test]
    fn our_own_confirmed_exit_goes_through() {
        assert_eq!(decide(true, None), Decision::TerminateNow);
        assert_eq!(decide(true, Some(fourcc(b"quia"))), Decision::TerminateNow);
    }

    #[test]
    fn log_out_restart_and_shut_down_are_never_blocked() {
        for code in [b"logo", b"rlgo", b"rest", b"rrst", b"shut", b"rsdn"] {
            assert_eq!(decide(false, Some(fourcc(code))), Decision::TerminateNow);
        }
    }

    /// The method added at runtime is callable with the right signature and, with our own exit
    /// allowed, lets the termination through.
    #[test]
    fn the_added_method_answers_terminate_now_once_allowed() {
        use objc2::runtime::ClassBuilder;
        let builder = ClassBuilder::new(c"PortolanoQuitTestDelegate", objc2::class!(NSObject))
            .expect("a fresh class name");
        let class = builder.register();
        assert!(!class.responds_to(sel!(applicationShouldTerminate:)));
        assert!(add_should_terminate(class));
        assert!(class.responds_to(sel!(applicationShouldTerminate:)));
        allow_terminate();
        // SAFETY: NSObject subclass without ivars; the selector was just added.
        let reply: usize = unsafe {
            let object: *mut AnyObject = msg_send![class, new];
            msg_send![object, applicationShouldTerminate: std::ptr::null_mut::<AnyObject>()]
        };
        assert_eq!(reply, TERMINATE_NOW);
    }

    #[test]
    fn the_four_character_codes_match_the_headers() {
        assert_eq!(KEY_AE_QUIT_REASON, 0x7768_793F);
        assert_eq!(fourcc(b"shut"), 0x7368_7574);
        assert_eq!(fourcc(b"rest"), 0x7265_7374);
    }
}
