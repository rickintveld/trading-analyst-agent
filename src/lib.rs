//! Trade Analyst — deterministic trading analytics engine.
//!
//! Architecture: Rust computes every number (parsing, statistics, scores,
//! behavioral flags, history). The AI layer (Claude Code) only *interprets*
//! the fixed JSON exchange format produced by [`analyzer`].
//!
//! Module responsibilities (Clean Architecture, one responsibility each):
//! - [`charts`]     Obsidian Charts plugin output (deterministic chart specs)
//! - [`shared`]     core domain types (Trade, Direction, Session)
//! - [`config`]     TOML configuration
//! - [`parser`]     CSV ingestion with format auto-detection
//! - [`statistics`] deterministic performance / period statistics
//! - [`behavior`]   rule-based behavioral pattern detection
//! - [`scoring`]    deterministic 0-100 scores
//! - [`ai`]         the fixed JSON exchange schema (the Rust<->LLM contract)
//! - [`history`]    historical aggregation & rolling averages
//! - [`analyzer`]   orchestration: trades -> AiExchange
//! - [`reports`]    Markdown report rendering (fixed section order)
//! - [`obsidian`]   Obsidian vault integration (analyses, dashboard)
//! - [`cli`]        command-line interface

pub mod ai;
pub mod analyzer;
pub mod behavior;
pub mod charts;
pub mod cli;
pub mod config;
pub mod history;
pub mod obsidian;
pub mod parser;
pub mod reports;
pub mod scoring;
pub mod shared;
pub mod statistics;

/// Version of the JSON exchange schema (the Rust <-> AI contract).
/// Bump the minor for backwards-compatible additions, the major for breaks.
/// 1.1.0: added `time_series`, `session_statistics.sessions[].average_rr`,
///        `historical_comparison.series` (all additive).
pub const SCHEMA_VERSION: &str = "1.1.0";

/// Engine version, taken from Cargo.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
