use super::built::*;
use serde_json::{Value, json};

fn post(factuur: &str, relatie_id: u64, open: f64) -> Post {
    Post {
        factuur: factuur.into(),
        relatie_id,
        relatie: "Bakkerij Jansen".into(),
        bedrag: Some(121.0),
        open,
        vervaldatum: "2026-10-01T00:00:00".into(),
    }
}

fn sjabloon() -> Aanmaansjabloon {
    Aanmaansjabloon {
        id: 5,
        naam: "Eerste herinnering".into(),
        onderwerp: "Herinnering factuur [FACTNUMMER]".into(),
        tekst: "<p>Beste [NAAM],</p><p>Factuur [FACTNUMMER] staat nog open: [OPENSTAAND].</p>"
            .into(),
    }
}

fn ontvanger(relatie_id: u64, herinnering: &str, algemeen: &str) -> Ontvanger {
    Ontvanger {
        relatie_id,
        naam: "Bakkerij Jansen".into(),
        herinnering: herinnering.into(),
        factuur: String::new(),
        algemeen: algemeen.into(),
    }
}

fn label<'a>(built: &'a crate::write::Built, label: &str) -> Option<&'a str> {
    built
        .lines
        .iter()
        .find(|l| l.label == label)
        .map(|l| l.value.as_str())
}

#[test]
fn reads_grid_rows_whatever_the_case_of_their_names() {
    let row = json!({"MutFactuur": "F001", "relatieId": "7", "Bedrijf": "Jansen", "sBedrag": "121,50", "teOntvangen": 100});
    assert_eq!(text(&row, "mutFactuur"), "F001");
    assert_eq!(number(&row, "sBedrag"), Some(121.5));
    let post = Post::from_row(&row).unwrap();
    assert_eq!(post.relatie_id, 7);
    assert_eq!(post.relatie, "Jansen");
    assert_eq!(post.open, 100.0);
    assert!(Post::from_row(&json!({"mutFactuur": "", "relatieId": 7, "teOntvangen": 1})).is_none());
    assert!(Post::from_row(&json!({"mutFactuur": "F1", "teOntvangen": 1})).is_none());
    assert_eq!(rows(&json!([1, 2])).len(), 2);
    assert_eq!(rows(&json!({"data": [{"id": 1}]})).len(), 1);
    assert_eq!(moment("2026-11-06T00:00:00"), "06-11-2026");
    assert_eq!(moment("2026-11-06T14:03:12"), "06-11-2026 14:03");
    assert_eq!(moment("gisteren"), "gisteren");
}

#[test]
fn finds_each_open_post_at_its_relatie_and_refuses_what_is_not_open() {
    let open = vec![
        (7, vec![post("F001", 7, 100.0), post("F002", 7, 50.0)]),
        (8, vec![post("F001", 8, 10.0)]),
    ];
    let found = find_posten(&[("F002".into(), 7), ("f001".into(), 8)], &open).unwrap();
    assert_eq!(found[0].factuur, "F002");
    assert_eq!(
        (found[1].factuur.as_str(), found[1].relatie_id),
        ("F001", 8)
    );
    let missing = find_posten(&[("F002".into(), 8)], &open).unwrap_err();
    assert!(missing.contains("staat niet open"), "{missing}");
    assert!(check_wanted(&[]).unwrap_err().contains("minstens één"));
    assert!(
        check_wanted(&[("F1".into(), 7), (" f1 ".into(), 7)])
            .unwrap_err()
            .contains("twee keer")
    );
    assert!(check_wanted(&[("F1".into(), 7), ("F1".into(), 8)]).is_ok());
    assert!(
        check_wanted(&[(" ".into(), 7)])
            .unwrap_err()
            .contains("leeg")
    );
    let many: Vec<(String, u64)> = (0..=MAX_POSTEN).map(|n| (format!("F{n}"), 1)).collect();
    assert!(check_wanted(&many).unwrap_err().contains("Hoogstens"));
    assert_eq!(relatie_ids([8, 7, 8]), vec![7, 8]);
}

