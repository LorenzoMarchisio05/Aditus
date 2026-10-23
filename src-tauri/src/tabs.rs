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
    webview::{DownloadEvent, NewWindowResponse, PageLoadEvent},
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewBuilder, WebviewUrl,
};
use url::Url;

const GESTURE_WINDOW: Duration = Duration::from_millis(2500);
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
    pub profile: String,
    pub entries: Vec<Entry>,
    pub idx: usize,
    pub title: String,
    pub pinned: bool,
    pub group: Option<String>,
    pub discarded: bool,
    pub loading: bool,
    pub audible: bool,
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

pub struct App {
    pub cfg: Config,
    pub tabs: Vec<Tab>,
    pub active: u32,
    next_id: u32,
    pub sidebar: f64,
    pub top: f64,
    prompt: Option<String>,
    skipped: HashSet<String>,
    toast: Option<Toast>,
    undo: Option<Undo>,
    gestures: HashMap<u32, Instant>,
    pub overlay_open: bool,
}

impl App {
    pub fn new(cfg: Config) -> Self {
        Self {
            cfg,
            tabs: vec![],
            active: 0,
            next_id: 1,
            sidebar: 240.0,
            top: 44.0,
            prompt: None,
            skipped: HashSet::new(),
            toast: None,
            undo: None,
            gestures: HashMap::new(),
            overlay_open: false,
        }
    }

    #[cfg(debug_assertions)]
    pub fn prompt_host(&self) -> Option<String> {
        self.prompt.clone()
    }

    fn tab(&mut self, id: u32) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }
}

impl Tab {
    fn url(&self) -> String {
        self.entries.get(self.idx).map(|e| e.url.clone()).unwrap_or_default()
    }
}

