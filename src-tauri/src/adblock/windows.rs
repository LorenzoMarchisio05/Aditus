//! WebView2 request interception.

use tauri::{AppHandle, Manager, Webview};
use webview2_com::{
    take_pwstr, WebResourceRequestedEventHandler,
    Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FETCH, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_MEDIA, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_SCRIPT,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_STYLESHEET, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_XML_HTTP_REQUEST,
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FONT, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT,
    },
};
use windows::core::{w, PWSTR};

fn kind(ctx: COREWEBVIEW2_WEB_RESOURCE_CONTEXT) -> &'static str {
    match ctx {
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_SCRIPT => "script",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE => "image",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_STYLESHEET => "stylesheet",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_MEDIA => "media",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FONT => "font",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_DOCUMENT => "document",
        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_XML_HTTP_REQUEST | COREWEBVIEW2_WEB_RESOURCE_CONTEXT_FETCH => "xhr",
        _ => "other",
    }
}

pub fn attach(app: &AppHandle, wv: &Webview) {
    let app = app.clone();
    let _ = wv.with_webview(move |pw| unsafe {
        let Ok(core) = pw.controller().CoreWebView2() else { return };
        let env = pw.environment();
        let _ = core.AddWebResourceRequestedFilter(w!("*"), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL);
        let handler = WebResourceRequestedEventHandler::create(Box::new(move |sender, args| {
            let (Some(sender), Some(args)) = (sender, args) else { return Ok(()) };
            let request = args.Request()?;
            let mut uri = PWSTR::null();
            request.Uri(&mut uri)?;
            let uri = take_pwstr(uri);
            let mut src = PWSTR::null();
            sender.Source(&mut src)?;
            let src = take_pwstr(src);
            let mut ctx = COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL;
            args.ResourceContext(&mut ctx)?;
            if super::check(&app, &uri, &src, kind(ctx)) {
                let resp = env.CreateWebResourceResponse(None, 403, w!("Blocked"), w!(""))?;
                args.SetResponse(&resp)?;
            }
            Ok(())
        }));
        let mut token = Default::default();
        let _ = core.add_WebResourceRequested(&handler, &mut token);
    });
}

/// Blocking is decided per request against the allowlist, so there is nothing to toggle.
pub fn set_enabled(_wv: &Webview, _enabled: bool) {}
pub fn apply_all(_app: &AppHandle) {}
pub fn reinstall(_app: &AppHandle) {}
