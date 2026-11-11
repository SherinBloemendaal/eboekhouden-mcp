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
/// The same budget EB Sessions keeps (DECISIONS.md, 2026-10-30) for how many run at a time: 2.
/// 60 starts a minute (Sherin's trial of 0.2.0, 2026-11-11: with the earlier 30 a tool that changes something was
/// refused on its own reads; it reads a handful of things first). e-Boekhouden's own pages make more calls than
/// this per page. Shown as the limit line on the Dashboard (LIMIT_PER_MINUTE in DashboardView.vue).
const PER_MINUTE: usize = 60;
const AT_ONCE: usize = 2;
const BACKOFF_START: Duration = Duration::from_secs(2);
const BACKOFF_MAX: Duration = Duration::from_secs(300);
const TIMEOUT: Duration = Duration::from_secs(30);
/// How long a call may wait for its turn under the rate limit before it gives up (Sherin, 2026-11-06).
const QUEUE_WAIT: Duration = Duration::from_secs(10);
/// A change was approved by the user: it waits out a full window rather than fail on EB MCP's own limit.
const CHANGE_WAIT: Duration = Duration::from_secs(75);

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

/// e-Boekhouden's classic pages, where its login form lives (`inloggen.asp`).
/// The template editor (factuur- and offertesjablonen): the API sends the browser there with a session hash.
pub const EDITOR_BASE: &str = "https://editor.e-boekhouden.nl/";

pub const CLASSIC_BASE: &str = "https://secure.e-boekhouden.nl/bh/";
/// The login form is a browser page; it gets a browser's User-Agent, as the page expects.
const BROWSER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

pub struct EbClient {
    base: Url,
    classic: Url,
    /// e-Boekhouden's template editor, a site of its own with its own small API.
    editor: Url,
    http: reqwest::Client,
    limiter: Limiter,
    /// Cookies the template editor set for an open session (its sticky server among them), by session hash.
    editor_cookies: std::sync::Mutex<std::collections::HashMap<String, Vec<(String, String)>>>,
}

impl EbClient {
    /// The API at `base` with the classic pages under `/bh/` on the same host (tests, with a mock).
    #[cfg(test)]
    pub fn new(base: &str) -> Self {
        let classic = Url::parse(base)
            .and_then(|url| url.join("/bh/"))
            .expect("classic base");
        let editor = Url::parse(base)
            .and_then(|url| url.join("/editor/"))
            .expect("editor base");
        Self::with_hosts(base, classic.as_str(), editor.as_str())
    }

    pub fn with_hosts(base: &str, classic: &str, editor: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("EB MCP/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("HTTP client");
        Self {
            base: Url::parse(base).expect("API base"),
            classic: Url::parse(classic).expect("classic base"),
            editor: Url::parse(editor).expect("editor base"),
            http,
            limiter: Limiter::new(),
            editor_cookies: Default::default(),
        }
    }

    pub fn production() -> Self {
        Self::with_hosts(API_BASE, CLASSIC_BASE, EDITOR_BASE)
    }

    /// GET `path` (relative to /v1/api/) with the given cookies of the account's session.
    #[cfg(test)]
    pub async fn get(
        &self,
        cookies: &[(String, String)],
        path: &str,
        query: &[(String, String)],
    ) -> ApiCall {
        self.send(cookies, "GET", path, query, None).await
    }

    /// Opens a session in the template editor for a factuur- or offertesjabloon. e-Boekhouden's API answers
    /// `GET <kind>sjabloon/editor/<id>/<taal>` with a redirect to the editor; its `sessionHash` is the key to the
    /// editor's own API (`editor`). Checked against the real editor on 2026-11-11.
    pub async fn editor_open(
        &self,
        cookies: &[(String, String)],
        path: &str,
    ) -> (ApiCall, Option<String>) {
        let mut call = ApiCall {
            method: "GET",
            path: path.to_owned(),
            query: Vec::new(),
            status: 0,
            duration_ms: 0,
            bytes: 0,
            rows: None,
            json: None,
            error: None,
        };
        let url = match self.url(path, &[]) {
            Ok(url) => url,
            Err(e) => {
                call.error = Some(e);
                return (call, None);
            }
        };
        let _permit = match self.limiter.acquire(QUEUE_WAIT).await {
            Ok(permit) => permit,
            Err(busy) => {
                call.error = Some(busy.message());
                return (call, None);
            }
        };
        let started = Instant::now();
        let mut request = self
            .http
            .get(url)
            .header(reqwest::header::COOKIE, cookie_header(cookies));
        if let Some(token) = auth_token(cookies) {
            request = request.bearer_auth(token);
        }
        let mut hash = None;
        match request.send().await {
            Err(_) => call.error = Some("Geen verbinding met e-Boekhouden".into()),
            Ok(response) => {
                call.status = response.status().as_u16();
                let mut session = with_set_cookies(cookies, response.headers());
                let target = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|location| self.editor.join(location).ok());
                hash = target.as_ref().and_then(|url| {
                    url.query_pairs()
                        .find(|(key, _)| key == "sessionHash")
                        .map(|(_, value)| value.into_owned())
                        .filter(|value| {
                            !value.is_empty() && value.chars().all(|c| c.is_ascii_alphanumeric())
                        })
                });
                if let (Some(hash), Some(page)) = (&hash, target) {
                    // The browser lands on the editor's page first, and that page's answer names the server that
                    // holds the session. Without that cookie the editor's API answers 500 (seen live, 0.3.1).
                    if let Ok(landed) = self
                        .http
                        .get(page)
                        .header(reqwest::header::COOKIE, cookie_header(&session))
                        .send()
                        .await
                    {
                        session = with_set_cookies(&session, landed.headers());
                    }
                    self.editor_cookies
                        .lock()
                        .unwrap()
                        .insert(hash.clone(), session);
                }
                call.error = match (call.status, &hash) {
                    (300..=399, Some(_)) => None,
                    // A redirect anywhere else is the login page.
                    (300..=399, None) | (401, _) => {
                        Some("Niet (meer) ingelogd bij e-Boekhouden".into())
                    }
                    (404, _) => Some("Dit sjabloon bestaat niet (meer) bij e-Boekhouden".into()),
                    (status, _) => Some(format!(
                        "e-Boekhouden opende de sjabloon-editor niet ({status})"
                    )),
                };
            }
        }
        call.duration_ms = started.elapsed().as_millis() as u64;
        (call, hash)
    }

