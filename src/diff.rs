//! Quarter-over-quarter position change detection.
//!
//! Compares two filings from consecutive quarters to classify each position
//! as new, increased, unchanged, decreased, or exited. Computes portfolio
//! impact metrics for each change.

use crate::types::{ConvictionLevel, Filing13F, Holding, PositionChange, Sector};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// PositionDiff
// ---------------------------------------------------------------------------

/// Describes how a single position changed between two quarters.
#[derive(Debug, Clone)]
pub struct PositionDiff {
    /// Ticker symbol.
    pub ticker: String,
    /// GICS sector.
    pub sector: Sector,
    /// Classification of the change.
    pub change: PositionChange,
    /// Share count in the prior quarter (0 for new positions).
    pub prior_shares: i64,
    /// Share count in the current quarter (0 for exits).
    pub current_shares: i64,
    /// Absolute change in share count (positive for buys, negative for sells).
    pub delta_shares: i64,
    /// Value of the position in the prior quarter.
    pub prior_value: f64,
    /// Value of the position in the current quarter.
    pub current_value: f64,
    /// Dollar value of the change.
    pub delta_value: f64,
    /// Percent change in shares.
    pub pct_change_shares: f64,
    /// Percent of the current portfolio's AUM.
    pub portfolio_weight: f64,
    /// Conviction level in the current filing.
    pub conviction: ConvictionLevel,
    /// Prior conviction level (None for new positions).
    pub prior_conviction: Option<ConvictionLevel>,
}

impl PositionDiff {
    /// Whether this represents a meaningful conviction change (new position or conviction upgrade).
    pub fn is_conviction_upgrade(&self) -> bool {
        self.conviction > self.prior_conviction.unwrap_or(ConvictionLevel::Low)
    }

    /// Whether this represents a conviction downgrade.
    pub fn is_conviction_downgrade(&self) -> bool {
        if self.prior_conviction.is_none() {
            return false;
        }
        self.conviction < self.prior_conviction.unwrap()
    }

    /// True if this is a new position.
    pub fn is_new(&self) -> bool {
        self.change == PositionChange::New
    }

    /// True if this is a complete exit.
    pub fn is_exit(&self) -> bool {
        self.change == PositionChange::Exited
    }

    /// Dollar impact as fraction of current portfolio AUM.
    pub fn portfolio_impact(&self) -> f64 {
        self.delta_value.abs() / self.current_value.max(1.0)
    }
}

// ---------------------------------------------------------------------------
// DiffResult
// ---------------------------------------------------------------------------

/// Complete result of comparing two quarters of a single manager's filings.
#[derive(Debug, Clone)]
pub struct DiffResult {
    /// Manager CIK.
    pub manager_cik: String,
    /// Manager name.
    pub manager_name: String,
    /// All position diffs.
    pub diffs: Vec<PositionDiff>,
    /// Summarized counts by change type.
    pub summary: DiffSummary,
}

impl DiffResult {
    /// Only the diffs for new positions.
    pub fn new_positions(&self) -> Vec<&PositionDiff> {
        self.diffs.iter().filter(|d| d.is_new()).collect()
    }

    /// Only the diffs for exited positions.
    pub fn exited_positions(&self) -> Vec<&PositionDiff> {
        self.diffs.iter().filter(|d| d.is_exit()).collect()
    }

    /// Only the diffs for increased positions.
    pub fn increased_positions(&self) -> Vec<&PositionDiff> {
        self.diffs.iter().filter(|d| d.change == PositionChange::Increased).collect()
    }

    /// Only the diffs for decreased positions.
    pub fn decreased_positions(&self) -> Vec<&PositionDiff> {
        self.diffs.iter().filter(|d| d.change == PositionChange::Decreased).collect()
    }

    /// Top-N largest position increases by value.
    pub fn top_increases(&self, n: usize) -> Vec<&PositionDiff> {
        let mut inc = self.increased_positions();
        inc.sort_by(|a, b| b.delta_value.partial_cmp(&a.delta_value).unwrap_or(std::cmp::Ordering::Equal));
        inc.truncate(n);
        inc
    }

    /// Top-N largest position decreases by value.
    pub fn top_decreases(&self, n: usize) -> Vec<&PositionDiff> {
        let mut dec = self.decreased_positions();
        dec.sort_by(|a, b| a.delta_value.partial_cmp(&b.delta_value).unwrap_or(std::cmp::Ordering::Equal));
        dec.truncate(n);
        dec
    }

