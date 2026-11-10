//! Calls to e-Boekhouden's internal API (`/v1/api/...`), GET only, with the
//! session's cookies, a shared rate limit and back-off after errors.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;
use tokio::sync::{Mutex, Semaphore};
use tokio::time::Instant as TokioInstant;
use url::Url;

pub const API_BASE: &str = "https://secure20.e-boekhouden.nl/v1/api/";
/// The same budget EB Sessions keeps (DECISIONS.md, 2026-10-30): 30 reads a minute, 2 at a time.
const PER_MINUTE: usize = 30;
const AT_ONCE: usize = 2;
const BACKOFF_START: Duration = Duration::from_secs(2);
const BACKOFF_MAX: Duration = Duration::from_secs(300);
const TIMEOUT: Duration = Duration::from_secs(30);
/// How long a call may wait for its turn under the rate limit before it gives up (Sherin, 2026-11-06).
const QUEUE_WAIT: Duration = Duration::from_secs(10);

/// One call as the request log keeps it. Secrets never get in here.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiCall {
    pub method: &'static str,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub status: u16,
    pub duration_ms: u64,
    pub bytes: usize,
    pub rows: Option<usize>,
    #[serde(skip)]
    pub json: Option<Value>,
    pub error: Option<String>,
}

impl ApiCall {
    pub fn ok(&self) -> bool {
        self.error.is_none() && (200..300).contains(&self.status)
    }
}

/// Where e-Boekhouden's own app runs; its API calls carry this as Origin.
const APP_ORIGIN: &str = "https://secure20.e-boekhouden.nl";

pub struct EbClient {
    base: Url,
    http: reqwest::Client,
    limiter: Limiter,
}

