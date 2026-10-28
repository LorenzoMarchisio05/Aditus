//! Pure navigation/profile decisions. No webview types so it stays unit-testable.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Mapping {
    /// Exact host (`youtube.com`) or wildcard (`*.google.com`, also matches `google.com`).
    pub pattern: String,
    pub profile: String,
}

#[derive(Debug, PartialEq)]
pub enum Decision {
    Allow,
    Swap(String),
    Prompt(String),
}

/// Hosts that take part in login flows. Never prompt or swap on these.
const AUTH_HOSTS: &[&str] =
    &["accounts.google.com", "login.microsoftonline.com", "login.live.com", "appleid.apple.com", "idmsa.apple.com", "github.com/login", "login.yahoo.com", "auth0.com", "okta.com"];

pub fn is_auth_host(host: &str) -> bool {
    AUTH_HOSTS.iter().any(|a| {
        let a = a.split('/').next().unwrap();
        host == a || host.ends_with(&format!(".{a}"))
    })
}

/// What a `window.open` request becomes.
#[derive(Debug, PartialEq)]
pub enum PopupPlan {
    /// Cancelled: an ad popup (no user gesture, or a listed target).
    Block,
    /// A real popup that keeps `window.opener`, docked as a tab: sign-in and payment windows.
    Related,
    /// Cancelled now and re-checked shortly after (the click's ping may not have landed yet), then a plain tab.
    Deferred,
}

/// `hijack`: the click-hijack heuristic flagged it. `popup_window`: it asked for a window, not a tab
/// (sized `window.open`). Only web addresses are related: an empty or `about:blank` popup is how
/// popunders start, and main-frame requests are not filtered. Auth hosts count as windows.
pub fn popup_plan(user_gesture: bool, listed: bool, hijack: bool, popup_window: bool, url: &str) -> PopupPlan {
    if !user_gesture || listed {
        return PopupPlan::Block;
    }
    let host = url::Url::parse(url).ok().filter(|u| matches!(u.scheme(), "http" | "https")).and_then(|u| u.host_str().map(String::from));
    match host {
        Some(h) if !hijack && (popup_window || is_auth_host(&h)) => PopupPlan::Related,
        _ => PopupPlan::Deferred,
    }
}

/// `(tab, opener)` pairs: is the tab part of an opener relation, on either side? Such tabs must keep
/// their view (no sleep, no profile swap), since the relation lives in the views.
pub fn linked(tabs: &[(u32, Option<u32>)], id: u32) -> bool {
    tabs.iter().any(|&(t, o)| (t == id && o.is_some()) || o == Some(id))
}

/// A navigation made with this request method carries a body that cannot be replayed in a new view.
pub fn is_form_post(method: &str) -> bool {
    method.eq_ignore_ascii_case("POST")
}

/// Reduces whatever was typed or saved (`https://Site.com:8080/a?b#c`) to the bare host pattern.
pub fn clean_pattern(p: &str) -> String {
    let p = p.trim().to_lowercase();
    let p = p.split_once("://").map_or(p.as_str(), |(_, r)| r);
    let p = p.split(['/', '?', '#']).next().unwrap_or_default();
    let p = p.rsplit_once(':').filter(|(_, port)| !port.is_empty() && port.chars().all(|c| c.is_ascii_digit())).map_or(p, |(h, _)| h);
    p.to_string()
}

/// `www.` is not a different site: `youtube.com` and `www.youtube.com` are the same mapping.
fn bare(h: &str) -> &str {
    h.strip_prefix("www.").unwrap_or(h)
}

pub fn matches(pattern: &str, host: &str) -> bool {
    let pattern = clean_pattern(pattern);
    let (pattern, host) = (bare(&pattern), bare(host));
    match pattern.strip_prefix("*.") {
        Some(base) => host == base || host.ends_with(&format!(".{base}")),
        None => pattern == host,
    }
}

/// Exact patterns beat wildcards; among wildcards the longest wins.
pub fn lookup<'a>(mappings: &'a [Mapping], host: &str) -> Option<&'a Mapping> {
    mappings.iter().filter(|m| matches(&m.pattern, host)).max_by_key(|m| (!m.pattern.starts_with("*."), m.pattern.len()))
}

