//! 13F filing ingestion and normalization.
//!
//! Handles parsing of SEC 13F filings from XML/CSV formats, CUSIP-to-ticker
//! resolution, portfolio weight computation, and quarterly aggregation.

use crate::types::{CusipMap, Filing13F, Holding, Manager, Sector};
use anyhow::{Context, Result};
use chrono::{Datelike, NaiveDate};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Validation errors
// ---------------------------------------------------------------------------

/// Errors produced during ingestion and normalization.
#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("Missing required field: {field}")]
    MissingField { field: String },
    #[error("Invalid CUSIP: {cusip}")]
    InvalidCusip { cusip: String },
    #[error("Invalid date format: {raw}")]
    InvalidDate { raw: String },
    #[error("Holding value negative: {value}")]
    NegativeValue { value: f64 },
    #[error("Duplicate CUSIP in filing: {cusip}")]
    DuplicateCusip { cusip: String },
    #[error("Filing has zero total AUM")]
    ZeroAum,
}

// ---------------------------------------------------------------------------
// Raw holding row
// ---------------------------------------------------------------------------

/// Intermediate holding representation before normalization.
#[derive(Debug, Clone)]
pub struct RawHolding {
    pub cusip: String,
    pub ticker: Option<String>,
    pub name: Option<String>,
    pub shares: Option<i64>,
    pub value: Option<f64>,
    pub share_type: Option<String>,
    pub discretion: Option<String>,
    pub is_option: bool,
}

// ---------------------------------------------------------------------------
// XML / CSV parsing
// ---------------------------------------------------------------------------

/// Parses a single XML information table row into a RawHolding.
pub fn parse_xml_holding(xml: &str) -> Result<RawHolding, IngestError> {
    let mut raw = RawHolding {
        cusip: String::new(),
        ticker: None,
        name: None,
        shares: None,
        value: None,
        share_type: None,
        discretion: None,
        is_option: false,
    };

    raw.cusip = extract_tag(xml, "cusip")
        .unwrap_or_default()
        .replace('-', "")
        .replace(" ", "");
    if raw.cusip.is_empty() {
        return Err(IngestError::MissingField { field: "cusip".into() });
    }
    if raw.cusip.len() < 6 {
        return Err(IngestError::InvalidCusip { cusip: raw.cusip.clone() });
    }

    raw.ticker = extract_tag(xml, "ticker").filter(|s| !s.is_empty());
    raw.name = extract_tag(xml, "nameOfIssuer").or_else(|| extract_tag(xml, "name"));
    raw.shares = extract_tag(xml, "sshPrnamt")
        .or_else(|| extract_tag(xml, "shares"))
        .and_then(|s| s.parse::<i64>().ok());
    raw.value = extract_tag(xml, "value")
        .or_else(|| extract_tag(xml, "ssValue"))
        .and_then(|s| s.parse::<f64>().ok().map(|v| v * 1000.0));
    raw.share_type = extract_tag(xml, "sshPrmType");
    raw.discretion = extract_tag(xml, "investmentDiscretion");

    if let Some(put_call) = extract_tag(xml, "putCall") {
        raw.is_option = !put_call.is_empty();
    }

    Ok(raw)
}

/// Parses a CSV row into a RawHolding.
/// Columns: cusip,ticker,name,shares,value,share_type,discretion,is_option
pub fn parse_csv_holding(row: &str) -> Result<RawHolding, IngestError> {
    let parts: Vec<&str> = row.trim().split(',').collect();
    if parts.len() < 4 {
        return Err(IngestError::MissingField {
            field: format!("expected >= 4 columns, got {}", parts.len()),
        });
    }

    let cusip = parts[0].replace('-', "").replace(" ", "");
    if cusip.is_empty() {
        return Err(IngestError::MissingField { field: "cusip".into() });
    }

    let shares = parts[3].parse::<i64>().ok();
    let value = parts.get(4).and_then(|s| s.parse::<f64>().ok()).map(|v| v * 1000.0);

    let raw = RawHolding {
        cusip,
        ticker: parts.get(1).map(|s| s.to_string()).filter(|s| !s.is_empty()),
        name: parts.get(2).map(|s| s.to_string()).filter(|s| !s.is_empty()),
        shares,
        value,
        share_type: parts.get(5).map(|s| s.to_string()).filter(|s| !s.is_empty()),
        discretion: parts.get(6).map(|s| s.to_string()).filter(|s| !s.is_empty()),
        is_option: parts.get(7).map(|s| *s == "true" || *s == "1").unwrap_or(false),
    };

    Ok(raw)
}

