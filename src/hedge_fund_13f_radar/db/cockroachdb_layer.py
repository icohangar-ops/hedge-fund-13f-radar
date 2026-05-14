# Hedge Fund 13F Radar — CockroachDB Persistence Layer
"""
SQLAlchemy ORM for distributed storage of 13F holdings, position signals,
and radar reports across multiple hedge fund managers.
"""
from __future__ import annotations
import os, logging
from sqlalchemy import create_engine, Column, String, Integer, Numeric, DateTime, Text, Index, JSON, func, select, desc
from sqlalchemy.orm import declarative_base, relationship, Session, sessionmaker
from sqlalchemy.dialects.postgresql import JSONB

logger = logging.getLogger("hedge_fund_13f_radar.db")

COCKROACH_URL = "cockroachdb+psycopg2://REDACTED@vortex-giraffe-15678.jxf.gcp-us-east1.cockroachlabs.cloud:26257/hedge_fund_13f_radar?sslmode=require"
DATABASE_URL = os.getenv("HFR_DATABASE_URL", COCKROACH_URL)
engine = create_engine(DATABASE_URL, pool_size=8, max_overflow=4, pool_timeout=30, pool_pre_ping=True)
SessionLocal = sessionmaker(bind=engine, autoflush=False)

def get_session() -> Session: return SessionLocal()

Base = declarative_base()

class TimestampMixin:
    created_at = Column(DateTime(timezone=True), server_default=func.now())
    updated_at = Column(DateTime(timezone=True), server_default=func.now(), onupdate=func.now())


class FundManagerModel(TimestampMixin, Base):
    __tablename__ = "fund_managers"
    manager_id = Column(String, primary_key=True, server_default=func.gen_random_uuid())
    name = Column(String, nullable=False, unique=True)
    aum_usd = Column(Numeric(18, 2), default=0)
    style = Column(String, default="")  # value, growth, macro, quant
    filing_count = Column(Integer, default=0)
    holdings = relationship("HoldingModel", back_populates="manager_rel", cascade="all, delete-orphan")


class HoldingModel(TimestampMixin, Base):
    __tablename__ = "holdings"
    holding_id = Column(String, primary_key=True, server_default=func.gen_random_uuid())
    manager_id = Column(String, nullable=False, index=True)
    quarter = Column(String, nullable=False, index=True)  # e.g. "2025-Q4"
    ticker = Column(String, nullable=False, index=True)
    company = Column(String, default="")
    sector = Column(String, default="", index=True)
    shares = Column(Numeric(18, 2), default=0)
    prior_shares = Column(Numeric(18, 2), default=0)
    value_usd = Column(Numeric(18, 2), default=0)
    weight_pct = Column(Numeric(8, 4), default=0)
    action = Column(String, default="HELD")  # INITIATED, EXITED, INCREASED, DECREASED, HELD
    conviction = Column(String, default="LOW")  # HIGH, MEDIUM, LOW
    source_url = Column(Text, default="")
    manager_rel = relationship("FundManagerModel", back_populates="holdings")

    __table_args__ = (Index("ix_holdings_quarter_ticker", "quarter", "ticker"),)


class PositionSignalModel(TimestampMixin, Base):
    __tablename__ = "position_signals"
    signal_id = Column(String, primary_key=True, server_default=func.gen_random_uuid())
    quarter = Column(String, nullable=False, index=True)
    manager = Column(String, default="")
    ticker = Column(String, nullable=False, index=True)
    company = Column(String, default="")
    sector = Column(String, default="")
    value_usd = Column(Numeric(18, 2), default=0)
    weight = Column(Numeric(8, 4), default=0)
    action = Column(String, default="")
    conviction = Column(String, default="")


class RadarReportModel(TimestampMixin, Base):
    __tablename__ = "radar_reports"
    report_id = Column(String, primary_key=True, server_default=func.gen_random_uuid())
    quarter = Column(String, nullable=False, index=True)
    consensus_tickers = Column(JSONB, default=[])
    sector_rotation = Column(JSONB, default={})
    verification_status = Column(String, default="")
    verification_confidence = Column(Integer, default=0)
    verification_violations = Column(JSONB, default=[])
    total_managers = Column(Integer, default=0)
    total_holdings = Column(Integer, default=0)
    report_json = Column(JSONB, default={})
    report_markdown = Column(Text, default="")


class ConsensusTickerModel(TimestampMixin, Base):
    __tablename__ = "consensus_tickers"
    id = Column(String, primary_key=True, server_default=func.gen_random_uuid())
    quarter = Column(String, nullable=False, index=True)
    ticker = Column(String, nullable=False, index=True)
    holder_count = Column(Integer, default=0)
    total_value_usd = Column(Numeric(18, 2), default=0)
    holders = Column(JSONB, default=[])

    __table_args__ = (Index("ix_consensus_quarter_count", "quarter", "holder_count"),)


# Repositories
class HoldingRepository:
    @staticmethod
    def get_by_quarter(session: Session, quarter: str) -> list[dict]:
        rows = session.execute(
            select(HoldingModel).where(HoldingModel.quarter == quarter).order_by(desc(HoldingModel.value_usd))
        ).scalars().all()
        return [{"ticker": r.ticker, "company": r.company, "manager": r.manager_id, "shares": float(r.shares), "value_usd": float(r.value_usd), "action": r.action, "conviction": r.conviction} for r in rows]

    @staticmethod
    def get_sector_summary(session: Session, quarter: str) -> list[dict]:
        rows = session.execute(
            select(HoldingModel.sector, func.count().label("positions"), func.sum(HoldingModel.value_usd).label("total_value")).where(HoldingModel.quarter == quarter).group_by(HoldingModel.sector).order_by(desc("total_value"))
        ).all()
        return [{"sector": r.sector, "positions": r.positions, "total_value_usd": float(r.total_value or 0)} for r in rows]

    @staticmethod
    def get_top_signals(session: Session, quarter: str, limit: int = 20) -> list[dict]:
        rows = session.execute(
            select(PositionSignalModel).where(PositionSignalModel.quarter == quarter, PositionSignalModel.conviction == "HIGH").order_by(desc(PositionSignalModel.weight)).limit(limit)
        ).scalars().all()
        return [{"manager": r.manager, "ticker": r.ticker, "action": r.action, "weight": float(r.weight), "conviction": r.conviction} for r in rows]


def health_check() -> dict:
    session = get_session()
    try:
        row = session.execute(func.current_timestamp()).scalar()
        managers = session.execute(select(func.count()).select_from(FundManagerModel)).scalar()
        holdings = session.execute(select(func.count()).select_from(HoldingModel)).scalar()
        reports = session.execute(select(func.count()).select_from(RadarReportModel)).scalar()
        return {"status": "ok", "connected": True, "server_time": str(row), "managers": managers, "holdings": holdings, "reports": reports, "backend": "CockroachDB"}
    except Exception as e:
        return {"status": "error", "connected": False, "error": str(e)}
    finally:
        session.close()

def create_tables():
    Base.metadata.create_all(bind=engine)
    logger.info("All tables created successfully")

if __name__ == "__main__":
    create_tables()
    print(health_check())
