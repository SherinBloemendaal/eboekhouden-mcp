//! The account list on disk: label, username or 1Password reference,
//! administration and which store holds the password. Never a password.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Store {
    /// Username and password come from a 1Password item through the `op` CLI.
    OnePassword,
    /// The password sits in the system store (Keychain, Credential Manager),
    /// read after Touch ID or Windows Hello.
    System,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: Uuid,
    pub label: String,
    /// The e-Boekhouden username. For 1Password the one read at the last check, for display only.
    pub username: String,
    pub store: Store,
    /// `op://vault/item`, for `Store::OnePassword`.
    pub op_ref: Option<String>,
    /// The 1Password item's title and vault when it was picked from the list, for display only.
    #[serde(default)]
    pub op_title: Option<String>,
    #[serde(default)]
    pub op_vault: Option<String>,
    /// Filled in from e-Boekhouden after a sign-in.
    pub administration: Option<String>,
    /// Whether AI clients may change this account's books. Off unless the user turns it on.
    #[serde(default)]
    pub write: WriteMode,
}

/// What AI clients may do in an account's books besides reading.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WriteMode {
    /// Read only: every tool that changes something is refused.
    #[default]
    Off,
    /// Each change waits for the user's approval in EB MCP.
    Ask,
    /// Changes go through at once. Deleting and crediting still ask.
    Auto,
}

/// What the UI sends to add or change an account.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInput {
    pub label: String,
    pub username: String,
    pub store: Store,
    pub op_ref: Option<String>,
    #[serde(default)]
    pub op_title: Option<String>,
    #[serde(default)]
    pub op_vault: Option<String>,
    #[serde(default)]
    pub write: WriteMode,
}

pub struct AccountStore {
    path: PathBuf,
    accounts: Mutex<Vec<Account>>,
}

/// What `AccountStore::load` found on disk.
enum Loaded {
    Accounts(Vec<Account>),
    /// The file is there but is not an account list; the parse error.
    Damaged(String),
}

