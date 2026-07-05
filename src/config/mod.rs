//! TOML configuration. Every field has a sensible default so the tool
//! works with no config file at all.

use anyhow::{bail, Context, Result};
use chrono::NaiveTime;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::shared::Session;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub general: GeneralConfig,
    pub vault: VaultConfig,
    pub csv: CsvConfig,
    pub sessions: SessionsConfig,
    pub behavior: BehaviorConfig,
    pub scores: ScoresConfig,
    pub output: OutputConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    /// Informational timezone label stored in report metadata. Trade times
    /// are interpreted as-is (broker/journal local time).
    pub timezone: String,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            timezone: "UTC".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VaultConfig {
    /// Obsidian vault root. Created if missing.
    pub path: PathBuf,
    pub analyses_dir: String,
    pub dashboard_dir: String,
    pub templates_dir: String,
    pub assets_dir: String,
}

impl Default for VaultConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::from("./TradingVault"),
            analyses_dir: "Analyses".to_string(),
            dashboard_dir: "Dashboard".to_string(),
            templates_dir: "Templates".to_string(),
            assets_dir: "Assets".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CsvConfig {
    /// Force a delimiter instead of auto-detection (e.g. ";").
    pub delimiter: Option<String>,
    /// Force a chrono date format (e.g. "%d-%m-%Y") instead of auto-detection.
    pub date_format: Option<String>,
    /// Force a decimal separator ("." or ",") instead of auto-detection.
    pub decimal_separator: Option<String>,
    /// When a date like 02/03/2026 is ambiguous, prefer day-first (EU) parsing.
    pub prefer_day_first: bool,
    /// Header overrides: canonical field -> exact CSV header name.
    /// Canonical fields: date, time, symbol, direction, entry, exit,
    /// position_size, risk, reward, rr, pnl, fees, strategy, session, notes.
    pub mapping: BTreeMap<String, String>,
}

impl Default for CsvConfig {
    fn default() -> Self {
        Self {
            delimiter: None,
            date_format: None,
            decimal_separator: None,
            prefer_day_first: true,
            mapping: BTreeMap::new(),
        }
    }
}

impl CsvConfig {
    /// Load a standalone mapping file (TOML with the same shape as `[csv]`:
    /// top-level delimiter/date_format/decimal_separator/prefer_day_first
    /// plus a `[mapping]` table). Produced by the AI csv-mapping workflow.
    pub fn load_mapping_file(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read mapping file {}", path.display()))?;
        toml::from_str(&raw).with_context(|| format!("invalid TOML in {}", path.display()))
    }

    /// Resolve a `--mapping` reference: an existing file path, or a bare
    /// profile name looked up as `<profiles_dir>/<name>.toml` (the shared
    /// per-broker mapping library).
    pub fn resolve_mapping_ref(raw: &Path, profiles_dir: &Path) -> Result<std::path::PathBuf> {
        if raw.exists() {
            return Ok(raw.to_path_buf());
        }
        // Bare name (no path separators): try the profile library.
        if raw.components().count() == 1 {
            if let Some(name) = raw.to_str() {
                let candidate =
                    profiles_dir.join(format!("{}.toml", name.trim_end_matches(".toml")));
                if candidate.exists() {
                    return Ok(candidate);
                }
            }
        }
        bail!(
            "mapping {} not found — expected a mapping file path or a profile name from {}/",
            raw.display(),
            profiles_dir.display()
        )
    }

    /// Overlay `other` on top of `self`: set fields in `other` win, mapping
    /// entries are merged with `other` taking precedence.
    pub fn merged_with(&self, other: &CsvConfig) -> CsvConfig {
        let mut mapping = self.mapping.clone();
        mapping.extend(other.mapping.clone());
        CsvConfig {
            delimiter: other.delimiter.clone().or_else(|| self.delimiter.clone()),
            date_format: other
                .date_format
                .clone()
                .or_else(|| self.date_format.clone()),
            decimal_separator: other
                .decimal_separator
                .clone()
                .or_else(|| self.decimal_separator.clone()),
            prefer_day_first: other.prefer_day_first,
            mapping,
        }
    }
}

/// Session windows in trade-local time, "HH:MM-HH:MM" (end exclusive).
/// A window may wrap midnight (e.g. "23:00-07:00").
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionsConfig {
    pub asia: String,
    pub london: String,
    pub overlap: String,
    pub new_york: String,
}

