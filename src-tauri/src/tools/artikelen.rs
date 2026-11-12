//! Producten en diensten (artikelen, eenheden, groepen) and standaard facturen.
//!
//! The tools of this part live in two submodules, each on its own router; `artikelen_router` adds them up for
//! tools.rs. Each tool is named in one of the lists below, which the tool-list test in mcp.rs reads.
//!
//! Every request shape comes from e-Boekhouden's own web app (the public Angular bundle of 2026-11-12):
//! - the product form (`app-artikel-form`) and list: `POST artikel`, `PUT artikel[?cascadeFull=true|?cascadePriceOnly=true]`,
//!   `DELETE artikel/{id}`, `GET artikel/{id}`, `GET artikel/gridtable[&searchTerm=]`;
//! - the unit form (`app-eenheid-form`): `POST eenheid`, `PUT eenheid`, `DELETE eenheid/{id}`, `GET eenheid[/{id}]`;
//! - the group form (`app-artikelgroep-form`): `POST artikelgroep`, `PUT artikelgroep`, `DELETE artikelgroep/{id}`;
//! - the standaard factuur form (`app-standaard-facturen-form`): `POST`/`PUT standaardfactuur/model` with the same
//!   `factuurItems` as an invoice, type "V"; `DELETE standaardfactuur/{id}`; its lines from `factuuritem/griditems`.
//!
//! Importing products from a file (`app-artikel-import`) is deliberately not built.

use super::*;

mod producten;
mod standaardfacturen;

/// The tools of this part that only read.
#[cfg(test)]
pub(super) const READS: &[&str] = &[
    "artikelen",
    "artikel",
    "eenheden",
    "artikelgroepen",
    "standaardfacturen",
    "standaardfactuur",
];
/// The tools of this part that change the books.
#[cfg(test)]
pub(super) const WRITES: &[&str] = &[
    "artikel_toevoegen",
    "artikel_wijzigen",
    "artikel_verwijderen",
    "eenheid_toevoegen",
    "eenheid_wijzigen",
    "eenheid_verwijderen",
    "artikelgroep_toevoegen",
    "artikelgroep_wijzigen",
    "artikelgroep_verwijderen",
    "standaardfactuur_toevoegen",
    "standaardfactuur_wijzigen",
    "standaardfactuur_verwijderen",
];
/// The ones in `WRITES` that cannot be undone.
#[cfg(test)]
pub(super) const DESTRUCTIVE: &[&str] = &[
    "artikel_verwijderen",
    "eenheid_verwijderen",
    "artikelgroep_verwijderen",
    "standaardfactuur_verwijderen",
];

impl EbMcp {
    /// The tools of this part: products, units and groups, and standaard facturen.
    pub(super) fn artikelen_router() -> ToolRouter<Self> {
        Self::artikelen_producten_router() + Self::artikelen_standaardfacturen_router()
    }

    /// Whether the administration charges btw (`btwinfo/btwplichtig`), which the product and invoice forms
    /// ask before they offer btw codes and prices incl. btw.
    async fn artikelen_btwplichtig(&self, run: &Run) -> Result<bool, String> {
        self.fetch(run, "btwinfo/btwplichtig", Vec::new())
            .await?
            .as_bool()
            .ok_or_else(|| "e-Boekhouden zei niet of deze administratie btw-plichtig is".to_owned())
    }

    /// A list that e-Boekhouden gives as a plain array (`eenheid`, `artikelgroep`, `factuursjabloon/active`).
    async fn artikelen_list(&self, run: &Run, path: &str) -> Result<Vec<Value>, String> {
        match self.fetch(run, path, Vec::new()).await? {
            Value::Array(rows) => Ok(rows),
            _ => Err(format!(
                "e-Boekhouden gaf {path} in een onbekende vorm terug"
            )),
        }
    }

