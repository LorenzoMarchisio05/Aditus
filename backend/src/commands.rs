//! Every `cefQuery` command. Runs on the UI thread; slow ones (network) answer from a worker.

use crate::{
    passwords,
    adblock,
    config::{self, Config, Settings},
    engine::{self, Item, Kind, Spec},
    ipc::{Ctx, Reply},
    search, tabs,
    tabs::{shared, AppHandle},
};
use serde_json::{json, Value};
use std::{sync::Mutex, time::Duration};
use url::Url;

const APP: AppHandle = AppHandle;

fn str_arg(a: &Value, k: &str) -> String {
    a[k].as_str().unwrap_or_default().to_string()
}

fn num(a: &Value, k: &str) -> f64 {
    a[k].as_f64().unwrap_or(0.0)
}

fn active_id(win: &str) -> u32 {
    shared(&APP).app.lock().unwrap().active(win)
}

fn mutate_cfg(f: impl FnOnce(&mut Config)) {
    let s = shared(&APP);
    {
        let mut g = s.app.lock().unwrap();
        f(&mut g.cfg);
        g.cfg.save(&s.cfg_path);
    }
    tabs::emit_state(&APP);
}

fn view_url(view: &str, win: &str) -> String {
    format!("aditus://app/index.html?view={view}&win={win}")
}

