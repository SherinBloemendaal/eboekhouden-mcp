//! Btw-aangifte, ICP-opgave and suppletie: drawing up, reading and correcting. Never filing.
//!
//! The tools of this part live in three submodules, each with its own router, joined here into
//! `btw_aangifte_router`, which tools.rs adds to the server's. Each tool is named in one of the lists below,
//! which the tool-list test in mcp.rs reads.
//!
//! - `omzet`: the btw-aangifte (omzetbelasting) and the suppletie: the returns sent, one return with its
//!   boxes, the mutaties behind a box, the corrections of earlier periods, and drawing a return or a
//!   suppletie up in the books ("verwerken in de boekhouding").
//! - `icp`: the ICP-opgave: the opgaven sent, one opgave with its lines, its details and its own lines.
//! - `betaallink`: the iDEAL link of a sent return, looked up for the user to open; never opened by EB MCP.
//!
//! Filing with the Belastingdienst is never built, not even behind approval. Every request of this part goes
//! through `btwaangifte_get` or `btwaangifte_send`, which refuse the paths in `FILING` before anything goes out;
//! the tests prove that no other call is made and that none of those paths is ever reached. The one exception,
//! asked for by the user, is `btwaangifte_betaling_starten`: it asks e-Boekhouden for a payment link of one
//! sent return, only after the user approved, and pays nothing.

use super::*;

mod betaallink;
mod icp;
mod omzet;
#[cfg(test)]
mod tests;

/// The tools of this part that only read.
#[cfg(test)]
pub(super) const READS: &[&str] = &[
    "btwaangiftes",
    "btwaangifte",
    "btwaangifte_mutaties",
    "btwaangifte_correcties",
    "suppleties",
    "suppletie",
    "icp_opgaven",
    "icp_opgave",
];
/// The tools of this part that change the books.
#[cfg(test)]
pub(super) const WRITES: &[&str] = &[
    "btwaangifte_verwerken",
    "btwaangifte_verschil_negeren",
    "suppletie_verwerken",
    "icp_opgave_openen",
    "icp_opgave_wijzigen",
    "icp_regel_toevoegen",
    "icp_regel_wijzigen",
    "icp_regel_verwijderen",
    "btwaangifte_betaallink",
];
/// The ones in `WRITES` that cannot be undone.
#[cfg(test)]
pub(super) const DESTRUCTIVE: &[&str] = &[
    "btwaangifte_verwerken",
    "suppletie_verwerken",
    "icp_regel_verwijderen",
];

impl EbMcp {
    /// The tools of this part: the btw-aangifte and the suppletie, and the ICP-opgave.
    pub(super) fn btw_aangifte_router() -> ToolRouter<Self> {
        Self::btwaangifte_omzet_router()
            + Self::btwaangifte_icp_router()
            + Self::btwaangifte_betaal_router()
    }
}

// filing-paths:start
/// What files a return, an ICP-opgave, a suppletie or a client listing with the tax authority, or pays or
/// confirms one, in e-Boekhouden's own web app (public bundle, 2026-11-12). Paths under /v1/api/, lowercase;
/// `{}` stands for one segment, and a path is refused when these segments appear in it one after the other:
///
/// - `POST btw-aangifte/createandsend`: the btw-aangifte, "Aangifte nu elektronisch versturen".
/// - `POST suppletie/createandsend`: the suppletie, "Suppletie versturen".
/// - `GET icpopgave/dosend/{jaar}/{periodType}/{periodId}`: the ICP-opgave, "Opgave elektronisch versturen".
/// - `btw-aangifte-be/...`: the Belgian return; its `verwerk` makes the file for Intervat in the same request,
///   `bevestigen` and `annuleren` confirm or undo that it was sent.
/// - `klantenlisting/process`, `klantenlisting/{id}/bevestigen`: the Belgian client listing, made and confirmed.
/// - `GET aangifte/pay/{id}`, `aangifte/paystatus/...`, `aangifte/paystatusgrid/...`: paying a return with iDEAL.
/// - `GET aangifte/{id}/andersbetaald`: marking a return as paid some other way.
const FILING: &[&str] = &[
    "createandsend",
    "dosend",
    "btw-aangifte-be",
    "klantenlisting/process",
    "klantenlisting/{}/bevestigen",
    "aangifte/pay",
    "aangifte/paystatus",
    "aangifte/paystatusgrid",
    "andersbetaald",
];
// filing-paths:end

