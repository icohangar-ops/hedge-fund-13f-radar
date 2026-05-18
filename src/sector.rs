//! Sector rotation analysis.
//!
//! Tracks how capital flows between GICS sectors across quarters:
//! - Sector allocation mapping per manager
//! - Quarter-over-quarter sector flow calculation
//! - Rotation heat map generation
//! - Sector momentum scoring
//! - Sector pair trade detection (contrarian rotation signals)

use crate::diff::DiffResult;
use crate::types::{Filing13F, PositionChange, RotationMetric, Sector};
use ndarray::Array2;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Sector Snapshot — per-manager, per-quarter allocation
// ---------------------------------------------------------------------------

/// Captures a single manager's sector allocation at a point in time.
#[derive(Debug, Clone)]
pub struct SectorSnapshot {
    pub manager_cik: String,
    pub manager_name: String,
    pub quarter: String,
    pub allocations: HashMap<Sector, f64>,
    pub total_aum: f64,
}

impl SectorSnapshot {
    pub fn from_filing(filing: &Filing13F, quarter: &str) -> Self {
        Self {
            manager_cik: filing.manager.cik.clone(),
            manager_name: filing.manager.name.clone(),
            quarter: quarter.to_string(),
            allocations: filing.sector_allocations(),
            total_aum: filing.total_aum,
        }
    }

    /// Get allocation for a sector (0.0 if not present).
    pub fn weight(&self, sector: Sector) -> f64 {
        self.allocations.get(&sector).copied().unwrap_or(0.0)
    }
}

// ---------------------------------------------------------------------------
// Rotation Heat Map
// ---------------------------------------------------------------------------

/// Heat map showing capital flows between sector pairs.
/// Entry (i, j) = net flow FROM sector i TO sector j.
#[derive(Debug, Clone)]
pub struct RotationHeatMap {
    /// Ordered list of sectors (row/column order).
    pub sectors: Vec<Sector>,
    /// 2D matrix: rows = source sector, cols = destination sector.
    /// Values are aggregated dollar-weight changes (positive = flow from row to col).
    pub matrix: Vec<Vec<f64>>,
}

impl RotationHeatMap {
    /// Create a zero-initialized heat map for the given sectors.
    pub fn new(sectors: &[Sector]) -> Self {
        let n = sectors.len();
        Self {
            sectors: sectors.to_vec(),
            matrix: vec![vec![0.0; n]; n],
        }
    }

    /// Get the flow from source to destination sector.
    pub fn flow(&self, from: Sector, to: Sector) -> f64 {
        let i = self.sectors.iter().position(|s| *s == from);
        let j = self.sectors.iter().position(|s| *s == to);
        match (i, j) {
            (Some(r), Some(c)) => self.matrix[r][c],
            _ => 0.0,
        }
    }

    /// Record a flow from one sector to another.
    pub fn add_flow(&mut self, from: Sector, to: Sector, amount: f64) {
        let i = self.sectors.iter().position(|s| *s == from);
        let j = self.sectors.iter().position(|s| *s == to);
        if let (Some(r), Some(c)) = (i, j) {
            self.matrix[r][c] += amount;
        }
    }

    /// Convert to an ndarray Array2 for numerical analysis.
    pub fn to_array(&self) -> Array2<f64> {
        let n = self.sectors.len();
        let mut data = vec![0.0f64; n * n];
        for i in 0..n {
            for j in 0..n {
                data[i * n + j] = self.matrix[i][j];
            }
        }
        Array2::from_shape_vec((n, n), data).unwrap_or_else(|_| Array2::zeros((0, 0)))
    }

    /// Row-normalize the matrix to show proportional outflows.
    pub fn row_normalized(&self) -> Array2<f64> {
        let arr = self.to_array();
        let n = arr.nrows();
        if n == 0 {
            return arr;
        }
        let mut normalized = arr.clone();
        for i in 0..n {
            let row_sum: f64 = normalized.row(i).sum();
            if row_sum > 0.0 {
                normalized.row_mut(i).mapv_inplace(|v| v / row_sum);
            }
        }
        normalized
    }

