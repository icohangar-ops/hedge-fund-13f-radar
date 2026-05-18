//! End-to-end pipeline integration.
//!
//! Orchestrates the full 13F analysis workflow:
//! ingest → diff → consensus → sector rotation.
//!
//! The pipeline accepts raw filings, produces structured analysis results,
//! and provides a summary report.

use crate::consensus::{
    ConsensusEngine, ConvictionCluster, SectorConsensus,
    TickerConsensus, WhaleTracker,
};
use crate::diff::diff_filings;
use crate::types::{CusipMap, ConsensusSignal, Filing13F, Holding, RotationMetric, Sector};
use crate::ingest::{QuarterlyAggregator, validate_filing};
use crate::sector::{
    PairTradeSignal, RotationHeatMap, SectorMomentum, SectorRotationEngine,
};
use anyhow::Result;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Pipeline Config
// ---------------------------------------------------------------------------

/// Configuration for the analysis pipeline.
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// CUSIP to ticker mapping.
    pub cusip_map: CusipMap,
    /// Minimum holders to qualify as high-conviction.
    pub min_conviction_holders: usize,
    /// Minimum average conviction score.
    pub min_avg_conviction: f64,
    /// Whale move threshold in millions.
    pub whale_threshold_m: f64,
    /// Minimum momentum spread for pair trades.
    pub pair_trade_min_spread: f64,
    /// Minimum rotating funds for pair trade signal.
    pub pair_trade_min_rotating: usize,
    /// Sector clustering similarity threshold (0..1).
    pub cluster_similarity: f64,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            cusip_map: CusipMap::new(),
            min_conviction_holders: 3,
            min_avg_conviction: 1.5,
            whale_threshold_m: 0.5,
            pair_trade_min_spread: 5.0,
            pair_trade_min_rotating: 2,
            cluster_similarity: 0.3,
        }
    }
}

// ---------------------------------------------------------------------------
// Pipeline Result
// ---------------------------------------------------------------------------

/// Complete output of the pipeline for a single quarter comparison.
#[derive(Debug, Clone)]
pub struct PipelineResult {
    /// Prior quarter label (e.g. "2024-Q1").
    pub prior_quarter: String,
    /// Current quarter label (e.g. "2024-Q2").
    pub current_quarter: String,
    /// Per-manager position diffs.
    pub diffs: Vec<crate::diff::DiffResult>,
    /// Per-ticker consensus signals.
    pub consensus_signals: Vec<ConsensusSignal>,
    /// High-conviction tickers.
    pub high_conviction: Vec<TickerConsensus>,
    /// Sector-level consensus.
    pub sector_consensus: HashMap<Sector, SectorConsensus>,
    /// Sector rotation metrics.
    pub rotation_metrics: Vec<RotationMetric>,
    /// Sector momentum rankings.
    pub sector_momentum: Vec<SectorMomentum>,
    /// Rotation heat map.
    pub heat_map: RotationHeatMap,
    /// Whale moves for the quarter.
    pub whale_moves: WhaleTracker,
    /// Pair trade signals.
    pub pair_trades: Vec<PairTradeSignal>,
    /// Conviction clusters.
    pub clusters: Vec<ConvictionCluster>,
    /// Validation warnings.
    pub warnings: Vec<String>,
}

impl PipelineResult {
    /// Top-N bullish consensus tickers (by net direction).
    pub fn top_bullish_consensus(&self, n: usize) -> Vec<&ConsensusSignal> {
        let mut sorted: Vec<&ConsensusSignal> = self.consensus_signals.iter().collect();
        sorted.sort_by(|a, b| b.net_direction.partial_cmp(&a.net_direction).unwrap_or(std::cmp::Ordering::Equal));
        sorted.truncate(n);
        sorted
    }

    /// Top-N bearish consensus tickers.
    pub fn top_bearish_consensus(&self, n: usize) -> Vec<&ConsensusSignal> {
        let mut sorted: Vec<&ConsensusSignal> = self.consensus_signals.iter().collect();
        sorted.sort_by(|a, b| a.net_direction.partial_cmp(&b.net_direction).unwrap_or(std::cmp::Ordering::Equal));
        sorted.truncate(n);
        sorted
    }

    /// Sectors with the strongest inflow momentum.
    pub fn top_inflow_sectors(&self, n: usize) -> Vec<&SectorMomentum> {
        self.sector_momentum.iter().take(n).collect()
    }

