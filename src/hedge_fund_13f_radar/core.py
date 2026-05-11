"""13F holdings analysis primitives."""
from __future__ import annotations

import csv
import json
from collections import Counter, defaultdict
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Dict, List


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


@dataclass
class VerificationGate:
    status: str
    confidence: int
    violations: List[str] = field(default_factory=list)


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
        return VerificationGate("REQUIRES_HUMAN_VERIFICATION", 50, violations)
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
    confidence = 100 if not violations else max(50, 100 - 10 * len(violations))
    return VerificationGate(
        status="CLEAR" if confidence == 100 else "REQUIRES_HUMAN_VERIFICATION",
        confidence=confidence,
        violations=violations,
    )


def report_json(report: RadarReport) -> str:
    return json.dumps(report.to_dict(), indent=2)