pub fn dispatch(ctx: &Ctx, cmd: &str, a: Value, reply: Reply) {
    let win = ctx.win.as_str();
    let ok = |v: Value| reply.done(Ok(v));
    match cmd {
        "get_state" => {
            tabs::emit_state(&APP);
            ok(Value::Null)
        }
        "new_tab" => {
            let url = a["url"].as_str().and_then(|u| Url::parse(u).ok());
            let id = active_id(win);
            // From a blank tab, a link (top site, history row) loads in place instead of piling up tabs.
            let blank = shared(&APP).app.lock().unwrap().tabs.iter().any(|t| t.id == id && tabs::is_blank(&t.url()));
            match url {
                Some(u) if blank => tabs::navigate(&APP, id, u, true),
                u => {
                    tabs::open_tab_in(&APP, win, u, None, true);
                }
            }
            ok(Value::Null)
        }
        "reader" => {
            tabs::page_action(&APP, active_id(win), "reader");
            ok(Value::Null)
        }
        "mute_tab" => {
            tabs::toggle_mute(&APP, num(&a, "id") as u32);
            ok(Value::Null)
        }
        "prewake_tab" => {
            tabs::prewake(&APP, num(&a, "id") as u32);
            ok(Value::Null)
        }
        "close_tab" => {
            tabs::close_tab(&APP, num(&a, "id") as u32);
            ok(Value::Null)
        }
        "activate_tab" => {
            tabs::activate(&APP, num(&a, "id") as u32);
            ok(Value::Null)
        }
        "address" => {
            close_suggest(win);
            tabs::address_bar(&APP, active_id(win), &str_arg(&a, "input"));
            ok(Value::Null)
        }
        "back" => {
            tabs::go_history(&APP, active_id(win), -1);
            ok(Value::Null)
        }
        "forward" => {
            tabs::go_history(&APP, active_id(win), 1);
            ok(Value::Null)
        }
        "reload" => {
            tabs::reload(&APP, active_id(win));
            ok(Value::Null)
        }
        "set_layout" => {
            {
                let mut g = shared(&APP).app.lock().unwrap();
                let w = g.win(win);
                w.sidebar = num(&a, "sidebar");
                w.top = num(&a, "top");
            }
            tabs::apply_layout(&APP);
            ok(Value::Null)
        }
        "assign_mapping" => {
            tabs::assign_mapping(&APP, &str_arg(&a, "host"), &str_arg(&a, "profile"));
            ok(Value::Null)
        }
        "answer_permission" => {
            tabs::answer_permission(&APP, a["allow"].as_bool().unwrap_or(false));
            ok(Value::Null)
        }
        "delete_permission" => {
            let (host, kind) = (str_arg(&a, "host"), str_arg(&a, "kind"));
            mutate_cfg(|c| c.permissions.retain(|r| !(r.host == host && r.kind == kind)));
            ok(Value::Null)
        }
        "set_permission" => {
            let (host, kind, allow) = (str_arg(&a, "host"), str_arg(&a, "kind"), a["allow"].as_bool().unwrap_or(false));
            mutate_cfg(|c| c.permissions.iter_mut().filter(|r| r.host == host && r.kind == kind).for_each(|r| r.allow = allow));
            ok(Value::Null)
        }
        "clear_permissions" => {
            mutate_cfg(|c| c.permissions.clear());
            ok(Value::Null)
        }
        "skip_prompt" => {
            tabs::skip_prompt(&APP);
            ok(Value::Null)
        }
        "undo_swap" => {
            tabs::undo_swap(&APP);
            ok(Value::Null)
        }
        "dismiss_toast" => {
            tabs::dismiss_toast(&APP);
            ok(Value::Null)
        }
        "save_settings" => match serde_json::from_value::<Settings>(a["settings"].clone()) {
            Ok(settings) => {
                save_settings(settings);
                ok(Value::Null)
            }
            Err(e) => reply.done(Err(e.to_string())),
        },
        "set_profile_color" => {
            let name = str_arg(&a, "name");
            let valid = |c: &str| c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|h| h.is_ascii_hexdigit());
            let color = a["color"].as_str().filter(|c| valid(c)).map(String::from);
            mutate_cfg(|c| {
                if let Some(p) = c.profiles.iter_mut().find(|p| p.name == name) {
                    p.color = color;
                }
            });
            ok(Value::Null)
        }
        "default_browser_settings" => {
            default_browser_settings();
            ok(Value::Null)
        }
        "adblock_info" => ok(serde_json::to_value(adblock::info(&APP)).unwrap_or(Value::Null)),
        "install_update" => {
            // Re-checks GitHub itself; the UI only says "go".
            std::thread::spawn(move || match crate::update::install() {
                Ok(()) => {
                    reply.done(Ok(Value::Null));
                    engine::defer_from_thread(engine::quit);
                }
                Err(e) => reply.done(Err(e)),
            });
        }
        "check_update" => {
            std::thread::spawn(move || reply.done(Ok(crate::update::check())));
        }
        "search_suggest" => {
            let (on, template, private) = {
                let mut g = shared(&APP).app.lock().unwrap();
                let private = g.win(win).incognito;
                (g.cfg.settings.search_suggestions, g.cfg.settings.search_template.clone(), private)
            };
            let q = str_arg(&a, "q");
            std::thread::spawn(move || reply.done(Ok(json!(search_suggest(on, &template, private, q.trim())))));
        }
        "add_profile" => {
            let name = str_arg(&a, "name").trim().to_string();
            if !name.is_empty() {
                mutate_cfg(|c| {
                    if c.profile(&name).is_none() {
                        c.profiles.push(config::Profile { name, id: uuid::Uuid::new_v4().to_string(), color: None });
                    }
                });
            }
            ok(Value::Null)
        }
        "delete_profile" => {
            let name = str_arg(&a, "name");
            if !shared(&APP).app.lock().unwrap().tabs.iter().any(|t| t.profile == name) {
                let dir = shared(&APP).app.lock().unwrap().cfg.profile(&name).map(|p| config::Config::data_dir(&shared(&APP).data_dir.join("cef"), p));
                mutate_cfg(|c| {
                    if c.profiles.len() > 1 {
                        if let Some(d) = &dir {
                            let _ = std::fs::remove_dir_all(d);
                        }
                        c.profiles.retain(|p| p.name != name);
                        c.mappings.retain(|m| m.profile != name);
                    }
                });
            }
            ok(Value::Null)
        }
        "delete_mapping" => {
            let pattern = str_arg(&a, "pattern");
            mutate_cfg(|c| c.mappings.retain(|m| m.pattern != pattern));
            ok(Value::Null)
        }
        "toggle_allowlist" => {
            let host = str_arg(&a, "host");
            mutate_cfg(|c| {
                if c.allowlist.contains(&host) {
                    c.allowlist.retain(|h| h != &host);
                } else {
                    c.allowlist.push(host);
                }
            });
            adblock::refresh(&APP);
            ok(Value::Null)
        }
        "pin_tab" => {
            let pinned = a["pinned"].as_bool().unwrap_or(false);
            tabs::with_tab(&APP, num(&a, "id") as u32, |t| t.pinned = pinned);
            ok(Value::Null)
        }
        "group_tab" => {
            let group = a["group"].as_str().filter(|g| !g.trim().is_empty()).map(String::from);
            tabs::with_tab(&APP, num(&a, "id") as u32, |t| t.group = group);
            ok(Value::Null)
        }
        "reorder_tab" => {
            tabs::reorder(&APP, num(&a, "id") as u32, num(&a, "to") as usize);
            ok(Value::Null)
        }
        "find" => {
            if let Some(l) = tabs::active_view(&APP, active_id(win)) {
                engine::find(&l, &str_arg(&a, "text"), a["forward"].as_bool().unwrap_or(true));
            }
            ok(Value::Null)
        }
        "find_clear" => {
            if let Some(l) = tabs::active_view(&APP, active_id(win)) {
                engine::stop_find(&l);
            }
            ok(Value::Null)
        }
        "ext_list" => ok(Value::Array(crate::extensions::list())),
        "ext_stage" => {
            let input = str_arg(&a, "input");
            std::thread::spawn(move || reply.done(crate::extensions::stage(&input).map_err(|e| e.into())));
        }
        "ext_commit" => reply.done(crate::extensions::commit(&str_arg(&a, "id")).map(|_| Value::Null)),
        "ext_discard" => {
            crate::extensions::discard(&str_arg(&a, "id"));
            ok(Value::Null)
        }
        "ext_toggle" => {
            crate::extensions::set_enabled(&str_arg(&a, "id"), a["enabled"].as_bool().unwrap_or(true));
            ok(Value::Null)
        }
        "ext_remove" => {
            crate::extensions::remove(&str_arg(&a, "id"));
            ok(Value::Null)
        }
        "ext_open" => {
            if let Some(u) = crate::extensions::page_url(&str_arg(&a, "id"), &str_arg(&a, "kind")).and_then(|u| u.parse().ok()) {
                tabs::open_tab_in(&APP, win, Some(u), None, true);
            }
            ok(Value::Null)
        }
        "open_devtools" => {
            if let Some(l) = tabs::active_view(&APP, active_id(win)) {
                engine::devtools(&l);
            }
            ok(Value::Null)
        }
        "history_search" => ok(serde_json::to_value(shared(&APP).db.search_history(&str_arg(&a, "q"), 200)).unwrap_or(Value::Null)),
        "bookmark_toggle" => {
            tabs::toggle_bookmark(&APP, active_id(win));
            ok(Value::Null)
        }
        "bookmark_search" => ok(serde_json::to_value(shared(&APP).db.search_bookmarks(&str_arg(&a, "q"), 200)).unwrap_or(Value::Null)),
        "bookmark_remove" => {
            shared(&APP).db.remove_bookmark(&str_arg(&a, "url"));
            tabs::emit_state(&APP);
            ok(Value::Null)
        }
        "import_bookmarks" => {
            // Optional `path` (tests); default is Chrome's own file.
            let path = a["path"].as_str().map(Into::into).or_else(crate::import::chrome_path);
            let Some(text) = path.and_then(|p| std::fs::read_to_string(p).ok()) else {
                return reply.done(Err("No Chrome bookmarks file found".into()));
            };
            let list = crate::import::chrome_bookmarks(&text);
            for (u, t) in &list {
                shared(&APP).db.add_bookmark(u, t, "");
            }
            tabs::emit_state(&APP);
            ok(json!(list.len()))
        }
        "import_history" => {
            // Optional `path` (tests); default is the History file next to Chrome's Bookmarks.
            let path = a["path"].as_str().map(Into::into).or_else(|| crate::import::chrome_path().map(|p| p.with_file_name("History")));
            let Some(rows) = path.and_then(|p| crate::import::chrome_history(&p)) else {
                return reply.done(Err("No Chrome history found".into()));
            };
            shared(&APP).db.import_visits(&rows, "");
            ok(json!(rows.len()))
        }
        "clear_data" => {
            let s = shared(&APP);
            let flag = |k: &str| a[k].as_bool().unwrap_or(false);
            if flag("history") {
                s.db.clear_history_since(a["hours"].as_u64().unwrap_or(0) as u32);
            }
            if flag("downloads") {
                s.db.clear_downloads();
            }
            if flag("cookies") || flag("cache") {
                let only = a["profile"].as_str().filter(|p| !p.is_empty());
                let profiles = s.app.lock().unwrap().cfg.profiles.clone();
                for p in profiles.iter().filter(|p| only.is_none_or(|n| n == p.name)) {
                    let dir = config::Config::data_dir(&s.data_dir.join("cef"), p);
                    engine::clear_site_data(&format!("profile-{}", p.id), &dir, flag("cookies"), flag("cache"));
                }
            }
            tabs::emit_state(&APP);
            ok(Value::Null)
        }
        "history_clear" => {
            shared(&APP).db.clear_history();
            ok(Value::Null)
        }
        "downloads_list" => ok(serde_json::to_value(shared(&APP).db.downloads()).unwrap_or(Value::Null)),
        "open_download" => {
            // Only paths in the downloads table qualify.
            let path = str_arg(&a, "path");
            if shared(&APP).db.downloads().iter().any(|d| d.path == path && d.status == "done") {
                tabs::open_external(&path);
            }
            ok(Value::Null)
        }
        "reveal_path" => {
            reveal_path(&str_arg(&a, "path"));
            ok(Value::Null)
        }
        "show_palette" => {
            show_palette(ctx);
            ok(Value::Null)
        }
        "hide_palette" => {
            shared(&APP).app.lock().unwrap().win(win).overlay_open = false;
            engine::set_visible(&tabs::wv_label("palette", win), false);
            close_suggest(win);
            if let Some(l) = tabs::active_view(&APP, active_id(win)) {
                engine::focus(&l);
            }
            ok(Value::Null)
        }
        "show_panel" => {
            let view = str_arg(&a, "view");
            if view == "newtab" {
                let label = tabs::wv_label("panel", win);
                engine::close(&label);
                engine::create(overlay(&label, win, &view, tabs::content_rect(&APP, win)));
                engine::focus(&tabs::wv_label("ui", win)); // the address bar takes the keys on a new tab
            } else {
                tabs::open_internal(&APP, win, &view);
            }
            ok(Value::Null)
        }
        "hide_panel" => {
            engine::close(&tabs::wv_label("panel", win));
            ok(Value::Null)
        }
        "profile_menu" => {
            profile_menu(win, at(&a, win));
            ok(Value::Null)
        }
        "tab_menu" => {
            tab_menu(win, num(&a, "id") as u32, at(&a, win));
            ok(Value::Null)
        }
        "switch_profile" => {
            tabs::switch_profile(&APP, active_id(win), &str_arg(&a, "profile"));
            ok(Value::Null)
        }
        "suggest_show" => {
            suggest_show(ctx, &a);
            ok(Value::Null)
        }
        "suggest_hide" => {
            close_suggest(win);
            ok(Value::Null)
        }
        "suggest_get" => ok(SUGGEST.lock().unwrap().iter().find(|(w, _)| w == win).map(|(_, v)| v.clone()).unwrap_or(Value::Null)),
        "cosmetic_css" => {
            let css = Url::parse(&ctx.frame_url).map(|u| adblock::cosmetic_css(&u)).unwrap_or_default();
            ok(Value::String(css))
        }
        "cosmetic_generic" => {
            // Untrusted input: cap the batch and keep only strings.
            let list = |k: &str| a[k].as_array().map(|v| v.iter().filter_map(|x| x.as_str().map(String::from)).take(2000).collect::<Vec<_>>()).unwrap_or_default();
            let css = Url::parse(&ctx.frame_url).map(|u| adblock::generic_css(&u, &list("c"), &list("i"))).unwrap_or_default();
            ok(Value::String(css))
        }
        "page_login" => {
            // Origin is the browser's view of the calling frame; the payload only carries what was typed.
            let (user, pw) = (str_arg(&a, "u"), str_arg(&a, "p"));
            let origin = passwords::origin_of(&ctx.frame_url);
            let skip = pw.is_empty() || pw.len() > 1024 || user.len() > 512 || tabs::label_private(&APP, &ctx.label);
            if let (Some(origin), false) = (origin, skip) {
                if passwords::get(passwords::SERVICE, &origin, &user).as_deref() != Some(pw.as_str()) {
                    let ev = json!({"label": ctx.label, "origin": origin, "username": user}).to_string();
                    passwords::offer(&ctx.label, origin, user, pw);
                    engine::emit_to(&tabs::wv_label("ui", &ctx.win), "login-offer", &ev);
                }
            }
            ok(Value::Null)
        }
        "login_save" => match passwords::take(&str_arg(&a, "label")) {
            Some(o) => match passwords::save(passwords::SERVICE, &o.origin, &o.user, &o.pw) {
                Ok(()) => {
                    shared(&APP).db.add_login(&o.origin, &o.user);
                    tabs::emit_state(&APP);
                    ok(Value::Null)
                }
                Err(e) => reply.done(Err(e)),
            },
            None => reply.done(Err("offer expired".into())),
        },
        "login_dismiss" => {
            passwords::take(&str_arg(&a, "label"));
            ok(Value::Null)
        }
        "login_list" => ok(json!(shared(&APP).db.logins(None))),
        "login_remove" => {
            let (o, u) = (str_arg(&a, "origin"), str_arg(&a, "username"));
            passwords::delete(passwords::SERVICE, &o, &u);
            shared(&APP).db.remove_login(&o, &u);
            tabs::emit_state(&APP);
            ok(Value::Null)
        }
        "login_fill" => {
            tabs::fill_login(&APP, active_id(win), Some(str_arg(&a, "username")).filter(|u| !u.is_empty()));
            ok(Value::Null)
        }
        "import_passwords_csv" => {
            // Keychain writes are slow (one per row): keep them off the UI thread.
            let text = str_arg(&a, "text");
            std::thread::spawn(move || {
                let mut n = 0;
                for (origin, user, pw) in crate::import::chrome_passwords_csv(&text) {
                    if passwords::save(passwords::SERVICE, &origin, &user, &pw).is_ok() {
                        shared(&APP).db.add_login(&origin, &user);
                        n += 1;
                    }
                }
                engine::defer_from_thread(|| tabs::emit_state(&APP));
                reply.done(Ok(json!(n)));
            });
        }
        "page_state" => {
            tabs::page_state(&APP, &ctx.label, &str_arg(&a, "s"));
            ok(Value::Null)
        }
        "address_selection" => {
            shared(&APP).app.lock().unwrap().win(win).address_sel = str_arg(&a, "text");            ok(Value::Null)
        }
        "page_link" => {
            tabs::page_link(&APP, &ctx.label, &str_arg(&a, "u"));
            ok(Value::Null)
        }
        "page_ping" => {
            tabs::page_ping(&APP, &ctx.label, &str_arg(&a, "kind"), &str_arg(&a, "href"), num(&a, "x"), num(&a, "y"));
            ok(Value::Null)
        }
        _ => reply.done(Err(format!("unknown command {cmd}"))),
    }
}

