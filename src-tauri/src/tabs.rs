//! Tab model and webview lifecycle.
//!
//! Lock discipline: never hold `Shared.app` while calling anything that blocks on the
//! main thread (`add_child`, `close`, ...). Handlers run on the main thread and take the
//! same lock, so holding it across those calls deadlocks.

use crate::{
    config::{self, Config},
    db::Db,
    nav::{self, Decision},
    search,
};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{
    webview::{DownloadEvent, NewWindowResponse, PageLoadEvent, PermissionKind, PermissionResponse},
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewBuilder, WebviewUrl,
};
use url::Url;

const LATE_WINDOW: Duration = Duration::from_millis(1500);
const GESTURE_WINDOW: Duration = Duration::from_millis(2500);
#[cfg(target_os = "macos")]
const SAFARI_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";
pub const NEWTAB: &str = "newtab.html";

pub struct Shared {
    pub app: Mutex<App>,
    pub db: Db,
    pub cfg_path: PathBuf,
    pub data_dir: PathBuf,
}

#[derive(Clone)]
pub struct Entry {
    pub url: String,
    pub profile: String,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Push,
    Traverse,
}

pub struct Tab {
    pub id: u32,
    pub win: String,
    pub profile: String,
    pub entries: Vec<Entry>,
    pub idx: usize,
    pub title: String,
    pub pinned: bool,
    pub group: Option<String>,
    pub discarded: bool,
    pub loading: bool,
    pub audible: bool,
    /// Private-window tab: no history, no saved session, no profile swaps, throwaway storage.
    pub private: bool,
    /// Page zoom in percent set with Cmd/Ctrl +/-/0; `None` follows the setting.
    pub zoom: Option<u32>,
    pub gen: u32,
    pub last_active: Instant,
    mode: Mode,
}

#[derive(Clone, Serialize)]
pub struct Toast {
    pub message: String,
    pub undo: bool,
}

#[derive(Clone)]
struct Undo {
    tab: u32,
    profile: String,
    url: String,
}

/// Per-window layout and selection. Tabs belong to a window via `Tab::win`.
pub struct Win {
    pub active: u32,
    pub sidebar: f64,
    pub top: f64,
    pub overlay_open: bool,
    pub incognito: bool,
}

pub struct App {
    pub cfg: Config,
    pub tabs: Vec<Tab>,
    pub wins: HashMap<String, Win>,
    /// Window that last had focus: menu actions and window-less callers act on it.
    pub focused: String,
    next_id: u32,
    next_win: u32,
    prompt: Option<String>,
    /// A site asked for a device permission with no saved answer: (host, kind, tab).
    perm_prompt: Option<(String, String, u32)>,
    /// Answers given in private windows; never written to disk.
    perm_tmp: Vec<config::PermRule>,
    skipped: HashSet<String>,
    toast: Option<Toast>,
    undo: Option<Undo>,
    gestures: HashMap<u32, Instant>,
    /// Navigation that would have swapped had the gesture ping arrived first (WebView2 orders them the other way).
    late: HashMap<u32, (String, String, Instant)>,
    /// Recently closed tabs (url, profile), newest last.
    closed: Vec<(String, String)>,
}

impl App {
    pub fn new(cfg: Config) -> Self {
        Self {
            cfg,
            tabs: vec![],
            wins: HashMap::new(),
            focused: "main".into(),
            next_id: 1,
            next_win: 2,
            prompt: None,
            perm_prompt: None,
            perm_tmp: vec![],
            skipped: HashSet::new(),
            toast: None,
            undo: None,
            gestures: HashMap::new(),
            late: HashMap::new(),
            closed: vec![],
        }
    }

    #[cfg(debug_assertions)]
    pub fn prompt_host(&self) -> Option<String> {
        self.prompt.clone()
    }

    fn tab(&mut self, id: u32) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    pub fn win(&mut self, w: &str) -> &mut Win {
        self.wins.entry(w.to_string()).or_insert(Win { active: 0, sidebar: 240.0, top: 52.0, overlay_open: false, incognito: false })
    }

    /// Selected tab of window `w`.
    pub fn active(&self, w: &str) -> u32 {
        self.wins.get(w).map_or(0, |x| x.active)
    }

    pub fn focused_active_with_win(&self) -> (String, u32) {
        (self.focused.clone(), self.focused_active())
    }

    pub fn focused_active(&self) -> u32 {
        self.active(&self.focused)
    }

    fn win_of(&self, id: u32) -> String {
        self.tabs.iter().find(|t| t.id == id).map_or_else(|| self.focused.clone(), |t| t.win.clone())
    }
}

/// Per-window webview label: the first window keeps the bare names.
pub fn wv_label(kind: &str, win: &str) -> String {
    if win == "main" { kind.to_string() } else { format!("{kind}-{win}") }
}

impl Tab {
    pub fn url(&self) -> String {
        self.entries.get(self.idx).map(|e| e.url.clone()).unwrap_or_default()
    }
}

fn shared(app: &AppHandle) -> &Shared {
    app.state::<Shared>().inner()
}

/// macOS: WebKit shrinks the viewport by the titlebar obscured inset (32pt) although our layout
/// already positions webviews below it. Zero the inset so the CSS viewport equals the frame.
#[cfg(target_os = "macos")]
pub fn no_inset(wv: &tauri::Webview) {
    let _ = wv.with_webview(|pw| unsafe {
        let v = &*(pw.inner() as *const objc2_web_kit::WKWebView);
        v.setObscuredContentInsets(objc2_foundation::NSEdgeInsets { top: 0.0, left: 0.0, bottom: 0.0, right: 0.0 });
    });
}
/// AppKit re-applies the inset on show/resize, so redo it for every webview then.
pub fn no_inset_all(app: &AppHandle) {
    for (_, wv) in app.webviews() {
        no_inset(&wv);
    }
}
#[cfg(not(target_os = "macos"))]
pub fn no_inset(_: &tauri::Webview) {}

/// Opens the inspector; it docks inside the page container (see `mount`), not across the window.
pub fn open_inspector(wv: &tauri::Webview) {
    wv.open_devtools();
    // WebKit remembers the last state; force docked so it never starts as a separate window.
    #[cfg(target_os = "macos")]
    {
        let wv = wv.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            let _ = wv.with_webview(|pw| unsafe {
                use objc2::{msg_send, runtime::AnyObject, sel};
                let v = &*(pw.inner() as *const AnyObject);
                let insp: *mut AnyObject = msg_send![v, _inspector];
                let ok = !insp.is_null() && msg_send![&*insp, respondsToSelector: sel!(attach)];
                eprintln!("[inspector] attach available: {ok}");
                if ok {
                    let _: () = msg_send![&*insp, attach];
                }
            });
        });
    }
}

/// macOS: WebKit docks the inspector across its superview and resizes the page to match. Page
/// webviews therefore live in a container view per window, covering just the content area.
#[cfg(target_os = "macos")]
unsafe fn is_container(view: *mut objc2::runtime::AnyObject) -> bool {
    use objc2::msg_send;
    let id: *mut objc2::runtime::AnyObject = msg_send![&*view, identifier];
    !id.is_null() && {
        let want = objc2_foundation::NSString::from_str("aditus-pages");
        msg_send![&*id, isEqual: &*want]
    }
}

