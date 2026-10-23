pub mod adblock;
pub mod config;
pub mod db;
pub mod nav;
pub mod search;
pub mod tabs;
#[cfg(debug_assertions)]
mod selftest;

use config::{Config, Settings};
use db::{Db, DownloadRow, HistoryRow};
use std::{sync::Mutex, time::Duration};
use tabs::{App, Shared};
use tauri::{
    menu::{CheckMenuItem, ContextMenu, Menu, MenuItem, PredefinedMenuItem, Submenu},
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Webview, WebviewBuilder, WebviewUrl,
    WindowEvent,
};
use url::Url;

/// Selected tab of the window that issued the command.
fn active_id(app: &AppHandle, win: &tauri::Window) -> u32 {
    app.state::<Shared>().app.lock().unwrap().active(win.label())
}

fn mutate_cfg(app: &AppHandle, f: impl FnOnce(&mut Config)) {
    let s = app.state::<Shared>();
    {
        let mut g = s.app.lock().unwrap();
        f(&mut g.cfg);
        g.cfg.save(&s.cfg_path);
    }
    tabs::emit_state(app);
}

// ---------- commands ----------

#[tauri::command]
fn get_state(app: AppHandle) {
    tabs::emit_state(&app);
}

#[tauri::command(async)]
fn new_tab(app: AppHandle, window: tauri::Window, url: Option<String>) {
    tabs::open_tab_in(&app, window.label(), url.and_then(|u| Url::parse(&u).ok()), None, true);
}

#[tauri::command(async)]
fn close_tab(app: AppHandle, id: u32) {
    tabs::close_tab(&app, id);
}

#[tauri::command(async)]
fn activate_tab(app: AppHandle, id: u32) {
    tabs::activate(&app, id);
}

#[tauri::command(async)]
fn address(app: AppHandle, window: tauri::Window, input: String) {
    close_suggest(&app, window.label());
    tabs::address_bar(&app, active_id(&app, &window), &input);
}

#[tauri::command(async)]
fn back(app: AppHandle, window: tauri::Window) {
    tabs::go_history(&app, active_id(&app, &window), -1);
}

#[tauri::command(async)]
fn forward(app: AppHandle, window: tauri::Window) {
    tabs::go_history(&app, active_id(&app, &window), 1);
}

#[tauri::command(async)]
fn reload(app: AppHandle, window: tauri::Window) {
    tabs::reload(&app, active_id(&app, &window));
}

#[tauri::command(async)]
fn set_layout(app: AppHandle, window: tauri::Window, sidebar: f64, top: f64) {
    {
        let mut g = app.state::<Shared>().inner().app.lock().unwrap();
        let w = g.win(window.label());
        w.sidebar = sidebar;
        w.top = top;
    }
    tabs::apply_layout(&app);
}

#[tauri::command(async)]
fn assign_mapping(app: AppHandle, host: String, profile: String) {
    tabs::assign_mapping(&app, &host, &profile);
}

#[tauri::command(async)]
fn answer_permission(app: AppHandle, allow: bool) {
    tabs::answer_permission(&app, allow);
}

#[tauri::command(async)]
fn delete_permission(app: AppHandle, host: String, kind: String) {
    mutate_cfg(&app, |c| c.permissions.retain(|r| !(r.host == host && r.kind == kind)));
}

#[tauri::command(async)]
fn skip_prompt(app: AppHandle) {
    tabs::skip_prompt(&app);
}

#[tauri::command(async)]
fn undo_swap(app: AppHandle) {
    tabs::undo_swap(&app);
}

#[tauri::command(async)]
fn dismiss_toast(app: AppHandle) {
    tabs::dismiss_toast(&app);
}

#[tauri::command(async)]
fn save_settings(app: AppHandle, settings: Settings) {
    let (lists_changed, blocking_changed) = {
        let state = app.state::<Shared>();
        let g = state.app.lock().unwrap();
        let old = &g.cfg.settings;
        (old.disabled_lists != settings.disabled_lists, old.adblock != settings.adblock || old.disabled_lists != settings.disabled_lists)
    };
    let zoom = settings.page_zoom;
    let zoom_changed = app.state::<Shared>().app.lock().unwrap().cfg.settings.page_zoom != zoom;
    mutate_cfg(&app, |c| c.settings = settings);
    if zoom_changed {
        for (label, wv) in app.webviews() {
            if label.starts_with("tab-") {
                let _ = wv.set_zoom(zoom as f64 / 100.0);
            }
        }
    }
    if lists_changed {
        adblock::rebuild(&app);
    }
    // Only blocking changes need a page reload; theme, width etc. must not reload the tab.
    if blocking_changed {
        adblock::refresh(&app);
    }
}

