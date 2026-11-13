//! What the app knows while it runs. Sessions live only here, in memory: per
//! signed-in account the cookies e-Boekhouden's `auth/login` gave, the
//! `auth-token` among them (signin.rs, SessionJar).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::accounts::AccountStore;
use crate::ebapi::{ApiCall, EbClient};
use crate::reqlog::RequestLog;

/// Holds the cookies of each session, by key, and forgets them at sign-out. The app keeps them in memory
/// (signin.rs, SessionJar); tests use a fake. Signing out only forgets: e-Boekhouden's API has no sign-out.
/// What a keep-alive check found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeepAlive {
    /// e-Boekhouden still knows the session.
    Alive,
    /// e-Boekhouden no longer knows it: the session is marked expired.
    Expired,
    /// No answer or a server error: nothing learned, nothing changed.
    Unknown,
    /// There was no session to check.
    NoSession,
}

pub trait CookieJar: Send + Sync {
    fn cookies(&self, key: &str) -> Result<Vec<(String, String)>, String>;
    fn put(&self, key: &str, cookies: Vec<(String, String)>);
    fn sign_out(&self, key: &str);
}

#[derive(Clone, Debug)]
pub struct Session {
    pub account_id: Uuid,
    pub label: String,
    pub administration: Option<String>,
    /// Where the session's cookies are in the CookieJar.
    pub key: String,
    pub started_at: u64,
    pub last_used: Instant,
    pub expired: bool,
}

/// A session as the UI and the `status` tool see it.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    pub account_id: Uuid,
    pub label: String,
    pub administration: Option<String>,
    pub started_at: u64,
    pub expired: bool,
}

/// Things the UI should hear about. lib.rs forwards them as Tauri events.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AppEvent {
    SessionsChanged,
    LogChanged,
    SessionExpired {
        account_id: Uuid,
        label: String,
    },
    /// A change waits for the user's approval, or one was answered or withdrawn.
    ApprovalsChanged,
}

/// The cookies of a live session and the account's label.
type SessionCookies = (Vec<(String, String)>, String);

