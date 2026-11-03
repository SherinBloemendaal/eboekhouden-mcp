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
    use super::op_message;

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
