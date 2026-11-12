//! End to end: the tools of this part over real MCP HTTP, against a stand-in for e-Boekhouden. A change reaches the
//! stand-in only after the user approved it; one that is refused, or that the tool refuses, sends nothing.

use std::sync::{Arc, RwLock};

use axum::body::Bytes;
use axum::extract::Query;
use axum::http::{Method, Uri};
use axum::routing::get;
use serde_json::{Value, json};

use crate::accounts::{AccountInput, Store, WriteMode};
use crate::approvals::{Proposal, Record, Status};
use crate::mcp::{PATH, router};
use crate::state::AppState;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// What the stand-in was asked to change: method, path and body, in order.
type Seen = Arc<std::sync::Mutex<Vec<Value>>>;

/// A stand-in for e-Boekhouden with the reads of the three screens; every change lands in `seen`.
async fn eboekhouden(seen: Seen) -> String {
    // Every change, whatever its path, is noted and answered the way e-Boekhouden answers it.
    let record = move |method: Method, uri: Uri, body: Bytes| {
        let seen = seen.clone();
        async move {
            let path = uri.path().trim_start_matches("/v1/api/").to_owned();
            let body = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
            seen.lock().unwrap().push(json!({"method": method.as_str(), "path": path, "query": uri.query(), "body": body}));
            let answer = match (method.as_str(), path.as_str()) {
                ("POST", "importregel") => json!({"id": 77}),
                ("POST", "import/verwerk") => {
                    json!({"countImport": 13, "countMutatie": 5, "countErrors": 0})
                }
                ("POST", "vasteactiva") => json!({"id": 12}),
                _ => json!(true),
            };
            axum::Json(answer)
        }
    };
    let app = axum::Router::new()
        .route(
            "/v1/api/grootboekrekening/search",
            get(|Query(query): Query<std::collections::HashMap<String, String>>| async move {
                let list = match query.get("rekeningCategorie").map(String::as_str) {
                    Some("VW,BAL,BTWRC") => json!([{"id": 45, "code": "4550", "omschrijving": "Bankkosten"}]),
                    Some("CRED") => json!([{"id": 17, "code": "1700", "omschrijving": "Crediteuren"}]),
                    _ => json!([]),
                };
                axum::Json(list)
            }),
        )
        .route(
            "/v1/api/btwinfo/all",
            get(|| async { axum::Json(json!([{"code": "GEEN", "omschrijving": "Geen btw", "percentage": 0, "soort": ""}])) }),
        )
        .route("/v1/api/import/countImport", get(|| async { axum::Json(json!(13)) }))
        .route(
            "/v1/api/importregel/gridtable",
            get(|| async { axum::Json(json!({"colMetadata": [{"name": "id"}], "data": [[1]], "rowCount": 4})) }),
        )
        .route(
            "/v1/api/vasteactivagroep",
            get(|| async {
                axum::Json(json!([{"id": 3, "naam": "Computers", "aantalJaren": 3, "grootboekrekeningActivaId": 130, "grootboekrekeningAfschrijvingId": 470}]))
            }),
        )
        .route(
            "/v1/api/grootboekrekening/selectlist/BAL",
            get(|| async { axum::Json(json!([{"id": 130, "code": "0130", "omschrijving": "Computers"}])) }),
        )
        .route(
            "/v1/api/grootboekrekening/selectlist/VW",
            get(|| async { axum::Json(json!([{"id": 470, "code": "4700", "omschrijving": "Afschrijving computers"}])) }),
        )
        .route(
            "/v1/api/vasteactiva/12",
            get(|| async {
                axum::Json(json!({"id": 12, "vasteActivaGroepId": 3, "omschrijving": "MacBook Pro", "aanschafDatum": "2026-03-15T00:00:00",
                                  "aanschafwaarde": 2499, "restwaarde": 0, "aantalJaren": 3, "boekenPer": "J",
                                  "grootboekrekeningBal": 130, "grootboekrekeningVw": 470, "geavanceerd": false}))
            })
            .delete(record.clone()),
        )
        .route(
            "/v1/api/vasteactivamutatie/gridtable/12",
            get(|| async {
                axum::Json(json!({"colMetadata": [{"name": "id"}, {"name": "bedrag"}, {"name": "mutatieNr"}], "data": [[1, 833, 4012], [2, 833, null]], "rowCount": 2}))
            }),
        )
        .route(
            "/v1/api/beginbalans/beginbalans",
            get(|| async {
                axum::Json(json!({"datum": {"day": 1, "month": 1, "year": 2026}, "oudsteMutatieDatum": null, "regels": [
                    {"gbId": 11, "code": "1010", "omschrijving": "Bank", "soort": "FIN", "activa": 1000, "passiva": 0},
                    {"gbId": 20, "code": "0500", "omschrijving": "Eigen vermogen", "soort": "BAL", "activa": 0, "passiva": 1000}
                ]}))
            })
            .put(record.clone()),
        )
        .route("/v1/api/beginbalans/saldobeginbalansopenpost/2", get(|| async { axum::Json(json!(0)) }))
        .route("/v1/api/beginbalans/saldobeginbalansopenpost/1", get(|| async { axum::Json(json!(0)) }))
        .fallback(record);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://127.0.0.1:{port}/v1/api/")
}