pub struct AppState {
    pub accounts: AccountStore,
    pub client: EbClient,
    pub log: RequestLog,
    /// Changes AI clients asked for that wait for the user's word (approvals.rs).
    pub approvals: crate::approvals::Approvals,
    jar: Arc<dyn CookieJar>,
    sessions: RwLock<HashMap<Uuid, Session>>,
    pub events: broadcast::Sender<AppEvent>,
    /// Set while EB MCP gets ready to restart into an update: new tool calls are refused.
    updating: AtomicBool,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl AppState {
    pub fn new(accounts: AccountStore, client: EbClient, jar: Arc<dyn CookieJar>) -> Self {
        let (events, _) = broadcast::channel(64);
        Self {
            accounts,
            client,
            log: RequestLog::default(),
            approvals: crate::approvals::Approvals::default(),
            jar,
            sessions: RwLock::new(HashMap::new()),
            events,
            updating: AtomicBool::new(false),
        }
    }

    pub fn sessions(&self) -> Vec<SessionView> {
        let mut list: Vec<SessionView> = self
            .sessions
            .read()
            .unwrap()
            .values()
            .map(|s| SessionView {
                account_id: s.account_id,
                label: s.label.clone(),
                administration: s.administration.clone(),
                started_at: s.started_at,
                expired: s.expired,
            })
            .collect();
        list.sort_by_key(|s| s.started_at);
        list
    }

    pub fn set_updating(&self, updating: bool) {
        self.updating.store(updating, Ordering::SeqCst);
    }

    pub fn is_updating(&self) -> bool {
        self.updating.load(Ordering::SeqCst)
    }

    pub fn session_count(&self) -> usize {
        self.sessions
            .read()
            .unwrap()
            .values()
            .filter(|s| !s.expired)
            .count()
    }

    /// Keeps a new session's cookies under `key`, before `add_session` names it.
    pub fn jar_put(&self, key: &str, cookies: Vec<(String, String)>) {
        self.jar.put(key, cookies);
    }

    /// Called by the sign-in once e-Boekhouden accepted the account; its cookies are in the jar under `key`.
    pub fn add_session(&self, account_id: Uuid, label: &str, key: &str) {
        let session = Session {
            account_id,
            label: label.to_owned(),
            administration: None,
            key: key.to_owned(),
            started_at: now_ms(),
            last_used: Instant::now(),
            expired: false,
        };
        if let Some(old) = self.sessions.write().unwrap().insert(account_id, session)
            && old.key != key
        {
            self.jar.sign_out(&old.key);
        }
        let _ = self.events.send(AppEvent::SessionsChanged);
    }

    pub fn set_administration(&self, account_id: Uuid, administration: &str) {
        if let Some(s) = self.sessions.write().unwrap().get_mut(&account_id) {
            s.administration = Some(administration.to_owned());
        }
        let _ = self.accounts.set_administration(account_id, administration);
        let _ = self.events.send(AppEvent::SessionsChanged);
    }

    /// Forgets the session and its token. e-Boekhouden's API has no sign-out; the token ends there when it expires.
    /// `why` goes to the log, so a session that is gone can always be traced to what ended it.
    pub fn sign_out(&self, account_id: Uuid, why: &str) -> bool {
        let removed = self.sessions.write().unwrap().remove(&account_id);
        if let Some(session) = &removed {
            log::info!("uitgelogd {}: {why}", session.label);
            self.jar.sign_out(&session.key);
            let _ = self.events.send(AppEvent::SessionsChanged);
        }
        removed.is_some()
    }

    pub fn sign_out_all(&self, why: &str) {
        let ids: Vec<Uuid> = self.sessions.read().unwrap().keys().copied().collect();
        for id in ids {
            self.sign_out(id, why);
        }
    }

    /// Forgets sessions no AI client used for `idle`.
    pub fn sign_out_idle(&self, idle: Duration) -> Vec<Uuid> {
        let stale: Vec<Uuid> = self
            .sessions
            .read()
            .unwrap()
            .values()
            .filter(|s| s.last_used.elapsed() >= idle)
            .map(|s| s.account_id)
            .collect();
        for id in &stale {
            self.sign_out(
                *id,
                &format!("{} minuten niet gebruikt", idle.as_secs() / 60),
            );
        }
        stale
    }

    /// A GET on e-Boekhouden's API in the session of `account_id`, logged
    /// under the MCP call `parent` (if any) for `client`.
    pub async fn api_get(
        &self,
        account_id: Uuid,
        path: &str,
        query: &[(String, String)],
        parent: Option<u64>,
        client: &str,
    ) -> ApiCall {
        self.api_send(account_id, "GET", path, query, None, parent, client)
            .await
    }

    /// The cookies of an account's live session and its label, or the call that says it is not signed in.
    fn session_cookies(
        &self,
        account_id: Uuid,
        path: &str,
    ) -> Result<SessionCookies, Box<(ApiCall, String)>> {
        let session = self.sessions.read().unwrap().get(&account_id).cloned();
        let label = session
            .as_ref()
            .map(|s| s.label.clone())
            .unwrap_or_else(|| self.label_of(account_id));
        match session
            .filter(|s| !s.expired)
            .and_then(|s| self.jar.cookies(&s.key).ok())
        {
            Some(cookies) => {
                self.note_use(account_id);
                Ok((cookies, label))
            }
            None => Err(Box::new((not_signed_in(path, &[], &label), label))),
        }
    }

    fn log_call(
        &self,
        call: &ApiCall,
        account_id: Uuid,
        label: String,
        parent: Option<u64>,
        client: &str,
        had_session: bool,
    ) {
        self.log
            .add_eb(call, parent, client, Some((account_id, label)), had_session);
        let _ = self.events.send(AppEvent::LogChanged);
    }

    /// Opens a template editor session in the account's session (ebapi.rs, `editor_open`): the call and the session hash.
    pub async fn api_editor_open(
        &self,
        account_id: Uuid,
        path: &str,
        parent: Option<u64>,
        client: &str,
    ) -> (ApiCall, Option<String>) {
        match self.session_cookies(account_id, path) {
            Err(refused) => {
                let (call, label) = *refused;
                self.log_call(&call, account_id, label, parent, client, false);
                (call, None)
            }
            Ok((cookies, label)) => {
                let (call, hash) = self.client.editor_open(&cookies, path).await;
                if call.error.is_some() && matches!(call.status, 300..=399 | 401) {
                    self.mark_expired(account_id);
                }
                self.log_call(&call, account_id, label, parent, client, true);
                (call, hash)
            }
        }
    }

    /// One call on the template editor's API in the account's session, logged like any other call.
    pub async fn api_editor(
        &self,
        account_id: Uuid,
        method: &'static str,
        path: &str,
        body: Option<&serde_json::Value>,
        parent: Option<u64>,
        client: &str,
    ) -> ApiCall {
        // The last part of an editor path is the session's hash: it never goes into the log.
        let shown = path.rsplit_once('/').map_or(path, |(head, _)| head);
        match self.session_cookies(account_id, shown) {
            Err(refused) => {
                let (call, label) = *refused;
                self.log_call(&call, account_id, label, parent, client, false);
                call
            }
            Ok((cookies, label)) => {
                let call = self.client.editor(&cookies, method, path, body).await;
                self.log_call(&call, account_id, label, parent, client, true);
                call
            }
        }
    }

    /// A file from e-Boekhouden in the session of `account_id`, logged like any other call.
    pub async fn api_download(
        &self,
        account_id: Uuid,
        path: &str,
        parent: Option<u64>,
        client: &str,
    ) -> (ApiCall, Option<(Vec<u8>, Option<String>)>) {
        match self.session_cookies(account_id, path) {
            Err(refused) => {
                let (call, label) = *refused;
                self.log_call(&call, account_id, label, parent, client, false);
                (call, None)
            }
            Ok((cookies, label)) => {
                let (call, file) = self.client.download(&cookies, path).await;
                if matches!(call.status, 300..=399 | 401) {
                    self.mark_expired(account_id);
                }
                self.log_call(&call, account_id, label, parent, client, true);
                (call, file)
            }
        }
    }

    /// A job on e-Boekhouden's hub in the session of `account_id`, logged like any other call.
    pub async fn api_hub(
        &self,
        account_id: Uuid,
        hub: &str,
        target: &str,
        argument: &serde_json::Value,
        parent: Option<u64>,
        client: &str,
    ) -> ApiCall {
        let session = self.sessions.read().unwrap().get(&account_id).cloned();
        let label = session
            .as_ref()
            .map(|s| s.label.clone())
            .unwrap_or_else(|| self.label_of(account_id));
        let path = format!("hub/{hub}");
        let cookies = session
            .filter(|s| !s.expired)
            .and_then(|s| self.jar.cookies(&s.key).ok());
        let (call, had_session) = match cookies {
            None => (not_signed_in(&path, &[], &label), false),
            Some(cookies) => {
                self.note_use(account_id);
                let call = self.client.hub(&cookies, hub, target, argument).await;
                if call.status == 401 {
                    self.mark_expired(account_id);
                }
                (call, true)
            }
        };
        self.log.add_eb(
            &call,
            parent,
            client,
            Some((account_id, label)),
            had_session,
        );
        let _ = self.events.send(AppEvent::LogChanged);
        call
    }

    /// One call on e-Boekhouden's API in the session of `account_id`, logged: a GET, or a change (POST, PUT,
    /// DELETE). The caller has already decided that this account may make the change.
    #[allow(clippy::too_many_arguments)]
    pub async fn api_send(
        &self,
        account_id: Uuid,
        method: &'static str,
        path: &str,
        query: &[(String, String)],
        body: Option<&serde_json::Value>,
        parent: Option<u64>,
        client: &str,
    ) -> ApiCall {
        let session = self.sessions.read().unwrap().get(&account_id).cloned();
        let label = session
            .as_ref()
            .map(|s| s.label.clone())
            .unwrap_or_else(|| self.label_of(account_id));
        let (call, had_session) = match session.filter(|s| !s.expired) {
            None => (not_signed_in(path, query, &label), false),
            Some(session) => match self.jar.cookies(&session.key) {
                Err(e) => {
                    let mut call = not_signed_in(path, query, &session.label);
                    call.error = Some(format!(
                        "De sessie van {} is niet te lezen: {e}",
                        session.label
                    ));
                    (call, false)
                }
                Ok(cookies) => {
                    self.note_use(account_id);
                    let call = self.client.send(&cookies, method, path, query, body).await;
                    // Only these mean "not signed in". A 403 is a part this account may not see.
                    if matches!(call.status, 300..=399 | 401) {
                        self.mark_expired(account_id);
                    }
                    (call, !cookies.is_empty())
                }
            },
        };
        self.log.add_eb(
            &call,
            parent,
            client,
            Some((account_id, label)),
            had_session,
        );
        let _ = self.events.send(AppEvent::LogChanged);
        call
    }

    /// Whether the user signed in on this account is an accountant: `isAccountant` in the answer to
    /// `POST auth`, the check e-Boekhouden's own app makes at startup. It changes nothing. Only that one
    /// field is kept and returned (`{"isAccountant": …}`): the rest of the answer describes the user and
    /// the session and has no business in the request log or a tool's answer.
    pub async fn api_accountant_flag(
        &self,
        account_id: Uuid,
        parent: Option<u64>,
        client: &str,
    ) -> Result<serde_json::Value, String> {
        let session = self.sessions.read().unwrap().get(&account_id).cloned();
        let label = session
            .as_ref()
            .map(|s| s.label.clone())
            .unwrap_or_else(|| self.label_of(account_id));
        let (mut call, had_session) = match session.filter(|s| !s.expired) {
            None => (not_signed_in("auth", &[], &label), false),
            Some(session) => match self.jar.cookies(&session.key) {
                Err(e) => {
                    let mut call = not_signed_in("auth", &[], &session.label);
                    call.error = Some(format!(
                        "De sessie van {} is niet te lezen: {e}",
                        session.label
                    ));
                    (call, false)
                }
                Ok(cookies) => {
                    self.note_use(account_id);
                    let call = self.client.auth(&cookies).await;
                    if matches!(call.status, 300..=399 | 401) {
                        self.mark_expired(account_id);
                    }
                    (call, !cookies.is_empty())
                }
            },
        };
        let kept = call.json.as_ref().map(|auth| {
            let flag = auth
                .get("isAccountant")
                .or_else(|| auth.get("IsAccountant"))
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            serde_json::json!({ "isAccountant": flag })
        });
        call.json = kept.clone();
        self.log.add_eb(
            &call,
            parent,
            client,
            Some((account_id, label)),
            had_session,
        );
        let _ = self.events.send(AppEvent::LogChanged);
        match (call.ok(), kept) {
            (true, Some(kept)) => Ok(kept),
            _ => Err(call
                .error
                .unwrap_or_else(|| format!("e-Boekhouden gaf {} op auth", call.status))),
        }
    }

    /// Asks e-Boekhouden whether the session of `account_id` still stands (`POST auth`, the check its own app
    /// makes), for Blijf ingelogd: a session that is asked after does not lapse there for lack of use, and
    /// cookies the answer renews are kept. It does not count as use by an AI client. A session e-Boekhouden
    /// no longer knows is marked expired, and only that shows in Verzoeken: a line for every quiet check
    /// would push the AI calls out of the log.
    pub async fn keep_alive(&self, account_id: Uuid) -> KeepAlive {
        let session = self.sessions.read().unwrap().get(&account_id).cloned();
        let Some(session) = session.filter(|s| !s.expired) else {
            return KeepAlive::NoSession;
        };
        let Ok(cookies) = self.jar.cookies(&session.key) else {
            return KeepAlive::NoSession;
        };
        let (call, renewed) = self.client.auth_keeping(&cookies).await;
        if crate::ebapi::valid_session(&call) {
            // Signed out or signed in again while the check was under way: those cookies are not this
            // session's any more.
            let same = self
                .sessions
                .read()
                .unwrap()
                .get(&account_id)
                .is_some_and(|s| s.key == session.key && !s.expired);
            if same && renewed != cookies {
                self.jar.put(&session.key, renewed);
            }
            return KeepAlive::Alive;
        }
        // No answer, or a server error: nothing is known about the session, so it stays as it is.
        if call.status == 0 || call.status >= 500 {
            log::warn!(
                "ingelogd blijven {}: e-Boekhouden gaf geen bruikbaar antwoord (status {})",
                session.label,
                call.status
            );
            return KeepAlive::Unknown;
        }
        self.log.add_signin(
            &call,
            "EB MCP (ingelogd blijven)",
            (account_id, session.label.clone()),
        );
        let _ = self.events.send(AppEvent::LogChanged);
        self.mark_expired(account_id);
        KeepAlive::Expired
    }

    /// `keep_alive` for every session that stands, one after the other; how many are still alive.
    pub async fn keep_alive_all(&self) -> usize {
        let ids: Vec<Uuid> = self
            .sessions
            .read()
            .unwrap()
            .values()
            .filter(|s| !s.expired)
            .map(|s| s.account_id)
            .collect();
        let mut alive = 0;
        for id in ids {
            if self.keep_alive(id).await == KeepAlive::Alive {
                alive += 1;
            }
        }
        alive
    }

    /// The accounts without a session that stands: never signed in, signed out, or expired.
    pub fn signed_out_accounts(&self) -> Vec<Uuid> {
        let sessions = self.sessions.read().unwrap();
        self.accounts
            .list()
            .into_iter()
            .map(|a| a.id)
            .filter(|id| sessions.get(id).is_none_or(|s| s.expired))
            .collect()
    }

    /// A sign-in attempt or one of its checks in Verzoeken, so they show next to the AI calls. They stay out
    /// of the Dashboard's statistics (reqlog.rs, `add_signin`).
    pub fn log_signin_call(&self, call: &ApiCall, account_id: Uuid, label: &str) {
        self.log
            .add_signin(call, "EB MCP (inloggen)", (account_id, label.to_owned()));
        let _ = self.events.send(AppEvent::LogChanged);
    }

    fn note_use(&self, account_id: Uuid) {
        if let Some(s) = self.sessions.write().unwrap().get_mut(&account_id) {
            s.last_used = Instant::now();
        }
    }

    fn mark_expired(&self, account_id: Uuid) {
        let label = {
            let mut sessions = self.sessions.write().unwrap();
            let Some(s) = sessions.get_mut(&account_id) else {
                return;
            };
            if s.expired {
                return;
            }
            s.expired = true;
            log::info!(
                "sessie verlopen {}: e-Boekhouden kent hem niet meer",
                s.label
            );
            s.label.clone()
        };
        let _ = self
            .events
            .send(AppEvent::SessionExpired { account_id, label });
        let _ = self.events.send(AppEvent::SessionsChanged);
    }

    fn label_of(&self, account_id: Uuid) -> String {
        self.accounts
            .get(account_id)
            .map(|a| a.label)
            .unwrap_or_else(|| "dit account".into())
    }
}

fn not_signed_in(path: &str, query: &[(String, String)], label: &str) -> ApiCall {
    ApiCall {
        method: "GET",
        path: path.to_owned(),
        query: query.to_vec(),
        status: 0,
        duration_ms: 0,
        bytes: 0,
        rows: None,
        json: None,
        error: Some(format!(
            "Niet ingelogd bij {label}. Log in via EB MCP in de menubalk."
        )),
    }
}

#[cfg(test)]
pub mod testing {
    use super::*;
    use std::sync::Mutex;