/// Menu position sent by the UI (`_x`, `_y` in window coordinates).
fn at(a: &Value, win: &str) -> (i32, i32) {
    let (sidebar, top) = {
        let mut g = shared(&APP).app.lock().unwrap();
        let w = g.win(win);
        (w.sidebar, w.top)
    };
    (a["_x"].as_f64().unwrap_or(sidebar) as i32, a["_y"].as_f64().unwrap_or(top) as i32)
}

fn overlay(label: &str, win: &str, view: &str, bounds: engine::Bounds) -> Spec {
    Spec {
        label: label.to_string(),
        win: win.to_string(),
        url: view_url(view, win),
        kind: Kind::Overlay,
        app_page: true,
        store: None,
        bounds,
        visible: true,
        zoom: 100,
        yt: false,
    }
}

fn save_settings(settings: Settings) {
    let (lists_changed, blocking_changed, zoom_changed, dock_changed) = {
        let g = shared(&APP).app.lock().unwrap();
        let old = &g.cfg.settings;
        (old.disabled_lists != settings.disabled_lists, old.adblock != settings.adblock || old.disabled_lists != settings.disabled_lists, old.page_zoom != settings.page_zoom, old.devtools_dock != settings.devtools_dock)
    };
    let zoom = settings.page_zoom;
    mutate_cfg(|c| c.settings = settings);
    if dock_changed {
        tabs::apply_layout(&APP);
    }
    if zoom_changed {
        let ids: Vec<u32> = shared(&APP).app.lock().unwrap().tabs.iter().filter(|t| t.zoom.is_none()).map(|t| t.id).collect();
        for id in ids {
            if let Some(l) = tabs::active_view(&APP, id) {
                engine::set_zoom(&l, zoom);
            }
        }
    }
    // Only blocking changes need a page reload; theme, width etc. must not reload the tab.
    // Building the engine takes seconds: off the UI thread, and the reload waits for it.
    if lists_changed {
        std::thread::spawn(|| {
            adblock::rebuild(&APP);
            engine::defer_from_thread(|| adblock::refresh(&APP));
        });
    } else if blocking_changed {
        adblock::refresh(&APP);
    }
}

