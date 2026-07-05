//! Command-line interface.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::analyzer;
use crate::config::Config;
use crate::history;
use crate::obsidian::Vault;
use crate::parser;
use crate::reports;

#[derive(Debug, Parser)]
#[command(
    name = "trade-analyzer",
    version,
    about = "Deterministic trading analytics engine + AI performance coach exchange format",
    long_about = "Parses trading journal CSVs, computes deterministic statistics, behavioral \
                  flags and scores, writes Obsidian analysis notes and emits the fixed JSON \
                  exchange format consumed by the AI coaching layer."
)]
pub struct Cli {
    /// Path to config.toml (defaults to ./config.toml when present).
    #[arg(short, long, global = true)]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Analyze a CSV journal: write the Obsidian report, exchange JSON,
    /// update the dashboard and append to history.
    Analyze {
        /// CSV file with trades.
        csv: PathBuf,
        /// Compute everything but write nothing to the vault or history.
        #[arg(long)]
        dry_run: bool,
        /// Also print the exchange JSON to stdout.
        #[arg(long)]
        json: bool,
        /// Mapping file overriding CSV auto-detection
        /// (default: <csv>.mapping.toml when present).
        #[arg(short, long)]
        mapping: Option<PathBuf>,
    },
    /// Print deterministic statistics for a CSV to stdout (no vault writes).
    Stats {
        /// CSV file with trades.
        csv: PathBuf,
        /// Mapping file overriding CSV auto-detection.
        #[arg(short, long)]
        mapping: Option<PathBuf>,
    },
    /// Emit the fixed AI exchange JSON for a CSV (no vault writes).
    ExportJson {
        /// CSV file with trades.
        csv: PathBuf,
        /// Write to a file instead of stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Mapping file overriding CSV auto-detection.
        #[arg(short, long)]
        mapping: Option<PathBuf>,
    },
    /// Emit a structured JSON sample of a CSV (headers, sample rows,
    /// detection attempt, canonical fields) for the AI mapping workflow.
    Inspect {
        /// CSV file to inspect.
        csv: PathBuf,
        /// Number of sample rows to include.
        #[arg(long, default_value_t = 10)]
        samples: usize,
        /// Mapping file to test-drive against the CSV.
        #[arg(short, long)]
        mapping: Option<PathBuf>,
    },
    /// Regenerate the Obsidian dashboard from existing analyses.
    Dashboard,
    /// Show deterministic deltas between the two most recent analyses.
    Compare,
}

pub fn run(cli: Cli) -> Result<()> {
    let cfg = Config::load(cli.config.as_deref())?;
    match cli.command {
        Command::Analyze {
            csv,
            dry_run,
            json,
            mapping,
        } => {
            let cfg = with_mapping(cfg, &csv, mapping.as_deref())?;
            analyze(&csv, &cfg, dry_run, json)
        }
        Command::Stats { csv, mapping } => {
            let cfg = with_mapping(cfg, &csv, mapping.as_deref())?;
            stats(&csv, &cfg)
        }
        Command::ExportJson {
            csv,
            output,
            mapping,
        } => {
            let cfg = with_mapping(cfg, &csv, mapping.as_deref())?;
            export_json(&csv, &cfg, output)
        }
        Command::Inspect {
            csv,
            samples,
            mapping,
        } => {
            let cfg = with_mapping(cfg, &csv, mapping.as_deref())?;
            inspect(&csv, &cfg, samples)
        }
        Command::Dashboard => dashboard(&cfg),
        Command::Compare => compare(&cfg),
    }
}

/// Resolve the effective CSV config: config.toml `[csv]` overlaid with an
/// explicit `--mapping` reference (a file path or a `profiles/<name>.toml`
/// profile name), or an auto-discovered `<csv>.mapping.toml`.
fn with_mapping(
    mut cfg: Config,
    csv: &std::path::Path,
    explicit: Option<&std::path::Path>,
) -> Result<Config> {
    let mapping_path = match explicit {
        Some(reference) => Some(crate::config::CsvConfig::resolve_mapping_ref(
            reference,
            std::path::Path::new("profiles"),
        )?),
        None => {
            let sibling = PathBuf::from(format!("{}.mapping.toml", csv.display()));
            sibling.exists().then_some(sibling)
        }
    };
    if let Some(path) = mapping_path {
        let overlay = crate::config::CsvConfig::load_mapping_file(&path)?;
        eprintln!("using mapping file {}", path.display());
        cfg.csv = cfg.csv.merged_with(&overlay);
    }
    Ok(cfg)
}