/// Place the window's container over the content area and the page webview filling it. Idempotent.
#[cfg(target_os = "macos")]
unsafe fn mount(page: *mut std::ffi::c_void, pos: LogicalPosition<f64>, size: LogicalSize<f64>) {
    use objc2::{class, msg_send, runtime::AnyObject};
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    let v = &*(page as *const AnyObject);
    let sv: *mut AnyObject = msg_send![v, superview];
    if sv.is_null() {
        return;
    }
    let content: *mut AnyObject = if is_container(sv) { msg_send![&*sv, superview] } else { sv };
    let cf: NSRect = msg_send![&*content, frame];
    let frame = NSRect::new(NSPoint::new(pos.x, cf.size.height - pos.y - size.height), NSSize::new(size.width, size.height));
    let mut c: *mut AnyObject = std::ptr::null_mut();
    let subs: *mut AnyObject = msg_send![&*content, subviews];
    let n: usize = msg_send![&*subs, count];
    for i in 0..n {
        let sub: *mut AnyObject = msg_send![&*subs, objectAtIndex: i];
        if is_container(sub) {
            c = sub;
            break;
        }
    }
    if c.is_null() {
        let a: *mut AnyObject = msg_send![class!(NSView), alloc];
        c = msg_send![a, initWithFrame: frame];
        let ident = objc2_foundation::NSString::from_str("aditus-pages");
        let _: () = msg_send![&*c, setIdentifier: &*ident];
        let _: () = msg_send![&*content, addSubview: &*c];
    } else {
        let _: () = msg_send![&*c, setFrame: frame];
    }
    if sv != c {
        let _: () = msg_send![v, removeFromSuperview];
        let _: () = msg_send![&*c, addSubview: v];
    }
    let _: () = msg_send![v, setFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(size.width, size.height))];
}

/// Size a page webview to the content area.
fn fit_page(app: &AppHandle, wv: &tauri::Webview) {
    let (pos, size) = content_rect(app, wv.window().label());
    #[cfg(target_os = "macos")]
    let _ = wv.with_webview(move |pw| unsafe { mount(pw.inner(), pos, size) });
    #[cfg(not(target_os = "macos"))]
    let _ = wv.set_bounds(tauri::Rect { position: pos.into(), size: size.into() });
}

/// Host of the tab's current entry. Never ask the engine: wry panics (and aborts) when a fresh webview has no URL yet.
pub fn host_of(app: &AppHandle, wlabel: &str) -> String {
    let g = shared(app).app.lock().unwrap();
    g.tabs
        .iter()
        .find(|t| label(t.id, t.gen) == wlabel)
        .and_then(|t| Url::parse(&t.url()).ok())
        .and_then(|u| u.host_str().map(String::from))
        .unwrap_or_default()
}

pub fn label(id: u32, gen: u32) -> String {
    format!("tab-{id}-{gen}")
}

/// Browser pages (settings, history, downloads) live in ordinary tabs as `aditus://<view>`.
pub fn internal_view(url: &str) -> Option<String> {
    let u = Url::parse(url).ok()?;
    if u.scheme() == "aditus" {
        return u.host_str().map(String::from);
    }
    let local = u.scheme() == "tauri" || matches!((u.host_str(), u.port()), (Some("tauri.localhost"), _) | (Some("localhost"), Some(1420)));
    if !local || !matches!(u.path(), "/" | "/index.html") {
        return None;
    }
    u.query_pairs().find(|(k, _)| k == "view").map(|(_, v)| v.into_owned())
}

fn normalize(url: String) -> String {
    internal_view(&url).map(|v| format!("aditus://{v}")).unwrap_or(url)
}

pub fn open_internal(app: &AppHandle, win: &str, view: &str) {
    let url = format!("aditus://{view}");
    let existing = shared(app).app.lock().unwrap().tabs.iter().find(|t| t.win == win && t.url() == url).map(|t| t.id);
    match existing {
        Some(id) => activate(app, id),
        None => {
            open_tab_in(app, win, Url::parse(&url).ok(), None, true);
        }
    }
}

pub fn is_blank(url: &str) -> bool {
    url.is_empty() || url.contains(NEWTAB)
}

// ---------- snapshot for the UI ----------

#[derive(Serialize)]
struct TabView {
    id: u32,
    title: String,
    url: String,
    host: String,
    profile: String,
    pinned: bool,
    group: Option<String>,
    discarded: bool,
    loading: bool,
    can_back: bool,
    can_forward: bool,
}

#[derive(Serialize)]
struct Snapshot<'a> {
    win: &'a str,
    tabs: Vec<TabView>,
    active: u32,
    cfg: &'a Config,
    prompt: Option<String>,
    perm: Option<(&'a str, &'a str)>,
    toast: Option<Toast>,
    adblock_site: bool,
    incognito: bool,
}

pub fn emit_state(app: &AppHandle) {
    emit(app, true);
}

/// One snapshot per window, tagged with its label; each frontend keeps only its own.
fn emit(app: &AppHandle, save: bool) {
    let s = shared(app);
    let g = s.app.lock().unwrap();
    for (win, w) in &g.wins {
        let active_host = g
            .tabs
            .iter()
            .find(|t| t.id == w.active)
            .and_then(|t| Url::parse(&t.url()).ok())
            .and_then(|u| u.host_str().map(String::from))
            .unwrap_or_default();
        let snap = Snapshot {
            win,
            tabs: g
                .tabs
                .iter()
                .filter(|t| &t.win == win)
                .map(|t| {
                    let url = t.url();
                    let parsed = Url::parse(&url).ok();
                    TabView {
                        id: t.id,
                        title: if t.title.is_empty() { if is_blank(&url) { "New Tab".into() } else { url.clone() } } else { t.title.clone() },
                        host: if is_blank(&url) || internal_view(&url).is_some() { String::new() } else { parsed.as_ref().and_then(|u| u.host_str().map(String::from)).unwrap_or_default() },
                        url: if is_blank(&url) { String::new() } else { url },
                        profile: t.profile.clone(),
                        pinned: t.pinned,
                        group: t.group.clone(),
                        discarded: t.discarded,
                        loading: t.loading,
                        can_back: t.idx > 0,
                        can_forward: t.idx + 1 < t.entries.len(),
                    }
                })
                .collect(),
            active: w.active,
            cfg: &g.cfg,
            prompt: g.prompt.clone(),
            perm: g.perm_prompt.as_ref().filter(|p| g.win_of(p.2) == *win).map(|p| (p.0.as_str(), p.1.as_str())),
            toast: g.toast.clone(),
            adblock_site: !g.cfg.allowlist.contains(&active_host),
            incognito: w.incognito,
        };
        let _ = app.emit("state", &snap);
    }
    drop(g);
    if save {
        save_session(app);
    }
}

