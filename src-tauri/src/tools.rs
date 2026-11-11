//! The read-only MCP tools. Each one resolves the account (the default one
//! unless the client names another), calls e-Boekhouden's API in that
//! session, and logs itself with the calls it caused. Nothing here books,
//! saves or deletes.
//!
//! Query parameter names come from e-Boekhouden's own Angular app (the public
//! bundle): grids take offset, limit, orderBy and sortDirection plus filters.

use std::sync::{Arc, LazyLock};

use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, Icon, Implementation, ServerCapabilities, ServerConfig},
    schemars::{self, JsonSchema},
    service::RequestContext,
    tool, tool_handler, tool_router,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::accounts::{Account, WriteMode};
use crate::approvals::{self, Ask, line};
use crate::reqlog::Outcome;
use crate::state::{AppEvent, AppState};
use crate::write::{self, Books, Built};

const MAX_ROWS: u32 = 200;
const GET_PATHS: &str = include_str!("../data/get-paths.txt");

static ALLOWED: LazyLock<Vec<Vec<String>>> = LazyLock::new(|| {
    GET_PATHS
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split('/').map(|s| s.to_ascii_lowercase()).collect())
        .collect()
});

/// Whether `path` is a plain read on the API map. `{}` stands for one id-like segment.
pub fn allowed_path(path: &str) -> bool {
    let path = path.trim().trim_matches('/');
    if path.is_empty()
        || path.contains("..")
        || path.contains('?')
        || path.contains('#')
        || path.contains('\\')
    {
        return false;
    }
    let parts: Vec<String> = path.split('/').map(|s| s.to_ascii_lowercase()).collect();
    let segment_ok = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    if !parts.iter().all(|p| segment_ok(p)) {
        return false;
    }
    ALLOWED.iter().any(|pattern| {
        pattern.len() == parts.len()
            && pattern
                .iter()
                .zip(&parts)
                .all(|(want, got)| want == "{}" || want == got)
    })
}

/// "2026-10-31" -> (2026, 10, 31), only real dates.
pub fn parse_date(text: &str) -> Result<(i32, u32, u32), String> {
    let bad = || format!("Ongeldige datum {text:?}: gebruik JJJJ-MM-DD");
    let mut parts = text.trim().split('-');
    let (Some(y), Some(m), Some(d), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(bad());
    };
    let (y, m, d): (i32, u32, u32) = (
        y.parse().map_err(|_| bad())?,
        m.parse().map_err(|_| bad())?,
        d.parse().map_err(|_| bad())?,
    );
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if !(1900..=2100).contains(&y) || !(1..=12).contains(&m) || d == 0 || d > days[(m - 1) as usize]
    {
        return Err(bad());
    }
    Ok((y, m, d))
}

/// Today on this computer's calendar, so default periods turn over at local midnight, not at UTC's.
pub(crate) fn today() -> (i32, u32, u32) {
    let now = chrono::Local::now();
    date_at(now.timestamp_millis(), now.offset().local_minus_utc())
}

