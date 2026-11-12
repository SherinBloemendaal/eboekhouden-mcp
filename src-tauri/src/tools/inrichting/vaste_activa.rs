//! Vaste activa (fixed assets), their afschrijfschema and booking the depreciations that are due.
//!
//! From e-Boekhouden's bundle (chunk-I0e1VsKK.js):
//! - list: `GET vasteactiva/gridtable` (columns naam, omschrijving, datum, aanschafWaarde, afgeschreven,
//!   afTeSchrijven, huidigeWaarde) and `GET vasteactivamutatie/boekingencount` (the depreciations due, shown as
//!   "Afschrijvingen boeken (n)").
//! - the form (`app-vaste-activa-form`, service `vasteactiva`): `GET vasteactiva/{id}`, `GET vasteactivagroep` (the
//!   groups with aantalJaren and their two rekeningen), `GET grootboekrekening/selectlist/BAL` (Rekening activa) and
//!   `.../VW` (Rekening afschrijvingen). Add: `POST vasteactiva`; edit: `PUT vasteactiva?recalc=<bool>` (the
//!   checkbox "Afschrijfschema opnieuw opmaken"); delete: `DELETE vasteactiva/{id}`. Body: {id, vasteActivaGroepId,
//!   omschrijving, aanschafDatum, aanschafwaarde, restwaarde, aantalJaren, boekenPer (J|M), grootboekrekeningBal,
//!   grootboekrekeningVw, afschrijvenStartenPer, boekwaardePerStart, geavanceerd}; dates as the date picker sends
//!   them, {day, month, year}.
//! - the schedule of one asset: `GET vasteactivamutatie/gridtable/{id}` (datum, omschrijving, bedrag,
//!   datumUitgevoerd, mutatieNr).
//! - "Afschrijvingen boeken": `POST vasteactiva/boeken` without a body; it books every depreciation that is due.

use super::*;

/// How often depreciations are booked: the form's two choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BoekenPer {
    Jaar,
    Maand,
}