#[test]
fn a_reminder_names_template_recipients_and_amounts() {
    let posten = [post("F001", 7, 100.0), post("F002", 7, 50.0)];
    let built = aanmaning(
        &posten,
        &[ontvanger(
            7,
            "debiteuren@jansen.example",
            "info@jansen.example",
        )],
        &sjabloon(),
        Some(" boekhouding@studio.example "),
        true,
    )
    .unwrap();
    assert_eq!(
        built.body,
        json!({
            "CreditDebet": "D", "DatumTot": null,
            "OpenPosten": [{"factuur": "F001", "relatieId": 7}, {"factuur": "F002", "relatieId": 7}],
            "SjabloonId": 5, "FactuurAlsBijlage": true, "KopieSturenNaar": "boekhouding@studio.example",
            "ProefAanmaning": "false"
        })
    );
    assert_eq!(built.title, "Herinneringen versturen voor 2 facturen");
    assert_eq!(label(&built, "Sjabloon"), Some("Eerste herinnering"));
    assert_eq!(
        label(&built, "Onderwerp"),
        Some("Herinnering factuur [FACTNUMMER]")
    );
    assert_eq!(
        label(&built, "Tekst"),
        Some("Beste [NAAM],\nFactuur [FACTNUMMER] staat nog open: [OPENSTAAND].")
    );
    assert_eq!(
        label(&built, "Relatie Bakkerij Jansen"),
        Some(
            "bekende e-mailadressen: debiteuren@jansen.example (herinneringen), info@jansen.example (algemeen)"
        )
    );
    assert!(
        label(&built, "Ontvanger")
            .unwrap()
            .contains("e-Boekhouden kiest zelf")
    );
    assert!(
        built.lines.iter().all(|l| !l.label.starts_with("Aan ")),
        "no line claims who receives the mail: {:?}",
        built.lines
    );
    assert_eq!(
        label(&built, "Factuur F001"),
        Some("Bakkerij Jansen: nog open € 100,00 van € 121,00, vervaldatum 01-10-2026")
    );
    assert_eq!(label(&built, "Totaal openstaand"), Some("€ 150,00"));
    assert_eq!(label(&built, "Bijlage"), Some("de facturen als pdf"));
    assert_eq!(
        label(&built, "Kopie naar"),
        Some("boekhouding@studio.example")
    );
    assert!(label(&built, "Let op").unwrap().contains("echte e-mail"));

    let one = aanmaning(
        &posten[..1],
        &[ontvanger(7, "", "info@jansen.example")],
        &sjabloon(),
        None,
        false,
    )
    .unwrap();
    assert_eq!(
        one.title,
        "Herinnering versturen: factuur F001 aan Bakkerij Jansen"
    );
    assert_eq!(one.body["KopieSturenNaar"], Value::Null);
    assert_eq!(one.body["FactuurAlsBijlage"], false);
    assert_eq!(label(&one, "Kopie naar"), None);
}

#[test]
fn a_reminder_without_address_or_debt_is_refused() {
    let posten = [post("F001", 7, 100.0)];
    let none = aanmaning(&posten, &[ontvanger(7, "", "")], &sjabloon(), None, false).unwrap_err();
    assert!(
        none.contains("Geen van deze relaties heeft een e-mailadres"),
        "{none}"
    );
    let bad_copy = aanmaning(
        &posten,
        &[ontvanger(7, "", "a@b.nl")],
        &sjabloon(),
        Some("geen adres"),
        false,
    )
    .unwrap_err();
    assert!(bad_copy.contains("kopie-adres"), "{bad_copy}");
    let paid = aanmaning(
        &[post("F001", 7, 0.0)],
        &[ontvanger(7, "", "a@b.nl")],
        &sjabloon(),
        None,
        false,
    )
    .unwrap_err();
    assert!(paid.contains("niet meer te ontvangen"), "{paid}");
    // One relatie without an address is shown as not getting one, while the others go.
    let mut other = ontvanger(8, "", "");
    other.naam = "Slagerij Pietersen".into();
    let built = aanmaning(
        &[post("F001", 7, 100.0), post("F009", 8, 20.0)],
        &[ontvanger(7, "", "a@b.nl"), other],
        &sjabloon(),
        None,
        false,
    )
    .unwrap();
    assert!(
        label(&built, "Relatie Slagerij Pietersen")
            .unwrap()
            .contains("verstuurt dan niets")
    );
}

fn rekeningen() -> Vec<Tegenrekening> {
    [
        json!({"id": 11, "code": "1010", "omschrijving": "Bank"}),
        json!({"id": 88, "code": "8990", "omschrijving": "Betalingsverschillen"}),
    ]
    .iter()
    .filter_map(Tegenrekening::from_row)
    .collect()
}

#[test]
fn writes_off_to_betalingsverschillen_unless_told_otherwise() {
    assert_eq!(pick_tegenrekening(&rekeningen(), None).unwrap().id, 88);
    assert_eq!(
        pick_tegenrekening(&rekeningen(), Some(" 1010 "))
            .unwrap()
            .id,
        11
    );
    assert!(
        pick_tegenrekening(&rekeningen(), Some("4000"))
            .unwrap_err()
            .contains("4000")
    );
    assert!(
        pick_tegenrekening(&rekeningen()[..1], None)
            .unwrap_err()
            .contains("tegenrekening")
    );
}

