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
//!
//! The times of an MCP entry: `duration_ms` is the whole span from the call coming in to its end,
//! the wait for the user's answer included. It meant that before the wait was recorded too, so older
//! entries and exports read the same. `wait_started_at` and `wait_ended_at` say when the proposal went
//! up and when the wait ended (approved, refused or lapsed), and `waited_ms` is that wait: 0 for a call
//! that never waited. What EB MCP itself worked on the call is `duration_ms - waited_ms`, the work
//! time Verzoeken shows (`workMs` in src/lib/log.ts).
//! The e-Boekhouden calls of a waiting call fall before `wait_started_at` (preparing) or after
//! `wait_ended_at` (making the change). The statistics never count the wait: they hold the times of
//! e-Boekhouden calls only.

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
    /// From the call coming in to its end, the wait for approval included; 0 until it ends.
    pub duration_ms: u64,
    pub bytes: usize,
    pub error: Option<String>,
    pub calls: Vec<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// The proposal (approvals.rs) of a change this call asked for, once it went up for approval or ran without.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<u64>,
    /// When the change went up for the user's approval; `None` when it never waited.
    #[serde(default)]
    pub wait_started_at: Option<u64>,
    /// When the wait ended: approved, refused or lapsed. `None` when it never waited or still waits.
    #[serde(default)]
    pub wait_ended_at: Option<u64>,
    /// The part of `duration_ms` spent waiting for the user; 0 when it never waited or still waits.
    #[serde(default)]
    pub waited_ms: u64,
}