fn now_stamp() -> (String, String) {
    let now = chrono::Local::now();
    (
        now.format("%Y-%m-%dT%H:%M:%S").to_string(),
        now.format("%Y-%m-%d_%H-%M").to_string(),
    )
}

fn parse(csv: &std::path::Path, cfg: &Config) -> Result<parser::ParseReport> {
    let report = parser::parse_csv(csv, &cfg.csv)?;
    eprintln!(
        "parsed {} trades from {} ({} rows skipped) — delimiter '{}', dates '{}', decimals '{}', {}",
        report.trades.len(),
        csv.display(),
        report.skipped_rows,
        report.detected.delimiter,
        report.detected.date_format,
        report.detected.decimal_separator,
        report.detected.encoding,
    );
    for warning in &report.warnings {
        eprintln!("  warning: {warning}");
    }
    Ok(report)
}

fn analyze(csv: &std::path::Path, cfg: &Config, dry_run: bool, print_json: bool) -> Result<()> {
    let report = parse(csv, cfg)?;
    let (generated_at, stamp) = now_stamp();

    // Build once without history to snapshot the current run, then attach
    // the comparison against stored history.
    let mut exchange = analyzer::build_exchange(&report, csv, cfg, generated_at, None)?;
    exchange.historical_comparison = analyzer::compare_against_vault_history(&exchange, cfg)?;

    let json = serde_json::to_string_pretty(&exchange)?;
    if print_json {
        println!("{json}");
    }

    if dry_run {
        eprintln!("dry run: nothing written");
        return Ok(());
    }

    let vault = Vault::new(&cfg.vault);
    vault.ensure_layout()?;

    // Two analyses within the same minute: extend the stamp with seconds
    // instead of silently overwriting the earlier report.
    let stamp = if vault.analyses_dir().join(format!("{stamp}.md")).exists() {
        chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string()
    } else {
        stamp
    };

    let markdown = reports::render(&exchange, &stamp);
    let report_path = vault.write_analysis(&stamp, &markdown)?;
    let json_path = if cfg.output.write_exchange_json {
        Some(vault.write_exchange_json(&stamp, &json)?)
    } else {
        None
    };
    // Append first so the historical dashboard includes this analysis.
    let updated_history = analyzer::append_to_history(&exchange, Some(format!("{stamp}.md")), cfg)?;
    vault.update_dashboard(&updated_history)?;

    let s = &exchange.summary;
    println!("Analysis complete: {stamp}");
    println!(
        "  {} trades | PnL {:.2} | win rate {:.1}% | overall score {:.0}/100",
        s.total_trades, s.total_pnl, s.win_rate, s.overall_score
    );
    if !s.detected_behaviors.is_empty() {
        println!("  detected behaviors: {}", s.detected_behaviors.join(", "));
    }
    println!("  report:   {}", report_path.display());
    if let Some(p) = &json_path {
        println!("  exchange: {}", p.display());
    }
    println!();
    println!("Open the report in Obsidian (Charts plugin required) to see the dashboard.");
    println!("Next step (AI coaching): have Claude Code read the exchange JSON and fill in");
    println!("the `<!-- ai:begin:... -->` blocks with short chart interpretations. The AI");
    println!("never calculates numbers or writes chart syntax.");
    Ok(())
}

