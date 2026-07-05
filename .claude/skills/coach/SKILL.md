---
name: coach
description: Run the AI performance coach on a trading analysis - fill every <!-- ai:begin:... --> section of an Obsidian dashboard report with short, direct coaching, one parallel trading-coach agent per section. Use when the user asks to coach an analysis, coach the latest report, or complete the AI sections of a dashboard.
---

# AI Performance Coach Workflow

Fill the `<!-- ai:begin:NAME -->` blocks of a dashboard report using parallel
`trading-coach` agents — one per section. The engine calculated everything;
the coach only interprets. Follow these steps in order, every time.
(For the full CSV-to-coached-dashboard pipeline, the `analyze` skill wraps
this workflow as its final step.)

## Step 1 — Locate the analysis

Default to the newest report unless the user names one:
- Report: highest-sorting `<vault>/Analyses/*.md` (vault path from
  `config.toml` `[vault]`, default `./TradingVault`)
- Exchange JSON: same stamp under `<vault>/Assets/exchange/<stamp>.json`

Grep the report for `<!-- ai:begin:` to get the exact section list — it varies
(e.g. `strategies`/`sessions` only exist when the journal has that data;
`historical` content depends on `previous_analyses`).

## Step 2 — Fan out one trading-coach agent per section, in parallel

Spawn ALL sections in a single message (subagent_type: `trading-coach`).
If that agent type is not registered in this session (definitions load at
session start), fall back to `general-purpose` and inline the rules from
`.claude/agents/trading-coach.md` into each prompt — same fan-out.
Each prompt must contain:
1. The absolute path of the exchange JSON.
2. The section name and its focus (table below).
3. The reminder: "Return only the callout content per your output format."

| Section | Focus |
| --- | --- |
| executive_summary | Overall state, single most important pattern, #1 priority |
| performance | Equity curve shape, PnL rhythm, win/loss quality (average_win vs average_loss, profit factor) |
| risk | Drawdown depth vs total PnL, risk sizing (or absence of risk data), RR quality |
| behavior | Detected flags + their metrics; likely psychological drivers; severity-ranked |
| pairs | Strongest/weakest market by PnL, win rate, RR; where to focus |
| sessions | Best/worst session; should trading hours shift |
| strategies | Which setup earns its place, which costs money (only if section exists) |
| historical | Trend across analyses: improving/regressing streaks (baseline statement if previous_analyses is 0) |
| coaching | Max 5 bullets: strengths & weaknesses ranked by impact |
| action_plan | Max 3 actions targeting the worst flags/scores, each with a measurable target |

## Step 3 — Apply the results

For each agent result, Edit the report: replace everything BETWEEN
`<!-- ai:begin:NAME -->` and `<!-- ai:end:NAME -->` (exclusive — keep both
markers) with the agent's callout block. Do not touch anything else: no chart
blocks, no tables, no deterministic text, no section order, no filename.

## Step 4 — Verify

- `grep -c "ai:begin"` equals `grep -c "ai:end"` and matches Step 1's count.
- No `_instruction_` placeholder text (`> _`) remains in any coached block.
- Spot-check that every quoted number exists in the exchange JSON.

## Step 5 — Report back

Tell the user: which report was coached, the 2-3 headline takeaways, and the
#1 action. Keep it as short as the sections themselves.

## Constraints

- Never recalculate statistics; never write or edit ```chart syntax.
- If an agent returns malformed output (missing `> ` prefixes, markers, code
  fences), fix the formatting yourself — do not re-run the whole fan-out.
- One coaching pass per report; re-coaching overwrites previous blocks.
