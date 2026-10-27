//! Windows, browser views and request contexts on top of CEF's Views framework.
//!
//! Everything here runs on the CEF UI thread (the macOS main thread); the registry is thread-local.
//! Views are addressed by label (`ui`, `tab-3-0`, `palette-w2`, ...). Never hold the registry
//! borrow across a CEF call: CEF calls back into our handlers synchronously, and those use it too.

use crate::{app::SCHEME, ipc, scheme, tabs};
use cef::{wrapper::message_router::MessageRouterBrowserSideHandlerCallbacks, *};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// The window's main UI (sidebar, top bar).
    Ui,
    /// Palette, suggestion list and panels drawn over the page area.
    Overlay,
    /// A tab's page.
    Page,
}

pub type Bounds = (f64, f64, f64, f64);

pub struct Spec {
    pub label: String,
    pub win: String,
    pub url: String,
    pub kind: Kind,
    /// An app page (`aditus://app/...`): may use every command until it navigates away.
    pub app_page: bool,
    /// Context key and cache dir (empty = in memory); `None` uses the global context.
    pub store: Option<(String, PathBuf)>,
    pub bounds: Bounds,
    pub visible: bool,
    pub zoom: u32,
    pub yt: bool,
}

struct Rec {
    bv: BrowserView,
    win: String,
    kind: Kind,
    /// Cleared the first time the view leaves the app scheme.
    app_page: bool,
    /// Where the view belongs; the window's own layout pass resets child views, so it is re-applied.
    bounds: Bounds,
}

enum CtxState {
    Pending(Option<RequestContext>, Vec<Box<dyn FnOnce(RequestContext)>>),
    Ready(RequestContext),
}

#[derive(Default)]
struct Reg {
    views: HashMap<String, Rec>,
    /// Views waiting for their context: label -> (bounds, visible), kept current meanwhile.
    pending: HashMap<String, (Bounds, bool)>,
    wins: HashMap<String, Window>,
    by_browser: HashMap<i32, String>,
    ctxs: HashMap<String, CtxState>,
    /// Window -> label of the page showing HTML fullscreen.
    fullscreen: HashMap<String, String>,
    find_text: HashMap<String, String>,
    downloads: HashMap<u32, (i64, String)>,
    popup: Option<(MenuModel, MenuModelDelegate)>,
}

wrap_panel_delegate! {
    struct HostDelegate {}
    impl ViewDelegate {}
    impl PanelDelegate {}
}

thread_local! {
    /// Per window: the panel holding the page views, sized to the page area. Its box layout gives a page
    /// exactly its `PREF` size, so the window's layout pass (which stretches every direct child to the
    /// window) can never show a page a full-window viewport.
    static HOST: RefCell<HashMap<String, Panel>> = RefCell::new(HashMap::new());
    /// Window -> the page last placed in its host.
    static SHOWN: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    /// Page label -> size its host should give it.
    static PREF: RefCell<HashMap<String, (i32, i32)>> = RefCell::new(HashMap::new());
    static REG: RefCell<Reg> = RefCell::new(Reg::default());
}

/// Labels of views showing web pages; the resource handler may run off the UI thread, where `REG` is empty.
static PAGES: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<String>>> = std::sync::LazyLock::new(Default::default);

fn set_page(label: &str, on: bool) {
    let mut p = PAGES.lock().unwrap();
    if on { p.insert(label.to_string()); } else { p.remove(label); }
}

fn reg<R>(f: impl FnOnce(&mut Reg) -> R) -> R {
    REG.with(|r| f(&mut r.borrow_mut()))
}

fn s(v: &str) -> CefString {
    CefString::from(v)
}

fn rect((x, y, w, h): Bounds) -> Rect {
    Rect { x: x.round() as i32, y: y.round() as i32, width: w.round().max(1.0) as i32, height: h.round().max(1.0) as i32 }
}

/// Set a view's bounds only when they differ. The Views layout pass reports every change back to
/// `relayout`, so redundant calls loop and make pages see transient sizes (a visible flicker).
fn place(bv: &BrowserView, b: Bounds) {
    place_view(&View::from(bv), b);
}

fn place_view(v: &View, b: Bounds) {
    let want = rect(b);
    let cur = v.bounds();
    if (cur.x, cur.y, cur.width, cur.height) != (want.x, want.y, want.width, want.height) {
        v.set_bounds(Some(&want));
    }
}

fn view(label: &str) -> Option<BrowserView> {
    reg(|r| r.views.get(label).map(|v| v.bv.clone()))
}

fn window(win: &str) -> Option<Window> {
    reg(|r| r.wins.get(win).cloned())
}

fn frame(label: &str) -> Option<Frame> {
    view(label)?.browser()?.main_frame()
}

/// Hand the renderer the scriptlets for `url` (applied in `app.rs`). Sent at navigation start so the renderer
/// has them when the document's context is created; `late` is the page script's own request, which also
/// covers a navigation whose message lost the race (renderer swap, redirect). Empty early messages clear
/// anything stale for the URL.
fn send_scriptlets(frame: &Frame, url: &str, late: bool) {
    let url = url.split('#').next().unwrap_or_default();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return;
    }
    let script = crate::adblock::scriptlets(url);
    if late && script.is_empty() {
        return;
    }
    let Some(mut m) = process_message_create(Some(&CefString::from("aditus-scriptlets"))) else { return };
    if let Some(a) = m.argument_list() {
        a.set_string(0, Some(&CefString::from(url)));
        a.set_string(1, Some(&CefString::from(script.as_str())));
        a.set_bool(2, late as i32);
    }
    frame.send_process_message(ProcessId::from(sys::cef_process_id_t::PID_RENDERER), Some(&mut m));
}

pub fn send_scriptlets_late(label: &str, url: &str) {
    if let Some(f) = frame(label) {
        send_scriptlets(&f, url, true);
    }
}

fn host(label: &str) -> Option<BrowserHost> {
    view(label)?.browser()?.host()
}

pub fn has_view(label: &str) -> bool {
    reg(|r| r.views.contains_key(label) || r.pending.contains_key(label))
}

/// Run `f` on the UI thread soon, outside of whatever CEF callback we are in.
pub fn defer(f: impl FnOnce() + 'static) {
    wrap_task! {
        struct Job { f: std::rc::Rc<std::cell::RefCell<Option<Box<dyn FnOnce()>>>> }
        impl Task { fn execute(&self) { if let Some(f) = self.f.borrow_mut().take() { f() } } }
    }
    let mut t = Job::new(std::rc::Rc::new(RefCell::new(Some(Box::new(f)))));
    post_task(ThreadId::UI, Some(&mut t));
}

/// Like `defer`, from any thread: the closure must be `Send`.
pub fn defer_from_thread(f: impl FnOnce() + Send + 'static) {
    wrap_task! {
        struct Job { f: std::sync::Arc<std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>> }
        impl Task { fn execute(&self) { if let Some(f) = self.f.lock().unwrap().take() { f() } } }
    }
    let mut t = Job::new(std::sync::Arc::new(std::sync::Mutex::new(Some(Box::new(f)))));
    post_task(ThreadId::UI, Some(&mut t));
}

pub fn defer_after(ms: i64, f: impl FnOnce() + 'static) {
    wrap_task! {
        struct Job { f: std::rc::Rc<std::cell::RefCell<Option<Box<dyn FnOnce()>>>> }
        impl Task { fn execute(&self) { if let Some(f) = self.f.borrow_mut().take() { f() } } }
    }
    let mut t = Job::new(std::rc::Rc::new(RefCell::new(Some(Box::new(f)))));
    post_delayed_task(ThreadId::UI, Some(&mut t), ms);
}

// ---------- start / stop ----------

/// CEF is initialised: build the first window and restore the session.
pub fn start() {
    scheme::register(None);
    tabs::startup(&tabs::AppHandle);
}

thread_local! {
    /// Windows whose close was refused (a page's beforeunload dialog is up), and when.
    static CLOSE_ASKED: RefCell<HashMap<String, Instant>> = RefCell::new(HashMap::new());
}

const CLOSE_FORCE: Duration = Duration::from_secs(3);

/// A close request right after a refused one (user insists) must go through.
fn force_close(asked: Option<Instant>, now: Instant) -> bool {
    asked.is_some_and(|t| now.duration_since(t) < CLOSE_FORCE)
}