/// The calendar date at `ms` since the epoch, in a zone `offset_secs` ahead of UTC.
fn date_at(ms: i64, offset_secs: i32) -> (i32, u32, u32) {
    let days = (ms + i64::from(offset_secs) * 1000).div_euclid(86_400_000);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

fn compact((y, m, d): (i32, u32, u32)) -> String {
    format!("{y:04}{m:02}{d:02}")
}

pub(crate) fn dashed((y, m, d): (i32, u32, u32)) -> String {
    format!("{y:04}-{m:02}-{d:02}")
}

fn grid(limit: Option<u32>, offset: Option<u32>) -> Result<Vec<(String, String)>, String> {
    let limit = limit.unwrap_or(50);
    if limit == 0 || limit > MAX_ROWS {
        return Err(format!("limit moet tussen 1 en {MAX_ROWS} liggen"));
    }
    Ok(vec![
        ("offset".into(), offset.unwrap_or(0).to_string()),
        ("limit".into(), limit.to_string()),
        ("orderBy".into(), String::new()),
        ("sortDirection".into(), String::new()),
    ])
}

/// The query for `import/gridtable`, with the filters of e-Boekhouden's own grids: `search<KIND><column>`.
/// Checked against a real administration on 2026-11-11: `BETWE` takes ISO dates and includes both ends,
/// `CONTA` matches part of the text. A date outside the calendar never goes out (the API answers 500 on one).
fn bankregels_query(args: &BankregelsArgs) -> Result<Vec<(String, String)>, String> {
    let mut query = grid(args.limit, args.offset)?;
    let van = args.van.as_deref().map(parse_date).transpose()?;
    let tot = args.tot.as_deref().map(parse_date).transpose()?;
    if let (Some(van), Some(tot)) = (van, tot)
        && van > tot
    {
        return Err("van ligt na tot".into());
    }
    if van.is_some() || tot.is_some() {
        // The grid has no open-ended date filter that includes its bound, so a missing end becomes a far one.
        let van = van.map(dashed).unwrap_or_else(|| "1900-01-01".into());
        let tot = tot.map(dashed).unwrap_or_else(|| "9999-12-31".into());
        query.push(("searchBETWEmutDatum".into(), format!("{van}<->{tot}")));
    }
    push(
        &mut query,
        "searchCONTAmutOmschrijving",
        args.omschrijving.as_ref(),
    );
    Ok(query)
}

/// A grid answer with its rows as objects. e-Boekhouden sends each row as a list of values and the column
/// names apart (`colMetadata`); here every row carries its names, so nobody has to count columns.
/// An answer of another shape comes back unchanged.
fn named_rows(mut grid: Value) -> Value {
    let names: Option<Vec<(usize, String)>> = grid
        .get("colMetadata")
        .and_then(Value::as_array)
        .map(|columns| {
            columns
                .iter()
                .filter_map(|column| {
                    let index = column
                        .get("readerIndex")
                        .or(column.get("index"))?
                        .as_u64()?;
                    Some((index as usize, column.get("name")?.as_str()?.to_owned()))
                })
                .collect()
        });
    let Some(names) = names.filter(|names| !names.is_empty()) else {
        return grid;
    };
    let Some(rows) = grid.get_mut("data").and_then(Value::as_array_mut) else {
        return grid;
    };
    if !rows.iter().all(Value::is_array) {
        return grid;
    }
    for row in rows.iter_mut() {
        let values = row.as_array().cloned().unwrap_or_default();
        let named: serde_json::Map<String, Value> = names
            .iter()
            .map(|(index, name)| {
                (
                    name.clone(),
                    values.get(*index).cloned().unwrap_or(Value::Null),
                )
            })
            .collect();
        *row = Value::Object(named);
    }
    if let Some(object) = grid.as_object_mut() {
        object.remove("colMetadata");
    }
    grid
}

fn push(query: &mut Vec<(String, String)>, key: &str, value: Option<impl ToString>) {
    if let Some(value) = value {
        let value = value.to_string();
        if !value.trim().is_empty() {
            query.push((key.to_owned(), value.trim().to_owned()));
        }
    }
}

// ---------------------------------------------------------------- parameters

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct RelatiesArgs {
    /// Zoekterm op naam, code, plaats of e-mail.
    pub zoek: Option<String>,
    /// Aantal rijen, 1 tot 200 (standaard 50).
    pub limit: Option<u32>,
    /// Vanaf welke rij (standaard 0).
    pub offset: Option<u32>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct IdArgs {
    /// Het id uit een lijst van e-Boekhouden.
    pub id: u64,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct MutatiesArgs {
    /// Vanaf datum, JJJJ-MM-DD.
    pub van: Option<String>,
    /// Tot en met datum, JJJJ-MM-DD.
    pub tot: Option<String>,
    /// Alleen mutaties van deze relatie (id).
    pub relatie_id: Option<u64>,
    /// Zoek in de omschrijving.
    pub omschrijving: Option<String>,
    /// Factuurnummer.
    pub factuur: Option<String>,
    /// Aantal rijen, 1 tot 200 (standaard 50).
    pub limit: Option<u32>,
    /// Vanaf welke rij (standaard 0).
    pub offset: Option<u32>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct BankregelsArgs {
    /// Vanaf boekdatum van de bankregel, JJJJ-MM-DD.
    pub van: Option<String>,
    /// Tot en met boekdatum van de bankregel, JJJJ-MM-DD.
    pub tot: Option<String>,
    /// Zoek in de omschrijving; daar staan ook de naam en het IBAN van de tegenpartij in.
    pub omschrijving: Option<String>,
    /// Aantal rijen, 1 tot 200 (standaard 50).
    pub limit: Option<u32>,
    /// Vanaf welke rij (standaard 0).
    pub offset: Option<u32>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct MutatieArgs {
    /// Mutatienummer.
    pub nr: u64,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ArchiefUploadenArgs {
    /// Volledig pad van het bestand op deze computer, bijvoorbeeld /Users/naam/Downloads/bon.pdf. Hoogstens 5 MB.
    pub pad: String,
    /// Naam in het archief (standaard de bestandsnaam).
    pub naam: Option<String>,
    /// Id van de map in het digitaal archief (standaard de Basismap).
    pub map_id: Option<u64>,
    /// Mutatienummer om het bestand meteen aan te koppelen.
    pub mutatie_nr: Option<u64>,
    /// Een bestaand bestand met dezelfde naam overschrijven (standaard niet).
    pub overschrijven: Option<bool>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ArchiefKoppelenArgs {
    /// Id van het bestand in het digitaal archief (uit digitaal_archief).
    pub bestand_id: u64,
    /// Mutatienummer waar het bestand bij hoort.
    pub mutatie_nr: u64,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Soort {
    Debiteuren,
    Crediteuren,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct OpenPostenArgs {
    /// debiteuren (te ontvangen, standaard) of crediteuren (te betalen).
    pub soort: Option<Soort>,
    /// Alleen open posten van deze relatie (id).
    pub relatie_id: Option<u64>,
    /// Alleen posten waarvan de vervaldatum voorbij is.
    pub alleen_verlopen: Option<bool>,
    /// Aantal rijen, 1 tot 200 (standaard 50).
    pub limit: Option<u32>,
    /// Vanaf welke rij (standaard 0).
    pub offset: Option<u32>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ListArgs {
    /// Aantal rijen, 1 tot 200 (standaard 50).
    pub limit: Option<u32>,
    /// Vanaf welke rij (standaard 0).
    pub offset: Option<u32>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct BalansArgs {
    /// Stand per datum, JJJJ-MM-DD (standaard vandaag).
    pub datum: Option<String>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct PeriodeArgs {
    /// Begin van de periode, JJJJ-MM-DD (standaard 1 januari van dit jaar).
    pub van: Option<String>,
    /// Einde van de periode, JJJJ-MM-DD (standaard vandaag).
    pub tot: Option<String>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PeriodeType {
    Maand,
    Kwartaal,
    Jaar,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct BtwArgs {
    /// Jaar van de aangifte.
    pub jaar: i32,
    /// maand, kwartaal (standaard) of jaar.
    pub periode_type: Option<PeriodeType>,
    /// Nummer van de maand (1 tot 12) of het kwartaal (1 tot 4). Niet nodig bij jaar.
    pub periode: Option<u32>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct UrenArgs {
    /// Begin van de periode, JJJJ-MM-DD (standaard de eerste van deze maand).
    pub van: Option<String>,
    /// Einde van de periode, JJJJ-MM-DD (standaard vandaag).
    pub tot: Option<String>,
    /// Alleen dit project (id).
    pub project_id: Option<u64>,
    /// Alleen deze activiteit (id).
    pub activiteit_id: Option<u64>,
    /// Aantal rijen, 1 tot 200 (standaard 50).
    pub limit: Option<u32>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ArchiefArgs {
    /// Id van een map. Leeg: de lijst met mappen.
    pub map_id: Option<u64>,
    /// Aantal bestanden, 1 tot 200 (standaard 50).
    pub limit: Option<u32>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetArgs {
    /// Pad onder /v1/api/, bijvoorbeeld "relatie/gridtable". Alleen leesbare paden van de API-kaart.
    pub pad: String,
    /// Queryparameters als sleutel en waarde.
    pub query: Option<std::collections::BTreeMap<String, String>>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

// ---------------------------------------------------------------- the server

#[derive(Clone)]
pub struct EbMcp {
    state: Arc<AppState>,
    #[expect(dead_code, reason = "read by the tool_handler macro")]
    tool_router: ToolRouter<Self>,
}

/// One tool call in progress: its log entry, its client and its account.
struct Run {
    id: u64,
    client: String,
    account: Account,
}

fn client_name(context: &RequestContext<RoleServer>) -> String {
    context
        .peer
        .peer_info()
        .map(|info| {
            info.client_info
                .title
                .clone()
                .unwrap_or_else(|| info.client_info.name.clone())
        })
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "Onbekende client".into())
}

/// The account a tool call is about. There is no default account: with several accounts a call must name
/// its own, so an AI client never answers from the wrong books. A single account needs no name.
fn pick_account(mut accounts: Vec<Account>, name: Option<&str>) -> Result<Account, String> {
    let labels = || {
        accounts
            .iter()
            .map(|a| a.label.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    match name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => accounts
            .iter()
            .find(|a| a.label.eq_ignore_ascii_case(name) || a.id.to_string() == name)
            .cloned()
            .ok_or_else(|| match accounts.len() {
                0 => {
                    "Er staat nog geen account in EB MCP. Voeg er een toe via Accounts.".to_owned()
                }
                _ => format!("Geen account {name:?} in EB MCP. Kies uit: {}.", labels()),
            }),
        None => match accounts.len() {
            0 => {
                Err("Er staat nog geen account in EB MCP. Voeg er een toe via Accounts.".to_owned())
            }
            1 => Ok(accounts.remove(0)),
            count => Err(format!(
                "Er staan {count} accounts in EB MCP. Geef met de parameter account aan welke je bedoelt: {}.",
                labels()
            )),
        },
    }
}

impl EbMcp {
    /// Logs the call and finds the account; a refusal is logged and returned as the tool's answer.
    fn begin(
        &self,
        context: &RequestContext<RoleServer>,
        tool: &str,
        args: &impl Serialize,
        account: Option<&str>,
    ) -> Result<Run, CallToolResult> {
        let client = client_name(context);
        let args = serde_json::to_value(args).unwrap_or(Value::Null);
        if self.state.is_updating() {
            let message =
                "EB MCP wordt bijgewerkt en start zo opnieuw. Probeer het over een minuut weer."
                    .to_owned();
            let id = self.state.log.start_mcp(&client, tool, args, None);
            self.state
                .log
                .finish_mcp(id, Outcome::Refused, None, Some(message.clone()));
            let _ = self.state.events.send(AppEvent::LogChanged);
            return Err(CallToolResult::error(vec![ContentBlock::text(message)]));
        }
        let account = match pick_account(self.state.accounts.list(), account) {
            Ok(account) => account,
            Err(message) => {
                let id = self.state.log.start_mcp(&client, tool, args, None);
                self.state
                    .log
                    .finish_mcp(id, Outcome::Error, None, Some(message.clone()));
                let _ = self.state.events.send(AppEvent::LogChanged);
                return Err(CallToolResult::error(vec![ContentBlock::text(message)]));
            }
        };
        let id = self
            .state
            .log
            .start_mcp(&client, tool, args, Some(account.label.clone()));
        let _ = self.state.events.send(AppEvent::LogChanged);
        Ok(Run {
            id,
            client,
            account,
        })
    }

    async fn fetch(
        &self,
        run: &Run,
        path: &str,
        query: Vec<(String, String)>,
    ) -> Result<Value, String> {
        let call = self
            .state
            .api_get(run.account.id, path, &query, Some(run.id), &run.client)
            .await;
        match (call.ok(), call.json) {
            (true, Some(json)) => Ok(json),
            _ => Err(call
                .error
                .unwrap_or_else(|| format!("e-Boekhouden gaf {} op {path}", call.status))),
        }
    }

    fn end(&self, run: Run, source: &str, result: Result<Value, String>) -> CallToolResult {
        let answer = match result {
            Ok(data) => {
                let rows = crate::ebapi::count_rows(&data);
                let value = json!({ "account": run.account.label, "administratie": run.account.administration, "bron": source, "rijen": rows, "data": data });
                self.state
                    .log
                    .finish_mcp(run.id, Outcome::Ok, Some(&value), None);
                CallToolResult::success(vec![ContentBlock::text(value.to_string())])
            }
            Err(message) => {
                self.state
                    .log
                    .finish_mcp(run.id, Outcome::Error, None, Some(message.clone()));
                CallToolResult::error(vec![ContentBlock::text(message)])
            }
        };
        let _ = self.state.events.send(AppEvent::LogChanged);
        answer
    }

    fn refuse(&self, run: Run, message: String) -> CallToolResult {
        self.state
            .log
            .finish_mcp(run.id, Outcome::Refused, None, Some(message.clone()));
        let _ = self.state.events.send(AppEvent::LogChanged);
        CallToolResult::error(vec![ContentBlock::text(message)])
    }

    /// The usual shape: begin, one GET, end. Bad input is refused before anything goes out.
    async fn simple(
        &self,
        context: &RequestContext<RoleServer>,
        tool: &str,
        args: &impl Serialize,
        account: Option<&str>,
        path: &str,
        query: Result<Vec<(String, String)>, String>,
    ) -> Result<CallToolResult, McpError> {
        let run = match self.begin(context, tool, args, account) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let query = match query {
            Ok(q) => q,
            Err(message) => return Ok(self.refuse(run, message)),
        };
        let result = self.fetch(&run, path, query).await;
        Ok(self.end(run, path, result))
    }
}

/// Why a tool that changes the books stopped before or at its change.
enum Halt {
    /// The user said no, or the account does not allow it: nothing went to e-Boekhouden.
    Refused(String),
    /// Wrong input, or e-Boekhouden refused or failed.
    Error(String),
}

impl From<String> for Halt {
    fn from(message: String) -> Self {
        Self::Error(message)
    }
}

/// Takes a proposal down when its tool call ends without an answer (the AI client gave up, or the wait ran out).
struct Withdraw<'a> {
    state: &'a AppState,
    id: u64,
}

impl Drop for Withdraw<'_> {
    fn drop(&mut self) {
        self.state.approvals.withdraw(self.id);
        let _ = self.state.events.send(AppEvent::ApprovalsChanged);
    }
}

/// What a tool args struct says about writing, for the user who reads `status`.
fn write_mode_name(mode: WriteMode) -> &'static str {
    match mode {
        WriteMode::Off => "uit",
        WriteMode::Ask => "met akkoord",
        WriteMode::Auto => "zonder akkoord",
    }
}

/// A file from this computer for the digitaal archief: its name and its bytes in base64, as `folder/upload` takes them.
fn read_upload(path: &str, name: Option<&str>) -> Result<(String, String, u64), String> {
    use base64::Engine;
    /// The web app's own limit.
    const MAX: u64 = 5120 * 1024;
    let path = std::path::Path::new(path.trim());
    if !path.is_absolute() {
        return Err("Geef het volledige pad van het bestand op deze computer".into());
    }
    let meta = std::fs::metadata(path)
        .map_err(|_| format!("Geen bestand gevonden op {}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is geen bestand", path.display()));
    }
    if meta.len() == 0 {
        return Err("Het bestand is leeg".into());
    }
    if meta.len() > MAX {
        return Err(
            "Het bestand is groter dan 5 MB; dat neemt het digitaal archief niet aan".into(),
        );
    }
    let file_name = name
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_owned)
        .or_else(|| path.file_name().map(|n| n.to_string_lossy().into_owned()))
        .ok_or("Het bestand heeft geen naam")?;
    if file_name.chars().count() > 195 || file_name.contains(['/', '\\']) {
        return Err("De bestandsnaam is te lang of bevat een schuine streep".into());
    }
    let bytes = std::fs::read(path).map_err(|_| format!("{} is niet te lezen", path.display()))?;
    Ok((
        file_name,
        base64::engine::general_purpose::STANDARD.encode(bytes),
        meta.len(),
    ))
}

impl EbMcp {
    /// Starts a tool that changes the books. The account must allow that; reading alone is not enough.
    fn begin_write(
        &self,
        context: &RequestContext<RoleServer>,
        tool: &str,
        args: &impl Serialize,
        account: Option<&str>,
    ) -> Result<Run, CallToolResult> {
        let run = self.begin(context, tool, args, account)?;
        if run.account.write == WriteMode::Off {
            let message = format!(
                "{} is alleen-lezen in EB MCP: er is niets gewijzigd. De gebruiker kan schrijven aanzetten in EB MCP onder Accounts, bij het bewerken van het account.",
                run.account.label
            );
            return Err(self.refuse(run, message));
        }
        Ok(run)
    }

    /// The rekeningschema and btw codes of the account's administration.
    async fn books(&self, run: &Run) -> Result<Books, String> {
        let all = vec![
            ("offset".to_owned(), "0".to_owned()),
            ("limit".to_owned(), "2000".to_owned()),
            ("orderBy".to_owned(), String::new()),
            ("sortDirection".to_owned(), String::new()),
        ];
        let grootboek = self.fetch(run, "grootboekrekening/gridtable", all).await?;
        let btw = self.fetch(run, "btwinfo/all", Vec::new()).await?;
        Books::from_api(&grootboek, &btw)
    }

    /// Puts the change in front of the user and waits for the answer. An account that needs no approval skips
    /// this, except for `danger` (deleting, crediting, invoicing in bulk): those always ask.
    async fn approve(
        &self,
        run: &Run,
        tool: &str,
        built: &Built,
        danger: bool,
    ) -> Result<(), Halt> {
        if run.account.write == WriteMode::Auto && !danger {
            return Ok(());
        }
        let (proposal, answer) = self.state.approvals.ask(
            Ask {
                account_id: run.account.id,
                account: run.account.label.clone(),
                client: run.client.clone(),
                tool: tool.to_owned(),
                title: built.title.clone(),
                lines: built.lines.clone(),
                danger,
            },
            crate::state::now_ms(),
        );
        let _ = self.state.events.send(AppEvent::ApprovalsChanged);
        let _withdraw = Withdraw {
            state: &self.state,
            id: proposal.id,
        };
        match tokio::time::timeout(approvals::WAIT, answer).await {
            Ok(Ok(true)) => Ok(()),
            Ok(Ok(false)) => Err(Halt::Refused(
                "De gebruiker heeft dit geweigerd in EB MCP. Er is niets gewijzigd.".into(),
            )),
            Ok(Err(_)) => Err(Halt::Refused("Het voorstel is ingetrokken. Er is niets gewijzigd.".into())),
            Err(_) => Err(Halt::Refused(
                "De gebruiker heeft niet binnen 5 minuten gereageerd in EB MCP. Er is niets gewijzigd.".into(),
            )),
        }
    }

    /// One change on e-Boekhouden, after the approval. `Null` when it answers without a body.
    async fn change(
        &self,
        run: &Run,
        method: &'static str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, Halt> {
        let call = self
            .state
            .api_send(
                run.account.id,
                method,
                path,
                &[],
                body,
                Some(run.id),
                &run.client,
            )
            .await;
        if call.ok() {
            return Ok(call.json.unwrap_or(Value::Null));
        }
        Err(Halt::Error(call.error.unwrap_or_else(|| {
            format!("e-Boekhouden gaf {} op {path}", call.status)
        })))
    }

    /// A change that e-Boekhouden answers with `true` when it did it.
    async fn change_done(
        &self,
        run: &Run,
        method: &'static str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, Halt> {
        match self.change(run, method, path, body).await? {
            Value::Bool(false) => Err(Halt::Error(
                "e-Boekhouden meldt dat dit niet gelukt is".into(),
            )),
            done => Ok(done),
        }
    }

    fn end_write(&self, run: Run, done: &str, result: Result<Value, Halt>) -> CallToolResult {
        let answer = match result {
            Ok(data) => {
                let value = json!({ "account": run.account.label, "administratie": run.account.administration, "gedaan": done, "data": data });
                self.state
                    .log
                    .finish_mcp(run.id, Outcome::Ok, Some(&value), None);
                CallToolResult::success(vec![ContentBlock::text(value.to_string())])
            }
            Err(halt) => {
                let (outcome, message) = match halt {
                    Halt::Refused(message) => (Outcome::Refused, message),
                    Halt::Error(message) => (Outcome::Error, message),
                };
                self.state
                    .log
                    .finish_mcp(run.id, outcome, None, Some(message.clone()));
                CallToolResult::error(vec![ContentBlock::text(message)])
            }
        };
        let _ = self.state.events.send(AppEvent::LogChanged);
        answer
    }

    /// What a new invoice takes from the administration: its next number, its sjabloon and whether it books at once.
    async fn factuur_context(&self, run: &Run) -> Result<write::FactuurContext, String> {
        let nummer = self
            .fetch(run, "factuur/nextfactuurnummer", Vec::new())
            .await?;
        let setting = self.fetch(run, "factuursetting", Vec::new()).await?;
        Ok(write::FactuurContext {
            volgend_nummer: nummer.as_str().unwrap_or_default().to_owned(),
            sjabloon_id: setting
                .get("factuurSjabloonId")
                .and_then(Value::as_u64)
                .ok_or("Deze administratie heeft geen factuursjabloon ingesteld")?,
            verwerken: setting
                .get("verwerken")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            vandaag: today(),
        })
    }

    /// An invoice or subscription with its lines, as the tools that read or change one need it.
    async fn with_items(&self, run: &Run, path: &str, id: u64) -> Result<(Value, Value), String> {
        let now = self.fetch(run, &format!("{path}/{id}"), Vec::new()).await?;
        let items = self
            .fetch(
                run,
                "factuuritem/griditems",
                vec![("factid".to_owned(), id.to_string())],
            )
            .await?;
        Ok((now, items))
    }
}

/// How many tools the server offers, counted on its tool router.
pub fn tool_count() -> usize {
    EbMcp::tool_router().list_all().len()
}

#[tool_router]
impl EbMcp {
    pub fn new(state: Arc<AppState>) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Welke e-Boekhouden-accounts er in EB MCP staan, welke is ingelogd en of er per account geboekt mag worden (schrijven: uit, met akkoord of zonder akkoord). Gebruik dit eerst als er meer dan één account kan zijn, of als een andere tool zegt dat er niemand is ingelogd.",
        annotations(title = "Status", read_only_hint = true, open_world_hint = false)
    )]
    async fn status(&self) -> Result<CallToolResult, McpError> {
        let sessions = self.state.sessions();
        let can_write = self
            .state
            .accounts
            .list()
            .iter()
            .any(|a| a.write != WriteMode::Off);
        let accounts: Vec<Value> = self
            .state
            .accounts
            .list()
            .into_iter()
            .map(|a| {
                let session = sessions.iter().find(|s| s.account_id == a.id);
                json!({
                    "account": a.label,
                    "administratie": a.administration,
                    "schrijven": write_mode_name(a.write),
                    "ingelogd": session.is_some_and(|s| !s.expired),
                    "verlopen": session.is_some_and(|s| s.expired),
                })
            })
            .collect();
        let value = json!({
            "app": "EB MCP",
            "versie": env!("CARGO_PKG_VERSION"),
            "alleen_lezen": !can_write,
            "accounts": accounts,
            "let_op": if self.state.session_count() == 0 {
                "Nog niemand ingelogd. Vraag de gebruiker in te loggen via EB MCP in de menubalk."
            } else if can_write {
                "Per account staat bij schrijven of er geboekt mag worden: uit (alleen lezen), met akkoord (de gebruiker keurt elke wijziging goed in EB MCP) of zonder akkoord. Verwijderen, crediteren en abonnementen factureren vragen altijd akkoord."
            } else {
                "Alleen lezen: schrijven staat voor elk account uit. De gebruiker kan het per account aanzetten in EB MCP onder Accounts."
            },
        });
        Ok(CallToolResult::success(vec![ContentBlock::text(
            value.to_string(),
        )]))
    }

    #[tool(
        description = "Relaties (klanten en leveranciers) zoeken of opsommen.",
        annotations(title = "Relaties", read_only_hint = true)
    )]
    async fn relaties(
        &self,
        Parameters(args): Parameters<RelatiesArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let query = grid(args.limit, args.offset).map(|mut q| {
            push(&mut q, "searchTerm", args.zoek.as_ref());
            q
        });
        self.simple(
            &context,
            "relaties",
            &args,
            args.account.as_deref(),
            "relatie/gridtable",
            query,
        )
        .await
    }

    #[tool(
        description = "Alle gegevens van één relatie, op id.",
        annotations(title = "Relatie", read_only_hint = true)
    )]
    async fn relatie(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let path = format!("relatie/{}", args.id);
        self.simple(
            &context,
            "relatie",
            &args,
            args.account.as_deref(),
            &path,
            Ok(vec![]),
        )
        .await
    }

    #[tool(
        description = "Mutaties (boekingen) zoeken, op periode, relatie, omschrijving of factuurnummer.",
        annotations(title = "Mutaties", read_only_hint = true)
    )]
    async fn mutaties(
        &self,
        Parameters(args): Parameters<MutatiesArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let query = (|| {
            let mut q = grid(args.limit, args.offset)?;
            push(
                &mut q,
                "datumVan",
                args.van.as_deref().map(parse_date).transpose()?.map(dashed),
            );
            push(
                &mut q,
                "datumTot",
                args.tot.as_deref().map(parse_date).transpose()?.map(dashed),
            );
            push(&mut q, "relatieId", args.relatie_id);
            push(&mut q, "omschrijving", args.omschrijving.as_ref());
            push(&mut q, "factuur", args.factuur.as_ref());
            Ok(q)
        })();
        self.simple(
            &context,
            "mutaties",
            &args,
            args.account.as_deref(),
            "mutatie/gridtable",
            query,
        )
        .await
    }

    #[tool(
        description = "Open posten: facturen die nog betaald moeten worden, van debiteuren of crediteuren.",
        annotations(title = "Open posten", read_only_hint = true)
    )]
    async fn open_posten(
        &self,
        Parameters(args): Parameters<OpenPostenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let query = grid(args.limit, args.offset).map(|mut q| {
            let soort = match args.soort {
                Some(Soort::Crediteuren) => "Crediteur",
                _ => "Debiteur",
            };
            q.push(("soort".into(), soort.into()));
            q.push(("zoeken".into(), "true".into()));
            push(&mut q, "relatieId", args.relatie_id);
            if args.alleen_verlopen == Some(true) {
                q.push(("expiredOnly".into(), "true".into()));
            }
            q
        });
        self.simple(
            &context,
            "open_posten",
            &args,
            args.account.as_deref(),
            "openpost/gridtable/openpost",
            query,
        )
        .await
    }

    // The list behind Boekhouden > Importeren in e-Boekhouden's own app (`import/gridtable`). Only the read: the
    // paths that process, book or delete these lines (`import/verwerk`, `import/bulk/wegboeken`) are never called.
    #[tool(
        description = "Geïmporteerde bankregels (afschriftregels) die nog niet verwerkt zijn tot een mutatie. Per regel: id, datum (van de import), rekening (IBAN van de eigen bankrekening), mutDatum (boekdatum), mutBedrag (negatief is af), mutOmschrijving (met naam en IBAN van de tegenpartij), mutFactuur, grootboekId, opmerking, hasFiles en verwerkFailureReason. Te filteren op boekdatum en omschrijving. Alleen lezen: verwerken of wegboeken kan EB MCP niet.",
        annotations(title = "Bankregels", read_only_hint = true)
    )]
    async fn bankregels(
        &self,
        Parameters(args): Parameters<BankregelsArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const PATH: &str = "import/gridtable";
        let run = match self.begin(&context, "bankregels", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let query = match bankregels_query(&args) {
            Ok(query) => query,
            Err(message) => return Ok(self.refuse(run, message)),
        };
        let result = self.fetch(&run, PATH, query).await.map(named_rows);
        Ok(self.end(run, PATH, result))
    }

    #[tool(
        description = "Het rekeningschema: alle grootboekrekeningen met code en omschrijving.",
        annotations(title = "Grootboek", read_only_hint = true)
    )]
    async fn grootboek(
        &self,
        Parameters(args): Parameters<ListArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let query = grid(args.limit.or(Some(MAX_ROWS)), args.offset);
        self.simple(
            &context,
            "grootboek",
            &args,
            args.account.as_deref(),
            "grootboekrekening/gridtable",
            query,
        )
        .await
    }

    #[tool(
        description = "De balans per datum.",
        annotations(title = "Balans", read_only_hint = true)
    )]
    async fn balans(
        &self,
        Parameters(args): Parameters<BalansArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let query = (|| {
            let datum = args
                .datum
                .as_deref()
                .map(parse_date)
                .transpose()?
                .unwrap_or_else(today);
            let mut q = grid(Some(MAX_ROWS), None)?;
            q.push(("datumTot".into(), compact(datum)));
            q.push(("showkp".into(), "false".into()));
            q.push(("showGroups".into(), "true".into()));
            Ok(q)
        })();
        self.simple(
            &context,
            "balans",
            &args,
            args.account.as_deref(),
            "overzicht/balans",
            query,
        )
        .await
    }

    #[tool(
        description = "Winst en verlies over een periode (standaard dit jaar tot vandaag).",
        annotations(title = "Winst en verlies", read_only_hint = true)
    )]
    async fn winst_en_verlies(
        &self,
        Parameters(args): Parameters<PeriodeArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let query = (|| {
            let now = today();
            let van = args
                .van
                .as_deref()
                .map(parse_date)
                .transpose()?
                .unwrap_or((now.0, 1, 1));
            let tot = args
                .tot
                .as_deref()
                .map(parse_date)
                .transpose()?
                .unwrap_or(now);
            if van > tot {
                return Err("van ligt na tot".to_owned());
            }
            let mut q = grid(Some(MAX_ROWS), None)?;
            q.extend([
                ("periodStart".into(), compact(van)),
                ("periodEnd".into(), compact(tot)),
                ("periodVergelijkStart".into(), "19000101".into()),
                ("periodVergelijkEnd".into(), "19000101".into()),
                ("budgetScenarioId".into(), "0".into()),
                ("showkp".into(), "false".into()),
                ("periodType".into(), "F".into()),
                ("showGroups".into(), "true".into()),
                ("vergelijk".into(), "false".into()),
            ]);
            Ok(q)
        })();
        self.simple(
            &context,
            "winst_en_verlies",
            &args,
            args.account.as_deref(),
            "overzicht/winstenverlies",
            query,
        )
        .await
    }

    #[tool(
        description = "De btw-aangifte van een maand, kwartaal of jaar, zoals e-Boekhouden hem berekent.",
        annotations(title = "Btw", read_only_hint = true)
    )]
    async fn btw(
        &self,
        Parameters(args): Parameters<BtwArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let query = (|| {
            if !(1990..=2100).contains(&args.jaar) {
                return Err("Ongeldig jaar".to_owned());
            }
            let (kind, id) = match args.periode_type.as_ref().unwrap_or(&PeriodeType::Kwartaal) {
                PeriodeType::Maand => (
                    "M",
                    args.periode
                        .filter(|p| (1..=12).contains(p))
                        .ok_or("Geef de maand, 1 tot 12")?,
                ),
                PeriodeType::Kwartaal => (
                    "K",
                    args.periode
                        .filter(|p| (1..=4).contains(p))
                        .ok_or("Geef het kwartaal, 1 tot 4")?,
                ),
                PeriodeType::Jaar => ("J", 1),
            };
            Ok(vec![
                ("periodYear".into(), args.jaar.to_string()),
                ("periodType".into(), kind.into()),
                ("periodId".into(), id.to_string()),
            ])
        })();
        self.simple(
            &context,
            "btw",
            &args,
            args.account.as_deref(),
            "btw-aangifte/aangifte",
            query,
        )
        .await
    }

    #[tool(
        description = "Geschreven uren over een periode, eventueel per project of activiteit.",
        annotations(title = "Uren", read_only_hint = true)
    )]
    async fn uren(
        &self,
        Parameters(args): Parameters<UrenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let query = (|| {
            let now = today();
            let van = args
                .van
                .as_deref()
                .map(parse_date)
                .transpose()?
                .unwrap_or((now.0, now.1, 1));
            let tot = args
                .tot
                .as_deref()
                .map(parse_date)
                .transpose()?
                .unwrap_or(now);
            if van > tot {
                return Err("van ligt na tot".to_owned());
            }
            let mut q = grid(args.limit, None)?;
            q.push(("periodStart".into(), compact(van)));
            q.push(("periodEnd".into(), compact(tot)));
            push(&mut q, "projectId", args.project_id);
            push(&mut q, "activiteitId", args.activiteit_id);
            Ok(q)
        })();
        self.simple(
            &context,
            "uren",
            &args,
            args.account.as_deref(),
            "uur/overzicht/gridtable",
            query,
        )
        .await
    }

    #[tool(
        description = "Het digitaal archief: zonder map_id de mappen, met map_id de bestanden in die map.",
        annotations(title = "Digitaal archief", read_only_hint = true)
    )]
    async fn digitaal_archief(
        &self,
        Parameters(args): Parameters<ArchiefArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        match args.map_id {
            None => {
                self.simple(
                    &context,
                    "digitaal_archief",
                    &args,
                    args.account.as_deref(),
                    "folder/directory",
                    Ok(vec![]),
                )
                .await
            }
            Some(id) => {
                let path = format!("folder/{id}/filelist");
                let query = grid(args.limit, None);
                self.simple(
                    &context,
                    "digitaal_archief",
                    &args,
                    args.account.as_deref(),
                    &path,
                    query,
                )
                .await
            }
        }
    }

    #[tool(
        description = "Een leesbaar pad van de interne API van e-Boekhouden opvragen (GET onder /v1/api/). Alleen paden van de API-kaart die niets veranderen; al het andere wordt geweigerd zonder dat er iets naar e-Boekhouden gaat.",
        annotations(title = "GET", read_only_hint = true)
    )]
    async fn get(
        &self,
        Parameters(args): Parameters<GetArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let run = match self.begin(&context, "get", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let path = args.pad.trim().trim_matches('/').to_owned();
        if !allowed_path(&path) {
            return Ok(self.refuse(run, format!("{path:?} staat niet op de lijst van leesbare paden. EB MCP leest alleen, en alleen paden van de API-kaart.")));
        }
        let query: Vec<(String, String)> =
            args.query.clone().unwrap_or_default().into_iter().collect();
        if query.len() > 30 || query.iter().any(|(k, v)| k.len() > 64 || v.len() > 500) {
            return Ok(self.refuse(run, "Te veel of te lange queryparameters".into()));
        }
        let result = self.fetch(&run, &path, query).await;
        Ok(self.end(run, &path, result))
    }

    // ------------------------------------------------------------ changes
    //
    // From here on the tools can change an account's books. Each one stops unless the account allows writing
    // (Accounts, per account), shows the user what it will do unless the account needs no approval, and sends
    // exactly one change. The request shapes are in write.rs.

    #[tool(
        description = "Eén mutatie met al zijn regels, op mutatienummer.",
        annotations(title = "Mutatie", read_only_hint = true)
    )]
    async fn mutatie(
        &self,
        Parameters(args): Parameters<MutatieArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let path = format!("mutatie/nr/{}/model", args.nr);
        self.simple(
            &context,
            "mutatie",
            &args,
            args.account.as_deref(),
            &path,
            Ok(Vec::new()),
        )
        .await
    }

    #[tool(
        description = "Een mutatie boeken, zoals op de schermen Invoeren > Facturen, Afschrift/bonnetjes en Memoriaal: factuur ontvangen of verstuurd, factuurbetaling ontvangen of verstuurd, geld ontvangen of uitgegeven, of memoriaal. Met bankregel_id wordt een onverwerkte bankregel daarmee verwerkt. Rekeningen gaan op code (zie grootboek), btw op code (GEEN, HOOG_VERK_21, HOOG_INK_21, LAAG_VERK_9, ...); e-Boekhouden rekent de btw zelf uit. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de boeking eerst goed in EB MCP.",
        annotations(
            title = "Mutatie toevoegen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn mutatie_toevoegen(
        &self,
        Parameters(args): Parameters<write::MutatieToevoegenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "mutatie_toevoegen";
        let run = match self.begin_write(&context, TOOL, &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let result = async {
            let books = self.books(&run).await?;
            let built = write::mutatie_nieuw(&args, &books)?;
            self.approve(&run, TOOL, &built, false).await?;
            self.change(&run, "POST", "mutatie", Some(&built.body))
                .await
        }
        .await;
        Ok(self.end_write(
            run,
            "Mutatie toegevoegd; data.mutNr is het mutatienummer",
            result,
        ))
    }

    #[tool(
        description = "Een bestaande mutatie wijzigen, op mutatienummer. Wat je niet noemt blijft zoals het is; regels of betalingen die je meegeeft vervangen alle bestaande. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de wijziging eerst goed in EB MCP.",
        annotations(
            title = "Mutatie wijzigen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn mutatie_wijzigen(
        &self,
        Parameters(args): Parameters<write::MutatieWijzigenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "mutatie_wijzigen";
        let run = match self.begin_write(&context, TOOL, &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let result = async {
            let model = self
                .fetch(&run, &format!("mutatie/nr/{}/model", args.nr), Vec::new())
                .await?;
            let books = self.books(&run).await?;
            let built = write::mutatie_gewijzigd(&args, &model, &books)?;
            self.approve(&run, TOOL, &built, false).await?;
            self.change(&run, "PUT", "mutatie", Some(&built.body)).await
        }
        .await;
        Ok(self.end_write(run, "Mutatie gewijzigd", result))
    }

    #[tool(
        description = "Een bestand van deze computer in het digitaal archief zetten, en het naar wens meteen aan een mutatie koppelen (mutatie_nr). Geef het volledige pad; hoogstens 5 MB. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt het eerst goed in EB MCP.",
        annotations(
            title = "Bestand uploaden",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn archief_uploaden(
        &self,
        Parameters(args): Parameters<ArchiefUploadenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "archief_uploaden";
        let run = match self.begin_write(&context, TOOL, &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let result = async {
            let (name, data, size) = read_upload(&args.pad, args.naam.as_deref())?;
            let mut lines = vec![
                line("Bestand", args.pad.trim()),
                line("Naam in het archief", &name),
                line("Grootte", format!("{} kB", size.div_ceil(1024))),
                line("Map", args.map_id.map_or("Basismap".to_owned(), |id| format!("id {id}"))),
            ];
            if args.overschrijven == Some(true) {
                lines.push(line("Bestaand bestand", "wordt overschreven"));
            }
            if let Some(nr) = args.mutatie_nr {
                // Before anything is uploaded: a mutatie that does not exist would leave a loose file behind.
                self.fetch(&run, &format!("mutatie/nr/{nr}/model"), Vec::new()).await?;
                lines.push(line("Koppelen aan", format!("mutatie {nr}")));
            }
            let built = Built { body: Value::Null, title: format!("Bestand uploaden: {name}"), lines };
            self.approve(&run, TOOL, &built, false).await?;
            let body = json!({"fileName": name, "data": data, "overwrite": args.overschrijven.unwrap_or(false), "folderId": args.map_id.unwrap_or(0)});
            let uploaded = self.change(&run, "POST", "folder/upload", Some(&body)).await?;
            if let Some(error) = uploaded.get("errorMessage").and_then(Value::as_str).filter(|e| !e.is_empty()) {
                return Err(Halt::Error(format!("e-Boekhouden weigert het bestand: {error}")));
            }
            let id = uploaded
                .get("folderId")
                .and_then(Value::as_u64)
                .filter(|_| uploaded.get("saved").and_then(Value::as_bool) != Some(false))
                .ok_or_else(|| Halt::Error("e-Boekhouden heeft het bestand niet opgeslagen".into()))?;
            let mut done = json!({"bestand_id": id, "naam": name, "gekoppeld_aan_mutatie": Value::Null});
            if let Some(nr) = args.mutatie_nr {
                let link = json!({"koppelId": nr, "folders": [{"id": id, "soort": "F"}], "koppelType": "MUT"});
                match self.change_done(&run, "POST", "folderkoppel/koppel/item", Some(&link)).await {
                    Ok(_) => done["gekoppeld_aan_mutatie"] = json!(nr),
                    Err(Halt::Error(message) | Halt::Refused(message)) => {
                        return Err(Halt::Error(format!(
                            "Het bestand staat in het archief (id {id}), maar koppelen aan mutatie {nr} lukte niet: {message}"
                        )));
                    }
                }
            }
            Ok(done)
        }
        .await;
        Ok(self.end_write(run, "Bestand geüpload", result))
    }

    #[tool(
        description = "Een bestand dat al in het digitaal archief staat aan een mutatie koppelen. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt het eerst goed in EB MCP.",
        annotations(
            title = "Bestand koppelen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true
        )
    )]
    async fn archief_koppelen(
        &self,
        Parameters(args): Parameters<ArchiefKoppelenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "archief_koppelen";
        let run = match self.begin_write(&context, TOOL, &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let result = async {
            let file = self.fetch(&run, &format!("folder/{}", args.bestand_id), Vec::new()).await?;
            let name = file.get("naam").and_then(Value::as_str).unwrap_or("bestand").to_owned();
            if file.get("soort").and_then(Value::as_str) != Some("F") {
                return Err(Halt::Error(format!("{name} is een map, geen bestand")));
            }
            self.fetch(&run, &format!("mutatie/nr/{}/model", args.mutatie_nr), Vec::new()).await?;
            let built = Built {
                body: json!({"koppelId": args.mutatie_nr, "folders": [{"id": args.bestand_id, "soort": "F"}], "koppelType": "MUT"}),
                title: format!("Bestand koppelen: {name}"),
                lines: vec![line("Bestand", &name), line("Koppelen aan", format!("mutatie {}", args.mutatie_nr))],
            };
            self.approve(&run, TOOL, &built, false).await?;
            self.change_done(&run, "POST", "folderkoppel/koppel/item", Some(&built.body)).await
        }
        .await;
        Ok(self.end_write(run, "Bestand gekoppeld aan de mutatie", result))
    }

    #[tool(
        description = "De verkoopfacturen uit e-Boekhouden: id, nummer, datum, relatie, bedragen en of de factuur verwerkt is.",
        annotations(title = "Facturen", read_only_hint = true)
    )]
    async fn facturen(
        &self,
        Parameters(args): Parameters<ListArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const PATH: &str = "factuur/normaal/gridtable";
        let run = match self.begin(&context, "facturen", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let query = match grid(args.limit, args.offset) {
            Ok(query) => query,
            Err(message) => return Ok(self.refuse(run, message)),
        };
        let result = self.fetch(&run, PATH, query).await.map(named_rows);
        Ok(self.end(run, PATH, result))
    }

    #[tool(
        description = "Eén verkoopfactuur met zijn regels, op id (uit facturen).",
        annotations(title = "Factuur", read_only_hint = true)
    )]
    async fn factuur(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let run = match self.begin(&context, "factuur", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let result = self
            .with_items(&run, "factuur", args.id)
            .await
            .map(|(factuur, regels)| json!({"factuur": factuur, "regels": regels}));
        Ok(self.end(run, &format!("factuur/{}", args.id), result))
    }

    #[tool(
        description = "Een verkoopfactuur maken. Prijzen per stuk, incl of excl btw; btw op code (HOOG_VERK_21, LAAG_VERK_9, GEEN, ...), de omzetrekening op code. Zonder nummer krijgt de factuur het volgende nummer van e-Boekhouden. De factuur wordt niet verstuurd. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de factuur eerst goed in EB MCP.",
        annotations(
            title = "Factuur toevoegen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn factuur_toevoegen(
        &self,
        Parameters(args): Parameters<write::FactuurToevoegenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "factuur_toevoegen";
        let run = match self.begin_write(&context, TOOL, &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let result = async {
            let books = self.books(&run).await?;
            let factuur_context = self.factuur_context(&run).await?;
            let built = write::factuur_nieuw(&args, &factuur_context, &books)?;
            self.approve(&run, TOOL, &built, false).await?;
            self.change(&run, "POST", "factuur/model", Some(&built.body))
                .await
        }
        .await;
        Ok(self.end_write(
            run,
            "Factuur toegevoegd; data.id is het id en data.nummer het factuurnummer",
            result,
        ))
    }

    #[tool(
        description = "Een verkoopfactuur wijzigen, op id. Wat je niet noemt blijft zoals het is; regels die je meegeeft vervangen alle bestaande. De mutatie van een verwerkte factuur volgt vanzelf. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de wijziging eerst goed in EB MCP.",
        annotations(
            title = "Factuur wijzigen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn factuur_wijzigen(
        &self,
        Parameters(args): Parameters<write::FactuurWijzigenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "factuur_wijzigen";
        let run = match self.begin_write(&context, TOOL, &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let result = async {
            let (now, items) = self.with_items(&run, "factuur", args.id).await?;
            let books = self.books(&run).await?;
            let built = write::factuur_gewijzigd(&args, &now, &items, &books)?;
            self.approve(&run, TOOL, &built, false).await?;
            self.change_done(&run, "PUT", "factuur/model", Some(&built.body))
                .await
        }
        .await;
        Ok(self.end_write(run, "Factuur gewijzigd", result))
    }

    #[tool(
        description = "Een verkoopfactuur verwijderen, op id. De mutatie van een verwerkte factuur verdwijnt mee. Dit is niet terug te draaien. De gebruiker moet dit altijd goedkeuren in EB MCP.",
        annotations(
            title = "Factuur verwijderen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn factuur_verwijderen(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "factuur_verwijderen";
        let run = match self.begin_write(&context, TOOL, &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let result = async {
            let now = self
                .fetch(&run, &format!("factuur/{}", args.id), Vec::new())
                .await?;
            let nummer = now
                .get("nummer")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let built = Built {
                body: Value::Null,
                title: format!("Factuur {nummer} verwijderen"),
                lines: vec![
                    line("Factuur", &nummer),
                    line(
                        "Relatie",
                        format!(
                            "id {}",
                            now.get("relatieId").and_then(Value::as_u64).unwrap_or(0)
                        ),
                    ),
                    line(
                        "Totaal incl. btw",
                        write::euro(now.get("totaal").and_then(Value::as_f64).unwrap_or(0.0)),
                    ),
                    line(
                        "Gevolg",
                        "de factuur en zijn mutatie verdwijnen; dit is niet terug te draaien",
                    ),
                ],
            };
            self.approve(&run, TOOL, &built, true).await?;
            self.change_done(
                &run,
                "DELETE",
                &format!("factuur/normaal/{}", args.id),
                None,
            )
            .await
        }
        .await;
        Ok(self.end_write(run, "Factuur verwijderd", result))
    }

    #[tool(
        description = "Een verkoopfactuur crediteren, op id: er komt een nieuwe factuur met dezelfde regels in negatieve aantallen, zoals Factuur crediteren in e-Boekhouden doet. De gebruiker moet dit altijd goedkeuren in EB MCP.",
        annotations(
            title = "Factuur crediteren",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn factuur_crediteren(
        &self,
        Parameters(args): Parameters<write::FactuurCreditArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "factuur_crediteren";
        let run = match self.begin_write(&context, TOOL, &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let result = async {
            let (now, items) = self.with_items(&run, "factuur", args.id).await?;
            let books = self.books(&run).await?;
            let factuur_context = self.factuur_context(&run).await?;
            let built = write::factuur_credit(&args, &now, &items, &factuur_context, &books)?;
            self.approve(&run, TOOL, &built, true).await?;
            self.change(&run, "POST", "factuur/model", Some(&built.body))
                .await
        }
        .await;
        Ok(self.end_write(
            run,
            "Creditfactuur gemaakt; data.id is het id en data.nummer het factuurnummer",
            result,
        ))
    }

    #[tool(
        description = "De abonnementen: id, naam, relatie, frequentie, startdatum, prolongatiedatum en bedrag.",
        annotations(title = "Abonnementen", read_only_hint = true)
    )]
    async fn abonnementen(
        &self,
        Parameters(args): Parameters<ListArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const PATH: &str = "abonnement/gridtable";
        let run = match self.begin(&context, "abonnementen", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let query = match grid(args.limit, args.offset) {
            Ok(query) => query,
            Err(message) => return Ok(self.refuse(run, message)),
        };
        let result = self.fetch(&run, PATH, query).await.map(named_rows);
        Ok(self.end(run, PATH, result))
    }

    #[tool(
        description = "Eén abonnement met zijn data en regels, op id (uit abonnementen).",
        annotations(title = "Abonnement", read_only_hint = true)
    )]
    async fn abonnement(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let run = match self.begin(&context, "abonnement", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let result = async {
            let (abonnement, regels) = self.with_items(&run, "abonnement", args.id).await?;
            let data = self
                .fetch(&run, &format!("factuurextra/{}", args.id), Vec::new())
                .await?;
            Ok(json!({"abonnement": abonnement, "data": data, "regels": regels}))
        }
        .await;
        Ok(self.end(run, &format!("abonnement/{}", args.id), result))
    }

    #[tool(
        description = "Een abonnement maken: een terugkerende factuur voor een relatie. Prijzen per stuk, incl of excl btw; btw en omzetrekening op code. Er wordt nog geen factuur gemaakt; dat doet abonnementen_factureren. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt het eerst goed in EB MCP.",
        annotations(
            title = "Abonnement toevoegen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn abonnement_toevoegen(
        &self,
        Parameters(args): Parameters<write::AbonnementToevoegenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "abonnement_toevoegen";
        let run = match self.begin_write(&context, TOOL, &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let result = async {
            let books = self.books(&run).await?;
            let factuur_context = self.factuur_context(&run).await?;
            let built =
                write::abonnement_nieuw(&args, factuur_context.sjabloon_id, today(), &books)?;
            self.approve(&run, TOOL, &built, false).await?;
            self.change(&run, "POST", "abonnement/model", Some(&built.body))
                .await
        }
        .await;
        Ok(self.end_write(run, "Abonnement toegevoegd; data.id is het id", result))
    }

    #[tool(
        description = "Een abonnement wijzigen, op id. Wat je niet noemt blijft zoals het is; regels die je meegeeft vervangen alle bestaande. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de wijziging eerst goed in EB MCP.",
        annotations(
            title = "Abonnement wijzigen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn abonnement_wijzigen(
        &self,
        Parameters(args): Parameters<write::AbonnementWijzigenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "abonnement_wijzigen";
        let run = match self.begin_write(&context, TOOL, &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let result = async {
            let (now, items) = self.with_items(&run, "abonnement", args.id).await?;
            let extra = self
                .fetch(&run, &format!("factuurextra/{}", args.id), Vec::new())
                .await?;
            let books = self.books(&run).await?;
            let built = write::abonnement_gewijzigd(&args, &now, &extra, &items, &books)?;
            self.approve(&run, TOOL, &built, false).await?;
            self.change_done(&run, "PUT", "abonnement/model", Some(&built.body))
                .await
        }
        .await;
        Ok(self.end_write(run, "Abonnement gewijzigd", result))
    }

    #[tool(
        description = "Een abonnement verwijderen, op id. Facturen die er al van gemaakt zijn blijven staan. Dit is niet terug te draaien. De gebruiker moet dit altijd goedkeuren in EB MCP.",
        annotations(
            title = "Abonnement verwijderen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn abonnement_verwijderen(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "abonnement_verwijderen";
        let run = match self.begin_write(&context, TOOL, &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let result = async {
            let now = self
                .fetch(&run, &format!("abonnement/{}", args.id), Vec::new())
                .await?;
            let naam = now
                .get("nummer")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let built = Built {
                body: Value::Null,
                title: format!("Abonnement {naam} verwijderen"),
                lines: vec![
                    line("Abonnement", &naam),
                    line(
                        "Relatie",
                        format!(
                            "id {}",
                            now.get("relatieId").and_then(Value::as_u64).unwrap_or(0)
                        ),
                    ),
                    line(
                        "Bedrag incl. btw",
                        write::euro(now.get("totaal").and_then(Value::as_f64).unwrap_or(0.0)),
                    ),
                    line(
                        "Gevolg",
                        "het abonnement verdwijnt; dit is niet terug te draaien",
                    ),
                ],
            };
            self.approve(&run, TOOL, &built, true).await?;
            self.change_done(&run, "DELETE", &format!("abonnement/{}", args.id), None)
                .await
        }
        .await;
        Ok(self.end_write(run, "Abonnement verwijderd", result))
    }

    #[tool(
        description = "Facturen maken voor abonnementen, zoals Facturen maken in e-Boekhouden: voor ALLE abonnementen waarvan de prolongatiedatum in het gekozen venster valt (standaard: al verlopen) komt er een factuur, en hun prolongatiedatum schuift een periode op. Eén abonnement kiezen kan niet. De gebruiker moet dit altijd goedkeuren in EB MCP.",
        annotations(
            title = "Abonnementen factureren",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn abonnementen_factureren(
        &self,
        Parameters(args): Parameters<write::AbonnementenFacturerenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "abonnementen_factureren";
        let run = match self.begin_write(&context, TOOL, &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let result = async {
            let counts = self
                .fetch(&run, "factuur/expiredabonnementencount", Vec::new())
                .await?;
            let venster = args.venster.unwrap_or(write::Venster::Verlopen);
            let aantal = counts
                .get(venster.count_key())
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let books = self.books(&run).await?;
            let built = write::abonnementen_factureren(&args, aantal, today(), &books)?;
            self.approve(&run, TOOL, &built, true).await?;
            let call = self
                .state
                .api_hub(
                    run.account.id,
                    "factuur/abonnementenfactureren",
                    "create",
                    &built.body,
                    Some(run.id),
                    &run.client,
                )
                .await;
            if !call.ok() {
                return Err(Halt::Error(
                    call.error
                        .unwrap_or_else(|| "e-Boekhouden gaf een fout".into()),
                ));
            }
            Ok(call.json.unwrap_or(Value::Null))
        }
        .await;
        Ok(self.end_write(run, "Facturen gemaakt; data.createdIds zijn de ids van de nieuwe facturen en data.errorMessages wat niet lukte", result))
    }
}

/// How clients name and picture the server: "e-Boekhouden MCP" with the app icon, sent inline so it
/// works without network access.
fn server_info() -> Implementation {
    use base64::Engine;
    let png =
        base64::engine::general_purpose::STANDARD.encode(include_bytes!("../icons/128x128.png"));
    let icon: Icon = serde_json::from_value(serde_json::json!({
        "src": format!("data:image/png;base64,{png}"),
        "mimeType": "image/png",
        "sizes": ["128x128"],
    }))
    .expect("a valid MCP icon");
    Implementation::new("e-boekhouden", env!("CARGO_PKG_VERSION"))
        .with_title("e-Boekhouden MCP")
        .with_description(
            "Leest je boekhouding in e-Boekhouden, en boekt alleen waar jij dat aanzet.",
        )
        .with_website_url("https://sherin.dev/eb-mcp/")
        .with_icons(vec![icon])
}

#[tool_handler]
impl ServerHandler for EbMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(server_info())
            .with_instructions(
                "Leest de boekhouding in e-Boekhouden van de accounts die in EB MCP zijn ingelogd. \
                 Wijzigen (mutaties, bestanden, facturen, abonnementen) kan alleen voor accounts waar de gebruiker schrijven heeft aangezet; \
                 status zegt per account of dat uit staat, met akkoord of zonder akkoord. Met akkoord toont EB MCP elke wijziging eerst aan de gebruiker, \
                 en de tool wacht tot die is goedgekeurd of geweigerd: vertel de gebruiker dat er in EB MCP iets klaarstaat. \
                 Staat er meer dan één account in EB MCP, dan moet elke tool de account-parameter meekrijgen; status zegt welke accounts er zijn.",
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_itself_e_boekhouden_mcp_with_the_app_icon() {
        let info = server_info();
        assert_eq!(info.name, "e-boekhouden");
        assert_eq!(info.title.as_deref(), Some("e-Boekhouden MCP"));
        let icon = &info.icons.as_ref().unwrap()[0];
        assert!(
            icon.src.starts_with("data:image/png;base64,iVBOR"),
            "{}",
            &icon.src[..40]
        );
    }

    fn account(label: &str) -> Account {
        Account {
            id: uuid::Uuid::new_v4(),
            label: label.into(),
            username: "lisa@atelierzuid.example".into(),
            store: crate::accounts::Store::System,
            op_ref: None,
            op_title: None,
            op_vault: None,
            administration: None,
            write: crate::accounts::WriteMode::Off,
        }
    }

    #[test]
    fn a_single_account_needs_no_name() {
        let only = account("Atelier Zuid");
        assert_eq!(pick_account(vec![only.clone()], None), Ok(only.clone()));
        assert_eq!(
            pick_account(vec![only.clone()], Some("  ")),
            Ok(only.clone())
        );
        assert_eq!(
            pick_account(vec![only.clone()], Some("atelier zuid")),
            Ok(only)
        );
    }

    #[test]
    fn several_accounts_must_be_named() {
        let (a, b) = (account("Atelier Zuid"), account("Studio Noord"));
        let both = vec![a.clone(), b.clone()];
        let unnamed = pick_account(both.clone(), None).unwrap_err();
        assert!(unnamed.contains("2 accounts"), "{unnamed}");
        assert!(
            unnamed.contains("parameter account") && unnamed.contains("Atelier Zuid, Studio Noord"),
            "{unnamed}"
        );
        assert_eq!(pick_account(both.clone(), Some(" ")).unwrap_err(), unnamed);
        assert_eq!(pick_account(both.clone(), Some("Studio Noord")), Ok(b));
        assert_eq!(pick_account(both.clone(), Some(&a.id.to_string())), Ok(a));
        let wrong = pick_account(both, Some("Werkplaats Oost")).unwrap_err();
        assert!(
            wrong.contains("\"Werkplaats Oost\"") && wrong.contains("Atelier Zuid, Studio Noord"),
            "{wrong}"
        );
    }

    #[test]
    fn says_so_when_there_is_no_account_yet() {
        for name in [None, Some("Atelier Zuid")] {
            let message = pick_account(Vec::new(), name).unwrap_err();
            assert!(message.contains("nog geen account"), "{message}");
        }
    }

    #[test]
    fn allows_reads_from_the_map_only() {
        assert!(allowed_path("relatie/gridtable"));
        assert!(allowed_path("/relatie/48211/"));
        assert!(allowed_path("folder/1184/filelist"));
        assert!(allowed_path("Overzicht/Balans"));
        assert!(!allowed_path("mutatie/opslaan"));
        assert!(!allowed_path("folder/12/undelete"));
        assert!(!allowed_path("auth/switch/1"));
        assert!(!allowed_path("relatie/../auth/login"));
        assert!(!allowed_path("relatie/gridtable?x=1"));
        assert!(!allowed_path("relatie"));
        assert!(!allowed_path(""));
    }

    #[test]
    fn parses_real_dates_only() {
        assert_eq!(parse_date("2026-10-31"), Ok((2026, 10, 31)));
        assert_eq!(parse_date("2024-02-29"), Ok((2024, 2, 29)));
        assert!(parse_date("2026-02-29").is_err());
        assert!(parse_date("31-10-2026").is_err());
        assert!(parse_date("2026-13-01").is_err());
        assert_eq!(compact((2026, 1, 5)), "20260105");
    }

    #[test]
    fn today_is_a_real_date() {
        let (y, m, d) = today();
        assert!(parse_date(&dashed((y, m, d))).is_ok());
        assert!(y >= 2026);
    }

    #[test]
    fn the_date_follows_the_local_offset() {
        const WINTER_EVENING: i64 = 1_794_353_400_000; // 2026-11-10 23:30 UTC
        assert_eq!(date_at(WINTER_EVENING, 0), (2026, 11, 10));
        assert_eq!(
            date_at(WINTER_EVENING, 3600),
            (2026, 11, 11),
            "00:30 in Amsterdam"
        );
        const SUMMER_EVENING: i64 = 1_782_857_700_000; // 2026-06-30 22:15 UTC
        assert_eq!(date_at(SUMMER_EVENING, 0), (2026, 6, 30));
        assert_eq!(
            date_at(SUMMER_EVENING, 7200),
            (2026, 7, 1),
            "00:15 in Amsterdam"
        );
        const LEAP_EVENING: i64 = 1_709_161_200_000; // 2024-02-28 23:00 UTC
        assert_eq!(date_at(LEAP_EVENING, 3600), (2024, 2, 29));
        assert_eq!(date_at(0, -3600), (1969, 12, 31), "west of UTC");
    }

    #[test]
    fn counts_its_tools_on_the_router() {
        assert_eq!(tool_count(), 30);
    }

    #[test]
    fn asks_for_bank_lines_the_way_e_boekhoudens_grid_filters() {
        let pair = |key: &str, value: &str| (key.to_owned(), value.to_owned());
        assert_eq!(
            bankregels_query(&BankregelsArgs::default()).unwrap(),
            grid(None, None).unwrap(),
            "no filter unless asked"
        );
        let both = BankregelsArgs {
            van: Some("2026-11-03".into()),
            tot: Some("2026-11-06".into()),
            omschrijving: Some(" bakker ".into()),
            limit: Some(20),
            ..Default::default()
        };
        let query = bankregels_query(&both).unwrap();
        assert!(query.contains(&pair("limit", "20")));
        assert!(query.contains(&pair("searchBETWEmutDatum", "2026-11-03<->2026-11-06")));
        assert!(query.contains(&pair("searchCONTAmutOmschrijving", "bakker")));
        // One bound alone still includes that day.
        let from = BankregelsArgs {
            van: Some("2026-11-06".into()),
            ..Default::default()
        };
        assert!(
            bankregels_query(&from)
                .unwrap()
                .contains(&pair("searchBETWEmutDatum", "2026-11-06<->9999-12-31"))
        );
        let until = BankregelsArgs {
            tot: Some("2026-11-06".into()),
            ..Default::default()
        };
        assert!(
            bankregels_query(&until)
                .unwrap()
                .contains(&pair("searchBETWEmutDatum", "1900-01-01<->2026-11-06"))
        );
        // Nothing goes out for a date that is none, or for a period that ends before it starts.
        let bad = BankregelsArgs {
            van: Some("06-11-2026".into()),
            ..Default::default()
        };
        assert!(bankregels_query(&bad).is_err());
        let backwards = BankregelsArgs {
            van: Some("2026-11-06".into()),
            tot: Some("2026-11-03".into()),
            ..Default::default()
        };
        assert_eq!(bankregels_query(&backwards).unwrap_err(), "van ligt na tot");
    }

    #[test]
    fn names_the_columns_of_a_grid_row() {
        // The shape of import/gridtable, with made-up values.
        let grid = json!({
            "colMetadata": [
                {"index": 0, "name": "id", "readerIndex": 0},
                {"index": 1, "name": "mutDatum", "readerIndex": 1},
                {"index": 2, "name": "mutBedrag", "readerIndex": 2},
                {"index": 3, "name": "opmerking", "readerIndex": 3}
            ],
            "data": [[101, "2026-11-06T00:00:00", -77.58, null], [102, "2026-11-07T00:00:00", 12.0]],
            "rowCount": 2, "limit": 50, "offset": 0
        });
        let named = named_rows(grid);
        assert_eq!(
            named["data"],
            json!([
                {"id": 101, "mutDatum": "2026-11-06T00:00:00", "mutBedrag": -77.58, "opmerking": null},
                {"id": 102, "mutDatum": "2026-11-07T00:00:00", "mutBedrag": 12.0, "opmerking": null}
            ])
        );
        assert_eq!(named["rowCount"], 2);
        assert!(named.get("colMetadata").is_none());
        assert_eq!(crate::ebapi::count_rows(&named), Some(2));
        // Another shape is left alone.
        let other = json!({"data": [{"id": 1}], "colMetadata": [{"name": "id", "readerIndex": 0}]});
        assert_eq!(named_rows(other.clone()), other);
        assert_eq!(named_rows(json!({"valid": true})), json!({"valid": true}));
    }

    #[test]
    fn takes_an_upload_only_from_a_real_file_with_a_full_path() {
        let dir = std::env::temp_dir().join(format!("ebmcp-upload-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("bon.pdf");
        std::fs::write(&file, b"%PDF-1.4 proef").unwrap();
        let path = file.to_str().unwrap();
        let (name, data, size) = read_upload(path, None).unwrap();
        assert_eq!((name.as_str(), size), ("bon.pdf", 14));
        // base64 of the bytes, without a data: prefix: what folder/upload takes.
        assert_eq!(data, "JVBERi0xLjQgcHJvZWY=");
        assert_eq!(
            read_upload(path, Some(" Bon november.pdf ")).unwrap().0,
            "Bon november.pdf"
        );
        assert!(
            read_upload("bon.pdf", None)
                .unwrap_err()
                .contains("volledige pad")
        );
        assert!(
            read_upload(dir.to_str().unwrap(), None)
                .unwrap_err()
                .contains("geen bestand")
        );
        assert!(
            read_upload(dir.join("weg.pdf").to_str().unwrap(), None)
                .unwrap_err()
                .contains("Geen bestand")
        );
        assert!(read_upload(path, Some("map/bon.pdf")).is_err());
        let empty = dir.join("leeg.pdf");
        std::fs::write(&empty, b"").unwrap();
        assert!(
            read_upload(empty.to_str().unwrap(), None)
                .unwrap_err()
                .contains("leeg")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn grid_limits() {
        assert!(grid(Some(0), None).is_err());
        assert!(grid(Some(201), None).is_err());
        assert_eq!(
            grid(None, Some(50)).unwrap()[0],
            ("offset".into(), "50".into())
        );
    }
}
