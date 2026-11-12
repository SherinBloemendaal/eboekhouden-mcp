//! The request bodies and approval summaries of this part, built from what the tools read first. Pure functions,
//! so the tests can check every body and every line without e-Boekhouden.

use super::super::{email, plain_text, targets};
use crate::approvals::line;
use crate::write::{Built, euro};
use serde_json::{Value, json};

/// The most open posten e-Boekhouden's own screen takes in one go (reminders and writing off).
pub(super) const MAX_POSTEN: usize = 1000;
/// The most messages one call marks as read or archives.
pub(super) const MAX_BERICHTEN: usize = 200;

/// A value of a grid row or an object, whatever the case of its name: e-Boekhouden's grids are not consistent.
pub(super) fn field<'a>(row: &'a Value, name: &str) -> Option<&'a Value> {
    let object = row.as_object()?;
    object.get(name).or_else(|| {
        object
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value)
    })
}

/// A field as text: a string as it is (trimmed), a number written out, anything else empty.
pub(super) fn text(row: &Value, name: &str) -> String {
    match field(row, name) {
        Some(Value::String(s)) => s.trim().to_owned(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// A field as a number: a JSON number, or text that reads as one (with a decimal point or comma).
pub(super) fn number(row: &Value, name: &str) -> Option<f64> {
    match field(row, name)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().replace(',', ".").parse().ok(),
        _ => None,
    }
}

/// The rows of a list: a plain array, or the `data` of a grid (after `named_rows`).
pub(super) fn rows(list: &Value) -> Vec<Value> {
    match list {
        Value::Array(rows) => rows.clone(),
        other => other
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
    }
}

pub(super) fn round2(amount: f64) -> f64 {
    (amount * 100.0).round() / 100.0
}

pub(super) fn money(amount: f64) -> String {
    format!("€ {}", euro(amount))
}

/// "2026-11-06T00:00:00" -> "06-11-2026", "2026-11-06T14:03:12" -> "06-11-2026 14:03"; other text unchanged.
pub(super) fn moment(value: &str) -> String {
    let value = value.trim();
    let (date, time) = match value.split_once('T') {
        Some((date, time)) => (date, Some(time)),
        None => (value, None),
    };
    let parts: Vec<&str> = date.split('-').collect();
    let [y, m, d] = parts.as_slice() else {
        return value.to_owned();
    };
    if y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return value.to_owned();
    }
    match time.map(|t| t.get(..5).unwrap_or(t)) {
        Some(hm) if hm != "00:00" => format!("{d}-{m}-{y} {hm}"),
        _ => format!("{d}-{m}-{y}"),
    }
}

fn dutch_date((y, m, d): (i32, u32, u32)) -> String {
    format!("{d:02}-{m:02}-{y}")
}

// ---------------------------------------------------------------- open posten

/// Which list of open posten: e-Boekhouden's `soort` in the grid and whether it is about suppliers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kant {
    Debiteuren,
    Crediteuren,
}

impl Kant {
    /// The grid's `soort`, and what the screens send as `soort` of a selected post.
    pub(super) fn soort(self) -> &'static str {
        match self {
            Self::Debiteuren => "Debiteur",
            Self::Crediteuren => "Crediteur",
        }
    }

    fn naam(self) -> &'static str {
        match self {
            Self::Debiteuren => "open posten debiteuren (te ontvangen)",
            Self::Crediteuren => "open posten crediteuren (te betalen)",
        }
    }
}

/// An open post as the grid `openpost/gridtable/openpost` gives it, with what the tools need.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Post {
    pub factuur: String,
    pub relatie_id: u64,
    pub relatie: String,
    /// The invoice's amount (`sBedrag`).
    pub bedrag: Option<f64>,
    /// What is still open (`teOntvangen`; e-Boekhouden uses that name for crediteuren too).
    pub open: f64,
    pub vervaldatum: String,
}

