//! A bank or card statement as a file with one line per row (csv), read by EB MCP itself before anything goes to
//! e-Boekhouden: which columns hold the date, the description and the amount, from which line it starts, and
//! which lines stay out. The user decides that on Akkoord (the sheet "Afschrift controleren"); an AI client may
//! only propose rules. What goes out is what `Statement::chosen` gives: nothing else of the file.
//!
//! e-Boekhouden's own upload screen knows "Starten vanaf regel" and no way to leave single lines out, which is
//! why the choice is made here.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::approvals::{Line, line};

/// Year, month, day.
pub type Date = (i32, u32, u32);

/// The most lines a statement may have: more than a year of a busy account, and still quick to show.
pub const MAX_ROWS: usize = 20_000;
/// The biggest file read. e-Boekhouden's own screens take 5 MB.
pub const MAX_BYTES: u64 = 5120 * 1024;

/// One record of the file with the line it starts on, counted as an editor counts them (1 is the first).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Raw {
    pub line: usize,
    pub cells: Vec<String>,
}

/// The file as records: no meaning given to any column yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub delimiter: char,
    /// The names of the columns: the first record when it is a header, else "Kolom 1", "Kolom 2".
    pub header: Vec<String>,
    /// The line of the header, when the file has one.
    pub header_line: Option<usize>,
    /// Every record with something in it, the header included.
    pub records: Vec<Raw>,
    /// Lines without anything on them, which are skipped without a word.
    pub empty: usize,
}

/// Text of a file: UTF-8, with or without a byte order mark, else read as Windows-1252, which is what Dutch
/// banks' older exports are.
pub fn decode(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => bytes.iter().map(|&b| windows_1252(b)).collect(),
    }
}

fn windows_1252(byte: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž',
        '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '\u{2013}', '\u{2014}', '˜', '™', 'š', '›',
        'œ', '\u{9d}', 'ž', 'Ÿ',
    ];
    match byte {
        0x80..=0x9f => HIGH[(byte - 0x80) as usize],
        other => other as char,
    }
}

/// The delimiter of a csv: the one of comma, semicolon and tab that the first line with text has most of,
/// outside quotes.
fn delimiter_of(text: &str) -> char {
    let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let mut counts = [(',', 0usize), (';', 0), ('\t', 0)];
    let mut quoted = false;
    for c in first.chars() {
        if c == '"' {
            quoted = !quoted;
        } else if !quoted {
            for (d, n) in &mut counts {
                if *d == c {
                    *n += 1;
                }
            }
        }
    }
    // A semicolon wins a tie with a comma: Dutch amounts carry commas of their own.
    let best = counts.iter().map(|(_, n)| *n).max().unwrap_or(0);
    [';', '\t', ',']
        .into_iter()
        .find(|d| counts.iter().any(|(c, n)| c == d && *n == best))
        .unwrap_or(',')
}

impl Table {
    /// Reads a csv: quoted cells may hold the delimiter, a doubled quote and line breaks.
    pub fn parse(text: &str) -> Result<Self, String> {
        let delimiter = delimiter_of(text);
        let mut records: Vec<Raw> = Vec::new();
        let mut empty = 0;
        let (mut cells, mut cell) = (Vec::new(), String::new());
        let (mut quoted, mut line, mut start, mut any) = (false, 1usize, 1usize, false);
        let mut chars = text.chars().peekable();
        let mut end_record = |cells: &mut Vec<String>,
                              cell: &mut String,
                              start: usize,
                              any: bool,
                              empty: &mut usize| {
            cells.push(std::mem::take(cell));
            let record = std::mem::take(cells);
            if any && record.iter().any(|c| !c.trim().is_empty()) {
                records.push(Raw {
                    line: start,
                    cells: record,
                });
            } else {
                *empty += 1;
            }
        };
        while let Some(c) = chars.next() {
            match c {
                '"' if quoted && chars.peek() == Some(&'"') => {
                    chars.next();
                    cell.push('"');
                }
                '"' => {
                    quoted = !quoted;
                    any = true;
                }
                '\r' if !quoted => {}
                '\n' if !quoted => {
                    end_record(&mut cells, &mut cell, start, any, &mut empty);
                    line += 1;
                    start = line;
                    any = false;
                }
                '\n' => {
                    line += 1;
                    cell.push('\n');
                }
                c if c == delimiter && !quoted => {
                    cells.push(std::mem::take(&mut cell));
                    any = true;
                }
                c => {
                    cell.push(c);
                    any = true;
                }
            }
        }
        if quoted {
            return Err("Het bestand heeft een aanhalingsteken dat niet sluit; het is niet als csv te lezen".into());
        }
        if any || !cell.is_empty() || !cells.is_empty() {
            end_record(&mut cells, &mut cell, start, any, &mut empty);
        }
        if records.is_empty() {
            return Err("Het bestand heeft geen regels".into());
        }
        if records.len() > MAX_ROWS {
            return Err(format!(
                "Het bestand heeft meer dan {MAX_ROWS} regels; splits het in delen"
            ));
        }
        let width = records.iter().map(|r| r.cells.len()).max().unwrap_or(0);
        if width < 2 {
            return Err(
                "Het bestand heeft maar één kolom: het is geen csv met komma, puntkomma of tab"
                    .into(),
            );
        }
        // A header is a first record without any amount or date in it.
        let first = &records[0];
        let is_header = !first
            .cells
            .iter()
            .any(|c| amount(c).is_some() && !c.trim().is_empty() || date(c).is_some());
        let header = (0..width)
            .map(|i| {
                let name = first.cells.get(i).map(|c| c.trim()).unwrap_or("");
                if is_header && !name.is_empty() {
                    name.to_owned()
                } else {
                    format!("Kolom {}", i + 1)
                }
            })
            .collect();
        Ok(Self {
            delimiter,
            header,
            header_line: is_header.then_some(first.line),
            records,
            empty,
        })
    }

    pub fn delimiter_name(&self) -> &'static str {
        match self.delimiter {
            ';' => "puntkomma",
            '\t' => "tab",
            _ => "komma",
        }
    }
}

/// A date as banks write it: 2026-11-06, 06-11-2026, 06/11/2026, 6-11-2026 or 20261106, with a time behind it
/// or not. Day before month when the year is last.
pub fn date(text: &str) -> Option<Date> {
    let text = text.trim().trim_start_matches('\'');
    let day = text.split(['T', ' ']).next()?;
    let valid = |y: i32, m: u32, d: u32| {
        ((1900..=2200).contains(&y) && chrono::NaiveDate::from_ymd_opt(y, m, d).is_some())
            .then_some((y, m, d))
    };
    if day.len() == 8 && day.bytes().all(|b| b.is_ascii_digit()) {
        return valid(
            day[..4].parse().ok()?,
            day[4..6].parse().ok()?,
            day[6..].parse().ok()?,
        );
    }
    let parts: Vec<&str> = day.split(['-', '/', '.']).collect();
    let [a, b, c] = parts.as_slice() else {
        return None;
    };
    if !parts
        .iter()
        .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    if a.len() == 4 {
        valid(a.parse().ok()?, b.parse().ok()?, c.parse().ok()?)
    } else if c.len() == 4 {
        valid(c.parse().ok()?, b.parse().ok()?, a.parse().ok()?)
    } else {
        None
    }
}

