//! Following up quotes, the invoice extras (incasso, Peppol, bulk mail and print) and smaller additions to existing tools.
//!
//! The tools of this part live here, on their own router (`verkoop_router`), which tools.rs adds to the
//! server's. Each tool is named in one of the lists below, which the tool-list test in mcp.rs reads.
//! What they send is built in `bodies` (pure, tested on its own); the requests come from e-Boekhouden's own
//! web app (the public bundle) and the live capture, as written at each builder.

use super::*;

mod bodies;
#[cfg(test)]
mod tests;

use bodies::*;

/// The tools of this part that only read.
#[cfg(test)]
pub(super) const READS: &[&str] = &[
    "offerte_voortgang",
    "offerte_agenda",
    "emailsjabloon_codes",
    "facturen_afdrukken",
];
/// The tools of this part that change the books.
#[cfg(test)]
pub(super) const WRITES: &[&str] = &[
    "offerte_mailen",
    "offerte_factureren",
    "offerte_voortgang_toevoegen",
    "offerte_voortgang_wijzigen",
    "offerte_voortgang_verwijderen",
    "offerte_agenda_toevoegen",
    "offerte_agenda_wijzigen",
    "offerte_agenda_verwijderen",
    "incassobestand_maken",
    "peppol_versturen",
    "facturen_mailen",
    "archief_ontkoppelen",
];
/// The ones in `WRITES` that cannot be undone.
#[cfg(test)]
pub(super) const DESTRUCTIVE: &[&str] = &[
    "offerte_mailen",
    "offerte_voortgang_verwijderen",
    "offerte_agenda_verwijderen",
    "incassobestand_maken",
    "peppol_versturen",
    "facturen_mailen",
];

impl EbMcp {
    /// The quote's number, read before anything is built: it also proves the quote exists.
    async fn offerte_nummer(&self, run: &Run, id: u64) -> Result<String, String> {
        let offerte = self
            .fetch(run, &format!("offerte/{id}"), Vec::new())
            .await?;
        Ok(s(&offerte, "nummer"))
    }

    /// Whether `id` is one of the quote's entries in a list (`offertevoortgang` or `offerteagenda`), so a tool
    /// never touches an entry of another quote.
    async fn offerte_entry(
        &self,
        run: &Run,
        list: &str,
        offerte_id: u64,
        id: u64,
    ) -> Result<(), String> {
        // Page through every entry: a quote with many is not a reason to say an entry is not its own.
        let mut offset = 0;
        loop {
            let page = rows(&named_rows(
                self.fetch(
                    run,
                    &format!("{list}/gridtable/{offerte_id}"),
                    grid(Some(MAX_ROWS), Some(offset))?,
                )
                .await?,
            ));
            if page.iter().any(|r| id_of(r, "id") == Some(id)) {
                return Ok(());
            }
            if page.len() < MAX_ROWS as usize || offset >= 100 * MAX_ROWS {
                break;
            }
            offset += MAX_ROWS;
        }
        Err(format!(
            "Regel {id} hoort niet bij offerte {offerte_id}; kijk met offerte_voortgang of offerte_agenda welke er zijn"
        ))
    }
}