    /// Net sector flow: sum of inflows minus outflows for each sector.
    pub fn net_sector_flows(&self) -> HashMap<Sector, f64> {
        let mut flows = HashMap::new();
        let n = self.sectors.len();
        for (idx, sector) in self.sectors.iter().enumerate() {
            let inflow: f64 = (0..n).filter(|&j| j != idx).map(|j| self.matrix[j][idx]).sum();
            let outflow: f64 = (0..n).filter(|&j| j != idx).map(|j| self.matrix[idx][j]).sum();
            flows.insert(*sector, inflow - outflow);
        }
        flows
    }
}

// ---------------------------------------------------------------------------
// Momentum
// ---------------------------------------------------------------------------

/// Sector momentum computed from weighted average of position changes.
#[derive(Debug, Clone)]
pub struct SectorMomentum {
    pub sector: Sector,
    /// Momentum score: positive = inflow momentum, negative = outflow.
    pub score: f64,
    /// Number of managers increasing allocation.
    pub bulls: usize,
    /// Number of managers decreasing allocation.
    pub bears: usize,
    /// Average weight change across managers.
    pub avg_weight_change: f64,
    /// Net AUM change in millions.
    pub net_aum_change_m: f64,
}

// ---------------------------------------------------------------------------
// Pair Trade Signal
// ---------------------------------------------------------------------------

/// A contrarian sector pair trade signal: rotate from one sector to another.
#[derive(Debug, Clone)]
pub struct PairTradeSignal {
    /// Sector with outflow momentum (short candidate).
    pub from_sector: Sector,
    /// Sector with inflow momentum (long candidate).
    pub to_sector: Sector,
    /// Strength of the divergence (higher = stronger signal).
    pub divergence_score: f64,
    /// Difference in momentum scores.
    pub momentum_spread: f64,
    /// Number of funds rotating from → to.
    pub rotating_funds: usize,
}

// ---------------------------------------------------------------------------
// Sector Rotation Engine
// ---------------------------------------------------------------------------

/// Main engine for sector rotation analysis.
pub struct SectorRotationEngine {
    /// Sectors to include in analysis.
    pub sectors: Vec<Sector>,
}

impl SectorRotationEngine {
    pub fn new() -> Self {
        Self {
            sectors: Sector::all().iter().filter(|s| **s != Sector::Unknown).copied().collect(),
        }
    }

    /// Create snapshots from filings grouped by quarter.
    pub fn build_snapshots(&self, filings: &[&Filing13F], quarter: &str) -> Vec<SectorSnapshot> {
        filings.iter().map(|f| SectorSnapshot::from_filing(f, quarter)).collect()
    }

    /// Compute quarter-over-quarter rotation metrics for a single manager.
    pub fn compute_rotation(
        &self,
        prior: &SectorSnapshot,
        current: &SectorSnapshot,
    ) -> Vec<RotationMetric> {
        let mut metrics = Vec::new();

        for &sector in &self.sectors {
            let prior_w = prior.weight(sector);
            let curr_w = current.weight(sector);
            let delta = curr_w - prior_w;
            let pct = if prior_w > 0.0 {
                (curr_w - prior_w) / prior_w * 100.0
            } else if curr_w > 0.0 {
                100.0 // new allocation
            } else {
                0.0
            };

            metrics.push(RotationMetric {
                sector,
                prior_weight: prior_w,
                current_weight: curr_w,
                weight_delta: delta,
                pct_change: pct,
                inflows: 0, // filled by batch method
                outflows: 0,
                momentum: delta * 100.0, // simple momentum = weight delta * 100
            });
        }

        metrics
    }

