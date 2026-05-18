//! Cross-fund consensus engine.
//!
//! Aggregates signals across multiple hedge funds to identify:
//! - High-conviction tickers (held by many funds)
//! - Multi-manager agreement on direction
//! - Sector-level consensus
//! - Conviction clustering
//! - Whale tracking (large position changes)

use crate::diff::{DiffResult, PositionDiff};
use crate::types::{
    ConsensusSignal, ConvictionLevel, Filing13F, PositionChange, Sector,
};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// ConsensusTickerAgg — per-ticker aggregation across managers
// ---------------------------------------------------------------------------

/// Aggregated view of a single ticker across all managers in a quarter.
#[derive(Debug, Clone)]
pub struct TickerConsensus {
    pub ticker: String,
    pub sector: Sector,
    /// Number of funds holding this ticker.
    pub holder_count: usize,
    /// Sum of conviction scores (0-3 each).
    pub conviction_score_sum: f64,
    /// Average conviction score.
    pub avg_conviction: f64,
    /// Number of funds that increased.
    pub increased: usize,
    /// Number of funds that decreased.
    pub decreased: usize,
    /// Number of funds with new position.
    pub new_count: usize,
    /// Number of funds that exited.
    pub exited: usize,
    /// Number unchanged.
    pub unchanged: usize,
    /// Aggregate dollar value across all holders (in millions).
    pub total_value_m: f64,
    /// Net direction score: +1 per increase/new, -1 per decrease/exit.
    pub direction_score: f64,
    /// The individual diffs that contributed.
    pub diffs: Vec<PositionDiff>,
}

impl TickerConsensus {
    /// Net direction ratio: -1..+1, positive = bullish.
    pub fn direction_ratio(&self) -> f64 {
        let total = (self.increased + self.decreased + self.new_count + self.exited) as f64;
        if total == 0.0 {
            return 0.0;
        }
        self.direction_score / total
    }

    /// Most common position change type.
    pub fn dominant_change(&self) -> PositionChange {
        let counts = [
            (PositionChange::New, self.new_count),
            (PositionChange::Increased, self.increased),
            (PositionChange::Unchanged, self.unchanged),
            (PositionChange::Decreased, self.decreased),
            (PositionChange::Exited, self.exited),
        ];
        counts.into_iter()
            .max_by_key(|(_, c)| *c)
            .map(|(ch, _)| ch)
            .unwrap_or(PositionChange::Unchanged)
    }

    /// Is this a high-conviction ticker (held by >= N funds)?
    pub fn is_high_conviction(&self, min_holders: usize) -> bool {
        self.holder_count >= min_holders
    }
}

// ---------------------------------------------------------------------------
// SectorConsensus — per-sector aggregation
// ---------------------------------------------------------------------------

/// Aggregated sector view across all managers.
#[derive(Debug, Clone)]
pub struct SectorConsensus {
    pub sector: Sector,
    /// Number of funds with at least one holding in this sector.
    pub holder_count: usize,
    /// Average sector weight across all funds (fraction of portfolio).
    pub avg_weight: f64,
    /// Number of funds increasing sector allocation.
    pub inflows: usize,
    /// Number of funds decreasing sector allocation.
    pub outflows: usize,
    /// Net inflow/outflow indicator.
    pub net_direction: f64,
    /// Total dollar value in sector across all funds (millions).
    pub total_value_m: f64,
}

// ---------------------------------------------------------------------------
// Whale — large position change tracker
// ---------------------------------------------------------------------------

/// Tracks a single manager's large position change.
#[derive(Debug, Clone)]
pub struct WhaleMove {
    pub manager_name: String,
    pub manager_cik: String,
    pub ticker: String,
    pub sector: Sector,
    pub change: PositionChange,
    pub delta_value_m: f64,
    /// New portfolio weight.
    pub weight: f64,
    pub conviction: ConvictionLevel,
}

/// A collection of whale moves for a quarter.
#[derive(Debug, Clone)]
pub struct WhaleTracker {
    pub moves: Vec<WhaleMove>,
    /// Minimum dollar threshold (millions) to qualify as a whale move.
    pub threshold_m: f64,
}

impl WhaleTracker {
    pub fn new(threshold_m: f64) -> Self {
        Self {
            moves: Vec::new(),
            threshold_m,
        }
    }

