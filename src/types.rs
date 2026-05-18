//! Core domain types for 13F filing analysis.
//!
//! Defines the fundamental data structures used across the crate:
//! filings, holdings, managers, sectors, position changes, conviction
//! levels, consensus signals, and rotation metrics.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Sector
// ---------------------------------------------------------------------------

/// GICS-style sector classification for holdings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Sector {
    Technology,
    Healthcare,
    Finance,
    ConsumerDiscretionary,
    ConsumerStaples,
    Energy,
    Industrials,
    Materials,
    RealEstate,
    Utilities,
    CommunicationServices,
    Unknown,
}

impl Sector {
    /// Returns a static lookup table mapping ticker prefixes to sectors.
    pub fn classify_ticker(ticker: &str) -> Self {
        let t = ticker.to_uppercase();
        match t.as_str() {
            // Technology
            "AAPL" | "MSFT" | "GOOGL" | "GOOG" | "META" | "NVDA" | "AMD" | "AVGO"
            | "ORCL" | "CRM" | "ADBE" | "INTC" | "CSCO" | "TXN" | "QCOM" | "PYPL"
            | "SHOP" | "SQ" | "SNOW" | "PLTR" | "NOW" | "INTU" | "UBER"
            | "DDOG" | "NET" | "ZS" | "CRWD" => Sector::Technology,

            // Healthcare
            "JNJ" | "UNH" | "PFE" | "ABBV" | "LLY" | "MRK" | "TMO" | "ABT"
            | "DHR" | "BMY" | "AMGN" | "GILD" | "VRTX" | "BIIB" | "REGN"
            | "ISRG" | "MDT" | "SYK" | "BSX" | "EW" => Sector::Healthcare,

            // Finance
            "BRK.B" | "JPM" | "BAC" | "GS" | "MS" | "V" | "MA" | "BLK"
            | "AXP" | "SCHW" | "C" | "WFC" | "USB" | "PGR" | "MET" | "PRU"
            | "AIG" | "SPGI" | "CB" | "MMC" => Sector::Finance,

            // Consumer Discretionary
            "AMZN" | "TSLA" | "HD" | "NKE" | "MCD" | "SBUX" | "CMG" | "LULU"
            | "TJX" | "LOW" | "BKNG" | "ETSY" | "RCL" | "MAR" | "GM" | "F"
            | "YUM" | "HLT" | "DPZ" => Sector::ConsumerDiscretionary,

            // Consumer Staples
            "WMT" | "PG" | "KO" | "PEP" | "COST" | "PM" | "MO" | "CL"
            | "EL" | "STZ" | "GIS" | "HSY" | "K" | "CLX" | "CPB" | "SYY"
            | "CHD" | "CAG" => Sector::ConsumerStaples,

            // Energy
            "XOM" | "CVX" | "COP" | "SLB" | "EOG" | "OXY" | "MPC" | "PXD"
            | "WFG" | "ET" | "EPD" | "EQT" | "DVN" | "FANG" | "CTRA" | "HES"
            => Sector::Energy,

            // Industrials
            "GE" | "CAT" | "MMM" | "HON" | "UPS" | "RTX" | "BA" | "LMT"
            | "DE" | "UNP" | "NSC" | "CSX" | "EMR" | "ITW" | "CMI" | "ETN"
            | "FDX" | "CHRW" | "ROP" => Sector::Industrials,

            // Materials
            "LIN" | "APD" | "ECL" | "SHW" | "DD" | "FCX" | "NEM" | "DOW"
            | "CTVA" | "PPG" | "EMN" | "ALB" | "FMC" | "VST" | "MOS" => Sector::Materials,

            // Real Estate
            "AMT" | "PLD" | "CCI" | "PSA" | "EQIX" | "SPG" | "O" | "VICI"
            | "WELL" | "DLR" | "AVB" | "EQR" | "INVH" | "SBAC" | "EXR" => Sector::RealEstate,

            // Utilities
            "NEE" | "DUK" | "SO" | "D" | "AEP" | "SRE" | "XEL" | "PEG"
            | "WEC" | "ES" | "ETR" | "EIX" | "PPL" | "AEE" | "CMS" | "DTE"
            => Sector::Utilities,

            // Communication Services
            "DIS" | "NFLX" | "CMCSA" | "T" | "VZ" | "TMUS" | "EA" | "ATVI"
            | "MTCH" | "SNAP" | "PINS" | "RBLX" | "Z" => Sector::CommunicationServices,

            _ => Sector::Unknown,
        }
    }

