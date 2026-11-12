//! Uren: hours, projects, activities, invoicing hours and the hour settings.
//!
//! The tools of this part live here, on their own router (`uren_beheer_router`), which tools.rs adds to the
//! server's. Each tool is named in one of the lists below, which the tool-list test in mcp.rs reads.
//!
//! The paths, query parameters and bodies come from the Uren module of e-Boekhouden's own web app (its Angular
//! bundle). `bouw` turns arguments into request bodies and approval summaries; the tools here read what they
//! need, ask, and send. The list of hours itself is the `uren` tool in tools.rs.

use std::collections::HashMap;

use super::*;

mod bouw;
mod factureren;
#[cfg(test)]
mod tests;

use bouw::*;
use factureren::*;

/// The tools of this part that only read.
#[cfg(test)]
pub(super) const READS: &[&str] = &[
    "uur",
    "uren_per_project",
    "uren_per_activiteit",
    "uren_per_jaar",
    "uren_medewerkers",
    "projecten",
    "project",
    "activiteiten",
    "activiteit",
    "uren_te_factureren",
    "uren_factuurinstellingen",
];
/// The tools of this part that change the books.
#[cfg(test)]
pub(super) const WRITES: &[&str] = &[
    "uur_toevoegen",
    "uur_wijzigen",
    "uur_verwijderen",
    "project_toevoegen",
    "project_wijzigen",
    "project_verwijderen",
    "activiteit_toevoegen",
    "activiteit_wijzigen",
    "activiteit_verwijderen",
    "uren_factureren",
    "uren_bij_factuur",
    "uren_factuurinstellingen_wijzigen",
];
/// The ones in `WRITES` that cannot be undone.
#[cfg(test)]
pub(super) const DESTRUCTIVE: &[&str] = &[
    "uur_verwijderen",
    "project_verwijderen",
    "activiteit_verwijderen",
    "uren_factureren",
    "uren_bij_factuur",
];

/// The most rows `uren_factureren` reads from the invoicing list (10 pages of 200).
const MAX_PAGINAS: u32 = 10;

/// A calendar date: year, month, day.
type Dag = (i32, u32, u32);

/// A period from `van` and `tot`, each defaulting as given; `van` after `tot` is refused.
fn periode(
    van: Option<&str>,
    tot: Option<&str>,
    default_van: Dag,
    default_tot: Dag,
) -> Result<(Dag, Dag), String> {
    let van = van.map(parse_date).transpose()?.unwrap_or(default_van);
    let tot = tot.map(parse_date).transpose()?.unwrap_or(default_tot);
    if van > tot {
        return Err("van ligt na tot".into());
    }
    Ok((van, tot))
}

/// The filters of the invoicing list (`uur/factureren/gridtable`), as its screen sends them; the remarks are
/// always shown, so the AI client sees what each line is about. The default period is this month, as there.
fn factureren_filters(
    van: Option<&str>,
    tot: Option<&str>,
    medewerker_id: Option<u64>,
    project_id: Option<u64>,
    activiteit_id: Option<u64>,
    zoeken_in: ZoekenIn,
    niet_facturabel: bool,
) -> Result<Vec<(String, String)>, String> {
    let now = today();
    let (van, tot) = periode(van, tot, (now.0, now.1, 1), laatste_dag(now.0, now.1))?;
    let mut query = vec![
        ("periodStart".to_owned(), compact(van)),
        ("periodEnd".to_owned(), compact(tot)),
    ];
    push(&mut query, "medewerkerId", medewerker_id);
    push(&mut query, "projectId", project_id);
    push(&mut query, "activiteitId", activiteit_id);
    query.push(("zoekenIn".into(), zoeken_in.code().into()));
    query.push(("nietFacturabel".into(), niet_facturabel.to_string()));
    query.push(("showOpmerkingen".into(), "true".into()));
    Ok(query)
}

/// The rows of a grid answer, named.
fn rijen(grid: Value) -> Vec<Value> {
    match named_rows(grid) {
        Value::Array(rows) => rows,
        other => other
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
    }
}

impl EbMcp {
    /// A grid read with its rows named; bad input is refused before anything goes out.
    async fn uren_grid(
        &self,
        context: &RequestContext<RoleServer>,
        tool: &str,
        args: &impl Serialize,
        account: Option<&str>,
        path: &str,
        query: Result<Vec<(String, String)>, String>,
    ) -> Result<CallToolResult, McpError> {
        let run = match self.begin(context, tool, args, account) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let query = match query {
            Ok(query) => query,
            Err(message) => return Ok(self.refuse(run, message)),
        };
        let result = self.fetch(&run, path, query).await.map(named_rows);
        Ok(self.end(run, path, result))
    }

    /// One record (`GET {path}`); an answer that is no record (e-Boekhouden gives null for an unknown id) is
    /// refused with `what` in it.
    async fn uren_record(&self, run: &Run, path: &str, what: &str) -> Result<Value, String> {
        let value = self.fetch(run, path, Vec::new()).await?;
        if value.is_object() {
            Ok(value)
        } else {
            Err(format!("{what} bestaat niet in deze administratie"))
        }
    }

    /// A project (`GET project/{id}`, `{project, users}`): its `project` part, which must be there.
    async fn uren_project(&self, run: &Run, id: u64) -> Result<Value, String> {
        let wrapper = self
            .uren_record(run, &format!("project/{id}"), &format!("Project {id}"))
            .await?;
        wrapper
            .get("project")
            .filter(|p| p.is_object())
            .cloned()
            .ok_or_else(|| format!("Project {id} bestaat niet in deze administratie"))
    }

