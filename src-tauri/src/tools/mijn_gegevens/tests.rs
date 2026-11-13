//! Tests of the Mijn gegevens tools: bodies, approval lines, refusals, and end to end with a stand-in for e-Boekhouden.

use super::*;

/// What `GET administratie/mijngegevens` gives: the form's fields and much more of the administration.
fn administratie() -> Value {
    json!({
        "id": 4711,
        "bedrijf": "Studio Noord B.V.",
        "naam": "Lisa de Vries",
        "geslacht": "v",
        "adres": "Oude Gracht 1",
        "postcode": "3511 AA",
        "plaats": "Utrecht",
        "telefoon": "030 123 4567",
        "email": "info@studionoord.example",
        "website": "studionoord.example",
        "obNummer": "NL123456782B01",
        "btwId": "",
        "kvkNummer": "12345678",
        "iban": "NL91ABNA0417164300",
        "bic": "ABNANL2A",
        "klantcode": "K123",
        "incassoRekening": "NL02RABO0123456789",
        "incassoTnv": "Studio Noord",
        "betaalwijze": "I",
        "diensten": "BH,FACT",
        "soort": "BV",
        "skRelatieId": 99
    })
}

fn field_of(fields: &'static [Field], key: &str) -> &'static Field {
    fields.iter().find(|f| f.key == key).unwrap()
}

#[test]
fn changes_only_the_named_fields_and_sends_the_rest_back_as_read() {
    let merged = merge(
        &administratie(),
        BEDRIJF,
        BEDRIJF_KEEP,
        vec![
            ("adres", " Nieuwe Gracht 12 ".into()),
            ("postcode", "3512ab".into()),
        ],
    )
    .unwrap();
    // Exactly the edit form's fields: nothing of the incasso, the licence or the id goes back.
    assert_eq!(
        Value::Object(merged.body),
        json!({
            "bedrijf": "Studio Noord B.V.",
            "naam": "Lisa de Vries",
            "geslacht": "v",
            "adres": "Nieuwe Gracht 12",
            "postcode": "3512 AB",
            "plaats": "Utrecht",
            "telefoon": "030 123 4567",
            "email": "info@studionoord.example",
            "website": "studionoord.example",
            "obNummer": "NL123456782B01",
            "btwId": "",
            "kvkNummer": "12345678",
            "iban": "NL91ABNA0417164300",
            "bic": "ABNANL2A",
            "klantcode": "K123"
        })
    );
    assert_eq!(merged.changed, ["adres", "postcode"]);
    let shown: Vec<(String, String)> = merged
        .lines
        .iter()
        .map(|l| (l.label.clone(), l.value.clone()))
        .collect();
    assert_eq!(
        shown,
        [
            ("Adres was".to_owned(), "Oude Gracht 1".to_owned()),
            ("Adres wordt".to_owned(), "Nieuwe Gracht 12".to_owned()),
            ("Postcode was".to_owned(), "3511 AA".to_owned()),
            ("Postcode wordt".to_owned(), "3512 AB".to_owned()),
        ]
    );
}

#[test]
fn a_field_the_model_lacks_stays_out_or_gets_the_forms_default() {
    let read = json!({"bedrijf": "A", "naam": "B", "adres": "C", "postcode": "1234 AB", "plaats": "D", "email": "a@b.nl", "website": null});
    let merged = merge(&read, BEDRIJF, BEDRIJF_KEEP, vec![("plaats", "E".into())]).unwrap();
    assert_eq!(merged.body.get("geslacht"), Some(&json!("m")));
    assert_eq!(merged.body.get("website"), Some(&Value::Null));
    for absent in ["iban", "bic", "obNummer", "klantcode", "telefoon"] {
        assert!(!merged.body.contains_key(absent), "{absent}");
    }
}

#[test]
fn shows_empty_and_unknown_values_as_such() {
    let read = json!({"bedrijf": "A", "naam": "B", "adres": "C", "postcode": "1234 AB", "plaats": "D", "email": "a@b.nl", "telefoon": ""});
    let merged = merge(
        &read,
        BEDRIJF,
        BEDRIJF_KEEP,
        vec![
            ("telefoon", "06 1234 5678".into()),
            ("website", "a.nl".into()),
            ("geslacht", "a".into()),
            ("plaats", "".into()),
        ],
    );
    assert_eq!(merged.unwrap_err(), "Plaats mag niet leeg zijn");
    let merged = merge(
        &read,
        BEDRIJF,
        BEDRIJF_KEEP,
        vec![
            ("telefoon", "06 1234 5678".into()),
            ("website", "a.nl".into()),
            ("geslacht", "a".into()),
        ],
    )
    .unwrap();
    let values: Vec<&str> = merged.lines.iter().map(|l| l.value.as_str()).collect();
    assert_eq!(
        values,
        [
            "(leeg)",
            "06 1234 5678",
            "onbekend",
            "a.nl",
            "man",
            "afdeling"
        ]
    );
}

#[test]
fn refuses_a_change_that_changes_nothing() {
    let unchanged = merge(
        &administratie(),
        BEDRIJF,
        BEDRIJF_KEEP,
        vec![("postcode", "3511aa".into()), ("plaats", "Utrecht".into())],
    );
    assert!(unchanged.unwrap_err().starts_with("Er is niets gewijzigd"));
    let nothing = merge(&administratie(), BEDRIJF, BEDRIJF_KEEP, Vec::new());
    assert!(nothing.unwrap_err().starts_with("Er is niets gewijzigd"));
}

#[test]
fn refuses_to_save_a_form_that_misses_a_required_field() {
    let mut read = administratie();
    read["naam"] = json!("");
    let error = merge(
        &read,
        BEDRIJF,
        BEDRIJF_KEEP,
        vec![("plaats", "Zeist".into())],
    )
    .unwrap_err();
    assert!(error.starts_with("Contactpersoon is verplicht"), "{error}");
}

#[test]
fn checks_new_values_as_the_form_does() {
    let check = |key: &str, value: &str| clean(field_of(BEDRIJF, key), value);
    assert_eq!(check("kvkNummer", "1234.5678 ").unwrap(), "12345678");
    assert!(
        check("kvkNummer", "1234567a")
            .unwrap_err()
            .contains("alleen cijfers")
    );
    assert_eq!(
        check("obNummer", "nl 1234.5678.2b01").unwrap(),
        "NL123456782B01"
    );
    assert_eq!(check("obNummer", "123456782B01").unwrap(), "123456782B01");
    assert!(
        check("obNummer", "NL123456789B01")
            .unwrap_err()
            .contains("OB-nummer")
    );
    assert!(
        check("obNummer", "NL-123")
            .unwrap_err()
            .contains("letters en cijfers")
    );
    assert_eq!(check("btwId", "NL0000 99998B57").unwrap(), "NL000099998B57");
    assert!(
        check("btwId", "NL123456789B01")
            .unwrap_err()
            .contains("btw-id")
    );
    assert_eq!(check("btwId", "").unwrap(), "");
    assert_eq!(
        check("iban", "nl91 abna 0417 1643 00").unwrap(),
        "NL91ABNA0417164300"
    );
    assert!(
        check("iban", "NL91ABNA0417164301")
            .unwrap_err()
            .contains("geen geldig IBAN")
    );
    assert!(
        check("iban", "NL91ABNA041716430").is_err(),
        "a Dutch IBAN has 18 characters"
    );
    assert_eq!(
        check("iban", "BE68539007547034").unwrap(),
        "BE68539007547034"
    );
    assert_eq!(check("iban", "").unwrap(), "");
    assert_eq!(check("bic", "rabonl2u").unwrap(), "RABONL2U");
    assert!(check("bic", "RABO").is_err());
    assert_eq!(
        check("email", "lisa@").unwrap_err(),
        "lisa@ is geen geldig e-mailadres"
    );
    assert!(
        check("email", "")
            .unwrap_err()
            .contains("mag niet leeg zijn")
    );
    assert_eq!(check("website", "").unwrap(), "");
    assert!(
        check("bedrijf", &"x".repeat(101))
            .unwrap_err()
            .contains("hoogstens 100")
    );
    assert!(
        check("bedrijf", &"é".repeat(100)).is_ok(),
        "the length counts characters"
    );
    assert_eq!(check("postcode", "  sw1a 1aa").unwrap(), "SW1A1AA");
    assert_eq!(check("postcode", "0123ab").unwrap(), "0123AB");
}

#[test]
fn takes_the_named_arguments_under_the_forms_keys() {
    let args: BedrijfsgegevensWijzigenArgs = serde_json::from_value(json!({
        "contactpersoon": "Jan", "geslacht": "geen", "kvk": "1", "ob_nummer": "x", "btw_id": "y", "telefoon": ""
    }))
    .unwrap();
    let mut given = args.given();
    given.sort();
    assert_eq!(
        given,
        [
            ("btwId", "y".to_owned()),
            ("geslacht", "b".to_owned()),
            ("kvkNummer", "1".to_owned()),
            ("naam", "Jan".to_owned()),
            ("obNummer", "x".to_owned()),
            ("telefoon", String::new()),
        ]
    );
    let args: FactuuradresWijzigenArgs = serde_json::from_value(json!({
        "email": "a@b.nl", "telefoon": "1", "mobiel": "2", "fax": "3", "geslacht": "vrouw"
    }))
    .unwrap();
    let mut given = args.given();
    given.sort();
    assert_eq!(
        given,
        [
            ("emailFactuur", "a@b.nl".to_owned()),
            ("faxnummer", "3".to_owned()),
            ("geslacht", "v".to_owned()),
            ("mobielnummer", "2".to_owned()),
            ("telefoonnummer", "1".to_owned()),
        ]
    );
}

#[test]
fn saves_the_invoice_address_under_the_names_the_form_sends() {
    // The form reads telefoon, mobiel and fax; a model with the relatie names keeps its values too.
    for read in [
        json!({"bedrijf": "Studio Noord", "contactpersoon": "Lisa", "geslacht": "v", "adres": "Oude Gracht 1", "postcode": "3511 AA", "plaats": "Utrecht", "land": "Nederland", "telefoon": "030", "mobiel": "06", "emailFactuur": "factuur@studionoord.example", "fax": "", "id": 5}),
        json!({"bedrijf": "Studio Noord", "contactpersoon": "Lisa", "geslacht": "v", "adres": "Oude Gracht 1", "postcode": "3511 AA", "plaats": "Utrecht", "land": "Nederland", "telefoonnummer": "030", "mobielnummer": "06", "emailFactuur": "factuur@studionoord.example", "faxnummer": "", "id": 5}),
    ] {
        let merged = merge(
            &read,
            FACTUUR,
            &[],
            vec![("emailFactuur", "boekhouding@studionoord.example".into())],
        )
        .unwrap();
        assert_eq!(
            Value::Object(merged.body),
            json!({
                "bedrijf": "Studio Noord", "contactpersoon": "Lisa", "geslacht": "v", "adres": "Oude Gracht 1",
                "postcode": "3511 AA", "plaats": "Utrecht", "land": "Nederland", "telefoonnummer": "030",
                "mobielnummer": "06", "emailFactuur": "boekhouding@studionoord.example", "faxnummer": ""
            })
        );
    }
}

#[test]
fn an_empty_invoice_address_needs_its_required_fields() {
    let error = merge(
        &Value::Null,
        FACTUUR,
        &[],
        vec![("bedrijf", "Studio Noord".into())],
    )
    .unwrap_err();
    assert!(error.starts_with("Adres is verplicht"), "{error}");
    let merged = merge(
        &Value::Null,
        FACTUUR,
        &[],
        vec![
            ("bedrijf", "Studio Noord".into()),
            ("adres", "Oude Gracht 1".into()),
            ("postcode", "3511aa".into()),
            ("plaats", "Utrecht".into()),
            ("emailFactuur", "f@studionoord.example".into()),
        ],
    )
    .unwrap();
    assert_eq!(
        Value::Object(merged.body),
        json!({"bedrijf": "Studio Noord", "geslacht": "m", "adres": "Oude Gracht 1", "postcode": "3511 AA", "plaats": "Utrecht", "emailFactuur": "f@studionoord.example"})
    );
    assert!(merge(&json!([1]), FACTUUR, &[], vec![("bedrijf", "A".into())]).is_err());
}

#[test]
fn only_a_real_answer_counts_as_saved() {
    assert!(saved(json!(true)).is_ok());
    assert!(saved(json!({"id": 1})).is_ok());
    for bad in [Value::Null, json!(false), json!("")] {
        let Err(Halt::Error(message)) = saved(bad) else {
            panic!("an empty answer counted as saved");
        };
        assert!(
            message.contains("niet bekend") && !message.contains("niets gewijzigd"),
            "{message}"
        );
    }
}

#[test]
fn checks_numbers_like_the_web_app() {
    assert_eq!(mod97("NL000099998B57"), Some(1));
    assert_eq!(mod97("nl"), None);
    assert!(ob_nummer_ok("NL123456782B01"));
    assert!(!ob_nummer_ok("NL000456782B01"));
    assert!(!ob_nummer_ok("NL123456782B00"));
    assert!(!ob_nummer_ok("NL123456782X01"));
    assert!(btw_id_ok("NL002455799B11"));
    assert!(!btw_id_ok("DE002455799B11"));
    assert!(iban_ok("NL91ABNA0417164300"));
    assert!(iban_ok("DE89370400440532013000"));
    assert!(!iban_ok("NL91ABNA04171643"));
    assert_eq!(postcode("1234ab"), "1234 AB");
    assert_eq!(postcode("1234 ab"), "1234 AB");
    assert_eq!(postcode("B-1000"), "B-1000");
}

// ------------------------------------------------------------ end to end, with a stand-in for e-Boekhouden

/// What the stand-in was asked to change, in order: every request that is not a GET, as "METHOD path?query body".
type Seen = Arc<Mutex<Vec<String>>>;

/// How the stand-in answers `GET bic`.
#[derive(Clone, Copy)]
enum BicAnswer {
    /// The BIC of the bank, for the one IBAN it knows; an empty answer for any other.
    Known,
    /// A BIC that is not one.
    Invalid,
    /// A server error.
    Fails,
}

/// What the stand-in serves; a test changes it between the proposal and the approval.
struct Stub {
    administratie: Mutex<Value>,
    factuuradres: Mutex<Value>,
    bic: Mutex<BicAnswer>,
}

fn factuuradres_model() -> Value {
    json!({"bedrijf": "Studio Noord", "contactpersoon": "Lisa", "geslacht": "v", "adres": "Oude Gracht 1", "postcode": "3511 AA", "plaats": "Utrecht", "land": "Nederland", "telefoon": "030", "mobiel": "", "emailFactuur": "factuur@studionoord.example", "fax": ""})
}

/// A stand-in for e-Boekhouden's API: it answers the reads of this part, and keeps every change it gets, on any
/// path and with any method, so a stray write cannot go unnoticed.
async fn mock(seen: Seen, stub: Arc<Stub>) -> String {
    use axum::http::{Method, StatusCode, Uri, header};
    use axum::response::IntoResponse;
    let app = axum::Router::new().fallback(
        move |method: Method, uri: Uri, body: axum::body::Bytes| {
            let (seen, stub) = (seen.clone(), stub.clone());
            async move {
                let path = uri.path().trim_start_matches("/v1/api/").to_owned();
                let query = uri.query().unwrap_or_default().to_owned();
                if method != Method::GET {
                    let body = serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
                    let shown = if query.is_empty() {
                        format!("{method} {path} {body}")
                    } else {
                        format!("{method} {path}?{query} {body}")
                    };
                    seen.lock().unwrap().push(shown);
                    return axum::Json(json!(true)).into_response();
                }
                match path.as_str() {
                    // The administration's own details only with adm=true.
                    "administratie/mijngegevens" if query == "adm=true" => {
                        axum::Json(stub.administratie.lock().unwrap().clone()).into_response()
                    }
                    "administratie/mijngegevens" => {
                        axum::Json(json!({"bedrijf": "Accountantskantoor"})).into_response()
                    }
                    "bic" => match *stub.bic.lock().unwrap() {
                        BicAnswer::Known if query == "iban=NL18RABO0123459876" => {
                            axum::Json(json!("RABONL2U")).into_response()
                        }
                        BicAnswer::Known => axum::Json(json!("")).into_response(),
                        BicAnswer::Invalid => axum::Json(json!("RABO-NL")).into_response(),
                        BicAnswer::Fails => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
                    },
                    "skillsource/relatie/mijngegevens/factuuradres" => {
                        axum::Json(stub.factuuradres.lock().unwrap().clone()).into_response()
                    }
                    "skillsource/factuur/recent/gridtable" => axum::Json(json!({
                        "colMetadata": [{"name": "id"}, {"name": "datum"}, {"name": "nummer"}, {"name": "periode"}, {"name": "totaal"}, {"name": "voldaan"}],
                        "data": [[8812, "2026-10-01T00:00:00", "SK-104", "okt 2026", 15.73, 15.73]], "rowCount": 1
                    }))
                    .into_response(),
                    "skillsource/openstaandeposten/gridtable" => axum::Json(json!({
                        "colMetadata": [{"name": "id"}, {"name": "nummer"}, {"name": "totaal"}],
                        "data": [[8813, "SK-105", 15.73]], "rowCount": 1
                    }))
                    .into_response(),
                    pdf if pdf.starts_with("SkillSource/factuur/recent/pdf/") => {
                        let id = pdf.rsplit('/').next().unwrap_or_default().to_owned();
                        (
                            [
                                (header::CONTENT_TYPE, "application/pdf".to_owned()),
                                (
                                    header::CONTENT_DISPOSITION,
                                    format!("attachment; filename=SK-{id}.pdf"),
                                ),
                            ],
                            b"%PDF-1.4 e-Boekhouden".to_vec(),
                        )
                            .into_response()
                    }
                    _ => StatusCode::NOT_FOUND.into_response(),
                }
            }
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://127.0.0.1:{port}/v1/api/")
}

/// The JSON-RPC answer in a body that is plain JSON or server-sent events.
fn answer_of(body: &str) -> Value {
    serde_json::from_str::<Value>(body).ok().unwrap_or_else(|| {
        body.lines()
            .filter_map(|l| l.strip_prefix("data:"))
            .filter_map(|d| serde_json::from_str::<Value>(d.trim()).ok())
            .find(|v| v.get("id").is_some())
            .unwrap_or_else(|| panic!("no JSON-RPC answer in {body}"))
    })
}

/// Waits until the background run of an answered proposal has ended.
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
async fn changing_the_own_details_always_asks_and_sends_only_after_approval() {
    use crate::accounts::{AccountInput, Store};
    use std::sync::RwLock;
    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let seen: Seen = Arc::default();
    let stub = Arc::new(Stub {
        administratie: Mutex::new(administratie()),
        factuuradres: Mutex::new(factuuradres_model()),
        bic: Mutex::new(BicAnswer::Known),
    });
    let jar = Arc::new(crate::state::testing::FakeJar::default());
    let state = Arc::new(crate::state::testing::state_with(
        jar,
        &mock(seen.clone(), stub.clone()).await,
    ));
    let input = |mode: Rights| AccountInput {
        label: "Studio Noord".into(),
        username: "lisa@studionoord.example".into(),
        store: Store::System,
        op_ref: None,
        op_title: None,
        op_vault: None,
        rights: mode,
    };
    let account = state.accounts.add(input(Rights::OFF)).unwrap();
    state.jar_put("sessie", vec![("auth-token".into(), "jwt".into())]);
    state.add_session(account.id, &account.label, "sessie");

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
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "title": "Claude", "version": "1"}}}),
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
        async move {
            let answer = answer_of(&request.await.unwrap().text().await.unwrap());
            let result = answer["result"].clone();
            let text = result["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            (result["isError"] == true, text, result)
        }
    };
    let made = || seen.lock().unwrap().clone();
    let proposal_up = |title: &str| {
        let list = state.approvals.list();
        assert_eq!(list.len(), 1, "{list:?}");
        let proposal = list.into_iter().next().unwrap();
        assert_eq!(proposal.title, title);
        assert!(proposal.danger, "{title} must always ask");
        proposal
    };
    let lines = |proposal: &approvals::Proposal| -> Vec<(String, String)> {
        proposal
            .lines
            .iter()
            .map(|l| (l.label.clone(), l.value.clone()))
            .collect()
    };

    // Reading works on a read-only account, and asks for the administration's own details.
    let (is_error, text, _) = call("bedrijfsgegevens", json!({})).await;
    assert!(!is_error, "{text}");
    let read: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(read["data"]["bedrijf"], "Studio Noord B.V.");
    assert_eq!(read["data"]["iban"], "NL91ABNA0417164300");
    let (is_error, text, _) = call("factuuradres", json!({})).await;
    assert!(
        !is_error && text.contains("factuur@studionoord.example"),
        "{text}"
    );

    // A read-only account: both changes are refused before anything is asked or sent.
    for (tool, arguments) in [
        (
            "bedrijfsgegevens_wijzigen",
            json!({"adres": "Nieuwe Gracht 12"}),
        ),
        (
            "factuuradres_wijzigen",
            json!({"email": "boekhouding@studionoord.example"}),
        ),
    ] {
        let (is_error, text, _) = call(tool, arguments).await;
        assert!(is_error && text.contains("alleen-lezen"), "{tool}: {text}");
    }
    assert_eq!(state.approvals.count(), 0);

    // Writing without approval for ordinary changes: the company details still ask.
    state
        .accounts
        .update(account.id, input(Rights::AUTO))
        .unwrap();

    // Bad input is refused before anything is asked or sent.
    let (is_error, text, _) = call(
        "bedrijfsgegevens_wijzigen",
        json!({"iban": "NL91ABNA0417164301"}),
    )
    .await;
    assert!(is_error && text.contains("geen geldig IBAN"), "{text}");
    let (is_error, text, _) = call("bedrijfsgegevens_wijzigen", json!({"plaats": "Utrecht"})).await;
    assert!(is_error && text.contains("niets gewijzigd"), "{text}");
    assert_eq!(state.approvals.count(), 0);
    assert!(made().is_empty());

    // The user says no: nothing goes out.
    let change = json!({"adres": "Nieuwe Gracht 12", "postcode": "3512ab", "iban": "NL18 RABO 0123 4598 76"});
    let (is_error, text, _) = call("bedrijfsgegevens_wijzigen", change.clone()).await;
    assert!(!is_error, "{text}");
    assert!(text.contains("\"status\":\"wacht\""), "{text}");
    let proposal = proposal_up("Bedrijfsgegevens wijzigen: Studio Noord B.V.");
    let shown = lines(&proposal);
    for want in [
        ("Adres was", "Oude Gracht 1"),
        ("Adres wordt", "Nieuwe Gracht 12"),
        ("Postcode wordt", "3512 AB"),
        ("IBAN was", "NL91ABNA0417164300"),
        ("IBAN wordt", "NL18RABO0123459876"),
        ("BIC was", "ABNANL2A"),
        ("BIC wordt", "RABONL2U (opgezocht bij het nieuwe IBAN)"),
    ] {
        assert!(
            shown.contains(&(want.0.to_owned(), want.1.to_owned())),
            "{want:?} not in {shown:?}"
        );
    }
    assert!(state.approvals.decide(proposal.id, false, 1));
    assert_eq!(
        settled(&state, proposal.id).await.status,
        approvals::Status::Refused
    );
    assert!(made().is_empty());

    // The user approves: exactly the form's fields go out, with the query of the edit screen.
    let (_, text, _) = call("bedrijfsgegevens_wijzigen", change).await;
    assert!(text.contains("\"status\":\"wacht\""), "{text}");
    let proposal = proposal_up("Bedrijfsgegevens wijzigen: Studio Noord B.V.");
    assert!(made().is_empty(), "nothing goes out before the answer");
    assert!(state.approvals.decide(proposal.id, true, 2));
    let record = settled(&state, proposal.id).await;
    assert_eq!(record.status, approvals::Status::Done, "{record:?}");
    let body = json!({
        "bedrijf": "Studio Noord B.V.", "naam": "Lisa de Vries", "geslacht": "v", "adres": "Nieuwe Gracht 12",
        "postcode": "3512 AB", "plaats": "Utrecht", "telefoon": "030 123 4567", "email": "info@studionoord.example",
        "website": "studionoord.example", "obNummer": "NL123456782B01", "btwId": "", "kvkNummer": "12345678",
        "iban": "NL18RABO0123459876", "bic": "RABONL2U", "klantcode": "K123"
    });
    assert_eq!(
        made(),
        [format!("PUT administratie/mijngegevens?adm=true {body}")]
    );
    assert_eq!(state.approvals.get(proposal.id).unwrap().body, body);

    // The invoice address: asks too, and goes out under the names the form sends.
    let (_, text, _) = call(
        "factuuradres_wijzigen",
        json!({"email": "boekhouding@studionoord.example", "mobiel": "06 1234 5678"}),
    )
    .await;
    assert!(text.contains("\"status\":\"wacht\""), "{text}");
    let proposal = proposal_up("Factuuradres wijzigen");
    let shown = lines(&proposal);
    for want in [
        (
            "E-mailadres voor facturen was",
            "factuur@studionoord.example",
        ),
        (
            "E-mailadres voor facturen wordt",
            "boekhouding@studionoord.example",
        ),
        ("Mobiel was", "(leeg)"),
        ("Mobiel wordt", "06 1234 5678"),
    ] {
        assert!(
            shown.contains(&(want.0.to_owned(), want.1.to_owned())),
            "{want:?} not in {shown:?}"
        );
    }
    assert!(shown.iter().any(|(label, _)| label == "Gevolg"));
    assert!(state.approvals.decide(proposal.id, true, 3));
    assert_eq!(
        settled(&state, proposal.id).await.status,
        approvals::Status::Done
    );
    let body = json!({
        "bedrijf": "Studio Noord", "contactpersoon": "Lisa", "geslacht": "v", "adres": "Oude Gracht 1",
        "postcode": "3511 AA", "plaats": "Utrecht", "land": "Nederland", "telefoonnummer": "030",
        "mobielnummer": "06 1234 5678", "emailFactuur": "boekhouding@studionoord.example", "faxnummer": ""
    });
    assert_eq!(
        made()[1],
        format!("PUT skillsource/relatie/mijngegevens/factuuradres {body}")
    );

    // e-Boekhouden's own invoices: the list with named columns, the open ones, and one as a pdf.
    let (is_error, text, _) = call("eboekhouden_facturen", json!({})).await;
    assert!(!is_error, "{text}");
    let list: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(list["data"]["data"][0]["nummer"], "SK-104");
    assert_eq!(list["data"]["data"][0]["totaal"], 15.73);
    let (_, text, _) = call("eboekhouden_facturen", json!({"openstaand": true})).await;
    assert!(
        text.contains("SK-105") && !text.contains("SK-104"),
        "{text}"
    );
    let (is_error, text, _) = call("eboekhouden_facturen", json!({"limit": 0})).await;
    assert!(is_error && text.contains("limit"), "{text}");

    let dir = std::env::temp_dir().join(format!("ebmcp-mg-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let (is_error, text, result) = call(
        "eboekhouden_factuur_pdf",
        json!({"id": 8812, "map": dir.to_string_lossy()}),
    )
    .await;
    assert!(!is_error, "{text}");
    let saved = dir.join("SK-8812.pdf");
    assert_eq!(std::fs::read(&saved).unwrap(), b"%PDF-1.4 e-Boekhouden");
    assert_eq!(
        result["content"][1]["resource"]["mimeType"],
        "application/pdf"
    );
    assert!(result["content"][1]["resource"]["blob"].as_str().is_some());
    std::fs::remove_dir_all(&dir).unwrap();

    // Refusing the invoice address sends nothing.
    let (_, text, _) = call("factuuradres_wijzigen", json!({"telefoon": "030 999 0000"})).await;
    assert!(text.contains("\"status\":\"wacht\""), "{text}");
    let proposal = proposal_up("Factuuradres wijzigen");
    assert!(state.approvals.decide(proposal.id, false, 4));
    assert_eq!(
        settled(&state, proposal.id).await.status,
        approvals::Status::Refused
    );
    assert_eq!(made().len(), 2);

    // A BIC that is not found, not valid, or not to be looked up: the BIC stays, and the approval says so.
    for (answer, iban, want) in [
        (
            BicAnswer::Known,
            "NL20INGB0001234567",
            "e-Boekhouden vond geen BIC bij het nieuwe IBAN; de BIC blijft ABNANL2A",
        ),
        (
            BicAnswer::Invalid,
            "NL18RABO0123459876",
            "e-Boekhouden gaf bij het nieuwe IBAN geen geldige BIC (RABO-NL); de BIC blijft ABNANL2A",
        ),
        (
            BicAnswer::Fails,
            "NL18RABO0123459876",
            "EB MCP kon de BIC bij het nieuwe IBAN niet opzoeken; de BIC blijft ABNANL2A",
        ),
    ] {
        *stub.bic.lock().unwrap() = answer;
        let (_, text, _) = call("bedrijfsgegevens_wijzigen", json!({"iban": iban})).await;
        assert!(text.contains("\"status\":\"wacht\""), "{text}");
        let proposal = proposal_up("Bedrijfsgegevens wijzigen: Studio Noord B.V.");
        let shown = lines(&proposal);
        assert!(
            shown
                .iter()
                .any(|(label, value)| label == "BIC" && value.starts_with(want)),
            "{want} not in {shown:?}"
        );
        assert!(
            !shown.iter().any(|(label, _)| label == "BIC wordt"),
            "{shown:?}"
        );
        assert_eq!(
            state.approvals.get(proposal.id).unwrap().body["bic"],
            "ABNANL2A"
        );
        assert!(state.approvals.decide(proposal.id, false, 5));
        settled(&state, proposal.id).await;
    }
    *stub.bic.lock().unwrap() = BicAnswer::Known;
    assert_eq!(made().len(), 2);

    // Approved after another field changed in e-Boekhouden: refused after the approval, nothing goes out.
    let (_, text, _) = call(
        "bedrijfsgegevens_wijzigen",
        json!({"website": "noord.example"}),
    )
    .await;
    assert!(text.contains("\"status\":\"wacht\""), "{text}");
    let proposal = proposal_up("Bedrijfsgegevens wijzigen: Studio Noord B.V.");
    stub.administratie.lock().unwrap()["telefoon"] = json!("030 765 4321");
    assert!(state.approvals.decide(proposal.id, true, 6));
    let record = settled(&state, proposal.id).await;
    assert_eq!(record.status, approvals::Status::Failed, "{record:?}");
    let error = record.error.unwrap_or_default();
    assert!(
        error.contains("intussen gewijzigd in e-Boekhouden")
            && error.contains("vraag de wijziging opnieuw aan"),
        "{error}"
    );
    assert_eq!(made().len(), 2);

    // Only the requested field changed meanwhile: the approved change still goes out, built on the fresh read.
    *stub.administratie.lock().unwrap() = administratie();
    let (_, text, _) = call(
        "bedrijfsgegevens_wijzigen",
        json!({"website": "noord.example"}),
    )
    .await;
    assert!(text.contains("\"status\":\"wacht\""), "{text}");
    let proposal = proposal_up("Bedrijfsgegevens wijzigen: Studio Noord B.V.");
    stub.administratie.lock().unwrap()["website"] = json!("iets-anders.example");
    assert!(state.approvals.decide(proposal.id, true, 7));
    assert_eq!(
        settled(&state, proposal.id).await.status,
        approvals::Status::Done
    );
    let mut body = form_of(&administratie(), BEDRIJF, BEDRIJF_KEEP).unwrap();
    body.insert("website".into(), json!("noord.example"));
    assert_eq!(
        made()[2],
        format!(
            "PUT administratie/mijngegevens?adm=true {}",
            Value::Object(body)
        )
    );

    // No invoice address yet (e-Boekhouden answers null): the required fields must be given, then it is made.
    *stub.factuuradres.lock().unwrap() = Value::Null;
    let (is_error, text, _) = call(
        "factuuradres_wijzigen",
        json!({"email": "f@studionoord.example"}),
    )
    .await;
    assert!(
        is_error && text.contains("Bedrijfsnaam is verplicht"),
        "{text}"
    );
    assert_eq!(state.approvals.count(), 0);
    let (_, text, _) = call(
        "factuuradres_wijzigen",
        json!({"bedrijf": "Studio Noord", "adres": "Oude Gracht 1", "postcode": "3511aa", "plaats": "Utrecht", "email": "f@studionoord.example"}),
    )
    .await;
    assert!(text.contains("\"status\":\"wacht\""), "{text}");
    let proposal = proposal_up("Factuuradres wijzigen");
    assert!(lines(&proposal).contains(&("Adres was".to_owned(), "onbekend".to_owned())));
    assert!(state.approvals.decide(proposal.id, true, 8));
    assert_eq!(
        settled(&state, proposal.id).await.status,
        approvals::Status::Done
    );
    let body = json!({"bedrijf": "Studio Noord", "geslacht": "m", "adres": "Oude Gracht 1", "postcode": "3511 AA", "plaats": "Utrecht", "emailFactuur": "f@studionoord.example"});
    assert_eq!(
        made()[3],
        format!("PUT skillsource/relatie/mijngegevens/factuuradres {body}")
    );

    // Nothing else was changed along the way, on any path or with any method.
    assert_eq!(made().len(), 4);
}

