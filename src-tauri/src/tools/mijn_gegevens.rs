//! Mijn gegevens: the administration's own company details and invoice address, and e-Boekhouden's invoices to the user.
//!
//! The tools of this part live here, on their own router (`mijn_gegevens_router`), which tools.rs adds to the
//! server's. Each tool is named in one of the lists below, which the tool-list test in mcp.rs reads.
//!
//! Request shapes come from e-Boekhouden's own web app (the public Angular bundle):
//! - `GET administratie/mijngegevens?adm=true` reads the company details; `PUT` on the same path and query saves
//!   them with exactly the fields of the edit form (`app-mijn-gegevens-edit-form`), nothing else of the model.
//! - `GET` / `PUT skillsource/relatie/mijngegevens/factuuradres` read and save the address e-Boekhouden sends its
//!   own invoices to (`app-factuuradres-form`). The form reads `telefoon`, `mobiel` and `fax` and saves them as
//!   `telefoonnummer`, `mobielnummer` and `faxnummer`.
//! - `GET skillsource/factuur/recent/gridtable` and `skillsource/openstaandeposten/gridtable` list e-Boekhouden's
//!   invoices to the user; `GET SkillSource/factuur/recent/pdf/{id}` gives one as a pdf.
//!
//! Never built here, on purpose: the incasso account (`administratie/automatischeincasso`), the licence and
//! cancelling the subscription.

use super::*;

/// The tools of this part that only read.
#[cfg(test)]
pub(super) const READS: &[&str] = &[
    "bedrijfsgegevens",
    "factuuradres",
    "eboekhouden_facturen",
    "eboekhouden_factuur_pdf",
];
/// The tools of this part that change the books.
#[cfg(test)]
pub(super) const WRITES: &[&str] = &["bedrijfsgegevens_wijzigen", "factuuradres_wijzigen"];
/// The ones in `WRITES` that cannot be undone.
#[cfg(test)]
pub(super) const DESTRUCTIVE: &[&str] = &[];

const MIJN_GEGEVENS: &str = "administratie/mijngegevens";
const FACTUURADRES: &str = "skillsource/relatie/mijngegevens/factuuradres";
const RECENTE_FACTUREN: &str = "skillsource/factuur/recent/gridtable";
const OPENSTAANDE_FACTUREN: &str = "skillsource/openstaandeposten/gridtable";

/// The query the edit screen reads and saves the company details with: the administration's own, also for an
/// accountant (who would otherwise get the accountant's details).
fn adm() -> Vec<(String, String)> {
    vec![("adm".into(), "true".into())]
}

// ---------------------------------------------------------------- the fields of the two forms

/// How a field's new value is cleaned and checked, as the web app's form does it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Check {
    Text,
    Email,
    /// Upper case without spaces, and a Dutch postcode as "1234 AB".
    Postcode,
    /// Digits only; spaces and dots are dropped.
    Kvk,
    /// The OB-nummer (btw-nummer), with the eleven test of the web app.
    ObNummer,
    /// The btw-id of a sole proprietorship: NL plus twelve characters, modulo 97 as the web app checks it.
    BtwId,
    Iban,
    Bic,
    /// One of the codes of `Geslacht`.
    Geslacht,
}

/// One field of a form: the key it is saved under, the keys it can be read from, and its rules.
struct Field {
    key: &'static str,
    read: &'static [&'static str],
    label: &'static str,
    max: usize,
    required: bool,
    check: Check,
    /// The form's own value when the model has none.
    default: Option<&'static str>,
}

const fn field(
    key: &'static str,
    read: &'static [&'static str],
    label: &'static str,
    max: usize,
    required: bool,
    check: Check,
) -> Field {
    Field {
        key,
        read,
        label,
        max,
        required,
        check,
        default: None,
    }
}

