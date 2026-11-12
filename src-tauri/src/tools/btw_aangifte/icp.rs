//! The ICP-opgave: the opgaven sent, one opgave with its lines, and drawing one up: its details and the lines
//! the user adds himself (corrections and A-B-C supplies). Request shapes from e-Boekhouden's web app
//! (`app-icp-opgave`, `app-icp-opgave-regel-list`, `app-icp-opgave-regel-form`, `app-ea-aangifte-status-list`).

use super::*;

// ---------------------------------------------------------------- parameters

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct IcpOpgaveArgs {
    /// Id van een verstuurde ICP-opgave (uit icp_opgaven). Of geef jaar, periode_type en periode.
    pub id: Option<u64>,
    /// Jaar van de opgave, 2010 tot en met dit jaar.
    pub jaar: Option<i32>,
    /// maand, twee_maanden, kwartaal of jaar.
    pub periode_type: Option<IcpTijdvak>,
    /// Nummer van de maand (1 tot 12), de twee maanden (1 tot 4) of het kwartaal (1 tot 4). Niet nodig bij jaar.
    pub periode: Option<u32>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct IcpOpgaveWijzigenArgs {
    /// Jaar van de opgave, 2010 tot en met dit jaar.
    pub jaar: i32,
    /// maand, twee_maanden, kwartaal of jaar.
    pub periode_type: IcpTijdvak,
    /// Nummer van de maand (1 tot 12), de twee maanden (1 tot 4) of het kwartaal (1 tot 4). Niet nodig bij jaar.
    pub periode: Option<u32>,
    /// Nieuw OB-nummer (btw-nummer van de onderneming): letters, cijfers, + en *, hoogstens 50 tekens.
    pub ob_nummer: Option<String>,
    /// Nieuwe bedrijfsnaam, hoogstens 50 tekens.
    pub bedrijf: Option<String>,
    /// Nieuwe contactpersoon, hoogstens 35 tekens.
    pub contactpersoon: Option<String>,
    /// Nieuw telefoonnummer, hoogstens 14 tekens.
    pub telefoon: Option<String>,
    /// Nieuw onderdeel van de fiscale eenheid: letters, cijfers, + en *, hoogstens 50 tekens.
    pub onderdeel_fiscale_eenheid: Option<String>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// The rubrieken of an ICP-opgave whose lines the user keeps himself; 3a and 3b come from the bookings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub enum IcpRubriek {
    /// Correcties intracommunautaire leveringen.
    #[serde(rename = "1a", alias = "1A")]
    R1a,
    /// Correcties intracommunautaire diensten.
    #[serde(rename = "1b", alias = "1B")]
    R1b,
    /// Correcties intracommunautaire A-B-C-leveringen.
    #[serde(rename = "2")]
    R2,
    /// Intracommunautaire A-B-C-leveringen.
    #[serde(rename = "4")]
    R4,
}

impl IcpRubriek {
    const ALL: [Self; 4] = [Self::R1a, Self::R1b, Self::R2, Self::R4];

    /// As the screen sends it.
    fn code(self) -> &'static str {
        match self {
            Self::R1a => "1A",
            Self::R1b => "1B",
            Self::R2 => "2",
            Self::R4 => "4",
        }
    }

    /// A correction names the period it corrects; an A-B-C supply does not.
    fn corrigeert(self) -> bool {
        self != Self::R4
    }
}

/// The rubrieken of an ICP-opgave, and of an ICL-opgave (before 2010), as the screen lists them.
const ICP_RUBRIEKEN: [&str; 6] = ["1A", "1B", "2", "3A", "3B", "4"];
const ICL_RUBRIEKEN: [&str; 4] = ["1", "2", "3", "4"];

