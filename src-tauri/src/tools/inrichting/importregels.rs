//! Automatische importregels: rules that book imported bank lines by themselves, and running them again.
//!
//! From e-Boekhouden's bundle (chunk-BHlgGYJf.js, `app-importregels-list` and `app-importregels-form`):
//! - list: `GET importregel/gridtable` (columns bijAf, omschrijving, omschrijving2, bedragVan, bedragTot,
//!   mutatiesoort, tegenrekening); one: `GET importregel/{id}`.
//! - add: `POST importregel {omschrijving, omschrijving2, bijAf, bedragVan, bedragTot, mutatieSoortId,
//!   mutatieBtwCode, mutatieTegenrekeningId, actief[, kostenplaatsId]}` -> `{id, ...}`; edit: `PUT importregel`
//!   with the same plus `id`; delete: `DELETE importregel/{id}`.
//! - The form offers mutatiesoort 5 (Geld ontvangen) and 6 (Geld uitgegeven) only, btw codes of soort VERK (5) or
//!   INK (6) plus GEEN, and tegenrekeningen from `grootboekrekening/search?rekeningCategorie=VW,BAL,BTWRC`. It wants
//!   at least one criterion: a text, or an amount that is not 0. Texts take 50 characters.
//! - "Alle regels opnieuw automatisch verwerken" (`app-import-list`, `processLines`): `POST import/verwerk` without
//!   a body -> `{countImport, countMutatie, countErrors}`. It processes the unprocessed bank lines again and books
//!   what it can in one go; the bundle does not show which lines that will be, so the approval does not claim it.
//! - Btw codes: the form takes `btwinfo/info`, keeps the codes valid today (`filterInfo(info, {date: today})`) and
//!   only those with `simpleExtended == 'S'` unless `setting/administratie.use-extended-btwcodes` is true or the user
//!   is an accountant (which EB MCP cannot see; it keeps to the setting).

use super::*;

/// Whether a rule is about money coming in or going out, as the form names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BijAf {
    Bijschrijving,
    Afschrijving,
}

impl BijAf {
    fn api(self) -> &'static str {
        match self {
            Self::Bijschrijving => "Bijschrijving",
            Self::Afschrijving => "Afschrijving",
        }
    }

    fn from_api(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "bijschrijving" => Some(Self::Bijschrijving),
            "afschrijving" => Some(Self::Afschrijving),
            _ => None,
        }
    }
}

/// The mutatiesoort a rule books as: the two the form offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RegelSoort {
    GeldOntvangen,
    GeldUitgegeven,
}

impl RegelSoort {
    fn nr(self) -> u64 {
        match self {
            Self::GeldOntvangen => 5,
            Self::GeldUitgegeven => 6,
        }
    }

    fn from_nr(nr: u64) -> Option<Self> {
        match nr {
            5 => Some(Self::GeldOntvangen),
            6 => Some(Self::GeldUitgegeven),
            _ => None,
        }
    }