fn maybe_quit() {
    if reg(|r| r.wins.is_empty() && r.by_browser.is_empty()) {
        quit_message_loop();
    }
}

/// Close every window (Cmd+Q, Quit menu); the message loop ends once the last browser is gone.
pub fn quit() {
    let wins: Vec<Window> = reg(|r| r.wins.values().cloned().collect());
    if wins.is_empty() {
        return quit_message_loop();
    }
    for w in wins {
        w.close();
    }
}

// ---------- request contexts ----------

wrap_request_context_handler! {
    struct CtxHandler {
        key: String,
    }

    impl RequestContextHandler {
        fn on_request_context_initialized(&self, ctx: Option<&mut RequestContext>) {
            let Some(ctx) = ctx else { return };
            scheme::register(Some(ctx));
            let waiting = reg(|r| match r.ctxs.insert(self.key.clone(), CtxState::Ready(ctx.clone())) {
                Some(CtxState::Pending(_, q)) => q,
                _ => vec![],
            });
            for f in waiting {
                f(ctx.clone());
            }
        }
    }
}

/// Runs `f` with the context for `key` once it is initialised (at once if it already is).
fn with_context(key: &str, cache: &Path, f: Box<dyn FnOnce(RequestContext)>) {
    enum Step {
        Run(RequestContext),
        Queued,
        Create,
    }
    let mut f = Some(f);
    let step = reg(|r| match r.ctxs.get_mut(key) {
        Some(CtxState::Ready(c)) => Step::Run(c.clone()),
        Some(CtxState::Pending(_, q)) => {
            q.push(f.take().unwrap());
            Step::Queued
        }
        None => Step::Create,
    });
    match step {
        Step::Run(c) => (f.take().unwrap())(c),
        Step::Queued => {}
        Step::Create => {
            reg(|r| r.ctxs.insert(key.to_string(), CtxState::Pending(None, vec![f.take().unwrap()])));
            let settings = RequestContextSettings {
                cache_path: s(&cache.to_string_lossy()),
                persist_session_cookies: !cache.as_os_str().is_empty() as i32,
                ..Default::default()
            };
            let mut handler = CtxHandler::new(key.to_string());
            let early = request_context_create_context(Some(&settings), Some(&mut handler));
            // Keep the context alive until its init callback.
            reg(|r| {
                if let Some(CtxState::Pending(slot, _)) = r.ctxs.get_mut(key) {
                    *slot = early;
                }
            });
        }
    }
}

/// Forget a window's private context (its data goes with it).
pub fn drop_context(key: &str) {
    reg(|r| r.ctxs.remove(key));
}

// ---------- views ----------

fn client(label: &str) -> Client {
    ViewClient::new(label.to_string())
}

wrap_browser_view_delegate! {
    struct PageViewDelegate {
        label: String,
        devtools: bool,
    }

    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size {
            let (width, height) = PREF.with(|p| p.borrow().get(&self.label).copied()).unwrap_or((800, 600));
            Size { width, height }
        }
    }

    impl BrowserViewDelegate {
        fn browser_runtime_style(&self) -> RuntimeStyle {
            // CEF only supports Chrome style for DevTools popups.
            if self.devtools { RuntimeStyle::CHROME } else { RuntimeStyle::ALLOY }
        }

        fn delegate_for_popup_browser_view(&self, _view: Option<&mut BrowserView>, _settings: Option<&BrowserSettings>, _client: Option<&mut Client>, is_devtools: i32) -> Option<BrowserViewDelegate> {
            (is_devtools != 0).then(|| PageViewDelegate::new(self.label.clone(), true))
        }

        fn on_popup_browser_view_created(&self, _view: Option<&mut BrowserView>, popup: Option<&mut BrowserView>, is_devtools: i32) -> i32 {
            let (Some(popup), true) = (popup, is_devtools != 0) else { return 0 };
            // A second docked DevTools in one window crashes CEF: later ones open in their own window.
            if dock_mode() == "window" || win_of(&self.label).is_some_and(|w| DEV.with(|d| d.borrow().values().any(|e| e.0 == w))) {
                return 0;
            }
            // CEF reads the popup's window right after this returns: it must be parented already.
            dock_devtools(&self.label, popup.clone());
            1
        }

    }
}

fn background(kind: Kind) -> u32 {
    let _ = kind;
    0xFF14_1414
}

/// Creates a page or UI view. The view appears once its context is ready; until then bounds and
/// visibility changes are remembered.
pub fn create(spec: Spec) {
    if has_view(&spec.label) {
        return;
    }
    reg(|r| r.pending.insert(spec.label.clone(), (spec.bounds, spec.visible)));
    match spec.store.clone() {
        None => build(spec, None),
        Some((key, path)) => {
            with_context(&key, &path, Box::new(move |ctx| build(spec, Some(ctx))));
        }
    }
}

fn build(spec: Spec, ctx: Option<RequestContext>) {
    // Closed while its context was starting?
    let Some((bounds, visible)) = reg(|r| r.pending.remove(&spec.label)) else { return };
    let Some(win) = window(&spec.win) else { return };
    let mut extra = dictionary_value_create();
    if let Some(e) = extra.as_mut() {
        e.set_bool(Some(&s("page")), (spec.kind == Kind::Page && !spec.app_page) as i32);
        e.set_bool(Some(&s("yt")), spec.yt as i32);
    }
    let settings = BrowserSettings { background_color: background(spec.kind), ..Default::default() };
    let mut ctx = ctx;
    let mut delegate = PageViewDelegate::new(spec.label.clone(), false);
    let Some(bv) = browser_view_create(
        Some(&mut client(&spec.label)),
        Some(&s(&spec.url)),
        Some(&settings),
        extra.as_mut(),
        ctx.as_mut(),
        Some(&mut delegate),
    ) else {
        eprintln!("browser_view_create failed for {}", spec.label);
        return;
    };
    reg(|r| {
        set_page(&spec.label, spec.kind == Kind::Page && !spec.app_page);
        r.views.insert(spec.label.clone(), Rec { bv: bv.clone(), win: spec.win.clone(), kind: spec.kind, app_page: spec.app_page, bounds });
    });
    let mut v = View::from(&bv);
    match HOST.with(|h| h.borrow().get(&spec.win).cloned()).filter(|_| spec.kind == Kind::Page) {
        Some(host) => {
            let (_, _, w, h) = bounds;
            PREF.with(|p| p.borrow_mut().insert(spec.label.clone(), (w.round() as i32, h.round() as i32)));
            host.add_child_view(Some(&mut v));
        }
        None => {
            win.add_child_view(Some(&mut v));
            bv.set_bounds(Some(&rect(bounds)));
        }
    }
    bv.set_visible(visible as i32);
    if spec.kind == Kind::Page && visible {
        set_bounds(&spec.label, bounds);
    }
    if spec.kind != Kind::Overlay {
        raise_overlays(&spec.win);
    }
}

