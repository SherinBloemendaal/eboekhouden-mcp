//! What the tools that change an account's books send to e-Boekhouden, and how the user sees it first.
//!
//! Every shape here was taken from e-Boekhouden's own web app and checked against a real administration
//! (2026-11-11): the mutatie screens (`POST`/`PUT mutatie`), the invoice form (`factuur/model`) and the
//! subscription form (`abonnement/model`). Nothing in this file talks to e-Boekhouden: it turns a tool's
//! arguments, plus what was read from the books, into a request body and a summary. tools.rs sends it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::approvals::{Line, line};
use crate::tools::{dashed, parse_date};

type Date = (i32, u32, u32);

/// A request body with the words the user approves it by.
#[derive(Debug)]
pub struct Built {
    pub body: Value,
    pub title: String,
    pub lines: Vec<Line>,
}

// ---------------------------------------------------------------- the books

/// A grootboekrekening, as tools name it (by `code`) and as the API wants it (by `id`).
#[derive(Clone, Debug, PartialEq)]
pub struct Rekening {
    pub id: u64,
    pub code: String,
    pub naam: String,
    pub categorie: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Btw {
    pub code: String,
    pub naam: String,
    pub percentage: f64,
    /// "INK", "VERK" or "" (Geen btw).
    pub soort: String,
}

/// A kostenplaats as `GET kostenplaats` lists it: a node in a tree, named by its path ("Projecten>Noord").
#[derive(Clone, Debug, PartialEq)]
pub struct Kostenplaats {
    pub id: u64,
    pub omschrijving: String,
    /// 0 for one at the top of the tree.
    pub parent: u64,
    pub actief: bool,
    pub pad: String,
}

/// The kostenplaatsen of `GET kostenplaats` (e-Boekhouden's own list, `getAll` of its kostenplaats service).
pub fn kostenplaatsen_from_api(list: &Value) -> Result<Vec<Kostenplaats>, String> {
    let rows = list
        .as_array()
        .ok_or("De kostenplaatsen van e-Boekhouden hebben een onbekende vorm")?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            let omschrijving = row
                .get("omschrijving")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            Some(Kostenplaats {
                id: row.get("id")?.as_u64()?,
                pad: row
                    .get("volledigPad")
                    .and_then(Value::as_str)
                    .filter(|p| !p.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| omschrijving.clone()),
                omschrijving,
                parent: row
                    .get("parentKostenplaatsId")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                actief: row.get("actief").and_then(Value::as_bool).unwrap_or(true),
            })
        })
        .collect())
}

/// Whether one of the lines a mutatie tool was given names a kostenplaats: only then are the kostenplaatsen
/// read and checked.
pub fn names_kostenplaats(regels: Option<&[RegelArg]>) -> bool {
    regels.is_some_and(|r| r.iter().any(|r| r.kostenplaats_id.unwrap_or(0) > 0))
}

/// Whether a mutatie as `GET mutatie/nr/{nr}/model` gives it has a kostenplaats on one of its lines.
pub fn model_has_kostenplaats(model: &Value) -> bool {
    model
        .get("mutatieRegels")
        .and_then(Value::as_array)
        .is_some_and(|rows| {
            rows.iter()
                .any(|r| r.get("kostenplaats").and_then(Value::as_u64).unwrap_or(0) > 0)
        })
}

/// The rekeningschema and the btw codes of one administration, read before a change is built.
#[derive(Clone, Debug, Default)]
pub struct Books {
    pub rekeningen: Vec<Rekening>,
    pub btw: Vec<Btw>,
    /// Read only for a mutatie that has a kostenplaats on a line (tools.rs); empty otherwise.
    pub kostenplaatsen: Vec<Kostenplaats>,
}