    /// The names of the projects and activities of hour lines, and of the medewerkers when `medewerkers`. A
    /// project or activity in `nieuw` must exist and be active: the hours form offers only active ones.
    async fn uren_namen(
        &self,
        run: &Run,
        projecten: &[u64],
        activiteiten: &[u64],
        nieuw: (Option<u64>, Option<u64>),
        medewerkers: bool,
    ) -> Result<Namen, String> {
        let mut namen = Namen::default();
        for id in projecten {
            if namen.projecten.contains_key(id) {
                continue;
            }
            let project = self.uren_project(run, *id).await?;
            if nieuw.0 == Some(*id) && project.get("actief").and_then(Value::as_bool) == Some(false)
            {
                return Err(format!(
                    "Project {id} is niet actief: zet het eerst actief met project_wijzigen"
                ));
            }
            let naam = project
                .get("naam")
                .and_then(Value::as_str)
                .unwrap_or_default();
            namen.projecten.insert(*id, naam.to_owned());
        }
        for id in activiteiten {
            if namen.activiteiten.contains_key(id) {
                continue;
            }
            let activiteit = self
                .uren_record(
                    run,
                    &format!("activiteit/{id}"),
                    &format!("Activiteit {id}"),
                )
                .await?;
            if nieuw.1 == Some(*id)
                && activiteit.get("actief").and_then(Value::as_bool) == Some(false)
            {
                return Err(format!(
                    "Activiteit {id} is niet actief: zet hem eerst actief met activiteit_wijzigen"
                ));
            }
            let naam = activiteit
                .get("naam")
                .and_then(Value::as_str)
                .unwrap_or_default();
            namen.activiteiten.insert(*id, naam.to_owned());
        }
        if medewerkers {
            let list = self.fetch(run, "user/selectlist", Vec::new()).await?;
            namen.medewerkers = id_namen(&list, "naam").into_iter().collect();
        }
        Ok(namen)
    }

    /// The users of the administration (`user/selectlist`): id and name.
    async fn uren_gebruikers(&self, run: &Run) -> Result<Vec<(u64, String)>, String> {
        let list = self.fetch(run, "user/selectlist", Vec::new()).await?;
        Ok(id_namen(&list, "naam"))
    }

    /// What an activity's sales side may point at, read as the activity form reads it.
    async fn uren_verkoop(&self, run: &Run) -> Result<Verkoop, String> {
        let rekeningen = self
            .fetch(
                run,
                "grootboekrekening/search",
                vec![
                    ("rekeningCategorie".into(), "VW,BAL".into()),
                    ("searchTerm".into(), String::new()),
                    ("actief".into(), "true".into()),
                ],
            )
            .await?;
        let btw = self.fetch(run, "btwinfo/info/VERK", Vec::new()).await?;
        let niet_btw_plichtig = self
            .uren_setting(run, "administratie.niet-btw-plichtig")
            .await?;
        Verkoop::from_api(&rekeningen, &btw, &niet_btw_plichtig, today())
    }

    /// A setting of the administration, read as the forms read it (`getSetting(key)`: `GET setting/{key}?acc=false`).
    async fn uren_setting(&self, run: &Run, key: &str) -> Result<Value, String> {
        self.fetch(
            run,
            &format!("setting/{key}"),
            vec![("acc".into(), "false".into())],
        )
        .await
    }

    /// The hour invoice settings: whether they exist (`factuursetting/exists`) and, only when they do, the
    /// settings themselves (`factuursetting`); `Null` when there are none yet.
    async fn uren_instellingen(&self, run: &Run) -> Result<(bool, Value), String> {
        let bestaat = match self.fetch(run, "factuursetting/exists", Vec::new()).await? {
            Value::Bool(bestaat) => bestaat,
            _ => {
                return Err(
                    "e-Boekhouden zei in een onbekende vorm of er factuurinstellingen zijn".into(),
                );
            }
        };
        if !bestaat {
            return Ok((false, Value::Null));
        }
        Ok((true, self.fetch(run, "factuursetting", Vec::new()).await?))
    }

    /// Every row of the invoicing list for these filters, up to `MAX_PAGINAS` pages.
    async fn uren_alle_rijen(
        &self,
        run: &Run,
        filters: &[(String, String)],
    ) -> Result<Vec<Value>, String> {
        let mut all = Vec::new();
        for page in 0..MAX_PAGINAS {
            let mut query = grid(Some(MAX_ROWS), Some(page * MAX_ROWS))?;
            query.extend(filters.iter().cloned());
            let rows = rijen(self.fetch(run, "uur/factureren/gridtable", query).await?);
            let count = rows.len();
            all.extend(rows);
            if count < MAX_ROWS as usize {
                return Ok(all);
            }
        }
        Err(format!(
            "Er zijn meer dan {} urenregels in deze periode; kies een kortere periode of een project",
            MAX_PAGINAS * MAX_ROWS
        ))
    }

