//! Tab model and view lifecycle.
//!
//! Ownership: `App` (behind `Shared.app`) owns all serializable state: tabs, windows, config,
//! history entries, prompts. `engine::REG` owns the live CEF handles (views, windows, contexts),
//! UI thread only, addressed by the labels built here (`label`, `wv_label`). Neither side caches
//! the other's facts: the engine only knows labels, and `engine::has_view` answers "is it alive".
//!
//! Everything runs on the CEF UI thread, so the state lock is only ever contended by the
//! ad-block and IPC worker threads. Still, never hold `Shared.app` across an `engine` call: CEF
//! calls back into our handlers synchronously, and those take the same lock. Lock only through
//! `Shared::lock_app`; every `engine` entry point reaches `reg`, which calls `assert_unlocked` and
//! panics in debug builds if the calling thread still holds a guard.

use crate::{
    config::{self, Config},
    db::Db,
    engine::{self, Bounds, Kind, Spec},
    history::{Entry, Hist, Step},
    nav::{self, Decision},
    search,
};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use url::Url;

const GESTURE_WINDOW: Duration = Duration::from_millis(2500);
pub const NEWTAB: &str = "newtab.html";

/// Zero-sized handle kept so the state machine reads the same as with a runtime object.
#[derive(Clone, Copy)]
pub struct AppHandle;

pub struct Shared {
    pub app: Mutex<App>,
    pub db: Db,
    pub cfg_path: PathBuf,
    pub data_dir: PathBuf,
}

impl Shared {
    /// The only way to lock `app`. In debug builds the guard counts itself per thread so that
    /// `assert_unlocked` (called by every engine entry point) panics if an engine call is made while
    /// a guard is alive; release builds compile the counting away.
    pub fn lock_app(&self) -> AppGuard<'_> {
        let g = self.app.lock().unwrap();
        #[cfg(debug_assertions)]
        HELD.with(|h| h.set(h.get() + 1));
        AppGuard(g)
    }
}

#[cfg(debug_assertions)]
thread_local! {
    static HELD: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Panics (debug builds only) when this thread holds an `App` guard. Engine calls re-enter our handlers.
#[track_caller]
pub fn assert_unlocked() {
    #[cfg(debug_assertions)]
    HELD.with(|h| assert!(h.get() == 0, "engine call while the App lock is held"));
}

pub struct AppGuard<'a>(std::sync::MutexGuard<'a, App>);

impl std::ops::Deref for AppGuard<'_> {
    type Target = App;
    fn deref(&self) -> &App {
        &self.0
    }
}

impl std::ops::DerefMut for AppGuard<'_> {
    fn deref_mut(&mut self) -> &mut App {
        &mut self.0
    }
}

#[cfg(debug_assertions)]
impl Drop for AppGuard<'_> {
    fn drop(&mut self) {
        HELD.with(|h| h.set(h.get() - 1));
    }
}

static SHARED: OnceLock<Shared> = OnceLock::new();

pub fn init_shared(s: Shared) {
    let _ = SHARED.set(s);
}

pub fn shared(_: &AppHandle) -> &'static Shared {
    SHARED.get().expect("state not initialised")
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
    pub hist: Hist,
    pub title: String,
    pub pinned: bool,
    pub group: Option<String>,
    pub discarded: bool,
    pub loading: bool,
    pub audible: bool,
    /// Frames currently playing media; `audible` is "any of them" (an iframe pausing must not silence the page).
    pub playing: HashSet<String>,
    /// The user silenced this tab (kept across sleep).
    pub muted: bool,
    /// Private-window tab: no history, no saved session, no profile swaps, throwaway storage.
    pub private: bool,
    /// Page zoom in percent set with Cmd/Ctrl +/-/0; `None` follows the setting.
    pub zoom: Option<u32>,
    pub gen: u32,
    pub last_active: Instant,
    /// Scroll, form and media state the page last reported, with the URL it belongs to. Memory only.
    snap: Option<(String, String)>,
    /// Put `snap` back after the next load (the tab was just woken from sleep).
    restore: bool,
    /// OS process id of the tab's renderer (reported by the renderer itself) and its share of that process's memory.
    pid: Option<u32>,
    mem_mb: u32,
    mode: Mode,
    /// The view was just created and has not committed a page yet: a different address is a redirect.
    fresh: bool,
    /// The engine reported the load in flight as a history traversal.
    fwd_back: bool,
    /// When a native go_back/go_forward was issued: the next commit within a few seconds belongs to it.
    /// (Not `Mode::Traverse`: a bfcache restore commits nothing, and that mode would then swallow a later push.)
    native_at: Option<Instant>,
    /// Label of the view this tab showed before a profile swap; kept on screen until the new view commits.
    retiring: Option<String>,
    /// Link inside the page's text selection, reported by the page script: Cmd+T / Cmd+Shift+N open it.
    link: Option<Url>,
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
    /// Text selected in the focused address bar; beats a link selected in the page.
    pub address_sel: String,
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
    perm_prompt: Vec<(String, String, u32)>,
    /// Answers given in private windows; never written to disk.
    perm_tmp: Vec<config::PermRule>,
    /// Pending HTTP-auth / certificate prompts, oldest first; the engine holds their callbacks by id.
    net_q: Vec<NetPrompt>,
    next_net: u64,
    /// Hosts the user chose to proceed to despite a certificate error: (host, private). Session only.
    cert_ok: HashSet<(String, bool)>,
    skipped: HashSet<String>,
    toast: Option<Toast>,
    undo: Option<Undo>,
    /// Last user gesture per tab (click, Enter, submit) and the link it landed on, reported by the page script.
    gestures: HashMap<u32, (Instant, String)>,
    /// Last moment a download was legitimate per tab: a user gesture or one of our own UI actions.
    dl_ok: HashMap<u32, Instant>,
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
            perm_prompt: vec![],
            perm_tmp: vec![],
            net_q: vec![],
            next_net: 1,
            cert_ok: HashSet::new(),
            skipped: HashSet::new(),
            toast: None,
            undo: None,
            gestures: HashMap::new(),
            dl_ok: HashMap::new(),
            closed: vec![],
        }
    }

    #[cfg(debug_assertions)]
    pub fn prompt_host(&self) -> Option<String> {
        self.prompt.clone()
    }

    /// The tab's profile id (what logins are filed under) and whether it is the first profile (owner of legacy keychain entries).
    fn profile_id(&self, id: u32) -> Option<(String, bool)> {
        let t = self.tabs.iter().find(|t| t.id == id)?;
        let first = self.cfg.profiles.first()?;
        let p = self.cfg.profile(&t.profile).unwrap_or(first);
        Some((p.id.clone(), p.id == first.id))
    }

    fn tab(&mut self, id: u32) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    pub fn win(&mut self, w: &str) -> &mut Win {
        self.wins.entry(w.to_string()).or_insert(Win { active: 0, sidebar: 250.0, top: 52.0, overlay_open: false, incognito: false, address_sel: String::new() })
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

/// Per-window view label: the first window keeps the bare names.
pub fn wv_label(kind: &str, win: &str) -> String {
    if win == "main" {
        kind.to_string()
    } else {
        format!("{kind}-{win}")
    }
}

impl Tab {
    pub fn url(&self) -> String {
        self.hist.url()
    }
}

pub fn label(id: u32, gen: u32) -> String {
    format!("tab-{id}-{gen}")
}

pub fn parse_label(l: &str) -> Option<(u32, u32)> {
    let mut it = l.strip_prefix("tab-")?.split('-');
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

/// Browser pages (settings, history, downloads) live in ordinary tabs as `aditus://<view>`; the
/// view itself is served as `aditus://app/index.html?view=<view>`.
pub fn internal_view(url: &str) -> Option<String> {
    let u = Url::parse(url).ok()?;
    if u.scheme() != "aditus" {
        return None;
    }
    match u.host_str()? {
        "app" if matches!(u.path(), "/" | "/index.html") => u.query_pairs().find(|(k, _)| k == "view").map(|(_, v)| v.into_owned()),
        "app" => None,
        other => Some(other.to_string()),
    }
}

fn normalize(url: String) -> String {
    internal_view(&url).map(|v| format!("aditus://{v}")).unwrap_or(url)
}

pub fn open_internal(app: &AppHandle, win: &str, view: &str) {
    let url = format!("aditus://{view}");
    let existing = shared(app).lock_app().tabs.iter().find(|t| t.win == win && t.url() == url).map(|t| t.id);
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
    audible: bool,
    muted: bool,
    mem_mb: u32,
    can_back: bool,
    can_forward: bool,
}

/// A held engine request waiting for the user: a login (`cert` false) or a bad certificate.
#[derive(Serialize, Clone)]
pub struct NetPrompt {
    id: u64,
    cert: bool,
    host: String,
    /// Realm for logins, a short reason for certificates.
    detail: String,
    #[serde(skip)]
    tab: u32,
}

pub enum Net {
    Allow,
    Deny,
    Wait(u64),
}

/// Short reason for a certificate error code (`net::ERR_CERT_*`).
pub fn cert_error_text(code: i32) -> &'static str {
    match code {
        -200 => "the certificate is for a different site",
        -201 => "the certificate has expired or is not yet valid",
        -202 => "the certificate is not from a trusted authority",
        -207 => "the certificate is invalid",
        -208 => "the certificate uses a weak algorithm",
        -211 => "the certificate has been revoked",
        _ => "the certificate could not be verified",
    }
}

#[derive(Serialize)]
struct Snapshot<'a> {
    win: &'a str,
    /// Per-window counter: a `state-delta` applies only on top of `seq - 1`, else the UI re-requests.
    seq: u64,
    tabs: Vec<TabView>,
    active: u32,
    prompt: Option<String>,
    perm: Option<(&'a str, &'a str)>,
    net: Option<&'a NetPrompt>,
    toast: Option<Toast>,
    adblock_site: bool,
    bookmarked: bool,
    has_login: bool,
    incognito: bool,
}