/// Opens the OS page where the default browser is chosen (apps can't set it silently).
fn default_browser_settings() {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("x-apple.systempreferences:com.apple.Desktop-Settings.extension").spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd").args(["/C", "start", "ms-settings:defaultapps"]).spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-settings").args(["set", "default-web-browser", "aditus.desktop"]).spawn();
}

/// Search-engine completions for what is being typed. Nothing is sent from private windows or when switched off.
fn search_suggest(on: bool, template: &str, private: bool, q: &str) -> Vec<String> {
    let Some(url) = (on && !private && !q.is_empty()).then(|| search::suggest_url(template, q)).flatten() else { return vec![] };
    ureq::get(&url)
        .set("User-Agent", "aditus")
        .timeout(Duration::from_secs(3))
        .call()
        .ok()
        .and_then(|r| r.into_string().ok())
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v[1].as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).take(6).collect()))
        .unwrap_or_default()
}

fn reveal_path(path: &str) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").args(["-R", path]).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer").args(["/select,", path]).spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(std::path::Path::new(path).parent().unwrap_or(std::path::Path::new("/"))).spawn();
}

// ---------- palette and suggestions ----------

/// Where the palette sits: centred over the page area, a little below the top bar.
fn palette_rect(win: &str) -> engine::Bounds {
    let (sidebar, top) = {
        let mut g = shared(&APP).app.lock().unwrap();
        let w = g.win(win);
        (w.sidebar, w.top)
    };
    let w = engine::win_size(win).0;
    let pw = 640.0_f64.min(w - 40.0);
    (sidebar + ((w - sidebar - pw) / 2.0).max(0.0), top + 24.0, pw, 72.0)
}

