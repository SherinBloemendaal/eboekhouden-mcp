//! The request bodies of the rekeningschema tools and the words the user approves them by. Nothing here talks to
//! e-Boekhouden: it turns a tool's arguments, plus what was read from the administration, into a `Built`.
//!
//! The shapes come from e-Boekhouden's own web app (the public bundle): the grootboekrekening form
//! (`app-grootboekrekening-form`, service `grootboekrekening`), the rekeninggroep form (`app-rekening-groep-form`,
//! service `rekeninggroep`) and the kostenplaats form and tree (`app-kostenplaatsen-form`, `app-kostenplaatsen-list`,
//! service `kostenplaats`).

use serde_json::{Map, Value, json};

use super::{
    GrootboekrekeningToevoegenArgs, GrootboekrekeningWijzigenArgs, KostenplaatsToevoegenArgs,
    KostenplaatsWijzigenArgs, RekeninggroepToevoegenArgs, RekeninggroepWijzigenArgs,
};
use crate::approvals::{Line, line};
use crate::write::{Built, Kostenplaats};

/// The form's limits: a code of at most 10 characters, an omschrijving of at most 100.
const CODE_MAX: usize = 10;
const OMSCHRIJVING_MAX: usize = 100;
/// The rekeninggroep form: code and naam of at most 50 characters each.
const GROEP_MAX: usize = 50;
/// The kostenplaats form: an omschrijving of at most 50 characters, a full path of at most 500.
const KOSTENPLAATS_MAX: usize = 50;
const PAD_MAX: usize = 500;

/// The words that say a change goes past what the approval setting of the account decides.
const ALTIJD: &str = "daarom vraagt dit altijd akkoord";

// ---------------------------------------------------------------- what is read first

/// A category of grootboekrekening, as `grootboekrekening/categories` gives it. A restricted one is not offered on
/// the form, and a rekening that has one cannot change its category there.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Categorie {
    pub code: String,
    pub omschrijving: String,
    pub restricted: bool,
}

pub(super) fn categorieen_from(list: &Value) -> Result<Vec<Categorie>, String> {
    let rows = list
        .as_array()
        .ok_or("De categorieën van e-Boekhouden hebben een onbekende vorm")?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            Some(Categorie {
                code: row.get("code")?.as_str()?.trim().to_owned(),
                omschrijving: row
                    .get("omschrijving")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                restricted: row
                    .get("restricted")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .filter(|c| !c.code.is_empty())
        .collect())
}

/// A rekeninggroep, as `GET rekeninggroep` and `GET rekeninggroep/{id}` give it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Groep {
    pub id: u64,
    pub code: String,
    pub naam: String,
}

fn groep_from(row: &Value) -> Option<Groep> {
    Some(Groep {
        id: row.get("id").and_then(Value::as_u64).unwrap_or(0),
        code: row.get("code")?.as_str()?.trim().to_owned(),
        naam: row
            .get("naam")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    })
}

pub(super) fn groepen_from(list: &Value) -> Result<Vec<Groep>, String> {
    let rows = list
        .as_array()
        .ok_or("De rekeninggroepen van e-Boekhouden hebben een onbekende vorm")?;
    Ok(rows.iter().filter_map(groep_from).collect())
}

/// A grootboekrekening as the rekeningschema grid lists it, inactive ones included.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct GridRekening {
    pub id: u64,
    pub code: String,
    pub omschrijving: String,
    /// The grid's `rekeningGroep` column, empty when the rekening is in no group.
    pub groep: String,
}

/// The rows of `grootboekrekening/gridtable` after `named_rows`.
pub(super) fn rekeningen_from(grid: &Value) -> Result<Vec<GridRekening>, String> {
    let rows = grid
        .get("data")
        .and_then(Value::as_array)
        .filter(|rows| rows.iter().all(Value::is_object))
        .ok_or("Het rekeningschema van e-Boekhouden heeft een onbekende vorm")?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            Some(GridRekening {
                id: row.get("id")?.as_u64()?,
                code: row.get("code")?.as_str()?.trim().to_owned(),
                omschrijving: row
                    .get("omschrijving")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                groep: row
                    .get("rekeningGroep")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_owned(),
            })
        })
        .collect())
}

/// The rekeningen of a group. The grid's `rekeningGroep` column names the group; the bundle does not show
/// whether by code, by naam or as the picker's "code - naam", so any of the three counts.
fn in_groep<'a>(rekeningen: &'a [GridRekening], groep: &Groep) -> Vec<&'a GridRekening> {
    let picker = format!("{} - {}", groep.code, groep.naam);
    rekeningen
        .iter()
        .filter(|r| {
            !r.groep.is_empty()
                && (r.groep == groep.code
                    || r.groep.eq_ignore_ascii_case(&groep.naam)
                    || r.groep == picker)
        })
        .collect()
}

/// The approval line that says how many rekeningen a group holds, and which.
fn in_groep_line(rekeningen: &[GridRekening], groep: &Groep) -> Line {
    let found = in_groep(rekeningen, groep);
    let value = match found.len() {
        0 => "geen".to_owned(),
        n => format!(
            "{n}: {}",
            found
                .iter()
                .map(|r| format!("{} {}", r.code, r.omschrijving))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    line("Rekeningen in deze groep", value)
}

/// Whether the group list knows `id`, before the group itself is read. A list without ids proves nothing.
pub(super) fn groep_bekend(groepen: &[Groep], id: u64) -> Result<(), String> {
    if groepen.iter().any(|g| g.id > 0) && !groepen.iter().any(|g| g.id == id) {
        return Err(geen_groep(id));
    }
    Ok(())
}

fn geen_groep(id: u64) -> String {
    format!("Geen rekeninggroep met id {id} in deze administratie (zie rekeninggroepen)")
}

/// The group `GET rekeninggroep/{id}` gave, or the plain refusal when it gave none.
fn groep_of(now: &Value, id: u64) -> Result<Groep, String> {
    groep_from(now)
        .filter(|g| g.id == 0 || g.id == id)
        .ok_or_else(|| geen_groep(id))
}

/// The id of the rekening a tool names, by id or by code (one of the two).
pub(super) fn which_rekening(
    rekeningen: &[GridRekening],
    id: Option<u64>,
    code: Option<&str>,
) -> Result<u64, String> {
    match (id, code.map(str::trim).filter(|c| !c.is_empty())) {
        (Some(_), Some(_)) => Err("Geef id of code van de grootboekrekening, niet allebei".into()),
        (Some(id), None) => rekeningen
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.id)
            .ok_or_else(|| format!("Geen grootboekrekening met id {id} in deze administratie")),
        (None, Some(code)) => rekeningen
            .iter()
            .find(|r| r.code == code)
            .map(|r| r.id)
            .ok_or_else(|| {
                format!("Geen grootboekrekening met code {code:?} in deze administratie")
            }),
        (None, None) => Err("Geef id of code van de grootboekrekening".into()),
    }
}

// ---------------------------------------------------------------- small checks

/// Trimmed, not empty, not longer than `max` characters.
fn required(value: &str, label: &str, max: usize) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{label} mag niet leeg zijn"));
    }
    if value.chars().count() > max {
        return Err(format!("{label} is langer dan {max} tekens"));
    }
    Ok(value.to_owned())
}

