//! Runs the real eb-mcp-bridge binary against a small mock MCP server.

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
