//! The request log on disk and the debug export, both gzip files only the user can read.
//!
//! The log file (`log_file`) holds one JSON object, `reqlog::Snapshot`: a format number, the app
//! version that wrote it, the entries oldest first with their (already masked) bodies, and the
//! statistics. It is written to a temp file next to it and renamed over it, so a reader never sees
//! half a file.
//!
//! The debug export (`export`) is JSON Lines, readable with `gunzip -c` and grep: a header line,
//! then one line per sign-in trace, one with the statistics and one per log entry, oldest first.
//! It names accounts by label only: no usernames, 1Password references, tokens or cookies.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::accounts::{Store, WriteMode};
use crate::reqlog::{self, Entry, MASK, RequestLog, Snapshot};
use crate::signin::Trace;

/// Where the request log lives in the app's config folder. Development builds keep their own
/// file, as `secrets::token_file` does, so a `tauri dev` run never touches the installed app's log.
pub fn log_file(dir: &Path) -> PathBuf {
    dir.join(if cfg!(debug_assertions) {
        "dev-request-log.json.gz"
    } else {
        "request-log.json.gz"
    })
}

/// The saved log, `None` when there is no file yet. An error for a file that is damaged or has
/// another format; the caller starts empty then.
pub fn read(path: &Path) -> Result<Option<Snapshot>, String> {
    let snapshot: Option<Snapshot> = read_versioned(path, reqlog::FORMAT)?;
    debug_assert!(
        snapshot
            .as_ref()
            .is_none_or(|s| s.format() == reqlog::FORMAT)
    );
    Ok(snapshot)
}

/// A gzip file holding one JSON object with a `format` number, `None` when there is no file yet.
/// An error for a file that is damaged or has another format than `format`.
pub(crate) fn read_versioned<T: DeserializeOwned>(
    path: &Path,
    format: u32,
) -> Result<Option<T>, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let mut json = Vec::new();
    GzDecoder::new(BufReader::new(file))
        .read_to_end(&mut json)
        .map_err(|e| format!("not a readable gzip file: {e}"))?;
    /// Only the format number, read first so a file of another format is not parsed as this one.
    #[derive(Deserialize)]
    struct Header {
        format: Option<u32>,
    }
    let header: Header = serde_json::from_slice(&json).map_err(|e| format!("damaged JSON: {e}"))?;
    match header.format {
        Some(found) if found == format => {}
        Some(other) => {
            return Err(format!(
                "format {other}, this version reads format {format}"
            ));
        }
        None => return Err("no format number".into()),
    }
    serde_json::from_slice(&json)
        .map(Some)
        .map_err(|e| format!("damaged JSON: {e}"))
}

/// Sets an unreadable file aside as `<name>.damaged` instead of overwriting it: it may still be worth a look.
pub(crate) fn set_aside(file: &Path) -> PathBuf {
    let aside = file.with_extension("gz.damaged");
    let _ = fs::rename(file, &aside);
    aside
}

/// Saves the log, compressed, atomically and readable by the user only.
pub fn write(path: &Path, snapshot: &Snapshot) -> Result<(), String> {
    write_gz(path, snapshot)
}

/// Saves any value as gzipped JSON, atomically and readable by the user only.
pub(crate) fn write_gz<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<(), String> {
    let bytes = gzip_json(value)?;
    write_private(path, &bytes).map_err(|e| format!("{}: {e}", path.display()))
}

fn gzip_json<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, String> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    serde_json::to_writer(&mut encoder, value).map_err(|e| e.to_string())?;
    encoder.finish().map_err(|e| e.to_string())
}

/// Writes `bytes` to a temp file next to `path`, private from the moment it exists (mode 0600 on
/// unix; on Windows the user's folder ACL protects it), and renames it over `path`.
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other(format!("no file name in {}", path.display())))?
        .to_string_lossy();
    let temp = dir.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4()));
    let written = create_private(&temp, bytes).and_then(|()| fs::rename(&temp, path));
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written
}

fn create_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    #[cfg(unix)]
    {
        // The mode above passes through the umask; make sure it is exact.
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(bytes)?;
    file.sync_all()
}

/// An account as the debug export names it: no username, no 1Password reference.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportAccount {
    pub label: String,
    pub administration: Option<String>,
    pub store: Store,
    pub write: WriteMode,
    pub signed_in: bool,
}