    /// Aggregate rotation metrics across all manager diffs for a quarter.
    pub fn aggregate_rotation(
        &self,
        prior_filings: &[&Filing13F],
        current_filings: &[&Filing13F],
        diffs: &[DiffResult],
    ) -> Vec<RotationMetric> {
        // Match managers between quarters
        let prior_map: HashMap<&str, &Filing13F> =
            prior_filings.iter().map(|f| (f.manager.cik.as_str(), *f)).collect();
        let current_map: HashMap<&str, &Filing13F> =
            current_filings.iter().map(|f| (f.manager.cik.as_str(), *f)).collect();

        let mut sector_deltas: HashMap<Sector, (f64, f64, usize, usize)> = HashMap::new();
        for &sector in &self.sectors {
            sector_deltas.insert(sector, (0.0, 0.0, 0, 0));
        }

        // Compute weight changes per manager
        let mut matched = 0;
        for (cik, curr_f) in &current_map {
            if let Some(prev_f) = prior_map.get(cik) {
                matched += 1;
                let prior_alloc = prev_f.sector_allocations();
                let curr_alloc = curr_f.sector_allocations();

                for &sector in &self.sectors {
                    let pw = prior_alloc.get(&sector).copied().unwrap_or(0.0);
                    let cw = curr_alloc.get(&sector).copied().unwrap_or(0.0);
                    let entry = sector_deltas.get_mut(&sector).unwrap();
                    entry.0 += pw; // sum of prior weights
                    entry.1 += cw; // sum of current weights
                }
            }
        }

        // Count inflows/outflows from diffs
        for diff in diffs {
            for d in &diff.diffs {
                match d.change {
                    PositionChange::New | PositionChange::Increased => {
                        if let Some(entry) = sector_deltas.get_mut(&d.sector) {
                            entry.2 += 1; // inflow count
                        }
                    }
                    PositionChange::Decreased | PositionChange::Exited => {
                        if let Some(entry) = sector_deltas.get_mut(&d.sector) {
                            entry.3 += 1; // outflow count
                        }
                    }
                    PositionChange::Unchanged => {}
                }
            }
        }

        let n = matched.max(1) as f64;
        let mut metrics = Vec::new();

        for &sector in &self.sectors {
            let (sum_prior, sum_curr, inflows, outflows) = sector_deltas[&sector];
            let avg_prior = sum_prior / n;
            let avg_curr = sum_curr / n;
            let delta = avg_curr - avg_prior;
            let pct = if avg_prior > 0.0 { delta / avg_prior * 100.0 } else { 0.0 };

            // Momentum: weighted combination of weight change and directional flow
            let total_flows = (inflows + outflows).max(1) as f64;
            let flow_signal = (inflows as f64 - outflows as f64) / total_flows;
            let momentum = delta * 50.0 + flow_signal * 10.0;

            metrics.push(RotationMetric {
                sector,
                prior_weight: avg_prior,
                current_weight: avg_curr,
                weight_delta: delta,
                pct_change: pct,
                inflows,
                outflows,
                momentum,
            });
        }

        // Sort by momentum descending
        metrics.sort_by(|a, b| b.momentum.partial_cmp(&a.momentum).unwrap_or(std::cmp::Ordering::Equal));
        metrics
    }

    /// Build a rotation heat map from prior → current filings.
    pub fn build_heat_map(
        &self,
        prior_filings: &[&Filing13F],
        current_filings: &[&Filing13F],
        diffs: &[DiffResult],
    ) -> RotationHeatMap {
        let mut hm = RotationHeatMap::new(&self.sectors);

        let prior_map: HashMap<&str, &Filing13F> =
            prior_filings.iter().map(|f| (f.manager.cik.as_str(), *f)).collect();
        let _current_map: HashMap<&str, &Filing13F> =
            current_filings.iter().map(|f| (f.manager.cik.as_str(), *f)).collect();

        for diff in diffs {
            for d in &diff.diffs {
                let cik = diff.manager_cik.as_str();
                let from_sector = if d.prior_shares > 0 {
                    // Look up prior sector
                    if let Some(prev_f) = prior_map.get(cik) {
                        prev_f.holdings.iter()
                            .find(|h| h.ticker == d.ticker)
                            .map(|h| h.sector)
                            .unwrap_or(d.sector)
                    } else {
                        d.sector
                    }
                } else {
                    d.sector // For new positions, infer from ticker
                };

                let to_sector = d.sector;

                let amount = if d.change == PositionChange::New {
                    d.current_value
                } else if d.change == PositionChange::Exited {
                    d.prior_value
                } else {
                    d.delta_value.abs()
                };

                // Exits flow money "out" of a sector (to a catch-all)
                match d.change {
                    PositionChange::Exited => {
                        hm.add_flow(from_sector, to_sector, amount);
                    }
                    PositionChange::New => {
                        hm.add_flow(Sector::Unknown, to_sector, amount);
                    }
                    _ => {
                        // For increases/decreases, we just note the sector direction
                        if d.delta_value > 0.0 {
                            hm.add_flow(Sector::Unknown, to_sector, amount);
                        } else {
                            hm.add_flow(from_sector, Sector::Unknown, amount);
                        }
                    }
                }
            }
        }

        hm
    }

