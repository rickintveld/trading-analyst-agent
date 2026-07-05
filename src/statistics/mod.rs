//! Deterministic statistics. Every function is pure: trades in, numbers out.
//! Trades are expected to be sorted chronologically (see
//! [`crate::shared::sort_chronologically`]).

use indexmap::IndexMap;
use rayon::prelude::*;

use crate::ai::{
    DailyStatistics, DayPnl, MonthPnl, MonthlyStatistics, PairStatistics, PairStats, Performance,
    SessionStatistics, SessionStats, StrategyStatistics, StrategyStats, Streaks, TimeSeries,
    TradePoint, TradingPeriod, WeekPnl, WeeklyStatistics,
};
use crate::config::SessionsConfig;
use crate::shared::{linear_slope, mean, round2, round4, Session, Trade, PNL_EPSILON};

pub fn trading_period(trades: &[Trade]) -> TradingPeriod {
    let first = trades.first().map(|t| t.date).unwrap_or_default();
    let last = trades.last().map(|t| t.date).unwrap_or_default();
    TradingPeriod {
        first_trade_date: first.to_string(),
        last_trade_date: last.to_string(),
        total_calendar_days: (last - first).num_days() + 1,
        total_trading_days: daily_pnl(trades).len(),
    }
}

pub fn performance(trades: &[Trade]) -> Performance {
    let total = trades.len();
    let wins: Vec<f64> = trades
        .iter()
        .filter(|t| t.is_win())
        .map(|t| t.pnl)
        .collect();
    let losses: Vec<f64> = trades
        .iter()
        .filter(|t| t.is_loss())
        .map(|t| t.pnl)
        .collect();
    let breakeven = total - wins.len() - losses.len();

    let total_pnl: f64 = trades.iter().map(|t| t.pnl).sum();
    let gross_profit: f64 = wins.iter().sum();
    let gross_loss: f64 = losses.iter().sum();
    let total_fees: f64 = trades.iter().filter_map(|t| t.fees).sum();

    let win_rate = if total > 0 {
        wins.len() as f64 / total as f64 * 100.0
    } else {
        0.0
    };
    let loss_rate = if total > 0 {
        losses.len() as f64 / total as f64 * 100.0
    } else {
        0.0
    };

    let average_win = mean(&wins);
    let average_loss = mean(&losses);
    let profit_factor = if gross_loss.abs() > PNL_EPSILON {
        Some(gross_profit / gross_loss.abs())
    } else {
        None
    };
    let expectancy = match (average_win, average_loss) {
        (None, None) => None,
        _ => Some(
            win_rate / 100.0 * average_win.unwrap_or(0.0)
                + loss_rate / 100.0 * average_loss.unwrap_or(0.0),
        ),
    };
    let rrs: Vec<f64> = trades.iter().filter_map(|t| t.rr).collect();

    Performance {
        total_trades: total,
        winning_trades: wins.len(),
        losing_trades: losses.len(),
        breakeven_trades: breakeven,
        total_pnl: round2(total_pnl),
        gross_profit: round2(gross_profit),
        gross_loss: round2(gross_loss),
        net_profit: round2(total_pnl - total_fees),
        total_fees: round2(total_fees),
        win_rate: round2(win_rate),
        loss_rate: round2(loss_rate),
        average_win: average_win.map(round2),
        average_loss: average_loss.map(round2),
        profit_factor: profit_factor.map(round4),
        expectancy: expectancy.map(round2),
        average_rr: mean(&rrs).map(round4),
        average_trade: if total > 0 {
            round2(total_pnl / total as f64)
        } else {
            0.0
        },
        largest_win: wins.iter().copied().max_by(f64::total_cmp).map(round2),
        largest_loss: losses.iter().copied().min_by(f64::total_cmp).map(round2),
    }
}

/// Ordered map date -> (pnl, trade count). Insertion order is chronological
/// because trades are sorted.
pub fn daily_pnl(trades: &[Trade]) -> IndexMap<chrono::NaiveDate, (f64, usize)> {
    let mut days: IndexMap<chrono::NaiveDate, (f64, usize)> = IndexMap::new();
    for t in trades {
        let entry = days.entry(t.date).or_insert((0.0, 0));
        entry.0 += t.pnl;
        entry.1 += 1;
    }
    days
}