#[test]
fn writing_off_sends_one_item_per_post_and_shows_what_stays_open() {
    let tegen = pick_tegenrekening(&rekeningen(), None).unwrap();
    let built = afboeken(
        Kant::Debiteuren,
        &[post("F001", 7, 100.0)],
        &tegen,
        (2026, 11, 12),
        "Betalingsverschil",
        Some(0.234),
    )
    .unwrap();
    assert_eq!(
        built.body,
        json!([{
            "datum": {"day": 12, "month": 11, "year": 2026}, "bedrag": 0.23, "omschrijving": "Betalingsverschil",
            "tegenrekeningId": 88, "factuurNummer": "F001", "relatieId": 7, "crediteuren": false
        }])
    );
    assert_eq!(
        built.title,
        "Open post afboeken: factuur F001 van Bakkerij Jansen"
    );
    assert_eq!(
        label(&built, "Factuur F001"),
        Some("Bakkerij Jansen: nog open € 100,00, afgeboekt € 0,23, blijft open € 99,77")
    );
    assert_eq!(
        label(&built, "Tegenrekening"),
        Some("8990 Betalingsverschillen")
    );
    assert_eq!(label(&built, "Datum"), Some("12-11-2026"));

    let bulk = afboeken(
        Kant::Crediteuren,
        &[post("I-1", 7, 10.0), post("I-2", 9, -2.5)],
        &tegen,
        (2026, 1, 2),
        "Afronding",
        None,
    )
    .unwrap();
    assert_eq!(bulk.title, "2 open posten afboeken");
    assert_eq!(bulk.body[1]["bedrag"], -2.5);
    assert_eq!(bulk.body[1]["crediteuren"], true);
    assert_eq!(label(&bulk, "Totaal afgeboekt"), Some("€ 7,50"));
    assert_eq!(
        label(&bulk, "Soort"),
        Some("open posten crediteuren (te betalen)")
    );

    let own_amount_for_two = afboeken(
        Kant::Debiteuren,
        &[post("F1", 7, 1.0), post("F2", 7, 1.0)],
        &tegen,
        (2026, 1, 2),
        "x",
        Some(1.0),
    )
    .unwrap_err();
    assert!(own_amount_for_two.contains("één open post"));
    let long = afboeken(
        Kant::Debiteuren,
        &[post("F1", 7, 1.0)],
        &tegen,
        (2026, 1, 2),
        &"x".repeat(51),
        None,
    )
    .unwrap_err();
    assert!(long.contains("50 tekens"));
    let nothing = afboeken(
        Kant::Debiteuren,
        &[post("F1", 7, 1.0)],
        &tegen,
        (2026, 1, 2),
        "x",
        Some(0.001),
    )
    .unwrap_err();
    assert!(nothing.contains("niets af te boeken"));
}

#[test]
fn says_nothing_was_sent_only_when_e_boekhouden_says_so() {
    assert_eq!(
        verzonden(&json!({"sendEmails": 2, "noEmail": 1})).unwrap(),
        json!({"uitkomst": "verstuurd", "verstuurd": 2, "niet_verstuurd_zonder_emailadres": 1})
    );
    assert_eq!(
        verzonden(&json!({"SendEmails": 1})).unwrap()["uitkomst"],
        "verstuurd"
    );
    let none = verzonden(&json!({"sendEmails": 0, "noEmail": 3})).unwrap_err();
    assert!(
        none.contains("geen herinnering verstuurd") && none.contains("3"),
        "{none}"
    );
    assert!(verzonden(&json!({"sendEmails": 0})).is_err());
    // An answer that does not say how many went out may have sent mail: unknown, with the advice to look first.
    for odd in [
        Value::Null,
        json!(true),
        json!({}),
        json!({"sendEmails": "1"}),
        json!({"sendEmails": null}),
    ] {
        let outcome = verzonden(&odd).unwrap();
        assert_eq!(outcome["uitkomst"], "onbekend", "{odd}");
        assert!(
            outcome["let_op"]
                .as_str()
                .unwrap()
                .contains("aanmaning_historie"),
            "{outcome}"
        );
    }
}

#[test]
fn knows_an_accountant_from_the_auth_answer() {
    assert!(is_accountant(&json!({"valid": true, "isAccountant": true})));
    assert!(is_accountant(&json!({"isAccountant": 1})));
    assert!(!is_accountant(
        &json!({"valid": true, "isAccountant": false})
    ));
    assert!(!is_accountant(&json!({"valid": true})));
    assert!(!is_accountant(&json!({"isAccountant": 0})));
}

#[test]
fn an_own_amount_stays_within_what_is_open_and_in_its_direction() {
    let tegen = pick_tegenrekening(&rekeningen(), None).unwrap();
    let afboek = |kant, open: f64, bedrag: f64| {
        afboeken(
            kant,
            &[post("F1", 7, open)],
            &tegen,
            (2026, 1, 2),
            "x",
            Some(bedrag),
        )
    };
    // Part of a debt, and all of it.
    assert_eq!(
        afboek(Kant::Debiteuren, 100.0, 40.0).unwrap().body[0]["bedrag"],
        40.0
    );
    assert_eq!(
        afboek(Kant::Debiteuren, 100.0, 100.0).unwrap().body[0]["bedrag"],
        100.0
    );
    // A creditor post open at -500: written off with a negative amount, never with +500.
    let crediteur = afboek(Kant::Crediteuren, -500.0, -120.0).unwrap();
    assert_eq!(crediteur.body[0]["bedrag"], -120.0);
    assert_eq!(crediteur.body[0]["crediteuren"], true);
    for (kant, open, bedrag) in [
        (Kant::Crediteuren, -500.0, 500.0),
        (Kant::Debiteuren, 100.0, -10.0),
        (Kant::Debiteuren, 100.0, 100.01),
        (Kant::Crediteuren, -500.0, -500.5),
    ] {
        let refused = afboek(kant, open, bedrag).unwrap_err();
        assert!(
            refused.contains("past niet bij wat er open staat"),
            "{refused}"
        );
    }
}

#[test]
fn reads_what_was_written_off() {
    assert_eq!(
        afgeboekt(&json!({"successIds": ["F1"], "errorIds": [{"factuur": "F2", "errorMessage": "Periode is afgesloten"}]})).unwrap(),
        json!({"afgeboekt": ["F1"], "fouten": ["factuur F2: Periode is afgesloten"]})
    );
    let none = afgeboekt(&json!({"successIds": [], "errorIds": [{"factuur": "F2", "errorMessage": "Periode is afgesloten"}]})).unwrap_err();
    assert!(none.contains("niets afgeboekt") && none.contains("afgesloten"));
}

