//! Tests of this part: every body and summary on its own (bodies.rs), and each family of tools end to end over
//! MCP against a stand-in for e-Boekhouden, where a change reaches the stand-in only after the user approves.

use super::*;
use crate::accounts::{AccountInput, Store, WriteMode};
use std::sync::{Arc, Mutex, RwLock};

// ---------------------------------------------------------------- the pure part

fn mail_args() -> OfferteMailenArgs {
    OfferteMailenArgs {
        id: 7,
        aan: None,
        sjabloon_id: None,
        onderwerp: None,
        bericht: None,
        cc: None,
        bcc: None,
        pdf: None,
        account: None,
    }
}

fn relatie() -> Value {
    json!({"id": 3, "bedrijf": "Studio Noord", "email": "jan@studionoord.example", "emailFactuur": "facturen@studionoord.example", "peppolId": "0106:12345678"})
}

fn user() -> Value {
    json!({"id": 5, "naam": "Lisa de Vries", "email": "lisa@atelierzuid.example"})
}

#[test]
fn a_quote_mail_takes_the_template_then_the_relatie_and_the_user() {
    let template = json!({"subject": "Offerte [OFFERTENUMMER]", "body": "<p>Beste [RELAANHEF],</p><a href=\"https://x.example\">kijk</a>", "ccEmail": "", "bccEmail": "archief@atelierzuid.example", "vanNaam": "", "vanEmail": "offertes@atelierzuid.example"});
    let mail = offerte_mail(&mail_args(), Some(&template), &relatie(), &user()).unwrap();
    assert_eq!(
        mail,
        Mail {
            aan: "jan@studionoord.example".into(),
            cc: None,
            bcc: Some("archief@atelierzuid.example".into()),
            van_naam: "Lisa de Vries".into(),
            van_email: "offertes@atelierzuid.example".into(),
            onderwerp: "Offerte [OFFERTENUMMER]".into(),
            bericht: "<p>Beste [RELAANHEF],</p><a href=\"https://x.example\">kijk</a>".into(),
        }
    );
    let built = offerte_mail_built("OF-00007", &mail, true);
    assert_eq!(
        built.body,
        json!({"vanNaam": "Lisa de Vries", "vanEmail": "offertes@atelierzuid.example", "aanEmail": "jan@studionoord.example",
               "ccEmail": null, "bccEmail": "archief@atelierzuid.example", "subject": "Offerte [OFFERTENUMMER]",
               "body": "<p>Beste [RELAANHEF],</p><a href=\"https://x.example\">kijk</a>", "bijlagen": []})
    );
    assert_eq!(
        built.title,
        "Offerte OF-00007 mailen naar jan@studionoord.example"
    );
    let shown = |label: &str| {
        built
            .lines
            .iter()
            .find(|l| l.label == label)
            .map(|l| l.value.clone())
    };
    assert_eq!(
        shown("Bijlage").as_deref(),
        Some("offerte OF-00007 als pdf")
    );
    assert_eq!(
        shown("Links en afbeeldingen in het bericht").as_deref(),
        Some("https://x.example")
    );
    assert!(shown("Let op").unwrap().contains("echte e-mail"));
    assert!(
        offerte_mail_built("OF-00007", &mail, false)
            .lines
            .iter()
            .any(|l| l.value.starts_with("geen"))
    );

    // What the tool is given goes first; bad or missing parts are refused.
    let mut args = mail_args();
    args.aan = Some("klant@elders.example".into());
    args.onderwerp = Some("Uw offerte".into());
    args.bericht = Some("Zie bijlage".into());
    let mail = offerte_mail(&args, None, &json!({}), &user()).unwrap();
    assert_eq!(
        (mail.aan.as_str(), mail.van_email.as_str()),
        ("klant@elders.example", "lisa@atelierzuid.example")
    );
    assert!(
        offerte_mail(&mail_args(), None, &json!({}), &user())
            .unwrap_err()
            .contains("geen e-mailadres")
    );
    assert!(
        offerte_mail(&mail_args(), None, &relatie(), &user())
            .unwrap_err()
            .contains("onderwerp")
    );
    args.cc = Some("geen adres".into());
    assert!(
        offerte_mail(&args, None, &relatie(), &user())
            .unwrap_err()
            .contains("cc-adres")
    );
    args.cc = None;
    args.onderwerp = Some("x".repeat(111));
    assert!(
        offerte_mail(&args, None, &relatie(), &user())
            .unwrap_err()
            .contains("110")
    );

    assert_eq!(mail_failed(&json!({"sendCount": 1, "errorCount": 0})), None);
    assert_eq!(
        mail_failed(&json!({"sendCount": 0, "errorCount": 1, "errorMessage": "SMTP"})).as_deref(),
        Some("SMTP")
    );
    assert!(mail_failed(&json!({"sendCount": 0})).is_some());
}

fn factureren(id: u64) -> OfferteFacturerenArgs {
    OfferteFacturerenArgs {
        id,
        regel_ids: None,
        percentage: None,
        bedrag: None,
        datum: None,
        nummer: None,
        termijn: None,
        direct_boeken: None,
        account: None,
    }
}

fn offerte() -> Value {
    json!({"id": 7, "nummer": "OF-00007", "relatieId": 3, "inEx": "EX", "statusId": 11, "totaalExcl": 150.0, "totaalIncl": 177.5, "factuurEmailsjabloonId": 4})
}

fn offerteregels() -> Value {
    json!([
        {"id": 71, "aantal": 2, "omschrijving": "Ontwerp", "prijsExcl": 50.0, "prijsIncl": 60.5, "btwCode": "HOOG_VERK_21", "grootboekId": 80,
         "kostenplaatsId": 0, "kortingIsPerc": true, "korting": null, "artikelId": null, "regelMetKortingExcl": 100.0, "regelMetKortingIncl": 121.0},
        {"id": 72, "aantal": 1, "omschrijving": "Drukwerk", "prijsExcl": 50.0, "prijsIncl": 56.5, "btwCode": "LAAG_VERK_9", "grootboekId": 81,
         "kostenplaatsId": 0, "kortingIsPerc": true}
    ])
}

