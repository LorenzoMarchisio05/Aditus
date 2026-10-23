//! Ad blocking. Filled in per platform in phase 3.

use tauri::{AppHandle, Webview};

pub fn attach(_app: &AppHandle, _wv: &Webview, _profile: &str) {}

pub fn init(_app: &AppHandle) {}
pub fn refresh(_app: &AppHandle) {}
