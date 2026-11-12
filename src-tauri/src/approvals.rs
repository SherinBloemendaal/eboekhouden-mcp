//! Changes AI clients asked for: the ones that wait for the user's word, and the history of all of them.
//!
//! A tool that would change an account's books puts its proposal here (`ask`) and its MCP call answers at
//! once that the change waits (tools.rs, `write_tool`). The change itself waits in a background task on the
//! receiver `ask` returns, and only "Goedkeuren" lets its requests go to e-Boekhouden. A proposal waits until
//! the user answers, the account is removed or set to read-only (refused), the account's sign-in details
//! change or EB MCP stops (lapsed). There is no time limit.
//!
//! Every proposal stays in the history once it is answered, with what was asked, the request body, the tool
//! and its arguments, the client, the account, the times, the decision (and whether the user gave it to all
//! waiting proposals at once), the app version and the outcome. A
//! change an "auto" account made without asking is recorded too (decision `auto`), so the history is the
//! full record of what AI clients changed. The history is kept in a gzip file in the app's config folder
//! (`history_file`, written through logstore.rs as the request log is), at most `CAPACITY` records. Values
//! under keys that look like a password, token, cookie or session key are masked before they are kept.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::logstore;
use crate::reqlog::{APP_VERSION, MASK, shorten, without_pay_links};
use crate::state::now_ms;

/// How many records the history keeps. Older answered ones go first; waiting and running ones stay.
pub const CAPACITY: usize = 2000;
/// The format of the history file. A file with another number is not read.
pub const FORMAT: u32 = 1;
/// The format of the lines of an export.
const EXPORT_FORMAT: u32 = 1;

/// Why nothing was changed, in the words the AI client and the user read.
pub const REFUSED_BY_USER: &str =
    "De gebruiker heeft dit geweigerd in EB MCP. Er is niets gewijzigd.";
pub const REFUSED_ACCOUNT_GONE: &str =
    "Het account is uit EB MCP verwijderd voordat de gebruiker antwoordde. Er is niets gewijzigd.";
pub const REFUSED_READ_ONLY: &str = "Schrijven is voor dit account uitgezet voordat de gebruiker antwoordde. Er is niets gewijzigd.";
pub const LAPSED_ACCOUNT_CHANGED: &str = "De inloggegevens van het account zijn gewijzigd voordat de gebruiker antwoordde; het voorstel is vervallen. Er is niets gewijzigd.";
pub const LAPSED_RESTART: &str = "EB MCP is opnieuw gestart voordat de gebruiker antwoordde; het voorstel is vervallen. Er is niets gewijzigd.";
pub const LAPSED_WHILE_RUNNING: &str =
    "EB MCP is gestopt terwijl deze wijziging liep. Kijk in e-Boekhouden wat er al is doorgevoerd.";
pub const LAPSED_NOBODY_WAITS: &str = "De taak die op dit antwoord wachtte bestaat niet meer; het voorstel is vervallen. Er is niets gewijzigd.";

/// One line of a proposal: what will be set and to what, in words the user knows from e-Boekhouden.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    pub label: String,
    pub value: String,
}

pub fn line(label: &str, value: impl ToString) -> Line {
    Line {
        label: label.to_owned(),
        value: value.to_string(),
    }
}

/// What a tool wants to do, as the user sees it before deciding.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Proposal {
    pub id: u64,
    pub account_id: Uuid,
    pub account: String,
    /// The AI client that asked ("Claude", "Cursor").
    pub client: String,
    pub tool: String,
    /// "Mutatie toevoegen: Geld uitgegeven", "Factuur F00012 verwijderen".
    pub title: String,
    pub lines: Vec<Line>,
    /// Deleting and crediting: shown as such, and asked even when the account needs no approval otherwise.
    pub danger: bool,
    pub asked_at: u64,
}

/// What a tool hands in; the registry adds the id and the time.
pub struct Ask {
    pub account_id: Uuid,
    pub account: String,
    pub client: String,
    pub tool: String,
    pub title: String,
    pub lines: Vec<Line>,
    pub danger: bool,
}

/// The technical side of a proposal, kept in the history but left out of its list.
pub struct Technical {
    /// The tool's arguments as the AI client gave them.
    pub args: Value,
    /// The request body the change sends (`Built.body`); `Null` when it sends none.
    pub body: Value,
    /// The MCP call in the request log that asked for it.
    pub log_id: Option<u64>,
}

/// How a proposal was decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Decision {
    Approved,
    Refused,
    /// The account needs no approval for this change: it ran without asking.
    Auto,
    /// Nobody answered: the app stopped, or the account's sign-in details changed.
    Lapsed,
}

/// Where a proposal is now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Waiting,
    Running,
    Done,
    Failed,
    Refused,
    Lapsed,
}

impl Status {
    /// Waiting or running: not done with yet, so the history keeps it whatever its age.
    fn open(self) -> bool {
        matches!(self, Status::Waiting | Status::Running)
    }

    /// The Dutch word `akkoord_status` uses.
    pub fn dutch(self) -> &'static str {
        match self {
            Status::Waiting => "wacht",
            Status::Running => "bezig",
            Status::Done => "uitgevoerd",
            Status::Failed => "mislukt",
            Status::Refused => "geweigerd",
            Status::Lapsed => "vervallen",
        }
    }
}