#[test]
fn marks_or_archives_only_messages_in_the_inbox() {
    let inbox = [
        json!({"id": 31, "datum": "2026-11-10T09:15:00", "onderwerp": "Btw Q3", "van": "Cijfers & Co"}),
        json!({"id": 32, "datum": "2026-11-11T00:00:00", "onderwerp": "Welkom"}),
    ];
    let built = berichten_actie(BerichtActie::Gelezen, &[31, 32], &inbox).unwrap();
    assert_eq!(built.body, json!([31, 32]));
    assert_eq!(built.title, "Berichten markeren als gelezen (2)");
    assert_eq!(
        label(&built, "Bericht 31"),
        Some("10-11-2026 09:15 van Cijfers & Co: Btw Q3")
    );
    assert_eq!(label(&built, "Bericht 32"), Some("11-11-2026 Welkom"));
    let one = berichten_actie(BerichtActie::Archiveren, &[32], &inbox).unwrap();
    assert_eq!(one.title, "Bericht naar het archief");
    assert_eq!(BerichtActie::Archiveren.path(), "bericht/archive");
    assert_eq!(BerichtActie::Gelezen.path(), "bericht/markread");
    assert!(
        berichten_actie(BerichtActie::Gelezen, &[40], &inbox)
            .unwrap_err()
            .contains("niet in het postvak")
    );
    assert!(
        berichten_actie(BerichtActie::Gelezen, &[31, 31], &inbox)
            .unwrap_err()
            .contains("twee keer")
    );
    assert!(berichten_actie(BerichtActie::Gelezen, &[], &inbox).is_err());
}

#[test]
fn a_message_goes_to_the_accountant() {
    assert_eq!(
        accountant(&json!({"bedrijf": "Cijfers & Co", "plaats": "Utrecht"})).as_deref(),
        Some("Cijfers & Co te Utrecht")
    );
    assert_eq!(accountant(&json!({"bedrijf": ""})), None);
    assert_eq!(accountant(&Value::Null), None);
    assert_eq!(antwoord_onderwerp("Btw Q3"), "Re: Btw Q3");
    assert_eq!(antwoord_onderwerp("Re: Btw Q3"), "Re: Btw Q3");
    let built = bericht(
        "Cijfers & Co te Utrecht",
        " Re: Btw Q3 ",
        "Klopt, dank je.\n",
        Some("bericht 31: Btw Q3"),
    )
    .unwrap();
    assert_eq!(
        built.body,
        json!({"onderwerp": "Re: Btw Q3", "body": "Klopt, dank je."})
    );
    assert_eq!(
        label(&built, "Aan"),
        Some("Cijfers & Co te Utrecht (de accountant van deze administratie)")
    );
    assert_eq!(label(&built, "Antwoord op"), Some("bericht 31: Btw Q3"));
    assert!(bericht("A", "", "x", None).is_err());
    assert!(bericht("A", &"o".repeat(101), "x", None).is_err());
    assert!(bericht("A", "o", "  ", None).is_err());
}

#[test]
fn a_check_request_names_its_period() {
    let maand = controle_aanvraag(
        "Cijfers & Co",
        false,
        2026,
        Some(3),
        None,
        " Graag ",
        (2026, 11, 12),
    )
    .unwrap();
    assert_eq!(
        maand.body,
        json!({"periodeType": "M", "jaar": 2026, "maand": 3, "kwartaal": 4, "opmerking": "Graag"})
    );
    assert_eq!(maand.title, "Controle-opdracht aanvragen: maart 2026");
    let kwartaal = controle_aanvraag("A", true, 2025, None, Some(2), "", (2026, 11, 12)).unwrap();
    assert_eq!(
        kwartaal.body,
        json!({"periodeType": "K", "jaar": 2025, "maand": 11, "kwartaal": 2, "opmerking": ""})
    );
    assert_eq!(label(&kwartaal, "Periode"), Some("2e kwartaal 2025"));
    assert_eq!(label(&kwartaal, "Opmerking"), None);
    assert!(controle_aanvraag("A", false, 2017, None, None, "", (2026, 11, 12)).is_err());
    assert!(controle_aanvraag("A", false, 2030, None, None, "", (2026, 11, 12)).is_err());
    assert!(controle_aanvraag("A", false, 2026, Some(13), None, "", (2026, 11, 12)).is_err());
    assert!(controle_aanvraag("A", true, 2026, Some(1), None, "", (2026, 11, 12)).is_err());
    assert!(controle_aanvraag("A", false, 2026, None, Some(1), "", (2026, 11, 12)).is_err());
    assert!(controle_aanvraag("A", true, 2026, None, Some(5), "", (2026, 11, 12)).is_err());
}

