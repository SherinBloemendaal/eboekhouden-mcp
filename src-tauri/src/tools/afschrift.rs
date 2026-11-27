//! Afschrift inlezen: "Digitaal afschrift uploaden" of e-Boekhouden (Boekhouden > Importeren), for a statement
//! that is a csv. The lines land with the unprocessed bank lines (`bankregels`), where the import rules and
//! `mutatie_toevoegen` with `bankregel_id` take over.
//!
//! The file never goes to e-Boekhouden as it is. EB MCP reads it itself (statement.rs), the user checks it line
//! by line on Akkoord and decides what goes along, and EB MCP then uploads a csv of its own with only those
//! lines, in one fixed shape. e-Boekhouden's own screen only knows "Starten vanaf regel", which is why leaving
//! single lines out happens here.
//!
//! The requests are the ones e-Boekhouden's web app makes, taken from a real administration on 2026-11-27:
//! - `GET grootboekrekening/bankgiro`: the accounts the upload screen offers.
//! - `POST importfile/uploadfile` `{fileData (base64), fileName, fileSize, fileType}` answers
//!   `{fileId, fileFormat, customFileId}`; a csv with semicolons is `CSV_SEMIL`.
//! - `GET importfile/csv/response?customImportId=<customFileId>` answers `{request, response}`: the request to
//!   send on, and the columns e-Boekhouden knows (`requiredTargetColumns`: Datum, Omschrijving, Bedrag).
//! - The hub `hub/ImportFile`, target `import`, with that request without the file's bytes, plus
//!   `startFromRow`, `customImportFileId`, `importFileRequest {importFileId, ontdubbelen, selectedGb}` and
//!   `mappedColumns`. It ends with `done {dubbel, mogelijkDubbel, aangemaakt, createdImports}`.
//! - `POST import/verwerk` `{importIds}` runs the import rules over the new lines and answers
//!   `{countImport, countMutatie, countErrors}`.

use super::*;
use crate::statement::{By, Chosen, Ledger, Mapping, SkipArg, Statement};

/// The tools of this part that only read.
#[cfg(test)]
pub(super) const READS: &[&str] = &[];
/// The tools of this part that change the books.
#[cfg(test)]
pub(super) const WRITES: &[&str] = &["afschrift_inlezen"];
/// The ones in `WRITES` that cannot be undone.
#[cfg(test)]
pub(super) const DESTRUCTIVE: &[&str] = &["afschrift_inlezen"];

