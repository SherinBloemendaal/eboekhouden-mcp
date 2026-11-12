//! What the Uren tools send to e-Boekhouden, and how the user sees it first. Nothing here talks to
//! e-Boekhouden: it turns a tool's arguments, plus what was read before, into a request body and the lines of
//! the approval window. uren_beheer.rs reads and sends.
//!
//! Every shape comes from the Uren module of e-Boekhouden's own web app (its Angular bundle, read on
//! 2026-11-12). Its screens use one generic service, `new Ke("uur" | "project" | "activiteit" |
//! "factuurSetting", form)`: a new record goes out as `POST {controller}`, a change as `PUT {controller}`, a
//! delete as `DELETE {controller}/{id}`, and one record is read with `GET {controller}/{id}`.

use std::collections::HashMap;

use serde_json::Map;

use crate::approvals::Line;

use super::*;

type Date = (i32, u32, u32);

/// The highest number the forms take for an amount of hours, kilometres or a tariff (`new $t(1e5)`).
const MAX_AANTAL: f64 = 100_000.0;
/// The most invoice lines (one per project and activity) the hours screen puts on one invoice.
pub(super) const MAX_FACTUURREGELS: usize = 100;

// ---------------------------------------------------------------- arguments

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct UurToevoegenArgs {
    /// Datum, JJJJ-MM-DD.
    pub datum: String,
    /// Id van het project (uit projecten).
    pub project_id: u64,
    /// Id van de activiteit (uit activiteiten).
    pub activiteit_id: u64,
    /// Aantal uren, bijvoorbeeld 1.5. Uren of kilometers is nodig.
    pub uren: Option<f64>,
    /// Aantal kilometers. Uren of kilometers is nodig.
    pub kilometers: Option<f64>,
    /// Opmerkingen bij de regel.
    pub opmerkingen: Option<String>,
    /// Id van de medewerker (uit uren_medewerkers), alleen voor een accountant of hoofdgebruiker. Zonder: de ingelogde gebruiker.
    pub medewerker_id: Option<u64>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct UurWijzigenArgs {
    /// Id van de urenregel (uit uren).
    pub id: u64,
    /// Nieuwe datum, JJJJ-MM-DD.
    pub datum: Option<String>,
    /// Ander project (id).
    pub project_id: Option<u64>,
    /// Andere activiteit (id).
    pub activiteit_id: Option<u64>,
    /// Nieuw aantal uren; 0 maakt het veld leeg (het gaat dan niet mee, zoals een leeg veld in e-Boekhouden).
    pub uren: Option<f64>,
    /// Nieuw aantal kilometers; 0 maakt het veld leeg (het gaat dan niet mee, zoals een leeg veld in e-Boekhouden).
    pub kilometers: Option<f64>,
    /// Nieuwe opmerkingen; een lege tekst maakt ze leeg.
    pub opmerkingen: Option<String>,
    /// Andere medewerker (id uit uren_medewerkers), alleen voor een accountant of hoofdgebruiker.
    pub medewerker_id: Option<u64>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ProjectToevoegenArgs {
    /// Naam van het project, hoogstens 50 tekens.
    pub naam: String,
    /// Id van de relatie (klant) van het project (uit relaties).
    pub relatie_id: u64,
    /// Omschrijving.
    pub omschrijving: Option<String>,
    /// Of de uren op dit project gefactureerd worden (standaard niet).
    pub facturabel: Option<bool>,
    /// Of het project actief is (standaard wel).
    pub actief: Option<bool>,
    /// Ids van de gebruikers die uren op dit project mogen schrijven (uit uren_medewerkers). Standaard alle gebruikers, zoals het formulier van e-Boekhouden.
    pub gebruiker_ids: Option<Vec<u64>>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ProjectWijzigenArgs {
    /// Id van het project (uit projecten).
    pub id: u64,
    /// Nieuwe naam, hoogstens 50 tekens.
    pub naam: Option<String>,
    /// Andere relatie (id).
    pub relatie_id: Option<u64>,
    /// Nieuwe omschrijving; een lege tekst maakt hem leeg.
    pub omschrijving: Option<String>,
    /// Facturabel aan of uit.
    pub facturabel: Option<bool>,
    /// Actief aan of uit.
    pub actief: Option<bool>,
    /// De gebruikers die uren op dit project mogen schrijven: deze lijst vervangt de huidige.
    pub gebruiker_ids: Option<Vec<u64>>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ActiviteitToevoegenArgs {
    /// Naam van de activiteit, hoogstens 50 tekens.
    pub naam: String,
    /// Omschrijving.
    pub omschrijving: Option<String>,
    /// Uurtarief in euro's.
    pub uurtarief: Option<f64>,
    /// Kilometertarief in euro's.
    pub kilometertarief: Option<f64>,
    /// Of de uren van deze activiteit gefactureerd worden (standaard niet). Dan zijn een tarief, een tegenrekening en (bij een btw-plichtige administratie) een btw-code nodig.
    pub facturabel: Option<bool>,
    /// Tekst op de factuur, hoogstens 50 tekens.
    pub factuuromschrijving: Option<String>,
    /// Btw-code voor de verkoop, bijvoorbeeld HOOG_VERK_21. Alleen bij facturabel.
    pub btw_code: Option<String>,
    /// Code van de omzetrekening (grootboekrekening van winst en verlies of balans). Alleen bij facturabel; standaard 8000 als die bestaat.
    pub tegenrekening: Option<String>,
    /// Of de activiteit actief is (standaard wel).
    pub actief: Option<bool>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ActiviteitWijzigenArgs {
    /// Id van de activiteit (uit activiteiten).
    pub id: u64,
    /// Nieuwe naam, hoogstens 50 tekens.
    pub naam: Option<String>,
    /// Nieuwe omschrijving; een lege tekst maakt hem leeg.
    pub omschrijving: Option<String>,
    /// Nieuw uurtarief in euro's.
    pub uurtarief: Option<f64>,
    /// Nieuw kilometertarief in euro's.
    pub kilometertarief: Option<f64>,
    /// Facturabel aan of uit.
    pub facturabel: Option<bool>,
    /// Nieuwe tekst op de factuur, hoogstens 50 tekens.
    pub factuuromschrijving: Option<String>,
    /// Andere btw-code voor de verkoop.
    pub btw_code: Option<String>,
    /// Code van een andere omzetrekening.
    pub tegenrekening: Option<String>,
    /// Actief aan of uit.
    pub actief: Option<bool>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// Which hours the invoicing screen lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ZoekenIn {
    /// Nog niet gefactureerde uren en kilometers.
    NogNietGefactureerd,
    /// Reeds gefactureerde uren en kilometers.
    Gefactureerd,
    /// Alle uren en kilometers.
    Alle,
}

impl ZoekenIn {
    pub(super) fn code(self) -> &'static str {
        match self {
            Self::NogNietGefactureerd => "FACTUREREN",
            Self::Gefactureerd => "GEFACTUREERD",
            Self::Alle => "ALLE",
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct UrenTeFacturerenArgs {
    /// Begin van de periode, JJJJ-MM-DD (standaard de eerste van deze maand).
    pub van: Option<String>,
    /// Einde van de periode, JJJJ-MM-DD (standaard de laatste dag van deze maand).
    pub tot: Option<String>,
    /// Alleen uren van deze medewerker (id uit uren_medewerkers).
    pub medewerker_id: Option<u64>,
    /// Alleen dit project (id).
    pub project_id: Option<u64>,
    /// Alleen deze activiteit (id).
    pub activiteit_id: Option<u64>,
    /// nog_niet_gefactureerd (standaard), gefactureerd of alle.
    pub zoeken_in: Option<ZoekenIn>,
    /// Ook uren en kilometers die niet facturabel zijn (standaard niet).
    pub ook_niet_facturabel: Option<bool>,
    /// Aantal rijen, 1 tot 200 (standaard 50).
    pub limit: Option<u32>,
    /// Vanaf welke rij (standaard 0).
    pub offset: Option<u32>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct UrenFacturerenArgs {
    /// Begin van de periode, JJJJ-MM-DD (standaard de eerste van deze maand).
    pub van: Option<String>,
    /// Einde van de periode, JJJJ-MM-DD (standaard de laatste dag van deze maand).
    pub tot: Option<String>,
    /// Alleen uren van deze medewerker (id).
    pub medewerker_id: Option<u64>,
    /// Alleen dit project (id).
    pub project_id: Option<u64>,
    /// Alleen deze activiteit (id).
    pub activiteit_id: Option<u64>,
    /// Ook uren en kilometers die niet facturabel zijn (standaard niet).
    pub ook_niet_facturabel: Option<bool>,
    /// Alleen deze urenregels (ids uit uren_te_factureren). Zonder: alle nog niet gefactureerde uren die aan de filters voldoen.
    pub uur_ids: Option<Vec<u64>>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct UrenBijFactuurArgs {
    /// Nummer van de bestaande factuur, bijvoorbeeld F00012.
    pub factuurnummer: String,
    /// Ids van de urenregels die op die factuur komen (uit uren_te_factureren of uren).
    pub uur_ids: Vec<u64>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// Hoe een overzicht de uren telt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Weergave {
    /// Aantal uren.
    Uren,
    /// Bedragen in euro's.
    Euro,
}

impl Weergave {
    pub(super) fn code(self) -> &'static str {
        match self {
            Self::Uren => "uren",
            Self::Euro => "euro",
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct UrenPerProjectArgs {
    /// Begin van de periode, JJJJ-MM-DD (standaard 1 januari van dit jaar).
    pub van: Option<String>,
    /// Einde van de periode, JJJJ-MM-DD (standaard vandaag).
    pub tot: Option<String>,
    /// Alleen deze activiteit (id).
    pub activiteit_id: Option<u64>,
    /// uren (standaard) of euro.
    pub weergave: Option<Weergave>,
    /// Aantal rijen, 1 tot 200 (standaard 50).
    pub limit: Option<u32>,
    /// Vanaf welke rij (standaard 0).
    pub offset: Option<u32>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct UrenPerActiviteitArgs {
    /// Begin van de periode, JJJJ-MM-DD (standaard 1 januari van dit jaar).
    pub van: Option<String>,
    /// Einde van de periode, JJJJ-MM-DD (standaard vandaag).
    pub tot: Option<String>,
    /// Alleen dit project (id).
    pub project_id: Option<u64>,
    /// uren (standaard) of euro.
    pub weergave: Option<Weergave>,
    /// Aantal rijen, 1 tot 200 (standaard 50).
    pub limit: Option<u32>,
    /// Vanaf welke rij (standaard 0).
    pub offset: Option<u32>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct UrenPerJaarArgs {
    /// Het jaar (standaard dit jaar): 1 januari tot en met 31 december.
    pub jaar: Option<i32>,
    /// Alleen dit project (id).
    pub project_id: Option<u64>,
    /// Alleen deze activiteit (id).
    pub activiteit_id: Option<u64>,
    /// uren (standaard) of euro.
    pub weergave: Option<Weergave>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ProjectenArgs {
    /// Zoek op de naam van het project.
    pub zoek: Option<String>,
    /// Alleen projecten van deze relatie (id).
    pub relatie_id: Option<u64>,
    /// Aantal rijen, 1 tot 200 (standaard 50).
    pub limit: Option<u32>,
    /// Vanaf welke rij (standaard 0).
    pub offset: Option<u32>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ActiviteitenArgs {
    /// Zoek op de naam van de activiteit.
    pub zoek: Option<String>,
    /// Aantal rijen, 1 tot 200 (standaard 50).
    pub limit: Option<u32>,
    /// Vanaf welke rij (standaard 0).
    pub offset: Option<u32>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FactuurinstellingenWijzigenArgs {
    /// Id van de eenheid voor uren op de factuur (uit uren_factuurinstellingen, eenheden).
    pub uur_eenheid_id: Option<u64>,
    /// Id van de eenheid voor kilometers op de factuur.
    pub km_eenheid_id: Option<u64>,
    /// Id van het factuursjabloon (uit uren_factuurinstellingen, factuursjablonen).
    pub factuursjabloon_id: Option<u64>,
    /// Opmaak van een factuurregel, hoogstens 100 tekens, met de codes [PROJECT], [ACTIVITEIT], [OPMERKING] en [DATUM].
    pub regeltekst: Option<String>,
    /// Betalingstermijn in dagen, 0 tot 99999.
    pub betalingstermijn: Option<u32>,
    /// Facturen klaarzetten voor verzending per post.
    pub per_post: Option<bool>,
    /// Facturen klaarzetten voor verzending per e-mail.
    pub per_email: Option<bool>,
    /// Facturen incasseren.
    pub incasseren: Option<bool>,
    /// Facturen meteen verwerken in de boekhouding.
    pub verwerken: Option<bool>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

// ---------------------------------------------------------------- small helpers

/// A date as the forms send it: `dt` serializes as {"day","month","year"}.
pub(super) fn datum_object((y, m, d): Date) -> Value {
    json!({"day": d, "month": m, "year": y})
}

/// A date as the API gives it: "2026-11-06T00:00:00" or {"day":6,"month":11,"year":2026}.
pub(super) fn datum_of(value: &Value) -> Option<Date> {
    if let Some(text) = value.as_str() {
        return parse_date(text.get(..10)?).ok();
    }
    let part = |name: &str| value.get(name).and_then(Value::as_u64);
    let (y, m, d) = (part("year")?, part("month")?, part("day")?);
    parse_date(&format!("{y:04}-{m:02}-{d:02}")).ok()
}

pub(super) fn nl_datum((y, m, d): Date) -> String {
    format!("{d:02}-{m:02}-{y}")
}

/// The last day of a month.
pub(super) fn laatste_dag(y: i32, m: u32) -> Date {
    (28..=31)
        .rev()
        .map(|d| (y, m, d))
        .find(|&(y, m, d)| parse_date(&format!("{y:04}-{m:02}-{d:02}")).is_ok())
        .unwrap_or((y, m, 28))
}

/// A number for the approval window, the way e-Boekhouden writes one: "1,5", "8".
pub(super) fn getal(value: f64) -> String {
    let text = format!("{:.2}", (value * 100.0).round() / 100.0);
    let text = text.trim_end_matches('0').trim_end_matches('.');
    text.replace('.', ",")
}

pub(super) fn ja_nee(value: bool) -> &'static str {
    if value { "ja" } else { "nee" }
}

/// An amount of hours or kilometres, or a tariff: a real number up to the forms' maximum, never negative.
fn aantal(value: f64, what: &str) -> Result<f64, String> {
    if !value.is_finite() || value < 0.0 || value > MAX_AANTAL {
        return Err(format!("{what} moet een getal van 0 tot 100000 zijn"));
    }
    Ok((value * 100.0).round() / 100.0)
}

/// An amount of 0 is an empty field: the forms leave an empty field out of what they send.
fn leeg_bij_nul(value: Option<f64>) -> Option<f64> {
    value.filter(|v| *v != 0.0)
}

/// A name the forms require, trimmed, with their maximum length.
fn naam(value: &str, what: &str, max: usize) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{what} is niet ingevuld"));
    }
    if value.chars().count() > max {
        return Err(format!("{what} is langer dan {max} tekens"));
    }
    Ok(value.to_owned())
}

/// An optional text with a maximum length; a blank text stays blank (it empties the field).
fn tekst(value: Option<&str>, what: &str, max: usize) -> Result<Option<String>, String> {
    match value.map(str::trim) {
        Some(value) if value.chars().count() > max => {
            Err(format!("{what} is langer dan {max} tekens"))
        }
        Some(value) => Ok(Some(value.to_owned())),
        None => Ok(None),
    }
}

/// A value of a record as text for the approval window: "(leeg)" for nothing.
fn shown(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => "(leeg)".to_owned(),
        Some(Value::String(text)) if text.trim().is_empty() => "(leeg)".to_owned(),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Bool(b)) => ja_nee(*b).to_owned(),
        Some(Value::Number(n)) => n.as_f64().map(getal).unwrap_or_else(|| n.to_string()),
        Some(other) => other.to_string(),
    }
}

/// Adds "X was" and "X wordt" when the two differ. Says whether they did.
fn wijziging(lines: &mut Vec<Line>, label: &str, was: String, wordt: String) -> bool {
    if was == wordt {
        return false;
    }
    lines.push(line(&format!("{label} was"), was));
    lines.push(line(&format!("{label} wordt"), wordt));
    true
}

pub(super) fn id_of(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64).filter(|id| *id != 0)
}

pub(super) fn text_of(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

/// A list of `{id, <name>}` objects, as `user/selectlist`, `eenheid` and `factuursjabloon/active` give them.
pub(super) fn id_namen(list: &Value, name: &str) -> Vec<(u64, String)> {
    list.as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| Some((id_of(item, "id")?, text_of(item, name))))
                .collect()
        })
        .unwrap_or_default()
}

fn naam_bij(list: &[(u64, String)], id: u64, what: &str) -> String {
    list.iter()
        .find(|(i, _)| *i == id)
        .map(|(_, n)| n.clone())
        .unwrap_or_else(|| format!("{what} {id}"))
}

/// Names by id, for the approval window; an id that was not looked up shows as "project 12".
#[derive(Debug, Default)]
pub(super) struct Namen {
    pub projecten: HashMap<u64, String>,
    pub activiteiten: HashMap<u64, String>,
    pub medewerkers: HashMap<u64, String>,
}

impl Namen {
    pub(super) fn project(&self, id: Option<u64>) -> String {
        Self::named(&self.projecten, id, "project")
    }

    pub(super) fn activiteit(&self, id: Option<u64>) -> String {
        Self::named(&self.activiteiten, id, "activiteit")
    }

    fn medewerker(&self, id: Option<u64>) -> String {
        Self::named(&self.medewerkers, id, "medewerker")
    }

    fn named(map: &HashMap<u64, String>, id: Option<u64>, what: &str) -> String {
        match id {
            None => "(leeg)".to_owned(),
            Some(id) => map
                .get(&id)
                .filter(|n| !n.is_empty())
                .cloned()
                .unwrap_or_else(|| format!("{what} {id}")),
        }
    }
}

// ---------------------------------------------------------------- uren

/// A new hour line, as the hours form sends it (`submitForm` of `app-uren-form`): gebruikerId only when a
/// medewerker is chosen, the amounts that are filled in, the date as an object.
pub(super) fn uur_nieuw(args: &UurToevoegenArgs, namen: &Namen) -> Result<Built, String> {
    let datum = parse_date(&args.datum)?;
    let uren = leeg_bij_nul(
        args.uren
            .map(|u| aantal(u, "Het aantal uren"))
            .transpose()?,
    );
    let km = leeg_bij_nul(
        args.kilometers
            .map(|k| aantal(k, "Het aantal kilometers"))
            .transpose()?,
    );
    if uren.is_none() && km.is_none() {
        return Err("Aantal is niet ingevuld: geef uren of kilometers".into());
    }
    let opmerkingen =
        tekst(args.opmerkingen.as_deref(), "De opmerking", 300_000)?.filter(|o| !o.is_empty());
    let mut body = Map::new();
    let mut lines = vec![line("Datum", nl_datum(datum))];
    if let Some(medewerker) = args.medewerker_id {
        body.insert("gebruikerId".into(), json!(medewerker));
        lines.push(line("Medewerker", namen.medewerker(Some(medewerker))));
    }
    if let Some(uren) = uren {
        body.insert("aantalUren".into(), json!(uren));
    }
    body.insert("datum".into(), datum_object(datum));
    body.insert("activiteitId".into(), json!(args.activiteit_id));
    body.insert("projectId".into(), json!(args.project_id));
    if let Some(opmerkingen) = &opmerkingen {
        body.insert("opmerkingen".into(), json!(opmerkingen));
    }
    if let Some(km) = km {
        body.insert("aantalKilometers".into(), json!(km));
    }
    lines.push(line("Project", namen.project(Some(args.project_id))));
    lines.push(line(
        "Activiteit",
        namen.activiteit(Some(args.activiteit_id)),
    ));
    if let Some(uren) = uren {
        lines.push(line("Uren", getal(uren)));
    }
    if let Some(km) = km {
        lines.push(line("Kilometers", getal(km)));
    }
    if let Some(opmerkingen) = &opmerkingen {
        lines.push(line("Opmerkingen", opmerkingen));
    }
    Ok(Built {
        body: Value::Object(body),
        title: format!(
            "Uren toevoegen: {} op {}",
            namen.project(Some(args.project_id)),
            nl_datum(datum)
        ),
        lines,
    })
}

/// A changed hour line: the record as e-Boekhouden gave it (`GET uur/{id}`) with the changes, in the shape the
/// form sends on save (`PUT uur`, with the id and the medewerker of the line).
pub(super) fn uur_gewijzigd(
    args: &UurWijzigenArgs,
    now: &Value,
    namen: &Namen,
) -> Result<Built, String> {
    if !now.is_object() {
        return Err("e-Boekhouden gaf deze urenregel in een onbekende vorm terug".into());
    }
    let datum_nu = now
        .get("datum")
        .and_then(datum_of)
        .ok_or("e-Boekhouden gaf deze urenregel zonder datum terug")?;
    let datum = args
        .datum
        .as_deref()
        .map(parse_date)
        .transpose()?
        .unwrap_or(datum_nu);
    let number = |key: &str| now.get(key).and_then(Value::as_f64);
    let uren = match args.uren {
        Some(u) => leeg_bij_nul(Some(aantal(u, "Het aantal uren")?)),
        None => number("aantalUren"),
    };
    let km = match args.kilometers {
        Some(k) => leeg_bij_nul(Some(aantal(k, "Het aantal kilometers")?)),
        None => number("aantalKilometers"),
    };
    if uren.unwrap_or(0.0) == 0.0 && km.unwrap_or(0.0) == 0.0 {
        return Err("Aantal is niet ingevuld: een urenregel heeft uren of kilometers".into());
    }
    let project_nu = id_of(now, "projectId");
    let activiteit_nu = id_of(now, "activiteitId");
    let gebruiker_nu = id_of(now, "gebruikerId");
    let project = args.project_id.or(project_nu);
    let activiteit = args.activiteit_id.or(activiteit_nu);
    let gebruiker = args.medewerker_id.or(gebruiker_nu);
    let (Some(project), Some(activiteit)) = (project, activiteit) else {
        return Err("Deze urenregel heeft geen project of activiteit: geef ze mee".into());
    };
    let opmerkingen = match tekst(args.opmerkingen.as_deref(), "De opmerking", 300_000)? {
        Some(text) => Some(json!(text)),
        None => now.get("opmerkingen").cloned(),
    };

    let mut lines = vec![line(
        "Urenregel",
        format!(
            "{}, {}, {}",
            nl_datum(datum_nu),
            namen.project(project_nu),
            namen.activiteit(activiteit_nu)
        ),
    )];
    let mut changed = wijziging(&mut lines, "Datum", nl_datum(datum_nu), nl_datum(datum));
    if args.medewerker_id.is_some() {
        changed |= wijziging(
            &mut lines,
            "Medewerker",
            namen.medewerker(gebruiker_nu),
            namen.medewerker(gebruiker),
        );
    }
    changed |= wijziging(
        &mut lines,
        "Project",
        namen.project(project_nu),
        namen.project(Some(project)),
    );
    changed |= wijziging(
        &mut lines,
        "Activiteit",
        namen.activiteit(activiteit_nu),
        namen.activiteit(Some(activiteit)),
    );
    let amount = |value: Option<f64>| value.map_or("(leeg)".to_owned(), getal);
    changed |= wijziging(
        &mut lines,
        "Uren",
        amount(number("aantalUren")),
        amount(uren),
    );
    changed |= wijziging(
        &mut lines,
        "Kilometers",
        amount(number("aantalKilometers")),
        amount(km),
    );
    changed |= wijziging(
        &mut lines,
        "Opmerkingen",
        shown(now.get("opmerkingen")),
        shown(opmerkingen.as_ref()),
    );
    if !changed {
        return Err("Er is niets gewijzigd: geef aan wat er anders moet".into());
    }

    let mut body = Map::new();
    if let Some(gebruiker) = gebruiker {
        body.insert("gebruikerId".into(), json!(gebruiker));
    }
    if let Some(uren) = uren {
        body.insert("aantalUren".into(), json!(uren));
    }
    body.insert("datum".into(), datum_object(datum));
    body.insert("activiteitId".into(), json!(activiteit));
    body.insert("projectId".into(), json!(project));
    if let Some(opmerkingen) = opmerkingen {
        body.insert("opmerkingen".into(), opmerkingen);
    }
    if let Some(km) = km {
        body.insert("aantalKilometers".into(), json!(km));
    }
    body.insert("id".into(), json!(args.id));
    Ok(Built {
        body: Value::Object(body),
        title: format!("Urenregel van {} wijzigen", nl_datum(datum_nu)),
        lines,
    })
}

/// The lines of a delete of an hour line.
pub(super) fn uur_weg(id: u64, now: &Value, namen: &Namen) -> Built {
    let datum = now
        .get("datum")
        .and_then(datum_of)
        .map_or("onbekend".to_owned(), nl_datum);
    let amount = |key: &str| {
        now.get(key)
            .and_then(Value::as_f64)
            .map_or("(leeg)".to_owned(), getal)
    };
    Built {
        body: Value::Null,
        title: format!("Urenregel van {datum} verwijderen"),
        lines: vec![
            line("Urenregel", format!("id {id}")),
            line("Datum", &datum),
            line("Project", namen.project(id_of(now, "projectId"))),
            line("Activiteit", namen.activiteit(id_of(now, "activiteitId"))),
            line("Uren", amount("aantalUren")),
            line("Kilometers", amount("aantalKilometers")),
            line("Opmerkingen", shown(now.get("opmerkingen"))),
            line(
                "Gevolg",
                "de urenregel verdwijnt; dit is niet terug te draaien",
            ),
        ],
    }
}

// ---------------------------------------------------------------- projecten

/// The ids of a project's users as `GET project/{id}` gives them (`users`: ids, or objects with an id).
pub(super) fn gebruiker_ids(list: Option<&Value>) -> Result<Vec<u64>, String> {
    let items = list
        .and_then(Value::as_array)
        .ok_or("e-Boekhouden gaf de gebruikers van dit project in een onbekende vorm terug")?;
    let ids: Vec<u64> = items
        .iter()
        .filter_map(|item| item.as_u64().or_else(|| id_of(item, "id")))
        .collect();
    // An entry that is no id would otherwise drop out, and the change would take that user off the project.
    if ids.len() < items.len() {
        return Err(
            "e-Boekhouden gaf de gebruikers van dit project in een onbekende vorm terug; er is niets gewijzigd"
                .into(),
        );
    }
    Ok(ids)
}

fn gebruikers_kiezen(ids: &[u64], gebruikers: &[(u64, String)]) -> Result<Vec<u64>, String> {
    let mut chosen: Vec<u64> = Vec::new();
    for id in ids {
        if !gebruikers.iter().any(|(g, _)| g == id) {
            return Err(format!(
                "Geen gebruiker met id {id} in deze administratie. Kies uit: {}",
                gebruikers
                    .iter()
                    .map(|(id, naam)| format!("{id} {naam}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !chosen.contains(id) {
            chosen.push(*id);
        }
    }
    Ok(chosen)
}

fn gebruikers_tekst(ids: &[u64], gebruikers: &[(u64, String)]) -> String {
    if ids.is_empty() {
        return "(geen)".to_owned();
    }
    ids.iter()
        .map(|id| naam_bij(gebruikers, *id, "gebruiker"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A new project, as the project form sends it: `{project: {relatieId, naam, facturabel, actief, omschrijving},
/// users: [ids]}`. A new project starts with every user ticked, as in the form.
pub(super) fn project_nieuw(
    args: &ProjectToevoegenArgs,
    relatie: &str,
    gebruikers: &[(u64, String)],
) -> Result<Built, String> {
    let naam = naam(&args.naam, "De naam van het project", 50)?;
    let omschrijving =
        tekst(args.omschrijving.as_deref(), "De omschrijving", 100_000)?.unwrap_or_default();
    let facturabel = args.facturabel.unwrap_or(false);
    let actief = args.actief.unwrap_or(true);
    let users = match &args.gebruiker_ids {
        Some(ids) => gebruikers_kiezen(ids, gebruikers)?,
        None => gebruikers.iter().map(|(id, _)| *id).collect(),
    };
    let mut lines = vec![
        line("Project", &naam),
        line("Relatie", relatie),
        line("Facturabel", ja_nee(facturabel)),
        line("Actief", ja_nee(actief)),
        line("Gebruikers", gebruikers_tekst(&users, gebruikers)),
    ];
    if !omschrijving.is_empty() {
        lines.insert(2, line("Omschrijving", &omschrijving));
    }
    Ok(Built {
        body: json!({
            "project": {"relatieId": args.relatie_id, "naam": naam, "facturabel": facturabel, "actief": actief, "omschrijving": omschrijving},
            "users": users,
        }),
        title: format!("Project toevoegen: {naam}"),
        lines,
    })
}

/// A changed project: `GET project/{id}` (`{project, users}`) with the changes, in the shape of the form.
pub(super) fn project_gewijzigd(
    args: &ProjectWijzigenArgs,
    now: &Value,
    relaties: &HashMap<u64, String>,
    gebruikers: &[(u64, String)],
) -> Result<Built, String> {
    let project = now
        .get("project")
        .filter(|p| p.is_object())
        .ok_or("e-Boekhouden gaf dit project in een onbekende vorm terug")?;
    let naam_nu = text_of(project, "naam");
    let naam = match &args.naam {
        Some(n) => naam(n, "De naam van het project", 50)?,
        None => naam_nu.clone(),
    };
    let relatie_nu = id_of(project, "relatieId");
    let relatie = args
        .relatie_id
        .or(relatie_nu)
        .ok_or("Dit project heeft geen relatie: geef relatie_id mee")?;
    let omschrijving = match tekst(args.omschrijving.as_deref(), "De omschrijving", 100_000)? {
        Some(text) => json!(text),
        None => project.get("omschrijving").cloned().unwrap_or(json!("")),
    };
    let flag = |key: &str, given: Option<bool>, default: bool| {
        given.unwrap_or_else(|| project.get(key).and_then(Value::as_bool).unwrap_or(default))
    };
    let facturabel = flag("facturabel", args.facturabel, false);
    let actief = flag("actief", args.actief, true);
    let users_nu = gebruiker_ids(now.get("users"))?;
    let users = match &args.gebruiker_ids {
        Some(ids) => gebruikers_kiezen(ids, gebruikers)?,
        None => users_nu.clone(),
    };
    let relatie_naam = |id: Option<u64>| match id {
        None => "(leeg)".to_owned(),
        Some(id) => relaties
            .get(&id)
            .filter(|n| !n.is_empty())
            .cloned()
            .unwrap_or_else(|| format!("relatie {id}")),
    };

    let mut lines = vec![line("Project", &naam_nu)];
    let mut changed = wijziging(&mut lines, "Naam", naam_nu.clone(), naam.clone());
    changed |= wijziging(
        &mut lines,
        "Relatie",
        relatie_naam(relatie_nu),
        relatie_naam(Some(relatie)),
    );
    changed |= wijziging(
        &mut lines,
        "Omschrijving",
        shown(project.get("omschrijving")),
        shown(Some(&omschrijving)),
    );
    changed |= wijziging(
        &mut lines,
        "Facturabel",
        shown(project.get("facturabel")),
        ja_nee(facturabel).to_owned(),
    );
    changed |= wijziging(
        &mut lines,
        "Actief",
        shown(project.get("actief")),
        ja_nee(actief).to_owned(),
    );
    let mut sorted_now = users_nu.clone();
    sorted_now.sort_unstable();
    let mut sorted_new = users.clone();
    sorted_new.sort_unstable();
    if sorted_now != sorted_new {
        changed = true;
        lines.push(line(
            "Gebruikers waren",
            gebruikers_tekst(&users_nu, gebruikers),
        ));
        lines.push(line(
            "Gebruikers worden",
            gebruikers_tekst(&users, gebruikers),
        ));
    }
    if !changed {
        return Err("Er is niets gewijzigd: geef aan wat er anders moet".into());
    }
    Ok(Built {
        body: json!({
            "project": {"relatieId": relatie, "naam": naam, "facturabel": facturabel, "actief": actief, "omschrijving": omschrijving, "id": args.id},
            "users": users,
        }),
        title: format!("Project wijzigen: {naam_nu}"),
        lines,
    })
}

// ---------------------------------------------------------------- activiteiten

/// The value of a setting as `GET setting/{key}?acc=false` gives it (`{key, value}`); `None` when the
/// administration never set it (the answer is null, which the forms read as "not on"). Any other shape is
/// refused rather than guessed.
pub(super) fn setting_waarde(answer: &Value, what: &str) -> Result<Option<String>, String> {
    match answer {
        Value::Null => Ok(None),
        Value::Object(object) => match object.get("value") {
            Some(Value::String(value)) => Ok(Some(value.clone())),
            _ => Err(format!(
                "e-Boekhouden gaf de instelling {what} in een onbekende vorm terug; er is niets gewijzigd"
            )),
        },
        _ => Err(format!(
            "e-Boekhouden gaf de instelling {what} in een onbekende vorm terug; er is niets gewijzigd"
        )),
    }
}

/// What an activity's sales side may point at, read the way the activity form reads it: the omzetrekeningen
/// (`grootboekrekening/search?rekeningCategorie=VW,BAL&actief=true`), the sales btw codes valid today
/// (`btwinfo/info/VERK`, without AFW_VERK) and whether the administration charges btw (the setting
/// `administratie.niet-btw-plichtig`, which the form reads with `getSetting`).
#[derive(Debug, Default)]
pub(super) struct Verkoop {
    /// (id, code, naam)
    pub rekeningen: Vec<(u64, String, String)>,
    /// (code, naam)
    pub btw: Vec<(String, String)>,
    pub btwplichtig: bool,
}

impl Verkoop {
    pub(super) fn from_api(
        rekeningen: &Value,
        btw: &Value,
        niet_btw_plichtig: &Value,
        vandaag: Date,
    ) -> Result<Self, String> {
        // As the form: `E && E.value.toLocaleLowerCase() === "true"` means not btw-plichtig.
        let niet_btw_plichtig = setting_waarde(niet_btw_plichtig, "niet btw-plichtig")?
            .is_some_and(|v| v.to_lowercase() == "true");
        let rekeningen = rekeningen
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|r| {
                        Some((
                            id_of(r, "id")?,
                            text_of(r, "code"),
                            text_of(r, "omschrijving"),
                        ))
                    })
                    .filter(|(_, code, _)| !code.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        // As `Rn.filterInfo(codes, {date: today})`: a code that ended on or before today, or starts on or after
        // today, is not offered.
        let btw = btw
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter(|c| {
                        let einde = c.get("einddatum").and_then(datum_of);
                        let begin = c.get("begindatum").and_then(datum_of);
                        !einde.is_some_and(|e| e <= vandaag) && !begin.is_some_and(|b| b >= vandaag)
                    })
                    .map(|c| (text_of(c, "code"), text_of(c, "omschrijving")))
                    .filter(|(code, _)| !code.is_empty() && code != "AFW_VERK")
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            rekeningen,
            btw,
            btwplichtig: !niet_btw_plichtig,
        })
    }

    fn rekening(&self, code: &str) -> Result<u64, String> {
        let code = code.trim();
        self.rekeningen
            .iter()
            .find(|(_, c, _)| c == code)
            .map(|(id, _, _)| *id)
            .ok_or_else(|| {
                format!(
                    "Geen actieve omzetrekening (winst en verlies of balans) met code {code:?} in deze administratie"
                )
            })
    }

    fn rekening_label(&self, id: Option<u64>) -> String {
        match id {
            None => "(leeg)".to_owned(),
            Some(id) => self
                .rekeningen
                .iter()
                .find(|(i, _, _)| *i == id)
                .map(|(_, code, naam)| format!("{code} {naam}"))
                .unwrap_or_else(|| format!("rekening {id}")),
        }
    }

    fn btw_code(&self, code: &str) -> Result<String, String> {
        let code = code.trim();
        self.btw
            .iter()
            .find(|(c, _)| c.eq_ignore_ascii_case(code))
            .map(|(c, _)| c.clone())
            .ok_or_else(|| {
                format!(
                    "Onbekende btw-code {code:?} voor de verkoop. Kies uit: {}",
                    self.btw
                        .iter()
                        .map(|(c, _)| c.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
    }

    fn btw_label(&self, code: Option<&str>) -> String {
        match code.filter(|c| !c.is_empty()) {
            None => "(leeg)".to_owned(),
            Some(code) => self
                .btw
                .iter()
                .find(|(c, _)| c == code)
                .map(|(c, naam)| format!("{c} ({naam})"))
                .unwrap_or_else(|| code.to_owned()),
        }
    }
}

/// What a facturabele activiteit needs, as the form's validators check it.
fn facturabel_compleet(
    uurtarief: Option<f64>,
    kilometertarief: Option<f64>,
    rekening: Option<u64>,
    btw: Option<&str>,
    btwplichtig: bool,
) -> Result<(), String> {
    if uurtarief.unwrap_or(0.0) == 0.0 && kilometertarief.unwrap_or(0.0) == 0.0 {
        return Err(
            "Uur- of kilometertarief is niet ingevuld: een facturabele activiteit heeft een tarief"
                .into(),
        );
    }
    if rekening.is_none() {
        return Err(
            "Tegenrekening is niet ingevuld: een facturabele activiteit heeft een omzetrekening"
                .into(),
        );
    }
    if btwplichtig && btw.is_none_or(str::is_empty) {
        return Err("Btw-code is niet ingevuld: een facturabele activiteit heeft een btw-code voor de verkoop".into());
    }
    Ok(())
}

/// A new activity, as the activity form sends it. On a new one that is not facturabel the form leaves the btw
/// code and the tegenrekening out; the omzetrekening defaults to 8000, as in the form.
pub(super) fn activiteit_nieuw(
    args: &ActiviteitToevoegenArgs,
    verkoop: &Verkoop,
) -> Result<Built, String> {
    let naam = naam(&args.naam, "De naam van de activiteit", 50)?;
    let omschrijving =
        tekst(args.omschrijving.as_deref(), "De omschrijving", 100_000)?.filter(|o| !o.is_empty());
    let factuur_omschrijving = tekst(
        args.factuuromschrijving.as_deref(),
        "De factuuromschrijving",
        50,
    )?
    .filter(|o| !o.is_empty());
    let uurtarief = args
        .uurtarief
        .map(|t| aantal(t, "Het uurtarief"))
        .transpose()?;
    let kilometertarief = args
        .kilometertarief
        .map(|t| aantal(t, "Het kilometertarief"))
        .transpose()?;
    let facturabel = args.facturabel.unwrap_or(false);
    let actief = args.actief.unwrap_or(true);
    let (rekening, btw) = if facturabel {
        let rekening = match &args.tegenrekening {
            Some(code) => Some(verkoop.rekening(code)?),
            None => verkoop.rekening("8000").ok(),
        };
        let btw = args
            .btw_code
            .as_deref()
            .map(|c| verkoop.btw_code(c))
            .transpose()?;
        facturabel_compleet(
            uurtarief,
            kilometertarief,
            rekening,
            btw.as_deref(),
            verkoop.btwplichtig,
        )?;
        (rekening, btw)
    } else {
        if args.btw_code.is_some() || args.tegenrekening.is_some() {
            return Err(
                "Een btw-code en tegenrekening horen bij een facturabele activiteit: zet facturabel op true of laat ze weg"
                    .into(),
            );
        }
        (None, None)
    };

    let mut body = Map::new();
    body.insert("naam".into(), json!(naam));
    let mut lines = vec![line("Activiteit", &naam)];
    if let Some(o) = &omschrijving {
        body.insert("omschrijving".into(), json!(o));
        lines.push(line("Omschrijving", o));
    }
    if let Some(t) = uurtarief {
        body.insert("uurtarief".into(), json!(t));
        lines.push(line("Uurtarief", write::euro(t)));
    }
    if let Some(t) = kilometertarief {
        body.insert("kilometertarief".into(), json!(t));
        lines.push(line("Kilometertarief", write::euro(t)));
    }
    body.insert("facturabel".into(), json!(facturabel));
    lines.push(line("Facturabel", ja_nee(facturabel)));
    if let Some(o) = &factuur_omschrijving {
        body.insert("factuurOmschrijving".into(), json!(o));
        lines.push(line("Factuuromschrijving", o));
    }
    if let Some(code) = &btw {
        body.insert("btwCode".into(), json!(code));
        lines.push(line("Btw-code", verkoop.btw_label(Some(code))));
    }
    if let Some(id) = rekening {
        body.insert("grootboekRekeningId".into(), json!(id));
        lines.push(line("Tegenrekening", verkoop.rekening_label(Some(id))));
    }
    body.insert("actief".into(), json!(actief));
    lines.push(line("Actief", ja_nee(actief)));
    Ok(Built {
        body: Value::Object(body),
        title: format!("Activiteit toevoegen: {naam}"),
        lines,
    })
}

/// A changed activity: `GET activiteit/{id}` with the changes, in the shape the form sends on save (`PUT
/// activiteit`): the same fields, the kostenplaats it has (read apart, as the form does), and the id.
pub(super) fn activiteit_gewijzigd(
    args: &ActiviteitWijzigenArgs,
    now: &Value,
    kostenplaats: Option<u64>,
    verkoop: &Verkoop,
) -> Result<Built, String> {
    if !now.is_object() {
        return Err("e-Boekhouden gaf deze activiteit in een onbekende vorm terug".into());
    }
    let naam_nu = text_of(now, "naam");
    let naam = match &args.naam {
        Some(n) => naam(n, "De naam van de activiteit", 50)?,
        None => naam_nu.clone(),
    };
    let field = |given: Option<String>, key: &str| match given {
        Some(text) => json!(text),
        None => now.get(key).cloned().unwrap_or(Value::Null),
    };
    let omschrijving = field(
        tekst(args.omschrijving.as_deref(), "De omschrijving", 100_000)?,
        "omschrijving",
    );
    let factuur_omschrijving = field(
        tekst(
            args.factuuromschrijving.as_deref(),
            "De factuuromschrijving",
            50,
        )?,
        "factuurOmschrijving",
    );
    let tarief = |given: Option<f64>, key: &str, what: &str| -> Result<Value, String> {
        Ok(match given {
            Some(t) => json!(aantal(t, what)?),
            None => now.get(key).cloned().unwrap_or(Value::Null),
        })
    };
    let uurtarief = tarief(args.uurtarief, "uurtarief", "Het uurtarief")?;
    let kilometertarief = tarief(
        args.kilometertarief,
        "kilometertarief",
        "Het kilometertarief",
    )?;
    let facturabel = args.facturabel.unwrap_or_else(|| {
        now.get("facturabel")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    });
    let actief = args
        .actief
        .unwrap_or_else(|| now.get("actief").and_then(Value::as_bool).unwrap_or(true));
    let btw_nu = Some(text_of(now, "btwCode")).filter(|c| !c.is_empty());
    let btw = match &args.btw_code {
        Some(code) => Some(verkoop.btw_code(code)?),
        None => btw_nu.clone(),
    };
    let rekening_nu = id_of(now, "grootboekRekeningId");
    let rekening = match &args.tegenrekening {
        Some(code) => Some(verkoop.rekening(code)?),
        None => rekening_nu,
    };
    if facturabel {
        facturabel_compleet(
            uurtarief.as_f64(),
            kilometertarief.as_f64(),
            rekening,
            btw.as_deref(),
            verkoop.btwplichtig,
        )?;
    }

    let mut lines = vec![line("Activiteit", &naam_nu)];
    let mut changed = wijziging(&mut lines, "Naam", naam_nu.clone(), naam.clone());
    changed |= wijziging(
        &mut lines,
        "Omschrijving",
        shown(now.get("omschrijving")),
        shown(Some(&omschrijving)),
    );
    let bedrag = |value: Option<&Value>| {
        value
            .and_then(Value::as_f64)
            .map_or("(leeg)".to_owned(), write::euro)
    };
    changed |= wijziging(
        &mut lines,
        "Uurtarief",
        bedrag(now.get("uurtarief")),
        bedrag(Some(&uurtarief)),
    );
    changed |= wijziging(
        &mut lines,
        "Kilometertarief",
        bedrag(now.get("kilometertarief")),
        bedrag(Some(&kilometertarief)),
    );
    changed |= wijziging(
        &mut lines,
        "Facturabel",
        shown(now.get("facturabel")),
        ja_nee(facturabel).to_owned(),
    );
    changed |= wijziging(
        &mut lines,
        "Factuuromschrijving",
        shown(now.get("factuurOmschrijving")),
        shown(Some(&factuur_omschrijving)),
    );
    changed |= wijziging(
        &mut lines,
        "Btw-code",
        verkoop.btw_label(btw_nu.as_deref()),
        verkoop.btw_label(btw.as_deref()),
    );
    changed |= wijziging(
        &mut lines,
        "Tegenrekening",
        verkoop.rekening_label(rekening_nu),
        verkoop.rekening_label(rekening),
    );
    changed |= wijziging(
        &mut lines,
        "Actief",
        shown(now.get("actief")),
        ja_nee(actief).to_owned(),
    );
    if !changed {
        return Err("Er is niets gewijzigd: geef aan wat er anders moet".into());
    }

    let mut body = Map::new();
    body.insert("naam".into(), json!(naam));
    body.insert("omschrijving".into(), omschrijving);
    body.insert("uurtarief".into(), uurtarief);
    body.insert("kilometertarief".into(), kilometertarief);
    body.insert("facturabel".into(), json!(facturabel));
    body.insert("factuurOmschrijving".into(), factuur_omschrijving);
    if let Some(code) = btw {
        body.insert("btwCode".into(), json!(code));
    }
    if let Some(id) = rekening {
        body.insert("grootboekRekeningId".into(), json!(id));
    }
    body.insert("actief".into(), json!(actief));
    if let Some(id) = kostenplaats {
        body.insert("kostenplaatsId".into(), json!(id));
    }
    body.insert("id".into(), json!(args.id));
    Ok(Built {
        body: Value::Object(body),
        title: format!("Activiteit wijzigen: {naam_nu}"),
        lines,
    })
}

// ---------------------------------------------------------------- factuurinstellingen

/// The factuurinstellingen of the Uren menu changed: `GET factuurSetting` with the changes, sent back whole
/// (`PUT factuurSetting`) with the fields the settings form sends.
pub(super) fn instellingen_gewijzigd(
    args: &FactuurinstellingenWijzigenArgs,
    now: &Value,
    eenheden: &[(u64, String)],
    sjablonen: &[(u64, String)],
) -> Result<Built, String> {
    let empty = json!({});
    let now = if now.is_object() { now } else { &empty };
    let kies =
        |given: Option<u64>, list: &[(u64, String)], what: &str| -> Result<Option<u64>, String> {
            match given {
                Some(id) if list.iter().any(|(i, _)| *i == id) => Ok(Some(id)),
                Some(id) => Err(format!(
                    "Geen {what} met id {id}. Kies uit: {}",
                    list.iter()
                        .map(|(id, naam)| format!("{id} {naam}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
                None => Ok(None),
            }
        };
    let uur_eenheid = kies(args.uur_eenheid_id, eenheden, "eenheid")?;
    let km_eenheid = kies(args.km_eenheid_id, eenheden, "eenheid")?;
    let sjabloon = kies(args.factuursjabloon_id, sjablonen, "actief factuursjabloon")?;
    let regeltekst = match args.regeltekst.as_deref() {
        Some(text) => Some(naam(text, "De opmaak van de factuurregel", 100)?),
        None => None,
    };
    if args.betalingstermijn.is_some_and(|t| t > 99_999) {
        return Err("Betalingstermijn is niet toegestaan: geef 0 tot 99999 dagen".into());
    }
    let pick = |given: Option<Value>, key: &str| {
        given.unwrap_or_else(|| now.get(key).cloned().unwrap_or(Value::Null))
    };
    let new = [
        (
            "uurEenheidId",
            pick(uur_eenheid.map(|v| json!(v)), "uurEenheidId"),
        ),
        (
            "kmEenheidId",
            pick(km_eenheid.map(|v| json!(v)), "kmEenheidId"),
        ),
        (
            "factuurSjabloonId",
            pick(sjabloon.map(|v| json!(v)), "factuurSjabloonId"),
        ),
        (
            "regelTekst",
            pick(regeltekst.map(|v| json!(v)), "regelTekst"),
        ),
        (
            "betalingsTermijn",
            pick(args.betalingstermijn.map(|v| json!(v)), "betalingsTermijn"),
        ),
        ("perPost", pick(args.per_post.map(|v| json!(v)), "perPost")),
        (
            "perEmail",
            pick(args.per_email.map(|v| json!(v)), "perEmail"),
        ),
        (
            "incasseren",
            pick(args.incasseren.map(|v| json!(v)), "incasseren"),
        ),
        (
            "verwerken",
            pick(args.verwerken.map(|v| json!(v)), "verwerken"),
        ),
    ];
    let value = |key: &str| new.iter().find(|(k, _)| *k == key).map(|(_, v)| v);
    for (key, what) in [
        ("uurEenheidId", "De eenheid voor uren"),
        ("kmEenheidId", "De eenheid voor kilometers"),
    ] {
        if value(key).and_then(Value::as_u64).is_none_or(|id| id == 0) {
            return Err(format!("{what} is niet ingevuld"));
        }
    }
    if value("regelTekst")
        .and_then(Value::as_str)
        .is_none_or(|t| t.trim().is_empty())
    {
        return Err("De opmaak van de factuurregel is niet ingevuld".into());
    }

    let named =
        |list: &[(u64, String)], v: Option<&Value>, what: &str| match v.and_then(Value::as_u64) {
            None | Some(0) => "(leeg)".to_owned(),
            Some(id) => naam_bij(list, id, what),
        };
    let labels: [(&str, &str); 9] = [
        ("uurEenheidId", "Eenheid voor uren"),
        ("kmEenheidId", "Eenheid voor kilometers"),
        ("factuurSjabloonId", "Factuursjabloon"),
        ("regelTekst", "Opmaak factuurregel"),
        ("betalingsTermijn", "Betalingstermijn (dagen)"),
        ("perPost", "Klaarzetten voor verzending per post"),
        ("perEmail", "Klaarzetten voor verzending per e-mail"),
        ("incasseren", "Incasseren"),
        ("verwerken", "Verwerken in de boekhouding"),
    ];
    let mut lines = Vec::new();
    for (key, label) in labels {
        let shown_as = |v: Option<&Value>| match key {
            "uurEenheidId" | "kmEenheidId" => named(eenheden, v, "eenheid"),
            "factuurSjabloonId" => named(sjablonen, v, "sjabloon"),
            "perPost" | "perEmail" | "incasseren" | "verwerken" => {
                ja_nee(v.and_then(Value::as_bool).unwrap_or(false)).to_owned()
            }
            _ => shown(v),
        };
        wijziging(
            &mut lines,
            label,
            shown_as(now.get(key)),
            shown_as(value(key)),
        );
    }
    if lines.is_empty() {
        return Err("Er is niets gewijzigd: geef aan wat er anders moet".into());
    }
    lines.push(line(
        "Let op",
        "dit zijn de factuurinstellingen van e-Boekhouden zelf; het factuursjabloon en verwerken gelden ook voor facturen die EB MCP met factuur_toevoegen maakt",
    ));

    let mut body = Map::new();
    if let Some(id) = now.get("id").filter(|v| !v.is_null()) {
        body.insert("id".into(), id.clone());
    }
    for (key, value) in new {
        body.insert(key.into(), value);
    }
    Ok(Built {
        body: Value::Object(body),
        title: "Factuurinstellingen voor uren wijzigen".into(),
        lines,
    })
}
