//! Secrets the app keeps in the system credential store (macOS Keychain,
//! Windows Credential Manager). Never in the app's own files.

use rand::RngCore;

const SERVICE: &str = "EB MCP";
const TOKEN_ACCOUNT: &str = "mcp-token";

/// Where secrets live. The real app uses the system store; tests use memory.
pub trait SecretStore: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<String>, String>;
    fn set(&self, account: &str, secret: &str) -> Result<(), String>;
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
    fn replaces_a_damaged_token() {
        let store = MemoryStore::default();
        store.set(TOKEN_ACCOUNT, "kapot").unwrap();
        let token = load_or_create_token(&store).unwrap();
        assert_ne!(token, "kapot");
        assert!(is_valid_token(&token));
    }

    #[test]
    fn shows_only_the_tail() {
        assert_eq!(token_tail("abcdef0123"), "0123");
        assert_eq!(token_tail("ab"), "ab");
    }
}
