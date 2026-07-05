//! Visual dashboard rendering. Every analysis is a chart-first Obsidian
//! note: KPI cards up top, then Obsidian Charts blocks generated entirely
//! by Rust (see [`crate::charts`]), each followed by a concise AI
//! interpretation placeholder.
//!
//! The top-level section order is fixed and must never change:
//! Trading Period → Executive Summary → Performance → Risk → Behavior →
//! Pair Performance → Session Performance → Strategy Performance →
//! Historical Comparison → AI Coaching → Action Plan.
//!
//! The AI layer only fills `<!-- ai:begin:NAME -->` blocks with short
//! interpretations of the charts — it never writes chart syntax, tables
//! or numbers of its own.

use crate::ai::{AiExchange, SessionStats};
use crate::behavior::Severity;
use crate::charts::{comparison_bar, snapshot_charts, ChartSpec, Series};

/// Marker wrapping a section the AI must complete. The AI replaces the whole
/// block between `<!-- ai:begin:NAME -->` and `<!-- ai:end:NAME -->`.
pub fn ai_placeholder(name: &str, instruction: &str) -> String {
    format!(
        "<!-- ai:begin:{name} -->\n> [!quote] Coach\n> _{instruction}_\n<!-- ai:end:{name} -->\n"
    )
}

fn fmt_money(v: f64) -> String {
    format!("{v:.2}")
}

fn fmt_opt(v: Option<f64>) -> String {
    v.map(|x| format!("{x:.2}"))
        .unwrap_or_else(|| "n/a".to_string())
}

fn fmt_pct(v: f64) -> String {
    format!("{v:.1}%")
}

fn severity_label(s: &Severity) -> &'static str {
    match s {
        Severity::None => "—",
        Severity::Low => "Low",
        Severity::Medium => "Medium",
        Severity::High => "High",
    }
}

fn session_display(key: &str) -> &'static str {
    match key {
        "asia" => "Asia",
        "london" => "London",
        "overlap" => "Overlap",
        "new_york" => "New York",
        _ => "Other",
    }
}

struct Doc(String);

impl Doc {
    fn new() -> Self {
        Doc(String::with_capacity(24 * 1024))
    }
    fn line(&mut self, s: impl AsRef<str>) {
        self.0.push_str(s.as_ref());
        self.0.push('\n');
    }
    fn blank(&mut self) {
        self.0.push('\n');
    }
    fn chart(&mut self, heading: &str, spec: &ChartSpec) {
        self.line(format!("### {heading}"));
        self.blank();
        self.0.push_str(&spec.render());
        self.blank();
    }
    fn ai(&mut self, name: &str, instruction: &str) {
        self.0.push_str(&ai_placeholder(name, instruction));
        self.blank();
    }
}