    /// Detect whale moves from a diff result.
    pub fn process_diff(&mut self, diff: &DiffResult) {
        for d in &diff.diffs {
            if d.change == PositionChange::Unchanged {
                continue;
            }
            let delta_m = d.delta_value.abs() / 1_000_000.0;
            if delta_m >= self.threshold_m {
                self.moves.push(WhaleMove {
                    manager_name: diff.manager_name.clone(),
                    manager_cik: diff.manager_cik.clone(),
                    ticker: d.ticker.clone(),
                    sector: d.sector,
                    change: d.change,
                    delta_value_m: delta_m,
                    weight: d.portfolio_weight,
                    conviction: d.conviction,
                });
            }
        }
    }

    /// Whale moves filtered by direction (new/increased).
    pub fn bullish_moves(&self) -> Vec<&WhaleMove> {
        self.moves.iter()
            .filter(|m| m.change == PositionChange::New || m.change == PositionChange::Increased)
            .collect()
    }

    /// Whale moves filtered by direction (decreased/exited).
    pub fn bearish_moves(&self) -> Vec<&WhaleMove> {
        self.moves.iter()
            .filter(|m| m.change == PositionChange::Decreased || m.change == PositionChange::Exited)
            .collect()
    }

    /// Whale moves for a specific ticker.
    pub fn for_ticker(&self, ticker: &str) -> Vec<&WhaleMove> {
        self.moves.iter()
            .filter(|m| m.ticker == ticker)
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Consensus Engine
// ---------------------------------------------------------------------------

/// Engine for computing cross-fund consensus from quarterly filings and diffs.
pub struct ConsensusEngine {
    /// Minimum number of holders to consider "high conviction".
    pub min_holders: usize,
    /// Minimum average conviction score to flag.
    pub min_avg_conviction: f64,
}

impl ConsensusEngine {
    pub fn new() -> Self {
        Self {
            min_holders: 3,
            min_avg_conviction: 1.5,
        }
    }

    pub fn with_params(min_holders: usize, min_avg_conviction: f64) -> Self {
        Self { min_holders, min_avg_conviction }
    }

    /// Compute per-ticker consensus across all diffs for a quarter.
    pub fn ticker_consensus(&self, diffs: &[DiffResult]) -> HashMap<String, TickerConsensus> {
        let mut agg: HashMap<String, TickerConsensus> = HashMap::new();

        for diff in diffs {
            for d in &diff.diffs {
                let entry = agg.entry(d.ticker.clone()).or_insert_with(|| {
                    TickerConsensus {
                        ticker: d.ticker.clone(),
                        sector: d.sector,
                        holder_count: 0,
                        conviction_score_sum: 0.0,
                        avg_conviction: 0.0,
                        increased: 0,
                        decreased: 0,
                        new_count: 0,
                        exited: 0,
                        unchanged: 0,
                        total_value_m: 0.0,
                        direction_score: 0.0,
                        diffs: Vec::new(),
                    }
                });

                entry.diffs.push(d.clone());

                // Count as a holder if they have a non-zero current position
                if d.current_shares > 0 {
                    entry.holder_count += 1;
                    entry.total_value_m += d.current_value / 1_000_000.0;
                    entry.conviction_score_sum += d.conviction.score() as f64;
                } else if d.prior_shares > 0 {
                    // They exited — count prior conviction
                    entry.conviction_score_sum += d.prior_conviction
                        .map(|c| c.score() as f64)
                        .unwrap_or(0.0);
                }

                match d.change {
                    PositionChange::New => {
                        entry.new_count += 1;
                        // holder_count and total_value_m already counted above (current_shares > 0)
                        entry.direction_score += 1.0;
                    }
                    PositionChange::Increased => {
                        entry.increased += 1;
                        entry.direction_score += 1.0;
                    }
                    PositionChange::Unchanged => {
                        entry.unchanged += 1;
                    }
                    PositionChange::Decreased => {
                        entry.decreased += 1;
                        entry.direction_score -= 1.0;
                    }
                    PositionChange::Exited => {
                        entry.exited += 1;
                        entry.direction_score -= 1.0;
                    }
                }
            }
        }

        // Compute averages
        for cons in agg.values_mut() {
            let total_participants = cons.new_count + cons.increased + cons.decreased
                + cons.exited + cons.unchanged;
            if total_participants > 0 {
                cons.avg_conviction = cons.conviction_score_sum / total_participants as f64;
            }
        }

        agg
    }

    /// Identify high-conviction tickers meeting minimum thresholds.
    pub fn high_conviction_tickers(&self, diffs: &[DiffResult]) -> Vec<TickerConsensus> {
        let agg = self.ticker_consensus(diffs);
        let mut result: Vec<TickerConsensus> = agg.into_values()
            .filter(|t| t.is_high_conviction(self.min_holders)
                && t.avg_conviction >= self.min_avg_conviction)
            .collect();
        // Sort by conviction score descending
        result.sort_by(|a, b| b.avg_conviction.partial_cmp(&a.avg_conviction).unwrap_or(std::cmp::Ordering::Equal));
        result
    }

    /// Compute sector-level consensus from current-quarter filings.
    pub fn sector_consensus(&self, filings: &[&Filing13F]) -> HashMap<Sector, SectorConsensus> {
        let mut agg: HashMap<Sector, SectorConsensus> = HashMap::new();

        for filing in filings {
            let alloc = filing.sector_allocations();
            for (&sector, &weight) in &alloc {
                let entry = agg.entry(sector).or_insert_with(|| SectorConsensus {
                    sector,
                    holder_count: 0,
                    avg_weight: 0.0,
                    inflows: 0,
                    outflows: 0,
                    net_direction: 0.0,
                    total_value_m: 0.0,
                });
                entry.holder_count += 1;
                entry.avg_weight += weight;
                entry.total_value_m += (weight * filing.total_aum) / 1_000_000.0;
            }
        }

        // Normalize average weights
        let n = filings.len().max(1) as f64;
        for cons in agg.values_mut() {
            cons.avg_weight /= n;
        }

        agg
    }

    /// Compute sector-level consensus incorporating diffs (inflows/outflows).
    pub fn sector_consensus_with_diffs(
        &self,
        filings: &[&Filing13F],
        diffs: &[DiffResult],
    ) -> HashMap<Sector, SectorConsensus> {
        let mut agg = self.sector_consensus(filings);

        // Count sector inflows/outflows from diffs
        for diff in diffs {
            for d in &diff.diffs {
                match d.change {
                    PositionChange::New | PositionChange::Increased => {
                        if let Some(cons) = agg.get_mut(&d.sector) {
                            cons.inflows += 1;
                            cons.net_direction += 1.0;
                        }
                    }
                    PositionChange::Decreased | PositionChange::Exited => {
                        if let Some(cons) = agg.get_mut(&d.sector) {
                            cons.outflows += 1;
                            cons.net_direction -= 1.0;
                        }
                    }
                    PositionChange::Unchanged => {}
                }
            }
        }

        agg
    }

    /// Convert TickerConsensus to ConsensusSignal for downstream consumption.
    pub fn to_signals(&self, diffs: &[DiffResult]) -> Vec<ConsensusSignal> {
        let agg = self.ticker_consensus(diffs);
        agg.into_values().map(|t| {
            let direction = t.direction_ratio();
            let dominant = t.dominant_change();
            ConsensusSignal {
            ticker: t.ticker,
            sector: t.sector,
            holder_count: t.holder_count,
            avg_conviction: t.avg_conviction,
            net_direction: direction,
            dominant_change: dominant,
            aggregate_value_m: t.total_value_m,
        }}).collect()
    }
}

// ---------------------------------------------------------------------------
// Conviction Clustering
// ---------------------------------------------------------------------------

/// Clusters managers by their conviction patterns.
/// Simple k-medoid style clustering based on sector allocation overlap.
#[derive(Debug, Clone)]
pub struct ConvictionCluster {
    /// Sector weight vector for the cluster centroid.
    pub centroid: HashMap<Sector, f64>,
    /// Manager CIKs in this cluster.
    pub members: Vec<String>,
    /// Average intra-cluster similarity (Jaccard-like).
    pub cohesion: f64,
}

/// Compute the Jaccard similarity of two sector allocation maps.
pub fn sector_jaccard(a: &HashMap<Sector, f64>, b: &HashMap<Sector, f64>) -> f64 {
    let all_sectors: std::collections::HashSet<&Sector> =
        a.keys().chain(b.keys()).collect();

    let mut intersection = 0.0f64;
    let mut union = 0.0f64;

    for &s in &all_sectors {
        let va = a.get(&s).copied().unwrap_or(0.0);
        let vb = b.get(&s).copied().unwrap_or(0.0);
        intersection += va.min(vb);
        union += va.max(vb);
    }

    if union == 0.0 { 0.0 } else { intersection / union }
}

/// Group filings into conviction clusters by sector allocation similarity.
/// Uses a simple greedy clustering approach.
pub fn cluster_by_conviction(
    filings: &[&Filing13F],
    similarity_threshold: f64,
) -> Vec<ConvictionCluster> {
    if filings.is_empty() {
        return vec![];
    }

    let allocations: Vec<HashMap<Sector, f64>> =
        filings.iter().map(|f| f.sector_allocations()).collect();

    let mut clusters: Vec<ConvictionCluster> = Vec::new();
    let mut assigned: Vec<bool> = vec![false; filings.len()];

    for (i, filing) in filings.iter().enumerate() {
        if assigned[i] {
            continue;
        }
        assigned[i] = true;

        let mut members = vec![filing.manager.cik.clone()];
        let mut weight_sum: HashMap<Sector, f64> = allocations[i].clone();

        // Greedy: find all unassigned filings that are similar enough
        for j in (i + 1)..filings.len() {
            if assigned[j] {
                continue;
            }
            let sim = sector_jaccard(&allocations[i], &allocations[j]);
            if sim >= similarity_threshold {
                assigned[j] = true;
                members.push(filings[j].manager.cik.clone());
                for (&s, &w) in &allocations[j] {
                    *weight_sum.entry(s).or_insert(0.0) += w;
                }
            }
        }

        // Compute centroid
        let n = members.len() as f64;
        let centroid: HashMap<Sector, f64> = weight_sum.into_iter()
            .map(|(s, w)| (s, w / n))
            .collect();

        // Compute cohesion
        let mut sim_sum = 0.0;
        let mut pair_count = 0;
        for j in 0..filings.len() {
            if members.contains(&filings[j].manager.cik) {
                for k in (j + 1)..filings.len() {
                    if members.contains(&filings[k].manager.cik) {
                        sim_sum += sector_jaccard(&allocations[j], &allocations[k]);
                        pair_count += 1;
                    }
                }
            }
        }
        let cohesion = if pair_count > 0 { sim_sum / pair_count as f64 } else { 1.0 };

        clusters.push(ConvictionCluster { centroid, members, cohesion });
    }

    clusters
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Holding, Manager};
    use chrono::NaiveDate;

    fn h(ticker: &str, shares: i64, value: f64, weight: f64) -> Holding {
        Holding {
            cusip: format!("{}_C", ticker),
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

    fn filing(cik: &str, name: &str, holdings: Vec<Holding>, aum: f64) -> Filing13F {
        Filing13F {
            accession_number: format!("{}_{}", cik, name),
            manager: Manager { cik: cik.into(), name: name.into(), filer_type: "HA".into() },
            report_date: NaiveDate::from_ymd_opt(2024, 6, 30).unwrap(),
            filing_date: NaiveDate::from_ymd_opt(2024, 8, 14).unwrap(),
            total_aum: aum,
            holdings,
            other_included_count: 0,
        }
    }

    fn diff_result(cik: &str, name: &str, diffs: Vec<PositionDiff>) -> DiffResult {
        DiffResult {
            manager_cik: cik.into(),
            manager_name: name.into(),
            diffs,
            summary: crate::diff::DiffSummary::default(),
        }
    }

    fn pd(ticker: &str, sector: Sector, change: PositionChange, prior_sh: i64, cur_sh: i64, prior_v: f64, cur_v: f64, weight: f64) -> PositionDiff {
        PositionDiff {
            ticker: ticker.into(),
            sector,
            change,
            prior_shares: prior_sh,
            current_shares: cur_sh,
            delta_shares: cur_sh - prior_sh,
            prior_value: prior_v,
            current_value: cur_v,
            delta_value: cur_v - prior_v,
            pct_change_shares: if prior_sh != 0 { (cur_sh - prior_sh) as f64 / prior_sh as f64 } else { 1.0 },
            portfolio_weight: weight,
            conviction: ConvictionLevel::from_portfolio_weight(weight),
            prior_conviction: if prior_sh > 0 { Some(ConvictionLevel::from_portfolio_weight(weight * 0.8)) } else { None },
        }
    }

    #[test]
    fn test_ticker_consensus_basic() {
        let engine = ConsensusEngine::new();
        let d1 = diff_result("C1", "Fund A", vec![
            pd("AAPL", Sector::Technology, PositionChange::Increased, 1000, 1500, 500_000.0, 750_000.0, 0.04),
        ]);
        let d2 = diff_result("C2", "Fund B", vec![
            pd("AAPL", Sector::Technology, PositionChange::Increased, 800, 1000, 400_000.0, 500_000.0, 0.03),
        ]);
        let d3 = diff_result("C3", "Fund C", vec![
            pd("AAPL", Sector::Technology, PositionChange::New, 0, 500, 0.0, 250_000.0, 0.02),
        ]);

        let agg = engine.ticker_consensus(&[d1, d2, d3]);
        let aapl = agg.get("AAPL").unwrap();
        assert_eq!(aapl.holder_count, 3);
        assert_eq!(aapl.increased, 2);
        assert_eq!(aapl.new_count, 1);
        assert!(aapl.direction_score > 0.0);
    }

    #[test]
    fn test_high_conviction_tickers() {
        let engine = ConsensusEngine::with_params(2, 1.0);
        let d1 = diff_result("C1", "Fund A", vec![
            pd("AAPL", Sector::Technology, PositionChange::Increased, 1000, 1500, 500_000.0, 750_000.0, 0.04),
            pd("MSFT", Sector::Technology, PositionChange::New, 0, 800, 0.0, 400_000.0, 0.03),
        ]);
        let d2 = diff_result("C2", "Fund B", vec![
            pd("AAPL", Sector::Technology, PositionChange::Increased, 800, 1000, 400_000.0, 500_000.0, 0.03),
            pd("MSFT", Sector::Technology, PositionChange::Increased, 500, 700, 250_000.0, 350_000.0, 0.02),
        ]);
        let d3 = diff_result("C3", "Fund C", vec![
            pd("AAPL", Sector::Technology, PositionChange::New, 0, 500, 0.0, 250_000.0, 0.02),
            pd("JNJ", Sector::Healthcare, PositionChange::Unchanged, 500, 500, 300_000.0, 300_000.0, 0.01),
        ]);

        let hc = engine.high_conviction_tickers(&[d1, d2, d3]);
        assert!(hc.len() >= 2); // AAPL and MSFT both qualify
        assert!(hc[0].ticker == "AAPL" || hc[1].ticker == "AAPL");
    }

    #[test]
    fn test_sector_consensus() {
        let engine = ConsensusEngine::new();
        let f1 = filing("C1", "Tech Fund", vec![
            h("AAPL", 1000, 500_000.0, 0.6),
            h("MSFT", 500, 300_000.0, 0.3),
            h("JNJ", 100, 100_000.0, 0.1),
        ], 900_000.0);
        let f2 = filing("C2", "Health Fund", vec![
            h("JNJ", 1000, 500_000.0, 0.6),
            h("UNH", 300, 300_000.0, 0.3),
            h("AAPL", 100, 100_000.0, 0.1),
        ], 900_000.0);

        let sc = engine.sector_consensus(&[&f1, &f2]);
        let tech = sc.get(&Sector::Technology).unwrap();
        let health = sc.get(&Sector::Healthcare).unwrap();
        assert_eq!(tech.holder_count, 2);
        assert_eq!(health.holder_count, 2);
        // Tech avg weight: (0.9 + 0.1) / 2 = 0.5
        assert!((tech.avg_weight - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_whale_tracker() {
        let mut tracker = WhaleTracker::new(0.3); // $300k threshold
        let diff = diff_result("C1", "Big Fund", vec![
            pd("AAPL", Sector::Technology, PositionChange::New, 0, 10000, 0.0, 1_500_000.0, 0.08),
            pd("JNJ", Sector::Healthcare, PositionChange::Increased, 500, 800, 300_000.0, 500_000.0, 0.02),
            pd("XOM", Sector::Energy, PositionChange::Decreased, 200, 180, 100_000.0, 90_000.0, 0.005),
        ]);
        tracker.process_diff(&diff);
        // AAPL delta = 1.5M > 0.3M ✓
        // JNJ delta = 200k = 0.2M < 0.3M ✗
        // XOM delta = 10k = 0.01M < 0.3M ✗
        assert_eq!(tracker.moves.len(), 1);
        assert_eq!(tracker.moves[0].ticker, "AAPL");
        assert_eq!(tracker.bullish_moves().len(), 1);
    }

    #[test]
    fn test_whale_tracker_for_ticker() {
        let mut tracker = WhaleTracker::new(0.1);
        let d1 = diff_result("C1", "Fund A", vec![
            pd("AAPL", Sector::Technology, PositionChange::Increased, 1000, 2000, 500_000.0, 1_000_000.0, 0.05),
        ]);
        let d2 = diff_result("C2", "Fund B", vec![
            pd("AAPL", Sector::Technology, PositionChange::New, 0, 500, 0.0, 250_000.0, 0.03),
        ]);
        tracker.process_diff(&d1);
        tracker.process_diff(&d2);
        assert_eq!(tracker.for_ticker("AAPL").len(), 2);
        assert_eq!(tracker.for_ticker("MSFT").len(), 0);
    }

    #[test]
    fn test_sector_jaccard() {
        let mut a = HashMap::new();
        a.insert(Sector::Technology, 0.6);
        a.insert(Sector::Healthcare, 0.4);

        let mut b = HashMap::new();
        b.insert(Sector::Technology, 0.5);
        b.insert(Sector::Finance, 0.5);

        // Intersection: min(Tech) + min(Others) = 0.5 + 0 + 0 = 0.5
        // Union: max(Tech) + max(Others) = 0.6 + 0.4 + 0.5 = 1.5
        let sim = sector_jaccard(&a, &b);
        assert!((sim - (0.5 / 1.5)).abs() < 1e-9);
    }

    #[test]
    fn test_sector_jaccard_identical() {
        let mut a = HashMap::new();
        a.insert(Sector::Technology, 0.5);
        a.insert(Sector::Healthcare, 0.5);
        let sim = sector_jaccian(&a, &a);
        assert!((sim - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_cluster_by_conviction() {
        let f1 = filing("C1", "Tech Fund A", vec![
            h("AAPL", 1000, 500_000.0, 0.7),
            h("MSFT", 500, 200_000.0, 0.3),
        ], 700_000.0);
        let f2 = filing("C2", "Tech Fund B", vec![
            h("AAPL", 800, 400_000.0, 0.6),
            h("MSFT", 400, 200_000.0, 0.3),
            h("NVDA", 100, 100_000.0, 0.1),
        ], 700_000.0);
        let f3 = filing("C3", "Health Fund", vec![
            h("JNJ", 1000, 500_000.0, 0.7),
            h("UNH", 500, 200_000.0, 0.3),
        ], 700_000.0);

        let clusters = cluster_by_conviction(&[&f1, &f2, &f3], 0.3);
        assert_eq!(clusters.len(), 2); // Tech pair + Health solo
        // One cluster should have 2 members, other 1
        let counts: Vec<usize> = clusters.iter().map(|c| c.members.len()).collect();
        assert!(counts.contains(&2));
        assert!(counts.contains(&1));
    }

    #[test]
    fn test_to_signals() {
        let engine = ConsensusEngine::new();
        let d1 = diff_result("C1", "Fund A", vec![
            pd("AAPL", Sector::Technology, PositionChange::New, 0, 1000, 0.0, 500_000.0, 0.03),
        ]);
        let d2 = diff_result("C2", "Fund B", vec![
            pd("AAPL", Sector::Technology, PositionChange::Increased, 500, 700, 250_000.0, 350_000.0, 0.02),
        ]);

        let signals = engine.to_signals(&[d1, d2]);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].ticker, "AAPL");
        assert_eq!(signals[0].holder_count, 2);
    }

    /// Helper alias to avoid the typo in test function
    fn sector_jaccian(a: &HashMap<Sector, f64>, b: &HashMap<Sector, f64>) -> f64 {
        sector_jaccard(a, b)
    }
}