fn books() -> Books {
    Books::from_api(
        &json!({"colMetadata": [{"name": "id"}, {"name": "code"}, {"name": "omschrijving"}, {"name": "categorie"}],
                "data": [[80, "8000", "Omzet ontwerp", "Winst en Verlies"], [81, "8010", "Omzet drukwerk", "Winst en Verlies"], [13, "1300", "Debiteuren", "Debiteuren"]]}),
        &json!([{"code": "HOOG_VERK_21", "omschrijving": "Hoog", "percentage": 21, "soort": "VERK"}]),
    )
    .unwrap()
}

fn factuur_context() -> write::FactuurContext {
    write::FactuurContext {
        volgend_nummer: "F00009".into(),
        sjabloon_id: 878203,
        verwerken: false,
        vandaag: (2026, 11, 12),
    }
}

#[test]
fn invoicing_a_quote_copies_its_lines_as_the_web_app_does() {
    let (offerte, regels, relatie) = (offerte(), offerteregels(), relatie());
    let bron = OfferteBron {
        offerte: &offerte,
        regels: &regels,
        berekend: None,
        relatie: &relatie,
    };
    let args = factureren(7);
    let built = offerte_factuur(
        &args,
        &args.deel().unwrap(),
        &bron,
        &factuur_context(),
        &books(),
    )
    .unwrap();
    assert_eq!(
        built.body,
        json!({
            "factuur": {"relatieId": 3, "nummer": "F00009", "inEx": "EX", "datum": {"day": 12, "month": 11, "year": 2026}, "termijn": 0,
                        "sjabloonId": 878203, "emailSjabloonId": 4, "type": "N", "btwLaag": 0, "btwHoog": 0, "totaal": 0,
                        "kenmerk": "OF-00007", "offerteId": 7},
            "factuurItems": {"insert": [
                {"id": -1, "factuurId": 0, "prijs": null, "volgorde": 0, "aantal": 2, "omschrijving": "Ontwerp", "prijsExcl": 50.0, "prijsIncl": 60.5,
                 "btw": "HOOG_VERK_21", "grootboekId": 80, "kostenplaatsId": 0, "kortingIsPerc": true},
                {"id": -2, "factuurId": 0, "prijs": null, "volgorde": 1, "aantal": 1, "omschrijving": "Drukwerk", "prijsExcl": 50.0, "prijsIncl": 56.5,
                 "btw": "LAAG_VERK_9", "grootboekId": 81, "kostenplaatsId": 0, "kortingIsPerc": true}
            ], "update": [], "delete": []},
            "verwerk": {"grootboekrekeningId": 13}
        })
    );
    assert_eq!(built.title, "Factuur F00009 maken van offerte OF-00007");
    let shown = |b: &Built, label: &str| {
        b.lines
            .iter()
            .find(|l| l.label == label)
            .map(|l| l.value.clone())
            .unwrap_or_default()
    };
    assert_eq!(shown(&built, "Relatie"), "Studio Noord");
    assert_eq!(shown(&built, "Totaal incl. btw"), "177,50");
    assert_eq!(
        shown(&built, "Regel 1"),
        "2 x Ontwerp: 50,00 excl. / 60,50 incl. btw per stuk (HOOG_VERK_21), op 8000 Omzet ontwerp"
    );

    // A choice of lines, not booked.
    let mut args = factureren(7);
    args.regel_ids = Some(vec![72]);
    args.direct_boeken = Some(false);
    args.nummer = Some("F01000".into());
    let built = offerte_factuur(
        &args,
        &args.deel().unwrap(),
        &bron,
        &factuur_context(),
        &books(),
    )
    .unwrap();
    let items = built.body["factuurItems"]["insert"].as_array().unwrap();
    assert_eq!(
        (items.len(), items[0]["omschrijving"].as_str()),
        (1, Some("Drukwerk"))
    );
    assert!(built.body.get("verwerk").is_none());
    assert_eq!(built.body["factuur"]["nummer"], "F01000");
    args.regel_ids = Some(vec![99]);
    assert!(
        offerte_factuur(
            &args,
            &args.deel().unwrap(),
            &bron,
            &factuur_context(),
            &books()
        )
        .unwrap_err()
        .contains("Regel 99")
    );

    // A percentage: one line per btw code, the share of the lines with their discount, on the first line's account.
    let mut args = factureren(7);
    args.percentage = Some(50.0);
    let built = offerte_factuur(
        &args,
        &args.deel().unwrap(),
        &bron,
        &factuur_context(),
        &books(),
    )
    .unwrap();
    let items = &built.body["factuurItems"]["insert"];
    assert_eq!(
        items[0]["omschrijving"],
        "Facturatie 50% t.b.v. offerte OF-00007"
    );
    assert_eq!(
        (
            items[0]["code"].as_str(),
            items[0]["prijsExcl"].as_f64(),
            items[0]["prijsIncl"].as_f64()
        ),
        (Some("PERC"), Some(50.0), Some(60.5))
    );
    assert_eq!(
        (
            items[1]["btw"].as_str(),
            items[1]["prijsExcl"].as_f64(),
            items[1]["grootboekId"].as_u64()
        ),
        (Some("LAAG_VERK_9"), Some(25.0), Some(80))
    );
    // An amount: shares of the quote's total.
    let mut args = factureren(7);
    args.bedrag = Some(75.0);
    let built = offerte_factuur(
        &args,
        &args.deel().unwrap(),
        &bron,
        &factuur_context(),
        &books(),
    )
    .unwrap();
    let items = &built.body["factuurItems"]["insert"];
    assert_eq!(
        (items[0]["code"].as_str(), items[0]["prijsExcl"].as_f64()),
        (Some("BEDRAG"), Some(50.0))
    );
    assert_eq!(
        items[0]["omschrijving"],
        "Facturatie t.b.v. offerte OF-00007"
    );
    // Lines e-Boekhouden computed itself are taken as they are.
    let berekend = json!([{"prijsExcl": 12.5, "prijsIncl": 15.13, "btwCode": "HOOG_VERK_21", "grootboekId": 80, "kostenplaatsId": 0}]);
    let bron2 = OfferteBron {
        berekend: Some(&berekend),
        ..bron
    };
    let mut args = factureren(7);
    args.percentage = Some(12.5);
    let built = offerte_factuur(
        &args,
        &args.deel().unwrap(),
        &bron2,
        &factuur_context(),
        &books(),
    )
    .unwrap();
    assert_eq!(
        built.body["factuurItems"]["insert"][0]["omschrijving"],
        "Facturatie 12.5% t.b.v. offerte OF-00007"
    );
    assert_eq!(built.body["factuurItems"]["insert"][0]["prijsIncl"], 15.13);

    // One way at a time, and only sensible numbers.
    let mut args = factureren(7);
    args.percentage = Some(10.0);
    args.bedrag = Some(10.0);
    assert!(args.deel().is_err());
    args.bedrag = None;
    args.percentage = Some(0.0);
    assert!(args.deel().is_err());
    let mut args = factureren(7);
    args.regel_ids = Some(vec![]);
    assert!(args.deel().is_err());
    // A quote without lines, or a line without an account, is refused.
    let leeg = json!([]);
    let bron3 = OfferteBron {
        regels: &leeg,
        ..bron
    };
    assert!(
        offerte_factuur(
            &factureren(7),
            &Deel::Regels(None),
            &bron3,
            &factuur_context(),
            &books()
        )
        .unwrap_err()
        .contains("geen regels")
    );
    let half = json!([{"id": 1, "aantal": 1, "omschrijving": "x", "prijsExcl": 1, "prijsIncl": 1, "btwCode": "GEEN"}]);
    let bron4 = OfferteBron {
        regels: &half,
        ..bron
    };
    assert!(
        offerte_factuur(
            &factureren(7),
            &Deel::Regels(None),
            &bron4,
            &factuur_context(),
            &books()
        )
        .unwrap_err()
        .contains("omzetrekening")
    );
    assert_eq!(js_number(50.0), "50");
    assert_eq!(js_number(12.5), "12.5");
}