    /// Sectors with the strongest outflow momentum.
    pub fn top_outflow_sectors(&self, n: usize) -> Vec<&SectorMomentum> {
        self.sector_momentum.iter().rev().take(n).collect()
    }

    /// Generate a text summary report.
    pub fn summary_report(&self) -> String {
        let mut report = String::new();
        report.push_str(&format!("═══ 13F Radar Report: {} → {} ═══\n\n",
            self.prior_quarter, self.current_quarter));

        report.push_str(&format!("Managers analyzed: {}\n", self.diffs.len()));
        report.push_str(&format!("Unique tickers tracked: {}\n", self.consensus_signals.len()));
        report.push_str(&format!("Whale moves: {}\n", self.whale_moves.moves.len()));
        report.push_str(&format!("Pair trade signals: {}\n\n", self.pair_trades.len()));

        report.push_str("─── High Conviction Tickers ───\n");
        for hc in &self.high_conviction {
            report.push_str(&format!("  {} ({}): {} holders, avg conviction {:.2}, dir {:.2}\n",
                hc.ticker, hc.sector, hc.holder_count, hc.avg_conviction,
                hc.direction_ratio()));
        }
        report.push('\n');

        report.push_str("─── Sector Momentum ───\n");
        for sm in &self.sector_momentum {
            let arrow = if sm.score > 0.0 { "↑" } else { "↓" };
            report.push_str(&format!("  {} {} {:.1} (bulls: {}, bears: {})\n",
                sm.sector, arrow, sm.score, sm.bulls, sm.bears));
        }
        report.push('\n');

        report.push_str("─── Pair Trade Signals ───\n");
        for pt in &self.pair_trades {
            report.push_str(&format!("  {} → {}: divergence {:.1}, spread {:.1}, {} funds rotating\n",
                pt.from_sector, pt.to_sector, pt.divergence_score,
                pt.momentum_spread, pt.rotating_funds));
        }
        report.push('\n');

        if !self.warnings.is_empty() {
            report.push_str("─── Warnings ───\n");
            for w in &self.warnings {
                report.push_str(&format!("  ⚠ {}\n", w));
            }
        }

        report
    }
}

// ---------------------------------------------------------------------------
// Pipeline Engine
// ---------------------------------------------------------------------------

/// Main pipeline orchestrator for 13F analysis.
pub struct Pipeline {
    config: PipelineConfig,
    consensus_engine: ConsensusEngine,
    sector_engine: SectorRotationEngine,
}

impl Pipeline {
    pub fn new() -> Self {
        Self::with_config(PipelineConfig::default())
    }

    pub fn with_config(config: PipelineConfig) -> Self {
        let consensus_engine = ConsensusEngine::with_params(
            config.min_conviction_holders,
            config.min_avg_conviction,
        );
        let sector_engine = SectorRotationEngine::new();
        Self { config, consensus_engine, sector_engine }
    }

