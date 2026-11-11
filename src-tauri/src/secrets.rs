//! Secrets the app keeps. Account passwords live in the system credential
//! store (macOS Keychain, Windows Credential Manager), never in the app's own
//! files. The MCP token lives in a file only the user can read (`FileStore`),
//! where the bridge can read it too without a Keychain prompt.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rand::RngCore;

const SERVICE: &str = "EB MCP";
const TOKEN_ACCOUNT: &str = "mcp-token";

/// Where secrets live: the system store, a private file, or memory in tests.
pub trait SecretStore: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<String>, String>;
    fn set(&self, account: &str, secret: &str) -> Result<(), String>;
    fn delete(&self, account: &str) -> Result<(), String>;
}

pub struct SystemStore;

impl SecretStore for SystemStore {
    fn get(&self, account: &str) -> Result<Option<String>, String> {
        let entry = keyring::Entry::new(SERVICE, account).map_err(|e| e.to_string())?;
        match entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), String> {
        keyring::Entry::new(SERVICE, account)
            .and_then(|entry| entry.set_password(secret))
            .map_err(|e| e.to_string())
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        let entry = keyring::Entry::new(SERVICE, account).map_err(|e| e.to_string())?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// The MCP token in a file only the user can read (mode 0600 on unix; on
/// Windows the user's AppData ACL protects it). Not in the Keychain: the app is
/// ad-hoc signed, so macOS would ask for the Keychain password at every start
/// after each update, and the bridge would ask again on its own. Account
/// passwords never go here; they stay in `SystemStore`.
pub struct FileStore(pub PathBuf);

/// The older name, from when only development builds used a file.
#[cfg(debug_assertions)]
#[allow(unused_imports)]
pub use self::FileStore as DevFileStore;

impl SecretStore for FileStore {
    fn get(&self, account: &str) -> Result<Option<String>, String> {
        Ok(self.read()?.get(account).cloned())
    }

    fn set(&self, account: &str, secret: &str) -> Result<(), String> {
        let mut map = self.read()?;
        map.insert(account.to_owned(), secret.to_owned());
        self.write(&map)
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        let mut map = self.read()?;
        if map.remove(account).is_none() {
            return Ok(());
        }
        self.write(&map)
    }
}

impl FileStore {
    fn read(&self) -> Result<BTreeMap<String, String>, String> {
        match std::fs::read_to_string(&self.0) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| e.to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Default::default()),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Writes a temp file next to the target and renames it over the target,
    /// so a reader never sees half a file. The temp file is private from the
    /// moment it exists.
    fn write(&self, map: &BTreeMap<String, String>) -> Result<(), String> {
        let dir = self
            .0
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let name = self
            .0
            .file_name()
            .ok_or_else(|| format!("geen bestandsnaam in {}", self.0.display()))?
            .to_string_lossy();
        let temp = dir.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4()));
        let json = serde_json::to_string(map).map_err(|e| e.to_string())?;
        let written = Self::write_private(&temp, json.as_bytes())
            .and_then(|()| std::fs::rename(&temp, &self.0));
        if let Err(e) = written {
            let _ = std::fs::remove_file(&temp);
            return Err(e.to_string());
        }
        Ok(())
    }

    fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path)?;
        #[cfg(unix)]
        {
            // The mode above passes through the umask; make sure it is exact.
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(bytes)?;
        file.sync_all()
    }
}

/// Where the MCP token file lives in the app's config folder. Development
/// builds keep their own file, so a `tauri dev` run never touches the token of
/// the installed app.
pub fn token_file(dir: &Path) -> PathBuf {
    dir.join(if cfg!(debug_assertions) {
        "dev-token.json"
    } else {
        "token.json"
    })
}

/// The store for the MCP token: a `FileStore` on `token_file(dir)`.
pub fn token_store(dir: &Path) -> Box<dyn SecretStore> {
    Box::new(FileStore(token_file(dir)))
}

