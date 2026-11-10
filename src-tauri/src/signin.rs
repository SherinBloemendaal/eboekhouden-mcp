//! Signing in (SPEC.md, Sign-in), the way e-Boekhouden's own app does it: the username and password from
//! 1Password or the Keychain go to `auth/login` on e-Boekhouden's API, which answers with the session token.
//! When the account uses two-step verification, the user types the code into EB MCP's own field and the
//! login is sent once more with it. The token and the cookies of the answer live only in memory (SessionJar).
//!
//! Until 0.1.8 EB MCP filled e-Boekhouden's classic login page in a hidden webview. That signs in to the
//! old ASP pages only: their API never accepted that session (Sherin's traces of 2026-11-10).

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;
use tauri::AppHandle;
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::creds;
use crate::ebapi::Login;
use crate::state::{AppState, CookieJar, now_ms};

/// e-Boekhouden's login page, for "Open e-Boekhouden in je browser".
pub const LOGIN_URL: &str = "https://secure.e-boekhouden.nl/bh/";
const POLL: Duration = Duration::from_millis(200);
const CODE_TIMEOUT: Duration = Duration::from_secs(300);
/// How many finished sign-ins Instellingen › Diagnose keeps.
const TRACES_KEPT: usize = 10;
/// Wrong codes in a row before EB MCP stops asking.
const CODE_TRIES: u32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Step {
    Credentials,
    Submit,
    Code,
    Administration,
    Done,
    Failed,
    Unknown,
    Cancelled,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SigninEvent {
    pub account_id: Uuid,
    pub step: Step,
    pub message: Option<String>,
}

/// One line of a sign-in's diagnosis: what EB MCP saw and did, with the time since the start.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceLine {
    pub ms: u64,
    pub text: String,
}

/// A sign-in as Instellingen › Diagnose shows it. Holds no password, code or cookie.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Trace {
    pub account_id: Uuid,
    pub label: String,
    pub started_at: u64,
    pub outcome: Option<String>,
    pub lines: Vec<TraceLine>,
}

/// The administration's name from `administratie/uwgegevens`, whatever its exact shape.
pub fn administration_name(json: &Value) -> Option<String> {
    const KEYS: [&str; 6] = [
        "bedrijfsnaam",
        "administratienaam",
        "naam",
        "companyname",
        "name",
        "administratie",
    ];
    let pick = |object: &serde_json::Map<String, Value>| {
        KEYS.iter().find_map(|key| {
            object
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .and_then(|(_, v)| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
        })
    };
    let object = json.as_object()?;
    pick(object).or_else(|| object.values().filter_map(Value::as_object).find_map(pick))
}
struct Pending {
    cancel: Arc<AtomicBool>,
    code: Option<oneshot::Sender<String>>,
}

/// Sign-ins that are running, by account, and the diagnosis of the last few.
#[derive(Default)]
pub struct Signins {
    pending: Mutex<HashMap<Uuid, Pending>>,
    traces: Mutex<VecDeque<(Instant, Trace)>>,
}

impl Signins {
    fn trace_start(&self, account_id: Uuid, label: &str) {
        let mut traces = self.traces.lock().unwrap();
        if traces.len() >= TRACES_KEPT {
            traces.pop_front();
        }
        traces.push_back((
            Instant::now(),
            Trace {
                account_id,
                label: label.to_owned(),
                started_at: now_ms(),
                outcome: None,
                lines: Vec::new(),
            },
        ));
    }

    /// Adds a line to the account's latest sign-in, and to the log file.
    fn trace(&self, account_id: Uuid, text: impl Into<String>) {
        let text = text.into();
        log::info!("aanmelden {account_id}: {text}");
        let mut traces = self.traces.lock().unwrap();
        if let Some((started, trace)) = traces
            .iter_mut()
            .rev()
            .find(|(_, t)| t.account_id == account_id)
        {
            trace.lines.push(TraceLine {
                ms: started.elapsed().as_millis() as u64,
                text,
            });
        }
    }

    fn trace_end(&self, account_id: Uuid, outcome: String) {
        self.trace(account_id, format!("Einde: {outcome}"));
        if let Some((_, trace)) = self
            .traces
            .lock()
            .unwrap()
            .iter_mut()
            .rev()
            .find(|(_, t)| t.account_id == account_id)
        {
            trace.outcome = Some(outcome);
        }
    }

    /// The last sign-ins, newest first.
    pub fn traces(&self) -> Vec<Trace> {
        self.traces
            .lock()
            .unwrap()
            .iter()
            .rev()
            .map(|(_, t)| t.clone())
            .collect()
    }

    pub fn is_running(&self, id: Uuid) -> bool {
        self.pending.lock().unwrap().contains_key(&id)
    }

    pub fn cancel(&self, id: Uuid) {
        if let Some(p) = self.pending.lock().unwrap().get(&id) {
            p.cancel.store(true, Ordering::SeqCst);
        }
    }

    pub fn give_code(&self, id: Uuid, code: String) -> Result<(), String> {
        let code = code.trim().to_owned();
        if code.is_empty() || code.len() > 12 || !code.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err("Vul de code uit je authenticator-app in".into());
        }
        let sender = self
            .pending
            .lock()
            .unwrap()
            .get_mut(&id)
            .and_then(|p| p.code.take());
        sender
            .ok_or("Er wordt nu geen code gevraagd")?
            .send(code)
            .map_err(|_| "De aanmelding is al gestopt".into())
    }
}