    /// Run the full pipeline on prior and current quarter filings.
    pub fn run(
        &self,
        prior_filings: &[Filing13F],
        current_filings: &[Filing13F],
        prior_quarter: &str,
        current_quarter: &str,
    ) -> Result<PipelineResult> {
        let mut all_warnings = Vec::new();

        for f in prior_filings.iter().chain(current_filings.iter()) {
            match validate_filing(f) {
                Ok(warnings) => all_warnings.extend(warnings),
                Err(e) => all_warnings.push(format!("Validation error: {}", e)),
            }
        }

        let prior_map: HashMap<&str, &Filing13F> =
            prior_filings.iter().map(|f| (f.manager.cik.as_str(), f)).collect();
        let current_map: HashMap<&str, &Filing13F> =
            current_filings.iter().map(|f| (f.manager.cik.as_str(), f)).collect();

        // Step 1: Diff
        let mut diff_pairs: Vec<(&Filing13F, &Filing13F)> = Vec::new();
        let mut unmatched_managers = Vec::new();

        for (cik, curr) in &current_map {
            if let Some(prev) = prior_map.get(cik) {
                diff_pairs.push((*prev, *curr));
            } else {
                unmatched_managers.push((*curr).manager.name.clone());
            }
        }

        let diffs: Vec<crate::diff::DiffResult> = diff_pairs.iter()
            .map(|(p, c)| diff_filings(p, c))
            .collect();

        if !unmatched_managers.is_empty() {
            all_warnings.push(format!(
                "New managers with no prior quarter data: {}",
                unmatched_managers.join(", ")
            ));
        }

        // Step 2: Consensus
        let consensus_signals = self.consensus_engine.to_signals(&diffs);
        let high_conviction = self.consensus_engine.high_conviction_tickers(&diffs);
        let sector_consensus = self.consensus_engine.sector_consensus_with_diffs(
            &current_filings.iter().collect::<Vec<_>>(),
            &diffs,
        );

        // Step 3: Sector rotation
        let prior_refs: Vec<&Filing13F> = prior_filings.iter().collect();
        let curr_refs: Vec<&Filing13F> = current_filings.iter().collect();

        let rotation_metrics = self.sector_engine.aggregate_rotation(
            &prior_refs, &curr_refs, &diffs,
        );

        let prior_snaps = self.sector_engine.build_snapshots(&prior_refs, prior_quarter);
        let curr_snaps = self.sector_engine.build_snapshots(&curr_refs, current_quarter);

        let sector_momentum = self.sector_engine.compute_momentum(
            &prior_snaps, &curr_snaps, &diffs,
        );

        let heat_map = self.sector_engine.build_heat_map(
            &prior_refs, &curr_refs, &diffs,
        );

        let pair_trades = self.sector_engine.detect_pair_trades(
            &sector_momentum,
            self.config.pair_trade_min_spread,
            self.config.pair_trade_min_rotating,
        );

        // Step 4: Whale tracking
        let mut whale_tracker = WhaleTracker::new(self.config.whale_threshold_m);
        for diff in &diffs {
            whale_tracker.process_diff(diff);
        }

        // Step 5: Conviction clustering
        let clusters = crate::consensus::cluster_by_conviction(
            &curr_refs,
            self.config.cluster_similarity,
        );

        Ok(PipelineResult {
            prior_quarter: prior_quarter.to_string(),
            current_quarter: current_quarter.to_string(),
            diffs,
            consensus_signals,
            high_conviction,
            sector_consensus,
            rotation_metrics,
            sector_momentum,
            heat_map,
            whale_moves: whale_tracker,
            pair_trades,
            clusters,
            warnings: all_warnings,
        })
    }

    /// Run the pipeline from raw filing data using the aggregator.
    pub fn run_from_aggregator(
        &self,
        aggregator: &QuarterlyAggregator,
        prior_quarter: &str,
        current_quarter: &str,
    ) -> Result<PipelineResult> {
        let prior = aggregator.filings_for_quarter(prior_quarter);
        let current = aggregator.filings_for_quarter(current_quarter);

        if prior.is_empty() {
            anyhow::bail!("No filings found for prior quarter: {}", prior_quarter);
        }
        if current.is_empty() {
            anyhow::bail!("No filings found for current quarter: {}", current_quarter);
        }

        let prior_owned: Vec<Filing13F> = prior.into_iter().cloned().collect();
        let current_owned: Vec<Filing13F> = current.into_iter().cloned().collect();

        self.run(&prior_owned, &current_owned, prior_quarter, current_quarter)
    }
}

// ---------------------------------------------------------------------------
// Convenience builder for test pipelines
// ---------------------------------------------------------------------------

/// Helper to quickly build test pipeline data.
pub struct TestDataBuilder {
    cusip_map: CusipMap,
}

impl TestDataBuilder {
    pub fn new() -> Self {
        let mut cusip_map = CusipMap::new();
        cusip_map.insert("037833100", "AAPL", "Apple Inc");
        cusip_map.insert("478160104", "JNJ", "Johnson & Johnson");
        cusip_map.insert("02079K305", "GOOGL", "Alphabet Inc");
        cusip_map.insert("594918104", "MSFT", "Microsoft Corp");
        cusip_map.insert("46647Q103", "JPM", "JPMorgan Chase");
        cusip_map.insert("30303M102", "META", "Meta Platforms");
        cusip_map.insert("88160R101", "TSLA", "Tesla Inc");
        cusip_map.insert("92343V104", "NVDA", "NVIDIA Corp");
        cusip_map.insert("580135101", "MA", "Mastercard Inc");
        cusip_map.insert("949746101", "UNH", "UnitedHealth Group");
        cusip_map.insert("54615U100", "LLY", "Eli Lilly & Co");
        Self { cusip_map }
    }

    pub fn cusip_map(&self) -> CusipMap {
        self.cusip_map.clone()
    }

