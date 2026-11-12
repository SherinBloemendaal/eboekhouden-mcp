//! The arguments of this part's tools and what they send, built together with the words the user approves them
//! by. Nothing here talks to e-Boekhouden: every function turns arguments plus what was read into a request
//! body (or query) and a summary, so each can be tested on its own. verkoop.rs reads, asks and sends.
//!
//! Where each shape comes from (e-Boekhouden's own web app, the public bundle of 2026-11-12) is written at the
//! function; ronde-verkoop.md in the notes folder has the search strings.

use super::*;
use crate::approvals::Line;
use serde_json::Map;

pub(super) type Date = (i32, u32, u32);
/// Query parameters of a request.
pub(super) type Query = Vec<(String, String)>;

/// Most invoices one bulk tool handles in one go: every one is a request of its own, and the approval lists each.
pub(super) const MAX_BULK: usize = 50;
/// The offerte form's own limit on lines to invoice at once.
const MAX_FACTUUR_REGELS: usize = 100;

// ---------------------------------------------------------------- small helpers

/// A text field of an API object, trimmed; empty when it is missing or not text.
pub(super) fn s(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_owned()
}

/// A text field that is filled in.
pub(super) fn filled(value: &Value, key: &str) -> Option<String> {
    Some(s(value, key)).filter(|v| !v.is_empty())
}

/// A number field as an id, also when the API sends it as text.
pub(super) fn id_of(value: &Value, key: &str) -> Option<u64> {
    let field = value.get(key)?;
    field
        .as_u64()
        .or_else(|| field.as_str().and_then(|t| t.trim().parse().ok()))
}

pub(super) fn amount_of(value: &Value, key: &str) -> Option<f64> {
    let field = value.get(key)?;
    field
        .as_f64()
        .or_else(|| field.as_str().and_then(|t| t.trim().parse().ok()))
}

/// The rows of a list: a plain array, or a grid answer (`data`, named by `named_rows`).
pub(super) fn rows(value: &Value) -> Vec<Value> {
    match value {
        Value::Array(rows) => rows.clone(),
        other => other
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
    }
}

/// An optional text argument, trimmed, `None` when empty.
pub(super) fn given(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
}

/// Whole cents, as e-Boekhouden's own forms round (`write::round_cents`).
pub(super) fn round2(amount: f64) -> f64 {
    crate::write::round_cents(amount)
}

/// A number as JavaScript writes it into a URL or a text (`${t}`): 50, 12.5.
pub(super) fn js_number(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < 1e15 {
        format!("{}", number as i64)
    } else {
        format!("{number}")
    }
}

pub(super) fn date_object((y, m, d): Date) -> Value {
    json!({"day": d, "month": m, "year": y})
}

/// "12-11-2026", as e-Boekhouden shows a date and as its date object writes itself into a URL (`toString`).
pub(super) fn dutch((y, m, d): Date) -> String {
    format!("{d:02}-{m:02}-{y}")
}

/// The calendar day after `date`.
pub(super) fn next_day((y, m, d): Date) -> Date {
    use chrono::Datelike;
    chrono::NaiveDate::from_ymd_opt(y, m, d)
        .and_then(|date| date.succ_opt())
        .map(|next| (next.year(), next.month(), next.day()))
        .unwrap_or((y, m, d))
}

/// The ids a bulk tool works on: at least one, at most `MAX_BULK`, each once.
pub(super) fn check_ids(ids: &[u64]) -> Result<(), String> {
    if ids.is_empty() {
        return Err("Geef minstens één factuur-id".into());
    }
    if ids.len() > MAX_BULK {
        return Err(format!("Hoogstens {MAX_BULK} facturen tegelijk"));
    }
    let mut seen = std::collections::BTreeSet::new();
    for id in ids {
        if !seen.insert(id) {
            return Err(format!("Factuur-id {id} staat er twee keer in"));
        }
    }
    Ok(())
}

