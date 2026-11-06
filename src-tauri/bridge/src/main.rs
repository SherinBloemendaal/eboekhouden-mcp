//! eb-mcp-bridge: lets a client that only starts stdio servers (Claude
//! Desktop) reach EB MCP's local HTTP server. Each JSON-RPC line on stdin goes
//! to the server as one POST; each answer comes back as one line on stdout.
//!
//!     eb-mcp-bridge --url http://127.0.0.1:47821/mcp [--token-file <dev-token.json>]
//!
//! The token never sits in Claude's config: the bridge reads it from the
//! Keychain (Windows: Credential Manager), where EB MCP keeps it. A development
//! build passes its dev token file instead; EB_MCP_TOKEN overrides both (tests).

use std::process::ExitCode;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const PROTOCOL_HEADER: &str = "mcp-protocol-version";
const SESSION_HEADER: &str = "mcp-session-id";

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut url = None;
    let mut token_file = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--url" => url = args.next(),
            "--token-file" => token_file = args.next(),
            _ => {}
        }
    }
    let Some(url) = url.filter(|u| loopback(u)) else {
        eprintln!("eb-mcp-bridge: --url moet een adres op 127.0.0.1 of localhost zijn");
        return ExitCode::from(2);
    };
    let token = match token(token_file.as_deref()) {
        Ok(token) => token,
        Err(e) => {
            eprintln!("eb-mcp-bridge: {e}");
            return ExitCode::from(2);
        }
    };
    let http = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("HTTP client");
    let mut session: Option<String> = None;
    let mut protocol: Option<String> = None;
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();

    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let message: Option<Value> = serde_json::from_str(line).ok();
        let id = message.as_ref().and_then(|m| m.get("id")).cloned();
        let mut request = http
            .post(&url)
            .bearer_auth(&token)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(line.to_owned());
        if let Some(s) = &session {
            request = request.header(SESSION_HEADER, s);
        }
        if let Some(p) = &protocol {
            request = request.header(PROTOCOL_HEADER, p);
        }
        let answers = match request.send().await {
            Err(_) => error_for(
                id,
                "EB MCP draait niet. Start EB MCP en probeer het opnieuw.",
            ),
            Ok(response) => {
                if let Some(s) = response
                    .headers()
                    .get(SESSION_HEADER)
                    .and_then(|v| v.to_str().ok())
                {
                    session = Some(s.to_owned());
                }
                let status = response.status();
                let stream = response
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|c| c.starts_with("text/event-stream"));
                let body = response.text().await.unwrap_or_default();
                if status == 401 {
                    error_for(
                        id,
                        "Het token klopt niet meer. Koppel EB MCP opnieuw met Claude.",
                    )
                } else if !status.is_success() {
                    error_for(id, &format!("EB MCP gaf {status}"))
                } else if stream {
                    events(&body)
                } else if body.trim().is_empty() {
                    Vec::new()
                } else {
                    vec![body.trim().to_owned()]
                }
            }
        };
        for answer in answers {
            if protocol.is_none()
                && let Ok(value) = serde_json::from_str::<Value>(&answer)
                && let Some(v) = value
                    .pointer("/result/protocolVersion")
                    .and_then(Value::as_str)
            {
                protocol = Some(v.to_owned());
            }
            if stdout
                .write_all(format!("{answer}\n").as_bytes())
                .await
                .is_err()
            {
                return ExitCode::SUCCESS;
            }
        }
        let _ = stdout.flush().await;
    }
    ExitCode::SUCCESS
}

/// The MCP token: EB_MCP_TOKEN, else the given dev token file, else the system store.
fn token(token_file: Option<&str>) -> Result<String, String> {
    if let Some(token) = std::env::var("EB_MCP_TOKEN").ok().filter(|t| !t.is_empty()) {
        return Ok(token);
    }
    if let Some(path) = token_file {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("tokenbestand niet te lezen: {e}"))?;
        let map: serde_json::Map<String, Value> =
            serde_json::from_str(&text).map_err(|e| format!("tokenbestand is kapot: {e}"))?;
        return map
            .get("mcp-token")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| "geen token in het tokenbestand; start EB MCP eerst".into());
    }
    keyring::Entry::new("EB MCP", "mcp-token")
        .and_then(|entry| entry.get_password())
        .map_err(|e| format!("geen token in de sleutelhanger ({e}); start EB MCP eerst"))
}

fn loopback(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|u| {
        u.scheme() == "http"
            && u.username().is_empty()
            && u.password().is_none()
            && matches!(u.host_str(), Some("127.0.0.1" | "localhost"))
    })
}

/// A JSON-RPC error for a request (one with an id); notifications get no answer.
fn error_for(id: Option<Value>, message: &str) -> Vec<String> {
    match id {
        Some(id) if !id.is_null() => vec![
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": message}})
                .to_string(),
        ],
        _ => Vec::new(),
    }
}

/// The `data:` payloads of a server-sent event stream, one per event.
fn events(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut data = String::new();
    for line in body.lines().chain(std::iter::once("")) {
        if let Some(rest) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.trim_start());
        } else if line.is_empty() && !data.is_empty() {
            out.push(std::mem::take(&mut data));
        }
    }
    out.into_iter()
        .filter(|d| serde_json::from_str::<Value>(d).is_ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_event_stream_payloads() {
        let body = "id: 1\nevent: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\n: ping\n\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"x\"}\n";
        assert_eq!(events(body).len(), 2);
        assert_eq!(events("data: \n\n"), Vec::<String>::new());
    }

    #[test]
    fn answers_requests_only() {
        assert_eq!(error_for(None, "x"), Vec::<String>::new());
        assert!(error_for(Some(json!(7)), "x")[0].contains("\"id\":7"));
    }

    #[test]
    fn reads_the_dev_token_file() {
        let path = std::env::temp_dir().join(format!("ebmcp-bridge-{}.json", std::process::id()));
        std::fs::write(&path, r#"{"mcp-token": "abc"}"#).unwrap();
        // EB_MCP_TOKEN is not set by the test runner, so the file decides.
        if std::env::var("EB_MCP_TOKEN").is_err() {
            assert_eq!(token(Some(path.to_str().unwrap())).unwrap(), "abc");
        }
        std::fs::write(&path, "{}").unwrap();
        if std::env::var("EB_MCP_TOKEN").is_err() {
            assert!(token(Some(path.to_str().unwrap())).is_err());
        }
    }

    #[test]
    fn only_loopback() {
        assert!(loopback("http://127.0.0.1:47821/mcp"));
        assert!(!loopback("http://evil.example/mcp"));
        assert!(!loopback("https://127.0.0.1.evil.example/"));
        assert!(!loopback("http://127.0.0.1:47821@evil.example/mcp"));
    }
}