fn emit(app: &AppHandle, account_id: Uuid, step: Step, message: Option<String>) {
    crate::to_our_windows(
        app,
        "signin",
        SigninEvent {
            account_id,
            step,
            message,
        },
    );
}

/// Runs one sign-in to the end, reporting each step. Never retries on its own.
pub async fn sign_in(
    app: AppHandle,
    state: Arc<AppState>,
    signins: Arc<Signins>,
    account_id: Uuid,
) {
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut pending = signins.pending.lock().unwrap();
        if pending.contains_key(&account_id) {
            return;
        }
        pending.insert(
            account_id,
            Pending {
                cancel: cancel.clone(),
                code: None,
            },
        );
    }
    let label = state
        .accounts
        .get(account_id)
        .map(|a| a.label)
        .unwrap_or_default();
    signins.trace_start(account_id, &label);
    let started = Instant::now();
    let (step, message) = run(&app, &state, &signins, account_id, &cancel).await;
    // The sign-in itself in Verzoeken, also when it failed.
    let call = crate::ebapi::ApiCall {
        method: "LOGIN",
        path: "inloggen".into(),
        query: Vec::new(),
        status: match step {
            Step::Done => 200,
            Step::Failed => 401,
            Step::Cancelled => 499,
            _ => 0,
        },
        duration_ms: started.elapsed().as_millis() as u64,
        bytes: 0,
        rows: None,
        json: None,
        error: (step != Step::Done).then(|| message.clone().unwrap_or_else(|| format!("{step:?}"))),
    };
    state.log_signin_call(&call, account_id, &label);
    signins.trace_end(
        account_id,
        match &message {
            Some(m) => format!("{step:?}: {m}"),
            None => format!("{step:?}"),
        },
    );
    signins.pending.lock().unwrap().remove(&account_id);
    emit(&app, account_id, step, message);
}

