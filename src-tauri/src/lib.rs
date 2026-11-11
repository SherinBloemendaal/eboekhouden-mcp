mod accounts;
mod approvals;
mod connect;
mod creds;
mod ebapi;
mod hub;
mod logstore;
mod mcp;
mod reqlog;
mod secrets;
mod settings;
mod signin;
mod state;
mod tools;
mod updates;
mod write;

use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, State, WindowEvent,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use uuid::Uuid;

use crate::accounts::{Account, AccountInput, AccountStore, Store};
use crate::ebapi::EbClient;
use crate::settings::{Settings, SettingsStore};
use crate::signin::{SessionJar, Signins};
use crate::state::{AppState, SessionView};

const PANEL: &str = "panel";
const MAIN: &str = "main";

/// What the UI shows about the local server. The token itself never leaves Rust
/// unless the user asks to copy it.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerInfo {
    url: String,
    token_tail: String,
    running: bool,
    error: Option<String>,
    sessions: usize,
    log_count: usize,
    /// How many tools the server offers.
    tool_count: usize,
}

struct Server {
    token: Arc<RwLock<String>>,
    store: Box<dyn secrets::SecretStore>,
    error: Mutex<Option<String>>,
}

impl Server {
    fn token(&self) -> String {
        self.token.read().unwrap().clone()
    }
}

/// An account with what is happening to it right now.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountView {
    #[serde(flatten)]
    account: Account,
    session: Option<SessionView>,
    signing_in: bool,
    /// The newest step of this account's latest sign-in since EB MCP started, its end state included.
    signin: Option<signin::SigninEvent>,
}

type Shared<'a, T> = State<'a, Arc<T>>;

/// Events go to EB MCP's two windows, the main window and the tray panel.
pub(crate) fn to_our_windows<S: Serialize + Clone>(app: &AppHandle, event: &str, payload: S) {
    for label in [MAIN, PANEL] {
        let _ = app.emit_to(label, event, payload.clone());
    }
}

#[tauri::command]
fn server_info(server: Shared<'_, Server>, app_state: Shared<'_, AppState>) -> ServerInfo {
    let error = server.error.lock().unwrap().clone();
    ServerInfo {
        url: mcp::url(),
        token_tail: secrets::token_tail(&server.token()).to_owned(),
        running: error.is_none(),
        error,
        sessions: app_state.session_count(),
        log_count: app_state.log.len(),
        tool_count: tools::tool_count(),
    }
}

/// Something the user should know from the start of this run (a damaged accounts.json that was kept aside).
struct StartupNotice(Option<String>);

/// The notice of this start, if any; the UI shows it once it has loaded.
#[tauri::command]
fn startup_notice(notice: State<'_, StartupNotice>) -> Option<String> {
    notice.0.clone()
}

#[tauri::command]
fn accounts_list(
    app_state: Shared<'_, AppState>,
    signins: Shared<'_, Signins>,
) -> Vec<AccountView> {
    let sessions = app_state.sessions();
    app_state
        .accounts
        .list()
        .into_iter()
        .map(|account| {
            let (signing_in, signin) = signins.state_of(account.id);
            AccountView {
                session: sessions
                    .iter()
                    .find(|s| s.account_id == account.id)
                    .cloned(),
                signing_in,
                signin,
                account,
            }
        })
        .collect()
}

fn start_sign_in(app: &AppHandle, id: Uuid) {
    let (state, signins) = (
        app.state::<Arc<AppState>>().inner().clone(),
        app.state::<Arc<Signins>>().inner().clone(),
    );
    tauri::async_runtime::spawn(signin::sign_in(app.clone(), state, signins, id));
}

/// The Login items in 1Password for the account picker; never a password.
#[tauri::command]
async fn op_items(app: AppHandle) -> Result<Vec<creds::OpItem>, creds::OpListError> {
    let items = creds::op_items().await;
    refocus_main(&app);
    items
}

/// 1Password's consent prompt and Touch ID take the focus. Without a Dock icon EB MCP then
/// sits behind other apps and looks gone, so the open main window comes back to the front.
/// The panel stays open during a sign-in (see the panel's focus handler in `run`); when it is
/// open, it was where the user started, so it gets the focus last.
pub(crate) fn refocus_main(app: &AppHandle) {
    if let Some(main) = app.get_webview_window(MAIN)
        && main.is_visible().unwrap_or(false)
    {
        let _ = main.unminimize();
        let _ = main.set_focus();
    }
    if let Some(panel) = app.get_webview_window(PANEL)
        && panel.is_visible().unwrap_or(false)
    {
        let _ = panel.set_focus();
    }
}

/// The panel does not hide on focus loss while a sign-in runs (its focus handler in `run`). When the
/// last sign-in ends and the panel is open without the focus, nothing would close it any more, so it
/// hides here, as a menu the user has left.
pub(crate) fn hide_unfocused_panel(app: &AppHandle) {
    if app
        .try_state::<Arc<Signins>>()
        .is_some_and(|signins| signins.any_running())
    {
        return;
    }
    if let Some(panel) = app.get_webview_window(PANEL)
        && panel.is_visible().unwrap_or(false)
        && !panel.is_focused().unwrap_or(true)
    {
        let _ = panel.hide();
    }
}

/// Saving checks the account right away by signing in once through e-Boekhouden's API (signin.rs).
#[tauri::command]
fn account_add(
    app: AppHandle,
    app_state: Shared<'_, AppState>,
    input: AccountInput,
    password: Option<String>,
) -> Result<Account, String> {
    if input.store == Store::System && password.as_deref().unwrap_or_default().is_empty() {
        return Err("Vul het wachtwoord in".into());
    }
    let account = app_state.accounts.add(input)?;
    if account.store == Store::System
        && let Err(e) = creds::save_password(account.id, password.as_deref().unwrap_or_default())
    {
        let _ = app_state.accounts.remove(account.id);
        return Err(format!("Het wachtwoord kon niet in de sleutelhanger: {e}"));
    }
    start_sign_in(&app, account.id);
    Ok(account)
}