/// macOS: keeps the traffic lights in the sidebar's top corner (see `mac::place_window_buttons`).
pub fn place_window_buttons(win: &str) {
    #[cfg(target_os = "macos")]
    {
        let Some(h) = host(&tabs::wv_label("ui", win)) else { return };
        let (collapsed, w) = {
            let g = tabs::shared(&tabs::AppHandle).app.lock().unwrap();
            (g.cfg.settings.sidebar_collapsed, g.wins.get(win).map_or(250.0, |w| w.sidebar))
        };
        let p = h.window_handle() as usize;
        let place = move || crate::mac::place_window_buttons(p as *mut std::ffi::c_void, collapsed, w);
        place();
        // Timer on purpose: AppKit re-lays the buttons out after resizes and focus changes with no callback
        // we can hook, so place them again once it has settled.
        defer_after(120, place);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = win;
}

/// Overlays (palette, suggestions, panels) must stay above pages created after them.
pub fn raise_overlays(win: &str) {
    let Some(w) = window(win) else { return };
    let overlays: Vec<BrowserView> = reg(|r| r.views.values().filter(|v| v.win == win && v.kind == Kind::Overlay).map(|v| v.bv.clone()).collect());
    for bv in overlays {
        w.reorder_child_view(Some(&mut View::from(&bv)), -1);
    }
}

pub fn set_bounds(label: &str, b: Bounds) {
    // A page is placed by moving and sizing its host (see `HOST`).
    if let Some((bv, win)) = reg(|r| r.views.get(label).filter(|v| v.kind == Kind::Page).map(|v| (v.bv.clone(), v.win.clone()))) {
        reg(|r| r.views.get_mut(label).map(|v| v.bounds = b));
        if let Some(host) = HOST.with(|h| h.borrow().get(&win).cloned()) {
            let size = (b.2.round() as i32, b.3.round() as i32);
            let old = PREF.with(|p| p.borrow_mut().insert(label.to_string(), size));
            let host_v = View::from(&host);
            if old != Some(size) {
                View::from(&bv).invalidate_layout();
            }
            let switched = SHOWN.with(|s| s.borrow_mut().insert(win.clone(), label.to_string())).as_deref() != Some(label);
            let moved = switched || {
                let c = host_v.bounds();
                (c.x, c.y) != (b.0.round() as i32, b.1.round() as i32)
            };
            place_view(&host_v, b);
            // The native view of a page does not follow its host moving; a nudge makes it re-attach.
            if moved {
                place_view(&View::from(&bv), (1.0, 0.0, b.2, b.3));
                place_view(&View::from(&bv), (0.0, 0.0, b.2, b.3));
            }
        }
        return;
    }
    let pending = reg(|r| r.pending.get_mut(label).map(|p| p.0 = b).is_some());
    if !pending {
        if let Some(bv) = view(label) {
            place(&bv, b);
            reg(|r| r.views.get_mut(label).map(|v| v.bounds = b));
        }
    }
}

pub fn set_visible(label: &str, visible: bool) {
    let pending = reg(|r| r.pending.get_mut(label).map(|p| p.1 = visible).is_some());
    if !pending {
        if let Some(bv) = view(label) {
            bv.set_visible(visible as i32);
        }
    }
}

wrap_completion_callback! {
    struct Done;

    impl CompletionCallback {
        fn on_complete(&self) {}
    }
}

/// Cookies and/or the HTTP cache of one profile's storage (loaded on demand if it isn't yet).
pub fn clear_site_data(key: &str, cache: &Path, cookies: bool, http_cache: bool) {
    with_context(
        key,
        cache,
        Box::new(move |ctx| {
            // These crash CEF when given no completion callback.
            if cookies {
                if let Some(m) = ctx.cookie_manager(Some(&mut Done::new())) {
                    m.delete_cookies(None, None, None);
                }
                ctx.clear_http_auth_credentials(Some(&mut Done::new()));
            }
            if http_cache {
                ctx.clear_http_cache(Some(&mut Done::new()));
            }
        }),
    );
}

pub fn close(label: &str) {
    set_page(label, false);
    // Destroying a browser that has DevTools docked crashes CEF (as does closing DevTools itself):
    // keep both alive, hidden and blank, instead.
    let docked = DEV.with(|d| d.borrow_mut().remove(label));
    if let Some((_, bv, _)) = &docked {
        navigate(label, "about:blank");
        bv.set_visible(0);
        PARK.with(|p| p.borrow_mut().push(bv.clone()));
    }
    let parked = docked.is_some();
    let rec = reg(|r| {
        r.pending.remove(label);
        r.find_text.remove(label);
        r.views.remove(label)
    });
    let Some(rec) = rec else { return };
    if parked {
        PARK.with(|p| p.borrow_mut().push(rec.bv.clone()));
    }
    // CloseBrowser on a view's browser would close the whole window. Removing the view and
    // dropping our last reference destroys the browser instead.
    PREF.with(|p| p.borrow_mut().remove(label));
    let host = HOST.with(|h| h.borrow().get(&rec.win).cloned()).filter(|_| rec.kind == Kind::Page);
    match (host, window(&rec.win)) {
        (Some(h), _) => h.remove_child_view(Some(&mut View::from(&rec.bv))),
        (None, Some(w)) => w.remove_child_view(Some(&mut View::from(&rec.bv))),
        _ => {}
    }
}

pub fn focus(label: &str) {
    if let Some(bv) = view(label) {
        bv.request_focus();
    }
}

pub fn navigate(label: &str, url: &str) {
    if let Some(f) = frame(label) {
        f.load_url(Some(&s(url)));
    }
}

/// Native history step of the view's browser. `false` when there is no live view or it cannot go that way.
pub fn go(label: &str, delta: i32) -> bool {
    let Some(b) = view(label).and_then(|v| v.browser()) else { return false };
    if delta < 0 && b.can_go_back() != 0 {
        b.go_back();
    } else if delta > 0 && b.can_go_forward() != 0 {
        b.go_forward();
    } else {
        return false;
    }
    true
}

pub fn eval(label: &str, js: &str) {
    if let Some(f) = frame(label) {
        f.execute_java_script(Some(&s(js)), Some(&s("aditus://eval")), 0);
    }
}

/// The page's real URL, as the engine reports it.
pub fn url_of(label: &str) -> Option<String> {
    let u = frame(label)?.url();
    Some(CefString::from(&u).to_string()).filter(|u| !u.is_empty())
}

pub fn set_muted(label: &str, muted: bool) {
    if let Some(h) = host(label) {
        h.set_audio_muted(muted as i32);
    }
}

pub fn set_zoom(label: &str, pct: u32) {
    if let Some(h) = host(label) {
        h.set_zoom_level((pct as f64 / 100.0).ln() / 1.2f64.ln());
    }
}

pub fn find(label: &str, text: &str, forward: bool) {
    let Some(h) = host(label) else { return };
    let next = reg(|r| r.find_text.insert(label.to_string(), text.to_string()).as_deref() == Some(text));
    h.find(Some(&s(text)), forward as i32, 0, next as i32);
}

pub fn stop_find(label: &str) {
    reg(|r| r.find_text.remove(label));
    if let Some(h) = host(label) {
        h.stop_finding(1);
    }
}

/// Download `url` through the normal downloads flow (Save Image As…).
pub fn download(label: &str, url: &str) {
    tabs::allow_download(&tabs::AppHandle, label);
    if let Some(h) = host(label) {
        h.start_download(Some(&s(url)));
    }
}

pub fn print(label: &str) {
    if let Some(h) = host(label) {
        h.print();
    }
}

thread_local! {
    /// Browsers that cannot be destroyed without crashing CEF, kept alive and hidden.
    static PARK: RefCell<Vec<BrowserView>> = RefCell::new(Vec::new());
    /// Docked DevTools by inspected page: (window, its view, shown). A DevTools view is never taken out
    /// of its window or closed (both crash CEF); it is only hidden.
    static DEV: RefCell<HashMap<String, (String, BrowserView, bool)>> = RefCell::new(HashMap::new());
}

fn dock_devtools(label: &str, popup: BrowserView) {
    let Some(win) = win_of(label) else { return };
    let Some(w) = window(&win) else { return };
    w.add_child_view(Some(&mut View::from(&popup)));
    DEV.with(|d| d.borrow_mut().insert(label.to_string(), (win.clone(), popup.clone(), true)));
    tabs::apply_layout(&tabs::AppHandle);
    raise_overlays(&win);
    popup.request_focus();
}

fn dock_mode() -> String {
    tabs::shared(&tabs::AppHandle).app.lock().unwrap().cfg.settings.devtools_dock.clone()
}

/// Called by the layout for the page showing in `win`: places its DevTools under it and returns the page's shorter area.
pub fn dock_split(win: &str, label: &str, area: Bounds) -> Bounds {
    let docs: Vec<(String, BrowserView, bool)> = DEV.with(|d| d.borrow().iter().filter(|(_, e)| e.0 == win).map(|(l, e)| (l.clone(), e.1.clone(), e.2)).collect());
    let mut out = area;
    let mode = dock_mode();
    for (l, bv, shown) in docs {
        let want = shown && l == label;
        bv.set_visible(want as i32);
        if want {
            let (x, y, w, h) = area;
            let (page, dev) = match mode.as_str() {
                "right" => ((x, y, (w * 0.6).round(), h), (x + (w * 0.6).round(), y, w - (w * 0.6).round(), h)),
                "left" => ((x + (w * 0.4).round(), y, w - (w * 0.4).round(), h), (x, y, (w * 0.4).round(), h)),
                "top" => ((x, y + (h * 0.4).round(), w, h - (h * 0.4).round()), (x, y, w, (h * 0.4).round())),
                // "bottom", and "window" for a DevTools that is already docked
                _ => ((x, y, w, (h * 0.6).round()), (x, y + (h * 0.6).round(), w, h - (h * 0.6).round())),
            };
            bv.set_bounds(Some(&rect(dev)));
            out = page;
        }
    }
    out
}

pub fn devtools(label: &str) {
    // The app's own pages (new tab) crash CEF's DevTools.
    if reg(|r| r.views.get(label).map_or(true, |v| v.app_page || v.kind != Kind::Page)) {
        return;
    }
    let mine = DEV.with(|d| {
        d.borrow_mut().get_mut(label).map(|e| {
            e.2 = !e.2;
            (e.1.clone(), e.2)
        })
    });
    if let Some((bv, shown)) = mine {
        tabs::apply_layout(&tabs::AppHandle);
        if shown {
            bv.request_focus();
        } else {
            focus(label);
        }
        return;
    }
    if let Some(h) = host(label) {
        h.show_dev_tools(Some(&WindowInfo::default()), None, Some(&BrowserSettings::default()), None);
    }
}

// ---------- IPC identity ----------

/// Who is calling, from the browser process' point of view.
pub fn ctx(browser: &Browser, frame: &Frame) -> Option<ipc::Ctx> {
    let label = reg(|r| r.by_browser.get(&browser.identifier()).cloned())?;
    let (win, app_page) = reg(|r| r.views.get(&label).map(|v| (v.win.clone(), v.app_page || v.kind != Kind::Page)))?;
    let frame_url = CefString::from(&frame.url()).to_string();
    let trusted = app_page && frame.is_main() != 0 && frame_url.starts_with(&format!("{SCHEME}://{}/", scheme::HOST));
    let frame = CefString::from(&frame.identifier()).to_string();
    Some(ipc::Ctx { label, win, trusted, frame_url, frame })
}

pub fn win_of(label: &str) -> Option<String> {
    reg(|r| r.views.get(label).map(|v| v.win.clone()))
}

// ---------- events to the UI ----------

fn trusted_labels() -> Vec<String> {
    reg(|r| r.views.iter().filter(|(_, v)| v.app_page || v.kind != Kind::Page).map(|(l, _)| l.clone()).collect())
}

fn emit_js(event: &str, json: &str) -> String {
    format!("window.__aditus_emit&&window.__aditus_emit({},{json})", serde_json::to_string(event).unwrap())
}

/// Broadcast to every app page (UI, overlays, settings tabs...).
pub fn emit(event: &str, json: &str) {
    let js = emit_js(event, json);
    for l in trusted_labels() {
        eval(&l, &js);
    }
}

pub fn emit_to(label: &str, event: &str, json: &str) {
    eval(label, &emit_js(event, json));
}

// ---------- windows ----------

pub fn win_size(win: &str) -> (f64, f64) {
    window(win).map(|w| w.size()).map(|s| (s.width as f64, s.height as f64)).unwrap_or((1280.0, 820.0))
}

pub fn set_title(win: &str, title: &str) {
    if let Some(w) = window(win) {
        w.set_title(Some(&s(title)));
    }
}

pub fn raise_window(win: &str) {
    if let Some(w) = window(win) {
        if w.is_minimized() != 0 {
            w.restore();
        }
        w.show();
        w.activate();
        w.bring_to_top();
    }
}

pub fn close_window(win: &str) {
    if let Some(w) = window(win) {
        w.close();
    }
}

pub fn window_count() -> usize {
    reg(|r| r.wins.len())
}

pub fn toggle_fullscreen(win: &str) {
    if let Some(w) = window(win) {
        w.set_fullscreen((w.is_fullscreen() == 0) as i32);
    }
}

pub fn minimize(win: &str) {
    if let Some(w) = window(win) {
        w.minimize();
    }
}

pub fn maximize_toggle(win: &str) {
    if let Some(w) = window(win) {
        if w.is_maximized() != 0 {
            w.restore();
        } else {
            w.maximize();
        }
    }
}

/// Fullscreen page of this window, if any.
pub fn fullscreen_label(win: &str) -> Option<String> {
    reg(|r| r.fullscreen.get(win).cloned())
}

/// Position the UI view and let the tab layer place pages and panels.
pub fn relayout(win: &str) {
    let (w, h) = win_size(win);
    if let Some(bv) = view(&tabs::wv_label("ui", win)) {
        place(&bv, (0.0, 0.0, w, h));
    }
    tabs::apply_layout(&tabs::AppHandle);
    // The layout pass stretches every child view; overlays are not part of apply_layout.
    let others: Vec<(BrowserView, Bounds)> = reg(|r| r.views.values().filter(|v| v.win == win && v.kind == Kind::Overlay).map(|v| (v.bv.clone(), v.bounds)).collect());
    for (bv, b) in others {
        place(&bv, b);
    }
}

wrap_window_delegate! {
    struct WinDelegate {
        label: String,
        ui: BrowserView,
    }

    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size {
            Size { width: 1280, height: 820 }
        }

        fn minimum_size(&self, _view: Option<&mut View>) -> Size {
            Size { width: 640, height: 400 }
        }

        fn on_layout_changed(&self, _view: Option<&mut View>, _bounds: Option<&Rect>) {
            relayout(&self.label);
        }
    }

    impl PanelDelegate {}

    impl WindowDelegate {
        fn on_window_created(&self, window: Option<&mut Window>) {
            let Some(window) = window else { return };
            reg(|r| r.wins.insert(self.label.clone(), window.clone()));
            let mut v = View::from(&self.ui);
            window.add_child_view(Some(&mut v));
            let sz = window.size();
            self.ui.set_bounds(Some(&Rect { x: 0, y: 0, width: sz.width, height: sz.height }));
            if let Some(host) = panel_create(Some(&mut HostDelegate::new())) {
                let settings = BoxLayoutSettings { horizontal: 1, main_axis_alignment: AxisAlignment::START, cross_axis_alignment: AxisAlignment::START, ..Default::default() };
                host.set_to_box_layout(Some(&settings));
                window.add_child_view(Some(&mut View::from(&host)));
                HOST.with(|h| h.borrow_mut().insert(self.label.clone(), host));
            }
            window.show();
        }

        fn on_window_destroyed(&self, _window: Option<&mut Window>) {
            HOST.with(|h| h.borrow_mut().remove(&self.label));
            SHOWN.with(|s| s.borrow_mut().remove(&self.label));
            let gone: Vec<String> = reg(|r| {
                r.wins.remove(&self.label);
                r.fullscreen.remove(&self.label);
                let gone: Vec<String> = r.views.iter().filter(|(_, v)| v.win == self.label).map(|(l, _)| l.clone()).collect();
                for l in &gone {
                    r.views.remove(l);
                }
                r.pending.retain(|l, _| !gone.contains(l));
                gone
            });
            drop(gone);
            let docked: Vec<String> = DEV.with(|d| d.borrow().iter().filter(|(_, e)| e.0 == self.label).map(|(l, _)| l.clone()).collect());
            for l in docked {
                if let Some((_, bv, _)) = DEV.with(|d| d.borrow_mut().remove(&l)) {
                    PARK.with(|p| p.borrow_mut().push(bv));
                }
            }
            tabs::window_closed(&tabs::AppHandle, &self.label);
            maybe_quit();
        }

        fn on_window_bounds_changed(&self, _window: Option<&mut Window>, _bounds: Option<&Rect>) {
            relayout(&self.label);
        }

        fn on_window_activation_changed(&self, _window: Option<&mut Window>, active: i32) {
            if active != 0 {
                tabs::set_focused(&tabs::AppHandle, &self.label);
            }
        }

        fn on_window_fullscreen_transition(&self, _window: Option<&mut Window>, completed: i32) {
            if completed != 0 {
                relayout(&self.label);
            }
        }

        // CEF's flow: ask each page browser (this is what runs beforeunload and shows its dialog) and
        // refuse while any says no; when the user accepts, CEF closes the window again by itself.
        // A second request within CLOSE_FORCE of the first closes regardless, so a window can never get stuck.
        fn can_close(&self, _window: Option<&mut Window>) -> i32 {
            let now = Instant::now();
            let asked = CLOSE_ASKED.with(|c| c.borrow().get(&self.label).copied());
            if force_close(asked, now) {
                CLOSE_ASKED.with(|c| c.borrow_mut().remove(&self.label));
                return 1;
            }
            let bvs: Vec<BrowserView> = reg(|r| r.views.values().filter(|v| v.win == self.label && v.kind == Kind::Page && !v.app_page).map(|v| v.bv.clone()).collect());
            let mut all = true;
            for bv in bvs {
                if let Some(h) = bv.browser().and_then(|b| b.host()) {
                    all &= h.try_close_browser() != 0;
                }
            }
            if !all {
                CLOSE_ASKED.with(|c| c.borrow_mut().insert(self.label.clone(), now));
            }
            all as i32
        }

        fn is_frameless(&self, _window: Option<&mut Window>) -> i32 {
            1
        }

        // A frameless window is not resizable unless it says so.
        fn can_resize(&self, _window: Option<&mut Window>) -> i32 {
            1
        }

        fn can_maximize(&self, _window: Option<&mut Window>) -> i32 {
            1
        }

        fn can_minimize(&self, _window: Option<&mut Window>) -> i32 {
            1
        }

        fn with_standard_window_buttons(&self, _window: Option<&mut Window>) -> i32 {
            1
        }

        fn window_runtime_style(&self) -> RuntimeStyle {
            RuntimeStyle::CHROME
        }
    }
}

