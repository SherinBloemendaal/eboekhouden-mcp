//! eb-mcp-bridge: lets a client that only starts stdio servers (Claude
//! Desktop) reach EB MCP's local HTTP server. Each JSON-RPC line on stdin goes
//! to the server as one POST; each answer comes back as one line on stdout.
//!
//!     eb-mcp-bridge --url http://127.0.0.1:47821/mcp [--token-file <dev-token.json>]
//!
//! The token never sits in Claude's config: the bridge reads it from the token
//! file EB MCP keeps in its config folder (macOS: ~/Library/Application
//! Support/dev.sherin.ebmcp/token.json), readable by the user only. No Keychain,
//! so no password prompt. A development build passes its dev token file with
//! --token-file; EB_MCP_TOKEN overrides both (tests).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const PROTOCOL_HEADER: &str = "mcp-protocol-version";
const SESSION_HEADER: &str = "mcp-session-id";
/// EB MCP's bundle identifier, the name of its config folder.
const APP_ID: &str = "dev.sherin.ebmcp";

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
    let token = match token(
        std::env::var("EB_MCP_TOKEN").ok(),
        token_file.as_deref().map(Path::new),
        default_token_file(std::env::consts::OS, |key| std::env::var_os(key)),
    ) {
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

/// The MCP token: EB_MCP_TOKEN, else the given token file, else EB MCP's own
/// token file in its config folder.
fn token(
    env_token: Option<String>,
    token_file: Option<&Path>,
    default_file: Option<PathBuf>,
) -> Result<String, String> {
    if let Some(token) = env_token.filter(|t| !t.is_empty()) {
        return Ok(token);
    }
    match token_file.map(Path::to_path_buf).or(default_file) {
        Some(path) => read_token_file(&path),
        None => Err("geen token gevonden: de map van EB MCP is onbekend (HOME of APPDATA ontbreekt); start EB MCP eerst".into()),
    }
}

fn read_token_file(path: &Path) -> Result<String, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!(
                "geen token gevonden in {}; start EB MCP eerst",
                path.display()
            ));
        }
        Err(e) => {
            return Err(format!(
                "tokenbestand {} niet te lezen: {e}",
                path.display()
            ));
        }
    };
    let map: serde_json::Map<String, Value> =
        serde_json::from_str(&text).map_err(|e| format!("tokenbestand is kapot: {e}"))?;
    map.get("mcp-token")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "geen token in het tokenbestand; start EB MCP eerst".into())
}