impl Decision {
    /// The Dutch words `akkoord_status` uses.
    pub fn dutch(self) -> &'static str {
        match self {
            Decision::Approved => "goedgekeurd",
            Decision::Refused => "geweigerd",
            Decision::Auto => "zonder akkoord",
            Decision::Lapsed => "vervallen",
        }
    }
}

/// A proposal in the history, as `approvals_history` lists it: without the technical data.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    #[serde(flatten)]
    pub proposal: Proposal,
    pub decided_at: Option<u64>,
    pub decision: Option<Decision>,
    pub status: Status,
    pub error: Option<String>,
    /// The app version that asked.
    pub version: String,
    /// The user answered it with Alles goedkeuren or Alles weigeren (`decide_all`), not on its own.
    #[serde(default)]
    pub bulk: bool,
}

/// A proposal with its technical data, as the history keeps it and `approval_get` shows it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Detail {
    #[serde(flatten)]
    pub record: Record,
    pub args: Value,
    pub body: Value,
    /// What the change gave back once it ran: the tool's answer to its client.
    pub result: Option<Value>,
    pub log_id: Option<u64>,
    /// When the change finished running.
    pub finished_at: Option<u64>,
}

impl Detail {
    /// The record as it is written to the history file or an export: without a link to pay something. In
    /// memory the result keeps it, because `akkoord_status` is how the AI client gets the outcome of a
    /// change that waited, and for a payment link the link is that outcome. After a restart it is gone.
    fn for_disk(&self) -> Detail {
        Detail {
            args: without_pay_links(&self.args),
            body: without_pay_links(&self.body),
            result: self.result.as_ref().map(without_pay_links),
            ..self.clone()
        }
    }
}

/// The user's answer, or why there is none, as the waiting change hears it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    Approved,
    Refused(String),
    Lapsed(String),
}

/// What `decide_all` did with the ids it was given.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecideAll {
    /// Answered now.
    pub decided: Vec<u64>,
    /// Not waiting any more (answered, refused or lapsed already, or never there).
    pub gone: Vec<u64>,
}

/// What the history file holds.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    format: u32,
    /// The app version that wrote the file.
    app_version: String,
    saved_at: u64,
    next: u64,
    /// Oldest first.
    records: Vec<Detail>,
}

struct Inner {
    next: u64,
    /// The waiting proposals and the channel to the change that waits on each.
    pending: BTreeMap<u64, oneshot::Sender<Answer>>,
    /// Oldest first.
    records: VecDeque<Detail>,
}

impl Inner {
    fn record_mut(&mut self, id: u64) -> Option<&mut Detail> {
        self.records.iter_mut().find(|d| d.record.proposal.id == id)
    }

    fn push(&mut self, detail: Detail) {
        self.records.push_back(detail);
        while self.records.len() > CAPACITY {
            match self.records.iter().position(|d| !d.record.status.open()) {
                Some(oldest) => {
                    self.records.remove(oldest);
                }
                None => break,
            }
        }
    }
}

pub struct Approvals {
    inner: Mutex<Inner>,
    /// Counts every change; `saved` holds the count the file on disk has.
    changes: AtomicU64,
    saved: AtomicU64,
    /// The file `save` writes, set once by `open`. Without it the history lives in memory only.
    file: OnceLock<PathBuf>,
    /// Held by `save` from the copy to the rename, so an older copy never lands after a newer one.
    saving: Mutex<()>,
}

impl Default for Approvals {
    fn default() -> Self {
        Self {
            inner: Mutex::new(Inner {
                next: 1,
                pending: BTreeMap::new(),
                records: VecDeque::new(),
            }),
            changes: AtomicU64::new(0),
            saved: AtomicU64::new(0),
            file: OnceLock::new(),
            saving: Mutex::new(()),
        }
    }
}

/// Where the history lives in the app's config folder. Development builds keep their own file, as the
/// request log does, so a `tauri dev` run never touches the installed app's history.
pub fn history_file(dir: &Path) -> PathBuf {
    dir.join(if cfg!(debug_assertions) {
        "dev-approvals.json.gz"
    } else {
        "approvals.json.gz"
    })
}

/// A copy without secrets: the value under any key that names a password, token, cookie, session key or
/// authorization is replaced by the mask, at any depth.
pub fn scrub(value: &Value) -> Value {
    const SECRET_KEYS: [&str; 8] = [
        "password",
        "wachtwoord",
        "token",
        "cookie",
        "secret",
        "authorization",
        "sessionhash",
        "sessionkey",
    ];
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| {
                    let lower = key.to_ascii_lowercase().replace(['-', '_'], "");
                    let secret = SECRET_KEYS.iter().any(|s| lower.contains(s));
                    let kept = if secret && !value.is_null() {
                        Value::String(MASK.to_owned())
                    } else {
                        scrub(value)
                    };
                    (key.clone(), kept)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(scrub).collect()),
        other => other.clone(),
    }
}

/// What the history keeps of a value: without secrets, and shortened when it is large.
fn kept(value: &Value) -> Value {
    shorten(&scrub(value)).0
}

