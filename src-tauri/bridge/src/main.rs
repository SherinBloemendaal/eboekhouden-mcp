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

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tokio::task::JoinSet;

const PROTOCOL_HEADER: &str = "mcp-protocol-version";
const SESSION_HEADER: &str = "mcp-session-id";
/// Sent with every request, so EB MCP knows its client gives up on a tool call after about a minute.
const BRIDGE_HEADER: &str = "x-eb-mcp-bridge";
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
    let link = Arc::new(Link { http, url, token });
    let shared = Arc::new(Shared::default());
    // One writer for stdout, so answers of requests that run side by side never mix within a line.
    let (out, mut lines_out) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(line) = lines_out.recv().await {
            if stdout
                .write_all(format!("{line}\n").as_bytes())
                .await
                .is_err()
            {
                break;
            }
            let _ = stdout.flush().await;
        }
    });
    let mut running = JoinSet::new();
    let mut lines = BufReader::new(tokio::io::stdin()).lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim().to_owned();
        if line.is_empty() {
            continue;
        }
        let message: Option<Value> = serde_json::from_str(&line).ok();
        let id = message
            .as_ref()
            .and_then(|m| m.get("id"))
            .filter(|id| !id.is_null())
            .cloned();
        let is_initialize = message
            .as_ref()
            .and_then(|m| m.get("method"))
            .and_then(Value::as_str)
            == Some("initialize");
        let (link, shared, out) = (link.clone(), shared.clone(), out.clone());
        let forward = forward(link, shared, out, line, id.clone(), is_initialize);
        // A request runs on its own: a tool call that waits minutes for the user's approval in EB MCP must not
        // hold up the client's other requests. The initialize and notifications go one by one, in order, so the
        // session exists before anything else is sent on it.
        if id.is_some() && !is_initialize {
            running.spawn(forward);
        } else {
            forward.await;
        }
    }
    // The client closed stdin: what it asked still gets its answer before the bridge stops.
    while running.join_next().await.is_some() {}
    drop(out);
    let _ = writer.await;
    ExitCode::SUCCESS
}

/// What the requests share: the session and protocol, and the client's own initialize to start a new session
/// with when EB MCP no longer knows this one.
#[derive(Default)]
struct Shared {
    state: Mutex<Session>,
    /// Held while a new session is started, so two requests that both hear 404 start only one.
    renewing: tokio::sync::Mutex<()>,
}

#[derive(Clone, Default)]
struct Session {
    id: Option<String>,
    protocol: Option<String>,
    initialize: Option<String>,
}

impl Shared {
    fn get(&self) -> Session {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn update(&self, change: impl FnOnce(&mut Session)) {
        change(&mut self.state.lock().unwrap_or_else(|e| e.into_inner()));
    }
}

/// Sends one line to EB MCP and writes what comes back to `out`, each message as soon as it arrives.
async fn forward(
    link: Arc<Link>,
    shared: Arc<Shared>,
    out: mpsc::UnboundedSender<String>,
    line: String,
    id: Option<Value>,
    is_initialize: bool,
) {
    if is_initialize {
        // A new start: whatever session there was is not sent along.
        shared.update(|s| {
            s.initialize = Some(line.clone());
            s.id = None;
        });
    }
    let before = shared.get();
    let mut sent = link
        .post(&line, before.id.as_deref(), before.protocol.as_deref())
        .await;
    // 404 on a session id: EB MCP forgot the session (it was restarted, by an update for one, or the
    // session sat unused for too long). The MCP rule for a client is to start a new session; the bridge
    // is that client, so it does so with the client's own initialize and sends the request once more.
    if let Ok(answer) = &sent
        && answer.status == 404
        && before.id.is_some()
        && !is_initialize
        && let Some(renewed) = renew(&link, &shared, before.id.as_deref()).await
    {
        let protocol = shared.get().protocol;
        sent = link.post(&line, Some(&renewed), protocol.as_deref()).await;
    }
    let mut answer = match sent {
        Err(()) => {
            send_all(
                &out,
                error_for(
                    id,
                    "EB MCP draait niet. Start EB MCP en probeer het opnieuw.",
                ),
            );
            return;
        }
        Ok(answer) => answer,
    };
    if let Some(s) = answer.session.clone() {
        shared.update(|state| state.id = Some(s));
    }
    if answer.status == 401 {
        send_all(
            &out,
            error_for(
                id,
                "Het token klopt niet meer. Koppel EB MCP opnieuw met Claude.",
            ),
        );
        return;
    }
    if !(200..300).contains(&answer.status) {
        send_all(
            &out,
            error_for(id, &format!("EB MCP gaf {}", answer.status)),
        );
        return;
    }
    let emit = |message: String| {
        if shared.get().protocol.is_none()
            && let Ok(value) = serde_json::from_str::<Value>(&message)
            && let Some(v) = value
                .pointer("/result/protocolVersion")
                .and_then(Value::as_str)
        {
            let v = v.to_owned();
            shared.update(|s| s.protocol = Some(v));
        }
        let _ = out.send(message);
    };
    if answer.stream {
        // An event stream: each event goes to the client as it comes, so progress reaches it while a tool
        // call still waits.
        let mut stream = EventStream::default();
        while let Some(chunk) = answer.chunk().await {
            stream.push(&chunk).into_iter().for_each(&emit);
        }
        stream.finish().into_iter().for_each(&emit);
    } else {
        let body = answer.text().await;
        if !body.trim().is_empty() {
            emit(body.trim().to_owned());
        }
    }
}

/// Starts a new session unless another request did so already since `stale` was sent. The session to use.
async fn renew(link: &Link, shared: &Shared, stale: Option<&str>) -> Option<String> {
    let _one = shared.renewing.lock().await;
    let now = shared.get();
    if now.id.is_some() && now.id.as_deref() != stale {
        return now.id;
    }
    let first = now.initialize?;
    let renewed = link.new_session(&first, now.protocol.as_deref()).await?;
    shared.update(|s| s.id = Some(renewed.clone()));
    Some(renewed)
}

fn send_all(out: &mpsc::UnboundedSender<String>, lines: Vec<String>) {
    for line in lines {
        let _ = out.send(line);
    }
}

/// What EB MCP answered to one POST; the body is read by `chunk` or `text`.
struct Answer {
    status: u16,
    /// The session id the answer named, if any.
    session: Option<String>,
    /// Whether the body is an event stream (else plain JSON, or nothing).
    stream: bool,
    response: reqwest::Response,
}

impl Answer {
    /// The next piece of the body; `None` at its end, or when the connection broke.
    async fn chunk(&mut self) -> Option<Vec<u8>> {
        self.response
            .chunk()
            .await
            .ok()
            .flatten()
            .map(|b| b.to_vec())
    }