/// A top-level window with its UI view. `private` windows get the incognito marker in the URL.
pub fn create_window(label: &str, private: bool) -> bool {
    let url = format!("{SCHEME}://{}/index.html?win={label}", scheme::HOST);
    let _ = private;
    let settings = BrowserSettings { background_color: background(Kind::Ui), ..Default::default() };
    let mut delegate = PageViewDelegate::new(tabs::wv_label("ui", label), false);
    let Some(ui) = browser_view_create(Some(&mut client(&tabs::wv_label("ui", label))), Some(&s(&url)), Some(&settings), None, None, Some(&mut delegate)) else {
        return false;
    };
    // Registered before the first navigation starts, or the request handler would block it.
    reg(|r| r.views.insert(tabs::wv_label("ui", label), Rec { bv: ui.clone(), win: label.to_string(), kind: Kind::Ui, app_page: true, bounds: (0.0, 0.0, 0.0, 0.0) }));
    let mut wd = WinDelegate::new(label.to_string(), ui);
    window_create_top_level(Some(&mut wd)).is_some()
}

// ---------- clients and handlers ----------

wrap_client! {
    struct ViewClient {
        label: String,
    }

    impl Client {
        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(Life::new(self.label.clone()))
        }

        fn request_handler(&self) -> Option<RequestHandler> {
            Some(Requests::new(self.label.clone()))
        }

        fn load_handler(&self) -> Option<LoadHandler> {
            Some(Loads::new(self.label.clone()))
        }

        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(Display::new(self.label.clone()))
        }

        fn download_handler(&self) -> Option<DownloadHandler> {
            Some(Downloads::new(self.label.clone()))
        }

        fn context_menu_handler(&self) -> Option<ContextMenuHandler> {
            Some(ContextMenus::new(self.label.clone()))
        }

        fn keyboard_handler(&self) -> Option<KeyboardHandler> {
            Some(Keys::new(0))
        }

        fn focus_handler(&self) -> Option<FocusHandler> {
            Some(Focus::new(self.label.clone()))
        }

        fn permission_handler(&self) -> Option<PermissionHandler> {
            Some(Permissions::new(self.label.clone()))
        }

        fn drag_handler(&self) -> Option<DragHandler> {
            Some(Drags::new(self.label.clone()))
        }

        fn on_process_message_received(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            source_process: ProcessId,
            message: Option<&mut ProcessMessage>,
        ) -> i32 {
            if let (Some(b), Some(m)) = (browser.as_ref(), message.as_ref()) {
                if CefString::from(&m.name()).to_string() == "aditus-pid" {
                    if let (Some(label), Some(args)) = (reg(|r| r.by_browser.get(&b.identifier()).cloned()), m.argument_list()) {
                        tabs::set_pid(&tabs::AppHandle, &label, args.int(0) as u32);
                    }
                    return 1;
                }
            }
            ipc::router().on_process_message_received(browser.cloned(), frame.cloned(), source_process, message.cloned()) as i32
        }
    }
}