// ---------------------------------------------------------------------------
// Normalization
// ---------------------------------------------------------------------------

/// Normalize a RawHolding into a Holding using the CUSIP map.
pub fn normalize_holding(raw: &RawHolding, cusip_map: &CusipMap, total_aum: f64) -> Result<Holding> {
    let shares = raw.shares.context(IngestError::MissingField { field: "shares".into() })?;
    let value = raw.value.context(IngestError::MissingField { field: "value".into() })?;

    if value < 0.0 {
        return Err(IngestError::NegativeValue { value }.into());
    }

    let (ticker, name) = match raw.ticker.as_deref() {
        Some(t) if !t.is_empty() => {
            let n = raw.name.as_deref().unwrap_or("Unknown");
            (t.to_string(), n.to_string())
        }
        _ => match cusip_map.get(&raw.cusip) {
            Some((t, n)) => (t.to_string(), n.to_string()),
            None => (raw.cusip.clone(), "Unknown".into()),
        }
    };

    let sector = Sector::classify_ticker(&ticker);
    let portfolio_weight = if total_aum > 0.0 { value / total_aum } else { 0.0 };

    Ok(Holding {
        cusip: raw.cusip.clone(),
        ticker,
        name,
        sector,
        shares,
        value,
        share_type: raw.share_type.clone().unwrap_or_else(|| "SH".into()),
        discretion: raw.discretion.clone().unwrap_or_else(|| "SO".into()),
        is_option: raw.is_option,
        portfolio_weight,
    })
}

// ---------------------------------------------------------------------------
// Filing builder
// ---------------------------------------------------------------------------

/// Builder for constructing a normalized Filing13F from raw data.
pub struct FilingBuilder {
    accession: String,
    manager: Manager,
    report_date: NaiveDate,
    filing_date: NaiveDate,
    total_aum: f64,
    raw_holdings: Vec<RawHolding>,
    other_included: usize,
}

impl FilingBuilder {
    pub fn new(manager: Manager, report_date: NaiveDate) -> Self {
        Self {
            accession: String::new(),
            manager,
            report_date,
            filing_date: report_date,
            total_aum: 0.0,
            raw_holdings: Vec::new(),
            other_included: 0,
        }
    }

    pub fn accession(&mut self, a: &str) -> &mut Self { self.accession = a.into(); self }
    pub fn filing_date(&mut self, d: NaiveDate) -> &mut Self { self.filing_date = d; self }
    pub fn total_aum(&mut self, aum: f64) -> &mut Self { self.total_aum = aum; self }
    pub fn other_included(&mut self, n: usize) -> &mut Self { self.other_included = n; self }

    pub fn add_raw(&mut self, raw: RawHolding) {
        self.raw_holdings.push(raw);
    }

    pub fn add_xml_holding(&mut self, xml: &str) -> Result<()> {
        let raw = parse_xml_holding(xml)?;
        self.raw_holdings.push(raw);
        Ok(())
    }

    pub fn add_csv_holding(&mut self, row: &str) -> Result<()> {
        let raw = parse_csv_holding(row)?;
        self.raw_holdings.push(raw);
        Ok(())
    }

    /// Build the final normalized Filing13F.
    pub fn build(self, cusip_map: &CusipMap) -> Result<Filing13F> {
        let aum = if self.total_aum > 0.0 {
            self.total_aum
        } else {
            let derived_aum: f64 = self.raw_holdings.iter().filter_map(|r| r.value).sum();
            if derived_aum <= 0.0 {
                return Err(IngestError::ZeroAum.into());
            }
            derived_aum
        };

        let mut seen_cusips = HashMap::new();
        let mut holdings = Vec::new();

        for raw in &self.raw_holdings {
            if seen_cusips.contains_key(&raw.cusip) {
                return Err(IngestError::DuplicateCusip { cusip: raw.cusip.clone() }.into());
            }
            seen_cusips.insert(raw.cusip.clone(), true);
            let h = normalize_holding(raw, cusip_map, aum)?;
            holdings.push(h);
        }

        holdings.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap_or(std::cmp::Ordering::Equal));

        Ok(Filing13F {
            accession_number: self.accession,
            manager: self.manager,
            report_date: self.report_date,
            filing_date: self.filing_date,
            total_aum: aum,
            holdings,
            other_included_count: self.other_included,
        })
    }
}

