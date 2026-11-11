//! The request log and its statistics, kept in memory and saved to disk.
//!
//! The log keeps the last 1000 calls: MCP tool calls and the e-Boekhouden
//! calls they caused, linked by id. Cookies, the auth-token and passwords are
//! never stored: request headers are kept masked, and nothing else carries them.
//! The statistics survive Leegmaken. They count AI traffic only: the sign-in's
//! own calls show in the log but not in the statistics, as they are not rate
//! limited and no AI client made them.
//!
//! Both survive a restart and an update: `open` loads them from a gzip file in
//! the app's config folder (logstore.rs) and `save` writes them back when they
//! changed. The statistics therefore count since `started_at`, the start of the
//! first run that this file knows: the first start with persistence, or the
//! first start after the file was missing, damaged or of an unknown format.
//!
//! A tool that changes the books on an account that asks for approval returns at once while its
//! proposal waits in EB MCP (approvals.rs). Its MCP entry is then `Waiting`, with the proposal's
//! id in `proposal`: it holds up neither a quit nor an update restart, as `running` leaves it out.
//! Once the user approves, the entry is `Running` again while the change goes out, and the
//! e-Boekhouden calls it makes are linked to it like any other; it ends `Ok` or `Error` when the
//! change ran, or `Refused` when the user refused it or the proposal lapsed. An entry still
//! waiting when EB MCP stopped comes back `Refused` with `LAPSED`.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::ebapi::ApiCall;
use crate::logstore;
use crate::state::now_ms;

pub const CAPACITY: usize = 1000;
/// Bodies larger than this are kept as a shortened copy.
const BODY_LIMIT: usize = 256 * 1024;
const DURATIONS_KEPT: usize = 500;
const MINUTES_KEPT: u64 = 24 * 60;
pub const MASK: &str = "••••••••";
/// The app version that made an entry.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
/// The format of the file `save` writes. A file with another number is not read.
pub const FORMAT: u32 = 1;
/// The error of an MCP call that still ran when EB MCP stopped.
pub const RESTARTED: &str =
    "EB MCP is opnieuw gestart terwijl deze aanroep liep, dus hij is niet afgemaakt.";
/// The error of an MCP call whose change still waited for the user's approval when EB MCP stopped.
pub const LAPSED: &str = "EB MCP is opnieuw gestart voordat de gebruiker antwoordde; het voorstel is vervallen en er is niets gewijzigd.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    Running,
    /// The change waits for the user's approval in EB MCP; the tool call has answered already.
    Waiting,
    Ok,
    Error,
    Refused,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpEntry {
    pub id: u64,
    pub at: u64,
    /// The app version that made the entry; empty for entries saved before versions were kept.
    #[serde(default)]
    pub version: String,
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
    /// The proposal (approvals.rs) of a change this call asked for, once it went up for approval or ran without.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EbEntry {
    pub id: u64,
    pub at: u64,
    /// The app version that made the entry; empty for entries saved before versions were kept.
    #[serde(default)]
    pub version: String,
    pub parent: Option<u64>,
    pub client: String,
    pub account: Option<String>,
    /// An owned copy of `ApiCall::method`, so entries read back from disk need no static string.
    pub method: String,
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
    /// Made by a sign-in (`auth/login`, `auth`, or the sign-in's own line): listed, never counted in the statistics.
    pub signin: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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

#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Endpoint {
    count: u32,
    total_ms: u64,
    durations: VecDeque<u32>,
    errors: u32,
    bytes: u64,
}

#[derive(Default, Clone, Copy, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Minute {
    pub eb: u32,
    pub mcp: u32,
    pub errors: u32,
}

#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
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

/// What the log file holds (logstore.rs): the entries, oldest first, and the statistics.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    format: u32,
    /// The app version that wrote the file.
    app_version: String,
    saved_at: u64,
    next: u64,
    entries: Vec<Entry>,
    stats: Stats,
}

impl Snapshot {
    pub fn format(&self) -> u32 {
        self.format
    }
}

pub struct RequestLog {
    entries: Mutex<VecDeque<Entry>>,
    next: Mutex<u64>,
    stats: Mutex<Stats>,
    /// Counts every change; `saved` holds the count the file on disk has.
    changes: AtomicU64,
    saved: AtomicU64,
    /// The file `save` writes, set once by `open`. Without it the log lives in memory only.
    file: OnceLock<PathBuf>,
    /// Held by `save` from the copy to the rename, so an older copy never lands after a newer one.
    saving: Mutex<()>,
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
            changes: AtomicU64::new(0),
            saved: AtomicU64::new(0),
            file: OnceLock::new(),
            saving: Mutex::new(()),
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

