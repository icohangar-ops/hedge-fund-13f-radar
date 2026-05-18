//! # hedge-fund-13f-radar
//!
//! Hedge fund 13F filing analysis library for conviction tracking and sector rotation.
//!
//! This crate provides end-to-end analysis of SEC Form 13F filings:
//!
//! - **Ingestion** (`ingest`): Parse and normalize 13F filings from XML/CSV formats,
//!   resolve CUSIP-to-ticker mappings, and aggregate by quarter.
//!
//! - **Diff** (`diff`): Quarter-over-quarter position change detection — classify
//!   positions as new, increased, unchanged, decreased, or exited. Compute turnover
//!   rates, conviction changes, and portfolio impact metrics.
//!
//! - **Consensus** (`consensus`): Cross-fund consensus engine — identify high-conviction
//!   tickers, multi-manager agreement, sector-level consensus, conviction clustering,
//!   and whale tracking.
//!
//! - **Sector** (`sector`): Sector rotation analysis — compute sector flow metrics,
//!   rotation heat maps, sector momentum scoring, and pair trade signals.
//!
//! - **Pipeline** (`pipeline`): End-to-end integration that orchestrates the full
//!   workflow and produces summary reports.
//!
//! ## Example
//!
//! ```rust,ignore
//! use hedge_fund_13f_radar::pipeline::{Pipeline, PipelineConfig};
//! use hedge_fund_13f_radar::ingest::QuarterlyAggregator;
//!
//! let config = PipelineConfig::default();
//! let pipeline = Pipeline::with_config(config);
//!
//! let mut agg = QuarterlyAggregator::new();
//! agg.add(prior_filing);
//! agg.add(current_filing);
//!
//! let result = pipeline.run_from_aggregator(&agg, "2024-Q1", "2024-Q2")?;
//! println!("{}", result.summary_report());
//! ```

pub mod types;
pub mod ingest;
pub mod diff;
pub mod consensus;
pub mod sector;
pub mod pipeline;

// Re-export commonly used types at the crate root for convenience
pub use types::{
    ConsensusSignal, ConvictionLevel, CusipMap, Filing13F, Holding, Manager,
    PositionChange, RotationMetric, Sector,
};
pub use diff::{DiffResult, DiffSummary, PositionDiff};
pub use ingest::{FilingBuilder, IngestError, QuarterlyAggregator};
pub use consensus::{
    ConsensusEngine, ConvictionCluster, SectorConsensus, TickerConsensus, WhaleTracker,
};
pub use sector::{
    PairTradeSignal, RotationHeatMap, SectorMomentum, SectorRotationEngine, SectorSnapshot,
};
pub use pipeline::{Pipeline, PipelineConfig, PipelineResult};

