//! The iDEAL link of a sent return, looked up and shown to the user: never opened, never paid, never filed.
//!
//! As the "Aangifte direct betalen" button of the list of sent returns (`app-aangifte-list`, `payNow`): a row
//! that carries a `payUrl` opens that link; otherwise the one pay request of this part
//! (`btwaangifte_betaling_starten`) makes e-Boekhouden start a payment at its payment provider and answer
//! with `startPaymentUrl`. The button shows when
//! `(iDeal || mustPayDiff || lastPayDate) && canPay && !mustPayDiff`; `lastPayStatus` says `Success` (paid with
//! iDEAL), `Betaald` (paid another way), `Open` (a payment runs) or `Te ontvangen` (money back).

use super::*;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct BetaallinkArgs {
    /// Id van een verstuurde aangifte (uit btwaangiftes).
    pub id: u64,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// What the AI client is told about the link.
const LET_OP: &str = "Toon deze link aan de gebruiker als een link die de gebruiker zelf opent. Open de link niet, haal hem niet op en volg hem niet: er wordt pas betaald als de gebruiker de link opent en in de eigen bank betaalt. De link kan verlopen; vraag dan een nieuwe op.";

/// A flag of a row as the screen's template reads it: true, a number other than 0, or a text that is not empty.
fn truthy(row: &Value, key: &str) -> bool {
    match row.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        _ => false,
    }
}

/// The kind of a sent return in plain words.
fn soort_naam(row: &Value) -> String {
    match text_of(row, "soort").as_str() {
        "OB" => "btw-aangifte".to_owned(),
        "SUP" => "suppletie".to_owned(),
        "" => "aangifte".to_owned(),
        other => other.to_owned(),
    }
}

/// Whether the screen would offer "Aangifte direct betalen" for this row, and if not, why, in plain Dutch.
fn betaalbaar(row: &Value) -> Result<(), String> {
    let wat = format!(
        "Deze {} ({})",
        soort_naam(row),
        known(text_of(row, "periode"))
    );
    match text_of(row, "lastPayStatus").as_str() {
        "Success" => {
            let op = text_of(row, "lastPayDate");
            return Err(if op.is_empty() {
                format!("{wat} is al via iDEAL betaald. Er is geen betaallink nodig.")
            } else {
                format!("{wat} is al via iDEAL betaald op {op}. Er is geen betaallink nodig.")
            });
        }
        "Betaald" => {
            return Err(format!(
                "{wat} staat in e-Boekhouden als op een andere manier betaald. Er is geen betaallink nodig."
            ));
        }
        "Te ontvangen" => {
            return Err(format!(
                "Over {} krijg je geld terug; er valt niets te betalen.",
                wat.trim_start_matches("Deze ")
            ));
        }
        _ => {}
    }
    let shown = (truthy(row, "iDeal") || truthy(row, "mustPayDiff") || truthy(row, "lastPayDate"))
        && truthy(row, "canPay")
        && !truthy(row, "mustPayDiff");
    if !shown {
        return Err(format!(
            "{wat} kan in e-Boekhouden niet met iDEAL betaald worden: de knop Aangifte direct betalen staat er niet bij. \
             Betaal hem zelf, bijvoorbeeld vanuit je bank met het betalingskenmerk."
        ));
    }
    Ok(())
}

/// A link e-Boekhouden gave: a web address, nothing else.
fn link(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    (text.starts_with("https://") || text.starts_with("http://"))
        .then(|| text.to_owned())
        .filter(|t| !t.contains(char::is_whitespace))
}

/// The answer with the link: the return, its amount when known, and what the AI client must do with the link.
fn antwoord(row: &Value, bedrag: Option<f64>, betaallink: &str, bestond_al: bool) -> Value {
    json!({
        "betaallink": betaallink,
        "aangifte_id": row.get("id").cloned().unwrap_or(Value::Null),
        "soort": soort_naam(row),
        "periode": text_of(row, "periode"),
        "te_betalen": bedrag.map(write::euro),
        "bestond_al": bestond_al,
        "let_op": LET_OP,
    })
}