    async fn text(self) -> String {
        self.response.text().await.unwrap_or_default()
    }
}

/// The way to EB MCP's HTTP server.
struct Link {
    http: reqwest::Client,
    url: String,
    token: String,
}

impl Link {
    /// One JSON-RPC line as one POST. `Err` when EB MCP cannot be reached at all.
    async fn post(
        &self,
        line: &str,
        session: Option<&str>,
        protocol: Option<&str>,
    ) -> Result<Answer, ()> {
        let mut request = self
            .http
            .post(&self.url)
            .bearer_auth(&self.token)
            // Tells EB MCP the call comes through the bridge, whose client gives up on a tool call after about a
            // minute: a change that waits for the user's approval waits less than that (tools.rs, BRIDGE_WAIT).
            .header(BRIDGE_HEADER, "1")
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(line.to_owned());
        if let Some(s) = session {
            request = request.header(SESSION_HEADER, s);
        }
        if let Some(p) = protocol {
            request = request.header(PROTOCOL_HEADER, p);
        }
        let response = request.send().await.map_err(|_| ())?;
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        let session = header(SESSION_HEADER);
        let stream = header("content-type").is_some_and(|c| c.starts_with("text/event-stream"));
        let status = response.status().as_u16();
        Ok(Answer {
            status,
            session,
            stream,
            response,
        })
    }

    /// Starts a new session with the client's own initialize and tells EB MCP the client is ready, as the
    /// client did the first time. The answers stay here: the client asked for neither. Gives the new session id.
    async fn new_session(&self, initialize: &str, protocol: Option<&str>) -> Option<String> {
        let started = self.post(initialize, None, None).await.ok()?;
        if !(200..300).contains(&started.status) {
            return None;
        }
        let session = started.session?;
        let ready = json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string();
        let told = self.post(&ready, Some(&session), protocol).await.ok()?;
        (200..300).contains(&told.status).then_some(session)
    }
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
#[cfg(test)]
fn events(body: &str) -> Vec<String> {
    let mut stream = EventStream::default();
    let mut out = stream.push(body.as_bytes());
    out.extend(stream.finish());
    out
}

/// Reads a server-sent event stream piece by piece and gives each event's `data:` payload once the event is
/// complete. Only payloads that are JSON are kept.
#[derive(Default)]
struct EventStream {
    /// Bytes after the last line break, not a whole line yet.
    pending: Vec<u8>,
    data: String,
}

impl EventStream {
    fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.pending.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(at) = self.pending.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=at).collect();
            let line = String::from_utf8_lossy(&line);
            self.line(line.trim_end_matches(['\n', '\r']), &mut out);
        }
        out
    }

    /// The end of the stream: a last event without its blank line still counts.
    fn finish(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        let rest = String::from_utf8_lossy(&std::mem::take(&mut self.pending)).into_owned();
        if !rest.is_empty() {
            self.line(rest.trim_end_matches('\r'), &mut out);
        }
        self.line("", &mut out);
        out
    }

    fn line(&mut self, line: &str, out: &mut Vec<String>) {
        if let Some(rest) = line.strip_prefix("data:") {
            if !self.data.is_empty() {
                self.data.push('\n');
            }
            self.data.push_str(rest.trim_start());
        } else if line.is_empty() && !self.data.is_empty() {
            let data = std::mem::take(&mut self.data);
            if serde_json::from_str::<Value>(&data).is_ok() {
                out.push(data);
            }
        }
    }
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
    fn reads_an_event_split_over_pieces_as_soon_as_it_is_whole() {
        let mut stream = EventStream::default();
        assert!(
            stream
                .push(b"event: message\r\ndata: {\"jsonrpc\":")
                .is_empty()
        );
        assert!(
            stream.push(b"\"2.0\",\"id\":1}\r\n").is_empty(),
            "not whole before the blank line"
        );
        assert_eq!(stream.push(b"\r\ndata: {\"a\"").len(), 1);
        assert_eq!(stream.push(b":1}").len(), 0);
        assert_eq!(
            stream.finish(),
            vec!["{\"a\":1}".to_owned()],
            "a last event without its blank line"
        );
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