fn soorten() -> Value {
    json!([{"id": 1, "caption": "Telefoon"}, {"id": 2, "caption": "Akkoord"}])
}

#[test]
fn progress_entries_name_their_kind() {
    let args = OfferteVoortgangToevoegenArgs {
        offerte_id: 7,
        soort: Some("akkoord".into()),
        soort_id: None,
        onderwerp: " Klant gaat akkoord ".into(),
        bericht: Some("<p>Per telefoon</p>".into()),
        account: None,
    };
    let built = voortgang_nieuw(&args, "OF-00007", &soorten()).unwrap();
    assert_eq!(
        built.body,
        json!({"offerteId": 7, "onderwerp": "Klant gaat akkoord", "voortgangsoortId": 2, "body": "<p>Per telefoon</p>"})
    );
    assert_eq!(
        built.title,
        "Voortgang toevoegen aan offerte OF-00007: Akkoord"
    );
    assert!(
        voortgang_soort(Some("Afgewezen"), None, &soorten())
            .unwrap_err()
            .contains("Telefoon (1), Akkoord (2)")
    );
    assert!(voortgang_soort(Some("x"), Some(1), &soorten()).is_err());
    assert!(
        voortgang_soort(None, Some(9), &json!([]))
            .unwrap_err()
            .contains("nog geen soorten")
    );
    let geen = OfferteVoortgangToevoegenArgs {
        soort: None,
        ..args
    };
    assert!(voortgang_nieuw(&geen, "OF-00007", &soorten()).is_err());

    let now = json!({"id": 21, "onderwerp": "Gebeld", "voortgangsoortId": 1, "body": "<p>oud</p>"});
    let mut wijzig = OfferteVoortgangWijzigenArgs {
        offerte_id: 7,
        id: 21,
        soort: None,
        soort_id: Some(2),
        onderwerp: None,
        bericht: None,
        account: None,
    };
    let built = voortgang_gewijzigd(&wijzig, &now, "OF-00007", &soorten()).unwrap();
    assert_eq!(
        built.body,
        json!({"id": 21, "offerteId": 7, "onderwerp": "Gebeld", "voortgangsoortId": 2, "body": "<p>oud</p>"})
    );
    assert!(
        built
            .lines
            .iter()
            .any(|l| l.label == "Soort wordt" && l.value == "Akkoord (was Telefoon)")
    );
    wijzig.soort_id = Some(1);
    assert!(
        voortgang_gewijzigd(&wijzig, &now, "OF-00007", &soorten())
            .unwrap_err()
            .contains("niets gewijzigd")
    );
    wijzig.onderwerp = Some(" ".into());
    assert!(voortgang_gewijzigd(&wijzig, &now, "OF-00007", &soorten()).is_err());
}

fn users() -> Value {
    json!([{"id": 5, "naam": "Lisa de Vries"}, {"id": 6, "naam": "Jan Bakker"}])
}