// ---------------------------------------------------------------------------
// Quarterly aggregation
// ---------------------------------------------------------------------------

/// Aggregates multiple filings by quarter.
pub struct QuarterlyAggregator {
    filings: Vec<Filing13F>,
}

impl QuarterlyAggregator {
    pub fn new() -> Self {
        Self { filings: Vec::new() }
    }

    pub fn add(&mut self, filing: Filing13F) {
        self.filings.push(filing);
    }

    /// Group filings by quarter (e.g. "2024-Q1").
    pub fn by_quarter(&self) -> HashMap<String, Vec<&Filing13F>> {
        let mut map: HashMap<String, Vec<&Filing13F>> = HashMap::new();
        for f in &self.filings {
            let q = quarter_key(f.report_date);
            map.entry(q).or_default().push(f);
        }
        map
    }

    /// Get the most recent quarter key.
    pub fn latest_quarter(&self) -> Option<String> {
        let binding = self.by_quarter();
        let quarters: Vec<&String> = binding.keys().collect();
        quarters.into_iter().max().cloned()
    }

    /// Get filings for a specific quarter.
    pub fn filings_for_quarter(&self, quarter: &str) -> Vec<&Filing13F> {
        self.by_quarter().get(quarter).cloned().unwrap_or_default()
    }

