//! The CEF `App`: runs in every process (browser, renderer, GPU, ...).
//!
//! Browser side: tuned command line, custom scheme registration, start-up hook.
//! Renderer side: `cefQuery` router and the page scripts, injected before any page script runs.

use cef::{
    wrapper::message_router::{
        MessageRouterConfig, MessageRouterRendererSide, MessageRouterRendererSideHandlerCallbacks, RendererSideRouter,
    },
    *,
};
use std::{cell::RefCell, collections::HashMap, sync::Arc};

pub const SCHEME: &str = "aditus";

/// What the browser process tells the renderer about a view (via `extra_info`).
#[derive(Clone, Default)]
struct Flags {
    /// Ordinary web page: gets the page script.
    page: bool,
    /// Ad blocking is on: gets the YouTube script.
    yt: bool,
}

/// `scheme://host/path?query` without the fragment: the key scriptlets are stashed under.
fn doc_key(url: &str) -> &str {
    url.split('#').next().unwrap_or_default()
}

/// Scriptlets the browser sent ahead of a document, by URL, until its context is created.
const STASH_MAX: usize = 32;

fn run_in(c: &V8Context, code: &str) {
    c.enter();
    // `retval` must be given or eval fails without an exception.
    let (mut ex, mut rv) = (None, None);
    let _ok = c.eval(Some(&CefString::from(code)), Some(&CefString::from("aditus://inject")), 1, Some(&mut rv), Some(&mut ex));
    c.exit();
}

thread_local! {
    static STASH: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    /// Frames (by id) whose current document already got its scriptlets.
    static APPLIED: RefCell<std::collections::HashSet<String>> = RefCell::new(Default::default());
    static ROUTER: Arc<RendererSideRouter> = RendererSideRouter::new(MessageRouterConfig::default());
    static FLAGS: RefCell<HashMap<i32, Flags>> = RefCell::new(HashMap::new());
}

wrap_render_process_handler! {
    struct AditusRenderHandler;

    impl RenderProcessHandler {
        fn on_browser_created(&self, browser: Option<&mut Browser>, extra_info: Option<&mut DictionaryValue>) {
            let (Some(b), Some(info)) = (browser, extra_info) else { return };
            let key = |k: &str| info.bool(Some(&CefString::from(k))) != 0;
            let f = Flags { page: key("page"), yt: key("yt") };
            FLAGS.with(|m| m.borrow_mut().insert(b.identifier(), f));
        }

        fn on_browser_destroyed(&self, browser: Option<&mut Browser>) {
            if let Some(b) = browser {
                FLAGS.with(|m| m.borrow_mut().remove(&b.identifier()));
            }
        }

        fn on_context_created(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, context: Option<&mut V8Context>) {
            let (b, f, c) = (browser.cloned(), frame.cloned(), context.cloned());
            let main = f.clone();
            if let Some(f) = main.as_ref() {
                APPLIED.with(|a| a.borrow_mut().remove(&CefString::from(&f.identifier()).to_string()));
            }
            ROUTER.with(|r| r.on_context_created(b.clone(), f, c.clone()));
            // Scripts go in through the context itself: no CSP applies, and they run before page code.
            let (Some(b), Some(c)) = (b, c) else { return };
            let flags = FLAGS.with(|m| m.borrow().get(&b.identifier()).cloned()).unwrap_or_default();
            if !flags.page {
                return;
            }
            // Tell the browser which OS process renders this tab, for the per-tab memory readout.
            if let Some(f) = main.clone().filter(|f| f.is_main() != 0) {
                if let Some(mut m) = process_message_create(Some(&CefString::from("aditus-pid"))) {
                    if let Some(args) = m.argument_list() {
                        args.set_int(0, std::process::id() as i32);
                        f.send_process_message(ProcessId::from(sys::cef_process_id_t::PID_BROWSER), Some(&mut m));
                    }
                }
            }
            let run = |code: &str| run_in(&c, code);
            // Scriptlets first, so they are in place before the page's own scripts.
            if let Some(f) = main.as_ref() {
                let key = CefString::from(&f.url()).to_string();
                if let Some(code) = STASH.with(|s| s.borrow_mut().remove(doc_key(&key))).filter(|c| !c.is_empty()) {
                    APPLIED.with(|a| a.borrow_mut().insert(CefString::from(&f.identifier()).to_string()));
                    run(&code);
                }
            }
            run(crate::scripts::PAGE);
            if flags.yt {
                run(crate::scripts::YOUTUBE);
            }
        }

        fn on_context_released(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, context: Option<&mut V8Context>) {
            let (b, f, c) = (browser.cloned(), frame.cloned(), context.cloned());
            ROUTER.with(|r| r.on_context_released(b, f, c));
        }

        fn on_process_message_received(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            source_process: ProcessId,
            message: Option<&mut ProcessMessage>,
        ) -> i32 {
            // Scriptlets come from the browser process only (a page cannot send process messages) and are only
            // ever run in the main world of the frame they were computed for.
            if let (Some(m), Some(f)) = (message.as_ref(), frame.as_ref()) {
                if CefString::from(&m.name()).to_string() == "aditus-scriptlets" {
                    if let Some(a) = m.argument_list() {
                        let url = CefString::from(&a.string(0)).to_string();
                        let code = CefString::from(&a.string(1)).to_string();
                        if a.bool(2) == 0 {
                            STASH.with(|s| {
                                let mut s = s.borrow_mut();
                                if s.len() >= STASH_MAX {
                                    s.clear();
                                }
                                s.insert(doc_key(&url).to_string(), code);
                            });
                        } else if doc_key(&CefString::from(&f.url()).to_string()) == doc_key(&url) {
                            let id = CefString::from(&f.identifier()).to_string();
                            if !APPLIED.with(|a| a.borrow().contains(&id)) {
                                if let Some(c) = f.v8_context() {
                                    APPLIED.with(|a| a.borrow_mut().insert(id));
                                    run_in(&c, &code);
                                }
                            }
                        }
                    }
                    return 1;
                }
            }
            let (b, f, m) = (browser.cloned(), frame.cloned(), message.cloned());
            ROUTER.with(|r| r.on_process_message_received(b, f, Some(source_process), m)) as i32
        }
    }
}