impl Books {
    /// From `grootboekrekening/gridtable` (rows as lists, names in `colMetadata`) and `btwinfo/all`.
    pub fn from_api(grootboek: &Value, btw: &Value) -> Result<Self, String> {
        let columns: Vec<&str> = grootboek
            .get("colMetadata")
            .and_then(Value::as_array)
            .map(|c| c.iter().filter_map(|x| x.get("name")?.as_str()).collect())
            .unwrap_or_default();
        let at = |name: &str| columns.iter().position(|c| *c == name);
        let (Some(id), Some(code), Some(naam), Some(categorie)) =
            (at("id"), at("code"), at("omschrijving"), at("categorie"))
        else {
            return Err("Het rekeningschema van e-Boekhouden heeft een onbekende vorm".into());
        };
        let inactief = at("inactief");
        let rekeningen = grootboek
            .get("data")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(Value::as_array)
                    .filter(|row| {
                        !inactief
                            .and_then(|i| row.get(i))
                            .is_some_and(|v| v.as_bool() == Some(true) || v.as_u64() == Some(1))
                    })
                    .filter_map(|row| {
                        Some(Rekening {
                            id: row.get(id)?.as_u64()?,
                            code: row.get(code)?.as_str()?.trim().to_owned(),
                            naam: row.get(naam)?.as_str().unwrap_or_default().to_owned(),
                            categorie: row.get(categorie)?.as_str().unwrap_or_default().to_owned(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let btw = btw
            .as_array()
            .map(|codes| {
                codes
                    .iter()
                    .filter_map(|c| {
                        Some(Btw {
                            code: c.get("code")?.as_str()?.to_owned(),
                            naam: c
                                .get("omschrijving")?
                                .as_str()
                                .unwrap_or_default()
                                .to_owned(),
                            percentage: c.get("percentage").and_then(Value::as_f64).unwrap_or(0.0),
                            soort: c
                                .get("soort")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            rekeningen,
            btw,
            kostenplaatsen: Vec::new(),
        })
    }

    /// A kostenplaats a line may carry: one of the administration, and active (e-Boekhouden's own picker
    /// refuses an inactive one).
    fn kostenplaats(&self, id: u64) -> Result<&Kostenplaats, String> {
        if self.kostenplaatsen.is_empty() {
            return Err("Deze administratie heeft geen kostenplaatsen".into());
        }
        let found = self
            .kostenplaatsen
            .iter()
            .find(|k| k.id == id)
            .ok_or_else(|| {
                format!("Geen kostenplaats met id {id} in deze administratie (zie kostenplaatsen)")
            })?;
        if !found.actief {
            return Err(format!("Kostenplaats {} is inactief", found.pad));
        }
        Ok(found)
    }

    fn kostenplaats_label(&self, id: u64) -> String {
        match self.kostenplaatsen.iter().find(|k| k.id == id) {
            Some(k) => k.pad.clone(),
            None => format!("id {id}"),
        }
    }

    pub fn rekening(&self, code: &str) -> Result<&Rekening, String> {
        let code = code.trim();
        self.rekeningen
            .iter()
            .find(|r| r.code == code)
            .ok_or_else(|| {
                format!("Geen grootboekrekening met code {code:?} in deze administratie")
            })
    }

    fn by_id(&self, id: u64) -> Option<&Rekening> {
        self.rekeningen.iter().find(|r| r.id == id)
    }

    /// The one rekening of a category (Debiteuren, Crediteuren), for the screens that pick it by themselves.
    fn only(&self, categorie: &str, what: &str) -> Result<&Rekening, String> {
        let found: Vec<&Rekening> = self
            .rekeningen
            .iter()
            .filter(|r| r.categorie.eq_ignore_ascii_case(categorie))
            .collect();
        match found.as_slice() {
            [one] => Ok(one),
            [] => Err(format!("Deze administratie heeft geen {what}")),
            more => Err(format!(
                "Deze administratie heeft meerdere {what}en: geef met rekening aan welke ({})",
                more.iter()
                    .map(|r| r.code.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    pub fn debiteuren(&self) -> Result<&Rekening, String> {
        self.only("Debiteuren", "debiteurenrekening")
    }

    fn crediteuren(&self) -> Result<&Rekening, String> {
        self.only("Crediteuren", "crediteurenrekening")
    }

    pub fn btw(&self, code: &str) -> Result<&Btw, String> {
        let code = code.trim();
        self.btw
            .iter()
            .find(|b| b.code.eq_ignore_ascii_case(code))
            .ok_or_else(|| {
                format!(
                    "Onbekende btw-code {code:?}. Kies uit: {}",
                    self.btw
                        .iter()
                        .map(|b| b.code.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
    }

    fn label(&self, id: u64) -> String {
        match self.by_id(id) {
            Some(r) => format!("{} {}", r.code, r.naam),
            None => format!("rekening {id}"),
        }
    }
}

// ---------------------------------------------------------------- small helpers

fn round2(amount: f64) -> f64 {
    round_cents(amount)
}

/// An amount in whole cents, rounded the way e-Boekhouden's own forms do it (`round(n, 2)` in the web app):
/// the amount in cents is first written with three decimals, which takes the float error out, and then
/// goes up from a first decimal of 5, away from zero. So 3,50 plus 21% (4,235, held as 423.49999999999994
/// cents) becomes 4,24 as on their screen, where plain rounding gives 4,23.
pub(crate) fn round_cents(amount: f64) -> f64 {
    let text = format!("{:.3}", amount * 100.0);
    let Some((whole, decimals)) = text.split_once('.') else {
        return amount;
    };
    let negative = whole.starts_with('-');
    let cents: f64 = whole.parse().unwrap_or(0.0);
    let up = decimals.as_bytes().first().is_some_and(|d| *d >= b'5');
    let cents = match (up, negative) {
        (true, true) => cents - 1.0,
        (true, false) => cents + 1.0,
        (false, _) => cents,
    };
    let rounded = cents / 100.0;
    if rounded == 0.0 { 0.0 } else { rounded }
}

fn round4(amount: f64) -> f64 {
    (amount * 10_000.0).round() / 10_000.0
}

/// "1.234,50" would mislead next to e-Boekhouden's own "1234,50"; this is its plain form.
pub fn euro(amount: f64) -> String {
    format!("{:.2}", round2(amount)).replace('.', ",")
}

fn dutch((y, m, d): Date) -> String {
    format!("{d:02}-{m:02}-{y}")
}

fn date_object((y, m, d): Date) -> Value {
    json!({"day": d, "month": m, "year": y})
}

/// A date as the API gives it: "2026-11-06T00:00:00" or {"day":6,"month":11,"year":2026}.
fn date_of(value: &Value) -> Option<Date> {
    if let Some(text) = value.as_str() {
        return parse_date(text.get(..10)?).ok();
    }
    let part = |name: &str| value.get(name).and_then(Value::as_u64);
    let (y, m, d) = (part("year")?, part("month")?, part("day")?);
    (y > 1).then_some((y as i32, m as u32, d as u32))
}

fn amount(value: f64, what: &str) -> Result<f64, String> {
    if !value.is_finite() || value.abs() > 1e11 {
        return Err(format!("{what} is geen geldig bedrag"));
    }
    Ok(round2(value))
}

fn text(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
}

// ---------------------------------------------------------------- mutaties

/// The seven kinds of mutatie of e-Boekhouden's three entry screens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Soort {
    /// Invoeren > Facturen: een inkoopfactuur.
    FactuurOntvangen,
    /// Invoeren > Facturen: een verkoopfactuur.
    FactuurVerstuurd,
    /// Invoeren > Afschrift/bonnetjes: een klant betaalt een factuur.
    FactuurbetalingOntvangen,
    /// Invoeren > Afschrift/bonnetjes: een factuur van een leverancier betalen.
    FactuurbetalingVerstuurd,
    /// Invoeren > Afschrift/bonnetjes.
    GeldOntvangen,
    /// Invoeren > Afschrift/bonnetjes.
    GeldUitgegeven,
    /// Invoeren > Memoriaal.
    Memoriaal,
}

impl Soort {
    pub fn nr(self) -> u8 {
        match self {
            Self::FactuurOntvangen => 1,
            Self::FactuurVerstuurd => 2,
            Self::FactuurbetalingOntvangen => 3,
            Self::FactuurbetalingVerstuurd => 4,
            Self::GeldOntvangen => 5,
            Self::GeldUitgegeven => 6,
            Self::Memoriaal => 7,
        }
    }

    fn from_nr(nr: u64) -> Option<Self> {
        Some(match nr {
            1 => Self::FactuurOntvangen,
            2 => Self::FactuurVerstuurd,
            3 => Self::FactuurbetalingOntvangen,
            4 => Self::FactuurbetalingVerstuurd,
            5 => Self::GeldOntvangen,
            6 => Self::GeldUitgegeven,
            7 => Self::Memoriaal,
            _ => return None,
        })
    }

    pub fn naam(self) -> &'static str {
        match self {
            Self::FactuurOntvangen => "Factuur ontvangen",
            Self::FactuurVerstuurd => "Factuur verstuurd",
            Self::FactuurbetalingOntvangen => "Factuurbetaling ontvangen",
            Self::FactuurbetalingVerstuurd => "Factuurbetaling verstuurd",
            Self::GeldOntvangen => "Geld ontvangen",
            Self::GeldUitgegeven => "Geld uitgegeven",
            Self::Memoriaal => "Memoriaal",
        }
    }

    fn is_factuur(self) -> bool {
        matches!(self, Self::FactuurOntvangen | Self::FactuurVerstuurd)
    }

    fn is_betaling(self) -> bool {
        matches!(
            self,
            Self::FactuurbetalingOntvangen | Self::FactuurbetalingVerstuurd
        )
    }
}

/// Whether the amounts of the lines include btw.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum InEx {
    Incl,
    Excl,
}

impl InEx {
    fn code(self) -> &'static str {
        match self {
            Self::Incl => "IN",
            Self::Excl => "EX",
        }
    }

    fn from_code(code: &str) -> Self {
        if code.eq_ignore_ascii_case("IN") {
            Self::Incl
        } else {
            Self::Excl
        }
    }

    fn naam(self) -> &'static str {
        match self {
            Self::Incl => "incl. btw",
            Self::Excl => "excl. btw",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct RegelArg {
    /// Code van de tegenrekening in het rekeningschema, bijvoorbeeld "4550".
    pub tegenrekening: String,
    /// Bedrag van de regel, incl. of excl. btw volgens `bedragen`.
    pub bedrag: f64,
    /// Btw-code zoals e-Boekhouden hem noemt, bijvoorbeeld HOOG_INK_21, HOOG_VERK_21, LAAG_VERK_9 of GEEN (standaard GEEN).
    pub btw_code: Option<String>,
    /// Eigen omschrijving van deze regel (standaard die van de mutatie).
    pub omschrijving: Option<String>,
    /// Id van een actieve kostenplaats voor deze regel (uit kostenplaatsen). Alleen als de administratie kostenplaatsen gebruikt; 0 of weglaten is geen kostenplaats.
    #[serde(default)]
    pub kostenplaats_id: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct BetalingArg {
    /// Id van de relatie van de factuur.
    pub relatie_id: u64,
    /// Factuurnummer van de open post die betaald wordt.
    pub factuur: String,
    /// Betaald bedrag.
    pub bedrag: f64,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct MutatieToevoegenArgs {
    /// Soort mutatie: factuur_ontvangen, factuur_verstuurd, factuurbetaling_ontvangen, factuurbetaling_verstuurd, geld_ontvangen, geld_uitgegeven of memoriaal.
    pub soort: Soort,
    /// Datum, JJJJ-MM-DD.
    pub datum: String,
    /// Code van de rekening: de bank- of kasrekening bij geld en betalingen, de rekening bij memoriaal. Bij facturen mag hij weg: dan is het de debiteuren- of crediteurenrekening.
    pub rekening: Option<String>,
    /// Omschrijving van de mutatie.
    pub omschrijving: Option<String>,
    /// Id van de relatie. Verplicht bij factuur_ontvangen en factuur_verstuurd.
    pub relatie_id: Option<u64>,
    /// Factuurnummer. Verplicht bij factuur_ontvangen en factuur_verstuurd.
    pub factuur: Option<String>,
    /// Betalingstermijn in dagen (facturen).
    pub termijn: Option<u32>,
    /// Zijn de bedragen incl of excl btw (standaard incl).
    pub bedragen: Option<InEx>,
    /// De regels van de mutatie. Niet bij factuurbetalingen: gebruik daar `betalingen`.
    pub regels: Option<Vec<RegelArg>>,
    /// Bij factuurbetaling_ontvangen en factuurbetaling_verstuurd: welke facturen er betaald worden.
    pub betalingen: Option<Vec<BetalingArg>>,
    /// Boekstuknummer.
    pub boekstuk: Option<String>,
    /// Id van een onverwerkte bankregel (uit bankregels): die is daarmee verwerkt.
    pub bankregel_id: Option<u64>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct MutatieWijzigenArgs {
    /// Mutatienummer.
    pub nr: u64,
    /// Nieuwe datum, JJJJ-MM-DD.
    pub datum: Option<String>,
    /// Nieuwe rekening (code).
    pub rekening: Option<String>,
    /// Nieuwe omschrijving.
    pub omschrijving: Option<String>,
    /// Nieuwe relatie (id).
    pub relatie_id: Option<u64>,
    /// Nieuw factuurnummer.
    pub factuur: Option<String>,
    /// Nieuwe betalingstermijn in dagen.
    pub termijn: Option<u32>,
    /// Bedragen incl of excl btw.
    pub bedragen: Option<InEx>,
    /// Vervangt alle regels van de mutatie. Laat weg om ze te houden.
    pub regels: Option<Vec<RegelArg>>,
    /// Vervangt alle betalingen (bij een factuurbetaling). Laat weg om ze te houden.
    pub betalingen: Option<Vec<BetalingArg>>,
    /// Nieuw boekstuknummer.
    pub boekstuk: Option<String>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct Regel {
    tegenrekening: u64,
    bedrag: f64,
    btw: String,
    omschrijving: Option<String>,
    kostenplaats: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
struct Betaling {
    relatie_id: u64,
    factuur: String,
    bedrag: f64,
}

/// A mutatie in between the tool's arguments (or the mutatie as it is now) and the request body.
#[derive(Clone, Debug, PartialEq)]
struct Mutatie {
    soort: Soort,
    datum: Date,
    rekening: u64,
    omschrijving: String,
    relatie_id: Option<u64>,
    factuur: Option<String>,
    termijn: Option<u32>,
    in_ex: InEx,
    boekstuk: Option<String>,
    betalingskenmerk: Option<String>,
    regels: Vec<Regel>,
    betalingen: Vec<Betaling>,
}

const MAX_REGELS: usize = 100;

fn regels_from(args: &[RegelArg], books: &Books) -> Result<Vec<Regel>, String> {
    if args.is_empty() {
        return Err("Geef minstens één regel".into());
    }
    if args.len() > MAX_REGELS {
        return Err(format!("Hoogstens {MAX_REGELS} regels per mutatie"));
    }
    args.iter()
        .enumerate()
        .map(|(i, regel)| {
            let bedrag = amount(regel.bedrag, &format!("Het bedrag van regel {}", i + 1))?;
            if bedrag == 0.0 {
                return Err(format!("Het bedrag van regel {} is nul", i + 1));
            }
            Ok(Regel {
                tegenrekening: books.rekening(&regel.tegenrekening)?.id,
                bedrag,
                btw: books
                    .btw(regel.btw_code.as_deref().unwrap_or("GEEN"))?
                    .code
                    .clone(),
                omschrijving: text(&regel.omschrijving),
                kostenplaats: regel
                    .kostenplaats_id
                    .filter(|id| *id > 0)
                    .map(|id| books.kostenplaats(id).map(|k| k.id))
                    .transpose()?,
            })
        })
        .collect()
}

fn betalingen_from(args: &[BetalingArg]) -> Result<Vec<Betaling>, String> {
    if args.is_empty() {
        return Err("Geef minstens één betaling".into());
    }
    if args.len() > MAX_REGELS {
        return Err(format!("Hoogstens {MAX_REGELS} betalingen per mutatie"));
    }
    args.iter()
        .enumerate()
        .map(|(i, betaling)| {
            let factuur = betaling.factuur.trim().to_owned();
            if factuur.is_empty() {
                return Err(format!("Betaling {} heeft geen factuurnummer", i + 1));
            }
            let bedrag = amount(
                betaling.bedrag,
                &format!("Het bedrag van betaling {}", i + 1),
            )?;
            if bedrag == 0.0 {
                return Err(format!("Het bedrag van betaling {} is nul", i + 1));
            }
            Ok(Betaling {
                relatie_id: betaling.relatie_id,
                factuur,
                bedrag,
            })
        })
        .collect()
}

impl Mutatie {
    /// What e-Boekhouden's own screens insist on, checked here because the API answers some of these with a bare 500.
    fn check(&self) -> Result<(), String> {
        if self.soort.is_factuur() {
            if self.relatie_id.unwrap_or(0) == 0 {
                return Err(format!(
                    "{} heeft een relatie nodig (relatie_id)",
                    self.soort.naam()
                ));
            }
            if self.factuur.as_deref().unwrap_or_default().is_empty() {
                return Err(format!(
                    "{} heeft een factuurnummer nodig (factuur)",
                    self.soort.naam()
                ));
            }
        }
        if self.soort.is_betaling() {
            if self.betalingen.is_empty() {
                return Err(format!("{} heeft betalingen nodig", self.soort.naam()));
            }
        } else if self.regels.is_empty() {
            return Err(format!("{} heeft regels nodig", self.soort.naam()));
        }
        Ok(())
    }

    fn body(&self, nr: Option<u64>, bankregel: Option<u64>) -> Value {
        let mut mutatie = Map::new();
        mutatie.insert("rekening".into(), json!(self.rekening));
        mutatie.insert("datum".into(), json!(dashed(self.datum)));
        mutatie.insert("soort".into(), json!(self.soort.nr()));
        mutatie.insert("omschrijving".into(), json!(self.omschrijving));
        if !self.soort.is_betaling() {
            mutatie.insert("inEx".into(), json!(self.in_ex.code()));
        }
        if self.soort.is_factuur() || self.soort == Soort::Memoriaal {
            if let Some(relatie) = self.relatie_id.filter(|id| *id > 0) {
                mutatie.insert("relatieId".into(), json!(relatie));
            }
            if let Some(factuur) = self.factuur.as_deref().filter(|f| !f.is_empty()) {
                mutatie.insert("factuur".into(), json!(factuur));
            }
            if let Some(termijn) = self.termijn {
                mutatie.insert("termijn".into(), json!(termijn));
            }
        }
        if let Some(boekstuk) = &self.boekstuk {
            mutatie.insert("boekstuk".into(), json!(boekstuk));
        }
        if let Some(kenmerk) = &self.betalingskenmerk {
            mutatie.insert("betalingsKenmerk".into(), json!(kenmerk));
        }
        if let Some(nr) = nr {
            mutatie.insert("nr".into(), json!(nr));
        }
        let regels: Vec<Value> = if self.soort.is_betaling() {
            self.betalingen
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    json!({"index": i, "bedrag": b.bedrag, "btw": 0, "btwCode": "GEEN", "relatieId": b.relatie_id, "factuur": b.factuur})
                })
                .collect()
        } else {
            self.regels
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    let mut regel = Map::new();
                    regel.insert("index".into(), json!(i));
                    regel.insert("tegenRekening".into(), json!(r.tegenrekening));
                    regel.insert("btwCode".into(), json!(r.btw));
                    regel.insert("bedrag".into(), json!(r.bedrag));
                    if let Some(omschrijving) = &r.omschrijving {
                        regel.insert("omschrijving".into(), json!(omschrijving));
                    }
                    if let Some(kostenplaats) = r.kostenplaats.filter(|k| *k > 0) {
                        regel.insert("kostenplaats".into(), json!(kostenplaats));
                    }
                    Value::Object(regel)
                })
                .collect()
        };
        let mut body = Map::new();
        body.insert("mutatie".into(), Value::Object(mutatie));
        body.insert("mutatieRegels".into(), Value::Array(regels));
        if let Some(nr) = nr {
            // The edit screen sends the mutatie's own number here; it is the key its files hang on.
            body.insert("folderKoppelId".into(), json!(nr));
        }
        if let Some(bankregel) = bankregel {
            body.insert("importId".into(), json!(bankregel));
        }
        Value::Object(body)
    }

    fn lines(&self, books: &Books, bankregel: Option<u64>) -> Vec<Line> {
        let mut lines = vec![
            line("Soort", self.soort.naam()),
            line("Datum", dutch(self.datum)),
            line("Rekening", books.label(self.rekening)),
        ];
        if let Some(relatie) = self.relatie_id.filter(|id| *id > 0) {
            lines.push(line("Relatie", format!("id {relatie}")));
        }
        if let Some(factuur) = self.factuur.as_deref().filter(|f| !f.is_empty()) {
            lines.push(line("Factuur", factuur));
        }
        if let Some(termijn) = self.termijn.filter(|_| self.soort.is_factuur()) {
            lines.push(line("Betalingstermijn", format!("{termijn} dagen")));
        }
        if !self.omschrijving.is_empty() {
            lines.push(line("Omschrijving", &self.omschrijving));
        }
        if let Some(boekstuk) = &self.boekstuk {
            lines.push(line("Boekstuk", boekstuk));
        }
        if self.soort.is_betaling() {
            for (i, b) in self.betalingen.iter().enumerate() {
                lines.push(line(
                    &format!("Betaling {}", i + 1),
                    format!(
                        "factuur {} van relatie {}: {}",
                        b.factuur,
                        b.relatie_id,
                        euro(b.bedrag)
                    ),
                ));
            }
        } else {
            lines.push(line("Bedragen", self.in_ex.naam()));
            for (i, r) in self.regels.iter().enumerate() {
                let btw = books
                    .btw(&r.btw)
                    .map(|b| b.naam.clone())
                    .unwrap_or_else(|_| r.btw.clone());
                let kostenplaats = r
                    .kostenplaats
                    .map(|id| format!(", kostenplaats {}", books.kostenplaats_label(id)))
                    .unwrap_or_default();
                lines.push(line(
                    &format!("Regel {}", i + 1),
                    format!(
                        "{}: {} ({btw}){kostenplaats}",
                        books.label(r.tegenrekening),
                        euro(r.bedrag)
                    ),
                ));
            }
        }
        if let Some(bankregel) = bankregel {
            lines.push(line(
                "Bankregel",
                format!("id {bankregel}, daarmee verwerkt"),
            ));
        }
        lines
    }

    /// The mutatie as `GET mutatie/nr/{nr}/model` gives it.
    fn from_model(model: &Value) -> Result<Self, String> {
        let bad = || "e-Boekhouden gaf deze mutatie in een onbekende vorm terug".to_owned();
        let mutatie = model.get("mutatie").ok_or_else(bad)?;
        let soort = mutatie
            .get("soort")
            .and_then(Value::as_u64)
            .and_then(Soort::from_nr)
            .ok_or("Deze soort mutatie (zoals een openingssaldo) kan EB MCP niet wijzigen")?;
        let string = |name: &str| {
            mutatie
                .get(name)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .filter(|s| !s.is_empty())
        };
        let rows = model
            .get("mutatieRegels")
            .and_then(Value::as_array)
            .ok_or_else(bad)?;
        let mut regels = Vec::new();
        let mut betalingen = Vec::new();
        for row in rows {
            let bedrag = row.get("bedrag").and_then(Value::as_f64).ok_or_else(bad)?;
            if soort.is_betaling() {
                let post = row.get("openPostModel").ok_or_else(bad)?;
                betalingen.push(Betaling {
                    relatie_id: post
                        .get("relatieId")
                        .and_then(Value::as_u64)
                        .ok_or_else(bad)?,
                    factuur: post
                        .get("factuurNummer")
                        .and_then(Value::as_str)
                        .ok_or_else(bad)?
                        .to_owned(),
                    bedrag,
                });
            } else {
                regels.push(Regel {
                    tegenrekening: row
                        .get("tegenRekening")
                        .and_then(Value::as_u64)
                        .ok_or_else(bad)?,
                    bedrag,
                    btw: row
                        .get("btwCode")
                        .and_then(Value::as_str)
                        .unwrap_or("GEEN")
                        .to_owned(),
                    omschrijving: row
                        .get("omschrijving")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .filter(|s| !s.is_empty()),
                    kostenplaats: row
                        .get("kostenplaats")
                        .and_then(Value::as_u64)
                        .filter(|k| *k > 0),
                });
            }
        }
        Ok(Self {
            soort,
            datum: mutatie.get("datum").and_then(date_of).ok_or_else(bad)?,
            rekening: mutatie
                .get("rekening")
                .and_then(Value::as_u64)
                .ok_or_else(bad)?,
            omschrijving: string("omschrijving").unwrap_or_default(),
            relatie_id: mutatie
                .get("relatieId")
                .and_then(Value::as_u64)
                .filter(|id| *id > 0),
            factuur: string("factuur"),
            termijn: mutatie
                .get("termijn")
                .and_then(Value::as_u64)
                .map(|t| t as u32),
            in_ex: InEx::from_code(mutatie.get("inEx").and_then(Value::as_str).unwrap_or("EX")),
            boekstuk: string("boekstuk"),
            betalingskenmerk: string("betalingsKenmerk"),
            regels,
            betalingen,
        })
    }
}

pub fn mutatie_nieuw(args: &MutatieToevoegenArgs, books: &Books) -> Result<Built, String> {
    let soort = args.soort;
    let rekening = match (&args.rekening, soort) {
        (Some(code), _) => books.rekening(code)?.id,
        (None, Soort::FactuurOntvangen) => books.crediteuren()?.id,
        (None, Soort::FactuurVerstuurd) => books.debiteuren()?.id,
        (None, _) => {
            return Err(format!(
                "{} heeft een rekening nodig (de code, bijvoorbeeld van de bank)",
                soort.naam()
            ));
        }
    };
    let (regels, betalingen) = if soort.is_betaling() {
        if args.regels.is_some() {
            return Err("Een factuurbetaling heeft betalingen, geen regels".into());
        }
        (
            Vec::new(),
            betalingen_from(args.betalingen.as_deref().unwrap_or_default())?,
        )
    } else {
        if args.betalingen.is_some() {
            return Err("Betalingen horen alleen bij een factuurbetaling".into());
        }
        (
            regels_from(args.regels.as_deref().unwrap_or_default(), books)?,
            Vec::new(),
        )
    };
    let mutatie = Mutatie {
        soort,
        datum: parse_date(&args.datum)?,
        rekening,
        omschrijving: text(&args.omschrijving).unwrap_or_default(),
        relatie_id: args.relatie_id,
        factuur: text(&args.factuur),
        termijn: args.termijn,
        // Memoriaal has no incl/excl choice on its screen: it always sends EX.
        in_ex: if soort == Soort::Memoriaal {
            InEx::Excl
        } else {
            args.bedragen.unwrap_or(InEx::Incl)
        },
        boekstuk: text(&args.boekstuk),
        betalingskenmerk: None,
        regels,
        betalingen,
    };
    mutatie.check()?;
    Ok(Built {
        body: mutatie.body(None, args.bankregel_id),
        title: format!("Mutatie toevoegen: {}", soort.naam()),
        lines: mutatie.lines(books, args.bankregel_id),
    })
}

pub fn mutatie_gewijzigd(
    args: &MutatieWijzigenArgs,
    model: &Value,
    books: &Books,
) -> Result<Built, String> {
    let mut mutatie = Mutatie::from_model(model)?;
    let before = mutatie.clone();
    if let Some(datum) = &args.datum {
        mutatie.datum = parse_date(datum)?;
    }
    if let Some(code) = &args.rekening {
        mutatie.rekening = books.rekening(code)?.id;
    }
    if let Some(omschrijving) = &args.omschrijving {
        let was = mutatie.omschrijving.clone();
        mutatie.omschrijving = omschrijving.trim().to_owned();
        // A line that only repeated the mutatie's omschrijving follows it, as on e-Boekhouden's screen.
        for regel in &mut mutatie.regels {
            if regel.omschrijving.as_deref() == Some(was.as_str()) {
                regel.omschrijving = None;
            }
        }
    }
    if let Some(relatie) = args.relatie_id {
        mutatie.relatie_id = Some(relatie);
    }
    if let Some(factuur) = &args.factuur {
        mutatie.factuur = text(&Some(factuur.clone()));
    }
    if let Some(termijn) = args.termijn {
        mutatie.termijn = Some(termijn);
    }
    if let Some(in_ex) = args.bedragen {
        if mutatie.soort == Soort::Memoriaal || mutatie.soort.is_betaling() {
            return Err(format!(
                "{} kent geen incl of excl btw",
                mutatie.soort.naam()
            ));
        }
        mutatie.in_ex = in_ex;
    }
    if let Some(boekstuk) = &args.boekstuk {
        mutatie.boekstuk = text(&Some(boekstuk.clone()));
    }
    if let Some(regels) = &args.regels {
        if mutatie.soort.is_betaling() {
            return Err("Een factuurbetaling heeft betalingen, geen regels".into());
        }
        mutatie.regels = regels_from(regels, books)?;
    }
    if let Some(betalingen) = &args.betalingen {
        if !mutatie.soort.is_betaling() {
            return Err("Betalingen horen alleen bij een factuurbetaling".into());
        }
        mutatie.betalingen = betalingen_from(betalingen)?;
    }
    mutatie.check()?;
    if mutatie == before {
        return Err("Er is niets gewijzigd: geef aan wat er anders moet".into());
    }
    let mut lines = vec![line("Mutatie", format!("nr {}", args.nr))];
    lines.extend(mutatie.lines(books, None));
    // New lines replace the old ones: a kostenplaats an old line had and no new line carries lapses, said aloud.
    let mut vervalt: Vec<u64> = Vec::new();
    for id in before.regels.iter().filter_map(|r| r.kostenplaats) {
        if !mutatie.regels.iter().any(|r| r.kostenplaats == Some(id)) && !vervalt.contains(&id) {
            vervalt.push(id);
        }
    }
    for id in vervalt {
        lines.push(line(
            "Kostenplaats",
            format!("kostenplaats {} vervalt", books.kostenplaats_label(id)),
        ));
    }
    Ok(Built {
        body: mutatie.body(Some(args.nr), None),
        title: format!("Mutatie {} wijzigen ({})", args.nr, mutatie.soort.naam()),
        lines,
    })
}

// ---------------------------------------------------------------- facturen en abonnementen

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct FactuurRegelArg {
    /// Aantal (standaard 1).
    pub aantal: Option<f64>,
    /// Omschrijving van het product of de dienst.
    pub omschrijving: String,
    /// Prijs per eenheid, incl. of excl. btw volgens `bedragen`.
    pub prijs: f64,
    /// Btw-code voor verkoop, bijvoorbeeld HOOG_VERK_21, LAAG_VERK_9 of GEEN.
    pub btw_code: String,
    /// Code van de omzetrekening in het rekeningschema, bijvoorbeeld "8000".
    pub tegenrekening: String,
    /// Productcode (optioneel).
    pub code: Option<String>,
}

/// A line of an invoice or subscription as `factuur/model` and `abonnement/model` take it in `factuurItems.insert`.
fn factuur_item(
    regel: &FactuurRegelArg,
    index: usize,
    in_ex: InEx,
    books: &Books,
) -> Result<(Value, Line), String> {
    let n = index + 1;
    let omschrijving = regel.omschrijving.trim();
    if omschrijving.is_empty() {
        return Err(format!("Regel {n} heeft geen omschrijving"));
    }
    let aantal = regel.aantal.unwrap_or(1.0);
    if !aantal.is_finite() || aantal == 0.0 {
        return Err(format!("Het aantal van regel {n} is geen geldig getal"));
    }
    let prijs = amount(regel.prijs, &format!("De prijs van regel {n}"))?;
    let btw = books.btw(&regel.btw_code)?;
    if btw.soort == "INK" {
        return Err(format!(
            "Btw-code {} is voor inkoop; een factuur wil een verkoopcode of GEEN",
            btw.code
        ));
    }
    if btw.code.starts_with("AFW") {
        return Err("Een afwijkend btw-percentage kan EB MCP niet op een factuur zetten".into());
    }
    let factor = 1.0 + btw.percentage / 100.0;
    let (excl, incl) = match in_ex {
        InEx::Excl => (prijs, round2(prijs * factor)),
        InEx::Incl => (round4(prijs / factor), prijs),
    };
    let rekening = books.rekening(&regel.tegenrekening)?;
    let mut item = Map::new();
    item.insert("id".into(), json!(-(n as i64)));
    item.insert("factuurId".into(), json!(0));
    item.insert("aantal".into(), json!(aantal));
    item.insert("omschrijving".into(), json!(omschrijving));
    item.insert("prijs".into(), Value::Null);
    item.insert("btw".into(), json!(btw.code));
    item.insert("grootboekId".into(), json!(rekening.id));
    item.insert("prijsExcl".into(), json!(excl));
    item.insert("prijsIncl".into(), json!(incl));
    item.insert("kortingIsPerc".into(), json!(true));
    item.insert("volgorde".into(), json!(index));
    if let Some(code) = text(&regel.code) {
        item.insert("code".into(), json!(code));
    }
    let shown = line(
        &format!("Regel {n}"),
        format!(
            "{aantal} x {omschrijving}: {} per stuk ({}), op {} {}",
            euro(prijs),
            btw.naam,
            rekening.code,
            rekening.naam
        ),
    );
    Ok((Value::Object(item), shown))
}

fn factuur_items(
    regels: &[FactuurRegelArg],
    in_ex: InEx,
    books: &Books,
) -> Result<(Vec<Value>, Vec<Line>), String> {
    if regels.is_empty() {
        return Err("Geef minstens één regel".into());
    }
    if regels.len() > MAX_REGELS {
        return Err(format!("Hoogstens {MAX_REGELS} regels"));
    }
    let mut items = Vec::new();
    let mut lines = vec![line("Prijzen", in_ex.naam())];
    for (i, regel) in regels.iter().enumerate() {
        let (item, shown) = factuur_item(regel, i, in_ex, books)?;
        items.push(item);
        lines.push(shown);
    }
    Ok((items, lines))
}

fn item_ids(items: &Value) -> Vec<Value> {
    items
        .as_array()
        .map(|rows| rows.iter().filter_map(|r| r.get("id").cloned()).collect())
        .unwrap_or_default()
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FactuurToevoegenArgs {
    /// Id van de relatie (de klant).
    pub relatie_id: u64,
    /// Factuurdatum, JJJJ-MM-DD (standaard vandaag).
    pub datum: Option<String>,
    /// Factuurnummer (standaard het volgende nummer dat e-Boekhouden voorstelt).
    pub nummer: Option<String>,
    /// Betalingstermijn in dagen (standaard 0, zoals het formulier van e-Boekhouden).
    pub termijn: Option<u32>,
    /// Zijn de prijzen incl of excl btw (standaard excl).
    pub bedragen: Option<InEx>,
    /// De producten of diensten op de factuur.
    pub regels: Vec<FactuurRegelArg>,
    /// Tekst onderaan de factuur.
    pub factuurtekst: Option<String>,
    /// Kenmerk van de klant.
    pub kenmerk: Option<String>,
    /// De factuur meteen boeken als mutatie (standaard zoals het in e-Boekhouden is ingesteld).
    pub direct_boeken: Option<bool>,
    /// Id van het factuursjabloon voor de opmaak (uit sjablonen met soort factuur; standaard het sjabloon dat de administratie heeft ingesteld).
    pub sjabloon_id: Option<u64>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// The invoice layout template a tool asks for, checked against the administration's own
/// (`factuursjabloon/gridtable`, rows with names): it must be there and active. Gives its id and name.
pub fn factuur_sjabloon(id: u64, sjablonen: Option<&Value>) -> Result<(u64, String), String> {
    let rows: Vec<Value> = match sjablonen {
        Some(Value::Array(rows)) => rows.clone(),
        Some(grid) => grid
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        None => Vec::new(),
    };
    let naam = |row: &Value| {
        row.get("naam")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned()
    };
    let inactief = |row: &Value| {
        row.get("inactief")
            .is_some_and(|v| v.as_bool() == Some(true) || v.as_u64() == Some(1))
    };
    let actief = || {
        rows.iter()
            .filter(|r| !inactief(r))
            .filter_map(|r| Some(format!("{} ({})", naam(r), r.get("id")?.as_u64()?)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match rows
        .iter()
        .find(|r| r.get("id").and_then(Value::as_u64) == Some(id))
    {
        None => Err(format!(
            "Geen factuursjabloon met id {id} in deze administratie. Kies uit: {}",
            actief()
        )),
        Some(row) if inactief(row) => Err(format!(
            "Factuursjabloon {} staat op inactief. Kies een actief sjabloon: {}",
            naam(row),
            actief()
        )),
        Some(row) => Ok((id, naam(row))),
    }
}

/// What a new invoice takes from the administration's settings.
#[derive(Clone, Debug)]
pub struct FactuurContext {
    /// `factuur/nextfactuurnummer`.
    pub volgend_nummer: String,
    /// `factuursetting.factuurSjabloonId`.
    pub sjabloon_id: u64,
    /// `factuursetting.verwerken`: book a new invoice at once.
    pub verwerken: bool,
    pub vandaag: Date,
}

/// `sjablonen` is the administration's `factuursjabloon/gridtable`, read only when the tool names a template.
pub fn factuur_nieuw(
    args: &FactuurToevoegenArgs,
    context: &FactuurContext,
    books: &Books,
    sjablonen: Option<&Value>,
) -> Result<Built, String> {
    let in_ex = args.bedragen.unwrap_or(InEx::Excl);
    let datum = args
        .datum
        .as_deref()
        .map(parse_date)
        .transpose()?
        .unwrap_or(context.vandaag);
    let nummer = text(&args.nummer).unwrap_or_else(|| context.volgend_nummer.clone());
    if nummer.is_empty() {
        return Err("Geef een factuurnummer".into());
    }
    let (items, item_lines) = factuur_items(&args.regels, in_ex, books)?;
    let sjabloon = args
        .sjabloon_id
        .map(|id| factuur_sjabloon(id, sjablonen))
        .transpose()?;
    let boeken = args.direct_boeken.unwrap_or(context.verwerken);
    let mut factuur = Map::new();
    factuur.insert("relatieId".into(), json!(args.relatie_id));
    factuur.insert("nummer".into(), json!(nummer));
    factuur.insert("inEx".into(), json!(in_ex.code()));
    factuur.insert("datum".into(), date_object(datum));
    factuur.insert("termijn".into(), json!(args.termijn.unwrap_or(0)));
    factuur.insert(
        "sjabloonId".into(),
        json!(sjabloon.as_ref().map_or(context.sjabloon_id, |(id, _)| *id)),
    );
    factuur.insert("emailSjabloonId".into(), Value::Null);
    factuur.insert("type".into(), json!("N"));
    factuur.insert("btwLaag".into(), json!(0));
    factuur.insert("btwHoog".into(), json!(0));
    factuur.insert("totaal".into(), json!(0));
    if let Some(tekst) = text(&args.factuurtekst) {
        factuur.insert("factuurtekst".into(), json!(tekst));
    }
    if let Some(kenmerk) = text(&args.kenmerk) {
        factuur.insert("kenmerk".into(), json!(kenmerk));
    }
    let mut body = Map::new();
    body.insert("factuur".into(), Value::Object(factuur));
    body.insert(
        "factuurItems".into(),
        json!({"insert": items, "update": [], "delete": []}),
    );
    if boeken {
        body.insert(
            "verwerk".into(),
            json!({"grootboekrekeningId": books.debiteuren()?.id}),
        );
    }
    let mut lines = vec![
        line("Relatie", format!("id {}", args.relatie_id)),
        line("Factuurnummer", &nummer),
        line("Datum", dutch(datum)),
        line(
            "Betalingstermijn",
            format!("{} dagen", args.termijn.unwrap_or(0)),
        ),
    ];
    if let Some((_, naam)) = &sjabloon {
        lines.push(line("Factuursjabloon", naam));
    }
    lines.extend(item_lines);
    lines.push(line(
        "Boeken",
        if boeken {
            "meteen als mutatie Factuur verstuurd"
        } else {
            "nog niet"
        },
    ));
    Ok(Built {
        body: Value::Object(body),
        title: format!("Factuur {nummer} toevoegen"),
        lines,
    })
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FactuurWijzigenArgs {
    /// Id van de factuur (uit facturen).
    pub id: u64,
    /// Nieuwe factuurdatum, JJJJ-MM-DD.
    pub datum: Option<String>,
    /// Nieuw factuurnummer.
    pub nummer: Option<String>,
    /// Nieuwe betalingstermijn in dagen.
    pub termijn: Option<u32>,
    /// Nieuwe tekst onderaan de factuur.
    pub factuurtekst: Option<String>,
    /// Nieuw kenmerk.
    pub kenmerk: Option<String>,
    /// Prijzen incl of excl btw. Alleen samen met `regels`.
    pub bedragen: Option<InEx>,
    /// Vervangt alle regels van de factuur. Laat weg om ze te houden.
    pub regels: Option<Vec<FactuurRegelArg>>,
    /// Ander factuursjabloon voor de opmaak (id uit sjablonen met soort factuur).
    pub sjabloon_id: Option<u64>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// The fields of an invoice or subscription that an edit sends back, taken from how it is now.
fn factuur_kept(now: &Value, kind: &str) -> Result<Map<String, Value>, String> {
    let bad = || format!("e-Boekhouden gaf {kind} in een onbekende vorm terug");
    let mut kept = Map::new();
    kept.insert(
        "id".into(),
        now.get("id")
            .filter(|v| v.is_u64())
            .cloned()
            .ok_or_else(bad)?,
    );
    kept.insert(
        "relatieId".into(),
        now.get("relatieId")
            .filter(|v| v.is_u64())
            .cloned()
            .ok_or_else(bad)?,
    );
    kept.insert(
        "nummer".into(),
        now.get("nummer")
            .filter(|v| v.is_string())
            .cloned()
            .ok_or_else(bad)?,
    );
    kept.insert(
        "inEx".into(),
        now.get("inEx")
            .filter(|v| v.is_string())
            .cloned()
            .ok_or_else(bad)?,
    );
    kept.insert(
        "termijn".into(),
        now.get("termijn").cloned().unwrap_or(json!(0)),
    );
    kept.insert(
        "sjabloonId".into(),
        now.get("sjabloonId").cloned().ok_or_else(bad)?,
    );
    for name in [
        "factuurtekst",
        "kenmerk",
        "printen",
        "emailen",
        "incasso",
        "versturenViaPeppol",
        "geenFactuur",
        "autoVerwerk",
        "factOm1",
        "factOm2",
        "factOm3",
    ] {
        if let Some(value) = now.get(name) {
            kept.insert(name.into(), value.clone());
        }
    }
    kept.insert("btwLaag".into(), json!(0));
    kept.insert("btwHoog".into(), json!(0));
    kept.insert("totaal".into(), json!(0));
    Ok(kept)
}

/// `sjablonen` is the administration's `factuursjabloon/gridtable`, read only when the tool names a template.
pub fn factuur_gewijzigd(
    args: &FactuurWijzigenArgs,
    now: &Value,
    items: &Value,
    books: &Books,
    sjablonen: Option<&Value>,
) -> Result<Built, String> {
    let mut factuur = factuur_kept(now, "deze factuur")?;
    let old_nummer = now
        .get("nummer")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let mut lines = vec![line("Factuur", &old_nummer)];
    let mut changed = false;
    factuur.insert("type".into(), json!("N"));
    factuur.insert("emailSjabloonId".into(), Value::Null);
    let datum = match &args.datum {
        Some(datum) => {
            let datum = parse_date(datum)?;
            lines.push(line("Datum wordt", dutch(datum)));
            changed = true;
            datum
        }
        None => now
            .get("datum")
            .and_then(date_of)
            .ok_or("e-Boekhouden gaf deze factuur zonder datum terug")?,
    };
    factuur.insert("datum".into(), date_object(datum));
    let mut body = Map::new();
    if let Some(nummer) = text(&args.nummer).filter(|n| *n != old_nummer) {
        lines.push(line("Factuurnummer wordt", &nummer));
        // The web form sends this along so that the invoice's mutatie and open post follow the new number.
        body.insert(
            "updateRelated".into(),
            json!({"oldFactuurNummer": old_nummer, "factuurNummer": nummer, "oldRelatieId": factuur["relatieId"], "relatieId": factuur["relatieId"]}),
        );
        factuur.insert("nummer".into(), json!(nummer));
        changed = true;
    }
    if let Some(termijn) = args.termijn {
        lines.push(line("Betalingstermijn wordt", format!("{termijn} dagen")));
        factuur.insert("termijn".into(), json!(termijn));
        changed = true;
    }
    if let Some(tekst) = &args.factuurtekst {
        lines.push(line("Factuurtekst wordt", tekst.trim()));
        factuur.insert("factuurtekst".into(), json!(tekst.trim()));
        changed = true;
    }
    if let Some(kenmerk) = &args.kenmerk {
        lines.push(line("Kenmerk wordt", kenmerk.trim()));
        factuur.insert("kenmerk".into(), json!(kenmerk.trim()));
        changed = true;
    }
    if let Some(id) = args.sjabloon_id {
        let (id, naam) = factuur_sjabloon(id, sjablonen)?;
        let old = now.get("sjabloonId").and_then(Value::as_u64);
        if old != Some(id) {
            let was = old
                .and_then(|old| factuur_sjabloon(old, sjablonen).ok())
                .map_or_else(|| "onbekend".to_owned(), |(_, naam)| naam);
            lines.push(line("Factuursjabloon wordt", format!("{naam} (was {was})")));
            factuur.insert("sjabloonId".into(), json!(id));
            changed = true;
        }
    }
    let mut item_changes = json!({"insert": [], "update": [], "delete": []});
    match &args.regels {
        Some(regels) => {
            let in_ex = args
                .bedragen
                .unwrap_or_else(|| InEx::from_code(factuur["inEx"].as_str().unwrap_or("EX")));
            factuur.insert("inEx".into(), json!(in_ex.code()));
            let (new_items, item_lines) = factuur_items(regels, in_ex, books)?;
            lines.push(line("Regels", "alle regels worden vervangen door:"));
            lines.extend(item_lines);
            item_changes = json!({"insert": new_items, "update": [], "delete": item_ids(items)});
            changed = true;
        }
        None if args.bedragen.is_some() => {
            return Err("Incl of excl btw wijzigen kan alleen samen met nieuwe regels".into());
        }
        None => {}
    }
    if !changed {
        return Err("Er is niets gewijzigd: geef aan wat er anders moet".into());
    }
    body.insert("factuur".into(), Value::Object(factuur));
    body.insert("factuurItems".into(), item_changes);
    Ok(Built {
        body: Value::Object(body),
        title: format!("Factuur {old_nummer} wijzigen"),
        lines,
    })
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FactuurCreditArgs {
    /// Id van de factuur die gecrediteerd wordt (uit facturen).
    pub id: u64,
    /// Datum van de creditfactuur, JJJJ-MM-DD (standaard vandaag).
    pub datum: Option<String>,
    /// Nummer van de creditfactuur (standaard het volgende nummer dat e-Boekhouden voorstelt).
    pub nummer: Option<String>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// A credit invoice as e-Boekhouden's own "Factuur crediteren" makes it: a new invoice with every line of the
/// original, its aantal negated and "- Creditering -" behind its omschrijving.
pub fn factuur_credit(
    args: &FactuurCreditArgs,
    now: &Value,
    items: &Value,
    context: &FactuurContext,
    books: &Books,
) -> Result<Built, String> {
    let kept = factuur_kept(now, "deze factuur")?;
    let old_nummer = now
        .get("nummer")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let rows = items
        .as_array()
        .filter(|rows| !rows.is_empty())
        .ok_or("Deze factuur heeft geen regels om te crediteren")?;
    let datum = args
        .datum
        .as_deref()
        .map(parse_date)
        .transpose()?
        .unwrap_or(context.vandaag);
    let nummer = text(&args.nummer).unwrap_or_else(|| context.volgend_nummer.clone());
    let mut credit_items = Vec::new();
    let mut lines = vec![
        line("Crediteert", format!("factuur {old_nummer}")),
        line("Nummer creditfactuur", &nummer),
        line("Datum", dutch(datum)),
    ];
    let mut totaal = 0.0;
    for (i, row) in rows.iter().enumerate() {
        let bad = || "e-Boekhouden gaf een factuurregel in een onbekende vorm terug".to_owned();
        let aantal = -row.get("aantal").and_then(Value::as_f64).ok_or_else(bad)?;
        let omschrijving = format!(
            "{}- Creditering -",
            row.get("omschrijving")
                .and_then(Value::as_str)
                .unwrap_or_default()
        );
        let incl = row
            .get("prijsIncl")
            .and_then(Value::as_f64)
            .ok_or_else(bad)?;
        let mut item = Map::new();
        item.insert("id".into(), json!(-((i + 1) as i64)));
        item.insert("factuurId".into(), json!(0));
        item.insert("aantal".into(), json!(aantal));
        item.insert("omschrijving".into(), json!(omschrijving));
        item.insert("prijs".into(), Value::Null);
        item.insert("volgorde".into(), json!(i));
        for name in [
            "btw",
            "grootboekId",
            "prijsExcl",
            "prijsIncl",
            "kortingIsPerc",
            "code",
            "eenheidId",
            "artikelId",
            "kostenplaatsId",
            "korting",
        ] {
            if let Some(value) = row.get(name).filter(|v| !v.is_null()) {
                item.insert(name.into(), value.clone());
            }
        }
        totaal += aantal * incl;
        lines.push(line(
            &format!("Regel {}", i + 1),
            format!(
                "{aantal} x {omschrijving}: {} per stuk incl. btw",
                euro(incl)
            ),
        ));
        credit_items.push(Value::Object(item));
    }
    lines.push(line("Totaal incl. btw", euro(totaal)));
    let mut factuur = Map::new();
    factuur.insert("relatieId".into(), kept["relatieId"].clone());
    factuur.insert("nummer".into(), json!(nummer));
    factuur.insert("inEx".into(), kept["inEx"].clone());
    factuur.insert("datum".into(), date_object(datum));
    factuur.insert("termijn".into(), kept["termijn"].clone());
    factuur.insert("sjabloonId".into(), kept["sjabloonId"].clone());
    factuur.insert("emailSjabloonId".into(), Value::Null);
    factuur.insert("type".into(), json!("N"));
    factuur.insert("btwLaag".into(), json!(0));
    factuur.insert("btwHoog".into(), json!(0));
    factuur.insert("totaal".into(), json!(0));
    let mut body = Map::new();
    body.insert("factuur".into(), Value::Object(factuur));
    body.insert(
        "factuurItems".into(),
        json!({"insert": credit_items, "update": [], "delete": []}),
    );
    if context.verwerken {
        body.insert(
            "verwerk".into(),
            json!({"grootboekrekeningId": books.debiteuren()?.id}),
        );
        lines.push(line("Boeken", "meteen als mutatie Factuur verstuurd"));
    }
    Ok(Built {
        body: Value::Object(body),
        title: format!("Factuur {old_nummer} crediteren"),
        lines,
    })
}

// ---------------------------------------------------------------- offertes

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct OfferteToevoegenArgs {
    /// Id van de relatie (de klant).
    pub relatie_id: u64,
    /// Offertedatum, JJJJ-MM-DD (standaard vandaag).
    pub datum: Option<String>,
    /// Vervaldatum, JJJJ-MM-DD (standaard een maand na de offertedatum).
    pub vervaldatum: Option<String>,
    /// Offertenummer (standaard het volgende nummer dat e-Boekhouden voorstelt).
    pub nummer: Option<String>,
    /// Kenmerk van de klant.
    pub kenmerk: Option<String>,
    /// Notitie voor de klant, op de offerte.
    pub notitie: Option<String>,
    /// Interne notitie.
    pub interne_notitie: Option<String>,
    /// Zijn de prijzen incl of excl btw (standaard excl).
    pub bedragen: Option<InEx>,
    /// De producten of diensten op de offerte.
    pub regels: Vec<FactuurRegelArg>,
    /// Id van het offertesjabloon (standaard het eerste actieve).
    pub sjabloon_id: Option<u64>,
    /// Id van de offertestatus (standaard de eerste, "Ingevoerd").
    pub status_id: Option<u64>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct OfferteWijzigenArgs {
    /// Id van de offerte (uit offertes).
    pub id: u64,
    /// Nieuwe offertedatum, JJJJ-MM-DD.
    pub datum: Option<String>,
    /// Nieuwe vervaldatum, JJJJ-MM-DD.
    pub vervaldatum: Option<String>,
    /// Nieuw offertenummer.
    pub nummer: Option<String>,
    /// Nieuw kenmerk.
    pub kenmerk: Option<String>,
    /// Nieuwe notitie voor de klant.
    pub notitie: Option<String>,
    /// Nieuwe interne notitie.
    pub interne_notitie: Option<String>,
    /// Nieuwe status (id uit de lijst met offertestatussen).
    pub status_id: Option<u64>,
    /// Prijzen incl of excl btw. Alleen samen met `regels`.
    pub bedragen: Option<InEx>,
    /// Vervangt alle regels van de offerte. Laat weg om ze te houden.
    pub regels: Option<Vec<FactuurRegelArg>>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// What a new quote takes from the administration.
#[derive(Clone, Debug)]
pub struct OfferteContext {
    /// `offerte/nummer/next`.
    pub volgend_nummer: String,
    /// The first active `offertesjabloon`.
    pub sjabloon_id: u64,
    /// The first `offertestatus`.
    pub status_id: u64,
    /// The signed-in user's id (`user.id`), who the quote is "in behandeling bij".
    pub gebruiker_id: u64,
    pub vandaag: Date,
}

/// The same day a month later, as e-Boekhouden's form proposes for the vervaldatum (kept within every month).
fn month_later((y, m, d): Date) -> Date {
    let (y, m) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    // The last day of the target month that still parses is the longest it has.
    let day = (28..=d.max(28))
        .rev()
        .find(|day| parse_date(&dashed((y, m, *day))).is_ok())
        .unwrap_or(28);
    (y, m, day.min(d.max(1)))
}

/// The lines of a quote, each as `POST offerteregel` takes it (without the quote's id, which the caller adds).
pub fn offerte_regels(
    regels: &[FactuurRegelArg],
    in_ex: InEx,
    books: &Books,
) -> Result<(Vec<Value>, Vec<Line>), String> {
    let (items, lines) = factuur_items(regels, in_ex, books)?;
    let regels = items
        .into_iter()
        .map(|item| {
            let mut regel = Map::new();
            regel.insert("id".into(), json!(0));
            regel.insert("volgorde".into(), Value::Null);
            for (from, to) in [
                ("aantal", "aantal"),
                ("omschrijving", "omschrijving"),
                ("prijsExcl", "prijsExcl"),
                ("prijsIncl", "prijsIncl"),
                ("btw", "btwCode"),
                ("grootboekId", "grootboekId"),
                ("kortingIsPerc", "kortingIsPerc"),
                ("code", "code"),
            ] {
                if let Some(value) = item.get(from) {
                    regel.insert(to.into(), value.clone());
                }
            }
            Value::Object(regel)
        })
        .collect();
    Ok((regels, lines))
}

/// A new quote: the body for `POST offerte`, its lines for `POST offerteregel`, and the summary.
pub fn offerte_nieuw(
    args: &OfferteToevoegenArgs,
    context: &OfferteContext,
    books: &Books,
) -> Result<(Built, Vec<Value>), String> {
    let in_ex = args.bedragen.unwrap_or(InEx::Excl);
    let datum = args
        .datum
        .as_deref()
        .map(parse_date)
        .transpose()?
        .unwrap_or(context.vandaag);
    let vervalt = args
        .vervaldatum
        .as_deref()
        .map(parse_date)
        .transpose()?
        .unwrap_or_else(|| month_later(datum));
    if vervalt < datum {
        return Err("De vervaldatum ligt voor de offertedatum".into());
    }
    let nummer = text(&args.nummer).unwrap_or_else(|| context.volgend_nummer.clone());
    if nummer.is_empty() {
        return Err("Geef een offertenummer".into());
    }
    let (regels, regel_lines) = offerte_regels(&args.regels, in_ex, books)?;
    let mut offerte = Map::new();
    offerte.insert(
        "statusId".into(),
        json!(args.status_id.unwrap_or(context.status_id)),
    );
    offerte.insert("nummer".into(), json!(nummer));
    offerte.insert("behandelGebruikerId".into(), json!(context.gebruiker_id));
    offerte.insert("relatieId".into(), json!(args.relatie_id));
    offerte.insert(
        "kenmerk".into(),
        json!(text(&args.kenmerk).unwrap_or_default()),
    );
    offerte.insert("datum".into(), date_object(datum));
    offerte.insert("vervaldatum".into(), date_object(vervalt));
    offerte.insert(
        "sjabloonId".into(),
        json!(args.sjabloon_id.unwrap_or(context.sjabloon_id)),
    );
    offerte.insert("factuurEmailsjabloonId".into(), json!(""));
    offerte.insert(
        "notitiePubliek".into(),
        json!(text(&args.notitie).unwrap_or_default()),
    );
    if let Some(intern) = text(&args.interne_notitie) {
        offerte.insert("notitieIntern".into(), json!(intern));
    }
    offerte.insert("inEx".into(), json!(in_ex.code()));
    let mut lines = vec![
        line("Relatie", format!("id {}", args.relatie_id)),
        line("Offertenummer", &nummer),
        line("Datum", dutch(datum)),
        line("Vervaldatum", dutch(vervalt)),
    ];
    if let Some(kenmerk) = text(&args.kenmerk) {
        lines.push(line("Kenmerk", kenmerk));
    }
    if let Some(notitie) = text(&args.notitie) {
        lines.push(line("Notitie voor de klant", notitie));
    }
    lines.extend(regel_lines);
    Ok((
        Built {
            body: Value::Object(offerte),
            title: format!("Offerte {nummer} toevoegen"),
            lines,
        },
        regels,
    ))
}

/// An edited quote: the body for `PUT offerte` (the quote as it is, with the named changes) and, when the lines
/// are replaced, the new ones.
pub fn offerte_gewijzigd(
    args: &OfferteWijzigenArgs,
    now: &Value,
    books: &Books,
) -> Result<(Built, Option<Vec<Value>>), String> {
    let mut offerte = now
        .as_object()
        .cloned()
        .ok_or("e-Boekhouden gaf deze offerte in een onbekende vorm terug")?;
    let nummer = offerte
        .get("nummer")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let mut lines = vec![line("Offerte", &nummer)];
    let date_at = |offerte: &Map<String, Value>, key: &str| offerte.get(key).and_then(date_of);
    if let Some(datum) = &args.datum {
        let datum = parse_date(datum)?;
        offerte.insert("datum".into(), date_object(datum));
        lines.push(line("Datum wordt", dutch(datum)));
    }
    if let Some(datum) = &args.vervaldatum {
        let datum = parse_date(datum)?;
        offerte.insert("vervaldatum".into(), date_object(datum));
        lines.push(line("Vervaldatum wordt", dutch(datum)));
    }
    if let (Some(datum), Some(vervalt)) =
        (date_at(&offerte, "datum"), date_at(&offerte, "vervaldatum"))
        && vervalt < datum
    {
        return Err("De vervaldatum ligt voor de offertedatum".into());
    }
    for (value, key, label) in [
        (&args.nummer, "nummer", "Offertenummer"),
        (&args.kenmerk, "kenmerk", "Kenmerk"),
        (&args.notitie, "notitiePubliek", "Notitie voor de klant"),
        (&args.interne_notitie, "notitieIntern", "Interne notitie"),
    ] {
        if let Some(value) = value {
            let value = value.trim();
            if key == "nummer" && value.is_empty() {
                return Err("Het offertenummer mag niet leeg zijn".into());
            }
            offerte.insert(key.into(), json!(value));
            lines.push(line(&format!("{label} wordt"), value));
        }
    }
    if let Some(status) = args.status_id {
        offerte.insert("statusId".into(), json!(status));
        lines.push(line("Status wordt", format!("id {status}")));
    }
    let regels = match &args.regels {
        Some(regels) => {
            let in_ex = args.bedragen.unwrap_or_else(|| {
                InEx::from_code(offerte.get("inEx").and_then(Value::as_str).unwrap_or("EX"))
            });
            offerte.insert("inEx".into(), json!(in_ex.code()));
            let (regels, regel_lines) = offerte_regels(regels, in_ex, books)?;
            lines.push(line("Regels", "alle regels worden vervangen door:"));
            lines.extend(regel_lines);
            Some(regels)
        }
        None if args.bedragen.is_some() => {
            return Err("Incl of excl btw wijzigen kan alleen samen met nieuwe regels".into());
        }
        None => None,
    };
    if lines.len() == 1 {
        return Err("Er is niets gewijzigd: geef aan wat er anders moet".into());
    }
    Ok((
        Built {
            body: Value::Object(offerte),
            title: format!("Offerte {nummer} wijzigen"),
            lines,
        },
        regels,
    ))
}

// ---------------------------------------------------------------- sjablonen

/// What changes between a template's layout as it is and as a tool wants it: one line per element that is new,
/// gone or different, in words. The layout is e-Boekhouden's editor document (`sjabloon.elements`).
pub fn opmaak_changes(now: &Value, new: &Value) -> Result<Vec<Line>, String> {
    let elements = |doc: &Value| -> Result<Vec<Value>, String> {
        doc.get("elements")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| "De opmaak heeft geen elements: geef het hele document zoals sjabloon het teruggaf, met je wijzigingen".to_owned())
    };
    let (old, new_elements) = (elements(now)?, elements(new)?);
    if new_elements.is_empty() {
        return Err(
            "De opmaak heeft geen enkel element; een leeg sjabloon slaat EB MCP niet op".into(),
        );
    }
    let id = |e: &Value| e.get("id").and_then(Value::as_i64);
    let name = |e: &Value| {
        let tag = e.get("tag").and_then(Value::as_str).unwrap_or("element");
        let words: String = e
            .get("content")
            .and_then(Value::as_str)
            .map(strip_tags)
            .unwrap_or_default()
            .chars()
            .take(60)
            .collect();
        if words.is_empty() {
            format!("{tag} {}", id(e).unwrap_or(0))
        } else {
            format!("{tag} {} \"{words}\"", id(e).unwrap_or(0))
        }
    };
    let mut seen = std::collections::HashSet::new();
    let mut lines = Vec::new();
    for element in &new_elements {
        let Some(element_id) = id(element) else {
            return Err("Elk element heeft een id nodig".into());
        };
        if !seen.insert(element_id) {
            return Err(format!("Element-id {element_id} komt twee keer voor"));
        }
        for key in ["x", "y", "width", "height"] {
            if !element.get(key).is_some_and(Value::is_number) {
                return Err(format!("Element {element_id} mist {key}"));
            }
        }
        match old.iter().find(|e| id(e) == Some(element_id)) {
            None => lines.push(line("Nieuw", name(element))),
            Some(before) if before != element => {
                let words = |e: &Value| {
                    e.get("content")
                        .and_then(Value::as_str)
                        .map(strip_tags)
                        .unwrap_or_default()
                };
                let label = format!(
                    "{} {element_id}",
                    element
                        .get("tag")
                        .and_then(Value::as_str)
                        .unwrap_or("element")
                );
                if before.get("content") != element.get("content") {
                    // Both texts in full: the user must be able to see a changed IBAN or amount, not just "tekst".
                    lines.push(line(&format!("Tekst van {label} was"), cut(&words(before))));
                    lines.push(line(
                        &format!("Tekst van {label} wordt"),
                        cut(&words(element)),
                    ));
                    if words(before) == words(element) {
                        lines.push(line(&format!("Opmaak van {label}"), "de tekst blijft gelijk, de opmaak ervan (lettertype, kleur, links) verandert"));
                    }
                }
                let moved: Vec<String> = ["x", "y", "width", "height", "page", "section"]
                    .iter()
                    .filter(|k| before.get(**k) != element.get(**k))
                    .map(|k| {
                        format!(
                            "{k} {} wordt {}",
                            before.get(*k).unwrap_or(&Value::Null),
                            element.get(*k).unwrap_or(&Value::Null)
                        )
                    })
                    .collect();
                if !moved.is_empty() {
                    lines.push(line(&format!("Plaats van {label}"), moved.join(", ")));
                }
                let other: Vec<&str> = match (before.as_object(), element.as_object()) {
                    (Some(b), Some(e)) => {
                        let mut keys: Vec<&str> =
                            b.keys().chain(e.keys()).map(String::as_str).collect();
                        keys.sort_unstable();
                        keys.dedup();
                        keys.into_iter()
                            .filter(|k| {
                                !["content", "x", "y", "width", "height", "page", "section"]
                                    .contains(k)
                            })
                            .filter(|k| b.get(*k) != e.get(*k))
                            .collect()
                    }
                    _ => vec!["alles"],
                };
                if !other.is_empty() {
                    lines.push(line(
                        &format!("Instellingen van {label}"),
                        format!("gewijzigd: {}", other.join(", ")),
                    ));
                }
            }
            Some(_) => {}
        }
    }
    for element in &old {
        if !id(element).is_some_and(|i| seen.contains(&i)) {
            lines.push(line("Verwijderd", name(element)));
        }
    }
    for key in ["documentDefinition", "sjabloon"] {
        if new.get(key).is_some() && now.get(key) != new.get(key) {
            lines.push(line(
                "Gewijzigd",
                if key == "sjabloon" {
                    "algemene instellingen (lettertype, kop- en voettekst, pagina's)"
                } else {
                    "paginaformaat of marges"
                },
            ));
        }
    }
    Ok(lines)
}

/// A text for the approval window, whole unless it is very long.
fn cut(text: &str) -> String {
    const MAX: usize = 600;
    if text.chars().count() <= MAX {
        return if text.is_empty() {
            "(leeg)".to_owned()
        } else {
            text.to_owned()
        };
    }
    format!(
        "{} … (ingekort, {} tekens)",
        text.chars().take(MAX).collect::<String>(),
        text.chars().count()
    )
}

/// The words of an HTML fragment on one line.
pub fn strip_tags(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => {
                in_tag = true;
                out.push(' ');
            }
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// How often a subscription is invoiced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Per {
    Week,
    TweeWeken,
    VierWeken,
    Maand,
    TweeMaanden,
    Kwartaal,
    Halfjaar,
    Jaar,
    TweeJaar,
}

impl Per {
    fn code(self) -> &'static str {
        match self {
            Self::Week => "W",
            Self::TweeWeken => "2W",
            Self::VierWeken => "4W",
            Self::Maand => "M",
            Self::TweeMaanden => "2M",
            Self::Kwartaal => "K",
            Self::Halfjaar => "H",
            Self::Jaar => "J",
            Self::TweeJaar => "2J",
        }
    }

    fn naam(self) -> &'static str {
        match self {
            Self::Week => "per week",
            Self::TweeWeken => "per 2 weken",
            Self::VierWeken => "per 4 weken",
            Self::Maand => "per maand",
            Self::TweeMaanden => "per 2 maanden",
            Self::Kwartaal => "per kwartaal",
            Self::Halfjaar => "per halfjaar",
            Self::Jaar => "per jaar",
            Self::TweeJaar => "per 2 jaar",
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AbonnementToevoegenArgs {
    /// Id van de relatie (de klant).
    pub relatie_id: u64,
    /// Naam van het abonnement.
    pub naam: String,
    /// Hoe vaak er gefactureerd wordt: week, twee_weken, vier_weken, maand, twee_maanden, kwartaal, halfjaar, jaar of twee_jaar.
    pub per: Per,
    /// Betalingstermijn in dagen.
    pub termijn: u32,
    /// Startdatum, JJJJ-MM-DD (standaard vandaag).
    pub start: Option<String>,
    /// Prolongatiedatum: de datum van de eerstvolgende factuur, JJJJ-MM-DD (standaard de startdatum).
    pub prolongatie: Option<String>,
    /// Zijn de prijzen incl of excl btw (standaard excl).
    pub bedragen: Option<InEx>,
    /// De producten of diensten van het abonnement.
    pub regels: Vec<FactuurRegelArg>,
    /// Kenmerk van de klant.
    pub kenmerk: Option<String>,
    /// Tijdelijk niet factureren.
    pub niet_factureren: Option<bool>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

pub fn abonnement_nieuw(
    args: &AbonnementToevoegenArgs,
    sjabloon_id: u64,
    vandaag: Date,
    books: &Books,
) -> Result<Built, String> {
    let naam = args.naam.trim();
    if naam.is_empty() {
        return Err("Geef het abonnement een naam".into());
    }
    if naam.chars().count() > 50 {
        return Err("De naam van het abonnement is te lang (hoogstens 50 tekens)".into());
    }
    let in_ex = args.bedragen.unwrap_or(InEx::Excl);
    let start = args
        .start
        .as_deref()
        .map(parse_date)
        .transpose()?
        .unwrap_or(vandaag);
    let prolongatie = args
        .prolongatie
        .as_deref()
        .map(parse_date)
        .transpose()?
        .unwrap_or(start);
    if prolongatie < start {
        return Err("De prolongatiedatum ligt voor de startdatum".into());
    }
    let (items, item_lines) = factuur_items(&args.regels, in_ex, books)?;
    let mut factuur = Map::new();
    factuur.insert("relatieId".into(), json!(args.relatie_id));
    factuur.insert("nummer".into(), json!(naam));
    factuur.insert("sjabloonId".into(), json!(sjabloon_id));
    factuur.insert("versturenViaPeppol".into(), json!(false));
    factuur.insert("inEx".into(), json!(in_ex.code()));
    factuur.insert("type".into(), json!("E"));
    factuur.insert("termijn".into(), json!(args.termijn));
    factuur.insert("btwLaag".into(), json!(0));
    factuur.insert("btwHoog".into(), json!(0));
    factuur.insert("totaal".into(), json!(0));
    // The texts e-Boekhouden's form puts on a subscription's invoices; it fills in the number and the period.
    factuur.insert("factOm1".into(), json!("Factuur: [FACTUUR]"));
    factuur.insert("factOm2".into(), json!("Periode: [START] t/m [EIND]"));
    factuur.insert("autoVerwerkMethod".into(), Value::Null);
    if let Some(kenmerk) = text(&args.kenmerk) {
        factuur.insert("kenmerk".into(), json!(kenmerk));
    }
    if args.niet_factureren == Some(true) {
        factuur.insert("geenFactuur".into(), json!(true));
    }
    let body = json!({
        "factuur": factuur,
        "factuurItems": {"insert": items, "update": [], "delete": []},
        "factuurExtra": {"startDatum": date_object(start), "expiratieDatum": date_object(prolongatie), "per": args.per.code()},
    });
    let mut lines = vec![
        line("Relatie", format!("id {}", args.relatie_id)),
        line("Naam", naam),
        line("Factureren", args.per.naam()),
        line("Startdatum", dutch(start)),
        line("Prolongatiedatum", dutch(prolongatie)),
        line("Betalingstermijn", format!("{} dagen", args.termijn)),
    ];
    lines.extend(item_lines);
    if args.niet_factureren == Some(true) {
        lines.push(line("Factureren", "tijdelijk niet"));
    }
    Ok(Built {
        body,
        title: format!("Abonnement {naam} toevoegen"),
        lines,
    })
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AbonnementWijzigenArgs {
    /// Id van het abonnement (uit abonnementen).
    pub id: u64,
    /// Nieuwe naam.
    pub naam: Option<String>,
    /// Nieuwe frequentie.
    pub per: Option<Per>,
    /// Nieuwe betalingstermijn in dagen.
    pub termijn: Option<u32>,
    /// Nieuwe startdatum, JJJJ-MM-DD.
    pub start: Option<String>,
    /// Nieuwe prolongatiedatum, JJJJ-MM-DD.
    pub prolongatie: Option<String>,
    /// Nieuw kenmerk.
    pub kenmerk: Option<String>,
    /// Tijdelijk niet factureren aan of uit.
    pub niet_factureren: Option<bool>,
    /// Prijzen incl of excl btw. Alleen samen met `regels`.
    pub bedragen: Option<InEx>,
    /// Vervangt alle regels van het abonnement. Laat weg om ze te houden.
    pub regels: Option<Vec<FactuurRegelArg>>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

pub fn abonnement_gewijzigd(
    args: &AbonnementWijzigenArgs,
    now: &Value,
    extra: &Value,
    items: &Value,
    books: &Books,
) -> Result<Built, String> {
    let mut factuur = factuur_kept(now, "dit abonnement")?;
    factuur.insert("type".into(), json!("E"));
    let old_naam = now
        .get("nummer")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let bad = || "e-Boekhouden gaf dit abonnement in een onbekende vorm terug".to_owned();
    let mut start = extra.get("startDatum").and_then(date_of).ok_or_else(bad)?;
    let mut prolongatie = extra
        .get("expiratieDatum")
        .and_then(date_of)
        .ok_or_else(bad)?;
    let mut per = extra
        .get("per")
        .and_then(Value::as_str)
        .ok_or_else(bad)?
        .to_owned();
    let mut lines = vec![line("Abonnement", &old_naam)];
    let mut changed = false;
    if let Some(naam) = text(&args.naam).filter(|n| *n != old_naam) {
        if naam.chars().count() > 50 {
            return Err("De naam van het abonnement is te lang (hoogstens 50 tekens)".into());
        }
        lines.push(line("Naam wordt", &naam));
        factuur.insert("nummer".into(), json!(naam));
        changed = true;
    }
    if let Some(new) = args.per {
        lines.push(line("Factureren wordt", new.naam()));
        per = new.code().to_owned();
        changed = true;
    }
    if let Some(termijn) = args.termijn {
        lines.push(line("Betalingstermijn wordt", format!("{termijn} dagen")));
        factuur.insert("termijn".into(), json!(termijn));
        changed = true;
    }
    if let Some(datum) = &args.start {
        start = parse_date(datum)?;
        lines.push(line("Startdatum wordt", dutch(start)));
        changed = true;
    }
    if let Some(datum) = &args.prolongatie {
        prolongatie = parse_date(datum)?;
        lines.push(line("Prolongatiedatum wordt", dutch(prolongatie)));
        changed = true;
    }
    if prolongatie < start {
        return Err("De prolongatiedatum ligt voor de startdatum".into());
    }
    if let Some(kenmerk) = &args.kenmerk {
        lines.push(line("Kenmerk wordt", kenmerk.trim()));
        factuur.insert("kenmerk".into(), json!(kenmerk.trim()));
        changed = true;
    }
    if let Some(stop) = args.niet_factureren {
        lines.push(line(
            "Tijdelijk niet factureren",
            if stop { "aan" } else { "uit" },
        ));
        factuur.insert("geenFactuur".into(), json!(stop));
        changed = true;
    }
    let mut item_changes = json!({"insert": [], "update": [], "delete": []});
    match &args.regels {
        Some(regels) => {
            let in_ex = args
                .bedragen
                .unwrap_or_else(|| InEx::from_code(factuur["inEx"].as_str().unwrap_or("EX")));
            factuur.insert("inEx".into(), json!(in_ex.code()));
            let (new_items, item_lines) = factuur_items(regels, in_ex, books)?;
            lines.push(line("Regels", "alle regels worden vervangen door:"));
            lines.extend(item_lines);
            item_changes = json!({"insert": new_items, "update": [], "delete": item_ids(items)});
            changed = true;
        }
        None if args.bedragen.is_some() => {
            return Err("Incl of excl btw wijzigen kan alleen samen met nieuwe regels".into());
        }
        None => {}
    }
    if !changed {
        return Err("Er is niets gewijzigd: geef aan wat er anders moet".into());
    }
    let mut extra_out = Map::new();
    extra_out.insert("startDatum".into(), date_object(start));
    extra_out.insert("expiratieDatum".into(), date_object(prolongatie));
    extra_out.insert("per".into(), json!(per));
    // An end date the subscription already has goes back as it came.
    if let Some(eind) = extra.get("eindDatum").filter(|v| date_of(v).is_some()) {
        extra_out.insert("eindDatum".into(), eind.clone());
    }
    let body = json!({"factuur": factuur, "factuurItems": item_changes, "factuurExtra": extra_out});
    Ok(Built {
        body,
        title: format!("Abonnement {old_naam} wijzigen"),
        lines,
    })
}

/// Which subscriptions "Facturen maken" takes, by their prolongatiedatum.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Venster {
    /// De prolongatiedatum is al verlopen.
    Verlopen,
    /// Verloopt binnen een week.
    Week,
    /// Verloopt binnen twee weken.
    TweeWeken,
    /// Verloopt binnen een maand.
    Maand,
    /// Verloopt binnen twee maanden.
    TweeMaanden,
}

impl Venster {
    pub fn code(self) -> &'static str {
        match self {
            Self::Verlopen => "Expired",
            Self::Week => "Week",
            Self::TweeWeken => "TwoWeeks",
            Self::Maand => "Month",
            Self::TweeMaanden => "TwoMonths",
        }
    }

    /// The name of this window's count in `factuur/expiredabonnementencount`.
    pub fn count_key(self) -> &'static str {
        match self {
            Self::Verlopen => "expired",
            Self::Week => "week",
            Self::TweeWeken => "twoWeeks",
            Self::Maand => "month",
            Self::TweeMaanden => "twoMonths",
        }
    }

    pub fn naam(self) -> &'static str {
        match self {
            Self::Verlopen => "prolongatiedatum al verlopen",
            Self::Week => "prolongatiedatum binnen 1 week",
            Self::TweeWeken => "prolongatiedatum binnen 2 weken",
            Self::Maand => "prolongatiedatum binnen 1 maand",
            Self::TweeMaanden => "prolongatiedatum binnen 2 maanden",
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AbonnementenFacturerenArgs {
    /// Welke abonnementen: verlopen (standaard), week, twee_weken, maand of twee_maanden, naar hun prolongatiedatum.
    pub venster: Option<Venster>,
    /// Factuurdatum, JJJJ-MM-DD (standaard vandaag).
    pub factuurdatum: Option<String>,
    /// De facturen meteen boeken als mutatie (standaard niet).
    pub direct_boeken: Option<bool>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// The argument of the hub's `create` (ebapi.rs, `hub_invoke`), as the "Facturen maken" form sends it.
pub fn abonnementen_factureren(
    args: &AbonnementenFacturerenArgs,
    aantal: u64,
    vandaag: Date,
    books: &Books,
) -> Result<Built, String> {
    let venster = args.venster.unwrap_or(Venster::Verlopen);
    let datum = args
        .factuurdatum
        .as_deref()
        .map(parse_date)
        .transpose()?
        .unwrap_or(vandaag);
    let boeken = args.direct_boeken.unwrap_or(false);
    if aantal == 0 {
        return Err(format!("Er zijn geen abonnementen met {}", venster.naam()));
    }
    let body = json!({
        "factuurdatum": dashed(datum),
        "directVerwerken": boeken,
        "expirationPeriod": venster.code(),
        "type": "E",
        "grootboekRekeningId": books.debiteuren()?.id,
    });
    let lines = vec![
        line(
            "Abonnementen",
            format!("alle {aantal} met {}", venster.naam()),
        ),
        line("Factuurdatum", dutch(datum)),
        line(
            "Boeken",
            if boeken {
                "meteen als mutatie"
            } else {
                "nog niet"
            },
        ),
        line(
            "Gevolg",
            "per abonnement een factuur, en de prolongatiedatum schuift een periode op",
        ),
    ];
    Ok(Built {
        body,
        title: format!(
            "Facturen maken voor {aantal} abonnement{}",
            if aantal == 1 { "" } else { "en" }
        ),
        lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_cents_as_e_boekhoudens_forms_do() {
        // 3,50 excl. 21% btw: 4,235 is held just under it, and their form still shows 4,24.
        assert_eq!(round_cents(3.50 * 1.21), 4.24);
        assert_eq!(round_cents(4.235), 4.24);
        assert_eq!(round_cents(4.234), 4.23);
        assert_eq!(round_cents(1.005), 1.01);
        assert_eq!(round_cents(2.0), 2.0);
        assert_eq!(round_cents(0.004), 0.0);
        // Away from zero for a negative amount, and never a negative zero.
        assert_eq!(round_cents(-4.235), -4.24);
        assert_eq!(round_cents(-0.005), -0.01);
        assert!(round_cents(-0.001).is_sign_positive());
        assert_eq!(round2(3.50 * 1.21), 4.24);
    }

    /// The rekeningschema and btw codes of the test administration, in the API's own shapes.
    fn books() -> Books {
        let grootboek = json!({
            "colMetadata": [{"name": "id"}, {"name": "code"}, {"name": "omschrijving"}, {"name": "categorie"}, {"name": "rekeningGroep"}, {"name": "inactief"}],
            "data": [
                [26852133, "1010", "Bank", "Betalingsmiddelen", "", false],
                [26852134, "1300", "Debiteuren", "Debiteuren", "", false],
                [26852140, "1700", "Crediteuren", "Crediteuren", "", false],
                [26852148, "2000", "Kruisposten", "Balans", "", false],
                [26852167, "4550", "Bankkosten", "Winst en Verlies", "", false],
                [26852160, "4700", "Kantoorbenodigdheden", "Winst en Verlies", "", false],
                [26852145, "4900", "Betalingsverschillen", "Winst en Verlies", "", false],
                [26852146, "0130", "Inventarissen", "Balans", "", false],
                [26852143, "8000", "Omzet groep 1", "Winst en Verlies", "", false],
                [99, "8999", "Oud", "Winst en Verlies", "", true]
            ]
        });
        let btw = json!([
            {"code": "HOOG_INK_21", "omschrijving": "Btw hoog 21%", "percentage": 21, "soort": "INK"},
            {"code": "LAAG_VERK_9", "omschrijving": "Btw laag 9%", "percentage": 9, "soort": "VERK"},
            {"code": "HOOG_VERK_21", "omschrijving": "Btw hoog 21%", "percentage": 21, "soort": "VERK"},
            {"code": "AFW_VERK", "omschrijving": "Afwijkend", "soort": "VERK"},
            {"code": "GEEN", "omschrijving": "Geen btw", "percentage": 0, "soort": ""}
        ]);
        Books::from_api(&grootboek, &btw).unwrap()
    }

    fn toevoegen(soort: Soort) -> MutatieToevoegenArgs {
        MutatieToevoegenArgs {
            soort,
            datum: "2026-11-06".into(),
            rekening: None,
            omschrijving: None,
            relatie_id: None,
            factuur: None,
            termijn: None,
            bedragen: None,
            regels: None,
            betalingen: None,
            boekstuk: None,
            bankregel_id: None,
            account: None,
        }
    }

    fn regel(tegenrekening: &str, bedrag: f64, btw: Option<&str>) -> RegelArg {
        RegelArg {
            tegenrekening: tegenrekening.into(),
            bedrag,
            btw_code: btw.map(str::to_owned),
            omschrijving: None,
            kostenplaats_id: None,
        }
    }

    /// The test administration's kostenplaatsen, as `GET kostenplaats` gives them.
    fn met_kostenplaatsen() -> Books {
        let mut books = books();
        books.kostenplaatsen = kostenplaatsen_from_api(&json!([
            {"id": 501, "omschrijving": "Projecten", "parentKostenplaatsId": 0, "actief": true, "volledigPad": "Projecten"},
            {"id": 502, "omschrijving": "Noord", "parentKostenplaatsId": 501, "actief": true, "volledigPad": "Projecten>Noord"},
            {"id": 503, "omschrijving": "Oud", "parentKostenplaatsId": 0, "actief": false, "volledigPad": "Oud"}
        ]))
        .unwrap();
        books
    }

    #[test]
    fn a_line_carries_its_kostenplaats_as_the_mutatie_form_sends_it() {
        let mut args = toevoegen(Soort::GeldUitgegeven);
        args.rekening = Some("1010".into());
        let mut line = regel("4550", 2.0, None);
        line.kostenplaats_id = Some(502);
        args.regels = Some(vec![line, regel("4700", 1.0, None)]);
        assert!(names_kostenplaats(args.regels.as_deref()));
        let built = mutatie_nieuw(&args, &met_kostenplaatsen()).unwrap();
        assert_eq!(
            built.body["mutatieRegels"],
            json!([
                {"index": 0, "tegenRekening": 26852167, "btwCode": "GEEN", "bedrag": 2.0, "kostenplaats": 502},
                {"index": 1, "tegenRekening": 26852160, "btwCode": "GEEN", "bedrag": 1.0}
            ])
        );
        let shown: Vec<String> = built
            .lines
            .iter()
            .map(|l| format!("{}: {}", l.label, l.value))
            .collect();
        assert!(
            shown.contains(
                &"Regel 1: 4550 Bankkosten: 2,00 (Geen btw), kostenplaats Projecten>Noord"
                    .to_owned()
            ),
            "{shown:?}"
        );
        assert!(
            shown.contains(&"Regel 2: 4700 Kantoorbenodigdheden: 1,00 (Geen btw)".to_owned()),
            "{shown:?}"
        );
    }

    #[test]
    fn refuses_a_kostenplaats_the_administration_does_not_offer() {
        let mut args = toevoegen(Soort::GeldUitgegeven);
        args.rekening = Some("1010".into());
        let mut line = regel("4550", 2.0, None);
        line.kostenplaats_id = Some(503);
        args.regels = Some(vec![line.clone()]);
        let inactive = mutatie_nieuw(&args, &met_kostenplaatsen()).unwrap_err();
        assert!(inactive.contains("Oud is inactief"), "{inactive}");
        line.kostenplaats_id = Some(999);
        args.regels = Some(vec![line.clone()]);
        let unknown = mutatie_nieuw(&args, &met_kostenplaatsen()).unwrap_err();
        assert!(unknown.contains("999"), "{unknown}");
        let none = mutatie_nieuw(&args, &books()).unwrap_err();
        assert!(none.contains("geen kostenplaatsen"), "{none}");
        assert!(!names_kostenplaats(Some(&[regel("4550", 1.0, None)])));
        assert!(!names_kostenplaats(None));
    }

    #[test]
    fn an_edit_keeps_the_kostenplaats_of_a_line_it_keeps() {
        assert!(!model_has_kostenplaats(&model()));
        let mut model = model();
        model["mutatieRegels"][0]["kostenplaats"] = json!(502);
        assert!(model_has_kostenplaats(&model));
        let mut args = wijzigen(3908);
        args.datum = Some("2026-11-07".into());
        let built = mutatie_gewijzigd(&args, &model, &met_kostenplaatsen()).unwrap();
        assert_eq!(built.body["mutatieRegels"][0]["kostenplaats"], 502);
        assert!(
            built
                .lines
                .iter()
                .any(|l| l.label == "Regel 1" && l.value.ends_with("kostenplaats Projecten>Noord")),
            "{:?}",
            built.lines
        );
        // Without the list (it could not be read) the line still names the kostenplaats, by id.
        let built = mutatie_gewijzigd(&args, &model, &books()).unwrap();
        assert!(
            built
                .lines
                .iter()
                .any(|l| l.label == "Regel 1" && l.value.ends_with("kostenplaats id 502")),
            "{:?}",
            built.lines
        );
        // New lines replace the old ones, each with the kostenplaats it names.
        let mut line = regel("4550", 2.0, None);
        line.kostenplaats_id = Some(501);
        args.regels = Some(vec![line]);
        let built = mutatie_gewijzigd(&args, &model, &met_kostenplaatsen()).unwrap();
        assert_eq!(built.body["mutatieRegels"][0]["kostenplaats"], 501);
        // The old line's kostenplaats is on no new line: the approval says it lapses.
        let lapses = |built: &Built| {
            built
                .lines
                .iter()
                .filter(|l| l.label == "Kostenplaats")
                .map(|l| l.value.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(lapses(&built), ["kostenplaats Projecten>Noord vervalt"]);
        // New lines without any kostenplaats: the same, and the body carries none.
        args.regels = Some(vec![regel("4550", 2.0, None)]);
        let built = mutatie_gewijzigd(&args, &model, &met_kostenplaatsen()).unwrap();
        assert!(built.body["mutatieRegels"][0].get("kostenplaats").is_none());
        assert_eq!(lapses(&built), ["kostenplaats Projecten>Noord vervalt"]);
        // A new line that carries the same kostenplaats keeps it: nothing lapses.
        let mut same = regel("4700", 3.0, None);
        same.kostenplaats_id = Some(502);
        args.regels = Some(vec![same]);
        let built = mutatie_gewijzigd(&args, &model, &met_kostenplaatsen()).unwrap();
        assert!(lapses(&built).is_empty(), "{:?}", built.lines);
    }

    #[test]
    fn a_kostenplaats_id_of_zero_is_no_kostenplaats() {
        let mut args = toevoegen(Soort::GeldUitgegeven);
        args.rekening = Some("1010".into());
        let mut line = regel("4550", 2.0, None);
        line.kostenplaats_id = Some(0);
        args.regels = Some(vec![line]);
        assert!(!names_kostenplaats(args.regels.as_deref()));
        // No kostenplaatsen read (an empty list) and still fine: 0 asks for none.
        let built = mutatie_nieuw(&args, &books()).unwrap();
        assert!(built.body["mutatieRegels"][0].get("kostenplaats").is_none());
    }

    #[test]
    fn reads_the_books_and_leaves_inactive_accounts_out() {
        let books = books();
        assert_eq!(books.rekening(" 1010 ").unwrap().id, 26852133);
        assert!(books.rekening("8999").is_err(), "inactive");
        assert!(books.rekening("1234").unwrap_err().contains("1234"));
        assert_eq!(books.debiteuren().unwrap().code, "1300");
        assert_eq!(books.btw("geen").unwrap().code, "GEEN");
        let unknown = books.btw("HOOG").unwrap_err();
        assert!(unknown.contains("HOOG_VERK_21"), "{unknown}");
    }

    #[test]
    fn books_a_bank_line_as_e_boekhoudens_own_screen_does() {
        // The request the web app sent for "Handmatig invoeren" of a bank line, without the parts it may leave out.
        let mut args = toevoegen(Soort::GeldUitgegeven);
        args.rekening = Some("1010".into());
        args.omschrijving = Some(" Parkeergarage ".into());
        args.regels = Some(vec![regel("4550", 2.0, None)]);
        args.bankregel_id = Some(465233844);
        let built = mutatie_nieuw(&args, &books()).unwrap();
        assert_eq!(
            built.body,
            json!({
                "mutatie": {"rekening": 26852133, "datum": "2026-11-06", "soort": 6, "inEx": "IN", "omschrijving": "Parkeergarage"},
                "mutatieRegels": [{"index": 0, "tegenRekening": 26852167, "btwCode": "GEEN", "bedrag": 2.0}],
                "importId": 465233844u64
            })
        );
        assert_eq!(built.title, "Mutatie toevoegen: Geld uitgegeven");
        let shown: Vec<String> = built
            .lines
            .iter()
            .map(|l| format!("{}: {}", l.label, l.value))
            .collect();
        assert!(
            shown.contains(&"Rekening: 1010 Bank".to_owned()),
            "{shown:?}"
        );
        assert!(
            shown.contains(&"Regel 1: 4550 Bankkosten: 2,00 (Geen btw)".to_owned()),
            "{shown:?}"
        );
        assert!(shown.iter().any(|l| l.contains("465233844")), "{shown:?}");
    }

    #[test]
    fn an_invoice_mutatie_needs_its_relatie_and_number_and_finds_its_own_account() {
        let mut args = toevoegen(Soort::FactuurOntvangen);
        args.regels = Some(vec![regel("4700", 1.21, Some("HOOG_INK_21"))]);
        assert!(
            mutatie_nieuw(&args, &books())
                .unwrap_err()
                .contains("relatie")
        );
        args.relatie_id = Some(46484769);
        assert!(
            mutatie_nieuw(&args, &books())
                .unwrap_err()
                .contains("factuurnummer")
        );
        args.factuur = Some("INK-1".into());
        args.termijn = Some(30);
        let body = mutatie_nieuw(&args, &books()).unwrap().body;
        assert_eq!(
            body["mutatie"],
            json!({"rekening": 26852140, "datum": "2026-11-06", "soort": 1, "inEx": "IN", "omschrijving": "", "relatieId": 46484769, "factuur": "INK-1", "termijn": 30})
        );
        assert_eq!(body["mutatieRegels"][0]["btwCode"], "HOOG_INK_21");
        // Money in or out has no account of its own to fall back on.
        let mut geld = toevoegen(Soort::GeldOntvangen);
        geld.regels = Some(vec![regel("8000", 1.0, None)]);
        assert!(
            mutatie_nieuw(&geld, &books())
                .unwrap_err()
                .contains("rekening")
        );
    }

    #[test]
    fn a_payment_names_the_invoices_it_pays() {
        let mut args = toevoegen(Soort::FactuurbetalingOntvangen);
        args.rekening = Some("1010".into());
        args.betalingen = Some(vec![BetalingArg {
            relatie_id: 46484769,
            factuur: " F00002 ".into(),
            bedrag: 1.21,
        }]);
        let body = mutatie_nieuw(&args, &books()).unwrap().body;
        assert_eq!(
            body["mutatie"],
            json!({"rekening": 26852133, "datum": "2026-11-06", "soort": 3, "omschrijving": ""})
        );
        assert_eq!(
            body["mutatieRegels"],
            json!([{"index": 0, "bedrag": 1.21, "btw": 0, "btwCode": "GEEN", "relatieId": 46484769, "factuur": "F00002"}])
        );
        args.regels = Some(vec![regel("8000", 1.0, None)]);
        assert!(
            mutatie_nieuw(&args, &books()).is_err(),
            "lines do not belong to a payment"
        );
    }

    #[test]
    fn refuses_what_the_api_would_only_answer_with_a_bare_error() {
        let mut args = toevoegen(Soort::Memoriaal);
        args.rekening = Some("2000".into());
        assert!(
            mutatie_nieuw(&args, &books())
                .unwrap_err()
                .contains("regel")
        );
        args.regels = Some(vec![regel("4900", 0.0, None)]);
        assert!(mutatie_nieuw(&args, &books()).unwrap_err().contains("nul"));
        args.regels = Some(vec![regel("4900", f64::NAN, None)]);
        assert!(mutatie_nieuw(&args, &books()).is_err());
        args.regels = Some(vec![regel("4900", 1.0, None)]);
        args.datum = "06-11-2026".into();
        assert!(mutatie_nieuw(&args, &books()).is_err());
        args.datum = "2026-11-06".into();
        args.bedragen = Some(InEx::Incl);
        assert_eq!(
            mutatie_nieuw(&args, &books()).unwrap().body["mutatie"]["inEx"],
            "EX",
            "memoriaal is always EX"
        );
    }

    /// `GET mutatie/nr/3908/model`, as e-Boekhouden gave it.
    fn model() -> Value {
        json!({
            "folderKoppelId": 3908,
            "mutatie": {"betalingsKenmerk": "", "boekstuk": "", "datum": "2026-11-06T00:00:00", "factuur": "", "inEx": "IN", "nr": 3908,
                        "omschrijving": "Parkeergarage", "rekening": 26852133, "relatieId": 0, "soort": 6, "termijn": 0},
            "mutatieRegels": [{"bedrag": 2, "bedragExclusief": 2, "bedragInclusief": 2, "btw": 0, "btwCode": "GEEN", "index": 0,
                               "kostenplaats": 0, "omschrijving": "Parkeergarage", "tegenRekening": 26852167}]
        })
    }

    fn wijzigen(nr: u64) -> MutatieWijzigenArgs {
        MutatieWijzigenArgs {
            nr,
            datum: None,
            rekening: None,
            omschrijving: None,
            relatie_id: None,
            factuur: None,
            termijn: None,
            bedragen: None,
            regels: None,
            betalingen: None,
            boekstuk: None,
            account: None,
        }
    }

    #[test]
    fn an_edit_sends_the_whole_mutatie_back_with_only_the_named_change() {
        let mut args = wijzigen(3908);
        assert!(
            mutatie_gewijzigd(&args, &model(), &books())
                .unwrap_err()
                .contains("niets gewijzigd")
        );
        args.omschrijving = Some("Parkeren Groningen".into());
        let built = mutatie_gewijzigd(&args, &model(), &books()).unwrap();
        assert_eq!(
            built.body,
            json!({
                "mutatie": {"rekening": 26852133, "datum": "2026-11-06", "soort": 6, "inEx": "IN", "omschrijving": "Parkeren Groningen", "nr": 3908},
                "mutatieRegels": [{"index": 0, "tegenRekening": 26852167, "btwCode": "GEEN", "bedrag": 2.0}],
                "folderKoppelId": 3908
            })
        );
        assert_eq!(built.title, "Mutatie 3908 wijzigen (Geld uitgegeven)");
        // New lines replace the old ones; the rest stays.
        let mut args = wijzigen(3908);
        args.regels = Some(vec![
            regel("4550", 1.5, None),
            regel("4700", 0.5, Some("HOOG_INK_21")),
        ]);
        let body = mutatie_gewijzigd(&args, &model(), &books()).unwrap().body;
        assert_eq!(body["mutatie"]["omschrijving"], "Parkeergarage");
        assert_eq!(body["mutatieRegels"].as_array().unwrap().len(), 2);
        assert_eq!(
            body["mutatieRegels"][1],
            json!({"index": 1, "tegenRekening": 26852160, "btwCode": "HOOG_INK_21", "bedrag": 0.5})
        );
        args.betalingen = Some(vec![]);
        assert!(mutatie_gewijzigd(&args, &model(), &books()).is_err());
    }

    #[test]
    fn an_edited_payment_keeps_the_invoices_it_paid() {
        let payment = json!({
            "mutatie": {"datum": "2026-11-11T00:00:00", "factuur": "F00002", "inEx": "EX", "nr": 11, "omschrijving": "Betaling", "rekening": 26852133, "relatieId": 46484769, "soort": 3, "termijn": 0},
            "mutatieRegels": [{"bedrag": 1.21, "btw": 0, "btwCode": "GEEN", "index": 0, "kostenplaats": 0, "omschrijving": "Betaling",
                               "openPostModel": {"bedrag": 1.21, "factuurNummer": "F00002", "relatieId": 46484769}, "tegenRekening": 26852134}]
        });
        let mut args = wijzigen(11);
        args.datum = Some("2026-11-12".into());
        let body = mutatie_gewijzigd(&args, &payment, &books()).unwrap().body;
        assert_eq!(
            body["mutatie"],
            json!({"rekening": 26852133, "datum": "2026-11-12", "soort": 3, "omschrijving": "Betaling", "nr": 11})
        );
        assert_eq!(
            body["mutatieRegels"],
            json!([{"index": 0, "bedrag": 1.21, "btw": 0, "btwCode": "GEEN", "relatieId": 46484769, "factuur": "F00002"}])
        );
        // An opening balance is not one of the seven kinds.
        let opening = json!({"mutatie": {"soort": 0}, "mutatieRegels": []});
        assert!(mutatie_gewijzigd(&wijzigen(1), &opening, &books()).is_err());
    }

    fn context() -> FactuurContext {
        FactuurContext {
            volgend_nummer: "F00003".into(),
            sjabloon_id: 878203,
            verwerken: true,
            vandaag: (2026, 11, 11),
        }
    }

    fn factuurregel() -> FactuurRegelArg {
        FactuurRegelArg {
            aantal: None,
            omschrijving: "Advies".into(),
            prijs: 1.0,
            btw_code: "LAAG_VERK_9".into(),
            tegenrekening: "0130".into(),
            code: None,
        }
    }

    #[test]
    fn makes_an_invoice_as_e_boekhoudens_form_does() {
        let args = FactuurToevoegenArgs {
            relatie_id: 46484769,
            datum: None,
            nummer: None,
            termijn: None,
            bedragen: None,
            regels: vec![factuurregel()],
            factuurtekst: None,
            kenmerk: None,
            direct_boeken: None,
            sjabloon_id: None,
            account: None,
        };
        let built = factuur_nieuw(&args, &context(), &books(), None).unwrap();
        // The body the web form posted to factuur/model for F00003.
        assert_eq!(
            built.body,
            json!({
                "factuur": {"relatieId": 46484769, "nummer": "F00003", "inEx": "EX", "datum": {"day": 11, "month": 11, "year": 2026}, "termijn": 0,
                            "sjabloonId": 878203, "emailSjabloonId": null, "type": "N", "btwLaag": 0, "btwHoog": 0, "totaal": 0},
                "factuurItems": {"insert": [{"id": -1, "factuurId": 0, "aantal": 1.0, "omschrijving": "Advies", "prijs": null, "btw": "LAAG_VERK_9",
                                             "grootboekId": 26852146, "prijsExcl": 1.0, "prijsIncl": 1.09, "kortingIsPerc": true, "volgorde": 0}],
                                 "update": [], "delete": []},
                "verwerk": {"grootboekrekeningId": 26852134}
            })
        );
        assert_eq!(built.title, "Factuur F00003 toevoegen");
        // Incl prices: the excl price is derived.
        let mut incl = args;
        incl.bedragen = Some(InEx::Incl);
        incl.direct_boeken = Some(false);
        incl.regels[0].prijs = 1.09;
        let body = factuur_nieuw(&incl, &context(), &books(), None)
            .unwrap()
            .body;
        assert_eq!(body["factuurItems"]["insert"][0]["prijsExcl"], 1.0);
        assert_eq!(body["factuurItems"]["insert"][0]["prijsIncl"], 1.09);
        assert!(body.get("verwerk").is_none());
        incl.regels[0].btw_code = "HOOG_INK_21".into();
        assert!(
            factuur_nieuw(&incl, &context(), &books(), None)
                .unwrap_err()
                .contains("inkoop")
        );
        incl.regels[0].btw_code = "AFW_VERK".into();
        assert!(factuur_nieuw(&incl, &context(), &books(), None).is_err());
        incl.regels.clear();
        assert!(factuur_nieuw(&incl, &context(), &books(), None).is_err());
    }

    fn factuur_now() -> (Value, Value) {
        (
            json!({"id": 73391389, "relatieId": 46484769, "nummer": "F00003", "inEx": "EX", "datum": {"day": 11, "month": 11, "year": 2026},
                   "termijn": 0, "sjabloonId": 878203, "type": "N", "factuurtekst": "", "kenmerk": "", "totaal": 1.09}),
            json!([{"id": 148611205, "aantal": 1, "btw": "LAAG_VERK_9", "code": "", "eenheidId": 0, "grootboekId": 26852146,
                    "kortingIsPerc": true, "omschrijving": "Advies", "prijsExcl": 1, "prijsIncl": 1.09, "volgorde": "0"}]),
        )
    }

    #[test]
    fn an_invoice_edit_keeps_what_it_does_not_name() {
        let (now, items) = factuur_now();
        let mut args = FactuurWijzigenArgs {
            id: 73391389,
            datum: None,
            nummer: None,
            termijn: None,
            factuurtekst: None,
            kenmerk: None,
            bedragen: None,
            regels: None,
            sjabloon_id: None,
            account: None,
        };
        assert!(
            factuur_gewijzigd(&args, &now, &items, &books(), None).is_err(),
            "nothing to change"
        );
        args.factuurtekst = Some("Bedankt".into());
        let body = factuur_gewijzigd(&args, &now, &items, &books(), None)
            .unwrap()
            .body;
        assert_eq!(body["factuur"]["id"], 73391389);
        assert_eq!(body["factuur"]["nummer"], "F00003");
        assert_eq!(
            body["factuur"]["datum"],
            json!({"day": 11, "month": 11, "year": 2026})
        );
        assert_eq!(body["factuur"]["factuurtekst"], "Bedankt");
        assert_eq!(
            body["factuurItems"],
            json!({"insert": [], "update": [], "delete": []})
        );
        assert!(body.get("updateRelated").is_none());
        // A new number takes its mutatie along; new lines replace the old ones.
        args.nummer = Some("F00009".into());
        args.regels = Some(vec![factuurregel()]);
        let body = factuur_gewijzigd(&args, &now, &items, &books(), None)
            .unwrap()
            .body;
        assert_eq!(
            body["updateRelated"],
            json!({"oldFactuurNummer": "F00003", "factuurNummer": "F00009", "oldRelatieId": 46484769, "relatieId": 46484769})
        );
        assert_eq!(body["factuurItems"]["delete"], json!([148611205]));
        assert_eq!(body["factuurItems"]["insert"].as_array().unwrap().len(), 1);
    }

    fn sjablonen() -> Value {
        json!({"data": [
            {"id": 878203, "naam": "Factuursjabloon", "inactief": false, "type": "E"},
            {"id": 900001, "naam": "Engels", "inactief": false, "type": "E"},
            {"id": 900002, "naam": "Oud", "inactief": true, "type": "A"}
        ]})
    }

    #[test]
    fn an_invoice_takes_the_layout_template_it_is_given() {
        let mut args = FactuurToevoegenArgs {
            relatie_id: 46484769,
            datum: None,
            nummer: None,
            termijn: None,
            bedragen: None,
            regels: vec![factuurregel()],
            factuurtekst: None,
            kenmerk: None,
            direct_boeken: None,
            sjabloon_id: Some(900001),
            account: None,
        };
        let built = factuur_nieuw(&args, &context(), &books(), Some(&sjablonen())).unwrap();
        assert_eq!(built.body["factuur"]["sjabloonId"], 900001);
        assert!(
            built
                .lines
                .iter()
                .any(|l| l.label == "Factuursjabloon" && l.value == "Engels"),
            "{:?}",
            built.lines
        );
        // Unknown, inactive, or not read: refused before anything is asked.
        args.sjabloon_id = Some(5);
        let unknown = factuur_nieuw(&args, &context(), &books(), Some(&sjablonen())).unwrap_err();
        assert!(
            unknown.contains("Geen factuursjabloon met id 5")
                && unknown.contains("Factuursjabloon (878203), Engels (900001)")
                && !unknown.contains("Oud"),
            "{unknown}"
        );
        args.sjabloon_id = Some(900002);
        assert!(
            factuur_nieuw(&args, &context(), &books(), Some(&sjablonen()))
                .unwrap_err()
                .contains("inactief")
        );
        args.sjabloon_id = Some(900001);
        assert!(factuur_nieuw(&args, &context(), &books(), None).is_err());
        // Without one the administration's own stays, and no line is added.
        args.sjabloon_id = None;
        let built = factuur_nieuw(&args, &context(), &books(), None).unwrap();
        assert_eq!(built.body["factuur"]["sjabloonId"], 878203);
        assert!(!built.lines.iter().any(|l| l.label == "Factuursjabloon"));
        // Plain arrays are read too.
        assert_eq!(
            factuur_sjabloon(878203, Some(&sjablonen()["data"])),
            Ok((878203, "Factuursjabloon".to_owned()))
        );
    }

    #[test]
    fn an_invoice_edit_shows_the_old_and_the_new_template() {
        let (now, items) = factuur_now();
        let mut args = FactuurWijzigenArgs {
            id: 73391389,
            datum: None,
            nummer: None,
            termijn: None,
            factuurtekst: None,
            kenmerk: None,
            bedragen: None,
            regels: None,
            sjabloon_id: Some(900001),
            account: None,
        };
        let built = factuur_gewijzigd(&args, &now, &items, &books(), Some(&sjablonen())).unwrap();
        assert_eq!(built.body["factuur"]["sjabloonId"], 900001);
        assert!(
            built
                .lines
                .iter()
                .any(|l| l.label == "Factuursjabloon wordt"
                    && l.value == "Engels (was Factuursjabloon)"),
            "{:?}",
            built.lines
        );
        // The template it has already is no change.
        args.sjabloon_id = Some(878203);
        assert!(
            factuur_gewijzigd(&args, &now, &items, &books(), Some(&sjablonen()))
                .unwrap_err()
                .contains("niets gewijzigd")
        );
        args.sjabloon_id = Some(900002);
        assert!(
            factuur_gewijzigd(&args, &now, &items, &books(), Some(&sjablonen()))
                .unwrap_err()
                .contains("inactief")
        );
    }

    #[test]
    fn a_credit_invoice_mirrors_the_original() {
        let (now, items) = factuur_now();
        let args = FactuurCreditArgs {
            id: 73391389,
            datum: None,
            nummer: Some("F00004".into()),
            account: None,
        };
        let built = factuur_credit(&args, &now, &items, &context(), &books()).unwrap();
        let item = &built.body["factuurItems"]["insert"][0];
        assert_eq!(item["aantal"], -1.0);
        assert_eq!(item["omschrijving"], "Advies- Creditering -");
        assert_eq!(item["prijsExcl"], 1);
        assert_eq!(item["btw"], "LAAG_VERK_9");
        assert_eq!(built.body["factuur"]["nummer"], "F00004");
        assert!(
            built.body["factuur"].get("id").is_none(),
            "a new invoice, not an edit"
        );
        assert_eq!(
            built.body["verwerk"],
            json!({"grootboekrekeningId": 26852134})
        );
        assert!(
            built
                .lines
                .iter()
                .any(|l| l.label == "Totaal incl. btw" && l.value == "-1,09"),
            "{:?}",
            built.lines
        );
        assert!(factuur_credit(&args, &now, &json!([]), &context(), &books()).is_err());
    }

    #[test]
    fn makes_a_subscription_as_e_boekhoudens_form_does() {
        let args = AbonnementToevoegenArgs {
            relatie_id: 46484769,
            naam: " Onderhoud ".into(),
            per: Per::Maand,
            termijn: 14,
            start: None,
            prolongatie: None,
            bedragen: None,
            regels: vec![factuurregel()],
            kenmerk: None,
            niet_factureren: None,
            account: None,
        };
        let built = abonnement_nieuw(&args, 878203, (2026, 11, 11), &books()).unwrap();
        assert_eq!(
            built.body["factuur"],
            json!({"relatieId": 46484769, "nummer": "Onderhoud", "sjabloonId": 878203, "versturenViaPeppol": false, "inEx": "EX", "type": "E",
                   "termijn": 14, "btwLaag": 0, "btwHoog": 0, "totaal": 0, "factOm1": "Factuur: [FACTUUR]",
                   "factOm2": "Periode: [START] t/m [EIND]", "autoVerwerkMethod": null})
        );
        assert_eq!(
            built.body["factuurExtra"],
            json!({"startDatum": {"day": 11, "month": 11, "year": 2026}, "expiratieDatum": {"day": 11, "month": 11, "year": 2026}, "per": "M"})
        );
        let mut early = args;
        early.start = Some("2026-12-01".into());
        early.prolongatie = Some("2026-11-01".into());
        assert!(abonnement_nieuw(&early, 878203, (2026, 11, 11), &books()).is_err());
    }

    #[test]
    fn a_subscription_edit_sends_its_dates_and_frequency_back() {
        let now = json!({"id": 73391447, "relatieId": 46484769, "nummer": "Onderhoud", "inEx": "EX", "termijn": 14, "sjabloonId": 878203,
                         "factOm1": "Factuur: [FACTUUR]", "factOm2": "Periode: [START] t/m [EIND]", "geenFactuur": false});
        let extra = json!({"id": 5360036, "startDatum": {"year": 2026, "month": 11, "day": 11}, "expiratieDatum": {"year": 2026, "month": 12, "day": 11}, "per": "M"});
        let mut args = AbonnementWijzigenArgs {
            id: 73391447,
            naam: None,
            per: Some(Per::Kwartaal),
            termijn: Some(30),
            start: None,
            prolongatie: None,
            kenmerk: None,
            niet_factureren: None,
            bedragen: None,
            regels: None,
            account: None,
        };
        let body = abonnement_gewijzigd(&args, &now, &extra, &json!([]), &books())
            .unwrap()
            .body;
        assert_eq!(body["factuur"]["id"], 73391447);
        assert_eq!(body["factuur"]["type"], "E");
        assert_eq!(body["factuur"]["termijn"], 30);
        assert_eq!(body["factuur"]["factOm2"], "Periode: [START] t/m [EIND]");
        assert_eq!(
            body["factuurExtra"],
            json!({"startDatum": {"day": 11, "month": 11, "year": 2026}, "expiratieDatum": {"day": 11, "month": 12, "year": 2026}, "per": "K"})
        );
        args.prolongatie = Some("2026-10-01".into());
        assert!(abonnement_gewijzigd(&args, &now, &extra, &json!([]), &books()).is_err());
    }

    #[test]
    fn invoicing_subscriptions_says_how_many_it_takes() {
        let args = AbonnementenFacturerenArgs {
            venster: None,
            factuurdatum: Some("2026-11-11".into()),
            direct_boeken: None,
            account: None,
        };
        let built = abonnementen_factureren(&args, 3, (2026, 11, 12), &books()).unwrap();
        assert_eq!(
            built.body,
            json!({"factuurdatum": "2026-11-11", "directVerwerken": false, "expirationPeriod": "Expired", "type": "E", "grootboekRekeningId": 26852134})
        );
        assert_eq!(built.title, "Facturen maken voor 3 abonnementen");
        assert!(
            abonnementen_factureren(&args, 0, (2026, 11, 12), &books())
                .unwrap_err()
                .contains("geen abonnementen")
        );
        assert_eq!(Venster::TweeWeken.count_key(), "twoWeeks");
    }

    #[test]
    fn makes_a_quote_as_e_boekhoudens_form_does() {
        let context = OfferteContext {
            volgend_nummer: "OF-00001".into(),
            sjabloon_id: 446945,
            status_id: 2023411,
            gebruiker_id: 532387,
            vandaag: (2026, 11, 11),
        };
        let args = OfferteToevoegenArgs {
            relatie_id: 46484769,
            datum: None,
            vervaldatum: None,
            nummer: None,
            kenmerk: Some("Proef".into()),
            notitie: Some("Notitie".into()),
            interne_notitie: None,
            bedragen: None,
            regels: vec![factuurregel()],
            sjabloon_id: None,
            status_id: None,
            account: None,
        };
        let (built, regels) = offerte_nieuw(&args, &context, &books()).unwrap();
        // The body the web form posted to `offerte`.
        assert_eq!(
            built.body,
            json!({"statusId": 2023411, "nummer": "OF-00001", "behandelGebruikerId": 532387, "relatieId": 46484769, "kenmerk": "Proef",
                   "datum": {"day": 11, "month": 11, "year": 2026}, "vervaldatum": {"day": 11, "month": 12, "year": 2026},
                   "sjabloonId": 446945, "factuurEmailsjabloonId": "", "notitiePubliek": "Notitie", "inEx": "EX"})
        );
        // And the line it posted to `offerteregel`, without the quote's id.
        assert_eq!(
            regels,
            vec![
                json!({"id": 0, "aantal": 1.0, "volgorde": null, "omschrijving": "Advies", "prijsExcl": 1.0, "prijsIncl": 1.09,
                        "btwCode": "LAAG_VERK_9", "grootboekId": 26852146, "kortingIsPerc": true})
            ]
        );
        assert_eq!(month_later((2026, 12, 31)), (2027, 1, 31));
        assert_eq!(month_later((2026, 3, 30)), (2026, 4, 30));
        assert_eq!(month_later((2026, 1, 31)), (2026, 2, 28));
        assert_eq!(month_later((2026, 11, 11)), (2026, 12, 11));
        let mut early = args;
        early.vervaldatum = Some("2026-11-01".into());
        assert!(offerte_nieuw(&early, &context, &books()).is_err());
    }

    #[test]
    fn a_quote_edit_sends_the_quote_back_with_the_named_change() {
        let now = json!({"id": 2587158, "nummer": "OF-00001", "relatieId": 46484769, "kenmerk": "Proef", "inEx": "EX", "statusId": 2023411,
                         "datum": {"day": 11, "month": 11, "year": 2026}, "vervaldatum": {"day": 11, "month": 12, "year": 2026}, "sjabloonId": 446945});
        let mut args = OfferteWijzigenArgs {
            id: 2587158,
            datum: None,
            vervaldatum: None,
            nummer: None,
            kenmerk: None,
            notitie: None,
            interne_notitie: None,
            status_id: None,
            bedragen: None,
            regels: None,
            account: None,
        };
        assert!(offerte_gewijzigd(&args, &now, &books()).is_err());
        args.kenmerk = Some(" Gewijzigd ".into());
        let (built, regels) = offerte_gewijzigd(&args, &now, &books()).unwrap();
        assert_eq!(built.body["kenmerk"], "Gewijzigd");
        assert_eq!(built.body["id"], 2587158);
        assert_eq!(built.body["sjabloonId"], 446945);
        assert!(regels.is_none(), "the lines stay");
        args.regels = Some(vec![factuurregel()]);
        assert_eq!(
            offerte_gewijzigd(&args, &now, &books())
                .unwrap()
                .1
                .unwrap()
                .len(),
            1
        );
        args.vervaldatum = Some("2026-10-01".into());
        assert!(offerte_gewijzigd(&args, &now, &books()).is_err());
    }

    #[test]
    fn says_what_changes_in_a_template_layout() {
        let element = |id: i64, content: &str, x: i64| json!({"id": id, "tag": "text", "x": x, "y": 10, "width": 100, "height": 20, "content": content, "style": {}});
        let now = json!({"elements": [element(1, "<h1>FACTUUR</h1>", 3), element(2, "<p>[DATUM]</p>", 5), element(3, "<p>weg</p>", 5)], "documentDefinition": {"width": 840}});
        let same = opmaak_changes(&now, &now).unwrap();
        assert!(same.is_empty(), "{same:?}");
        let new = json!({"elements": [element(1, "<h1>FACTUUR PROEF</h1>", 3), element(2, "<p>[DATUM]</p>", 50), element(8, "<p>Nieuw <b>blok</b></p>", 5)], "documentDefinition": {"width": 840}});
        let shown: Vec<String> = opmaak_changes(&now, &new)
            .unwrap()
            .iter()
            .map(|l| format!("{}: {}", l.label, l.value))
            .collect();
        assert_eq!(
            shown,
            vec![
                "Tekst van text 1 was: FACTUUR",
                "Tekst van text 1 wordt: FACTUUR PROEF",
                "Plaats van text 2: x 5 wordt 50",
                "Nieuw: text 8 \"Nieuw blok\"",
                "Verwijderd: text 3 \"weg\"",
            ]
        );
        // A change that leaves the words alone (a link's target, a colour) is still named.
        let mut restyled = now.clone();
        restyled["elements"][0]["content"] =
            json!("<h1><a href=\"https://x.example\">FACTUUR</a></h1>");
        restyled["elements"][1]["style"] = json!({"zindex": 3});
        let shown: Vec<String> = opmaak_changes(&now, &restyled)
            .unwrap()
            .iter()
            .map(|l| format!("{}: {}", l.label, l.value))
            .collect();
        assert!(
            shown.iter().any(|l| l.starts_with("Opmaak van text 1")),
            "{shown:?}"
        );
        assert!(
            shown.contains(&"Instellingen van text 2: gewijzigd: style".to_owned()),
            "{shown:?}"
        );
        assert!(opmaak_changes(&now, &json!({"elements": []})).is_err());
        assert!(opmaak_changes(&now, &json!({})).is_err());
        assert!(
            opmaak_changes(
                &now,
                &json!({"elements": [element(1, "a", 1), element(1, "b", 1)]})
            )
            .unwrap_err()
            .contains("twee keer")
        );
        assert!(
            opmaak_changes(&now, &json!({"elements": [{"id": 1, "tag": "text"}]}))
                .unwrap_err()
                .contains("mist")
        );
    }

    #[test]
    fn writes_amounts_and_dates_the_dutch_way() {
        assert_eq!(euro(1234.5), "1234,50");
        assert_eq!(euro(-2.185), "-2,19");
        assert_eq!(date_of(&json!("2026-11-06T00:00:00")), Some((2026, 11, 6)));
        assert_eq!(
            date_of(&json!({"day": 1, "month": 1, "year": 1})),
            None,
            "e-Boekhouden's empty date"
        );
    }
}
