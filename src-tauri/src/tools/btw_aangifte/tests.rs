//! The proof that this part never files, and its tools end to end against a stand-in for e-Boekhouden.

use super::*;
use std::sync::{Arc, Mutex, RwLock};

/// The paths that file, pay or confirm, written out here on their own, so the tests do not judge the guard
/// by the guard. A path is one of them when it starts with or contains one of these, lowercase.
const FILING_LITERAL: &[&str] = &[
    "btw-aangifte/createandsend",
    "suppletie/createandsend",
    "icpopgave/dosend/",
    "btw-aangifte-be/",
    "klantenlisting/process",
    "/bevestigen",
    "aangifte/pay/",
    "aangifte/paystatus",
    "/andersbetaald",
];

fn files_literally(path: &str) -> bool {
    let path = format!("/{}", path.to_ascii_lowercase().trim_start_matches('/'));
    FILING_LITERAL.iter().any(|part| path.contains(part))
}

/// The source of this part without its tests, without the list of filing paths and, with `requests`, without
/// the two functions that make its requests.
fn source(requests: bool) -> String {
    fn cut(text: &str, start: &str, end: &str) -> String {
        let (Some(a), Some(b)) = (text.find(start), text.find(end)) else {
            panic!("marker {start} or {end} is gone");
        };
        format!("{}{}", &text[..a], &text[b + end.len()..])
    }
    let mut main = cut(
        include_str!("../btw_aangifte.rs"),
        "// filing-paths:start",
        "// filing-paths:end",
    );
    main = if requests {
        cut(&main, "// requests:start", "// requests:end")
    } else {
        cut(&main, "// pay-link:start", "// pay-link:end")
    };
    [
        main.as_str(),
        without_tests(include_str!("omzet.rs")),
        without_tests(include_str!("icp.rs")),
        without_tests(include_str!("betaallink.rs")),
    ]
    .concat()
}

/// Every `self.<name>` and `this.<name>` in `code`, with what follows the name.
fn members(code: &str) -> Vec<String> {
    let bytes = code.as_bytes();
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut out = Vec::new();
    for word in ["self", "this"] {
        let mut from = 0;
        while let Some(at) = code[from..].find(word) {
            let start = from + at;
            from = start + word.len();
            if start > 0 && ident(bytes[start - 1]) {
                continue;
            }
            let mut i = from;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= bytes.len() || bytes[i] != b'.' {
                continue;
            }
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            let name_start = i;
            while i < bytes.len() && ident(bytes[i]) {
                i += 1;
            }
            out.push(code[name_start..i].to_owned());
        }
    }
    out
}

#[test]
fn the_list_of_returns_carries_no_link_to_pay_one() {
    let row = json!({"id": 7, "soort": "OB", "canPay": 1, "payUrl": "https://betalen.example/x", "lastPayStatus": null});
    let cleaned = without_pay_link(json!({"data": [row], "rowCount": 1}));
    assert!(cleaned["data"][0].get("payUrl").is_none(), "{cleaned}");
    // What says whether it was paid stays.
    assert_eq!(cleaned["data"][0]["canPay"], 1);
    assert_eq!(cleaned["rowCount"], 1);
}

#[test]
fn refuses_every_path_that_files_pays_or_confirms() {
    for path in [
        "btw-aangifte/createandsend",
        "/Suppletie/CreateAndSend/",
        "suppletie/createandsend",
        "icpopgave/dosend/2026/K/3",
        "btw-aangifte-be/verwerk",
        "btw-aangifte-be/download?jaar=2026",
        "btw-aangifte-be/bevestigen/4",
        "btw-aangifte-be/annuleren/4",
        "klantenlisting/process",
        "klantenlisting/2026/bevestigen",
        "aangifte/pay/12",
        "aangifte/paystatus/abc",
        "aangifte/paystatusgrid/12",
        "aangifte/12/andersbetaald",
    ] {
        assert!(files_literally(path), "the literal list misses {path}");
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
        "icpopgave/2026/K/3",
        "aangifte/gridtable",
        "aangifte/12",
        "klantenlisting/gridtable",
    ] {
        assert!(!files_literally(path), "{path}");
        assert_eq!(refuse_filing(path), Ok(()), "{path}");
    }
}

