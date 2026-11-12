//! The beginbalans (opening balance): reading it, and entering or changing its lines.
//!
//! From e-Boekhouden's bundle (chunk-I0e1VsKK.js, `app-beginbalans-form` and its service):
//! - `GET beginbalans/beginbalans` -> {datum, oudsteMutatieDatum, regels: [{gbId, code, omschrijving, soort, activa,
//!   passiva}], ...}; `GET beginbalans/saldobeginbalansopenpost/2` (debiteuren) and `/1` (crediteuren): what the open
//!   posten on the beginbalans add up to.
//! - Opslaan: `PUT beginbalans/beginbalans` with the whole object it read, `datum` from the date picker
//!   ({day, month, year}) and every regel's activa and passiva as numbers (an empty one becomes 0).
//! - The date must lie on or before the oldest mutatie (`oudsteMutatieDatum`). A rekening that is not on the list
//!   yet is added as {gbId, code, omschrijving, soort} with soort its category; the form offers BAL, FIN, CRED and
//!   DEB for that. The page says the activa and passiva columns must always be exactly equal.

use super::*;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct BeginbalansRegelArg {
    /// Code van de grootboekrekening, bijvoorbeeld 1010.
    pub rekening: String,
    /// Nieuw bedrag in de kolom activa (debet). Weglaten: blijft zoals het is.
    pub activa: Option<f64>,
    /// Nieuw bedrag in de kolom passiva (credit). Weglaten: blijft zoals het is.
    pub passiva: Option<f64>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct BeginbalansWijzigenArgs {
    /// Datum van de beginbalans, JJJJ-MM-DD. Moet op of voor de oudste mutatie liggen. Weglaten: blijft zoals hij is.
    pub datum: Option<String>,
    /// De regels die veranderen. Een rekening die nog niet op de beginbalans staat (een balans-, liquide-middelen-, debiteuren- of crediteurenrekening) komt erbij.
    pub regels: Option<Vec<BeginbalansRegelArg>>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// The categories of the rekeningen the form lets you add to the beginbalans.
const NIEUWE_SOORTEN: [&str; 4] = ["BAL", "FIN", "CRED", "DEB"];

/// What the open posten on the beginbalans add up to, as e-Boekhouden gives it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct OpenPosten {
    debiteuren: Option<f64>,
    crediteuren: Option<f64>,
}

/// An amount of a regel (`activa` or `passiva`): a number or a numeric text; nothing counts as 0, as the form
/// does (`e.activa || 0`). Anything else is refused: the whole beginbalans goes back, so a guess would be sent.
fn num(regel: &Value, key: &str) -> Result<f64, String> {
    optional(regel.get(key), read_f64)
        .map(|v| v.unwrap_or(0.0))
        .map_err(|()| {
            onleesbaar(
                "de beginbalans",
                &format!("{key} van rekening {}", regel_label(regel)),
            )
        })
}

fn cents(amount: f64) -> i64 {
    (amount * 100.0).round() as i64
}

/// The two columns and the debiteuren and crediteuren saldo of a list of regels, as the page computes them.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Totalen {
    activa: f64,
    passiva: f64,
    debiteuren: f64,
    crediteuren: f64,
}

fn totalen(regels: &[Value]) -> Result<Totalen, String> {
    let mut t = Totalen {
        activa: 0.0,
        passiva: 0.0,
        debiteuren: 0.0,
        crediteuren: 0.0,
    };
    for regel in regels {
        let (activa, passiva) = (num(regel, "activa")?, num(regel, "passiva")?);
        t.activa += activa;
        t.passiva += passiva;
        match regel.get("soort").and_then(Value::as_str) {
            Some("DEB") => t.debiteuren += activa - passiva,
            Some("CRED") => t.crediteuren += passiva - activa,
            _ => {}
        }
    }
    t.activa = round2(t.activa);
    t.passiva = round2(t.passiva);
    t.debiteuren = round2(t.debiteuren);
    t.crediteuren = round2(t.crediteuren);
    Ok(t)
}