    /// The relatie of every project the rows name: from `project/rel` (the list the invoicing screen reads), and
    /// for a project that list leaves out (an inactive one) from the project and its relatie themselves.
    /// `byUser=false`: the screen sends `byUser` true only for a user who is neither accountant nor main user,
    /// which it knows from its login (`isAccountant`, `user.isSuper`), not from an API EB MCP reads. A project
    /// the list would not give for that user is still looked up on its own, so nothing depends on it.
    async fn uren_project_relaties(
        &self,
        run: &Run,
        rows: &[Value],
    ) -> Result<HashMap<u64, ProjectRelatie>, String> {
        let list = self
            .fetch(
                run,
                "project/rel",
                vec![
                    ("byUser".into(), "false".into()),
                    ("active".into(), "true".into()),
                ],
            )
            .await?;
        let mut projecten = project_relaties(&list);
        let mut missing: Vec<u64> = rows
            .iter()
            .filter_map(|r| r.get("projectId").and_then(Value::as_u64))
            .filter(|id| *id != 0 && !projecten.contains_key(id))
            .collect();
        missing.sort_unstable();
        missing.dedup();
        for id in missing {
            let project = self.uren_project(run, id).await?;
            let Some(relatie_id) = project
                .get("relatieId")
                .and_then(Value::as_u64)
                .filter(|r| *r != 0)
            else {
                continue;
            };
            let relatie = self
                .uren_record(
                    run,
                    &format!("relatie/{relatie_id}"),
                    &format!("Relatie {relatie_id}"),
                )
                .await?;
            projecten.insert(
                id,
                ProjectRelatie {
                    relatie_id,
                    relatie: relatie
                        .get("bedrijf")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                },
            );
        }
        Ok(projecten)
    }

    /// The units and the active invoice templates the settings may point at.
    async fn uren_instelling_keuzes(
        &self,
        run: &Run,
    ) -> Result<(Vec<(u64, String)>, Vec<(u64, String)>), String> {
        let eenheden = self.fetch(run, "eenheid", Vec::new()).await?;
        let sjablonen = self
            .fetch(run, "factuursjabloon/active", Vec::new())
            .await?;
        Ok((
            id_namen(&eenheden, "meervoud"),
            id_namen(&sjablonen, "naam"),
        ))
    }
}

#[tool_router(router = uren_beheer_router, vis = "pub(super)")]
impl EbMcp {
    // ------------------------------------------------------------ reading