/// Moves the MCP token of an older version from the Keychain (`old`) to the
/// token file, once, so clients linked with that token keep working. Costs one
/// last Keychain prompt. Does nothing when the file already holds a valid
/// token. Errors are ignored on purpose: `load_or_create_token` then makes a
/// new token, and the clients need linking again. Release builds only call it:
/// development builds never had their token in the Keychain.
#[cfg_attr(debug_assertions, allow(dead_code))]
pub fn migrate_token(file: &dyn SecretStore, old: &dyn SecretStore) {
    if let Ok(Some(token)) = file.get(TOKEN_ACCOUNT)
        && is_valid_token(&token)
    {
        return;
    }
    let Ok(Some(token)) = old.get(TOKEN_ACCOUNT) else {
        return;
    };
    if !is_valid_token(&token) {
        return;
    }
    if file.set(TOKEN_ACCOUNT, &token).is_ok() {
        let _ = old.delete(TOKEN_ACCOUNT);
    }
}

/// The MCP token of this installation: read it, or make and store one the first time.
pub fn load_or_create_token(store: &dyn SecretStore) -> Result<String, String> {
    if let Some(token) = store.get(TOKEN_ACCOUNT)?
        && is_valid_token(&token)
    {
        return Ok(token);
    }
    let token = new_token();
    store.set(TOKEN_ACCOUNT, &token)?;
    Ok(token)
}

/// Replaces the token. Every linked client needs the new one afterwards.
pub fn renew_token(store: &dyn SecretStore) -> Result<String, String> {
    let token = new_token();
    store.set(TOKEN_ACCOUNT, &token)?;
    Ok(token)
}

