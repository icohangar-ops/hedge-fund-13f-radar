"""13F holdings analysis primitives."""
from __future__ import annotations

import csv
import json
import logging
import uuid
from collections import Counter, defaultdict
from dataclasses import asdict, dataclass
from decimal import Decimal
from pathlib import Path
from typing import Dict, List

from cubiczan_resilience.verification_gate import (
    CONFIDENCE_FLOOR,
    REQUIRES_HUMAN_VERIFICATION,
    VerificationGate,
    build_gate,
)

__all__ = ["VerificationGate", "RadarReport", "analyze_13f"]

logger = logging.getLogger(__name__)


REQUIRED_COLUMNS = {
    "manager",
    "quarter",
    "ticker",
    "company",
    "sector",
    "shares",
    "prior_shares",
    "value_usd",
    "source_url",
}


@dataclass
class PositionSignal:
    manager: str
    ticker: str
    company: str
    sector: str
    value_usd: float
    weight: float
    action: str
    conviction: str


# VerificationGate moved to the canonical cubiczan_resilience.verification_gate
# module (row 29): one deterministic confidence rule for the whole portfolio.
# This module re-exports it for backwards-compatible imports; construct gates
# with build_gate().


@dataclass
class RadarReport:
    quarter: str
    consensus_tickers: List[str]
    sector_rotation: Dict[str, float]
    position_signals: List[PositionSignal]
    verification: VerificationGate

    def to_dict(self) -> Dict[str, object]:
        return asdict(self)

    def to_markdown(self) -> str:
        lines = [
            "# Hedge Fund 13F Radar",
            f"- Quarter: {self.quarter}",
            f"- Verification: {self.verification.status}",
            "",
            "## Consensus Tickers",
        ]
        lines.extend(f"- {ticker}" for ticker in self.consensus_tickers or ["NONE"])
        lines.append("")
        lines.append("## Top Position Signals")
        for signal in self.position_signals[:10]:
            lines.append(f"- {signal.manager} {signal.action} {signal.ticker}: {signal.weight:.1%} weight, {signal.conviction}")
        if self.verification.violations:
            lines.append("")
            lines.append("## Blocking Verification Issues")
            lines.extend(f"- {item}" for item in self.verification.violations)
        return "\n".join(lines)


def analyze_13f(holdings_path: str | Path) -> RadarReport:
    rows = _read_csv(holdings_path)
    quarter = rows[0].get("quarter", "UNKNOWN") if rows else "UNKNOWN"
    totals = defaultdict(float)
    for row in rows:
        totals[row["manager"]] += float(row["value_usd"])

    signals = [_position_signal(row, totals[row["manager"]]) for row in rows]
    consensus = _consensus_tickers(rows)
    sector_rotation = _sector_rotation(rows)
    verification = _verify(rows)
    return RadarReport(
        quarter=quarter,
        consensus_tickers=consensus,
        sector_rotation=sector_rotation,
        position_signals=sorted(signals, key=lambda item: (item.conviction != "HIGH", -item.weight)),
        verification=verification,
    )


def _read_csv(path: str | Path) -> List[Dict[str, str]]:
    with Path(path).open(newline="", encoding="utf-8") as handle:
        return list(csv.DictReader(handle))


def _position_signal(row: Dict[str, str], manager_total: float) -> PositionSignal:
    shares = float(row["shares"])
    prior = float(row["prior_shares"])
    value = float(row["value_usd"])
    weight = value / manager_total if manager_total else 0
    if prior == 0 and shares > 0:
        action = "INITIATED"
    elif shares == 0 and prior > 0:
        action = "EXITED"
    elif shares > prior * 1.15:
        action = "INCREASED"
    elif shares < prior * 0.85:
        action = "DECREASED"
    else:
        action = "HELD"
    conviction = "HIGH" if weight >= 0.12 or action == "INITIATED" and weight >= 0.08 else "MEDIUM" if weight >= 0.05 else "LOW"
    return PositionSignal(
        manager=row["manager"],
        ticker=row["ticker"],
        company=row["company"],
        sector=row["sector"],
        value_usd=value,
        weight=weight,
        action=action,
        conviction=conviction,
    )


def _consensus_tickers(rows: List[Dict[str, str]]) -> List[str]:
    holders: Dict[str, set[str]] = defaultdict(set)
    for row in rows:
        if float(row["shares"]) > 0:
            holders[row["ticker"]].add(row["manager"])
    count = Counter({ticker: len(managers) for ticker, managers in holders.items()})
    return [ticker for ticker, managers in count.most_common() if managers >= 2]


def _sector_rotation(rows: List[Dict[str, str]]) -> Dict[str, float]:
    rotation = defaultdict(float)
    for row in rows:
        value = float(row["value_usd"])
        shares = float(row["shares"])
        prior = float(row["prior_shares"])
        direction = 1 if shares > prior else -1 if shares < prior else 0
        rotation[row["sector"]] += direction * value
    return dict(sorted(rotation.items(), key=lambda item: abs(item[1]), reverse=True))


