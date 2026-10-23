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
const AUTH_HOSTS: &[&str] = &[
    "accounts.google.com",
    "login.microsoftonline.com",
    "login.live.com",
    "appleid.apple.com",
    "idmsa.apple.com",
    "github.com/login",
    "login.yahoo.com",
    "auth0.com",
    "okta.com",
];

pub fn is_auth_host(host: &str) -> bool {
    AUTH_HOSTS.iter().any(|a| {
        let a = a.split('/').next().unwrap();
        host == a || host.ends_with(&format!(".{a}"))
    })
}

pub fn matches(pattern: &str, host: &str) -> bool {
    match pattern.strip_prefix("*.") {
        Some(base) => host == base || host.ends_with(&format!(".{base}")),
        None => pattern == host,
    }
}

/// Exact patterns beat wildcards; among wildcards the longest wins.
pub fn lookup<'a>(mappings: &'a [Mapping], host: &str) -> Option<&'a Mapping> {
    mappings
        .iter()
        .filter(|m| matches(&m.pattern, host))
        .max_by_key(|m| (!m.pattern.starts_with("*."), m.pattern.len()))
}

pub fn decide(
    mappings: &[Mapping],
    current_profile: &str,
    host: &str,
    user_initiated: bool,
) -> Decision {
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
    fn redirects_and_auth_never_swap() {
        let maps = vec![m("youtube.com", "B")];
        assert_eq!(decide(&maps, "A", "youtube.com", false), Decision::Allow);
        assert_eq!(decide(&maps, "A", "accounts.google.com", true), Decision::Allow);
        assert_eq!(decide(&maps, "A", "sub.okta.com", true), Decision::Allow);
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