wrap_life_span_handler! {
    struct Life {
        label: String,
    }

    impl LifeSpanHandler {
        fn on_after_created(&self, browser: Option<&mut Browser>) {
            let Some(b) = browser else { return };
            reg(|r| r.by_browser.insert(b.identifier(), self.label.clone()));
            tabs::on_view_ready(&tabs::AppHandle, &self.label);
            crate::commands::suggest_ready(&self.label);
        }

        fn on_before_popup(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            _popup_id: i32,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            _target_disposition: WindowOpenDisposition,
            user_gesture: i32,
            _popup_features: Option<&PopupFeatures>,
            _window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut i32>,
        ) -> i32 {
            // Never a native popup window: it becomes a tab beside the opener.
            // Known limit: this is a brand-new browser, so `window.opener`, `postMessage` to the opener and
            // `window.close()` do not work (popup sign-ins and 3-D Secure rely on them; `nav::AUTH_HOSTS` papers
            // over the profile side of that). Returning 0 here instead would give a real related popup, and the
            // views API can hand it over (`delegate_for_popup_browser_view` + `on_popup_browser_view_created`, the
            // way DevTools is docked above) with a client of our own. Not done: it needs a tab that owns an
            // existing view, `window.close()` -> tab close, and no sleep/profile swap of the opener, none of which
            // can be checked without running CEF (popup teardown already crashes in this codebase, see `close`).
            let url = target_url.map(|u| u.to_string()).unwrap_or_default();
            // Ad popups: scripted `window.open` without a click, or a target the filter lists block.
            let source = frame.map(|f| CefString::from(&f.url()).to_string()).unwrap_or_default();
            if user_gesture == 0 || crate::adblock::check(&url, &source, "document") {
                return 1;
            }
            // Click-hijacking ads too (animeworld's even ask for a sized popup window, so that is no sign of a
            // sign-in). Delayed so the click's page ping, sent just before but itself deferred, has landed.
            let label = self.label.clone();
            defer_after(50, move || {
                if !crate::adblock::hijack(&url, &source, &tabs::clicked_link(&tabs::AppHandle, &label)) {
                    tabs::on_popup(&tabs::AppHandle, &label, &url);
                }
            });
            1
        }

        fn do_close(&self, _browser: Option<&mut Browser>) -> i32 {
            0
        }

        fn on_before_close(&self, browser: Option<&mut Browser>) {
            let Some(b) = browser else { return };
            ipc::router().on_before_close(Some(b.clone()));
            reg(|r| r.by_browser.remove(&b.identifier()));
            maybe_quit();
        }
    }
}

fn url_of_request(request: Option<&mut Request>) -> String {
    request.map(|r| CefString::from(&r.url()).to_string()).unwrap_or_default()
}

fn is_app_url(url: &str) -> bool {
    url.starts_with(&format!("{SCHEME}://"))
}

