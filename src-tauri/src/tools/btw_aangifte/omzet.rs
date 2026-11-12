//! The btw-aangifte (omzetbelasting) and the suppletie: reading what was sent, the boxes and what lies behind
//! them, and drawing a return or a suppletie up in the books. Request shapes from e-Boekhouden's web app
//! (`app-btw-aangifte-form`, `app-suppletie-form`, `app-aangifte-list`, `app-btw-specificatie-list`,
//! `app-btw-aangifte-correctie-list`).

use super::*;

// ---------------------------------------------------------------- parameters

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AangifteIdArgs {
    /// Id van een verstuurde btw-aangifte (uit btwaangiftes, soort OB).
    pub id: u64,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// A box of the return whose mutaties the screen lists ("Specificatie btw rubriek").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub enum SpecificatieRubriek {
    #[serde(rename = "1a")]
    R1a,
    #[serde(rename = "1b")]
    R1b,
    #[serde(rename = "1c")]
    R1c,
    #[serde(rename = "2a")]
    R2a,
    #[serde(rename = "4a")]
    R4a,
    #[serde(rename = "4b")]
    R4b,
}

impl SpecificatieRubriek {
    fn code(self) -> &'static str {
        match self {
            Self::R1a => "1a",
            Self::R1b => "1b",
            Self::R1c => "1c",
            Self::R2a => "2a",
            Self::R4a => "4a",
            Self::R4b => "4b",
        }
    }
}

/// A box with corrections of earlier periods ("Correcties btw-aangifte").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub enum CorrectieRubriek {
    #[serde(rename = "1a")]
    R1a,
    #[serde(rename = "1b")]
    R1b,
    #[serde(rename = "1c")]
    R1c,
    #[serde(rename = "1e")]
    R1e,
    #[serde(rename = "2a")]
    R2a,
    #[serde(rename = "3a")]
    R3a,
    #[serde(rename = "3b")]
    R3b,
    #[serde(rename = "3c")]
    R3c,
    #[serde(rename = "4a")]
    R4a,
    #[serde(rename = "4b")]
    R4b,
    #[serde(rename = "5b")]
    R5b,
}

impl CorrectieRubriek {
    fn code(self) -> &'static str {
        match self {
            Self::R1a => "1a",
            Self::R1b => "1b",
            Self::R1c => "1c",
            Self::R1e => "1e",
            Self::R2a => "2a",
            Self::R3a => "3a",
            Self::R3b => "3b",
            Self::R3c => "3c",
            Self::R4a => "4a",
            Self::R4b => "4b",
            Self::R5b => "5b",
        }
    }
}

