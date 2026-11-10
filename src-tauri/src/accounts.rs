//! The account list on disk: label, username or 1Password reference,
//! administration and which store holds the password. Never a password.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

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
    pub is_default: bool,
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
    pub is_default: bool,
}

pub struct AccountStore {
    path: PathBuf,
    accounts: Mutex<Vec<Account>>,
}

impl AccountStore {
    pub fn open(dir: &Path) -> Result<Self, String> {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let path = dir.join("accounts.json");
        let accounts = match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|e| format!("accounts.json is beschadigd: {e}"))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.to_string()),
        };
        Ok(Self {
            path,
            accounts: Mutex::new(accounts),
        })
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

    pub fn default_account(&self) -> Option<Account> {
        let accounts = self.accounts.lock().unwrap();
        accounts
            .iter()
            .find(|a| a.is_default)
            .or(accounts.first())
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
            is_default: input.is_default || accounts.is_empty(),
        };
        if account.is_default {
            accounts.iter_mut().for_each(|a| a.is_default = false);
        }
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
        let updated = account.clone();
        if input.is_default {
            accounts.iter_mut().for_each(|a| a.is_default = a.id == id);
        }
        let result = accounts
            .iter()
            .find(|a| a.id == id)
            .cloned()
            .unwrap_or(updated);
        self.save(&accounts)?;
        Ok(result)
    }

    pub fn set_administration(&self, id: Uuid, administration: &str) -> Result<(), String> {
        let mut accounts = self.accounts.lock().unwrap();
        if let Some(account) = accounts.iter_mut().find(|a| a.id == id) {
            account.administration = Some(administration.to_owned());
        }
        self.save(&accounts)
    }

    pub fn set_default(&self, id: Uuid) -> Result<(), String> {
        let mut accounts = self.accounts.lock().unwrap();
        if !accounts.iter().any(|a| a.id == id) {
            return Err("Dit account bestaat niet meer".into());
        }
        accounts.iter_mut().for_each(|a| a.is_default = a.id == id);
        self.save(&accounts)
    }

    pub fn remove(&self, id: Uuid) -> Result<Account, String> {
        let mut accounts = self.accounts.lock().unwrap();
        let index = accounts
            .iter()
            .position(|a| a.id == id)
            .ok_or("Dit account bestaat niet meer")?;
        let removed = accounts.remove(index);
        if removed.is_default
            && let Some(first) = accounts.first_mut()
        {
            first.is_default = true;
        }
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
            is_default: false,
        }
    }

    #[test]
    fn keeps_accounts_across_restarts_and_never_a_password() {
        let dir = dir();
        let store = AccountStore::open(&dir).unwrap();
        let added = store.add(system("Atelier Zuid")).unwrap();
        assert!(added.is_default, "the first account becomes the default");
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
            is_default: false,
        };
        assert!(store.add(bad_ref).is_err());
        let good_ref = AccountInput {
            label: "B".into(),
            username: String::new(),
            store: Store::OnePassword,
            op_ref: Some(" op://Werk/e-Boekhouden Studio Noord ".into()),
            op_title: Some(" e-Boekhouden Studio Noord ".into()),
            op_vault: Some("  ".into()),
            is_default: false,
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
            is_default: false,
        };
        assert!(store.add(by_id).is_ok());
        // Accounts written before these fields existed still load.
        let old: Account = serde_json::from_str(
            r#"{"id":"00000000-0000-0000-0000-000000000001","label":"Oud","username":"","store":"onePassword","opRef":"op://Werk/EB","administration":null,"isDefault":false}"#,
        )
        .unwrap();
        assert_eq!(old.op_title, None);
    }

    #[test]
    fn one_default_at_a_time() {
        let store = AccountStore::open(&dir()).unwrap();
        let a = store.add(system("A")).unwrap();
        let mut input = system("B");
        input.is_default = true;
        let b = store.add(input).unwrap();
        assert_eq!(store.default_account().unwrap().id, b.id);
        store.set_default(a.id).unwrap();
        assert_eq!(store.list().iter().filter(|x| x.is_default).count(), 1);
        store.remove(a.id).unwrap();
        assert!(
            store.get(b.id).unwrap().is_default,
            "removing the default hands it on"
        );
    }

    #[test]
    fn item_references() {
        assert!(is_item_reference("op://Werk/e-Boekhouden"));
        assert!(!is_item_reference("op://Werk/item/password"));
        assert!(!is_item_reference("Werk/item"));
        assert!(!is_item_reference("op:///item"));
    }
}