/// A new password is optional: empty keeps the one in the store.
#[tauri::command]
fn account_update(
    app_state: Shared<'_, AppState>,
    id: Uuid,
    input: AccountInput,
    password: Option<String>,
) -> Result<Account, String> {
    let before = app_state
        .accounts
        .get(id)
        .ok_or("Dit account bestaat niet meer")?;
    if input.store == Store::System
        && before.store != Store::System
        && password.as_deref().unwrap_or_default().is_empty()
    {
        return Err("Vul het wachtwoord in voor de sleutelhanger".into());
    }
    let account = app_state.accounts.update(id, input)?;
    // Writing turned off: whatever this account still had waiting for approval is refused. Other sign-in
    // details: what waits was asked for the old ones, so it lapses. The store changed first (approve in
    // tools.rs reads it again after it put a proposal up, so none slips through in between).
    let now = state::now_ms();
    let ended = if account.write == accounts::WriteMode::Off {
        app_state.approvals.end_account(
            id,
            approvals::Answer::Refused(approvals::REFUSED_READ_ONLY.into()),
            now,
        )
    } else if account.username != before.username || account.op_ref != before.op_ref {
        app_state.approvals.end_account(
            id,
            approvals::Answer::Lapsed(approvals::LAPSED_ACCOUNT_CHANGED.into()),
            now,
        )
    } else {
        0
    };
    if ended > 0 {
        let _ = app_state.events.send(state::AppEvent::ApprovalsChanged);
    }
    match account.store {
        Store::System => {
            if let Some(password) = password.filter(|p| !p.is_empty()) {
                creds::save_password(id, &password)?;
            }
        }
        // Switching to 1Password takes the old password out of the system store.
        Store::OnePassword => creds::delete_password(id)?,
    }
    Ok(account)
}

#[tauri::command]
fn account_remove(
    app_state: Shared<'_, AppState>,
    signins: Shared<'_, Signins>,
    id: Uuid,
) -> Result<(), String> {
    signins.cancel(id);
    app_state.sign_out(id, "account verwijderd");
    creds::delete_password(id)?;
    app_state.accounts.remove(id)?;
    // After the store: approve (tools.rs) reads it again once its proposal is up.
    if app_state.approvals.end_account(
        id,
        approvals::Answer::Refused(approvals::REFUSED_ACCOUNT_GONE.into()),
        state::now_ms(),
    ) > 0
    {
        let _ = app_state.events.send(state::AppEvent::ApprovalsChanged);
    }
    signins.forget(id);
    let _ = app_state.events.send(state::AppEvent::SessionsChanged);
    Ok(())
}

/// The changes AI clients asked for that wait for the user's approval, oldest first.
#[tauri::command]
fn approvals_list(app_state: Shared<'_, AppState>) -> Vec<approvals::Proposal> {
    app_state.approvals.list()
}

/// The user's answer to one of them. Only Goedkeuren (`approve`) lets the request go to e-Boekhouden; the change
/// then runs in the background, in the task of the tool that asked (tools.rs, `write_tool`).
#[tauri::command]
fn approval_decide(app_state: Shared<'_, AppState>, id: u64, approve: bool) -> Result<(), String> {
    let answered = app_state.approvals.decide(id, approve, state::now_ms());
    let _ = app_state.events.send(state::AppEvent::ApprovalsChanged);
    if answered {
        Ok(())
    } else {
        Err("Dit voorstel wacht niet meer: het is al beantwoord of vervallen".into())
    }
}

/// One answer for several proposals: exactly the ids the user saw, so one that came up meanwhile is never
/// approved unseen. Says which were answered now and which were not waiting any more.
#[tauri::command]
fn approvals_decide_all(
    app_state: Shared<'_, AppState>,
    ids: Vec<u64>,
    approve: bool,
) -> approvals::DecideAll {
    let done = app_state
        .approvals
        .decide_all(&ids, approve, state::now_ms());
    let _ = app_state.events.send(state::AppEvent::ApprovalsChanged);
    done
}

/// What AI clients changed or asked to change, newest first, without the technical data.
#[tauri::command]
fn approvals_history(
    app_state: Shared<'_, AppState>,
    limit: Option<usize>,
) -> Vec<approvals::Record> {
    app_state.approvals.history(limit)
}

/// One proposal with its technical data: the tool's arguments, the request body and the result.
#[tauri::command]
fn approval_get(app_state: Shared<'_, AppState>, id: u64) -> Result<approvals::Detail, String> {
    app_state
        .approvals
        .get(id)
        .ok_or_else(|| "Dit voorstel staat niet meer in de geschiedenis".into())
}

/// The usernames and 1Password references of the accounts: an export masks them wherever they show up.
fn hidden_account_values(app_state: &AppState) -> Vec<String> {
    let mut hidden = Vec::new();
    for account in app_state.accounts.list() {
        hidden.push(account.username);
        hidden.extend(account.op_ref);
    }
    hidden
}

/// Where an export goes: Downloads, or the app's config folder when that fails.
fn export_dirs(app: &AppHandle) -> Result<Vec<std::path::PathBuf>, String> {
    let dirs: Vec<std::path::PathBuf> = [app.path().download_dir(), app.path().app_config_dir()]
        .into_iter()
        .filter_map(Result::ok)
        .collect();
    if dirs.is_empty() {
        return Err("Er is geen map gevonden om de export in te bewaren.".into());
    }
    Ok(dirs)
}

/// The whole approval history with its technical data, as one gzip file in Downloads (or the app's config
/// folder): a header line, then one record per line, oldest first. Returns its absolute path.
#[tauri::command]
async fn approvals_export(
    app: AppHandle,
    app_state: Shared<'_, AppState>,
) -> Result<String, String> {
    let hidden = hidden_account_values(&app_state);
    let dirs = export_dirs(&app)?;
    let state = app_state.inner().clone();
    let written = tauri::async_runtime::spawn_blocking(move || {
        let exported_at = chrono::Local::now();
        let mut failures = Vec::new();
        for dir in &dirs {
            match state.approvals.export(dir, &hidden, &exported_at) {
                Ok(path) => return Ok(path),
                Err(e) => {
                    log::warn!("approval history export not written: {e}");
                    failures.push(e);
                }
            }
        }
        Err(failures.join("; "))
    })
    .await
    .map_err(|e| format!("De export kon niet worden gemaakt: {e}"))?;
    written
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(|e| format!("De export kon niet worden bewaard: {e}"))
}

#[tauri::command]
fn signin_start(app: AppHandle, id: Uuid) {
    start_sign_in(&app, id);
}

/// Instellingen › Diagnose: the last sign-ins, step by step, without passwords, codes or cookies.
#[tauri::command]
fn signin_traces(signins: Shared<'_, Signins>) -> Vec<signin::Trace> {
    signins.traces()
}

