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
            "grootboek",
            "balans",
            "winst_en_verlies",
            "btw",
            "uren",
            "digitaal_archief",
            "get",
        ] {
            assert!(names.contains(&want), "{want} missing from {names:?}");
        }
        assert_eq!(names.len(), 12);
        assert!(
            tools
                .iter()
                .all(|t| t["annotations"]["readOnlyHint"] == true),
            "every tool is read-only"
        );

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

    #[test]
    fn compares_tokens_exactly() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }
}