impl Post {
    pub(super) fn from_row(row: &Value) -> Option<Self> {
        let factuur = text(row, "mutFactuur");
        let relatie_id = field(row, "relatieId").and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        })?;
        if factuur.is_empty() {
            return None;
        }
        Some(Self {
            factuur,
            relatie_id,
            relatie: text(row, "bedrijf"),
            bedrag: number(row, "sBedrag"),
            open: number(row, "teOntvangen")?,
            vervaldatum: text(row, "vervaldatum"),
        })
    }

    fn summary(&self) -> String {
        let mut out = format!("{}: nog open {}", known(&self.relatie), money(self.open));
        if let Some(bedrag) = self.bedrag.filter(|b| round2(*b) != round2(self.open)) {
            out.push_str(&format!(" van {}", money(bedrag)));
        }
        if !self.vervaldatum.is_empty() {
            out.push_str(&format!(", vervaldatum {}", moment(&self.vervaldatum)));
        }
        out
    }
}

fn known(value: &str) -> String {
    if value.trim().is_empty() {
        "onbekend".to_owned()
    } else {
        value.trim().to_owned()
    }
}

/// Whether a list of open posten can be asked for at all: one or more, at most `MAX_POSTEN`, each with an
/// invoice number, none twice. Checked before anything is read.
pub(super) fn check_wanted(wanted: &[(String, u64)]) -> Result<(), String> {
    if wanted.is_empty() {
        return Err("Geef minstens één open post (factuurnummer met relatie_id)".into());
    }
    if wanted.len() > MAX_POSTEN {
        return Err(format!(
            "Hoogstens {MAX_POSTEN} open posten tegelijk, zoals in e-Boekhouden zelf"
        ));
    }
    for (n, (factuur, relatie_id)) in wanted.iter().enumerate() {
        let factuur = factuur.trim();
        if factuur.is_empty() {
            return Err("Een factuurnummer is leeg".into());
        }
        if wanted[..n]
            .iter()
            .any(|(f, r)| r == relatie_id && f.trim().eq_ignore_ascii_case(factuur))
        {
            return Err(format!("Factuur {factuur} staat er twee keer in"));
        }
    }
    Ok(())
}

/// The relaties of a list of open posten, each once.
pub(super) fn relatie_ids(wanted: impl IntoIterator<Item = u64>) -> Vec<u64> {
    let mut ids: Vec<u64> = wanted.into_iter().collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// The posten the tool was asked for, in that order, each found among the open posten of its relatie.
/// Asking for one twice, or for one that is not open, is refused.
pub(super) fn find_posten(
    wanted: &[(String, u64)],
    open: &[(u64, Vec<Post>)],
) -> Result<Vec<Post>, String> {
    check_wanted(wanted)?;
    let mut found = Vec::with_capacity(wanted.len());
    for (factuur, relatie_id) in wanted {
        let factuur = factuur.trim();
        let post = open
            .iter()
            .filter(|(id, _)| id == relatie_id)
            .flat_map(|(_, posts)| posts)
            .find(|p| p.factuur == factuur)
            .or_else(|| {
                open.iter()
                    .filter(|(id, _)| id == relatie_id)
                    .flat_map(|(_, posts)| posts)
                    .find(|p| p.factuur.eq_ignore_ascii_case(factuur))
            })
            .ok_or_else(|| {
                format!(
                    "Factuur {factuur} van relatie {relatie_id} staat niet open; zoek de open posten op met open_posten"
                )
            })?;
        found.push(post.clone());
    }
    Ok(found)
}

/// An e-mail template for reminders (`aanmaansjabloon`, soort E).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Aanmaansjabloon {
    pub id: u64,
    pub naam: String,
    pub onderwerp: String,
    pub tekst: String,
}

/// A relatie's addresses, as `relatie/{id}` gives them.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Ontvanger {
    pub relatie_id: u64,
    pub naam: String,
    pub herinnering: String,
    pub factuur: String,
    pub algemeen: String,
}

impl Ontvanger {
    pub(super) fn from_relatie(relatie_id: u64, relatie: &Value) -> Self {
        Self {
            relatie_id,
            naam: text(relatie, "bedrijf"),
            herinnering: text(relatie, "emailHerinnering"),
            factuur: text(relatie, "emailFactuur"),
            algemeen: text(relatie, "email"),
        }
    }

