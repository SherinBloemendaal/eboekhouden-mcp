mod accounts;
mod connect;
mod creds;
mod ebapi;
mod mcp;
mod reqlog;
mod secrets;
mod settings;
mod signin;
mod state;
mod tools;
mod updates;

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
        .map(|account| AccountView {
            session: sessions
                .iter()
                .find(|s| s.account_id == account.id)
                .cloned(),
            signing_in: signins.is_running(account.id),
            account,
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
    let _ = app_state.events.send(state::AppEvent::SessionsChanged);
    Ok(())
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

#[tauri::command]
fn log_clear(app_state: Shared<'_, AppState>) {
    app_state.log.clear();
    let _ = app_state.events.send(state::AppEvent::LogChanged);
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
    quit_now(&app, &app_state).await;
    Ok(())
}

async fn quit_now(app: &AppHandle, app_state: &AppState) {
    app_state.sign_out_all("EB MCP afgesloten");
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
            if state.log.running() > 0 {
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
            open_log_dir,
            open_op_install,
            log_list,
            log_get,
            log_clear,
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
            app.manage(app_state.clone());
            app.manage(settings.clone());
            app.manage(Arc::new(Signins::default()));

            // Forward state changes to every window.
            let mut events = app_state.events.subscribe();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                while let Ok(event) = events.recv().await {
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
            TrayIconBuilder::with_id("eb-mcp")
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
        .run(|app, event| {
            // A click on the Dock icon (when it shows) opens the window again.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen {
                has_visible_windows: false,
                ..
            } = event
            {
                open_main(app.clone(), None);
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

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
