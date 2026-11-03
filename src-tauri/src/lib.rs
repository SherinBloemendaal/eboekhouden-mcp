mod mcp;
mod secrets;
mod state;

use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{
    AppHandle, Manager, PhysicalPosition, WindowEvent,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

use crate::state::AppState;

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
}

struct Server {
    token_tail: String,
    error: Mutex<Option<String>>,
}

#[tauri::command]
fn server_info(
    server: tauri::State<'_, Arc<Server>>,
    app_state: tauri::State<'_, Arc<AppState>>,
) -> ServerInfo {
    let error = server.error.lock().unwrap().clone();
    ServerInfo {
        url: mcp::url(),
        token_tail: server.token_tail.clone(),
        running: error.is_none(),
        error,
        sessions: app_state.session_count(),
    }
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

#[tauri::command]
fn quit(app: AppHandle) {
    // Signing out at e-Boekhouden happens here once sessions exist (M1).
    app.exit(0);
}

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
            .unwrap_or(520.0 * scale);
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
    let app_state = Arc::new(AppState::default());

    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .build(),
        )
        .manage(app_state.clone())
        .invoke_handler(tauri::generate_handler![server_info, open_main, quit])
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let server = match secrets::load_or_create_token(&secrets::SystemStore) {
                Ok(token) => {
                    let server = Arc::new(Server {
                        token_tail: secrets::token_tail(&token).to_owned(),
                        error: Mutex::new(None),
                    });
                    let (state, token, report) =
                        (app_state.clone(), Arc::<str>::from(token), server.clone());
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
                    token_tail: String::new(),
                    error: Mutex::new(Some(format!("Geen token in de sleutelhanger: {e}"))),
                }),
            };
            app.manage(server);

            let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray@2x.png"))?;
            TrayIconBuilder::with_id("eb-mcp")
                .icon(icon)
                .icon_as_template(true)
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
        .run(tauri::generate_context!())
        .expect("EB MCP kon niet starten");
}