    /// One call on the template editor's own API (`api/session/<hash>`, `api/sjabloon/codes/<hash>`,
    /// `api/session/end/<hash>`), with the session's cookies: without them the editor answers 500.
    pub async fn editor(
        &self,
        cookies: &[(String, String)],
        method: &'static str,
        path: &str,
        body: Option<&Value>,
    ) -> ApiCall {
        let mut call = ApiCall {
            method,
            // The session hash is the key to the session: it stays out of the log.
            path: match path.rsplit_once('/') {
                Some((head, _)) => format!("editor/{head}/…"),
                None => format!("editor/{path}"),
            },
            query: Vec::new(),
            status: 0,
            duration_ms: 0,
            bytes: 0,
            rows: None,
            json: None,
            error: None,
        };
        let Ok(url) = self.editor.join(path) else {
            call.error = Some("Ongeldig pad".into());
            return call;
        };
        let started = Instant::now();
        let verb = if method == "POST" {
            reqwest::Method::POST
        } else {
            reqwest::Method::GET
        };
        // The session's own cookies, as collected when it was opened; they end with the session.
        let hash = path.rsplit('/').next().unwrap_or_default();
        let session = {
            let mut known = self.editor_cookies.lock().unwrap();
            if path.contains("session/end/") {
                known.remove(hash)
            } else {
                known.get(hash).cloned()
            }
        };
        let cookies = session.as_deref().unwrap_or(cookies);
        let mut request = self
            .http
            .request(verb, url)
            .header(reqwest::header::ACCEPT, "application/json, text/plain, */*")
            .header(reqwest::header::COOKIE, cookie_header(cookies));
        if let Some(body) = body {
            request = request.json(body);
        }
        match request.send().await {
            Err(e) => {
                call.error = Some(if e.is_timeout() {
                    "De sjabloon-editor van e-Boekhouden antwoordde niet binnen 30 s".into()
                } else {
                    "Geen verbinding met de sjabloon-editor van e-Boekhouden".into()
                })
            }
            Ok(response) => {
                call.status = response.status().as_u16();
                let bytes = response.bytes().await.unwrap_or_default();
                call.bytes = bytes.len();
                match call.status {
                    200..=299 => {
                        call.json = serde_json::from_slice::<Value>(&bytes)
                            .ok()
                            .map(|mut json| {
                                without_session_keys(&mut json);
                                json
                            })
                    }
                    status => {
                        call.error = Some(format!(
                            "De sjabloon-editor van e-Boekhouden gaf een fout ({status})"
                        ))
                    }
                }
            }
        }
        call.duration_ms = started.elapsed().as_millis() as u64;
        call
    }