wrap_browser_process_handler! {
    struct AditusBrowserHandler;

    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            crate::engine::start();
        }

        fn on_already_running_app_relaunch(&self, command_line: Option<&mut CommandLine>, _dir: Option<&CefString>) -> i32 {
            // Second launch of the same profile directory (Windows, Linux): hand its links to this one.
            let mut list = CefStringList::new();
            if let Some(c) = command_line {
                c.arguments(Some(&mut list));
            }
            let urls: Vec<String> = list.into_iter().collect();
            crate::tabs::open_urls(&crate::tabs::AppHandle, urls);
            1
        }
    }
}

wrap_app! {
    pub struct AditusApp;

    impl App {
        fn on_before_command_line_processing(&self, process_type: Option<&CefString>, command_line: Option<&mut CommandLine>) {
            // Children inherit what the browser process sets.
            if !process_type.map(|p| p.to_string().is_empty()).unwrap_or(true) {
                return;
            }
            let Some(cl) = command_line else { return };
            if let Some(p) = std::env::var("ADITUS_NETLOG").ok().filter(|_| cfg!(debug_assertions)) {
                cl.append_switch_with_value(Some(&CefString::from("log-net-log")), Some(&CefString::from(p.as_str())));
                cl.append_switch_with_value(Some(&CefString::from("net-log-capture-mode")), Some(&CefString::from("Everything")));
            }
            // Debug runs only: a fixed key instead of the Keychain, whose prompt blocks scripted runs.
            if cfg!(debug_assertions) {
                cl.append_switch(Some(&CefString::from("use-mock-keychain")));
            }
            // Debug runs only: try extra switches, e.g. ADITUS_FLAGS="renderer-process-limit=4 enable-low-end-device-mode".
            if cfg!(debug_assertions) {
                for f in std::env::var("ADITUS_FLAGS").unwrap_or_default().split_whitespace() {
                    let (k, v) = f.split_once('=').unwrap_or((f, ""));
                    if v.is_empty() {
                        cl.append_switch(Some(&CefString::from(k)));
                    } else {
                        cl.append_switch_with_value(Some(&CefString::from(k)), Some(&CefString::from(v)));
                    }
                }
            }
            if let Some(dirs) = crate::extensions::load_arg() {
                cl.append_switch_with_value(Some(&CefString::from("load-extension")), Some(&CefString::from(dirs.as_str())));
            }
            if cfg!(debug_assertions) && std::env::var_os("NOFLAGS").is_some() {
                return;
            }
            for (k, v) in [
                ("disable-features", "Translate,MediaRouter,OptimizationHints,AutofillServerCommunication,InterestFeedContentSuggestions,SpareRendererForSitePerProcess"),
                ("disable-background-networking", ""),
                ("disable-component-update", ""),
                ("disable-sync", ""),
                ("no-pings", ""),
                ("disable-default-apps", ""),
                ("metrics-recording-only", ""),
                ("no-first-run", ""),
            ] {
                if v.is_empty() {
                    cl.append_switch(Some(&CefString::from(k)));
                } else {
                    cl.append_switch_with_value(Some(&CefString::from(k)), Some(&CefString::from(v)));
                }
            }
        }

        fn on_register_custom_schemes(&self, registrar: Option<&mut SchemeRegistrar>) {
            let Some(r) = registrar else { return };
            use sys::cef_scheme_options_t as O;
            let opts = O::CEF_SCHEME_OPTION_STANDARD as i32 | O::CEF_SCHEME_OPTION_SECURE as i32 | O::CEF_SCHEME_OPTION_CORS_ENABLED as i32 | O::CEF_SCHEME_OPTION_FETCH_ENABLED as i32;
            r.add_custom_scheme(Some(&CefString::from(SCHEME)), opts);
        }

        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(AditusBrowserHandler::new())
        }

        fn render_process_handler(&self) -> Option<RenderProcessHandler> {
            Some(AditusRenderHandler::new())
        }
    }
}
