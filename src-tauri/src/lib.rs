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
use std::time::Duration;

use serde::Serialize;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, State, WindowEvent,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use uuid::Uuid;

use crate::accounts::{Account, AccountInput, AccountStore, Store};
use crate::ebapi::EbClient;
use crate::settings::{Settings, SettingsStore};
use crate::signin::{Signins, WebviewJar};
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

/// Events go to EB MCP's own windows only, never to the hidden e-Boekhouden webviews.
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
    }
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
async fn op_items() -> Result<Vec<creds::OpItem>, creds::OpListError> {
    creds::op_items().await
}

/// Saving checks the account right away by signing in once, out of sight.
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
    app_state.sign_out(id);
    creds::delete_password(id)?;
    app_state.accounts.remove(id)?;
    let _ = app_state.events.send(state::AppEvent::SessionsChanged);
    Ok(())
}

#[tauri::command]
fn account_set_default(app_state: Shared<'_, AppState>, id: Uuid) -> Result<(), String> {
    app_state.accounts.set_default(id)?;
    let _ = app_state.events.send(state::AppEvent::SessionsChanged);
    Ok(())
}

#[tauri::command]
fn signin_start(app: AppHandle, id: Uuid) {
    start_sign_in(&app, id);
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
    app_state.sign_out(id);
}

#[tauri::command]
fn sign_out_all(app_state: Shared<'_, AppState>) {
    app_state.sign_out_all();
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

/// The bridge Claude Desktop starts. Development builds pass the dev token file,
/// since their token is not in the Keychain (secrets::DevFileStore).
fn bridge(app: &AppHandle) -> connect::Bridge {
    let token_file = cfg!(debug_assertions)
        .then(|| app.path().app_config_dir().ok())
        .flatten()
        .map(|dir| dir.join("dev-token.json"));
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
    let had_sessions = app_state.session_count() > 0;
    app_state.sign_out_all();
    if had_sessions {
        tokio::time::sleep(Duration::from_millis(2500)).await;
    }
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
    let (appearance, dock) = (value.appearance, value.show_in_dock);
    settings.set(value)?;
    apply_appearance(&app, appearance);
    apply_dock(&app, dock);
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

/// Sets EB MCP's own windows to light, dark or the system's choice; the pages follow
/// through prefers-color-scheme. The hidden e-Boekhouden webviews are left alone.
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
    let had_sessions = app_state.session_count() > 0;
    app_state.sign_out_all();
    if had_sessions {
        tokio::time::sleep(Duration::from_millis(2500)).await;
    }
    app.exit(0);
    Ok(())
}

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

fn toggle_panel(app: &AppHandle, rect: tauri::Rect) {
    let Some(panel) = app.get_webview_window(PANEL) else {
        return;
    };
    if panel.is_visible().unwrap_or(false) {
        let _ = panel.hide();
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
    // Centred under the icon on macOS; on Windows the tray sits at the bottom, so open above it.
    let x = pos.x + size.width / 2.0 - width / 2.0;
    let y = if cfg!(target_os = "windows") {
        let height = panel
            .outer_size()
            .map(|s| s.height as f64)
            .unwrap_or(540.0 * scale);
        pos.y - height - 8.0 * scale
    } else {
        pos.y + size.height + 6.0 * scale
    };
    let _ = panel.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
    let _ = panel.show();
    let _ = panel.set_focus();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
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
            account_set_default,
            signin_start,
            signin_cancel,
            signin_code,
            sign_out,
            sign_out_all,
            settings_get,
            settings_set,
            open_eboekhouden,
            op_items,
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
            let accounts = AccountStore::open(&dir).map_err(std::io::Error::other)?;
            let settings = Arc::new(SettingsStore::open(&dir));
            apply_appearance(app.handle(), settings.get().appearance);
            apply_dock(app.handle(), settings.get().show_in_dock);
            if std::env::args().any(|a| a == AUTOSTART_ARG)
                && let Some(main) = app.get_webview_window(MAIN)
            {
                let _ = main.hide();
            }
            let app_state = Arc::new(AppState::new(
                accounts,
                EbClient::production(),
                Arc::new(WebviewJar(app.handle().clone())),
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

            // See secrets::DevFileStore: dev builds would ask for Keychain access at every rebuild.
            #[cfg(debug_assertions)]
            let token_store: Box<dyn secrets::SecretStore> =
                Box::new(secrets::DevFileStore(dir.join("dev-token.json")));
            #[cfg(not(debug_assertions))]
            let token_store: Box<dyn secrets::SecretStore> = Box::new(secrets::SystemStore);
            let server = match secrets::load_or_create_token(token_store.as_ref()) {
                Ok(token) => {
                    let token = Arc::new(RwLock::new(token));
                    let server = Arc::new(Server {
                        token: token.clone(),
                        store: token_store,
                        error: Mutex::new(None),
                    });
                    let (state, report) = (app_state.clone(), server.clone());
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = mcp::serve(state, token).await {
                            log::error!("MCP server stopped: {e}");
                            *report.error.lock().unwrap() =
                                Some(format!("Poort {} is niet vrij: {e}", mcp::PORT));
                        }
                    });
                    server
                }
                Err(e) => Arc::new(Server {
                    token: Arc::new(RwLock::new(String::new())),
                    store: token_store,
                    error: Mutex::new(Some(format!("Geen token in de sleutelhanger: {e}"))),
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

            if let Some(panel) = app.get_webview_window(PANEL) {
                let handle = panel.clone();
                panel.on_window_event(move |event| {
                    if let WindowEvent::Focused(false) = event {
                        let _ = handle.hide();
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
