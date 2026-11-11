//! e-Boekhouden's SignalR hub, for the one job its web app does not do with a plain request: "Facturen maken"
//! for subscriptions (`hub/factuur/abonnementenfactureren`). Checked against the real hub on 2026-11-11:
//! only the WebSocket transport carries the session (long polling answers "Je sessie is verlopen").
//!
//! The protocol, as the web app's client speaks it: `POST <hub>/negotiate` gives a connection token; a WebSocket
//! to `<hub>?id=<token>` takes `{"protocol":"json","version":1}` and answers `{}`; then one invocation goes out
//! and the hub calls back `init`, `progress` and `done` (or an error by name). Every message ends in 0x1e.

use std::time::Duration;

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::Role;
use url::Url;

use crate::ebapi::{auth_token, cookie_header};

/// SignalR's record separator.
const RS: char = '\u{1e}';
/// The web app gives the job five minutes (`serverTimeoutInMilliseconds`).
const JOB: Duration = Duration::from_secs(300);
const STEP: Duration = Duration::from_secs(30);

/// What the hub said when the job ended.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// The argument of `done`.
    Done(Value),
    /// A refusal the hub put into words (`validationError`, `badRequestError`, `unhandledError`).
    Refused(String),
}

/// One message of the hub, already cut at the record separator.
pub fn outcome_of(message: &str) -> Option<Outcome> {
    let json: Value = serde_json::from_str(message).ok()?;
    let argument = || {
        json.get("arguments")
            .and_then(|a| a.get(0))
            .cloned()
            .unwrap_or(Value::Null)
    };
    match (
        json.get("type").and_then(Value::as_u64),
        json.get("target").and_then(Value::as_str),
    ) {
        (Some(1), Some("done")) => Some(Outcome::Done(argument())),
        (Some(1), Some("validationError" | "badRequestError" | "unhandledError")) => {
            let said = argument();
            let text = said
                .as_str()
                .map(str::to_owned)
                .or_else(|| {
                    said.get("message")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| "e-Boekhouden gaf een fout zonder uitleg".into());
            Some(Outcome::Refused(text))
        }
        // The hub closed the connection, or completed the call with an error of its own.
        (Some(7), _) => Some(Outcome::Refused(
            json.get("error")
                .and_then(Value::as_str)
                .unwrap_or("e-Boekhouden sloot de verbinding")
                .to_owned(),
        )),
        (Some(3), _) => json
            .get("error")
            .and_then(Value::as_str)
            .map(|e| Outcome::Refused(e.to_owned())),
        _ => None,
    }
}

/// Calls `target` on the hub at `<base>hub/<hub>` with one argument and waits for the job to end.
pub async fn invoke(
    base: &Url,
    cookies: &[(String, String)],
    hub: &str,
    target: &str,
    argument: &Value,
) -> Result<Outcome, String> {
    let lost = || "Geen verbinding met e-Boekhouden".to_owned();
    let address = base
        .join(&format!("hub/{hub}"))
        .map_err(|_| "Ongeldig pad".to_owned())?;
    // HTTP/1.1 only: a WebSocket starts as an upgrade of a plain request, which HTTP/2 does not have.
    let http = reqwest::Client::builder()
        .http1_only()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("EB MCP/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| lost())?;
    let signed = |request: reqwest::RequestBuilder| {
        let request = request.header(reqwest::header::COOKIE, cookie_header(cookies));
        match auth_token(cookies) {
            Some(token) => request.bearer_auth(token),
            None => request,
        }
    };

    let mut negotiate = address.clone();
    negotiate.set_path(&format!("{}/negotiate", address.path()));
    negotiate.set_query(Some("negotiateVersion=1"));
    let answer = tokio::time::timeout(STEP, signed(http.post(negotiate)).send())
        .await
        .map_err(|_| "e-Boekhouden antwoordde niet binnen 30 s".to_owned())?
        .map_err(|_| lost())?;
    match answer.status().as_u16() {
        200 => {}
        300..=399 | 401 => return Err("Niet (meer) ingelogd bij e-Boekhouden".into()),
        status => return Err(format!("e-Boekhouden gaf een fout ({status})")),
    }
    let negotiated: Value = answer
        .json()
        .await
        .map_err(|_| "e-Boekhouden gaf geen JSON terug".to_owned())?;
    let token = negotiated
        .get("connectionToken")
        .and_then(Value::as_str)
        .ok_or("e-Boekhouden gaf geen verbinding voor deze taak")?;

    let mut socket_url = address.clone();
    socket_url.query_pairs_mut().append_pair("id", token);
    let key = base64::engine::general_purpose::STANDARD.encode(rand::random::<[u8; 16]>());
    let upgrade = signed(http.get(socket_url))
        .header(reqwest::header::CONNECTION, "Upgrade")
        .header(reqwest::header::UPGRADE, "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", key);
    let answer = tokio::time::timeout(STEP, upgrade.send())
        .await
        .map_err(|_| "e-Boekhouden antwoordde niet binnen 30 s".to_owned())?
        .map_err(|_| lost())?;
    if answer.status().as_u16() != 101 {
        return Err(format!(
            "e-Boekhouden opende de verbinding niet ({})",
            answer.status().as_u16()
        ));
    }
    let upgraded = answer.upgrade().await.map_err(|_| lost())?;
    let mut socket = WebSocketStream::from_raw_socket(upgraded, Role::Client, None).await;

    let job = async {
        let send = |text: String| Message::Text(format!("{text}{RS}").into());
        socket
            .send(send(json!({"protocol": "json", "version": 1}).to_string()))
            .await
            .map_err(|_| lost())?;
        let mut invoked = false;
        while let Some(frame) = socket.next().await {
            let frame = frame.map_err(|_| lost())?;
            let text = match frame {
                Message::Text(text) => text.to_string(),
                Message::Close(_) => break,
                _ => continue,
            };
            for message in text.split(RS).filter(|m| !m.is_empty()) {
                if !invoked {
                    // The answer to the handshake: `{}`, or `{"error": "..."}`.
                    if let Some(error) = serde_json::from_str::<Value>(message)
                        .ok()
                        .and_then(|j| j.get("error").and_then(Value::as_str).map(str::to_owned))
                    {
                        return Err(format!("e-Boekhouden weigerde de verbinding: {error}"));
                    }
                    let call = json!({"type": 1, "invocationId": "0", "target": target, "arguments": [argument]});
                    socket
                        .send(send(call.to_string()))
                        .await
                        .map_err(|_| lost())?;
                    invoked = true;
                    continue;
                }
                if let Some(outcome) = outcome_of(message) {
                    return Ok(outcome);
                }
            }
        }
        Err("e-Boekhouden sloot de verbinding voordat de taak klaar was. Kijk in e-Boekhouden wat er is gedaan.".to_owned())
    };
    let result = tokio::time::timeout(JOB, job).await;
    let _ = socket.close(None).await;
    result.map_err(|_| {
        "De taak duurde langer dan 5 minuten. Kijk in e-Boekhouden wat er is gedaan.".to_owned()
    })?
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
    use axum::http::HeaderMap;
    use axum::response::IntoResponse;
    use axum::routing::{get, post};

    #[test]
    fn reads_the_end_of_a_job() {
        assert_eq!(
            outcome_of(r#"{"type":1,"target":"init","arguments":[2]}"#),
            None
        );
        assert_eq!(
            outcome_of(r#"{"type":1,"target":"progress","arguments":[1]}"#),
            None
        );
        assert_eq!(outcome_of(r#"{"type":6}"#), None, "a ping");
        assert_eq!(
            outcome_of(
                r#"{"type":1,"target":"done","arguments":[{"createdIds":[7],"errorMessages":[]}]}"#
            ),
            Some(Outcome::Done(
                json!({"createdIds": [7], "errorMessages": []})
            ))
        );
        assert_eq!(
            outcome_of(
                r#"{"type":1,"target":"validationError","arguments":["Er is geen debiteurenrekening beschikbaar."]}"#
            ),
            Some(Outcome::Refused(
                "Er is geen debiteurenrekening beschikbaar.".into()
            ))
        );
        assert_eq!(
            outcome_of(
                r#"{"type":1,"target":"badRequestError","arguments":[{"code":"X","message":"Kan niet"}]}"#
            ),
            Some(Outcome::Refused("Kan niet".into()))
        );
        assert_eq!(
            outcome_of(r#"{"type":3,"invocationId":"0","result":null}"#),
            None,
            "the plain completion follows done"
        );
        assert_eq!(outcome_of("geen json"), None);
    }

    /// A hub that speaks the protocol as e-Boekhouden's did, and refuses a connection without the session cookie.
    async fn serve(socket: WebSocket, signed_in: bool) {
        let mut socket = socket;
        let Some(Ok(WsMessage::Text(handshake))) = socket.recv().await else {
            return;
        };
        assert_eq!(
            handshake.as_str(),
            "{\"protocol\":\"json\",\"version\":1}\u{1e}"
        );
        socket
            .send(WsMessage::Text("{}\u{1e}".into()))
            .await
            .unwrap();
        let Some(Ok(WsMessage::Text(call))) = socket.recv().await else {
            return;
        };
        let call: Value = serde_json::from_str(call.as_str().trim_end_matches('\u{1e}')).unwrap();
        assert_eq!(call["target"], "create");
        let answer = if !signed_in {
            r#"{"type":1,"target":"validationError","arguments":["Je sessie is verlopen. Je moet nu opnieuw inloggen."]}"#.to_owned()
        } else {
            let count = call["arguments"][0]["aantal"].as_u64().unwrap_or(0);
            format!(
                "{}\u{1e}{}",
                json!({"type": 1, "target": "init", "arguments": [count]}),
                json!({"type": 1, "target": "done", "arguments": [{"createdIds": [41, 42], "errorMessages": []}]})
            )
        };
        socket
            .send(WsMessage::Text(
                format!(
                    "{answer}\u{1e}{{\"type\":3,\"invocationId\":\"0\",\"result\":null}}\u{1e}"
                )
                .into(),
            ))
            .await
            .unwrap();
    }

    async fn mock() -> Url {
        let app = axum::Router::new()
            .route(
                "/v1/api/hub/factuur/abonnementenfactureren/negotiate",
                post(|headers: HeaderMap| async move {
                    let cookie = headers
                        .get("cookie")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or_default();
                    if cookie.is_empty() {
                        return axum::http::StatusCode::UNAUTHORIZED.into_response();
                    }
                    axum::Json(json!({"negotiateVersion": 1, "connectionToken": "tok en/1"}))
                        .into_response()
                }),
            )
            .route(
                "/v1/api/hub/factuur/abonnementenfactureren",
                get(
                    |upgrade: WebSocketUpgrade,
                     headers: HeaderMap,
                     axum::extract::RawQuery(query): axum::extract::RawQuery| async move {
                        assert_eq!(
                            query.as_deref(),
                            Some("id=tok+en%2F1"),
                            "the token goes in the query, encoded"
                        );
                        let signed_in = headers
                            .get("cookie")
                            .and_then(|v| v.to_str().ok())
                            .is_some_and(|c| c.contains("auth-token=jwt"));
                        upgrade.on_upgrade(move |socket| serve(socket, signed_in))
                    },
                ),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Url::parse(&format!("http://127.0.0.1:{port}/v1/api/")).unwrap()
    }

    #[tokio::test]
    async fn runs_a_job_on_the_hub_over_a_websocket() {
        let base = mock().await;
        let cookies = vec![("auth-token".to_owned(), "jwt".to_owned())];
        let outcome = invoke(
            &base,
            &cookies,
            "factuur/abonnementenfactureren",
            "create",
            &json!({"aantal": 2}),
        )
        .await;
        assert_eq!(
            outcome,
            Ok(Outcome::Done(
                json!({"createdIds": [41, 42], "errorMessages": []})
            ))
        );
    }

    #[tokio::test]
    async fn passes_on_what_the_hub_refuses_and_stops_when_signed_out() {
        let base = mock().await;
        let other = vec![("SRVRID".to_owned(), "s1".to_owned())];
        let outcome = invoke(
            &base,
            &other,
            "factuur/abonnementenfactureren",
            "create",
            &json!({}),
        )
        .await;
        assert_eq!(
            outcome,
            Ok(Outcome::Refused(
                "Je sessie is verlopen. Je moet nu opnieuw inloggen.".into()
            ))
        );
        let nobody = invoke(
            &base,
            &[],
            "factuur/abonnementenfactureren",
            "create",
            &json!({}),
        )
        .await;
        assert_eq!(nobody, Err("Niet (meer) ingelogd bij e-Boekhouden".into()));
    }
}
