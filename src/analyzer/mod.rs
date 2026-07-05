//! Orchestration: normalized trades -> complete [`AiExchange`] document.

use anyhow::{bail, Result};
use std::path::Path;

use crate::ai::{AiExchange, Metadata, Summary};
use crate::behavior;
use crate::config::Config;
use crate::history::{self, HistoricalComparison};
use crate::parser::ParseReport;
use crate::scoring;
use crate::shared::sort_chronologically;
use crate::statistics;

/// Build the full exchange document from a parse report.
///
/// `historical` lets the caller inject a comparison against stored history;
/// pass [`HistoricalComparison::empty`] when no vault history should be read.
pub fn build_exchange(
    report: &ParseReport,
    source_file: &Path,
    cfg: &Config,
    generated_at: String,
    historical: Option<HistoricalComparison>,
) -> Result<AiExchange> {
    if report.trades.is_empty() {
        bail!("no valid trades parsed from {}", source_file.display());
    }
    let mut trades = report.trades.clone();
    sort_chronologically(&mut trades);

    let trading_period = statistics::trading_period(&trades);
    let performance = statistics::performance(&trades);
    let daily_statistics = statistics::daily_statistics(&trades);
    let weekly_statistics = statistics::weekly_statistics(&trades);
    let monthly_statistics = statistics::monthly_statistics(&trades);
    let pair_statistics = statistics::pair_statistics(&trades);
    let strategy_statistics = statistics::strategy_statistics(&trades);
    let session_statistics = statistics::session_statistics(&trades, &cfg.sessions);
    let streaks = statistics::streaks(&trades);
    let time_series = statistics::time_series(&trades);
    let behavioral_flags = behavior::detect(&trades, &cfg.behavior);
    let scores = scoring::compute(
        &trades,
        &daily_statistics,
        &weekly_statistics,
        &behavioral_flags,
        &cfg.scores,
    );

    let detected_behaviors: Vec<String> = behavioral_flags
        .iter()
        .filter(|f| f.detected)
        .map(|f| f.flag.clone())
        .collect();

    let summary = Summary {
        total_trades: performance.total_trades,
        total_pnl: performance.total_pnl,
        win_rate: performance.win_rate,
        profit_factor: performance.profit_factor,
        expectancy: performance.expectancy,
        average_rr: performance.average_rr,
        max_daily_drawdown: daily_statistics.max_daily_drawdown,
        best_pair: pair_statistics.best_pair.clone(),
        worst_pair: pair_statistics.worst_pair.clone(),
        best_strategy: strategy_statistics.best_strategy.clone(),
        best_session: session_statistics.best_session.clone(),
        discipline_score: scores.discipline.score,
        consistency_score: scores.consistency.score,
        emotional_control_score: scores.emotional_control.score,
        overall_score: scores.overall,
        detected_behaviors,
        monthly_trend: monthly_statistics.trend.clone(),
    };

    Ok(AiExchange {
        metadata: Metadata {
            schema_version: crate::SCHEMA_VERSION.to_string(),
            engine_version: crate::ENGINE_VERSION.to_string(),
            generated_at,
            source_file: source_file.display().to_string(),
            timezone: cfg.general.timezone.clone(),
            total_rows: report.trades.len() + report.skipped_rows,
            parsed_trades: report.trades.len(),
            skipped_rows: report.skipped_rows,
        },
        trading_period,
        performance,
        daily_statistics,
        weekly_statistics,
        monthly_statistics,
        pair_statistics,
        strategy_statistics,
        session_statistics,
        streaks,
        scores,
        behavioral_flags,
        historical_comparison: historical.unwrap_or_else(HistoricalComparison::empty),
        summary,
        time_series,
    })
}

/// Load vault history and produce the comparison for a fresh exchange.
pub fn compare_against_vault_history(
    exchange: &AiExchange,
    cfg: &Config,
) -> Result<HistoricalComparison> {
    let path = history::history_path(&cfg.vault.path, &cfg.vault.assets_dir);
    let stored = history::load(&path)?;
    let snapshot = history::snapshot_from(exchange, None);
    Ok(history::compare(&snapshot, &stored))
}

/// Append the completed analysis to vault history and return the updated
/// history (used to rebuild the historical dashboard).
pub fn append_to_history(
    exchange: &AiExchange,
    report_file: Option<String>,
    cfg: &Config,
) -> Result<history::HistoryFile> {
    let path = history::history_path(&cfg.vault.path, &cfg.vault.assets_dir);
    let mut stored = history::load(&path)?;
    stored
        .snapshots
        .push(history::snapshot_from(exchange, report_file));
    history::save(&path, &stored)?;
    Ok(stored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{DetectedFormat, ParseReport};
    use crate::shared::Trade;
    use chrono::NaiveDate;

    fn parse_report(trades: Vec<Trade>) -> ParseReport {
        ParseReport {
            trades,
            warnings: vec![],
            skipped_rows: 0,
            detected: DetectedFormat {
                delimiter: ',',
                encoding: "utf-8",
                date_format: "%Y-%m-%d".to_string(),
                decimal_separator: '.',
                mapped_headers: vec![],
                unmapped_headers: vec![],
            },
        }
    }

    fn trade(day: u32, pnl: f64) -> Trade {
        Trade {
            index: 0,
            date: NaiveDate::from_ymd_opt(2026, 1, day).unwrap(),
            time: None,
            symbol: "EURUSD".to_string(),
            direction: None,
            entry: None,
            exit: None,
            position_size: None,
            risk: None,
            reward: None,
            rr: None,
            pnl,
            fees: None,
            strategy: None,
            session: None,
            notes: None,
        }
    }

    #[test]
    fn exchange_has_all_contract_keys() {
        let report = parse_report(vec![trade(1, 100.0), trade(2, -50.0)]);
        let cfg = Config::default();
        let exchange = build_exchange(
            &report,
            Path::new("trades.csv"),
            &cfg,
            "2026-01-05T10:00:00".to_string(),
            None,
        )
        .unwrap();
        let value = serde_json::to_value(&exchange).unwrap();
        let obj = value.as_object().unwrap();
        let expected = [
            "metadata",
            "trading_period",
            "performance",
            "daily_statistics",
            "weekly_statistics",
            "monthly_statistics",
            "pair_statistics",
            "strategy_statistics",
            "session_statistics",
            "streaks",
            "scores",
            "behavioral_flags",
            "historical_comparison",
            "summary",
            "time_series",
        ];
        assert_eq!(obj.len(), expected.len());
        // preserve_order keeps struct order — the contract includes key order.
        for (actual, expected) in obj.keys().zip(expected.iter()) {
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn empty_input_fails_cleanly() {
        let report = parse_report(vec![]);
        let cfg = Config::default();
        let result = build_exchange(&report, Path::new("x.csv"), &cfg, "t".to_string(), None);
        assert!(result.is_err());
    }
}