fn save_cfg(app: &AppHandle) {
    let s = shared(app);
    let g = s.app.lock().unwrap();
    g.cfg.save(&s.cfg_path);
}

// ---------- layout ----------

pub fn content_rect(app: &AppHandle, win: &str) -> (LogicalPosition<f64>, LogicalSize<f64>) {
    let (sidebar, top) = {
        let mut g = shared(app).app.lock().unwrap();
        let w = g.win(win);
        (w.sidebar, w.top)
    };
    let (w, h) = app
        .get_window(win)
        .and_then(|win| {
            let sf = win.scale_factor().ok()?;
            let s = win.inner_size().ok()?;
            Some((s.width as f64 / sf, s.height as f64 / sf))
        })
        .unwrap_or((1280.0, 800.0));
    (
        LogicalPosition::new(sidebar, top),
        LogicalSize::new((w - sidebar).max(1.0), (h - top).max(1.0)),
    )
}

pub fn apply_layout(app: &AppHandle) {
    no_inset_all(app);
    let wins: Vec<(String, Option<(u32, u32)>)> = {
        let g = shared(app).app.lock().unwrap();
        g.wins
            .iter()
            .map(|(name, w)| (name.clone(), g.tabs.iter().find(|t| t.id == w.active).map(|t| (t.id, t.gen))))
            .collect()
    };
    for (win, tab) in wins {
        let (pos, size) = content_rect(app, &win);
        if let Some((id, gen)) = tab {
            if let Some(wv) = app.get_webview(&label(id, gen)) {
                fit_page(app, &wv);
            }
        }
        if let Some(wv) = app.get_webview(&wv_label("panel", &win)) {
            let _ = wv.set_bounds(tauri::Rect { position: pos.into(), size: size.into() });
        }
    }
}

/// Manual profile change for the active tab (no mapping change).
pub fn switch_profile(app: &AppHandle, tab_id: u32, profile: &str) {
    let url = {
        let mut g = shared(app).app.lock().unwrap();
        match g.tab(tab_id) {
            Some(t) if t.profile != profile => t.url(),
            _ => return,
        }
    };
    if is_blank(&url) {
        let old = {
            let mut g = shared(app).app.lock().unwrap();
            let active = g.active(&g.win_of(tab_id));
            let t = g.tab(tab_id).unwrap();
            let old = label(t.id, t.gen);
            t.gen += 1;
            t.profile = profile.into();
            (old, active == tab_id)
        };
        if let Some(wv) = app.get_webview(&old.0) {
            let _ = wv.close();
        }
        let _ = build_webview(app, tab_id, old.1);
        emit_state(app);
    } else {
        swap(app, tab_id, profile, &url, true);
    }
}

// ---------- webview creation ----------

/// Injected into every page webview. Pings are the only IPC page webviews may use; Rust
/// never trusts their payload (it reads the real URL from the webview instead).
const PAGE_SCRIPT: &str = r#"(()=>{
if(window.__aditus)return;window.__aditus=1;
const ping=k=>{try{window.__TAURI_INTERNALS__.invoke('page_ping',{kind:k})}catch(e){}};
addEventListener('click',()=>ping('g'),true);
addEventListener('keydown',e=>{if(e.key==='Enter')ping('g')},true);
addEventListener('submit',()=>ping('g'),true);
for(const m of ['pushState','replaceState']){const o=history[m];history[m]=function(){const r=o.apply(this,arguments);ping('u');return r}}
addEventListener('popstate',()=>ping('u'));addEventListener('hashchange',()=>ping('u'));
addEventListener('contextmenu',e=>{if(e.defaultPrevented)return;const el=e.target;if(!el||!el.closest)return;const ed=el.closest('input,textarea,[contenteditable=""],[contenteditable="true"]');if(ed&&!/Mac/.test(navigator.platform))return;const a=el.closest('a[href]'),im=el.closest('img');e.preventDefault();try{window.__TAURI_INTERNALS__.invoke('page_context',{link:a&&typeof a.href==='string'?a.href:null,image:im?im.currentSrc||im.src:null,selection:String(getSelection()||'').slice(0,2000),editable:!!ed})}catch(_){}});
addEventListener('playing',()=>ping('p1'),true);
addEventListener('pause',()=>ping('p0'),true);
addEventListener('ended',()=>ping('p0'),true);
if(window.top===window){try{window.__TAURI_INTERNALS__.invoke('cosmetic_css').then(css=>{if(!css)return;const s=document.createElement('style');s.textContent=css;(document.head||document.documentElement).appendChild(s)}).catch(()=>{})}catch(e){}}
})();"#;

/// YouTube serves ads from its own origin, so network and cosmetic filters can't touch them:
/// strip ad data from player responses and skip any ad that still plays.
/// ponytail: hand-written, breaks when YouTube renames fields; port uBO scriptlets if it keeps breaking.
/// Paused per tab via sessionStorage (set by adblock::refresh).
const YT_SCRIPT: &str = r#"(()=>{
if(!/(^|\.)youtube\.com$/.test(location.hostname))return;
try{if(sessionStorage.getItem('__aditus_off'))return}catch(e){}
const K=['adPlacements','playerAds','adSlots','adBreakParams','adBreakHeartbeatParams','adSafetyReason'];
const strip=(o,d=0)=>{if(o&&typeof o==='object'&&d<5){for(const k of K)if(k in o){try{delete o[k]}catch(e){}console.debug('[aditus] stripped',k)}for(const k in o)if(o[k]&&typeof o[k]==='object')strip(o[k],d+1)}return o};
let v;try{Object.defineProperty(window,'ytInitialPlayerResponse',{configurable:true,get:()=>v,set:x=>{v=strip(x)}})}catch(e){}
const p=JSON.parse;JSON.parse=function(){return strip(p.apply(this,arguments))};
const f=window.fetch;window.fetch=function(...a){return f.apply(this,a).then(r=>{try{const u=String(a[0]&&a[0].url||a[0]);if(!/\/youtubei\/v1\/(player|next|browse|search|reel)/.test(u))return r;return r.clone().json().then(j=>new Response(JSON.stringify(strip(j)),{status:r.status,statusText:r.statusText,headers:r.headers})).catch(()=>r)}catch(e){return r}})};
const st=document.createElement('style');st.textContent='#movie_player.ad-showing video{opacity:0!important}.video-ads,.ytp-ad-module,.ytp-ad-overlay-container,#player-ads,ytd-ad-slot-renderer,ytd-in-feed-ad-layout-renderer,ytd-display-ad-renderer,ytd-promoted-sparkles-web-renderer{display:none!important}';
const add=()=>(document.head||document.documentElement).appendChild(st);document.documentElement?add():addEventListener('DOMContentLoaded',add);
let was=0;const skip=()=>{const m=document.querySelector('#movie_player.ad-showing');if(m&&!was)console.debug('[aditus] ad playing, skipping');if(!m){if(was){was=0;const pl=document.querySelector('#movie_player');if(pl&&pl.playVideo)pl.playVideo()}return}was=1;const vd=m.querySelector('video');if(vd&&isFinite(vd.duration)){vd.muted=true;vd.playbackRate=16;vd.currentTime=vd.duration}const b=document.querySelector('.ytp-skip-ad-button,.ytp-ad-skip-button-modern,.ytp-ad-skip-button');if(b)b.click()};
new MutationObserver(skip).observe(document,{subtree:true,attributes:true,attributeFilter:['class']});for(const ev of ['loadstart','loadedmetadata','durationchange','playing'])document.addEventListener(ev,skip,true);setInterval(skip,100);
})();"#;