impl Approvals {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn changed(&self) {
        self.changes.fetch_add(1, Ordering::SeqCst);
    }

    fn new_detail(
        inner: &mut Inner,
        ask: Ask,
        technical: Technical,
        now: u64,
        auto: bool,
    ) -> Detail {
        let id = inner.next;
        inner.next += 1;
        Detail {
            record: Record {
                proposal: Proposal {
                    id,
                    account_id: ask.account_id,
                    account: ask.account,
                    client: ask.client,
                    tool: ask.tool,
                    title: ask.title,
                    lines: ask.lines,
                    danger: ask.danger,
                    asked_at: now,
                },
                decided_at: auto.then_some(now),
                decision: auto.then_some(Decision::Auto),
                status: if auto {
                    Status::Running
                } else {
                    Status::Waiting
                },
                error: None,
                version: APP_VERSION.to_owned(),
                bulk: false,
            },
            args: kept(&technical.args),
            body: kept(&technical.body),
            result: None,
            log_id: technical.log_id,
            finished_at: None,
        }
    }

    /// Puts a proposal up for the user. The receiver hears the answer, or why there is none.
    pub fn ask(
        &self,
        ask: Ask,
        technical: Technical,
        now: u64,
    ) -> (Proposal, oneshot::Receiver<Answer>) {
        let (sender, receiver) = oneshot::channel();
        let proposal = {
            let mut inner = self.lock();
            let detail = Self::new_detail(&mut inner, ask, technical, now, false);
            let proposal = detail.record.proposal.clone();
            inner.push(detail);
            inner.pending.insert(proposal.id, sender);
            proposal
        };
        self.changed();
        (proposal, receiver)
    }

    /// Records a change that runs without asking, on an account that needs no approval for it. Its id.
    pub fn record_auto(&self, ask: Ask, technical: Technical, now: u64) -> u64 {
        let id = {
            let mut inner = self.lock();
            let detail = Self::new_detail(&mut inner, ask, technical, now, true);
            let id = detail.record.proposal.id;
            inner.push(detail);
            id
        };
        self.changed();
        id
    }

    /// Ends the wait of one proposal with `answer`. `false` when it was not waiting (any more), or when the
    /// change that waited on it is gone; the record then says the proposal lapsed. `bulk`: the user gave the
    /// answer to all waiting proposals at once; the record keeps that when the answer arrived.
    fn close(&self, inner: &mut Inner, id: u64, answer: Answer, now: u64, bulk: bool) -> bool {
        let Some(sender) = inner.pending.remove(&id) else {
            return false;
        };
        let delivered = sender.send(answer.clone()).is_ok();
        if let Some(detail) = inner.record_mut(id) {
            let record = &mut detail.record;
            record.decided_at = Some(now);
            record.bulk = bulk && delivered;
            match (&answer, delivered) {
                (Answer::Approved, true) => {
                    record.decision = Some(Decision::Approved);
                    record.status = Status::Running;
                }
                (Answer::Refused(message), true) => {
                    record.decision = Some(Decision::Refused);
                    record.status = Status::Refused;
                    record.error = Some(message.clone());
                }
                (Answer::Lapsed(message), true) => {
                    record.decision = Some(Decision::Lapsed);
                    record.status = Status::Lapsed;
                    record.error = Some(message.clone());
                }
                (_, false) => {
                    record.decision = Some(Decision::Lapsed);
                    record.status = Status::Lapsed;
                    record.error = Some(LAPSED_NOBODY_WAITS.to_owned());
                }
            }
        }
        self.changed();
        delivered
    }

    /// The user's answer. `false` when the proposal is not waiting any more.
    pub fn decide(&self, id: u64, approve: bool, now: u64) -> bool {
        let answer = if approve {
            Answer::Approved
        } else {
            Answer::Refused(REFUSED_BY_USER.to_owned())
        };
        let mut inner = self.lock();
        self.close(&mut inner, id, answer, now, false)
    }

    /// The same answer for exactly the proposals in `ids`, the ones the user saw. A proposal that came up
    /// meanwhile is not among them and keeps waiting.
    pub fn decide_all(&self, ids: &[u64], approve: bool, now: u64) -> DecideAll {
        let mut done = DecideAll::default();
        let mut inner = self.lock();
        for &id in ids {
            if done.decided.contains(&id) || done.gone.contains(&id) {
                continue;
            }
            let answer = if approve {
                Answer::Approved
            } else {
                Answer::Refused(REFUSED_BY_USER.to_owned())
            };
            if self.close(&mut inner, id, answer, now, true) {
                done.decided.push(id);
            } else {
                done.gone.push(id);
            }
        }
        done
    }

    /// Ends one waiting proposal without the user's answer: refused (`Answer::Refused`) or lapsed.
    pub fn end(&self, id: u64, answer: Answer, now: u64) -> bool {
        let mut inner = self.lock();
        self.close(&mut inner, id, answer, now, false)
    }