/// Whether the segments of `pattern` (`{}` for any one) appear in `path` one after the other.
fn has_segments(path: &[&str], pattern: &str) -> bool {
    let pattern: Vec<&str> = pattern.split('/').collect();
    path.windows(pattern.len()).any(|window| {
        window
            .iter()
            .zip(&pattern)
            .all(|(got, want)| *want == "{}" || got == want)
    })
}

/// Refuses a path that files, pays or confirms a return. Every request of this part passes here first.
fn refuse_filing(path: &str) -> Result<(), String> {
    let path = path.trim().to_ascii_lowercase();
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if FILING
        .iter()
        .any(|pattern| has_segments(&segments, pattern))
    {
        return Err(
            "EB MCP verstuurt, betaalt en bevestigt geen btw-aangifte, ICP-opgave, suppletie of klantenlisting bij de belastingdienst. \
             Dat doet de gebruiker zelf in e-Boekhouden. Er is niets verstuurd."
                .into(),
        );
    }
    Ok(())
}

// requests:start
impl EbMcp {
    /// A read of this part, never one that files.
    async fn btwaangifte_get(
        &self,
        run: &Run,
        path: &str,
        query: Vec<(String, String)>,
    ) -> Result<Value, String> {
        refuse_filing(path)?;
        self.fetch(run, path, query).await
    }

    /// A change of this part, after its approval, never one that files.
    async fn btwaangifte_send(
        &self,
        run: &Run,
        method: &'static str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, Halt> {
        refuse_filing(path)?;
        self.change_done(run, method, path, body).await
    }

    // pay-link:start
    /// The one exception to `refuse_filing`, asked for by the user: `GET aangifte/pay/{id}`, which makes
    /// e-Boekhouden start an iDEAL payment for a sent return and answer with its link (`startPaymentUrl`). It
    /// pays nothing: the user pays only by opening the link in their own bank. The id is a number, so the path
    /// is exactly this one and no other. Used only by `btwaangifte_betaallink`, after the user approved.
    async fn btwaangifte_betaling_starten(&self, run: &Run, id: u64) -> Result<Value, Halt> {
        self.fetch(run, &format!("aangifte/pay/{id}"), Vec::new())
            .await
            .map_err(Halt::Error)
    }
    // pay-link:end
}
// requests:end

impl EbMcp {
    /// Every row of a grid, page by page, as named rows; `query` holds the grid's own filters.
    async fn btwaangifte_rows(
        &self,
        run: &Run,
        path: &str,
        query: &[(String, String)],
    ) -> Result<Vec<Value>, String> {
        /// Pages of `MAX_ROWS`: ten thousand rows, far more than a return or an opgave has.
        const PAGES: u32 = 50;
        let mut rows = Vec::new();
        for page in 0..PAGES {
            let mut page_query = grid(Some(MAX_ROWS), Some(page * MAX_ROWS))?;
            page_query.extend(query.iter().cloned());
            let answer = named_rows(self.btwaangifte_get(run, path, page_query).await?);
            let found = answer
                .get("data")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let total = answer.get("rowCount").and_then(Value::as_u64);
            let got = found.len();
            rows.extend(found);
            let more = match total {
                Some(total) => (rows.len() as u64) < total,
                None => got == MAX_ROWS as usize,
            };
            if !more || got == 0 {
                return Ok(rows);
            }
        }
        Err(format!(
            "e-Boekhouden gaf meer dan {} rijen op {path}; EB MCP leest er niet meer dan dat",
            PAGES * MAX_ROWS
        ))
    }

    /// The list of returns sent ("Verstuurde aangiftes"), all of it, as named rows. It holds btw-aangiftes,
    /// suppleties and ICP-opgaven together; the screen tells them apart by `soort`.
    async fn btwaangifte_sent(&self, run: &Run) -> Result<Vec<Value>, String> {
        self.btwaangifte_rows(run, "aangifte/gridtable", &[])
            .await
            .map(|rows| rows.into_iter().map(without_pay_link).collect())
    }
}

/// A row of the list of sent returns without e-Boekhouden's link to pay it (`payUrl`): paying a return is
/// the user's own act in e-Boekhouden, and a link in a tool's answer invites an AI client to offer it.
pub(super) fn without_pay_link(mut row: Value) -> Value {
    match &mut row {
        Value::Object(fields) => {
            fields.remove("payUrl");
            for value in fields.values_mut() {
                *value = without_pay_link(value.take());
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                *item = without_pay_link(item.take());
            }
        }
        _ => {}
    }
    row
}

/// The kind of a row in the list of returns sent, as the screen opens it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verzonden {
    Aangifte,
    Suppletie,
    Icp,
    /// Belgian returns and client listings: not part of these tools.
    Anders,
}

