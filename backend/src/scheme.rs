//! `aditus://app/...`: serves the built Svelte UI (`dist`) and the new-tab page from disk.

use cef::*;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

pub const HOST: &str = "app";

/// Defense in depth for the privileged UI origin: bundled scripts only, no plugins, no framing, no form posts.
/// Inline styles (Svelte `style=`), data: images/fonts (Vite inlines small assets) and https favicons are needed.
const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: https: http:; font-src 'self' data:; connect-src 'self'; object-src 'none'; frame-src 'none'; base-uri 'none'; form-action 'none'";

static DIST: OnceLock<PathBuf> = OnceLock::new();

/// Where `dist` lives: next to the executable (`Resources/dist` in a macOS bundle), or the
/// source tree's build output for a plain `cargo run`.
pub fn dist_dir() -> &'static Path {
    DIST.get_or_init(|| {
        if let Some(d) = std::env::var_os("ADITUS_DIST").filter(|_| cfg!(debug_assertions)) {
            return d.into();
        }
        let exe = std::env::current_exe().unwrap_or_default();
        let dir = exe.parent().unwrap_or(Path::new("."));
        [dir.join("../Resources/dist"), dir.join("dist"), Path::new(env!("CARGO_MANIFEST_DIR")).join("../ui/dist")]
            .into_iter()
            .find(|p| p.join("index.html").exists())
            .unwrap_or_else(|| dir.join("dist"))
    })
}

fn mime(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html",
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "json" | "webmanifest" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "wasm" => "application/wasm",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    }
}

/// Request path -> file under `dist`; `None` for anything that could leave it.
fn resolve(url: &str) -> Option<PathBuf> {
    let u = url::Url::parse(url).ok()?;
    let rel = u.path().trim_start_matches('/');
    let rel = if rel.is_empty() { "index.html" } else { rel };
    if rel.split('/').any(|s| s == ".." || s.contains('\\') || s.is_empty()) {
        return None;
    }
    Some(dist_dir().join(rel))
}

struct Body {
    data: Vec<u8>,
    pos: usize,
    mime: &'static str,
    status: i32,
    cache: &'static str,
}

wrap_resource_handler! {
    struct AppResource {
        body: Arc<Mutex<Body>>,
    }

    impl ResourceHandler {
        fn open(&self, _request: Option<&mut Request>, handle_request: Option<&mut i32>, _callback: Option<&mut Callback>) -> i32 {
            if let Some(h) = handle_request {
                *h = 1;
            }
            1
        }

        fn response_headers(&self, response: Option<&mut Response>, response_length: Option<&mut i64>, _redirect: Option<&mut CefString>) {
            let b = self.body.lock().unwrap();
            if let Some(r) = response {
                r.set_status(b.status);
                r.set_mime_type(Some(&CefString::from(b.mime)));
                r.set_header_by_name(Some(&CefString::from("Content-Security-Policy")), Some(&CefString::from(CSP)), 1);
                r.set_header_by_name(Some(&CefString::from("Cache-Control")), Some(&CefString::from(b.cache)), 1);
            }
            if let Some(l) = response_length {
                *l = b.data.len() as i64;
            }
        }

        fn read(&self, data_out: *mut u8, bytes_to_read: i32, bytes_read: Option<&mut i32>, _callback: Option<&mut ResourceReadCallback>) -> i32 {
            let mut b = self.body.lock().unwrap();
            let n = (b.data.len() - b.pos).min(bytes_to_read.max(0) as usize);
            if n > 0 {
                // SAFETY: CEF hands us a buffer of at least `bytes_to_read` bytes.
                unsafe { std::ptr::copy_nonoverlapping(b.data.as_ptr().add(b.pos), data_out, n) };
                b.pos += n;
            }
            if let Some(r) = bytes_read {
                *r = n as i32;
            }
            (n > 0) as i32
        }
    }
}

wrap_scheme_handler_factory! {
    pub struct AppSchemes;

    impl SchemeHandlerFactory {
        fn create(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _scheme: Option<&CefString>, request: Option<&mut Request>) -> Option<ResourceHandler> {
            let url = request.map(|r| CefString::from(&r.url()).to_string()).unwrap_or_default();
            let (path, file) = match resolve(&url) {
                Some(p) => (p.to_string_lossy().into_owned(), std::fs::read(&p).ok().or_else(|| (!url.contains('.')).then(|| std::fs::read(dist_dir().join("index.html")).ok()).flatten())),
                None => (String::new(), None),
            };
            let body = match file {
                Some(data) => Body { data, pos: 0, mime: mime(&path), status: 200, cache: if url.contains("/assets/") { "max-age=31536000, immutable" } else { "no-store" } },
                None => Body { data: b"not found".to_vec(), pos: 0, mime: "text/plain", status: 404, cache: "no-store" },
            };
            Some(AppResource::new(Arc::new(Mutex::new(body))))
        }
    }
}

/// Registers the factory on `ctx` (a profile's context), or on the global context when `None`.
pub fn register(ctx: Option<&RequestContext>) {
    let (scheme, host) = (CefString::from(super::app::SCHEME), CefString::from(HOST));
    let mut factory = AppSchemes::new();
    match ctx {
        Some(c) => {
            c.register_scheme_handler_factory(Some(&scheme), Some(&host), Some(&mut factory));
        }
        None => {
            register_scheme_handler_factory(Some(&scheme), Some(&host), Some(&mut factory));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_stays_inside_dist() {
        assert!(resolve("aditus://app/assets/x.js").unwrap().starts_with(dist_dir()));
        assert!(resolve("aditus://app/").unwrap().ends_with("index.html"));
        // The url crate folds `..` and `%2e%2e` away before resolve sees them, so both land inside dist.
        assert_eq!(resolve("aditus://app/../secret").unwrap(), dist_dir().join("secret"));
        assert_eq!(resolve("aditus://app/a/%2e%2e/b").unwrap(), dist_dir().join("b"));
        assert!(resolve(r"aditus://app/a\b").is_none());
        assert!(resolve("aditus://app/a//b").is_none());
    }

    #[test]
    fn mimes() {
        assert_eq!(mime("a.js"), "text/javascript");
        assert_eq!(mime("a.css"), "text/css");
    }
}