impl AccountStore {
    /// Opens the list, and refuses a damaged file. The app opens with `open_or_recover`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn open(dir: &Path) -> Result<Self, String> {
        let path = dir.join("accounts.json");
        match Self::load(dir, &path)? {
            Loaded::Accounts(accounts) => Ok(Self::with(path, accounts)),
            Loaded::Damaged(e) => Err(format!("accounts.json is beschadigd: {e}")),
        }
    }

    /// Opens the list. A file that cannot be read as an account list is kept aside as
    /// `accounts.broken-<unix ms>.json` and the app starts with no accounts, so it still opens.
    /// The second value is the notice for the user that says so.
    pub fn open_or_recover(dir: &Path) -> Result<(Self, Option<String>), String> {
        let path = dir.join("accounts.json");
        match Self::load(dir, &path)? {
            Loaded::Accounts(accounts) => Ok((Self::with(path, accounts), None)),
            Loaded::Damaged(e) => {
                let millis = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0);
                let name = format!("accounts.broken-{millis}.json");
                let kept = dir.join(&name);
                // Without the rename the next save would overwrite the damaged file, so stop instead.
                fs::rename(&path, &kept).map_err(|r| {
                    format!("accounts.json is beschadigd ({e}) en kon niet opzij worden gezet: {r}")
                })?;
                log::warn!(
                    "accounts.json is beschadigd ({e}), bewaard als {}",
                    kept.display()
                );
                let notice = format!(
                    "Je lijst met accounts (accounts.json) was beschadigd. EB MCP heeft hem bewaard als {name} in {} en begint met een lege lijst. Voeg je accounts opnieuw toe.",
                    dir.display()
                );
                Ok((Self::with(path, Vec::new()), Some(notice)))
            }
        }
    }

    fn load(dir: &Path, path: &Path) -> Result<Loaded, String> {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        match fs::read_to_string(path) {
            Ok(text) => Ok(match serde_json::from_str(&text) {
                Ok(accounts) => Loaded::Accounts(accounts),
                Err(e) => Loaded::Damaged(e.to_string()),
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Loaded::Accounts(Vec::new())),
            Err(e) => Err(e.to_string()),
        }
    }

    fn with(path: PathBuf, accounts: Vec<Account>) -> Self {
        Self {
            path,
            accounts: Mutex::new(accounts),
        }
    }

    pub fn list(&self) -> Vec<Account> {
        self.accounts.lock().unwrap().clone()
    }

    pub fn get(&self, id: Uuid) -> Option<Account> {
        self.accounts
            .lock()
            .unwrap()
            .iter()
            .find(|a| a.id == id)
            .cloned()
    }

    pub fn add(&self, input: AccountInput) -> Result<Account, String> {
        let input = validate(input)?;
        let mut accounts = self.accounts.lock().unwrap();
        let account = Account {
            id: Uuid::new_v4(),
            label: input.label,
            username: input.username,
            store: input.store,
            op_ref: input.op_ref,
            op_title: input.op_title,
            op_vault: input.op_vault,
            administration: None,
            write: input.write,
        };
        accounts.push(account.clone());
        self.save(&accounts)?;
        Ok(account)
    }

    pub fn update(&self, id: Uuid, input: AccountInput) -> Result<Account, String> {
        let input = validate(input)?;
        let mut accounts = self.accounts.lock().unwrap();
        let account = accounts
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or("Dit account bestaat niet meer")?;
        account.label = input.label;
        account.username = input.username;
        account.store = input.store;
        account.op_ref = input.op_ref;
        account.op_title = input.op_title;
        account.op_vault = input.op_vault;
        account.write = input.write;
        let updated = account.clone();
        self.save(&accounts)?;
        Ok(updated)
    }

    pub fn set_administration(&self, id: Uuid, administration: &str) -> Result<(), String> {
        let mut accounts = self.accounts.lock().unwrap();
        if let Some(account) = accounts.iter_mut().find(|a| a.id == id) {
            account.administration = Some(administration.to_owned());
        }
        self.save(&accounts)
    }

    pub fn remove(&self, id: Uuid) -> Result<Account, String> {
        let mut accounts = self.accounts.lock().unwrap();
        let index = accounts
            .iter()
            .position(|a| a.id == id)
            .ok_or("Dit account bestaat niet meer")?;
        let removed = accounts.remove(index);
        self.save(&accounts)?;
        Ok(removed)
    }

    /// Writes next to the file and renames, so a crash never leaves half a file.
    fn save(&self, accounts: &[Account]) -> Result<(), String> {
        let text = serde_json::to_string_pretty(accounts).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, text).map_err(|e| e.to_string())?;
        fs::rename(&tmp, &self.path).map_err(|e| e.to_string())
    }
}

fn validate(mut input: AccountInput) -> Result<AccountInput, String> {
    input.label = input.label.trim().to_owned();
    input.username = input.username.trim().to_owned();
    if input.label.is_empty() {
        return Err("Geef het account een naam".into());
    }
    if input.label.chars().count() > 60 {
        return Err("De naam is te lang (hoogstens 60 tekens)".into());
    }
    match input.store {
        Store::OnePassword => {
            let reference = input
                .op_ref
                .as_deref()
                .map(str::trim)
                .unwrap_or_default()
                .to_owned();
            if !is_item_reference(&reference) {
                return Err("Geef een 1Password-item als op://kluis/item".into());
            }
            input.op_ref = Some(reference);
            let clean =
                |v: Option<String>| v.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
            input.op_title = clean(input.op_title);
            input.op_vault = clean(input.op_vault);
        }
        Store::System => {
            if input.username.is_empty() {
                return Err("Vul de gebruikersnaam of het e-mailadres in".into());
            }
            input.op_ref = None;
            input.op_title = None;
            input.op_vault = None;
        }
    }
    Ok(input)
}