fn build_webview(app: &AppHandle, tab_id: u32, show: bool) -> Result<(), String> {
    let (profile_name, gen, url, cfg_profile, win, private, zoom) = {
        let g = shared(app).app.lock().unwrap();
        let t = g.tabs.iter().find(|t| t.id == tab_id).ok_or("no tab")?;
        let p = g.cfg.profile(&t.profile).or(g.cfg.profiles.first()).ok_or("no profile")?.clone();
        (t.profile.clone(), t.gen, t.url(), p, t.win.clone(), t.private, t.zoom.unwrap_or(g.cfg.settings.page_zoom))
    };
    let window = app.get_window(&win).ok_or("no window")?;
    let adblock_on = shared(app).app.lock().unwrap().cfg.settings.adblock;
    let data_dir = config::Config::data_dir(&shared(app).data_dir, &cfg_profile);
    if !private {
        let _ = std::fs::create_dir_all(&data_dir);
    }

    let wurl = if let Some(view) = internal_view(&url) {
        WebviewUrl::App(format!("index.html?view={view}").into())
    } else if is_blank(&url) {
        WebviewUrl::App(NEWTAB.into())
    } else {
        WebviewUrl::External(Url::parse(&url).map_err(|e| e.to_string())?)
    };

    let (a_nav, a_title, a_load, a_new, a_dl, a_perm) =
        (app.clone(), app.clone(), app.clone(), app.clone(), app.clone(), app.clone());

    let mut builder = WebviewBuilder::new(label(tab_id, gen), wurl);
    // Embedded WKWebView omits the Safari token; YouTube serves a blank page without it.
    #[cfg(target_os = "macos")]
    {
        builder = builder.user_agent(SAFARI_UA);
    }
    // Private tabs get a throwaway store, never the profile's.
    builder = if private {
        builder.incognito(true)
    } else {
        builder.data_store_identifier(config::store_identifier(&cfg_profile)).data_directory(data_dir)
    };
    let mut builder = builder
        .initialization_script(PAGE_SCRIPT)
        .initialization_script(if adblock_on { YT_SCRIPT } else { "" })
        .on_navigation(move |u| on_navigation(&a_nav, tab_id, gen, u))
        .on_permission_request(move |wv, kind| on_permission(&a_perm, tab_id, wv.url().ok().and_then(|u| u.host_str().map(String::from)), kind))
        .on_document_title_changed(move |_, title| {
            let s = shared(&a_title);
            let url = {
                let mut g = s.app.lock().unwrap();
                match g.tab(tab_id) {
                    Some(t) if t.gen == gen => {
                        t.title = title.clone();
                        t.url()
                    }
                    _ => return,
                }
            };
            s.db.set_title(&url, &title);
            emit_state(&a_title);
        })
        .on_page_load(move |wv, p| {
            let url = p.url().to_string();
            let started = matches!(p.event(), PageLoadEvent::Started);
            let _ = wv;
            on_page_load(&a_load, tab_id, gen, url, started);
        })
        .on_new_window(move |u, _| {
            let (profile, win) = {
                let g = shared(&a_new).app.lock().unwrap();
                match g.tabs.iter().find(|t| t.id == tab_id) {
                    Some(t) => (Some(t.profile.clone()), t.win.clone()),
                    None => (None, g.focused.clone()),
                }
            };
            let app = a_new.clone();
            let front = !shared(&app).app.lock().unwrap().cfg.settings.links_background;
            tauri::async_runtime::spawn(async move {
                open_tab_in(&app, &win, Some(u), profile, front);
            });
            NewWindowResponse::Deny
        })
        .on_download(move |_, ev| match ev {
            DownloadEvent::Requested { url, destination } => {
                let custom = shared(&a_dl).app.lock().unwrap().cfg.settings.download_dir.clone();
                let custom = (!custom.is_empty()).then(|| PathBuf::from(custom)).filter(|d| std::fs::create_dir_all(d).is_ok());
                if let Some(dir) = custom.or_else(|| a_dl.path().download_dir().ok()) {
                    let name = destination.file_name().map(|n| n.to_owned()).unwrap_or_else(|| "download".into());
                    *destination = dir.join(name);
                }
                shared(&a_dl).db.add_download(url.as_str(), &destination.to_string_lossy());
                let _ = a_dl.emit("downloads-changed", ());
                true
            }
            DownloadEvent::Finished { url, path, success } => {
                shared(&a_dl).db.finish_download(
                    url.as_str(),
                    &path.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
                    success,
                );
                let _ = a_dl.emit("downloads-changed", ());
                true
            }
            _ => true,
        });

    if !cfg!(debug_assertions) {
        builder = builder.devtools(true);
    }

    let (pos, size) = content_rect(app, &win);
    let wv = window.add_child(builder, pos, size).map_err(|e| e.to_string())?;
    no_inset(&wv);
    fit_page(app, &wv);
    if zoom != 100 {
        let _ = wv.set_zoom(zoom as f64 / 100.0);
    }
    crate::adblock::attach(app, &wv, &profile_name);
    if !show {
        let _ = wv.hide();
    }
    Ok(())
}

fn on_navigation(app: &AppHandle, tab_id: u32, gen: u32, u: &Url) -> bool {
    if matches!(u.scheme(), "mailto" | "tel" | "sms") {
        // Hand to the OS, but only for a click: a page must not open the mail app by itself.
        let user = shared(app).app.lock().unwrap().gestures.get(&tab_id).is_some_and(|t| t.elapsed() < GESTURE_WINDOW);
        if user {
            open_external(u.as_str());
        }
        return false;
    }
    if !matches!(u.scheme(), "http" | "https") || internal_view(u.as_str()).is_some() {
        return true;
    }
    let host = u.host_str().unwrap_or_default().to_string();
    let s = shared(app);
    let decision = {
        let mut g = s.app.lock().unwrap();
        let user = g
            .gestures
            .remove(&tab_id)
            .map(|t| t.elapsed() < GESTURE_WINDOW)
            .unwrap_or(false);
        let Some(t) = g.tabs.iter().find(|t| t.id == tab_id && t.gen == gen) else { return true };
        let (profile, private) = (t.profile.clone(), t.private);
        if cfg!(debug_assertions) {
            eprintln!("[nav] tab={tab_id} user={user} {u}");
        }
        g.late.remove(&tab_id);
        if !user && !private {
            if let Decision::Swap(p) = nav::decide(&g.cfg.mappings, &profile, &host, true) {
                g.late.insert(tab_id, (p, u.to_string(), Instant::now()));
            }
        }
        let mut d = if private { Decision::Allow } else { nav::decide(&g.cfg.mappings, &profile, &host, user) };
        if let Decision::Prompt(h) = &d {
            if g.skipped.contains(h) || g.prompt.is_some() {
                d = Decision::Allow;
            }
        }
        d
    };
    match decision {
        Decision::Allow => {
            crate::adblock::on_navigate(app, &label(tab_id, gen), &host);
            true
        }
        Decision::Prompt(host) => {
            crate::adblock::on_navigate(app, &label(tab_id, gen), &host);
            s.app.lock().unwrap().prompt = Some(host);
            emit_state(app);
            true
        }
        Decision::Swap(profile) => {
            let app = app.clone();
            let url = u.to_string();
            tauri::async_runtime::spawn(async move {
                swap(&app, tab_id, &profile, &url, true);
            });
            false
        }
    }
}