impl Verzonden {
    /// As the screen's `_navigateOnSelect`: OB opens a return, SUP a suppletie, the Belgian kinds their own
    /// screens, anything else an ICP-opgave.
    fn of(row: &Value) -> Self {
        match row.get("soort").and_then(Value::as_str).unwrap_or_default() {
            "OB" => Self::Aangifte,
            "SUP" => Self::Suppletie,
            "Klantenlisting" | "Klantenlisting met omzet" | "Btw" => Self::Anders,
            _ => Self::Icp,
        }
    }
}

/// A number from e-Boekhouden, which sends amounts as numbers and, in a return sent, sometimes as text.
fn number(value: Option<&Value>) -> Result<f64, String> {
    match value {
        None | Some(Value::Null) => Ok(0.0),
        Some(Value::Number(n)) => n
            .as_f64()
            .ok_or_else(|| "e-Boekhouden gaf een bedrag dat EB MCP niet kan lezen".to_owned()),
        Some(Value::String(text)) if text.trim().is_empty() => Ok(0.0),
        Some(Value::String(text)) => text.trim().replace(',', ".").parse().map_err(|_| {
            format!("e-Boekhouden gaf een bedrag dat EB MCP niet kan lezen: {text:?}")
        }),
        Some(other) => Err(format!(
            "e-Boekhouden gaf een bedrag dat EB MCP niet kan lezen: {other}"
        )),
    }
}

/// An amount for a request body: a whole number stays whole, as the web app's number fields send it.
fn amount(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        json!(value as i64)
    } else {
        json!(value)
    }
}