/// What goes into a debug export besides the log itself.
pub struct ExportInput<'a> {
    pub accounts: &'a [ExportAccount],
    pub traces: &'a [Trace],
    /// Usernames and 1Password references of the accounts: masked wherever they would show up.
    pub hidden: &'a [String],
    pub exported_at: DateTime<Local>,
}

/// The format of the debug export's lines.
const EXPORT_FORMAT: u32 = 1;
/// Shorter values are not masked: masking "ab" would mask half the file.
const HIDDEN_MIN_LEN: usize = 4;

/// The export's lines: header, sign-in traces, statistics, then the entries oldest first.
pub fn export_lines(log: &RequestLog, input: &ExportInput<'_>) -> Result<Vec<String>, String> {
    let entries = log.export_entries();
    let mcp = entries
        .iter()
        .filter(|e| matches!(e, Entry::Mcp(_)))
        .count();
    let header = json!({
        "type": "header",
        "exportFormat": EXPORT_FORMAT,
        "app": "EB MCP",
        "appVersion": reqlog::APP_VERSION,
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "exportedAt": input.exported_at.timestamp_millis(),
        "exportedAtLocal": input.exported_at.format("%Y-%m-%d %H:%M:%S %:z").to_string(),
        "counts": {
            "entries": entries.len(),
            "mcpEntries": mcp,
            "ebEntries": entries.len() - mcp,
            "signinTraces": input.traces.len(),
            "accounts": input.accounts.len(),
        },
        "accounts": input.accounts,
    });
    let mut lines = vec![header];
    for trace in input.traces {
        lines.push(tagged("signinTrace", trace)?);
    }
    lines.push(tagged("stats", &log.stats())?);
    for entry in &entries {
        lines.push(tagged("entry", entry)?);
    }
    masked_lines(&lines, input.hidden)
}

/// Each JSON value as one line of text, with every value in `hidden` (usernames, 1Password
/// references) replaced by the mask wherever it shows up.
pub(crate) fn masked_lines(lines: &[Value], hidden: &[String]) -> Result<Vec<String>, String> {
    let hidden: Vec<String> = hidden
        .iter()
        .map(|h| h.trim())
        .filter(|h| h.chars().count() >= HIDDEN_MIN_LEN)
        .filter_map(|h| {
            // The value as it appears inside a JSON string.
            serde_json::to_string(h)
                .ok()
                .map(|s| s[1..s.len() - 1].to_owned())
        })
        .collect();
    lines
        .iter()
        .map(|line| {
            let mut text = serde_json::to_string(line).map_err(|e| e.to_string())?;
            for value in &hidden {
                text = text.replace(value.as_str(), MASK);
            }
            Ok(text)
        })
        .collect()
}

/// A JSON object with `"type": kind` added.
pub(crate) fn tagged<T: Serialize>(kind: &str, value: &T) -> Result<Value, String> {
    let mut value = serde_json::to_value(value).map_err(|e| e.to_string())?;
    match &mut value {
        Value::Object(map) => {
            map.insert("type".into(), Value::String(kind.into()));
            Ok(value)
        }
        _ => Err(format!("{kind} is not a JSON object")),
    }
}

/// Writes the debug export into `dir` as `eb-mcp-debug-<YYYYMMDD-HHMMSS>.jsonl.gz` (with `-2`,
/// `-3` and so on when that name is taken) and returns its path.
pub fn export(dir: &Path, log: &RequestLog, input: &ExportInput<'_>) -> Result<PathBuf, String> {
    let lines = export_lines(log, input)?;
    write_export(dir, DEBUG_PREFIX, &input.exported_at, &lines)
}

/// The name every debug export starts with.
const DEBUG_PREFIX: &str = "eb-mcp-debug-";
/// The name every export of the approval history starts with (approvals.rs).
pub(crate) const APPROVALS_PREFIX: &str = "eb-mcp-akkoord-";

/// Writes `lines` as one gzipped JSON Lines file `<prefix><YYYYMMDD-HHMMSS>.jsonl.gz` into `dir`
/// (with `-2`, `-3` and so on when that name is taken), readable by the user only, and returns its path.
pub(crate) fn write_export(
    dir: &Path,
    prefix: &str,
    exported_at: &DateTime<Local>,
    lines: &[String],
) -> Result<PathBuf, String> {
    let mut text = lines.join("\n");
    text.push('\n');
    let bytes = gzip_json_lines(&text)?;
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let stamp = exported_at.format("%Y%m%d-%H%M%S");
    for n in 1..=99 {
        let name = if n == 1 {
            format!("{prefix}{stamp}.jsonl.gz")
        } else {
            format!("{prefix}{stamp}-{n}.jsonl.gz")
        };
        let path = dir.join(name);
        if path.exists() {
            continue;
        }
        write_private(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        return Ok(path);
    }
    Err(format!(
        "{}: no free name for {prefix}{stamp}",
        dir.display()
    ))
}

fn gzip_json_lines(text: &str) -> Result<Vec<u8>, String> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(text.as_bytes())
        .map_err(|e| e.to_string())?;
    encoder.finish().map_err(|e| e.to_string())
}