    fn changed(&self) {
        self.changes.fetch_add(1, Ordering::SeqCst);
    }

    fn push(&self, entry: Entry) {
        {
            let mut entries = self.entries.lock().unwrap();
            entries.push_back(entry);
            while entries.len() > CAPACITY {
                entries.pop_front();
            }
        }
        self.changed();
    }

    /// Loads the log and the statistics from `file` and saves to it from now on. A missing,
    /// damaged or unknown file leaves the log empty and is replaced at the next save; it never
    /// stops the app. Called once, at startup, before any call is logged.
    pub fn open(&self, file: PathBuf) {
        match logstore::read(&file) {
            Ok(Some(snapshot)) => {
                let restored = self.restore(snapshot);
                log::info!(
                    "request log: {restored} entries loaded from {}",
                    file.display()
                );
            }
            Ok(None) => log::info!(
                "request log: no file yet at {}, starting empty",
                file.display()
            ),
            Err(e) => {
                // The unreadable file is set aside, not overwritten: it may still be worth a look.
                let aside = logstore::set_aside(&file);
                log::warn!(
                    "request log: {} not read ({e}), set aside as {}, starting empty",
                    file.display(),
                    aside.display()
                );
            }
        }
        if self.file.set(file).is_err() {
            log::warn!("request log: opened twice, the first file stays in use");
        }
    }

    /// Takes over a saved log. An MCP call that still ran when the app stopped becomes an error, one
    /// whose change still waited for approval is refused; both are counted in the statistics as
    /// `finish_mcp` would have. Returns how many entries were loaded.
    fn restore(&self, snapshot: Snapshot) -> usize {
        let Snapshot {
            next,
            entries: saved,
            stats: mut saved_stats,
            ..
        } = snapshot;
        let mut interrupted = Vec::new();
        let mut restored: VecDeque<Entry> = saved
            .into_iter()
            .map(|mut entry| {
                if let Entry::Mcp(e) = &mut entry {
                    match e.outcome {
                        Outcome::Running => {
                            e.outcome = Outcome::Error;
                            e.error = Some(RESTARTED.to_owned());
                            interrupted.push(e.clone());
                        }
                        Outcome::Waiting => {
                            e.outcome = Outcome::Refused;
                            e.error = Some(LAPSED.to_owned());
                            interrupted.push(e.clone());
                        }
                        Outcome::Ok | Outcome::Error | Outcome::Refused => {}
                    }
                }
                entry
            })
            .collect();
        while restored.len() > CAPACITY {
            restored.pop_front();
        }
        let highest = restored.iter().map(Entry::id).max().unwrap_or(0);
        if saved_stats.started_at == 0 {
            saved_stats.started_at = now_ms();
        }
        for e in &interrupted {
            saved_stats.count_mcp(e);
        }
        saved_stats.prune(now_ms());
        let count = restored.len();
        *self.entries.lock().unwrap() = restored;
        *self.next.lock().unwrap() = next.max(highest + 1).max(1);
        *self.stats.lock().unwrap() = saved_stats;
        if !interrupted.is_empty() {
            // The interrupted calls are errors now: the file should say so too.
            self.changed();
        }
        count
    }

    /// A copy of everything `save` writes. Holds each mutex only for the copy.
    fn snapshot(&self) -> Snapshot {
        let entries: Vec<Entry> = self.entries.lock().unwrap().iter().cloned().collect();
        let next = *self.next.lock().unwrap();
        let stats = self.stats.lock().unwrap().clone();
        Snapshot {
            format: FORMAT,
            app_version: APP_VERSION.to_owned(),
            saved_at: now_ms(),
            next,
            entries,
            stats,
        }
    }

    /// Whether something changed since the last save.
    pub fn dirty(&self) -> bool {
        self.changes.load(Ordering::SeqCst) != self.saved.load(Ordering::SeqCst)
    }

