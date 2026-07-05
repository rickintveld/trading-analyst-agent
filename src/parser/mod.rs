//! High-performance CSV ingestion with format auto-detection.
//!
//! Detection is **content-aware**: header aliases only propose a mapping,
//! the sampled column values must confirm it (a column named "Open" holding
//! datetimes becomes the date column, not an entry price). Malformed rows
//! are skipped and reported as warnings, never a hard failure.
//!
//! For exports the heuristics can't resolve, a mapping file (TOML, same
//! shape as `[csv]` in config.toml) can pin every column — see the
//! `inspect` CLI command and the csv-mapping skill, which let the AI layer
//! author that mapping from a structured sample without ever entering the
//! Rust engine.

use anyhow::{bail, Context, Result};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;

use crate::config::CsvConfig;
use crate::shared::{Direction, Session, Trade};

/// Number of values sampled per column for content checks.
const CONTENT_SAMPLES: usize = 40;

/// Canonical trade fields a CSV column can map to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Field {
    Date,
    Time,
    Symbol,
    Direction,
    Entry,
    Exit,
    PositionSize,
    Risk,
    Reward,
    Rr,
    Pnl,
    Fees,
    Strategy,
    Session,
    Notes,
}

impl Field {
    pub fn canonical_name(&self) -> &'static str {
        match self {
            Field::Date => "date",
            Field::Time => "time",
            Field::Symbol => "symbol",
            Field::Direction => "direction",
            Field::Entry => "entry",
            Field::Exit => "exit",
            Field::PositionSize => "position_size",
            Field::Risk => "risk",
            Field::Reward => "reward",
            Field::Rr => "rr",
            Field::Pnl => "pnl",
            Field::Fees => "fees",
            Field::Strategy => "strategy",
            Field::Session => "session",
            Field::Notes => "notes",
        }
    }

    pub fn required(&self) -> bool {
        matches!(self, Field::Date | Field::Symbol | Field::Pnl)
    }

    /// One-line description used by the `inspect` command so the AI layer
    /// knows what each canonical field means.
    pub fn description(&self) -> &'static str {
        match self {
            Field::Date => "trade date, or full open datetime (time is extracted automatically)",
            Field::Time => {
                "trade open time (HH:MM or HH:MM:SS) when stored separately from the date"
            }
            Field::Symbol => "instrument / pair / ticker, e.g. EURUSD",
            Field::Direction => "trade direction: long/short, buy/sell, b/s",
            Field::Entry => "entry price",
            Field::Exit => "exit price",
            Field::PositionSize => "position size: lots, quantity, volume, units, contracts",
            Field::Risk => "money risked on the trade",
            Field::Reward => "planned reward / target amount",
            Field::Rr => "realized risk-reward multiple (R)",
            Field::Pnl => "realized profit/loss of the trade (required)",
            Field::Fees => "commissions / fees / swap costs (sign is ignored)",
            Field::Strategy => "strategy / setup / playbook name",
            Field::Session => "market session label (London, New York, Asia, Overlap)",
            Field::Notes => "free-form trade notes",
        }
    }

    pub fn all() -> &'static [Field] {
        &[
            Field::Date,
            Field::Time,
            Field::Symbol,
            Field::Direction,
            Field::Entry,
            Field::Exit,
            Field::PositionSize,
            Field::Risk,
            Field::Reward,
            Field::Rr,
            Field::Pnl,
            Field::Fees,
            Field::Strategy,
            Field::Session,
            Field::Notes,
        ]
    }

    /// Known header aliases (lowercased, stripped of spaces/`_`/`-`/`.`),
    /// in priority order. Aliases only *propose* — content must confirm.
    fn aliases(&self) -> &'static [&'static str] {
        match self {
            Field::Date => &[
                "date",
                "opendate",
                "entrydate",
                "tradedate",
                "day",
                "opentime",
                "datetime",
                "timeopened",
                "created",
                "open",
            ],
            Field::Time => &["time", "entrytime", "timeofday"],
            Field::Symbol => &[
                "symbol",
                "pair",
                "instrument",
                "ticker",
                "market",
                "asset",
                "currencypair",
                "product",
            ],
            Field::Direction => &[
                "direction",
                "side",
                "type",
                "position",
                "buysell",
                "longshort",
                "action",
            ],
            Field::Entry => &[
                "entry",
                "entryprice",
                "open",
                "openprice",
                "pricein",
                "avgentry",
                "price",
            ],
            Field::Exit => &[
                "exit",
                "exitprice",
                "close",
                "closeprice",
                "priceout",
                "avgexit",
                "price",
            ],
            Field::PositionSize => &[
                "positionsize",
                "size",
                "lots",
                "lotsize",
                "quantity",
                "qty",
                "volume",
                "units",
                "contracts",
                "shares",
            ],
            Field::Risk => &[
                "risk",
                "riskamount",
                "riskusd",
                "risked",
                "riskpercent",
                "risk$",
                "riskpertrade",
            ],
            Field::Reward => &[
                "reward",
                "rewardamount",
                "target",
                "targetamount",
                "plannedreward",
            ],
            Field::Rr => &[
                "rr",
                "riskreward",
                "riskrewardratio",
                "rmultiple",
                "rmultiples",
                "realizedrr",
                "plannedrr",
                "ratio",
            ],
            Field::Pnl => &[
                "pnl",
                "pl",
                "profit",
                "profitloss",
                "netpnl",
                "netprofit",
                "result",
                "gain",
                "realizedpnl",
                "netgain",
                "return",
            ],
            Field::Fees => &[
                "fees",
                "fee",
                "commission",
                "commissions",
                "costs",
                "cost",
                "swap",
            ],
            Field::Strategy => &[
                "strategy",
                "setup",
                "system",
                "playbook",
                "model",
                "setupname",
                "edge",
            ],
            Field::Session => &["session", "marketsession", "killzone", "tradingsession"],
            Field::Notes => &[
                "notes",
                "note",
                "comment",
                "comments",
                "remarks",
                "journal",
                "description",
                "mistakes",
            ],
        }
    }
}