/// The per-tab fields that change many times a second; everything else is "structural".
#[derive(Clone, PartialEq, Debug)]
struct Hot {
    id: u32,
    title: String,
    loading: bool,
    mem_mb: u32,
}

/// What was last sent for a window: the structural key (JSON with hot fields blanked, plus the cfg
/// version), the hot fields, and the sequence number.
#[derive(Default)]
struct Sent {
    key: String,
    hot: Vec<Hot>,
    seq: u64,
}

enum Out {
    Skip,
    Full,
    /// Changed tabs only, as `{id, <changed fields>}`.
    Delta(Vec<serde_json::Value>),
}

/// Full snapshot when the structure changed (or nothing was sent yet), a delta when only hot
/// fields moved, nothing when identical.
fn plan(prev: Option<&Sent>, key: &str, hot: &[Hot]) -> Out {
    let Some(p) = prev.filter(|p| p.key == key && p.hot.len() == hot.len() && p.hot.iter().zip(hot).all(|(a, b)| a.id == b.id)) else {
        return Out::Full;
    };
    let changed: Vec<_> = p
        .hot
        .iter()
        .zip(hot)
        .filter(|(a, b)| a != b)
        .map(|(a, b)| {
            let mut m = serde_json::Map::new();
            m.insert("id".into(), b.id.into());
            if a.title != b.title {
                m.insert("title".into(), b.title.clone().into());
            }
            if a.loading != b.loading {
                m.insert("loading".into(), b.loading.into());
            }
            if a.mem_mb != b.mem_mb {
                m.insert("mem_mb".into(), b.mem_mb.into());
            }
            m.into()
        })
        .collect();
    if changed.is_empty() {
        Out::Skip
    } else {
        Out::Delta(changed)
    }
}

/// Splices pre-serialized config into a serialized snapshot object.
fn with_cfg(snap: &str, cfg: &str) -> String {
    format!("{},\"cfg\":{cfg}}}", &snap[..snap.len() - 1])
}

/// Serialized config, re-serialized only when it changed. Lock order: App, then this.
fn cfg_json(cfg: &Config) -> (u64, std::sync::Arc<String>) {
    static CACHE: Mutex<Option<(u64, Config, std::sync::Arc<String>)>> = Mutex::new(None);
    let mut c = CACHE.lock().unwrap();
    match c.as_ref().filter(|(_, old, _)| old == cfg) {
        Some((v, _, j)) => (*v, j.clone()),
        None => {
            let v = c.as_ref().map_or(1, |x| x.0 + 1);
            // The UI shows and sends profile names; mappings are stored by id.
            let mut view = serde_json::to_value(cfg).unwrap_or_default();
            for m in view["mappings"].as_array_mut().into_iter().flatten() {
                m["profile"] = cfg.name_of(m["profile"].as_str().unwrap_or_default()).into();
            }
            let j = std::sync::Arc::new(view.to_string());
            *c = Some((v, cfg.clone(), j.clone()));
            (v, j)
        }
    }
}

/// `(bookmarked, has_login)` for the active page, cached until a bookmark or login is written.
fn db_flags(db: &Db, url: &str, profile: Option<&str>, incognito: bool) -> (bool, bool) {
    static CACHE: Mutex<(u64, Option<HashMap<(String, Option<String>, bool), (bool, bool)>>)> = Mutex::new((0, None));
    let ver = db.flags_ver();
    let key = (url.to_string(), profile.map(String::from), incognito);
    {
        let mut c = CACHE.lock().unwrap();
        if c.0 != ver || c.1.as_ref().is_some_and(|m| m.len() > 256) {
            *c = (ver, None);
        }
        if let Some(v) = c.1.as_ref().and_then(|m| m.get(&key)) {
            return *v;
        }
    }
    let v = (db.is_bookmarked(url), !incognito && crate::passwords::origin_of(url).is_some_and(|o| !db.logins(profile, Some(&o)).is_empty()));
    let mut c = CACHE.lock().unwrap();
    if c.0 == ver {
        c.1.get_or_insert_with(Default::default).insert(key, v);
    }
    v
}

/// Request a state broadcast. Calls within one 16ms window coalesce into a single `flush`.
pub fn emit_state(app: &AppHandle) {
    schedule(app, true);
}

static PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static WANT_SAVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SENT: std::sync::LazyLock<Mutex<HashMap<String, Sent>>> = std::sync::LazyLock::new(Default::default);

fn schedule(_app: &AppHandle, save: bool) {
    use std::sync::atomic::Ordering::SeqCst;
    if save {
        WANT_SAVE.store(true, SeqCst);
    }
    if !PENDING.swap(true, SeqCst) {
        engine::defer_after(16, || {
            PENDING.store(false, SeqCst);
            flush(&AppHandle, WANT_SAVE.swap(false, SeqCst));
        });
    }
}

/// A frontend asked for the state (it just loaded or lost sync): send full snapshots even if unchanged.
pub fn resend_state(app: &AppHandle) {
    SENT.lock().unwrap().values_mut().for_each(|s| s.key.clear());
    emit_state(app);
}

fn emit(app: &AppHandle, save: bool) {
    schedule(app, save);
}

/// Sends each window's state to that window's own views only: a full snapshot when its structure
/// changed, a small `state-delta` when only title/loading/memory moved.
fn flush(app: &AppHandle, save: bool) {
    let s = shared(app);
    // Prompts of tabs that have since closed would never be answered.
    let gone: Vec<u64> = {
        let mut g = s.lock_app();
        let ids: Vec<u32> = g.tabs.iter().map(|t| t.id).collect();
        let gone = g.net_q.iter().filter(|p| !ids.contains(&p.tab)).map(|p| p.id).collect();
        g.net_q.retain(|p| ids.contains(&p.tab));
        gone
    };
    for id in gone {
        engine::net_answer(id, false, "", "");
    }
    // DB lookups happen outside the App lock (and mostly hit the cache).
    let flags: HashMap<String, (bool, bool)> = {
        let want: Vec<(String, String, Option<String>, bool)> = {
            let g = s.lock_app();
            g.wins.iter().map(|(win, w)| (win.clone(), g.tabs.iter().find(|t| t.id == w.active).map(|t| t.url()).unwrap_or_default(), g.profile_id(w.active).map(|p| p.0), w.incognito)).collect()
        };
        want.into_iter().map(|(win, url, pid, inc)| (win, db_flags(&s.db, &url, pid.as_deref(), inc))).collect()
    };
    let out: Vec<(String, String, bool)> = {
        let g = s.lock_app();
        crate::adblock::sync_cfg(g.cfg.settings.adblock, &g.cfg.allowlist);
        let (cfg_ver, cfg) = cfg_json(&g.cfg);
        let mut sent = SENT.lock().unwrap();
        sent.retain(|w, _| g.wins.contains_key(w));
        g.wins
            .iter()
            .filter_map(|(win, w)| {
                let active_host = g.tabs.iter().find(|t| t.id == w.active).and_then(|t| Url::parse(&t.url()).ok()).and_then(|u| u.host_str().map(String::from)).unwrap_or_default();
                let (bookmarked, has_login) = flags.get(win).copied().unwrap_or_default();
                let mut hot = vec![];
                let mut snap = Snapshot {
                    win,
                    seq: 0,
                    tabs: g
                        .tabs
                        .iter()
                        .filter(|t| &t.win == win)
                        .map(|t| {
                            let url = t.url();
                            let parsed = Url::parse(&url).ok();
                            let title = if t.title.is_empty() {
                                if is_blank(&url) {
                                    "New Tab".into()
                                } else {
                                    url.clone()
                                }
                            } else {
                                t.title.clone()
                            };
                            hot.push(Hot { id: t.id, title, loading: t.loading, mem_mb: t.mem_mb });
                            TabView {
                                id: t.id,
                                // hot fields are filled in below, only if a full snapshot is sent
                                title: String::new(),
                                host: if is_blank(&url) || internal_view(&url).is_some() { String::new() } else { parsed.as_ref().and_then(|u| u.host_str().map(String::from)).unwrap_or_default() },
                                url: if is_blank(&url) { String::new() } else { url },
                                profile: g.cfg.name_of(&t.profile),
                                pinned: t.pinned,
                                group: t.group.clone(),
                                discarded: t.discarded,
                                loading: false,
                                audible: t.audible,
                                muted: t.muted,
                                mem_mb: 0,
                                can_back: t.hist.can_back(),
                                can_forward: t.hist.can_forward(),
                            }
                        })
                        .collect(),
                    active: w.active,
                    prompt: g.prompt.clone(),
                    perm: g.perm_prompt.iter().find(|p| g.win_of(p.2) == *win).map(|p| (p.0.as_str(), p.1.as_str())),
                    net: g.net_q.iter().find(|p| g.win_of(p.tab) == *win),
                    toast: g.toast.clone(),
                    adblock_site: !g.cfg.allowlist.contains(&active_host),
                    bookmarked,
                    has_login,
                    incognito: w.incognito,
                };
                let key = format!("{cfg_ver}|{}", serde_json::to_string(&snap).unwrap_or_default());
                let prev = sent.get(win);
                let seq = prev.map_or(0, |p| p.seq) + 1;
                let (json, event) = match plan(prev, &key, &hot) {
                    Out::Skip => return None,
                    Out::Delta(tabs) => (serde_json::json!({ "win": win, "seq": seq, "tabs": tabs }).to_string(), false),
                    Out::Full => {
                        snap.seq = seq;
                        for (v, h) in snap.tabs.iter_mut().zip(&hot) {
                            (v.title, v.loading, v.mem_mb) = (h.title.clone(), h.loading, h.mem_mb);
                        }
                        (with_cfg(&serde_json::to_string(&snap).unwrap_or_default(), &cfg), true)
                    }
                };
                sent.insert(win.clone(), Sent { key, hot, seq });
                Some((win.clone(), json, event))
            })
            .collect()
    };
    for (win, json, full) in out {
        engine::emit_win(&win, if full { "state" } else { "state-delta" }, &json);
    }
    if save {
        save_session(app);
    }
}