    /// The addresses the relatie has, named, or `None` when it has none.
    fn adressen(&self) -> Option<String> {
        let named: Vec<String> = [
            ("herinneringen", &self.herinnering),
            ("facturen", &self.factuur),
            ("algemeen", &self.algemeen),
        ]
        .into_iter()
        .filter(|(_, address)| !address.is_empty())
        .map(|(what, address)| format!("{address} ({what})"))
        .collect();
        (!named.is_empty()).then(|| named.join(", "))
    }
}

/// `POST OpenPost/aanmaning`, as e-Boekhouden's aanmaning form sends it for reminders by e-mail.
pub(super) fn aanmaning(
    posten: &[Post],
    ontvangers: &[Ontvanger],
    sjabloon: &Aanmaansjabloon,
    kopie: Option<&str>,
    bijlage: bool,
) -> Result<Built, String> {
    let kopie = match kopie.map(str::trim).filter(|k| !k.is_empty()) {
        Some(kopie) if kopie.chars().count() > 200 => {
            return Err("Het kopie-adres is langer dan 200 tekens".into());
        }
        Some(kopie) => Some(email(kopie, "Het kopie-adres")?),
        None => None,
    };
    if let Some(post) = posten.iter().find(|p| round2(p.open) <= 0.0) {
        return Err(format!(
            "Factuur {} staat niet meer te ontvangen (nog open {}); daar gaat geen herinnering voor",
            post.factuur,
            money(post.open)
        ));
    }
    if !ontvangers.iter().any(|o| o.adressen().is_some()) {
        return Err(
            "Geen van deze relaties heeft een e-mailadres; e-Boekhouden kan de herinnering niet versturen. Vul eerst een e-mailadres in met relatie_wijzigen.".into(),
        );
    }
    let mut lines = vec![
        line("Verzendwijze", "per e-mail, vanuit e-Boekhouden"),
        line("Sjabloon", &sjabloon.naam),
        line("Onderwerp", &sjabloon.onderwerp),
        line("Tekst", plain_text(&sjabloon.tekst)),
    ];
    if let Some(links) = targets(&sjabloon.tekst) {
        lines.push(line("Links en afbeeldingen in de tekst", links));
    }
    lines.push(line(
        "Codes",
        "e-Boekhouden vult codes als [NAAM], [FACTNUMMER] en [OPENSTAAND] per relatie en factuur in",
    ));
    // Which address gets the mail is e-Boekhouden's choice, and the bundle does not show it: the approval says
    // so and lists what EB MCP knows of each relatie, without claiming who receives it or how many mails go out.
    lines.push(line(
        "Ontvanger",
        "e-Boekhouden kiest zelf naar welk e-mailadres van de relatie de herinnering gaat",
    ));
    for ontvanger in ontvangers {
        lines.push(line(
            &format!("Relatie {}", known(&ontvanger.naam)),
            match ontvanger.adressen() {
                Some(adressen) => format!("bekende e-mailadressen: {adressen}"),
                None => "EB MCP kent geen e-mailadres van deze relatie; e-Boekhouden verstuurt dan niets aan haar".to_owned(),
            },
        ));
    }
    for post in posten {
        lines.push(line(&format!("Factuur {}", post.factuur), post.summary()));
    }
    let totaal: f64 = posten.iter().map(|p| p.open).sum();
    lines.push(line("Totaal openstaand", money(totaal)));
    lines.push(line(
        "Bijlage",
        if bijlage {
            "de facturen als pdf"
        } else {
            "geen"
        },
    ));
    if let Some(kopie) = &kopie {
        lines.push(line("Kopie naar", kopie));
    }
    lines.push(line(
        "Let op",
        "dit verstuurt echte e-mail naar de relaties en is niet terug te draaien",
    ));
    let title = match posten {
        [post] => format!(
            "Herinnering versturen: factuur {} aan {}",
            post.factuur,
            known(&post.relatie)
        ),
        _ => format!("Herinneringen versturen voor {} facturen", posten.len()),
    };
    Ok(Built {
        body: json!({
            "CreditDebet": "D",
            "DatumTot": Value::Null,
            "OpenPosten": posten.iter().map(|p| json!({"factuur": p.factuur, "relatieId": p.relatie_id})).collect::<Vec<_>>(),
            "SjabloonId": sjabloon.id,
            "FactuurAlsBijlage": bijlage,
            "KopieSturenNaar": kopie,
            // The form takes this from the page's query string, so it goes out as text.
            "ProefAanmaning": "false",
        }),
        title,
        lines,
    })
}