pub fn iso((y, m, d): Date) -> String {
    format!("{y:04}-{m:02}-{d:02}")
}

pub fn dutch((y, m, d): Date) -> String {
    format!("{d:02}-{m:02}-{y:04}")
}

/// An amount in cents, as banks write it: "-24,00", "2.000,04", "1,234.56", "12.5", "€ 3,10", "3,10-", "+5".
/// The last comma or point is the decimal mark when one or two digits follow it; the others group thousands.
pub fn amount(text: &str) -> Option<i64> {
    let mut text = text
        .trim()
        .trim_start_matches('\'')
        .replace(['€', ' ', '\u{a0}'], "");
    if text.is_empty() {
        return None;
    }
    let mut negative = false;
    if let Some(rest) = text.strip_suffix('-') {
        negative = true;
        text = rest.to_owned();
    }
    if let Some(rest) = text.strip_prefix('-') {
        negative = !negative;
        text = rest.to_owned();
    } else if let Some(rest) = text.strip_prefix('+') {
        text = rest.to_owned();
    }
    if text.is_empty()
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b',' || b == b'.')
    {
        return None;
    }
    let mark = text.rfind([',', '.']);
    let (whole, cents) = match mark {
        Some(i) if (1..=2).contains(&(text.len() - i - 1)) => {
            let decimals = &text[i + 1..];
            let cents: i64 = decimals.parse().ok()?;
            (
                &text[..i],
                if decimals.len() == 1 {
                    cents * 10
                } else {
                    cents
                },
            )
        }
        // A mark with three digits behind it groups thousands; with more, or none, it is no amount.
        Some(i) if text.len() - i - 1 != 3 => return None,
        _ => (text.as_str(), 0),
    };
    // The decimal mark is not also the mark between thousands: "12,345,6" is no amount.
    if let Some(i) = mark.filter(|i| text.len() - i - 1 != 3)
        && whole.contains(&text[i..=i])
    {
        return None;
    }
    let digits: String = whole.chars().filter(char::is_ascii_digit).collect();
    if digits.is_empty() || digits.len() > 12 {
        return None;
    }
    // Groups of thousands are three digits wide; "1.2,50" is not an amount.
    if whole.contains([',', '.'])
        && !whole.split([',', '.']).enumerate().all(|(i, part)| {
            if i == 0 {
                (1..=3).contains(&part.len())
            } else {
                part.len() == 3
            }
        })
    {
        return None;
    }
    let value = digits.parse::<i64>().ok()? * 100 + cents;
    Some(if negative { -value } else { value })
}

/// Cents as "1.234,56" or "-24,00".
pub fn euro(cents: i64) -> String {
    let (sign, cents) = (if cents < 0 { "-" } else { "" }, cents.unsigned_abs());
    let whole = (cents / 100).to_string();
    let mut grouped = String::new();
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            grouped.push('.');
        }
        grouped.push(c);
    }
    format!("{sign}{grouped},{:02}", cents % 100)
}

/// Which columns of the file mean what, and where the lines begin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mapping {
    /// Indexes into `Table::header`.
    pub date: usize,
    pub description: usize,
    pub amount: usize,
    /// The first line that counts ("Starten vanaf regel").
    pub start: usize,
}

impl Mapping {
    /// The best guess from the names of the columns and what stands in them.
    pub fn guess(table: &Table) -> Result<Self, String> {
        let start = match table.header_line {
            Some(header) => table
                .records
                .iter()
                .map(|r| r.line)
                .find(|&l| l > header)
                .unwrap_or(header + 1),
            None => table.records[0].line,
        };
        let data: Vec<&Raw> = table.records.iter().filter(|r| r.line >= start).collect();
        let share = |column: usize, is: &dyn Fn(&str) -> bool| -> f64 {
            if data.is_empty() {
                return 0.0;
            }
            data.iter()
                .filter(|r| r.cells.get(column).is_some_and(|c| is(c)))
                .count() as f64
                / data.len() as f64
        };
        let name = |column: usize| table.header[column].to_lowercase();
        let columns = 0..table.header.len();
        let pick = |score: &dyn Fn(usize) -> f64| -> Option<usize> {
            columns
                .clone()
                .map(|c| (c, score(c)))
                .filter(|(_, s)| *s > 0.0)
                .fold(None, |best: Option<(usize, f64)>, (c, s)| match best {
                    Some((_, b)) if b >= s => best,
                    _ => Some((c, s)),
                })
                .map(|(c, _)| c)
        };
        let date_column = pick(&|c| {
            let share = share(c, &|t| date(t).is_some());
            if share < 0.8 {
                return 0.0;
            }
            let n = name(c);
            share
                + if n.contains("transactie") { 0.3 } else { 0.0 }
                + if n.contains("datum") || n.contains("date") {
                    0.2
                } else {
                    0.0
                }
        })
        .ok_or("EB MCP vindt in dit bestand geen kolom met een datum")?;
        let amount_column = pick(&|c| {
            if c == date_column {
                return 0.0;
            }
            // A column of whole numbers without a mark is more likely a number than money.
            let money = share(c, &|t| amount(t).is_some() && t.contains([',', '.']));
            if money < 0.8 {
                return 0.0;
            }
            let n = name(c);
            money
                + if n.contains("eur") { 0.4 } else { 0.0 }
                + if n.contains("bedrag") || n.contains("amount") {
                    0.2
                } else {
                    0.0
                }
                - if n.contains("koers")
                    || n.contains("saldo")
                    || n.contains("rate")
                    || n.contains("balance")
                {
                    0.6
                } else {
                    0.0
                }
        })
        .ok_or("EB MCP vindt in dit bestand geen kolom met een bedrag")?;
        let description_column = pick(&|c| {
            if c == date_column || c == amount_column {
                return 0.0;
            }
            let n = name(c);
            let named =
                n.contains("omschrijving") || n.contains("description") || n.contains("mededeling");
            let text = share(c, &|t| {
                !t.trim().is_empty() && amount(t).is_none() && date(t).is_none()
            });
            let length = data
                .iter()
                .filter_map(|r| r.cells.get(c))
                .map(|t| t.trim().chars().count())
                .sum::<usize>() as f64
                / data.len().max(1) as f64;
            if text < 0.5 {
                return 0.0;
            }
            text + (length / 100.0).min(0.5) + if named { 1.0 } else { 0.0 }
        })
        .ok_or("EB MCP vindt in dit bestand geen kolom met een omschrijving")?;
        Ok(Self {
            date: date_column,
            description: description_column,
            amount: amount_column,
            start,
        })
    }
}

/// Who put a rule there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum By {
    /// The AI client, as a proposal.
    Client,
    User,
}