    /// Ends every waiting proposal of an account with `answer`: refused when the account is removed or set to
    /// read-only, lapsed when its sign-in details changed. How many there were.
    pub fn end_account(&self, account_id: Uuid, answer: Answer, now: u64) -> usize {
        let mut inner = self.lock();
        let ids: Vec<u64> = inner
            .records
            .iter()
            .filter(|d| {
                d.record.proposal.account_id == account_id
                    && inner.pending.contains_key(&d.record.proposal.id)
            })
            .map(|d| d.record.proposal.id)
            .collect();
        for &id in &ids {
            self.close(&mut inner, id, answer.clone(), now, false);
        }
        ids.len()
    }

    /// The outcome of a change that ran: done with the tool's answer, or failed with the error. Only a
    /// running record takes it; one that was refused or lapsed keeps saying so.
    pub fn finish(&self, id: u64, outcome: Result<&Value, String>, now: u64) {
        let mut inner = self.lock();
        let Some(detail) = inner.record_mut(id) else {
            return;
        };
        if detail.record.status != Status::Running {
            return;
        }
        match outcome {
            Ok(result) => {
                detail.record.status = Status::Done;
                detail.result = Some(kept(result));
            }
            Err(message) => {
                detail.record.status = Status::Failed;
                detail.record.error = Some(message);
            }
        }
        detail.finished_at = Some(now);
        drop(inner);
        self.changed();
    }

    /// The waiting proposals, oldest first.
    pub fn list(&self) -> Vec<Proposal> {
        let inner = self.lock();
        inner
            .pending
            .keys()
            .filter_map(|id| {
                inner
                    .records
                    .iter()
                    .find(|d| d.record.proposal.id == *id)
                    .map(|d| d.record.proposal.clone())
            })
            .collect()
    }

    pub fn count(&self) -> usize {
        self.lock().pending.len()
    }

    /// The history, newest first, without the technical data; at most `limit` records.
    pub fn history(&self, limit: Option<usize>) -> Vec<Record> {
        self.lock()
            .records
            .iter()
            .rev()
            .take(limit.unwrap_or(CAPACITY))
            .map(|d| d.record.clone())
            .collect()
    }

    /// One proposal with its technical data.
    pub fn get(&self, id: u64) -> Option<Detail> {
        self.lock()
            .records
            .iter()
            .find(|d| d.record.proposal.id == id)
            .cloned()
    }

    /// The proposals asked at or after `since_ms`, newest first, with their technical data.
    pub fn since(&self, since_ms: u64) -> Vec<Detail> {
        self.lock()
            .records
            .iter()
            .rev()
            .filter(|d| d.record.proposal.asked_at >= since_ms)
            .cloned()
            .collect()
    }

    /// The highest id given out so far; 0 before the first.
    pub fn last_id(&self) -> u64 {
        self.lock().next.saturating_sub(1)
    }

    /// Loads the history from `file` and saves to it from now on. Proposals that still waited or ran when
    /// EB MCP stopped come back lapsed. A missing, damaged or unknown file leaves the history empty: a damaged
    /// one is set aside and replaced at the next save. Called once, at startup, before any tool call.
    pub fn open(&self, file: PathBuf) {
        match logstore::read_versioned::<Snapshot>(&file, FORMAT) {
            Ok(Some(snapshot)) => {
                let (count, lapsed) = self.restore(snapshot, now_ms());
                log::info!(
                    "approvals: {count} records loaded from {}, {lapsed} lapsed",
                    file.display()
                );
            }
            Ok(None) => log::info!(
                "approvals: no history yet at {}, starting empty",
                file.display()
            ),
            Err(e) => {
                let aside = logstore::set_aside(&file);
                log::warn!(
                    "approvals: {} not read ({e}), set aside as {}, starting empty",
                    file.display(),
                    aside.display()
                );
            }
        }
        if self.file.set(file).is_err() {
            log::warn!("approvals: opened twice, the first file stays in use");
        }
    }

    /// Takes over a saved history: how many records, and how many of them lapsed now.
    fn restore(&self, snapshot: Snapshot, now: u64) -> (usize, usize) {
        let mut lapsed = 0;
        let mut inner = self.lock();
        inner.pending.clear();
        inner.records.clear();
        let mut highest = 0;
        for mut detail in snapshot.records {
            let record = &mut detail.record;
            highest = highest.max(record.proposal.id);
            match record.status {
                Status::Waiting => {
                    record.status = Status::Lapsed;
                    record.decision = Some(Decision::Lapsed);
                    record.decided_at = Some(now);
                    record.error = Some(LAPSED_RESTART.to_owned());
                    lapsed += 1;
                }
                Status::Running => {
                    // Approved or auto, and stopped halfway: the decision stays, the outcome is unknown.
                    record.status = Status::Lapsed;
                    record.error = Some(LAPSED_WHILE_RUNNING.to_owned());
                    detail.finished_at = Some(now);
                    lapsed += 1;
                }
                Status::Done | Status::Failed | Status::Refused | Status::Lapsed => {}
            }
            inner.push(detail);
        }
        inner.next = snapshot.next.max(highest + 1).max(1);
        let count = inner.records.len();
        drop(inner);
        if lapsed > 0 {
            // The lapsed ones are final now: the file should say so too.
            self.changed();
        }
        (count, lapsed)
    }

    /// Whether something changed since the last save.
    pub fn dirty(&self) -> bool {
        self.changes.load(Ordering::SeqCst) != self.saved.load(Ordering::SeqCst)
    }