fn show_palette(ctx: &Ctx) {
    let win = ctx.win.as_str();
    shared(&APP).app.lock().unwrap().win(win).overlay_open = true;
    let label = tabs::wv_label("palette", win);
    let bounds = palette_rect(win);
    if engine::has_view(&label) {
        engine::set_bounds(&label, bounds);
        engine::set_visible(&label, true);
        engine::focus(&label);
        engine::emit_to(&label, "palette-open", "null");
    } else {
        engine::create(overlay(&label, win, "palette", bounds));
        // The page is still loading: tell it to open once it can listen.
        let l = label.clone();
        engine::defer_after(250, move || {
            engine::focus(&l);
            engine::emit_to(&l, "palette-open", "null");
        });
    }
}

/// Rows last pushed to each window's suggestion list, for a list that is still loading.
static SUGGEST: Mutex<Vec<(String, Value)>> = Mutex::new(Vec::new());

fn close_suggest(win: &str) {
    SUGGEST.lock().unwrap().retain(|(w, _)| w != win);
    engine::close(&tabs::wv_label("suggest", win));
}

/// Shows the list under an address input; `x`, `y`, `w` are in window coordinates, or ignored for the palette.
fn suggest_show(ctx: &Ctx, a: &Value) {
    let win = ctx.win.as_str();
    let payload = a["payload"].clone();
    let rows = payload["rows"].as_array().map_or(0, |r| r.len().min(8)) as f64;
    let (x, y, w) = if a["palette"].as_bool().unwrap_or(false) {
        let p = palette_rect(win);
        (p.0 + 12.0, p.1 + p.3 - 4.0, p.2 - 24.0)
    } else {
        (num(a, "x"), num(a, "y"), num(a, "w"))
    };
    let bounds = (x, y, w.max(200.0), rows * 38.0 + 16.0);
    {
        let mut g = SUGGEST.lock().unwrap();
        g.retain(|(w, _)| w != win);
        g.push((win.to_string(), payload.clone()));
    }
    let label = tabs::wv_label("suggest", win);
    if engine::has_view(&label) {
        engine::set_bounds(&label, bounds);
        engine::emit_to(&label, "suggest-rows", &payload.to_string());
    } else {
        engine::create(overlay(&label, win, "suggest", bounds));
        // The list never takes the keyboard from the input being typed in.
        let from = ctx.label.clone();
        let again = from.clone();
        engine::defer_after(60, move || engine::focus(&from));
        engine::defer_after(400, move || engine::focus(&again));
    }
}