def _verify(rows: List[Dict[str, str]]) -> VerificationGate:
    violations: List[str] = []
    if not rows:
        violations.append("holdings file is empty")
        # DELIBERATE divergence from the canonical rule (reversible): an empty
        # holdings file is categorically worse than one minor gap, but
        # build_gate's equal-weight arithmetic scores one violation 88. We pin
        # this case at the canonical CONFIDENCE_FLOOR — the floor stays
        # canonical; only the severity override is local. Migration condition:
        # when cubiczan_resilience.verification_gate.build_gate grows a
        # severity-hint parameter, delete this override and call build_gate
        # directly. Keep in sync with earnings-call-nlp-lab's identical
        # override and the canonical module's docstring.
        return VerificationGate(
            status=REQUIRES_HUMAN_VERIFICATION,
            confidence=CONFIDENCE_FLOOR,
            violations=violations,
        )
    missing = REQUIRED_COLUMNS - set(rows[0].keys())
    if missing:
        violations.append(f"missing columns: {', '.join(sorted(missing))}")
    if len({row.get("manager") for row in rows}) < 2:
        violations.append("cross-fund analysis requires at least two managers")
    for idx, row in enumerate(rows, start=2):
        if not row.get("source_url"):
            violations.append(f"row {idx} missing source_url")
        for field in ("shares", "prior_shares", "value_usd"):
            try:
                float(row.get(field, ""))
            except ValueError:
                violations.append(f"row {idx} has invalid numeric field: {field}")
    # Canonical arithmetic (row 29): confidence and status come from the
    # shared rule — PENALTY_PER_VIOLATION=12 per violation, CONFIDENCE_FLOOR=50.
    # This repo previously used -10; confidences for failing gates drop
    # accordingly (1 violation: 90 -> 88; that drift is what row 29 ends).
    # The empty-file case above is the one deliberate local override (pinned
    # at the floor); everything else is pure canonical.
    return build_gate(violations)


def report_json(report: RadarReport) -> str:
    return json.dumps(report.to_dict(), indent=2)


def store_report(report: RadarReport) -> dict:
    """Persist a RadarReport and all associated data to CockroachDB."""
    try:
        from hedge_fund_13f_radar.db.cockroachdb_layer import (
            get_session, FundManagerModel, HoldingModel, PositionSignalModel,
            RadarReportModel, ConsensusTickerModel,
        )
        from sqlalchemy import select, func
    except ImportError:
        logger.warning("CockroachDB layer not available — skipping store")
        return {"stored": False, "reason": "cockroachdb_layer not importable"}

    session = get_session()
    try:
        report_id = str(uuid.uuid4())

        # Deduplicate managers from signals
        managers_seen: Dict[str, Dict] = {}
        for sig in report.position_signals:
            mgr_name = sig.manager
            if mgr_name not in managers_seen:
                managers_seen[mgr_name] = {"name": mgr_name, "aum": 0.0, "style": "", "holdings_count": 0}
            managers_seen[mgr_name]["aum"] += sig.value_usd
            managers_seen[mgr_name]["holdings_count"] += 1

        for mgr_data in managers_seen.values():
            existing = session.execute(
                select(FundManagerModel).where(FundManagerModel.name == mgr_data["name"])
            ).scalars().first()
            if not existing:
                session.add(FundManagerModel(
                    manager_id=str(uuid.uuid4()),
                    name=mgr_data["name"],
                    aum_usd=Decimal(str(round(mgr_data["aum"], 2))),
                    style="",
                    filing_count=1,
                ))

        # Store holdings and signals
        for sig in report.position_signals:
            session.add(HoldingModel(
                holding_id=str(uuid.uuid4()),
                manager_id=sig.manager,
                quarter=report.quarter,
                ticker=sig.ticker,
                company=sig.company,
                sector=sig.sector,
                shares=Decimal("0"),
                prior_shares=Decimal("0"),
                value_usd=Decimal(str(round(sig.value_usd, 2))),
                weight_pct=Decimal(str(round(sig.weight * 100, 4))),
                action=sig.action,
                conviction=sig.conviction,
            ))
            session.add(PositionSignalModel(
                signal_id=str(uuid.uuid4()),
                quarter=report.quarter,
                manager=sig.manager,
                ticker=sig.ticker,
                company=sig.company,
                sector=sig.sector,
                value_usd=Decimal(str(round(sig.value_usd, 2))),
                weight=Decimal(str(round(sig.weight, 4))),
                action=sig.action,
                conviction=sig.conviction,
            ))

        # Consensus tickers
        for ticker in (report.consensus_tickers or []):
            session.add(ConsensusTickerModel(
                quarter=report.quarter,
                ticker=ticker,
                holder_count=0,
            ))

        # Radar report
        session.add(RadarReportModel(
            report_id=report_id,
            quarter=report.quarter,
            consensus_tickers=report.consensus_tickers or [],
            sector_rotation=report.sector_rotation or {},
            verification_status=report.verification.status,
            verification_confidence=report.verification.confidence,
            verification_violations=report.verification.violations or [],
            total_managers=len(managers_seen),
            total_holdings=len(report.position_signals),
            report_json=report.to_dict(),
            report_markdown=report.to_markdown(),
        ))

        session.commit()
        logger.info("Stored radar report %s for quarter %s (%d holdings)", report_id, report.quarter, len(report.position_signals))
        return {"stored": True, "report_id": report_id, "quarter": report.quarter, "holdings": len(report.position_signals)}
    except Exception as e:
        session.rollback()
        logger.error("Failed to store radar report: %s", e)
        return {"stored": False, "error": str(e)}
    finally:
        session.close()

