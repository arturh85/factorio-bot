//! Isolated trial runner for frozen experiments.
//!
//! Executes each trial from the expanded matrix, managing process lifecycle
//! and recording outcomes.

use crate::experiment::manifest::{Manifest, ResolvedManifest, TrialKey, ManifestError};

/// Outcome of a single trial.
#[derive(Debug, Clone, PartialEq)]
pub enum TrialOutcome {
    Success,
    Failure,
    Timeout,
    Unsupported,
    Invalid,
}

/// Record of one completed trial.
#[derive(Debug, Clone)]
pub struct TrialResult {
    pub key: TrialKey,
    pub manifest_hash: String,
    pub outcome: TrialOutcome,
    pub game_ticks: Option<u64>,
    pub planning_ms: Option<u64>,
    pub wall_ms: u64,
    pub reason: Option<String>,
}

/// Execute the trial matrix defined by a prepared manifest.
///
/// For each trial, spawns an isolated process (headless server + planner)
/// with the trial's parameters, waits for completion or timeout, and
/// records the outcome.
pub fn run_experiment(
    _manifest: &ResolvedManifest,
    _output_dir: &str,
    _smoke: bool,
) -> Result<Vec<TrialResult>, ManifestError> {
    // Placeholder: returns empty results.
    // Full implementation will:
    // 1. Iterate over expanded trial keys
    // 2. For each trial, set up a unique workspace with the prepared save
    // 3. Launch a headless Factorio server with the trial parameters
    // 4. Run the planner/benchmark script
    // 5. Monitor game ticks and wall time
    // 6. Record outcome and clean up
    // 7. Repeat for all trials
    Ok(Vec::new())
}
