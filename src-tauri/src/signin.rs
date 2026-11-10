//! Signing in out of sight (SPEC.md, Sign-in): a hidden, in-memory webview
//! opens e-Boekhouden's own login page; js/signin-driver.js fills it and
//! clicks their button; the user only sees the steps, and types a two-step
//! code into EB MCP's own field when e-Boekhouden asks for one.

use std::collections::{HashMap, VecDeque};
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
const ASP_ORIGIN: &str = "https://secure.e-boekhouden.nl/";
/// e-Boekhouden's own way into its app after a login (its "hybrid route", as the EB Sessions extension uses it).
/// Plain /bh/ keeps the old frameset; only the app on secure20 sets the `auth-token` cookie the API needs.
const APP_ENTRY: &str =
    "https://secure.e-boekhouden.nl/bh?v2Url=https%3A%2F%2Fsecure20.e-boekhouden.nl%2F";
/// After the login answer, how long e-Boekhouden gets to finish its own token steps before EB MCP opens the app.
const APP_SETTLE: Duration = Duration::from_millis(1500);
/// After the login answer: how often EB MCP asks e-Boekhouden's API whether the session is signed in.
const AUTH_EVERY: Duration = Duration::from_millis(1500);
/// Checks that say "not yet" before EB MCP opens e-Boekhouden's app, which completes the session on secure20.
const AUTH_TRIES_BEFORE_APP: u32 = 2;
const DRIVER: &str = include_str!("../js/signin-driver.js");
const DIALOG_HOOK: &str = include_str!("../js/dialog-hook.js");
const POLL: Duration = Duration::from_millis(400);
const LOAD_TIMEOUT: Duration = Duration::from_secs(30);
const SUBMIT_TIMEOUT: Duration = Duration::from_secs(30);
const CODE_TIMEOUT: Duration = Duration::from_secs(300);
/// The whole sign-in, not counting the time the user takes to type a two-step code.
const TOTAL_TIMEOUT: Duration = Duration::from_secs(120);
/// How many finished sign-ins Instellingen › Diagnose keeps.
const TRACES_KEPT: usize = 10;

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
    /// For the diagnosis only: which frames the driver saw and what it found in them. Never a value typed in.
    #[serde(default)]
    pub detail: Option<String>,
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
    /// The hidden webview, once it exists, so Toon e-Boekhouden can show it.
    window: Option<String>,
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

    /// The hidden webview of a running sign-in, for Toon e-Boekhouden.
    pub fn window_of(&self, id: Uuid) -> Option<String> {
        self.pending
            .lock()
            .unwrap()
            .get(&id)
            .and_then(|p| p.window.clone())
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
                window: None,
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

    emit(app, account_id, Step::Page, None);
    signins.trace(account_id, "Inlogpagina openen in een verborgen venster");
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
    if let Some(p) = signins.pending.lock().unwrap().get_mut(&account_id) {
        p.window = Some(label.clone());
    }

    let mut filled = false;
    let mut since = Instant::now();
    let started = Instant::now();
    let mut code_time = Duration::ZERO;
    let mut fill_failing: Option<Instant> = None;
    let mut last_seen = String::new();
    let mut last_phase = String::new();
    let mut had_token = false;
    let mut app_opened = false;
    let mut last_cookies = String::new();
    let mut logged_in_page = false;
    let mut last_auth: Option<Instant> = None;
    let mut auth_tries = 0u32;
    let mut session_valid = false;
    let mut last_auth_status = 0u16;
    let outcome = loop {
        if cancel.load(Ordering::SeqCst) {
            break (Step::Cancelled, None);
        }
        if started.elapsed().saturating_sub(code_time) > TOTAL_TIMEOUT {
            break (
                Step::Unknown,
                Some("Inloggen bij e-Boekhouden duurde langer dan 2 minuten. Kijk bij Details of met Toon e-Boekhouden waar het bleef hangen.".into()),
            );
        }
        tokio::time::sleep(POLL).await;
        let probe = probe(&window).await;
        let seen = format!(
            "{} {}",
            probe.phase,
            probe.detail.as_deref().unwrap_or_default()
        );
        if seen != last_seen {
            signins.trace(
                account_id,
                format!(
                    "Pagina: {}{}",
                    probe.phase,
                    probe
                        .detail
                        .as_deref()
                        .map(|d| format!(" ({d})"))
                        .unwrap_or_default()
                ),
            );
            last_seen = seen;
        }
        if probe.phase != last_phase {
            last_phase = probe.phase.clone();
            since = Instant::now();
        }
        let has_token = cookies_of(&window)
            .iter()
            .any(|(k, v)| k == "auth-token" && !v.is_empty());
        let names = cookie_names(&window);
        if names != last_cookies {
            signins.trace(account_id, format!("Cookies (alleen namen): {names}"));
            last_cookies = names;
        }
        if filled && probe.phase == "app" {
            logged_in_page = true;
        }
        // After the login answer, ask e-Boekhouden's API itself whether these cookies make a signed-in session:
        // that is what the MCP calls need, whatever cookie carries it.
        if logged_in_page
            && !session_valid
            && since.elapsed() > APP_SETTLE
            && last_auth.is_none_or(|t| t.elapsed() >= AUTH_EVERY)
        {
            last_auth = Some(Instant::now());
            auth_tries += 1;
            let call = state.client.auth(&cookies_of(&window)).await;
            state.log_signin_call(&call, account_id, &account.label);
            session_valid = crate::ebapi::valid_session(&call);
            last_auth_status = call.status;
            signins.trace(
                account_id,
                format!(
                    "API-controle {auth_tries}: status {}, {}",
                    call.status,
                    if session_valid {
                        "ingelogd"
                    } else {
                        "nog niet ingelogd"
                    }
                ),
            );
            if session_valid {
                break (Step::Done, None);
            }
            // Signed in on the old frameset only: open e-Boekhouden's app, which completes the session on secure20.
            if !app_opened && auth_tries >= AUTH_TRIES_BEFORE_APP {
                signins.trace(
                    account_id,
                    "De app van e-Boekhouden openen om de sessie af te maken",
                );
                if let Err(e) = window.navigate(APP_ENTRY.parse().expect("app entry URL")) {
                    signins.trace(account_id, format!("De app openen lukte niet: {e}"));
                }
                app_opened = true;
                since = Instant::now();
                continue;
            }
        }
        if has_token != had_token {
            signins.trace(
                account_id,
                if has_token {
                    "Sessiecookie van e-Boekhouden ontvangen"
                } else {
                    "Sessiecookie weer weg"
                },
            );
            had_token = has_token;
        }
        match decide(&probe, has_token, filled, since.elapsed()) {
            Action::Wait => {}
            Action::Fill => {
                let js = format!(
                    "{DRIVER}\n;window.__ebmcpSignin.fill({}, {})",
                    serde_json::to_string(&credentials.username).unwrap_or_default(),
                    serde_json::to_string(&credentials.password).unwrap_or_default()
                );
                let result = run_js(&window, js).await;
                if result.as_deref() == Some("\"ok\"") {
                    signins.trace(account_id, "Gegevens ingevuld en verstuurd");
                    filled = true;
                    fill_failing = None;
                    since = Instant::now();
                    emit(app, account_id, Step::Submit, None);
                } else {
                    let result = result.unwrap_or_else(|| "geen antwoord".into());
                    let first = *fill_failing.get_or_insert_with(|| {
                        signins.trace(account_id, format!("Invullen lukt niet: {result}"));
                        Instant::now()
                    });
                    // The form is there but cannot be filled (no password field, say): stop instead of trying forever.
                    if first.elapsed() > LOAD_TIMEOUT {
                        break (Step::Unknown, Some(fill_failed_message(&result)));
                    }
                }
            }
            Action::AskCode => {
                let (tx, rx) = oneshot::channel();
                if let Some(p) = signins.pending.lock().unwrap().get_mut(&account_id) {
                    p.code = Some(tx);
                }
                emit(app, account_id, Step::Code, None);
                signins.trace(account_id, "e-Boekhouden vraagt om een tweestapscode");
                let asked = Instant::now();
                let code = tokio::select! {
                    code = rx => code.ok(),
                    _ = tokio::time::sleep(CODE_TIMEOUT) => None,
                    _ = wait_cancel(cancel) => None,
                };
                code_time += asked.elapsed();
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
            // Signed in on the page, but the API check itself kept failing: say that, not that the page did not load.
            Action::Unknown(_)
                if auth_tries > 0 && !matches!(last_auth_status, 200..=299 | 401 | 403) =>
            {
                break (
                    Step::Unknown,
                    Some(format!(
                        "Ingelogd op de pagina, maar EB MCP kon de sessie niet controleren: e-Boekhouden gaf status {last_auth_status}"
                    )),
                );
            }
            Action::Unknown(message) => break (Step::Unknown, Some(message)),
        }
    };
    drop(credentials);

    if outcome.0 != Step::Done {
        let _ = window.destroy();
        return outcome;
    }
    // Toon e-Boekhouden may have shown the window; the session itself stays out of sight.
    let _ = window.hide();
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

/// What to tell the user when e-Boekhouden's form is there but cannot be filled.
fn fill_failed_message(result: &str) -> String {
    if result.contains("nopassword") {
        "EB MCP vindt het wachtwoordveld niet op de inlogpagina van e-Boekhouden. Kijk met Toon e-Boekhouden wat de pagina laat zien.".into()
    } else {
        "EB MCP kan het inlogformulier van e-Boekhouden niet invullen. Kijk met Toon e-Boekhouden wat de pagina laat zien.".into()
    }
}

/// Toon e-Boekhouden: brings the hidden sign-in window into view, to see what e-Boekhouden shows.
pub fn show_window(app: &AppHandle, signins: &Signins, id: Uuid) -> Result<(), String> {
    let label = signins
        .window_of(id)
        .ok_or("Er loopt nu geen aanmelding voor dit account")?;
    let window = app
        .get_webview_window(&label)
        .ok_or("Het inlogvenster is al gesloten")?;
    signins.trace(id, "Inlogvenster getoond (Toon e-Boekhouden)");
    let _ = window.center();
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())
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
            detail: Some("geen antwoord van de pagina".into()),
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

/// For the diagnosis: the names of e-Boekhouden's cookies per host, never their values.
fn cookie_names(window: &WebviewWindow) -> String {
    let names = |origin: &str| {
        let mut list: Vec<String> = window
            .cookies_for_url(origin.parse().expect("origin"))
            .map(|l| l.into_iter().map(|c| c.name().to_owned()).collect())
            .unwrap_or_default();
        list.sort();
        list.dedup();
        if list.is_empty() {
            "geen".to_owned()
        } else {
            list.join(", ")
        }
    };
    format!(
        "secure20: {}; secure: {}",
        names(API_ORIGIN),
        names(ASP_ORIGIN)
    )
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
            detail: None,
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
                window: None,
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
    fn names_a_missing_password_field() {
        assert!(fill_failed_message("\"nopassword\"").contains("wachtwoordveld"));
        assert!(fill_failed_message("geen antwoord").contains("inlogformulier"));
    }
}