    /// Compute momentum scores for all sectors.
    pub fn compute_momentum(
        &self,
        prior_snapshots: &[SectorSnapshot],
        current_snapshots: &[SectorSnapshot],
        diffs: &[DiffResult],
    ) -> Vec<SectorMomentum> {
        let prior_map: HashMap<&str, &SectorSnapshot> =
            prior_snapshots.iter().map(|s| (s.manager_cik.as_str(), s)).collect();
        let current_map: HashMap<&str, &SectorSnapshot> =
            current_snapshots.iter().map(|s| (s.manager_cik.as_str(), s)).collect();

        let mut sector_data: HashMap<Sector, (f64, f64, usize, usize)> = HashMap::new();
        for &sector in &self.sectors {
            sector_data.insert(sector, (0.0, 0.0, 0, 0));
        }

        for (cik, curr) in &current_map {
            if let Some(prev) = prior_map.get(cik) {
                for &sector in &self.sectors {
                    let pw = prev.weight(sector);
                    let cw = curr.weight(sector);
                    let delta = cw - pw;
                    let entry = sector_data.get_mut(&sector).unwrap();
                    entry.0 += delta; // sum of weight changes
                    entry.1 += 1.0;   // count of managers
                }
            }
        }

        // Count bulls/bears from diffs
        for diff in diffs {
            for d in &diff.diffs {
                match d.change {
                    PositionChange::New | PositionChange::Increased => {
                        if let Some(entry) = sector_data.get_mut(&d.sector) {
                            entry.2 += 1;
                        }
                    }
                    PositionChange::Decreased | PositionChange::Exited => {
                        if let Some(entry) = sector_data.get_mut(&d.sector) {
                            entry.3 += 1;
                        }
                    }
                    _ => {}
                }
            }
        }

        let mut momentum_list = Vec::new();
        for &sector in &self.sectors {
            let (sum_delta, count, bulls, bears) = sector_data[&sector];
            let avg_change = if count > 0.0 { sum_delta / count } else { 0.0 };
            let net_flow = bulls as f64 - bears as f64;
            let score = avg_change * 100.0 + net_flow * 0.5;
            let net_aum = sum_delta * 1000.0 / 1_000_000.0; // rough approximation

            momentum_list.push(SectorMomentum {
                sector,
                score,
                bulls,
                bears,
                avg_weight_change: avg_change,
                net_aum_change_m: net_aum,
            });
        }

        momentum_list.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        momentum_list
    }

    /// Detect sector pair trade signals based on momentum divergence.
    pub fn detect_pair_trades(
        &self,
        momentum: &[SectorMomentum],
        min_spread: f64,
        min_rotating: usize,
    ) -> Vec<PairTradeSignal> {
        let mut signals = Vec::new();

        for i in 0..momentum.len() {
            for j in (i + 1)..momentum.len() {
                let a = &momentum[i];
                let b = &momentum[j];

                // Only consider meaningful pairs with opposing momentum
                if a.score * b.score >= 0.0 {
                    continue;
                }

                let spread = (a.score - b.score).abs();
                if spread < min_spread {
                    continue;
                }

                let rotating = a.bulls.min(b.bears) + a.bears.min(b.bulls);
                if rotating < min_rotating {
                    continue;
                }

                let (from, to, _from_score, _to_score) = if a.score < b.score {
                    (a, b, a.score, b.score)
                } else {
                    (b, a, b.score, a.score)
                };

                let divergence = spread * 0.1 + rotating as f64 * 2.0;

                signals.push(PairTradeSignal {
                    from_sector: from.sector,
                    to_sector: to.sector,
                    divergence_score: divergence,
                    momentum_spread: spread,
                    rotating_funds: rotating,
                });
            }
        }

        // Sort by divergence score descending
        signals.sort_by(|a, b| b.divergence_score.partial_cmp(&a.divergence_score).unwrap_or(std::cmp::Ordering::Equal));
        signals
    }
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

    #[test]
    fn test_sector_snapshot_from_filing() {
        let f = filing("C1", "Tech Fund", vec![
            h("AAPL", 1000, 700_000.0, 0.7),
            h("JNJ", 200, 300_000.0, 0.3),
        ], 1_000_000.0);
        let snap = SectorSnapshot::from_filing(&f, "2024-Q2");
        assert_eq!(snap.quarter, "2024-Q2");
        assert!((snap.weight(Sector::Technology) - 0.7).abs() < 0.01);
        assert!((snap.weight(Sector::Healthcare) - 0.3).abs() < 0.01);
        assert!((snap.weight(Sector::Energy)).abs() < 1e-9);
    }

