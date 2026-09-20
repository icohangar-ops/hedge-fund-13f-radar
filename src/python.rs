//! pyo3 bindings for the 13F radar engine.
//!
//! Compiled only with the `python` feature (built by maturin; see
//! pyproject.toml). The Python surface mirrors the Rust public API —
//! `FilingBuilder`, `QuarterlyAggregator`, `Pipeline`, `diff_filings` and the
//! ingest helpers — with data crossing the boundary as plain dicts/lists that
//! use the same field names as the Rust structs.
//!
//! Input types (`Filing13F`, `Holding`, `Manager`) implement serde's
//! `Deserialize`, so Python dicts are decoded straight into engine types.
//! Analysis outputs that lack serde derives are converted to dicts here via
//! the `*_json` helpers — keep those in sync with the Rust structs.
//!
//! Scope note: `ConsensusEngine` and `SectorRotationEngine` are orchestrated
//! through `Pipeline`, whose `PipelineResult` already exposes their complete
//! outputs (consensus signals, high-conviction tickers, sector momentum,
//! rotation metrics, heat map, whale moves, pair trades, clusters).

use std::collections::HashMap;

use pyo3::exceptions::{PyKeyError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict, PyModule};

use crate::consensus::{
    ConvictionCluster, SectorConsensus, TickerConsensus, WhaleMove, WhaleTracker,
};
use crate::diff::{DiffResult, DiffSummary, PositionDiff};
use crate::ingest::{FilingBuilder, QuarterlyAggregator, RawHolding};
use crate::pipeline::{Pipeline, PipelineConfig, PipelineResult};
use crate::sector::{PairTradeSignal, RotationHeatMap, SectorMomentum};
use crate::types::{ConvictionLevel, CusipMap, Filing13F, Manager, PositionChange, Sector};

// ---------------------------------------------------------------------------
// serde <-> Python helpers
// ---------------------------------------------------------------------------

fn to_py<T: serde::Serialize>(py: Python<'_>, value: &T) -> PyResult<Py<PyAny>> {
    pythonize::pythonize(py, value)
        .map(|bound| bound.unbind())
        .map_err(|e| PyValueError::new_err(e.to_string()))
}

fn from_py<T: serde::de::DeserializeOwned>(obj: &Bound<'_, PyAny>) -> PyResult<T> {
    pythonize::depythonize(obj).map_err(|e| PyValueError::new_err(e.to_string()))
}

fn filings_from_py(obj: &Bound<'_, PyAny>) -> PyResult<Vec<Filing13F>> {
    obj.try_iter()?
        .map(|item| from_py::<Filing13F>(&item?))
        .collect()
}

fn py_err<T, E: std::fmt::Display>(result: Result<T, E>) -> PyResult<T> {
    result.map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Optional key lookup on a mapping — missing key and `None` both mean absent.
fn opt_item<'a>(obj: &'a Bound<'_, PyAny>, key: &str) -> PyResult<Option<Bound<'a, PyAny>>> {
    match obj.get_item(key) {
        Ok(value) => Ok(if value.is_none() { None } else { Some(value) }),
        Err(e) if e.is_instance_of::<PyKeyError>(obj.py()) => Ok(None),
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------------------
// Enum names — serde variant names, so Python strings round-trip with the
// same representation serde_json produces for the Rust enums.
// ---------------------------------------------------------------------------

fn sector_name(sector: &Sector) -> &'static str {
    match sector {
        Sector::Technology => "Technology",
        Sector::Healthcare => "Healthcare",
        Sector::Finance => "Finance",
        Sector::ConsumerDiscretionary => "ConsumerDiscretionary",
        Sector::ConsumerStaples => "ConsumerStaples",
        Sector::Energy => "Energy",
        Sector::Industrials => "Industrials",
        Sector::Materials => "Materials",
        Sector::RealEstate => "RealEstate",
        Sector::Utilities => "Utilities",
        Sector::CommunicationServices => "CommunicationServices",
        Sector::Unknown => "Unknown",
    }
}

fn position_change_name(change: &PositionChange) -> &'static str {
    match change {
        PositionChange::New => "New",
        PositionChange::Increased => "Increased",
        PositionChange::Unchanged => "Unchanged",
        PositionChange::Decreased => "Decreased",
        PositionChange::Exited => "Exited",
    }
}

fn conviction_name(level: &ConvictionLevel) -> &'static str {
    match level {
        ConvictionLevel::Low => "Low",
        ConvictionLevel::Medium => "Medium",
        ConvictionLevel::High => "High",
        ConvictionLevel::VeryHigh => "VeryHigh",
    }
}