/// Camera, microphone, location and notification requests. The handler must answer at once, so an
/// unknown site is denied and a card in the sidebar asks; allowing reloads the page to ask again.
fn on_permission(app: &AppHandle, tab_id: u32, host: Option<String>, kind: PermissionKind) -> PermissionResponse {
    let name = match kind {
        PermissionKind::Camera => "camera",
        PermissionKind::Microphone => "microphone",
        PermissionKind::Geolocation => "location",
        PermissionKind::Notifications => "notifications",
        _ => return PermissionResponse::Default,
    };
    let Some(host) = host else { return PermissionResponse::Deny };
    let mut g = shared(app).app.lock().unwrap();
    let saved = g.cfg.permissions.iter().chain(&g.perm_tmp).find(|r| r.host == host && r.kind == name).map(|r| r.allow);
    match saved {
        Some(true) => PermissionResponse::Allow,
        Some(false) => PermissionResponse::Deny,
        None => {
            g.perm_prompt = Some((host, name.into(), tab_id));
            drop(g);
            emit_state(app);
            PermissionResponse::Deny
        }
    }
}

/// The sidebar card's answer. Private tabs keep it for the session only.
pub fn answer_permission(app: &AppHandle, allow: bool) {
    let (rule, tab, private) = {
        let mut g = shared(app).app.lock().unwrap();
        let Some((host, kind, tab)) = g.perm_prompt.take() else { return };
        let private = g.tabs.iter().find(|t| t.id == tab).is_some_and(|t| t.private);
        (config::PermRule { host, kind, allow }, tab, private)
    };
    {
        let s = shared(app);
        let mut g = s.app.lock().unwrap();
        if private {
            g.perm_tmp.push(rule);
        } else {
            g.cfg.permissions.push(rule);
            g.cfg.save(&s.cfg_path);
        }
    }
    if allow {
        reload(app, tab);
    }
    emit_state(app);
}

fn on_page_load(app: &AppHandle, tab_id: u32, gen: u32, url: String, started: bool) {
    let url = normalize(url);
    let s = shared(app);
    let visit = {
        let mut g = s.app.lock().unwrap();
        let Some(t) = g.tab(tab_id) else { return };
        if t.gen != gen {
            return;
        }
        t.loading = started;
        if started {
            t.title.clear();
            None
        } else {
            record_entry(t, &url);
            (!t.private && !is_blank(&url) && internal_view(&url).is_none()).then(|| (url, t.title.clone(), t.profile.clone()))
        }
    };
    if let Some((url, title, profile)) = visit {
        s.db.add_visit(&url, &title, &profile);
    }
    emit_state(app);
}

fn record_entry(t: &mut Tab, url: &str) {
    match t.mode {
        Mode::Traverse => t.mode = Mode::Push,
        Mode::Push => {
            if t.entries.get(t.idx).map(|e| e.url.as_str()) == Some(url) {
                return;
            }
            t.entries.truncate(t.idx + 1);
            t.entries.push(Entry { url: url.into(), profile: t.profile.clone() });
            t.idx = t.entries.len() - 1;
        }
    }
}

/// Called from the `page_ping` command with the calling webview's label.
pub fn page_ping(app: &AppHandle, webview_label: &str, kind: &str) {
    let Some((id, gen)) = parse_label(webview_label) else { return };
    let s = shared(app);
    match kind {
        "g" => {
            let late = {
                let mut g = s.app.lock().unwrap();
                g.gestures.insert(id, Instant::now());
                g.late.remove(&id).filter(|(_, _, t)| t.elapsed() < LATE_WINDOW)
            };
            if let Some((profile, url, _)) = late {
                s.app.lock().unwrap().gestures.remove(&id);
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    swap(&app, id, &profile, &url, true);
                });
            }
        }
        "p1" | "p0" => {
            let mut g = s.app.lock().unwrap();
            if let Some(t) = g.tab(id) {
                t.audible = kind == "p1";
            }
        }
        "u" => {
            // SPA navigation: trust the engine's URL, not the page.
            let Some(url) = app.get_webview(webview_label).and_then(|w| w.url().ok()) else { return };
            let mut g = s.app.lock().unwrap();
            if let Some(t) = g.tab(id) {
                if t.gen == gen {
                    record_entry(t, url.as_str());
                }
            }
            drop(g);
            emit_state(app);
        }
        _ => {}
    }
}

