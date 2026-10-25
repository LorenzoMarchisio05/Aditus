//! macOS glue CEF needs: an `NSApplication` that speaks Chromium's protocols, and an app delegate.

use cef::application_mac::{CefAppProtocol, CrAppControlProtocol, CrAppProtocol};
use objc2::{
    define_class, extern_methods, msg_send,
    rc::Retained,
    runtime::{AnyObject, Bool, NSObject, NSObjectProtocol, ProtocolObject},
    ClassType, DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::{NSApp, NSApplication, NSApplicationDelegate, NSApplicationTerminateReply, NSEvent};
use objc2_foundation::{NSArray, NSURL};
use std::cell::Cell;

define_class! {
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    pub struct AppDelegate;

    impl AppDelegate {}

    unsafe impl NSObjectProtocol for AppDelegate {}

    unsafe impl NSApplicationDelegate for AppDelegate {
        #[unsafe(method(applicationShouldTerminate:))]
        unsafe fn application_should_terminate(&self, _sender: &NSApplication) -> NSApplicationTerminateReply {
            NSApplicationTerminateReply::TerminateNow
        }

        /// Dock icon clicked while running: bring the focused window back.
        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        unsafe fn application_should_handle_reopen(&self, _sender: &NSApplication, _has_visible_windows: Bool) -> Bool {
            crate::reopen();
            Bool::NO
        }

        /// Links opened with Aditus as the default browser.
        #[unsafe(method(application:openURLs:))]
        unsafe fn application_open_urls(&self, _sender: &NSApplication, urls: &NSArray<NSURL>) {
            let list = urls.iter().filter_map(|u| u.absoluteString()).map(|s| s.to_string()).collect();
            crate::queue_urls(list);
        }

        #[unsafe(method(applicationSupportsSecureRestorableState:))]
        unsafe fn application_supports_secure_restorable_state(&self, _sender: &NSApplication) -> Bool {
            Bool::YES
        }
    }
}

impl AppDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

#[derive(Default)]
pub struct AppIvars {
    handling_send_event: Cell<Bool>,
}

define_class!(
    /// Chromium needs to know when the app is inside `sendEvent:` and wants `terminate:` to end the
    /// message loop instead of calling `exit()`.
    #[unsafe(super(NSApplication))]
    #[ivars = AppIvars]
    pub struct Application;

    impl Application {
        #[unsafe(method(sendEvent:))]
        unsafe fn send_event(&self, event: &NSEvent) {
            let was = self.is_handling_send_event();
            if !was {
                self.set_handling_send_event(true);
            }
            let _: () = msg_send![super(self), sendEvent: event];
            if !was {
                self.set_handling_send_event(false);
            }
        }

        #[unsafe(method(terminate:))]
        unsafe fn terminate(&self, _sender: &AnyObject) {
            crate::engine::quit();
        }
    }

    unsafe impl CrAppControlProtocol for Application {
        #[unsafe(method(setHandlingSendEvent:))]
        unsafe fn _set_handling_send_event(&self, v: Bool) {
            self.ivars().handling_send_event.set(v);
        }
    }

    unsafe impl CrAppProtocol for Application {
        #[unsafe(method(isHandlingSendEvent))]
        unsafe fn _is_handling_send_event(&self) -> Bool {
            self.ivars().handling_send_event.get()
        }
    }

    unsafe impl CefAppProtocol for Application {}
);

impl Application {
    extern_methods! {
        #[unsafe(method(sharedApplication))]
        fn shared_application() -> Retained<Self>;

        #[unsafe(method(setHandlingSendEvent:))]
        fn set_handling_send_event(&self, v: bool);

        #[unsafe(method(isHandlingSendEvent))]
        fn is_handling_send_event(&self) -> bool;
    }
}

/// Must run before anything else touches `NSApp`.
pub fn setup_application() {
    let _ = Application::shared_application();
    assert!(NSApp(MainThreadMarker::new().expect("main thread")).isKindOfClass(Application::class()));
}

pub fn setup_delegate() -> Retained<AppDelegate> {
    let mtm = MainThreadMarker::new().expect("main thread");
    let delegate = AppDelegate::new(mtm);
    NSApp(mtm).setDelegate(Some(&ProtocolObject::<dyn NSApplicationDelegate>::from_retained(delegate.clone())));
    delegate
}

/// A dev run is a bare binary, so macOS names the app menu after it ("aditus").
pub fn brand_name() {
    use objc2_foundation::NSString;
    unsafe {
        let bundle: *mut AnyObject = msg_send![objc2::class!(NSBundle), mainBundle];
        let info: *mut AnyObject = msg_send![bundle, infoDictionary];
        let (name, key) = (NSString::from_str("Aditus"), NSString::from_str("CFBundleName"));
        let _: () = msg_send![info, setObject: &*name, forKey: &*key];
    }
}

/// Puts the traffic lights where the sidebar wants them: a row beside the brand, or a tight row
/// that fits the icon-only sidebar. `view` is any NSView of the window. AppKit resets the buttons on
/// resize and full screen, so this runs on every layout.
pub fn place_window_buttons(view: *mut std::ffi::c_void, collapsed: bool, sidebar_w: f64) {
    use objc2_foundation::{NSPoint, NSRect};
    if view.is_null() {
        return;
    }
    unsafe {
        let view = view as *mut AnyObject;
        let win: *mut AnyObject = msg_send![view, window];
        if win.is_null() {
            return;
        }
        // 0 close, 1 miniaturize, 2 zoom
        for kind in 0_isize..3 {
            let btn: *mut AnyObject = msg_send![win, standardWindowButton: kind];
            if btn.is_null() {
                continue;
            }
            let sup: *mut AnyObject = msg_send![btn, superview];
            if sup.is_null() {
                continue;
            }
            let f: NSRect = msg_send![btn, frame];
            let sf: NSRect = msg_send![sup, frame];
            let i = kind as f64;
            // Offsets from the window's top-left corner, in points. Collapsed: a tight row centred in the strip.
            let (gap, left) = if collapsed { (5.0, (sidebar_w - (3.0 * f.size.width + 10.0)) / 2.0) } else { (8.0, 18.0) };
            let (x, top) = (left + i * (f.size.width + gap), 19.0);
            let origin = NSPoint::new(x, sf.size.height - top - f.size.height);
            let _: () = msg_send![btn, setFrameOrigin: origin];
        }
    }
}