/// Opens the folder with EB MCP's log files in Finder or Explorer.
#[tauri::command]
fn open_log_dir(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let dir = app.path().app_log_dir().map_err(|e| e.to_string())?;
    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn signin_cancel(signins: Shared<'_, Signins>, id: Uuid) {
    signins.cancel(id);
}

#[tauri::command]
fn signin_code(signins: Shared<'_, Signins>, id: Uuid, code: String) -> Result<(), String> {
    signins.give_code(id, code)
}

#[tauri::command]
fn sign_out(app_state: Shared<'_, AppState>, id: Uuid) {
    app_state.sign_out(id, "Uitloggen gekozen");
}

#[tauri::command]
fn sign_out_all(app_state: Shared<'_, AppState>) {
    app_state.sign_out_all("Alles uitloggen gekozen");
}

/// The request log, newest first and without bodies.
#[tauri::command]
fn log_list(app_state: Shared<'_, AppState>) -> Vec<reqlog::Entry> {
    app_state.log.list()
}

/// One entry with its request and response, already masked.
#[tauri::command]
fn log_get(app_state: Shared<'_, AppState>, id: u64) -> Option<reqlog::Entry> {
    app_state.log.get(id)
}

/// Leegmaken empties the log on disk too, so it waits off the main thread for a save that may be running.
#[tauri::command]
async fn log_clear(app_state: Shared<'_, AppState>) -> Result<(), String> {
    let state = app_state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.log.clear())
        .await
        .map_err(|e| format!("Leegmaken lukte niet: {e}"))?;
    let _ = app_state.events.send(state::AppEvent::LogChanged);
    Ok(())
}

/// How often the request log is saved at most, when it changed.
const LOG_SAVE_INTERVAL: Duration = Duration::from_secs(30);

/// Saves the request log and the approval history when they changed, compressing them off the async
/// runtime's threads.
async fn save_log(app_state: &Arc<AppState>) {
    if app_state.log.dirty() {
        let state = Arc::clone(app_state);
        match tauri::async_runtime::spawn_blocking(move || state.log.save()).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => log::warn!("request log not saved: {e}"),
            Err(e) => log::warn!("request log not saved: {e}"),
        }
    }
    save_approvals(app_state).await;
}

/// Saves the approval history when it changed: after every change of the queue or the history, and with the
/// request log.
async fn save_approvals(app_state: &Arc<AppState>) {
    if !app_state.approvals.dirty() {
        return;
    }
    let state = Arc::clone(app_state);
    match tauri::async_runtime::spawn_blocking(move || state.approvals.save()).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => log::warn!("approval history not saved: {e}"),
        Err(e) => log::warn!("approval history not saved: {e}"),
    }
}

/// The menu-bar icon's count of changes that wait for the user: the title next to the icon (macOS) and
/// the tooltip. Nothing next to the icon when none wait.
fn tray_badge(waiting: usize) -> (Option<String>, String) {
    match waiting {
        0 => (None, "EB MCP".to_owned()),
        1 => (
            Some("1".to_owned()),
            "EB MCP: 1 wijziging wacht op je akkoord".to_owned(),
        ),
        n => (
            Some(n.to_string()),
            format!("EB MCP: {n} wijzigingen wachten op je akkoord"),
        ),
    }
}

const TRAY_ID: &str = "eb-mcp";

fn update_tray(app: &AppHandle, waiting: usize) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let (title, tooltip) = tray_badge(waiting);
    #[cfg(target_os = "macos")]
    let _ = tray.set_title(title);
    #[cfg(not(target_os = "macos"))]
    let _ = title;
    let _ = tray.set_tooltip(Some(tooltip));
}

/// The main window's page with the proposals (src/router.ts).
const APPROVALS_ROUTE: &str = "/akkoord";

/// How long proposals that come up close together are gathered into one notification.
const NOTIFY_GATHER: Duration = Duration::from_secs(2);

/// Remembers the highest proposal id that was considered for a notification, so each proposal is
/// announced at most once.
struct Arrivals {
    seen: u64,
}

impl Arrivals {
    /// The waiting proposals that came up since the last look; `last_id` is the highest id given out now.
    fn fresh(&mut self, waiting: &[approvals::Proposal], last_id: u64) -> Vec<approvals::Proposal> {
        let fresh = waiting
            .iter()
            .filter(|p| p.id > self.seen)
            .cloned()
            .collect();
        self.seen = self.seen.max(last_id);
        fresh
    }
}

/// The notification for proposals that came up together: its title and its text.
fn notification_text(fresh: &[approvals::Proposal]) -> (String, String) {
    const SHOWN: usize = 3;
    let title = match fresh.len() {
        1 => "EB MCP: 1 wijziging wacht op je akkoord".to_owned(),
        n => format!("EB MCP: {n} wijzigingen wachten op je akkoord"),
    };
    let mut lines: Vec<String> = fresh.iter().take(SHOWN).map(|p| p.title.clone()).collect();
    if fresh.len() > SHOWN {
        lines.push(format!("en nog {}", fresh.len() - SHOWN));
    }
    (title, lines.join("\n"))
}

