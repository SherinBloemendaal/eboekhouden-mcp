//! Small app settings on disk (no secrets): the idle sign-out time, staying signed in, the appearance, the
//! Dock icon and whether a tool call waits for the user's approval.
//! Opening at login is not kept here: the system's own login item is the truth (lib.rs, autostart).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

pub const IDLE_CHOICES: [u32; 4] = [15, 30, 60, 240];
/// How many minutes a tool call may wait for the user's approval (Instellingen, Akkoord).
pub const WAIT_CHOICES: [u32; 4] = [1, 2, 5, 10];

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
    /// Minutes without an AI call before an account is signed out. Not used while `keep_signed_in` is on.
    pub idle_minutes: u32,
    /// Blijf ingelogd: no idle sign-out, and EB MCP asks e-Boekhouden every few minutes whether each session
    /// still stands, so it does not lapse there for lack of use. Off unless the user turns it on: a session
    /// that stays open is a session an AI client can use at any time.
    pub keep_signed_in: bool,
    pub appearance: Appearance,
    /// macOS: an icon in the Dock as well as in the menu bar. On by default (Sherin, 2026-11-10).
    pub show_in_dock: bool,
    /// Wacht op akkoord in de aanroep: a tool whose change needs approval keeps its MCP call open until the user
    /// answered (or `wait_minutes` passed) and answers with the outcome, so the AI client carries on by itself.
    /// Off: the call answers at once with status wacht, as before. On by default (Dion, 2026-11-24).
    pub wait_for_approval: bool,
    /// How long a call waits at most before it answers with status wacht after all. One of `WAIT_CHOICES`.
    pub wait_minutes: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            idle_minutes: 30,
            keep_signed_in: false,
            appearance: Appearance::System,
            show_in_dock: true,
            wait_for_approval: true,
            wait_minutes: 5,
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
        if !WAIT_CHOICES.contains(&settings.wait_minutes) {
            return Err("Kies 1, 2, 5 of 10 minuten".into());
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
                keep_signed_in: true,
                appearance: Appearance::Dark,
                show_in_dock: true,
                wait_for_approval: false,
                wait_minutes: 2,
            })
            .unwrap();
        assert!(!SettingsStore::open(&dir).get().wait_for_approval);
        assert_eq!(SettingsStore::open(&dir).get().wait_minutes, 2);
        assert!(SettingsStore::open(&dir).get().keep_signed_in);
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
                // Staying signed in is a choice: a file from before it has it off.
                keep_signed_in: false,
                appearance: Appearance::System,
                show_in_dock: true,
                // Waiting for approval is new and on: a file from before it waits too.
                wait_for_approval: true,
                wait_minutes: 5,
            }
        );
        let kept: Settings = serde_json::from_str(r#"{"keepSignedIn": true}"#).unwrap();
        assert!(kept.keep_signed_in);
    }

    #[test]
    fn keeps_a_valid_wait_only() {
        let dir = std::env::temp_dir().join(format!("ebmcp-settings-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let store = SettingsStore::open(&dir);
        assert!(store.get().wait_for_approval);
        assert_eq!(store.get().wait_minutes, 5);
        assert!(
            store
                .set(Settings {
                    wait_minutes: 3,
                    ..Settings::default()
                })
                .is_err()
        );
        let off: Settings =
            serde_json::from_str(r#"{"waitForApproval": false, "waitMinutes": 10}"#).unwrap();
        assert!(!off.wait_for_approval);
        assert_eq!(off.wait_minutes, 10);
    }

    #[test]
    fn keeps_the_dock_choice() {
        for show in [true, false] {
            let text = serde_json::to_string(&Settings {
                show_in_dock: show,
                ..Settings::default()
            })
            .unwrap();
            let settings: Settings = serde_json::from_str(&text).unwrap();
            assert_eq!(settings.show_in_dock, show, "{text}");
        }
        let hidden: Settings =
            serde_json::from_str(r#"{"idleMinutes": 30, "showInDock": false}"#).unwrap();
        assert!(!hidden.show_in_dock);
    }
}
