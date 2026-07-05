//! The fixed JSON exchange format — the permanent contract between the
//! deterministic Rust engine and any AI model.
//!
//! Rules of the contract:
//! - always exactly the same top-level keys, in the same order
//! - snake_case field names, no dynamic key names
//! - machine-readable values only; no prose explanations
//! - `null` (Option) for anything that cannot be computed from the input
//! - backwards compatible: only additive changes without a major
//!   [`crate::SCHEMA_VERSION`] bump

use serde::{Deserialize, Serialize};

use crate::behavior::BehavioralFlags;
use crate::history::HistoricalComparison;
use crate::scoring::Scores;

/// Root document handed to the AI layer. Field order here defines the JSON
/// key order (serde_json preserves struct order).
#[derive(Debug, Clone, Serialize)]
pub struct AiExchange {
    pub metadata: Metadata,
    pub trading_period: TradingPeriod,
    pub performance: Performance,
    pub daily_statistics: DailyStatistics,
    pub weekly_statistics: WeeklyStatistics,
    pub monthly_statistics: MonthlyStatistics,
    pub pair_statistics: PairStatistics,
    pub strategy_statistics: StrategyStatistics,
    pub session_statistics: SessionStatistics,
    pub streaks: Streaks,
    pub scores: Scores,
    pub behavioral_flags: BehavioralFlags,
    pub historical_comparison: HistoricalComparison,
    pub summary: Summary,
    /// Added in schema 1.1.0 (appended last to keep prior key order stable).
    pub time_series: TimeSeries,
}