impl BoekenPer {
    fn code(self) -> &'static str {
        match self {
            Self::Jaar => "J",
            Self::Maand => "M",
        }
    }

    fn from_code(code: &str) -> Option<Self> {
        match code.trim() {
            "J" => Some(Self::Jaar),
            "M" => Some(Self::Maand),
            _ => None,
        }
    }

    fn naam(self) -> &'static str {
        match self {
            Self::Jaar => "jaar",
            Self::Maand => "maand",
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct VastactiefToevoegenArgs {
    /// Id van de vaste-activagroep (uit vaste_activa_groepen). Niet nodig als er maar één groep is.
    pub groep_id: Option<u64>,
    /// Omschrijving, hoogstens 50 tekens, bijvoorbeeld "MacBook Pro 2026".
    pub omschrijving: String,
    /// Aanschafdatum, JJJJ-MM-DD.
    pub aanschafdatum: String,
    /// Aanschafwaarde in euro.
    pub aanschafwaarde: f64,
    /// Restwaarde in euro na het afschrijven; niet hoger dan de aanschafwaarde (vaak 0).
    pub restwaarde: f64,
    /// In hoeveel hele jaren het wordt afgeschreven (standaard dat van de groep).
    pub afschrijven_in_jaren: Option<u32>,
    /// Afschrijvingen boeken per jaar (standaard) of per maand.
    pub boeken_per: Option<BoekenPer>,
    /// Code van de balansrekening voor de activa (standaard die van de groep).
    pub rekening_activa: Option<String>,
    /// Code van de winst-en-verliesrekening voor de afschrijvingen (standaard die van de groep).
    pub rekening_afschrijvingen: Option<String>,
    /// Geavanceerd: afschrijven starten per deze datum, JJJJ-MM-DD (voor een activum dat al eerder in gebruik was).
    pub afschrijven_vanaf: Option<String>,
    /// Geavanceerd: de boekwaarde in euro op de startdatum van het afschrijven.
    pub boekwaarde_bij_start: Option<f64>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct VastactiefWijzigenArgs {
    /// Id van het vaste activum (uit vaste_activa).
    pub id: u64,
    /// Id van de nieuwe groep. Zonder afschrijven_in_jaren en rekeningen nemen die de waarden van de nieuwe groep over, zoals in e-Boekhouden.
    pub groep_id: Option<u64>,
    /// Nieuwe omschrijving, hoogstens 50 tekens.
    pub omschrijving: Option<String>,
    /// Nieuwe aanschafdatum, JJJJ-MM-DD.
    pub aanschafdatum: Option<String>,
    /// Nieuwe aanschafwaarde in euro.
    pub aanschafwaarde: Option<f64>,
    /// Nieuwe restwaarde in euro.
    pub restwaarde: Option<f64>,
    /// Nieuw aantal hele jaren.
    pub afschrijven_in_jaren: Option<u32>,
    /// jaar of maand.
    pub boeken_per: Option<BoekenPer>,
    /// Code van de nieuwe rekening activa.
    pub rekening_activa: Option<String>,
    /// Code van de nieuwe rekening afschrijvingen.
    pub rekening_afschrijvingen: Option<String>,
    /// Geavanceerd: afschrijven starten per deze datum, JJJJ-MM-DD.
    pub afschrijven_vanaf: Option<String>,
    /// Geavanceerd: boekwaarde in euro op de startdatum.
    pub boekwaarde_bij_start: Option<f64>,
    /// Het afschrijfschema opnieuw opmaken met de nieuwe gegevens (standaard niet; zonder blijft het bestaande schema staan).
    pub schema_opnieuw: Option<bool>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AfschrijvingsschemaArgs {
    /// Id van het vaste activum (uit vaste_activa).
    pub id: u64,
    /// Aantal rijen, 1 tot 200 (standaard 50).
    pub limit: Option<u32>,
    /// Vanaf welke rij (standaard 0).
    pub offset: Option<u32>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// A vaste-activagroep: its defaults for a new asset.
#[derive(Clone, Debug, PartialEq)]
struct Groep {
    id: u64,
    naam: String,
    jaren: Option<u32>,
    activa: Option<u64>,
    afschrijving: Option<u64>,
}

fn groepen(list: &Value) -> Vec<Groep> {
    let number = |item: &Value, key: &str| {
        item.get(key).and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        })
    };
    list.as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    Some(Groep {
                        id: number(item, "id")?,
                        naam: item
                            .get("naam")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        jaren: number(item, "aantalJaren").and_then(|j| u32::try_from(j).ok()),
                        activa: number(item, "grootboekrekeningActivaId").filter(|id| *id > 0),
                        afschrijving: number(item, "grootboekrekeningAfschrijvingId")
                            .filter(|id| *id > 0),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What an asset is checked against: the groups and the rekeningen the form offers.
struct ActivaContext {
    groepen: Vec<Groep>,
    /// Rekening activa: `grootboekrekening/selectlist/BAL`.
    balans: Vec<Keuze>,
    /// Rekening afschrijvingen: `grootboekrekening/selectlist/VW`.
    kosten: Vec<Keuze>,
}

impl ActivaContext {
    fn groep(&self, id: u64) -> Result<&Groep, String> {
        self.groepen.iter().find(|g| g.id == id).ok_or_else(|| {
            format!(
                "Geen vaste-activagroep met id {id}. Kies uit: {}",
                self.groepen_lijst()
            )
        })
    }

    fn groepen_lijst(&self) -> String {
        self.groepen
            .iter()
            .map(|g| format!("{} ({})", g.id, g.naam))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn groep_naam(&self, id: u64) -> String {
        self.groepen
            .iter()
            .find(|g| g.id == id)
            .map(|g| g.naam.clone())
            .unwrap_or_else(|| format!("groep met id {id}"))
    }
}

/// One vaste activum, as the form holds it.
#[derive(Clone, Debug, PartialEq)]
struct Activum {
    groep: u64,
    omschrijving: String,
    aanschafdatum: Date,
    aanschafwaarde: f64,
    restwaarde: f64,
    jaren: u32,
    per: BoekenPer,
    activa: u64,
    afschrijving: u64,
    start: Option<Date>,
    boekwaarde_start: Option<f64>,
    geavanceerd: bool,
}

impl Activum {
    /// The form's own checks, before anything is asked.
    fn check(&self, ctx: &ActivaContext) -> Result<(), String> {
        if self.omschrijving.is_empty() {
            return Err("Een vast activum heeft een omschrijving nodig".into());
        }
        kort(&self.omschrijving, 50, "omschrijving")?;
        ctx.groep(self.groep)?;
        if self.restwaarde > self.aanschafwaarde {
            return Err("De restwaarde is groter dan de aanschafwaarde waardoor afschrijven niet mogelijk is".into());
        }
        if self.jaren == 0 {
            return Err("afschrijven_in_jaren moet groter zijn dan 0".into());
        }
        if !ctx.balans.iter().any(|k| k.id == self.activa) {
            return Err(format!(
                "De rekening activa ({}) is geen actieve balansrekening: geef met rekening_activa de code van een balansrekening",
                rekening_naam(&ctx.balans, self.activa)
            ));
        }
        if !ctx.kosten.iter().any(|k| k.id == self.afschrijving) {
            return Err(format!(
                "De rekening afschrijvingen ({}) is geen actieve winst-en-verliesrekening: geef met rekening_afschrijvingen de code van een winst-en-verliesrekening",
                rekening_naam(&ctx.kosten, self.afschrijving)
            ));
        }
        Ok(())
    }

    /// The request body, with the keys of the form's `createVasteActiva`. An empty advanced field is left out.
    fn body(&self, id: Option<u64>) -> Value {
        let mut body = serde_json::Map::new();
        if let Some(id) = id {
            body.insert("id".into(), json!(id));
        }
        body.insert("vasteActivaGroepId".into(), json!(self.groep));
        body.insert("omschrijving".into(), json!(self.omschrijving));
        body.insert("aanschafDatum".into(), date_object(self.aanschafdatum));
        body.insert("aanschafwaarde".into(), json!(self.aanschafwaarde));
        body.insert("restwaarde".into(), json!(self.restwaarde));
        body.insert("aantalJaren".into(), json!(self.jaren));
        body.insert("boekenPer".into(), json!(self.per.code()));
        body.insert("grootboekrekeningBal".into(), json!(self.activa));
        body.insert("grootboekrekeningVw".into(), json!(self.afschrijving));
        if let Some(start) = self.start {
            body.insert("afschrijvenStartenPer".into(), date_object(start));
        }
        if let Some(boekwaarde) = self.boekwaarde_start {
            body.insert("boekwaardePerStart".into(), json!(boekwaarde));
        }
        body.insert("geavanceerd".into(), json!(self.geavanceerd));
        Value::Object(body)
    }

    fn fields(&self, ctx: &ActivaContext) -> Vec<(&'static str, String)> {
        let mut fields = vec![
            ("Groep", ctx.groep_naam(self.groep)),
            ("Omschrijving", self.omschrijving.clone()),
            ("Aanschafdatum", dutch(self.aanschafdatum)),
            ("Aanschafwaarde", write::euro(self.aanschafwaarde)),
            ("Restwaarde", write::euro(self.restwaarde)),
            ("Afschrijven in", format!("{} jaar", self.jaren)),
            ("Afschrijvingen boeken per", self.per.naam().to_owned()),
            ("Rekening activa", rekening_naam(&ctx.balans, self.activa)),
            (
                "Rekening afschrijvingen",
                rekening_naam(&ctx.kosten, self.afschrijving),
            ),
        ];
        if let Some(start) = self.start {
            fields.push(("Afschrijven starten per", dutch(start)));
        }
        if let Some(boekwaarde) = self.boekwaarde_start {
            fields.push(("Boekwaarde bij start", write::euro(boekwaarde)));
        }
        fields
    }

    /// The form asks before saving an asset that cannot be depreciated.
    fn warning(&self) -> Option<approvals::Line> {
        (self.restwaarde == self.aanschafwaarde).then(|| {
            line(
                "Let op",
                "de restwaarde is gelijk aan de aanschafwaarde, dus er wordt niets afgeschreven",
            )
        })
    }

    /// From `GET vasteactiva/{id}`.
    fn from_api(value: &Value) -> Result<Self, String> {
        let bad = || "e-Boekhouden gaf dit vaste activum in een onbekende vorm terug".to_owned();
        let number = |key: &str| {
            value.get(key).and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
            })
        };
        let amount = |key: &str| value.get(key).and_then(Value::as_f64);
        let start = value.get("afschrijvenStartenPer").and_then(date_of);
        Ok(Self {
            groep: number("vasteActivaGroepId").unwrap_or(0),
            omschrijving: value
                .get("omschrijving")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_owned(),
            aanschafdatum: value
                .get("aanschafDatum")
                .and_then(date_of)
                .ok_or_else(bad)?,
            aanschafwaarde: amount("aanschafwaarde").ok_or_else(bad)?,
            restwaarde: amount("restwaarde").unwrap_or(0.0),
            jaren: number("aantalJaren")
                .and_then(|j| u32::try_from(j).ok())
                .ok_or_else(bad)?,
            per: value
                .get("boekenPer")
                .and_then(Value::as_str)
                .and_then(BoekenPer::from_code)
                .unwrap_or(BoekenPer::Jaar),
            activa: number("grootboekrekeningBal").unwrap_or(0),
            afschrijving: number("grootboekrekeningVw").unwrap_or(0),
            start,
            boekwaarde_start: amount("boekwaardePerStart"),
            geavanceerd: value
                .get("geavanceerd")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }
}

fn vastactief_nieuw(args: &VastactiefToevoegenArgs, ctx: &ActivaContext) -> Result<Built, String> {
    let groep = match args.groep_id {
        Some(id) => ctx.groep(id)?,
        None => match ctx.groepen.as_slice() {
            [one] => one,
            [] => {
                return Err("Deze administratie heeft nog geen vaste-activagroep; maak er eerst een in e-Boekhouden onder Beheer > Vaste activa > Groepen beheren".into());
            }
            _ => {
                return Err(format!(
                    "Geef met groep_id aan in welke groep het vaste activum hoort: {}",
                    ctx.groepen_lijst()
                ));
            }
        },
    };
    let activa = match &args.rekening_activa {
        Some(code) => kies(&ctx.balans, code, "rekening activa (balansrekening)")?.id,
        None => groep.activa.ok_or_else(|| {
            format!(
                "Groep {} heeft geen rekening activa: geef rekening_activa",
                groep.naam
            )
        })?,
    };
    let afschrijving = match &args.rekening_afschrijvingen {
        Some(code) => {
            kies(
                &ctx.kosten,
                code,
                "rekening afschrijvingen (winst-en-verliesrekening)",
            )?
            .id
        }
        None => groep.afschrijving.ok_or_else(|| {
            format!(
                "Groep {} heeft geen rekening afschrijvingen: geef rekening_afschrijvingen",
                groep.naam
            )
        })?,
    };
    let jaren = args.afschrijven_in_jaren.or(groep.jaren).ok_or_else(|| {
        format!(
            "Groep {} noemt geen aantal jaren: geef afschrijven_in_jaren",
            groep.naam
        )
    })?;
    let start = args
        .afschrijven_vanaf
        .as_deref()
        .map(parse_date)
        .transpose()?;
    let boekwaarde_start = args
        .boekwaarde_bij_start
        .map(|b| bedrag(b, "boekwaarde_bij_start"))
        .transpose()?;
    let activum = Activum {
        groep: groep.id,
        omschrijving: tekst(Some(&args.omschrijving)).unwrap_or_default(),
        aanschafdatum: parse_date(&args.aanschafdatum)?,
        aanschafwaarde: bedrag(args.aanschafwaarde, "aanschafwaarde")?,
        restwaarde: bedrag(args.restwaarde, "restwaarde")?,
        jaren,
        per: args.boeken_per.unwrap_or(BoekenPer::Jaar),
        activa,
        afschrijving,
        start,
        boekwaarde_start,
        geavanceerd: start.is_some() || boekwaarde_start.is_some(),
    };
    activum.check(ctx)?;
    let mut lines: Vec<approvals::Line> = activum
        .fields(ctx)
        .into_iter()
        .map(|(label, value)| line(label, value))
        .collect();
    lines.extend(activum.warning());
    lines.push(line(
        "Gevolg",
        "e-Boekhouden maakt er een afschrijfschema bij; de afschrijvingen worden pas geboekt met afschrijvingen_boeken. De aanschaf zelf boekt dit niet.",
    ));
    Ok(Built {
        body: activum.body(None),
        title: format!("Vast activum toevoegen: {}", activum.omschrijving),
        lines,
    })
}

fn vastactief_gewijzigd(
    args: &VastactiefWijzigenArgs,
    now: &Value,
    ctx: &ActivaContext,
) -> Result<Built, String> {
    let before = Activum::from_api(now)?;
    let mut activum = before.clone();
    if let Some(id) = args.groep_id
        && id != before.groep
    {
        // As the form does when the group changes: its years and rekeningen come along.
        let groep = ctx.groep(id)?;
        activum.groep = groep.id;
        if let Some(jaren) = groep.jaren {
            activum.jaren = jaren;
        }
        if let Some(activa) = groep.activa {
            activum.activa = activa;
        }
        if let Some(afschrijving) = groep.afschrijving {
            activum.afschrijving = afschrijving;
        }
    }
    if let Some(text) = &args.omschrijving {
        activum.omschrijving = text.trim().to_owned();
    }
    if let Some(date) = &args.aanschafdatum {
        activum.aanschafdatum = parse_date(date)?;
    }
    if let Some(value) = args.aanschafwaarde {
        activum.aanschafwaarde = bedrag(value, "aanschafwaarde")?;
    }
    if let Some(value) = args.restwaarde {
        activum.restwaarde = bedrag(value, "restwaarde")?;
    }
    if let Some(jaren) = args.afschrijven_in_jaren {
        activum.jaren = jaren;
    }
    if let Some(per) = args.boeken_per {
        activum.per = per;
    }
    if let Some(code) = &args.rekening_activa {
        activum.activa = kies(&ctx.balans, code, "rekening activa (balansrekening)")?.id;
    }
    if let Some(code) = &args.rekening_afschrijvingen {
        activum.afschrijving = kies(
            &ctx.kosten,
            code,
            "rekening afschrijvingen (winst-en-verliesrekening)",
        )?
        .id;
    }
    if let Some(date) = &args.afschrijven_vanaf {
        activum.start = Some(parse_date(date)?);
        activum.geavanceerd = true;
    }
    if let Some(value) = args.boekwaarde_bij_start {
        activum.boekwaarde_start = Some(bedrag(value, "boekwaarde_bij_start")?);
        activum.geavanceerd = true;
    }
    activum.check(ctx)?;
    let recalc = args.schema_opnieuw.unwrap_or(false);
    let changed = changes(&before.fields(ctx), &activum.fields(ctx));
    if changed.is_empty() && !recalc {
        return Err("Er is niets gewijzigd: geef aan wat er anders moet".into());
    }
    let mut lines = vec![line("Vast activum", &before.omschrijving)];
    lines.extend(changed);
    lines.extend(activum.warning());
    lines.push(line(
        "Afschrijfschema",
        if recalc {
            "wordt opnieuw opgemaakt met deze gegevens"
        } else {
            "blijft zoals het is (schema_opnieuw staat uit)"
        },
    ));
    let id = now.get("id").and_then(Value::as_u64).unwrap_or(args.id);
    Ok(Built {
        body: activum.body(Some(id)),
        title: format!("Vast activum wijzigen: {}", before.omschrijving),
        lines,
    })
}

/// The approval for deleting an asset: what it is, and how much of its schedule was booked already.
fn vastactief_weg(now: &Value, schema: &[Value], ctx: &ActivaContext) -> Result<Built, String> {
    let activum = Activum::from_api(now)?;
    let mut lines: Vec<approvals::Line> = activum
        .fields(ctx)
        .into_iter()
        .map(|(label, value)| line(label, value))
        .collect();
    let booked: Vec<String> = schema
        .iter()
        .filter_map(|row| row.get("mutatieNr").and_then(Value::as_u64))
        .filter(|nr| *nr > 0)
        .map(|nr| nr.to_string())
        .collect();
    lines.push(line(
        "Afschrijfschema",
        if booked.is_empty() {
            format!("{} regels, nog geen geboekt", schema.len())
        } else {
            format!(
                "{} regels, waarvan {} al geboekt als mutatie {}",
                schema.len(),
                booked.len(),
                booked.join(", ")
            )
        },
    ));
    lines.push(line(
        "Gevolg",
        "het vaste activum verdwijnt uit e-Boekhouden; dit is niet terug te draaien",
    ));
    Ok(Built {
        body: Value::Null,
        title: format!("Vast activum verwijderen: {}", activum.omschrijving),
        lines,
    })
}

fn afschrijvingen_boeken_built(due: u64) -> Result<Built, String> {
    if due == 0 {
        return Err("Er staan geen afschrijvingen klaar om te boeken".into());
    }
    Ok(Built {
        body: Value::Null,
        title: format!("{due} afschrijvingen boeken"),
        lines: vec![
            line("Klaar om te boeken", format!("{due} afschrijvingen")),
            line(
                "Gevolg",
                "e-Boekhouden boekt alle afschrijvingen die klaarstaan, van alle vaste activa, in één keer als mutatie. Bekijk ze eerst met vaste_activa en afschrijvingsschema.",
            ),
            line(
                "Terugdraaien",
                "kan alleen door elke geboekte mutatie apart te verwijderen",
            ),
        ],
    })
}

impl EbMcp {
    /// The groups and rekeningen the vaste-activa form offers.
    async fn activa_context(&self, run: &Run) -> Result<ActivaContext, String> {
        let groepen = groepen(&self.fetch(run, "vasteactivagroep", Vec::new()).await?);
        let balans = self
            .keuzelijst(run, "grootboekrekening/selectlist/BAL", Vec::new())
            .await?;
        let kosten = self
            .keuzelijst(run, "grootboekrekening/selectlist/VW", Vec::new())
            .await?;
        Ok(ActivaContext {
            groepen,
            balans,
            kosten,
        })
    }

    /// One asset, or a clear refusal when there is none with this id.
    async fn vastactief_lezen(&self, run: &Run, id: u64) -> Result<Value, String> {
        let now = self
            .fetch(run, &format!("vasteactiva/{id}"), Vec::new())
            .await?;
        if now.is_null() {
            return Err(format!("Er is geen vast activum met id {id}"));
        }
        Ok(now)
    }

    /// The number of depreciations that are due, as the list shows it next to "Afschrijvingen boeken".
    async fn afschrijvingen_klaar(&self, run: &Run) -> Result<u64, String> {
        self.fetch(run, "vasteactivamutatie/boekingencount", Vec::new())
            .await?
            .as_u64()
            .ok_or_else(|| {
                "e-Boekhouden gaf het aantal te boeken afschrijvingen in een onbekende vorm terug"
                    .into()
            })
    }
}

#[tool_router(router = vaste_activa_router, vis = "pub(super)")]
impl EbMcp {
    #[tool(
        description = "De vaste activa (bedrijfsmiddelen die worden afgeschreven). Per activum: id, naam (groep), omschrijving, datum (aanschaf), aanschafWaarde, afgeschreven, afTeSchrijven en huidigeWaarde. afschrijvingen_te_boeken zegt hoeveel afschrijvingen klaarstaan om te boeken (afschrijvingen_boeken).",
        annotations(title = "Vaste activa", read_only_hint = true)
    )]
    async fn vaste_activa(
        &self,
        Parameters(args): Parameters<ListArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const PATH: &str = "vasteactiva/gridtable";
        let run = match self.begin(&context, "vaste_activa", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let query = match grid(args.limit, args.offset) {
            Ok(query) => query,
            Err(message) => return Ok(self.refuse(run, message)),
        };
        let result = async {
            let mut list = named_rows(self.fetch(&run, PATH, query).await?);
            let due = self.afschrijvingen_klaar(&run).await?;
            if let Some(object) = list.as_object_mut() {
                object.insert("afschrijvingen_te_boeken".into(), json!(due));
            }
            Ok(list)
        }
        .await;
        Ok(self.end(run, PATH, result))
    }

    #[tool(
        description = "Eén vast activum, op id (uit vaste_activa): groep, omschrijving, aanschafdatum, aanschafwaarde, restwaarde, aantal jaren, boeken per jaar of maand, de rekening activa en de rekening afschrijvingen met code en naam, en de geavanceerde start van het afschrijven. Het schema geeft afschrijvingsschema.",
        annotations(title = "Vast activum", read_only_hint = true)
    )]
    async fn vastactief(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let run = match self.begin(&context, "vastactief", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let path = format!("vasteactiva/{}", args.id);
        let result = async {
            let now = self.vastactief_lezen(&run, args.id).await?;
            let ctx = self.activa_context(&run).await?;
            let id = |key: &str| now.get(key).and_then(Value::as_u64).unwrap_or(0);
            Ok(json!({
                "vast_activum": now,
                "groep": ctx.groep_naam(id("vasteActivaGroepId")),
                "rekening_activa": rekening_naam(&ctx.balans, id("grootboekrekeningBal")),
                "rekening_afschrijvingen": rekening_naam(&ctx.kosten, id("grootboekrekeningVw")),
            }))
        }
        .await;
        Ok(self.end(run, &path, result))
    }

    #[tool(
        description = "De vaste-activagroepen: per groep id, naam, aantalJaren en de standaard rekening activa (grootboekrekeningActivaId) en rekening afschrijvingen (grootboekrekeningAfschrijvingId). Een nieuw vast activum hoort in een groep en neemt die waarden over.",
        annotations(title = "Vaste-activagroepen", read_only_hint = true)
    )]
    async fn vaste_activa_groepen(
        &self,
        Parameters(args): Parameters<AccountArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        self.simple(
            &context,
            "vaste_activa_groepen",
            &args,
            args.account.as_deref(),
            "vasteactivagroep",
            Ok(Vec::new()),
        )
        .await
    }

    #[tool(
        description = "Het afschrijfschema van één vast activum, op id (uit vaste_activa): per regel datum, omschrijving, bedrag, datumUitgevoerd (wanneer geboekt) en mutatieNr (de mutatie van de boeking, leeg zolang die niet geboekt is).",
        annotations(title = "Afschrijvingsschema", read_only_hint = true)
    )]
    async fn afschrijvingsschema(
        &self,
        Parameters(args): Parameters<AfschrijvingsschemaArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let run = match self.begin(
            &context,
            "afschrijvingsschema",
            &args,
            args.account.as_deref(),
        ) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let path = format!("vasteactivamutatie/gridtable/{}", args.id);
        let query = match grid(args.limit, args.offset) {
            Ok(query) => query,
            Err(message) => return Ok(self.refuse(run, message)),
        };
        let result = self.fetch(&run, &path, query).await.map(named_rows);
        Ok(self.end(run, &path, result))
    }

    #[tool(
        description = "Een vast activum aanmaken, zoals Vaste activa toevoegen in e-Boekhouden. Nodig: omschrijving, aanschafdatum, aanschafwaarde en restwaarde; groep_id als er meer dan één groep is (vaste_activa_groepen). Aantal jaren en de twee rekeningen komen standaard van de groep. e-Boekhouden maakt er een afschrijfschema bij; de aanschaf zelf wordt hiermee niet geboekt en de afschrijvingen pas met afschrijvingen_boeken. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt het eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Vast activum toevoegen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn vastactief_toevoegen(
        &self,
        Parameters(args): Parameters<VastactiefToevoegenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "vastactief_toevoegen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Vast activum toegevoegd; data.id is het id",
            move |this, run, args| async move {
                let ctx = this.activa_context(&run).await?;
                let built = vastactief_nieuw(&args, &ctx)?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change(&run, "POST", "vasteactiva", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een vast activum wijzigen, op id (uit vaste_activa). Wat je niet noemt blijft zoals het is. Het afschrijfschema blijft staan, tenzij schema_opnieuw true is: dan maakt e-Boekhouden het opnieuw op met de nieuwe gegevens, zoals het vinkje Afschrijfschema opnieuw opmaken. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Vast activum wijzigen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn vastactief_wijzigen(
        &self,
        Parameters(args): Parameters<VastactiefWijzigenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "vastactief_wijzigen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Vast activum gewijzigd",
            move |this, run, args| async move {
                let now = this.vastactief_lezen(&run, args.id).await?;
                let ctx = this.activa_context(&run).await?;
                let built = vastactief_gewijzigd(&args, &now, &ctx)?;
                this.approve(&run, TOOL, &built, false).await?;
                let recalc = args.schema_opnieuw.unwrap_or(false);
                this.change_with_query(
                    &run,
                    "PUT",
                    "vasteactiva",
                    &[("recalc".to_owned(), recalc.to_string())],
                    Some(&built.body),
                )
                .await
            },
        )
        .await
    }

    #[tool(
        description = "Een vast activum verwijderen, op id (uit vaste_activa). De goedkeuring laat zien hoeveel van het afschrijfschema al geboekt is. Dit is niet terug te draaien. De gebruiker moet dit altijd goedkeuren in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Vast activum verwijderen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn vastactief_verwijderen(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "vastactief_verwijderen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Vast activum verwijderd",
            move |this, run, args| async move {
                let now = this.vastactief_lezen(&run, args.id).await?;
                let ctx = this.activa_context(&run).await?;
                let schema = rows_of(
                    this.fetch(
                        &run,
                        &format!("vasteactivamutatie/gridtable/{}", args.id),
                        grid(Some(MAX_ROWS), None)?,
                    )
                    .await?,
                );
                let built = vastactief_weg(&now, &schema, &ctx)?;
                this.approve(&run, TOOL, &built, true).await?;
                this.change_done(&run, "DELETE", &format!("vasteactiva/{}", args.id), None)
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Alle afschrijvingen boeken die klaarstaan, zoals Afschrijvingen boeken in e-Boekhouden: elke klaarstaande afschrijving van elk vast activum wordt in één keer als mutatie geboekt. Eén activum kiezen kan niet; vaste_activa zegt hoeveel er klaarstaan. De gebruiker moet dit altijd goedkeuren in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Afschrijvingen boeken",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn afschrijvingen_boeken(
        &self,
        Parameters(args): Parameters<AccountArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "afschrijvingen_boeken";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Afschrijvingen geboekt",
            move |this, run, _args| async move {
                let due = this.afschrijvingen_klaar(&run).await?;
                let built = afschrijvingen_boeken_built(due)?;
                this.approve(&run, TOOL, &built, true).await?;
                this.change(&run, "POST", "vasteactiva/boeken", None).await
            },
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ActivaContext {
        ActivaContext {
            groepen: groepen(&json!([
                {"id": 3, "naam": "Computers", "aantalJaren": 3, "grootboekrekeningActivaId": 130, "grootboekrekeningAfschrijvingId": 470},
                {"id": 4, "naam": "Inventaris", "aantalJaren": 5, "grootboekrekeningActivaId": 131, "grootboekrekeningAfschrijvingId": 471}
            ])),
            balans: keuzes(&json!([
                {"id": 130, "code": "0130", "omschrijving": "Computers"},
                {"id": 131, "code": "0140", "omschrijving": "Inventaris"}
            ])),
            kosten: keuzes(&json!([
                {"id": 470, "code": "4700", "omschrijving": "Afschrijving computers"},
                {"id": 471, "code": "4710", "omschrijving": "Afschrijving inventaris"}
            ])),
        }
    }

    fn toevoegen() -> VastactiefToevoegenArgs {
        VastactiefToevoegenArgs {
            groep_id: Some(3),
            omschrijving: " MacBook Pro ".into(),
            aanschafdatum: "2026-03-15".into(),
            aanschafwaarde: 2499.0,
            restwaarde: 0.0,
            afschrijven_in_jaren: None,
            boeken_per: None,
            rekening_activa: None,
            rekening_afschrijvingen: None,
            afschrijven_vanaf: None,
            boekwaarde_bij_start: None,
            account: None,
        }
    }

    fn label_values(built: &Built) -> Vec<(String, String)> {
        built
            .lines
            .iter()
            .map(|l| (l.label.clone(), l.value.clone()))
            .collect()
    }

    #[test]
    fn makes_an_asset_as_e_boekhoudens_form_does_with_the_groups_defaults() {
        let built = vastactief_nieuw(&toevoegen(), &ctx()).unwrap();
        assert_eq!(
            built.body,
            json!({"vasteActivaGroepId": 3, "omschrijving": "MacBook Pro", "aanschafDatum": {"day": 15, "month": 3, "year": 2026},
                   "aanschafwaarde": 2499.0, "restwaarde": 0.0, "aantalJaren": 3, "boekenPer": "J",
                   "grootboekrekeningBal": 130, "grootboekrekeningVw": 470, "geavanceerd": false})
        );
        assert_eq!(built.title, "Vast activum toevoegen: MacBook Pro");
        let lines = label_values(&built);
        assert!(lines.contains(&("Groep".into(), "Computers".into())));
        assert!(lines.contains(&("Aanschafdatum".into(), "15-03-2026".into())));
        assert!(lines.contains(&("Aanschafwaarde".into(), "2499,00".into())));
        assert!(lines.contains(&(
            "Rekening afschrijvingen".into(),
            "4700 Afschrijving computers".into()
        )));
        assert!(!lines.iter().any(|(l, _)| l == "Let op"));
    }

    #[test]
    fn an_asset_with_its_own_choices_and_an_advanced_start() {
        let mut args = toevoegen();
        args.groep_id = Some(4);
        args.afschrijven_in_jaren = Some(7);
        args.boeken_per = Some(BoekenPer::Maand);
        args.rekening_activa = Some("0130".into());
        args.afschrijven_vanaf = Some("2026-07-01".into());
        args.boekwaarde_bij_start = Some(1999.999);
        args.restwaarde = 2499.0;
        let built = vastactief_nieuw(&args, &ctx()).unwrap();
        assert_eq!(built.body["vasteActivaGroepId"], 4);
        assert_eq!(built.body["aantalJaren"], 7);
        assert_eq!(built.body["boekenPer"], "M");
        assert_eq!(built.body["grootboekrekeningBal"], 130);
        assert_eq!(built.body["grootboekrekeningVw"], 471);
        assert_eq!(
            built.body["afschrijvenStartenPer"],
            json!({"day": 1, "month": 7, "year": 2026})
        );
        assert_eq!(built.body["boekwaardePerStart"], json!(2000.0));
        assert_eq!(built.body["geavanceerd"], true);
        assert!(
            label_values(&built)
                .iter()
                .any(|(l, v)| l == "Let op" && v.contains("niets afgeschreven"))
        );
    }

    #[test]
    fn refuses_what_the_form_would_refuse() {
        let ctx = ctx();
        let mut args = toevoegen();
        args.groep_id = None;
        assert!(
            vastactief_nieuw(&args, &ctx)
                .unwrap_err()
                .contains("groep_id")
        );
        let mut args = toevoegen();
        args.groep_id = Some(9);
        assert!(
            vastactief_nieuw(&args, &ctx)
                .unwrap_err()
                .contains("Computers")
        );
        let mut args = toevoegen();
        args.restwaarde = 3000.0;
        assert!(
            vastactief_nieuw(&args, &ctx)
                .unwrap_err()
                .contains("restwaarde")
        );
        let mut args = toevoegen();
        args.afschrijven_in_jaren = Some(0);
        let error = vastactief_nieuw(&args, &ctx).unwrap_err();
        assert!(error.contains("groter zijn dan 0"), "{error}");
        let mut args = toevoegen();
        args.omschrijving = "  ".into();
        assert!(
            vastactief_nieuw(&args, &ctx)
                .unwrap_err()
                .contains("omschrijving")
        );
        let mut args = toevoegen();
        args.aanschafdatum = "15-03-2026".into();
        assert!(vastactief_nieuw(&args, &ctx).is_err());
        // A cost account is no place for the asset itself.
        let mut args = toevoegen();
        args.rekening_activa = Some("4700".into());
        assert!(vastactief_nieuw(&args, &ctx).unwrap_err().contains("4700"));
        // One group needs no name; none at all is refused with where to make one.
        let mut one = ActivaContext {
            groepen: ctx.groepen[..1].to_vec(),
            balans: ctx.balans.clone(),
            kosten: ctx.kosten.clone(),
        };
        let mut args = toevoegen();
        args.groep_id = None;
        assert_eq!(
            vastactief_nieuw(&args, &one).unwrap().body["vasteActivaGroepId"],
            3
        );
        one.groepen.clear();
        assert!(
            vastactief_nieuw(&args, &one)
                .unwrap_err()
                .contains("Groepen beheren")
        );
        // A group whose rekening is no longer active must be overruled.
        let mut stale = ActivaContext {
            groepen: ctx.groepen.clone(),
            balans: ctx.balans[1..].to_vec(),
            kosten: ctx.kosten.clone(),
        };
        assert!(
            vastactief_nieuw(&toevoegen(), &stale)
                .unwrap_err()
                .contains("rekening_activa")
        );
        stale.balans = ctx.balans.clone();
        assert!(vastactief_nieuw(&toevoegen(), &stale).is_ok());
    }

    fn now() -> Value {
        json!({"id": 12, "vasteActivaGroepId": 3, "omschrijving": "MacBook Pro", "aanschafDatum": "2026-03-15T00:00:00", "aanschafwaarde": 2499,
               "restwaarde": 0, "aantalJaren": 3, "boekenPer": "J", "grootboekrekeningBal": 130, "grootboekrekeningVw": 470,
               "afschrijvenStartenPer": null, "boekwaardePerStart": null, "geavanceerd": false, "code": "VA1"})
    }

    fn wijzigen() -> VastactiefWijzigenArgs {
        VastactiefWijzigenArgs {
            id: 12,
            groep_id: None,
            omschrijving: None,
            aanschafdatum: None,
            aanschafwaarde: None,
            restwaarde: None,
            afschrijven_in_jaren: None,
            boeken_per: None,
            rekening_activa: None,
            rekening_afschrijvingen: None,
            afschrijven_vanaf: None,
            boekwaarde_bij_start: None,
            schema_opnieuw: None,
            account: None,
        }
    }

    #[test]
    fn an_edit_sends_the_whole_asset_back_with_only_the_named_change() {
        let mut args = wijzigen();
        args.restwaarde = Some(250.0);
        let built = vastactief_gewijzigd(&args, &now(), &ctx()).unwrap();
        assert_eq!(
            built.body,
            json!({"id": 12, "vasteActivaGroepId": 3, "omschrijving": "MacBook Pro", "aanschafDatum": {"day": 15, "month": 3, "year": 2026},
                   "aanschafwaarde": 2499.0, "restwaarde": 250.0, "aantalJaren": 3, "boekenPer": "J",
                   "grootboekrekeningBal": 130, "grootboekrekeningVw": 470, "geavanceerd": false})
        );
        let lines = label_values(&built);
        assert_eq!(lines[0], ("Vast activum".into(), "MacBook Pro".into()));
        assert!(lines.contains(&("Restwaarde was".into(), "0,00".into())));
        assert!(lines.contains(&("Restwaarde wordt".into(), "250,00".into())));
        assert!(
            lines
                .iter()
                .any(|(l, v)| l == "Afschrijfschema" && v.contains("blijft"))
        );
    }

    #[test]
    fn a_new_group_brings_its_years_and_rekeningen_unless_named() {
        let mut args = wijzigen();
        args.groep_id = Some(4);
        args.afschrijven_in_jaren = Some(10);
        args.schema_opnieuw = Some(true);
        let built = vastactief_gewijzigd(&args, &now(), &ctx()).unwrap();
        assert_eq!(built.body["vasteActivaGroepId"], 4);
        assert_eq!(built.body["aantalJaren"], 10);
        assert_eq!(built.body["grootboekrekeningBal"], 131);
        assert_eq!(built.body["grootboekrekeningVw"], 471);
        let lines = label_values(&built);
        assert!(lines.contains(&("Groep wordt".into(), "Inventaris".into())));
        assert!(
            lines
                .iter()
                .any(|(l, v)| l == "Afschrijfschema" && v.contains("opnieuw opgemaakt"))
        );
    }

    #[test]
    fn an_edit_refuses_nothing_to_do_but_allows_a_new_schedule_alone() {
        assert!(
            vastactief_gewijzigd(&wijzigen(), &now(), &ctx())
                .unwrap_err()
                .contains("niets gewijzigd")
        );
        let mut args = wijzigen();
        args.schema_opnieuw = Some(true);
        assert!(vastactief_gewijzigd(&args, &now(), &ctx()).is_ok());
        let mut args = wijzigen();
        args.aanschafwaarde = Some(-1.0);
        assert!(
            vastactief_gewijzigd(&args, &now(), &ctx())
                .unwrap_err()
                .contains("restwaarde")
        );
        assert!(vastactief_gewijzigd(&wijzigen(), &json!({"id": 12}), &ctx()).is_err());
    }

    #[test]
    fn deleting_says_how_much_was_booked_and_booking_says_how_many() {
        let schema = vec![
            json!({"datum": "2026-12-31T00:00:00", "bedrag": 833, "mutatieNr": 4012}),
            json!({"datum": "2027-12-31T00:00:00", "bedrag": 833, "mutatieNr": null}),
        ];
        let built = vastactief_weg(&now(), &schema, &ctx()).unwrap();
        assert_eq!(built.title, "Vast activum verwijderen: MacBook Pro");
        let lines = label_values(&built);
        assert!(lines.contains(&(
            "Afschrijfschema".into(),
            "2 regels, waarvan 1 al geboekt als mutatie 4012".into()
        )));
        assert!(
            lines
                .iter()
                .any(|(l, v)| l == "Gevolg" && v.contains("niet terug te draaien"))
        );
        let built = vastactief_weg(&now(), &[], &ctx()).unwrap();
        assert!(label_values(&built).contains(&(
            "Afschrijfschema".into(),
            "0 regels, nog geen geboekt".into()
        )));

        let built = afschrijvingen_boeken_built(4).unwrap();
        assert_eq!(built.title, "4 afschrijvingen boeken");
        assert_eq!(built.body, Value::Null);
        assert!(
            afschrijvingen_boeken_built(0)
                .unwrap_err()
                .contains("geen afschrijvingen")
        );
    }
}
