const COMMANDS: &[&str] = &[
    "get_state", "new_tab", "close_tab", "activate_tab", "address", "back", "forward", "reload",
    "set_layout", "assign_mapping", "skip_prompt", "undo_swap", "dismiss_toast", "save_settings",
    "add_profile", "delete_profile", "delete_mapping", "toggle_allowlist", "pin_tab", "group_tab",
    "find", "find_clear", "open_devtools", "history_search", "history_clear", "downloads_list",
    "show_palette", "hide_palette", "reveal_path", "reorder_tab", "page_ping", "cosmetic_css", "show_panel", "hide_panel", "switch_profile", "adblock_info", "check_update",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("tauri build failed");
}
