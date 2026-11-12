//! The local MCP server: Streamable HTTP on 127.0.0.1, a bearer token per
//! installation, loopback Host only and no browser Origin, so no website can
//! reach it. Tools that read are marked read-only; the ones that change the books go through tools.rs's `write_tool`.

use std::sync::{Arc, RwLock};

use axum::{
    Router,
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};

use crate::state::AppState;
use crate::tools::EbMcp;

pub const PORT: u16 = 47821;
pub const PATH: &str = "/mcp";

/// How long an MCP session may sit unused before EB MCP forgets it.
const SESSION_IDLE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

pub fn url() -> String {
    format!("http://127.0.0.1:{PORT}{PATH}")
}

/// The HTTP app: the MCP service behind the token check.
pub fn router(state: Arc<AppState>, token: Arc<RwLock<String>>) -> Router {
    let mut config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(["127.0.0.1", "localhost"])
        // An empty allow list plus enforcement: any request with an Origin header is refused.
        .enforce_origin_validation();
    config.json_response = true;
    // rmcp forgets a session nobody used for five minutes and answers 404 on its id after that. An AI client
    // pauses longer than that all the time (the user reads, approves, has lunch), and its first call after
    // the pause then failed. A session costs next to nothing here and ends with the app, so it may sit unused
    // for a day; a client that outlasts even that starts a new one (the bridge does so by itself).
    let mut sessions = LocalSessionManager::default();
    sessions.session_config.keep_alive = Some(SESSION_IDLE);
    let service = StreamableHttpService::new(
        move || Ok(EbMcp::new(state.clone())),
        Arc::new(sessions),
        config,
    );
    Router::new()
        .nest_service(PATH, service)
        .layer(middleware::from_fn_with_state(token, require_token))
}

async fn require_token(
    State(token): State<Arc<RwLock<String>>>,
    request: Request,
    next: Next,
) -> Response {
    let ok = bearer_matches(request.headers(), &token.read().unwrap());
    if ok {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
            "Token ontbreekt of klopt niet",
        )
            .into_response()
    }
}

