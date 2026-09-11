#![allow(dead_code)]
//! Comparison report from experiment results.
//!
//! Produces paired comparisons between planner variants, trial summaries,
//! and the final research report. The report is emitted as:
//! - `trials.csv` — one row per trial
//! - `comparison.json` — aggregated statistics by variant/task
//! - `report.md` — human-readable summary

use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::experiment::runner::{TrialOutcome, TrialResult};

// ---------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------

/// Aggregated comparison of results across planner variants.
#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
pub struct Comparison {
    /// Number of declared trials per (variant, task).
    pub declared_counts: BTreeMap<String, usize>,
    /// Number of observed (completed) trials per (variant, task).
    pub observed_counts: BTreeMap<String, usize>,
    /// Distribution of outcomes across all trials.
    pub outcome_distribution: BTreeMap<String, usize>,
    /// Paired successful trial times, keyed by (seed, roster, task, rep).
    pub paired_times: BTreeMap<String, PairedTime>,
    /// Variants that were compared.
    pub variants: Vec<String>,
    /// Total successful trials.
    pub successful_trials: usize,
    /// Total failed/timeout trials.
    pub failed_trials: usize,
}

/// A paired time comparison between two variants for the same seed/roster/task.
#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
pub struct PairedTime {
    pub seed: u32,
    pub bots: u32,
    pub task: String,
    pub repetition: u32,
    pub baseline_ticks: Option<u64>,
    pub experimental_ticks: Option<u64>,
    pub baseline_planning_ms: Option<u64>,
    pub experimental_planning_ms: Option<u64>,
}

// ---------------------------------------------------------------------------
// Report error
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum ReportError {
    MixedManifests(String),
    DuplicateRows(String),
    InvalidResult(String),
    IoError(String),
}

impl std::fmt::Display for ReportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReportError::MixedManifests(s) => write!(f, "mixed manifests: {s}"),
            ReportError::DuplicateRows(s) => write!(f, "duplicate rows: {s}"),
            ReportError::InvalidResult(s) => write!(f, "invalid result: {s}"),
            ReportError::IoError(s) => write!(f, "I/O error: {s}"),
        }
    }
}

impl std::error::Error for ReportError {}

// ---------------------------------------------------------------------------
// compare
// ---------------------------------------------------------------------------

/// Aggregate results into a [`Comparison`].
///
/// Groups results by variant, counts outcomes, and pairs successful trials
/// by (seed, roster, task, repetition) for the first two variants found.
#[allow(dead_code)]
pub fn compare(results: &[TrialResult]) -> Result<Comparison, ReportError> {
    if results.is_empty() {
        return Ok(Comparison {
            declared_counts: BTreeMap::new(),
            observed_counts: BTreeMap::new(),
            outcome_distribution: BTreeMap::new(),
            paired_times: BTreeMap::new(),
            variants: vec![],
            successful_trials: 0,
            failed_trials: 0,
        });
    }

    // Collect variants.
    let manifest_hash = results[0].manifest_hash.clone();
    let mut variants: Vec<String> = Vec::new();
    let mut variant_set = BTreeSet::new();
    for result in results {
        if result.manifest_hash != manifest_hash {
            return Err(ReportError::MixedManifests(format!(
                "expected '{}', got '{}'",
                manifest_hash, result.manifest_hash
            )));
        }
        if variant_set.insert(result.key.variant.clone()) {
            variants.push(result.key.variant.clone());
        }
    }
    variants.sort();

    // Count outcomes.
    let mut declared: BTreeMap<String, usize> = BTreeMap::new();
    let mut observed: BTreeMap<String, usize> = BTreeMap::new();
    let mut outcomes: BTreeMap<String, usize> = BTreeMap::new();
    let mut successful = 0usize;
    let mut failed = 0usize;

    for result in results {
        let key = format!("{}/{}", result.key.variant, result.key.task);
        *declared.entry(key.clone()).or_default() += 1;
        if result.outcome != TrialOutcome::Invalid {
            *observed.entry(key).or_default() += 1;
        }
        *outcomes.entry(format!("{:?}", result.outcome)).or_default() += 1;
        match result.outcome {
            TrialOutcome::Success => successful += 1,
            TrialOutcome::Failure | TrialOutcome::Timeout => failed += 1,
            _ => {}
        }
    }

    // Pair successful trials by (seed, roster, task, repetition).
    let mut paired: BTreeMap<String, PairedTime> = BTreeMap::new();
    if variants.len() >= 2 {
        let baseline = &variants[0];
        let experimental = &variants[1];

        // Index results by variant.
        let mut by_variant: BTreeMap<String, Vec<&TrialResult>> = BTreeMap::new();
        for result in results {
            by_variant.entry(result.key.variant.clone()).or_default().push(result);
        }

        for b_result in by_variant.get(baseline).into_iter().flatten() {
            let pair_key = format!(
                "{}/{}/{}/{}",
                b_result.key.seed, b_result.key.bots, b_result.key.task, b_result.key.repetition
            );
            if !matches!(b_result.outcome, TrialOutcome::Success) {
                continue;
            }
            let e_result = by_variant.get(experimental).into_iter().flatten().find(|r| {
                r.key.seed == b_result.key.seed
                    && r.key.bots == b_result.key.bots
                    && r.key.task == b_result.key.task
                    && r.key.repetition == b_result.key.repetition
                    && matches!(r.outcome, TrialOutcome::Success)
            });
            if e_result.is_some() {
                paired.insert(pair_key, PairedTime {
                    seed: b_result.key.seed,
                    bots: b_result.key.bots,
                    task: b_result.key.task.clone(),
                    repetition: b_result.key.repetition,
                    baseline_ticks: b_result.game_ticks,
                    experimental_ticks: e_result.and_then(|r| r.game_ticks),
                    baseline_planning_ms: b_result.planning_ms,
                    experimental_planning_ms: e_result.and_then(|r| r.planning_ms),
                });
            }
        }
    }

    Ok(Comparison {
        declared_counts: declared,
        observed_counts: observed,
        outcome_distribution: outcomes,
        paired_times: paired,
        variants,
        successful_trials: successful,
        failed_trials: failed,
    })
}

