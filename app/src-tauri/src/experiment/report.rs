//! Comparison report from experiment results.
//!
//! Produces paired comparisons between planner variants, trial summaries,
//! and the final research report.

use crate::experiment::runner::{TrialOutcome, TrialResult};
use crate::experiment::manifest::{Manifest, TrialKey};

/// Comparison of results between two planner variants.
#[derive(Debug, Clone)]
pub struct Comparison {
    pub declared_counts: BTreeMap<String, usize>,
    pub observed_counts: BTreeMap<String, usize>,
    pub successful_trials: usize,
    pub failed_trials: usize,
    pub outcome_distribution: BTreeMap<String, usize>,
}

use std::collections::BTreeMap;

/// Compare results across planner variants.
pub fn compare(results: &[TrialResult]) -> Result<Comparison, ReportError> {
    let mut declared: BTreeMap<String, usize> = BTreeMap::new();
    let mut observed: BTreeMap<String, usize> = BTreeMap::new();
    let mut outcomes: BTreeMap<String, usize> = BTreeMap::new();

    for result in results {
        let key = format!("{}/{}", result.key.variant, result.key.task);
        *declared.entry(key.clone()).or_default() += 1;
        *observed.entry(key.clone()).or_default() += 1;
        *outcomes.entry(format!("{:?}", result.outcome)).or_default() += 1;
    }

    let successful = results.iter().filter(|r| matches!(r.outcome, TrialOutcome::Success)).count();
    let failed = results.iter().filter(|r| matches!(r.outcome, TrialOutcome::Failure | TrialOutcome::Timeout)).count();

    Ok(Comparison {
        declared_counts: declared,
        observed_counts: observed,
        successful_trials: successful,
        failed_trials: failed,
        outcome_distribution: outcomes,
    })
}

#[derive(Debug, Clone)]
pub enum ReportError {
    MixedManifests(String),
    DuplicateRows(String),
    InvalidResult(String),
}

impl std::fmt::Display for ReportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReportError::MixedManifests(s) => write!(f, "mixed manifests: {s}"),
            ReportError::DuplicateRows(s) => write!(f, "duplicate rows: {s}"),
            ReportError::InvalidResult(s) => write!(f, "invalid result: {s}"),
        }
    }
}

impl std::error::Error for ReportError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_counts_success_and_failure() {
        let results = vec![
            TrialResult {
                key: TrialKey {
                    seed: 31337, bots: 4,
                    task: "automation".into(),
                    variant: "legacy".into(),
                    repetition: 0,
                },
                manifest_hash: "abc".into(),
                outcome: TrialOutcome::Success,
                game_ticks: Some(22000),
                planning_ms: Some(500),
                wall_ms: 15000,
                reason: None,
            },
            TrialResult {
                key: TrialKey {
                    seed: 31337, bots: 4,
                    task: "automation".into(),
                    variant: "modules".into(),
                    repetition: 0,
                },
                manifest_hash: "abc".into(),
                outcome: TrialOutcome::Failure,
                game_ticks: None,
                planning_ms: Some(600),
                wall_ms: 20000,
                reason: Some("budget exhausted".into()),
            },
        ];
        let comparison = compare(&results).unwrap();
        assert_eq!(comparison.successful_trials, 1);
        assert_eq!(comparison.failed_trials, 1);
    }
}
