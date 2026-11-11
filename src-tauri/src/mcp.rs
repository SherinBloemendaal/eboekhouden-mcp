//! The local MCP server: Streamable HTTP on 127.0.0.1, a bearer token per
//! installation, loopback Host only and no browser Origin, so no website can
//! reach it. Every tool is read-only.

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
    let service = StreamableHttpService::new(
        move || Ok(EbMcp::new(state.clone())),
        Arc::new(LocalSessionManager::default()),
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
        ] {
            assert!(names.contains(&want), "{want} missing from {names:?}");
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
        ];
        let destructive = [
            "factuur_verwijderen",
            "factuur_crediteren",
            "abonnement_verwijderen",
            "abonnementen_factureren",
        ];
        for tool in tools {
            let name = tool["name"].as_str().unwrap();
            let writes_it = writes.contains(&name);
            assert_eq!(tool["annotations"]["readOnlyHint"], !writes_it, "{name}");
            if writes_it {
                assert_eq!(
                    tool["annotations"]["destructiveHint"],
                    destructive.contains(&name),
                    "{name}"
                );
            }
        }
        assert_eq!(names.len(), 18 + writes.len());
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
        // The user's answer in the app, given once the proposal is up.
        let answer_with = |approve: bool| {
            let state = state.clone();
            async move {
                for _ in 0..200 {
                    if let Some(proposal) = state.approvals.list().into_iter().next() {
                        assert_eq!(proposal.title, "Mutatie toevoegen: Geld uitgegeven");
                        assert_eq!(proposal.client, "Claude");
                        assert!(!proposal.danger);
                        assert!(
                            proposal.lines.iter().any(|l| l.label == "Regel 1"
                                && l.value == "4550 Bankkosten: 2,00 (Geen btw)"),
                            "{:?}",
                            proposal.lines
                        );
                        assert!(state.approvals.decide(proposal.id, approve));
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                panic!("no proposal came up");
            }
        };

        // Writing is off: refused, and nothing was even read from e-Boekhouden.
        let (is_error, text) = book(2).await;
        assert!(is_error && text.contains("alleen-lezen"), "{text}");
        assert_eq!(state.approvals.count(), 0);

        // With approval, and the user says no: nothing goes out.
        set_write(WriteMode::Ask);
        let ((is_error, text), ()) = tokio::join!(book(3), answer_with(false));
        assert!(is_error && text.contains("geweigerd"), "{text}");
        assert!(seen.lock().unwrap().is_empty());

        // The user approves: exactly the request e-Boekhouden's own screen sends.
        let ((is_error, text), ()) = tokio::join!(book(4), answer_with(true));
        assert!(!is_error, "{text}");
        assert!(text.contains("\"mutNr\":3908"), "{text}");
        assert_eq!(
            *seen.lock().unwrap(),
            vec![serde_json::json!({
                "mutatie": {"rekening": 11, "datum": "2026-11-06", "soort": 6, "inEx": "IN", "omschrijving": "Parkeergarage"},
                "mutatieRegels": [{"index": 0, "tegenRekening": 45, "btwCode": "GEEN", "bedrag": 2.0}],
                "importId": 465233844u64
            })]
        );
        assert_eq!(state.approvals.count(), 0, "an answered proposal is gone");

        // Without approval the booking goes straight through.
        set_write(WriteMode::Auto);
        let (is_error, text) = book(5).await;
        assert!(!is_error, "{text}");
        assert_eq!(seen.lock().unwrap().len(), 2);
        assert_eq!(state.approvals.count(), 0);
    }

    #[test]
    fn compares_tokens_exactly() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }
}
