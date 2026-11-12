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
    /// Geavanceerd: afschrijven starten per deze datum, JJJJ-MM-DD (voor een vast actief dat al eerder in gebruik was).
    pub afschrijven_vanaf: Option<String>,
    /// Geavanceerd: de boekwaarde in euro op de startdatum van het afschrijven.
    pub boekwaarde_bij_start: Option<f64>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct VastactiefWijzigenArgs {
    /// Id van het vaste actief (uit vaste_activa).
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
    /// Id van het vaste actief (uit vaste_activa).
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

/// One vaste actief, as the form holds it.
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
            return Err("Een vast actief heeft een omschrijving nodig".into());
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
        // Every value here goes back to e-Boekhouden on an edit: one that cannot be read stops the edit.
        let bad = |veld: &str| onleesbaar("dit vaste actief", veld);
        let number = |key: &str| value.get(key).and_then(read_u64).ok_or_else(|| bad(key));
        let amount = |key: &str| value.get(key).and_then(read_f64).ok_or_else(|| bad(key));
        Ok(Self {
            groep: number("vasteActivaGroepId")?,
            omschrijving: value
                .get("omschrijving")
                .and_then(Value::as_str)
                .ok_or_else(|| bad("omschrijving"))?
                .trim()
                .to_owned(),
            aanschafdatum: value
                .get("aanschafDatum")
                .and_then(date_of)
                .ok_or_else(|| bad("aanschafDatum"))?,
            aanschafwaarde: amount("aanschafwaarde")?,
            restwaarde: amount("restwaarde")?,
            jaren: u32::try_from(number("aantalJaren")?).map_err(|_| bad("aantalJaren"))?,
            per: value
                .get("boekenPer")
                .and_then(Value::as_str)
                .and_then(BoekenPer::from_code)
                .ok_or_else(|| bad("boekenPer"))?,
            activa: number("grootboekrekeningBal")?,
            afschrijving: number("grootboekrekeningVw")?,
            start: optional(value.get("afschrijvenStartenPer"), date_of)
                .map_err(|()| bad("afschrijvenStartenPer"))?,
            boekwaarde_start: optional(value.get("boekwaardePerStart"), read_f64)
                .map_err(|()| bad("boekwaardePerStart"))?,
            geavanceerd: value
                .get("geavanceerd")
                .and_then(read_bool)
                .ok_or_else(|| bad("geavanceerd"))?,
        })
    }
}

/// A purchase date: a real date on or after 1-1-1900, as the form's date validator wants it.
fn aanschafdatum(text: &str) -> Result<Date, String> {
    let early = text
        .trim()
        .get(..4)
        .and_then(|y| y.parse::<i32>().ok())
        .is_some_and(|y| y < 1900);
    if early {
        return Err("De aanschafdatum moet op of na 01-01-1900 liggen".into());
    }
    parse_date(text)
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
                    "Geef met groep_id aan in welke groep het vaste actief hoort: {}",
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
        aanschafdatum: aanschafdatum(&args.aanschafdatum)?,
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
        title: format!("Vast actief toevoegen: {}", activum.omschrijving),
        lines,
    })
}

