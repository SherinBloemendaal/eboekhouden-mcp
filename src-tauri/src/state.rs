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
    SessionExpired { account_id: Uuid, label: String },
}

pub struct AppState {
    pub accounts: AccountStore,
    pub client: EbClient,
    pub log: RequestLog,
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
    pub fn sign_out(&self, account_id: Uuid) -> bool {
        let removed = self.sessions.write().unwrap().remove(&account_id);
        if let Some(session) = &removed {
            self.jar.sign_out(&session.key);
            let _ = self.events.send(AppEvent::SessionsChanged);
        }
        removed.is_some()
    }

    pub fn sign_out_all(&self) {
        let ids: Vec<Uuid> = self.sessions.read().unwrap().keys().copied().collect();
        for id in ids {
            self.sign_out(id);
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
            self.sign_out(*id);
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
                    let call = self.client.get(&cookies, path, query).await;
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
        assert!(state.sign_out(id));
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