    /// Writes the history to the file `open` gave, when something changed since the last save. Compressing
    /// takes time for a full history: async code calls this through `spawn_blocking`. `Ok(true)` when a file
    /// was written.
    pub fn save(&self) -> Result<bool, String> {
        let Some(file) = self.file.get() else {
            return Ok(false);
        };
        let _saving = self.saving.lock().unwrap_or_else(|e| e.into_inner());
        let changes = self.changes.load(Ordering::SeqCst);
        if changes == self.saved.load(Ordering::SeqCst) {
            return Ok(false);
        }
        let snapshot = {
            let inner = self.lock();
            Snapshot {
                format: FORMAT,
                app_version: APP_VERSION.to_owned(),
                saved_at: now_ms(),
                next: inner.next,
                records: inner.records.iter().map(Detail::for_disk).collect(),
            }
        };
        logstore::write_gz(file, &snapshot)?;
        self.saved.store(changes, Ordering::SeqCst);
        Ok(true)
    }

    /// The export's lines: a header, then one line per record, oldest first, with the technical data.
    /// Every value in `hidden` (account usernames, 1Password references) is masked.
    pub fn export_lines(
        &self,
        hidden: &[String],
        exported_at: &DateTime<Local>,
    ) -> Result<Vec<String>, String> {
        let records: Vec<Detail> = self.lock().records.iter().map(Detail::for_disk).collect();
        let mut lines = vec![json!({
            "type": "header",
            "exportFormat": EXPORT_FORMAT,
            "app": "EB MCP",
            "appVersion": APP_VERSION,
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "exportedAt": exported_at.timestamp_millis(),
            "exportedAtLocal": exported_at.format("%Y-%m-%d %H:%M:%S %:z").to_string(),
            "counts": { "records": records.len() },
        })];
        for record in &records {
            lines.push(logstore::tagged("record", record)?);
        }
        logstore::masked_lines(&lines, hidden)
    }

