//! Debug-only end-to-end check, driven by a local test server (see ADITUS_SELFTEST_PORT).
//! Results go to stderr as `[selftest] ...` lines; the server logs what pages sent it.

use crate::{tabs, Shared};
use std::{thread::sleep, time::Duration};
use tauri::{AppHandle, Manager};
use url::Url;

fn wait(s: u64) {
    sleep(Duration::from_secs(s));
}

fn profile_of(app: &AppHandle, id: u32) -> String {
    let g = app.state::<Shared>().inner().app.lock().unwrap();
    g.tabs.iter().find(|t| t.id == id).map(|t| t.profile.clone()).unwrap_or_default()
}

fn url_of(app: &AppHandle, id: u32) -> String {
    let g = app.state::<Shared>().inner().app.lock().unwrap();
    g.tabs.iter().find(|t| t.id == id).and_then(|t| t.entries.get(t.idx)).map(|e| e.url.clone()).unwrap_or_default()
}

pub fn run(app: &AppHandle, port: u16) {
    wait(5);
    let base = format!("http://127.0.0.1:{port}");
    if std::env::var("ADITUS_SELFTEST_ONLY").as_deref() == Ok("back") {
        {
            let mut g = app.state::<Shared>().inner().app.lock().unwrap();
            g.cfg.profiles.push(crate::config::Profile { name: "Work".into(), id: uuid_str() });
            g.cfg.mappings.push(crate::nav::Mapping { pattern: "localhost".into(), profile: "Work".into() });
            g.cfg.mappings.push(crate::nav::Mapping { pattern: "127.0.0.1".into(), profile: "Default".into() });
        }
        let t = tabs::open_tab(app, Some(Url::parse(&format!("{base}/report?step=h1")).unwrap()), Some("Default".into()), true);
        wait(4);
        tabs::address_bar(app, t, &format!("http://localhost:{port}/report?step=h2"));
        wait(5);
        eprintln!("[selftest] back: after nav profile={} url={}", profile_of(app, t), url_of(app, t));
        tabs::go_history(app, t, -1);
        wait(5);
        eprintln!("[selftest] back: after back profile={} (want Default) url={}", profile_of(app, t), url_of(app, t));
        tabs::go_history(app, t, 1);
        wait(5);
        eprintln!("[selftest] back: after forward profile={} (want Work) url={}", profile_of(app, t), url_of(app, t));
        return;
    }
    if let Ok(n) = std::env::var("ADITUS_SELFTEST_MEM").map(|n| n.parse::<usize>().unwrap_or(10)) {
        let sites = ["https://en.wikipedia.org/wiki/Rust_(programming_language)", "https://en.wikipedia.org/wiki/WebKit", "https://news.ycombinator.com", "https://example.com", "https://en.wikipedia.org/wiki/Tauri", "https://en.wikipedia.org/wiki/Browser", "https://www.rust-lang.org", "https://en.wikipedia.org/wiki/SQLite", "https://en.wikipedia.org/wiki/Svelte", "https://en.wikipedia.org/wiki/Linux"];
        for i in 0..n {
            tabs::open_tab(app, Some(Url::parse(sites[i % sites.len()]).unwrap()), Some("Default".into()), true);
            wait(2);
        }
        wait(20);
        eprintln!("[selftest] mem ready tabs={n} pid={}", std::process::id());
        wait(40);
        return;
    }
    if std::env::var("ADITUS_SELFTEST_ONLY").as_deref() == Ok("ui2") {
        {
            let mut g = app.state::<Shared>().inner().app.lock().unwrap();
            g.cfg.mappings.clear();
        }
        let t = tabs::open_tab(app, Some(Url::parse(&format!("{base}/report?step=u1")).unwrap()), Some("Default".into()), true);
        wait(4);
        tabs::address_bar(app, t, &format!("http://localhost:{port}/report?step=u2"));
        wait(4);
        if let (Some(w), Some(win)) = (app.get_webview("ui"), app.get_window("main")) {
            eprintln!("[selftest] ui webview size={:?} window inner={:?} scale={:?}", w.size(), win.inner_size(), win.scale_factor());
        }
        if let Some(w) = app.get_webview("ui") {
            let _ = w.eval(format!("window.__TAURI_INTERNALS__.invoke('new_tab',{{url:'http://127.0.0.1:{port}/report?step=vp_'+innerWidth+'x'+innerHeight+'_'+document.querySelector('.side').getBoundingClientRect().height}})"));
        }
        wait(2);
        snapshot(app, "ui", "/tmp/aditus-snap-prompt.tiff");
        wait(2);
        tabs::open_tab(app, None, Some("Default".into()), true);
        wait(4);
        snapshot(app, "panel", "/tmp/aditus-snap-newtab.tiff");
        if let Some(w) = app.get_webview("ui") {
            let _ = w.eval("document.querySelector('.pcard')?.click()");
        }
        wait(1);
        snapshot(app, "ui", "/tmp/aditus-snap-picker.tiff");
        crate::hide_panel(app.clone());
        wait(1);
        let _ = tauri::Emitter::emit(app, "menu", "search-tabs");
        wait(1);
        if let Some(w) = app.get_webview("ui") {
            let _ = w.eval("const i=document.querySelector('.tabsearch'); if(i){i.value='new';i.dispatchEvent(new Event('input',{bubbles:true}))}");
        }
        wait(1);
        snapshot(app, "ui", "/tmp/aditus-snap-search.tiff");
        wait(2);
        {
            let mut g = app.state::<Shared>().inner().app.lock().unwrap();
            g.cfg.settings.sidebar_collapsed = true;
            g.cfg.settings.topbar_visible = false;
        }
        tabs::emit_state(app);
        wait(3);
        snapshot(app, "ui", "/tmp/aditus-snap-collapsed.tiff");
        wait(2);
        return;
    }
    if std::env::var("ADITUS_SELFTEST_ONLY").as_deref() == Ok("restore") {
        wait(8);
        let g = app.state::<Shared>().inner().app.lock().unwrap();
        for t in &g.tabs {
            eprintln!("[selftest] restored tab {} profile={} pinned={} group={:?} discarded={} url={}", t.id, t.profile, t.pinned, t.group, t.discarded, t.entries.get(t.idx).map(|e| e.url.as_str()).unwrap_or(""));
        }
        let active = g.active;
        drop(g);
        eprintln!("[selftest] restored active={active}");
        snapshot(app, "ui", "/tmp/aditus-snap-restored-ui.tiff");
        wait(2);
        return;
    }
    if std::env::var("ADITUS_SELFTEST_ONLY").as_deref() == Ok("download") {
        tabs::open_tab(app, Some(Url::parse(&format!("{base}/download")).unwrap()), Some("Default".into()), true);
        wait(6);
        let dl = app.state::<Shared>().inner().db.downloads();
        eprintln!("[selftest] downloads={:?}", dl.iter().map(|d| format!("{}:{}", d.path, d.status)).collect::<Vec<_>>());
        return;
    }
    let local = format!("http://localhost:{port}");
    {
        let mut g = app.state::<Shared>().inner().app.lock().unwrap();
        g.cfg.profiles.push(crate::config::Profile { name: "Work".into(), id: uuid_str() });
        g.cfg.mappings.push(crate::nav::Mapping { pattern: "localhost".into(), profile: "Work".into() });
        g.cfg.mappings.push(crate::nav::Mapping { pattern: "127.0.0.1".into(), profile: "Default".into() });
        g.cfg.settings.adblock = false;
    }
    let u = |s: &str| Url::parse(s).unwrap();

    // 1. isolation: cookie set in Default must not reach Work
    let t1 = tabs::open_tab(app, Some(u(&format!("{base}/set"))), Some("Default".into()), true);
    wait(4);
    let t2 = tabs::open_tab(app, Some(u(&format!("{base}/report?step=isolation_work_profile"))), Some("Work".into()), true);
    wait(4);
    tabs::activate(app, t1);
    tabs::address_bar(app, t1, &format!("{base}/report?step=same_profile_persists"));
    wait(4);

    // 2. page IPC lockdown
    tabs::open_tab(app, Some(u(&format!("{base}/ipc"))), Some("Default".into()), true);
    wait(5);

    // 3. address-bar swap: localhost is mapped to Work, tab is Default
    tabs::activate(app, t1);
    tabs::address_bar(app, t1, &format!("{local}/set"));
    wait(5);
    eprintln!("[selftest] address_bar_swap profile={} (want Work) url={}", profile_of(app, t1), url_of(app, t1));
    tabs::address_bar(app, t1, &format!("{local}/report?step=work_cookie_after_swap"));
    wait(4);
    tabs::undo_swap(app);
    wait(5);
    eprintln!("[selftest] undo profile={} (want Default) url={}", profile_of(app, t1), url_of(app, t1));

    // 4. link click on a page: localhost link inside a Default tab must swap to Work
    let t4 = tabs::open_tab(app, Some(u(&format!("{base}/link"))), Some("Default".into()), true);
    wait(7);
    eprintln!("[selftest] link_click_swap profile={} (want Work) url={}", profile_of(app, t4), url_of(app, t4));

    // 5. redirect must NOT swap: /redirect bounces 127.0.0.1 -> localhost
    let t5 = tabs::open_tab(app, Some(u(&format!("{base}/redirect"))), Some("Default".into()), true);
    wait(6);
    eprintln!("[selftest] redirect_no_swap profile={} (want Default) url={}", profile_of(app, t5), url_of(app, t5));

    // 6. discard + restore
    tabs::activate(app, t1);
    {
        let mut g = app.state::<Shared>().inner().app.lock().unwrap();
        g.cfg.settings.idle_minutes = 0;
        for t in g.tabs.iter_mut() {
            t.last_active = std::time::Instant::now() - Duration::from_secs(120);
        }
    }
    tabs::discard_idle(app);
    let discarded = app.state::<Shared>().inner().app.lock().unwrap().tabs.iter().filter(|t| t.discarded).count();
    eprintln!("[selftest] discarded_tabs={discarded} (want >0, active tab kept)");
    tabs::activate(app, t2);
    wait(4);
    eprintln!("[selftest] reactivated t2 profile={} url={}", profile_of(app, t2), url_of(app, t2));
    // 7. ad blocker: network block, cosmetic hide, per-site pause
    crate::adblock::install(app, &["||127.0.0.1:8765/ads/\n127.0.0.1##.sponsor\n".into()]);
    {
        app.state::<Shared>().inner().app.lock().unwrap().cfg.settings.adblock = true;
    }
    wait(5);
    let t7 = tabs::open_tab(app, Some(u(&format!("{base}/adpage"))), Some("Default".into()), true);
    wait(6);
    eprintln!("[selftest] adblock_on page loaded (server log shows whether /ads/banner.js was fetched)");
    app.state::<Shared>().inner().app.lock().unwrap().cfg.allowlist.push("127.0.0.1".into());
    crate::adblock::refresh(app);
    wait(6);
    eprintln!("[selftest] adblock_paused t7={t7} (server log should now show /ads/banner.js)");
    // 8. popups open as tabs in the opener's profile
    let before = tab_count(app);
    let t8 = tabs::open_tab(app, Some(u(&format!("{base}/popup"))), Some("Default".into()), true);
    wait(6);
    eprintln!("[selftest] popup tabs {} -> {} (want +2: opener and popup) opener_profile={}", before, tab_count(app), profile_of(app, t8));

    // 9. downloads + history
    tabs::open_tab(app, Some(u(&format!("{base}/download"))), Some("Default".into()), true);
    wait(6);
    let shared = app.state::<Shared>();
    let dl = shared.inner().db.downloads();
    eprintln!("[selftest] downloads={:?}", dl.iter().map(|d| format!("{}:{}", d.path, d.status)).collect::<Vec<_>>());
    let hist = shared.inner().db.search_history("127.0.0.1", 50);
    eprintln!("[selftest] history_rows={} first={:?}", hist.len(), hist.first().map(|h| (&h.url, &h.profile)));

    // 10. unmapped host prompts; answering stores a mapping
    {
        app.state::<Shared>().inner().app.lock().unwrap().cfg.mappings.retain(|m| m.pattern != "127.0.0.1");
    }
    let t10 = tabs::open_tab(app, None, Some("Default".into()), true);
    tabs::address_bar(app, t10, &format!("{base}/report?step=prompt_host"));
    wait(4);
    let prompt = app.state::<Shared>().inner().app.lock().unwrap().prompt_host();
    eprintln!("[selftest] prompt={prompt:?} (want Some(\"127.0.0.1\"))");
    tabs::assign_mapping(app, "127.0.0.1", "Work");
    wait(5);
    eprintln!("[selftest] after assign: profile={} (want Work) prompt={:?}", profile_of(app, t10), app.state::<Shared>().inner().app.lock().unwrap().prompt_host());

    // 11. pin, group, reorder
    tabs::with_tab(app, t10, |t| {
        t.pinned = true;
        t.group = Some("g1".into());
    });
    tabs::reorder(app, t10, 0);
    let order: Vec<u32> = app.state::<Shared>().inner().app.lock().unwrap().tabs.iter().map(|t| t.id).collect();
    eprintln!("[selftest] reorder first={} (want {t10}) ", order[0]);

    // 12. overlay webviews + devtools + find do not error
    crate::show_palette(app.clone());
    wait(3);
    eprintln!("[selftest] palette webview exists={}", app.get_webview("palette").is_some());
    snapshot(app, "palette", "/tmp/aditus-snap-palette.tiff");
    wait(2);
    crate::hide_palette(app.clone());
    crate::show_panel(app.clone(), "settings".into());
    wait(3);
    eprintln!("[selftest] panel webview exists={}", app.get_webview("panel").is_some());
    snapshot(app, "panel", "/tmp/aditus-snap-settings.tiff");
    crate::hide_panel(app.clone());
    crate::show_panel(app.clone(), "history".into());
    wait(3);
    snapshot(app, "panel", "/tmp/aditus-snap-history.tiff");
    wait(2);
    crate::hide_panel(app.clone());
    crate::show_panel(app.clone(), "downloads".into());
    wait(3);
    snapshot(app, "panel", "/tmp/aditus-snap-downloads.tiff");
    wait(2);
    snapshot(app, "ui", "/tmp/aditus-snap-ui.tiff");
    let active = app.state::<Shared>().inner().app.lock().unwrap().active;
    if let Some(wv) = tabs::active_webview(app, active) {
        snapshot(app, wv.label(), "/tmp/aditus-snap-page.tiff");
    }
    wait(4);
    crate::hide_panel(app.clone());
    eprintln!("[selftest] panels cycled");
    crate::find(app.clone(), "report".into(), true);
    eprintln!("[selftest] find ok");
    eprintln!("[selftest] done");
    if std::env::var_os("ADITUS_SELFTEST_EXIT").is_some() {
        app.exit(0);
        return;
    }
    crate::open_devtools(app.clone());
    wait(2);
}

