//! WebKitGTK content filters (same JSON format as Safari content blockers).
//!
//! The `webkit2gtk` crate has no safe wrapper for filter stores, so this uses the -sys FFI.

use super::blocker;
use std::{cell::RefCell, ffi::CString, rc::Rc};
use tauri::{AppHandle, Manager, Webview};
use webkit2gtk::{
    gio,
    glib::{self, translate::ToGlibPtr},
    UserContentManagerExt, WebViewExt,
};
use webkit2gtk_sys as ffi;

type Filter = *mut ffi::WebKitUserContentFilter;

thread_local! {
    static FILTERS: RefCell<Vec<Filter>> = const { RefCell::new(Vec::new()) };
}

pub fn set_enabled(wv: &Webview, enabled: bool) {
    let _ = wv.with_webview(move |pw| {
        let Some(ucm) = pw.inner().user_content_manager() else { return };
        let ptr: *mut ffi::WebKitUserContentManager = ucm.to_glib_none().0;
        unsafe {
            ffi::webkit_user_content_manager_remove_all_filters(ptr);
            if enabled {
                FILTERS.with(|f| f.borrow().iter().for_each(|&filter| ffi::webkit_user_content_manager_add_filter(ptr, filter)));
            }
        }
    });
}

pub fn attach(_app: &AppHandle, wv: &Webview) {
    set_enabled(wv, true);
}

pub fn apply_all(app: &AppHandle) {
    for (label, wv) in app.webviews() {
        if label.starts_with("tab-") {
            let host = wv.url().ok().and_then(|u| u.host_str().map(String::from)).unwrap_or_default();
            set_enabled(&wv, super::enabled_for(app, &host));
        }
    }
}

type Done = Box<dyn FnOnce(Filter)>;

unsafe extern "C" fn save_done(
    store: *mut glib::gobject_ffi::GObject,
    res: *mut gio::ffi::GAsyncResult,
    data: glib::ffi::gpointer,
) {
    let done = Box::from_raw(data as *mut Done);
    let mut err = std::ptr::null_mut();
    let filter = ffi::webkit_user_content_filter_store_save_finish(store as *mut ffi::WebKitUserContentFilterStore, res, &mut err);
    if !err.is_null() {
        eprintln!("[adblock] filter compile failed");
        glib::ffi::g_error_free(err);
    }
    done(filter);
}

pub fn reinstall(app: &AppHandle) {
    let Some(b) = blocker(app) else { return };
    let chunks = b.rules.read().unwrap().clone();
    let dir = b.dir.join("compiled");
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || unsafe {
        let total = chunks.len();
        if total == 0 {
            FILTERS.with(|f| f.borrow_mut().clear());
            apply_all(&app2);
            return;
        }
        let _ = std::fs::create_dir_all(&dir);
        let path = CString::new(dir.to_string_lossy().as_bytes()).unwrap();
        // The store is intentionally leaked: it lives as long as the process.
        let store = ffi::webkit_user_content_filter_store_new(path.as_ptr());
        let results: Rc<RefCell<Vec<Filter>>> = Rc::default();
        let finished = Rc::new(RefCell::new(0usize));
        for (i, json) in chunks.iter().enumerate() {
            let (results, finished, app3) = (results.clone(), finished.clone(), app2.clone());
            let done: Done = Box::new(move |filter| {
                if !filter.is_null() {
                    results.borrow_mut().push(filter);
                }
                *finished.borrow_mut() += 1;
                if *finished.borrow() == total {
                    FILTERS.with(|f| *f.borrow_mut() = results.borrow().clone());
                    apply_all(&app3);
                }
            });
            let id = CString::new(format!("aditus-{i}")).unwrap();
            let bytes = glib::Bytes::from(json.as_bytes());
            ffi::webkit_user_content_filter_store_save(
                store,
                id.as_ptr(),
                bytes.to_glib_none().0,
                std::ptr::null_mut(),
                Some(save_done),
                Box::into_raw(Box::new(done)) as glib::ffi::gpointer,
            );
        }
    });
}
