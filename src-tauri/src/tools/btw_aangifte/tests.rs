//! The proof that this part never files, and its tools end to end against a stand-in for e-Boekhouden.

use super::*;
use std::sync::{Arc, Mutex, RwLock};

/// The source of this part without its tests, without the list of filing paths and, with `requests`, without
/// the two functions that make its requests.
fn source(requests: bool) -> String {
    fn cut(text: &str, start: &str, end: &str) -> String {
        let (Some(a), Some(b)) = (text.find(start), text.find(end)) else {
            panic!("marker {start} or {end} is gone");
        };
        format!("{}{}", &text[..a], &text[b + end.len()..])
    }
    fn without_tests(text: &str) -> &str {
        text.split("#[cfg(test)]\nmod tests {").next().unwrap()
    }
    let mut main = cut(
        include_str!("../btw_aangifte.rs"),
        "// filing-paths:start",
        "// filing-paths:end",
    );
    if requests {
        main = cut(&main, "// requests:start", "// requests:end");
    }
    [
        main.as_str(),
        without_tests(include_str!("omzet.rs")),
        without_tests(include_str!("icp.rs")),
    ]
    .concat()
}

#[test]
fn refuses_every_path_that_files_pays_or_confirms() {
    for path in [
        "btw-aangifte/createandsend",
        "/Suppletie/CreateAndSend/",
        "suppletie/createandsend",
        "icpopgave/dosend/2026/K/3",
        "btw-aangifte-be/verwerk",
        "btw-aangifte-be/download",
        "btw-aangifte-be/bevestigen/4",
        "btw-aangifte-be/annuleren/4",
        "aangifte/pay/12",
        "aangifte/paystatus/abc",
        "aangifte/12/andersbetaald",
    ] {
        let refused = refuse_filing(path).unwrap_err();
        assert!(
            refused.contains("Er is niets verstuurd"),
            "{path}: {refused}"
        );
    }
    for path in [
        "btw-aangifte/verwerk",
        "suppletie/verwerk",
        "btw-aangifte/aangifte",
        "btw-aangifte/correcties/toggleignore",
        "icpopgave/regels",
        "icpopgave",
        "aangifte/gridtable",
        "aangifte/12",
    ] {
        assert_eq!(refuse_filing(path), Ok(()), "{path}");
    }
}

/// Every request of this part goes through the two guarded functions, and no path that files is written
/// anywhere else in the part: no tool can reach one.
#[test]
fn the_source_names_no_filing_path_and_makes_no_unguarded_request() {
    let code = source(false).to_ascii_lowercase();
    for part in FILING {
        assert!(
            !code.contains(part),
            "{part} appears outside the list of filing paths"
        );
    }
    let code = source(true);
    for call in [
        ".fetch(",
        ".change(",
        ".change_done(",
        ".simple(",
        "api_send",
        "api_get",
        "api_download",
        "api_editor",
        ".first_active(",
        ".with_items(",
        ".books(",
        ".factuur_context(",
    ] {
        assert!(
            !code.contains(call),
            "{call} makes a request around btwaangifte_get and btwaangifte_send"
        );
    }
}

// ---------------------------------------------------------------- end to end

/// What the stand-in was asked: method, path under /v1/api/ (with its query), body.
type Seen = Arc<Mutex<Vec<(String, String, Value)>>>;

fn grid_of(names: &[&str], rows: Value) -> Value {
    let count = rows.as_array().map(Vec::len).unwrap_or(0);
    json!({"colMetadata": names.iter().map(|n| json!({"name": n})).collect::<Vec<_>>(), "data": rows, "rowCount": count, "limit": 200, "offset": 0})
}

