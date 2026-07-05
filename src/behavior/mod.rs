//! Rule-based behavioral pattern detection. Pure heuristics over trade data;
//! no AI involved. Output is machine-readable flags (booleans, severities,
//! confidences, supporting metrics). The AI layer interprets them.

use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::config::BehaviorConfig;
use crate::shared::{mean, median, round4, Trade};
use crate::statistics::daily_pnl;

/// Fixed set of detectable behaviors. The `flag` labels are part of the
/// JSON contract and must never change.
pub const FLAG_NAMES: [&str; 10] = [
    "revenge_trading",
    "overtrading",
    "fomo",
    "position_size_escalation",
    "risk_escalation",
    "tilt",
    "panic_exits",
    "consecutive_rule_violations",
    "strategy_switching",
    "excessive_trading_frequency",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    None,
    Low,
    Medium,
    High,
}

impl Severity {
    /// Map a 0..1 intensity onto a severity bucket.
    fn from_intensity(detected: bool, intensity: f64) -> Self {
        if !detected {
            Severity::None
        } else if intensity >= 0.6 {
            Severity::High
        } else if intensity >= 0.3 {
            Severity::Medium
        } else {
            Severity::Low
        }
    }

    /// Control factor used by the scoring engine (1.0 = fully in control).
    pub fn control_factor(&self) -> f64 {
        match self {
            Severity::None => 1.0,
            Severity::Low => 0.7,
            Severity::Medium => 0.4,
            Severity::High => 0.15,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BehavioralFlag {
    /// Stable machine name, one of [`FLAG_NAMES`].
    pub flag: String,
    pub detected: bool,
    pub severity: Severity,
    /// 0..1 — how much data supported this detection.
    pub confidence: f64,
    /// False when the required columns are missing from the CSV.
    pub data_available: bool,
    /// Flag-specific supporting metrics (fixed keys per flag).
    pub metrics: Map<String, Value>,
}

impl BehavioralFlag {
    fn unavailable(flag: &str, missing: &str) -> Self {
        let mut metrics = Map::new();
        metrics.insert("missing_data".to_string(), json!(missing));
        Self {
            flag: flag.to_string(),
            detected: false,
            severity: Severity::None,
            confidence: 0.0,
            data_available: false,
            metrics,
        }
    }

    fn new(
        flag: &str,
        detected: bool,
        intensity: f64,
        confidence: f64,
        metrics: Map<String, Value>,
    ) -> Self {
        Self {
            flag: flag.to_string(),
            detected,
            severity: Severity::from_intensity(detected, intensity),
            confidence: round4(confidence.clamp(0.0, 1.0)),
            data_available: true,
            metrics,
        }
    }
}

pub type BehavioralFlags = Vec<BehavioralFlag>;

/// Run every detector. Trades must be sorted chronologically.
pub fn detect(trades: &[Trade], cfg: &BehaviorConfig) -> BehavioralFlags {
    vec![
        revenge_trading(trades, cfg),
        overtrading(trades, cfg),
        fomo(trades, cfg),
        position_size_escalation(trades),
        risk_escalation(trades),
        tilt(trades, cfg),
        panic_exits(trades),
        consecutive_rule_violations(trades, cfg),
        strategy_switching(trades),
        excessive_trading_frequency(trades, cfg),
    ]
}

fn has_times(trades: &[Trade]) -> bool {
    trades.iter().filter(|t| t.time.is_some()).count() * 2 >= trades.len().max(1)
}

/// Re-entry shortly after a loss with escalated size/risk.
fn revenge_trading(trades: &[Trade], cfg: &BehaviorConfig) -> BehavioralFlag {
    const FLAG: &str = "revenge_trading";
    if !has_times(trades) {
        return BehavioralFlag::unavailable(FLAG, "trade times");
    }
    let mut quick_reentries = 0usize;
    let mut escalated_reentries = 0usize;
    let mut losses = 0usize;
    for pair in trades.windows(2) {
        let (prev, next) = (&pair[0], &pair[1]);
        if !prev.is_loss() {
            continue;
        }
        losses += 1;
        if prev.date != next.date {
            continue;
        }
        let gap = (next.datetime() - prev.datetime()).num_minutes();
        if gap >= 0 && gap <= cfg.revenge_window_minutes {
            quick_reentries += 1;
            let escalated_size = match (prev.position_size, next.position_size) {
                (Some(a), Some(b)) if a > 0.0 => b / a >= 1.25,
                _ => false,
            };
            let escalated_risk = match (prev.risk, next.risk) {
                (Some(a), Some(b)) if a > 0.0 => b / a >= 1.25,
                _ => false,
            };
            if escalated_size || escalated_risk {
                escalated_reentries += 1;
            }
        }
    }
    let ratio = if losses > 0 {
        quick_reentries as f64 / losses as f64
    } else {
        0.0
    };
    let escalation_ratio = if quick_reentries > 0 {
        escalated_reentries as f64 / quick_reentries as f64
    } else {
        0.0
    };
    let detected = quick_reentries >= 2 && (ratio >= 0.2 || escalated_reentries >= 2);
    let intensity = (ratio * 0.6 + escalation_ratio * 0.4).clamp(0.0, 1.0);
    let mut metrics = Map::new();
    metrics.insert("losses".into(), json!(losses));
    metrics.insert("quick_reentries_after_loss".into(), json!(quick_reentries));
    metrics.insert("escalated_reentries".into(), json!(escalated_reentries));
    metrics.insert("quick_reentry_ratio".into(), json!(round4(ratio)));
    metrics.insert("window_minutes".into(), json!(cfg.revenge_window_minutes));
    let confidence = 0.4 + (quick_reentries as f64 / 10.0).min(0.5);
    BehavioralFlag::new(
        FLAG,
        detected,
        intensity,
        if detected { confidence } else { 0.8 },
        metrics,
    )
}

/// Days with a trade count far above the trader's own norm or limit.
fn overtrading(trades: &[Trade], cfg: &BehaviorConfig) -> BehavioralFlag {
    const FLAG: &str = "overtrading";
    let days = daily_pnl(trades);
    if days.is_empty() {
        return BehavioralFlag::unavailable(FLAG, "trades");
    }
    let counts: Vec<f64> = days.values().map(|(_, c)| *c as f64).collect();
    let avg = mean(&counts).unwrap_or(0.0);
    let max = counts.iter().copied().fold(0.0f64, f64::max);
    let days_over_limit = counts
        .iter()
        .filter(|c| **c >= cfg.max_trades_per_day as f64)
        .count();
    let burst_ratio = if avg > 0.0 { max / avg } else { 0.0 };
    // A relative burst only counts when the day was also meaningfully busy
    // in absolute terms, so 3 trades on a 1-trade/day journal don't flag.
    let detected =
        days_over_limit > 0 || (burst_ratio >= 2.5 && max >= (cfg.max_trades_per_day as f64 / 2.0));
    let over_limit_share = days_over_limit as f64 / days.len() as f64;
    let intensity = (over_limit_share * 2.0 + ((burst_ratio - 2.5) / 5.0).max(0.0)).clamp(0.0, 1.0);
    let mut metrics = Map::new();
    metrics.insert("average_trades_per_day".into(), json!(round4(avg)));
    metrics.insert("max_trades_in_one_day".into(), json!(max as usize));
    metrics.insert("days_over_limit".into(), json!(days_over_limit));
    metrics.insert("configured_limit".into(), json!(cfg.max_trades_per_day));
    metrics.insert("burst_ratio".into(), json!(round4(burst_ratio)));
    BehavioralFlag::new(FLAG, detected, intensity, 0.9, metrics)
}

/// Chasing: re-entry very quickly after a winner with a worse planned RR.
fn fomo(trades: &[Trade], cfg: &BehaviorConfig) -> BehavioralFlag {
    const FLAG: &str = "fomo";
    if !has_times(trades) {
        return BehavioralFlag::unavailable(FLAG, "trade times");
    }
    let rrs: Vec<f64> = trades.iter().filter_map(|t| t.rr).collect();
    let avg_rr = mean(&rrs);
    let mut chases = 0usize;
    let mut wins = 0usize;
    for pair in trades.windows(2) {
        let (prev, next) = (&pair[0], &pair[1]);
        if !prev.is_win() {
            continue;
        }
        wins += 1;
        if prev.date != next.date {
            continue;
        }
        let gap = (next.datetime() - prev.datetime()).num_minutes();
        if gap >= 0 && gap <= cfg.fomo_window_minutes {
            let worse_rr = match (next.rr, avg_rr) {
                (Some(rr), Some(avg)) => rr < avg * 0.6,
                _ => true, // no RR data: the quick re-entry itself is the signal
            };
            if worse_rr {
                chases += 1;
            }
        }
    }
    let ratio = if wins > 0 {
        chases as f64 / wins as f64
    } else {
        0.0
    };
    let detected = chases >= 2 && ratio >= 0.15;
    let mut metrics = Map::new();
    metrics.insert("wins".into(), json!(wins));
    metrics.insert("quick_chases_after_win".into(), json!(chases));
    metrics.insert("chase_ratio".into(), json!(round4(ratio)));
    metrics.insert("window_minutes".into(), json!(cfg.fomo_window_minutes));
    // Heuristic signal only — cap the confidence.
    BehavioralFlag::new(FLAG, detected, ratio, 0.5, metrics)
}

/// Average position size after a loss vs. overall average.
fn position_size_escalation(trades: &[Trade]) -> BehavioralFlag {
    const FLAG: &str = "position_size_escalation";
    escalation_flag(FLAG, trades, |t| t.position_size)
}

/// Average risk after a loss vs. overall average, plus a rising risk trend.
fn risk_escalation(trades: &[Trade]) -> BehavioralFlag {
    const FLAG: &str = "risk_escalation";
    escalation_flag(FLAG, trades, |t| t.risk)
}

fn escalation_flag(
    flag: &str,
    trades: &[Trade],
    value: fn(&Trade) -> Option<f64>,
) -> BehavioralFlag {
    let values: Vec<f64> = trades.iter().filter_map(value).collect();
    if values.len() * 2 < trades.len().max(1) || values.is_empty() {
        return BehavioralFlag::unavailable(flag, "position size / risk values");
    }
    let overall_avg = mean(&values).unwrap_or(0.0);
    let mut after_loss = Vec::new();
    for pair in trades.windows(2) {
        if pair[0].is_loss() {
            if let Some(v) = value(&pair[1]) {
                after_loss.push(v);
            }
        }
    }
    if after_loss.is_empty() || overall_avg <= 0.0 {
        let mut metrics = Map::new();
        metrics.insert("samples_after_loss".into(), json!(0));
        return BehavioralFlag::new(flag, false, 0.0, 0.5, metrics);
    }
    let after_loss_avg = mean(&after_loss).unwrap_or(0.0);
    let escalation_ratio = after_loss_avg / overall_avg;
    let detected = escalation_ratio >= 1.3;
    let intensity = ((escalation_ratio - 1.3) / 0.7)
        .clamp(0.0, 1.0)
        .max(if detected { 0.1 } else { 0.0 });
    let mut metrics = Map::new();
    metrics.insert("overall_average".into(), json!(round4(overall_avg)));
    metrics.insert("average_after_loss".into(), json!(round4(after_loss_avg)));
    metrics.insert("escalation_ratio".into(), json!(round4(escalation_ratio)));
    metrics.insert("samples_after_loss".into(), json!(after_loss.len()));
    let confidence = 0.5 + (after_loss.len() as f64 / 20.0).min(0.4);
    BehavioralFlag::new(flag, detected, intensity, confidence, metrics)
}

/// Days where losses kept stacking and the trader kept going.
fn tilt(trades: &[Trade], cfg: &BehaviorConfig) -> BehavioralFlag {
    const FLAG: &str = "tilt";
    if trades.is_empty() {
        return BehavioralFlag::unavailable(FLAG, "trades");
    }
    let mut tilt_days = 0usize;
    let mut worst_consecutive = 0usize;
    let mut current_date = None;
    let mut consecutive_losses = 0usize;
    let mut trades_after_tilt = 0usize;
    for t in trades {
        if current_date != Some(t.date) {
            current_date = Some(t.date);
            consecutive_losses = 0;
        }
        if t.is_loss() {
            consecutive_losses += 1;
            if consecutive_losses == cfg.tilt_consecutive_losses {
                tilt_days += 1;
            }
            if consecutive_losses >= cfg.tilt_consecutive_losses {
                trades_after_tilt += 1;
            }
            worst_consecutive = worst_consecutive.max(consecutive_losses);
        } else {
            if consecutive_losses >= cfg.tilt_consecutive_losses {
                trades_after_tilt += 1;
            }
            consecutive_losses = 0;
        }
    }
    let total_days = daily_pnl(trades).len();
    let tilt_day_ratio = tilt_days as f64 / total_days.max(1) as f64;
    let detected = tilt_days > 0;
    let intensity = (tilt_day_ratio * 3.0).clamp(0.0, 1.0);
    let mut metrics = Map::new();
    metrics.insert("tilt_days".into(), json!(tilt_days));
    metrics.insert("tilt_day_ratio".into(), json!(round4(tilt_day_ratio)));
    metrics.insert(
        "worst_consecutive_losses_in_day".into(),
        json!(worst_consecutive),
    );
    metrics.insert(
        "trades_after_tilt_threshold".into(),
        json!(trades_after_tilt),
    );
    metrics.insert("threshold".into(), json!(cfg.tilt_consecutive_losses));
    BehavioralFlag::new(FLAG, detected, intensity, 0.85, metrics)
}

/// Winners cut early: high win rate but tiny average win vs. average loss.
fn panic_exits(trades: &[Trade]) -> BehavioralFlag {
    const FLAG: &str = "panic_exits";
    let wins: Vec<f64> = trades
        .iter()
        .filter(|t| t.is_win())
        .map(|t| t.pnl)
        .collect();
    let losses: Vec<f64> = trades
        .iter()
        .filter(|t| t.is_loss())
        .map(|t| t.pnl.abs())
        .collect();
    if wins.is_empty() || losses.is_empty() {
        return BehavioralFlag::unavailable(FLAG, "both wins and losses");
    }
    let avg_win = mean(&wins).unwrap_or(0.0);
    let avg_loss = mean(&losses).unwrap_or(0.0);
    let win_loss_ratio = if avg_loss > 0.0 {
        avg_win / avg_loss
    } else {
        f64::MAX
    };
    let win_rate = wins.len() as f64 / trades.len() as f64;

    // Planned-vs-realized RR when both are available.
    let mut realized_below_plan = 0usize;
    let mut planned_samples = 0usize;
    for t in trades.iter().filter(|t| t.is_win()) {
        if let (Some(risk), Some(reward)) = (t.risk, t.reward) {
            if risk > 0.0 {
                planned_samples += 1;
                let planned_rr = reward / risk;
                let realized_rr = t.pnl / risk;
                if realized_rr < planned_rr * 0.5 {
                    realized_below_plan += 1;
                }
            }
        }
    }

    let cut_winners = win_rate >= 0.55 && win_loss_ratio < 0.5;
    let below_plan_ratio = if planned_samples > 0 {
        realized_below_plan as f64 / planned_samples as f64
    } else {
        0.0
    };
    let detected = cut_winners || (planned_samples >= 5 && below_plan_ratio >= 0.4);
    let intensity = if cut_winners {
        (0.5 - win_loss_ratio).clamp(0.0, 0.5) * 2.0
    } else {
        below_plan_ratio
    };
    let mut metrics = Map::new();
    metrics.insert("average_win".into(), json!(round4(avg_win)));
    metrics.insert("average_loss".into(), json!(round4(avg_loss)));
    metrics.insert(
        "win_loss_ratio".into(),
        json!(round4(win_loss_ratio.min(999.0))),
    );
    metrics.insert("win_rate".into(), json!(round4(win_rate)));
    metrics.insert("planned_rr_samples".into(), json!(planned_samples));
    metrics.insert(
        "wins_realized_below_half_plan".into(),
        json!(realized_below_plan),
    );
    let confidence = if planned_samples >= 5 { 0.8 } else { 0.6 };
    BehavioralFlag::new(FLAG, detected, intensity, confidence, metrics)
}

/// Risk repeatedly above `risk_violation_multiple` x median risk, in a row.
fn consecutive_rule_violations(trades: &[Trade], cfg: &BehaviorConfig) -> BehavioralFlag {
    const FLAG: &str = "consecutive_rule_violations";
    let risks: Vec<f64> = trades.iter().filter_map(|t| t.risk).collect();
    if risks.len() * 2 < trades.len().max(1) || risks.is_empty() {
        return BehavioralFlag::unavailable(FLAG, "risk values");
    }
    let med = median(&risks).unwrap_or(0.0);
    if med <= 0.0 {
        return BehavioralFlag::unavailable(FLAG, "positive risk values");
    }
    let limit = med * cfg.risk_violation_multiple;
    let mut violations = 0usize;
    let mut max_consecutive = 0usize;
    let mut current = 0usize;
    for t in trades {
        let violated = t.risk.map(|r| r > limit).unwrap_or(false);
        if violated {
            violations += 1;
            current += 1;
            max_consecutive = max_consecutive.max(current);
        } else {
            current = 0;
        }
    }
    let violation_rate = violations as f64 / trades.len() as f64;
    let detected = max_consecutive >= cfg.consecutive_violation_threshold;
    let intensity = (violation_rate * 3.0).clamp(0.0, 1.0);
    let mut metrics = Map::new();
    metrics.insert("median_risk".into(), json!(round4(med)));
    metrics.insert("violation_limit".into(), json!(round4(limit)));
    metrics.insert("violations".into(), json!(violations));
    metrics.insert("violation_rate".into(), json!(round4(violation_rate)));
    metrics.insert("max_consecutive_violations".into(), json!(max_consecutive));
    metrics.insert(
        "threshold".into(),
        json!(cfg.consecutive_violation_threshold),
    );
    BehavioralFlag::new(FLAG, detected, intensity, 0.85, metrics)
}

/// Switching strategies right after losses more often than after wins.
fn strategy_switching(trades: &[Trade]) -> BehavioralFlag {
    const FLAG: &str = "strategy_switching";
    let with_strategy = trades.iter().filter(|t| t.strategy.is_some()).count();
    if with_strategy * 2 < trades.len().max(1) || with_strategy < 4 {
        return BehavioralFlag::unavailable(FLAG, "strategy labels");
    }
    let mut switches_after_loss = 0usize;
    let mut losses_followed = 0usize;
    let mut switches_after_win = 0usize;
    let mut wins_followed = 0usize;
    for pair in trades.windows(2) {
        let (prev, next) = (&pair[0], &pair[1]);
        let (Some(a), Some(b)) = (&prev.strategy, &next.strategy) else {
            continue;
        };
        let switched = a != b;
        if prev.is_loss() {
            losses_followed += 1;
            if switched {
                switches_after_loss += 1;
            }
        } else if prev.is_win() {
            wins_followed += 1;
            if switched {
                switches_after_win += 1;
            }
        }
    }
    let loss_switch_rate = if losses_followed > 0 {
        switches_after_loss as f64 / losses_followed as f64
    } else {
        0.0
    };
    let win_switch_rate = if wins_followed > 0 {
        switches_after_win as f64 / wins_followed as f64
    } else {
        0.0
    };
    let distinct: std::collections::HashSet<&String> =
        trades.iter().filter_map(|t| t.strategy.as_ref()).collect();
    let detected = losses_followed >= 3
        && loss_switch_rate >= 0.5
        && loss_switch_rate > win_switch_rate + 0.15;
    let intensity = (loss_switch_rate - win_switch_rate).clamp(0.0, 1.0);
    let mut metrics = Map::new();
    metrics.insert("distinct_strategies".into(), json!(distinct.len()));
    metrics.insert(
        "switch_rate_after_loss".into(),
        json!(round4(loss_switch_rate)),
    );
    metrics.insert(
        "switch_rate_after_win".into(),
        json!(round4(win_switch_rate)),
    );
    metrics.insert("losses_followed".into(), json!(losses_followed));
    BehavioralFlag::new(FLAG, detected, intensity, 0.7, metrics)
}

/// Sustained high frequency (not just one burst day).
fn excessive_trading_frequency(trades: &[Trade], cfg: &BehaviorConfig) -> BehavioralFlag {
    const FLAG: &str = "excessive_trading_frequency";
    let days = daily_pnl(trades);
    if days.is_empty() {
        return BehavioralFlag::unavailable(FLAG, "trades");
    }
    let counts: Vec<f64> = days.values().map(|(_, c)| *c as f64).collect();
    let avg = mean(&counts).unwrap_or(0.0);
    let limit = cfg.max_trades_per_day as f64;
    let days_over = counts.iter().filter(|c| **c >= limit).count();
    let share_over = days_over as f64 / days.len() as f64;
    let detected = avg >= limit * 0.8 || share_over >= 0.3;
    let intensity = ((avg / limit).min(2.0) / 2.0 * 0.5 + share_over * 0.5).clamp(0.0, 1.0);
    let mut metrics = Map::new();
    metrics.insert("average_trades_per_day".into(), json!(round4(avg)));
    metrics.insert("days_at_or_over_limit".into(), json!(days_over));
    metrics.insert("share_of_days_over_limit".into(), json!(round4(share_over)));
    metrics.insert("configured_limit".into(), json!(cfg.max_trades_per_day));
    BehavioralFlag::new(FLAG, detected, intensity, 0.9, metrics)
}

/// Look up a flag by name (all detectors always emit all flags).
pub fn find<'a>(flags: &'a [BehavioralFlag], name: &str) -> Option<&'a BehavioralFlag> {
    flags.iter().find(|f| f.flag == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, NaiveTime};

    fn trade(day: u32, hm: (u32, u32), pnl: f64, size: Option<f64>, risk: Option<f64>) -> Trade {
        Trade {
            index: 0,
            date: NaiveDate::from_ymd_opt(2026, 1, day).unwrap(),
            time: Some(NaiveTime::from_hms_opt(hm.0, hm.1, 0).unwrap()),
            symbol: "EURUSD".to_string(),
            direction: None,
            entry: None,
            exit: None,
            position_size: size,
            risk,
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
    fn all_flags_always_emitted() {
        let trades = vec![trade(1, (9, 0), 100.0, None, None)];
        let flags = detect(&trades, &BehaviorConfig::default());
        assert_eq!(flags.len(), FLAG_NAMES.len());
        for name in FLAG_NAMES {
            assert!(find(&flags, name).is_some(), "missing flag {name}");
        }
    }

    #[test]
    fn revenge_trading_detected() {
        // Losses immediately followed by bigger-size re-entries, same day.
        let trades = vec![
            trade(1, (9, 0), -100.0, Some(1.0), None),
            trade(1, (9, 3), -150.0, Some(2.0), None),
            trade(1, (9, 7), -200.0, Some(4.0), None),
            trade(1, (9, 10), 50.0, Some(8.0), None),
        ];
        let flag = revenge_trading(&trades, &BehaviorConfig::default());
        assert!(flag.detected);
        assert!(flag.severity > Severity::None);
        assert_eq!(flag.metrics["quick_reentries_after_loss"], json!(3));
    }

    #[test]
    fn slow_reentries_are_not_revenge() {
        // Same escalation pattern but 20+ minutes between trades: for a
        // scalper-tuned window this is just the next setup, not revenge.
        let trades = vec![
            trade(1, (9, 0), -100.0, Some(1.0), None),
            trade(1, (9, 25), -150.0, Some(2.0), None),
            trade(1, (9, 50), -200.0, Some(4.0), None),
            trade(1, (10, 15), 50.0, Some(8.0), None),
        ];
        let flag = revenge_trading(&trades, &BehaviorConfig::default());
        assert!(!flag.detected);
        assert_eq!(flag.metrics["quick_reentries_after_loss"], json!(0));
    }

    #[test]
    fn revenge_needs_time_data() {
        let mut t = trade(1, (9, 0), -100.0, None, None);
        t.time = None;
        let flag = revenge_trading(&[t], &BehaviorConfig::default());
        assert!(!flag.data_available);
        assert!(!flag.detected);
    }

    #[test]
    fn overtrading_detected_on_burst_day() {
        let mut trades: Vec<Trade> = (0..12)
            .map(|i| trade(1, (9, i), 10.0, None, None))
            .collect();
        trades.push(trade(2, (9, 0), 10.0, None, None));
        let flag = overtrading(&trades, &BehaviorConfig::default());
        assert!(flag.detected);
        assert_eq!(flag.metrics["max_trades_in_one_day"], json!(12));
    }

    #[test]
    fn tilt_detected_after_consecutive_losses() {
        let trades = vec![
            trade(1, (9, 0), -50.0, None, None),
            trade(1, (10, 0), -50.0, None, None),
            trade(1, (11, 0), -50.0, None, None),
            trade(1, (12, 0), -50.0, None, None),
        ];
        let flag = tilt(&trades, &BehaviorConfig::default());
        assert!(flag.detected);
        assert_eq!(flag.metrics["worst_consecutive_losses_in_day"], json!(4));
    }

    #[test]
    fn panic_exits_on_cut_winners() {
        // 70% win rate but avg win far below avg loss.
        let mut trades = Vec::new();
        for i in 0..7 {
            trades.push(trade(1 + i, (9, 0), 10.0, None, None));
        }
        for i in 0..3 {
            trades.push(trade(10 + i, (9, 0), -100.0, None, None));
        }
        let flag = panic_exits(&trades);
        assert!(flag.detected);
    }

    #[test]
    fn risk_escalation_after_losses() {
        let trades = vec![
            trade(1, (9, 0), 80.0, None, Some(100.0)),
            trade(1, (11, 0), -50.0, None, Some(100.0)),
            trade(2, (9, 0), -50.0, None, Some(300.0)),
            trade(2, (11, 0), -50.0, None, Some(500.0)),
            trade(3, (9, 0), 100.0, None, Some(500.0)),
        ];
        let flag = risk_escalation(&trades);
        assert!(flag.data_available);
        assert!(flag.detected);
    }

    #[test]
    fn no_false_positive_on_clean_journal() {
        let trades = vec![
            trade(1, (9, 0), 100.0, Some(1.0), Some(100.0)),
            trade(2, (9, 0), -50.0, Some(1.0), Some(100.0)),
            trade(3, (9, 0), 120.0, Some(1.0), Some(100.0)),
            trade(4, (9, 0), -40.0, Some(1.0), Some(100.0)),
            trade(5, (9, 0), 90.0, Some(1.0), Some(100.0)),
        ];
        let flags = detect(&trades, &BehaviorConfig::default());
        let detected: Vec<&str> = flags
            .iter()
            .filter(|f| f.detected)
            .map(|f| f.flag.as_str())
            .collect();
        assert!(detected.is_empty(), "unexpected flags: {detected:?}");
    }
}