fn new_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn is_valid_token(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The last four characters, the only part the UI ever shows unasked.
pub fn token_tail(token: &str) -> &str {
    &token[token.len().saturating_sub(4)..]
}

#[cfg(test)]
pub mod memory {
    use super::SecretStore;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct MemoryStore(Mutex<HashMap<String, String>>);

    impl SecretStore for MemoryStore {
        fn get(&self, account: &str) -> Result<Option<String>, String> {
            Ok(self.0.lock().unwrap().get(account).cloned())
        }

        fn set(&self, account: &str, secret: &str) -> Result<(), String> {
            self.0.lock().unwrap().insert(account.into(), secret.into());
            Ok(())
        }

        fn delete(&self, account: &str) -> Result<(), String> {
            self.0.lock().unwrap().remove(account);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::memory::MemoryStore;
    use super::*;

    #[test]
    fn creates_a_token_once_and_keeps_it() {
        let store = MemoryStore::default();
        let first = load_or_create_token(&store).unwrap();
        assert_eq!(first.len(), 64);
        assert!(is_valid_token(&first));
        assert_eq!(load_or_create_token(&store).unwrap(), first);
    }

    #[test]
    fn renewing_gives_a_different_token() {
        let store = MemoryStore::default();
        let old = load_or_create_token(&store).unwrap();
        let new = renew_token(&store).unwrap();
        assert_ne!(old, new);
        assert_eq!(load_or_create_token(&store).unwrap(), new);
    }

    #[test]
    fn replaces_a_damaged_token() {
        let store = MemoryStore::default();
        store.set(TOKEN_ACCOUNT, "kapot").unwrap();
        let token = load_or_create_token(&store).unwrap();
        assert_ne!(token, "kapot");
        assert!(is_valid_token(&token));
    }

    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("ebmcp-{}", uuid::Uuid::new_v4()))
            .join(name)
    }

    fn valid() -> String {
        "ab".repeat(32)
    }

    #[test]
    fn the_file_store_keeps_what_it_is_given() {
        let path = temp_file("token.json");
        let store = FileStore(path.clone());
        assert_eq!(store.get("a").unwrap(), None);
        store.set("a", "1").unwrap();
        store.set("b", "2").unwrap();
        assert_eq!(
            FileStore(path.clone()).get("a").unwrap().as_deref(),
            Some("1")
        );
        store.delete("a").unwrap();
        store.delete("nooit-gezet").unwrap();
        assert_eq!(store.get("a").unwrap(), None);
        assert_eq!(store.get("b").unwrap().as_deref(), Some("2"));
        let token = load_or_create_token(&store).unwrap();
        assert_eq!(load_or_create_token(&store).unwrap(), token);
        // Only the token file is left: no temp files next to it.
        let names: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("token.json")]);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn the_file_store_reports_a_broken_file() {
        let path = temp_file("token.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "geen json").unwrap();
        assert!(FileStore(path.clone()).get(TOKEN_ACCOUNT).is_err());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn the_file_store_keeps_the_token_private() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_file("token.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // A file left with wider rights is replaced by a private one.
        std::fs::write(&path, "{}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let store = FileStore(path.clone());
        load_or_create_token(&store).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn the_token_file_name_follows_the_build() {
        let dir = Path::new("config");
        let expected = if cfg!(debug_assertions) {
            "dev-token.json"
        } else {
            "token.json"
        };
        assert_eq!(token_file(dir), dir.join(expected));
        let base = temp_file("x");
        let dir = base.parent().unwrap();
        let store = token_store(dir);
        let token = load_or_create_token(store.as_ref()).unwrap();
        assert!(
            std::fs::read_to_string(dir.join(expected))
                .unwrap()
                .contains(&token)
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn migration_moves_a_valid_token_out_of_the_old_store() {
        let file = MemoryStore::default();
        let old = MemoryStore::default();
        old.set(TOKEN_ACCOUNT, &valid()).unwrap();
        old.set("ander", "blijft").unwrap();
        migrate_token(&file, &old);
        assert_eq!(file.get(TOKEN_ACCOUNT).unwrap(), Some(valid()));
        assert_eq!(old.get(TOKEN_ACCOUNT).unwrap(), None);
        assert_eq!(old.get("ander").unwrap().as_deref(), Some("blijft"));
        assert_eq!(load_or_create_token(&file).unwrap(), valid());
    }

    #[test]
    fn migration_moves_into_a_real_file_store() {
        let path = temp_file("token.json");
        let file = FileStore(path.clone());
        let old = MemoryStore::default();
        old.set(TOKEN_ACCOUNT, &valid()).unwrap();
        migrate_token(&file, &old);
        assert_eq!(
            FileStore(path.clone()).get(TOKEN_ACCOUNT).unwrap(),
            Some(valid())
        );
        assert_eq!(old.get(TOKEN_ACCOUNT).unwrap(), None);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn migration_leaves_an_existing_file_token_alone() {
        let file = MemoryStore::default();
        let old = MemoryStore::default();
        let current = "cd".repeat(32);
        file.set(TOKEN_ACCOUNT, &current).unwrap();
        old.set(TOKEN_ACCOUNT, &valid()).unwrap();
        migrate_token(&file, &old);
        assert_eq!(file.get(TOKEN_ACCOUNT).unwrap(), Some(current));
        assert_eq!(old.get(TOKEN_ACCOUNT).unwrap(), Some(valid()));
    }

    #[test]
    fn migration_replaces_a_damaged_file_token() {
        let file = MemoryStore::default();
        let old = MemoryStore::default();
        file.set(TOKEN_ACCOUNT, "kapot").unwrap();
        old.set(TOKEN_ACCOUNT, &valid()).unwrap();
        migrate_token(&file, &old);
        assert_eq!(file.get(TOKEN_ACCOUNT).unwrap(), Some(valid()));
        assert_eq!(old.get(TOKEN_ACCOUNT).unwrap(), None);
    }

    #[test]
    fn migration_ignores_a_missing_or_invalid_old_token() {
        let file = MemoryStore::default();
        let old = MemoryStore::default();
        migrate_token(&file, &old);
        assert_eq!(file.get(TOKEN_ACCOUNT).unwrap(), None);

        old.set(TOKEN_ACCOUNT, "kapot").unwrap();
        migrate_token(&file, &old);
        assert_eq!(file.get(TOKEN_ACCOUNT).unwrap(), None);
        assert_eq!(old.get(TOKEN_ACCOUNT).unwrap().as_deref(), Some("kapot"));
    }

    #[test]
    fn migration_ignores_an_old_store_that_fails() {
        struct Failing;
        impl SecretStore for Failing {
            fn get(&self, _: &str) -> Result<Option<String>, String> {
                Err("geweigerd".into())
            }
            fn set(&self, _: &str, _: &str) -> Result<(), String> {
                Err("geweigerd".into())
            }
            fn delete(&self, _: &str) -> Result<(), String> {
                Err("geweigerd".into())
            }
        }
        let file = MemoryStore::default();
        migrate_token(&file, &Failing);
        assert_eq!(file.get(TOKEN_ACCOUNT).unwrap(), None);
        assert!(is_valid_token(&load_or_create_token(&file).unwrap()));
    }

    #[test]
    fn shows_only_the_tail() {
        assert_eq!(token_tail("abcdef0123"), "0123");
        assert_eq!(token_tail("ab"), "ab");
    }
}