/// The category a tool names, by code or by name. A restricted one is refused: e-Boekhouden's form does not
/// offer those.
fn pick_categorie<'a>(categorieen: &'a [Categorie], wanted: &str) -> Result<&'a Categorie, String> {
    let wanted = wanted.trim();
    let found = categorieen.iter().find(|c| {
        c.code.eq_ignore_ascii_case(wanted) || c.omschrijving.eq_ignore_ascii_case(wanted)
    });
    match found {
        Some(c) if !c.restricted => Ok(c),
        Some(c) => Err(format!(
            "De categorie {} is in e-Boekhouden niet te kiezen voor een rekening",
            categorie_label(c)
        )),
        None => Err(format!(
            "Onbekende categorie {wanted:?}. Kies uit: {}",
            categorieen
                .iter()
                .filter(|c| !c.restricted)
                .map(categorie_label)
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn categorie_label(c: &Categorie) -> String {
    if c.omschrijving.is_empty() {
        c.code.clone()
    } else {
        format!("{} ({})", c.code, c.omschrijving)
    }
}

fn categorie_naam(categorieen: &[Categorie], code: &str) -> String {
    categorieen
        .iter()
        .find(|c| c.code == code)
        .map(categorie_label)
        .unwrap_or_else(|| code.to_owned())
}

/// The group a tool names, by code or by name; an empty text means no group.
fn pick_groep<'a>(groepen: &'a [Groep], wanted: &str) -> Result<Option<&'a Groep>, String> {
    let wanted = wanted.trim();
    if wanted.is_empty() {
        return Ok(None);
    }
    groepen
        .iter()
        .find(|g| g.code == wanted)
        .or_else(|| groepen.iter().find(|g| g.naam.eq_ignore_ascii_case(wanted)))
        .map(Some)
        .ok_or_else(|| {
            if groepen.is_empty() {
                "Deze administratie heeft geen rekeninggroepen; maak er eerst een met rekeninggroep_toevoegen"
                    .to_owned()
            } else {
                format!(
                    "Onbekende rekeninggroep {wanted:?}. Kies uit: {}",
                    groepen.iter().map(groep_label).collect::<Vec<_>>().join(", ")
                )
            }
        })
}

fn groep_label(g: &Groep) -> String {
    format!("{} {}", g.code, g.naam)
}

fn groep_naam(groepen: &[Groep], code: &str) -> String {
    if code.is_empty() {
        return "(geen)".to_owned();
    }
    groepen
        .iter()
        .find(|g| g.code == code)
        .map(groep_label)
        .unwrap_or_else(|| code.to_owned())
}

fn text_of(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

fn ja_nee(value: bool) -> &'static str {
    if value { "ja" } else { "nee" }
}

// ---------------------------------------------------------------- grootboekrekeningen

/// `POST grootboekrekening`, as the form sends it for a Dutch administration: omschrijving, code,
/// rekeningCategorie, inactief and, when a group is chosen, rekeningGroepCode.
pub(super) fn grootboekrekening_nieuw(
    args: &GrootboekrekeningToevoegenArgs,
    rekeningen: &[GridRekening],
    categorieen: &[Categorie],
    groepen: &[Groep],
) -> Result<Built, String> {
    let code = required(&args.code, "De code", CODE_MAX)?;
    let omschrijving = required(&args.omschrijving, "De omschrijving", OMSCHRIJVING_MAX)?;
    if let Some(taken) = rekeningen.iter().find(|r| r.code == code) {
        return Err(format!(
            "Er is al een grootboekrekening met code {code}: {}",
            taken.omschrijving
        ));
    }
    // The form starts on Winst en Verlies (VW).
    let categorie = pick_categorie(categorieen, args.categorie.as_deref().unwrap_or("VW"))?;
    let groep = pick_groep(groepen, args.groep.as_deref().unwrap_or_default())?;
    let mut body = Map::new();
    body.insert("omschrijving".into(), json!(omschrijving));
    body.insert("code".into(), json!(code));
    body.insert("rekeningCategorie".into(), json!(categorie.code));
    body.insert("inactief".into(), json!(false));
    let mut lines = vec![
        line("Code", &code),
        line("Omschrijving", &omschrijving),
        line("Categorie", categorie_label(categorie)),
    ];
    if let Some(groep) = groep {
        body.insert("rekeningGroepCode".into(), json!(groep.code));
        lines.push(line("Groep", groep_label(groep)));
    }
    Ok(Built {
        body: Value::Object(body),
        title: format!("Grootboekrekening toevoegen: {code} {omschrijving}"),
        lines,
    })
}

/// `PUT grootboekrekening`: the fields the form sends, from the rekening as it is now with the named changes.
/// The second value says whether the change always asks: a new code or category moves every booking on the
/// rekening along with it. `rgs`: the administration uses RGS codes (the setting the form reads, `hasRgs`).
pub(super) fn grootboekrekening_gewijzigd(
    args: &GrootboekrekeningWijzigenArgs,
    id: u64,
    now: &Value,
    rekeningen: &[GridRekening],
    categorieen: &[Categorie],
    groepen: &[Groep],
    rgs: bool,
) -> Result<(Built, bool), String> {
    if !now.is_object() {
        return Err("e-Boekhouden gaf deze grootboekrekening in een onbekende vorm terug".into());
    }
    let code_was = text_of(now, "code");
    let omschrijving_was = text_of(now, "omschrijving");
    let categorie_was = text_of(now, "rekeningCategorie");
    let groep_was = text_of(now, "rekeningGroepCode");
    let inactief_was = now
        .get("inactief")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let mut lines = vec![line("Rekening", format!("{code_was} {omschrijving_was}"))];
    let mut change = |label: &str, was: String, wordt: String| {
        lines.push(line(&format!("{label} was"), was));
        lines.push(line(&format!("{label} wordt"), wordt));
    };
    let mut serious = false;

    let mut code = code_was.clone();
    if let Some(nieuw) = &args.nieuwe_code {
        let nieuw = required(nieuw, "De code", CODE_MAX)?;
        if nieuw != code_was {
            if let Some(taken) = rekeningen.iter().find(|r| r.code == nieuw && r.id != id) {
                return Err(format!(
                    "Er is al een grootboekrekening met code {nieuw}: {}",
                    taken.omschrijving
                ));
            }
            change("Code", code_was.clone(), nieuw.clone());
            serious = true;
            code = nieuw;
        }
    }
    let mut omschrijving = omschrijving_was.clone();
    if let Some(nieuw) = &args.omschrijving {
        let nieuw = required(nieuw, "De omschrijving", OMSCHRIJVING_MAX)?;
        if nieuw != omschrijving_was {
            change("Omschrijving", omschrijving_was.clone(), nieuw.clone());
            omschrijving = nieuw;
        }
    }
    let mut categorie = categorie_was.clone();
    // Naming the category the rekening has already is no change, also when it is one the form keeps fixed.
    let huidige = |wanted: &str| {
        let wanted = wanted.trim();
        wanted.eq_ignore_ascii_case(&categorie_was)
            || categorieen
                .iter()
                .any(|c| c.code == categorie_was && c.omschrijving.eq_ignore_ascii_case(wanted))
    };
    if let Some(wanted) = args.categorie.as_deref().filter(|w| !huidige(w)) {
        let nieuw = pick_categorie(categorieen, wanted)?;
        if nieuw.code != categorie_was {
            if categorieen
                .iter()
                .any(|c| c.code == categorie_was && c.restricted)
            {
                return Err(format!(
                    "De categorie van deze rekening ({}) staat vast in e-Boekhouden en is niet te wijzigen",
                    categorie_naam(categorieen, &categorie_was)
                ));
            }
            change(
                "Categorie",
                categorie_naam(categorieen, &categorie_was),
                categorie_label(nieuw),
            );
            serious = true;
            categorie = nieuw.code.clone();
        }
    }
    let mut groep = groep_was.clone();
    if let Some(wanted) = &args.groep {
        let nieuw = pick_groep(groepen, wanted)?
            .map(|g| g.code.clone())
            .unwrap_or_default();
        if nieuw != groep_was {
            change(
                "Groep",
                groep_naam(groepen, &groep_was),
                groep_naam(groepen, &nieuw),
            );
            groep = nieuw;
        }
    }
    let mut inactief = inactief_was;
    if let Some(nieuw) = args.inactief
        && nieuw != inactief_was
    {
        change(
            "Inactief",
            ja_nee(inactief_was).to_owned(),
            ja_nee(nieuw).to_owned(),
        );
        inactief = nieuw;
    }
    if lines.len() == 1 {
        return Err("Er is niets gewijzigd: geef aan wat er anders moet".into());
    }
    if serious {
        lines.push(line(
            "Let op",
            format!(
                "een andere code of categorie geldt ook voor alle boekingen die al op deze rekening staan; balans en winst-en-verliesrekening veranderen daardoor mee. Het is een ingrijpende wijziging, {ALTIJD}."
            ),
        ));
    }

    let mut body = Map::new();
    body.insert("omschrijving".into(), json!(omschrijving));
    body.insert("code".into(), json!(code));
    body.insert("rekeningCategorie".into(), json!(categorie));
    body.insert("inactief".into(), json!(inactief));
    if !groep.is_empty() {
        body.insert("rekeningGroepCode".into(), json!(groep));
    }
    // As the form: rgscodeId only for an administration with RGS, sent back as it was read. The Belgian fields
    // (inkoopklasse, uitsluitenBtwAangifte) go only on the .be domain, and EB MCP works on e-boekhouden.nl.
    if rgs {
        body.insert(
            "rgscodeId".into(),
            now.get("rgscodeId").cloned().unwrap_or(Value::Null),
        );
    }
    body.insert("id".into(), json!(id));
    Ok((
        Built {
            body: Value::Object(body),
            title: format!("Grootboekrekening wijzigen: {code_was} {omschrijving_was}"),
            lines,
        },
        serious,
    ))
}

/// The approval of `DELETE grootboekrekening/{id}`.
pub(super) fn grootboekrekening_weg(now: &Value, categorieen: &[Categorie]) -> Built {
    let code = text_of(now, "code");
    let omschrijving = text_of(now, "omschrijving");
    Built {
        body: Value::Null,
        title: format!("Grootboekrekening verwijderen: {code} {omschrijving}"),
        lines: vec![
            line("Rekening", format!("{code} {omschrijving}")),
            line(
                "Categorie",
                categorie_naam(categorieen, &text_of(now, "rekeningCategorie")),
            ),
            line(
                "Gevolg",
                "de rekening verdwijnt uit het rekeningschema; dit is niet terug te draaien. Staan er nog boekingen op, dan kan e-Boekhouden weigeren.",
            ),
        ],
    }
}

// ---------------------------------------------------------------- rekeninggroepen

/// `POST rekeninggroep` with code and naam, as the form sends them.
pub(super) fn rekeninggroep_nieuw(
    args: &RekeninggroepToevoegenArgs,
    groepen: &[Groep],
) -> Result<Built, String> {
    let code = required(&args.code, "De code", GROEP_MAX)?;
    let naam = required(&args.naam, "De naam", GROEP_MAX)?;
    if let Some(taken) = groepen.iter().find(|g| g.code == code) {
        return Err(format!(
            "Er is al een rekeninggroep met code {code}: {}",
            taken.naam
        ));
    }
    Ok(Built {
        body: json!({"code": code, "naam": naam}),
        title: format!("Rekeninggroep toevoegen: {code} {naam}"),
        lines: vec![line("Code", &code), line("Naam", &naam)],
    })
}

/// `PUT rekeninggroep` with code, naam and id. A new code always asks: the rekeningen of the group name it by
/// its code (`rekeningGroepCode`).
pub(super) fn rekeninggroep_gewijzigd(
    args: &RekeninggroepWijzigenArgs,
    now: &Value,
    groepen: &[Groep],
    rekeningen: &[GridRekening],
) -> Result<(Built, bool), String> {
    let was = groep_of(now, args.id)?;
    let mut lines = vec![line("Groep", groep_label(&was))];
    let mut serious = false;
    let mut code = was.code.clone();
    if let Some(nieuw) = &args.code {
        let nieuw = required(nieuw, "De code", GROEP_MAX)?;
        if nieuw != was.code {
            if let Some(taken) = groepen.iter().find(|g| g.code == nieuw && g.id != args.id) {
                return Err(format!(
                    "Er is al een rekeninggroep met code {nieuw}: {}",
                    taken.naam
                ));
            }
            lines.push(line("Code was", &was.code));
            lines.push(line("Code wordt", &nieuw));
            serious = true;
            code = nieuw;
        }
    }
    let mut naam = was.naam.clone();
    if let Some(nieuw) = &args.naam {
        let nieuw = required(nieuw, "De naam", GROEP_MAX)?;
        if nieuw != was.naam {
            lines.push(line("Naam was", &was.naam));
            lines.push(line("Naam wordt", &nieuw));
            naam = nieuw;
        }
    }
    if lines.len() == 1 {
        return Err("Er is niets gewijzigd: geef aan wat er anders moet".into());
    }
    if serious {
        lines.push(in_groep_line(rekeningen, &was));
        lines.push(line(
            "Let op",
            format!(
                "de rekeningen van deze groep verwijzen naar de groep met zijn code, en de overzichten per groep tonen de nieuwe code; {ALTIJD}."
            ),
        ));
    }
    Ok((
        Built {
            body: json!({"code": code, "naam": naam, "id": args.id}),
            title: format!("Rekeninggroep wijzigen: {}", groep_label(&was)),
            lines,
        },
        serious,
    ))
}

/// The approval of `DELETE rekeninggroep/{id}`.
pub(super) fn rekeninggroep_weg(
    id: u64,
    now: &Value,
    rekeningen: &[GridRekening],
) -> Result<Built, String> {
    let groep = groep_of(now, id)?;
    Ok(Built {
        body: Value::Null,
        title: format!("Rekeninggroep verwijderen: {}", groep_label(&groep)),
        lines: vec![
            line("Groep", groep_label(&groep)),
            in_groep_line(rekeningen, &groep),
            line(
                "Gevolg",
                "de groep verdwijnt uit het rekeningschema; dit is niet terug te draaien",
            ),
        ],
    })
}

// ---------------------------------------------------------------- kostenplaatsen

/// The path of a kostenplaats as the tree builds it: the omschrijvingen from the top down, joined by ">".
fn pad_van(lijst: &[Kostenplaats], id: u64) -> String {
    let mut parts = Vec::new();
    let mut at = lijst.iter().find(|k| k.id == id);
    while let Some(k) = at {
        if parts.len() > lijst.len() {
            break;
        }
        parts.push(k.omschrijving.clone());
        at = (k.parent != 0)
            .then(|| lijst.iter().find(|p| p.id == k.parent))
            .flatten();
    }
    parts.reverse();
    parts.join(">")
}

/// An omschrijving as the kostenplaats form takes it: at most 50 characters, without < or >.
fn kostenplaats_omschrijving(value: &str) -> Result<String, String> {
    let value = required(value, "De omschrijving", KOSTENPLAATS_MAX)?;
    if value.contains(['<', '>']) {
        return Err("De omschrijving van een kostenplaats mag geen < of > bevatten".into());
    }
    Ok(value)
}

fn pad_onder(parent: &str, omschrijving: &str) -> Result<String, String> {
    let pad = if parent.is_empty() {
        omschrijving.to_owned()
    } else {
        format!("{parent}>{omschrijving}")
    };
    if pad.chars().count() > PAD_MAX {
        return Err("Het volledige pad van de kostenplaats is te lang".into());
    }
    Ok(pad)
}

/// Every kostenplaats below `top`, at any depth: under it in the tree, or with a path that starts with its path
/// (the web app treats each one whose path holds "<name>>" as gone with it). In the order of the list.
fn onder<'a>(lijst: &'a [Kostenplaats], top: &Kostenplaats) -> Vec<&'a Kostenplaats> {
    let prefix = format!("{}>", top.pad);
    let mut ids = vec![top.id];
    // Walk down the tree; a list never holds more levels than it has rows.
    for _ in 0..lijst.len() {
        let before = ids.len();
        for k in lijst {
            if !ids.contains(&k.id) && ids.contains(&k.parent) {
                ids.push(k.id);
            }
        }
        if ids.len() == before {
            break;
        }
    }
    lijst
        .iter()
        .filter(|k| k.id != top.id && (ids.contains(&k.id) || k.pad.starts_with(&prefix)))
        .collect()
}