/// What a rule looks at.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Test {
    /// The description holds this text, whatever the case.
    Contains { text: String },
    /// A column holds exactly this value (a card number, a name), whatever the case.
    Column { column: usize, value: String },
    /// Dated on or before this day (already booked up to here).
    Until { date: Date },
    /// Dated on or after this day.
    From { date: Date },
    /// Exactly this amount, in cents.
    Amount { cents: i64 },
    /// Money coming in.
    Incoming,
    /// Money going out.
    Outgoing,
    /// Two lines of the same day that cancel each other: one in, one out, the same amount.
    Cancelling,
}

/// One rule that leaves lines out. Off: it is kept, but does nothing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    #[serde(flatten)]
    pub test: Test,
    pub on: bool,
    pub by: By,
    /// Why, in the proposer's words ("al geboekt"); shown under the rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

impl Rule {
    /// The rule in the words of the sheet.
    pub fn label(&self, header: &[String]) -> String {
        match &self.test {
            Test::Contains { text } => format!("Omschrijving bevat {text}"),
            Test::Column { column, value } => format!(
                "{} is {value}",
                header.get(*column).map(String::as_str).unwrap_or("Kolom")
            ),
            Test::Until { date } => format!("Datum t/m {}", dutch(*date)),
            Test::From { date } => format!("Datum vanaf {}", dutch(*date)),
            Test::Amount { cents } => format!("Bedrag is {}", euro(*cents)),
            Test::Incoming => "Bedrag bij".to_owned(),
            Test::Outgoing => "Bedrag af".to_owned(),
            Test::Cancelling => "Valt weg tegen een regel van dezelfde dag".to_owned(),
        }
    }
}

/// Why a line does not go along.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Out {
    /// Before "Starten vanaf regel", the header included.
    BeforeStart,
    /// No date or amount to read in the chosen columns: it cannot be read in, whatever the user ticks.
    Unreadable { what: String },
    /// A rule; its place in `Statement::rules`.
    Rule { rule: usize },
    /// The user unticked it.
    User,
}

/// One line of the file as the sheet shows it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub line: usize,
    pub cells: Vec<String>,
    /// ISO.
    pub date: Option<String>,
    pub description: String,
    pub cents: Option<i64>,
    /// Goes to e-Boekhouden.
    pub included: bool,
    pub out: Option<Out>,
    /// The rule that would leave it out, when the user ticked it all the same.
    pub overruled: Option<usize>,
}

/// What the sheet shows at its foot and the proposal in its lines.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    /// Lines from the start line on that can be read.
    pub lines: usize,
    pub included: usize,
    pub skipped: usize,
    pub unreadable: usize,
    /// The sum of what goes along, in cents.
    pub cents: i64,
    /// ISO, of what goes along.
    pub first: Option<String>,
    pub last: Option<String>,
    /// Lines each rule leaves out, by its place in `rules`.
    pub per_rule: Vec<usize>,
}

/// A statement that waits on Akkoord: the file, how it is read and what stays out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Statement {
    pub file_name: String,
    pub table: Table,
    pub mapping: Mapping,
    pub rules: Vec<Rule>,
    /// What the user ticked or unticked by hand, by line: this goes above any rule.
    pub overrides: BTreeMap<usize, bool>,
    /// Let e-Boekhouden leave out what it has already ("ontdubbelen").
    pub dedupe: bool,
    /// The accounts e-Boekhouden offers for a statement, and the one it is read in on (an index into them).
    pub ledgers: Vec<Ledger>,
    pub ledger: usize,
}

/// A grootboekrekening a statement can be read in on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ledger {
    pub id: u64,
    pub code: String,
    pub name: String,
}

impl Ledger {
    pub fn label(&self) -> String {
        format!("{} {}", self.code, self.name)
    }
}

/// What a proposal says of its statement without the lines: enough for the queue to know there is a sheet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub file_name: String,
    pub lines: usize,
    pub included: usize,
}

/// A rule as the sheet lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleView {
    #[serde(flatten)]
    pub rule: Rule,
    pub label: String,
    /// The lines it leaves out now.
    pub count: usize,
}

/// Everything the sheet "Afschrift controleren" shows.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub file_name: String,
    pub header: Vec<String>,
    pub delimiter: String,
    pub empty: usize,
    pub mapping: Mapping,
    pub rules: Vec<RuleView>,
    pub dedupe: bool,
    pub ledgers: Vec<Ledger>,
    pub ledger: usize,
    pub rows: Vec<Row>,
    pub totals: Totals,
}

/// What the user changes on the sheet.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Edit {
    Mapping {
        mapping: Mapping,
    },
    Ledger {
        index: usize,
    },
    Dedupe {
        on: bool,
    },
    AddRule {
        test: Test,
    },
    ToggleRule {
        index: usize,
        on: bool,
    },
    RemoveRule {
        index: usize,
    },
    /// Ticks or unticks these lines by hand.
    Tick {
        lines: Vec<usize>,
        included: bool,
    },
    /// Forgets every tick made by hand: the rules decide again.
    ClearTicks,
}

/// A rule as an AI client proposes it with `afschrift_inlezen`, and as it gets the sheet back when the user
/// sends the proposal back. Exactly one of the tests is given.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct SkipArg {
    /// Sla regels over waarvan de omschrijving deze tekst bevat (hoofdletters maken niet uit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub omschrijving_bevat: Option<String>,
    /// Sla regels over met een datum tot en met deze dag, JJJJ-MM-DD (bijvoorbeeld omdat het tot daar al geboekt is).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub datum_tot_en_met: Option<String>,
    /// Sla regels over met een datum vanaf deze dag, JJJJ-MM-DD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub datum_vanaf: Option<String>,
    /// Sla regels over waarvan deze kolom (de naam uit de kopregel van het bestand) precies `waarde` bevat, bijvoorbeeld één kaart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kolom: Option<String>,
    /// De waarde bij `kolom`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waarde: Option<String>,
    /// Sla regels over met precies dit bedrag in euro (negatief is af).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bedrag: Option<f64>,
    /// Sla alle regels over in één richting: "bij" of "af".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub richting: Option<String>,
    /// true: sla twee regels van dezelfde dag over die tegen elkaar wegvallen (hetzelfde bedrag bij en af).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wegvallend: Option<bool>,
    /// Waarom, in een paar woorden; de gebruiker ziet dit bij de regel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reden: Option<String>,
}

