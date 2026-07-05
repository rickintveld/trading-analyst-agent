//! Obsidian Charts plugin output — the reusable chart component engine.
//!
//! Every chart in every report and dashboard is a [`ChartSpec`] rendered to a
//! fenced ```chart block (YAML understood by the obsidian-charts plugin,
//! which drives Chart.js). Chart definitions are generated deterministically
//! by Rust; the AI layer never writes chart syntax.
//!
//! Adding a new visualization = building a new `ChartSpec` from exchange
//! data. Existing charts and reports are never touched.

use crate::history::Snapshot;
use crate::shared::round2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartType {
    Line,
    Bar,
}

impl ChartType {
    fn label(&self) -> &'static str {
        match self {
            ChartType::Line => "line",
            ChartType::Bar => "bar",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Series {
    pub title: String,
    pub data: Vec<f64>,
}

impl Series {
    pub fn new(title: impl Into<String>, data: Vec<f64>) -> Self {
        Self {
            title: title.into(),
            data: data.into_iter().map(round2).collect(),
        }
    }
}

/// A deterministic chart definition, rendered to obsidian-charts YAML.
#[derive(Debug, Clone)]
pub struct ChartSpec {
    chart_type: ChartType,
    labels: Vec<String>,
    series: Vec<Series>,
    /// `indexAxis: y` — horizontal bars.
    horizontal: bool,
    begin_at_zero: bool,
    fill: bool,
    tension: f64,
    legend: bool,
    y_title: Option<String>,
    x_title: Option<String>,
    width: Option<String>,
}

impl ChartSpec {
    pub fn line(labels: Vec<String>, series: Vec<Series>) -> Self {
        let legend = series.len() > 1;
        Self {
            chart_type: ChartType::Line,
            labels,
            series,
            horizontal: false,
            begin_at_zero: false,
            fill: false,
            tension: 0.2,
            legend,
            y_title: None,
            x_title: None,
            width: None,
        }
    }

    pub fn bar(labels: Vec<String>, series: Vec<Series>) -> Self {
        let legend = series.len() > 1;
        Self {
            chart_type: ChartType::Bar,
            labels,
            series,
            horizontal: false,
            begin_at_zero: true,
            fill: false,
            tension: 0.0,
            legend,
            y_title: None,
            x_title: None,
            width: None,
        }
    }

    /// Horizontal bar chart — used for per-pair / per-strategy rankings.
    pub fn horizontal_bar(labels: Vec<String>, series: Vec<Series>) -> Self {
        Self {
            horizontal: true,
            ..Self::bar(labels, series)
        }
    }

    pub fn y_title(mut self, title: impl Into<String>) -> Self {
        self.y_title = Some(title.into());
        self
    }

    pub fn x_title(mut self, title: impl Into<String>) -> Self {
        self.x_title = Some(title.into());
        self
    }

    pub fn begin_at_zero(mut self, v: bool) -> Self {
        self.begin_at_zero = v;
        self
    }

    pub fn fill(mut self, v: bool) -> Self {
        self.fill = v;
        self
    }

    pub fn width(mut self, w: impl Into<String>) -> Self {
        self.width = Some(w.into());
        self
    }

    /// Render the fenced ```chart block.
    pub fn render(&self) -> String {
        let mut out = String::with_capacity(512);
        out.push_str("```chart\n");
        out.push_str(&format!("type: {}\n", self.chart_type.label()));
        out.push_str(&format!(
            "labels: [{}]\n",
            self.labels
                .iter()
                .map(|l| yaml_quote(l))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        out.push_str("series:\n");
        for s in &self.series {
            out.push_str(&format!("  - title: {}\n", yaml_quote(&s.title)));
            out.push_str(&format!(
                "    data: [{}]\n",
                s.data
                    .iter()
                    .map(|v| fmt_num(*v))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if self.horizontal {
            out.push_str("indexAxis: y\n");
        }
        out.push_str(&format!("tension: {}\n", fmt_num(self.tension)));
        out.push_str(&format!("fill: {}\n", self.fill));
        out.push_str(&format!("beginAtZero: {}\n", self.begin_at_zero));
        out.push_str(&format!("legend: {}\n", self.legend));
        out.push_str("labelColors: false\n");
        if let Some(t) = &self.y_title {
            out.push_str(&format!("yTitle: {}\n", yaml_quote(t)));
        }
        if let Some(t) = &self.x_title {
            out.push_str(&format!("xTitle: {}\n", yaml_quote(t)));
        }
        if let Some(w) = &self.width {
            out.push_str(&format!("width: {}\n", yaml_quote(w)));
        }
        out.push_str("```\n");
        out
    }
}

fn yaml_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Compact deterministic number formatting: integers without decimals,
/// everything else with at most two.
fn fmt_num(v: f64) -> String {
    let r = round2(v);
    if r == r.trunc() && r.abs() < 1e15 {
        format!("{}", r as i64)
    } else {
        let s = format!("{r:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Short label for a snapshot timestamp: "2026-07-05T11:08:55" -> "07-05 11:08".
pub fn snapshot_label(generated_at: &str) -> String {
    if generated_at.len() >= 16 {
        generated_at[5..16].replace('T', " ")
    } else {
        generated_at.to_string()
    }
}

/// Charts over the analysis history. Every field is `None` until at least
/// two snapshots exist — a one-point line chart is noise, not signal.
#[derive(Debug, Clone, Default)]
pub struct SnapshotCharts {
    pub scores: Option<ChartSpec>,
    pub win_rate: Option<ChartSpec>,
    pub profit_factor: Option<ChartSpec>,
    pub average_rr: Option<ChartSpec>,
    pub weekly_pnl: Option<ChartSpec>,
    pub monthly_pnl: Option<ChartSpec>,
    pub max_drawdown: Option<ChartSpec>,
    pub total_pnl: Option<ChartSpec>,
}

/// Build the historical development charts shared by analysis reports and
/// the vault dashboard.
pub fn snapshot_charts(series: &[Snapshot]) -> SnapshotCharts {
    if series.len() < 2 {
        return SnapshotCharts::default();
    }
    let labels: Vec<String> = series
        .iter()
        .map(|s| snapshot_label(&s.generated_at))
        .collect();
    let line = |title: &str, data: Vec<f64>, y: &str| {
        Some(
            ChartSpec::line(labels.clone(), vec![Series::new(title, data)])
                .y_title(y)
                .x_title("Analysis"),
        )
    };

    SnapshotCharts {
        scores: Some(
            ChartSpec::line(
                labels.clone(),
                vec![
                    Series::new(
                        "Discipline",
                        series.iter().map(|s| s.discipline_score).collect(),
                    ),
                    Series::new(
                        "Consistency",
                        series.iter().map(|s| s.consistency_score).collect(),
                    ),
                    Series::new(
                        "Emotional Control",
                        series.iter().map(|s| s.emotional_control_score).collect(),
                    ),
                ],
            )
            .y_title("Score (0-100)")
            .x_title("Analysis")
            .begin_at_zero(true),
        ),
        win_rate: line(
            "Win Rate %",
            series.iter().map(|s| s.win_rate).collect(),
            "Win rate %",
        ),
        profit_factor: {
            let values: Vec<f64> = series.iter().filter_map(|s| s.profit_factor).collect();
            if values.len() == series.len() {
                line("Profit Factor", values, "Profit factor")
            } else {
                None // gaps would misalign labels; skip rather than lie
            }
        },
        average_rr: {
            let values: Vec<f64> = series.iter().filter_map(|s| s.average_rr).collect();
            if values.len() == series.len() {
                line("Average RR", values, "Average RR")
            } else {
                None
            }
        },
        weekly_pnl: line(
            "Avg Weekly PnL",
            series.iter().map(|s| s.average_weekly_pnl).collect(),
            "PnL",
        ),
        monthly_pnl: line(
            "Avg Monthly PnL",
            series.iter().map(|s| s.average_monthly_pnl).collect(),
            "PnL",
        ),
        max_drawdown: line(
            "Max Daily Drawdown",
            series.iter().map(|s| s.max_daily_drawdown).collect(),
            "Drawdown",
        ),
        total_pnl: line(
            "Total PnL",
            series.iter().map(|s| s.total_pnl).collect(),
            "PnL",
        ),
    }
}

/// "Current vs historical average / best / worst" comparison bar.
/// `higher_is_better` controls which end is labeled best/worst.
pub fn comparison_bar(
    metric: &str,
    current: f64,
    previous_values: &[f64],
    higher_is_better: bool,
) -> Option<ChartSpec> {
    if previous_values.is_empty() {
        return None;
    }
    let avg = previous_values.iter().sum::<f64>() / previous_values.len() as f64;
    let max = previous_values.iter().copied().fold(f64::MIN, f64::max);
    let min = previous_values.iter().copied().fold(f64::MAX, f64::min);
    let (best, worst) = if higher_is_better {
        (max, min)
    } else {
        (min, max)
    };
    Some(
        ChartSpec::bar(
            vec![
                "Current".into(),
                "Hist. Average".into(),
                "Hist. Best".into(),
                "Hist. Worst".into(),
            ],
            vec![Series::new(metric, vec![current, avg, best, worst])],
        )
        .y_title(metric),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(at: &str, win_rate: f64) -> Snapshot {
        Snapshot {
            generated_at: at.to_string(),
            source_file: "t.csv".into(),
            report_file: None,
            total_trades: 10,
            total_pnl: 100.0,
            win_rate,
            profit_factor: Some(2.0),
            average_rr: Some(1.5),
            average_trade: 10.0,
            average_weekly_pnl: 50.0,
            average_monthly_pnl: 100.0,
            max_daily_drawdown: 30.0,
            discipline_score: 70.0,
            consistency_score: 60.0,
            emotional_control_score: 80.0,
            best_pair: None,
            best_pair_pnl: None,
        }
    }

    #[test]
    fn renders_valid_chart_block() {
        let spec = ChartSpec::line(
            vec!["1".into(), "2".into(), "3".into()],
            vec![Series::new("Equity", vec![100.0, 250.5, 200.0])],
        )
        .y_title("Cumulative PnL")
        .x_title("Trade #");
        let md = spec.render();
        assert!(md.starts_with("```chart\n"));
        assert!(md.ends_with("```\n"));
        assert!(md.contains("type: line"));
        assert!(md.contains("labels: [\"1\", \"2\", \"3\"]"));
        assert!(md.contains("  - title: \"Equity\""));
        assert!(md.contains("    data: [100, 250.5, 200]"));
        assert!(md.contains("yTitle: \"Cumulative PnL\""));
        assert!(md.contains("xTitle: \"Trade #\""));
    }

    #[test]
    fn horizontal_bar_sets_index_axis() {
        let spec =
            ChartSpec::horizontal_bar(vec!["EURUSD".into()], vec![Series::new("PnL", vec![500.0])]);
        let md = spec.render();
        assert!(md.contains("type: bar"));
        assert!(md.contains("indexAxis: y"));
        assert!(md.contains("beginAtZero: true"));
    }

    #[test]
    fn quotes_are_escaped() {
        let spec = ChartSpec::bar(vec!["a\"b".into()], vec![Series::new("s", vec![1.0])]);
        assert!(spec.render().contains("\"a\\\"b\""));
    }

    #[test]
    fn number_formatting_is_compact() {
        assert_eq!(fmt_num(100.0), "100");
        assert_eq!(fmt_num(1.5), "1.5");
        assert_eq!(fmt_num(-0.25), "-0.25");
        assert_eq!(fmt_num(2.505), "2.51");
    }

    #[test]
    fn snapshot_charts_need_two_points() {
        let one = snapshot_charts(&[snapshot("2026-07-01T10:00:00", 50.0)]);
        assert!(one.scores.is_none());
        assert!(one.win_rate.is_none());

        let two = snapshot_charts(&[
            snapshot("2026-07-01T10:00:00", 50.0),
            snapshot("2026-07-05T11:00:00", 60.0),
        ]);
        let win_rate = two.win_rate.unwrap().render();
        assert!(win_rate.contains("labels: [\"07-01 10:00\", \"07-05 11:00\"]"));
        assert!(win_rate.contains("data: [50, 60]"));
        let scores = two.scores.unwrap().render();
        assert!(scores.contains("Discipline"));
        assert!(scores.contains("Emotional Control"));
    }

    #[test]
    fn comparison_bar_orients_best_and_worst() {
        // higher is better: best = max
        let spec = comparison_bar("Win Rate", 60.0, &[40.0, 55.0], true).unwrap();
        assert!(spec.render().contains("data: [60, 47.5, 55, 40]"));
        // lower is better (drawdown): best = min
        let spec = comparison_bar("Drawdown", 100.0, &[300.0, 200.0], false).unwrap();
        assert!(spec.render().contains("data: [100, 250, 200, 300]"));
        assert!(comparison_bar("x", 1.0, &[], true).is_none());
    }
}
