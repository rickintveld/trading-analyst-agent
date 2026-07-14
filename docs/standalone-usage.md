# Using the engine without Claude Code

You don't need Claude Code, an Anthropic account, or a Rust toolchain to use this
tool. The core is a single self-contained native binary — `trade-analyzer` — that
parses your trading CSV, computes every statistic deterministically, and writes a
complete visual dashboard into an Obsidian vault.

The AI coaching layer is **optional**. Without it you still get the full report
(KPIs, equity curve, drawdown, behavior flags, scores, per-pair/session/strategy
charts, history). The only difference is that the `<!-- ai:begin:... -->` blocks —
the short written interpretations under each chart group — stay empty. Everything
else renders immediately.

---

## 1. Get the binary

You have two options: download a prebuilt release (no Rust needed) or build from
source.

### Option A — download a prebuilt release (recommended)

Download the asset for your operating system from the project's **GitHub Releases**
page (`https://github.com/rickintveld/trading-analyst-agent/releases/latest`):

| OS            | Release asset                          |
|---------------|----------------------------------------|
| macOS         | `trading-analyst-agent-mac` (universal)|
| Windows       | `trading-analyst-agent-windows.exe`    |
| Linux         | `trading-analyst-agent-linux`          |

You can either use the helper scripts (they auto-pick the right asset and only
re-download when a newer release exists), or place the file yourself.

**Helper scripts** — run from the repository root:

```bash
scripts/update.sh                                              # macOS / Linux
```
```powershell
powershell -ExecutionPolicy Bypass -File scripts\update.ps1    # Windows
```

Both download into `scripts/bin/` and name the file `trade-analyzer`
(`trade-analyzer.exe` on Windows).

**Manual placement** — if you downloaded the asset from the browser:

- Rename it to `trade-analyzer` (keep the `.exe` on Windows: `trade-analyzer.exe`).
- Put it wherever you like. A common choice is `scripts/bin/` in the repo, but any
  directory on your `PATH` works too.
- On macOS/Linux, make it executable and clear the quarantine flag:

  ```bash
  chmod +x trade-analyzer
  xattr -d com.apple.quarantine trade-analyzer   # macOS only, if Gatekeeper blocks it
  ```

Verify it runs:

```bash
./scripts/bin/trade-analyzer --version
```

### Option B — build from source

Requires a Rust toolchain (`rustup`):

```bash
cargo build --release
# binary lands at ./target/release/trade-analyzer
```

---

## 2. Understand where files go (important)

The tool writes into an **Obsidian vault** — a plain folder of Markdown files. Two
paths matter, and both are resolved **relative to the directory you run the command
from** (your current working directory), *not* relative to where the binary lives:

