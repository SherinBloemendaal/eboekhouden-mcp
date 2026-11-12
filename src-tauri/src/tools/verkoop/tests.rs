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
    // As captured live: GET user has no id.
    json!({"naam": "Lisa de Vries", "email": "lisa@atelierzuid.example", "gebruikersnaam": "lisa@atelierzuid.example"})
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
    // Own text goes out as it is: a code in it would reach the customer unfilled.
    args.onderwerp = Some("Offerte [OFFERTENUMMER]".into());
    assert!(
        offerte_mail(&args, None, &relatie(), &user())
            .unwrap_err()
            .contains("zoals hij is")
    );
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
        &json!([{"code": "HOOG_VERK_21", "omschrijving": "Hoog", "percentage": 21, "soort": "VERK"},
                {"code": "LAAG_VERK_9", "omschrijving": "Laag", "percentage": 9, "soort": "VERK"}]),
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
    // Totals from the leading price (excl.) with btw on top: 100 + 21 and 50 + 4,50.
    assert_eq!(shown(&built, "Totaal excl. btw"), "150,00");
    assert_eq!(shown(&built, "Btw"), "25,50");
    assert_eq!(shown(&built, "Totaal incl. btw"), "175,50");
    assert_eq!(
        shown(&built, "Regel 1"),
        "2 x Ontwerp: 50,00 per stuk excl. btw (HOOG_VERK_21), op 8000 Omzet ontwerp"
    );
    assert!(shown(&built, "Meteen boeken").starts_with("ja, de factuur wordt meteen geboekt"));

    // An amount on a quote excl. btw is excl.: 1000 becomes 1210 incl. 21% btw, and the summary says so.
    let ex = json!({"id": 8, "nummer": "OF-00008", "relatieId": 3, "inEx": "EX", "totaalExcl": 1800.0, "totaalIncl": 2178.0});
    let enkel = json!([{"id": 81, "aantal": 1, "omschrijving": "Ontwerp", "prijsExcl": 2000.0, "prijsIncl": 2420.0, "btwCode": "HOOG_VERK_21",
                         "grootboekId": 80, "kostenplaatsId": 0, "kortingIsPerc": true, "korting": 10, "regelMetKortingExcl": 1800.0}]);
    let bron_ex = OfferteBron {
        offerte: &ex,
        regels: &enkel,
        berekend: None,
        relatie: &relatie,
    };
    let mut bedrag = factureren(8);
    bedrag.bedrag = Some(1000.0);
    let built = offerte_factuur(
        &bedrag,
        &bedrag.deel().unwrap(),
        &bron_ex,
        &factuur_context(),
        &books(),
    )
    .unwrap();
    assert_eq!(shown(&built, "Totaal excl. btw"), "1000,00");
    assert_eq!(shown(&built, "Totaal incl. btw"), "1210,00");
    // A line's discount shows in its words, and the total follows the discounted amount.
    let built = offerte_factuur(
        &factureren(8),
        &Deel::Regels(None),
        &bron_ex,
        &factuur_context(),
        &books(),
    )
    .unwrap();
    assert_eq!(
        shown(&built, "Regel 1"),
        "1 x Ontwerp: 2000,00 per stuk excl. btw, korting 10% (HOOG_VERK_21), op 8000 Omzet ontwerp"
    );
    assert_eq!(shown(&built, "Totaal incl. btw"), "2178,00");
    // Incl. leads on a quote incl. btw: the btw is taken out.
    let incl = json!({"id": 9, "nummer": "OF-00009", "relatieId": 3, "inEx": "IN"});
    let regel = json!([{"id": 91, "aantal": 1, "omschrijving": "Boek", "prijsExcl": 3.21, "prijsIncl": 3.5, "btwCode": "LAAG_VERK_9", "grootboekId": 81}]);
    let bron_in = OfferteBron {
        offerte: &incl,
        regels: &regel,
        berekend: None,
        relatie: &relatie,
    };
    let built = offerte_factuur(
        &factureren(9),
        &Deel::Regels(None),
        &bron_in,
        &factuur_context(),
        &books(),
    )
    .unwrap();
    assert_eq!(
        (
            shown(&built, "Totaal excl. btw").as_str(),
            shown(&built, "Totaal incl. btw").as_str()
        ),
        ("3,21", "3,50")
    );
    // An unknown btw code is refused, not guessed.
    let vreemd = json!([{"id": 92, "aantal": 1, "omschrijving": "X", "prijsExcl": 1, "prijsIncl": 1, "btwCode": "ONBEKEND", "grootboekId": 81}]);
    let bron_v = OfferteBron {
        regels: &vreemd,
        ..bron_in
    };
    assert!(
        offerte_factuur(
            &factureren(9),
            &Deel::Regels(None),
            &bron_v,
            &factuur_context(),
            &books()
        )
        .is_err()
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
    // Without a medewerker and without an id from `user`, the tool asks for one.
    assert!(
        agenda_nieuw(&args, "OF-00007", &users(), None)
            .unwrap_err()
            .contains("medewerker_id")
    );
    let built = agenda_nieuw(&args, "OF-00007", &users(), Some(5)).unwrap();
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
    let body = agenda_nieuw(&klaar, "OF-00007", &users(), None)
        .unwrap()
        .body;
    assert_eq!(
        (body["afgehandeld"].as_str(), body["gebruikerId"].as_u64()),
        (Some("2026-11-21T00:00:00.000Z"), Some(6))
    );
    let fout = OfferteAgendaToevoegenArgs {
        medewerker_id: Some(9),
        ..klaar
    };
    assert!(
        agenda_nieuw(&fout, "OF-00007", &users(), Some(5))
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
            Some(5)
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
    let (built, query, lijst) = incasso_plan(
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
    // Right before the file is made the list must be the one approved.
    assert_eq!(
        lijst.nummers,
        vec!["F00010".to_owned(), "F00011".to_owned()]
    );
    assert_eq!(incasso_zelfde(&lijst, &info, &incasso_grid()), Ok(()));
    let erbij = json!({"data": [{"nummer": "F00010", "bedrijf": "Studio Noord", "bedrag": 121.0, "machtigingMessage": ""},
                                {"nummer": "F00012", "bedrijf": "Bakkerij Oost", "bedrag": 60.5, "machtigingMessage": ""}]});
    let changed = incasso_zelfde(&lijst, &info, &erbij).unwrap_err();
    assert!(
        changed.starts_with("Er is geen incassobestand gemaakt")
            && changed.contains("erbij: F00012")
            && changed.contains("niet meer: F00011"),
        "{changed}"
    );
    let meer = incasso_zelfde(&lijst, &json!({"aantal": 3, "bedrag": 250.0}), &json!({"data": [
        {"nummer": "F00010", "bedrag": 121.0}, {"nummer": "F00011", "bedrag": 60.5}, {"nummer": "F00013", "bedrag": 68.5}]})).unwrap_err();
    assert!(
        meer.contains("goedgekeurd waren 2 facturen voor 181,50, nu staan er 3 voor 250,00"),
        "{meer}"
    );
    assert!(
        incasso_zelfde(&lijst, &json!({"aantal": 0}), &json!({"data": []}))
            .unwrap_err()
            .contains("geen facturen")
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

fn eigen(
    onderwerp: Option<&str>,
    bericht: Option<&str>,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    (
        onderwerp.map(str::to_owned),
        bericht.map(str::to_owned),
        None,
        None,
    )
}

fn ontvanger(id: u64, nummer: &str, relatie: &str, aan: &str, filled: &Value) -> Ontvanger {
    let none = None;
    let mail = factuur_mail(
        &EigenMail {
            onderwerp: &none,
            bericht: &none,
            cc: &none,
            bcc: &none,
        },
        Some(filled),
        aan,
        &user(),
        Some("Atelier Zuid"),
    )
    .unwrap();
    Ontvanger {
        factuur_id: id,
        nummer: nummer.into(),
        relatie: relatie.into(),
        mail,
    }
}

#[test]
fn an_invoice_mail_sends_the_filled_template_text_and_refuses_codes_in_own_text() {
    // factuur/maildata as e-Boekhouden answers it: the template's text filled for this invoice.
    let filled = json!({"subject": "Factuur F00012", "body": "<p>Beste Studio Noord, hierbij factuur F00012.</p>", "ccEmail": "",
                        "bccEmail": "archief@atelierzuid.example", "vanNaam": "Atelier Zuid", "vanEmail": "facturen@atelierzuid.example"});
    let none = None;
    let mail = factuur_mail(
        &EigenMail {
            onderwerp: &none,
            bericht: &none,
            cc: &none,
            bcc: &none,
        },
        Some(&filled),
        "jan@studionoord.example",
        &user(),
        None,
    )
    .unwrap();
    assert_eq!(
        mail,
        FactuurMail {
            aan: "jan@studionoord.example".into(),
            cc: None,
            bcc: Some("archief@atelierzuid.example".into()),
            van_naam: "Atelier Zuid".into(),
            van_email: "facturen@atelierzuid.example".into(),
            onderwerp: "Factuur F00012".into(),
            bericht: "<p>Beste Studio Noord, hierbij factuur F00012.</p>".into(),
        }
    );
    assert_eq!(
        factuur_mail_body(&mail, false, 12)["subject"],
        "Factuur F00012"
    );
    // Own text goes out as it is: a code in it is refused, written-out text is fine.
    let (o, b, c, d) = eigen(Some("Factuur [FACTNUMMER]"), Some("Hierbij"));
    let refused = factuur_mail(
        &EigenMail {
            onderwerp: &o,
            bericht: &b,
            cc: &c,
            bcc: &d,
        },
        None,
        "jan@studionoord.example",
        &user(),
        None,
    )
    .unwrap_err();
    assert!(
        refused.contains("[FACTNUMMER]") && refused.contains("zoals hij is"),
        "{refused}"
    );
    let (o, b, c, d) = eigen(Some("Uw factuur"), Some("<p>Beste [RELAANHEF]</p>"));
    assert!(
        factuur_mail(
            &EigenMail {
                onderwerp: &o,
                bericht: &b,
                cc: &c,
                bcc: &d
            },
            None,
            "jan@studionoord.example",
            &user(),
            None
        )
        .unwrap_err()
        .contains("Het bericht")
    );
    let (o, b, c, d) = eigen(Some("Factuur F00012 [concept]"), Some("Hierbij"));
    assert!(
        factuur_mail(
            &EigenMail {
                onderwerp: &o,
                bericht: &b,
                cc: &c,
                bcc: &d
            },
            None,
            "jan@studionoord.example",
            &user(),
            None
        )
        .is_ok(),
        "lower case in brackets is no code"
    );
    // Caps of the mail form.
    let (o, b, c, d) = eigen(Some(&"x".repeat(111)), Some("Hierbij"));
    assert!(
        factuur_mail(
            &EigenMail {
                onderwerp: &o,
                bericht: &b,
                cc: &c,
                bcc: &d
            },
            None,
            "a@b.example",
            &user(),
            None
        )
        .unwrap_err()
        .contains("110")
    );
    let lang = json!({"subject": "Factuur", "body": "x", "vanNaam": "n".repeat(51)});
    assert!(
        factuur_mail(
            &EigenMail {
                onderwerp: &none,
                bericht: &none,
                cc: &none,
                bcc: &none
            },
            Some(&lang),
            "a@b.example",
            &user(),
            None
        )
        .unwrap_err()
        .contains("50")
    );
    assert!(for_invoices(&json!([{"id": 4}]), 4).is_ok());
    assert!(
        for_invoices(&json!([{"id": 4}]), 5)
            .unwrap_err()
            .contains("niet voor facturen")
    );
    assert_eq!(
        own_text_code("a [B] [FACT_NR] c"),
        Some("[FACT_NR]".to_owned())
    );
}

#[test]
fn a_bulk_mail_shows_every_recipient_and_text_as_it_is_sent() {
    let ontvangers = vec![
        ontvanger(
            12,
            "F00012",
            "Studio Noord",
            "facturen@studionoord.example",
            &json!({"subject": "Factuur F00012", "body": "<p>Hierbij F00012</p>", "vanNaam": "Atelier Zuid", "vanEmail": "f@atelierzuid.example"}),
        ),
        ontvanger(
            13,
            "F00013",
            "Bakkerij Oost",
            "info@bakkerijoost.example",
            &json!({"subject": "Factuur F00013", "body": "<p>Hierbij F00013</p>", "vanNaam": "Atelier Zuid", "vanEmail": "f@atelierzuid.example"}),
        ),
    ];
    let built = facturen_mail_built(&ontvangers, false);
    assert_eq!(built.title, "2 facturen mailen");
    assert_eq!(
        built.body["mails"][1],
        json!({"vanNaam": "Atelier Zuid", "vanEmail": "f@atelierzuid.example", "aanEmail": "info@bakkerijoost.example", "ccEmail": null,
               "bccEmail": null, "subject": "Factuur F00013", "body": "<p>Hierbij F00013</p>", "factuurUbl": false,
               "rememberMessage": false, "useEmailSjabloon": false, "bijlagen": [], "factuurId": 13})
    );
    let shown = |label: &str| {
        built
            .lines
            .iter()
            .find(|l| l.label == label)
            .map(|l| l.value.clone())
    };
    assert_eq!(
        shown("Factuur F00012").as_deref(),
        Some("aan facturen@studionoord.example (Studio Noord)")
    );
    assert_eq!(
        shown("Factuur F00013, onderwerp").as_deref(),
        Some("Factuur F00013")
    );
    assert_eq!(
        shown("Factuur F00013, bericht").as_deref(),
        Some("Hierbij F00013")
    );
    assert_eq!(
        shown("Van bij elke mail").as_deref(),
        Some("Atelier Zuid <f@atelierzuid.example>")
    );
    assert!(shown("Let op").unwrap().contains("2 echte e-mails"));
    assert!(
        !built.lines.iter().any(|l| l.value.contains("vult")),
        "no claim that codes get filled"
    );
    // Stopping halfway says what went out, what happened, and what was not tried.
    let three = vec![
        ontvanger(
            12,
            "F00012",
            "A",
            "a@a.example",
            &json!({"subject": "s", "body": "b", "vanEmail": "f@x.example"}),
        ),
        ontvanger(
            13,
            "F00013",
            "B",
            "b@b.example",
            &json!({"subject": "s", "body": "b", "vanEmail": "f@x.example"}),
        ),
        ontvanger(
            14,
            "F00014",
            "C",
            "c@c.example",
            &json!({"subject": "s", "body": "b", "vanEmail": "f@x.example"}),
        ),
    ];
    let certain = bulk_mail_stopped(&three, 1, "Mailbox vol", true);
    assert_eq!(
        certain,
        "Factuur F00013 is niet verstuurd: Mailbox vol. Verstuurd: F00012. Niet geprobeerd: F00014."
    );
    let unknown = bulk_mail_stopped(&three, 0, "e-Boekhouden antwoordde niet binnen 30 s", false);
    assert!(
        unknown.starts_with("Factuur F00012: onbekend, mogelijk verstuurd")
            && unknown.ends_with("Verstuurd: geen. Niet geprobeerd: F00013, F00014."),
        "{unknown}"
    );
    assert!(
        certainly_not_done("e-Boekhouden weigert dit: x")
            && !certainly_not_done("e-Boekhouden antwoordde niet binnen 30 s")
    );
    assert!(!certainly_not_done("Geen verbinding met e-Boekhouden"));
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
}

fn links() -> Value {
    json!({"koppelMutatie": [{"id": 901, "nummer": 16, "datum": "2026-11-11T00:00:00", "bedrag": 2.42}],
           "koppelImport": [{"id": 902, "datum": "2026-11-10T00:00:00", "bedrag": -2.42, "rekening": "NL91"}],
           "koppelRelatie": [], "koppelOfferte": [{"id": 903, "offerteId": 7, "nummer": "OF-00007"}]})
}

#[test]
fn unlinking_names_each_link_and_keeps_the_file() {
    let (built, ids, danger) = ontkoppel_plan("bon.pdf", &links(), Some(16), false).unwrap();
    assert!(!danger, "a mutatie link comes back with archief_koppelen");
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
    let (built, ids, danger) = ontkoppel_plan("bon.pdf", &links(), None, true).unwrap();
    assert!(
        danger,
        "links to a bank line or quote cannot be restored: always asked"
    );
    assert!(
        built
            .lines
            .iter()
            .any(|l| l.label == "Let op" && l.value.contains("niet terugzetten"))
    );
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
/// Switches a test flips to change what the stand-in answers (an invoice set ready for incasso meanwhile, ...).
type Flags = Arc<Mutex<std::collections::BTreeSet<&'static str>>>;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// The answer of the stand-in to one request, or `None` for a path it does not know (404). Only fields the live
/// captures or the web app's code show are used.
fn answer(
    method: &str,
    path: &str,
    body: &Value,
    flags: &std::collections::BTreeSet<&'static str>,
) -> Option<axum::response::Response> {
    use axum::response::IntoResponse;
    let grid = |names: &[&str], data: Value| json!({"colMetadata": names.iter().map(|n| json!({"name": n})).collect::<Vec<_>>(), "data": data});
    let json = |value: Value| Some(axum::Json(value).into_response());
    let pdf = |name: &str| {
        Some(
            (
                [(
                    axum::http::header::CONTENT_DISPOSITION,
                    format!("attachment; filename={name}"),
                )],
                "%PDF-1.4 proef",
            )
                .into_response(),
        )
    };
    let incasso_extra = flags.contains("incasso_extra");
    match (method, path) {
        ("GET", "offerte/7") => json(offerte()),
        ("GET", "offerteregel/getbyofferteid/7") => json(offerteregels()),
        ("GET", "relatie/3") => json(relatie()),
        ("GET", "relatie/4") => json(json!({"id": 4, "bedrijf": "Bakkerij Oost"})),
        ("GET", "relatie/5") => json(
            json!({"id": 5, "bedrijf": "Drukkerij West", "email": "info@drukkerijwest.example"}),
        ),
        ("GET", "user") => json(user()),
        ("GET", "user/selectlist") => json(users()),
        ("GET", "emailsjabloonab/forofferte") => json(json!([{"id": 4, "naam": "Offerte"}])),
        ("GET", "emailsjabloonab/forfactuur") => json(json!([{"id": 6, "naam": "Factuur"}])),
        ("GET", "offerte/getmaildata/7/4") => json(
            json!({"subject": "Offerte OF-00007", "body": "<p>Beste klant</p>", "vanNaam": "Atelier Zuid", "vanEmail": "offertes@atelierzuid.example"}),
        ),
        ("GET", p) if p.starts_with("factuur/maildata/6/") => {
            let id = p.rsplit('/').next().unwrap_or_default();
            json(
                json!({"subject": format!("Factuur nummer {id}"), "body": format!("<p>Hierbij factuur {id}</p>"), "ccEmail": "", "bccEmail": "",
                        "vanNaam": "Atelier Zuid", "vanEmail": "facturen@atelierzuid.example"}),
            )
        }
        ("POST", "offerte/verstuur/7/true") | ("POST", "offerte/verstuur/7/false") => {
            json(json!({"sendCount": 1, "errorCount": 0}))
        }
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
        ("GET", "offerteagenda/gridtable/7") => json(grid(
            &[
                "id",
                "dag",
                "datum",
                "onderwerp",
                "medewerker",
                "afgehandeld",
            ],
            json!([[
                31,
                "vr",
                "2026-11-20T09:05:00",
                "Nabellen",
                "Lisa de Vries",
                null
            ]]),
        )),
        ("GET", "offerteagenda/getordefault/31") => json(
            json!({"datum": "2026-11-20T09:05:00", "gebruikerId": 5, "onderwerp": "Nabellen", "body": null, "afgehandeld": null}),
        ),
        ("GET", "offerteagenda/volgendeactie/7") => json(json!({"datum": "2026-11-20T09:05:00"})),
        ("POST", "offerteagenda") => json(json!({"id": 32})),
        ("PUT", "offerteagenda") | ("DELETE", "offerteagenda/31") => json(json!(true)),
        ("GET", "offertestatus/gridtable") => json(grid(
            &["id", "status"],
            json!([[11, "Ingevoerd"], [12, "Geaccepteerd"]]),
        )),
        ("GET", "grootboekrekening/gridtable") => json(grid(
            &["id", "code", "omschrijving", "categorie"],
            json!([
                [80, "8000", "Omzet ontwerp", "Winst en Verlies"],
                [81, "8010", "Omzet drukwerk", "Winst en Verlies"],
                [13, "1300", "Debiteuren", "Debiteuren"]
            ]),
        )),
        ("GET", "btwinfo/all") => json(
            json!([{"code": "HOOG_VERK_21", "omschrijving": "Hoog", "percentage": 21, "soort": "VERK"},
                                               {"code": "LAAG_VERK_9", "omschrijving": "Laag", "percentage": 9, "soort": "VERK"}]),
        ),
        ("GET", "factuur/nextfactuurnummer") => json(json!("F00009")),
        ("GET", "factuursetting") => json(json!({"factuurSjabloonId": 878203, "verwerken": false})),
        ("POST", "factuur/model") => json(json!({"id": 900, "nummer": "F00009", "totaal": 121.0})),
        ("GET", "folder/5") => json(
            json!({"id": 5, "naam": "bon.pdf", "soort": "F", "isDeleted": false, "koppelCount": 3}),
        ),
        ("GET", "folder/5/koppel") => json(links()),
        ("DELETE", p) if p.starts_with("folderkoppel/") => json(json!(true)),
        ("GET", "administratie/incassoinfo") => json(incasso_admin()),
        ("GET", "factuur/incassoinfo") if incasso_extra => {
            json(json!({"aantal": 3, "bedrag": 250.0}))
        }
        ("GET", "factuur/incassoinfo") => json(json!({"aantal": 2, "bedrag": 181.5})),
        ("GET", "factuur/machtiginggrid") => {
            let mut data = vec![
                json!(["F00010", "Studio Noord", 121.0, ""]),
                json!(["F00011", "Bakkerij Oost", 60.5, ""]),
            ];
            if incasso_extra {
                data.push(json!(["F00013", "Drukkerij West", 68.5, ""]));
            }
            json(grid(
                &["nummer", "bedrijf", "bedrag", "machtigingMessage"],
                json!(data),
            ))
        }
        ("GET", "incassobestand/create") if flags.contains("incasso_timeout") => {
            Some(axum::http::StatusCode::BAD_GATEWAY.into_response())
        }
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
        ("GET", "factuur/14") => {
            json(json!({"id": 14, "nummer": "F00014", "relatieId": 5, "totaal": 68.5}))
        }
        ("GET", "factuur/15") => {
            json(json!({"id": 15, "nummer": "F00015", "relatieId": 5, "totaal": 10.0}))
        }
        ("GET", "factuur/16") => {
            json(json!({"id": 16, "nummer": "F00016", "relatieId": 3, "totaal": 10.0}))
        }
        ("GET", "factuur/12/print") => pdf("F00012.pdf"),
        ("GET", "factuur/14/print") => pdf("F00014.pdf"),
        ("GET", "peppol/getpeppolaccount") if flags.contains("geen_peppol") => json(Value::Null),
        ("GET", "peppol/getpeppolaccount") => json(json!({"peppolId": "0106:87654321"})),
        ("GET", "factuur/haspeppolverzending/12") => {
            json(json!(flags.contains("peppol_al_verstuurd")))
        }
        ("POST", "peppol/send") => Some(axum::http::StatusCode::OK.into_response()),
        ("POST", "factuur/mail") => match body["factuurId"].as_u64() {
            // Invoice 15: e-Boekhouden answers that it did not send. Invoice 16: the request times out at the gateway.
            Some(15) => json(json!({"errorCount": 1, "errorMessage": "Mailbox vol"})),
            Some(16) => Some(axum::http::StatusCode::GATEWAY_TIMEOUT.into_response()),
            _ => json(json!({"errorCount": 0, "errorMessage": ""})),
        },
        _ => None,
    }
}

async fn eboekhouden(seen: Seen, flags: Flags) -> String {
    use axum::response::IntoResponse;
    let app = axum::Router::new().fallback(
        move |method: axum::http::Method, uri: axum::http::Uri, body: axum::body::Bytes| {
            let (seen, flags) = (seen.clone(), flags.clone());
            async move {
                let path = uri.path().trim_start_matches("/v1/api/").to_owned();
                let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                // A GET that makes something (the incasso file) counts as a change, as do all other methods.
                if method != axum::http::Method::GET || path == "incassobestand/create" {
                    let shown = match uri.query() {
                        Some(query) => format!("{path}?{query}"),
                        None => path.clone(),
                    };
                    seen.lock()
                        .unwrap()
                        .push((method.to_string(), shown, body.clone()));
                }
                let flags = flags.lock().unwrap().clone();
                answer(method.as_str(), &path, &body, &flags)
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
    flags: Flags,
}

impl Harness {
    async fn start(mode: WriteMode) -> Self {
        let seen: Seen = Arc::default();
        let flags: Flags = Arc::default();
        let jar = Arc::new(crate::state::testing::FakeJar::default());
        let state = Arc::new(crate::state::testing::state_with(
            jar,
            &eboekhouden(seen.clone(), flags.clone()).await,
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
            flags,
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

    fn flag(&self, name: &'static str, on: bool) {
        let mut flags = self.flags.lock().unwrap();
        if on {
            flags.insert(name);
        } else {
            flags.remove(name);
        }
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

fn shown(proposal: &crate::approvals::Proposal, label: &str) -> String {
    proposal
        .lines
        .iter()
        .find(|l| l.label == label)
        .map(|l| l.value.clone())
        .unwrap_or_default()
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
    let (is_error, text) = h
        .call(
            "offerte_mailen",
            json!({"id": 7, "onderwerp": "Offerte [OFFERTENUMMER]", "bericht": "x"}),
        )
        .await;
    assert!(is_error && text.contains("zoals hij is"), "{text}");
    assert_eq!(h.state.approvals.count(), 0);

    // Mail: the user sees the filled text; refused, nothing goes out.
    let proposal = h
        .waits("offerte_mailen", json!({"id": 7, "sjabloon_id": 4}))
        .await;
    assert!(proposal.danger);
    assert_eq!(
        proposal.title,
        "Offerte OF-00007 mailen naar jan@studionoord.example"
    );
    assert_eq!(shown(&proposal, "Onderwerp"), "Offerte OF-00007");
    assert_eq!(h.decide(proposal.id, false).await.status, Status::Refused);
    assert!(h.changes().is_empty());
    // Approved: exactly the form's request, with the text that was shown.
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
    assert_eq!(changes[0].2["subject"], "Offerte OF-00007");
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

    // An invoice from the quote: refused sends nothing, approved sends exactly the body that was shown.
    let proposal = h
        .waits("offerte_factureren", json!({"id": 7, "regel_ids": [71]}))
        .await;
    assert!(!proposal.danger);
    assert_eq!(proposal.title, "Factuur F00009 maken van offerte OF-00007");
    assert!(shown(&proposal, "Meteen boeken").starts_with("ja"));
    assert_eq!(shown(&proposal, "Totaal incl. btw"), "121,00");
    assert_eq!(h.decide(proposal.id, false).await.status, Status::Refused);
    assert_eq!(h.changes().len(), 4);
    let proposal = h
        .waits(
            "offerte_factureren",
            json!({"id": 7, "regel_ids": [71], "direct_boeken": false}),
        )
        .await;
    assert!(shown(&proposal, "Meteen boeken").starts_with("nee"));
    assert_eq!(h.decide(proposal.id, true).await.status, Status::Done);
    let (method, path, body) = h.changes()[4].clone();
    assert_eq!((method.as_str(), path.as_str()), ("POST", "factuur/model"));
    assert_eq!(
        body,
        json!({
            "factuur": {"relatieId": 3, "nummer": "F00009", "inEx": "EX", "datum": body["factuur"]["datum"], "termijn": 0,
                        "sjabloonId": 878203, "emailSjabloonId": 4, "type": "N", "btwLaag": 0, "btwHoog": 0, "totaal": 0,
                        "kenmerk": "OF-00007", "offerteId": 7},
            "factuurItems": {"insert": [{"id": -1, "factuurId": 0, "prijs": null, "volgorde": 0, "aantal": 2, "omschrijving": "Ontwerp",
                                         "prijsExcl": 50.0, "prijsIncl": 60.5, "btw": "HOOG_VERK_21", "grootboekId": 80, "kostenplaatsId": 0,
                                         "kortingIsPerc": true}],
                             "update": [], "delete": []}
        })
    );

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
async fn the_agenda_of_a_quote_end_to_end() {
    use crate::approvals::Status;
    let h = Harness::start(WriteMode::Ask).await;
    let (is_error, text) = h.call("offerte_agenda", json!({"offerte_id": 7})).await;
    assert!(!is_error, "{text}");
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["data"]["agenda"][0]["onderwerp"], "Nabellen");
    assert_eq!(
        value["data"]["medewerkers"][0],
        json!({"id": 5, "naam": "Lisa de Vries"})
    );

    // `user` has no id: without a medewerker the tool asks for one, and nothing is asked of the user.
    let (is_error, text) = h
        .call("offerte_agenda_toevoegen", json!({"offerte_id": 7, "datum": "2026-11-27", "tijd": "10:00", "onderwerp": "Terugbellen"}))
        .await;
    assert!(is_error && text.contains("medewerker_id"), "{text}");
    assert_eq!(h.state.approvals.count(), 0);
    let proposal = h
        .waits("offerte_agenda_toevoegen", json!({"offerte_id": 7, "datum": "2026-11-27", "tijd": "10:00", "onderwerp": "Terugbellen", "medewerker_id": 6}))
        .await;
    assert_eq!(shown(&proposal, "Medewerker"), "Jan Bakker");
    assert!(h.changes().is_empty());
    assert_eq!(h.decide(proposal.id, true).await.status, Status::Done);
    assert_eq!(
        h.changes()[0],
        (
            "POST".to_owned(),
            "offerteagenda".to_owned(),
            json!({"offerteId": 7, "onderwerp": "Terugbellen", "datum": "2026-11-27T10:00:00.000Z", "gebruikerId": 6})
        )
    );

    let proposal = h
        .waits(
            "offerte_agenda_wijzigen",
            json!({"offerte_id": 7, "id": 31, "afgehandeld": "2026-11-21"}),
        )
        .await;
    h.decide(proposal.id, true).await;
    assert_eq!(
        h.changes()[1],
        (
            "PUT".to_owned(),
            "offerteagenda".to_owned(),
            json!({"id": 31, "offerteId": 7, "onderwerp": "Nabellen", "datum": "2026-11-20T09:05:00.000Z", "gebruikerId": 5, "afgehandeld": "2026-11-21T00:00:00.000Z"})
        )
    );
    let (is_error, text) = h
        .call(
            "offerte_agenda_verwijderen",
            json!({"offerte_id": 7, "id": 99}),
        )
        .await;
    assert!(
        is_error && text.contains("hoort niet bij offerte 7"),
        "{text}"
    );
    let proposal = h
        .waits(
            "offerte_agenda_verwijderen",
            json!({"offerte_id": 7, "id": 31}),
        )
        .await;
    assert!(proposal.danger);
    assert_eq!(h.decide(proposal.id, false).await.status, Status::Refused);
    assert_eq!(h.changes().len(), 2);
    h.decide(
        h.waits(
            "offerte_agenda_verwijderen",
            json!({"offerte_id": 7, "id": 31}),
        )
        .await
        .id,
        true,
    )
    .await;
    assert_eq!(
        h.changes()[2],
        (
            "DELETE".to_owned(),
            "offerteagenda/31".to_owned(),
            Value::Null
        )
    );
}

#[tokio::test]
async fn invoice_mail_sends_the_filled_text_that_was_shown() {
    use crate::approvals::Status;
    let h = Harness::start(WriteMode::Ask).await;

    // factuur_mailen: own text with a code is refused; a template for quotes only is refused.
    let (is_error, text) = h
        .call(
            "factuur_mailen",
            json!({"id": 12, "onderwerp": "Factuur [FACTNUMMER]", "bericht": "x"}),
        )
        .await;
    assert!(is_error && text.contains("[FACTNUMMER]"), "{text}");
    let (is_error, text) = h
        .call("factuur_mailen", json!({"id": 12, "sjabloon_id": 4}))
        .await;
    assert!(is_error && text.contains("niet voor facturen"), "{text}");
    // With a template: the text e-Boekhouden filled for this invoice is shown, and exactly that is sent.
    let proposal = h
        .waits("factuur_mailen", json!({"id": 12, "sjabloon_id": 6}))
        .await;
    assert_eq!(shown(&proposal, "Onderwerp"), "Factuur nummer 12");
    assert_eq!(shown(&proposal, "Bericht"), "Hierbij factuur 12");
    assert_eq!(shown(&proposal, "Aan"), "facturen@studionoord.example");
    assert!(h.changes().is_empty());
    assert_eq!(h.decide(proposal.id, true).await.status, Status::Done);
    let (_, path, body) = h.changes()[0].clone();
    assert_eq!(path, "factuur/mail");
    assert_eq!(
        (body["subject"].as_str(), body["body"].as_str()),
        (Some("Factuur nummer 12"), Some("<p>Hierbij factuur 12</p>"))
    );
    assert_eq!(
        (body["vanEmail"].as_str(), body["factuurId"].as_u64()),
        (Some("facturen@atelierzuid.example"), Some(12))
    );

    // facturen_mailen: the filled text per invoice; refused sends nothing.
    let (is_error, text) = h
        .call(
            "facturen_mailen",
            json!({"ids": [12, 13], "onderwerp": "Uw factuur", "bericht": "Hierbij"}),
        )
        .await;
    assert!(
        is_error && text.contains("F00013 (Bakkerij Oost)"),
        "a relatie without address: {text}"
    );
    let (is_error, text) = h
        .call(
            "facturen_mailen",
            json!({"ids": [12], "onderwerp": "Factuur [FACTNUMMER]", "bericht": "x"}),
        )
        .await;
    assert!(is_error && text.contains("zoals hij is"), "{text}");
    let (is_error, text) = h
        .call("facturen_mailen", json!({"ids": [12], "sjabloon_id": 4}))
        .await;
    assert!(is_error && text.contains("niet voor facturen"), "{text}");
    let proposal = h
        .waits(
            "facturen_mailen",
            json!({"ids": [12, 14], "sjabloon_id": 6}),
        )
        .await;
    assert!(proposal.danger);
    assert_eq!(
        shown(&proposal, "Factuur F00014"),
        "aan info@drukkerijwest.example (Drukkerij West)"
    );
    assert_eq!(
        shown(&proposal, "Factuur F00014, onderwerp"),
        "Factuur nummer 14"
    );
    assert_eq!(h.decide(proposal.id, false).await.status, Status::Refused);
    assert_eq!(h.changes().len(), 1);
    let proposal = h
        .waits(
            "facturen_mailen",
            json!({"ids": [12, 14], "sjabloon_id": 6}),
        )
        .await;
    assert_eq!(h.decide(proposal.id, true).await.status, Status::Done);
    let changes = h.changes();
    assert_eq!(
        (
            changes[1].2["factuurId"].as_u64(),
            changes[1].2["subject"].as_str()
        ),
        (Some(12), Some("Factuur nummer 12"))
    );
    assert_eq!(
        (
            changes[2].2["factuurId"].as_u64(),
            changes[2].2["subject"].as_str()
        ),
        (Some(14), Some("Factuur nummer 14"))
    );

    // Halfway: e-Boekhouden says no for invoice 15; the rest is not tried, and the message says so.
    let proposal = h
        .waits(
            "facturen_mailen",
            json!({"ids": [12, 15, 14], "onderwerp": "Uw factuur", "bericht": "Hierbij"}),
        )
        .await;
    let record = h.decide(proposal.id, true).await;
    assert_eq!(record.status, Status::Failed);
    let error = record.error.unwrap_or_default();
    assert!(error.contains("Factuur F00015 is niet verstuurd: Mailbox vol. Verstuurd: F00012. Niet geprobeerd: F00014."), "{error}");
    assert_eq!(h.changes().len(), 5, "12 and 15 went out, 14 was not tried");
    // A gateway timeout for invoice 16: unknown, maybe sent.
    let proposal = h
        .waits(
            "facturen_mailen",
            json!({"ids": [16, 14], "onderwerp": "Uw factuur", "bericht": "Hierbij"}),
        )
        .await;
    let error = h.decide(proposal.id, true).await.error.unwrap_or_default();
    assert!(
        error.contains("Factuur F00016: onbekend, mogelijk verstuurd")
            && error.contains("Niet geprobeerd: F00014."),
        "{error}"
    );
}

#[tokio::test]
async fn peppol_and_incasso_ask_always_and_check_again_after_approval() {
    use crate::approvals::Status;
    let h = Harness::start(WriteMode::Auto).await;

    // Peppol: always asked, refused sends nothing, the invoice's id is the body.
    let proposal = h.waits("peppol_versturen", json!({"id": 12})).await;
    assert!(proposal.danger && shown(&proposal, "Aan").contains("0106:12345678"));
    assert_eq!(h.decide(proposal.id, false).await.status, Status::Refused);
    assert!(h.changes().is_empty());
    let proposal = h.waits("peppol_versturen", json!({"id": 12})).await;
    assert_eq!(h.decide(proposal.id, true).await.status, Status::Done);
    assert_eq!(
        h.changes()[0],
        ("POST".to_owned(), "peppol/send".to_owned(), json!(12))
    );
    // Sent meanwhile: nothing goes out after the approval.
    let proposal = h.waits("peppol_versturen", json!({"id": 12})).await;
    h.flag("peppol_al_verstuurd", true);
    let record = h.decide(proposal.id, true).await;
    assert_eq!(record.status, Status::Failed);
    assert!(
        record
            .error
            .unwrap_or_default()
            .contains("intussen al via Peppol verstuurd")
    );
    assert_eq!(h.changes().len(), 1);
    // An administration without Peppol is refused plainly, before anything else is read.
    h.flag("peppol_al_verstuurd", false);
    h.flag("geen_peppol", true);
    let (is_error, text) = h.call("peppol_versturen", json!({"id": 12})).await;
    assert!(
        is_error && text.contains("niet op Peppol aangesloten"),
        "{text}"
    );

    // Incasso: refused makes nothing; approved makes the file in the folder named.
    let dir = std::env::temp_dir().join(format!("ebmcp-incasso-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let map = dir.to_str().unwrap();
    let (is_error, text) = h
        .call(
            "incassobestand_maken",
            json!({"uitvoerdatum": "2020-01-01", "map": map}),
        )
        .await;
    assert!(is_error && text.contains("verleden"), "{text}");
    let proposal = h
        .waits(
            "incassobestand_maken",
            json!({"uitvoerdatum": "2099-01-02", "map": map}),
        )
        .await;
    assert!(proposal.danger && proposal.title.contains("2 facturen, 181,50"));
    assert_eq!(h.decide(proposal.id, false).await.status, Status::Refused);
    assert_eq!(h.changes().len(), 1, "no file made");
    // An invoice set ready while the approval waited: no file is made.
    let proposal = h
        .waits(
            "incassobestand_maken",
            json!({"uitvoerdatum": "2099-01-02", "map": map}),
        )
        .await;
    h.flag("incasso_extra", true);
    let record = h.decide(proposal.id, true).await;
    assert_eq!(record.status, Status::Failed);
    let error = record.error.unwrap_or_default();
    assert!(
        error.contains("Er is geen incassobestand gemaakt") && error.contains("erbij: F00013"),
        "{error}"
    );
    assert_eq!(h.changes().len(), 1);
    h.flag("incasso_extra", false);
    // A failure after the request went out: the outcome is unknown, and the user is told where to look.
    let proposal = h
        .waits(
            "incassobestand_maken",
            json!({"uitvoerdatum": "2099-01-02", "map": map}),
        )
        .await;
    h.flag("incasso_timeout", true);
    let error = h.decide(proposal.id, true).await.error.unwrap_or_default();
    assert!(
        error.contains("niet zeker") && error.contains("Oude incassobestanden"),
        "{error}"
    );
    h.flag("incasso_timeout", false);
    let proposal = h
        .waits(
            "incassobestand_maken",
            json!({"uitvoerdatum": "2099-01-02", "map": map}),
        )
        .await;
    assert_eq!(h.decide(proposal.id, true).await.status, Status::Done);
    assert_eq!(
        h.changes().last().unwrap().1,
        "incassobestand/create?iban=NL91ABNA0417164300&incassoidrekhouder=NL00ZZZ123456780000&uitvoerdatum=02-01-2099"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("EBOEK_DD_proef.xml")).unwrap(),
        "<Document>pain.008</Document>"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn unlinking_printing_and_the_codes() {
    use crate::approvals::Status;
    let h = Harness::start(WriteMode::Auto).await;
    // Unlinking from a mutatie needs no question on an account without approval, and removes only that link.
    let (is_error, text) = h
        .call(
            "archief_ontkoppelen",
            json!({"bestand_id": 5, "mutatie_nr": 16}),
        )
        .await;
    assert!(!is_error && text.contains("\"losgemaakt\":1"), "{text}");
    assert_eq!(
        h.changes()[0],
        (
            "DELETE".to_owned(),
            "folderkoppel/901".to_owned(),
            Value::Null
        )
    );
    // All links, a bank line and a quote among them, always ask.
    let proposal = h
        .waits(
            "archief_ontkoppelen",
            json!({"bestand_id": 5, "alles": true}),
        )
        .await;
    assert!(proposal.danger);
    assert!(proposal.lines.iter().any(|l| l.value == "offerte OF-00007"));
    assert_eq!(h.decide(proposal.id, true).await.status, Status::Done);
    let paths: Vec<String> = h.changes().iter().skip(1).map(|c| c.1.clone()).collect();
    assert_eq!(
        paths,
        ["folderkoppel/901", "folderkoppel/902", "folderkoppel/903"]
    );

    // Printing reads the pdfs and saves them; it changes nothing and asks nothing.
    let dir = std::env::temp_dir().join(format!("ebmcp-print-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let (is_error, text) = h
        .call(
            "facturen_afdrukken",
            json!({"ids": [12, 14, 13], "map": dir.to_str().unwrap()}),
        )
        .await;
    assert!(!is_error, "{text}");
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["data"]["opgeslagen"].as_array().unwrap().len(), 2);
    assert_eq!(value["data"]["fouten"][0]["id"], 13);
    assert!(dir.join("F00012.pdf").exists() && dir.join("F00014.pdf").exists());
    std::fs::remove_dir_all(&dir).unwrap();
    let (is_error, text) = h.call("facturen_afdrukken", json!({"ids": [12, 12]})).await;
    assert!(is_error && text.contains("twee keer"), "{text}");

    let (is_error, text) = h.call("emailsjabloon_codes", json!({})).await;
    assert!(!is_error && text.contains("[FACTNUMMER]"), "{text}");
    assert_eq!(h.changes().len(), 4, "reading changes nothing");
    h.set_write(WriteMode::Off);
    let (is_error, text) = h.call("peppol_versturen", json!({"id": 12})).await;
    assert!(is_error && text.contains("alleen-lezen"), "{text}");
    assert_eq!(h.changes().len(), 4);
}
