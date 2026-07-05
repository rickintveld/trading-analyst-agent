# Claude Code Trade Analyst Agent

An AI-powered trading performance coach: a **deterministic Rust analytics engine** that turns trading journal CSVs into **interactive visual dashboards** ([Obsidian Charts](https://community.obsidian.md/plugins/obsidian-charts)) — plus an **AI coaching layer** (Claude Code) that interprets those charts like a trading psychologist and quantitative performance coach.

**Rust calculates and draws. AI explains. Never the other way around.**

## How it works

```
CSV ──► Rust engine ──► chart datasets ──► Obsidian dashboard ──► Claude coaching
       (deterministic)  (exchange JSON)    (chart-first report)   (interpretation only)
```

1. `trade-analyzer analyze trades.csv` parses your journal, computes everything deterministically, and writes a **visual dashboard** into your Obsidian vault: KPI cards up top, then equity curve, PnL, drawdown, behavior, pair/session/strategy and historical-development charts — all generated as native Obsidian Charts blocks by Rust. The **exchange JSON** is archived next to it.
2. The AI layer (Claude Code) reads that JSON — never the raw CSV, never writing chart syntax — and adds a concise interpretation under each chart group plus a short coaching summary and action plan.
3. Every analysis is appended to a rolling history: reports gain score/win-rate/profit-factor development charts, and `Dashboard.md` becomes a historical analytics dashboard across all analyses.

> Reports require the [Obsidian Charts](https://community.obsidian.md/plugins/obsidian-charts) community plugin to render. No manual editing is ever needed — every report renders immediately.

## Quickstart

**No Rust required** — download the latest release binary for your OS into `scripts/bin/`:

```bash
scripts/update.sh                                              # macOS / Linux
powershell -ExecutionPolicy Bypass -File scripts\update.ps1    # Windows
```

(Assets per OS: `trading-analyst-agent-mac` (universal), `trading-analyst-agent-windows.exe`, `trading-analyst-agent-linux` — also downloadable manually from GitHub Releases; put the file in `scripts/bin/` as `trade-analyzer`.)

Or build from source:

```bash
cargo build --release

# analyze a journal (creates ./TradingVault on first run)
./target/release/trade-analyzer analyze trades.csv

# quick deterministic stats in the terminal
./target/release/trade-analyzer stats trades.csv

# emit the AI exchange JSON
./target/release/trade-analyzer export-json trades.csv -o exchange.json

# regenerate the Obsidian dashboard / compare the two latest analyses
./target/release/trade-analyzer dashboard
./target/release/trade-analyzer compare
```

### One-prompt usage (Claude Code)

Open the repo in Claude Code, attach or name your CSV, and prompt:

> Analyze my trades

The `analyze` skill runs the whole pipeline: fetches the engine binary if missing → maps unknown CSV formats (AI-authored mapping file / broker profile) → parses & writes the visual dashboard to the vault → fans out parallel `trading-coach` agents (Sonnet) to fill every coaching section → replies with the KPIs and the #1 action. Or run steps individually:

> Coach my latest analysis.

See `CLAUDE.md` for the exact protocols.

## CSV input — any broker format

The parser auto-detects delimiter, encoding, headers, date format and decimal separator, and skips malformed rows with warnings. Mapping is **content-aware**: it checks what the values look like, not just the header names — so MetaTrader-style exports (datetime `Open`/`Close` columns, duplicate `Price` headers, `Type`/`Volume`/`Profit` naming) parse with zero configuration. Only three columns are required — **date, symbol/pair, PnL** — everything else (time, direction, entry/exit, size, risk, reward, RR, fees, strategy, session, notes) unlocks deeper analytics when present.

### Unknown formats: the AI mapping workflow

For exports the heuristics can't resolve, the AI layer authors a **mapping file** — the LLM never touches the engine or the raw data:

```bash
trade-analyzer inspect trades.csv     # structured sample: headers, example rows, detection attempt
```

Claude Code (via the bundled `csv-mapping` skill in `.claude/skills/`) reads that JSON, decides the mapping by content, and writes `trades.csv.mapping.toml` next to your file:

```toml
date_format = "%Y-%m-%d %H:%M:%S"

[mapping]
date = "Open"          # exact header name...
entry = "col:5"        # ...or col:<index> for duplicate headers
pnl = "Profit"
fees = "Commissions"
```

The mapping file is auto-discovered on every future run. For named broker formats, save it to the shared **profile library** instead — `profiles/<platform>.toml` — and anyone in the community can use it by name:

```bash
trade-analyzer analyze trades.csv --mapping metatrader
```

One mapping per broker *format*, not per file. See `profiles/README.md` for contribution conventions; `profiles/metatrader.toml` ships as the reference.

## What the engine computes

- **Performance** — PnL, gross profit/loss, win rate, profit factor, expectancy, average RR, largest win/loss
- **Daily / weekly / monthly** — best/worst periods, max daily drawdown, monthly trend
- **Pairs / strategies / sessions** — per-group win rate, RR and PnL; best and worst of each
- **Streaks** — trade-level and day-level winning/losing streaks
- **Behavioral flags** (pure heuristics, no AI): revenge trading, overtrading, FOMO, position-size escalation, risk escalation, tilt, panic exits, consecutive rule violations, strategy switching, excessive trading frequency
- **Scores 0–100** — discipline, consistency, emotional control, each with weighted components and formula identifiers
- **Time series** — equity curve, rolling drawdown, risk per trade, rolling RR and streak length per trade (auto-downsampled for very large journals)
- **History** — rolling averages, deltas and full snapshot series across analyses

## What the dashboard shows

Every analysis note renders, in fixed order: Trading Period → **Executive KPI Summary** → Performance charts (equity curve, daily/weekly/monthly PnL, winning-vs-losing days) → Risk charts (drawdown vs. maximum, risk per trade, rolling RR) → Behavior (flags table, streaks, score/win-rate/profit-factor/RR development) → Pair Performance (sorted horizontal bars: PnL, win rate, RR, trades) → Session Performance → Strategy Performance → Historical Comparison (current vs. historical average/best/worst) → AI Coaching → Action Plan.

`Dashboard.md` aggregates all analyses: total PnL, win rate, profit factor, RR, weekly/monthly PnL, drawdown and score trends over time, plus links to every report.

## The exchange JSON contract

The engine always emits the same top-level schema (see `src/ai/mod.rs`):

```json
{
  "metadata": {},
  "trading_period": {},
  "performance": {},
  "daily_statistics": {},
  "weekly_statistics": {},
  "monthly_statistics": {},
  "pair_statistics": {},
  "strategy_statistics": {},
  "session_statistics": {},
  "streaks": {},
  "scores": {},
  "behavioral_flags": {},
  "historical_comparison": {},
  "summary": {},
  "time_series": {}
}
```

Snake_case keys, no dynamic key names, `null` for anything the input couldn't support. The schema is versioned (`metadata.schema_version`) and changes are backwards compatible.

## Obsidian vault

```
TradingVault/
    Analyses/          # one note per analysis: YYYY-MM-DD_HH-mm.md
    Dashboard/         # auto-generated Dashboard.md linking every analysis
    Templates/
    Assets/
        exchange/      # archived exchange JSONs
        history.json   # rolling deterministic history
```

## Configuration

Copy `config.example.toml` to `config.toml` and adjust vault location, CSV mapping, session windows, behavioral thresholds and score weights. Everything has sensible defaults; the tool runs with no config file at all.

## Development

```bash
cargo test            # unit + integration tests
cargo clippy          # lints
cargo build --release # optimized native binary, no runtime dependencies
```

Architecture and engineering conventions live in `CLAUDE.md`.

## License

MIT