/// What auto-detection concluded about the file format.
#[derive(Debug, Clone)]
pub struct DetectedFormat {
    pub delimiter: char,
    pub encoding: &'static str,
    pub date_format: String,
    pub decimal_separator: char,
    /// header name -> canonical field, for diagnostics.
    pub mapped_headers: Vec<(String, &'static str)>,
    pub unmapped_headers: Vec<String>,
}

/// Result of parsing a CSV file.
#[derive(Debug)]
pub struct ParseReport {
    pub trades: Vec<Trade>,
    pub warnings: Vec<String>,
    pub skipped_rows: usize,
    pub detected: DetectedFormat,
}

/// Parse a trading journal CSV into normalized [`Trade`]s.
pub fn parse_csv(path: &Path, cfg: &CsvConfig) -> Result<ParseReport> {
    let (headers, records, delimiter, encoding) = read_table(path, cfg)?;
    let mapping = map_headers(&headers, &records, cfg)?;

    // --- date format detection -------------------------------------------
    let date_idx = *mapping
        .get(&Field::Date)
        .with_context(|| missing_column_help("date", &headers))?;
    let date_samples: Vec<&str> = records
        .iter()
        .filter_map(|r| r.get(date_idx))
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .take(200)
        .collect();
    let date_format = match cfg.date_format.clone() {
        Some(f) => f,
        None => detect_date_format(&date_samples, cfg.prefer_day_first)
            .context("could not detect the date format — set date_format in the mapping file or [csv] config")?,
    };

    // --- decimal separator detection --------------------------------------
    let decimal_separator = match cfg
        .decimal_separator
        .as_deref()
        .and_then(|d| d.chars().next())
    {
        Some(d) => d,
        None => detect_decimal_separator(&records, &mapping, delimiter),
    };

    // --- row conversion ----------------------------------------------------
    let mut trades = Vec::with_capacity(records.len());
    let mut warnings = Vec::new();
    let mut skipped = 0usize;

    for (row_number, record) in records.iter().enumerate() {
        match build_trade(
            row_number,
            record,
            &mapping,
            &date_format,
            decimal_separator,
        ) {
            Ok(trade) => trades.push(trade),
            Err(err) => {
                skipped += 1;
                if warnings.len() < 50 {
                    // +2: 1-based and the header row.
                    warnings.push(format!("row {}: {err}", row_number + 2));
                }
            }
        }
    }
    if skipped > 50 {
        warnings.push(format!("... and {} more skipped rows", skipped - 50));
    }

    let mapped_headers: Vec<(String, &'static str)> = mapping
        .iter()
        .map(|(field, idx)| (headers[*idx].clone(), field.canonical_name()))
        .collect();
    let unmapped_headers: Vec<String> = headers
        .iter()
        .enumerate()
        .filter(|(i, _)| !mapping.values().any(|idx| idx == i))
        .map(|(_, h)| h.clone())
        .collect();

    Ok(ParseReport {
        trades,
        warnings,
        skipped_rows: skipped,
        detected: DetectedFormat {
            delimiter,
            encoding,
            date_format,
            decimal_separator,
            mapped_headers,
            unmapped_headers,
        },
    })
}

/// Read and decode the raw table: headers + all non-empty records.
fn read_table(
    path: &Path,
    cfg: &CsvConfig,
) -> Result<(Vec<String>, Vec<csv::StringRecord>, char, &'static str)> {
    let bytes =
        std::fs::read(path).with_context(|| format!("cannot read CSV file {}", path.display()))?;
    let (content, encoding) = decode(&bytes);
    if content.trim().is_empty() {
        bail!("CSV file {} is empty", path.display());
    }
    let delimiter = match cfg.delimiter.as_deref().and_then(|d| d.chars().next()) {
        Some(d) => d,
        None => detect_delimiter(&content),
    };
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter as u8)
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(content.as_bytes());
    let headers: Vec<String> = reader
        .headers()
        .context("cannot read CSV headers")?
        .iter()
        .map(|h| h.to_string())
        .collect();
    let records: Vec<csv::StringRecord> = reader
        .records()
        .filter_map(|r| r.ok())
        .filter(|r| r.iter().any(|f| !f.trim().is_empty()))
        .collect();
    Ok((headers, records, delimiter, encoding))
}

fn missing_column_help(field: &str, headers: &[String]) -> String {
    format!(
        "no {field} column found in headers {headers:?} — run `trade-analyzer inspect <csv>` \
         and create a mapping file (<csv>.mapping.toml) with [mapping] {field} = \"<your header>\" \
         (the csv-mapping skill automates this)"
    )
}

/// Decode bytes as UTF-8 (with optional BOM) or fall back to Latin-1.
fn decode(bytes: &[u8]) -> (String, &'static str) {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => (s.to_string(), "utf-8"),
        Err(_) => (bytes.iter().map(|&b| b as char).collect(), "latin-1"),
    }
}