wrap_request_handler! {
    struct Requests {
        label: String,
    }

    impl RequestHandler {
        fn on_before_browse(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            request: Option<&mut Request>,
            user_gesture: i32,
            is_redirect: i32,
        ) -> i32 {
            let url = url_of_request(request);
            let main = frame.as_ref().map_or(true, |f| f.is_main() != 0);
            let (kind, app_page) = reg(|r| r.views.get(&self.label).map(|v| (v.kind, v.app_page)).unwrap_or((Kind::Page, false)));
            if is_app_url(&url) {
                // Web pages must never reach the app's own pages (they would get every command).
                if kind == Kind::Page && !app_page {
                    return 1;
                }
            } else if kind != Kind::Page {
                // The UI never leaves the app scheme.
                return 1;
            } else if main && app_page {
                reg(|r| r.views.get_mut(&self.label).map(|v| v.app_page = false));
                set_page(&self.label, true);
            }
            if kind == Kind::Page && main && !is_app_url(&url) {
                if let Ok(u) = url::Url::parse(&url) {
                    NAV_HOST.with(|n| n.borrow_mut().insert(self.label.clone(), u.host_str().unwrap_or_default().to_string()));
                    if !tabs::on_navigation(&tabs::AppHandle, &self.label, &u, user_gesture != 0, is_redirect != 0) {
                        return 1;
                    }
                }
            }
            if kind == Kind::Page {
                if let Some(f) = frame.as_ref() {
                    send_scriptlets(f, &url, false);
                }
            }
            ipc::router().on_before_browse(browser.cloned(), frame.cloned());
            0
        }

        fn resource_request_handler(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _request: Option<&mut Request>,
            _is_navigation: i32,
            _is_download: i32,
            _initiator: Option<&CefString>,
            _disable_default_handling: Option<&mut i32>,
        ) -> Option<ResourceRequestHandler> {
            PAGES.lock().unwrap().contains(&self.label).then(Blocker::new)
        }

        fn auth_credentials(
            &self,
            _browser: Option<&mut Browser>,
            _origin_url: Option<&CefString>,
            is_proxy: i32,
            host: Option<&CefString>,
            _port: i32,
            realm: Option<&CefString>,
            _scheme: Option<&CefString>,
            callback: Option<&mut AuthCallback>,
        ) -> i32 {
            let Some(cb) = callback else { return 0 };
            if !reg(|r| r.views.get(&self.label).is_some_and(|v| v.kind == Kind::Page)) {
                return 0;
            }
            let host = host.map(|h| h.to_string()).unwrap_or_default();
            let realm = realm.map(|h| h.to_string()).unwrap_or_default();
            let detail = if is_proxy != 0 { format!("proxy {realm}") } else { realm };
            match tabs::on_net_prompt(&tabs::AppHandle, &self.label, false, &host, detail.trim()) {
                tabs::Net::Wait(id) => NET.with(|n| n.borrow_mut().insert(id, Pending::Auth(cb.clone()))),
                _ => {
                    cb.cancel();
                    None
                }
            };
            1
        }

        fn on_certificate_error(
            &self,
            browser: Option<&mut Browser>,
            cert_error: Errorcode,
            request_url: Option<&CefString>,
            _ssl_info: Option<&mut Sslinfo>,
            callback: Option<&mut Callback>,
        ) -> i32 {
            let Some(cb) = callback else { return 0 };
            if !reg(|r| r.views.get(&self.label).is_some_and(|v| v.kind == Kind::Page)) {
                return 0;
            }
            // Only the page being navigated to is worth asking about; sub-resources fail quietly.
            let url = request_url.map(|u| u.to_string()).unwrap_or_default();
            let Some(host) = url::Url::parse(&url).ok().and_then(|u| u.host_str().map(String::from)) else { return 0 };
            if NAV_HOST.with(|n| n.borrow().get(&self.label).cloned()).as_deref() != Some(host.as_str()) {
                return 0;
            }
            let text = tabs::cert_error_text(sys::cef_errorcode_t::from(cert_error) as i32);
            match tabs::on_net_prompt(&tabs::AppHandle, &self.label, true, &host, text) {
                tabs::Net::Allow => cb.cont(),
                tabs::Net::Wait(id) => {
                    NET.with(|n| n.borrow_mut().insert(id, Pending::Cert(cb.clone())));
                }
                tabs::Net::Deny => cb.cancel(),
            }
            1
        }

        fn on_render_process_terminated(&self, browser: Option<&mut Browser>, _status: TerminationStatus, _code: i32, _text: Option<&CefString>) {
            ipc::router().on_render_process_terminated(browser.cloned());
            tabs::on_crash(&tabs::AppHandle, &self.label);
        }
    }
}

/// Engine callbacks waiting for the user's answer in the sidebar, by prompt id (UI thread only).
enum Pending {
    Auth(AuthCallback),
    Cert(Callback),
}

thread_local! {
    /// Host each page view last started a main-frame navigation to.
    static NAV_HOST: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    static NET: RefCell<HashMap<u64, Pending>> = RefCell::new(HashMap::new());
}

/// Finish a held login or certificate request. Without `allow` it is cancelled.
pub fn net_answer(id: u64, allow: bool, user: &str, pass: &str) {
    match NET.with(|n| n.borrow_mut().remove(&id)) {
        Some(Pending::Auth(cb)) if allow => cb.cont(Some(&s(user)), Some(&s(pass))),
        Some(Pending::Auth(cb)) => cb.cancel(),
        Some(Pending::Cert(cb)) if allow => cb.cont(),
        Some(Pending::Cert(cb)) => cb.cancel(),
        None => {}
    }
}

wrap_resource_request_handler! {
    struct Blocker;

    impl ResourceRequestHandler {
        fn on_before_resource_load(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            request: Option<&mut Request>,
            _callback: Option<&mut Callback>,
        ) -> ReturnValue {
            // IO thread: no registry, no tab state; the ad-block engine has its own locks.
            let (Some(browser), Some(request)) = (browser, request) else { return ReturnValue::CONTINUE };
            let kind = match request.resource_type() {
                ResourceType::MAIN_FRAME => return ReturnValue::CONTINUE,
                ResourceType::SUB_FRAME => "sub_frame",
                ResourceType::STYLESHEET => "stylesheet",
                ResourceType::SCRIPT | ResourceType::WORKER | ResourceType::SHARED_WORKER | ResourceType::SERVICE_WORKER => "script",
                ResourceType::IMAGE | ResourceType::FAVICON => "image",
                ResourceType::FONT_RESOURCE => "font",
                ResourceType::MEDIA => "media",
                ResourceType::XHR => "xmlhttprequest",
                ResourceType::PING => "ping",
                ResourceType::OBJECT => "object",
                _ => "other",
            };
            let url = CefString::from(&request.url()).to_string();
            let source = browser.main_frame().map(|f| CefString::from(&f.url()).to_string()).unwrap_or_default();
            if crate::adblock::check(&url, &source, kind) {
                ReturnValue::CANCEL
            } else {
                ReturnValue::CONTINUE
            }
        }
    }
}

/// Our own error page; the tab keeps the failed URL as its address (see `tabs::on_load`).
pub fn error_page(failed: &str, code: &str) -> String {
    let q = url::form_urlencoded::Serializer::new(String::new()).append_pair("u", failed).append_pair("e", code).finish();
    format!("{SCHEME}://{}/error.html?{q}", scheme::HOST)
}

wrap_load_handler! {
    struct Loads {
        label: String,
    }

    impl LoadHandler {
        fn on_load_start(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, t: TransitionType) {
            let Some(f) = frame else { return };
            if f.is_main() != 0 {
                // TT_FORWARD_BACK_FLAG: the page (or CEF) traversed history, rather than navigating to a new page.
                let fwd_back = t.get_raw() & 0x0100_0000 != 0;
                tabs::on_load(&tabs::AppHandle, &self.label, CefString::from(&f.url()).to_string(), true, fwd_back);
            } else {
                tabs::frame_reset(&tabs::AppHandle, &self.label, &CefString::from(&f.identifier()).to_string());
            }
        }

        fn on_load_end(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, _status: i32) {
            let Some(f) = frame else { return };
            if f.is_main() != 0 {
                tabs::on_load(&tabs::AppHandle, &self.label, CefString::from(&f.url()).to_string(), false, false);
                crate::commands::suggest_ready(&self.label);
            }
        }

        fn on_load_error(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            error_code: Errorcode,
            error_text: Option<&CefString>,
            failed_url: Option<&CefString>,
        ) {
            let Some(f) = frame else { return };
            if f.is_main() == 0 || sys::cef_errorcode_t::from(error_code) == sys::cef_errorcode_t::ERR_ABORTED {
                return;
            }
            let text = error_text.map(|t| t.to_string()).unwrap_or_default();
            let failed = failed_url.map(|t| t.to_string()).unwrap_or_default();
            f.load_url(Some(&s(&error_page(&failed, &text))));
        }
    }
}

wrap_display_handler! {
    struct Display {
        label: String,
    }

    impl DisplayHandler {
        fn on_title_change(&self, _browser: Option<&mut Browser>, title: Option<&CefString>) {
            tabs::on_title(&tabs::AppHandle, &self.label, title.map(|t| t.to_string()).unwrap_or_default());
        }

        fn on_fullscreen_mode_change(&self, _browser: Option<&mut Browser>, fullscreen: i32) {
            let Some(win) = win_of(&self.label) else { return };
            let label = self.label.clone();
            reg(|r| {
                if fullscreen != 0 {
                    r.fullscreen.insert(win.clone(), label);
                } else {
                    r.fullscreen.remove(&win);
                }
            });
            if let Some(w) = window(&win) {
                w.set_fullscreen(fullscreen);
            }
            relayout(&win);
        }
    }
}

fn downloads_dir() -> PathBuf {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
    home.join("Downloads")
}

