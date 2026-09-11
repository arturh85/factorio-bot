//! Frozen experiment runner for paired planner comparison.
//!
//! Executes the manifest-defined trial matrix (seeds, rosters, variants)
//! with isolated processes, records outcomes, and produces a comparison
//! report. This is the phase-0–2 research platform for measuring the
//! module-backed planner against the baseline.

pub mod manifest;
pub mod runner;
pub mod report;
pub mod offline;