    /// Writes the export into `dir` as `eb-mcp-akkoord-<YYYYMMDD-HHMMSS>.jsonl.gz` and returns its path.
    pub fn export(
        &self,
        dir: &Path,
        hidden: &[String],
        exported_at: &DateTime<Local>,
    ) -> Result<PathBuf, String> {
        let lines = self.export_lines(hidden, exported_at)?;
        logstore::write_export(dir, logstore::APPROVALS_PREFIX, exported_at, &lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::GzDecoder;
    use std::io::Read;

    fn ask(account_id: Uuid, title: &str) -> Ask {
        Ask {
            account_id,
            account: "Studio Noord".into(),
            client: "Claude".into(),
            tool: "mutatie_toevoegen".into(),
            title: title.into(),
            lines: vec![line("Bedrag", "2,00")],
            danger: false,
        }
    }

    fn technical() -> Technical {
        Technical {
            args: json!({"omschrijving": "Parkeergarage", "account": "Studio Noord"}),
            body: json!({"mutatie": {"omschrijving": "Parkeergarage"}}),
            log_id: Some(4),
        }
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("ebmcp-approvals-{}", Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_link_to_pay_stays_in_memory_and_off_the_disk() {
        let dir = TempDir::new();
        let file = history_file(&dir.0);
        let approvals = Approvals::default();
        approvals.open(file.clone());
        let id = approvals.record_auto(ask(Uuid::new_v4(), "Betaallink"), technical(), 10);
        approvals.finish(
            id,
            Ok(&json!({"data": {"betaallink": "https://betalen.example/abc", "periode": "3e kwartaal 2026"}})),
            20,
        );
        // The client asks for the outcome and gets the link.
        let now = approvals.get(id).unwrap();
        assert_eq!(
            now.result.unwrap()["data"]["betaallink"],
            "https://betalen.example/abc"
        );
        // The file and the export do not hold it.
        approvals.save().unwrap();
        let mut text = String::new();
        GzDecoder::new(std::fs::File::open(&file).unwrap())
            .read_to_string(&mut text)
            .unwrap();
        assert!(!text.contains("betalen.example"), "{text}");
        assert!(text.contains("3e kwartaal 2026"));
        let lines = approvals
            .export_lines(&[], &Local::now())
            .unwrap()
            .join("\n");
        assert!(!lines.contains("betalen.example"), "{lines}");
    }

    #[tokio::test]
    async fn an_answer_reaches_the_waiting_change_once() {
        let approvals = Approvals::default();
        let account = Uuid::new_v4();
        let (first, yes) = approvals.ask(ask(account, "Eerste"), technical(), 1);
        let (second, no) = approvals.ask(ask(account, "Tweede"), technical(), 2);
        assert_eq!(
            approvals.list().iter().map(|p| p.id).collect::<Vec<_>>(),
            vec![first.id, second.id],
            "oldest first"
        );
        assert!(approvals.decide(first.id, true, 10));
        assert!(approvals.decide(second.id, false, 11));
        assert_eq!(yes.await, Ok(Answer::Approved));
        assert_eq!(no.await, Ok(Answer::Refused(REFUSED_BY_USER.into())));
        assert_eq!(approvals.count(), 0);
        assert!(!approvals.decide(first.id, true, 12), "answered already");

        let approved = approvals.get(first.id).unwrap().record;
        assert_eq!(approved.decision, Some(Decision::Approved));
        assert_eq!(approved.status, Status::Running);
        assert_eq!(approved.decided_at, Some(10));
        let refused = approvals.get(second.id).unwrap().record;
        assert_eq!(
            (refused.decision, refused.status),
            (Some(Decision::Refused), Status::Refused)
        );
        approvals.finish(first.id, Ok(&json!({"mutNr": 3908})), 20);
        let done = approvals.get(first.id).unwrap();
        assert_eq!(done.record.status, Status::Done);
        assert_eq!(done.result, Some(json!({"mutNr": 3908})));
        assert_eq!(done.finished_at, Some(20));
        // A refused one stays refused, whatever is reported later.
        approvals.finish(second.id, Err("te laat".into()), 21);
        assert_eq!(
            approvals.get(second.id).unwrap().record.status,
            Status::Refused
        );
    }

    #[tokio::test]
    async fn approve_all_answers_exactly_the_ids_it_was_given() {
        let approvals = Approvals::default();
        let account = Uuid::new_v4();
        let (a, ra) = approvals.ask(ask(account, "A"), technical(), 1);
        let (b, rb) = approvals.ask(ask(account, "B"), technical(), 2);
        // The user saw A and B; C came up after the list was drawn.
        let (c, mut rc) = approvals.ask(ask(account, "C"), technical(), 3);
        assert!(
            approvals.decide(b.id, false, 4),
            "B answered on its own first"
        );
        let done = approvals.decide_all(&[a.id, b.id, a.id, 999], true, 5);
        assert_eq!(done.decided, vec![a.id]);
        assert_eq!(done.gone, vec![b.id, 999]);
        assert_eq!(ra.await, Ok(Answer::Approved));
        assert_eq!(rb.await, Ok(Answer::Refused(REFUSED_BY_USER.into())));
        assert!(rc.try_recv().is_err(), "C was not approved unseen");
        assert_eq!(
            approvals.list().iter().map(|p| p.id).collect::<Vec<_>>(),
            vec![c.id]
        );
        let refused = approvals.decide_all(&[c.id], false, 6);
        assert_eq!(refused.decided, vec![c.id]);
        assert_eq!(rc.await, Ok(Answer::Refused(REFUSED_BY_USER.into())));
        assert_eq!(
            serde_json::to_value(&done).unwrap(),
            json!({"decided": [a.id], "gone": [b.id, 999]})
        );
        // Each record says how it was answered: A and C with the bulk action, B on its own.
        assert!(approvals.get(a.id).unwrap().record.bulk);
        assert!(!approvals.get(b.id).unwrap().record.bulk);
        let c_record = approvals.get(c.id).unwrap().record;
        assert!(c_record.bulk);
        assert_eq!(c_record.decision, Some(Decision::Refused));
        assert_eq!(
            serde_json::to_value(&approvals.history(None)[0]).unwrap()["bulk"],
            true
        );
    }

    #[tokio::test]
    async fn a_bulk_answer_nobody_waits_for_lapses_without_the_bulk_mark() {
        let approvals = Approvals::default();
        let (proposal, receiver) = approvals.ask(ask(Uuid::new_v4(), "Weg"), technical(), 1);
        drop(receiver);
        assert_eq!(
            approvals.decide_all(&[proposal.id], true, 2).gone,
            vec![proposal.id]
        );
        let record = approvals.get(proposal.id).unwrap().record;
        assert_eq!(record.decision, Some(Decision::Lapsed));
        assert!(!record.bulk);
    }

    #[test]
    fn a_record_saved_before_bulk_was_kept_loads_as_answered_on_its_own() {
        let mut saved = serde_json::to_value(Detail {
            record: Record {
                proposal: Proposal {
                    id: 1,
                    account_id: Uuid::nil(),
                    account: "Studio Noord".into(),
                    client: "Claude".into(),
                    tool: "mutatie_toevoegen".into(),
                    title: "Oud".into(),
                    lines: vec![],
                    danger: false,
                    asked_at: 1,
                },
                decided_at: Some(2),
                decision: Some(Decision::Approved),
                status: Status::Done,
                error: None,
                version: "0.4.0".into(),
                bulk: true,
            },
            args: Value::Null,
            body: Value::Null,
            result: None,
            log_id: Some(4),
            finished_at: Some(3),
        })
        .unwrap();
        saved.as_object_mut().unwrap().remove("bulk");
        let detail: Detail = serde_json::from_value(saved).unwrap();
        assert!(!detail.record.bulk);
        assert_eq!(detail.record.proposal.title, "Oud");
    }

    #[tokio::test]
    async fn an_answer_nobody_waits_for_lapses() {
        let approvals = Approvals::default();
        let (proposal, receiver) = approvals.ask(ask(Uuid::new_v4(), "Weg"), technical(), 1);
        drop(receiver);
        assert!(!approvals.decide(proposal.id, true, 2));
        let record = approvals.get(proposal.id).unwrap().record;
        assert_eq!(record.status, Status::Lapsed);
        assert_eq!(approvals.count(), 0);
    }

    #[tokio::test]
    async fn an_account_that_goes_or_changes_ends_what_it_had_waiting() {
        let approvals = Approvals::default();
        let (gone, other) = (Uuid::new_v4(), Uuid::new_v4());
        let (refused, r1) = approvals.ask(ask(gone, "Van het account"), technical(), 1);
        let (kept, _r2) = approvals.ask(ask(other, "Van een ander"), technical(), 2);
        assert_eq!(
            approvals.end_account(gone, Answer::Refused(REFUSED_READ_ONLY.into()), 3),
            1
        );
        assert_eq!(r1.await, Ok(Answer::Refused(REFUSED_READ_ONLY.into())));
        assert_eq!(
            approvals.list().iter().map(|p| p.id).collect::<Vec<_>>(),
            vec![kept.id]
        );
        let record = approvals.get(refused.id).unwrap().record;
        assert_eq!(
            (record.decision, record.status),
            (Some(Decision::Refused), Status::Refused)
        );
        assert_eq!(record.error.as_deref(), Some(REFUSED_READ_ONLY));
        assert_eq!(
            approvals.end_account(other, Answer::Lapsed(LAPSED_ACCOUNT_CHANGED.into()), 4),
            1
        );
        let record = approvals.get(kept.id).unwrap().record;
        assert_eq!(
            (record.decision, record.status),
            (Some(Decision::Lapsed), Status::Lapsed)
        );
    }

    #[test]
    fn a_change_without_approval_is_recorded_as_auto() {
        let approvals = Approvals::default();
        let id = approvals.record_auto(ask(Uuid::new_v4(), "Zonder akkoord"), technical(), 7);
        assert_eq!(approvals.count(), 0, "nothing waits");
        let record = approvals.get(id).unwrap().record;
        assert_eq!(record.decision, Some(Decision::Auto));
        assert_eq!(record.status, Status::Running);
        assert_eq!(record.decided_at, Some(7));
        approvals.finish(id, Err("Niet ingelogd bij Studio Noord.".into()), 8);
        let record = approvals.get(id).unwrap().record;
        assert_eq!(record.status, Status::Failed);
        assert_eq!(
            record.error.as_deref(),
            Some("Niet ingelogd bij Studio Noord.")
        );
    }

    #[test]
    fn the_history_round_trips_and_lists_newest_first_without_technical_data() {
        let dir = TempDir::new();
        let file = history_file(&dir.0);
        let approvals = Approvals::default();
        approvals.open(file.clone());
        let auto = approvals.record_auto(ask(Uuid::new_v4(), "Eerst"), technical(), 1);
        approvals.finish(auto, Ok(&json!({"gedaan": "ja"})), 2);
        let (refused, _r) = approvals.ask(ask(Uuid::new_v4(), "Daarna"), technical(), 3);
        approvals.decide(refused.id, false, 4);
        assert!(approvals.dirty());
        assert!(approvals.save().unwrap());
        assert!(!approvals.dirty());
        assert!(
            !approvals.save().unwrap(),
            "nothing changed, nothing written"
        );

        let again = Approvals::default();
        again.open(file.clone());
        assert!(!again.dirty());
        let listed = again.history(None);
        assert_eq!(
            listed.iter().map(|r| r.proposal.id).collect::<Vec<_>>(),
            vec![refused.id, auto]
        );
        let shown = serde_json::to_value(&listed[1]).unwrap();
        assert_eq!(shown["title"], "Eerst");
        assert_eq!(shown["decision"], "auto");
        assert_eq!(shown["status"], "done");
        assert_eq!(shown["version"], APP_VERSION);
        assert_eq!(shown["decidedAt"], 1);
        assert_eq!(shown["bulk"], false);
        for technical in ["args", "body", "result", "logId"] {
            assert!(shown.get(technical).is_none(), "{technical} is not listed");
        }
        let detail = serde_json::to_value(again.get(auto).unwrap()).unwrap();
        assert_eq!(
            detail["body"],
            json!({"mutatie": {"omschrijving": "Parkeergarage"}})
        );
        assert_eq!(detail["args"]["omschrijving"], "Parkeergarage");
        assert_eq!(detail["result"], json!({"gedaan": "ja"}));
        assert_eq!(detail["logId"], 4);
        assert_eq!(detail["accountId"].as_str().unwrap().len(), 36);
        assert_eq!(again.history(Some(1)).len(), 1);
        // New ids follow the old ones.
        assert!(again.record_auto(ask(Uuid::new_v4(), "Nieuw"), technical(), 5) > refused.id);
    }

    #[test]
    fn what_waited_or_ran_at_the_stop_comes_back_lapsed() {
        let dir = TempDir::new();
        let file = history_file(&dir.0);
        let approvals = Approvals::default();
        approvals.open(file.clone());
        let (waiting, _r) = approvals.ask(ask(Uuid::new_v4(), "Wacht"), technical(), 1);
        let running = approvals.record_auto(ask(Uuid::new_v4(), "Loopt"), technical(), 2);
        approvals.save().unwrap();

        let again = Approvals::default();
        again.open(file.clone());
        assert_eq!(again.count(), 0, "nothing waits after a restart");
        assert!(
            !again.decide(waiting.id, true, 9),
            "a lapsed one cannot be approved"
        );
        let w = again.get(waiting.id).unwrap().record;
        assert_eq!(
            (w.decision, w.status),
            (Some(Decision::Lapsed), Status::Lapsed)
        );
        assert_eq!(w.error.as_deref(), Some(LAPSED_RESTART));
        let r = again.get(running).unwrap().record;
        assert_eq!(
            (r.decision, r.status),
            (Some(Decision::Auto), Status::Lapsed)
        );
        assert_eq!(r.error.as_deref(), Some(LAPSED_WHILE_RUNNING));
        assert!(again.dirty(), "written back at the next save");
        again.save().unwrap();
        let third = Approvals::default();
        third.open(file);
        assert!(!third.dirty(), "lapsed once, not again");
    }

    #[test]
    fn a_damaged_or_foreign_file_is_set_aside_and_the_history_starts_empty() {
        let dir = TempDir::new();
        let file = history_file(&dir.0);
        std::fs::write(&file, b"geen gzip").unwrap();
        let approvals = Approvals::default();
        approvals.open(file.clone());
        assert!(approvals.history(None).is_empty());
        let aside = file.with_extension("gz.damaged");
        assert_eq!(std::fs::read(&aside).unwrap(), b"geen gzip");
        approvals.record_auto(ask(Uuid::new_v4(), "Na de schade"), technical(), 1);
        assert!(approvals.save().unwrap());
        assert_eq!(Approvals::default().history(None).len(), 0);
        let again = Approvals::default();
        again.open(file.clone());
        assert_eq!(again.history(None).len(), 1);

        logstore::write_gz(&file, &json!({"format": 99, "records": []})).unwrap();
        let foreign = Approvals::default();
        foreign.open(file.clone());
        assert!(foreign.history(None).is_empty());
        assert!(file.with_extension("gz.damaged").exists());
    }

    #[cfg(unix)]
    #[test]
    fn the_history_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new();
        let file = history_file(&dir.0);
        let approvals = Approvals::default();
        approvals.open(file.clone());
        approvals.record_auto(ask(Uuid::new_v4(), "Prive"), technical(), 1);
        approvals.save().unwrap();
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let expected = if cfg!(debug_assertions) {
            "dev-approvals.json.gz"
        } else {
            "approvals.json.gz"
        };
        assert_eq!(file.file_name().unwrap(), expected);
    }

    #[tokio::test]
    async fn keeps_the_last_two_thousand_but_never_drops_what_waits() {
        let approvals = Approvals::default();
        let account = Uuid::new_v4();
        let (first, _receiver) = approvals.ask(ask(account, "Wacht nog"), technical(), 0);
        for n in 0..CAPACITY + 10 {
            let id = approvals.record_auto(ask(account, &format!("Nr {n}")), technical(), 1);
            approvals.finish(id, Ok(&Value::Null), 2);
        }
        let history = approvals.history(None);
        assert_eq!(history.len(), CAPACITY);
        assert!(approvals.get(first.id).is_some(), "the waiting one stays");
        assert_eq!(approvals.list().len(), 1);
        assert!(approvals.decide(first.id, true, 3));
        // The oldest finished ones went first.
        assert!(approvals.get(first.id + 1).is_none());
        assert_eq!(history[0].proposal.title, format!("Nr {}", CAPACITY + 9));
    }

    #[test]
    fn secrets_are_never_kept_and_the_export_masks_who_signs_in() {
        let approvals = Approvals::default();
        let id = approvals.record_auto(
            ask(Uuid::new_v4(), "Opmaak"),
            Technical {
                args: json!({"opmaak": {"sessionHash": "abc123", "elements": []}, "Password": "hunter2", "nested": [{"auth-token": "jwt"}]}),
                body: json!({"cookie": "auth-token=jwt", "naam": "jan.devries@example.nl"}),
                log_id: None,
            },
            1,
        );
        approvals.finish(id, Ok(&json!({"sessionKey": "geheim", "id": 9})), 2);
        let kept = serde_json::to_string(&approvals.get(id).unwrap()).unwrap();
        for secret in ["abc123", "hunter2", "jwt", "geheim"] {
            assert!(!kept.contains(secret), "{secret} is kept in {kept}");
        }
        assert!(kept.contains(MASK));

        let dir = TempDir::new();
        let at = DateTime::parse_from_rfc3339("2026-11-11T14:03:22+01:00")
            .unwrap()
            .with_timezone(&Local);
        let hidden = [
            "jan.devries@example.nl".to_owned(),
            "op://Prive/e-Boekhouden".to_owned(),
        ];
        let path = approvals.export(&dir.0, &hidden, &at).unwrap();
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("eb-mcp-akkoord-") && name.ends_with(".jsonl.gz"));
        assert!(logstore::is_export(&path));
        let mut text = String::new();
        GzDecoder::new(std::fs::File::open(&path).unwrap())
            .read_to_string(&mut text)
            .unwrap();
        let lines: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["type"], "header");
        assert_eq!(lines[0]["counts"]["records"], 1);
        assert_eq!(lines[1]["type"], "record");
        assert_eq!(lines[1]["result"]["id"], 9);
        assert!(!text.contains("jan.devries"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