fn find_kostenplaats(lijst: &[Kostenplaats], id: u64) -> Result<&Kostenplaats, String> {
    lijst
        .iter()
        .find(|k| k.id == id)
        .ok_or_else(|| format!("Geen kostenplaats met id {id} in deze administratie"))
}

/// `POST kostenplaats` with omschrijving, actief, volledigPad and parentKostenplaatsId, as the form sends it.
pub(super) fn kostenplaats_nieuw(
    args: &KostenplaatsToevoegenArgs,
    lijst: &[Kostenplaats],
) -> Result<Built, String> {
    let omschrijving = kostenplaats_omschrijving(&args.omschrijving)?;
    let (parent_id, parent_pad) = match args.onder_id.filter(|id| *id > 0) {
        Some(id) => {
            let parent = find_kostenplaats(lijst, id)?;
            if !parent.actief {
                return Err(
                    "Het is niet mogelijk om een nieuwe kostenplaats onder een inactieve kostenplaats toe te voegen"
                        .into(),
                );
            }
            (id, pad_van(lijst, id))
        }
        None => (0, String::new()),
    };
    let pad = pad_onder(&parent_pad, &omschrijving)?;
    let mut lines = vec![line("Kostenplaats", &pad)];
    if parent_id != 0 {
        lines.push(line("Onder", &parent_pad));
    }
    Ok(Built {
        body: json!({"omschrijving": omschrijving, "actief": true, "volledigPad": pad, "parentKostenplaatsId": parent_id}),
        title: format!("Kostenplaats toevoegen: {pad}"),
        lines,
    })
}

