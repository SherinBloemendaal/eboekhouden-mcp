//! The local MCP server: Streamable HTTP on 127.0.0.1, a bearer token per
//! installation, loopback Host only and no browser Origin, so no website can
//! reach it. Every tool is read-only.

use std::sync::Arc;

use axum::{
    Router,
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::router::tool::ToolRouter,
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig},
    tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use serde::Serialize;

use crate::state::AppState;

pub const PORT: u16 = 47821;
pub const PATH: &str = "/mcp";

pub fn url() -> String {
    format!("http://127.0.0.1:{PORT}{PATH}")
}

#[derive(Clone)]
pub struct EbMcp {
    state: Arc<AppState>,
    #[expect(dead_code, reason = "read by the tool_handler macro")]
    tool_router: ToolRouter<Self>,
}

#[derive(Serialize)]
struct Status {
    app: &'static str,
    version: &'static str,
    signed_in: Vec<String>,
    note: &'static str,
}

#[tool_router]
impl EbMcp {
    pub fn new(state: Arc<AppState>) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Toont welke e-Boekhouden-accounts nu in EB MCP zijn ingelogd. Gebruik dit eerst als een andere tool zegt dat er niemand is ingelogd.",
        annotations(title = "Status", read_only_hint = true, open_world_hint = false)
    )]
    async fn status(&self) -> Result<CallToolResult, McpError> {
        let signed_in = self.state.signed_in_labels();
        let note = if signed_in.is_empty() {
            "Nog niemand ingelogd. Log in via EB MCP in de menubalk."
        } else {
            "Alleen lezen: EB MCP kan niets boeken, opslaan of verwijderen."
        };
        let status = Status {
            app: "EB MCP",
            version: env!("CARGO_PKG_VERSION"),
            signed_in,
            note,
        };
        Ok(CallToolResult::success(vec![ContentBlock::json(status)?]))
    }
}

#[tool_handler]
impl ServerHandler for EbMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("eb-mcp", env!("CARGO_PKG_VERSION")).with_title("EB MCP"))
            .with_instructions(
                "Leest de boekhouding in e-Boekhouden van de accounts die in EB MCP zijn ingelogd. Alleen lezen.",
            )
    }
}

/// The HTTP app: the MCP service behind the token check.
pub fn router(state: Arc<AppState>, token: Arc<str>) -> Router {
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

async fn require_token(State(token): State<Arc<str>>, request: Request, next: Next) -> Response {
    if bearer_matches(request.headers(), &token) {
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
pub async fn serve(state: Arc<AppState>, token: Arc<str>) -> std::io::Result<()> {
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
        router(Arc::new(AppState::default()), Arc::from(TOKEN))
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
        assert!(text.contains("\"name\":\"eb-mcp\""), "{text}");
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

    #[test]
    fn compares_tokens_exactly() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }
}