    /// A read that e-Boekhouden may answer without a body (or with null) when there is nothing: then `None`.
    async fn artikelen_maybe(&self, run: &Run, path: &str) -> Result<Option<Value>, String> {
        let call = self
            .state
            .api_get(run.account.id, path, &[], Some(run.id), &run.client)
            .await;
        if call.ok() {
            return Ok(call.json.filter(|json| !json.is_null()));
        }
        Err(call
            .error
            .unwrap_or_else(|| format!("e-Boekhouden gaf {} op {path}", call.status)))
    }
}

// ---------------------------------------------------------------- small helpers of this part

/// A text field as the forms check it: trimmed, required, and at most `max` characters.
fn required_text(value: &str, what: &str, max: usize) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{what} is leeg"));
    }
    if value.chars().count() > max {
        return Err(format!("{what} is te lang (hoogstens {max} tekens)"));
    }
    Ok(value.to_owned())
}

/// An amount in euros, rounded to cents; refused when it is no real number.
fn euros(value: f64, what: &str) -> Result<f64, String> {
    if !value.is_finite() || value.abs() > 1e11 {
        return Err(format!("{what} is geen geldig bedrag"));
    }
    Ok(round2(value))
}

fn round2(amount: f64) -> f64 {
    (amount * 100.0).round() / 100.0
}

/// A long text for the approval window: whole unless it is very long.
fn shown(text: &str) -> String {
    const MAX: usize = 300;
    if text.is_empty() {
        return "(leeg)".to_owned();
    }
    if text.chars().count() <= MAX {
        return text.to_owned();
    }
    format!(
        "{} ... (ingekort, {} tekens)",
        text.chars().take(MAX).collect::<String>(),
        text.chars().count()
    )
}

/// A text field of a record read from e-Boekhouden ("" when it is missing or null).
fn text_of(record: &Value, key: &str) -> String {
    record
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// An id field of a record; 0 and null both mean "none", as in e-Boekhouden's forms.
fn id_of(record: &Value, key: &str) -> Option<u64> {
    record.get(key).and_then(Value::as_u64).filter(|id| *id > 0)
}

/// The `InEx` code e-Boekhouden uses for prices incl or excl btw.
fn in_ex_code(in_ex: write::InEx) -> &'static str {
    match in_ex {
        write::InEx::Incl => "IN",
        write::InEx::Excl => "EX",
    }
}

fn in_ex_from(code: &str) -> write::InEx {
    if code.eq_ignore_ascii_case("IN") {
        write::InEx::Incl
    } else {
        write::InEx::Excl
    }
}

#[cfg(test)]
mod tests {
    //! End to end: the tools of this part against a stand-in for e-Boekhouden, through the real MCP server.
    use super::*;
    use crate::accounts::{AccountInput, Store, WriteMode};
    use crate::mcp::{PATH, router};
    use axum::extract::{Path, RawQuery};
    use axum::routing::get;
    use std::sync::RwLock;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// What the stand-in was asked to change, in order: "METHOD path?query body".
    type Changes = Arc<Mutex<Vec<String>>>;

    fn note(changes: &Changes, method: &str, path: &str, query: Option<String>, body: &Value) {
        let query = query.map(|q| format!("?{q}")).unwrap_or_default();
        changes
            .lock()
            .unwrap()
            .push(format!("{method} {path}{query} {body}"));
    }

