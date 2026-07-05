//! Deterministic 0-100 scoring. Every score exposes its weighted components
//! and a terse formula identifier so the AI layer can explain *why* a score
//! is what it is without recalculating anything.

use serde::Serialize;

use crate::ai::{DailyStatistics, WeeklyStatistics};
use crate::behavior::{find, BehavioralFlag};
use crate::config::ScoresConfig;
use crate::shared::{clamp01, mean, round2, round4, std_dev, Trade};

#[derive(Debug, Clone, Serialize)]
pub struct ScoreComponent {
    /// Stable machine name of the component.
    pub name: String,
    /// Normalized weight (all weights in a score sum to 1).
    pub weight: f64,
    /// Raw component value in 0..1 (1 = perfect).
    pub value: f64,
    /// weight * value * 100 — the points contributed to the score.
    pub contribution: f64,
    /// Terse formula identifier (machine-readable, not prose).
    pub formula: String,
    /// False when the underlying data was missing and a neutral 0.5 was used.
    pub data_available: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Score {
    /// 0-100.
    pub score: f64,
    pub components: Vec<ScoreComponent>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Scores {
    pub discipline: Score,
    pub consistency: Score,
    pub emotional_control: Score,
    /// Mean of the three scores, 0-100.
    pub overall: f64,
}

struct RawComponent {
    name: &'static str,
    weight: f64,
    value: f64,
    formula: &'static str,
    data_available: bool,
}

fn build_score(raw: Vec<RawComponent>) -> Score {
    let total_weight: f64 = raw.iter().map(|c| c.weight).sum();
    let total_weight = if total_weight <= 0.0 {
        1.0
    } else {
        total_weight
    };
    let components: Vec<ScoreComponent> = raw
        .into_iter()
        .map(|c| {
            let weight = c.weight / total_weight;
            let value = clamp01(c.value);
            ScoreComponent {
                name: c.name.to_string(),
                weight: round4(weight),
                value: round4(value),
                contribution: round2(weight * value * 100.0),
                formula: c.formula.to_string(),
                data_available: c.data_available,
            }
        })
        .collect();
    let score = components.iter().map(|c| c.weight * c.value).sum::<f64>() * 100.0;
    Score {
        score: round2(score),
        components,
    }
}

/// Coefficient-of-variation based consistency: 1 when perfectly steady,
/// approaching 0 as variation exceeds the mean.
fn cv_consistency(values: &[f64]) -> Option<f64> {
    let m = mean(values)?;
    if m.abs() <= f64::EPSILON {
        return None;
    }
    let sd = std_dev(values)?;
    Some(clamp01(1.0 - sd / m.abs()))
}

pub fn compute(
    trades: &[Trade],
    daily: &DailyStatistics,
    weekly: &WeeklyStatistics,
    flags: &[BehavioralFlag],
    cfg: &ScoresConfig,
) -> Scores {
    let discipline = discipline_score(trades, flags, cfg);
    let consistency = consistency_score(trades, daily, weekly, cfg);
    let emotional = emotional_control_score(trades, flags, cfg);
    let overall = round2((discipline.score + consistency.score + emotional.score) / 3.0);
    Scores {
        discipline,
        consistency,
        emotional_control: emotional,
        overall,
    }
}

fn flag_control(flags: &[BehavioralFlag], name: &str) -> (f64, bool) {
    match find(flags, name) {
        Some(f) if f.data_available => (f.severity.control_factor(), true),
        _ => (0.5, false), // neutral when the data is missing
    }
}

fn discipline_score(trades: &[Trade], flags: &[BehavioralFlag], cfg: &ScoresConfig) -> Score {
    let w = &cfg.discipline;

    // Risk sizing consistency: CV of risk (fallback: position size).
    let risks: Vec<f64> = trades.iter().filter_map(|t| t.risk).collect();
    let sizes: Vec<f64> = trades.iter().filter_map(|t| t.position_size).collect();
    let (risk_value, risk_available, risk_formula) = if risks.len() >= 2 {
        (cv_consistency(&risks).unwrap_or(0.5), true, "1 - cv(risk)")
    } else if sizes.len() >= 2 {
        (
            cv_consistency(&sizes).unwrap_or(0.5),
            true,
            "1 - cv(position_size)",
        )
    } else {
        (0.5, false, "neutral(no risk/size data)")
    };

    let (overtrading_value, overtrading_available) = flag_control(flags, "overtrading");
    let (violation_value, violation_available) = rule_adherence(flags);
    let (focus_value, focus_available) = strategy_focus(trades);

    build_score(vec![
        RawComponent {
            name: "risk_consistency",
            weight: w.risk_consistency,
            value: risk_value,
            formula: risk_formula,
            data_available: risk_available,
        },
        RawComponent {
            name: "overtrading_control",
            weight: w.overtrading_control,
            value: overtrading_value,
            formula: "severity_control(overtrading)",
            data_available: overtrading_available,
        },
        RawComponent {
            name: "rule_adherence",
            weight: w.rule_adherence,
            value: violation_value,
            formula: "1 - violation_rate",
            data_available: violation_available,
        },
        RawComponent {
            name: "strategy_focus",
            weight: w.strategy_focus,
            value: focus_value,
            formula: "1 - clamp((distinct_strategies - 1) / 5)",
            data_available: focus_available,
        },
    ])
}

fn rule_adherence(flags: &[BehavioralFlag]) -> (f64, bool) {
    match find(flags, "consecutive_rule_violations") {
        Some(f) if f.data_available => {
            let rate = f
                .metrics
                .get("violation_rate")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            (clamp01(1.0 - rate * 2.0), true)
        }
        _ => (0.5, false),
    }
}

fn strategy_focus(trades: &[Trade]) -> (f64, bool) {
    let strategies: std::collections::HashSet<&String> =
        trades.iter().filter_map(|t| t.strategy.as_ref()).collect();
    if strategies.is_empty() {
        (0.5, false)
    } else {
        let distinct = strategies.len() as f64;
        (clamp01(1.0 - (distinct - 1.0) / 5.0), true)
    }
}

fn consistency_score(
    trades: &[Trade],
    daily: &DailyStatistics,
    weekly: &WeeklyStatistics,
    cfg: &ScoresConfig,
) -> Score {
    let w = &cfg.consistency;

    let decided_days = daily.positive_days + daily.negative_days;
    let (positive_ratio, days_available) = if decided_days > 0 {
        (daily.positive_days as f64 / decided_days as f64, true)
    } else {
        (0.5, false)
    };

    // Weekly PnL stability: penalize std dev vs. mean absolute weekly PnL.
    let weekly_pnls: Vec<f64> = weekly.weeks.iter().map(|w| w.pnl).collect();
    let (weekly_value, weekly_available) = if weekly_pnls.len() >= 2 {
        let sd = std_dev(&weekly_pnls).unwrap_or(0.0);
        let mean_abs = weekly_pnls.iter().map(|p| p.abs()).sum::<f64>() / weekly_pnls.len() as f64;
        if mean_abs > 0.0 {
            (clamp01(1.0 - sd / (2.0 * mean_abs)), true)
        } else {
            (0.5, false)
        }
    } else {
        (0.5, false)
    };

    // PnL concentration: how much of gross profit came from the single best trade.
    let wins: Vec<f64> = trades
        .iter()
        .filter(|t| t.is_win())
        .map(|t| t.pnl)
        .collect();
    let gross: f64 = wins.iter().sum();
    let (concentration_value, concentration_available) = if gross > 0.0 && wins.len() >= 2 {
        let top = wins.iter().copied().fold(0.0f64, f64::max);
        (clamp01(1.0 - top / gross), true)
    } else {
        (0.5, false)
    };

    // Win-rate stability across weeks.
    let week_rates: Vec<f64> = weekly.weeks.iter().map(|w| w.win_rate).collect();
    let (winrate_value, winrate_available) = if week_rates.len() >= 2 {
        let sd = std_dev(&week_rates).unwrap_or(0.0);
        (clamp01(1.0 - sd / 50.0), true)
    } else {
        (0.5, false)
    };

    build_score(vec![
        RawComponent {
            name: "positive_day_ratio",
            weight: w.positive_day_ratio,
            value: positive_ratio,
            formula: "positive_days / (positive_days + negative_days)",
            data_available: days_available,
        },
        RawComponent {
            name: "weekly_stability",
            weight: w.weekly_stability,
            value: weekly_value,
            formula: "1 - std(weekly_pnl) / (2 * mean|weekly_pnl|)",
            data_available: weekly_available,
        },
        RawComponent {
            name: "pnl_concentration",
            weight: w.pnl_concentration,
            value: concentration_value,
            formula: "1 - largest_win / gross_profit",
            data_available: concentration_available,
        },
        RawComponent {
            name: "win_rate_stability",
            weight: w.win_rate_stability,
            value: winrate_value,
            formula: "1 - std(weekly_win_rate) / 50",
            data_available: winrate_available,
        },
    ])
}

fn emotional_control_score(
    trades: &[Trade],
    flags: &[BehavioralFlag],
    cfg: &ScoresConfig,
) -> Score {
    let w = &cfg.emotional_control;

    let (revenge_value, revenge_available) = flag_control(flags, "revenge_trading");
    let (tilt_value, tilt_available) = flag_control(flags, "tilt");
    let (panic_value, panic_available) = flag_control(flags, "panic_exits");

    // Size reaction after losses: 1 when sizing stays flat, 0 when doubled.
    let sizes: Vec<f64> = trades.iter().filter_map(|t| t.position_size).collect();
    let (loss_reaction_value, loss_reaction_available) = if sizes.len() >= 2 {
        let overall = mean(&sizes).unwrap_or(0.0);
        let mut after_loss = Vec::new();
        for pair in trades.windows(2) {
            if pair[0].is_loss() {
                if let Some(s) = pair[1].position_size {
                    after_loss.push(s);
                }
            }
        }
        if overall > 0.0 && !after_loss.is_empty() {
            let ratio = mean(&after_loss).unwrap_or(overall) / overall;
            (clamp01(1.0 - (ratio - 1.0).max(0.0)), true)
        } else {
            (0.5, false)
        }
    } else {
        (0.5, false)
    };

    build_score(vec![
        RawComponent {
            name: "revenge_control",
            weight: w.revenge_control,
            value: revenge_value,
            formula: "severity_control(revenge_trading)",
            data_available: revenge_available,
        },
        RawComponent {
            name: "tilt_control",
            weight: w.tilt_control,
            value: tilt_value,
            formula: "severity_control(tilt)",
            data_available: tilt_available,
        },
        RawComponent {
            name: "panic_exit_control",
            weight: w.panic_exit_control,
            value: panic_value,
            formula: "severity_control(panic_exits)",
            data_available: panic_available,
        },
        RawComponent {
            name: "loss_reaction",
            weight: w.loss_reaction,
            value: loss_reaction_value,
            formula: "1 - max(0, avg_size_after_loss / avg_size - 1)",
            data_available: loss_reaction_available,
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behavior;
    use crate::config::{BehaviorConfig, Config};
    use crate::statistics;
    use chrono::NaiveDate;

    fn trade(day: u32, pnl: f64, risk: Option<f64>) -> Trade {
        Trade {
            index: 0,
            date: NaiveDate::from_ymd_opt(2026, 1, day).unwrap(),
            time: None,
            symbol: "EURUSD".to_string(),
            direction: None,
            entry: None,
            exit: None,
            position_size: None,
            risk,
            reward: None,
            rr: None,
            pnl,
            fees: None,
            strategy: Some("Breakout".to_string()),
            session: None,
            notes: None,
        }
    }

    fn score_for(trades: &[Trade]) -> Scores {
        let cfg = Config::default();
        let daily = statistics::daily_statistics(trades);
        let weekly = statistics::weekly_statistics(trades);
        let flags = behavior::detect(trades, &BehaviorConfig::default());
        compute(trades, &daily, &weekly, &flags, &cfg.scores)
    }

    #[test]
    fn scores_are_bounded() {
        let trades: Vec<Trade> = (1..=20)
            .map(|d| trade(d, if d % 3 == 0 { -50.0 } else { 80.0 }, Some(100.0)))
            .collect();
        let scores = score_for(&trades);
        for s in [
            &scores.discipline,
            &scores.consistency,
            &scores.emotional_control,
        ] {
            assert!(
                s.score >= 0.0 && s.score <= 100.0,
                "score out of range: {}",
                s.score
            );
            let weight_sum: f64 = s.components.iter().map(|c| c.weight).sum();
            assert!(
                (weight_sum - 1.0).abs() < 0.01,
                "weights not normalized: {weight_sum}"
            );
        }
        assert!(scores.overall >= 0.0 && scores.overall <= 100.0);
    }

    #[test]
    fn steady_risk_scores_higher_than_erratic() {
        let steady: Vec<Trade> = (1..=10).map(|d| trade(d, 50.0, Some(100.0))).collect();
        let erratic: Vec<Trade> = (1..=10)
            .map(|d| trade(d, 50.0, Some(if d % 2 == 0 { 400.0 } else { 50.0 })))
            .collect();
        let s1 = score_for(&steady);
        let s2 = score_for(&erratic);
        assert!(
            s1.discipline.score > s2.discipline.score,
            "steady {} should beat erratic {}",
            s1.discipline.score,
            s2.discipline.score
        );
    }

    #[test]
    fn empty_input_stays_neutral() {
        let scores = score_for(&[]);
        assert!(scores.overall >= 0.0 && scores.overall <= 100.0);
    }
}