    /// Fetches a file (an invoice's PDF) in the account's session: the call for the log, and on success the
    /// bytes with the file name e-Boekhouden gives it.
    pub async fn download(
        &self,
        cookies: &[(String, String)],
        path: &str,
    ) -> (ApiCall, Option<(Vec<u8>, Option<String>)>) {
        let mut call = ApiCall {
            method: "GET",
            path: path.to_owned(),
            query: Vec::new(),
            status: 0,
            duration_ms: 0,
            bytes: 0,
            rows: None,
            json: None,
            error: None,
        };
        let url = match self.url(path, &[]) {
            Ok(url) => url,
            Err(e) => {
                call.error = Some(e);
                return (call, None);
            }
        };
        let _permit = match self.limiter.acquire(QUEUE_WAIT).await {
            Ok(permit) => permit,
            Err(busy) => {
                call.error = Some(busy.message());
                return (call, None);
            }
        };
        let started = Instant::now();
        let mut request = self.http.get(url);
        if !cookies.is_empty() {
            request = request.header(reqwest::header::COOKIE, cookie_header(cookies));
        }
        if let Some(token) = auth_token(cookies) {
            request = request.bearer_auth(token);
        }
        let mut file = None;
        match request.send().await {
            Err(_) => {
                self.limiter.failed(None).await;
                call.error = Some("Geen verbinding met e-Boekhouden".into());
            }
            Ok(response) => {
                call.status = response.status().as_u16();
                let name = response
                    .headers()
                    .get(reqwest::header::CONTENT_DISPOSITION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(file_name_of);
                let body = response.bytes().await.unwrap_or_default();
                call.bytes = body.len();
                if call.status == 429 || call.status >= 500 {
                    self.limiter.failed(None).await;
                } else {
                    self.limiter.succeeded().await;
                }
                match call.status {
                    200..=299 if body.is_empty() => {
                        call.error = Some("e-Boekhouden gaf een leeg bestand terug".into())
                    }
                    200..=299 => file = Some((body.to_vec(), name)),
                    300..=399 | 401 => {
                        call.error = Some("Niet (meer) ingelogd bij e-Boekhouden".into())
                    }
                    404 => {
                        call.error = Some("Dit bestand bestaat niet (meer) bij e-Boekhouden".into())
                    }
                    status => call.error = Some(format!("e-Boekhouden gaf een fout ({status})")),
                }
            }
        }
        call.duration_ms = started.elapsed().as_millis() as u64;
        (call, file)
    }

    /// Runs a job on e-Boekhouden's hub (hub.rs) in the account's session and waits for its end. Not rate
    /// limited with the other calls: it is one long connection, made at most once per approved tool call.
    pub async fn hub(
        &self,
        cookies: &[(String, String)],
        hub: &str,
        target: &str,
        argument: &Value,
    ) -> ApiCall {
        let started = Instant::now();
        let mut call = ApiCall {
            method: "HUB",
            path: format!("hub/{hub}"),
            query: vec![("target".into(), target.to_owned())],
            status: 0,
            duration_ms: 0,
            bytes: 0,
            rows: None,
            json: None,
            error: None,
        };
        match crate::hub::invoke(&self.base, cookies, hub, target, argument).await {
            Ok(crate::hub::Outcome::Done(result)) => {
                call.status = 200;
                call.json = Some(result);
            }
            Ok(crate::hub::Outcome::Refused(said)) => {
                call.status = 400;
                call.error = Some(format!("e-Boekhouden weigert dit: {said}"));
            }
            Err(error) => {
                if error.contains("ingelogd") {
                    call.status = 401;
                }
                call.error = Some(error);
            }
        }
        call.duration_ms = started.elapsed().as_millis() as u64;
        call
    }

    /// One call on e-Boekhouden's API in the account's session: GET to read, or POST, PUT or DELETE (with a
    /// JSON body where the API takes one) to change something. Who may change what is decided before this
    /// is called (tools.rs, approvals.rs); here it is one request, never retried.
    pub async fn send(
        &self,
        cookies: &[(String, String)],
        method: &'static str,
        path: &str,
        query: &[(String, String)],
        body: Option<&Value>,
    ) -> ApiCall {
        let mut call = ApiCall {
            method,
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
        let patience = if method == "GET" {
            QUEUE_WAIT
        } else {
            CHANGE_WAIT
        };
        let _permit = match self.limiter.acquire(patience).await {
            Ok(permit) => permit,
            Err(busy) => {
                call.error = Some(busy.message());
                return call;
            }
        };
        let started = Instant::now();
        let verb = match method {
            "POST" => reqwest::Method::POST,
            "PUT" => reqwest::Method::PUT,
            "DELETE" => reqwest::Method::DELETE,
            _ => reqwest::Method::GET,
        };
        let mut request = self
            .http
            .request(verb, url)
            .header(reqwest::header::ACCEPT, "application/json");
        if let Some(body) = body {
            request = request.json(body);
        }
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
                    // A change may be answered without a body: that is a success with nothing to read.
                    200..=299 if body.is_empty() => {}
                    200..=299 => match serde_json::from_slice::<Value>(&body) {
                        Ok(json) => {
                            call.rows = count_rows(&json);
                            call.json = Some(json);
                        }
                        Err(_) => call.error = Some("e-Boekhouden gaf geen JSON terug".into()),
                    },
                    300..=399 | 401 => {
                        call.error = Some("Niet (meer) ingelogd bij e-Boekhouden".into())
                    }
                    // Signed in, but without rights to this part: the session itself is fine.
                    403 => {
                        call.error = Some(
                            "Geen toegang tot dit onderdeel bij e-Boekhouden: dit account mag het niet inzien".into(),
                        )
                    }
                    404 => call.error = Some("Dit pad bestaat niet (meer) bij e-Boekhouden".into()),
                    429 => call.error = Some("e-Boekhouden vraagt om rustiger aan te doen".into()),
                    // What e-Boekhouden says about a request it refuses ("Onbekend grootboekrekening voor regel.")
                    // is the useful part; a server fault only carries its own error code.
                    status => {
                        let said = serde_json::from_slice::<Value>(&body).ok().and_then(|json| {
                            json.get("message")
                                .and_then(Value::as_str)
                                .map(str::trim)
                                .filter(|message| !message.is_empty())
                                .map(str::to_owned)
                        });
                        call.error = Some(match said {
                            Some(message) if status == 400 => {
                                format!("e-Boekhouden weigert dit: {message}")
                            }
                            Some(message) => format!("e-Boekhouden gaf een fout ({status}): {message}"),
                            None => format!("e-Boekhouden gaf een fout ({status})"),
                        });
                    }
                }
            }
        }
        call
    }

    /// Signs in the way the browser does (Sherin's capture of 2026-11-11): fetch `inloggen.asp` for its
    /// session cookies and the hidden `txtFormCheck`, then post the form. e-Boekhouden answers with the
    /// `auth-token` cookie its API reads. `device` stands in for the page's browser fingerprint (`FP`).
    /// The password goes only into the form's body. `notes` tell the diagnosis what came back, without values.
    pub async fn login(&self, username: &str, password: &str, device: u32) -> LoginRun {
        let mut run = LoginRun {
            calls: Vec::new(),
            notes: Vec::new(),
            outcome: Login::Error(String::new()),
        };
        let page_url = self.classic.join("inloggen.asp").expect("login page URL");
        let (call, page) = self.classic_call("GET", page_url.clone(), None, &[]).await;
        run.calls.push(call.clone());
        let Some(page) = page else {
            run.outcome = Login::Error(call.error.unwrap_or_default());
            return run;
        };
        let mut cookies = page.cookies;
        run.notes.push(format!(
            "Inlogpagina: status {}, cookies gezet: {}",
            call.status,
            cookie_names(&cookies)
        ));
        let Some(form_check) = input_value(&page.body, "txtFormCheck") else {
            let message = format!(
                "De inlogpagina van e-Boekhouden is veranderd: het veld txtFormCheck ontbreekt (status {})",
                call.status
            );
            run.calls.last_mut().expect("page call").error = Some(message.clone());
            run.outcome = Login::Error(message);
            return run;
        };
        let server = cookies
            .iter()
            .find(|c| c.name == "SRVRID")
            .map(|c| c.value.clone())
            .unwrap_or_default();
        let form = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("txtEmail", username)
            .append_pair("txtWachtwoord", password)
            .append_pair("txtFormCheck", &form_check)
            .append_pair("FP", &device.to_string())
            .append_pair("XSRVX", &server)
            .finish();
        let mut post_url = page_url.clone();
        post_url.set_query(Some("LOGIN=1"));
        let sent: Vec<(String, String)> = cookies
            .iter()
            .map(|c| (c.name.clone(), c.value.clone()))
            .collect();
        let (mut call, answer) = self
            .classic_call("POST", post_url, Some((form, page_url.as_str())), &sent)
            .await;
        let Some(answer) = answer else {
            run.outcome = Login::Error(call.error.clone().unwrap_or_default());
            run.calls.push(call);
            return run;
        };
        run.notes.push(format!(
            "Formulier verstuurd: status {}, cookies gezet: {}{}, paginatitel: {}, velden: {}",
            call.status,
            cookie_names(&answer.cookies),
            answer
                .location
                .as_deref()
                .map(|l| format!(", doorverwijzing naar {}", l.split('?').next().unwrap_or(l)))
                .unwrap_or_default(),
            page_title(&answer.body).unwrap_or_else(|| "geen".into()),
            {
                let names = input_names(&answer.body);
                if names.is_empty() {
                    "geen".to_owned()
                } else {
                    names.join(", ")
                }
            }
        ));
        for cookie in answer.cookies {
            cookies.retain(|c| c.name != cookie.name);
            cookies.push(cookie);
        }
        run.outcome = classic_outcome(call.status, &answer.body, &cookies);
        if let Login::Refused(m) | Login::Error(m) = &run.outcome {
            call.error = Some(m.clone());
        }
        run.calls.push(call);
        run
    }

    /// One request to the classic pages, as a browser would send it. The answer's body and cookies come back
    /// beside the call; `None` when there was no answer at all.
    async fn classic_call(
        &self,
        method: &'static str,
        url: Url,
        form: Option<(String, &str)>,
        cookies: &[(String, String)],
    ) -> (ApiCall, Option<ClassicAnswer>) {
        let mut call = ApiCall {
            method,
            path: format!(
                "bh/{}",
                url.path_segments()
                    .and_then(|mut s| s.next_back())
                    .unwrap_or_default()
            ),
            query: url.query_pairs().into_owned().collect(),
            status: 0,
            duration_ms: 0,
            bytes: 0,
            rows: None,
            json: None,
            error: None,
        };
        let origin = url.origin().ascii_serialization();
        let mut request = match form {
            Some((body, referer)) => self
                .http
                .post(url)
                .header(
                    reqwest::header::CONTENT_TYPE,
                    "application/x-www-form-urlencoded",
                )
                .header(reqwest::header::ORIGIN, origin)
                .header(reqwest::header::REFERER, referer)
                .body(body),
            None => self.http.get(url),
        }
        .header(reqwest::header::USER_AGENT, BROWSER_AGENT)
        .header(
            reqwest::header::ACCEPT,
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header(reqwest::header::ACCEPT_LANGUAGE, "nl-NL,nl;q=0.9");
        if !cookies.is_empty() {
            request = request.header(reqwest::header::COOKIE, cookie_header(cookies));
        }
        let started = Instant::now();
        let result = request.send().await;
        let answer = match result {
            Err(e) => {
                call.error = Some(if e.is_timeout() {
                    "e-Boekhouden antwoordde niet binnen 30 s".into()
                } else {
                    "Geen verbinding met e-Boekhouden".into()
                });
                None
            }
            Ok(response) => {
                call.status = response.status().as_u16();
                let cookies = set_cookies(response.headers());
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned);
                let bytes = response.bytes().await.unwrap_or_default();
                call.bytes = bytes.len();
                Some(ClassicAnswer {
                    body: String::from_utf8_lossy(&bytes).into_owned(),
                    cookies,
                    location,
                })
            }
        };
        call.duration_ms = started.elapsed().as_millis() as u64;
        (call, answer)
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

/// `cookies` with what a response set on top (`Set-Cookie`), a later value replacing an earlier one. e-Boekhouden
/// sits behind a load balancer that names the server in a cookie; a conversation that must stay on one server
/// (the template editor's session, a hub connection) has to send that cookie back.
pub fn with_set_cookies(
    cookies: &[(String, String)],
    headers: &reqwest::header::HeaderMap,
) -> Vec<(String, String)> {
    let mut all = cookies.to_vec();
    for header in headers.get_all(reqwest::header::SET_COOKIE) {
        if let Some(set) = header.to_str().ok().and_then(parse_set_cookie) {
            all.retain(|(name, _)| *name != set.name);
            all.push((set.name, set.value));
        }
    }
    all
}

/// Takes the editor's session keys out of what it answers: they are the keys to an open session and have no
/// business in a tool's answer, the request log or the debug export.
pub fn without_session_keys(value: &mut Value) {
    match value {
        Value::Object(object) => {
            object.remove("sessionHash");
            object.remove("sessionKey");
            object.values_mut().for_each(without_session_keys);
        }
        Value::Array(items) => items.iter_mut().for_each(without_session_keys),
        _ => {}
    }
}

/// The file name in a `Content-Disposition` header (`attachment; filename=F00002.pdf`), without any folder.
pub fn file_name_of(disposition: &str) -> Option<String> {
    let value = disposition
        .split(';')
        .map(str::trim)
        .find_map(|part| part.strip_prefix("filename="))?;
    let name = value.trim_matches('"').rsplit(['/', '\\']).next()?.trim();
    (!name.is_empty() && name != "." && name != "..").then(|| name.to_owned())
}

pub fn cookie_header(cookies: &[(String, String)]) -> String {
    cookies
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ")
}

/// How a sign-in on the classic form ended.
#[derive(Debug, PartialEq)]
pub enum Login {
    /// Signed in: the session's cookies for the API, `auth-token` among them. Never logged or shown.
    Session(Vec<(String, String)>),
    /// The page asks for a two-step code.
    NeedsCode,
    /// e-Boekhouden refused, with its own message where it gave one.
    Refused(String),
    Error(String),
}

/// A sign-in with what it sent: the calls for Verzoeken and notes for the diagnosis. No secret in either.
pub struct LoginRun {
    pub calls: Vec<ApiCall>,
    pub notes: Vec<String>,
    pub outcome: Login,
}

/// A cookie from a `Set-Cookie` header. `shared` when it names a domain: the browser then sends it to
/// e-Boekhouden's API host as well, which is how the classic login hands its token to the API.
#[derive(Clone, Debug, PartialEq)]
pub struct SetCookie {
    pub name: String,
    pub value: String,
    pub shared: bool,
}

struct ClassicAnswer {
    body: String,
    cookies: Vec<SetCookie>,
    location: Option<String>,
}

const TWO_STEP: [&str; 8] = [
    "twofactor",
    "two-factor",
    "two factor",
    "2fa",
    "tweestap",
    "authenticator",
    "verificatiecode",
    "verificatie code",
];

fn set_cookies(headers: &reqwest::header::HeaderMap) -> Vec<SetCookie> {
    headers
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(parse_set_cookie)
        .collect()
}

/// `name=value; Domain=...; Path=/` to a cookie. A cookie that is being removed (empty value) is skipped.
pub fn parse_set_cookie(header: &str) -> Option<SetCookie> {
    let mut parts = header.split(';');
    let (name, value) = parts.next()?.split_once('=')?;
    let (name, value) = (name.trim(), value.trim());
    if name.is_empty() || value.is_empty() {
        return None;
    }
    let shared = parts.any(|attribute| {
        attribute
            .split_once('=')
            .is_some_and(|(k, v)| k.trim().eq_ignore_ascii_case("domain") && !v.trim().is_empty())
    });
    Some(SetCookie {
        name: name.to_owned(),
        value: value.to_owned(),
        shared,
    })
}

fn cookie_names(cookies: &[SetCookie]) -> String {
    if cookies.is_empty() {
        return "geen".into();
    }
    let mut names: Vec<&str> = cookies.iter().map(|c| c.name.as_str()).collect();
    names.sort_unstable();
    names.join(", ")
}

/// The tags of every `<input>` in a page, lower-cased apart from their values.
fn input_tags(html: &str) -> impl Iterator<Item = &str> {
    let lower = html.to_ascii_lowercase();
    let mut starts = Vec::new();
    let mut from = 0;
    while let Some(at) = lower[from..].find("<input") {
        starts.push(from + at);
        from += at + 6;
    }
    starts.into_iter().filter_map(move |start| {
        let end = html[start..].find('>')?;
        Some(&html[start..start + end])
    })
}

/// An attribute of a tag, quoted or bare: `name="x"`, `name='x'` or `name=x`.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    loop {
        let at = from + lower[from..].find(name)?;
        let before_ok = at == 0 || lower.as_bytes()[at - 1].is_ascii_whitespace();
        let rest = tag[at + name.len()..].trim_start();
        if before_ok && let Some(rest) = rest.strip_prefix('=') {
            let rest = rest.trim_start();
            let value = match rest.chars().next() {
                Some(quote @ ('"' | '\'')) => rest[1..].split(quote).next().unwrap_or_default(),
                _ => rest
                    .split(|c: char| c.is_ascii_whitespace() || c == '>')
                    .next()
                    .unwrap_or_default(),
            };
            return Some(value.to_owned());
        }
        from = at + name.len();
    }
}