/// The name of a box of the return, as the Belastingdienst calls it.
fn rubriek_naam(code: &str) -> &'static str {
    match code {
        "1a" => "Leveringen/diensten belast met hoog tarief",
        "1b" => "Leveringen/diensten belast met laag tarief",
        "1c" => "Leveringen/diensten belast met overige tarieven, behalve 0%",
        "1d" => "Privégebruik",
        "1e" => "Leveringen/diensten belast met 0% of niet bij u belast",
        "2a" => "Leveringen/diensten waarbij de omzetbelasting naar u is verlegd",
        "3a" => "Leveringen naar landen buiten de EU",
        "3b" => "Leveringen naar of diensten in landen binnen de EU",
        "3c" => "Installatie/afstandsverkopen binnen de EU",
        "4a" => "Leveringen/diensten uit landen buiten de EU",
        "4b" => "Leveringen/diensten uit landen binnen de EU",
        "5b" => "Voorbelasting",
        "5d" => "Vermindering volgens de kleineondernemersregeling",
        _ => "",
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct RubriekMutatiesArgs {
    /// Jaar van de aangifte.
    pub jaar: i32,
    /// maand, kwartaal (standaard) of jaar.
    pub periode_type: Option<PeriodeType>,
    /// Nummer van de maand (1 tot 12) of het kwartaal (1 tot 4). Niet nodig bij jaar.
    pub periode: Option<u32>,
    /// Rubriek: 1a, 1b, 1c, 2a, 4a of 4b.
    pub rubriek: SpecificatieRubriek,
    /// Aantal rijen, 1 tot 200 (standaard 50).
    pub limit: Option<u32>,
    /// Vanaf welke rij (standaard 0).
    pub offset: Option<u32>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CorrectiesArgs {
    /// Jaar van de aangifte.
    pub jaar: i32,
    /// maand, kwartaal (standaard) of jaar.
    pub periode_type: Option<PeriodeType>,
    /// Nummer van de maand (1 tot 12) of het kwartaal (1 tot 4). Niet nodig bij jaar.
    pub periode: Option<u32>,
    /// Rubriek: 1a, 1b, 1c, 1e, 2a, 3a, 3b, 3c, 4a, 4b of 5b.
    pub rubriek: CorrectieRubriek,
    /// Aantal rijen, 1 tot 200 (standaard 50).
    pub limit: Option<u32>,
    /// Vanaf welke rij (standaard 0).
    pub offset: Option<u32>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SuppletieArgs {
    /// Id van een verstuurde suppletie (uit suppleties, soort SUP).
    pub id: Option<u64>,
    /// Id van een verstuurde btw-aangifte (uit btwaangiftes, soort OB): geeft de suppletie die e-Boekhouden daarvoor voorstelt.
    pub aangifte_id: Option<u64>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// Own btw amounts per box, in whole euros, instead of e-Boekhouden's proposal; as the screen's input fields.
#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct BtwBedragen {
    /// Btw in rubriek 1a (hoog tarief), hele euro's.
    pub btw_1a: Option<i64>,
    /// Btw in rubriek 1b (laag tarief), hele euro's.
    pub btw_1b: Option<i64>,
    /// Btw in rubriek 1c (overige tarieven), hele euro's.
    pub btw_1c: Option<i64>,
    /// Btw in rubriek 1d (privégebruik), hele euro's.
    pub btw_1d: Option<i64>,
    /// Btw in rubriek 2a (verlegd naar u), hele euro's.
    pub btw_2a: Option<i64>,
    /// Btw in rubriek 4a (uit landen buiten de EU), hele euro's.
    pub btw_4a: Option<i64>,
    /// Btw in rubriek 4b (uit landen binnen de EU), hele euro's.
    pub btw_4b: Option<i64>,
    /// Voorbelasting, rubriek 5b, hele euro's.
    pub btw_5b: Option<i64>,
    /// Kleineondernemersregeling, rubriek 5d, hele euro's. Alleen voor aangiftes over jaren voor 2021.
    pub btw_5d: Option<i64>,
}

impl BtwBedragen {
    fn get(&self, code: &str) -> Option<i64> {
        match code {
            "1a" => self.btw_1a,
            "1b" => self.btw_1b,
            "1c" => self.btw_1c,
            "1d" => self.btw_1d,
            "2a" => self.btw_2a,
            "4a" => self.btw_4a,
            "4b" => self.btw_4b,
            "5b" => self.btw_5b,
            "5d" => self.btw_5d,
            _ => None,
        }
    }

    fn any(&self) -> bool {
        VAKKEN.iter().any(|vak| self.get(vak.code).is_some())
    }
}

/// Which amounts a return that was sent already is drawn up with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Bron {
    /// De bedragen van de aangifte zoals die verstuurd is.
    Verstuurd,
    /// Het voorstel van e-Boekhouden uit de boekingen, eventueel met eigen bedragen.
    Voorstel,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AangifteVerwerkenArgs {
    /// Jaar van de aangifte.
    pub jaar: i32,
    /// maand, kwartaal (standaard) of jaar.
    pub periode_type: Option<PeriodeType>,
    /// Nummer van de maand (1 tot 12) of het kwartaal (1 tot 4). Niet nodig bij jaar.
    pub periode: Option<u32>,
    /// verstuurd of voorstel. Standaard verstuurd als de aangifte over deze periode al verstuurd is, anders voorstel. Eigen bedragen kunnen alleen bij voorstel.
    pub bedragen_van: Option<Bron>,
    #[serde(flatten)]
    pub btw: BtwBedragen,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct VerschilNegerenArgs {
    /// Jaar van de aangifte waarin de correctie meetelt.
    pub jaar: i32,
    /// maand, kwartaal (standaard) of jaar.
    pub periode_type: Option<PeriodeType>,
    /// Nummer van de maand (1 tot 12) of het kwartaal (1 tot 4). Niet nodig bij jaar.
    pub periode: Option<u32>,
    /// Rubriek van de correctie: 1a, 1b, 1c, 1e, 2a, 3a, 3b, 3c, 4a, 4b of 5b.
    pub rubriek: CorrectieRubriek,
    /// periodCode van de rij in btwaangifte_correcties: de eerdere periode met het verschil.
    pub periode_code: String,
    /// true: het negeren opheffen, zodat het verschil weer meetelt. Standaard false: het verschil negeren.
    pub opheffen: Option<bool>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SuppletieVerwerkenArgs {
    /// Id van de verstuurde btw-aangifte die de suppletie corrigeert (uit btwaangiftes, soort OB).
    pub aangifte_id: u64,
    #[serde(flatten)]
    pub btw: BtwBedragen,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

// ---------------------------------------------------------------- the boxes a return books

/// A box that `btw-aangifte/verwerk` and `suppletie/verwerk` take: its code, its key in the request, and where
/// a return that was sent keeps it (`verzondenAangifte.<groep>.<veld>`).
struct Vak {
    code: &'static str,
    key: &'static str,
    groep: &'static str,
    veld: &'static str,
}

const VAKKEN: [Vak; 9] = [
    Vak {
        code: "1a",
        key: "s1aBtw",
        groep: "doorUBinnenland",
        veld: "omzetbelastingHoog",
    },
    Vak {
        code: "1b",
        key: "s1bBtw",
        groep: "doorUBinnenland",
        veld: "omzetbelastingLaag",
    },
    Vak {
        code: "1c",
        key: "s1cBtw",
        groep: "doorUBinnenland",
        veld: "omzetbelastingOverig",
    },
    Vak {
        code: "1d",
        key: "s1dBtw",
        groep: "doorUBinnenland",
        veld: "omzetbelastingPriveGebruik",
    },
    Vak {
        code: "2a",
        key: "s2aBtw",
        groep: "aanUBinnenland",
        veld: "omzetbelastingVerlegd",
    },
    Vak {
        code: "4a",
        key: "s4aBtw",
        groep: "aanUBuitenland",
        veld: "omzetbelastingBuitenEUInvoer",
    },
    Vak {
        code: "4b",
        key: "s4bBtw",
        groep: "aanUBuitenland",
        veld: "omzetbelastingGoederenBinnenEU",
    },
    Vak {
        code: "5b",
        key: "s5bBtw",
        groep: "berekening",
        veld: "voorbelasting",
    },
    Vak {
        code: "5d",
        key: "s5dBtw",
        groep: "berekening",
        veld: "kleineOndernemersregeling",
    },
];

/// The boxes as they will be booked: their amounts in the request and in the approval lines, and the balance
/// (5a minus 5b and 5d, which is 5g for a return and 5e for a suppletie).
struct Vakken {
    body: serde_json::Map<String, Value>,
    lines: Vec<approvals::Line>,
    saldo: f64,
}

/// The amounts per box. From the return sent when `verzonden` is given, otherwise e-Boekhouden's proposal
/// (`rubrieken[].btwVoorgesteld`) with the user's own amounts over it. The kleineondernemersregeling (5d)
/// counts only before 2021, as the screen's `periodYear < 2021`.
fn vakken(
    model: &Value,
    verzonden: Option<&Value>,
    eigen: &BtwBedragen,
    jaar: i32,
) -> Result<Vakken, String> {
    let rubrieken = model.get("rubrieken").and_then(Value::as_array).ok_or(
        "e-Boekhouden gaf de aangifte in een onbekende vorm terug: er staan geen rubrieken in",
    )?;
    let kor = jaar < 2021;
    if !kor && eigen.btw_5d.is_some() {
        return Err(
            "Rubriek 5d (kleineondernemersregeling) staat vanaf 2021 niet meer in de aangifte"
                .into(),
        );
    }
    let mut body = serde_json::Map::new();
    let mut lines = Vec::new();
    let mut saldo = 0.0;
    for vak in &VAKKEN {
        let voorstel = number(
            rubrieken
                .iter()
                .find(|r| r.get("rubriekCode").and_then(Value::as_str) == Some(vak.code))
                .and_then(|r| r.get("btwVoorgesteld")),
        )?;
        let own = eigen.get(vak.code);
        let value = if vak.code == "5d" && !kor {
            0.0
        } else if let Some(sent) = verzonden {
            number(sent.get(vak.groep).and_then(|g| g.get(vak.veld)))?
        } else {
            own.map(|v| v as f64).unwrap_or(voorstel)
        };
        body.insert(vak.key.into(), amount(value));
        if vak.code == "5d" && !kor {
            continue;
        }
        let mut shown = euro(value);
        if verzonden.is_none() && own.is_some() && (value - voorstel).abs() > f64::EPSILON {
            shown = format!("{shown} (voorstel van e-Boekhouden {})", euro(voorstel));
        }
        lines.push(line(
            &format!("{} {} (btw)", vak.code, rubriek_naam(vak.code)),
            shown,
        ));
        saldo += match vak.code {
            "5b" | "5d" => -value,
            _ => value,
        };
    }
    Ok(Vakken { body, lines, saldo })
}

fn euro(value: f64) -> String {
    write::euro(value)
}

/// The balance as the user reads it: what is paid or asked back.
fn saldo_line(label: &str, saldo: f64) -> approvals::Line {
    if saldo < 0.0 {
        line(label, format!("{} terug te vragen", euro(-saldo)))
    } else {
        line(label, format!("{} te betalen", euro(saldo)))
    }
}

/// What the screen says about a return at the Belastingdienst, by its `eaStatus`.
fn status_bij_belastingdienst(model: &Value) -> &'static str {
    match model.get("eaStatus") {
        Some(Value::String(s)) if s == "acknowledgement" => "verstuurd en geaccepteerd",
        Some(Value::String(s)) if s.is_empty() => "verstuurd, nog in behandeling",
        Some(Value::String(s)) if s == "error" => "verstuurd, maar niet geaccepteerd",
        // No status is also what a return filed outside e-Boekhouden has.
        _ => "niet via e-Boekhouden verstuurd",
    }
}

/// The request of "Btw-aangifte verwerken in de boekhouding" and its approval, from the screen's model of the
/// period (`btw-aangifte/aangifte`).
fn aangifte_verwerken(
    model: &Value,
    periode: BtwPeriode,
    bron: Option<Bron>,
    eigen: &BtwBedragen,
    vandaag: (i32, u32, u32),
) -> Result<Built, String> {
    let verzonden = model.get("verzondenAangifte").filter(|v| v.is_object());
    let bron = bron.unwrap_or(if verzonden.is_some() {
        Bron::Verstuurd
    } else {
        Bron::Voorstel
    });
    let sent = match bron {
        Bron::Verstuurd => {
            let Some(sent) = verzonden else {
                return Err(format!(
                    "Over {} is nog geen aangifte verstuurd; gebruik bedragen_van voorstel",
                    periode.naam()
                ));
            };
            if eigen.any() {
                return Err(
                    "Eigen bedragen kunnen alleen bij het voorstel van e-Boekhouden: geef bedragen_van voorstel mee"
                        .into(),
                );
            }
            Some(sent)
        }
        Bron::Voorstel => None,
    };
    let Vakken {
        mut body,
        lines: vak_lines,
        saldo,
    } = vakken(model, sent, eigen, periode.jaar)?;
    body.insert("periodYear".into(), json!(periode.jaar));
    body.insert("periodType".into(), json!(periode.soort));
    body.insert("periodId".into(), json!(periode.id));

    let mut lines = vec![
        line("Periode", periode.naam()),
        line(
            "Bedragen",
            match (bron, eigen.any()) {
                (Bron::Verstuurd, _) => "zoals de aangifte verstuurd is",
                (Bron::Voorstel, false) => "het voorstel van e-Boekhouden uit de boekingen",
                (Bron::Voorstel, true) => "het voorstel van e-Boekhouden, met eigen bedragen",
            },
        ),
    ];
    lines.extend(vak_lines);
    lines.push(saldo_line("Saldo (5g)", saldo));
    let verwerkt = text_of(model, "bhDatum");
    lines.push(line(
        "Al verwerkt",
        if verwerkt.is_empty() {
            "nee".to_owned()
        } else {
            format!("ja, op {verwerkt}. Dit verwerkt de aangifte opnieuw; EB MCP weet niet of e-Boekhouden de mutaties van toen vervangt of er nieuwe bij boekt. Kijk de mutaties daarna na.")
        },
    ));
    lines.push(line(
        "Bij de Belastingdienst",
        status_bij_belastingdienst(model),
    ));
    if !periode.over(vandaag) {
        lines.push(line("Let op", "deze periode is nog niet voorbij"));
    }
    lines.push(line(
        "Gevolg",
        "e-Boekhouden boekt de voorbelasting en de af te dragen btw over naar de rekening BTW R/C, met mutaties in de boekhouding. EB MCP kan dat niet terugdraaien. Er gaat niets naar de Belastingdienst: versturen doet de gebruiker zelf in e-Boekhouden.",
    ));
    Ok(Built {
        body: Value::Object(body),
        title: format!("Btw-aangifte verwerken: {}", periode.naam()),
        lines,
    })
}

/// The period of a suppletie, from the return it corrects (`verzondenAangifte.identificerendeGegevens`):
/// `aangiftetijdvak` 1 to 12 is a month, 21, 24, 27 and 30 the four quarters, 40 a year (no `periodId`, which
/// the screen then sends as 0). The web app's `getEAPeriode` names 22, 23, 25, 26, 28, 29, 31 and 32 as months
/// too, but its `loadPeriod` would book them as a quarter; those are refused rather than booked wrong.
fn suppletie_periode(model: &Value) -> Result<(i32, &'static str, u32, String), String> {
    let ident = model
        .get("verzondenAangifte")
        .and_then(|v| v.get("identificerendeGegevens"))
        .ok_or("e-Boekhouden gaf de suppletie zonder de aangifte die hij corrigeert")?;
    let jaar = number(ident.get("aangiftejaar"))? as i32;
    let tijdvak = number(ident.get("aangiftetijdvak"))? as u32;
    if !(1990..=2100).contains(&jaar) {
        return Err("e-Boekhouden gaf de suppletie zonder geldig jaar".into());
    }
    match tijdvak {
        1..=12 => Ok((
            jaar,
            "M",
            tijdvak,
            format!("{} {jaar}", MAANDEN[(tijdvak - 1) as usize]),
        )),
        21 | 24 | 27 | 30 => {
            let kwartaal = (tijdvak - 21) / 3 + 1;
            Ok((jaar, "K", kwartaal, format!("{kwartaal}e kwartaal {jaar}")))
        }
        22..=32 => Err(format!(
            "De aangifte die deze suppletie corrigeert heeft tijdvak {tijdvak}. Dat is geen maand of kwartaal dat EB MCP kan verwerken \
             (e-Boekhouden noemt het {} {jaar}); verwerk deze suppletie zelf in e-Boekhouden",
            MAANDEN[(tijdvak - 21) as usize]
        )),
        40 => Ok((jaar, "J", 0, format!("jaar {jaar}"))),
        _ => Err(format!(
            "e-Boekhouden gaf een tijdvak dat EB MCP niet kent: {tijdvak}"
        )),
    }
}

/// The request of "Suppletie verwerken in de boekhouding" and its approval, from `suppletie/getSuppletie`.
fn suppletie_verwerken(model: &Value, eigen: &BtwBedragen) -> Result<Built, String> {
    let (jaar, soort, id, naam) = suppletie_periode(model)?;
    let Vakken {
        mut body,
        lines: vak_lines,
        saldo,
    } = vakken(model, None, eigen, jaar)?;
    body.insert("periodYear".into(), json!(jaar));
    body.insert("periodType".into(), json!(soort));
    body.insert("periodId".into(), json!(id));
    let al = number(model.get("saldoAlAangegeven"))?;
    let mut lines = vec![
        line("Aangifte die gecorrigeerd wordt", &naam),
        line(
            "Bedragen",
            if eigen.any() {
                "het voorstel van e-Boekhouden, met eigen bedragen"
            } else {
                "het voorstel van e-Boekhouden uit de boekingen"
            },
        ),
    ];
    lines.extend(vak_lines);
    lines.push(saldo_line("Saldo (5e)", saldo));
    lines.push(saldo_line("Al aangegeven (5f)", al));
    lines.push(saldo_line("Saldo van de suppletie (5g)", saldo - al));
    let verwerkt = text_of(model, "bhDatum");
    lines.push(line(
        "Al verwerkt",
        if verwerkt.is_empty() {
            "nee".to_owned()
        } else {
            format!("ja, op {verwerkt}. Dit verwerkt de suppletie opnieuw; EB MCP weet niet of e-Boekhouden de mutaties van toen vervangt of er nieuwe bij boekt. Kijk de mutaties daarna na.")
        },
    ));
    lines.push(line(
        "Gevolg",
        "e-Boekhouden boekt de verschillen in voorbelasting en af te dragen btw over naar de rekening BTW R/C, met mutaties in de boekhouding. EB MCP kan dat niet terugdraaien. Er gaat niets naar de Belastingdienst: versturen doet de gebruiker zelf in e-Boekhouden.",
    ));
    Ok(Built {
        body: Value::Object(body),
        title: format!("Suppletie verwerken: {naam}"),
        lines,
    })
}

/// Whether a correction row is ignored ("Verschil negeren").
fn genegeerd(row: &Value) -> bool {
    match row.get("verschilGenegeerd") {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_i64() == Some(1),
        _ => false,
    }
}

/// The request of "Verschil negeren" (or of lifting it) for one row of the corrections, and its approval.
fn verschil_negeren(
    rows: &[Value],
    periode: BtwPeriode,
    rubriek: CorrectieRubriek,
    periode_code: &str,
    opheffen: bool,
) -> Result<Built, String> {
    let code = periode_code.trim();
    let row = rows
        .iter()
        .find(|row| text_of(row, "periodCode") == code)
        .ok_or_else(|| {
            format!(
                "In de correcties van rubriek {} voor {} staat geen periode {code:?}; zie btwaangifte_correcties",
                rubriek.code(),
                periode.naam()
            )
        })?;
    let ignored = genegeerd(row);
    let omzet = number(row.get("omzetVerschil"))?;
    let btw = number(row.get("btwVerschil"))?;
    if opheffen && !ignored {
        return Err("Dit verschil wordt niet genegeerd; er is niets op te heffen".into());
    }
    if !opheffen && ignored {
        return Err("Dit verschil wordt al genegeerd".into());
    }
    if !opheffen && omzet == 0.0 && btw == 0.0 {
        return Err("Deze periode heeft geen verschil om te negeren".into());
    }
    let body = json!({
        "category": rubriek.code(),
        "periodCode": row.get("periodCode").cloned().unwrap_or(Value::Null),
        "btw": row.get("btwVerschil").cloned().unwrap_or(json!(0)),
        "omzet": row.get("omzetVerschil").cloned().unwrap_or(json!(0)),
        "delete": ignored,
    });
    let eerdere = match text_of(row, "period") {
        p if p.is_empty() => code.to_owned(),
        p => p,
    };
    let lines = vec![
        line("Aangifte", periode.naam()),
        line(
            "Rubriek",
            format!("{} {}", rubriek.code(), rubriek_naam(rubriek.code())),
        ),
        line("Eerdere periode", &eerdere),
        line("Verschil omzet", euro(omzet)),
        line("Verschil btw", euro(btw)),
        line(
            "Gevolg",
            if opheffen {
                format!(
                    "het verschil telt weer mee als correctie in de aangifte over {}",
                    periode.naam()
                )
            } else {
                format!(
                    "het verschil telt niet meer mee als correctie in de aangifte over {}",
                    periode.naam()
                )
            },
        ),
    ];
    Ok(Built {
        body,
        title: format!(
            "{}: rubriek {}, {eerdere}",
            if opheffen {
                "Btw-verschil weer meetellen"
            } else {
                "Btw-verschil negeren"
            },
            rubriek.code()
        ),
        lines,
    })
}

// ---------------------------------------------------------------- the tools

#[tool_router(router = btwaangifte_omzet_router, vis = "pub(super)")]
impl EbMcp {
    #[tool(
        description = "De verstuurde aangiftes (het scherm Verstuurde aangiftes): btw-aangiftes (soort OB), suppleties (soort SUP) en ICP-opgaven, met datum, periode, status en betaalstatus. Het id opent een btw-aangifte met btwaangifte, een suppletie met suppletie en een ICP-opgave met icp_opgave.",
        annotations(title = "Verstuurde aangiftes", read_only_hint = true)
    )]
    async fn btwaangiftes(
        &self,
        Parameters(args): Parameters<ListArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const PATH: &str = "aangifte/gridtable";
        let run = match self.begin(&context, "btwaangiftes", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let query = match grid(args.limit, args.offset) {
            Ok(query) => query,
            Err(message) => return Ok(self.refuse(run, message)),
        };
        let result = self
            .btwaangifte_get(&run, PATH, query)
            .await
            .map(named_rows)
            .map(without_pay_link);
        Ok(self.end(run, PATH, result))
    }

    #[tool(
        description = "Eén verstuurde btw-aangifte, op id (uit btwaangiftes, soort OB): alle rubrieken zoals verstuurd, de gegevens van de verzending en of er een suppletie bij hoort. De aangifte zoals e-Boekhouden hem voor een periode berekent geeft de tool btw.",
        annotations(title = "Btw-aangifte", read_only_hint = true)
    )]
    async fn btwaangifte(
        &self,
        Parameters(args): Parameters<AangifteIdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let source = format!("aangifte/{}", args.id);
        let run = match self.begin(&context, "btwaangifte", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let result = async {
            let id = args.id;
            let aangifte = self
                .btwaangifte_get(&run, &format!("aangifte/{id}"), Vec::new())
                .await?;
            let omzetbelasting = self
                .btwaangifte_get(
                    &run,
                    &format!("aangifte/aangifteomzetbelasting/{id}"),
                    Vec::new(),
                )
                .await?;
            let suppletie = self
                .btwaangifte_get(
                    &run,
                    &format!("suppletie/getSuppletieDetails/{id}"),
                    Vec::new(),
                )
                .await?;
            Ok(json!({"aangifte": aangifte, "omzetbelasting": omzetbelasting, "suppletie": suppletie}))
        }
        .await;
        Ok(self.end(run, &source, result))
    }

    #[tool(
        description = "De mutaties achter één rubriek van de btw-aangifte over een periode (het scherm Specificatie btw rubriek): nummer, datum, omschrijving, relatie, factuur, bedragen en btw-code. Rubriek 1a, 1b, 1c, 2a, 4a of 4b; de voorbelasting (5b) staat in het grootboek.",
        annotations(title = "Mutaties achter een btw-rubriek", read_only_hint = true)
    )]
    async fn btwaangifte_mutaties(
        &self,
        Parameters(args): Parameters<RubriekMutatiesArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const PATH: &str = "mutatie/btw-specificatie/gridtable";
        let run = match self.begin(
            &context,
            "btwaangifte_mutaties",
            &args,
            args.account.as_deref(),
        ) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let query = (|| {
            let periode = BtwPeriode::new(args.jaar, args.periode_type.as_ref(), args.periode)?;
            let mut query = grid(args.limit, args.offset)?;
            query.push(("periodStart".into(), compact(periode.start())));
            query.push(("periodEnd".into(), compact(periode.end())));
            query.push(("category".into(), args.rubriek.code().into()));
            Ok::<_, String>(query)
        })();
        let query = match query {
            Ok(query) => query,
            Err(message) => return Ok(self.refuse(run, message)),
        };
        let result = self
            .btwaangifte_get(&run, PATH, query)
            .await
            .map(named_rows);
        Ok(self.end(run, PATH, result))
    }

    #[tool(
        description = "De correcties van eerdere perioden in één rubriek van de btw-aangifte (het scherm Correcties btw-aangifte): per eerdere periode wat toen is aangegeven, wat de boekhouding nu zegt, het verschil en of dat verschil genegeerd wordt. Met btwaangifte_verschil_negeren telt een verschil niet meer mee.",
        annotations(title = "Correcties btw-aangifte", read_only_hint = true)
    )]
    async fn btwaangifte_correcties(
        &self,
        Parameters(args): Parameters<CorrectiesArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const PATH: &str = "btw-aangifte/correcties/gridtable";
        let run = match self.begin(
            &context,
            "btwaangifte_correcties",
            &args,
            args.account.as_deref(),
        ) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let query = (|| {
            let periode = BtwPeriode::new(args.jaar, args.periode_type.as_ref(), args.periode)?;
            let mut query = grid(args.limit, args.offset)?;
            query.push(("category".into(), args.rubriek.code().into()));
            query.extend(periode.query());
            Ok::<_, String>(query)
        })();
        let query = match query {
            Ok(query) => query,
            Err(message) => return Ok(self.refuse(run, message)),
        };
        let result = self
            .btwaangifte_get(&run, PATH, query)
            .await
            .map(named_rows);
        Ok(self.end(run, PATH, result))
    }

    #[tool(
        description = "De verstuurde suppleties (soort SUP in de verstuurde aangiftes), met datum, periode en status. Open er een met suppletie.",
        annotations(title = "Suppleties", read_only_hint = true)
    )]
    async fn suppleties(
        &self,
        Parameters(args): Parameters<AccountArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let run = match self.begin(&context, "suppleties", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let result = self.btwaangifte_sent(&run).await.map(|rows| {
            let rows: Vec<Value> = rows
                .into_iter()
                .filter(|row| Verzonden::of(row) == Verzonden::Suppletie)
                .collect();
            json!({"rowCount": rows.len(), "data": rows})
        });
        Ok(self.end(run, "aangifte/gridtable", result))
    }

    #[tool(
        description = "Een suppletie. Met id (uit suppleties, soort SUP): de verstuurde suppletie met alle rubrieken. Met aangifte_id (een verstuurde btw-aangifte uit btwaangiftes): of e-Boekhouden een suppletie toestaat, en zo ja de suppletie die het voorstelt, per rubriek wat verstuurd is, wat de boekhouding nu zegt en het voorstel. Geef id of aangifte_id.",
        annotations(title = "Suppletie", read_only_hint = true)
    )]
    async fn suppletie(
        &self,
        Parameters(args): Parameters<SuppletieArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let run = match self.begin(&context, "suppletie", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let (source, result) = match (args.id, args.aangifte_id) {
            (Some(id), None) => {
                let result = async {
                    let omzetbelasting = self
                        .btwaangifte_get(
                            &run,
                            &format!("aangifte/aangifteomzetbelasting/{id}"),
                            Vec::new(),
                        )
                        .await?;
                    let details = self
                        .btwaangifte_get(
                            &run,
                            &format!("suppletie/getSuppletieDetails/{id}"),
                            Vec::new(),
                        )
                        .await?;
                    Ok(json!({"omzetbelasting": omzetbelasting, "suppletie": details}))
                }
                .await;
                (format!("aangifte/aangifteomzetbelasting/{id}"), result)
            }
            (None, Some(id)) => {
                let result = async {
                    let mag = self
                        .btwaangifte_get(&run, &format!("suppletie/canSuppletie/{id}"), Vec::new())
                        .await?;
                    if mag.get("succes").and_then(Value::as_bool) != Some(true) {
                        return Ok(json!({"kan_suppletie": false, "melding": mag.get("melding").cloned().unwrap_or(Value::Null)}));
                    }
                    let voorstel = self
                        .btwaangifte_get(&run, &format!("suppletie/getSuppletie/{id}"), Vec::new())
                        .await?;
                    let details = self
                        .btwaangifte_get(
                            &run,
                            &format!("suppletie/getSuppletieDetails/{id}"),
                            Vec::new(),
                        )
                        .await?;
                    Ok(json!({"kan_suppletie": true, "voorstel": voorstel, "verstuurde_suppletie": details}))
                }
                .await;
                (format!("suppletie/getSuppletie/{id}"), result)
            }
            _ => {
                return Ok(self.refuse(
                    run,
                    "Geef id (een suppletie) of aangifte_id (een btw-aangifte), niet allebei"
                        .into(),
                ));
            }
        };
        Ok(self.end(run, &source, result))
    }

    #[tool(
        description = "Een btw-aangifte opmaken: hem verwerken in de boekhouding, zoals de knop Btw-aangifte verwerken in de boekhouding. e-Boekhouden boekt de voorbelasting en de af te dragen btw over naar de rekening BTW R/C en geeft de nummers van die mutaties. Standaard met het voorstel van e-Boekhouden uit de boekingen, of, als de aangifte al verstuurd is, met de bedragen zoals verstuurd; eigen bedragen per rubriek in hele euro's kunnen bij het voorstel, zoals in het scherm. Een aangifte die al verwerkt is, wordt zo opnieuw verwerkt, zoals het scherm dat aanbiedt; EB MCP weet niet of e-Boekhouden de eerdere mutaties dan vervangt of er nieuwe bij boekt. EB MCP kan het verwerken niet terugdraaien. Dit verstuurt niets naar de Belastingdienst: versturen, betalen en indienen doet de gebruiker zelf in e-Boekhouden. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet. De gebruiker moet dit altijd goedkeuren in EB MCP, met de periode en de bedragen per rubriek. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Btw-aangifte verwerken",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn btwaangifte_verwerken(
        &self,
        Parameters(args): Parameters<AangifteVerwerkenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "btwaangifte_verwerken";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Btw-aangifte verwerkt in de boekhouding; data zijn de nummers van de mutaties. Er is niets naar de Belastingdienst gestuurd.",
            move |this, run, args| async move {
                let periode = BtwPeriode::new(args.jaar, args.periode_type.as_ref(), args.periode)?;
                let model = this
                    .btwaangifte_get(&run, "btw-aangifte/aangifte", periode.query())
                    .await?;
                let built = aangifte_verwerken(&model, periode, args.bedragen_van, &args.btw, today())?;
                this.approve(&run, TOOL, &built, true).await?;
                this.btwaangifte_send(&run, "POST", "btw-aangifte/verwerk", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een verschil in de correcties van de btw-aangifte negeren, of dat negeren opheffen, zoals de link Verschil negeren in het scherm Correcties btw-aangifte. Een genegeerd verschil telt niet meer mee als correctie in de aangifte. Zoek de rij eerst op met btwaangifte_correcties en geef zijn periodCode. Dit verstuurt niets naar de Belastingdienst. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet. De gebruiker moet dit altijd goedkeuren in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Btw-verschil negeren",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn btwaangifte_verschil_negeren(
        &self,
        Parameters(args): Parameters<VerschilNegerenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "btwaangifte_verschil_negeren";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Gedaan; btwaangifte_correcties toont het verschil nu zoals gevraagd. Er is niets naar de Belastingdienst gestuurd.",
            move |this, run, args| async move {
                let periode = BtwPeriode::new(args.jaar, args.periode_type.as_ref(), args.periode)?;
                if args.periode_code.trim().is_empty() {
                    return Err(Halt::Error("Geef de periodCode van de rij uit btwaangifte_correcties".into()));
                }
                let mut query = vec![("category".to_owned(), args.rubriek.code().to_owned())];
                query.extend(periode.query());
                let rows = this
                    .btwaangifte_rows(&run, "btw-aangifte/correcties/gridtable", &query)
                    .await?;
                let built = verschil_negeren(
                    &rows,
                    periode,
                    args.rubriek,
                    &args.periode_code,
                    args.opheffen.unwrap_or(false),
                )?;
                this.approve(&run, TOOL, &built, true).await?;
                this.btwaangifte_send(&run, "POST", "btw-aangifte/correcties/toggleignore", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een suppletie opmaken: hem verwerken in de boekhouding, zoals de knop Suppletie verwerken in de boekhouding. De suppletie corrigeert een verstuurde btw-aangifte (aangifte_id uit btwaangiftes); e-Boekhouden boekt de verschillen over naar de rekening BTW R/C en geeft de nummers van die mutaties. Standaard met het voorstel van e-Boekhouden; eigen bedragen per rubriek in hele euro's kunnen, zoals in het scherm. Alleen als e-Boekhouden voor die aangifte een suppletie toestaat (zie suppletie met aangifte_id). Een suppletie die al verwerkt is, wordt zo opnieuw verwerkt; EB MCP weet niet of e-Boekhouden de eerdere mutaties dan vervangt of er nieuwe bij boekt. EB MCP kan het verwerken niet terugdraaien. Dit verstuurt niets naar de Belastingdienst: versturen doet de gebruiker zelf in e-Boekhouden. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet. De gebruiker moet dit altijd goedkeuren in EB MCP, met de periode en de bedragen per rubriek. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Suppletie verwerken",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn suppletie_verwerken(
        &self,
        Parameters(args): Parameters<SuppletieVerwerkenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "suppletie_verwerken";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Suppletie verwerkt in de boekhouding; data zijn de nummers van de mutaties. Er is niets naar de Belastingdienst gestuurd.",
            move |this, run, args| async move {
                let id = args.aangifte_id;
                let mag = this
                    .btwaangifte_get(&run, &format!("suppletie/canSuppletie/{id}"), Vec::new())
                    .await?;
                if mag.get("succes").and_then(Value::as_bool) != Some(true) {
                    let melding = text_of(&mag, "melding");
                    return Err(Halt::Error(if melding.is_empty() {
                        "e-Boekhouden staat voor deze aangifte geen suppletie toe".to_owned()
                    } else {
                        format!("e-Boekhouden staat voor deze aangifte geen suppletie toe: {melding}")
                    }));
                }
                let model = this
                    .btwaangifte_get(&run, &format!("suppletie/getSuppletie/{id}"), Vec::new())
                    .await?;
                let built = suppletie_verwerken(&model, &args.btw)?;
                this.approve(&run, TOOL, &built, true).await?;
                this.btwaangifte_send(&run, "POST", "suppletie/verwerk", Some(&built.body))
                    .await
            },
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kwartaal(jaar: i32, id: u32) -> BtwPeriode {
        BtwPeriode::new(jaar, Some(&PeriodeType::Kwartaal), Some(id)).unwrap()
    }

    /// The screen's model of a period, as `btw-aangifte/aangifte` gives it.
    fn model(sent: bool) -> Value {
        let mut model = json!({
            "periodDescription": "3e kwartaal 2026",
            "periodCode": "263",
            "eaStatus": null,
            "bhDatum": "",
            "rubrieken": [
                {"rubriekCode": "1a", "btwVoorgesteld": 2100, "omzetVoorgesteld": 10000},
                {"rubriekCode": "1b", "btwVoorgesteld": 90, "omzetVoorgesteld": 1000},
                {"rubriekCode": "2a", "btwVoorgesteld": 0},
                {"rubriekCode": "4b", "btwVoorgesteld": 42},
                {"rubriekCode": "5b", "btwVoorgesteld": 630.4},
                {"rubriekCode": "5g", "btwVoorgesteld": 1601}
            ]
        });
        if sent {
            model["eaStatus"] = json!("acknowledgement");
            model["verzondenAangifte"] = json!({
                "doorUBinnenland": {"omzetbelastingHoog": "2000", "omzetbelastingLaag": 90},
                "aanUBinnenland": {"omzetbelastingVerlegd": 0},
                "aanUBuitenland": {"omzetbelastingBuitenEUInvoer": 0, "omzetbelastingGoederenBinnenEU": 42},
                "berekening": {"voorbelasting": 600, "kleineOndernemersregeling": 0}
            });
        }
        model
    }

    fn value_of<'a>(built: &'a Built, label: &str) -> &'a str {
        built
            .lines
            .iter()
            .find(|l| l.label == label)
            .map(|l| l.value.as_str())
            .unwrap_or_else(|| panic!("no line {label}: {:?}", built.lines))
    }

    #[test]
    fn draws_up_a_return_with_the_proposal_as_the_screen_sends_it() {
        let built = aangifte_verwerken(
            &model(false),
            kwartaal(2026, 3),
            None,
            &BtwBedragen::default(),
            (2026, 10, 2),
        )
        .unwrap();
        assert_eq!(
            built.body,
            json!({
                "s1aBtw": 2100, "s1bBtw": 90, "s1cBtw": 0, "s1dBtw": 0, "s2aBtw": 0,
                "s4aBtw": 0, "s4bBtw": 42, "s5bBtw": 630.4, "s5dBtw": 0,
                "periodYear": 2026, "periodType": "K", "periodId": 3
            })
        );
        assert_eq!(built.title, "Btw-aangifte verwerken: 3e kwartaal 2026");
        assert_eq!(value_of(&built, "Periode"), "3e kwartaal 2026");
        assert_eq!(
            value_of(
                &built,
                "1a Leveringen/diensten belast met hoog tarief (btw)"
            ),
            "2100,00"
        );
        assert_eq!(value_of(&built, "5b Voorbelasting (btw)"), "630,40");
        // 2100 + 90 + 42 - 630,40
        assert_eq!(value_of(&built, "Saldo (5g)"), "1601,60 te betalen");
        assert_eq!(value_of(&built, "Al verwerkt"), "nee");
        assert_eq!(
            value_of(&built, "Bij de Belastingdienst"),
            "niet via e-Boekhouden verstuurd"
        );
        assert!(value_of(&built, "Gevolg").contains("Er gaat niets naar de Belastingdienst"));
        assert!(!built.lines.iter().any(|l| l.label == "Let op"));
        // From 2021 on the kleineondernemersregeling is not in the return.
        assert!(!built.lines.iter().any(|l| l.label.starts_with("5d")));
    }

    #[test]
    fn own_amounts_replace_the_proposal_and_show_it() {
        let eigen = BtwBedragen {
            btw_1a: Some(2000),
            btw_5b: Some(700),
            ..Default::default()
        };
        let built = aangifte_verwerken(
            &model(false),
            kwartaal(2026, 3),
            None,
            &eigen,
            (2026, 10, 2),
        )
        .unwrap();
        assert_eq!(built.body["s1aBtw"], 2000);
        assert_eq!(built.body["s5bBtw"], 700);
        assert_eq!(built.body["s1bBtw"], 90);
        assert_eq!(
            value_of(
                &built,
                "1a Leveringen/diensten belast met hoog tarief (btw)"
            ),
            "2000,00 (voorstel van e-Boekhouden 2100,00)"
        );
        assert_eq!(value_of(&built, "Saldo (5g)"), "1432,00 te betalen");
        assert_eq!(
            value_of(&built, "Bedragen"),
            "het voorstel van e-Boekhouden, met eigen bedragen"
        );
        // More voorbelasting than btw: money back.
        let terug = BtwBedragen {
            btw_5b: Some(5000),
            ..Default::default()
        };
        let built = aangifte_verwerken(
            &model(false),
            kwartaal(2026, 3),
            None,
            &terug,
            (2026, 10, 2),
        )
        .unwrap();
        assert_eq!(value_of(&built, "Saldo (5g)"), "2768,00 terug te vragen");
    }

    #[test]
    fn a_return_sent_is_drawn_up_with_what_was_sent_unless_the_proposal_is_asked_for() {
        let mut sent = model(true);
        sent["bhDatum"] = json!("05-10-2026 14:12");
        let built = aangifte_verwerken(
            &sent,
            kwartaal(2026, 3),
            None,
            &BtwBedragen::default(),
            (2026, 10, 6),
        )
        .unwrap();
        assert_eq!(
            built.body["s1aBtw"], 2000,
            "the text amount is read as a number"
        );
        assert_eq!(built.body["s5bBtw"], 600);
        assert_eq!(
            built.body["s1cBtw"], 0,
            "a missing amount is 0, as the screen's ||= 0"
        );
        assert_eq!(
            value_of(&built, "Bedragen"),
            "zoals de aangifte verstuurd is"
        );
        assert_eq!(
            value_of(&built, "Al verwerkt"),
            "ja, op 05-10-2026 14:12. Dit verwerkt de aangifte opnieuw; EB MCP weet niet of e-Boekhouden de mutaties van toen vervangt of er nieuwe bij boekt. Kijk de mutaties daarna na."
        );
        assert_eq!(
            value_of(&built, "Bij de Belastingdienst"),
            "verstuurd en geaccepteerd"
        );

        let eigen = BtwBedragen {
            btw_1a: Some(1),
            ..Default::default()
        };
        let refused =
            aangifte_verwerken(&sent, kwartaal(2026, 3), None, &eigen, (2026, 10, 6)).unwrap_err();
        assert!(refused.contains("bedragen_van voorstel"), "{refused}");
        let built = aangifte_verwerken(
            &sent,
            kwartaal(2026, 3),
            Some(Bron::Voorstel),
            &eigen,
            (2026, 10, 6),
        )
        .unwrap();
        assert_eq!(built.body["s1aBtw"], 1);
        assert_eq!(built.body["s5bBtw"], 630.4);

        let refused = aangifte_verwerken(
            &model(false),
            kwartaal(2026, 3),
            Some(Bron::Verstuurd),
            &BtwBedragen::default(),
            (2026, 10, 6),
        )
        .unwrap_err();
        assert!(refused.contains("nog geen aangifte verstuurd"), "{refused}");
    }

    #[test]
    fn the_kleineondernemersregeling_counts_before_2021_only() {
        let mut old = model(false);
        old["rubrieken"]
            .as_array_mut()
            .unwrap()
            .push(json!({"rubriekCode": "5d", "btwVoorgesteld": 300}));
        let built = aangifte_verwerken(
            &old,
            kwartaal(2020, 4),
            None,
            &BtwBedragen::default(),
            (2021, 2, 1),
        )
        .unwrap();
        assert_eq!(built.body["s5dBtw"], 300);
        assert_eq!(
            value_of(
                &built,
                "5d Vermindering volgens de kleineondernemersregeling (btw)"
            ),
            "300,00"
        );
        assert_eq!(value_of(&built, "Saldo (5g)"), "1301,60 te betalen");
        let built = aangifte_verwerken(
            &old,
            kwartaal(2026, 3),
            None,
            &BtwBedragen::default(),
            (2026, 10, 2),
        )
        .unwrap();
        assert_eq!(built.body["s5dBtw"], 0);
        let eigen = BtwBedragen {
            btw_5d: Some(10),
            ..Default::default()
        };
        assert!(
            aangifte_verwerken(&old, kwartaal(2026, 3), None, &eigen, (2026, 10, 2))
                .unwrap_err()
                .contains("5d")
        );
    }

    #[test]
    fn warns_when_the_period_is_not_over_and_refuses_an_odd_model() {
        let built = aangifte_verwerken(
            &model(false),
            kwartaal(2026, 4),
            None,
            &BtwBedragen::default(),
            (2026, 11, 12),
        )
        .unwrap();
        assert_eq!(
            value_of(&built, "Let op"),
            "deze periode is nog niet voorbij"
        );
        assert!(
            aangifte_verwerken(
                &json!({"fout": true}),
                kwartaal(2026, 3),
                None,
                &BtwBedragen::default(),
                (2026, 10, 2)
            )
            .unwrap_err()
            .contains("geen rubrieken")
        );
        let mut odd = model(false);
        odd["rubrieken"][0]["btwVoorgesteld"] = json!("veel");
        assert!(
            aangifte_verwerken(
                &odd,
                kwartaal(2026, 3),
                None,
                &BtwBedragen::default(),
                (2026, 10, 2)
            )
            .is_err()
        );
    }

    #[test]
    fn periods_as_the_btw_screens_take_them() {
        let maand = BtwPeriode::new(2026, Some(&PeriodeType::Maand), Some(2)).unwrap();
        assert_eq!((maand.start(), maand.end()), ((2026, 2, 1), (2026, 2, 28)));
        assert_eq!(maand.naam(), "februari 2026");
        let schrikkel = BtwPeriode::new(2028, Some(&PeriodeType::Maand), Some(2)).unwrap();
        assert_eq!(schrikkel.end(), (2028, 2, 29));
        let kwartaal = BtwPeriode::new(2026, None, Some(4)).unwrap();
        assert_eq!(
            kwartaal.query(),
            vec![
                ("periodYear".to_owned(), "2026".to_owned()),
                ("periodType".to_owned(), "K".to_owned()),
                ("periodId".to_owned(), "4".to_owned())
            ]
        );
        assert_eq!(
            (kwartaal.start(), kwartaal.end()),
            ((2026, 10, 1), (2026, 12, 31))
        );
        assert!(!kwartaal.over((2026, 12, 31)));
        assert!(kwartaal.over((2027, 1, 1)));
        let jaar = BtwPeriode::new(2025, Some(&PeriodeType::Jaar), None).unwrap();
        assert_eq!(
            (jaar.soort, jaar.id, jaar.naam()),
            ("J", 1, "jaar 2025".to_owned())
        );
        assert!(BtwPeriode::new(2026, None, None).is_err());
        assert!(BtwPeriode::new(2026, Some(&PeriodeType::Maand), Some(13)).is_err());
        assert!(BtwPeriode::new(1800, Some(&PeriodeType::Jaar), None).is_err());
    }

    fn suppletie_model() -> Value {
        json!({
            "periodDescription": "2e kwartaal 2026",
            "bhDatum": null,
            "saldoAlAangegeven": 1500,
            "verzondenAangifte": {"identificerendeGegevens": {"aangiftejaar": "2026", "aangiftetijdvak": "24"}},
            "rubrieken": [
                {"rubriekCode": "1a", "btwVoorgesteld": 2100},
                {"rubriekCode": "5b", "btwVoorgesteld": 400}
            ]
        })
    }

    #[test]
    fn draws_up_a_suppletie_for_the_period_of_the_return_it_corrects() {
        let built = suppletie_verwerken(&suppletie_model(), &BtwBedragen::default()).unwrap();
        assert_eq!(
            built.body,
            json!({
                "s1aBtw": 2100, "s1bBtw": 0, "s1cBtw": 0, "s1dBtw": 0, "s2aBtw": 0,
                "s4aBtw": 0, "s4bBtw": 0, "s5bBtw": 400, "s5dBtw": 0,
                "periodYear": 2026, "periodType": "K", "periodId": 2
            })
        );
        assert_eq!(built.title, "Suppletie verwerken: 2e kwartaal 2026");
        assert_eq!(value_of(&built, "Saldo (5e)"), "1700,00 te betalen");
        assert_eq!(value_of(&built, "Al aangegeven (5f)"), "1500,00 te betalen");
        assert_eq!(
            value_of(&built, "Saldo van de suppletie (5g)"),
            "200,00 te betalen"
        );
        let eigen = BtwBedragen {
            btw_1a: Some(1800),
            ..Default::default()
        };
        let built = suppletie_verwerken(&suppletie_model(), &eigen).unwrap();
        assert_eq!(built.body["s1aBtw"], 1800);
        assert_eq!(
            value_of(&built, "Saldo van de suppletie (5g)"),
            "100,00 terug te vragen"
        );

        let mut maand = suppletie_model();
        maand["verzondenAangifte"]["identificerendeGegevens"]["aangiftetijdvak"] = json!(7);
        let built = suppletie_verwerken(&maand, &BtwBedragen::default()).unwrap();
        assert_eq!(
            (
                built.body["periodType"].clone(),
                built.body["periodId"].clone()
            ),
            (json!("M"), json!(7))
        );
        let mut jaar = suppletie_model();
        jaar["verzondenAangifte"]["identificerendeGegevens"]["aangiftetijdvak"] = json!(40);
        let built = suppletie_verwerken(&jaar, &BtwBedragen::default()).unwrap();
        assert_eq!(
            (
                built.body["periodType"].clone(),
                built.body["periodId"].clone()
            ),
            (json!("J"), json!(0))
        );
        // B1: only 21, 24, 27 and 30 are quarters; the codes between are months the web app names, refused.
        for (tijdvak, kwartaal) in [(21, 1), (24, 2), (27, 3), (30, 4)] {
            let mut model = suppletie_model();
            model["verzondenAangifte"]["identificerendeGegevens"]["aangiftetijdvak"] =
                json!(tijdvak);
            let built = suppletie_verwerken(&model, &BtwBedragen::default()).unwrap();
            assert_eq!(built.body["periodId"], kwartaal, "{tijdvak}");
            assert_eq!(
                built.title,
                format!("Suppletie verwerken: {kwartaal}e kwartaal 2026")
            );
        }
        for (tijdvak, maand) in [
            (22, "februari"),
            (23, "maart"),
            (25, "mei"),
            (26, "juni"),
            (28, "augustus"),
            (29, "september"),
            (31, "november"),
            (32, "december"),
        ] {
            let mut model = suppletie_model();
            model["verzondenAangifte"]["identificerendeGegevens"]["aangiftetijdvak"] =
                json!(tijdvak.to_string());
            let refused = suppletie_verwerken(&model, &BtwBedragen::default()).unwrap_err();
            assert!(
                refused.contains(&format!("tijdvak {tijdvak}")) && refused.contains(maand),
                "{refused}"
            );
            assert!(refused.contains("zelf in e-Boekhouden"), "{refused}");
        }
        let mut vreemd = suppletie_model();
        vreemd["verzondenAangifte"]["identificerendeGegevens"]["aangiftetijdvak"] = json!(35);
        assert!(suppletie_verwerken(&vreemd, &BtwBedragen::default()).is_err());
        assert!(suppletie_verwerken(&json!({"rubrieken": []}), &BtwBedragen::default()).is_err());
    }

    fn correcties() -> Vec<Value> {
        vec![
            json!({"period": "1e kwartaal 2026", "periodCode": "261", "omzetVerschil": 500, "btwVerschil": 105, "verschilGenegeerd": false}),
            json!({"period": "2e kwartaal 2026", "periodCode": "262", "omzetVerschil": -200, "btwVerschil": -42, "verschilGenegeerd": true}),
            json!({"period": "4e kwartaal 2025", "periodCode": "254", "omzetVerschil": 0, "btwVerschil": 0, "verschilGenegeerd": false}),
        ]
    }

    #[test]
    fn ignores_a_difference_or_lifts_that_as_the_screen_sends_it() {
        let built = verschil_negeren(
            &correcties(),
            kwartaal(2026, 3),
            CorrectieRubriek::R1a,
            " 261 ",
            false,
        )
        .unwrap();
        assert_eq!(
            built.body,
            json!({"category": "1a", "periodCode": "261", "btw": 105, "omzet": 500, "delete": false})
        );
        assert_eq!(
            built.title,
            "Btw-verschil negeren: rubriek 1a, 1e kwartaal 2026"
        );
        assert_eq!(value_of(&built, "Verschil btw"), "105,00");
        assert!(value_of(&built, "Gevolg").contains("niet meer mee"));

        let built = verschil_negeren(
            &correcties(),
            kwartaal(2026, 3),
            CorrectieRubriek::R1a,
            "262",
            true,
        )
        .unwrap();
        assert_eq!(built.body["delete"], true);
        assert_eq!(built.body["btw"], -42);
        assert!(value_of(&built, "Gevolg").contains("weer mee"));

        let refuse = |code: &str, opheffen: bool| {
            verschil_negeren(
                &correcties(),
                kwartaal(2026, 3),
                CorrectieRubriek::R1a,
                code,
                opheffen,
            )
            .unwrap_err()
        };
        assert!(refuse("262", false).contains("al genegeerd"));
        assert!(refuse("261", true).contains("niets op te heffen"));
        assert!(refuse("254", false).contains("geen verschil"));
        assert!(refuse("199", false).contains("btwaangifte_correcties"));
    }

    #[test]
    fn tells_the_kinds_of_returns_sent_apart_as_the_screen_does() {
        let kind = |soort: &str| Verzonden::of(&json!({"soort": soort}));
        assert_eq!(kind("OB"), Verzonden::Aangifte);
        assert_eq!(kind("SUP"), Verzonden::Suppletie);
        assert_eq!(kind("ICP"), Verzonden::Icp);
        assert_eq!(kind("Btw"), Verzonden::Anders);
        assert_eq!(kind("Klantenlisting met omzet"), Verzonden::Anders);
    }
}