    #[test]
    fn test_rotation_metric_single_manager() {
        let engine = SectorRotationEngine::new();

        let prior = SectorSnapshot {
            manager_cik: "C1".into(),
            manager_name: "Fund A".into(),
            quarter: "2024-Q1".into(),
            allocations: {
                let mut m = HashMap::new();
                m.insert(Sector::Technology, 0.5);
                m.insert(Sector::Healthcare, 0.3);
                m.insert(Sector::Finance, 0.2);
                m
            },
            total_aum: 1_000_000.0,
        };

        let current = SectorSnapshot {
            manager_cik: "C1".into(),
            manager_name: "Fund A".into(),
            quarter: "2024-Q2".into(),
            allocations: {
                let mut m = HashMap::new();
                m.insert(Sector::Technology, 0.6);
                m.insert(Sector::Healthcare, 0.2);
                m.insert(Sector::Finance, 0.2);
                m
            },
            total_aum: 1_200_000.0,
        };

        let metrics = engine.compute_rotation(&prior, &current);
        let tech = metrics.iter().find(|m| m.sector == Sector::Technology).unwrap();
        assert!((tech.weight_delta - 0.1).abs() < 1e-9);
        assert!((tech.pct_change - 20.0).abs() < 1e-6);

        let health = metrics.iter().find(|m| m.sector == Sector::Healthcare).unwrap();
        assert!((health.weight_delta - (-0.1)).abs() < 1e-9);
    }

    #[test]
    fn test_aggregate_rotation() {
        let engine = SectorRotationEngine::new();

        let prior1 = filing("C1", "Fund A", vec![
            h("AAPL", 1000, 500_000.0, 0.5),
            h("JNJ", 500, 500_000.0, 0.5),
        ], 1_000_000.0);
        let prior2 = filing("C2", "Fund B", vec![
            h("AAPL", 800, 400_000.0, 0.4),
            h("JPM", 600, 600_000.0, 0.6),
        ], 1_000_000.0);

        let curr1 = filing("C1", "Fund A", vec![
            h("AAPL", 1200, 600_000.0, 0.6),
            h("JNJ", 400, 400_000.0, 0.4),
        ], 1_000_000.0);
        let curr2 = filing("C2", "Fund B", vec![
            h("AAPL", 1000, 500_000.0, 0.5),
            h("JPM", 500, 500_000.0, 0.5),
        ], 1_000_000.0);

        let metrics = engine.aggregate_rotation(
            &[&prior1, &prior2],
            &[&curr1, &curr2],
            &[],
        );

        let tech = metrics.iter().find(|m| m.sector == Sector::Technology).unwrap();
        // Both funds increased tech weight: (0.5→0.6) and (0.4→0.5) → avg delta = 0.1
        assert!(tech.weight_delta > 0.0);
    }

    #[test]
    fn test_heat_map_new() {
        let sectors = vec![Sector::Technology, Sector::Healthcare, Sector::Finance];
        let hm = RotationHeatMap::new(&sectors);
        assert_eq!(hm.matrix.len(), 3);
        assert_eq!(hm.matrix[0].len(), 3);
        assert!((hm.flow(Sector::Technology, Sector::Healthcare)).abs() < 1e-9);
    }

    #[test]
    fn test_heat_map_add_flow() {
        let sectors = vec![Sector::Technology, Sector::Healthcare];
        let mut hm = RotationHeatMap::new(&sectors);
        hm.add_flow(Sector::Technology, Sector::Healthcare, 100.0);
        assert!((hm.flow(Sector::Technology, Sector::Healthcare) - 100.0).abs() < 1e-9);
        assert!((hm.flow(Sector::Healthcare, Sector::Technology)).abs() < 1e-9);
    }

    #[test]
    fn test_heat_map_to_array() {
        let sectors = vec![Sector::Technology, Sector::Healthcare];
        let mut hm = RotationHeatMap::new(&sectors);
        hm.add_flow(Sector::Technology, Sector::Healthcare, 50.0);
        let arr = hm.to_array();
        assert_eq!(arr.shape(), &[2, 2]);
        assert!((arr[[0, 1]] - 50.0).abs() < 1e-9);
    }

