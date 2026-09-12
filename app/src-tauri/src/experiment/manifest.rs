#![allow(dead_code)]
//! Experiment manifest: source declaration and resolved matrix.
//!
//! Two formats:
//! - Starter Modules format (`experiments/starter-modules.json`): seeds × bots ×
//!   tasks × variants × repetitions for planner comparison.
//! - First Rocket format (`experiments/first-rocket.json`): stages-based rocket
//!   launch milestone with policy hash, recipe preferences, and deadlines.
//!
//! [`expand_matrix`] produces the full list of [`TrialKey`]s from either format.
//! [`expand_first_rocket_trials`] produces trials from the first-rocket format.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Source manifest (matches JSON schema) — Starter modules format
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
// Policy stage — a single milestone stage (first-rocket format)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub struct PolicyStage {
  pub id: u32,
  pub name: String,
  #[serde(default)]
  pub deadline_ticks: Option<u64>,
  #[serde(default)]
  pub iron_target: Option<u32>,
  #[serde(default)]
  pub copper_target: Option<u32>,
  #[serde(default)]
  pub prerequisites: Vec<String>,
}

// ---------------------------------------------------------------------------
// First rocket manifest (matches experiments/first-rocket.json)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub struct FirstRocketManifest {
  pub schema: u32,
  pub name: String,
  pub description: String,
  pub seeds: Vec<u32>,
  pub bots: Vec<u32>,
  pub stages: u32,
  pub rocket_parts_required: u32,
  pub policy_hash: FirstRocketPolicy,
  pub recipe_preferences: BTreeMap<String, Vec<String>>,
  pub row_variants: BTreeMap<String, Vec<String>>,
  pub copy_budgets: BTreeMap<String, u32>,
  pub deadlines: BTreeMap<String, u64>,
  pub support_horizon: BTreeMap<String, u64>,
  pub research_priorities: Vec<ResearchBranch>,
  pub materials: BTreeMap<String, factorio_bot_core::serde_json::Value>,
  pub rate_export_distinction: BTreeMap<String, String>,
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
  pub reset_policy: String,
  pub sink_policy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub struct FirstRocketPolicy {
  pub stages: Vec<PolicyStage>,
  pub rocket_parts_required: u32,
  pub starter_pack: BTreeMap<String, u32>,
  pub foundation_cost: BTreeMap<String, u32>,
  pub total_payload_steel: u32,
  pub total_payload_cable: u32,
  pub rocket_part_ingredients: BTreeMap<String, u32>,
  pub default_bots: Vec<u32>,
  pub preferred_roles: BTreeMap<String, Vec<u32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub struct ResearchBranch {
  pub branch: u32,
  pub techs: Vec<String>,
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
// Matrix expansion — starter modules format
// ---------------------------------------------------------------------------

/// Expand a manifest into the full list of trial keys.
#[allow(dead_code)]
pub fn expand_matrix(manifest: &Manifest) -> Result<Vec<TrialKey>, ManifestError> {
  if manifest.seeds.is_empty() {
    return Err(ManifestError::InvalidField(
      "seeds must not be empty".into(),
    ));
  }
  if manifest.bots.is_empty() {
    return Err(ManifestError::InvalidField("bots must not be empty".into()));
  }
  if manifest.tasks.is_empty() {
    return Err(ManifestError::InvalidField(
      "tasks must not be empty".into(),
    ));
  }
  if manifest.variants.is_empty() {
    return Err(ManifestError::InvalidField(
      "variants must not be empty".into(),
    ));
  }
  if manifest.repetitions == 0 {
    return Err(ManifestError::InvalidField(
      "repetitions must be > 0".into(),
    ));
  }
  if manifest.candidate_limit == 0 {
    return Err(ManifestError::InvalidField(
      "candidate_limit must be > 0".into(),
    ));
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
// Matrix expansion — first rocket format
// ---------------------------------------------------------------------------

/// Expand a first-rocket manifest into trial keys.
/// Each trial is a (seed × 1-bot-config × task=rocket × variant=base) for the
/// diagnostic peaceful trial, plus default-enemy variants for the primary cohort.
#[allow(dead_code)]
pub fn expand_first_rocket_trials(
  manifest: &FirstRocketManifest,
  with_peaceful_diagnostic: bool,
) -> Result<Vec<TrialKey>, ManifestError> {
  if manifest.seeds.is_empty() {
    return Err(ManifestError::InvalidField(
      "seeds must not be empty".into(),
    ));
  }
  if manifest.bots.is_empty() {
    return Err(ManifestError::InvalidField("bots must not be empty".into()));
  }

  let mut trials = Vec::new();

  // Peaceful diagnostic (single trial)
  if with_peaceful_diagnostic {
    for seed in &manifest.seeds {
      for bots in &manifest.bots {
        trials.push(TrialKey {
          seed: *seed,
          bots: *bots,
          task: "rocket".into(),
          variant: "peaceful-diagnostic".into(),
          repetition: 0,
        });
      }
    }
  }

  // Primary default-enemy cohort: one repetition per seed × bots
  for seed in &manifest.seeds {
    for bots in &manifest.bots {
      trials.push(TrialKey {
        seed: *seed,
        bots: *bots,
        task: "rocket".into(),
        variant: "default-enemy".into(),
        repetition: 0,
      });
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

/// A resolved first-rocket manifest with fingerprints and save paths.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ResolvedFirstRocketManifest {
  pub schema: u32,
  pub name: String,
  pub description: String,
  pub seeds: Vec<u32>,
  pub bots: Vec<u32>,
  pub stages: u32,
  pub rocket_parts_required: u32,
  pub policy_hash: FirstRocketPolicy,
  pub deadlines: BTreeMap<String, u64>,
  pub game_tick_limit: u64,
  pub trial_wall_seconds: u64,
  pub planning_deadline_ms: u64,
  pub peaceful: bool,
  pub game_speed: u32,
  pub git_commit: String,
  pub mod_versions: BTreeMap<String, String>,
  pub prototype_hash: String,
  pub save_paths: BTreeMap<String, String>,
  pub binary_hash: String,
  pub script_hash: String,
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
    assert!(matches!(
      expand_matrix(&manifest),
      Err(ManifestError::InvalidField(_))
    ));
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
    assert_eq!(trials.len(), 32);
  }

  #[test]
  fn first_rocket_expansion_produces_peaceful_and_default_enemy() {
    let raw = include_str!("../../../../experiments/first-rocket.json");
    let manifest: FirstRocketManifest = serde_json::from_str(raw).unwrap();
    let trials = expand_first_rocket_trials(&manifest, true).unwrap();
    // 3 seeds × 1 bot count × 2 variants (peaceful-diagnostic + default-enemy)
    assert_eq!(trials.len(), 6);
    let peaceful_count = trials
      .iter()
      .filter(|t| t.variant == "peaceful-diagnostic")
      .count();
    let enemy_count = trials
      .iter()
      .filter(|t| t.variant == "default-enemy")
      .count();
    assert_eq!(peaceful_count, 3);
    assert_eq!(enemy_count, 3);
  }

  #[test]
  fn first_rocket_no_peaceful_produces_only_default_enemy() {
    let raw = include_str!("../../../../experiments/first-rocket.json");
    let manifest: FirstRocketManifest = serde_json::from_str(raw).unwrap();
    let trials = expand_first_rocket_trials(&manifest, false).unwrap();
    assert_eq!(trials.len(), 3);
    for trial in &trials {
      assert_eq!(trial.variant, "default-enemy");
    }
  }
}