#[tauri::command(async)]
fn set_profile_color(app: AppHandle, name: String, color: Option<String>) {
    let ok = |c: &str| c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|h| h.is_ascii_hexdigit());
    let color = color.filter(|c| ok(c));
    mutate_cfg(&app, |c| {
        if let Some(p) = c.profiles.iter_mut().find(|p| p.name == name) {
            p.color = color;
        }
    });
}

/// Opens the OS page where the default browser is chosen (apps can't set it silently).
#[tauri::command(async)]
fn default_browser_settings() {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("x-apple.systempreferences:com.apple.Desktop-Settings.extension").spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd").args(["/C", "start", "ms-settings:defaultapps"]).spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-settings").args(["set", "default-web-browser", "aditus.desktop"]).spawn();
}

#[tauri::command(async)]
fn adblock_info(app: AppHandle) -> adblock::Info {
    adblock::info(&app)
}

#[derive(serde::Serialize)]
struct UpdateInfo {
    current: String,
    latest: Option<String>,
    url: Option<String>,
    newer: bool,
}

/// Latest GitHub release vs this build; `latest: None` when none is published (or offline).
#[tauri::command(async)]
fn check_update() -> UpdateInfo {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let res = ureq::get("https://api.github.com/repos/LorenzoMarchisio05/Aditus/releases/latest")
        .set("User-Agent", "aditus")
        .timeout(std::time::Duration::from_secs(10))
        .call()
        .ok()
        .and_then(|r| r.into_string().ok())
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
    let tag = res.as_ref().and_then(|v| v["tag_name"].as_str()).map(|t| t.trim_start_matches('v').to_string());
    let url = res.as_ref().and_then(|v| v["html_url"].as_str()).map(String::from);
    let newer = tag.as_deref().is_some_and(|t| t != current);
    UpdateInfo { current, latest: tag, url, newer }
}

#[tauri::command(async)]
fn add_profile(app: AppHandle, name: String) {
    let name = name.trim().to_string();
    if name.is_empty() {
        return;
    }
    mutate_cfg(&app, |c| {
        if c.profile(&name).is_none() {
            c.profiles.push(config::Profile { name, id: uuid::Uuid::new_v4().to_string(), color: None });
        }
    });
}

#[tauri::command(async)]
fn delete_profile(app: AppHandle, name: String) {
    let in_use = app.state::<Shared>().app.lock().unwrap().tabs.iter().any(|t| t.profile == name);
    if in_use {
        return;
    }
    mutate_cfg(&app, |c| {
        if c.profiles.len() > 1 {
            c.profiles.retain(|p| p.name != name);
            c.mappings.retain(|m| m.profile != name);
        }
    });
}

#[tauri::command(async)]
fn delete_mapping(app: AppHandle, pattern: String) {
    mutate_cfg(&app, |c| c.mappings.retain(|m| m.pattern != pattern));
}

#[tauri::command(async)]
fn toggle_allowlist(app: AppHandle, host: String) {
    mutate_cfg(&app, |c| {
        if c.allowlist.contains(&host) {
            c.allowlist.retain(|h| h != &host);
        } else {
            c.allowlist.push(host);
        }
    });
    adblock::refresh(&app);
}

#[tauri::command(async)]
fn pin_tab(app: AppHandle, id: u32, pinned: bool) {
    tabs::with_tab(&app, id, |t| t.pinned = pinned);
}

#[tauri::command(async)]
fn group_tab(app: AppHandle, id: u32, group: Option<String>) {
    tabs::with_tab(&app, id, |t| t.group = group.filter(|g| !g.trim().is_empty()));
}

#[tauri::command(async)]
fn reorder_tab(app: AppHandle, id: u32, to: usize) {
    tabs::reorder(&app, id, to);
}

#[tauri::command(async)]
fn find(app: AppHandle, window: tauri::Window, text: String, forward: bool) {
    let id = active_id(&app, &window);
    if let Some(wv) = tabs::active_webview(&app, id) {
        let t = serde_json::to_string(&text).unwrap();
        let _ = wv.eval(format!("window.find({t}, false, {}, true)", !forward));
    }
}

#[tauri::command(async)]
fn find_clear(app: AppHandle, window: tauri::Window) {
    if let Some(wv) = tabs::active_webview(&app, active_id(&app, &window)) {
        let _ = wv.eval("getSelection()&&getSelection().removeAllRanges()");
    }
}

#[tauri::command(async)]
fn open_devtools(app: AppHandle, window: tauri::Window) {
    if let Some(wv) = tabs::active_webview(&app, active_id(&app, &window)) {
        tabs::open_inspector(&wv);
    }
}

