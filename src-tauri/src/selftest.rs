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
    eprintln!("[selftest] done");
}

fn uuid_str() -> String {
    uuid::Uuid::new_v4().to_string()
}