fn icp_rubriek_naam(code: &str) -> &'static str {
    match code {
        "1" | "1A" => "Correcties intracommunautaire leveringen",
        "1B" => "Correcties intracommunautaire diensten",
        "2" => "Correcties intracommunautaire A-B-C-leveringen",
        "3" | "3A" => "Intracommunautaire leveringen",
        "3B" => "Intracommunautaire diensten",
        "4" => "Intracommunautaire A-B-C-leveringen",
        _ => "",
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct IcpRegelToevoegenArgs {
    /// Jaar van de opgave, 2010 tot en met dit jaar.
    pub jaar: i32,
    /// maand, twee_maanden, kwartaal of jaar: het tijdvak van de opgave.
    pub periode_type: IcpTijdvak,
    /// Nummer van de maand (1 tot 12), de twee maanden (1 tot 4) of het kwartaal (1 tot 4). Niet nodig bij jaar.
    pub periode: Option<u32>,
    /// Rubriek: 1a (correctie leveringen), 1b (correctie diensten), 2 (correctie A-B-C-leveringen) of 4 (A-B-C-leveringen).
    pub rubriek: IcpRubriek,
    /// Landcode, zoals DE of BE (de landen van de ICP-opgave in e-Boekhouden).
    pub land: String,
    /// Btw-nummer van de afnemer, 4 tot 14 tekens.
    pub btw_nummer: String,
    /// Bedrag in hele euro's.
    pub bedrag: i64,
    /// Bij een correctie (1a, 1b, 2): jaar van het tijdvak dat gecorrigeerd wordt.
    pub correctie_jaar: Option<i32>,
    /// Bij een correctie: maand, twee_maanden, kwartaal of jaar van het tijdvak dat gecorrigeerd wordt.
    pub correctie_periode_type: Option<IcpTijdvak>,
    /// Bij een correctie: nummer van de maand, de twee maanden of het kwartaal dat gecorrigeerd wordt. Niet nodig bij jaar.
    pub correctie_periode: Option<u32>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct IcpRegelWijzigenArgs {
    /// Jaar van de opgave, 2010 tot en met dit jaar.
    pub jaar: i32,
    /// maand, twee_maanden, kwartaal of jaar: het tijdvak van de opgave.
    pub periode_type: IcpTijdvak,
    /// Nummer van de maand (1 tot 12), de twee maanden (1 tot 4) of het kwartaal (1 tot 4). Niet nodig bij jaar.
    pub periode: Option<u32>,
    /// Id van de regel (uit icp_opgave).
    pub id: u64,
    /// Nieuwe landcode.
    pub land: Option<String>,
    /// Nieuw btw-nummer van de afnemer.
    pub btw_nummer: Option<String>,
    /// Nieuw bedrag in hele euro's.
    pub bedrag: Option<i64>,
    /// Bij een correctie: nieuw jaar van het gecorrigeerde tijdvak. Geef dan ook correctie_periode_type.
    pub correctie_jaar: Option<i32>,
    /// Bij een correctie: nieuwe maand, twee_maanden, kwartaal of jaar.
    pub correctie_periode_type: Option<IcpTijdvak>,
    /// Bij een correctie: nieuw nummer van de maand, de twee maanden of het kwartaal.
    pub correctie_periode: Option<u32>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct IcpRegelVerwijderenArgs {
    /// Jaar van de opgave, 2010 tot en met dit jaar.
    pub jaar: i32,
    /// maand, twee_maanden, kwartaal of jaar: het tijdvak van de opgave.
    pub periode_type: IcpTijdvak,
    /// Nummer van de maand (1 tot 12), de twee maanden (1 tot 4) of het kwartaal (1 tot 4). Niet nodig bij jaar.
    pub periode: Option<u32>,
    /// Id van de regel (uit icp_opgave).
    pub id: u64,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

// ---------------------------------------------------------------- building

/// An id e-Boekhouden gives (a number or a text) as one path segment, never anything that leaves the path.
fn segment(value: Option<&Value>, what: &str) -> Result<String, String> {
    let text = match value {
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::String(s)) => s.trim().to_owned(),
        _ => String::new(),
    };
    if text.is_empty()
        || !text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        return Err(format!(
            "e-Boekhouden gaf de ICP-opgave zonder bruikbare {what}"
        ));
    }
    Ok(text)
}

/// A land of the ICP-opgave (`icpopgave/landen`): its code and its name as the screen shows it, "Duitsland (DE)".
/// For rubriek 4 the screen leaves out a land whose `einddatum` is not after the start of the opgave.
fn land(
    landen: &Value,
    code: &str,
    rubriek: IcpRubriek,
    start: (i32, u32, u32),
) -> Result<(String, String), String> {
    let code = code.trim().to_ascii_uppercase();
    let list = landen
        .as_array()
        .ok_or("e-Boekhouden gaf de landen van de ICP-opgave in een onbekende vorm terug")?;
    let found = list
        .iter()
        .find(|l| text_of(l, "code").eq_ignore_ascii_case(&code))
        .ok_or_else(|| {
            format!("Onbekende landcode {code:?}; de ICP-opgave kent alleen landen van de EU")
        })?;
    if rubriek == IcpRubriek::R4
        && let Some(einde) = found
            .get("einddatum")
            .and_then(Value::as_str)
            .and_then(|d| parse_date(d.get(..10)?).ok())
        && einde <= start
    {
        return Err(format!(
            "{code} hoort sinds {} niet meer bij de A-B-C-leveringen van de ICP-opgave",
            dashed(einde)
        ));
    }
    Ok((code.clone(), format!("{} ({code})", text_of(found, "land"))))
}

fn check_btw_nummer(nummer: &str) -> Result<String, String> {
    let nummer = nummer.trim();
    let len = nummer.chars().count();
    if !(4..=14).contains(&len) {
        return Err("Het btw-nummer van de afnemer heeft 4 tot 14 tekens".into());
    }
    Ok(nummer.to_owned())
}

fn check_bedrag(bedrag: i64) -> Result<i64, String> {
    // The screen's field takes 15 characters.
    if bedrag.to_string().len() > 15 {
        return Err("Het bedrag is te groot".into());
    }
    Ok(bedrag)
}

/// The period a correction corrects: required for 1a, 1b and 2 and none for 4; within the years the screen
/// offers (eight back, three ahead) and wholly before the opgave, as the line form's date checks.
fn correctie(
    rubriek: IcpRubriek,
    opgave: IcpPeriode,
    jaar: Option<i32>,
    tijdvak: Option<IcpTijdvak>,
    periode: Option<u32>,
    this_year: i32,
) -> Result<Option<IcpPeriode>, String> {
    if !rubriek.corrigeert() {
        if jaar.is_some() || tijdvak.is_some() || periode.is_some() {
            return Err(
                "Rubriek 4 (A-B-C-leveringen) heeft geen gecorrigeerd tijdvak; laat de correctie_ velden weg".into(),
            );
        }
        return Ok(None);
    }
    let jaar = jaar.ok_or("Een correctie noemt het tijdvak dat hij corrigeert: geef correctie_jaar en correctie_periode_type")?;
    if !(this_year - 8..=this_year + 3).contains(&jaar) {
        return Err(format!(
            "Het gecorrigeerde jaar ligt tussen {} en {}",
            this_year - 8,
            this_year + 3
        ));
    }
    let gecorrigeerd = IcpPeriode::new(jaar, tijdvak, periode)
        .map_err(|e| format!("Gecorrigeerd tijdvak: {e}"))?;
    if gecorrigeerd.after() > opgave.start() {
        return Err(format!(
            "Een correctie gaat over een tijdvak voor de opgave: {} ligt niet helemaal voor {}",
            gecorrigeerd.naam(),
            opgave.naam()
        ));
    }
    Ok(Some(gecorrigeerd))
}

/// The request for a line of the opgave (`icpopgave/regels`), as the line form builds it: the period fields
/// only for a correction, the year and the period number as the form's selects give them, as text.
fn regel_body(
    id: Option<u64>,
    rubriek: IcpRubriek,
    ea_periode: &Value,
    land: &str,
    btw_nummer: &str,
    bedrag: i64,
    gecorrigeerd: Option<IcpPeriode>,
) -> Value {
    let mut body = serde_json::Map::new();
    if let Some(id) = id {
        body.insert("id".into(), json!(id));
    }
    body.insert("bedrag".into(), json!(bedrag));
    body.insert("btwNummer".into(), json!(btw_nummer));
    body.insert("rubriek".into(), json!(rubriek.code()));
    body.insert("eaPeriode".into(), ea_periode.clone());
    body.insert("land".into(), json!(land));
    if let Some(p) = gecorrigeerd {
        body.insert("jaar".into(), json!(p.jaar.to_string()));
        body.insert("periodType".into(), json!(p.soort));
        body.insert("periodId".into(), json!(p.id.to_string()));
    }
    Value::Object(body)
}

fn rubriek_line(rubriek: IcpRubriek) -> approvals::Line {
    line(
        "Rubriek",
        format!(
            "{} {}",
            rubriek.code().to_ascii_lowercase(),
            icp_rubriek_naam(rubriek.code())
        ),
    )
}

/// The details of the opgave (`PUT icpopgave`), as the screen's form sends them: all five, with the new ones
/// over what e-Boekhouden has.
fn opgave_wijzigen(
    opgave: &Value,
    periode: IcpPeriode,
    args: &IcpOpgaveWijzigenArgs,
) -> Result<Built, String> {
    let id = opgave
        .get("id")
        .and_then(Value::as_u64)
        .ok_or("e-Boekhouden gaf de ICP-opgave zonder id")?;
    // key, label, new value, longest, only letters, digits, + and *
    let velden: [(&str, &str, Option<&String>, usize, bool); 5] = [
        ("btwNummer", "OB-nummer", args.ob_nummer.as_ref(), 50, true),
        ("bedrijf", "Bedrijf", args.bedrijf.as_ref(), 50, false),
        (
            "contactpersoon",
            "Contactpersoon",
            args.contactpersoon.as_ref(),
            35,
            false,
        ),
        ("telefoon", "Telefoon", args.telefoon.as_ref(), 14, false),
        (
            "onderdeelFiscaleEenheid",
            "Onderdeel fiscale eenheid",
            args.onderdeel_fiscale_eenheid.as_ref(),
            50,
            true,
        ),
    ];
    let mut body = serde_json::Map::new();
    body.insert("id".into(), json!(id));
    let mut lines = vec![line("ICP-opgave", periode.naam())];
    for (key, label, new, max, code) in velden {
        let before = text_of(opgave, key);
        let Some(new) = new.map(|v| v.trim().to_owned()) else {
            body.insert(key.into(), opgave.get(key).cloned().unwrap_or(Value::Null));
            continue;
        };
        if new.chars().count() > max {
            return Err(format!("{label} heeft hoogstens {max} tekens"));
        }
        if code
            && !new
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '*'))
        {
            return Err(format!(
                "{label} kan alleen letters, cijfers, '+' en '*' bevatten"
            ));
        }
        body.insert(key.into(), json!(new));
        if new != before {
            lines.push(line(&format!("{label} was"), known(before)));
            lines.push(line(
                &format!("{label} wordt"),
                if new.is_empty() {
                    "(leeg)".to_owned()
                } else {
                    new
                },
            ));
        }
    }
    if lines.len() == 1 {
        return Err("Er is niets gewijzigd: geef aan wat er anders moet".into());
    }
    lines.push(line(
        "Gevolg",
        "de gegevens van de ICP-opgave veranderen in e-Boekhouden. Er gaat niets naar de Belastingdienst.",
    ));
    Ok(Built {
        body: Value::Object(body),
        title: format!("ICP-opgave wijzigen: {}", periode.naam()),
        lines,
    })
}

