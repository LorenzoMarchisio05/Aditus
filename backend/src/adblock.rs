//! Ad blocking: one `adblock-rust` engine.
//!
//! - network blocking: `engine::Blocker` asks `check` for every sub-resource request (IO thread)
//! - cosmetic filtering: the page script fetches CSS through the `cosmetic_css` command

use crate::tabs::{self, AppHandle};
use adblock::{
    lists::{FilterSet, ParseOptions},
    request::Request,
    Engine,
};
use std::{
    path::PathBuf,
    sync::{atomic::{AtomicUsize, Ordering}, Arc, OnceLock, RwLock},
    time::{Duration, SystemTime},
};

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


struct Blocker {
    engine: RwLock<Option<Arc<Engine>>>,
    rules: AtomicUsize,
    dir: PathBuf,
}

static BLOCKER: OnceLock<Blocker> = OnceLock::new();

/// Master switch and allowlist, mirrored from the config so the IO thread never takes the tab lock.
static CFG: RwLock<(bool, Vec<String>)> = RwLock::new((true, Vec::new()));

pub fn sync_cfg(enabled: bool, allowlist: &[String]) {
    *CFG.write().unwrap() = (enabled, allowlist.to_vec());
}

/// Build the matching engine from raw list texts; also returns the rule count.
pub fn build(texts: &[String]) -> (Engine, usize) {
    let mut set = FilterSet::new(false);
    let mut rules = 0;
    for t in texts {
        set.add_filter_list(t.clone(), ParseOptions::default());
        rules += t.lines().filter(|l| !l.is_empty() && !l.starts_with('!') && !l.starts_with('[')).count();
    }
    (Engine::new_with_filter_set(set), rules)
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

pub fn enabled_for(host: &str) -> bool {
    let g = CFG.read().unwrap();
    g.0 && !g.1.iter().any(|h| h == host)
}

fn engine() -> Option<Arc<Engine>> {
    BLOCKER.get()?.engine.read().unwrap().clone()
}

pub fn cosmetic_css(url: &url::Url) -> String {
    if !enabled_for(url.host_str().unwrap_or_default()) {
        return String::new();
    }
    engine().map(|e| css_for(&e, url.as_str())).unwrap_or_default()
}

/// Should this sub-resource request (made by the page at `source`) be cancelled?
pub fn check(url: &str, source: &str, kind: &str) -> bool {
    let host = url::Url::parse(source).ok().and_then(|u| u.host_str().map(String::from)).unwrap_or_default();
    match engine() {
        Some(e) if enabled_for(&host) => is_blocked(&e, url, source, kind),
        _ => false,
    }
}

/// Settings or allowlist changed: reload the active tab so it is re-filtered.
pub fn refresh(app: &AppHandle) {
    let (id, url) = {
        let g = tabs::shared(app).app.lock().unwrap();
        let id = g.focused_active();
        (id, g.tabs.iter().find(|t| t.id == id).map(|t| t.url()).unwrap_or_default())
    };
    // Internal pages (settings, history...) have nothing to re-filter; reloading would reset them.
    if tabs::internal_view(&url).is_none() {
        let host = url::Url::parse(&url).ok().and_then(|u| u.host_str().map(String::from)).unwrap_or_default();
        let off = !enabled_for(&host);
        let pre = if off { "try{sessionStorage.setItem('__aditus_off','1')}catch(e){}" } else { "try{sessionStorage.removeItem('__aditus_off')}catch(e){}" };
        tabs::reload_with(app, id, pre);
    }
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

fn load_texts(dir: &PathBuf, disabled: &[String]) -> Vec<String> {
    let mut texts: Vec<String> = LISTS
        .iter()
        .filter(|(name, _)| !disabled.contains(&name.to_string()))
        .filter_map(|(name, _)| std::fs::read_to_string(dir.join(format!("{name}.txt"))).ok())
        .collect();
    if texts.is_empty() {
        texts.push(FALLBACK.into());
    }
    texts
}

fn disabled_lists(app: &AppHandle) -> Vec<String> {
    tabs::shared(app).app.lock().unwrap().cfg.settings.disabled_lists.clone()
}

/// Rebuild from the cached lists (or the fallback). Runs on a worker thread; the engine swap is atomic.
pub fn rebuild(app: &AppHandle) {
    let Some(b) = BLOCKER.get() else { return };
    install(&load_texts(&b.dir, &disabled_lists(app)));
}

#[derive(serde::Serialize)]
pub struct ListInfo {
    name: &'static str,
    enabled: bool,
    /// Hours since the cached copy was fetched; None if never.
    age_hours: Option<u64>,
}

#[derive(serde::Serialize)]
pub struct Info {
    lists: Vec<ListInfo>,
    rules: usize,
}

pub fn info(app: &AppHandle) -> Info {
    let disabled = disabled_lists(app);
    let Some(b) = BLOCKER.get() else { return Info { lists: vec![], rules: 0 } };
    let lists = LISTS
        .iter()
        .map(|(name, _)| ListInfo {
            name,
            enabled: !disabled.contains(&name.to_string()),
            age_hours: std::fs::metadata(b.dir.join(format!("{name}.txt")))
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| SystemTime::now().duration_since(t).ok())
                .map(|d| d.as_secs() / 3600),
        })
        .collect();
    Info { lists, rules: b.rules.load(Ordering::Relaxed) }
}

/// Swap in a new engine built from `texts`.
pub fn install(texts: &[String]) {
    let Some(b) = BLOCKER.get() else { return };
    let (engine, rules) = build(texts);
    *b.engine.write().unwrap() = Some(Arc::new(engine));
    b.rules.store(rules, Ordering::Relaxed);
}

/// Start the list loader: cached lists first, then a refresh every 6 hours. Never blocks start-up.
pub fn init(dir: PathBuf) {
    let _ = BLOCKER.set(Blocker { engine: RwLock::new(None), rules: AtomicUsize::new(0), dir: dir.clone() });
    std::thread::spawn(move || loop {
        rebuild(&AppHandle); // from cache (or fallback) right away
        if LISTS.iter().any(|(n, _)| stale(&dir.join(format!("{n}.txt")))) {
            download(&dir);
            rebuild(&AppHandle);
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
        let (engine, rules) = build(&[list.into()]);
        assert!(is_blocked(&engine, "https://ads.example.com/x.js", "https://example.org/", "script"));
        assert!(!is_blocked(&engine, "https://ads.example.com/allowed", "https://example.org/", "script"));
        assert!(!is_blocked(&engine, "https://cdn.example.org/app.js", "https://example.org/", "script"));
        assert!(css_for(&engine, "https://example.org/page").contains(".banner-ad"));
        assert!(css_for(&engine, "https://other.net/").is_empty());
        assert!(rules > 0);
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
        let (engine, rules) = build(&texts);
        println!("built in {:?}, {} rules", t.elapsed(), rules);
        assert!(is_blocked(&engine, "https://securepubads.g.doubleclick.net/tag/js/gpt.js", "https://example.com/", "script"));
        assert!(!is_blocked(&engine, "https://example.com/app.js", "https://example.com/", "script"));
        assert!(rules > 0);
    }

    #[test]
    fn fallback_list_compiles() {
        let (engine, _) = build(&[FALLBACK.into()]);
        assert!(is_blocked(&engine, "https://pagead2.googlesyndication.com/x", "https://site.com/", "script"));
    }
}