/// The edit form of Beheer > Mijn gegevens (Dutch administrations), in its order. Lengths and required fields are
/// the form's own.
const BEDRIJF: &[Field] = &[
    field(
        "bedrijf",
        &["bedrijf"],
        "Bedrijfsnaam",
        100,
        true,
        Check::Text,
    ),
    field("naam", &["naam"], "Contactpersoon", 50, true, Check::Text),
    Field {
        key: "geslacht",
        read: &["geslacht"],
        label: "Geslacht",
        max: 1,
        required: true,
        check: Check::Geslacht,
        default: Some("m"),
    },
    field("adres", &["adres"], "Adres", 50, true, Check::Text),
    field(
        "postcode",
        &["postcode"],
        "Postcode",
        50,
        true,
        Check::Postcode,
    ),
    field("plaats", &["plaats"], "Plaats", 50, true, Check::Text),
    field(
        "telefoon",
        &["telefoon"],
        "Telefoon",
        50,
        false,
        Check::Text,
    ),
    field("email", &["email"], "E-mailadres", 150, true, Check::Email),
    field("website", &["website"], "Website", 50, false, Check::Text),
    field(
        "obNummer",
        &["obNummer"],
        "Btw-nummer (OB-nummer)",
        50,
        false,
        Check::ObNummer,
    ),
    field(
        "btwId",
        &["btwId"],
        "Btw-id (eenmanszaak)",
        50,
        false,
        Check::BtwId,
    ),
    field(
        "kvkNummer",
        &["kvkNummer"],
        "KvK-nummer",
        20,
        false,
        Check::Kvk,
    ),
    field("iban", &["iban"], "IBAN", 50, false, Check::Iban),
    field("bic", &["bic"], "BIC", 20, false, Check::Bic),
];

/// Fields of the edit form that no tool argument sets: they go back exactly as read.
const BEDRIJF_KEEP: &[&str] = &["klantcode"];

/// The form of Beheer > Mijn gegevens > Factuuradres. It reads `telefoon`, `mobiel` and `fax` and saves them under
/// other names; the relatie names are read as well, so a value is never lost to a name.
const FACTUUR: &[Field] = &[
    field(
        "bedrijf",
        &["bedrijf"],
        "Bedrijfsnaam",
        100,
        true,
        Check::Text,
    ),
    field(
        "contactpersoon",
        &["contactpersoon"],
        "Contactpersoon",
        50,
        false,
        Check::Text,
    ),
    Field {
        key: "geslacht",
        read: &["geslacht"],
        label: "Geslacht",
        max: 1,
        required: false,
        check: Check::Geslacht,
        default: Some("m"),
    },
    field("adres", &["adres"], "Adres", 50, true, Check::Text),
    field(
        "postcode",
        &["postcode"],
        "Postcode",
        50,
        true,
        Check::Postcode,
    ),
    field("plaats", &["plaats"], "Plaats", 50, true, Check::Text),
    field("land", &["land"], "Land", 50, false, Check::Text),
    field(
        "telefoonnummer",
        &["telefoon", "telefoonnummer"],
        "Telefoon",
        50,
        false,
        Check::Text,
    ),
    field(
        "mobielnummer",
        &["mobiel", "mobielnummer"],
        "Mobiel",
        50,
        false,
        Check::Text,
    ),
    field(
        "emailFactuur",
        &["emailFactuur"],
        "E-mailadres voor facturen",
        150,
        true,
        Check::Email,
    ),
    field(
        "faxnummer",
        &["fax", "faxnummer"],
        "Fax",
        50,
        false,
        Check::Text,
    ),
];

/// The aanhef e-Boekhouden keeps with the contact person.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Geslacht {
    Man,
    Vrouw,
    Afdeling,
    /// Geen aanhef.
    Geen,
}

impl Geslacht {
    fn code(self) -> &'static str {
        match self {
            Self::Man => "m",
            Self::Vrouw => "v",
            Self::Afdeling => "a",
            Self::Geen => "b",
        }
    }
}

/// A geslacht code as the user reads it.
fn geslacht_naam(code: &str) -> String {
    match code {
        "m" => "man".into(),
        "v" => "vrouw".into(),
        "a" => "afdeling".into(),
        "b" | "" => "geen".into(),
        other => other.into(),
    }
}

