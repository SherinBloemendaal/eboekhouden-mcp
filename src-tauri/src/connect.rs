//! Koppelen: writes the EB MCP entry into the MCP config of Claude Desktop,
//! Cursor or Claude Code, after the user saw exactly what changes. A backup goes next to
//! the file first; a file that is not valid JSON is never touched; after
//! writing, the file is read back to check.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Map, Value, json};

pub const ENTRY: &str = "eb-mcp";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Client {
    Claude,
    Cursor,
    ClaudeCode,
}

/// How Claude Desktop starts EB MCP: the bridge next to the app. Release builds
/// let it read the token from the Keychain; development builds point it at the
/// dev token file (secrets::DevFileStore). Claude's config never holds the token.
pub struct Bridge {
    pub path: PathBuf,
    pub token_file: Option<PathBuf>,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum LinkState {
    /// No config file, or no EB MCP entry in it.
    None,
    Linked,
    /// Linked, but not as EB MCP would write it now (an older token, or the app moved).
    Stale,
    /// The file exists but is not valid JSON.
    Broken,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientStatus {
    pub client: Client,
    pub path: String,
    pub state: LinkState,
    pub error: Option<String>,
}

/// What Koppelen would change, for the confirm sheet. The token is masked.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub client: Client,
    pub path: String,
    pub exists: bool,
    pub other_servers: Vec<String>,
    pub replaces: bool,
    pub entry: Value,
    pub backup: String,
}

pub struct Paths {
    pub home: PathBuf,
    /// %APPDATA% on Windows.
    pub app_data: Option<PathBuf>,
}

impl Paths {
    pub fn system() -> Self {
        Self {
            home: dirs_home(),
            app_data: std::env::var_os("APPDATA").map(PathBuf::from),
        }
    }

    pub fn config(&self, client: Client) -> PathBuf {
        match client {
            Client::Claude if cfg!(target_os = "windows") => self
                .app_data
                .clone()
                .unwrap_or_else(|| self.home.join("AppData").join("Roaming"))
                .join("Claude")
                .join("claude_desktop_config.json"),
            Client::Claude => self
                .home
                .join("Library")
                .join("Application Support")
                .join("Claude")
                .join("claude_desktop_config.json"),
            Client::Cursor => self.home.join(".cursor").join("mcp.json"),
            Client::ClaudeCode => self.home.join(".claude.json"),
        }
    }
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// The entry for a client. Claude Desktop only starts stdio servers, so it gets
/// the bridge, which finds the token itself; Cursor and Claude Code speak HTTP
/// and get the URL with the token header.
pub fn entry(client: Client, url: &str, token: &str, bridge: &Bridge) -> Value {
    match client {
        Client::Claude => {
            let mut args = vec![json!("--url"), json!(url)];
            if let Some(file) = &bridge.token_file {
                args.push(json!("--token-file"));
                args.push(json!(file.to_string_lossy()));
            }
            json!({ "command": bridge.path.to_string_lossy(), "args": args })
        }
        Client::Cursor => json!({
            "url": url,
            "headers": { "Authorization": format!("Bearer {token}") },
        }),
        Client::ClaudeCode => json!({
            "type": "http",
            "url": url,
            "headers": { "Authorization": format!("Bearer {token}") },
        }),
    }
}

pub fn mask(value: &Value, token: &str) -> Value {
    let tail = &token[token.len().saturating_sub(4)..];
    let masked = format!("••••••••••••{tail}");
    match value {
        Value::String(s) if s.contains(token) => Value::String(s.replace(token, &masked)),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), mask(v, token)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(|v| mask(v, token)).collect()),
        other => other.clone(),
    }
}

fn read(path: &Path) -> Result<Option<Value>, String> {
    match fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok(Some(json!({}))),
        Ok(text) => serde_json::from_str::<Value>(&text).map(Some).map_err(|e| {
            format!(
                "{} is geen geldige JSON (regel {}, teken {}): {}",
                file_name(path),
                e.line(),
                e.column(),
                short(&e)
            )
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{} is niet te lezen: {e}", file_name(path))),
    }
}

fn short(e: &serde_json::Error) -> String {
    let text = e.to_string();
    text.split(" at line").next().unwrap_or(&text).to_owned()
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn servers(config: &Value) -> Option<&Map<String, Value>> {
    config.get("mcpServers").and_then(Value::as_object)
}

pub fn backup_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".eb-mcp-backup");
    path.with_file_name(name)
}

