//! Menu bar (macOS) and the handler for every menu / popup pick.

use crate::{
    engine,
    tabs::{self, shared, AppHandle},
};

const APP: AppHandle = AppHandle;

/// Menu bar picks, tab / profile popup picks and shortcuts all land here.
pub fn picked(id: &str) {
    let (win, cur) = shared(&APP).lock_app().focused_active_with_win();
    let ui = tabs::wv_label("ui", &win);
    // A link selected in the address bar or the page is what a new tab or private window opens.
    let link = matches!(id, "new-tab" | "new-private-window").then(|| tabs::selected_link(&APP, &win, cur)).flatten();
    // The page usually holds keyboard focus; an input in the UI can't take it otherwise.
    if matches!(id, "focus-address" | "new-tab" | "find" | "search-tabs") && link.is_none() {
        engine::focus(&ui);
    }
    let to_ui = |what: &str| engine::emit_to(&ui, "menu", &serde_json::to_string(what).unwrap());
    match id {
        "new-tab" => match link {
            Some((u, profile)) => {
                tabs::open_tab_in(&APP, &win, Some(u), Some(profile), true);
                to_ui("blur-address"); // a focused bar would keep showing the old tab's text
            }
            None => {
                tabs::open_tab_in(&APP, &win, None, None, true);
                to_ui("focus-address");
            }
        },
        "new-window" => tabs::new_window(&APP, false, None),
        "new-private-window" => tabs::new_window(&APP, true, link.map(|(u, _)| u)),
        "close-tab" => tabs::close_tab(&APP, cur),
        "reopen-tab" => tabs::reopen_closed(&APP),
        "reload" => tabs::reload(&APP, cur),
        "zoom-in" => tabs::zoom(&APP, cur, 1),
        "zoom-out" => tabs::zoom(&APP, cur, -1),
        "zoom-reset" => tabs::zoom(&APP, cur, 0),
        "print" | "save-page" | "reader" => tabs::page_action(&APP, cur, id),
        "back" => tabs::go_history(&APP, cur, -1),
        "forward" => tabs::go_history(&APP, cur, 1),
        "next-tab" => tabs::cycle(&APP, 1),
        "prev-tab" => tabs::cycle(&APP, -1),
        "devtools" => {
            if let Some(l) = tabs::active_view(&APP, cur) {
                engine::devtools(&l);
            }
        }
        "view-source" => {
            if let Some(u) = tabs::active_url(&APP, cur).and_then(|u| url::Url::parse(&format!("view-source:{u}")).ok()) {
                tabs::open_tab_in(&APP, &win, Some(u), None, true);
            }
        }
        "fill-login" => tabs::fill_login(&APP, cur, (200, 100)),
        "bookmark" => tabs::toggle_bookmark(&APP, cur),
        "fullscreen" => engine::toggle_fullscreen(&win),
        "minimize" => engine::minimize(&win),
        "maximize" => engine::maximize_toggle(&win),
        "close-window" => engine::close_window(&win),
        "quit" => engine::quit(),
        "help-site" => tabs::open_external("https://github.com/LorenzoMarchisio05/Aditus"),
        "help-issue" => tabs::open_external("https://github.com/LorenzoMarchisio05/Aditus/issues/new"),
        other if other.starts_with("login:") => tabs::fill_login_as(&APP, cur, &other[6..]),
        other if other.starts_with("select-tab:") => tabs::select(&APP, other[11..].parse().unwrap_or(1)),
        other => to_ui(other),
    }
}

