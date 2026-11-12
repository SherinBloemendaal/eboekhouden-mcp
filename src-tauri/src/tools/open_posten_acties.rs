//! Open posten: reminders and dunning, postponing, writing off; and berichten and controleopdrachten.
//!
//! The tools of this part live here, on their own router (`open_posten_acties_router`), which tools.rs adds to the
//! server's. Each tool is named in one of the lists below, which the tool-list test in mcp.rs reads.
//!
//! Every request shape comes from e-Boekhouden's own web app (the public bundle): the open posten list and its
//! aanmaning and afboek forms, the berichten pages and the controle-opdracht pages. Postponing an open post
//! ("Uitstellen") is not built: the web app opens an old ASP page for it (`bh/bh_openposten.asp?ACTION=UITSTELLEN`)
//! instead of calling the API, so there is no request shape to follow.

use super::*;

mod built;
#[cfg(test)]
mod tests;

use built::{
    Aanmaansjabloon, BerichtActie, Kant, Ontvanger, Post, Tegenrekening, aanmaning, accountant,
    afboeken, afgeboekt, antwoord_onderwerp, berichten_actie, check_wanted, controle_aanvraag,
    controle_beoordeling, field, find_posten, is_accountant, moment, pick_tegenrekening,
    relatie_ids, rows, text, verzonden,
};

/// The tools of this part that only read.
#[cfg(test)]
pub(super) const READS: &[&str] = &[
    "aanmaning_historie",
    "berichten",
    "bericht",
    "controleopdrachten",
    "controleopdracht",
];
/// The tools of this part that change the books.
#[cfg(test)]
pub(super) const WRITES: &[&str] = &[
    "aanmaning_versturen",
    "openpost_afboeken",
    "bericht_gelezen",
    "bericht_archiveren",
    "bericht_versturen",
    "controleopdracht_aanvragen",
    "controleopdracht_beoordelen",
];
/// The ones in `WRITES` that cannot be undone.
#[cfg(test)]
pub(super) const DESTRUCTIVE: &[&str] = &[
    "aanmaning_versturen",
    "openpost_afboeken",
    "bericht_versturen",
    "controleopdracht_aanvragen",
    "controleopdracht_beoordelen",
];

// ---------------------------------------------------------------- parameters