#[test]
fn agenda_moments_are_wall_clock_times_as_the_form_writes_them() {
    let args = OfferteAgendaToevoegenArgs {
        offerte_id: 7,
        datum: "2026-11-20".into(),
        tijd: "9:05".into(),
        onderwerp: "Nabellen".into(),
        bericht: None,
        medewerker_id: None,
        afgehandeld: None,
        account: None,
    };
    let built = agenda_nieuw(&args, "OF-00007", &users(), 5).unwrap();
    assert_eq!(
        built.body,
        json!({"offerteId": 7, "onderwerp": "Nabellen", "datum": "2026-11-20T09:05:00.000Z", "gebruikerId": 5})
    );
    assert!(
        built
            .lines
            .iter()
            .any(|l| l.label == "Wanneer" && l.value == "20-11-2026 om 09:05")
    );
    assert!(
        built
            .lines
            .iter()
            .any(|l| l.label == "Medewerker" && l.value == "Lisa de Vries")
    );
    let klaar = OfferteAgendaToevoegenArgs {
        afgehandeld: Some("2026-11-21".into()),
        medewerker_id: Some(6),
        ..args
    };
    let body = agenda_nieuw(&klaar, "OF-00007", &users(), 5).unwrap().body;
    assert_eq!(
        (body["afgehandeld"].as_str(), body["gebruikerId"].as_u64()),
        (Some("2026-11-21T00:00:00.000Z"), Some(6))
    );
    let fout = OfferteAgendaToevoegenArgs {
        medewerker_id: Some(9),
        ..klaar
    };
    assert!(
        agenda_nieuw(&fout, "OF-00007", &users(), 5)
            .unwrap_err()
            .contains("Lisa de Vries (5)")
    );
    assert!(parse_time("24:00").is_err() && parse_time("9.30").is_err());
    assert!(
        agenda_nieuw(
            &OfferteAgendaToevoegenArgs {
                tijd: "25:00".into(),
                medewerker_id: None,
                ..fout
            },
            "OF-00007",
            &users(),
            5
        )
        .is_err()
    );

    let now = json!({"id": 31, "datum": "2026-11-20T09:05:00", "onderwerp": "Nabellen", "gebruikerId": 5, "body": null, "afgehandeld": "2026-11-21T00:00:00"});
    let mut wijzig = OfferteAgendaWijzigenArgs {
        offerte_id: 7,
        id: 31,
        datum: None,
        tijd: Some("14:30".into()),
        onderwerp: None,
        bericht: None,
        medewerker_id: None,
        afgehandeld: Some(String::new()),
        account: None,
    };
    let built = agenda_gewijzigd(&wijzig, &now, "OF-00007", &users()).unwrap();
    assert_eq!(
        built.body,
        json!({"id": 31, "offerteId": 7, "onderwerp": "Nabellen", "datum": "2026-11-20T14:30:00.000Z", "gebruikerId": 5})
    );
    assert!(
        built
            .lines
            .iter()
            .any(|l| l.value == "20-11-2026 om 14:30 (was 20-11-2026 om 09:05)")
    );
    assert!(
        built
            .lines
            .iter()
            .any(|l| l.label == "Afgehandeld" && l.value == "wordt weer open")
    );
    wijzig.tijd = Some("09:05".into());
    wijzig.afgehandeld = None;
    assert!(
        agenda_gewijzigd(&wijzig, &now, "OF-00007", &users())
            .unwrap_err()
            .contains("niets gewijzigd")
    );
}

fn incasso_admin() -> Value {
    json!({"iban": "nl91 abna 0417 1643 00", "incassoContractId": "NL00ZZZ123456780000"})
}

fn incasso_grid() -> Value {
    json!({"data": [{"nummer": "F00010", "bedrijf": "Studio Noord", "bedrag": 121.0, "machtigingMessage": ""},
                    {"nummer": "F00011", "bedrijf": "Bakkerij Oost", "bedrag": 60.5, "machtigingMessage": ""}]})
}

#[test]
fn an_incasso_file_lists_every_invoice_and_uses_the_settings_as_they_are() {
    let info = json!({"aantal": 2, "bedrag": 181.5});
    let (built, query) = incasso_plan(
        (2026, 11, 13),
        (2026, 11, 12),
        &incasso_admin(),
        &info,
        &incasso_grid(),
    )
    .unwrap();
    assert_eq!(
        query,
        vec![
            ("iban".to_owned(), "NL91ABNA0417164300".to_owned()),
            (
                "incassoidrekhouder".to_owned(),
                "NL00ZZZ123456780000".to_owned()
            ),
            ("uitvoerdatum".to_owned(), "13-11-2026".to_owned()),
        ]
    );
    assert_eq!(built.title, "Incassobestand maken: 2 facturen, 181,50");
    assert!(
        built
            .lines
            .iter()
            .any(|l| l.label == "Factuur F00011" && l.value == "Bakkerij Oost: 60,50")
    );
    assert!(
        built
            .lines
            .iter()
            .any(|l| l.label == "Let op" && l.value.contains("betaalopdracht"))
    );
    // Refusals: nothing ready, a problem with a mandate, a list that does not match, a date in the past, no settings.
    let today = (2026, 11, 12);
    assert!(
        incasso_plan(
            (2026, 11, 13),
            today,
            &incasso_admin(),
            &json!({"aantal": 0}),
            &json!({"data": []})
        )
        .unwrap_err()
        .contains("geen facturen")
    );
    let problem = json!({"data": [{"nummer": "F00010", "bedrag": 121.0, "machtigingMessage": "Machtiging ontbreekt"}]});
    assert!(
        incasso_plan(
            (2026, 11, 13),
            today,
            &incasso_admin(),
            &json!({"aantal": 1, "bedrag": 121.0}),
            &problem
        )
        .unwrap_err()
        .contains("F00010: Machtiging ontbreekt")
    );
    assert!(
        incasso_plan(
            (2026, 11, 13),
            today,
            &incasso_admin(),
            &json!({"aantal": 3, "bedrag": 181.5}),
            &incasso_grid()
        )
        .unwrap_err()
        .contains("ziet er 2")
    );
    assert!(
        incasso_plan(
            (2026, 11, 11),
            today,
            &incasso_admin(),
            &info,
            &incasso_grid()
        )
        .unwrap_err()
        .contains("verleden")
    );
    assert!(
        incasso_plan(
            (2026, 11, 13),
            today,
            &json!({"iban": "", "incassoContractId": "X"}),
            &info,
            &incasso_grid()
        )
        .unwrap_err()
        .contains("IBAN")
    );
    assert!(
        incasso_plan(
            (2026, 11, 13),
            today,
            &json!({"iban": "NL91", "incassoContractId": "NL-1"}),
            &info,
            &incasso_grid()
        )
        .unwrap_err()
        .contains("letters en cijfers")
    );
    assert_eq!(next_day((2026, 12, 31)), (2027, 1, 1));
    assert_eq!(safe_file_name(" EBOEK_DD_1.xml "), Some("EBOEK_DD_1.xml"));
    assert_eq!(safe_file_name("../x.xml"), None);
    assert_eq!(safe_file_name("a/b.xml"), None);
}