    /// Human-readable sector label.
    pub fn label(&self) -> &'static str {
        match self {
            Sector::Technology => "Technology",
            Sector::Healthcare => "Healthcare",
            Sector::Finance => "Finance",
            Sector::ConsumerDiscretionary => "Consumer Discretionary",
            Sector::ConsumerStaples => "Consumer Staples",
            Sector::Energy => "Energy",
            Sector::Industrials => "Industrials",
            Sector::Materials => "Materials",
            Sector::RealEstate => "Real Estate",
            Sector::Utilities => "Utilities",
            Sector::CommunicationServices => "Communication Services",
            Sector::Unknown => "Unknown",
        }
    }

    /// Returns all sector variants.
    pub fn all() -> &'static [Sector] {
        &[
            Sector::Technology,
            Sector::Healthcare,
            Sector::Finance,
            Sector::ConsumerDiscretionary,
            Sector::ConsumerStaples,
            Sector::Energy,
            Sector::Industrials,
            Sector::Materials,
            Sector::RealEstate,
            Sector::Utilities,
            Sector::CommunicationServices,
            Sector::Unknown,
        ]
    }
}

impl std::fmt::Display for Sector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

// ---------------------------------------------------------------------------
// PositionChange
// ---------------------------------------------------------------------------

/// Classification of how a position changed between two quarters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PositionChange {
    /// Brand-new position not present in the prior quarter.
    New,
    /// Existing position with increased share count.
    Increased,
    /// Existing position with unchanged share count.
    Unchanged,
    /// Existing position with decreased share count.
    Decreased,
    /// Position fully liquidated — present last quarter, absent this quarter.
    Exited,
}

impl std::fmt::Display for PositionChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PositionChange::New => write!(f, "New"),
            PositionChange::Increased => write!(f, "Increased"),
            PositionChange::Unchanged => write!(f, "Unchanged"),
            PositionChange::Decreased => write!(f, "Decreased"),
            PositionChange::Exited => write!(f, "Exited"),
        }
    }
}

// ---------------------------------------------------------------------------
// ConvictionLevel
// ---------------------------------------------------------------------------

/// How strongly a manager is positioned in a given holding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ConvictionLevel {
    /// Passively held, small allocation (< 1% of portfolio).
    Low,
    /// Moderate allocation (1–3% of portfolio).
    Medium,
    /// Significant position (3–5% of portfolio).
    High,
    /// Top conviction idea (> 5% of portfolio).
    VeryHigh,
}

impl ConvictionLevel {
    /// Classify conviction from the fraction of portfolio a single holding represents.
    pub fn from_portfolio_weight(weight: f64) -> Self {
        if weight >= 0.05 {
            ConvictionLevel::VeryHigh
        } else if weight >= 0.03 {
            ConvictionLevel::High
        } else if weight >= 0.01 {
            ConvictionLevel::Medium
        } else {
            ConvictionLevel::Low
        }
    }

    /// Numeric score for the conviction level (0..3).
    pub fn score(&self) -> u8 {
        match self {
            ConvictionLevel::Low => 0,
            ConvictionLevel::Medium => 1,
            ConvictionLevel::High => 2,
            ConvictionLevel::VeryHigh => 3,
        }
    }
}

impl std::fmt::Display for ConvictionLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConvictionLevel::Low => write!(f, "Low"),
            ConvictionLevel::Medium => write!(f, "Medium"),
            ConvictionLevel::High => write!(f, "High"),
            ConvictionLevel::VeryHigh => write!(f, "VeryHigh"),
        }
    }
}

// ---------------------------------------------------------------------------
// ConsensusSignal
// ---------------------------------------------------------------------------

/// A cross-fund consensus indicator for a given ticker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsensusSignal {
    pub ticker: String,
    pub sector: Sector,
    /// Number of funds holding this ticker.
    pub holder_count: usize,
    /// Average conviction level across all holders.
    pub avg_conviction: f64,
    /// Aggregate direction: >0 bullish, <0 bearish, 0 neutral.
    pub net_direction: f64,
    /// Most common position change across holders.
    pub dominant_change: PositionChange,
    /// Total estimated dollar value across all holders (in millions).
    pub aggregate_value_m: f64,
}

// ---------------------------------------------------------------------------
// RotationMetric
// ---------------------------------------------------------------------------