fn regel_label(regel: &Value) -> String {
    let text = |key: &str| {
        regel
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned()
    };
    format!("{} {}", text("code"), text("omschrijving"))
        .trim()
        .to_owned()
}

/// The approval lines that warn when the debiteuren or crediteuren saldo differs from the open posten entered.
fn open_posten_lines(t: &Totalen, open: OpenPosten) -> Vec<approvals::Line> {
    let mut lines = Vec::new();
    for (wat, saldo, posten) in [
        ("debiteuren", t.debiteuren, open.debiteuren),
        ("crediteuren", t.crediteuren, open.crediteuren),
    ] {
        if let Some(posten) = posten
            && cents(saldo) != cents(posten)
        {
            lines.push(line(
                "Let op",
                format!(
                    "het saldo van de {wat}rekening(en) wordt {}, maar de open posten {wat} op de beginbalans tellen op tot {}; vul die open posten aan in e-Boekhouden onder Beheer > Beginbalans",
                    write::euro(saldo),
                    write::euro(posten)
                ),
            ));
        }
    }
    lines
}

fn beginbalans_gewijzigd(
    args: &BeginbalansWijzigenArgs,
    now: &Value,
    nieuw: &[(Keuze, &'static str)],
    open: OpenPosten,
) -> Result<Built, String> {
    let bad = || "e-Boekhouden gaf de beginbalans in een onbekende vorm terug".to_owned();
    let mut body = now.clone();
    let object = body.as_object_mut().ok_or_else(bad)?;
    let before: Vec<Value> = object
        .get("regels")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(bad)?;
    let mut regels = before.clone();
    // As the form does: every regel goes out with both amounts, an empty one as 0.
    for regel in regels.iter_mut() {
        let (activa, passiva) = (num(regel, "activa")?, num(regel, "passiva")?);
        let regel = regel.as_object_mut().ok_or_else(bad)?;
        regel.insert("activa".into(), json!(activa));
        regel.insert("passiva".into(), json!(passiva));
    }

    let oud = object.get("datum").and_then(date_of);
    let datum = match args.datum.as_deref().map(parse_date).transpose()? {
        Some(datum) => datum,
        None => oud.ok_or("Deze beginbalans heeft nog geen datum: geef datum mee")?,
    };
    if let Some(oudste) = object.get("oudsteMutatieDatum").and_then(date_of)
        && datum > oudste
    {
        return Err(format!(
            "De datum van de beginbalans ({}) ligt na de oudste mutatie ({}); hij moet daarop of daarvoor liggen",
            dutch(datum),
            dutch(oudste)
        ));
    }
    object.insert("datum".into(), date_object(datum));

    let mut seen: Vec<String> = Vec::new();
    for arg in args.regels.as_deref().unwrap_or_default() {
        let code = arg.rekening.trim().to_owned();
        if code.is_empty() {
            return Err("Geef bij elke regel de code van de rekening".into());
        }
        if seen.contains(&code) {
            return Err(format!("Rekening {code} staat twee keer in regels"));
        }
        seen.push(code.clone());
        if arg.activa.is_none() && arg.passiva.is_none() {
            return Err(format!(
                "Geef bij rekening {code} het bedrag voor activa, passiva of allebei"
            ));
        }
        let activa = arg
            .activa
            .map(|a| bedrag(a, &format!("activa van {code}")))
            .transpose()?;
        let passiva = arg
            .passiva
            .map(|a| bedrag(a, &format!("passiva van {code}")))
            .transpose()?;
        let at = |r: &Value| r.get("code").and_then(Value::as_str).map(str::trim) == Some(&code);
        let index = match regels.iter().position(at) {
            Some(index) => index,
            None => {
                let (keuze, soort) = nieuw
                    .iter()
                    .find(|(k, _)| k.code == code)
                    .ok_or_else(|| {
                        format!(
                            "Rekening {code} staat niet op de beginbalans en is geen actieve balans-, liquide-middelen-, debiteuren- of crediteurenrekening"
                        )
                    })?;
                regels.push(json!({"gbId": keuze.id, "code": keuze.code, "omschrijving": keuze.naam, "soort": soort, "activa": 0.0, "passiva": 0.0}));
                regels.len() - 1
            }
        };
        if let Some(activa) = activa {
            regels[index]["activa"] = json!(activa);
        }
        if let Some(passiva) = passiva {
            regels[index]["passiva"] = json!(passiva);
        }
    }

    let mut lines = Vec::new();
    if oud != Some(datum) {
        lines.push(line(
            "Datum was",
            oud.map(dutch).unwrap_or_else(|| "(leeg)".into()),
        ));
        lines.push(line("Datum wordt", dutch(datum)));
    }
    for regel in &regels {
        let id = regel.get("gbId");
        let old = before.iter().find(|r| r.get("gbId") == id);
        let (old_a, old_p) = match old {
            Some(r) => (num(r, "activa")?, num(r, "passiva")?),
            None => (0.0, 0.0),
        };
        let (new_a, new_p) = (num(regel, "activa")?, num(regel, "passiva")?);
        if cents(old_a) == cents(new_a) && cents(old_p) == cents(new_p) {
            continue;
        }
        let kolom = |naam: &str, old: f64, new: f64| {
            if cents(old) == cents(new) {
                format!("{naam} blijft {}", write::euro(new))
            } else {
                format!(
                    "{naam} was {}, wordt {}",
                    write::euro(old),
                    write::euro(new)
                )
            }
        };
        let label = if old.is_some() {
            regel_label(regel)
        } else {
            format!("{} (nieuw op de beginbalans)", regel_label(regel))
        };
        lines.push(line(
            &label,
            format!(
                "{}; {}",
                kolom("activa", old_a, new_a),
                kolom("passiva", old_p, new_p)
            ),
        ));
    }
    if lines.is_empty() {
        return Err(
            "Er is niets gewijzigd: geef een andere datum of regels met nieuwe bedragen".into(),
        );
    }

    let (was, wordt) = (totalen(&before)?, totalen(&regels)?);
    if cents(wordt.activa) != cents(wordt.passiva) {
        return Err(format!(
            "Activa ({}) en passiva ({}) zijn dan niet gelijk; het verschil is {}. Debet en credit van de beginbalans moeten precies gelijk zijn: pas de regels zo aan dat beide kanten even groot worden",
            write::euro(wordt.activa),
            write::euro(wordt.passiva),
            write::euro((wordt.activa - wordt.passiva).abs())
        ));
    }
    lines.push(line(
        "Totaal activa",
        format!(
            "was {}, wordt {}",
            write::euro(was.activa),
            write::euro(wordt.activa)
        ),
    ));
    lines.push(line(
        "Totaal passiva",
        format!(
            "was {}, wordt {}",
            write::euro(was.passiva),
            write::euro(wordt.passiva)
        ),
    ));
    lines.push(line(
        "Debet en credit",
        format!("gelijk: allebei {}", write::euro(wordt.activa)),
    ));
    lines.extend(open_posten_lines(&wordt, open));
    lines.push(line(
        "Gevolg",
        "de beginbalans bepaalt de beginstand van elke balansrekening, en daarmee de hele balans van elke periode erna",
    ));
    object.insert("regels".into(), Value::Array(regels));
    Ok(Built {
        body,
        title: "Beginbalans wijzigen".into(),
        lines,
    })
}

impl EbMcp {
    /// What the open posten on the beginbalans add up to (debiteuren 2, crediteuren 1, as the page asks).
    async fn open_posten_saldi(&self, run: &Run) -> Result<OpenPosten, String> {
        let saldo = |value: Value| {
            value
                .as_f64()
                .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
        };
        let debiteuren = self
            .fetch(run, "beginbalans/saldobeginbalansopenpost/2", Vec::new())
            .await?;
        let crediteuren = self
            .fetch(run, "beginbalans/saldobeginbalansopenpost/1", Vec::new())
            .await?;
        Ok(OpenPosten {
            debiteuren: saldo(debiteuren),
            crediteuren: saldo(crediteuren),
        })
    }
}

#[tool_router(router = beginbalans_router, vis = "pub(super)")]
impl EbMcp {
    #[tool(
        description = "De beginbalans: de datum, per grootboekrekening (gbId, code, omschrijving, soort) het bedrag in de kolom activa (debet) en passiva (credit), de totalen van beide kolommen en of ze gelijk zijn, en het saldo van de debiteuren- en crediteurenrekeningen naast wat de open posten op de beginbalans optellen.",
        annotations(title = "Beginbalans", read_only_hint = true)
    )]
    async fn beginbalans(
        &self,
        Parameters(args): Parameters<AccountArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const PATH: &str = "beginbalans/beginbalans";
        let run = match self.begin(&context, "beginbalans", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let result = async {
            let now = self.fetch(&run, PATH, Vec::new()).await?;
            let open = self.open_posten_saldi(&run).await?;
            let regels = now
                .get("regels")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let t = totalen(&regels)?;
            Ok(json!({
                "beginbalans": now,
                "totaal_activa": t.activa,
                "totaal_passiva": t.passiva,
                "in_evenwicht": cents(t.activa) == cents(t.passiva),
                "saldo_debiteurenrekeningen": t.debiteuren,
                "saldo_open_posten_debiteuren": open.debiteuren,
                "saldo_crediteurenrekeningen": t.crediteuren,
                "saldo_open_posten_crediteuren": open.crediteuren,
            }))
        }
        .await;
        Ok(self.end(run, PATH, result))
    }

    #[tool(
        description = "De beginbalans wijzigen: de datum en/of de bedragen in de kolommen activa (debet) en passiva (credit) per grootboekrekening, zoals Beheer > Beginbalans in e-Boekhouden. Noem alleen de rekeningen die veranderen; een rekening die er nog niet op staat komt erbij. Na de wijziging moeten de totalen van activa en passiva precies gelijk zijn, anders weigert EB MCP. De goedkeuring toont per rekening het oude en nieuwe bedrag en beide totalen. De beginbalans bepaalt de hele balans, dus de gebruiker moet dit altijd goedkeuren in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Beginbalans wijzigen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn beginbalans_wijzigen(
        &self,
        Parameters(args): Parameters<BeginbalansWijzigenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "beginbalans_wijzigen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Beginbalans gewijzigd",
            move |this, run, args| async move {
                let now = this
                    .fetch(&run, "beginbalans/beginbalans", Vec::new())
                    .await?;
                let open = this.open_posten_saldi(&run).await?;
                // The pick lists are read only when a rekening is not on the beginbalans yet.
                let on_it = |code: &str| {
                    now.get("regels")
                        .and_then(Value::as_array)
                        .is_some_and(|regels| {
                            regels.iter().any(|r| {
                                r.get("code").and_then(Value::as_str).map(str::trim)
                                    == Some(code.trim())
                            })
                        })
                };
                let mut nieuw: Vec<(Keuze, &'static str)> = Vec::new();
                if args
                    .regels
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .any(|r| !on_it(&r.rekening))
                {
                    for soort in NIEUWE_SOORTEN {
                        for keuze in this.rekeningen_van(&run, soort).await? {
                            nieuw.push((keuze, soort));
                        }
                    }
                }
                let built = beginbalans_gewijzigd(&args, &now, &nieuw, open)?;
                this.approve(&run, TOOL, &built, true).await?;
                this.change_done(&run, "PUT", "beginbalans/beginbalans", Some(&built.body))
                    .await
            },
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> Value {
        json!({
            "datum": {"day": 1, "month": 1, "year": 2026},
            "oudsteMutatieDatum": {"day": 3, "month": 1, "year": 2026},
            "regels": [
                {"gbId": 11, "code": "1010", "omschrijving": "Bank", "soort": "FIN", "activa": 1000, "passiva": null},
                {"gbId": 13, "code": "1300", "omschrijving": "Debiteuren", "soort": "DEB", "activa": 500, "passiva": 0},
                {"gbId": 20, "code": "0500", "omschrijving": "Eigen vermogen", "soort": "BAL", "activa": null, "passiva": 1500}
            ]
        })
    }

    fn args(
        datum: Option<&str>,
        regels: Vec<(&str, Option<f64>, Option<f64>)>,
    ) -> BeginbalansWijzigenArgs {
        BeginbalansWijzigenArgs {
            datum: datum.map(str::to_owned),
            regels: Some(
                regels
                    .into_iter()
                    .map(|(rekening, activa, passiva)| BeginbalansRegelArg {
                        rekening: rekening.into(),
                        activa,
                        passiva,
                    })
                    .collect(),
            ),
            account: None,
        }
    }

    fn nieuw() -> Vec<(Keuze, &'static str)> {
        vec![(
            Keuze {
                id: 17,
                code: "1700".into(),
                naam: "Crediteuren".into(),
            },
            "CRED",
        )]
    }

    fn open() -> OpenPosten {
        OpenPosten {
            debiteuren: Some(500.0),
            crediteuren: Some(0.0),
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
    fn changes_two_lines_and_sends_the_whole_beginbalans_back() {
        let built = beginbalans_gewijzigd(
            &args(
                None,
                vec![("1010", Some(1250.0), None), ("0500", None, Some(1750.0))],
            ),
            &now(),
            &[],
            open(),
        )
        .unwrap();
        assert_eq!(
            built.body,
            json!({
                "datum": {"day": 1, "month": 1, "year": 2026},
                "oudsteMutatieDatum": {"day": 3, "month": 1, "year": 2026},
                "regels": [
                    {"gbId": 11, "code": "1010", "omschrijving": "Bank", "soort": "FIN", "activa": 1250.0, "passiva": 0.0},
                    {"gbId": 13, "code": "1300", "omschrijving": "Debiteuren", "soort": "DEB", "activa": 500.0, "passiva": 0.0},
                    {"gbId": 20, "code": "0500", "omschrijving": "Eigen vermogen", "soort": "BAL", "activa": 0.0, "passiva": 1750.0}
                ]
            })
        );
        assert_eq!(built.title, "Beginbalans wijzigen");
        let lines = label_values(&built);
        assert!(lines.contains(&(
            "1010 Bank".into(),
            "activa was 1000,00, wordt 1250,00; passiva blijft 0,00".into()
        )));
        assert!(lines.contains(&(
            "0500 Eigen vermogen".into(),
            "activa blijft 0,00; passiva was 1500,00, wordt 1750,00".into()
        )));
        assert!(!lines.iter().any(|(l, _)| l.starts_with("1300")));
        assert!(lines.contains(&("Totaal activa".into(), "was 1500,00, wordt 1750,00".into())));
        assert!(lines.contains(&("Totaal passiva".into(), "was 1500,00, wordt 1750,00".into())));
        assert!(lines.contains(&("Debet en credit".into(), "gelijk: allebei 1750,00".into())));
        assert!(!lines.iter().any(|(l, _)| l == "Let op"));
        assert!(!lines.iter().any(|(l, _)| l.starts_with("Datum")));
    }

    #[test]
    fn refuses_a_beginbalans_that_does_not_balance() {
        let error = beginbalans_gewijzigd(
            &args(None, vec![("1010", Some(1250.0), None)]),
            &now(),
            &[],
            open(),
        )
        .unwrap_err();
        assert!(
            error.contains("1750,00") && error.contains("1500,00") && error.contains("250,00"),
            "{error}"
        );
    }

    #[test]
    fn adds_a_rekening_that_is_not_on_it_yet_and_warns_about_open_posten() {
        let built = beginbalans_gewijzigd(
            &args(
                Some("2025-12-31"),
                vec![("1700", None, Some(200.0)), ("1300", Some(700.0), None)],
            ),
            &now(),
            &nieuw(),
            open(),
        )
        .unwrap();
        assert_eq!(
            built.body["datum"],
            json!({"day": 31, "month": 12, "year": 2025})
        );
        assert_eq!(
            built.body["regels"][3],
            json!({"gbId": 17, "code": "1700", "omschrijving": "Crediteuren", "soort": "CRED", "activa": 0.0, "passiva": 200.0})
        );
        let lines = label_values(&built);
        assert!(lines.contains(&("Datum was".into(), "01-01-2026".into())));
        assert!(lines.contains(&("Datum wordt".into(), "31-12-2025".into())));
        assert!(lines.contains(&(
            "1700 Crediteuren (nieuw op de beginbalans)".into(),
            "activa blijft 0,00; passiva was 0,00, wordt 200,00".into()
        )));
        let warnings: Vec<&String> = lines
            .iter()
            .filter(|(l, _)| l == "Let op")
            .map(|(_, v)| v)
            .collect();
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(
            warnings[0].contains("debiteuren")
                && warnings[0].contains("700,00")
                && warnings[0].contains("500,00")
        );
        assert!(warnings[1].contains("crediteuren") && warnings[1].contains("200,00"));
    }

    #[test]
    fn refuses_bad_input_before_asking() {
        let now = now();
        // After the oldest mutatie.
        let error = beginbalans_gewijzigd(&args(Some("2026-01-04"), vec![]), &now, &[], open())
            .unwrap_err();
        assert!(error.contains("03-01-2026"), "{error}");
        // On the oldest mutatie is fine.
        assert!(
            beginbalans_gewijzigd(&args(Some("2026-01-03"), vec![]), &now, &[], open()).is_ok()
        );
        // Nothing to do.
        assert!(
            beginbalans_gewijzigd(&args(None, vec![]), &now, &[], open())
                .unwrap_err()
                .contains("niets gewijzigd")
        );
        // Unknown rekening, twice the same, no amount, a broken amount.
        assert!(
            beginbalans_gewijzigd(
                &args(None, vec![("4000", Some(1.0), None)]),
                &now,
                &nieuw(),
                open()
            )
            .unwrap_err()
            .contains("4000")
        );
        assert!(
            beginbalans_gewijzigd(
                &args(
                    None,
                    vec![("1010", Some(1.0), None), ("1010", Some(2.0), None)]
                ),
                &now,
                &[],
                open()
            )
            .unwrap_err()
            .contains("twee keer")
        );
        assert!(
            beginbalans_gewijzigd(&args(None, vec![("1010", None, None)]), &now, &[], open())
                .unwrap_err()
                .contains("activa, passiva")
        );
        assert!(
            beginbalans_gewijzigd(
                &args(None, vec![("1010", Some(f64::NAN), None)]),
                &now,
                &[],
                open()
            )
            .is_err()
        );
        // A beginbalans without a date needs one.
        let mut empty = now.clone();
        empty["datum"] = Value::Null;
        assert!(
            beginbalans_gewijzigd(
                &args(None, vec![("1010", Some(1000.0), None)]),
                &empty,
                &[],
                open()
            )
            .unwrap_err()
            .contains("geef datum")
        );
        assert!(beginbalans_gewijzigd(&args(None, vec![]), &json!([]), &[], open()).is_err());
    }

    #[test]
    fn adds_up_the_columns_as_the_page_does() {
        let regels = now()["regels"].as_array().cloned().unwrap();
        let t = totalen(&regels).unwrap();
        assert_eq!(
            t,
            Totalen {
                activa: 1500.0,
                passiva: 1500.0,
                debiteuren: 500.0,
                crediteuren: 0.0
            }
        );
        assert_eq!(num(&json!({"activa": "12.5"}), "activa").unwrap(), 12.5);
        assert_eq!(num(&json!({"activa": null}), "activa").unwrap(), 0.0);
        assert_eq!(num(&json!({}), "passiva").unwrap(), 0.0);
        assert!(totalen(&[json!({"code": "1010", "activa": "veel"})]).is_err());
        assert!(open_posten_lines(&t, OpenPosten::default()).is_empty());
    }

    #[test]
    fn a_line_it_cannot_read_stops_the_change() {
        let mut now = now();
        now["regels"][2]["passiva"] = json!("1.500,00");
        let error = beginbalans_gewijzigd(
            &args(None, vec![("1010", Some(1250.0), None)]),
            &now,
            &[],
            open(),
        )
        .unwrap_err();
        assert!(
            error.contains("onbekende vorm") && error.contains("0500 Eigen vermogen"),
            "{error}"
        );
    }
}