/// No path that files is written anywhere in the part but in its list; and the only calls that can reach
/// e-Boekhouden are the two guarded functions: every `self.`/`this.` in the part is on a short list of this
/// part's own helpers, the steps every tool takes, and fields of its own small types.
#[test]
fn the_source_names_no_filing_path_and_reaches_e_boekhouden_only_through_the_guard() {
    let code = source(false).to_ascii_lowercase();
    for pattern in FILING {
        let literal = pattern.split("{}").next().unwrap();
        assert!(
            !code.contains(literal),
            "{literal} appears outside the list of filing paths"
        );
    }

    let code = source(true);
    const ALLOWED: &[&str] = &[
        // The steps of every tool in tools.rs: logging, approval and the task; none of them calls e-Boekhouden.
        "begin",
        "end",
        "refuse",
        "write_tool",
        "approve",
        // The two guarded requests, and this part's helpers built on them only (their bodies are scanned too).
        "btwaangifte_get",
        "btwaangifte_send",
        "btwaangifte_rows",
        "btwaangifte_sent",
        "icp_opgave_van",
        "icp_regels",
        "icp_eigen_regel",
        "icp_opgave_inhoud",
        // The one pay request, checked on its own below.
        "btwaangifte_betaling_starten",
        // Fields and methods of this part's own value types (BtwPeriode, IcpPeriode, BtwBedragen).
        "jaar",
        "soort",
        "id",
        "months",
        "first_month",
        "get",
        "btw_1a",
        "btw_1b",
        "btw_1c",
        "btw_1d",
        "btw_2a",
        "btw_4a",
        "btw_4b",
        "btw_5b",
        "btw_5d",
    ];
    let found = members(&code);
    assert!(
        found.contains(&"btwaangifte_get".to_owned()),
        "the scan finds calls"
    );
    for name in &found {
        assert!(
            ALLOWED.contains(&name.as_str()),
            "self.{name} or this.{name} is not on the list of calls that cannot reach e-Boekhouden"
        );
    }
    for word in [
        "api_", "reqwest", "state.", "client.", ".fetch(", ".change(", ".change_", "save_pdf",
        "editor_",
    ] {
        let hits: Vec<&str> = code
            .lines()
            .filter(|l| l.contains(word) && !l.trim_start().starts_with("//"))
            .collect();
        assert!(hits.is_empty(), "{word} in {hits:?}");
    }
    let mut from = 0;
    while let Some(at) = code[from..].find("Self::") {
        let rest = &code[from + at + 6..];
        assert!(
            rest.starts_with("btwaangifte_omzet_router()")
                || rest.starts_with("btwaangifte_icp_router()")
                || rest.starts_with("btwaangifte_betaal_router()")
                || rest.starts_with("new(")
                || rest.chars().next().is_some_and(char::is_uppercase),
            "Self::{}",
            &rest[..rest.len().min(30)]
        );
        from += at + 6;
    }
    assert!(!code.contains("EbMcp::"), "a call by the type's name");
}

/// The one exception to the guard asks for `aangifte/pay/{id}` with a numeric id and nothing else, and only
/// the betaallink tool calls it, after its approval.
#[test]
fn the_pay_request_builds_one_path_and_runs_only_after_approval_in_its_tool() {
    let main = include_str!("../btw_aangifte.rs");
    let block =
        &main[main.find("// pay-link:start").unwrap()..main.find("// pay-link:end").unwrap()];
    let code: Vec<&str> = block
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("//"))
        .collect();
    assert_eq!(
        code,
        [
            "async fn btwaangifte_betaling_starten(&self, run: &Run, id: u64) -> Result<Value, Halt> {",
            "self.fetch(run, &format!(\"aangifte/pay/{id}\"), Vec::new())",
            ".await",
            ".map_err(Halt::Error)",
            "}",
        ],
        "the pay request changed: check it still asks for exactly one numeric path"
    );
    // Outside its block, the pay request is named once: in the betaallink tool, after the approval.
    let elsewhere = source(false);
    assert_eq!(
        elsewhere.matches("btwaangifte_betaling_starten").count(),
        3,
        "its mentions in the two module docs and its one call"
    );
    let tool = without_tests(include_str!("betaallink.rs"));
    let approve = tool
        .find("this.approve(&run, TOOL, &built, true)")
        .expect("the tool asks, with danger");
    let call = tool
        .find("this.btwaangifte_betaling_starten(")
        .expect("the tool makes the request");
    assert!(approve < call, "the request comes after the approval");
    assert_eq!(
        tool.matches("this.btwaangifte_betaling_starten(").count(),
        1
    );
    for other in [include_str!("omzet.rs"), include_str!("icp.rs")] {
        assert!(!other.contains("btwaangifte_betaling_starten"));
    }
}

fn without_tests(text: &str) -> &str {
    text.split("#[cfg(test)]\nmod tests {").next().unwrap()
}

// ---------------------------------------------------------------- end to end

/// What the stand-in was asked: method, path under /v1/api/ (with its query), body.
type Seen = Arc<Mutex<Vec<(String, String, Value)>>>;

fn grid_of(names: &[&str], rows: Value) -> Value {
    let count = rows.as_array().map(Vec::len).unwrap_or(0);
    json!({"colMetadata": names.iter().map(|n| json!({"name": n})).collect::<Vec<_>>(), "data": rows, "rowCount": count, "limit": 200, "offset": 0})
}

/// A query parameter of a request uri.
fn param(query: Option<&str>, key: &str) -> Option<String> {
    query?.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| v.to_owned())
    })
}

/// Rubriek 1a of the opgave of the third quarter: 201 lines, so a second page; line 77 is the last one, on it.
fn rubriek_1a(offset: usize) -> Value {
    let all: Vec<Value> = (0..201)
        .map(|n| {
            if n == 200 {
                json!([77, "1e kwartaal 2026", "Duitsland (DE)", "DE123456789", 100])
            } else {
                json!([
                    1000 + n,
                    "1e kwartaal 2026",
                    "Belgie (BE)",
                    "BE0123456789",
                    1
                ])
            }
        })
        .collect();
    let page: Vec<Value> = all.into_iter().skip(offset).take(200).collect();
    json!({"colMetadata": [{"name": "id"}, {"name": "tijdvak"}, {"name": "land"}, {"name": "btwNummer"}, {"name": "bedrag"}],
           "data": page, "rowCount": 201, "limit": 200, "offset": offset})
}

