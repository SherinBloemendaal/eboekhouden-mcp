//! Signing in out of sight (SPEC.md, Sign-in): a hidden, in-memory webview
//! opens e-Boekhouden's own login page; js/signin-driver.js fills it and
//! clicks their button; the user only sees the steps, and types a two-step
//! code into EB MCP's own field when e-Boekhouden asks for one.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::creds;
use crate::state::{AppState, CookieJar, now_ms};

pub const LOGIN_URL: &str = "https://secure.e-boekhouden.nl/bh/";
const LOGOUT_URL: &str = "https://secure.e-boekhouden.nl/bh/uitloggen.asp";
const API_ORIGIN: &str = "https://secure20.e-boekhouden.nl/";
const DRIVER: &str = include_str!("../js/signin-driver.js");
const DIALOG_HOOK: &str = include_str!("../js/dialog-hook.js");
const POLL: Duration = Duration::from_millis(400);
const LOAD_TIMEOUT: Duration = Duration::from_secs(30);
const SUBMIT_TIMEOUT: Duration = Duration::from_secs(30);
const CODE_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Step {
    Credentials,
    Page,
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

#[derive(Debug, Deserialize, PartialEq)]
pub struct Probe {
    pub phase: String,
    pub message: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum Action {
    Wait,
    Fill,
    AskCode,
    Success,
    Fail(String),
    Unknown(String),
}

/// What to do next, from what the page shows. Pure, so it can be tested.
pub fn decide(probe: &Probe, has_token: bool, filled: bool, waited: Duration) -> Action {
    if filled && has_token {
        return Action::Success;
    }
    match probe.phase.as_str() {
        "form" if !filled => Action::Fill,
        "form" | "app" if waited > SUBMIT_TIMEOUT => {
            Action::Unknown("e-Boekhouden reageerde niet zoals verwacht op het inloggen".into())
        }
        "form" | "app" => Action::Wait,
        "error" => Action::Fail(
            probe
                .message
                .clone()
                .unwrap_or_else(|| "e-Boekhouden weigerde het inloggen".into()),
        ),
        "code" => Action::AskCode,
        "unknown" => Action::Unknown(
            probe
                .message
                .clone()
                .unwrap_or_else(|| "e-Boekhouden vroeg om een stap die EB MCP niet kent".into()),
        ),
        _ if waited > LOAD_TIMEOUT => {
            Action::Unknown("De inlogpagina van e-Boekhouden laadde niet".into())
        }
        _ => Action::Wait,
    }
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

/// Sign-ins that are running, by account.
#[derive(Default)]
pub struct Signins {
    pending: Mutex<HashMap<Uuid, Pending>>,
}

impl Signins {
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
    let (step, message) = run(&app, &state, &signins, account_id, &cancel).await;
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
    let credentials = match creds::read(&account).await {
        Ok(c) => c,
        Err(e) => return (Step::Failed, Some(e.0)),
    };
    if cancel.load(Ordering::SeqCst) {
        return (Step::Cancelled, None);
    }

    emit(app, account_id, Step::Page, None);
    let label = format!("eb-{}-{}", account_id.simple(), now_ms());
    let url = WebviewUrl::External(LOGIN_URL.parse().expect("login URL"));
    let window = match WebviewWindowBuilder::new(app, &label, url)
        .title("e-Boekhouden")
        .visible(false)
        .incognito(true)
        .skip_taskbar(true)
        .inner_size(1100.0, 800.0)
        .initialization_script_for_all_frames(DIALOG_HOOK)
        .build()
    {
        Ok(w) => w,
        Err(e) => {
            return (
                Step::Failed,
                Some(format!("EB MCP kon geen sessie openen: {e}")),
            );
        }
    };

    let mut filled = false;
    let mut since = Instant::now();
    let mut last_phase = String::new();
    let outcome = loop {
        if cancel.load(Ordering::SeqCst) {
            break (Step::Cancelled, None);
        }
        tokio::time::sleep(POLL).await;
        let probe = probe(&window).await;
        if probe.phase != last_phase {
            last_phase = probe.phase.clone();
            since = Instant::now();
        }
        let has_token = cookies_of(&window)
            .iter()
            .any(|(k, v)| k == "auth-token" && !v.is_empty());
        match decide(&probe, has_token, filled, since.elapsed()) {
            Action::Wait => {}
            Action::Fill => {
                let js = format!(
                    "{DRIVER}\n;window.__ebmcpSignin.fill({}, {})",
                    serde_json::to_string(&credentials.username).unwrap_or_default(),
                    serde_json::to_string(&credentials.password).unwrap_or_default()
                );
                if run_js(&window, js).await.as_deref() == Some("\"ok\"") {
                    filled = true;
                    since = Instant::now();
                    emit(app, account_id, Step::Submit, None);
                }
            }
            Action::AskCode => {
                let (tx, rx) = oneshot::channel();
                if let Some(p) = signins.pending.lock().unwrap().get_mut(&account_id) {
                    p.code = Some(tx);
                }
                emit(app, account_id, Step::Code, None);
                let code = tokio::select! {
                    code = rx => code.ok(),
                    _ = tokio::time::sleep(CODE_TIMEOUT) => None,
                    _ = wait_cancel(cancel) => None,
                };
                let Some(code) = code else {
                    break if cancel.load(Ordering::SeqCst) {
                        (Step::Cancelled, None)
                    } else {
                        (
                            Step::Failed,
                            Some("Geen code ontvangen binnen 5 minuten".into()),
                        )
                    };
                };
                let js = format!(
                    "{DRIVER}\n;window.__ebmcpSignin.code({})",
                    serde_json::to_string(&code).unwrap_or_default()
                );
                run_js(&window, js).await;
                since = Instant::now();
                emit(app, account_id, Step::Submit, None);
            }
            Action::Success => break (Step::Done, None),
            Action::Fail(message) => break (Step::Failed, Some(message)),
            Action::Unknown(message) => break (Step::Unknown, Some(message)),
        }
    };
    drop(credentials);

    if outcome.0 != Step::Done {
        let _ = window.destroy();
        return outcome;
    }
    state.add_session(account_id, &account.label, &label);
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

async fn wait_cancel(cancel: &AtomicBool) {
    while !cancel.load(Ordering::SeqCst) {
        tokio::time::sleep(POLL).await;
    }
}

async fn probe(window: &WebviewWindow) -> Probe {
    let js = format!(
        "{DRIVER}\n;(() => {{ try {{ return window.__ebmcpSignin.probe(); }} catch (e) {{ return {{ phase: 'loading', message: null }}; }} }})()"
    );
    run_js(window, js)
        .await
        .and_then(|text| serde_json::from_str::<Probe>(&text).ok())
        .unwrap_or(Probe {
            phase: "loading".into(),
            message: None,
        })
}

/// Evaluates `js` in the page and returns its result as JSON text.
async fn run_js(window: &WebviewWindow, js: String) -> Option<String> {
    let (tx, rx) = oneshot::channel();
    let tx = Mutex::new(Some(tx));
    window
        .eval_with_callback(js, move |result| {
            if let Some(tx) = tx.lock().unwrap().take() {
                let _ = tx.send(result);
            }
        })
        .ok()?;
    tokio::time::timeout(Duration::from_secs(5), rx)
        .await
        .ok()?
        .ok()
}

fn cookies_of(window: &WebviewWindow) -> Vec<(String, String)> {
    window
        .cookies_for_url(API_ORIGIN.parse().expect("API origin"))
        .map(|list| {
            list.into_iter()
                .map(|c| (c.name().to_owned(), c.value().to_owned()))
                .collect()
        })
        .unwrap_or_default()
}

/// The webviews as the session store sees them.
pub struct WebviewJar(pub AppHandle);

impl CookieJar for WebviewJar {
    fn cookies(&self, webview: &str) -> Result<Vec<(String, String)>, String> {
        let window = self
            .0
            .get_webview_window(webview)
            .ok_or("De sessie is gesloten")?;
        Ok(cookies_of(&window))
    }

    /// Signs out at e-Boekhouden in the session's own webview, then closes it.
    fn sign_out(&self, webview: &str) {
        let Some(window) = self.0.get_webview_window(webview) else {
            return;
        };
        let _ = window.navigate(LOGOUT_URL.parse().expect("logout URL"));
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(3)).await;
            let _ = window.destroy();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn p(phase: &str, message: Option<&str>) -> Probe {
        Probe {
            phase: phase.into(),
            message: message.map(str::to_owned),
        }
    }

    const SHORT: Duration = Duration::from_secs(1);
    const LONG: Duration = Duration::from_secs(31);

    #[test]
    fn fills_once_and_then_waits() {
        assert_eq!(decide(&p("form", None), false, false, SHORT), Action::Fill);
        assert_eq!(decide(&p("form", None), false, true, SHORT), Action::Wait);
        assert!(matches!(
            decide(&p("form", None), false, true, LONG),
            Action::Unknown(_)
        ));
    }

    #[test]
    fn success_needs_the_token_after_our_submit() {
        assert_eq!(decide(&p("app", None), true, true, SHORT), Action::Success);
        assert_eq!(decide(&p("app", None), false, true, SHORT), Action::Wait);
        assert_eq!(
            decide(&p("form", None), true, false, SHORT),
            Action::Fill,
            "a token before our submit proves nothing"
        );
    }

    #[test]
    fn passes_on_errors_codes_and_unknown_steps() {
        assert_eq!(
            decide(&p("error", Some("Onjuist wachtwoord")), false, true, SHORT),
            Action::Fail("Onjuist wachtwoord".into())
        );
        assert_eq!(
            decide(&p("code", None), false, true, SHORT),
            Action::AskCode
        );
        assert!(matches!(
            decide(&p("unknown", None), false, true, SHORT),
            Action::Unknown(_)
        ));
        assert_eq!(
            decide(&p("loading", None), false, false, SHORT),
            Action::Wait
        );
        assert!(matches!(
            decide(&p("loading", None), false, false, LONG),
            Action::Unknown(_)
        ));
    }

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
}