/// A ledger account the open post can be written off to, from `grootboekrekening/selectlist/actief/VWBAL`.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Tegenrekening {
    pub id: u64,
    pub code: String,
    pub omschrijving: String,
}

impl Tegenrekening {
    pub(super) fn from_row(row: &Value) -> Option<Self> {
        Some(Self {
            id: field(row, "id")?.as_u64()?,
            code: text(row, "code"),
            omschrijving: text(row, "omschrijving"),
        })
    }
}

/// The ledger account to write off to: the one named by code, or the one e-Boekhouden's form suggests
/// (the first whose name holds "betalingsverschillen").
pub(super) fn pick_tegenrekening(
    choices: &[Tegenrekening],
    code: Option<&str>,
) -> Result<Tegenrekening, String> {
    match code.map(str::trim).filter(|c| !c.is_empty()) {
        Some(code) => choices
            .iter()
            .find(|r| r.code.eq_ignore_ascii_case(code))
            .cloned()
            .ok_or_else(|| {
                format!(
                    "Grootboekrekening {code} is geen actieve balans- of resultatenrekening; kies er een uit grootboek"
                )
            }),
        None => choices
            .iter()
            .find(|r| r.omschrijving.to_lowercase().contains("betalingsverschillen"))
            .cloned()
            .ok_or_else(|| {
                "Deze administratie heeft geen rekening Betalingsverschillen; geef de tegenrekening (code) op".into()
            }),
    }
}

/// `POST openpost/wegboeken`, as e-Boekhouden's afboek form sends it: one item per open post.
pub(super) fn afboeken(
    kant: Kant,
    posten: &[Post],
    tegenrekening: &Tegenrekening,
    datum: (i32, u32, u32),
    omschrijving: &str,
    bedrag: Option<f64>,
) -> Result<Built, String> {
    let omschrijving = omschrijving.trim();
    if omschrijving.is_empty() || omschrijving.chars().count() > 50 {
        return Err("Geef een omschrijving van 1 tot 50 tekens".into());
    }
    if bedrag.is_some() && posten.len() != 1 {
        return Err(
            "Een eigen bedrag kan alleen bij één open post; bij meer wordt van elke post het hele openstaande bedrag afgeboekt".into(),
        );
    }
    let mut items = Vec::with_capacity(posten.len());
    let mut lines = vec![line("Soort", kant.naam())];
    let mut totaal = 0.0;
    for post in posten {
        let amount = round2(bedrag.unwrap_or(post.open));
        // An own amount writes off part of what is open, in the same direction: a creditor post open at -500 is
        // never written off with +500, which would book 1000 the wrong way.
        if bedrag.is_some() && (amount * post.open < 0.0 || amount.abs() > round2(post.open).abs())
        {
            return Err(format!(
                "Factuur {}: het bedrag {} past niet bij wat er open staat ({}); geef een bedrag met hetzelfde teken, hoogstens het openstaande bedrag",
                post.factuur,
                money(amount),
                money(post.open)
            ));
        }
        if !amount.is_finite() || amount == 0.0 {
            return Err(format!(
                "Factuur {}: er is niets af te boeken (bedrag {})",
                post.factuur,
                money(amount)
            ));
        }
        if !(-999_999_999.0..=9_999_999_999.0).contains(&amount) {
            return Err(format!(
                "Factuur {}: het bedrag valt buiten wat e-Boekhouden aanneemt",
                post.factuur
            ));
        }
        totaal += amount;
        let rest = round2(post.open - amount);
        let mut summary = format!(
            "{}: nog open {}, afgeboekt {}",
            known(&post.relatie),
            money(post.open),
            money(amount)
        );
        if rest != 0.0 {
            summary.push_str(&format!(", blijft open {}", money(rest)));
        }
        lines.push(line(&format!("Factuur {}", post.factuur), summary));
        items.push(json!({
            "datum": {"day": datum.2, "month": datum.1, "year": datum.0},
            "bedrag": amount,
            "omschrijving": omschrijving,
            "tegenrekeningId": tegenrekening.id,
            "factuurNummer": post.factuur,
            "relatieId": post.relatie_id,
            "crediteuren": kant == Kant::Crediteuren,
        }));
    }
    lines.push(line(
        "Tegenrekening",
        format!("{} {}", tegenrekening.code, tegenrekening.omschrijving),
    ));
    lines.push(line("Datum", dutch_date(datum)));
    lines.push(line("Omschrijving", omschrijving));
    if posten.len() > 1 {
        lines.push(line("Totaal afgeboekt", money(round2(totaal))));
    }
    lines.push(line(
        "Gevolg",
        "e-Boekhouden boekt het bedrag van de open post naar de tegenrekening; de factuur staat daarna niet meer (of voor het restbedrag) open. De vordering of schuld wordt zo afgeschreven.",
    ));
    let title = match posten {
        [post] => format!(
            "Open post afboeken: factuur {} van {}",
            post.factuur,
            known(&post.relatie)
        ),
        _ => format!("{} open posten afboeken", posten.len()),
    };
    Ok(Built {
        body: Value::Array(items),
        title,
        lines,
    })
}