    /// Total dollar value of all new positions.
    pub fn total_new_value(&self) -> f64 {
        self.new_positions().iter().map(|d| d.current_value).sum()
    }

    /// Total dollar value of all exits.
    pub fn total_exit_value(&self) -> f64 {
        self.exited_positions().iter().map(|d| d.prior_value).sum()
    }

    /// Net buying/selling: positive means net buyer.
    pub fn net_flow(&self) -> f64 {
        self.diffs.iter().map(|d| d.delta_value).sum()
    }
}

// ---------------------------------------------------------------------------
// DiffSummary
// ---------------------------------------------------------------------------

/// Summary statistics for a quarter-over-quarter comparison.
#[derive(Debug, Clone, Default)]
pub struct DiffSummary {
    pub new_count: usize,
    pub increased_count: usize,
    pub unchanged_count: usize,
    pub decreased_count: usize,
    pub exited_count: usize,
    pub total_positions_current: usize,
    pub total_positions_prior: usize,
    pub turnover_rate: f64,  // fraction of portfolio that turned over
    pub net_delta_value: f64,
    pub largest_new_value: f64,
    pub largest_exit_value: f64,
}

// ---------------------------------------------------------------------------
// Diff Engine
// ---------------------------------------------------------------------------

/// Compare two filings and produce a full diff result.
pub fn diff_filings(prior: &Filing13F, current: &Filing13F) -> DiffResult {
    let prior_map: HashMap<&str, &Holding> = prior.holdings.iter()
        .map(|h| (h.ticker.as_str(), h))
        .collect();
    let current_map: HashMap<&str, &Holding> = current.holdings.iter()
        .map(|h| (h.ticker.as_str(), h))
        .collect();

    let mut diffs = Vec::new();
    let mut summary = DiffSummary::default();

    // Process current holdings (new + changed)
    for h in &current.holdings {
        let ticker = h.ticker.as_str();
        match prior_map.get(ticker) {
            Some(ph) => {
                let change = classify_change(ph.shares, h.shares);
                let delta_shares = h.shares - ph.shares;
                let delta_value = h.value - ph.value;
                let pct = if ph.shares != 0 {
                    (h.shares - ph.shares) as f64 / ph.shares as f64
                } else {
                    if h.shares > 0 { 1.0 } else { 0.0 }
                };

                diffs.push(PositionDiff {
                    ticker: h.ticker.clone(),
                    sector: h.sector,
                    change,
                    prior_shares: ph.shares,
                    current_shares: h.shares,
                    delta_shares,
                    prior_value: ph.value,
                    current_value: h.value,
                    delta_value,
                    pct_change_shares: pct,
                    portfolio_weight: h.portfolio_weight,
                    conviction: h.conviction(),
                    prior_conviction: Some(ph.conviction()),
                });
            }
            None => {
                // New position
                diffs.push(PositionDiff {
                    ticker: h.ticker.clone(),
                    sector: h.sector,
                    change: PositionChange::New,
                    prior_shares: 0,
                    current_shares: h.shares,
                    delta_shares: h.shares,
                    prior_value: 0.0,
                    current_value: h.value,
                    delta_value: h.value,
                    pct_change_shares: 1.0,
                    portfolio_weight: h.portfolio_weight,
                    conviction: h.conviction(),
                    prior_conviction: None,
                });
            }
        }
    }

    // Detect exits (in prior but not in current)
    for h in &prior.holdings {
        let ticker = h.ticker.as_str();
        if !current_map.contains_key(ticker) {
            diffs.push(PositionDiff {
                ticker: h.ticker.clone(),
                sector: h.sector,
                change: PositionChange::Exited,
                prior_shares: h.shares,
                current_shares: 0,
                delta_shares: -h.shares,
                prior_value: h.value,
                current_value: 0.0,
                delta_value: -h.value,
                pct_change_shares: -1.0,
                portfolio_weight: 0.0,
                conviction: ConvictionLevel::Low,
                prior_conviction: Some(h.conviction()),
            });
        }
    }

    // Compute summary
    for d in &diffs {
        match d.change {
            PositionChange::New => {
                summary.new_count += 1;
                summary.largest_new_value = summary.largest_new_value.max(d.current_value);
            }
            PositionChange::Increased => summary.increased_count += 1,
            PositionChange::Unchanged => summary.unchanged_count += 1,
            PositionChange::Decreased => summary.decreased_count += 1,
            PositionChange::Exited => {
                summary.exited_count += 1;
                summary.largest_exit_value = summary.largest_exit_value.max(d.prior_value);
            }
        }
    }

    summary.total_positions_current = current.holdings.len();
    summary.total_positions_prior = prior.holdings.len();
    summary.net_delta_value = diffs.iter().map(|d| d.delta_value).sum();

    // Turnover = (new value + exit value) / 2 / current AUM
    let new_val: f64 = diffs.iter().filter(|d| d.is_new()).map(|d| d.current_value).sum();
    let exit_val: f64 = diffs.iter().filter(|d| d.is_exit()).map(|d| d.prior_value).sum();
    let avg_aum = (prior.total_aum + current.total_aum) / 2.0;
    summary.turnover_rate = if avg_aum > 0.0 {
        (new_val + exit_val) / 2.0 / avg_aum
    } else {
        0.0
    };

    DiffResult {
        manager_cik: current.manager.cik.clone(),
        manager_name: current.manager.name.clone(),
        diffs,
        summary,
    }
}