/// A stand-in for e-Boekhouden's btw screens. It answers the filing paths too, so a request to one would be
/// seen; the test checks that none comes.
async fn btw_mock(seen: Seen) -> String {
    use axum::http::{Method, StatusCode, Uri};
    use axum::response::IntoResponse;
    let app = axum::Router::new().fallback(
        move |method: Method, uri: Uri, headers: axum::http::HeaderMap, body: axum::body::Bytes| {
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
                let offset: usize = param(uri.query(), "offset")
                    .and_then(|o| o.parse().ok())
                    .unwrap_or(0);
                // Payment links point back at this stand-in, so a request to one would be seen.
                let host = headers
                    .get("host")
                    .and_then(|h| h.to_str().ok())
                    .unwrap_or("127.0.0.1")
                    .to_owned();
                let answer = match (method.as_str(), path.as_str()) {
                    ("GET", "aangifte/pay/5") => {
                        json!({"startPaymentUrl": format!("http://{host}/betaal/sessie-5")})
                    }
                    ("GET", "aangifte/pay/16") => json!({"startPaymentUrl": null}),
                    // The filing paths: answered, so a request would be seen.
                    (_, p) if files_literally(p) => json!(true),
                    ("GET", "aangifte/gridtable") => grid_of(
                        &["id", "datum", "soort", "periode", "status", "canPay", "iDeal", "payUrl", "lastPayStatus", "lastPayDate", "mustPayDiff"],
                        json!([[5, "2026-10-20", "OB", "3e kwartaal 2026", "Geaccepteerd", true, true, null, null, null, false],
                               [6, "2026-11-01", "SUP", "2e kwartaal 2026", "Geaccepteerd", true, true, format!("http://{host}/betaal/bestaand-6"), "Open", null, false],
                               [9, "2026-10-21", "ICP", "3e kwartaal 2026", "Geaccepteerd", false, false, null, null, null, false],
                               [12, "2026-10-21", "Btw", "3e kwartaal 2026", "", false, false, null, null, null, false],
                               [13, "2026-04-20", "OB", "1e kwartaal 2026", "Geaccepteerd", false, true, null, null, null, false],
                               [14, "2026-01-20", "OB", "4e kwartaal 2025", "Geaccepteerd", true, true, null, "Success", "20-01-2026 10:00", false],
                               [15, "2025-10-20", "OB", "3e kwartaal 2025", "Geaccepteerd", true, true, null, "Betaald", null, false],
                               [16, "2026-07-20", "OB", "2e kwartaal 2026", "Geaccepteerd", true, true, null, null, null, false]]),
                    ),
                    ("GET", "aangifte/5") => json!({"id": 5, "periode": "3e kwartaal 2026"}),
                    (
                        "GET",
                        "aangifte/aangifteomzetbelasting/5" | "aangifte/aangifteomzetbelasting/6",
                    ) => {
                        json!({"berekening": {"voorbelasting": 600, "totaalTeBetalenTerugTeVragen": 1234}})
                    }
                    (
                        "GET",
                        "suppletie/getSuppletieDetails/5" | "suppletie/getSuppletieDetails/6",
                    ) => json!({"id": 6}),
                    ("GET", "suppletie/canSuppletie/5") => json!({"succes": true, "melding": ""}),
                    ("GET", "suppletie/canSuppletie/7") => {
                        json!({"succes": false, "melding": "Er loopt al een suppletie"})
                    }
                    ("GET", "suppletie/getSuppletie/5") => json!({
                        "periodDescription": "3e kwartaal 2026", "saldoAlAangegeven": 1500,
                        "verzondenAangifte": {"identificerendeGegevens": {"aangiftejaar": "2026", "aangiftetijdvak": "27"}},
                        "rubrieken": [{"rubriekCode": "1a", "btwVoorgesteld": 2100}, {"rubriekCode": "5b", "btwVoorgesteld": 400}]
                    }),
                    ("GET", "btw-aangifte/aangifte")
                        if param(uri.query(), "periodId").as_deref() == Some("2") =>
                    {
                        // The second quarter was sent already, with other amounts than the books give now.
                        json!({
                            "periodDescription": "2e kwartaal 2026", "eaStatus": "acknowledgement", "bhDatum": "03-07-2026 10:00",
                            "rubrieken": [{"rubriekCode": "1a", "btwVoorgesteld": 2500}, {"rubriekCode": "5b", "btwVoorgesteld": 700}],
                            "verzondenAangifte": {
                                "doorUBinnenland": {"omzetbelastingHoog": "2400", "omzetbelastingLaag": "0"},
                                "aanUBinnenland": {"omzetbelastingVerlegd": 0},
                                "aanUBuitenland": {"omzetbelastingBuitenEUInvoer": 0, "omzetbelastingGoederenBinnenEU": 12},
                                "berekening": {"voorbelasting": "650", "kleineOndernemersregeling": 0}
                            }
                        })
                    }
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
                        &[
                            "period",
                            "periodCode",
                            "omzetVerschil",
                            "btwVerschil",
                            "verschilGenegeerd",
                        ],
                        json!([["1e kwartaal 2026", "261", 500, 105, false]]),
                    ),
                    ("GET", "icpopgave/2026/K/3") => {
                        json!({"id": 55, "eaPeriod": "2026K3", "jaar": 2026, "periodType": "K", "period": 3,
                                                        "btwNummer": "NL001234567B01", "bedrijf": "Studio Noord", "contactpersoon": "Lisa",
                                                        "telefoon": "", "onderdeelFiscaleEenheid": ""})
                    }
                    ("GET", "icpopgave/9") => {
                        json!({"id": 9, "jaar": 2026, "periodType": "K", "period": 3})
                    }
                    ("GET", "icpopgave/regels/2026K3/1A/gridtable") => rubriek_1a(offset),
                    ("GET", "icpopgave/regels/77") => {
                        json!({"id": 77, "bedrag": 100, "btwNummer": "DE123456789", "land": "DE",
                                                         "jaar": 2026, "periodType": "K", "periodId": 1})
                    }
                    ("GET", "icpopgave/landen") => {
                        json!([{"code": "DE", "land": "Duitsland", "einddatum": null},
                                                      {"code": "BE", "land": "Belgie", "einddatum": null}])
                    }
                    ("GET", p)
                        if p.starts_with("icpopgave/regels/") || p.starts_with("aangifte/2026K3/") =>
                    {
                        empty()
                    }
                    ("POST", "btw-aangifte/verwerk") => json!([4001, 4002]),
                    ("POST", "suppletie/verwerk") => json!([4003]),
                    ("POST", "btw-aangifte/correcties/toggleignore") => json!(true),
                    ("PUT", "icpopgave") => json!(true),
                    ("POST", "icpopgave/regels") => json!({"id": 78}),
                    ("PUT", "icpopgave/regels") => json!(true),
                    ("DELETE", "icpopgave/regels/77") => json!(true),
                    _ => return StatusCode::NOT_FOUND.into_response(),
                };
                axum::Json(answer).into_response()
            }
        },
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

