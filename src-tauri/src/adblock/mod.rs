//! Ad blocking: one `adblock-rust` engine, three platform backends.
//!
//! - cosmetic filtering: all platforms, CSS fetched by the page's init script via `cosmetic_css`
//! - network blocking: macOS (WKContentRuleList), Linux (WebKitGTK content filters),
//!   Windows (WebView2 request interception)

use crate::tabs::Shared;
use adblock::{
    lists::{FilterSet, ParseOptions},
    request::Request,
    Engine,
};
use std::{
    path::PathBuf,
    sync::{Arc, RwLock},
    time::{Duration, SystemTime},
};
use tauri::{AppHandle, Manager, Webview};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
use linux as native;
#[cfg(target_os = "macos")]
use macos as native;
#[cfg(windows)]
use windows as native;

const LISTS: &[(&str, &str)] = &[
    ("easylist", "https://easylist.to/easylist/easylist.txt"),
    ("easyprivacy", "https://easylist.to/easylist/easyprivacy.txt"),
    ("ublock", "https://ublockorigin.github.io/uAssets/filters/filters.min.txt"),
];

/// Used until the first real list is cached, and when offline.
const FALLBACK: &str = "||doubleclick.net^\n||googlesyndication.com^\n||googleadservices.com^\n\
||adservice.google.com^\n||google-analytics.com^\n||googletagmanager.com^\n||adnxs.com^\n\
||taboola.com^\n||outbrain.com^\n||criteo.com^\n||scorecardresearch.com^\n||hotjar.com^\n\
||amazon-adsystem.com^\n||pubmatic.com^\n||rubiconproject.com^\n";

/// WebKit refuses very large lists; split well below its 150k-rule cap.
const RULES_PER_LIST: usize = 40_000;

pub struct Blocker {
    engine: RwLock<Option<Arc<Engine>>>,
    rules: RwLock<Arc<Vec<String>>>,
    dir: PathBuf,
}

fn blocker(app: &AppHandle) -> Option<tauri::State<'_, Blocker>> {
    app.try_state::<Blocker>()
}

/// Build the matching engine and the WebKit-style rule JSON chunks from raw list texts.
pub fn build(texts: &[String]) -> (Engine, Vec<String>) {
    let mut set = FilterSet::new(false);
    let mut cb = FilterSet::new(true);
    for t in texts {
        set.add_filter_list(t.clone(), ParseOptions::default());
        cb.add_filter_list(t.clone(), ParseOptions::default());
    }
    let engine = Engine::new_with_filter_set(set);
    let rules = cb.into_content_blocking().map(|(r, _)| r).unwrap_or_default();
    let chunks = rules
        .chunks(RULES_PER_LIST)
        .filter_map(|c| serde_json::to_string(c).ok())
        .collect();
    (engine, chunks)
}

pub fn is_blocked(engine: &Engine, url: &str, source: &str, kind: &str) -> bool {
    Request::new(url, source, kind, "GET")
        .map(|r| engine.check_network_request(&r).should_block())
        .unwrap_or(false)
}

fn css_for(engine: &Engine, url: &str) -> String {
    let res = engine.url_cosmetic_resources(url);
    let mut sels: Vec<&String> = res.hide_selectors.iter().collect();
    sels.sort();
    if sels.is_empty() {
        return String::new();
    }
    let mut css = String::new();
    // Chunked so a single invalid selector only drops its own chunk.
    for chunk in sels.chunks(200) {
        let list: Vec<&str> = chunk.iter().map(|s| s.as_str()).collect();
        css.push_str(&list.join(","));
        css.push_str("{display:none!important}\n");
    }
    css
}

pub fn enabled_for(app: &AppHandle, host: &str) -> bool {
    let g = app.state::<Shared>().inner().app.lock().unwrap();
    g.cfg.settings.adblock && !g.cfg.allowlist.iter().any(|h| h == host)
}

pub fn cosmetic_css(app: &AppHandle, url: &url::Url) -> String {
    let host = url.host_str().unwrap_or_default();
    if !enabled_for(app, host) {
        return String::new();
    }
    let Some(b) = blocker(app) else { return String::new() };
    let engine = b.engine.read().unwrap().clone();
    engine.map(|e| css_for(&e, url.as_str())).unwrap_or_default()
}

pub fn check(app: &AppHandle, url: &str, source: &str, kind: &str) -> bool {
    let Some(b) = blocker(app) else { return false };
    let engine = b.engine.read().unwrap().clone();
    let host = url::Url::parse(source).ok().and_then(|u| u.host_str().map(String::from)).unwrap_or_default();
    match engine {
        Some(e) if enabled_for(app, &host) => is_blocked(&e, url, source, kind),
        _ => false,
    }
}