pub fn daily_statistics(trades: &[Trade]) -> DailyStatistics {
    let days = daily_pnl(trades);
    let day_stats: Vec<DayPnl> = days
        .iter()
        .map(|(date, (pnl, count))| DayPnl {
            date: date.to_string(),
            pnl: round2(*pnl),
            trades: *count,
        })
        .collect();

    let positive = day_stats.iter().filter(|d| d.pnl > PNL_EPSILON).count();
    let negative = day_stats.iter().filter(|d| d.pnl < -PNL_EPSILON).count();
    let flat = day_stats.len() - positive - negative;

    let pnls: Vec<f64> = day_stats.iter().map(|d| d.pnl).collect();
    let best = day_stats
        .iter()
        .max_by(|a, b| a.pnl.total_cmp(&b.pnl))
        .cloned();
    let worst = day_stats
        .iter()
        .min_by(|a, b| a.pnl.total_cmp(&b.pnl))
        .cloned();

    // Max drawdown over the daily cumulative equity curve.
    let mut equity = 0.0f64;
    let mut peak = 0.0f64;
    let mut max_dd = 0.0f64;
    for pnl in &pnls {
        equity += pnl;
        peak = peak.max(equity);
        max_dd = max_dd.max(peak - equity);
    }

    let trade_counts: Vec<usize> = days.values().map(|(_, c)| *c).collect();
    DailyStatistics {
        trading_days: day_stats.len(),
        positive_days: positive,
        negative_days: negative,
        flat_days: flat,
        average_daily_pnl: round2(mean(&pnls).unwrap_or(0.0)),
        best_day: best,
        worst_day: worst,
        max_daily_drawdown: round2(max_dd),
        average_trades_per_day: round2(if trade_counts.is_empty() {
            0.0
        } else {
            trade_counts.iter().sum::<usize>() as f64 / trade_counts.len() as f64
        }),
        max_trades_in_one_day: trade_counts.iter().copied().max().unwrap_or(0),
    }
}

pub fn weekly_statistics(trades: &[Trade]) -> WeeklyStatistics {
    use chrono::Datelike;
    let mut weeks: IndexMap<String, Vec<&Trade>> = IndexMap::new();
    for t in trades {
        let iso = t.date.iso_week();
        let key = format!("{}-W{:02}", iso.year(), iso.week());
        weeks.entry(key).or_default().push(t);
    }
    let week_stats: Vec<WeekPnl> = weeks
        .iter()
        .map(|(week, ts)| {
            let pnl: f64 = ts.iter().map(|t| t.pnl).sum();
            let wins = ts.iter().filter(|t| t.is_win()).count();
            WeekPnl {
                week: week.clone(),
                pnl: round2(pnl),
                trades: ts.len(),
                win_rate: round2(wins as f64 / ts.len() as f64 * 100.0),
            }
        })
        .collect();

    let pnls: Vec<f64> = week_stats.iter().map(|w| w.pnl).collect();
    WeeklyStatistics {
        average_weekly_pnl: round2(mean(&pnls).unwrap_or(0.0)),
        best_week: week_stats
            .iter()
            .max_by(|a, b| a.pnl.total_cmp(&b.pnl))
            .cloned(),
        worst_week: week_stats
            .iter()
            .min_by(|a, b| a.pnl.total_cmp(&b.pnl))
            .cloned(),
        winning_weeks: week_stats.iter().filter(|w| w.pnl > PNL_EPSILON).count(),
        losing_weeks: week_stats.iter().filter(|w| w.pnl < -PNL_EPSILON).count(),
        weeks: week_stats,
    }
}