impl McpEntry {
    /// Ends the wait at `end`, once; a call that never waited is left alone.
    fn end_wait(&mut self, end: u64) {
        if let (Some(start), None) = (self.wait_started_at, self.wait_ended_at) {
            let end = end.max(start);
            self.wait_ended_at = Some(end);
            self.waited_ms = end - start;
        }
    }
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

/// Whether a call to e-Boekhouden failed, for the statistics. A redirect is a failure (the login page),
/// except the one that opens the template editor: that call answers 302 with the editor's session in its
/// `Location`, and `editor_open` leaves it without an error when it found one.
fn failed(call: &ApiCall) -> bool {
    let editor_opened = call.error.is_none()
        && (300..400).contains(&call.status)
        && call.path.contains(EDITOR_OPEN);
    !(call.ok() || editor_opened)
}

/// The part of a path that opens the template editor (`factuursjabloon/editor/{id}/NL`).
const EDITOR_OPEN: &str = "sjabloon/editor/";

/// Up to 0.5.0 every opening of the template editor was counted as a failure, so the Dashboard said that
/// path "faalt vaak". A log saved by such a version carries those counts.
fn counted_editor_redirects(saved_by: &str) -> bool {
    let mut parts = saved_by.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    let version = (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    );
    version <= (0, 5, 0)
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
    /// `finish_mcp` would have. A wait still open ends at the file's last save, the nearest known
    /// moment to the stop, so the wait never takes the time EB MCP was not running for work. Returns
    /// how many entries were loaded.
    fn restore(&self, snapshot: Snapshot) -> usize {
        let Snapshot {
            next,
            saved_at,
            entries: saved,
            stats: mut saved_stats,
            app_version,
            ..
        } = snapshot;
        if counted_editor_redirects(&app_version) {
            saved_stats.forget_editor_redirect_errors();
        }
        let mut interrupted = Vec::new();
        let mut restored: VecDeque<Entry> = saved
            .into_iter()
            .map(|mut entry| {
                if let Entry::Mcp(e) = &mut entry {
                    match e.outcome {
                        Outcome::Running => {
                            e.outcome = Outcome::Error;
                            e.error = Some(RESTARTED.to_owned());
                            // Approved, then cut off while the change ran: the span reaches at least
                            // the end of the wait, so the work time never comes out negative.
                            if let Some(ended) = e.wait_ended_at {
                                e.duration_ms = saved_at.max(ended).saturating_sub(e.at);
                            }
                            interrupted.push(e.clone());
                        }
                        Outcome::Waiting => {
                            e.outcome = Outcome::Refused;
                            e.error = Some(LAPSED.to_owned());
                            let end = saved_at.max(e.wait_started_at.unwrap_or(e.at)).max(e.at);
                            e.end_wait(end);
                            e.duration_ms = end - e.at;
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
            wait_started_at: None,
            wait_ended_at: None,
            waited_ms: 0,
        }));
        id
    }

    /// Gives an entry from before waits were recorded (0.4.0) the wait its proposal's record knows:
    /// asked at `asked`, answered at `decided`. Only an ended call of the same tool that carries that
    /// proposal and has no wait yet takes it, and the wait stays inside the call's own span. A call on
    /// an account that needs no approval never waited: the caller leaves those out. `true` when set.
    pub fn backfill_wait(
        &self,
        proposal: u64,
        tool: &str,
        asked: u64,
        decided: Option<u64>,
    ) -> bool {
        let found = {
            let mut entries = self.entries.lock().unwrap();
            let entry = entries.iter_mut().find_map(|e| match e {
                Entry::Mcp(m)
                    if m.proposal == Some(proposal)
                        && m.tool == tool
                        && m.wait_started_at.is_none()
                        && !matches!(m.outcome, Outcome::Running | Outcome::Waiting) =>
                {
                    Some(m)
                }
                _ => None,
            });
            match entry {
                Some(m) => {
                    let end_of_call = m.at + m.duration_ms;
                    let start = asked.clamp(m.at, end_of_call);
                    m.wait_started_at = Some(start);
                    m.end_wait(decided.unwrap_or(end_of_call).clamp(start, end_of_call));
                    true
                }
                None => false,
            }
        };
        if found {
            self.changed();
        }
        found
    }

    /// Links an MCP call to the proposal of the change it asked for (approvals.rs).
    pub fn link_proposal(&self, id: u64, proposal: u64) {
        self.set_mcp(id, |e| e.proposal = Some(proposal));
    }

    /// The call's change waits for the user's approval: the call no longer counts as running, and
    /// the wait starts now.
    pub fn wait_mcp(&self, id: u64) {
        let now = now_ms();
        self.set_mcp(id, |e| {
            if e.outcome == Outcome::Running && e.wait_started_at.is_none() {
                e.outcome = Outcome::Waiting;
                e.wait_started_at = Some(now.max(e.at));
            }
        });
    }

    /// The user approved: the wait ends now, and the call runs again until its change is done.
    pub fn resume_mcp(&self, id: u64) {
        let now = now_ms();
        self.set_mcp(id, |e| {
            if e.outcome == Outcome::Waiting {
                e.outcome = Outcome::Running;
                e.end_wait(now);
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
                let now = now_ms();
                // Refused or lapsed while it waited: the wait ends with the call.
                if e.outcome == Outcome::Waiting {
                    e.end_wait(now);
                }
                e.outcome = outcome;
                e.duration_ms = now.saturating_sub(e.at);
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
            let failed = failed(call);
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
    /// Takes back the failures that were counted for opening the template editor (see
    /// `counted_editor_redirects`): none of them was one.
    fn forget_editor_redirect_errors(&mut self) {
        for (path, endpoint) in self.endpoints.iter_mut() {
            if path.contains(EDITOR_OPEN) {
                self.errors = self.errors.saturating_sub(endpoint.errors);
                endpoint.errors = 0;
            }
        }
    }

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

    fn mcp(log: &RequestLog, id: u64) -> McpEntry {
        match log.get(id) {
            Some(Entry::Mcp(m)) => m,
            other => panic!("no MCP entry {id}: {other:?}"),
        }
    }

    /// What EB MCP itself worked on a call, as Verzoeken shows it.
    fn work(m: &McpEntry) -> u64 {
        m.duration_ms - m.waited_ms
    }

    fn eb_at(log: &RequestLog, id: u64) -> u64 {
        match log.get(id) {
            Some(Entry::Eb(e)) => e.at,
            other => panic!("no e-Boekhouden entry {id}: {other:?}"),
        }
    }

    fn pause(ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }

    #[test]
    fn an_approved_change_records_its_wait_apart_from_the_work() {
        let log = RequestLog::default();
        let id = log.start_mcp("Claude", "mutatie_toevoegen", json!({}), Some("SN".into()));
        let read = log.add_eb(
            &call("grootboekrekening/gridtable", 200, 0),
            Some(id),
            "Claude",
            None,
            true,
        );
        pause(5);
        log.link_proposal(id, 7);
        log.wait_mcp(id);
        let waiting = mcp(&log, id);
        let started = waiting.wait_started_at.expect("the wait started");
        assert!(started >= waiting.at);
        assert_eq!(
            (waiting.wait_ended_at, waiting.waited_ms),
            (None, 0),
            "still waiting"
        );
        let listed = serde_json::to_value(log.list()).unwrap();
        assert_eq!(listed[1]["waitStartedAt"], started);
        assert!(listed[1]["waitEndedAt"].is_null());
        assert_eq!(listed[1]["waitedMs"], 0);

        pause(40);
        log.resume_mcp(id);
        let resumed = mcp(&log, id);
        let ended = resumed
            .wait_ended_at
            .expect("the wait ended at the approval");
        assert_eq!(resumed.waited_ms, ended - started);
        assert!(resumed.waited_ms >= 40, "{}", resumed.waited_ms);
        let sent = log.add_eb(&call("mutatie", 200, 0), Some(id), "Claude", None, true);
        log.finish_mcp(id, Outcome::Ok, Some(&json!({"mutNr": 1})), None);

        let done = mcp(&log, id);
        assert_eq!(
            done.wait_ended_at,
            Some(ended),
            "the end of the wait stays the approval"
        );
        assert_eq!(done.waited_ms, ended - started);
        assert!(done.duration_ms >= done.waited_ms);
        assert!(work(&done) < done.duration_ms);
        // The e-Boekhouden calls fall on either side of the wait.
        assert!(eb_at(&log, read) <= started);
        assert!(eb_at(&log, sent) >= ended);
    }

    #[test]
    fn a_refused_change_waited_until_its_end() {
        let log = RequestLog::default();
        let id = log.start_mcp(
            "Claude",
            "factuur_verwijderen",
            json!({}),
            Some("SN".into()),
        );
        log.wait_mcp(id);
        pause(30);
        log.finish_mcp(id, Outcome::Refused, None, Some("geweigerd".into()));
        let m = mcp(&log, id);
        assert_eq!(m.outcome, Outcome::Refused);
        let (started, ended) = (m.wait_started_at.unwrap(), m.wait_ended_at.unwrap());
        assert_eq!(m.waited_ms, ended - started);
        assert!(m.waited_ms >= 30);
        assert_eq!(m.duration_ms, ended - m.at, "nothing ran after the wait");
        assert_eq!(work(&m), started - m.at);
    }

    #[test]
    fn an_entry_from_before_waits_were_recorded_takes_the_wait_of_its_proposal() {
        let log = RequestLog::default();
        let id = log.start_mcp("Claude", "mutatie_toevoegen", json!({}), Some("SN".into()));
        log.link_proposal(id, 7);
        pause(30);
        log.finish_mcp(id, Outcome::Ok, None, None);
        let before = mcp(&log, id);
        assert_eq!((before.wait_started_at, before.waited_ms), (None, 0));

        // Another tool, another proposal or a call that still runs is left alone.
        assert!(!log.backfill_wait(7, "relatie_toevoegen", before.at + 5, Some(before.at + 20)));
        assert!(!log.backfill_wait(8, "mutatie_toevoegen", before.at + 5, Some(before.at + 20)));

        assert!(log.backfill_wait(7, "mutatie_toevoegen", before.at + 5, Some(before.at + 20)));
        let after = mcp(&log, id);
        assert_eq!(after.wait_started_at, Some(before.at + 5));
        assert_eq!(after.wait_ended_at, Some(before.at + 20));
        assert_eq!(after.waited_ms, 15);
        assert_eq!(
            after.duration_ms, before.duration_ms,
            "the whole span stays"
        );
        // Once only, and a wait never reaches outside the call.
        assert!(!log.backfill_wait(7, "mutatie_toevoegen", 0, None));

        let refused = log.start_mcp(
            "Claude",
            "factuur_verwijderen",
            json!({}),
            Some("SN".into()),
        );
        log.link_proposal(refused, 9);
        pause(10);
        log.finish_mcp(refused, Outcome::Refused, None, Some("geweigerd".into()));
        let r = mcp(&log, refused);
        assert!(log.backfill_wait(9, "factuur_verwijderen", 0, Some(u64::MAX)));
        let r2 = mcp(&log, refused);
        assert_eq!(r2.wait_started_at, Some(r.at));
        assert_eq!(r2.wait_ended_at, Some(r.at + r.duration_ms));
        assert_eq!(r2.waited_ms, r.duration_ms);
    }

    #[test]
    fn a_call_that_never_waited_has_no_wait() {
        let log = RequestLog::default();
        let id = log.start_mcp("Claude", "relaties", json!({}), Some("SN".into()));
        pause(5);
        // Resuming what never waited changes nothing.
        log.resume_mcp(id);
        log.finish_mcp(id, Outcome::Ok, None, None);
        let m = mcp(&log, id);
        assert_eq!(
            (m.wait_started_at, m.wait_ended_at, m.waited_ms),
            (None, None, 0)
        );
        assert_eq!(work(&m), m.duration_ms);
        let listed = serde_json::to_value(log.list()).unwrap();
        assert!(listed[0]["waitStartedAt"].is_null());
        assert!(listed[0]["waitEndedAt"].is_null());
        assert_eq!(listed[0]["waitedMs"], 0);
    }

    #[test]
    fn a_wait_open_at_the_stop_ends_at_the_last_save() {
        let log = RequestLog::default();
        let waiting = log.start_mcp("Claude", "mutatie_toevoegen", json!({}), None);
        log.wait_mcp(waiting);
        let approved = log.start_mcp("Claude", "relatie_wijzigen", json!({}), None);
        log.wait_mcp(approved);
        log.resume_mcp(approved);
        let started = mcp(&log, waiting).wait_started_at.unwrap();
        let approved_end = mcp(&log, approved).wait_ended_at.unwrap();
        let mut snapshot = log.snapshot();
        snapshot.saved_at = started + 5_000;

        let again = RequestLog::default();
        again.restore(snapshot);
        let lapsed = mcp(&again, waiting);
        assert_eq!(lapsed.outcome, Outcome::Refused);
        assert_eq!(lapsed.error.as_deref(), Some(LAPSED));
        assert_eq!(lapsed.wait_ended_at, Some(started + 5_000));
        assert_eq!(lapsed.waited_ms, 5_000);
        assert_eq!(lapsed.duration_ms, started + 5_000 - lapsed.at);
        assert_eq!(work(&lapsed), started - lapsed.at);
        // Cut off while the approved change ran: the wait stays as it was, the span covers it.
        let cut = mcp(&again, approved);
        assert_eq!(cut.outcome, Outcome::Error);
        assert_eq!(cut.wait_ended_at, Some(approved_end));
        assert!(cut.duration_ms >= cut.waited_ms);
        assert_eq!(cut.duration_ms, started + 5_000 - cut.at);
    }

    #[test]
    fn the_statistics_leave_the_wait_out() {
        let log = RequestLog::default();
        let id = log.start_mcp("Claude", "mutatie_toevoegen", json!({}), Some("SN".into()));
        log.add_eb(
            &call("grootboekrekening/gridtable", 200, 100),
            Some(id),
            "Claude",
            None,
            true,
        );
        log.wait_mcp(id);
        pause(60);
        log.resume_mcp(id);
        log.add_eb(&call("mutatie", 200, 300), Some(id), "Claude", None, true);
        log.finish_mcp(id, Outcome::Ok, None, None);
        assert!(mcp(&log, id).waited_ms >= 60);
        let stats = log.stats();
        assert_eq!((stats.mcp, stats.eb), (1, 2));
        assert_eq!(stats.avg_ms, 200, "the e-Boekhouden calls only");
        assert_eq!(stats.p95_ms, 300);
    }

    #[test]
    fn an_entry_saved_before_waits_were_kept_loads_without_one() {
        let old = json!({
            "kind": "mcp", "id": 4, "at": 1000, "version": "0.3.2", "client": "Claude", "tool": "mutatie_toevoegen",
            "args": {}, "account": "SN", "outcome": "ok", "durationMs": 47_600, "bytes": 2,
            "error": null, "calls": [], "proposal": 2
        });
        let Entry::Mcp(m) = serde_json::from_value::<Entry>(old).unwrap() else {
            panic!()
        };
        assert_eq!(
            (m.wait_started_at, m.wait_ended_at, m.waited_ms),
            (None, None, 0)
        );
        assert_eq!(
            work(&m),
            47_600,
            "without a recorded wait the whole span counts"
        );
    }

    #[test]
    fn opening_the_template_editor_is_not_a_failure_but_a_login_redirect_is() {
        let log = RequestLog::default();
        // The editor opens with a 302 that carries its session: no error on the call.
        log.add_eb(
            &call("factuursjabloon/editor/878203/NL", 302, 40),
            None,
            "Claude",
            None,
            true,
        );
        assert_eq!(log.stats().errors, 0);
        // Any other redirect is the login page, and a redirect with an error stays a failure.
        log.add_eb(
            &call("relatie/gridtable", 302, 40),
            None,
            "Claude",
            None,
            true,
        );
        let mut lost = call("offertesjabloon/editor/5/NL", 302, 40);
        lost.error = Some("Niet (meer) ingelogd bij e-Boekhouden".into());
        log.add_eb(&lost, None, "Claude", None, true);
        assert_eq!(log.stats().errors, 2);
    }

    #[test]
    fn a_log_saved_up_to_0_5_0_loses_its_false_editor_failures() {
        assert!(counted_editor_redirects("0.3.1"));
        assert!(counted_editor_redirects("0.5.0"));
        assert!(!counted_editor_redirects("0.5.1"));
        assert!(!counted_editor_redirects("1.0.0"));
        let mut stats = Stats {
            errors: 9,
            ..Default::default()
        };
        stats
            .endpoints
            .entry("factuursjabloon/editor/{id}/NL".into())
            .or_default()
            .errors = 7;
        stats
            .endpoints
            .entry("relatie/gridtable".into())
            .or_default()
            .errors = 2;
        stats.forget_editor_redirect_errors();
        assert_eq!(stats.errors, 2);
        assert_eq!(stats.endpoints["factuursjabloon/editor/{id}/NL"].errors, 0);
        assert_eq!(stats.endpoints["relatie/gridtable"].errors, 2);
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
