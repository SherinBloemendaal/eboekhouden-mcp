//! Secrets the app keeps in the system credential store (macOS Keychain,
//! Windows Credential Manager). Never in the app's own files.

use rand::RngCore;

const SERVICE: &str = "EB MCP";
const TOKEN_ACCOUNT: &str = "mcp-token";

/// Where secrets live. The real app uses the system store; tests use memory.
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

/// Development builds only: the MCP token in a file (mode 0600) instead of the
/// Keychain. Every `tauri dev` rebuild is a new unsigned binary to macOS, which
/// then asks for Keychain access again at each start. Release builds are signed
/// and keep the token in the Keychain. Account passwords never go here.
#[cfg(debug_assertions)]
pub struct DevFileStore(pub std::path::PathBuf);

#[cfg(debug_assertions)]
impl SecretStore for DevFileStore {
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
        map.remove(account);
        self.write(&map)
    }
}

#[cfg(debug_assertions)]
impl DevFileStore {
    fn read(&self) -> Result<std::collections::BTreeMap<String, String>, String> {
        match std::fs::read_to_string(&self.0) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| e.to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Default::default()),
            Err(e) => Err(e.to_string()),
        }
    }

    fn write(&self, map: &std::collections::BTreeMap<String, String>) -> Result<(), String> {
        if let Some(dir) = self.0.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(
            &self.0,
            serde_json::to_string(map).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| e.to_string())?;
        }
        Ok(())
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

    #[test]
    fn the_dev_file_store_keeps_the_token_private() {
        let path = std::env::temp_dir().join(format!("ebmcp-dev-{}.json", uuid::Uuid::new_v4()));
        let store = DevFileStore(path.clone());
        let token = load_or_create_token(&store).unwrap();
        assert_eq!(load_or_create_token(&store).unwrap(), token);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn shows_only_the_tail() {
        assert_eq!(token_tail("abcdef0123"), "0123");
        assert_eq!(token_tail("ab"), "ab");
    }
}