    fn naam(self) -> &'static str {
        match self {
            Self::GeldOntvangen => "Geld ontvangen",
            Self::GeldUitgegeven => "Geld uitgegeven",
        }
    }

    /// The btw soort the form lists for this mutatiesoort (GEEN fits both).
    fn btw_soort(self) -> &'static str {
        match self {
            Self::GeldOntvangen => "VERK",
            Self::GeldUitgegeven => "INK",
        }
    }

    /// What the form would pick for a bank line of this kind.
    fn voor(bij_af: BijAf) -> Self {
        match bij_af {
            BijAf::Bijschrijving => Self::GeldOntvangen,
            BijAf::Afschrijving => Self::GeldUitgegeven,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ImportregelToevoegenArgs {
    /// Tekst die in de omschrijving van de bankregel moet voorkomen, bijvoorbeeld de naam of het IBAN van de tegenpartij. Hoogstens 50 tekens.
    pub omschrijving: Option<String>,
    /// Een tweede tekst die daarnaast ook in de omschrijving moet voorkomen. Hoogstens 50 tekens.
    pub omschrijving2: Option<String>,
    /// bijschrijving (geld komt binnen) of afschrijving (geld gaat eraf).
    pub bij_af: BijAf,
    /// Het bedrag van de bankregel is gelijk aan of groter dan dit bedrag.
    pub bedrag_van: Option<f64>,
    /// Het bedrag van de bankregel is gelijk aan of kleiner dan dit bedrag.
    pub bedrag_tot: Option<f64>,
    /// Als welke mutatie de regel boekt: geld_ontvangen of geld_uitgegeven. Standaard geld_ontvangen bij een bijschrijving en geld_uitgegeven bij een afschrijving.
    pub soort: Option<RegelSoort>,
    /// Code van de grootboekrekening waarop de regel boekt (een balans-, winst-en-verlies- of btw-rekening), bijvoorbeeld 4550.
    pub tegenrekening: String,
    /// Btw-code van de boeking: een verkoopcode bij geld_ontvangen, een inkoopcode bij geld_uitgegeven, of GEEN.
    pub btw_code: String,
    /// Of de regel meteen meedoet (standaard ja).
    pub actief: Option<bool>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// A field of an importregel that an edit can empty.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportregelVeld {
    Omschrijving,
    Omschrijving2,
    BedragVan,
    BedragTot,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ImportregelWijzigenArgs {
    /// Id van de importregel (uit importregels).
    pub id: u64,
    /// Nieuwe tekst die in de omschrijving moet voorkomen. Hoogstens 50 tekens.
    pub omschrijving: Option<String>,
    /// Nieuwe tweede tekst. Hoogstens 50 tekens.
    pub omschrijving2: Option<String>,
    /// bijschrijving of afschrijving.
    pub bij_af: Option<BijAf>,
    /// Nieuwe ondergrens van het bedrag.
    pub bedrag_van: Option<f64>,
    /// Nieuwe bovengrens van het bedrag.
    pub bedrag_tot: Option<f64>,
    /// geld_ontvangen of geld_uitgegeven. Past de huidige btw-code daar niet bij, geef dan ook btw_code.
    pub soort: Option<RegelSoort>,
    /// Code van de nieuwe tegenrekening.
    pub tegenrekening: Option<String>,
    /// Nieuwe btw-code.
    pub btw_code: Option<String>,
    /// Aan (true) of uit (false) zetten.
    pub actief: Option<bool>,
    /// Velden om leeg te maken: omschrijving, omschrijving2, bedrag_van, bedrag_tot.
    pub wissen: Option<Vec<ImportregelVeld>>,
    /// Naam van het account in EB MCP, zoals status hem noemt. Verplicht zodra er meer dan één account in EB MCP staat.
    pub account: Option<String>,
}

/// What a rule is checked against: the tegenrekeningen and btw codes the form offers.
struct RegelContext {
    rekeningen: Vec<Keuze>,
    /// The codes the form offers today (`btw_aangeboden`).
    btw: Vec<BtwCode>,
    /// Whether the administration uses the extended btw codes; without them the form offers only the plain ones.
    uitgebreid: bool,
}

impl RegelContext {
    fn btw(&self, code: &str, soort: RegelSoort) -> Result<&BtwCode, String> {
        let code = code.trim();
        let fits = |b: &&BtwCode| b.code == "GEEN" || b.soort == soort.btw_soort();
        let allowed: Vec<&BtwCode> = self.btw.iter().filter(fits).collect();
        allowed
            .iter()
            .copied()
            .find(|b| b.code.eq_ignore_ascii_case(code))
            .ok_or_else(|| {
                format!(
                    "Btw-code {code:?} biedt het formulier vandaag niet aan bij {}{}. Kies uit: {}",
                    soort.naam(),
                    if self.uitgebreid {
                        ""
                    } else {
                        " (uitgebreide btw-codes staan uit in deze administratie)"
                    },
                    allowed
                        .iter()
                        .map(|b| b.code.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
    }

    fn btw_naam(&self, code: &str) -> String {
        match self.btw.iter().find(|b| b.code.eq_ignore_ascii_case(code)) {
            Some(b) => format!("{} ({})", b.code, b.naam),
            None => code.to_owned(),
        }
    }
}

/// One importregel, as the form holds it.
#[derive(Clone, Debug, PartialEq)]
struct Regel {
    omschrijving: Option<String>,
    omschrijving2: Option<String>,
    bij_af: BijAf,
    bedrag_van: Option<f64>,
    bedrag_tot: Option<f64>,
    soort: RegelSoort,
    tegenrekening: u64,
    btw_code: String,
    actief: bool,
    /// Kept as it was: the tools do not set a kostenplaats.
    kostenplaats: Option<u64>,
}

impl Regel {
    /// The form's own checks, before anything is asked.
    fn check(&self) -> Result<(), String> {
        let amount_set = |a: Option<f64>| a.is_some_and(|a| a != 0.0);
        if self.omschrijving.is_none()
            && !amount_set(self.bedrag_van)
            && !amount_set(self.bedrag_tot)
        {
            return Err("Een importregel heeft minstens één criterium nodig: een tekst in omschrijving, of bedrag_van of bedrag_tot (niet 0)".into());
        }
        if let Some(text) = &self.omschrijving {
            kort(text, 50, "omschrijving")?;
        }
        if let Some(text) = &self.omschrijving2 {
            kort(text, 50, "omschrijving2")?;
        }
        if let (Some(van), Some(tot)) = (self.bedrag_van, self.bedrag_tot)
            && van > tot
        {
            return Err(format!(
                "bedrag_van ({}) is groter dan bedrag_tot ({}): dan past er nooit een bankregel",
                write::euro(van),
                write::euro(tot)
            ));
        }
        Ok(())
    }

    /// The request body, with the keys e-Boekhouden's form sends. A field left empty is left out, as the form does.
    fn body(&self, id: Option<u64>) -> Value {
        let mut body = serde_json::Map::new();
        if let Some(id) = id {
            body.insert("id".into(), json!(id));
        }
        if let Some(text) = &self.omschrijving {
            body.insert("omschrijving".into(), json!(text));
        }
        if let Some(text) = &self.omschrijving2 {
            body.insert("omschrijving2".into(), json!(text));
        }
        body.insert("bijAf".into(), json!(self.bij_af.api()));
        if let Some(van) = self.bedrag_van {
            body.insert("bedragVan".into(), json!(van));
        }
        if let Some(tot) = self.bedrag_tot {
            body.insert("bedragTot".into(), json!(tot));
        }
        body.insert("mutatieSoortId".into(), json!(self.soort.nr()));
        body.insert("mutatieBtwCode".into(), json!(self.btw_code));
        body.insert("mutatieTegenrekeningId".into(), json!(self.tegenrekening));
        body.insert("actief".into(), json!(self.actief));
        if let Some(kostenplaats) = self.kostenplaats {
            body.insert("kostenplaatsId".into(), json!(kostenplaats));
        }
        Value::Object(body)
    }

    /// The rule in the words of the form, for the approval window.
    fn fields(&self, ctx: &RegelContext) -> Vec<(&'static str, String)> {
        let mut fields = Vec::new();
        if let Some(text) = &self.omschrijving {
            fields.push(("Omschrijving bevat", text.clone()));
        }
        if let Some(text) = &self.omschrijving2 {
            fields.push(("En bevat ook", text.clone()));
        }
        fields.push(("Soort bankregel", self.bij_af.api().to_owned()));
        if let Some(van) = self.bedrag_van {
            fields.push(("Bedrag vanaf", write::euro(van)));
        }
        if let Some(tot) = self.bedrag_tot {
            fields.push(("Bedrag tot en met", write::euro(tot)));
        }
        fields.push(("Boeken als", self.soort.naam().to_owned()));
        fields.push((
            "Tegenrekening",
            rekening_naam(&ctx.rekeningen, self.tegenrekening),
        ));
        fields.push(("Btw-code", ctx.btw_naam(&self.btw_code)));
        fields.push(("Actief", ja_nee(self.actief).to_owned()));
        fields
    }

    /// A short name for the rule in a title.
    fn naam(&self) -> String {
        let mut parts = Vec::new();
        if let Some(text) = &self.omschrijving {
            parts.push(format!("\"{text}\""));
        }
        if let Some(text) = &self.omschrijving2 {
            parts.push(format!("\"{text}\""));
        }
        match (self.bedrag_van, self.bedrag_tot) {
            (Some(van), Some(tot)) => parts.push(format!(
                "{} tot en met {}",
                write::euro(van),
                write::euro(tot)
            )),
            (Some(van), None) => parts.push(format!("vanaf {}", write::euro(van))),
            (None, Some(tot)) => parts.push(format!("tot en met {}", write::euro(tot))),
            (None, None) => {}
        }
        format!(
            "{} ({})",
            parts.join(" en "),
            self.bij_af.api().to_lowercase()
        )
    }

    /// A warning when the kind of bank line and the mutatiesoort do not go together.
    fn mismatch(&self) -> Option<approvals::Line> {
        (RegelSoort::voor(self.bij_af) != self.soort).then(|| {
            line(
                "Let op",
                format!(
                    "een {} wordt geboekt als {}; meestal hoort bij een bijschrijving Geld ontvangen en bij een afschrijving Geld uitgegeven",
                    self.bij_af.api().to_lowercase(),
                    self.soort.naam()
                ),
            )
        })
    }

    /// From `GET importregel/{id}`.
    fn from_api(value: &Value) -> Result<Self, String> {
        // Every value here goes back to e-Boekhouden on an edit: one that cannot be read stops the edit.
        let bad = |veld: &str| onleesbaar("deze importregel", veld);
        let text = |key: &str| -> Result<Option<String>, String> {
            optional(value.get(key), |v| v.as_str().map(str::to_owned))
                .map(|t| tekst(t.as_deref()))
                .map_err(|()| bad(key))
        };
        let amount = |key: &str| optional(value.get(key), read_f64).map_err(|()| bad(key));
        let number = |key: &str| value.get(key).and_then(read_u64).ok_or_else(|| bad(key));
        Ok(Self {
            omschrijving: text("omschrijving")?,
            omschrijving2: text("omschrijving2")?,
            bij_af: value
                .get("bijAf")
                .and_then(Value::as_str)
                .and_then(BijAf::from_api)
                .ok_or_else(|| bad("bijAf"))?,
            bedrag_van: amount("bedragVan")?,
            bedrag_tot: amount("bedragTot")?,
            soort: RegelSoort::from_nr(number("mutatieSoortId")?)
                .ok_or_else(|| bad("mutatieSoortId"))?,
            tegenrekening: number("mutatieTegenrekeningId")?,
            btw_code: text("mutatieBtwCode")?.ok_or_else(|| bad("mutatieBtwCode"))?,
            actief: value
                .get("actief")
                .and_then(read_bool)
                .ok_or_else(|| bad("actief"))?,
            kostenplaats: optional(value.get("kostenplaatsId"), read_u64)
                .map_err(|()| bad("kostenplaatsId"))?
                .filter(|id| *id > 0),
        })
    }
}

fn importregel_nieuw(args: &ImportregelToevoegenArgs, ctx: &RegelContext) -> Result<Built, String> {
    let soort = args.soort.unwrap_or_else(|| RegelSoort::voor(args.bij_af));
    let regel = Regel {
        omschrijving: tekst(args.omschrijving.as_deref()),
        omschrijving2: tekst(args.omschrijving2.as_deref()),
        bij_af: args.bij_af,
        bedrag_van: args
            .bedrag_van
            .map(|a| bedrag(a, "bedrag_van"))
            .transpose()?,
        bedrag_tot: args
            .bedrag_tot
            .map(|a| bedrag(a, "bedrag_tot"))
            .transpose()?,
        soort,
        tegenrekening: kies(&ctx.rekeningen, &args.tegenrekening, "tegenrekening")?.id,
        btw_code: ctx.btw(&args.btw_code, soort)?.code.clone(),
        actief: args.actief.unwrap_or(true),
        kostenplaats: None,
    };
    regel.check()?;
    let mut lines: Vec<approvals::Line> = regel
        .fields(ctx)
        .into_iter()
        .map(|(label, value)| line(label, value))
        .collect();
    lines.extend(regel.mismatch());
    lines.push(line(
        "Gevolg",
        if regel.actief {
            "bankregels die hierop passen, boekt e-Boekhouden voortaan bij het importeren zelf als mutatie"
        } else {
            "de regel staat uit en boekt niets tot hij aan wordt gezet"
        },
    ));
    Ok(Built {
        body: regel.body(None),
        title: format!("Importregel toevoegen: {}", regel.naam()),
        lines,
    })
}

fn importregel_gewijzigd(
    args: &ImportregelWijzigenArgs,
    now: &Value,
    ctx: &RegelContext,
) -> Result<Built, String> {
    let before = Regel::from_api(now)?;
    let mut regel = before.clone();
    let wissen = args.wissen.clone().unwrap_or_default();
    let given_and_cleared = |field: ImportregelVeld, given: bool| given && wissen.contains(&field);
    for (field, given) in [
        (ImportregelVeld::Omschrijving, args.omschrijving.is_some()),
        (ImportregelVeld::Omschrijving2, args.omschrijving2.is_some()),
        (ImportregelVeld::BedragVan, args.bedrag_van.is_some()),
        (ImportregelVeld::BedragTot, args.bedrag_tot.is_some()),
    ] {
        if given_and_cleared(field, given) {
            return Err(format!(
                "{} staat zowel bij de nieuwe waarden als bij wissen",
                serde_json::to_value(field)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default()
            ));
        }
    }
    if let Some(text) = &args.omschrijving {
        regel.omschrijving = tekst(Some(text));
    }
    if let Some(text) = &args.omschrijving2 {
        regel.omschrijving2 = tekst(Some(text));
    }
    if let Some(bij_af) = args.bij_af {
        regel.bij_af = bij_af;
    }
    if let Some(van) = args.bedrag_van {
        regel.bedrag_van = Some(bedrag(van, "bedrag_van")?);
    }
    if let Some(tot) = args.bedrag_tot {
        regel.bedrag_tot = Some(bedrag(tot, "bedrag_tot")?);
    }
    for field in &wissen {
        match field {
            ImportregelVeld::Omschrijving => regel.omschrijving = None,
            ImportregelVeld::Omschrijving2 => regel.omschrijving2 = None,
            ImportregelVeld::BedragVan => regel.bedrag_van = None,
            ImportregelVeld::BedragTot => regel.bedrag_tot = None,
        }
    }
    if let Some(soort) = args.soort {
        regel.soort = soort;
    }
    if let Some(code) = &args.tegenrekening {
        regel.tegenrekening = kies(&ctx.rekeningen, code, "tegenrekening")?.id;
    }
    if let Some(actief) = args.actief {
        regel.actief = actief;
    }
    match &args.btw_code {
        Some(code) => regel.btw_code = ctx.btw(code, regel.soort)?.code.clone(),
        None if regel.soort != before.soort => {
            // The form picks a new btw code when the soort changes; here the current one must still fit.
            regel.btw_code = ctx
                .btw(&regel.btw_code, regel.soort)
                .map_err(|e| {
                    format!(
                        "{e}. Geef met btw_code een code die bij {} past",
                        regel.soort.naam()
                    )
                })?
                .code
                .clone();
        }
        None => {}
    }
    regel.check()?;
    let mut lines = vec![line("Importregel", before.naam())];
    let changed = changes(&before.fields(ctx), &regel.fields(ctx));
    if changed.is_empty() {
        return Err("Er is niets gewijzigd: geef aan wat er anders moet".into());
    }
    lines.extend(changed);
    lines.extend(regel.mismatch());
    if regel.actief {
        lines.push(line(
            "Gevolg",
            "geldt voor bankregels die hierna geïmporteerd of opnieuw verwerkt worden; wat al geboekt is blijft staan",
        ));
    }
    let id = now.get("id").and_then(Value::as_u64).unwrap_or(args.id);
    Ok(Built {
        body: regel.body(Some(id)),
        title: format!("Importregel wijzigen: {}", before.naam()),
        lines,
    })
}

fn importregel_weg(now: &Value, ctx: &RegelContext) -> Result<Built, String> {
    let regel = Regel::from_api(now)?;
    let mut lines: Vec<approvals::Line> = regel
        .fields(ctx)
        .into_iter()
        .map(|(label, value)| line(label, value))
        .collect();
    lines.push(line(
        "Gevolg",
        "de importregel verdwijnt; wat hij al geboekt heeft blijft staan. Dit is niet terug te draaien.",
    ));
    Ok(Built {
        body: Value::Null,
        title: format!("Importregel verwijderen: {}", regel.naam()),
        lines,
    })
}

fn importregels_verwerken_built(niet_verwerkt: u64, regels: Option<u64>) -> Result<Built, String> {
    if niet_verwerkt == 0 {
        return Err(
            "Er zijn geen niet verwerkte bankregels: er valt niets opnieuw te verwerken".into(),
        );
    }
    let mut lines = vec![line("Niet verwerkte bankregels nu", niet_verwerkt)];
    if let Some(count) = regels {
        lines.push(line("Automatische importregels", count));
    }
    lines.push(line(
        "Gevolg",
        "e-Boekhouden verwerkt de niet verwerkte bankregels opnieuw automatisch, met onder meer de automatische importregels, en boekt wat het zo kan verwerken meteen als mutatie, allemaal in één keer",
    ));
    lines.push(line(
        "Vooraf te zien",
        "nee: EB MCP kan niet laten zien welke bankregels e-Boekhouden boekt, op welke rekening of voor welk bedrag. Na afloop staat het aantal geboekte mutaties in het resultaat. Bekijk de bankregels en importregels eerst met bankregels en importregels.",
    ));
    lines.push(line(
        "Terugdraaien",
        "kan alleen door elke geboekte mutatie apart te verwijderen",
    ));
    Ok(Built {
        body: Value::Null,
        title: "Niet verwerkte bankregels opnieuw automatisch verwerken".into(),
        lines,
    })
}

/// The setting e-Boekhouden's importregel form reads to offer the extended btw codes.
const UITGEBREIDE_BTW: &str = "setting/administratie.use-extended-btwcodes";

impl EbMcp {
    /// The tegenrekeningen and btw codes the importregel form offers: `btwinfo/info`, kept for today and to the
    /// plain codes unless the administration uses the extended ones (the form's `filterInfo` and `simpleExtended`).
    async fn regel_context(&self, run: &Run) -> Result<RegelContext, String> {
        let rekeningen = self.rekeningen_van(run, "VW,BAL,BTWRC").await?;
        let info = self.fetch(run, "btwinfo/info", Vec::new()).await?;
        let setting = self
            .fetch(
                run,
                UITGEBREIDE_BTW,
                vec![("acc".to_owned(), "false".to_owned())],
            )
            .await?;
        // The form: `ue && ue.value.toLocaleLowerCase() === 'true'`; no setting means off.
        let uitgebreid = setting
            .get("value")
            .and_then(Value::as_str)
            .is_some_and(|v| v.eq_ignore_ascii_case("true"));
        Ok(RegelContext {
            rekeningen,
            btw: btw_aangeboden(btw_codes(&info), today(), uitgebreid),
            uitgebreid,
        })
    }
}

#[tool_router(router = importregels_router, vis = "pub(super)")]
impl EbMcp {
    #[tool(
        description = "De automatische importregels: regels die geïmporteerde bankregels zelf als mutatie boeken. Per regel: id, bijAf (Bijschrijving of Afschrijving), omschrijving en omschrijving2 (tekst die in de bankregel moet voorkomen), bedragVan, bedragTot, mutatiesoort en tegenrekening. De bankregels zelf geeft bankregels.",
        annotations(title = "Importregels", read_only_hint = true)
    )]
    async fn importregels(
        &self,
        Parameters(args): Parameters<ListArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const PATH: &str = "importregel/gridtable";
        let run = match self.begin(&context, "importregels", &args, args.account.as_deref()) {
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
        description = "Eén automatische importregel, op id (uit importregels): de criteria, als welke mutatiesoort hij boekt, de tegenrekening met code en naam, de btw-code en of hij actief is.",
        annotations(title = "Importregel", read_only_hint = true)
    )]
    async fn importregel(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let run = match self.begin(&context, "importregel", &args, args.account.as_deref()) {
            Ok(run) => run,
            Err(answer) => return Ok(answer),
        };
        let path = format!("importregel/{}", args.id);
        let result = async {
            let regel = self.fetch(&run, &path, Vec::new()).await?;
            if regel.is_null() {
                return Err(format!("Er is geen importregel met id {}", args.id));
            }
            let rekeningen = self.rekeningen_van(&run, "VW,BAL,BTWRC").await?;
            let soort = regel
                .get("mutatieSoortId")
                .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()))
                .and_then(RegelSoort::from_nr)
                .map(RegelSoort::naam);
            let tegenrekening = regel
                .get("mutatieTegenrekeningId")
                .and_then(Value::as_u64)
                .map(|id| rekening_naam(&rekeningen, id));
            Ok(json!({"importregel": regel, "mutatiesoort": soort, "tegenrekening": tegenrekening}))
        }
        .await;
        Ok(self.end(run, &path, result))
    }

    #[tool(
        description = "Een automatische importregel aanmaken: bankregels waarvan de omschrijving een tekst bevat en/of waarvan het bedrag binnen grenzen valt, worden bij het importeren zelf geboekt als Geld ontvangen of Geld uitgegeven op de gekozen tegenrekening met de gekozen btw-code. Minstens één criterium is nodig (omschrijving, bedrag_van of bedrag_tot). Bestaande bankregels worden pas geboekt bij een nieuwe import of met importregels_verwerken. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet; de gebruiker keurt de regel eerst goed in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Importregel toevoegen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn importregel_toevoegen(
        &self,
        Parameters(args): Parameters<ImportregelToevoegenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "importregel_toevoegen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Importregel toegevoegd; data.id is het id",
            move |this, run, args| async move {
                let ctx = this.regel_context(&run).await?;
                let built = importregel_nieuw(&args, &ctx)?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change(&run, "POST", "importregel", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een automatische importregel wijzigen, op id (uit importregels). Wat je niet noemt blijft zoals het is; met wissen maak je omschrijving, omschrijving2, bedrag_van of bedrag_tot leeg. Werkt alleen als de gebruiker schrijven voor het account heeft aangezet. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Importregel wijzigen",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn importregel_wijzigen(
        &self,
        Parameters(args): Parameters<ImportregelWijzigenArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "importregel_wijzigen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Importregel gewijzigd",
            move |this, run, args| async move {
                let now = this
                    .fetch(&run, &format!("importregel/{}", args.id), Vec::new())
                    .await?;
                if now.is_null() {
                    return Err(Halt::Error(format!(
                        "Er is geen importregel met id {}",
                        args.id
                    )));
                }
                let ctx = this.regel_context(&run).await?;
                let built = importregel_gewijzigd(&args, &now, &ctx)?;
                this.approve(&run, TOOL, &built, false).await?;
                this.change_done(&run, "PUT", "importregel", Some(&built.body))
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Een automatische importregel verwijderen, op id (uit importregels). Mutaties die de regel al geboekt heeft blijven staan. Dit is niet terug te draaien. De gebruiker keurt dit goed in EB MCP, tenzij verwijderen voor het account zonder akkoord staat. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Importregel verwijderen",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn importregel_verwijderen(
        &self,
        Parameters(args): Parameters<IdArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "importregel_verwijderen";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Importregel verwijderd",
            move |this, run, args| async move {
                let now = this
                    .fetch(&run, &format!("importregel/{}", args.id), Vec::new())
                    .await?;
                if now.is_null() {
                    return Err(Halt::Error(format!(
                        "Er is geen importregel met id {}",
                        args.id
                    )));
                }
                let ctx = this.regel_context(&run).await?;
                let built = importregel_weg(&now, &ctx)?;
                this.approve(&run, TOOL, &built, true).await?;
                this.change_done(&run, "DELETE", &format!("importregel/{}", args.id), None)
                    .await
            },
        )
        .await
    }

    #[tool(
        description = "Alle niet verwerkte bankregels opnieuw automatisch verwerken, zoals de link Alle regels opnieuw automatisch verwerken in e-Boekhouden: e-Boekhouden verwerkt alle niet verwerkte bankregels opnieuw automatisch, met onder meer de automatische importregels, en boekt wat het zo kan verwerken meteen als mutatie, allemaal in één keer. Eén regel kiezen kan niet, en EB MCP kan vooraf niet laten zien welke mutaties het worden; het resultaat zegt hoeveel er geboekt zijn. Bekijk eerst bankregels en importregels. De gebruiker moet dit altijd goedkeuren in EB MCP. Vraagt de wijziging akkoord, dan komt het antwoord meteen terug met status wacht en een voorstel_id: de wijziging gaat pas door als de gebruiker goedkeurt, en akkoord_status geeft later de uitkomst. Ga intussen gerust verder met ander werk.",
        annotations(
            title = "Importregels opnieuw verwerken",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn importregels_verwerken(
        &self,
        Parameters(args): Parameters<AccountArgs>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        const TOOL: &str = "importregels_verwerken";
        let account = args.account.clone();
        self.write_tool(
            &context,
            TOOL,
            account.as_deref(),
            args,
            "Bankregels opnieuw verwerkt; data.countMutatie is het aantal geboekte mutaties, data.countErrors het aantal dat niet lukte",
            move |this, run, _args| async move {
                let count = this
                    .fetch(
                        &run,
                        "import/countImport",
                        vec![("countNietVerwerkt".to_owned(), "true".to_owned())],
                    )
                    .await?;
                let niet_verwerkt = count.as_u64().ok_or_else(|| {
                    Halt::Error("e-Boekhouden gaf het aantal niet verwerkte bankregels in een onbekende vorm terug".into())
                })?;
                // Only the count of the rules is needed: the grid's rowCount.
                let regels = this
                    .fetch(&run, "importregel/gridtable", grid(Some(1), None)?)
                    .await?;
                let aantal = regels.get("rowCount").and_then(Value::as_u64);
                let built = importregels_verwerken_built(niet_verwerkt, aantal)?;
                this.approve(&run, TOOL, &built, true).await?;
                this.change(&run, "POST", "import/verwerk", None).await
            },
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> RegelContext {
        RegelContext {
            rekeningen: keuzes(&json!([
                {"id": 45, "code": "4550", "omschrijving": "Bankkosten"},
                {"id": 80, "code": "8000", "omschrijving": "Omzet"},
                {"id": 11, "code": "1010", "omschrijving": "Bank"}
            ])),
            btw: btw_codes(&json!([
                {"code": "HOOG_INK_21", "omschrijving": "Btw hoog 21%", "percentage": 21, "soort": "INK"},
                {"code": "HOOG_VERK_21", "omschrijving": "Btw hoog 21%", "percentage": 21, "soort": "VERK"},
                {"code": "GEEN", "omschrijving": "Geen btw", "percentage": 0, "soort": ""}
            ])),
            uitgebreid: true,
        }
    }

    fn toevoegen() -> ImportregelToevoegenArgs {
        ImportregelToevoegenArgs {
            omschrijving: Some(" Triodos ".into()),
            omschrijving2: None,
            bij_af: BijAf::Afschrijving,
            bedrag_van: None,
            bedrag_tot: None,
            soort: None,
            tegenrekening: "4550".into(),
            btw_code: "geen".into(),
            actief: None,
            account: None,
        }
    }

    fn label_values(built: &Built) -> Vec<(String, String)> {
        built
            .lines
            .iter()
            .map(|l| (l.label.clone(), l.value.clone()))
            .collect()
    }

    #[test]
    fn makes_a_rule_as_e_boekhoudens_form_does() {
        let built = importregel_nieuw(&toevoegen(), &ctx()).unwrap();
        assert_eq!(
            built.body,
            json!({"omschrijving": "Triodos", "bijAf": "Afschrijving", "mutatieSoortId": 6, "mutatieBtwCode": "GEEN",
                   "mutatieTegenrekeningId": 45, "actief": true})
        );
        assert_eq!(
            built.title,
            "Importregel toevoegen: \"Triodos\" (afschrijving)"
        );
        let lines = label_values(&built);
        assert!(lines.contains(&("Boeken als".into(), "Geld uitgegeven".into())));
        assert!(lines.contains(&("Tegenrekening".into(), "4550 Bankkosten".into())));
        assert!(lines.contains(&("Btw-code".into(), "GEEN (Geen btw)".into())));
        assert!(!lines.iter().any(|(l, _)| l == "Let op"));
    }

    #[test]
    fn a_rule_on_amounts_alone_sends_them_rounded() {
        let mut args = toevoegen();
        args.omschrijving = Some("  ".into());
        args.omschrijving2 = Some("huur".into());
        args.bij_af = BijAf::Bijschrijving;
        args.bedrag_van = Some(99.999);
        args.bedrag_tot = Some(150.0);
        args.soort = Some(RegelSoort::GeldUitgegeven);
        args.btw_code = "HOOG_INK_21".into();
        args.actief = Some(false);
        let built = importregel_nieuw(&args, &ctx()).unwrap();
        assert_eq!(
            built.body,
            json!({"omschrijving2": "huur", "bijAf": "Bijschrijving", "bedragVan": 100.0, "bedragTot": 150.0, "mutatieSoortId": 6,
                   "mutatieBtwCode": "HOOG_INK_21", "mutatieTegenrekeningId": 45, "actief": false})
        );
        let lines = label_values(&built);
        assert!(
            lines
                .iter()
                .any(|(l, v)| l == "Let op" && v.contains("bijschrijving"))
        );
        assert!(lines.contains(&("Bedrag vanaf".into(), "100,00".into())));
        assert!(
            lines
                .iter()
                .any(|(l, v)| l == "Gevolg" && v.contains("staat uit"))
        );
    }

    #[test]
    fn refuses_what_the_form_would_refuse() {
        let ctx = ctx();
        let mut args = toevoegen();
        args.omschrijving = None;
        args.bedrag_van = Some(0.0);
        assert!(
            importregel_nieuw(&args, &ctx)
                .unwrap_err()
                .contains("criterium")
        );
        let mut args = toevoegen();
        args.omschrijving = Some("x".repeat(51));
        assert!(
            importregel_nieuw(&args, &ctx)
                .unwrap_err()
                .contains("50 tekens")
        );
        let mut args = toevoegen();
        args.bedrag_van = Some(10.0);
        args.bedrag_tot = Some(5.0);
        assert!(
            importregel_nieuw(&args, &ctx)
                .unwrap_err()
                .contains("groter")
        );
        let mut args = toevoegen();
        args.tegenrekening = "9999".into();
        assert!(importregel_nieuw(&args, &ctx).unwrap_err().contains("9999"));
        // A sales code does not fit money going out.
        let mut args = toevoegen();
        args.btw_code = "HOOG_VERK_21".into();
        let error = importregel_nieuw(&args, &ctx).unwrap_err();
        assert!(
            error.contains("HOOG_INK_21") && error.contains("GEEN"),
            "{error}"
        );
        let mut args = toevoegen();
        args.bedrag_tot = Some(f64::INFINITY);
        assert!(importregel_nieuw(&args, &ctx).is_err());
    }

    fn now() -> Value {
        json!({"id": 31, "omschrijving": "Triodos", "omschrijving2": null, "bijAf": "Afschrijving", "bedragVan": 5, "bedragTot": null,
               "mutatieSoortId": 6, "mutatieBtwCode": "GEEN", "mutatieTegenrekeningId": 45, "actief": true, "kostenplaatsId": 7})
    }

    fn wijzigen() -> ImportregelWijzigenArgs {
        ImportregelWijzigenArgs {
            id: 31,
            omschrijving: None,
            omschrijving2: None,
            bij_af: None,
            bedrag_van: None,
            bedrag_tot: None,
            soort: None,
            tegenrekening: None,
            btw_code: None,
            actief: None,
            wissen: None,
            account: None,
        }
    }

    #[test]
    fn an_edit_sends_the_whole_rule_back_with_only_the_named_change() {
        let mut args = wijzigen();
        args.tegenrekening = Some("1010".into());
        args.wissen = Some(vec![ImportregelVeld::BedragVan]);
        let built = importregel_gewijzigd(&args, &now(), &ctx()).unwrap();
        assert_eq!(
            built.body,
            json!({"id": 31, "omschrijving": "Triodos", "bijAf": "Afschrijving", "mutatieSoortId": 6, "mutatieBtwCode": "GEEN",
                   "mutatieTegenrekeningId": 11, "actief": true, "kostenplaatsId": 7})
        );
        let lines = label_values(&built);
        assert_eq!(
            lines[0],
            (
                "Importregel".into(),
                "\"Triodos\" en vanaf 5,00 (afschrijving)".into()
            )
        );
        assert!(lines.contains(&("Tegenrekening was".into(), "4550 Bankkosten".into())));
        assert!(lines.contains(&("Tegenrekening wordt".into(), "1010 Bank".into())));
        assert!(lines.contains(&("Bedrag vanaf was".into(), "5,00".into())));
        assert!(lines.contains(&("Bedrag vanaf wordt".into(), "(leeg)".into())));
        assert!(!lines.iter().any(|(l, _)| l.starts_with("Btw")));
    }

    #[test]
    fn an_edit_of_the_soort_needs_a_btw_code_that_fits() {
        let mut args = wijzigen();
        args.soort = Some(RegelSoort::GeldOntvangen);
        // GEEN fits both.
        assert!(importregel_gewijzigd(&args, &now(), &ctx()).is_ok());
        let mut now = now();
        now["mutatieBtwCode"] = json!("HOOG_INK_21");
        let error = importregel_gewijzigd(&args, &now, &ctx()).unwrap_err();
        assert!(error.contains("btw_code"), "{error}");
        args.btw_code = Some("HOOG_VERK_21".into());
        let built = importregel_gewijzigd(&args, &now, &ctx()).unwrap();
        assert_eq!(built.body["mutatieSoortId"], 5);
        assert_eq!(built.body["mutatieBtwCode"], "HOOG_VERK_21");
    }

    #[test]
    fn an_edit_refuses_nothing_to_do_and_contradictions() {
        assert!(
            importregel_gewijzigd(&wijzigen(), &now(), &ctx())
                .unwrap_err()
                .contains("niets gewijzigd")
        );
        let mut args = wijzigen();
        args.omschrijving = Some("nieuw".into());
        args.wissen = Some(vec![ImportregelVeld::Omschrijving]);
        assert!(
            importregel_gewijzigd(&args, &now(), &ctx())
                .unwrap_err()
                .contains("wissen")
        );
        // Emptying the only text while no amount is left leaves no criterion.
        let mut args = wijzigen();
        args.wissen = Some(vec![
            ImportregelVeld::Omschrijving,
            ImportregelVeld::BedragVan,
        ]);
        assert!(
            importregel_gewijzigd(&args, &now(), &ctx())
                .unwrap_err()
                .contains("criterium")
        );
        assert!(importregel_gewijzigd(&wijzigen(), &json!({"id": 31}), &ctx()).is_err());
    }

    #[test]
    fn a_rule_is_read_with_string_numbers_too() {
        let read = json!({"bijAf": "bijschrijving", "mutatieSoortId": "5", "mutatieTegenrekeningId": "80", "mutatieBtwCode": "HOOG_VERK_21",
                          "omschrijving": "", "bedragVan": "12.50", "kostenplaatsId": 0, "actief": 0});
        let regel = Regel::from_api(&read).unwrap();
        assert_eq!(regel.soort, RegelSoort::GeldOntvangen);
        assert_eq!(regel.tegenrekening, 80);
        assert_eq!(regel.omschrijving, None);
        assert_eq!(regel.bedrag_van, Some(12.5));
        assert_eq!(regel.kostenplaats, None);
        // A rule that is switched off stays off when it is edited.
        assert!(!regel.actief);
        let mut aan = read.clone();
        aan["actief"] = json!(1);
        assert!(Regel::from_api(&aan).unwrap().actief);
        aan["actief"] = json!("true");
        assert!(Regel::from_api(&aan).unwrap().actief);
    }

    #[test]
    fn a_rule_with_a_value_it_cannot_read_is_not_edited() {
        let good = now();
        assert!(Regel::from_api(&good).is_ok());
        for (key, value) in [
            ("actief", Value::Null),
            ("actief", json!(2)),
            ("actief", json!("ja")),
            ("mutatieBtwCode", Value::Null),
            ("mutatieBtwCode", json!("")),
            ("bedragVan", json!("vijf")),
            ("bedragTot", json!({"x": 1})),
            ("omschrijving", json!(12)),
            ("kostenplaatsId", json!("geen")),
            ("mutatieTegenrekeningId", Value::Null),
        ] {
            let mut read = good.clone();
            read[key] = value.clone();
            let error = Regel::from_api(&read).unwrap_err();
            assert!(
                error.contains(key) && error.contains("onbekende vorm"),
                "{key} {value}: {error}"
            );
            // And so the edit is refused before anything is asked.
            let mut args = wijzigen();
            args.omschrijving = Some("nieuw".into());
            assert!(importregel_gewijzigd(&args, &read, &ctx()).is_err());
        }
        let mut read = good.clone();
        read.as_object_mut().unwrap().remove("actief");
        assert!(Regel::from_api(&read).unwrap_err().contains("actief"));
    }

    #[test]
    fn an_edit_keeps_a_switched_off_rule_off() {
        let mut now = now();
        now["actief"] = json!(0);
        let mut args = wijzigen();
        args.omschrijving = Some("Triodos Bank".into());
        let built = importregel_gewijzigd(&args, &now, &ctx()).unwrap();
        assert_eq!(built.body["actief"], false);
        assert!(
            !label_values(&built)
                .iter()
                .any(|(l, _)| l.starts_with("Actief"))
        );
    }

    #[test]
    fn offers_the_btw_codes_the_form_offers_today() {
        let info = btw_codes(&json!([
            {"code": "HOOG_INK_21", "omschrijving": "Btw hoog 21%", "soort": "INK", "simpleExtended": "S", "begindatum": {"day": 1, "month": 1, "year": 2019}, "einddatum": null},
            {"code": "LAAG_INK_6", "omschrijving": "Btw laag 6%", "soort": "INK", "simpleExtended": "S", "begindatum": null, "einddatum": {"day": 1, "month": 1, "year": 2019}},
            {"code": "VERL_INK", "omschrijving": "Verlegd", "soort": "INK", "simpleExtended": "E", "begindatum": null, "einddatum": null},
            {"code": "NIEUW_INK", "omschrijving": "Nieuw", "soort": "INK", "simpleExtended": "S", "begindatum": {"day": 12, "month": 11, "year": 2026}, "einddatum": null},
            {"code": "GEEN", "omschrijving": "Geen btw", "soort": "", "simpleExtended": "S"}
        ]));
        let today = (2026, 11, 12);
        let codes = |uitgebreid: bool| -> Vec<String> {
            btw_aangeboden(info.clone(), today, uitgebreid)
                .into_iter()
                .map(|c| c.code)
                .collect()
        };
        assert_eq!(codes(false), vec!["HOOG_INK_21", "GEEN"]);
        assert_eq!(codes(true), vec!["HOOG_INK_21", "VERL_INK", "GEEN"]);
        let ctx = RegelContext {
            rekeningen: ctx().rekeningen,
            btw: btw_aangeboden(info, today, false),
            uitgebreid: false,
        };
        let mut args = toevoegen();
        args.btw_code = "VERL_INK".into();
        let error = importregel_nieuw(&args, &ctx).unwrap_err();
        assert!(error.contains("uitgebreide btw-codes staan uit"), "{error}");
        args.btw_code = "HOOG_INK_21".into();
        assert!(importregel_nieuw(&args, &ctx).is_ok());
    }

    #[test]
    fn deleting_and_running_say_what_happens() {
        let built = importregel_weg(&now(), &ctx()).unwrap();
        assert_eq!(built.body, Value::Null);
        assert_eq!(
            built.title,
            "Importregel verwijderen: \"Triodos\" en vanaf 5,00 (afschrijving)"
        );
        assert!(
            built
                .lines
                .iter()
                .any(|l| l.label == "Gevolg" && l.value.contains("niet terug te draaien"))
        );

        let built = importregels_verwerken_built(13, Some(4)).unwrap();
        assert_eq!(built.body, Value::Null);
        assert_eq!(
            built.title,
            "Niet verwerkte bankregels opnieuw automatisch verwerken"
        );
        let lines = label_values(&built);
        assert!(lines.contains(&("Niet verwerkte bankregels nu".into(), "13".into())));
        assert!(
            lines
                .iter()
                .any(|(l, v)| l == "Vooraf te zien" && v.starts_with("nee"))
        );
        assert!(!lines.iter().any(|(_, v)| v.contains("open post")));
        assert!(lines.contains(&("Automatische importregels".into(), "4".into())));
        assert!(
            lines
                .iter()
                .any(|(l, v)| l == "Gevolg" && v.contains("in één keer"))
        );
        assert!(
            importregels_verwerken_built(0, Some(4))
                .unwrap_err()
                .contains("geen niet verwerkte")
        );
    }
}
