//! Runs the real eb-mcp-bridge binary against a small mock MCP server.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use axum::{Router, http::HeaderMap, routing::post};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

#[derive(Default)]
struct Seen {
    auth: Vec<String>,
    session: Vec<String>,
    protocol: Vec<String>,
}

async fn mock(seen: Arc<Mutex<Seen>>) -> String {
    let app = Router::new().route(
        "/mcp",
        post(move |headers: HeaderMap, body: String| {
            let seen = seen.clone();
            async move {
                let get = |k: &str| headers.get(k).and_then(|v| v.to_str().ok()).unwrap_or_default().to_owned();
                {
                    let mut s = seen.lock().unwrap();
                    s.auth.push(get("authorization"));
                    s.session.push(get("mcp-session-id"));
                    s.protocol.push(get("mcp-protocol-version"));
                }
                let message: serde_json::Value = serde_json::from_str(&body).unwrap();
                match message["method"].as_str() {
                    Some("initialize") => (
                        [("content-type", "application/json"), ("mcp-session-id", "s-1")],
                        serde_json::json!({"jsonrpc": "2.0", "id": message["id"], "result": {"protocolVersion": "2025-06-18"}}).to_string(),
                    ),
                    Some("notifications/initialized") => ([("content-type", "application/json"), ("mcp-session-id", "s-1")], String::new()),
                    _ => (
                        [("content-type", "text/event-stream"), ("mcp-session-id", "s-1")],
                        format!("event: message\ndata: {}\n\n", serde_json::json!({"jsonrpc": "2.0", "id": message["id"], "result": {"tools": []}})),
                    ),
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://127.0.0.1:{}/mcp", addr.port())
}

#[tokio::test]
async fn forwards_lines_with_token_session_and_protocol() {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let url = mock(seen.clone()).await;
    let mut child = Command::new(env!("CARGO_BIN_EXE_eb-mcp-bridge"))
        .args(["--url", &url])
        .env("EB_MCP_TOKEN", "t0k3n")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap()).lines();
    stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n")
        .await
        .unwrap();
    let first = out.next_line().await.unwrap().unwrap();
    assert!(first.contains("2025-06-18"), "{first}");
    stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
        .await
        .unwrap();
    stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n")
        .await
        .unwrap();
    let second = out.next_line().await.unwrap().unwrap();
    assert!(
        second.contains("\"id\":2") && second.contains("tools"),
        "{second}"
    );
    drop(stdin);
    child.wait().await.unwrap();

    let seen = seen.lock().unwrap();
    assert!(seen.auth.iter().all(|a| a == "Bearer t0k3n"));
    assert_eq!(seen.session, vec!["", "s-1", "s-1"]);
    assert_eq!(seen.protocol, vec!["", "2025-06-18", "2025-06-18"]);
}

#[tokio::test]
async fn says_so_when_eb_mcp_is_not_running() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_eb-mcp-bridge"))
        .args(["--url", "http://127.0.0.1:9/mcp"])
        .env("EB_MCP_TOKEN", "t")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap()).lines();
    stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/list\"}\n")
        .await
        .unwrap();
    let line = out.next_line().await.unwrap().unwrap();
    assert!(
        line.contains("EB MCP draait niet") && line.contains("\"id\":5"),
        "{line}"
    );
    drop(stdin);
    child.wait().await.unwrap();
}

/// A fresh folder that stands in for the user's home in one test.
fn fake_home(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ebmcp-bridge-home-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Where EB MCP keeps its token under `home`, as the bridge looks for it.
fn default_token_file(home: &Path) -> PathBuf {
    let config = if cfg!(target_os = "macos") {
        home.join("Library").join("Application Support")
    } else if cfg!(windows) {
        home.join("AppData")
    } else {
        home.join(".config")
    };
    config.join("dev.sherin.ebmcp").join("token.json")
}

/// The bridge with `home` as the only place to find a token.
fn bridge_in(home: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_eb-mcp-bridge"));
    command
        .env_remove("EB_MCP_TOKEN")
        .env_remove("XDG_CONFIG_HOME")
        .env("HOME", home)
        .env("APPDATA", home.join("AppData"));
    command
}

/// Sends one initialize through the bridge and returns the Authorization headers the server saw.
async fn auth_seen(mut command: Command, seen: Arc<Mutex<Seen>>) -> Vec<String> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap()).lines();
    stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n")
        .await
        .unwrap();
    let first = out.next_line().await.unwrap().unwrap();
    assert!(first.contains("2025-06-18"), "{first}");
    drop(stdin);
    child.wait().await.unwrap();
    seen.lock().unwrap().auth.clone()
}

#[tokio::test]
async fn reads_the_token_from_eb_mcps_config_folder_by_default() {
    let home = fake_home("default");
    let file = default_token_file(&home);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, r#"{"mcp-token": "uit-bestand"}"#).unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let url = mock(seen.clone()).await;
    let mut command = bridge_in(&home);
    command.args(["--url", &url]);
    assert_eq!(auth_seen(command, seen).await, vec!["Bearer uit-bestand"]);
    std::fs::remove_dir_all(&home).unwrap();
}

#[tokio::test]
async fn a_given_token_file_goes_before_the_default() {
    let home = fake_home("given");
    let file = default_token_file(&home);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, r#"{"mcp-token": "uit-standaard"}"#).unwrap();
    let given = home.join("dev-token.json");
    std::fs::write(&given, r#"{"mcp-token": "uit-optie"}"#).unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let url = mock(seen.clone()).await;
    let mut command = bridge_in(&home);
    command.args(["--url", &url, "--token-file"]).arg(&given);
    assert_eq!(auth_seen(command, seen).await, vec!["Bearer uit-optie"]);
    std::fs::remove_dir_all(&home).unwrap();
}

#[tokio::test]
async fn says_to_start_eb_mcp_when_there_is_no_token() {
    let home = fake_home("none");
    let output = bridge_in(&home)
        .args(["--url", "http://127.0.0.1:9/mcp"])
        .stdin(Stdio::null())
        .output()
        .await
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("geen token gevonden") && stderr.contains("start EB MCP eerst"),
        "{stderr}"
    );
    std::fs::remove_dir_all(&home).unwrap();
}