/// What the user approves before e-Boekhouden starts a payment.
fn voorstel(row: &Value, bedrag: Option<f64>) -> Built {
    let soort = soort_naam(row);
    let periode = known(text_of(row, "periode"));
    let mut lines = vec![
        line("Aangifte", format!("{soort} {periode}")),
        line(
            "Te betalen",
            bedrag
                .map(write::euro)
                .unwrap_or_else(|| "onbekend".to_owned()),
        ),
    ];
    if text_of(row, "lastPayStatus") == "Open" {
        lines.push(line(
            "Betaalstatus",
            "er loopt al een betaling die nog niet is afgerond",
        ));
    }
    lines.push(line(
        "Gevolg",
        "e-Boekhouden start bij zijn betaaldienst een iDEAL-betaling voor deze aangifte en geeft een betaallink. Die poging staat daarna in de betaalgeschiedenis van de aangifte.",
    ));
    lines.push(line(
        "Betalen",
        "er wordt niets betaald tot de gebruiker de link zelf opent en in de eigen bank betaalt. EB MCP opent de link nooit en verstuurt niets naar de Belastingdienst.",
    ));
    Built {
        body: Value::Null,
        title: format!("Betaallink opvragen: {soort} {periode}"),
        lines,
    }
}

#[tool_router(router = btwaangifte_betaal_router, vis = "pub(super)")]
impl EbMcp {
    #[tool(
        description = "De iDEAL-betaallink van een verstuurde aangifte opzoeken (id uit btwaangiftes), zoals de knop Aangifte direct betalen in Verstuurde aangiftes, om aan de gebruiker te tonen. Kan de aangifte niet met iDEAL betaald worden of is hij al betaald, dan zegt de tool dat en geeft geen link. Heeft e-Boekhouden al een link bij de aangifte, dan komt die meteen terug. Anders start e-Boekhouden een iDEAL-betaling bij zijn betaaldienst en geeft een nieuwe link; dat vraagt altijd eerst akkoord van de gebruiker in EB MCP. Er wordt nooit iets betaald of verstuurd: alleen de gebruiker betaalt, door de link zelf te openen. Toon de link aan de gebruiker; open, volg of haal hem nooit op. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Betaallink btw-aangifte",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn btwaangifte_betaallink(
        &self,
        Parameters(args): Parameters<BetaallinkArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "btwaangifte_betaallink";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Betaallink gevonden. Toon hem aan de gebruiker en open hem niet; er is niets betaald en niets verstuurd.",
            move |this, run, args| async move {
                let id = args.id;
                let rows = this.btwaangifte_rows(&run, "aangifte/gridtable", &[]).await?;
                let row = rows
                    .into_iter()
                    .find(|row| row.get("id").and_then(Value::as_u64) == Some(id))
                    .ok_or_else(|| format!("Er is geen verstuurde aangifte met id {id}; zie btwaangiftes"))?;
                betaalbaar(&row)?;
                let bedrag = match Verzonden::of(&row) {
                    Verzonden::Aangifte | Verzonden::Suppletie => this
                        .btwaangifte_get(&run, &format!("aangifte/aangifteomzetbelasting/{id}"), Vec::new())
                        .await
                        .ok()
                        .and_then(|a| {
                            a.get("berekening")
                                .and_then(|b| b.get("totaalTeBetalenTerugTeVragen"))
                                .and_then(|v| number(Some(v)).ok())
                        }),
                    _ => None,
                };
                // The screen opens a link the row has already: nothing changes at e-Boekhouden, so no approval.
                if let Some(bestaand) = link(row.get("payUrl")) {
                    return Ok(antwoord(&row, bedrag, &bestaand, true));
                }
                let built = voorstel(&row, bedrag);
                this.approve(&run, TOOL, &built, true).await?;
                let gestart = this.btwaangifte_betaling_starten(&run, id).await?;
                let nieuw = link(gestart.get("startPaymentUrl")).ok_or_else(|| {
                    Halt::Error(
                        "e-Boekhouden gaf geen betaallink terug. Er is niets betaald; probeer het later opnieuw of betaal vanuit e-Boekhouden zelf."
                            .to_owned(),
                    )
                })?;
                Ok(antwoord(&row, bedrag, &nieuw, false))
            },
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(fields: Value) -> Value {
        let mut row = json!({"id": 5, "soort": "OB", "periode": "3e kwartaal 2026", "canPay": true, "iDeal": true,
                             "payUrl": null, "lastPayStatus": null, "lastPayDate": null, "mustPayDiff": false});
        for (key, value) in fields.as_object().unwrap() {
            row[key] = value.clone();
        }
        row
    }

    #[test]
    fn offers_a_link_only_where_the_screen_shows_the_button() {
        assert_eq!(betaalbaar(&row(json!({}))), Ok(()));
        assert_eq!(betaalbaar(&row(json!({"lastPayStatus": "Open"}))), Ok(()));
        // Without iDEAL, a past payment date still shows the button, as the template's condition.
        assert_eq!(
            betaalbaar(&row(
                json!({"iDeal": false, "lastPayDate": "2026-10-01T10:00:00"})
            )),
            Ok(())
        );
        let refused = |fields: Value| betaalbaar(&row(fields)).unwrap_err();
        assert!(refused(json!({"canPay": false})).contains("knop Aangifte direct betalen"));
        assert!(refused(json!({"iDeal": false})).contains("niet met iDEAL"));
        assert!(refused(json!({"mustPayDiff": true})).contains("niet met iDEAL"));
        let paid = refused(json!({"lastPayStatus": "Success", "lastPayDate": "01-10-2026 10:00"}));
        assert!(
            paid.contains("al via iDEAL betaald op 01-10-2026 10:00"),
            "{paid}"
        );
        assert!(
            refused(json!({"lastPayStatus": "Betaald"})).contains("op een andere manier betaald")
        );
        assert!(refused(json!({"lastPayStatus": "Te ontvangen"})).contains("geld terug"));
        assert!(
            refused(json!({"soort": "SUP", "canPay": 0}))
                .starts_with("Deze suppletie (3e kwartaal 2026)")
        );
    }

    #[test]
    fn takes_only_a_web_address_as_a_link() {
        assert_eq!(
            link(Some(&json!(" https://betaal.example/s/1 "))),
            Some("https://betaal.example/s/1".into())
        );
        assert_eq!(link(Some(&json!("javascript:alert(1)"))), None);
        assert_eq!(link(Some(&json!("https://a b"))), None);
        assert_eq!(link(Some(&json!(""))), None);
        assert_eq!(link(Some(&json!(12))), None);
        assert_eq!(link(None), None);
    }

    #[test]
    fn the_approval_says_what_happens_and_that_nothing_is_paid() {
        let built = voorstel(&row(json!({"lastPayStatus": "Open"})), Some(1234.5));
        assert_eq!(
            built.title,
            "Betaallink opvragen: btw-aangifte 3e kwartaal 2026"
        );
        assert_eq!(built.body, Value::Null);
        let lines: Vec<(&str, &str)> = built
            .lines
            .iter()
            .map(|l| (l.label.as_str(), l.value.as_str()))
            .collect();
        assert!(lines.contains(&("Aangifte", "btw-aangifte 3e kwartaal 2026")));
        assert!(lines.contains(&("Te betalen", "1234,50")));
        assert!(
            lines
                .iter()
                .any(|(l, v)| *l == "Betaalstatus" && v.contains("loopt al"))
        );
        assert!(
            lines
                .iter()
                .any(|(l, v)| *l == "Gevolg" && v.contains("iDEAL-betaling"))
        );
        assert!(
            lines
                .iter()
                .any(|(l, v)| *l == "Betalen" && v.contains("opent de link nooit"))
        );
        let unknown = voorstel(&row(json!({})), None);
        assert!(
            unknown
                .lines
                .iter()
                .any(|l| l.label == "Te betalen" && l.value == "onbekend")
        );
        let answer = antwoord(
            &row(json!({})),
            Some(1234.5),
            "https://betaal.example/s/1",
            false,
        );
        assert_eq!(answer["betaallink"], "https://betaal.example/s/1");
        assert_eq!(answer["te_betalen"], "1234,50");
        assert_eq!(answer["periode"], "3e kwartaal 2026");
        assert!(
            answer["let_op"]
                .as_str()
                .unwrap()
                .contains("Open de link niet")
        );
    }
}
