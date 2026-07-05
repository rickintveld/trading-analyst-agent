//! Core domain types shared by every module.

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Serialize};

/// Trade direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Long,
    Short,
}

impl Direction {
    /// Parse broker export variants: long/short, buy/sell, b/s, l/s.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "long" | "buy" | "b" | "l" | "bought" => Some(Self::Long),
            "short" | "sell" | "s" | "sold" => Some(Self::Short),
            _ => None,
        }
    }
}

/// Market session. `Overlap` is the London/New York overlap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Session {
    Asia,
    London,
    Overlap,
    NewYork,
    Other,
}

impl Session {
    pub const ALL: [Session; 5] = [
        Session::Asia,
        Session::London,
        Session::Overlap,
        Session::NewYork,
        Session::Other,
    ];

    /// Stable machine label used in the JSON exchange format.
    pub fn label(&self) -> &'static str {
        match self {
            Session::Asia => "asia",
            Session::London => "london",
            Session::Overlap => "overlap",
            Session::NewYork => "new_york",
            Session::Other => "other",
        }
    }

    /// Human label used in Markdown reports.
    pub fn display(&self) -> &'static str {
        match self {
            Session::Asia => "Asia",
            Session::London => "London",
            Session::Overlap => "Overlap",
            Session::NewYork => "New York",
            Session::Other => "Other",
        }
    }

    /// Parse a session value from a CSV column.
    pub fn parse(raw: &str) -> Option<Self> {
        let s = raw.trim().to_ascii_lowercase().replace([' ', '-', '_'], "");
        match s.as_str() {
            "asia" | "asian" | "tokyo" | "sydney" => Some(Session::Asia),
            "london" | "ldn" | "eu" | "europe" | "frankfurt" => Some(Session::London),
            "overlap" | "londonnewyork" | "londonny" | "nylondon" => Some(Session::Overlap),
            "newyork" | "ny" | "us" | "nyc" | "america" | "newyorkam" | "newyorkpm" => {
                Some(Session::NewYork)
            }
            "" => None,
            _ => Some(Session::Other),
        }
    }
}

/// A single normalized trade. Only `date`, `symbol` and `pnl` are required;
/// every analytics stage degrades gracefully when optional fields are absent.
#[derive(Debug, Clone, Serialize)]
pub struct Trade {
    /// 0-based row order in the source file (stable chronological tiebreaker).
    pub index: usize,
    pub date: NaiveDate,
    pub time: Option<NaiveTime>,
    pub symbol: String,
    pub direction: Option<Direction>,
    pub entry: Option<f64>,
    pub exit: Option<f64>,
    pub position_size: Option<f64>,
    pub risk: Option<f64>,
    pub reward: Option<f64>,
    /// Realized risk:reward multiple.
    pub rr: Option<f64>,
    pub pnl: f64,
    pub fees: Option<f64>,
    pub strategy: Option<String>,
    pub session: Option<Session>,
    pub notes: Option<String>,
}

impl Trade {
    /// Combined timestamp; midnight when no time column exists.
    pub fn datetime(&self) -> NaiveDateTime {
        self.date.and_time(
            self.time
                .unwrap_or_else(|| NaiveTime::from_hms_opt(0, 0, 0).unwrap()),
        )
    }

    pub fn is_win(&self) -> bool {
        self.pnl > PNL_EPSILON
    }

    pub fn is_loss(&self) -> bool {
        self.pnl < -PNL_EPSILON
    }
}

/// PnL values within this band of zero count as breakeven.
pub const PNL_EPSILON: f64 = 1e-9;

/// Sort trades chronologically (date, time, then source row order).
pub fn sort_chronologically(trades: &mut [Trade]) {
    trades.sort_by(|a, b| a.datetime().cmp(&b.datetime()).then(a.index.cmp(&b.index)));
}

/// Round to 2 decimals for stable JSON output (money values).
pub fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Round to 4 decimals (ratios, rates, scores components).
pub fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

/// Mean of a slice; `None` when empty.
pub fn mean(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        None
    } else {
        Some(values.iter().sum::<f64>() / values.len() as f64)
    }
}

/// Population standard deviation; `None` when empty.
pub fn std_dev(values: &[f64]) -> Option<f64> {
    let m = mean(values)?;
    let var = values.iter().map(|v| (v - m).powi(2)).sum::<f64>() / values.len() as f64;
    Some(var.sqrt())
}

/// Median of a slice; `None` when empty.
pub fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        Some((sorted[mid - 1] + sorted[mid]) / 2.0)
    } else {
        Some(sorted[mid])
    }
}

/// Clamp to the inclusive [0, 1] range.
pub fn clamp01(v: f64) -> f64 {
    v.clamp(0.0, 1.0)
}

/// Slope of a simple least-squares regression over (0..n) -> values.
/// `None` when fewer than 2 points.
pub fn linear_slope(values: &[f64]) -> Option<f64> {
    let n = values.len();
    if n < 2 {
        return None;
    }
    let n_f = n as f64;
    let mean_x = (n_f - 1.0) / 2.0;
    let mean_y = values.iter().sum::<f64>() / n_f;
    let mut num = 0.0;
    let mut den = 0.0;
    for (i, y) in values.iter().enumerate() {
        let dx = i as f64 - mean_x;
        num += dx * (y - mean_y);
        den += dx * dx;
    }
    if den == 0.0 {
        None
    } else {
        Some(num / den)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direction_parsing() {
        assert_eq!(Direction::parse("BUY"), Some(Direction::Long));
        assert_eq!(Direction::parse(" sell "), Some(Direction::Short));
        assert_eq!(Direction::parse("Long"), Some(Direction::Long));
        assert_eq!(Direction::parse("??"), None);
    }

    #[test]
    fn session_parsing() {
        assert_eq!(Session::parse("New York"), Some(Session::NewYork));
        assert_eq!(Session::parse("LONDON"), Some(Session::London));
        assert_eq!(Session::parse(""), None);
        assert_eq!(Session::parse("weird"), Some(Session::Other));
    }

    #[test]
    fn math_helpers() {
        assert_eq!(mean(&[1.0, 2.0, 3.0]), Some(2.0));
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), Some(2.5));
        assert!(std_dev(&[2.0, 2.0]).unwrap().abs() < 1e-12);
        assert!(linear_slope(&[1.0, 2.0, 3.0]).unwrap() - 1.0 < 1e-12);
        assert_eq!(linear_slope(&[1.0]), None);
        assert_eq!(round2(1.005001), 1.01);
    }
}