/// `PUT kostenplaats` with omschrijving, actief, volledigPad and id, as the form sends it.
pub(super) fn kostenplaats_gewijzigd(
    args: &KostenplaatsWijzigenArgs,
    lijst: &[Kostenplaats],
) -> Result<Built, String> {
    let now = find_kostenplaats(lijst, args.id)?;
    let pad_was = now.pad.clone();
    // As the form's getFullPath: the parent's own path (its volledigPad) with the new omschrijving after it.
    let parent_pad = if now.parent == 0 {
        String::new()
    } else {
        lijst
            .iter()
            .find(|k| k.id == now.parent)
            .map(|p| p.pad.clone())
            .unwrap_or_else(|| pad_van(lijst, now.parent))
    };
    let mut lines = vec![line("Kostenplaats", &pad_was)];
    let mut omschrijving = now.omschrijving.clone();
    if let Some(nieuw) = &args.omschrijving {
        let nieuw = kostenplaats_omschrijving(nieuw)?;
        if nieuw != now.omschrijving {
            lines.push(line("Omschrijving was", &now.omschrijving));
            lines.push(line("Omschrijving wordt", &nieuw));
            omschrijving = nieuw;
        }
    }
    let mut actief = now.actief;
    if let Some(inactief) = args.inactief
        && inactief == now.actief
    {
        lines.push(line("Inactief was", ja_nee(!now.actief)));
        lines.push(line("Inactief wordt", ja_nee(inactief)));
        actief = !inactief;
    }
    if lines.len() == 1 {
        return Err("Er is niets gewijzigd: geef aan wat er anders moet".into());
    }
    let pad = pad_onder(&parent_pad, &omschrijving)?;
    let onder = onder(lijst, now).len();
    if onder > 0 && omschrijving != now.omschrijving {
        lines.push(line(
            "Onderliggend",
            format!("{onder} kostenplaats(en) eronder; hun pad begint met deze omschrijving"),
        ));
    }
    Ok(Built {
        body: json!({"omschrijving": omschrijving, "actief": actief, "volledigPad": pad, "id": now.id}),
        title: format!("Kostenplaats wijzigen: {pad_was}"),
        lines,
    })
}