fn save_cfg(app: &AppHandle) {
    let s = shared(app);
    let g = s.lock_app();
    g.cfg.save(&s.cfg_path);
}

// ---------- layout ----------

/// The page area of window `win`: right of the sidebar, below the top bar.
pub fn content_rect(app: &AppHandle, win: &str) -> Bounds {
    let (sidebar, top) = {
        let mut g = shared(app).lock_app();
        let w = g.win(win);
        (w.sidebar, w.top)
    };
    let (w, h) = engine::win_size(win);
    (sidebar, top, (w - sidebar).max(1.0), (h - top).max(1.0))
}

pub fn apply_layout(app: &AppHandle) {
    let wins: Vec<(String, Option<String>)> = {
        let g = shared(app).lock_app();
        g.wins.iter().map(|(name, w)| (name.clone(), g.tabs.iter().find(|t| t.id == w.active).map(|t| label(t.id, t.gen)))).collect()
    };
    for (win, tab) in wins {
        let area = content_rect(app, &win);
        let full = engine::win_size(&win);
        if let Some(fs) = engine::fullscreen_label(&win) {
            engine::set_bounds(&fs, (0.0, 0.0, full.0, full.1));
            continue;
        }
        if let Some(l) = tab {
            engine::set_bounds(&l, engine::dock_split(&win, &l, area));
        }
        engine::set_bounds(&wv_label("panel", &win), area);
        engine::place_window_buttons(&win);
    }
}

/// Manual profile change for the active tab (no mapping change).
pub fn switch_profile(app: &AppHandle, tab_id: u32, profile: &str) {
    let Some(profile) = shared(app).lock_app().cfg.profile(profile).map(|p| p.id.clone()) else { return };
    let profile = profile.as_str();
    let url = {
        let mut g = shared(app).lock_app();
        match g.tab(tab_id) {
            Some(t) if t.profile != profile => t.url(),
            _ => return,
        }
    };
    if is_blank(&url) {
        let (old, show) = {
            let mut g = shared(app).lock_app();
            let active = g.active(&g.win_of(tab_id));
            let Some(t) = g.tab(tab_id) else { return };
            let old = label(t.id, t.gen);
            t.gen += 1;
            t.profile = profile.into();
            (old, active == tab_id)
        };
        engine::close(&old);
        build_webview(app, tab_id, show);
        emit_state(app);
    } else {
        swap(app, tab_id, profile, &url, true);
    }
}

// ---------- view creation ----------

fn app_url(query: &str) -> String {
    format!("aditus://app/index.html?{query}")
}

/// Creates the tab's view (asynchronously: the profile's storage may still be starting).
fn build_webview(app: &AppHandle, tab_id: u32, show: bool) {
    let s = shared(app);
    let (gen, url, profile, win, private, zoom, adblock_on) = {
        let mut g = s.lock_app();
        // The new view's native history is empty and its first load is the current entry.
        if let Some(t) = g.tab(tab_id) {
            t.hist.fresh_view();
            t.fresh = true;
            t.fwd_back = false;
            t.native_at = None;
        }
        let Some(t) = g.tabs.iter().find(|t| t.id == tab_id) else { return };
        let Some(p) = g.cfg.profile(&t.profile).or(g.cfg.profiles.first()).cloned() else { return };
        (t.gen, t.url(), p, t.win.clone(), t.private, t.zoom.unwrap_or(g.cfg.settings.page_zoom), g.cfg.settings.adblock)
    };
    let store = if private {
        (format!("private-{win}"), PathBuf::new())
    } else {
        let dir = config::Config::data_dir(&s.data_dir.join("cef"), &profile);
        let _ = std::fs::create_dir_all(&dir);
        (format!("profile-{}", profile.id), dir)
    };
    let (real_url, app_page) = if let Some(view) = internal_view(&url) {
        (app_url(&format!("view={view}&win={win}")), true)
    } else if is_blank(&url) {
        (format!("aditus://app/{NEWTAB}"), true)
    } else {
        (url, false)
    };
    engine::create(Spec {
        label: label(tab_id, gen),
        win: win.clone(),
        url: real_url,
        kind: Kind::Page,
        app_page,
        store: Some(store),
        bounds: content_rect(app, &win),
        visible: show,
        zoom,
        yt: adblock_on,
    });
}

/// The engine finished creating a view: apply per-tab state that needs the browser.
pub fn on_view_ready(app: &AppHandle, label: &str) {
    let Some((id, gen)) = parse_label(label) else { return };
    let (zoom, muted) = {
        let g = shared(app).lock_app();
        (g.tabs.iter().find(|t| t.id == id && t.gen == gen).map(|t| t.zoom.unwrap_or(g.cfg.settings.page_zoom)), g.tabs.iter().any(|t| t.id == id && t.gen == gen && t.muted))
    };
    if muted {
        engine::set_muted(label, true);
    }
    if let Some(z) = zoom.filter(|z| *z != 100) {
        engine::set_zoom(label, z);
    }
}

/// Main-frame navigation about to start. `false` cancels it (a profile swap takes over).
pub fn on_navigation(app: &AppHandle, label: &str, u: &Url, user_gesture: bool, is_redirect: bool) -> bool {
    let Some((tab_id, gen)) = parse_label(label) else { return true };
    if matches!(u.scheme(), "mailto" | "tel" | "sms") {
        // Hand to the OS, but only for a click: a page must not open the mail app by itself.
        if user_gesture {
            open_external(u.as_str());
        }
        return false;
    }
    if !matches!(u.scheme(), "http" | "https") {
        return true;
    }
    let host = u.host_str().unwrap_or_default().to_string();
    let s = shared(app);
    let decision = {
        let mut g = s.lock_app();
        // A redirect is not a new gesture, but it continues the click that started it.
        let recent = g.gestures.get(&tab_id).filter(|t| t.0.elapsed() < GESTURE_WINDOW).map(|t| t.1.clone());
        let user = user_gesture || recent.is_some();
        // Tab-under: right after a click the page sends this tab to an ad (often opening the link in a new tab).
        // After a click on no link only listed ads are stopped, so script buttons (sign-in) still go cross-site.
        if let (Some(link), false, Some(t)) = (&recent, is_redirect, g.tabs.iter().find(|t| t.id == tab_id)) {
            let cur = t.url();
            let ad = if link.is_empty() { crate::adblock::check(u.as_str(), &cur, "document") } else { crate::adblock::hijack(u.as_str(), &cur, link) };
            if ad {
                return false;
            }
        }
        if user && !is_redirect {
            g.gestures.remove(&tab_id);
            // The click may be on a file link: the download starts after this navigation.
            g.dl_ok.insert(tab_id, Instant::now());
        }
        let Some(t) = g.tabs.iter().find(|t| t.id == tab_id && t.gen == gen) else { return true };
        let (profile, private) = (t.profile.clone(), t.private);
        let mut d = if private { Decision::Allow } else { nav::decide(&g.cfg.mappings, &profile, &host, user) };
        if let Decision::Prompt(h) = &d {
            if g.skipped.contains(h) || g.prompt.is_some() {
                d = Decision::Allow;
            }
        }
        d
    };
    match decision {
        Decision::Allow => true,
        Decision::Prompt(host) => {
            s.lock_app().prompt = Some(host);
            emit_state(app);
            true
        }
        Decision::Swap(profile) => {
            let url = u.to_string();
            engine::defer(move || swap(&AppHandle, tab_id, &profile, &url, true));
            false
        }
    }
}

/// Camera, microphone, location and notification requests. The handler must answer at once, so an
/// unknown site is denied and a card in the sidebar asks; allowing reloads the page to ask again.
pub fn on_permission(app: &AppHandle, label: &str, origin: &str, name: &str) -> bool {
    let Some((tab_id, _)) = parse_label(label) else { return false };
    let Some(host) = Url::parse(origin).ok().and_then(|u| u.host_str().map(String::from)) else { return false };
    let mut g = shared(app).lock_app();
    // Private tabs only see private answers, regular tabs only saved ones.
    let private = g.tab(tab_id).is_some_and(|t| t.private);
    let rules = if private { &g.perm_tmp } else { &g.cfg.permissions };
    let saved = rules.iter().find(|r| r.host == host && r.kind == name).map(|r| r.allow);
    match saved {
        Some(allow) => allow,
        None => {
            // Queue: a pending card is never overwritten.
            let p = (host, name.into(), tab_id);
            if !g.perm_prompt.contains(&p) {
                g.perm_prompt.push(p);
            }
            drop(g);
            emit_state(app);
            false
        }
    }
}

/// A page needs a login (basic auth, proxy) or has a bad certificate. The engine keeps the callback
/// and the answer arrives from the sidebar card. A certificate the user already accepted this
/// session goes straight through; nothing here is ever written to disk.
pub fn on_net_prompt(app: &AppHandle, label: &str, cert: bool, host: &str, detail: &str) -> Net {
    let Some((tab_id, _)) = parse_label(label) else { return Net::Deny };
    let id = {
        let mut g = shared(app).lock_app();
        let private = g.tabs.iter().find(|t| t.id == tab_id).is_some_and(|t| t.private);
        if cert && g.cert_ok.contains(&(host.to_string(), private)) {
            return Net::Allow;
        }
        let id = g.next_net;
        g.next_net += 1;
        g.net_q.push(NetPrompt { id, cert, host: host.into(), detail: detail.into(), tab: tab_id });
        id
    };
    emit_state(app);
    Net::Wait(id)
}

