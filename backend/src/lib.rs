pub mod adblock;
pub mod app;
pub mod applog;
pub mod commands;
pub mod config;
pub mod db;
pub mod engine;
pub mod extensions;
pub mod import;
pub mod ipc;
pub mod menu;
pub mod passwords;
pub mod nav;
pub mod scheme;
pub mod scripts;
pub mod search;
pub mod tabs;
pub mod update;
#[cfg(target_os = "macos")]
mod mac;

use cef::*;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};

/// Links that arrived before the UI was up (macOS `openURLs`), drained at start-up.
static OPEN_QUEUE: Mutex<Vec<String>> = Mutex::new(Vec::new());
static STARTED: AtomicBool = AtomicBool::new(false);

pub fn queue_urls(urls: Vec<String>) {
    OPEN_QUEUE.lock().unwrap().extend(urls);
    if STARTED.load(Ordering::Relaxed) {
        engine::defer_from_thread(drain_open_queue);
    }
}

pub fn drain_open_queue() {
    STARTED.store(true, Ordering::Relaxed);
    let urls = std::mem::take(&mut *OPEN_QUEUE.lock().unwrap());
    tabs::open_urls(&tabs::AppHandle, urls);
}

/// Dock icon clicked: bring the focused window forward.
pub fn reopen() {
    let win = tabs::shared(&tabs::AppHandle).app.lock().unwrap().focused.clone();
    engine::raise_window(&win);
}

pub(crate) fn data_dir() -> PathBuf {
    if cfg!(debug_assertions) {
        if let Some(d) = std::env::var_os("ADITUS_DATA_DIR") {
            return d.into();
        }
    }
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"));
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "linux")]
    let base = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    base.unwrap_or_else(|| PathBuf::from(".")).join("dev.aditus.browser")
}

fn load_state() -> Result<PathBuf, String> {
    let dir = data_dir();
    std::fs::create_dir_all(dir.join("cef")).map_err(|e| e.to_string())?;
    let cfg_path = dir.join("config.toml");
    let cfg = config::Config::load(&cfg_path);
    cfg.save(&cfg_path);
    let db = db::Db::open(&dir.join("aditus.db")).map_err(|e| e.to_string())?;
    adblock::sync_cfg(cfg.settings.adblock, &cfg.allowlist);
    tabs::init_shared(tabs::Shared { app: Mutex::new(tabs::App::new(cfg)), db, cfg_path, data_dir: dir.clone() });
    Ok(dir)
}

#[cfg(target_os = "macos")]
type Loader = library_loader::LibraryLoader;
#[cfg(not(target_os = "macos"))]
type Loader = ();

fn load_cef(helper: bool) -> Loader {
    #[cfg(target_os = "macos")]
    {
        let loader = library_loader::LibraryLoader::new(&std::env::current_exe().unwrap(), helper);
        assert!(loader.load(), "cannot load the CEF framework");
        let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
        loader
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = helper;
        let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    }
}

pub fn run_helper() {
    let args = args::Args::new();
    #[cfg(all(target_os = "macos", feature = "sandbox"))]
    let _sandbox = {
        let mut sandbox = sandbox::Sandbox::new();
        sandbox.initialize(args.as_main_args());
        sandbox
    };
    let _loader = load_cef(true);
    execute_process(Some(args.as_main_args()), Some(&mut app::AditusApp::new()), std::ptr::null_mut());
}

pub fn run_browser() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    mac::brand_name();
    let _loader = load_cef(false);
    #[cfg(target_os = "macos")]
    mac::setup_application();

    let args = args::Args::new();
    let main_args = args.as_main_args();
    let mut application = app::AditusApp::new();
    let ret = execute_process(Some(main_args), Some(&mut application), std::ptr::null_mut());
    if ret >= 0 {
        return Ok(()); // a child process role ran and finished
    }

    let dir = load_state()?;
    let settings = Settings {
        no_sandbox: !cfg!(feature = "sandbox") as _,
        root_cache_path: CefString::from(dir.join("cef").to_string_lossy().as_ref()),
        cache_path: CefString::from(dir.join("cef/ui").to_string_lossy().as_ref()),
        locale: CefString::from("en-US"),
        log_severity: if std::env::var_os("ADITUS_VERBOSE").is_some() { LogSeverity::VERBOSE } else { LogSeverity::DEFAULT },
        // Debug runs only: DevTools protocol for scripted checks (ADITUS_DEBUG_PORT=9333).
        remote_debugging_port: if cfg!(debug_assertions) { std::env::var("ADITUS_DEBUG_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(0) } else { 0 },
        ..Default::default()
    };
    if initialize(Some(main_args), Some(&settings), Some(&mut application), std::ptr::null_mut()) != 1 {
        return Err("CEF failed to initialise (another Aditus running?)".into());
    }
    #[cfg(target_os = "macos")]
    let _delegate = mac::setup_delegate();

    run_message_loop();
    shutdown();

    let s = tabs::shared(&tabs::AppHandle);
    if s.app.lock().unwrap().cfg.settings.clear_history_on_quit {
        s.db.clear_history();
    }
    Ok(())
}
