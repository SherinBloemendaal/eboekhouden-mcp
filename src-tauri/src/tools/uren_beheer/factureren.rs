//! Invoicing hours: grouping the lines of the invoicing list per relatie as its screen does, making the
//! invoices (`POST uur/factureren`) and putting hours on an invoice that exists (`POST uur/bestaand/factureren`).
//! Like `bouw`, nothing here talks to e-Boekhouden.

use std::collections::{BTreeMap, HashMap};

use super::bouw::*;
use super::*;

/// The project of an hour line and the relatie its invoice goes to.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ProjectRelatie {
    pub relatie_id: u64,
    pub relatie: String,
}

/// The projects as `project/rel` lists them: id, naam, relatieId, relatieBedrijf.
pub(super) fn project_relaties(list: &Value) -> HashMap<u64, ProjectRelatie> {
    list.as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|p| {
                    Some((
                        id_of(p, "id")?,
                        ProjectRelatie {
                            relatie_id: id_of(p, "relatieId")?,
                            relatie: text_of(p, "relatieBedrijf"),
                        },
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What one invoice of the hours screen holds: one per relatie.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Groep {
    pub relatie_id: u64,
    pub relatie: String,
    pub uur_ids: Vec<u64>,
    /// The distinct project and activity pairs: each becomes one invoice line.
    pub factuurregels: usize,
    pub uren: f64,
    pub kilometers: f64,
    /// Hours times the hour tariff plus kilometres times the km tariff, as the list shows them.
    pub bedrag: f64,
}

/// The rows of `uur/factureren/gridtable` per relatie, the way the screen groups them before it invoices
/// (`bulkFactureren`: per relatieId of the project, the set of "project-activiteit"). Rows without a project or
/// activity, or with a project that is not known, come back apart.
pub(super) fn per_relatie(
    rows: &[Value],
    projecten: &HashMap<u64, ProjectRelatie>,
) -> (Vec<Groep>, Vec<u64>) {
    let mut groepen: BTreeMap<u64, (Groep, Vec<String>)> = BTreeMap::new();
    let mut los = Vec::new();
    for row in rows {
        let Some(id) = id_of(row, "id") else { continue };
        let activiteit = text_of(row, "activiteit");
        let project = id_of(row, "projectId").and_then(|p| projecten.get(&p));
        let Some(project) = project.filter(|_| !activiteit.is_empty()) else {
            los.push(id);
            continue;
        };
        let (groep, paren) = groepen.entry(project.relatie_id).or_insert_with(|| {
            (
                Groep {
                    relatie_id: project.relatie_id,
                    relatie: project.relatie.clone(),
                    uur_ids: Vec::new(),
                    factuurregels: 0,
                    uren: 0.0,
                    kilometers: 0.0,
                    bedrag: 0.0,
                },
                Vec::new(),
            )
        });
        let number = |key: &str| row.get(key).and_then(Value::as_f64).unwrap_or(0.0);
        let paar = format!("{}-{activiteit}", text_of(row, "project"));
        if !paren.contains(&paar) {
            paren.push(paar);
        }
        groep.uur_ids.push(id);
        groep.factuurregels = paren.len();
        groep.uren += number("aantalUren");
        groep.kilometers += number("aantalKilometers");
        groep.bedrag += number("aantalUren") * number("uurtarief")
            + number("aantalKilometers") * number("kilometertarief");
    }
    (groepen.into_values().map(|(g, _)| g).collect(), los)
}

/// The groups as the reading tool shows them.
pub(super) fn groepen_json(groepen: &[Groep]) -> Value {
    json!(
        groepen
            .iter()
            .map(|g| json!({
                "relatieId": g.relatie_id,
                "relatie": g.relatie,
                "uurIds": g.uur_ids,
                "factuurregels": g.factuurregels,
                "uren": (g.uren * 100.0).round() / 100.0,
                "kilometers": (g.kilometers * 100.0).round() / 100.0,
                "bedragVolgensTarief": (g.bedrag * 100.0).round() / 100.0,
            }))
            .collect::<Vec<_>>()
    )
}

/// The rows to invoice: all, or the ones named. A named id that is not among the rows is refused.
pub(super) fn kies_rijen(rows: Vec<Value>, ids: Option<&[u64]>) -> Result<Vec<Value>, String> {
    let Some(ids) = ids else {
        return Ok(rows);
    };
    if ids.is_empty() {
        return Err("Geef minstens één urenregel in uur_ids, of laat uur_ids weg".into());
    }
    let missing: Vec<String> = ids
        .iter()
        .filter(|id| !rows.iter().any(|r| id_of(r, "id") == Some(**id)))
        .map(u64::to_string)
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "Deze urenregels staan niet bij de nog niet gefactureerde uren van deze periode en filters: {}",
            missing.join(", ")
        ));
    }
    Ok(rows
        .into_iter()
        .filter(|r| id_of(r, "id").is_some_and(|id| ids.contains(&id)))
        .collect())
}

/// Making the invoices of the hours screen (`POST uur/factureren` with the ids of the chosen lines): one
/// invoice per relatie, with what each will hold and the settings that decide what happens to them.
pub(super) fn uren_factureren(
    rows: &[Value],
    projecten: &HashMap<u64, ProjectRelatie>,
    instellingen: &Value,
    sjabloon: &str,
) -> Result<Built, String> {
    if rows.is_empty() {
        return Err(
            "Er zijn geen nog niet gefactureerde uren die aan deze periode en filters voldoen"
                .into(),
        );
    }
    let (groepen, los) = per_relatie(rows, projecten);
    if !los.is_empty() {
        return Err(format!(
            "Deze urenregels hebben geen project met een relatie of geen activiteit, dus geen factuur om op te komen: {}",
            los.iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if let Some(groep) = groepen.iter().find(|g| g.factuurregels > MAX_FACTUURREGELS) {
        return Err(format!(
            "Je kunt maximaal {MAX_FACTUURREGELS} factuurregels op een factuur plaatsen; voor {} zouden het er {} worden. Kies minder uren.",
            groep.relatie, groep.factuurregels
        ));
    }
    let ids: Vec<u64> = rows.iter().filter_map(|r| id_of(r, "id")).collect();
    let flag = |key: &str| {
        ja_nee(
            instellingen
                .get(key)
                .and_then(Value::as_bool)
                .unwrap_or(false),
        )
    };
    let aantal_facturen = groepen.len();
    let mut lines = vec![line(
        "Facturen",
        format!(
            "{aantal_facturen} (één per relatie), met {} urenregels",
            ids.len()
        ),
    )];
    let totaal: f64 = groepen.iter().map(|g| g.bedrag).sum();
    for g in &groepen {
        lines.push(line(
            &format!("Aan {}", known(g.relatie.clone())),
            format!(
                "{} uur en {} km op {} factuurregel{}, {} volgens tarief",
                getal(g.uren),
                getal(g.kilometers),
                g.factuurregels,
                if g.factuurregels == 1 { "" } else { "s" },
                write::euro(g.bedrag)
            ),
        ));
    }
    lines.push(line("Totaal volgens tarief", write::euro(totaal)));
    lines.push(line(
        "Bedragen",
        "aantal maal het uur- of kilometertarief uit de lijst, zonder btw; e-Boekhouden rekent de factuur zelf uit",
    ));
    lines.push(line("Factuursjabloon", known(sjabloon.to_owned())));
    lines.push(line(
        "Betalingstermijn",
        instellingen
            .get("betalingsTermijn")
            .and_then(Value::as_u64)
            .map_or("onbekend".to_owned(), |t| format!("{t} dagen")),
    ));
    lines.push(line("Verwerken in de boekhouding", flag("verwerken")));
    lines.push(line(
        "Klaarzetten voor verzending per post",
        flag("perPost"),
    ));
    lines.push(line(
        "Klaarzetten voor verzending per e-mail",
        flag("perEmail"),
    ));
    lines.push(line("Incasseren", flag("incasseren")));
    lines.push(line(
        "Gevolg",
        "e-Boekhouden maakt de facturen meteen en de uren gelden daarna als gefactureerd. Dit is niet terug te draaien; een factuur weghalen kan daarna alleen met factuur_verwijderen.",
    ));
    Ok(Built {
        body: json!(ids),
        title: if aantal_facturen == 1 {
            format!("Uren factureren: 1 factuur aan {}", groepen[0].relatie)
        } else {
            format!("Uren factureren: {aantal_facturen} facturen")
        },
        lines,
    })
}

/// The invoice a number names, from `factuur/searchexisting` (factuurId, factuurNummer, relatieBedrijf,
/// factuurDatum): only an exact match on the number.
pub(super) fn factuur_bij_nummer(list: &Value, nummer: &str) -> Result<Value, String> {
    let nummer = nummer.trim();
    list.as_array()
        .and_then(|items| {
            items.iter().find(|f| {
                f.get("factuurNummer")
                    .and_then(Value::as_str)
                    .is_some_and(|n| n.trim().eq_ignore_ascii_case(nummer))
            })
        })
        .filter(|f| id_of(f, "factuurId").is_some())
        .cloned()
        .ok_or_else(|| format!("Geen bestaande factuur met nummer {nummer:?} gevonden"))
}

/// Hours added to an invoice that exists (`POST uur/bestaand/factureren` with `{uren, factuurId}`).
pub(super) fn uren_bij_factuur(
    factuur: &Value,
    uren: &[(u64, Value)],
    namen: &Namen,
) -> Result<Built, String> {
    let factuur_id = id_of(factuur, "factuurId").ok_or("De factuur heeft geen id")?;
    if uren.is_empty() {
        return Err("Geef minstens één urenregel in uur_ids".into());
    }
    let nummer = text_of(factuur, "factuurNummer");
    let datum = factuur
        .get("factuurDatum")
        .and_then(datum_of)
        .map_or("onbekend".to_owned(), nl_datum);
    let mut lines = vec![
        line("Factuur", &nummer),
        line("Relatie", known(text_of(factuur, "relatieBedrijf"))),
        line("Factuurdatum", datum),
    ];
    let (mut totaal_uren, mut totaal_km) = (0.0, 0.0);
    for (id, uur) in uren {
        let number = |key: &str| uur.get(key).and_then(Value::as_f64).unwrap_or(0.0);
        totaal_uren += number("aantalUren");
        totaal_km += number("aantalKilometers");
        lines.push(line(
            &format!("Urenregel {id}"),
            format!(
                "{}, {}, {}: {} uur, {} km",
                uur.get("datum")
                    .and_then(datum_of)
                    .map_or("onbekend".to_owned(), nl_datum),
                namen.project(id_of(uur, "projectId")),
                namen.activiteit(id_of(uur, "activiteitId")),
                getal(number("aantalUren")),
                getal(number("aantalKilometers"))
            ),
        ));
    }
    lines.push(line(
        "Samen",
        format!("{} uur en {} km", getal(totaal_uren), getal(totaal_km)),
    ));
    lines.push(line(
        "Gevolg",
        format!("de uren komen als regels op de bestaande factuur {nummer}, die daardoor verandert. Dit is niet terug te draaien."),
    ));
    let ids: Vec<u64> = uren.iter().map(|(id, _)| *id).collect();
    Ok(Built {
        body: json!({"uren": ids, "factuurId": factuur_id}),
        title: format!("Uren op factuur {nummer} zetten"),
        lines,
    })
}