/// Where EB MCP keeps its token file: its config folder, as Tauri's
/// `app_config_dir()` gives it, plus `token.json`. Pure, so tests can feed it
/// any OS and environment. None when the needed variable is missing.
fn default_token_file(os: &str, env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let var = |key: &str| env(key).filter(|v| !v.is_empty()).map(PathBuf::from);
    let config = match os {
        "macos" => var("HOME")?.join("Library").join("Application Support"),
        "windows" => var("APPDATA")?,
        _ => var("XDG_CONFIG_HOME")
            .filter(|p| p.is_absolute())
            .or_else(|| var("HOME").map(|home| home.join(".config")))?,
    };
    Some(config.join(APP_ID).join("token.json"))
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

    fn temp_file(name: &str, content: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("ebmcp-bridge-{}-{name}.json", std::process::id()));
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn reads_a_token_file() {
        let path = temp_file("read", r#"{"mcp-token": "abc"}"#);
        assert_eq!(token(None, Some(path.as_path()), None).unwrap(), "abc");
        std::fs::write(&path, "{}").unwrap();
        assert!(token(None, Some(path.as_path()), None).is_err());
        std::fs::write(&path, r#"{"mcp-token": ""}"#).unwrap();
        assert!(token(None, Some(path.as_path()), None).is_err());
        std::fs::write(&path, "kapot").unwrap();
        assert!(
            token(None, Some(path.as_path()), None)
                .unwrap_err()
                .contains("kapot")
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn takes_the_token_in_order() {
        let given = temp_file("given", r#"{"mcp-token": "uit-optie"}"#);
        let default = temp_file("default", r#"{"mcp-token": "uit-standaard"}"#);
        assert_eq!(
            token(
                Some("uit-env".into()),
                Some(given.as_path()),
                Some(default.clone())
            )
            .unwrap(),
            "uit-env"
        );
        assert_eq!(
            token(
                Some(String::new()),
                Some(given.as_path()),
                Some(default.clone())
            )
            .unwrap(),
            "uit-optie"
        );
        assert_eq!(
            token(None, None, Some(default.clone())).unwrap(),
            "uit-standaard"
        );
        std::fs::remove_file(&given).unwrap();
        std::fs::remove_file(&default).unwrap();
    }

    #[test]
    fn says_to_start_eb_mcp_when_there_is_no_token() {
        let missing = std::env::temp_dir().join("ebmcp-bridge-bestaat-niet/token.json");
        let error = token(None, None, Some(missing)).unwrap_err();
        assert!(error.contains("geen token gevonden"), "{error}");
        assert!(error.contains("start EB MCP eerst"), "{error}");
        let error = token(None, None, None).unwrap_err();
        assert!(error.contains("start EB MCP eerst"), "{error}");
    }

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |key| {
            vars.iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| OsString::from(v))
        }
    }

    #[test]
    fn finds_the_default_token_file_per_os() {
        // XDG_CONFIG_HOME must be absolute on the host that runs the test.
        let xdg = if cfg!(windows) {
            "C:\\cfg"
        } else {
            "/home/jan/cfg"
        };
        let vars = [
            ("HOME", "/Users/jan"),
            ("APPDATA", "C:\\Users\\jan\\AppData\\Roaming"),
            ("XDG_CONFIG_HOME", xdg),
        ];
        assert_eq!(
            default_token_file("macos", env(&vars)),
            Some(PathBuf::from(
                "/Users/jan/Library/Application Support/dev.sherin.ebmcp/token.json"
            ))
        );
        assert_eq!(
            default_token_file("windows", env(&vars)),
            Some(
                PathBuf::from("C:\\Users\\jan\\AppData\\Roaming")
                    .join("dev.sherin.ebmcp")
                    .join("token.json")
            )
        );
        assert_eq!(
            default_token_file("linux", env(&vars)),
            Some(
                PathBuf::from(xdg)
                    .join("dev.sherin.ebmcp")
                    .join("token.json")
            )
        );
    }

    #[test]
    fn falls_back_to_home_config_without_a_usable_xdg_dir() {
        let home = [("HOME", "/home/jan")];
        let expected = Some(PathBuf::from(
            "/home/jan/.config/dev.sherin.ebmcp/token.json",
        ));
        assert_eq!(default_token_file("linux", env(&home)), expected);
        let empty = [("HOME", "/home/jan"), ("XDG_CONFIG_HOME", "")];
        assert_eq!(default_token_file("freebsd", env(&empty)), expected);
        let relative = [("HOME", "/home/jan"), ("XDG_CONFIG_HOME", "cfg")];
        assert_eq!(default_token_file("linux", env(&relative)), expected);
    }

    #[test]
    fn has_no_default_without_the_needed_variable() {
        assert_eq!(default_token_file("macos", env(&[])), None);
        assert_eq!(default_token_file("macos", env(&[("HOME", "")])), None);
        assert_eq!(default_token_file("windows", env(&[("HOME", "/x")])), None);
        assert_eq!(default_token_file("linux", env(&[])), None);
    }

    #[test]
    fn only_loopback() {
        assert!(loopback("http://127.0.0.1:47821/mcp"));
        assert!(!loopback("http://evil.example/mcp"));
        assert!(!loopback("https://127.0.0.1.evil.example/"));
        assert!(!loopback("http://127.0.0.1:47821@evil.example/mcp"));
    }
}