/// Pick the delimiter with the highest consistent count across sample lines.
fn detect_delimiter(content: &str) -> char {
    let candidates = [',', ';', '\t', '|'];
    let lines: Vec<&str> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(10)
        .collect();
    let mut best = (',', 0usize);
    for c in candidates {
        let counts: Vec<usize> = lines.iter().map(|l| l.matches(c).count()).collect();
        let min = counts.iter().copied().min().unwrap_or(0);
        // Require the delimiter to appear on every sampled line.
        if min > 0 && min > best.1 {
            best = (c, min);
        }
    }
    best.0
}

/// Candidate chrono formats, in priority order. Day-first before month-first
/// when `prefer_day_first`; ambiguity is otherwise resolved by samples with a
/// first component > 12 (which eliminates month-first candidates).
fn date_format_candidates(prefer_day_first: bool) -> Vec<&'static str> {
    let mut iso = vec![
        "%Y-%m-%d",
        "%Y/%m/%d",
        "%Y.%m.%d",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y/%m/%d %H:%M",
        "%Y.%m.%d %H:%M:%S",
        "%Y.%m.%d %H:%M",
    ];
    let day_first = vec![
        "%d-%m-%Y",
        "%d/%m/%Y",
        "%d.%m.%Y",
        "%d-%m-%y",
        "%d/%m/%y",
        "%d-%m-%Y %H:%M",
        "%d/%m/%Y %H:%M",
        "%d.%m.%Y %H:%M",
        "%d-%m-%Y %H:%M:%S",
        "%d/%m/%Y %H:%M:%S",
    ];
    let month_first = vec![
        "%m-%d-%Y",
        "%m/%d/%Y",
        "%m.%d.%Y",
        "%m/%d/%y",
        "%m/%d/%Y %H:%M",
        "%m/%d/%Y %H:%M:%S",
    ];
    if prefer_day_first {
        iso.extend(day_first);
        iso.extend(month_first);
    } else {
        iso.extend(month_first);
        iso.extend(day_first);
    }
    iso
}