/// Tracks how much capital flowed into or out of a sector in a single quarter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotationMetric {
    pub sector: Sector,
    pub prior_weight: f64,
    pub current_weight: f64,
    /// Difference in allocation weight (positive = inflow).
    pub weight_delta: f64,
    /// Percent change in sector allocation.
    pub pct_change: f64,
    /// Number of managers with increased allocation.
    pub inflows: usize,
    /// Number of managers with decreased allocation.
    pub outflows: usize,
    /// Net momentum score: weighted average of size and direction.
    pub momentum: f64,
}

// ---------------------------------------------------------------------------
// Holding
// ---------------------------------------------------------------------------

/// A single holding within a 13F filing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Holding {
    /// CUSIP identifier from the filing.
    pub cusip: String,
    /// Normalized ticker symbol.
    pub ticker: String,
    /// Human-readable issuer name.
    pub name: String,
    /// GICS sector classification.
    pub sector: Sector,
    /// Number of shares reported.
    pub shares: i64,
    /// Market value in dollars (from filing).
    pub value: f64,
    /// Share type ("SH", "PR", "AD").
    pub share_type: String,
    /// Investment discretion ("SO", "DS", "DO").
    pub discretion: String,
    /// Option indicator (true if puts/calls).
    pub is_option: bool,
    /// Percentage of the total filing AUM.
    pub portfolio_weight: f64,
}

impl Holding {
    /// Conviction classification based on portfolio weight.
    pub fn conviction(&self) -> ConvictionLevel {
        ConvictionLevel::from_portfolio_weight(self.portfolio_weight)
    }
}

// ---------------------------------------------------------------------------
// Manager
// ---------------------------------------------------------------------------

/// Represents a single hedge fund / institutional manager.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manager {
    pub cik: String,
    pub name: String,
    pub filer_type: String,
}

// ---------------------------------------------------------------------------
// Filing13F
// ---------------------------------------------------------------------------

/// A parsed and normalized SEC Form 13F filing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Filing13F {
    pub accession_number: String,
    pub manager: Manager,
    /// Quarter-end date of the filing.
    pub report_date: NaiveDate,
    /// Date the filing was accepted by SEC.
    pub filing_date: NaiveDate,
    /// Total reported AUM at quarter end (dollars).
    pub total_aum: f64,
    /// List of individual holdings.
    pub holdings: Vec<Holding>,
    /// Number of other managers included (multi-manager filings).
    pub other_included_count: usize,
}

impl Filing13F {
    /// Compute sector-level allocation weights.
    pub fn sector_allocations(&self) -> HashMap<Sector, f64> {
        let total: f64 = self.holdings.iter().map(|h| h.value).sum();
        let mut map = HashMap::new();
        for h in &self.holdings {
            *map.entry(h.sector).or_insert(0.0) += h.value / total;
        }
        map
    }

    /// Return the top-N holdings by value.
    pub fn top_holdings(&self, n: usize) -> Vec<&Holding> {
        let mut sorted: Vec<&Holding> = self.holdings.iter().collect();
        sorted.sort_by(|a, b| b.value.partial_cmp(&a.value).unwrap_or(std::cmp::Ordering::Equal));
        sorted.truncate(n);
        sorted
    }

    /// New positions: holdings with no prior quarter reference (requires caller to filter).
    /// This is a helper for the diff module.
    pub fn total_holdings_value(&self) -> f64 {
        self.holdings.iter().map(|h| h.value).sum()
    }
}

// ---------------------------------------------------------------------------
// CUSIP → Ticker Mapping
// ---------------------------------------------------------------------------

/// Simple in-memory CUSIP-to-ticker lookup table.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CusipMap {
    entries: HashMap<String, (String, String)>, // cusip -> (ticker, name)
}

impl CusipMap {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Insert a CUSIP mapping.
    pub fn insert(&mut self, cusip: &str, ticker: &str, name: &str) {
        self.entries.insert(cusip.to_string(), (ticker.to_string(), name.to_string()));
    }

