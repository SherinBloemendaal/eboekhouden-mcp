//! The MCP tools. Each one resolves the account (the only one, or the one
//! the client names), calls e-Boekhouden's API in that session, and logs
//! itself with the calls it caused. The tools that read are marked read-only;
//! the ones that book, save or delete all run through `write_tool`.
//!
//! Query parameter names come from e-Boekhouden's own Angular app (the public
//! bundle): grids take offset, limit, orderBy and sortDirection plus filters.

use std::future::Future;
use std::sync::{Arc, LazyLock, Mutex};

use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CallToolResult, ContentBlock, Icon, Implementation, ResourceContents, ServerCapabilities,
        ServerConfig,
    },
    schemars::{self, JsonSchema},
    service::RequestContext,
    tool, tool_handler, tool_router,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::oneshot;

use crate::accounts::{Account, Kind, Level, Rights};
use crate::approvals::{self, Answer, Ask, Technical, line};
use crate::reqlog::Outcome;
use crate::state::{AppEvent, AppState};
use crate::write::{self, Books, Built};

mod artikelen;
mod btw_aangifte;
mod inrichting;
mod mijn_gegevens;
mod open_posten_acties;
mod rekeningschema;
mod uren_beheer;
mod verkoop;

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

/// What a path segment may not contain: the words that name an action in e-Boekhouden's API, where some
/// GETs change things. The same words as `DENY` in scripts/gen_get_paths.py, which keeps such paths off
/// the map; keep the two in step.
const ACTING: &[&str] = &[
    "annuleer",
    "annuleren",
    "bevestig",
    "undelete",
    "delete",
    "verwijder",
    "reset",
    "wachtwoord",
    "password",
    "uitlog",
    "logout",
    "verstuur",
    "versturen",
    "send",
    "verwerk",
    "switch",
    "changelanguage",
    "refresh",
    "toggle",
    "koppel",
    "ontkoppel",
    "activeer",
    "deactiveer",
    "opzeg",
    "aanmaken",
    "create",
    "insert",
    "update",
    "save",
    "opslaan",
    "copy",
    "kopie",
    "import",
    "upload",
    "test",
    "poc",
    "oauth",
    "token",
    "login",
    "mail",
    "print",
    "export",
    "download",
    "sms",
    "afmeld",
    "aanmeld",
    "archiveer",
    "herstel",
    "genereer",
    "bereken",
    "maak",
    "pay",
    "betaald",
    "overdragen",
    "onboarding",
    "consent",
    "connection-request",
    "verleng",
    "deactivate",
    "reactivate",
    "cancel",
    "close",
    "actualiseer",
    "licentiewijzigen",
    "licentieupgrade",
    "apiinfo",
    "async",
    "chat",
    "contact",
];

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
    // A `{}` in the map stands for an id or a code, never for an action: "voorraad/{}" must not let
    // "voorraad/actualiseer" through. So no segment may name one, wherever it sits.
    if parts
        .iter()
        .any(|p| ACTING.iter().any(|word| p.contains(word)))
    {
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
            // A row's values come in the order of the columns. The `index` and `readerIndex` a column carries are
            // not always filled in (the KvK list has 0 for every column), so the position decides.
            columns
                .iter()
                .enumerate()
                .filter_map(|(position, column)| {
                    Some((position, column.get("name")?.as_str()?.to_owned()))
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
pub struct AkkoordStatusArgs {
    /// Het voorstel_id uit het antwoord van een tool die iets wijzigt. Zonder: alle voorstellen van de laatste 24 uur.
    pub voorstel_id: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AkkoordIntrekkenArgs {
    /// Het voorstel_id van een wijziging die nog op akkoord wacht.
    pub voorstel_id: u64,
    /// Waarom het voorstel wordt ingetrokken, in een korte zin. De gebruiker ziet dit in de geschiedenis.
    pub reden: Option<String>,
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

/// De gegevens van een relatie zoals het formulier van e-Boekhouden ze kent. Alles is optioneel bij wijzigen.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct RelatieVelden {
    /// Bedrijfsnaam, of de naam van een particulier.
    pub bedrijf: Option<String>,
    /// Relatiecode, uniek binnen de administratie (bijvoorbeeld een afkorting van de naam).
    pub code: Option<String>,
    /// true voor een particulier, false voor een bedrijf (standaard bedrijf).
    pub particulier: Option<bool>,
    /// KvK-nummer.
    pub kvk: Option<String>,
    /// Contactpersoon.
    pub contactpersoon: Option<String>,
    /// Aanhef in brieven en e-mails.
    pub aanhef: Option<String>,
    /// Straat en huisnummer.
    pub adres: Option<String>,
    pub postcode: Option<String>,
    pub plaats: Option<String>,
    pub land: Option<String>,
    /// Telefoonnummer.
    pub telefoon: Option<String>,
    /// Mobiel nummer.
    pub mobiel: Option<String>,
    /// Algemeen e-mailadres.
    pub email: Option<String>,
    /// E-mailadres voor facturen.
    pub email_factuur: Option<String>,
    /// E-mailadres voor herinneringen en aanmaningen.
    pub email_herinnering: Option<String>,
    /// Website.
    pub website: Option<String>,
    /// IBAN van de relatie.
    pub iban: Option<String>,
    /// BIC van de bank.
    pub bic: Option<String>,
    /// Btw-nummer.
    pub btw_nr: Option<String>,
    /// Eigen indeling van de relatie (vrije tekst, bijvoorbeeld Klant of Leverancier).
    pub soort: Option<String>,
    /// Notitie.
    pub notitie: Option<String>,
}

impl RelatieVelden {
    /// Each given field as (the API's key, the label for the user, the value, whether it decides where money or
    /// invoices go).
    fn given(&self) -> Result<Vec<(&'static str, &'static str, String, bool)>, String> {
        let mut out = Vec::new();
        let fields: [(&Option<String>, &'static str, &'static str, bool); 19] = [
            (&self.bedrijf, "bedrijf", "Naam", false),
            (&self.code, "code", "Code", false),
            (&self.kvk, "kvk", "KvK-nummer", false),
            (
                &self.contactpersoon,
                "contactpersoon",
                "Contactpersoon",
                false,
            ),
            (&self.aanhef, "aanhef", "Aanhef", false),
            (&self.adres, "adres", "Adres", false),
            (&self.postcode, "postcode", "Postcode", false),
            (&self.plaats, "plaats", "Plaats", false),
            (&self.land, "land", "Land", false),
            (&self.telefoon, "telefoonnummer", "Telefoon", false),
            (&self.mobiel, "mobielnummer", "Mobiel", false),
            (&self.email, "email", "E-mail", true),
            (
                &self.email_factuur,
                "emailFactuur",
                "E-mail voor facturen",
                true,
            ),
            (
                &self.email_herinnering,
                "emailHerinnering",
                "E-mail voor herinneringen",
                true,
            ),
            (&self.website, "site", "Website", false),
            (&self.iban, "iban", "IBAN", true),
            (&self.bic, "bic", "BIC", true),
            (&self.btw_nr, "btwNr", "Btw-nummer", false),
            (&self.soort, "soort", "Soort", false),
        ];
        for (value, key, label, sensitive) in fields {
            if let Some(value) = value {
                let value = value.trim().to_owned();
                if matches!(key, "email" | "emailFactuur" | "emailHerinnering") && !value.is_empty()
                {
                    email(&value, label)?;
                }
                if matches!(key, "bedrijf" | "code") && value.is_empty() {
                    return Err(format!("{label} mag niet leeg zijn"));
                }
                out.push((key, label, value, sensitive));
            }
        }
        if let Some(notitie) = &self.notitie {
            out.push(("notitie", "Notitie", notitie.trim().to_owned(), false));
        }
        Ok(out)
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct RelatieToevoegenArgs {
    #[serde(flatten)]
    pub velden: RelatieVelden,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct RelatieWijzigenArgs {
    /// Id van de relatie (uit relaties).
    pub id: u64,
    #[serde(flatten)]
    pub velden: RelatieVelden,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct KvkZoekenArgs {
    /// Bedrijfsnaam, KvK-nummer, of postcode met huisnummer.
    pub zoek: String,
    /// Aantal resultaten, 1 tot 200 (standaard 20).
    pub limit: Option<u32>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct KvkAdresArgs {
    /// Vestigingsnummer uit kvk_zoeken. Gaat voor het KvK-nummer.
    pub vestigingsnummer: Option<String>,
    /// KvK-nummer uit kvk_zoeken, voor het adres van de eigenaar.
    pub kvk_nr: Option<String>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// De soorten sjablonen naast e-mailsjablonen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SjabloonSoort {
    /// De opmaak van de factuur-pdf.
    Factuur,
    /// De opmaak van de offerte-pdf.
    Offerte,
    /// De tekst van een herinnering of aanmaning per e-mail.
    Aanmaan,
}

impl SjabloonSoort {
    fn path(self) -> &'static str {
        match self {
            Self::Factuur => "factuursjabloon",
            Self::Offerte => "offertesjabloon",
            Self::Aanmaan => "aanmaansjabloon",
        }
    }

    fn naam(self) -> &'static str {
        match self {
            Self::Factuur => "Factuursjabloon",
            Self::Offerte => "Offertesjabloon",
            Self::Aanmaan => "Aanmaansjabloon",
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SjablonenArgs {
    /// Welke sjablonen: factuur, offerte of aanmaan.
    pub soort: SjabloonSoort,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SjabloonArgs {
    /// factuur, offerte of aanmaan.
    pub soort: SjabloonSoort,
    /// Id van het sjabloon (uit sjablonen).
    pub id: u64,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SjabloonToevoegenArgs {
    /// factuur, offerte of aanmaan.
    pub soort: SjabloonSoort,
    /// Naam van het nieuwe sjabloon.
    pub naam: String,
    /// Bij factuur en offerte: id van het sjabloon waarvan de opmaak wordt overgenomen (standaard het eerste actieve). Pas de kopie daarna aan met sjabloon_wijzigen.
    pub kopie_van: Option<u64>,
    /// Bij aanmaan: het onderwerp van de e-mail.
    pub onderwerp: Option<String>,
    /// Bij aanmaan: de tekst van de e-mail als HTML, met codes zoals [NAAM], [FACTNUMMER], [FACTDATUM], [TOTAAL], [VOLDAAN] en [OPENSTAAND].
    pub bericht: Option<String>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SjabloonWijzigenArgs {
    /// factuur, offerte of aanmaan.
    pub soort: SjabloonSoort,
    /// Id van het sjabloon (uit sjablonen).
    pub id: u64,
    /// Nieuwe naam.
    pub naam: Option<String>,
    /// Bij factuur en offerte: het sjabloon op inactief zetten (true) of weer actief (false).
    pub inactief: Option<bool>,
    /// Bij factuur en offerte: de nieuwe opmaak. Geef het hele document `opmaak` zoals de tool sjabloon het teruggaf, met je wijzigingen: elements (elk met id, tag, x, y, width, height, content als HTML met codes), documentDefinition en sjabloon.
    pub opmaak: Option<Value>,
    /// Bij aanmaan: nieuw onderwerp.
    pub onderwerp: Option<String>,
    /// Bij aanmaan: nieuwe tekst als HTML.
    pub bericht: Option<String>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct PdfArgs {
    /// Het id, zoals de lijst het geeft.
    pub id: u64,
    /// Map op deze computer om de pdf in te zetten, als volledig pad (standaard de map Downloads).
    pub map: Option<String>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct BestandArgs {
    /// Id van het bestand in het digitaal archief (uit digitaal_archief).
    pub bestand_id: u64,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FactuurMailenArgs {
    /// Id van de factuur (uit facturen).
    pub id: u64,
    /// E-mailadres van de ontvanger (standaard het factuur- of e-mailadres van de relatie).
    pub aan: Option<String>,
    /// Id van een e-mailsjabloon voor facturen (uit emailsjablonen) voor onderwerp, bericht en afzender. EB MCP haalt de tekst op zoals e-Boekhouden hem voor deze factuur invult, en toont precies die.
    pub sjabloon_id: Option<u64>,
    /// Eigen onderwerp (gaat voor het sjabloon), hoogstens 110 tekens. Gaat zoals het is: zonder codes zoals [FACTNUMMER], schrijf die uit.
    pub onderwerp: Option<String>,
    /// Eigen bericht als HTML (gaat voor het sjabloon). Gaat zoals het is: zonder codes, schrijf die uit.
    pub bericht: Option<String>,
    /// Cc-adres.
    pub cc: Option<String>,
    /// Bcc-adres.
    pub bcc: Option<String>,
    /// Ook het UBL-bestand meesturen (standaard niet).
    pub ubl: Option<bool>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AccountArgs {
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// Waarvoor een e-mailsjabloon te kiezen is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SjabloonVoor {
    Factuur,
    Offerte,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct EmailsjabloonToevoegenArgs {
    /// Naam van het sjabloon.
    pub naam: String,
    /// Onderwerp. Codes zoals [FACTNUMMER] en [FACTKENMERK] vult e-Boekhouden in.
    pub onderwerp: String,
    /// Bericht als HTML. Codes zoals [RELBEDRIJF], [RELCONTACTPERSOON], [RELAANHEF], [RELADRES], [RELPOSTCODE], [RELPLAATS], [DATUM], [FACTNUMMER] en [ABBEDRIJF] vult e-Boekhouden in.
    pub bericht: String,
    /// Waarvoor het sjabloon te kiezen is: factuur, offerte of beide (standaard factuur).
    pub voor: Option<Vec<SjabloonVoor>>,
    /// Naam van de afzender (standaard de naam van de administratie).
    pub van_naam: Option<String>,
    /// E-mailadres van de afzender (standaard dat van de ingelogde gebruiker).
    pub van_email: Option<String>,
    /// Vast cc-adres.
    pub cc: Option<String>,
    /// Vast bcc-adres.
    pub bcc: Option<String>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct EmailsjabloonWijzigenArgs {
    /// Id van het sjabloon (uit emailsjablonen).
    pub id: u64,
    /// Nieuwe naam.
    pub naam: Option<String>,
    /// Nieuw onderwerp.
    pub onderwerp: Option<String>,
    /// Nieuw bericht als HTML.
    pub bericht: Option<String>,
    /// Waarvoor het sjabloon te kiezen is: factuur, offerte of beide.
    pub voor: Option<Vec<SjabloonVoor>>,
    /// Nieuwe naam van de afzender.
    pub van_naam: Option<String>,
    /// Nieuw e-mailadres van de afzender.
    pub van_email: Option<String>,
    /// Nieuw cc-adres.
    pub cc: Option<String>,
    /// Nieuw bcc-adres.
    pub bcc: Option<String>,
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
    tool_router: ToolRouter<Self>,
}

/// One tool call in progress: its log entry, its client and its account.
struct Run {
    id: u64,
    client: String,
    account: Account,
    /// For a tool that changes the books: what its proposal needs and how its MCP call hears that it went up.
    write: Option<WriteRun>,
}

/// The part of a run that only a tool that changes the books has (`write_tool`).
struct WriteRun {
    /// The tool's arguments, for the history.
    args: Value,
    gate: Arc<Gate>,
    /// Tells the MCP call that the proposal went up and waits; taken by `approve`.
    queued: Mutex<Option<oneshot::Sender<u64>>>,
    /// The change's record in the history (approvals.rs), once `approve` made one.
    proposal: Mutex<Option<u64>>,
}

impl Run {
    fn proposal(&self) -> Option<u64> {
        self.write
            .as_ref()
            .and_then(|w| *w.proposal.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

/// Whether a write tool's change can still be stopped together with its MCP call. Before `approve` the task
/// only reads, and a call the client abandons stops it. From `approve` on, the change belongs to the queue or
/// runs, and nothing stops it halfway.
#[derive(Default)]
struct Gate(Mutex<GateState>);

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum GateState {
    #[default]
    Preparing,
    Committed,
    Abandoned,
}

impl Gate {
    fn state(&self) -> std::sync::MutexGuard<'_, GateState> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Passed by `approve` before anything is asked or sent. `false` when the MCP call was abandoned already.
    fn commit(&self) -> bool {
        let mut state = self.state();
        if *state == GateState::Abandoned {
            return false;
        }
        *state = GateState::Committed;
        true
    }

    /// The MCP call ended without the task's answer. `true` when the change was not committed yet, so the
    /// task may be stopped: it has only read.
    fn abandon(&self) -> bool {
        let mut state = self.state();
        if *state == GateState::Preparing {
            *state = GateState::Abandoned;
            return true;
        }
        false
    }

    fn committed(&self) -> bool {
        *self.state() == GateState::Committed
    }
}

/// The answer of a write tool whose MCP call was abandoned before anything was asked or sent.
const ABANDONED: &str =
    "De AI-client heeft de aanroep afgebroken; er is niets gevraagd of gewijzigd.";
/// The answer of a write tool whose task stopped halfway (a panic, or the app shutting down).
const CUT_OFF: &str = "De aanroep is afgebroken voordat hij klaar was. Kijk in e-Boekhouden wat er al is doorgevoerd.";

/// Stops a write tool's task when its MCP call ends before the task gave an answer, as long as the change
/// was not committed (`Gate`). Disarmed once the call has its answer.
struct StopUnanswered {
    gate: Arc<Gate>,
    task: tokio::task::AbortHandle,
    armed: bool,
}

impl Drop for StopUnanswered {
    fn drop(&mut self) {
        if self.armed && self.gate.abandon() {
            self.task.abort();
        }
    }
}

/// Inside a write tool's task: ends its log entry and its record when the task is dropped before
/// `end_write` (stopped by `StopUnanswered`, a panic, or the runtime shutting down).
struct Unfinished<'a> {
    mcp: &'a EbMcp,
    run: &'a Run,
    armed: bool,
}

impl Drop for Unfinished<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let state = &self.mcp.state;
        let waiting = matches!(
            state.log.get(self.run.id),
            Some(crate::reqlog::Entry::Mcp(m)) if m.outcome == Outcome::Waiting
        );
        let committed = self.run.write.as_ref().is_some_and(|w| w.gate.committed());
        if waiting {
            // Still waiting for the user: the history keeps it waiting, and it lapses at the next start.
            state.log.finish_mcp(
                self.run.id,
                Outcome::Refused,
                None,
                Some(crate::reqlog::LAPSED.to_owned()),
            );
        } else if committed {
            state
                .log
                .finish_mcp(self.run.id, Outcome::Error, None, Some(CUT_OFF.to_owned()));
            if let Some(id) = self.run.proposal() {
                state
                    .approvals
                    .finish(id, Err(CUT_OFF.to_owned()), crate::state::now_ms());
                let _ = state.events.send(AppEvent::ApprovalsChanged);
            }
        } else {
            state.log.finish_mcp(
                self.run.id,
                Outcome::Refused,
                None,
                Some(ABANDONED.to_owned()),
            );
        }
        let _ = state.events.send(AppEvent::LogChanged);
    }
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
            write: None,
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
            // An id e-Boekhouden does not know (any more) comes back as an empty answer (204), seen live.
            (true, None) => Err(format!(
                "e-Boekhouden gaf niets terug op {path}: dit bestaat niet (meer), of het id klopt niet"
            )),
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

/// The refusal for an account that is read-only in EB MCP.
fn read_only(label: &str) -> String {
    format!(
        "{label} is alleen-lezen in EB MCP: er is niets gewijzigd. De gebruiker kan schrijven aanzetten in EB MCP onder Accounts, bij het bewerken van het account."
    )
}

/// The refusal for a kind of change the account does not allow, while it allows another.
fn not_allowed(label: &str, kind: Kind) -> String {
    format!(
        "{label} staat {} niet toe in EB MCP: er is niets gewijzigd. De gebruiker kan dit aanzetten in EB MCP onder Accounts, bij het bewerken van het account.",
        kind.name()
    )
}

/// Why a change of this kind is refused, or `None` when the account allows it (with or without approval).
fn refusal(label: &str, rights: Rights, kind: Kind) -> Option<String> {
    if rights.level(kind) != Level::Off {
        None
    } else if rights.read_only() {
        Some(read_only(label))
    } else {
        Some(not_allowed(label, kind))
    }
}

/// A level as `status` names it.
fn level_name(level: Level) -> &'static str {
    match level {
        Level::Off => "uit",
        Level::Ask => "met akkoord",
        Level::Auto => "zonder akkoord",
    }
}

/// What an account allows, per kind of change, as `status` names it.
fn rights_value(rights: Rights) -> Value {
    json!({
        "boeken_en_wijzigen": level_name(rights.change),
        "verwijderen": level_name(rights.delete),
        "versturen": level_name(rights.send),
        "ingrijpend": level_name(rights.heavy),
    })
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
        // A tool is one kind of change, or one of two when a call can reach further than usual. Refused here
        // when the account allows neither; `approve` checks the kind the change turns out to be.
        let rights = run.account.rights;
        let kinds = [Kind::of(tool, false), Kind::of(tool, true)];
        if kinds.iter().all(|&k| rights.level(k) == Level::Off) {
            let message = refusal(&run.account.label, rights, kinds[0]).unwrap_or_default();
            return Err(self.refuse(run, message));
        }
        Ok(run)
    }

    /// Runs a tool that changes the books: `work` reads what it needs, builds the change, passes `approve`
    /// and sends it. It runs as a task of its own, so the MCP call can answer before the change is done:
    ///
    /// - with the task's own answer when it ends without waiting: bad input, a refusal, or a change that
    ///   needed no approval and ran;
    /// - with the "waiting" answer (`waiting_answer`) as soon as `approve` put a proposal up for the user.
    ///   The task lives on, waits for the answer, sends exactly the requests it would have sent, and ends its
    ///   log entry and its record in the history with the outcome.
    ///
    /// When the MCP call ends before either (the client gave up), a task that has only read is stopped.
    async fn write_tool<A, F, Fut>(
        &self,
        context: &RequestContext<RoleServer>,
        tool: &'static str,
        account: Option<&str>,
        args: A,
        done: &'static str,
        work: F,
    ) -> Result<CallToolResult, McpError>
    where
        A: Serialize + Send + 'static,
        F: FnOnce(EbMcp, Arc<Run>, A) -> Fut + Send + 'static,
        Fut: Future<Output = Result<Value, Halt>> + Send + 'static,
    {
        let mut run = match self.begin_write(context, tool, &args, account) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let gate = Arc::new(Gate::default());
        let (queued, mut proposal) = oneshot::channel();
        run.write = Some(WriteRun {
            args: serde_json::to_value(&args).unwrap_or(Value::Null),
            gate: gate.clone(),
            queued: Mutex::new(Some(queued)),
            proposal: Mutex::new(None),
        });
        let run = Arc::new(run);
        let mut task = tokio::spawn({
            let (this, run) = (self.clone(), run.clone());
            async move {
                let mut unfinished = Unfinished {
                    mcp: &this,
                    run: &run,
                    armed: true,
                };
                let result = work(this.clone(), run.clone(), args).await;
                unfinished.armed = false;
                this.end_write(&run, done, result)
            }
        });
        let mut stop = StopUnanswered {
            gate,
            task: task.abort_handle(),
            armed: true,
        };
        let answer = tokio::select! {
            // A change that was answered and ran before this call looked gives its own outcome.
            biased;
            joined = &mut task => match joined {
                Ok(answer) => answer,
                Err(e) => {
                    log::error!("{tool}: the task of the change stopped: {e}");
                    CallToolResult::error(vec![ContentBlock::text(CUT_OFF.to_owned())])
                }
            },
            Ok(id) = &mut proposal => self.waiting_answer(&run, id),
        };
        stop.armed = false;
        Ok(answer)
    }

    /// The answer of a tool whose change waits for the user: not an error, and it says how to follow up.
    fn waiting_answer(&self, run: &Run, id: u64) -> CallToolResult {
        let title = self
            .state
            .approvals
            .get(id)
            .map(|d| d.record.proposal.title)
            .unwrap_or_default();
        let value = json!({
            "account": run.account.label,
            "administratie": run.account.administration,
            "status": "wacht",
            "voorstel_id": id,
            "titel": title,
            "let_op": format!(
                "Deze wijziging wacht op het akkoord van de gebruiker in EB MCP; er is nog niets gewijzigd. \
                 Je hoeft niet te wachten: ga gerust verder met ander werk. Zodra de gebruiker goedkeurt, voert EB MCP \
                 de wijziging zelf uit. Vraag de uitkomst later op met akkoord_status (voorstel_id {id}), en vertel \
                 de gebruiker dat er in EB MCP een wijziging op akkoord wacht."
            ),
        });
        CallToolResult::success(vec![ContentBlock::text(value.to_string())])
    }

    /// The rekeningschema and btw codes of the account's administration.
    /// The unprocessed bank line with this id, read fresh from `import/gridtable`. A line that is not there is
    /// refused: it does not exist, belongs to another administration, or was processed already, and booking
    /// it again would put the same money in the books twice.
    async fn bankregel(&self, run: &Run, id: u64) -> Result<write::Bankregel, String> {
        // The size e-Boekhouden's own page asks for; checked against a real administration on 2026-11-11.
        const PAGE: u64 = 2000;
        let mut offset = 0;
        loop {
            let query = vec![
                ("offset".to_owned(), offset.to_string()),
                ("limit".to_owned(), PAGE.to_string()),
                ("orderBy".to_owned(), String::new()),
                ("sortDirection".to_owned(), String::new()),
            ];
            let page = named_rows(self.fetch(run, "import/gridtable", query).await?);
            if let Some(found) = write::Bankregel::find(&page, id) {
                return found;
            }
            let rows = page
                .get("data")
                .and_then(Value::as_array)
                .map_or(0, Vec::len) as u64;
            let total = page.get("rowCount").and_then(Value::as_u64).unwrap_or(0);
            offset += rows;
            if rows == 0 || offset >= total {
                return Err(format!(
                    "Bankregel {id} staat niet bij de onverwerkte bankregels van deze administratie: het id klopt niet, of de regel is al verwerkt. Er is niets geboekt; vraag de regels op met bankregels."
                ));
            }
        }
    }

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

    /// Puts the change in front of the user and waits for the answer; the MCP call has answered already by
    /// then (`write_tool`). The account sets a level per kind of change (`Kind::of`): `heavier` says this call
    /// reaches further than the tool usually does (crediting, invoicing in bulk, where mail goes), which the
    /// account never lets through without approval. A kind set to "zonder akkoord" skips the wait. Either way
    /// the change gets a record in the history. The account is read again here: the kind may have been turned
    /// off since the call began.
    async fn approve(
        &self,
        run: &Run,
        tool: &str,
        built: &Built,
        heavier: bool,
    ) -> Result<(), Halt> {
        let kind = Kind::of(tool, heavier);
        let danger = kind != Kind::Change;
        let Some(write) = run.write.as_ref() else {
            return Err(Halt::Error(
                "EB MCP kon deze wijziging niet voorleggen; er is niets gewijzigd.".into(),
            ));
        };
        if !write.gate.commit() {
            return Err(Halt::Refused(ABANDONED.into()));
        }
        let rights = match self.state.accounts.get(run.account.id) {
            Some(account) => account.rights,
            None => {
                return Err(Halt::Refused(
                    "Dit account staat niet meer in EB MCP. Er is niets gewijzigd.".into(),
                ));
            }
        };
        if let Some(message) = refusal(&run.account.label, rights, kind) {
            return Err(Halt::Refused(message));
        }
        let ask = Ask {
            account_id: run.account.id,
            account: run.account.label.clone(),
            client: run.client.clone(),
            tool: tool.to_owned(),
            title: built.title.clone(),
            lines: built.lines.clone(),
            danger,
        };
        let technical = Technical {
            args: write.args.clone(),
            body: built.body.clone(),
            log_id: Some(run.id),
        };
        let now = crate::state::now_ms();
        if rights.level(kind) == Level::Auto {
            let id = self.state.approvals.record_auto(ask, technical, now);
            *write.proposal.lock().unwrap_or_else(|e| e.into_inner()) = Some(id);
            self.state.log.link_proposal(run.id, id);
            let _ = self.state.events.send(AppEvent::ApprovalsChanged);
            return Ok(());
        }
        let (proposal, answer) = self.state.approvals.ask(ask, technical, now);
        *write.proposal.lock().unwrap_or_else(|e| e.into_inner()) = Some(proposal.id);
        self.state.log.link_proposal(run.id, proposal.id);
        self.state.log.wait_mcp(run.id);
        // Account removed or this kind of change turned off between the check above and the ask: account_update and
        // account_remove change the store before they end what waits, so one of the two sees it.
        let ended = match self.state.accounts.get(run.account.id) {
            None => self.state.approvals.end(
                proposal.id,
                Answer::Refused(approvals::REFUSED_ACCOUNT_GONE.into()),
                now,
            ),
            Some(account) if account.rights.level(kind) == Level::Off => self.state.approvals.end(
                proposal.id,
                Answer::Refused(approvals::turned_off(account.rights, kind)),
                now,
            ),
            Some(_) => false,
        };
        let _ = self.state.events.send(AppEvent::ApprovalsChanged);
        let _ = self.state.events.send(AppEvent::LogChanged);
        if !ended
            && let Some(queued) = write
                .queued
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
        {
            let _ = queued.send(proposal.id);
        }
        match answer.await {
            Ok(Answer::Approved) => {
                self.state.log.resume_mcp(run.id);
                let _ = self.state.events.send(AppEvent::LogChanged);
                Ok(())
            }
            Ok(Answer::Refused(message) | Answer::Lapsed(message) | Answer::Withdrawn(message)) => {
                Err(Halt::Refused(message))
            }
            Err(_) => Err(Halt::Refused(approvals::LAPSED_NOBODY_WAITS.into())),
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
        self.change_query(run, method, path, &[], body).await
    }

    /// The same with a query string, which a path may not carry.
    async fn change_query(
        &self,
        run: &Run,
        method: &'static str,
        path: &str,
        query: &[(String, String)],
        body: Option<&Value>,
    ) -> Result<Value, Halt> {
        let call = self
            .state
            .api_send(
                run.account.id,
                method,
                path,
                query,
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

    /// A change with a query string where e-Boekhouden answers `false` when it did not happen.
    async fn change_query_done(
        &self,
        run: &Run,
        method: &'static str,
        path: &str,
        query: &[(String, String)],
        body: Option<&Value>,
    ) -> Result<Value, Halt> {
        match self.change_query(run, method, path, query, body).await? {
            Value::Bool(false) => Err(Halt::Error(
                "e-Boekhouden meldt dat dit niet gelukt is".into(),
            )),
            done => Ok(done),
        }
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

    /// Ends a write tool: its log entry and, when the change got that far, its record in the history. The
    /// answer goes to the MCP call when it still waits; a change that waited for approval ends here too.
    fn end_write(&self, run: &Run, done: &str, result: Result<Value, Halt>) -> CallToolResult {
        let proposal = run.proposal();
        let now = crate::state::now_ms();
        let answer = match result {
            Ok(data) => {
                let value = json!({ "account": run.account.label, "administratie": run.account.administration, "gedaan": done, "data": data });
                self.state
                    .log
                    .finish_mcp(run.id, Outcome::Ok, Some(&value), None);
                if let Some(id) = proposal {
                    self.state.approvals.finish(id, Ok(&value), now);
                }
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
                if let Some(id) = proposal {
                    self.state.approvals.finish(id, Err(message.clone()), now);
                }
                CallToolResult::error(vec![ContentBlock::text(message)])
            }
        };
        let _ = self.state.events.send(AppEvent::LogChanged);
        if proposal.is_some() {
            let _ = self.state.events.send(AppEvent::ApprovalsChanged);
        }
        answer
    }

    /// Fetches a pdf from e-Boekhouden and saves it on this computer, next to whatever is there already.
    async fn save_pdf(
        &self,
        run: &Run,
        path: &str,
        map: Option<&str>,
        fallback: &str,
    ) -> Result<(Value, String), String> {
        use base64::Engine;
        let dir = download_dir(map)?;
        let (call, file) = self
            .state
            .api_download(run.account.id, path, Some(run.id), &run.client)
            .await;
        let (bytes, name) = match file {
            Some(file) if call.ok() => file,
            _ => {
                return Err(call
                    .error
                    .unwrap_or_else(|| format!("e-Boekhouden gaf {} op {path}", call.status)));
            }
        };
        if !bytes.starts_with(b"%PDF") {
            return Err("e-Boekhouden gaf geen pdf terug".to_owned());
        }
        let name = name
            .filter(|n| n.to_ascii_lowercase().ends_with(".pdf"))
            .unwrap_or_else(|| fallback.to_owned());
        let target = write_new(&dir, &name, &bytes)
            .map_err(|e| format!("De pdf is niet op te slaan in {}: {e}", dir.display()))?;
        let saved = json!({"pad": target.to_string_lossy(), "bestandsnaam": target.file_name().map(|n| n.to_string_lossy().into_owned()), "bytes": bytes.len()});
        Ok((
            saved,
            base64::engine::general_purpose::STANDARD.encode(&bytes),
        ))
    }

    /// The answer of a pdf tool: where the file was saved, and the pdf itself as an embedded resource, so an AI
    /// client without access to this computer's files can read it too. The log keeps the first part only.
    fn end_pdf(
        &self,
        run: Run,
        source: &str,
        result: Result<(Value, String), String>,
    ) -> CallToolResult {
        let (saved, pdf) = match result {
            Ok(done) => done,
            Err(message) => return self.end(run, source, Err(message)),
        };
        let name = saved
            .get("bestandsnaam")
            .and_then(Value::as_str)
            .unwrap_or("document.pdf")
            .to_owned();
        let mut answer = self.end(run, source, Ok(saved));
        answer.content.push(ContentBlock::resource(
            ResourceContents::BlobResourceContents {
                uri: format!("eb-mcp://pdf/{}", name.replace(' ', "%20")),
                mime_type: Some("application/pdf".to_owned()),
                blob: pdf,
                meta: None,
            },
        ));
        answer
    }

    /// Opens an editor session for a factuur- or offertesjabloon and reads its layout document.
    async fn editor_session(
        &self,
        run: &Run,
        soort: SjabloonSoort,
        id: u64,
    ) -> Result<(String, Value), String> {
        // The editor opens a template in its own language; "NL" when e-Boekhouden does not say.
        let taal = self
            .fetch(run, &format!("{}/{id}/language", soort.path()), Vec::new())
            .await
            .ok()
            .and_then(|t| t.as_str().map(str::to_owned))
            .filter(|t| !t.is_empty() && t.chars().all(|c| c.is_ascii_alphabetic()))
            .unwrap_or_else(|| "NL".to_owned());
        let path = format!("{}/editor/{id}/{taal}", soort.path());
        let (call, hash) = self
            .state
            .api_editor_open(run.account.id, &path, Some(run.id), &run.client)
            .await;
        let hash = match hash {
            Some(hash) if call.error.is_none() => hash,
            _ => {
                return Err(call
                    .error
                    .unwrap_or_else(|| "e-Boekhouden opende de sjabloon-editor niet".into()));
            }
        };
        let session = self
            .state
            .api_editor(
                run.account.id,
                "GET",
                &format!("api/session/{hash}"),
                None,
                Some(run.id),
                &run.client,
            )
            .await;
        let document = session
            .json
            .and_then(|mut j| j.get_mut("sjabloon").map(Value::take))
            .filter(|document| document.get("elements").is_some_and(Value::is_array));
        match (session.error, document) {
            (None, Some(document)) => Ok((hash, document)),
            (error, _) => {
                // The session is open at e-Boekhouden: it is closed again, without saving, before the tool gives up.
                let _ = self.editor_end(run, &hash, Value::Null, false).await;
                Err(error.unwrap_or_else(|| {
                    "De sjabloon-editor gaf de opmaak in een onbekende vorm terug".into()
                }))
            }
        }
    }

    /// Ends an editor session: with `save` the document becomes the template's layout, without it nothing changes.
    async fn editor_end(
        &self,
        run: &Run,
        hash: &str,
        mut document: Value,
        save: bool,
    ) -> Result<(), String> {
        if let Some(object) = document.as_object_mut() {
            // As the editor does when it saves: the document carries the hash of the session it is saved in.
            object.insert("sessionHash".into(), json!(hash));
        }
        let body = json!({"saveChanges": save, "saveData": document});
        let call = self
            .state
            .api_editor(
                run.account.id,
                "POST",
                &format!("api/session/end/{hash}"),
                Some(&body),
                Some(run.id),
                &run.client,
            )
            .await;
        match call.error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// The first row of a list (`gridtable` with named rows, or a plain array) that is not inactive: its id.
    async fn first_active(&self, run: &Run, path: &str) -> Result<Option<u64>, String> {
        let list = self.fetch(run, path, grid(Some(MAX_ROWS), None)?).await?;
        let rows = match named_rows(list) {
            Value::Array(rows) => rows,
            other => other
                .get("data")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        };
        Ok(rows
            .iter()
            .find(|row| {
                !row.get("inactief")
                    .is_some_and(|v| v.as_bool() == Some(true) || v.as_u64() == Some(1))
            })
            .and_then(|row| row.get("id").and_then(Value::as_u64)))
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

    /// The administration's invoice layout templates, read only when a tool names one (`sjabloon_id`).
    async fn factuur_sjablonen(
        &self,
        run: &Run,
        chosen: Option<u64>,
    ) -> Result<Option<Value>, String> {
        match chosen {
            Some(_) => Ok(Some(named_rows(
                self.fetch(
                    run,
                    "factuursjabloon/gridtable",
                    grid(Some(MAX_ROWS), None)?,
                )
                .await?,
            ))),
            None => Ok(None),
        }
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

/// A plausible e-mail address: enough to stop a slip before it reaches e-Boekhouden, which does the real check.
/// A value for the approval window, or "onbekend" when e-Boekhouden did not give it: never a made-up default.
fn known(value: String) -> String {
    if value.trim().is_empty() {
        "onbekend".to_owned()
    } else {
        value
    }
}

/// The first code in square brackets ("[FACTNUMMER]") in a text the caller wrote. Own text goes out as it is: only a
/// template's text is filled in, by `factuur/maildata` or `offerte/getmaildata`, before it is shown and sent.
fn own_text_code(text: &str) -> Option<String> {
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        if let Some(close) = after.find(']') {
            let inner = &after[..close];
            if inner.len() >= 2
                && inner
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            {
                return Some(format!("[{inner}]"));
            }
        }
        rest = after;
    }
    None
}

/// Refuses own subject or message text that holds a code: nobody would fill it in.
fn own_text_without_codes(onderwerp: Option<&str>, bericht: Option<&str>) -> Result<(), String> {
    for (label, text) in [("Het onderwerp", onderwerp), ("Het bericht", bericht)] {
        if let Some(code) = text.and_then(own_text_code) {
            return Err(format!(
                "{label} bevat de code {code}. Eigen tekst gaat zoals hij is de deur uit; e-Boekhouden vult daar geen codes in. Schrijf het uit (bijvoorbeeld het factuurnummer zelf), of laat de eigen tekst weg en kies een e-mailsjabloon met sjabloon_id."
            ));
        }
    }
    Ok(())
}

/// One invoice's e-mail, settled before the user is asked and sent exactly so.
#[derive(Debug, Clone, PartialEq)]
struct FactuurMail {
    aan: String,
    cc: Option<String>,
    bcc: Option<String>,
    van_naam: String,
    van_email: String,
    onderwerp: String,
    bericht: String,
}

/// What the caller gave for an invoice e-mail.
struct EigenMail<'a> {
    onderwerp: &'a Option<String>,
    bericht: &'a Option<String>,
    cc: &'a Option<String>,
    bcc: &'a Option<String>,
}

/// An invoice e-mail as the factuur mail form fills it: the caller's own text first (without codes), else the
/// template's text as e-Boekhouden fills it for this invoice (`GET factuur/maildata/{sjabloon}/{factuur}`:
/// subject, body, ccEmail, bccEmail, vanNaam, vanEmail; the form's `getMailData(+i, this.factuurId)` and
/// `updateFormFromMail`), then the administration's name and the signed-in user.
fn factuur_mail(
    eigen: &EigenMail,
    filled: Option<&Value>,
    aan: &str,
    user: &Value,
    administratie: Option<&str>,
) -> Result<FactuurMail, String> {
    let given = |value: &Option<String>| {
        value
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
    };
    let field = |value: Option<&Value>, name: &str| {
        value
            .and_then(|v| v.get(name))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
    };
    own_text_without_codes(
        given(eigen.onderwerp).as_deref(),
        given(eigen.bericht).as_deref(),
    )?;
    let aan = email(aan, "De ontvanger")?;
    let cc = given(eigen.cc)
        .or_else(|| field(filled, "ccEmail"))
        .map(|v| email(&v, "Het cc-adres"))
        .transpose()?;
    let bcc = given(eigen.bcc)
        .or_else(|| field(filled, "bccEmail"))
        .map(|v| email(&v, "Het bcc-adres"))
        .transpose()?;
    let onderwerp = given(eigen.onderwerp)
        .or_else(|| field(filled, "subject"))
        .ok_or("Geef een onderwerp, of een e-mailsjabloon met sjabloon_id")?;
    if onderwerp.chars().count() > 110 {
        return Err("Het onderwerp mag hoogstens 110 tekens zijn".into());
    }
    let bericht = given(eigen.bericht)
        .or_else(|| field(filled, "body"))
        .ok_or("Geef een bericht, of een e-mailsjabloon met sjabloon_id")?;
    let van_naam = field(filled, "vanNaam")
        .or_else(|| {
            administratie
                .map(str::trim)
                .filter(|a| !a.is_empty())
                .map(str::to_owned)
        })
        .or_else(|| field(Some(user), "naam"))
        .ok_or("De naam van de afzender is niet bekend")?;
    if van_naam.chars().count() > 50 {
        return Err("De naam van de afzender mag hoogstens 50 tekens zijn".into());
    }
    let van_email = field(filled, "vanEmail")
        .or_else(|| field(Some(user), "email"))
        .ok_or("Het e-mailadres van de afzender is niet bekend")?;
    let van_email = email(&van_email, "Het adres van de afzender")?;
    Ok(FactuurMail {
        aan,
        cc,
        bcc,
        van_naam,
        van_email,
        onderwerp,
        bericht,
    })
}

/// The body `POST factuur/mail` takes for one invoice (captured from the web form's code).
fn factuur_mail_body(mail: &FactuurMail, ubl: bool, factuur_id: u64) -> Value {
    json!({
        "vanNaam": mail.van_naam, "vanEmail": mail.van_email, "aanEmail": mail.aan, "ccEmail": mail.cc,
        "bccEmail": mail.bcc, "subject": mail.onderwerp, "body": mail.bericht, "factuurUbl": ubl,
        "rememberMessage": false, "useEmailSjabloon": false, "bijlagen": [], "factuurId": factuur_id
    })
}

/// Whether an invoice e-mail template may be chosen for invoices (`GET emailsjabloonab/forfactuur`).
fn for_invoices(templates: &Value, id: u64) -> Result<(), String> {
    let found = templates.as_array().is_some_and(|all| {
        all.iter()
            .any(|t| t.get("id").and_then(Value::as_u64) == Some(id))
    });
    if found {
        Ok(())
    } else {
        Err(format!(
            "E-mailsjabloon {id} is niet voor facturen te kiezen; zet het aan voor facturen met emailsjabloon_wijzigen of kies een ander"
        ))
    }
}

fn email(value: &str, what: &str) -> Result<String, String> {
    let value = value.trim();
    let ok = value.len() <= 150
        && !value.contains(char::is_whitespace)
        && value.split_once('@').is_some_and(|(name, host)| {
            !name.is_empty() && host.contains('.') && !host.starts_with('.') && !host.ends_with('.')
        });
    if ok {
        Ok(value.to_owned())
    } else {
        Err(format!("{what} is geen geldig e-mailadres"))
    }
}

/// Every address an HTML message points at (`href`, `src`), which plain words do not show: a link's text can say
/// one thing and lead somewhere else, and an image can report that the mail was opened.
fn targets(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut found: Vec<String> = Vec::new();
    for attribute in ["href", "src"] {
        let mut from = 0;
        while let Some(at) = lower[from..].find(attribute) {
            from += at + attribute.len();
            let Some(after) = html[from..].trim_start().strip_prefix('=') else {
                continue;
            };
            let after = after.trim_start();
            let value = match after.chars().next() {
                Some(quote @ ('"' | '\'')) => after[1..].split(quote).next().unwrap_or_default(),
                _ => after
                    .split(|c: char| c.is_whitespace() || c == '>')
                    .next()
                    .unwrap_or_default(),
            };
            let value = value.trim();
            if !value.is_empty() && !found.iter().any(|f| f == value) {
                found.push(value.to_owned());
            }
        }
    }
    (!found.is_empty()).then(|| found.join("\n"))
}

/// An HTML message as plain words, for the approval window: the user reads what will be sent, not markup.
fn plain_text(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    // Line ends in the source are only white space in HTML; the breaks come from the tags.
    for c in html
        .replace(['\n', '\r'], " ")
        .replace("<br", "\n<br")
        .replace("</div>", "\n</div>")
        .replace("</p>", "\n</p>")
        .chars()
    {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">");
    let lines: Vec<&str> = out.lines().map(str::trim).collect();
    let mut text = String::new();
    for line in lines {
        if line.is_empty() && (text.is_empty() || text.ends_with("\n\n")) {
            continue;
        }
        text.push_str(line);
        text.push('\n');
    }
    text.trim().to_owned()
}

/// Saves `bytes` as `dir/name`, or `dir/name (2).pdf` and on while that name is taken. The file is created new:
/// an existing file is never overwritten and a symlink with that name is never followed.
fn write_new(
    dir: &std::path::Path,
    name: &str,
    bytes: &[u8],
) -> std::io::Result<std::path::PathBuf> {
    use std::io::Write;
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) => (stem, format!(".{ext}")),
        None => (name, String::new()),
    };
    for n in 1..10_000u32 {
        let path = if n == 1 {
            dir.join(name)
        } else {
            dir.join(format!("{stem} ({n}){ext}"))
        };
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(bytes)?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("te veel bestanden met deze naam"))
}

/// The folder a tool may save a download in: the one named (a full path to an existing folder) or Downloads.
fn download_dir(named: Option<&str>) -> Result<std::path::PathBuf, String> {
    match named.map(str::trim).filter(|d| !d.is_empty()) {
        Some(dir) => {
            let path = std::path::PathBuf::from(dir);
            if !path.is_absolute() {
                return Err("Geef de map als volledig pad".into());
            }
            if !path.is_dir() {
                return Err(format!("{} is geen bestaande map", path.display()));
            }
            Ok(path)
        }
        None => {
            let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .ok_or("De map Downloads is niet te vinden; geef een map op")?;
            let downloads = std::path::PathBuf::from(home).join("Downloads");
            if downloads.is_dir() {
                Ok(downloads)
            } else {
                Err("De map Downloads is niet te vinden; geef een map op".into())
            }
        }
    }
}

/// Which uses an e-mail template is offered for, as the API's three flags.
fn sjabloon_voor(voor: &[SjabloonVoor]) -> Result<(bool, bool), String> {
    if voor.is_empty() {
        return Err("Geef aan waarvoor het sjabloon is: factuur, offerte of beide".into());
    }
    Ok((
        voor.contains(&SjabloonVoor::Factuur),
        voor.contains(&SjabloonVoor::Offerte),
    ))
}

/// A moment as the AI client reads it: local time with its offset, to the second.
fn local_time(ms: u64) -> Option<String> {
    chrono::DateTime::from_timestamp_millis(i64::try_from(ms).ok()?).map(|t| {
        t.with_timezone(&chrono::Local)
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
    })
}

/// One proposal as `akkoord_status` gives it; `full` adds its lines and, once it ran, its result.
fn proposal_status(detail: &approvals::Detail, full: bool) -> Value {
    let record = &detail.record;
    let mut value = json!({
        "voorstel_id": record.proposal.id,
        "status": record.status.dutch(),
        "titel": record.proposal.title,
        "tool": record.proposal.tool,
        "account": record.proposal.account,
        "client": record.proposal.client,
        "gevraagd_op": local_time(record.proposal.asked_at),
        "beantwoord_op": record.decided_at.and_then(local_time),
        "besluit": record.decision.map(approvals::Decision::dutch),
        "fout": record.error,
    });
    if full {
        value["regels"] = json!(record.proposal.lines);
        value["klaar_op"] = json!(detail.finished_at.and_then(local_time));
        value["resultaat"] = detail.result.clone().unwrap_or(Value::Null);
    }
    value
}

/// What the history shows for a proposal an AI client took back: who did, and why if it said so.
fn withdrawal_reason(client: &str, reason: Option<&str>) -> String {
    const MAX: usize = 300;
    let reason: String = reason
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let reason: String = if reason.chars().count() > MAX {
        reason.chars().take(MAX).chain(['…']).collect()
    } else {
        reason
    };
    if reason.is_empty() {
        format!("Ingetrokken door {client}, zonder reden. Er is niets gewijzigd.")
    } else {
        format!("Ingetrokken door {client}: {reason} Er is niets gewijzigd.")
    }
}

/// What the states of `akkoord_status` mean, for the AI client.
const STATUS_WORDS: &str = "wacht: de gebruiker heeft nog niet geantwoord; bezig: goedgekeurd en de wijziging loopt; uitgevoerd: gedaan, resultaat geeft het antwoord van de tool; mislukt: goedgekeurd maar e-Boekhouden of EB MCP gaf een fout, zie fout; geweigerd: de gebruiker weigerde, of het account ging uit of werd alleen-lezen, er is niets gewijzigd; vervallen: EB MCP startte opnieuw of de inloggegevens van het account veranderden voor er een antwoord was, er is niets gewijzigd. ingetrokken: een AI-client trok het voorstel zelf in met akkoord_intrekken voordat de gebruiker antwoordde, er is niets gewijzigd.";

/// How many tools the server offers, counted on its tool router.
pub fn tool_count() -> usize {
    EbMcp::routers().list_all().len()
}

/// The tools of the parts that live in their own module (tools/*.rs): the ones that read, the ones that
/// change the books, and of those the ones that cannot be undone.
#[cfg(test)]
pub fn part_tools() -> [Vec<&'static str>; 3] {
    let parts: [[&[&str]; 3]; 8] = [
        [
            mijn_gegevens::READS,
            mijn_gegevens::WRITES,
            mijn_gegevens::DESTRUCTIVE,
        ],
        [
            uren_beheer::READS,
            uren_beheer::WRITES,
            uren_beheer::DESTRUCTIVE,
        ],
        [artikelen::READS, artikelen::WRITES, artikelen::DESTRUCTIVE],
        [
            rekeningschema::READS,
            rekeningschema::WRITES,
            rekeningschema::DESTRUCTIVE,
        ],
        [
            open_posten_acties::READS,
            open_posten_acties::WRITES,
            open_posten_acties::DESTRUCTIVE,
        ],
        [
            inrichting::READS,
            inrichting::WRITES,
            inrichting::DESTRUCTIVE,
        ],
        [
            btw_aangifte::READS,
            btw_aangifte::WRITES,
            btw_aangifte::DESTRUCTIVE,
        ],
        [verkoop::READS, verkoop::WRITES, verkoop::DESTRUCTIVE],
    ];
    [0, 1, 2].map(|kind| {
        parts
            .iter()
            .flat_map(|part| part[kind].iter().copied())
            .collect()
    })
}

impl EbMcp {
    /// The router of tools.rs itself plus the one of every part.
    fn routers() -> ToolRouter<Self> {
        let mut all = Self::tool_router()
            + Self::mijn_gegevens_router()
            + Self::uren_beheer_router()
            + Self::artikelen_router()
            + Self::rekeningschema_router()
            + Self::open_posten_acties_router()
            + Self::inrichting_router()
            + Self::btw_aangifte_router()
            + Self::verkoop_router();
        // A field that may be left out names one plain type, so a client sends a code as text (lenient.rs).
        for route in all.map.values_mut() {
            let mut schema = Value::Object((*route.attr.input_schema).clone());
            crate::lenient::plain_optional_types(&mut schema);
            if let Value::Object(schema) = schema {
                route.attr.input_schema = Arc::new(schema);
            }
        }
        all
    }
}

#[tool_router]
impl EbMcp {
    pub fn new(state: Arc<AppState>) -> Self {
        Self {
            state,
            tool_router: Self::routers(),
        }
    }

    #[tool(
        description = "Welke e-Boekhouden-accounts er in EB MCP staan, welke is ingelogd en wat er per account mag (rechten: boeken en wijzigen, verwijderen, versturen en ingrijpend, elk uit, met akkoord of zonder akkoord). Gebruik dit eerst als er meer dan één account kan zijn, of als een andere tool zegt dat er niemand is ingelogd.",
        annotations(title = "Status", read_only_hint = true, open_world_hint = false)
    )]
    async fn status(&self) -> Result<CallToolResult, McpError> {
        let sessions = self.state.sessions();
        let can_write = self
            .state
            .accounts
            .list()
            .iter()
            .any(|a| !a.rights.read_only());
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
                    "rechten": rights_value(a.rights),
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
                "Per account staat bij rechten wat er mag, per soort wijziging: boeken_en_wijzigen (toevoegen en wijzigen), verwijderen, versturen (mail, Peppol, het incassobestand, een bericht of opdracht aan de accountant) en ingrijpend (crediteren, in bulk factureren of boeken, afboeken, de beginbalans, bedrijfsgegevens, een btw-aangifte verwerken). Elk staat op uit (de tool weigert), met akkoord (de gebruiker keurt elke wijziging goed in EB MCP) of zonder akkoord; versturen en ingrijpend gaan nooit zonder akkoord. Een wijziging die op akkoord wacht geeft meteen antwoord met een voorstel_id; akkoord_status geeft later de uitkomst."
            } else {
                "Alleen lezen: schrijven staat voor elk account uit. De gebruiker kan het per account aanzetten in EB MCP onder Accounts."
            },
        });
        Ok(CallToolResult::success(vec![ContentBlock::text(
            value.to_string(),
        )]))
    }

    #[tool(
        description = "Hoe het staat met wijzigingen die op het akkoord van de gebruiker in EB MCP wachten of wachtten. Met voorstel_id (uit het antwoord van een tool die iets wijzigt): dat ene voorstel met zijn status, wat er gevraagd is en, zodra het is uitgevoerd, het resultaat of de fout. Zonder voorstel_id: alle voorstellen van de laatste 24 uur met hun status, ook wijzigingen die zonder akkoord zijn doorgevoerd. Statussen: wacht, bezig, uitgevoerd, mislukt, geweigerd, vervallen. Leest alleen; vraag niet telkens opnieuw als de status wacht is, de gebruiker beslist in zijn eigen tempo.",
        annotations(
            title = "Akkoord status",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    async fn akkoord_status(
        &self,
        Parameters(args): Parameters<AkkoordStatusArgs>,
    ) -> Result<CallToolResult, McpError> {
        let value = match args.voorstel_id {
            Some(id) => match self.state.approvals.get(id) {
                Some(detail) => {
                    let mut value = proposal_status(&detail, true);
                    value["uitleg_status"] = json!(STATUS_WORDS);
                    value
                }
                None => {
                    return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                        "Er is geen voorstel met id {id} in EB MCP. akkoord_status zonder voorstel_id geeft de voorstellen van de laatste 24 uur."
                    ))]));
                }
            },
            None => {
                let since = crate::state::now_ms().saturating_sub(24 * 60 * 60 * 1000);
                let list: Vec<Value> = self
                    .state
                    .approvals
                    .since(since)
                    .iter()
                    .map(|detail| proposal_status(detail, false))
                    .collect();
                json!({
                    "voorstellen": list,
                    "wachten": self.state.approvals.count(),
                    "uitleg_status": STATUS_WORDS,
                    "let_op": "Nieuwste eerst. Geef voorstel_id mee voor de regels en het resultaat van één voorstel.",
                })
            }
        };
        Ok(CallToolResult::success(vec![ContentBlock::text(
            value.to_string(),
        )]))
    }

    #[tool(
        description = "Een wijziging intrekken die nog op het akkoord van de gebruiker wacht: het voorstel verdwijnt uit de wachtrij in EB MCP en er gaat niets naar e-Boekhouden. Gebruik dit als een voorstel niet klopt (verkeerd bedrag, verkeerde rekening, dubbel gevraagd) of niet meer nodig is, in plaats van de gebruiker te vragen het te weigeren; dien daarna zo nodig een verbeterd voorstel in. Kan alleen zolang de status wacht is: een goedgekeurd voorstel loopt al of is al uitgevoerd en is niet meer in te trekken. Geef een korte reden mee; de gebruiker ziet in de geschiedenis dat het is ingetrokken, door wie en waarom.",
        annotations(
            title = "Akkoord intrekken",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn akkoord_intrekken(
        &self,
        Parameters(args): Parameters<AkkoordIntrekkenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let id = args.voorstel_id;
        let reason = withdrawal_reason(&client_name(&context), args.reden.as_deref());
        let refused = |text: String| Ok(CallToolResult::error(vec![ContentBlock::text(text)]));
        match self
            .state
            .approvals
            .withdraw(id, reason, crate::state::now_ms())
        {
            Ok(()) => {
                let _ = self.state.events.send(AppEvent::ApprovalsChanged);
                let value = json!({
                    "voorstel_id": id,
                    "status": approvals::Status::Withdrawn.dutch(),
                    "let_op": "Het voorstel is ingetrokken en uit de wachtrij; er is niets gewijzigd in e-Boekhouden. De tool die het vroeg, eindigt als niet uitgevoerd.",
                });
                Ok(CallToolResult::success(vec![ContentBlock::text(
                    value.to_string(),
                )]))
            }
            Err(approvals::NotWithdrawn::Unknown) => refused(format!(
                "Er is geen voorstel met id {id} in EB MCP. akkoord_status zonder voorstel_id geeft de voorstellen van de laatste 24 uur."
            )),
            Err(approvals::NotWithdrawn::NotWaiting(status)) => refused(format!(
                "Voorstel {id} wacht niet meer op akkoord (status: {}), dus het is niet in te trekken. {}",
                status.dutch(),
                match status {
                    approvals::Status::Running | approvals::Status::Done =>
                        "De gebruiker heeft het goedgekeurd; is de wijziging fout, herstel haar dan met een nieuwe wijziging.",
                    _ => "Er is niets gewijzigd in e-Boekhouden.",
                }
            )),
        }
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
        description = "Geïmporteerde bankregels (afschriftregels) die nog niet verwerkt zijn tot een mutatie. Per regel: id, datum (van de import), rekening (IBAN van de eigen bankrekening), mutDatum (boekdatum), mutBedrag (negatief is af), mutOmschrijving (met naam en IBAN van de tegenpartij), mutFactuur, grootboekId, opmerking, hasFiles en verwerkFailureReason. Te filteren op boekdatum en omschrijving. Alleen lezen. Eén regel verwerken kan met mutatie_toevoegen (bankregel_id); alle regels opnieuw door de automatische importregels halen kan met importregels_verwerken.",
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
        description = "Een mutatie boeken, zoals op de schermen Invoeren > Facturen, Afschrift/bonnetjes en Memoriaal: factuur ontvangen of verstuurd, factuurbetaling ontvangen of verstuurd, geld ontvangen of uitgegeven, of memoriaal. Met bankregel_id wordt een onverwerkte bankregel daarmee verwerkt, bij elke soort, ook een factuurbetaling: geef de bankrekening, de datum en het bedrag van de regel, zodat hij niet nog eens geboekt hoeft te worden. Het voorstel toont de bankregel en zegt het als rekening, richting of totaal ervan afwijken. Zonder bankregel_id blijft de bankregel open staan. Rekeningen gaan op code (zie grootboek), btw op code (GEEN, HOOG_VERK_21, HOOG_INK_21, LAAG_VERK_9, ...); e-Boekhouden rekent de btw zelf uit. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de boeking eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
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
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Mutatie toegevoegd; data.mutNr is het mutatienummer",
            move |this, run, args| async move {
                let mut books = this.books(&run).await?;
                books.kostenplaatsen = this
                    .mutatie_kostenplaatsen(
                        &run,
                        write::names_kostenplaats(args.regels.as_deref()),
                        false,
                    )
                    .await?;
                let bankregel = match args.bankregel_id {
                    Some(id) => Some(this.bankregel(&run, id).await?),
                    None => None,
                };
                let built = write::mutatie_nieuw(&args, &books, bankregel.as_ref())?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change(&run, "POST", "mutatie", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een bestaande mutatie wijzigen, op mutatienummer. Wat je niet noemt blijft zoals het is; regels of betalingen die je meegeeft vervangen alle bestaande. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de wijziging eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
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
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Mutatie gewijzigd",
            move |this, run, args| async move {
                let model = this
                    .fetch(&run, &format!("mutatie/nr/{}/model", args.nr), Vec::new())
                    .await?;
                let mut books = this.books(&run).await?;
                books.kostenplaatsen = this
                    .mutatie_kostenplaatsen(
                        &run,
                        write::names_kostenplaats(args.regels.as_deref()),
                        write::model_has_kostenplaats(&model),
                    )
                    .await?;
                let built = write::mutatie_gewijzigd(&args, &model, &books)?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change(&run, "PUT", "mutatie", Some(&built.body)).await
            },
        )
        .await
    }

    #[tool(
        description = "Een bestand van deze computer in het digitaal archief zetten, en het naar wens meteen aan een mutatie koppelen (mutatie_nr). Geef het volledige pad; hoogstens 5 MB. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt het eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
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
        let account = args.account.clone();
        self.write_tool(&context, TOOL, account.as_deref(), args, "Bestand geüpload", move |this, run, args| async move {
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
                this.fetch(&run, &format!("mutatie/nr/{nr}/model"), Vec::new()).await?;
                lines.push(line("Koppelen aan", format!("mutatie {nr}")));
            }
            let built = Built { body: Value::Null, title: format!("Bestand uploaden: {name}"), lines };
            this.approve(&run, TOOL, &built, false).await?;
            let body = json!({"fileName": name, "data": data, "overwrite": args.overschrijven.unwrap_or(false), "folderId": args.map_id.unwrap_or(0)});
            let uploaded = this.change(&run, "POST", "folder/upload", Some(&body)).await?;
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
                match this.change_done(&run, "POST", "folderkoppel/koppel/item", Some(&link)).await {
                    Ok(_) => done["gekoppeld_aan_mutatie"] = json!(nr),
                    Err(Halt::Error(message) | Halt::Refused(message)) => {
                        return Err(Halt::Error(format!(
                            "Het bestand staat in het archief (id {id}), maar koppelen aan mutatie {nr} lukte niet: {message}"
                        )));
                    }
                }
            }
            Ok(done)
        })
        .await
    }

    #[tool(
        description = "Een bestand dat al in het digitaal archief staat aan een mutatie koppelen. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt het eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
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
        let account = args.account.clone();
        self.write_tool(&context, TOOL, account.as_deref(), args, "Bestand gekoppeld aan de mutatie", move |this, run, args| async move {
            let file = this.fetch(&run, &format!("folder/{}", args.bestand_id), Vec::new()).await?;
            let name = file.get("naam").and_then(Value::as_str).unwrap_or("bestand").to_owned();
            if file.get("soort").and_then(Value::as_str) != Some("F") {
                return Err(Halt::Error(format!("{name} is een map, geen bestand")));
            }
            this.fetch(&run, &format!("mutatie/nr/{}/model", args.mutatie_nr), Vec::new()).await?;
            let built = Built {
                body: json!({"koppelId": args.mutatie_nr, "folders": [{"id": args.bestand_id, "soort": "F"}], "koppelType": "MUT"}),
                title: format!("Bestand koppelen: {name}"),
                lines: vec![line("Bestand", &name), line("Koppelen aan", format!("mutatie {}", args.mutatie_nr))],
            };
            this.approve(&run, TOOL, &built, false).await?;
            this.change_done(&run, "POST", "folderkoppel/koppel/item", Some(&built.body)).await
        })
        .await
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
        description = "Een verkoopfactuur maken. Prijzen per stuk, incl of excl btw; btw op code (HOOG_VERK_21, LAAG_VERK_9, GEEN, ...), de omzetrekening op code. Zonder nummer krijgt de factuur het volgende nummer van e-Boekhouden. De factuur wordt niet verstuurd. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de factuur eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
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
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Factuur toegevoegd; data.id is het id en data.nummer het factuurnummer",
            move |this, run, args| async move {
                let books = this.books(&run).await?;
                let factuur_context = this.factuur_context(&run).await?;
                let sjablonen = this.factuur_sjablonen(&run, args.sjabloon_id).await?;
                let built =
                    write::factuur_nieuw(&args, &factuur_context, &books, sjablonen.as_ref())?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change(&run, "POST", "factuur/model", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een verkoopfactuur wijzigen, op id. Wat je niet noemt blijft zoals het is; regels die je meegeeft vervangen alle bestaande. De mutatie van een verwerkte factuur volgt vanzelf. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de wijziging eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
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
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Factuur gewijzigd",
            move |this, run, args| async move {
                let (now, items) = this.with_items(&run, "factuur", args.id).await?;
                let books = this.books(&run).await?;
                let sjablonen = this.factuur_sjablonen(&run, args.sjabloon_id).await?;
                let built =
                    write::factuur_gewijzigd(&args, &now, &items, &books, sjablonen.as_ref())?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change_done(&run, "PUT", "factuur/model", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een verkoopfactuur verwijderen, op id. De mutatie van een verwerkte factuur verdwijnt mee. Dit is niet terug te draaien. De gebruiker keurt dit goed in EB MCP, tenzij verwijderen voor het account zonder akkoord staat. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
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
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Factuur verwijderd",
            move |this, run, args| async move {
                let now = this
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
                this.approve(&run, TOOL, &built, true).await?;
                this.change_done(
                    &run,
                    "DELETE",
                    &format!("factuur/normaal/{}", args.id),
                    None,
                )
                .await
            },
        )
        .await
    }

    #[tool(
        description = "Een verkoopfactuur crediteren, op id: er komt een nieuwe factuur met dezelfde regels in negatieve aantallen, zoals Factuur crediteren in e-Boekhouden doet. De gebruiker moet dit altijd goedkeuren in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
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
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Creditfactuur gemaakt; data.id is het id en data.nummer het factuurnummer",
            move |this, run, args| async move {
                let (now, items) = this.with_items(&run, "factuur", args.id).await?;
                let books = this.books(&run).await?;
                let factuur_context = this.factuur_context(&run).await?;
                let built = write::factuur_credit(&args, &now, &items, &factuur_context, &books)?;
                this.approve(&run, TOOL, &built, true).await?;
                this.change(&run, "POST", "factuur/model", Some(&built.body))
                    .await
            },
        )
        .await
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
        description = "Een abonnement maken: een terugkerende factuur voor een relatie. Prijzen per stuk, incl of excl btw; btw en omzetrekening op code. Er wordt nog geen factuur gemaakt; dat doet abonnementen_factureren. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt het eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
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
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Abonnement toegevoegd; data.id is het id",
            move |this, run, args| async move {
                let books = this.books(&run).await?;
                let factuur_context = this.factuur_context(&run).await?;
                let built =
                    write::abonnement_nieuw(&args, factuur_context.sjabloon_id, today(), &books)?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change(&run, "POST", "abonnement/model", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een abonnement wijzigen, op id. Wat je niet noemt blijft zoals het is; regels die je meegeeft vervangen alle bestaande. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de wijziging eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
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
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Abonnement gewijzigd",
            move |this, run, args| async move {
                let (now, items) = this.with_items(&run, "abonnement", args.id).await?;
                let extra = this
                    .fetch(&run, &format!("factuurextra/{}", args.id), Vec::new())
                    .await?;
                let books = this.books(&run).await?;
                let built = write::abonnement_gewijzigd(&args, &now, &extra, &items, &books)?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change_done(&run, "PUT", "abonnement/model", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een abonnement verwijderen, op id. Facturen die er al van gemaakt zijn blijven staan. Dit is niet terug te draaien. De gebruiker keurt dit goed in EB MCP, tenzij verwijderen voor het account zonder akkoord staat. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
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
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Abonnement verwijderd",
            move |this, run, args| async move {
                let now = this
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
                this.approve(&run, TOOL, &built, true).await?;
                this.change_done(&run, "DELETE", &format!("abonnement/{}", args.id), None)
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Facturen maken voor abonnementen, zoals Facturen maken in e-Boekhouden: voor ALLE abonnementen waarvan de prolongatiedatum in het gekozen venster valt (standaard: al verlopen) komt er een factuur, en hun prolongatiedatum schuift een periode op. Eén abonnement kiezen kan niet. De gebruiker moet dit altijd goedkeuren in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
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
        let account = args.account.clone();
        self.write_tool(&context, TOOL, account.as_deref(), args, "Facturen gemaakt; data.createdIds zijn de ids van de nieuwe facturen en data.errorMessages wat niet lukte", move |this, run, args| async move {
            let counts = this
                .fetch(&run, "factuur/expiredabonnementencount", Vec::new())
                .await?;
            let venster = args.venster.unwrap_or(write::Venster::Verlopen);
            let aantal = counts
                .get(venster.count_key())
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let books = this.books(&run).await?;
            let built = write::abonnementen_factureren(&args, aantal, today(), &books)?;
            this.approve(&run, TOOL, &built, true).await?;
            let call = this
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
        })
        .await
    }

    #[tool(
        description = "Een mutatie verwijderen, op mutatienummer. Boekhoudkundig wordt dit afgeraden: corrigeer liever met een tegenboeking of wijzig de mutatie, zodat zichtbaar blijft wat er is gebeurd. Dit is niet terug te draaien. De gebruiker keurt dit goed in EB MCP, tenzij verwijderen voor het account zonder akkoord staat. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Mutatie verwijderen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn mutatie_verwijderen(
        &self,
        Parameters(args): Parameters<MutatieArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "mutatie_verwijderen";
        let account = args.account.clone();
        self.write_tool(&context, TOOL, account.as_deref(), args, "Mutatie verwijderd", move |this, run, args| async move {
            let model = this.fetch(&run, &format!("mutatie/nr/{}/model", args.nr), Vec::new()).await?;
            let mutatie = model.get("mutatie").cloned().unwrap_or(Value::Null);
            let datum = mutatie
                .get("datum")
                .and_then(Value::as_str)
                .and_then(|d| d.get(..10))
                .ok_or_else(|| Halt::Error("e-Boekhouden gaf deze mutatie zonder datum terug".into()))?
                .to_owned();
            // Deleting goes by the mutatie's id, which changes with every edit and is only in the list: the
            // list of that one day is searched for the number.
            let mut id = None;
            let mut shown = Value::Null;
            for page in 0..10u32 {
                let mut query = grid(Some(MAX_ROWS), Some(page * MAX_ROWS)).map_err(Halt::Error)?;
                query.push(("datumVan".into(), datum.clone()));
                query.push(("datumTot".into(), datum.clone()));
                let rows = named_rows(this.fetch(&run, "mutatie/gridtable", query).await?);
                let rows = rows.get("data").and_then(Value::as_array).cloned().unwrap_or_default();
                if let Some(row) = rows.iter().find(|r| r.get("mutatieNr").and_then(Value::as_u64) == Some(args.nr)) {
                    id = row.get("id").and_then(Value::as_u64);
                    shown = row.clone();
                    break;
                }
                if rows.len() < MAX_ROWS as usize {
                    break;
                }
            }
            let id = id.ok_or_else(|| Halt::Error(format!("Mutatie {} staat niet in de lijst van {datum}", args.nr)))?;
            let text = |name: &str| shown.get(name).and_then(Value::as_str).unwrap_or_default().to_owned();
            let built = Built {
                body: json!([id]),
                title: format!("Mutatie {} verwijderen", args.nr),
                lines: vec![
                    line("Mutatie", format!("nr {}", args.nr)),
                    line("Soort", known(text("soort"))),
                    line("Datum", &datum),
                    line("Bedrag", shown.get("bedrag").and_then(Value::as_f64).map_or("onbekend".to_owned(), write::euro)),
                    line("Rekening", format!("{} tegen {}", text("rekening"), text("tegenrekening"))),
                    line("Omschrijving", mutatie.get("omschrijving").and_then(Value::as_str).unwrap_or_default()),
                    line("Let op", "boekhoudkundig wordt verwijderen afgeraden: een tegenboeking laat zien wat er is gebeurd. Dit is niet terug te draaien."),
                ],
            };
            this.approve(&run, TOOL, &built, true).await?;
            let done = this.change(&run, "POST", "mutatie/bulk/delete", Some(&built.body)).await?;
            let removed = done
                .get("removedIds")
                .and_then(Value::as_array)
                .is_some_and(|ids| ids.iter().any(|v| v.as_u64() == Some(id)));
            if !removed {
                let why = done
                    .get("errorMessages")
                    .and_then(Value::as_array)
                    .and_then(|m| m.first())
                    .map(|m| m.as_str().map(str::to_owned).unwrap_or_else(|| m.to_string()))
                    .unwrap_or_else(|| "e-Boekhouden heeft de mutatie niet verwijderd".into());
                return Err(Halt::Error(format!("e-Boekhouden weigert dit: {why}")));
            }
            Ok(done)
        })
        .await
    }

    #[tool(
        description = "Een bestand uit het digitaal archief verwijderen; het gaat naar de prullenbak van het archief. Is het bestand nog aan een mutatie of iets anders gekoppeld, dan staat er een waarschuwing bij. Of de gebruiker het eerst goedkeurt, volgt uit wat het account voor verwijderen toestaat. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Bestand verwijderen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn archief_verwijderen(
        &self,
        Parameters(args): Parameters<BestandArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "archief_verwijderen";
        let account = args.account.clone();
        self.write_tool(&context, TOOL, account.as_deref(), args, "Bestand verwijderd (staat in de prullenbak van het archief)", move |this, run, args| async move {
            let file = this
                .fetch(&run, &format!("folder/{}", args.bestand_id), Vec::new())
                .await?;
            let name = file
                .get("naam")
                .and_then(Value::as_str)
                .unwrap_or("bestand")
                .to_owned();
            if file.get("soort").and_then(Value::as_str) != Some("F") {
                return Err(Halt::Error(format!("{name} is een map, geen bestand")));
            }
            if file.get("isDeleted").and_then(Value::as_bool) == Some(true) {
                return Err(Halt::Error(format!(
                    "{name} staat al in de prullenbak van het archief; EB MCP verwijdert het daar niet definitief"
                )));
            }
            let links = this
                .fetch(&run, &format!("folder/{}/koppel", args.bestand_id), Vec::new())
                .await?;
            // Anything that is not plainly "no link" counts as a link: an unknown shape must ask, never slip through.
            let linked_to = |value: &Value| -> Option<usize> {
                match value {
                    Value::Null | Value::Bool(false) => None,
                    Value::Array(items) => (!items.is_empty()).then_some(items.len()),
                    Value::Number(n) => (n.as_f64() != Some(0.0)).then(|| n.as_u64().unwrap_or(1) as usize),
                    _ => Some(1),
                }
            };
            let mut linked: Vec<String> = Vec::new();
            match links.as_object() {
                Some(object) => {
                    for (key, value) in object {
                        if let Some(count) = linked_to(value) {
                            let (one, more) = match key.as_str() {
                                "koppelMutatie" => ("mutatie", "mutaties"),
                                "koppelImport" => ("bankregel", "bankregels"),
                                "koppelRelatie" => ("relatie", "relaties"),
                                "koppelOfferte" => ("offerte", "offertes"),
                                _ => ("ander onderdeel", "andere onderdelen"),
                            };
                            linked.push(if count == 1 { format!("1 {one}") } else { format!("{count} {more}") });
                        }
                    }
                }
                None => linked.push("iets wat EB MCP niet kan lezen".to_owned()),
            }
            if linked.is_empty() && file.get("koppelCount").and_then(Value::as_u64).unwrap_or(0) > 0 {
                linked.push("iets anders".to_owned());
            }
            let mut lines = vec![
                line("Bestand", &name),
                line(
                    "Gevolg",
                    "het bestand gaat naar de prullenbak van het digitaal archief",
                ),
            ];
            if !linked.is_empty() {
                lines.push(line(
                    "Let op",
                    format!(
                        "dit bestand is nog gekoppeld aan {}; die koppeling verdwijnt mee",
                        linked.join(", ")
                    ),
                ));
            }
            let built = Built {
                body: Value::Null,
                title: format!("Bestand verwijderen: {name}"),
                lines,
            };
            this.approve(&run, TOOL, &built, !linked.is_empty()).await?;
            this.change_done(&run, "DELETE", &format!("folder/{}", args.bestand_id), None)
                .await
        })
        .await
    }

    #[tool(
        description = "Een verkoopfactuur als pdf ophalen, op id (uit facturen). De pdf wordt op deze computer opgeslagen, standaard in de map Downloads; het antwoord geeft het volledige pad en de pdf zelf als bijlage, zodat je hem kunt lezen en de gebruiker hem kan openen. Bestaande bestanden worden niet overschreven.",
        annotations(title = "Factuur als pdf", read_only_hint = true)
    )]
    async fn factuur_pdf(
        &self,
        Parameters(args): Parameters<PdfArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let run = match self.begin(&context, "factuur_pdf", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let path = format!("factuur/{}/print", args.id);
        let result = self
            .save_pdf(
                &run,
                &path,
                args.map.as_deref(),
                &format!("factuur-{}.pdf", args.id),
            )
            .await;
        Ok(self.end_pdf(run, &path, result))
    }

    #[tool(
        description = "Een verkoopfactuur per e-mail versturen vanuit e-Boekhouden, met de factuur als pdf in de bijlage. Onderwerp en bericht komen uit een e-mailsjabloon voor facturen (sjabloon_id), zoals e-Boekhouden het voor deze factuur invult, of geef je zelf mee; eigen tekst gaat zoals hij is, dus zonder codes zoals [FACTNUMMER]. Zonder ontvanger gaat de mail naar het factuur- of e-mailadres van de relatie. Dit verstuurt echte e-mail en is niet terug te draaien. De gebruiker moet dit altijd goedkeuren in EB MCP en ziet daar de ontvanger, het onderwerp en het bericht. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Factuur mailen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn factuur_mailen(
        &self,
        Parameters(args): Parameters<FactuurMailenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "factuur_mailen";
        let account = args.account.clone();
        self.write_tool(&context, TOOL, account.as_deref(), args, "Factuur verstuurd per e-mail", move |this, run, args| async move {
            let factuur = this.fetch(&run, &format!("factuur/{}", args.id), Vec::new()).await?;
            let nummer = factuur.get("nummer").and_then(Value::as_str).unwrap_or_default().to_owned();
            let relatie_id = factuur.get("relatieId").and_then(Value::as_u64).unwrap_or(0);
            let given = |value: &Option<String>| value.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_owned);
            own_text_without_codes(given(&args.onderwerp).as_deref(), given(&args.bericht).as_deref())?;
            // A template's text as e-Boekhouden fills it for this invoice: that is what is shown and sent.
            let filled = match args.sjabloon_id {
                Some(id) => {
                    for_invoices(&this.fetch(&run, "emailsjabloonab/forfactuur", Vec::new()).await?, id)?;
                    Some(this.fetch(&run, &format!("factuur/maildata/{id}/{}", args.id), Vec::new()).await?)
                }
                None => None,
            };
            let aan = match given(&args.aan) {
                Some(aan) => aan,
                None => {
                    let relatie = this.fetch(&run, &format!("relatie/{relatie_id}"), Vec::new()).await?;
                    let field = |name: &str| relatie.get(name).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()).map(str::to_owned);
                    field("emailFactuur")
                        .or_else(|| field("email"))
                        .ok_or_else(|| Halt::Error("De relatie van deze factuur heeft geen e-mailadres; geef de ontvanger op met aan".into()))?
                }
            };
            let user = this.fetch(&run, "user", Vec::new()).await?;
            let eigen = EigenMail { onderwerp: &args.onderwerp, bericht: &args.bericht, cc: &args.cc, bcc: &args.bcc };
            let mail = factuur_mail(&eigen, filled.as_ref(), &aan, &user, run.account.administration.as_deref())?;
            let FactuurMail { aan, cc, bcc, van_naam, van_email, onderwerp: subject, bericht: body } = mail.clone();
            let ubl = args.ubl.unwrap_or(false);
            let built = Built {
                body: factuur_mail_body(&mail, ubl, args.id),
                title: format!("Factuur {nummer} mailen naar {aan}"),
                lines: [
                    Some(line("Aan", &aan)),
                    cc.as_ref().map(|cc| line("Cc", cc)),
                    bcc.as_ref().map(|bcc| line("Bcc", bcc)),
                    Some(line("Van", format!("{van_naam} <{van_email}>"))),
                    Some(line("Onderwerp", &subject)),
                    Some(line("Bericht", plain_text(&body))),
                    targets(&body).map(|t| line("Links en afbeeldingen in het bericht", t)),
                    Some(line("Bijlage", if ubl { format!("factuur {nummer} als pdf en als UBL") } else { format!("factuur {nummer} als pdf") })),
                    Some(line("Let op", "dit verstuurt echte e-mail en is niet terug te draaien")),
                ]
                .into_iter()
                .flatten()
                .collect(),
            };
            this.approve(&run, TOOL, &built, true).await?;
            let sent = this.change(&run, "POST", "factuur/mail", Some(&built.body)).await?;
            let failed = sent.get("errorCount").and_then(Value::as_u64).unwrap_or(0) > 0;
            let said = sent.get("errorMessage").and_then(Value::as_str).filter(|m| !m.is_empty());
            if failed || said.is_some() {
                return Err(Halt::Error(format!("e-Boekhouden kon de mail niet versturen: {}", said.unwrap_or("geen uitleg gegeven"))));
            }
            Ok(json!({"verstuurd_aan": aan, "factuur": nummer}))
        })
        .await
    }

    #[tool(
        description = "De e-mailsjablonen van de administratie, met onderwerp, bericht (HTML), afzender en waarvoor ze te kiezen zijn. In onderwerp en bericht staan codes die e-Boekhouden invult, zoals [FACTNUMMER], [FACTKENMERK], [RELBEDRIJF], [RELCONTACTPERSOON], [RELAANHEF], [RELADRES], [RELPOSTCODE], [RELPLAATS], [DATUM] en [ABBEDRIJF].",
        annotations(title = "E-mailsjablonen", read_only_hint = true)
    )]
    async fn emailsjablonen(
        &self,
        Parameters(args): Parameters<AccountArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        self.simple(
            &context,
            "emailsjablonen",
            &args,
            args.account.as_deref(),
            "emailsjabloonab",
            Ok(Vec::new()),
        )
        .await
    }

    #[tool(
        description = "Een e-mailsjabloon maken voor het mailen van facturen of offertes. Gebruik in onderwerp en bericht de codes van e-Boekhouden, zoals [FACTNUMMER], [RELBEDRIJF] en [ABBEDRIJF] (zie emailsjablonen voor voorbeelden). Het bericht is HTML. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt het eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "E-mailsjabloon toevoegen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn emailsjabloon_toevoegen(
        &self,
        Parameters(args): Parameters<EmailsjabloonToevoegenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "emailsjabloon_toevoegen";
        let account = args.account.clone();
        self.write_tool(&context, TOOL, account.as_deref(), args, "E-mailsjabloon toegevoegd; data.id is het id", move |this, run, args| async move {
            let naam = args.naam.trim().to_owned();
            let (subject, body) = (args.onderwerp.trim().to_owned(), args.bericht.trim().to_owned());
            if naam.is_empty() || subject.is_empty() || body.is_empty() {
                return Err(Halt::Error("Naam, onderwerp en bericht zijn alle drie nodig".into()));
            }
            let (factuur, offerte) = sjabloon_voor(args.voor.as_deref().unwrap_or(&[SjabloonVoor::Factuur]))?;
            let given = |value: &Option<String>| value.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_owned);
            let user = this.fetch(&run, "user", Vec::new()).await?;
            let van_naam = given(&args.van_naam)
                .or_else(|| run.account.administration.clone())
                .ok_or_else(|| Halt::Error("Geef de naam van de afzender met van_naam".into()))?;
            let van_email = given(&args.van_email)
                .or_else(|| user.get("email").and_then(Value::as_str).map(str::to_owned))
                .ok_or_else(|| Halt::Error("Geef het e-mailadres van de afzender met van_email".into()))?;
            let van_email = email(&van_email, "Het adres van de afzender")?;
            let cc = given(&args.cc).map(|v| email(&v, "Het cc-adres")).transpose()?.unwrap_or_default();
            let bcc = given(&args.bcc).map(|v| email(&v, "Het bcc-adres")).transpose()?.unwrap_or_default();
            let built = Built {
                body: json!({"naam": naam, "vanNaam": van_naam, "vanEmail": van_email, "cc": cc, "bcc": bcc, "subject": subject, "body": body,
                             "useOfferte": offerte, "useFactuur": factuur, "useNB": false}),
                title: format!("E-mailsjabloon toevoegen: {naam}"),
                lines: vec![
                    line("Naam", &naam),
                    line("Te kiezen bij", match (factuur, offerte) { (true, true) => "facturen en offertes", (true, false) => "facturen", _ => "offertes" }),
                    line("Van", format!("{van_naam} <{van_email}>")),
                    line("Onderwerp", &subject),
                    line("Bericht", plain_text(&body)),
                ],
            };
            this.approve(&run, TOOL, &built, false).await?;
            this.change(&run, "POST", "emailsjabloonab", Some(&built.body)).await
        })
        .await
    }

    #[tool(
        description = "Een e-mailsjabloon wijzigen, op id (uit emailsjablonen). Wat je niet noemt blijft zoals het is. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de wijziging eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "E-mailsjabloon wijzigen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn emailsjabloon_wijzigen(
        &self,
        Parameters(args): Parameters<EmailsjabloonWijzigenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "emailsjabloon_wijzigen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "E-mailsjabloon gewijzigd",
            move |this, run, args| async move {
                let mut sjabloon = this
                    .fetch(&run, &format!("EmailSjabloonAb/{}", args.id), Vec::new())
                    .await?;
                let old_name = sjabloon
                    .get("naam")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let object = sjabloon.as_object_mut().ok_or_else(|| {
                    Halt::Error("e-Boekhouden gaf dit sjabloon in een onbekende vorm terug".into())
                })?;
                let mut lines = vec![line("Sjabloon", &old_name)];
                let mut set = |key: &str,
                               label: &str,
                               value: &Option<String>,
                               shown: fn(&str) -> String|
                 -> Result<(), Halt> {
                    if let Some(value) = value {
                        let value = value.trim();
                        if value.is_empty()
                            && matches!(key, "naam" | "subject" | "body" | "vanNaam")
                        {
                            return Err(Halt::Error(format!("{label} mag niet leeg zijn")));
                        }
                        if matches!(key, "vanEmail" | "cc" | "bcc") && !value.is_empty() {
                            email(value, label)?;
                        }
                        object.insert(key.to_owned(), json!(value));
                        lines.push(line(&format!("{label} wordt"), shown(value)));
                    }
                    Ok(())
                };
                set("naam", "Naam", &args.naam, str::to_owned)?;
                set("subject", "Onderwerp", &args.onderwerp, str::to_owned)?;
                set("body", "Bericht", &args.bericht, plain_text)?;
                set("vanNaam", "Naam afzender", &args.van_naam, str::to_owned)?;
                set("vanEmail", "Adres afzender", &args.van_email, str::to_owned)?;
                set("cc", "Cc", &args.cc, str::to_owned)?;
                set("bcc", "Bcc", &args.bcc, str::to_owned)?;
                if let Some(voor) = &args.voor {
                    let (factuur, offerte) = sjabloon_voor(voor)?;
                    object.insert("useFactuur".into(), json!(factuur));
                    object.insert("useOfferte".into(), json!(offerte));
                    lines.push(line(
                        "Te kiezen bij",
                        match (factuur, offerte) {
                            (true, true) => "facturen en offertes",
                            (true, false) => "facturen",
                            _ => "offertes",
                        },
                    ));
                }
                if lines.len() == 1 {
                    return Err(Halt::Error(
                        "Er is niets gewijzigd: geef aan wat er anders moet".into(),
                    ));
                }
                let built = Built {
                    body: sjabloon,
                    title: format!("E-mailsjabloon wijzigen: {old_name}"),
                    lines,
                };
                // Where the mail comes from and who gets a copy decides where customers' invoices go: always asked.
                let redirects = args.cc.is_some() || args.bcc.is_some() || args.van_email.is_some();
                this.approve(&run, TOOL, &built, redirects).await?;
                this.change_done(&run, "PUT", "emailsjabloonab", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een e-mailsjabloon verwijderen, op id. Dit is niet terug te draaien. De gebruiker keurt dit goed in EB MCP, tenzij verwijderen voor het account zonder akkoord staat. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "E-mailsjabloon verwijderen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn emailsjabloon_verwijderen(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "emailsjabloon_verwijderen";
        let account = args.account.clone();
        self.write_tool(&context, TOOL, account.as_deref(), args, "E-mailsjabloon verwijderd", move |this, run, args| async move {
            let sjabloon = this.fetch(&run, &format!("EmailSjabloonAb/{}", args.id), Vec::new()).await?;
            let naam = sjabloon.get("naam").and_then(Value::as_str).unwrap_or_default().to_owned();
            let built = Built {
                body: Value::Null,
                title: format!("E-mailsjabloon verwijderen: {naam}"),
                lines: vec![
                    line("Sjabloon", &naam),
                    line("Onderwerp", sjabloon.get("subject").and_then(Value::as_str).unwrap_or_default()),
                    line("Gevolg", "het sjabloon verdwijnt; facturen en abonnementen die het gebruikten hebben dan geen e-mailsjabloon meer. Niet terug te draaien."),
                ],
            };
            this.approve(&run, TOOL, &built, true).await?;
            this.change_done(&run, "DELETE", &format!("emailsjabloonab/{}", args.id), None).await
        })
        .await
    }

    #[tool(
        description = "De offertes: id, nummer, status, datum, vervaldatum, relatie, kenmerk en bedragen.",
        annotations(title = "Offertes", read_only_hint = true)
    )]
    async fn offertes(
        &self,
        Parameters(args): Parameters<ListArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const PATH: &str = "offerte/gridtable";
        let run = match self.begin(&context, "offertes", &args, args.account.as_deref()) {
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
        description = "Eén offerte met zijn regels, op id (uit offertes), plus de offertestatussen om uit te kiezen.",
        annotations(title = "Offerte", read_only_hint = true)
    )]
    async fn offerte(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let run = match self.begin(&context, "offerte", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let result = async {
            let offerte = self.fetch(&run, &format!("offerte/{}", args.id), Vec::new()).await?;
            let regels = self.fetch(&run, &format!("offerteregel/getbyofferteid/{}", args.id), Vec::new()).await?;
            let statussen = named_rows(self.fetch(&run, "offertestatus/gridtable", grid(Some(MAX_ROWS), None)?).await?);
            Ok(json!({"offerte": offerte, "regels": regels, "statussen": statussen.get("data").cloned().unwrap_or(Value::Null)}))
        }
        .await;
        Ok(self.end(run, &format!("offerte/{}", args.id), result))
    }

    #[tool(
        description = "Een offerte als pdf ophalen, op id (uit offertes). De pdf wordt op deze computer opgeslagen, standaard in de map Downloads; het antwoord geeft het volledige pad en de pdf zelf als bijlage. Bestaande bestanden worden niet overschreven.",
        annotations(title = "Offerte als pdf", read_only_hint = true)
    )]
    async fn offerte_pdf(
        &self,
        Parameters(args): Parameters<PdfArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let run = match self.begin(&context, "offerte_pdf", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let path = format!("offerte/pdf/{}", args.id);
        let result = self
            .save_pdf(
                &run,
                &path,
                args.map.as_deref(),
                &format!("offerte-{}.pdf", args.id),
            )
            .await;
        Ok(self.end_pdf(run, &path, result))
    }

    #[tool(
        description = "Een offerte maken voor een relatie. Prijzen per stuk, incl of excl btw; btw op code (HOOG_VERK_21, LAAG_VERK_9, GEEN, ...), de omzetrekening op code. Zonder nummer krijgt de offerte het volgende nummer van e-Boekhouden. De offerte wordt niet verstuurd. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de offerte eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Offerte toevoegen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn offerte_toevoegen(
        &self,
        Parameters(args): Parameters<write::OfferteToevoegenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "offerte_toevoegen";
        let account = args.account.clone();
        self.write_tool(&context, TOOL, account.as_deref(), args, "Offerte toegevoegd; data.id is het id en data.nummer het offertenummer", move |this, run, args| async move {
            let books = this.books(&run).await?;
            let nummer = this.fetch(&run, "offerte/nummer/next", Vec::new()).await?;
            let user = this.fetch(&run, "user", Vec::new()).await?;
            let sjabloon_id = match args.sjabloon_id {
                Some(id) => id,
                None => this
                    .first_active(&run, "offertesjabloon/gridtable")
                    .await?
                    .ok_or_else(|| Halt::Error("Deze administratie heeft geen actief offertesjabloon".into()))?,
            };
            let status_id = match args.status_id {
                Some(id) => id,
                None => this
                    .first_active(&run, "offertestatus/gridtable")
                    .await?
                    .ok_or_else(|| Halt::Error("Deze administratie heeft geen offertestatussen".into()))?,
            };
            let offerte_context = write::OfferteContext {
                volgend_nummer: nummer.as_str().unwrap_or_default().to_owned(),
                sjabloon_id,
                status_id,
                gebruiker_id: user
                    .get("id")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| Halt::Error("e-Boekhouden gaf de ingelogde gebruiker niet terug".into()))?,
                vandaag: today(),
            };
            let (built, regels) = write::offerte_nieuw(&args, &offerte_context, &books)?;
            this.approve(&run, TOOL, &built, false).await?;
            let made = this.change(&run, "POST", "offerte", Some(&built.body)).await?;
            let id = made
                .get("id")
                .and_then(Value::as_u64)
                .ok_or_else(|| Halt::Error("e-Boekhouden gaf de nieuwe offerte niet terug".into()))?;
            for (n, mut regel) in regels.into_iter().enumerate() {
                regel["offerteId"] = json!(id);
                if let Err(Halt::Error(message) | Halt::Refused(message)) = this.change(&run, "POST", "offerteregel", Some(&regel)).await {
                    return Err(Halt::Error(format!(
                        "De offerte is aangemaakt (id {id}), maar regel {} is niet opgeslagen: {message}. Vul de regels aan met offerte_wijzigen.",
                        n + 1
                    )));
                }
            }
            // The quote exists; a failed read-back must not look like a failed change (a retry would make a second one).
            Ok(this
                .fetch(&run, &format!("offerte/{id}"), Vec::new())
                .await
                .unwrap_or(made))
        })
        .await
    }

    #[tool(
        description = "Een offerte wijzigen, op id. Wat je niet noemt blijft zoals het is; regels die je meegeeft vervangen alle bestaande. De statussen staan bij de tool offerte. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de wijziging eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Offerte wijzigen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn offerte_wijzigen(
        &self,
        Parameters(args): Parameters<write::OfferteWijzigenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "offerte_wijzigen";
        let account = args.account.clone();
        self.write_tool(&context, TOOL, account.as_deref(), args, "Offerte gewijzigd", move |this, run, args| async move {
            let now = this.fetch(&run, &format!("offerte/{}", args.id), Vec::new()).await?;
            let old = this.fetch(&run, &format!("offerteregel/getbyofferteid/{}", args.id), Vec::new()).await?;
            let books = this.books(&run).await?;
            let (built, regels) = write::offerte_gewijzigd(&args, &now, &books)?;
            this.approve(&run, TOOL, &built, false).await?;
            this.change_done(&run, "PUT", "offerte", Some(&built.body)).await?;
            if let Some(regels) = regels {
                // New lines go in before the old ones go out: a failure halfway leaves too many lines, never none.
                for (n, mut regel) in regels.into_iter().enumerate() {
                    regel["offerteId"] = json!(args.id);
                    if let Err(Halt::Error(message) | Halt::Refused(message)) = this.change(&run, "POST", "offerteregel", Some(&regel)).await {
                        return Err(Halt::Error(format!(
                            "De offerte is gewijzigd, maar nieuwe regel {} is niet opgeslagen: {message}. De oude regels staan er nog; kijk met offerte wat er nu staat.",
                            n + 1
                        )));
                    }
                }
                for regel in old.as_array().cloned().unwrap_or_default() {
                    if let Some(id) = regel.get("id").and_then(Value::as_u64)
                        && let Err(Halt::Error(message) | Halt::Refused(message)) =
                            this.change_done(&run, "DELETE", &format!("offerteregel/{id}"), None).await
                    {
                        return Err(Halt::Error(format!(
                            "De nieuwe regels staan erin, maar een oude regel (id {id}) is niet verwijderd: {message}. Kijk met offerte wat er nu staat."
                        )));
                    }
                }
            }
            Ok(this
                .fetch(&run, &format!("offerte/{}", args.id), Vec::new())
                .await
                .unwrap_or_else(|_| json!({"id": args.id})))
        })
        .await
    }

    #[tool(
        description = "Een offerte verwijderen, op id. Dit is niet terug te draaien. De gebruiker keurt dit goed in EB MCP, tenzij verwijderen voor het account zonder akkoord staat. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Offerte verwijderen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn offerte_verwijderen(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "offerte_verwijderen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Offerte verwijderd",
            move |this, run, args| async move {
                let now = this
                    .fetch(&run, &format!("offerte/{}", args.id), Vec::new())
                    .await?;
                let nummer = now
                    .get("nummer")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let built = Built {
                    body: Value::Null,
                    title: format!("Offerte {nummer} verwijderen"),
                    lines: vec![
                        line("Offerte", &nummer),
                        line(
                            "Relatie",
                            format!(
                                "id {}",
                                now.get("relatieId").and_then(Value::as_u64).unwrap_or(0)
                            ),
                        ),
                        line(
                            "Totaal incl. btw",
                            write::euro(
                                now.get("totaalIncl").and_then(Value::as_f64).unwrap_or(0.0),
                            ),
                        ),
                        line(
                            "Gevolg",
                            "de offerte en zijn regels verdwijnen; dit is niet terug te draaien",
                        ),
                    ],
                };
                this.approve(&run, TOOL, &built, true).await?;
                this.change_done(&run, "DELETE", &format!("offerte/{}", args.id), None)
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "De sjablonen van één soort: factuur (opmaak van de factuur-pdf), offerte (opmaak van de offerte-pdf) of aanmaan (herinneringen en aanmaningen). E-mailsjablonen staan bij emailsjablonen.",
        annotations(title = "Sjablonen", read_only_hint = true)
    )]
    async fn sjablonen(
        &self,
        Parameters(args): Parameters<SjablonenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let path = format!("{}/gridtable", args.soort.path());
        let run = match self.begin(&context, "sjablonen", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let result = async {
            Ok(named_rows(
                self.fetch(&run, &path, grid(Some(MAX_ROWS), None)?).await?,
            ))
        }
        .await;
        Ok(self.end(run, &path, result))
    }

    #[tool(
        description = "Eén sjabloon lezen, op soort en id. Bij aanmaan: het onderwerp en de tekst. Bij factuur en offerte: de opmaak zoals e-Boekhoudens sjabloon-editor hem kent (`opmaak`: elements met id, tag, plaats x en y, width, height en content als HTML met codes; documentDefinition met het paginaformaat; sjabloon met de algemene instellingen) en `codes`, alle codes die e-Boekhouden invult met hun uitleg. Geef `opmaak` met je wijzigingen terug aan sjabloon_wijzigen.",
        annotations(title = "Sjabloon", read_only_hint = true)
    )]
    async fn sjabloon(
        &self,
        Parameters(args): Parameters<SjabloonArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let run = match self.begin(&context, "sjabloon", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let path = format!("{}/{}", args.soort.path(), args.id);
        let result = async {
            let sjabloon = self.fetch(&run, &path, Vec::new()).await?;
            if args.soort == SjabloonSoort::Aanmaan {
                return Ok(json!({"sjabloon": sjabloon}));
            }
            if sjabloon.get("type").and_then(Value::as_str) != Some("E") {
                return Ok(json!({"sjabloon": sjabloon, "opmaak": Value::Null, "let_op": "Dit sjabloon is een geüpload RTF-bestand; de opmaak daarvan is niet te lezen of te wijzigen via EB MCP."}));
            }
            let (hash, opmaak) = self.editor_session(&run, args.soort, args.id).await?;
            let codes = self
                .state
                .api_editor(run.account.id, "GET", &format!("api/sjabloon/codes/{hash}"), None, Some(run.id), &run.client)
                .await;
            // Reading opens a session in the editor; it is closed again without saving anything.
            self.editor_end(&run, &hash, opmaak.clone(), false).await?;
            Ok(json!({"sjabloon": sjabloon, "opmaak": opmaak, "codes": codes.json.unwrap_or(Value::Null)}))
        }
        .await;
        Ok(self.end(run, &path, result))
    }

    #[tool(
        description = "Een sjabloon maken. Factuur en offerte: een kopie van een bestaand sjabloon (kopie_van, standaard het eerste actieve); pas de opmaak daarna aan met sjabloon_wijzigen. Aanmaan: een e-mailtekst met onderwerp en bericht. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt het eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Sjabloon toevoegen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn sjabloon_toevoegen(
        &self,
        Parameters(args): Parameters<SjabloonToevoegenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "sjabloon_toevoegen";
        let account = args.account.clone();
        self.write_tool(&context, TOOL, account.as_deref(), args, "Sjabloon toegevoegd; data.id is het id", move |this, run, args| async move {
            let naam = args.naam.trim().to_owned();
            if naam.is_empty() || naam.chars().count() > 50 {
                return Err(Halt::Error("Geef het sjabloon een naam van hoogstens 50 tekens".into()));
            }
            let built = if args.soort == SjabloonSoort::Aanmaan {
                let given = |value: &Option<String>, what: &str| {
                    value
                        .as_deref()
                        .map(str::trim)
                        .filter(|v| !v.is_empty())
                        .map(str::to_owned)
                        .ok_or_else(|| Halt::Error(format!("Een aanmaansjabloon heeft {what} nodig")))
                };
                let (onderwerp, bericht) = (given(&args.onderwerp, "een onderwerp")?, given(&args.bericht, "een bericht")?);
                Built {
                    body: json!({"naam": naam, "soort": "E", "onderwerp": onderwerp, "tekstEmail": bericht}),
                    title: format!("Aanmaansjabloon toevoegen: {naam}"),
                    lines: vec![line("Naam", &naam), line("Verzendwijze", "per e-mail"), line("Onderwerp", &onderwerp), line("Bericht", plain_text(&bericht))],
                }
            } else {
                let from = match args.kopie_van {
                    Some(id) => id,
                    None => this
                        .first_active(&run, &format!("{}/gridtable", args.soort.path()))
                        .await?
                        .ok_or_else(|| Halt::Error("Er is geen actief sjabloon om te kopiëren; geef kopie_van".into()))?,
                };
                let source = this.fetch(&run, &format!("{}/{from}", args.soort.path()), Vec::new()).await?;
                if source.get("type").and_then(Value::as_str) != Some("E") {
                    return Err(Halt::Error("Het sjabloon om te kopiëren is een RTF-bestand; kies er een dat in de editor is gemaakt".into()));
                }
                Built {
                    body: json!({"naam": naam, "type": "E", "taal": "NL", "inactief": false, "copyFromId": from}),
                    title: format!("{} toevoegen: {naam}", args.soort.naam()),
                    lines: vec![
                        line("Naam", &naam),
                        line("Opmaak", format!("kopie van \"{}\"", source.get("naam").and_then(Value::as_str).unwrap_or_default())),
                    ],
                }
            };
            this.approve(&run, TOOL, &built, false).await?;
            this.change(&run, "POST", args.soort.path(), Some(&built.body)).await
        })
        .await
    }

    #[tool(
        description = "Een sjabloon wijzigen, op soort en id. Aanmaan: naam, onderwerp of bericht. Factuur en offerte: naam, actief of inactief, en de opmaak: geef dan het hele document `opmaak` uit de tool sjabloon terug met je wijzigingen (tekst in content, plaats in x en y, grootte in width en height; nieuwe elementen krijgen een nieuw id). De gebruiker ziet per element wat er verandert. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de wijziging eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Sjabloon wijzigen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn sjabloon_wijzigen(
        &self,
        Parameters(args): Parameters<SjabloonWijzigenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "sjabloon_wijzigen";
        let account = args.account.clone();
        let path = format!("{}/{}", args.soort.path(), args.id);
        self.write_tool(&context, TOOL, account.as_deref(), args, "Sjabloon gewijzigd", move |this, run, args| async move {
            let mut sjabloon = this.fetch(&run, &path, Vec::new()).await?;
            let old_name = sjabloon.get("naam").and_then(Value::as_str).unwrap_or_default().to_owned();
            let mut lines = vec![line(args.soort.naam(), &old_name)];
            let mut meta_changed = false;
            let object = sjabloon
                .as_object_mut()
                .ok_or_else(|| Halt::Error("e-Boekhouden gaf dit sjabloon in een onbekende vorm terug".into()))?;
            if let Some(naam) = args.naam.as_deref().map(str::trim) {
                if naam.is_empty() || naam.chars().count() > 50 {
                    return Err(Halt::Error("Geef het sjabloon een naam van hoogstens 50 tekens".into()));
                }
                object.insert("naam".into(), json!(naam));
                lines.push(line("Naam wordt", naam));
                meta_changed = true;
            }
            let aanmaan = args.soort == SjabloonSoort::Aanmaan;
            if aanmaan {
                if args.inactief.is_some() || args.opmaak.is_some() {
                    return Err(Halt::Error("Een aanmaansjabloon heeft geen opmaak en geen inactief; wijzig naam, onderwerp of bericht".into()));
                }
                for (value, key, label) in [(&args.onderwerp, "onderwerp", "Onderwerp"), (&args.bericht, "tekstEmail", "Bericht")] {
                    if let Some(value) = value.as_deref().map(str::trim) {
                        if value.is_empty() {
                            return Err(Halt::Error(format!("{label} mag niet leeg zijn")));
                        }
                        object.insert(key.into(), json!(value));
                        lines.push(line(&format!("{label} wordt"), if key == "tekstEmail" { plain_text(value) } else { value.to_owned() }));
                        meta_changed = true;
                    }
                }
            } else {
                if args.onderwerp.is_some() || args.bericht.is_some() {
                    return Err(Halt::Error("Onderwerp en bericht horen bij een aanmaansjabloon; wijzig hier naam, inactief of opmaak".into()));
                }
                if let Some(inactief) = args.inactief {
                    object.insert("inactief".into(), json!(inactief));
                    lines.push(line("Wordt", if inactief { "inactief" } else { "actief" }));
                    meta_changed = true;
                }
            }
            // The layout: compared with what the editor holds now, so the user sees what changes per element.
            let mut layout = None;
            if let Some(opmaak) = &args.opmaak {
                if sjabloon.get("type").and_then(Value::as_str) != Some("E") {
                    return Err(Halt::Error("Dit sjabloon is een geüpload RTF-bestand; de opmaak daarvan is niet te wijzigen via EB MCP".into()));
                }
                let (hash, now) = this.editor_session(&run, args.soort, args.id).await?;
                let checked = write::opmaak_changes(&now, opmaak);
                // The session that was opened to compare is closed without saving, whatever comes next.
                this.editor_end(&run, &hash, now.clone(), false).await?;
                let changes = checked?;
                if !changes.is_empty() {
                    lines.extend(changes);
                    // What the tool did not send stays as the editor has it.
                    let mut document = now;
                    for key in ["elements", "documentDefinition", "sjabloon"] {
                        if let Some(value) = opmaak.get(key) {
                            document[key] = value.clone();
                        }
                    }
                    layout = Some(document);
                }
            }
            if !meta_changed && layout.is_none() {
                return Err(Halt::Error("Er is niets gewijzigd: geef aan wat er anders moet".into()));
            }
            let built = Built { body: sjabloon, title: format!("{} wijzigen: {old_name}", args.soort.naam()), lines };
            this.approve(&run, TOOL, &built, false).await?;
            if meta_changed {
                this.change_done(&run, "PUT", args.soort.path(), Some(&built.body)).await?;
            }
            if let Some(document) = layout {
                let (hash, _) = this.editor_session(&run, args.soort, args.id).await.map_err(|e| {
                    Halt::Error(if meta_changed { format!("Naam of status is gewijzigd, maar de opmaak niet: {e}") } else { e })
                })?;
                this.editor_end(&run, &hash, document, true).await.map_err(|e| {
                    Halt::Error(if meta_changed {
                        format!("Naam of status is gewijzigd, maar de opmaak niet: {e}")
                    } else {
                        e
                    })
                })?;
            }
            Ok(json!(true))
        })
        .await
    }

    #[tool(
        description = "Een sjabloon verwijderen, op soort en id. Dit is niet terug te draaien. De gebruiker keurt dit goed in EB MCP, tenzij verwijderen voor het account zonder akkoord staat. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Sjabloon verwijderen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn sjabloon_verwijderen(
        &self,
        Parameters(args): Parameters<SjabloonArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "sjabloon_verwijderen";
        let account = args.account.clone();
        let path = format!("{}/{}", args.soort.path(), args.id);
        self.write_tool(&context, TOOL, account.as_deref(), args, "Sjabloon verwijderd", move |this, run, args| async move {
            let sjabloon = this.fetch(&run, &path, Vec::new()).await?;
            let naam = sjabloon.get("naam").and_then(Value::as_str).unwrap_or_default().to_owned();
            let built = Built {
                body: Value::Null,
                title: format!("{} verwijderen: {naam}", args.soort.naam()),
                lines: vec![
                    line(args.soort.naam(), &naam),
                    line("Gevolg", "het sjabloon verdwijnt; wat het gebruikte heeft dan geen sjabloon meer. Niet terug te draaien."),
                ],
            };
            this.approve(&run, TOOL, &built, true).await?;
            this.change_done(&run, "DELETE", &path, None).await
        })
        .await
    }

    #[tool(
        description = "Een bedrijf zoeken in het Handelsregister van de KvK, via e-Boekhouden: op bedrijfsnaam, KvK-nummer of postcode met huisnummer. Geeft per resultaat het KvK-nummer (nr), de naam, plaats, straat en het vestigingsnummer. Het volledige adres haal je op met kvk_adres. Handig om een relatie mee aan te maken.",
        annotations(title = "KvK zoeken", read_only_hint = true, open_world_hint = true)
    )]
    async fn kvk_zoeken(
        &self,
        Parameters(args): Parameters<KvkZoekenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const PATH: &str = "kvk/gridtable";
        let run = match self.begin(&context, "kvk_zoeken", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let zoek = args.zoek.trim();
        if zoek.chars().count() < 2 || zoek.chars().count() > 50 {
            return Ok(self.refuse(run, "Geef een zoekterm van 2 tot 50 tekens".into()));
        }
        let query = match grid(Some(args.limit.unwrap_or(20)), None) {
            Ok(mut query) => {
                query.insert(0, ("search".to_owned(), zoek.to_owned()));
                query
            }
            Err(message) => return Ok(self.refuse(run, message)),
        };
        let result = self.fetch(&run, PATH, query).await.map(named_rows);
        Ok(self.end(run, PATH, result))
    }

    #[tool(
        description = "Het adres bij een resultaat van kvk_zoeken: van een vestiging (vestigingsnummer) of van de eigenaar (kvk_nr). Geeft straat, huisnummer, postcode, plaats en land.",
        annotations(title = "KvK-adres", read_only_hint = true, open_world_hint = true)
    )]
    async fn kvk_adres(
        &self,
        Parameters(args): Parameters<KvkAdresArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let run = match self.begin(&context, "kvk_adres", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let digits = |value: &Option<String>| {
            value
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty() && v.len() <= 12 && v.chars().all(|c| c.is_ascii_digit()))
                .map(str::to_owned)
        };
        let path = match (digits(&args.vestigingsnummer), digits(&args.kvk_nr)) {
            (Some(vestiging), _) => format!("kvk/vestiging/{vestiging}/adres"),
            (None, Some(kvk)) => format!("kvk/{kvk}/eigenaar/adres"),
            (None, None) => {
                return Ok(self.refuse(
                    run,
                    "Geef een vestigingsnummer of een KvK-nummer (alleen cijfers)".into(),
                ));
            }
        };
        let result = self.fetch(&run, &path, Vec::new()).await;
        Ok(self.end(run, &path, result))
    }

    #[tool(
        description = "Een relatie (klant of leverancier) aanmaken. Naam en code zijn nodig; de code is uniek binnen de administratie. Zoek de gegevens van een bedrijf eerst op met kvk_zoeken en kvk_adres. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de relatie eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Relatie toevoegen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn relatie_toevoegen(
        &self,
        Parameters(args): Parameters<RelatieToevoegenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "relatie_toevoegen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Relatie toegevoegd; data.id is het id",
            move |this, run, args| async move {
                let given = args.velden.given()?;
                let has = |key: &str| given.iter().any(|(k, _, v, _)| *k == key && !v.is_empty());
                if !has("bedrijf") || !has("code") {
                    return Err(Halt::Error(
                        "Een relatie heeft een naam (bedrijf) en een code nodig".into(),
                    ));
                }
                let particulier = args.velden.particulier.unwrap_or(false);
                // The fields e-Boekhouden's form always sends along, with the values of an untouched form.
                let mut relatie = serde_json::Map::new();
                relatie.insert("bp".into(), json!(if particulier { "P" } else { "B" }));
                let mut lines = vec![line(
                    "Soort",
                    if particulier {
                        "particulier"
                    } else {
                        "bedrijf"
                    },
                )];
                for (key, label, value, _) in &given {
                    if !value.is_empty() {
                        relatie.insert((*key).into(), json!(value));
                        lines.push(line(label, value));
                    }
                }
                for key in [
                    "sepaMachtiging",
                    "menHAllowMail",
                    "menHAllowHerken",
                    "geenEmail",
                    "la",
                ] {
                    relatie.insert(key.into(), json!(0));
                }
                let naam = args
                    .velden
                    .bedrijf
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                let built = Built {
                    body: json!({"relatie": relatie}),
                    title: format!("Relatie toevoegen: {naam}"),
                    lines,
                };
                this.approve(&run, TOOL, &built, false).await?;
                this.change(&run, "POST", "relatie/model", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een relatie wijzigen, op id. Wat je niet noemt blijft zoals het is; een leeg veld maakt het leeg. Een wijziging van het IBAN, de BIC of een e-mailadres bepaalt waar geld en facturen heen gaan en vraagt daarom altijd het akkoord van de gebruiker. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Relatie wijzigen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn relatie_wijzigen(
        &self,
        Parameters(args): Parameters<RelatieWijzigenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "relatie_wijzigen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Relatie gewijzigd",
            move |this, run, args| async move {
                let mut relatie = this
                    .fetch(&run, &format!("relatie/{}", args.id), Vec::new())
                    .await?;
                let naam = relatie
                    .get("bedrijf")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let object = relatie.as_object_mut().ok_or_else(|| {
                    Halt::Error("e-Boekhouden gaf deze relatie in een onbekende vorm terug".into())
                })?;
                let mut lines = vec![line("Relatie", &naam)];
                let mut sensitive = false;
                for (key, label, value, decides) in args.velden.given()? {
                    let before = object
                        .get(key)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    if before == value {
                        continue;
                    }
                    sensitive |= decides;
                    lines.push(line(&format!("{label} was"), known(before)));
                    lines.push(line(
                        &format!("{label} wordt"),
                        if value.is_empty() {
                            "(leeg)".to_owned()
                        } else {
                            value.clone()
                        },
                    ));
                    object.insert(key.into(), json!(value));
                }
                if let Some(particulier) = args.velden.particulier {
                    let bp = if particulier { "P" } else { "B" };
                    if object.get("bp").and_then(Value::as_str) != Some(bp) {
                        object.insert("bp".into(), json!(bp));
                        lines.push(line(
                            "Soort wordt",
                            if particulier {
                                "particulier"
                            } else {
                                "bedrijf"
                            },
                        ));
                    }
                }
                if lines.len() == 1 {
                    return Err(Halt::Error(
                        "Er is niets gewijzigd: geef aan wat er anders moet".into(),
                    ));
                }
                let built = Built {
                    body: json!({"relatie": relatie}),
                    title: format!("Relatie wijzigen: {naam}"),
                    lines,
                };
                this.approve(&run, TOOL, &built, sensitive).await?;
                this.change_done(&run, "PUT", "relatie/model", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een relatie verwijderen, op id. Dit is niet terug te draaien. De gebruiker keurt dit goed in EB MCP, tenzij verwijderen voor het account zonder akkoord staat. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Relatie verwijderen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn relatie_verwijderen(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "relatie_verwijderen";
        let account = args.account.clone();
        self.write_tool(&context, TOOL, account.as_deref(), args, "Relatie verwijderd", move |this, run, args| async move {
            let relatie = this.fetch(&run, &format!("relatie/{}", args.id), Vec::new()).await?;
            let text = |key: &str| relatie.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
            let naam = text("bedrijf");
            let built = Built {
                body: Value::Null,
                title: format!("Relatie verwijderen: {naam}"),
                lines: vec![
                    line("Relatie", &naam),
                    line("Code", known(text("code"))),
                    line("Plaats", known(text("plaats"))),
                    line("Gevolg", "de relatie verdwijnt uit de administratie; dit is niet terug te draaien. e-Boekhouden kan weigeren als er nog boekingen aan hangen."),
                ],
            };
            this.approve(&run, TOOL, &built, true).await?;
            this.change_done(&run, "DELETE", &format!("relatie/{}", args.id), None).await
        })
        .await
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

// The router is named: without it the macro serves `Self::tool_router()`, the tools of this file alone.
#[tool_handler(router = self.tool_router)]
impl ServerHandler for EbMcp {
    /// As the macro would write it, with one step before the tool reads its arguments: a value of the wrong
    /// kind that can only mean one thing gets the kind the tool's schema asks for (lenient.rs).
    async fn call_tool(
        &self,
        mut request: rmcp::model::CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::CallToolResponse, McpError> {
        if let Some(arguments) = request.arguments.as_mut()
            && let Some(route) = self.tool_router.map.get(request.name.as_ref())
        {
            crate::lenient::fit(arguments, &route.attr.input_schema);
        }
        let call = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        self.tool_router.call(call).await
    }

    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(server_info())
            .with_instructions(
                "Leest de boekhouding in e-Boekhouden van de accounts die in EB MCP zijn ingelogd. \
                 Wijzigen (relaties, mutaties, bestanden, facturen, offertes, abonnementen, sjablonen, uren en projecten, producten, grootboekrekeningen en kostenplaatsen, importregels, vaste activa, de beginbalans, bedrijfsgegevens) en alles wat e-Boekhouden verlaat (facturen, offertes en aanmaningen mailen, Peppol, het incassobestand) kan alleen voor accounts waar de gebruiker dat heeft aangezet, per soort wijziging: boeken en wijzigen, verwijderen, versturen en ingrijpend; \
                 status zegt per account en per soort of die uit staat, met akkoord of zonder akkoord. Staat een soort uit, dan weigert de tool en zegt hij welke. Met akkoord toont EB MCP elke wijziging eerst aan de gebruiker; \
                 alles wat mail of een bestand de deur uit doet, crediteren, in bulk factureren of boeken, afboeken, bedrijfsgegevens en het verwerken van een btw-aangifte vragen altijd akkoord, en verwijderen ook tenzij de gebruiker dat anders heeft gezet. Zo'n tool wacht niet op de gebruiker: \
                 hij antwoordt meteen met status wacht en een voorstel_id, en voert de wijziging zelf uit zodra de gebruiker goedkeurt. \
                 Vertel de gebruiker dat er in EB MCP een wijziging op akkoord wacht, ga gerust verder met ander werk, \
                 en vraag de uitkomst later op met akkoord_status (met voorstel_id voor één voorstel en zijn resultaat; zonder voor alles van de laatste 24 uur). \
                 Klopt een voorstel dat nog wacht niet, of is het niet meer nodig, trek het dan zelf in met akkoord_intrekken (met een korte reden) in plaats van de gebruiker te vragen het te weigeren; na goedkeuring kan dat niet meer. \
                 Een geïmporteerde bankregel verwerken kan: bankregels geeft de onverwerkte regels met hun id, en mutatie_toevoegen met bankregel_id boekt de mutatie en verwerkt de regel in één keer, bij elke soort, ook een factuurbetaling. \
                 Boek geld van een bankregel nooit zonder bankregel_id: dan blijft de regel open staan en dreigt een dubbele boeking. \
                 Een btw-aangifte, ICP-opgave of suppletie versturen, indienen of betalen kan EB MCP niet: dat doet de gebruiker zelf. \
                 Wel kan btwaangifte_betaallink de iDEAL-link van een verstuurde aangifte opzoeken: toon die link aan de gebruiker om zelf te openen, open of volg hem nooit zelf. \
                 Zegt een tool dat de uitkomst onbekend is (een mail of bestand dat mogelijk al weg is), probeer het dan niet opnieuw maar laat de gebruiker eerst kijken. \
                 Staat er meer dan één account in EB MCP, dan moet elke tool de account-parameter meekrijgen; status zegt welke accounts er zijn.",
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_names_one_plain_type_for_a_field_that_may_be_left_out() {
        fn nullable_types(value: &Value, found: &mut Vec<String>, at: &str) {
            match value {
                Value::Object(object) => {
                    if let Some(Value::Array(types)) = object.get("type")
                        && types.iter().any(|t| t.as_str() == Some("null"))
                    {
                        found.push(at.to_owned());
                    }
                    for (name, inner) in object {
                        nullable_types(inner, found, &format!("{at}.{name}"));
                    }
                }
                Value::Array(items) => items.iter().for_each(|i| nullable_types(i, found, at)),
                _ => {}
            }
        }
        let tools = EbMcp::routers().list_all();
        assert!(tools.len() > 170);
        let mut found = Vec::new();
        for tool in &tools {
            nullable_types(
                &Value::Object((*tool.input_schema).clone()),
                &mut found,
                &tool.name,
            );
        }
        assert!(
            found.is_empty(),
            "a client reads these as untyped and sends a code as a number: {found:?}"
        );
        // The field the bug was reported on, and one inside a list.
        let mutatie = tools
            .iter()
            .find(|t| t.name == "mutatie_toevoegen")
            .unwrap();
        assert_eq!(
            mutatie.input_schema["properties"]["rekening"]["type"],
            "string"
        );
        assert_eq!(
            mutatie.input_schema["properties"]["bankregel_id"]["type"],
            "integer"
        );
        // Still optional: only what the tool cannot do without is required.
        let required: Vec<&str> = mutatie.input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(required, ["soort", "datum"]);
    }

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
            rights: crate::accounts::Rights::OFF,
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
        // GETs that act: paying or marking a btw return paid, handing an administration over, starting or
        // stopping an integration, recounting stock, and the page that shows API keys.
        for acting in [
            "aangifte/pay/12",
            "aangifte/paystatus/12",
            "aangifte/12/andersbetaald",
            "accountant/administratie/7/overdragen",
            "ing/connection-request",
            "kvk/verleng/starter",
            "microsoft/auth/office/deactivate",
            "tabletactie/cancel",
            "tip/3/close",
            "voorraad/actualiseer",
            "apiinfo/getByKey/abc",
            "ibanxs/newConsent",
            // Opening an ICP period may make the opgave: only the ICP tools read it, and say so.
            "icpopgave/2026/K/1",
            // An action word where the map has an id.
            "aangifte/pay",
            "factuur/verwijder",
            "folder/undelete",
            "vasteactiva/delete",
        ] {
            assert!(!allowed_path(acting), "{acting}");
        }
        assert!(allowed_path("aangifte/gridtable"));
        assert!(allowed_path("aangifte/12"));
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
    fn a_call_given_up_stops_its_change_only_before_approval() {
        let gate = Gate::default();
        assert!(gate.abandon(), "only read so far: stopped");
        assert!(!gate.commit(), "approve refuses once the call was given up");
        assert!(!gate.committed());

        let gate = Gate::default();
        assert!(gate.commit());
        assert!(gate.committed());
        assert!(
            !gate.abandon(),
            "once committed the change belongs to the queue and is not stopped halfway"
        );
        assert!(gate.committed());
    }

    #[test]
    fn counts_its_tools_on_the_router() {
        // The tools of this file, plus what each part names in its own module.
        let [reads, writes, _] = part_tools();
        assert_eq!(tool_count(), 56 + reads.len() + writes.len());
    }

    #[test]
    fn every_tool_that_sends_something_out_is_a_tool() {
        let names: Vec<String> = EbMcp::routers()
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        for tool in crate::accounts::SENDS {
            assert!(names.iter().any(|n| n == tool), "{tool} is not a tool");
        }
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
        // Columns that all claim index 0, as the KvK list sends them, still get their own value.
        let kvk = json!({
            "colMetadata": [{"name": "nr", "index": 0, "readerIndex": 0}, {"name": "bedrijf", "index": 0, "readerIndex": 0}, {"name": "plaats", "index": 0, "readerIndex": 0}],
            "data": [["85504289", "Vitrion", "Groningen"]]
        });
        assert_eq!(
            named_rows(kvk)["data"],
            json!([{"nr": "85504289", "bedrijf": "Vitrion", "plaats": "Groningen"}])
        );
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
    fn checks_addresses_and_shows_a_message_as_plain_words() {
        assert_eq!(
            email(" jan@studionoord.example ", "X").as_deref(),
            Ok("jan@studionoord.example")
        );
        for bad in [
            "",
            "jan",
            "jan@",
            "@x.nl",
            "jan@localhost",
            "jan @x.nl",
            "jan@x.",
            "jan@.nl",
        ] {
            assert!(
                email(bad, "De ontvanger")
                    .unwrap_err()
                    .contains("De ontvanger"),
                "{bad}"
            );
        }
        assert_eq!(
            plain_text(
                "<div>[RELBEDRIJF]<br />\n[RELADRES]<br />\n<br />\nGeachte heer/mevrouw,&nbsp;<br /></div>\n<div><span style=\"x\">[ABBEDRIJF]</span> &amp; co</div>"
            ),
            "[RELBEDRIJF]\n[RELADRES]\n\nGeachte heer/mevrouw,\n\n[ABBEDRIJF] & co"
        );
        assert_eq!(
            targets("<a href=\"https://evil.example/x\">https://bank.nl</a> <img SRC='https://t.example/p.gif'> <a href=https://evil.example/x>nog eens</a>").as_deref(),
            Some("https://evil.example/x\nhttps://t.example/p.gif")
        );
        assert_eq!(targets("<p>Geen links</p>"), None);
        assert_eq!(known(" ".into()), "onbekend");
        let velden = RelatieVelden {
            bedrijf: Some(" Studio Noord ".into()),
            iban: Some("NL91ABNA0417164300".into()),
            telefoon: Some(String::new()),
            ..Default::default()
        };
        assert_eq!(
            velden.given().unwrap(),
            vec![
                ("bedrijf", "Naam", "Studio Noord".to_owned(), false),
                ("telefoonnummer", "Telefoon", String::new(), false),
                ("iban", "IBAN", "NL91ABNA0417164300".to_owned(), true),
            ]
        );
        assert!(
            RelatieVelden {
                email_factuur: Some("geen adres".into()),
                ..Default::default()
            }
            .given()
            .is_err()
        );
        assert!(
            RelatieVelden {
                code: Some(" ".into()),
                ..Default::default()
            }
            .given()
            .is_err()
        );
        assert_eq!(sjabloon_voor(&[SjabloonVoor::Offerte]), Ok((false, true)));
        assert!(sjabloon_voor(&[]).is_err());
    }

    #[test]
    fn saves_a_download_next_to_what_is_there_already() {
        let dir = std::env::temp_dir().join(format!("ebmcp-pdf-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(
            write_new(&dir, "F00002.pdf", b"een").unwrap(),
            dir.join("F00002.pdf")
        );
        assert_eq!(
            write_new(&dir, "F00002.pdf", b"twee").unwrap(),
            dir.join("F00002 (2).pdf")
        );
        assert_eq!(
            write_new(&dir, "F00002.pdf", b"drie").unwrap(),
            dir.join("F00002 (3).pdf")
        );
        assert_eq!(
            std::fs::read(dir.join("F00002.pdf")).unwrap(),
            b"een",
            "never overwritten"
        );
        // A symlink with the wanted name is not followed: the file lands under the next name.
        #[cfg(unix)]
        {
            let elsewhere = dir.join("elders.txt");
            std::os::unix::fs::symlink(&elsewhere, dir.join("F9.pdf")).unwrap();
            assert_eq!(
                write_new(&dir, "F9.pdf", b"x").unwrap(),
                dir.join("F9 (2).pdf")
            );
            assert!(!elsewhere.exists());
        }
        assert_eq!(download_dir(Some(dir.to_str().unwrap())), Ok(dir.clone()));
        assert!(
            download_dir(Some("Downloads"))
                .unwrap_err()
                .contains("volledig pad")
        );
        assert!(download_dir(Some(dir.join("weg").to_str().unwrap())).is_err());
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