/// Return the candidate format that parses the most samples (ties resolved
/// by priority order). Requires a strict majority so a file full of garbage
/// still fails loudly, while a few malformed rows don't break detection.
fn detect_date_format(samples: &[&str], prefer_day_first: bool) -> Option<String> {
    if samples.is_empty() {
        return None;
    }
    let mut best: Option<(&'static str, usize)> = None;
    for fmt in date_format_candidates(prefer_day_first) {
        let parsed = samples
            .iter()
            .filter(|s| parse_date_with(s.trim(), fmt).is_some())
            .count();
        if parsed == samples.len() {
            return Some(fmt.to_string()); // perfect match wins immediately
        }
        if parsed > best.map(|(_, n)| n).unwrap_or(0) {
            best = Some((fmt, parsed));
        }
    }
    best.filter(|(_, n)| *n * 2 > samples.len())
        .map(|(fmt, _)| fmt.to_string())
}

/// Parse with a format that may or may not carry a time component.
fn parse_date_with(raw: &str, fmt: &str) -> Option<(NaiveDate, Option<NaiveTime>)> {
    if fmt.contains("%H") {
        NaiveDateTime::parse_from_str(raw, fmt)
            .ok()
            .map(|dt| (dt.date(), Some(dt.time())))
    } else {
        NaiveDate::parse_from_str(raw, fmt).ok().map(|d| (d, None))
    }
}

/// Inspect numeric columns: values shaped like `1.234,56` or `12,5` mean a
/// comma decimal separator.
fn detect_decimal_separator(
    records: &[csv::StringRecord],
    mapping: &HashMap<Field, usize>,
    delimiter: char,
) -> char {
    let numeric_fields = [
        Field::Pnl,
        Field::Entry,
        Field::Exit,
        Field::Risk,
        Field::Rr,
    ];
    let mut comma_votes = 0usize;
    let mut dot_votes = 0usize;
    for record in records.iter().take(200) {
        for field in numeric_fields {
            let Some(&idx) = mapping.get(&field) else {
                continue;
            };
            let Some(value) = record.get(idx) else {
                continue;
            };
            let v = value.trim().trim_start_matches('-');
            if v.is_empty() {
                continue;
            }
            let has_comma = v.contains(',');
            let has_dot = v.contains('.');
            match (has_comma, has_dot) {
                // Both present: the rightmost one is the decimal separator.
                (true, true) => {
                    if v.rfind(',') > v.rfind('.') {
                        comma_votes += 1;
                    } else {
                        dot_votes += 1;
                    }
                }
                (true, false) => comma_votes += 1,
                (false, true) => dot_votes += 1,
                (false, false) => {}
            }
        }
    }
    if comma_votes > dot_votes && delimiter != ',' {
        ','
    } else {
        '.'
    }
}

// --- content checks ---------------------------------------------------------

fn looks_like_date(raw: &str) -> bool {
    date_format_candidates(true)
        .iter()
        .any(|fmt| parse_date_with(raw, fmt).is_some())
}

fn looks_numeric(raw: &str) -> bool {
    parse_number(raw, '.').is_some() || parse_number(raw, ',').is_some()
}

/// At least `num/den` of the sampled values satisfy the predicate.
fn ratio_ok(samples: &[&str], num: usize, den: usize, pred: impl Fn(&str) -> bool) -> bool {
    if samples.is_empty() {
        return false;
    }
    samples.iter().filter(|s| pred(s)).count() * den >= samples.len() * num
}

/// Do the sampled values plausibly belong to this canonical field?
///
/// `num/den` is the required share of conforming values: a strict majority
/// (1/2) when the header *name* already matched an alias — a few malformed
/// rows must not break a well-named column — and 4/5 for the name-less
/// content rescue, where the values are the only evidence.
fn content_compatible(field: Field, samples: &[&str], num: usize, den: usize) -> bool {
    match field {
        Field::Date => ratio_ok(samples, num, den, looks_like_date),
        Field::Time => ratio_ok(samples, num, den, |s| parse_time(s).is_some()),
        Field::Direction => ratio_ok(samples, num, den, |s| Direction::parse(s).is_some()),
        Field::Entry
        | Field::Exit
        | Field::PositionSize
        | Field::Risk
        | Field::Reward
        | Field::Rr
        | Field::Pnl
        | Field::Fees => ratio_ok(samples, num, den, looks_numeric),
        // Free-text fields accept anything (including mostly-empty columns).
        Field::Symbol | Field::Strategy | Field::Session | Field::Notes => true,
    }
}

/// Map CSV headers to canonical fields.
///
/// Pass 1: explicit config mapping (exact header name or `col:N` index) —
///         trusted blindly, this is the user/AI override.
/// Pass 2: alias matching in field-priority order, but a column is only
///         accepted when its *content* matches the field type.
/// Pass 3: date rescue — when no date column was found by name, take the
///         first unmapped column whose values parse as dates/datetimes.
fn map_headers(
    headers: &[String],
    records: &[csv::StringRecord],
    cfg: &CsvConfig,
) -> Result<HashMap<Field, usize>> {
    let mut mapping: HashMap<Field, usize> = HashMap::new();

    // Sampled non-empty values per column, for content checks.
    let samples: Vec<Vec<&str>> = (0..headers.len())
        .map(|idx| {
            records
                .iter()
                .filter_map(|r| r.get(idx))
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .take(CONTENT_SAMPLES)
                .collect()
        })
        .collect();

    // Pass 1: explicit mapping.
    for field in Field::all() {
        let Some(wanted) = cfg.mapping.get(field.canonical_name()) else {
            continue;
        };
        let idx = if let Some(n) = wanted.strip_prefix("col:") {
            let idx: usize = n.trim().parse().with_context(|| {
                format!(
                    "invalid column index {wanted:?} for {}",
                    field.canonical_name()
                )
            })?;
            if idx >= headers.len() {
                bail!(
                    "mapping {} = {wanted:?} is out of range ({} columns)",
                    field.canonical_name(),
                    headers.len()
                );
            }
            idx
        } else {
            match headers.iter().position(|h| h.eq_ignore_ascii_case(wanted)) {
                Some(idx) => idx,
                None => bail!(
                    "mapping {} = {wanted:?} does not match any header {:?} — use the exact header name or col:N",
                    field.canonical_name(),
                    headers
                ),
            }
        };
        mapping.insert(*field, idx);
    }

    // Pass 2: content-confirmed alias matching, alias priority first.
    let used = |mapping: &HashMap<Field, usize>, idx: usize| mapping.values().any(|i| *i == idx);
    'fields: for field in Field::all() {
        if mapping.contains_key(field) {
            continue;
        }
        for alias in field.aliases() {
            for (idx, header) in headers.iter().enumerate() {
                if used(&mapping, idx) {
                    continue;
                }
                if normalize_header(header) == *alias
                    && content_compatible(*field, &samples[idx], 1, 2)
                {
                    mapping.insert(*field, idx);
                    continue 'fields;
                }
            }
        }
    }

    // Pass 3: date rescue by content. Prefer columns whose header hints at
    // a date/time; fall back to any column holding date-like values.
    if !mapping.contains_key(&Field::Date) {
        let hint = |h: &str| {
            let n = normalize_header(h);
            ["date", "time", "open", "created", "day"]
                .iter()
                .any(|k| n.contains(k))
        };
        let candidate = headers
            .iter()
            .enumerate()
            .filter(|(idx, _)| {
                !used(&mapping, *idx) && content_compatible(Field::Date, &samples[*idx], 4, 5)
            })
            .min_by_key(|(idx, h)| (!hint(h), *idx));
        if let Some((idx, _)) = candidate {
            mapping.insert(Field::Date, idx);
        }
    }

    if !mapping.contains_key(&Field::Date) {
        bail!(missing_column_help("date", headers));
    }
    if !mapping.contains_key(&Field::Symbol) {
        bail!(missing_column_help("symbol", headers));
    }
    if !mapping.contains_key(&Field::Pnl) {
        bail!(missing_column_help("pnl", headers));
    }
    Ok(mapping)
}