impl Statement {
    /// A rule from a tool's arguments.
    pub fn rule_of(&self, arg: &SkipArg, by: By) -> Result<Rule, String> {
        let day = |text: &str| {
            date(text).ok_or_else(|| format!("{text:?} is geen datum; schrijf JJJJ-MM-DD"))
        };
        let mut tests = Vec::new();
        if let Some(text) = arg.omschrijving_bevat.as_deref().map(str::trim) {
            if text.is_empty() {
                return Err("omschrijving_bevat mag niet leeg zijn".into());
            }
            tests.push(Test::Contains {
                text: text.to_owned(),
            });
        }
        if let Some(text) = &arg.datum_tot_en_met {
            tests.push(Test::Until { date: day(text)? });
        }
        if let Some(text) = &arg.datum_vanaf {
            tests.push(Test::From { date: day(text)? });
        }
        match (&arg.kolom, &arg.waarde) {
            (Some(kolom), Some(waarde)) => tests.push(Test::Column {
                column: self.column(kolom)?,
                value: waarde.trim().to_owned(),
            }),
            (None, None) => {}
            _ => return Err("kolom en waarde horen bij elkaar: geef ze allebei".into()),
        }
        if let Some(bedrag) = arg.bedrag {
            if !bedrag.is_finite() || bedrag.abs() > 1e10 {
                return Err("bedrag is geen geldig bedrag".into());
            }
            tests.push(Test::Amount {
                cents: (bedrag * 100.0).round() as i64,
            });
        }
        match arg
            .richting
            .as_deref()
            .map(|r| r.trim().to_lowercase())
            .as_deref()
        {
            Some("bij") => tests.push(Test::Incoming),
            Some("af") => tests.push(Test::Outgoing),
            Some(other) => return Err(format!("richting is \"bij\" of \"af\", niet {other:?}")),
            None => {}
        }
        if arg.wegvallend == Some(true) {
            tests.push(Test::Cancelling);
        }
        let mut tests = tests.into_iter();
        match (tests.next(), tests.next()) {
            (Some(test), None) => Ok(Rule {
                test,
                on: true,
                by,
                why: arg.reden.as_deref().map(str::trim).filter(|r| !r.is_empty()).map(str::to_owned),
            }),
            (None, _) => Err("Een regel bij overslaan zegt niet waarop hij let: geef omschrijving_bevat, datum_tot_en_met, datum_vanaf, kolom met waarde, bedrag, richting of wegvallend".into()),
            _ => Err("Een regel bij overslaan let op één ding; maak er meer regels van".into()),
        }
    }

    /// A rule as a tool argument, for the client that gets its proposal back.
    fn arg_of(&self, rule: &Rule) -> SkipArg {
        let mut arg = SkipArg {
            reden: rule.why.clone(),
            ..SkipArg::default()
        };
        match &rule.test {
            Test::Contains { text } => arg.omschrijving_bevat = Some(text.clone()),
            Test::Column { column, value } => {
                arg.kolom = self.table.header.get(*column).cloned();
                arg.waarde = Some(value.clone());
            }
            Test::Until { date } => arg.datum_tot_en_met = Some(iso(*date)),
            Test::From { date } => arg.datum_vanaf = Some(iso(*date)),
            Test::Amount { cents } => arg.bedrag = Some(*cents as f64 / 100.0),
            Test::Incoming => arg.richting = Some("bij".into()),
            Test::Outgoing => arg.richting = Some("af".into()),
            Test::Cancelling => arg.wegvallend = Some(true),
        }
        arg
    }

    pub fn view(&self) -> View {
        let rows = self.rows();
        let totals = self.totals(&rows);
        View {
            file_name: self.file_name.clone(),
            header: self.table.header.clone(),
            delimiter: self.table.delimiter_name().to_owned(),
            empty: self.table.empty,
            mapping: self.mapping.clone(),
            rules: self
                .rules
                .iter()
                .enumerate()
                .map(|(i, rule)| RuleView {
                    rule: rule.clone(),
                    label: rule.label(&self.table.header),
                    count: totals.per_rule[i],
                })
                .collect(),
            dedupe: self.dedupe,
            ledgers: self.ledgers.clone(),
            ledger: self.ledger,
            rows,
            totals,
        }
    }

    pub fn info(&self) -> Info {
        let totals = self.totals(&self.rows());
        Info {
            file_name: self.file_name.clone(),
            lines: totals.lines,
            included: totals.included,
        }
    }

    /// The title of the proposal, which follows what the user ticks.
    pub fn title(&self) -> String {
        format!("Afschrift inlezen: {}", self.file_name)
    }

    /// The proposal's lines: what the queue shows of the statement.
    pub fn lines(&self) -> Vec<Line> {
        let rows = self.rows();
        let totals = self.totals(&rows);
        let plural = |n: usize| if n == 1 { "regel" } else { "regels" };
        let mut lines = vec![
            line("Bestand", &self.file_name),
            line(
                "Herkend als",
                format!(
                    "csv met {}, {} {} met een datum en een bedrag",
                    self.table.delimiter_name(),
                    totals.lines,
                    plural(totals.lines)
                ),
            ),
            line(
                "Rekening",
                self.ledgers
                    .get(self.ledger)
                    .map(Ledger::label)
                    .unwrap_or_default(),
            ),
        ];
        let period = match (&totals.first, &totals.last) {
            (Some(first), Some(last)) => format!(
                ", {} t/m {}",
                date(first).map(dutch).unwrap_or_default(),
                date(last).map(dutch).unwrap_or_default()
            ),
            _ => String::new(),
        };
        lines.push(line(
            "Gaat mee",
            format!(
                "{} {}{period}, samen {}",
                totals.included,
                plural(totals.included),
                euro(totals.cents)
            ),
        ));
        if totals.skipped > 0 {
            let mut parts: Vec<String> = self
                .rules
                .iter()
                .enumerate()
                .filter(|(i, rule)| rule.on && totals.per_rule[*i] > 0)
                .map(|(i, rule)| {
                    format!(
                        "{} ({})",
                        rule.label(&self.table.header),
                        totals.per_rule[i]
                    )
                })
                .collect();
            let by_hand = rows.iter().filter(|r| r.out == Some(Out::User)).count();
            if by_hand > 0 {
                parts.push(format!("zelf uitgezet ({by_hand})"));
            }
            lines.push(line(
                "Overgeslagen",
                format!(
                    "{} {}: {}",
                    totals.skipped,
                    plural(totals.skipped),
                    parts.join(", ")
                ),
            ));
        }
        if totals.unreadable > 0 {
            lines.push(line(
                "Niet te lezen",
                format!(
                    "{} {} zonder datum of bedrag",
                    totals.unreadable,
                    plural(totals.unreadable)
                ),
            ));
        }
        lines.push(line(
            "Ontdubbelen",
            if self.dedupe {
                "aan: e-Boekhouden slaat over wat er al staat"
            } else {
                "uit"
            },
        ));
        lines
    }