#[tauri::command]
fn history_search(app: AppHandle, q: String) -> Vec<HistoryRow> {
    app.state::<Shared>().db.search_history(&q, 200)
}

#[tauri::command]
fn history_clear(app: AppHandle) {
    app.state::<Shared>().db.clear_history();
}

#[tauri::command]
fn downloads_list(app: AppHandle) -> Vec<DownloadRow> {
    app.state::<Shared>().db.downloads()
}

/// Opens a finished download with its default app. Only paths in the downloads table qualify.
#[tauri::command]
fn open_download(app: AppHandle, path: String) {
    if app.state::<Shared>().db.downloads().iter().any(|d| d.path == path && d.status == "done") {
        tabs::open_external(&path);
    }
}

#[tauri::command]
fn reveal_path(path: String) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").args(["-R", &path]).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer").args(["/select,", &path]).spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open")
        .arg(std::path::Path::new(&path).parent().unwrap_or(std::path::Path::new("/")))
        .spawn();
}

/// Where the palette sits: centred over the page area, a little below the top bar.
fn palette_rect(app: &AppHandle, window: &tauri::Window) -> (LogicalPosition<f64>, LogicalSize<f64>) {
    let (sidebar, top) = {
        let mut g = app.state::<Shared>().inner().app.lock().unwrap();
        let w = g.win(window.label());
        (w.sidebar, w.top)
    };
    let sf = window.scale_factor().unwrap_or(1.0);
    let w = window.inner_size().map(|s| s.width as f64 / sf).unwrap_or(1200.0);
    let pw = 640.0_f64.min(w - 40.0);
    (LogicalPosition::new(sidebar + ((w - sidebar - pw) / 2.0).max(0.0), top + 24.0), LogicalSize::new(pw, 72.0))
}

#[tauri::command(async)]
fn show_palette(app: AppHandle, window: tauri::Window) {
    let win = window.label().to_string();
    app.state::<Shared>().inner().app.lock().unwrap().win(&win).overlay_open = true;
    let (pos, size) = palette_rect(&app, &window);
    let label = tabs::wv_label("palette", &win);
    let wv = match app.get_webview(&label) {
        Some(wv) => {
            let _ = wv.set_bounds(tauri::Rect { position: pos.into(), size: size.into() });
            wv
        }
        None => match window.add_child(
            WebviewBuilder::new(&label, WebviewUrl::App("index.html?view=palette".into())),
            pos,
            size,
        ) {
            Ok(wv) => {
                tabs::no_inset(&wv);
                wv
            }
            Err(e) => return eprintln!("palette: {e}"),
        },
    };
    let _ = wv.show();
    let _ = wv.set_focus();
    let _ = app.emit_to(label, "palette-open", ());
}

#[tauri::command(async)]
fn show_panel(app: AppHandle, window: tauri::Window, view: String) {
    let win = window.label().to_string();
    if view != "newtab" {
        return tabs::open_internal(&app, &win, &view);
    }
    let label = tabs::wv_label("panel", &win);
    if let Some(wv) = app.get_webview(&label) {
        let _ = wv.close();
    }
    let (pos, size) = tabs::content_rect(&app, &win);
    let url = WebviewUrl::App(format!("index.html?view={view}").into());
    match window.add_child(WebviewBuilder::new(&label, url), pos, size) {
        Ok(wv) => {
            tabs::no_inset(&wv);
            let _ = wv.set_focus();
        }
        Err(e) => eprintln!("panel: {e}"),
    }
}

#[tauri::command(async)]
fn hide_panel(app: AppHandle, window: tauri::Window) {
    if let Some(wv) = app.get_webview(&tabs::wv_label("panel", window.label())) {
        let _ = wv.close();
    }
}

/// Native popup (draws above the page webviews, unlike an HTML menu); picks arrive as `menu` events `profile:<name>`.
#[tauri::command(async)]
fn profile_menu(app: AppHandle, window: tauri::Window) -> Result<(), String> {
    let (names, cur) = {
        let state = app.state::<Shared>();
        let g = state.app.lock().unwrap();
        let active = g.active(window.label());
        let cur = g.tabs.iter().find(|t| t.id == active).map(|t| t.profile.clone()).unwrap_or_default();
        (g.cfg.profiles.iter().map(|p| p.name.clone()).collect::<Vec<_>>(), cur)
    };
    let items = names
        .iter()
        .map(|n| CheckMenuItem::with_id(&app, format!("profile:{n}"), n, true, *n == cur, None::<&str>))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let refs: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> = items.iter().map(|i| i as _).collect();
    let menu = Menu::with_items(&app, &refs).map_err(|e| e.to_string())?;
    app.run_on_main_thread(move || {
        eprintln!("[profile_menu] popup -> {:?}", menu.popup(window));
    })
    .map_err(|e| e.to_string())
}

