//! Hand-editable TOML config: profiles, host mappings, settings, ad-block allowlist.

use crate::nav::Mapping;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    /// Stable id for the engine-side data store (macOS) / directory name (Windows, Linux).
    pub id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    pub search_template: String,
    pub idle_minutes: u64,
    pub sidebar_collapsed: bool,
    pub topbar_visible: bool,
    pub adblock: bool,
    /// "system" | "dark" | "light"
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_sidebar")]
    pub sidebar_width: u32,
    /// Filter list names (see adblock::LISTS) switched off.
    #[serde(default)]
    pub disabled_lists: Vec<String>,
}

fn default_theme() -> String {
    "system".into()
}

fn default_sidebar() -> u32 {
    236
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            search_template: crate::search::DDG.into(),
            idle_minutes: 15,
            sidebar_collapsed: false,
            topbar_visible: true,
            adblock: true,
            theme: default_theme(),
            sidebar_width: default_sidebar(),
            disabled_lists: vec![],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub settings: Settings,
    pub profiles: Vec<Profile>,
    #[serde(default)]
    pub mappings: Vec<Mapping>,
    #[serde(default)]
    pub allowlist: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            settings: Settings::default(),
            profiles: vec![Profile { name: "Default".into(), id: uuid::Uuid::new_v4().to_string() }],
            mappings: vec![],
            allowlist: vec![],
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .filter(|c: &Config| !c.profiles.is_empty())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) {
        if let Ok(s) = toml::to_string_pretty(self) {
            let _ = std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")));
            let _ = std::fs::write(path, s);
        }
    }

    pub fn profile(&self, name: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.name == name)
    }

    pub fn data_dir(base: &Path, p: &Profile) -> PathBuf {
        base.join("profiles").join(&p.id)
    }
}

pub fn store_identifier(p: &Profile) -> [u8; 16] {
    uuid::Uuid::parse_str(&p.id).map(|u| *u.as_bytes()).unwrap_or([0; 16])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_keeps_profile_id() {
        let c = Config::default();
        let s = toml::to_string_pretty(&c).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(back.profiles[0].id, c.profiles[0].id);
        assert_eq!(store_identifier(&back.profiles[0]), store_identifier(&c.profiles[0]));
    }
}
