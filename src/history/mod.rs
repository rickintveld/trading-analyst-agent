//! Historical aggregation. Each completed analysis appends a deterministic
//! snapshot to `<vault>/Assets/history.json`; rolling averages and deltas are
//! computed from those snapshots and embedded in the exchange JSON.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::ai::AiExchange;
use crate::shared::{round2, round4};

/// Number of most recent snapshots used for rolling averages.
pub const ROLLING_WINDOW: usize = 10;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryFile {
    pub schema_version: String,
    pub snapshots: Vec<Snapshot>,
}

/// Compact deterministic record of one analysis run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub generated_at: String,
    pub source_file: String,
    pub report_file: Option<String>,
    pub total_trades: usize,
    pub total_pnl: f64,
    pub win_rate: f64,
    pub profit_factor: Option<f64>,
    pub average_rr: Option<f64>,
    pub average_trade: f64,
    pub average_weekly_pnl: f64,
    pub average_monthly_pnl: f64,
    pub max_daily_drawdown: f64,
    pub discipline_score: f64,
    pub consistency_score: f64,
    pub emotional_control_score: f64,
    pub best_pair: Option<String>,
    pub best_pair_pnl: Option<f64>,
}

/// Differences between the current analysis and the previous snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deltas {
    pub total_pnl_change: f64,
    pub win_rate_change: f64,
    pub profit_factor_change: Option<f64>,
    pub average_rr_change: Option<f64>,
    pub average_trade_change: f64,
    pub max_daily_drawdown_change: f64,
    pub discipline_score_change: f64,
    pub consistency_score_change: f64,
    pub emotional_control_score_change: f64,
}

/// Rolling averages over the last [`ROLLING_WINDOW`] snapshots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollingAverages {
    pub window: usize,
    pub win_rate: f64,
    pub profit_factor: Option<f64>,
    pub average_rr: Option<f64>,
    pub average_weekly_pnl: f64,
    pub average_monthly_pnl: f64,
    pub discipline_score: f64,
    pub consistency_score: f64,
    pub emotional_control_score: f64,
    pub max_daily_drawdown: f64,
    pub average_trade: f64,
    pub best_pair_pnl: f64,
}

/// The `historical_comparison` section of the exchange JSON.
#[derive(Debug, Clone, Serialize)]
pub struct HistoricalComparison {
    /// Number of previous analyses on record (excluding the current one).
    pub previous_analyses: usize,
    pub previous: Option<Snapshot>,
    pub deltas: Option<Deltas>,
    pub rolling_averages: Option<RollingAverages>,
    /// All snapshots in chronological order, the current analysis last
    /// (schema 1.1.0). Feeds the historical development charts.
    pub series: Vec<Snapshot>,
}

impl HistoricalComparison {
    pub fn empty() -> Self {
        Self {
            previous_analyses: 0,
            previous: None,
            deltas: None,
            rolling_averages: None,
            series: Vec::new(),
        }
    }
}

pub fn history_path(vault: &Path, assets_dir: &str) -> PathBuf {
    vault.join(assets_dir).join("history.json")
}

pub fn load(path: &Path) -> Result<HistoryFile> {
    if !path.exists() {
        return Ok(HistoryFile {
            schema_version: crate::SCHEMA_VERSION.to_string(),
            snapshots: Vec::new(),
        });
    }
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read history file {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("invalid history file {}", path.display()))
}

pub fn save(path: &Path, history: &HistoryFile) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_string_pretty(history)?;
    std::fs::write(path, raw)
        .with_context(|| format!("cannot write history file {}", path.display()))
}

/// Build the deterministic snapshot for a completed analysis.
pub fn snapshot_from(exchange: &AiExchange, report_file: Option<String>) -> Snapshot {
    let best_pair = exchange.pair_statistics.best_pair.clone();
    let best_pair_pnl = best_pair.as_ref().and_then(|name| {
        exchange
            .pair_statistics
            .pairs
            .iter()
            .find(|p| &p.pair == name)
            .map(|p| p.total_pnl)
    });
    Snapshot {
        generated_at: exchange.metadata.generated_at.clone(),
        source_file: exchange.metadata.source_file.clone(),
        report_file,
        total_trades: exchange.performance.total_trades,
        total_pnl: exchange.performance.total_pnl,
        win_rate: exchange.performance.win_rate,
        profit_factor: exchange.performance.profit_factor,
        average_rr: exchange.performance.average_rr,
        average_trade: exchange.performance.average_trade,
        average_weekly_pnl: exchange.weekly_statistics.average_weekly_pnl,
        average_monthly_pnl: exchange.monthly_statistics.average_monthly_pnl,
        max_daily_drawdown: exchange.daily_statistics.max_daily_drawdown,
        discipline_score: exchange.scores.discipline.score,
        consistency_score: exchange.scores.consistency.score,
        emotional_control_score: exchange.scores.emotional_control.score,
        best_pair,
        best_pair_pnl,
    }
}