/// Native tab context menu (an HTML one would be clipped by the page webview). Picks arrive as
/// `menu` events `tabm:<action>:<tab id>[:<arg>]`, which the UI webview carries out.
#[tauri::command(async)]
fn tab_menu(app: AppHandle, window: tauri::Window, id: u32) -> Result<(), String> {
    use tauri::menu::{IsMenuItem, Submenu};
    let (tab, groups, profiles) = {
        let state = app.state::<Shared>();
        let g = state.app.lock().unwrap();
        let Some(t) = g.tabs.iter().find(|t| t.id == id) else { return Ok(()) };
        let url = t.url();
        let host = if tabs::is_blank(&url) || tabs::internal_view(&url).is_some() { false } else { url::Url::parse(&url).ok().is_some_and(|u| u.host_str().is_some()) };
        let tab = (t.pinned, t.group.clone(), t.profile.clone(), host && !t.private, host); // pin only real sites, not new-tab or aditus:// pages
        let mut groups: Vec<String> = g.tabs.iter().filter_map(|t| t.group.clone()).collect();
        groups.sort();
        groups.dedup();
        (tab, groups, g.cfg.profiles.iter().map(|p| p.name.clone()).collect::<Vec<_>>())
    };
    fn e<T>(r: tauri::Result<T>) -> Result<T, String> {
        r.map_err(|e| e.to_string())
    }
    let plain = |act: &str, text: &str| MenuItem::with_id(&app, format!("tabm:{act}:{id}"), text, true, None::<&str>);

    let reload = e(plain("reload", "Reload"))?;
    let pin = e(MenuItem::with_id(&app, format!("tabm:pin:{id}"), if tab.0 { "Unpin Tab" } else { "Pin Tab" }, tab.0 || tab.4, None::<&str>))?;
    let mut group_items: Vec<Box<dyn IsMenuItem<tauri::Wry>>> = vec![];
    for n in &groups {
        group_items.push(Box::new(e(CheckMenuItem::with_id(&app, format!("tabm:group:{id}:{n}"), n, true, tab.1.as_deref() == Some(n), None::<&str>))?));
    }
    group_items.push(Box::new(e(plain("newgroup", "New Group…"))?));
    if tab.1.is_some() {
        group_items.push(Box::new(e(plain("ungroup", "Remove from Group"))?));
    }
    let grefs: Vec<&dyn IsMenuItem<tauri::Wry>> = group_items.iter().map(|i| i.as_ref()).collect();
    let group_menu = e(Submenu::with_items(&app, "Add to Group", true, &grefs))?;
    let pitems = profiles
        .iter()
        .map(|n| e(CheckMenuItem::with_id(&app, format!("tabm:profile:{id}:{n}"), n, true, *n == tab.2, None::<&str>)))
        .collect::<Result<Vec<_>, _>>()?;
    let prefs: Vec<&dyn IsMenuItem<tauri::Wry>> = pitems.iter().map(|i| i as _).collect();
    let profile_menu = e(Submenu::with_items(&app, "Always Open This Site In", tab.3, &prefs))?;
    let sep1 = e(PredefinedMenuItem::separator(&app))?;
    let sep2 = e(PredefinedMenuItem::separator(&app))?;
    let close = e(plain("close", "Close Tab"))?;
    let others = e(plain("closeothers", "Close Other Tabs"))?;
    let menu = e(Menu::with_items(&app, &[&reload, &pin, &sep1, &group_menu, &profile_menu, &sep2, &close, &others]))?;
    app.run_on_main_thread(move || {
        let _ = menu.popup(window);
    })
    .map_err(|e| e.to_string())
}

struct PageCtx {
    tab: u32,
    link: Option<url::Url>,
    image: Option<url::Url>,
    selection: String,
}

static PAGE_CTX: Mutex<Option<PageCtx>> = Mutex::new(None);