fn shared(app: &AppHandle) -> &Shared {
    app.state::<Shared>().inner()
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

fn is_blank(url: &str) -> bool {
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
    tabs: Vec<TabView>,
    active: u32,
    cfg: &'a Config,
    prompt: Option<String>,
    toast: Option<Toast>,
    adblock_site: bool,
}

pub fn emit_state(app: &AppHandle) {
    let s = shared(app);
    let g = s.app.lock().unwrap();
    let active_host = g
        .tabs
        .iter()
        .find(|t| t.id == g.active)
        .and_then(|t| Url::parse(&t.url()).ok())
        .and_then(|u| u.host_str().map(String::from))
        .unwrap_or_default();
    let snap = Snapshot {
        tabs: g
            .tabs
            .iter()
            .map(|t| {
                let url = t.url();
                let parsed = Url::parse(&url).ok();
                TabView {
                    id: t.id,
                    title: if t.title.is_empty() { if is_blank(&url) { "New Tab".into() } else { url.clone() } } else { t.title.clone() },
                    host: parsed.as_ref().and_then(|u| u.host_str().map(String::from)).unwrap_or_default(),
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
        active: g.active,
        cfg: &g.cfg,
        prompt: g.prompt.clone(),
        toast: g.toast.clone(),
        adblock_site: !g.cfg.allowlist.contains(&active_host),
    };
    let _ = app.emit("state", &snap);
    drop(g);
    save_session(app);
}

fn save_cfg(app: &AppHandle) {
    let s = shared(app);
    let g = s.app.lock().unwrap();
    g.cfg.save(&s.cfg_path);
}

// ---------- layout ----------

pub fn content_rect(app: &AppHandle) -> (LogicalPosition<f64>, LogicalSize<f64>) {
    let (sidebar, top) = {
        let g = shared(app).app.lock().unwrap();
        (g.sidebar, g.top)
    };
    let (w, h) = app
        .get_window("main")
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
    let (pos, size) = content_rect(app);
    let (id, gen) = {
        let g = shared(app).app.lock().unwrap();
        match g.tabs.iter().find(|t| t.id == g.active) {
            Some(t) => (t.id, t.gen),
            None => return,
        }
    };
    if let Some(wv) = app.get_webview(&label(id, gen)) {
        let _ = wv.set_bounds(tauri::Rect { position: pos.into(), size: size.into() });
    }
    if let Some(wv) = app.get_webview("panel") {
        let _ = wv.set_bounds(tauri::Rect { position: pos.into(), size: size.into() });
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
            let active = g.active;
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
addEventListener('playing',()=>ping('p1'),true);
addEventListener('pause',()=>ping('p0'),true);
addEventListener('ended',()=>ping('p0'),true);
if(window.top===window){try{window.__TAURI_INTERNALS__.invoke('cosmetic_css').then(css=>{if(!css)return;const s=document.createElement('style');s.textContent=css;(document.head||document.documentElement).appendChild(s)}).catch(()=>{})}catch(e){}}
})();"#;

fn build_webview(app: &AppHandle, tab_id: u32, show: bool) -> Result<(), String> {
    let window = app.get_window("main").ok_or("no main window")?;
    let (profile_name, gen, url, cfg_profile) = {
        let g = shared(app).app.lock().unwrap();
        let t = g.tabs.iter().find(|t| t.id == tab_id).ok_or("no tab")?;
        let p = g.cfg.profile(&t.profile).or(g.cfg.profiles.first()).ok_or("no profile")?.clone();
        (t.profile.clone(), t.gen, t.url(), p)
    };
    let data_dir = config::Config::data_dir(&shared(app).data_dir, &cfg_profile);
    let _ = std::fs::create_dir_all(&data_dir);

    let wurl = if is_blank(&url) {
        WebviewUrl::App(NEWTAB.into())
    } else {
        WebviewUrl::External(Url::parse(&url).map_err(|e| e.to_string())?)
    };

    let (a_nav, a_title, a_load, a_new, a_dl) =
        (app.clone(), app.clone(), app.clone(), app.clone(), app.clone());

    let mut builder = WebviewBuilder::new(label(tab_id, gen), wurl)
        .initialization_script(PAGE_SCRIPT)
        .data_store_identifier(config::store_identifier(&cfg_profile))
        .data_directory(data_dir)
        .on_navigation(move |u| on_navigation(&a_nav, tab_id, gen, u))
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
            let profile = {
                let g = shared(&a_new).app.lock().unwrap();
                g.tabs.iter().find(|t| t.id == tab_id).map(|t| t.profile.clone())
            };
            let app = a_new.clone();
            tauri::async_runtime::spawn(async move {
                open_tab(&app, Some(u), profile, true);
            });
            NewWindowResponse::Deny
        })
        .on_download(move |_, ev| match ev {
            DownloadEvent::Requested { url, destination } => {
                if let Ok(dir) = a_dl.path().download_dir() {
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

    let (pos, size) = content_rect(app);
    let wv = window.add_child(builder, pos, size).map_err(|e| e.to_string())?;
    crate::adblock::attach(app, &wv, &profile_name);
    if !show {
        let _ = wv.hide();
    }
    Ok(())
}

fn on_navigation(app: &AppHandle, tab_id: u32, gen: u32, u: &Url) -> bool {
    if !matches!(u.scheme(), "http" | "https") {
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
        let profile = t.profile.clone();
        if cfg!(debug_assertions) {
            eprintln!("[nav] tab={tab_id} user={user} {u}");
        }
        let mut d = nav::decide(&g.cfg.mappings, &profile, &host, user);
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

fn on_page_load(app: &AppHandle, tab_id: u32, gen: u32, url: String, started: bool) {
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
            (!is_blank(&url)).then(|| (url, t.title.clone(), t.profile.clone()))
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
            s.app.lock().unwrap().gestures.insert(id, Instant::now());
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

fn parse_label(l: &str) -> Option<(u32, u32)> {
    let mut it = l.strip_prefix("tab-")?.split('-');
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

// ---------- tab operations ----------

fn hide_all_except(app: &AppHandle, keep: Option<(u32, u32)>) {
    let labels: Vec<String> = {
        let g = shared(app).app.lock().unwrap();
        g.tabs.iter().map(|t| label(t.id, t.gen)).collect()
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

pub fn open_tab(app: &AppHandle, url: Option<Url>, profile: Option<String>, activate_it: bool) -> u32 {
    let id = {
        let mut g = shared(app).app.lock().unwrap();
        let profile = profile
            .filter(|p| g.cfg.profile(p).is_some())
            .unwrap_or_else(|| {
                g.tabs
                    .iter()
                    .find(|t| t.id == g.active)
                    .map(|t| t.profile.clone())
                    .unwrap_or_else(|| g.cfg.profiles[0].name.clone())
            });
        let id = g.next_id;
        g.next_id += 1;
        let entries = url
            .map(|u| vec![Entry { url: u.to_string(), profile: profile.clone() }])
            .unwrap_or_default();
        g.tabs.push(Tab {
            id,
            profile,
            entries,
            idx: 0,
            title: String::new(),
            pinned: false,
            group: None,
            discarded: true,
            loading: false,
            audible: false,
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
    let (need_build, gen) = {
        let mut g = shared(app).app.lock().unwrap();
        if g.tabs.iter().all(|t| t.id != id) {
            return;
        }
        g.active = id;
        let t = g.tab(id).unwrap();
        t.last_active = Instant::now();
        let nb = t.discarded;
        t.discarded = false;
        if nb {
            t.loading = true;
        }
        (nb, t.gen)
    };
    hide_all_except(app, Some((id, gen)));
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
    let (lbl, next) = {
        let mut g = shared(app).app.lock().unwrap();
        let Some(pos) = g.tabs.iter().position(|t| t.id == id) else { return };
        let t = g.tabs.remove(pos);
        g.gestures.remove(&id);
        let next = if g.active == id {
            g.tabs.get(pos.min(g.tabs.len().saturating_sub(1))).map(|t| t.id)
        } else {
            None
        };
        (label(t.id, t.gen), next)
    };
    if let Some(wv) = app.get_webview(&lbl) {
        let _ = wv.close();
    }
    match next {
        Some(n) => activate(app, n),
        None => {
            let empty = shared(app).app.lock().unwrap().tabs.is_empty();
            if empty {
                open_tab(app, None, None, true);
            } else {
                emit_state(app);
            }
        }
    }
}

/// Replace the tab's webview with one in `profile`, loading `url`.
pub fn swap(app: &AppHandle, tab_id: u32, profile: &str, url: &str, announce: bool) {
    let (old_label, is_active) = {
        let mut g = shared(app).app.lock().unwrap();
        let active = g.active;
        let Some(t) = g.tab(tab_id) else { return };
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

pub fn reload(app: &AppHandle, tab_id: u32) {
    let lbl = {
        let mut g = shared(app).app.lock().unwrap();
        let Some(t) = g.tab(tab_id) else { return };
        t.mode = Mode::Traverse;
        label(t.id, t.gen)
    };
    if let Some(wv) = app.get_webview(&lbl) {
        let _ = wv.eval("location.reload()");
    }
}

pub fn address_bar(app: &AppHandle, tab_id: u32, input: &str) {
    let template = shared(app).app.lock().unwrap().cfg.settings.search_template.clone();
    navigate(app, tab_id, search::resolve(input, &template), true);
}

pub fn assign_mapping(app: &AppHandle, host: &str, profile: &str) {
    let to_swap = {
        let mut g = shared(app).app.lock().unwrap();
        g.prompt = None;
        g.cfg.mappings.retain(|m| m.pattern != host);
        g.cfg.mappings.push(nav::Mapping { pattern: host.into(), profile: profile.into() });
        let active = g.active;
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
        let active = g.active;
        g.tabs
            .iter_mut()
            .filter(|t| t.id != active && !t.pinned && !t.audible && !t.discarded && t.last_active.elapsed() > limit)
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
        let t = g.tabs.remove(from);
        let to = to.min(g.tabs.len());
        g.tabs.insert(to, t);
    }
    emit_state(app);
}

pub fn cycle(app: &AppHandle, delta: i32) {
    let next = {
        let g = shared(app).app.lock().unwrap();
        let Some(pos) = g.tabs.iter().position(|t| t.id == g.active) else { return };
        let n = g.tabs.len() as i32;
        g.tabs[((pos as i32 + delta).rem_euclid(n)) as usize].id
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

// ---------- session ----------

#[derive(Serialize, serde::Deserialize)]
struct Saved {
    url: String,
    profile: String,
    pinned: bool,
    group: Option<String>,
    title: String,
}

#[derive(Serialize, serde::Deserialize, Default)]
struct Session {
    tabs: Vec<Saved>,
    active: usize,
}

fn session_path(app: &AppHandle) -> PathBuf {
    shared(app).data_dir.join("session.json")
}

pub fn save_session(app: &AppHandle) {
    let session = {
        let g = shared(app).app.lock().unwrap();
        Session {
            active: g.tabs.iter().position(|t| t.id == g.active).unwrap_or(0),
            tabs: g
                .tabs
                .iter()
                .map(|t| Saved {
                    url: t.url(),
                    profile: t.profile.clone(),
                    pinned: t.pinned,
                    group: t.group.clone(),
                    title: t.title.clone(),
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
    if session.tabs.is_empty() {
        open_tab(app, None, None, true);
        return;
    }
    let mut ids = vec![];
    for s in session.tabs {
        let id = open_tab(app, Url::parse(&s.url).ok(), Some(s.profile), false);
        with_tab(app, id, |t| {
            t.pinned = s.pinned;
            t.group = s.group;
            t.title = s.title;
        });
        ids.push(id);
    }
    activate(app, ids[session.active.min(ids.len() - 1)]);
}