/// Classify the change between prior and current share counts.
fn classify_change(prior: i64, current: i64) -> PositionChange {
    if prior == current {
        PositionChange::Unchanged
    } else if current > prior {
        PositionChange::Increased
    } else {
        PositionChange::Decreased
    }
}

/// Convenience: compute diff for all managers in a paired list.
/// Takes slices of (prior, current) filing pairs.
pub fn batch_diff(pairs: &[(&Filing13F, &Filing13F)]) -> Vec<DiffResult> {
    pairs.iter().map(|(p, c)| diff_filings(p, c)).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Filing13F, Holding, Manager};
    use chrono::NaiveDate;

    fn make_holding(ticker: &str, shares: i64, value: f64, weight: f64) -> Holding {
        Holding {
            cusip: format!("{}_CUSIP", ticker),
            ticker: ticker.into(),
            name: format!("{} Corp", ticker),
            sector: Sector::classify_ticker(ticker),
            shares,
            value,
            share_type: "SH".into(),
            discretion: "SO".into(),
            is_option: false,
            portfolio_weight: weight,
        }
    }

    fn make_filing_custom(holdings: Vec<Holding>, total_aum: f64) -> Filing13F {
        let total: f64 = holdings.iter().map(|h| h.value).sum();
        Filing13F {
            accession_number: "test".into(),
            manager: Manager {
                cik: "0001111111".into(),
                name: "Alpha Fund".into(),
                filer_type: "HA".into(),
            },
            report_date: NaiveDate::from_ymd_opt(2024, 6, 30).unwrap(),
            filing_date: NaiveDate::from_ymd_opt(2024, 8, 14).unwrap(),
            total_aum: if total_aum > 0.0 { total_aum } else { total },
            holdings,
            other_included_count: 0,
        }
    }

    #[test]
    fn test_classify_change() {
        assert_eq!(classify_change(100, 100), PositionChange::Unchanged);
        assert_eq!(classify_change(100, 150), PositionChange::Increased);
        assert_eq!(classify_change(100, 50), PositionChange::Decreased);
    }

    #[test]
    fn test_diff_new_and_exit() {
        let prior = make_filing_custom(vec![
            make_holding("AAPL", 1000, 500_000.0, 0.5),
            make_holding("JNJ", 500, 500_000.0, 0.5),
        ], 1_000_000.0);

        let current = make_filing_custom(vec![
            make_holding("AAPL", 1200, 600_000.0, 0.6),
            make_holding("MSFT", 300, 400_000.0, 0.4),
        ], 1_000_000.0);

        let diff = diff_filings(&prior, &current);
        assert_eq!(diff.summary.new_count, 1);
        assert_eq!(diff.summary.exited_count, 1);
        assert_eq!(diff.summary.increased_count, 1);
        assert_eq!(diff.summary.total_positions_current, 2);
        assert_eq!(diff.summary.total_positions_prior, 2);
    }

    #[test]
    fn test_diff_unchanged() {
        let h = make_holding("AAPL", 1000, 500_000.0, 1.0);
        let prior = make_filing_custom(vec![h.clone()], 500_000.0);
        let current = make_filing_custom(vec![h], 500_000.0);

        let diff = diff_filings(&prior, &current);
        assert_eq!(diff.summary.unchanged_count, 1);
        assert_eq!(diff.summary.new_count, 0);
    }

    #[test]
    fn test_diff_new_position_details() {
        let prior = make_filing_custom(vec![
            make_holding("AAPL", 1000, 1_000_000.0, 1.0),
        ], 1_000_000.0);

        let current = make_filing_custom(vec![
            make_holding("AAPL", 1000, 1_000_000.0, 0.7),
            make_holding("NVDA", 500, 400_000.0, 0.3),
        ], 1_400_000.0);

        let diff = diff_filings(&prior, &current);
        let new_pos = diff.new_positions();
        assert_eq!(new_pos.len(), 1);
        assert_eq!(new_pos[0].ticker, "NVDA");
        assert_eq!(new_pos[0].prior_shares, 0);
        assert_eq!(new_pos[0].current_shares, 500);
        assert!(new_pos[0].is_new());
    }

    #[test]
    fn test_diff_exit_details() {
        let prior = make_filing_custom(vec![
            make_holding("AAPL", 1000, 1_000_000.0, 0.7),
            make_holding("XOM", 500, 400_000.0, 0.3),
        ], 1_400_000.0);

        let current = make_filing_custom(vec![
            make_holding("AAPL", 1000, 1_000_000.0, 1.0),
        ], 1_000_000.0);

        let diff = diff_filings(&prior, &current);
        let exits = diff.exited_positions();
        assert_eq!(exits.len(), 1);
        assert_eq!(exits[0].ticker, "XOM");
        assert_eq!(exits[0].current_shares, 0);
        assert!(exits[0].is_exit());
    }

    #[test]
    fn test_diff_conviction_upgrade() {
        let prior = make_filing_custom(vec![
            make_holding("AAPL", 100, 50_000.0, 0.005),  // Low conviction (<1%)
        ], 1_000_000.0);

        let current = make_filing_custom(vec![
            make_holding("AAPL", 2000, 400_000.0, 0.4),  // VeryHigh conviction (>5%)
        ], 1_000_000.0);

        let diff = diff_filings(&prior, &current);
        let aapl_diff = &diff.diffs[0];
        assert!(aapl_diff.is_conviction_upgrade());
    }

    #[test]
    fn test_diff_summary_net_flow() {
        let prior = make_filing_custom(vec![
            make_holding("AAPL", 1000, 500_000.0, 0.5),
            make_holding("JNJ", 500, 500_000.0, 0.5),
        ], 1_000_000.0);

        let current = make_filing_custom(vec![
            make_holding("AAPL", 1500, 750_000.0, 0.75),
            make_holding("JNJ", 250, 250_000.0, 0.25),
        ], 1_000_000.0);

        let diff = diff_filings(&prior, &current);
        let net = diff.net_flow();
        // AAPL +250k, JNJ -250k → net = 0
        assert!(net.abs() < 1.0);
    }

    #[test]
    fn test_top_increases() {
        let prior = make_filing_custom(vec![
            make_holding("AAPL", 100, 100_000.0, 0.1),
            make_holding("MSFT", 100, 200_000.0, 0.2),
            make_holding("JNJ", 100, 300_000.0, 0.3),
        ], 1_000_000.0);

        let current = make_filing_custom(vec![
            make_holding("AAPL", 200, 200_000.0, 0.2),
            make_holding("MSFT", 200, 400_000.0, 0.3),
            make_holding("JNJ", 80, 240_000.0, 0.24),
        ], 1_000_000.0);

        let diff = diff_filings(&prior, &current);
        let top = diff.top_increases(1);
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].ticker, "MSFT"); // +200k > AAPL's +100k
    }

    #[test]
    fn test_total_new_value() {
        let prior = make_filing_custom(vec![
            make_holding("AAPL", 1000, 1_000_000.0, 1.0),
        ], 1_000_000.0);

        let current = make_filing_custom(vec![
            make_holding("AAPL", 800, 800_000.0, 0.5),
            make_holding("NVDA", 200, 800_000.0, 0.5),
        ], 1_600_000.0);

        let diff = diff_filings(&prior, &current);
        assert!((diff.total_new_value() - 800_000.0).abs() < 1.0);
    }

    #[test]
    fn test_batch_diff() {
        let prior = make_filing_custom(vec![
            make_holding("AAPL", 1000, 500_000.0, 1.0),
        ], 500_000.0);

        let current = make_filing_custom(vec![
            make_holding("AAPL", 1200, 600_000.0, 1.0),
        ], 600_000.0);

        let results = batch_diff(&[(&prior, &current)]);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].summary.increased_count, 1);
    }
}