#[test]
fn a_null_under_one_name_does_not_hide_a_value_under_the_other() {
    let read = json!({"bedrijf": "A", "telefoon": null, "telefoonnummer": "030", "mobiel": null, "fax": "", "faxnummer": "071"});
    let form = form_of(&read, FACTUUR, &[]).unwrap();
    assert_eq!(form.get("telefoonnummer"), Some(&json!("030")));
    // Null under every name stays null; an empty text is a value and wins.
    assert_eq!(form.get("mobielnummer"), Some(&Value::Null));
    assert_eq!(form.get("faxnummer"), Some(&json!("")));
}

#[test]
fn sends_after_the_approval_only_what_still_matches_the_fresh_read() {
    let approved = merge(
        &administratie(),
        BEDRIJF,
        BEDRIJF_KEEP,
        vec![("adres", "Nieuwe Gracht 12".into())],
    )
    .unwrap();
    let approved_body = Value::Object(approved.body);
    // Unchanged: the same body goes out.
    let body = rebuild(
        &administratie(),
        BEDRIJF,
        BEDRIJF_KEEP,
        &approved_body,
        &approved.changed,
        "bedrijfsgegevens",
    );
    assert_eq!(body.ok(), Some(approved_body.clone()));
    // Fields outside the form (incasso, id) may change; they never go back anyway.
    let mut fresh = administratie();
    fresh["incassoRekening"] = json!("NL20INGB0001234567");
    assert!(
        rebuild(
            &fresh,
            BEDRIJF,
            BEDRIJF_KEEP,
            &approved_body,
            &approved.changed,
            "bedrijfsgegevens"
        )
        .is_ok()
    );
    // A form field, a kept field, or a field that appeared: refused.
    for (key, value) in [
        ("plaats", json!("Zeist")),
        ("klantcode", json!("K9")),
        ("bic", Value::Null),
    ] {
        let mut fresh = administratie();
        fresh[key] = value;
        let Err(Halt::Error(message)) = rebuild(
            &fresh,
            BEDRIJF,
            BEDRIJF_KEEP,
            &approved_body,
            &approved.changed,
            "bedrijfsgegevens",
        ) else {
            panic!("{key} changed but the change went through");
        };
        assert!(
            message.starts_with("De bedrijfsgegevens zijn intussen gewijzigd in e-Boekhouden"),
            "{message}"
        );
    }
    let mut fresh = administratie();
    fresh.as_object_mut().unwrap().remove("telefoon");
    assert!(
        rebuild(
            &fresh,
            BEDRIJF,
            BEDRIJF_KEEP,
            &approved_body,
            &approved.changed,
            "bedrijfsgegevens"
        )
        .is_err()
    );
    // The requested field itself may differ now: the approved value goes out.
    let mut fresh = administratie();
    fresh["adres"] = json!("Elders 3");
    let Ok(body) = rebuild(
        &fresh,
        BEDRIJF,
        BEDRIJF_KEEP,
        &approved_body,
        &approved.changed,
        "bedrijfsgegevens",
    ) else {
        panic!("refused a change of the requested field only");
    };
    assert_eq!(body["adres"], "Nieuwe Gracht 12");
}