#[tool_router(router = verkoop_router, vis = "pub(super)")]
impl EbMcp {
    #[tool(
        description = "Een offerte per e-mail versturen vanuit e-Boekhouden, standaard met de offerte als pdf in de bijlage. Onderwerp en bericht komen uit een e-mailsjabloon voor offertes (sjabloon_id) of geef je zelf mee; zonder ontvanger gaat de mail naar het e-mailadres van de relatie. Dit verstuurt echte e-mail en is niet terug te draaien. De gebruiker moet dit altijd goedkeuren in EB MCP en ziet daar de ontvanger, het onderwerp en het bericht. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Offerte mailen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn offerte_mailen(
        &self,
        Parameters(args): Parameters<OfferteMailenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "offerte_mailen";
        let account = args.account.clone();
        self.write_tool(&context, TOOL, account.as_deref(), args, "Offerte verstuurd per e-mail", move |this, run, args| async move {
            let offerte = this.fetch(&run, &format!("offerte/{}", args.id), Vec::new()).await?;
            let nummer = s(&offerte, "nummer");
            let relatie_id = id_of(&offerte, "relatieId").unwrap_or(0);
            let relatie = this.fetch(&run, &format!("relatie/{relatie_id}"), Vec::new()).await?;
            let template = match args.sjabloon_id {
                Some(sjabloon) => {
                    let voor_offertes = this.fetch(&run, "emailsjabloonab/forofferte", Vec::new()).await?;
                    if !rows(&voor_offertes).iter().any(|t| id_of(t, "id") == Some(sjabloon)) {
                        return Err(Halt::Error(format!(
                            "E-mailsjabloon {sjabloon} is niet voor offertes te kiezen; zet het aan voor offertes met emailsjabloon_wijzigen of kies een ander"
                        )));
                    }
                    Some(this.fetch(&run, &format!("offerte/getmaildata/{}/{sjabloon}", args.id), Vec::new()).await?)
                }
                None => None,
            };
            let user = this.fetch(&run, "user", Vec::new()).await?;
            let mail = offerte_mail(&args, template.as_ref(), &relatie, &user)?;
            let pdf = args.pdf.unwrap_or(true);
            let built = offerte_mail_built(&nummer, &mail, pdf);
            this.approve(&run, TOOL, &built, true).await?;
            let sent = this
                .change(&run, "POST", &format!("offerte/verstuur/{}/{pdf}", args.id), Some(&built.body))
                .await?;
            if let Some(why) = mail_failed(&sent) {
                return Err(Halt::Error(format!("e-Boekhouden kon de offerte niet versturen: {why}")));
            }
            Ok(json!({"verstuurd_aan": mail.aan, "offerte": nummer}))
        })
        .await
    }

    #[tool(
        description = "Een verkoopfactuur maken van een offerte, zoals Factuur maken op de offerte in e-Boekhouden: van alle regels, van een paar regels (regel_ids uit de tool offerte), van een percentage van het offertebedrag of van een bepaald bedrag. De factuur krijgt het offertenummer als kenmerk en hoort daarna bij de offerte. Zonder nummer krijgt de factuur het volgende nummer van e-Boekhouden; standaard wordt hij meteen geboekt. De factuur wordt niet verstuurd. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de factuur eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Offerte factureren",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn offerte_factureren(
        &self,
        Parameters(args): Parameters<OfferteFacturerenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "offerte_factureren";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Factuur gemaakt van de offerte; data.id is het id en data.nummer het factuurnummer",
            move |this, run, args| async move {
                let deel = args.deel()?;
                let offerte = this
                    .fetch(&run, &format!("offerte/{}", args.id), Vec::new())
                    .await?;
                let regels = this
                    .fetch(
                        &run,
                        &format!("offerteregel/getbyofferteid/{}", args.id),
                        Vec::new(),
                    )
                    .await?;
                let berekend = match &deel {
                    Deel::Regels(_) => None,
                    Deel::Percentage(t) | Deel::Bedrag(t) => {
                        let per_regel = this
                            .fetch(
                                &run,
                                "Administratie/isBtwBerekeningAdministration",
                                Vec::new(),
                            )
                            .await?;
                        if per_regel.as_bool() == Some(true) {
                            let how = if matches!(deel, Deel::Percentage(_)) {
                                "percentageBedrag"
                            } else {
                                "herberekenBedrag"
                            };
                            Some(
                                this.fetch(
                                    &run,
                                    &format!("offerteregel/{how}/{}/{}", args.id, js_number(*t)),
                                    Vec::new(),
                                )
                                .await?,
                            )
                        } else {
                            None
                        }
                    }
                };
                let relatie_id = id_of(&offerte, "relatieId").unwrap_or(0);
                let relatie = this
                    .fetch(&run, &format!("relatie/{relatie_id}"), Vec::new())
                    .await?;
                let books = this.books(&run).await?;
                let factuur_context = this.factuur_context(&run).await?;
                let bron = OfferteBron {
                    offerte: &offerte,
                    regels: &regels,
                    berekend: berekend.as_ref(),
                    relatie: &relatie,
                };
                let built = offerte_factuur(&args, &deel, &bron, &factuur_context, &books)?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change(&run, "POST", "factuur/model", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "De voortgang van een offerte, op offerte_id: de status (met alle statussen om uit te kiezen; een andere status zet je met offerte_wijzigen en status_id), de voortgangsregels (datum, medewerker, soort, onderwerp) en de soorten voortgang die de administratie kent, voor offerte_voortgang_toevoegen.",
        annotations(title = "Offerte voortgang", read_only_hint = true)
    )]
    async fn offerte_voortgang(
        &self,
        Parameters(args): Parameters<OfferteIdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let run = match self.begin(
            &context,
            "offerte_voortgang",
            &args,
            args.account.as_deref(),
        ) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let path = format!("offertevoortgang/gridtable/{}", args.offerte_id);
        let result = async {
            let offerte = self.fetch(&run, &format!("offerte/{}", args.offerte_id), Vec::new()).await?;
            let statussen = rows(&named_rows(self.fetch(&run, "offertestatus/gridtable", grid(Some(MAX_ROWS), None)?).await?));
            let voortgang = named_rows(self.fetch(&run, &path, grid(Some(MAX_ROWS), None)?).await?);
            let soorten = self.fetch(&run, "voortgangsoort/selectlist", Vec::new()).await?;
            let status_id = id_of(&offerte, "statusId");
            let status = statussen
                .iter()
                .find(|r| status_id.is_some() && id_of(r, "id") == status_id)
                .map(|r| s(r, "status"));
            Ok(json!({
                "offerte": {"id": args.offerte_id, "nummer": s(&offerte, "nummer"), "status_id": status_id, "status": status},
                "voortgang": voortgang.get("data").cloned().unwrap_or(Value::Null),
                "soorten": rows(&soorten).iter().map(|r| json!({"id": id_of(r, "id"), "naam": s(r, "caption")})).collect::<Vec<_>>(),
                "statussen": statussen,
            }))
        }
        .await;
        Ok(self.end(run, &path, result))
    }

    #[tool(
        description = "Een voortgangsregel aan een offerte toevoegen, zoals het tabblad Voortgang in e-Boekhouden: een soort (bijvoorbeeld Telefoon, Akkoord of Afgewezen, zoals offerte_voortgang ze noemt), een onderwerp en eventueel een bericht. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt het eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Offerte voortgang toevoegen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn offerte_voortgang_toevoegen(
        &self,
        Parameters(args): Parameters<OfferteVoortgangToevoegenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "offerte_voortgang_toevoegen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Voortgang toegevoegd; data.id is het id",
            move |this, run, args| async move {
                let nummer = this.offerte_nummer(&run, args.offerte_id).await?;
                let soorten = this
                    .fetch(&run, "voortgangsoort/selectlist", Vec::new())
                    .await?;
                let built = voortgang_nieuw(&args, &nummer, &soorten)?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change(&run, "POST", "offertevoortgang", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een voortgangsregel van een offerte wijzigen, op offerte_id en id (uit offerte_voortgang). Wat je niet noemt blijft zoals het is. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de wijziging eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Offerte voortgang wijzigen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn offerte_voortgang_wijzigen(
        &self,
        Parameters(args): Parameters<OfferteVoortgangWijzigenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "offerte_voortgang_wijzigen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Voortgang gewijzigd",
            move |this, run, args| async move {
                let nummer = this.offerte_nummer(&run, args.offerte_id).await?;
                this.offerte_entry(&run, "offertevoortgang", args.offerte_id, args.id)
                    .await?;
                let now = this
                    .fetch(
                        &run,
                        &format!("offertevoortgang/getordefault/{}", args.id),
                        Vec::new(),
                    )
                    .await?;
                let soorten = this
                    .fetch(&run, "voortgangsoort/selectlist", Vec::new())
                    .await?;
                let built = voortgang_gewijzigd(&args, &now, &nummer, &soorten)?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change_done(&run, "PUT", "offertevoortgang", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een voortgangsregel van een offerte verwijderen, op offerte_id en id (uit offerte_voortgang). Dit is niet terug te draaien. De gebruiker moet dit altijd goedkeuren in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Offerte voortgang verwijderen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn offerte_voortgang_verwijderen(
        &self,
        Parameters(args): Parameters<OfferteItemArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "offerte_voortgang_verwijderen";
        let account = args.account.clone();
        self.write_tool(&context, TOOL, account.as_deref(), args, "Voortgang verwijderd", move |this, run, args| async move {
            let nummer = this.offerte_nummer(&run, args.offerte_id).await?;
            this.offerte_entry(&run, "offertevoortgang", args.offerte_id, args.id).await?;
            let now = this.fetch(&run, &format!("offertevoortgang/getordefault/{}", args.id), Vec::new()).await?;
            let built = Built {
                body: Value::Null,
                title: format!("Voortgang van offerte {nummer} verwijderen"),
                lines: vec![
                    line("Offerte", &nummer),
                    line("Voortgang", known(s(&now, "onderwerp"))),
                    line("Gevolg", "de regel verdwijnt uit de voortgang van de offerte; niet terug te draaien"),
                ],
            };
            this.approve(&run, TOOL, &built, true).await?;
            this.change_done(&run, "DELETE", &format!("offertevoortgang/{}", args.id), None).await
        })
        .await
    }

    #[tool(
        description = "De agenda van een offerte, op offerte_id: de afspraken en vervolgacties (datum, onderwerp, medewerker, afgehandeld op), de eerstvolgende actie, en de medewerkers om uit te kiezen voor offerte_agenda_toevoegen.",
        annotations(title = "Offerte agenda", read_only_hint = true)
    )]
    async fn offerte_agenda(
        &self,
        Parameters(args): Parameters<OfferteIdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let run = match self.begin(&context, "offerte_agenda", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let path = format!("offerteagenda/gridtable/{}", args.offerte_id);
        let result = async {
            let nummer = self.offerte_nummer(&run, args.offerte_id).await?;
            let agenda = named_rows(self.fetch(&run, &path, grid(Some(MAX_ROWS), None)?).await?);
            let volgende = self.fetch(&run, &format!("offerteagenda/volgendeactie/{}", args.offerte_id), Vec::new()).await.unwrap_or(Value::Null);
            let users = self.fetch(&run, "user/selectlist", Vec::new()).await?;
            Ok(json!({
                "offerte": {"id": args.offerte_id, "nummer": nummer},
                "agenda": agenda.get("data").cloned().unwrap_or(Value::Null),
                "volgende_actie": volgende,
                "medewerkers": rows(&users).iter().map(|u| json!({"id": id_of(u, "id"), "naam": s(u, "naam")})).collect::<Vec<_>>(),
            }))
        }
        .await;
        Ok(self.end(run, &path, result))
    }

    #[tool(
        description = "Een afspraak of vervolgactie in de agenda van een offerte zetten, zoals het tabblad Agenda in e-Boekhouden: datum, tijd, onderwerp, eventueel een bericht en een medewerker (standaard de ingelogde gebruiker). De eerstvolgende open afspraak is de volgende actie van de offerte. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt het eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Offerte agenda toevoegen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn offerte_agenda_toevoegen(
        &self,
        Parameters(args): Parameters<OfferteAgendaToevoegenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "offerte_agenda_toevoegen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Afspraak toegevoegd; data.id is het id",
            move |this, run, args| async move {
                let nummer = this.offerte_nummer(&run, args.offerte_id).await?;
                let users = this.fetch(&run, "user/selectlist", Vec::new()).await?;
                // The signed-in user is only the default; `user` need not carry an id (the capture has none).
                let ingelogd = match args.medewerker_id {
                    Some(_) => None,
                    None => id_of(&this.fetch(&run, "user", Vec::new()).await?, "id"),
                };
                let built = agenda_nieuw(&args, &nummer, &users, ingelogd)?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change(&run, "POST", "offerteagenda", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een afspraak in de agenda van een offerte wijzigen, op offerte_id en id (uit offerte_agenda): datum, tijd, onderwerp, bericht, medewerker of afgehandeld (een lege tekst maakt hem weer open). Wat je niet noemt blijft zoals het is. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de wijziging eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Offerte agenda wijzigen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn offerte_agenda_wijzigen(
        &self,
        Parameters(args): Parameters<OfferteAgendaWijzigenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "offerte_agenda_wijzigen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Afspraak gewijzigd",
            move |this, run, args| async move {
                let nummer = this.offerte_nummer(&run, args.offerte_id).await?;
                this.offerte_entry(&run, "offerteagenda", args.offerte_id, args.id)
                    .await?;
                let now = this
                    .fetch(
                        &run,
                        &format!("offerteagenda/getordefault/{}", args.id),
                        Vec::new(),
                    )
                    .await?;
                let users = this.fetch(&run, "user/selectlist", Vec::new()).await?;
                let built = agenda_gewijzigd(&args, &now, &nummer, &users)?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change_done(&run, "PUT", "offerteagenda", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een afspraak uit de agenda van een offerte verwijderen, op offerte_id en id (uit offerte_agenda). Dit is niet terug te draaien. De gebruiker moet dit altijd goedkeuren in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Offerte agenda verwijderen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn offerte_agenda_verwijderen(
        &self,
        Parameters(args): Parameters<OfferteItemArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "offerte_agenda_verwijderen";
        let account = args.account.clone();
        self.write_tool(&context, TOOL, account.as_deref(), args, "Afspraak verwijderd", move |this, run, args| async move {
            let nummer = this.offerte_nummer(&run, args.offerte_id).await?;
            this.offerte_entry(&run, "offerteagenda", args.offerte_id, args.id).await?;
            let now = this.fetch(&run, &format!("offerteagenda/getordefault/{}", args.id), Vec::new()).await?;
            let built = Built {
                body: Value::Null,
                title: format!("Afspraak bij offerte {nummer} verwijderen"),
                lines: vec![
                    line("Offerte", &nummer),
                    line("Afspraak", known(s(&now, "onderwerp"))),
                    line("Op", known(s(&now, "datum").replace('T', " ").chars().take(16).collect())),
                    line("Gevolg", "de afspraak verdwijnt uit de agenda van de offerte; niet terug te draaien"),
                ],
            };
            this.approve(&run, TOOL, &built, true).await?;
            this.change_done(&run, "DELETE", &format!("offerteagenda/{}", args.id), None).await
        })
        .await
    }

    #[tool(
        description = "Het SEPA-incassobestand maken voor alle facturen die in e-Boekhouden klaarstaan voor automatische incasso, zoals Facturen > Incassobestand. Gebruikt de IBAN en het incassocontract-ID die in e-Boekhouden zijn ingesteld (EB MCP wijzigt die niet). Het bestand wordt op deze computer opgeslagen, standaard in de map Downloads; het geld wordt pas geïncasseerd als iemand het bestand bij de bank aanbiedt. De facturen staan daarna niet meer klaar voor incasso. Dit is een betaalopdracht: de gebruiker moet dit altijd goedkeuren in EB MCP en ziet daar elke factuur en het totaal. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Incassobestand maken",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn incassobestand_maken(
        &self,
        Parameters(args): Parameters<IncassobestandArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "incassobestand_maken";
        let account = args.account.clone();
        self.write_tool(&context, TOOL, account.as_deref(), args, "Incassobestand gemaakt; data.pad is waar het staat", move |this, run, args| async move {
            // The folder is checked before anything is asked, so a wrong one never costs a made file.
            let dir = download_dir(args.map.as_deref())?;
            let vandaag = today();
            let uitvoer = match args.uitvoerdatum.as_deref() {
                Some(datum) => parse_date(datum)?,
                None => next_day(vandaag),
            };
            let admin = this.fetch(&run, "administratie/incassoinfo", Vec::new()).await?;
            let info = this.fetch(&run, "factuur/incassoinfo", Vec::new()).await?;
            let facturen = named_rows(this.fetch(&run, "factuur/machtiginggrid", grid(Some(MAX_ROWS), None)?).await?);
            let (built, query, lijst) = incasso_plan(uitvoer, vandaag, &admin, &info, &facturen)?;
            this.approve(&run, TOOL, &built, true).await?;
            // The file takes every invoice ready at the moment it is made, and the approval may have waited for
            // hours: the list must still be the one the user saw.
            let info = this.fetch(&run, "factuur/incassoinfo", Vec::new()).await?;
            let facturen = named_rows(this.fetch(&run, "factuur/machtiginggrid", grid(Some(MAX_ROWS), None)?).await?);
            incasso_zelfde(&lijst, &info, &facturen)?;
            let made = match this.change_query(&run, "GET", "incassobestand/create", &query, None).await {
                Ok(made) => made,
                Err(Halt::Error(e) | Halt::Refused(e)) if certainly_not_done(&e) => {
                    return Err(Halt::Error(format!("e-Boekhouden heeft het incassobestand niet gemaakt: {e}")));
                }
                Err(Halt::Error(e) | Halt::Refused(e)) => {
                    return Err(Halt::Error(format!(
                        "Of het incassobestand is gemaakt, is niet zeker ({e}). Kijk in e-Boekhouden onder Facturen > Incassobestand > Oude incassobestanden voordat je het opnieuw probeert."
                    )));
                }
            };
            let name = made.as_str().unwrap_or_default().to_owned();
            // From here the file exists at e-Boekhouden: whatever happens with the download, the change is done.
            let manual = "Download het in e-Boekhouden onder Facturen > Incassobestand > Oude incassobestanden.";
            let Some(safe) = safe_file_name(&name) else {
                return Ok(json!({"gemaakt": true, "bestand": name, "let_op": format!("e-Boekhouden maakte het bestand, maar gaf er geen bruikbare naam bij. {manual}")}));
            };
            let (call, file) = this
                .state
                .api_download(run.account.id, &format!("incassobestand/download/{safe}"), Some(run.id), &run.client)
                .await;
            match file {
                Some((bytes, _)) if call.ok() => match write_new(&dir, safe, &bytes) {
                    Ok(target) => Ok(json!({"gemaakt": true, "bestand": safe, "pad": target.to_string_lossy(), "bytes": bytes.len()})),
                    Err(e) => Ok(json!({"gemaakt": true, "bestand": safe, "let_op": format!("Het bestand is niet op te slaan in {}: {e}. {manual}", dir.display())})),
                },
                _ => Ok(json!({"gemaakt": true, "bestand": safe, "let_op": format!(
                    "e-Boekhouden maakte het bestand, maar EB MCP kon het niet ophalen ({}). {manual}",
                    call.error.unwrap_or_else(|| format!("status {}", call.status))
                )})),
            }
        })
        .await
    }

    #[tool(
        description = "Eén verkoopfactuur via Peppol versturen, op id (uit facturen), zoals de P in het factuuroverzicht van e-Boekhouden. Kan alleen als de relatie een Peppol-ID heeft, de administratie op Peppol is aangesloten en de factuur nog niet eerder via Peppol ging: een factuur kan maar één keer via Peppol. De factuur verlaat e-Boekhouden; dit is niet terug te draaien. De gebruiker moet dit altijd goedkeuren in EB MCP en ziet daar de factuur en de ontvanger. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Factuur via Peppol",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn peppol_versturen(
        &self,
        Parameters(args): Parameters<PeppolVersturenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "peppol_versturen";
        let account = args.account.clone();
        self.write_tool(&context, TOOL, account.as_deref(), args, "Factuur verstuurd via Peppol", move |this, run, args| async move {
            // As the web app: only an administration on Peppol looks at Peppol sendings (`peppol/getpeppolaccount`).
            // It answers the account, or nothing (null or an empty body) when there is none.
            let call = this.state.api_get(run.account.id, "peppol/getpeppolaccount", &[], Some(run.id), &run.client).await;
            if !call.ok() {
                return Err(Halt::Error(call.error.unwrap_or_else(|| format!("e-Boekhouden gaf {} op peppol/getpeppolaccount", call.status))));
            }
            if !call.json.as_ref().is_some_and(Value::is_object) {
                return Err(Halt::Error(
                    "Deze administratie is niet op Peppol aangesloten; koppel eerst een Peppol-account in e-Boekhouden onder Beheer > Koppelingen".into(),
                ));
            }
            let factuur = this.fetch(&run, &format!("factuur/{}", args.id), Vec::new()).await?;
            let relatie_id = id_of(&factuur, "relatieId").unwrap_or(0);
            let relatie = this.fetch(&run, &format!("relatie/{relatie_id}"), Vec::new()).await?;
            let verstuurd = this.fetch(&run, &format!("factuur/haspeppolverzending/{}", args.id), Vec::new()).await?;
            let built = peppol_plan(args.id, &factuur, &relatie, &verstuurd)?;
            this.approve(&run, TOOL, &built, true).await?;
            // An invoice goes over Peppol once: it may have gone while the approval waited.
            let verstuurd = this.fetch(&run, &format!("factuur/haspeppolverzending/{}", args.id), Vec::new()).await?;
            if verstuurd.as_bool() != Some(false) {
                return Err(Halt::Error(format!(
                    "Factuur {} is niet verstuurd: hij is intussen al via Peppol verstuurd, of e-Boekhouden zegt dat niet duidelijk ({verstuurd}).",
                    s(&factuur, "nummer")
                )));
            }
            this.change(&run, "POST", "peppol/send", Some(&built.body)).await?;
            Ok(json!({"factuur": s(&factuur, "nummer"), "verstuurd_via": "Peppol", "aan": s(&relatie, "bedrijf")}))
        })
        .await
    }

    #[tool(
        description = "Meerdere verkoopfacturen per e-mail versturen (hoogstens 50), elk naar het factuur- of e-mailadres van zijn eigen relatie en met de eigen factuur als pdf in de bijlage. Onderwerp en bericht komen uit een e-mailsjabloon voor facturen (sjabloon_id), per factuur zoals e-Boekhouden het invult, of geef je zelf mee; eigen tekst gaat zoals hij is naar elke ontvanger, dus zonder codes zoals [FACTNUMMER]. Dit verstuurt echte e-mail en is niet terug te draaien. De gebruiker moet dit altijd goedkeuren in EB MCP en ziet daar elke factuur met zijn ontvanger, onderwerp en bericht. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Facturen mailen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn facturen_mailen(
        &self,
        Parameters(args): Parameters<FacturenMailenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "facturen_mailen";
        let account = args.account.clone();
        self.write_tool(&context, TOOL, account.as_deref(), args, "Facturen verstuurd per e-mail", move |this, run, args| async move {
            check_ids(&args.ids)?;
            own_text_without_codes(given(&args.onderwerp).as_deref(), given(&args.bericht).as_deref())?;
            if let Some(id) = args.sjabloon_id {
                for_invoices(&this.fetch(&run, "emailsjabloonab/forfactuur", Vec::new()).await?, id)?;
            }
            let user = this.fetch(&run, "user", Vec::new()).await?;
            let eigen = EigenMail { onderwerp: &args.onderwerp, bericht: &args.bericht, cc: &args.cc, bcc: &args.bcc };
            let mut relaties: std::collections::BTreeMap<u64, Value> = std::collections::BTreeMap::new();
            let mut ontvangers = Vec::new();
            let mut zonder = Vec::new();
            for id in &args.ids {
                let factuur = this.fetch(&run, &format!("factuur/{id}"), Vec::new()).await?;
                let nummer = s(&factuur, "nummer");
                let relatie_id = id_of(&factuur, "relatieId").unwrap_or(0);
                if let std::collections::btree_map::Entry::Vacant(slot) = relaties.entry(relatie_id) {
                    slot.insert(this.fetch(&run, &format!("relatie/{relatie_id}"), Vec::new()).await?);
                }
                let relatie = &relaties[&relatie_id];
                let Some(aan) = factuur_adres(relatie) else {
                    zonder.push(format!("{nummer} ({})", known(s(relatie, "bedrijf"))));
                    continue;
                };
                // The template's text as e-Boekhouden fills it for this invoice: what is shown is what is sent.
                let filled = match args.sjabloon_id {
                    Some(sjabloon) => Some(this.fetch(&run, &format!("factuur/maildata/{sjabloon}/{id}"), Vec::new()).await?),
                    None => None,
                };
                let mail = factuur_mail(&eigen, filled.as_ref(), &aan, &user, run.account.administration.as_deref())
                    .map_err(|e| Halt::Error(format!("Factuur {nummer}: {e}")))?;
                ontvangers.push(Ontvanger { factuur_id: *id, nummer, relatie: known(s(relatie, "bedrijf")), mail });
            }
            if !zonder.is_empty() {
                return Err(Halt::Error(format!(
                    "De relatie van deze facturen heeft geen e-mailadres: {}. Laat ze weg, of mail ze los met factuur_mailen en een ontvanger.",
                    zonder.join(", ")
                )));
            }
            let built = facturen_mail_built(&ontvangers, args.ubl.unwrap_or(false));
            this.approve(&run, TOOL, &built, true).await?;
            for (at, mail) in rows(&built.body["mails"]).iter().enumerate() {
                let stopped = match this.change(&run, "POST", "factuur/mail", Some(mail)).await {
                    // e-Boekhouden answered and said it did not send: certain.
                    Ok(sent) => mail_failed(&sent).map(|why| (why, true)),
                    // A timeout or lost connection can come after the mail went out.
                    Err(Halt::Error(e) | Halt::Refused(e)) => {
                        let certain = certainly_not_done(&e);
                        Some((e, certain))
                    }
                };
                if let Some((why, certain)) = stopped {
                    return Err(Halt::Error(bulk_mail_stopped(&ontvangers, at, &why, certain)));
                }
            }
            Ok(json!({"verstuurd": ontvangers.iter().map(|o| json!({"factuur": o.nummer, "aan": o.mail.aan})).collect::<Vec<_>>()}))
        })
        .await
    }

    #[tool(
        description = "Meerdere verkoopfacturen als pdf ophalen om af te drukken (hoogstens 50), op id (uit facturen). Elke pdf wordt op deze computer opgeslagen, standaard in de map Downloads; het antwoord geeft per factuur het volledige pad, zodat de gebruiker ze kan openen en afdrukken. Bestaande bestanden worden niet overschreven. Leest alleen: er wordt niets verstuurd of gewijzigd, al houdt e-Boekhouden mogelijk bij hoe vaak een factuur is afgedrukt (net als bij factuur_pdf). Wil je één pdf zelf lezen, gebruik dan factuur_pdf.",
        annotations(title = "Facturen afdrukken", read_only_hint = true)
    )]
    async fn facturen_afdrukken(
        &self,
        Parameters(args): Parameters<FacturenAfdrukkenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let run = match self.begin(
            &context,
            "facturen_afdrukken",
            &args,
            args.account.as_deref(),
        ) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        if let Err(message) =
            check_ids(&args.ids).and_then(|_| download_dir(args.map.as_deref()).map(|_| ()))
        {
            return Ok(self.refuse(run, message));
        }
        let mut opgeslagen = Vec::new();
        let mut fouten = Vec::new();
        for id in &args.ids {
            match self
                .save_pdf(
                    &run,
                    &format!("factuur/{id}/print"),
                    args.map.as_deref(),
                    &format!("factuur-{id}.pdf"),
                )
                .await
            {
                Ok((saved, _)) => opgeslagen.push(
                    json!({"id": id, "pad": saved["pad"], "bestandsnaam": saved["bestandsnaam"]}),
                ),
                Err(e) => fouten.push(json!({"id": id, "fout": e})),
            }
        }
        let result = if opgeslagen.is_empty() {
            Err(format!(
                "Geen enkele pdf is opgehaald: {}",
                fouten
                    .iter()
                    .map(|f| format!("{} ({})", f["id"], f["fout"].as_str().unwrap_or_default()))
                    .collect::<Vec<_>>()
                    .join("; ")
            ))
        } else {
            Ok(json!({"opgeslagen": opgeslagen, "fouten": fouten}))
        };
        Ok(self.end(run, "factuur/{id}/print", result))
    }

    #[tool(
        description = "Een bestand in het digitaal archief loskoppelen: van één mutatie (mutatie_nr) of van alles waar het aan hangt (alles: true, ook bankregels, relaties en offertes). Het bestand zelf blijft; e-Boekhouden verwijdert een gekoppeld bestand niet, dus koppel het eerst los als het weg moet (archief_verwijderen). Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt het eerst goed in EB MCP, en altijd als er een koppeling met een bankregel, relatie of offerte verdwijnt (die kan EB MCP niet terugzetten). Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Bestand ontkoppelen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true
        )
    )]
    async fn archief_ontkoppelen(
        &self,
        Parameters(args): Parameters<ArchiefOntkoppelenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "archief_ontkoppelen";
        let account = args.account.clone();
        self.write_tool(&context, TOOL, account.as_deref(), args, "Bestand ontkoppeld; het staat nog in het archief", move |this, run, args| async move {
            let file = this.fetch(&run, &format!("folder/{}", args.bestand_id), Vec::new()).await?;
            let naam = filled(&file, "naam").unwrap_or_else(|| "bestand".to_owned());
            if s(&file, "soort") != "F" {
                return Err(Halt::Error(format!("{naam} is een map, geen bestand")));
            }
            let links = this.fetch(&run, &format!("folder/{}/koppel", args.bestand_id), Vec::new()).await?;
            let (built, ids, danger) = ontkoppel_plan(&naam, &links, args.mutatie_nr, args.alles.unwrap_or(false))?;
            this.approve(&run, TOOL, &built, danger).await?;
            for (n, id) in ids.iter().enumerate() {
                if let Err(Halt::Error(e) | Halt::Refused(e)) = this.change_done(&run, "DELETE", &format!("folderkoppel/{id}"), None).await {
                    return Err(Halt::Error(if n == 0 {
                        e
                    } else {
                        format!("{n} van de {} koppelingen zijn losgemaakt, daarna ging het mis: {e}", ids.len())
                    }));
                }
            }
            Ok(json!({"bestand": naam, "losgemaakt": ids.len()}))
        })
        .await
    }

    #[tool(
        description = "De codes die je in een e-mailsjabloon voor facturen kunt gebruiken (onderwerp en bericht), met hun uitleg, zoals e-Boekhoudens pagina Sjablooncodes factuur mailen ze geeft: [RELBEDRIJF], [FACTNUMMER], [TOTAAL], [ABBEDRIJF] en de rest. e-Boekhouden vult ze in wanneer een factuur met dat sjabloon wordt gemaild (factuur_mailen of facturen_mailen met sjabloon_id); in eigen tekst werken ze niet. De codes van de opmaak van een factuur- of offertesjabloon geeft de tool sjabloon.",
        annotations(title = "E-mailsjabloon codes", read_only_hint = true)
    )]
    async fn emailsjabloon_codes(
        &self,
        Parameters(args): Parameters<AccountArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let run = match self.begin(
            &context,
            "emailsjabloon_codes",
            &args,
            args.account.as_deref(),
        ) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let codes: Vec<Value> = EMAIL_CODES
            .iter()
            .map(|(code, uitleg)| json!({"code": code, "uitleg": uitleg}))
            .collect();
        Ok(self.end(
            run,
            "e-Boekhouden, Sjablooncodes factuur mailen",
            Ok(json!({"codes": codes, "let_op": "Typ een code precies zo, met de blokhaken. [ABKVKNUMMER] werkt in Nederlandse administraties, [ABONDERNEMINGSNR] in Belgische."})),
        ))
    }
}