/// EB MCP on a stand-in for e-Boekhouden, with one account and an MCP session. Each test makes its own,
/// because EB MCP allows 60 requests a minute per running app.
struct Harness {
    state: Arc<crate::state::AppState>,
    account: uuid::Uuid,
    seen: Seen,
    http: reqwest::Client,
    url: String,
    session: Option<String>,
}

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

impl Harness {
    async fn new() -> Self {
        use crate::accounts::WriteMode;
        let seen: Seen = Arc::default();
        let jar = Arc::new(crate::state::testing::FakeJar::default());
        let state = Arc::new(crate::state::testing::state_with(
            jar,
            &btw_mock(seen.clone()).await,
        ));
        let account = state.accounts.add(Self::input(WriteMode::Off)).unwrap();
        state.jar_put("sessie", vec![("auth-token".into(), "jwt".into())]);
        state.add_session(account.id, &account.label, "sessie");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let mcp = crate::mcp::router(state.clone(), Arc::new(RwLock::new(TOKEN.to_owned())));
        tokio::spawn(async move { axum::serve(listener, mcp).await.unwrap() });
        let mut harness = Self {
            state,
            account: account.id,
            seen,
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

    fn input(mode: crate::accounts::WriteMode) -> crate::accounts::AccountInput {
        crate::accounts::AccountInput {
            label: "Studio Noord".into(),
            username: "lisa@studionoord.example".into(),
            store: crate::accounts::Store::System,
            op_ref: None,
            op_title: None,
            op_vault: None,
            write: mode,
        }
    }

    fn set_write(&self, mode: crate::accounts::WriteMode) {
        self.state
            .accounts
            .update(self.account, Self::input(mode))
            .unwrap();
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
        let response = self
            .post(json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": {"name": tool, "arguments": arguments}}))
            .await;
        answer_of(&response.text().await.unwrap())
    }

    /// The requests that were no read, in order.
    fn changes(&self) -> Vec<(String, String, Value)> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(method, _, _)| method != "GET")
            .cloned()
            .collect()
    }

    /// How often the opgave of the third quarter of 2026 was opened.
    fn opened(&self) -> usize {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, p, _)| p == "icpopgave/2026/K/3")
            .count()
    }

    fn lines_of(&self, id: u64) -> Vec<(String, String)> {
        self.state
            .approvals
            .list()
            .into_iter()
            .find(|p| p.id == id)
            .unwrap()
            .lines
            .iter()
            .map(|l| (l.label.clone(), l.value.clone()))
            .collect()
    }

    /// Approves the change a "waiting" answer names, and waits until it ran.
    async fn approve(&self, text: &str) -> approvals::Record {
        let id = waiting_id(text);
        assert!(self.state.approvals.decide(id, true, 10));
        settled(&self.state, id).await
    }

    async fn refuse(&self, text: &str) -> approvals::Record {
        let id = waiting_id(text);
        assert!(self.state.approvals.decide(id, false, 10));
        settled(&self.state, id).await
    }

    /// Not one request of the run went to a path that files, pays or confirms, by the list written out in
    /// this file, not by the guard under test.
    fn filed_nothing(&self, at_least: usize) {
        let all = self.seen.lock().unwrap();
        assert!(all.len() >= at_least, "the run made its requests");
        for (method, path, _) in all.iter() {
            assert!(!files_literally(path), "{method} {path}");
        }
    }
}