    /// A stand-in for e-Boekhouden with a product, a unit, a group and a standaard factuur.
    async fn eboekhouden(changes: Changes) -> String {
        use axum::Json;
        let artikel = json!({"id": 31, "code": "UUR", "omschrijving": "Advies per uur", "groepId": 4, "eenheidId": 6,
            "inkoopPrijsExclusief": 0, "prijsExclusief": 100.0, "prijsInclusief": 121.0, "prijsLeading": "EX",
            "btw": "HOOG_VERK_21", "grootboekrekeningId": 80, "actief": 1});
        let (c1, c2, c3, c4, c5, c6, c7, c8) = (
            changes.clone(),
            changes.clone(),
            changes.clone(),
            changes.clone(),
            changes.clone(),
            changes.clone(),
            changes.clone(),
            changes.clone(),
        );
        let app = axum::Router::new()
            .route(
                "/v1/api/grootboekrekening/gridtable",
                get(|| async {
                    Json(json!({
                        "colMetadata": [{"name": "id"}, {"name": "code"}, {"name": "omschrijving"}, {"name": "categorie"}, {"name": "inactief"}],
                        "data": [[80, "8000", "Omzet", "Winst en Verlies", false], [13, "1300", "Debiteuren", "Debiteuren", false]]
                    }))
                }),
            )
            .route(
                "/v1/api/btwinfo/all",
                get(|| async {
                    Json(json!([
                        {"code": "GEEN", "omschrijving": "Geen btw", "percentage": 0, "soort": ""},
                        {"code": "HOOG_VERK_21", "omschrijving": "Hoog 21%", "percentage": 21, "soort": "VERK"},
                        {"code": "LAAG_VERK_9", "omschrijving": "Laag 9%", "percentage": 9, "soort": "VERK"}
                    ]))
                }),
            )
            .route("/v1/api/btwinfo/btwplichtig", get(|| async { Json(json!(true)) }))
            .route(
                "/v1/api/artikelgroep",
                get(|| async { Json(json!([{"id": 4, "code": "DIENST", "omschrijving": "Diensten"}])) }).put(
                    move |Json(body): Json<Value>| {
                        let changes = c1.clone();
                        async move {
                            note(&changes, "PUT", "artikelgroep", None, &body);
                            Json(json!(true))
                        }
                    },
                ),
            )
            .route(
                "/v1/api/artikelgroep/{id}",
                get(|Path(id): Path<u64>| async move { Json(json!({"id": id, "code": "DIENST", "omschrijving": "Diensten"})) }).delete(
                    move |Path(id): Path<u64>| {
                        let changes = c2.clone();
                        async move {
                            note(&changes, "DELETE", &format!("artikelgroep/{id}"), None, &Value::Null);
                            Json(json!(true))
                        }
                    },
                ),
            )
            .route(
                "/v1/api/eenheid",
                get(|| async { Json(json!([{"id": 6, "enkelvoud": "uur", "meervoud": "uren"}])) }).post(
                    move |Json(body): Json<Value>| {
                        let changes = c3.clone();
                        async move {
                            note(&changes, "POST", "eenheid", None, &body);
                            Json(json!({"id": 7, "enkelvoud": body["enkelvoud"], "meervoud": body["meervoud"]}))
                        }
                    },
                ),
            )
            .route(
                "/v1/api/eenheid/{id}",
                get(|Path(id): Path<u64>| async move { Json(json!({"id": id, "enkelvoud": "uur", "meervoud": "uren"})) }).delete(
                    move |Path(id): Path<u64>| {
                        let changes = c4.clone();
                        async move {
                            note(&changes, "DELETE", &format!("eenheid/{id}"), None, &Value::Null);
                            Json(json!(true))
                        }
                    },
                ),
            )
            .route(
                "/v1/api/artikel",
                axum::routing::post(move |Json(body): Json<Value>| {
                    let changes = c5.clone();
                    async move {
                        note(&changes, "POST", "artikel", None, &body);
                        Json(json!({"id": 32, "code": body["code"]}))
                    }
                })
                .put(move |RawQuery(query): RawQuery, Json(body): Json<Value>| {
                    let changes = c6.clone();
                    async move {
                        note(&changes, "PUT", "artikel", query, &body);
                        Json(json!(true))
                    }
                }),
            )
            .route("/v1/api/artikel/{id}", get(move || {
                let artikel = artikel.clone();
                async move { Json(artikel) }
            }))
            // No kostenplaats on this product: e-Boekhouden answers without a body.
            .route("/v1/api/kostenplaats/GetByArtikelId/{id}", get(|| async { axum::http::StatusCode::NO_CONTENT }))
            .route("/v1/api/factuursetting", get(|| async { Json(json!({"factuurSjabloonId": 9, "verwerken": false})) }))
            .route("/v1/api/factuursjabloon/active", get(|| async { Json(json!([{"id": 8, "naam": "Oud"}, {"id": 9, "naam": "Standaard"}])) }))
            .route("/v1/api/emailsjabloonab/forfactuur", get(|| async { Json(json!([{"id": 5, "naam": "Factuurmail"}])) }))
            .route(
                "/v1/api/standaardfactuur/model",
                axum::routing::post(move |Json(body): Json<Value>| {
                    let changes = c7.clone();
                    async move {
                        note(&changes, "POST", "standaardfactuur/model", None, &body);
                        Json(json!({"id": 501, "nummer": body["factuur"]["nummer"]}))
                    }
                }),
            )
            .route(
                "/v1/api/standaardfactuur/{id}",
                get(|Path(id): Path<u64>| async move {
                    Json(json!({"id": id, "nummer": "Maandelijks onderhoud", "sjabloonId": 9, "emailSjabloonId": null, "inEx": "EX", "type": "V"}))
                })
                .delete(move |Path(id): Path<u64>| {
                    let changes = c8.clone();
                    async move {
                        note(&changes, "DELETE", &format!("standaardfactuur/{id}"), None, &Value::Null);
                        Json(json!(true))
                    }
                }),
            )
            .route(
                "/v1/api/factuuritem/griditems",
                get(|| async { Json(json!([{"id": 9001, "aantal": 1, "omschrijving": "Onderhoud", "btw": "HOOG_VERK_21", "grootboekId": 80, "prijsExcl": 50, "prijsIncl": 60.5}])) }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://127.0.0.1:{port}/v1/api/")
    }

    /// The JSON-RPC answer in a body that is plain JSON or server-sent events: whether it is an error, and its text.
    fn answer_of(body: &str) -> (bool, String) {
        let answer = serde_json::from_str::<Value>(body).ok().unwrap_or_else(|| {
            body.lines()
                .filter_map(|l| l.strip_prefix("data:"))
                .filter_map(|d| serde_json::from_str::<Value>(d.trim()).ok())
                .find(|v| v.get("id").is_some())
                .unwrap_or_else(|| panic!("no JSON-RPC answer in {body}"))
        });
        (
            answer["result"]["isError"] == true,
            answer["result"]["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        )
    }

    /// The proposal id of a "waiting" answer.
    fn waiting_id(text: &str) -> u64 {
        let value: Value =
            serde_json::from_str(text).unwrap_or_else(|_| panic!("not JSON: {text}"));
        assert_eq!(value["status"], "wacht", "{text}");
        value["voorstel_id"].as_u64().unwrap()
    }

    /// Waits for the background run of an answered proposal to end, and gives its record.
    async fn settled(state: &AppState, id: u64) -> approvals::Record {
        use approvals::Status;
        for _ in 0..500 {
            if let Some(detail) = state.approvals.get(id)
                && !matches!(detail.record.status, Status::Waiting | Status::Running)
                && let Some(crate::reqlog::Entry::Mcp(entry)) =
                    detail.log_id.and_then(|log_id| state.log.get(log_id))
                && !matches!(entry.outcome, Outcome::Waiting | Outcome::Running)
            {
                return detail.record;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("proposal {id} did not settle");
    }

    #[tokio::test]
    async fn products_units_groups_and_standard_invoices_change_only_after_approval() {
        let changes: Changes = Arc::default();
        let jar = Arc::new(crate::state::testing::FakeJar::default());
        let state = Arc::new(crate::state::testing::state_with(
            jar,
            &eboekhouden(changes.clone()).await,
        ));
        let input = |mode: WriteMode| AccountInput {
            label: "Studio Noord".into(),
            username: "lisa@studionoord.example".into(),
            store: Store::System,
            op_ref: None,
            op_title: None,
            op_vault: None,
            write: mode,
        };
        let account = state.accounts.add(input(WriteMode::Ask)).unwrap();
        state.jar_put("sessie", vec![("auth-token".into(), "jwt".into())]);
        state.add_session(account.id, &account.label, "sessie");

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let mcp = router(state.clone(), Arc::new(RwLock::new(TOKEN.to_owned())));
        tokio::spawn(async move { axum::serve(listener, mcp).await.unwrap() });
        let http = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{port}{PATH}");
        let post = |body: Value, session: Option<String>| {
            let mut request = http
                .post(&url)
                .bearer_auth(TOKEN)
                .header("accept", "application/json, text/event-stream")
                .header("mcp-protocol-version", "2025-06-18")
                .json(&body);
            if let Some(s) = session {
                request = request.header("mcp-session-id", s);
            }
            request.send()
        };
        let init = post(
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}}),
            None,
        )
        .await
        .unwrap();
        let session = init
            .headers()
            .get("mcp-session-id")
            .map(|v| v.to_str().unwrap().to_owned());
        post(
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            session.clone(),
        )
        .await
        .unwrap();
        let call = |tool: &'static str, arguments: Value| {
            let request = post(
                json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": tool, "arguments": arguments}}),
                session.clone(),
            );
            async move { answer_of(&request.await.unwrap().text().await.unwrap()) }
        };
        let changed = || changes.lock().unwrap().clone();
        let proposal = |id: u64| {
            state
                .approvals
                .list()
                .into_iter()
                .find(|p| p.id == id)
                .unwrap_or_else(|| panic!("proposal {id} is not up"))
        };

        // A new product: up for approval, refused, and nothing reaches e-Boekhouden.
        let new_product = json!({"code": "UUR2", "omschrijving": "Ontwerp per uur", "prijs": 121, "bedragen": "incl",
            "btw_code": "HOOG_VERK_21", "tegenrekening": "8000", "groep_id": 4, "eenheid_id": 6});
        let (is_error, text) = call("artikel_toevoegen", new_product.clone()).await;
        assert!(!is_error, "{text}");
        let id = waiting_id(&text);
        let up = proposal(id);
        assert_eq!(up.title, "Product UUR2 toevoegen");
        assert!(!up.danger);
        assert!(
            up.lines
                .iter()
                .any(|l| l.label == "Prijs excl. btw" && l.value == "100,00"),
            "{:?}",
            up.lines
        );
        assert!(state.approvals.decide(id, false, 1));
        settled(&state, id).await;
        assert!(changed().is_empty(), "a refused change sends nothing");

        // Approved: exactly the body of e-Boekhouden's own product form.
        let (_, text) = call("artikel_toevoegen", new_product).await;
        let id = waiting_id(&text);
        assert!(changed().is_empty(), "nothing goes out before the answer");
        assert!(state.approvals.decide(id, true, 2));
        assert_eq!(settled(&state, id).await.status, approvals::Status::Done);
        assert_eq!(
            changed(),
            vec![format!(
                "POST artikel {}",
                json!({"groepId": 4, "code": "UUR2", "omschrijving": "Ontwerp per uur", "eenheidId": 6, "inkoopPrijsExclusief": 0.0,
                    "prijsExclusief": 100.0, "prijsInclusief": 121.0, "prijsLeading": "IN", "btw": "HOOG_VERK_21",
                    "grootboekrekeningId": 80, "actief": 1})
            )]
        );

        // Writing without approval: an edit goes straight through, but passing it on to the subscriptions
        // changes more than this product, so that still asks; the query says how far it goes.
        state
            .accounts
            .update(account.id, input(WriteMode::Auto))
            .unwrap();
        let (is_error, text) = call(
            "artikel_wijzigen",
            json!({"id": 31, "prijs": 110, "doorvoeren": "alleen_prijs"}),
        )
        .await;
        assert!(!is_error, "{text}");
        let id = waiting_id(&text);
        assert!(proposal(id).danger);
        assert!(state.approvals.decide(id, true, 3));
        settled(&state, id).await;
        let last = changed().pop().unwrap();
        assert!(
            last.starts_with("PUT artikel?cascadePriceOnly=true "),
            "{last}"
        );
        assert!(
            last.contains("\"prijsExclusief\":110.0") && last.contains("\"prijsInclusief\":133.1"),
            "{last}"
        );
        assert!(last.contains("\"id\":31"), "{last}");

        // A unit is added at once; deleting one always asks, even now.
        let (is_error, text) = call(
            "eenheid_toevoegen",
            json!({"enkelvoud": "dag", "meervoud": "dagen"}),
        )
        .await;
        assert!(!is_error, "{text}");
        assert!(text.contains("\"id\":7"), "{text}");
        assert_eq!(
            changed().pop().unwrap(),
            format!(
                "POST eenheid {}",
                json!({"enkelvoud": "dag", "meervoud": "dagen"})
            )
        );
        let before = changed().len();
        let (_, text) = call("eenheid_verwijderen", json!({"id": 6})).await;
        let id = waiting_id(&text);
        let up = proposal(id);
        assert!(up.danger);
        assert_eq!(up.title, "Eenheid uur verwijderen");
        assert!(state.approvals.decide(id, false, 4));
        settled(&state, id).await;
        assert_eq!(changed().len(), before, "a refused delete sends nothing");

        // A group: renamed straight away, and deleted only after approval.
        let (is_error, text) = call(
            "artikelgroep_wijzigen",
            json!({"id": 4, "omschrijving": "Advies en ontwerp"}),
        )
        .await;
        assert!(!is_error, "{text}");
        assert_eq!(
            changed().pop().unwrap(),
            format!(
                "PUT artikelgroep {}",
                json!({"code": "DIENST", "omschrijving": "Advies en ontwerp", "id": 4})
            )
        );
        let (_, text) = call("artikelgroep_verwijderen", json!({"id": 4})).await;
        let id = waiting_id(&text);
        assert!(state.approvals.decide(id, true, 5));
        settled(&state, id).await;
        assert_eq!(
            changed().pop().unwrap(),
            "DELETE artikelgroep/4 null".to_owned()
        );

        // A standaard factuur with the default invoice template and the lines of an invoice.
        let (is_error, text) = call(
            "standaardfactuur_toevoegen",
            json!({"naam": "Maandelijks onderhoud", "email_sjabloon_id": 5,
                "regels": [{"omschrijving": "Onderhoud", "prijs": 50, "btw_code": "HOOG_VERK_21", "tegenrekening": "8000"}]}),
        )
        .await;
        assert!(!is_error, "{text}");
        assert!(text.contains("\"id\":501"), "{text}");
        assert_eq!(
            changed().pop().unwrap(),
            format!(
                "POST standaardfactuur/model {}",
                json!({"factuur": {"nummer": "Maandelijks onderhoud", "sjabloonId": 9, "emailSjabloonId": 5, "type": "V", "inEx": "EX"},
                    "factuurItems": {"insert": [{"id": -1, "factuurId": 0, "aantal": 1.0, "omschrijving": "Onderhoud", "prijs": null,
                        "btw": "HOOG_VERK_21", "grootboekId": 80, "prijsExcl": 50.0, "prijsIncl": 60.5, "kortingIsPerc": true, "volgorde": 0}],
                        "update": [], "delete": []}})
            )
        );
        // Deleting it always asks, and goes out after the yes.
        let (_, text) = call("standaardfactuur_verwijderen", json!({"id": 501})).await;
        let id = waiting_id(&text);
        let up = proposal(id);
        assert!(up.danger);
        assert_eq!(
            up.title,
            "Standaard factuur Maandelijks onderhoud verwijderen"
        );
        assert!(state.approvals.decide(id, true, 6));
        settled(&state, id).await;
        assert_eq!(
            changed().pop().unwrap(),
            "DELETE standaardfactuur/501 null".to_owned()
        );

        // Bad input is refused before anything is sent.
        let before = changed().len();
        let (is_error, text) = call(
            "artikel_toevoegen",
            json!({"code": "X", "omschrijving": "Iets", "prijs": 1, "btw_code": "HOOG_INK_21", "tegenrekening": "8000"}),
        )
        .await;
        assert!(is_error && text.contains("HOOG_INK_21"), "{text}");
        assert_eq!(changed().len(), before);

        // Reading needs no approval and changes nothing.
        let (is_error, text) = call("standaardfactuur", json!({"id": 501})).await;
        assert!(!is_error, "{text}");
        assert!(
            text.contains("\"regels\"") && text.contains("Onderhoud"),
            "{text}"
        );
        assert_eq!(changed().len(), before);
    }
}