impl EbClient {
    pub fn new(base: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("EB MCP/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("HTTP client");
        Self {
            base: Url::parse(base).expect("API base"),
            http,
            limiter: Limiter::new(),
        }
    }

    pub fn production() -> Self {
        Self::new(API_BASE)
    }

    /// GET `path` (relative to /v1/api/) with the given cookies of the account's session.
    pub async fn get(
        &self,
        cookies: &[(String, String)],
        path: &str,
        query: &[(String, String)],
    ) -> ApiCall {
        let mut call = ApiCall {
            method: "GET",
            path: path.to_owned(),
            query: query.to_vec(),
            status: 0,
            duration_ms: 0,
            bytes: 0,
            rows: None,
            json: None,
            error: None,
        };
        let url = match self.url(path, query) {
            Ok(url) => url,
            Err(e) => {
                call.error = Some(e);
                return call;
            }
        };
        let Some(_permit) = self.limiter.acquire().await else {
            call.error = Some(
                "Te veel aanroepen tegelijk: EB MCP houdt e-Boekhouden rustig (30 per minuut). Probeer het over een minuut opnieuw.".into(),
            );
            return call;
        };
        let started = Instant::now();
        let mut request = self
            .http
            .get(url)
            .header(reqwest::header::ACCEPT, "application/json");
        if !cookies.is_empty() {
            request = request.header(reqwest::header::COOKIE, cookie_header(cookies));
        }
        if let Some(token) = auth_token(cookies) {
            request = request.bearer_auth(token);
        }
        let result = request.send().await;
        call.duration_ms = started.elapsed().as_millis() as u64;
        match result {
            Err(e) => {
                self.limiter.failed(None).await;
                call.error = Some(if e.is_timeout() {
                    "e-Boekhouden antwoordde niet binnen 30 s".into()
                } else {
                    "Geen verbinding met e-Boekhouden".into()
                });
            }
            Ok(response) => {
                call.status = response.status().as_u16();
                let retry = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .map(Duration::from_secs);
                let body = response.bytes().await.unwrap_or_default();
                call.duration_ms = started.elapsed().as_millis() as u64;
                call.bytes = body.len();
                if call.status == 429 || call.status >= 500 {
                    self.limiter.failed(retry).await;
                } else {
                    self.limiter.succeeded().await;
                }
                match call.status {
                    200..=299 => match serde_json::from_slice::<Value>(&body) {
                        Ok(json) => {
                            call.rows = count_rows(&json);
                            call.json = Some(json);
                        }
                        Err(_) => call.error = Some("e-Boekhouden gaf geen JSON terug".into()),
                    },
                    300..=399 | 401 | 403 => {
                        call.error = Some("Niet (meer) ingelogd bij e-Boekhouden".into())
                    }
                    404 => call.error = Some("Dit pad bestaat niet (meer) bij e-Boekhouden".into()),
                    429 => call.error = Some("e-Boekhouden vraagt om rustiger aan te doen".into()),
                    status => call.error = Some(format!("e-Boekhouden gaf een fout ({status})")),
                }
            }
        }
        call
    }

    /// `POST /v1/api/auth/login`, how e-Boekhouden's own app signs in: username, password and, when the account
    /// asks for it, the two-step code. The answer's token becomes the `auth-token` cookie the API reads, next to
    /// the cookies the answer sets. The password goes only into this request's body.
    pub async fn login(
        &self,
        username: &str,
        password: &str,
        code: Option<&str>,
    ) -> (ApiCall, Login) {
        let mut call = ApiCall {
            method: "POST",
            path: "auth/login".into(),
            query: Vec::new(),
            status: 0,
            duration_ms: 0,
            bytes: 0,
            rows: None,
            json: None,
            error: None,
        };
        let url = self.base.join("auth/login").expect("login URL");
        // As their app sends it: `twoFactorCode` only when there is a code (theirs is `undefined`, so left out).
        let mut body = serde_json::json!({ "username": username, "password": password });
        if let Some(code) = code {
            body["twoFactorCode"] = Value::String(code.to_owned());
        }
        let started = Instant::now();
        let result = self
            .http
            .post(url)
            .header(reqwest::header::ACCEPT, "application/json")
            // What the browser adds for their app on secure20.
            .header(reqwest::header::ORIGIN, APP_ORIGIN)
            .header(reqwest::header::REFERER, format!("{APP_ORIGIN}/login"))
            .json(&body)
            .send()
            .await;
        let outcome = match result {
            Err(e) => Login::Error(if e.is_timeout() {
                "e-Boekhouden antwoordde niet binnen 30 s".into()
            } else {
                "Geen verbinding met e-Boekhouden".into()
            }),
            Ok(response) => {
                call.status = response.status().as_u16();
                let set: Vec<(String, String)> = response
                    .headers()
                    .get_all(reqwest::header::SET_COOKIE)
                    .iter()
                    .filter_map(|v| v.to_str().ok())
                    .filter_map(|v| v.split(';').next())
                    .filter_map(|pair| pair.split_once('='))
                    .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
                    .filter(|(k, _)| !k.is_empty())
                    .collect();
                let bytes = response.bytes().await.unwrap_or_default();
                call.bytes = bytes.len();
                let json = serde_json::from_slice::<Value>(&bytes).ok();
                let text = String::from_utf8_lossy(&bytes).into_owned();
                login_outcome(call.status, json.as_ref(), &text, set)
            }
        };
        call.duration_ms = started.elapsed().as_millis() as u64;
        if let Login::Refused(m) | Login::Error(m) = &outcome {
            call.error = Some(m.clone());
        }
        if matches!(outcome, Login::NeedsCode) {
            call.error = Some("e-Boekhouden vraagt om een tweestapscode".into());
        }
        (call, outcome)
    }

    /// `POST /v1/api/auth`, the check e-Boekhouden's own app makes at startup: whether these cookies make a
    /// signed-in session. The answer's `valid` says so; `valid_session` reads it.
    pub async fn auth(&self, cookies: &[(String, String)]) -> ApiCall {
        let mut call = ApiCall {
            method: "POST",
            path: "auth".into(),
            query: Vec::new(),
            status: 0,
            duration_ms: 0,
            bytes: 0,
            rows: None,
            json: None,
            error: None,
        };
        let url = self.base.join("auth").expect("auth URL");
        let started = Instant::now();
        let mut request = self
            .http
            .post(url)
            .header(reqwest::header::ACCEPT, "application/json")
            // As e-Boekhouden's own app posts it: JSON content type, no body. Without it the API answers 415.
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::CONTENT_LENGTH, "0");
        if !cookies.is_empty() {
            request = request.header(reqwest::header::COOKIE, cookie_header(cookies));
        }
        if let Some(token) = auth_token(cookies) {
            request = request.bearer_auth(token);
        }
        match request.send().await {
            Err(_) => call.error = Some("Geen verbinding met e-Boekhouden".into()),
            Ok(response) => {
                call.status = response.status().as_u16();
                let body = response.bytes().await.unwrap_or_default();
                call.bytes = body.len();
                call.json = serde_json::from_slice::<Value>(&body).ok();
                if !valid_session(&call) {
                    call.error = Some("Nog geen geldige sessie bij e-Boekhouden".into());
                }
            }
        }
        call.duration_ms = started.elapsed().as_millis() as u64;
        call
    }