#[tokio::test]
async fn reads_and_draws_up_returns_only_after_approval_and_never_files() {
    use crate::accounts::WriteMode;
    use approvals::Status;
    let eb = Harness::new().await;

    // Reading works on a read-only account.
    let (is_error, text) = eb.call("btwaangiftes", json!({})).await;
    assert!(!is_error && text.contains("\"soort\":\"OB\""), "{text}");
    let (is_error, text) = eb.call("suppleties", json!({})).await;
    assert!(!is_error, "{text}");
    let list: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(list["data"]["data"].as_array().unwrap().len(), 1);
    assert_eq!(list["data"]["data"][0]["id"], 6);
    let (is_error, text) = eb.call("btwaangifte", json!({"id": 5})).await;
    assert!(!is_error && text.contains("voorbelasting"), "{text}");
    let (is_error, text) = eb
        .call(
            "btwaangifte_mutaties",
            json!({"jaar": 2026, "periode": 3, "rubriek": "1a"}),
        )
        .await;
    assert!(!is_error && text.contains("\"mutatieNr\":3908"), "{text}");
    assert!(eb.seen.lock().unwrap().iter().any(|(_, p, _)| {
        p.starts_with("mutatie/btw-specificatie/gridtable")
            && p.contains("periodStart=20260701")
            && p.contains("periodEnd=20260930")
            && p.contains("category=1a")
    }));
    let (is_error, text) = eb
        .call(
            "btwaangifte_correcties",
            json!({"jaar": 2026, "periode": 3, "rubriek": "1a"}),
        )
        .await;
    assert!(
        !is_error && text.contains("\"periodCode\":\"261\""),
        "{text}"
    );
    let (is_error, text) = eb.call("suppletie", json!({"aangifte_id": 5})).await;
    assert!(
        !is_error && text.contains("\"kan_suppletie\":true"),
        "{text}"
    );
    let (is_error, text) = eb.call("suppletie", json!({"aangifte_id": 7})).await;
    assert!(
        !is_error && text.contains("Er loopt al een suppletie"),
        "{text}"
    );
    let (is_error, _) = eb
        .call("suppletie", json!({"id": 6, "aangifte_id": 5}))
        .await;
    assert!(is_error);
    assert!(eb.changes().is_empty(), "reading changed nothing");

    // Writing is off: refused, nothing read or asked.
    let before = eb.seen.lock().unwrap().len();
    let (is_error, text) = eb
        .call("btwaangifte_verwerken", json!({"jaar": 2026, "periode": 3}))
        .await;
    assert!(is_error && text.contains("alleen-lezen"), "{text}");
    assert_eq!(eb.seen.lock().unwrap().len(), before);
    assert_eq!(eb.state.approvals.count(), 0);

    // Even without approval set for the account, drawing up always asks; refused: nothing goes out.
    eb.set_write(WriteMode::Auto);
    let (is_error, text) = eb
        .call(
            "btwaangifte_verwerken",
            json!({"jaar": 2026, "periode": 3, "btw_5b": 700}),
        )
        .await;
    assert!(!is_error, "{text}");
    let id = waiting_id(&text);
    let proposal = eb
        .state
        .approvals
        .list()
        .into_iter()
        .find(|p| p.id == id)
        .unwrap();
    assert!(proposal.danger);
    assert_eq!(proposal.title, "Btw-aangifte verwerken: 3e kwartaal 2026");
    let lines = eb.lines_of(id);
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
    assert!(lines.contains(&(
        "Bij de Belastingdienst".into(),
        "niet via e-Boekhouden verstuurd".into()
    )));
    assert_eq!(eb.refuse(&text).await.status, Status::Refused);
    assert!(eb.changes().is_empty());

    // Approved: exactly the screen's request, made only after the answer.
    eb.set_write(WriteMode::Ask);
    let (_, text) = eb
        .call("btwaangifte_verwerken", json!({"jaar": 2026, "periode": 3}))
        .await;
    assert!(eb.changes().is_empty(), "nothing before the answer");
    assert_eq!(eb.approve(&text).await.status, Status::Done);

    // A return sent already is drawn up with what was sent, as the screen does.
    let (_, text) = eb
        .call("btwaangifte_verwerken", json!({"jaar": 2026, "periode": 2}))
        .await;
    let lines = eb.lines_of(waiting_id(&text));
    assert!(
        lines.contains(&("Bedragen".into(), "zoals de aangifte verstuurd is".into())),
        "{lines:?}"
    );
    assert!(lines.contains(&(
        "Bij de Belastingdienst".into(),
        "verstuurd en geaccepteerd".into()
    )));
    assert!(
        lines
            .iter()
            .any(|(l, v)| l == "Al verwerkt" && v.contains("EB MCP weet niet")),
        "{lines:?}"
    );
    assert_eq!(eb.approve(&text).await.status, Status::Done);

    let (_, text) = eb
        .call("suppletie_verwerken", json!({"aangifte_id": 5}))
        .await;
    assert_eq!(eb.approve(&text).await.status, Status::Done);
    let (_, text) = eb
        .call(
            "btwaangifte_verschil_negeren",
            json!({"jaar": 2026, "periode": 3, "rubriek": "1a", "periode_code": "261"}),
        )
        .await;
    assert_eq!(eb.approve(&text).await.status, Status::Done);

    let changes = eb.changes();
    assert_eq!(
        changes
            .iter()
            .map(|(m, p, _)| format!("{m} {p}"))
            .collect::<Vec<_>>(),
        [
            "POST btw-aangifte/verwerk",
            "POST btw-aangifte/verwerk",
            "POST suppletie/verwerk",
            "POST btw-aangifte/correcties/toggleignore",
        ]
    );
    assert_eq!(
        changes[0].2,
        json!({"s1aBtw": 2100, "s1bBtw": 90, "s1cBtw": 0, "s1dBtw": 0, "s2aBtw": 0, "s4aBtw": 0, "s4bBtw": 0,
               "s5bBtw": 630, "s5dBtw": 0, "periodYear": 2026, "periodType": "K", "periodId": 3})
    );
    assert_eq!(
        changes[1].2,
        json!({"s1aBtw": 2400, "s1bBtw": 0, "s1cBtw": 0, "s1dBtw": 0, "s2aBtw": 0, "s4aBtw": 0, "s4bBtw": 12,
               "s5bBtw": 650, "s5dBtw": 0, "periodYear": 2026, "periodType": "K", "periodId": 2}),
        "the amounts as sent, not the books of now"
    );
    assert_eq!(
        changes[2].2,
        json!({"s1aBtw": 2100, "s1bBtw": 0, "s1cBtw": 0, "s1dBtw": 0, "s2aBtw": 0, "s4aBtw": 0, "s4bBtw": 0,
               "s5bBtw": 400, "s5dBtw": 0, "periodYear": 2026, "periodType": "K", "periodId": 3})
    );
    assert_eq!(
        changes[3].2,
        json!({"category": "1a", "periodCode": "261", "btw": 105, "omzet": 500, "delete": false})
    );
    eb.filed_nothing(20);
}