#[test]
fn an_accountant_approves_or_rejects_a_period() {
    let opdracht = json!({"datum": "2026-11-01T10:00:00", "periode": "Oktober 2026", "opmerkingen": "Graag voor de 15e"});
    let built = controle_beoordeling(12, &opdracht, false, "De bank sluit niet aan").unwrap();
    assert_eq!(
        built.body,
        json!({"id": 12, "accept": false, "opmerking": "De bank sluit niet aan"})
    );
    assert_eq!(built.title, "Controle-opdracht afkeuren: Oktober 2026");
    assert_eq!(
        label(&built, "Opmerkingen van de klant"),
        Some("Graag voor de 15e")
    );
    assert!(controle_beoordeling(12, &opdracht, true, &"x".repeat(8001)).is_err());
}

// ---------------------------------------------------------------- end to end

mod end_to_end {
    use std::sync::{Arc, Mutex, RwLock};

    use serde_json::{Value, json};

    use crate::accounts::{AccountInput, Rights, Store};
    use crate::state::AppState;

    /// What the stand-in for e-Boekhouden was sent: method and path, with the query for a read, and the body.
    type Seen = Arc<Mutex<Vec<(String, Value)>>>;
    /// Answers that replace the stand-in's own, by "METHOD path".
    type Answers = Arc<Mutex<std::collections::HashMap<String, Value>>>;