/// A stand-in for e-Boekhouden's btw screens. It answers the filing paths too, so a request to one would be
/// seen; the test checks that none comes.
async fn btw_mock(seen: Seen) -> String {
    use axum::http::{Method, StatusCode, Uri};
    use axum::response::IntoResponse;
    let app = axum::Router::new().fallback(move |method: Method, uri: Uri, body: axum::body::Bytes| {
        let seen = seen.clone();
        async move {
            let path = uri.path().trim_start_matches("/v1/api/").to_owned();
            let full = match uri.query() {
                Some(q) => format!("{path}?{q}"),
                None => path.clone(),
            };
            let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            seen.lock().unwrap().push((method.to_string(), full, body));
            let empty = || grid_of(&["id"], json!([]));
            let answer = match (method.as_str(), path.as_str()) {
                ("GET", "aangifte/gridtable") => grid_of(
                    &["id", "datum", "soort", "periode", "status"],
                    json!([[5, "2026-10-20", "OB", "3e kwartaal 2026", "Geaccepteerd"],
                           [6, "2026-11-01", "SUP", "2e kwartaal 2026", "Geaccepteerd"],
                           [9, "2026-10-21", "ICP", "3e kwartaal 2026", "Geaccepteerd"],
                           [12, "2026-10-21", "Btw", "3e kwartaal 2026", ""]]),
                ),
                ("GET", "aangifte/5") => json!({"id": 5, "periode": "3e kwartaal 2026"}),
                ("GET", "aangifte/aangifteomzetbelasting/5" | "aangifte/aangifteomzetbelasting/6") => {
                    json!({"berekening": {"voorbelasting": 600}})
                }
                ("GET", "suppletie/getSuppletieDetails/5" | "suppletie/getSuppletieDetails/6") => json!({"id": 6}),
                ("GET", "suppletie/canSuppletie/5") => json!({"succes": true, "melding": ""}),
                ("GET", "suppletie/canSuppletie/7") => json!({"succes": false, "melding": "Er loopt al een suppletie"}),
                ("GET", "suppletie/getSuppletie/5") => json!({
                    "periodDescription": "3e kwartaal 2026", "saldoAlAangegeven": 1500,
                    "verzondenAangifte": {"identificerendeGegevens": {"aangiftejaar": "2026", "aangiftetijdvak": "27"}},
                    "rubrieken": [{"rubriekCode": "1a", "btwVoorgesteld": 2100}, {"rubriekCode": "5b", "btwVoorgesteld": 400}]
                }),
                ("GET", "btw-aangifte/aangifte") => json!({
                    "periodDescription": "3e kwartaal 2026", "eaStatus": null, "bhDatum": "",
                    "rubrieken": [{"rubriekCode": "1a", "btwVoorgesteld": 2100}, {"rubriekCode": "1b", "btwVoorgesteld": 90},
                                  {"rubriekCode": "5b", "btwVoorgesteld": 630}]
                }),
                ("GET", "mutatie/btw-specificatie/gridtable") => grid_of(
                    &["mutatieNr", "datum", "omschrijving", "btw"],
                    json!([[3908, "2026-08-02", "Factuur 12", "HOOG_VERK_21"]]),
                ),
                ("GET", "btw-aangifte/correcties/gridtable") => grid_of(
                    &["period", "periodCode", "omzetVerschil", "btwVerschil", "verschilGenegeerd"],
                    json!([["1e kwartaal 2026", "261", 500, 105, false]]),
                ),
                ("GET", "icpopgave/2026/K/3") => json!({"id": 55, "eaPeriod": "2026K3", "jaar": 2026, "periodType": "K", "period": 3,
                                                        "btwNummer": "NL001234567B01", "bedrijf": "Studio Noord", "contactpersoon": "Lisa",
                                                        "telefoon": "", "onderdeelFiscaleEenheid": ""}),
                ("GET", "icpopgave/9") => json!({"id": 9, "jaar": 2026, "periodType": "K", "period": 3}),
                ("GET", "icpopgave/regels/2026K3/1A/gridtable") => grid_of(
                    &["id", "tijdvak", "land", "btwNummer", "bedrag"],
                    json!([[77, "1e kwartaal 2026", "Duitsland (DE)", "DE123456789", 100]]),
                ),
                ("GET", "icpopgave/regels/77") => json!({"id": 77, "bedrag": 100, "btwNummer": "DE123456789", "land": "DE",
                                                         "jaar": 2026, "periodType": "K", "periodId": 1}),
                ("GET", "icpopgave/landen") => json!([{"code": "DE", "land": "Duitsland", "einddatum": null},
                                                      {"code": "BE", "land": "Belgie", "einddatum": null}]),
                ("GET", p) if p.starts_with("icpopgave/regels/") || p.starts_with("aangifte/2026K3/") => empty(),
                ("POST", "btw-aangifte/verwerk") => json!([4001, 4002]),
                ("POST", "suppletie/verwerk") => json!([4003]),
                ("POST", "btw-aangifte/correcties/toggleignore") => json!(true),
                ("PUT", "icpopgave") => json!(true),
                ("POST", "icpopgave/regels") => json!({"id": 78}),
                ("PUT", "icpopgave/regels") => json!(true),
                ("DELETE", "icpopgave/regels/77") => json!(true),
                // The filing paths: answered, so a request would be seen.
                (_, p) if refuse_filing(p).is_err() => json!(true),
                _ => return StatusCode::NOT_FOUND.into_response(),
            };
            axum::Json(answer).into_response()
        }
    });
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

fn waiting_id(text: &str) -> u64 {
    let value: Value = serde_json::from_str(text).unwrap_or_else(|_| panic!("not JSON: {text}"));
    assert_eq!(value["status"], "wacht", "{text}");
    value["voorstel_id"].as_u64().unwrap()
}

async fn settled(state: &crate::state::AppState, id: u64) -> approvals::Record {
    use crate::approvals::Status;
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
async fn reads_draws_up_and_corrects_only_after_approval_and_never_files() {
    use crate::accounts::{AccountInput, Store, WriteMode};
    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let seen: Seen = Arc::default();
    let jar = Arc::new(crate::state::testing::FakeJar::default());
    let state = Arc::new(crate::state::testing::state_with(
        jar,
        &btw_mock(seen.clone()).await,
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
    let account = state.accounts.add(input(WriteMode::Off)).unwrap();
    state.jar_put("sessie", vec![("auth-token".into(), "jwt".into())]);
    state.add_session(account.id, &account.label, "sessie");
    let set_write = |mode: WriteMode| state.accounts.update(account.id, input(mode)).unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mcp = crate::mcp::router(state.clone(), Arc::new(RwLock::new(TOKEN.to_owned())));
    tokio::spawn(async move { axum::serve(listener, mcp).await.unwrap() });
    let http = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}{}", crate::mcp::PATH);
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
            json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": {"name": tool, "arguments": arguments}}),
            session.clone(),
        );
        async move { answer_of(&request.await.unwrap().text().await.unwrap()) }
    };
    let changes = |seen: &Seen| -> Vec<(String, String, Value)> {
        seen.lock()
            .unwrap()
            .iter()
            .filter(|(method, _, _)| method != "GET")
            .cloned()
            .collect()
    };

    // Reading works on a read-only account.
    let (is_error, text) = call("btwaangiftes", json!({})).await;
    assert!(!is_error && text.contains("\"soort\":\"OB\""), "{text}");
    let (is_error, text) = call("suppleties", json!({})).await;
    assert!(!is_error, "{text}");
    let list: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(list["data"]["data"].as_array().unwrap().len(), 1);
    assert_eq!(list["data"]["data"][0]["id"], 6);
    let (is_error, text) = call("icp_opgaven", json!({})).await;
    assert!(!is_error, "{text}");
    let list: Value = serde_json::from_str(&text).unwrap();
    let ids: Vec<&Value> = list["data"]["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| &r["id"])
        .collect();
    assert_eq!(ids, [&json!(9)], "the Belgian return is not an ICP-opgave");
    let (is_error, text) = call("btwaangifte", json!({"id": 5})).await;
    assert!(!is_error && text.contains("voorbelasting"), "{text}");
    let (is_error, text) = call(
        "btwaangifte_mutaties",
        json!({"jaar": 2026, "periode": 3, "rubriek": "1a"}),
    )
    .await;
    assert!(!is_error && text.contains("\"mutatieNr\":3908"), "{text}");
    assert!(seen.lock().unwrap().iter().any(|(_, p, _)| {
        p.starts_with("mutatie/btw-specificatie/gridtable")
            && p.contains("periodStart=20260701")
            && p.contains("periodEnd=20260930")
            && p.contains("category=1a")
    }));
    let (is_error, text) = call(
        "btwaangifte_correcties",
        json!({"jaar": 2026, "periode": 3, "rubriek": "1a"}),
    )
    .await;
    assert!(
        !is_error && text.contains("\"periodCode\":\"261\""),
        "{text}"
    );
    let (is_error, text) = call("suppletie", json!({"aangifte_id": 5})).await;
    assert!(
        !is_error && text.contains("\"kan_suppletie\":true"),
        "{text}"
    );
    let (is_error, text) = call("suppletie", json!({"aangifte_id": 7})).await;
    assert!(
        !is_error && text.contains("Er loopt al een suppletie"),
        "{text}"
    );
    let (is_error, _) = call("suppletie", json!({"id": 6, "aangifte_id": 5})).await;
    assert!(is_error);
    let (is_error, text) = call(
        "icp_opgave",
        json!({"jaar": 2026, "periode_type": "kwartaal", "periode": 3}),
    )
    .await;
    assert!(
        !is_error && text.contains("\"totaal\":100.0") && text.contains("\"verzendingen\""),
        "{text}"
    );
    let (is_error, text) = call("icp_opgave", json!({"id": 9})).await;
    assert!(!is_error && text.contains("\"3b\""), "{text}");
    assert!(changes(&seen).is_empty(), "reading changed nothing");

    // Writing is off: refused, nothing read or asked.
    let before = seen.lock().unwrap().len();
    let (is_error, text) = call("btwaangifte_verwerken", json!({"jaar": 2026, "periode": 3})).await;
    assert!(is_error && text.contains("alleen-lezen"), "{text}");
    assert_eq!(seen.lock().unwrap().len(), before);
    assert_eq!(state.approvals.count(), 0);

    // Even without approval set for the account, these always ask; refused: nothing goes out.
    set_write(WriteMode::Auto);
    let (is_error, text) = call(
        "btwaangifte_verwerken",
        json!({"jaar": 2026, "periode": 3, "btw_5b": 700}),
    )
    .await;
    assert!(!is_error, "{text}");
    let id = waiting_id(&text);
    let proposal = state
        .approvals
        .list()
        .into_iter()
        .find(|p| p.id == id)
        .unwrap();
    assert!(proposal.danger);
    assert_eq!(proposal.title, "Btw-aangifte verwerken: 3e kwartaal 2026");
    let lines: Vec<(String, String)> = proposal
        .lines
        .iter()
        .map(|l| (l.label.clone(), l.value.clone()))
        .collect();
    assert!(
        lines.contains(&("Periode".into(), "3e kwartaal 2026".into())),
        "{lines:?}"
    );
    assert!(
        lines.contains(&(
            "5b Voorbelasting (btw)".into(),
            "700,00 (voorstel van e-Boekhouden 630,00)".into()
        )),
        "{lines:?}"
    );
    assert!(
        lines.contains(&("Saldo (5g)".into(), "1490,00 te betalen".into())),
        "{lines:?}"
    );
    assert!(state.approvals.decide(id, false, 1));
    assert_eq!(settled(&state, id).await.status, approvals::Status::Refused);
    assert!(changes(&seen).is_empty());

    // Approved: exactly the screen's request.
    set_write(WriteMode::Ask);
    let mut approved = 0;
    let mut approve_last = |text: String| {
        approved += 1;
        let id = waiting_id(&text);
        assert!(
            state
                .approvals
                .list()
                .iter()
                .any(|p| p.id == id && p.danger)
        );
        assert!(state.approvals.decide(id, true, 10 + approved));
        id
    };
    let (_, text) = call("btwaangifte_verwerken", json!({"jaar": 2026, "periode": 3})).await;
    assert!(changes(&seen).is_empty(), "nothing before the answer");
    let id = approve_last(text);
    assert_eq!(settled(&state, id).await.status, approvals::Status::Done);
    assert_eq!(
        changes(&seen),
        vec![(
            "POST".into(),
            "btw-aangifte/verwerk".into(),
            json!({
                "s1aBtw": 2100, "s1bBtw": 90, "s1cBtw": 0, "s1dBtw": 0, "s2aBtw": 0, "s4aBtw": 0, "s4bBtw": 0,
                "s5bBtw": 630, "s5dBtw": 0, "periodYear": 2026, "periodType": "K", "periodId": 3
            })
        )]
    );

    let (_, text) = call("suppletie_verwerken", json!({"aangifte_id": 5})).await;
    let id = approve_last(text);
    assert_eq!(settled(&state, id).await.status, approvals::Status::Done);
    let (_, text) = call(
        "btwaangifte_verschil_negeren",
        json!({"jaar": 2026, "periode": 3, "rubriek": "1a", "periode_code": "261"}),
    )
    .await;
    let id = approve_last(text);
    assert_eq!(settled(&state, id).await.status, approvals::Status::Done);
    let (_, text) = call(
        "icp_opgave_wijzigen",
        json!({"jaar": 2026, "periode_type": "kwartaal", "periode": 3, "telefoon": "0612345678"}),
    )
    .await;
    let id = approve_last(text);
    assert_eq!(settled(&state, id).await.status, approvals::Status::Done);

    // A bad line is refused before anything is asked or sent.
    let asked = state.approvals.history(None).len();
    let (is_error, text) = call("icp_regel_toevoegen", json!({"jaar": 2026, "periode_type": "kwartaal", "periode": 3, "rubriek": "1a",
        "land": "US", "btw_nummer": "US12345", "bedrag": 10, "correctie_jaar": 2026, "correctie_periode_type": "kwartaal", "correctie_periode": 1})).await;
    assert!(is_error && text.contains("Onbekende landcode"), "{text}");
    assert_eq!(state.approvals.history(None).len(), asked);

    let (_, text) = call("icp_regel_toevoegen", json!({"jaar": 2026, "periode_type": "kwartaal", "periode": 3, "rubriek": "1a",
        "land": "de", "btw_nummer": "DE987654321", "bedrag": -40, "correctie_jaar": 2026, "correctie_periode_type": "kwartaal", "correctie_periode": 2})).await;
    let id = approve_last(text);
    assert_eq!(settled(&state, id).await.status, approvals::Status::Done);
    let (_, text) = call(
        "icp_regel_wijzigen",
        json!({"jaar": 2026, "periode_type": "kwartaal", "periode": 3, "id": 77, "bedrag": 120}),
    )
    .await;
    let id = approve_last(text);
    assert_eq!(settled(&state, id).await.status, approvals::Status::Done);
    let (_, text) = call(
        "icp_regel_verwijderen",
        json!({"jaar": 2026, "periode_type": "kwartaal", "periode": 3, "id": 77}),
    )
    .await;
    let id = approve_last(text);
    assert_eq!(settled(&state, id).await.status, approvals::Status::Done);
    let (is_error, text) = call(
        "icp_regel_verwijderen",
        json!({"jaar": 2026, "periode_type": "kwartaal", "periode": 3, "id": 99}),
    )
    .await;
    assert!(is_error && text.contains("Regel 99"), "{text}");

    let changes = changes(&seen);
    assert_eq!(
        changes
            .iter()
            .map(|(m, p, _)| format!("{m} {p}"))
            .collect::<Vec<_>>(),
        [
            "POST btw-aangifte/verwerk",
            "POST suppletie/verwerk",
            "POST btw-aangifte/correcties/toggleignore",
            "PUT icpopgave",
            "POST icpopgave/regels",
            "PUT icpopgave/regels",
            "DELETE icpopgave/regels/77",
        ]
    );
    assert_eq!(
        changes[1].2,
        json!({"s1aBtw": 2100, "s1bBtw": 0, "s1cBtw": 0, "s1dBtw": 0, "s2aBtw": 0, "s4aBtw": 0, "s4bBtw": 0,
               "s5bBtw": 400, "s5dBtw": 0, "periodYear": 2026, "periodType": "K", "periodId": 3})
    );
    assert_eq!(
        changes[2].2,
        json!({"category": "1a", "periodCode": "261", "btw": 105, "omzet": 500, "delete": false})
    );
    assert_eq!(
        changes[3].2,
        json!({"id": 55, "btwNummer": "NL001234567B01", "bedrijf": "Studio Noord", "contactpersoon": "Lisa",
               "telefoon": "0612345678", "onderdeelFiscaleEenheid": ""})
    );
    assert_eq!(
        changes[4].2,
        json!({"bedrag": -40, "btwNummer": "DE987654321", "rubriek": "1A", "eaPeriode": "2026K3", "land": "DE",
               "jaar": "2026", "periodType": "K", "periodId": "2"})
    );
    assert_eq!(
        changes[5].2,
        json!({"id": 77, "bedrag": 120, "btwNummer": "DE123456789", "rubriek": "1A", "eaPeriode": "2026K3", "land": "DE",
               "jaar": "2026", "periodType": "K", "periodId": "1"})
    );

    // Not one request of the whole run went to a path that files, pays or confirms.
    for (method, path, _) in seen.lock().unwrap().iter() {
        assert_eq!(
            refuse_filing(path.split('?').next().unwrap()),
            Ok(()),
            "{method} {path}"
        );
    }
}