    #[tool(
        description = "Eén urenregel, op id (uit uren): datum, medewerker (gebruikerId), project, activiteit, aantal uren en kilometers en opmerkingen.",
        annotations(title = "Urenregel", read_only_hint = true)
    )]
    async fn uur(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let path = format!("uur/{}", args.id);
        self.simple(
            &context,
            "uur",
            &args,
            args.account.as_deref(),
            &path,
            Ok(vec![]),
        )
        .await
    }

    #[tool(
        description = "Overzicht van de uren per project over een periode (standaard dit jaar tot vandaag), in aantal uren of in euro's, eventueel alleen voor één activiteit. Het overzicht Uren per project van e-Boekhouden.",
        annotations(title = "Uren per project", read_only_hint = true)
    )]
    async fn uren_per_project(
        &self,
        Parameters(args): Parameters<UrenPerProjectArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let query = (|| {
            let now = today();
            let (van, tot) = periode(args.van.as_deref(), args.tot.as_deref(), (now.0, 1, 1), now)?;
            let mut q = grid(args.limit, args.offset)?;
            q.push(("periodStart".into(), compact(van)));
            q.push(("periodEnd".into(), compact(tot)));
            push(&mut q, "activiteitId", args.activiteit_id);
            q.push((
                "weergave".into(),
                args.weergave.unwrap_or(Weergave::Uren).code().into(),
            ));
            Ok(q)
        })();
        self.uren_grid(
            &context,
            "uren_per_project",
            &args,
            args.account.as_deref(),
            "uur/perproject/gridtable",
            query,
        )
        .await
    }

    #[tool(
        description = "Overzicht van de uren per activiteit over een periode (standaard dit jaar tot vandaag), in aantal uren of in euro's, eventueel alleen voor één project. Het overzicht Uren per activiteit van e-Boekhouden.",
        annotations(title = "Uren per activiteit", read_only_hint = true)
    )]
    async fn uren_per_activiteit(
        &self,
        Parameters(args): Parameters<UrenPerActiviteitArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let query = (|| {
            let now = today();
            let (van, tot) = periode(args.van.as_deref(), args.tot.as_deref(), (now.0, 1, 1), now)?;
            let mut q = grid(args.limit, args.offset)?;
            q.push(("periodStart".into(), compact(van)));
            q.push(("periodEnd".into(), compact(tot)));
            push(&mut q, "projectId", args.project_id);
            q.push((
                "weergave".into(),
                args.weergave.unwrap_or(Weergave::Uren).code().into(),
            ));
            Ok(q)
        })();
        self.uren_grid(
            &context,
            "uren_per_activiteit",
            &args,
            args.account.as_deref(),
            "uur/peractiviteit/gridtable",
            query,
        )
        .await
    }

    #[tool(
        description = "Overzicht van de uren van één kalenderjaar per gebruiker en per maand (jan tot en met dec), in aantal uren of in euro's, eventueel alleen voor één project of één activiteit. Het overzicht Uren per jaar van e-Boekhouden.",
        annotations(title = "Uren per jaar", read_only_hint = true)
    )]
    async fn uren_per_jaar(
        &self,
        Parameters(args): Parameters<UrenPerJaarArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let query = (|| {
            let jaar = args.jaar.unwrap_or(today().0);
            if !(1990..=2100).contains(&jaar) {
                return Err("Ongeldig jaar".to_owned());
            }
            let mut q = grid(Some(MAX_ROWS), None)?;
            q.push(("periodStart".into(), compact((jaar, 1, 1))));
            q.push(("periodEnd".into(), compact((jaar, 12, 31))));
            push(&mut q, "activiteitId", args.activiteit_id);
            push(&mut q, "projectId", args.project_id);
            q.push((
                "weergave".into(),
                args.weergave.unwrap_or(Weergave::Uren).code().into(),
            ));
            Ok(q)
        })();
        self.uren_grid(
            &context,
            "uren_per_jaar",
            &args,
            args.account.as_deref(),
            "uur/perjaar/gridtable",
            query,
        )
        .await
    }

    #[tool(
        description = "De gebruikers (medewerkers) van de administratie met hun id, voor medewerker_id bij uren en gebruiker_ids bij projecten.",
        annotations(title = "Medewerkers", read_only_hint = true)
    )]
    async fn uren_medewerkers(
        &self,
        Parameters(args): Parameters<AccountArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        self.simple(
            &context,
            "uren_medewerkers",
            &args,
            args.account.as_deref(),
            "user/selectlist",
            Ok(vec![]),
        )
        .await
    }

    #[tool(
        description = "Projecten voor de urenregistratie zoeken of opsommen, met hun relatie. Te filteren op naam en relatie.",
        annotations(title = "Projecten", read_only_hint = true)
    )]
    async fn projecten(
        &self,
        Parameters(args): Parameters<ProjectenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let query = grid(args.limit, args.offset).map(|mut q| {
            push(&mut q, "relatieId", args.relatie_id);
            push(&mut q, "project", args.zoek.as_ref());
            q
        });
        self.uren_grid(
            &context,
            "projecten",
            &args,
            args.account.as_deref(),
            "project/gridtable",
            query,
        )
        .await
    }

    #[tool(
        description = "Eén project, op id (uit projecten): naam, relatie, omschrijving, facturabel en actief, de ids van de gebruikers die er uren op mogen schrijven, en de afwijkende tarieven per medewerker en activiteit.",
        annotations(title = "Project", read_only_hint = true)
    )]
    async fn project(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let run = match self.begin(&context, "project", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let result = async {
            let project = self
                .uren_record(
                    &run,
                    &format!("project/{}", args.id),
                    &format!("Project {}", args.id),
                )
                .await?;
            let tarieven = self
                .fetch(
                    &run,
                    &format!("afwijkendtarief/gridtable/{}", args.id),
                    grid(Some(MAX_ROWS), None)?,
                )
                .await
                .map(named_rows)?;
            Ok(json!({
                "project": project.get("project").cloned().unwrap_or(Value::Null),
                "gebruikers": project.get("users").cloned().unwrap_or(Value::Null),
                "afwijkende_tarieven": tarieven,
            }))
        }
        .await;
        Ok(self.end(run, &format!("project/{}", args.id), result))
    }

    #[tool(
        description = "Activiteiten voor de urenregistratie zoeken of opsommen, met tarieven. Te filteren op naam.",
        annotations(title = "Activiteiten", read_only_hint = true)
    )]
    async fn activiteiten(
        &self,
        Parameters(args): Parameters<ActiviteitenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let query = grid(args.limit, args.offset).map(|mut q| {
            push(&mut q, "activiteit", args.zoek.as_ref());
            q
        });
        self.uren_grid(
            &context,
            "activiteiten",
            &args,
            args.account.as_deref(),
            "activiteit/gridtable",
            query,
        )
        .await
    }

    #[tool(
        description = "Eén activiteit, op id (uit activiteiten): naam, omschrijving, uur- en kilometertarief, facturabel, factuuromschrijving, btw-code, tegenrekening (grootboekRekeningId) en actief.",
        annotations(title = "Activiteit", read_only_hint = true)
    )]
    async fn activiteit(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let path = format!("activiteit/{}", args.id);
        self.simple(
            &context,
            "activiteit",
            &args,
            args.account.as_deref(),
            &path,
            Ok(vec![]),
        )
        .await
    }

    #[tool(
        description = "De lijst van Uren > Factureren: urenregels met datum, facturabel, medewerker, project, activiteit, uren, uurtarief, kilometers, km-tarief, factuur en opmerkingen. Standaard de nog niet gefactureerde uren van deze maand. per_relatie zegt per relatie welke factuur er van de gelezen regels (deze pagina, volgens limit en offset) zou komen: de ids, het aantal factuurregels (één per project en activiteit), de uren, de kilometers en het bedrag volgens tarief (zonder btw). Let op: uren_factureren zonder uur_ids factureert alle regels die aan de filters voldoen, ook die op andere pagina's. Er verandert niets.",
        annotations(title = "Uren te factureren", read_only_hint = true)
    )]
    async fn uren_te_factureren(
        &self,
        Parameters(args): Parameters<UrenTeFacturerenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const PATH: &str = "uur/factureren/gridtable";
        let run = match self.begin(
            &context,
            "uren_te_factureren",
            &args,
            args.account.as_deref(),
        ) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let query = grid(args.limit, args.offset).and_then(|mut q| {
            q.extend(factureren_filters(
                args.van.as_deref(),
                args.tot.as_deref(),
                args.medewerker_id,
                args.project_id,
                args.activiteit_id,
                args.zoeken_in.unwrap_or(ZoekenIn::NogNietGefactureerd),
                args.ook_niet_facturabel.unwrap_or(false),
            )?);
            Ok(q)
        });
        let query = match query {
            Ok(query) => query,
            Err(message) => return Ok(self.refuse(run, message)),
        };
        let result = async {
            let grid = named_rows(self.fetch(&run, PATH, query).await?);
            let rows = grid
                .get("data")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let projecten = self.uren_project_relaties(&run, &rows).await?;
            let (groepen, los) = per_relatie(&rows, &projecten);
            Ok(json!({
                "rijen": grid,
                "per_relatie": groepen_json(&groepen),
                "zonder_relatie": los,
            }))
        }
        .await;
        Ok(self.end(run, PATH, result))
    }

    #[tool(
        description = "De factuurinstellingen van het menu Uren: eenheid voor uren en kilometers, factuursjabloon, opmaak van de factuurregel, betalingstermijn, en of facturen klaargezet worden voor post of e-mail, geïncasseerd en meteen verwerkt worden. Met de eenheden en actieve factuursjablonen om uit te kiezen. bestaat zegt of ze al zijn ingesteld; zonder kan uren_factureren niet.",
        annotations(title = "Factuurinstellingen uren", read_only_hint = true)
    )]
    async fn uren_factuurinstellingen(
        &self,
        Parameters(args): Parameters<AccountArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let run = match self.begin(
            &context,
            "uren_factuurinstellingen",
            &args,
            args.account.as_deref(),
        ) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let result = async {
            let (bestaat, instellingen) = self.uren_instellingen(&run).await?;
            let eenheden = self.fetch(&run, "eenheid", Vec::new()).await?;
            let sjablonen = self
                .fetch(&run, "factuursjabloon/active", Vec::new())
                .await?;
            Ok(json!({
                "bestaat": bestaat,
                "instellingen": instellingen,
                "eenheden": eenheden,
                "factuursjablonen": sjablonen,
            }))
        }
        .await;
        Ok(self.end(run, "factuursetting", result))
    }

    // ------------------------------------------------------------ hours

    #[tool(
        description = "Een urenregel toevoegen: datum, project, activiteit, en uren of kilometers (of beide), met opmerkingen. Zoek project en activiteit eerst op met projecten en activiteiten; ze moeten actief zijn. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt het eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Uren toevoegen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn uur_toevoegen(
        &self,
        Parameters(args): Parameters<UurToevoegenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "uur_toevoegen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Urenregel toegevoegd; data.id is het id",
            move |this, run, args| async move {
                // Bad input is refused before anything is read.
                uur_nieuw(&args, &Namen::default())?;
                let namen = this
                    .uren_namen(
                        &run,
                        &[args.project_id],
                        &[args.activiteit_id],
                        (Some(args.project_id), Some(args.activiteit_id)),
                        args.medewerker_id.is_some(),
                    )
                    .await?;
                if let Some(id) = args.medewerker_id
                    && !namen.medewerkers.contains_key(&id)
                {
                    return Err(Halt::Error(format!(
                        "Geen medewerker met id {id} in deze administratie; uren_medewerkers geeft ze"
                    )));
                }
                let built = uur_nieuw(&args, &namen)?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change(&run, "POST", "uur", Some(&built.body)).await
            },
        )
        .await
    }

    #[tool(
        description = "Een urenregel wijzigen, op id (uit uren). Wat je niet noemt blijft zoals het is. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de wijziging eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Uren wijzigen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn uur_wijzigen(
        &self,
        Parameters(args): Parameters<UurWijzigenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "uur_wijzigen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Urenregel gewijzigd",
            move |this, run, args| async move {
                let now = this
                    .uren_record(&run, &format!("uur/{}", args.id), &format!("Urenregel {}", args.id))
                    .await?;
                let ids = |key: &str, new: Option<u64>| -> Vec<u64> {
                    now.get(key)
                        .and_then(Value::as_u64)
                        .filter(|id| *id != 0)
                        .into_iter()
                        .chain(new)
                        .collect()
                };
                let new_project = args
                    .project_id
                    .filter(|p| Some(*p) != now.get("projectId").and_then(Value::as_u64));
                let new_activiteit = args
                    .activiteit_id
                    .filter(|a| Some(*a) != now.get("activiteitId").and_then(Value::as_u64));
                let namen = this
                    .uren_namen(
                        &run,
                        &ids("projectId", args.project_id),
                        &ids("activiteitId", args.activiteit_id),
                        (new_project, new_activiteit),
                        args.medewerker_id.is_some(),
                    )
                    .await?;
                if let Some(id) = args.medewerker_id
                    && !namen.medewerkers.contains_key(&id)
                {
                    return Err(Halt::Error(format!(
                        "Geen medewerker met id {id} in deze administratie; uren_medewerkers geeft ze"
                    )));
                }
                let built = uur_gewijzigd(&args, &now, &namen)?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change(&run, "PUT", "uur", Some(&built.body)).await
            },
        )
        .await
    }

    #[tool(
        description = "Een urenregel verwijderen, op id (uit uren). Dit is niet terug te draaien. De gebruiker moet dit altijd goedkeuren in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Uren verwijderen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn uur_verwijderen(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "uur_verwijderen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Urenregel verwijderd",
            move |this, run, args| async move {
                let now = this
                    .uren_record(
                        &run,
                        &format!("uur/{}", args.id),
                        &format!("Urenregel {}", args.id),
                    )
                    .await?;
                let id = |key: &str| -> Vec<u64> {
                    now.get(key)
                        .and_then(Value::as_u64)
                        .filter(|id| *id != 0)
                        .into_iter()
                        .collect()
                };
                let namen = this
                    .uren_namen(
                        &run,
                        &id("projectId"),
                        &id("activiteitId"),
                        (None, None),
                        false,
                    )
                    .await
                    .unwrap_or_default();
                let built = uur_weg(args.id, &now, &namen);
                this.approve(&run, TOOL, &built, true).await?;
                this.change_done(&run, "DELETE", &format!("uur/{}", args.id), None)
                    .await
            },
        )
        .await
    }

    // ------------------------------------------------------------ projects

    #[tool(
        description = "Een project voor de urenregistratie aanmaken bij een relatie. Naam en relatie zijn nodig. Standaard mogen alle gebruikers er uren op schrijven, zoals in e-Boekhouden. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt het eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Project toevoegen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn project_toevoegen(
        &self,
        Parameters(args): Parameters<ProjectToevoegenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "project_toevoegen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Project toegevoegd; data.project.id is het id",
            move |this, run, args| async move {
                let relatie = this
                    .uren_record(
                        &run,
                        &format!("relatie/{}", args.relatie_id),
                        &format!("Relatie {}", args.relatie_id),
                    )
                    .await?;
                let gebruikers = this.uren_gebruikers(&run).await?;
                let naam = relatie
                    .get("bedrijf")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let built = project_nieuw(&args, &known(naam.to_owned()), &gebruikers)?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change(&run, "POST", "project", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een project wijzigen, op id (uit projecten): naam, relatie, omschrijving, facturabel, actief of de gebruikers die er uren op mogen schrijven. Wat je niet noemt blijft zoals het is. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de wijziging eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Project wijzigen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn project_wijzigen(
        &self,
        Parameters(args): Parameters<ProjectWijzigenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "project_wijzigen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Project gewijzigd",
            move |this, run, args| async move {
                let now = this
                    .uren_record(
                        &run,
                        &format!("project/{}", args.id),
                        &format!("Project {}", args.id),
                    )
                    .await?;
                let mut relaties = HashMap::new();
                let relatie_nu = now
                    .get("project")
                    .and_then(|p| p.get("relatieId"))
                    .and_then(Value::as_u64)
                    .filter(|r| *r != 0);
                for id in relatie_nu.into_iter().chain(args.relatie_id) {
                    if relaties.contains_key(&id) {
                        continue;
                    }
                    let relatie = this
                        .uren_record(&run, &format!("relatie/{id}"), &format!("Relatie {id}"))
                        .await?;
                    relaties.insert(
                        id,
                        relatie
                            .get("bedrijf")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                    );
                }
                let gebruikers = this.uren_gebruikers(&run).await?;
                let built = project_gewijzigd(&args, &now, &relaties, &gebruikers)?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change(&run, "PUT", "project", Some(&built.body)).await
            },
        )
        .await
    }

    #[tool(
        description = "Een project verwijderen, op id (uit projecten). Dit is niet terug te draaien. De gebruiker moet dit altijd goedkeuren in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Project verwijderen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn project_verwijderen(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "project_verwijderen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Project verwijderd",
            move |this, run, args| async move {
                let project = this.uren_project(&run, args.id).await?;
                let text = |key: &str| {
                    project
                        .get(key)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                };
                let naam = text("naam");
                let relatie = match project.get("relatieId").and_then(Value::as_u64) {
                    Some(id) if id != 0 => this
                        .uren_record(&run, &format!("relatie/{id}"), &format!("Relatie {id}"))
                        .await
                        .ok()
                        .and_then(|r| r.get("bedrijf").and_then(Value::as_str).map(str::to_owned))
                        .unwrap_or_else(|| format!("relatie {id}")),
                    _ => "onbekend".to_owned(),
                };
                let built = Built {
                    body: Value::Null,
                    title: format!("Project verwijderen: {naam}"),
                    lines: vec![
                        line("Project", &naam),
                        line("Relatie", relatie),
                        line("Omschrijving", known(text("omschrijving"))),
                        line(
                            "Gevolg",
                            "het project verdwijnt uit de urenregistratie; dit is niet terug te draaien",
                        ),
                    ],
                };
                this.approve(&run, TOOL, &built, true).await?;
                this.change_done(&run, "DELETE", &format!("project/{}", args.id), None)
                    .await
            },
        )
        .await
    }

    // ------------------------------------------------------------ activities

    #[tool(
        description = "Een activiteit voor de urenregistratie aanmaken, met uur- en kilometertarief. Een facturabele activiteit heeft een tarief, een omzetrekening (tegenrekening, standaard 8000) en bij een btw-plichtige administratie een btw-code voor de verkoop nodig. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt het eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Activiteit toevoegen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn activiteit_toevoegen(
        &self,
        Parameters(args): Parameters<ActiviteitToevoegenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "activiteit_toevoegen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Activiteit toegevoegd; data.id is het id",
            move |this, run, args| async move {
                let verkoop = this.uren_verkoop(&run).await?;
                let built = activiteit_nieuw(&args, &verkoop)?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change(&run, "POST", "activiteit", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een activiteit wijzigen, op id (uit activiteiten): naam, omschrijving, tarieven, facturabel, factuuromschrijving, btw-code, tegenrekening of actief. Wat je niet noemt blijft zoals het is, ook de kostenplaats. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de wijziging eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Activiteit wijzigen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn activiteit_wijzigen(
        &self,
        Parameters(args): Parameters<ActiviteitWijzigenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "activiteit_wijzigen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Activiteit gewijzigd",
            move |this, run, args| async move {
                let now = this
                    .uren_record(
                        &run,
                        &format!("activiteit/{}", args.id),
                        &format!("Activiteit {}", args.id),
                    )
                    .await?;
                // The form reads the kostenplaats apart and sends it back with the change, but only when the
                // administration works with kostenplaatsen (`this.showKostenplaatsen && (t.kostenplaatsId = ...)`).
                let met_kostenplaatsen = setting_waarde(
                    &this
                        .uren_setting(&run, "administratie.use-kostenplaatsen")
                        .await?,
                    "kostenplaatsen gebruiken",
                )?
                .is_some_and(|v| v == "true");
                let kostenplaats = if met_kostenplaatsen {
                    let gevonden = this
                        .fetch(
                            &run,
                            &format!("kostenplaats/GetByActiviteitId/{}", args.id),
                            Vec::new(),
                        )
                        .await
                        .map_err(|_| {
                            Halt::Error(
                                "De kostenplaats van deze activiteit is niet te lezen, dus EB MCP kan hem niet bewaren; er is niets gewijzigd.".into(),
                            )
                        })?;
                    if !gevonden.is_null() && !gevonden.is_object() {
                        return Err(Halt::Error(
                            "e-Boekhouden gaf de kostenplaats van deze activiteit in een onbekende vorm terug; er is niets gewijzigd.".into(),
                        ));
                    }
                    gevonden
                        .get("id")
                        .and_then(Value::as_u64)
                        .filter(|id| *id != 0)
                } else {
                    None
                };
                let verkoop = this.uren_verkoop(&run).await?;
                let built = activiteit_gewijzigd(&args, &now, kostenplaats, &verkoop)?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change(&run, "PUT", "activiteit", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een activiteit verwijderen, op id (uit activiteiten). Dit is niet terug te draaien. De gebruiker moet dit altijd goedkeuren in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Activiteit verwijderen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn activiteit_verwijderen(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "activiteit_verwijderen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Activiteit verwijderd",
            move |this, run, args| async move {
                let activiteit = this
                    .uren_record(
                        &run,
                        &format!("activiteit/{}", args.id),
                        &format!("Activiteit {}", args.id),
                    )
                    .await?;
                let text = |key: &str| {
                    activiteit
                        .get(key)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                };
                let naam = text("naam");
                let built = Built {
                    body: Value::Null,
                    title: format!("Activiteit verwijderen: {naam}"),
                    lines: vec![
                        line("Activiteit", &naam),
                        line("Omschrijving", known(text("omschrijving"))),
                        line(
                            "Uurtarief",
                            activiteit
                                .get("uurtarief")
                                .and_then(Value::as_f64)
                                .map_or("onbekend".to_owned(), write::euro),
                        ),
                        line(
                            "Gevolg",
                            "de activiteit verdwijnt uit de urenregistratie; dit is niet terug te draaien",
                        ),
                    ],
                };
                this.approve(&run, TOOL, &built, true).await?;
                this.change_done(&run, "DELETE", &format!("activiteit/{}", args.id), None)
                    .await
            },
        )
        .await
    }

    // ------------------------------------------------------------ invoicing hours

    #[tool(
        description = "Facturen maken van uren, zoals Uren > Factureren in e-Boekhouden: voor de nog niet gefactureerde uren van de periode en filters (standaard deze maand), of alleen de urenregels in uur_ids, maakt e-Boekhouden per relatie één factuur, met een factuurregel per project en activiteit. Bekijk eerst met uren_te_factureren wat er gefactureerd wordt. De factuurinstellingen (uren_factuurinstellingen) bepalen het sjabloon en of de facturen meteen verwerkt, voor post of e-mail klaargezet of geïncasseerd worden. Na het akkoord leest EB MCP de uren opnieuw; zijn ze intussen gewijzigd of gefactureerd, dan gebeurt er niets. Dit is niet terug te draaien. De gebruiker moet dit altijd goedkeuren in EB MCP en ziet per relatie wat er gefactureerd wordt. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Uren factureren",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn uren_factureren(
        &self,
        Parameters(args): Parameters<UrenFacturerenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "uren_factureren";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Facturen gemaakt",
            move |this, run, args| async move {
                let filters = factureren_filters(
                    args.van.as_deref(),
                    args.tot.as_deref(),
                    args.medewerker_id,
                    args.project_id,
                    args.activiteit_id,
                    ZoekenIn::NogNietGefactureerd,
                    args.ook_niet_facturabel.unwrap_or(false),
                )?;
                let (bestaat, instellingen) = this.uren_instellingen(&run).await?;
                if !bestaat {
                    return Err(Halt::Error(
                        "Je hebt nog geen factuurinstellingen voor uren ingesteld. Stel ze eerst in met uren_factuurinstellingen_wijzigen.".into(),
                    ));
                }
                let rows = this.uren_alle_rijen(&run, &filters).await?;
                let rows = kies_rijen(rows, args.uur_ids.as_deref())?;
                let projecten = this.uren_project_relaties(&run, &rows).await?;
                let sjabloon = match instellingen.get("factuurSjabloonId").and_then(Value::as_u64) {
                    Some(id) => this
                        .fetch(&run, &format!("factuursjabloon/{id}"), Vec::new())
                        .await
                        .ok()
                        .and_then(|s| s.get("naam").and_then(Value::as_str).map(str::to_owned))
                        .unwrap_or_else(|| format!("sjabloon {id}")),
                    None => String::new(),
                };
                let built = factureren::uren_factureren(&rows, &projecten, &instellingen, &sjabloon)?;
                this.approve(&run, TOOL, &built, true).await?;
                // The approval can wait for hours: the hours go out only as the user saw them.
                let nu = this.uren_alle_rijen(&run, &filters).await?;
                nog_zoals_goedgekeurd(&rows, &nu)?;
                this.change(&run, "POST", "uur/factureren", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Urenregels op een bestaande factuur zetten, zoals Factureren > Toevoegen aan bestaande factuur in e-Boekhouden: de uren komen als regels op die factuur. Geef het factuurnummer en de ids van de urenregels (uit uren_te_factureren). De uren moeten nog niet gefactureerd zijn en bij een project van de relatie van die factuur horen. Dit is niet terug te draaien. De gebruiker moet dit altijd goedkeuren in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Uren bij factuur",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn uren_bij_factuur(
        &self,
        Parameters(args): Parameters<UrenBijFactuurArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "uren_bij_factuur";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Uren op de factuur gezet",
            move |this, run, args| async move {
                let nummer = args.factuurnummer.trim().to_owned();
                if nummer.is_empty() {
                    return Err(Halt::Error("Geef het nummer van de factuur".into()));
                }
                let mut ids: Vec<u64> = Vec::new();
                for id in &args.uur_ids {
                    if !ids.contains(id) {
                        ids.push(*id);
                    }
                }
                if ids.is_empty() {
                    return Err(Halt::Error("Geef minstens één urenregel in uur_ids".into()));
                }
                if ids.len() > MAX_ROWS as usize {
                    return Err(Halt::Error(format!(
                        "Hoogstens {MAX_ROWS} urenregels tegelijk"
                    )));
                }
                let gevonden = this
                    .fetch(
                        &run,
                        "factuur/searchexisting",
                        vec![("searchQuery".into(), nummer.clone())],
                    )
                    .await?;
                let factuur = factuur_bij_nummer(&gevonden, &nummer)?;
                let mut uren = Vec::new();
                for id in &ids {
                    let uur = this
                        .uren_record(&run, &format!("uur/{id}"), &format!("Urenregel {id}"))
                        .await?;
                    uren.push((*id, uur));
                }
                let collect = |key: &str| -> Vec<u64> {
                    let mut all: Vec<u64> = uren
                        .iter()
                        .filter_map(|(_, u)| u.get(key).and_then(Value::as_u64))
                        .filter(|id| *id != 0)
                        .collect();
                    all.sort_unstable();
                    all.dedup();
                    all
                };
                let namen = this
                    .uren_namen(
                        &run,
                        &collect("projectId"),
                        &collect("activiteitId"),
                        (None, None),
                        false,
                    )
                    .await
                    .unwrap_or_default();
                // The invoice's relatie, and the hours that are not invoiced yet over the days of these hours
                // (facturabel or not), read as uren_factureren reads them.
                let factuur_id = factuur
                    .get("factuurId")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let factuur_relatie = this
                    .uren_record(
                        &run,
                        &format!("factuur/{factuur_id}"),
                        &format!("Factuur {nummer}"),
                    )
                    .await?
                    .get("relatieId")
                    .and_then(Value::as_u64)
                    .filter(|r| *r != 0)
                    .ok_or_else(|| Halt::Error(format!("Factuur {nummer} heeft geen relatie")))?;
                let mut dagen = Vec::new();
                for (id, uur) in &uren {
                    let dag = uur.get("datum").and_then(datum_of).ok_or_else(|| {
                        Halt::Error(format!(
                            "e-Boekhouden gaf urenregel {id} zonder datum terug"
                        ))
                    })?;
                    dagen.push(dag);
                }
                let (van, tot) = (
                    dagen.iter().min().copied().unwrap_or_else(today),
                    dagen.iter().max().copied().unwrap_or_else(today),
                );
                let filters = factureren_filters(
                    Some(&dashed(van)),
                    Some(&dashed(tot)),
                    None,
                    None,
                    None,
                    ZoekenIn::NogNietGefactureerd,
                    true,
                )?;
                let lijst = this.uren_alle_rijen(&run, &filters).await?;
                let records: Vec<Value> = uren.iter().map(|(_, u)| u.clone()).collect();
                let projecten = this.uren_project_relaties(&run, &records).await?;
                let built = factureren::uren_bij_factuur(
                    &factuur,
                    factuur_relatie,
                    &uren,
                    &lijst,
                    &projecten,
                    &namen,
                )?;
                this.approve(&run, TOOL, &built, true).await?;
                this.change_done(&run, "POST", "uur/bestaand/factureren", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "De factuurinstellingen van het menu Uren wijzigen: eenheid voor uren en kilometers, factuursjabloon, opmaak van de factuurregel, betalingstermijn, en of facturen klaargezet worden voor post of e-mail, geïncasseerd en meteen verwerkt worden. Wat je niet noemt blijft zoals het is. Het factuursjabloon en verwerken gelden ook voor facturen die factuur_toevoegen maakt. De gebruiker moet dit altijd goedkeuren in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Factuurinstellingen uren wijzigen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn uren_factuurinstellingen_wijzigen(
        &self,
        Parameters(args): Parameters<FactuurinstellingenWijzigenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "uren_factuurinstellingen_wijzigen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Factuurinstellingen gewijzigd",
            move |this, run, args| async move {
                let (_, now) = this.uren_instellingen(&run).await?;
                let (eenheden, sjablonen) = this.uren_instelling_keuzes(&run).await?;
                let built = instellingen_gewijzigd(&args, &now, &eenheden, &sjablonen)?;
                this.approve(&run, TOOL, &built, true).await?;
                this.change(&run, "PUT", "factuurSetting", Some(&built.body))
                    .await
            },
        )
        .await
    }
}
