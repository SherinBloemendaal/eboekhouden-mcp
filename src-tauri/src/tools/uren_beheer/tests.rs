//! Tests of the Uren tools: the bodies and summaries `bouw` builds, the refusals of bad input, and every write
//! family end to end against a stand-in for e-Boekhouden (nothing goes out before the user approves).

use std::sync::Mutex;

use super::*;

fn find<'a>(built: &'a Built, label: &str) -> &'a str {
    built
        .lines
        .iter()
        .find(|l| l.label == label)
        .map(|l| l.value.as_str())
        .unwrap_or_else(|| panic!("no line {label:?} in {:?}", built.lines))
}

fn namen() -> Namen {
    Namen {
        projecten: [(10, "Website".to_owned()), (11, "Onderhoud".to_owned())].into(),
        activiteiten: [(20, "Ontwerp".to_owned()), (21, "Advies".to_owned())].into(),
        medewerkers: [(3, "Lisa".to_owned()), (4, "Piet".to_owned())].into(),
    }
}

fn uur_args() -> UurToevoegenArgs {
    UurToevoegenArgs {
        datum: "2026-11-11".into(),
        project_id: 10,
        activiteit_id: 20,
        uren: Some(1.5),
        kilometers: None,
        opmerkingen: Some(" Ontwerp homepage ".into()),
        medewerker_id: None,
        account: None,
    }
}

fn wijzig_uur(id: u64) -> UurWijzigenArgs {
    UurWijzigenArgs {
        id,
        datum: None,
        project_id: None,
        activiteit_id: None,
        uren: None,
        kilometers: None,
        opmerkingen: None,
        medewerker_id: None,
        account: None,
    }
}

fn uur_nu() -> Value {
    json!({"id": 7, "gebruikerId": 3, "datum": "2026-11-10T00:00:00", "projectId": 10, "activiteitId": 20,
           "aantalUren": 2, "aantalKilometers": null, "opmerkingen": "Overleg"})
}

// ---------------------------------------------------------------- helpers

#[test]
fn reads_dates_and_writes_numbers_the_way_e_boekhouden_does() {
    assert_eq!(datum_of(&json!("2026-11-06T00:00:00")), Some((2026, 11, 6)));
    assert_eq!(
        datum_of(&json!({"day": 6, "month": 11, "year": 2026})),
        Some((2026, 11, 6))
    );
    assert_eq!(
        datum_of(&json!({"day": 31, "month": 2, "year": 2026})),
        None
    );
    assert_eq!(datum_of(&json!(null)), None);
    assert_eq!(
        datum_object((2026, 1, 9)),
        json!({"day": 9, "month": 1, "year": 2026})
    );
    assert_eq!(nl_datum((2026, 1, 9)), "09-01-2026");
    assert_eq!(getal(1.5), "1,5");
    assert_eq!(getal(8.0), "8");
    assert_eq!(getal(0.25), "0,25");
    assert_eq!(laatste_dag(2024, 2), (2024, 2, 29));
    assert_eq!(laatste_dag(2026, 2), (2026, 2, 28));
    assert_eq!(laatste_dag(2026, 11), (2026, 11, 30));
    assert_eq!(laatste_dag(2026, 12), (2026, 12, 31));
}

