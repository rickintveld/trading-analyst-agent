---
name: csv-mapping
description: Map an unknown trading-journal CSV export to trade-analyzer's canonical fields by authoring a mapping file. Use when trade-analyzer analyze/stats/export-json fails to parse a CSV (missing date/symbol/pnl column, wrong columns mapped, many skipped rows) or when onboarding a new broker export format for the community.
---

# CSV Mapping Workflow

You are the mapping layer between an arbitrary broker CSV export and the
deterministic Rust engine. You never parse the CSV yourself and you never
edit engine code to fit a file — you author a **mapping file** that the
engine consumes. Follow these steps in order, every time.

`$TA` below is the engine binary, resolved in this order:
`scripts/bin/trade-analyzer` (downloaded release; `.exe` on Windows) →
`target/release/trade-analyzer` → run `scripts/update.sh` to fetch it
(fallback: `cargo build --release`).

## Step 0 — Check the profile library first

List `profiles/*.toml`. If a profile matches the broker/platform the CSV
came from, verify it instead of authoring a new mapping:

```bash
$TA analyze <csv> --dry-run --mapping <profile-name>
```

If it parses every row, tell the user to run with `--mapping <profile-name>`
and stop. Only continue when no profile fits.

## Step 1 — Inspect (never read the raw CSV)

```bash
$TA inspect <csv>
```

Read only this JSON. It contains:
- `headers` — every column with its 0-based index
- `sample_rows` — real example values per column
- `canonical_fields` — the target fields, with `required: true` on date/symbol/pnl
- `detection` — what auto-detection currently concludes (`status: ok` with a
  `trial_parse`, or `status: error` with the failure)

If `detection.status` is `ok` with `skipped_rows: 0` and the `mapped` fields
look semantically right, no mapping file is needed — tell the user and stop.

## Step 2 — Decide the mapping

Decide by **content, not header names**. Rules:
- `date` = the column holding the trade **open** date or datetime (a full
  datetime is preferred: it also provides the time for session analytics).
- Columns named Open/Close holding datetimes are open/close *times*, not prices.
- `pnl` = realized profit/loss per trade (usually named Profit/PnL/Result).
- `fees` = commissions/swap/costs; sign is normalized by the engine.
- Duplicate header names cannot be addressed by name — use `col:<index>`.
- Map only what exists. Optional fields you cannot identify are simply omitted;
  the engine degrades gracefully.
- Set `date_format` (chrono syntax) only when detection got it wrong;
  set `decimal_separator` only for comma-decimal exports.

## Step 3 — Write the mapping file

Pick the destination by reusability:
- Format is a recognizable broker/platform standard (MT4/5, TradeLocker,
  NinjaTrader, TradingView, cTrader, Edgewonk, TraderVue, an exchange...):
  save as **`profiles/<platform>.toml`** (kebab-case) so the whole community
  reuses it, and follow `profiles/README.md` conventions.
- One-off personal/custom sheet: save as **`<csv>.mapping.toml`** next to
  the CSV (auto-discovered on every future run, no flags needed).

See `profiles/metatrader.toml` for the reference example. The TOML shape:

```toml
# <Platform> export — expected columns: ...
date_format = "%Y-%m-%d %H:%M:%S"   # only when detection needs pinning

[mapping]
date = "Open"            # open datetime
symbol = "Symbol"
direction = "Type"       # buy/sell
position_size = "Volume"
entry = "col:5"          # col:<0-based index> for duplicate headers
exit = "col:9"
pnl = "Profit"
fees = "Commissions"
```

## Step 4 — Verify

```bash
$TA analyze <csv> --dry-run                       # <csv>.mapping.toml (auto-discovered)
$TA analyze <csv> --dry-run --mapping <profile>   # profile from profiles/
```

Check stderr: parsed count must match `total_rows` from Step 1 and
`skipped rows` must be 0 (or explained, e.g. genuinely malformed lines).
If not, fix the mapping file and repeat this step. Never proceed with a
mapping that silently drops trades.

Sanity-check the numbers: run `trade-analyzer stats <csv>` and confirm the
totals are plausible for the sample values you saw in Step 1 (e.g. PnL
magnitude, trade count, date range).

## Step 5 — Run for real

```bash
$TA analyze <csv>
```

Report to the user: which columns were mapped (and any that were left
unmapped and why), the trade count, and where the report landed.

## Constraints

- Never compute statistics yourself; the engine does all math.
- Never modify Rust source, config.toml score weights, or the CSV itself.
- One mapping per CSV *format*, not per file — prefer a shared profile in
  `profiles/` over per-file mapping files whenever the format has a name.
