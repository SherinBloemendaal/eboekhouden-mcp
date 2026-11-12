//! Grootboekrekeningen, rekeninggroepen and kostenplaatsen.
//!
//! The tools of this part live here, on their own router (`rekeningschema_router`), which tools.rs adds to the
//! server's. Each tool is named in one of the lists below, which the tool-list test in mcp.rs reads.
//!
//! Every path and body comes from e-Boekhouden's own web app (the public bundle): the services `grootboekrekening`,
//! `rekeninggroep` and `kostenplaats` and their forms. The bodies and approval summaries are built in `build`;
//! here the tools read what they need, pass `approve` and send the one change.

use super::*;

mod build;
#[cfg(test)]
mod e2e;

use build::{Categorie, GridRekening, Groep};
use write::Kostenplaats;

/// The tools of this part that only read.
#[cfg(test)]
pub(super) const READS: &[&str] = &[
    "grootboekrekening",
    "grootboekrekening_categorieen",
    "rekeninggroepen",
    "kostenplaatsen",
    "kostenplaats",
];
/// The tools of this part that change the books.
#[cfg(test)]
pub(super) const WRITES: &[&str] = &[
    "grootboekrekening_toevoegen",
    "grootboekrekening_wijzigen",
    "grootboekrekening_verwijderen",
    "rekeninggroep_toevoegen",
    "rekeninggroep_wijzigen",
    "rekeninggroep_verwijderen",
    "kostenplaats_toevoegen",
    "kostenplaats_wijzigen",
    "kostenplaats_verwijderen",
];
/// The ones in `WRITES` that cannot be undone.
#[cfg(test)]
pub(super) const DESTRUCTIVE: &[&str] = &[
    "grootboekrekening_verwijderen",
    "rekeninggroep_verwijderen",
    "kostenplaats_verwijderen",
];