fn bearer_matches(headers: &HeaderMap, token: &str) -> bool {
    let Some(value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    let Some(given) = value.strip_prefix("Bearer ") else {
        return false;
    };
    constant_time_eq(given.trim().as_bytes(), token.as_bytes())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Binds 127.0.0.1:PORT and serves until the app quits.
pub async fn serve(state: Arc<AppState>, token: Arc<RwLock<String>>) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", PORT)).await?;
    axum::serve(listener, router(state, token)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn app() -> Router {
        let state =
            crate::state::testing::state_with(Default::default(), "http://127.0.0.1:9/v1/api/");
        router(Arc::new(state), Arc::new(RwLock::new(TOKEN.to_owned())))
    }

    fn initialize(auth: Option<&str>, host: &str, origin: Option<&str>) -> Request<Body> {
        let body = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}
        });
        let mut builder = Request::builder()
            .method("POST")
            .uri(PATH)
            .header(header::HOST, host)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json, text/event-stream");
        if let Some(auth) = auth {
            builder = builder.header(header::AUTHORIZATION, auth);
        }
        if let Some(origin) = origin {
            builder = builder.header(header::ORIGIN, origin);
        }
        builder.body(Body::from(body.to_string())).unwrap()
    }

    #[tokio::test]
    async fn answers_with_the_right_token() {
        let response = app()
            .oneshot(initialize(
                Some(&format!("Bearer {TOKEN}")),
                "127.0.0.1:47821",
                None,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("\"title\":\"e-Boekhouden MCP\""), "{text}");
    }

    #[tokio::test]
    async fn refuses_without_or_with_a_wrong_token() {
        let none = app()
            .oneshot(initialize(None, "127.0.0.1:47821", None))
            .await
            .unwrap();
        assert_eq!(none.status(), StatusCode::UNAUTHORIZED);
        let wrong = app()
            .oneshot(initialize(Some("Bearer nope"), "127.0.0.1:47821", None))
            .await
            .unwrap();
        assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
        let basic = app()
            .oneshot(initialize(
                Some(&format!("Basic {TOKEN}")),
                "127.0.0.1:47821",
                None,
            ))
            .await
            .unwrap();
        assert_eq!(basic.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn refuses_any_browser_origin() {
        let auth = format!("Bearer {TOKEN}");
        for origin in ["https://evil.example", "http://127.0.0.1:47821", "null"] {
            let response = app()
                .oneshot(initialize(Some(&auth), "127.0.0.1:47821", Some(origin)))
                .await
                .unwrap();
            assert!(
                response.status().is_client_error(),
                "{origin}: {}",
                response.status()
            );
        }
    }

    #[tokio::test]
    async fn refuses_a_foreign_host() {
        let auth = format!("Bearer {TOKEN}");
        let response = app()
            .oneshot(initialize(Some(&auth), "evil.example:47821", None))
            .await
            .unwrap();
        assert!(response.status().is_client_error(), "{}", response.status());
    }

    /// Initialize, list the tools and call `status` over real HTTP, the way a client does.
    #[tokio::test]
    async fn serves_the_read_only_tools_end_to_end() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app()).await.unwrap() });
        let http = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{port}{PATH}");
        let post = |body: serde_json::Value, session: Option<String>| {
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
            serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}}),
            None,
        )
        .await
        .unwrap();
        assert_eq!(init.status(), 200);
        let session = init
            .headers()
            .get("mcp-session-id")
            .map(|v| v.to_str().unwrap().to_owned());
        post(
            serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            session.clone(),
        )
        .await
        .unwrap();

        // Answers come as JSON or, in session mode, as server-sent events: take the JSON-RPC message either way.
        let message = |body: String| -> serde_json::Value {
            if let Ok(value) = serde_json::from_str(&body) {
                return value;
            }
            body.lines()
                .filter_map(|l| l.strip_prefix("data:"))
                .filter_map(|d| serde_json::from_str::<serde_json::Value>(d.trim()).ok())
                .find(|v| v.get("id").is_some())
                .unwrap_or_else(|| panic!("no JSON-RPC answer in {body}"))
        };
        let list = message(
            post(
                serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
                session.clone(),
            )
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        );
        let tools = list["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        for want in [
            "status",
            "relaties",
            "relatie",
            "mutaties",
            "open_posten",
            "bankregels",
            "grootboek",
            "balans",
            "winst_en_verlies",
            "btw",
            "uren",
            "digitaal_archief",
            "get",
            "mutatie",
            "facturen",
            "factuur",
            "abonnementen",
            "abonnement",
            "factuur_pdf",
            "emailsjablonen",
            "offertes",
            "offerte",
            "offerte_pdf",
            "sjablonen",
            "sjabloon",
            "kvk_zoeken",
            "kvk_adres",
            "akkoord_status",
        ] {
            assert!(names.contains(&want), "{want} missing from {names:?}");
        }
        // The parts that live in their own module name their tools themselves (tools/*.rs).
        let [part_reads, part_writes, part_destructive] = crate::tools::part_tools();
        for want in part_reads.iter().chain(&part_writes) {
            assert!(names.contains(want), "{want} missing from {names:?}");
        }
        for name in &part_destructive {
            assert!(
                part_writes.contains(name),
                "{name} cannot be undone but is not listed as a write"
            );
        }
        // The tools that change the books say so; the three that cannot be undone say that too.
        let writes = [
            "mutatie_toevoegen",
            "mutatie_wijzigen",
            "archief_uploaden",
            "archief_koppelen",
            "factuur_toevoegen",
            "factuur_wijzigen",
            "factuur_verwijderen",
            "factuur_crediteren",
            "abonnement_toevoegen",
            "abonnement_wijzigen",
            "abonnement_verwijderen",
            "abonnementen_factureren",
            "mutatie_verwijderen",
            "archief_verwijderen",
            "factuur_mailen",
            "emailsjabloon_toevoegen",
            "emailsjabloon_wijzigen",
            "emailsjabloon_verwijderen",
            "offerte_toevoegen",
            "offerte_wijzigen",
            "offerte_verwijderen",
            "sjabloon_toevoegen",
            "sjabloon_wijzigen",
            "sjabloon_verwijderen",
            "relatie_toevoegen",
            "relatie_wijzigen",
            "relatie_verwijderen",
        ];
        let destructive = [
            "relatie_verwijderen",
            "offerte_verwijderen",
            "sjabloon_verwijderen",
            "mutatie_verwijderen",
            "archief_verwijderen",
            "factuur_mailen",
            "emailsjabloon_verwijderen",
            "factuur_verwijderen",
            "factuur_crediteren",
            "abonnement_verwijderen",
            "abonnementen_factureren",
        ];
        for tool in tools {
            let name = tool["name"].as_str().unwrap();
            let writes_it = writes.contains(&name) || part_writes.contains(&name);
            assert_eq!(tool["annotations"]["readOnlyHint"], !writes_it, "{name}");
            if writes_it {
                assert_eq!(
                    tool["annotations"]["destructiveHint"],
                    destructive.contains(&name) || part_destructive.contains(&name),
                    "{name}"
                );
            }
        }
        assert_eq!(
            names.len(),
            28 + writes.len() + part_reads.len() + part_writes.len()
        );
        assert_eq!(crate::tools::tool_count(), names.len());

        let call = message(
            post(serde_json::json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "status", "arguments": {}}}), session.clone())
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
        );
        let text = call["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("\"alleen_lezen\":true"), "{text}");

        // A tool without any account explains how to add one, and goes nowhere near e-Boekhouden.
        let call = message(
            post(serde_json::json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "relaties", "arguments": {"zoek": "bakker"}}}), session)
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
        );
        assert_eq!(call["result"]["isError"], true);
        assert!(
            call["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("nog geen account")
        );
    }

    /// A stand-in for e-Boekhouden's API with the three calls a new mutatie needs, keeping what it was sent.
    async fn eboekhouden(seen: Arc<std::sync::Mutex<Vec<serde_json::Value>>>) -> String {
        use axum::routing::{get, post};
        let app = axum::Router::new()
            .route(
                "/v1/api/grootboekrekening/gridtable",
                get(|| async {
                    axum::Json(serde_json::json!({
                        "colMetadata": [{"name": "id"}, {"name": "code"}, {"name": "omschrijving"}, {"name": "categorie"}, {"name": "inactief"}],
                        "data": [[11, "1010", "Bank", "Betalingsmiddelen", false], [45, "4550", "Bankkosten", "Winst en Verlies", false]]
                    }))
                }),
            )
            .route(
                "/v1/api/btwinfo/all",
                get(|| async { axum::Json(serde_json::json!([{"code": "GEEN", "omschrijving": "Geen btw", "percentage": 0, "soort": ""}])) }),
            )
            .route(
                // The unprocessed bank lines: the one the tests book, in the shape the real grid has.
                "/v1/api/import/gridtable",
                get(|| async {
                    axum::Json(serde_json::json!({
                        "colMetadata": [{"name": "id"}, {"name": "datum"}, {"name": "rekening"}, {"name": "mutDatum"}, {"name": "mutBedrag"}, {"name": "mutOmschrijving"}, {"name": "mutFactuur"}, {"name": "grootboekId"}, {"name": "opmerking"}, {"name": "hasFiles"}, {"name": "verwerkFailureReason"}],
                        "data": [[465233844u64, "2026-11-10T08:00:00", "NL00BANK0123456789", "2026-11-06T00:00:00", -2.0, "BCK*Parkeergarage Dams Groningen", "", 11, null, null, null]],
                        "rowCount": 1, "offset": 0, "limit": 2000
                    }))
                }),
            )
            .route(
                "/v1/api/mutatie",
                post(move |axum::Json(body): axum::Json<serde_json::Value>| {
                    let seen = seen.clone();
                    async move {
                        seen.lock().unwrap().push(body);
                        axum::Json(serde_json::json!({"mutNr": 3908, "mutId": 4770877556u64}))
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://127.0.0.1:{port}/v1/api/")
    }

    #[tokio::test]
    async fn a_change_waits_for_the_user_and_goes_out_only_when_approved() {
        use crate::accounts::{AccountInput, Store, WriteMode};
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let jar = Arc::new(crate::state::testing::FakeJar::default());
        let state = Arc::new(crate::state::testing::state_with(
            jar.clone(),
            &eboekhouden(seen.clone()).await,
        ));
        let account = state
            .accounts
            .add(AccountInput {
                label: "Studio Noord".into(),
                username: "lisa@studionoord.example".into(),
                store: Store::System,
                op_ref: None,
                op_title: None,
                op_vault: None,
                write: WriteMode::Off,
            })
            .unwrap();
        state.jar_put("sessie", vec![("auth-token".into(), "jwt".into())]);
        state.add_session(account.id, &account.label, "sessie");
        let set_write = |mode: WriteMode| {
            state
                .accounts
                .update(
                    account.id,
                    AccountInput {
                        label: "Studio Noord".into(),
                        username: "lisa@studionoord.example".into(),
                        store: Store::System,
                        op_ref: None,
                        op_title: None,
                        op_vault: None,
                        write: mode,
                    },
                )
                .unwrap();
        };

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let mcp = router(state.clone(), Arc::new(RwLock::new(TOKEN.to_owned())));
        tokio::spawn(async move { axum::serve(listener, mcp).await.unwrap() });
        let http = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{port}{PATH}");
        let post = |body: serde_json::Value, session: Option<String>| {
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
            serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "title": "Claude", "version": "1"}}}),
            None,
        )
        .await
        .unwrap();
        let session = init
            .headers()
            .get("mcp-session-id")
            .map(|v| v.to_str().unwrap().to_owned());
        post(
            serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            session.clone(),
        )
        .await
        .unwrap();
        let book = |id: u32| {
            let call = post(
                serde_json::json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": "mutatie_toevoegen", "arguments": {
                    "soort": "geld_uitgegeven", "datum": "2026-11-06", "rekening": "1010", "omschrijving": "Parkeergarage",
                    "regels": [{"tegenrekening": "4550", "bedrag": 2}], "bankregel_id": 465233844u64
                }}}),
                session.clone(),
            );
            async move {
                let body = call.await.unwrap().text().await.unwrap();
                let answer = serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .unwrap_or_else(|| {
                        body.lines()
                            .filter_map(|l| l.strip_prefix("data:"))
                            .filter_map(|d| {
                                serde_json::from_str::<serde_json::Value>(d.trim()).ok()
                            })
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
        };
        let ask_status = |id: u64| {
            let call = post(
                serde_json::json!({"jsonrpc": "2.0", "id": 90, "method": "tools/call", "params": {"name": "akkoord_status", "arguments": {"voorstel_id": id}}}),
                session.clone(),
            );
            async move { answer_of(call.await.unwrap().text().await.unwrap()) }
        };
        // The one proposal that is up, checked against what the tool asked for.
        let proposal_up = || {
            let list = state.approvals.list();
            assert_eq!(list.len(), 1, "{list:?}");
            let proposal = list.into_iter().next().unwrap();
            assert_eq!(proposal.title, "Mutatie toevoegen: Geld uitgegeven");
            assert_eq!(proposal.client, "Claude");
            assert!(!proposal.danger);
            assert!(
                proposal
                    .lines
                    .iter()
                    .any(|l| l.label == "Regel 1" && l.value == "4550 Bankkosten: 2,00 (Geen btw)"),
                "{:?}",
                proposal.lines
            );
            // The bank line was read from e-Boekhouden and is shown as the statement has it; it matches the booking.
            assert!(
                proposal.lines.iter().any(|l| l.label == "Bankregel"
                    && l.value == "06-11-2026, -2,00: BCK*Parkeergarage Dams Groningen (id 465233844); deze boeking verwerkt hem"),
                "{:?}",
                proposal.lines
            );
            assert!(
                !proposal.lines.iter().any(|l| l.label == "Let op"),
                "{:?}",
                proposal.lines
            );
            proposal
        };
        let request = serde_json::json!({
            "mutatie": {"rekening": 11, "datum": "2026-11-06", "soort": 6, "inEx": "IN", "omschrijving": "Parkeergarage"},
            "mutatieRegels": [{"index": 0, "tegenRekening": 45, "btwCode": "GEEN", "bedrag": 2.0}],
            "importId": 465233844u64
        });

        // Writing is off: refused, and nothing was even read from e-Boekhouden.
        let (is_error, text) = book(2).await;
        assert!(is_error && text.contains("alleen-lezen"), "{text}");
        assert_eq!(state.approvals.count(), 0);
        assert!(
            state.approvals.history(None).is_empty(),
            "nothing was asked"
        );

        // With approval the call answers at once that the change waits; the user says no: nothing goes out.
        set_write(WriteMode::Ask);
        // A bank line that is not among the unprocessed ones (a wrong id, or one that was processed already) is
        // refused before the user is asked anything: booking it would put the same money in the books twice.
        let unknown = post(
            serde_json::json!({"jsonrpc": "2.0", "id": 30, "method": "tools/call", "params": {"name": "mutatie_toevoegen", "arguments": {
                // The account code as a number, as a client sent it (2026-11-12): read as the text 1010, so the
                // call gets as far as the bank line instead of being refused for its kind.
                "soort": "geld_uitgegeven", "datum": "2026-11-06", "rekening": 1010, "omschrijving": "Parkeergarage",
                "regels": [{"tegenrekening": 4550, "bedrag": "2"}], "bankregel_id": 1
            }}}),
            session.clone(),
        )
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
        let (is_error, text) = answer_of(unknown);
        assert!(
            is_error && text.contains("Bankregel 1 staat niet bij de onverwerkte bankregels"),
            "{text}"
        );
        assert_eq!(state.approvals.count(), 0);
        assert!(
            state.approvals.history(None).is_empty(),
            "nothing was asked"
        );
        assert!(seen.lock().unwrap().is_empty(), "nothing was sent");

        let (is_error, text) = book(3).await;
        assert!(!is_error, "waiting is not an error: {text}");
        let id = waiting_id(&text);
        assert!(text.contains("akkoord_status"), "{text}");
        assert_eq!(proposal_up().id, id);
        assert_eq!(state.log.running(), 0, "a waiting change holds up no quit");
        let entry = mcp_entry(&state, id);
        assert_eq!(entry.outcome, crate::reqlog::Outcome::Waiting);
        assert!(state.approvals.decide(id, false, 1));
        let record = settled(&state, id).await;
        assert_eq!(record.status, crate::approvals::Status::Refused);
        assert!(seen.lock().unwrap().is_empty());
        let refused = mcp_entry(&state, id);
        assert_eq!(refused.outcome, crate::reqlog::Outcome::Refused);
        // The wait ran until the refusal; nothing ran after it.
        let (started, ended) = (
            refused.wait_started_at.unwrap(),
            refused.wait_ended_at.unwrap(),
        );
        assert_eq!(refused.waited_ms, ended - started);
        assert_eq!(
            refused.duration_ms - refused.waited_ms,
            started - refused.at
        );
        let (_, status) = ask_status(id).await;
        assert!(status.contains("\"status\":\"geweigerd\""), "{status}");

        // The user approves: exactly the request e-Boekhouden's own screen sends, made in the background.
        let (is_error, text) = book(4).await;
        assert!(!is_error, "{text}");
        let id = waiting_id(&text);
        assert_eq!(proposal_up().id, id);
        let (_, status) = ask_status(id).await;
        assert!(status.contains("\"status\":\"wacht\""), "{status}");
        assert!(
            seen.lock().unwrap().is_empty(),
            "nothing goes out before the answer"
        );
        assert!(state.approvals.decide(id, true, 2));
        let record = settled(&state, id).await;
        assert_eq!(record.status, crate::approvals::Status::Done, "{record:?}");
        assert_eq!(*seen.lock().unwrap(), vec![request.clone()]);
        assert_eq!(state.approvals.count(), 0, "an answered proposal is gone");
        let (is_error, status) = ask_status(id).await;
        assert!(!is_error, "{status}");
        let status: serde_json::Value = serde_json::from_str(&status).unwrap();
        assert_eq!(status["status"], "uitgevoerd");
        assert_eq!(status["besluit"], "goedgekeurd");
        assert_eq!(status["resultaat"]["data"]["mutNr"], 3908);
        // The log links the e-Boekhouden calls to the MCP call and shows how it ended.
        let entry = mcp_entry(&state, id);
        assert_eq!(entry.outcome, crate::reqlog::Outcome::Ok);
        assert!(entry.result.unwrap().to_string().contains("3908"));
        let posted = entry.calls.iter().any(|call| {
            matches!(state.log.get(*call), Some(crate::reqlog::Entry::Eb(e)) if e.method == "POST" && e.path == "mutatie")
        });
        assert!(posted, "the POST is linked to the MCP call");
        // The reads came before the wait, the POST after the approval.
        let (started, ended) = (entry.wait_started_at.unwrap(), entry.wait_ended_at.unwrap());
        assert_eq!(entry.waited_ms, ended - started);
        for call in &entry.calls {
            let Some(crate::reqlog::Entry::Eb(e)) = state.log.get(*call) else {
                panic!("call {call} is not in the log")
            };
            if e.method == "POST" {
                assert!(
                    e.at >= ended,
                    "{} at {} before the approval {ended}",
                    e.path,
                    e.at
                );
            } else {
                assert!(
                    e.at <= started,
                    "{} at {} after the wait began {started}",
                    e.path,
                    e.at
                );
            }
        }
        let detail = state.approvals.get(id).unwrap();
        assert_eq!(detail.body, request);
        assert_eq!(detail.args["omschrijving"], "Parkeergarage");

        // Without approval the booking goes straight through, and the history says so.
        set_write(WriteMode::Auto);
        let (is_error, text) = book(5).await;
        assert!(!is_error, "{text}");
        assert!(text.contains("\"mutNr\":3908"), "{text}");
        assert_eq!(seen.lock().unwrap().len(), 2);
        assert_eq!(state.approvals.count(), 0);
        let auto = &state.approvals.history(Some(1))[0];
        assert_eq!(auto.decision, Some(crate::approvals::Decision::Auto));
        assert_eq!(auto.status, crate::approvals::Status::Done);
        let ran = mcp_entry(&state, auto.proposal.id);
        assert_eq!(
            (ran.wait_started_at, ran.waited_ms),
            (None, 0),
            "it never waited"
        );

        // Approved after the account signed out: it fails as "Niet ingelogd", and nothing went out.
        set_write(WriteMode::Ask);
        let (_, text) = book(6).await;
        let id = waiting_id(&text);
        state.sign_out(account.id, "test");
        assert!(state.approvals.decide(id, true, 3));
        let record = settled(&state, id).await;
        assert_eq!(record.status, crate::approvals::Status::Failed);
        assert!(
            record
                .error
                .as_deref()
                .unwrap_or_default()
                .starts_with("Niet ingelogd"),
            "{record:?}"
        );
        assert_eq!(mcp_entry(&state, id).outcome, crate::reqlog::Outcome::Error);
        assert_eq!(seen.lock().unwrap().len(), 2);
        let (_, status) = ask_status(id).await;
        assert!(status.contains("\"status\":\"mislukt\""), "{status}");

        // Writing turned off while a change waits: it is refused, and can no longer be approved.
        state.add_session(account.id, &account.label, "sessie");
        let (_, text) = book(7).await;
        let id = waiting_id(&text);
        set_write(WriteMode::Off);
        assert_eq!(
            state.approvals.end_account(
                account.id,
                crate::approvals::Answer::Refused(crate::approvals::REFUSED_READ_ONLY.into()),
                4
            ),
            1
        );
        assert!(!state.approvals.decide(id, true, 5));
        assert_eq!(
            settled(&state, id).await.status,
            crate::approvals::Status::Refused
        );
        assert_eq!(seen.lock().unwrap().len(), 2);

        // Without an id: the proposals of the last day, newest first.
        let (is_error, list) = message_text(
            post(
                serde_json::json!({"jsonrpc": "2.0", "id": 91, "method": "tools/call", "params": {"name": "akkoord_status", "arguments": {}}}),
                session.clone(),
            )
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        );
        assert!(!is_error, "{list}");
        let list: serde_json::Value = serde_json::from_str(&list).unwrap();
        let states: Vec<&str> = list["voorstellen"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["status"].as_str().unwrap())
            .collect();
        assert_eq!(
            states,
            [
                "geweigerd",
                "mislukt",
                "uitgevoerd",
                "uitgevoerd",
                "geweigerd"
            ]
        );
    }

    /// The JSON-RPC answer in a body that is plain JSON or server-sent events: whether it is an error, and its text.
    fn message_text(body: String) -> (bool, String) {
        answer_of(body)
    }

    fn answer_of(body: String) -> (bool, String) {
        let answer = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .unwrap_or_else(|| {
                body.lines()
                    .filter_map(|l| l.strip_prefix("data:"))
                    .filter_map(|d| serde_json::from_str::<serde_json::Value>(d.trim()).ok())
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
        let value: serde_json::Value =
            serde_json::from_str(text).unwrap_or_else(|_| panic!("not JSON: {text}"));
        assert_eq!(value["status"], "wacht", "{text}");
        value["voorstel_id"].as_u64().unwrap()
    }

    /// The MCP entry in the request log that asked for proposal `id`.
    fn mcp_entry(state: &AppState, id: u64) -> crate::reqlog::McpEntry {
        let log_id = state.approvals.get(id).unwrap().log_id.unwrap();
        match state.log.get(log_id) {
            Some(crate::reqlog::Entry::Mcp(entry)) => {
                assert_eq!(entry.proposal, Some(id));
                entry
            }
            other => panic!("no MCP entry for proposal {id}: {other:?}"),
        }
    }

    /// Waits for the background run of an answered proposal to end (its record and its MCP entry in the
    /// request log), and gives its record.
    async fn settled(state: &AppState, id: u64) -> crate::approvals::Record {
        use crate::approvals::Status;
        use crate::reqlog::Outcome;
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

    /// What the stand-in for e-Boekhouden below was asked to change, in order.
    type Changes = Arc<std::sync::Mutex<Vec<String>>>;

    /// A stand-in for e-Boekhouden with a mutatie, two archive files (one linked) and a template in the editor.
    async fn books_mock(changes: Changes) -> String {
        use axum::extract::Path;
        use axum::routing::{delete, get, post};
        use serde_json::json;
        let note = |changes: &Changes, what: String| changes.lock().unwrap().push(what);
        let (c1, c2, c3) = (changes.clone(), changes.clone(), changes.clone());
        let app = axum::Router::new()
            .route("/v1/api/mutatie/nr/16/model", get(|| async { axum::Json(json!({"mutatie": {"nr": 16, "datum": "2026-11-11T00:00:00", "omschrijving": "Proef", "soort": 6}, "mutatieRegels": []})) }))
            .route(
                "/v1/api/mutatie/gridtable",
                get(|| async {
                    axum::Json(json!({"colMetadata": [{"name": "id", "readerIndex": 0}, {"name": "mutatieNr", "readerIndex": 1}, {"name": "soort", "readerIndex": 2}, {"name": "bedrag", "readerIndex": 3}],
                                      "data": [[777, 16, "Geld uit", 2.42]]}))
                }),
            )
            .route(
                "/v1/api/mutatie/bulk/delete",
                post(move |axum::Json(body): axum::Json<serde_json::Value>| {
                    let changes = c1.clone();
                    async move {
                        note(&changes, format!("delete mutaties {body}"));
                        axum::Json(json!({"removedIds": body, "errorMessages": [], "notFoundIds": []}))
                    }
                }),
            )
            .route(
                "/v1/api/folder/{id}",
                get(|Path(id): Path<u64>| async move { axum::Json(json!({"id": id, "naam": format!("bon-{id}.pdf"), "soort": "F", "isDeleted": id == 3, "koppelCount": 0})) }).delete(
                    move |Path(id): Path<u64>| {
                        let changes = c2.clone();
                        async move {
                            note(&changes, format!("delete file {id}"));
                            axum::Json(json!(true))
                        }
                    },
                ),
            )
            .route(
                "/v1/api/folder/{id}/koppel",
                get(|Path(id): Path<u64>| async move {
                    // File 2 is linked to a mutatie; the API gives lists.
                    axum::Json(json!({"koppelImport": [], "koppelMutatie": if id == 2 { json!([{"mutNr": 16}]) } else { json!([]) }, "koppelOfferte": [], "koppelRelatie": []}))
                }),
            )
            .route("/v1/api/factuursjabloon/9", get(|| async { axum::Json(json!({"id": 9, "naam": "Factuursjabloon", "inactief": false, "type": "E"})) }))
            .route("/v1/api/factuursjabloon/9/language", get(|| async { axum::Json(json!("NL")) }))
            .route(
                "/v1/api/factuursjabloon/editor/9/NL",
                get(|| async { (axum::http::StatusCode::FOUND, [(axum::http::header::LOCATION, "/editor/?sessionHash=abc123")]) }),
            )
            .route(
                "/editor/api/session/abc123",
                get(|headers: HeaderMap| async move {
                    if !headers.get("cookie").and_then(|v| v.to_str().ok()).is_some_and(|c| c.contains("auth-token=jwt")) {
                        return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
                    }
                    axum::Json(json!({"sessionKey": "geheim", "sjabloon": {"sessionHash": "ouder", "elements": [{"id": 1, "tag": "text", "x": 1, "y": 2, "width": 3, "height": 4, "content": "<h1>FACTUUR</h1>"}], "documentDefinition": {"width": 840}}})).into_response()
                }),
            )
            .route("/editor/api/sjabloon/codes/abc123", get(|| async { axum::Json(json!([{"category": "Factuur", "code": "[NUMMER]", "description": "Het factuurnummer"}])) }))
            .route(
                "/editor/api/session/end/abc123",
                post(move |axum::Json(body): axum::Json<serde_json::Value>| {
                    let changes = c3.clone();
                    async move {
                        note(&changes, format!("editor end save={} hash={}", body["saveChanges"], body["saveData"]["sessionHash"]));
                        ""
                    }
                }),
            )
            .route("/v1/api/folder/x", delete(|| async { "" }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://127.0.0.1:{port}/v1/api/")
    }

    #[tokio::test]
    async fn what_cannot_be_undone_always_asks_and_reading_a_template_saves_nothing() {
        use crate::accounts::{AccountInput, Store, WriteMode};
        let changes: Changes = Arc::default();
        let jar = Arc::new(crate::state::testing::FakeJar::default());
        let state = Arc::new(crate::state::testing::state_with(
            jar,
            &books_mock(changes.clone()).await,
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

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let mcp = router(state.clone(), Arc::new(RwLock::new(TOKEN.to_owned())));
        tokio::spawn(async move { axum::serve(listener, mcp).await.unwrap() });
        let http = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{port}{PATH}");
        let post = |body: serde_json::Value, session: Option<String>| {
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
            serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}}),
            None,
        )
        .await
        .unwrap();
        let session = init
            .headers()
            .get("mcp-session-id")
            .map(|v| v.to_str().unwrap().to_owned());
        post(
            serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            session.clone(),
        )
        .await
        .unwrap();
        let call = |tool: &'static str, arguments: serde_json::Value| {
            let request = post(
                serde_json::json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": {"name": tool, "arguments": arguments}}),
                session.clone(),
            );
            async move {
                let body = request.await.unwrap().text().await.unwrap();
                let answer = serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .unwrap_or_else(|| {
                        body.lines()
                            .filter_map(|l| l.strip_prefix("data:"))
                            .filter_map(|d| {
                                serde_json::from_str::<serde_json::Value>(d.trim()).ok()
                            })
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
        };
        // Answers the one proposal that is up, after checking it is marked as not undoable, and waits for its
        // background run to end.
        let decide = |approve: bool, title: &'static str| {
            let state = state.clone();
            async move {
                let list = state.approvals.list();
                assert_eq!(list.len(), 1, "{list:?}");
                let proposal = list.into_iter().next().unwrap();
                assert_eq!(proposal.title, title);
                assert!(proposal.danger, "{title} must be shown as not undoable");
                assert!(state.approvals.decide(proposal.id, approve, 1));
                settled(&state, proposal.id).await;
                proposal
            }
        };
        let made = || changes.lock().unwrap().clone();

        // A read-only account: the new tools are refused before anything is asked or sent.
        for (tool, arguments) in [
            ("mutatie_verwijderen", serde_json::json!({"nr": 16})),
            ("archief_verwijderen", serde_json::json!({"bestand_id": 1})),
            (
                "sjabloon_verwijderen",
                serde_json::json!({"soort": "factuur", "id": 9}),
            ),
            (
                "factuur_mailen",
                serde_json::json!({"id": 1, "aan": "jan@example.com"}),
            ),
            ("offerte_verwijderen", serde_json::json!({"id": 1})),
        ] {
            let (is_error, text) = call(tool, arguments).await;
            assert!(is_error && text.contains("alleen-lezen"), "{tool}: {text}");
        }
        assert_eq!(state.approvals.count(), 0);
        assert!(made().is_empty());

        // An account that needs no approval for ordinary changes still asks before a mutatie is deleted.
        state
            .accounts
            .update(account.id, input(WriteMode::Auto))
            .unwrap();
        let (is_error, text) = call("mutatie_verwijderen", serde_json::json!({"nr": 16})).await;
        assert!(!is_error, "{text}");
        let id = waiting_id(&text);
        assert!(made().is_empty(), "nothing is deleted before the answer");
        let proposal = decide(false, "Mutatie 16 verwijderen").await;
        assert_eq!(proposal.id, id);
        assert_eq!(
            settled(&state, id).await.status,
            crate::approvals::Status::Refused
        );
        assert!(
            proposal.lines.iter().any(|l| l.value.contains("afgeraden")),
            "{:?}",
            proposal.lines
        );
        assert!(made().is_empty(), "refused: nothing was deleted");
        let (is_error, text) = call("mutatie_verwijderen", serde_json::json!({"nr": 16})).await;
        assert!(!is_error, "{text}");
        let id = waiting_id(&text);
        assert!(made().is_empty(), "nothing is deleted before the answer");
        decide(true, "Mutatie 16 verwijderen").await;
        assert_eq!(
            made(),
            vec!["delete mutaties [777]"],
            "by the id the list gives for that number"
        );
        let (is_error, status) =
            call("akkoord_status", serde_json::json!({"voorstel_id": id})).await;
        assert!(!is_error, "{status}");
        let status: serde_json::Value = serde_json::from_str(&status).unwrap();
        assert_eq!(status["status"], "uitgevoerd");
        assert_eq!(status["resultaat"]["gedaan"], "Mutatie verwijderd");
        assert!(
            status["regels"]
                .as_array()
                .unwrap()
                .iter()
                .any(|l| l["value"].as_str().unwrap().contains("afgeraden")),
            "{status}"
        );

        // A file nothing hangs on goes without asking on this account; a linked one always asks, with a warning.
        let (is_error, text) =
            call("archief_verwijderen", serde_json::json!({"bestand_id": 1})).await;
        assert!(!is_error, "{text}");
        assert_eq!(made().last().unwrap(), "delete file 1");
        let (is_error, text) =
            call("archief_verwijderen", serde_json::json!({"bestand_id": 2})).await;
        assert!(!is_error, "{text}");
        let id = waiting_id(&text);
        let proposal = decide(false, "Bestand verwijderen: bon-2.pdf").await;
        assert_eq!(proposal.id, id);
        let (_, status) = call("akkoord_status", serde_json::json!({"voorstel_id": id})).await;
        assert!(status.contains("\"status\":\"geweigerd\""), "{status}");
        assert!(
            proposal.lines.iter().any(|l| l.value.contains("1 mutatie")),
            "{:?}",
            proposal.lines
        );
        assert_eq!(made().len(), 2, "the linked file is still there");
        // A file that is in the bin already is left alone.
        let (is_error, text) =
            call("archief_verwijderen", serde_json::json!({"bestand_id": 3})).await;
        assert!(is_error && text.contains("prullenbak"), "{text}");

        // Reading a layout opens the editor and closes it without saving; the session's keys stay inside.
        let (is_error, text) =
            call("sjabloon", serde_json::json!({"soort": "factuur", "id": 9})).await;
        assert!(!is_error, "{text}");
        assert!(
            text.contains("FACTUUR") && text.contains("[NUMMER]"),
            "{text}"
        );
        assert!(
            !text.contains("sessionHash") && !text.contains("abc123") && !text.contains("geheim"),
            "{text}"
        );
        assert_eq!(
            made().last().unwrap(),
            "editor end save=false hash=\"abc123\""
        );
        let logged = serde_json::to_string(&state.log.list()).unwrap()
            + &state
                .log
                .list()
                .iter()
                .filter_map(|e| {
                    serde_json::to_string(&state.log.get(match e {
                        crate::reqlog::Entry::Mcp(m) => m.id,
                        crate::reqlog::Entry::Eb(b) => b.id,
                    }))
                    .ok()
                })
                .collect::<String>();
        assert!(
            !logged.contains("abc123") && !logged.contains("geheim"),
            "the session's keys are not in the log"
        );

        // A changed layout says what changes, and is saved in a new session only after the user's word.
        state
            .accounts
            .update(account.id, input(WriteMode::Ask))
            .unwrap();
        let opmaak = serde_json::json!({"elements": [{"id": 1, "tag": "text", "x": 1, "y": 2, "width": 3, "height": 4, "content": "<h1>FACTUUR 2</h1>"}]});
        let (is_error, text) = call(
            "sjabloon_wijzigen",
            serde_json::json!({"soort": "factuur", "id": 9, "opmaak": opmaak}),
        )
        .await;
        assert!(!is_error, "{text}");
        let id = waiting_id(&text);
        let list = state.approvals.list();
        assert_eq!(list.len(), 1);
        let proposal = &list[0];
        assert_eq!(proposal.id, id);
        assert!(!proposal.danger);
        assert!(
            proposal
                .lines
                .iter()
                .any(|l| l.label == "Tekst van text 1 was" && l.value == "FACTUUR"),
            "{:?}",
            proposal.lines
        );
        assert!(
            proposal
                .lines
                .iter()
                .any(|l| l.label == "Tekst van text 1 wordt" && l.value == "FACTUUR 2"),
            "{:?}",
            proposal.lines
        );
        assert_eq!(
            made().last().unwrap(),
            "editor end save=false hash=\"abc123\"",
            "nothing saved before the answer"
        );
        assert!(state.approvals.decide(id, true, 1));
        assert_eq!(
            settled(&state, id).await.status,
            crate::approvals::Status::Done
        );
        assert_eq!(
            made().last().unwrap(),
            "editor end save=true hash=\"abc123\""
        );
        // The history keeps the layout the tool was given, without the editor's session keys.
        let kept = serde_json::to_string(&state.approvals.get(id).unwrap()).unwrap();
        assert!(kept.contains("FACTUUR 2"), "{kept}");
        assert!(
            !kept.contains("abc123") && !kept.contains("geheim"),
            "{kept}"
        );
        let (_, status) = call("akkoord_status", serde_json::json!({"voorstel_id": id})).await;
        assert!(status.contains("\"status\":\"uitgevoerd\""), "{status}");
    }

    #[test]
    fn compares_tokens_exactly() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }
}