    pub fn holding(&self, ticker: &str, shares: i64, value: f64, weight: f64) -> Holding {
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

    pub fn filing(&self, cik: &str, name: &str, holdings: Vec<Holding>, date: chrono::NaiveDate, aum: f64) -> Filing13F {
        Filing13F {
            accession_number: format!("{}_{}", cik, date),
            manager: crate::types::Manager { cik: cik.into(), name: name.into(), filer_type: "HA".into() },
            report_date: date,
            filing_date: date,
            total_aum: aum,
            holdings,
            other_included_count: 0,
        }
    }

    pub fn default_config() -> PipelineConfig {
        let mut config = PipelineConfig::default();
        config.cusip_map = Self::new().cusip_map();
        config.min_conviction_holders = 2;
        config.min_avg_conviction = 0.5;
        config.whale_threshold_m = 0.01;
        config.pair_trade_min_spread = 1.0;
        config.pair_trade_min_rotating = 1;
        config.cluster_similarity = 0.2;
        config
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn tb() -> TestDataBuilder {
        TestDataBuilder::new()
    }

    fn make_manager_pair(name: &str, cik: &str, date1: NaiveDate, date2: NaiveDate) -> (Filing13F, Filing13F) {
        let prior_holdings = vec![
            tb().holding("AAPL", 1000, 500_000.0, 0.3),
            tb().holding("JNJ", 500, 300_000.0, 0.18),
            tb().holding("MSFT", 800, 400_000.0, 0.24),
            tb().holding("JPM", 400, 200_000.0, 0.12),
            tb().holding("XOM", 300, 150_000.0, 0.09),
            tb().holding("UNH", 200, 120_000.0, 0.07),
        ];

        let current_holdings = vec![
            tb().holding("AAPL", 1400, 700_000.0, 0.35),
            tb().holding("JNJ", 300, 180_000.0, 0.09),
            tb().holding("MSFT", 1000, 500_000.0, 0.25),
            tb().holding("JPM", 300, 150_000.0, 0.075),
            tb().holding("NVDA", 200, 300_000.0, 0.15),
            tb().holding("UNH", 250, 150_000.0, 0.075),
            tb().holding("META", 100, 20_000.0, 0.01),
        ];

        let prior_aum: f64 = prior_holdings.iter().map(|h| h.value).sum();
        let curr_aum: f64 = current_holdings.iter().map(|h| h.value).sum();

        let prior = tb().filing(cik, name, prior_holdings, date1, prior_aum);
        let current = tb().filing(cik, name, current_holdings, date2, curr_aum);
        (prior, current)
    }

    #[test]
    fn test_pipeline_full_run() {
        let config = TestDataBuilder::default_config();
        let pipeline = Pipeline::with_config(config);

        let (p1, c1) = make_manager_pair("Alpha Fund", "C0001",
            NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
            NaiveDate::from_ymd_opt(2024, 6, 30).unwrap());

        let (p2, c2) = make_manager_pair("Beta Fund", "C0002",
            NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
            NaiveDate::from_ymd_opt(2024, 6, 30).unwrap());

        let (p3, c3) = make_manager_pair("Gamma Fund", "C0003",
            NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
            NaiveDate::from_ymd_opt(2024, 6, 30).unwrap());

        let result = pipeline.run(
            &[p1, p2, p3],
            &[c1, c2, c3],
            "2024-Q1",
            "2024-Q2",
        ).unwrap();

        assert_eq!(result.diffs.len(), 3);
        assert!(!result.consensus_signals.is_empty());
        assert!(!result.sector_momentum.is_empty());
        assert!(!result.rotation_metrics.is_empty());
    }

    #[test]
    fn test_pipeline_consensus_signals() {
        let config = TestDataBuilder::default_config();
        let pipeline = Pipeline::with_config(config);

        let (p1, c1) = make_manager_pair("Alpha", "C1",
            NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
            NaiveDate::from_ymd_opt(2024, 6, 30).unwrap());

        let result = pipeline.run(&[p1], &[c1], "2024-Q1", "2024-Q2").unwrap();

        let aapl = result.consensus_signals.iter().find(|s| s.ticker == "AAPL");
        assert!(aapl.is_some());
        assert!(aapl.unwrap().net_direction > 0.0);
    }

    #[test]
    fn test_pipeline_whale_tracking() {
        let config = TestDataBuilder::default_config();
        let pipeline = Pipeline::with_config(config);

        let (p1, c1) = make_manager_pair("Big Fund", "C1",
            NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
            NaiveDate::from_ymd_opt(2024, 6, 30).unwrap());

        let result = pipeline.run(&[p1], &[c1], "2024-Q1", "2024-Q2").unwrap();
        assert!(!result.whale_moves.moves.is_empty());
    }

    #[test]
    fn test_pipeline_new_manager_warning() {
        let config = TestDataBuilder::default_config();
        let pipeline = Pipeline::with_config(config);

        let (_, c1) = make_manager_pair("New Fund", "CNEW",
            NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
            NaiveDate::from_ymd_opt(2024, 6, 30).unwrap());

        let result = pipeline.run(&[], &[c1], "2024-Q1", "2024-Q2").unwrap();
        assert!(!result.warnings.is_empty());
        assert!(result.warnings.iter().any(|w| w.contains("no prior quarter")));
    }

    #[test]
    fn test_pipeline_summary_report() {
        let config = TestDataBuilder::default_config();
        let pipeline = Pipeline::with_config(config);

        let (p1, c1) = make_manager_pair("Alpha", "C1",
            NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
            NaiveDate::from_ymd_opt(2024, 6, 30).unwrap());

        let result = pipeline.run(&[p1], &[c1], "2024-Q1", "2024-Q2").unwrap();
        let report = result.summary_report();
        assert!(report.contains("13F Radar Report"));
        assert!(report.contains("2024-Q1"));
        assert!(report.contains("2024-Q2"));
        assert!(report.contains("Sector Momentum"));
    }

    #[test]
    fn test_pipeline_top_bullish() {
        let config = TestDataBuilder::default_config();
        let pipeline = Pipeline::with_config(config);

        let (p1, c1) = make_manager_pair("Alpha", "C1",
            NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
            NaiveDate::from_ymd_opt(2024, 6, 30).unwrap());

        let result = pipeline.run(&[p1], &[c1], "2024-Q1", "2024-Q2").unwrap();
        let top = result.top_bullish_consensus(3);
        assert!(top.len() <= 3);
        for s in &top {
            assert!(s.net_direction >= 0.0);
        }
    }

    #[test]
    fn test_pipeline_sector_flow() {
        let config = TestDataBuilder::default_config();
        let pipeline = Pipeline::with_config(config);

        let (p1, c1) = make_manager_pair("Alpha", "C1",
            NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
            NaiveDate::from_ymd_opt(2024, 6, 30).unwrap());

        let result = pipeline.run(&[p1], &[c1], "2024-Q1", "2024-Q2").unwrap();
        let top_in = result.top_inflow_sectors(3);
        let top_out = result.top_outflow_sectors(3);
        assert!(top_in.len() > 0);
        assert!(top_out.len() > 0);
    }

    #[test]
    fn test_pipeline_with_aggregator() {
        let config = TestDataBuilder::default_config();
        let pipeline = Pipeline::with_config(config);

        let mut agg = QuarterlyAggregator::new();

        let (p1, c1) = make_manager_pair("Alpha", "C1",
            NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
            NaiveDate::from_ymd_opt(2024, 6, 30).unwrap());
        let (p2, c2) = make_manager_pair("Beta", "C2",
            NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
            NaiveDate::from_ymd_opt(2024, 6, 30).unwrap());

        agg.add(p1);
        agg.add(p2);
        agg.add(c1);
        agg.add(c2);

        let result = pipeline.run_from_aggregator(&agg, "2024-Q1", "2024-Q2").unwrap();
        assert_eq!(result.diffs.len(), 2);
    }

    #[test]
    fn test_pipeline_missing_quarter_error() {
        let config = TestDataBuilder::default_config();
        let pipeline = Pipeline::with_config(config);

        let agg = QuarterlyAggregator::new();
        let result = pipeline.run_from_aggregator(&agg, "2024-Q1", "2024-Q2");
        assert!(result.is_err());
    }

    #[test]
    fn test_test_data_builder() {
        let _h = TestDataBuilder::new().holding("AAPL", 100, 50_000.0, 0.1);
        let _f = TestDataBuilder::new().filing("C1", "Test", vec![], NaiveDate::from_ymd_opt(2024, 6, 30).unwrap(), 0.0);
        let _config = TestDataBuilder::default_config();
    }

    #[test]
    fn test_pipeline_heat_map() {
        let config = TestDataBuilder::default_config();
        let pipeline = Pipeline::with_config(config);

        let (p1, c1) = make_manager_pair("Alpha", "C1",
            NaiveDate::from_ymd_opt(2024, 3, 31).unwrap(),
            NaiveDate::from_ymd_opt(2024, 6, 30).unwrap());

        let result = pipeline.run(&[p1], &[c1], "2024-Q1", "2024-Q2").unwrap();
        let net = result.heat_map.net_sector_flows();
        assert!(!net.is_empty());
    }
}