// ---------------------------------------------------------------------------
// JSON converters for result types that do not derive Serialize
// ---------------------------------------------------------------------------

fn position_diff_json(d: &PositionDiff) -> serde_json::Value {
    serde_json::json!({
        "ticker": d.ticker,
        "sector": sector_name(&d.sector),
        "change": position_change_name(&d.change),
        "prior_shares": d.prior_shares,
        "current_shares": d.current_shares,
        "delta_shares": d.delta_shares,
        "prior_value": d.prior_value,
        "current_value": d.current_value,
        "delta_value": d.delta_value,
        "pct_change_shares": d.pct_change_shares,
        "portfolio_weight": d.portfolio_weight,
        "conviction": conviction_name(&d.conviction),
        "prior_conviction": d.prior_conviction.map(|c| conviction_name(&c)),
        "is_new": d.is_new(),
        "is_exit": d.is_exit(),
        "is_conviction_upgrade": d.is_conviction_upgrade(),
        "is_conviction_downgrade": d.is_conviction_downgrade(),
        "portfolio_impact": d.portfolio_impact(),
    })
}

fn diff_summary_json(s: &DiffSummary) -> serde_json::Value {
    serde_json::json!({
        "new_count": s.new_count,
        "increased_count": s.increased_count,
        "unchanged_count": s.unchanged_count,
        "decreased_count": s.decreased_count,
        "exited_count": s.exited_count,
        "total_positions_current": s.total_positions_current,
        "total_positions_prior": s.total_positions_prior,
        "turnover_rate": s.turnover_rate,
        "net_delta_value": s.net_delta_value,
        "largest_new_value": s.largest_new_value,
        "largest_exit_value": s.largest_exit_value,
    })
}

fn diff_result_json(r: &DiffResult) -> serde_json::Value {
    serde_json::json!({
        "manager_cik": r.manager_cik,
        "manager_name": r.manager_name,
        "diffs": r.diffs.iter().map(position_diff_json).collect::<Vec<_>>(),
        "summary": diff_summary_json(&r.summary),
    })
}

fn ticker_consensus_json(t: &TickerConsensus) -> serde_json::Value {
    serde_json::json!({
        "ticker": t.ticker,
        "sector": sector_name(&t.sector),
        "holder_count": t.holder_count,
        "conviction_score_sum": t.conviction_score_sum,
        "avg_conviction": t.avg_conviction,
        "increased": t.increased,
        "decreased": t.decreased,
        "new_count": t.new_count,
        "exited": t.exited,
        "unchanged": t.unchanged,
        "total_value_m": t.total_value_m,
        "direction_score": t.direction_score,
        "direction_ratio": t.direction_ratio(),
        "dominant_change": position_change_name(&t.dominant_change()),
        "diffs": t.diffs.iter().map(position_diff_json).collect::<Vec<_>>(),
    })
}

fn sector_consensus_json(s: &SectorConsensus) -> serde_json::Value {
    serde_json::json!({
        "sector": sector_name(&s.sector),
        "holder_count": s.holder_count,
        "avg_weight": s.avg_weight,
        "inflows": s.inflows,
        "outflows": s.outflows,
        "net_direction": s.net_direction,
        "total_value_m": s.total_value_m,
    })
}

fn sector_consensus_map_json(map: &HashMap<Sector, SectorConsensus>) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    for (sector, value) in map {
        out.insert(
            sector_name(sector).to_string(),
            sector_consensus_json(value),
        );
    }
    serde_json::Value::Object(out)
}

fn whale_move_json(m: &WhaleMove) -> serde_json::Value {
    serde_json::json!({
        "manager_name": m.manager_name,
        "manager_cik": m.manager_cik,
        "ticker": m.ticker,
        "sector": sector_name(&m.sector),
        "change": position_change_name(&m.change),
        "delta_value_m": m.delta_value_m,
        "weight": m.weight,
        "conviction": conviction_name(&m.conviction),
    })
}