/// The rows of a grid answer, named.
fn rows_of(grid: Value) -> Vec<Value> {
    named_rows(grid)
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// What the tools that change an opgave need of it: its period, its `eaPeriod` (which the lines and the status
/// list are kept under) and the record itself.
struct Opgave {
    periode: IcpPeriode,
    ea_periode: Value,
    ea_segment: String,
    record: Value,
}

impl EbMcp {
    /// The opgave of a period as the screen opens it (`icpopgave/{jaar}/{periodType}/{periodId}`).
    async fn icp_opgave_van(&self, run: &Run, periode: IcpPeriode) -> Result<Opgave, String> {
        let record = self
            .btwaangifte_get(run, &periode.path(), Vec::new())
            .await?;
        if record.is_null() {
            return Err(format!(
                "e-Boekhouden heeft geen ICP-opgave over {}",
                periode.naam()
            ));
        }
        let ea_periode = record.get("eaPeriod").cloned().unwrap_or(Value::Null);
        let ea_segment = segment(Some(&ea_periode), "periode (eaPeriod)")?;
        Ok(Opgave {
            periode,
            ea_periode,
            ea_segment,
            record,
        })
    }

    /// The lines of one rubriek of an opgave not sent yet.
    async fn icp_regels(&self, run: &Run, ea: &str, rubriek: &str) -> Result<Vec<Value>, String> {
        let path = format!("icpopgave/regels/{ea}/{rubriek}/gridtable");
        Ok(rows_of(
            self.btwaangifte_get(run, &path, grid(Some(MAX_ROWS), None)?)
                .await?,
        ))
    }

    /// A line the user keeps himself, found in the rubrieken of the opgave, with its rubriek and its own record.
    async fn icp_eigen_regel(
        &self,
        run: &Run,
        opgave: &Opgave,
        id: u64,
    ) -> Result<(IcpRubriek, Value), String> {
        for rubriek in IcpRubriek::ALL {
            let rows = self
                .icp_regels(run, &opgave.ea_segment, rubriek.code())
                .await?;
            if rows
                .iter()
                .any(|row| row.get("id").and_then(Value::as_u64) == Some(id))
            {
                let regel = self
                    .btwaangifte_get(run, &format!("icpopgave/regels/{id}"), Vec::new())
                    .await?;
                if !regel.is_object() {
                    return Err(format!("e-Boekhouden gaf regel {id} niet terug"));
                }
                return Ok((rubriek, regel));
            }
        }
        Err(format!(
            "Regel {id} staat niet bij de regels van rubriek 1a, 1b, 2 of 4 van de ICP-opgave over {}; zie icp_opgave",
            opgave.periode.naam()
        ))
    }
}

/// The sum of the `bedrag` column, as the screen's "Totaal".
fn totaal(rows: &[Value]) -> f64 {
    rows.iter()
        .map(|row| number(row.get("bedrag")).unwrap_or(0.0))
        .sum()
}

// ---------------------------------------------------------------- the tools

#[tool_router(router = btwaangifte_icp_router, vis = "pub(super)")]
impl EbMcp {
    #[tool(
        description = "De verstuurde ICP-opgaven (uit de verstuurde aangiftes), met datum, periode en status. Open er een met icp_opgave.",
        annotations(title = "ICP-opgaven", read_only_hint = true)
    )]
    async fn icp_opgaven(
        &self,
        Parameters(args): Parameters<AccountArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let run = match self.begin(&context, "icp_opgaven", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let result = self.btwaangifte_sent(&run).await.map(|rows| {
            let rows: Vec<Value> = rows
                .into_iter()
                .filter(|row| Verzonden::of(row) == Verzonden::Icp)
                .collect();
            json!({"rowCount": rows.len(), "data": rows})
        });
        Ok(self.end(run, "aangifte/gridtable", result))
    }

    #[tool(
        description = "Eén ICP-opgave met al zijn regels per rubriek en het totaal per rubriek. Met id (uit icp_opgaven): een verstuurde opgave. Met jaar, periode_type en periode: de opgave van dat tijdvak zoals e-Boekhouden hem opmaakt, met de verzendingen van dat tijdvak; rubriek 3a en 3b komen uit de boekingen, 1a, 1b, 2 en 4 houdt de gebruiker zelf bij met icp_regel_toevoegen.",
        annotations(title = "ICP-opgave", read_only_hint = true)
    )]
    async fn icp_opgave(
        &self,
        Parameters(args): Parameters<IcpOpgaveArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let run = match self.begin(&context, "icp_opgave", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let gekozen = match (args.id, args.jaar) {
            (Some(id), None) if args.periode_type.is_none() && args.periode.is_none() => {
                Ok(Err(id))
            }
            (None, Some(jaar)) => IcpPeriode::opgave(jaar, args.periode_type, args.periode).map(Ok),
            _ => Err(
                "Geef id (een verstuurde opgave) of jaar, periode_type en periode, niet allebei"
                    .to_owned(),
            ),
        };
        let gekozen = match gekozen {
            Ok(gekozen) => gekozen,
            Err(message) => return Ok(self.refuse(run, message)),
        };
        let (source, result) = match gekozen {
            Err(id) => {
                let source = format!("icpopgave/{id}");
                let result = async {
                    let opgave = self.btwaangifte_get(&run, &source, Vec::new()).await?;
                    if opgave.is_null() {
                        return Err(format!("e-Boekhouden heeft geen ICP-opgave {id}"));
                    }
                    let icl = number(opgave.get("jaar"))? < 2010.0;
                    let mut regels = serde_json::Map::new();
                    for rubriek in if icl { &ICL_RUBRIEKEN[..] } else { &ICP_RUBRIEKEN[..] } {
                        let path = format!("icpopgave/regels/id/{id}/{rubriek}/gridtable");
                        let rows = rows_of(
                            self.btwaangifte_get(&run, &path, grid(Some(MAX_ROWS), None)?)
                                .await?,
                        );
                        regels.insert(
                            rubriek.to_ascii_lowercase(),
                            json!({"naam": icp_rubriek_naam(rubriek), "totaal": totaal(&rows), "regels": rows}),
                        );
                    }
                    Ok(json!({"opgave": opgave, "rubrieken": regels}))
                }
                .await;
                (source, result)
            }
            Ok(periode) => {
                let source = periode.path();
                let result = async {
                    let opgave = self.icp_opgave_van(&run, periode).await?;
                    let mut regels = serde_json::Map::new();
                    for rubriek in ICP_RUBRIEKEN {
                        let rows = self.icp_regels(&run, &opgave.ea_segment, rubriek).await?;
                        regels.insert(
                            rubriek.to_ascii_lowercase(),
                            json!({"naam": icp_rubriek_naam(rubriek), "totaal": totaal(&rows), "regels": rows}),
                        );
                    }
                    let verzendingen = rows_of(
                        self.btwaangifte_get(
                            &run,
                            &format!("aangifte/{}/gridtable", opgave.ea_segment),
                            grid(Some(MAX_ROWS), None)?,
                        )
                        .await?,
                    );
                    Ok(json!({"periode": periode.naam(), "opgave": opgave.record, "rubrieken": regels, "verzendingen": verzendingen}))
                }
                .await;
                (source, result)
            }
        };
        Ok(self.end(run, &source, result))
    }

    #[tool(
        description = "De gegevens van een ICP-opgave wijzigen: OB-nummer, bedrijf, contactpersoon, telefoon en onderdeel fiscale eenheid, zoals Opslaan in het scherm ICP-opgave. Wat je niet noemt blijft zoals het is. Dit verstuurt niets naar de Belastingdienst: versturen doet de gebruiker zelf in e-Boekhouden. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet. De gebruiker moet dit altijd goedkeuren in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "ICP-opgave wijzigen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn icp_opgave_wijzigen(
        &self,
        Parameters(args): Parameters<IcpOpgaveWijzigenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "icp_opgave_wijzigen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "ICP-opgave gewijzigd. Er is niets naar de Belastingdienst gestuurd.",
            move |this, run, args| async move {
                let periode = IcpPeriode::opgave(args.jaar, Some(args.periode_type), args.periode)?;
                let opgave = this.icp_opgave_van(&run, periode).await?;
                let built = opgave_wijzigen(&opgave.record, periode, &args)?;
                this.approve(&run, TOOL, &built, true).await?;
                this.btwaangifte_send(&run, "PUT", "icpopgave", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een regel toevoegen aan een ICP-opgave, zoals Toevoegen in het scherm: een correctie van een eerder tijdvak (rubriek 1a leveringen, 1b diensten, 2 A-B-C-leveringen, met het gecorrigeerde tijdvak) of een A-B-C-levering (rubriek 4). Rubriek 3a en 3b komen uit de boekingen. Nodig: het tijdvak van de opgave, land, btw-nummer van de afnemer en bedrag in hele euro's. Dit verstuurt niets naar de Belastingdienst. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet. De gebruiker moet dit altijd goedkeuren in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "ICP-regel toevoegen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn icp_regel_toevoegen(
        &self,
        Parameters(args): Parameters<IcpRegelToevoegenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "icp_regel_toevoegen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Regel toegevoegd aan de ICP-opgave; data.id is het id. Er is niets naar de Belastingdienst gestuurd.",
            move |this, run, args| async move {
                let periode = IcpPeriode::opgave(args.jaar, Some(args.periode_type), args.periode)?;
                let btw_nummer = check_btw_nummer(&args.btw_nummer)?;
                let bedrag = check_bedrag(args.bedrag)?;
                let gecorrigeerd = correctie(
                    args.rubriek,
                    periode,
                    args.correctie_jaar,
                    args.correctie_periode_type,
                    args.correctie_periode,
                    today().0,
                )?;
                let opgave = this.icp_opgave_van(&run, periode).await?;
                let landen = this.btwaangifte_get(&run, "icpopgave/landen", Vec::new()).await?;
                let (code, land_naam) = land(&landen, &args.land, args.rubriek, periode.start())?;
                let mut lines = vec![
                    line("ICP-opgave", periode.naam()),
                    rubriek_line(args.rubriek),
                    line("Land", &land_naam),
                    line("Btw-nummer afnemer", &btw_nummer),
                    line("Bedrag", euro_whole(bedrag)),
                ];
                if let Some(p) = gecorrigeerd {
                    lines.push(line("Gecorrigeerd tijdvak", p.naam()));
                }
                lines.push(line("Gevolg", "de regel komt in de ICP-opgave in e-Boekhouden. Er gaat niets naar de Belastingdienst."));
                let built = Built {
                    body: regel_body(None, args.rubriek, &opgave.ea_periode, &code, &btw_nummer, bedrag, gecorrigeerd),
                    title: format!("ICP-regel toevoegen: {} {}", land_naam, euro_whole(bedrag)),
                    lines,
                };
                this.approve(&run, TOOL, &built, true).await?;
                this.btwaangifte_send(&run, "POST", "icpopgave/regels", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een regel van een ICP-opgave wijzigen, op id (uit icp_opgave): land, btw-nummer, bedrag of het gecorrigeerde tijdvak. Alleen de regels die de gebruiker zelf bijhoudt (rubriek 1a, 1b, 2 en 4). Wat je niet noemt blijft zoals het is. Dit verstuurt niets naar de Belastingdienst. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet. De gebruiker moet dit altijd goedkeuren in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "ICP-regel wijzigen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn icp_regel_wijzigen(
        &self,
        Parameters(args): Parameters<IcpRegelWijzigenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "icp_regel_wijzigen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Regel van de ICP-opgave gewijzigd. Er is niets naar de Belastingdienst gestuurd.",
            move |this, run, args| async move {
                let periode = IcpPeriode::opgave(args.jaar, Some(args.periode_type), args.periode)?;
                let opgave = this.icp_opgave_van(&run, periode).await?;
                let (rubriek, regel) = this.icp_eigen_regel(&run, &opgave, args.id).await?;
                let was_land = text_of(&regel, "land");
                let was_nummer = text_of(&regel, "btwNummer");
                let was_bedrag = number(regel.get("bedrag"))?;
                let was_tijdvak = IcpPeriode::from_record(&regel, "periodId");

                let btw_nummer = check_btw_nummer(args.btw_nummer.as_deref().unwrap_or(&was_nummer))?;
                let bedrag = match args.bedrag {
                    Some(bedrag) => check_bedrag(bedrag)?,
                    None => was_bedrag.round() as i64,
                };
                let nieuw_tijdvak = args.correctie_jaar.is_some()
                    || args.correctie_periode_type.is_some()
                    || args.correctie_periode.is_some();
                let gecorrigeerd = if nieuw_tijdvak {
                    correctie(rubriek, periode, args.correctie_jaar, args.correctie_periode_type, args.correctie_periode, today().0)?
                } else if rubriek.corrigeert() {
                    Some(was_tijdvak.ok_or_else(|| "e-Boekhouden gaf deze correctie zonder het tijdvak dat hij corrigeert; geef correctie_jaar en correctie_periode_type".to_owned())?)
                } else {
                    None
                };
                let landen = this.btwaangifte_get(&run, "icpopgave/landen", Vec::new()).await?;
                let (code, land_naam) = land(&landen, args.land.as_deref().unwrap_or(&was_land), rubriek, periode.start())?;

                let mut lines = vec![line("ICP-opgave", periode.naam()), rubriek_line(rubriek)];
                let mut change = |label: &str, before: String, after: String| {
                    if before != after {
                        lines.push(line(&format!("{label} was"), known(before)));
                        lines.push(line(&format!("{label} wordt"), after));
                    }
                };
                change("Land", was_land.to_ascii_uppercase(), code.clone());
                change("Btw-nummer afnemer", was_nummer, btw_nummer.clone());
                change("Bedrag", euro_whole(was_bedrag.round() as i64), euro_whole(bedrag));
                change(
                    "Gecorrigeerd tijdvak",
                    was_tijdvak.filter(|_| rubriek.corrigeert()).map(IcpPeriode::naam).unwrap_or_default(),
                    gecorrigeerd.map(IcpPeriode::naam).unwrap_or_default(),
                );
                if lines.len() == 2 {
                    return Err(Halt::Error("Er is niets gewijzigd: geef aan wat er anders moet".into()));
                }
                lines.push(line("Gevolg", "de regel verandert in de ICP-opgave in e-Boekhouden. Er gaat niets naar de Belastingdienst."));
                let built = Built {
                    body: regel_body(Some(args.id), rubriek, &opgave.ea_periode, &code, &btw_nummer, bedrag, gecorrigeerd),
                    title: format!("ICP-regel wijzigen: {land_naam}"),
                    lines,
                };
                this.approve(&run, TOOL, &built, true).await?;
                this.btwaangifte_send(&run, "PUT", "icpopgave/regels", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een regel van een ICP-opgave verwijderen, op id (uit icp_opgave). Alleen de regels die de gebruiker zelf bijhoudt (rubriek 1a, 1b, 2 en 4). Dit is niet terug te draaien. Dit verstuurt niets naar de Belastingdienst. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet. De gebruiker moet dit altijd goedkeuren in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "ICP-regel verwijderen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn icp_regel_verwijderen(
        &self,
        Parameters(args): Parameters<IcpRegelVerwijderenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "icp_regel_verwijderen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Regel van de ICP-opgave verwijderd. Er is niets naar de Belastingdienst gestuurd.",
            move |this, run, args| async move {
                let periode = IcpPeriode::opgave(args.jaar, Some(args.periode_type), args.periode)?;
                let opgave = this.icp_opgave_van(&run, periode).await?;
                let (rubriek, regel) = this.icp_eigen_regel(&run, &opgave, args.id).await?;
                let bedrag = number(regel.get("bedrag"))?;
                let mut lines = vec![
                    line("ICP-opgave", periode.naam()),
                    rubriek_line(rubriek),
                    line("Land", known(text_of(&regel, "land"))),
                    line("Btw-nummer afnemer", known(text_of(&regel, "btwNummer"))),
                    line("Bedrag", euro_whole(bedrag.round() as i64)),
                ];
                if let Some(p) = IcpPeriode::from_record(&regel, "periodId").filter(|_| rubriek.corrigeert()) {
                    lines.push(line("Gecorrigeerd tijdvak", p.naam()));
                }
                lines.push(line("Gevolg", "de regel verdwijnt uit de ICP-opgave; dit is niet terug te draaien. Er gaat niets naar de Belastingdienst."));
                let built = Built {
                    body: Value::Null,
                    title: format!("ICP-regel verwijderen: {}", periode.naam()),
                    lines,
                };
                this.approve(&run, TOOL, &built, true).await?;
                this.btwaangifte_send(&run, "DELETE", &format!("icpopgave/regels/{}", args.id), None)
                    .await
            },
        )
        .await
    }
}

/// Whole euros for the approval window.
fn euro_whole(bedrag: i64) -> String {
    write::euro(bedrag as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kwartaal(jaar: i32, id: u32) -> IcpPeriode {
        IcpPeriode::new(jaar, Some(IcpTijdvak::Kwartaal), Some(id)).unwrap()
    }

    fn landen() -> Value {
        json!([
            {"code": "BE", "land": "Belgie", "einddatum": null},
            {"code": "DE", "land": "Duitsland", "einddatum": null},
            {"code": "GB", "land": "Verenigd Koninkrijk", "einddatum": "2020-12-31T00:00:00"}
        ])
    }

    #[test]
    fn icp_periods_as_the_icp_screens_take_them() {
        let twee = IcpPeriode::new(2026, Some(IcpTijdvak::TweeMaanden), Some(2)).unwrap();
        assert_eq!((twee.start(), twee.after()), ((2026, 4, 1), (2026, 6, 1)));
        assert_eq!(twee.naam(), "april/mei 2026");
        assert_eq!(twee.path(), "icpopgave/2026/2M/2");
        let k4 = kwartaal(2026, 4);
        assert_eq!((k4.start(), k4.after()), ((2026, 10, 1), (2027, 1, 1)));
        let jaar = IcpPeriode::new(2025, Some(IcpTijdvak::Jaar), None).unwrap();
        assert_eq!(
            jaar.path(),
            "icpopgave/2025/J/2025",
            "a year's period id is the year"
        );
        assert_eq!(jaar.after(), (2026, 1, 1));
        let maand = IcpPeriode::new(2026, Some(IcpTijdvak::Maand), Some(12)).unwrap();
        assert_eq!(maand.after(), (2027, 1, 1));
        assert!(IcpPeriode::new(2026, None, Some(1)).is_err());
        assert!(IcpPeriode::new(2026, Some(IcpTijdvak::TweeMaanden), Some(5)).is_err());
        assert!(IcpPeriode::opgave(2009, Some(IcpTijdvak::Jaar), None).is_err());
        assert!(IcpPeriode::opgave(today().0 + 1, Some(IcpTijdvak::Jaar), None).is_err());
        assert_eq!(
            IcpPeriode::from_record(
                &json!({"jaar": "2025", "periodType": "K", "periodId": 2}),
                "periodId"
            ),
            Some(kwartaal(2025, 2))
        );
        assert_eq!(
            IcpPeriode::from_record(&json!({"jaar": 2025}), "periodId"),
            None
        );
    }

    #[test]
    fn a_line_is_sent_as_the_line_form_builds_it() {
        let opgave = kwartaal(2026, 3);
        let gecorrigeerd = correctie(
            IcpRubriek::R1a,
            opgave,
            Some(2026),
            Some(IcpTijdvak::Kwartaal),
            Some(1),
            2026,
        )
        .unwrap();
        assert_eq!(
            regel_body(
                None,
                IcpRubriek::R1a,
                &json!("2026K3"),
                "DE",
                "DE123456789",
                -250,
                gecorrigeerd
            ),
            json!({"bedrag": -250, "btwNummer": "DE123456789", "rubriek": "1A", "eaPeriode": "2026K3", "land": "DE",
                   "jaar": "2026", "periodType": "K", "periodId": "1"})
        );
        // An A-B-C supply has no period; an edit carries its id.
        assert_eq!(
            regel_body(
                Some(77),
                IcpRubriek::R4,
                &json!(901),
                "BE",
                "BE0123456789",
                1000,
                None
            ),
            json!({"id": 77, "bedrag": 1000, "btwNummer": "BE0123456789", "rubriek": "4", "eaPeriode": 901, "land": "BE"})
        );
        let jaar = correctie(
            IcpRubriek::R2,
            opgave,
            Some(2025),
            Some(IcpTijdvak::Jaar),
            None,
            2026,
        )
        .unwrap();
        assert_eq!(
            regel_body(None, IcpRubriek::R2, &json!(1), "DE", "DE1234", 5, jaar)["periodId"],
            "2025"
        );
    }

    #[test]
    fn a_correction_names_a_period_wholly_before_the_opgave() {
        let opgave = kwartaal(2026, 3);
        let corr = |jaar, tijdvak, periode| {
            correctie(IcpRubriek::R1b, opgave, jaar, tijdvak, periode, 2026)
        };
        assert!(corr(Some(2026), Some(IcpTijdvak::Kwartaal), Some(2)).is_ok());
        assert!(corr(Some(2026), Some(IcpTijdvak::Maand), Some(6)).is_ok());
        let err = corr(Some(2026), Some(IcpTijdvak::Kwartaal), Some(3)).unwrap_err();
        assert!(err.contains("niet helemaal voor"), "{err}");
        assert!(corr(Some(2026), Some(IcpTijdvak::Maand), Some(7)).is_err());
        assert!(
            corr(Some(2026), Some(IcpTijdvak::Jaar), None).is_err(),
            "the year of the opgave itself"
        );
        assert!(corr(Some(2025), Some(IcpTijdvak::Jaar), None).is_ok());
        assert!(
            corr(Some(2017), Some(IcpTijdvak::Jaar), None)
                .unwrap_err()
                .contains("tussen 2018")
        );
        assert!(
            corr(None, None, None)
                .unwrap_err()
                .contains("correctie_jaar")
        );
        assert!(corr(Some(2026), None, None).is_err());
        // A-B-C supplies have no period at all.
        assert_eq!(
            correctie(IcpRubriek::R4, opgave, None, None, None, 2026),
            Ok(None)
        );
        assert!(correctie(IcpRubriek::R4, opgave, Some(2025), None, None, 2026).is_err());
    }

    #[test]
    fn checks_land_number_and_amount_before_asking() {
        let start = (2026, 7, 1);
        assert_eq!(
            land(&landen(), " de ", IcpRubriek::R1a, start),
            Ok(("DE".into(), "Duitsland (DE)".into()))
        );
        assert!(
            land(&landen(), "US", IcpRubriek::R1a, start)
                .unwrap_err()
                .contains("Onbekende landcode")
        );
        // A land that left: still there for a correction, gone for an A-B-C supply after its end.
        assert!(land(&landen(), "GB", IcpRubriek::R1a, start).is_ok());
        assert!(
            land(&landen(), "GB", IcpRubriek::R4, start)
                .unwrap_err()
                .contains("2020-12-31")
        );
        assert!(land(&landen(), "GB", IcpRubriek::R4, (2020, 10, 1)).is_ok());
        assert!(check_btw_nummer("DE1").is_err());
        assert!(check_btw_nummer("DE123456789012X").is_err());
        assert_eq!(check_btw_nummer(" DE123456789 "), Ok("DE123456789".into()));
        assert!(check_bedrag(1_000_000_000_000_000).is_err());
        assert_eq!(check_bedrag(-12), Ok(-12));
    }

    #[test]
    fn the_details_of_an_opgave_show_old_and_new() {
        let opgave = json!({"id": 55, "eaPeriod": "2026K3", "btwNummer": "NL001234567B01", "bedrijf": "Studio Noord",
                            "contactpersoon": "Lisa", "telefoon": null, "onderdeelFiscaleEenheid": ""});
        let args = |ob: Option<&str>, telefoon: Option<&str>| IcpOpgaveWijzigenArgs {
            jaar: 2026,
            periode_type: IcpTijdvak::Kwartaal,
            periode: Some(3),
            ob_nummer: ob.map(str::to_owned),
            bedrijf: None,
            contactpersoon: Some("Lisa".into()),
            telefoon: telefoon.map(str::to_owned),
            onderdeel_fiscale_eenheid: None,
            account: None,
        };
        let built = opgave_wijzigen(
            &opgave,
            kwartaal(2026, 3),
            &args(Some("NL001234567B02"), Some("0612345678")),
        )
        .unwrap();
        assert_eq!(
            built.body,
            json!({"id": 55, "btwNummer": "NL001234567B02", "bedrijf": "Studio Noord", "contactpersoon": "Lisa",
                   "telefoon": "0612345678", "onderdeelFiscaleEenheid": ""})
        );
        assert_eq!(built.title, "ICP-opgave wijzigen: 3e kwartaal 2026");
        let labels: Vec<(&str, &str)> = built
            .lines
            .iter()
            .map(|l| (l.label.as_str(), l.value.as_str()))
            .collect();
        assert!(
            labels.contains(&("OB-nummer was", "NL001234567B01")),
            "{labels:?}"
        );
        assert!(labels.contains(&("OB-nummer wordt", "NL001234567B02")));
        assert!(labels.contains(&("Telefoon was", "onbekend")));
        assert!(
            !labels.iter().any(|(l, _)| l.starts_with("Contactpersoon")),
            "unchanged is not shown"
        );
        assert!(
            opgave_wijzigen(&opgave, kwartaal(2026, 3), &args(None, None))
                .unwrap_err()
                .contains("niets gewijzigd")
        );
        assert!(
            opgave_wijzigen(&opgave, kwartaal(2026, 3), &args(Some("NL 0012"), None))
                .unwrap_err()
                .contains("letters")
        );
        assert!(
            opgave_wijzigen(
                &opgave,
                kwartaal(2026, 3),
                &args(None, Some("+31 6 1234 5678 9"))
            )
            .unwrap_err()
            .contains("14")
        );
        assert!(
            opgave_wijzigen(&json!({}), kwartaal(2026, 3), &args(Some("NL1"), None))
                .unwrap_err()
                .contains("zonder id")
        );
    }

    #[test]
    fn an_id_from_e_boekhouden_stays_one_path_segment() {
        assert_eq!(segment(Some(&json!(901)), "x"), Ok("901".into()));
        assert_eq!(segment(Some(&json!("2026K3")), "x"), Ok("2026K3".into()));
        assert!(segment(Some(&json!("../auth")), "x").is_err());
        assert!(segment(Some(&json!("a/b")), "x").is_err());
        assert!(segment(None, "x").is_err());
    }
}
