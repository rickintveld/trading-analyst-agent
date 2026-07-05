# Broker mapping profiles

Reusable CSV mappings, one per broker/platform export format. A profile is a
mapping file (same TOML shape as `[csv]` in `config.toml`) that works for
**every** CSV in that format — write it once, share it with the community.

## Using a profile

```bash
trade-analyzer analyze trades.csv --mapping metatrader   # name resolves to profiles/metatrader.toml
trade-analyzer analyze trades.csv --mapping path/to/custom.toml
```

Without `--mapping`, the engine tries content-aware auto-detection and an
auto-discovered `<csv>.mapping.toml` next to the file. Most formats need no
profile at all — add one only when detection gets a format wrong.

## Contributing a profile

1. Run `trade-analyzer inspect <csv>` (or let Claude Code do it via the
   `csv-mapping` skill, which automates this whole checklist).
2. Author the mapping: exact header names, or `col:<0-based index>` when a
   header name appears twice.
3. Verify: `trade-analyzer analyze <csv> --dry-run --mapping <file>` must
   parse every row (0 skipped).
4. Save as `profiles/<platform>.toml` (kebab-case, e.g. `tradelocker.toml`,
   `ninjatrader.toml`) with a comment listing the expected columns.

## Format

```toml
# <Platform> export — expected columns: ...
date_format = "%Y-%m-%d %H:%M:%S"   # only when detection needs pinning
# delimiter = ";"                    # only for non-comma exports
# decimal_separator = ","            # only for comma-decimal exports

[mapping]
date = "Open"
symbol = "Symbol"
pnl = "Profit"
# ... only map what exists; optional fields may be omitted
```
