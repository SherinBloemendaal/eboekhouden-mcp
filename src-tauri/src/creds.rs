//! Reading an account's credentials at sign-in, and only then.
//!
//! - 1Password: `op read` on the item's username and password fields. The
//!   1Password app asks the user itself.
//! - System store: the password from the Keychain or Credential Manager, after
//!   Touch ID or Windows Hello confirms it is the user.

use std::path::PathBuf;
use std::process::Command;

use uuid::Uuid;

use crate::accounts::{Account, Store};
use crate::secrets::{SecretStore, SystemStore};

pub struct Credentials {
    pub username: String,
    pub password: String,
}

/// Why credentials could not be read; the message is shown as is.
#[derive(Debug, PartialEq)]
pub struct CredError(pub String);

fn password_key(id: Uuid) -> String {
    format!("account:{id}")
}

pub fn save_password(id: Uuid, password: &str) -> Result<(), String> {
    if password.is_empty() {
        return Err("Vul het wachtwoord in".into());
    }
    SystemStore.set(&password_key(id), password)
}

pub fn delete_password(id: Uuid) -> Result<(), String> {
    SystemStore.delete(&password_key(id))
}

pub async fn read(account: &Account) -> Result<Credentials, CredError> {
    match account.store {
        Store::OnePassword => {
            let reference = account
                .op_ref
                .clone()
                .ok_or_else(|| CredError("Geen 1Password-item ingesteld".into()))?;
            tokio::task::spawn_blocking(move || read_1password(&reference))
                .await
                .map_err(|e| CredError(e.to_string()))?
        }
        Store::System => {
            confirm_user(&format!(
                "het wachtwoord van {} te gebruiken",
                account.label
            ))
            .await?;
            let password = SystemStore
                .get(&password_key(account.id))
                .map_err(CredError)?
                .ok_or_else(|| {
                    CredError(
                        "Geen wachtwoord in de sleutelhanger. Vul het opnieuw in bij Bewerken."
                            .into(),
                    )
                })?;
            Ok(Credentials {
                username: account.username.clone(),
                password,
            })
        }
    }
}

fn read_1password(reference: &str) -> Result<Credentials, CredError> {
    let op = op_binary().ok_or_else(|| CredError("De 1Password-CLI (op) is niet gevonden. Installeer hem en zet de koppeling met de 1Password-app aan.".into()))?;
    let field = |name: &str| -> Result<String, CredError> {
        let output = Command::new(&op)
            .args(["read", "--no-newline", &format!("{reference}/{name}")])
            .output()
            .map_err(|e| CredError(format!("1Password-CLI start niet: {e}")))?;
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr);
            return Err(CredError(op_message(&detail, reference)));
        }
        String::from_utf8(output.stdout)
            .map_err(|_| CredError("1Password gaf geen leesbare tekst".into()))
    };
    let username = field("username")?;
    let password = field("password")?;
    if username.is_empty() || password.is_empty() {
        return Err(CredError(format!(
            "Het item {reference} heeft geen gebruikersnaam of wachtwoord"
        )));
    }
    Ok(Credentials { username, password })
}

/// A login in 1Password, as the picker shows it. Never carries a password.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpItem {
    /// `op://<vault id>/<item id>`: ids keep working when the item or vault is renamed.
    pub reference: String,
    pub title: String,
    pub vault: String,
    pub username: String,
    /// The item is for e-Boekhouden: its website or title says so.
    pub suggested: bool,
}

/// Why the list could not be read. The picker shows a fitting notice per kind.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(tag = "kind", content = "message", rename_all = "camelCase")]
pub enum OpListError {
    /// No `op` binary on this machine.
    Missing,
    /// 1Password is locked, or the user declined the prompt.
    Locked,
    Other(String),
}

/// Lists the Login items through `op item list`. The 1Password app asks the user for consent itself.
pub async fn op_items() -> Result<Vec<OpItem>, OpListError> {
    tokio::task::spawn_blocking(|| {
        let op = op_binary().ok_or(OpListError::Missing)?;
        let output = Command::new(&op)
            .args(["item", "list", "--categories", "Login", "--format", "json"])
            .output()
            .map_err(|e| OpListError::Other(format!("1Password-CLI start niet: {e}")))?;
        if !output.status.success() {
            return Err(list_error(&String::from_utf8_lossy(&output.stderr)));
        }
        parse_items(&output.stdout)
    })
    .await
    .map_err(|e| OpListError::Other(e.to_string()))?
}

fn list_error(stderr: &str) -> OpListError {
    let lower = stderr.to_lowercase();
    if lower.contains("locked")
        || lower.contains("authorization")
        || lower.contains("signed in")
        || lower.contains("sign in")
        || lower.contains("dismissed")
    {
        OpListError::Locked
    } else {
        // The list's stderr holds no secrets; its first line helps when something odd happens.
        let line = stderr
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or_default();
        let line = line.split_once("] ").map_or(line, |(_, rest)| rest).trim();
        OpListError::Other(if line.is_empty() {
            "1Password gaf geen lijst met items".into()
        } else {
            format!("1Password gaf geen lijst met items: {line}")
        })
    }
}