/// Whether one of EB MCP's windows has the focus: then the user sees the proposal in the app already.
fn any_window_focused(app: &AppHandle) -> bool {
    [MAIN, PANEL].into_iter().any(|label| {
        app.get_webview_window(label)
            .is_some_and(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
    })
}

/// A system notification for proposals that came up. When notifications are not allowed or not available,
/// nothing happens: the count in the menu bar and the app itself show what waits.
fn notify_waiting(app: &AppHandle, fresh: &[approvals::Proposal]) {
    use tauri::plugin::PermissionState;
    use tauri_plugin_notification::NotificationExt;
    let notification = app.notification();
    let allowed = match notification.permission_state() {
        Ok(PermissionState::Granted) => true,
        Ok(PermissionState::Prompt | PermissionState::PromptWithRationale) => matches!(
            notification.request_permission(),
            Ok(PermissionState::Granted)
        ),
        Ok(PermissionState::Denied) | Err(_) => false,
    };
    if !allowed {
        log::info!("no notification for a waiting change: notifications are not allowed");
        return;
    }
    let (title, body) = notification_text(fresh);
    if let Err(e) = notification.builder().title(title).body(body).show() {
        log::info!("no notification for a waiting change: {e}");
    }
}

/// A debug export of the request log for a bug report: one gzip file in Downloads (or, when that
/// fails, the app's config folder) with the log, the statistics and the last sign-ins. Returns its
/// absolute path. Accounts appear by label only.
#[tauri::command]
async fn log_export(
    app: AppHandle,
    app_state: Shared<'_, AppState>,
    signins: Shared<'_, Signins>,
) -> Result<String, String> {
    let sessions = app_state.sessions();
    let hidden = hidden_account_values(&app_state);
    let accounts: Vec<logstore::ExportAccount> = app_state
        .accounts
        .list()
        .into_iter()
        .map(|account| logstore::ExportAccount {
            signed_in: sessions
                .iter()
                .any(|s| s.account_id == account.id && !s.expired),
            label: account.label,
            administration: account.administration,
            store: account.store,
            write: account.write,
        })
        .collect();
    // Oldest first, like the entries.
    let mut traces = signins.traces();
    traces.reverse();
    let dirs = export_dirs(&app)?;
    let state = app_state.inner().clone();
    let written = tauri::async_runtime::spawn_blocking(move || {
        let input = logstore::ExportInput {
            accounts: &accounts,
            traces: &traces,
            hidden: &hidden,
            exported_at: chrono::Local::now(),
        };
        let mut failures = Vec::new();
        for dir in &dirs {
            match logstore::export(dir, &state.log, &input) {
                Ok(path) => return Ok(path),
                Err(e) => {
                    log::warn!("debug export not written: {e}");
                    failures.push(e);
                }
            }
        }
        Err(failures.join("; "))
    })
    .await
    .map_err(|e| format!("De export kon niet worden gemaakt: {e}"))?;
    written
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(|e| format!("De export kon niet worden bewaard: {e}"))
}

/// Shows an export from `log_export` or `approvals_export` in Finder or Explorer. Only those files: any other
/// path is refused.
#[tauri::command]
fn log_export_reveal(app: AppHandle, path: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let path = std::path::PathBuf::from(path);
    if !logstore::is_export(&path) {
        return Err("Dit exportbestand bestaat niet meer.".into());
    }
    app.opener()
        .reveal_item_in_dir(&path)
        .map_err(|e| format!("Het exportbestand kon niet worden getoond: {e}"))
}

#[tauri::command]
fn stats(app_state: Shared<'_, AppState>) -> reqlog::StatsView {
    app_state.log.stats()
}

/// The full token, only when the user asks to see or copy it.
#[tauri::command]
fn token_reveal(server: Shared<'_, Server>) -> String {
    server.token()
}

/// A new token; linked clients then need their file written again.
#[tauri::command]
fn token_renew(server: Shared<'_, Server>) -> Result<String, String> {
    let token = secrets::renew_token(server.store.as_ref())?;
    *server.token.write().unwrap() = token.clone();
    Ok(secrets::token_tail(&token).to_owned())
}

/// Next to the app's own executable: inside the .app bundle, or target/<profile> in development.
fn bridge_path() -> std::path::PathBuf {
    let name = if cfg!(windows) {
        "eb-mcp-bridge.exe"
    } else {
        "eb-mcp-bridge"
    };
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(name)))
        .unwrap_or_else(|| name.into())
}

/// The bridge Claude Desktop starts. It finds the token file of the installed app by itself;
/// development builds keep their own file (secrets::token_file) and pass it.
fn bridge(app: &AppHandle) -> connect::Bridge {
    let token_file = cfg!(debug_assertions)
        .then(|| app.path().app_config_dir().ok())
        .flatten()
        .map(|dir| secrets::token_file(&dir));
    connect::Bridge {
        path: bridge_path(),
        token_file,
    }
}

#[tauri::command]
fn clients_status(app: AppHandle, server: Shared<'_, Server>) -> Vec<connect::ClientStatus> {
    let paths = connect::Paths::system();
    let (token, bridge) = (server.token(), bridge(&app));
    [
        connect::Client::Claude,
        connect::Client::Cursor,
        connect::Client::ClaudeCode,
        connect::Client::Codex,
    ]
    .into_iter()
    .map(|c| connect::status(&paths, c, &mcp::url(), &token, &bridge))
    .collect()
}

#[tauri::command]
fn connect_preview(
    app: AppHandle,
    server: Shared<'_, Server>,
    client: connect::Client,
) -> Result<connect::Preview, String> {
    connect::preview(
        &connect::Paths::system(),
        client,
        &mcp::url(),
        &server.token(),
        &bridge(&app),
    )
}

#[tauri::command]
fn connect_link(
    app: AppHandle,
    server: Shared<'_, Server>,
    client: connect::Client,
) -> Result<(), String> {
    let needs_bridge = matches!(client, connect::Client::Claude | connect::Client::Codex);
    if needs_bridge && !bridge_path().is_file() {
        return Err(
            "De brug (eb-mcp-bridge) ontbreekt naast de app. Installeer EB MCP opnieuw.".into(),
        );
    }
    connect::link(
        &connect::Paths::system(),
        client,
        &mcp::url(),
        &server.token(),
        &bridge(&app),
    )
}

#[tauri::command]
fn connect_unlink(client: connect::Client) -> Result<(), String> {
    connect::unlink(&connect::Paths::system(), client)
}

/// Settings to paste into any other MCP client that speaks HTTP. Contains the token: only on request.
#[tauri::command]
fn other_client_settings(server: Shared<'_, Server>) -> String {
    let value = serde_json::json!({ "e-boekhouden": { "url": mcp::url(), "headers": { "Authorization": format!("Bearer {}", server.token()) } } });
    serde_json::to_string_pretty(&value).unwrap_or_default()
}

/// Before restarting into an update: refuse new tool calls, wait (at most 30 s) for the running ones, then sign
/// every session out at e-Boekhouden. `false` puts things back when the user chose to wait.
#[tauri::command]
async fn prepare_update_restart(
    app_state: Shared<'_, AppState>,
    ready: bool,
) -> Result<(), String> {
    if !ready {
        app_state.set_updating(false);
        return Ok(());
    }
    app_state.set_updating(true);
    for _ in 0..60 {
        if app_state.log.running() == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    // Sessions are only in memory, so forgetting them is immediate.
    app_state.sign_out_all("herstart voor een update");
    // The request log and its statistics carry over into the new version.
    save_log(app_state.inner()).await;
    Ok(())
}

#[tauri::command]
fn running_calls(app_state: Shared<'_, AppState>) -> usize {
    app_state.log.running()
}

#[tauri::command]
fn settings_get(settings: Shared<'_, SettingsStore>) -> Settings {
    settings.get()
}

#[tauri::command]
fn settings_set(
    app: AppHandle,
    settings: Shared<'_, SettingsStore>,
    value: Settings,
) -> Result<(), String> {
    let before = settings.get();
    let (appearance, dock) = (value.appearance, value.show_in_dock);
    settings.set(value)?;
    // Only what changed: setting the Dock policy deactivates the app and brings the window back
    // (apply_dock), which should not happen when only the idle time changed.
    if appearance != before.appearance {
        apply_appearance(&app, appearance);
    }
    if dock != before.show_in_dock {
        apply_dock(&app, dock);
    }
    Ok(())
}

/// macOS: with the Dock icon EB MCP is a regular app (Dock, Cmd+Tab); without it only the menu-bar
/// icon shows. Either way it keeps running in the background when its window is closed.
fn apply_dock(app: &AppHandle, show: bool) {
    #[cfg(target_os = "macos")]
    {
        let policy = if show {
            tauri::ActivationPolicy::Regular
        } else {
            tauri::ActivationPolicy::Accessory
        };
        let _ = app.set_activation_policy(policy);
        // Changing the policy deactivates the app and sends the window behind the others, which looks like
        // it closed. Once macOS has applied it, the window that was open comes back to the front.
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            let _ = handle.show();
            refocus_main(&handle);
        });
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, show);
}