// ---------------------------------------------------------------------------
// Report output
// ---------------------------------------------------------------------------

/// Write the comparison report to a directory.
///
/// Emits:
/// - `trials.csv` — one row per trial
/// - `comparison.json` — aggregated results
/// - `report.md` — human-readable summary
#[allow(dead_code)]
pub fn write_report(
    results: &[TrialResult],
    comparison: &Comparison,
    output_dir: &Path,
) -> Result<(), ReportError> {
    use std::fs;
    use std::io::Write;

    fs::create_dir_all(output_dir)
        .map_err(|e| ReportError::IoError(e.to_string()))?;

    // trials.csv
    let csv_path = output_dir.join("trials.csv");
    let mut csv = fs::File::create(&csv_path)
        .map_err(|e| ReportError::IoError(e.to_string()))?;
    writeln!(csv, "seed,bots,task,variant,repetition,outcome,game_ticks,planning_ms,wall_ms,reason")
        .map_err(|e| ReportError::IoError(e.to_string()))?;
    for result in results {
        writeln!(
            csv,
            "{},{},{},{},{},{:?},{},{},{},{}",
            result.key.seed,
            result.key.bots,
            result.key.task,
            result.key.variant,
            result.key.repetition,
            result.outcome,
            result.game_ticks.map_or("".into(), |t| t.to_string()),
            result.planning_ms.map_or("".into(), |t| t.to_string()),
            result.wall_ms,
            result.reason.as_deref().unwrap_or(""),
        )
        .map_err(|e| ReportError::IoError(e.to_string()))?;
    }

    // comparison.json
    let json_path = output_dir.join("comparison.json");
    let json = factorio_bot_core::serde_json::to_string_pretty(comparison)
        .map_err(|e| ReportError::IoError(e.to_string()))?;
    fs::write(&json_path, json)
        .map_err(|e| ReportError::IoError(e.to_string()))?;

    // report.md
    let md_path = output_dir.join("report.md");
    let mut md = fs::File::create(&md_path)
        .map_err(|e| ReportError::IoError(e.to_string()))?;

    writeln!(md, "# Experiment Report\n").unwrap();
    writeln!(md, "## Summary\n").unwrap();
    writeln!(md, "- Total trials: {}", results.len()).unwrap();
    writeln!(md, "- Successful: {}", comparison.successful_trials).unwrap();
    writeln!(md, "- Failed/timeout: {}", comparison.failed_trials).unwrap();
    writeln!(md).unwrap();

    writeln!(md, "## Outcome Distribution\n").unwrap();
    writeln!(md, "| Outcome | Count |").unwrap();
    writeln!(md, "|---|---|").unwrap();
    for (outcome, count) in &comparison.outcome_distribution {
        writeln!(md, "| {} | {} |", outcome, count).unwrap();
    }
    writeln!(md).unwrap();

    if !comparison.paired_times.is_empty() {
        writeln!(md, "## Paired Comparisons\n").unwrap();
        writeln!(md, "| Seed | Bots | Task | Baseline ticks | Experimental ticks | Difference |").unwrap();
        writeln!(md, "|---|---|---|---|---|---|").unwrap();
        for (_key, pt) in &comparison.paired_times {
            let diff = match (pt.baseline_ticks, pt.experimental_ticks) {
                (Some(b), Some(e)) => format!("{}", e as i64 - b as i64),
                _ => "-".into(),
            };
            writeln!(
                md,
                "| {} | {} | {} | {} | {} | {} |",
                pt.seed,
                pt.bots,
                pt.task,
                pt.baseline_ticks.map_or("-".into(), |t| t.to_string()),
                pt.experimental_ticks.map_or("-".into(), |t| t.to_string()),
                diff,
            )
            .unwrap();
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// First-rocket report
// ---------------------------------------------------------------------------

/// Write a first-rocket experiment report.
#[allow(dead_code)]
pub fn write_first_rocket_report(
    results: &[TrialResult],
    output_dir: &Path,
) -> Result<(), ReportError> {
    use std::fs;
    use std::io::Write;

    fs::create_dir_all(output_dir)
        .map_err(|e| ReportError::IoError(e.to_string()))?;

    let md_path = output_dir.join("report.md");
    let mut md = fs::File::create(&md_path)
        .map_err(|e| ReportError::IoError(e.to_string()))?;

    writeln!(md, "# First Rocket Experiment Report
").unwrap();

    // Summary
    let total = results.len();
    let successes = results.iter().filter(|r| r.outcome == TrialOutcome::Success).count();
    let failures = results.iter().filter(|r| r.outcome == TrialOutcome::Failure).count();
    let timeouts = results.iter().filter(|r| r.outcome == TrialOutcome::Timeout).count();
    let invalid = results.iter().filter(|r| r.outcome == TrialOutcome::Invalid).count();

    writeln!(md, "## Summary
").unwrap();
    writeln!(md, "| Metric | Value |").unwrap();
    writeln!(md, "|---|---|").unwrap();
    writeln!(md, "| Total trials | {} |", total).unwrap();
    writeln!(md, "| Successful | {} |", successes).unwrap();
    writeln!(md, "| Failed | {} |", failures).unwrap();
    writeln!(md, "| Timeout | {} |", timeouts).unwrap();
    writeln!(md, "| Invalid | {} |", invalid).unwrap();
    writeln!(md).unwrap();

    // Per-variant breakdown
    writeln!(md, "## Per-Variant Results
").unwrap();
    writeln!(md, "| Variant | Trials | Success | Failure | Timeout |").unwrap();
    writeln!(md, "|---|---|---|---|---|").unwrap();

    let mut variants: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for r in results {
        variants.insert(r.key.variant.clone());
    }
    for variant in &variants {
        let v_trials: Vec<&TrialResult> = results.iter().filter(|r| &r.key.variant == variant).collect();
        let v_total = v_trials.len();
        let v_ok = v_trials.iter().filter(|r| r.outcome == TrialOutcome::Success).count();
        let v_fail = v_trials.iter().filter(|r| r.outcome == TrialOutcome::Failure).count();
        let v_time = v_trials.iter().filter(|r| r.outcome == TrialOutcome::Timeout).count();
        writeln!(md, "| {} | {} | {} | {} | {} |", variant, v_total, v_ok, v_fail, v_time).unwrap();
    }
    writeln!(md).unwrap();

    // Detailed trial results
    writeln!(md, "## Detailed Results
").unwrap();
    writeln!(md, "| # | Seed | Bots | Variant | Outcome | Ticks | Planning | Wall | Reason |").unwrap();
    writeln!(md, "|---|---|---|---|---|---|---|---|---|").unwrap();

    for (i, result) in results.iter().enumerate() {
        let ticks = result.game_ticks.map_or("-".into(), |t| format!("{} ({:.1}m)", t, t as f64 / 3600.0 / 60.0));
        let planning = result.planning_ms.map_or("-".into(), |t| format!("{} ms", t));
        let wall = format!("{} s", result.wall_ms / 1000);
        let reason = result.reason.as_deref().unwrap_or("-");
        writeln!(
            md,
            "| {} | {} | {} | {} | {:?} | {} | {} | {} | {} |",
            i + 1, result.key.seed, result.key.bots, result.key.variant,
            result.outcome, ticks, planning, wall, reason
        ).unwrap();
    }
    writeln!(md).unwrap();

    // Terminal evidence
    let has_evidence: Vec<&TrialResult> = results.iter()
        .filter(|r| !r.terminal_evidence_paths.is_empty()).collect();
    if !has_evidence.is_empty() {
        writeln!(md, "## Terminal Evidence
").unwrap();
        for result in &has_evidence {
            writeln!(md, "- Seed {} variant {}:", result.key.seed, result.key.variant).unwrap();
            for path in &result.terminal_evidence_paths {
                writeln!(md, "  - `{}`", path).unwrap();
            }
        }
        writeln!(md).unwrap();
    }

    // Per-seed timing
    writeln!(md, "## Timing by Seed
").unwrap();
    let mut seeds: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    for r in results {
        seeds.insert(r.key.seed);
    }
    for seed in &seeds {
        let s_results: Vec<&TrialResult> = results.iter().filter(|r| r.key.seed == *seed).collect();
        let best_ticks = s_results.iter()
            .filter(|r| r.outcome == TrialOutcome::Success)
            .filter_map(|r| r.game_ticks)
            .min();
        if let Some(best) = best_ticks {
            writeln!(md, "- Seed {}: best {} ticks ({:.1}m)", seed, best, best as f64 / 3600.0 / 60.0).unwrap();
        } else {
            writeln!(md, "- Seed {}: no successful trials", seed).unwrap();
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::experiment::manifest::TrialKey;

    fn make_result(
        variant: &str,
        task: &str,
        seed: u32,
        bots: u32,
        rep: u32,
        outcome: TrialOutcome,
        ticks: Option<u64>,
    ) -> TrialResult {
        TrialResult {
            key: TrialKey {
                seed,
                bots,
                task: task.into(),
                variant: variant.into(),
                repetition: rep,
            },
            manifest_hash: "abc".into(),
            outcome,
            game_ticks: ticks,
            planning_ms: Some(500),
            wall_ms: 15000,
            reason: None,
            policy_hash: None,
            save_hash: None,
            terminal_evidence_paths: vec![],
            terminal_tick: None,
            initial_tick: None,
            planning_pause_ms: None,
            phase_times: BTreeMap::new(),
            active_wall_ms: None,
        }
    }

    #[test]
    fn compare_counts_success_and_failure() {
        let results = vec![
            make_result("legacy", "automation", 31337, 4, 0, TrialOutcome::Success, Some(22000)),
            make_result("modules", "automation", 31337, 4, 0, TrialOutcome::Failure, None),
        ];
        let comparison = compare(&results).unwrap();
        assert_eq!(comparison.successful_trials, 1);
        assert_eq!(comparison.failed_trials, 1);
        assert_eq!(comparison.paired_times.len(), 0); // experimental failed, no pair
    }

    #[test]
    fn paired_times_when_both_succeed() {
        let results = vec![
            make_result("legacy", "automation", 31337, 4, 0, TrialOutcome::Success, Some(22000)),
            make_result("modules", "automation", 31337, 4, 0, TrialOutcome::Success, Some(21000)),
        ];
        let comparison = compare(&results).unwrap();
        assert_eq!(comparison.paired_times.len(), 1);
        let pt = comparison.paired_times.values().next().unwrap();
        assert_eq!(pt.baseline_ticks, Some(22000));
        assert_eq!(pt.experimental_ticks, Some(21000));
    }

    #[test]
    fn duplicate_or_missing_manifest_is_rejected() {
        let results = vec![
            TrialResult {
                manifest_hash: "abc".into(),
                ..make_result("legacy", "automation", 31337, 4, 0, TrialOutcome::Success, None)
            },
            TrialResult {
                manifest_hash: "def".into(),
                ..make_result("modules", "automation", 31337, 4, 0, TrialOutcome::Success, None)
            },
        ];
        assert!(compare(&results).is_err());
    }

    #[test]
    fn empty_results_are_ok() {
        let comparison = compare(&[]).unwrap();
        assert_eq!(comparison.successful_trials, 0);
    }
}