/// `dir/name`, or `dir/name (1).ext` if taken.
fn unique(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let p = Path::new(name);
    let (stem, ext) = (p.file_stem().and_then(|s| s.to_str()).unwrap_or(name), p.extension().and_then(|s| s.to_str()));
    (1..)
        .map(|n| dir.join(match ext {
            Some(e) => format!("{stem} ({n}).{e}"),
            None => format!("{stem} ({n})"),
        }))
        .find(|p| !p.exists())
        .unwrap()
}

wrap_download_handler! {
    struct Downloads {
        label: String,
    }

    impl DownloadHandler {
        fn can_download(&self, _browser: Option<&mut Browser>, _url: Option<&CefString>, _method: Option<&CefString>) -> i32 {
            1
        }

        fn on_before_download(
            &self,
            browser: Option<&mut Browser>,
            item: Option<&mut DownloadItem>,
            suggested_name: Option<&CefString>,
            callback: Option<&mut BeforeDownloadCallback>,
        ) -> i32 {
            let (Some(item), Some(cb)) = (item, callback) else { return 0 };
            // No user gesture behind it (drive-by download): not calling `cont` cancels it.
            if !tabs::download_allowed(&tabs::AppHandle, &self.label) {
                return 0;
            }
            let custom = tabs::download_dir();
            let dir = custom.filter(|d| std::fs::create_dir_all(d).is_ok()).unwrap_or_else(downloads_dir);
            let name = suggested_name.map(|n| n.to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| "download".into());
            let path = unique(&dir, &name.replace(['/', '\\'], "_")).to_string_lossy().into_owned();
            let url = CefString::from(&item.url()).to_string();
            // Private windows leave no record: untracked here means on_download_updated skips it too.
            let private = browser.and_then(|b| reg(|r| r.by_browser.get(&b.identifier()).cloned())).is_some_and(|l| tabs::label_private(&tabs::AppHandle, &l));
            if !private {
                let id = tabs::on_download_start(&tabs::AppHandle, &url, &path);
                reg(|r| r.downloads.insert(item.id(), (id, path.clone())));
            }
            cb.cont(Some(&s(&path)), 0);
            1
        }

        fn on_download_updated(&self, _browser: Option<&mut Browser>, item: Option<&mut DownloadItem>, _callback: Option<&mut DownloadItemCallback>) {
            let Some(item) = item else { return };
            let done = item.is_complete() != 0;
            if !done && item.is_canceled() == 0 && item.is_interrupted() == 0 {
                return;
            }
            if let Some((id, path)) = reg(|r| r.downloads.remove(&item.id())) {
                tabs::on_download_end(&tabs::AppHandle, id, &path, done);
            }
        }
    }
}

// ---- permissions ----

wrap_keyboard_handler! {
    struct Keys {
        _unused: i32,
    }

    impl KeyboardHandler {
        // Windows and Linux have no menu bar to carry accelerators, so keys are matched here.
        fn on_pre_key_event(
            &self,
            browser: Option<&mut Browser>,
            event: Option<&KeyEvent>,
            _os_event: *mut u8,
            _is_keyboard_shortcut: Option<&mut i32>,
        ) -> i32 {
            let Some(e) = event else { return 0 };
            // CEF has no browser chrome to leave HTML fullscreen (YouTube, any video): Esc does it.
            if e.type_ == KeyEventType::RAWKEYDOWN && e.windows_key_code == 0x1B {
                if let Some(b) = browser {
                    let label = reg(|r| r.by_browser.get(&b.identifier()).cloned());
                    if let Some(l) = label.filter(|l| reg(|r| r.fullscreen.values().any(|v| v == l))) {
                        defer(move || eval(&l, "document.exitFullscreen()"));
                        return 1;
                    }
                }
            }
            if cfg!(target_os = "macos") {
                return 0;
            }
            if e.type_ != KeyEventType::RAWKEYDOWN {
                return 0;
            }
            let Some(id) = crate::menu::shortcut(e.modifiers, e.windows_key_code) else { return 0 };
            defer(move || crate::menu::picked(&id));
            1
        }
    }
}

wrap_permission_handler! {
    struct Permissions {
        label: String,
    }

    impl PermissionHandler {
        fn on_request_media_access_permission(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            origin: Option<&CefString>,
            requested: u32,
            callback: Option<&mut MediaAccessCallback>,
        ) -> i32 {
            let Some(cb) = callback else { return 0 };
            let origin = origin.map(|o| o.to_string()).unwrap_or_default();
            let mut allowed = 0u32;
            let mut all = true;
            for (bit, kind) in [(1u32, "microphone"), (2u32, "camera")] {
                if requested & bit != 0 {
                    if tabs::on_permission(&tabs::AppHandle, &self.label, &origin, kind) {
                        allowed |= bit;
                    } else {
                        all = false;
                    }
                }
            }
            if all && allowed != 0 {
                cb.cont(allowed);
            } else {
                cb.cancel();
            }
            1
        }

        fn on_show_permission_prompt(
            &self,
            _browser: Option<&mut Browser>,
            _prompt_id: u64,
            origin: Option<&CefString>,
            requested: u32,
            callback: Option<&mut PermissionPromptCallback>,
        ) -> i32 {
            let Some(cb) = callback else { return 0 };
            let kind = if requested & 256 != 0 {
                "location"
            } else if requested & 32768 != 0 {
                "notifications"
            } else if requested & 4096 != 0 {
                "microphone"
            } else if requested & 4 != 0 {
                "camera"
            } else {
                return 0;
            };
            let origin = origin.map(|o| o.to_string()).unwrap_or_default();
            let ok = tabs::on_permission(&tabs::AppHandle, &self.label, &origin, kind);
            cb.cont(if ok { PermissionRequestResult::ACCEPT } else { PermissionRequestResult::DENY });
            1
        }
    }
}

// ---- dragging the frameless window by the UI's `-webkit-app-region: drag` areas ----

wrap_focus_handler! {
    struct Focus {
        label: String,
    }

    impl FocusHandler {
        // The suggestion list must never take the keyboard from the address input being typed in.
        fn on_set_focus(&self, _browser: Option<&mut Browser>, _source: FocusSource) -> i32 {
            self.label.starts_with("suggest") as i32
        }
    }
}

wrap_drag_handler! {
    struct Drags {
        label: String,
    }

    impl DragHandler {
        fn on_draggable_regions_changed(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, regions: Option<&[DraggableRegion]>) {
            let is_ui = reg(|r| r.views.get(&self.label).map(|v| v.kind == Kind::Ui)).unwrap_or(false);
            if !is_ui {
                return;
            }
            if let Some(w) = win_of(&self.label).and_then(|w| window(&w)) {
                w.set_draggable_regions(regions);
            }
        }
    }
}

// ---- right-click menu inside pages ----

pub const CTX_LINK_TAB: i32 = 26500;
pub const CTX_LINK_COPY: i32 = 26501;
pub const CTX_IMG_TAB: i32 = 26502;
pub const CTX_IMG_COPY: i32 = 26503;
pub const CTX_SEL_COPY: i32 = 26504;
pub const CTX_SEL_SEARCH: i32 = 26505;
pub const CTX_BACK: i32 = 26506;
pub const CTX_FORWARD: i32 = 26507;
pub const CTX_RELOAD: i32 = 26508;
pub const CTX_DEVTOOLS: i32 = 26509;
pub const CTX_LINK_WIN: i32 = 26510;
pub const CTX_LINK_PRIV: i32 = 26511;
pub const CTX_IMG_SAVE: i32 = 26512;
pub const CTX_PRINT: i32 = 26513;
pub const CTX_SOURCE: i32 = 26514;
pub const CTX_PIP: i32 = 26515;

thread_local! {
    /// Spelling suggestions of the context menu that is open (ids SPELLCHECK_SUGGESTION_0..4 index into it) and the word.
    static SPELL: RefCell<(String, Vec<String>)> = RefCell::new((String::new(), Vec::new()));
}

