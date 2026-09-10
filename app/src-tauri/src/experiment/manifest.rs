#![allow(dead_code)]
//! Experiment manifest: source declaration and resolved matrix.
//!
//! The source manifest (`experiments/starter-modules.json`) declares
//! seeds, rosters, tasks, and variants. [`Manifest`] deserializes it;
//! [`expand_matrix`] produces the full list of [`TrialKey`]s.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Source manifest (matches JSON schema)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub struct Manifest {
    pub schema: u32,
    pub seeds: Vec<u32>,
    pub bots: Vec<u32>,
    pub tasks: Vec<String>,
    pub variants: Vec<String>,
    pub repetitions: u32,
    pub surface: String,
    pub peaceful: bool,
    pub visibility: String,
    pub game_speed: u32,
    pub pause_during_planning: bool,
    pub game_tick_limit: u64,
    pub trial_wall_seconds: u64,
    pub planning_deadline_ms: u64,
    pub candidate_limit: usize,
    pub support_ticks: u32,
    pub budget_maxima: BTreeMap<String, u64>,
    pub library_initial_state: String,
    pub reset_policy: String,
    pub sink_policy: String,
}

// ---------------------------------------------------------------------------
// Trial key
// ---------------------------------------------------------------------------

/// Uniquely identifies one trial in the matrix.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct TrialKey {
    pub seed: u32,
    pub bots: u32,
    pub task: String,
    pub variant: String,
    pub repetition: u32,
}

// ---------------------------------------------------------------------------
// Manifest errors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum ManifestError {
    InvalidField(String),
    MissingProvenance(String),
    IncompatibleRuntime(String),
    IoError(String),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::InvalidField(s) => write!(f, "invalid manifest field: {s}"),
            ManifestError::MissingProvenance(s) => write!(f, "missing provenance: {s}"),
            ManifestError::IncompatibleRuntime(s) => write!(f, "incompatible runtime: {s}"),
            ManifestError::IoError(s) => write!(f, "I/O error: {s}"),
        }
    }
}

impl std::error::Error for ManifestError {}

// ---------------------------------------------------------------------------
// Matrix expansion
// ---------------------------------------------------------------------------

/// Expand a manifest into the full list of trial keys.
#[allow(dead_code)]
pub fn expand_matrix(manifest: &Manifest) -> Result<Vec<TrialKey>, ManifestError> {
    // Validate limits.
    if manifest.seeds.is_empty() {
        return Err(ManifestError::InvalidField("seeds must not be empty".into()));
    }
    if manifest.bots.is_empty() {
        return Err(ManifestError::InvalidField("bots must not be empty".into()));
    }
    if manifest.tasks.is_empty() {
        return Err(ManifestError::InvalidField("tasks must not be empty".into()));
    }
    if manifest.variants.is_empty() {
        return Err(ManifestError::InvalidField("variants must not be empty".into()));
    }
    if manifest.repetitions == 0 {
        return Err(ManifestError::InvalidField("repetitions must be > 0".into()));
    }
    if manifest.candidate_limit == 0 {
        return Err(ManifestError::InvalidField("candidate_limit must be > 0".into()));
    }

    let mut trials = Vec::new();
    for seed in &manifest.seeds {
        for bots in &manifest.bots {
            for task in &manifest.tasks {
                for variant in &manifest.variants {
                    for rep in 0..manifest.repetitions {
                        trials.push(TrialKey {
                            seed: *seed,
                            bots: *bots,
                            task: task.clone(),
                            variant: variant.clone(),
                            repetition: rep,
                        });
                    }
                }
            }
        }
    }
    Ok(trials)
}

// ---------------------------------------------------------------------------
// Resolved manifest
// ---------------------------------------------------------------------------

/// A manifest that has been prepared (resolved against the current
/// environment) and is ready to run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ResolvedManifest {
    pub schema: u32,
    pub source: Manifest,
    pub git_commit: String,
    pub mod_versions: BTreeMap<String, String>,
    pub prototype_hash: String,
    pub per_case_saves: BTreeMap<String, String>,
    pub module_library_hash: String,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_declared_matrix_has_no_hidden_extra_trials() {
        let raw = include_str!("../../../../experiments/starter-modules.json");
        let manifest: Manifest = serde_json::from_str(raw).unwrap();
        let trials = expand_matrix(&manifest).unwrap();
        // Expected: 10 seeds * 2 bot counts * 2 tasks * 3 variants * 3 reps = 360
        assert_eq!(trials.len(), 360);
    }

    #[test]
    fn empty_seeds_is_rejected() {
        let manifest = Manifest {
            schema: 1,
            seeds: vec![],
            bots: vec![1, 4],
            tasks: vec!["automation".into()],
            variants: vec!["legacy".into()],
            repetitions: 1,
            surface: "nauvis".into(),
            peaceful: true,
            visibility: "explored-only".into(),
            game_speed: 10,
            pause_during_planning: true,
            game_tick_limit: 216000,
            trial_wall_seconds: 1800,
            planning_deadline_ms: 120000,
            candidate_limit: 8,
            support_ticks: 18000,
            budget_maxima: BTreeMap::new(),
            library_initial_state: "warm".into(),
            reset_policy: "same-prepared-save-per-case".into(),
            sink_policy: "available-red-only-research-by-name".into(),
        };
        assert!(matches!(expand_matrix(&manifest), Err(ManifestError::InvalidField(_))));
    }

    #[test]
    fn trial_count_matches_calculation() {
        let manifest = Manifest {
            schema: 1,
            seeds: vec![31337, 104729],
            bots: vec![1, 4],
            tasks: vec!["automation".into(), "red-delivery".into()],
            variants: vec!["legacy".into(), "modules-cache-on".into()],
            repetitions: 2,
            surface: "nauvis".into(),
            peaceful: true,
            visibility: "explored-only".into(),
            game_speed: 10,
            pause_during_planning: true,
            game_tick_limit: 216000,
            trial_wall_seconds: 1800,
            planning_deadline_ms: 120000,
            candidate_limit: 8,
            support_ticks: 18000,
            budget_maxima: BTreeMap::new(),
            library_initial_state: "warm".into(),
            reset_policy: "same-prepared-save-per-case".into(),
            sink_policy: "available-red-only-research-by-name".into(),
        };
        let trials = expand_matrix(&manifest).unwrap();
        // 2 seeds * 2 bot counts * 2 tasks * 2 variants * 2 reps = 32
        assert_eq!(trials.len(), 32);
    }
}