    /// Total number of filings.
    pub fn len(&self) -> usize {
        self.filings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.filings.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Data validation
// ---------------------------------------------------------------------------

/// Validates a Filing13F for internal consistency.
pub fn validate_filing(filing: &Filing13F) -> Result<Vec<String>> {
    let mut warnings = Vec::new();

    if filing.holdings.is_empty() {
        warnings.push("Filing has zero holdings".into());
    }

    if filing.total_aum <= 0.0 {
        warnings.push("Total AUM is zero or negative".into());
    }

    let total_value: f64 = filing.holdings.iter().map(|h| h.value).sum();
    let weight_sum: f64 = filing.holdings.iter().map(|h| h.portfolio_weight).sum();

    if filing.total_aum > 0.0 {
        let coverage = total_value / filing.total_aum;
        if coverage < 0.9 {
            warnings.push(format!(
                "Holdings cover only {:.1}% of reported AUM (total holdings: ${:.0}, AUM: ${:.0})",
                coverage * 100.0, total_value, filing.total_aum
            ));
        }
    }

    if (weight_sum - 1.0).abs() > 0.1 {
        warnings.push(format!("Portfolio weights sum to {:.4}, expected ~1.0", weight_sum));
    }

    for h in &filing.holdings {
        if h.shares <= 0 {
            warnings.push(format!("{} has non-positive shares: {}", h.ticker, h.shares));
        }
        if h.value <= 0.0 && !h.is_option {
            warnings.push(format!("{} has non-positive value: ${:.2}", h.ticker, h.value));
        }
    }

    Ok(warnings)
}

// ---------------------------------------------------------------------------
// Utility helpers
// ---------------------------------------------------------------------------

fn extract_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    if let Some(start) = xml.find(&open) {
        let content_start = start + open.len();
        if let Some(end) = xml[content_start..].find(&close) {
            let val = xml[content_start..content_start + end].trim().to_string();
            if val.is_empty() { None } else { Some(val) }
        } else {
            None
        }
    } else {
        None
    }
}

fn quarter_key(date: NaiveDate) -> String {
    let y = date.year();
    let m = date.month();
    let q = (m - 1) / 3 + 1;
    format!("{}-Q{}", y, q)
}

/// Parse a date string in "YYYY-MM-DD" format.
pub fn parse_date(s: &str) -> Result<NaiveDate, IngestError> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| IngestError::InvalidDate { raw: s.into() })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cusip_map() -> CusipMap {
        let mut m = CusipMap::new();
        m.insert("037833100", "AAPL", "Apple Inc");
        m.insert("478160104", "JNJ", "Johnson & Johnson");
        m.insert("02079K305", "GOOGL", "Alphabet Inc");
        m.insert("594918104", "MSFT", "Microsoft Corp");
        m.insert("46647Q103", "JPM", "JPMorgan Chase");
        m
    }

    fn make_test_filing() -> Filing13F {
        Filing13F {
            accession_number: "000000000000-01-000001".into(),
            manager: Manager {
                cik: "0001234567".into(),
                name: "Test Capital".into(),
                filer_type: "HA".into(),
            },
            report_date: NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
            filing_date: NaiveDate::from_ymd_opt(2024, 5, 14).unwrap(),
            total_aum: 1_000_000.0,
            holdings: vec![
                Holding {
                    cusip: "037833100".into(),
                    ticker: "AAPL".into(),
                    name: "Apple Inc".into(),
                    sector: Sector::Technology,
                    shares: 1000,
                    value: 500_000.0,
                    share_type: "SH".into(),
                    discretion: "SO".into(),
                    is_option: false,
                    portfolio_weight: 0.5,
                },
                Holding {
                    cusip: "478160104".into(),
                    ticker: "JNJ".into(),
                    name: "Johnson & Johnson".into(),
                    sector: Sector::Healthcare,
                    shares: 500,
                    value: 500_000.0,
                    share_type: "SH".into(),
                    discretion: "SO".into(),
                    is_option: false,
                    portfolio_weight: 0.5,
                },
            ],
            other_included_count: 0,
        }
    }

    #[test]
    fn test_parse_xml_holding() {
        let xml = r#"
            <infoTable>
                <cusip>037833100</cusip>
                <ticker>AAPL</ticker>
                <nameOfIssuer>Apple Inc</nameOfIssuer>
                <sshPrnamt>1000</sshPrnamt>
                <value>500</value>
                <sshPrmType>SH</sshPrmType>
                <investmentDiscretion>SO</investmentDiscretion>
            </infoTable>
        "#;
        let raw = parse_xml_holding(xml).unwrap();
        assert_eq!(raw.cusip, "037833100");
        assert_eq!(raw.ticker.as_deref(), Some("AAPL"));
        assert_eq!(raw.shares, Some(1000));
        assert!((raw.value.unwrap() - 500_000.0).abs() < 1.0);
    }

    #[test]
    fn test_parse_xml_missing_cusip() {
        let xml = "<infoTable><ticker>AAPL</ticker></infoTable>";
        let result = parse_xml_holding(xml);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_csv_holding() {
        let row = "037833100,AAPL,Apple Inc,1000,500.0,SH,SO,false";
        let raw = parse_csv_holding(row).unwrap();
        assert_eq!(raw.cusip, "037833100");
        assert_eq!(raw.ticker.as_deref(), Some("AAPL"));
        assert_eq!(raw.shares, Some(1000));
        assert!((raw.value.unwrap() - 500_000.0).abs() < 1.0);
        assert!(!raw.is_option);
    }

    #[test]
    fn test_parse_csv_option() {
        let row = "037833100,AAPL,Apple Inc,500,200.0,PUT,DS,true";
        let raw = parse_csv_holding(row).unwrap();
        assert!(raw.is_option);
    }

    #[test]
    fn test_parse_csv_too_few_cols() {
        let row = "037833100";
        let result = parse_csv_holding(row);
        assert!(result.is_err());
    }

    #[test]
    fn test_normalize_holding() {
        let raw = RawHolding {
            cusip: "037833100".into(), ticker: Some("AAPL".into()),
            name: Some("Apple Inc".into()), shares: Some(1000),
            value: Some(500_000.0), share_type: Some("SH".into()),
            discretion: Some("SO".into()), is_option: false,
        };
        let map = test_cusip_map();
        let h = normalize_holding(&raw, &map, 1_000_000.0).unwrap();
        assert_eq!(h.ticker, "AAPL");
        assert_eq!(h.sector, Sector::Technology);
        assert!((h.portfolio_weight - 0.5).abs() < 1e-9);
    }

    #[test]
    fn test_normalize_holding_negative_value() {
        let raw = RawHolding {
            cusip: "037833100".into(), ticker: Some("AAPL".into()),
            name: Some("Apple Inc".into()), shares: Some(1000),
            value: Some(-500.0), share_type: Some("SH".into()),
            discretion: Some("SO".into()), is_option: false,
        };
        let map = test_cusip_map();
        let result = normalize_holding(&raw, &map, 1_000_000.0);
        assert!(result.is_err());
    }

    #[test]
    fn test_normalize_holding_cusip_fallback() {
        let raw = RawHolding {
            cusip: "037833100".into(), ticker: None, name: None,
            shares: Some(1000), value: Some(500_000.0),
            share_type: None, discretion: None, is_option: false,
        };
        let map = test_cusip_map();
        let h = normalize_holding(&raw, &map, 1_000_000.0).unwrap();
        assert_eq!(h.ticker, "AAPL");
        assert_eq!(h.name, "Apple Inc");
    }

    #[test]
    fn test_filing_builder() {
        let map = test_cusip_map();
        let mgr = Manager {
            cik: "0001234567".into(), name: "Test Capital".into(), filer_type: "HA".into(),
        };
        let mut builder = FilingBuilder::new(mgr, NaiveDate::from_ymd_opt(2024, 3, 31).unwrap());
        builder.accession("000000000000-01-000001");
        builder.total_aum(1_000_000.0);
        builder.add_csv_holding("037833100,AAPL,Apple Inc,1000,500.0,SH,SO,false").unwrap();
        builder.add_csv_holding("478160104,JNJ,Johnson & Johnson,500,500.0,SH,SO,false").unwrap();

        let filing = builder.build(&map).unwrap();
        assert_eq!(filing.holdings.len(), 2);
        assert_eq!(filing.total_aum, 1_000_000.0);
    }

    #[test]
    fn test_filing_builder_duplicate_cusip() {
        let map = test_cusip_map();
        let mgr = Manager {
            cik: "0001234567".into(), name: "Test Capital".into(), filer_type: "HA".into(),
        };
        let mut builder = FilingBuilder::new(mgr, NaiveDate::from_ymd_opt(2024, 3, 31).unwrap());
        builder.total_aum(1_000_000.0);
        builder.add_csv_holding("037833100,AAPL,Apple Inc,1000,500.0,SH,SO,false").unwrap();
        builder.add_csv_holding("037833100,AAPL,Apple Inc,1000,500.0,SH,SO,false").unwrap();
        let result = builder.build(&map);
        assert!(result.is_err());
    }

    #[test]
    fn test_quarterly_aggregator() {
        let mut agg = QuarterlyAggregator::new();
        let f1 = make_filing("A", NaiveDate::from_ymd_opt(2024, 3, 31).unwrap());
        let f2 = make_filing("B", NaiveDate::from_ymd_opt(2024, 3, 31).unwrap());
        let f3 = make_filing("C", NaiveDate::from_ymd_opt(2024, 6, 30).unwrap());
        agg.add(f1);
        agg.add(f2);
        agg.add(f3);

        let by_q = agg.by_quarter();
        assert_eq!(by_q["2024-Q1"].len(), 2);
        assert_eq!(by_q["2024-Q2"].len(), 1);
        assert_eq!(agg.latest_quarter().as_deref(), Some("2024-Q2"));
    }

    #[test]
    fn test_validate_filing_ok() {
        let filing = make_test_filing();
        let warnings = validate_filing(&filing).unwrap();
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_validate_filing_empty() {
        let filing = Filing13F {
            accession_number: String::new(),
            manager: Manager {
                cik: "0000000000".into(), name: "Empty Fund".into(), filer_type: "HA".into(),
            },
            report_date: NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
            filing_date: NaiveDate::from_ymd_opt(2024, 5, 14).unwrap(),
            total_aum: 0.0,
            holdings: vec![],
            other_included_count: 0,
        };
        let warnings = validate_filing(&filing).unwrap();
        assert!(!warnings.is_empty());
    }

    #[test]
    fn test_quarter_key() {
        assert_eq!(quarter_key(NaiveDate::from_ymd_opt(2024, 1, 15).unwrap()), "2024-Q1");
        assert_eq!(quarter_key(NaiveDate::from_ymd_opt(2024, 4, 30).unwrap()), "2024-Q2");
        assert_eq!(quarter_key(NaiveDate::from_ymd_opt(2024, 12, 31).unwrap()), "2024-Q4");
    }

    #[test]
    fn test_parse_date() {
        let d = parse_date("2024-03-31").unwrap();
        assert_eq!(d, NaiveDate::from_ymd_opt(2024, 3, 31).unwrap());
    }

    #[test]
    fn test_parse_date_invalid() {
        let result = parse_date("not-a-date");
        assert!(result.is_err());
    }

    fn make_filing(name: &str, date: NaiveDate) -> Filing13F {
        Filing13F {
            accession_number: format!("{}_{}", name, date),
            manager: Manager {
                cik: "0000000000".into(), name: name.into(), filer_type: "HA".into(),
            },
            report_date: date,
            filing_date: date,
            total_aum: 100_000.0,
            holdings: vec![],
            other_included_count: 0,
        }
    }
}