/// Right-click inside a page. Pages call this with arbitrary args, so it only keeps http(s) URLs and
/// text, and the menu has fixed ids; nothing from the page reaches a path or a command line.
#[tauri::command(async)]
fn page_context(app: AppHandle, window: tauri::Window, webview: Webview, link: Option<String>, image: Option<String>, selection: String, editable: bool) -> Result<(), String> {
    use tauri::menu::PredefinedMenuItem as P;
    let web = |s: Option<String>| s.filter(|s| s.len() < 8192).and_then(|s| url::Url::parse(&s).ok()).filter(|u| matches!(u.scheme(), "http" | "https"));
    let Some((tab, _)) = tabs::parse_label(webview.label()) else { return Ok(()) };
    let (link, image) = (web(link), web(image));
    let selection: String = selection.chars().take(2000).collect();
    let (can_back, can_forward) = {
        let state = app.state::<Shared>();
        let g = state.app.lock().unwrap();
        let Some(t) = g.tabs.iter().find(|t| t.id == tab) else { return Ok(()) };
        (t.idx > 0, t.idx + 1 < t.entries.len())
    };
    let e = |r: tauri::Result<MenuItem<tauri::Wry>>| r.map_err(|e| e.to_string());
    let it = |id: &str, text: &str, on: bool| e(MenuItem::with_id(&app, format!("ctx:{id}"), text, on, None::<&str>));
    let sep = || PredefinedMenuItem::separator(&app).map_err(|e| e.to_string());
    let menu = Menu::new(&app).map_err(|e| e.to_string())?;
    let m = |r: tauri::Result<()>| r.map_err(|e| e.to_string());
    if editable {
        for p in [P::cut(&app, None), P::copy(&app, None), P::paste(&app, None), P::select_all(&app, None)] {
            m(menu.append(&p.map_err(|e| e.to_string())?))?;
        }
    } else {
        if link.is_some() {
            m(menu.append(&it("link-tab", "Open Link in New Tab", true)?))?;
            m(menu.append(&it("link-copy", "Copy Link Address", true)?))?;
            m(menu.append(&sep()?))?;
        }
        if image.is_some() {
            m(menu.append(&it("img-tab", "Open Image in New Tab", true)?))?;
            m(menu.append(&it("img-copy", "Copy Image Address", true)?))?;
            m(menu.append(&sep()?))?;
        }
        if !selection.trim().is_empty() {
            let short: String = selection.trim().chars().take(24).collect();
            m(menu.append(&it("sel-copy", "Copy", true)?))?;
            m(menu.append(&it("sel-search", &format!("Search for “{short}”"), true)?))?;
            m(menu.append(&sep()?))?;
        }
        m(menu.append(&it("back", "Back", can_back)?))?;
        m(menu.append(&it("forward", "Forward", can_forward)?))?;
        m(menu.append(&it("reload", "Reload", true)?))?;
    }
    *PAGE_CTX.lock().unwrap() = Some(PageCtx { tab, link, image, selection });
    app.run_on_main_thread(move || {
        let _ = menu.popup(window);
    })
    .map_err(|e| e.to_string())
}

fn page_ctx_action(app: &AppHandle, act: &str) {
    let Some(c) = PAGE_CTX.lock().unwrap().take() else { return };
    let copy = |s: &str| {
        if let Ok(mut cb) = arboard::Clipboard::new() {
            let _ = cb.set_text(s);
        }
    };
    let (win, profile, tpl) = {
        let state = app.state::<Shared>();
        let g = state.app.lock().unwrap();
        let Some(t) = g.tabs.iter().find(|t| t.id == c.tab) else { return };
        (t.win.clone(), t.profile.clone(), g.cfg.settings.search_template.clone())
    };
    match act {
        "link-tab" => drop(c.link.map(|u| tabs::open_tab_in(app, &win, Some(u), Some(profile), true))),
        "img-tab" => drop(c.image.map(|u| tabs::open_tab_in(app, &win, Some(u), Some(profile), true))),
        "link-copy" => c.link.iter().for_each(|u| copy(u.as_str())),
        "img-copy" => c.image.iter().for_each(|u| copy(u.as_str())),
        "sel-copy" => copy(&c.selection),
        "sel-search" => drop(tabs::open_tab_in(app, &win, Some(search::resolve(c.selection.trim(), &tpl)), Some(profile), true)),
        "back" => tabs::go_history(app, c.tab, -1),
        "forward" => tabs::go_history(app, c.tab, 1),
        "reload" => tabs::reload(app, c.tab),
        _ => {}
    }
}

#[tauri::command(async)]
fn switch_profile(app: AppHandle, window: tauri::Window, profile: String) {
    tabs::switch_profile(&app, active_id(&app, &window), &profile);
}

#[tauri::command(async)]
fn hide_palette(app: AppHandle, window: tauri::Window) {
    app.state::<Shared>().app.lock().unwrap().win(window.label()).overlay_open = false;
    if let Some(wv) = app.get_webview(&tabs::wv_label("palette", window.label())) {
        let _ = wv.hide();
    }
    close_suggest(&app, window.label());
}

