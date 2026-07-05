//! End-to-end: CSV fixture -> exchange JSON -> report -> vault -> history.

use std::path::{Path, PathBuf};

use trade_analyst::analyzer;
use trade_analyst::config::{Config, VaultConfig};
use trade_analyst::history;
use trade_analyst::obsidian::Vault;
use trade_analyst::parser;
use trade_analyst::reports;

/// Journal-style export: every canonical field populated.
fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/journal_trades.csv")
}

/// Real-world MetaTrader-style export: datetime Open/Close columns,
/// duplicate Price headers, Type/Volume/Profit/Commissions naming.
fn mt_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample_trades.csv")
}

fn temp_vault(name: &str) -> VaultConfig {
    let path = std::env::temp_dir()
        .join("trade-analyst-integration")
        .join(name);
    let _ = std::fs::remove_dir_all(&path);
    VaultConfig {
        path,
        ..VaultConfig::default()
    }
}

#[test]
fn full_pipeline() {
    let cfg = Config {
        vault: temp_vault("full"),
        ..Config::default()
    };

    // 1. parse
    let report = parser::parse_csv(&fixture(), &cfg.csv).expect("parse fixture");
    assert_eq!(report.trades.len(), 40);
    assert_eq!(report.skipped_rows, 0);
    assert_eq!(report.detected.delimiter, ',');
    assert_eq!(report.detected.date_format, "%Y-%m-%d");

    // 2. build exchange
    let exchange = analyzer::build_exchange(
        &report,
        &fixture(),
        &cfg,
        "2026-03-01T10:00:00".to_string(),
        None,
    )
    .expect("build exchange");

    // deterministic figures straight from the fixture
    assert_eq!(exchange.performance.total_trades, 40);
    assert_eq!(exchange.trading_period.first_trade_date, "2026-01-05");
    assert_eq!(exchange.trading_period.last_trade_date, "2026-02-23");
    assert!(exchange.performance.total_pnl > 0.0);
    assert!(exchange.performance.win_rate > 50.0);
    assert!(exchange.strategy_statistics.data_available);
    assert!(exchange.session_statistics.data_available);
    assert_eq!(exchange.behavioral_flags.len(), 10);
    assert!(exchange.scores.overall > 0.0 && exchange.scores.overall <= 100.0);

    // the fixture contains a revenge/tilt day (2026-01-06) with escalation
    let revenge = exchange
        .behavioral_flags
        .iter()
        .find(|f| f.flag == "revenge_trading")
        .unwrap();
    assert!(revenge.data_available);

    // time_series feeds the charts deterministically
    let ts = &exchange.time_series;
    assert!(!ts.downsampled);
    assert_eq!(ts.trades.len(), 40);
    assert_eq!(
        ts.trades.last().unwrap().cumulative_pnl,
        exchange.performance.total_pnl
    );
    assert_eq!(ts.daily.len(), exchange.daily_statistics.trading_days);

    // 3. schema contract: exact top-level keys in order
    let json = serde_json::to_value(&exchange).unwrap();
    let keys: Vec<&String> = json.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        vec![
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
            "time_series"
        ]
    );

    // 4. render + vault
    let vault = Vault::new(&cfg.vault);
    vault.ensure_layout().unwrap();
    let markdown = reports::render(&exchange, "2026-03-01_10-00");
    assert!(
        markdown.matches("```chart").count() >= 12,
        "dashboard must be chart-first"
    );
    let report_path = vault.write_analysis("2026-03-01_10-00", &markdown).unwrap();
    assert!(report_path.exists());

    // 5. history: first append, then a second analysis sees the first
    let updated =
        analyzer::append_to_history(&exchange, Some("2026-03-01_10-00.md".into()), &cfg).unwrap();
    vault.update_dashboard(&updated).unwrap();
    let dashboard = std::fs::read_to_string(vault.dashboard_file()).unwrap();
    assert!(dashboard.contains("[[2026-03-01_10-00]]"));

    let comparison = analyzer::compare_against_vault_history(&exchange, &cfg).unwrap();
    assert_eq!(comparison.previous_analyses, 1);
    assert!(comparison.previous.is_some());
    assert!(comparison.deltas.is_some());
    // series = stored snapshot + current
    assert_eq!(comparison.series.len(), 2);
    let deltas = comparison.deltas.unwrap();
    // identical analysis -> zero deltas
    assert_eq!(deltas.total_pnl_change, 0.0);
    assert_eq!(deltas.win_rate_change, 0.0);

    let history_file = history::history_path(&cfg.vault.path, &cfg.vault.assets_dir);
    assert!(history_file.exists());

    // 6. second analysis: report + dashboard now carry development charts
    let mut exchange2 = analyzer::build_exchange(
        &report,
        &fixture(),
        &cfg,
        "2026-03-08T10:00:00".to_string(),
        None,
    )
    .unwrap();
    exchange2.historical_comparison =
        analyzer::compare_against_vault_history(&exchange2, &cfg).unwrap();
    let markdown2 = reports::render(&exchange2, "2026-03-08_10-00");
    assert!(markdown2.contains("### Score Development"));
    assert!(markdown2.contains("### Win Rate %: Current vs History"));
    vault
        .write_analysis("2026-03-08_10-00", &markdown2)
        .unwrap();
    let updated =
        analyzer::append_to_history(&exchange2, Some("2026-03-08_10-00.md".into()), &cfg).unwrap();
    vault.update_dashboard(&updated).unwrap();
    let dashboard = std::fs::read_to_string(vault.dashboard_file()).unwrap();
    assert!(dashboard.contains("## Score Development"));
    assert!(dashboard.matches("```chart").count() >= 6);
}