/// The card's answer. Proceeding past a certificate error is remembered for this session only.
pub fn answer_net(app: &AppHandle, id: u64, allow: bool, user: &str, pass: &str) {
    {
        let mut g = shared(app).lock_app();
        let Some(i) = g.net_q.iter().position(|p| p.id == id) else { return };
        let p = g.net_q.remove(i);
        if p.cert && allow {
            let private = g.tabs.iter().find(|t| t.id == p.tab).is_some_and(|t| t.private);
            g.cert_ok.insert((p.host, private));
        }
    }
    engine::net_answer(id, allow, user, pass);
    emit_state(app);
}

/// The sidebar card's answer. Private tabs keep it for the session only.
pub fn answer_permission(app: &AppHandle, allow: bool) {
    let (rule, tab, private) = {
        let mut g = shared(app).lock_app();
        if g.perm_prompt.is_empty() {
            return;
        }
        let (host, kind, tab) = g.perm_prompt.remove(0);
        let private = g.tabs.iter().find(|t| t.id == tab).is_some_and(|t| t.private);
        (config::PermRule { host, kind, allow }, tab, private)
    };
    {
        let s = shared(app);
        let mut g = s.lock_app();
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

pub fn on_title(app: &AppHandle, label: &str, title: String) {
    let Some((id, gen)) = parse_label(label) else { return };
    let s = shared(app);
    let url = {
        let mut g = s.lock_app();
        match g.tab(id) {
            Some(t) if t.gen == gen => {
                if t.title == title {
                    return;
                }
                t.title = title.clone();
                t.url()
            }
            _ => return,
        }
    };
    s.db.set_title(&url, &title);
    emit_state(app);
}

/// The failed address inside one of our error-page URLs.
fn error_target(url: &str) -> Option<String> {
    let u = Url::parse(url).ok()?;
    if u.scheme() != "aditus" || u.host_str() != Some("app") || u.path() != "/error.html" {
        return None;
    }
    u.query_pairs().find(|(k, _)| k == "u").map(|(_, v)| v.into_owned())
}

/// The renderer of a tab died: show the error page in its place instead of a blank view.
pub fn on_crash(app: &AppHandle, label: &str) {
    let Some((id, gen)) = parse_label(label) else { return };
    let url = {
        let mut g = shared(app).lock_app();
        let Some(t) = g.tab(id).filter(|t| t.gen == gen && !t.discarded) else { return };
        t.url()
    };
    if internal_view(&url).is_none() && !is_blank(&url) {
        engine::defer_after(0, move || engine::navigate(&label_of(id, gen), &engine::error_page(&url, "CRASHED")));
    }
}

fn label_of(id: u32, gen: u32) -> String {
    label(id, gen)
}

pub fn on_load(app: &AppHandle, label: &str, url: String, started: bool, fwd_back: bool) {
    let Some((tab_id, gen)) = parse_label(label) else { return };
    // An error page stands for the page that failed: same address, no history entry, no visit.
    let (url, errored) = match error_target(&url) {
        Some(failed) => (failed, true),
        None => (normalize(url), false),
    };
    let s = shared(app);
    let (mut visit, mut zoom, mut restore, mut reveal) = (None, None, None, None);
    {
        let mut g = s.lock_app();
        let base = g.cfg.settings.page_zoom;
        let Some(t) = g.tab(tab_id) else { return };
        if t.gen != gen {
            return;
        }
        t.loading = started;
        if started {
            t.fwd_back = fwd_back;
            // The new view of a profile swap has committed a page: it can replace the old one on screen.
            reveal = t.retiring.take();
            t.link = None;
            t.playing.clear();
            t.audible = false;
            // Keep the old title while a page on the same site reloads (a login click often does),
            // so the tab label doesn't flash its address.
            let host = |u: &str| Url::parse(u).ok().and_then(|u| u.host_str().map(String::from));
            if host(&t.url()) != host(&url) {
                t.title.clear();
            }
        } else {
            record_entry(t, &url, true);
            if errored {
                t.hist.fence();
            }
            zoom = Some(t.zoom.unwrap_or(base));
            if std::mem::take(&mut t.restore) {
                restore = t.snap.take().filter(|(u, _)| *u == url).map(|(_, j)| j);
            }
            visit = (!errored && !t.private && !is_blank(&url) && internal_view(&url).is_none()).then(|| (url, t.title.clone(), t.profile.clone()));
        }
    }
    if let Some(old) = reveal {
        engine::defer(move || reveal_swapped(&AppHandle, tab_id, gen, old));
    }
    if let Some(j) = restore {
        engine::eval(label, &format!("({RESTORE})({})", serde_json::to_string(&j).unwrap()));
    }
    if let Some((url, title, profile)) = visit {
        s.db.add_visit(&url, &title, &profile);
    }
    // Zoom is remembered per host by the engine; the tab's own setting wins on every load.
    if let Some(z) = zoom {
        engine::set_zoom(label, z);
    }
    emit_state(app);
}

/// A main-frame page committed (`load`) or an in-page navigation happened (`!load`).
fn record_entry(t: &mut Tab, url: &str, load: bool) {
    let native = t.native_at.take().is_some_and(|at| at.elapsed() < Duration::from_secs(5));
    let traverse = std::mem::replace(&mut t.mode, Mode::Push) == Mode::Traverse || native;
    // Only a real load settles "fresh" and "traversal"; an early pushState must not use them up.
    let (fresh, fwd_back) = if load { (std::mem::take(&mut t.fresh), std::mem::take(&mut t.fwd_back)) } else { (false, false) };
    t.hist.commit(url, &t.profile, traverse, fresh, fwd_back);
}

/// The new view of a swap committed (or gave up waiting): drop the old one and show the new.
fn reveal_swapped(app: &AppHandle, id: u32, gen: u32, old: String) {
    engine::close(&old);
    let (show, win) = {
        let g = shared(app).lock_app();
        let Some(t) = g.tabs.iter().find(|t| t.id == id && t.gen == gen) else { return };
        (g.active(&t.win) == id && !t.discarded, t.win.clone())
    };
    if show {
        engine::set_visible(&label(id, gen), true);
        hide_all_except(app, &win, Some((id, gen)));
        apply_layout(app);
    }
}

/// Safety net: if the new view never commits (blocked, hung), the swap completes anyway.
fn finish_swap(app: &AppHandle, id: u32, gen: u32) {
    let old = shared(app).lock_app().tab(id).filter(|t| t.gen == gen).and_then(|t| t.retiring.take());
    if let Some(old) = old {
        reveal_swapped(app, id, gen, old);
    }
}

/// `window.open` / target=_blank: a tab beside the opener, in its profile.
/// The link the tab's last recent click landed on ("" when none), for `adblock::hijack`.
pub fn clicked_link(app: &AppHandle, label: &str) -> String {
    let Some((id, _)) = parse_label(label) else { return String::new() };
    let g = shared(app).lock_app();
    g.gestures.get(&id).filter(|t| t.0.elapsed() < GESTURE_WINDOW).map(|t| t.1.clone()).unwrap_or_default()
}

pub fn on_popup(app: &AppHandle, label: &str, url: &str) {
    let Ok(u) = Url::parse(url) else { return };
    if !matches!(u.scheme(), "http" | "https") {
        return;
    }
    let opener = parse_label(label).map(|(id, _)| id);
    let (profile, win, front) = {
        let g = shared(app).lock_app();
        match opener.and_then(|id| g.tabs.iter().find(|t| t.id == id)) {
            Some(t) => (Some(t.profile.clone()), t.win.clone(), !g.cfg.settings.links_background),
            None => (None, g.focused.clone(), !g.cfg.settings.links_background),
        }
    };
    open_tab_in(app, &win, Some(u), profile, front);
}

/// Back/forward availability of the tab behind `label`, for the page context menu.
pub fn nav_state(app: &AppHandle, label: &str) -> (bool, bool) {
    let Some((id, _)) = parse_label(label) else { return (false, false) };
    let g = shared(app).lock_app();
    g.tabs.iter().find(|t| t.id == id).map_or((false, false), |t| (t.hist.can_back(), t.hist.can_forward()))
}

pub fn download_dir() -> Option<PathBuf> {
    let g = shared(&AppHandle).lock_app();
    (!g.cfg.settings.download_dir.is_empty()).then(|| PathBuf::from(&g.cfg.settings.download_dir))
}

/// How long after a click (or one of our own download actions) a tab may start downloads.
const DOWNLOAD_WINDOW: Duration = Duration::from_secs(10);

fn download_fresh(last: Option<Instant>) -> bool {
    last.is_some_and(|t| t.elapsed() < DOWNLOAD_WINDOW)
}

/// Our own UI starts a download in this view (Save Image As, Save Page): let it through.
pub fn allow_download(app: &AppHandle, view_label: &str) {
    if let Some((id, _)) = parse_label(view_label) {
        shared(app).lock_app().dl_ok.insert(id, Instant::now());
    }
}

/// May this view start a download now? Views that are not tabs (app pages) always may; a page that
/// starts one with no recent user gesture is refused, with a toast.
pub fn download_allowed(app: &AppHandle, view_label: &str) -> bool {
    let Some((id, _)) = parse_label(view_label) else { return true };
    let ok = {
        let mut g = shared(app).lock_app();
        let ok = download_fresh(g.dl_ok.get(&id).copied());
        if !ok {
            g.toast = Some(Toast { message: "Blocked an automatic download".into(), undo: false });
        }
        ok
    };
    if !ok {
        emit_state(app);
    }
    ok
}

pub fn on_download_start(app: &AppHandle, url: &str, path: &str) -> i64 {
    let id = shared(app).db.add_download(url, path);
    engine::emit("downloads-changed", "null");
    id
}

/// `id` is the row from `on_download_start`, so concurrent downloads of one URL can't be mixed up.
pub fn on_download_end(app: &AppHandle, id: i64, path: &str, ok: bool) {
    shared(app).db.finish_download(id, path, ok);
    engine::emit("downloads-changed", "null");
}

/// Called from the `page_ping` command with the calling view's label.
/// Applies a saved snapshot: scroll (retried while the page lays out), empty form fields, media position.
const RESTORE: &str = "j=>{const s=JSON.parse(j);let n=0;const go=()=>{n++;try{const els=document.querySelectorAll('input,textarea,select');for(const [i,v] of s.f){const e=els[i];if(!e)continue;if(typeof v==='boolean'){if(e.checked!==v)e.click()}else if(e.value===''){e.value=v;e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}))}}const ms=document.querySelectorAll('video,audio');for(const [i,t] of s.m){const e=ms[i];if(e&&e.currentTime<1)e.currentTime=t}}catch(e){}scrollTo(s.x,s.y);if(n<15&&Math.abs(scrollY-s.y)>3)setTimeout(go,200)};go()}";

/// A page's own report of its scroll / form / media state. Kept as data for `RESTORE`, never interpreted here.
pub fn page_state(app: &AppHandle, view_label: &str, json: &str) {
    let Some((id, gen)) = parse_label(view_label) else { return };
    if json.len() > 60_000 {
        return;
    }
    // The URL comes from the engine, not the page.
    let Some(url) = engine::url_of(view_label).map(normalize) else { return };
    if let Some(t) = shared(app).lock_app().tab(id) {
        if t.gen == gen && !t.discarded {
            t.snap = Some((url, json.to_string()));
        }
    }
}

/// Pointer rests on a sleeping tab: start loading it hidden so the click finds it (nearly) ready.
pub fn prewake(app: &AppHandle, id: u32) {
    let go = {
        let mut g = shared(app).lock_app();
        g.tab(id).map_or(false, |t| {
            let wake = t.discarded;
            if wake {
                t.discarded = false;
                t.loading = true;
                t.restore = true;
                t.last_active = Instant::now();
            }
            wake
        })
    };
    if go {
        build_webview(app, id, false);
        emit_state(app);
    }
}

/// Tells the window's UI where the pointer is over the active page (`x`, `y` as fractions of it), for the mascot.
fn tell_pointer(app: &AppHandle, id: u32, json: &str) {
    let win = {
        let g = shared(app).lock_app();
        let win = g.win_of(id);
        if g.active(&win) != id {
            return;
        }
        win
    };
    engine::emit_to(&wv_label("ui", &win), "page-pointer", json);
}

/// Records one frame's playing state; true when the tab's audibility changed.
fn set_playing(t: &mut Tab, frame: &str, on: bool) -> bool {
    if on {
        t.playing.insert(frame.to_string());
    } else {
        t.playing.remove(frame);
    }
    std::mem::replace(&mut t.audible, !t.playing.is_empty()) != !t.playing.is_empty()
}

/// A subframe started a new load: whatever it was playing is gone.
pub fn frame_reset(app: &AppHandle, view_label: &str, frame: &str) {
    let Some((id, _)) = parse_label(view_label) else { return };
    let changed = shared(app).lock_app().tab(id).is_some_and(|t| set_playing(t, frame, false));
    if changed {
        emit_state(app);
    }
}

pub fn page_ping(app: &AppHandle, view_label: &str, frame: &str, kind: &str, href: &str, x: f64, y: f64) {
    let Some((id, gen)) = parse_label(view_label) else { return };
    let s = shared(app);
    match kind {
        "g" => {
            let mut g = s.lock_app();
            g.gestures.insert(id, (Instant::now(), href.to_string()));
            g.dl_ok.insert(id, Instant::now());
            drop(g);
            tell_pointer(app, id, r#"{"c":1}"#);
        }
        "m" => tell_pointer(app, id, &format!(r#"{{"x":{:.2},"y":{:.2}}}"#, x.clamp(0.0, 1.0), y.clamp(0.0, 1.0))),
        "b" | "f" => go_history(app, id, if kind == "b" { -1 } else { 1 }),
        "p1" | "p0" => {
            let changed = {
                let mut g = s.lock_app();
                g.tab(id).is_some_and(|t| set_playing(t, frame, kind == "p1"))
            };
            if changed {
                emit_state(app);
            }
        }
        "r" => {
            // history.replaceState: the entry changed address, there is no new one.
            let Some(url) = engine::url_of(view_label) else { return };
            let changed = {
                let mut g = s.lock_app();
                g.tab(id).filter(|t| t.gen == gen).is_some_and(|t| {
                    let url = normalize(url);
                    let same = t.url() == url;
                    if !same {
                        t.hist.replace(&url);
                    }
                    !same
                })
            };
            if changed {
                emit_state(app);
            }
        }
        "u" => {
            // SPA navigation: trust the engine's URL, not the page.
            let Some(url) = engine::url_of(view_label) else { return };
            let mut g = s.lock_app();
            // Pages call replaceState constantly; only a real change is worth redrawing the UI.
            let before = g.tab(id).map(|t| t.url());
            if let Some(t) = g.tab(id) {
                if t.gen == gen {
                    record_entry(t, &url, false);
                }
            }
            let changed = g.tab(id).map(|t| t.url()) != before;
            drop(g);
            if changed {
                emit_state(app);
            }
        }
        _ => {}
    }
}

/// The page says which link its selection holds ("" = none). Only web addresses are kept.
pub fn page_link(app: &AppHandle, view_label: &str, url: &str) {
    let Some((id, gen)) = parse_label(view_label) else { return };
    let link = Url::parse(url).ok().filter(|u| matches!(u.scheme(), "http" | "https"));
    if let Some(t) = shared(app).lock_app().tab(id).filter(|t| t.gen == gen) {
        t.link = link;
    }
}

/// What a new tab or private window should open, and in which profile: the address bar's selected
/// text (read as if Enter was pressed there), else the link selected in the active tab's page.
pub fn selected_link(app: &AppHandle, win: &str, id: u32) -> Option<(Url, String)> {
    let mut g = shared(app).lock_app();
    let typed = g.win(win).address_sel.clone();
    let template = g.cfg.settings.search_template.clone();
    let t = g.tabs.iter().find(|t| t.id == id)?;
    let typed = (!typed.is_empty()).then(|| search::resolve(&typed, &template)).filter(|u| matches!(u.scheme(), "http" | "https"));
    Some((typed.or_else(|| t.link.clone())?, t.profile.clone()))
}

// ---------- tab operations ----------

fn hide_all_except(app: &AppHandle, win: &str, keep: Option<(u32, u32)>) {
    let labels: Vec<String> = {
        let g = shared(app).lock_app();
        g.tabs.iter().filter(|t| t.win == win).flat_map(|t| [Some(label(t.id, t.gen)), t.retiring.clone()]).flatten().collect()
    };
    let keep = keep.map(|(i, g)| label(i, g));
    for l in labels {
        engine::set_visible(&l, keep.as_deref() == Some(l.as_str()));
    }
}

/// New tab in the window that last had focus.
pub fn open_tab(app: &AppHandle, url: Option<Url>, profile: Option<String>, activate_it: bool) -> u32 {
    let win = shared(app).lock_app().focused.clone();
    open_tab_in(app, &win, url, profile, activate_it)
}

pub fn open_tab_in(app: &AppHandle, win: &str, url: Option<Url>, profile: Option<String>, activate_it: bool) -> u32 {
    let id = {
        let mut g = shared(app).lock_app();
        g.win(win);
        // No explicit profile: a mapped site opens in its profile, anything else in the default one.
        let mapped = url.as_ref().and_then(|u| u.host_str()).and_then(|h| nav::lookup(&g.cfg.mappings, h)).map(|m| m.profile.clone());
        let profile = profile.or(mapped).and_then(|p| g.cfg.profile(&p).map(|p| p.id.clone())).unwrap_or_else(|| g.cfg.profiles.first().map(|p| p.id.clone()).unwrap_or_default());
        let id = g.next_id;
        g.next_id += 1;
        let private = g.wins.get(win).is_some_and(|w| w.incognito);
        let hist = url.map(|u| Hist::seeded(u.as_str(), &profile)).unwrap_or_default();
        g.tabs.push(Tab {
            id,
            win: win.to_string(),
            profile,
            hist,
            title: String::new(),
            pinned: false,
            group: None,
            discarded: true,
            loading: false,
            audible: false,
            playing: HashSet::new(),
            muted: false,
            private,
            zoom: None,
            gen: 0,
            last_active: Instant::now(),
            snap: None,
            restore: false,
            pid: None,
            mem_mb: 0,
            mode: Mode::Push,
            fresh: false,
            fwd_back: false,
            native_at: None,
            retiring: None,
            link: None,
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
        let mut g = shared(app).lock_app();
        let Some(win) = g.tabs.iter().find(|t| t.id == id).map(|t| t.win.clone()) else { return };
        g.win(&win).active = id;
        let Some(t) = g.tab(id) else { return };
        t.last_active = Instant::now();
        let nb = t.discarded;
        t.discarded = false;
        if nb {
            t.loading = true;
            t.restore = true;
        }
        (nb, t.gen, win)
    };
    hide_all_except(app, &win, Some((id, gen)));
    if need_build {
        // The recreated tab starts at its current history entry.
        build_webview(app, id, true);
    } else {
        apply_layout(app);
    }
    emit_state(app);
}

pub fn close_tab(app: &AppHandle, id: u32) {
    let (lbl, next, win, left, retiring) = {
        let mut g = shared(app).lock_app();
        let Some(pos) = g.tabs.iter().position(|t| t.id == id) else { return };
        let t = g.tabs.remove(pos);
        g.gestures.remove(&id);
        g.dl_ok.remove(&id);
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
        (label(t.id, t.gen), next, t.win.clone(), same.len(), t.retiring.clone())
    };
    if let Some(r) = retiring {
        engine::defer(move || engine::close(&r));
    }
    engine::close(&lbl);
    match next {
        Some(n) => activate(app, n),
        None if left == 0 => {
            // Last tab of a secondary window closes the window; the only window gets a blank tab.
            let others = shared(app).lock_app().wins.len() > 1;
            if others && win != "main" {
                engine::close_window(&win);
            } else {
                open_tab_in(app, &win, None, None, true);
            }
        }
        None => emit_state(app),
    }
}

/// Cmd/Ctrl+Shift+T: reopen the most recently closed tab in its profile.
pub fn reopen_closed(app: &AppHandle) {
    let last = shared(app).lock_app().closed.pop();
    if let Some((url, profile)) = last {
        open_tab(app, Url::parse(&url).ok(), Some(profile), true);
    }
}

/// Replace the tab's view with one in `profile`, loading `url`.
///
/// A view is bound to one request context, so another profile needs a new view (CEF cannot move a
/// browser between contexts). The old view stays on screen until the new one commits a page, so the
/// tab never flashes blank. What cannot survive: the page's JS state, a POST body (the cancelled
/// request is replayed as a GET) and `window.opener`.
pub fn swap(app: &AppHandle, tab_id: u32, profile: &str, url: &str, announce: bool) {
    let (old_label, is_active, drop_now, gen) = {
        let mut g = shared(app).lock_app();
        let active = g.active(&g.win_of(tab_id));
        let Some(t) = g.tab(tab_id) else { return };
        if t.private {
            return;
        }
        let previous = Undo { tab: tab_id, profile: t.profile.clone(), url: t.url() };
        let had_view = !t.discarded;
        let old = label(t.id, t.gen);
        // A swap still waiting on its page: that unfinished view goes at once, the first old view stays up.
        let (keep, drop_now) = match t.retiring.take() {
            Some(first) if had_view => (Some(first), Some(old)),
            _ if had_view => (Some(old), None),
            _ => (None, Some(old)),
        };
        t.retiring = keep.clone();
        t.gen += 1;
        t.profile = profile.to_string();
        t.title.clear();
        t.loading = true;
        t.discarded = false;
        if t.mode == Mode::Traverse {
            // History entry already exists; keep idx and mode.
        } else {
            t.hist.push(url, profile);
            t.mode = Mode::Traverse; // the load below must not push a second entry
        }
        if announce {
            g.undo = Some(previous);
            g.toast = Some(Toast { message: format!("Switched to profile {}", g.cfg.name_of(profile)), undo: true });
        }
        let gen = g.tab(tab_id).map_or(0, |t| t.gen);
        (keep, active == tab_id, drop_now, gen)
    };
    if let Some(d) = drop_now {
        engine::close(&d);
    }
    // The old view (if any) keeps the screen; the new one is created hidden and revealed on commit.
    build_webview(app, tab_id, is_active && old_label.is_none());
    if old_label.is_some() {
        engine::defer_after(8000, move || finish_swap(&AppHandle, tab_id, gen));
    }
    emit_state(app);
}

pub fn undo_swap(app: &AppHandle) {
    let u = {
        let mut g = shared(app).lock_app();
        g.toast = None;
        g.undo.take()
    };
    if let Some(u) = u {
        if let Some(t) = shared(app).lock_app().tab(u.tab) {
            t.mode = Mode::Push;
        }
        swap(app, u.tab, &u.profile, &u.url, false);
    }
    emit_state(app);
}

pub fn dismiss_toast(app: &AppHandle) {
    {
        let mut g = shared(app).lock_app();
        g.toast = None;
        g.undo = None;
    }
    emit_state(app);
}

/// Load a URL in a tab, honoring profile mappings. Address-bar loads are user-initiated.
pub fn navigate(app: &AppHandle, tab_id: u32, url: Url, mode_push: bool) {
    let s = shared(app);
    let (decision, lbl, was_discarded, fresh_view) = {
        let mut g = s.lock_app();
        let host = url.host_str().unwrap_or_default().to_string();
        let Some(t) = g.tabs.iter().find(|t| t.id == tab_id) else { return };
        let (profile, lbl, disc) = (t.profile.clone(), label(t.id, t.gen), t.discarded);
        let was_blank = is_blank(&t.url());
        let mut d = nav::decide(&g.cfg.mappings, &profile, &host, mode_push);
        if let Decision::Prompt(h) = &d {
            if g.skipped.contains(h) || g.prompt.is_some() {
                d = Decision::Allow;
            }
        }
        if let Some(t) = g.tab(tab_id) {
            t.mode = if mode_push { Mode::Push } else { Mode::Traverse };
        }
        // The new-tab view is an app page: its renderer never gets the page scripts, so web content needs a fresh view.
        let fresh = was_blank && !disc && matches!(d, Decision::Allow | Decision::Prompt(_));
        (d, lbl, disc, fresh.then_some(profile))
    };
    match decision {
        Decision::Swap(p) => swap(app, tab_id, &p, url.as_str(), true),
        other => {
            if let Decision::Prompt(h) = other {
                s.lock_app().prompt = Some(h);
            }
            if was_discarded {
                // No view: seed the history entry, then recreate on activation.
                {
                    let mut g = s.lock_app();
                    if let Some(t) = g.tab(tab_id) {
                        if mode_push {
                            let profile = t.profile.clone();
                            t.hist.push(url.as_str(), &profile);
                            t.mode = Mode::Traverse;
                        }
                    }
                }
                activate(app, tab_id);
            } else if let Some(p) = fresh_view {
                swap(app, tab_id, &p, url.as_str(), false);
            } else {
                engine::navigate(&lbl, url.as_str());
            }
            emit_state(app);
        }
    }
}

/// Back/forward. Inside the current view's native history this is CEF's own `go_back` / `go_forward`
/// (bfcache, scroll and form restore, POST re-submit); the tab's own list only decides when that is
/// not possible: another profile, a view built after the entry was visited (restart, wake-up, swap),
/// or an error page in between.
pub fn go_history(app: &AppHandle, tab_id: u32, delta: i32) {
    let (step, lbl, profile, cur_url) = {
        let mut g = shared(app).lock_app();
        let Some(t) = g.tab(tab_id) else { return };
        let step = t.hist.step(delta, &t.profile);
        if step == Step::None {
            return;
        }
        let cur_url = t.url(); // already the target
        if step == Step::Native {
            t.native_at = Some(Instant::now());
        } else {
            t.mode = Mode::Traverse;
        }
        (step, label(t.id, t.gen), t.profile.clone(), cur_url)
    };
    let entry = match step {
        Step::Native if engine::go(&lbl, delta) => return,
        Step::Native => {
            // No live view (sleeping tab) or the engine disagrees: load the target instead.
            let mut g = shared(app).lock_app();
            let Some(t) = g.tab(tab_id) else { return };
            t.hist.fresh_view();
            t.native_at = None;
            t.mode = Mode::Traverse;
            Entry { url: cur_url, profile: profile.clone() }
        }
        Step::Load(e) => e,
        Step::None => return,
    };
    let Ok(url) = Url::parse(&entry.url) else { return };
    // App pages (new tab, settings) and web pages need different views; so do profiles.
    let on_app_view = engine::url_of(&lbl).is_some_and(|u| u.starts_with(&format!("{}://", crate::app::SCHEME)));
    let discarded = shared(app).lock_app().tab(tab_id).is_some_and(|t| t.discarded);
    let new_view = entry.profile != profile || (!discarded && (on_app_view || is_blank(&entry.url) || internal_view(&entry.url).is_some()));
    if new_view {
        swap(app, tab_id, &entry.profile, &entry.url, false);
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
    let win = shared(app).lock_app().focused.clone();
    engine::raise_window(&win);
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

/// Cmd/Ctrl +, -, 0: `step` is +1, -1, or 0 to reset. Remembered per tab so a rebuilt view keeps it.
pub fn zoom(app: &AppHandle, tab_id: u32, step: i32) {
    let pct = {
        let mut g = shared(app).lock_app();
        let base = g.cfg.settings.page_zoom;
        let Some(t) = g.tab(tab_id) else { return };
        let pct = if step == 0 { base } else { (t.zoom.unwrap_or(base) as i32 + step * 10).clamp(30, 300) as u32 };
        t.zoom = (step != 0).then_some(pct);
        pct
    };
    if let Some(l) = active_view(app, tab_id) {
        engine::set_zoom(&l, pct);
    }
}

/// File > Print / Save Page.
/// The current (real, not normalized) URL of a tab, when it is a web page.
pub fn active_url(app: &AppHandle, tab_id: u32) -> Option<String> {
    let l = active_view(app, tab_id)?;
    engine::url_of(&l).filter(|u| u.starts_with("http://") || u.starts_with("https://"))
}

/// Star or unstar the page in a tab. Private windows keep no bookmarks.
pub fn toggle_bookmark(app: &AppHandle, tab_id: u32) {
    let (url, title, profile) = {
        let mut g = shared(app).lock_app();
        let Some(t) = g.tab(tab_id) else { return };
        if t.private {
            return;
        }
        (t.url(), t.title.clone(), t.profile.clone())
    };
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return;
    }
    let s = shared(app);
    if s.db.is_bookmarked(&url) {
        s.db.remove_bookmark(&url);
    } else {
        s.db.add_bookmark(&url, &title, &profile);
    }
    emit_state(app);
}

pub fn toggle_mute(app: &AppHandle, tab_id: u32) {
    let (lbl, muted) = {
        let mut g = shared(app).lock_app();
        let Some(t) = g.tab(tab_id) else { return };
        t.muted = !t.muted;
        (label(t.id, t.gen), t.muted)
    };
    engine::set_muted(&lbl, muted);
    emit_state(app);
}

pub fn page_action(app: &AppHandle, tab_id: u32, act: &str) {
    let Some(l) = active_view(app, tab_id) else { return };
    match act {
        "print" => engine::print(&l),
        "reader" => engine::eval(&l, crate::scripts::READER),
        _ => {
            allow_download(app, &l);
            engine::eval(&l, "(()=>{const a=document.createElement('a');a.href=location.href;a.download=(document.title||'page')+'.html';document.body.appendChild(a);a.click();a.remove()})()");
        }
    }
}

pub fn label_private(app: &AppHandle, view_label: &str) -> bool {
    let Some((id, _)) = parse_label(view_label) else { return true };
    shared(app).lock_app().tabs.iter().find(|t| t.id == id).map_or(true, |t| t.private)
}

pub fn profile_of(app: &AppHandle, tab_id: u32) -> Option<(String, bool)> {
    shared(app).lock_app().profile_id(tab_id)
}

/// Fill the active tab with its saved login; with several users saved for the origin, a menu at `pos` asks which.
/// The origin is read from the engine right here, and only that exact origin's entry is used.
pub fn fill_login(app: &AppHandle, tab_id: u32, pos: (i32, i32)) {
    let Some(l) = active_view(app, tab_id) else { return };
    let Some(origin) = engine::url_of(&l).and_then(|u| crate::passwords::origin_of(&u)) else { return };
    let Some((pid, _)) = profile_of(app, tab_id) else { return };
    let mut users: Vec<String> = shared(app).db.logins(Some(&pid), Some(&origin)).into_iter().map(|l| l.1).collect();
    if users.len() > 1 {
        let win = shared(app).lock_app().tabs.iter().find(|t| t.id == tab_id).map(|t| t.win.clone()).unwrap_or_default();
        let items =
            users.into_iter().map(|u| engine::Item::Entry { label: if u.is_empty() { "(no username)".into() } else { u.clone() }, id: format!("login:{u}"), enabled: true, checked: false }).collect();
        engine::popup(&win, items, pos);
    } else if let Some(user) = users.pop() {
        fill_login_as(app, tab_id, &user);
    }
}

/// Fills `user`'s saved password into the active tab (menu pick or the single saved login).
pub fn fill_login_as(app: &AppHandle, tab_id: u32, user: &str) {
    let Some(l) = active_view(app, tab_id) else { return };
    let Some(origin) = engine::url_of(&l).and_then(|u| crate::passwords::origin_of(&u)) else { return };
    let Some((pid, first)) = profile_of(app, tab_id) else { return };
    if !shared(app).db.logins(Some(&pid), Some(&origin)).iter().any(|l| l.1 == user) {
        return;
    }
    if let Some(pw) = crate::passwords::get(crate::passwords::SERVICE, &pid, &origin, user, first) {
        engine::eval(&l, &crate::passwords::fill_js(user, &pw));
    }
}

pub fn reload(app: &AppHandle, tab_id: u32) {
    reload_with(app, tab_id, "");
}

/// `pre` runs first in the page; it survives the reload only via storage.
pub fn reload_with(app: &AppHandle, tab_id: u32, pre: &str) {
    let lbl = {
        let mut g = shared(app).lock_app();
        let Some(t) = g.tab(tab_id) else { return };
        t.mode = Mode::Traverse;
        label(t.id, t.gen)
    };
    engine::eval(&lbl, &format!("{pre}location.reload()"));
}

pub fn address_bar(app: &AppHandle, tab_id: u32, input: &str) {
    let (template, internal, win) = {
        let g = shared(app).lock_app();
        (g.cfg.settings.search_template.clone(), g.tabs.iter().find(|t| t.id == tab_id).is_some_and(|t| internal_view(&t.url()).is_some()), g.win_of(tab_id))
    };
    if let Some(view) = internal_view(input.trim()).filter(|v| matches!(v.as_str(), "settings" | "history" | "downloads" | "bookmarks")) {
        return open_internal(app, &win, &view);
    }
    // easter egg: ask the door nicely
    if input.trim().eq_ignore_ascii_case("open sesame") {
        shared(app).lock_app().toast = Some(Toast { message: "Aditus: the door was never locked.".into(), undo: false });
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
    let Some(profile) = shared(app).lock_app().cfg.profile(profile).map(|p| p.id.clone()) else { return };
    let profile = profile.as_str();
    // Mappings match on the host only: drop scheme, path, query and port if a full URL was typed.
    let host = nav::clean_pattern(host);
    let host = host.as_str();
    if host.is_empty() {
        return;
    }
    let to_swap = {
        let mut g = shared(app).lock_app();
        g.prompt = None;
        g.cfg.mappings.retain(|m| m.pattern != host);
        g.cfg.mappings.push(nav::Mapping { pattern: host.into(), profile: profile.into() });
        let active = g.focused_active();
        g.tab(active).filter(|t| t.profile != profile && Url::parse(&t.url()).ok().and_then(|u| u.host_str().map(|h| nav::matches(host, h))).unwrap_or(false)).map(|t| (t.id, t.url()))
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
        let mut g = shared(app).lock_app();
        if let Some(h) = g.prompt.take() {
            g.skipped.insert(h);
        }
    }
    emit_state(app);
}

/// Destroy views of tabs idle past the configured timeout.
pub fn discard_idle(app: &AppHandle) {
    let victims: Vec<String> = {
        let mut g = shared(app).lock_app();
        let limit = Duration::from_secs(g.cfg.settings.idle_minutes * 60);
        let actives: Vec<u32> = g.wins.values().map(|w| w.active).collect();
        g.tabs
            .iter_mut()
            .filter(|t| !actives.contains(&t.id) && !t.pinned && !t.audible && !t.discarded && t.last_active.elapsed() > limit)
            .flat_map(|t| {
                t.discarded = true;
                t.loading = false;
                [Some(label(t.id, t.gen)), t.retiring.take()]
            })
            .flatten()
            .collect()
    };
    for l in &victims {
        engine::close(l);
    }
    if !victims.is_empty() {
        emit_state(app);
    }
    sleep_for_memory(app);
}

/// Process table as (pid, ppid, rss KB). Forks `ps`, so call it off the UI thread; empty where there is no `ps`.
fn process_rows() -> Vec<(u32, u32, u64)> {
    if !cfg!(unix) {
        return Vec::new();
    }
    let Ok(out) = std::process::Command::new("ps").args(["-axo", "pid=,ppid=,rss="]).output() else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace().map(|x| x.parse::<u64>().ok());
            Some((f.next()?? as u32, f.next()?? as u32, f.next()??))
        })
        .collect()
}

/// Gives each tab its share of its renderer's resident memory and returns the MB used by this
/// process and its descendants (shared pages count once per process, so it overstates the real footprint a little).
fn apply_memory(app: &AppHandle, rows: &[(u32, u32, u64)]) -> u64 {
    let mut tree = vec![std::process::id()];
    let mut kb = 0;
    while let Some(p) = tree.pop() {
        for &(pid, ppid, rss) in rows {
            if pid == p {
                kb += rss;
            } else if ppid == p {
                tree.push(pid);
            }
        }
    }
    let mut changed = false;
    {
        let mut g = shared(app).lock_app();
        let mut users: HashMap<u32, u64> = HashMap::new();
        for p in g.tabs.iter().filter(|t| !t.discarded).filter_map(|t| t.pid) {
            *users.entry(p).or_default() += 1;
        }
        for t in g.tabs.iter_mut() {
            let mb = match t.pid.filter(|_| !t.discarded) {
                Some(p) => rows.iter().find(|r| r.0 == p).map_or(0, |r| r.2 / 1024 / users.get(&p).copied().unwrap_or(1).max(1)) as u32,
                None => 0,
            };
            changed |= mb.abs_diff(t.mem_mb) >= 5;
            t.mem_mb = mb;
        }
    }
    if changed {
        emit_state(app);
    }
    kb / 1024
}

/// The renderer of a tab's main frame reported its process id (it changes when a navigation swaps processes).
pub fn set_pid(app: &AppHandle, view_label: &str, pid: u32) {
    let Some((id, gen)) = parse_label(view_label) else { return };
    if let Some(t) = shared(app).lock_app().tab(id) {
        if t.gen == gen {
            t.pid = Some(pid);
        }
    }
}

/// Over the memory limit: sleep the least recently used background tab (one per tick).
fn sleep_for_memory(_app: &AppHandle) {
    // `ps` runs on a worker so the UI thread never blocks on a fork; the result is applied back on the UI thread.
    std::thread::spawn(|| {
        let rows = process_rows();
        engine::defer_from_thread(move || sleep_for_memory_with(&AppHandle, &rows));
    });
}

fn sleep_for_memory_with(app: &AppHandle, rows: &[(u32, u32, u64)]) {
    let total = apply_memory(app, rows);
    let limit = shared(app).lock_app().cfg.settings.memory_limit_mb;
    if limit == 0 || total <= limit {
        return;
    }
    let victim = {
        let mut g = shared(app).lock_app();
        let actives: Vec<u32> = g.wins.values().map(|w| w.active).collect();
        g.tabs.iter_mut().filter(|t| !actives.contains(&t.id) && !t.pinned && !t.audible && !t.discarded).min_by_key(|t| t.last_active).map(|t| {
            t.discarded = true;
            t.loading = false;
            [Some(label(t.id, t.gen)), t.retiring.take()]
        })
    };
    if let Some(ls) = victim {
        for l in ls.into_iter().flatten() {
            engine::close(&l);
        }
        emit_state(app);
    }
}

pub fn with_tab(app: &AppHandle, id: u32, f: impl FnOnce(&mut Tab)) {
    if let Some(t) = shared(app).lock_app().tab(id) {
        f(t);
    }
    emit_state(app);
}

pub fn reorder(app: &AppHandle, id: u32, to: usize) {
    {
        let mut g = shared(app).lock_app();
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
        let g = shared(app).lock_app();
        let ids: Vec<u32> = g.tabs.iter().filter(|t| t.win == g.focused).map(|t| t.id).collect();
        let Some(pos) = ids.iter().position(|&i| i == g.focused_active()) else { return };
        ids[((pos as i32 + delta).rem_euclid(ids.len() as i32)) as usize]
    };
    activate(app, next);
}

/// Cmd/Ctrl+1..9: the n-th tab of the focused window (1-based); 9 is always the last one.
pub fn select(app: &AppHandle, n: usize) {
    let id = {
        let g = shared(app).lock_app();
        let ids: Vec<u32> = g.tabs.iter().filter(|t| t.win == g.focused).map(|t| t.id).collect();
        if n >= 9 {
            ids.last().copied()
        } else {
            ids.get(n - 1).copied()
        }
    };
    if let Some(id) = id {
        activate(app, id);
    }
}

/// Label of the tab's live view, if it has one.
pub fn active_view(app: &AppHandle, id: u32) -> Option<String> {
    let l = {
        let g = shared(app).lock_app();
        let t = g.tabs.iter().find(|t| t.id == id)?;
        label(t.id, t.gen)
    };
    engine::has_view(&l).then_some(l)
}

// ---------- windows ----------

pub fn create_window(app: &AppHandle, label: &str, private: bool) -> bool {
    shared(app).lock_app().win(label);
    engine::create_window(label, private)
}

/// New window with one blank tab; it takes focus.
pub fn new_window(app: &AppHandle, incognito: bool, url: Option<Url>) {
    let label = {
        let mut g = shared(app).lock_app();
        let n = g.next_win;
        g.next_win += 1;
        format!("w{n}")
    };
    {
        let mut g = shared(app).lock_app();
        g.win(&label).incognito = incognito;
    }
    if !create_window(app, &label, incognito) {
        shared(app).lock_app().wins.remove(&label);
        return eprintln!("new window failed");
    }
    shared(app).lock_app().focused = label.clone();
    if incognito {
        engine::set_title(&label, "Aditus (Private)");
    }
    open_tab_in(app, &label, url, None, true);
}

pub fn set_focused(app: &AppHandle, win: &str) {
    let mut g = shared(app).lock_app();
    if g.wins.contains_key(win) {
        g.focused = win.to_string();
    }
}

/// A window went away: drop its tabs (remembered for "reopen closed tab") and its state.
pub fn window_closed(app: &AppHandle, label: &str) {
    {
        let mut g = shared(app).lock_app();
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
    }
    engine::drop_context(&format!("private-{label}"));
    emit(app, false);
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
    /// Back/forward list around the current page (older sessions have none).
    #[serde(default)]
    history: Vec<Entry>,
    #[serde(default)]
    idx: usize,
}

#[derive(Serialize, serde::Deserialize, Default)]
struct Session {
    tabs: Vec<Saved>,
}

fn session_path(app: &AppHandle) -> PathBuf {
    shared(app).data_dir.join("session.json")
}

/// Forget the open tabs' URLs and titles (clear-history-on-quit).
pub fn clear_session(app: &AppHandle) {
    let _ = std::fs::remove_file(session_path(app));
}

pub fn save_session(app: &AppHandle) {
    let session = {
        let g = shared(app).lock_app();
        Session {
            tabs: g
                .tabs
                .iter()
                .filter(|t| !t.private)
                .map(|t| {
                    let (history, idx) = t.hist.for_save();
                    Saved {
                        url: t.url(),
                        profile: t.profile.clone(),
                        pinned: t.pinned,
                        group: t.group.clone(),
                        title: t.title.clone(),
                        win: t.win.clone(),
                        active: g.active(&t.win) == t.id,
                        history,
                        idx,
                    }
                })
                .collect(),
        }
    };
    if let Ok(s) = serde_json::to_string(&session) {
        let _ = crate::applog::check("session save", config::write_atomic(&session_path(app), s));
    }
}

pub fn restore_session(app: &AppHandle) {
    let session: Session = std::fs::read_to_string(session_path(app)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let (startup, home) = {
        let g = shared(app).lock_app();
        (g.cfg.settings.startup.clone(), g.cfg.settings.home_url.clone())
    };
    if startup != "restore" || session.tabs.is_empty() {
        let url = if startup == "home" { Url::parse(&home).ok() } else { None };
        open_tab_in(app, "main", url, None, true);
        return;
    }
    // Sessions saved before profile ids hold names; `profile()` takes either, entries are rewritten to ids.
    let profiles: Vec<String> = shared(app).lock_app().cfg.profiles.iter().map(|p| p.id.clone()).collect();
    let mut to_activate: HashMap<String, u32> = HashMap::new();
    let mut first_of: HashMap<String, u32> = HashMap::new();
    let mut made: HashSet<String> = HashSet::from(["main".to_string()]);
    for mut s in session.tabs {
        if !made.contains(&s.win) {
            if let Some(n) = s.win.strip_prefix('w').and_then(|n| n.parse::<u32>().ok()) {
                let mut g = shared(app).lock_app();
                g.next_win = g.next_win.max(n + 1);
            }
            if !create_window(app, &s.win, false) {
                continue;
            }
            made.insert(s.win.clone());
        }
        let profile = {
            let g = shared(app).lock_app();
            for e in &mut s.history {
                if let Some(p) = g.cfg.profile(&e.profile) {
                    e.profile = p.id.clone();
                }
            }
            // open_tab_in resolves a name or id and falls back to the first profile.
            s.profile.clone()
        };
        let id = open_tab_in(app, &s.win, Url::parse(&s.url).ok(), Some(profile), false);
        with_tab(app, id, |t| {
            let profile = t.profile.clone();
            t.hist.restore(s.history, s.idx, &s.url, &profile, |p| profiles.iter().any(|x| x == p));
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

// ---------- start-up ----------

/// CEF is up: first window, menu, session, background work.
pub fn startup(app: &AppHandle) {
    let s = shared(app);
    create_window(app, "main", false);
    crate::menu::install();
    crate::adblock::init(s.data_dir.join("filters"));
    crate::extensions::init_updates();
    {
        let days = s.lock_app().cfg.settings.history_days;
        if days > 0 {
            s.db.prune_history(days);
        }
    }
    restore_session(app);
    if crate::config::BACKED_UP.load(std::sync::atomic::Ordering::Relaxed) {
        s.lock_app().toast = Some(Toast { message: "config.toml was invalid; saved as config.toml.bak".into(), undo: false });
    }
    open_urls(app, std::env::args().skip(1));
    crate::drain_open_queue();
    idle_tick();
}

/// Every 10 s: discard idle tabs, and sleep tabs while over the memory limit.
fn idle_tick() {
    engine::defer_after(10_000, || {
        discard_idle(&AppHandle);
        idle_tick();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hot(id: u32, title: &str, loading: bool, mem: u32) -> Hot {
        Hot { id, title: title.into(), loading, mem_mb: mem }
    }

    #[test]
    fn plan_full_skip_delta() {
        let a = vec![hot(1, "a", false, 10), hot(2, "b", false, 20)];
        let sent = Sent { key: "k".into(), hot: a.clone(), seq: 3 };
        assert!(matches!(plan(None, "k", &a), Out::Full));
        assert!(matches!(plan(Some(&sent), "other", &a), Out::Full), "structure changed");
        assert!(matches!(plan(Some(&sent), "k", &a[..1]), Out::Full), "tab closed");
        assert!(matches!(plan(Some(&sent), "k", &[hot(1, "a", false, 10), hot(3, "b", false, 20)]), Out::Full), "tab replaced");
        assert!(matches!(plan(Some(&sent), "k", &a), Out::Skip));
        let b = vec![hot(1, "a", true, 10), hot(2, "b2", false, 25)];
        let Out::Delta(d) = plan(Some(&sent), "k", &b) else { panic!("expected delta") };
        assert_eq!(d, vec![serde_json::json!({"id": 1, "loading": true}), serde_json::json!({"id": 2, "title": "b2", "mem_mb": 25})]);
    }

    #[test]
    fn cfg_splice() {
        assert_eq!(with_cfg("{\"a\":1}", "{\"b\":2}"), "{\"a\":1,\"cfg\":{\"b\":2}}");
    }

    #[test]
    fn labels() {
        assert_eq!(parse_label(&label(7, 3)), Some((7, 3)));
        assert_eq!(parse_label("tab-7"), None);
        assert_eq!(parse_label("ui"), None);
        assert_eq!(parse_label("tab-x-1"), None);
        assert_eq!(wv_label("ui", "main"), "ui");
        assert_eq!(wv_label("ui", "w2"), "ui-w2");
    }

    #[test]
    fn internal_views() {
        assert_eq!(internal_view("aditus://settings").as_deref(), Some("settings"));
        assert_eq!(internal_view("aditus://app/index.html?view=history").as_deref(), Some("history"));
        assert_eq!(internal_view("aditus://app/assets/x.js"), None);
        assert_eq!(internal_view("https://example.com/"), None);
        assert_eq!(normalize("aditus://app/?view=downloads".into()), "aditus://downloads");
        assert_eq!(normalize("https://example.com/".into()), "https://example.com/");
    }

    #[test]
    fn blank() {
        assert!(is_blank(""));
        assert!(is_blank("aditus://app/newtab.html"));
        assert!(!is_blank("https://example.com/"));
    }

    #[test]
    fn download_needs_recent_gesture() {
        assert!(!download_fresh(None));
        assert!(download_fresh(Some(Instant::now())));
        assert!(!download_fresh(Instant::now().checked_sub(DOWNLOAD_WINDOW + Duration::from_secs(1))));
    }

    #[test]
    fn guard_blocks_engine_calls_until_dropped() {
        let s = Shared { app: Mutex::new(App::new(Config::default())), db: Db::open(std::path::Path::new(":memory:")).unwrap(), cfg_path: PathBuf::new(), data_dir: PathBuf::new() };
        assert_unlocked();
        let g = s.lock_app();
        assert_eq!(g.focused, "main");
        assert!(std::panic::catch_unwind(assert_unlocked).is_err());
        drop(g);
        assert_unlocked();
        s.lock_app().focused = "w2".into();
        assert_unlocked();
    }

    #[test]
    fn cert_errors_have_reasons() {
        assert!(cert_error_text(-201).contains("expired"));
        assert!(cert_error_text(-200).contains("different site"));
        assert!(cert_error_text(-999).contains("could not be verified"));
    }
}