// ---------------------------------------------------------------------------
// Integration Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::pipeline::TestDataBuilder;
    use chrono::NaiveDate;

    /// Helper to create a realistic multi-manager filing pair.
    fn make_fund_pair(cik: &str, name: &str, shift: i64) -> (Filing13F, Filing13F) {
        let tb = TestDataBuilder::new();

        let date1 = NaiveDate::from_ymd_opt(2024, 3, 31).unwrap();
        let date2 = NaiveDate::from_ymd_opt(2024, 6, 30).unwrap();

        let prior_holdings = vec![
            tb.holding("AAPL", 1000 + shift, 500_000.0, 0.3),
            tb.holding("MSFT", 800, 400_000.0, 0.24),
            tb.holding("JNJ", 500, 300_000.0, 0.18),
            tb.holding("JPM", 400, 200_000.0, 0.12),
            tb.holding("UNH", 200, 120_000.0, 0.07),
            tb.holding("XOM", 300, 150_000.0, 0.09),
        ];

        let current_holdings = vec![
            tb.holding("AAPL", 1400 + shift, 700_000.0, 0.35),
            tb.holding("MSFT", 1000, 500_000.0, 0.25),
            tb.holding("NVDA", 200, 300_000.0, 0.15),
            tb.holding("JPM", 300, 150_000.0, 0.075),
            tb.holding("JNJ", 300, 180_000.0, 0.09),
            tb.holding("UNH", 250, 150_000.0, 0.075),
        ];

        let prior_aum: f64 = prior_holdings.iter().map(|h| h.value).sum();
        let curr_aum: f64 = current_holdings.iter().map(|h| h.value).sum();

        let prior = tb.filing(cik, name, prior_holdings, date1, prior_aum);
        let current = tb.filing(cik, name, current_holdings, date2, curr_aum);
        (prior, current)
    }

    #[test]
    fn integration_full_pipeline_multi_manager() {
        let config = TestDataBuilder::default_config();
        let pipeline = Pipeline::with_config(config);

        let (p1, c1) = make_fund_pair("C0001", "Viking Global", 0);
        let (p2, c2) = make_fund_pair("C0002", "Bridgewater", 100);
        let (p3, c3) = make_fund_pair("C0003", "Third Point", 200);
        let (p4, c4) = make_fund_pair("C0004", "Greenlight", 300);
        let (p5, c5) = make_fund_pair("C0005", "Citadel", 400);

        let result = pipeline.run(
            &[p1, p2, p3, p4, p5],
            &[c1, c2, c3, c4, c5],
            "2024-Q1", "2024-Q2",
        ).unwrap();

        // Verify pipeline ran completely
        assert_eq!(result.diffs.len(), 5);
        assert!(!result.consensus_signals.is_empty());
        assert!(!result.high_conviction.is_empty());
        assert!(!result.sector_momentum.is_empty());
        assert!(!result.rotation_metrics.is_empty());
        assert!(!result.clusters.is_empty());

        // AAPL should be a high-conviction ticker (all 5 funds hold it)
        let aapl_hc = result.high_conviction.iter()
            .find(|t| t.ticker == "AAPL");
        assert!(aapl_hc.is_some());
        assert!(aapl_hc.unwrap().holder_count >= 5);

        // All funds increased AAPL — net direction should be positive
        let aapl_signal = result.consensus_signals.iter()
            .find(|s| s.ticker == "AAPL");
        assert!(aapl_signal.is_some());
        assert!(aapl_signal.unwrap().net_direction > 0.0);
    }

    #[test]
    fn integration_whale_detection() {
        let config = TestDataBuilder::default_config();
        let pipeline = Pipeline::with_config(config);

        let (p1, c1) = make_fund_pair("C0001", "Large Fund", 0);
        let (p2, c2) = make_fund_pair("C0002", "Small Fund", 0);

        let result = pipeline.run(
            &[p1, p2],
            &[c1, c2],
            "2024-Q1", "2024-Q2",
        ).unwrap();

        // With very low threshold, should detect several whale moves
        assert!(result.whale_moves.moves.len() > 0);
    }

    #[test]
    fn integration_sector_rotation() {
        let config = TestDataBuilder::default_config();
        let pipeline = Pipeline::with_config(config);

        let (p1, c1) = make_fund_pair("C0001", "Tech Fund", 0);
        let (p2, c2) = make_fund_pair("C0002", "Health Fund", 0);
        let (p3, c3) = make_fund_pair("C0003", "Macro Fund", 0);

        let result = pipeline.run(
            &[p1, p2, p3],
            &[c1, c2, c3],
            "2024-Q1", "2024-Q2",
        ).unwrap();

        // Technology should have positive momentum (all funds increased AAPL/MSFT)
        let tech_momentum = result.sector_momentum.iter()
            .find(|m| m.sector == Sector::Technology);
        assert!(tech_momentum.is_some());
        assert!(tech_momentum.unwrap().score > 0.0);

        // Energy should have negative momentum (all funds exited XOM)
        let energy_momentum = result.sector_momentum.iter()
            .find(|m| m.sector == Sector::Energy);
        assert!(energy_momentum.is_some());
        assert!(energy_momentum.unwrap().score < 0.0);
    }

    #[test]
    fn integration_with_aggregator() {
        let config = TestDataBuilder::default_config();
        let pipeline = Pipeline::with_config(config);

        let mut agg = QuarterlyAggregator::new();

        let (p1, c1) = make_fund_pair("C0001", "Fund A", 0);
        let (p2, c2) = make_fund_pair("C0002", "Fund B", 0);
        let (p3, c3) = make_fund_pair("C0003", "Fund C", 0);

        agg.add(p1); agg.add(p2); agg.add(p3);
        agg.add(c1); agg.add(c2); agg.add(c3);

        let result = pipeline.run_from_aggregator(&agg, "2024-Q1", "2024-Q2").unwrap();
        assert_eq!(result.diffs.len(), 3);
    }

    #[test]
    fn integration_report_generation() {
        let config = TestDataBuilder::default_config();
        let pipeline = Pipeline::with_config(config);

        let (p1, c1) = make_fund_pair("C0001", "Alpha", 0);
        let (p2, c2) = make_fund_pair("C0002", "Beta", 0);

        let result = pipeline.run(
            &[p1, p2],
            &[c1, c2],
            "2024-Q1", "2024-Q2",
        ).unwrap();

        let report = result.summary_report();
        assert!(report.contains("13F Radar Report"));
        assert!(report.contains("Managers analyzed: 2"));
        assert!(report.contains("Sector Momentum"));
        assert!(report.contains("High Conviction Tickers"));
    }

    #[test]
    fn integration_type_serialization() {
        let holding = Holding {
            cusip: "037833100".into(),
            ticker: "AAPL".into(),
            name: "Apple Inc".into(),
            sector: Sector::Technology,
            shares: 1000,
            value: 500_000.0,
            share_type: "SH".into(),
            discretion: "SO".into(),
            is_option: false,
            portfolio_weight: 0.05,
        };

        let json = serde_json::to_string(&holding).unwrap();
        let deserialized: Holding = serde_json::from_str(&json).unwrap();
        assert_eq!(holding.ticker, deserialized.ticker);
        assert_eq!(holding.sector, deserialized.sector);
    }

    #[test]
    fn integration_consensus_signal_serialization() {
        let signal = ConsensusSignal {
            ticker: "AAPL".into(),
            sector: Sector::Technology,
            holder_count: 10,
            avg_conviction: 2.5,
            net_direction: 0.8,
            dominant_change: PositionChange::Increased,
            aggregate_value_m: 1500.0,
        };

        let json = serde_json::to_string(&signal).unwrap();
        let deserialized: ConsensusSignal = serde_json::from_str(&json).unwrap();
        assert_eq!(signal.ticker, deserialized.ticker);
        assert_eq!(signal.holder_count, deserialized.holder_count);
        assert!((signal.net_direction - deserialized.net_direction).abs() < 1e-9);
    }
}