#[test]
fn mt_style_export_parses_without_any_config() {
    // This is the format community members get from MT4/MT5-style brokers.
    let cfg = Config {
        vault: temp_vault("mt"),
        ..Config::default()
    };
    let report = parser::parse_csv(&mt_fixture(), &cfg.csv).expect("parse MT export");
    assert_eq!(report.skipped_rows, 0, "warnings: {:?}", report.warnings);
    assert_eq!(report.trades.len(), 30);
    assert_eq!(report.detected.date_format, "%Y-%m-%d %H:%M:%S");

    // datetime "Open" column became date + time, duplicate Price columns
    // became entry/exit, Commissions became fees (sign normalized)
    let t = report
        .trades
        .iter()
        .find(|t| t.pnl == 2553.2)
        .expect("known trade");
    assert_eq!(t.date.to_string(), "2024-09-06");
    assert!(t.time.is_some());
    assert_eq!(t.symbol, "USDJPY");
    assert_eq!(t.entry, Some(142.885));
    assert_eq!(t.exit, Some(142.958));
    assert_eq!(t.position_size, Some(50.0));
    assert_eq!(t.fees, Some(150.0));

    // full pipeline works on it: exchange + dashboard render
    let exchange =
        analyzer::build_exchange(&report, &mt_fixture(), &cfg, "t".into(), None).unwrap();
    assert_eq!(exchange.performance.total_trades, 30);
    assert_eq!(exchange.trading_period.first_trade_date, "2024-08-26");
    assert_eq!(exchange.trading_period.last_trade_date, "2024-09-06");
    assert!(exchange.session_statistics.data_available); // inferred from times
    assert!(!exchange.strategy_statistics.data_available);
    let md = reports::render(&exchange, "stamp");
    assert!(md.contains("### Equity Curve"));
    assert!(md.contains("_No strategy data in this journal._"));
}

#[test]
fn inspect_provides_the_ai_mapping_contract() {
    let cfg = Config::default();
    let value = parser::inspect_csv(&mt_fixture(), &cfg.csv, 5).unwrap();
    assert_eq!(value["detection"]["status"], "ok");
    assert_eq!(value["detection"]["trial_parse"]["trades"], 30);
    assert_eq!(value["detection"]["mapped"]["date"], "Open");
    assert_eq!(value["detection"]["mapped"]["pnl"], "Profit");
    assert_eq!(value["sample_rows"].as_array().unwrap().len(), 5);
    assert!(value["mapping_file_howto"]["path"]
        .as_str()
        .unwrap()
        .ends_with("sample_trades.csv.mapping.toml"));
}

#[test]
fn report_has_fixed_sections_and_ai_markers() {
    let cfg = Config {
        vault: temp_vault("report"),
        ..Config::default()
    };
    let report = parser::parse_csv(&fixture(), &cfg.csv).unwrap();
    let exchange = analyzer::build_exchange(&report, &fixture(), &cfg, "t".into(), None).unwrap();
    let md = reports::render(&exchange, "stamp");

    for section in [
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
    ] {
        assert!(md.contains(section), "missing section: {section}");
    }
    assert!(md.contains("<!-- ai:begin:executive_summary -->"));
    // fixture has sessions + strategies -> their chart groups must render
    assert!(md.contains("### PnL per Session"));
    assert!(md.contains("### PnL per Strategy"));
    assert!(md.contains("### Average RR per Session"));
}
