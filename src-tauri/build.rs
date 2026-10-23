const COMMANDS: &[&str] = &[
    "get_state", "new_tab", "close_tab", "activate_tab", "address", "back", "forward", "reload",
    "set_layout", "assign_mapping", "skip_prompt", "undo_swap", "dismiss_toast", "save_settings",
    "add_profile", "delete_profile", "delete_mapping", "toggle_allowlist", "pin_tab", "group_tab",
    "find", "find_clear", "open_devtools", "history_search", "history_clear", "downloads_list",
    "show_palette", "hide_palette", "suggest_show", "suggest_hide", "suggest_get", "reveal_path", "reorder_tab", "page_ping", "cosmetic_css", "show_panel", "hide_panel", "switch_profile", "profile_menu", "tab_menu", "adblock_info", "check_update", "default_browser_settings", "set_profile_color", "page_context", "open_download", "answer_permission", "delete_permission",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("tauri build failed");
}