fn whale_tracker_json(w: &WhaleTracker) -> serde_json::Value {
    serde_json::json!({
        "threshold_m": w.threshold_m,
        "moves": w.moves.iter().map(whale_move_json).collect::<Vec<_>>(),
        "bullish_moves": w.bullish_moves().iter().map(|m| whale_move_json(m)).collect::<Vec<_>>(),
        "bearish_moves": w.bearish_moves().iter().map(|m| whale_move_json(m)).collect::<Vec<_>>(),
    })
}

fn heat_map_json(h: &RotationHeatMap) -> serde_json::Value {
    let net: serde_json::Map<String, serde_json::Value> = h
        .net_sector_flows()
        .iter()
        .map(|(sector, flow)| (sector_name(sector).to_string(), serde_json::json!(flow)))
        .collect();
    serde_json::json!({
        "sectors": h.sectors.iter().map(sector_name).collect::<Vec<_>>(),
        "matrix": h.matrix,
        "net_flows": net,
    })
}

fn sector_momentum_json(m: &SectorMomentum) -> serde_json::Value {
    serde_json::json!({
        "sector": sector_name(&m.sector),
        "score": m.score,
        "bulls": m.bulls,
        "bears": m.bears,
        "avg_weight_change": m.avg_weight_change,
        "net_aum_change_m": m.net_aum_change_m,
    })
}

fn pair_trade_json(p: &PairTradeSignal) -> serde_json::Value {
    serde_json::json!({
        "from_sector": sector_name(&p.from_sector),
        "to_sector": sector_name(&p.to_sector),
        "divergence_score": p.divergence_score,
        "momentum_spread": p.momentum_spread,
        "rotating_funds": p.rotating_funds,
    })
}

fn cluster_json(c: &ConvictionCluster) -> serde_json::Value {
    let mut centroid = serde_json::Map::new();
    for (sector, weight) in &c.centroid {
        centroid.insert(sector_name(sector).to_string(), serde_json::json!(weight));
    }
    serde_json::json!({
        "centroid": centroid,
        "members": c.members,
        "cohesion": c.cohesion,
    })
}

// ---------------------------------------------------------------------------
// CusipMap
// ---------------------------------------------------------------------------

#[pyclass(name = "CusipMap")]
struct PyCusipMap {
    inner: CusipMap,
}

#[pymethods]
impl PyCusipMap {
    #[new]
    fn new() -> Self {
        Self {
            inner: CusipMap::new(),
        }
    }

    fn insert(&mut self, cusip: &str, ticker: &str, name: &str) {
        self.inner.insert(cusip, ticker, name);
    }