fn normalize_header(header: &str) -> String {
    header
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '$')
        .collect()
}

/// Parse one number, tolerating currency symbols, spaces, percent signs and
/// either decimal separator convention.
fn parse_number(raw: &str, decimal_separator: char) -> Option<f64> {
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|c| !matches!(c, '$' | '€' | '£' | '%' | ' ' | '\u{a0}' | '+'))
        .collect();
    if cleaned.is_empty() || cleaned == "-" {
        return None;
    }
    let normalized = if decimal_separator == ',' {
        // 1.234,56 -> 1234.56
        cleaned.replace('.', "").replace(',', ".")
    } else {
        // 1,234.56 -> 1234.56
        cleaned.replace(',', "")
    };
    normalized.parse::<f64>().ok()
}

fn get<'a>(
    record: &'a csv::StringRecord,
    mapping: &HashMap<Field, usize>,
    field: Field,
) -> Option<&'a str> {
    mapping
        .get(&field)
        .and_then(|idx| record.get(*idx))
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

fn build_trade(
    row_number: usize,
    record: &csv::StringRecord,
    mapping: &HashMap<Field, usize>,
    date_format: &str,
    decimal_separator: char,
) -> Result<Trade> {
    let date_raw = get(record, mapping, Field::Date).context("missing date")?;
    let (date, embedded_time) = parse_date_with(date_raw, date_format).with_context(|| {
        format!("unparseable date {date_raw:?} (expected format {date_format})")
    })?;

    let time = match get(record, mapping, Field::Time) {
        Some(raw) => Some(parse_time(raw).with_context(|| format!("unparseable time {raw:?}"))?),
        None => embedded_time,
    };

    let symbol = get(record, mapping, Field::Symbol)
        .context("missing symbol")?
        .to_uppercase();

    let pnl_raw = get(record, mapping, Field::Pnl).context("missing pnl")?;
    let pnl = parse_number(pnl_raw, decimal_separator)
        .with_context(|| format!("unparseable pnl {pnl_raw:?}"))?;

    let num = |field: Field| {
        get(record, mapping, field).and_then(|raw| parse_number(raw, decimal_separator))
    };

    Ok(Trade {
        index: row_number,
        date,
        time,
        symbol,
        direction: get(record, mapping, Field::Direction).and_then(Direction::parse),
        entry: num(Field::Entry),
        exit: num(Field::Exit),
        position_size: num(Field::PositionSize),
        risk: num(Field::Risk).map(f64::abs),
        reward: num(Field::Reward),
        rr: num(Field::Rr),
        pnl,
        // Brokers report costs with mixed signs; store the magnitude.
        fees: num(Field::Fees).map(f64::abs),
        strategy: get(record, mapping, Field::Strategy).map(str::to_string),
        session: get(record, mapping, Field::Session).and_then(Session::parse),
        notes: get(record, mapping, Field::Notes).map(str::to_string),
    })
}