// ---------- address suggestions ----------

/// Rows last pushed to each window's suggestion list, for a list that is still loading.
static SUGGEST: Mutex<Vec<(String, serde_json::Value)>> = Mutex::new(Vec::new());

fn close_suggest(app: &AppHandle, win: &str) {
    SUGGEST.lock().unwrap().retain(|(w, _)| w != win);
    if let Some(wv) = app.get_webview(&tabs::wv_label("suggest", win)) {
        let _ = wv.close();
    }
}

/// Shows the list under an address input; `x`, `y`, `w` are in window coordinates, or ignored for the palette.
#[tauri::command(async)]
fn suggest_show(app: AppHandle, window: tauri::Window, webview: Webview, payload: serde_json::Value, palette: bool, x: f64, y: f64, w: f64) {
    let win = window.label().to_string();
    let rows = payload["rows"].as_array().map_or(0, |r| r.len().min(8)) as f64;
    let (pos, width) = if palette {
        let (p, s) = palette_rect(&app, &window);
        (LogicalPosition::new(p.x + 12.0, p.y + s.height - 4.0), s.width - 24.0)
    } else {
        (LogicalPosition::new(x, y), w)
    };
    let size = LogicalSize::new(width.max(200.0), rows * 36.0 + 14.0);
    {
        let mut g = SUGGEST.lock().unwrap();
        g.retain(|(w, _)| *w != win);
        g.push((win.clone(), payload.clone()));
    }
    let label = tabs::wv_label("suggest", &win);
    match app.get_webview(&label) {
        Some(wv) => {
            let _ = wv.set_bounds(tauri::Rect { position: pos.into(), size: size.into() });
            let _ = app.emit_to(label, "suggest-rows", payload);
        }
        None => {
            let b = WebviewBuilder::new(&label, WebviewUrl::App("index.html?view=suggest".into()));
            if let Ok(wv) = window.add_child(b, pos, size) {
                tabs::no_inset(&wv);
                // the list never takes the keyboard from the input being typed in
                let _ = webview.set_focus();
            }
        }
    }
}

#[tauri::command]
fn suggest_hide(app: AppHandle, window: tauri::Window) {
    close_suggest(&app, window.label());
}

#[tauri::command]
fn suggest_get(window: tauri::Window) -> Option<serde_json::Value> {
    SUGGEST.lock().unwrap().iter().find(|(w, _)| w == window.label()).map(|(_, v)| v.clone())
}

/// Page webviews may call this: CSS hiding ads on the page's real (engine-reported) URL.
#[tauri::command]
fn cosmetic_css(app: AppHandle, webview: Webview) -> String {
    webview.url().ok().map(|u| adblock::cosmetic_css(&app, &u)).unwrap_or_default()
}

/// Page webviews may call only this. The payload is a hint; Rust re-derives everything.
#[tauri::command]
fn page_ping(app: AppHandle, webview: Webview, kind: String) {
    tabs::page_ping(&app, webview.label(), &kind);
}