pub fn monthly_statistics(trades: &[Trade]) -> MonthlyStatistics {
    let mut months: IndexMap<String, (f64, usize)> = IndexMap::new();
    for t in trades {
        let key = t.date.format("%Y-%m").to_string();
        let entry = months.entry(key).or_insert((0.0, 0));
        entry.0 += t.pnl;
        entry.1 += 1;
    }
    let month_stats: Vec<MonthPnl> = months
        .iter()
        .map(|(month, (pnl, trades))| MonthPnl {
            month: month.clone(),
            pnl: round2(*pnl),
            trades: *trades,
        })
        .collect();

    let pnls: Vec<f64> = month_stats.iter().map(|m| m.pnl).collect();
    let trend = match linear_slope(&pnls) {
        None => "insufficient_data".to_string(),
        Some(slope) => {
            let mean_abs = pnls.iter().map(|p| p.abs()).sum::<f64>() / pnls.len() as f64;
            let threshold = (mean_abs * 0.02).max(PNL_EPSILON);
            if slope > threshold {
                "improving".to_string()
            } else if slope < -threshold {
                "declining".to_string()
            } else {
                "flat".to_string()
            }
        }
    };

    MonthlyStatistics {
        average_monthly_pnl: round2(mean(&pnls).unwrap_or(0.0)),
        best_month: month_stats
            .iter()
            .max_by(|a, b| a.pnl.total_cmp(&b.pnl))
            .cloned(),
        worst_month: month_stats
            .iter()
            .min_by(|a, b| a.pnl.total_cmp(&b.pnl))
            .cloned(),
        trend,
        months: month_stats,
    }
}

/// Generic per-group aggregation used for pairs and strategies.
fn group_stats(
    groups: IndexMap<String, Vec<&Trade>>,
) -> Vec<(String, usize, f64, Option<f64>, f64)> {
    let entries: Vec<(String, Vec<&Trade>)> = groups.into_iter().collect();
    let mut stats: Vec<(String, usize, f64, Option<f64>, f64)> = entries
        .par_iter()
        .map(|(name, ts)| {
            let wins = ts.iter().filter(|t| t.is_win()).count();
            let rrs: Vec<f64> = ts.iter().filter_map(|t| t.rr).collect();
            let pnl: f64 = ts.iter().map(|t| t.pnl).sum();
            (
                name.clone(),
                ts.len(),
                round2(wins as f64 / ts.len() as f64 * 100.0),
                mean(&rrs).map(round4),
                round2(pnl),
            )
        })
        .collect();
    stats.sort_by(|a, b| b.4.total_cmp(&a.4));
    stats
}

pub fn pair_statistics(trades: &[Trade]) -> PairStatistics {
    let mut groups: IndexMap<String, Vec<&Trade>> = IndexMap::new();
    for t in trades {
        groups.entry(t.symbol.clone()).or_default().push(t);
    }
    let stats = group_stats(groups);
    let pairs: Vec<PairStats> = stats
        .into_iter()
        .map(
            |(pair, total_trades, win_rate, average_rr, total_pnl)| PairStats {
                pair,
                total_trades,
                win_rate,
                average_rr,
                total_pnl,
            },
        )
        .collect();
    PairStatistics {
        best_pair: pairs.first().map(|p| p.pair.clone()),
        worst_pair: if pairs.len() > 1 {
            pairs.last().map(|p| p.pair.clone())
        } else {
            None
        },
        pairs,
    }
}

pub fn strategy_statistics(trades: &[Trade]) -> StrategyStatistics {
    let mut groups: IndexMap<String, Vec<&Trade>> = IndexMap::new();
    for t in trades {
        if let Some(strategy) = &t.strategy {
            groups.entry(strategy.clone()).or_default().push(t);
        }
    }
    if groups.is_empty() {
        return StrategyStatistics {
            data_available: false,
            strategies: Vec::new(),
            best_strategy: None,
            worst_strategy: None,
        };
    }
    let stats = group_stats(groups);
    let strategies: Vec<StrategyStats> = stats
        .into_iter()
        .map(
            |(strategy, total_trades, win_rate, average_rr, total_pnl)| StrategyStats {
                strategy,
                total_trades,
                win_rate,
                average_rr,
                total_pnl,
            },
        )
        .collect();
    StrategyStatistics {
        data_available: true,
        best_strategy: strategies.first().map(|s| s.strategy.clone()),
        worst_strategy: if strategies.len() > 1 {
            strategies.last().map(|s| s.strategy.clone())
        } else {
            None
        },
        strategies,
    }
}