    /// One change from the sheet.
    pub fn apply(&mut self, edit: Edit) -> Result<(), String> {
        const MAX_RULES: usize = 50;
        let gone = || "Die regel staat er niet meer".to_owned();
        match edit {
            Edit::Mapping { mapping } => {
                self.check(&mapping)?;
                self.mapping = mapping;
            }
            Edit::Ledger { index } => {
                if index >= self.ledgers.len() {
                    return Err("Die rekening staat niet in de lijst".into());
                }
                self.ledger = index;
            }
            Edit::Dedupe { on } => self.dedupe = on,
            Edit::AddRule { test } => {
                if self.rules.len() >= MAX_RULES {
                    return Err(format!("Hoogstens {MAX_RULES} regels"));
                }
                match &test {
                    Test::Contains { text } if text.trim().is_empty() => {
                        return Err("Vul de tekst in waar de regel op let".into());
                    }
                    Test::Column { column, value }
                        if *column >= self.table.header.len() || value.trim().is_empty() =>
                    {
                        return Err("Kies een kolom en vul de waarde in".into());
                    }
                    Test::Until { date } | Test::From { date }
                        if chrono::NaiveDate::from_ymd_opt(date.0, date.1, date.2).is_none() =>
                    {
                        return Err("Dat is geen datum".into());
                    }
                    _ => {}
                }
                if self.rules.iter().any(|r| r.test == test) {
                    return Err("Die regel staat er al".into());
                }
                self.rules.push(Rule {
                    test,
                    on: true,
                    by: By::User,
                    why: None,
                });
            }
            Edit::ToggleRule { index, on } => self.rules.get_mut(index).ok_or_else(gone)?.on = on,
            Edit::RemoveRule { index } => {
                if index >= self.rules.len() {
                    return Err(gone());
                }
                self.rules.remove(index);
            }
            Edit::Tick { lines, included } => {
                let rows = self.rows();
                for line in lines {
                    // Only a line that can be read in can be ticked; its tick is kept only where it differs
                    // from what the rules say.
                    let Some(row) = rows.iter().find(|r| r.line == line) else {
                        continue;
                    };
                    if matches!(row.out, Some(Out::BeforeStart | Out::Unreadable { .. })) {
                        continue;
                    }
                    let by_rules =
                        !matches!(row.out, Some(Out::Rule { .. })) && row.overruled.is_none();
                    if by_rules == included {
                        self.overrides.remove(&line);
                    } else {
                        self.overrides.insert(line, included);
                    }
                }
            }
            Edit::ClearTicks => self.overrides.clear(),
        }
        Ok(())
    }

    /// The sheet as it stands, in the words of the tool's own arguments: what an AI client gets when the user
    /// sends the proposal back, so it can make a new one from where the user left off.
    pub fn as_args(&self) -> serde_json::Value {
        let header = &self.table.header;
        let rows = self.rows();
        let lines_where = |want: bool| -> Vec<usize> {
            self.overrides
                .iter()
                .filter(|(_, on)| **on == want)
                .map(|(line, _)| *line)
                .collect()
        };
        serde_json::json!({
            "rekening": self.ledgers.get(self.ledger).map(|l| l.code.clone()),
            "kolommen": {
                "datum": header[self.mapping.date],
                "omschrijving": header[self.mapping.description],
                "bedrag": header[self.mapping.amount],
            },
            "start_regel": self.mapping.start,
            "ontdubbelen": self.dedupe,
            "overslaan": self.rules.iter().filter(|r| r.on).map(|r| self.arg_of(r)).collect::<Vec<_>>(),
            "uitgezette_regels_van_de_gebruiker": self.rules.iter().filter(|r| !r.on).map(|r| self.arg_of(r)).collect::<Vec<_>>(),
            "regels_zelf_uitgevinkt": lines_where(false),
            "regels_zelf_aangevinkt": lines_where(true),
            "gaat_mee": rows.iter().filter(|r| r.included).count(),
        })
    }
}

/// One line that goes to e-Boekhouden.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chosen {
    pub line: usize,
    pub date: Date,
    pub description: String,
    pub cents: i64,
}

impl Statement {
    pub fn new(file_name: &str, bytes: &[u8]) -> Result<Self, String> {
        let table = Table::parse(&decode(bytes))?;
        let mapping = Mapping::guess(&table)?;
        Ok(Self {
            file_name: file_name.to_owned(),
            table,
            mapping,
            rules: Vec::new(),
            overrides: BTreeMap::new(),
            dedupe: true,
            ledgers: Vec::new(),
            ledger: 0,
        })
    }

    /// The date, description and amount of a record in the chosen columns.
    fn read(&self, raw: &Raw) -> (Option<Date>, String, Option<i64>) {
        let cell = |i: usize| raw.cells.get(i).map(String::as_str).unwrap_or("");
        let description = cell(self.mapping.description)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        (
            date(cell(self.mapping.date)),
            description,
            amount(cell(self.mapping.amount)),
        )
    }

    /// Every line with whether it goes along and why not.
    pub fn rows(&self) -> Vec<Row> {
        let read: Vec<(Option<Date>, String, Option<i64>)> =
            self.table.records.iter().map(|r| self.read(r)).collect();
        let live = |i: usize| {
            self.table.records[i].line >= self.mapping.start
                && read[i].0.is_some()
                && read[i].2.is_some()
        };
        // Lines that cancel each other: per day, each line in pairs off with one line out of the same amount.
        let mut cancelled = vec![false; read.len()];
        if self
            .rules
            .iter()
            .any(|r| r.on && r.test == Test::Cancelling)
        {
            let mut open: BTreeMap<(Date, i64), Vec<usize>> = BTreeMap::new();
            for i in (0..read.len()).filter(|&i| live(i)) {
                let (Some(day), Some(cents)) = (read[i].0, read[i].2) else {
                    continue;
                };
                if cents == 0 {
                    continue;
                }
                match open.get_mut(&(day, -cents)).and_then(Vec::pop) {
                    Some(other) => {
                        cancelled[i] = true;
                        cancelled[other] = true;
                    }
                    None => open.entry((day, cents)).or_default().push(i),
                }
            }
        }
        self.table
            .records
            .iter()
            .enumerate()
            .map(|(i, raw)| {
                let (day, description, cents) = read[i].clone();
                let hit = self.rules.iter().position(|rule| {
                    rule.on
                        && match (&rule.test, day, cents) {
                            (Test::Contains { text }, ..) => {
                                !text.trim().is_empty()
                                    && description
                                        .to_lowercase()
                                        .contains(&text.trim().to_lowercase())
                            }
                            (Test::Column { column, value }, ..) => raw
                                .cells
                                .get(*column)
                                .is_some_and(|c| c.trim().eq_ignore_ascii_case(value.trim())),
                            (Test::Until { date }, Some(day), _) => day <= *date,
                            (Test::From { date }, Some(day), _) => day >= *date,
                            (Test::Amount { cents: want }, _, Some(cents)) => cents == *want,
                            (Test::Incoming, _, Some(cents)) => cents > 0,
                            (Test::Outgoing, _, Some(cents)) => cents < 0,
                            (Test::Cancelling, ..) => cancelled[i],
                            _ => false,
                        }
                });
                let fixed = if raw.line < self.mapping.start {
                    Some(Out::BeforeStart)
                } else if day.is_none() {
                    Some(Out::Unreadable {
                        what: "geen datum".into(),
                    })
                } else if cents.is_none() {
                    Some(Out::Unreadable {
                        what: "geen bedrag".into(),
                    })
                } else {
                    None
                };
                let (out, overruled) = match (fixed, self.overrides.get(&raw.line), hit) {
                    (Some(fixed), ..) => (Some(fixed), None),
                    (None, Some(true), hit) => (None, hit),
                    (None, Some(false), _) => (Some(Out::User), None),
                    (None, None, Some(rule)) => (Some(Out::Rule { rule }), None),
                    (None, None, None) => (None, None),
                };
                Row {
                    line: raw.line,
                    cells: raw.cells.clone(),
                    date: day.map(iso),
                    description,
                    cents,
                    included: out.is_none(),
                    out,
                    overruled,
                }
            })
            .collect()
    }