/// Menu id for a key press, for platforms without a native menu bar (the primary modifier is Ctrl).
/// `modifiers` are CEF event flags: shift 2, ctrl 4, alt 8. `vk` is the Windows key code.
pub fn shortcut(modifiers: u32, vk: i32) -> Option<String> {
    let (shift, ctrl, alt) = (modifiers & 2 != 0, modifiers & 4 != 0, modifiers & 8 != 0);
    let ch = if (0x41..=0x5A).contains(&vk) { (vk as u8 as char).to_ascii_lowercase() } else { '\0' };
    let id = match (ctrl, shift, alt, vk, ch) {
        (_, _, _, 0x7B, _) => "devtools",   // F12
        (_, _, _, 0x7A, _) => "fullscreen", // F11
        (false, false, true, 0x25, _) => "back",
        (false, false, true, 0x27, _) => "forward",
        (true, false, false, 9, _) => "next-tab",
        (true, true, false, 9, _) => "prev-tab",
        (true, false, false, 0xBB, _) => "zoom-in",
        (true, false, false, 0xBD, _) => "zoom-out",
        (true, false, false, 0x30, _) => "zoom-reset",
        (true, false, false, 0xBC, _) => "settings",
        (true, false, false, 0xDC, _) => "toggle-sidebar",
        (true, false, false, 0xDB, _) => "back",
        (true, false, false, 0xDD, _) => "forward",
        (true, false, false, 0x31..=0x39, _) => return Some(format!("select-tab:{}", vk - 0x30)),
        (true, false, false, _, 't') => "new-tab",
        (true, false, false, _, 'n') => "new-window",
        (true, true, false, _, 'n') => "new-private-window",
        (true, false, false, _, 'w') => "close-tab",
        (true, true, false, _, 't') => "reopen-tab",
        (true, false, false, _, 'l') => "focus-address",
        (true, true, false, _, 'l') => "toggle-topbar",
        (true, false, false, _, 's') => "save-page",
        (true, false, false, _, 'p') => "print",
        (true, false, false, _, 'f') => "find",
        (true, false, false, _, 'r') => "reload",
        (true, false, false, _, 'y') => "history",
        (true, false, false, _, 'd') => "bookmark",
        (true, false, true, _, 'b') => "bookmarks",
        (true, true, false, _, 'j') => "downloads",
        (true, true, false, _, 'a') => "search-tabs",
        (true, false, true, _, 'i') => "devtools",
        (true, false, true, _, 'r') => "reader",
        (true, false, true, _, 'u') => "view-source",
        (true, false, true, _, 'f') => "fill-login",
        (true, false, true, _, 'p') => "pin-active",
        (true, false, true, _, 'w') => "close-others",
        (true, true, false, _, 'w') => "close-window",
        _ => return None,
    };
    Some(id.into())
}

#[cfg(test)]
mod tests {
    use super::shortcut;

    #[test]
    fn shortcuts() {
        assert_eq!(shortcut(4, b'T' as i32).as_deref(), Some("new-tab"));
        assert_eq!(shortcut(4 | 2, b'N' as i32).as_deref(), Some("new-private-window"));
        assert_eq!(shortcut(4, 0x33).as_deref(), Some("select-tab:3"));
        assert_eq!(shortcut(4 | 8, b'R' as i32).as_deref(), Some("reader"));
        assert_eq!(shortcut(0, 0x7B).as_deref(), Some("devtools"));
        assert_eq!(shortcut(4, b'D' as i32).as_deref(), Some("bookmark"));
        assert_eq!(shortcut(4 | 8, b'B' as i32).as_deref(), Some("bookmarks"));
        assert_eq!(shortcut(0, b'T' as i32), None);
    }
}