- **Config file:** the tool looks for `./config.toml` in the current directory. If
  none is found (and you don't pass `--config`), built-in defaults are used.
- **Vault:** default location is `./TradingVault`. It is created automatically on
  the first `analyze` run.

> **Practical rule:** always `cd` into the folder you want to be your "workspace"
> before running the binary. That folder is where `TradingVault/` (and any
> `config.toml`) will be created and updated. Running from a different folder each
> time will scatter separate vaults around your disk.

The simplest, most predictable setup is to run everything from the repository root.

---

## 3. Analyze your trades

From your workspace directory, point the binary at your CSV:

```bash
# macOS / Linux (binary in scripts/bin/)
./scripts/bin/trade-analyzer analyze /path/to/trades.csv

# Windows (PowerShell)
.\scripts\bin\trade-analyzer.exe analyze C:\path\to\trades.csv

# built from source
./target/release/trade-analyzer analyze /path/to/trades.csv
```

This single command:

1. Parses the CSV (auto-detecting delimiter, encoding, date format, decimals).
2. Computes all statistics, behavior flags and scores.
3. Writes a dated analysis note into `TradingVault/Analyses/YYYY-MM-DD_HH-mm.md`.
4. Archives the exchange JSON into `TradingVault/Assets/exchange/`.
5. Appends the run to the rolling history and rebuilds `TradingVault/Dashboard/Dashboard.md`.

On success it prints a summary and the exact paths written, e.g.:

```
Analysis complete: 2026-07-14_09-32
  142 trades | PnL 1830.50 | win rate 58.5% | overall score 71/100
  report:   TradingVault/Analyses/2026-07-14_09-32.md
  exchange: TradingVault/Assets/exchange/2026-07-14_09-32.json
```

### View the report

Open the `TradingVault/` folder as a vault in [Obsidian](https://obsidian.md), and
install the [Obsidian Charts](https://community.obsidian.md/plugins/obsidian-charts)
community plugin (Settings → Community plugins → Browse → "Charts"). Without that
plugin the ```chart blocks show as raw code instead of graphs. Then open the note
under `Analyses/` or open `Dashboard/Dashboard.md` for the cross-analysis view.

Every run appends to history — from your **second** analysis onward the reports and
the dashboard gain trend charts (score / win-rate / profit-factor development over
time).

---

## 4. Other commands

All CSV commands accept `--mapping <file-or-profile>` (see section 6) and take the
same binary prefix as above (shown here as `trade-analyzer` for brevity):

```bash
# Quick stats in the terminal — writes nothing to the vault
trade-analyzer stats trades.csv

# Compute everything but write nothing (verify parsing before committing)
trade-analyzer analyze trades.csv --dry-run

# ...and also print the full exchange JSON to the screen
trade-analyzer analyze trades.csv --dry-run --json

# Emit the exchange JSON to a file (no vault writes)
trade-analyzer export-json trades.csv -o exchange.json

# Rebuild Dashboard.md from the analyses already in the vault
trade-analyzer dashboard

# Print JSON deltas between the two most recent analyses (needs >= 2 in history)
trade-analyzer compare

# Inspect a CSV's structure (used when authoring a mapping — see section 6)
trade-analyzer inspect trades.csv
```

Diagnostics (parse warnings, detected format) go to **stderr**; results
(`compare` / `export-json`) print pure JSON to **stdout**, so you can pipe them.

---

## 5. Configuration (optional)

The tool runs with zero config. To customize, copy the example and edit it:

```bash
cp config.example.toml config.toml
```

Keep `config.toml` in the directory you run commands from. Common things to change:

- `[vault] path` — where the vault lives (e.g. an absolute path to a folder already
  inside your existing Obsidian vault).
- `[csv]` — force a delimiter / date format / decimal separator if auto-detection
  guesses wrong; `prefer_day_first` resolves ambiguous dates like `02/03/2026`.
- `[sessions]` — the time windows used to label Asia / London / Overlap / New York.
- `[behavior]` — thresholds for the behavioral detectors (e.g. what counts as
  overtrading or a revenge trade). Tune these to your timeframe.
- `[scores.*]` — weighting of each score component.

Every key is optional; omitted keys fall back to the defaults documented inline in
`config.example.toml`. To use a config from elsewhere, pass `--config <path>`
(works with any command).

### Point the vault at your existing Obsidian vault

If you already keep an Obsidian vault, set the path so reports land inside it:

```toml
[vault]
path = "/Users/you/ObsidianVault/Trading"
```

The subfolders (`Analyses/`, `Dashboard/`, `Assets/`, `Templates/`) are created
under that path automatically.

---

## 6. When a CSV won't parse cleanly

The parser handles most broker exports out of the box, including MetaTrader-style
files. If `analyze` reports a missing column, maps columns wrongly, or skips many
rows, you supply a small **mapping file** — you never edit the engine.

1. Check `profiles/` for an existing broker profile. If one fits, just name it:

   ```bash
   trade-analyzer analyze trades.csv --mapping metatrader
   ```

   (A bare name resolves to `profiles/<name>.toml`.)

2. Otherwise, inspect the file's structure:

   ```bash
   trade-analyzer inspect trades.csv
   ```

   This prints the detected headers, sample rows, and the detection attempt.

3. Write a mapping file. Save it as `trades.csv.mapping.toml` **next to your CSV**
   (it is auto-discovered on future runs), or as `profiles/<broker>.toml` for a
   reusable named profile. Only the three required fields — date, symbol, pnl — must
   resolve; map any others that exist:

   ```toml
   date_format = "%Y-%m-%d %H:%M:%S"   # chrono format string

   [mapping]
   date   = "Open"        # exact header name...
   symbol = "Symbol"
   entry  = "col:5"       # ...or col:<index> (0-based) for duplicate headers
   pnl    = "Profit"
   fees   = "Commissions"
   ```

   Canonical fields you can map: `date`, `time`, `symbol`, `direction`, `entry`,
   `exit`, `position_size`, `risk`, `reward`, `rr`, `pnl`, `fees`, `strategy`,
   `session`, `notes`.

4. Verify with a dry run — you want **0 rows skipped** — then analyze for real:

   ```bash
   trade-analyzer analyze trades.csv --dry-run
   trade-analyzer analyze trades.csv
   ```

One mapping per broker *format*, not per file.

---

## 7. Optional: add AI coaching later

The written coaching under each chart is produced by Claude Code reading the
exchange JSON (`TradingVault/Assets/exchange/<stamp>.json`) — it never sees your raw
CSV and never touches the numbers or charts. If you later open this repository in
Claude Code, prompt it with *"Coach my latest analysis"* and it will fill in the
`ai:begin` blocks of the most recent report. Until then, those sections simply stay
as placeholders; the rest of the dashboard is fully usable on its own.

---

## Troubleshooting

- **"command not found" / won't run** — you're likely calling it by the wrong path.
  Use the full path (`./scripts/bin/trade-analyzer`) or add its directory to `PATH`.
- **macOS "cannot be opened because the developer cannot be verified"** — run
  `xattr -d com.apple.quarantine <binary>`, or allow it in System Settings →
  Privacy & Security.
- **Charts show as raw text in Obsidian** — install/enable the Obsidian Charts
  community plugin.
- **A new `TradingVault` appeared somewhere unexpected** — you ran the binary from a
  different working directory. `cd` into your workspace folder first, or set an
  absolute `[vault] path` in `config.toml`.
- **Many rows skipped / wrong columns** — author a mapping file (section 6).
