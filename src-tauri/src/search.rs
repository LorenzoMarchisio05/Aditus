//! Address bar input -> URL.

use url::Url;

pub const DDG: &str = "https://duckduckgo.com/?q=%s";
pub const GOOGLE: &str = "https://www.google.com/search?q=%s";

pub fn resolve(input: &str, template: &str) -> Url {
    let input = input.trim();
    if let Ok(u) = Url::parse(input) {
        if matches!(u.scheme(), "http" | "https" | "file" | "about") && (u.has_host() || u.scheme() != "http") {
            return u;
        }
    }
    let looks_like_host = !input.contains(' ')
        && (input.contains('.') || input.starts_with("localhost"))
        && !input.starts_with('.')
        && !input.ends_with('.');
    if looks_like_host {
        let scheme = if input.starts_with("localhost") || input.starts_with("127.") { "http" } else { "https" };
        if let Ok(u) = Url::parse(&format!("{scheme}://{input}")) {
            return u;
        }
    }
    let q = url::form_urlencoded::byte_serialize(input.as_bytes()).collect::<String>();
    Url::parse(&template.replace("%s", &q)).unwrap_or_else(|_| Url::parse(&DDG.replace("%s", &q)).unwrap())
}

/// The engine's suggestion endpoint for the configured search template; None for a custom one.
pub fn suggest_url(template: &str, q: &str) -> Option<String> {
    let q = url::form_urlencoded::byte_serialize(q.as_bytes()).collect::<String>();
    if template.contains("duckduckgo.com") {
        Some(format!("https://duckduckgo.com/ac/?type=list&q={q}"))
    } else if template.contains("google.com/search") {
        Some(format!("https://suggestqueries.google.com/complete/search?client=firefox&q={q}"))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn suggest_url_per_engine() {
        assert!(super::suggest_url(super::DDG, "a b").unwrap().ends_with("q=a+b"));
        assert!(super::suggest_url(super::GOOGLE, "x").unwrap().contains("suggestqueries.google.com"));
        assert!(super::suggest_url("https://example.com/?q=%s", "x").is_none());
    }

    use super::*;

    #[test]
    fn urls_hosts_and_queries() {
        assert_eq!(resolve("https://a.com/x", DDG).as_str(), "https://a.com/x");
        assert_eq!(resolve("youtube.com", DDG).as_str(), "https://youtube.com/");
        assert_eq!(resolve("localhost:3000", DDG).as_str(), "http://localhost:3000/");
        assert_eq!(resolve("rust lang", DDG).as_str(), "https://duckduckgo.com/?q=rust+lang");
        assert_eq!(resolve("hello", GOOGLE).as_str(), "https://www.google.com/search?q=hello");
    }
}