    pub fn totals(&self, rows: &[Row]) -> Totals {
        let mut totals = Totals {
            lines: 0,
            included: 0,
            skipped: 0,
            unreadable: 0,
            cents: 0,
            first: None,
            last: None,
            per_rule: vec![0; self.rules.len()],
        };
        for row in rows {
            match &row.out {
                Some(Out::BeforeStart) => continue,
                Some(Out::Unreadable { .. }) => {
                    totals.unreadable += 1;
                    continue;
                }
                Some(Out::Rule { rule }) => {
                    totals.per_rule[*rule] += 1;
                    totals.skipped += 1;
                }
                Some(Out::User) => totals.skipped += 1,
                None => {
                    totals.included += 1;
                    totals.cents += row.cents.unwrap_or(0);
                    if let Some(day) = &row.date {
                        if totals.first.as_ref().is_none_or(|f| day < f) {
                            totals.first = Some(day.clone());
                        }
                        if totals.last.as_ref().is_none_or(|l| day > l) {
                            totals.last = Some(day.clone());
                        }
                    }
                }
            }
            totals.lines += 1;
        }
        totals
    }

    /// The lines that go to e-Boekhouden, in the order of the file.
    pub fn chosen(&self) -> Vec<Chosen> {
        self.rows()
            .into_iter()
            .filter(|row| row.included)
            .filter_map(|row| {
                Some(Chosen {
                    line: row.line,
                    date: date(row.date.as_deref()?)?,
                    description: row.description,
                    cents: row.cents?,
                })
            })
            .collect()
    }

    /// Checks a mapping against this file before it is taken over.
    pub fn check(&self, mapping: &Mapping) -> Result<(), String> {
        let width = self.table.header.len();
        if mapping.date >= width || mapping.description >= width || mapping.amount >= width {
            return Err("Die kolom staat niet in het bestand".into());
        }
        if mapping.date == mapping.amount
            || mapping.date == mapping.description
            || mapping.amount == mapping.description
        {
            return Err("Datum, omschrijving en bedrag moeten elk een eigen kolom hebben".into());
        }
        let last = self.table.records.last().map_or(1, |r| r.line);
        if mapping.start == 0 || mapping.start > last {
            return Err(format!(
                "Starten vanaf regel moet tussen 1 en {last} liggen"
            ));
        }
        Ok(())
    }