#[test]
fn peppol_goes_once_and_only_to_a_relatie_with_a_peppol_id() {
    let factuur = json!({"id": 12, "nummer": "F00012", "relatieId": 3, "totaal": 121.0});
    let built = peppol_plan(12, &factuur, &relatie(), &json!(false)).unwrap();
    assert_eq!(built.body, json!(12));
    assert_eq!(
        built.title,
        "Factuur F00012 via Peppol versturen naar Studio Noord"
    );
    assert!(
        built
            .lines
            .iter()
            .any(|l| l.label == "Aan" && l.value == "Studio Noord (Peppol-ID 0106:12345678)")
    );
    assert!(
        peppol_plan(12, &factuur, &relatie(), &json!(true))
            .unwrap_err()
            .contains("maar één keer")
    );
    assert!(
        peppol_plan(
            12,
            &factuur,
            &json!({"bedrijf": "Bakkerij Oost"}),
            &json!(false)
        )
        .unwrap_err()
        .contains("geen Peppol-ID")
    );
}

fn bulk_args(ids: Vec<u64>) -> FacturenMailenArgs {
    FacturenMailenArgs {
        ids,
        sjabloon_id: None,
        onderwerp: Some("Factuur [FACTNUMMER]".into()),
        bericht: Some("<p>Hierbij uw factuur.</p>".into()),
        cc: None,
        bcc: None,
        ubl: None,
        account: None,
    }
}

#[test]
fn a_bulk_mail_names_every_recipient_and_sends_the_single_mail_request_per_invoice() {
    let gemeen = facturen_mail_gemeen(
        &bulk_args(vec![12, 13]),
        None,
        &user(),
        Some("Atelier Zuid"),
    )
    .unwrap();
    assert_eq!(gemeen.van_naam, "Atelier Zuid");
    let ontvangers = vec![
        Ontvanger {
            factuur_id: 12,
            nummer: "F00012".into(),
            relatie: "Studio Noord".into(),
            aan: "facturen@studionoord.example".into(),
        },
        Ontvanger {
            factuur_id: 13,
            nummer: "F00013".into(),
            relatie: "Bakkerij Oost".into(),
            aan: "info@bakkerijoost.example".into(),
        },
    ];
    let built = facturen_mail_built(&gemeen, &ontvangers);
    assert_eq!(built.title, "2 facturen mailen");
    assert_eq!(
        built.body["mails"][1],
        json!({"vanNaam": "Atelier Zuid", "vanEmail": "lisa@atelierzuid.example", "aanEmail": "info@bakkerijoost.example", "ccEmail": null,
               "bccEmail": null, "subject": "Factuur [FACTNUMMER]", "body": "<p>Hierbij uw factuur.</p>", "factuurUbl": false,
               "rememberMessage": false, "useEmailSjabloon": false, "bijlagen": [], "factuurId": 13})
    );
    assert!(built.lines.iter().any(|l| l.label == "Factuur F00012"
        && l.value == "aan facturen@studionoord.example (Studio Noord)"));
    assert!(
        built
            .lines
            .iter()
            .any(|l| l.label == "Let op" && l.value.contains("2 echte e-mails"))
    );
    assert_eq!(
        factuur_adres(&relatie()).as_deref(),
        Some("facturen@studionoord.example")
    );
    assert_eq!(
        factuur_adres(&json!({"email": "a@b.example"})).as_deref(),
        Some("a@b.example")
    );
    assert_eq!(factuur_adres(&json!({})), None);
    assert!(check_ids(&[]).is_err());
    assert!(check_ids(&[1, 2, 1]).unwrap_err().contains("twee keer"));
    assert!(check_ids(&(1..=51).collect::<Vec<_>>()).is_err());
    let mut zonder = bulk_args(vec![1]);
    zonder.onderwerp = None;
    assert!(
        facturen_mail_gemeen(&zonder, None, &user(), None)
            .unwrap_err()
            .contains("onderwerp")
    );
}

fn links() -> Value {
    json!({"koppelMutatie": [{"id": 901, "nummer": 16, "datum": "2026-11-11T00:00:00", "bedrag": 2.42}],
           "koppelImport": [{"id": 902, "datum": "2026-11-10T00:00:00", "bedrag": -2.42, "rekening": "NL91"}],
           "koppelRelatie": [], "koppelOfferte": [{"id": 903, "offerteId": 7, "nummer": "OF-00007"}]})
}

#[test]
fn unlinking_names_each_link_and_keeps_the_file() {
    let (built, ids) = ontkoppel_plan("bon.pdf", &links(), Some(16), false).unwrap();
    assert_eq!(ids, vec![901]);
    assert_eq!(built.title, "Bestand ontkoppelen: bon.pdf");
    assert!(
        built
            .lines
            .iter()
            .any(|l| l.label == "Ontkoppelen van" && l.value == "mutatie 16")
    );
    assert!(
        built
            .lines
            .iter()
            .any(|l| l.label == "Gevolg" && l.value.contains("blijft"))
    );
    let (built, ids) = ontkoppel_plan("bon.pdf", &links(), None, true).unwrap();
    assert_eq!(ids, vec![901, 902, 903]);
    assert!(
        built
            .lines
            .iter()
            .any(|l| l.value == "bankregel van 2026-11-10 (-2,42)")
    );
    assert!(built.lines.iter().any(|l| l.value == "offerte OF-00007"));
    let other = ontkoppel_plan("bon.pdf", &links(), Some(17), false).unwrap_err();
    assert!(
        other.contains("niet gekoppeld aan mutatie 17") && other.contains("mutatie 16"),
        "{other}"
    );
    assert!(ontkoppel_plan("bon.pdf", &links(), Some(16), true).is_err());
    assert!(ontkoppel_plan("bon.pdf", &links(), None, false).is_err());
    assert!(
        ontkoppel_plan("bon.pdf", &json!({}), None, true)
            .unwrap_err()
            .contains("nergens")
    );
}

