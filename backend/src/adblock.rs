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
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, OnceLock, RwLock,
    },
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
        set.add_filter_list(popup_as_document(t), ParseOptions::default());
        rules += t.lines().filter(|l| !l.is_empty() && !l.starts_with('!') && !l.starts_with('[')).count();
    }
    set.add_filter_list(youtube_rules(), ParseOptions::default());
    let mut engine = Engine::new_with_filter_set(set);
    engine.use_resources(crate::scriptlets::resources());
    (engine, rules)
}

/// Built-in scriptlet rules stripping YouTube's ad fields from player responses (list: `scripts::YT_AD_FIELDS`).
/// The remaining ad handling is the DOM fallback in `scripts::YOUTUBE`.
fn youtube_rules() -> String {
    let f = crate::scripts::YT_AD_FIELDS;
    let paths: Vec<String> = f.iter().map(|x| x.to_string()).chain(f.iter().map(|x| format!("playerResponse.{x}"))).collect();
    let mut out = format!("youtube.com##+js(json-prune, {})\n", paths.join(" "));
    for x in f {
        out.push_str(&format!("youtube.com##+js(set-constant, ytInitialPlayerResponse.{x}, undefined)\n"));
    }
    out
}

/// adblock-rust ignores `$popup`, and only popups are ever checked as "document" (main-frame loads skip the filter),
/// so block rules become `$document`. Exceptions are dropped: `@@...$document` would mean "unfiltered page".
fn popup_as_document(list: &str) -> String {
    if !list.contains("popup") {
        return list.to_string();
    }
    let mut out = String::with_capacity(list.len());
    for line in list.lines() {
        match line.rsplit_once('$') {
            Some((rule, opts)) if !line.starts_with('!') && opts.split(',').any(|o| o == "popup") => {
                if line.starts_with("@@") {
                    continue;
                }
                let opts: Vec<&str> = opts.split(',').map(|o| if o == "popup" { "document" } else { o }).collect();
                out.push_str(&format!("{rule}${}\n", opts.join(",")));
            }
            _ => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    out
}

pub fn is_blocked(engine: &Engine, url: &str, source: &str, kind: &str) -> bool {
    Request::new(url, source, kind, "GET").map(|r| engine.check_network_request(&r).should_block()).unwrap_or(false)
}

fn css_for(engine: &Engine, url: &str) -> String {
    let res = engine.url_cosmetic_resources(url);
    hide_css(res.hide_selectors.into_iter().collect())
}

fn hide_css(mut sels: Vec<String>) -> String {
    sels.sort();
    let mut css = String::new();
    // Chunked so a single invalid selector only drops its own chunk.
    for chunk in sels.chunks(200) {
        css.push_str(&chunk.join(","));
        css.push_str("{display:none!important}\n");
    }
    css
}

/// `url_cosmetic_resources` leaves out generic `##.class` / `###id` rules: the page reports the classes and ids it
/// uses and gets the matching ones back here.
fn generic_css_for(engine: &Engine, url: &str, classes: &[String], ids: &[String]) -> String {
    let res = engine.url_cosmetic_resources(url);
    if res.generichide {
        return String::new();
    }
    hide_css(engine.hidden_class_id_selectors(classes, ids, &res.exceptions))
}

pub fn enabled_for(host: &str) -> bool {
    let g = CFG.read().unwrap();
    g.0 && !g.1.iter().any(|h| covers(h, host))
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

pub fn generic_css(url: &url::Url, classes: &[String], ids: &[String]) -> String {
    if !enabled_for(url.host_str().unwrap_or_default()) {
        return String::new();
    }
    engine().map(|e| generic_css_for(&e, url.as_str(), classes, ids)).unwrap_or_default()
}

/// Scriptlet code for the page at `url` (`##+js(...)` rules), to run in its main world before page scripts.
/// Built by the engine from our bundled scriptlets only; empty when blocking is off for the host.
pub fn scriptlets(url: &str) -> String {
    let host = url::Url::parse(url).ok().and_then(|u| u.host_str().map(String::from)).unwrap_or_default();
    match engine() {
        Some(e) if enabled_for(&host) => e.url_cosmetic_resources(url).injected_script,
        _ => String::new(),
    }
}

/// Should this sub-resource request (made by the page at `source`) be cancelled?
pub fn check(url: &str, source: &str, kind: &str) -> bool {
    let host = url::Url::parse(source).ok().and_then(|u| u.host_str().map(String::from)).unwrap_or_default();
    match engine() {
        Some(e) if enabled_for(&host) => is_blocked(&e, url, source, kind),
        _ => false,
    }
}

/// Click-hijacking ads (popunders, tab-unders) fire inside a real click, so the gesture check lets them through,
/// and their domains rotate faster than the lists (animeworld's loader generates one with WebAssembly when its
/// listed script is blocked). A page-started popup or navigation passes if it stays on the page's site or goes
/// to the site of the link the click landed on (`link`, empty when the click wasn't on one).
pub fn hijack(url: &str, source: &str, link: &str) -> bool {
    let host = url::Url::parse(source).ok().and_then(|u| u.host_str().map(String::from)).unwrap_or_default();
    enabled_for(&host) && is_hijack(url, source, link)
}

/// Sign-in popups open from script buttons (no link), so they look like hijacks; popunders never use these.
const AUTH_HOSTS: &[&str] = &["accounts.google.com", "appleid.apple.com", "login.microsoftonline.com", "login.live.com", "www.facebook.com", "github.com"];

/// Checkout and 3-D Secure pages, opened the same way.
const PAYMENT_HOSTS: &[&str] =
    &["stripe.com", "paypal.com", "adyen.com", "braintreegateway.com", "klarna.com", "checkout.com", "mollie.com", "worldpay.com", "cardinalcommerce.com", "3dsecure.io", "secure.checkout.visa.com"];

fn host_of(url: &str) -> String {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(String::from)).unwrap_or_default()
}

/// Registrable domain (eTLD+1) via the public suffix list; the host itself for IPs and unknown suffixes.
fn site(host: &str) -> &str {
    psl::domain_str(host).unwrap_or(host)
}

fn listed(hosts: &[&str], host: &str) -> bool {
    hosts.iter().any(|h| host == *h || host.strip_suffix(h).is_some_and(|p| p.ends_with('.')))
}

/// Should a page-started popup/navigation from `opener` to `target` (hosts) be treated as a hijack?
/// `link` is the href of the link the user's click landed on ("" if none). Allowed: same site, the clicked link's
/// site, known sign-in hosts, known payment hosts. Everything else is blocked.
/// (Callers only get here after a user gesture, so there is no "no gesture" input.)
fn is_hijack_host(opener: &str, target: &str, link: &str) -> bool {
    if target.is_empty() {
        return true;
    }
    let same = |h: &str| !h.is_empty() && site(h) == site(target);
    let ok = same(opener) || same(&host_of(link)) || listed(AUTH_HOSTS, target) || listed(PAYMENT_HOSTS, target);
    !ok
}

fn is_hijack(url: &str, source: &str, link: &str) -> bool {
    if crate::nav::is_auth_host(&host_of(url)) {
        return false;
    }
    is_hijack_host(&host_of(source), &host_of(url), link)
}

/// Settings or allowlist changed: reload the active tab so it is re-filtered.
pub fn refresh(app: &AppHandle) {
    let (id, url) = {
        let g = tabs::shared(app).lock_app();
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

/// Does an allowlist entry for `entry` cover `host` (the site itself or any subdomain)?
fn covers(entry: &str, host: &str) -> bool {
    !entry.is_empty() && (host == entry || host.strip_suffix(entry).is_some_and(|p| p.ends_with('.')))
}

/// `host`'s allowlist entry was toggled: reload every awake tab (any window) it covers, so each is re-filtered.
pub fn refresh_host(app: &AppHandle, toggled: &str) {
    let hit: Vec<(u32, String)> = {
        let g = tabs::shared(app).lock_app();
        g.tabs
            .iter()
            .filter(|t| !t.discarded && tabs::internal_view(&t.url()).is_none())
            .filter_map(|t| {
                let host = url::Url::parse(&t.url()).ok()?.host_str()?.to_string();
                covers(toggled, &host).then_some((t.id, host))
            })
            .collect()
    };
    for (id, host) in hit {
        let pre = if enabled_for(&host) { "try{sessionStorage.removeItem('__aditus_off')}catch(e){}" } else { "try{sessionStorage.setItem('__aditus_off','1')}catch(e){}" };
        tabs::reload_with(app, id, pre);
    }
}

fn stale(path: &PathBuf) -> bool {
    std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok()).map(|age| age > Duration::from_secs(24 * 3600)).unwrap_or(true)
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
    let mut texts: Vec<String> = LISTS.iter().filter(|(name, _)| !disabled.contains(&name.to_string())).filter_map(|(name, _)| std::fs::read_to_string(dir.join(format!("{name}.txt"))).ok()).collect();
    if texts.is_empty() {
        texts.push(FALLBACK.into());
    }
    texts
}

fn disabled_lists(app: &AppHandle) -> Vec<String> {
    tabs::shared(app).lock_app().cfg.settings.disabled_lists.clone()
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
            age_hours: std::fs::metadata(b.dir.join(format!("{name}.txt"))).and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok()).map(|d| d.as_secs() / 3600),
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
    fn scriptlets_are_injected_from_bundled_resources() {
        let list = "example.org##+js(aopr, adsbygoogle)\nexample.org##+js(ubo-set-constant, a.b, true)\nexample.org##+js(json-prune, ads.*)\n\
                    example.org##+js(rc, ad, .box)\nexample.org##+js(nope-unknown, x)\nexample.net##+js(nostif, \"x\\\"y\")\n";
        let (e, _) = build(&[list.into()]);
        let js = e.url_cosmetic_resources("https://example.org/p").injected_script;
        assert!(js.contains("aditusAopr(\"adsbygoogle\")"), "{js}");
        assert!(js.contains("aditusSet(\"a.b\", \"true\")"), "{js}");
        assert!(js.contains("aditusJsonPrune(\"ads.*\")") && js.contains("function aditusPrune"), "{js}");
        assert!(js.contains("aditusRc(\"ad\", \".box\")") && js.contains("function aditusDom"), "{js}");
        assert!(!js.contains("nope-unknown") && !js.contains("aditusNostif"));
        let net = e.url_cosmetic_resources("https://example.net/").injected_script;
        assert!(net.contains("aditusNostif("), "{net}");
        assert!(e.url_cosmetic_resources("https://other.org/").injected_script.is_empty());
    }

    #[test]
    fn blocks_ads_and_hides_elements() {
        let list = "||ads.example.com^\n@@||ads.example.com/allowed^\nexample.org##.banner-ad\n";
        let (engine, rules) = build(&[list.into()]);
        assert!(is_blocked(&engine, "https://ads.example.com/x.js", "https://example.org/", "script"));
        assert!(!is_blocked(&engine, "https://ads.example.com/allowed", "https://example.org/", "script"));
        assert!(!is_blocked(&engine, "https://cdn.example.org/app.js", "https://example.org/", "script"));
        assert!(css_for(&engine, "https://example.org/page").contains(".banner-ad"));
        let (e, _) = build(&["||pop.example^$popup,third-party\n@@||ok.pop.example^$popup\n".to_string()]);
        assert!(is_blocked(&e, "https://pop.example/x", "https://site.com/", "document"));
        assert!(!is_blocked(&e, "https://pop.example/x", "https://site.com/", "script"));
        assert!(css_for(&engine, "https://other.net/").is_empty());
        assert!(rules > 0);
        let page = "https://www.animeworld.ac/play/x";
        assert!(is_hijack("https://ocbwmegh.in/ad", page, ""));
        assert!(is_hijack("https://ocbwmegh.in/ad", page, "https://www.animeworld.ac/news/y"));
        assert!(!is_hijack("https://t.me/AnimeWorldITA2", page, "https://t.me/AnimeWorldITA2"));
        assert!(!is_hijack("https://static.animeworld.ac/sfondi", page, ""));
        assert!(is_hijack("about:blank", page, ""));
        assert!(!is_hijack("https://accounts.google.com/o/oauth2/auth", "https://shop.com/", ""));
        assert!(!is_hijack("https://www.paypal.com/checkout", "https://shop.com/", ""));
        assert!(is_hijack("https://notpaypal.com/x", "https://shop.com/", ""));
    }

    #[test]
    fn hijack_decision_table() {
        // (opener host, target host, clicked link, hijack?)
        let t = [
            ("shop.com", "accounts.google.com", "", false), // OAuth
            ("shop.com", "login.microsoftonline.com", "", false),
            ("shop.com", "checkout.stripe.com", "", false), // payment
            ("shop.com", "hooks.stripe.com", "", false),    // 3DS
            ("shop.com", "www.paypal.com", "", false),
            ("shop.com", "notpaypal.com", "", true),
            ("shop.com", "evil-stripe.com", "", true),
            ("anime.ac", "ocbwmegh.in", "", true), // tab-under ad
            ("anime.ac", "ocbwmegh.in", "https://anime.ac/news", true),
            ("anime.ac", "t.me", "https://t.me/x", false),  // click on a link to that site
            ("www.anime.ac", "static.anime.ac", "", false), // same site, subdomains
            ("anime.ac", "www.anime.ac", "", false),
            ("a.co.uk", "b.co.uk", "", true), // a public suffix is not a site
            ("a.github.io", "b.github.io", "", true),
            ("a.co.uk", "x.a.co.uk", "", false),
            ("anime.ac", "", "", true), // about:blank
        ];
        for (o, tg, l, want) in t {
            assert_eq!(is_hijack_host(o, tg, l), want, "{o} -> {tg} (link {l:?})");
        }
        assert!(is_hijack("about:blank", "https://anime.ac/", ""));
    }

    #[test]
    fn youtube_ads_are_pruned_by_scriptlets() {
        let (e, _) = build(&[FALLBACK.into()]);
        let js = e.url_cosmetic_resources("https://www.youtube.com/watch?v=x").injected_script;
        assert!(js.contains("aditusJsonPrune(\"adPlacements playerAds"), "{js}");
        assert!(js.contains("playerResponse.adSlots"), "{js}");
        assert!(js.contains("aditusSet(\"ytInitialPlayerResponse.playerAds\", \"undefined\")"), "{js}");
        assert!(e.url_cosmetic_resources("https://example.org/").injected_script.is_empty());
    }

    #[test]
    fn generic_rules_and_allowlist() {
        let (engine, _) = build(&["##.banner-ad\n#@#.keep\n##.keep\nexample.org#@#.banner-ad\n".to_string()]);
        let c = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(css_for(&engine, "https://a.com/").is_empty());
        assert!(generic_css_for(&engine, "https://a.com/", &c(&["x", "banner-ad"]), &[]).contains(".banner-ad"));
        assert!(generic_css_for(&engine, "https://a.com/", &c(&["x"]), &[]).is_empty());
        assert!(generic_css_for(&engine, "https://example.org/", &c(&["banner-ad"]), &[]).is_empty());
        sync_cfg(true, &["example.com".to_string()]);
        assert!(!enabled_for("example.com") && !enabled_for("www.example.com"));
        assert!(enabled_for("badexample.com") && enabled_for("example.org"));
        sync_cfg(true, &[]);
    }

    /// Network + slow: `cargo test real_lists -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_lists() {
        let texts: Vec<String> = LISTS.iter().map(|(_, u)| ureq::get(u).call().unwrap().into_string().unwrap()).collect();
        let t = std::time::Instant::now();
        let (engine, rules) = build(&texts);
        println!("built in {:?}, {} rules", t.elapsed(), rules);
        assert!(is_blocked(&engine, "https://securepubads.g.doubleclick.net/tag/js/gpt.js", "https://example.com/", "script"));
        assert!(!is_blocked(&engine, "https://example.com/app.js", "https://example.com/", "script"));
        assert!(rules > 0);
    }

    #[test]
    fn allowlist_covers_subdomains() {
        assert!(covers("example.com", "example.com"));
        assert!(covers("example.com", "www.example.com"));
        assert!(!covers("example.com", "badexample.com"));
        assert!(!covers("", "example.com"));
    }

    #[test]
    fn fallback_list_compiles() {
        let (engine, _) = build(&[FALLBACK.into()]);
        assert!(is_blocked(&engine, "https://pagead2.googlesyndication.com/x", "https://site.com/", "script"));
    }
}
