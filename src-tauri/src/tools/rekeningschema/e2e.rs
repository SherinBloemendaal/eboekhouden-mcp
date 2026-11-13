//! The tools of this part end to end: over the MCP server, against a stand-in for e-Boekhouden that notes every
//! change it is sent. A change reaches it only after approval; a refused or rejected one sends nothing.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use axum::body::Bytes;
use axum::extract::{Path, RawQuery};
use axum::http::{Method, Uri};
use axum::routing::{get, post};
use serde_json::{Value, json};

use crate::accounts::{Account, AccountInput, Rights, Store};
use crate::approvals::{Proposal, Status};
use crate::state::AppState;

/// What the stand-in was asked to change, in order: "METHOD path" and the body (null without one).
type Changes = Arc<Mutex<Vec<(String, Value)>>>;

/// A stand-in for e-Boekhouden with a small rekeningschema, two groups and three kostenplaatsen.
async fn eboekhouden(changes: Changes, kostenplaatsen_aan: Arc<AtomicBool>) -> String {
    let note =
        |changes: &Changes, what: String, body: Value| changes.lock().unwrap().push((what, body));
    let sent = |changes: Changes, method: &'static str, path: &'static str, answer: Value| {
        move |axum::Json(body): axum::Json<Value>| {
            let changes = changes.clone();
            let answer = answer.clone();
            async move {
                note(&changes, format!("{method} {path}"), body);
                axum::Json(answer)
            }
        }
    };
    let deleted = |changes: Changes, path: &'static str| {
        move |Path(id): Path<u64>| {
            let changes = changes.clone();
            async move {
                note(&changes, format!("DELETE {path}/{id}"), Value::Null);
                axum::Json(json!(true))
            }
        }
    };
    // Any request to a path or method this stand-in does not know is noted as a stray, so a change that goes
    // anywhere unexpected shows up in the tests' lists of what was sent.
    let stray = {
        let changes = changes.clone();
        move |method: Method, uri: Uri, body: Bytes| {
            let changes = changes.clone();
            async move {
                let body = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
                note(&changes, format!("STRAY {method} {}", uri.path()), body);
                axum::Json(json!(true))
            }
        }
    };
    let c = changes;
    let app = axum::Router::new()
        .route(
            "/v1/api/grootboekrekening/gridtable",
            get(|| async {
                axum::Json(json!({
                    "colMetadata": [{"name": "id"}, {"name": "code"}, {"name": "omschrijving"}, {"name": "categorie"}, {"name": "rekeningGroep"}, {"name": "inactief"}],
                    "data": [[11, "1010", "Bank", "Financiële rekeningen", "", false], [45, "4550", "Bankkosten", "Winst & verlies", "KOST", false],
                             [99, "8999", "Oud", "Winst & verlies", "", true]]
                }))
            }),
        )
        .route(
            "/v1/api/grootboekrekening/categories",
            get(|RawQuery(query): RawQuery| async move {
                let all = query.as_deref() == Some("incudeRestricted=true");
                let mut list = vec![
                    json!({"code": "BAL", "omschrijving": "Balans", "restricted": false}),
                    json!({"code": "VW", "omschrijving": "Winst & verlies", "restricted": false}),
                ];
                if all {
                    list.push(json!({"code": "FIN", "omschrijving": "Financiële rekeningen", "restricted": true}));
                }
                axum::Json(Value::Array(list))
            }),
        )
        .route(
            "/v1/api/grootboekrekening/{id}",
            get(|Path(id): Path<u64>| async move {
                axum::Json(match id {
                    45 => json!({"id": 45, "code": "4550", "omschrijving": "Bankkosten", "rekeningCategorie": "VW", "inactief": false, "rekeningGroepCode": "KOST"}),
                    _ => json!({"id": 11, "code": "1010", "omschrijving": "Bank", "rekeningCategorie": "FIN", "inactief": false}),
                })
            })
            .delete(deleted(c.clone(), "grootboekrekening")),
        )
        .route(
            "/v1/api/grootboekrekening",
            post(sent(c.clone(), "POST", "grootboekrekening", json!({"id": 46, "code": "4560", "omschrijving": "Telefoonkosten"})))
                .put(sent(c.clone(), "PUT", "grootboekrekening", Value::Null)),
        )
        .route(
            "/v1/api/rekeninggroep",
            get(|| async { axum::Json(json!([{"id": 7, "code": "KOST", "naam": "Kosten"}, {"id": 8, "code": "OMZ", "naam": "Omzet"}])) })
                .post(sent(c.clone(), "POST", "rekeninggroep", json!({"id": 9, "code": "REIS", "naam": "Reiskosten"})))
                .put(sent(c.clone(), "PUT", "rekeninggroep", Value::Null)),
        )
        .route(
            "/v1/api/rekeninggroep/gridtable",
            get(|| async {
                axum::Json(json!({"colMetadata": [{"name": "id"}, {"name": "code"}, {"name": "naam"}], "data": [[7, "KOST", "Kosten"], [8, "OMZ", "Omzet"]]}))
            }),
        )
        .route(
            "/v1/api/rekeninggroep/{id}",
            get(|Path(id): Path<u64>| async move { axum::Json(json!({"id": id, "code": "KOST", "naam": "Kosten"})) })
                .delete(deleted(c.clone(), "rekeninggroep")),
        )
        .route(
            "/v1/api/kostenplaats",
            get(|| async {
                axum::Json(json!([
                    {"id": 501, "omschrijving": "Projecten", "parentKostenplaatsId": 0, "actief": true, "volledigPad": "Projecten"},
                    {"id": 502, "omschrijving": "Noord", "parentKostenplaatsId": 501, "actief": true, "volledigPad": "Projecten>Noord"},
                    {"id": 503, "omschrijving": "Oud", "parentKostenplaatsId": 0, "actief": false, "volledigPad": "Oud"}
                ]))
            })
            .post(sent(c.clone(), "POST", "kostenplaats", json!({"id": 504})))
            .put(sent(c.clone(), "PUT", "kostenplaats", Value::Null)),
        )
        .route(
            "/v1/api/kostenplaats/{id}",
            get(|Path(id): Path<u64>| async move { axum::Json(json!({"id": id, "omschrijving": "Noord", "parentKostenplaatsId": 501, "actief": true, "volledigPad": "Projecten>Noord"})) })
                .delete(deleted(c.clone(), "kostenplaats")),
        )
        .route(
            "/v1/api/setting/administratie.use-kostenplaatsen",
            get(move |RawQuery(query): RawQuery| {
                let aan = kostenplaatsen_aan.load(Ordering::SeqCst);
                async move {
                    assert_eq!(query.as_deref(), Some("acc=false"));
                    axum::Json(json!({"name": "administratie.use-kostenplaatsen", "value": aan.to_string()}))
                }
            }),
        )
        .route(
            "/v1/api/btwinfo/all",
            get(|| async { axum::Json(json!([{"code": "GEEN", "omschrijving": "Geen btw", "percentage": 0, "soort": ""}])) }),
        )
        .route(
            "/v1/api/setting/administratie.use-rgs",
            get(|| async { axum::Json(json!({"name": "administratie.use-rgs", "value": "nee"})) }),
        )
        .route(
            "/v1/api/mutatie",
            post(sent(c.clone(), "POST", "mutatie", json!({"mutNr": 3909, "mutId": 4770877557u64})))
                .put(sent(c.clone(), "PUT", "mutatie", json!({"mutNr": 3908, "mutId": 4770877558u64}))),
        )
        .route(
            "/v1/api/mutatie/nr/3908/model",
            // A mutatie whose one line has kostenplaats 502 (Projecten>Noord).
            get(|| async {
                axum::Json(json!({
                    "folderKoppelId": 3908,
                    "mutatie": {"betalingsKenmerk": "", "boekstuk": "", "datum": "2026-11-06T00:00:00", "factuur": "", "inEx": "IN", "nr": 3908,
                                "omschrijving": "Parkeergarage", "rekening": 11, "relatieId": 0, "soort": 6, "termijn": 0},
                    "mutatieRegels": [{"bedrag": 2, "btw": 0, "btwCode": "GEEN", "index": 0, "kostenplaats": 502, "omschrijving": "Parkeergarage", "tegenRekening": 45}]
                }))
            }),
        )
        .fallback(stray.clone())
        .method_not_allowed_fallback(stray);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://127.0.0.1:{port}/v1/api/")
}

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// The MCP server with one signed-in account, and a client session on it.
struct Harness {
    state: Arc<AppState>,
    account: Account,
    changes: Changes,
    kostenplaatsen_aan: Arc<AtomicBool>,
    http: reqwest::Client,
    url: String,
    session: Option<String>,
}