// ---------- native menus ----------

fn profile_menu(win: &str, pos: (i32, i32)) {
    let (names, cur) = {
        let g = shared(&APP).app.lock().unwrap();
        let active = g.active(win);
        let cur = g.tabs.iter().find(|t| t.id == active).map(|t| t.profile.clone()).unwrap_or_default();
        (g.cfg.profiles.iter().map(|p| p.name.clone()).collect::<Vec<_>>(), cur)
    };
    let items = names.into_iter().map(|n| Item::Entry { id: format!("profile:{n}"), checked: n == cur, label: n, enabled: true }).collect();
    engine::popup(win, items, pos);
}

fn tab_menu(win: &str, id: u32, pos: (i32, i32)) {
    let (pinned, group, profile, can_map, is_site, groups, profiles) = {
        let g = shared(&APP).app.lock().unwrap();
        let Some(t) = g.tabs.iter().find(|t| t.id == id) else { return };
        let url = t.url();
        let site = !tabs::is_blank(&url) && tabs::internal_view(&url).is_none() && Url::parse(&url).ok().is_some_and(|u| u.host_str().is_some());
        let mut groups: Vec<String> = g.tabs.iter().filter_map(|t| t.group.clone()).collect();
        groups.sort();
        groups.dedup();
        (t.pinned, t.group.clone(), t.profile.clone(), site && !t.private, site, groups, g.cfg.profiles.iter().map(|p| p.name.clone()).collect::<Vec<_>>())
    };
    let plain = |act: &str, label: &str| Item::Entry { id: format!("tabm:{act}:{id}"), label: label.into(), enabled: true, checked: false };
    let mut group_items: Vec<Item> = groups
        .iter()
        .map(|n| Item::Entry { id: format!("tabm:group:{id}:{n}"), label: n.clone(), enabled: true, checked: group.as_deref() == Some(n) })
        .collect();
    group_items.push(plain("newgroup", "New Group…"));
    if group.is_some() {
        group_items.push(plain("ungroup", "Remove from Group"));
    }
    let profile_items = profiles
        .iter()
        .map(|n| Item::Entry { id: format!("tabm:profile:{id}:{n}"), label: n.clone(), enabled: true, checked: *n == profile })
        .collect();
    let items = vec![
        plain("reload", "Reload"),
        Item::Entry { id: format!("tabm:pin:{id}"), label: if pinned { "Unpin Tab" } else { "Pin Tab" }.into(), enabled: pinned || is_site, checked: false },
        Item::Sep,
        Item::Sub { label: "Add to Group".into(), enabled: true, items: group_items },
        Item::Sub { label: "Always Open This Site In".into(), enabled: can_map, items: profile_items },
        Item::Sep,
        plain("close", "Close Tab"),
        plain("closeothers", "Close Other Tabs"),
    ];
    engine::popup(win, items, pos);
}