wrap_context_menu_handler! {
    struct ContextMenus {
        label: String,
    }

    impl ContextMenuHandler {
        fn on_before_context_menu(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, params: Option<&mut ContextMenuParams>, model: Option<&mut MenuModel>) {
            let (Some(p), Some(m)) = (params, model) else { return };
            let kind = reg(|r| r.views.get(&self.label).map(|v| (v.kind, v.app_page)));
            m.clear();
            // Only tab content gets a menu; the UI is HTML with its own controls.
            let Some((Kind::Page, _)) = kind else { return };
            if p.is_editable() != 0 {
                let word = CefString::from(&p.misspelled_word()).to_string();
                if !word.is_empty() {
                    let mut list = CefStringList::new();
                    p.dictionary_suggestions(Some(&mut list));
                    let sug: Vec<String> = list.into_iter().take(5).collect();
                    for (i, w) in sug.iter().enumerate() {
                        m.add_item(sys::cef_menu_id_t::MENU_ID_SPELLCHECK_SUGGESTION_0 as i32 + i as i32, Some(&s(w)));
                    }
                    if sug.is_empty() {
                        m.add_item(sys::cef_menu_id_t::MENU_ID_NO_SPELLING_SUGGESTIONS as i32, Some(&s("No spelling suggestions")));
                        m.set_enabled(sys::cef_menu_id_t::MENU_ID_NO_SPELLING_SUGGESTIONS as i32, 0);
                    }
                    m.add_item(sys::cef_menu_id_t::MENU_ID_ADD_TO_DICTIONARY as i32, Some(&s("Add to Dictionary")));
                    m.add_separator();
                    SPELL.with(|c| *c.borrow_mut() = (word, sug));
                }
                for (id, name) in [(sys::cef_menu_id_t::MENU_ID_UNDO, "Undo"), (sys::cef_menu_id_t::MENU_ID_REDO, "Redo")] {
                    m.add_item(id as i32, Some(&s(name)));
                }
                m.add_separator();
                for (id, name) in [(sys::cef_menu_id_t::MENU_ID_CUT, "Cut"), (sys::cef_menu_id_t::MENU_ID_COPY, "Copy"), (sys::cef_menu_id_t::MENU_ID_PASTE, "Paste"), (sys::cef_menu_id_t::MENU_ID_SELECT_ALL, "Select All")] {
                    m.add_item(id as i32, Some(&s(name)));
                }
                return;
            }
            let web = |u: String| (u.starts_with("http://") || u.starts_with("https://")) && u.len() < 8192;
            let link = CefString::from(&p.unfiltered_link_url()).to_string();
            let src = CefString::from(&p.source_url()).to_string();
            let selection = CefString::from(&p.selection_text()).to_string();
            let is_img = p.media_type() == ContextMenuMediaType::IMAGE;
            let mut any = false;
            if p.media_type() == ContextMenuMediaType::VIDEO {
                m.add_item(CTX_PIP, Some(&s("Picture in Picture")));
                m.add_separator();
            }
            if web(link.clone()) {
                m.add_item(CTX_LINK_TAB, Some(&s("Open Link in New Tab")));
                m.add_item(CTX_LINK_WIN, Some(&s("Open Link in New Window")));
                m.add_item(CTX_LINK_PRIV, Some(&s("Open Link in Private Window")));
                m.add_item(CTX_LINK_COPY, Some(&s("Copy Link Address")));
                any = true;
            }
            if is_img && web(src.clone()) {
                if any {
                    m.add_separator();
                }
                m.add_item(CTX_IMG_TAB, Some(&s("Open Image in New Tab")));
                m.add_item(CTX_IMG_COPY, Some(&s("Copy Image Address")));
                m.add_item(CTX_IMG_SAVE, Some(&s("Save Image As…")));
                any = true;
            }
            if !selection.trim().is_empty() {
                if any {
                    m.add_separator();
                }
                let short: String = selection.trim().chars().take(24).collect();
                m.add_item(CTX_SEL_COPY, Some(&s("Copy")));
                m.add_item(CTX_SEL_SEARCH, Some(&s(&format!("Search for “{short}”"))));
                any = true;
            }
            if any {
                m.add_separator();
            }
            let (back, fwd) = tabs::nav_state(&tabs::AppHandle, &self.label);
            m.add_item(CTX_BACK, Some(&s("Back")));
            m.set_enabled(CTX_BACK, back as i32);
            m.add_item(CTX_FORWARD, Some(&s("Forward")));
            m.set_enabled(CTX_FORWARD, fwd as i32);
            m.add_item(CTX_RELOAD, Some(&s("Reload")));
            m.add_separator();
            m.add_item(CTX_PRINT, Some(&s("Print…")));
            m.add_item(CTX_SOURCE, Some(&s("View Page Source")));
            m.add_item(CTX_DEVTOOLS, Some(&s("Inspect")));
            crate::commands::remember_page_context(&self.label, web(link.clone()).then_some(link), (is_img && web(src.clone())).then_some(src), selection);
        }

        fn on_context_menu_command(&self, browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _params: Option<&mut ContextMenuParams>, command: i32, _flags: EventFlags) -> i32 {
            let first = sys::cef_menu_id_t::MENU_ID_SPELLCHECK_SUGGESTION_0 as i32;
            let host = browser.and_then(|b| b.host());
            if let (Some(h), true) = (&host, (first..first + 5).contains(&command)) {
                if let Some(w) = SPELL.with(|c| c.borrow().1.get((command - first) as usize).cloned()) {
                    h.replace_misspelling(Some(&s(&w)));
                }
                return 1;
            }
            if let (Some(h), true) = (&host, command == sys::cef_menu_id_t::MENU_ID_ADD_TO_DICTIONARY as i32) {
                h.add_word_to_dictionary(Some(&s(&SPELL.with(|c| c.borrow().0.clone()))));
                return 1;
            }
            if (CTX_LINK_TAB..=CTX_PIP).contains(&command) {
                crate::commands::page_context_action(&self.label, command);
                return 1;
            }
            0
        }
    }
}

// ---------- native popup menus (tab / profile menus) ----------

pub enum Item {
    Sep,
    Entry { id: String, label: String, enabled: bool, checked: bool },
    Sub { label: String, enabled: bool, items: Vec<Item> },
}

wrap_menu_model_delegate! {
    struct Picks {
        ids: std::rc::Rc<RefCell<Vec<String>>>,
    }

    impl MenuModelDelegate {
        fn execute_command(&self, _menu: Option<&mut MenuModel>, command: i32, _flags: EventFlags) {
            let id = self.ids.borrow().get((command - 1) as usize).cloned();
            if let Some(id) = id {
                defer(move || crate::menu::picked(&id));
            }
        }
    }
}

fn fill(model: &MenuModel, items: Vec<Item>, ids: &std::rc::Rc<RefCell<Vec<String>>>) {
    for it in items {
        match it {
            Item::Sep => {
                model.add_separator();
            }
            Item::Entry { id, label, enabled, checked } => {
                ids.borrow_mut().push(id);
                let cmd = ids.borrow().len() as i32;
                if checked {
                    model.add_check_item(cmd, Some(&s(&label)));
                    model.set_checked(cmd, 1);
                } else {
                    model.add_item(cmd, Some(&s(&label)));
                }
                model.set_enabled(cmd, enabled as i32);
            }
            Item::Sub { label, enabled, items } => {
                // Sub-menu ids live above the entries' range.
                let cmd = 10_000 + ids.borrow().len() as i32;
                if let Some(sub) = model.add_sub_menu(cmd, Some(&s(&label))) {
                    model.set_enabled(cmd, enabled as i32);
                    fill(&sub, items, ids);
                }
            }
        }
    }
}

/// Shows a menu at window coordinates `(x, y)`; picks go to `menu::picked`.
pub fn popup(win: &str, items: Vec<Item>, at: (i32, i32)) {
    let Some(w) = window(win) else { return };
    let ids = std::rc::Rc::new(RefCell::new(vec![]));
    let mut delegate = Picks::new(ids.clone());
    let Some(model) = menu_model_create(Some(&mut delegate)) else { return };
    fill(&model, items, &ids);
    let origin = w.client_area_bounds_in_screen();
    let mut model_ref = model.clone();
    w.show_menu(Some(&mut model_ref), Some(&Point { x: origin.x + at.0, y: origin.y + at.1 }), MenuAnchorPosition::TOPLEFT);
    reg(|r| r.popup = Some((model, delegate)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_close_forces() {
        let t = Instant::now();
        assert!(!force_close(None, t));
        assert!(force_close(Some(t), t + Duration::from_secs(1)));
        assert!(!force_close(Some(t), t + CLOSE_FORCE + Duration::from_secs(1)));
    }
}