/// Render the full dashboard report. `stamp` is the vault filename stem.
pub fn render(exchange: &AiExchange, stamp: &str) -> String {
    let mut doc = Doc::new();
    let ts = &exchange.time_series;

    doc.line("# Trading Dashboard");
    doc.blank();

    // --- Trading Period -----------------------------------------------------
    doc.line("## Trading Period");
    doc.blank();
    let p = &exchange.trading_period;
    let m = &exchange.metadata;
    doc.line(format!(
        "**{} → {}** · {} calendar days · {} trading days · {} trades",
        p.first_trade_date,
        p.last_trade_date,
        p.total_calendar_days,
        p.total_trading_days,
        exchange.performance.total_trades
    ));
    doc.blank();
    doc.line(format!(
        "[[{stamp}]] · generated {} · `{}` · engine v{} (schema v{}) · {} rows skipped",
        m.generated_at, m.source_file, m.engine_version, m.schema_version, m.skipped_rows
    ));
    doc.blank();

    // --- Executive KPI Summary ------------------------------------------------
    doc.line("## Executive Summary");
    doc.blank();
    let perf = &exchange.performance;
    let scores = &exchange.scores;
    doc.line("| Total PnL | Win Rate | Profit Factor | Average RR |");
    doc.line("| :---: | :---: | :---: | :---: |");
    doc.line(format!(
        "| **{}** | **{}** | **{}** | **{}** |",
        fmt_money(perf.total_pnl),
        fmt_pct(perf.win_rate),
        fmt_opt(perf.profit_factor),
        fmt_opt(perf.average_rr)
    ));
    doc.blank();
    doc.line("| Discipline | Consistency | Emotional Control | Max Daily Drawdown |");
    doc.line("| :---: | :---: | :---: | :---: |");
    doc.line(format!(
        "| **{:.0} / 100** | **{:.0} / 100** | **{:.0} / 100** | **{}** |",
        scores.discipline.score,
        scores.consistency.score,
        scores.emotional_control.score,
        fmt_money(exchange.daily_statistics.max_daily_drawdown)
    ));
    doc.blank();
    if !exchange.summary.detected_behaviors.is_empty() {
        doc.line(format!(
            "> [!warning] Detected behaviors: {}",
            exchange
                .summary
                .detected_behaviors
                .iter()
                .map(|b| b.replace('_', " "))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        doc.blank();
    }
    doc.ai(
        "executive_summary",
        "2-3 sentences: overall state, the most important pattern on this dashboard, the #1 priority. Cite only numbers from the exchange JSON.",
    );

    // --- Performance Charts ----------------------------------------------------
    doc.line("## Performance");
    doc.blank();
    if ts.downsampled {
        doc.line(format!(
            "_Curves downsampled to at most {} points for rendering; totals are exact._",
            ts.max_points
        ));
        doc.blank();
    }

    let trade_labels: Vec<String> = ts.trades.iter().map(|t| t.n.to_string()).collect();
    doc.chart(
        "Equity Curve",
        &ChartSpec::line(
            trade_labels.clone(),
            vec![Series::new(
                "Cumulative PnL",
                ts.trades.iter().map(|t| t.cumulative_pnl).collect(),
            )],
        )
        .fill(true)
        .y_title("Cumulative PnL")
        .x_title("Trade #"),
    );

    doc.chart(
        "Daily PnL",
        &ChartSpec::line(
            ts.daily.iter().map(|d| d.date.clone()).collect(),
            vec![Series::new(
                "Daily PnL",
                ts.daily.iter().map(|d| d.pnl).collect(),
            )],
        )
        .y_title("PnL")
        .x_title("Date"),
    );

    let weekly = &exchange.weekly_statistics;
    doc.chart(
        "Weekly PnL",
        &ChartSpec::bar(
            weekly.weeks.iter().map(|w| w.week.clone()).collect(),
            vec![Series::new(
                "Weekly PnL",
                weekly.weeks.iter().map(|w| w.pnl).collect(),
            )],
        )
        .y_title("PnL")
        .x_title("Week"),
    );

    let monthly = &exchange.monthly_statistics;
    doc.chart(
        "Monthly PnL",
        &ChartSpec::bar(
            monthly.months.iter().map(|mo| mo.month.clone()).collect(),
            vec![Series::new(
                "Monthly PnL",
                monthly.months.iter().map(|mo| mo.pnl).collect(),
            )],
        )
        .y_title("PnL")
        .x_title("Month"),
    );

    let daily = &exchange.daily_statistics;
    doc.chart(
        "Winning vs Losing Days",
        &ChartSpec::bar(
            vec!["Winning".into(), "Losing".into(), "Break-even".into()],
            vec![Series::new(
                "Days",
                vec![
                    daily.positive_days as f64,
                    daily.negative_days as f64,
                    daily.flat_days as f64,
                ],
            )],
        )
        .y_title("Days"),
    );
    doc.ai(
        "performance",
        "2-3 sentences on the equity curve shape and PnL rhythm (daily/weekly/monthly trend, best vs worst periods). Reference what the charts show.",
    );

    // --- Risk Charts -------------------------------------------------------------
    doc.line("## Risk");
    doc.blank();

    let max_dd = ts.trades.iter().map(|t| t.drawdown).fold(0.0f64, f64::max);
    doc.chart(
        "Drawdown",
        &ChartSpec::line(
            trade_labels.clone(),
            vec![
                Series::new("Drawdown", ts.trades.iter().map(|t| t.drawdown).collect()),
                Series::new("Maximum", ts.trades.iter().map(|_| max_dd).collect()),
            ],
        )
        .begin_at_zero(true)
        .y_title("Distance below equity peak")
        .x_title("Trade #"),
    );

    let risk_points: Vec<&crate::ai::TradePoint> =
        ts.trades.iter().filter(|t| t.risk.is_some()).collect();
    if risk_points.is_empty() {
        doc.line("### Risk per Trade");
        doc.blank();
        doc.line("_No risk data in this journal._");
        doc.blank();
    } else {
        doc.chart(
            "Risk per Trade",
            &ChartSpec::line(
                risk_points.iter().map(|t| t.n.to_string()).collect(),
                vec![Series::new(
                    "Risk",
                    risk_points.iter().map(|t| t.risk.unwrap()).collect(),
                )],
            )
            .begin_at_zero(true)
            .y_title("Risk")
            .x_title("Trade #"),
        );
    }

    let rr_points: Vec<&crate::ai::TradePoint> = ts
        .trades
        .iter()
        .filter(|t| t.rolling_rr.is_some())
        .collect();
    if rr_points.is_empty() {
        doc.line("### Average RR Over Time");
        doc.blank();
        doc.line("_No RR data in this journal._");
        doc.blank();
    } else {
        doc.chart(
            "Average RR Over Time",
            &ChartSpec::line(
                rr_points.iter().map(|t| t.n.to_string()).collect(),
                vec![Series::new(
                    "Rolling RR (10 trades)",
                    rr_points.iter().map(|t| t.rolling_rr.unwrap()).collect(),
                )],
            )
            .y_title("Realized RR")
            .x_title("Trade #"),
        );
    }
    doc.ai(
        "risk",
        "2-3 sentences on drawdown depth/recovery, risk sizing stability and RR development visible in these charts.",
    );

    // --- Behavior Charts -----------------------------------------------------------
    doc.line("## Behavior");
    doc.blank();
    doc.line("| Pattern | Detected | Severity | Confidence |");
    doc.line("| --- | --- | --- | --- |");
    for flag in &exchange.behavioral_flags {
        let detected = if !flag.data_available {
            "no data"
        } else if flag.detected {
            "**yes**"
        } else {
            "no"
        };
        doc.line(format!(
            "| {} | {} | {} | {:.0}% |",
            flag.flag.replace('_', " "),
            detected,
            severity_label(&flag.severity),
            flag.confidence * 100.0
        ));
    }
    doc.blank();

    doc.chart(
        "Streaks",
        &ChartSpec::line(
            trade_labels,
            vec![Series::new(
                "Streak (+wins / -losses)",
                ts.trades.iter().map(|t| t.streak as f64).collect(),
            )],
        )
        .y_title("Consecutive wins / losses")
        .x_title("Trade #"),
    );

    let history_charts = snapshot_charts(&exchange.historical_comparison.series);
    if let Some(spec) = &history_charts.scores {
        doc.chart("Score Development", spec);
    }
    if let Some(spec) = &history_charts.win_rate {
        doc.chart("Win Rate History", spec);
    }
    if let Some(spec) = &history_charts.profit_factor {
        doc.chart("Profit Factor History", spec);
    }
    if let Some(spec) = &history_charts.average_rr {
        doc.chart("Average RR History", spec);
    }
    if history_charts.scores.is_none() {
        doc.line("_Development charts appear from the second analysis onward._");
        doc.blank();
    }
    doc.ai(
        "behavior",
        "2-3 sentences interpreting the detected flags (use behavioral_flags metrics) and the streak/score-development charts. Hedge on low-confidence flags; skip flags without data.",
    );

    // --- Pair Performance -----------------------------------------------------------
    doc.line("## Pair Performance");
    doc.blank();
    let pairs = &exchange.pair_statistics.pairs;
    let pair_chart = |values: Vec<(&String, f64)>, title: &str| {
        let mut sorted = values;
        sorted.sort_by(|a, b| b.1.total_cmp(&a.1));
        ChartSpec::horizontal_bar(
            sorted.iter().map(|(p, _)| (*p).clone()).collect(),
            vec![Series::new(title, sorted.iter().map(|(_, v)| *v).collect())],
        )
    };
    doc.chart(
        "PnL per Pair",
        &pair_chart(
            pairs.iter().map(|p| (&p.pair, p.total_pnl)).collect(),
            "PnL",
        ),
    );
    doc.chart(
        "Win Rate per Pair",
        &pair_chart(
            pairs.iter().map(|p| (&p.pair, p.win_rate)).collect(),
            "Win Rate %",
        ),
    );
    let rr_pairs: Vec<(&String, f64)> = pairs
        .iter()
        .filter_map(|p| p.average_rr.map(|rr| (&p.pair, rr)))
        .collect();
    if !rr_pairs.is_empty() {
        doc.chart("Average RR per Pair", &pair_chart(rr_pairs, "Average RR"));
    }
    doc.chart(
        "Trades per Pair",
        &pair_chart(
            pairs
                .iter()
                .map(|p| (&p.pair, p.total_trades as f64))
                .collect(),
            "Trades",
        ),
    );
    doc.ai(
        "pairs",
        "2-3 sentences: strongest and weakest markets across PnL, win rate and RR, and what to focus on. Only rankings visible in these charts.",
    );

    // --- Session Performance ------------------------------------------------------------
    doc.line("## Session Performance");
    doc.blank();
    let sess = &exchange.session_statistics;
    let active: Vec<&SessionStats> = sess
        .sessions
        .iter()
        .filter(|s| s.total_trades > 0)
        .collect();
    if sess.data_available && !active.is_empty() {
        let labels: Vec<String> = active
            .iter()
            .map(|s| session_display(&s.session).to_string())
            .collect();
        doc.chart(
            "PnL per Session",
            &ChartSpec::bar(
                labels.clone(),
                vec![Series::new(
                    "PnL",
                    active.iter().map(|s| s.total_pnl).collect(),
                )],
            )
            .y_title("PnL"),
        );
        doc.chart(
            "Win Rate per Session",
            &ChartSpec::bar(
                labels.clone(),
                vec![Series::new(
                    "Win Rate %",
                    active.iter().map(|s| s.win_rate).collect(),
                )],
            )
            .y_title("Win rate %"),
        );
        let rr_sessions: Vec<(&SessionStats, f64)> = active
            .iter()
            .filter_map(|s| s.average_rr.map(|rr| (*s, rr)))
            .collect();
        if !rr_sessions.is_empty() {
            doc.chart(
                "Average RR per Session",
                &ChartSpec::bar(
                    rr_sessions
                        .iter()
                        .map(|(s, _)| session_display(&s.session).to_string())
                        .collect(),
                    vec![Series::new(
                        "Average RR",
                        rr_sessions.iter().map(|(_, rr)| *rr).collect(),
                    )],
                )
                .y_title("Average RR"),
            );
        }
        doc.ai(
            "sessions",
            "1-2 sentences: which session performs best/worst and whether trading hours should shift.",
        );
    } else {
        doc.line("_No session or trade-time data in this journal._");
        doc.blank();
    }

    // --- Strategy Performance --------------------------------------------------------------
    doc.line("## Strategy Performance");
    doc.blank();
    let strat = &exchange.strategy_statistics;
    if strat.data_available {
        let strat_chart = |values: Vec<(&String, f64)>, title: &str| {
            let mut sorted = values;
            sorted.sort_by(|a, b| b.1.total_cmp(&a.1));
            ChartSpec::horizontal_bar(
                sorted.iter().map(|(s, _)| (*s).clone()).collect(),
                vec![Series::new(title, sorted.iter().map(|(_, v)| *v).collect())],
            )
        };
        let strategies = &strat.strategies;
        doc.chart(
            "PnL per Strategy",
            &strat_chart(
                strategies
                    .iter()
                    .map(|s| (&s.strategy, s.total_pnl))
                    .collect(),
                "PnL",
            ),
        );
        doc.chart(
            "Win Rate per Strategy",
            &strat_chart(
                strategies
                    .iter()
                    .map(|s| (&s.strategy, s.win_rate))
                    .collect(),
                "Win Rate %",
            ),
        );
        let rr_strats: Vec<(&String, f64)> = strategies
            .iter()
            .filter_map(|s| s.average_rr.map(|rr| (&s.strategy, rr)))
            .collect();
        if !rr_strats.is_empty() {
            doc.chart(
                "Average RR per Strategy",
                &strat_chart(rr_strats, "Average RR"),
            );
        }
        doc.chart(
            "Trades per Strategy",
            &strat_chart(
                strategies
                    .iter()
                    .map(|s| (&s.strategy, s.total_trades as f64))
                    .collect(),
                "Trades",
            ),
        );
        doc.ai(
            "strategies",
            "1-2 sentences: which setup earns its place and which is costing money, based on these rankings.",
        );
    } else {
        doc.line("_No strategy data in this journal._");
        doc.blank();
    }

    // --- Historical Comparison ------------------------------------------------------------
    doc.line("## Historical Comparison");
    doc.blank();
    let h = &exchange.historical_comparison;
    if h.previous_analyses == 0 {
        doc.line("_First analysis on record — comparisons appear from the next analysis onward._");
        doc.blank();
    } else {
        let prior = &h.series[..h.series.len().saturating_sub(1)];
        let comparisons: Vec<(&str, f64, Vec<f64>, bool)> = vec![
            (
                "Win Rate %",
                perf.win_rate,
                prior.iter().map(|s| s.win_rate).collect(),
                true,
            ),
            (
                "Profit Factor",
                perf.profit_factor.unwrap_or(0.0),
                prior.iter().filter_map(|s| s.profit_factor).collect(),
                true,
            ),
            (
                "Average RR",
                perf.average_rr.unwrap_or(0.0),
                prior.iter().filter_map(|s| s.average_rr).collect(),
                true,
            ),
            (
                "Max Daily Drawdown",
                daily.max_daily_drawdown,
                prior.iter().map(|s| s.max_daily_drawdown).collect(),
                false,
            ),
        ];
        for (metric, current, previous_values, higher_is_better) in comparisons {
            if let Some(spec) = comparison_bar(metric, current, &previous_values, higher_is_better)
            {
                doc.chart(&format!("{metric}: Current vs History"), &spec);
            }
        }
        if let Some(deltas) = &h.deltas {
            doc.line("| Metric | Change vs previous |");
            doc.line("| --- | --- |");
            doc.line(format!("| Total PnL | {:+.2} |", deltas.total_pnl_change));
            doc.line(format!("| Win rate | {:+.2} pp |", deltas.win_rate_change));
            if let Some(pf) = deltas.profit_factor_change {
                doc.line(format!("| Profit factor | {pf:+.2} |"));
            }
            if let Some(rr) = deltas.average_rr_change {
                doc.line(format!("| Average RR | {rr:+.2} |"));
            }
            doc.line(format!(
                "| Max daily drawdown | {:+.2} |",
                deltas.max_daily_drawdown_change
            ));
            doc.line(format!(
                "| Discipline | {:+.2} |",
                deltas.discipline_score_change
            ));
            doc.line(format!(
                "| Consistency | {:+.2} |",
                deltas.consistency_score_change
            ));
            doc.line(format!(
                "| Emotional control | {:+.2} |",
                deltas.emotional_control_score_change
            ));
            doc.blank();
        }
    }
    doc.ai(
        "historical",
        "2-3 sentences on development across analyses: what is trending up, what is regressing, streaks of improvement (e.g. 'third consecutive analysis with a higher discipline score'). Use historical_comparison only; skip if this is the first analysis.",
    );

    // --- AI Coaching ---------------------------------------------------------------------
    doc.line("## AI Coaching");
    doc.blank();
    doc.ai(
        "coaching",
        "Max 5 short bullets: evidence-backed strengths and weaknesses, most impactful first. Each bullet cites a number or chart from this dashboard. Focus on trends over isolated stats.",
    );

    // --- Action Plan -----------------------------------------------------------------------
    doc.line("## Action Plan");
    doc.blank();
    doc.ai(
        "action_plan",
        "Max 3 concrete actions targeting the biggest weakness/behavior, each with a measurable success metric from the exchange JSON to check next analysis.",
    );

    doc.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::build_exchange;
    use crate::config::Config;
    use crate::history::{HistoricalComparison, HistoryFile, Snapshot};
    use crate::parser::{DetectedFormat, ParseReport};
    use crate::shared::Trade;
    use chrono::NaiveDate;
    use std::path::Path;

    fn sample_exchange() -> AiExchange {
        let trades: Vec<Trade> = (1..=6)
            .map(|d| Trade {
                index: 0,
                date: NaiveDate::from_ymd_opt(2026, 1, d).unwrap(),
                time: None,
                symbol: if d % 2 == 0 {
                    "GBPUSD".into()
                } else {
                    "EURUSD".into()
                },
                direction: None,
                entry: None,
                exit: None,
                position_size: None,
                risk: Some(100.0),
                reward: None,
                rr: Some(if d % 2 == 0 { -1.0 } else { 2.0 }),
                pnl: if d % 2 == 0 { -100.0 } else { 200.0 },
                fees: None,
                strategy: Some("Breakout".into()),
                session: None,
                notes: None,
            })
            .collect();
        let report = ParseReport {
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
        };
        build_exchange(
            &report,
            Path::new("t.csv"),
            &Config::default(),
            "2026-01-06T10:00:00".into(),
            None,
        )
        .unwrap()
    }

    #[test]
    fn dashboard_section_order_is_fixed() {
        let md = render(&sample_exchange(), "2026-01-06_10-00");
        let sections = [
            "# Trading Dashboard",
            "## Trading Period",
            "## Executive Summary",
            "## Performance",
            "## Risk",
            "## Behavior",
            "## Pair Performance",
            "## Session Performance",
            "## Strategy Performance",
            "## Historical Comparison",
            "## AI Coaching",
            "## Action Plan",
        ];
        let mut last = 0;
        for section in sections {
            let pos = md
                .find(section)
                .unwrap_or_else(|| panic!("missing section {section}"));
            assert!(pos >= last, "section {section} out of order");
            last = pos;
        }
    }

    #[test]
    fn charts_are_generated_by_rust() {
        let md = render(&sample_exchange(), "stamp");
        // chart-first: the core visualizations exist as valid chart blocks
        for heading in [
            "### Equity Curve",
            "### Daily PnL",
            "### Weekly PnL",
            "### Monthly PnL",
            "### Winning vs Losing Days",
            "### Drawdown",
            "### Risk per Trade",
            "### Average RR Over Time",
            "### Streaks",
            "### PnL per Pair",
            "### Win Rate per Pair",
            "### Trades per Pair",
            "### PnL per Strategy",
        ] {
            assert!(md.contains(heading), "missing chart heading {heading}");
        }
        assert!(
            md.matches("```chart").count() >= 12,
            "expected at least 12 chart blocks"
        );
        // every block is closed
        assert_eq!(
            md.matches("```chart").count() * 2,
            md.matches("```").count()
        );
    }

    #[test]
    fn weekly_and_monthly_pnl_are_bar_charts() {
        let md = render(&sample_exchange(), "stamp");
        let chart_type_of = |heading: &str| {
            let start = md
                .find(heading)
                .unwrap_or_else(|| panic!("missing {heading}"));
            let block = &md[start..md[start..].find("```\n").map(|e| start + e).unwrap()];
            block
                .lines()
                .find(|l| l.starts_with("type: "))
                .unwrap()
                .to_string()
        };
        assert_eq!(chart_type_of("### Weekly PnL"), "type: bar");
        assert_eq!(chart_type_of("### Monthly PnL"), "type: bar");
        assert_eq!(chart_type_of("### Daily PnL"), "type: line");
        assert_eq!(chart_type_of("### Equity Curve"), "type: line");
    }

    #[test]
    fn equity_curve_uses_time_series_values() {
        let md = render(&sample_exchange(), "stamp");
        // cumulative: 200, 100, 300, 200, 400, 300
        assert!(md.contains("data: [200, 100, 300, 200, 400, 300]"));
    }

    #[test]
    fn ai_placeholders_are_concise_interpretation_blocks() {
        let md = render(&sample_exchange(), "stamp");
        for name in [
            "executive_summary",
            "performance",
            "risk",
            "behavior",
            "pairs",
            "strategies",
            "historical",
            "coaching",
            "action_plan",
        ] {
            assert!(
                md.contains(&format!("<!-- ai:begin:{name} -->")),
                "missing placeholder {name}"
            );
            assert!(
                md.contains(&format!("<!-- ai:end:{name} -->")),
                "missing end marker {name}"
            );
        }
    }

    #[test]
    fn history_charts_appear_with_series() {
        let mut exchange = sample_exchange();
        let older = Snapshot {
            generated_at: "2026-01-01T09:00:00".into(),
            source_file: "t.csv".into(),
            report_file: None,
            total_trades: 5,
            total_pnl: 100.0,
            win_rate: 40.0,
            profit_factor: Some(1.2),
            average_rr: Some(1.0),
            average_trade: 20.0,
            average_weekly_pnl: 50.0,
            average_monthly_pnl: 100.0,
            max_daily_drawdown: 250.0,
            discipline_score: 55.0,
            consistency_score: 50.0,
            emotional_control_score: 60.0,
            best_pair: None,
            best_pair_pnl: None,
        };
        let history = HistoryFile {
            schema_version: "1.1.0".into(),
            snapshots: vec![older],
        };
        let current = crate::history::snapshot_from(&exchange, None);
        exchange.historical_comparison = crate::history::compare(&current, &history);

        let md = render(&exchange, "stamp");
        assert!(md.contains("### Score Development"));
        assert!(md.contains("### Win Rate History"));
        assert!(md.contains("### Profit Factor History"));
        assert!(md.contains("### Win Rate %: Current vs History"));
        assert!(md.contains("Hist. Average"));
    }

    #[test]
    fn first_analysis_skips_history_charts() {
        let mut exchange = sample_exchange();
        exchange.historical_comparison = HistoricalComparison::empty();
        let md = render(&exchange, "stamp");
        assert!(!md.contains("### Score Development"));
        assert!(md.contains("_Development charts appear from the second analysis onward._"));
        assert!(md.contains("_First analysis on record"));
    }
}
