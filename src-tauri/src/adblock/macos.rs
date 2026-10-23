//! WKWebView content rule lists. All objc calls run on the main thread.

use super::blocker;
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_foundation::{NSError, NSString};
use objc2_web_kit::{WKContentRuleList, WKContentRuleListStore, WKUserContentController};
use std::{cell::RefCell, rc::Rc};
use tauri::{AppHandle, Manager, Webview};

thread_local! {
    static LISTS: RefCell<Vec<Retained<WKContentRuleList>>> = const { RefCell::new(Vec::new()) };
    static GEN: RefCell<u64> = const { RefCell::new(0) };
}

fn controller(ptr: *mut std::ffi::c_void) -> Option<&'static WKUserContentController> {
    // The pointer is borrowed from wry for the duration of `with_webview`.
    unsafe { (ptr as *const WKUserContentController).as_ref() }
}

pub fn set_enabled(wv: &Webview, enabled: bool) {
    let _ = wv.with_webview(move |pw| {
        let Some(ctrl) = controller(pw.controller()) else { return };
        unsafe {
            ctrl.removeAllContentRuleLists();
            if enabled {
                LISTS.with(|l| l.borrow().iter().for_each(|r| ctrl.addContentRuleList(r)));
            }
        }
    });
}

pub fn attach(app: &AppHandle, wv: &Webview) {
    // New tabs start unmapped-and-enabled; `on_navigate` corrects it per host.
    let _ = app;
    set_enabled(wv, true);
}

pub fn apply_all(app: &AppHandle) {
    for (label, wv) in app.webviews() {
        if !label.starts_with("tab-") {
            continue;
        }
        let host = wv.url().ok().and_then(|u| u.host_str().map(String::from)).unwrap_or_default();
        set_enabled(&wv, super::enabled_for(app, &host));
    }
}

/// Recompile all rule chunks for the current engine, then apply to open tabs.
pub fn reinstall(app: &AppHandle) {
    let Some(b) = blocker(app) else { return };
    let chunks = b.rules.read().unwrap().clone();
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let Some(store) = (unsafe { WKContentRuleListStore::defaultStore(mtm) }) else { return };
        let gen = GEN.with(|g| {
            *g.borrow_mut() += 1;
            *g.borrow()
        });
        if chunks.is_empty() {
            LISTS.with(|l| l.borrow_mut().clear());
            apply_all(&app2);
            return;
        }
        let total = chunks.len();
        let done: Rc<RefCell<Vec<Retained<WKContentRuleList>>>> = Rc::default();
        let finished = Rc::new(RefCell::new(0usize));
        for (i, json) in chunks.iter().enumerate() {
            let (done, finished, app3) = (done.clone(), finished.clone(), app2.clone());
            let block = RcBlock::new(move |list: *mut WKContentRuleList, err: *mut NSError| {
                if let Some(list) = unsafe { Retained::retain(list) } {
                    done.borrow_mut().push(list);
                } else if let Some(e) = unsafe { err.as_ref() } {
                    eprintln!("[adblock] rule list failed: {}", e.localizedDescription());
                }
                *finished.borrow_mut() += 1;
                if *finished.borrow() == total && GEN.with(|g| *g.borrow() == gen) {
                    LISTS.with(|l| *l.borrow_mut() = done.borrow().clone());
                    apply_all(&app3);
                }
            });
            let id = NSString::from_str(&format!("aditus-{gen}-{i}"));
            let json = NSString::from_str(json);
            unsafe {
                store.compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(
                    Some(&id),
                    Some(&json),
                    Some(&block),
                );
            }
        }
    });
}
