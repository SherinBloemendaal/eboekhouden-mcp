//! The request log and the run's statistics, in memory only.
//!
//! The log keeps the last 1000 calls: MCP tool calls and the e-Boekhouden
//! calls they caused, linked by id. Cookies, the auth-token and passwords are
//! never stored: request headers are kept masked, and nothing else carries them.
//! The statistics count from the start of the run and survive Leegmaken.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Mutex;

use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::ebapi::ApiCall;
use crate::state::now_ms;

pub const CAPACITY: usize = 1000;
/// Bodies larger than this are kept as a shortened copy.
const BODY_LIMIT: usize = 256 * 1024;
const DURATIONS_KEPT: usize = 500;
const MINUTES_KEPT: u64 = 24 * 60;
pub const MASK: &str = "••••••••";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    Running,
    Ok,
    Error,
    Refused,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpEntry {
    pub id: u64,
    pub at: u64,
    pub client: String,
    pub tool: String,
    pub args: Value,
    pub account: Option<String>,
    pub outcome: Outcome,
    pub duration_ms: u64,
    pub bytes: usize,
    pub error: Option<String>,
    pub calls: Vec<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EbEntry {
    pub id: u64,
    pub at: u64,
    pub parent: Option<u64>,
    pub client: String,
    pub account: Option<String>,
    pub method: &'static str,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub status: u16,
    pub duration_ms: u64,
    pub bytes: usize,
    pub rows: Option<usize>,
    pub error: Option<String>,
    pub headers: Vec<(String, String)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<Value>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Entry {
    Mcp(McpEntry),
    Eb(EbEntry),
}

impl Entry {
    fn id(&self) -> u64 {
        match self {
            Entry::Mcp(e) => e.id,
            Entry::Eb(e) => e.id,
        }
    }

    /// The list view leaves the bodies out; the detail view asks for one entry.
    fn summary(&self) -> Entry {
        match self.clone() {
            Entry::Mcp(mut e) => {
                e.result = None;
                Entry::Mcp(e)
            }
            Entry::Eb(mut e) => {
                e.response = None;
                Entry::Eb(e)
            }
        }
    }
}

#[derive(Default)]
struct Endpoint {
    count: u32,
    total_ms: u64,
    durations: VecDeque<u32>,
    errors: u32,
    bytes: u64,
}

#[derive(Default, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Minute {
    pub eb: u32,
    pub mcp: u32,
    pub errors: u32,
}

#[derive(Default)]
struct Stats {
    started_at: u64,
    mcp: u32,
    eb: u32,
    errors: u32,
    bytes_in: u64,
    bytes_out: u64,
    eb_total_ms: u64,
    eb_durations: VecDeque<u32>,
    minutes: BTreeMap<u64, Minute>,
    endpoints: HashMap<String, Endpoint>,
    tools: HashMap<String, u32>,
    clients: HashMap<String, (u32, u32)>,
    accounts: HashMap<String, u32>,
    statuses: HashMap<String, u32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EndpointStats {
    pub path: String,
    pub count: u32,
    pub avg_ms: u64,
    pub p95_ms: u64,
    pub errors: u32,
    pub bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Named {
    pub name: String,
    pub count: u32,
    pub eb: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsView {
    pub started_at: u64,
    pub mcp: u32,
    pub eb: u32,
    pub errors: u32,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub avg_ms: u64,
    pub p95_ms: u64,
    /// Minute buckets, oldest first, as (minute since epoch, counts).
    pub minutes: Vec<(u64, Minute)>,
    pub endpoints: Vec<EndpointStats>,
    pub tools: Vec<Named>,
    pub clients: Vec<Named>,
    pub accounts: Vec<Named>,
    pub statuses: Vec<Named>,
}

pub struct RequestLog {
    entries: Mutex<VecDeque<Entry>>,
    next: Mutex<u64>,
    stats: Mutex<Stats>,
}

impl Default for RequestLog {
    fn default() -> Self {
        Self {
            entries: Mutex::new(VecDeque::new()),
            next: Mutex::new(1),
            stats: Mutex::new(Stats {
                started_at: now_ms(),
                ..Default::default()
            }),
        }
    }
}

impl RequestLog {
    fn next_id(&self) -> u64 {
        let mut next = self.next.lock().unwrap();
        let id = *next;
        *next += 1;
        id
    }

    fn push(&self, entry: Entry) {
        let mut entries = self.entries.lock().unwrap();
        entries.push_back(entry);
        while entries.len() > CAPACITY {
            entries.pop_front();
        }
    }

    pub fn start_mcp(&self, client: &str, tool: &str, args: Value, account: Option<String>) -> u64 {
        let id = self.next_id();
        self.push(Entry::Mcp(McpEntry {
            id,
            at: now_ms(),
            client: client.to_owned(),
            tool: tool.to_owned(),
            args,
            account,
            outcome: Outcome::Running,
            duration_ms: 0,
            bytes: 0,
            error: None,
            calls: Vec::new(),
            result: None,
        }));
        id
    }

    pub fn finish_mcp(
        &self,
        id: u64,
        outcome: Outcome,
        result: Option<&Value>,
        error: Option<String>,
    ) {
        let mut done = None;
        {
            let mut entries = self.entries.lock().unwrap();
            if let Some(Entry::Mcp(e)) = entries.iter_mut().find(|e| e.id() == id) {
                e.outcome = outcome;
                e.duration_ms = now_ms().saturating_sub(e.at);
                e.bytes = result.map(|r| r.to_string().len()).unwrap_or(0);
                e.result = result.map(|r| shorten(r).0);
                e.error = error;
                done = Some(e.clone());
            }
        }
        if let Some(e) = done {
            let mut s = self.stats.lock().unwrap();
            s.mcp += 1;
            s.bytes_out += e.bytes as u64;
            if outcome != Outcome::Ok {
                s.errors += 1;
            }
            s.minute(e.at).mcp += 1;
            if outcome != Outcome::Ok {
                s.minute(e.at).errors += 1;
            }
            *s.tools.entry(e.tool.clone()).or_default() += 1;
            s.clients.entry(e.client.clone()).or_default().0 += 1;
            if outcome == Outcome::Refused {
                *s.statuses.entry("Geweigerd".into()).or_default() += 1;
            }
        }
    }

    /// Records an e-Boekhouden call. `account` is the label; the cookies never reach here.
    pub fn add_eb(
        &self,
        call: &ApiCall,
        parent: Option<u64>,
        client: &str,
        account: Option<(Uuid, String)>,
        had_session: bool,
    ) -> u64 {
        let id = self.next_id();
        let at = now_ms().saturating_sub(call.duration_ms);
        let mut headers = vec![
            ("Host".to_owned(), "secure20.e-boekhouden.nl".to_owned()),
            ("Accept".to_owned(), "application/json".to_owned()),
        ];
        if had_session {
            headers.push(("Cookie".to_owned(), MASK.to_owned()));
            headers.push(("Authorization".to_owned(), MASK.to_owned()));
        }
        headers.push((
            "User-Agent".to_owned(),
            concat!("EB MCP/", env!("CARGO_PKG_VERSION")).to_owned(),
        ));
        let (response, truncated) = match &call.json {
            Some(json) => {
                let (value, cut) = shorten(json);
                (Some(value), cut)
            }
            None => (None, false),
        };
        let label = account.as_ref().map(|(_, l)| l.clone());
        self.push(Entry::Eb(EbEntry {
            id,
            at,
            parent,
            client: client.to_owned(),
            account: label.clone(),
            method: call.method,
            path: call.path.clone(),
            query: call.query.clone(),
            status: call.status,
            duration_ms: call.duration_ms,
            bytes: call.bytes,
            rows: call.rows,
            error: call.error.clone(),
            headers,
            response,
            truncated,
        }));
        if let Some(parent) = parent {
            let mut entries = self.entries.lock().unwrap();
            if let Some(Entry::Mcp(e)) = entries.iter_mut().find(|e| e.id() == parent) {
                e.calls.push(id);
            }
        }
        if call.status != 0 {
            let mut s = self.stats.lock().unwrap();
            s.eb += 1;
            s.bytes_in += call.bytes as u64;
            let failed = !call.ok();
            if failed {
                s.errors += 1;
            }
            let minute = s.minute(at);
            minute.eb += 1;
            if failed {
                minute.errors += 1;
            }
            s.eb_total_ms += call.duration_ms;
            push_capped(&mut s.eb_durations, call.duration_ms as u32);
            let endpoint = s.endpoints.entry(normalize(&call.path)).or_default();
            endpoint.count += 1;
            endpoint.total_ms += call.duration_ms;
            push_capped(&mut endpoint.durations, call.duration_ms as u32);
            endpoint.bytes += call.bytes as u64;
            if failed {
                endpoint.errors += 1;
            }
            s.clients.entry(client.to_owned()).or_default().1 += 1;
            if let Some(label) = label {
                *s.accounts.entry(label).or_default() += 1;
            }
            *s.statuses.entry(call.status.to_string()).or_default() += 1;
        }
        id
    }

    /// Newest first, without bodies.
    pub fn list(&self) -> Vec<Entry> {
        self.entries
            .lock()
            .unwrap()
            .iter()
            .rev()
            .map(Entry::summary)
            .collect()
    }

    pub fn get(&self, id: u64) -> Option<Entry> {
        self.entries
            .lock()
            .unwrap()
            .iter()
            .find(|e| e.id() == id)
            .cloned()
    }

    /// MCP calls still running, which an update restart waits for.
    pub fn running(&self) -> usize {
        self.entries
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, Entry::Mcp(m) if m.outcome == Outcome::Running))
            .count()
    }

    pub fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }

    /// Leegmaken: the log only. The statistics keep counting.
    pub fn clear(&self) {
        self.entries.lock().unwrap().clear();
    }

    pub fn stats(&self) -> StatsView {
        let s = self.stats.lock().unwrap();
        let named = |map: &HashMap<String, u32>| {
            let mut list: Vec<Named> = map
                .iter()
                .map(|(k, v)| Named {
                    name: k.clone(),
                    count: *v,
                    eb: 0,
                })
                .collect();
            list.sort_by(|a, b| b.count.cmp(&a.count).then(a.name.cmp(&b.name)));
            list
        };
        let mut endpoints: Vec<EndpointStats> = s
            .endpoints
            .iter()
            .map(|(path, e)| EndpointStats {
                path: path.clone(),
                count: e.count,
                avg_ms: e.total_ms / u64::from(e.count.max(1)),
                p95_ms: p95(&e.durations),
                errors: e.errors,
                bytes: e.bytes,
            })
            .collect();
        endpoints.sort_by(|a, b| b.count.cmp(&a.count).then(a.path.cmp(&b.path)));
        let mut clients: Vec<Named> = s
            .clients
            .iter()
            .map(|(k, (m, e))| Named {
                name: k.clone(),
                count: *m,
                eb: *e,
            })
            .collect();
        clients.sort_by(|a, b| {
            (b.count + b.eb)
                .cmp(&(a.count + a.eb))
                .then(a.name.cmp(&b.name))
        });
        StatsView {
            started_at: s.started_at,
            mcp: s.mcp,
            eb: s.eb,
            errors: s.errors,
            bytes_in: s.bytes_in,
            bytes_out: s.bytes_out,
            avg_ms: s.eb_total_ms / u64::from(s.eb.max(1)),
            p95_ms: p95(&s.eb_durations),
            minutes: s.minutes.iter().map(|(k, v)| (*k, *v)).collect(),
            endpoints,
            tools: named(&s.tools),
            clients,
            accounts: named(&s.accounts),
            statuses: named(&s.statuses),
        }
    }
}

impl Stats {
    fn minute(&mut self, at_ms: u64) -> &mut Minute {
        let minute = at_ms / 60_000;
        while self
            .minutes
            .first_key_value()
            .is_some_and(|(k, _)| *k + MINUTES_KEPT < minute)
        {
            self.minutes.pop_first();
        }
        self.minutes.entry(minute).or_default()
    }
}

fn push_capped(list: &mut VecDeque<u32>, value: u32) {
    list.push_back(value);
    while list.len() > DURATIONS_KEPT {
        list.pop_front();
    }
}

fn p95(durations: &VecDeque<u32>) -> u64 {
    if durations.is_empty() {
        return 0;
    }
    let mut sorted: Vec<u32> = durations.iter().copied().collect();
    sorted.sort_unstable();
    let index = ((sorted.len() as f64) * 0.95).ceil() as usize;
    u64::from(sorted[index.clamp(1, sorted.len()) - 1])
}

/// `relatie/48211` and `folder/1184/filelist` count as `relatie/{id}` and `folder/{id}/filelist`.
pub fn normalize(path: &str) -> String {
    path.split('/')
        .map(|part| {
            let id_like = !part.is_empty()
                && (part.chars().all(|c| c.is_ascii_digit())
                    || (part.len() >= 32
                        && part.chars().all(|c| c.is_ascii_hexdigit() || c == '-')));
            if id_like { "{id}" } else { part }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// A copy small enough to keep: long lists keep their first 50 items and say how many were cut.
fn shorten(value: &Value) -> (Value, bool) {
    if value.to_string().len() <= BODY_LIMIT {
        return (value.clone(), false);
    }
    fn cut(value: &Value) -> Value {
        match value {
            Value::Array(items) if items.len() > 50 => {
                let mut kept: Vec<Value> = items.iter().take(50).map(cut).collect();
                kept.push(Value::String(format!("… nog {} items", items.len() - 50)));
                Value::Array(kept)
            }
            Value::Array(items) => Value::Array(items.iter().map(cut).collect()),
            Value::Object(map) => {
                Value::Object(map.iter().map(|(k, v)| (k.clone(), cut(v))).collect())
            }
            Value::String(s) if s.len() > 2000 => {
                Value::String(format!("{}…", s.chars().take(2000).collect::<String>()))
            }
            other => other.clone(),
        }
    }
    (cut(value), true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(path: &str, status: u16, ms: u64) -> ApiCall {
        ApiCall {
            method: "GET",
            path: path.into(),
            query: vec![],
            status,
            duration_ms: ms,
            bytes: 100,
            rows: Some(2),
            json: Some(json!({"items": [1, 2]})),
            error: (status >= 400).then(|| "fout".to_owned()),
        }
    }

    #[test]
    fn links_calls_to_their_tool_and_masks_the_session() {
        let log = RequestLog::default();
        let mcp = log.start_mcp(
            "Claude",
            "relaties",
            json!({"zoek": "bakker"}),
            Some("Studio Noord".into()),
        );
        let eb = log.add_eb(
            &call("relatie/gridtable", 200, 120),
            Some(mcp),
            "Claude",
            Some((Uuid::nil(), "Studio Noord".into())),
            true,
        );
        log.finish_mcp(mcp, Outcome::Ok, Some(&json!({"aantal": 2})), None);
        let Some(Entry::Mcp(m)) = log.get(mcp) else {
            panic!()
        };
        assert_eq!(m.calls, vec![eb]);
        let Some(Entry::Eb(e)) = log.get(eb) else {
            panic!()
        };
        assert!(e.headers.iter().any(|(k, v)| k == "Cookie" && v == MASK));
        assert!(!serde_json::to_string(&e).unwrap().contains("auth-token"));
    }

    #[test]
    fn keeps_the_last_thousand_and_lists_newest_first() {
        let log = RequestLog::default();
        for i in 0..1005 {
            log.add_eb(
                &call(&format!("relatie/{i}"), 200, 10),
                None,
                "EB MCP",
                None,
                true,
            );
        }
        assert_eq!(log.len(), CAPACITY);
        let list = log.list();
        let Entry::Eb(first) = &list[0] else { panic!() };
        assert_eq!(first.path, "relatie/1004");
        assert!(first.response.is_none(), "the list leaves bodies out");
    }

    #[test]
    fn clearing_keeps_the_statistics() {
        let log = RequestLog::default();
        log.add_eb(
            &call("relatie/1", 200, 100),
            None,
            "Claude",
            Some((Uuid::nil(), "SN".into())),
            true,
        );
        log.add_eb(
            &call("relatie/2", 500, 300),
            None,
            "Claude",
            Some((Uuid::nil(), "SN".into())),
            true,
        );
        log.clear();
        assert_eq!(log.len(), 0);
        let stats = log.stats();
        assert_eq!((stats.eb, stats.errors), (2, 1));
        assert_eq!(stats.endpoints[0].path, "relatie/{id}");
        assert_eq!(stats.endpoints[0].count, 2);
        assert_eq!(stats.endpoints[0].p95_ms, 300);
        assert_eq!(stats.accounts[0].count, 2);
    }

    #[test]
    fn counts_refused_and_failed_tools() {
        let log = RequestLog::default();
        let a = log.start_mcp("Cursor", "get", json!({"pad": "mutatie/opslaan"}), None);
        log.finish_mcp(a, Outcome::Refused, None, Some("niet toegestaan".into()));
        let stats = log.stats();
        assert_eq!((stats.mcp, stats.errors), (1, 1));
        assert_eq!(stats.statuses[0].name, "Geweigerd");
    }

    #[test]
    fn normalizes_ids_and_percentiles() {
        assert_eq!(normalize("folder/1184/filelist"), "folder/{id}/filelist");
        assert_eq!(normalize("overzicht/balans"), "overzicht/balans");
        let durations: VecDeque<u32> = (1..=100).collect();
        assert_eq!(p95(&durations), 95);
    }

    #[test]
    fn shortens_large_bodies() {
        let big = json!({"items": (0..10000).map(|i| json!({"naam": format!("Relatie {i} met een lange omschrijving")})).collect::<Vec<_>>()});
        let (short, cut) = shorten(&big);
        assert!(cut);
        assert_eq!(short["items"].as_array().unwrap().len(), 51);
    }
}