/// A text of a record, empty when it is missing.
fn text_of(record: &Value, key: &str) -> String {
    match record.get(key) {
        Some(Value::String(text)) => text.trim().to_owned(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

const MAANDEN: [&str; 12] = [
    "januari",
    "februari",
    "maart",
    "april",
    "mei",
    "juni",
    "juli",
    "augustus",
    "september",
    "oktober",
    "november",
    "december",
];

fn days_in((y, m): (i32, u32)) -> u32 {
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    match m {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// The period of a btw-aangifte, as the btw screens take it: `periodYear`, `periodType` M, K or J and
/// `periodId` (the month, the quarter, or 1 for a year, as the `btw` tool sends it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BtwPeriode {
    jaar: i32,
    soort: &'static str,
    id: u32,
}

impl BtwPeriode {
    /// Quarter by default, as the `btw` tool.
    fn new(jaar: i32, soort: Option<&PeriodeType>, periode: Option<u32>) -> Result<Self, String> {
        if !(1990..=2100).contains(&jaar) {
            return Err("Ongeldig jaar".into());
        }
        let (soort, id) = match soort.unwrap_or(&PeriodeType::Kwartaal) {
            PeriodeType::Maand => (
                "M",
                periode
                    .filter(|p| (1..=12).contains(p))
                    .ok_or("Geef de maand, 1 tot 12")?,
            ),
            PeriodeType::Kwartaal => (
                "K",
                periode
                    .filter(|p| (1..=4).contains(p))
                    .ok_or("Geef het kwartaal, 1 tot 4")?,
            ),
            PeriodeType::Jaar => ("J", 1),
        };
        Ok(Self { jaar, soort, id })
    }

    fn months(self) -> (u32, u32) {
        match self.soort {
            "M" => (self.id, self.id),
            "K" => (self.id * 3 - 2, self.id * 3),
            _ => (1, 12),
        }
    }

    fn start(self) -> (i32, u32, u32) {
        (self.jaar, self.months().0, 1)
    }

    fn end(self) -> (i32, u32, u32) {
        let last = self.months().1;
        (self.jaar, last, days_in((self.jaar, last)))
    }

    fn naam(self) -> String {
        match self.soort {
            "M" => format!("{} {}", MAANDEN[(self.id - 1) as usize], self.jaar),
            "K" => format!("{}e kwartaal {}", self.id, self.jaar),
            _ => format!("jaar {}", self.jaar),
        }
    }

    /// Whether the period is over on `today`: the screen sends a return only then.
    fn over(self, today: (i32, u32, u32)) -> bool {
        today > self.end()
    }

    fn query(self) -> Vec<(String, String)> {
        vec![
            ("periodYear".into(), self.jaar.to_string()),
            ("periodType".into(), self.soort.into()),
            ("periodId".into(), self.id.to_string()),
        ]
    }
}

/// The period of an ICP-opgave or of the period an ICP line corrects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IcpTijdvak {
    Maand,
    /// Januari/februari, april/mei, juli/augustus of oktober/november (periode 1 tot 4).
    TweeMaanden,
    Kwartaal,
    Jaar,
}

/// An ICP period as the ICP screens take it: `periodType` M, 2M, K or J and `periodId` (the month, the
/// two-month period, the quarter, or the year itself for a year, as the screen's search form sends it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct IcpPeriode {
    jaar: i32,
    soort: &'static str,
    id: u32,
}

impl IcpPeriode {
    fn new(jaar: i32, tijdvak: Option<IcpTijdvak>, periode: Option<u32>) -> Result<Self, String> {
        if !(1990..=2100).contains(&jaar) {
            return Err("Ongeldig jaar".into());
        }
        let tijdvak = tijdvak.ok_or("Geef het tijdvak: maand, twee_maanden, kwartaal of jaar")?;
        let (soort, id) = match tijdvak {
            IcpTijdvak::Maand => (
                "M",
                periode
                    .filter(|p| (1..=12).contains(p))
                    .ok_or("Geef de maand, 1 tot 12")?,
            ),
            IcpTijdvak::TweeMaanden => (
                "2M",
                periode
                    .filter(|p| (1..=4).contains(p))
                    .ok_or("Geef de periode van twee maanden, 1 tot 4 (januari/februari tot oktober/november)")?,
            ),
            IcpTijdvak::Kwartaal => (
                "K",
                periode
                    .filter(|p| (1..=4).contains(p))
                    .ok_or("Geef het kwartaal, 1 tot 4")?,
            ),
            IcpTijdvak::Jaar => ("J", jaar as u32),
        };
        Ok(Self { jaar, soort, id })
    }

    /// The period an opgave is drawn up for: the screen offers 2010 up to this year.
    fn opgave(
        jaar: i32,
        tijdvak: Option<IcpTijdvak>,
        periode: Option<u32>,
    ) -> Result<Self, String> {
        let this_year = today().0;
        if !(2010..=this_year).contains(&jaar) {
            return Err(format!(
                "Een ICP-opgave kan over 2010 tot en met {this_year}"
            ));
        }
        Self::new(jaar, tijdvak, periode)
    }

    /// From what e-Boekhouden gives for an opgave or a line: `jaar`, `periodType` and `period` or `periodId`.
    /// None when one of them is missing or out of range.
    fn from_record(record: &Value, id_key: &str) -> Option<Self> {
        let jaar = number(record.get("jaar")).ok()? as i32;
        let tijdvak = match record.get("periodType").and_then(Value::as_str)? {
            "M" => IcpTijdvak::Maand,
            "2M" => IcpTijdvak::TweeMaanden,
            "K" => IcpTijdvak::Kwartaal,
            "J" => IcpTijdvak::Jaar,
            _ => return None,
        };
        // A missing or odd period number is no period: the checks of `new` decide, so nothing indexes out of range.
        let id = number(record.get(id_key)).ok()?;
        let id = (id.fract() == 0.0 && (1.0..=12.0).contains(&id)).then_some(id as u32);
        Self::new(jaar, Some(tijdvak), id).ok()
    }

    fn first_month(self) -> u32 {
        match self.soort {
            "M" => self.id,
            "2M" | "K" => self.id * 3 - 2,
            _ => 1,
        }
    }

    /// The first day of the period, as the screen's `setIcpDatum`.
    fn start(self) -> (i32, u32, u32) {
        (self.jaar, self.first_month(), 1)
    }

    /// The first day after the period, as the line form's checks count it.
    fn after(self) -> (i32, u32, u32) {
        let months = match self.soort {
            "M" => 1,
            "2M" => 2,
            "K" => 3,
            _ => 12,
        };
        let next = self.first_month() + months;
        if next > 12 {
            (self.jaar + 1, next - 12, 1)
        } else {
            (self.jaar, next, 1)
        }
    }

    fn naam(self) -> String {
        match self.soort {
            "M" => format!("{} {}", MAANDEN[(self.id - 1) as usize], self.jaar),
            "2M" => {
                let first = self.first_month() as usize;
                format!("{}/{} {}", MAANDEN[first - 1], MAANDEN[first], self.jaar)
            }
            "K" => format!("{}e kwartaal {}", self.id, self.jaar),
            _ => format!("jaar {}", self.jaar),
        }
    }

    fn path(self) -> String {
        format!("icpopgave/{}/{}/{}", self.jaar, self.soort, self.id)
    }
}