    #[test]
    fn test_heat_map_net_flows() {
        let sectors = vec![Sector::Technology, Sector::Healthcare];
        let mut hm = RotationHeatMap::new(&sectors);
        hm.add_flow(Sector::Technology, Sector::Healthcare, 100.0);
        hm.add_flow(Sector::Healthcare, Sector::Technology, 30.0);

        let flows = hm.net_sector_flows();
        // Tech: inflow 30, outflow 100 → net = -70
        assert!((flows[&Sector::Technology] - (-70.0)).abs() < 1e-9);
        // Health: inflow 100, outflow 30 → net = 70
        assert!((flows[&Sector::Healthcare] - 70.0).abs() < 1e-9);
    }

    #[test]
    fn test_heat_map_row_normalized() {
        let sectors = vec![Sector::Technology, Sector::Healthcare];
        let mut hm = RotationHeatMap::new(&sectors);
        hm.add_flow(Sector::Technology, Sector::Healthcare, 75.0);
        hm.add_flow(Sector::Technology, Sector::Technology, 25.0);

        let norm = hm.row_normalized();
        // Row 0: [25, 75] → [0.25, 0.75]
        assert!((norm[[0, 0]] - 0.25).abs() < 1e-9);
        assert!((norm[[0, 1]] - 0.75).abs() < 1e-9);
    }

    #[test]
    fn test_momentum_computation() {
        let engine = SectorRotationEngine::new();

        let prior = SectorSnapshot {
            manager_cik: "C1".into(),
            manager_name: "Fund A".into(),
            quarter: "2024-Q1".into(),
            allocations: {
                let mut m = HashMap::new();
                m.insert(Sector::Technology, 0.5);
                m.insert(Sector::Healthcare, 0.5);
                m
            },
            total_aum: 1_000_000.0,
        };

        let current = SectorSnapshot {
            manager_cik: "C1".into(),
            manager_name: "Fund A".into(),
            quarter: "2024-Q2".into(),
            allocations: {
                let mut m = HashMap::new();
                m.insert(Sector::Technology, 0.7);
                m.insert(Sector::Healthcare, 0.3);
                m
            },
            total_aum: 1_000_000.0,
        };

        let momentum = engine.compute_momentum(&[prior], &[current], &[]);
        let tech = momentum.iter().find(|m| m.sector == Sector::Technology).unwrap();
        assert!(tech.score > 0.0);
        let health = momentum.iter().find(|m| m.sector == Sector::Healthcare).unwrap();
        assert!(health.score < tech.score);
    }

    #[test]
    fn test_pair_trade_detection() {
        let momentum = vec![
            SectorMomentum {
                sector: Sector::Technology,
                score: 15.0,
                bulls: 8,
                bears: 2,
                avg_weight_change: 0.03,
                net_aum_change_m: 500.0,
            },
            SectorMomentum {
                sector: Sector::Healthcare,
                score: -12.0,
                bulls: 2,
                bears: 8,
                avg_weight_change: -0.02,
                net_aum_change_m: -400.0,
            },
            SectorMomentum {
                sector: Sector::Finance,
                score: 1.0,
                bulls: 5,
                bears: 5,
                avg_weight_change: 0.001,
                net_aum_change_m: 10.0,
            },
        ];

        let engine = SectorRotationEngine::new();
        let signals = engine.detect_pair_trades(&momentum, 5.0, 1);
        assert!(!signals.is_empty());
        // Should detect Tech→Healthcare pair
        let found = signals.iter().any(|s|
            s.from_sector == Sector::Healthcare && s.to_sector == Sector::Technology
        );
        assert!(found);
    }

    #[test]
    fn test_pair_trade_no_signal_same_direction() {
        let momentum = vec![
            SectorMomentum {
                sector: Sector::Technology,
                score: 10.0,
                bulls: 7,
                bears: 3,
                avg_weight_change: 0.02,
                net_aum_change_m: 200.0,
            },
            SectorMomentum {
                sector: Sector::Healthcare,
                score: 5.0,
                bulls: 6,
                bears: 4,
                avg_weight_change: 0.01,
                net_aum_change_m: 100.0,
            },
        ];

        let engine = SectorRotationEngine::new();
        let signals = engine.detect_pair_trades(&momentum, 5.0, 1);
        // Both positive momentum — no contrarian signal
        assert!(signals.is_empty());
    }
}
