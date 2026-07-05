---
name: analyze
description: End-to-end trading analysis - take a user's trades CSV (attached to the prompt or given as a path), resolve the engine binary, map/parse the CSV, write the visual dashboard + exchange JSON to the Obsidian vault, then run the AI performance coach. Use when the user says "analyze my trades", attaches a trading CSV, or asks for a full analysis of a journal/export.
---

# Full Analysis Workflow

One prompt in ("Analyze my trades"), one coached dashboard out. This skill
chains the engine, the csv-mapping workflow and the coach workflow in a fixed
order. The Rust engine calculates everything; AI only maps columns and
interprets results.

## Step 1 — Resolve the engine binary

In order, use the first that exists:
1. `scripts/bin/trade-analyzer` (`.exe` on Windows) — the downloaded release
2. `target/release/trade-analyzer` — locally built
3. Neither present: run `scripts/update.sh` (Windows:
   `powershell -ExecutionPolicy Bypass -File scripts/update.ps1`) to fetch the
   latest release into `scripts/bin/`. If the download fails (no release yet /
   offline) and a Rust toolchain exists, fall back to
   `cargo build --release` and use `target/release/trade-analyzer`.

Call the resolved binary `$TA` below.

## Step 2 — Locate the CSV

- File attached to the prompt: use its path directly.
- Path or filename mentioned: verify it exists.
- Neither: ask the user for the CSV location — do not guess.

## Step 3 — Parse (map first when needed)

```bash
$TA analyze <csv> --dry-run
```

- Parses cleanly (0 skipped rows, mapping semantically sane)? → Step 4.
- Fails or skips rows? → run the **csv-mapping** skill workflow (check
  `profiles/` first, then `$TA inspect <csv>` → author the mapping file →
  re-verify with `--dry-run`). Return here once the dry run is clean.

## Step 4 — Analyze into the vault

```bash
$TA analyze <csv>
```

This writes the visual dashboard to `<vault>/Analyses/<stamp>.md`, archives
the exchange JSON, appends history and rebuilds the historical Dashboard.md.
Capture `<stamp>` and both paths from the output.

## Step 5 — Coach

Run the **coach** skill workflow on the report from Step 4: one parallel
`trading-coach` agent per `ai:begin` section, apply results between the
markers, verify (marker counts match, no leftover `> _` placeholders).

## Step 6 — Report back to the user

Short and direct, like the coach sections themselves:
- KPI line: trades, PnL, win rate, profit factor, overall score
- Detected behaviors (if any)
- The 2-3 headline coaching takeaways and the #1 action
- Where to look: the report path (remind: Obsidian Charts plugin required)

## Constraints

- Never modify the CSV; never compute statistics yourself; never edit chart
  blocks or deterministic report content.
- If the vault is not where `config.toml` expects (or no config exists,
  default `./TradingVault`), the engine creates it — do not pre-create.
- Steps must run in this order; a failed step stops the flow with a clear
  message about what the user should provide or fix.