pub fn parse_label(l: &str) -> Option<(u32, u32)> {
    let mut it = l.strip_prefix("tab-")?.split('-');
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

// ---------- tab operations ----------

fn hide_all_except(app: &AppHandle, win: &str, keep: Option<(u32, u32)>) {
    let labels: Vec<String> = {
        let g = shared(app).app.lock().unwrap();
        g.tabs.iter().filter(|t| t.win == win).map(|t| label(t.id, t.gen)).collect()
    };
    for l in labels {
        if let Some(wv) = app.get_webview(&l) {
            if keep.map(|(i, g)| label(i, g)) == Some(l.clone()) {
                let _ = wv.show();
            } else {
                let _ = wv.hide();
            }
        }
    }
}

/// New tab in the window that last had focus.
pub fn open_tab(app: &AppHandle, url: Option<Url>, profile: Option<String>, activate_it: bool) -> u32 {
    let win = shared(app).app.lock().unwrap().focused.clone();
    open_tab_in(app, &win, url, profile, activate_it)
}

pub fn open_tab_in(app: &AppHandle, win: &str, url: Option<Url>, profile: Option<String>, activate_it: bool) -> u32 {
    let id = {
        let mut g = shared(app).app.lock().unwrap();
        g.win(win);
        // No explicit profile: a mapped site opens in its profile, anything else in the default one.
        let mapped = url
            .as_ref()
            .and_then(|u| u.host_str())
            .and_then(|h| nav::lookup(&g.cfg.mappings, h))
            .map(|m| m.profile.clone());
        let profile = profile
            .or(mapped)
            .filter(|p| g.cfg.profile(p).is_some())
            .unwrap_or_else(|| g.cfg.profiles[0].name.clone());
        let id = g.next_id;
        g.next_id += 1;
        let private = g.wins.get(win).is_some_and(|w| w.incognito);
        let entries = url
            .map(|u| vec![Entry { url: u.to_string(), profile: profile.clone() }])
            .unwrap_or_default();
        g.tabs.push(Tab {
            id,
            win: win.to_string(),
            profile,
            entries,
            idx: 0,
            title: String::new(),
            pinned: false,
            group: None,
            discarded: true,
            loading: false,
            audible: false,
            private,
            zoom: None,
            gen: 0,
            last_active: Instant::now(),
            mode: Mode::Push,
        });
        id
    };
    if activate_it {
        activate(app, id);
    } else {
        emit_state(app);
    }
    id
}

pub fn activate(app: &AppHandle, id: u32) {
    let (need_build, gen, win) = {
        let mut g = shared(app).app.lock().unwrap();
        let Some(win) = g.tabs.iter().find(|t| t.id == id).map(|t| t.win.clone()) else { return };
        g.win(&win).active = id;
        let t = g.tab(id).unwrap();
        t.last_active = Instant::now();
        let nb = t.discarded;
        t.discarded = false;
        if nb {
            t.loading = true;
        }
        (nb, t.gen, win)
    };
    hide_all_except(app, &win, Some((id, gen)));
    if need_build {
        // The recreated tab starts at its current history entry.
        if let Err(e) = build_webview(app, id, true) {
            eprintln!("build_webview: {e}");
        }
    } else {
        apply_layout(app);
    }
    emit_state(app);
}

pub fn close_tab(app: &AppHandle, id: u32) {
    let (lbl, next, win, left) = {
        let mut g = shared(app).app.lock().unwrap();
        let Some(pos) = g.tabs.iter().position(|t| t.id == id) else { return };
        let t = g.tabs.remove(pos);
        g.gestures.remove(&id);
        let url = t.url();
        if !t.private && !is_blank(&url) && internal_view(&url).is_none() {
            g.closed.push((url, t.profile.clone()));
            if g.closed.len() > 20 {
                g.closed.remove(0);
            }
        }
        let same: Vec<u32> = g.tabs.iter().filter(|x| x.win == t.win).map(|x| x.id).collect();
        let next = if g.active(&t.win) == id {
            // the neighbour that slid into the closed tab's place, within this window
            let before = g.tabs.iter().take(pos).filter(|x| x.win == t.win).count();
            same.get(before.min(same.len().saturating_sub(1))).copied()
        } else {
            None
        };
        (label(t.id, t.gen), next, t.win.clone(), same.len())
    };
    if let Some(wv) = app.get_webview(&lbl) {
        let _ = wv.close();
    }
    match next {
        Some(n) => activate(app, n),
        None if left == 0 => {
            // Last tab of a secondary window closes the window; the only window gets a blank tab.
            let others = shared(app).app.lock().unwrap().wins.len() > 1;
            if others && win != "main" {
                if let Some(w) = app.get_window(&win) {
                    let _ = w.close();
                }
            } else {
                open_tab_in(app, &win, None, None, true);
            }
        }
        None => emit_state(app),
    }
}

/// Cmd/Ctrl+Shift+T: reopen the most recently closed tab in its profile.
pub fn reopen_closed(app: &AppHandle) {
    let last = shared(app).app.lock().unwrap().closed.pop();
    if let Some((url, profile)) = last {
        open_tab(app, Url::parse(&url).ok(), Some(profile), true);
    }
}

/// Replace the tab's webview with one in `profile`, loading `url`.
pub fn swap(app: &AppHandle, tab_id: u32, profile: &str, url: &str, announce: bool) {
    let (old_label, is_active) = {
        let mut g = shared(app).app.lock().unwrap();
        let active = g.active(&g.win_of(tab_id));
        let Some(t) = g.tab(tab_id) else { return };
        if t.private {
            return;
        }
        let previous = Undo { tab: tab_id, profile: t.profile.clone(), url: t.url() };
        let old = label(t.id, t.gen);
        t.gen += 1;
        t.profile = profile.to_string();
        t.title.clear();
        t.loading = true;
        t.discarded = false;
        if t.mode == Mode::Traverse {
            // History entry already exists; keep idx and mode.
        } else {
            t.entries.truncate(t.idx + 1);
            t.entries.push(Entry { url: url.into(), profile: profile.into() });
            t.idx = t.entries.len() - 1;
            t.mode = Mode::Traverse; // the load below must not push a second entry
        }
        if announce {
            g.undo = Some(previous);
            g.toast = Some(Toast { message: format!("Switched to profile {profile}"), undo: true });
        }
        (old, active == tab_id)
    };
    if let Some(wv) = app.get_webview(&old_label) {
        let _ = wv.close();
    }
    if let Err(e) = build_webview(app, tab_id, is_active) {
        eprintln!("swap build_webview: {e}");
    }
    emit_state(app);
}

pub fn undo_swap(app: &AppHandle) {
    let u = {
        let mut g = shared(app).app.lock().unwrap();
        g.toast = None;
        g.undo.take()
    };
    if let Some(u) = u {
        if let Some(t) = shared(app).app.lock().unwrap().tab(u.tab) {
            t.mode = Mode::Push;
        }
        swap(app, u.tab, &u.profile, &u.url, false);
    }
    emit_state(app);
}

pub fn dismiss_toast(app: &AppHandle) {
    {
        let mut g = shared(app).app.lock().unwrap();
        g.toast = None;
        g.undo = None;
    }
    emit_state(app);
}

/// Load a URL in a tab, honoring profile mappings. Address-bar loads are user-initiated.
pub fn navigate(app: &AppHandle, tab_id: u32, url: Url, mode_push: bool) {
    let s = shared(app);
    let (decision, lbl, was_discarded) = {
        let mut g = s.app.lock().unwrap();
        let host = url.host_str().unwrap_or_default().to_string();
        let Some(t) = g.tabs.iter().find(|t| t.id == tab_id) else { return };
        let (profile, lbl, disc) = (t.profile.clone(), label(t.id, t.gen), t.discarded);
        let mut d = nav::decide(&g.cfg.mappings, &profile, &host, mode_push);
        if let Decision::Prompt(h) = &d {
            if g.skipped.contains(h) || g.prompt.is_some() {
                d = Decision::Allow;
            }
        }
        if let Some(t) = g.tab(tab_id) {
            t.mode = if mode_push { Mode::Push } else { Mode::Traverse };
        }
        (d, lbl, disc)
    };
    match decision {
        Decision::Swap(p) => swap(app, tab_id, &p, url.as_str(), true),
        other => {
            if let Decision::Prompt(h) = other {
                s.app.lock().unwrap().prompt = Some(h);
            }
            if was_discarded {
                // No webview: seed the history entry, then recreate on activation.
                {
                    let mut g = s.app.lock().unwrap();
                    if let Some(t) = g.tab(tab_id) {
                        if mode_push {
                            t.entries.truncate(t.idx + 1);
                            t.entries.push(Entry { url: url.to_string(), profile: t.profile.clone() });
                            t.idx = t.entries.len() - 1;
                            t.mode = Mode::Traverse;
                        }
                    }
                }
                activate(app, tab_id);
            } else if let Some(wv) = app.get_webview(&lbl) {
                let _ = wv.navigate(url);
            }
            emit_state(app);
        }
    }
}

pub fn go_history(app: &AppHandle, tab_id: u32, delta: i32) {
    let target = {
        let mut g = shared(app).app.lock().unwrap();
        let Some(t) = g.tab(tab_id) else { return };
        let new = t.idx as i32 + delta;
        if new < 0 || new as usize >= t.entries.len() {
            return;
        }
        t.idx = new as usize;
        t.mode = Mode::Traverse;
        t.entries[t.idx].clone()
    };
    let Ok(url) = Url::parse(&target.url) else { return };
    let cur_profile = shared(app).app.lock().unwrap().tab(tab_id).map(|t| t.profile.clone());
    if cur_profile.as_deref() != Some(target.profile.as_str()) {
        swap(app, tab_id, &target.profile, &target.url, false);
    } else {
        navigate(app, tab_id, url, false);
    }
}

/// Links handed to the app from outside (default browser, second launch): http(s) only, each in a new tab.
pub fn open_urls<S: AsRef<str>>(app: &AppHandle, urls: impl IntoIterator<Item = S>) {
    let urls: Vec<Url> = urls.into_iter().filter_map(|u| Url::parse(u.as_ref()).ok()).filter(|u| matches!(u.scheme(), "http" | "https")).collect();
    if urls.is_empty() {
        return;
    }
    for u in urls {
        open_tab(app, Some(u), None, true);
    }
    let win = shared(app).app.lock().unwrap().focused.clone();
    if let Some(w) = app.get_window(&win) {
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Opens `url` with the OS handler. Callers pass only allow-listed schemes.
pub fn open_external(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

/// Cmd/Ctrl +, -, 0: `step` is +1, -1, or 0 to reset. Remembered per tab so a rebuilt webview keeps it.
pub fn zoom(app: &AppHandle, tab_id: u32, step: i32) {
    let pct = {
        let mut g = shared(app).app.lock().unwrap();
        let base = g.cfg.settings.page_zoom;
        let Some(t) = g.tab(tab_id) else { return };
        let pct = if step == 0 { base } else { (t.zoom.unwrap_or(base) as i32 + step * 10).clamp(30, 300) as u32 };
        t.zoom = (step != 0).then_some(pct);
        pct
    };
    if let Some(wv) = active_webview(app, tab_id) {
        let _ = wv.set_zoom(pct as f64 / 100.0);
    }
}

/// File > Print / Save Page: run in the page, since only macOS has a native print call.
pub fn page_action(app: &AppHandle, tab_id: u32, act: &str) {
    let Some(wv) = active_webview(app, tab_id) else { return };
    let _ = match act {
        "print" => wv.eval("window.print()"),
        _ => wv.eval("(()=>{const a=document.createElement('a');a.href=location.href;a.download=(document.title||'page')+'.html';document.body.appendChild(a);a.click();a.remove()})()"),
    };
}

pub fn reload(app: &AppHandle, tab_id: u32) {
    reload_with(app, tab_id, "");
}

/// `pre` runs first in the page; it survives the reload only via storage.
pub fn reload_with(app: &AppHandle, tab_id: u32, pre: &str) {
    let lbl = {
        let mut g = shared(app).app.lock().unwrap();
        let Some(t) = g.tab(tab_id) else { return };
        t.mode = Mode::Traverse;
        label(t.id, t.gen)
    };
    if let Some(wv) = app.get_webview(&lbl) {
        let _ = wv.eval(format!("{pre}location.reload()"));
    }
}

pub fn address_bar(app: &AppHandle, tab_id: u32, input: &str) {
    let (template, internal, win) = {
        let g = shared(app).app.lock().unwrap();
        (g.cfg.settings.search_template.clone(), g.tabs.iter().find(|t| t.id == tab_id).is_some_and(|t| internal_view(&t.url()).is_some()), g.win_of(tab_id))
    };
    if let Some(view) = internal_view(input.trim()).filter(|v| matches!(v.as_str(), "settings" | "history" | "downloads")) {
        return open_internal(app, &win, &view);
    }
    // easter egg: ask the door nicely
    if input.trim().eq_ignore_ascii_case("open sesame") {
        shared(app).app.lock().unwrap().toast = Some(Toast { message: "Aditus: the door was never locked.".into(), undo: false });
        return emit(app, false);
    }
    let url = search::resolve(input, &template);
    if internal {
        open_tab_in(app, &win, Some(url), None, true);
    } else {
        navigate(app, tab_id, url, true);
    }
}

pub fn assign_mapping(app: &AppHandle, host: &str, profile: &str) {
    // Mappings match on the host only: drop scheme, path, query and port if a full URL was typed.
    let host = nav::clean_pattern(host);
    let host = host.as_str();
    if host.is_empty() {
        return;
    }
    let to_swap = {
        let mut g = shared(app).app.lock().unwrap();
        g.prompt = None;
        g.cfg.mappings.retain(|m| m.pattern != host);
        g.cfg.mappings.push(nav::Mapping { pattern: host.into(), profile: profile.into() });
        let active = g.focused_active();
        g.tab(active)
            .filter(|t| t.profile != profile && Url::parse(&t.url()).ok().and_then(|u| u.host_str().map(|h| nav::matches(host, h))).unwrap_or(false))
            .map(|t| (t.id, t.url()))
    };
    save_cfg(app);
    if let Some((id, url)) = to_swap {
        swap(app, id, profile, &url, true);
    } else {
        emit_state(app);
    }
}

pub fn skip_prompt(app: &AppHandle) {
    {
        let mut g = shared(app).app.lock().unwrap();
        if let Some(h) = g.prompt.take() {
            g.skipped.insert(h);
        }
    }
    emit_state(app);
}

/// Destroy webviews of tabs idle past the configured timeout.
pub fn discard_idle(app: &AppHandle) {
    let victims: Vec<String> = {
        let mut g = shared(app).app.lock().unwrap();
        let limit = Duration::from_secs(g.cfg.settings.idle_minutes * 60);
        let actives: Vec<u32> = g.wins.values().map(|w| w.active).collect();
        g.tabs
            .iter_mut()
            .filter(|t| !actives.contains(&t.id) && !t.pinned && !t.audible && !t.discarded && t.last_active.elapsed() > limit)
            .map(|t| {
                t.discarded = true;
                t.loading = false;
                label(t.id, t.gen)
            })
            .collect()
    };
    for l in &victims {
        if let Some(wv) = app.get_webview(l) {
            let _ = wv.close();
        }
    }
    if !victims.is_empty() {
        emit_state(app);
    }
}

pub fn with_tab(app: &AppHandle, id: u32, f: impl FnOnce(&mut Tab)) {
    if let Some(t) = shared(app).app.lock().unwrap().tab(id) {
        f(t);
    }
    emit_state(app);
}

pub fn reorder(app: &AppHandle, id: u32, to: usize) {
    {
        let mut g = shared(app).app.lock().unwrap();
        let Some(from) = g.tabs.iter().position(|t| t.id == id) else { return };
        let win = g.tabs[from].win.clone();
        let t = g.tabs.remove(from);
        // `to` indexes the window's own tab list; translate it to a position in the global list
        let ids: Vec<u32> = g.tabs.iter().filter(|x| x.win == win).map(|x| x.id).collect();
        let to = match ids.get(to) {
            Some(&tid) => g.tabs.iter().position(|x| x.id == tid).unwrap_or(g.tabs.len()),
            None => ids.last().and_then(|&tid| g.tabs.iter().position(|x| x.id == tid)).map_or(g.tabs.len(), |p| p + 1),
        };
        g.tabs.insert(to, t);
    }
    emit_state(app);
}

pub fn cycle(app: &AppHandle, delta: i32) {
    let next = {
        let g = shared(app).app.lock().unwrap();
        let ids: Vec<u32> = g.tabs.iter().filter(|t| t.win == g.focused).map(|t| t.id).collect();
        let Some(pos) = ids.iter().position(|&i| i == g.focused_active()) else { return };
        ids[((pos as i32 + delta).rem_euclid(ids.len() as i32)) as usize]
    };
    activate(app, next);
}

pub fn active_webview(app: &AppHandle, id: u32) -> Option<tauri::Webview> {
    let l = {
        let g = shared(app).app.lock().unwrap();
        let t = g.tabs.iter().find(|t| t.id == id)?;
        label(t.id, t.gen)
    };
    app.get_webview(&l)
}

// ---------- windows ----------

/// Window + its UI webview. Window state is created lazily by `Shared::win`.
pub fn create_window(app: &AppHandle, label: &str) -> tauri::Result<()> {
    let wb = tauri::window::WindowBuilder::new(app, label)
        .title("Aditus")
        .inner_size(1280.0, 820.0)
        .min_inner_size(640.0, 400.0);
    // The native title bar would swallow clicks on the top ~28px of the top bar; the UI
    // draws its own drag regions instead.
    #[cfg(target_os = "macos")]
    let wb = wb.title_bar_style(tauri::TitleBarStyle::Overlay).hidden_title(true);
    let window = wb.build()?;
    let sf = window.scale_factor()?;
    let size = window.inner_size()?.to_logical::<f64>(sf);
    let ui = window.add_child(
        WebviewBuilder::new(wv_label("ui", label), WebviewUrl::App("index.html".into())).auto_resize(),
        LogicalPosition::new(0.0, 0.0),
        size,
    )?;
    no_inset(&ui);
    shared(app).app.lock().unwrap().win(label);
    Ok(())
}

/// New window with one blank tab; it takes focus.
pub fn new_window(app: &AppHandle, incognito: bool) {
    let label = {
        let mut g = shared(app).app.lock().unwrap();
        let n = g.next_win;
        g.next_win += 1;
        format!("w{n}")
    };
    if let Err(e) = create_window(app, &label) {
        return eprintln!("new window: {e}");
    }
    {
        let mut g = shared(app).app.lock().unwrap();
        g.focused = label.clone();
        g.win(&label).incognito = incognito;
    }
    if incognito {
        if let Some(w) = app.get_window(&label) {
            let _ = w.set_title("Aditus (Private)");
        }
    }
    open_tab_in(app, &label, None, None, true);
}

/// A window went away: drop its tabs (remembered for "reopen closed tab") and its state.
pub fn window_closed(app: &AppHandle, label: &str) {
    let victims: Vec<String> = {
        let mut g = shared(app).app.lock().unwrap();
        if g.wins.len() <= 1 {
            return; // last window: the app is quitting, keep the saved session intact
        }
        let (gone, kept): (Vec<Tab>, Vec<Tab>) = std::mem::take(&mut g.tabs).into_iter().partition(|t| t.win == label);
        g.tabs = kept;
        g.wins.remove(label);
        if g.focused == label {
            g.focused = g.wins.keys().next().cloned().unwrap_or_else(|| "main".into());
        }
        for t in &gone {
            let url = t.url();
            if !t.private && !is_blank(&url) && internal_view(&url).is_none() {
                g.closed.push((url, t.profile.clone()));
            }
        }
        gone.iter().map(|t| label_of(t)).collect()
    };
    for l in victims {
        if let Some(wv) = app.get_webview(&l) {
            let _ = wv.close();
        }
    }
    emit(app, false);
}

fn label_of(t: &Tab) -> String {
    label(t.id, t.gen)
}

// ---------- session ----------

fn main_win() -> String {
    "main".into()
}

#[derive(Serialize, serde::Deserialize)]
struct Saved {
    url: String,
    profile: String,
    pinned: bool,
    group: Option<String>,
    title: String,
    #[serde(default = "main_win")]
    win: String,
    /// Selected tab of its window.
    #[serde(default)]
    active: bool,
}

#[derive(Serialize, serde::Deserialize, Default)]
struct Session {
    tabs: Vec<Saved>,
}

fn session_path(app: &AppHandle) -> PathBuf {
    shared(app).data_dir.join("session.json")
}

pub fn save_session(app: &AppHandle) {
    let session = {
        let g = shared(app).app.lock().unwrap();
        Session {
            tabs: g
                .tabs
                .iter()
                .filter(|t| !t.private)
                .map(|t| Saved {
                    url: t.url(),
                    profile: t.profile.clone(),
                    pinned: t.pinned,
                    group: t.group.clone(),
                    title: t.title.clone(),
                    win: t.win.clone(),
                    active: g.active(&t.win) == t.id,
                })
                .collect(),
        }
    };
    if let Ok(s) = serde_json::to_string(&session) {
        let _ = std::fs::write(session_path(app), s);
    }
}

pub fn restore_session(app: &AppHandle) {
    let session: Session = std::fs::read_to_string(session_path(app))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let (startup, home) = {
        let g = shared(app).app.lock().unwrap();
        (g.cfg.settings.startup.clone(), g.cfg.settings.home_url.clone())
    };
    if startup != "restore" || session.tabs.is_empty() {
        let url = if startup == "home" { Url::parse(&home).ok() } else { None };
        open_tab_in(app, "main", url, None, true);
        return;
    }
    let mut to_activate: HashMap<String, u32> = HashMap::new();
    let mut first_of: HashMap<String, u32> = HashMap::new();
    for s in session.tabs {
        if s.win != "main" && app.get_window(&s.win).is_none() {
            if let Some(n) = s.win.strip_prefix('w').and_then(|n| n.parse::<u32>().ok()) {
                let mut g = shared(app).app.lock().unwrap();
                g.next_win = g.next_win.max(n + 1);
            }
            if create_window(app, &s.win).is_err() {
                continue;
            }
        }
        let id = open_tab_in(app, &s.win, Url::parse(&s.url).ok(), Some(s.profile), false);
        with_tab(app, id, |t| {
            t.pinned = s.pinned;
            t.group = s.group;
            t.title = s.title;
        });
        first_of.entry(s.win.clone()).or_insert(id);
        if s.active {
            to_activate.insert(s.win, id);
        }
    }
    for (win, first) in first_of {
        activate(app, to_activate.get(&win).copied().unwrap_or(first));
    }
}