#[tokio::test]
async fn opens_and_keeps_the_icp_opgave_only_after_approval_and_never_files() {
    use crate::accounts::WriteMode;
    use approvals::Status;
    let eb = Harness::new().await;

    // Reading a sent opgave works on a read-only account; nothing opens the opgave of a period.
    let (is_error, text) = eb.call("icp_opgaven", json!({})).await;
    assert!(!is_error, "{text}");
    let list: Value = serde_json::from_str(&text).unwrap();
    let ids: Vec<&Value> = list["data"]["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| &r["id"])
        .collect();
    assert_eq!(ids, [&json!(9)], "the Belgian return is not an ICP-opgave");
    let (is_error, text) = eb.call("icp_opgave", json!({"id": 9})).await;
    assert!(!is_error && text.contains("\"3b\""), "{text}");
    assert_eq!(eb.opened(), 0);

    // Opening a period asks like a change; writing off: refused, and nothing opened.
    let (is_error, _) = eb
        .call(
            "icp_opgave_openen",
            json!({"jaar": 2026, "periode_type": "kwartaal", "periode": 3}),
        )
        .await;
    assert!(is_error);
    assert_eq!(eb.opened(), 0);

    eb.set_write(WriteMode::Ask);
    let (is_error, text) = eb
        .call(
            "icp_opgave_openen",
            json!({"jaar": 2026, "periode_type": "kwartaal", "periode": 3}),
        )
        .await;
    assert!(!is_error, "{text}");
    assert!(
        eb.lines_of(waiting_id(&text))
            .iter()
            .any(|(l, v)| l == "Gevolg" && v.contains("lege opgave"))
    );
    assert_eq!(eb.opened(), 0, "nothing opened before the answer");
    assert_eq!(eb.refuse(&text).await.status, Status::Refused);
    assert_eq!(eb.opened(), 0);

    // Approved: the opgave with every line of rubriek 1a, past the first page of 200; the total counts all 201.
    let (_, text) = eb
        .call(
            "icp_opgave_openen",
            json!({"jaar": 2026, "periode_type": "kwartaal", "periode": 3}),
        )
        .await;
    let id = waiting_id(&text);
    assert_eq!(eb.approve(&text).await.status, Status::Done);
    assert_eq!(eb.opened(), 1);
    let detail = eb.state.approvals.get(id).unwrap();
    let result = detail
        .result
        .clone()
        .unwrap_or_else(|| panic!("{:?}", detail.record));
    let rubriek = &result["data"]["rubrieken"]["1a"];
    assert_eq!(rubriek["regels"].as_array().unwrap().len(), 201, "{result}");
    assert_eq!(rubriek["totaal"], 300.0);
    assert!(eb.seen.lock().unwrap().iter().any(|(_, p, _)| {
        p.starts_with("icpopgave/regels/2026K3/1A/gridtable") && p.contains("offset=200")
    }));

    // The details: the period is opened only after the approval.
    let before = eb.opened();
    let (_, text) = eb
        .call(
            "icp_opgave_wijzigen",
            json!({"jaar": 2026, "periode_type": "kwartaal", "periode": 3, "telefoon": "0612345678"}),
        )
        .await;
    assert_eq!(eb.opened(), before, "not opened before the answer");
    assert_eq!(eb.approve(&text).await.status, Status::Done);

    // A bad line is refused before anything is asked or sent.
    let asked = eb.state.approvals.history(None).len();
    let (is_error, text) = eb
        .call(
            "icp_regel_toevoegen",
            json!({"jaar": 2026, "periode_type": "kwartaal", "periode": 3, "rubriek": "1a", "land": "US",
                   "btw_nummer": "US12345", "bedrag": 10, "correctie_jaar": 2026, "correctie_periode_type": "kwartaal", "correctie_periode": 1}),
        )
        .await;
    assert!(is_error && text.contains("Onbekende landcode"), "{text}");
    let (is_error, text) = eb
        .call(
            "icp_regel_toevoegen",
            json!({"jaar": 2026, "periode_type": "kwartaal", "periode": 3, "rubriek": "4", "land": "BE",
                   "btw_nummer": "BE0123456789", "bedrag": 10, "correctie_jaar": 2026, "correctie_periode_type": "kwartaal", "correctie_periode": 1}),
        )
        .await;
    assert!(is_error && text.contains("Rubriek 4"), "{text}");
    assert_eq!(eb.state.approvals.history(None).len(), asked);

    let (_, text) = eb
        .call(
            "icp_regel_toevoegen",
            json!({"jaar": 2026, "periode_type": "kwartaal", "periode": 3, "rubriek": "1a", "land": "de",
                   "btw_nummer": "DE987654321", "bedrag": -40, "correctie_jaar": 2026, "correctie_periode_type": "kwartaal", "correctie_periode": 2}),
        )
        .await;
    assert_eq!(eb.approve(&text).await.status, Status::Done);
    // An A-B-C supply: rubriek 4, without a corrected period.
    let (_, text) = eb
        .call(
            "icp_regel_toevoegen",
            json!({"jaar": 2026, "periode_type": "kwartaal", "periode": 3, "rubriek": "4", "land": "BE",
                   "btw_nummer": "BE0123456789", "bedrag": 2500}),
        )
        .await;
    let lines = eb.lines_of(waiting_id(&text));
    assert!(
        lines.contains(&(
            "Rubriek".into(),
            "4 Intracommunautaire A-B-C-leveringen".into()
        )),
        "{lines:?}"
    );
    assert!(!lines.iter().any(|(l, _)| l == "Gecorrigeerd tijdvak"));
    assert_eq!(eb.approve(&text).await.status, Status::Done);
    // Line 77 sits on the second page of rubriek 1a; its corrected period moves to the second quarter.
    let (_, text) = eb
        .call(
            "icp_regel_wijzigen",
            json!({"jaar": 2026, "periode_type": "kwartaal", "periode": 3, "id": 77, "bedrag": 120,
                   "correctie_jaar": 2026, "correctie_periode_type": "kwartaal", "correctie_periode": 2}),
        )
        .await;
    let lines = eb.lines_of(waiting_id(&text));
    assert!(
        lines.contains(&("Gecorrigeerd tijdvak was".into(), "1e kwartaal 2026".into())),
        "{lines:?}"
    );
    assert!(
        lines.contains(&(
            "Gecorrigeerd tijdvak wordt".into(),
            "2e kwartaal 2026".into()
        )),
        "{lines:?}"
    );
    assert!(
        lines.contains(&("Bedrag was".into(), "100,00".into())),
        "{lines:?}"
    );
    assert_eq!(eb.approve(&text).await.status, Status::Done);
    let (_, text) = eb
        .call(
            "icp_regel_verwijderen",
            json!({"jaar": 2026, "periode_type": "kwartaal", "periode": 3, "id": 77}),
        )
        .await;
    assert_eq!(eb.approve(&text).await.status, Status::Done);
    let (is_error, text) = eb
        .call(
            "icp_regel_verwijderen",
            json!({"jaar": 2026, "periode_type": "kwartaal", "periode": 3, "id": 99}),
        )
        .await;
    assert!(is_error && text.contains("Regel 99"), "{text}");

    let changes = eb.changes();
    assert_eq!(
        changes
            .iter()
            .map(|(m, p, _)| format!("{m} {p}"))
            .collect::<Vec<_>>(),
        [
            "PUT icpopgave",
            "POST icpopgave/regels",
            "POST icpopgave/regels",
            "PUT icpopgave/regels",
            "DELETE icpopgave/regels/77",
        ]
    );
    assert_eq!(
        changes[0].2,
        json!({"id": 55, "btwNummer": "NL001234567B01", "bedrijf": "Studio Noord", "contactpersoon": "Lisa",
               "telefoon": "0612345678", "onderdeelFiscaleEenheid": ""})
    );
    assert_eq!(
        changes[1].2,
        json!({"bedrag": -40, "btwNummer": "DE987654321", "rubriek": "1A", "eaPeriode": "2026K3", "land": "DE",
               "jaar": "2026", "periodType": "K", "periodId": "2"})
    );
    assert_eq!(
        changes[2].2,
        json!({"bedrag": 2500, "btwNummer": "BE0123456789", "rubriek": "4", "eaPeriode": "2026K3", "land": "BE"})
    );
    assert_eq!(
        changes[3].2,
        json!({"id": 77, "bedrag": 120, "btwNummer": "DE123456789", "rubriek": "1A", "eaPeriode": "2026K3", "land": "DE",
               "jaar": "2026", "periodType": "K", "periodId": "2"})
    );
    eb.filed_nothing(30);
}

#[tokio::test]
async fn shows_a_pay_link_after_approval_and_never_pays_opens_or_files() {
    use crate::accounts::WriteMode;
    use approvals::Status;
    let eb = Harness::new().await;
    let pay_requests = |eb: &Harness| -> Vec<String> {
        eb.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, p, _)| files_literally(p))
            .map(|(m, p, _)| format!("{m} {p}"))
            .collect()
    };

    // Writing off: refused before anything is read.
    let (is_error, text) = eb.call("btwaangifte_betaallink", json!({"id": 5})).await;
    assert!(is_error && text.contains("alleen-lezen"), "{text}");
    assert!(eb.seen.lock().unwrap().is_empty());

    // Even an account that needs no approval: what cannot be paid says so, and nothing is asked or started.
    eb.set_write(WriteMode::Auto);
    for (id, want) in [
        (13, "kan in e-Boekhouden niet met iDEAL betaald worden"),
        (14, "al via iDEAL betaald op 20-01-2026 10:00"),
        (15, "op een andere manier betaald"),
        (9, "niet met iDEAL"),
        (99, "geen verstuurde aangifte met id 99"),
    ] {
        let (is_error, text) = eb.call("btwaangifte_betaallink", json!({"id": id})).await;
        assert!(is_error && text.contains(want), "{id}: {text}");
        assert!(!text.contains("http"), "no link for {id}: {text}");
    }
    assert!(
        eb.state.approvals.history(None).is_empty(),
        "nothing was asked"
    );

    // A link the row has already comes back at once: no approval, no request to pay.
    let (is_error, text) = eb.call("btwaangifte_betaallink", json!({"id": 6})).await;
    assert!(!is_error, "{text}");
    let answer: Value = serde_json::from_str(&text).unwrap();
    assert!(
        answer["data"]["betaallink"]
            .as_str()
            .unwrap()
            .ends_with("/betaal/bestaand-6"),
        "{text}"
    );
    assert_eq!(answer["data"]["bestond_al"], true);
    assert_eq!(answer["data"]["soort"], "suppletie");
    assert_eq!(answer["data"]["te_betalen"], "1234,00");
    assert!(
        answer["data"]["let_op"]
            .as_str()
            .unwrap()
            .contains("Open de link niet")
    );
    assert!(eb.state.approvals.history(None).is_empty());
    assert!(pay_requests(&eb).is_empty());

    // A new link always asks, also on this account; refused: no payment is started.
    let (is_error, text) = eb.call("btwaangifte_betaallink", json!({"id": 5})).await;
    assert!(!is_error, "{text}");
    let id = waiting_id(&text);
    let proposal = eb
        .state
        .approvals
        .list()
        .into_iter()
        .find(|p| p.id == id)
        .unwrap();
    assert!(proposal.danger);
    assert_eq!(
        proposal.title,
        "Betaallink opvragen: btw-aangifte 3e kwartaal 2026"
    );
    let lines = eb.lines_of(id);
    assert!(
        lines.contains(&("Te betalen".into(), "1234,00".into())),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|(l, v)| l == "Betalen" && v.contains("opent de link nooit")),
        "{lines:?}"
    );
    assert!(pay_requests(&eb).is_empty(), "nothing before the answer");
    assert_eq!(eb.refuse(&text).await.status, Status::Refused);
    assert!(pay_requests(&eb).is_empty(), "a refusal starts nothing");

    // Approved: exactly one request to pay, and the link in the answer.
    let (_, text) = eb.call("btwaangifte_betaallink", json!({"id": 5})).await;
    let id = waiting_id(&text);
    assert!(pay_requests(&eb).is_empty());
    assert_eq!(eb.approve(&text).await.status, Status::Done);
    assert_eq!(pay_requests(&eb), ["GET aangifte/pay/5"]);
    let result = eb.state.approvals.get(id).unwrap().result.unwrap();
    let data = &result["data"];
    assert!(
        data["betaallink"]
            .as_str()
            .unwrap()
            .ends_with("/betaal/sessie-5"),
        "{result}"
    );
    assert_eq!(data["bestond_al"], false);
    assert_eq!(data["periode"], "3e kwartaal 2026");
    assert_eq!(data["te_betalen"], "1234,00");
    assert!(data["let_op"].as_str().unwrap().contains("kan verlopen"));

    // e-Boekhouden answers without a link: an error, nothing to show.
    let (_, text) = eb.call("btwaangifte_betaallink", json!({"id": 16})).await;
    let record = eb.approve(&text).await;
    assert_eq!(record.status, Status::Failed);
    assert!(record.error.unwrap_or_default().contains("geen betaallink"));

    // Only this tool, only after approval, only these two; no other filing path; and no link was ever opened.
    assert_eq!(
        pay_requests(&eb),
        ["GET aangifte/pay/5", "GET aangifte/pay/16"]
    );
    for (method, path, _) in eb.seen.lock().unwrap().iter() {
        assert!(
            !path.contains("betaal/"),
            "EB MCP fetched a payment link: {method} {path}"
        );
    }
}
