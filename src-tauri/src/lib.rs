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
    window::WindowBuilder,
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Webview, WebviewBuilder, WebviewUrl,
    WindowEvent,
};
use url::Url;

fn active_id(app: &AppHandle) -> u32 {
    app.state::<Shared>().app.lock().unwrap().active
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
fn new_tab(app: AppHandle, url: Option<String>) {
    tabs::open_tab(&app, url.and_then(|u| Url::parse(&u).ok()), None, true);
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
fn address(app: AppHandle, input: String) {
    tabs::address_bar(&app, active_id(&app), &input);
}

#[tauri::command(async)]
fn back(app: AppHandle) {
    tabs::go_history(&app, active_id(&app), -1);
}

#[tauri::command(async)]
fn forward(app: AppHandle) {
    tabs::go_history(&app, active_id(&app), 1);
}

#[tauri::command(async)]
fn reload(app: AppHandle) {
    tabs::reload(&app, active_id(&app));
}

#[tauri::command(async)]
fn set_layout(app: AppHandle, sidebar: f64, top: f64) {
    {
        let mut g = app.state::<Shared>().inner().app.lock().unwrap();
        g.sidebar = sidebar;
        g.top = top;
    }
    tabs::apply_layout(&app);
}

#[tauri::command(async)]
fn assign_mapping(app: AppHandle, host: String, profile: String) {
    tabs::assign_mapping(&app, &host, &profile);
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
    mutate_cfg(&app, |c| c.settings = settings);
    if lists_changed {
        adblock::rebuild(&app);
    }
    // Only blocking changes need a page reload; theme, width etc. must not reload the tab.
    if blocking_changed {
        adblock::refresh(&app);
    }
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
            c.profiles.push(config::Profile { name, id: uuid::Uuid::new_v4().to_string() });
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
fn find(app: AppHandle, text: String, forward: bool) {
    let id = active_id(&app);
    if let Some(wv) = tabs::active_webview(&app, id) {
        let t = serde_json::to_string(&text).unwrap();
        let _ = wv.eval(format!("window.find({t}, false, {}, true)", !forward));
    }
}

#[tauri::command(async)]
fn find_clear(app: AppHandle) {
    if let Some(wv) = tabs::active_webview(&app, active_id(&app)) {
        let _ = wv.eval("getSelection()&&getSelection().removeAllRanges()");
    }
}

#[tauri::command(async)]
fn open_devtools(app: AppHandle) {
    if let Some(wv) = tabs::active_webview(&app, active_id(&app)) {
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

#[tauri::command(async)]
fn show_palette(app: AppHandle) {
    let Some(window) = app.get_window("main") else { return };
    let (sidebar, top) = {
        let mut g = app.state::<Shared>().inner().app.lock().unwrap();
        g.overlay_open = true;
        (g.sidebar, g.top)
    };
    let sf = window.scale_factor().unwrap_or(1.0);
    let w = window.inner_size().map(|s| s.width as f64 / sf).unwrap_or(1200.0);
    let pw = 640.0_f64.min(w - 40.0);
    let pos = LogicalPosition::new(sidebar + ((w - sidebar - pw) / 2.0).max(0.0), top + 24.0);
    let size = LogicalSize::new(pw, 72.0);
    let wv = match app.get_webview("palette") {
        Some(wv) => {
            let _ = wv.set_bounds(tauri::Rect { position: pos.into(), size: size.into() });
            wv
        }
        None => match window.add_child(
            WebviewBuilder::new("palette", WebviewUrl::App("index.html?view=palette".into())),
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
    let _ = app.emit_to("palette", "palette-open", ());
}

#[tauri::command(async)]
fn show_panel(app: AppHandle, view: String) {
    if view != "newtab" {
        return tabs::open_internal(&app, &view);
    }
    let Some(window) = app.get_window("main") else { return };
    if let Some(wv) = app.get_webview("panel") {
        let _ = wv.close();
    }
    let (pos, size) = tabs::content_rect(&app);
    let url = WebviewUrl::App(format!("index.html?view={view}").into());
    match window.add_child(WebviewBuilder::new("panel", url), pos, size) {
        Ok(wv) => {
            tabs::no_inset(&wv);
            let _ = wv.set_focus();
        }
        Err(e) => eprintln!("panel: {e}"),
    }
}

#[tauri::command(async)]
fn hide_panel(app: AppHandle) {
    if let Some(wv) = app.get_webview("panel") {
        let _ = wv.close();
    }
}

/// Native popup (draws above the page webviews, unlike an HTML menu); picks arrive as `menu` events `profile:<name>`.
#[tauri::command(async)]
fn profile_menu(app: AppHandle) -> Result<(), String> {
    let (names, cur) = {
        let state = app.state::<Shared>();
        let g = state.app.lock().unwrap();
        let cur = g.tabs.iter().find(|t| t.id == g.active).map(|t| t.profile.clone()).unwrap_or_default();
        (g.cfg.profiles.iter().map(|p| p.name.clone()).collect::<Vec<_>>(), cur)
    };
    let items = names
        .iter()
        .map(|n| CheckMenuItem::with_id(&app, format!("profile:{n}"), n, true, *n == cur, None::<&str>))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let refs: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> = items.iter().map(|i| i as _).collect();
    let menu = Menu::with_items(&app, &refs).map_err(|e| e.to_string())?;
    let window = app.get_window("main").ok_or("no window")?;
    app.run_on_main_thread(move || {
        eprintln!("[profile_menu] popup -> {:?}", menu.popup(window));
    })
    .map_err(|e| e.to_string())
}

#[tauri::command(async)]
fn switch_profile(app: AppHandle, profile: String) {
    tabs::switch_profile(&app, active_id(&app), &profile);
}

#[tauri::command(async)]
fn hide_palette(app: AppHandle) {
    app.state::<Shared>().app.lock().unwrap().overlay_open = false;
    if let Some(wv) = app.get_webview("palette") {
        let _ = wv.hide();
    }
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
            &item("close-tab", "Close Tab", "CmdOrCtrl+W")?,
            &item("reopen-tab", "Reopen Closed Tab", "CmdOrCtrl+Shift+T")?,
            &item("focus-address", "Open Location", "CmdOrCtrl+L")?,
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
            &item("back", "Back", "CmdOrCtrl+[")?,
            &item("forward", "Forward", "CmdOrCtrl+]")?,
            &item("toggle-sidebar", "Toggle Sidebar", "CmdOrCtrl+\\")?,
            &item("toggle-topbar", "Toggle Top Bar", "CmdOrCtrl+Shift+L")?,
            &item("next-tab", "Next Tab", "Ctrl+Tab")?,
            &item("prev-tab", "Previous Tab", "Ctrl+Shift+Tab")?,
            &item("devtools", "Developer Tools", "CmdOrCtrl+Alt+I")?,
            &item("history", "History", "CmdOrCtrl+Y")?,
            &item("downloads", "Downloads", "CmdOrCtrl+Shift+J")?,
            &item("settings", "Settings", "CmdOrCtrl+,")?,
        ],
    )?;
    Menu::with_items(app, &[&aditus, &file, &edit, &view])
}

fn on_menu(app: &AppHandle, id: &str) {
    let app = app.clone();
    let id = id.to_string();
    tauri::async_runtime::spawn(async move {
        let cur = active_id(&app);
        match id.as_str() {
            "new-tab" => {
                tabs::open_tab(&app, None, None, true);
                let _ = app.emit("menu", "focus-address");
            }
            "close-tab" => tabs::close_tab(&app, cur),
            "reopen-tab" => tabs::reopen_closed(&app),
            "reload" => tabs::reload(&app, cur),
            "back" => tabs::go_history(&app, cur, -1),
            "forward" => tabs::go_history(&app, cur, 1),
            "next-tab" => tabs::cycle(&app, 1),
            "prev-tab" => tabs::cycle(&app, -1),
            "devtools" => {
                if let Some(wv) = tabs::active_webview(&app, cur) {
                    tabs::open_inspector(&wv);
                }
            }
            other => {
                let _ = app.emit("menu", other);
            }
        }
    });
}

// ---------- entry ----------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            get_state, new_tab, close_tab, activate_tab, address, back, forward, reload, set_layout,
            assign_mapping, skip_prompt, undo_swap, dismiss_toast, save_settings, add_profile,
            delete_profile, delete_mapping, toggle_allowlist, pin_tab, group_tab, find, find_clear,
            open_devtools, history_search, history_clear, downloads_list, show_palette, hide_palette,
            reveal_path, reorder_tab, page_ping, cosmetic_css, show_panel, hide_panel, switch_profile, profile_menu,
            adblock_info, check_update
        ])
        .on_menu_event(|app, ev| on_menu(app, ev.id().as_ref()))
        .on_window_event(|win, ev| {
            if let WindowEvent::Resized(_) = ev {
                tabs::apply_layout(win.app_handle());
            }
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
            let wb = WindowBuilder::new(app, "main")
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
                WebviewBuilder::new("ui", WebviewUrl::App("index.html".into())).auto_resize(),
                LogicalPosition::new(0.0, 0.0),
                size,
            )?;
            tabs::no_inset(&ui);
            app.set_menu(build_menu(&handle)?)?;

            adblock::init(&handle);
            tabs::restore_session(&handle);

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
        .run(tauri::generate_context!())
        .expect("error while running Aditus");
}