/// Whether the entry is the one EB MCP would write now: for Claude the same
/// bridge and arguments (and no token of an older version), for the HTTP
/// clients the current token.
fn is_current(client: Client, entry: &Value, url: &str, token: &str, bridge: &Bridge) -> bool {
    match client {
        Client::Claude => entry == &self::entry(client, url, token, bridge),
        Client::Cursor | Client::ClaudeCode => {
            entry
                .pointer("/headers/Authorization")
                .and_then(Value::as_str)
                == Some(&format!("Bearer {token}"))
        }
    }
}

pub fn status(
    paths: &Paths,
    client: Client,
    url: &str,
    token: &str,
    bridge: &Bridge,
) -> ClientStatus {
    let path = paths.config(client);
    let (state, error) = match read(&path) {
        Err(e) => (LinkState::Broken, Some(e)),
        Ok(None) => (LinkState::None, None),
        Ok(Some(config)) => match servers(&config).and_then(|s| s.get(ENTRY)) {
            None => (LinkState::None, None),
            Some(entry) if is_current(client, entry, url, token, bridge) => {
                (LinkState::Linked, None)
            }
            Some(_) => (LinkState::Stale, None),
        },
    };
    ClientStatus {
        client,
        path: path.to_string_lossy().into_owned(),
        state,
        error,
    }
}

pub fn preview(
    paths: &Paths,
    client: Client,
    url: &str,
    token: &str,
    bridge: &Bridge,
) -> Result<Preview, String> {
    let path = paths.config(client);
    let config = read(&path)?;
    let others: Vec<String> = config
        .as_ref()
        .and_then(servers)
        .map(|s| s.keys().filter(|k| *k != ENTRY).cloned().collect())
        .unwrap_or_default();
    let replaces = config
        .as_ref()
        .and_then(servers)
        .is_some_and(|s| s.contains_key(ENTRY));
    Ok(Preview {
        client,
        path: path.to_string_lossy().into_owned(),
        exists: config.is_some(),
        other_servers: others,
        replaces,
        entry: mask(&entry(client, url, token, bridge), token),
        backup: file_name(&backup_path(&path)),
    })
}

/// Adds or replaces the entry; everything else in the file stays as it was.
pub fn link(
    paths: &Paths,
    client: Client,
    url: &str,
    token: &str,
    bridge: &Bridge,
) -> Result<(), String> {
    let path = paths.config(client);
    let mut config = read(&path)?.unwrap_or_else(|| json!({}));
    let Some(root) = config.as_object_mut() else {
        return Err(format!(
            "{} heeft geen JSON-object bovenaan",
            file_name(&path)
        ));
    };
    let servers = root.entry("mcpServers").or_insert_with(|| json!({}));
    let Some(servers) = servers.as_object_mut() else {
        return Err("mcpServers in het bestand is geen object".into());
    };
    servers.insert(ENTRY.into(), entry(client, url, token, bridge));
    write(&path, &config)?;
    match status(paths, client, url, token, bridge).state {
        LinkState::Linked => Ok(()),
        _ => Err("Het bestand is geschreven, maar EB MCP leest de koppeling niet terug".into()),
    }
}

pub fn unlink(paths: &Paths, client: Client) -> Result<(), String> {
    let path = paths.config(client);
    let Some(mut config) = read(&path)? else {
        return Ok(());
    };
    let removed = config
        .get_mut("mcpServers")
        .and_then(Value::as_object_mut)
        .and_then(|s| s.remove(ENTRY))
        .is_some();
    if removed {
        write(&path, &config)?;
    }
    Ok(())
}