    fn url(&self, path: &str, query: &[(String, String)]) -> Result<Url, String> {
        let path = path.trim_start_matches('/');
        if path.is_empty()
            || path.contains("..")
            || path.contains("://")
            || path.contains('?')
            || path.contains('#')
        {
            return Err("Ongeldig pad".into());
        }
        let mut url = self
            .base
            .join(path)
            .map_err(|_| "Ongeldig pad".to_owned())?;
        if !url.as_str().starts_with(self.base.as_str()) {
            return Err("Ongeldig pad".into());
        }
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        Ok(url)
    }
}

pub fn cookie_header(cookies: &[(String, String)]) -> String {
    cookies
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ")
}

/// What `auth/login` answered.
#[derive(Debug, PartialEq)]
pub enum Login {
    /// Signed in: the session's cookies, `auth-token` among them. Never logged or shown.
    Session(Vec<(String, String)>),
    /// The account uses two-step verification: sign in again with the code.
    NeedsCode,
    /// e-Boekhouden refused, with its own message where it gave one.
    Refused(String),
    Error(String),
}

const TWO_STEP: [&str; 7] = [
    "twofactor",
    "two-factor",
    "two factor",
    "2fa",
    "tweestap",
    "authenticator",
    "verificatiecode",
];

/// Reads `auth/login`'s answer. Pure, so it can be tested without e-Boekhouden.
pub fn login_outcome(
    status: u16,
    json: Option<&Value>,
    text: &str,
    mut cookies: Vec<(String, String)>,
) -> Login {
    let token = json
        .and_then(|j| j.get("token"))
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty());
    if (200..300).contains(&status)
        && let Some(token) = token
    {
        cookies.retain(|(k, _)| k != "auth-token");
        cookies.push(("auth-token".into(), token.to_owned()));
        return Login::Session(cookies);
    }
    let lower = text.to_lowercase();
    if (200..300).contains(&status) || status == 400 || status == 401 || status == 403 {
        if TWO_STEP.iter().any(|w| lower.contains(w)) {
            return Login::NeedsCode;
        }
        let message = json
            .and_then(|j| {
                ["message", "error", "detail", "title"]
                    .iter()
                    .find_map(|k| j.get(*k).and_then(Value::as_str))
            })
            .map(str::to_owned)
            .or_else(|| json.and_then(Value::as_str).map(str::to_owned))
            .or_else(|| {
                (!text.trim().is_empty() && text.len() < 300 && !text.contains('<'))
                    .then(|| text.trim().to_owned())
            })
            .unwrap_or_else(|| "e-Boekhouden weigerde de gebruikersnaam of het wachtwoord".into());
        return Login::Refused(message.chars().take(300).collect());
    }
    if status == 429 {
        return Login::Refused("e-Boekhouden vraagt om even te wachten met inloggen".into());
    }
    let detail = json
        .and_then(|j| {
            ["message", "error", "detail", "title"]
                .iter()
                .find_map(|k| j.get(*k).and_then(Value::as_str))
        })
        .map(str::to_owned)
        .or_else(|| {
            (!text.trim().is_empty() && !text.contains('<')).then(|| text.trim().to_owned())
        })
        .map(|d| format!(": {}", d.chars().take(200).collect::<String>()))
        .unwrap_or_default();
    Login::Error(format!(
        "e-Boekhouden gaf een fout bij het inloggen ({status}){detail}"
    ))
}