/// Called for every new page webview.
pub fn attach(app: &AppHandle, wv: &Webview, _profile: &str) {
    native::attach(app, wv);
}

/// Called on each main-frame navigation, before the load starts.
pub fn on_navigate(app: &AppHandle, label: &str, host: &str) {
    if let Some(wv) = app.get_webview(label) {
        native::set_enabled(&wv, enabled_for(app, host));
    }
}

/// Settings or allowlist changed: re-evaluate all tabs and reload the active one.
pub fn refresh(app: &AppHandle) {
    native::apply_all(app);
    let id = app.state::<Shared>().inner().app.lock().unwrap().active;
    crate::tabs::reload(app, id);
}

fn stale(path: &PathBuf) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .map(|age| age > Duration::from_secs(24 * 3600))
        .unwrap_or(true)
}

fn download(dir: &PathBuf) {
    let _ = std::fs::create_dir_all(dir);
    for (name, url) in LISTS {
        let path = dir.join(format!("{name}.txt"));
        if !stale(&path) {
            continue;
        }
        let res = ureq::get(url).timeout(Duration::from_secs(60)).call();
        if let Ok(text) = res.and_then(|r| r.into_string().map_err(Into::into)) {
            if text.len() > 1000 {
                let _ = std::fs::write(&path, text);
            }
        }
    }
}

fn load_texts(dir: &PathBuf) -> Vec<String> {
    let mut texts: Vec<String> = LISTS
        .iter()
        .filter_map(|(name, _)| std::fs::read_to_string(dir.join(format!("{name}.txt"))).ok())
        .collect();
    if texts.is_empty() {
        texts.push(FALLBACK.into());
    }
    texts
}

fn rebuild(app: &AppHandle) {
    let Some(b) = blocker(app) else { return };
    install(app, &load_texts(&b.dir));
}

/// Swap in a new engine built from `texts` and push it to the platform backend.
pub fn install(app: &AppHandle, texts: &[String]) {
    let Some(b) = blocker(app) else { return };
    let (engine, chunks) = build(texts);
    *b.engine.write().unwrap() = Some(Arc::new(engine));
    *b.rules.write().unwrap() = Arc::new(chunks);
    native::reinstall(app);
}

pub fn init(app: &AppHandle) {
    let dir = app.state::<Shared>().inner().data_dir.join("filters");
    app.manage(Blocker { engine: RwLock::new(None), rules: RwLock::new(Arc::new(vec![])), dir: dir.clone() });
    let app = app.clone();
    std::thread::spawn(move || loop {
        rebuild(&app); // from cache (or fallback) right away
        if LISTS.iter().any(|(n, _)| stale(&dir.join(format!("{n}.txt")))) {
            download(&dir);
            rebuild(&app);
        }
        std::thread::sleep(Duration::from_secs(6 * 3600));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_ads_and_hides_elements() {
        let list = "||ads.example.com^\n@@||ads.example.com/allowed^\nexample.org##.banner-ad\n";
        let (engine, chunks) = build(&[list.into()]);
        assert!(is_blocked(&engine, "https://ads.example.com/x.js", "https://example.org/", "script"));
        assert!(!is_blocked(&engine, "https://ads.example.com/allowed", "https://example.org/", "script"));
        assert!(!is_blocked(&engine, "https://cdn.example.org/app.js", "https://example.org/", "script"));
        assert!(css_for(&engine, "https://example.org/page").contains(".banner-ad"));
        assert!(css_for(&engine, "https://other.net/").is_empty());
        assert!(!chunks.is_empty());
    }

    /// Network + slow: `cargo test real_lists -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_lists() {
        let texts: Vec<String> = LISTS
            .iter()
            .map(|(_, u)| ureq::get(u).call().unwrap().into_string().unwrap())
            .collect();
        let t = std::time::Instant::now();
        let (engine, chunks) = build(&texts);
        println!("built in {:?}, {} rule chunks, sizes {:?}", t.elapsed(), chunks.len(), chunks.iter().map(|c| c.len()).collect::<Vec<_>>());
        assert!(is_blocked(&engine, "https://securepubads.g.doubleclick.net/tag/js/gpt.js", "https://example.com/", "script"));
        assert!(!is_blocked(&engine, "https://example.com/app.js", "https://example.com/", "script"));
        assert!(!chunks.is_empty());
    }

    #[test]
    fn fallback_list_compiles() {
        let (engine, _) = build(&[FALLBACK.into()]);
        assert!(is_blocked(&engine, "https://pagead2.googlesyndication.com/x", "https://site.com/", "script"));
    }
}