#[test]
fn the_e_mail_codes_are_unique_and_bracketed() {
    let mut seen = std::collections::BTreeSet::new();
    for (code, uitleg) in EMAIL_CODES {
        assert!(code.starts_with('[') && code.ends_with(']') && !uitleg.is_empty());
        assert!(seen.insert(code), "{code} twice");
    }
    assert!(seen.contains("[FACTNUMMER]") && seen.contains("[ABBEDRIJF]"));
}

// ---------------------------------------------------------------- end to end

/// What the stand-in for e-Boekhouden was asked to change, in order: method, path with query, body.
type Seen = Arc<Mutex<Vec<(String, String, Value)>>>;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// The answer of the stand-in to one request, or `None` for a path it does not know (404).
fn answer(method: &str, path: &str) -> Option<axum::response::Response> {
    use axum::response::IntoResponse;
    let grid = |names: &[&str], data: Value| json!({"colMetadata": names.iter().map(|n| json!({"name": n})).collect::<Vec<_>>(), "data": data});
    let json = |value: Value| Some(axum::Json(value).into_response());
    match (method, path) {
        ("GET", "offerte/7") => json(offerte()),
        ("GET", "offerteregel/getbyofferteid/7") => json(offerteregels()),
        ("GET", "relatie/3") => json(relatie()),
        ("GET", "relatie/4") => json(json!({"id": 4, "bedrijf": "Bakkerij Oost"})),
        ("GET", "user") => json(user()),
        ("GET", "user/selectlist") => json(users()),
        ("GET", "emailsjabloonab/forofferte") => json(json!([{"id": 4, "naam": "Offerte"}])),
        ("GET", "offerte/getmaildata/7/4") => json(
            json!({"subject": "Offerte [OFFERTENUMMER]", "body": "<p>Beste klant</p>", "vanNaam": "Atelier Zuid", "vanEmail": "offertes@atelierzuid.example"}),
        ),
        ("POST", "offerte/verstuur/7/true") => json(json!({"sendCount": 1, "errorCount": 0})),
        ("GET", "voortgangsoort/selectlist") => json(soorten()),
        ("GET", "offertevoortgang/gridtable/7") => json(grid(
            &["id", "datum", "medewerker", "soort", "onderwerp"],
            json!([[21, "2026-11-11T10:00:00", "Lisa", "Telefoon", "Gebeld"]]),
        )),
        ("GET", "offertevoortgang/getordefault/21") => {
            json(json!({"id": 21, "onderwerp": "Gebeld", "voortgangsoortId": 1}))
        }
        ("POST", "offertevoortgang") => json(json!({"id": 22})),
        ("PUT", "offertevoortgang") | ("DELETE", "offertevoortgang/21") => json(json!(true)),
        ("GET", "grootboekrekening/gridtable") => json(grid(
            &["id", "code", "omschrijving", "categorie"],
            json!([
                [80, "8000", "Omzet ontwerp", "Winst en Verlies"],
                [13, "1300", "Debiteuren", "Debiteuren"]
            ]),
        )),
        ("GET", "btwinfo/all") => json(
            json!([{"code": "HOOG_VERK_21", "omschrijving": "Hoog", "percentage": 21, "soort": "VERK"}]),
        ),
        ("GET", "factuur/nextfactuurnummer") => json(json!("F00009")),
        ("GET", "factuursetting") => json(json!({"factuurSjabloonId": 878203, "verwerken": false})),
        ("GET", "offertestatus/gridtable") => json(grid(
            &["id", "status"],
            json!([[11, "Ingevoerd"], [12, "Geaccepteerd"]]),
        )),
        ("GET", "folder/5") => json(
            json!({"id": 5, "naam": "bon.pdf", "soort": "F", "isDeleted": false, "koppelCount": 2}),
        ),
        ("GET", "folder/5/koppel") => json(links()),
        ("DELETE", "folderkoppel/901") => json(json!(true)),
        ("GET", "administratie/incassoinfo") => json(incasso_admin()),
        ("GET", "factuur/incassoinfo") => json(json!({"aantal": 2, "bedrag": 181.5})),
        ("GET", "factuur/machtiginggrid") => json(grid(
            &["nummer", "bedrijf", "bedrag", "machtigingMessage"],
            json!([
                ["F00010", "Studio Noord", 121.0, ""],
                ["F00011", "Bakkerij Oost", 60.5, ""]
            ]),
        )),
        ("GET", "incassobestand/create") => json(json!("EBOEK_DD_proef.xml")),
        ("GET", "incassobestand/download/EBOEK_DD_proef.xml") => Some(
            (
                [(
                    axum::http::header::CONTENT_DISPOSITION,
                    "attachment; filename=EBOEK_DD_proef.xml",
                )],
                "<Document>pain.008</Document>",
            )
                .into_response(),
        ),
        ("GET", "factuur/12") => {
            json(json!({"id": 12, "nummer": "F00012", "relatieId": 3, "totaal": 121.0}))
        }
        ("GET", "factuur/13") => {
            json(json!({"id": 13, "nummer": "F00013", "relatieId": 4, "totaal": 60.5}))
        }
        ("GET", "factuur/haspeppolverzending/12") => json(json!(false)),
        ("POST", "peppol/send") => Some(axum::http::StatusCode::OK.into_response()),
        ("POST", "factuur/mail") => json(json!({"errorCount": 0, "errorMessage": ""})),
        _ => None,
    }
}