#[cfg(target_os = "macos")]
pub fn install() {
    use muda::{accelerator::Accelerator, Menu, MenuEvent, MenuItem, PredefinedMenuItem as P, Submenu};

    let item = |id: &str, text: &str, acc: &str| MenuItem::with_id(id, text, true, acc.parse::<Accelerator>().ok());
    let sep = || P::separator();
    let menu = Menu::new();
    let build = || -> muda::Result<()> {
        let aditus = Submenu::with_items("Aditus", true, &[&P::about(None, None), &sep(), &P::hide(None), &item("quit", "Quit Aditus", "CmdOrCtrl+Q")])?;
        let file = Submenu::with_items(
            "File",
            true,
            &[
                &item("new-tab", "New Tab", "CmdOrCtrl+T"),
                &item("new-window", "New Window", "CmdOrCtrl+N"),
                &item("new-private-window", "New Private Window", "CmdOrCtrl+Shift+N"),
                &item("close-tab", "Close Tab", "CmdOrCtrl+W"),
                &item("reopen-tab", "Reopen Closed Tab", "CmdOrCtrl+Shift+T"),
                &item("focus-address", "Open Location", "CmdOrCtrl+L"),
                &item("save-page", "Save Page As…", "CmdOrCtrl+S"),
                &item("print", "Print…", "CmdOrCtrl+P"),
                &item("find", "Find in Page", "CmdOrCtrl+F"),
                &item("fill-login", "Fill Saved Password", "CmdOrCtrl+Alt+F"),
            ],
        )?;
        let edit = Submenu::with_items("Edit", true, &[&P::undo(None), &P::redo(None), &sep(), &P::cut(None), &P::copy(None), &P::paste(None), &P::select_all(None)])?;
        let view = Submenu::with_items(
            "View",
            true,
            &[
                &item("reload", "Reload", "CmdOrCtrl+R"),
                &item("reader", "Reader View", "CmdOrCtrl+Alt+R"),
                &item("view-source", "View Page Source", "CmdOrCtrl+Alt+U"),
                &item("translate", "Translate Page…", ""),
                &sep(),
                &item("zoom-in", "Zoom In", "CmdOrCtrl+="),
                &item("zoom-out", "Zoom Out", "CmdOrCtrl+-"),
                &item("zoom-reset", "Actual Size", "CmdOrCtrl+0"),
                &sep(),
                &item("toggle-sidebar", "Toggle Sidebar", "CmdOrCtrl+\\"),
                &item("toggle-topbar", "Toggle Top Bar", "CmdOrCtrl+Shift+L"),
                &sep(),
                &item("devtools", "Developer Tools", "CmdOrCtrl+Alt+I"),
                &item("fullscreen", "Enter Full Screen", "Ctrl+CmdOrCtrl+F"),
            ],
        )?;
        let go = Submenu::with_items(
            "Go",
            true,
            &[
                &item("back", "Back", "CmdOrCtrl+["),
                &item("forward", "Forward", "CmdOrCtrl+]"),
                &sep(),
                &item("bookmark", "Bookmark This Page", "CmdOrCtrl+D"),
                &item("bookmarks", "Bookmarks", "CmdOrCtrl+Alt+B"),
                &item("history", "History", "CmdOrCtrl+Y"),
                &item("downloads", "Downloads", "CmdOrCtrl+Shift+J"),
                &item("settings", "Settings", "CmdOrCtrl+,"),
            ],
        )?;
        let tabs_menu = Submenu::new("Tabs", true);
        for it in [item("next-tab", "Next Tab", "Ctrl+Tab"), item("prev-tab", "Previous Tab", "Ctrl+Shift+Tab"), item("search-tabs", "Search Tabs", "CmdOrCtrl+Shift+A")] {
            tabs_menu.append(&it)?;
        }
        tabs_menu.append(&sep())?;
        tabs_menu.append(&item("pin-active", "Pin or Unpin Tab", "CmdOrCtrl+Alt+P"))?;
        tabs_menu.append(&item("close-others", "Close Other Tabs", "CmdOrCtrl+Alt+W"))?;
        tabs_menu.append(&sep())?;
        for n in 1..=9 {
            let text = if n == 9 { "Last Tab".to_string() } else { format!("Tab {n}") };
            tabs_menu.append(&item(&format!("select-tab:{n}"), &text, &format!("CmdOrCtrl+{n}")))?;
        }
        let window =
            Submenu::with_items("Window", true, &[&item("minimize", "Minimize", "CmdOrCtrl+M"), &item("maximize", "Zoom", ""), &sep(), &item("close-window", "Close Window", "CmdOrCtrl+Shift+W")])?;
        window.set_as_windows_menu_for_nsapp();
        let help = Submenu::with_items("Help", true, &[&item("help-site", "Aditus on GitHub", ""), &item("help-issue", "Report an Issue", "")])?;
        help.set_as_help_menu_for_nsapp();
        menu.append_items(&[&aditus, &file, &edit, &view, &go, &tabs_menu, &window, &help])
    };
    if let Err(e) = build() {
        return eprintln!("menu: {e}");
    }
    menu.init_for_nsapp();
    // Keep the menu alive for the life of the process.
    std::mem::forget(menu);
    // Picks arrive on the main thread already, but the handler is `Send`: hop through the task queue.
    MenuEvent::set_event_handler(Some(|ev: MenuEvent| {
        let id = ev.id.0;
        engine::defer_from_thread(move || picked(&id));
    }));
}

#[cfg(not(target_os = "macos"))]
pub fn install() {
    // No native menu bar on Windows / Linux: shortcuts come from the keyboard handler (`shortcut`).
}