async fn run(
    app: &AppHandle,
    state: &Arc<AppState>,
    signins: &Signins,
    account_id: Uuid,
    cancel: &AtomicBool,
) -> (Step, Option<String>) {
    let Some(account) = state.accounts.get(account_id) else {
        return (Step::Failed, Some("Dit account bestaat niet meer".into()));
    };
    emit(app, account_id, Step::Credentials, None);
    signins.trace(
        account_id,
        format!("Gegevens lezen uit {:?}", account.store),
    );
    let credentials = creds::read(&account).await;
    crate::refocus_main(app);
    let credentials = match credentials {
        Ok(c) => {
            signins.trace(account_id, "Gegevens gelezen");
            c
        }
        Err(e) => return (Step::Failed, Some(e.0)),
    };
    if cancel.load(Ordering::SeqCst) {
        return (Step::Cancelled, None);
    }

    emit(app, account_id, Step::Submit, None);
    let mut code: Option<String> = None;
    let mut code_tries = 0u32;
    let cookies = loop {
        signins.trace(
            account_id,
            if code.is_some() {
                "Inloggen bij e-Boekhouden met de tweestapscode"
            } else {
                "Inloggen bij e-Boekhouden (auth/login)"
            },
        );
        let (call, outcome) = state
            .client
            .login(
                &credentials.username,
                &credentials.password,
                code.as_deref(),
            )
            .await;
        state.log_signin_call(&call, account_id, &account.label);
        signins.trace(
            account_id,
            format!("Antwoord: status {}, {}", call.status, describe(&outcome)),
        );
        if cancel.load(Ordering::SeqCst) {
            return (Step::Cancelled, None);
        }
        match outcome {
            Login::Session(cookies) => break cookies,
            Login::Refused(message) if code.is_none() => return (Step::Failed, Some(message)),
            Login::Refused(_) | Login::NeedsCode if code.is_some() && code_tries >= CODE_TRIES => {
                return (
                    Step::Failed,
                    Some("De tweestapscode werd drie keer niet geaccepteerd".into()),
                );
            }
            Login::Refused(_) | Login::NeedsCode => {
                if code.is_some() {
                    signins.trace(account_id, "De code werd niet geaccepteerd; opnieuw vragen");
                }
                let (tx, rx) = oneshot::channel();
                if let Some(p) = signins.pending.lock().unwrap().get_mut(&account_id) {
                    p.code = Some(tx);
                }
                emit(
                    app,
                    account_id,
                    Step::Code,
                    code.is_some().then(|| {
                        "Die code werd niet geaccepteerd. Probeer de volgende.".to_owned()
                    }),
                );
                let given = tokio::select! {
                    given = rx => given.ok(),
                    _ = tokio::time::sleep(CODE_TIMEOUT) => None,
                    _ = wait_cancel(cancel) => None,
                };
                let Some(given) = given else {
                    return if cancel.load(Ordering::SeqCst) {
                        (Step::Cancelled, None)
                    } else {
                        (
                            Step::Failed,
                            Some("Geen code ontvangen binnen 5 minuten".into()),
                        )
                    };
                };
                code = Some(given);
                code_tries += 1;
                emit(app, account_id, Step::Submit, None);
            }
            Login::Error(message) => return (Step::Unknown, Some(message)),
        }
    };
    drop(credentials);

    // The check e-Boekhouden's own app makes at startup: does the API accept this session?
    let check = state.client.auth(&cookies).await;
    state.log_signin_call(&check, account_id, &account.label);
    let valid = crate::ebapi::valid_session(&check);
    signins.trace(
        account_id,
        format!(
            "API-controle: status {}, {}",
            check.status,
            if valid { "ingelogd" } else { "niet ingelogd" }
        ),
    );
    if !valid {
        return (
            Step::Unknown,
            Some(format!(
                "e-Boekhouden gaf een sessie, maar de API accepteert hem niet (status {})",
                check.status
            )),
        );
    }

    let key = format!("api-{}-{}", account_id.simple(), now_ms());
    state.jar_put(&key, cookies);
    state.add_session(account_id, &account.label, &key);
    signins.trace(account_id, "Ingelogd; sessie staat alleen in het geheugen");

    emit(app, account_id, Step::Administration, None);
    let query = [("adm".to_owned(), "true".to_owned())];
    let call = state
        .api_get(
            account_id,
            "administratie/uwgegevens",
            &query,
            None,
            "EB MCP",
        )
        .await;
    if let Some(error) = &call.error {
        signins.trace(account_id, format!("Administratie ophalen: {error}"));
    }
    if let Some(name) = call.json.as_ref().and_then(administration_name) {
        state.set_administration(account_id, &name);
    }
    (
        Step::Done,
        state
            .accounts
            .get(account_id)
            .and_then(|a| a.administration),
    )
}

