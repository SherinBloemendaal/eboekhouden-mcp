//! Producten en diensten (artikelen), their eenheden and their groepen, as e-Boekhouden's own forms keep them.

use serde_json::Map;

use super::*;
use crate::approvals::Line;
use crate::write::InEx;

// ---------------------------------------------------------------- arguments

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ArtikelenArgs {
    /// Zoekterm, zoals het zoekveld boven de lijst Producten en diensten in e-Boekhouden.
    pub zoek: Option<String>,
    /// Aantal rijen, 1 tot 200 (standaard 50).
    pub limit: Option<u32>,
    /// Vanaf welke rij (standaard 0).
    pub offset: Option<u32>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ArtikelToevoegenArgs {
    /// Code van het product of de dienst, hoogstens 20 tekens.
    pub code: String,
    /// Omschrijving, zoals die op een factuurregel komt.
    pub omschrijving: String,
    /// Verkoopprijs per eenheid, incl. of excl. btw volgens `bedragen`.
    pub prijs: f64,
    /// Is de prijs incl of excl btw (standaard excl). De andere prijs rekent EB MCP uit met het btw-percentage, zoals het formulier van e-Boekhouden.
    pub bedragen: Option<InEx>,
    /// Btw-code voor verkoop, bijvoorbeeld HOOG_VERK_21, LAAG_VERK_9 of GEEN. Verplicht als de administratie btw-plichtig is; anders weglaten.
    pub btw_code: Option<String>,
    /// Code van de tegenrekening (meestal een omzetrekening) in het rekeningschema, bijvoorbeeld "8000".
    pub tegenrekening: String,
    /// Id van de groep (uit artikelgroepen).
    pub groep_id: Option<u64>,
    /// Id van de eenheid (uit eenheden).
    pub eenheid_id: Option<u64>,
    /// Inkoopprijs excl. btw (standaard 0).
    pub inkoopprijs: Option<f64>,
    /// Actief (standaard ja).
    pub actief: Option<bool>,
    /// Opnemen in de automatische voorraadverwerking (standaard nee). Alleen als de administratie voorraad bijhoudt.
    pub voorraad: Option<bool>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// Whether an edit of a product is passed on to the subscriptions that have it on a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Doorvoeren {
    /// Alleen het product wijzigen.
    Nee,
    /// Alle wijzigingen ook doorvoeren in bestaande abonnementen.
    Alles,
    /// Alleen de prijswijziging doorvoeren in bestaande abonnementen.
    AlleenPrijs,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ArtikelWijzigenArgs {
    /// Id van het product (uit artikelen).
    pub id: u64,
    /// Nieuwe code, hoogstens 20 tekens.
    pub code: Option<String>,
    /// Nieuwe omschrijving.
    pub omschrijving: Option<String>,
    /// Nieuwe verkoopprijs, incl. of excl. btw volgens `bedragen` (standaard zoals de prijs nu is ingevoerd).
    pub prijs: Option<f64>,
    /// Is de nieuwe prijs incl of excl btw. Alleen samen met `prijs`.
    pub bedragen: Option<InEx>,
    /// Nieuwe btw-code voor verkoop. De prijs die is ingevoerd blijft staan; de andere wordt opnieuw uitgerekend.
    pub btw_code: Option<String>,
    /// Code van de nieuwe tegenrekening.
    pub tegenrekening: Option<String>,
    /// Id van de nieuwe groep (uit artikelgroepen); 0 haalt de groep weg.
    pub groep_id: Option<u64>,
    /// Id van de nieuwe eenheid (uit eenheden); 0 haalt de eenheid weg.
    pub eenheid_id: Option<u64>,
    /// Nieuwe inkoopprijs excl. btw.
    pub inkoopprijs: Option<f64>,
    /// Actief aan of uit.
    pub actief: Option<bool>,
    /// Opnemen in de automatische voorraadverwerking aan of uit. Alleen als de administratie voorraad bijhoudt.
    pub voorraad: Option<bool>,
    /// De wijziging ook doorvoeren in bestaande abonnementen met dit product: nee (standaard), alles of alleen_prijs. Dit vraagt altijd het akkoord van de gebruiker.
    pub doorvoeren: Option<Doorvoeren>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct EenheidToevoegenArgs {
    /// De eenheid in het enkelvoud, bijvoorbeeld "uur" (hoogstens 50 tekens).
    pub enkelvoud: String,
    /// De eenheid in het meervoud, bijvoorbeeld "uren" (hoogstens 50 tekens).
    pub meervoud: String,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct EenheidWijzigenArgs {
    /// Id van de eenheid (uit eenheden).
    pub id: u64,
    /// Nieuw enkelvoud.
    pub enkelvoud: Option<String>,
    /// Nieuw meervoud.
    pub meervoud: Option<String>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ArtikelgroepToevoegenArgs {
    /// Code van de groep (hoogstens 50 tekens).
    pub code: String,
    /// Omschrijving van de groep (hoogstens 50 tekens).
    pub omschrijving: String,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ArtikelgroepWijzigenArgs {
    /// Id van de groep (uit artikelgroepen).
    pub id: u64,
    /// Nieuwe code.
    pub code: Option<String>,
    /// Nieuwe omschrijving.
    pub omschrijving: Option<String>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

// ---------------------------------------------------------------- what a product is checked against

/// What the product form reads before it saves: the books (btw codes), whether the administration charges
/// btw, the counter-accounts it offers (`grootboekrekening/selectlist/actief/VWBAL`: the active accounts of
/// Winst en Verlies and Balans), its groups (`GET artikelgroep`) and units (`GET eenheid`), and whether it
/// keeps stock (`setting/administratie.use-voorraad`, the form's `showVoorraadAdm`).
pub(super) struct Keuzes {
    pub books: Books,
    pub btwplichtig: bool,
    pub tegenrekeningen: Vec<Value>,
    pub groepen: Vec<Value>,
    pub eenheden: Vec<Value>,
    pub voorraad: bool,
}

impl Keuzes {
    /// A counter-account the product form offers, by its code: its id and its name.
    fn tegenrekening(&self, code: &str) -> Result<(u64, String), String> {
        let code = code.trim();
        self.tegenrekeningen
            .iter()
            .find(|r| text_of(r, "code").trim() == code)
            .and_then(|r| {
                Some((
                    r.get("id")?.as_u64()?,
                    format!("{} {}", code, text_of(r, "omschrijving")),
                ))
            })
            .ok_or_else(|| {
                format!(
                    "Grootboekrekening {code:?} kan geen tegenrekening van een product zijn: e-Boekhouden biedt daar alleen actieve rekeningen van de balans en de winst-en-verliesrekening voor aan"
                )
            })
    }

    fn groep(&self, id: u64) -> Result<String, String> {
        self.groepen
            .iter()
            .find(|g| g.get("id").and_then(Value::as_u64) == Some(id))
            .map(|g| format!("{} - {}", text_of(g, "code"), text_of(g, "omschrijving")))
            .ok_or_else(|| format!("Geen groep met id {id}; artikelgroepen geeft de groepen"))
    }

    fn eenheid(&self, id: u64) -> Result<String, String> {
        self.eenheden
            .iter()
            .find(|e| e.get("id").and_then(Value::as_u64) == Some(id))
            .map(|e| text_of(e, "enkelvoud"))
            .ok_or_else(|| format!("Geen eenheid met id {id}; eenheden geeft de eenheden"))
    }

    /// The counter-account a product has now, also when the form no longer offers it (inactive since).
    fn rekening_label(&self, id: u64) -> String {
        self.tegenrekeningen
            .iter()
            .find(|r| r.get("id").and_then(Value::as_u64) == Some(id))
            .map(|r| format!("{} {}", text_of(r, "code"), text_of(r, "omschrijving")))
            .or_else(|| {
                self.books
                    .rekeningen
                    .iter()
                    .find(|r| r.id == id)
                    .map(|r| format!("{} {}", r.code, r.naam))
            })
            .unwrap_or_else(|| format!("rekening {id}"))
    }

    /// The stock flag a product is saved with: only when the administration keeps stock, as the form does
    /// (`inVoorraad = showVoorraadAdm ? ... : undefined`).
    fn voorraad(&self, wanted: Option<bool>) -> Result<Option<bool>, String> {
        if !self.voorraad {
            if wanted == Some(true) {
                return Err(
                    "Deze administratie houdt geen voorraad bij; zet dat eerst aan in e-Boekhouden, of laat voorraad weg"
                        .into(),
                );
            }
            return Ok(None);
        }
        Ok(Some(wanted.unwrap_or(false)))
    }
}

/// A btw code a product may carry, with the percentage its other price is worked out with.
#[derive(Debug, PartialEq)]
struct Tarief {
    code: String,
    naam: String,
    percentage: f64,
}

impl Tarief {
    fn geen() -> Self {
        Self {
            code: "GEEN".into(),
            naam: "Geen btw".into(),
            percentage: 0.0,
        }
    }
}

/// The btw code of a product. Without btw duty the form shows no btw at all and saves GEEN.
fn tarief(books: &Books, btwplichtig: bool, code: Option<&str>) -> Result<Tarief, String> {
    let code = code.map(str::trim).filter(|c| !c.is_empty());
    if !btwplichtig {
        if let Some(code) = code.filter(|c| !c.eq_ignore_ascii_case("GEEN")) {
            return Err(format!(
                "Deze administratie is niet btw-plichtig: een product krijgt dan geen btw-code (laat {code} weg)"
            ));
        }
        return Ok(Tarief::geen());
    }
    let code = code
        .ok_or("Geef een btw-code voor verkoop, bijvoorbeeld HOOG_VERK_21, LAAG_VERK_9 of GEEN")?;
    let btw = books.btw(code)?;
    if btw.soort == "INK" {
        return Err(format!(
            "Btw-code {} is voor inkoop; een product wil een verkoopcode of GEEN",
            btw.code
        ));
    }
    if btw.code.starts_with("AFW") {
        return Err("Een afwijkend btw-percentage kan EB MCP niet op een product zetten".into());
    }
    Ok(Tarief {
        code: btw.code.clone(),
        naam: btw.naam.clone(),
        percentage: btw.percentage,
    })
}

/// Both prices from the one that was entered, rounded to cents as the product form does (`getPrijsIncl`,
/// `getPrijsExcl`).
fn prijzen(prijs: f64, leading: InEx, percentage: f64) -> (f64, f64) {
    if percentage == 0.0 {
        return (prijs, prijs);
    }
    let factor = 1.0 + percentage / 100.0;
    match leading {
        InEx::Excl => (prijs, write::round_cents(prijs * factor)),
        InEx::Incl => (write::round_cents(prijs / factor), prijs),
    }
}

/// A number of a record read from e-Boekhouden.
fn number_of(record: &Value, key: &str) -> Option<f64> {
    record.get(key).and_then(Value::as_f64)
}

/// A flag that e-Boekhouden gives as true/false or as 1/0.
fn flag_of(record: &Value, key: &str) -> Option<bool> {
    record
        .get(key)
        .and_then(|v| v.as_bool().or_else(|| v.as_u64().map(|n| n == 1)))
}

/// A setting as the product form tests it: `e && e.value.toLocaleLowerCase() === 'true'`.
fn setting_on(setting: &Value) -> bool {
    setting.get("value").is_some_and(|v| {
        v.as_bool() == Some(true) || v.as_str().is_some_and(|s| s.eq_ignore_ascii_case("true"))
    })
}

fn differs(a: f64, b: f64) -> bool {
    (a - b).abs() >= 0.005
}

fn ja_nee(value: bool) -> &'static str {
    if value { "ja" } else { "nee" }
}

// ---------------------------------------------------------------- products

/// A new product, as `POST artikel` from the product form: only the fields the form sends.
pub(super) fn artikel_nieuw(args: &ArtikelToevoegenArgs, k: &Keuzes) -> Result<Built, String> {
    let code = required_text(&args.code, "De code", 20)?;
    let omschrijving = required_text(&args.omschrijving, "De omschrijving", 20_000)?;
    let tarief = tarief(&k.books, k.btwplichtig, args.btw_code.as_deref())?;
    // Without btw both prices are the same; the form then always saves the price as excl.
    let leading = if k.btwplichtig {
        args.bedragen.unwrap_or(InEx::Excl)
    } else {
        InEx::Excl
    };
    let (excl, incl) = prijzen(euros(args.prijs, "De prijs")?, leading, tarief.percentage);
    let (rekening, rekening_naam) = k.tegenrekening(&args.tegenrekening)?;
    let voorraad = k.voorraad(args.voorraad)?;
    let inkoop = args
        .inkoopprijs
        .map(|p| euros(p, "De inkoopprijs"))
        .transpose()?
        .unwrap_or(0.0);
    let groep = args.groep_id.map(|id| k.groep(id)).transpose()?;
    let eenheid = args.eenheid_id.map(|id| k.eenheid(id)).transpose()?;
    let actief = args.actief.unwrap_or(true);

    let mut body = Map::new();
    if let Some(id) = args.groep_id {
        body.insert("groepId".into(), json!(id));
    }
    body.insert("code".into(), json!(code));
    body.insert("omschrijving".into(), json!(omschrijving));
    if let Some(id) = args.eenheid_id {
        body.insert("eenheidId".into(), json!(id));
    }
    body.insert("inkoopPrijsExclusief".into(), json!(inkoop));
    body.insert("prijsExclusief".into(), json!(excl));
    body.insert("prijsInclusief".into(), json!(incl));
    body.insert("prijsLeading".into(), json!(in_ex_code(leading)));
    body.insert("btw".into(), json!(tarief.code));
    body.insert("grootboekrekeningId".into(), json!(rekening));
    body.insert("actief".into(), json!(u8::from(actief)));
    if let Some(in_voorraad) = voorraad {
        body.insert("inVoorraad".into(), json!(u8::from(in_voorraad)));
    }

    let mut lines = vec![
        line("Code", &code),
        line("Omschrijving", shown(&omschrijving)),
    ];
    if k.btwplichtig {
        lines.push(line("Prijs excl. btw", write::euro(excl)));
        lines.push(line("Prijs incl. btw", write::euro(incl)));
        lines.push(line("Btw", &tarief.naam));
    } else {
        lines.push(line("Prijs", write::euro(excl)));
    }
    lines.push(line("Tegenrekening", rekening_naam));
    if let Some(groep) = groep {
        lines.push(line("Groep", groep));
    }
    if let Some(eenheid) = eenheid {
        lines.push(line("Eenheid", eenheid));
    }
    if inkoop != 0.0 {
        lines.push(line("Inkoopprijs excl. btw", write::euro(inkoop)));
    }
    if !actief {
        lines.push(line("Actief", "nee"));
    }
    if voorraad == Some(true) {
        lines.push(line("Automatische voorraadverwerking", "ja"));
    }
    Ok(Built {
        body: Value::Object(body),
        title: format!("Product {code} toevoegen"),
        lines,
    })
}

/// What the approval says about the subscriptions an edit is passed on to. e-Boekhouden's product form sends
/// only the cascade flag (`?cascadeFull=true` / `?cascadePriceOnly=true`); the web app has no call that lists
/// the subscriptions it will touch, so EB MCP does not guess them.
const WELKE_ABONNEMENTEN: &str = "elk abonnement waar dit product op staat; e-Boekhouden kiest ze zelf en EB MCP kan niet vooraf laten zien welke dat zijn";

/// An edit of a product: the request, the query that passes it on to subscriptions, and whether it must
/// always be asked.
#[derive(Debug)]
pub(super) struct ArtikelWijziging {
    pub built: Built,
    pub query: Vec<(String, String)>,
    pub danger: bool,
}

/// A group or unit a product has now: its name, or its id when it is no longer in the list.
fn old_label(id: Option<u64>, look: impl Fn(u64) -> Result<String, String>) -> String {
    match id {
        None => "geen".to_owned(),
        Some(id) => look(id).unwrap_or_else(|_| format!("id {id}")),
    }
}

fn was_wordt(lines: &mut Vec<Line>, what: &str, was: impl ToString, wordt: impl ToString) {
    lines.push(line(&format!("{what} was"), was));
    lines.push(line(&format!("{what} wordt"), wordt));
}

/// An edit of a product, as `PUT artikel` from the product form: the form's fields, taken from the product as
/// it is (`now`, from `GET artikel/{id}`) with the named changes. A kostenplaats and the stock flag the
/// product has go back as they are.
pub(super) fn artikel_gewijzigd(
    args: &ArtikelWijzigenArgs,
    now: &Value,
    kostenplaats: Option<u64>,
    k: &Keuzes,
) -> Result<ArtikelWijziging, String> {
    let old_code = text_of(now, "code");
    if old_code.is_empty() {
        return Err("e-Boekhouden gaf dit product in een onbekende vorm terug".into());
    }
    let old_omschrijving = text_of(now, "omschrijving");
    let mut lines = vec![line(
        "Product",
        format!("{old_code}: {}", shown(&old_omschrijving)),
    )];
    let mut changed = false;

    let mut code = old_code.clone();
    if let Some(new) = &args.code {
        let new = required_text(new, "De code", 20)?;
        if new != code {
            was_wordt(&mut lines, "Code", &code, &new);
            code = new;
            changed = true;
        }
    }
    let mut omschrijving = old_omschrijving.clone();
    if let Some(new) = &args.omschrijving {
        let new = required_text(new, "De omschrijving", 20_000)?;
        if new != omschrijving {
            was_wordt(
                &mut lines,
                "Omschrijving",
                shown(&omschrijving),
                shown(&new),
            );
            omschrijving = new;
            changed = true;
        }
    }

    let old_groep = id_of(now, "groepId");
    let mut groep = old_groep;
    if let Some(new) = args.groep_id.map(|id| (id > 0).then_some(id))
        && new != old_groep
    {
        let wordt = match new {
            Some(id) => k.groep(id)?,
            None => "geen".to_owned(),
        };
        was_wordt(
            &mut lines,
            "Groep",
            old_label(old_groep, |id| k.groep(id)),
            wordt,
        );
        groep = new;
        changed = true;
    }
    let old_eenheid = id_of(now, "eenheidId");
    let mut eenheid = old_eenheid;
    if let Some(new) = args.eenheid_id.map(|id| (id > 0).then_some(id))
        && new != old_eenheid
    {
        let wordt = match new {
            Some(id) => k.eenheid(id)?,
            None => "geen".to_owned(),
        };
        was_wordt(
            &mut lines,
            "Eenheid",
            old_label(old_eenheid, |id| k.eenheid(id)),
            wordt,
        );
        eenheid = new;
        changed = true;
    }

    let old_rekening = id_of(now, "grootboekrekeningId");
    let rekening = match &args.tegenrekening {
        Some(new) => {
            let (id, naam) = k.tegenrekening(new)?;
            if Some(id) != old_rekening {
                was_wordt(
                    &mut lines,
                    "Tegenrekening",
                    old_rekening.map_or_else(|| "geen".to_owned(), |id| k.rekening_label(id)),
                    naam,
                );
                changed = true;
            }
            id
        }
        None => old_rekening
            .ok_or("Dit product heeft geen tegenrekening; geef er een met tegenrekening")?,
    };

    let old_inkoop = number_of(now, "inkoopPrijsExclusief").unwrap_or(0.0);
    let mut inkoop = old_inkoop;
    if let Some(new) = args.inkoopprijs {
        let new = euros(new, "De inkoopprijs")?;
        if differs(new, old_inkoop) {
            was_wordt(
                &mut lines,
                "Inkoopprijs excl. btw",
                write::euro(old_inkoop),
                write::euro(new),
            );
            inkoop = new;
            changed = true;
        }
    }
    let old_actief = flag_of(now, "actief").unwrap_or(false);
    let mut actief = old_actief;
    if let Some(new) = args.actief
        && new != old_actief
    {
        was_wordt(&mut lines, "Actief", ja_nee(old_actief), ja_nee(new));
        actief = new;
        changed = true;
    }
    // The stock flag goes out only when the administration keeps stock; then it is the one read, or the new one.
    let old_voorraad = flag_of(now, "inVoorraad").unwrap_or(false);
    let voorraad = k.voorraad(args.voorraad.or(k.voorraad.then_some(old_voorraad)))?;
    if let Some(new) = voorraad
        && new != old_voorraad
    {
        was_wordt(
            &mut lines,
            "Automatische voorraadverwerking",
            ja_nee(old_voorraad),
            ja_nee(new),
        );
        changed = true;
    }

    // Prices and btw, as the form keeps them: the price that was entered (`prijsLeading`) stays, the other is
    // worked out. Without btw duty the form shows one price and saves it as both, with GEEN.
    if args.bedragen.is_some() && args.prijs.is_none() {
        return Err("bedragen geldt alleen samen met een nieuwe prijs".into());
    }
    let old_btw = Some(text_of(now, "btw"))
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "GEEN".to_owned());
    let old_excl = number_of(now, "prijsExclusief").unwrap_or(0.0);
    let old_incl = if k.btwplichtig {
        number_of(now, "prijsInclusief").unwrap_or(old_excl)
    } else {
        old_excl
    };
    let mut leading = if k.btwplichtig {
        in_ex_from(&text_of(now, "prijsLeading"))
    } else {
        InEx::Excl
    };
    let new_tarief = match &args.btw_code {
        Some(code) => Some(tarief(&k.books, k.btwplichtig, Some(code))?),
        None if !k.btwplichtig => Some(Tarief::geen()),
        None => None,
    };
    let btw_changes = new_tarief
        .as_ref()
        .is_some_and(|t| !t.code.eq_ignore_ascii_case(&old_btw));
    let (mut btw, mut excl, mut incl) = (old_btw.clone(), old_excl, old_incl);
    if args.prijs.is_some() || btw_changes {
        let tarief = match new_tarief {
            Some(tarief) => tarief,
            None => tarief(&k.books, true, Some(&old_btw))?,
        };
        if let Some(new) = args.bedragen
            && k.btwplichtig
        {
            leading = new;
        }
        let base = match args.prijs {
            Some(prijs) => euros(prijs, "De prijs")?,
            None if leading == InEx::Incl => old_incl,
            None => old_excl,
        };
        (excl, incl) = prijzen(base, leading, tarief.percentage);
        if btw_changes {
            let was = k
                .books
                .btw(&old_btw)
                .map(|b| b.naam.clone())
                .unwrap_or_else(|_| old_btw.clone());
            was_wordt(&mut lines, "Btw", was, &tarief.naam);
            changed = true;
        }
        btw = tarief.code;
    }
    if k.btwplichtig {
        if differs(excl, old_excl) {
            was_wordt(
                &mut lines,
                "Prijs excl. btw",
                write::euro(old_excl),
                write::euro(excl),
            );
            changed = true;
        }
        if differs(incl, old_incl) {
            was_wordt(
                &mut lines,
                "Prijs incl. btw",
                write::euro(old_incl),
                write::euro(incl),
            );
            changed = true;
        }
    } else if differs(excl, old_excl) {
        was_wordt(
            &mut lines,
            "Prijs",
            write::euro(old_excl),
            write::euro(excl),
        );
        changed = true;
    }

    let (query, danger) = match args.doorvoeren.unwrap_or(Doorvoeren::Nee) {
        Doorvoeren::Nee => (Vec::new(), false),
        Doorvoeren::Alles => {
            lines.push(line(
                "Bestaande abonnementen",
                "alle wijzigingen van dit product worden ook doorgevoerd in de bestaande abonnementen met dit product",
            ));
            lines.push(line("Welke abonnementen", WELKE_ABONNEMENTEN));
            (vec![("cascadeFull".to_owned(), "true".to_owned())], true)
        }
        Doorvoeren::AlleenPrijs => {
            lines.push(line(
                "Bestaande abonnementen",
                "de prijs van dit product wordt ook doorgevoerd in de bestaande abonnementen met dit product",
            ));
            lines.push(line("Welke abonnementen", WELKE_ABONNEMENTEN));
            (
                vec![("cascadePriceOnly".to_owned(), "true".to_owned())],
                true,
            )
        }
    };
    if !changed && !danger {
        return Err("Er is niets gewijzigd: geef aan wat er anders moet".into());
    }

    let mut body = Map::new();
    body.insert("id".into(), json!(args.id));
    if let Some(id) = groep {
        body.insert("groepId".into(), json!(id));
    }
    body.insert("code".into(), json!(code));
    body.insert("omschrijving".into(), json!(omschrijving));
    if let Some(id) = eenheid {
        body.insert("eenheidId".into(), json!(id));
    }
    body.insert("inkoopPrijsExclusief".into(), json!(inkoop));
    body.insert("prijsExclusief".into(), json!(excl));
    body.insert("prijsInclusief".into(), json!(incl));
    body.insert("prijsLeading".into(), json!(in_ex_code(leading)));
    body.insert("btw".into(), json!(btw));
    body.insert("grootboekrekeningId".into(), json!(rekening));
    body.insert("actief".into(), json!(u8::from(actief)));
    if let Some(in_voorraad) = voorraad {
        body.insert("inVoorraad".into(), json!(u8::from(in_voorraad)));
    }
    if let Some(id) = kostenplaats {
        body.insert("kostenplaatsId".into(), json!(id));
    }
    Ok(ArtikelWijziging {
        built: Built {
            body: Value::Object(body),
            title: format!("Product {old_code} wijzigen"),
            lines,
        },
        query,
        danger,
    })
}

// ---------------------------------------------------------------- units and groups

/// A new unit, as `POST eenheid` from the unit form.
pub(super) fn eenheid_nieuw(args: &EenheidToevoegenArgs) -> Result<Built, String> {
    let enkelvoud = required_text(&args.enkelvoud, "Het enkelvoud", 50)?;
    let meervoud = required_text(&args.meervoud, "Het meervoud", 50)?;
    Ok(Built {
        body: json!({"enkelvoud": enkelvoud, "meervoud": meervoud}),
        title: format!("Eenheid {enkelvoud} toevoegen"),
        lines: vec![line("Enkelvoud", &enkelvoud), line("Meervoud", &meervoud)],
    })
}

/// An edit of a unit, as `PUT eenheid` from the unit form: both names and the id.
pub(super) fn eenheid_gewijzigd(args: &EenheidWijzigenArgs, now: &Value) -> Result<Built, String> {
    let old_enkelvoud = text_of(now, "enkelvoud");
    let old_meervoud = text_of(now, "meervoud");
    let mut lines = vec![line("Eenheid", &old_enkelvoud)];
    let enkelvoud = match &args.enkelvoud {
        Some(new) => required_text(new, "Het enkelvoud", 50)?,
        None => old_enkelvoud.clone(),
    };
    let meervoud = match &args.meervoud {
        Some(new) => required_text(new, "Het meervoud", 50)?,
        None => old_meervoud.clone(),
    };
    if enkelvoud.is_empty() || meervoud.is_empty() {
        return Err("Een eenheid heeft een enkelvoud en een meervoud nodig".into());
    }
    if enkelvoud != old_enkelvoud {
        was_wordt(&mut lines, "Enkelvoud", &old_enkelvoud, &enkelvoud);
    }
    if meervoud != old_meervoud {
        was_wordt(&mut lines, "Meervoud", known(old_meervoud), &meervoud);
    }
    if lines.len() == 1 {
        return Err("Er is niets gewijzigd: geef aan wat er anders moet".into());
    }
    Ok(Built {
        body: json!({"enkelvoud": enkelvoud, "meervoud": meervoud, "id": args.id}),
        title: format!("Eenheid {old_enkelvoud} wijzigen"),
        lines,
    })
}

/// A new group, as `POST artikelgroep` from the group form.
pub(super) fn artikelgroep_nieuw(args: &ArtikelgroepToevoegenArgs) -> Result<Built, String> {
    let code = required_text(&args.code, "De code", 50)?;
    let omschrijving = required_text(&args.omschrijving, "De omschrijving", 50)?;
    Ok(Built {
        body: json!({"code": code, "omschrijving": omschrijving}),
        title: format!("Groep {code} toevoegen"),
        lines: vec![line("Code", &code), line("Omschrijving", &omschrijving)],
    })
}

/// An edit of a group, as `PUT artikelgroep` from the group form: code, omschrijving and the id.
pub(super) fn artikelgroep_gewijzigd(
    args: &ArtikelgroepWijzigenArgs,
    now: &Value,
) -> Result<Built, String> {
    let old_code = text_of(now, "code");
    let old_omschrijving = text_of(now, "omschrijving");
    let mut lines = vec![line("Groep", format!("{old_code} - {old_omschrijving}"))];
    let code = match &args.code {
        Some(new) => required_text(new, "De code", 50)?,
        None => old_code.clone(),
    };
    let omschrijving = match &args.omschrijving {
        Some(new) => required_text(new, "De omschrijving", 50)?,
        None => old_omschrijving.clone(),
    };
    if code.is_empty() || omschrijving.is_empty() {
        return Err("Een groep heeft een code en een omschrijving nodig".into());
    }
    if code != old_code {
        was_wordt(&mut lines, "Code", &old_code, &code);
    }
    if omschrijving != old_omschrijving {
        was_wordt(
            &mut lines,
            "Omschrijving",
            known(old_omschrijving),
            &omschrijving,
        );
    }
    if lines.len() == 1 {
        return Err("Er is niets gewijzigd: geef aan wat er anders moet".into());
    }
    Ok(Built {
        body: json!({"code": code, "omschrijving": omschrijving, "id": args.id}),
        title: format!("Groep {old_code} wijzigen"),
        lines,
    })
}

// ---------------------------------------------------------------- the tools

impl EbMcp {
    /// What a product is checked against before it is saved (`Keuzes`).
    async fn artikelen_keuzes(&self, run: &Run) -> Result<Keuzes, String> {
        // The form reads the setting with `getSetting(key)`, so `acc=false`; no setting at all means off.
        let voorraad = self
            .artikelen_maybe(
                run,
                "setting/administratie.use-voorraad",
                &[("acc".to_owned(), "false".to_owned())],
            )
            .await?
            .is_some_and(|setting| setting_on(&setting));
        Ok(Keuzes {
            books: self.books(run).await?,
            btwplichtig: self.artikelen_btwplichtig(run).await?,
            tegenrekeningen: self
                .artikelen_list(run, "grootboekrekening/selectlist/actief/VWBAL")
                .await?,
            groepen: self.artikelen_list(run, "artikelgroep").await?,
            eenheden: self.artikelen_list(run, "eenheid").await?,
            voorraad,
        })
    }

    /// The kostenplaats of a product: the product form reads it apart (`kostenplaats/GetByArtikelId/{id}`).
    async fn artikelen_kostenplaats(&self, run: &Run, id: u64) -> Result<Option<u64>, String> {
        Ok(self
            .artikelen_maybe(run, &format!("kostenplaats/GetByArtikelId/{id}"), &[])
            .await?
            .and_then(|kostenplaats| id_of(&kostenplaats, "id")))
    }
}

#[tool_router(router = artikelen_producten_router, vis = "pub(super)")]
impl EbMcp {
    #[tool(
        description = "De producten en diensten uit e-Boekhouden (Factureren > Producten en diensten): id, groep, code, omschrijving, tegenrekening en btw-code. Met zoek filter je zoals het zoekveld van die lijst.",
        annotations(title = "Producten en diensten", read_only_hint = true)
    )]
    async fn artikelen(
        &self,
        Parameters(args): Parameters<ArtikelenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const PATH: &str = "artikel/gridtable";
        let run = match self.begin(&context, "artikelen", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let mut query = match grid(args.limit, args.offset) {
            Ok(query) => query,
            Err(message) => return Ok(self.refuse(run, message)),
        };
        push(&mut query, "searchTerm", args.zoek.as_ref());
        let result = self.fetch(&run, PATH, query).await.map(named_rows);
        Ok(self.end(run, PATH, result))
    }

    #[tool(
        description = "Eén product of dienst met al zijn gegevens (prijzen, btw-code, tegenrekening, groep, eenheid), op id (uit artikelen).",
        annotations(title = "Product of dienst", read_only_hint = true)
    )]
    async fn artikel(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let path = format!("artikel/{}", args.id);
        self.simple(
            &context,
            "artikel",
            &args,
            args.account.as_deref(),
            &path,
            Ok(Vec::new()),
        )
        .await
    }

    #[tool(
        description = "De eenheden van producten en diensten (zoals uur/uren of stuk/stuks): id, enkelvoud en meervoud.",
        annotations(title = "Eenheden", read_only_hint = true)
    )]
    async fn eenheden(
        &self,
        Parameters(args): Parameters<ListArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const PATH: &str = "eenheid/gridtable";
        let run = match self.begin(&context, "eenheden", &args, args.account.as_deref()) {
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
        description = "De groepen van producten en diensten: id, code en omschrijving.",
        annotations(title = "Productgroepen", read_only_hint = true)
    )]
    async fn artikelgroepen(
        &self,
        Parameters(args): Parameters<ListArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const PATH: &str = "artikelgroep/gridtable";
        let run = match self.begin(&context, "artikelgroepen", &args, args.account.as_deref()) {
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
        description = "Een product of dienst toevoegen aan Factureren > Producten en diensten. Code, omschrijving, verkoopprijs (incl of excl btw), btw-code voor verkoop en tegenrekening (op code) zijn nodig; groep en eenheid op id (uit artikelgroepen en eenheden). De andere prijs rekent EB MCP uit zoals het formulier van e-Boekhouden. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt het product eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Product toevoegen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn artikel_toevoegen(
        &self,
        Parameters(args): Parameters<ArtikelToevoegenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "artikel_toevoegen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Product toegevoegd; data.id is het id",
            move |this, run, args| async move {
                let keuzes = this.artikelen_keuzes(&run).await?;
                let built = artikel_nieuw(&args, &keuzes)?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change(&run, "POST", "artikel", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een product of dienst wijzigen, op id. Wat je niet noemt blijft zoals het is. Een nieuwe prijs of btw-code rekent de andere prijs opnieuw uit. Met doorvoeren gaat de wijziging (alles, of alleen de prijs) ook naar de bestaande abonnementen met dit product; dat vraagt altijd het akkoord van de gebruiker. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Product wijzigen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn artikel_wijzigen(
        &self,
        Parameters(args): Parameters<ArtikelWijzigenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "artikel_wijzigen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Product gewijzigd",
            move |this, run, args| async move {
                let now = this
                    .fetch(&run, &format!("artikel/{}", args.id), Vec::new())
                    .await?;
                let kostenplaats = this.artikelen_kostenplaats(&run, args.id).await?;
                let keuzes = this.artikelen_keuzes(&run).await?;
                let wijziging = artikel_gewijzigd(&args, &now, kostenplaats, &keuzes)?;
                this.approve(&run, TOOL, &wijziging.built, wijziging.danger)
                    .await?;
                this.change_query_done(
                    &run,
                    "PUT",
                    "artikel",
                    &wijziging.query,
                    Some(&wijziging.built.body),
                )
                .await
            },
        )
        .await
    }

    #[tool(
        description = "Een product of dienst verwijderen, op id. Eventueel ingevoerde voorraad van het product verdwijnt mee. Dit is niet terug te draaien. De gebruiker moet dit altijd goedkeuren in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Product verwijderen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn artikel_verwijderen(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "artikel_verwijderen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Product verwijderd",
            move |this, run, args| async move {
                let now = this
                    .fetch(&run, &format!("artikel/{}", args.id), Vec::new())
                    .await?;
                let code = text_of(&now, "code");
                let built = Built {
                    body: Value::Null,
                    title: format!("Product {code} verwijderen"),
                    lines: vec![
                        line("Product", &code),
                        line("Omschrijving", shown(&text_of(&now, "omschrijving"))),
                        line(
                            "Prijs excl. btw",
                            write::euro(number_of(&now, "prijsExclusief").unwrap_or(0.0)),
                        ),
                        line(
                            "Gevolg",
                            "het product verdwijnt uit Producten en diensten, met eventueel ingevoerde voorraad; dit is niet terug te draaien",
                        ),
                    ],
                };
                this.approve(&run, TOOL, &built, true).await?;
                this.change_done(&run, "DELETE", &format!("artikel/{}", args.id), None)
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een eenheid voor producten en diensten toevoegen, met enkelvoud en meervoud (bijvoorbeeld uur en uren). Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de eenheid eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Eenheid toevoegen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn eenheid_toevoegen(
        &self,
        Parameters(args): Parameters<EenheidToevoegenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "eenheid_toevoegen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Eenheid toegevoegd; data.id is het id",
            move |this, run, args| async move {
                let built = eenheid_nieuw(&args)?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change(&run, "POST", "eenheid", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een eenheid wijzigen, op id (uit eenheden). Wat je niet noemt blijft zoals het is. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de wijziging eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Eenheid wijzigen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn eenheid_wijzigen(
        &self,
        Parameters(args): Parameters<EenheidWijzigenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "eenheid_wijzigen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Eenheid gewijzigd",
            move |this, run, args| async move {
                let now = this
                    .fetch(&run, &format!("eenheid/{}", args.id), Vec::new())
                    .await?;
                let built = eenheid_gewijzigd(&args, &now)?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change_done(&run, "PUT", "eenheid", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een eenheid verwijderen, op id (uit eenheden). Dit is niet terug te draaien. De gebruiker moet dit altijd goedkeuren in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Eenheid verwijderen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn eenheid_verwijderen(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "eenheid_verwijderen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Eenheid verwijderd",
            move |this, run, args| async move {
                let now = this
                    .fetch(&run, &format!("eenheid/{}", args.id), Vec::new())
                    .await?;
                let enkelvoud = text_of(&now, "enkelvoud");
                let built = Built {
                    body: Value::Null,
                    title: format!("Eenheid {enkelvoud} verwijderen"),
                    lines: vec![
                        line(
                            "Eenheid",
                            format!("{enkelvoud} / {}", text_of(&now, "meervoud")),
                        ),
                        line(
                            "Gevolg",
                            "de eenheid verdwijnt uit de lijst; dit is niet terug te draaien",
                        ),
                    ],
                };
                this.approve(&run, TOOL, &built, true).await?;
                this.change_done(&run, "DELETE", &format!("eenheid/{}", args.id), None)
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een groep voor producten en diensten toevoegen, met code en omschrijving. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de groep eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Productgroep toevoegen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn artikelgroep_toevoegen(
        &self,
        Parameters(args): Parameters<ArtikelgroepToevoegenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "artikelgroep_toevoegen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Groep toegevoegd; data.id is het id",
            move |this, run, args| async move {
                let built = artikelgroep_nieuw(&args)?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change(&run, "POST", "artikelgroep", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een groep van producten en diensten wijzigen, op id (uit artikelgroepen). Wat je niet noemt blijft zoals het is. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de wijziging eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Productgroep wijzigen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn artikelgroep_wijzigen(
        &self,
        Parameters(args): Parameters<ArtikelgroepWijzigenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "artikelgroep_wijzigen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Groep gewijzigd",
            move |this, run, args| async move {
                let now = this
                    .fetch(&run, &format!("artikelgroep/{}", args.id), Vec::new())
                    .await?;
                let built = artikelgroep_gewijzigd(&args, &now)?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change_done(&run, "PUT", "artikelgroep", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een groep van producten en diensten verwijderen, op id (uit artikelgroepen). Dit is niet terug te draaien. De gebruiker moet dit altijd goedkeuren in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Productgroep verwijderen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn artikelgroep_verwijderen(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "artikelgroep_verwijderen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Groep verwijderd",
            move |this, run, args| async move {
                let now = this
                    .fetch(&run, &format!("artikelgroep/{}", args.id), Vec::new())
                    .await?;
                let code = text_of(&now, "code");
                let built = Built {
                    body: Value::Null,
                    title: format!("Groep {code} verwijderen"),
                    lines: vec![
                        line(
                            "Groep",
                            format!("{code} - {}", text_of(&now, "omschrijving")),
                        ),
                        line(
                            "Gevolg",
                            "de groep verdwijnt uit de lijst; dit is niet terug te draaien",
                        ),
                    ],
                };
                this.approve(&run, TOOL, &built, true).await?;
                this.change_done(&run, "DELETE", &format!("artikelgroep/{}", args.id), None)
                    .await
            },
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn books() -> Books {
        Books::from_api(
            &json!({
                "colMetadata": [{"name": "id"}, {"name": "code"}, {"name": "omschrijving"}, {"name": "categorie"}],
                "data": [[80, "8000", "Omzet", "Winst en Verlies"], [81, "8100", "Omzet advies", "Winst en Verlies"], [13, "1300", "Debiteuren", "Debiteuren"]]
            }),
            &json!([
                {"code": "GEEN", "omschrijving": "Geen btw", "percentage": 0, "soort": ""},
                {"code": "HOOG_VERK_21", "omschrijving": "Hoog 21%", "percentage": 21, "soort": "VERK"},
                {"code": "LAAG_VERK_9", "omschrijving": "Laag 9%", "percentage": 9, "soort": "VERK"},
                {"code": "HOOG_INK_21", "omschrijving": "Hoog inkoop", "percentage": 21, "soort": "INK"},
                {"code": "AFW_VERK", "omschrijving": "Afwijkend", "percentage": 0, "soort": "VERK"}
            ]),
        )
        .unwrap()
    }

    fn keuzes(btwplichtig: bool) -> Keuzes {
        Keuzes {
            books: books(),
            btwplichtig,
            // The selectlist leaves 1300 Debiteuren out, as the form offers only active VW and BAL accounts.
            tegenrekeningen: vec![
                json!({"id": 80, "code": "8000", "omschrijving": "Omzet"}),
                json!({"id": 81, "code": "8100", "omschrijving": "Omzet advies"}),
            ],
            groepen: vec![json!({"id": 4, "code": "DIENST", "omschrijving": "Diensten"})],
            eenheden: vec![
                json!({"id": 6, "enkelvoud": "uur", "meervoud": "uren"}),
                json!({"id": 7, "enkelvoud": "stuk", "meervoud": "stuks"}),
            ],
            voorraad: false,
        }
    }

    fn nieuw() -> ArtikelToevoegenArgs {
        ArtikelToevoegenArgs {
            code: " UUR ".into(),
            omschrijving: "Advies per uur".into(),
            prijs: 100.0,
            bedragen: None,
            btw_code: Some("HOOG_VERK_21".into()),
            tegenrekening: "8000".into(),
            groep_id: Some(4),
            eenheid_id: Some(6),
            inkoopprijs: None,
            actief: None,
            voorraad: None,
            account: None,
        }
    }

    fn label<'a>(built: &'a Built, label: &str) -> Option<&'a str> {
        built
            .lines
            .iter()
            .find(|l| l.label == label)
            .map(|l| l.value.as_str())
    }

    #[test]
    fn makes_a_product_as_e_boekhoudens_form_does() {
        let built = artikel_nieuw(&nieuw(), &keuzes(true)).unwrap();
        assert_eq!(
            built.body,
            json!({"groepId": 4, "code": "UUR", "omschrijving": "Advies per uur", "eenheidId": 6, "inkoopPrijsExclusief": 0.0,
                "prijsExclusief": 100.0, "prijsInclusief": 121.0, "prijsLeading": "EX", "btw": "HOOG_VERK_21",
                "grootboekrekeningId": 80, "actief": 1})
        );
        assert_eq!(built.title, "Product UUR toevoegen");
        assert_eq!(label(&built, "Prijs incl. btw"), Some("121,00"));
        assert_eq!(label(&built, "Btw"), Some("Hoog 21%"));
        assert_eq!(label(&built, "Tegenrekening"), Some("8000 Omzet"));
        assert_eq!(label(&built, "Groep"), Some("DIENST - Diensten"));
        assert_eq!(label(&built, "Eenheid"), Some("uur"));
        assert_eq!(label(&built, "Actief"), None);

        // A price incl btw: the excl price is worked out and rounded to cents, and "IN" says which was entered.
        let args = ArtikelToevoegenArgs {
            prijs: 10.0,
            bedragen: Some(InEx::Incl),
            btw_code: Some("LAAG_VERK_9".into()),
            groep_id: None,
            eenheid_id: None,
            inkoopprijs: Some(4.5),
            actief: Some(false),
            ..nieuw()
        };
        let built = artikel_nieuw(&args, &keuzes(true)).unwrap();
        assert_eq!(built.body["prijsExclusief"], json!(9.17));
        assert_eq!(built.body["prijsInclusief"], json!(10.0));
        assert_eq!(built.body["prijsLeading"], "IN");
        assert_eq!(built.body["inkoopPrijsExclusief"], json!(4.5));
        assert_eq!(built.body["actief"], 0);
        assert!(built.body.get("groepId").is_none() && built.body.get("eenheidId").is_none());
        assert_eq!(label(&built, "Actief"), Some("nee"));
        assert_eq!(label(&built, "Inkoopprijs excl. btw"), Some("4,50"));
    }

    #[test]
    fn a_product_without_btw_duty_has_one_price_and_geen() {
        let args = ArtikelToevoegenArgs {
            btw_code: None,
            bedragen: Some(InEx::Incl),
            ..nieuw()
        };
        let built = artikel_nieuw(&args, &keuzes(false)).unwrap();
        assert_eq!(built.body["btw"], "GEEN");
        assert_eq!(built.body["prijsExclusief"], json!(100.0));
        assert_eq!(built.body["prijsInclusief"], json!(100.0));
        assert_eq!(built.body["prijsLeading"], "EX");
        assert_eq!(label(&built, "Prijs"), Some("100,00"));
        assert_eq!(label(&built, "Btw"), None);
        let err = artikel_nieuw(&nieuw(), &keuzes(false)).unwrap_err();
        assert!(err.contains("niet btw-plichtig"), "{err}");
    }

    #[test]
    fn refuses_a_product_e_boekhouden_would_refuse() {
        let k = keuzes(true);
        let refused = |args: ArtikelToevoegenArgs| artikel_nieuw(&args, &k).unwrap_err();
        assert!(
            refused(ArtikelToevoegenArgs {
                code: "  ".into(),
                ..nieuw()
            })
            .contains("De code is leeg")
        );
        assert!(
            refused(ArtikelToevoegenArgs {
                code: "X".repeat(21),
                ..nieuw()
            })
            .contains("hoogstens 20")
        );
        assert!(
            refused(ArtikelToevoegenArgs {
                omschrijving: String::new(),
                ..nieuw()
            })
            .contains("omschrijving")
        );
        assert!(
            refused(ArtikelToevoegenArgs {
                btw_code: None,
                ..nieuw()
            })
            .contains("Geef een btw-code")
        );
        assert!(
            refused(ArtikelToevoegenArgs {
                btw_code: Some("HOOG_INK_21".into()),
                ..nieuw()
            })
            .contains("inkoop")
        );
        assert!(
            refused(ArtikelToevoegenArgs {
                btw_code: Some("AFW_VERK".into()),
                ..nieuw()
            })
            .contains("afwijkend")
        );
        assert!(
            refused(ArtikelToevoegenArgs {
                btw_code: Some("NUL".into()),
                ..nieuw()
            })
            .contains("Onbekende btw-code")
        );
        assert!(
            refused(ArtikelToevoegenArgs {
                tegenrekening: "9999".into(),
                ..nieuw()
            })
            .contains("9999")
        );
        assert!(
            refused(ArtikelToevoegenArgs {
                groep_id: Some(5),
                ..nieuw()
            })
            .contains("Geen groep met id 5")
        );
        assert!(
            refused(ArtikelToevoegenArgs {
                eenheid_id: Some(8),
                ..nieuw()
            })
            .contains("Geen eenheid met id 8")
        );
        assert!(
            refused(ArtikelToevoegenArgs {
                prijs: f64::NAN,
                ..nieuw()
            })
            .contains("geen geldig bedrag")
        );
        assert!(
            refused(ArtikelToevoegenArgs {
                inkoopprijs: Some(f64::INFINITY),
                ..nieuw()
            })
            .contains("inkoopprijs")
        );
    }

    fn product() -> Value {
        json!({"id": 31, "code": "UUR", "omschrijving": "Advies per uur", "groepId": 4, "eenheidId": 6,
            "inkoopPrijsExclusief": 0, "prijsExclusief": 100.0, "prijsInclusief": 121.0, "prijsLeading": "EX",
            "btw": "HOOG_VERK_21", "grootboekrekeningId": 80, "actief": 1, "inVoorraad": true, "datum": "2026-01-02T00:00:00"})
    }

    fn wijzigen() -> ArtikelWijzigenArgs {
        ArtikelWijzigenArgs {
            id: 31,
            code: None,
            omschrijving: None,
            prijs: None,
            bedragen: None,
            btw_code: None,
            tegenrekening: None,
            groep_id: None,
            eenheid_id: None,
            inkoopprijs: None,
            actief: None,
            voorraad: None,
            doorvoeren: None,
            account: None,
        }
    }

    #[test]
    fn a_product_edit_sends_the_form_back_with_only_the_named_change() {
        let k = keuzes(true);
        let args = ArtikelWijzigenArgs {
            omschrijving: Some("Advies per uur, op locatie".into()),
            ..wijzigen()
        };
        let edit = artikel_gewijzigd(&args, &product(), Some(12), &k).unwrap();
        assert_eq!(
            edit.built.body,
            json!({"id": 31, "groepId": 4, "code": "UUR", "omschrijving": "Advies per uur, op locatie", "eenheidId": 6,
                "inkoopPrijsExclusief": 0.0, "prijsExclusief": 100.0, "prijsInclusief": 121.0, "prijsLeading": "EX",
                "btw": "HOOG_VERK_21", "grootboekrekeningId": 80, "actief": 1, "kostenplaatsId": 12})
        );
        assert!(edit.query.is_empty() && !edit.danger);
        assert_eq!(edit.built.title, "Product UUR wijzigen");
        assert_eq!(
            label(&edit.built, "Omschrijving was"),
            Some("Advies per uur")
        );
        assert_eq!(
            label(&edit.built, "Omschrijving wordt"),
            Some("Advies per uur, op locatie")
        );
        assert_eq!(label(&edit.built, "Prijs excl. btw wordt"), None);

        // Group and unit taken off with 0: the keys stay out, as the form leaves an empty choice out.
        let args = ArtikelWijzigenArgs {
            groep_id: Some(0),
            eenheid_id: Some(7),
            tegenrekening: Some("8100".into()),
            actief: Some(false),
            ..wijzigen()
        };
        let edit = artikel_gewijzigd(&args, &product(), None, &k).unwrap();
        assert!(edit.built.body.get("groepId").is_none());
        assert!(edit.built.body.get("kostenplaatsId").is_none());
        assert_eq!(edit.built.body["eenheidId"], 7);
        assert_eq!(edit.built.body["grootboekrekeningId"], 81);
        assert_eq!(edit.built.body["actief"], 0);
        assert_eq!(label(&edit.built, "Groep was"), Some("DIENST - Diensten"));
        assert_eq!(label(&edit.built, "Groep wordt"), Some("geen"));
        assert_eq!(label(&edit.built, "Eenheid wordt"), Some("stuk"));
        assert_eq!(label(&edit.built, "Tegenrekening was"), Some("8000 Omzet"));
        assert_eq!(label(&edit.built, "Actief wordt"), Some("nee"));
    }

    #[test]
    fn a_new_price_or_btw_code_works_out_the_other_price() {
        let k = keuzes(true);
        // A new price keeps the way it was entered (excl), the incl price follows.
        let args = ArtikelWijzigenArgs {
            prijs: Some(110.0),
            ..wijzigen()
        };
        let edit = artikel_gewijzigd(&args, &product(), None, &k).unwrap();
        assert_eq!(edit.built.body["prijsExclusief"], json!(110.0));
        assert_eq!(edit.built.body["prijsInclusief"], json!(133.1));
        assert_eq!(label(&edit.built, "Prijs excl. btw was"), Some("100,00"));
        assert_eq!(label(&edit.built, "Prijs incl. btw wordt"), Some("133,10"));

        // A new btw code on a price entered incl: the incl price stays, the excl price is worked out again.
        let mut incl = product();
        incl["prijsLeading"] = json!("IN");
        let args = ArtikelWijzigenArgs {
            btw_code: Some("LAAG_VERK_9".into()),
            ..wijzigen()
        };
        let edit = artikel_gewijzigd(&args, &incl, None, &k).unwrap();
        assert_eq!(edit.built.body["btw"], "LAAG_VERK_9");
        assert_eq!(edit.built.body["prijsInclusief"], json!(121.0));
        assert_eq!(edit.built.body["prijsExclusief"], json!(111.01));
        assert_eq!(edit.built.body["prijsLeading"], "IN");
        assert_eq!(label(&edit.built, "Btw was"), Some("Hoog 21%"));
        assert_eq!(label(&edit.built, "Btw wordt"), Some("Laag 9%"));

        // A new price entered incl turns the leading price around.
        let args = ArtikelWijzigenArgs {
            prijs: Some(242.0),
            bedragen: Some(InEx::Incl),
            ..wijzigen()
        };
        let edit = artikel_gewijzigd(&args, &product(), None, &k).unwrap();
        assert_eq!(edit.built.body["prijsExclusief"], json!(200.0));
        assert_eq!(edit.built.body["prijsLeading"], "IN");

        // bedragen alone says nothing to work out.
        let args = ArtikelWijzigenArgs {
            bedragen: Some(InEx::Incl),
            ..wijzigen()
        };
        assert!(
            artikel_gewijzigd(&args, &product(), None, &k)
                .unwrap_err()
                .contains("alleen samen met een nieuwe prijs")
        );
    }

    #[test]
    fn passing_an_edit_on_to_subscriptions_always_asks() {
        let k = keuzes(true);
        let args = ArtikelWijzigenArgs {
            prijs: Some(120.0),
            doorvoeren: Some(Doorvoeren::AlleenPrijs),
            ..wijzigen()
        };
        let edit = artikel_gewijzigd(&args, &product(), None, &k).unwrap();
        assert!(edit.danger);
        assert_eq!(
            edit.query,
            vec![("cascadePriceOnly".to_owned(), "true".to_owned())]
        );
        assert!(
            label(&edit.built, "Bestaande abonnementen")
                .unwrap()
                .contains("prijs")
        );
        // Passing everything on is a change by itself, even when the product stays as it is.
        let args = ArtikelWijzigenArgs {
            doorvoeren: Some(Doorvoeren::Alles),
            ..wijzigen()
        };
        let edit = artikel_gewijzigd(&args, &product(), None, &k).unwrap();
        assert_eq!(
            edit.query,
            vec![("cascadeFull".to_owned(), "true".to_owned())]
        );
        assert!(edit.danger);
    }

    #[test]
    fn refuses_an_edit_that_changes_nothing_or_is_wrong() {
        let k = keuzes(true);
        let err =
            |args: ArtikelWijzigenArgs| artikel_gewijzigd(&args, &product(), None, &k).unwrap_err();
        assert!(err(wijzigen()).contains("niets gewijzigd"));
        // The same values as now are no change either.
        assert!(
            err(ArtikelWijzigenArgs {
                code: Some("UUR".into()),
                prijs: Some(100.0),
                groep_id: Some(4),
                doorvoeren: Some(Doorvoeren::Nee),
                ..wijzigen()
            })
            .contains("niets gewijzigd")
        );
        assert!(
            err(ArtikelWijzigenArgs {
                code: Some(String::new()),
                ..wijzigen()
            })
            .contains("leeg")
        );
        assert!(
            err(ArtikelWijzigenArgs {
                groep_id: Some(9),
                ..wijzigen()
            })
            .contains("Geen groep")
        );
        assert!(
            err(ArtikelWijzigenArgs {
                btw_code: Some("HOOG_INK_21".into()),
                ..wijzigen()
            })
            .contains("inkoop")
        );
        assert!(
            artikel_gewijzigd(&wijzigen(), &json!({"id": 31}), None, &k)
                .unwrap_err()
                .contains("onbekende vorm")
        );
        let mut zonder = product();
        zonder["grootboekrekeningId"] = Value::Null;
        assert!(
            artikel_gewijzigd(
                &ArtikelWijzigenArgs {
                    code: Some("U2".into()),
                    ..wijzigen()
                },
                &zonder,
                None,
                &k
            )
            .unwrap_err()
            .contains("geen tegenrekening")
        );
    }

    #[test]
    fn without_btw_duty_an_edit_saves_one_price_and_geen() {
        let k = keuzes(false);
        let args = ArtikelWijzigenArgs {
            prijs: Some(90.0),
            ..wijzigen()
        };
        let edit = artikel_gewijzigd(&args, &product(), None, &k).unwrap();
        assert_eq!(edit.built.body["btw"], "GEEN");
        assert_eq!(edit.built.body["prijsExclusief"], json!(90.0));
        assert_eq!(edit.built.body["prijsInclusief"], json!(90.0));
        assert_eq!(edit.built.body["prijsLeading"], "EX");
        assert_eq!(label(&edit.built, "Prijs wordt"), Some("90,00"));
        assert_eq!(label(&edit.built, "Btw wordt"), Some("Geen btw"));
    }

    #[test]
    fn units_and_groups_send_what_their_forms_send() {
        let built = eenheid_nieuw(&EenheidToevoegenArgs {
            enkelvoud: " dag ".into(),
            meervoud: "dagen".into(),
            account: None,
        })
        .unwrap();
        assert_eq!(built.body, json!({"enkelvoud": "dag", "meervoud": "dagen"}));
        assert_eq!(built.title, "Eenheid dag toevoegen");
        assert!(
            eenheid_nieuw(&EenheidToevoegenArgs {
                enkelvoud: "dag".into(),
                meervoud: " ".into(),
                account: None
            })
            .unwrap_err()
            .contains("meervoud is leeg")
        );
        assert!(
            eenheid_nieuw(&EenheidToevoegenArgs {
                enkelvoud: "d".repeat(51),
                meervoud: "dagen".into(),
                account: None
            })
            .unwrap_err()
            .contains("hoogstens 50")
        );

        let now = json!({"id": 6, "enkelvoud": "uur", "meervoud": "uren"});
        let edit = eenheid_gewijzigd(
            &EenheidWijzigenArgs {
                id: 6,
                enkelvoud: None,
                meervoud: Some("uurtjes".into()),
                account: None,
            },
            &now,
        )
        .unwrap();
        assert_eq!(
            edit.body,
            json!({"enkelvoud": "uur", "meervoud": "uurtjes", "id": 6})
        );
        assert_eq!(label(&edit, "Meervoud was"), Some("uren"));
        assert!(
            eenheid_gewijzigd(
                &EenheidWijzigenArgs {
                    id: 6,
                    enkelvoud: Some("uur".into()),
                    meervoud: None,
                    account: None
                },
                &now
            )
            .unwrap_err()
            .contains("niets gewijzigd")
        );

        let built = artikelgroep_nieuw(&ArtikelgroepToevoegenArgs {
            code: "PROD".into(),
            omschrijving: "Producten".into(),
            account: None,
        })
        .unwrap();
        assert_eq!(
            built.body,
            json!({"code": "PROD", "omschrijving": "Producten"})
        );
        assert_eq!(built.title, "Groep PROD toevoegen");
        assert!(
            artikelgroep_nieuw(&ArtikelgroepToevoegenArgs {
                code: String::new(),
                omschrijving: "Producten".into(),
                account: None
            })
            .unwrap_err()
            .contains("code is leeg")
        );
        let now = json!({"id": 4, "code": "DIENST", "omschrijving": "Diensten"});
        let edit = artikelgroep_gewijzigd(
            &ArtikelgroepWijzigenArgs {
                id: 4,
                code: Some("ADVIES".into()),
                omschrijving: None,
                account: None,
            },
            &now,
        )
        .unwrap();
        assert_eq!(
            edit.body,
            json!({"code": "ADVIES", "omschrijving": "Diensten", "id": 4})
        );
        assert_eq!(edit.title, "Groep DIENST wijzigen");
        assert_eq!(label(&edit, "Code wordt"), Some("ADVIES"));
        assert!(
            artikelgroep_gewijzigd(
                &ArtikelgroepWijzigenArgs {
                    id: 4,
                    code: None,
                    omschrijving: Some("o".repeat(51)),
                    account: None
                },
                &now
            )
            .unwrap_err()
            .contains("hoogstens 50")
        );
    }
    #[test]
    fn rounds_cents_as_e_boekhoudens_form_does() {
        // 3,50 x 1,21 is 4,2349999... in floating point; the form's round(n, 2) makes it 4,24, not 4,23.
        assert_eq!(prijzen(3.5, InEx::Excl, 21.0), (3.5, 4.24));
        // And back: 4,24 incl. 21% is 3,50 excl.
        assert_eq!(prijzen(4.24, InEx::Incl, 21.0), (3.5, 4.24));
        let args = ArtikelToevoegenArgs {
            prijs: 3.5,
            ..nieuw()
        };
        let built = artikel_nieuw(&args, &keuzes(true)).unwrap();
        assert_eq!(built.body["prijsInclusief"], json!(4.24));
        assert_eq!(label(&built, "Prijs incl. btw"), Some("4,24"));
        // A price typed with more decimals is rounded the same way.
        assert_eq!(euros(1.005, "De prijs").unwrap(), 1.01);
        let args = ArtikelWijzigenArgs {
            prijs: Some(4.24),
            bedragen: Some(InEx::Incl),
            ..wijzigen()
        };
        let edit = artikel_gewijzigd(&args, &product(), None, &keuzes(true)).unwrap();
        assert_eq!(edit.built.body["prijsExclusief"], json!(3.5));
    }

    #[test]
    fn the_counter_account_is_one_the_form_offers() {
        let k = keuzes(true);
        let err = artikel_nieuw(
            &ArtikelToevoegenArgs {
                tegenrekening: "1300".into(),
                ..nieuw()
            },
            &k,
        )
        .unwrap_err();
        assert!(
            err.contains("\"1300\"") && err.contains("actieve rekeningen"),
            "{err}"
        );
        let err = artikel_gewijzigd(
            &ArtikelWijzigenArgs {
                tegenrekening: Some("1300".into()),
                ..wijzigen()
            },
            &product(),
            None,
            &k,
        )
        .unwrap_err();
        assert!(err.contains("actieve rekeningen"), "{err}");
        // A product whose account is no longer offered keeps it when the edit leaves it alone.
        let mut oud = product();
        oud["grootboekrekeningId"] = json!(13);
        let edit = artikel_gewijzigd(
            &ArtikelWijzigenArgs {
                code: Some("UUR2".into()),
                ..wijzigen()
            },
            &oud,
            None,
            &k,
        )
        .unwrap();
        assert_eq!(edit.built.body["grootboekrekeningId"], 13);
        let edit = artikel_gewijzigd(
            &ArtikelWijzigenArgs {
                tegenrekening: Some("8100".into()),
                ..wijzigen()
            },
            &oud,
            None,
            &k,
        )
        .unwrap();
        assert_eq!(
            label(&edit.built, "Tegenrekening was"),
            Some("1300 Debiteuren")
        );
        assert_eq!(
            label(&edit.built, "Tegenrekening wordt"),
            Some("8100 Omzet advies")
        );
    }

    #[test]
    fn the_stock_flag_goes_out_only_when_the_administration_keeps_stock() {
        let zonder = keuzes(true);
        let met = Keuzes {
            voorraad: true,
            ..keuzes(true)
        };
        // A new product: left out without stock, 0 or 1 with it, and refused when asked for without it.
        let built = artikel_nieuw(&nieuw(), &zonder).unwrap();
        assert!(built.body.get("inVoorraad").is_none());
        let built = artikel_nieuw(&nieuw(), &met).unwrap();
        assert_eq!(built.body["inVoorraad"], 0);
        let args = ArtikelToevoegenArgs {
            voorraad: Some(true),
            ..nieuw()
        };
        let built = artikel_nieuw(&args, &met).unwrap();
        assert_eq!(built.body["inVoorraad"], 1);
        assert_eq!(label(&built, "Automatische voorraadverwerking"), Some("ja"));
        assert!(
            artikel_nieuw(&args, &zonder)
                .unwrap_err()
                .contains("houdt geen voorraad bij")
        );
        // An edit: the flag read goes back with stock, and stays out without it.
        let rename = ArtikelWijzigenArgs {
            code: Some("UUR2".into()),
            ..wijzigen()
        };
        let edit = artikel_gewijzigd(&rename, &product(), None, &met).unwrap();
        assert_eq!(edit.built.body["inVoorraad"], 1);
        let edit = artikel_gewijzigd(&rename, &product(), None, &zonder).unwrap();
        assert!(edit.built.body.get("inVoorraad").is_none());
        let edit = artikel_gewijzigd(
            &ArtikelWijzigenArgs {
                voorraad: Some(false),
                ..wijzigen()
            },
            &product(),
            None,
            &met,
        )
        .unwrap();
        assert_eq!(edit.built.body["inVoorraad"], 0);
        assert_eq!(
            label(&edit.built, "Automatische voorraadverwerking wordt"),
            Some("nee")
        );
    }

    #[test]
    fn passing_an_edit_on_says_which_subscriptions_cannot_be_named() {
        for doorvoeren in [Doorvoeren::Alles, Doorvoeren::AlleenPrijs] {
            let args = ArtikelWijzigenArgs {
                doorvoeren: Some(doorvoeren),
                ..wijzigen()
            };
            let edit = artikel_gewijzigd(&args, &product(), None, &keuzes(true)).unwrap();
            let welke = label(&edit.built, "Welke abonnementen").unwrap();
            assert!(
                welke.contains("elk abonnement") && welke.contains("niet vooraf"),
                "{welke}"
            );
        }
        let edit = artikel_gewijzigd(
            &ArtikelWijzigenArgs {
                code: Some("UUR2".into()),
                ..wijzigen()
            },
            &product(),
            None,
            &keuzes(true),
        )
        .unwrap();
        assert_eq!(label(&edit.built, "Welke abonnementen"), None);
    }

    #[test]
    fn reads_the_stock_setting_as_the_form_does() {
        assert!(setting_on(&json!({"value": "True"})));
        assert!(setting_on(&json!({"value": true})));
        assert!(!setting_on(&json!({"value": "false"})));
        assert!(!setting_on(&json!({"key": "administratie.use-voorraad"})));
    }
}