/// `op://vault/item`: exactly two non-empty parts, the fields are added when reading.
pub fn is_item_reference(reference: &str) -> bool {
    let Some(rest) = reference.strip_prefix("op://") else {
        return false;
    };
    let parts: Vec<&str> = rest.split('/').collect();
    parts.len() == 2 && parts.iter().all(|p| !p.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ebmcp-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn system(label: &str) -> AccountInput {
        AccountInput {
            label: label.into(),
            username: "lisa@atelierzuid.example".into(),
            store: Store::System,
            op_ref: None,
            op_title: None,
            op_vault: None,
            write: WriteMode::Off,
        }
    }

    #[test]
    fn keeps_accounts_across_restarts_and_never_a_password() {
        let dir = dir();
        let store = AccountStore::open(&dir).unwrap();
        let added = store.add(system("Atelier Zuid")).unwrap();
        let again = AccountStore::open(&dir).unwrap();
        assert_eq!(again.list(), vec![added]);
        let text = fs::read_to_string(dir.join("accounts.json")).unwrap();
        assert!(!text.to_lowercase().contains("password"), "{text}");
    }

    #[test]
    fn checks_the_input() {
        let store = AccountStore::open(&dir()).unwrap();
        assert!(store.add(system("  ")).is_err());
        let mut no_user = system("A");
        no_user.username = " ".into();
        assert!(store.add(no_user).is_err());
        let bad_ref = AccountInput {
            label: "B".into(),
            username: String::new(),
            store: Store::OnePassword,
            op_ref: Some("op://Werk".into()),
            op_title: None,
            op_vault: None,
            write: WriteMode::Off,
        };
        assert!(store.add(bad_ref).is_err());
        let good_ref = AccountInput {
            label: "B".into(),
            username: String::new(),
            store: Store::OnePassword,
            op_ref: Some(" op://Werk/e-Boekhouden Studio Noord ".into()),
            op_title: Some(" e-Boekhouden Studio Noord ".into()),
            op_vault: Some("  ".into()),
            write: WriteMode::Off,
        };
        let added = store.add(good_ref).unwrap();
        assert_eq!(
            added.op_ref.as_deref(),
            Some("op://Werk/e-Boekhouden Studio Noord")
        );
        assert_eq!(added.op_title.as_deref(), Some("e-Boekhouden Studio Noord"));
        assert_eq!(added.op_vault, None);
        // A picked item is stored by its ids, which survive a rename in 1Password.
        let by_id = AccountInput {
            label: "C".into(),
            username: String::new(),
            store: Store::OnePassword,
            op_ref: Some("op://abcvault123/xyzitem456".into()),
            op_title: Some("e-Boekhouden".into()),
            op_vault: Some("Werk".into()),
            write: WriteMode::Off,
        };
        assert!(store.add(by_id).is_ok());
        // Accounts written before these fields existed, and with the default mark of 0.1.12 and older, still load.
        let old: Account = serde_json::from_str(
            r#"{"id":"00000000-0000-0000-0000-000000000001","label":"Oud","username":"","store":"onePassword","opRef":"op://Werk/EB","administration":null,"isDefault":false}"#,
        )
        .unwrap();
        assert_eq!(old.op_title, None);
    }

    #[test]
    fn a_damaged_file_is_kept_aside_and_the_app_starts_empty() {
        let dir = dir();
        fs::write(dir.join("accounts.json"), "[{\"label\": kapot").unwrap();
        assert!(
            AccountStore::open(&dir).is_err(),
            "plain open still refuses it"
        );

        let (store, notice) = AccountStore::open_or_recover(&dir).unwrap();
        assert!(store.list().is_empty());
        let kept: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.starts_with("accounts.broken-") && n.ends_with(".json"))
            .collect();
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert_eq!(
            fs::read_to_string(dir.join(&kept[0])).unwrap(),
            "[{\"label\": kapot",
            "the damaged file is kept as it was"
        );
        let notice = notice.expect("the user hears about it");
        assert!(notice.contains(&kept[0]), "{notice}");
        assert!(!dir.join("accounts.json").exists());

        // The new list saves and opens again without a notice.
        let added = store.add(system("Atelier Zuid")).unwrap();
        let (again, notice) = AccountStore::open_or_recover(&dir).unwrap();
        assert_eq!(again.list(), vec![added]);
        assert_eq!(notice, None);
    }

    #[test]
    fn a_missing_file_is_an_empty_list_without_a_notice() {
        let (store, notice) = AccountStore::open_or_recover(&dir()).unwrap();
        assert!(store.list().is_empty());
        assert_eq!(notice, None);
    }

    #[test]
    fn item_references() {
        assert!(is_item_reference("op://Werk/e-Boekhouden"));
        assert!(!is_item_reference("op://Werk/item/password"));
        assert!(!is_item_reference("Werk/item"));
        assert!(!is_item_reference("op:///item"));
    }
}