pub fn decide(mappings: &[Mapping], current_profile: &str, host: &str, user_initiated: bool) -> Decision {
    if !user_initiated || host.is_empty() || is_auth_host(host) {
        return Decision::Allow;
    }
    match lookup(mappings, host) {
        Some(m) if m.profile != current_profile => Decision::Swap(m.profile.clone()),
        Some(_) => Decision::Allow,
        None => Decision::Prompt(host.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(p: &str, prof: &str) -> Mapping {
        Mapping { pattern: p.into(), profile: prof.into() }
    }

    #[test]
    fn photos_a_youtube_b() {
        let maps = vec![m("photos.google.com", "A"), m("youtube.com", "B")];
        assert_eq!(decide(&maps, "A", "youtube.com", true), Decision::Swap("B".into()));
        assert_eq!(decide(&maps, "B", "youtube.com", true), Decision::Allow);
        assert_eq!(decide(&maps, "B", "photos.google.com", true), Decision::Swap("A".into()));
    }

    #[test]
    fn path_and_query_in_a_saved_pattern_are_ignored() {
        let maps = vec![m("https://youtube.com/watch?v=1", "B")];
        assert_eq!(decide(&maps, "A", "youtube.com", true), Decision::Swap("B".into()));
    }

    #[test]
    fn www_is_ignored() {
        let maps = vec![m("www.youtube.com", "B")];
        assert_eq!(decide(&maps, "A", "youtube.com", true), Decision::Swap("B".into()));
        let maps = vec![m("youtube.com", "B")];
        assert_eq!(decide(&maps, "A", "www.youtube.com", true), Decision::Swap("B".into()));
    }

    #[test]
    fn redirects_and_auth_never_swap() {
        let maps = vec![m("youtube.com", "B")];
        assert_eq!(decide(&maps, "A", "youtube.com", false), Decision::Allow);
        assert_eq!(decide(&maps, "A", "accounts.google.com", true), Decision::Allow);
        assert_eq!(decide(&maps, "A", "sub.okta.com", true), Decision::Allow);
    }

    #[test]
    fn popup_plans() {
        assert_eq!(popup_plan(false, false, false, true, "https://a.com/"), PopupPlan::Block);
        assert_eq!(popup_plan(true, true, false, true, "https://ads.com/"), PopupPlan::Block);
        assert_eq!(popup_plan(true, false, false, true, "https://shop.com/pay"), PopupPlan::Related);
        assert_eq!(popup_plan(true, false, false, false, "https://shop.com/"), PopupPlan::Deferred); // plain tab
        assert_eq!(popup_plan(true, false, false, false, "https://accounts.google.com/o/oauth2"), PopupPlan::Related);
        assert_eq!(popup_plan(true, false, true, true, "https://popunder.biz/"), PopupPlan::Deferred);
        assert_eq!(popup_plan(true, false, false, true, "about:blank"), PopupPlan::Deferred);
        assert_eq!(popup_plan(true, false, false, true, ""), PopupPlan::Deferred);
        assert_eq!(popup_plan(true, false, false, true, "javascript:alert(1)"), PopupPlan::Deferred);
    }

    #[test]
    fn opener_relation_pins_both_sides() {
        let tabs = [(1, None), (2, Some(1)), (3, None)];
        assert!(linked(&tabs, 1) && linked(&tabs, 2));
        assert!(!linked(&tabs, 3));
        assert!(!linked(&[], 1));
    }

    #[test]
    fn post_is_detected() {
        assert!(is_form_post("POST") && is_form_post("post"));
        assert!(!is_form_post("GET") && !is_form_post(""));
    }

    #[test]
    fn unmapped_prompts() {
        assert_eq!(decide(&[], "A", "example.com", true), Decision::Prompt("example.com".into()));
    }

    #[test]
    fn wildcard_and_precedence() {
        let maps = vec![m("*.google.com", "A"), m("mail.google.com", "B")];
        assert_eq!(lookup(&maps, "mail.google.com").unwrap().profile, "B");
        assert_eq!(lookup(&maps, "photos.google.com").unwrap().profile, "A");
        assert_eq!(lookup(&maps, "google.com").unwrap().profile, "A");
        assert!(lookup(&maps, "notgoogle.com").is_none());
    }
}