/// Resolve the session of a trade: explicit column first, otherwise inferred
/// from the trade time via configured session windows.
pub fn resolve_session(trade: &Trade, sessions: &SessionsConfig) -> Option<Session> {
    trade
        .session
        .or_else(|| trade.time.map(|t| sessions.session_for(t)))
}

pub fn session_statistics(trades: &[Trade], sessions_cfg: &SessionsConfig) -> SessionStatistics {
    let resolved: Vec<(Option<Session>, &Trade)> = trades
        .iter()
        .map(|t| (resolve_session(t, sessions_cfg), t))
        .collect();
    let data_available = resolved.iter().any(|(s, _)| s.is_some());

    let sessions: Vec<SessionStats> = Session::ALL
        .iter()
        .map(|session| {
            let ts: Vec<&Trade> = resolved
                .iter()
                .filter(|(s, _)| *s == Some(*session))
                .map(|(_, t)| *t)
                .collect();
            let pnl: f64 = ts.iter().map(|t| t.pnl).sum();
            let wins = ts.iter().filter(|t| t.is_win()).count();
            let rrs: Vec<f64> = ts.iter().filter_map(|t| t.rr).collect();
            SessionStats {
                session: session.label().to_string(),
                total_trades: ts.len(),
                win_rate: if ts.is_empty() {
                    0.0
                } else {
                    round2(wins as f64 / ts.len() as f64 * 100.0)
                },
                total_pnl: round2(pnl),
                average_pnl: if ts.is_empty() {
                    0.0
                } else {
                    round2(pnl / ts.len() as f64)
                },
                average_rr: mean(&rrs).map(round4),
            }
        })
        .collect();

    let best_session = sessions
        .iter()
        .filter(|s| s.total_trades > 0)
        .max_by(|a, b| a.total_pnl.total_cmp(&b.total_pnl))
        .map(|s| s.session.clone());

    SessionStatistics {
        data_available,
        sessions,
        best_session,
    }
}

pub fn streaks(trades: &[Trade]) -> Streaks {
    // Trade-level streaks (breakeven trades break streaks).
    let mut longest_win = 0usize;
    let mut longest_loss = 0usize;
    let mut current: i64 = 0;
    for t in trades {
        if t.is_win() {
            current = if current > 0 { current + 1 } else { 1 };
        } else if t.is_loss() {
            current = if current < 0 { current - 1 } else { -1 };
        } else {
            current = 0;
        }
        longest_win = longest_win.max(current.max(0) as usize);
        longest_loss = longest_loss.max((-current).max(0) as usize);
    }
    let current_trades = current;

    // Day-level streaks over daily PnL.
    let days = daily_pnl(trades);
    let mut longest_win_days = 0usize;
    let mut longest_loss_days = 0usize;
    let mut current_days: i64 = 0;
    for (pnl, _) in days.values() {
        if *pnl > PNL_EPSILON {
            current_days = if current_days > 0 {
                current_days + 1
            } else {
                1
            };
        } else if *pnl < -PNL_EPSILON {
            current_days = if current_days < 0 {
                current_days - 1
            } else {
                -1
            };
        } else {
            current_days = 0;
        }
        longest_win_days = longest_win_days.max(current_days.max(0) as usize);
        longest_loss_days = longest_loss_days.max((-current_days).max(0) as usize);
    }

    Streaks {
        longest_winning_streak_trades: longest_win,
        longest_losing_streak_trades: longest_loss,
        current_streak_trades: current_trades,
        longest_winning_streak_days: longest_win_days,
        longest_losing_streak_days: longest_loss_days,
        current_streak_days: current_days,
    }
}

/// Rolling RR window size for [`time_series`].
pub const ROLLING_RR_WINDOW: usize = 10;

/// Maximum points kept per time-series array in the exchange JSON. Larger
/// journals are stride-sampled deterministically (last point always kept)
/// to protect both chart rendering and AI token usage.
pub const MAX_TIME_SERIES_POINTS: usize = 1000;

fn stride_sample<T: Clone>(items: &[T], max: usize) -> Vec<T> {
    if items.len() <= max {
        return items.to_vec();
    }
    let stride = items.len().div_ceil(max);
    let mut sampled: Vec<T> = items.iter().step_by(stride).cloned().collect();
    if !(items.len() - 1).is_multiple_of(stride) {
        sampled.push(items.last().unwrap().clone()); // always keep the final point
    }
    sampled
}