/// The value of the `<input>` named `name`.
pub fn input_value(html: &str, name: &str) -> Option<String> {
    input_tags(html)
        .find(|tag| attribute(tag, "name").is_some_and(|n| n.eq_ignore_ascii_case(name)))
        .and_then(|tag| attribute(tag, "value"))
        .filter(|v| !v.is_empty())
}

/// The names of a page's inputs, for the diagnosis of a page EB MCP does not know.
fn input_names(html: &str) -> Vec<String> {
    let mut names: Vec<String> = input_tags(html)
        .filter_map(|tag| attribute(tag, "name"))
        .filter(|n| !n.is_empty())
        .collect();
    names.dedup();
    names.truncate(12);
    names
}

fn page_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title>")? + 7;
    let end = start + lower[start..].find("</title>")?;
    let title = html[start..end].trim();
    (!title.is_empty()).then(|| title.chars().take(80).collect())
}

/// What a page told the user in an `alert(...)`, apart from the form's own "moet ingevuld zijn" checks.
fn page_alert(html: &str) -> Option<String> {
    let mut from = 0;
    while let Some(at) = html[from..].find("alert(") {
        let rest = &html[from + at + 6..];
        from += at + 6;
        let Some(quote @ ('"' | '\'')) = rest.chars().next() else {
            continue;
        };
        let text = rest[1..].split(quote).next().unwrap_or_default().trim();
        if !text.is_empty() && !text.contains("moet ingevuld zijn") && !text.contains("doorsturen")
        {
            return Some(text.chars().take(200).collect());
        }
    }
    None
}