// ---------------------------------------------------------------- parameters

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GrootboekrekeningArgs {
    /// Id van de grootboekrekening (uit grootboek). Geef id of code.
    pub id: Option<u64>,
    /// Code van de grootboekrekening, bijvoorbeeld "4550". Geef id of code.
    pub code: Option<String>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GrootboekrekeningToevoegenArgs {
    /// Code van de nieuwe rekening, hoogstens 10 tekens en uniek in het rekeningschema, bijvoorbeeld "4560".
    pub code: String,
    /// Omschrijving, hoogstens 100 tekens.
    pub omschrijving: String,
    /// Categorie, op code of naam (zie grootboekrekening_categorieen). Standaard VW, winst en verlies, zoals het formulier van e-Boekhouden.
    pub categorie: Option<String>,
    /// Rekeninggroep, op code of naam (zie rekeninggroepen). Standaard geen.
    pub groep: Option<String>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GrootboekrekeningWijzigenArgs {
    /// Id van de grootboekrekening (uit grootboek). Geef id of code.
    pub id: Option<u64>,
    /// Huidige code van de grootboekrekening. Geef id of code.
    pub code: Option<String>,
    /// Nieuwe code, hoogstens 10 tekens. Vraagt altijd akkoord: de boekingen op de rekening gaan mee.
    pub nieuwe_code: Option<String>,
    /// Nieuwe omschrijving, hoogstens 100 tekens.
    pub omschrijving: Option<String>,
    /// Nieuwe categorie, op code of naam (zie grootboekrekening_categorieen). Vraagt altijd akkoord.
    pub categorie: Option<String>,
    /// Nieuwe rekeninggroep, op code of naam (zie rekeninggroepen); een lege tekst haalt de rekening uit zijn groep.
    pub groep: Option<String>,
    /// true maakt de rekening inactief, false weer actief.
    pub inactief: Option<bool>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct RekeninggroepToevoegenArgs {
    /// Code van de groep, hoogstens 50 tekens en uniek.
    pub code: String,
    /// Naam van de groep, hoogstens 50 tekens.
    pub naam: String,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct RekeninggroepWijzigenArgs {
    /// Id van de groep (uit rekeninggroepen).
    pub id: u64,
    /// Nieuwe code. Vraagt altijd akkoord: de rekeningen van de groep verwijzen naar de groep met zijn code.
    pub code: Option<String>,
    /// Nieuwe naam.
    pub naam: Option<String>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct KostenplaatsenArgs {
    /// Alleen kostenplaatsen waarvan de omschrijving of het pad dit bevat.
    pub zoek: Option<String>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct KostenplaatsToevoegenArgs {
    /// Omschrijving, hoogstens 50 tekens, zonder < of >.
    pub omschrijving: String,
    /// Id van de kostenplaats waaronder de nieuwe komt (uit kostenplaatsen). Leeg: bovenaan.
    pub onder_id: Option<u64>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct KostenplaatsWijzigenArgs {
    /// Id van de kostenplaats (uit kostenplaatsen).
    pub id: u64,
    /// Nieuwe omschrijving, hoogstens 50 tekens, zonder < of >.
    pub omschrijving: Option<String>,
    /// true maakt de kostenplaats inactief, false weer actief.
    pub inactief: Option<bool>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

// ---------------------------------------------------------------- reading what a change needs

/// The setting e-Boekhouden's mutatie form reads to show the kostenplaats of a line (`administratieUseKostenplaatsen`).
const KOSTENPLAATSEN_AAN: &str = "setting/administratie.use-kostenplaatsen";
/// The setting the grootboekrekening form reads to send a rekening's RGS code (`administratieUseRgs`).
const RGS: &str = "setting/administratie.use-rgs";

impl EbMcp {
    /// Every grootboekrekening of the administration, inactive ones included.
    async fn alle_rekeningen(&self, run: &Run) -> Result<Vec<GridRekening>, String> {
        let all = vec![
            ("offset".to_owned(), "0".to_owned()),
            ("limit".to_owned(), "2000".to_owned()),
            ("orderBy".to_owned(), String::new()),
            ("sortDirection".to_owned(), String::new()),
        ];
        let grid = self.fetch(run, "grootboekrekening/gridtable", all).await?;
        build::rekeningen_from(&named_rows(grid))
    }

    /// The categories of grootboekrekening; `restricted` adds the ones the form does not offer, as its edit
    /// screen asks (`incudeRestricted` is e-Boekhouden's own spelling).
    async fn categorieen(&self, run: &Run, restricted: bool) -> Result<Vec<Categorie>, String> {
        let list = self
            .fetch(
                run,
                "grootboekrekening/categories",
                vec![("incudeRestricted".to_owned(), restricted.to_string())],
            )
            .await?;
        build::categorieen_from(&list)
    }

    async fn groepen(&self, run: &Run) -> Result<Vec<Groep>, String> {
        build::groepen_from(&self.fetch(run, "rekeninggroep", Vec::new()).await?)
    }

    /// Whether the administration uses RGS codes, as the grootboekrekening form decides it (`hasRgs`): the setting
    /// `administratie.use-rgs` is there and is not "nee".
    async fn rgs(&self, run: &Run) -> Result<bool, String> {
        let setting = self
            .fetch(run, RGS, vec![("acc".to_owned(), "false".to_owned())])
            .await?;
        Ok(setting
            .get("value")
            .and_then(Value::as_str)
            .is_some_and(|v| !v.trim().eq_ignore_ascii_case("nee")))
    }

    async fn kostenplaats_lijst(&self, run: &Run) -> Result<Vec<Kostenplaats>, String> {
        write::kostenplaatsen_from_api(&self.fetch(run, "kostenplaats", Vec::new()).await?)
    }

    /// The kostenplaatsen a mutatie needs. `named`: a line the tool was given names one, so the administration must
    /// use kostenplaatsen (the mutatie form shows the field only then) and the list is read to check it. `kept`: a
    /// line the mutatie keeps has one, and the list is read only to show its name (by id when it cannot be read).
    pub(super) async fn mutatie_kostenplaatsen(
        &self,
        run: &Run,
        named: bool,
        kept: bool,
    ) -> Result<Vec<Kostenplaats>, String> {
        if named {
            let setting = self
                .fetch(
                    run,
                    KOSTENPLAATSEN_AAN,
                    vec![("acc".to_owned(), "false".to_owned())],
                )
                .await?;
            let aan = setting.get("value").is_some_and(|v| {
                v.as_bool() == Some(true)
                    || v.as_str().is_some_and(|s| s.eq_ignore_ascii_case("true"))
            });
            if !aan {
                return Err(
                    "Deze administratie gebruikt geen kostenplaatsen; zet ze eerst aan in e-Boekhouden, of laat kostenplaats_id weg"
                        .into(),
                );
            }
            return self.kostenplaats_lijst(run).await;
        }
        if kept {
            return Ok(self.kostenplaats_lijst(run).await.unwrap_or_default());
        }
        Ok(Vec::new())
    }
}

#[tool_router(router = rekeningschema_router, vis = "pub(super)")]
impl EbMcp {
    // ------------------------------------------------------------ reading

    #[tool(
        description = "Alle gegevens van één grootboekrekening, op id of code: code, omschrijving, rekeningCategorie (de code van de categorie), rekeningGroepCode, inactief en wat de administratie verder bijhoudt. De lijst van alle rekeningen geeft grootboek.",
        annotations(title = "Grootboekrekening", read_only_hint = true)
    )]
    async fn grootboekrekening(
        &self,
        Parameters(args): Parameters<GrootboekrekeningArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let run = match self.begin(
            &context,
            "grootboekrekening",
            &args,
            args.account.as_deref(),
        ) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let id = match (args.id, args.code.as_deref()) {
            (Some(id), None) => id,
            (None, None) => {
                return Ok(self.refuse(run, "Geef id of code van de grootboekrekening".into()));
            }
            (id, code) => {
                let rekeningen = match self.alle_rekeningen(&run).await {
                    Ok(rekeningen) => rekeningen,
                    Err(message) => {
                        return Ok(self.end(run, "grootboekrekening/gridtable", Err(message)));
                    }
                };
                match build::which_rekening(&rekeningen, id, code) {
                    Ok(id) => id,
                    Err(message) => return Ok(self.refuse(run, message)),
                }
            }
        };
        let path = format!("grootboekrekening/{id}");
        let result = self.fetch(&run, &path, Vec::new()).await;
        Ok(self.end(run, &path, result))
    }

    #[tool(
        description = "De categorieën die een grootboekrekening kan hebben (code en omschrijving), zoals het formulier van e-Boekhouden ze aanbiedt. Een categorie met restricted true is niet te kiezen.",
        annotations(title = "Categorieën van grootboekrekeningen", read_only_hint = true)
    )]
    async fn grootboekrekening_categorieen(
        &self,
        Parameters(args): Parameters<AccountArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        self.simple(
            &context,
            "grootboekrekening_categorieen",
            &args,
            args.account.as_deref(),
            "grootboekrekening/categories",
            Ok(vec![("incudeRestricted".to_owned(), "true".to_owned())]),
        )
        .await
    }

    #[tool(
        description = "De rekeninggroepen van het rekeningschema, met id, code en naam. Een grootboekrekening hoort bij een groep via zijn code.",
        annotations(title = "Rekeninggroepen", read_only_hint = true)
    )]
    async fn rekeninggroepen(
        &self,
        Parameters(args): Parameters<ListArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const PATH: &str = "rekeninggroep/gridtable";
        let run = match self.begin(&context, "rekeninggroepen", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let query = match grid(args.limit.or(Some(MAX_ROWS)), args.offset) {
            Ok(query) => query,
            Err(message) => return Ok(self.refuse(run, message)),
        };
        let result = self.fetch(&run, PATH, query).await.map(named_rows);
        Ok(self.end(run, PATH, result))
    }

    #[tool(
        description = "De kostenplaatsen van de administratie, een boom: per kostenplaats id, omschrijving, parentKostenplaatsId (0 is bovenaan), actief en volledigPad (zoals \"Projecten>Noord\"). Een mutatieregel kan een actieve kostenplaats krijgen (kostenplaats_id bij mutatie_toevoegen en mutatie_wijzigen).",
        annotations(title = "Kostenplaatsen", read_only_hint = true)
    )]
    async fn kostenplaatsen(
        &self,
        Parameters(args): Parameters<KostenplaatsenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const PATH: &str = "kostenplaats";
        let run = match self.begin(&context, "kostenplaatsen", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let zoek = args
            .zoek
            .as_deref()
            .map(|z| z.trim().to_lowercase())
            .filter(|z| !z.is_empty());
        let result = self
            .fetch(&run, PATH, Vec::new())
            .await
            .map(|list| match (zoek, list) {
                (Some(zoek), Value::Array(rows)) => Value::Array(
                    rows.into_iter()
                        .filter(|row| {
                            ["omschrijving", "volledigPad"].iter().any(|key| {
                                row.get(key)
                                    .and_then(Value::as_str)
                                    .is_some_and(|t| t.to_lowercase().contains(&zoek))
                            })
                        })
                        .collect(),
                ),
                (_, list) => list,
            });
        Ok(self.end(run, PATH, result))
    }

    #[tool(
        description = "Eén kostenplaats, op id (uit kostenplaatsen).",
        annotations(title = "Kostenplaats", read_only_hint = true)
    )]
    async fn kostenplaats(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let path = format!("kostenplaats/{}", args.id);
        self.simple(
            &context,
            "kostenplaats",
            &args,
            args.account.as_deref(),
            &path,
            Ok(Vec::new()),
        )
        .await
    }

    // ------------------------------------------------------------ grootboekrekeningen

    #[tool(
        description = "Een grootboekrekening aan het rekeningschema toevoegen, zoals Beheer > Inrichting > Grootboekrekeningen in e-Boekhouden: code (uniek, hoogstens 10 tekens), omschrijving, categorie (standaard VW, winst en verlies; zie grootboekrekening_categorieen) en eventueel een rekeninggroep. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de rekening eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Grootboekrekening toevoegen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn grootboekrekening_toevoegen(
        &self,
        Parameters(args): Parameters<GrootboekrekeningToevoegenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "grootboekrekening_toevoegen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Grootboekrekening toegevoegd; data.id is het id",
            move |this, run, args| async move {
                let rekeningen = this.alle_rekeningen(&run).await?;
                let categorieen = this.categorieen(&run, false).await?;
                let groepen = this.groepen(&run).await?;
                let built =
                    build::grootboekrekening_nieuw(&args, &rekeningen, &categorieen, &groepen)?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change_done(&run, "POST", "grootboekrekening", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een grootboekrekening wijzigen, op id of code: nieuwe code, omschrijving, categorie, rekeninggroep of inactief. Wat je niet noemt blijft zoals het is. Een andere code of categorie geldt ook voor alle boekingen die al op de rekening staan; zo'n wijziging vraagt daarom altijd het akkoord van de gebruiker. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Grootboekrekening wijzigen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn grootboekrekening_wijzigen(
        &self,
        Parameters(args): Parameters<GrootboekrekeningWijzigenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "grootboekrekening_wijzigen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Grootboekrekening gewijzigd",
            move |this, run, args| async move {
                let rekeningen = this.alle_rekeningen(&run).await?;
                let id = build::which_rekening(&rekeningen, args.id, args.code.as_deref())?;
                let now = this
                    .fetch(&run, &format!("grootboekrekening/{id}"), Vec::new())
                    .await?;
                let categorieen = this.categorieen(&run, true).await?;
                let groepen = this.groepen(&run).await?;
                let rgs = this.rgs(&run).await?;
                let (built, serious) = build::grootboekrekening_gewijzigd(
                    &args,
                    id,
                    &now,
                    &rekeningen,
                    &categorieen,
                    &groepen,
                    rgs,
                )?;
                this.approve(&run, TOOL, &built, serious).await?;
                this.change_done(&run, "PUT", "grootboekrekening", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een grootboekrekening uit het rekeningschema verwijderen, op id of code. Dit is niet terug te draaien; staan er nog boekingen op de rekening, dan kan e-Boekhouden weigeren. Wil je hem alleen niet meer gebruiken, zet hem dan op inactief met grootboekrekening_wijzigen. De gebruiker moet dit altijd goedkeuren in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Grootboekrekening verwijderen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn grootboekrekening_verwijderen(
        &self,
        Parameters(args): Parameters<GrootboekrekeningArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "grootboekrekening_verwijderen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Grootboekrekening verwijderd",
            move |this, run, args| async move {
                let rekeningen = this.alle_rekeningen(&run).await?;
                let id = build::which_rekening(&rekeningen, args.id, args.code.as_deref())?;
                let now = this
                    .fetch(&run, &format!("grootboekrekening/{id}"), Vec::new())
                    .await?;
                let categorieen = this.categorieen(&run, true).await?;
                let built = build::grootboekrekening_weg(&now, &categorieen);
                this.approve(&run, TOOL, &built, true).await?;
                this.change_done(&run, "DELETE", &format!("grootboekrekening/{id}"), None)
                    .await
            },
        )
        .await
    }

    // ------------------------------------------------------------ rekeninggroepen

    #[tool(
        description = "Een rekeninggroep toevoegen, met code (uniek) en naam, elk hoogstens 50 tekens. Daarna kun je grootboekrekeningen in de groep zetten met grootboekrekening_wijzigen. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de groep eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Rekeninggroep toevoegen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn rekeninggroep_toevoegen(
        &self,
        Parameters(args): Parameters<RekeninggroepToevoegenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "rekeninggroep_toevoegen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Rekeninggroep toegevoegd; data.id is het id",
            move |this, run, args| async move {
                let groepen = this.groepen(&run).await?;
                let built = build::rekeninggroep_nieuw(&args, &groepen)?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change_done(&run, "POST", "rekeninggroep", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een rekeninggroep wijzigen, op id (uit rekeninggroepen): code of naam. Een andere code vraagt altijd het akkoord van de gebruiker, omdat de rekeningen van de groep er met die code naar verwijzen. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Rekeninggroep wijzigen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn rekeninggroep_wijzigen(
        &self,
        Parameters(args): Parameters<RekeninggroepWijzigenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "rekeninggroep_wijzigen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Rekeninggroep gewijzigd",
            move |this, run, args| async move {
                let groepen = this.groepen(&run).await?;
                build::groep_bekend(&groepen, args.id)?;
                let now = this
                    .fetch(&run, &format!("rekeninggroep/{}", args.id), Vec::new())
                    .await?;
                let rekeningen = this.alle_rekeningen(&run).await?;
                let (built, serious) =
                    build::rekeninggroep_gewijzigd(&args, &now, &groepen, &rekeningen)?;
                this.approve(&run, TOOL, &built, serious).await?;
                this.change_done(&run, "PUT", "rekeninggroep", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een rekeninggroep verwijderen, op id (uit rekeninggroepen). Dit is niet terug te draaien. De gebruiker moet dit altijd goedkeuren in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Rekeninggroep verwijderen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn rekeninggroep_verwijderen(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "rekeninggroep_verwijderen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Rekeninggroep verwijderd",
            move |this, run, args| async move {
                let groepen = this.groepen(&run).await?;
                build::groep_bekend(&groepen, args.id)?;
                let now = this
                    .fetch(&run, &format!("rekeninggroep/{}", args.id), Vec::new())
                    .await?;
                let rekeningen = this.alle_rekeningen(&run).await?;
                let built = build::rekeninggroep_weg(args.id, &now, &rekeningen)?;
                this.approve(&run, TOOL, &built, true).await?;
                this.change_done(&run, "DELETE", &format!("rekeninggroep/{}", args.id), None)
                    .await
            },
        )
        .await
    }

    // ------------------------------------------------------------ kostenplaatsen

    #[tool(
        description = "Een kostenplaats toevoegen, bovenaan of onder een andere (onder_id, uit kostenplaatsen), zoals Boekhouden > Kostenplaatsen in e-Boekhouden. Onder een inactieve kostenplaats kan dat niet. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de kostenplaats eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Kostenplaats toevoegen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn kostenplaats_toevoegen(
        &self,
        Parameters(args): Parameters<KostenplaatsToevoegenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "kostenplaats_toevoegen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Kostenplaats toegevoegd; data.id is het id",
            move |this, run, args| async move {
                let lijst = this.kostenplaats_lijst(&run).await?;
                let built = build::kostenplaats_nieuw(&args, &lijst)?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change_done(&run, "POST", "kostenplaats", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een kostenplaats wijzigen, op id (uit kostenplaatsen): omschrijving of inactief. De plaats in de boom blijft. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de wijziging eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Kostenplaats wijzigen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn kostenplaats_wijzigen(
        &self,
        Parameters(args): Parameters<KostenplaatsWijzigenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "kostenplaats_wijzigen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Kostenplaats gewijzigd",
            move |this, run, args| async move {
                let lijst = this.kostenplaats_lijst(&run).await?;
                let built = build::kostenplaats_gewijzigd(&args, &lijst)?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change_done(&run, "PUT", "kostenplaats", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een kostenplaats verwijderen, op id (uit kostenplaatsen). Dit is niet terug te draaien; het akkoordvenster noemt de kostenplaatsen die eronder staan. De gebruiker moet dit altijd goedkeuren in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Kostenplaats verwijderen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn kostenplaats_verwijderen(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "kostenplaats_verwijderen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Kostenplaats verwijderd",
            move |this, run, args| async move {
                let lijst = this.kostenplaats_lijst(&run).await?;
                let built = build::kostenplaats_weg(args.id, &lijst)?;
                this.approve(&run, TOOL, &built, true).await?;
                this.change_done(&run, "DELETE", &format!("kostenplaats/{}", args.id), None)
                    .await
            },
        )
        .await
    }
}