async fn eboekhouden(seen: Seen) -> String {
    use axum::response::IntoResponse;
    let app = axum::Router::new().fallback(
        move |method: axum::http::Method, uri: axum::http::Uri, body: axum::body::Bytes| {
            let seen = seen.clone();
            async move {
                let path = uri.path().trim_start_matches("/v1/api/").to_owned();
                // A GET that makes something (the incasso file) counts as a change, as do all other methods.
                if method != axum::http::Method::GET || path == "incassobestand/create" {
                    let shown = match uri.query() {
                        Some(query) => format!("{path}?{query}"),
                        None => path.clone(),
                    };
                    let body = serde_json::from_slice(&body).unwrap_or(Value::Null);
                    seen.lock().unwrap().push((method.to_string(), shown, body));
                }
                answer(method.as_str(), &path)
                    .unwrap_or_else(|| axum::http::StatusCode::NOT_FOUND.into_response())
            }
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://127.0.0.1:{port}/v1/api/")
}

/// EB MCP with one signed-in account, served over HTTP the way a client reaches it.
struct Harness {
    state: Arc<crate::state::AppState>,
    account: uuid::Uuid,
    http: reqwest::Client,
    url: String,
    session: Option<String>,
    seen: Seen,
}

impl Harness {
    async fn start(mode: WriteMode) -> Self {
        let seen: Seen = Arc::default();
        let jar = Arc::new(crate::state::testing::FakeJar::default());
        let state = Arc::new(crate::state::testing::state_with(
            jar,
            &eboekhouden(seen.clone()).await,
        ));
        let account = state.accounts.add(Self::input(mode)).unwrap();
        state.jar_put("sessie", vec![("auth-token".into(), "jwt".into())]);
        state.add_session(account.id, &account.label, "sessie");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let mcp = crate::mcp::router(state.clone(), Arc::new(RwLock::new(TOKEN.to_owned())));
        tokio::spawn(async move { axum::serve(listener, mcp).await.unwrap() });
        let mut harness = Self {
            state,
            account: account.id,
            http: reqwest::Client::new(),
            url: format!("http://127.0.0.1:{port}{}", crate::mcp::PATH),
            session: None,
            seen,
        };
        let init = harness
            .post(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "title": "Claude", "version": "1"}}}))
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

    fn input(mode: WriteMode) -> AccountInput {
        AccountInput {
            label: "Atelier Zuid".into(),
            username: "lisa@atelierzuid.example".into(),
            store: Store::System,
            op_ref: None,
            op_title: None,
            op_vault: None,
            write: mode,
        }
    }

    fn set_write(&self, mode: WriteMode) {
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

    /// Calls a tool: whether it answered with an error, and its text.
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

    /// Calls a tool that must wait for the user, and gives its proposal.
    async fn waits(&self, tool: &str, arguments: Value) -> crate::approvals::Proposal {
        let (is_error, text) = self.call(tool, arguments).await;
        assert!(!is_error, "{tool}: {text}");
        let value: Value =
            serde_json::from_str(&text).unwrap_or_else(|_| panic!("not JSON: {text}"));
        assert_eq!(value["status"], "wacht", "{text}");
        let id = value["voorstel_id"].as_u64().unwrap();
        self.state
            .approvals
            .list()
            .into_iter()
            .find(|p| p.id == id)
            .expect("the proposal is up")
    }

    /// Answers a proposal and waits until its change has run; gives its record.
    async fn decide(&self, id: u64, yes: bool) -> crate::approvals::Record {
        use crate::approvals::Status;
        assert!(self.state.approvals.decide(id, yes, 1));
        for _ in 0..500 {
            if let Some(detail) = self.state.approvals.get(id)
                && !matches!(detail.record.status, Status::Waiting | Status::Running)
                && let Some(crate::reqlog::Entry::Mcp(entry)) =
                    detail.log_id.and_then(|log_id| self.state.log.get(log_id))
                && !matches!(
                    entry.outcome,
                    crate::reqlog::Outcome::Waiting | crate::reqlog::Outcome::Running
                )
            {
                return detail.record;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("proposal {id} did not settle");
    }

    fn changes(&self) -> Vec<(String, String, Value)> {
        self.seen.lock().unwrap().clone()
    }
}

#[tokio::test]
async fn following_up_a_quote_goes_out_only_after_approval() {
    use crate::approvals::Status;
    let h = Harness::start(WriteMode::Ask).await;

    // Bad input is refused before anything is asked or sent.
    let (is_error, text) = h
        .call("offerte_mailen", json!({"id": 7, "aan": "geen adres"}))
        .await;
    assert!(is_error && text.contains("ontvanger"), "{text}");
    let (is_error, text) = h
        .call("offerte_mailen", json!({"id": 7, "sjabloon_id": 99}))
        .await;
    assert!(is_error && text.contains("niet voor offertes"), "{text}");
    assert_eq!(h.state.approvals.count(), 0);

    // Mail: the user sees recipient, subject and message; refused, nothing goes out.
    let proposal = h
        .waits("offerte_mailen", json!({"id": 7, "sjabloon_id": 4}))
        .await;
    assert!(proposal.danger);
    assert_eq!(
        proposal.title,
        "Offerte OF-00007 mailen naar jan@studionoord.example"
    );
    assert!(
        proposal
            .lines
            .iter()
            .any(|l| l.label == "Onderwerp" && l.value == "Offerte [OFFERTENUMMER]")
    );
    assert_eq!(h.decide(proposal.id, false).await.status, Status::Refused);
    assert!(h.changes().is_empty());
    // Approved: exactly the form's request.
    let proposal = h
        .waits("offerte_mailen", json!({"id": 7, "sjabloon_id": 4}))
        .await;
    assert!(h.changes().is_empty(), "nothing before the answer");
    assert_eq!(h.decide(proposal.id, true).await.status, Status::Done);
    let changes = h.changes();
    assert_eq!(changes.len(), 1);
    assert_eq!(
        (changes[0].0.as_str(), changes[0].1.as_str()),
        ("POST", "offerte/verstuur/7/true")
    );
    assert_eq!(changes[0].2["aanEmail"], "jan@studionoord.example");
    assert_eq!(changes[0].2["vanEmail"], "offertes@atelierzuid.example");

    // Mail always asks, also for an account that needs no approval; progress does not.
    h.set_write(WriteMode::Auto);
    let proposal = h
        .waits(
            "offerte_mailen",
            json!({"id": 7, "onderwerp": "Offerte", "bericht": "Zie bijlage", "pdf": false}),
        )
        .await;
    h.decide(proposal.id, true).await;
    assert_eq!(h.changes()[1].1, "offerte/verstuur/7/false");
    let (is_error, text) = h
        .call(
            "offerte_voortgang_toevoegen",
            json!({"offerte_id": 7, "soort": "Akkoord", "onderwerp": "Akkoord per mail"}),
        )
        .await;
    assert!(!is_error && text.contains("\"id\":22"), "{text}");
    assert_eq!(
        h.changes()[2],
        (
            "POST".to_owned(),
            "offertevoortgang".to_owned(),
            json!({"offerteId": 7, "onderwerp": "Akkoord per mail", "voortgangsoortId": 2})
        )
    );
    h.set_write(WriteMode::Ask);

    // Progress of another quote is never touched; removing asks always.
    let (is_error, text) = h
        .call(
            "offerte_voortgang_verwijderen",
            json!({"offerte_id": 7, "id": 99}),
        )
        .await;
    assert!(
        is_error && text.contains("hoort niet bij offerte 7"),
        "{text}"
    );
    let proposal = h
        .waits(
            "offerte_voortgang_verwijderen",
            json!({"offerte_id": 7, "id": 21}),
        )
        .await;
    assert!(proposal.danger);
    h.decide(proposal.id, true).await;
    assert_eq!(h.changes()[3].1, "offertevoortgang/21");

    // An invoice from the quote, read and built before the question.
    let proposal = h
        .waits("offerte_factureren", json!({"id": 7, "regel_ids": [71]}))
        .await;
    assert!(!proposal.danger);
    assert_eq!(proposal.title, "Factuur F00009 maken van offerte OF-00007");
    assert_eq!(h.changes().len(), 4);
    assert_eq!(h.decide(proposal.id, false).await.status, Status::Refused);
    assert_eq!(h.changes().len(), 4);

    // The read tool shows status and progress.
    let (is_error, text) = h.call("offerte_voortgang", json!({"offerte_id": 7})).await;
    assert!(!is_error, "{text}");
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["data"]["offerte"]["status"], "Ingevoerd");
    assert_eq!(value["data"]["voortgang"][0]["onderwerp"], "Gebeld");
    assert_eq!(
        value["data"]["soorten"][1],
        json!({"id": 2, "naam": "Akkoord"})
    );
}

#[tokio::test]
async fn invoice_extras_ask_always_and_send_what_was_shown() {
    use crate::approvals::Status;
    let h = Harness::start(WriteMode::Auto).await;

    // Peppol: always asked, the invoice's id is the body.
    let proposal = h.waits("peppol_versturen", json!({"id": 12})).await;
    assert!(
        proposal.danger
            && proposal
                .lines
                .iter()
                .any(|l| l.value.contains("0106:12345678"))
    );
    assert!(h.changes().is_empty());
    assert_eq!(h.decide(proposal.id, true).await.status, Status::Done);
    assert_eq!(
        h.changes()[0],
        ("POST".to_owned(), "peppol/send".to_owned(), json!(12))
    );

    // Bulk mail: one line per invoice and recipient, one request per invoice.
    let (is_error, text) = h
        .call(
            "facturen_mailen",
            json!({"ids": [12, 13], "onderwerp": "Factuur [FACTNUMMER]", "bericht": "Hierbij"}),
        )
        .await;
    assert!(
        is_error && text.contains("F00013 (Bakkerij Oost)"),
        "a relatie without address: {text}"
    );
    let proposal = h
        .waits(
            "facturen_mailen",
            json!({"ids": [12], "onderwerp": "Factuur [FACTNUMMER]", "bericht": "Hierbij"}),
        )
        .await;
    assert!(proposal.lines.iter().any(|l| l.label == "Factuur F00012"
        && l.value == "aan facturen@studionoord.example (Studio Noord)"));
    h.decide(proposal.id, true).await;
    let mail = &h.changes()[1];
    assert_eq!(
        (mail.1.as_str(), mail.2["factuurId"].as_u64()),
        ("factuur/mail", Some(12))
    );

    // Incasso: the file is made only after approval and lands in the folder named.
    let dir = std::env::temp_dir().join(format!("ebmcp-incasso-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let (is_error, text) = h
        .call(
            "incassobestand_maken",
            json!({"uitvoerdatum": "2020-01-01", "map": dir.to_str().unwrap()}),
        )
        .await;
    assert!(is_error && text.contains("verleden"), "{text}");
    let proposal = h
        .waits(
            "incassobestand_maken",
            json!({"uitvoerdatum": "2099-01-02", "map": dir.to_str().unwrap()}),
        )
        .await;
    assert!(proposal.danger && proposal.title.contains("2 facturen, 181,50"));
    assert_eq!(h.changes().len(), 2, "not made before the answer");
    assert_eq!(h.decide(proposal.id, true).await.status, Status::Done);
    assert_eq!(
        h.changes()[2].1,
        "incassobestand/create?iban=NL91ABNA0417164300&incassoidrekhouder=NL00ZZZ123456780000&uitvoerdatum=02-01-2099"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("EBOEK_DD_proef.xml")).unwrap(),
        "<Document>pain.008</Document>"
    );
    std::fs::remove_dir_all(&dir).unwrap();

    // Unlinking a file needs no question on an account without approval, and removes only that link.
    let (is_error, text) = h
        .call(
            "archief_ontkoppelen",
            json!({"bestand_id": 5, "mutatie_nr": 16}),
        )
        .await;
    assert!(!is_error && text.contains("\"losgemaakt\":1"), "{text}");
    assert_eq!(
        h.changes()[3],
        (
            "DELETE".to_owned(),
            "folderkoppel/901".to_owned(),
            Value::Null
        )
    );

    // Reading writes nothing; the codes come without a call to e-Boekhouden.
    let (is_error, text) = h.call("emailsjabloon_codes", json!({})).await;
    assert!(!is_error && text.contains("[FACTNUMMER]"), "{text}");
    assert_eq!(h.changes().len(), 4);
    h.set_write(WriteMode::Off);
    let (is_error, text) = h.call("peppol_versturen", json!({"id": 12})).await;
    assert!(is_error && text.contains("alleen-lezen"), "{text}");
    assert_eq!(h.changes().len(), 4);
}