/// Started by the login item: stay in the menu bar instead of opening the window.
const AUTOSTART_ARG: &str = "--autostart";

#[tauri::command]
fn autostart_get(app: AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
fn autostart_set(app: AppHandle, enabled: bool) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    let manager = app.autolaunch();
    if enabled {
        manager.enable()
    } else {
        manager.disable()
    }
    .map_err(|e| format!("Openen bij inloggen lukte niet: {e}"))?;
    manager.is_enabled().map_err(|e| e.to_string())
}

/// Sets EB MCP's windows (main and panel) to light, dark or the system's choice; the pages
/// follow through prefers-color-scheme.
fn apply_appearance(app: &AppHandle, appearance: settings::Appearance) {
    let theme = match appearance {
        settings::Appearance::System => None,
        settings::Appearance::Light => Some(tauri::Theme::Light),
        settings::Appearance::Dark => Some(tauri::Theme::Dark),
    };
    for label in [MAIN, PANEL] {
        if let Some(window) = app.get_webview_window(label) {
            let _ = window.set_theme(theme);
        }
    }
}

/// For a sign-in that stopped at a step EB MCP does not know: the user goes on in their own browser.
#[tauri::command]
fn open_eboekhouden(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(signin::LOGIN_URL, None::<&str>)
        .map_err(|e| e.to_string())
}

/// The 1Password CLI's install guide, for the picker when `op` is missing.
#[tauri::command]
fn open_op_install(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(
            "https://developer.1password.com/docs/cli/get-started/",
            None::<&str>,
        )
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn open_main(app: AppHandle, route: Option<String>) {
    if let Some(panel) = app.get_webview_window(PANEL) {
        let _ = panel.hide();
    }
    if let Some(main) = app.get_webview_window(MAIN) {
        let _ = main.show();
        let _ = main.set_focus();
        if let Some(route) = route {
            let _ = main.eval(format!(
                "window.location.hash = {}",
                serde_json::to_string(&route).unwrap_or_default()
            ));
        }
    }
}

/// Quitting signs every session out at e-Boekhouden first.
#[tauri::command]
async fn quit(app: AppHandle, app_state: Shared<'_, AppState>) -> Result<(), String> {
    quit_now(&app, app_state.inner()).await;
    Ok(())
}

/// Signs every session out and saves the request log, then quits.
async fn quit_now(app: &AppHandle, app_state: &Arc<AppState>) {
    app_state.sign_out_all("EB MCP afgesloten");
    save_log(app_state).await;
    app.exit(0);
}

/// Sent to the main window when Stop EB MCP (Cmd+Q) is chosen while AI calls still run; it asks first.
#[cfg(target_os = "macos")]
const QUIT_REQUESTED: &str = "quit-requested";

/// The macOS menu bar, as every Mac app has it: Instellingen and Zoek naar updates in the app menu,
/// the usual Edit menu (copy and paste in fields need it) and a Window menu.
#[cfg(target_os = "macos")]
fn build_menu(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu};
    let sep = || PredefinedMenuItem::separator(app);
    let about = AboutMetadata {
        name: Some("EB MCP".into()),
        comments: Some("e-Boekhouden MCP: je boekhouding voor AI-clients, alleen lezen.".into()),
        website: Some("https://sherin.dev/eb-mcp/".into()),
        ..Default::default()
    };
    let app_menu = Submenu::with_items(
        app,
        "EB MCP",
        true,
        &[
            &PredefinedMenuItem::about(app, Some("Over EB MCP"), Some(about))?,
            &MenuItem::with_id(app, MENU_UPDATES, "Zoek naar updates…", true, None::<&str>)?,
            &sep()?,
            &MenuItem::with_id(
                app,
                MENU_SETTINGS,
                "Instellingen…",
                true,
                Some("CmdOrCtrl+,"),
            )?,
            &sep()?,
            &PredefinedMenuItem::services(app, Some("Voorzieningen"))?,
            &sep()?,
            &PredefinedMenuItem::hide(app, Some("Verberg EB MCP"))?,
            &PredefinedMenuItem::hide_others(app, Some("Verberg andere"))?,
            &PredefinedMenuItem::show_all(app, Some("Toon alles"))?,
            &sep()?,
            &MenuItem::with_id(app, MENU_QUIT, "Stop EB MCP", true, Some("CmdOrCtrl+Q"))?,
        ],
    )?;
    let file_menu = Submenu::with_items(
        app,
        "Archief",
        true,
        &[
            &MenuItem::with_id(app, MENU_SETTINGS_FILE, "Instellingen…", true, None::<&str>)?,
            &MenuItem::with_id(
                app,
                MENU_UPDATES_FILE,
                "Zoek naar updates…",
                true,
                None::<&str>,
            )?,
            &sep()?,
            &PredefinedMenuItem::close_window(app, Some("Sluit venster"))?,
        ],
    )?;
    let edit_menu = Submenu::with_items(
        app,
        "Wijzig",
        true,
        &[
            &PredefinedMenuItem::undo(app, Some("Herstel"))?,
            &PredefinedMenuItem::redo(app, Some("Opnieuw"))?,
            &sep()?,
            &PredefinedMenuItem::cut(app, Some("Knip"))?,
            &PredefinedMenuItem::copy(app, Some("Kopieer"))?,
            &PredefinedMenuItem::paste(app, Some("Plak"))?,
            &PredefinedMenuItem::select_all(app, Some("Selecteer alles"))?,
        ],
    )?;
    let window_menu = Submenu::with_items(
        app,
        "Venster",
        true,
        &[
            &PredefinedMenuItem::minimize(app, Some("Minimaliseer"))?,
            &PredefinedMenuItem::maximize(app, Some("Zoom"))?,
        ],
    )?;
    app.set_menu(Menu::with_items(
        app,
        &[&app_menu, &file_menu, &edit_menu, &window_menu],
    )?)?;
    app.on_menu_event(|app, event| match event.id().as_ref() {
        MENU_SETTINGS | MENU_SETTINGS_FILE => open_main(app.clone(), Some("/instellingen".into())),
        MENU_UPDATES | MENU_UPDATES_FILE => updates::check_for_updates_natively(app.clone()),
        MENU_QUIT => {
            let state = app.state::<Arc<AppState>>();
            // Calls that still run would fail, and changes that wait for approval would lapse: ask first.
            if state.log.running() > 0 || state.approvals.count() > 0 {
                open_main(app.clone(), None);
                let _ = app.emit_to(MAIN, QUIT_REQUESTED, state.log.running());
            } else {
                let (app, state) = (app.clone(), Arc::clone(&state));
                tauri::async_runtime::spawn(async move { quit_now(&app, &state).await });
            }
        }
        _ => {}
    });
    Ok(())
}