    /// Lookup a CUSIP, returning (ticker, name) or None.
    fn get(&self, cusip: &str) -> Option<(String, String)> {
        self.inner
            .get(cusip)
            .map(|(t, n)| (t.to_string(), n.to_string()))
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

/// Accept a `CusipMap` instance or a `{cusip: (ticker, name)}` dict.
fn cusip_map_from_py(obj: &Bound<'_, PyAny>) -> PyResult<CusipMap> {
    if let Ok(wrapper) = obj.cast::<PyCusipMap>() {
        return Ok(wrapper.borrow().inner.clone());
    }

    let dict = obj.cast::<PyDict>().map_err(|_| {
        PyValueError::new_err("cusip_map must be a CusipMap or a dict of {cusip: (ticker, name)}")
    })?;

    let mut map = CusipMap::new();
    for (key, value) in dict.iter() {
        let cusip: String = key
            .extract()
            .map_err(|_| PyKeyError::new_err("cusip map keys must be strings"))?;
        let (ticker, name): (String, String) = value
            .extract()
            .map_err(|_| PyValueError::new_err("cusip map values must be (ticker, name) tuples"))?;
        map.insert(&cusip, &ticker, &name);
    }
    Ok(map)
}

// ---------------------------------------------------------------------------
// FilingBuilder
// ---------------------------------------------------------------------------

#[pyclass(name = "FilingBuilder")]
struct PyFilingBuilder {
    inner: Option<FilingBuilder>,
}

fn builder_mut(slot: &mut Option<FilingBuilder>) -> PyResult<&mut FilingBuilder> {
    slot.as_mut()
        .ok_or_else(|| PyValueError::new_err("FilingBuilder was already built"))
}

#[pymethods]
impl PyFilingBuilder {
    /// `FilingBuilder(manager_dict, report_date)` — manager is
    /// `{"cik": ..., "name": ..., "filer_type": ...}`, date a `datetime.date`.
    #[new]
    fn new(manager: &Bound<'_, PyAny>, report_date: chrono::NaiveDate) -> PyResult<Self> {
        let manager: Manager = from_py(manager)?;
        Ok(Self {
            inner: Some(FilingBuilder::new(manager, report_date)),
        })
    }

    fn accession(&mut self, accession: &str) -> PyResult<()> {
        builder_mut(&mut self.inner)?.accession(accession);
        Ok(())
    }

    fn filing_date(&mut self, filing_date: chrono::NaiveDate) -> PyResult<()> {
        builder_mut(&mut self.inner)?.filing_date(filing_date);
        Ok(())
    }

    fn total_aum(&mut self, total_aum: f64) -> PyResult<()> {
        builder_mut(&mut self.inner)?.total_aum(total_aum);
        Ok(())
    }

    fn other_included(&mut self, other_included: usize) -> PyResult<()> {
        builder_mut(&mut self.inner)?.other_included(other_included);
        Ok(())
    }

    fn add_xml_holding(&mut self, xml: &str) -> PyResult<()> {
        py_err(builder_mut(&mut self.inner)?.add_xml_holding(xml))
    }

    fn add_csv_holding(&mut self, row: &str) -> PyResult<()> {
        py_err(builder_mut(&mut self.inner)?.add_csv_holding(row))
    }

    /// Build the normalized `Filing13F` dict. Consumes the builder.
    fn build(&mut self, py: Python<'_>, cusip_map: &PyCusipMap) -> PyResult<Py<PyAny>> {
        let builder = self
            .inner
            .take()
            .ok_or_else(|| PyValueError::new_err("FilingBuilder was already built"))?;
        let filing = py_err(builder.build(&cusip_map.inner))?;
        to_py(py, &filing)
    }
}

// ---------------------------------------------------------------------------
// QuarterlyAggregator
// ---------------------------------------------------------------------------

#[pyclass(name = "QuarterlyAggregator")]
struct PyQuarterlyAggregator {
    inner: QuarterlyAggregator,
}

#[pymethods]
impl PyQuarterlyAggregator {
    #[new]
    fn new() -> Self {
        Self {
            inner: QuarterlyAggregator::new(),
        }
    }

    fn add(&mut self, filing: &Bound<'_, PyAny>) -> PyResult<()> {
        self.inner.add(from_py::<Filing13F>(filing)?);
        Ok(())
    }

    /// Filings grouped by quarter label: `{quarter: [filing, ...]}`.
    fn by_quarter(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let out = PyDict::new(py);
        for (quarter, filings) in self.inner.by_quarter() {
            out.set_item(quarter, to_py(py, &filings)?)?;
        }
        Ok(out.into_any().unbind())
    }

    fn latest_quarter(&self) -> Option<String> {
        self.inner.latest_quarter()
    }

    fn filings_for_quarter(&self, py: Python<'_>, quarter: &str) -> PyResult<Py<PyAny>> {
        to_py(py, &self.inner.filings_for_quarter(quarter))
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Pipeline + PipelineResult
// ---------------------------------------------------------------------------

#[pyclass(name = "Pipeline")]
struct PyPipeline {
    inner: Pipeline,
}

#[pymethods]
impl PyPipeline {
    #[new]
    fn new() -> Self {
        Self {
            inner: Pipeline::new(),
        }
    }

    /// The default configuration as a dict (all keys accepted by `with_config`).
    #[staticmethod]
    fn default_config(py: Python<'_>) -> PyResult<Py<PyAny>> {
        let config = PipelineConfig::default();
        to_py(
            py,
            &serde_json::json!({
                "min_conviction_holders": config.min_conviction_holders,
                "min_avg_conviction": config.min_avg_conviction,
                "whale_threshold_m": config.whale_threshold_m,
                "pair_trade_min_spread": config.pair_trade_min_spread,
                "pair_trade_min_rotating": config.pair_trade_min_rotating,
                "cluster_similarity": config.cluster_similarity,
            }),
        )
    }

    /// Build a pipeline from a config dict. Every key is optional; `cusip_map`
    /// accepts a `CusipMap` or a `{cusip: (ticker, name)}` dict.
    #[staticmethod]
    fn with_config(config: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut cfg = PipelineConfig::default();

        if let Some(value) = opt_item(config, "cusip_map")? {
            cfg.cusip_map = cusip_map_from_py(&value)?;
        }
        if let Some(value) = opt_item(config, "min_conviction_holders")? {
            cfg.min_conviction_holders = value.extract()?;
        }
        if let Some(value) = opt_item(config, "min_avg_conviction")? {
            cfg.min_avg_conviction = value.extract()?;
        }
        if let Some(value) = opt_item(config, "whale_threshold_m")? {
            cfg.whale_threshold_m = value.extract()?;
        }
        if let Some(value) = opt_item(config, "pair_trade_min_spread")? {
            cfg.pair_trade_min_spread = value.extract()?;
        }
        if let Some(value) = opt_item(config, "pair_trade_min_rotating")? {
            cfg.pair_trade_min_rotating = value.extract()?;
        }
        if let Some(value) = opt_item(config, "cluster_similarity")? {
            cfg.cluster_similarity = value.extract()?;
        }

        Ok(Self {
            inner: Pipeline::with_config(cfg),
        })
    }

    /// Run the full pipeline on prior and current quarter filings
    /// (lists of filing dicts).
    fn run(
        &self,
        prior_filings: &Bound<'_, PyAny>,
        current_filings: &Bound<'_, PyAny>,
        prior_quarter: &str,
        current_quarter: &str,
    ) -> PyResult<PyPipelineResult> {
        let prior = filings_from_py(prior_filings)?;
        let current = filings_from_py(current_filings)?;
        let result = py_err(
            self.inner
                .run(&prior, &current, prior_quarter, current_quarter),
        )?;
        Ok(PyPipelineResult { inner: result })
    }

    /// Run the pipeline from a `QuarterlyAggregator`.
    fn run_from_aggregator(
        &self,
        aggregator: &PyQuarterlyAggregator,
        prior_quarter: &str,
        current_quarter: &str,
    ) -> PyResult<PyPipelineResult> {
        let result = py_err(self.inner.run_from_aggregator(
            &aggregator.inner,
            prior_quarter,
            current_quarter,
        ))?;
        Ok(PyPipelineResult { inner: result })
    }
}

#[pyclass(name = "PipelineResult")]
struct PyPipelineResult {
    inner: PipelineResult,
}

#[pymethods]
impl PyPipelineResult {
    #[getter]
    fn prior_quarter(&self) -> &str {
        &self.inner.prior_quarter
    }

    #[getter]
    fn current_quarter(&self) -> &str {
        &self.inner.current_quarter
    }

    /// Per-manager position diffs.
    #[getter]
    fn diffs(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let value: Vec<serde_json::Value> = self.inner.diffs.iter().map(diff_result_json).collect();
        to_py(py, &value)
    }

    /// Per-ticker consensus signals.
    #[getter]
    fn consensus_signals(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_py(py, &self.inner.consensus_signals)
    }

    /// High-conviction tickers.
    #[getter]
    fn high_conviction(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let value: Vec<serde_json::Value> = self
            .inner
            .high_conviction
            .iter()
            .map(ticker_consensus_json)
            .collect();
        to_py(py, &value)
    }

    /// Sector-level consensus, keyed by sector name.
    #[getter]
    fn sector_consensus(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_py(py, &sector_consensus_map_json(&self.inner.sector_consensus))
    }

    /// Sector rotation metrics.
    #[getter]
    fn rotation_metrics(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_py(py, &self.inner.rotation_metrics)
    }

    /// Sector momentum rankings.
    #[getter]
    fn sector_momentum(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let value: Vec<serde_json::Value> = self
            .inner
            .sector_momentum
            .iter()
            .map(sector_momentum_json)
            .collect();
        to_py(py, &value)
    }

    /// Rotation heat map (sectors, flow matrix, net flows).
    #[getter]
    fn heat_map(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_py(py, &heat_map_json(&self.inner.heat_map))
    }

    /// Whale moves for the quarter.
    #[getter]
    fn whale_moves(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_py(py, &whale_tracker_json(&self.inner.whale_moves))
    }

    /// Pair trade signals.
    #[getter]
    fn pair_trades(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let value: Vec<serde_json::Value> =
            self.inner.pair_trades.iter().map(pair_trade_json).collect();
        to_py(py, &value)
    }

    /// Conviction clusters.
    #[getter]
    fn clusters(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let value: Vec<serde_json::Value> = self.inner.clusters.iter().map(cluster_json).collect();
        to_py(py, &value)
    }

    /// Validation warnings.
    #[getter]
    fn warnings(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_py(py, &self.inner.warnings)
    }

    /// Text summary report generated by the engine.
    fn summary_report(&self) -> String {
        self.inner.summary_report()
    }

    /// Top-N bullish consensus tickers (by net direction).
    fn top_bullish_consensus(&self, py: Python<'_>, n: usize) -> PyResult<Py<PyAny>> {
        to_py(py, &self.inner.top_bullish_consensus(n))
    }

    /// Top-N bearish consensus tickers (by net direction).
    fn top_bearish_consensus(&self, py: Python<'_>, n: usize) -> PyResult<Py<PyAny>> {
        to_py(py, &self.inner.top_bearish_consensus(n))
    }

    /// Sectors with the strongest inflow momentum.
    fn top_inflow_sectors(&self, py: Python<'_>, n: usize) -> PyResult<Py<PyAny>> {
        let value: Vec<serde_json::Value> = self
            .inner
            .top_inflow_sectors(n)
            .into_iter()
            .map(sector_momentum_json)
            .collect();
        to_py(py, &value)
    }

    /// Sectors with the strongest outflow momentum.
    fn top_outflow_sectors(&self, py: Python<'_>, n: usize) -> PyResult<Py<PyAny>> {
        let value: Vec<serde_json::Value> = self
            .inner
            .top_outflow_sectors(n)
            .into_iter()
            .map(sector_momentum_json)
            .collect();
        to_py(py, &value)
    }

    /// The complete result as a plain dict.
    fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let out = PyDict::new(py);
        out.set_item("prior_quarter", &self.inner.prior_quarter)?;
        out.set_item("current_quarter", &self.inner.current_quarter)?;
        out.set_item(
            "diffs",
            to_py(
                py,
                &self
                    .inner
                    .diffs
                    .iter()
                    .map(diff_result_json)
                    .collect::<Vec<_>>(),
            )?,
        )?;
        out.set_item(
            "consensus_signals",
            to_py(py, &self.inner.consensus_signals)?,
        )?;
        out.set_item(
            "high_conviction",
            to_py(
                py,
                &self
                    .inner
                    .high_conviction
                    .iter()
                    .map(ticker_consensus_json)
                    .collect::<Vec<_>>(),
            )?,
        )?;
        out.set_item(
            "sector_consensus",
            to_py(py, &sector_consensus_map_json(&self.inner.sector_consensus))?,
        )?;
        out.set_item("rotation_metrics", to_py(py, &self.inner.rotation_metrics)?)?;
        out.set_item(
            "sector_momentum",
            to_py(
                py,
                &self
                    .inner
                    .sector_momentum
                    .iter()
                    .map(sector_momentum_json)
                    .collect::<Vec<_>>(),
            )?,
        )?;
        out.set_item("heat_map", to_py(py, &heat_map_json(&self.inner.heat_map))?)?;
        out.set_item(
            "whale_moves",
            to_py(py, &whale_tracker_json(&self.inner.whale_moves))?,
        )?;
        out.set_item(
            "pair_trades",
            to_py(
                py,
                &self
                    .inner
                    .pair_trades
                    .iter()
                    .map(pair_trade_json)
                    .collect::<Vec<_>>(),
            )?,
        )?;
        out.set_item(
            "clusters",
            to_py(
                py,
                &self
                    .inner
                    .clusters
                    .iter()
                    .map(cluster_json)
                    .collect::<Vec<_>>(),
            )?,
        )?;
        out.set_item("warnings", to_py(py, &self.inner.warnings)?)?;
        Ok(out.into_any().unbind())
    }

    fn __str__(&self) -> String {
        self.inner.summary_report()
    }
}

// ---------------------------------------------------------------------------
// Free functions — module-level ingest/diff utilities
// ---------------------------------------------------------------------------

fn raw_holding_json(raw: &RawHolding) -> serde_json::Value {
    serde_json::json!({
        "cusip": raw.cusip,
        "ticker": raw.ticker,
        "name": raw.name,
        "shares": raw.shares,
        "value": raw.value,
        "share_type": raw.share_type,
        "discretion": raw.discretion,
        "is_option": raw.is_option,
    })
}

/// Parse a single XML information-table row into a raw holding dict.
#[pyfunction]
fn parse_xml_holding(py: Python<'_>, xml: &str) -> PyResult<Py<PyAny>> {
    let raw = py_err(crate::ingest::parse_xml_holding(xml))?;
    to_py(py, &raw_holding_json(&raw))
}

/// Parse a CSV row into a raw holding dict.
/// Columns: cusip,ticker,name,shares,value,share_type,discretion,is_option
#[pyfunction]
fn parse_csv_holding(py: Python<'_>, row: &str) -> PyResult<Py<PyAny>> {
    let raw = py_err(crate::ingest::parse_csv_holding(row))?;
    to_py(py, &raw_holding_json(&raw))
}

/// Normalize a raw holding dict into a `Holding` dict using a CUSIP map.
#[pyfunction]
fn normalize_holding(
    py: Python<'_>,
    raw: &Bound<'_, PyAny>,
    cusip_map: &PyCusipMap,
    total_aum: f64,
) -> PyResult<Py<PyAny>> {
    let cusip: String = match opt_item(raw, "cusip")? {
        Some(value) => value.extract()?,
        None => return Err(PyValueError::new_err("Missing required field: cusip")),
    };

    let raw_holding = RawHolding {
        cusip,
        ticker: match opt_item(raw, "ticker")? {
            Some(value) => value.extract()?,
            None => None,
        },
        name: match opt_item(raw, "name")? {
            Some(value) => value.extract()?,
            None => None,
        },
        shares: match opt_item(raw, "shares")? {
            Some(value) => value.extract()?,
            None => None,
        },
        value: match opt_item(raw, "value")? {
            Some(value) => value.extract()?,
            None => None,
        },
        share_type: match opt_item(raw, "share_type")? {
            Some(value) => value.extract()?,
            None => None,
        },
        discretion: match opt_item(raw, "discretion")? {
            Some(value) => value.extract()?,
            None => None,
        },
        is_option: match opt_item(raw, "is_option")? {
            Some(value) => value.extract()?,
            None => false,
        },
    };

    let holding = py_err(crate::ingest::normalize_holding(
        &raw_holding,
        &cusip_map.inner,
        total_aum,
    ))?;
    to_py(py, &holding)
}

/// Validate a filing dict; returns a list of warning strings.
#[pyfunction]
fn validate_filing(py: Python<'_>, filing: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let parsed: Filing13F = from_py(filing)?;
    let warnings = py_err(crate::ingest::validate_filing(&parsed))?;
    to_py(py, &warnings)
}

/// Compare two filings; returns the full diff result dict.
#[pyfunction]
fn diff_filings(
    py: Python<'_>,
    prior: &Bound<'_, PyAny>,
    current: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let prior: Filing13F = from_py(prior)?;
    let current: Filing13F = from_py(current)?;
    to_py(
        py,
        &diff_result_json(&crate::diff::diff_filings(&prior, &current)),
    )
}

/// Parse a "YYYY-MM-DD" date string.
#[pyfunction]
fn parse_date(raw: &str) -> PyResult<chrono::NaiveDate> {
    py_err(crate::ingest::parse_date(raw))
}

/// Classify a ticker into a sector name.
#[pyfunction]
fn classify_ticker(ticker: &str) -> &'static str {
    sector_name(&Sector::classify_ticker(ticker))
}

/// Conviction level for a fraction of portfolio weight.
#[pyfunction]
fn conviction_from_portfolio_weight(weight: f64) -> &'static str {
    conviction_name(&ConvictionLevel::from_portfolio_weight(weight))
}

// ---------------------------------------------------------------------------
// Module
// ---------------------------------------------------------------------------

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyCusipMap>()?;
    module.add_class::<PyFilingBuilder>()?;
    module.add_class::<PyQuarterlyAggregator>()?;
    module.add_class::<PyPipeline>()?;
    module.add_class::<PyPipelineResult>()?;
    module.add_function(wrap_pyfunction!(parse_xml_holding, module)?)?;
    module.add_function(wrap_pyfunction!(parse_csv_holding, module)?)?;
    module.add_function(wrap_pyfunction!(normalize_holding, module)?)?;
    module.add_function(wrap_pyfunction!(validate_filing, module)?)?;
    module.add_function(wrap_pyfunction!(diff_filings, module)?)?;
    module.add_function(wrap_pyfunction!(parse_date, module)?)?;
    module.add_function(wrap_pyfunction!(classify_ticker, module)?)?;
    module.add_function(wrap_pyfunction!(conviction_from_portfolio_weight, module)?)?;
    Ok(())
}