/// Whether `POST auth` said the session is signed in (`{"valid": true, ...}`).
pub fn valid_session(call: &ApiCall) -> bool {
    (200..300).contains(&call.status)
        && call
            .json
            .as_ref()
            .and_then(|j| j.get("valid"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

/// e-Boekhouden's app reads its JWT from the script-set `auth-token` cookie.
pub fn auth_token(cookies: &[(String, String)]) -> Option<&str> {
    cookies
        .iter()
        .find(|(k, _)| k == "auth-token")
        .map(|(_, v)| v.as_str())
        .filter(|v| !v.is_empty())
}

/// Rows in a list answer: a top-level array, or the first array under a usual key.
pub fn count_rows(json: &Value) -> Option<usize> {
    if let Some(list) = json.as_array() {
        return Some(list.len());
    }
    let object = json.as_object()?;
    for key in [
        "items", "Items", "data", "Data", "rows", "Rows", "result", "Result", "records",
    ] {
        if let Some(list) = object.get(key).and_then(Value::as_array) {
            return Some(list.len());
        }
    }
    None
}

/// 30 starts per minute, 2 at a time, and a growing pause after errors.
struct Limiter {
    slots: Semaphore,
    state: Mutex<LimiterState>,
}

struct LimiterState {
    starts: VecDeque<TokioInstant>,
    failures: u32,
    paused_until: Option<TokioInstant>,
}

impl Limiter {
    fn new() -> Self {
        Self {
            slots: Semaphore::new(AT_ONCE),
            state: Mutex::new(LimiterState {
                starts: VecDeque::new(),
                failures: 0,
                paused_until: None,
            }),
        }
    }

    /// A place under the limit, or None when none comes free within QUEUE_WAIT.
    async fn acquire(&self) -> Option<tokio::sync::SemaphorePermit<'_>> {
        tokio::time::timeout(QUEUE_WAIT, self.wait_for_turn())
            .await
            .ok()
    }

    async fn wait_for_turn(&self) -> tokio::sync::SemaphorePermit<'_> {
        let permit = self.slots.acquire().await.expect("limiter closed");
        loop {
            let wait = {
                let mut state = self.state.lock().await;
                let now = TokioInstant::now();
                while state
                    .starts
                    .front()
                    .is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(60))
                {
                    state.starts.pop_front();
                }
                let pause = state.paused_until.filter(|t| *t > now).map(|t| t - now);
                let window = (state.starts.len() >= PER_MINUTE)
                    .then(|| Duration::from_secs(60) - now.duration_since(state.starts[0]));
                match pause.max(window) {
                    Some(wait) => wait,
                    None => {
                        state.starts.push_back(now);
                        return permit;
                    }
                }
            };
            tokio::time::sleep(wait).await;
        }
    }

    async fn failed(&self, retry_after: Option<Duration>) {
        let mut state = self.state.lock().await;
        state.failures += 1;
        let backoff = (BACKOFF_START * 2u32.saturating_pow(state.failures - 1)).min(BACKOFF_MAX);
        let wait = backoff.max(retry_after.unwrap_or_default().min(BACKOFF_MAX));
        state.paused_until = Some(TokioInstant::now() + wait);
    }

    async fn succeeded(&self) {
        let mut state = self.state.lock().await;
        state.failures = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, http::HeaderMap, routing::get};
    use serde_json::json;

    async fn mock() -> String {
        let app = Router::new()
            .route(
                "/v1/api/relatie/gridtable",
                get(|headers: HeaderMap| async move {
                    let cookie = headers.get("cookie").and_then(|v| v.to_str().ok()).unwrap_or_default().to_owned();
                    let auth = headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or_default().to_owned();
                    if !cookie.contains("auth-token=jwt") || auth != "Bearer jwt" {
                        return (axum::http::StatusCode::UNAUTHORIZED, String::new());
                    }
                    (axum::http::StatusCode::OK, json!({"items": [{"naam": "Bakkerij de Korf"}, {"naam": "Bakkerij Zuidwal"}], "totaal": 2}).to_string())
                }),
            )
            .route("/v1/api/kapot", get(|| async { (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "boom") }))
            .route(
                "/v1/api/auth/login",
                axum::routing::post(|body: String| async move {
                    let body: Value = serde_json::from_str(&body).unwrap_or_default();
                    // e-Boekhouden answers 500 on an empty code; their app leaves the field out.
                    if body.get("twoFactorCode").and_then(Value::as_str) == Some("") {
                        return (
                            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                            [("set-cookie", "SRVRID=9s13; path=/; Secure")],
                            "Object reference not set".to_owned(),
                        );
                    }
                    match (body["username"].as_str(), body["password"].as_str(), body["twoFactorCode"].as_str()) {
                        (Some("jan@studionoord.example"), Some("geheim"), Some("123456")) => (
                            axum::http::StatusCode::OK,
                            [("set-cookie", "SRVRID=9s13; path=/; Secure")],
                            json!({"token": "jwt"}).to_string(),
                        ),
                        (Some("jan@studionoord.example"), Some("geheim"), _) => (
                            axum::http::StatusCode::BAD_REQUEST,
                            [("set-cookie", "SRVRID=9s13; path=/; Secure")],
                            json!({"message": "TwoFactorCodeRequired"}).to_string(),
                        ),
                        _ => (
                            axum::http::StatusCode::UNAUTHORIZED,
                            [("set-cookie", "SRVRID=9s13; path=/; Secure")],
                            json!({"message": "Gebruikersnaam of wachtwoord onjuist"}).to_string(),
                        ),
                    }
                }),
            )
            // Like e-Boekhouden: without a JSON content type the API answers 415.
            .route(
                "/v1/api/auth",
                axum::routing::post(|headers: HeaderMap| async move {
                    let json_type = headers.get("content-type").and_then(|v| v.to_str().ok()) == Some("application/json");
                    let cookie = headers.get("cookie").and_then(|v| v.to_str().ok()).unwrap_or_default().to_owned();
                    if !json_type {
                        return (axum::http::StatusCode::UNSUPPORTED_MEDIA_TYPE, String::new());
                    }
                    (axum::http::StatusCode::OK, json!({"valid": cookie.contains("ISLOGINX")}).to_string())
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}/v1/api/")
    }

    fn cookies() -> Vec<(String, String)> {
        vec![
            ("ASPSESSIONID".into(), "x".into()),
            ("auth-token".into(), "jwt".into()),
        ]
    }

    #[tokio::test]
    async fn sends_the_session_and_counts_rows() {
        let client = EbClient::new(&mock().await);
        let call = client
            .get(
                &cookies(),
                "relatie/gridtable",
                &[("zoek".into(), "bakker".into())],
            )
            .await;
        assert!(call.ok(), "{call:?}");
        assert_eq!(call.rows, Some(2));
        assert!(call.bytes > 0);
    }

    #[tokio::test]
    async fn names_a_missing_session_and_server_errors() {
        let client = EbClient::new(&mock().await);
        let call = client.get(&[], "relatie/gridtable", &[]).await;
        assert_eq!(call.status, 401);
        assert_eq!(
            call.error.as_deref(),
            Some("Niet (meer) ingelogd bij e-Boekhouden")
        );
        let call = client.get(&cookies(), "kapot", &[]).await;
        assert_eq!(call.status, 500);
        assert!(!call.ok());
    }

    #[tokio::test]
    async fn refuses_paths_that_leave_the_api() {
        let client = EbClient::new(&mock().await);
        for bad in [
            "../bh/uitloggen.asp",
            "https://evil.example/x",
            "relatie?x=1",
            "",
        ] {
            let call = client.get(&cookies(), bad, &[]).await;
            assert_eq!(call.error.as_deref(), Some("Ongeldig pad"), "{bad}");
            assert_eq!(call.status, 0);
        }
    }

    #[test]
    fn counts_rows_in_usual_shapes() {
        assert_eq!(count_rows(&json!([1, 2, 3])), Some(3));
        assert_eq!(count_rows(&json!({"data": [1]})), Some(1));
        assert_eq!(count_rows(&json!({"naam": "x"})), None);
    }

    #[tokio::test(start_paused = true)]
    async fn keeps_to_thirty_a_minute() {
        let limiter = Limiter::new();
        let start = tokio::time::Instant::now();
        for _ in 0..PER_MINUTE {
            assert!(limiter.acquire().await.is_some());
        }
        assert!(start.elapsed() < Duration::from_secs(1));
        // The 31st waits at most 10 s for a place, then gives up instead of hanging for the rest of the minute.
        assert!(limiter.acquire().await.is_none());
        let waited = start.elapsed();
        assert!(
            waited >= Duration::from_secs(10) && waited < Duration::from_secs(11),
            "{waited:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn pauses_longer_after_each_failure() {
        let limiter = Limiter::new();
        limiter.failed(None).await;
        let start = tokio::time::Instant::now();
        assert!(limiter.acquire().await.is_some());
        assert!(start.elapsed() >= Duration::from_secs(2));
        limiter.failed(Some(Duration::from_secs(6))).await;
        let start = tokio::time::Instant::now();
        assert!(limiter.acquire().await.is_some());
        assert!(start.elapsed() >= Duration::from_secs(6));
        // A pause longer than the queue wait fails the call instead of holding it for minutes.
        limiter.failed(Some(Duration::from_secs(120))).await;
        assert!(limiter.acquire().await.is_none());
    }

    #[test]
    fn reads_whether_auth_says_signed_in() {
        let call = |status: u16, json: Option<Value>| ApiCall {
            method: "POST",
            path: "auth".into(),
            query: Vec::new(),
            status,
            duration_ms: 0,
            bytes: 0,
            rows: None,
            json,
            error: None,
        };
        assert!(valid_session(&call(
            200,
            Some(serde_json::json!({"valid": true, "abId": 1}))
        )));
        assert!(!valid_session(&call(
            200,
            Some(serde_json::json!({"valid": false}))
        )));
        assert!(!valid_session(&call(
            401,
            Some(serde_json::json!({"valid": true}))
        )));
        assert!(!valid_session(&call(200, None)));
    }

    #[tokio::test]
    async fn checks_the_session_with_a_json_post() {
        let client = EbClient::new(&mock().await);
        let signed_in = client.auth(&[("ISLOGINX".into(), "1".into())]).await;
        assert_eq!(signed_in.status, 200, "{:?}", signed_in.error);
        assert!(valid_session(&signed_in));
        let not_yet = client.auth(&[("SRVRID".into(), "1".into())]).await;
        assert_eq!(not_yet.status, 200);
        assert!(!valid_session(&not_yet));
        assert!(not_yet.error.is_some());
    }

    #[tokio::test]
    async fn signs_in_like_e_boekhoudens_app_with_a_two_step_code() {
        let client = EbClient::new(&mock().await);
        let (call, wrong) = client.login("jan@studionoord.example", "fout", None).await;
        assert_eq!(call.status, 401);
        assert_eq!(
            wrong,
            Login::Refused("Gebruikersnaam of wachtwoord onjuist".into())
        );
        let (_, needs) = client
            .login("jan@studionoord.example", "geheim", None)
            .await;
        assert_eq!(needs, Login::NeedsCode);
        let (call, ok) = client
            .login("jan@studionoord.example", "geheim", Some("123456"))
            .await;
        assert_eq!(call.status, 200);
        assert_eq!(
            ok,
            Login::Session(vec![
                ("SRVRID".into(), "9s13".into()),
                ("auth-token".into(), "jwt".into())
            ])
        );
        assert!(call.error.is_none());
    }

    #[test]
    fn reads_the_login_answer() {
        let json = |v: Value| Some(v);
        assert!(matches!(
            login_outcome(200, json(json!({"token": "t"})).as_ref(), "", vec![]),
            Login::Session(_)
        ));
        assert_eq!(
            login_outcome(200, json(json!({"token": ""})).as_ref(), "", vec![]),
            Login::Refused("e-Boekhouden weigerde de gebruikersnaam of het wachtwoord".into())
        );
        assert_eq!(
            login_outcome(400, None, "Two-factor authentication code required", vec![]),
            Login::NeedsCode
        );
        assert_eq!(
            login_outcome(401, json(json!("Account geblokkeerd")).as_ref(), "", vec![]),
            Login::Refused("Account geblokkeerd".into())
        );
        assert_eq!(
            login_outcome(401, None, "<html>nope</html>", vec![]),
            Login::Refused("e-Boekhouden weigerde de gebruikersnaam of het wachtwoord".into())
        );
        assert!(matches!(
            login_outcome(502, None, "", vec![]),
            Login::Error(_)
        ));
        assert_eq!(
            login_outcome(500, None, "Object reference not set", vec![]),
            Login::Error(
                "e-Boekhouden gaf een fout bij het inloggen (500): Object reference not set".into()
            )
        );
        assert_eq!(
            login_outcome(500, None, "<html>stack</html>", vec![]),
            Login::Error("e-Boekhouden gaf een fout bij het inloggen (500)".into())
        );
    }
}
