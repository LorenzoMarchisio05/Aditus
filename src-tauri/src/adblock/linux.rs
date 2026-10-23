//! WebKitGTK content filters (same JSON format as Safari content blockers).

use super::blocker;
use tauri::{AppHandle, Manager, Webview};
use webkit2gtk::{
    gio, glib, UserContentFilter, UserContentFilterStore, UserContentManagerExt, WebViewExt,
};

thread_local! {
    static FILTERS: std::cell::RefCell<Vec<UserContentFilter>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub fn set_enabled(wv: &Webview, enabled: bool) {
    let _ = wv.with_webview(move |pw| {
        let Some(ucm) = pw.inner().user_content_manager() else { return };
        ucm.remove_all_filters();
        if enabled {
            FILTERS.with(|f| f.borrow().iter().for_each(|filter| ucm.add_filter(filter)));
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

pub fn reinstall(app: &AppHandle) {
    let Some(b) = blocker(app) else { return };
    let chunks = b.rules.read().unwrap().clone();
    let dir = b.dir.join("compiled");
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        let store = UserContentFilterStore::new(&dir.to_string_lossy());
        let total = chunks.len();
        let done = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        if total == 0 {
            FILTERS.with(|f| f.borrow_mut().clear());
            apply_all(&app2);
            return;
        }
        for (i, json) in chunks.iter().enumerate() {
            let (done, app3) = (done.clone(), app2.clone());
            store.save(
                &format!("aditus-{i}"),
                &glib::Bytes::from(json.as_bytes()),
                None::<&gio::Cancellable>,
                move |res| {
                    match res {
                        Ok(f) => done.borrow_mut().push(f),
                        Err(e) => eprintln!("[adblock] filter failed: {e}"),
                    }
                    if done.borrow().len() == total {
                        FILTERS.with(|f| *f.borrow_mut() = done.borrow().clone());
                        apply_all(&app3);
                    }
                },
            );
        }
    });
}