/// Whether `path` is a debug export or an export of the approval history: the only files
/// `log_export_reveal` shows.
pub fn is_export(path: &Path) -> bool {
    path.is_absolute()
        && path.is_file()
        && path.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
            (n.starts_with(DEBUG_PREFIX) || n.starts_with(APPROVALS_PREFIX))
                && n.ends_with(".jsonl.gz")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ebapi::ApiCall;
    use crate::reqlog::{Outcome, RESTARTED};
    use crate::signin::TraceLine;
    use uuid::Uuid;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("ebmcp-log-{}", Uuid::new_v4()));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn read_error(path: &Path) -> String {
        match read(path) {
            Err(e) => e,
            Ok(_) => panic!("{} was read", path.display()),
        }
    }

    fn call(path: &str, status: u16) -> ApiCall {
        ApiCall {
            method: "GET",
            path: path.into(),
            query: vec![("q".into(), "bakker".into())],
            status,
            duration_ms: 120,
            bytes: 100,
            rows: Some(1),
            json: Some(json!({"items": [{"naam": "Bakker"}]})),
            error: (status >= 400).then(|| "fout".to_owned()),
        }
    }

    fn gunzip(path: &Path) -> String {
        let mut text = String::new();
        GzDecoder::new(File::open(path).unwrap())
            .read_to_string(&mut text)
            .unwrap();
        text
    }

    fn gzip_file(path: &Path, text: &str) {
        fs::write(path, gzip_json_lines(text).unwrap()).unwrap();
    }

    /// A log with an MCP call, the e-Boekhouden call it caused and a sign-in call.
    fn filled(log: &RequestLog) -> (u64, u64) {
        let mcp = log.start_mcp("Claude", "relaties", json!({"zoek": "bakker"}), None);
        let eb = log.add_eb(
            &call("relatie/48211", 200),
            Some(mcp),
            "Claude",
            Some((Uuid::nil(), "Studio Noord".into())),
            true,
        );
        log.finish_mcp(mcp, Outcome::Ok, Some(&json!({"aantal": 1})), None);
        log.add_signin(
            &call("auth", 200),
            "EB MCP (inloggen)",
            (Uuid::nil(), "Studio Noord".into()),
        );
        (mcp, eb)
    }

    #[test]
    fn the_file_name_follows_the_build() {
        let dir = Path::new("config");
        let expected = if cfg!(debug_assertions) {
            "dev-request-log.json.gz"
        } else {
            "request-log.json.gz"
        };
        assert_eq!(log_file(dir), dir.join(expected));
    }

    #[test]
    fn the_log_and_statistics_survive_a_restart() {
        let dir = TempDir::new();
        let file = log_file(&dir.0);
        let log = RequestLog::default();
        log.open(file.clone());
        let (mcp, eb) = filled(&log);
        assert!(log.dirty());
        assert!(log.save().unwrap());
        assert!(!log.dirty());
        assert!(!log.save().unwrap(), "nothing changed, nothing written");
        let before = serde_json::to_value(log.stats()).unwrap();

        let again = RequestLog::default();
        again.open(file.clone());
        assert_eq!(again.len(), 3);
        assert!(!again.dirty());
        let Some(Entry::Mcp(m)) = again.get(mcp) else {
            panic!()
        };
        assert_eq!(m.calls, vec![eb]);
        assert_eq!(m.version, reqlog::APP_VERSION);
        assert_eq!(m.result, Some(json!({"aantal": 1})));
        let Some(Entry::Eb(e)) = again.get(eb) else {
            panic!()
        };
        assert_eq!(e.method, "GET");
        assert_eq!(e.version, reqlog::APP_VERSION);
        assert_eq!(e.response, Some(json!({"items": [{"naam": "Bakker"}]})));
        assert_eq!(serde_json::to_value(again.stats()).unwrap(), before);
        // New ids follow the old ones.
        let next = again.start_mcp("Claude", "status", json!({}), None);
        assert!(next > eb);
        // The statistics keep counting from where they were.
        again.finish_mcp(next, Outcome::Ok, None, None);
        assert_eq!(again.stats().mcp, 2);
        assert_eq!(again.stats().started_at, log.stats().started_at);
    }

    #[test]
    fn a_missing_or_damaged_file_starts_empty() {
        let dir = TempDir::new();
        let file = log_file(&dir.0);
        assert!(read(&file).unwrap().is_none());

        fs::write(&file, b"geen gzip").unwrap();
        assert!(read(&file).is_err());
        let log = RequestLog::default();
        log.open(file.clone());
        assert_eq!(log.len(), 0);

        gzip_file(&file, "{\"format\": 1, \"entries\": [kapot");
        assert!(read(&file).is_err());

        // A cut-off gzip stream.
        let bytes = gzip_json_lines("{\"format\": 1, \"entries\": []}").unwrap();
        fs::write(&file, &bytes[..bytes.len() / 2]).unwrap();
        assert!(read(&file).is_err());

        // The damaged file is replaced at the next save.
        log.add_eb(&call("relatie/1", 200), None, "Claude", None, true);
        assert!(log.save().unwrap());
        assert_eq!(read(&file).unwrap().unwrap().format(), reqlog::FORMAT);
    }

    #[test]
    fn a_file_of_another_format_is_not_read() {
        let dir = TempDir::new();
        let file = log_file(&dir.0);
        gzip_file(&file, "{\"entries\": [], \"stats\": {}}");
        assert!(read_error(&file).contains("no format"));
        gzip_file(&file, "{\"format\": 0, \"entries\": []}");
        assert!(read_error(&file).contains("format 0"));
        gzip_file(&file, "{\"format\": 2, \"whatever\": true}");
        assert!(read_error(&file).contains("format 2"));
        let log = RequestLog::default();
        log.open(file);
        assert_eq!(log.len(), 0);
        assert_eq!(log.stats().mcp, 0);
    }

    #[test]
    fn entries_saved_without_a_version_get_an_empty_one() {
        let dir = TempDir::new();
        let file = log_file(&dir.0);
        let old = json!({
            "format": 1,
            "appVersion": "0.1.0",
            "savedAt": 1,
            "next": 3,
            "entries": [
                {"kind": "mcp", "id": 1, "at": 1000, "client": "Claude", "tool": "status",
                 "args": {}, "account": null, "outcome": "ok", "durationMs": 5, "bytes": 2,
                 "error": null, "calls": [2]},
                {"kind": "eb", "id": 2, "at": 1001, "parent": 1, "client": "Claude",
                 "account": "SN", "method": "POST", "path": "auth", "query": [], "status": 200,
                 "durationMs": 4, "bytes": 10, "rows": null, "error": null,
                 "headers": [["Cookie", MASK]], "truncated": false, "signin": false}
            ],
            "stats": {"startedAt": 1000, "mcp": 1}
        });
        gzip_file(&file, &old.to_string());
        let log = RequestLog::default();
        log.open(file);
        assert_eq!(log.len(), 2);
        let Some(Entry::Mcp(m)) = log.get(1) else {
            panic!()
        };
        assert_eq!(m.version, "");
        assert_eq!(m.result, None);
        let Some(Entry::Eb(e)) = log.get(2) else {
            panic!()
        };
        assert_eq!(e.version, "");
        assert_eq!(e.method, "POST");
        let listed = serde_json::to_value(log.list()).unwrap();
        assert_eq!(listed[0]["version"], "");
        assert_eq!(log.stats().started_at, 1000);
        assert_eq!(log.stats().mcp, 1);
        assert!(log.start_mcp("Claude", "status", json!({}), None) >= 3);
    }

    #[test]
    fn a_call_running_at_the_stop_comes_back_as_an_error() {
        let dir = TempDir::new();
        let file = log_file(&dir.0);
        let log = RequestLog::default();
        log.open(file.clone());
        let running = log.start_mcp("Cursor", "mutaties", json!({}), None);
        assert_eq!(log.running(), 1);
        log.save().unwrap();

        let again = RequestLog::default();
        again.open(file.clone());
        assert_eq!(again.running(), 0);
        let Some(Entry::Mcp(m)) = again.get(running) else {
            panic!()
        };
        assert_eq!(m.outcome, Outcome::Error);
        assert_eq!(m.error.as_deref(), Some(RESTARTED));
        let stats = again.stats();
        assert_eq!((stats.mcp, stats.errors), (1, 1));
        assert!(again.dirty(), "the error is written back at the next save");
        again.save().unwrap();

        // Loaded once more, it is not counted twice.
        let third = RequestLog::default();
        third.open(file);
        assert_eq!((third.stats().mcp, third.stats().errors), (1, 1));
    }

    #[test]
    fn a_change_waiting_at_the_stop_comes_back_refused() {
        let dir = TempDir::new();
        let file = log_file(&dir.0);
        let log = RequestLog::default();
        log.open(file.clone());
        let waiting = log.start_mcp("Claude", "mutatie_toevoegen", json!({}), None);
        log.link_proposal(waiting, 3);
        log.wait_mcp(waiting);
        assert_eq!(log.running(), 0);
        log.save().unwrap();

        let again = RequestLog::default();
        again.open(file);
        let Some(Entry::Mcp(m)) = again.get(waiting) else {
            panic!()
        };
        assert_eq!(m.outcome, Outcome::Refused);
        assert_eq!(m.error.as_deref(), Some(reqlog::LAPSED));
        assert_eq!(m.proposal, Some(3));
        assert_eq!((again.stats().mcp, again.stats().errors), (1, 1));
    }

    #[test]
    fn clearing_clears_the_file_but_not_the_statistics() {
        let dir = TempDir::new();
        let file = log_file(&dir.0);
        let log = RequestLog::default();
        log.open(file.clone());
        filled(&log);
        log.save().unwrap();
        log.clear();
        assert!(!log.dirty(), "clearing saves right away");

        let again = RequestLog::default();
        again.open(file);
        assert_eq!(again.len(), 0);
        let stats = again.stats();
        assert_eq!((stats.mcp, stats.eb), (1, 1));
    }

    #[test]
    fn a_log_without_a_file_never_writes() {
        let log = RequestLog::default();
        filled(&log);
        assert!(!log.save().unwrap());
        log.clear();
        assert_eq!(log.len(), 0);
    }

    #[test]
    fn the_saved_file_holds_masked_headers_only() {
        let dir = TempDir::new();
        let file = log_file(&dir.0);
        let log = RequestLog::default();
        log.open(file.clone());
        filled(&log);
        log.save().unwrap();
        let text = gunzip(&file);
        let saved: Value = serde_json::from_str(&text).unwrap();
        let mut masked = 0;
        for entry in saved["entries"].as_array().unwrap() {
            for header in entry["headers"].as_array().into_iter().flatten() {
                let (name, value) = (header[0].as_str().unwrap(), header[1].as_str().unwrap());
                if ["Cookie", "Authorization"].contains(&name) {
                    assert_eq!(value, MASK, "{name} is stored masked");
                    masked += 1;
                }
            }
        }
        assert!(masked >= 2);
        for secret in [
            "auth-token",
            "Bearer ",
            "password",
            "wachtwoord",
            "Set-Cookie",
        ] {
            assert!(!text.contains(secret), "{secret} is not in the file");
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_saved_file_is_private_and_leaves_no_temp_files() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new();
        let file = log_file(&dir.0);
        fs::write(&file, b"oud").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        let log = RequestLog::default();
        log.open(file.clone());
        filled(&log);
        log.save().unwrap();
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let mut names: Vec<String> = fs::read_dir(&dir.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        // The unreadable file from before is set aside, not lost; no temp file stays behind.
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(names, vec![name.clone(), format!("{name}.damaged")]);
        assert_eq!(
            fs::read(dir.0.join(format!("{name}.damaged"))).unwrap(),
            b"oud"
        );
    }

    fn export_input<'a>(
        accounts: &'a [ExportAccount],
        traces: &'a [Trace],
        hidden: &'a [String],
    ) -> ExportInput<'a> {
        ExportInput {
            accounts,
            traces,
            hidden,
            exported_at: DateTime::parse_from_rfc3339("2026-11-11T14:03:22+01:00")
                .unwrap()
                .with_timezone(&Local),
        }
    }

    #[test]
    fn the_export_holds_header_traces_stats_and_entries() {
        let dir = TempDir::new();
        let log = RequestLog::default();
        let (mcp, eb) = filled(&log);
        let accounts = [ExportAccount {
            label: "Studio Noord".into(),
            administration: Some("Studio Noord B.V.".into()),
            store: Store::OnePassword,
            write: WriteMode::Ask,
            signed_in: true,
        }];
        let traces = [Trace {
            account_id: Uuid::nil(),
            label: "Studio Noord".into(),
            started_at: 1,
            outcome: Some("ingelogd".into()),
            lines: vec![TraceLine {
                ms: 3,
                text: "Gegevens lezen voor jan.devries@example.nl".into(),
            }],
        }];
        let hidden = [
            "jan.devries@example.nl".to_owned(),
            "op://Prive/e-Boekhouden".to_owned(),
        ];
        let input = export_input(&accounts, &traces, &hidden);
        let path = export(&dir.0, &log, &input).unwrap();
        assert!(path.is_absolute());
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        assert!(name.starts_with("eb-mcp-debug-"));
        assert!(name.ends_with(".jsonl.gz"));
        assert!(is_export(&path));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        let text = gunzip(&path);
        let lines: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 1 + 1 + 1 + 3);
        let header = &lines[0];
        assert_eq!(header["type"], "header");
        assert_eq!(header["appVersion"], reqlog::APP_VERSION);
        assert_eq!(header["os"], std::env::consts::OS);
        assert_eq!(header["arch"], std::env::consts::ARCH);
        assert_eq!(header["counts"]["entries"], 3);
        assert_eq!(header["counts"]["mcpEntries"], 1);
        assert_eq!(header["counts"]["ebEntries"], 2);
        assert_eq!(header["counts"]["signinTraces"], 1);
        let account = &header["accounts"][0];
        assert_eq!(account["label"], "Studio Noord");
        assert_eq!(account["administration"], "Studio Noord B.V.");
        assert_eq!(account["store"], "onePassword");
        assert_eq!(account["write"], "ask");
        assert_eq!(account["signedIn"], true);
        assert_eq!(
            account.as_object().unwrap().len(),
            5,
            "label, administration, store, write and signedIn only"
        );
        assert_eq!(lines[1]["type"], "signinTrace");
        assert_eq!(lines[2]["type"], "stats");
        assert_eq!(lines[2]["mcp"], 1);
        // Entries oldest first, with their bodies.
        let ids: Vec<u64> = lines[3..]
            .iter()
            .map(|l| {
                assert_eq!(l["type"], "entry");
                l["id"].as_u64().unwrap()
            })
            .collect();
        assert_eq!(ids, vec![mcp, eb, eb + 1]);
        assert_eq!(lines[3]["kind"], "mcp");
        assert_eq!(lines[3]["result"], json!({"aantal": 1}));
        assert_eq!(lines[4]["response"], json!({"items": [{"naam": "Bakker"}]}));
        assert_eq!(lines[4]["version"], reqlog::APP_VERSION);

        // No username, 1Password reference, token or cookie anywhere.
        assert!(!text.contains("jan.devries"));
        assert!(!text.contains("op://"));
        assert!(text.contains(MASK));
        for secret in ["auth-token", "Bearer ", "username", "opRef"] {
            assert!(!text.contains(secret), "{secret} is not in the export");
        }

        // A second export in the same second gets its own name.
        let second = export(&dir.0, &log, &input).unwrap();
        assert_ne!(second, path);
        assert!(second.to_str().unwrap().ends_with("-2.jsonl.gz"));
    }

    #[test]
    fn short_hidden_values_are_left_alone() {
        let log = RequestLog::default();
        filled(&log);
        let hidden = ["ab".to_owned(), "  ".to_owned()];
        let lines = export_lines(&log, &export_input(&[], &[], &hidden)).unwrap();
        assert!(lines.iter().all(|l| !l.contains(&format!("{MASK}{MASK}"))));
        assert!(lines[0].contains("\"type\":\"header\""));
    }

    #[test]
    fn only_debug_exports_are_revealed() {
        let dir = TempDir::new();
        let export = dir.0.join("eb-mcp-debug-20261111-140322.jsonl.gz");
        fs::write(&export, b"x").unwrap();
        assert!(is_export(&export));
        let other = dir.0.join("token.json");
        fs::write(&other, b"x").unwrap();
        assert!(!is_export(&other));
        assert!(!is_export(&dir.0.join("eb-mcp-debug-missing.jsonl.gz")));
        assert!(!is_export(Path::new("eb-mcp-debug-1.jsonl.gz")));
        let history = dir.0.join("eb-mcp-akkoord-20261111-140322.jsonl.gz");
        fs::write(&history, b"x").unwrap();
        assert!(is_export(&history));
        let other = dir.0.join("eb-mcp-akkoord-20261111-140322.json");
        fs::write(&other, b"x").unwrap();
        assert!(!is_export(&other));
    }
}