#[derive(serde::Deserialize)]
struct RawItem {
    id: String,
    #[serde(default)]
    title: String,
    vault: RawVault,
    #[serde(default)]
    additional_information: Option<String>,
    #[serde(default)]
    urls: Vec<RawUrl>,
}

#[derive(serde::Deserialize)]
struct RawVault {
    id: String,
    #[serde(default)]
    name: String,
}

#[derive(serde::Deserialize)]
struct RawUrl {
    #[serde(default)]
    href: String,
}

/// Parses `op item list --format json`, e-Boekhouden items first, then by title.
pub fn parse_items(json: &[u8]) -> Result<Vec<OpItem>, OpListError> {
    // `op` prints nothing at all when there are no items.
    if json.iter().all(u8::is_ascii_whitespace) {
        return Ok(Vec::new());
    }
    let raw: Vec<RawItem> = serde_json::from_slice(json).map_err(|_| {
        OpListError::Other("1Password gaf een lijst die EB MCP niet kan lezen".into())
    })?;
    let mut items: Vec<OpItem> = raw
        .into_iter()
        .filter(|r| !r.id.is_empty() && !r.vault.id.is_empty())
        .map(|r| {
            let suggested = r.title.to_lowercase().contains("boekhouden")
                || r.urls
                    .iter()
                    .any(|u| u.href.to_lowercase().contains("e-boekhouden.nl"));
            OpItem {
                reference: format!("op://{}/{}", r.vault.id, r.id),
                title: r.title,
                vault: r.vault.name,
                username: r.additional_information.unwrap_or_default(),
                suggested,
            }
        })
        .collect();
    items.sort_by(|a, b| {
        b.suggested
            .cmp(&a.suggested)
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
    Ok(items)
}

/// Turns `op`'s stderr into one line for the user, without echoing anything secret.
pub fn op_message(stderr: &str, reference: &str) -> String {
    let lower = stderr.to_lowercase();
    if lower.contains("isn't an item") || lower.contains("not found") || lower.contains("no item") {
        format!("1Password vindt {reference} niet. Kies het item opnieuw bij Bewerken.")
    } else if lower.contains("locked")
        || lower.contains("authorization")
        || lower.contains("not signed in")
        || lower.contains("sign in")
    {
        "1Password is vergrendeld of gaf geen toestemming. Ontgrendel 1Password en probeer opnieuw."
            .into()
    } else if lower.contains("does not have a field") || lower.contains("no field") {
        format!("Het item {reference} heeft geen veld username of password")
    } else {
        format!("1Password gaf geen gegevens voor {reference}")
    }
}

fn op_binary() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> =
        vec!["/opt/homebrew/bin/op".into(), "/usr/local/bin/op".into()];
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(
            PathBuf::from(local)
                .join("Microsoft")
                .join("WinGet")
                .join("Links")
                .join("op.exe"),
        );
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            candidates.push(dir.join(if cfg!(windows) { "op.exe" } else { "op" }));
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Touch ID (or the account password) on macOS, Windows Hello on Windows.
async fn confirm_user(reason: &str) -> Result<(), CredError> {
    #[cfg(target_os = "macos")]
    {
        return mac::confirm(reason).await;
    }
    #[cfg(target_os = "windows")]
    {
        return win::confirm(reason).await;
    }
    #[allow(unreachable_code)]
    {
        let _ = reason;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use super::CredError;
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_local_authentication::{LAContext, LAPolicy};
    use std::sync::Mutex;

    pub async fn confirm(reason: &str) -> Result<(), CredError> {
        // Everything Objective-C stays in this block, so nothing that is not Send lives across the await.
        let rx = {
            let (tx, rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
            let tx = Mutex::new(Some(tx));
            let context = unsafe { LAContext::new() };
            // The reply keeps the context alive until macOS answers, then lets it go.
            let keep = Mutex::new(Some(context.clone()));
            let reply = RcBlock::new(move |ok: Bool, error: *mut NSError| {
                let result = if ok.as_bool() {
                    Ok(())
                } else {
                    // SAFETY: LocalAuthentication passes a valid NSError or null.
                    let code = unsafe { error.as_ref() }.map(|e| e.code()).unwrap_or(0);
                    Err(match code {
                        -2 | -4 | -9 => "Touch ID geannuleerd".to_owned(),
                        -8 => "Touch ID is vergrendeld. Ontgrendel je Mac met je wachtwoord."
                            .to_owned(),
                        _ => "Touch ID is niet gelukt".to_owned(),
                    })
                };
                if let Some(tx) = tx.lock().unwrap().take() {
                    let _ = tx.send(result);
                }
                keep.lock().unwrap().take();
            });
            let text = NSString::from_str(&format!("EB MCP wil {reason}"));
            // DeviceOwnerAuthentication: Touch ID, with the account password as fallback.
            unsafe {
                context.evaluatePolicy_localizedReason_reply(
                    LAPolicy::DeviceOwnerAuthentication,
                    &text,
                    &reply,
                )
            };
            rx
        };
        rx.await
            .map_err(|_| CredError("Touch ID gaf geen antwoord".into()))?
            .map_err(CredError)
    }
}

#[cfg(target_os = "windows")]
mod win {
    use super::CredError;
    use windows::Security::Credentials::UI::{UserConsentVerificationResult, UserConsentVerifier};
    use windows::core::HSTRING;

    pub async fn confirm(reason: &str) -> Result<(), CredError> {
        let message = HSTRING::from(format!("EB MCP wil {reason}"));
        let result = tokio::task::spawn_blocking(
            move || -> windows::core::Result<UserConsentVerificationResult> {
                UserConsentVerifier::RequestVerificationAsync(&message)?.join()
            },
        )
        .await
        .map_err(|e| CredError(e.to_string()))?
        .map_err(|e| CredError(format!("Windows Hello is niet gelukt: {e}")))?;
        match result {
            UserConsentVerificationResult::Verified => Ok(()),
            UserConsentVerificationResult::Canceled => {
                Err(CredError("Windows Hello geannuleerd".into()))
            }
            UserConsentVerificationResult::DeviceNotPresent
            | UserConsentVerificationResult::NotConfiguredForUser => Err(CredError(
                "Windows Hello staat niet aan op deze computer".into(),
            )),
            _ => Err(CredError("Windows Hello is niet gelukt".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{OpListError, list_error, op_message, parse_items};

    const LIST: &str = r#"[
      {"id":"itm1","title":"Zakelijke bank","version":3,"vault":{"id":"vlt1","name":"Werk"},"category":"LOGIN",
       "additional_information":"jan@studionoord.example","urls":[{"label":"website","primary":true,"href":"https://bank.example"}]},
      {"id":"itm2","title":"Studio Noord","vault":{"id":"vlt1","name":"Werk"},"category":"LOGIN",
       "additional_information":"jan@studionoord.example","urls":[{"primary":true,"href":"https://secure.e-Boekhouden.nl/bh/"}]},
      {"id":"itm3","title":"e-Boekhouden (Praktijk West)","vault":{"id":"vlt2","name":"Privé"},"category":"LOGIN"},
      {"id":"itm4","title":"agenda","vault":{"id":"vlt2","name":"Privé"},"category":"LOGIN","additional_information":"west"}
    ]"#;

    #[test]
    fn lists_e_boekhouden_items_first_by_their_ids() {
        let items = parse_items(LIST.as_bytes()).unwrap();
        let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "e-Boekhouden (Praktijk West)",
                "Studio Noord",
                "agenda",
                "Zakelijke bank"
            ]
        );
        assert!(items[0].suggested && items[1].suggested && !items[2].suggested);
        assert_eq!(items[1].reference, "op://vlt1/itm2");
        assert_eq!(items[1].vault, "Werk");
        assert_eq!(items[1].username, "jan@studionoord.example");
        assert_eq!(items[0].username, "");
    }

    #[test]
    fn reads_an_empty_list_and_refuses_garbage() {
        assert_eq!(parse_items(b"").unwrap(), vec![]);
        assert_eq!(parse_items(b" \n").unwrap(), vec![]);
        assert_eq!(parse_items(b"[]").unwrap(), vec![]);
        assert!(matches!(parse_items(b"{oops"), Err(OpListError::Other(_))));
    }

    #[test]
    fn tells_a_locked_1password_from_other_errors() {
        assert_eq!(
            list_error("[ERROR] authorization prompt dismissed, please try again"),
            OpListError::Locked
        );
        assert_eq!(
            list_error("[ERROR] You are not currently signed in."),
            OpListError::Locked
        );
        assert_eq!(
            list_error("[ERROR] 2026/11/10 12:00:00 something odd\n"),
            OpListError::Other(
                "1Password gaf geen lijst met items: 2026/11/10 12:00:00 something odd".into()
            )
        );
        let json = serde_json::to_string(&OpListError::Missing).unwrap();
        assert_eq!(json, r#"{"kind":"missing"}"#);
    }

    #[test]
    fn explains_1password_errors_without_details() {
        let r = "op://Werk/EB";
        assert!(
            op_message("[ERROR] \"EB\" isn't an item in the \"Werk\" vault", r)
                .contains("vindt op://Werk/EB niet")
        );
        assert!(op_message("[ERROR] authorization prompt dismissed", r).contains("vergrendeld"));
        assert!(op_message("[ERROR] item does not have a field password", r).contains("geen veld"));
        assert_eq!(
            op_message("something odd: s3cr3t", r),
            "1Password gaf geen gegevens voor op://Werk/EB"
        );
    }
}
