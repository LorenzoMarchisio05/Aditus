//! Browser side of `cefQuery`: every call from a page or from the UI ends up in `commands::dispatch`.
//!
//! Trust is decided here, from what the browser process knows (which view, which frame, its real
//! URL), never from the payload: only the app's own pages (`aditus://app/...` in a main frame of a
//! view that may host them) get the full command set; web pages may only ping.

use cef::{
    wrapper::message_router::{
        BrowserSideCallback, BrowserSideHandler, BrowserSideRouter, MessageRouterBrowserSide, MessageRouterConfig,
    },
    *,
};
use serde_json::Value;
use std::sync::{Arc, Mutex};

pub use cef::wrapper::message_router::MessageRouterBrowserSideHandlerCallbacks;

/// Who is calling: the view's label, its window, and whether it may use the full command set.
pub struct Ctx {
    pub label: String,
    pub win: String,
    pub trusted: bool,
    pub frame_url: String,
    /// The engine's id of the calling frame (not from the payload).
    pub frame: String,
}

/// Answers one query; usable from any thread.
#[derive(Clone)]
pub struct Reply(Arc<Mutex<dyn BrowserSideCallback>>);

impl Reply {
    pub fn done(&self, r: Result<Value, String>) {
        let cb = self.0.lock().unwrap();
        match r {
            Ok(v) => cb.success_str(&v.to_string()),
            Err(e) => cb.failure(1, &e),
        }
    }
}

/// Commands a web page may call.
const PAGE_COMMANDS: &[&str] = &["page_ping", "page_state", "page_login", "page_link", "cosmetic_css"];

struct Handler;

impl BrowserSideHandler for Handler {
    fn on_query_str(
        &self,
        browser: Option<Browser>,
        frame: Option<Frame>,
        _query_id: i64,
        request: &str,
        _persistent: bool,
        callback: Arc<Mutex<dyn BrowserSideCallback>>,
    ) -> bool {
        let reply = Reply(callback);
        let (Some(browser), Some(frame)) = (browser, frame) else { return false };
        let Some(ctx) = crate::engine::ctx(&browser, &frame) else { return false };
        let Ok(mut q) = serde_json::from_str::<Value>(request) else {
            reply.done(Err("bad request".into()));
            return true;
        };
        let cmd = q["cmd"].as_str().unwrap_or_default().to_string();
        if !ctx.trusted && !PAGE_COMMANDS.contains(&cmd.as_str()) {
            reply.done(Err("not allowed".into()));
            return true;
        }
        let args = q["args"].take();
        // The router holds its own lock while calling us; a command that navigates would re-enter it.
        let args = if args.is_null() { Value::Object(Default::default()) } else { args };
        crate::engine::defer(move || crate::commands::dispatch(&ctx, &cmd, args, reply));
        true
    }
}

thread_local! {
    static ROUTER: Arc<BrowserSideRouter> = {
        let r = BrowserSideRouter::new(MessageRouterConfig::default());
        r.add_handler(Arc::new(Handler), false);
        r
    };
}

pub fn router() -> Arc<BrowserSideRouter> {
    ROUTER.with(Arc::clone)
}