fn vastactief_gewijzigd(
    args: &VastactiefWijzigenArgs,
    now: &Value,
    ctx: &ActivaContext,
    schema: Option<&Schema>,
) -> Result<Built, String> {
    let before = Activum::from_api(now)?;
    let mut activum = before.clone();
    if let Some(id) = args.groep_id
        && id != before.groep
    {
        // As the form does when the group changes: its years and rekeningen come along.
        // A value the group lacks is emptied by the form; here it must be named instead.
        let groep = ctx.groep(id)?;
        activum.groep = groep.id;
        let missing = |wat: &str, arg: &str| {
            format!(
                "Groep {} heeft geen {wat}; e-Boekhouden maakt dat veld dan leeg. Geef {arg} mee",
                groep.naam
            )
        };
        activum.jaren = match (groep.jaren, args.afschrijven_in_jaren) {
            (_, Some(jaren)) | (Some(jaren), None) => jaren,
            (None, None) => return Err(missing("aantal jaren", "afschrijven_in_jaren")),
        };
        activum.activa = match (groep.activa, &args.rekening_activa) {
            (Some(activa), None) => activa,
            (_, Some(_)) => activum.activa,
            (None, None) => return Err(missing("rekening activa", "rekening_activa")),
        };
        activum.afschrijving = match (groep.afschrijving, &args.rekening_afschrijvingen) {
            (Some(afschrijving), None) => afschrijving,
            (_, Some(_)) => activum.afschrijving,
            (None, None) => {
                return Err(missing(
                    "rekening afschrijvingen",
                    "rekening_afschrijvingen",
                ));
            }
        };
    }
    if let Some(text) = &args.omschrijving {
        activum.omschrijving = text.trim().to_owned();
    }
    if let Some(date) = &args.aanschafdatum {
        activum.aanschafdatum = aanschafdatum(date)?;
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
    let mut lines = vec![line("Vast actief", &before.omschrijving)];
    lines.extend(changed);
    lines.extend(activum.warning());
    match (recalc, schema) {
        (true, Some(schema)) => {
            lines.push(line("Afschrijfschema nu", schema.beschrijving()));
            lines.push(line(
                "Afschrijfschema",
                "wordt helemaal opnieuw opgemaakt met deze gegevens. Wat er met al geboekte of met de hand aangepaste regels gebeurt, bepaalt e-Boekhouden; EB MCP kan dat niet vooraf laten zien.",
            ));
        }
        (true, None) => {
            return Err("EB MCP kon het huidige afschrijfschema niet lezen; zonder dat wordt het niet opnieuw opgemaakt".into());
        }
        (false, _) => lines.push(line(
            "Afschrijfschema",
            "blijft zoals het is (schema_opnieuw staat uit)",
        )),
    }
    let id = now.get("id").and_then(Value::as_u64).unwrap_or(args.id);
    Ok(Built {
        body: activum.body(Some(id)),
        title: format!("Vast actief wijzigen: {}", before.omschrijving),
        lines,
    })
}

/// The afschrijfschema of one asset: its rows as read, and how many e-Boekhouden says there are.
#[derive(Clone, Debug, PartialEq)]
struct Schema {
    aantal: u64,
    rows: Vec<Value>,
}

impl Schema {
    fn geboekt(&self) -> Vec<u64> {
        self.rows
            .iter()
            .filter_map(|row| row.get("mutatieNr").and_then(read_u64))
            .filter(|nr| *nr > 0)
            .collect()
    }

    /// "12 regels, waarvan 2 al geboekt als mutatie 4012, 4388", saying so when not every row could be read.
    fn beschrijving(&self) -> String {
        let booked = self.geboekt();
        let mut text = if booked.is_empty() {
            format!("{} regels, nog geen geboekt", self.aantal)
        } else {
            format!(
                "{} regels, waarvan {} al geboekt als mutatie {}",
                self.aantal,
                booked.len(),
                booked
                    .iter()
                    .map(u64::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        if (self.rows.len() as u64) < self.aantal {
            text.push_str(&format!(
                " (EB MCP las de eerste {} regels)",
                self.rows.len()
            ));
        }
        text
    }

    /// The rows with a date up to `vandaag` that were not carried out yet: date and amount. A row whose date or
    /// amount cannot be read is refused, not skipped.
    fn klaar(&self, vandaag: Date) -> Result<Vec<(Date, f64)>, String> {
        let mut due = Vec::new();
        for row in &self.rows {
            let uitgevoerd = match row.get("datumUitgevoerd") {
                None | Some(Value::Null) => false,
                Some(Value::String(s)) => !s.trim().is_empty(),
                Some(_) => true,
            };
            if uitgevoerd {
                continue;
            }
            let datum = row
                .get("datum")
                .and_then(date_of)
                .ok_or_else(|| onleesbaar("een regel van het afschrijfschema", "datum"))?;
            if datum > vandaag {
                continue;
            }
            let bedrag = row
                .get("bedrag")
                .and_then(read_f64)
                .ok_or_else(|| onleesbaar("een regel van het afschrijfschema", "bedrag"))?;
            due.push((datum, bedrag));
        }
        Ok(due)
    }
}

/// The approval for deleting an asset: what it is, and how much of its schedule was booked already.
fn vastactief_weg(now: &Value, schema: &Schema, ctx: &ActivaContext) -> Result<Built, String> {
    let activum = Activum::from_api(now)?;
    let mut lines: Vec<approvals::Line> = activum
        .fields(ctx)
        .into_iter()
        .map(|(label, value)| line(label, value))
        .collect();
    lines.push(line("Afschrijfschema", schema.beschrijving()));
    lines.push(line(
        "Gevolg",
        "het vaste actief verdwijnt uit e-Boekhouden; dit is niet terug te draaien",
    ));
    Ok(Built {
        body: Value::Null,
        title: format!("Vast actief verwijderen: {}", activum.omschrijving),
        lines,
    })
}

/// How many assets and dates the approval for booking lists before it sums up the rest.
const TOON_ACTIVA: usize = 20;
const TOON_DATUMS: usize = 6;

/// The approval for booking the due depreciations: per asset its due lines (date and amount) and the total, as
/// read from each schedule (a date up to today, not carried out yet), next to e-Boekhouden's own count.
fn afschrijvingen_boeken_built(
    due: u64,
    per_activum: &[(String, Vec<(Date, f64)>)],
) -> Result<Built, String> {
    if due == 0 {
        return Err("Er staan geen afschrijvingen klaar om te boeken".into());
    }
    let with_lines: Vec<&(String, Vec<(Date, f64)>)> =
        per_activum.iter().filter(|(_, d)| !d.is_empty()).collect();
    let mut lines = vec![line(
        "Klaar volgens e-Boekhouden",
        format!("{due} afschrijvingen"),
    )];
    for (naam, regels) in with_lines.iter().take(TOON_ACTIVA) {
        let mut parts: Vec<String> = regels
            .iter()
            .take(TOON_DATUMS)
            .map(|(datum, bedrag)| format!("{}: {}", dutch(*datum), write::euro(*bedrag)))
            .collect();
        if regels.len() > TOON_DATUMS {
            parts.push(format!("en nog {} regels", regels.len() - TOON_DATUMS));
        }
        let som: f64 = regels.iter().map(|(_, b)| b).sum();
        lines.push(line(
            naam,
            format!("{} (samen {})", parts.join("; "), write::euro(som)),
        ));
    }
    if with_lines.len() > TOON_ACTIVA {
        let rest = &with_lines[TOON_ACTIVA..];
        let count: usize = rest.iter().map(|(_, r)| r.len()).sum();
        let som: f64 = rest
            .iter()
            .flat_map(|(_, r)| r.iter().map(|(_, b)| b))
            .sum();
        lines.push(line(
            "Nog meer vaste activa",
            format!(
                "{} vaste activa met {count} afschrijvingen, samen {}",
                rest.len(),
                write::euro(som)
            ),
        ));
    }
    let found: usize = with_lines.iter().map(|(_, r)| r.len()).sum();
    let totaal: f64 = with_lines
        .iter()
        .flat_map(|(_, r)| r.iter().map(|(_, b)| b))
        .sum();
    lines.push(line(
        "Totaal",
        format!("{found} afschrijvingen, samen {}", write::euro(totaal)),
    ));
    if found as u64 != due {
        lines.push(line(
            "Let op",
            format!(
                "EB MCP vond in de afschrijfschema's {found} regels met een datum tot en met vandaag die nog niet geboekt zijn; e-Boekhouden telt er {due} en boekt zijn eigen telling"
            ),
        ));
    }
    lines.push(line(
        "Gevolg",
        "e-Boekhouden boekt alle afschrijvingen die klaarstaan, van alle vaste activa, in één keer als mutatie",
    ));
    lines.push(line(
        "Terugdraaien",
        "kan alleen door elke geboekte mutatie apart te verwijderen",
    ));
    Ok(Built {
        body: Value::Null,
        title: format!("{due} afschrijvingen boeken"),
        lines,
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
            return Err(format!("Er is geen vast actief met id {id}"));
        }
        Ok(now)
    }

    /// The afschrijfschema of one asset, every row of it.
    async fn schema_lezen(&self, run: &Run, id: u64) -> Result<Schema, String> {
        let (aantal, rows) = self
            .alle_rijen(run, &format!("vasteactivamutatie/gridtable/{id}"))
            .await?;
        Ok(Schema { aantal, rows })
    }

    /// Per asset its omschrijving and the schedule rows that are due by date and not carried out yet.
    async fn klaarstaand(&self, run: &Run) -> Result<Vec<(String, Vec<(Date, f64)>)>, String> {
        let (_, activa) = self.alle_rijen(run, "vasteactiva/gridtable").await?;
        let vandaag = today();
        let mut per_activum = Vec::new();
        for row in activa {
            let id = row
                .get("id")
                .and_then(read_u64)
                .ok_or_else(|| onleesbaar("de lijst met vaste activa", "id"))?;
            let naam = row
                .get("omschrijving")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("Vast actief {id}"));
            let due = self.schema_lezen(run, id).await?.klaar(vandaag)?;
            per_activum.push((naam, due));
        }
        Ok(per_activum)
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
        description = "De vaste activa (bedrijfsmiddelen die worden afgeschreven). Per actief: id, naam (groep), omschrijving, datum (aanschaf), aanschafWaarde, afgeschreven, afTeSchrijven en huidigeWaarde. afschrijvingen_te_boeken zegt hoeveel afschrijvingen klaarstaan om te boeken (afschrijvingen_boeken).",
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
        description = "Eén vast actief, op id (uit vaste_activa): groep, omschrijving, aanschafdatum, aanschafwaarde, restwaarde, aantal jaren, boeken per jaar of maand, de rekening activa en de rekening afschrijvingen met code en naam, en de geavanceerde start van het afschrijven. Het schema geeft afschrijvingsschema.",
        annotations(title = "Vast actief", read_only_hint = true)
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
                "vast_actief": now,
                "groep": ctx.groep_naam(id("vasteActivaGroepId")),
                "rekening_activa": rekening_naam(&ctx.balans, id("grootboekrekeningBal")),
                "rekening_afschrijvingen": rekening_naam(&ctx.kosten, id("grootboekrekeningVw")),
            }))
        }
        .await;
        Ok(self.end(run, &path, result))
    }

    #[tool(
        description = "De vaste-activagroepen: per groep id, naam, aantalJaren en de standaard rekening activa (grootboekrekeningActivaId) en rekening afschrijvingen (grootboekrekeningAfschrijvingId). Een nieuw vast actief hoort in een groep en neemt die waarden over.",
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
        description = "Het afschrijfschema van één vast actief, op id (uit vaste_activa): per regel datum, omschrijving, bedrag, datumUitgevoerd (wanneer geboekt) en mutatieNr (de mutatie van de boeking, leeg zolang die niet geboekt is).",
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
        description = "Een vast actief aanmaken, zoals Vaste activa toevoegen in e-Boekhouden. Nodig: omschrijving, aanschafdatum, aanschafwaarde en restwaarde; groep_id als er meer dan één groep is (vaste_activa_groepen). Aantal jaren en de twee rekeningen komen standaard van de groep. e-Boekhouden maakt er een afschrijfschema bij; de aanschaf zelf wordt hiermee niet geboekt en de afschrijvingen pas met afschrijvingen_boeken. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt het eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Vast actief toevoegen",
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
            "Vast actief toegevoegd; data.id is het id",
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
        description = "Een vast actief wijzigen, op id (uit vaste_activa). Wat je niet noemt blijft zoals het is. Het afschrijfschema blijft staan, tenzij schema_opnieuw true is: dan maakt e-Boekhouden het opnieuw op met de nieuwe gegevens, zoals het vinkje Afschrijfschema opnieuw opmaken. Dat vervangt het hele schema, ook regels die al geboekt of met de hand aangepast zijn, en vraagt daarom altijd het akkoord van de gebruiker; de goedkeuring toont het huidige schema. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Vast actief wijzigen",
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
            "Vast actief gewijzigd",
            move |this, run, args| async move {
                let now = this.vastactief_lezen(&run, args.id).await?;
                let ctx = this.activa_context(&run).await?;
                let recalc = args.schema_opnieuw.unwrap_or(false);
                // A new schedule replaces the old one, booked lines and all: it always asks, and shows the schedule.
                let schema = if recalc {
                    Some(this.schema_lezen(&run, args.id).await?)
                } else {
                    None
                };
                let built = vastactief_gewijzigd(&args, &now, &ctx, schema.as_ref())?;
                this.approve(&run, TOOL, &built, recalc).await?;
                this.change_query_done(
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
        description = "Een vast actief verwijderen, op id (uit vaste_activa). De goedkeuring laat zien hoeveel van het afschrijfschema al geboekt is. Dit is niet terug te draaien. De gebruiker moet dit altijd goedkeuren in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Vast actief verwijderen",
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
            "Vast actief verwijderd",
            move |this, run, args| async move {
                let now = this.vastactief_lezen(&run, args.id).await?;
                let ctx = this.activa_context(&run).await?;
                let schema = this.schema_lezen(&run, args.id).await?;
                let built = vastactief_weg(&now, &schema, &ctx)?;
                this.approve(&run, TOOL, &built, true).await?;
                this.change_done(&run, "DELETE", &format!("vasteactiva/{}", args.id), None)
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Alle afschrijvingen boeken die klaarstaan, zoals Afschrijvingen boeken in e-Boekhouden: elke klaarstaande afschrijving van elk vast actief wordt in één keer als mutatie geboekt. Eén vast actief kiezen kan niet; vaste_activa zegt hoeveel er klaarstaan. De goedkeuring toont per vast actief de regels uit het afschrijfschema met een datum tot en met vandaag die nog niet geboekt zijn, met datum, bedrag en totaal. De gebruiker moet dit altijd goedkeuren in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
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
                if due == 0 {
                    return Err(Halt::Error(
                        "Er staan geen afschrijvingen klaar om te boeken".into(),
                    ));
                }
                let per_activum = this.klaarstaand(&run).await?;
                let built = afschrijvingen_boeken_built(due, &per_activum)?;
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
        assert_eq!(built.title, "Vast actief toevoegen: MacBook Pro");
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

    fn schema() -> Schema {
        Schema {
            aantal: 2,
            rows: vec![
                json!({"datum": "2026-12-31T00:00:00", "bedrag": 833, "mutatieNr": 4012, "datumUitgevoerd": "2026-12-31T00:00:00"}),
                json!({"datum": "2027-12-31T00:00:00", "bedrag": 833, "mutatieNr": null, "datumUitgevoerd": null}),
            ],
        }
    }

    #[test]
    fn an_edit_sends_the_whole_asset_back_with_only_the_named_change() {
        let mut args = wijzigen();
        args.restwaarde = Some(250.0);
        let built = vastactief_gewijzigd(&args, &now(), &ctx(), None).unwrap();
        assert_eq!(
            built.body,
            json!({"id": 12, "vasteActivaGroepId": 3, "omschrijving": "MacBook Pro", "aanschafDatum": {"day": 15, "month": 3, "year": 2026},
                   "aanschafwaarde": 2499.0, "restwaarde": 250.0, "aantalJaren": 3, "boekenPer": "J",
                   "grootboekrekeningBal": 130, "grootboekrekeningVw": 470, "geavanceerd": false})
        );
        let lines = label_values(&built);
        assert_eq!(lines[0], ("Vast actief".into(), "MacBook Pro".into()));
        assert!(lines.contains(&("Restwaarde was".into(), "0,00".into())));
        assert!(lines.contains(&("Restwaarde wordt".into(), "250,00".into())));
        assert!(
            lines
                .iter()
                .any(|(l, v)| l == "Afschrijfschema" && v.contains("blijft"))
        );
    }

    #[test]
    fn an_edit_sends_back_what_it_read_monthly_and_advanced_included() {
        let mut now = now();
        now["boekenPer"] = json!("M");
        now["geavanceerd"] = json!(1);
        now["restwaarde"] = json!("100.50");
        now["afschrijvenStartenPer"] = json!({"day": 1, "month": 7, "year": 2026});
        now["boekwaardePerStart"] = json!(2000);
        let mut args = wijzigen();
        args.omschrijving = Some("MacBook Pro 16".into());
        let built = vastactief_gewijzigd(&args, &now, &ctx(), None).unwrap();
        assert_eq!(built.body["boekenPer"], "M");
        assert_eq!(built.body["geavanceerd"], true);
        assert_eq!(built.body["restwaarde"], json!(100.5));
        assert_eq!(
            built.body["afschrijvenStartenPer"],
            json!({"day": 1, "month": 7, "year": 2026})
        );
        assert_eq!(built.body["boekwaardePerStart"], json!(2000.0));
    }

    #[test]
    fn an_asset_with_a_value_it_cannot_read_is_not_edited() {
        for (key, value) in [
            ("restwaarde", Value::Null),
            ("restwaarde", json!("nul")),
            ("boekenPer", Value::Null),
            ("boekenPer", json!("K")),
            ("geavanceerd", Value::Null),
            ("geavanceerd", json!(2)),
            ("grootboekrekeningBal", Value::Null),
            ("vasteActivaGroepId", json!("x")),
            ("afschrijvenStartenPer", json!("later")),
            ("boekwaardePerStart", json!("veel")),
            ("aanschafwaarde", json!(true)),
        ] {
            let mut read = now();
            read[key] = value.clone();
            let error = Activum::from_api(&read).unwrap_err();
            assert!(
                error.contains(key) && error.contains("onbekende vorm"),
                "{key} {value}: {error}"
            );
            let mut args = wijzigen();
            args.omschrijving = Some("nieuw".into());
            assert!(vastactief_gewijzigd(&args, &read, &ctx(), None).is_err());
        }
    }

    #[test]
    fn a_new_group_brings_its_years_and_rekeningen_unless_named() {
        let mut args = wijzigen();
        args.groep_id = Some(4);
        args.afschrijven_in_jaren = Some(10);
        args.schema_opnieuw = Some(true);
        let built = vastactief_gewijzigd(&args, &now(), &ctx(), Some(&schema())).unwrap();
        assert_eq!(built.body["vasteActivaGroepId"], 4);
        assert_eq!(built.body["aantalJaren"], 10);
        assert_eq!(built.body["grootboekrekeningBal"], 131);
        assert_eq!(built.body["grootboekrekeningVw"], 471);
        let lines = label_values(&built);
        assert!(lines.contains(&("Groep wordt".into(), "Inventaris".into())));
        assert!(lines.contains(&(
            "Afschrijfschema nu".into(),
            "2 regels, waarvan 1 al geboekt als mutatie 4012".into()
        )));
        assert!(
            lines
                .iter()
                .any(|(l, v)| l == "Afschrijfschema" && v.contains("helemaal opnieuw opgemaakt"))
        );
        // Without the current schedule there is no new one.
        assert!(
            vastactief_gewijzigd(&args, &now(), &ctx(), None)
                .unwrap_err()
                .contains("afschrijfschema")
        );
    }

    #[test]
    fn a_group_without_accounts_needs_them_named() {
        let ctx = ActivaContext {
            groepen: groepen(&json!([
                {"id": 3, "naam": "Computers", "aantalJaren": 3, "grootboekrekeningActivaId": 130, "grootboekrekeningAfschrijvingId": 470},
                {"id": 5, "naam": "Leeg", "aantalJaren": null, "grootboekrekeningActivaId": 0, "grootboekrekeningAfschrijvingId": null}
            ])),
            ..ctx()
        };
        let mut args = wijzigen();
        args.groep_id = Some(5);
        let error = vastactief_gewijzigd(&args, &now(), &ctx, None).unwrap_err();
        assert!(error.contains("afschrijven_in_jaren"), "{error}");
        args.afschrijven_in_jaren = Some(4);
        let error = vastactief_gewijzigd(&args, &now(), &ctx, None).unwrap_err();
        assert!(error.contains("rekening_activa"), "{error}");
        args.rekening_activa = Some("0140".into());
        let error = vastactief_gewijzigd(&args, &now(), &ctx, None).unwrap_err();
        assert!(error.contains("rekening_afschrijvingen"), "{error}");
        args.rekening_afschrijvingen = Some("4710".into());
        let built = vastactief_gewijzigd(&args, &now(), &ctx, None).unwrap();
        assert_eq!(built.body["vasteActivaGroepId"], 5);
        assert_eq!(built.body["grootboekrekeningBal"], 131);
        assert_eq!(built.body["grootboekrekeningVw"], 471);
    }

    #[test]
    fn a_purchase_date_before_1900_is_refused() {
        let mut args = toevoegen();
        args.aanschafdatum = "1899-12-31".into();
        assert!(
            vastactief_nieuw(&args, &ctx())
                .unwrap_err()
                .contains("01-01-1900")
        );
        args.aanschafdatum = "1900-01-01".into();
        assert!(vastactief_nieuw(&args, &ctx()).is_ok());
        let mut args = wijzigen();
        args.aanschafdatum = Some("0999-01-01".into());
        assert!(
            vastactief_gewijzigd(&args, &now(), &ctx(), None)
                .unwrap_err()
                .contains("01-01-1900")
        );
    }

    #[test]
    fn an_edit_refuses_nothing_to_do_but_allows_a_new_schedule_alone() {
        assert!(
            vastactief_gewijzigd(&wijzigen(), &now(), &ctx(), None)
                .unwrap_err()
                .contains("niets gewijzigd")
        );
        let mut args = wijzigen();
        args.schema_opnieuw = Some(true);
        assert!(vastactief_gewijzigd(&args, &now(), &ctx(), Some(&schema())).is_ok());
        let mut args = wijzigen();
        args.aanschafwaarde = Some(-1.0);
        assert!(
            vastactief_gewijzigd(&args, &now(), &ctx(), None)
                .unwrap_err()
                .contains("restwaarde")
        );
        assert!(vastactief_gewijzigd(&wijzigen(), &json!({"id": 12}), &ctx(), None).is_err());
    }

    #[test]
    fn deleting_says_how_much_was_booked_by_e_boekhoudens_count() {
        let built = vastactief_weg(&now(), &schema(), &ctx()).unwrap();
        assert_eq!(built.title, "Vast actief verwijderen: MacBook Pro");
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
        let empty = Schema {
            aantal: 0,
            rows: vec![],
        };
        let built = vastactief_weg(&now(), &empty, &ctx()).unwrap();
        assert!(label_values(&built).contains(&(
            "Afschrijfschema".into(),
            "0 regels, nog geen geboekt".into()
        )));
        // More rows than were read: the count is e-Boekhouden's, and it says so.
        let big = Schema {
            aantal: 250,
            ..schema()
        };
        assert_eq!(
            big.beschrijving(),
            "250 regels, waarvan 1 al geboekt als mutatie 4012 (EB MCP las de eerste 2 regels)"
        );
    }

    #[test]
    fn finds_the_due_lines_of_a_schedule() {
        let schema = Schema {
            aantal: 4,
            rows: vec![
                json!({"datum": "2025-12-31T00:00:00", "bedrag": 833, "datumUitgevoerd": "2025-12-31T00:00:00", "mutatieNr": 4012}),
                json!({"datum": "2026-06-30T00:00:00", "bedrag": "416.50", "datumUitgevoerd": null}),
                json!({"datum": {"day": 12, "month": 11, "year": 2026}, "bedrag": 10, "datumUitgevoerd": ""}),
                json!({"datum": "2026-12-31T00:00:00", "bedrag": 833, "datumUitgevoerd": null}),
            ],
        };
        assert_eq!(
            schema.klaar((2026, 11, 12)).unwrap(),
            vec![((2026, 6, 30), 416.5), ((2026, 11, 12), 10.0)]
        );
        let broken = Schema {
            aantal: 1,
            rows: vec![
                json!({"datum": "2026-06-30T00:00:00", "bedrag": "veel", "datumUitgevoerd": null}),
            ],
        };
        assert!(broken.klaar((2026, 11, 12)).unwrap_err().contains("bedrag"));
    }

    #[test]
    fn booking_lists_per_asset_the_due_lines_and_the_total() {
        let per_activum = vec![
            (
                "MacBook Pro".to_owned(),
                vec![((2026, 6, 30), 416.5), ((2026, 11, 12), 10.0)],
            ),
            ("Bureau".to_owned(), vec![]),
            ("Bus".to_owned(), vec![((2026, 1, 31), 300.0)]),
        ];
        let built = afschrijvingen_boeken_built(3, &per_activum).unwrap();
        assert_eq!(built.title, "3 afschrijvingen boeken");
        assert_eq!(built.body, Value::Null);
        let lines = label_values(&built);
        assert!(lines.contains(&(
            "MacBook Pro".into(),
            "30-06-2026: 416,50; 12-11-2026: 10,00 (samen 426,50)".into()
        )));
        assert!(lines.contains(&("Bus".into(), "31-01-2026: 300,00 (samen 300,00)".into())));
        assert!(!lines.iter().any(|(l, _)| l == "Bureau"));
        assert!(lines.contains(&("Totaal".into(), "3 afschrijvingen, samen 726,50".into())));
        assert!(!lines.iter().any(|(l, _)| l == "Let op"));
        // e-Boekhouden counts otherwise: said, not hidden.
        let built = afschrijvingen_boeken_built(5, &per_activum).unwrap();
        assert!(
            label_values(&built)
                .iter()
                .any(|(l, v)| l == "Let op" && v.contains("telt er 5"))
        );
        assert!(
            afschrijvingen_boeken_built(0, &per_activum)
                .unwrap_err()
                .contains("geen afschrijvingen")
        );
    }

    #[test]
    fn a_long_booking_list_is_cut_and_says_how_much_more() {
        let many: Vec<(String, Vec<(Date, f64)>)> = (0..25)
            .map(|i| {
                (
                    format!("Activum {i}"),
                    (1..=8).map(|m| ((2026, m, 1), 1.0)).collect(),
                )
            })
            .collect();
        let built = afschrijvingen_boeken_built(200, &many).unwrap();
        let lines = label_values(&built);
        let first = &lines.iter().find(|(l, _)| l == "Activum 0").unwrap().1;
        assert!(
            first.contains("en nog 2 regels") && first.contains("samen 8,00"),
            "{first}"
        );
        assert!(!lines.iter().any(|(l, _)| l == "Activum 20"));
        assert!(lines.contains(&(
            "Nog meer vaste activa".into(),
            "5 vaste activa met 40 afschrijvingen, samen 40,00".into()
        )));
        assert!(lines.contains(&("Totaal".into(), "200 afschrijvingen, samen 200,00".into())));
    }
}