/// Compare a fresh snapshot against stored history (which must NOT yet
/// contain the fresh snapshot).
pub fn compare(current: &Snapshot, history: &HistoryFile) -> HistoricalComparison {
    let previous = history.snapshots.last().cloned();
    let deltas = previous.as_ref().map(|prev| Deltas {
        total_pnl_change: round2(current.total_pnl - prev.total_pnl),
        win_rate_change: round2(current.win_rate - prev.win_rate),
        profit_factor_change: match (current.profit_factor, prev.profit_factor) {
            (Some(a), Some(b)) => Some(round4(a - b)),
            _ => None,
        },
        average_rr_change: match (current.average_rr, prev.average_rr) {
            (Some(a), Some(b)) => Some(round4(a - b)),
            _ => None,
        },
        average_trade_change: round2(current.average_trade - prev.average_trade),
        max_daily_drawdown_change: round2(current.max_daily_drawdown - prev.max_daily_drawdown),
        discipline_score_change: round2(current.discipline_score - prev.discipline_score),
        consistency_score_change: round2(current.consistency_score - prev.consistency_score),
        emotional_control_score_change: round2(
            current.emotional_control_score - prev.emotional_control_score,
        ),
    });

    // Rolling window includes the current snapshot.
    let mut window: Vec<&Snapshot> = history.snapshots.iter().collect();
    window.push(current);
    let window: Vec<&Snapshot> = window.into_iter().rev().take(ROLLING_WINDOW).collect();
    let n = window.len() as f64;
    let avg = |f: fn(&Snapshot) -> f64| round2(window.iter().map(|s| f(s)).sum::<f64>() / n);
    let avg_opt = |f: fn(&Snapshot) -> Option<f64>| {
        let values: Vec<f64> = window.iter().filter_map(|s| f(s)).collect();
        if values.is_empty() {
            None
        } else {
            Some(round4(values.iter().sum::<f64>() / values.len() as f64))
        }
    };

    let rolling = RollingAverages {
        window: window.len(),
        win_rate: avg(|s| s.win_rate),
        profit_factor: avg_opt(|s| s.profit_factor),
        average_rr: avg_opt(|s| s.average_rr),
        average_weekly_pnl: avg(|s| s.average_weekly_pnl),
        average_monthly_pnl: avg(|s| s.average_monthly_pnl),
        discipline_score: avg(|s| s.discipline_score),
        consistency_score: avg(|s| s.consistency_score),
        emotional_control_score: avg(|s| s.emotional_control_score),
        max_daily_drawdown: avg(|s| s.max_daily_drawdown),
        average_trade: avg(|s| s.average_trade),
        best_pair_pnl: round2(
            window.iter().filter_map(|s| s.best_pair_pnl).sum::<f64>()
                / window
                    .iter()
                    .filter(|s| s.best_pair_pnl.is_some())
                    .count()
                    .max(1) as f64,
        ),
    };

    let mut series = history.snapshots.clone();
    series.push(current.clone());

    HistoricalComparison {
        previous_analyses: history.snapshots.len(),
        previous,
        deltas,
        rolling_averages: Some(rolling),
        series,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(pnl: f64, win_rate: f64) -> Snapshot {
        Snapshot {
            generated_at: "2026-01-01T10:00:00".to_string(),
            source_file: "trades.csv".to_string(),
            report_file: None,
            total_trades: 10,
            total_pnl: pnl,
            win_rate,
            profit_factor: Some(2.0),
            average_rr: Some(1.5),
            average_trade: pnl / 10.0,
            average_weekly_pnl: pnl / 2.0,
            average_monthly_pnl: pnl,
            max_daily_drawdown: 100.0,
            discipline_score: 70.0,
            consistency_score: 60.0,
            emotional_control_score: 80.0,
            best_pair: Some("EURUSD".to_string()),
            best_pair_pnl: Some(pnl * 0.6),
        }
    }

    #[test]
    fn first_analysis_has_no_previous() {
        let history = HistoryFile {
            schema_version: "1.0.0".into(),
            snapshots: vec![],
        };
        let cmp = compare(&snapshot(500.0, 55.0), &history);
        assert_eq!(cmp.previous_analyses, 0);
        assert!(cmp.previous.is_none());
        assert!(cmp.deltas.is_none());
        assert_eq!(cmp.rolling_averages.unwrap().window, 1);
    }

    #[test]
    fn deltas_against_previous() {
        let history = HistoryFile {
            schema_version: "1.0.0".into(),
            snapshots: vec![snapshot(500.0, 50.0)],
        };
        let cmp = compare(&snapshot(800.0, 60.0), &history);
        let deltas = cmp.deltas.unwrap();
        assert_eq!(deltas.total_pnl_change, 300.0);
        assert_eq!(deltas.win_rate_change, 10.0);
        assert_eq!(cmp.rolling_averages.unwrap().win_rate, 55.0);
    }

    #[test]
    fn rolling_window_caps_at_ten() {
        let history = HistoryFile {
            schema_version: "1.0.0".into(),
            snapshots: (0..15).map(|i| snapshot(100.0 * i as f64, 50.0)).collect(),
        };
        let cmp = compare(&snapshot(2000.0, 50.0), &history);
        assert_eq!(cmp.rolling_averages.unwrap().window, ROLLING_WINDOW);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join("trade-analyst-history-test");
        let path = dir.join("history.json");
        let _ = std::fs::remove_file(&path);
        let mut history = load(&path).unwrap();
        assert!(history.snapshots.is_empty());
        history.snapshots.push(snapshot(100.0, 50.0));
        save(&path, &history).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.snapshots.len(), 1);
        assert_eq!(loaded.snapshots[0].total_pnl, 100.0);
    }
}