    /// A stand-in for e-Boekhouden with one open post (F001 of relatie 7), a reminder template, a ledger account
    /// for writing off, a message in the inbox and an accountant.
    async fn eboekhouden(seen: Seen, answers: Answers) -> String {
        use axum::http::{Method, Uri};
        use axum::response::IntoResponse;
        let app = axum::Router::new().fallback(
            move |method: Method, uri: Uri, body: axum::body::Bytes| {
                let (seen, answers) = (seen.clone(), answers.clone());
                async move {
                    let path = uri.path().trim_start_matches("/v1/api/").to_owned();
                    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                    let what = match uri.query() {
                        Some(q) if method == Method::GET => format!("GET {path}?{q}"),
                        _ => format!("{method} {path}"),
                    };
                    seen.lock().unwrap().push((what, body));
                    if let Some(answer) = answers.lock().unwrap().get(&format!("{method} {path}")) {
                        return axum::Json(answer.clone()).into_response();
                    }
                    let answer = match (method.as_str(), path.as_str()) {
                        ("GET", "openpost/gridtable/openpost") => json!({
                            "colMetadata": [{"name": "mutFactuur"}, {"name": "relatieId"}, {"name": "bedrijf"}, {"name": "sBedrag"}, {"name": "sBetaald"}, {"name": "teOntvangen"}, {"name": "vervaldatum"}],
                            "data": [["F001", 7, "Bakkerij Jansen", 121.0, 21.0, 100.0, "2026-10-01T00:00:00"]]
                        }),
                        ("GET", "openpost/gridtable/aanmaning") => json!({
                            "colMetadata": [{"name": "id"}, {"name": "datum"}, {"name": "omschrijving"}, {"name": "per"}],
                            "data": [[1, "2026-10-15T10:00:00", "Eerste herinnering", "Email"]]
                        }),
                        ("GET", "aanmaansjabloon/soort/E") => json!([{"id": 5, "naam": "Eerste herinnering", "soort": "E"}]),
                        ("GET", "aanmaansjabloon/5") => json!({"id": 5, "naam": "Eerste herinnering", "soort": "E", "onderwerp": "Herinnering [FACTNUMMER]", "tekstEmail": "<p>Beste [NAAM]</p>"}),
                        ("GET", "relatie/7") => json!({"id": 7, "bedrijf": "Bakkerij Jansen", "email": "info@jansen.example", "emailHerinnering": ""}),
                        ("POST", "OpenPost/aanmaning") => json!({"sendEmails": 1, "noEmail": 0}),
                        ("GET", "grootboekrekening/selectlist/actief/VWBAL") => json!([{"id": 88, "code": "8990", "omschrijving": "Betalingsverschillen"}]),
                        ("POST", "openpost/wegboeken") => json!({"successIds": ["F001"], "errorIds": []}),
                        ("GET", "bericht/gridtable") => json!({
                            "colMetadata": [{"name": "id"}, {"name": "datum"}, {"name": "onderwerp"}],
                            "data": [[31, "2026-11-10T09:15:00", "Btw Q3"]]
                        }),
                        ("GET", "bericht/31") => json!({"id": 31, "onderwerp": "Btw Q3", "van": "Cijfers & Co", "body": "Klopt de btw?\nOmzet <5000 euro."}),
                        ("GET", "administratie/accountantinfo") => json!({"bedrijf": "Cijfers & Co", "plaats": "Utrecht"}),
                        ("POST", "bericht") => json!({"id": 99}),
                        ("POST", "controleopdracht") => json!({"id": 12}),
                        ("POST", "auth") => json!({"valid": true, "isAccountant": false}),
                        ("GET", "controleopdracht/12") => json!({"id": 12, "datum": "2026-11-01T10:00:00", "periode": "Oktober 2026", "opmerkingen": "Graag"}),
                        ("PUT", _) => json!(true),
                        ("POST", _) => Value::Null,
                        _ => return axum::http::StatusCode::NOT_FOUND.into_response(),
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

    /// An MCP client on a session with the server: calls a tool and gives whether it is an error, and its text.
    struct Client {
        http: reqwest::Client,
        url: String,
        session: Option<String>,
    }

    impl Client {
        async fn start(state: Arc<AppState>) -> Self {
            const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let mcp = crate::mcp::router(state, Arc::new(RwLock::new(TOKEN.to_owned())));
            tokio::spawn(async move { axum::serve(listener, mcp).await.unwrap() });
            let mut client = Self {
                http: reqwest::Client::new(),
                url: format!("http://127.0.0.1:{port}{}", crate::mcp::PATH),
                session: None,
            };
            let init = client
                .post(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "title": "Claude", "version": "1"}}}))
                .await;
            client.session = init
                .headers()
                .get("mcp-session-id")
                .map(|v| v.to_str().unwrap().to_owned());
            client
                .post(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
                .await;
            client
        }

        async fn post(&self, body: Value) -> reqwest::Response {
            let mut request = self
                .http
                .post(&self.url)
                .bearer_auth("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                .header("accept", "application/json, text/event-stream")
                .header("mcp-protocol-version", "2025-06-18")
                .json(&body);
            if let Some(session) = &self.session {
                request = request.header("mcp-session-id", session);
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
    }

    fn waiting_id(text: &str) -> u64 {
        let value: Value =
            serde_json::from_str(text).unwrap_or_else(|_| panic!("not JSON: {text}"));
        assert_eq!(value["status"], "wacht", "{text}");
        value["voorstel_id"].as_u64().unwrap()
    }

    async fn settled(state: &AppState, id: u64) -> crate::approvals::Status {
        use crate::approvals::Status;
        use crate::reqlog::Outcome;
        for _ in 0..500 {
            if let Some(detail) = state.approvals.get(id)
                && !matches!(detail.record.status, Status::Waiting | Status::Running)
                && let Some(crate::reqlog::Entry::Mcp(entry)) =
                    detail.log_id.and_then(|log_id| state.log.get(log_id))
                && !matches!(entry.outcome, Outcome::Waiting | Outcome::Running)
            {
                return detail.record.status;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("proposal {id} did not settle");
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

    #[tokio::test]
    async fn reminders_write_offs_and_messages_go_out_only_after_approval() {
        use crate::approvals::Status;
        let seen: Seen = Arc::default();
        let answers: Answers = Arc::default();
        let jar = Arc::new(crate::state::testing::FakeJar::default());
        let state = Arc::new(crate::state::testing::state_with(
            jar,
            &eboekhouden(seen.clone(), answers.clone()).await,
        ));
        let account = state.accounts.add(input(Rights::OFF)).unwrap();
        state.jar_put("sessie", vec![("auth-token".into(), "jwt".into())]);
        state.add_session(account.id, &account.label, "sessie");
        let client = Client::start(state.clone()).await;
        let changes = || {
            seen.lock()
                .unwrap()
                .iter()
                // `POST auth` only asks who is signed in.
                .filter(|(what, _)| !what.starts_with("GET ") && what != "POST auth")
                .cloned()
                .collect::<Vec<_>>()
        };
        let reads = || {
            seen.lock()
                .unwrap()
                .iter()
                .filter(|(what, _)| what.starts_with("GET "))
                .map(|(what, _)| what.clone())
                .collect::<Vec<_>>()
        };
        // The one proposal that is up.
        let up = || {
            let list = state.approvals.list();
            assert_eq!(list.len(), 1, "{list:?}");
            list.into_iter().next().unwrap()
        };
        let reminder = json!({"posten": [{"factuur": "F001", "relatie_id": 7}]});

        // Reading works on a read-only account; the history of reminders is asked by invoice and relatie.
        let (is_error, text) = client
            .call(
                "aanmaning_historie",
                json!({"factuur": "F001", "relatie_id": 7}),
            )
            .await;
        assert!(!is_error && text.contains("Eerste herinnering"), "{text}");
        assert!(
            reads()
                .iter()
                .any(|r| r.starts_with("GET openpost/gridtable/aanmaning?")
                    && r.contains("factuurNummer=F001")
                    && r.contains("relatieId=7")),
            "{:?}",
            reads()
        );
        let (is_error, text) = client.call("bericht", json!({"id": 31})).await;
        assert!(
            !is_error && text.contains(r#""body":"Klopt de btw?\nOmzet <5000 euro.""#),
            "{text}"
        );

        // A read-only account: every write is refused before anything is read, asked or sent.
        for (tool, arguments) in [
            ("aanmaning_versturen", reminder.clone()),
            ("openpost_afboeken", reminder.clone()),
            ("bericht_gelezen", json!({"ids": [31]})),
            ("bericht_archiveren", json!({"ids": [31]})),
            (
                "bericht_versturen",
                json!({"onderwerp": "Vraag", "bericht": "Hallo"}),
            ),
            (
                "controleopdracht_aanvragen",
                json!({"periode": "maand", "jaar": 2026, "maand": 10}),
            ),
            (
                "controleopdracht_beoordelen",
                json!({"id": 12, "goedkeuren": true}),
            ),
        ] {
            let (is_error, text) = client.call(tool, arguments).await;
            assert!(is_error && text.contains("alleen-lezen"), "{tool}: {text}");
        }
        assert!(state.approvals.list().is_empty());
        assert!(changes().is_empty());

        // An account that needs no approval for ordinary changes still asks before e-mail goes out.
        state
            .accounts
            .update(account.id, input(Rights::AUTO))
            .unwrap();
        let (is_error, text) = client.call("aanmaning_versturen", reminder.clone()).await;
        assert!(!is_error, "{text}");
        let id = waiting_id(&text);
        let proposal = up();
        assert_eq!(proposal.id, id);
        assert!(proposal.danger);
        assert_eq!(
            proposal.title,
            "Herinnering versturen: factuur F001 aan Bakkerij Jansen"
        );
        let line = |label: &str| {
            proposal
                .lines
                .iter()
                .find(|l| l.label == label)
                .map(|l| l.value.clone())
        };
        assert_eq!(
            line("Relatie Bakkerij Jansen").as_deref(),
            Some("bekende e-mailadressen: info@jansen.example (algemeen)")
        );
        assert_eq!(
            line("Factuur F001").as_deref(),
            Some("Bakkerij Jansen: nog open € 100,00 van € 121,00, vervaldatum 01-10-2026")
        );
        assert_eq!(line("Sjabloon").as_deref(), Some("Eerste herinnering"));
        assert!(
            reads()
                .iter()
                .any(|r| r.starts_with("GET openpost/gridtable/openpost?")
                    && r.contains("soort=Debiteur")
                    && r.contains("relatieId=7")),
            "{:?}",
            reads()
        );
        assert!(changes().is_empty(), "nothing is sent before the answer");
        assert!(state.approvals.decide(id, false, 1));
        assert_eq!(settled(&state, id).await, Status::Refused);
        assert!(changes().is_empty(), "refused: nothing was sent");

        let (_, text) = client.call("aanmaning_versturen", reminder.clone()).await;
        let id = waiting_id(&text);
        assert!(state.approvals.decide(id, true, 1));
        assert_eq!(settled(&state, id).await, Status::Done);
        assert_eq!(
            changes(),
            vec![(
                "POST OpenPost/aanmaning".to_owned(),
                json!({"CreditDebet": "D", "DatumTot": null, "OpenPosten": [{"factuur": "F001", "relatieId": 7}],
                       "SjabloonId": 5, "FactuurAlsBijlage": false, "KopieSturenNaar": null, "ProefAanmaning": "false"})
            )]
        );
        seen.lock().unwrap().clear();

        // Asking for an open post that is not open is refused without a proposal; nothing goes out.
        let (is_error, text) = client
            .call(
                "openpost_afboeken",
                json!({"posten": [{"factuur": "F404", "relatie_id": 7}]}),
            )
            .await;
        assert!(is_error && text.contains("staat niet open"), "{text}");
        assert!(state.approvals.list().is_empty());
        assert!(changes().is_empty());

        // Writing off always asks, and shows the amount and the ledger account.
        let (_, text) = client
            .call(
                "openpost_afboeken",
                json!({"posten": [{"factuur": "F001", "relatie_id": 7}], "datum": "2026-11-12"}),
            )
            .await;
        let id = waiting_id(&text);
        let proposal = up();
        assert!(proposal.danger);
        assert!(
            proposal
                .lines
                .iter()
                .any(|l| l.label == "Tegenrekening" && l.value == "8990 Betalingsverschillen"),
            "{:?}",
            proposal.lines
        );
        assert!(changes().is_empty());
        assert!(state.approvals.decide(id, true, 1));
        assert_eq!(settled(&state, id).await, Status::Done);
        assert_eq!(
            changes(),
            vec![(
                "POST openpost/wegboeken".to_owned(),
                json!([{"datum": {"day": 12, "month": 11, "year": 2026}, "bedrag": 100.0, "omschrijving": "Betalingsverschil",
                        "tegenrekeningId": 88, "factuurNummer": "F001", "relatieId": 7, "crediteuren": false}])
            )]
        );
        seen.lock().unwrap().clear();

        // Archiving is an ordinary change: on this account it goes without asking.
        let (is_error, text) = client
            .call("bericht_archiveren", json!({"ids": [31]}))
            .await;
        assert!(!is_error, "{text}");
        assert!(text.contains("naar het archief verplaatst"), "{text}");
        assert_eq!(
            changes(),
            vec![("POST bericht/archive".to_owned(), json!([31]))]
        );
        seen.lock().unwrap().clear();

        // A message to the accountant always asks, even here.
        let (_, text) = client
            .call(
                "bericht_versturen",
                json!({"antwoord_op": 31, "bericht": "Ja, die klopt."}),
            )
            .await;
        let id = waiting_id(&text);
        let proposal = up();
        assert!(proposal.danger);
        assert_eq!(
            proposal.title,
            "Bericht versturen aan Cijfers & Co te Utrecht: Re: Btw Q3"
        );
        assert!(changes().is_empty());
        assert!(state.approvals.decide(id, true, 1));
        assert_eq!(settled(&state, id).await, Status::Done);
        assert_eq!(
            changes(),
            vec![(
                "POST bericht".to_owned(),
                json!({"onderwerp": "Re: Btw Q3", "body": "Ja, die klopt."})
            )]
        );
        seen.lock().unwrap().clear();

        // With approval asked for every change: marking as read waits too, and a refused check request sends nothing.
        state
            .accounts
            .update(account.id, input(Rights::ASK))
            .unwrap();
        let (_, text) = client.call("bericht_gelezen", json!({"ids": [31]})).await;
        let id = waiting_id(&text);
        assert!(!up().danger);
        assert!(changes().is_empty());
        assert!(state.approvals.decide(id, true, 1));
        assert_eq!(settled(&state, id).await, Status::Done);
        assert_eq!(
            changes(),
            vec![("POST bericht/markread".to_owned(), json!([31]))]
        );
        seen.lock().unwrap().clear();

        let (is_error, text) = client
            .call(
                "controleopdracht_aanvragen",
                json!({"periode": "maand", "jaar": 1990}),
            )
            .await;
        assert!(is_error && text.contains("Kies een jaar"), "{text}");
        assert!(
            reads().is_empty(),
            "bad input is refused before anything is read"
        );
        let (_, text) = client
            .call(
                "controleopdracht_aanvragen",
                json!({"periode": "kwartaal", "jaar": crate::tools::today().0, "kwartaal": 1, "opmerking": "Graag"}),
            )
            .await;
        let id = waiting_id(&text);
        assert!(up().danger);
        assert!(state.approvals.decide(id, false, 1));
        assert_eq!(settled(&state, id).await, Status::Refused);
        assert!(changes().is_empty());

        // e-Boekhouden takes a reminder but does not say how many went out: the outcome is unknown, not an error.
        let answer = |key: &str, value: Value| {
            answers.lock().unwrap().insert(key.to_owned(), value);
        };
        let status_of = |id: u64| {
            let client = &client;
            async move {
                let (_, text) = client
                    .call("akkoord_status", json!({"voorstel_id": id}))
                    .await;
                serde_json::from_str::<Value>(&text).unwrap()
            }
        };
        answer("POST OpenPost/aanmaning", json!(true));
        let (_, text) = client.call("aanmaning_versturen", reminder.clone()).await;
        let id = waiting_id(&text);
        assert!(state.approvals.decide(id, true, 1));
        assert_eq!(settled(&state, id).await, Status::Done);
        let status = status_of(id).await;
        assert_eq!(
            status["resultaat"]["data"]["uitkomst"], "onbekend",
            "{status}"
        );
        assert!(
            status["resultaat"]["data"]["let_op"]
                .as_str()
                .unwrap()
                .contains("aanmaning_historie"),
            "{status}"
        );
        // It says that nothing went out: then it is an error.
        answer(
            "POST OpenPost/aanmaning",
            json!({"sendEmails": 0, "noEmail": 1}),
        );
        let (_, text) = client.call("aanmaning_versturen", reminder.clone()).await;
        let id = waiting_id(&text);
        assert!(state.approvals.decide(id, true, 1));
        assert_eq!(settled(&state, id).await, Status::Failed);
        assert!(
            status_of(id).await["fout"]
                .as_str()
                .unwrap()
                .contains("geen herinnering verstuurd")
        );
        seen.lock().unwrap().clear();

        // An administration without an accountant: no message can go, and nothing is asked or sent.
        answer("GET administratie/accountantinfo", json!({}));
        let (is_error, text) = client
            .call(
                "bericht_versturen",
                json!({"onderwerp": "Vraag", "bericht": "Hallo"}),
            )
            .await;
        assert!(
            is_error && text.contains("niet aan een accountant gekoppeld"),
            "{text}"
        );
        assert!(state.approvals.list().is_empty());
        assert!(changes().is_empty());

        // Only an accountant judges a check request; this account is none.
        let (is_error, text) = client
            .call(
                "controleopdracht_beoordelen",
                json!({"id": 12, "goedkeuren": true}),
            )
            .await;
        assert!(
            is_error && text.contains("niet van een accountant"),
            "{text}"
        );

        // An accountant's login: sending a message or asking for a check is refused, judging one goes through approval.
        answer("POST auth", json!({"valid": true, "isAccountant": true}));
        let (is_error, text) = client
            .call(
                "bericht_versturen",
                json!({"onderwerp": "Vraag", "bericht": "Hallo"}),
            )
            .await;
        assert!(is_error && text.contains("van een accountant"), "{text}");
        let (is_error, text) = client
            .call(
                "controleopdracht_aanvragen",
                json!({"periode": "maand", "jaar": crate::tools::today().0, "maand": 1}),
            )
            .await;
        assert!(is_error && text.contains("van een accountant"), "{text}");
        assert!(state.approvals.list().is_empty());
        assert!(changes().is_empty());
        let (_, text) = client
            .call(
                "controleopdracht_beoordelen",
                json!({"id": 12, "goedkeuren": false, "opmerking": "De bank sluit niet aan"}),
            )
            .await;
        let id = waiting_id(&text);
        let proposal = up();
        assert!(proposal.danger);
        assert_eq!(proposal.title, "Controle-opdracht afkeuren: Oktober 2026");
        assert!(changes().is_empty(), "nothing is sent before the answer");
        assert!(state.approvals.decide(id, true, 1));
        assert_eq!(settled(&state, id).await, Status::Done);
        assert_eq!(
            changes(),
            vec![(
                "PUT controleopdracht/accountant/opmerking".to_owned(),
                json!({"id": 12, "accept": false, "opmerking": "De bank sluit niet aan"})
            )]
        );
    }
}