// ---------------------------------------------------------------- parameters

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct BedrijfsgegevensWijzigenArgs {
    /// Bedrijfsnaam (of naam van de vereniging of stichting).
    pub bedrijf: Option<String>,
    /// Contactpersoon.
    pub contactpersoon: Option<String>,
    /// Aanhef van de contactpersoon: man, vrouw, afdeling of geen.
    pub geslacht: Option<Geslacht>,
    /// Straat en huisnummer.
    pub adres: Option<String>,
    /// Postcode; een Nederlandse wordt geschreven als 1234 AB.
    pub postcode: Option<String>,
    pub plaats: Option<String>,
    /// Telefoonnummer.
    pub telefoon: Option<String>,
    /// E-mailadres van de administratie.
    pub email: Option<String>,
    /// Website.
    pub website: Option<String>,
    /// KvK-nummer, alleen cijfers.
    pub kvk: Option<String>,
    /// Btw-nummer (OB-nummer), zoals NL123456789B01.
    pub ob_nummer: Option<String>,
    /// Btw-identificatienummer van een eenmanszaak, zoals NL000099998B57. Leeg voor een onderneming die geen eenmanszaak is.
    pub btw_id: Option<String>,
    /// IBAN van het bedrijf. Niet het rekeningnummer voor de automatische incasso van e-Boekhouden: dat kan hier niet.
    pub iban: Option<String>,
    /// BIC bij het IBAN. Zonder BIC zoekt EB MCP bij een nieuw IBAN de BIC op, zoals het scherm van e-Boekhouden doet.
    pub bic: Option<String>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

impl BedrijfsgegevensWijzigenArgs {
    /// Each named field as (the form's key, the value as given).
    fn given(&self) -> Vec<(&'static str, String)> {
        let texts: [(&'static str, &Option<String>); 13] = [
            ("bedrijf", &self.bedrijf),
            ("naam", &self.contactpersoon),
            ("adres", &self.adres),
            ("postcode", &self.postcode),
            ("plaats", &self.plaats),
            ("telefoon", &self.telefoon),
            ("email", &self.email),
            ("website", &self.website),
            ("obNummer", &self.ob_nummer),
            ("btwId", &self.btw_id),
            ("kvkNummer", &self.kvk),
            ("iban", &self.iban),
            ("bic", &self.bic),
        ];
        let mut out: Vec<(&'static str, String)> = texts
            .into_iter()
            .filter_map(|(key, value)| value.clone().map(|v| (key, v)))
            .collect();
        if let Some(geslacht) = self.geslacht {
            out.push(("geslacht", geslacht.code().to_owned()));
        }
        out
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct FactuuradresWijzigenArgs {
    /// Bedrijfsnaam op de facturen van e-Boekhouden.
    pub bedrijf: Option<String>,
    /// Contactpersoon.
    pub contactpersoon: Option<String>,
    /// Aanhef van de contactpersoon: man, vrouw, afdeling of geen.
    pub geslacht: Option<Geslacht>,
    /// Straat en huisnummer.
    pub adres: Option<String>,
    /// Postcode; een Nederlandse wordt geschreven als 1234 AB.
    pub postcode: Option<String>,
    pub plaats: Option<String>,
    pub land: Option<String>,
    /// Telefoonnummer.
    pub telefoon: Option<String>,
    /// Mobiel nummer.
    pub mobiel: Option<String>,
    /// E-mailadres waar e-Boekhouden zijn facturen heen stuurt.
    pub email: Option<String>,
    /// Faxnummer.
    pub fax: Option<String>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

impl FactuuradresWijzigenArgs {
    /// Each named field as (the key it is saved under, the value as given).
    fn given(&self) -> Vec<(&'static str, String)> {
        let texts: [(&'static str, &Option<String>); 10] = [
            ("bedrijf", &self.bedrijf),
            ("contactpersoon", &self.contactpersoon),
            ("adres", &self.adres),
            ("postcode", &self.postcode),
            ("plaats", &self.plaats),
            ("land", &self.land),
            ("telefoonnummer", &self.telefoon),
            ("mobielnummer", &self.mobiel),
            ("emailFactuur", &self.email),
            ("faxnummer", &self.fax),
        ];
        let mut out: Vec<(&'static str, String)> = texts
            .into_iter()
            .filter_map(|(key, value)| value.clone().map(|v| (key, v)))
            .collect();
        if let Some(geslacht) = self.geslacht {
            out.push(("geslacht", geslacht.code().to_owned()));
        }
        out
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct EboekhoudenFacturenArgs {
    /// Alleen de facturen die nog open staan (standaard alle recente facturen).
    pub openstaand: Option<bool>,
    /// Aantal rijen, 1 tot 200 (standaard 50).
    pub limit: Option<u32>,
    /// Vanaf welke rij (standaard 0).
    pub offset: Option<u32>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

// ---------------------------------------------------------------- checks

/// A value as text, for comparing and showing: a string as it is, nothing as empty.
fn text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Spaces and dots out, as the form does on leaving a KvK, OB or btw-id field.
fn without_spaces_and_dots(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '.')
        .collect()
}

/// The web app's `formatPostalCode`: upper case without white space, and a Dutch postcode with one space.
fn postcode(value: &str) -> String {
    let compact: String = value
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_uppercase();
    let b = compact.as_bytes();
    let dutch = b.len() == 6
        && (b'1'..=b'9').contains(&b[0])
        && b[1..4].iter().all(u8::is_ascii_digit)
        && b[4..].iter().all(u8::is_ascii_uppercase);
    if dutch {
        format!("{} {}", &compact[..4], &compact[4..])
    } else {
        compact
    }
}

/// The remainder modulo 97 of a text read as a number, letters A to Z counting as 10 to 35 (`validateMod97`).
fn mod97(value: &str) -> Option<u32> {
    let mut rest = 0u32;
    for c in value.chars() {
        let n = match c {
            'A'..='Z' => c as u32 - 55,
            '0'..='9' => c as u32 - 48,
            _ => return None,
        };
        for digit in n.to_string().bytes() {
            rest = (rest * 10 + u32::from(digit - b'0')) % 97;
        }
    }
    Some(rest)
}

/// The web app's `validateObNummer`: NL optional, nine digits, B, two digits, and the eleven test.
fn ob_nummer_ok(value: &str) -> bool {
    let d = value.strip_prefix("NL").unwrap_or(value);
    let b = d.as_bytes();
    if b.len() != 12
        || !b[..9].iter().all(u8::is_ascii_digit)
        || &d[..3] == "000"
        || b[9] != b'B'
        || !b[10..].iter().all(u8::is_ascii_digit)
        || &d[10..] == "00"
    {
        return false;
    }
    let sum: u32 = (0..8)
        .map(|i| u32::from(b[i] - b'0') * (9 - i as u32))
        .sum();
    sum % 11 == u32::from(b[8] - b'0')
}

/// The web app's btw-id check: fourteen characters starting with NL, the whole of it 1 modulo 97.
fn btw_id_ok(value: &str) -> bool {
    value.len() == 14 && value.starts_with("NL") && mod97(value) == Some(1)
}

/// An IBAN by ISO 13616: country, check digits, the right length for a Dutch one, and 1 modulo 97.
fn iban_ok(value: &str) -> bool {
    let b = value.as_bytes();
    if !(15..=34).contains(&b.len())
        || !b[..2].iter().all(u8::is_ascii_uppercase)
        || !b[2..4].iter().all(u8::is_ascii_digit)
        || !b.iter().all(u8::is_ascii_alphanumeric)
        || (value.starts_with("NL") && b.len() != 18)
    {
        return false;
    }
    mod97(&format!("{}{}", &value[4..], &value[..4])) == Some(1)
}

/// A new value cleaned as the form cleans it, and checked by the form's rules. Refused with a Dutch sentence.
fn clean(field: &Field, raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    let label = field.label;
    let value = match field.check {
        Check::Text | Check::Geslacht => raw.to_owned(),
        Check::Email if raw.is_empty() => String::new(),
        Check::Email => email(raw, label)?,
        Check::Postcode => postcode(raw),
        Check::Kvk => {
            let value = without_spaces_and_dots(raw);
            if !value.chars().all(|c| c.is_ascii_digit()) {
                return Err(format!("Het {label} mag alleen cijfers bevatten"));
            }
            value
        }
        Check::ObNummer | Check::BtwId => {
            let value = without_spaces_and_dots(raw).to_ascii_uppercase();
            if !value.chars().all(|c| c.is_ascii_alphanumeric()) {
                return Err(format!(
                    "Het {label} mag alleen letters en cijfers bevatten"
                ));
            }
            if !value.is_empty() && field.check == Check::ObNummer && !ob_nummer_ok(&value) {
                return Err(
                    "Het btw-nummer (OB-nummer) klopt niet. Een OB-nummer ziet er zo uit: NL123456789B01."
                        .into(),
                );
            }
            if !value.is_empty() && field.check == Check::BtwId && !btw_id_ok(&value) {
                return Err(
                    "Het btw-id klopt niet. Een btw-id ziet er zo uit: NL000099998B57.".into(),
                );
            }
            value
        }
        Check::Iban => {
            let value: String = raw
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
                .to_ascii_uppercase();
            if !value.is_empty() && !iban_ok(&value) {
                return Err(format!("{value} is geen geldig IBAN"));
            }
            value
        }
        Check::Bic => {
            let value = raw.to_ascii_uppercase();
            if !value.is_empty()
                && (!matches!(value.len(), 8 | 11)
                    || !value.chars().all(|c| c.is_ascii_alphanumeric()))
            {
                return Err(format!(
                    "{value} is geen geldige BIC: die heeft 8 of 11 letters en cijfers"
                ));
            }
            value
        }
    };
    if field.required && value.is_empty() {
        return Err(format!("{label} mag niet leeg zijn"));
    }
    if value.chars().count() > field.max {
        return Err(format!("{label} mag hoogstens {} tekens hebben", field.max));
    }
    Ok(value)
}

/// A value of a field for the approval window.
fn shown(field: &Field, value: Option<&Value>) -> String {
    let Some(value) = value else {
        return "onbekend".into();
    };
    let value = text(value);
    if field.check == Check::Geslacht {
        geslacht_naam(&value)
    } else if value.trim().is_empty() {
        "(leeg)".into()
    } else {
        value
    }
}

/// What goes back to e-Boekhouden: every field of the form as it was read, with only the named ones changed,
/// and the lines that show the user each change.
#[derive(Debug)]
struct Merged {
    body: serde_json::Map<String, Value>,
    lines: Vec<approvals::Line>,
    changed: Vec<&'static str>,
}

/// Builds the body a form saves from what was read, and applies the named changes. A field the caller does not
/// name is sent back exactly as read (a field the model does not have is left out, as the web app leaves it out,
/// or gets the form's own default). Fields of the model that are not on the form never go back.
fn merge(
    read: &Value,
    fields: &[Field],
    keep: &[&str],
    given: Vec<(&'static str, String)>,
) -> Result<Merged, String> {
    let empty = serde_json::Map::new();
    let model = match read {
        Value::Object(model) => model,
        Value::Null => &empty,
        _ => return Err("e-Boekhouden gaf deze gegevens in een onbekende vorm terug".into()),
    };
    let mut body = serde_json::Map::new();
    for field in fields {
        match field.read.iter().find_map(|key| model.get(*key)) {
            Some(value) => {
                body.insert(field.key.into(), value.clone());
            }
            None => {
                if let Some(default) = field.default {
                    body.insert(field.key.into(), json!(default));
                }
            }
        }
    }
    for key in keep {
        if let Some(value) = model.get(*key) {
            body.insert((*key).into(), value.clone());
        }
    }
    let mut lines = Vec::new();
    let mut changed = Vec::new();
    for (key, raw) in given {
        let Some(field) = fields.iter().find(|f| f.key == key) else {
            return Err(format!("EB MCP kent het veld {key} niet"));
        };
        let value = clean(field, &raw)?;
        let before = body.get(key);
        if before.map(text).unwrap_or_default() == value {
            continue;
        }
        lines.push(line(&format!("{} was", field.label), shown(field, before)));
        lines.push(line(
            &format!("{} wordt", field.label),
            shown(field, Some(&json!(value))),
        ));
        body.insert(key.into(), json!(value));
        changed.push(key);
    }
    if changed.is_empty() {
        return Err(
            "Er is niets gewijzigd: noem wat er anders moet. Wat je noemde staat al zo in e-Boekhouden."
                .into(),
        );
    }
    for field in fields.iter().filter(|f| f.required) {
        if body
            .get(field.key)
            .map(text)
            .unwrap_or_default()
            .trim()
            .is_empty()
        {
            return Err(format!(
                "{} is verplicht op dit scherm van e-Boekhouden en is nu leeg; geef het mee",
                field.label
            ));
        }
    }
    Ok(Merged {
        body,
        lines,
        changed,
    })
}

/// The answer of a save that e-Boekhouden's screen counts as done only when it is something (`true`).
fn saved(answer: Value) -> Result<Value, Halt> {
    match &answer {
        Value::Null | Value::Bool(false) => Err(Halt::Error(
            "e-Boekhouden meldt dat het opslaan niet gelukt is; er is niets gewijzigd".into(),
        )),
        Value::String(s) if s.is_empty() => Err(Halt::Error(
            "e-Boekhouden meldt dat het opslaan niet gelukt is; er is niets gewijzigd".into(),
        )),
        _ => Ok(answer),
    }
}

impl EbMcp {
    /// One change with a query on e-Boekhouden, after the approval (`change` sends none).
    async fn change_with_query(
        &self,
        run: &Run,
        method: &'static str,
        path: &str,
        query: &[(String, String)],
        body: &Value,
    ) -> Result<Value, Halt> {
        let call = self
            .state
            .api_send(
                run.account.id,
                method,
                path,
                query,
                Some(body),
                Some(run.id),
                &run.client,
            )
            .await;
        if call.ok() {
            return Ok(call.json.unwrap_or(Value::Null));
        }
        Err(Halt::Error(call.error.unwrap_or_else(|| {
            format!("e-Boekhouden gaf {} op {path}", call.status)
        })))
    }
}

#[tool_router(router = mijn_gegevens_router, vis = "pub(super)")]
impl EbMcp {
    #[tool(
        description = "De bedrijfsgegevens van de administratie, zoals ze onder Beheer > Mijn gegevens staan: bedrijfsnaam, contactpersoon, adres, postcode, plaats, telefoon, e-mail, website, KvK-nummer, btw-nummer (OB-nummer), btw-id, IBAN en BIC, en daarnaast wat e-Boekhouden verder van de administratie bijhoudt (soort, diensten, betaalwijze).",
        annotations(title = "Bedrijfsgegevens", read_only_hint = true)
    )]
    async fn bedrijfsgegevens(
        &self,
        Parameters(args): Parameters<AccountArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        self.simple(
            &context,
            "bedrijfsgegevens",
            &args,
            args.account.as_deref(),
            MIJN_GEGEVENS,
            Ok(adm()),
        )
        .await
    }

    #[tool(
        description = "De bedrijfsgegevens van de administratie wijzigen (Beheer > Mijn gegevens): bedrijfsnaam, contactpersoon, aanhef, adres, postcode, plaats, telefoon, e-mail, website, KvK-nummer, btw-nummer (OB-nummer), btw-id, IBAN of BIC. Alleen de velden die je noemt veranderen; al het andere gaat terug zoals het nu in e-Boekhouden staat, een leeg veld maakt het leeg. Bij een nieuw IBAN zonder BIC zoekt EB MCP de BIC op. Het rekeningnummer voor de automatische incasso van e-Boekhouden, de licentie en opzeggen kunnen hier niet. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet. De gebruiker moet dit altijd goedkeuren in EB MCP en ziet daar per veld de oude en de nieuwe waarde. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Bedrijfsgegevens wijzigen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true
        )
    )]
    async fn bedrijfsgegevens_wijzigen(
        &self,
        Parameters(args): Parameters<BedrijfsgegevensWijzigenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "bedrijfsgegevens_wijzigen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Bedrijfsgegevens gewijzigd; data.bedrijfsgegevens is wat e-Boekhouden nu heeft",
            move |this, run, args| async move {
                let given = args.given();
                let bic_given = given.iter().any(|(key, _)| *key == "bic");
                // Bad input is refused before anything is read.
                for (key, raw) in &given {
                    if let Some(field) = BEDRIJF.iter().find(|f| f.key == *key) {
                        clean(field, raw)?;
                    }
                }
                let read = this.fetch(&run, MIJN_GEGEVENS, adm()).await?;
                // Never save a form built from nothing: every field would go out empty.
                if !read.is_object() {
                    return Err(Halt::Error(
                        "e-Boekhouden gaf de bedrijfsgegevens niet terug; er is niets gewijzigd"
                            .into(),
                    ));
                }
                let naam = read
                    .get("bedrijf")
                    .map(text)
                    .unwrap_or_default();
                let mut merged = merge(&read, BEDRIJF, BEDRIJF_KEEP, given)?;
                // As e-Boekhouden's screen does when the IBAN changes: it looks up the BIC of the new one.
                if merged.changed.contains(&"iban") && !bic_given {
                    let iban = merged.body.get("iban").map(text).unwrap_or_default();
                    let before = merged.body.get("bic").cloned();
                    if !iban.is_empty() {
                        let found = this
                            .fetch(&run, "bic", vec![("iban".into(), iban)])
                            .await
                            .ok()
                            .and_then(|bic| bic.as_str().map(|b| b.trim().to_ascii_uppercase()))
                            .filter(|bic| !bic.is_empty());
                        match found {
                            Some(bic) if Some(bic.as_str()) != before.as_ref().and_then(Value::as_str) => {
                                let bic_field = BEDRIJF.iter().find(|f| f.key == "bic");
                                if let Some(bic_field) = bic_field {
                                    merged.lines.push(line("BIC was", shown(bic_field, before.as_ref())));
                                    merged.lines.push(line("BIC wordt", format!("{bic} (opgezocht bij het nieuwe IBAN)")));
                                }
                                merged.body.insert("bic".into(), json!(bic));
                            }
                            Some(_) => {}
                            None => merged.lines.push(line(
                                "BIC",
                                format!(
                                    "e-Boekhouden vond geen BIC bij het nieuwe IBAN; de BIC blijft {}",
                                    before.as_ref().map(text).filter(|b| !b.is_empty()).unwrap_or_else(|| "leeg".into())
                                ),
                            )),
                        }
                    }
                }
                let mut lines = vec![
                    line("Account", &run.account.label),
                    line(
                        "Scherm",
                        "Beheer > Mijn gegevens: de bedrijfsgegevens van deze administratie in e-Boekhouden",
                    ),
                ];
                lines.append(&mut merged.lines);
                lines.push(line(
                    "Blijft gelijk",
                    "alle andere velden gaan terug zoals ze nu in e-Boekhouden staan",
                ));
                let built = Built {
                    body: Value::Object(merged.body),
                    title: format!("Bedrijfsgegevens wijzigen: {}", known(naam)),
                    lines,
                };
                this.approve(&run, TOOL, &built, true).await?;
                let answer = saved(
                    this.change_with_query(&run, "PUT", MIJN_GEGEVENS, &adm(), &built.body)
                        .await?,
                )?;
                let now = this.fetch(&run, MIJN_GEGEVENS, adm()).await.ok();
                Ok(json!({"opgeslagen": answer, "bedrijfsgegevens": now}))
            },
        )
        .await
    }

    #[tool(
        description = "Het factuuradres: waar e-Boekhouden zijn eigen facturen voor het abonnement heen stuurt (Beheer > Mijn gegevens > Factuuradres), met bedrijfsnaam, contactpersoon, adres, postcode, plaats, land, telefoon, mobiel, fax en het e-mailadres voor facturen.",
        annotations(title = "Factuuradres", read_only_hint = true)
    )]
    async fn factuuradres(
        &self,
        Parameters(args): Parameters<AccountArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        self.simple(
            &context,
            "factuuradres",
            &args,
            args.account.as_deref(),
            FACTUURADRES,
            Ok(Vec::new()),
        )
        .await
    }

    #[tool(
        description = "Het factuuradres wijzigen: waar e-Boekhouden zijn eigen facturen voor het abonnement heen stuurt (Beheer > Mijn gegevens > Factuuradres). Alleen de velden die je noemt veranderen; al het andere gaat terug zoals het nu in e-Boekhouden staat, een leeg veld maakt het leeg. email is het adres waar die facturen per e-mail heen gaan. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet. De gebruiker moet dit altijd goedkeuren in EB MCP en ziet daar per veld de oude en de nieuwe waarde. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Factuuradres wijzigen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true
        )
    )]
    async fn factuuradres_wijzigen(
        &self,
        Parameters(args): Parameters<FactuuradresWijzigenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "factuuradres_wijzigen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Factuuradres gewijzigd; data.factuuradres is wat e-Boekhouden nu heeft",
            move |this, run, args| async move {
                let given = args.given();
                for (key, raw) in &given {
                    if let Some(field) = FACTUUR.iter().find(|f| f.key == *key) {
                        clean(field, raw)?;
                    }
                }
                let read = this.fetch(&run, FACTUURADRES, Vec::new()).await?;
                let mut merged = merge(&read, FACTUUR, &[], given)?;
                let mut lines = vec![
                    line("Account", &run.account.label),
                    line(
                        "Scherm",
                        "Beheer > Mijn gegevens > Factuuradres: waar e-Boekhouden zijn eigen facturen voor het abonnement heen stuurt",
                    ),
                ];
                lines.append(&mut merged.lines);
                if merged.changed.contains(&"emailFactuur") {
                    lines.push(line(
                        "Gevolg",
                        "e-Boekhouden mailt zijn facturen voortaan naar het nieuwe e-mailadres",
                    ));
                }
                lines.push(line(
                    "Blijft gelijk",
                    "alle andere velden gaan terug zoals ze nu in e-Boekhouden staan",
                ));
                let built = Built {
                    body: Value::Object(merged.body),
                    title: "Factuuradres wijzigen".into(),
                    lines,
                };
                this.approve(&run, TOOL, &built, true).await?;
                let answer = this
                    .change(&run, "PUT", FACTUURADRES, Some(&built.body))
                    .await?;
                let now = this.fetch(&run, FACTUURADRES, Vec::new()).await.ok();
                Ok(json!({"opgeslagen": answer, "factuuradres": now}))
            },
        )
        .await
    }

    #[tool(
        description = "De facturen die e-Boekhouden zelf aan de gebruiker stuurt, voor het abonnement: id, datum, nummer, periode, bedrag (totaal) en wat er voldaan is. Met openstaand alleen de facturen die nog betaald moeten worden; een betaling die net loopt kan daar nog tussen staan. Haal er een op als pdf met eboekhouden_factuur_pdf.",
        annotations(title = "Facturen van e-Boekhouden", read_only_hint = true)
    )]
    async fn eboekhouden_facturen(
        &self,
        Parameters(args): Parameters<EboekhoudenFacturenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let path = if args.openstaand.unwrap_or(false) {
            OPENSTAANDE_FACTUREN
        } else {
            RECENTE_FACTUREN
        };
        let run = match self.begin(
            &context,
            "eboekhouden_facturen",
            &args,
            args.account.as_deref(),
        ) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let query = match grid(args.limit, args.offset) {
            Ok(query) => query,
            Err(message) => return Ok(self.refuse(run, message)),
        };
        let result = self.fetch(&run, path, query).await.map(named_rows);
        Ok(self.end(run, path, result))
    }

    #[tool(
        description = "Een factuur die e-Boekhouden zelf aan de gebruiker stuurde (voor het abonnement) als pdf ophalen, op id (uit eboekhouden_facturen). De pdf wordt op deze computer opgeslagen, standaard in de map Downloads; het antwoord geeft het volledige pad en de pdf zelf als bijlage, zodat je hem kunt lezen en de gebruiker hem kan openen. Bestaande bestanden worden niet overschreven.",
        annotations(title = "Factuur van e-Boekhouden als pdf", read_only_hint = true)
    )]
    async fn eboekhouden_factuur_pdf(
        &self,
        Parameters(args): Parameters<PdfArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let run = match self.begin(
            &context,
            "eboekhouden_factuur_pdf",
            &args,
            args.account.as_deref(),
        ) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        // The path as the web app writes it.
        let path = format!("SkillSource/factuur/recent/pdf/{}", args.id);
        let result = self
            .save_pdf(
                &run,
                &path,
                args.map.as_deref(),
                &format!("e-boekhouden-factuur-{}.pdf", args.id),
            )
            .await;
        Ok(self.end_pdf(run, &path, result))
    }
}

#[cfg(test)]
mod tests;