/// For the diagnosis: what `auth/login` answered, without any token.
fn describe(outcome: &Login) -> String {
    match outcome {
        Login::Session(cookies) => {
            let mut names: Vec<&str> = cookies.iter().map(|(k, _)| k.as_str()).collect();
            names.sort_unstable();
            format!("ingelogd (cookies: {})", names.join(", "))
        }
        Login::NeedsCode => "vraagt om een tweestapscode".into(),
        Login::Refused(m) => format!("geweigerd: {m}"),
        Login::Error(m) => format!("fout: {m}"),
    }
}

async fn wait_cancel(cancel: &AtomicBool) {
    while !cancel.load(Ordering::SeqCst) {
        tokio::time::sleep(POLL).await;
    }
}

/// The sessions' cookies, in memory only: what `auth/login` gave, `auth-token` among them.
#[derive(Default)]
pub struct SessionJar {
    sessions: Mutex<HashMap<String, Vec<(String, String)>>>,
}

impl CookieJar for SessionJar {
    fn cookies(&self, key: &str) -> Result<Vec<(String, String)>, String> {
        self.sessions
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .ok_or_else(|| "De sessie is gesloten".into())
    }

    fn put(&self, key: &str, cookies: Vec<(String, String)>) {
        self.sessions
            .lock()
            .unwrap()
            .insert(key.to_owned(), cookies);
    }

    /// Forgets the session. e-Boekhouden's API has no sign-out of its own; the token stops working when it expires.
    fn sign_out(&self, key: &str) {
        self.sessions.lock().unwrap().remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn finds_the_administration_name() {
        assert_eq!(
            administration_name(&json!({"Bedrijfsnaam": "Studio Noord B.V."})).as_deref(),
            Some("Studio Noord B.V.")
        );
        assert_eq!(
            administration_name(&json!({"gegevens": {"naam": "Demo Holding B.V."}})).as_deref(),
            Some("Demo Holding B.V.")
        );
        assert_eq!(administration_name(&json!({"naam": "  "})), None);
        assert_eq!(administration_name(&json!([1])), None);
    }

    #[test]
    fn accepts_only_plausible_codes() {
        let signins = Signins::default();
        let id = Uuid::new_v4();
        assert!(signins.give_code(id, "48 29".into()).is_err());
        assert_eq!(
            signins.give_code(id, "482913".into()),
            Err("Er wordt nu geen code gevraagd".into())
        );
        let (tx, rx) = oneshot::channel();
        signins.pending.lock().unwrap().insert(
            id,
            Pending {
                cancel: Arc::new(AtomicBool::new(false)),
                code: Some(tx),
            },
        );
        signins.give_code(id, " 482913 ".into()).unwrap();
        assert_eq!(rx.blocking_recv().unwrap(), "482913");
    }

    #[test]
    fn keeps_a_trace_per_sign_in_without_secrets_and_only_the_last_ten() {
        let signins = Signins::default();
        let id = Uuid::new_v4();
        for n in 0..12 {
            signins.trace_start(id, &format!("Studio {n}"));
            signins.trace(
                id,
                "Pagina: form (/bh/inloggen.asp: e-mailveld in formulier)",
            );
            signins.trace_end(id, "Done".into());
        }
        let traces = signins.traces();
        assert_eq!(traces.len(), TRACES_KEPT);
        assert_eq!(traces[0].label, "Studio 11", "newest first");
        assert_eq!(traces[0].lines.len(), 2);
        assert_eq!(traces[0].outcome.as_deref(), Some("Done"));
    }

    #[test]
    fn the_jar_keeps_cookies_until_sign_out() {
        let jar = SessionJar::default();
        assert!(jar.cookies("a").is_err());
        jar.put("a", vec![("auth-token".into(), "jwt".into())]);
        assert_eq!(
            jar.cookies("a").unwrap(),
            vec![("auth-token".to_owned(), "jwt".to_owned())]
        );
        jar.sign_out("a");
        assert!(jar.cookies("a").is_err());
    }

    #[test]
    fn describes_the_answer_without_the_token() {
        let text = describe(&Login::Session(vec![
            ("auth-token".into(), "geheim-jwt".into()),
            ("SRVRID".into(), "9".into()),
        ]));
        assert!(text.contains("auth-token") && text.contains("SRVRID"));
        assert!(!text.contains("geheim-jwt"));
    }
}