    /// Writes the log and the statistics to the file `open` gave, when something changed since
    /// the last save. Compressing happens outside the log's mutexes, but it takes time for a full
    /// log: async code calls this through `spawn_blocking`. `Ok(true)` when a file was written.
    pub fn save(&self) -> Result<bool, String> {
        let Some(file) = self.file.get() else {
            return Ok(false);
        };
        let _saving = self.saving.lock().unwrap_or_else(|e| e.into_inner());
        let changes = self.changes.load(Ordering::SeqCst);
        if changes == self.saved.load(Ordering::SeqCst) {
            return Ok(false);
        }
        logstore::write(file, &self.snapshot())?;
        self.saved.store(changes, Ordering::SeqCst);
        Ok(true)
    }

    /// Every entry with its bodies, oldest first, for the debug export.
    pub fn export_entries(&self) -> Vec<Entry> {
        self.entries.lock().unwrap().iter().cloned().collect()
    }

    pub fn start_mcp(&self, client: &str, tool: &str, args: Value, account: Option<String>) -> u64 {
        let id = self.next_id();
        self.push(Entry::Mcp(McpEntry {
            id,
            at: now_ms(),
            version: APP_VERSION.to_owned(),
            client: client.to_owned(),
            tool: tool.to_owned(),
            // A tool can be handed a whole layout document or a long message: the log keeps a short copy.
            args: shorten(&args).0,
            account,
            outcome: Outcome::Running,
            duration_ms: 0,
            bytes: 0,
            error: None,
            calls: Vec::new(),
            result: None,
            proposal: None,
        }));
        id
    }

    /// Links an MCP call to the proposal of the change it asked for (approvals.rs).
    pub fn link_proposal(&self, id: u64, proposal: u64) {
        self.set_mcp(id, |e| e.proposal = Some(proposal));
    }

    /// The call's change waits for the user's approval: the call no longer counts as running.
    pub fn wait_mcp(&self, id: u64) {
        self.set_mcp(id, |e| {
            if e.outcome == Outcome::Running {
                e.outcome = Outcome::Waiting;
            }
        });
    }

    /// The user approved: the call runs again until its change is done.
    pub fn resume_mcp(&self, id: u64) {
        self.set_mcp(id, |e| {
            if e.outcome == Outcome::Waiting {
                e.outcome = Outcome::Running;
            }
        });
    }

    fn set_mcp(&self, id: u64, change: impl FnOnce(&mut McpEntry)) {
        let found = {
            let mut entries = self.entries.lock().unwrap();
            match entries.iter_mut().find(|e| e.id() == id) {
                Some(Entry::Mcp(e)) => {
                    change(e);
                    true
                }
                _ => false,
            }
        };
        if found {
            self.changed();
        }
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
            // Once only: a call that ended already keeps its outcome and is not counted twice.
            if let Some(Entry::Mcp(e)) = entries.iter_mut().find(|e| e.id() == id)
                && matches!(e.outcome, Outcome::Running | Outcome::Waiting)
            {
                e.outcome = outcome;
                e.duration_ms = now_ms().saturating_sub(e.at);
                e.bytes = result.map(|r| r.to_string().len()).unwrap_or(0);
                e.result = result.map(|r| shorten(r).0);
                e.error = error;
                done = Some(e.clone());
            }
        }
        if let Some(e) = done {
            self.stats.lock().unwrap().count_mcp(&e);
            self.changed();
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
        self.record(call, parent, client, account, had_session, false)
    }

    /// Records a call of a sign-in: in the log like any other, but left out of the statistics.
    pub fn add_signin(&self, call: &ApiCall, client: &str, account: (Uuid, String)) -> u64 {
        self.record(call, None, client, Some(account), true, true)
    }