/// Een open post, zoals open_posten hem geeft.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct OpenPostRef {
    /// Factuurnummer van de open post (mutFactuur in open_posten).
    pub factuur: String,
    /// Id van de relatie van de open post (relatieId in open_posten).
    pub relatie_id: u64,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AanmaningVersturenArgs {
    /// De open posten van debiteuren waarvoor een herinnering of aanmaning gaat, hoogstens 1000. Per relatie gaat er één e-mail.
    pub posten: Vec<OpenPostRef>,
    /// Id van het aanmaansjabloon per e-mail (uit sjablonen met soort aanmaan). Standaard het eerste, zoals e-Boekhouden het voorstelt.
    pub sjabloon_id: Option<u64>,
    /// De facturen als pdf meesturen (standaard niet).
    pub facturen_als_bijlage: Option<bool>,
    /// Een e-mailadres dat een kopie van elke herinnering krijgt.
    pub kopie_naar: Option<String>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AanmaningHistorieArgs {
    /// Factuurnummer van de open post.
    pub factuur: String,
    /// Id van de relatie van de open post.
    pub relatie_id: u64,
    /// Aantal rijen, 1 tot 200 (standaard 50).
    pub limit: Option<u32>,
    /// Vanaf welke rij (standaard 0).
    pub offset: Option<u32>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct OpenpostAfboekenArgs {
    /// debiteuren (te ontvangen, standaard) of crediteuren (te betalen).
    pub soort: Option<Soort>,
    /// De open posten om af te boeken, hoogstens 1000.
    pub posten: Vec<OpenPostRef>,
    /// Code van de grootboekrekening waar het bedrag heen gaat. Standaard de rekening Betalingsverschillen, zoals e-Boekhouden voorstelt.
    pub tegenrekening: Option<String>,
    /// Het bedrag om af te boeken, alleen bij één open post. Standaard het hele openstaande bedrag.
    pub bedrag: Option<f64>,
    /// Boekdatum, JJJJ-MM-DD (standaard vandaag).
    pub datum: Option<String>,
    /// Omschrijving van de boeking, hoogstens 50 tekens (standaard Betalingsverschil).
    pub omschrijving: Option<String>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// De drie mappen van berichten in e-Boekhouden.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum BerichtenMap {
    /// Ontvangen berichten.
    Postvak,
    /// Verstuurde berichten.
    Verzonden,
    /// Gearchiveerde berichten.
    Archief,
}

impl BerichtenMap {
    /// The grid's `type`, as the berichten page sends it.
    fn type_value(self) -> u8 {
        match self {
            Self::Postvak => 1,
            Self::Verzonden => 2,
            Self::Archief => 3,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct BerichtenArgs {
    /// postvak (standaard), verzonden of archief.
    pub map: Option<BerichtenMap>,
    /// Aantal rijen, 1 tot 200 (standaard 50).
    pub limit: Option<u32>,
    /// Vanaf welke rij (standaard 0).
    pub offset: Option<u32>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct BerichtIdsArgs {
    /// De ids van berichten in het postvak (uit berichten), hoogstens 200.
    pub ids: Vec<u64>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct BerichtVersturenArgs {
    /// Onderwerp, hoogstens 100 tekens. Bij een antwoord standaard "Re: " met het onderwerp van het bericht.
    pub onderwerp: Option<String>,
    /// De tekst van het bericht, hoogstens 40000 tekens.
    pub bericht: String,
    /// Id van het bericht waar dit een antwoord op is (uit berichten).
    pub antwoord_op: Option<u64>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ControleopdrachtenArgs {
    /// true voor de lijst van een accountant: de opdrachten van zijn klanten. Standaard de opdrachten van deze administratie.
    pub als_accountant: Option<bool>,
    /// Aantal rijen, 1 tot 200 (standaard 50).
    pub limit: Option<u32>,
    /// Vanaf welke rij (standaard 0).
    pub offset: Option<u32>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// Welke periode de accountant controleert.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ControlePeriode {
    Maand,
    Kwartaal,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ControleopdrachtAanvragenArgs {
    /// maand of kwartaal.
    pub periode: ControlePeriode,
    /// Het jaar, van 8 jaar terug tot 3 jaar vooruit.
    pub jaar: i32,
    /// De maand, 1 tot en met 12 (bij periode maand; standaard deze maand).
    pub maand: Option<u32>,
    /// Het kwartaal, 1 tot en met 4 (bij periode kwartaal; standaard dit kwartaal).
    pub kwartaal: Option<u32>,
    /// Een opmerking voor de accountant, hoogstens 8000 tekens.
    pub opmerking: Option<String>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ControleopdrachtBeoordelenArgs {
    /// Id van de controle-opdracht (uit controleopdrachten met als_accountant).
    pub id: u64,
    /// true om de periode goed te keuren, false om hem af te keuren.
    pub goedkeuren: bool,
    /// Een opmerking voor de klant, hoogstens 8000 tekens.
    pub opmerking: Option<String>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// The refusals for an accountant's login: e-Boekhouden's own pages work differently for an accountant.
const ACCOUNTANT_BERICHT: &str = "Dit account is van een accountant. Een accountant kiest in e-Boekhouden bij elk bericht de administratie waar het heen gaat, en dat kan EB MCP niet; verstuur het bericht in e-Boekhouden zelf.";
const ACCOUNTANT_CONTROLE: &str = "Dit account is van een accountant. Een controle-opdracht vraagt een administratie aan haar accountant; e-Boekhouden biedt dat een accountant niet aan. Beoordeel opdrachten van klanten met controleopdracht_beoordelen.";
const GEEN_ACCOUNTANT: &str = "Dit account is niet van een accountant. Alleen een accountant beoordeelt controle-opdrachten; e-Boekhouden toont dat scherm alleen aan accountants.";

/// A grid query with room for every row e-Boekhouden gives in one page.
fn page(offset: u32) -> Vec<(String, String)> {
    vec![
        ("offset".into(), offset.to_string()),
        ("limit".into(), MAX_ROWS.to_string()),
        ("orderBy".into(), String::new()),
        ("sortDirection".into(), String::new()),
    ]
}

/// How many pages of a list a tool reads to find what it was asked for.
const MAX_PAGES: u32 = 25;

impl EbMcp {
    /// Every row of a grid, page after page, up to `MAX_PAGES` pages.
    async fn opa_all_rows(
        &self,
        run: &Run,
        path: &str,
        filters: &[(&str, String)],
    ) -> Result<Vec<Value>, String> {
        let mut all = Vec::new();
        for n in 0..MAX_PAGES {
            let mut query = page(n * MAX_ROWS);
            query.extend(filters.iter().map(|(k, v)| ((*k).to_owned(), v.clone())));
            let found = rows(&named_rows(self.fetch(run, path, query).await?));
            let last = found.len() < MAX_ROWS as usize;
            all.extend(found);
            if last {
                break;
            }
        }
        Ok(all)
    }

    /// The open posten asked for, each found in the open posten of its relatie (`openpost/gridtable/openpost`).
    async fn opa_open_posten(
        &self,
        run: &Run,
        kant: Kant,
        wanted: &[OpenPostRef],
    ) -> Result<Vec<Post>, String> {
        let wanted: Vec<(String, u64)> = wanted
            .iter()
            .map(|p| (p.factuur.clone(), p.relatie_id))
            .collect();
        check_wanted(&wanted)?;
        let relaties = relatie_ids(wanted.iter().map(|(_, id)| *id));
        let mut open = Vec::with_capacity(relaties.len());
        for relatie_id in relaties {
            let rows = self
                .opa_all_rows(
                    run,
                    "openpost/gridtable/openpost",
                    &[
                        ("soort", kant.soort().to_owned()),
                        ("zoeken", "true".to_owned()),
                        ("relatieId", relatie_id.to_string()),
                    ],
                )
                .await?;
            open.push((
                relatie_id,
                rows.iter().filter_map(Post::from_row).collect::<Vec<_>>(),
            ));
        }
        find_posten(&wanted, &open)
    }

    /// Whether the signed-in user is an accountant, from `POST auth`: the check e-Boekhouden's own app makes at
    /// startup (`isAccountant`). It changes nothing.
    async fn opa_is_accountant(&self, run: &Run) -> Result<bool, String> {
        let call = self
            .state
            .api_send(
                run.account.id,
                "POST",
                "auth",
                &[],
                None,
                Some(run.id),
                &run.client,
            )
            .await;
        match (call.ok(), call.json) {
            (true, Some(auth)) => Ok(is_accountant(&auth)),
            _ => Err(call
                .error
                .unwrap_or_else(|| format!("e-Boekhouden gaf {} op auth", call.status))),
        }
    }

    /// The accountant a message or a check request goes to, or a refusal when the administration has none.
    async fn opa_accountant(&self, run: &Run) -> Result<String, String> {
        let info = self
            .fetch(run, "administratie/accountantinfo", Vec::new())
            .await?;
        accountant(&info).ok_or_else(|| {
            "Deze administratie is niet aan een accountant gekoppeld; berichten en controle-opdrachten gaan altijd naar de accountant.".to_owned()
        })
    }

    /// Marks messages in the inbox as read or moves them to the archive: e-Boekhouden's two bulk actions.
    async fn opa_berichten(
        &self,
        context: RequestContext<RoleServer>,
        tool: &'static str,
        args: BerichtIdsArgs,
        actie: BerichtActie,
        done: &'static str,
    ) -> Result<CallToolResult, McpError> {
        let account = args.account.clone();
        self.write_tool(
            &context,
            tool,
            account.as_deref(),
            args,
            done,
            move |this, run, args| async move {
                // The inbox is read from its list, which leaves the messages as they are.
                let inbox = this
                    .opa_all_rows(&run, "bericht/gridtable", &[("type", "1".to_owned())])
                    .await?;
                let built = berichten_actie(actie, &args.ids, &inbox)?;
                this.approve(&run, tool, &built, false).await?;
                this.change(&run, "POST", actie.path(), Some(&built.body))
                    .await?;
                Ok(json!({"berichten": args.ids}))
            },
        )
        .await
    }
}

#[tool_router(router = open_posten_acties_router, vis = "pub(super)")]
impl EbMcp {
    #[tool(
        description = "Een herinnering of aanmaning per e-mail versturen vanuit e-Boekhouden, voor één of meer open posten van debiteuren (uit open_posten), zoals Aanmanen per e-mail op de open posten in e-Boekhouden. De tekst komt uit een aanmaansjabloon per e-mail (sjablonen met soort aanmaan; maak er een met sjabloon_toevoegen); e-Boekhouden vult de codes per relatie en factuur in en kiest zelf naar welk e-mailadres van de relatie de mail gaat. Dit verstuurt echte e-mail en is niet terug te draaien. De gebruiker moet dit altijd goedkeuren in EB MCP en ziet daar de relaties met de e-mailadressen die EB MCP van ze kent, het sjabloon met onderwerp en tekst, en de facturen met hun openstaande bedragen. Is de uitkomst onbekend, kijk dan eerst met aanmaning_historie of de herinnering al is verstuurd voordat je het opnieuw probeert. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Herinnering versturen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn aanmaning_versturen(
        &self,
        Parameters(args): Parameters<AanmaningVersturenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "aanmaning_versturen";
        let account = args.account.clone();
        self.write_tool(&context, TOOL, account.as_deref(), args, "Herinneringen aangeboden aan e-Boekhouden; data.uitkomst zegt wat er is verstuurd", move |this, run, args| async move {
            let posten = this.opa_open_posten(&run, Kant::Debiteuren, &args.posten).await?;
            // The templates the aanmaning form offers for e-mail; the first is its default.
            let choices = rows(&this.fetch(&run, "aanmaansjabloon/soort/E", Vec::new()).await?);
            let ids: Vec<u64> = choices.iter().filter_map(|r| field(r, "id").and_then(Value::as_u64)).collect();
            let id = match args.sjabloon_id {
                Some(id) if ids.contains(&id) => id,
                Some(id) => {
                    return Err(Halt::Error(format!(
                        "Aanmaansjabloon {id} is geen sjabloon per e-mail in deze administratie; kies er een uit sjablonen (soort aanmaan)"
                    )));
                }
                None => *ids.first().ok_or_else(|| {
                    Halt::Error("Deze administratie heeft geen aanmaansjabloon per e-mail; maak er een met sjabloon_toevoegen (soort aanmaan)".into())
                })?,
            };
            let found = this.fetch(&run, &format!("aanmaansjabloon/{id}"), Vec::new()).await?;
            let sjabloon = Aanmaansjabloon {
                id,
                naam: text(&found, "naam"),
                onderwerp: text(&found, "onderwerp"),
                tekst: field(&found, "tekstEmail").and_then(Value::as_str).unwrap_or_default().to_owned(),
            };
            if sjabloon.onderwerp.is_empty() || sjabloon.tekst.trim().is_empty() {
                return Err(Halt::Error(format!(
                    "Aanmaansjabloon {} heeft geen onderwerp of tekst; vul die aan met sjabloon_wijzigen",
                    sjabloon.naam
                )));
            }
            let relaties = relatie_ids(posten.iter().map(|p| p.relatie_id));
            let mut ontvangers = Vec::with_capacity(relaties.len());
            for relatie_id in relaties {
                let relatie = this.fetch(&run, &format!("relatie/{relatie_id}"), Vec::new()).await?;
                ontvangers.push(Ontvanger::from_relatie(relatie_id, &relatie));
            }
            let built = aanmaning(
                &posten,
                &ontvangers,
                &sjabloon,
                args.kopie_naar.as_deref(),
                args.facturen_als_bijlage.unwrap_or(false),
            )?;
            this.approve(&run, TOOL, &built, true).await?;
            let sent = this.change(&run, "POST", "OpenPost/aanmaning", Some(&built.body)).await?;
            // Only an answer that says so means nothing went out; anything else may have sent mail, and is
            // reported as unknown so nobody sends the same reminder twice.
            verzonden(&sent).map_err(Halt::Error)
        })
        .await
    }

    #[tool(
        description = "De historie van herinneringen en aanmaningen van één open post: per keer de datum, de omschrijving en of het per post of per e-mail ging. Geef het factuurnummer en de relatie_id uit open_posten.",
        annotations(title = "Historie aanmaningen", read_only_hint = true)
    )]
    async fn aanmaning_historie(
        &self,
        Parameters(args): Parameters<AanmaningHistorieArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const PATH: &str = "openpost/gridtable/aanmaning";
        let run = match self.begin(
            &context,
            "aanmaning_historie",
            &args,
            args.account.as_deref(),
        ) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let factuur = args.factuur.trim();
        if factuur.is_empty() {
            return Ok(self.refuse(run, "Geef het factuurnummer van de open post".into()));
        }
        let query = match grid(args.limit, args.offset) {
            Ok(mut query) => {
                query.push(("factuurNummer".into(), factuur.to_owned()));
                query.push(("relatieId".into(), args.relatie_id.to_string()));
                query
            }
            Err(message) => return Ok(self.refuse(run, message)),
        };
        let result = self.fetch(&run, PATH, query).await.map(named_rows);
        Ok(self.end(run, PATH, result))
    }

    #[tool(
        description = "Een of meer open posten afboeken (wegboeken), zoals Afboeken op de open posten in e-Boekhouden: het openstaande bedrag gaat naar een tegenrekening, standaard Betalingsverschillen, en de factuur staat daarna niet meer open. Bij één open post kan een eigen bedrag; bij meer wordt elk hele openstaande bedrag afgeboekt. Dit schrijft een vordering of schuld af. De gebruiker moet dit altijd goedkeuren in EB MCP en ziet daar per factuur het bedrag en de grootboekrekening waar het heen gaat. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Open post afboeken",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn openpost_afboeken(
        &self,
        Parameters(args): Parameters<OpenpostAfboekenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "openpost_afboeken";
        let account = args.account.clone();
        self.write_tool(&context, TOOL, account.as_deref(), args, "Open posten afgeboekt", move |this, run, args| async move {
            let kant = if matches!(args.soort, Some(Soort::Crediteuren)) { Kant::Crediteuren } else { Kant::Debiteuren };
            let datum = match args.datum.as_deref() {
                Some(datum) => parse_date(datum)?,
                None => today(),
            };
            let omschrijving = args.omschrijving.clone().unwrap_or_else(|| "Betalingsverschil".to_owned());
            if args.bedrag.is_some() && args.posten.len() > 1 {
                return Err(Halt::Error(
                    "Een eigen bedrag kan alleen bij één open post; bij meer wordt van elke post het hele openstaande bedrag afgeboekt".into(),
                ));
            }
            let posten = this.opa_open_posten(&run, kant, &args.posten).await?;
            // The accounts the afboek form offers: active balance and result accounts.
            let choices: Vec<Tegenrekening> = rows(&this.fetch(&run, "grootboekrekening/selectlist/actief/VWBAL", Vec::new()).await?)
                .iter()
                .filter_map(Tegenrekening::from_row)
                .collect();
            let tegenrekening = pick_tegenrekening(&choices, args.tegenrekening.as_deref())?;
            let built = afboeken(kant, &posten, &tegenrekening, datum, &omschrijving, args.bedrag)?;
            this.approve(&run, TOOL, &built, true).await?;
            let answer = this.change(&run, "POST", "openpost/wegboeken", Some(&built.body)).await?;
            afgeboekt(&answer).map_err(Halt::Error)
        })
        .await
    }

    #[tool(
        description = "De berichten in e-Boekhouden (Overig > Berichten): van en aan de accountant en van e-Boekhouden zelf. Kies de map postvak (standaard), verzonden of archief. Per bericht onder meer id, datum en onderwerp; lees een bericht met bericht.",
        annotations(title = "Berichten", read_only_hint = true)
    )]
    async fn berichten(
        &self,
        Parameters(args): Parameters<BerichtenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const PATH: &str = "bericht/gridtable";
        let run = match self.begin(&context, "berichten", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let map = args.map.unwrap_or(BerichtenMap::Postvak);
        let query = match grid(args.limit, args.offset) {
            Ok(mut query) => {
                query.push(("type".into(), map.type_value().to_string()));
                query
            }
            Err(message) => return Ok(self.refuse(run, message)),
        };
        let result = self.fetch(&run, PATH, query).await.map(named_rows);
        Ok(self.end(run, PATH, result))
    }

    #[tool(
        description = "Eén bericht lezen, op id (uit berichten): van, aan, datum, onderwerp en de tekst (body, platte tekst). Dit is dezelfde vraag die e-Boekhouden stelt als de gebruiker het bericht opent; of e-Boekhouden het bericht daarbij als gelezen markeert, is niet bekend. Verder verandert er niets.",
        annotations(title = "Bericht", read_only_hint = true)
    )]
    // Kept read-only: it asks exactly what e-Boekhouden's own page asks when the user opens a message, and it
    // changes nothing in the books. The most it can change is the message's read mark, which the description
    // says; treating every look at a message as a change to approve would cost more than it protects.
    async fn bericht(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let run = match self.begin(&context, "bericht", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let path = format!("bericht/{}", args.id);
        // The page shows `body` as it is (plain text, line breaks kept), so it is passed on untouched.
        let result = self.fetch(&run, &path, Vec::new()).await;
        Ok(self.end(run, &path, result))
    }

    #[tool(
        description = "Berichten in het postvak markeren als gelezen, op id (uit berichten), zoals Markeren als gelezen in e-Boekhouden. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt het eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Bericht gelezen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true
        )
    )]
    async fn bericht_gelezen(
        &self,
        Parameters(args): Parameters<BerichtIdsArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        self.opa_berichten(
            context,
            "bericht_gelezen",
            args,
            BerichtActie::Gelezen,
            "Berichten gemarkeerd als gelezen",
        )
        .await
    }

    #[tool(
        description = "Berichten uit het postvak naar het archief verplaatsen, op id (uit berichten), zoals Verplaatsen naar het archief in e-Boekhouden. In het archief zijn ze nog te lezen (berichten met map archief). Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt het eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Bericht archiveren",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true
        )
    )]
    async fn bericht_archiveren(
        &self,
        Parameters(args): Parameters<BerichtIdsArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        self.opa_berichten(
            context,
            "bericht_archiveren",
            args,
            BerichtActie::Archiveren,
            "Berichten naar het archief verplaatst",
        )
        .await
    }

    #[tool(
        description = "Een bericht versturen aan de accountant van de administratie, via e-Boekhouden (Overig > Berichten), of een bericht beantwoorden (antwoord_op). Alleen voor een administratie die aan een accountant gekoppeld is, niet voor de login van een accountant; het bericht gaat altijd naar die accountant. Bij antwoord_op leest EB MCP eerst het oorspronkelijke bericht, zoals e-Boekhouden doet als je het opent. Dit verstuurt een bericht en is niet terug te draaien. De gebruiker moet dit altijd goedkeuren in EB MCP en ziet daar de ontvanger, het onderwerp en de tekst. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Bericht versturen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn bericht_versturen(
        &self,
        Parameters(args): Parameters<BerichtVersturenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "bericht_versturen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Bericht verstuurd; data.id is het id",
            move |this, run, args| async move {
                if this.opa_is_accountant(&run).await? {
                    return Err(Halt::Error(ACCOUNTANT_BERICHT.into()));
                }
                let given = args
                    .onderwerp
                    .as_deref()
                    .map(str::trim)
                    .filter(|o| !o.is_empty())
                    .map(str::to_owned);
                if given.is_none() && args.antwoord_op.is_none() {
                    return Err(Halt::Error("Geef een onderwerp".into()));
                }
                let aan = this.opa_accountant(&run).await?;
                let (onderwerp, original) = match args.antwoord_op {
                    Some(id) => {
                        let original = this
                            .fetch(&run, &format!("bericht/{id}"), Vec::new())
                            .await?;
                        let subject = text(&original, "onderwerp");
                        let mut about = format!(
                            "bericht {id}: {}",
                            if subject.is_empty() {
                                "zonder onderwerp"
                            } else {
                                &subject
                            }
                        );
                        let van = text(&original, "van");
                        if !van.is_empty() {
                            about.push_str(&format!(", van {van}"));
                        }
                        let datum = text(&original, "datum");
                        if !datum.is_empty() {
                            about.push_str(&format!(", {}", moment(&datum)));
                        }
                        (
                            given.unwrap_or_else(|| antwoord_onderwerp(&subject)),
                            Some(about),
                        )
                    }
                    None => (given.unwrap_or_default(), None),
                };
                let built = built::bericht(&aan, &onderwerp, &args.bericht, original.as_deref())?;
                this.approve(&run, TOOL, &built, true).await?;
                this.change(&run, "POST", "bericht", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "De controle-opdrachten: verzoeken aan de accountant om een maand of kwartaal te controleren, met datum, periode, gecontroleerd op en resultaat. Met als_accountant de opdrachten die klanten aan deze accountant stuurden.",
        annotations(title = "Controle-opdrachten", read_only_hint = true)
    )]
    async fn controleopdrachten(
        &self,
        Parameters(args): Parameters<ControleopdrachtenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let path = if args.als_accountant == Some(true) {
            "controleopdracht/accountant/gridtable"
        } else {
            "controleopdracht/gridtable"
        };
        let run = match self.begin(
            &context,
            "controleopdrachten",
            &args,
            args.account.as_deref(),
        ) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let query = match grid(args.limit, args.offset) {
            Ok(query) => query,
            Err(message) => return Ok(self.refuse(run, message)),
        };
        let result = self.fetch(&run, path, query).await.map(named_rows);
        Ok(self.end(run, path, result))
    }

    #[tool(
        description = "Eén controle-opdracht lezen, op id (uit controleopdrachten): datum, periode, de opmerkingen bij de aanvraag en de opmerkingen van de accountant.",
        annotations(title = "Controle-opdracht", read_only_hint = true)
    )]
    async fn controleopdracht(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let path = format!("controleopdracht/{}", args.id);
        self.simple(
            &context,
            "controleopdracht",
            &args,
            args.account.as_deref(),
            &path,
            Ok(Vec::new()),
        )
        .await
    }

    #[tool(
        description = "De accountant vragen een maand of kwartaal te controleren (een controle-opdracht), met een opmerking erbij. Alleen voor een administratie die aan een accountant gekoppeld is, niet voor de login van een accountant. Dit stuurt een verzoek naar de accountant en is niet terug te draaien. De gebruiker moet dit altijd goedkeuren in EB MCP en ziet daar de accountant, de periode en de opmerking. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Controle-opdracht aanvragen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn controleopdracht_aanvragen(
        &self,
        Parameters(args): Parameters<ControleopdrachtAanvragenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "controleopdracht_aanvragen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Controle-opdracht verstuurd; data.id is het id",
            move |this, run, args| async move {
                let opmerking = args.opmerking.clone().unwrap_or_default();
                let kwartaal = args.periode == ControlePeriode::Kwartaal;
                // Checked before anything is read; the accountant's name is filled in below.
                controle_aanvraag(
                    "",
                    kwartaal,
                    args.jaar,
                    args.maand,
                    args.kwartaal,
                    &opmerking,
                    today(),
                )?;
                if this.opa_is_accountant(&run).await? {
                    return Err(Halt::Error(ACCOUNTANT_CONTROLE.into()));
                }
                let aan = this.opa_accountant(&run).await?;
                let built = controle_aanvraag(
                    &aan,
                    kwartaal,
                    args.jaar,
                    args.maand,
                    args.kwartaal,
                    &opmerking,
                    today(),
                )?;
                this.approve(&run, TOOL, &built, true).await?;
                this.change(&run, "POST", "controleopdracht", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Als accountant een controle-opdracht van een klant goedkeuren of afkeuren, met een opmerking voor de klant, op id (uit controleopdrachten met als_accountant). Alleen voor een account van een accountant. De klant ziet het besluit; het is niet terug te draaien. De gebruiker moet dit altijd goedkeuren in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Controle-opdracht beoordelen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn controleopdracht_beoordelen(
        &self,
        Parameters(args): Parameters<ControleopdrachtBeoordelenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "controleopdracht_beoordelen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Controle-opdracht beoordeeld",
            move |this, run, args| async move {
                let opmerking = args.opmerking.clone().unwrap_or_default();
                if !this.opa_is_accountant(&run).await? {
                    return Err(Halt::Error(GEEN_ACCOUNTANT.into()));
                }
                let opdracht = this
                    .fetch(&run, &format!("controleopdracht/{}", args.id), Vec::new())
                    .await?;
                let built = controle_beoordeling(args.id, &opdracht, args.goedkeuren, &opmerking)?;
                this.approve(&run, TOOL, &built, true).await?;
                this.change(
                    &run,
                    "PUT",
                    "controleopdracht/accountant/opmerking",
                    Some(&built.body),
                )
                .await
            },
        )
        .await
    }
}
