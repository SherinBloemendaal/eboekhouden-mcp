//! Small app settings on disk (no secrets): the idle sign-out time, the appearance and the Dock icon.
//! Opening at login is not kept here: the system's own login item is the truth (lib.rs, autostart).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

pub const IDLE_CHOICES: [u32; 4] = [15, 30, 60, 240];

/// Systeem, Licht or Donker (Instellingen, Cmd+,).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Minutes without an AI call before an account is signed out.
    pub idle_minutes: u32,
    pub appearance: Appearance,
    /// macOS: an icon in the Dock as well as in the menu bar. On by default (Sherin, 2026-11-10).
    pub show_in_dock: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            idle_minutes: 30,
            appearance: Appearance::System,
            show_in_dock: true,
        }
    }
}

pub struct SettingsStore {
    path: PathBuf,
    value: Mutex<Settings>,
}

impl SettingsStore {
    pub fn open(dir: &Path) -> Self {
        let path = dir.join("settings.json");
        let value = fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Self {
            path,
            value: Mutex::new(value),
        }
    }

    pub fn get(&self) -> Settings {
        self.value.lock().unwrap().clone()
    }

    pub fn set(&self, settings: Settings) -> Result<(), String> {
        if !IDLE_CHOICES.contains(&settings.idle_minutes) {
            return Err("Kies 15, 30, 60 of 240 minuten".into());
        }
        let text = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, text).map_err(|e| e.to_string())?;
        fs::rename(&tmp, &self.path).map_err(|e| e.to_string())?;
        *self.value.lock().unwrap() = settings;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_a_valid_idle_time_only() {
        let dir = std::env::temp_dir().join(format!("ebmcp-settings-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let store = SettingsStore::open(&dir);
        assert_eq!(store.get().idle_minutes, 30);
        assert!(
            store
                .set(Settings {
                    idle_minutes: 7,
                    ..Settings::default()
                })
                .is_err()
        );
        store
            .set(Settings {
                idle_minutes: 60,
                appearance: Appearance::Dark,
                show_in_dock: true,
            })
            .unwrap();
        assert_eq!(SettingsStore::open(&dir).get().idle_minutes, 60);
        assert_eq!(SettingsStore::open(&dir).get().appearance, Appearance::Dark);
        assert!(SettingsStore::open(&dir).get().show_in_dock);
    }
}

#[cfg(test)]
mod older_files {
    use super::*;

    #[test]
    fn a_file_from_before_the_appearance_setting_still_loads() {
        let settings: Settings = serde_json::from_str(r#"{"idleMinutes": 15}"#).unwrap();
        assert_eq!(
            settings,
            Settings {
                idle_minutes: 15,
                appearance: Appearance::System,
                show_in_dock: true,
            }
        );
    }

    #[test]
    fn keeps_the_dock_choice() {
        let settings: Settings =
            serde_json::from_str(r#"{"idleMinutes": 30, "showInDock": true}"#).unwrap();
        assert!(settings.show_in_dock);
    }
}
