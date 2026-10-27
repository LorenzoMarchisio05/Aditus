//! Saved logins. Passwords live in the OS keychain (one entry per origin + username); the database
//! only indexes which logins exist. Origins always come from the browser process, never a payload.

use std::{cell::RefCell, collections::HashMap, time::Instant};
use url::Url;

/// Debug builds use their own service so development never touches real saved passwords.
pub const SERVICE: &str = if cfg!(debug_assertions) { "dev.aditus.browser.passwords.dev" } else { "dev.aditus.browser.passwords" };

/// `scheme://host[:port]` for http(s) URLs: the unit a login belongs to.
pub fn origin_of(url: &str) -> Option<String> {
    let u = Url::parse(url).ok().filter(|u| matches!(u.scheme(), "http" | "https"))?;
    Some(u.origin().ascii_serialization())
}

/// Keychain account: per profile since 0.1.5. Older entries (`origin\tuser`) belong to the first profile.
fn account(profile: &str, origin: &str, user: &str) -> String {
    format!("{profile}\t{origin}\t{user}")
}

fn entry(service: &str, account: &str) -> keyring::Result<keyring::Entry> {
    keyring::Entry::new(service, account)
}

pub fn save(service: &str, profile: &str, origin: &str, user: &str, pw: &str) -> Result<(), String> {
    entry(service, &account(profile, origin, user)).and_then(|e| e.set_password(pw)).map_err(|e| e.to_string())
}

/// `legacy`: this is the first profile, which owns pre-profile entries. Those are copied to the new key when read.
pub fn get(service: &str, profile: &str, origin: &str, user: &str, legacy: bool) -> Option<String> {
    if let Some(pw) = entry(service, &account(profile, origin, user)).ok().and_then(|e| e.get_password().ok()) {
        return Some(pw);
    }
    let old = entry(service, &format!("{origin}\t{user}")).ok().filter(|_| legacy)?;
    let pw = old.get_password().ok()?;
    if save(service, profile, origin, user, &pw).is_ok() {
        let _ = old.delete_credential();
    }
    Some(pw)
}

pub fn delete(service: &str, profile: &str, origin: &str, user: &str, legacy: bool) {
    let mut accounts = vec![account(profile, origin, user)];
    if legacy {
        accounts.push(format!("{origin}\t{user}"));
    }
    for a in accounts {
        if let Ok(e) = entry(service, &a) {
            let _ = e.delete_credential();
        }
    }
}

/// A submitted login waiting for the user's yes. Memory only, one per page view, short-lived.
pub struct Offer {
    pub profile: String,
    pub origin: String,
    pub user: String,
    pub pw: String,
    at: Instant,
}

thread_local! {
    static PENDING: RefCell<HashMap<String, Offer>> = RefCell::new(HashMap::new());
}

pub fn offer(label: &str, profile: String, origin: String, user: String, pw: String) {
    PENDING.with(|p| p.borrow_mut().insert(label.into(), Offer { profile, origin, user, pw, at: Instant::now() }));
}

/// Take the pending offer (accept or dismiss); stale ones (> 5 min) are dropped.
pub fn take(label: &str) -> Option<Offer> {
    PENDING.with(|p| p.borrow_mut().remove(label)).filter(|o| o.at.elapsed().as_secs() < 300)
}

/// JS that fills the page's first password field and the field before it.
pub fn fill_js(user: &str, pw: &str) -> String {
    let (u, p) = (serde_json::to_string(user).unwrap(), serde_json::to_string(pw).unwrap());
    format!(
        "(()=>{{const set=(el,v)=>{{const d=Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value');d.set.call(el,v);el.dispatchEvent(new Event('input',{{bubbles:true}}));el.dispatchEvent(new Event('change',{{bubbles:true}}))}};\
         const p=[...document.querySelectorAll('input[type=password]')].find(e=>e.offsetParent);if(!p)return;\
         const all=[...document.querySelectorAll('input')];const i=all.indexOf(p);\
         const u=all.slice(0,i).reverse().find(e=>/^(text|email|tel|)$/.test(e.type)&&e.offsetParent);\
         if(u)set(u,{u});set(p,{p})}})()"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accounts() {
        assert_eq!(account("p1", "https://a.com", "me"), "p1\thttps://a.com\tme");
        assert_ne!(account("p1", "https://a.com", "me"), account("p2", "https://a.com", "me"));
    }

    #[test]
    fn origin() {
        assert_eq!(origin_of("https://a.com/x?y").as_deref(), Some("https://a.com"));
        assert_eq!(origin_of("http://a.com:8080/").as_deref(), Some("http://a.com:8080"));
        assert_eq!(origin_of("aditus://app/x"), None);
        assert_ne!(origin_of("http://a.com/"), origin_of("https://a.com/"));
    }

    /// Hits the real OS keychain under a throwaway service name; run with `--ignored`.
    #[test]
    #[ignore]
    fn keychain_roundtrip() {
        let svc = "dev.aditus.test.passwords";
        save(svc, "p1", "https://t.example", "me", "s3cret").unwrap();
        assert_eq!(get(svc, "p1", "https://t.example", "me", false).as_deref(), Some("s3cret"));
        assert_eq!(get(svc, "p2", "https://t.example", "me", false), None);
        delete(svc, "p1", "https://t.example", "me", false);
        assert_eq!(get(svc, "p1", "https://t.example", "me", false), None);
    }
}