/// The columns of the csv EB MCP uploads, which are the names e-Boekhouden asks for.
const COLUMNS: [&str; 3] = ["Datum", "Omschrijving", "Bedrag"];

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct KolommenArg {
    /// Naam van de kolom met de datum, zoals in de kopregel van het bestand.
    pub datum: Option<String>,
    /// Naam van de kolom met de omschrijving.
    pub omschrijving: Option<String>,
    /// Naam van de kolom met het bedrag in euro (negatief is af).
    pub bedrag: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AfschriftInlezenArgs {
    /// Volledig pad van het csv-bestand op deze computer (komma, puntkomma of tab; met of zonder kopregel).
    pub pad: String,
    /// Code van de grootboekrekening waarop het afschrift wordt ingelezen, bijvoorbeeld "1010" of "1820". Alleen een rekening die e-Boekhouden bij Digitaal afschrift uploaden aanbiedt (bank, giro, creditcard).
    pub rekening: String,
    /// Regels die volgens jou niet mee hoeven, als voorstel: de gebruiker ziet elke regel in EB MCP, zet een regel aan of uit en beslist zelf. Elke regel let op één ding.
    pub overslaan: Option<Vec<SkipArg>>,
    /// Welke kolommen de datum, de omschrijving en het bedrag zijn. Laat weg wat EB MCP zelf goed herkent; noem bijvoorbeeld bedrag als het bestand er twee heeft.
    pub kolommen: Option<KolommenArg>,
    /// De eerste regel van het bestand die telt (standaard de regel na de kopregel).
    pub start_regel: Option<usize>,
    /// Laat e-Boekhouden overslaan wat er al staat (standaard true).
    pub ontdubbelen: Option<bool>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// The bytes of the statement on this computer and the name it has there.
fn read_statement(path: &str) -> Result<(String, Vec<u8>), String> {
    let path = std::path::Path::new(path.trim());
    if !path.is_absolute() {
        return Err("Geef het volledige pad van het bestand op deze computer".into());
    }
    let meta = std::fs::metadata(path)
        .map_err(|_| format!("Geen bestand gevonden op {}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is geen bestand", path.display()));
    }
    if meta.len() == 0 {
        return Err("Het bestand is leeg".into());
    }
    if meta.len() > crate::statement::MAX_BYTES {
        return Err("Het bestand is groter dan 5 MB; splits het in delen".into());
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or("Het bestand heeft geen naam")?;
    let bytes = std::fs::read(path).map_err(|_| format!("{} is niet te lezen", path.display()))?;
    if bytes.starts_with(b"%PDF") || bytes.starts_with(b"PK") || bytes.contains(&0) {
        return Err(format!(
            "{name} is geen csv: EB MCP leest een afschrift als tekst met een regel per boeking. Exporteer het bij de bank als csv."
        ));
    }
    Ok((name, bytes))
}

/// The accounts e-Boekhouden offers for a statement (`grootboekrekening/bankgiro`), without the inactive ones.
fn ledgers_of(answer: &Value) -> Vec<Ledger> {
    answer
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter(|row| row.get("inactief").and_then(Value::as_bool) != Some(true))
                .filter_map(|row| {
                    Some(Ledger {
                        id: row.get("id")?.as_u64()?,
                        code: row.get("code")?.as_str()?.trim().to_owned(),
                        name: row
                            .get("omschrijving")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .trim()
                            .to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The statement as the tool's arguments ask for it: the file read, the columns and start line taken over,
/// the client's rules put up as proposals, and the account chosen from what e-Boekhouden offers.
fn statement_of(
    args: &AfschriftInlezenArgs,
    name: &str,
    bytes: &[u8],
    ledgers: Vec<Ledger>,
) -> Result<Statement, String> {
    let mut statement = Statement::new(name, bytes)?;
    let mut mapping: Mapping = statement.mapping.clone();
    if let Some(kolommen) = &args.kolommen {
        if let Some(name) = &kolommen.datum {
            mapping.date = statement.column(name)?;
        }
        if let Some(name) = &kolommen.omschrijving {
            mapping.description = statement.column(name)?;
        }
        if let Some(name) = &kolommen.bedrag {
            mapping.amount = statement.column(name)?;
        }
    }
    if let Some(start) = args.start_regel {
        mapping.start = start;
    }
    statement.check(&mapping)?;
    statement.mapping = mapping;
    for arg in args.overslaan.iter().flatten() {
        let rule = statement.rule_of(arg, By::Client)?;
        // The same rule twice says nothing new.
        if !statement.rules.iter().any(|r| r.test == rule.test) {
            statement.rules.push(rule);
        }
    }
    statement.dedupe = args.ontdubbelen.unwrap_or(true);
    let code = args.rekening.trim();
    statement.ledger = ledgers
        .iter()
        .position(|l| l.code.eq_ignore_ascii_case(code))
        .ok_or_else(|| {
            if ledgers.is_empty() {
                "e-Boekhouden biedt in deze administratie geen rekening aan om een afschrift op in te lezen".to_owned()
            } else {
                format!(
                    "Op rekening {code:?} kan e-Boekhouden geen afschrift inlezen. Het biedt aan: {}",
                    ledgers.iter().map(Ledger::label).collect::<Vec<_>>().join(", ")
                )
            }
        })?;
    statement.ledgers = ledgers;
    let totals = statement.totals(&statement.rows());
    if totals.lines == 0 {
        return Err(format!(
            "{name} heeft geen regel met een datum en een bedrag in de gekozen kolommen. Er is niets voorgelegd."
        ));
    }
    Ok(statement)
}

/// The csv EB MCP uploads: the chosen lines in e-Boekhouden's own column names, semicolons between them, the
/// date as dd-mm-jjjj and the amount with a comma and no thousands mark.
fn upload_csv(chosen: &[Chosen]) -> String {
    let mut csv = format!("{}\r\n", COLUMNS.join(";"));
    for row in chosen {
        // One line per row whatever the description holds; quotes are doubled inside a quoted cell.
        let description = row
            .description
            .replace(['\r', '\n'], " ")
            .replace('"', "\"\"");
        let (y, m, d) = row.date;
        let (sign, cents) = (
            if row.cents < 0 { "-" } else { "" },
            row.cents.unsigned_abs(),
        );
        csv.push_str(&format!(
            "{d:02}-{m:02}-{y:04};\"{description}\";{sign}{},{:02}\r\n",
            cents / 100,
            cents % 100
        ));
    }
    csv
}

/// The argument of the hub's `import`: the request e-Boekhouden handed out for this file, without the file's
/// bytes, with where to start, which columns are which, the account and ontdubbelen.
fn import_request(
    handed: &Value,
    file_id: u64,
    custom_id: u64,
    ledger: u64,
    dedupe: bool,
) -> Result<Value, String> {
    let known: Vec<&str> = handed["response"]["requiredTargetColumns"]
        .as_array()
        .map(|c| c.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if let Some(missing) = COLUMNS.iter().find(|c| !known.contains(c)) {
        return Err(format!(
            "e-Boekhouden kent de kolom {missing} niet meer bij het inlezen van een csv. Er is niets ingelezen; EB MCP moet hierop worden aangepast."
        ));
    }
    let mut request = handed
        .get("request")
        .and_then(Value::as_object)
        .cloned()
        .ok_or(
            "e-Boekhouden gaf het bestand in een onbekende vorm terug. Er is niets ingelezen.",
        )?;
    if let Some(file) = request.get_mut("importfile").and_then(Value::as_object_mut) {
        file.remove("data");
        file.remove("size");
    }
    request.insert("startFromRow".into(), json!(2));
    request.insert("customImportFileId".into(), json!(custom_id));
    request.insert(
        "importFileRequest".into(),
        json!({"importFileId": file_id, "ontdubbelen": dedupe, "selectedGb": ledger}),
    );
    request.insert("mappedColumns".into(), json!(COLUMNS));
    Ok(Value::Object(request))
}

/// What the tool answers once the lines are in.
fn outcome(statement: &Statement, sent: usize, done: &Value, rules: Result<&Value, &str>) -> Value {
    let count = |value: &Value, key: &str| value.get(key).and_then(Value::as_u64).unwrap_or(0);
    let created = count(done, "aangemaakt");
    let (double, maybe) = (count(done, "dubbel"), count(done, "mogelijkDubbel"));
    let mut answer = json!({
        "bestand": statement.file_name,
        "rekening": statement.ledgers.get(statement.ledger).map(Ledger::label),
        "regels_gestuurd": sent,
        "ingelezen": created,
        "dubbel_overgeslagen": double,
        "mogelijk_dubbel": maybe,
        "bankregel_ids": done.get("createdImports").cloned().unwrap_or(json!([])),
    });
    let mut notes: Vec<String> = Vec::new();
    match rules {
        Ok(ran) => {
            let booked = count(ran, "countMutatie");
            answer["door_importregels_geboekt"] = json!(booked);
            answer["onverwerkt"] = json!(created.saturating_sub(booked));
            let failed = count(ran, "countErrors");
            if failed > 0 {
                answer["importregels_mislukt"] = json!(failed);
                notes.push(format!(
                    "Bij {failed} {} mislukte een automatische importregel; die staan nog bij de onverwerkte bankregels.",
                    if failed == 1 { "regel" } else { "regels" }
                ));
            }
        }
        Err(why) => {
            answer["onverwerkt"] = json!(created);
            notes.push(format!(
                "De regels zijn ingelezen, maar de automatische importregels zijn er niet overheen gegaan ({why}). Ze staan bij de onverwerkte bankregels."
            ));
        }
    }
    if double > 0 {
        notes.push(format!(
            "e-Boekhouden herkende {double} {} als al aanwezig en sloeg die over.",
            if double == 1 { "regel" } else { "regels" }
        ));
    }
    if maybe > 0 {
        notes.push(format!(
            "e-Boekhouden noemt {maybe} {} mogelijk dubbel: kijk die na bij de onverwerkte bankregels.",
            if maybe == 1 { "regel" } else { "regels" }
        ));
    }
    if created + double != sent as u64 {
        notes.push(format!(
            "Er gingen {sent} regels heen en e-Boekhouden meldt er {created} ingelezen en {double} dubbel: kijk bij bankregels wat er staat."
        ));
    }
    if !notes.is_empty() {
        answer["let_op"] = json!(notes.join(" "));
    }
    answer
}

/// The note the history shows when not every line that went out is a new bank line.
pub(super) fn missed(data: &Value) -> Option<String> {
    data.get("regels_gestuurd")?;
    data.get("let_op")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

#[tool_router(router = afschrift_router, vis = "pub(super)")]
impl EbMcp {
    #[tool(
        description = "Een afschrift van een bank of creditcard inlezen uit een csv-bestand op deze computer, zoals Boekhouden > Importeren > Digitaal afschrift uploaden in e-Boekhouden: de regels komen bij de onverwerkte bankregels (zie bankregels), de automatische importregels lopen eroverheen, en daarna boek je met mutatie_toevoegen en bankregel_id. EB MCP leest het bestand zelf en legt elke regel aan de gebruiker voor in een controlescherm: de gebruiker kiest daar de kolommen, de startregel en welke regels meegaan, niet jij. Met overslaan stel je regels voor die eruit kunnen (al geboekt tot een datum, een tekst in de omschrijving, één kaart, een bedrag, bij of af, of twee regels van dezelfde dag die tegen elkaar wegvallen); de gebruiker zet ze aan of uit. Alleen wat de gebruiker aanvinkt gaat naar e-Boekhouden. De gebruiker kan het voorstel ook terugsturen met een opmerking: je krijgt die dan als antwoord, met hoe het scherm stond, en doet een nieuw voorstel. Het antwoord zegt hoeveel regels zijn ingelezen, hoeveel e-Boekhouden als dubbel oversloeg en hoeveel de importregels meteen boekten. Lees hetzelfde bestand niet opnieuw in als het antwoord zegt dat de uitkomst onbekend is: kijk dan eerst bij bankregels. De gebruiker moet dit altijd goedkeuren in EB MCP. Vraagt de wijziging akkoord, dan komt er status wacht met een voorstel_id terug: de wijziging gaat door zodra de gebruiker in EB MCP goedkeurt, akkoord_status geeft later de uitkomst, en je kunt intussen verder met ander werk. Doe de aanroep dan niet opnieuw. Heeft de gebruiker wachten in de aanroep aangezet, dan blijft de aanroep eerst hooguit een paar minuten open en geeft hij de uitkomst zelf als de gebruiker in die tijd beslist.",
        annotations(
            title = "Afschrift inlezen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn afschrift_inlezen(
        &self,
        Parameters(args): Parameters<AfschriftInlezenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "afschrift_inlezen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Afschrift ingelezen; data.ingelezen is het aantal nieuwe bankregels",
            move |this, run, args| async move {
                let (name, bytes) = read_statement(&args.pad)?;
                let ledgers = ledgers_of(
                    &this
                        .fetch(&run, "grootboekrekening/bankgiro", Vec::new())
                        .await?,
                );
                let proposed = statement_of(&args, &name, &bytes, ledgers)?;
                let built = Built {
                    body: proposed.as_args(),
                    title: proposed.title(),
                    lines: proposed.lines(),
                };
                run.attach_statement(proposed);
                // Always with approval, whatever the account allows for a single booking: this brings in many
                // lines at once, and which ones is the user's choice on the sheet.
                this.approve(&run, TOOL, &built, true).await?;
                // What goes out is the statement as the user left it, not as this tool made it.
                let statement = run.approved_statement().ok_or_else(|| {
                    Halt::Error(
                        "EB MCP is het afschrift van dit voorstel kwijt; er is niets ingelezen."
                            .into(),
                    )
                })?;
                let chosen = statement.chosen();
                if chosen.is_empty() {
                    return Err(Halt::Error(
                        "Er is geen regel aangevinkt; er is niets ingelezen.".into(),
                    ));
                }
                let ledger = statement
                    .ledgers
                    .get(statement.ledger)
                    .map(|l| l.id)
                    .ok_or_else(|| {
                        Halt::Error("De rekening is niet gekozen; er is niets ingelezen.".into())
                    })?;
                this.read_in(&run, &statement, &chosen, ledger).await
            },
        )
        .await
    }

    /// Sends the chosen lines to e-Boekhouden the way its own upload screen does.
    async fn read_in(
        &self,
        run: &Run,
        statement: &Statement,
        chosen: &[Chosen],
        ledger: u64,
    ) -> Result<Value, Halt> {
        use base64::Engine;
        let csv = upload_csv(chosen);
        let stem = statement
            .file_name
            .rsplit_once('.')
            .map_or(statement.file_name.as_str(), |(stem, _)| stem);
        let upload_name = format!("eb-mcp-{stem}.csv");
        let uploaded = self
            .change(
                run,
                "POST",
                "importfile/uploadfile",
                Some(&json!({
                    "fileData": base64::engine::general_purpose::STANDARD.encode(csv.as_bytes()),
                    "fileName": upload_name,
                    "fileSize": csv.len(),
                    "fileType": "text/csv",
                })),
            )
            .await?;
        let nothing_in = "Er is niets ingelezen.";
        let (Some(file_id), Some(custom_id)) = (
            uploaded.get("fileId").and_then(Value::as_u64),
            uploaded
                .get("customFileId")
                .and_then(Value::as_u64)
                .filter(|id| *id > 0),
        ) else {
            return Err(Halt::Error(format!(
                "e-Boekhouden nam het bestand aan maar herkende het niet als csv ({}). {nothing_in}",
                uploaded
                    .get("fileFormat")
                    .and_then(Value::as_str)
                    .unwrap_or("onbekend formaat")
            )));
        };
        let handed = self
            .fetch(
                run,
                "importfile/csv/response",
                vec![("customImportId".to_owned(), custom_id.to_string())],
            )
            .await
            .map_err(|e| Halt::Error(format!("{e} {nothing_in}")))?;
        let request = import_request(&handed, file_id, custom_id, ledger, statement.dedupe)?;
        // From here lines may be in while the answer is not: an error no longer means that nothing happened.
        let job = self
            .state
            .api_hub(
                run.account.id,
                "ImportFile",
                "import",
                &request,
                Some(run.id),
                &run.client,
            )
            .await;
        if !job.ok() {
            return Err(Halt::Error(format!(
                "{} Onbekend of er regels zijn ingelezen: kijk bij bankregels, en lees het bestand niet opnieuw in voordat je dat weet.",
                job.error
                    .unwrap_or_else(|| "e-Boekhouden gaf een fout bij het inlezen.".into())
            )));
        }
        let done = job.json.unwrap_or(Value::Null);
        let created: Vec<Value> = done
            .get("createdImports")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if created.is_empty() {
            return Ok(outcome(statement, chosen.len(), &done, Ok(&json!({}))));
        }
        let ran = self
            .change(
                run,
                "POST",
                "import/verwerk",
                Some(&json!({"importIds": created})),
            )
            .await;
        Ok(match &ran {
            Ok(ran) => outcome(statement, chosen.len(), &done, Ok(ran)),
            Err(Halt::Error(why) | Halt::Refused(why)) => {
                outcome(statement, chosen.len(), &done, Err(why))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARD: &[u8] = b"Kaart,Transactiedatum,Omschrijving,Bedrag,Bedrag in EUR\n\
1111,2026-10-27,KOSTEN TE LATE BETALING,\"-24,00\",\"-24,00\"\n\
1111,2026-10-26,BETALING ONTVANGEN - DANK U,\"2.000,04\",\"2.000,04\"\n\
2222,2026-10-17,\"CURSOR; \"\"AI\"\" IDE\",\"-52,75\",\"-48,26\"\n\
1111,2026-06-07,LARAVEL FORGE,\"-19,00\",\"-17,42\"\n";

    fn args() -> AfschriftInlezenArgs {
        AfschriftInlezenArgs {
            pad: "/tmp/kaart.csv".into(),
            rekening: "1820".into(),
            overslaan: None,
            kolommen: None,
            start_regel: None,
            ontdubbelen: None,
            account: None,
        }
    }

    fn ledgers() -> Vec<Ledger> {
        ledgers_of(&json!([
            {"id": 11, "code": "1010", "omschrijving": "Bank", "inactief": false},
            {"id": 18, "code": "1820", "omschrijving": " Credit card ", "inactief": false},
            {"id": 19, "code": "1830", "omschrijving": "Oud", "inactief": true},
        ]))
    }

    #[test]
    fn builds_the_statement_the_arguments_ask_for() {
        assert_eq!(
            ledgers().iter().map(Ledger::label).collect::<Vec<_>>(),
            ["1010 Bank", "1820 Credit card"]
        );
        let plain = statement_of(&args(), "kaart.csv", CARD, ledgers()).unwrap();
        assert_eq!(plain.table.header[plain.mapping.amount], "Bedrag in EUR");
        assert_eq!(
            (plain.ledger, plain.dedupe, plain.rules.len()),
            (1, true, 0)
        );
        let asked = AfschriftInlezenArgs {
            overslaan: Some(vec![
                SkipArg {
                    datum_tot_en_met: Some("2026-06-07".into()),
                    reden: Some("al geboekt".into()),
                    ..SkipArg::default()
                },
                SkipArg {
                    omschrijving_bevat: Some("BETALING ONTVANGEN".into()),
                    ..SkipArg::default()
                },
                SkipArg {
                    omschrijving_bevat: Some("BETALING ONTVANGEN".into()),
                    ..SkipArg::default()
                },
            ]),
            kolommen: Some(KolommenArg {
                datum: None,
                omschrijving: None,
                bedrag: Some("bedrag".into()),
            }),
            ontdubbelen: Some(false),
            ..args()
        };
        let statement = statement_of(&asked, "kaart.csv", CARD, ledgers()).unwrap();
        assert_eq!(statement.table.header[statement.mapping.amount], "Bedrag");
        assert_eq!(statement.rules.len(), 2, "the same rule twice is one rule");
        assert!(statement.rules.iter().all(|r| r.by == By::Client && r.on));
        assert!(!statement.dedupe);
        assert_eq!(
            statement
                .chosen()
                .iter()
                .map(|c| (c.line, c.cents))
                .collect::<Vec<_>>(),
            [(2, -2400), (4, -5275)]
        );
        // What cannot be: an account e-Boekhouden does not offer, a column that is not there, a file without lines.
        for (wrong, part) in [
            (
                AfschriftInlezenArgs {
                    rekening: "1830".into(),
                    ..args()
                },
                "Het biedt aan: 1010 Bank, 1820 Credit card",
            ),
            (
                AfschriftInlezenArgs {
                    kolommen: Some(KolommenArg {
                        datum: Some("Boekdatum".into()),
                        omschrijving: None,
                        bedrag: None,
                    }),
                    ..args()
                },
                "geen kolom \"Boekdatum\"",
            ),
            (
                AfschriftInlezenArgs {
                    start_regel: Some(40),
                    ..args()
                },
                "tussen 1 en 5",
            ),
            (
                AfschriftInlezenArgs {
                    overslaan: Some(vec![SkipArg::default()]),
                    ..args()
                },
                "zegt niet waarop hij let",
            ),
        ] {
            let error = statement_of(&wrong, "kaart.csv", CARD, ledgers()).unwrap_err();
            assert!(error.contains(part), "{error}");
        }
        assert!(
            statement_of(&args(), "kaart.csv", CARD, Vec::new())
                .unwrap_err()
                .contains("geen rekening aan")
        );
    }

    #[test]
    fn refuses_a_file_that_is_not_there_or_not_text() {
        assert!(
            read_statement("kaart.csv")
                .unwrap_err()
                .contains("volledige pad")
        );
        let dir = std::env::temp_dir().join(format!("ebmcp-afschrift-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(
            read_statement(dir.join("weg.csv").to_str().unwrap())
                .unwrap_err()
                .contains("Geen bestand gevonden")
        );
        assert!(
            read_statement(dir.to_str().unwrap())
                .unwrap_err()
                .contains("is geen bestand")
        );
        for (name, bytes, part) in [
            ("leeg.csv", &b""[..], "is leeg"),
            ("afschrift.pdf", &b"%PDF-1.7"[..], "is geen csv"),
            ("afschrift.xlsx", &b"PK\x03\x04"[..], "is geen csv"),
        ] {
            let path = dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            assert!(
                read_statement(path.to_str().unwrap())
                    .unwrap_err()
                    .contains(part),
                "{name}"
            );
        }
        let path = dir.join("kaart.csv");
        std::fs::write(&path, CARD).unwrap();
        assert_eq!(
            read_statement(path.to_str().unwrap()).unwrap(),
            ("kaart.csv".to_owned(), CARD.to_vec())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn uploads_only_the_chosen_lines_in_e_boekhoudens_own_columns() {
        let statement = statement_of(&args(), "kaart.csv", CARD, ledgers()).unwrap();
        let chosen = statement.chosen();
        assert_eq!(
            upload_csv(&chosen[..3]),
            "Datum;Omschrijving;Bedrag\r\n\
27-10-2026;\"KOSTEN TE LATE BETALING\";-24,00\r\n\
26-10-2026;\"BETALING ONTVANGEN - DANK U\";2000,04\r\n\
17-10-2026;\"CURSOR; \"\"AI\"\" IDE\";-48,26\r\n"
        );
        // What EB MCP uploads reads back as the same lines.
        let back = Statement::new("x.csv", upload_csv(&chosen).as_bytes())
            .unwrap()
            .chosen();
        assert_eq!(
            back.iter()
                .map(|c| (c.date, c.description.as_str(), c.cents))
                .collect::<Vec<_>>(),
            chosen
                .iter()
                .map(|c| (c.date, c.description.as_str(), c.cents))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            upload_csv(&[Chosen {
                line: 1,
                date: (2026, 1, 2),
                description: "a\nb".into(),
                cents: 5
            }]),
            "Datum;Omschrijving;Bedrag\r\n02-01-2026;\"a b\";0,05\r\n"
        );
    }

    #[test]
    fn asks_the_hub_for_the_import_as_the_web_app_does() {
        let handed = json!({
            "request": {"step": 0, "importfile": {"data": "QUJD", "name": "eb-mcp-kaart.csv", "size": 3, "type": "text/csv"}, "fileGuid": "g.csv", "sheet": 1, "startFromRow": 0, "importType": "AFSCHRIFT", "forAcc": false},
            "response": {"requiredTargetColumns": ["Datum", "Omschrijving", "Bedrag"], "optionalTargetColumns": ["Credit/Debit", "Rekeningnummer"]}
        });
        assert_eq!(
            import_request(&handed, 3148475, 1506486, 26852133, true).unwrap(),
            json!({
                "step": 0, "importfile": {"name": "eb-mcp-kaart.csv", "type": "text/csv"}, "fileGuid": "g.csv", "sheet": 1, "startFromRow": 2,
                "importType": "AFSCHRIFT", "forAcc": false, "customImportFileId": 1506486,
                "importFileRequest": {"importFileId": 3148475, "ontdubbelen": true, "selectedGb": 26852133},
                "mappedColumns": ["Datum", "Omschrijving", "Bedrag"]
            })
        );
        let renamed = json!({"request": {}, "response": {"requiredTargetColumns": ["Datum", "Omschrijving", "Saldo"]}});
        assert!(
            import_request(&renamed, 1, 2, 3, true)
                .unwrap_err()
                .contains("kent de kolom Bedrag niet meer")
        );
        assert!(
            import_request(
                &json!({"response": {"requiredTargetColumns": COLUMNS}}),
                1,
                2,
                3,
                true
            )
            .unwrap_err()
            .contains("onbekende vorm")
        );
    }

    #[test]
    fn says_what_came_of_it_in_numbers_and_in_words() {
        let statement = statement_of(&args(), "kaart.csv", CARD, ledgers()).unwrap();
        let whole = outcome(
            &statement,
            3,
            &json!({"dubbel": 0, "mogelijkDubbel": 0, "aangemaakt": 3, "createdImports": [1, 2, 3]}),
            Ok(&json!({"countImport": 3, "countMutatie": 1, "countErrors": 0})),
        );
        assert_eq!(
            whole,
            json!({"bestand": "kaart.csv", "rekening": "1820 Credit card", "regels_gestuurd": 3, "ingelezen": 3, "dubbel_overgeslagen": 0,
                   "mogelijk_dubbel": 0, "bankregel_ids": [1, 2, 3], "door_importregels_geboekt": 1, "onverwerkt": 2})
        );
        assert_eq!(missed(&whole), None);
        let double = outcome(
            &statement,
            3,
            &json!({"dubbel": 2, "mogelijkDubbel": 1, "aangemaakt": 1, "createdImports": [4]}),
            Err("geen verbinding"),
        );
        let note = missed(&double).unwrap();
        assert!(note.contains("herkende 2 regels als al aanwezig"), "{note}");
        assert!(note.contains("1 regel mogelijk dubbel"), "{note}");
        assert!(
            note.contains(
                "de automatische importregels zijn er niet overheen gegaan (geen verbinding)"
            ),
            "{note}"
        );
        assert_eq!(double["onverwerkt"], 1);
        let short = outcome(
            &statement,
            3,
            &json!({"aangemaakt": 2, "createdImports": [5, 6]}),
            Ok(&json!({"countErrors": 1})),
        );
        let note = missed(&short).unwrap();
        assert!(
            note.contains("Er gingen 3 regels heen") && note.contains("Bij 1 regel mislukte"),
            "{note}"
        );
        // Not an answer of this tool: no note.
        assert_eq!(missed(&json!({"let_op": "iets anders"})), None);
    }
}