// ---------- page context menu ----------

struct PageCtx {
    tab: u32,
    link: Option<Url>,
    image: Option<Url>,
    selection: String,
}

static PAGE_CTX: Mutex<Option<PageCtx>> = Mutex::new(None);

/// Called by the engine when a page's context menu opens: URLs are already filtered to http(s).
pub fn remember_page_context(label: &str, link: Option<String>, image: Option<String>, selection: String) {
    let Some((tab, _)) = tabs::parse_label(label) else { return };
    *PAGE_CTX.lock().unwrap() = Some(PageCtx {
        tab,
        link: link.and_then(|u| Url::parse(&u).ok()),
        image: image.and_then(|u| Url::parse(&u).ok()),
        selection: selection.chars().take(2000).collect(),
    });
}

pub fn page_context_action(_label: &str, id: i32) {
    let Some(c) = PAGE_CTX.lock().unwrap().take() else { return };
    let copy = |s: &str| {
        if let Ok(mut cb) = arboard::Clipboard::new() {
            let _ = cb.set_text(s);
        }
    };
    let (win, profile, tpl) = {
        let g = shared(&APP).app.lock().unwrap();
        let Some(t) = g.tabs.iter().find(|t| t.id == c.tab) else { return };
        (t.win.clone(), t.profile.clone(), g.cfg.settings.search_template.clone())
    };
    match id {
        engine::CTX_LINK_TAB => drop(c.link.map(|u| tabs::open_tab_in(&APP, &win, Some(u), Some(profile), true))),
        engine::CTX_IMG_TAB => drop(c.image.map(|u| tabs::open_tab_in(&APP, &win, Some(u), Some(profile), true))),
        engine::CTX_LINK_WIN => drop(c.link.map(|u| tabs::new_window(&APP, false, Some(u)))),
        engine::CTX_LINK_PRIV => drop(c.link.map(|u| tabs::new_window(&APP, true, Some(u)))),
        engine::CTX_IMG_SAVE => {
            if let (Some(u), Some(l)) = (&c.image, tabs::active_view(&APP, c.tab)) {
                engine::download(&l, u.as_str());
            }
        }
        engine::CTX_PIP => {
            if let Some(l) = tabs::active_view(&APP, c.tab) {
                engine::eval(&l, "(()=>{const v=[...document.querySelectorAll('video')].find(v=>!v.paused)||document.querySelector('video');if(v)v.requestPictureInPicture().catch(()=>{})})()");
            }
        }
        engine::CTX_PRINT => tabs::page_action(&APP, c.tab, "print"),
        engine::CTX_SOURCE => {
            if let Some(u) = tabs::active_url(&APP, c.tab).and_then(|u| Url::parse(&format!("view-source:{u}")).ok()) {
                tabs::open_tab_in(&APP, &win, Some(u), Some(profile), true);
            }
        }
        engine::CTX_LINK_COPY => c.link.iter().for_each(|u| copy(u.as_str())),
        engine::CTX_IMG_COPY => c.image.iter().for_each(|u| copy(u.as_str())),
        engine::CTX_SEL_COPY => copy(&c.selection),
        engine::CTX_SEL_SEARCH => drop(tabs::open_tab_in(&APP, &win, Some(search::resolve(c.selection.trim(), &tpl)), Some(profile), true)),
        engine::CTX_BACK => tabs::go_history(&APP, c.tab, -1),
        engine::CTX_FORWARD => tabs::go_history(&APP, c.tab, 1),
        engine::CTX_RELOAD => tabs::reload(&APP, c.tab),
        engine::CTX_DEVTOOLS => {
            if let Some(l) = tabs::active_view(&APP, c.tab) {
                engine::devtools(&l);
            }
        }
        _ => {}
    }
}