    /// Cookies per session key, and the sign-outs it was asked for.
    #[derive(Default)]
    pub struct FakeJar {
        pub cookies: Mutex<HashMap<String, Vec<(String, String)>>>,
        pub signed_out: Mutex<Vec<String>>,
    }

    impl CookieJar for FakeJar {
        fn cookies(&self, key: &str) -> Result<Vec<(String, String)>, String> {
            Ok(self
                .cookies
                .lock()
                .unwrap()
                .get(key)
                .cloned()
                .unwrap_or_default())
        }

        fn put(&self, key: &str, cookies: Vec<(String, String)>) {
            self.cookies.lock().unwrap().insert(key.to_owned(), cookies);
        }

        fn sign_out(&self, key: &str) {
            self.signed_out.lock().unwrap().push(key.to_owned());
        }
    }

    pub fn state_with(jar: Arc<FakeJar>, base: &str) -> AppState {
        let dir = std::env::temp_dir().join(format!("ebmcp-state-{}", Uuid::new_v4()));
        AppState::new(AccountStore::open(&dir).unwrap(), EbClient::new(base), jar)
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    #[tokio::test]
    async fn calls_without_a_session_explain_how_to_sign_in() {
        let state = state_with(Arc::new(FakeJar::default()), "http://127.0.0.1:9/v1/api/");
        let call = state
            .api_get(Uuid::new_v4(), "relatie/gridtable", &[], None, "test")
            .await;
        assert!(call.error.unwrap().starts_with("Niet ingelogd bij"));
        assert_eq!(call.status, 0);
    }

    #[test]
    fn signing_out_forgets_the_session_and_its_cookies() {
        let jar = Arc::new(FakeJar::default());
        let state = state_with(jar.clone(), "http://127.0.0.1:9/v1/api/");
        let id = Uuid::new_v4();
        state.add_session(id, "Studio Noord", "eb-1");
        assert_eq!(state.session_count(), 1);
        assert!(state.sign_out(id, "test"));
        assert_eq!(state.session_count(), 0);
        assert_eq!(*jar.signed_out.lock().unwrap(), vec!["eb-1".to_owned()]);
    }

    #[test]
    fn signs_out_idle_sessions_only() {
        let jar = Arc::new(FakeJar::default());
        let state = state_with(jar.clone(), "http://127.0.0.1:9/v1/api/");
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        state.add_session(a, "A", "eb-a");
        state.add_session(b, "B", "eb-b");
        state
            .sessions
            .write()
            .unwrap()
            .get_mut(&a)
            .unwrap()
            .last_used = Instant::now() - Duration::from_secs(31 * 60);
        assert_eq!(state.sign_out_idle(Duration::from_secs(30 * 60)), vec![a]);
        assert_eq!(
            state
                .sessions()
                .iter()
                .map(|s| s.account_id)
                .collect::<Vec<_>>(),
            vec![b]
        );
    }

    #[tokio::test]
    async fn a_403_leaves_the_session_alone_and_a_401_expires_it() {
        let app = axum::Router::new()
            .route(
                "/v1/api/geen-toegang",
                axum::routing::get(|| async { axum::http::StatusCode::FORBIDDEN }),
            )
            .route(
                "/v1/api/verlopen",
                axum::routing::get(|| async { axum::http::StatusCode::UNAUTHORIZED }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let jar = Arc::new(FakeJar::default());
        jar.put("eb-1", vec![("auth-token".into(), "jwt".into())]);
        let state = state_with(jar, &format!("http://{addr}/v1/api/"));
        let id = Uuid::new_v4();
        state.add_session(id, "A", "eb-1");
        let mut events = state.events.subscribe();

        let call = state.api_get(id, "geen-toegang", &[], None, "test").await;
        assert_eq!(call.status, 403);
        assert!(call.error.unwrap().starts_with("Geen toegang"));
        assert!(!state.sessions()[0].expired);
        assert_eq!(state.session_count(), 1);
        while let Ok(event) = events.try_recv() {
            assert!(
                !matches!(event, AppEvent::SessionExpired { .. }),
                "{event:?}"
            );
        }

        let call = state.api_get(id, "verlopen", &[], None, "test").await;
        assert_eq!(call.status, 401);
        assert!(state.sessions()[0].expired);
        assert_eq!(state.session_count(), 0);
    }

    /// e-Boekhouden's `POST auth` for three sessions, told apart by their token: one that stands and gets a
    /// renewed token, one it no longer knows, and one it answers with a server error.
    async fn auth_server() -> String {
        use axum::http::{HeaderMap, StatusCode, header};
        use axum::response::IntoResponse;
        let app = axum::Router::new().route(
            "/v1/api/auth",
            axum::routing::post(|headers: HeaderMap| async move {
                let cookie = headers
                    .get(header::COOKIE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default()
                    .to_owned();
                if cookie.contains("auth-token=goed") || cookie.contains("auth-token=vernieuwd") {
                    (
                        [(header::SET_COOKIE, "auth-token=vernieuwd; path=/")],
                        axum::Json(serde_json::json!({"valid": true})),
                    )
                        .into_response()
                } else if cookie.contains("auth-token=kapot") {
                    StatusCode::INTERNAL_SERVER_ERROR.into_response()
                } else {
                    axum::Json(serde_json::json!({"valid": false})).into_response()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}/v1/api/")
    }

    #[tokio::test]
    async fn keeping_alive_renews_what_stands_and_expires_only_what_e_boekhouden_dropped() {
        let jar = Arc::new(FakeJar::default());
        jar.put(
            "eb-goed",
            vec![
                ("auth-token".into(), "goed".into()),
                ("server".into(), "a".into()),
            ],
        );
        jar.put("eb-oud", vec![("auth-token".into(), "oud".into())]);
        jar.put("eb-kapot", vec![("auth-token".into(), "kapot".into())]);
        let state = state_with(jar.clone(), &auth_server().await);
        let (goed, oud, kapot) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        state.add_session(goed, "Goed", "eb-goed");
        state.add_session(oud, "Oud", "eb-oud");
        state.add_session(kapot, "Kapot", "eb-kapot");
        // Not used for longer than the idle time: the check must not count as use.
        let long_ago = Instant::now() - Duration::from_secs(31 * 60);
        state
            .sessions
            .write()
            .unwrap()
            .get_mut(&goed)
            .unwrap()
            .last_used = long_ago;
        let logged = state.log.len();

        // The session that stands: alive, and the token the answer set replaces the old one, the rest stays.
        assert_eq!(state.keep_alive(goed).await, KeepAlive::Alive);
        assert_eq!(
            jar.cookies("eb-goed").unwrap(),
            vec![
                ("server".to_owned(), "a".to_owned()),
                ("auth-token".to_owned(), "vernieuwd".to_owned())
            ]
        );
        assert_eq!(state.sessions.read().unwrap()[&goed].last_used, long_ago);
        assert_eq!(
            state.log.len(),
            logged,
            "a quiet check leaves no line in Verzoeken"
        );

        // A server error says nothing about the session: it stays, and nothing is logged in Verzoeken.
        assert_eq!(state.keep_alive(kapot).await, KeepAlive::Unknown);
        assert!(!state.sessions.read().unwrap()[&kapot].expired);
        assert_eq!(state.log.len(), logged);

        // The session e-Boekhouden dropped: expired, told to the windows, and shown in Verzoeken once.
        let mut events = state.events.subscribe();
        assert_eq!(state.keep_alive(oud).await, KeepAlive::Expired);
        assert!(state.sessions.read().unwrap()[&oud].expired);
        assert_eq!(
            jar.cookies("eb-oud").unwrap(),
            vec![("auth-token".to_owned(), "oud".to_owned())]
        );
        assert_eq!(state.log.len(), logged + 1);
        let mut told = false;
        while let Ok(event) = events.try_recv() {
            told |=
                matches!(event, AppEvent::SessionExpired { account_id, .. } if account_id == oud);
        }
        assert!(told);
        // An expired session, or an account without one, is not asked after again.
        assert_eq!(state.keep_alive(oud).await, KeepAlive::NoSession);
        assert_eq!(state.keep_alive(Uuid::new_v4()).await, KeepAlive::NoSession);
        assert_eq!(state.log.len(), logged + 1);

        // All at once: only the one that stands counts, and it is asked after with its renewed token.
        assert_eq!(state.keep_alive_all().await, 1);
    }

    #[test]
    fn alles_inloggen_takes_the_accounts_without_a_session_that_stands() {
        use crate::accounts::{AccountInput, Rights, Store};
        let state = state_with(Arc::new(FakeJar::default()), "http://127.0.0.1:9/v1/api/");
        let add = |label: &str| {
            state
                .accounts
                .add(AccountInput {
                    label: label.into(),
                    username: format!("{label}@example.test"),
                    store: Store::System,
                    op_ref: None,
                    op_title: None,
                    op_vault: None,
                    rights: Rights::OFF,
                })
                .unwrap()
                .id
        };
        let (signed_in, expired, never) = (add("In"), add("Verlopen"), add("Nooit"));
        state.add_session(signed_in, "In", "eb-1");
        state.add_session(expired, "Verlopen", "eb-2");
        state.mark_expired(expired);
        let mut todo = state.signed_out_accounts();
        todo.sort();
        let mut expected = vec![expired, never];
        expected.sort();
        assert_eq!(todo, expected);
    }

    #[tokio::test]
    async fn no_connection_leaves_a_session_as_it_is() {
        let jar = Arc::new(FakeJar::default());
        jar.put("eb-1", vec![("auth-token".into(), "goed".into())]);
        // Nothing listens there.
        let state = state_with(jar.clone(), "http://127.0.0.1:9/v1/api/");
        let id = Uuid::new_v4();
        state.add_session(id, "A", "eb-1");
        assert_eq!(state.keep_alive(id).await, KeepAlive::Unknown);
        assert!(!state.sessions()[0].expired);
        assert_eq!(
            jar.cookies("eb-1").unwrap(),
            vec![("auth-token".to_owned(), "goed".to_owned())]
        );
    }

    #[test]
    fn a_new_sign_in_replaces_the_old_session() {
        let jar = Arc::new(FakeJar::default());
        let state = state_with(jar.clone(), "http://127.0.0.1:9/v1/api/");
        let id = Uuid::new_v4();
        state.add_session(id, "A", "eb-1");
        state.add_session(id, "A", "eb-2");
        assert_eq!(*jar.signed_out.lock().unwrap(), vec!["eb-1".to_owned()]);
        assert_eq!(state.session_count(), 1);
    }
}