#[cfg(target_os = "macos")]
fn snapshot(app: &AppHandle, label: &str, path: &str) {
    let Some(wv) = app.get_webview(label) else {
        return eprintln!("[selftest] snapshot {label}: no webview");
    };
    let (path, label) = (path.to_string(), label.to_string());
    let _ = wv.with_webview(move |pw| unsafe {
        let Some(view) = (pw.inner() as *const objc2_web_kit::WKWebView).as_ref() else { return };
        let block = block2::RcBlock::new(move |img: *mut objc2_app_kit::NSImage, _err: *mut objc2_foundation::NSError| {
            if let Some(data) = img.as_ref().and_then(|i| i.TIFFRepresentation()) {
                let _ = std::fs::write(&path, data.to_vec());
                eprintln!("[selftest] snapshot {label} saved {path}");
            } else {
                eprintln!("[selftest] snapshot {label} failed");
            }
        });
        view.takeSnapshotWithConfiguration_completionHandler(None, &block);
    });
}

#[cfg(not(target_os = "macos"))]
fn snapshot(_: &AppHandle, _: &str, _: &str) {}

fn tab_count(app: &AppHandle) -> usize {
    app.state::<Shared>().inner().app.lock().unwrap().tabs.len()
}

fn uuid_str() -> String {
    uuid::Uuid::new_v4().to_string()
}
