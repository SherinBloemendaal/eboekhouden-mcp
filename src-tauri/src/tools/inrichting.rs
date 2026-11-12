//! Automatische importregels, vaste activa and the beginbalans.
//!
//! The tools of this part live here, on their own router (`inrichting_router`), which tools.rs adds to the
//! server's. Each tool is named in one of the lists below, which the tool-list test in mcp.rs reads.
//!
//! Every request shape comes from e-Boekhouden's own web app (the public Angular bundle, 2026-11-12):
//! - `importregels.rs`: the screens Boekhouden > Importeren > Automatische importregels (`app-importregels-list`,
//!   `app-importregels-form`) and the link "Alle regels opnieuw automatisch verwerken" (`app-import-list`).
//! - `vaste_activa.rs`: Beheer > Vaste activa (`app-vaste-activa-form`, the list with "Afschrijvingen boeken" and
//!   the Afschrijfschema of one asset, `app-vaste-activa-mutatie-list`).
//! - `beginbalans.rs`: Beheer > Beginbalans (`app-beginbalans-form`).
//!
//! This file holds what the three share: the router that joins theirs, the pick lists of grootboekrekeningen
//! and btw codes, dates and amounts.

use super::*;

mod beginbalans;
mod importregels;
mod vaste_activa;

#[cfg(test)]
mod e2e;

/// The tools of this part that only read.
#[cfg(test)]
pub(super) const READS: &[&str] = &[
    "importregels",
    "importregel",
    "vaste_activa",
    "vastactief",
    "vaste_activa_groepen",
    "afschrijvingsschema",
    "beginbalans",
];
/// The tools of this part that change the books.
#[cfg(test)]
pub(super) const WRITES: &[&str] = &[
    "importregel_toevoegen",
    "importregel_wijzigen",
    "importregel_verwijderen",
    "importregels_verwerken",
    "vastactief_toevoegen",
    "vastactief_wijzigen",
    "vastactief_verwijderen",
    "afschrijvingen_boeken",
    "beginbalans_wijzigen",
];
/// The ones in `WRITES` that cannot be undone.
#[cfg(test)]
pub(super) const DESTRUCTIVE: &[&str] = &[
    "importregel_verwijderen",
    "importregels_verwerken",
    "vastactief_verwijderen",
    "afschrijvingen_boeken",
];

impl EbMcp {
    /// The tools of this part: the routers of its three screens together.
    pub(super) fn inrichting_router() -> ToolRouter<Self> {
        Self::importregels_router() + Self::vaste_activa_router() + Self::beginbalans_router()
    }

    /// A pick list of grootboekrekeningen, as the screens load it (`grootboekrekening/search?...`,
    /// `grootboekrekening/selectlist/..`): the active ones only.
    async fn keuzelijst(
        &self,
        run: &Run,
        path: &str,
        query: Vec<(String, String)>,
    ) -> Result<Vec<Keuze>, String> {
        let list = self.fetch(run, path, query).await?;
        if !list.is_array() {
            return Err(format!(
                "e-Boekhouden gaf de lijst met grootboekrekeningen ({path}) in een onbekende vorm terug"
            ));
        }
        Ok(keuzes(&list))
    }

    /// The grootboekrekeningen of the categories `categorieen` (e-Boekhouden's codes, such as VW, BAL, BTWRC), the
    /// way the screens search them with an empty search term.
    async fn rekeningen_van(&self, run: &Run, categorieen: &str) -> Result<Vec<Keuze>, String> {
        self.keuzelijst(
            run,
            "grootboekrekening/search",
            vec![
                ("rekeningCategorie".to_owned(), categorieen.to_owned()),
                ("searchTerm".to_owned(), String::new()),
            ],
        )
        .await
    }

    /// A change on e-Boekhouden with a query string (`tools.rs`'s `change` sends none), after the approval.
    async fn change_with_query(
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
            return match call.json.unwrap_or(Value::Null) {
                Value::Bool(false) => Err(Halt::Error(
                    "e-Boekhouden meldt dat dit niet gelukt is".into(),
                )),
                done => Ok(done),
            };
        }
        Err(Halt::Error(call.error.unwrap_or_else(|| {
            format!("e-Boekhouden gaf {} op {path}", call.status)
        })))
    }
}