#[cfg(target_os = "macos")]
const MENU_SETTINGS: &str = "settings";
#[cfg(target_os = "macos")]
const MENU_SETTINGS_FILE: &str = "settings-file";
#[cfg(target_os = "macos")]
const MENU_UPDATES: &str = "check-updates";
#[cfg(target_os = "macos")]
const MENU_UPDATES_FILE: &str = "check-updates-file";
#[cfg(target_os = "macos")]
const MENU_QUIT: &str = "quit";

/// The panel takes the height of its content (PanelView measures it), between limits that keep it
/// on screen. On Windows it opens above the tray, so the bottom edge stays where it is.
#[tauri::command]
fn panel_fit(app: AppHandle, height: f64) {
    let Some(panel) = app.get_webview_window(PANEL) else {
        return;
    };
    let height = height.clamp(PANEL_MIN_HEIGHT, PANEL_MAX_HEIGHT);
    let scale = panel.scale_factor().unwrap_or(1.0);
    let before = panel.outer_size().ok();
    if before.is_some_and(|s| (s.height as f64 - height * scale).abs() < 1.0) {
        return;
    }
    let _ = panel.set_size(tauri::LogicalSize::new(PANEL_WIDTH, height));
    if cfg!(target_os = "windows")
        && let (Some(before), Ok(pos), Ok(after)) =
            (before, panel.outer_position(), panel.outer_size())
    {
        let shift = before.height as i32 - after.height as i32;
        let _ = panel.set_position(PhysicalPosition::new(pos.x, pos.y + shift));
    }
}

const PANEL_WIDTH: f64 = 336.0;
const PANEL_MIN_HEIGHT: f64 = 160.0;
const PANEL_MAX_HEIGHT: f64 = 640.0;
/// Room kept between the panel and the edges of the screen's work area, in logical pixels.
const PANEL_SCREEN_MARGIN: f64 = 8.0;

/// When the panel last hid because it lost the focus. A click on the tray icon takes the focus
/// from the open panel before the click's mouse-up arrives, so that click would open it again.
#[derive(Default)]
struct PanelHidden(Mutex<Option<Instant>>);

/// A tray click this soon after the panel hid on focus loss is the click that hid it.
const PANEL_REOPEN_GUARD: Duration = Duration::from_millis(250);

/// Keeps a span (`start`, `len`) inside an area (`area_start`, `area_len`) with `margin` to spare
/// on both sides. An area too small for it gets the span at its start.
fn clamp_span(start: f64, len: f64, area_start: f64, area_len: f64, margin: f64) -> f64 {
    let (min, max) = (area_start + margin, area_start + area_len - margin - len);
    if max < min {
        min
    } else {
        start.clamp(min, max)
    }
}

/// The work area (without menu bar, Dock or taskbar) of the screen holding a point, in physical
/// pixels: (x, y, width, height).
fn work_area_at(app: &AppHandle, x: f64, y: f64) -> Option<(f64, f64, f64, f64)> {
    let monitor = app.monitor_from_point(x, y).ok().flatten().or_else(|| {
        app.available_monitors().ok()?.into_iter().find(|m| {
            let (pos, size) = (m.position(), m.size());
            x >= f64::from(pos.x)
                && x < f64::from(pos.x) + f64::from(size.width)
                && y >= f64::from(pos.y)
                && y < f64::from(pos.y) + f64::from(size.height)
        })
    })?;
    let area = monitor.work_area();
    Some((
        f64::from(area.position.x),
        f64::from(area.position.y),
        f64::from(area.size.width),
        f64::from(area.size.height),
    ))
}