#[test]
fn asks_the_invoicing_list_with_its_screens_filters() {
    let query = factureren_filters(
        Some("2026-11-01"),
        Some("2026-11-30"),
        None,
        Some(10),
        None,
        ZoekenIn::NogNietGefactureerd,
        false,
    )
    .unwrap();
    let get = |key: &str| {
        query
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(get("periodStart"), Some("20261101"));
    assert_eq!(get("periodEnd"), Some("20261130"));
    assert_eq!(get("projectId"), Some("10"));
    assert_eq!(get("medewerkerId"), None);
    assert_eq!(get("zoekenIn"), Some("FACTUREREN"));
    assert_eq!(get("nietFacturabel"), Some("false"));
    assert_eq!(get("showOpmerkingen"), Some("true"));
    let alle = factureren_filters(None, None, None, None, None, ZoekenIn::Alle, true).unwrap();
    assert!(alle.contains(&("zoekenIn".into(), "ALLE".into())));
    assert!(alle.contains(&("nietFacturabel".into(), "true".into())));
    assert!(
        factureren_filters(
            Some("2026-12-01"),
            Some("2026-11-01"),
            None,
            None,
            None,
            ZoekenIn::Alle,
            false
        )
        .unwrap_err()
        .contains("van ligt na tot")
    );
    assert!(
        factureren_filters(
            Some("2026-02-30"),
            None,
            None,
            None,
            None,
            ZoekenIn::Alle,
            false
        )
        .is_err()
    );
}

#[test]
fn names_grid_rows() {
    let rows = rijen(
        json!({"colMetadata": [{"name": "id"}, {"name": "naam"}], "data": [[1, "a"], [2, "b"]]}),
    );
    assert_eq!(
        rows,
        vec![json!({"id": 1, "naam": "a"}), json!({"id": 2, "naam": "b"})]
    );
    assert!(rijen(json!({"data": "x"})).is_empty());
}

// ---------------------------------------------------------------- uren

#[test]
fn a_new_hour_line_is_sent_as_the_hours_form_sends_it() {
    let built = uur_nieuw(&uur_args(), &namen()).unwrap();
    assert_eq!(
        built.body,
        json!({"aantalUren": 1.5, "datum": {"day": 11, "month": 11, "year": 2026}, "activiteitId": 20,
               "projectId": 10, "opmerkingen": "Ontwerp homepage"})
    );
    assert_eq!(built.title, "Uren toevoegen: Website op 11-11-2026");
    assert_eq!(find(&built, "Project"), "Website");
    assert_eq!(find(&built, "Activiteit"), "Ontwerp");
    assert_eq!(find(&built, "Uren"), "1,5");
    assert!(!built.lines.iter().any(|l| l.label == "Kilometers"));

    // Kilometres only, for another medewerker, without remarks.
    let mut args = uur_args();
    args.uren = None;
    args.kilometers = Some(42.0);
    args.opmerkingen = Some("   ".into());
    args.medewerker_id = Some(4);
    let built = uur_nieuw(&args, &namen()).unwrap();
    assert_eq!(
        built.body,
        json!({"gebruikerId": 4, "datum": {"day": 11, "month": 11, "year": 2026}, "activiteitId": 20,
               "projectId": 10, "aantalKilometers": 42.0})
    );
    assert_eq!(find(&built, "Medewerker"), "Piet");
    assert_eq!(find(&built, "Kilometers"), "42");
}

#[test]
fn refuses_an_hour_line_e_boekhouden_would_not_take() {
    let refused = |change: fn(&mut UurToevoegenArgs)| {
        let mut args = uur_args();
        change(&mut args);
        uur_nieuw(&args, &namen()).unwrap_err()
    };
    assert!(refused(|a| a.uren = None).contains("Aantal is niet ingevuld"));
    assert!(refused(|a| a.uren = Some(0.0)).contains("Aantal is niet ingevuld"));
    assert!(refused(|a| a.uren = Some(-1.0)).contains("0 tot 100000"));
    assert!(refused(|a| a.uren = Some(100_001.0)).contains("0 tot 100000"));
    assert!(refused(|a| a.uren = Some(f64::NAN)).contains("0 tot 100000"));
    assert!(refused(|a| a.datum = "11-11-2026".into()).contains("Ongeldige datum"));
    // Names that were not looked up show as ids, never as nothing.
    let built = uur_nieuw(&uur_args(), &Namen::default()).unwrap();
    assert_eq!(find(&built, "Project"), "project 10");
}

#[test]
fn a_changed_hour_line_keeps_what_was_not_named() {
    let mut args = wijzig_uur(7);
    args.uren = Some(3.0);
    args.project_id = Some(11);
    let built = uur_gewijzigd(&args, &uur_nu(), &namen()).unwrap();
    assert_eq!(
        built.body,
        json!({"gebruikerId": 3, "aantalUren": 3.0, "datum": {"day": 10, "month": 11, "year": 2026},
               "activiteitId": 20, "projectId": 11, "opmerkingen": "Overleg", "id": 7})
    );
    assert_eq!(built.title, "Urenregel van 10-11-2026 wijzigen");
    assert_eq!(find(&built, "Urenregel"), "10-11-2026, Website, Ontwerp");
    assert_eq!(find(&built, "Uren was"), "2");
    assert_eq!(find(&built, "Uren wordt"), "3");
    assert_eq!(find(&built, "Project was"), "Website");
    assert_eq!(find(&built, "Project wordt"), "Onderhoud");
    assert!(!built.lines.iter().any(|l| l.label.starts_with("Datum")));

    // Emptying the remarks and moving the date, for another medewerker.
    let mut args = wijzig_uur(7);
    args.opmerkingen = Some(String::new());
    args.datum = Some("2026-11-12".into());
    args.medewerker_id = Some(4);
    let built = uur_gewijzigd(&args, &uur_nu(), &namen()).unwrap();
    assert_eq!(built.body["opmerkingen"], "");
    assert_eq!(built.body["gebruikerId"], 4);
    assert_eq!(
        built.body["datum"],
        json!({"day": 12, "month": 11, "year": 2026})
    );
    assert_eq!(find(&built, "Opmerkingen wordt"), "(leeg)");
    assert_eq!(find(&built, "Medewerker was"), "Lisa");
    assert_eq!(find(&built, "Medewerker wordt"), "Piet");
}

#[test]
fn refuses_a_change_of_an_hour_line_that_changes_nothing_or_empties_it() {
    let err = uur_gewijzigd(&wijzig_uur(7), &uur_nu(), &namen()).unwrap_err();
    assert!(err.contains("niets gewijzigd"), "{err}");
    let mut same = wijzig_uur(7);
    same.uren = Some(2.0);
    assert!(
        uur_gewijzigd(&same, &uur_nu(), &namen())
            .unwrap_err()
            .contains("niets gewijzigd")
    );
    // 0 empties the field: it is left out of what is sent, as an empty field in the form.
    let mut km_leeg = wijzig_uur(7);
    km_leeg.kilometers = Some(0.0);
    km_leeg.opmerkingen = Some("Anders".into());
    let built = uur_gewijzigd(&km_leeg, &uur_nu(), &namen()).unwrap();
    assert!(
        built.body.get("aantalKilometers").is_none(),
        "{}",
        built.body
    );
    let mut alleen_km = uur_args();
    alleen_km.uren = Some(0.0);
    alleen_km.kilometers = Some(5.0);
    let built = uur_nieuw(&alleen_km, &namen()).unwrap();
    assert!(built.body.get("aantalUren").is_none(), "{}", built.body);
    let mut empty = wijzig_uur(7);
    empty.uren = Some(0.0);
    assert!(
        uur_gewijzigd(&empty, &uur_nu(), &namen())
            .unwrap_err()
            .contains("Aantal is niet ingevuld")
    );
    assert!(
        uur_gewijzigd(&wijzig_uur(7), &json!(null), &namen())
            .unwrap_err()
            .contains("onbekende vorm")
    );
    assert!(
        uur_gewijzigd(&wijzig_uur(7), &json!({"projectId": 10}), &namen())
            .unwrap_err()
            .contains("zonder datum")
    );
}

#[test]
fn shows_what_a_deleted_hour_line_was() {
    let built = uur_weg(7, &uur_nu(), &namen());
    assert_eq!(built.body, Value::Null);
    assert_eq!(built.title, "Urenregel van 10-11-2026 verwijderen");
    assert_eq!(find(&built, "Project"), "Website");
    assert_eq!(find(&built, "Uren"), "2");
    assert_eq!(find(&built, "Kilometers"), "(leeg)");
    assert!(find(&built, "Gevolg").contains("niet terug te draaien"));
}

// ---------------------------------------------------------------- projecten

fn gebruikers() -> Vec<(u64, String)> {
    vec![(3, "Lisa".into()), (4, "Piet".into())]
}

fn project_args() -> ProjectToevoegenArgs {
    ProjectToevoegenArgs {
        naam: " Onderhoud ".into(),
        relatie_id: 5,
        omschrijving: None,
        facturabel: None,
        actief: None,
        gebruiker_ids: None,
        account: None,
    }
}

#[test]
fn a_new_project_starts_with_every_user_as_in_the_form() {
    let built = project_nieuw(&project_args(), "Bakkerij Jansen", &gebruikers()).unwrap();
    assert_eq!(
        built.body,
        json!({"project": {"relatieId": 5, "naam": "Onderhoud", "facturabel": false, "actief": true, "omschrijving": ""},
               "users": [3, 4]})
    );
    assert_eq!(built.title, "Project toevoegen: Onderhoud");
    assert_eq!(find(&built, "Relatie"), "Bakkerij Jansen");
    assert_eq!(find(&built, "Gebruikers"), "Lisa, Piet");

    let mut args = project_args();
    args.gebruiker_ids = Some(vec![4, 4]);
    args.facturabel = Some(true);
    args.omschrijving = Some("Maandelijks".into());
    let built = project_nieuw(&args, "Bakkerij Jansen", &gebruikers()).unwrap();
    assert_eq!(built.body["users"], json!([4]));
    assert_eq!(built.body["project"]["facturabel"], true);
    assert_eq!(find(&built, "Omschrijving"), "Maandelijks");
}

#[test]
fn refuses_a_project_without_a_name_or_with_an_unknown_user() {
    let mut args = project_args();
    args.naam = "  ".into();
    assert!(
        project_nieuw(&args, "x", &gebruikers())
            .unwrap_err()
            .contains("niet ingevuld")
    );
    args.naam = "x".repeat(51);
    assert!(
        project_nieuw(&args, "x", &gebruikers())
            .unwrap_err()
            .contains("50 tekens")
    );
    let mut args = project_args();
    args.gebruiker_ids = Some(vec![9]);
    let err = project_nieuw(&args, "x", &gebruikers()).unwrap_err();
    assert!(err.contains("id 9") && err.contains("3 Lisa"), "{err}");
}

fn project_nu() -> Value {
    json!({"project": {"id": 10, "naam": "Website", "relatieId": 5, "omschrijving": "", "facturabel": true, "actief": true},
           "users": [3]})
}

fn wijzig_project() -> ProjectWijzigenArgs {
    ProjectWijzigenArgs {
        id: 10,
        naam: None,
        relatie_id: None,
        omschrijving: None,
        facturabel: None,
        actief: None,
        gebruiker_ids: None,
        account: None,
    }
}

#[test]
fn a_changed_project_keeps_its_users_and_shows_old_and_new() {
    let relaties: HashMap<u64, String> =
        [(5, "Bakkerij Jansen".into()), (6, "Slagerij Smit".into())].into();
    let mut args = wijzig_project();
    args.actief = Some(false);
    args.relatie_id = Some(6);
    let built = project_gewijzigd(&args, &project_nu(), &relaties, &gebruikers()).unwrap();
    assert_eq!(
        built.body,
        json!({"project": {"relatieId": 6, "naam": "Website", "facturabel": true, "actief": false, "omschrijving": "", "id": 10},
               "users": [3]})
    );
    assert_eq!(find(&built, "Relatie was"), "Bakkerij Jansen");
    assert_eq!(find(&built, "Relatie wordt"), "Slagerij Smit");
    assert_eq!(find(&built, "Actief was"), "ja");
    assert_eq!(find(&built, "Actief wordt"), "nee");

    let mut args = wijzig_project();
    args.gebruiker_ids = Some(vec![4, 3]);
    let built = project_gewijzigd(&args, &project_nu(), &relaties, &gebruikers()).unwrap();
    assert_eq!(built.body["users"], json!([4, 3]));
    assert_eq!(find(&built, "Gebruikers waren"), "Lisa");
    assert_eq!(find(&built, "Gebruikers worden"), "Piet, Lisa");

    // The same users in another order are no change.
    let mut args = wijzig_project();
    args.gebruiker_ids = Some(vec![3]);
    assert!(
        project_gewijzigd(&args, &project_nu(), &relaties, &gebruikers())
            .unwrap_err()
            .contains("niets gewijzigd")
    );
    assert!(
        project_gewijzigd(
            &wijzig_project(),
            &json!({"project": null}),
            &relaties,
            &gebruikers()
        )
        .unwrap_err()
        .contains("onbekende vorm")
    );
    assert_eq!(
        gebruiker_ids(Some(&json!([{"id": 3}, 4]))).unwrap(),
        vec![3, 4]
    );
    // An entry that is no id, or no list at all, is refused: it must never take users off the project.
    assert!(
        gebruiker_ids(Some(&json!([{"id": 3}, 4, "x"])))
            .unwrap_err()
            .contains("onbekende vorm")
    );
    assert!(gebruiker_ids(None).is_err());
    let mut vreemd = project_nu();
    vreemd["users"] = json!([{"naam": "Lisa"}]);
    let mut args = wijzig_project();
    args.naam = Some("Nieuw".into());
    assert!(
        project_gewijzigd(&args, &vreemd, &relaties, &gebruikers())
            .unwrap_err()
            .contains("onbekende vorm")
    );
}

// ---------------------------------------------------------------- activiteiten

fn verkoop() -> Verkoop {
    Verkoop::from_api(
        &json!([{"id": 800, "code": "8000", "omschrijving": "Omzet"}, {"id": 801, "code": "8100", "omschrijving": "Omzet advies"}]),
        &json!([
            {"code": "HOOG_VERK_21", "omschrijving": "Btw hoog 21%", "begindatum": null, "einddatum": null},
            {"code": "LAAG_VERK_9", "omschrijving": "Btw laag 9%", "begindatum": {"day": 1, "month": 1, "year": 2019}},
            {"code": "LAAG_VERK_6", "omschrijving": "Btw laag 6%", "einddatum": {"day": 1, "month": 1, "year": 2019}},
            {"code": "NIEUW_VERK", "omschrijving": "Nog niet", "begindatum": {"day": 1, "month": 1, "year": 2099}},
            {"code": "AFW_VERK", "omschrijving": "Afwijkend"}
        ]),
        &json!({"key": "administratie.niet-btw-plichtig", "value": "false"}),
        (2026, 11, 12),
    )
    .unwrap()
}

fn activiteit_args() -> ActiviteitToevoegenArgs {
    ActiviteitToevoegenArgs {
        naam: "Advies".into(),
        omschrijving: None,
        uurtarief: Some(95.0),
        kilometertarief: None,
        facturabel: Some(true),
        factuuromschrijving: None,
        btw_code: Some("hoog_verk_21".into()),
        tegenrekening: None,
        actief: None,
        account: None,
    }
}

#[test]
fn offers_only_the_sales_codes_valid_today_as_the_form_does() {
    let v = verkoop();
    let codes: Vec<&str> = v.btw.iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(codes, ["HOOG_VERK_21", "LAAG_VERK_9"]);
    assert!(v.btwplichtig);
    assert_eq!(v.rekeningen.len(), 2);
    let geen = Verkoop::from_api(
        &json!(null),
        &json!(null),
        &json!({"value": "True"}),
        (2026, 1, 1),
    )
    .unwrap();
    assert!(!geen.btwplichtig && geen.btw.is_empty() && geen.rekeningen.is_empty());
    // Never set: btw-plichtig, as the form reads it. An answer of another shape is refused, not guessed.
    let nooit = Verkoop::from_api(&json!([]), &json!([]), &json!(null), (2026, 1, 1)).unwrap();
    assert!(nooit.btwplichtig);
    for vreemd in [json!(true), json!({"value": null}), json!("false")] {
        assert!(
            Verkoop::from_api(&json!([]), &json!([]), &vreemd, (2026, 1, 1))
                .unwrap_err()
                .contains("onbekende vorm"),
            "{vreemd}"
        );
    }
    assert_eq!(
        setting_waarde(&json!({"value": "true"}), "x").unwrap(),
        Some("true".to_owned())
    );
}

#[test]
fn a_new_activity_is_sent_as_the_activity_form_sends_it() {
    let built = activiteit_nieuw(&activiteit_args(), &verkoop()).unwrap();
    assert_eq!(
        built.body,
        json!({"naam": "Advies", "uurtarief": 95.0, "facturabel": true, "btwCode": "HOOG_VERK_21",
               "grootboekRekeningId": 800, "actief": true})
    );
    assert_eq!(find(&built, "Tegenrekening"), "8000 Omzet");
    assert_eq!(find(&built, "Btw-code"), "HOOG_VERK_21 (Btw hoog 21%)");
    assert_eq!(find(&built, "Uurtarief"), "95,00");

    // Not facturabel: no btw code and no tegenrekening, as on a new one in the form.
    let mut args = activiteit_args();
    args.facturabel = None;
    args.btw_code = None;
    args.omschrijving = Some("Intern".into());
    let built = activiteit_nieuw(&args, &verkoop()).unwrap();
    assert_eq!(
        built.body,
        json!({"naam": "Advies", "omschrijving": "Intern", "uurtarief": 95.0, "facturabel": false, "actief": true})
    );

    // Another omzetrekening, and an administration without btw needs no code.
    let mut args = activiteit_args();
    args.tegenrekening = Some("8100".into());
    args.btw_code = None;
    let mut zonder_btw = verkoop();
    zonder_btw.btwplichtig = false;
    let built = activiteit_nieuw(&args, &zonder_btw).unwrap();
    assert_eq!(built.body["grootboekRekeningId"], 801);
    assert!(built.body.get("btwCode").is_none());
}

#[test]
fn refuses_an_activity_the_form_would_refuse() {
    let refused = |change: fn(&mut ActiviteitToevoegenArgs)| {
        let mut args = activiteit_args();
        change(&mut args);
        activiteit_nieuw(&args, &verkoop()).unwrap_err()
    };
    assert!(refused(|a| a.btw_code = None).contains("Btw-code is niet ingevuld"));
    assert!(refused(|a| a.uurtarief = None).contains("tarief"));
    assert!(refused(|a| a.btw_code = Some("AFW_VERK".into())).contains("Onbekende btw-code"));
    assert!(refused(|a| a.btw_code = Some("LAAG_VERK_6".into())).contains("Onbekende btw-code"));
    assert!(refused(|a| a.tegenrekening = Some("1300".into())).contains("omzetrekening"));
    assert!(refused(|a| a.facturabel = Some(false)).contains("facturabele activiteit"));
    assert!(refused(|a| a.naam = String::new()).contains("niet ingevuld"));
    assert!(refused(|a| a.factuuromschrijving = Some("x".repeat(51))).contains("50 tekens"));
    // Without 8000 in the books a facturabele activity names its omzetrekening.
    let mut v = verkoop();
    v.rekeningen.retain(|(_, code, _)| code != "8000");
    assert!(
        activiteit_nieuw(&activiteit_args(), &v)
            .unwrap_err()
            .contains("Tegenrekening is niet ingevuld")
    );
}

fn activiteit_nu() -> Value {
    json!({"id": 20, "naam": "Ontwerp", "omschrijving": null, "uurtarief": 80, "kilometertarief": null, "facturabel": true,
           "factuurOmschrijving": "Ontwerpwerk", "btwCode": "HOOG_VERK_21", "grootboekRekeningId": 800, "actief": true})
}

fn wijzig_activiteit() -> ActiviteitWijzigenArgs {
    ActiviteitWijzigenArgs {
        id: 20,
        naam: None,
        omschrijving: None,
        uurtarief: None,
        kilometertarief: None,
        facturabel: None,
        factuuromschrijving: None,
        btw_code: None,
        tegenrekening: None,
        actief: None,
        account: None,
    }
}

#[test]
fn a_changed_activity_keeps_its_kostenplaats_and_other_fields() {
    let mut args = wijzig_activiteit();
    args.uurtarief = Some(85.0);
    let built = activiteit_gewijzigd(&args, &activiteit_nu(), Some(31), &verkoop()).unwrap();
    assert_eq!(
        built.body,
        json!({"naam": "Ontwerp", "omschrijving": null, "uurtarief": 85.0, "kilometertarief": null, "facturabel": true,
               "factuurOmschrijving": "Ontwerpwerk", "btwCode": "HOOG_VERK_21", "grootboekRekeningId": 800, "actief": true,
               "kostenplaatsId": 31, "id": 20})
    );
    assert_eq!(find(&built, "Uurtarief was"), "80,00");
    assert_eq!(find(&built, "Uurtarief wordt"), "85,00");

    let mut args = wijzig_activiteit();
    args.tegenrekening = Some("8100".into());
    args.btw_code = Some("LAAG_VERK_9".into());
    let built = activiteit_gewijzigd(&args, &activiteit_nu(), None, &verkoop()).unwrap();
    assert!(built.body.get("kostenplaatsId").is_none());
    assert_eq!(find(&built, "Tegenrekening was"), "8000 Omzet");
    assert_eq!(find(&built, "Tegenrekening wordt"), "8100 Omzet advies");
    assert_eq!(find(&built, "Btw-code wordt"), "LAAG_VERK_9 (Btw laag 9%)");
}

#[test]
fn refuses_a_change_that_leaves_a_facturabele_activity_incomplete() {
    let mut niet = activiteit_nu();
    niet["facturabel"] = json!(false);
    niet["uurtarief"] = json!(null);
    let mut args = wijzig_activiteit();
    args.facturabel = Some(true);
    assert!(
        activiteit_gewijzigd(&args, &niet, None, &verkoop())
            .unwrap_err()
            .contains("tarief")
    );
    assert!(
        activiteit_gewijzigd(&wijzig_activiteit(), &activiteit_nu(), None, &verkoop())
            .unwrap_err()
            .contains("niets gewijzigd")
    );
    let mut args = wijzig_activiteit();
    args.btw_code = Some("HOOG_INK_21".into());
    assert!(
        activiteit_gewijzigd(&args, &activiteit_nu(), None, &verkoop())
            .unwrap_err()
            .contains("Onbekende btw-code")
    );
}

// ---------------------------------------------------------------- factuurinstellingen

fn instellingen_nu() -> Value {
    json!({"id": 1, "uurEenheidId": 61, "kmEenheidId": 62, "factuurSjabloonId": 9, "regelTekst": "[PROJECT] [ACTIVITEIT]",
           "betalingsTermijn": 14, "perPost": false, "perEmail": false, "incasseren": false, "verwerken": true})
}

fn eenheden() -> Vec<(u64, String)> {
    vec![(61, "uren".into()), (62, "kilometers".into())]
}

fn sjablonen() -> Vec<(u64, String)> {
    vec![(9, "Standaard".into()), (10, "Engels".into())]
}

fn wijzig_instellingen() -> FactuurinstellingenWijzigenArgs {
    FactuurinstellingenWijzigenArgs {
        uur_eenheid_id: None,
        km_eenheid_id: None,
        factuursjabloon_id: None,
        regeltekst: None,
        betalingstermijn: None,
        per_post: None,
        per_email: None,
        incasseren: None,
        verwerken: None,
        account: None,
    }
}

#[test]
fn changed_settings_go_back_whole_with_old_and_new_shown() {
    let mut args = wijzig_instellingen();
    args.per_email = Some(true);
    args.factuursjabloon_id = Some(10);
    let built =
        instellingen_gewijzigd(&args, &instellingen_nu(), &eenheden(), &sjablonen()).unwrap();
    let mut want = instellingen_nu();
    want["perEmail"] = json!(true);
    want["factuurSjabloonId"] = json!(10);
    assert_eq!(built.body, want);
    assert_eq!(find(&built, "Factuursjabloon was"), "Standaard");
    assert_eq!(find(&built, "Factuursjabloon wordt"), "Engels");
    assert_eq!(
        find(&built, "Klaarzetten voor verzending per e-mail was"),
        "nee"
    );
    assert_eq!(
        find(&built, "Klaarzetten voor verzending per e-mail wordt"),
        "ja"
    );
    assert!(find(&built, "Let op").contains("factuur_toevoegen"));
    assert!(!built.lines.iter().any(|l| l.label.starts_with("Verwerken")));
}

#[test]
fn refuses_settings_e_boekhouden_would_not_take() {
    let mut args = wijzig_instellingen();
    args.uur_eenheid_id = Some(99);
    assert!(
        instellingen_gewijzigd(&args, &instellingen_nu(), &eenheden(), &sjablonen())
            .unwrap_err()
            .contains("Geen eenheid met id 99")
    );
    let mut args = wijzig_instellingen();
    args.betalingstermijn = Some(100_000);
    assert!(
        instellingen_gewijzigd(&args, &instellingen_nu(), &eenheden(), &sjablonen())
            .unwrap_err()
            .contains("Betalingstermijn")
    );
    let mut args = wijzig_instellingen();
    args.regeltekst = Some(" ".into());
    assert!(
        instellingen_gewijzigd(&args, &instellingen_nu(), &eenheden(), &sjablonen())
            .unwrap_err()
            .contains("niet ingevuld")
    );
    assert!(
        instellingen_gewijzigd(
            &wijzig_instellingen(),
            &instellingen_nu(),
            &eenheden(),
            &sjablonen()
        )
        .unwrap_err()
        .contains("niets gewijzigd")
    );
    // Settings that were never made need their required fields; then they go out without an id.
    let mut args = wijzig_instellingen();
    args.per_post = Some(true);
    assert!(
        instellingen_gewijzigd(&args, &json!(null), &eenheden(), &sjablonen())
            .unwrap_err()
            .contains("eenheid voor uren")
    );
    args.uur_eenheid_id = Some(61);
    args.km_eenheid_id = Some(62);
    args.regeltekst = Some("[PROJECT]".into());
    let built = instellingen_gewijzigd(&args, &json!(null), &eenheden(), &sjablonen()).unwrap();
    assert!(built.body.get("id").is_none());
    assert_eq!(built.body["perPost"], true);
    assert_eq!(built.body["regelTekst"], "[PROJECT]");
}

// ---------------------------------------------------------------- invoicing hours

fn rij(id: u64, project_id: Option<u64>, project: &str, activiteit: &str, uren: f64) -> Value {
    json!({"id": id, "projectId": project_id, "project": project, "activiteit": activiteit, "aantalUren": uren,
           "uurtarief": 80, "aantalKilometers": 10, "kilometertarief": 0.23})
}

fn projecten_rel() -> HashMap<u64, ProjectRelatie> {
    project_relaties(&json!([
        {"id": 10, "naam": "Website", "relatieId": 5, "relatieBedrijf": "Bakkerij Jansen"},
        {"id": 11, "naam": "Onderhoud", "relatieId": 5, "relatieBedrijf": "Bakkerij Jansen"},
        {"id": 12, "naam": "Logo", "relatieId": 6, "relatieBedrijf": "Slagerij Smit"},
        {"id": 13, "naam": "Zonder relatie"}
    ]))
}

#[test]
fn groups_hours_per_relatie_the_way_the_screen_invoices_them() {
    let rows = vec![
        rij(1, Some(10), "Website", "Ontwerp", 2.0),
        rij(2, Some(10), "Website", "Ontwerp", 1.5),
        rij(3, Some(11), "Onderhoud", "Ontwerp", 1.0),
        rij(4, Some(12), "Logo", "Advies", 3.0),
        rij(5, None, "", "Advies", 1.0),
        rij(6, Some(10), "Website", "", 1.0),
    ];
    let (groepen, los) = per_relatie(&rows, &projecten_rel());
    assert_eq!(los, vec![5, 6]);
    assert_eq!(groepen.len(), 2);
    let jansen = &groepen[0];
    assert_eq!(jansen.relatie, "Bakkerij Jansen");
    assert_eq!(jansen.uur_ids, vec![1, 2, 3]);
    assert_eq!(jansen.factuurregels, 2);
    assert!((jansen.uren - 4.5).abs() < 1e-9);
    assert!((jansen.bedrag - (4.5 * 80.0 + 30.0 * 0.23)).abs() < 1e-9);
    assert_eq!(groepen[1].relatie, "Slagerij Smit");
    let shown = groepen_json(&groepen);
    assert_eq!(shown[0]["bedragVolgensTarief"], 366.9);
    assert_eq!(shown[1]["uurIds"], json!([4]));
}

#[test]
fn invoicing_hours_shows_per_relatie_what_is_invoiced() {
    let rows = vec![
        rij(1, Some(10), "Website", "Ontwerp", 2.0),
        rij(4, Some(12), "Logo", "Advies", 3.0),
    ];
    let built = uren_factureren(&rows, &projecten_rel(), &instellingen_nu(), "Standaard").unwrap();
    assert_eq!(built.body, json!([1, 4]));
    assert_eq!(built.title, "Uren factureren: 2 facturen");
    assert_eq!(
        find(&built, "Facturen"),
        "2 (één per relatie), met 2 urenregels"
    );
    assert_eq!(
        find(&built, "Aan Bakkerij Jansen"),
        "2 uur en 10 km op 1 factuurregel, 162,30 volgens tarief"
    );
    assert_eq!(
        find(&built, "Aan Slagerij Smit"),
        "3 uur en 10 km op 1 factuurregel, 242,30 volgens tarief"
    );
    assert_eq!(find(&built, "Totaal volgens tarief"), "404,60");
    assert_eq!(find(&built, "Factuursjabloon"), "Standaard");
    assert_eq!(find(&built, "Betalingstermijn"), "14 dagen");
    assert_eq!(find(&built, "Verwerken in de boekhouding"), "ja");
    assert_eq!(
        find(&built, "Klaarzetten voor verzending per e-mail"),
        "nee"
    );
    assert!(find(&built, "Gevolg").contains("niet terug te draaien"));

    let one = uren_factureren(&rows[..1], &projecten_rel(), &instellingen_nu(), "").unwrap();
    assert_eq!(one.title, "Uren factureren: 1 factuur aan Bakkerij Jansen");
    assert_eq!(find(&one, "Factuursjabloon"), "onbekend");
}

#[test]
fn refuses_invoicing_nothing_lines_without_a_relatie_or_too_many_lines() {
    let refused = |rows: Vec<Value>| {
        uren_factureren(&rows, &projecten_rel(), &instellingen_nu(), "Standaard").unwrap_err()
    };
    assert!(refused(vec![]).contains("geen nog niet gefactureerde uren"));
    assert!(refused(vec![rij(5, None, "", "Advies", 1.0)]).contains(": 5"));
    assert!(refused(vec![rij(7, Some(13), "Zonder relatie", "Advies", 1.0)]).contains(": 7"));
    let many: Vec<Value> = (0..=MAX_FACTUURREGELS as u64)
        .map(|i| rij(i + 1, Some(10), "Website", &format!("Activiteit {i}"), 1.0))
        .collect();
    assert!(refused(many).contains("maximaal 100 factuurregels"));
}

#[test]
fn picks_the_named_hour_lines_and_refuses_one_that_is_not_there() {
    let rows = vec![
        rij(1, Some(10), "Website", "Ontwerp", 2.0),
        rij(2, Some(10), "Website", "Ontwerp", 1.0),
    ];
    assert_eq!(kies_rijen(rows.clone(), None).unwrap().len(), 2);
    let picked = kies_rijen(rows.clone(), Some(&[2])).unwrap();
    assert_eq!(picked, vec![rows[1].clone()]);
    let err = kies_rijen(rows.clone(), Some(&[2, 9])).unwrap_err();
    assert!(err.ends_with(": 9"), "{err}");
    assert!(kies_rijen(rows, Some(&[])).is_err());
}

#[test]
fn puts_hours_on_the_invoice_its_number_names() {
    let gevonden = json!([
        {"factuurId": 554, "factuurNummer": "F000120", "relatieBedrijf": "Ander", "factuurDatum": "2026-10-01T00:00:00"},
        {"factuurId": 555, "factuurNummer": "F00012", "relatieBedrijf": "Bakkerij Jansen", "factuurDatum": "2026-11-01T00:00:00"}
    ]);
    let factuur = factuur_bij_nummer(&gevonden, " f00012 ").unwrap();
    assert_eq!(factuur["factuurId"], 555);
    assert!(
        factuur_bij_nummer(&gevonden, "F0001")
            .unwrap_err()
            .contains("Geen bestaande factuur")
    );
    let lijst = vec![rij(7, Some(10), "Website", "Ontwerp", 2.0)];
    let built = uren_bij_factuur(
        &factuur,
        5,
        &[(7, uur_nu())],
        &lijst,
        &projecten_rel(),
        &namen(),
    )
    .unwrap();
    assert_eq!(built.body, json!({"uren": [7], "factuurId": 555}));
    assert_eq!(built.title, "Uren op factuur F00012 zetten");
    assert_eq!(find(&built, "Relatie"), "Bakkerij Jansen");
    assert_eq!(find(&built, "Relatie van de uren"), "Bakkerij Jansen");
    assert_eq!(find(&built, "Factuurdatum"), "01-11-2026");
    assert_eq!(
        find(&built, "Urenregel 7"),
        "10-11-2026, Website, Ontwerp: 2 uur, 0 km"
    );
    assert!(uren_bij_factuur(&factuur, 5, &[], &lijst, &projecten_rel(), &namen()).is_err());
    // An hour that is not on the list of hours not invoiced yet: already invoiced, or unknown.
    let err = uren_bij_factuur(
        &factuur,
        5,
        &[(7, uur_nu())],
        &[],
        &projecten_rel(),
        &namen(),
    )
    .unwrap_err();
    assert!(
        err.contains("al gefactureerd of onbekend") && err.ends_with(": 7"),
        "{err}"
    );
    // An hour of another relatie than the invoice's.
    let err = uren_bij_factuur(
        &factuur,
        6,
        &[(7, uur_nu())],
        &lijst,
        &projecten_rel(),
        &namen(),
    )
    .unwrap_err();
    assert!(
        err.contains("Bakkerij Jansen") && err.contains("F00012"),
        "{err}"
    );
    // An hour without a project with a relatie.
    let mut zonder = uur_nu();
    zonder["projectId"] = json!(13);
    assert!(
        uren_bij_factuur(
            &factuur,
            5,
            &[(7, zonder)],
            &lijst,
            &projecten_rel(),
            &namen()
        )
        .unwrap_err()
        .contains("geen project met een relatie")
    );
}

#[test]
fn invoicing_refuses_rows_without_an_id_and_rows_changed_after_the_approval() {
    let mut zonder_id = rij(1, Some(10), "Website", "Ontwerp", 2.0);
    zonder_id.as_object_mut().unwrap().remove("id");
    let err = uren_factureren(
        &[rij(2, Some(10), "Website", "Ontwerp", 1.0), zonder_id],
        &projecten_rel(),
        &instellingen_nu(),
        "Standaard",
    )
    .unwrap_err();
    assert!(err.contains("zonder id"), "{err}");

    let goedgekeurd = vec![
        rij(1, Some(10), "Website", "Ontwerp", 2.0),
        rij(2, Some(12), "Logo", "Advies", 1.0),
    ];
    let mut nu = goedgekeurd.clone();
    nu.push(rij(3, Some(10), "Website", "Ontwerp", 5.0));
    assert!(
        nog_zoals_goedgekeurd(&goedgekeurd, &nu).is_ok(),
        "a new row does not matter"
    );
    let mut ander_tarief = goedgekeurd.clone();
    ander_tarief[1]["uurtarief"] = json!(90);
    let mut weg = goedgekeurd.clone();
    weg.remove(0);
    let mut meer_uren = goedgekeurd.clone();
    meer_uren[0]["aantalUren"] = json!(3);
    for nu in [ander_tarief, weg, meer_uren] {
        assert!(
            nog_zoals_goedgekeurd(&goedgekeurd, &nu)
                .unwrap_err()
                .contains("intussen gewijzigd")
        );
    }
}

// ---------------------------------------------------------------- end to end

/// What the stand-in for e-Boekhouden was asked to change: method, path and body.
type Seen = Arc<Mutex<Vec<(String, String, Value)>>>;

/// The answer of the stand-in to a read, by path; `None` is a 404.
fn read_answer(path: &str) -> Option<Value> {
    Some(match path {
        "uur/7" => uur_nu(),
        "project/10" => project_nu(),
        "project/11" => {
            json!({"project": {"id": 11, "naam": "Onderhoud", "relatieId": 5, "actief": true}, "users": [3]})
        }
        "project/12" => {
            json!({"project": {"id": 12, "naam": "Oud", "relatieId": 5, "actief": false}, "users": []})
        }
        "activiteit/20" => activiteit_nu(),
        "relatie/5" => json!({"id": 5, "bedrijf": "Bakkerij Jansen"}),
        "user/selectlist" => json!([{"id": 3, "naam": "Lisa"}, {"id": 4, "naam": "Piet"}]),
        "grootboekrekening/search" => json!([{"id": 800, "code": "8000", "omschrijving": "Omzet"}]),
        "btwinfo/info/VERK" => json!([{"code": "HOOG_VERK_21", "omschrijving": "Btw hoog 21%"}]),
        "setting/administratie.niet-btw-plichtig" => {
            json!({"key": "administratie.niet-btw-plichtig", "value": "false"})
        }
        "setting/administratie.use-kostenplaatsen" => {
            json!({"key": "administratie.use-kostenplaatsen", "value": "false"})
        }
        "uur/8" => {
            json!({"id": 8, "gebruikerId": 3, "datum": "2026-11-10T00:00:00", "projectId": 10, "activiteitId": 20, "aantalUren": 1})
        }
        "project/13" => {
            json!({"project": {"id": 13, "naam": "Logo", "relatieId": 6, "actief": false}, "users": [3]})
        }
        "relatie/6" => json!({"id": 6, "bedrijf": "Slagerij Smit"}),
        "factuur/555" => json!({"id": 555, "nummer": "F00012", "relatieId": 5}),
        "factuur/556" => json!({"id": 556, "nummer": "F00013", "relatieId": 6}),
        "kostenplaats/GetByActiviteitId/20" => json!(null),
        "factuursetting/exists" => json!(true),
        "factuursetting" => instellingen_nu(),
        "factuursjabloon/9" => json!({"id": 9, "naam": "Standaard"}),
        "factuursjabloon/active" => json!([{"id": 9, "naam": "Standaard"}]),
        "eenheid" => json!([{"id": 61, "meervoud": "uren"}, {"id": 62, "meervoud": "kilometers"}]),
        "uur/factureren/gridtable" => json!({
            "colMetadata": [{"name": "id"}, {"name": "datum"}, {"name": "facturabel"}, {"name": "medewerker"}, {"name": "project"},
                            {"name": "projectId"}, {"name": "activiteit"}, {"name": "aantalUren"}, {"name": "uurtarief"},
                            {"name": "aantalKilometers"}, {"name": "kilometertarief"}, {"name": "factuur"}, {"name": "opmerkingen"}],
            "data": [[7, "2026-11-10T00:00:00", true, "Lisa", "Website", 10, "Ontwerp", 2, 80, null, null, null, "Overleg"]]
        }),
        "project/rel" => {
            json!([{"id": 10, "naam": "Website", "relatieId": 5, "relatieBedrijf": "Bakkerij Jansen"}])
        }
        "factuur/searchexisting" => json!([
            {"factuurId": 555, "factuurNummer": "F00012", "relatieBedrijf": "Bakkerij Jansen", "factuurDatum": "2026-11-01T00:00:00"},
            {"factuurId": 556, "factuurNummer": "F00013", "relatieBedrijf": "Slagerij Smit", "factuurDatum": "2026-11-02T00:00:00"}
        ]),
        _ => return None,
    })
}

/// Answers a scenario puts in place of `read_answer`'s, by path: `None` makes the stand-in fail (500).
type Overrides = Arc<Mutex<HashMap<String, Option<Value>>>>;

/// The invoicing list with these rows: (id, project id, project, aantalUren, uurtarief).
fn factureren_lijst(rows: &[(u64, u64, &str, f64, f64)]) -> Value {
    json!({
        "colMetadata": [{"name": "id"}, {"name": "datum"}, {"name": "facturabel"}, {"name": "medewerker"}, {"name": "project"},
                        {"name": "projectId"}, {"name": "activiteit"}, {"name": "aantalUren"}, {"name": "uurtarief"},
                        {"name": "aantalKilometers"}, {"name": "kilometertarief"}, {"name": "factuur"}, {"name": "opmerkingen"}],
        "data": rows.iter().map(|(id, project_id, project, uren, tarief)| json!(
            [id, "2026-11-10T00:00:00", true, "Lisa", project, project_id, "Ontwerp", uren, tarief, null, null, null, ""]
        )).collect::<Vec<_>>()
    })
}

/// A stand-in for e-Boekhouden's API: it answers the reads above (or a scenario's own), and keeps every change it
/// gets.
async fn eboekhouden(seen: Seen, reads: Arc<Mutex<Vec<String>>>, overrides: Overrides) -> String {
    use axum::http::{Method, StatusCode, Uri};
    use axum::response::IntoResponse;
    let app =
        axum::Router::new().fallback(move |method: Method, uri: Uri, body: axum::body::Bytes| {
            let (seen, reads, overrides) = (seen.clone(), reads.clone(), overrides.clone());
            async move {
                let path = uri.path().trim_start_matches("/v1/api/").to_owned();
                if method == Method::GET {
                    reads
                        .lock()
                        .unwrap()
                        .push(format!("{path}?{}", uri.query().unwrap_or_default()));
                    let own = overrides.lock().unwrap().get(&path).cloned();
                    if let Some(own) = own {
                        return match own {
                            Some(answer) => axum::Json(answer).into_response(),
                            None => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
                        };
                    }
                    return match read_answer(&path) {
                        Some(answer) => axum::Json(answer).into_response(),
                        None => StatusCode::NOT_FOUND.into_response(),
                    };
                }
                let body = serde_json::from_slice(&body).unwrap_or(Value::Null);
                seen.lock()
                    .unwrap()
                    .push((method.to_string(), path.clone(), body));
                let answer = match (method.as_str(), path.as_str()) {
                    ("POST", "uur") => json!({"id": 99}),
                    ("POST", "project") => json!({"project": {"id": 14}}),
                    ("POST", "activiteit") => json!({"id": 22}),
                    _ => json!(true),
                };
                axum::Json(answer).into_response()
            }
        });
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

/// Waits for the background run of an answered proposal to end, and gives its record.
async fn settled(state: &AppState, id: u64) -> approvals::Record {
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

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// The MCP server on a fresh state with one signed-in account, and the stand-in for e-Boekhouden behind it.
/// Each scenario takes its own: EB MCP lets one account make 60 calls a minute.
struct Harness {
    state: Arc<AppState>,
    seen: Seen,
    reads: Arc<Mutex<Vec<String>>>,
    overrides: Overrides,
    account: uuid::Uuid,
    http: reqwest::Client,
    url: String,
    session: Option<String>,
}

fn account_input(mode: WriteMode) -> crate::accounts::AccountInput {
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

impl Harness {
    async fn new(mode: WriteMode) -> Self {
        let seen: Seen = Arc::default();
        let reads: Arc<Mutex<Vec<String>>> = Arc::default();
        let overrides: Overrides = Arc::default();
        let jar = Arc::new(crate::state::testing::FakeJar::default());
        let state = Arc::new(crate::state::testing::state_with(
            jar,
            &eboekhouden(seen.clone(), reads.clone(), overrides.clone()).await,
        ));
        let account = state.accounts.add(account_input(mode)).unwrap();
        state.jar_put("sessie", vec![("auth-token".into(), "jwt".into())]);
        state.add_session(account.id, &account.label, "sessie");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = crate::mcp::router(
            state.clone(),
            Arc::new(std::sync::RwLock::new(TOKEN.to_owned())),
        );
        tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });
        let mut harness = Self {
            state,
            seen,
            reads,
            overrides,
            account: account.id,
            http: reqwest::Client::new(),
            url: format!("http://127.0.0.1:{port}{}", crate::mcp::PATH),
            session: None,
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
    async fn call(&self, name: &str, arguments: Value) -> (bool, String) {
        let response = self
            .post(json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": name, "arguments": arguments}}))
            .await;
        answer_of(&response.text().await.unwrap())
    }

    /// Answers `path` with `answer` from now on; `None` makes it fail.
    fn answer(&self, path: &str, answer: Option<Value>) {
        self.overrides
            .lock()
            .unwrap()
            .insert(path.to_owned(), answer);
    }

    fn read(&self, path: &str) -> bool {
        self.reads
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.starts_with(&format!("{path}?")))
    }

    fn sent(&self) -> Vec<(String, String, Value)> {
        self.seen.lock().unwrap().clone()
    }
}

/// Every write family: what it is called with, what e-Boekhouden's own screen sends for it, and whether it
/// always asks.
type Family = (
    &'static str,
    Value,
    (&'static str, &'static str, Value),
    bool,
);

fn families() -> Vec<Family> {
    vec![
        (
            "uur_toevoegen",
            json!({"datum": "2026-11-11", "project_id": 10, "activiteit_id": 20, "uren": 1.5, "opmerkingen": "Ontwerp homepage", "medewerker_id": 3}),
            (
                "POST",
                "uur",
                json!({"gebruikerId": 3, "aantalUren": 1.5, "datum": {"day": 11, "month": 11, "year": 2026}, "activiteitId": 20, "projectId": 10, "opmerkingen": "Ontwerp homepage"}),
            ),
            false,
        ),
        (
            "uur_wijzigen",
            json!({"id": 7, "uren": 3}),
            (
                "PUT",
                "uur",
                json!({"gebruikerId": 3, "aantalUren": 3.0, "datum": {"day": 10, "month": 11, "year": 2026}, "activiteitId": 20, "projectId": 10, "opmerkingen": "Overleg", "id": 7}),
            ),
            false,
        ),
        (
            "uur_verwijderen",
            json!({"id": 7}),
            ("DELETE", "uur/7", Value::Null),
            true,
        ),
        (
            "project_toevoegen",
            json!({"naam": "Onderhoud", "relatie_id": 5}),
            (
                "POST",
                "project",
                json!({"project": {"relatieId": 5, "naam": "Onderhoud", "facturabel": false, "actief": true, "omschrijving": ""}, "users": [3, 4]}),
            ),
            false,
        ),
        (
            "project_wijzigen",
            json!({"id": 10, "actief": false}),
            (
                "PUT",
                "project",
                json!({"project": {"relatieId": 5, "naam": "Website", "facturabel": true, "actief": false, "omschrijving": "", "id": 10}, "users": [3]}),
            ),
            false,
        ),
        (
            "project_verwijderen",
            json!({"id": 10}),
            ("DELETE", "project/10", Value::Null),
            true,
        ),
        (
            "activiteit_toevoegen",
            json!({"naam": "Advies", "uurtarief": 95, "facturabel": true, "btw_code": "HOOG_VERK_21"}),
            (
                "POST",
                "activiteit",
                json!({"naam": "Advies", "uurtarief": 95.0, "facturabel": true, "btwCode": "HOOG_VERK_21", "grootboekRekeningId": 800, "actief": true}),
            ),
            false,
        ),
        (
            "activiteit_wijzigen",
            json!({"id": 20, "uurtarief": 85}),
            (
                "PUT",
                "activiteit",
                json!({"naam": "Ontwerp", "omschrijving": null, "uurtarief": 85.0, "kilometertarief": null, "facturabel": true,
                       "factuurOmschrijving": "Ontwerpwerk", "btwCode": "HOOG_VERK_21", "grootboekRekeningId": 800, "actief": true, "id": 20}),
            ),
            false,
        ),
        (
            "activiteit_verwijderen",
            json!({"id": 20}),
            ("DELETE", "activiteit/20", Value::Null),
            true,
        ),
        (
            "uren_factureren",
            json!({"van": "2026-11-01", "tot": "2026-11-30"}),
            ("POST", "uur/factureren", json!([7])),
            true,
        ),
        (
            "uren_bij_factuur",
            json!({"factuurnummer": "F00012", "uur_ids": [7]}),
            (
                "POST",
                "uur/bestaand/factureren",
                json!({"uren": [7], "factuurId": 555}),
            ),
            true,
        ),
        (
            "uren_factuurinstellingen_wijzigen",
            json!({"per_email": true}),
            (
                "PUT",
                "factuurSetting",
                json!({"id": 1, "uurEenheidId": 61, "kmEenheidId": 62, "factuurSjabloonId": 9, "regelTekst": "[PROJECT] [ACTIVITEIT]",
                       "betalingsTermijn": 14, "perPost": false, "perEmail": true, "incasseren": false, "verwerken": true}),
            ),
            true,
        ),
    ]
}

#[tokio::test]
async fn every_change_waits_for_the_user_and_goes_out_only_when_approved() {
    use crate::approvals::Status;
    for (tool, arguments, (method, path, body), danger) in families() {
        let h = Harness::new(WriteMode::Ask).await;
        // The user says no: nothing goes out.
        let (is_error, text) = h.call(tool, arguments.clone()).await;
        assert!(!is_error, "{tool}: {text}");
        let id = waiting_id(&text);
        let up = h.state.approvals.list();
        assert_eq!(up.len(), 1, "{tool}: {up:?}");
        assert_eq!(up[0].tool, tool);
        assert_eq!(up[0].danger, danger, "{tool}");
        assert!(!up[0].lines.is_empty(), "{tool}");
        assert!(h.state.approvals.decide(id, false, 1));
        assert_eq!(
            settled(&h.state, id).await.status,
            Status::Refused,
            "{tool}"
        );
        assert!(h.sent().is_empty(), "{tool}: sent after a refusal");

        // The user says yes: exactly the request of e-Boekhouden's own screen goes out, once.
        let (_, text) = h.call(tool, arguments).await;
        let id = waiting_id(&text);
        assert!(h.sent().is_empty(), "{tool}: sent before the answer");
        assert!(h.state.approvals.decide(id, true, 2));
        let record = settled(&h.state, id).await;
        assert_eq!(record.status, Status::Done, "{tool}: {record:?}");
        assert_eq!(
            h.sent(),
            vec![(method.to_owned(), path.to_owned(), body)],
            "{tool}"
        );
        if tool == "uren_factureren" {
            // The invoicing list was asked for this period's hours that are not invoiced yet.
            let reads = h.reads.lock().unwrap().clone();
            assert!(
                reads
                    .iter()
                    .any(|r| r.starts_with("uur/factureren/gridtable?")
                        && r.contains("zoekenIn=FACTUREREN")
                        && r.contains("periodStart=20261101")
                        && r.contains("periodEnd=20261130")),
                "{reads:?}"
            );
        }
    }
}

#[tokio::test]
async fn bad_input_is_refused_before_anything_is_asked_or_sent() {
    for (tool, arguments, why) in [
        (
            "uur_toevoegen",
            json!({"datum": "2026-11-11", "project_id": 10, "activiteit_id": 20}),
            "Aantal is niet ingevuld",
        ),
        (
            "uur_toevoegen",
            json!({"datum": "2026-11-11", "project_id": 12, "activiteit_id": 20, "uren": 1}),
            "niet actief",
        ),
        // Two users and none named: e-Boekhouden would refuse the line, so EB MCP asks which one.
        (
            "uur_toevoegen",
            json!({"datum": "2026-11-11", "project_id": 10, "activiteit_id": 20, "uren": 1}),
            "Geef medewerker_id mee",
        ),
        ("uur_wijzigen", json!({"id": 6, "uren": 1}), "bestaat niet"),
        (
            "activiteit_toevoegen",
            json!({"naam": "Advies", "uurtarief": 95, "facturabel": true}),
            "Btw-code is niet ingevuld",
        ),
        (
            "uren_factureren",
            json!({"uur_ids": [9]}),
            "niet bij de nog niet gefactureerde uren",
        ),
        (
            "uren_bij_factuur",
            json!({"factuurnummer": "F99999", "uur_ids": [7]}),
            "Geen bestaande factuur",
        ),
        (
            "uren_bij_factuur",
            json!({"factuurnummer": "F00012", "uur_ids": [8]}),
            "al gefactureerd of onbekend",
        ),
        (
            "uren_bij_factuur",
            json!({"factuurnummer": "F00013", "uur_ids": [7]}),
            "niet bij de relatie van factuur F00013",
        ),
        (
            "uren_factuurinstellingen_wijzigen",
            json!({"uur_eenheid_id": 1}),
            "Geen eenheid",
        ),
    ] {
        let h = Harness::new(WriteMode::Ask).await;
        let (is_error, text) = h.call(tool, arguments).await;
        assert!(is_error && text.contains(why), "{tool}: {text}");
        assert_eq!(h.state.approvals.count(), 0, "{tool}: nothing was put up");
        assert!(h.sent().is_empty(), "{tool}: nothing went out");
    }
    // A read-only account is refused before anything is read.
    let h = Harness::new(WriteMode::Off).await;
    let (is_error, text) = h.call("uur_verwijderen", json!({"id": 7})).await;
    assert!(is_error && text.contains("alleen-lezen"), "{text}");
    assert!(h.reads.lock().unwrap().is_empty());
}

#[tokio::test]
async fn without_approval_an_ordinary_change_goes_through_but_a_delete_still_asks() {
    let h = Harness::new(WriteMode::Auto).await;
    h.answer("user/selectlist", Some(json!([{"id": 3, "naam": "Lisa"}])));
    let (is_error, text) = h
        .call(
            "uur_toevoegen",
            json!({"datum": "2026-11-11", "project_id": 10, "activiteit_id": 20, "kilometers": 12}),
        )
        .await;
    assert!(!is_error && text.contains("\"id\":99"), "{text}");
    assert_eq!(h.sent().len(), 1);
    assert_eq!(h.sent()[0].2["aantalKilometers"], 12.0);
    // No user was named, and the administration has one: the line goes out with that user, as the form
    // would fill in who is signed in.
    assert_eq!(h.sent()[0].2["gebruikerId"], 3);
    let (_, text) = h.call("uur_verwijderen", json!({"id": 7})).await;
    let id = waiting_id(&text);
    assert!(h.state.approvals.list()[0].danger);
    assert!(h.state.approvals.decide(id, false, 1));
    settled(&h.state, id).await;
    assert_eq!(h.sent().len(), 1);
    // Writing turned off: refused before anything is asked.
    h.state
        .accounts
        .update(h.account, account_input(WriteMode::Off))
        .unwrap();
    let (is_error, _) = h.call("uur_verwijderen", json!({"id": 7})).await;
    assert!(is_error);
    assert_eq!(h.sent().len(), 1);
}

#[tokio::test]
async fn the_invoicing_list_shows_per_relatie_what_would_be_invoiced() {
    let h = Harness::new(WriteMode::Ask).await;
    let (is_error, text) = h
        .call(
            "uren_te_factureren",
            json!({"van": "2026-11-01", "tot": "2026-11-30"}),
        )
        .await;
    assert!(!is_error, "{text}");
    let answer: Value = serde_json::from_str(&text).unwrap();
    let groep = &answer["data"]["per_relatie"][0];
    assert_eq!(groep["relatie"], "Bakkerij Jansen");
    assert_eq!(groep["uurIds"], json!([7]));
    assert_eq!(groep["bedragVolgensTarief"], 160.0);
    assert_eq!(answer["data"]["rijen"]["data"][0]["project"], "Website");
    assert_eq!(answer["data"]["zonder_relatie"], json!([]));
    let (is_error, text) = h.call("uur", json!({"id": 7})).await;
    assert!(!is_error && text.contains("Overleg"), "{text}");
    assert!(h.sent().is_empty());
}

#[tokio::test]
async fn invoicing_several_relaties_makes_one_invoice_each_and_finds_inactive_projects() {
    use crate::approvals::Status;
    let h = Harness::new(WriteMode::Ask).await;
    // Project 13 is inactive, so project/rel leaves it out: its relatie comes from the project itself.
    h.answer(
        "uur/factureren/gridtable",
        Some(factureren_lijst(&[
            (7, 10, "Website", 2.0, 80.0),
            (9, 13, "Logo", 1.0, 100.0),
        ])),
    );
    let (is_error, text) = h
        .call(
            "uren_te_factureren",
            json!({"van": "2026-11-01", "tot": "2026-11-30"}),
        )
        .await;
    assert!(!is_error, "{text}");
    let answer: Value = serde_json::from_str(&text).unwrap();
    let groepen = answer["data"]["per_relatie"].as_array().unwrap();
    assert_eq!(groepen.len(), 2, "{groepen:?}");
    assert_eq!(groepen[1]["relatie"], "Slagerij Smit");
    assert_eq!(groepen[1]["bedragVolgensTarief"], 100.0);
    assert!(h.read("project/13") && h.read("relatie/6"));

    let (_, text) = h
        .call(
            "uren_factureren",
            json!({"van": "2026-11-01", "tot": "2026-11-30"}),
        )
        .await;
    let id = waiting_id(&text);
    let proposal = h.state.approvals.list().remove(0);
    assert_eq!(proposal.title, "Uren factureren: 2 facturen");
    let line = |label: &str| {
        proposal
            .lines
            .iter()
            .find(|l| l.label == label)
            .map(|l| l.value.clone())
            .unwrap_or_else(|| panic!("no {label} in {:?}", proposal.lines))
    };
    assert!(line("Aan Bakkerij Jansen").contains("160,00"));
    assert!(line("Aan Slagerij Smit").contains("100,00"));
    assert_eq!(line("Totaal volgens tarief"), "260,00");
    assert!(h.state.approvals.decide(id, true, 1));
    assert_eq!(settled(&h.state, id).await.status, Status::Done);
    assert_eq!(
        h.sent(),
        vec![(
            "POST".to_owned(),
            "uur/factureren".to_owned(),
            json!([7, 9])
        )]
    );
}

#[tokio::test]
async fn hours_changed_while_the_approval_waits_are_not_invoiced() {
    use crate::approvals::Status;
    let h = Harness::new(WriteMode::Ask).await;
    let (_, text) = h
        .call(
            "uren_factureren",
            json!({"van": "2026-11-01", "tot": "2026-11-30"}),
        )
        .await;
    let id = waiting_id(&text);
    // Someone changes the tariff in e-Boekhouden before the user approves.
    h.answer(
        "uur/factureren/gridtable",
        Some(factureren_lijst(&[(7, 10, "Website", 2.0, 95.0)])),
    );
    assert!(h.state.approvals.decide(id, true, 1));
    let record = settled(&h.state, id).await;
    assert_ne!(record.status, Status::Done);
    assert!(
        record
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("intussen gewijzigd"),
        "{record:?}"
    );
    assert!(h.sent().is_empty(), "nothing was invoiced");

    // Already invoiced in the meantime: the line is gone from the list.
    let h = Harness::new(WriteMode::Ask).await;
    let (_, text) = h
        .call(
            "uren_factureren",
            json!({"van": "2026-11-01", "tot": "2026-11-30"}),
        )
        .await;
    let id = waiting_id(&text);
    h.answer("uur/factureren/gridtable", Some(factureren_lijst(&[])));
    assert!(h.state.approvals.decide(id, true, 1));
    assert_ne!(settled(&h.state, id).await.status, Status::Done);
    assert!(h.sent().is_empty());
}

#[tokio::test]
async fn an_activity_keeps_its_kostenplaats_only_where_the_administration_uses_them() {
    let wijzig = json!({"id": 20, "uurtarief": 85});
    // Cost centres on: the kostenplaats the activity has goes back with the change.
    let h = Harness::new(WriteMode::Auto).await;
    h.answer(
        "setting/administratie.use-kostenplaatsen",
        Some(json!({"key": "administratie.use-kostenplaatsen", "value": "true"})),
    );
    h.answer(
        "kostenplaats/GetByActiviteitId/20",
        Some(json!({"id": 31, "omschrijving": "Studio"})),
    );
    let (is_error, text) = h.call("activiteit_wijzigen", wijzig.clone()).await;
    assert!(!is_error, "{text}");
    assert_eq!(h.sent()[0].2["kostenplaatsId"], 31);
    assert!(
        h.reads
            .lock()
            .unwrap()
            .iter()
            .any(|r| r == "setting/administratie.use-kostenplaatsen?acc=false")
    );

    // Cost centres on but the kostenplaats cannot be read: a plain refusal, nothing sent.
    let h = Harness::new(WriteMode::Auto).await;
    h.answer(
        "setting/administratie.use-kostenplaatsen",
        Some(json!({"value": "true"})),
    );
    h.answer("kostenplaats/GetByActiviteitId/20", None);
    let (is_error, text) = h.call("activiteit_wijzigen", wijzig.clone()).await;
    assert!(
        is_error && text.contains("kostenplaats van deze activiteit is niet te lezen"),
        "{text}"
    );
    assert!(h.sent().is_empty());

    // Cost centres off: the kostenplaats is not read, and a failing lookup cannot block the change.
    let h = Harness::new(WriteMode::Auto).await;
    h.answer("kostenplaats/GetByActiviteitId/20", None);
    let (is_error, text) = h.call("activiteit_wijzigen", wijzig.clone()).await;
    assert!(!is_error, "{text}");
    assert!(h.sent()[0].2.get("kostenplaatsId").is_none());
    assert!(!h.read("kostenplaats/GetByActiviteitId/20"));

    // An unreadable btw setting is an error, not "btw-plichtig".
    let h = Harness::new(WriteMode::Auto).await;
    h.answer("setting/administratie.niet-btw-plichtig", Some(json!(42)));
    let (is_error, text) = h.call("activiteit_wijzigen", wijzig).await;
    assert!(is_error && text.contains("onbekende vorm"), "{text}");
    assert!(h.sent().is_empty());
    // Not btw-plichtig: a facturabele activity needs no btw code.
    let h = Harness::new(WriteMode::Auto).await;
    h.answer(
        "setting/administratie.niet-btw-plichtig",
        Some(json!({"value": "true"})),
    );
    let (is_error, text) = h
        .call(
            "activiteit_toevoegen",
            json!({"naam": "Advies", "uurtarief": 95, "facturabel": true}),
        )
        .await;
    assert!(!is_error, "{text}");
    assert!(h.sent()[0].2.get("btwCode").is_none());
}

#[tokio::test]
async fn without_invoice_settings_the_tools_say_so_and_can_make_them() {
    use crate::approvals::Status;
    let h = Harness::new(WriteMode::Ask).await;
    h.answer("factuursetting/exists", Some(json!(false)));
    h.answer("factuursetting", None);
    let (is_error, text) = h.call("uren_factuurinstellingen", json!({})).await;
    assert!(!is_error, "{text}");
    let answer: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(answer["data"]["bestaat"], false);
    assert_eq!(answer["data"]["instellingen"], Value::Null);
    assert!(
        !h.read("factuursetting"),
        "the settings are not read when there are none"
    );

    let (is_error, text) = h
        .call(
            "uren_factuurinstellingen_wijzigen",
            json!({"per_post": true}),
        )
        .await;
    assert!(is_error && text.contains("niet ingevuld"), "{text}");
    let (_, text) = h
        .call(
            "uren_factuurinstellingen_wijzigen",
            json!({"uur_eenheid_id": 61, "km_eenheid_id": 62, "regeltekst": "[PROJECT]", "factuursjabloon_id": 9}),
        )
        .await;
    let id = waiting_id(&text);
    assert!(h.state.approvals.decide(id, true, 1));
    assert_eq!(settled(&h.state, id).await.status, Status::Done);
    let (method, path, body) = h.sent().remove(0);
    assert_eq!((method.as_str(), path.as_str()), ("PUT", "factuurSetting"));
    assert!(body.get("id").is_none(), "{body}");
    assert_eq!(body["regelTekst"], "[PROJECT]");

    let (is_error, text) = h.call("uren_factureren", json!({})).await;
    assert!(
        is_error && text.contains("nog geen factuurinstellingen"),
        "{text}"
    );
    assert!(!h.read("factuursetting"));
}