// ---------- menu ----------

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let item = |id: &str, text: &str, acc: &str| MenuItem::with_id(app, id, text, true, Some(acc));
    let aditus = Submenu::with_items(
        app,
        "Aditus",
        true,
        &[
            &PredefinedMenuItem::about(app, None, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;
    let file = Submenu::with_items(
        app,
        "File",
        true,
        &[
            &item("new-tab", "New Tab", "CmdOrCtrl+T")?,
            &item("new-window", "New Window", "CmdOrCtrl+N")?,
            &item("new-private-window", "New Private Window", "CmdOrCtrl+Shift+N")?,
            &item("close-tab", "Close Tab", "CmdOrCtrl+W")?,
            &item("reopen-tab", "Reopen Closed Tab", "CmdOrCtrl+Shift+T")?,
            &item("focus-address", "Open Location", "CmdOrCtrl+L")?,
            &item("save-page", "Save Page As…", "CmdOrCtrl+S")?,
            &item("print", "Print…", "CmdOrCtrl+P")?,
            &item("find", "Find in Page", "CmdOrCtrl+F")?,
            &item("search-tabs", "Search Tabs", "CmdOrCtrl+Shift+A")?,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    let view = Submenu::with_items(
        app,
        "View",
        true,
        &[
            &item("reload", "Reload", "CmdOrCtrl+R")?,
            &item("zoom-in", "Zoom In", "CmdOrCtrl+=")?,
            &item("zoom-out", "Zoom Out", "CmdOrCtrl+-")?,
            &item("zoom-reset", "Actual Size", "CmdOrCtrl+0")?,
            &PredefinedMenuItem::separator(app)?,
            &item("toggle-sidebar", "Toggle Sidebar", "CmdOrCtrl+\\")?,
            &item("toggle-topbar", "Toggle Top Bar", "CmdOrCtrl+Shift+L")?,
            &PredefinedMenuItem::separator(app)?,
            &item("devtools", "Developer Tools", "CmdOrCtrl+Alt+I")?,
            &PredefinedMenuItem::fullscreen(app, None)?,
        ],
    )?;
    let go = Submenu::with_items(
        app,
        "Go",
        true,
        &[
            &item("back", "Back", "CmdOrCtrl+[")?,
            &item("forward", "Forward", "CmdOrCtrl+]")?,
            &PredefinedMenuItem::separator(app)?,
            &item("history", "History", "CmdOrCtrl+Y")?,
            &item("downloads", "Downloads", "CmdOrCtrl+Shift+J")?,
            &item("settings", "Settings", "CmdOrCtrl+,")?,
        ],
    )?;
    let digits: Vec<MenuItem<tauri::Wry>> = (1..=9)
        .map(|n| item(&format!("select-tab:{n}"), &if n == 9 { "Last Tab".to_string() } else { format!("Tab {n}") }, &format!("CmdOrCtrl+{n}")))
        .collect::<Result<_, _>>()?;
    let (next, prev, search) = (item("next-tab", "Next Tab", "Ctrl+Tab")?, item("prev-tab", "Previous Tab", "Ctrl+Shift+Tab")?, item("search-tabs", "Search Tabs", "CmdOrCtrl+Shift+A")?);
    let (pin, others, sep) = (item("pin-active", "Pin or Unpin Tab", "CmdOrCtrl+Alt+P")?, item("close-others", "Close Other Tabs", "CmdOrCtrl+Alt+W")?, PredefinedMenuItem::separator(app)?);
    let mut tab_items: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> = vec![&next, &prev, &search, &sep, &pin, &others, &sep];
    tab_items.extend(digits.iter().map(|d| d as &dyn tauri::menu::IsMenuItem<tauri::Wry>));
    let tabs_menu = Submenu::with_items(app, "Tabs", true, &tab_items)?;
    let window = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;
    #[cfg(target_os = "macos")]
    let _ = window.set_as_windows_menu_for_nsapp();
    let help = Submenu::with_items(
        app,
        "Help",
        true,
        &[&item("help-site", "Aditus on GitHub", "")?, &item("help-issue", "Report an Issue", "")?],
    )?;
    #[cfg(target_os = "macos")]
    let _ = help.set_as_help_menu_for_nsapp();
    Menu::with_items(app, &[&aditus, &file, &edit, &view, &go, &tabs_menu, &window, &help])
}

fn on_menu(app: &AppHandle, id: &str) {
    let app = app.clone();
    let id = id.to_string();
    tauri::async_runtime::spawn(async move {
        let (win, cur) = app.state::<Shared>().app.lock().unwrap().focused_active_with_win();
        let ui = tabs::wv_label("ui", &win);
        // The page webview usually holds keyboard focus; an input in the UI webview can't take it otherwise.
        if matches!(id.as_str(), "focus-address" | "new-tab" | "find" | "search-tabs") {
            if let Some(wv) = app.get_webview(&ui) {
                let _ = wv.set_focus();
            }
        }
        match id.as_str() {
            "new-tab" => {
                tabs::open_tab_in(&app, &win, None, None, true);
                let _ = app.emit_to(ui, "menu", "focus-address");
            }
            "new-window" => tabs::new_window(&app, false),
            "new-private-window" => tabs::new_window(&app, true),
            "close-tab" => tabs::close_tab(&app, cur),
            "reopen-tab" => tabs::reopen_closed(&app),
            "reload" => tabs::reload(&app, cur),
            "zoom-in" => tabs::zoom(&app, cur, 1),
            "zoom-out" => tabs::zoom(&app, cur, -1),
            "zoom-reset" => tabs::zoom(&app, cur, 0),
            "print" | "save-page" => tabs::page_action(&app, cur, &id),
            "back" => tabs::go_history(&app, cur, -1),
            "forward" => tabs::go_history(&app, cur, 1),
            "next-tab" => tabs::cycle(&app, 1),
            "prev-tab" => tabs::cycle(&app, -1),
            "devtools" => {
                if let Some(wv) = tabs::active_webview(&app, cur) {
                    tabs::open_inspector(&wv);
                }
            }
            "help-site" => tabs::open_external("https://github.com/LorenzoMarchisio05/Aditus"),
            "help-issue" => tabs::open_external("https://github.com/LorenzoMarchisio05/Aditus/issues/new"),
            other if other.starts_with("select-tab:") => tabs::select(&app, other[11..].parse().unwrap_or(1)),
            other if other.starts_with("ctx:") => page_ctx_action(&app, &other[4..]),
            other => {
                let _ = app.emit_to(ui, "menu", other);
            }
        }
    });
}

// ---------- entry ----------

/// A dev run is a bare binary, so macOS names the app menu after it ("aditus").
#[cfg(target_os = "macos")]
fn brand_name() {
    use objc2::{class, msg_send, runtime::AnyObject};
    use objc2_foundation::NSString;
    unsafe {
        let bundle: *mut AnyObject = msg_send![class!(NSBundle), mainBundle];
        let info: *mut AnyObject = msg_send![bundle, infoDictionary];
        let (name, key) = (NSString::from_str("Aditus"), NSString::from_str("CFBundleName"));
        let _: () = msg_send![info, setObject: &*name, forKey: &*key];
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "macos")]
    brand_name();
    let builder = tauri::Builder::default();
    // A second launch (a link opened from another app) hands its arguments to the running one.
    #[cfg(not(target_os = "macos"))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, _| tabs::open_urls(app, argv.iter().skip(1))));
    builder
        .invoke_handler(tauri::generate_handler![
            get_state, new_tab, close_tab, activate_tab, address, back, forward, reload, set_layout,
            assign_mapping, skip_prompt, undo_swap, dismiss_toast, save_settings, add_profile,
            delete_profile, delete_mapping, toggle_allowlist, pin_tab, group_tab, find, find_clear,
            open_devtools, history_search, history_clear, downloads_list, show_palette, hide_palette, suggest_show, suggest_hide, suggest_get,
            reveal_path, reorder_tab, page_ping, cosmetic_css, show_panel, hide_panel, switch_profile, profile_menu, tab_menu,
            adblock_info, check_update, default_browser_settings, set_profile_color, page_context, open_download, answer_permission, delete_permission
        ])
        .on_menu_event(|app, ev| on_menu(app, ev.id().as_ref()))
        .on_window_event(|win, ev| match ev {
            WindowEvent::Resized(_) => tabs::apply_layout(win.app_handle()),
            WindowEvent::Focused(true) => {
                if let Some(s) = win.app_handle().try_state::<Shared>() {
                    let mut g = s.app.lock().unwrap();
                    if g.wins.contains_key(win.label()) {
                        g.focused = win.label().to_string();
                    }
                }
            }
            WindowEvent::Destroyed => tabs::window_closed(win.app_handle(), win.label()),
            _ => {}
        })
        .setup(|app| {
            let dir = std::env::var_os("ADITUS_DATA_DIR")
                .filter(|_| cfg!(debug_assertions))
                .map(std::path::PathBuf::from)
                .map_or_else(|| app.path().app_data_dir(), Ok)?;
            let cfg_path = dir.join("config.toml");
            let cfg = Config::load(&cfg_path);
            cfg.save(&cfg_path);
            let db = Db::open(&dir.join("aditus.db"))?;
            app.manage(Shared { app: Mutex::new(App::new(cfg)), db, cfg_path, data_dir: dir });

            let handle = app.handle().clone();
            tabs::create_window(&handle, "main")?;
            app.set_menu(build_menu(&handle)?)?;

            adblock::init(&handle);
            {
                let state = handle.state::<Shared>();
                let days = state.app.lock().unwrap().cfg.settings.history_days;
                if days > 0 {
                    state.db.prune_history(days);
                }
            }
            tabs::restore_session(&handle);
            #[cfg(not(target_os = "macos"))]
            tabs::open_urls(&handle, std::env::args().skip(1));

            #[cfg(debug_assertions)]
            if let Some(port) = std::env::var("ADITUS_SELFTEST_PORT").ok().and_then(|p| p.parse().ok()) {
                let h = handle.clone();
                std::thread::spawn(move || selftest::run(&h, port));
            }

            let h = handle.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(1));
                tabs::no_inset_all(&h);
            });
            let h = handle.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_secs(30));
                tabs::discard_idle(&h);
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Aditus")
        .run(|app, ev| {
            match ev {
                tauri::RunEvent::Exit => {
                    let s = app.state::<Shared>();
                    if s.app.lock().unwrap().cfg.settings.clear_history_on_quit {
                        s.db.clear_history();
                    }
                }
                #[cfg(target_os = "macos")]
                tauri::RunEvent::Opened { urls } => tabs::open_urls(app, urls.iter().map(|u| u.as_str())),
                _ => {}
            }
        });
}