fn input(mode: Rights) -> AccountInput {
    AccountInput {
        label: "Studio Noord".into(),
        username: "lisa@studionoord.example".into(),
        store: Store::System,
        op_ref: None,
        op_title: None,
        op_vault: None,
        rights: mode,
    }
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

impl Harness {
    async fn new() -> Self {
        let changes: Changes = Arc::default();
        let kostenplaatsen_aan = Arc::new(AtomicBool::new(true));
        let jar = Arc::new(crate::state::testing::FakeJar::default());
        let state = Arc::new(crate::state::testing::state_with(
            jar,
            &eboekhouden(changes.clone(), kostenplaatsen_aan.clone()).await,
        ));
        let account = state.accounts.add(input(Rights::OFF)).unwrap();
        state.jar_put("sessie", vec![("auth-token".into(), "jwt".into())]);
        state.add_session(account.id, &account.label, "sessie");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let mcp = crate::mcp::router(state.clone(), Arc::new(RwLock::new(TOKEN.to_owned())));
        tokio::spawn(async move { axum::serve(listener, mcp).await.unwrap() });
        let mut harness = Self {
            state,
            account,
            changes,
            kostenplaatsen_aan,
            http: reqwest::Client::new(),
            url: format!("http://127.0.0.1:{port}{}", crate::mcp::PATH),
            session: None,
        };
        let init = harness
            .post(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}}))
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

    async fn call(&self, tool: &str, arguments: Value) -> (bool, String) {
        let body = self
            .post(json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": {"name": tool, "arguments": arguments}}))
            .await
            .text()
            .await
            .unwrap();
        answer_of(&body)
    }

    fn set_write(&self, mode: Rights) {
        self.state
            .accounts
            .update(self.account.id, input(mode))
            .unwrap();
    }

    /// Nothing went to a path or method the stand-in does not know.
    fn no_strays(&self) {
        let changes = self.changes();
        assert!(
            !changes.iter().any(|(what, _)| what.starts_with("STRAY")),
            "{changes:?}"
        );
    }

    fn changes(&self) -> Vec<(String, Value)> {
        self.changes.lock().unwrap().clone()
    }

    /// The one proposal that is up, checked against its title and whether it always asks.
    fn up(&self, title: &str, danger: bool) -> Proposal {
        let list = self.state.approvals.list();
        assert_eq!(list.len(), 1, "{list:?}");
        let proposal = list.into_iter().next().unwrap();
        assert_eq!(proposal.title, title);
        assert_eq!(proposal.danger, danger, "{title}");
        proposal
    }

    /// Answers the one proposal that is up and waits for its background run to end.
    async fn decide(&self, approve: bool, title: &str, danger: bool) -> (Proposal, Status) {
        let proposal = self.up(title, danger);
        assert!(self.state.approvals.decide(proposal.id, approve, 1));
        for _ in 0..500 {
            if let Some(detail) = self.state.approvals.get(proposal.id)
                && !matches!(detail.record.status, Status::Waiting | Status::Running)
                && let Some(crate::reqlog::Entry::Mcp(entry)) =
                    detail.log_id.and_then(|id| self.state.log.get(id))
                && !matches!(
                    entry.outcome,
                    crate::reqlog::Outcome::Waiting | crate::reqlog::Outcome::Running
                )
            {
                return (proposal, detail.record.status);
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("proposal {} did not settle", proposal.id);
    }
}

fn change(what: &str, body: Value) -> (String, Value) {
    (what.to_owned(), body)
}

fn waiting(text: &str) {
    let value: Value = serde_json::from_str(text).unwrap_or_else(|_| panic!("not JSON: {text}"));
    assert_eq!(value["status"], "wacht", "{text}");
}

fn has_line(proposal: &Proposal, label: &str, value: &str) -> bool {
    proposal
        .lines
        .iter()
        .any(|l| l.label == label && l.value.contains(value))
}

#[tokio::test]
async fn grootboekrekeningen_change_only_after_approval() {
    let h = Harness::new().await;

    // Reading needs no writing, and finds a rekening by its code.
    let (is_error, text) = h.call("grootboekrekening", json!({"code": "4550"})).await;
    assert!(!is_error, "{text}");
    assert!(text.contains("\"rekeningGroepCode\":\"KOST\""), "{text}");
    let (is_error, text) = h.call("grootboekrekening", json!({"code": "7777"})).await;
    assert!(is_error && text.contains("7777"), "{text}");
    let (is_error, text) = h.call("grootboekrekening_categorieen", json!({})).await;
    assert!(!is_error && text.contains("FIN"), "{text}");

    // Writing is off: refused, nothing asked or sent.
    let add = json!({"code": "4560", "omschrijving": "Telefoonkosten", "groep": "KOST"});
    let (is_error, text) = h.call("grootboekrekening_toevoegen", add.clone()).await;
    assert!(is_error && text.contains("alleen-lezen"), "{text}");
    assert_eq!(h.state.approvals.count(), 0);

    // Bad input is refused before anything is asked.
    h.set_write(Rights::ASK);
    let (is_error, text) = h
        .call(
            "grootboekrekening_toevoegen",
            json!({"code": "4550", "omschrijving": "Dubbel"}),
        )
        .await;
    assert!(
        is_error && text.contains("al een grootboekrekening"),
        "{text}"
    );
    assert_eq!(h.state.approvals.count(), 0);

    // Refused by the user: nothing goes out.
    let (is_error, text) = h.call("grootboekrekening_toevoegen", add.clone()).await;
    assert!(!is_error, "{text}");
    waiting(&text);
    let (_, status) = h
        .decide(
            false,
            "Grootboekrekening toevoegen: 4560 Telefoonkosten",
            false,
        )
        .await;
    assert_eq!(status, Status::Refused);
    assert!(h.changes().is_empty());

    // Approved: exactly the form's request.
    h.call("grootboekrekening_toevoegen", add).await;
    let (proposal, status) = h
        .decide(
            true,
            "Grootboekrekening toevoegen: 4560 Telefoonkosten",
            false,
        )
        .await;
    assert_eq!(status, Status::Done);
    assert!(
        has_line(&proposal, "Groep", "KOST Kosten"),
        "{:?}",
        proposal.lines
    );
    assert_eq!(
        h.changes(),
        [change(
            "POST grootboekrekening",
            json!({"code":"4560","inactief":false,"omschrijving":"Telefoonkosten","rekeningCategorie":"VW","rekeningGroepCode":"KOST"})
        )]
    );

    // A plain edit follows the account's setting; without approval it goes straight through.
    h.set_write(Rights::AUTO);
    let (is_error, text) = h
        .call(
            "grootboekrekening_wijzigen",
            json!({"code": "4550", "omschrijving": "Bank- en betaalkosten"}),
        )
        .await;
    assert!(!is_error && text.contains("gewijzigd"), "{text}");
    assert_eq!(
        h.changes()[1],
        change(
            "PUT grootboekrekening",
            json!({"code":"4550","id":45,"inactief":false,"omschrijving":"Bank- en betaalkosten","rekeningCategorie":"VW","rekeningGroepCode":"KOST"})
        )
    );

    // A new code always asks, even without approval set, and says why.
    let (is_error, text) = h
        .call(
            "grootboekrekening_wijzigen",
            json!({"id": 45, "nieuwe_code": "4551"}),
        )
        .await;
    assert!(!is_error, "{text}");
    waiting(&text);
    let (proposal, status) = h
        .decide(true, "Grootboekrekening wijzigen: 4550 Bankkosten", true)
        .await;
    assert_eq!(status, Status::Done);
    assert!(
        has_line(&proposal, "Let op", "altijd akkoord"),
        "{:?}",
        proposal.lines
    );
    assert_eq!(h.changes()[2].1["code"], "4551", "{:?}", h.changes());

    // A fixed category cannot change: refused before asking.
    let (is_error, text) = h
        .call(
            "grootboekrekening_wijzigen",
            json!({"code": "1010", "categorie": "BAL"}),
        )
        .await;
    assert!(is_error && text.contains("staat vast"), "{text}");

    // Deleting always asks.
    let (is_error, text) = h
        .call("grootboekrekening_verwijderen", json!({"code": "4550"}))
        .await;
    assert!(!is_error, "{text}");
    waiting(&text);
    let (_, status) = h
        .decide(true, "Grootboekrekening verwijderen: 4550 Bankkosten", true)
        .await;
    assert_eq!(status, Status::Done);
    assert_eq!(h.changes().last().unwrap().0, "DELETE grootboekrekening/45");
    h.no_strays();
    assert_eq!(h.changes().len(), 4);
}

#[tokio::test]
async fn rekeninggroepen_change_only_after_approval() {
    let h = Harness::new().await;
    let (is_error, text) = h.call("rekeninggroepen", json!({})).await;
    assert!(!is_error && text.contains("\"code\":\"OMZ\""), "{text}");

    h.set_write(Rights::ASK);
    let (is_error, text) = h
        .call(
            "rekeninggroep_toevoegen",
            json!({"code": "KOST", "naam": "Dubbel"}),
        )
        .await;
    assert!(is_error && text.contains("al een rekeninggroep"), "{text}");
    h.call(
        "rekeninggroep_toevoegen",
        json!({"code": "REIS", "naam": "Reiskosten"}),
    )
    .await;
    let (_, status) = h
        .decide(false, "Rekeninggroep toevoegen: REIS Reiskosten", false)
        .await;
    assert_eq!(status, Status::Refused);
    assert!(h.changes().is_empty());
    h.call(
        "rekeninggroep_toevoegen",
        json!({"code": "REIS", "naam": "Reiskosten"}),
    )
    .await;
    h.decide(true, "Rekeninggroep toevoegen: REIS Reiskosten", false)
        .await;
    assert_eq!(
        h.changes(),
        [change(
            "POST rekeninggroep",
            json!({"code":"REIS","naam":"Reiskosten"})
        )]
    );

    // A new name follows the setting; a new code always asks.
    h.set_write(Rights::AUTO);
    let (is_error, text) = h
        .call(
            "rekeninggroep_wijzigen",
            json!({"id": 7, "naam": "Algemene kosten"}),
        )
        .await;
    assert!(!is_error, "{text}");
    assert_eq!(
        h.changes()[1],
        change(
            "PUT rekeninggroep",
            json!({"code":"KOST","id":7,"naam":"Algemene kosten"})
        )
    );
    // An id the administration does not know: said plainly, nothing asked or sent.
    for tool in ["rekeninggroep_wijzigen", "rekeninggroep_verwijderen"] {
        let (is_error, text) = h.call(tool, json!({"id": 99, "naam": "x"})).await;
        assert!(
            is_error && text.contains("Geen rekeninggroep met id 99"),
            "{tool}: {text}"
        );
    }
    assert_eq!(h.state.approvals.count(), 0);
    assert_eq!(h.changes().len(), 2);

    // A new code always asks, and says how many rekeningen are in the group.
    h.call("rekeninggroep_wijzigen", json!({"id": 7, "code": "ALG"}))
        .await;
    let (proposal, _) = h
        .decide(true, "Rekeninggroep wijzigen: KOST Kosten", true)
        .await;
    assert!(
        has_line(&proposal, "Rekeningen in deze groep", "1: 4550 Bankkosten"),
        "{:?}",
        proposal.lines
    );
    assert_eq!(
        h.changes()[2],
        change(
            "PUT rekeninggroep",
            json!({"code":"ALG","id":7,"naam":"Kosten"})
        )
    );

    h.call("rekeninggroep_verwijderen", json!({"id": 7})).await;
    let (proposal, _) = h
        .decide(true, "Rekeninggroep verwijderen: KOST Kosten", true)
        .await;
    assert!(
        has_line(&proposal, "Rekeningen in deze groep", "1: 4550 Bankkosten"),
        "{:?}",
        proposal.lines
    );
    assert_eq!(h.changes().last().unwrap().0, "DELETE rekeninggroep/7");
    h.no_strays();
}

#[tokio::test]
async fn kostenplaatsen_change_only_after_approval_and_go_on_a_mutatie_line() {
    let h = Harness::new().await;
    let (is_error, text) = h.call("kostenplaatsen", json!({"zoek": "noord"})).await;
    assert!(!is_error, "{text}");
    let answer: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(answer["data"].as_array().unwrap().len(), 1, "{text}");
    let (is_error, text) = h.call("kostenplaats", json!({"id": 502})).await;
    assert!(!is_error && text.contains("Projecten>Noord"), "{text}");

    h.set_write(Rights::ASK);
    let (is_error, text) = h
        .call(
            "kostenplaats_toevoegen",
            json!({"omschrijving": "X", "onder_id": 503}),
        )
        .await;
    assert!(is_error && text.contains("inactieve"), "{text}");
    h.call(
        "kostenplaats_toevoegen",
        json!({"omschrijving": "Groningen", "onder_id": 502}),
    )
    .await;
    let (_, status) = h
        .decide(
            false,
            "Kostenplaats toevoegen: Projecten>Noord>Groningen",
            false,
        )
        .await;
    assert_eq!(status, Status::Refused);
    assert!(h.changes().is_empty());
    h.call(
        "kostenplaats_toevoegen",
        json!({"omschrijving": "Groningen", "onder_id": 502}),
    )
    .await;
    h.decide(
        true,
        "Kostenplaats toevoegen: Projecten>Noord>Groningen",
        false,
    )
    .await;
    assert_eq!(
        h.changes(),
        [change(
            "POST kostenplaats",
            json!({"actief":true,"omschrijving":"Groningen","parentKostenplaatsId":502,"volledigPad":"Projecten>Noord>Groningen"})
        )]
    );

    h.call(
        "kostenplaats_wijzigen",
        json!({"id": 502, "inactief": true}),
    )
    .await;
    h.decide(true, "Kostenplaats wijzigen: Projecten>Noord", false)
        .await;
    assert_eq!(
        h.changes()[1],
        change(
            "PUT kostenplaats",
            json!({"actief":false,"id":502,"omschrijving":"Noord","volledigPad":"Projecten>Noord"})
        )
    );

    // Deleting always asks, also without approval set.
    h.set_write(Rights::AUTO);
    let (_, text) = h.call("kostenplaats_verwijderen", json!({"id": 501})).await;
    waiting(&text);
    let (proposal, _) = h
        .decide(true, "Kostenplaats verwijderen: Projecten", true)
        .await;
    assert!(
        has_line(&proposal, "Eronder", "Projecten>Noord"),
        "{:?}",
        proposal.lines
    );
    assert_eq!(h.changes()[2].0, "DELETE kostenplaats/501");

    // A mutatie line with a kostenplaats: checked against the administration's, sent as the mutatie form sends it.
    let boeking = |kostenplaats: u64| {
        json!({"soort": "geld_uitgegeven", "datum": "2026-11-06", "rekening": "1010", "omschrijving": "Parkeergarage",
               "regels": [{"tegenrekening": "4550", "bedrag": 2, "kostenplaats_id": kostenplaats}]})
    };
    let (is_error, text) = h.call("mutatie_toevoegen", boeking(503)).await;
    assert!(is_error && text.contains("inactief"), "{text}");
    h.kostenplaatsen_aan.store(false, Ordering::SeqCst);
    let (is_error, text) = h.call("mutatie_toevoegen", boeking(502)).await;
    assert!(is_error && text.contains("geen kostenplaatsen"), "{text}");
    assert_eq!(h.changes().len(), 3, "nothing was booked");
    h.kostenplaatsen_aan.store(true, Ordering::SeqCst);
    let (is_error, text) = h.call("mutatie_toevoegen", boeking(502)).await;
    assert!(!is_error && text.contains("3909"), "{text}");
    assert_eq!(
        h.changes()[3],
        change(
            "POST mutatie",
            json!({"mutatie":{"datum":"2026-11-06","inEx":"IN","omschrijving":"Parkeergarage","rekening":11,"soort":6},"mutatieRegels":[{"bedrag":2.0,"btwCode":"GEEN","index":0,"kostenplaats":502,"tegenRekening":45}]})
        )
    );
    let history = &h.state.approvals.history(Some(1))[0];
    assert!(
        history
            .proposal
            .lines
            .iter()
            .any(|l| l.label == "Regel 1" && l.value.ends_with("kostenplaats Projecten>Noord")),
        "{:?}",
        history.proposal.lines
    );
    h.no_strays();
}

#[tokio::test]
async fn an_edited_mutatie_keeps_or_names_its_kostenplaats() {
    let h = Harness::new().await;
    h.set_write(Rights::AUTO);
    let last_lines = |h: &Harness| h.state.approvals.history(Some(1))[0].proposal.lines.clone();

    // A kept line keeps its kostenplaats and shows it by name; with kostenplaatsen turned off that still works,
    // since nothing new is named.
    h.kostenplaatsen_aan.store(false, Ordering::SeqCst);
    let (is_error, text) = h
        .call(
            "mutatie_wijzigen",
            json!({"nr": 3908, "datum": "2026-11-07"}),
        )
        .await;
    assert!(!is_error, "{text}");
    let (what, body) = &h.changes()[0];
    assert_eq!(what, "PUT mutatie");
    assert_eq!(body["mutatieRegels"][0]["kostenplaats"], 502, "{body}");
    assert_eq!(body["mutatie"]["datum"], "2026-11-07");
    assert!(
        last_lines(&h)
            .iter()
            .any(|l| l.label == "Regel 1" && l.value.ends_with("kostenplaats Projecten>Noord")),
        "{:?}",
        last_lines(&h)
    );

    // A named one needs kostenplaatsen turned on.
    let named = json!({"nr": 3908, "regels": [{"tegenrekening": "4550", "bedrag": 2, "kostenplaats_id": 501}]});
    let (is_error, text) = h.call("mutatie_wijzigen", named.clone()).await;
    assert!(is_error && text.contains("geen kostenplaatsen"), "{text}");
    assert_eq!(h.changes().len(), 1);

    // New lines with another kostenplaats: sent, and the approval says the old one lapses.
    h.kostenplaatsen_aan.store(true, Ordering::SeqCst);
    let (is_error, text) = h.call("mutatie_wijzigen", named).await;
    assert!(!is_error, "{text}");
    assert_eq!(h.changes()[1].1["mutatieRegels"][0]["kostenplaats"], 501);
    let lines = last_lines(&h);
    assert!(
        lines
            .iter()
            .any(|l| l.label == "Regel 1" && l.value.ends_with("kostenplaats Projecten")),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.label == "Kostenplaats" && l.value == "kostenplaats Projecten>Noord vervalt"),
        "{lines:?}"
    );

    // 0 is no kostenplaats: nothing is checked, the line goes without one, and the old one lapses.
    let (is_error, text) = h
        .call("mutatie_wijzigen", json!({"nr": 3908, "regels": [{"tegenrekening": "4550", "bedrag": 2, "kostenplaats_id": 0}]}))
        .await;
    assert!(!is_error, "{text}");
    assert!(
        h.changes()[2].1["mutatieRegels"][0]
            .get("kostenplaats")
            .is_none()
    );
    assert!(
        last_lines(&h)
            .iter()
            .any(|l| l.value == "kostenplaats Projecten>Noord vervalt"),
        "{:?}",
        last_lines(&h)
    );
    assert_eq!(h.changes().len(), 3);
    h.no_strays();
}