#[derive(Debug, Clone, Serialize)]
pub struct Metadata {
    pub schema_version: String,
    pub engine_version: String,
    /// ISO-8601 local timestamp of generation.
    pub generated_at: String,
    pub source_file: String,
    pub timezone: String,
    pub total_rows: usize,
    pub parsed_trades: usize,
    pub skipped_rows: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct TradingPeriod {
    /// ISO date of the first trade.
    pub first_trade_date: String,
    /// ISO date of the last trade.
    pub last_trade_date: String,
    /// Inclusive calendar days between first and last trade.
    pub total_calendar_days: i64,
    /// Distinct dates with at least one trade.
    pub total_trading_days: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Performance {
    pub total_trades: usize,
    pub winning_trades: usize,
    pub losing_trades: usize,
    pub breakeven_trades: usize,
    pub total_pnl: f64,
    pub gross_profit: f64,
    /// Negative number (sum of losing trades).
    pub gross_loss: f64,
    /// total_pnl minus total fees (equal to total_pnl when no fee data).
    pub net_profit: f64,
    pub total_fees: f64,
    /// Percentage 0-100.
    pub win_rate: f64,
    /// Percentage 0-100.
    pub loss_rate: f64,
    pub average_win: Option<f64>,
    /// Negative number.
    pub average_loss: Option<f64>,
    /// gross_profit / |gross_loss|; null when there are no losses.
    pub profit_factor: Option<f64>,
    /// Expected value per trade: p(win)*avg_win + p(loss)*avg_loss.
    pub expectancy: Option<f64>,
    /// Mean realized RR over trades that report it; null without RR data.
    pub average_rr: Option<f64>,
    /// total_pnl / total_trades.
    pub average_trade: f64,
    pub largest_win: Option<f64>,
    pub largest_loss: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DayPnl {
    /// ISO date.
    pub date: String,
    pub pnl: f64,
    pub trades: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct DailyStatistics {
    pub trading_days: usize,
    pub positive_days: usize,
    pub negative_days: usize,
    pub flat_days: usize,
    pub average_daily_pnl: f64,
    pub best_day: Option<DayPnl>,
    pub worst_day: Option<DayPnl>,
    /// Largest peak-to-trough decline of the daily cumulative equity curve,
    /// expressed as a positive magnitude.
    pub max_daily_drawdown: f64,
    pub average_trades_per_day: f64,
    pub max_trades_in_one_day: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct WeekPnl {
    /// ISO week, e.g. "2026-W07".
    pub week: String,
    pub pnl: f64,
    pub trades: usize,
    /// Percentage 0-100 for that week.
    pub win_rate: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct WeeklyStatistics {
    pub weeks: Vec<WeekPnl>,
    pub average_weekly_pnl: f64,
    pub best_week: Option<WeekPnl>,
    pub worst_week: Option<WeekPnl>,
    pub winning_weeks: usize,
    pub losing_weeks: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct MonthPnl {
    /// Month, e.g. "2026-02".
    pub month: String,
    pub pnl: f64,
    pub trades: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct MonthlyStatistics {
    pub months: Vec<MonthPnl>,
    pub average_monthly_pnl: f64,
    pub best_month: Option<MonthPnl>,
    pub worst_month: Option<MonthPnl>,
    /// "improving" | "declining" | "flat" | "insufficient_data"
    /// (least-squares slope of monthly PnL vs. 2% of mean |monthly PnL|).
    pub trend: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PairStats {
    pub pair: String,
    pub total_trades: usize,
    /// Percentage 0-100.
    pub win_rate: f64,
    pub average_rr: Option<f64>,
    pub total_pnl: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PairStatistics {
    /// Sorted by total_pnl descending.
    pub pairs: Vec<PairStats>,
    pub best_pair: Option<String>,
    pub worst_pair: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StrategyStats {
    pub strategy: String,
    pub total_trades: usize,
    pub win_rate: f64,
    pub average_rr: Option<f64>,
    pub total_pnl: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct StrategyStatistics {
    pub data_available: bool,
    /// Sorted by total_pnl descending; empty without strategy data.
    pub strategies: Vec<StrategyStats>,
    pub best_strategy: Option<String>,
    pub worst_strategy: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionStats {
    /// "asia" | "london" | "overlap" | "new_york" | "other"
    pub session: String,
    pub total_trades: usize,
    pub win_rate: f64,
    pub total_pnl: f64,
    pub average_pnl: f64,
    pub average_rr: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionStatistics {
    /// False when neither a session column nor trade times exist.
    pub data_available: bool,
    /// Always all five sessions, fixed order, zero-filled when unused.
    pub sessions: Vec<SessionStats>,
    pub best_session: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Streaks {
    /// Trade-level streaks (chronological).
    pub longest_winning_streak_trades: usize,
    pub longest_losing_streak_trades: usize,
    /// Signed: positive = current winning streak, negative = losing.
    pub current_streak_trades: i64,
    /// Day-level streaks over daily PnL.
    pub longest_winning_streak_days: usize,
    pub longest_losing_streak_days: usize,
    /// Signed: positive = current run of green days, negative = red days.
    pub current_streak_days: i64,
}

/// Compact repetition of the key figures for cheap prompting.
/// Machine-readable only — the AI writes the human summary.
#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub total_trades: usize,
    pub total_pnl: f64,
    pub win_rate: f64,
    pub profit_factor: Option<f64>,
    pub expectancy: Option<f64>,
    pub average_rr: Option<f64>,
    pub max_daily_drawdown: f64,
    pub best_pair: Option<String>,
    pub worst_pair: Option<String>,
    pub best_strategy: Option<String>,
    pub best_session: Option<String>,
    pub discipline_score: f64,
    pub consistency_score: f64,
    pub emotional_control_score: f64,
    pub overall_score: f64,
    /// Names of behavioral flags with detected = true.
    pub detected_behaviors: Vec<String>,
    pub monthly_trend: String,
}

/// Per-trade and per-day chart datasets (schema 1.1.0). These feed every
/// time-based visualization; the reporting layer never recomputes them.
#[derive(Debug, Clone, Serialize)]
pub struct TimeSeries {
    /// True when the source exceeded `max_points` and was stride-sampled.
    pub downsampled: bool,
    pub max_points: usize,
    /// Chronological per-trade curve points.
    pub trades: Vec<TradePoint>,
    /// Chronological per-day PnL.
    pub daily: Vec<DayPnl>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TradePoint {
    /// 1-based trade number (original numbering survives downsampling).
    pub n: usize,
    /// ISO date of the trade.
    pub date: String,
    pub pnl: f64,
    pub cumulative_pnl: f64,
    /// Distance below the running equity peak (positive magnitude).
    pub drawdown: f64,
    pub risk: Option<f64>,
    pub rr: Option<f64>,
    /// Rolling mean of realized RR over the last 10 RR-reporting trades.
    pub rolling_rr: Option<f64>,
    /// Signed running streak after this trade (+3 = third win in a row).
    pub streak: i64,
}

/// Kept `Deserialize`-able so future tooling can read stored exchanges.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredExchangeRef {
    pub file: String,
    pub generated_at: String,
}