    /// The column with this name, as the file's header has it, whatever the case.
    pub fn column(&self, name: &str) -> Result<usize, String> {
        self.table
            .header
            .iter()
            .position(|h| h.trim().eq_ignore_ascii_case(name.trim()))
            .ok_or_else(|| {
                format!(
                    "Het bestand heeft geen kolom {name:?}; de kolommen zijn: {}",
                    self.table.header.join(", ")
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARD: &str = "Accountnummer,Kaartnummer,Naam op kaart,Transactiedatum,Boekingsdatum,Omschrijving,Valuta,Bedrag,Koers,Bedrag in EUR\r\n\
'80000216660,1111,A. Tester,2026-10-27,2026-10-28,KOSTEN TE LATE BETALING,EUR,\"-24,00\",,\"-24,00\"\r\n\
'80000216660,1111,A. Tester,2026-10-26,2026-10-27,BETALING ONTVANGEN - DANK U,EUR,\"2.000,04\",1.0,\"2.000,04\"\r\n\
\r\n\
'80000216660,2222,B. Tester,2026-10-19,2026-10-20,CORRECTIE BETALING,EUR,\"-2.000,04\",,\"-2.000,04\"\r\n\
'80000216660,2222,B. Tester,2026-10-19,2026-10-20,BETALING ONTVANGEN - DANK U,EUR,\"2.000,04\",,\"2.000,04\"\r\n\
'80000216660,1111,A. Tester,2026-10-17,2026-10-18,\"CURSOR, AI POWERED IDE  SAN FRANCISCO\",USD,\"-52,75\",1.093037712,\"-48,26\"\r\n\
'80000216660,1111,A. Tester,2026-06-07,2026-06-08,LARAVEL FORGE NEW YORK,USD,\"-19,00\",1.090700344,\"-17,42\"\r\n";

    fn card() -> Statement {
        Statement::new("kaart.csv", CARD.as_bytes()).unwrap()
    }

    #[test]
    fn reads_amounts_and_dates_the_way_banks_write_them() {
        for (text, cents) in [
            ("-24,00", -2400),
            ("2.000,04", 200_004),
            ("1,234.56", 123_456),
            ("12.5", 1250),
            ("€ 3,10", 310),
            ("3,10-", -310),
            ("+5", 500),
            ("1.000", 100_000),
            ("'-7,5", -750),
        ] {
            assert_eq!(amount(text), Some(cents), "{text}");
        }
        for text in ["", "abc", "1.093037712", "1.2,50", "12,345,6", "-", "1e3"] {
            assert_eq!(amount(text), None, "{text}");
        }
        for (text, day) in [
            ("2026-11-06", (2026, 11, 6)),
            ("06-11-2026", (2026, 11, 6)),
            ("6/11/2026", (2026, 11, 6)),
            ("20261106", (2026, 11, 6)),
            ("2026-11-06T08:00:00", (2026, 11, 6)),
            ("06.11.2026 08:00", (2026, 11, 6)),
        ] {
            assert_eq!(date(text), Some(day), "{text}");
        }
        for text in [
            "",
            "2026-13-01",
            "31-02-2026",
            "06-11-26",
            "80000216660",
            "1.0",
        ] {
            assert_eq!(date(text), None, "{text}");
        }
        assert_eq!(euro(-941_237), "-9.412,37");
        assert_eq!(euro(5), "0,05");
        assert_eq!(euro(100_000_000), "1.000.000,00");
    }

    #[test]
    fn reads_a_csv_with_quotes_empty_lines_and_its_own_delimiter() {
        let table = Table::parse(CARD).unwrap();
        assert_eq!(
            (table.delimiter, table.header_line, table.empty),
            (',', Some(1), 1)
        );
        assert_eq!(table.records.len(), 7);
        // The line numbers are those of the file: the empty line 4 is counted.
        assert_eq!(
            table.records.iter().map(|r| r.line).collect::<Vec<_>>(),
            [1, 2, 3, 5, 6, 7, 8]
        );
        assert_eq!(
            table.records[5].cells[5],
            "CURSOR, AI POWERED IDE  SAN FRANCISCO"
        );
        let semi = Table::parse("Datum;Naam;Bedrag\n01-02-2026;\"Regel met \"\"quote\"\" en\nbreuk\";1,50\n02-02-2026;B;-2,00").unwrap();
        assert_eq!((semi.delimiter, semi.records.len()), (';', 3));
        assert_eq!(semi.records[1].cells[1], "Regel met \"quote\" en\nbreuk");
        assert_eq!(
            semi.records[2].line, 4,
            "a break inside quotes counts as a line"
        );
        let bare = Table::parse("01-02-2026\tA\t1,50\n02-02-2026\tB\t2,50").unwrap();
        assert_eq!((bare.delimiter, bare.header_line), ('\t', None));
        assert_eq!(bare.header, ["Kolom 1", "Kolom 2", "Kolom 3"]);
        assert!(Table::parse("a\nb").unwrap_err().contains("één kolom"));
        assert!(Table::parse("\n\n").unwrap_err().contains("geen regels"));
        assert!(
            Table::parse("a;\"b\n1;2")
                .unwrap_err()
                .contains("aanhalingsteken")
        );
        assert_eq!(decode(b"\xef\xbb\xbfa;b"), "a;b");
        assert_eq!(decode(b"caf\xe9 \x80 5"), "café € 5");
    }

    #[test]
    fn guesses_the_columns_of_a_card_statement() {
        let statement = card();
        let m = &statement.mapping;
        assert_eq!(statement.table.header[m.date], "Transactiedatum");
        assert_eq!(statement.table.header[m.description], "Omschrijving");
        // Not "Bedrag" (in the currency of the purchase) and not "Koers".
        assert_eq!(statement.table.header[m.amount], "Bedrag in EUR");
        assert_eq!(m.start, 2);
        let rows = statement.rows();
        assert_eq!(rows.len(), 7);
        assert_eq!(rows[0].out, Some(Out::BeforeStart));
        assert_eq!(
            rows[5].description, "CURSOR, AI POWERED IDE SAN FRANCISCO",
            "runs of spaces become one"
        );
        assert_eq!(
            (rows[5].date.as_deref(), rows[5].cents),
            (Some("2026-10-17"), Some(-4826))
        );
        let totals = statement.totals(&rows);
        assert_eq!((totals.lines, totals.included, totals.skipped), (6, 6, 0));
        assert_eq!(
            totals.cents,
            -2400 + 200_004 - 200_004 + 200_004 - 4826 - 1742
        );
        assert_eq!(
            (totals.first.as_deref(), totals.last.as_deref()),
            (Some("2026-06-07"), Some("2026-10-27"))
        );
        assert!(
            Statement::new("x.csv", b"a;b\nc;d")
                .unwrap_err()
                .contains("geen kolom met een datum")
        );
    }

    #[test]
    fn rules_leave_lines_out_and_a_tick_goes_above_a_rule() {
        let mut statement = card();
        let rule = |test| Rule {
            test,
            on: true,
            by: By::Client,
            why: None,
        };
        statement.rules = vec![
            rule(Test::Until { date: (2026, 6, 7) }),
            rule(Test::Contains {
                text: "betaling ontvangen".into(),
            }),
            rule(Test::Contains {
                text: "CORRECTIE BETALING".into(),
            }),
        ];
        let rows = statement.rows();
        let out: Vec<Option<Out>> = rows.iter().map(|r| r.out.clone()).collect();
        assert_eq!(
            out,
            [
                Some(Out::BeforeStart),
                None,
                Some(Out::Rule { rule: 1 }),
                Some(Out::Rule { rule: 2 }),
                Some(Out::Rule { rule: 1 }),
                None,
                Some(Out::Rule { rule: 0 }),
            ]
        );
        let totals = statement.totals(&rows);
        assert_eq!(
            (totals.included, totals.skipped, totals.per_rule.clone()),
            (2, 4, vec![1, 2, 1])
        );
        assert_eq!(totals.cents, -2400 - 4826);
        // A rule that is off does nothing; a tick brings a line back and an untick takes one out.
        statement.rules[2].on = false;
        statement.overrides.insert(3, true);
        statement.overrides.insert(2, false);
        let rows = statement.rows();
        assert_eq!(rows[3].out, None, "the rule is off");
        assert_eq!((rows[2].included, rows[2].overruled), (true, Some(1)));
        assert_eq!(rows[1].out, Some(Out::User));
        // What goes out is exactly what is ticked.
        let chosen = statement.chosen();
        assert_eq!(chosen.iter().map(|c| c.line).collect::<Vec<_>>(), [3, 5, 7]);
        assert_eq!(
            chosen[2],
            Chosen {
                line: 7,
                date: (2026, 10, 17),
                description: "CURSOR, AI POWERED IDE SAN FRANCISCO".into(),
                cents: -4826
            }
        );
        assert_eq!(
            statement.rules[0].label(&statement.table.header),
            "Datum t/m 07-06-2026"
        );
        assert_eq!(
            Rule {
                test: Test::Column {
                    column: 1,
                    value: "2222".into()
                },
                on: true,
                by: By::User,
                why: None
            }
            .label(&statement.table.header),
            "Kaartnummer is 2222"
        );
    }

    #[test]
    fn two_lines_of_a_day_that_cancel_each_other_go_out_as_a_pair() {
        let mut statement = card();
        statement.rules = vec![Rule {
            test: Test::Cancelling,
            on: true,
            by: By::User,
            why: None,
        }];
        let rows = statement.rows();
        // 19-10: -2.000,04 and +2.000,04. The +2.000,04 of 26-10 has no partner on its day.
        let out: Vec<usize> = rows
            .iter()
            .filter(|r| r.out == Some(Out::Rule { rule: 0 }))
            .map(|r| r.line)
            .collect();
        assert_eq!(out, [5, 6]);
        let column = Rule {
            test: Test::Column {
                column: 1,
                value: "2222".into(),
            },
            on: true,
            by: By::User,
            why: None,
        };
        statement.rules = vec![
            column,
            Rule {
                test: Test::Outgoing,
                on: true,
                by: By::User,
                why: None,
            },
        ];
        let rows = statement.rows();
        assert_eq!(
            rows.iter()
                .filter(|r| r.included)
                .map(|r| r.line)
                .collect::<Vec<_>>(),
            [3]
        );
    }

    #[test]
    fn a_line_without_a_date_or_amount_cannot_be_ticked_and_the_start_line_moves() {
        let mut statement = Statement::new(
            "bank.csv",
            b"Datum;Omschrijving;Bedrag\n01-02-2026;A;1,50\ntotaal;;\n02-02-2026;B;-2,00\n03-02-2026;C;n.v.t.\n04-02-2026;D;3,00\n05-02-2026;E;4,00\n06-02-2026;F;5,00\n07-02-2026;G;6,00\n08-02-2026;H;7,00\n09-02-2026;I;8,00\n10-02-2026;J;9,00\n11-02-2026;K;1,00\n12-02-2026;L;2,00",
        )
        .unwrap();
        statement.overrides.insert(3, true);
        let rows = statement.rows();
        assert_eq!(
            rows[2].out,
            Some(Out::Unreadable {
                what: "geen datum".into()
            })
        );
        assert_eq!(
            rows[4].out,
            Some(Out::Unreadable {
                what: "geen bedrag".into()
            })
        );
        assert_eq!(statement.totals(&rows).unreadable, 2);
        let mut later = statement.mapping.clone();
        later.start = 6;
        statement.check(&later).unwrap();
        statement.mapping = later;
        let rows = statement.rows();
        assert_eq!(
            rows.iter()
                .filter(|r| r.out == Some(Out::BeforeStart))
                .count(),
            5
        );
        assert_eq!(statement.totals(&rows).included, 9);
        let mut wrong = statement.mapping.clone();
        wrong.amount = wrong.date;
        assert!(statement.check(&wrong).unwrap_err().contains("eigen kolom"));
        wrong = statement.mapping.clone();
        wrong.start = 99;
        assert!(
            statement
                .check(&wrong)
                .unwrap_err()
                .contains("tussen 1 en 14")
        );
        assert_eq!(statement.column("bedrag"), Ok(2));
        assert!(
            statement
                .column("Saldo")
                .unwrap_err()
                .contains("Datum, Omschrijving, Bedrag")
        );
    }

    #[test]
    fn the_sheet_edits_the_statement_and_says_what_it_shows() {
        let mut statement = card();
        statement.ledgers = vec![
            Ledger {
                id: 11,
                code: "1010".into(),
                name: "Bank".into(),
            },
            Ledger {
                id: 18,
                code: "1820".into(),
                name: "Credit card".into(),
            },
        ];
        let proposed = [
            SkipArg {
                datum_tot_en_met: Some("2026-06-07".into()),
                reden: Some(" al geboekt ".into()),
                ..SkipArg::default()
            },
            SkipArg {
                omschrijving_bevat: Some("BETALING ONTVANGEN".into()),
                ..SkipArg::default()
            },
            SkipArg {
                kolom: Some("kaartnummer".into()),
                waarde: Some("2222".into()),
                ..SkipArg::default()
            },
        ];
        for arg in &proposed {
            let rule = statement.rule_of(arg, By::Client).unwrap();
            statement.rules.push(rule);
        }
        assert_eq!(statement.rules[0].why.as_deref(), Some("al geboekt"));
        for (arg, part) in [
            (SkipArg::default(), "zegt niet waarop hij let"),
            (
                SkipArg {
                    wegvallend: Some(true),
                    richting: Some("af".into()),
                    ..SkipArg::default()
                },
                "één ding",
            ),
            (
                SkipArg {
                    kolom: Some("Kaartnummer".into()),
                    ..SkipArg::default()
                },
                "horen bij elkaar",
            ),
            (
                SkipArg {
                    kolom: Some("Saldo".into()),
                    waarde: Some("1".into()),
                    ..SkipArg::default()
                },
                "geen kolom",
            ),
            (
                SkipArg {
                    datum_vanaf: Some("gisteren".into()),
                    ..SkipArg::default()
                },
                "geen datum",
            ),
            (
                SkipArg {
                    richting: Some("op".into()),
                    ..SkipArg::default()
                },
                "\"bij\" of \"af\"",
            ),
        ] {
            let error = statement.rule_of(&arg, By::Client).unwrap_err();
            assert!(error.contains(part), "{error}");
        }
        statement.apply(Edit::Ledger { index: 1 }).unwrap();
        assert!(statement.apply(Edit::Ledger { index: 2 }).is_err());
        statement
            .apply(Edit::AddRule {
                test: Test::Contains {
                    text: "kosten".into(),
                },
            })
            .unwrap();
        assert!(
            statement
                .apply(Edit::AddRule {
                    test: Test::Contains {
                        text: "kosten".into()
                    }
                })
                .unwrap_err()
                .contains("staat er al")
        );
        assert!(
            statement
                .apply(Edit::AddRule {
                    test: Test::Contains { text: " ".into() }
                })
                .is_err()
        );
        statement
            .apply(Edit::ToggleRule {
                index: 2,
                on: false,
            })
            .unwrap();
        // Line 3 is out by a rule: ticking it is kept. Line 7 is in: ticking it again keeps nothing.
        statement
            .apply(Edit::Tick {
                lines: vec![3, 7, 1, 99],
                included: true,
            })
            .unwrap();
        statement
            .apply(Edit::Tick {
                lines: vec![7],
                included: false,
            })
            .unwrap();
        assert_eq!(statement.overrides, BTreeMap::from([(3, true), (7, false)]));
        let view = statement.view();
        assert_eq!(
            view.rules
                .iter()
                .map(|r| (r.label.as_str(), r.count, r.rule.by))
                .collect::<Vec<_>>(),
            [
                ("Datum t/m 07-06-2026", 1, By::Client),
                ("Omschrijving bevat BETALING ONTVANGEN", 1, By::Client),
                ("Kaartnummer is 2222", 0, By::Client),
                ("Omschrijving bevat kosten", 1, By::User),
            ]
        );
        assert_eq!((view.totals.included, view.totals.skipped), (2, 4));
        assert_eq!(
            statement.info(),
            Info {
                file_name: "kaart.csv".into(),
                lines: 6,
                included: 2
            }
        );
        let lines: Vec<(String, String)> = statement
            .lines()
            .into_iter()
            .map(|l| (l.label, l.value))
            .collect();
        assert_eq!(lines[2], ("Rekening".into(), "1820 Credit card".into()));
        assert_eq!(
            lines[3],
            (
                "Gaat mee".into(),
                "2 regels, 19-10-2026 t/m 26-10-2026, samen 0,00".into()
            )
        );
        assert_eq!(
            lines[4].1,
            "4 regels: Datum t/m 07-06-2026 (1), Omschrijving bevat BETALING ONTVANGEN (1), Omschrijving bevat kosten (1), zelf uitgezet (1)"
        );
        // Unticking what a rule leaves out anyway forgets the tick; a tick is cleared when it says what the rules say.
        statement
            .apply(Edit::Tick {
                lines: vec![3],
                included: false,
            })
            .unwrap();
        assert_eq!(statement.overrides, BTreeMap::from([(7, false)]));
        let args = statement.as_args();
        assert_eq!(args["rekening"], "1820");
        assert_eq!(args["kolommen"]["bedrag"], "Bedrag in EUR");
        assert_eq!(
            args["overslaan"][0],
            serde_json::json!({"datum_tot_en_met": "2026-06-07", "reden": "al geboekt"})
        );
        assert_eq!(
            args["uitgezette_regels_van_de_gebruiker"][0],
            serde_json::json!({"kolom": "Kaartnummer", "waarde": "2222"})
        );
        assert_eq!(args["regels_zelf_uitgevinkt"], serde_json::json!([7]));
        statement.apply(Edit::RemoveRule { index: 3 }).unwrap();
        statement.apply(Edit::ClearTicks).unwrap();
        assert!(statement.apply(Edit::RemoveRule { index: 9 }).is_err());
        let wire: Edit = serde_json::from_value(
            serde_json::json!({"kind": "addRule", "test": {"kind": "until", "date": [2026, 6, 7]}}),
        )
        .unwrap();
        assert_eq!(
            wire,
            Edit::AddRule {
                test: Test::Until { date: (2026, 6, 7) }
            }
        );
    }
}