/// Per-trade and per-day chart datasets. Trades must be sorted.
pub fn time_series(trades: &[Trade]) -> TimeSeries {
    let mut points = Vec::with_capacity(trades.len());
    let mut equity = 0.0f64;
    let mut peak = 0.0f64;
    let mut streak = 0i64;
    let mut rr_window: std::collections::VecDeque<f64> =
        std::collections::VecDeque::with_capacity(ROLLING_RR_WINDOW);

    for (i, t) in trades.iter().enumerate() {
        equity += t.pnl;
        peak = peak.max(equity);
        streak = if t.is_win() {
            if streak > 0 {
                streak + 1
            } else {
                1
            }
        } else if t.is_loss() {
            if streak < 0 {
                streak - 1
            } else {
                -1
            }
        } else {
            0
        };
        if let Some(rr) = t.rr {
            if rr_window.len() == ROLLING_RR_WINDOW {
                rr_window.pop_front();
            }
            rr_window.push_back(rr);
        }
        let rolling_rr = if rr_window.is_empty() {
            None
        } else {
            Some(round4(
                rr_window.iter().sum::<f64>() / rr_window.len() as f64,
            ))
        };
        points.push(TradePoint {
            n: i + 1,
            date: t.date.to_string(),
            pnl: round2(t.pnl),
            cumulative_pnl: round2(equity),
            drawdown: round2(peak - equity),
            risk: t.risk.map(round2),
            rr: t.rr.map(round4),
            rolling_rr,
            streak,
        });
    }

    let daily: Vec<DayPnl> = daily_pnl(trades)
        .iter()
        .map(|(date, (pnl, count))| DayPnl {
            date: date.to_string(),
            pnl: round2(*pnl),
            trades: *count,
        })
        .collect();

    let downsampled = points.len() > MAX_TIME_SERIES_POINTS || daily.len() > MAX_TIME_SERIES_POINTS;
    TimeSeries {
        downsampled,
        max_points: MAX_TIME_SERIES_POINTS,
        trades: stride_sample(&points, MAX_TIME_SERIES_POINTS),
        daily: stride_sample(&daily, MAX_TIME_SERIES_POINTS),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, NaiveTime};

    fn trade(day: u32, pnl: f64) -> Trade {
        trade_at(day, None, pnl)
    }

    fn trade_at(day: u32, time: Option<(u32, u32)>, pnl: f64) -> Trade {
        Trade {
            index: 0,
            date: NaiveDate::from_ymd_opt(2026, 1, day).unwrap(),
            time: time.map(|(h, m)| NaiveTime::from_hms_opt(h, m, 0).unwrap()),
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
    fn performance_basics() {
        let trades = vec![
            trade(1, 100.0),
            trade(2, -50.0),
            trade(3, 200.0),
            trade(4, 0.0),
        ];
        let p = performance(&trades);
        assert_eq!(p.total_trades, 4);
        assert_eq!(p.winning_trades, 2);
        assert_eq!(p.losing_trades, 1);
        assert_eq!(p.breakeven_trades, 1);
        assert_eq!(p.total_pnl, 250.0);
        assert_eq!(p.gross_profit, 300.0);
        assert_eq!(p.gross_loss, -50.0);
        assert_eq!(p.win_rate, 50.0);
        assert_eq!(p.profit_factor, Some(6.0));
        assert_eq!(p.average_win, Some(150.0));
        assert_eq!(p.average_loss, Some(-50.0));
        assert_eq!(p.largest_win, Some(200.0));
        assert_eq!(p.largest_loss, Some(-50.0));
        // expectancy = 0.5*150 + 0.25*(-50) = 62.5
        assert_eq!(p.expectancy, Some(62.5));
    }

    #[test]
    fn profit_factor_none_without_losses() {
        let p = performance(&[trade(1, 100.0)]);
        assert_eq!(p.profit_factor, None);
    }

    #[test]
    fn daily_stats_and_drawdown() {
        // days: +100, -200, +50 -> equity 100, -100, -50; peak 100; max dd 200
        let trades = vec![trade(1, 100.0), trade(2, -200.0), trade(3, 50.0)];
        let d = daily_statistics(&trades);
        assert_eq!(d.trading_days, 3);
        assert_eq!(d.positive_days, 2);
        assert_eq!(d.negative_days, 1);
        assert_eq!(d.max_daily_drawdown, 200.0);
        assert_eq!(d.best_day.unwrap().pnl, 100.0);
        assert_eq!(d.worst_day.unwrap().pnl, -200.0);
    }

    #[test]
    fn streak_tracking() {
        let trades = vec![
            trade(1, 10.0),
            trade(2, 10.0),
            trade(3, 10.0),
            trade(4, -5.0),
            trade(5, -5.0),
            trade(6, 10.0),
        ];
        let s = streaks(&trades);
        assert_eq!(s.longest_winning_streak_trades, 3);
        assert_eq!(s.longest_losing_streak_trades, 2);
        assert_eq!(s.current_streak_trades, 1);
        assert_eq!(s.longest_winning_streak_days, 3);
        assert_eq!(s.current_streak_days, 1);
    }

    #[test]
    fn monthly_trend_detection() {
        let mut trades = Vec::new();
        for (month, pnl) in [(1u32, -100.0), (2, 50.0), (3, 300.0)] {
            trades.push(Trade {
                date: NaiveDate::from_ymd_opt(2026, month, 15).unwrap(),
                ..trade(1, pnl)
            });
        }
        let m = monthly_statistics(&trades);
        assert_eq!(m.trend, "improving");
        assert_eq!(m.months.len(), 3);
    }

    #[test]
    fn session_inference_from_time() {
        let cfg = SessionsConfig::default();
        let trades = vec![
            trade_at(1, Some((8, 0)), 100.0),  // london
            trade_at(1, Some((14, 0)), -50.0), // overlap
            trade_at(2, Some((17, 0)), 75.0),  // new york
        ];
        let s = session_statistics(&trades, &cfg);
        assert!(s.data_available);
        let london = s.sessions.iter().find(|x| x.session == "london").unwrap();
        assert_eq!(london.total_trades, 1);
        assert_eq!(london.total_pnl, 100.0);
        assert_eq!(s.best_session.as_deref(), Some("london"));
    }

    #[test]
    fn time_series_curve() {
        let trades = vec![trade(1, 100.0), trade(2, -200.0), trade(3, 50.0)];
        let ts = time_series(&trades);
        assert!(!ts.downsampled);
        assert_eq!(ts.trades.len(), 3);
        let cumulative: Vec<f64> = ts.trades.iter().map(|p| p.cumulative_pnl).collect();
        assert_eq!(cumulative, vec![100.0, -100.0, -50.0]);
        let drawdowns: Vec<f64> = ts.trades.iter().map(|p| p.drawdown).collect();
        assert_eq!(drawdowns, vec![0.0, 200.0, 150.0]);
        let streaks: Vec<i64> = ts.trades.iter().map(|p| p.streak).collect();
        assert_eq!(streaks, vec![1, -1, 1]);
        assert_eq!(ts.daily.len(), 3);
    }

    #[test]
    fn time_series_downsamples_large_journals() {
        let trades: Vec<Trade> = (0..2500)
            .map(|i| Trade {
                date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap() + chrono::Duration::days(i / 10),
                ..trade(1, 10.0)
            })
            .collect();
        let ts = time_series(&trades);
        assert!(ts.downsampled);
        assert!(ts.trades.len() <= MAX_TIME_SERIES_POINTS + 1);
        // final point survives sampling
        assert_eq!(ts.trades.last().unwrap().n, 2500);
        assert_eq!(ts.trades.last().unwrap().cumulative_pnl, 25000.0);
    }

    #[test]
    fn pair_ranking() {
        let mut trades = vec![trade(1, 100.0), trade(2, 50.0)];
        trades.push(Trade {
            symbol: "GBPUSD".into(),
            ..trade(3, -75.0)
        });
        let p = pair_statistics(&trades);
        assert_eq!(p.best_pair.as_deref(), Some("EURUSD"));
        assert_eq!(p.worst_pair.as_deref(), Some("GBPUSD"));
        assert_eq!(p.pairs[0].total_pnl, 150.0);
    }
}