/// What e-Boekhouden answered on `openpost/wegboeken`: `Ok` with the result for the AI client when at least one
/// post was written off, `Err` when none was.
pub(super) fn afgeboekt(answer: &Value) -> Result<Value, String> {
    let gelukt = answer
        .get("successIds")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let fouten: Vec<String> = answer
        .get("errorIds")
        .and_then(Value::as_array)
        .map(|errors| {
            errors
                .iter()
                .map(|e| {
                    format!(
                        "factuur {}: {}",
                        text(e, "factuur"),
                        text(e, "errorMessage")
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    if gelukt.is_empty() && !fouten.is_empty() {
        return Err(format!(
            "e-Boekhouden heeft niets afgeboekt: {}",
            fouten.join("; ")
        ));
    }
    Ok(json!({"afgeboekt": gelukt, "fouten": fouten}))
}

/// What e-Boekhouden answered on `OpenPost/aanmaning`. `Err` only when the answer says that nothing was sent
/// (`sendEmails` is there and 0). Any other answer that does not say how many went out may still have sent mail:
/// it comes back as an unknown outcome with the advice to look at the history first, never as a plain error
/// that invites sending the same reminder again.
pub(super) fn verzonden(answer: &Value) -> Result<Value, String> {
    let zonder = field(answer, "noEmail")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    match field(answer, "sendEmails").map(Value::as_u64) {
        Some(Some(0)) => Err(if zonder > 0 {
            format!(
                "e-Boekhouden heeft geen herinnering verstuurd: bij {zonder} relatie(s) is geen e-mailadres bekend"
            )
        } else {
            "e-Boekhouden heeft geen herinnering verstuurd en gaf geen uitleg".to_owned()
        }),
        Some(Some(verstuurd)) => Ok(json!({
            "uitkomst": "verstuurd",
            "verstuurd": verstuurd,
            "niet_verstuurd_zonder_emailadres": zonder,
        })),
        _ => Ok(json!({
            "uitkomst": "onbekend",
            "antwoord_van_e_boekhouden": answer,
            "let_op": "e-Boekhouden nam de opdracht aan maar zei niet hoeveel herinneringen er zijn verstuurd. Er kan dus al e-mail zijn verstuurd. Probeer het niet opnieuw voordat de historie (aanmaning_historie per factuur) is bekeken, en vertel de gebruiker dat de uitkomst onbekend is.",
        })),
    }
}

/// Whether the answer of `POST auth` says the signed-in user is an accountant (`isAccountant`), as
/// e-Boekhouden's own app reads it (`!!t.isAccountant`).
pub(super) fn is_accountant(auth: &Value) -> bool {
    match field(auth, "isAccountant") {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        _ => false,
    }
}

// ---------------------------------------------------------------- berichten

/// What happens to selected messages in the inbox: e-Boekhouden's two bulk actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BerichtActie {
    Gelezen,
    Archiveren,
}

impl BerichtActie {
    pub(super) fn path(self) -> &'static str {
        match self {
            Self::Gelezen => "bericht/markread",
            Self::Archiveren => "bericht/archive",
        }
    }
}

/// The ids of the messages to mark or archive, each found in the inbox, with a line per message.
pub(super) fn berichten_actie(
    actie: BerichtActie,
    ids: &[u64],
    inbox: &[Value],
) -> Result<Built, String> {
    if ids.is_empty() {
        return Err("Geef minstens één bericht-id (uit berichten)".into());
    }
    if ids.len() > MAX_BERICHTEN {
        return Err(format!("Hoogstens {MAX_BERICHTEN} berichten tegelijk"));
    }
    let mut lines = Vec::with_capacity(ids.len() + 1);
    let mut seen = Vec::with_capacity(ids.len());
    for id in ids {
        if seen.contains(id) {
            return Err(format!("Bericht {id} staat er twee keer in"));
        }
        seen.push(*id);
        let row = inbox
            .iter()
            .find(|row| field(row, "id").and_then(Value::as_u64) == Some(*id))
            .ok_or_else(|| {
                format!("Bericht {id} staat niet in het postvak; alleen berichten in het postvak kunnen dit")
            })?;
        let mut summary = known(&text(row, "onderwerp"));
        let van = text(row, "van");
        if !van.is_empty() {
            summary = format!("van {van}: {summary}");
        }
        let datum = text(row, "datum");
        if !datum.is_empty() {
            summary = format!("{} {summary}", moment(&datum));
        }
        lines.push(line(&format!("Bericht {id}"), summary));
    }
    let (title, gevolg) = match actie {
        BerichtActie::Gelezen => (
            "Berichten markeren als gelezen",
            "de berichten blijven in het postvak, als gelezen",
        ),
        BerichtActie::Archiveren => (
            "Berichten naar het archief",
            "de berichten gaan van het postvak naar het archief; ze zijn daar nog te lezen",
        ),
    };
    lines.push(line("Gevolg", gevolg));
    Ok(Built {
        body: json!(ids),
        title: if ids.len() == 1 {
            title.replace("Berichten", "Bericht")
        } else {
            format!("{title} ({})", ids.len())
        },
        lines,
    })
}

/// The accountant of the administration, from `administratie/accountantinfo`: who a message or a check request
/// goes to. `None` when the administration has none.
pub(super) fn accountant(info: &Value) -> Option<String> {
    let bedrijf = text(info, "bedrijf");
    if bedrijf.is_empty() {
        return None;
    }
    let plaats = text(info, "plaats");
    Some(if plaats.is_empty() {
        bedrijf
    } else {
        format!("{bedrijf} te {plaats}")
    })
}

/// The subject of a reply, as e-Boekhouden's "Antwoorden" fills it in.
pub(super) fn antwoord_onderwerp(original: &str) -> String {
    let original = original.trim();
    if original.starts_with("Re:") {
        original.to_owned()
    } else {
        format!("Re: {original}")
    }
}

/// `POST bericht` from an administration: the message goes to its accountant.
pub(super) fn bericht(
    aan: &str,
    onderwerp: &str,
    tekst: &str,
    antwoord_op: Option<&str>,
) -> Result<Built, String> {
    let onderwerp = onderwerp.trim();
    if onderwerp.is_empty() || onderwerp.chars().count() > 100 {
        return Err("Geef een onderwerp van 1 tot 100 tekens".into());
    }
    if tekst.trim().is_empty() || tekst.chars().count() > 40_000 {
        return Err("Geef een bericht van 1 tot 40000 tekens".into());
    }
    let mut lines = vec![line(
        "Aan",
        format!("{aan} (de accountant van deze administratie)"),
    )];
    if let Some(original) = antwoord_op {
        lines.push(line("Antwoord op", original));
    }
    lines.push(line("Onderwerp", onderwerp));
    lines.push(line("Bericht", tekst.trim()));
    lines.push(line(
        "Let op",
        "dit verstuurt een bericht aan de accountant via e-Boekhouden en is niet terug te draaien",
    ));
    Ok(Built {
        body: json!({"onderwerp": onderwerp, "body": tekst.trim()}),
        title: format!("Bericht versturen aan {aan}: {onderwerp}"),
        lines,
    })
}

// ---------------------------------------------------------------- controleopdrachten

const MAANDEN: [&str; 12] = [
    "januari",
    "februari",
    "maart",
    "april",
    "mei",
    "juni",
    "juli",
    "augustus",
    "september",
    "oktober",
    "november",
    "december",
];

/// `POST controleopdracht`: an administration asks its accountant to check a month or a quarter. As the form
/// does, the body carries a month and a quarter both; `periodeType` says which counts.
pub(super) fn controle_aanvraag(
    aan: &str,
    kwartaal_periode: bool,
    jaar: i32,
    maand: Option<u32>,
    kwartaal: Option<u32>,
    opmerking: &str,
    vandaag: (i32, u32, u32),
) -> Result<Built, String> {
    let (this_year, this_month, _) = vandaag;
    if !(this_year - 8..=this_year + 3).contains(&jaar) {
        return Err(format!(
            "Kies een jaar van {} tot en met {}",
            this_year - 8,
            this_year + 3
        ));
    }
    if kwartaal_periode && maand.is_some() {
        return Err("Geef bij periode kwartaal een kwartaal, geen maand".into());
    }
    if !kwartaal_periode && kwartaal.is_some() {
        return Err("Geef bij periode maand een maand, geen kwartaal".into());
    }
    let maand_value = maand.unwrap_or(this_month);
    let kwartaal_value = kwartaal.unwrap_or(this_month.div_ceil(3));
    if !(1..=12).contains(&maand_value) {
        return Err("Een maand is 1 tot en met 12".into());
    }
    if !(1..=4).contains(&kwartaal_value) {
        return Err("Een kwartaal is 1 tot en met 4".into());
    }
    if opmerking.chars().count() > 8000 {
        return Err("De opmerking is langer dan 8000 tekens".into());
    }
    let periode = if kwartaal_periode {
        format!("{kwartaal_value}e kwartaal {jaar}")
    } else {
        format!("{} {jaar}", MAANDEN[(maand_value - 1) as usize])
    };
    let mut lines = vec![
        line(
            "Aan",
            format!("{aan} (de accountant van deze administratie)"),
        ),
        line("Periode", &periode),
    ];
    if !opmerking.trim().is_empty() {
        lines.push(line("Opmerking", opmerking.trim()));
    }
    lines.push(line(
        "Let op",
        "dit stuurt een controle-opdracht naar de accountant en is niet terug te draaien",
    ));
    Ok(Built {
        body: json!({
            "periodeType": if kwartaal_periode { "K" } else { "M" },
            "jaar": jaar,
            "maand": maand_value,
            "kwartaal": kwartaal_value,
            "opmerking": opmerking.trim(),
        }),
        title: format!("Controle-opdracht aanvragen: {periode}"),
        lines,
    })
}

/// `PUT controleopdracht/accountant/opmerking`: the accountant approves or rejects a period, with a remark.
pub(super) fn controle_beoordeling(
    id: u64,
    opdracht: &Value,
    goedkeuren: bool,
    opmerking: &str,
) -> Result<Built, String> {
    if opmerking.chars().count() > 8000 {
        return Err("De opmerking is langer dan 8000 tekens".into());
    }
    let periode = known(&text(opdracht, "periode"));
    let mut lines = vec![line("Periode", &periode)];
    let datum = text(opdracht, "datum");
    if !datum.is_empty() {
        lines.push(line("Aangevraagd op", moment(&datum)));
    }
    let gevraagd = text(opdracht, "opmerkingen");
    if !gevraagd.is_empty() {
        lines.push(line("Opmerkingen van de klant", gevraagd));
    }
    lines.push(line(
        "Besluit",
        if goedkeuren { "goedkeuren" } else { "afkeuren" },
    ));
    if !opmerking.trim().is_empty() {
        lines.push(line("Opmerking aan de klant", opmerking.trim()));
    }
    lines.push(line(
        "Let op",
        "de klant ziet dit besluit in e-Boekhouden; het is niet terug te draaien",
    ));
    Ok(Built {
        body: json!({"id": id, "accept": goedkeuren, "opmerking": opmerking.trim()}),
        title: format!(
            "Controle-opdracht {}: {periode}",
            if goedkeuren { "goedkeuren" } else { "afkeuren" }
        ),
        lines,
    })
}