// ---------------------------------------------------------------- grootboekrekeningen and btw codes

/// A grootboekrekening from one of e-Boekhouden's pick lists: what a tool names (`code`) and what the API takes (`id`).
#[derive(Clone, Debug, PartialEq)]
struct Keuze {
    id: u64,
    code: String,
    naam: String,
}

impl Keuze {
    fn label(&self) -> String {
        format!("{} {}", self.code, self.naam)
    }
}

/// The active rekeningen of a pick list: `[{id, code, omschrijving, inactief?}]`.
fn keuzes(list: &Value) -> Vec<Keuze> {
    list.as_array()
        .map(|items| {
            items
                .iter()
                .filter(|item| {
                    !item
                        .get("inactief")
                        .is_some_and(|v| v.as_bool() == Some(true) || v.as_u64() == Some(1))
                })
                .filter_map(|item| {
                    Some(Keuze {
                        id: item.get("id")?.as_u64()?,
                        code: item.get("code")?.as_str()?.trim().to_owned(),
                        naam: item
                            .get("omschrijving")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .trim()
                            .to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The rekening with `code` in a pick list, or a refusal that says which kind of rekening was wanted.
fn kies<'a>(list: &'a [Keuze], code: &str, what: &str) -> Result<&'a Keuze, String> {
    let code = code.trim();
    if code.is_empty() {
        return Err(format!("Geef de code van de {what}"));
    }
    list.iter().find(|k| k.code == code).ok_or_else(|| {
        format!("Geen actieve grootboekrekening met code {code:?} die als {what} te kiezen is")
    })
}

/// How a rekening id reads in an approval: code and name when the pick list has it.
fn rekening_naam(list: &[Keuze], id: u64) -> String {
    list.iter()
        .find(|k| k.id == id)
        .map(Keuze::label)
        .unwrap_or_else(|| format!("rekening met id {id}"))
}

/// A btw code as `btwinfo/all` gives it.
#[derive(Clone, Debug, PartialEq)]
struct BtwCode {
    code: String,
    naam: String,
    /// "INK", "VERK" or "" (Geen btw).
    soort: String,
}

fn btw_codes(list: &Value) -> Vec<BtwCode> {
    list.as_array()
        .map(|codes| {
            codes
                .iter()
                .filter_map(|c| {
                    Some(BtwCode {
                        code: c.get("code")?.as_str()?.to_owned(),
                        naam: c
                            .get("omschrijving")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        soort: c
                            .get("soort")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------- dates, amounts, texts

type Date = (i32, u32, u32);

/// A date as the API gives it: "2026-11-06T00:00:00" or {"day":6,"month":11,"year":2026}.
fn date_of(value: &Value) -> Option<Date> {
    if let Some(text) = value.as_str() {
        return parse_date(text.get(..10)?).ok();
    }
    let part = |name: &str| value.get(name).and_then(Value::as_u64);
    let (y, m, d) = (part("year")?, part("month")?, part("day")?);
    let date = (i32::try_from(y).ok()?, m as u32, d as u32);
    parse_date(&dashed(date)).ok()
}

/// A date as e-Boekhouden's date picker sends it.
fn date_object((y, m, d): Date) -> Value {
    json!({"day": d, "month": m, "year": y})
}

/// "06-11-2026", as e-Boekhouden shows dates.
fn dutch((y, m, d): Date) -> String {
    format!("{d:02}-{m:02}-{y}")
}

fn round2(amount: f64) -> f64 {
    (amount * 100.0).round() / 100.0
}

/// An amount in euros, rounded to cents; a value no amount field takes is refused.
fn bedrag(value: f64, what: &str) -> Result<f64, String> {
    if !value.is_finite() || value.abs() > 1e11 {
        return Err(format!("{what} is geen geldig bedrag"));
    }
    Ok(round2(value))
}

/// A text argument without its surrounding spaces; nothing when it is empty.
fn tekst(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
}

/// A text with at most `max` characters, as the form allows.
fn kort(value: &str, max: usize, what: &str) -> Result<(), String> {
    let count = value.chars().count();
    if count > max {
        return Err(format!(
            "{what} mag hoogstens {max} tekens hebben (nu {count})"
        ));
    }
    Ok(())
}

fn ja_nee(value: bool) -> &'static str {
    if value { "ja" } else { "nee" }
}

/// The rows of a grid answer (`gridtable`) as objects.
fn rows_of(grid: Value) -> Vec<Value> {
    named_rows(grid)
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// What changed between the fields of a record before and after, as approval lines ("X was", "X wordt").
fn changes(
    before: &[(&'static str, String)],
    after: &[(&'static str, String)],
) -> Vec<approvals::Line> {
    let mut lines = Vec::new();
    for (label, new) in after {
        let old = before
            .iter()
            .find(|(l, _)| l == label)
            .map(|(_, v)| v.as_str())
            .unwrap_or("(leeg)");
        if old != new {
            lines.push(line(&format!("{label} was"), old));
            lines.push(line(&format!("{label} wordt"), new));
        }
    }
    for (label, old) in before {
        if !after.iter().any(|(l, _)| l == label) {
            lines.push(line(&format!("{label} was"), old));
            lines.push(line(&format!("{label} wordt"), "(leeg)"));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_pick_list_without_its_inactive_rekeningen() {
        let list = json!([
            {"id": 45, "code": "4550", "omschrijving": "Bankkosten"},
            {"id": 46, "code": " 4560 ", "omschrijving": "Rente", "inactief": false},
            {"id": 99, "code": "4999", "omschrijving": "Oud", "inactief": true},
            {"code": "zonder id"}
        ]);
        let found = keuzes(&list);
        assert_eq!(
            found,
            vec![
                Keuze {
                    id: 45,
                    code: "4550".into(),
                    naam: "Bankkosten".into()
                },
                Keuze {
                    id: 46,
                    code: "4560".into(),
                    naam: "Rente".into()
                },
            ]
        );
        assert_eq!(kies(&found, " 4560", "tegenrekening").unwrap().id, 46);
        assert!(
            kies(&found, "4999", "tegenrekening")
                .unwrap_err()
                .contains("4999")
        );
        assert!(kies(&found, "", "tegenrekening").is_err());
        assert_eq!(rekening_naam(&found, 45), "4550 Bankkosten");
        assert_eq!(rekening_naam(&found, 7), "rekening met id 7");
    }

    #[test]
    fn reads_dates_in_both_shapes_and_only_real_ones() {
        assert_eq!(date_of(&json!("2026-11-06T00:00:00")), Some((2026, 11, 6)));
        assert_eq!(
            date_of(&json!({"day": 6, "month": 11, "year": 2026})),
            Some((2026, 11, 6))
        );
        assert_eq!(date_of(&json!({"day": 31, "month": 2, "year": 2026})), None);
        assert_eq!(date_of(&Value::Null), None);
        assert_eq!(
            date_object((2026, 1, 2)),
            json!({"day": 2, "month": 1, "year": 2026})
        );
        assert_eq!(dutch((2026, 1, 2)), "02-01-2026");
    }

    #[test]
    fn checks_amounts_and_texts() {
        assert_eq!(bedrag(12.345, "Bedrag").unwrap(), 12.35);
        assert!(bedrag(f64::NAN, "Bedrag").is_err());
        assert!(bedrag(1e12, "Bedrag").is_err());
        assert_eq!(tekst(Some("  a ")), Some("a".into()));
        assert_eq!(tekst(Some("   ")), None);
        assert!(kort("é".repeat(50).as_str(), 50, "Omschrijving").is_ok());
        assert!(
            kort(&"x".repeat(51), 50, "Omschrijving")
                .unwrap_err()
                .contains("51")
        );
    }

    #[test]
    fn says_what_changed_between_two_records() {
        let before = vec![("Naam", "Oud".to_owned()), ("Weg", "x".to_owned())];
        let after = vec![("Naam", "Nieuw".to_owned()), ("Erbij", "y".to_owned())];
        let lines = changes(&before, &after);
        let pairs: Vec<(&str, &str)> = lines
            .iter()
            .map(|l| (l.label.as_str(), l.value.as_str()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("Naam was", "Oud"),
                ("Naam wordt", "Nieuw"),
                ("Erbij was", "(leeg)"),
                ("Erbij wordt", "y"),
                ("Weg was", "x"),
                ("Weg wordt", "(leeg)"),
            ]
        );
        assert!(changes(&before, &before).is_empty());
    }
}