fn toggle_panel(app: &AppHandle, rect: tauri::Rect) {
    let Some(panel) = app.get_webview_window(PANEL) else {
        return;
    };
    if panel.is_visible().unwrap_or(false) {
        let _ = panel.hide();
        return;
    }
    if let Some(hidden) = app.try_state::<PanelHidden>()
        && hidden
            .0
            .lock()
            .unwrap()
            .take()
            .is_some_and(|at| at.elapsed() < PANEL_REOPEN_GUARD)
    {
        return;
    }
    let scale = panel.scale_factor().unwrap_or(1.0);
    let (pos, size) = (
        rect.position.to_physical::<f64>(scale),
        rect.size.to_physical::<f64>(scale),
    );
    let width = panel
        .outer_size()
        .map(|s| s.width as f64)
        .unwrap_or(336.0 * scale);
    let height = panel
        .outer_size()
        .map(|s| s.height as f64)
        .unwrap_or(540.0 * scale);
    // Centred under the icon on macOS; on Windows the tray sits at the bottom, so open above it.
    let mut x = pos.x + size.width / 2.0 - width / 2.0;
    let mut y = if cfg!(target_os = "windows") {
        pos.y - height - 8.0 * scale
    } else {
        pos.y + size.height + 6.0 * scale
    };
    // An icon near the edge of the screen would put part of the panel off it. On macOS the panel
    // hangs from the menu bar, so only its x moves; the Windows tray can sit on any edge.
    if let Some((area_x, area_y, area_width, area_height)) =
        work_area_at(app, pos.x + size.width / 2.0, pos.y + size.height / 2.0)
    {
        let margin = PANEL_SCREEN_MARGIN * scale;
        x = clamp_span(x, width, area_x, area_width, margin);
        if cfg!(target_os = "windows") {
            y = clamp_span(y, height, area_y, area_height, margin);
        }
    }
    let _ = panel.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
    let _ = panel.show();
    let _ = panel.set_focus();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // First, so a second start hands over before anything else sets up: it shows the running
        // app's window and quits.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            open_main(app.clone(), None);
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .build(),
        )
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![AUTOSTART_ARG]),
        ))
        .manage(updates::UpdateListener::default())
        .manage(PanelHidden::default())
        // A new page in the main window does not listen for Sparkle's relaunch question until it says so (updates.rs).
        .on_page_load(|webview, payload| {
            if webview.label() == MAIN && payload.event() == tauri::webview::PageLoadEvent::Started
            {
                webview.state::<updates::UpdateListener>().page_loading();
            }
        })
        .invoke_handler(tauri::generate_handler![
            server_info,
            panel_fit,
            autostart_get,
            autostart_set,
            accounts_list,
            account_add,
            account_update,
            account_remove,
            signin_start,
            signin_cancel,
            signin_code,
            sign_out,
            sign_out_all,
            settings_get,
            settings_set,
            open_eboekhouden,
            op_items,
            signin_traces,
            approvals_list,
            approval_decide,
            approvals_decide_all,
            approvals_history,
            approval_get,
            approvals_export,
            open_log_dir,
            open_op_install,
            log_list,
            log_get,
            log_clear,
            log_export,
            log_export_reveal,
            stats,
            token_reveal,
            token_renew,
            clients_status,
            connect_preview,
            connect_link,
            connect_unlink,
            other_client_settings,
            prepare_update_restart,
            running_calls,
            startup_notice,
            updates::update_channel,
            updates::check_for_updates_natively,
            updates::update_listening,
            updates::update_relaunch,
            open_main,
            quit
        ])
        .setup(|app| {
            // macOS updates through Sparkle; Windows through Tauri's updater, only in builds that carry its
            // configuration (release builds, scripts/release-config.mjs).
            #[cfg(target_os = "macos")]
            updates::setup(app);
            #[cfg(not(target_os = "macos"))]
            if app.config().plugins.0.contains_key("updater") {
                app.handle()
                    .plugin(tauri_plugin_updater::Builder::new().build())?;
            }

            let dir = app.path().app_config_dir()?;
            let (accounts, notice) =
                AccountStore::open_or_recover(&dir).map_err(std::io::Error::other)?;
            app.manage(StartupNotice(notice));
            let settings = Arc::new(SettingsStore::open(&dir));
            apply_appearance(app.handle(), settings.get().appearance);
            apply_dock(app.handle(), settings.get().show_in_dock);
            #[cfg(target_os = "macos")]
            build_menu(app)?;
            // The main window starts hidden (tauri.conf.json), so a start by the login item stays
            // in the menu bar without a window flashing up; any other start shows it.
            if !std::env::args().any(|a| a == AUTOSTART_ARG)
                && let Some(main) = app.get_webview_window(MAIN)
            {
                let _ = main.show();
                let _ = main.set_focus();
            }
            let app_state = Arc::new(AppState::new(
                accounts,
                EbClient::production(),
                Arc::new(SessionJar::default()),
            ));
            // The request log and its statistics of earlier runs, and the approval history, before the
            // server takes its first call. What still waited for an answer when EB MCP stopped lapses.
            app_state.log.open(logstore::log_file(&dir));
            app_state.approvals.open(approvals::history_file(&dir));
            app.manage(app_state.clone());
            app.manage(settings.clone());
            app.manage(Arc::new(Signins::default()));

            // The request log is saved when it changed, at most every LOG_SAVE_INTERVAL, and at quit.
            let saving = app_state.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(LOG_SAVE_INTERVAL).await;
                    save_log(&saving).await;
                }
            });

            // A click on a notification of a waiting change opens the main window at Akkoord.
            {
                use tauri_plugin_notification::NotificationExt;
                let handle = app.handle().clone();
                let _ = app
                    .notification()
                    .on_action(move |_| open_main(handle.clone(), Some(APPROVALS_ROUTE.into())));
            }

            // Proposals that came up while no EB MCP window had the focus get a notification; those close
            // together share one.
            let (arrived, mut arrivals) = tokio::sync::mpsc::unbounded_channel::<()>();
            let (handle, notifying) = (app.handle().clone(), app_state.clone());
            tauri::async_runtime::spawn(async move {
                let mut seen = Arrivals {
                    seen: notifying.approvals.last_id(),
                };
                while arrivals.recv().await.is_some() {
                    tokio::time::sleep(NOTIFY_GATHER).await;
                    while arrivals.try_recv().is_ok() {}
                    let fresh =
                        seen.fresh(&notifying.approvals.list(), notifying.approvals.last_id());
                    if !fresh.is_empty() && !any_window_focused(&handle) {
                        notify_waiting(&handle, &fresh);
                    }
                }
            });

            // Forward state changes to every window. A change of the queue or the history also updates the
            // count in the menu bar, may lead to a notification and is saved. The app never takes the focus
            // for it: the AI client does not wait for the answer.
            let mut events = app_state.events.subscribe();
            let handle = app.handle().clone();
            let watching = app_state.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    let event = match events.recv().await {
                        Ok(event) => event,
                        // Events were dropped while this loop was busy: one of them may have been about
                        // the queue, so the count and the windows catch up as if it was.
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            state::AppEvent::ApprovalsChanged
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    };
                    if matches!(event, state::AppEvent::ApprovalsChanged) {
                        update_tray(&handle, watching.approvals.count());
                        let _ = arrived.send(());
                        let saving = watching.clone();
                        tauri::async_runtime::spawn(async move { save_approvals(&saving).await });
                    }
                    to_our_windows(&handle, "state", event);
                }
            });

            // Idle sign-out, checked every minute.
            let idle_state = app_state.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    let minutes = settings.get().idle_minutes;
                    idle_state.sign_out_idle(Duration::from_secs(u64::from(minutes) * 60));
                }
            });

            // The MCP token lives in a file only the user can read (secrets::token_store), where
            // the bridge reads it too without a Keychain prompt. Release builds move the token of an
            // older version out of the Keychain once, so clients linked with it keep working.
            let token_store = secrets::token_store(&dir);
            #[cfg(not(debug_assertions))]
            secrets::migrate_token(token_store.as_ref(), &secrets::SystemStore);
            let server = match secrets::load_or_create_token(token_store.as_ref()) {
                Ok(token) => {
                    let token = Arc::new(RwLock::new(token));
                    let server = Arc::new(Server {
                        token: token.clone(),
                        store: token_store,
                        error: Mutex::new(None),
                    });
                    let (serve_state, report, events) =
                        (app_state.clone(), server.clone(), app_state.events.clone());
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = mcp::serve(serve_state, token).await {
                            log::error!("MCP server stopped: {e}");
                            *report.error.lock().unwrap() =
                                Some(format!("Poort {} is niet vrij: {e}", mcp::PORT));
                            // The windows read server_info again on a state event; without one they
                            // keep showing the server as running.
                            let _ = events.send(state::AppEvent::SessionsChanged);
                        }
                    });
                    server
                }
                Err(e) => Arc::new(Server {
                    token: Arc::new(RwLock::new(String::new())),
                    store: token_store,
                    error: Mutex::new(Some(format!(
                        "Het MCP-token in {} kon niet worden gelezen of bewaard: {e}",
                        secrets::token_file(&dir).display()
                    ))),
                }),
            };
            app.manage(server);

            // macOS: a template image the menu bar colours itself. Windows has no template icons and the taskbar can be
            // light or dark, so it gets the coloured app icon.
            #[cfg(target_os = "macos")]
            let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray@2x.png"))?;
            #[cfg(not(target_os = "macos"))]
            let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png"))?;
            TrayIconBuilder::with_id(TRAY_ID)
                .icon(icon)
                .icon_as_template(cfg!(target_os = "macos"))
                .tooltip("EB MCP")
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        rect,
                        ..
                    } = event
                    {
                        toggle_panel(tray.app_handle(), rect);
                    }
                })
                .build(app)?;

            // The panel closes when it loses the focus, like a menu. Not during a sign-in: Touch ID
            // and 1Password's prompt take the focus, and the panel shows how the sign-in goes.
            if let Some(panel) = app.get_webview_window(PANEL) {
                let handle = panel.clone();
                panel.on_window_event(move |event| {
                    if let WindowEvent::Focused(false) = event {
                        let app = handle.app_handle();
                        if app
                            .try_state::<Arc<Signins>>()
                            .is_some_and(|signins| signins.any_running())
                            || !handle.is_visible().unwrap_or(false)
                        {
                            return;
                        }
                        let _ = handle.hide();
                        if let Some(hidden) = app.try_state::<PanelHidden>() {
                            *hidden.0.lock().unwrap() = Some(Instant::now());
                        }
                    }
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the main window keeps the app (and the server) running in the menu bar.
            if window.label() == MAIN
                && let WindowEvent::CloseRequested { api, .. } = event
            {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("EB MCP kon niet starten")
        .run(|app, event| match event {
            // Any other way out (an updater's relaunch, the system ending the app): the request log
            // is saved once more when it changed since the last save.
            tauri::RunEvent::Exit => {
                if let Some(state) = app.try_state::<Arc<AppState>>() {
                    if let Err(e) = state.log.save() {
                        log::warn!("request log not saved at exit: {e}");
                    }
                    if let Err(e) = state.approvals.save() {
                        log::warn!("approval history not saved at exit: {e}");
                    }
                }
            }
            // A click on the Dock icon (when it shows) opens the window again.
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen {
                has_visible_windows: false,
                ..
            } => open_main(app.clone(), None),
            _ => {}
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proposal(id: u64, title: &str) -> approvals::Proposal {
        approvals::Proposal {
            id,
            account_id: Uuid::nil(),
            account: "Studio Noord".into(),
            client: "Claude".into(),
            tool: "mutatie_toevoegen".into(),
            title: title.into(),
            lines: Vec::new(),
            danger: false,
            asked_at: 1,
        }
    }

    #[test]
    fn the_menu_bar_counts_what_waits() {
        assert_eq!(tray_badge(0), (None, "EB MCP".to_owned()));
        assert_eq!(
            tray_badge(1),
            (
                Some("1".to_owned()),
                "EB MCP: 1 wijziging wacht op je akkoord".to_owned()
            )
        );
        assert_eq!(
            tray_badge(12),
            (
                Some("12".to_owned()),
                "EB MCP: 12 wijzigingen wachten op je akkoord".to_owned()
            )
        );
    }

    #[test]
    fn proposals_that_come_up_together_share_one_notification() {
        let mut seen = Arrivals { seen: 4 };
        // 3 was there before EB MCP started looking; 5 was answered before the look.
        let waiting = [
            proposal(3, "Oud"),
            proposal(6, "Eerste"),
            proposal(7, "Tweede"),
        ];
        let fresh = seen.fresh(&waiting, 7);
        assert_eq!(fresh.iter().map(|p| p.id).collect::<Vec<_>>(), vec![6, 7]);
        assert_eq!(
            notification_text(&fresh),
            (
                "EB MCP: 2 wijzigingen wachten op je akkoord".to_owned(),
                "Eerste\nTweede".to_owned()
            )
        );
        assert!(seen.fresh(&waiting, 7).is_empty(), "announced once");
        let one = seen.fresh(&[proposal(6, "Eerste"), proposal(8, "Derde")], 8);
        assert_eq!(
            notification_text(&one),
            (
                "EB MCP: 1 wijziging wacht op je akkoord".to_owned(),
                "Derde".to_owned()
            )
        );
        let many: Vec<_> = (10..15).map(|n| proposal(n, &format!("Nr {n}"))).collect();
        assert_eq!(notification_text(&many).1, "Nr 10\nNr 11\nNr 12\nen nog 2");
    }

    #[test]
    fn the_panel_stays_on_the_screen() {
        // A 2560 px wide work area starting at 0, 672 px panel, 16 px margin.
        let clamp = |x| clamp_span(x, 672.0, 0.0, 2560.0, 16.0);
        assert_eq!(clamp(1000.0), 1000.0, "room enough: unchanged");
        assert_eq!(clamp(2300.0), 2560.0 - 16.0 - 672.0, "near the right edge");
        assert_eq!(clamp(-40.0), 16.0, "near the left edge");
        // A second screen to the left of the main one has a negative origin.
        assert_eq!(clamp_span(-100.0, 672.0, -1920.0, 1920.0, 16.0), -688.0);
        // An area narrower than the panel keeps its start.
        assert_eq!(clamp_span(50.0, 672.0, 0.0, 600.0, 16.0), 16.0);
    }
}