/// Backup first, then write next to the file and rename over it.
fn write(path: &Path, config: &Value) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)
            .map_err(|e| format!("Map {} is niet te maken: {e}", dir.display()))?;
    }
    if path.exists() {
        fs::copy(path, backup_path(path)).map_err(|e| format!("Reservekopie mislukt: {e}"))?;
    }
    let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())? + "\n";
    let tmp = path.with_extension("json.eb-mcp-tmp");
    fs::write(&tmp, text).map_err(|e| format!("Schrijven mislukt: {e}"))?;
    fs::rename(&tmp, path).map_err(|e| format!("Schrijven mislukt: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789ab3f9a";
    const URL: &str = "http://127.0.0.1:47821/mcp";

    fn paths() -> Paths {
        let home = std::env::temp_dir().join(format!("ebmcp-home-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&home).unwrap();
        Paths {
            home,
            app_data: None,
        }
    }

    fn bridge() -> Bridge {
        Bridge {
            path: PathBuf::from("/Applications/EB MCP.app/Contents/MacOS/eb-mcp-bridge"),
            token_file: None,
        }
    }

    #[test]
    fn links_cursor_and_keeps_other_servers() {
        let p = paths();
        let file = p.config(Client::Cursor);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(
            &file,
            r#"{"mcpServers": {"agenda": {"url": "http://x"}}, "andere": 1}"#,
        )
        .unwrap();
        assert_eq!(
            status(&p, Client::Cursor, URL, TOKEN, &bridge()).state,
            LinkState::None
        );
        link(&p, Client::Cursor, URL, TOKEN, &bridge()).unwrap();
        let written: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(written["andere"], 1);
        assert_eq!(written["mcpServers"]["agenda"]["url"], "http://x");
        assert_eq!(
            written["mcpServers"][ENTRY]["headers"]["Authorization"],
            format!("Bearer {TOKEN}")
        );
        assert!(backup_path(&file).exists());
        assert_eq!(
            status(&p, Client::Cursor, URL, TOKEN, &bridge()).state,
            LinkState::Linked
        );
        assert_eq!(
            status(&p, Client::Cursor, URL, "another", &bridge()).state,
            LinkState::Stale
        );
    }

    #[test]
    fn creates_claudes_file_with_the_bridge_and_no_token() {
        let p = paths();
        link(&p, Client::Claude, URL, TOKEN, &bridge()).unwrap();
        let text = fs::read_to_string(p.config(Client::Claude)).unwrap();
        let written: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(written["mcpServers"][ENTRY]["args"], json!(["--url", URL]));
        assert!(
            !text.contains(TOKEN),
            "Claude's config must not hold the token"
        );
        // A token renewal changes nothing for Claude: the bridge reads the current token itself.
        assert_eq!(
            status(&p, Client::Claude, URL, "renewed", &bridge()).state,
            LinkState::Linked
        );
    }

    #[test]
    fn an_older_claude_entry_with_a_token_is_stale() {
        let p = paths();
        let file = p.config(Client::Claude);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        let old = json!({"mcpServers": {ENTRY: {"command": bridge().path, "args": ["--url", URL], "env": {"EB_MCP_TOKEN": TOKEN}}}});
        fs::write(&file, old.to_string()).unwrap();
        assert_eq!(
            status(&p, Client::Claude, URL, TOKEN, &bridge()).state,
            LinkState::Stale
        );
    }

    #[test]
    fn development_builds_point_the_bridge_at_the_token_file() {
        let dev = Bridge {
            path: PathBuf::from("/x/eb-mcp-bridge"),
            token_file: Some(PathBuf::from("/cfg/dev-token.json")),
        };
        assert_eq!(
            entry(Client::Claude, URL, TOKEN, &dev)["args"],
            json!(["--url", URL, "--token-file", "/cfg/dev-token.json"])
        );
    }

    #[test]
    fn links_claude_code_over_http_in_its_own_file() {
        let p = paths();
        let file = p.config(Client::ClaudeCode);
        fs::write(&file, r#"{"numStartups": 12, "projects": {"/x": {}}}"#).unwrap();
        link(&p, Client::ClaudeCode, URL, TOKEN, &bridge()).unwrap();
        let written: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(
            written["numStartups"], 12,
            "Claude Code's own settings stay"
        );
        assert_eq!(written["mcpServers"][ENTRY]["type"], "http");
        assert_eq!(
            written["mcpServers"][ENTRY]["headers"]["Authorization"],
            format!("Bearer {TOKEN}")
        );
        assert!(file.ends_with(".claude.json"));
    }

    #[test]
    fn never_touches_a_broken_file() {
        let p = paths();
        let file = p.config(Client::Cursor);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(
            &file,
            "{\n  \"mcpServers\": {\n    \"a\": 1\n    \"b\": 2\n  }\n}",
        )
        .unwrap();
        let error = link(&p, Client::Cursor, URL, TOKEN, &bridge()).unwrap_err();
        assert!(error.contains("regel 4"), "{error}");
        assert!(fs::read_to_string(&file).unwrap().contains("\"b\": 2"));
        assert_eq!(
            status(&p, Client::Cursor, URL, TOKEN, &bridge()).state,
            LinkState::Broken
        );
    }

    #[test]
    fn preview_masks_the_token() {
        let p = paths();
        let preview = preview(&p, Client::Cursor, URL, TOKEN, &bridge()).unwrap();
        let text = preview.entry.to_string();
        assert!(!text.contains(TOKEN));
        assert!(text.contains("••••••••••••3f9a"));
        assert!(!preview.exists);
    }

    #[test]
    fn unlinks_only_our_entry() {
        let p = paths();
        link(&p, Client::Cursor, URL, TOKEN, &bridge()).unwrap();
        unlink(&p, Client::Cursor).unwrap();
        let written: Value =
            serde_json::from_str(&fs::read_to_string(p.config(Client::Cursor)).unwrap()).unwrap();
        assert_eq!(written["mcpServers"], json!({}));
    }
}