    /// Lookup a CUSIP, returning (ticker, name) or None.
    pub fn get(&self, cusip: &str) -> Option<(&str, &str)> {
        self.entries.get(cusip).map(|(t, n)| (t.as_str(), n.as_str()))
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sector_classify_known_tickers() {
        assert_eq!(Sector::classify_ticker("AAPL"), Sector::Technology);
        assert_eq!(Sector::classify_ticker("JNJ"), Sector::Healthcare);
        assert_eq!(Sector::classify_ticker("JPM"), Sector::Finance);
        assert_eq!(Sector::classify_ticker("AMZN"), Sector::ConsumerDiscretionary);
        assert_eq!(Sector::classify_ticker("WMT"), Sector::ConsumerStaples);
        assert_eq!(Sector::classify_ticker("XOM"), Sector::Energy);
        assert_eq!(Sector::classify_ticker("CAT"), Sector::Industrials);
        assert_eq!(Sector::classify_ticker("LIN"), Sector::Materials);
        assert_eq!(Sector::classify_ticker("AMT"), Sector::RealEstate);
        assert_eq!(Sector::classify_ticker("NEE"), Sector::Utilities);
        assert_eq!(Sector::classify_ticker("NFLX"), Sector::CommunicationServices);
        assert_eq!(Sector::classify_ticker("ZZZZ"), Sector::Unknown);
    }

    #[test]
    fn sector_case_insensitive() {
        assert_eq!(Sector::classify_ticker("aapl"), Sector::Technology);
        assert_eq!(Sector::classify_ticker("MsFt"), Sector::Technology);
    }

    #[test]
    fn sector_display() {
        assert_eq!(Sector::Technology.to_string(), "Technology");
        assert_eq!(Sector::Unknown.to_string(), "Unknown");
    }

    #[test]
    fn sector_all_variants() {
        let all = Sector::all();
        assert!(all.len() >= 11);
    }

    #[test]
    fn conviction_level_from_weight() {
        assert_eq!(ConvictionLevel::from_portfolio_weight(0.005), ConvictionLevel::Low);
        assert_eq!(ConvictionLevel::from_portfolio_weight(0.02), ConvictionLevel::Medium);
        assert_eq!(ConvictionLevel::from_portfolio_weight(0.04), ConvictionLevel::High);
        assert_eq!(ConvictionLevel::from_portfolio_weight(0.08), ConvictionLevel::VeryHigh);
    }

    #[test]
    fn conviction_level_ord() {
        assert!(ConvictionLevel::VeryHigh > ConvictionLevel::High);
        assert!(ConvictionLevel::High > ConvictionLevel::Medium);
        assert!(ConvictionLevel::Medium > ConvictionLevel::Low);
    }

    #[test]
    fn conviction_level_score() {
        assert_eq!(ConvictionLevel::Low.score(), 0);
        assert_eq!(ConvictionLevel::Medium.score(), 1);
        assert_eq!(ConvictionLevel::High.score(), 2);
        assert_eq!(ConvictionLevel::VeryHigh.score(), 3);
    }

    #[test]
    fn holding_conviction() {
        let h = Holding {
            cusip: "037833100".into(),
            ticker: "AAPL".into(),
            name: "Apple Inc".into(),
            sector: Sector::Technology,
            shares: 1000,
            value: 500_000.0,
            share_type: "SH".into(),
            discretion: "SO".into(),
            is_option: false,
            portfolio_weight: 0.04,
        };
        assert_eq!(h.conviction(), ConvictionLevel::High);
    }

    #[test]
    fn position_change_display() {
        assert_eq!(PositionChange::New.to_string(), "New");
        assert_eq!(PositionChange::Exited.to_string(), "Exited");
    }

    #[test]
    fn cusip_map_insert_lookup() {
        let mut map = CusipMap::new();
        map.insert("037833100", "AAPL", "Apple Inc");
        let (ticker, name) = map.get("037833100").unwrap();
        assert_eq!(ticker, "AAPL");
        assert_eq!(name, "Apple Inc");
        assert!(map.get("000000000").is_none());
    }

    #[test]
    fn cusip_map_len() {
        let mut map = CusipMap::new();
        assert!(map.is_empty());
        map.insert("037833100", "AAPL", "Apple Inc");
        map.insert("023135106", "AMZN", "Amazon.com Inc");
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn filing_sector_allocations() {
        let filing = make_test_filing();
        let alloc = filing.sector_allocations();
        // AAPL=500k, JNJ=500k, total=1M → each 50%
        assert!((alloc.get(&Sector::Technology).unwrap() - 0.5).abs() < 1e-9);
        assert!((alloc.get(&Sector::Healthcare).unwrap() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn filing_top_holdings() {
        let filing = make_test_filing();
        let top = filing.top_holdings(1);
        assert_eq!(top.len(), 1);
        // Both equal, but top_holdings should return one
        assert_eq!(top[0].ticker, "AAPL");
    }

    /// Helper to create a minimal test filing with two holdings.
    pub fn make_test_filing() -> Filing13F {
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
}