fn stats(csv: &std::path::Path, cfg: &Config) -> Result<()> {
    let report = parse(csv, cfg)?;
    let (generated_at, _) = now_stamp();
    let exchange = analyzer::build_exchange(&report, csv, cfg, generated_at, None)?;
    let p = &exchange.performance;
    let d = &exchange.daily_statistics;

    println!(
        "Period       {} .. {} ({} trading days)",
        exchange.trading_period.first_trade_date,
        exchange.trading_period.last_trade_date,
        exchange.trading_period.total_trading_days
    );
    println!(
        "Trades       {} ({} W / {} L / {} BE)",
        p.total_trades, p.winning_trades, p.losing_trades, p.breakeven_trades
    );
    println!(
        "Total PnL    {:.2}   (gross +{:.2} / {:.2}, fees {:.2})",
        p.total_pnl, p.gross_profit, p.gross_loss, p.total_fees
    );
    println!("Win rate     {:.1}%", p.win_rate);
    println!(
        "Avg win/loss {} / {}",
        p.average_win
            .map(|v| format!("{v:.2}"))
            .unwrap_or_else(|| "n/a".into()),
        p.average_loss
            .map(|v| format!("{v:.2}"))
            .unwrap_or_else(|| "n/a".into())
    );
    println!(
        "Profit factor {}",
        p.profit_factor
            .map(|v| format!("{v:.2}"))
            .unwrap_or_else(|| "n/a".into())
    );
    println!(
        "Expectancy   {}",
        p.expectancy
            .map(|v| format!("{v:.2}"))
            .unwrap_or_else(|| "n/a".into())
    );
    println!(
        "Avg RR       {}",
        p.average_rr
            .map(|v| format!("{v:.2}"))
            .unwrap_or_else(|| "n/a".into())
    );
    println!("Max daily DD {:.2}", d.max_daily_drawdown);
    println!(
        "Best day     {}",
        d.best_day
            .as_ref()
            .map(|x| format!("{} ({:.2})", x.date, x.pnl))
            .unwrap_or_else(|| "n/a".into())
    );
    println!(
        "Worst day    {}",
        d.worst_day
            .as_ref()
            .map(|x| format!("{} ({:.2})", x.date, x.pnl))
            .unwrap_or_else(|| "n/a".into())
    );
    println!(
        "Scores       discipline {:.0} | consistency {:.0} | emotional {:.0} | overall {:.0}",
        exchange.scores.discipline.score,
        exchange.scores.consistency.score,
        exchange.scores.emotional_control.score,
        exchange.scores.overall
    );
    if !exchange.summary.detected_behaviors.is_empty() {
        println!(
            "Behaviors    {}",
            exchange.summary.detected_behaviors.join(", ")
        );
    }
    Ok(())
}

fn export_json(csv: &std::path::Path, cfg: &Config, output: Option<PathBuf>) -> Result<()> {
    let report = parse(csv, cfg)?;
    let (generated_at, _) = now_stamp();
    let mut exchange = analyzer::build_exchange(&report, csv, cfg, generated_at, None)?;
    // Include the comparison when history exists, but never append to it.
    exchange.historical_comparison = analyzer::compare_against_vault_history(&exchange, cfg)?;
    let json = serde_json::to_string_pretty(&exchange)?;
    match output {
        Some(path) => {
            std::fs::write(&path, &json)
                .with_context(|| format!("cannot write {}", path.display()))?;
            eprintln!("exchange JSON written to {}", path.display());
        }
        None => println!("{json}"),
    }
    Ok(())
}

fn inspect(csv: &std::path::Path, cfg: &Config, samples: usize) -> Result<()> {
    let value = parser::inspect_csv(csv, &cfg.csv, samples)?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

fn dashboard(cfg: &Config) -> Result<()> {
    let vault = Vault::new(&cfg.vault);
    vault.ensure_layout()?;
    let stored = history::load(&history::history_path(
        &cfg.vault.path,
        &cfg.vault.assets_dir,
    ))?;
    let path = vault.update_dashboard(&stored)?;
    println!("dashboard updated: {}", path.display());
    Ok(())
}

fn compare(cfg: &Config) -> Result<()> {
    let path = history::history_path(&cfg.vault.path, &cfg.vault.assets_dir);
    let stored = history::load(&path)?;
    if stored.snapshots.len() < 2 {
        bail!(
            "need at least 2 analyses in history to compare (found {}) — run `trade-analyzer analyze` first",
            stored.snapshots.len()
        );
    }
    let current = stored.snapshots.last().unwrap().clone();
    let previous_history = history::HistoryFile {
        schema_version: stored.schema_version.clone(),
        snapshots: stored.snapshots[..stored.snapshots.len() - 1].to_vec(),
    };
    let comparison = history::compare(&current, &previous_history);
    println!("{}", serde_json::to_string_pretty(&comparison)?);
    Ok(())
}
