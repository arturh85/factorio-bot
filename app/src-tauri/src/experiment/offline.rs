//! Offline experiment runner for planner comparison.
//!
//! Compares module vs legacy planning using dumped worlds, without Factorio.
//! Produces trial results that can be fed into the report system.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use factorio_bot_core::miette::{IntoDiagnostic, Result as MResult, miette};
use factorio_bot_core::serde_json;
use factorio_bot_core::types::Position;

use crate::experiment::manifest::{Manifest, TrialKey, ManifestError, expand_matrix};
use crate::experiment::runner::{TrialOutcome, TrialResult};

/// Run an offline experiment against dumped worlds.
///
/// For each trial, loads the corresponding map dump and plans it with both
/// legacy and module mode, recording the faster plan.
pub fn run_offline_experiment(
    manifest: &Manifest,
    map_dir: &Path,
    output_dir: &Path,
) -> Vec<TrialResult> {
    let trial_keys = match expand_matrix(manifest) {
        Ok(keys) => keys,
        Err(e) => {
            eprintln!("Failed to expand manifest: {:?}", e);
            return vec![];
        }
    };

    let mut results = Vec::new();
    for trial_key in &trial_keys {
        let result = run_single_trial(trial_key, map_dir);
        results.push(result);
    }

    // Write results CSV
    let csv_path = output_dir.join("trials.csv");
    if let Ok(mut csv) = std::fs::File::create(&csv_path) {
        use std::io::Write;
        let _ = writeln!(csv, "seed,bots,task,variant,repetition,outcome,game_ticks,planning_ms,wall_ms,reason");
        for r in &results {
            let _ = writeln!(
                csv,
                "{},{},{},{},{},{:?},{},{},{},{}",
                r.key.seed, r.key.bots, r.key.task, r.key.variant, r.key.repetition,
                r.outcome,
                r.game_ticks.map_or("".into(), |t| t.to_string()),
                r.planning_ms.map_or("".into(), |t| t.to_string()),
                r.wall_ms,
                r.reason.as_deref().unwrap_or(""),
            );
        }
    }

    results
}

fn run_single_trial(key: &TrialKey, map_dir: &Path) -> TrialResult {
    let map_path = map_dir.join(format!("map-{}.json", key.seed));

    // Load world
    let world_data = match std::fs::read_to_string(&map_path) {
        Ok(d) => d,
        Err(e) => {
            return TrialResult {
                key: key.clone(),
                manifest_hash: String::new(),
                outcome: TrialOutcome::Invalid,
                game_ticks: None,
                planning_ms: None,
                wall_ms: 0,
                reason: Some(format!("missing map: {e}")),
            };
        }
    };

    let start = Instant::now();
    let world: factorio_bot_core::factorio::world::FactorioSurface = match serde_json::from_str(&world_data) {
        Ok(w) => w,
        Err(e) => {
            return TrialResult {
                key: key.clone(),
                manifest_hash: String::new(),
                outcome: TrialOutcome::Invalid,
                game_ticks: None,
                planning_ms: None,
                wall_ms: 0,
                reason: Some(format!("parse error: {e}")),
            };
        }
    };

    let bots: Vec<factorio_bot_planner::BotId> = (1..=key.bots)
        .map(|i| factorio_bot_planner::BotId(i as u8))
        .collect();

    let state = factorio_bot_planner::PlanState::from_world(
        std::sync::Arc::new(world),
        &bots,
    );

    let chain_actor = match factorio_bot_planner::pick_chain_actor(&state, &bots) {
        Some(a) => a,
        None => {
            return TrialResult {
                key: key.clone(),
                manifest_hash: String::new(),
                outcome: TrialOutcome::Invalid,
                game_ticks: None,
                planning_ms: None,
                wall_ms: 0,
                reason: Some("no chain actor available".into()),
            };
        }
    };

    let registry = factorio_bot_planner::registry_for(&bots);

    // Decide which planner variant to use
    let use_modules = match key.variant.as_str() {
        "modules" | "modules-cache-on" | "modules-cache-off" => true,
        _ => false,
    };
    let result = if use_modules {
        factorio_bot_planner::plan_best_modules(
            &[make_goal(&key.task)],
            &state,
            &registry,
            chain_actor,
            &bots,
        )
    } else {
        factorio_bot_planner::plan_best(
            &[make_goal(&key.task)],
            &state,
            &registry,
            chain_actor,
            &bots,
        )
    };

    let elapsed_ms = start.elapsed().as_millis() as u64;

    match result {
        Ok((_net, sched, _mem)) => {
            let makespan = sched.steps.iter().map(|s| s.end).max().unwrap_or(0) as u64;
            TrialResult {
                key: key.clone(),
                manifest_hash: String::new(),
                outcome: TrialOutcome::Success,
                game_ticks: Some(makespan),
                planning_ms: Some(elapsed_ms),
                wall_ms: elapsed_ms,
                reason: None,
            }
        }
        Err(e) => TrialResult {
            key: key.clone(),
            manifest_hash: String::new(),
            outcome: TrialOutcome::Failure,
            game_ticks: None,
            planning_ms: Some(elapsed_ms),
            wall_ms: elapsed_ms,
            reason: Some(e.to_string()),
        },
    }
}

fn make_goal(task: &str) -> factorio_bot_planner::Goal {
    match task {
        "automation" => factorio_bot_planner::Goal::Researched("automation".into()),
        "red-delivery" => factorio_bot_planner::Goal::Produced {
            item: "automation-science-pack".into(),
            count: 10,
            whose: factorio_bot_planner::Holder::Anyone,
            unlocks: None,
            via: None,
        },
        "iron-plate" => factorio_bot_planner::Goal::Have {
            item: "iron-plate".into(),
            count: 5,
            whose: factorio_bot_planner::Holder::Anyone,
            via: None,
        },
        other => factorio_bot_planner::Goal::Have {
            item: other.into(),
            count: 1,
            whose: factorio_bot_planner::Holder::Anyone,
            via: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::experiment::manifest::Manifest;

    #[test]
    fn offline_trial_produces_a_result() {
        // Ensure map file exists
        let map_dir = std::path::Path::new("workspace/scripts");
        let map_path = map_dir.join("map.json");
        if !map_path.exists() {
            eprintln!("Skipping: no map.json at {map_path:?}");
            return;
        }

        let manifest = Manifest {
            schema: 1,
            seeds: vec![31337],
            bots: vec![1],
            tasks: vec!["iron-plate".into()],
            variants: vec!["legacy".into()],
            repetitions: 1,
            surface: "nauvis".into(),
            peaceful: true,
            visibility: "normal".into(),
            game_speed: 1,
            pause_during_planning: true,
            game_tick_limit: 100000,
            trial_wall_seconds: 60,
            planning_deadline_ms: 30000,
            candidate_limit: 4,
            support_ticks: 18000,
            budget_maxima: BTreeMap::new(),
            library_initial_state: "empty".into(),
            reset_policy: "recreate".into(),
            sink_policy: "discard".into(),
        };

        let result = run_single_trial(
            &TrialKey {
                seed: 31337,
                bots: 1,
                task: "iron-plate".into(),
                variant: "legacy".into(),
                repetition: 0,
            },
            std::path::Path::new("workspace/scripts"),
        );
        assert_eq!(result.outcome, TrialOutcome::Success, "trial failed: {:?}", result.reason);
    }
}