/// Reads the answer to the login form. Pure, so it can be tested without e-Boekhouden. The cookies are those
/// of the login page and of this answer together.
pub fn classic_outcome(status: u16, body: &str, cookies: &[SetCookie]) -> Login {
    if cookies.iter().any(|c| c.name == "auth-token") {
        // What a browser would send on to the API host: the token and the other cookies set for the whole domain.
        return Login::Session(
            cookies
                .iter()
                .filter(|c| c.shared || c.name == "auth-token")
                .map(|c| (c.name.clone(), c.value.clone()))
                .collect(),
        );
    }
    if status == 429 {
        return Login::Refused("e-Boekhouden vraagt om even te wachten met inloggen".into());
    }
    if status >= 500 {
        return Login::Error(format!(
            "e-Boekhouden gaf een fout bij het inloggen ({status})"
        ));
    }
    let lower = body.to_lowercase();
    let names = input_names(body);
    let asks_password = names
        .iter()
        .any(|n| n.eq_ignore_ascii_case("txtWachtwoord"));
    if !asks_password && TWO_STEP.iter().any(|w| lower.contains(w)) {
        return Login::NeedsCode;
    }
    if asks_password {
        return Login::Refused(page_alert(body).unwrap_or_else(|| {
            "e-Boekhouden weigerde de gebruikersnaam of het wachtwoord".into()
        }));
    }
    Login::Error(format!(
        "e-Boekhouden gaf na het inloggen geen token (status {status}{})",
        page_title(body)
            .map(|t| format!(", pagina: {t}"))
            .unwrap_or_default()
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

/// Why a call gave up waiting for its turn.
#[derive(Debug, PartialEq)]
enum Busy {
    /// The 60 a minute (or the 2 at a time) are taken.
    RateLimit,
    /// EB MCP pauses after errors from e-Boekhouden; this much of the pause is left.
    Paused(Duration),
}

impl Busy {
    fn message(&self) -> String {
        match self {
            Busy::RateLimit => "Te veel aanroepen tegelijk: EB MCP houdt e-Boekhouden rustig (60 per minuut). Probeer het over een minuut opnieuw.".into(),
            Busy::Paused(left) => format!(
                "e-Boekhouden gaf net fouten, dus EB MCP wacht nog {} met nieuwe aanroepen. Probeer het daarna opnieuw.",
                wait_text(*left)
            ),
        }
    }
}

/// "12 seconden", "1 minuut", "5 minuten": rounded up, so the user never retries too early.
fn wait_text(wait: Duration) -> String {
    let seconds = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
    match seconds {
        0 | 1 => "1 seconde".into(),
        2..=59 => format!("{seconds} seconden"),
        60 => "1 minuut".into(),
        _ => format!("{} minuten", seconds.div_ceil(60)),
    }
}

/// 60 starts per minute, 2 at a time, and a growing pause after errors.
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

    /// A place under the limit, or why none came free within `patience`.
    async fn acquire(&self, patience: Duration) -> Result<tokio::sync::SemaphorePermit<'_>, Busy> {
        if let Ok(permit) = tokio::time::timeout(patience, self.wait_for_turn()).await {
            return Ok(permit);
        }
        let state = self.state.lock().await;
        let now = TokioInstant::now();
        Err(match state.paused_until.filter(|t| *t > now) {
            Some(until) => Busy::Paused(until - now),
            None => Busy::RateLimit,
        })
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
            .route("/v1/api/verboden", get(|| async { axum::http::StatusCode::FORBIDDEN }))
            // The classic login page: a session cookie, the server's name and a hidden check value.
            .route(
                "/bh/inloggen.asp",
                get(|| async {
                    // Two Set-Cookie headers, as the real page sends them.
                    let mut headers = HeaderMap::new();
                    headers.append("set-cookie", "ASPSESSIONIDX=sess; path=/; HttpOnly; Secure".parse().unwrap());
                    headers.append("set-cookie", "SRVRID=9s13; path=/; Secure".parse().unwrap());
                    headers.insert("content-type", "text/html".parse().unwrap());
                    (headers, LOGIN_PAGE)
                })
                .post(|headers: HeaderMap, body: String| async move {
                    let cookie = headers.get("cookie").and_then(|v| v.to_str().ok()).unwrap_or_default().to_owned();
                    let agent = headers.get("user-agent").and_then(|v| v.to_str().ok()).unwrap_or_default().to_owned();
                    let field = |name: &str| {
                        url::form_urlencoded::parse(body.as_bytes())
                            .find(|(k, _)| k == name)
                            .map(|(_, v)| v.into_owned())
                            .unwrap_or_default()
                    };
                    // As the real page: the form only counts in the session it was made for.
                    let same_session = cookie.contains("ASPSESSIONIDX=sess")
                        && cookie.contains("SRVRID=9s13")
                        && field("txtFormCheck") == "643214248783498236735031453023"
                        && field("XSRVX") == "9s13"
                        && !field("FP").is_empty()
                        && agent.starts_with("Mozilla/");
                    let html = [("content-type", "text/html")];
                    if !same_session {
                        return (axum::http::StatusCode::OK, html, [("set-cookie", "X=; path=/")], "<title>Fout</title>".to_owned());
                    }
                    match (field("txtEmail").as_str(), field("txtWachtwoord").as_str()) {
                        ("jan@studionoord.example", "geheim") => (
                            axum::http::StatusCode::OK,
                            html,
                            [("set-cookie", "auth-token=jwt; domain=.e-boekhouden.nl; path=/; Secure")],
                            "<title>Inloggen | e-Boekhouden.nl</title><script>top.frames.sessionframe.location=\"updatesessie.asp\"</script>".to_owned(),
                        ),
                        ("tweestaps@studionoord.example", "geheim") => (
                            axum::http::StatusCode::OK,
                            html,
                            [("set-cookie", "X=; path=/")],
                            "<title>Tweestapsverificatie</title><form><input name=\"txtCode\"></form>".to_owned(),
                        ),
                        _ => (
                            axum::http::StatusCode::OK,
                            html,
                            [("set-cookie", "X=; path=/")],
                            format!("{LOGIN_PAGE}<script>alert(\"Gebruikersnaam en/of wachtwoord is onjuist.\")</script>"),
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
                    (axum::http::StatusCode::OK, json!({"valid": cookie.contains("auth-token=jwt")}).to_string())
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}/v1/api/")
    }

    /// The parts of e-Boekhouden's login page the sign-in reads (saved 2026-10-29, checked live 2026-11-11).
    const LOGIN_PAGE: &str = r#"<title>Inloggen | e-Boekhouden.nl</title>
        <form name="frm" id="frm" Action="inloggen.asp?LOGIN=1" method="post" OnSubmit='return ValidateForm()' novalidate>
        <INPUT class="INPUTBOX" type="text" id="txtEmail" name="txtEmail" value="" size="35" maxlength="50" tabindex=1>
        <INPUT class="INPUTBOX" type="password" id="txtWachtwoord" name="txtWachtwoord" value="" size="35" tabindex=2>
        <INPUT class="act-btn" type="submit"  value="Inloggen"  tabindex=3 id=submit1 name=submit1   >
        <input type="hidden" id="txtFormCheck" name="txtFormCheck" value="643214248783498236735031453023">
        <input type="hidden" id="FP" name="FP" value="">
        <input type="hidden" id="XSRVX" name="XSRVX" value=""></form>
        <script>if (document.frm.txtEmail.value==''){alert("Gebruikersnaam moet ingevuld zijn.");}</script>"#;

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
    async fn a_403_means_no_rights_not_signed_out() {
        let client = EbClient::new(&mock().await);
        let call = client.get(&cookies(), "verboden", &[]).await;
        assert_eq!(call.status, 403);
        let error = call.error.unwrap();
        assert!(
            error.starts_with("Geen toegang tot dit onderdeel"),
            "{error}"
        );
        assert!(!error.contains("ingelogd"), "{error}");
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
            assert!(limiter.acquire(QUEUE_WAIT).await.is_ok());
        }
        assert!(start.elapsed() < Duration::from_secs(1));
        // The next one over the limit waits at most 10 s for a place, then gives up instead of hanging for the rest of the minute.
        assert_eq!(
            limiter.acquire(QUEUE_WAIT).await.err(),
            Some(Busy::RateLimit)
        );
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
        assert!(limiter.acquire(QUEUE_WAIT).await.is_ok());
        assert!(start.elapsed() >= Duration::from_secs(2));
        limiter.failed(Some(Duration::from_secs(6))).await;
        let start = tokio::time::Instant::now();
        assert!(limiter.acquire(QUEUE_WAIT).await.is_ok());
        assert!(start.elapsed() >= Duration::from_secs(6));
        // A pause longer than the queue wait fails the call instead of holding it for minutes, and says so.
        limiter.failed(Some(Duration::from_secs(120))).await;
        let Err(Busy::Paused(left)) = limiter.acquire(QUEUE_WAIT).await else {
            panic!("the call should give up because of the pause")
        };
        assert!(
            left > Duration::from_secs(100) && left <= Duration::from_secs(110),
            "{left:?}"
        );
    }

    #[test]
    fn says_why_a_call_gave_up_waiting() {
        assert!(Busy::RateLimit.message().contains("60 per minuut"));
        let paused = Busy::Paused(Duration::from_millis(109_400)).message();
        assert!(paused.contains("e-Boekhouden gaf net fouten"), "{paused}");
        assert!(paused.contains("nog 2 minuten"), "{paused}");
        assert!(!paused.contains("60 per minuut"), "{paused}");
        assert_eq!(wait_text(Duration::from_millis(300)), "1 seconde");
        assert_eq!(wait_text(Duration::from_millis(11_200)), "12 seconden");
        assert_eq!(wait_text(Duration::from_secs(60)), "1 minuut");
        assert_eq!(wait_text(Duration::from_secs(61)), "2 minuten");
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
        let signed_in = client.auth(&[("auth-token".into(), "jwt".into())]).await;
        assert_eq!(signed_in.status, 200, "{:?}", signed_in.error);
        assert!(valid_session(&signed_in));
        let not_yet = client.auth(&[("SRVRID".into(), "1".into())]).await;
        assert_eq!(not_yet.status, 200);
        assert!(!valid_session(&not_yet));
        assert!(not_yet.error.is_some());
    }

    #[tokio::test]
    async fn signs_in_through_the_classic_form_like_a_browser() {
        let client = EbClient::new(&mock().await);
        let run = client
            .login("jan@studionoord.example", "geheim", 4075681428)
            .await;
        // The token and nothing that belongs to the classic host only (its ASP session, the server name).
        assert_eq!(
            run.outcome,
            Login::Session(vec![("auth-token".into(), "jwt".into())])
        );
        let paths: Vec<(&str, &str)> = run
            .calls
            .iter()
            .map(|c| (c.method, c.path.as_str()))
            .collect();
        assert_eq!(
            paths,
            [("GET", "bh/inloggen.asp"), ("POST", "bh/inloggen.asp")]
        );
        assert!(
            run.calls
                .iter()
                .all(|c| c.error.is_none() && c.status == 200)
        );
        // The diagnosis names cookies and fields, never a value.
        let notes = run.notes.join("\n");
        assert!(notes.contains("ASPSESSIONIDX, SRVRID"), "{notes}");
        assert!(notes.contains("auth-token"), "{notes}");
        assert!(
            !notes.contains("jwt") && !notes.contains("geheim") && !notes.contains("sess"),
            "{notes}"
        );
        assert!(!format!("{:?}", run.calls).contains("geheim"));
    }

    #[tokio::test]
    async fn tells_a_wrong_password_from_a_two_step_page() {
        let client = EbClient::new(&mock().await);
        let wrong = client.login("jan@studionoord.example", "fout", 1).await;
        assert_eq!(
            wrong.outcome,
            Login::Refused("Gebruikersnaam en/of wachtwoord is onjuist.".into())
        );
        assert!(wrong.calls[1].error.is_some());
        let code = client
            .login("tweestaps@studionoord.example", "geheim", 1)
            .await;
        assert_eq!(code.outcome, Login::NeedsCode);
        assert!(
            code.calls[1].error.is_none(),
            "asking for the code is no error"
        );
        assert!(code.notes.join(" ").contains("txtCode"), "{:?}", code.notes);
    }

    #[test]
    fn sends_back_the_cookies_a_server_set_along_the_way() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.append(
            reqwest::header::SET_COOKIE,
            "SRVRID=s7; path=/".parse().unwrap(),
        );
        headers.append(
            reqwest::header::SET_COOKIE,
            "EBSRV=b2; path=/; HttpOnly".parse().unwrap(),
        );
        let base = vec![
            ("auth-token".to_owned(), "jwt".to_owned()),
            ("SRVRID".to_owned(), "s1".to_owned()),
        ];
        assert_eq!(
            with_set_cookies(&base, &headers),
            vec![
                ("auth-token".to_owned(), "jwt".to_owned()),
                ("SRVRID".to_owned(), "s7".to_owned()),
                ("EBSRV".to_owned(), "b2".to_owned())
            ]
        );
        assert_eq!(
            with_set_cookies(&base, &reqwest::header::HeaderMap::new()),
            base
        );
    }

    #[test]
    fn keeps_the_editors_session_keys_to_itself() {
        let mut answer = json!({"sessionKey": "k", "taal": "NL", "sjabloon": {"sessionHash": "h", "elements": [{"id": 1, "sessionHash": "x"}]}});
        without_session_keys(&mut answer);
        assert_eq!(
            answer,
            json!({"taal": "NL", "sjabloon": {"elements": [{"id": 1}]}})
        );
    }

    #[test]
    fn reads_a_download_file_name_without_any_folder() {
        assert_eq!(
            file_name_of("attachment; filename=F00002.pdf").as_deref(),
            Some("F00002.pdf")
        );
        assert_eq!(
            file_name_of("attachment; filename=\"Factuur 12.pdf\"").as_deref(),
            Some("Factuur 12.pdf")
        );
        assert_eq!(
            file_name_of("attachment; filename=../../etc/x.pdf").as_deref(),
            Some("x.pdf")
        );
        assert_eq!(file_name_of("attachment; filename=..").as_deref(), None);
        assert_eq!(file_name_of("inline"), None);
    }

    #[tokio::test]
    async fn names_a_changed_login_page() {
        let app = Router::new().route(
            "/bh/inloggen.asp",
            get(|| async { "<title>Onderhoud</title>" }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let run = EbClient::new(&format!("http://{addr}/v1/api/"))
            .login("a", "b", 1)
            .await;
        let Login::Error(message) = run.outcome else {
            panic!("{:?}", run.outcome)
        };
        assert!(message.contains("txtFormCheck"), "{message}");
        assert_eq!(
            run.calls.len(),
            1,
            "nothing is posted to a page it does not know"
        );
    }

    #[test]
    fn reads_the_login_page_and_its_cookies() {
        assert_eq!(
            input_value(LOGIN_PAGE, "txtFormCheck").as_deref(),
            Some("643214248783498236735031453023")
        );
        assert_eq!(input_value(LOGIN_PAGE, "FP"), None);
        assert_eq!(
            input_names(LOGIN_PAGE),
            [
                "txtEmail",
                "txtWachtwoord",
                "submit1",
                "txtFormCheck",
                "FP",
                "XSRVX"
            ]
        );
        assert_eq!(
            page_title(LOGIN_PAGE).as_deref(),
            Some("Inloggen | e-Boekhouden.nl")
        );
        assert_eq!(
            page_alert(LOGIN_PAGE),
            None,
            "the form's own checks are no answer"
        );
        assert_eq!(
            parse_set_cookie("auth-token=abc; domain=.e-boekhouden.nl; path=/; Secure"),
            Some(SetCookie {
                name: "auth-token".into(),
                value: "abc".into(),
                shared: true
            })
        );
        assert_eq!(
            parse_set_cookie("SRVRID=9s13; path=/; Secure; SameSite=LAX"),
            Some(SetCookie {
                name: "SRVRID".into(),
                value: "9s13".into(),
                shared: false
            })
        );
        assert_eq!(
            parse_set_cookie("auth-token=; path=/"),
            None,
            "a removed cookie"
        );
    }

    #[test]
    fn reads_the_answer_to_the_login_form() {
        let cookie = |name: &str, shared: bool| SetCookie {
            name: name.into(),
            value: "v".into(),
            shared,
        };
        assert_eq!(
            classic_outcome(
                200,
                "",
                &[
                    cookie("ASPSESSIONID", false),
                    cookie("SessionId", true),
                    cookie("auth-token", false)
                ]
            ),
            Login::Session(vec![
                ("SessionId".into(), "v".into()),
                ("auth-token".into(), "v".into())
            ])
        );
        assert_eq!(
            classic_outcome(200, LOGIN_PAGE, &[]),
            Login::Refused("e-Boekhouden weigerde de gebruikersnaam of het wachtwoord".into())
        );
        assert_eq!(
            classic_outcome(
                200,
                "<title>Extra controle</title>Vul de verificatiecode in <input name=\"code\">",
                &[]
            ),
            Login::NeedsCode
        );
        assert!(matches!(classic_outcome(429, "", &[]), Login::Refused(_)));
        assert_eq!(
            classic_outcome(500, "<html>stack</html>", &[]),
            Login::Error("e-Boekhouden gaf een fout bij het inloggen (500)".into())
        );
        assert_eq!(
            classic_outcome(302, "<title>Object moved</title>", &[]),
            Login::Error(
                "e-Boekhouden gaf na het inloggen geen token (status 302, pagina: Object moved)"
                    .into()
            )
        );
    }
}
