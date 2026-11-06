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
            .route("/v1/api/kapot", get(|| async { (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "boom") }));
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
}