/// The approval of `DELETE kostenplaats/{id}`.
pub(super) fn kostenplaats_weg(id: u64, lijst: &[Kostenplaats]) -> Result<Built, String> {
    let now = find_kostenplaats(lijst, id)?;
    let pad = now.pad.clone();
    let mut lines: Vec<Line> = vec![line("Kostenplaats", &pad)];
    let onder: Vec<String> = onder(lijst, now).iter().map(|k| k.pad.clone()).collect();
    if onder.is_empty() {
        lines.push(line(
            "Gevolg",
            "de kostenplaats verdwijnt; dit is niet terug te draaien",
        ));
    } else {
        lines.push(line("Eronder", onder.join(", ")));
        lines.push(line(
            "Gevolg",
            format!(
                "de kostenplaats en de {} kostenplaats(en) eronder worden verwijderd; dit is niet terug te draaien",
                onder.len()
            ),
        ));
    }
    Ok(Built {
        body: Value::Null,
        title: format!("Kostenplaats verwijderen: {pad}"),
        lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write::kostenplaatsen_from_api;

    fn categorieen() -> Vec<Categorie> {
        categorieen_from(&json!([
            {"code": "BAL", "omschrijving": "Balans", "restricted": false},
            {"code": "VW", "omschrijving": "Winst & verlies", "restricted": false},
            {"code": "FIN", "omschrijving": "Financiële rekeningen", "restricted": true}
        ]))
        .unwrap()
    }

    fn groepen() -> Vec<Groep> {
        groepen_from(&json!([
            {"id": 7, "code": "KOST", "naam": "Kosten"},
            {"id": 8, "code": "OMZ", "naam": "Omzet"}
        ]))
        .unwrap()
    }

    fn rekeningen() -> Vec<GridRekening> {
        rekeningen_from(&json!({"data": [
            {"id": 11, "code": "1010", "omschrijving": "Bank", "categorie": "Financiële rekeningen", "inactief": false},
            {"id": 45, "code": "4550", "omschrijving": "Bankkosten", "categorie": "Winst & verlies", "rekeningGroep": "KOST", "inactief": false},
            {"id": 99, "code": "8999", "omschrijving": "Oud", "categorie": "Winst & verlies", "inactief": true}
        ]}))
        .unwrap()
    }

    fn shown(built: &Built) -> Vec<String> {
        built
            .lines
            .iter()
            .map(|l| format!("{}: {}", l.label, l.value))
            .collect()
    }

    fn toevoegen() -> GrootboekrekeningToevoegenArgs {
        GrootboekrekeningToevoegenArgs {
            code: " 4560 ".into(),
            omschrijving: " Telefoonkosten ".into(),
            categorie: None,
            groep: None,
            account: None,
        }
    }

    fn wijzigen() -> GrootboekrekeningWijzigenArgs {
        GrootboekrekeningWijzigenArgs {
            id: Some(45),
            code: None,
            nieuwe_code: None,
            omschrijving: None,
            categorie: None,
            groep: None,
            inactief: None,
            account: None,
        }
    }

    /// `GET grootboekrekening/45`, in the shape the form reads.
    fn bankkosten() -> Value {
        json!({"id": 45, "code": "4550", "omschrijving": "Bankkosten", "rekeningCategorie": "VW", "inactief": false,
               "rekeningGroepCode": "KOST", "rgscodeId": 1234, "inkoopklasse": null})
    }

    #[test]
    fn finds_a_rekening_by_id_or_code_and_reads_the_lists() {
        let rekeningen = rekeningen();
        assert_eq!(which_rekening(&rekeningen, None, Some(" 4550 ")), Ok(45));
        assert_eq!(
            which_rekening(&rekeningen, Some(99), None),
            Ok(99),
            "inactive ones too"
        );
        assert!(
            which_rekening(&rekeningen, Some(1), None)
                .unwrap_err()
                .contains("id 1")
        );
        assert!(
            which_rekening(&rekeningen, None, Some("1234"))
                .unwrap_err()
                .contains("1234")
        );
        assert!(which_rekening(&rekeningen, None, None).is_err());
        assert!(which_rekening(&rekeningen, Some(45), Some("4550")).is_err());
        assert!(
            rekeningen_from(&json!({"data": [[1, "1010"]]})).is_err(),
            "rows must carry their names"
        );
        assert!(categorieen_from(&json!({})).is_err());
        assert!(groepen_from(&json!(null)).is_err());
        assert!(categorieen()[2].restricted);
    }

    #[test]
    fn adds_a_rekening_as_the_form_does() {
        let built =
            grootboekrekening_nieuw(&toevoegen(), &rekeningen(), &categorieen(), &groepen())
                .unwrap();
        assert_eq!(
            built.body,
            json!({"omschrijving": "Telefoonkosten", "code": "4560", "rekeningCategorie": "VW", "inactief": false})
        );
        assert_eq!(
            built.title,
            "Grootboekrekening toevoegen: 4560 Telefoonkosten"
        );
        assert!(shown(&built).contains(&"Categorie: VW (Winst & verlies)".to_owned()));
        let mut args = toevoegen();
        args.categorie = Some("balans".into());
        args.groep = Some("Kosten".into());
        let built =
            grootboekrekening_nieuw(&args, &rekeningen(), &categorieen(), &groepen()).unwrap();
        assert_eq!(built.body["rekeningCategorie"], "BAL");
        assert_eq!(built.body["rekeningGroepCode"], "KOST");
        assert!(shown(&built).contains(&"Groep: KOST Kosten".to_owned()));
    }

    #[test]
    fn refuses_a_rekening_the_form_would_refuse() {
        let (r, c, g) = (rekeningen(), categorieen(), groepen());
        let mut args = toevoegen();
        args.code = "8999".into();
        assert!(
            grootboekrekening_nieuw(&args, &r, &c, &g)
                .unwrap_err()
                .contains("Oud"),
            "taken, even by an inactive one"
        );
        args.code = "12345678901".into();
        assert!(
            grootboekrekening_nieuw(&args, &r, &c, &g)
                .unwrap_err()
                .contains("10 tekens")
        );
        args.code = " ".into();
        assert!(
            grootboekrekening_nieuw(&args, &r, &c, &g)
                .unwrap_err()
                .contains("leeg")
        );
        let mut args = toevoegen();
        args.omschrijving = "x".repeat(101);
        assert!(
            grootboekrekening_nieuw(&args, &r, &c, &g)
                .unwrap_err()
                .contains("100 tekens")
        );
        let mut args = toevoegen();
        args.categorie = Some("KOSTEN".into());
        let unknown = grootboekrekening_nieuw(&args, &r, &c, &g).unwrap_err();
        assert!(
            unknown.contains("VW (Winst & verlies)") && !unknown.contains("FIN"),
            "{unknown}"
        );
        args.categorie = Some("FIN".into());
        assert!(
            grootboekrekening_nieuw(&args, &r, &c, &g)
                .unwrap_err()
                .contains("niet te kiezen")
        );
        let mut args = toevoegen();
        args.groep = Some("XYZ".into());
        assert!(
            grootboekrekening_nieuw(&args, &r, &c, &g)
                .unwrap_err()
                .contains("KOST Kosten")
        );
        assert!(
            grootboekrekening_nieuw(&args, &r, &c, &[])
                .unwrap_err()
                .contains("geen rekeninggroepen")
        );
    }

    #[test]
    fn an_edit_sends_the_form_back_with_only_the_named_change() {
        let (r, c, g) = (rekeningen(), categorieen(), groepen());
        let mut args = wijzigen();
        assert!(
            grootboekrekening_gewijzigd(&args, 45, &bankkosten(), &r, &c, &g, false)
                .unwrap_err()
                .contains("niets gewijzigd")
        );
        args.omschrijving = Some("Bank- en betaalkosten".into());
        // With RGS on, the form sends the rekening's RGS code back; never the Belgian fields.
        let (built, serious) =
            grootboekrekening_gewijzigd(&args, 45, &bankkosten(), &r, &c, &g, true).unwrap();
        assert!(!serious);
        assert_eq!(
            built.body,
            json!({"omschrijving": "Bank- en betaalkosten", "code": "4550", "rekeningCategorie": "VW", "inactief": false,
                   "rekeningGroepCode": "KOST", "rgscodeId": 1234, "id": 45})
        );
        let (built, _) =
            grootboekrekening_gewijzigd(&args, 45, &bankkosten(), &r, &c, &g, false).unwrap();
        assert!(built.body.get("rgscodeId").is_none(), "no RGS, no RGS code");
        assert!(built.body.get("inkoopklasse").is_none());
        assert_eq!(built.title, "Grootboekrekening wijzigen: 4550 Bankkosten");
        assert_eq!(
            shown(&built),
            [
                "Rekening: 4550 Bankkosten",
                "Omschrijving was: Bankkosten",
                "Omschrijving wordt: Bank- en betaalkosten"
            ]
        );
        // Out of its group, and inactive.
        let mut args = wijzigen();
        args.groep = Some(String::new());
        args.inactief = Some(true);
        let (built, serious) =
            grootboekrekening_gewijzigd(&args, 45, &bankkosten(), &r, &c, &g, false).unwrap();
        assert!(!serious);
        assert!(built.body.get("rekeningGroepCode").is_none());
        assert_eq!(built.body["inactief"], true);
        assert!(shown(&built).contains(&"Groep wordt: (geen)".to_owned()));
        assert!(shown(&built).contains(&"Inactief wordt: ja".to_owned()));
    }

    #[test]
    fn a_new_code_or_category_always_asks_and_says_why() {
        let (r, c, g) = (rekeningen(), categorieen(), groepen());
        let mut args = wijzigen();
        args.nieuwe_code = Some("4551".into());
        let (built, serious) =
            grootboekrekening_gewijzigd(&args, 45, &bankkosten(), &r, &c, &g, false).unwrap();
        assert!(serious);
        assert_eq!(built.body["code"], "4551");
        let lines = shown(&built);
        assert!(
            lines.contains(&"Code was: 4550".to_owned())
                && lines.contains(&"Code wordt: 4551".to_owned())
        );
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("Let op:") && l.contains("altijd akkoord")),
            "{lines:?}"
        );
        // The same code again is no change at all.
        args.nieuwe_code = Some("4550".into());
        assert!(grootboekrekening_gewijzigd(&args, 45, &bankkosten(), &r, &c, &g, false).is_err());
        args.nieuwe_code = Some("1010".into());
        assert!(
            grootboekrekening_gewijzigd(&args, 45, &bankkosten(), &r, &c, &g, false)
                .unwrap_err()
                .contains("Bank")
        );
        let mut args = wijzigen();
        args.categorie = Some("BAL".into());
        let (built, serious) =
            grootboekrekening_gewijzigd(&args, 45, &bankkosten(), &r, &c, &g, false).unwrap();
        assert!(serious);
        assert_eq!(built.body["rekeningCategorie"], "BAL");
        assert!(shown(&built).contains(&"Categorie was: VW (Winst & verlies)".to_owned()));
        // A rekening in a category the form keeps fixed cannot move out of it.
        let bank = json!({"id": 11, "code": "1010", "omschrijving": "Bank", "rekeningCategorie": "FIN", "inactief": false});
        assert!(
            grootboekrekening_gewijzigd(&args, 11, &bank, &r, &c, &g, false)
                .unwrap_err()
                .contains("staat vast")
        );
        assert!(grootboekrekening_gewijzigd(&args, 11, &json!([]), &r, &c, &g, false).is_err());
        // Naming the fixed category it has already is no change, by code or by name.
        for same in ["FIN", "financiële rekeningen"] {
            let mut args = wijzigen();
            args.categorie = Some(same.into());
            assert!(
                grootboekrekening_gewijzigd(&args, 11, &bank, &r, &c, &g, false)
                    .unwrap_err()
                    .contains("niets gewijzigd")
            );
            args.omschrijving = Some("Bank ING".into());
            let (built, serious) =
                grootboekrekening_gewijzigd(&args, 11, &bank, &r, &c, &g, false).unwrap();
            assert!(!serious);
            assert_eq!(built.body["rekeningCategorie"], "FIN");
        }
    }

    #[test]
    fn deleting_a_rekening_says_what_goes() {
        let built = grootboekrekening_weg(&bankkosten(), &categorieen());
        assert_eq!(
            built.title,
            "Grootboekrekening verwijderen: 4550 Bankkosten"
        );
        assert!(
            shown(&built)
                .iter()
                .any(|l| l.contains("niet terug te draaien"))
        );
        assert!(shown(&built).contains(&"Categorie: VW (Winst & verlies)".to_owned()));
    }

    #[test]
    fn groups_take_a_unique_code_and_a_new_code_always_asks() {
        let args = RekeninggroepToevoegenArgs {
            code: " REIS ".into(),
            naam: "Reiskosten".into(),
            account: None,
        };
        let built = rekeninggroep_nieuw(&args, &groepen()).unwrap();
        assert_eq!(built.body, json!({"code": "REIS", "naam": "Reiskosten"}));
        assert_eq!(built.title, "Rekeninggroep toevoegen: REIS Reiskosten");
        let taken = RekeninggroepToevoegenArgs {
            code: "KOST".into(),
            naam: "x".into(),
            account: None,
        };
        assert!(
            rekeninggroep_nieuw(&taken, &groepen())
                .unwrap_err()
                .contains("Kosten")
        );
        let empty = RekeninggroepToevoegenArgs {
            code: "X".into(),
            naam: " ".into(),
            account: None,
        };
        assert!(rekeninggroep_nieuw(&empty, &groepen()).is_err());

        let now = json!({"id": 7, "code": "KOST", "naam": "Kosten"});
        let mut args = RekeninggroepWijzigenArgs {
            id: 7,
            code: None,
            naam: Some("Kosten".into()),
            account: None,
        };
        assert!(
            rekeninggroep_gewijzigd(&args, &now, &groepen(), &rekeningen())
                .unwrap_err()
                .contains("niets gewijzigd")
        );
        args.naam = Some("Algemene kosten".into());
        let (built, serious) =
            rekeninggroep_gewijzigd(&args, &now, &groepen(), &rekeningen()).unwrap();
        assert!(!serious);
        assert_eq!(
            built.body,
            json!({"code": "KOST", "naam": "Algemene kosten", "id": 7})
        );
        args.code = Some("OMZ".into());
        assert!(
            rekeninggroep_gewijzigd(&args, &now, &groepen(), &rekeningen())
                .unwrap_err()
                .contains("Omzet")
        );
        args.code = Some("ALG".into());
        let (built, serious) =
            rekeninggroep_gewijzigd(&args, &now, &groepen(), &rekeningen()).unwrap();
        assert!(serious);
        assert_eq!(built.body["code"], "ALG");
        assert!(shown(&built).iter().any(|l| l.starts_with("Let op:")));
        assert!(
            shown(&built).contains(&"Rekeningen in deze groep: 1: 4550 Bankkosten".to_owned()),
            "{:?}",
            shown(&built)
        );
        let gone = rekeninggroep_weg(7, &now, &rekeningen()).unwrap();
        assert_eq!(gone.title, "Rekeninggroep verwijderen: KOST Kosten");
        assert!(shown(&gone).contains(&"Rekeningen in deze groep: 1: 4550 Bankkosten".to_owned()));
        let omzet = json!({"id": 8, "code": "OMZ", "naam": "Omzet"});
        let empty = rekeninggroep_weg(8, &omzet, &rekeningen()).unwrap();
        assert!(shown(&empty).contains(&"Rekeningen in deze groep: geen".to_owned()));
    }

    #[test]
    fn an_unknown_group_is_named_as_such() {
        assert_eq!(
            groep_bekend(&groepen(), 99).unwrap_err(),
            "Geen rekeninggroep met id 99 in deze administratie (zie rekeninggroepen)"
        );
        assert!(groep_bekend(&groepen(), 7).is_ok());
        assert!(
            groep_bekend(&[], 99).is_ok(),
            "a list without ids proves nothing"
        );
        // What GET rekeninggroep/{id} gives for an id it does not know.
        for now in [
            json!(null),
            json!({}),
            json!({"id": 8, "code": "OMZ", "naam": "Omzet"}),
        ] {
            assert!(
                rekeninggroep_weg(99, &now, &rekeningen())
                    .unwrap_err()
                    .contains("id 99")
            );
            let args = RekeninggroepWijzigenArgs {
                id: 99,
                code: None,
                naam: Some("x".into()),
                account: None,
            };
            assert!(
                rekeninggroep_gewijzigd(&args, &now, &groepen(), &rekeningen())
                    .unwrap_err()
                    .contains("id 99")
            );
        }
        // The group column may name the group by code, by naam or as the picker does.
        for groep in ["KOST", "kosten", "KOST - Kosten"] {
            let mut r = rekeningen();
            r[1].groep = groep.into();
            let kost = &groepen()[0];
            assert_eq!(in_groep(&r, kost).len(), 1, "{groep}");
        }
    }

    fn lijst() -> Vec<Kostenplaats> {
        kostenplaatsen_from_api(&json!([
            {"id": 501, "omschrijving": "Projecten", "parentKostenplaatsId": 0, "actief": true, "volledigPad": "Projecten"},
            {"id": 502, "omschrijving": "Noord", "parentKostenplaatsId": 501, "actief": true, "volledigPad": "Projecten>Noord"},
            {"id": 503, "omschrijving": "Oud", "parentKostenplaatsId": 0, "actief": false, "volledigPad": "Oud"},
            {"id": 505, "omschrijving": "Groningen", "parentKostenplaatsId": 502, "actief": true, "volledigPad": "Projecten>Noord>Groningen"}
        ]))
        .unwrap()
    }

    #[test]
    fn adds_a_kostenplaats_in_its_place_in_the_tree() {
        let top = KostenplaatsToevoegenArgs {
            omschrijving: " Zuid ".into(),
            onder_id: None,
            account: None,
        };
        let built = kostenplaats_nieuw(&top, &lijst()).unwrap();
        assert_eq!(
            built.body,
            json!({"omschrijving": "Zuid", "actief": true, "volledigPad": "Zuid", "parentKostenplaatsId": 0})
        );
        let under = KostenplaatsToevoegenArgs {
            omschrijving: "Groningen".into(),
            onder_id: Some(502),
            account: None,
        };
        let built = kostenplaats_nieuw(&under, &lijst()).unwrap();
        assert_eq!(
            built.body,
            json!({"omschrijving": "Groningen", "actief": true, "volledigPad": "Projecten>Noord>Groningen", "parentKostenplaatsId": 502})
        );
        assert_eq!(
            built.title,
            "Kostenplaats toevoegen: Projecten>Noord>Groningen"
        );
        assert!(shown(&built).contains(&"Onder: Projecten>Noord".to_owned()));
        let inactive = KostenplaatsToevoegenArgs {
            omschrijving: "X".into(),
            onder_id: Some(503),
            account: None,
        };
        assert!(
            kostenplaats_nieuw(&inactive, &lijst())
                .unwrap_err()
                .contains("inactieve")
        );
        let unknown = KostenplaatsToevoegenArgs {
            omschrijving: "X".into(),
            onder_id: Some(9),
            account: None,
        };
        assert!(
            kostenplaats_nieuw(&unknown, &lijst())
                .unwrap_err()
                .contains("id 9")
        );
        let bad = KostenplaatsToevoegenArgs {
            omschrijving: "a>b".into(),
            onder_id: None,
            account: None,
        };
        assert!(
            kostenplaats_nieuw(&bad, &lijst())
                .unwrap_err()
                .contains("<")
        );
        let long = KostenplaatsToevoegenArgs {
            omschrijving: "x".repeat(51),
            onder_id: None,
            account: None,
        };
        assert!(
            kostenplaats_nieuw(&long, &lijst())
                .unwrap_err()
                .contains("50 tekens")
        );
        // The full path has a limit of its own.
        let deep: Vec<Value> = (1..=11u64)
            .map(|i| json!({"id": i, "omschrijving": "y".repeat(48), "parentKostenplaatsId": i - 1, "actief": true}))
            .collect();
        let deep = kostenplaatsen_from_api(&Value::Array(deep)).unwrap();
        let args = KostenplaatsToevoegenArgs {
            omschrijving: "z".into(),
            onder_id: Some(11),
            account: None,
        };
        assert!(
            kostenplaats_nieuw(&args, &deep)
                .unwrap_err()
                .contains("te lang")
        );
    }

    #[test]
    fn an_edited_kostenplaats_keeps_its_place() {
        let mut args = KostenplaatsWijzigenArgs {
            id: 502,
            omschrijving: Some("Noord".into()),
            inactief: Some(false),
            account: None,
        };
        assert!(
            kostenplaats_gewijzigd(&args, &lijst())
                .unwrap_err()
                .contains("niets gewijzigd")
        );
        args.omschrijving = Some("Noord-Nederland".into());
        let built = kostenplaats_gewijzigd(&args, &lijst()).unwrap();
        assert_eq!(
            built.body,
            json!({"omschrijving": "Noord-Nederland", "actief": true, "volledigPad": "Projecten>Noord-Nederland", "id": 502})
        );
        assert_eq!(built.title, "Kostenplaats wijzigen: Projecten>Noord");
        let top = KostenplaatsWijzigenArgs {
            id: 501,
            omschrijving: Some("Werk".into()),
            inactief: Some(true),
            account: None,
        };
        let built = kostenplaats_gewijzigd(&top, &lijst()).unwrap();
        assert_eq!(
            built.body,
            json!({"omschrijving": "Werk", "actief": false, "volledigPad": "Werk", "id": 501})
        );
        let lines = shown(&built);
        assert!(
            lines.contains(&"Inactief wordt: ja".to_owned()),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.starts_with("Onderliggend: 2")),
            "{lines:?}"
        );
        let gone = KostenplaatsWijzigenArgs {
            id: 9,
            omschrijving: Some("x".into()),
            inactief: None,
            account: None,
        };
        assert!(kostenplaats_gewijzigd(&gone, &lijst()).is_err());
    }

    #[test]
    fn an_edit_builds_the_path_on_the_parents_own_path() {
        // The parent's stored path differs from its omschrijvingen up the tree: the stored one counts, as in the form.
        let lijst = kostenplaatsen_from_api(&json!([
            {"id": 601, "omschrijving": "Projecten", "parentKostenplaatsId": 0, "actief": true, "volledigPad": "Werk>Projecten"},
            {"id": 602, "omschrijving": "Noord", "parentKostenplaatsId": 601, "actief": true, "volledigPad": "Werk>Projecten>Noord"}
        ]))
        .unwrap();
        let args = KostenplaatsWijzigenArgs {
            id: 602,
            omschrijving: Some("Zuid".into()),
            inactief: None,
            account: None,
        };
        let built = kostenplaats_gewijzigd(&args, &lijst).unwrap();
        assert_eq!(built.body["volledigPad"], "Werk>Projecten>Zuid");
        assert_eq!(built.title, "Kostenplaats wijzigen: Werk>Projecten>Noord");
    }

    #[test]
    fn deleting_a_kostenplaats_names_what_is_under_it() {
        let built = kostenplaats_weg(501, &lijst()).unwrap();
        assert_eq!(built.title, "Kostenplaats verwijderen: Projecten");
        // Every level below, not only the first.
        assert!(
            shown(&built)
                .contains(&"Eronder: Projecten>Noord, Projecten>Noord>Groningen".to_owned()),
            "{:?}",
            shown(&built)
        );
        assert!(
            shown(&built).iter().any(|l| l.starts_with(
                "Gevolg: de kostenplaats en de 2 kostenplaats(en) eronder worden verwijderd"
            )),
            "{:?}",
            shown(&built)
        );
        let leaf = kostenplaats_weg(505, &lijst()).unwrap();
        assert!(!shown(&leaf).iter().any(|l| l.starts_with("Eronder")));
        // One whose path says it is below, though the tree does not link it.
        let mut loose = lijst();
        loose.push(Kostenplaats {
            id: 506,
            omschrijving: "Los".into(),
            parent: 0,
            actief: true,
            pad: "Projecten>Los".into(),
        });
        assert!(
            shown(&kostenplaats_weg(501, &loose).unwrap())
                .iter()
                .any(|l| l.contains("Projecten>Los"))
        );
        assert!(kostenplaats_weg(9, &lijst()).is_err());
    }
}