impl Default for SessionsConfig {
    fn default() -> Self {
        Self {
            asia: "00:00-07:00".to_string(),
            london: "07:00-13:00".to_string(),
            overlap: "13:00-16:00".to_string(),
            new_york: "16:00-22:00".to_string(),
        }
    }
}

impl SessionsConfig {
    /// Resolve a trade time to a session using the configured windows.
    pub fn session_for(&self, time: NaiveTime) -> Session {
        let windows = [
            (Session::Asia, &self.asia),
            (Session::London, &self.london),
            (Session::Overlap, &self.overlap),
            (Session::NewYork, &self.new_york),
        ];
        for (session, window) in windows {
            if let Some((start, end)) = parse_window(window) {
                let inside = if start <= end {
                    time >= start && time < end
                } else {
                    // wraps midnight
                    time >= start || time < end
                };
                if inside {
                    return session;
                }
            }
        }
        Session::Other
    }
}

fn parse_window(raw: &str) -> Option<(NaiveTime, NaiveTime)> {
    let (a, b) = raw.split_once('-')?;
    let start = NaiveTime::parse_from_str(a.trim(), "%H:%M").ok()?;
    let end = NaiveTime::parse_from_str(b.trim(), "%H:%M").ok()?;
    Some((start, end))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BehaviorConfig {
    /// Trades per day at/above which a day counts as overtraded.
    pub max_trades_per_day: usize,
    /// Minutes after a loss within which a re-entry counts as revenge-window.
    pub revenge_window_minutes: i64,
    /// Minutes after a win within which a re-entry counts as FOMO-window.
    pub fomo_window_minutes: i64,
    /// Risk above `risk_violation_multiple` x median risk is a rule violation.
    pub risk_violation_multiple: f64,
    /// Consecutive rule violations needed to raise the flag.
    pub consecutive_violation_threshold: usize,
    /// Consecutive same-day losses that define a tilt day.
    pub tilt_consecutive_losses: usize,
}

impl Default for BehaviorConfig {
    fn default() -> Self {
        Self {
            max_trades_per_day: 4,
            revenge_window_minutes: 5,
            fomo_window_minutes: 15,
            risk_violation_multiple: 2.0,
            consecutive_violation_threshold: 3,
            tilt_consecutive_losses: 3,
        }
    }
}

/// Score component weights. Each group is normalized before use, so the
/// weights only need to be proportional.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ScoresConfig {
    pub discipline: DisciplineWeights,
    pub consistency: ConsistencyWeights,
    pub emotional_control: EmotionalWeights,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DisciplineWeights {
    pub risk_consistency: f64,
    pub overtrading_control: f64,
    pub rule_adherence: f64,
    pub strategy_focus: f64,
}

impl Default for DisciplineWeights {
    fn default() -> Self {
        Self {
            risk_consistency: 0.35,
            overtrading_control: 0.25,
            rule_adherence: 0.25,
            strategy_focus: 0.15,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ConsistencyWeights {
    pub positive_day_ratio: f64,
    pub weekly_stability: f64,
    pub pnl_concentration: f64,
    pub win_rate_stability: f64,
}

impl Default for ConsistencyWeights {
    fn default() -> Self {
        Self {
            positive_day_ratio: 0.3,
            weekly_stability: 0.3,
            pnl_concentration: 0.2,
            win_rate_stability: 0.2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EmotionalWeights {
    pub revenge_control: f64,
    pub tilt_control: f64,
    pub panic_exit_control: f64,
    pub loss_reaction: f64,
}

impl Default for EmotionalWeights {
    fn default() -> Self {
        Self {
            revenge_control: 0.3,
            tilt_control: 0.3,
            panic_exit_control: 0.2,
            loss_reaction: 0.2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputConfig {
    /// Also write the exchange JSON next to the report inside the vault.
    pub write_exchange_json: bool,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            write_exchange_json: true,
        }
    }
}

impl Config {
    /// Load configuration. Search order:
    /// 1. explicit `--config` path (error when missing),
    /// 2. `./config.toml`,
    /// 3. built-in defaults.
    pub fn load(explicit: Option<&Path>) -> Result<Self> {
        if let Some(path) = explicit {
            let raw = std::fs::read_to_string(path)
                .with_context(|| format!("cannot read config file {}", path.display()))?;
            let cfg: Config = toml::from_str(&raw)
                .with_context(|| format!("invalid TOML in {}", path.display()))?;
            return Ok(cfg.expand_home());
        }
        let default_path = Path::new("config.toml");
        if default_path.exists() {
            let raw = std::fs::read_to_string(default_path).context("cannot read config.toml")?;
            let cfg: Config = toml::from_str(&raw).context("invalid TOML in config.toml")?;
            return Ok(cfg.expand_home());
        }
        Ok(Config::default())
    }

    /// Expand a leading `~` in the vault path.
    fn expand_home(mut self) -> Self {
        if let Some(rest) = self.vault.path.to_str().and_then(|s| s.strip_prefix("~/")) {
            if let Some(home) = std::env::var_os("HOME") {
                self.vault.path = PathBuf::from(home).join(rest);
            }
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        let cfg = Config::default();
        assert_eq!(cfg.behavior.max_trades_per_day, 4);
        assert!(cfg.output.write_exchange_json);
    }

    #[test]
    fn session_windows() {
        let s = SessionsConfig::default();
        let t = |h, m| NaiveTime::from_hms_opt(h, m, 0).unwrap();
        assert_eq!(s.session_for(t(3, 0)), Session::Asia);
        assert_eq!(s.session_for(t(8, 30)), Session::London);
        assert_eq!(s.session_for(t(14, 0)), Session::Overlap);
        assert_eq!(s.session_for(t(18, 0)), Session::NewYork);
        assert_eq!(s.session_for(t(23, 0)), Session::Other);
    }

    #[test]
    fn session_window_wraps_midnight() {
        let s = SessionsConfig {
            asia: "23:00-07:00".to_string(),
            ..Default::default()
        };
        let t = |h, m| NaiveTime::from_hms_opt(h, m, 0).unwrap();
        assert_eq!(s.session_for(t(23, 30)), Session::Asia);
        assert_eq!(s.session_for(t(2, 0)), Session::Asia);
    }

    #[test]
    fn partial_toml_uses_defaults() {
        let cfg: Config = toml::from_str("[vault]\npath = \"/tmp/vault\"\n").unwrap();
        assert_eq!(cfg.vault.path, PathBuf::from("/tmp/vault"));
        assert_eq!(cfg.behavior.max_trades_per_day, 4);
    }

    #[test]
    fn mapping_ref_resolution() {
        let dir = std::env::temp_dir().join("trade-analyst-profiles-test");
        let profiles = dir.join("profiles");
        std::fs::create_dir_all(&profiles).unwrap();
        std::fs::write(
            profiles.join("mybroker.toml"),
            "[mapping]\npnl = \"Profit\"\n",
        )
        .unwrap();
        let direct = dir.join("direct.toml");
        std::fs::write(&direct, "[mapping]\npnl = \"PL\"\n").unwrap();

        // existing path wins as-is
        assert_eq!(
            CsvConfig::resolve_mapping_ref(&direct, &profiles).unwrap(),
            direct
        );
        // bare name resolves into the profile library, with or without .toml
        assert_eq!(
            CsvConfig::resolve_mapping_ref(Path::new("mybroker"), &profiles).unwrap(),
            profiles.join("mybroker.toml")
        );
        assert_eq!(
            CsvConfig::resolve_mapping_ref(Path::new("mybroker.toml"), &profiles).unwrap(),
            profiles.join("mybroker.toml")
        );
        // unknown name fails loudly
        assert!(CsvConfig::resolve_mapping_ref(Path::new("nope"), &profiles).is_err());
    }

    #[test]
    fn mapping_file_merge() {
        let base: CsvConfig = toml::from_str(
            "delimiter = \";\"\n[mapping]\npnl = \"Winst\"\nsymbol = \"Instrument\"\n",
        )
        .unwrap();
        let overlay: CsvConfig = toml::from_str(
            "date_format = \"%Y-%m-%d %H:%M:%S\"\n[mapping]\npnl = \"Profit\"\ndate = \"col:1\"\n",
        )
        .unwrap();
        let merged = base.merged_with(&overlay);
        assert_eq!(merged.delimiter.as_deref(), Some(";"));
        assert_eq!(merged.date_format.as_deref(), Some("%Y-%m-%d %H:%M:%S"));
        assert_eq!(merged.mapping["pnl"], "Profit"); // overlay wins
        assert_eq!(merged.mapping["symbol"], "Instrument"); // base kept
        assert_eq!(merged.mapping["date"], "col:1");
    }
}