fn parse_time(raw: &str) -> Option<NaiveTime> {
    for fmt in ["%H:%M:%S", "%H:%M", "%I:%M %p", "%I:%M:%S %p"] {
        if let Ok(t) = NaiveTime::parse_from_str(raw, fmt) {
            return Some(t);
        }
    }
    None
}

/// Structured sample of a CSV for the AI mapping workflow: headers, sample
/// rows, canonical field docs and the engine's current detection attempt.
/// This is what the AI reads instead of the raw file.
pub fn inspect_csv(path: &Path, cfg: &CsvConfig, sample_rows: usize) -> Result<serde_json::Value> {
    let (headers, records, delimiter, encoding) = read_table(path, cfg)?;

    let header_list: Vec<serde_json::Value> = headers
        .iter()
        .enumerate()
        .map(|(index, name)| json!({ "index": index, "name": name }))
        .collect();
    let sample: Vec<Vec<String>> = records
        .iter()
        .take(sample_rows)
        .map(|r| r.iter().map(str::to_string).collect())
        .collect();
    let canonical_fields: Vec<serde_json::Value> = Field::all()
        .iter()
        .map(|f| {
            json!({
                "name": f.canonical_name(),
                "required": f.required(),
                "description": f.description(),
            })
        })
        .collect();

    // Run the real parser to show what detection currently concludes.
    let detection = match parse_csv(path, cfg) {
        Ok(report) => {
            let mapped: serde_json::Map<String, serde_json::Value> = report
                .detected
                .mapped_headers
                .iter()
                .map(|(header, field)| (field.to_string(), json!(header)))
                .collect();
            json!({
                "status": "ok",
                "date_format": report.detected.date_format,
                "decimal_separator": report.detected.decimal_separator.to_string(),
                "mapped": mapped,
                "unmapped_headers": report.detected.unmapped_headers,
                "trial_parse": {
                    "trades": report.trades.len(),
                    "skipped_rows": report.skipped_rows,
                    "warnings": report.warnings.iter().take(10).collect::<Vec<_>>(),
                },
            })
        }
        Err(err) => json!({ "status": "error", "error": format!("{err:#}") }),
    };

    Ok(json!({
        "file": path.display().to_string(),
        "encoding": encoding,
        "delimiter": delimiter.to_string(),
        "total_rows": records.len(),
        "headers": header_list,
        "sample_rows": sample,
        "canonical_fields": canonical_fields,
        "detection": detection,
        "mapping_file_howto": {
            "path": format!("{}.mapping.toml", path.display()),
            "syntax": "top-level: delimiter / date_format / decimal_separator / prefer_day_first; [mapping] <canonical_field> = \"<exact header>\" or \"col:<0-based index>\" for duplicate headers",
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp(content: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("trade-analyst-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("t{}.csv", content.len()));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn parses_standard_csv() {
        let path = write_temp(
            "Date,Time,Pair,Direction,Entry,Exit,Size,Risk,RR,PnL,Strategy,Session\n\
             2026-01-05,08:30,EURUSD,Long,1.0450,1.0480,1.0,100,2.0,200,Breakout,London\n\
             2026-01-06,14:15,GBPUSD,Short,1.2700,1.2750,0.5,50,1.5,-50,Reversal,New York\n",
        );
        let report = parse_csv(&path, &CsvConfig::default()).unwrap();
        assert_eq!(report.trades.len(), 2);
        assert_eq!(report.skipped_rows, 0);
        assert_eq!(report.detected.delimiter, ',');
        let t = &report.trades[0];
        assert_eq!(t.symbol, "EURUSD");
        assert_eq!(t.direction, Some(Direction::Long));
        assert_eq!(t.pnl, 200.0);
        assert_eq!(t.session, Some(Session::London));
    }

    #[test]
    fn parses_mt_style_export_without_config() {
        // MetaTrader-style: "Open"/"Close" are datetimes, duplicate "Price"
        // columns (entry & exit), Type=buy/sell, Volume, Swap + Commissions.
        let path = write_temp(
            "Ticket,Open,Type,Volume,Symbol,Price,SL,TP,Close,Price,Swap,Commissions,Profit,Pips\n\
             35785874,\"2024-09-06 14:12:23\",buy,50,USDJPY,142.885,142.885,142.959,\"2024-09-06 14:23:46\",142.958,0,-150,2553.20,0\n\
             35785377,\"2024-09-06 14:08:05\",sell,38.29,USDJPY,142.856,142.946,142.058,\"2024-09-06 14:12:07\",142.893,0,-114.88,-991.46,0\n\
             35773413,\"2024-09-06 12:48:31\",buy,22.51,GBPUSD,1.31709,1.31701,1.31929,\"2024-09-06 13:08:36\",1.31714,0,-67.54,112.55,0\n",
        );
        let report = parse_csv(&path, &CsvConfig::default()).unwrap();
        assert_eq!(report.trades.len(), 3, "warnings: {:?}", report.warnings);
        assert_eq!(report.skipped_rows, 0);
        assert_eq!(report.detected.date_format, "%Y-%m-%d %H:%M:%S");

        let t = &report.trades[0];
        assert_eq!(t.date, NaiveDate::from_ymd_opt(2024, 9, 6).unwrap());
        assert_eq!(t.time, Some(NaiveTime::from_hms_opt(14, 12, 23).unwrap()));
        assert_eq!(t.symbol, "USDJPY");
        assert_eq!(t.direction, Some(Direction::Long));
        assert_eq!(t.position_size, Some(50.0));
        assert_eq!(t.entry, Some(142.885)); // first Price column
        assert_eq!(t.exit, Some(142.958)); // second Price column
        assert_eq!(t.fees, Some(150.0)); // Commissions, sign normalized
        assert_eq!(t.pnl, 2553.20);
        assert_eq!(report.trades[1].direction, Some(Direction::Short));
    }

    #[test]
    fn content_beats_header_names() {
        // A column literally named "Date" holding prices must not win over
        // a datetime column named "Executed".
        let path = write_temp(
            "Date,Executed,Symbol,PnL\n\
             1.2345,2026-01-05 09:00:00,EURUSD,100\n\
             1.2350,2026-01-06 10:30:00,EURUSD,-40\n",
        );
        let report = parse_csv(&path, &CsvConfig::default()).unwrap();
        assert_eq!(report.trades.len(), 2);
        assert_eq!(
            report.trades[0].date,
            NaiveDate::from_ymd_opt(2026, 1, 5).unwrap()
        );
        assert_eq!(
            report.trades[0].time,
            Some(NaiveTime::from_hms_opt(9, 0, 0).unwrap())
        );
    }

    #[test]
    fn column_index_mapping() {
        let path = write_temp(
            "A,B,C,D\n\
             2026-01-05,EURUSD,x,100\n\
             2026-01-06,GBPUSD,x,-50\n",
        );
        let mut cfg = CsvConfig::default();
        cfg.mapping.insert("date".into(), "col:0".into());
        cfg.mapping.insert("symbol".into(), "col:1".into());
        cfg.mapping.insert("pnl".into(), "col:3".into());
        let report = parse_csv(&path, &cfg).unwrap();
        assert_eq!(report.trades.len(), 2);
        assert_eq!(report.trades[0].symbol, "EURUSD");
        assert_eq!(report.trades[1].pnl, -50.0);
    }

    #[test]
    fn bad_explicit_mapping_fails_loudly() {
        let path = write_temp("Date,Symbol,PnL\n2026-01-05,EURUSD,100\n");
        let mut cfg = CsvConfig::default();
        cfg.mapping.insert("pnl".into(), "Nonexistent".into());
        let err = parse_csv(&path, &cfg).unwrap_err().to_string();
        assert!(err.contains("does not match any header"), "{err}");
    }

    #[test]
    fn detects_semicolon_and_comma_decimals() {
        let path = write_temp(
            "Datum;Pair;Winst\n\
             05-01-2026;EURUSD;1.234,50\n\
             06-01-2026;GBPUSD;-12,25\n",
        );
        let mut cfg = CsvConfig::default();
        cfg.mapping.insert("date".into(), "Datum".into());
        cfg.mapping.insert("pnl".into(), "Winst".into());
        let report = parse_csv(&path, &cfg).unwrap();
        assert_eq!(report.detected.delimiter, ';');
        assert_eq!(report.detected.decimal_separator, ',');
        assert_eq!(report.trades[0].pnl, 1234.50);
        assert_eq!(report.trades[1].pnl, -12.25);
        assert_eq!(report.detected.date_format, "%d-%m-%Y");
    }

    #[test]
    fn skips_malformed_rows() {
        let path = write_temp(
            "Date,Pair,PnL\n\
             2026-01-05,EURUSD,100\n\
             not-a-date,EURUSD,50\n\
             2026-01-07,GBPUSD,\n\
             2026-01-08,USDJPY,-30\n",
        );
        let report = parse_csv(&path, &CsvConfig::default()).unwrap();
        assert_eq!(report.trades.len(), 2);
        assert_eq!(report.skipped_rows, 2);
        assert_eq!(report.warnings.len(), 2);
    }

    #[test]
    fn ambiguous_dates_prefer_day_first() {
        let samples = vec!["03/02/2026", "04/02/2026"];
        assert_eq!(detect_date_format(&samples, true).unwrap(), "%d/%m/%Y");
        assert_eq!(detect_date_format(&samples, false).unwrap(), "%m/%d/%Y");
        // 13 in first position rules out month-first regardless of preference
        let samples = vec!["13/02/2026"];
        assert_eq!(detect_date_format(&samples, false).unwrap(), "%d/%m/%Y");
    }

    #[test]
    fn number_parsing_handles_currency_symbols() {
        assert_eq!(parse_number("$1,250.75", '.'), Some(1250.75));
        assert_eq!(parse_number("€ -12,50", ','), Some(-12.50));
        assert_eq!(parse_number("+150", '.'), Some(150.0));
        assert_eq!(parse_number("", '.'), None);
    }

    #[test]
    fn datetime_in_date_column() {
        let path = write_temp(
            "Open Time,Symbol,Profit\n\
             2026-01-05 08:30:00,EURUSD,100\n\
             2026-01-05 15:45:00,EURUSD,-40\n",
        );
        let report = parse_csv(&path, &CsvConfig::default()).unwrap();
        assert_eq!(report.trades.len(), 2);
        assert_eq!(
            report.trades[0].time,
            Some(NaiveTime::from_hms_opt(8, 30, 0).unwrap())
        );
    }

    #[test]
    fn inspect_emits_structured_sample() {
        let path = write_temp(
            "Weird1,Weird2,Weird3\n\
             a,b,c\n\
             d,e,f\n",
        );
        let value = inspect_csv(&path, &CsvConfig::default(), 5).unwrap();
        assert_eq!(value["headers"].as_array().unwrap().len(), 3);
        assert_eq!(value["sample_rows"].as_array().unwrap().len(), 2);
        assert_eq!(value["detection"]["status"], "error"); // nothing mappable
        assert!(value["canonical_fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["name"] == "pnl" && f["required"] == true));

        // and a healthy file reports its trial parse
        let ok = write_temp("Date,Symbol,PnL\n2026-01-05,EURUSD,100\n");
        let value = inspect_csv(&ok, &CsvConfig::default(), 5).unwrap();
        assert_eq!(value["detection"]["status"], "ok");
        assert_eq!(value["detection"]["trial_parse"]["trades"], 1);
    }
}