/// One signed-in account behind a running MCP server, and a client session on it.
struct Harness {
    state: Arc<AppState>,
    seen: Seen,
    account: uuid::Uuid,
    http: reqwest::Client,
    url: String,
    session: Option<String>,
}

fn input(mode: WriteMode) -> AccountInput {
    AccountInput {
        label: "Studio Noord".into(),
        username: "lisa@studionoord.example".into(),
        store: Store::System,
        op_ref: None,
        op_title: None,
        op_vault: None,
        write: mode,
    }
}

impl Harness {
    async fn new() -> Self {
        let seen: Seen = Arc::default();
        let jar = Arc::new(crate::state::testing::FakeJar::default());
        let state = Arc::new(crate::state::testing::state_with(
            jar,
            &eboekhouden(seen.clone()).await,
        ));
        let account = state.accounts.add(input(WriteMode::Off)).unwrap();
        state.jar_put("sessie", vec![("auth-token".into(), "jwt".into())]);
        state.add_session(account.id, &account.label, "sessie");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let mcp = router(state.clone(), Arc::new(RwLock::new(TOKEN.to_owned())));
        tokio::spawn(async move { axum::serve(listener, mcp).await.unwrap() });
        let mut harness = Self {
            state,
            seen,
            account: account.id,
            http: reqwest::Client::new(),
            url: format!("http://127.0.0.1:{port}{PATH}"),
            session: None,
        };
        let init = harness
            .post(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "title": "Claude", "version": "1"}}}))
            .await;
        harness.session = init
            .headers()
            .get("mcp-session-id")
            .map(|v| v.to_str().unwrap().to_owned());
        harness
            .post(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .await;
        harness
    }

    async fn post(&self, body: Value) -> reqwest::Response {
        let mut request = self
            .http
            .post(&self.url)
            .bearer_auth(TOKEN)
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", "2025-06-18")
            .json(&body);
        if let Some(s) = &self.session {
            request = request.header("mcp-session-id", s);
        }
        request.send().await.unwrap()
    }

    fn set_write(&self, mode: WriteMode) {
        self.state
            .accounts
            .update(self.account, input(mode))
            .unwrap();
    }

    /// Calls a tool: whether it answered with an error, and its text.
    async fn call(&self, name: &str, arguments: Value) -> (bool, String) {
        let body = self
            .post(json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": name, "arguments": arguments}}))
            .await
            .text()
            .await
            .unwrap();
        let answer = serde_json::from_str::<Value>(&body)
            .ok()
            .unwrap_or_else(|| {
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

    /// The one proposal that waits for the user.
    fn proposal_up(&self) -> Proposal {
        let list = self.state.approvals.list();
        assert_eq!(list.len(), 1, "{list:?}");
        list.into_iter().next().unwrap()
    }

    fn seen(&self) -> Vec<Value> {
        self.seen.lock().unwrap().clone()
    }

    /// Waits for the background run of an answered proposal to end, and gives its record.
    async fn settled(&self, id: u64) -> Record {
        for _ in 0..500 {
            if let Some(detail) = self.state.approvals.get(id)
                && !matches!(detail.record.status, Status::Waiting | Status::Running)
                && let Some(crate::reqlog::Entry::Mcp(entry)) =
                    detail.log_id.and_then(|log_id| self.state.log.get(log_id))
                && !matches!(
                    entry.outcome,
                    crate::reqlog::Outcome::Waiting | crate::reqlog::Outcome::Running
                )
            {
                return detail.record;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("proposal {id} did not settle");
    }
}

/// The proposal id of a "waiting" answer.
fn waiting_id(text: &str) -> u64 {
    let value: Value = serde_json::from_str(text).unwrap_or_else(|_| panic!("not JSON: {text}"));
    assert_eq!(value["status"], "wacht", "{text}");
    value["voorstel_id"].as_u64().unwrap()
}

#[tokio::test]
async fn an_importregel_goes_out_only_when_approved_and_running_them_always_asks() {
    let h = Harness::new().await;
    let regel = json!({"omschrijving": "Triodos", "bij_af": "afschrijving", "tegenrekening": "4550", "btw_code": "GEEN"});

    // Writing is off: refused, nothing read or asked.
    let (is_error, text) = h.call("importregel_toevoegen", regel.clone()).await;
    assert!(is_error && text.contains("alleen-lezen"), "{text}");
    assert_eq!(h.state.approvals.count(), 0);

    // A rule without any criterion is refused before anything is asked.
    h.set_write(WriteMode::Ask);
    let (is_error, text) = h
        .call(
            "importregel_toevoegen",
            json!({"bij_af": "afschrijving", "tegenrekening": "4550", "btw_code": "GEEN"}),
        )
        .await;
    assert!(is_error && text.contains("criterium"), "{text}");
    assert_eq!(h.state.approvals.count(), 0);

    // A good rule waits; approved, exactly the form's body goes out.
    let (is_error, text) = h.call("importregel_toevoegen", regel).await;
    assert!(!is_error, "{text}");
    let id = waiting_id(&text);
    let proposal = h.proposal_up();
    assert_eq!(proposal.id, id);
    assert_eq!(
        proposal.title,
        "Importregel toevoegen: \"Triodos\" (afschrijving)"
    );
    assert!(!proposal.danger);
    assert!(h.seen().is_empty(), "nothing goes out before the answer");
    assert!(h.state.approvals.decide(id, true, 1));
    {
        let r = h.settled(id).await;
        assert_eq!(r.status, Status::Done, "{r:?}");
    }
    assert_eq!(
        h.seen(),
        vec![
            json!({"method": "POST", "path": "importregel", "query": null, "body": {
            "omschrijving": "Triodos", "bijAf": "Afschrijving", "mutatieSoortId": 6, "mutatieBtwCode": "GEEN",
            "mutatieTegenrekeningId": 45, "actief": true}})
        ]
    );

    // Running all rules books in bulk: it asks even when the account needs no approval.
    h.set_write(WriteMode::Auto);
    let (is_error, text) = h.call("importregels_verwerken", json!({})).await;
    assert!(!is_error, "{text}");
    let id = waiting_id(&text);
    let proposal = h.proposal_up();
    assert!(proposal.danger);
    assert!(proposal.title.contains("13"), "{}", proposal.title);
    assert!(h.state.approvals.decide(id, false, 2));
    assert_eq!(h.settled(id).await.status, Status::Refused);
    assert_eq!(h.seen().len(), 1, "a refused run sends nothing");

    let (_, text) = h.call("importregels_verwerken", json!({})).await;
    let id = waiting_id(&text);
    assert!(h.state.approvals.decide(id, true, 3));
    {
        let r = h.settled(id).await;
        assert_eq!(r.status, Status::Done, "{r:?}");
    }
    assert_eq!(
        h.seen()[1],
        json!({"method": "POST", "path": "import/verwerk", "query": null, "body": null})
    );
}

#[tokio::test]
async fn a_vast_activum_is_added_after_approval_and_deleting_always_asks() {
    let h = Harness::new().await;
    h.set_write(WriteMode::Ask);

    // Reading names the group and the rekeningen.
    let (is_error, text) = h.call("vastactief", json!({"id": 12})).await;
    assert!(!is_error, "{text}");
    assert!(
        text.contains("\"groep\":\"Computers\"") && text.contains("4700 Afschrijving computers"),
        "{text}"
    );

    let (is_error, text) = h
        .call(
            "vastactief_toevoegen",
            json!({"omschrijving": "MacBook Pro", "aanschafdatum": "2026-03-15", "aanschafwaarde": 2499, "restwaarde": 3000}),
        )
        .await;
    assert!(is_error && text.contains("restwaarde"), "{text}");
    assert_eq!(h.state.approvals.count(), 0);

    let (is_error, text) = h
        .call(
            "vastactief_toevoegen",
            json!({"omschrijving": "MacBook Pro", "aanschafdatum": "2026-03-15", "aanschafwaarde": 2499, "restwaarde": 0}),
        )
        .await;
    assert!(!is_error, "{text}");
    let id = waiting_id(&text);
    assert!(!h.proposal_up().danger);
    assert!(h.seen().is_empty());
    assert!(h.state.approvals.decide(id, true, 1));
    {
        let r = h.settled(id).await;
        assert_eq!(r.status, Status::Done, "{r:?}");
    }
    assert_eq!(
        h.seen(),
        vec![
            json!({"method": "POST", "path": "vasteactiva", "query": null, "body": {
            "vasteActivaGroepId": 3, "omschrijving": "MacBook Pro", "aanschafDatum": {"day": 15, "month": 3, "year": 2026},
            "aanschafwaarde": 2499.0, "restwaarde": 0.0, "aantalJaren": 3, "boekenPer": "J",
            "grootboekrekeningBal": 130, "grootboekrekeningVw": 470, "geavanceerd": false}})
        ]
    );

    // An edit carries the recalc flag in the query.
    h.set_write(WriteMode::Auto);
    let (is_error, text) = h
        .call(
            "vastactief_wijzigen",
            json!({"id": 12, "restwaarde": 100, "schema_opnieuw": true}),
        )
        .await;
    assert!(!is_error, "{text}");
    let edit = &h.seen()[1];
    assert_eq!(edit["method"], "PUT");
    assert_eq!(edit["path"], "vasteactiva");
    assert_eq!(edit["query"], "recalc=true");
    assert_eq!(edit["body"]["restwaarde"], json!(100.0));

    // Deleting asks even without approval otherwise, and says what was booked.
    let (is_error, text) = h.call("vastactief_verwijderen", json!({"id": 12})).await;
    assert!(!is_error, "{text}");
    let id = waiting_id(&text);
    let proposal = h.proposal_up();
    assert!(proposal.danger);
    assert!(
        proposal
            .lines
            .iter()
            .any(|l| l.label == "Afschrijfschema" && l.value.contains("mutatie 4012")),
        "{:?}",
        proposal.lines
    );
    assert_eq!(h.seen().len(), 2);
    assert!(h.state.approvals.decide(id, true, 2));
    {
        let r = h.settled(id).await;
        assert_eq!(r.status, Status::Done, "{r:?}");
    }
    assert_eq!(
        h.seen()[2],
        json!({"method": "DELETE", "path": "vasteactiva/12", "query": null, "body": null})
    );
}

#[tokio::test]
async fn the_beginbalans_changes_only_in_balance_and_always_asks() {
    let h = Harness::new().await;
    h.set_write(WriteMode::Auto);

    let (is_error, text) = h.call("beginbalans", json!({})).await;
    assert!(!is_error, "{text}");
    assert!(text.contains("\"in_evenwicht\":true"), "{text}");

    // Out of balance: refused, nothing asked or sent.
    let (is_error, text) = h
        .call(
            "beginbalans_wijzigen",
            json!({"regels": [{"rekening": "1010", "activa": 1200}]}),
        )
        .await;
    assert!(is_error && text.contains("niet gelijk"), "{text}");
    assert_eq!(h.state.approvals.count(), 0);
    assert!(h.seen().is_empty());

    // In balance, with a rekening that is new on the beginbalans: asked although the account needs no approval.
    let (is_error, text) = h
        .call(
            "beginbalans_wijzigen",
            json!({"regels": [{"rekening": "1010", "activa": 1200}, {"rekening": "1700", "passiva": 200}]}),
        )
        .await;
    assert!(!is_error, "{text}");
    let id = waiting_id(&text);
    let proposal = h.proposal_up();
    assert!(proposal.danger);
    assert!(
        proposal.lines.iter().any(|l| l.label == "1010 Bank"
            && l.value == "activa was 1000,00, wordt 1200,00; passiva blijft 0,00"),
        "{:?}",
        proposal.lines
    );
    assert!(h.seen().is_empty());
    assert!(h.state.approvals.decide(id, true, 1));
    {
        let r = h.settled(id).await;
        assert_eq!(r.status, Status::Done, "{r:?}");
    }
    let sent = h.seen();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["method"], "PUT");
    assert_eq!(sent[0]["path"], "beginbalans/beginbalans");
    assert_eq!(
        sent[0]["body"]["regels"],
        json!([
            {"gbId": 11, "code": "1010", "omschrijving": "Bank", "soort": "FIN", "activa": 1200.0, "passiva": 0.0},
            {"gbId": 20, "code": "0500", "omschrijving": "Eigen vermogen", "soort": "BAL", "activa": 0.0, "passiva": 1000.0},
            {"gbId": 17, "code": "1700", "omschrijving": "Crediteuren", "soort": "CRED", "activa": 0.0, "passiva": 200.0}
        ])
    );
    assert_eq!(
        sent[0]["body"]["datum"],
        json!({"day": 1, "month": 1, "year": 2026})
    );
}