/// A text the API caps at `max` characters, as its forms do.
fn capped(value: &str, max: usize, what: &str) -> Result<(), String> {
    if value.chars().count() > max {
        Err(format!("{what} mag hoogstens {max} tekens zijn"))
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------- offerte mailen

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct OfferteMailenArgs {
    /// Id van de offerte (uit offertes).
    pub id: u64,
    /// E-mailadres van de ontvanger (standaard het e-mailadres van de relatie).
    pub aan: Option<String>,
    /// Id van een e-mailsjabloon voor offertes (uit emailsjablonen) voor onderwerp, bericht en afzender. EB MCP haalt de tekst op zoals e-Boekhouden hem voor deze offerte invult, en toont precies die.
    pub sjabloon_id: Option<u64>,
    /// Eigen onderwerp (gaat voor het sjabloon), hoogstens 110 tekens. Gaat zoals het is: zonder codes, schrijf die uit.
    pub onderwerp: Option<String>,
    /// Eigen bericht als HTML (gaat voor het sjabloon). Gaat zoals het is: zonder codes, schrijf die uit.
    pub bericht: Option<String>,
    /// Cc-adres.
    pub cc: Option<String>,
    /// Bcc-adres.
    pub bcc: Option<String>,
    /// De offerte als pdf meesturen (standaard ja).
    pub pdf: Option<bool>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// Who an e-mail goes to and what it says, settled before the user is asked.
#[derive(Debug, PartialEq)]
pub(super) struct Mail {
    pub aan: String,
    pub cc: Option<String>,
    pub bcc: Option<String>,
    pub van_naam: String,
    pub van_email: String,
    pub onderwerp: String,
    pub bericht: String,
}

/// The e-mail of a quote, as the offerte mail form fills it (`app-offerte-mailen`): the template's text from
/// `offerte/getmaildata/{offerte}/{sjabloon}` (subject, body, ccEmail, bccEmail, vanNaam, vanEmail), the relatie's
/// e-mail as the recipient and the signed-in user as the sender; what the tool is given goes first.
pub(super) fn offerte_mail(
    args: &OfferteMailenArgs,
    template: Option<&Value>,
    relatie: &Value,
    user: &Value,
) -> Result<Mail, String> {
    own_text_without_codes(
        given(&args.onderwerp).as_deref(),
        given(&args.bericht).as_deref(),
    )?;
    let from_template = |key: &str| template.and_then(|t| filled(t, key));
    let aan = given(&args.aan)
        .or_else(|| filled(relatie, "email"))
        .ok_or(
            "De relatie van deze offerte heeft geen e-mailadres; geef de ontvanger op met aan",
        )?;
    let aan = email(&aan, "De ontvanger")?;
    let cc = given(&args.cc)
        .or_else(|| from_template("ccEmail"))
        .map(|v| email(&v, "Het cc-adres"))
        .transpose()?;
    let bcc = given(&args.bcc)
        .or_else(|| from_template("bccEmail"))
        .map(|v| email(&v, "Het bcc-adres"))
        .transpose()?;
    let onderwerp = given(&args.onderwerp)
        .or_else(|| from_template("subject"))
        .ok_or("Geef een onderwerp, of een e-mailsjabloon met sjabloon_id")?;
    capped(&onderwerp, 110, "Het onderwerp")?;
    let bericht = given(&args.bericht)
        .or_else(|| from_template("body"))
        .ok_or("Geef een bericht, of een e-mailsjabloon met sjabloon_id")?;
    let van_naam = from_template("vanNaam")
        .or_else(|| filled(user, "naam"))
        .ok_or("De naam van de afzender is niet bekend")?;
    capped(&van_naam, 50, "De naam van de afzender")?;
    let van_email = from_template("vanEmail")
        .or_else(|| filled(user, "email"))
        .ok_or("Het e-mailadres van de afzender is niet bekend")?;
    let van_email = email(&van_email, "Het adres van de afzender")?;
    Ok(Mail {
        aan,
        cc,
        bcc,
        van_naam,
        van_email,
        onderwerp,
        bericht,
    })
}

/// `POST offerte/verstuur/{offerte}/{pdf}` with the form's body; it answers {sendCount, errorCount, errorMessage}.
pub(super) fn offerte_mail_built(nummer: &str, mail: &Mail, pdf: bool) -> Built {
    Built {
        body: json!({
            "vanNaam": mail.van_naam, "vanEmail": mail.van_email, "aanEmail": mail.aan,
            "ccEmail": mail.cc, "bccEmail": mail.bcc, "subject": mail.onderwerp, "body": mail.bericht,
            "bijlagen": []
        }),
        title: format!("Offerte {nummer} mailen naar {}", mail.aan),
        lines: mail_lines(
            mail,
            if pdf {
                format!("offerte {nummer} als pdf")
            } else {
                "geen; de offerte gaat niet mee".to_owned()
            },
        ),
    }
}

fn mail_lines(mail: &Mail, bijlage: String) -> Vec<Line> {
    [
        Some(line("Aan", &mail.aan)),
        mail.cc.as_ref().map(|cc| line("Cc", cc)),
        mail.bcc.as_ref().map(|bcc| line("Bcc", bcc)),
        Some(line(
            "Van",
            format!("{} <{}>", mail.van_naam, mail.van_email),
        )),
        Some(line("Onderwerp", &mail.onderwerp)),
        Some(line("Bericht", plain_text(&mail.bericht))),
        targets(&mail.bericht).map(|t| line("Links en afbeeldingen in het bericht", t)),
        Some(line("Bijlage", bijlage)),
        Some(line(
            "Let op",
            "dit verstuurt echte e-mail en is niet terug te draaien",
        )),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// What e-Boekhouden answers on a mail: a failure it reports is a failure of the tool.
pub(super) fn mail_failed(sent: &Value) -> Option<String> {
    let errors = sent.get("errorCount").and_then(Value::as_u64).unwrap_or(0);
    let said = filled(sent, "errorMessage");
    let none_sent = sent.get("sendCount").and_then(Value::as_u64) == Some(0);
    (errors > 0 || said.is_some() || none_sent)
        .then(|| said.unwrap_or_else(|| "geen uitleg gegeven".to_owned()))
}

// ---------------------------------------------------------------- offerte factureren

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct OfferteFacturerenArgs {
    /// Id van de offerte (uit offertes).
    pub id: u64,
    /// Alleen deze regels factureren (ids uit de tool offerte). Laat weg voor alle regels.
    pub regel_ids: Option<Vec<u64>>,
    /// Een percentage van het offertebedrag factureren, bijvoorbeeld 50. Niet samen met regel_ids of bedrag.
    pub percentage: Option<f64>,
    /// Een bepaald bedrag factureren (incl of excl btw zoals de offerte is). Niet samen met regel_ids of percentage.
    pub bedrag: Option<f64>,
    /// Factuurdatum, JJJJ-MM-DD (standaard vandaag).
    pub datum: Option<String>,
    /// Factuurnummer (standaard het volgende nummer dat e-Boekhouden voorstelt).
    pub nummer: Option<String>,
    /// Betalingstermijn in dagen (standaard 0, zoals factuur_toevoegen).
    pub termijn: Option<u32>,
    /// De factuur meteen boeken als mutatie (standaard ja, zoals e-Boekhouden het bij een offerte voorstelt).
    pub direct_boeken: Option<bool>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// Which part of a quote becomes the invoice: the three choices of the quote's "Factuur maken".
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Deel {
    /// "Eén of meer offerteregels factureren": the lines (all when `None`), copied.
    Regels(Option<Vec<u64>>),
    /// "Een percentage van het offertebedrag factureren".
    Percentage(f64),
    /// "Een bepaald bedrag factureren".
    Bedrag(f64),
}

impl OfferteFacturerenArgs {
    pub(super) fn deel(&self) -> Result<Deel, String> {
        match (&self.regel_ids, self.percentage, self.bedrag) {
            (None, None, None) => Ok(Deel::Regels(None)),
            (Some(ids), None, None) => {
                if ids.is_empty() {
                    return Err("regel_ids is leeg: laat het weg voor alle regels".into());
                }
                Ok(Deel::Regels(Some(ids.clone())))
            }
            (None, Some(percentage), None) => {
                if !percentage.is_finite() || percentage <= 0.0 || percentage > 999_999_999_999.0 {
                    return Err("Het percentage moet groter dan 0 zijn".into());
                }
                Ok(Deel::Percentage(percentage))
            }
            (None, None, Some(bedrag)) => {
                if !bedrag.is_finite() || bedrag <= 0.0 || bedrag > 999_999_999_999.0 {
                    return Err("Het bedrag moet groter dan 0 zijn".into());
                }
                Ok(Deel::Bedrag(round2(bedrag)))
            }
            _ => Err("Kies één manier: regel_ids, percentage of bedrag".into()),
        }
    }
}

/// What was read for invoicing a quote.
pub(super) struct OfferteBron<'a> {
    /// `offerte/{id}`.
    pub offerte: &'a Value,
    /// `offerteregel/getbyofferteid/{id}`.
    pub regels: &'a Value,
    /// For a percentage or amount in an administration that computes btw per line
    /// (`Administratie/isBtwBerekeningAdministration`): the lines e-Boekhouden computed
    /// (`offerteregel/percentageBedrag/{id}/{t}` or `offerteregel/herberekenBedrag/{id}/{t}`).
    pub berekend: Option<&'a Value>,
    /// `relatie/{relatieId}`, for its name.
    pub relatie: &'a Value,
}

/// One line of the new invoice, with the line's total after discount as e-Boekhouden computed it on the quote
/// (`regelMetKortingExcl`, `regelMetKortingIncl`), when it comes from a quote line.
struct FactuurRegel {
    item: Map<String, Value>,
    met_korting: (Option<f64>, Option<f64>),
}

/// The fields `app-offerte-factureren-regels` copies from a quote line into an invoice line (`btwCode` becomes `btw`).
const REGEL_VELDEN: [(&str, &str); 14] = [
    ("artikelId", "artikelId"),
    ("aantal", "aantal"),
    ("eenheidId", "eenheidId"),
    ("enkelvoud", "enkelvoud"),
    ("meervoud", "meervoud"),
    ("code", "code"),
    ("omschrijving", "omschrijving"),
    ("prijsExcl", "prijsExcl"),
    ("prijsIncl", "prijsIncl"),
    ("btwCode", "btw"),
    ("korting", "korting"),
    ("kortingIsPerc", "kortingIsPerc"),
    ("grootboekId", "grootboekId"),
    ("kostenplaatsId", "kostenplaatsId"),
];

/// A line that stands for part of the quote (percentage or amount): one per btw code, as the web app makes it.
fn deel_regel(
    code: &str,
    omschrijving: &str,
    excl: f64,
    incl: f64,
    btw: &str,
    from: &Value,
) -> FactuurRegel {
    let mut item = Map::new();
    item.insert("aantal".into(), json!(1));
    item.insert("code".into(), json!(code));
    item.insert("omschrijving".into(), json!(omschrijving));
    item.insert("prijsExcl".into(), json!(excl));
    item.insert("prijsIncl".into(), json!(incl));
    item.insert("btw".into(), json!(btw));
    for key in ["grootboekId", "kostenplaatsId"] {
        if let Some(value) = from.get(key).filter(|v| !v.is_null()) {
            item.insert(key.into(), value.clone());
        }
    }
    FactuurRegel {
        item,
        met_korting: (None, None),
    }
}

/// The quote's lines grouped by btw code, in the order the codes first appear (`fi.groupByArray`).
fn per_btw(regels: &[Value]) -> Vec<(String, Vec<&Value>)> {
    let mut groups: Vec<(String, Vec<&Value>)> = Vec::new();
    for regel in regels {
        let code = s(regel, "btwCode");
        match groups.iter_mut().find(|(c, _)| *c == code) {
            Some((_, members)) => members.push(regel),
            None => groups.push((code, vec![regel])),
        }
    }
    groups
}

/// The total of a group of lines with their discount, as the web app adds them up.
fn sum(regels: &[&Value], with_discount: &str, price: &str) -> f64 {
    regels
        .iter()
        .map(|r| {
            amount_of(r, with_discount).unwrap_or_else(|| {
                amount_of(r, "aantal").unwrap_or(0.0) * amount_of(r, price).unwrap_or(0.0)
            })
        })
        .sum()
}

fn factuur_regels(deel: &Deel, bron: &OfferteBron) -> Result<Vec<FactuurRegel>, String> {
    let nummer = s(bron.offerte, "nummer");
    let regels = rows(bron.regels);
    let mut out = Vec::new();
    match deel {
        Deel::Regels(chosen) => {
            let picked: Vec<&Value> = match chosen {
                None => regels.iter().collect(),
                Some(ids) => {
                    let mut picked = Vec::new();
                    for id in ids {
                        let regel = regels
                            .iter()
                            .find(|r| id_of(r, "id") == Some(*id))
                            .ok_or_else(|| {
                                format!("Regel {id} hoort niet bij offerte {nummer}; de regels staan bij de tool offerte")
                            })?;
                        if !picked.iter().any(|p: &&Value| id_of(p, "id") == Some(*id)) {
                            picked.push(regel);
                        }
                    }
                    picked
                }
            };
            if picked.is_empty() {
                return Err(format!(
                    "Offerte {nummer} heeft nog geen regels om te factureren"
                ));
            }
            if picked.len() > MAX_FACTUUR_REGELS {
                return Err(format!(
                    "Hoogstens {MAX_FACTUUR_REGELS} regels tegelijk, zoals in e-Boekhouden"
                ));
            }
            for regel in picked {
                let mut item = Map::new();
                for (from, to) in REGEL_VELDEN {
                    if let Some(value) = regel.get(from).filter(|v| !v.is_null()) {
                        item.insert(to.into(), value.clone());
                    }
                }
                let met_korting = (
                    amount_of(regel, "regelMetKortingExcl"),
                    amount_of(regel, "regelMetKortingIncl"),
                );
                out.push(FactuurRegel { item, met_korting });
            }
        }
        Deel::Percentage(t) | Deel::Bedrag(t) => {
            let (code, omschrijving) = match deel {
                Deel::Percentage(_) => (
                    "PERC",
                    format!("Facturatie {}% t.b.v. offerte {nummer}", js_number(*t)),
                ),
                _ => ("BEDRAG", format!("Facturatie t.b.v. offerte {nummer}")),
            };
            match bron.berekend {
                // e-Boekhouden computed the lines itself: one per line it gives back.
                Some(berekend) => {
                    for regel in rows(berekend) {
                        let (excl, incl) = (
                            amount_of(&regel, "prijsExcl").unwrap_or(0.0),
                            amount_of(&regel, "prijsIncl").unwrap_or(0.0),
                        );
                        out.push(deel_regel(
                            code,
                            &omschrijving,
                            excl,
                            incl,
                            &s(&regel, "btwCode"),
                            &regel,
                        ));
                    }
                }
                // As the web app computes it: per btw code the share of the lines' totals, on the first line's
                // ledger account and cost centre.
                None => {
                    let first = regels.first().ok_or_else(|| {
                        format!("Offerte {nummer} heeft nog geen regels om te factureren")
                    })?;
                    let (totaal_excl, totaal_incl) = (
                        amount_of(bron.offerte, "totaalExcl").unwrap_or(0.0),
                        amount_of(bron.offerte, "totaalIncl").unwrap_or(0.0),
                    );
                    if matches!(deel, Deel::Bedrag(_)) && (totaal_excl == 0.0 || totaal_incl == 0.0)
                    {
                        return Err(format!(
                            "Offerte {nummer} heeft geen totaal om een bedrag van te nemen"
                        ));
                    }
                    for (btw, members) in per_btw(&regels) {
                        let (excl, incl) = (
                            sum(&members, "regelMetKortingExcl", "prijsExcl"),
                            sum(&members, "regelMetKortingIncl", "prijsIncl"),
                        );
                        let (excl, incl) = match deel {
                            Deel::Percentage(_) => {
                                (round2(excl * t / 100.0), round2(incl * t / 100.0))
                            }
                            _ => (
                                round2(excl / totaal_excl * t),
                                round2(incl / totaal_incl * t),
                            ),
                        };
                        out.push(deel_regel(code, &omschrijving, excl, incl, &btw, first));
                    }
                }
            }
            if out.is_empty() {
                return Err(format!(
                    "e-Boekhouden gaf voor offerte {nummer} geen regels om te factureren"
                ));
            }
        }
    }
    for (n, regel) in out.iter().enumerate() {
        let item = &regel.item;
        if item
            .get("btw")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .is_empty()
            || item.get("grootboekId").and_then(Value::as_u64).unwrap_or(0) == 0
        {
            return Err(format!(
                "Regel {} van de offerte heeft geen btw-code of omzetrekening; vul die eerst aan met offerte_wijzigen",
                n + 1
            ));
        }
    }
    Ok(out)
}

/// A line's totals excl. and incl. btw, from the price that leads (excl. on an invoice excl. btw, incl. on one
/// incl. btw) after its discount, with btw on top or taken out; and its discount in words for the summary.
fn regel_totaal(
    regel: &FactuurRegel,
    in_ex: &str,
    books: &Books,
) -> Result<(f64, f64, String), String> {
    let item = Value::Object(regel.item.clone());
    let incl_leads = in_ex == "IN";
    let aantal = amount_of(&item, "aantal").unwrap_or(0.0);
    let prijs = amount_of(&item, if incl_leads { "prijsIncl" } else { "prijsExcl" }).unwrap_or(0.0);
    let korting = amount_of(&item, "korting").unwrap_or(0.0);
    let percentage = item
        .get("kortingIsPerc")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let given = if incl_leads {
        regel.met_korting.1
    } else {
        regel.met_korting.0
    };
    let net = given.unwrap_or_else(|| {
        let bruto = aantal * prijs;
        match (korting > 0.0, percentage) {
            (false, _) => bruto,
            (true, true) => bruto * (1.0 - korting / 100.0),
            (true, false) => bruto - korting,
        }
    });
    let code = s(&item, "btw");
    let pct = books.btw(&code)?.percentage;
    let (excl, incl) = if incl_leads {
        let incl = round2(net);
        (round2(incl / (1.0 + pct / 100.0)), incl)
    } else {
        let excl = round2(net);
        (excl, excl + round2(excl * pct / 100.0))
    };
    let words = match (korting > 0.0, percentage) {
        (false, _) => String::new(),
        (true, true) => format!(", korting {}%", js_number(korting)),
        (true, false) => format!(", korting {}", write::euro(korting)),
    };
    Ok((excl, incl, words))
}

/// The invoice the quote's "Factuur maken" opens, saved as the invoice form saves it: `POST factuur/model` with
/// `factuur.offerteId`, `kenmerk` the quote's number and the quote's e-mail template for invoices
/// (`getFactuurFromState` in the factuur form; the prefill in `app-offerte-factureren-*`).
pub(super) fn offerte_factuur(
    args: &OfferteFacturerenArgs,
    deel: &Deel,
    bron: &OfferteBron,
    context: &write::FactuurContext,
    books: &Books,
) -> Result<Built, String> {
    let offerte_nummer = s(bron.offerte, "nummer");
    let relatie_id = id_of(bron.offerte, "relatieId")
        .filter(|id| *id > 0)
        .ok_or("Deze offerte heeft geen relatie; een factuur heeft er een nodig")?;
    let datum = args
        .datum
        .as_deref()
        .map(parse_date)
        .transpose()?
        .unwrap_or(context.vandaag);
    let nummer = given(&args.nummer).unwrap_or_else(|| context.volgend_nummer.clone());
    if nummer.is_empty() {
        return Err("Geef een factuurnummer".into());
    }
    let termijn = args.termijn.unwrap_or(0);
    let in_ex = if s(bron.offerte, "inEx").eq_ignore_ascii_case("IN") {
        "IN"
    } else {
        "EX"
    };
    let regels = factuur_regels(deel, bron)?;
    let boeken = args.direct_boeken.unwrap_or(true);

    let mut factuur = Map::new();
    factuur.insert("relatieId".into(), json!(relatie_id));
    factuur.insert("nummer".into(), json!(nummer));
    factuur.insert("inEx".into(), json!(in_ex));
    factuur.insert("datum".into(), date_object(datum));
    factuur.insert("termijn".into(), json!(termijn));
    factuur.insert("sjabloonId".into(), json!(context.sjabloon_id));
    factuur.insert(
        "emailSjabloonId".into(),
        id_of(bron.offerte, "factuurEmailsjabloonId")
            .filter(|id| *id > 0)
            .map_or(Value::Null, |id| json!(id)),
    );
    factuur.insert("type".into(), json!("N"));
    factuur.insert("btwLaag".into(), json!(0));
    factuur.insert("btwHoog".into(), json!(0));
    factuur.insert("totaal".into(), json!(0));
    factuur.insert("kenmerk".into(), json!(offerte_nummer));
    factuur.insert("offerteId".into(), json!(args.id));

    let rekening = |id: u64| match books.rekeningen.iter().find(|r| r.id == id) {
        Some(r) => format!("{} {}", r.code, r.naam),
        None => format!("rekening {id}"),
    };
    let relatie_naam =
        filled(bron.relatie, "bedrijf").unwrap_or_else(|| format!("id {relatie_id}"));
    let mut lines = vec![
        line("Offerte", &offerte_nummer),
        line("Relatie", relatie_naam),
        line("Factuurnummer", &nummer),
        line("Datum", dutch(datum)),
        line("Betalingstermijn", format!("{termijn} dagen")),
        line(
            "Wat",
            match deel {
                Deel::Regels(None) => "alle regels van de offerte".to_owned(),
                Deel::Regels(Some(ids)) => format!("{} regel(s) van de offerte", ids.len()),
                Deel::Percentage(t) => format!("{}% van het offertebedrag", js_number(*t)),
                Deel::Bedrag(t) => format!(
                    "een bedrag van {} {}",
                    write::euro(*t),
                    if in_ex == "IN" {
                        "incl. btw"
                    } else {
                        "excl. btw"
                    }
                ),
            },
        ),
    ];
    let mut items = Vec::new();
    let (mut totaal_excl, mut totaal_incl) = (0.0, 0.0);
    for (n, mut regel) in regels.into_iter().enumerate() {
        let (excl, incl, korting) = regel_totaal(&regel, in_ex, books)?;
        totaal_excl += excl;
        totaal_incl += incl;
        let item = &regel.item;
        let field = |key: &str| amount_of(&Value::Object(item.clone()), key).unwrap_or(0.0);
        lines.push(line(
            &format!("Regel {}", n + 1),
            format!(
                "{} x {}: {} per stuk {}{} ({}), op {}",
                js_number(field("aantal")),
                item.get("omschrijving")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                write::euro(field(if in_ex == "IN" {
                    "prijsIncl"
                } else {
                    "prijsExcl"
                })),
                if in_ex == "IN" {
                    "incl. btw"
                } else {
                    "excl. btw"
                },
                korting,
                item.get("btw").and_then(Value::as_str).unwrap_or_default(),
                rekening(field("grootboekId") as u64)
            ),
        ));
        regel.item.insert("id".into(), json!(-((n + 1) as i64)));
        regel.item.insert("factuurId".into(), json!(0));
        regel.item.insert("prijs".into(), Value::Null);
        regel.item.insert("volgorde".into(), json!(n));
        regel.item.entry("kortingIsPerc").or_insert(json!(true));
        items.push(Value::Object(regel.item));
    }
    lines.push(line("Totaal excl. btw", write::euro(totaal_excl)));
    lines.push(line("Btw", write::euro(totaal_incl - totaal_excl)));
    lines.push(line("Totaal incl. btw", write::euro(totaal_incl)));
    let mut body = Map::new();
    body.insert("factuur".into(), Value::Object(factuur));
    body.insert(
        "factuurItems".into(),
        json!({"insert": items, "update": [], "delete": []}),
    );
    if boeken {
        body.insert(
            "verwerk".into(),
            json!({"grootboekrekeningId": books.debiteuren()?.id}),
        );
    }
    lines.push(line(
        "Meteen boeken",
        if boeken {
            "ja, de factuur wordt meteen geboekt als mutatie Factuur verstuurd (zet direct_boeken op false om dat niet te doen)"
        } else {
            "nee, de factuur wordt nog niet geboekt"
        },
    ));
    lines.push(line("Versturen", "de factuur wordt niet verstuurd"));
    Ok(Built {
        body: Value::Object(body),
        title: format!("Factuur {nummer} maken van offerte {offerte_nummer}"),
        lines,
    })
}

// ---------------------------------------------------------------- offerte voortgang

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct OfferteVoortgangToevoegenArgs {
    /// Id van de offerte (uit offertes).
    pub offerte_id: u64,
    /// Soort voortgang op naam, zoals offerte_voortgang ze noemt (bijvoorbeeld Telefoon of Akkoord). Of geef soort_id.
    pub soort: Option<String>,
    /// Id van de soort voortgang (uit offerte_voortgang).
    pub soort_id: Option<u64>,
    /// Onderwerp, hoogstens 500 tekens.
    pub onderwerp: String,
    /// Bericht als HTML (optioneel).
    pub bericht: Option<String>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct OfferteVoortgangWijzigenArgs {
    /// Id van de offerte.
    pub offerte_id: u64,
    /// Id van de voortgang (uit offerte_voortgang).
    pub id: u64,
    /// Nieuwe soort op naam.
    pub soort: Option<String>,
    /// Nieuwe soort op id.
    pub soort_id: Option<u64>,
    /// Nieuw onderwerp.
    pub onderwerp: Option<String>,
    /// Nieuw bericht als HTML.
    pub bericht: Option<String>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct OfferteItemArgs {
    /// Id van de offerte.
    pub offerte_id: u64,
    /// Id van de regel (uit offerte_voortgang of offerte_agenda).
    pub id: u64,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct OfferteIdArgs {
    /// Id van de offerte (uit offertes).
    pub offerte_id: u64,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// The kinds of progress the administration has (`voortgangsoort/selectlist`: [{id, caption}]), by name or id.
pub(super) fn voortgang_soort(
    soort: Option<&str>,
    soort_id: Option<u64>,
    soorten: &Value,
) -> Result<Option<(u64, String)>, String> {
    let all: Vec<(u64, String)> = rows(soorten)
        .iter()
        .filter_map(|r| Some((id_of(r, "id")?, s(r, "caption"))))
        .collect();
    let choose = || {
        if all.is_empty() {
            "Deze administratie heeft nog geen soorten voortgang; maak ze in e-Boekhouden onder Facturen > Voortgangsoorten".to_owned()
        } else {
            format!(
                "Kies uit: {}",
                all.iter()
                    .map(|(id, naam)| format!("{naam} ({id})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    };
    let soort = soort.map(str::trim).filter(|s| !s.is_empty());
    match (soort_id, soort) {
        (Some(_), Some(_)) => Err("Geef soort of soort_id, niet allebei".into()),
        (Some(id), None) => all
            .iter()
            .find(|(i, _)| *i == id)
            .cloned()
            .map(Some)
            .ok_or_else(|| format!("Geen soort voortgang met id {id}. {}", choose())),
        (None, Some(naam)) => all
            .iter()
            .find(|(_, n)| n.eq_ignore_ascii_case(naam))
            .cloned()
            .map(Some)
            .ok_or_else(|| format!("Geen soort voortgang {naam:?}. {}", choose())),
        (None, None) => Ok(None),
    }
}

fn onderwerp_ok(onderwerp: &str) -> Result<String, String> {
    let onderwerp = onderwerp.trim();
    if onderwerp.is_empty() {
        return Err("Het onderwerp mag niet leeg zijn".into());
    }
    capped(onderwerp, 500, "Het onderwerp")?;
    Ok(onderwerp.to_owned())
}

/// `POST offertevoortgang` {offerteId, onderwerp, voortgangsoortId, body}, as `app-offerte-voortgang-form` sends it.
pub(super) fn voortgang_nieuw(
    args: &OfferteVoortgangToevoegenArgs,
    offerte_nummer: &str,
    soorten: &Value,
) -> Result<Built, String> {
    let (soort_id, soort) = voortgang_soort(args.soort.as_deref(), args.soort_id, soorten)?.ok_or(
        "Geef de soort voortgang met soort of soort_id; offerte_voortgang noemt de soorten",
    )?;
    let onderwerp = onderwerp_ok(&args.onderwerp)?;
    let mut body =
        json!({"offerteId": args.offerte_id, "onderwerp": onderwerp, "voortgangsoortId": soort_id});
    let mut lines = vec![
        line("Offerte", offerte_nummer),
        line("Soort", &soort),
        line("Onderwerp", &onderwerp),
    ];
    if let Some(bericht) = given(&args.bericht) {
        lines.push(line("Bericht", plain_text(&bericht)));
        body["body"] = json!(bericht);
    }
    Ok(Built {
        body,
        title: format!("Voortgang toevoegen aan offerte {offerte_nummer}: {soort}"),
        lines,
    })
}

/// `PUT offertevoortgang` {id, offerteId, onderwerp, voortgangsoortId, body}: the entry as it is
/// (`offertevoortgang/getordefault/{id}`) with the named changes.
pub(super) fn voortgang_gewijzigd(
    args: &OfferteVoortgangWijzigenArgs,
    now: &Value,
    offerte_nummer: &str,
    soorten: &Value,
) -> Result<Built, String> {
    let soort_naam = |id: Option<u64>| {
        rows(soorten)
            .iter()
            .find(|r| id.is_some() && id_of(r, "id") == id)
            .map(|r| s(r, "caption"))
            .unwrap_or_else(|| "onbekend".to_owned())
    };
    let old_soort = id_of(now, "voortgangsoortId");
    let old_onderwerp = s(now, "onderwerp");
    let mut soort_id = old_soort;
    let mut onderwerp = old_onderwerp.clone();
    let mut bericht = now.get("body").cloned().unwrap_or(Value::Null);
    let mut lines = vec![
        line("Offerte", offerte_nummer),
        line("Voortgang", &old_onderwerp),
    ];
    if let Some((id, naam)) = voortgang_soort(args.soort.as_deref(), args.soort_id, soorten)?
        && Some(id) != old_soort
    {
        lines.push(line(
            "Soort wordt",
            format!("{naam} (was {})", soort_naam(old_soort)),
        ));
        soort_id = Some(id);
    }
    if let Some(nieuw) = &args.onderwerp {
        let nieuw = onderwerp_ok(nieuw)?;
        if nieuw != old_onderwerp {
            lines.push(line(
                "Onderwerp wordt",
                format!("{nieuw} (was {old_onderwerp})"),
            ));
            onderwerp = nieuw;
        }
    }
    if let Some(nieuw) = &args.bericht {
        lines.push(line("Bericht wordt", plain_text(nieuw)));
        bericht = json!(nieuw.trim());
    }
    if lines.len() == 2 {
        return Err("Er is niets gewijzigd: geef aan wat er anders moet".into());
    }
    let soort_id =
        soort_id.ok_or("Deze voortgang heeft geen soort; geef er een met soort of soort_id")?;
    let mut body = json!({"id": args.id, "offerteId": args.offerte_id, "onderwerp": onderwerp, "voortgangsoortId": soort_id});
    if !bericht.is_null() {
        body["body"] = bericht;
    }
    Ok(Built {
        body,
        title: format!("Voortgang van offerte {offerte_nummer} wijzigen"),
        lines,
    })
}

// ---------------------------------------------------------------- offerte agenda

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct OfferteAgendaToevoegenArgs {
    /// Id van de offerte (uit offertes).
    pub offerte_id: u64,
    /// Datum van de afspraak of de volgende actie, JJJJ-MM-DD.
    pub datum: String,
    /// Tijd, UU:MM (24 uur).
    pub tijd: String,
    /// Onderwerp, hoogstens 500 tekens.
    pub onderwerp: String,
    /// Bericht als HTML (optioneel).
    pub bericht: Option<String>,
    /// Id van de medewerker (uit offerte_agenda). Zonder: de ingelogde gebruiker, als e-Boekhouden die met id teruggeeft; anders is medewerker_id nodig.
    pub medewerker_id: Option<u64>,
    /// Datum waarop het is afgehandeld, JJJJ-MM-DD (optioneel).
    pub afgehandeld: Option<String>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct OfferteAgendaWijzigenArgs {
    /// Id van de offerte.
    pub offerte_id: u64,
    /// Id van de afspraak (uit offerte_agenda).
    pub id: u64,
    /// Nieuwe datum, JJJJ-MM-DD.
    pub datum: Option<String>,
    /// Nieuwe tijd, UU:MM.
    pub tijd: Option<String>,
    /// Nieuw onderwerp.
    pub onderwerp: Option<String>,
    /// Nieuw bericht als HTML.
    pub bericht: Option<String>,
    /// Andere medewerker (id uit offerte_agenda).
    pub medewerker_id: Option<u64>,
    /// Afgehandeld op, JJJJ-MM-DD; een lege tekst maakt het weer open.
    pub afgehandeld: Option<String>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// "14:30" -> (14, 30).
pub(super) fn parse_time(text: &str) -> Result<(u32, u32), String> {
    let bad = || format!("Ongeldige tijd {text:?}: gebruik UU:MM");
    let (h, m) = text.trim().split_once(':').ok_or_else(bad)?;
    let (h, m): (u32, u32) = (h.parse().map_err(|_| bad())?, m.parse().map_err(|_| bad())?);
    if h > 23 || m > 59 {
        return Err(bad());
    }
    Ok((h, m))
}

/// A moment as the agenda form sends it: the wall-clock time written as if it were UTC
/// (`Li.addTimezoneOffset(date).toISOString()`), so "2026-11-12 14:30" is "2026-11-12T14:30:00.000Z".
pub(super) fn moment(datum: Date, (h, m): (u32, u32)) -> String {
    format!("{}T{h:02}:{m:02}:00.000Z", dashed(datum))
}

/// A date and time as the API gives them back ("2026-11-12T14:30:00"): read as the wall clock it stands for.
fn moment_of(value: &Value) -> Option<(Date, (u32, u32))> {
    let text = value.as_str()?;
    let datum = parse_date(text.get(..10)?).ok()?;
    let tijd = text
        .get(11..16)
        .and_then(|t| parse_time(t).ok())
        .unwrap_or((0, 0));
    Some((datum, tijd))
}

fn medewerker(id: u64, users: &Value) -> Result<String, String> {
    let all = rows(users);
    all.iter()
        .find(|u| id_of(u, "id") == Some(id))
        .map(|u| s(u, "naam"))
        .ok_or_else(|| {
            format!(
                "Geen medewerker met id {id}. Kies uit: {}",
                all.iter()
                    .filter_map(|u| Some(format!("{} ({})", s(u, "naam"), id_of(u, "id")?)))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

/// `POST offerteagenda` {offerteId, onderwerp, body, datum, afgehandeld, gebruikerId}, as `app-offerte-agenda-form`.
pub(super) fn agenda_nieuw(
    args: &OfferteAgendaToevoegenArgs,
    offerte_nummer: &str,
    users: &Value,
    ingelogd: Option<u64>,
) -> Result<Built, String> {
    let datum = parse_date(&args.datum)?;
    let tijd = parse_time(&args.tijd)?;
    let onderwerp = onderwerp_ok(&args.onderwerp)?;
    let gebruiker = args.medewerker_id.or(ingelogd).ok_or(
        "e-Boekhouden zegt niet wie er is ingelogd; geef de medewerker op met medewerker_id (offerte_agenda noemt de medewerkers)",
    )?;
    let naam = medewerker(gebruiker, users)?;
    let mut body = json!({"offerteId": args.offerte_id, "onderwerp": onderwerp, "datum": moment(datum, tijd), "gebruikerId": gebruiker});
    let mut lines = vec![
        line("Offerte", offerte_nummer),
        line(
            "Wanneer",
            format!("{} om {:02}:{:02}", dutch(datum), tijd.0, tijd.1),
        ),
        line("Medewerker", &naam),
        line("Onderwerp", &onderwerp),
    ];
    if let Some(bericht) = given(&args.bericht) {
        lines.push(line("Bericht", plain_text(&bericht)));
        body["body"] = json!(bericht);
    }
    if let Some(klaar) = given(&args.afgehandeld) {
        let klaar = parse_date(&klaar)?;
        lines.push(line("Afgehandeld op", dutch(klaar)));
        body["afgehandeld"] = json!(moment(klaar, (0, 0)));
    }
    Ok(Built {
        body,
        title: format!("Afspraak toevoegen bij offerte {offerte_nummer}: {onderwerp}"),
        lines,
    })
}

/// `PUT offerteagenda` with the whole entry: as it is (`offerteagenda/getordefault/{id}`) with the named changes.
pub(super) fn agenda_gewijzigd(
    args: &OfferteAgendaWijzigenArgs,
    now: &Value,
    offerte_nummer: &str,
    users: &Value,
) -> Result<Built, String> {
    let (old_datum, old_tijd) = now
        .get("datum")
        .and_then(moment_of)
        .ok_or("e-Boekhouden gaf deze afspraak zonder datum terug")?;
    let old_onderwerp = s(now, "onderwerp");
    let mut lines = vec![
        line("Offerte", offerte_nummer),
        line("Afspraak", &old_onderwerp),
    ];
    let datum = args
        .datum
        .as_deref()
        .map(parse_date)
        .transpose()?
        .unwrap_or(old_datum);
    let tijd = args
        .tijd
        .as_deref()
        .map(parse_time)
        .transpose()?
        .unwrap_or(old_tijd);
    if (datum, tijd) != (old_datum, old_tijd) {
        lines.push(line(
            "Wanneer wordt",
            format!(
                "{} om {:02}:{:02} (was {} om {:02}:{:02})",
                dutch(datum),
                tijd.0,
                tijd.1,
                dutch(old_datum),
                old_tijd.0,
                old_tijd.1
            ),
        ));
    }
    let mut onderwerp = old_onderwerp.clone();
    if let Some(nieuw) = &args.onderwerp {
        let nieuw = onderwerp_ok(nieuw)?;
        if nieuw != old_onderwerp {
            lines.push(line(
                "Onderwerp wordt",
                format!("{nieuw} (was {old_onderwerp})"),
            ));
            onderwerp = nieuw;
        }
    }
    let mut bericht = now.get("body").cloned().unwrap_or(Value::Null);
    if let Some(nieuw) = &args.bericht {
        lines.push(line("Bericht wordt", plain_text(nieuw)));
        bericht = json!(nieuw.trim());
    }
    let old_gebruiker = id_of(now, "gebruikerId");
    let gebruiker = match args.medewerker_id {
        Some(id) if Some(id) != old_gebruiker => {
            let was = old_gebruiker
                .and_then(|old| medewerker(old, users).ok())
                .unwrap_or_else(|| "onbekend".to_owned());
            lines.push(line(
                "Medewerker wordt",
                format!("{} (was {was})", medewerker(id, users)?),
            ));
            Some(id)
        }
        _ => old_gebruiker,
    };
    let gebruiker =
        gebruiker.ok_or("Deze afspraak heeft geen medewerker; geef er een met medewerker_id")?;
    let old_klaar = now.get("afgehandeld").and_then(moment_of).map(|(d, _)| d);
    let klaar = match args.afgehandeld.as_deref().map(str::trim) {
        None => old_klaar,
        Some("") => {
            if old_klaar.is_some() {
                lines.push(line("Afgehandeld", "wordt weer open"));
            }
            None
        }
        Some(datum) => {
            let datum = parse_date(datum)?;
            if Some(datum) != old_klaar {
                lines.push(line("Afgehandeld op wordt", dutch(datum)));
            }
            Some(datum)
        }
    };
    if lines.len() == 2 {
        return Err("Er is niets gewijzigd: geef aan wat er anders moet".into());
    }
    let mut body = json!({"id": args.id, "offerteId": args.offerte_id, "onderwerp": onderwerp, "datum": moment(datum, tijd), "gebruikerId": gebruiker});
    if !bericht.is_null() {
        body["body"] = bericht;
    }
    if let Some(klaar) = klaar {
        body["afgehandeld"] = json!(moment(klaar, (0, 0)));
    }
    Ok(Built {
        body,
        title: format!("Afspraak bij offerte {offerte_nummer} wijzigen"),
        lines,
    })
}

// ---------------------------------------------------------------- incasso

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct IncassobestandArgs {
    /// Uitvoerdatum van de incasso, JJJJ-MM-DD (standaard morgen, zoals e-Boekhouden voorstelt).
    pub uitvoerdatum: Option<String>,
    /// Map op deze computer om het bestand in te zetten, als volledig pad (standaard de map Downloads).
    pub map: Option<String>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// What the incasso form (`app-incassobestand-maken-form`) checks and sends: the administration's own incasso
/// IBAN and contract id (`administratie/incassoinfo`, never changed here), the invoices that are ready
/// (`factuur/incassoinfo` {aantal, bedrag}, and per invoice `factuur/machtiginggrid` with its problems in
/// `machtigingMessage`), and `GET incassobestand/create?iban=&incassoidrekhouder=&uitvoerdatum=DD-MM-YYYY`.
/// The invoices ready for incasso as the user approves them: compared again right before the file is made.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct IncassoLijst {
    pub aantal: usize,
    pub bedrag: f64,
    /// The invoice numbers, sorted.
    pub nummers: Vec<String>,
    pub facturen: Vec<Value>,
}

/// The invoices ready for incasso, from `factuur/incassoinfo` and `factuur/machtiginggrid`, refused when they do not
/// agree or one has a problem with its mandate (the web app does not offer the file then).
pub(super) fn incasso_lijst(info: &Value, facturen: &Value) -> Result<IncassoLijst, String> {
    let aantal = id_of(info, "aantal").unwrap_or(0) as usize;
    if aantal == 0 {
        return Err("Er staan op dit moment geen facturen klaar voor incasso".into());
    }
    let bedrag = amount_of(info, "bedrag").unwrap_or(0.0);
    let list = rows(facturen);
    let problems: Vec<String> = list
        .iter()
        .filter_map(|f| filled(f, "machtigingMessage").map(|m| format!("{}: {m}", s(f, "nummer"))))
        .collect();
    if !problems.is_empty() {
        return Err(format!(
            "e-Boekhouden maakt het bestand niet zolang deze facturen een probleem met hun machtiging hebben: {}",
            problems.join("; ")
        ));
    }
    let som: f64 = list
        .iter()
        .map(|f| amount_of(f, "bedrag").unwrap_or(0.0))
        .sum();
    if list.len() != aantal || (som - bedrag).abs() > 0.005 {
        return Err(format!(
            "e-Boekhouden meldt {aantal} facturen voor {} klaar voor incasso, maar EB MCP ziet er {} voor {}. Maak het bestand in e-Boekhouden zelf, zodat je ziet welke facturen meegaan.",
            write::euro(bedrag),
            list.len(),
            write::euro(som)
        ));
    }
    let mut nummers: Vec<String> = list.iter().map(|f| s(f, "nummer")).collect();
    nummers.sort();
    Ok(IncassoLijst {
        aantal,
        bedrag,
        nummers,
        facturen: list,
    })
}

/// `GET incassobestand/create` takes no list: e-Boekhouden puts in every invoice ready at that moment. So right
/// before it is called, the list is read again and must be the one the user approved.
pub(super) fn incasso_zelfde(
    approved: &IncassoLijst,
    info: &Value,
    facturen: &Value,
) -> Result<(), String> {
    let now = incasso_lijst(info, facturen).map_err(|why| {
        format!(
            "Er is geen incassobestand gemaakt: sinds de goedkeuring is er iets veranderd. {why}"
        )
    })?;
    if now.aantal == approved.aantal
        && (now.bedrag - approved.bedrag).abs() <= 0.005
        && now.nummers == approved.nummers
    {
        return Ok(());
    }
    let extra: Vec<&str> = now
        .nummers
        .iter()
        .filter(|n| !approved.nummers.contains(n))
        .map(String::as_str)
        .collect();
    let weg: Vec<&str> = approved
        .nummers
        .iter()
        .filter(|n| !now.nummers.contains(n))
        .map(String::as_str)
        .collect();
    let mut what = vec![format!(
        "goedgekeurd waren {} facturen voor {}, nu staan er {} voor {} klaar",
        approved.aantal,
        write::euro(approved.bedrag),
        now.aantal,
        write::euro(now.bedrag)
    )];
    if !extra.is_empty() {
        what.push(format!("erbij: {}", extra.join(", ")));
    }
    if !weg.is_empty() {
        what.push(format!("niet meer: {}", weg.join(", ")));
    }
    Err(format!(
        "Er is geen incassobestand gemaakt: de facturen die klaarstaan voor incasso zijn veranderd sinds de goedkeuring ({}). Vraag het opnieuw aan, zodat je ziet welke facturen meegaan.",
        what.join("; ")
    ))
}

pub(super) fn incasso_plan(
    uitvoer: Date,
    vandaag: Date,
    admin: &Value,
    info: &Value,
    facturen: &Value,
) -> Result<(Built, Query, IncassoLijst), String> {
    let iban: String = s(admin, "iban")
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_uppercase();
    if iban.is_empty() {
        return Err("Er is in e-Boekhouden geen IBAN voor incasso ingesteld. Stel die zelf in e-Boekhouden in; EB MCP wijzigt hem niet.".into());
    }
    let contract = s(admin, "incassoContractId");
    if contract.is_empty() {
        return Err("Er is in e-Boekhouden geen incassocontract-ID ingesteld. Stel die zelf in e-Boekhouden in; EB MCP wijzigt hem niet.".into());
    }
    if !contract.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("Het incassocontract-ID in e-Boekhouden mag enkel letters en cijfers bevatten; pas het daar aan".into());
    }
    if uitvoer < vandaag {
        return Err("De uitvoerdatum ligt in het verleden".into());
    }
    let lijst = incasso_lijst(info, facturen)?;
    let (aantal, bedrag) = (lijst.aantal, lijst.bedrag);
    let query = vec![
        ("iban".to_owned(), iban.clone()),
        ("incassoidrekhouder".to_owned(), contract.clone()),
        ("uitvoerdatum".to_owned(), dutch(uitvoer)),
    ];
    let mut lines = vec![
        line("Incasseren op rekening", &iban),
        line("Incassocontract-ID", &contract),
        line("Uitvoerdatum", dutch(uitvoer)),
    ];
    for factuur in &lijst.facturen {
        lines.push(line(
            &format!("Factuur {}", s(factuur, "nummer")),
            format!(
                "{}: {}",
                known(s(factuur, "bedrijf")),
                write::euro(amount_of(factuur, "bedrag").unwrap_or(0.0))
            ),
        ));
    }
    lines.push(line(
        "Totaal",
        format!("{} voor {aantal} facturen", write::euro(bedrag)),
    ));
    lines.push(line(
        "Gevolg",
        "e-Boekhouden maakt een SEPA-incassobestand van deze facturen; ze staan daarna niet meer klaar voor incasso. EB MCP zet het bestand op deze computer; het geld wordt pas geïncasseerd als iemand het bestand bij de bank aanbiedt.",
    ));
    lines.push(line(
        "Let op",
        "dit is een betaalopdracht voor de bank en is in EB MCP niet terug te draaien. Komt er voor het maken een factuur bij of gaat er een af, dan maakt EB MCP het bestand niet.",
    ));
    Ok((
        Built {
            body: json!({"iban": iban, "incassoidrekhouder": contract, "uitvoerdatum": dutch(uitvoer)}),
            title: format!(
                "Incassobestand maken: {aantal} facturen, {}",
                write::euro(bedrag)
            ),
            lines,
        },
        query,
        lijst,
    ))
}

/// A file name e-Boekhouden gave for the new incasso file: one path segment, nothing else.
pub(super) fn safe_file_name(name: &str) -> Option<&str> {
    let name = name.trim();
    (!name.is_empty()
        && name.len() <= 200
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !name.contains(".."))
    .then_some(name)
}

// ---------------------------------------------------------------- Peppol

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct PeppolVersturenArgs {
    /// Id van de factuur (uit facturen).
    pub id: u64,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// One invoice over Peppol, as the invoice list's "P" does (`app-factuur-versturen-peppol`): `POST peppol/send`
/// with the invoice's id as the body. The list offers it only when the relatie has a Peppol ID and the invoice
/// was not sent before (`factuur/haspeppolverzending/{id}`); an invoice goes over Peppol once.
pub(super) fn peppol_plan(
    id: u64,
    factuur: &Value,
    relatie: &Value,
    verstuurd: &Value,
) -> Result<Built, String> {
    let nummer = s(factuur, "nummer");
    if verstuurd.as_bool() == Some(true) {
        return Err(format!(
            "Factuur {nummer} is al eens via Peppol verstuurd; dat kan maar één keer. Maak zo nodig een nieuwe factuur."
        ));
    }
    let bedrijf = known(s(relatie, "bedrijf"));
    let peppol_id = filled(relatie, "peppolId").ok_or_else(|| {
        format!("Relatie {bedrijf} heeft geen Peppol-ID; zonder kan de factuur niet via Peppol")
    })?;
    Ok(Built {
        body: json!(id),
        title: format!("Factuur {nummer} via Peppol versturen naar {bedrijf}"),
        lines: vec![
            line("Factuur", &nummer),
            line("Aan", format!("{bedrijf} (Peppol-ID {peppol_id})")),
            line(
                "Totaal incl. btw",
                write::euro(amount_of(factuur, "totaal").unwrap_or(0.0)),
            ),
            line(
                "Let op",
                "de factuur gaat via het Peppol-netwerk naar de klant; dat kan maar één keer en is niet terug te draaien",
            ),
        ],
    })
}

// ---------------------------------------------------------------- facturen mailen en afdrukken

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FacturenMailenArgs {
    /// Ids van de facturen (uit facturen), hoogstens 50. Elke factuur gaat naar het factuur- of e-mailadres van zijn eigen relatie.
    pub ids: Vec<u64>,
    /// Id van een e-mailsjabloon voor facturen (uit emailsjablonen). EB MCP haalt per factuur de tekst op zoals e-Boekhouden hem invult, en toont precies die.
    pub sjabloon_id: Option<u64>,
    /// Eigen onderwerp voor elke mail (gaat voor het sjabloon), hoogstens 110 tekens. Gaat zoals het is: zonder codes zoals [FACTNUMMER].
    pub onderwerp: Option<String>,
    /// Eigen bericht als HTML voor elke mail (gaat voor het sjabloon). Gaat zoals het is: zonder codes.
    pub bericht: Option<String>,
    /// Cc-adres voor elke mail.
    pub cc: Option<String>,
    /// Bcc-adres voor elke mail.
    pub bcc: Option<String>,
    /// Ook het UBL-bestand meesturen (standaard niet).
    pub ubl: Option<bool>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FacturenAfdrukkenArgs {
    /// Ids van de facturen (uit facturen), hoogstens 50.
    pub ids: Vec<u64>,
    /// Map op deze computer om de pdf's in te zetten, als volledig pad (standaard de map Downloads).
    pub map: Option<String>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// One invoice of a bulk send: its mail, settled per invoice as factuur_mailen settles one (`factuur_mail`).
#[derive(Debug, PartialEq)]
pub(super) struct Ontvanger {
    pub factuur_id: u64,
    pub nummer: String,
    pub relatie: String,
    pub mail: FactuurMail,
}

/// The address an invoice goes to: the relatie's invoice address, else its general one.
pub(super) fn factuur_adres(relatie: &Value) -> Option<String> {
    filled(relatie, "emailFactuur").or_else(|| filled(relatie, "email"))
}

type Part = fn(&FactuurMail) -> String;

fn van(m: &FactuurMail) -> String {
    format!("{} <{}>", m.van_naam, m.van_email)
}

fn cc_of(m: &FactuurMail) -> String {
    m.cc.clone().unwrap_or_default()
}

fn bcc_of(m: &FactuurMail) -> String {
    m.bcc.clone().unwrap_or_default()
}

fn onderwerp_of(m: &FactuurMail) -> String {
    m.onderwerp.clone()
}

fn bericht_of(m: &FactuurMail) -> String {
    plain_text(&m.bericht)
}

/// Every mail of a bulk send, each the body `POST factuur/mail` takes for one invoice (the request factuur_mailen
/// sends), under `mails` in the order they go out. The approval shows each invoice's recipient, subject and
/// message as they will be sent; what all mails share is shown once.
pub(super) fn facturen_mail_built(ontvangers: &[Ontvanger], ubl: bool) -> Built {
    let mails: Vec<Value> = ontvangers
        .iter()
        .map(|o| factuur_mail_body(&o.mail, ubl, o.factuur_id))
        .collect();
    let parts: [(&str, Part); 5] = [
        ("van", van),
        ("cc", cc_of),
        ("bcc", bcc_of),
        ("onderwerp", onderwerp_of),
        ("bericht", bericht_of),
    ];
    let mut per_mail: Vec<(&str, Part)> = Vec::new();
    let mut shared = Vec::new();
    for (label, part) in parts {
        let first = ontvangers
            .first()
            .map(|o| part(&o.mail))
            .unwrap_or_default();
        if ontvangers.iter().all(|o| part(&o.mail) == first) {
            if !first.is_empty() {
                let mut label = label.to_owned();
                label[..1].make_ascii_uppercase();
                shared.push(line(&format!("{label} bij elke mail"), first));
            }
        } else {
            per_mail.push((label, part));
        }
    }
    let mut lines = Vec::new();
    for o in ontvangers {
        lines.push(line(
            &format!("Factuur {}", o.nummer),
            format!("aan {} ({})", o.mail.aan, o.relatie),
        ));
        for (label, part) in &per_mail {
            let value = part(&o.mail);
            if !value.is_empty() {
                lines.push(line(&format!("Factuur {}, {label}", o.nummer), value));
            }
        }
        if let Some(links) = targets(&o.mail.bericht) {
            lines.push(line(
                &format!("Factuur {}, links en afbeeldingen", o.nummer),
                links,
            ));
        }
    }
    lines.extend(shared);
    lines.push(line(
        "Bijlage",
        if ubl {
            "de eigen factuur als pdf en als UBL"
        } else {
            "de eigen factuur als pdf"
        },
    ));
    lines.push(line(
        "Let op",
        format!(
            "dit verstuurt {} echte e-mails, één per factuur, precies zoals hierboven, en is niet terug te draaien",
            ontvangers.len()
        ),
    ));
    Built {
        body: json!({"mails": mails}),
        title: format!("{} facturen mailen", ontvangers.len()),
        lines,
    }
}

/// Whether an error of a change means e-Boekhouden certainly did not do it: it refused in words, the session
/// was gone, or the request never went out. A timeout or a lost connection can come after the change was done.
pub(super) fn certainly_not_done(error: &str) -> bool {
    [
        "e-Boekhouden weigert dit",
        "Niet (meer) ingelogd",
        "Niet ingelogd",
        "Geen toegang",
        "Dit bestaat niet (meer) bij e-Boekhouden",
        "e-Boekhouden vraagt om rustiger",
    ]
    .iter()
    .any(|start| error.starts_with(start))
}

/// What a bulk mail that stopped at invoice `at` says: what went out, what happened to that one, what was not tried.
pub(super) fn bulk_mail_stopped(
    ontvangers: &[Ontvanger],
    at: usize,
    why: &str,
    certain: bool,
) -> String {
    let nummers = |part: &[Ontvanger]| {
        if part.is_empty() {
            "geen".to_owned()
        } else {
            part.iter()
                .map(|o| o.nummer.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        }
    };
    let nummer = &ontvangers[at].nummer;
    let this = if certain {
        format!("Factuur {nummer} is niet verstuurd: {why}.")
    } else {
        format!(
            "Factuur {nummer}: onbekend, mogelijk verstuurd ({why}). Kijk in e-Boekhouden of hij is verstuurd voordat je hem opnieuw mailt."
        )
    };
    format!(
        "{this} Verstuurd: {}. Niet geprobeerd: {}.",
        nummers(&ontvangers[..at]),
        nummers(&ontvangers[at + 1..])
    )
}

// ---------------------------------------------------------------- archief ontkoppelen

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ArchiefOntkoppelenArgs {
    /// Id van het bestand in het digitaal archief (uit digitaal_archief).
    pub bestand_id: u64,
    /// Ontkoppelen van deze mutatie (mutatienummer).
    pub mutatie_nr: Option<u64>,
    /// Alle koppelingen van het bestand verbreken: mutaties, bankregels, relaties en offertes. Niet samen met mutatie_nr.
    pub alles: Option<bool>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// One link of a file (`folder/{id}/koppel` gives them per kind) in words, with the id `DELETE folderkoppel/{id}` takes.
fn link_words(kind: &str, item: &Value) -> String {
    match kind {
        "koppelMutatie" => format!(
            "mutatie {}",
            known(
                item.get("nummer")
                    .map(|n| n.to_string().trim_matches('"').to_owned())
                    .unwrap_or_default()
            )
        ),
        "koppelImport" => format!(
            "bankregel van {} ({})",
            known(s(item, "datum").get(..10).unwrap_or_default().to_owned()),
            write::euro(amount_of(item, "bedrag").unwrap_or(0.0))
        ),
        "koppelRelatie" => format!("relatie {} {}", s(item, "relatieCode"), s(item, "bedrijf"))
            .trim()
            .to_owned(),
        "koppelOfferte" => format!("offerte {}", known(s(item, "nummer"))),
        _ => "iets anders".to_owned(),
    }
}

/// The links to undo: the one with the named mutatie, or all of them. The digitaal archief's own screens unlink
/// with `DELETE folderkoppel/{koppel id}` ("alleen de koppeling wordt verwijderd, het bestand blijft behouden").
pub(super) fn ontkoppel_plan(
    naam: &str,
    links: &Value,
    mutatie_nr: Option<u64>,
    alles: bool,
) -> Result<(Built, Vec<u64>, bool), String> {
    if mutatie_nr.is_some() == alles {
        return Err(
            "Geef mutatie_nr om één mutatie los te maken, of alles: true voor alle koppelingen"
                .into(),
        );
    }
    let kinds = [
        "koppelMutatie",
        "koppelImport",
        "koppelRelatie",
        "koppelOfferte",
    ];
    let mut all: Vec<(&str, Value)> = Vec::new();
    for kind in kinds {
        for item in links
            .get(kind)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            all.push((kind, item));
        }
    }
    let chosen: Vec<&(&str, Value)> = match mutatie_nr {
        Some(nr) => all
            .iter()
            .filter(|(kind, item)| *kind == "koppelMutatie" && id_of(item, "nummer") == Some(nr))
            .collect(),
        None => all.iter().collect(),
    };
    if chosen.is_empty() {
        let now = if all.is_empty() {
            "nergens aan".to_owned()
        } else {
            format!(
                "alleen aan {}",
                all.iter()
                    .map(|(k, i)| link_words(k, i))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        return Err(match mutatie_nr {
            Some(nr) => {
                format!("{naam} is niet gekoppeld aan mutatie {nr}; het is gekoppeld {now}")
            }
            None => format!("{naam} is nergens aan gekoppeld"),
        });
    }
    let mut ids = Vec::new();
    let mut lines = vec![line("Bestand", naam)];
    // A link to a mutatie comes back with archief_koppelen; one to a bank line, relatie or quote EB MCP cannot restore.
    let danger = chosen.iter().any(|(kind, _)| *kind != "koppelMutatie");
    for (kind, item) in &chosen {
        let id = id_of(item, "id")
            .ok_or("e-Boekhouden gaf een koppeling zonder id terug; ontkoppel dit bestand in e-Boekhouden zelf")?;
        ids.push(id);
        lines.push(line("Ontkoppelen van", link_words(kind, item)));
    }
    lines.push(line(
        "Gevolg",
        "alleen de koppeling verdwijnt; het bestand blijft in het digitaal archief en kan daarna worden verwijderd",
    ));
    if danger {
        lines.push(line(
            "Let op",
            "een koppeling met een bankregel, relatie of offerte kan EB MCP niet terugzetten; dat moet dan in e-Boekhouden zelf",
        ));
    }
    Ok((
        Built {
            body: json!({"koppelIds": ids}),
            title: format!("Bestand ontkoppelen: {naam}"),
            lines,
        },
        ids,
        danger,
    ))
}

// ---------------------------------------------------------------- e-mailsjabloon codes

/// The codes e-Boekhouden fills in an e-mail for an invoice, from its own page "Sjablooncodes factuur mailen"
/// (`app-factuur-mailen-uitleg-popup`, linked from the mail form's "Sjablooncodes").
pub(super) const EMAIL_CODES: [(&str, &str); 33] = [
    ("[RELBEDRIJF]", "De bedrijfsnaam van je relatie"),
    ("[RELCONTACTPERSOON]", "De contactpersoon van je relatie"),
    ("[RELCODE]", "De code van je relatie"),
    ("[RELAANHEF]", "De aanhef van je relatie"),
    ("[RELADRES]", "Het adres van je relatie"),
    ("[RELPOSTCODE]", "De postcode van je relatie"),
    ("[RELPLAATS]", "De plaats van je relatie"),
    ("[RELLAND]", "Het land van je relatie"),
    ("[RELADRES2]", "Het vestigingsadres van je relatie"),
    ("[RELPOSTCODE2]", "De vestigingspostcode van je relatie"),
    ("[RELPLAATS2]", "De vestigingsplaats van je relatie"),
    ("[RELLAND2]", "Het land van je relatie"),
    ("[RELEMAIL]", "E-mailadres relatie"),
    ("[RELEMAILFACTUUR]", "E-mailadres tbv facturen van relatie"),
    ("[RELIBAN]", "De iban uit de machtiging"),
    (
        "[RELSEPAMACHTIGINGID]",
        "Het machtigingskenmerk uit de machtiging",
    ),
    ("[RELBTWNR]", "Het btw-nummer van je relatie"),
    ("[FACTDATUM]", "De factuurdatum"),
    ("[VERVALDATUM]", "De vervaldatum van je factuur"),
    ("[FACTNUMMER]", "Factuurnummer"),
    ("[FACTKENMERK]", "Het kenmerk van je factuur"),
    ("[FACTTERMIJN]", "Betalingstermijn"),
    ("[TOTAAL]", "Het totaalbedrag van je factuur, inclusief btw"),
    ("[ABBEDRIJF]", "Je bedrijfsnaam"),
    ("[ABADRES]", "Je adres"),
    ("[ABPOSTCODE]", "Je postcode"),
    ("[ABPLAATS]", "Je plaats"),
    ("[ABTELEFOON]", "Je telefoonnummer"),
    ("[ABEMAIL]", "Je e-mailadres"),
    ("[ABWEBSITE]", "Het adres van je website"),
    ("[ABOBNUMMER]", "Je btw-id (indien gevuld) of OB-nummer"),
    (
        "[ABKVKNUMMER]",
        "Je KVK-nummer (administraties in Nederland)",
    ),
    (
        "[ABONDERNEMINGSNR]",
        "Je ondernemingsnummer (administraties in België)",
    ),
];
