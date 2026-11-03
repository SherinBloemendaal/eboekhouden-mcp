//! What the app knows while it runs. Sessions live only here, in memory: one
//! hidden webview per signed-in account holds e-Boekhouden's own session and
//! keeps renewing it the way their app does.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::accounts::AccountStore;
use crate::ebapi::{ApiCall, EbClient, auth_token};
use crate::reqlog::RequestLog;

/// Reads the cookies of a session's webview and signs it out. The app uses the
/// webviews; tests use a fake.
pub trait CookieJar: Send + Sync {
    fn cookies(&self, webview: &str) -> Result<Vec<(String, String)>, String>;
    fn sign_out(&self, webview: &str);
}

#[derive(Clone, Debug)]
pub struct Session {
    pub account_id: Uuid,
    pub label: String,
    pub administration: Option<String>,
    pub webview: String,
    pub started_at: u64,
    pub renewed_at: Option<u64>,
    pub last_used: Instant,
    pub expired: bool,
    token_hash: Option<u64>,
}

/// A session as the UI and the `status` tool see it.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    pub account_id: Uuid,
    pub label: String,
    pub administration: Option<String>,
    pub started_at: u64,
    pub renewed_at: Option<u64>,
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

fn hash(value: &str) -> u64 {
    let mut h = DefaultHasher::new();
    value.hash(&mut h);
    h.finish()
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
                renewed_at: s.renewed_at,
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

    /// Called by the sign-in once e-Boekhouden accepted the account.
    pub fn add_session(&self, account_id: Uuid, label: &str, webview: &str) {
        let token_hash = self
            .jar
            .cookies(webview)
            .ok()
            .and_then(|c| auth_token(&c).map(hash));
        let session = Session {
            account_id,
            label: label.to_owned(),
            administration: None,
            webview: webview.to_owned(),
            started_at: now_ms(),
            renewed_at: None,
            last_used: Instant::now(),
            expired: false,
            token_hash,
        };
        if let Some(old) = self.sessions.write().unwrap().insert(account_id, session)
            && old.webview != webview
        {
            self.jar.sign_out(&old.webview);
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

    /// Signs out at e-Boekhouden and forgets the session.
    pub fn sign_out(&self, account_id: Uuid) -> bool {
        let removed = self.sessions.write().unwrap().remove(&account_id);
        if let Some(session) = &removed {
            self.jar.sign_out(&session.webview);
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

    /// Signs out sessions no AI client used for `idle`.
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
            Some(session) => match self.jar.cookies(&session.webview) {
                Err(e) => {
                    let mut call = not_signed_in(path, query, &session.label);
                    call.error = Some(format!(
                        "De sessie van {} is niet te lezen: {e}",
                        session.label
                    ));
                    (call, false)
                }
                Ok(cookies) => {
                    self.note_use(account_id, auth_token(&cookies).map(hash));
                    let call = self.client.get(&cookies, path, query).await;
                    if matches!(call.status, 300..=399 | 401 | 403) {
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

    fn note_use(&self, account_id: Uuid, token_hash: Option<u64>) {
        if let Some(s) = self.sessions.write().unwrap().get_mut(&account_id) {
            s.last_used = Instant::now();
            if token_hash.is_some() && s.token_hash.is_some() && token_hash != s.token_hash {
                s.renewed_at = Some(now_ms());
            }
            if token_hash.is_some() {
                s.token_hash = token_hash;
            }
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

    /// Cookies per webview, and the sign-outs it was asked for.
    #[derive(Default)]
    pub struct FakeJar {
        pub cookies: Mutex<HashMap<String, Vec<(String, String)>>>,
        pub signed_out: Mutex<Vec<String>>,
    }

    impl CookieJar for FakeJar {
        fn cookies(&self, webview: &str) -> Result<Vec<(String, String)>, String> {
            Ok(self
                .cookies
                .lock()
                .unwrap()
                .get(webview)
                .cloned()
                .unwrap_or_default())
        }

        fn sign_out(&self, webview: &str) {
            self.signed_out.lock().unwrap().push(webview.to_owned());
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
    fn signing_out_closes_the_webview_and_forgets_the_session() {
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

    #[test]
    fn notices_a_renewed_token() {
        let jar = Arc::new(FakeJar::default());
        jar.cookies
            .lock()
            .unwrap()
            .insert("eb-1".into(), vec![("auth-token".into(), "one".into())]);
        let state = state_with(jar.clone(), "http://127.0.0.1:9/v1/api/");
        let id = Uuid::new_v4();
        state.add_session(id, "A", "eb-1");
        state.note_use(id, Some(hash("one")));
        assert_eq!(state.sessions()[0].renewed_at, None);
        state.note_use(id, Some(hash("two")));
        assert!(state.sessions()[0].renewed_at.is_some());
    }

    #[test]
    fn a_new_sign_in_replaces_the_old_webview() {
        let jar = Arc::new(FakeJar::default());
        let state = state_with(jar.clone(), "http://127.0.0.1:9/v1/api/");
        let id = Uuid::new_v4();
        state.add_session(id, "A", "eb-1");
        state.add_session(id, "A", "eb-2");
        assert_eq!(*jar.signed_out.lock().unwrap(), vec!["eb-1".to_owned()]);
        assert_eq!(state.session_count(), 1);
    }
}
