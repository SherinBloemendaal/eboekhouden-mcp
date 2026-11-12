//! Standaard facturen (Factureren > Standaard facturen): an invoice kept as a starting point, with a name, a
//! template and fixed lines, that a new invoice or subscription can be started from. e-Boekhouden keeps it as
//! an invoice of type "V" with the invoice's own lines (`factuuritem`).

use serde_json::Map;

use super::*;
use crate::approvals::Line;
use crate::write::{FactuurRegelArg, InEx};

/// The type e-Boekhouden gives a standaard factuur (`Zt.StandaardFactuur` in its web app).
const TYPE: &str = "V";

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct StandaardfactuurToevoegenArgs {
    /// Naam van de standaard factuur (hoogstens 40 tekens).
    pub naam: String,
    /// Id van het factuursjabloon, de opmaak (uit sjablonen). Standaard het factuursjabloon uit de factuurinstellingen.
    pub sjabloon_id: Option<u64>,
    /// Id van het e-mailsjabloon voor facturen (uit emailsjablonen). Standaard geen.
    pub email_sjabloon_id: Option<u64>,
    /// Zijn de prijzen incl of excl btw (standaard excl).
    pub bedragen: Option<InEx>,
    /// De vaste regels: de producten of diensten die een factuur van deze standaard factuur meekrijgt.
    pub regels: Vec<FactuurRegelArg>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct StandaardfactuurWijzigenArgs {
    /// Id van de standaard factuur (uit standaardfacturen).
    pub id: u64,
    /// Nieuwe naam.
    pub naam: Option<String>,
    /// Id van het nieuwe factuursjabloon (uit sjablonen).
    pub sjabloon_id: Option<u64>,
    /// Id van het nieuwe e-mailsjabloon (uit emailsjablonen); 0 haalt het e-mailsjabloon weg.
    pub email_sjabloon_id: Option<u64>,
    /// Prijzen incl of excl btw. Alleen samen met `regels`.
    pub bedragen: Option<InEx>,
    /// Vervangt alle regels. Laat weg om ze te houden.
    pub regels: Option<Vec<FactuurRegelArg>>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// What the standaard factuur form reads before it saves: the books, whether the administration charges btw,
/// the active invoice templates (`factuursjabloon/active`), the e-mail templates for invoices
/// (`emailsjabloonab/forfactuur`) and the invoice template of the invoice settings.
pub(super) struct SfKeuzes {
    pub books: Books,
    pub btwplichtig: bool,
    pub sjablonen: Vec<(u64, String)>,
    pub emailsjablonen: Vec<(u64, String)>,
    pub standaard_sjabloon: Option<u64>,
}

/// Id and naam of each row of a list.
fn named_list(rows: &[Value]) -> Vec<(u64, String)> {
    rows.iter()
        .filter_map(|row| Some((row.get("id")?.as_u64()?, text_of(row, "naam"))))
        .collect()
}

fn choices(list: &[(u64, String)]) -> String {
    list.iter()
        .map(|(id, naam)| format!("{id} {naam}"))
        .collect::<Vec<_>>()
        .join(", ")
}

impl SfKeuzes {
    /// The invoice template: the one named, else the one of the invoice settings, else the first active one,
    /// as the form picks it.
    fn sjabloon(&self, id: Option<u64>) -> Result<(u64, String), String> {
        let active = |id: u64| self.sjablonen.iter().find(|(s, _)| *s == id).cloned();
        match id {
            Some(id) => active(id).ok_or_else(|| {
                format!(
                    "Geen actief factuursjabloon met id {id}. Kies uit: {}",
                    choices(&self.sjablonen)
                )
            }),
            None => self
                .standaard_sjabloon
                .and_then(active)
                .or_else(|| self.sjablonen.first().cloned())
                .ok_or_else(|| "Deze administratie heeft geen actief factuursjabloon".to_owned()),
        }
    }

    fn emailsjabloon(&self, id: u64) -> Result<String, String> {
        self.emailsjablonen
            .iter()
            .find(|(s, _)| *s == id)
            .map(|(_, naam)| naam.clone())
            .ok_or_else(|| {
                format!(
                    "Geen e-mailsjabloon voor facturen met id {id}. Kies uit: {}",
                    choices(&self.emailsjablonen)
                )
            })
    }

    /// A template as the standaard factuur has it now: its name, or its id when it is no longer listed.
    fn sjabloon_naam(&self, id: u64) -> String {
        self.sjablonen
            .iter()
            .find(|(s, _)| *s == id)
            .map(|(_, naam)| naam.clone())
            .unwrap_or_else(|| format!("id {id}"))
    }

    fn emailsjabloon_naam(&self, id: Option<u64>) -> String {
        match id {
            None => "geen".to_owned(),
            Some(id) => self
                .emailsjabloon(id)
                .unwrap_or_else(|_| format!("id {id}")),
        }
    }

    /// Incl or excl btw. Without btw duty the form offers excl only.
    fn bedragen(&self, in_ex: InEx) -> Result<InEx, String> {
        if !self.btwplichtig && in_ex == InEx::Incl {
            return Err(
                "Deze administratie is niet btw-plichtig: de prijzen zijn dan altijd excl. btw"
                    .into(),
            );
        }
        Ok(in_ex)
    }
}

/// The lines of a standaard factuur, built and checked exactly as an invoice's. The standaard factuur form
/// uses the invoice's own line list (`app-factuurregel-list`), so `factuurItems.insert` has the same shape;
/// `write::factuur_nieuw` is the public way in to those lines (`write::factuur_items` is private to write.rs).
fn regel_items(
    regels: &[FactuurRegelArg],
    in_ex: InEx,
    k: &SfKeuzes,
) -> Result<(Vec<Value>, Vec<Line>), String> {
    if !k.btwplichtig
        && let Some(n) = regels
            .iter()
            .position(|r| !r.btw_code.trim().eq_ignore_ascii_case("GEEN"))
    {
        return Err(format!(
            "Deze administratie is niet btw-plichtig: regel {} krijgt btw-code GEEN",
            n + 1
        ));
    }
    let invoice = write::FactuurToevoegenArgs {
        relatie_id: 0,
        datum: None,
        nummer: Some(TYPE.to_owned()),
        termijn: None,
        bedragen: Some(in_ex),
        regels: regels.to_vec(),
        factuurtekst: None,
        kenmerk: None,
        direct_boeken: Some(false),
        account: None,
    };
    let context = write::FactuurContext {
        volgend_nummer: String::new(),
        sjabloon_id: 0,
        verwerken: false,
        vandaag: today(),
    };
    let built = write::factuur_nieuw(&invoice, &context, &k.books)?;
    let items = built
        .body
        .pointer("/factuurItems/insert")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if items.len() != regels.len() {
        return Err("EB MCP kon de regels niet opbouwen; er is niets gewijzigd".into());
    }
    let lines = built
        .lines
        .into_iter()
        .filter(|l| l.label == "Prijzen" || l.label.starts_with("Regel "))
        .collect();
    Ok((items, lines))
}

/// The ids of the lines a standaard factuur has now (`factuuritem/griditems`).
fn item_ids(items: &Value) -> Vec<Value> {
    items
        .as_array()
        .map(|rows| rows.iter().filter_map(|r| r.get("id").cloned()).collect())
        .unwrap_or_default()
}

/// A new standaard factuur, as `POST standaardfactuur/model` from its form.
pub(super) fn standaardfactuur_nieuw(
    args: &StandaardfactuurToevoegenArgs,
    k: &SfKeuzes,
) -> Result<Built, String> {
    let naam = required_text(&args.naam, "De naam", 40)?;
    let in_ex = k.bedragen(args.bedragen.unwrap_or(InEx::Excl))?;
    let (sjabloon_id, sjabloon_naam) = k.sjabloon(args.sjabloon_id)?;
    let email = args
        .email_sjabloon_id
        .filter(|id| *id > 0)
        .map(|id| k.emailsjabloon(id).map(|naam| (id, naam)))
        .transpose()?;
    let (items, item_lines) = regel_items(&args.regels, in_ex, k)?;
    let mut factuur = Map::new();
    factuur.insert("nummer".into(), json!(naam));
    factuur.insert("sjabloonId".into(), json!(sjabloon_id));
    if let Some((id, _)) = &email {
        factuur.insert("emailSjabloonId".into(), json!(id));
    }
    factuur.insert("type".into(), json!(TYPE));
    factuur.insert("inEx".into(), json!(in_ex_code(in_ex)));
    let mut lines = vec![line("Naam", &naam), line("Factuursjabloon", sjabloon_naam)];
    if let Some((_, naam)) = email {
        lines.push(line("E-mailsjabloon", naam));
    }
    lines.extend(item_lines);
    Ok(Built {
        body: json!({"factuur": factuur, "factuurItems": {"insert": items, "update": [], "delete": []}}),
        title: format!("Standaard factuur {naam} toevoegen"),
        lines,
    })
}

/// An edit of a standaard factuur, as `PUT standaardfactuur/model` from its form: the form's fields, taken from
/// the standaard factuur as it is (`now`), with the named changes. New lines replace all the old ones.
pub(super) fn standaardfactuur_gewijzigd(
    args: &StandaardfactuurWijzigenArgs,
    now: &Value,
    items: &Value,
    k: &SfKeuzes,
) -> Result<Built, String> {
    let bad = || "e-Boekhouden gaf deze standaard factuur in een onbekende vorm terug".to_owned();
    let old_naam = now
        .get("nummer")
        .and_then(Value::as_str)
        .ok_or_else(bad)?
        .to_owned();
    let old_sjabloon = now
        .get("sjabloonId")
        .and_then(Value::as_u64)
        .ok_or_else(bad)?;
    let old_email = id_of(now, "emailSjabloonId");
    let old_in_ex = in_ex_from(&text_of(now, "inEx"));
    let mut lines = vec![line("Standaard factuur", &old_naam)];
    let mut changed = false;

    let mut naam = old_naam.clone();
    if let Some(new) = &args.naam {
        let new = required_text(new, "De naam", 40)?;
        if new != naam {
            lines.push(line("Naam wordt", &new));
            naam = new;
            changed = true;
        }
    }
    let mut sjabloon = old_sjabloon;
    if let Some(id) = args.sjabloon_id.filter(|id| *id != old_sjabloon) {
        let (id, nieuw) = k.sjabloon(Some(id))?;
        lines.push(line("Factuursjabloon was", k.sjabloon_naam(old_sjabloon)));
        lines.push(line("Factuursjabloon wordt", nieuw));
        sjabloon = id;
        changed = true;
    }
    let mut email = old_email;
    if let Some(new) = args.email_sjabloon_id.map(|id| (id > 0).then_some(id))
        && new != old_email
    {
        let wordt = match new {
            Some(id) => k.emailsjabloon(id)?,
            None => "geen".to_owned(),
        };
        lines.push(line("E-mailsjabloon was", k.emailsjabloon_naam(old_email)));
        lines.push(line("E-mailsjabloon wordt", wordt));
        email = new;
        changed = true;
    }

    let mut in_ex = if k.btwplichtig { old_in_ex } else { InEx::Excl };
    let mut item_changes = json!({"insert": [], "update": [], "delete": []});
    match &args.regels {
        Some(regels) => {
            if let Some(new) = args.bedragen {
                in_ex = k.bedragen(new)?;
            }
            let (new_items, item_lines) = regel_items(regels, in_ex, k)?;
            lines.push(line("Regels", "alle regels worden vervangen door:"));
            lines.extend(item_lines);
            item_changes = json!({"insert": new_items, "update": [], "delete": item_ids(items)});
            changed = true;
        }
        None if args.bedragen.is_some() => {
            return Err("Incl of excl btw wijzigen kan alleen samen met nieuwe regels".into());
        }
        None => {}
    }
    if !changed {
        return Err("Er is niets gewijzigd: geef aan wat er anders moet".into());
    }
    let factuur = json!({
        "id": args.id,
        "nummer": naam,
        "sjabloonId": sjabloon,
        "emailSjabloonId": email,
        "type": TYPE,
        "inEx": in_ex_code(in_ex),
    });
    Ok(Built {
        body: json!({"factuur": factuur, "factuurItems": item_changes}),
        title: format!("Standaard factuur {old_naam} wijzigen"),
        lines,
    })
}

impl EbMcp {
    /// What a standaard factuur is checked against before it is saved (`SfKeuzes`).
    async fn artikelen_sf_keuzes(&self, run: &Run) -> Result<SfKeuzes, String> {
        let setting = self.fetch(run, "factuursetting", Vec::new()).await?;
        Ok(SfKeuzes {
            books: self.books(run).await?,
            btwplichtig: self.artikelen_btwplichtig(run).await?,
            sjablonen: named_list(&self.artikelen_list(run, "factuursjabloon/active").await?),
            emailsjablonen: named_list(
                &self
                    .artikelen_list(run, "emailsjabloonab/forfactuur")
                    .await?,
            ),
            standaard_sjabloon: setting.get("factuurSjabloonId").and_then(Value::as_u64),
        })
    }
}

#[tool_router(router = artikelen_standaardfacturen_router, vis = "pub(super)")]
impl EbMcp {
    #[tool(
        description = "De standaard facturen (Factureren > Standaard facturen): vaste facturen met regels waar een nieuwe factuur of een abonnement mee kan beginnen. Per standaard factuur het id en de naam.",
        annotations(title = "Standaard facturen", read_only_hint = true)
    )]
    async fn standaardfacturen(
        &self,
        Parameters(args): Parameters<ListArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const PATH: &str = "standaardfactuur/gridtable";
        let run = match self.begin(
            &context,
            "standaardfacturen",
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
        let result = self.fetch(&run, PATH, query).await.map(named_rows);
        Ok(self.end(run, PATH, result))
    }

    #[tool(
        description = "Eén standaard factuur met zijn sjabloon, e-mailsjabloon, incl of excl btw en zijn regels, op id (uit standaardfacturen).",
        annotations(title = "Standaard factuur", read_only_hint = true)
    )]
    async fn standaardfactuur(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let run = match self.begin(&context, "standaardfactuur", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let result = self
            .with_items(&run, "standaardfactuur", args.id)
            .await
            .map(|(standaardfactuur, regels)| {
                json!({"standaardfactuur": standaardfactuur, "regels": regels})
            });
        Ok(self.end(run, &format!("standaardfactuur/{}", args.id), result))
    }

    #[tool(
        description = "Een standaard factuur maken: een naam, het factuursjabloon (standaard dat uit de factuurinstellingen), eventueel een e-mailsjabloon, en vaste regels zoals op een factuur (prijs per stuk incl of excl btw, btw op code, omzetrekening op code). Er wordt geen factuur gemaakt of verstuurd. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de standaard factuur eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Standaard factuur toevoegen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn standaardfactuur_toevoegen(
        &self,
        Parameters(args): Parameters<StandaardfactuurToevoegenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "standaardfactuur_toevoegen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Standaard factuur toegevoegd; data.id is het id",
            move |this, run, args| async move {
                let keuzes = this.artikelen_sf_keuzes(&run).await?;
                let built = standaardfactuur_nieuw(&args, &keuzes)?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change(&run, "POST", "standaardfactuur/model", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een standaard factuur wijzigen, op id. Wat je niet noemt blijft zoals het is; regels die je meegeeft vervangen alle bestaande. Facturen die al met deze standaard factuur gemaakt zijn veranderen niet. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de wijziging eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Standaard factuur wijzigen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn standaardfactuur_wijzigen(
        &self,
        Parameters(args): Parameters<StandaardfactuurWijzigenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "standaardfactuur_wijzigen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Standaard factuur gewijzigd",
            move |this, run, args| async move {
                let (now, items) = this.with_items(&run, "standaardfactuur", args.id).await?;
                let keuzes = this.artikelen_sf_keuzes(&run).await?;
                let built = standaardfactuur_gewijzigd(&args, &now, &items, &keuzes)?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change_done(&run, "PUT", "standaardfactuur/model", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een standaard factuur verwijderen, op id. Facturen die er al mee gemaakt zijn blijven staan. Dit is niet terug te draaien. De gebruiker moet dit altijd goedkeuren in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Standaard factuur verwijderen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn standaardfactuur_verwijderen(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "standaardfactuur_verwijderen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Standaard factuur verwijderd",
            move |this, run, args| async move {
                let (now, items) = this.with_items(&run, "standaardfactuur", args.id).await?;
                let naam = text_of(&now, "nummer");
                let built = Built {
                    body: Value::Null,
                    title: format!("Standaard factuur {naam} verwijderen"),
                    lines: vec![
                        line("Standaard factuur", &naam),
                        line("Regels", item_ids(&items).len()),
                        line(
                            "Gevolg",
                            "de standaard factuur verdwijnt; facturen die er al mee gemaakt zijn blijven staan. Dit is niet terug te draaien",
                        ),
                    ],
                };
                this.approve(&run, TOOL, &built, true).await?;
                this.change_done(
                    &run,
                    "DELETE",
                    &format!("standaardfactuur/{}", args.id),
                    None,
                )
                .await
            },
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keuzes(btwplichtig: bool) -> SfKeuzes {
        SfKeuzes {
            books: Books::from_api(
                &json!({
                    "colMetadata": [{"name": "id"}, {"name": "code"}, {"name": "omschrijving"}, {"name": "categorie"}],
                    "data": [[80, "8000", "Omzet", "Winst en Verlies"], [13, "1300", "Debiteuren", "Debiteuren"]]
                }),
                &json!([
                    {"code": "GEEN", "omschrijving": "Geen btw", "percentage": 0, "soort": ""},
                    {"code": "HOOG_VERK_21", "omschrijving": "Hoog 21%", "percentage": 21, "soort": "VERK"},
                    {"code": "HOOG_INK_21", "omschrijving": "Hoog inkoop", "percentage": 21, "soort": "INK"}
                ]),
            )
            .unwrap(),
            btwplichtig,
            sjablonen: vec![(8, "Oud".into()), (9, "Standaard".into())],
            emailsjablonen: vec![(5, "Factuurmail".into())],
            standaard_sjabloon: Some(9),
        }
    }

    fn regel(btw: &str, prijs: f64) -> FactuurRegelArg {
        FactuurRegelArg {
            aantal: Some(2.0),
            omschrijving: "Onderhoud".into(),
            prijs,
            btw_code: btw.into(),
            tegenrekening: "8000".into(),
            code: Some("OND".into()),
        }
    }

    fn nieuw() -> StandaardfactuurToevoegenArgs {
        StandaardfactuurToevoegenArgs {
            naam: " Maandelijks onderhoud ".into(),
            sjabloon_id: None,
            email_sjabloon_id: None,
            bedragen: None,
            regels: vec![regel("HOOG_VERK_21", 50.0)],
            account: None,
        }
    }

    fn value<'a>(built: &'a Built, label: &str) -> Option<&'a str> {
        built
            .lines
            .iter()
            .find(|l| l.label == label)
            .map(|l| l.value.as_str())
    }

    #[test]
    fn makes_a_standard_invoice_as_e_boekhoudens_form_does() {
        let built = standaardfactuur_nieuw(&nieuw(), &keuzes(true)).unwrap();
        assert_eq!(
            built.body,
            json!({"factuur": {"nummer": "Maandelijks onderhoud", "sjabloonId": 9, "type": "V", "inEx": "EX"},
                "factuurItems": {"insert": [{"id": -1, "factuurId": 0, "aantal": 2.0, "omschrijving": "Onderhoud", "prijs": null,
                    "btw": "HOOG_VERK_21", "grootboekId": 80, "prijsExcl": 50.0, "prijsIncl": 60.5, "kortingIsPerc": true,
                    "volgorde": 0, "code": "OND"}], "update": [], "delete": []}})
        );
        assert_eq!(
            built.title,
            "Standaard factuur Maandelijks onderhoud toevoegen"
        );
        assert_eq!(value(&built, "Factuursjabloon"), Some("Standaard"));
        assert_eq!(value(&built, "Prijzen"), Some("excl. btw"));
        assert_eq!(
            value(&built, "Regel 1"),
            Some("2 x Onderhoud: 50,00 per stuk (Hoog 21%), op 8000 Omzet")
        );
        assert_eq!(value(&built, "Relatie"), None, "no invoice lines leak in");

        // Prices incl btw, another template and an e-mail template.
        let args = StandaardfactuurToevoegenArgs {
            sjabloon_id: Some(8),
            email_sjabloon_id: Some(5),
            bedragen: Some(InEx::Incl),
            regels: vec![regel("HOOG_VERK_21", 60.5), regel("GEEN", 10.0)],
            ..nieuw()
        };
        let built = standaardfactuur_nieuw(&args, &keuzes(true)).unwrap();
        assert_eq!(built.body["factuur"]["sjabloonId"], 8);
        assert_eq!(built.body["factuur"]["emailSjabloonId"], 5);
        assert_eq!(built.body["factuur"]["inEx"], "IN");
        let items = built.body["factuurItems"]["insert"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["prijsExcl"], json!(50.0));
        assert_eq!(items[1]["id"], -2);
        assert_eq!(items[1]["volgorde"], 1);
        assert_eq!(value(&built, "E-mailsjabloon"), Some("Factuurmail"));
    }

    #[test]
    fn takes_the_first_active_template_when_the_settings_name_none() {
        let mut k = keuzes(true);
        k.standaard_sjabloon = Some(77);
        let built = standaardfactuur_nieuw(&nieuw(), &k).unwrap();
        assert_eq!(built.body["factuur"]["sjabloonId"], 8);
        k.sjablonen.clear();
        assert!(
            standaardfactuur_nieuw(&nieuw(), &k)
                .unwrap_err()
                .contains("geen actief factuursjabloon")
        );
    }

    #[test]
    fn refuses_a_standard_invoice_e_boekhouden_would_refuse() {
        let k = keuzes(true);
        let err =
            |args: StandaardfactuurToevoegenArgs| standaardfactuur_nieuw(&args, &k).unwrap_err();
        assert!(
            err(StandaardfactuurToevoegenArgs {
                naam: " ".into(),
                ..nieuw()
            })
            .contains("De naam is leeg")
        );
        assert!(
            err(StandaardfactuurToevoegenArgs {
                naam: "n".repeat(41),
                ..nieuw()
            })
            .contains("hoogstens 40")
        );
        assert!(
            err(StandaardfactuurToevoegenArgs {
                sjabloon_id: Some(3),
                ..nieuw()
            })
            .contains("8 Oud, 9 Standaard")
        );
        assert!(
            err(StandaardfactuurToevoegenArgs {
                email_sjabloon_id: Some(6),
                ..nieuw()
            })
            .contains("5 Factuurmail")
        );
        assert!(
            err(StandaardfactuurToevoegenArgs {
                regels: vec![],
                ..nieuw()
            })
            .contains("minstens één regel")
        );
        assert!(
            err(StandaardfactuurToevoegenArgs {
                regels: vec![regel("HOOG_INK_21", 1.0)],
                ..nieuw()
            })
            .contains("inkoop")
        );
        let mut leeg = regel("GEEN", 1.0);
        leeg.omschrijving = String::new();
        assert!(
            err(StandaardfactuurToevoegenArgs {
                regels: vec![leeg],
                ..nieuw()
            })
            .contains("geen omschrijving")
        );

        // Without btw duty: GEEN on every line, and prices excl.
        let k = keuzes(false);
        let err =
            |args: StandaardfactuurToevoegenArgs| standaardfactuur_nieuw(&args, &k).unwrap_err();
        assert!(err(nieuw()).contains("regel 1 krijgt btw-code GEEN"));
        assert!(
            err(StandaardfactuurToevoegenArgs {
                bedragen: Some(InEx::Incl),
                regels: vec![regel("GEEN", 1.0)],
                ..nieuw()
            })
            .contains("altijd excl. btw")
        );
        let built = standaardfactuur_nieuw(
            &StandaardfactuurToevoegenArgs {
                regels: vec![regel("geen", 1.0)],
                ..nieuw()
            },
            &k,
        )
        .unwrap();
        assert_eq!(built.body["factuurItems"]["insert"][0]["btw"], "GEEN");
    }

    fn now() -> Value {
        json!({"id": 501, "nummer": "Maandelijks onderhoud", "sjabloonId": 9, "emailSjabloonId": 5, "inEx": "IN", "type": "V", "relatieId": 0})
    }

    fn items() -> Value {
        json!([{"id": 9001, "omschrijving": "Onderhoud"}, {"id": 9002, "omschrijving": "Licentie"}])
    }

    fn wijzigen() -> StandaardfactuurWijzigenArgs {
        StandaardfactuurWijzigenArgs {
            id: 501,
            naam: None,
            sjabloon_id: None,
            email_sjabloon_id: None,
            bedragen: None,
            regels: None,
            account: None,
        }
    }

    #[test]
    fn an_edit_sends_the_form_back_with_only_the_named_change() {
        let k = keuzes(true);
        let args = StandaardfactuurWijzigenArgs {
            naam: Some("Onderhoud per maand".into()),
            email_sjabloon_id: Some(0),
            ..wijzigen()
        };
        let built = standaardfactuur_gewijzigd(&args, &now(), &items(), &k).unwrap();
        assert_eq!(
            built.body,
            json!({"factuur": {"id": 501, "nummer": "Onderhoud per maand", "sjabloonId": 9, "emailSjabloonId": null, "type": "V", "inEx": "IN"},
                "factuurItems": {"insert": [], "update": [], "delete": []}})
        );
        assert_eq!(
            built.title,
            "Standaard factuur Maandelijks onderhoud wijzigen"
        );
        assert_eq!(value(&built, "Naam wordt"), Some("Onderhoud per maand"));
        assert_eq!(value(&built, "E-mailsjabloon was"), Some("Factuurmail"));
        assert_eq!(value(&built, "E-mailsjabloon wordt"), Some("geen"));

        // New lines replace all the old ones; the way prices are entered stays unless named.
        let args = StandaardfactuurWijzigenArgs {
            regels: Some(vec![regel("HOOG_VERK_21", 121.0)]),
            sjabloon_id: Some(8),
            ..wijzigen()
        };
        let built = standaardfactuur_gewijzigd(&args, &now(), &items(), &k).unwrap();
        assert_eq!(built.body["factuurItems"]["delete"], json!([9001, 9002]));
        assert_eq!(
            built.body["factuurItems"]["insert"][0]["prijsExcl"],
            json!(100.0)
        );
        assert_eq!(built.body["factuur"]["inEx"], "IN");
        assert_eq!(built.body["factuur"]["sjabloonId"], 8);
        assert_eq!(value(&built, "Factuursjabloon was"), Some("Standaard"));
        assert_eq!(value(&built, "Factuursjabloon wordt"), Some("Oud"));
        assert_eq!(
            value(&built, "Regels"),
            Some("alle regels worden vervangen door:")
        );
        let args = StandaardfactuurWijzigenArgs {
            regels: Some(vec![regel("HOOG_VERK_21", 100.0)]),
            bedragen: Some(InEx::Excl),
            ..wijzigen()
        };
        let built = standaardfactuur_gewijzigd(&args, &now(), &items(), &k).unwrap();
        assert_eq!(built.body["factuur"]["inEx"], "EX");
        assert_eq!(
            built.body["factuurItems"]["insert"][0]["prijsIncl"],
            json!(121.0)
        );
    }

    #[test]
    fn refuses_an_edit_that_changes_nothing_or_is_wrong() {
        let k = keuzes(true);
        let err = |args: StandaardfactuurWijzigenArgs| {
            standaardfactuur_gewijzigd(&args, &now(), &items(), &k).unwrap_err()
        };
        assert!(err(wijzigen()).contains("niets gewijzigd"));
        assert!(
            err(StandaardfactuurWijzigenArgs {
                naam: Some("Maandelijks onderhoud".into()),
                sjabloon_id: Some(9),
                email_sjabloon_id: Some(5),
                ..wijzigen()
            })
            .contains("niets gewijzigd")
        );
        assert!(
            err(StandaardfactuurWijzigenArgs {
                bedragen: Some(InEx::Excl),
                ..wijzigen()
            })
            .contains("alleen samen met nieuwe regels")
        );
        assert!(
            err(StandaardfactuurWijzigenArgs {
                sjabloon_id: Some(4),
                ..wijzigen()
            })
            .contains("Geen actief factuursjabloon met id 4")
        );
        assert!(
            standaardfactuur_gewijzigd(&wijzigen(), &json!({"id": 501}), &items(), &k)
                .unwrap_err()
                .contains("onbekende vorm")
        );
    }
}
