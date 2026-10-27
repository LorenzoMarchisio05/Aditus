//! Hand-editable TOML config: profiles, host mappings, settings, ad-block allowlist.

use crate::nav::Mapping;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    /// Stable id for the engine-side data store (macOS) / directory name (Windows, Linux).
    pub id: String,
    /// "#rrggbb"; None = automatic (neutral for the first profile, a hash hue for the rest).
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)] // a hand-edited file missing a field keeps the default instead of failing the parse
pub struct Settings {
    pub search_template: String,
    pub idle_minutes: u64,
    /// Sleep the least recently used background tabs while the app's processes use more than this many MB; 0 = off.
    #[serde(default = "default_memory_limit")]
    pub memory_limit_mb: u64,
    pub sidebar_collapsed: bool,
    pub topbar_visible: bool,
    pub adblock: bool,
    /// "system" | "dark" | "light"
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_sidebar")]
    pub sidebar_width: u32,
    /// Where DevTools sit: "bottom" | "right" | "left" | "top" | "window" (a separate window).
    #[serde(default = "default_devtools_dock")]
    pub devtools_dock: String,
    /// Drag the sidebar edge to resize it.
    #[serde(default = "default_true")]
    pub sidebar_resizable: bool,
    /// Filter list names (see adblock::LISTS) switched off.
    #[serde(default)]
    pub disabled_lists: Vec<String>,
    /// "restore" | "newtab" | "home"
    #[serde(default = "default_startup")]
    pub startup: String,
    #[serde(default)]
    pub home_url: String,
    /// Empty = the system Downloads folder.
    #[serde(default)]
    pub download_dir: String,
    /// Delete history older than this many days; 0 keeps everything.
    #[serde(default)]
    pub history_days: u32,
    #[serde(default)]
    pub clear_history_on_quit: bool,
    /// "amber" | "blue" | "green" | "pink" | "violet"
    #[serde(default = "default_accent")]
    pub accent: String,
    #[serde(default)]
    pub compact_tabs: bool,
    /// Links that open a new tab load behind the current one.
    #[serde(default)]
    pub links_background: bool,
    #[serde(default = "yes")]
    pub auto_update: bool,
    /// Ask the search engine for suggestions while typing in the address bar (not in private windows).
    #[serde(default)]
    pub search_suggestions: bool,
    /// Page zoom in percent.
    #[serde(default = "default_zoom")]
    pub page_zoom: u32,
}

fn default_startup() -> String {
    "restore".into()
}

fn default_accent() -> String {
    "amber".into()
}

fn default_memory_limit() -> u64 {
    4096
}

fn yes() -> bool {
    true
}

fn default_zoom() -> u32 {
    100
}

fn default_devtools_dock() -> String {
    "bottom".into()
}

fn default_true() -> bool {
    true
}

fn default_theme() -> String {
    "system".into()
}

fn default_sidebar() -> u32 {
    290
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            search_template: crate::search::DDG.into(),
            idle_minutes: 10,
            memory_limit_mb: default_memory_limit(),
            sidebar_collapsed: false,
            topbar_visible: true,
            adblock: true,
            theme: default_theme(),
            sidebar_width: default_sidebar(),
            sidebar_resizable: true,
            devtools_dock: default_devtools_dock(),
            disabled_lists: vec![],
            startup: default_startup(),
            home_url: String::new(),
            download_dir: String::new(),
            history_days: 0,
            clear_history_on_quit: false,
            accent: default_accent(),
            compact_tabs: false,
            links_background: false,
            auto_update: true,
            search_suggestions: false,
            page_zoom: default_zoom(),
        }
    }
}

/// A remembered answer to a site asking for camera, microphone, location or notifications.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PermRule {
    pub host: String,
    pub kind: String,
    pub allow: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub settings: Settings,
    pub profiles: Vec<Profile>,
    #[serde(default)]
    pub mappings: Vec<Mapping>,
    #[serde(default)]
    pub allowlist: Vec<String>,
    #[serde(default)]
    pub permissions: Vec<PermRule>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            settings: Settings::default(),
            profiles: vec![Profile { name: "Default".into(), id: uuid::Uuid::new_v4().to_string(), color: None }],
            mappings: vec![],
            allowlist: vec![],
            permissions: vec![],
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Self {
        let Ok(s) = std::fs::read_to_string(path) else { return Self::default() };
        match toml::from_str(&s).ok().filter(|c: &Config| !c.profiles.is_empty()) {
            Some(c) => c,
            None => {
                // Keep the broken file so a typo doesn't cost the user their profiles.
                let _ = std::fs::rename(path, path.with_extension("toml.bak"));
                Self::default()
            }
        }
    }

    pub fn save(&self, path: &Path) {
        if let Ok(s) = toml::to_string_pretty(self) {
            let _ = std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")));
            let _ = write_atomic(path, s);
        }
    }

    pub fn profile(&self, name: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.name == name)
    }

    pub fn data_dir(base: &Path, p: &Profile) -> PathBuf {
        base.join(format!("p-{}", p.id))
    }
}

/// Write to a sibling temp file, then rename over `path`, so a crash never leaves a half-written file.
pub fn write_atomic(path: &Path, data: impl AsRef<[u8]>) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)
}

pub fn store_identifier(p: &Profile) -> [u8; 16] {
    uuid::Uuid::parse_str(&p.id).map(|u| *u.as_bytes()).unwrap_or([0; 16])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_config_is_backed_up_and_atomic_write() {
        let d = std::env::temp_dir().join(format!("aditus-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("config.toml");
        std::fs::write(&p, "settings = [oops").unwrap();
        assert_eq!(Config::load(&p).profiles.len(), 1);
        assert_eq!(std::fs::read_to_string(d.join("config.toml.bak")).unwrap(), "settings = [oops");
        write_atomic(&p, "a").unwrap();
        write_atomic(&p, "b").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "b");
        let c: Config = toml::from_str("profiles = [{ name = \"W\", id = \"x\" }]\n[settings]\nadblock = false").unwrap();
        assert!(!c.settings.adblock && c.settings.idle_minutes == 10);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn suggestions_off_by_default() {
        assert!(!Config::default().settings.search_suggestions);
        let old = "[settings]\nsearch_template='x'\nidle_minutes=1\nsidebar_collapsed=false\ntopbar_visible=true\nadblock=true\nsearch_suggestions=true\n[[profiles]]\nname='a'\nid='b'\n";
        assert!(toml::from_str::<Config>(old).unwrap().settings.search_suggestions);
    }

    #[test]
    fn roundtrip_keeps_profile_id() {
        let c = Config::default();
        let s = toml::to_string_pretty(&c).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back.profiles[0].id, c.profiles[0].id);
        assert_eq!(store_identifier(&back.profiles[0]), store_identifier(&c.profiles[0]));
    }
}