    fn record(
        &self,
        call: &ApiCall,
        parent: Option<u64>,
        client: &str,
        account: Option<(Uuid, String)>,
        had_session: bool,
        signin: bool,
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
            version: APP_VERSION.to_owned(),
            parent,
            client: client.to_owned(),
            account: label.clone(),
            method: call.method.to_owned(),
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
            signin,
        }));
        if let Some(parent) = parent {
            let mut entries = self.entries.lock().unwrap();
            if let Some(Entry::Mcp(e)) = entries.iter_mut().find(|e| e.id() == parent) {
                e.calls.push(id);
            }
        }
        if call.status != 0 && !signin {
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
        // Once more after the link to the parent and the statistics, so a save that copied the
        // log in between is followed by another.
        self.changed();
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

    /// MCP calls still running, which an update restart and a quit wait for. A call whose change
    /// waits for the user's approval does not count: it has answered its client already.
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

    /// Leegmaken: the log only, in memory and on disk. The statistics keep counting. Saves right
    /// away, so the cleared entries do not come back after a restart; with an empty log that file
    /// is small, but the save may wait for one that runs: async code calls this through `spawn_blocking`.
    pub fn clear(&self) {
        self.entries.lock().unwrap().clear();
        self.changed();
        if let Err(e) = self.save() {
            log::warn!("request log: the cleared log could not be saved: {e}");
        }
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
        self.prune(at_ms);
        self.minutes.entry(at_ms / 60_000).or_default()
    }

    /// Drops the minute buckets older than a day before `now_ms`.
    fn prune(&mut self, now_ms: u64) {
        let minute = now_ms / 60_000;
        while self
            .minutes
            .first_key_value()
            .is_some_and(|(k, _)| *k + MINUTES_KEPT < minute)
        {
            self.minutes.pop_first();
        }
    }

    /// Counts a finished MCP call.
    fn count_mcp(&mut self, e: &McpEntry) {
        let failed = e.outcome != Outcome::Ok;
        self.mcp += 1;
        self.bytes_out += e.bytes as u64;
        if failed {
            self.errors += 1;
        }
        let minute = self.minute(e.at);
        minute.mcp += 1;
        if failed {
            minute.errors += 1;
        }
        *self.tools.entry(e.tool.clone()).or_default() += 1;
        self.clients.entry(e.client.clone()).or_default().0 += 1;
        if e.outcome == Outcome::Refused {
            *self.statuses.entry("Geweigerd".into()).or_default() += 1;
        }
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
pub(crate) fn shorten(value: &Value) -> (Value, bool) {
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
    fn lists_sign_in_calls_without_counting_them() {
        let log = RequestLog::default();
        let account = (Uuid::nil(), "SN".to_owned());
        log.add_signin(
            &call("auth/login", 401, 200),
            "EB MCP (inloggen)",
            account.clone(),
        );
        log.add_signin(&call("auth", 200, 80), "EB MCP (inloggen)", account.clone());
        log.add_eb(
            &call("relatie/gridtable", 500, 300),
            None,
            "Claude",
            Some(account),
            true,
        );
        let list = log.list();
        assert_eq!(list.len(), 3, "the sign-in's calls stay in Verzoeken");
        let signins = list
            .iter()
            .filter(|e| matches!(e, Entry::Eb(eb) if eb.signin))
            .count();
        assert_eq!(signins, 2);
        let stats = log.stats();
        assert_eq!((stats.eb, stats.errors), (1, 1));
        assert_eq!(stats.minutes.len(), 1);
        assert_eq!((stats.minutes[0].1.eb, stats.minutes[0].1.errors), (1, 1));
        assert_eq!(stats.endpoints.len(), 1);
        assert_eq!(stats.endpoints[0].path, "relatie/gridtable");
        assert_eq!(
            stats
                .clients
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            ["Claude"]
        );
        assert_eq!(stats.accounts[0].count, 1);
        assert_eq!(stats.avg_ms, 300);
        assert_eq!(
            stats
                .statuses
                .iter()
                .map(|n| n.name.as_str())
                .collect::<Vec<_>>(),
            ["500"]
        );
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
    fn a_waiting_change_is_not_running_and_ends_once() {
        let log = RequestLog::default();
        let id = log.start_mcp("Claude", "mutatie_toevoegen", json!({}), Some("SN".into()));
        assert_eq!(log.running(), 1);
        log.link_proposal(id, 7);
        log.wait_mcp(id);
        assert_eq!(
            log.running(),
            0,
            "a waiting change holds up no quit or restart"
        );
        let Some(Entry::Mcp(m)) = log.get(id) else {
            panic!()
        };
        assert_eq!((m.outcome, m.proposal), (Outcome::Waiting, Some(7)));
        assert_eq!(log.stats().mcp, 0, "not counted before it ends");
        log.resume_mcp(id);
        assert_eq!(log.running(), 1);
        log.finish_mcp(id, Outcome::Ok, Some(&json!({"mutNr": 1})), None);
        log.finish_mcp(id, Outcome::Error, None, Some("later".into()));
        let Some(Entry::Mcp(m)) = log.get(id) else {
            panic!()
        };
        assert_eq!(m.outcome, Outcome::Ok, "the first end stays");
        assert_eq!(log.stats().mcp, 1);
        let listed = serde_json::to_value(log.list()).unwrap();
        assert_eq!(listed[0]["proposal"], 7);
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
