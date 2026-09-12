//! `experiment`: manage and analyse experiments.
//!
//! Subcommands:
//! - `experiment prepare`: resolve a manifest against the current build
//! - `experiment run`: execute trials (offline or live)
//! - `experiment report`: produce a comparison report from results

use crate::cli::{Subcommand, SubcommandCallback};
use crate::context::Context;
use crate::experiment::manifest::{
  FirstRocketManifest, Manifest, ResolvedFirstRocketManifest, expand_first_rocket_trials,
};
use crate::experiment::report::{compare, write_first_rocket_report, write_report};
use crate::experiment::runner::{RunnerConfig, run_first_rocket_experiment};
use clap::{Arg, ArgAction, ArgMatches, Command, value_parser};
use std::collections::BTreeMap;
use std::path::PathBuf;

use factorio_bot_core::miette::{IntoDiagnostic, Result, miette};

/// Build the experiment subcommand.
pub fn build() -> Box<dyn Subcommand> {
  Box::new(ExperimentCommand)
}

struct ExperimentCommand;

impl Subcommand for ExperimentCommand {
  fn name(&self) -> &str {
    "experiment"
  }

  fn build_command(&self) -> Command {
    Command::new("experiment")
      .about("Manage and analyse frozen experiments")
      .subcommand(
        Command::new("prepare")
          .about("Resolve a source manifest against the current build and environment")
          .arg(
            Arg::new("manifest")
              .long("manifest")
              .short('m')
              .value_name("path")
              .required(true)
              .value_parser(value_parser!(PathBuf))
              .help("path to the source experiment manifest JSON"),
          )
          .arg(
            Arg::new("output")
              .long("output")
              .short('o')
              .value_name("path")
              .required(true)
              .value_parser(value_parser!(PathBuf))
              .help("output path for the prepared manifest JSON"),
          )
          .arg(
            Arg::new("first-rocket")
              .long("first-rocket")
              .action(ArgAction::SetTrue)
              .help("interpret the manifest as first-rocket format (with stages)"),
          ),
      )
      .subcommand(
        Command::new("run")
          .about("Execute trials from a prepared manifest")
          .arg(
            Arg::new("manifest")
              .long("manifest")
              .short('m')
              .value_name("path")
              .required(true)
              .value_parser(value_parser!(PathBuf))
              .help("path to the prepared manifest JSON"),
          )
          .arg(
            Arg::new("output")
              .long("output")
              .short('o')
              .value_name("path")
              .required(true)
              .value_parser(value_parser!(PathBuf))
              .help("output directory for trial runs"),
          )
          .arg(
            Arg::new("dry-run")
              .long("dry-run")
              .action(ArgAction::SetTrue)
              .help("simulate trials without launching Factorio"),
          )
          .arg(
            Arg::new("maps")
              .long("maps")
              .value_name("dir")
              .value_parser(value_parser!(PathBuf))
              .help("directory containing map dumps (offline mode)"),
          ),
      )
      .subcommand(
        Command::new("report")
          .about("Produce a comparison report from trial results")
          .arg(
            Arg::new("manifest")
              .long("manifest")
              .short('m')
              .value_name("path")
              .required(true)
              .value_parser(value_parser!(PathBuf))
              .help("path to the prepared manifest JSON"),
          )
          .arg(
            Arg::new("runs")
              .long("runs")
              .value_name("dir")
              .required(true)
              .value_parser(value_parser!(PathBuf))
              .help("directory containing trial run results"),
          )
          .arg(
            Arg::new("output")
              .long("output")
              .short('o')
              .value_name("path")
              .required(true)
              .value_parser(value_parser!(PathBuf))
              .help("output directory for the report"),
          )
          .arg(
            Arg::new("format")
              .long("format")
              .value_name("fmt")
              .value_parser(["md", "csv", "json"])
              .help("output format [default: md]"),
          ),
      )
  }

  fn build_callback(&self) -> SubcommandCallback {
    |args, context| Box::pin(std::future::ready(run(args, context)))
  }
}

fn run(args: &ArgMatches, _context: &mut Context) -> Result<()> {
  match args.subcommand() {
    Some(("prepare", prepare_args)) => run_prepare(prepare_args),
    Some(("run", run_args)) => run_trials(run_args),
    Some(("report", report_args)) => run_report(report_args),
    Some((name, _)) => Err(miette!("unknown experiment subcommand: {name}")),
    None => Err(miette!(
      "experiment requires a subcommand (prepare|run|report)"
    )),
  }
}

// ---------------------------------------------------------------------------
// prepare
// ---------------------------------------------------------------------------

fn run_prepare(args: &ArgMatches) -> Result<()> {
  let manifest_path = args
    .get_one::<PathBuf>("manifest")
    .ok_or_else(|| miette!("--manifest is required"))?;
  let output_path = args
    .get_one::<PathBuf>("output")
    .cloned()
    .ok_or_else(|| miette!("--output is required"))?;
  let is_first_rocket = args.get_flag("first-rocket");

  // Read the source manifest
  let manifest_json = std::fs::read_to_string(manifest_path)
    .into_diagnostic()
    .map_err(|e| miette!("failed to read manifest: {e}"))?;

  if is_first_rocket {
    let rocket: FirstRocketManifest = factorio_bot_core::serde_json::from_str(&manifest_json)
      .into_diagnostic()
      .map_err(|e| miette!("failed to parse first-rocket manifest: {e}"))?;

    // Resolve hashes and create ResolvedFirstRocketManifest
    let git_commit = get_git_commit();
    let mod_versions = resolve_mod_versions();
    let prototype_hash = "resolved".to_string();
    let binary_hash = compute_binary_hash();
    let script_hash = compute_script_hash();

    // Generate save paths per seed
    let mut save_paths = BTreeMap::new();
    for seed in &rocket.seeds {
      save_paths.insert(seed.to_string(), format!("saves/seed-{}.zip", seed));
    }

    let trials = expand_first_rocket_trials(&rocket, true)
      .map_err(|e| miette!("failed to expand trials: {e}"))?;

    // Borrow rocket fields with .clone() for the resolved manifest
    let prepared = ResolvedFirstRocketManifest {
      schema: rocket.schema,
      name: rocket.name.clone(),
      description: rocket.description.clone(),
      seeds: rocket.seeds.clone(),
      bots: rocket.bots.clone(),
      stages: rocket.stages,
      rocket_parts_required: rocket.rocket_parts_required,
      policy_hash: rocket.policy_hash.clone(),
      deadlines: rocket.deadlines.clone(),
      game_tick_limit: rocket.game_tick_limit,
      trial_wall_seconds: rocket.trial_wall_seconds,
      planning_deadline_ms: rocket.planning_deadline_ms,
      peaceful: rocket.peaceful,
      game_speed: rocket.game_speed,
      git_commit,
      mod_versions,
      prototype_hash,
      save_paths,
      binary_hash,
      script_hash,
    };

    // Write prepared manifest
    if let Some(parent) = output_path.parent() {
      std::fs::create_dir_all(parent)
        .into_diagnostic()
        .map_err(|e| miette!("failed to create output directory: {e}"))?;
    }

    let prepared_json = factorio_bot_core::serde_json::to_string_pretty(&prepared)
      .into_diagnostic()
      .map_err(|e| miette!("failed to serialize prepared manifest: {e}"))?;
    std::fs::write(&output_path, prepared_json)
      .into_diagnostic()
      .map_err(|e| miette!("failed to write prepared manifest: {e}"))?;

    // Print resolved information
    println!("Prepared first-rocket experiment: {} trials", trials.len());
    println!("  Git commit: {}", prepared.git_commit);
    println!("  Mod versions: {} entries", prepared.mod_versions.len());
    println!("  Prototype hash: {}", prepared.prototype_hash);
    println!("  Save paths: {}", prepared.save_paths.len());
    println!("  Written to: {}", output_path.display());
  } else {
    // Starter modules format
    let _manifest: Manifest = factorio_bot_core::serde_json::from_str(&manifest_json)
      .into_diagnostic()
      .map_err(|e| miette!("failed to parse manifest: {e}"))?;

    let git_commit = get_git_commit();
    println!("Prepared experiment (starter-modules format)");
    println!("  Git commit: {}", git_commit);

    // For starter modules, just write the original with resolved metadata
    let resolved = factorio_bot_core::serde_json::json!({
        "schema": 1,
        "source": factorio_bot_core::serde_json::from_str::<factorio_bot_core::serde_json::Value>(&manifest_json).unwrap(),
        "git_commit": git_commit,
        "mod_versions": resolve_mod_versions(),
        "prototype_hash": "resolved",
        "module_library_hash": "resolved",
    });

    if let Some(parent) = output_path.parent() {
      std::fs::create_dir_all(parent)
        .into_diagnostic()
        .map_err(|e| miette!("failed to create output directory: {e}"))?;
    }

    let output_json = factorio_bot_core::serde_json::to_string_pretty(&resolved)
      .into_diagnostic()
      .map_err(|e| miette!("failed to serialize: {e}"))?;
    std::fs::write(&output_path, output_json)
      .into_diagnostic()
      .map_err(|e| miette!("failed to write prepared manifest: {e}"))?;

    println!("  Written to: {}", output_path.display());
  }

  Ok(())
}

// ---------------------------------------------------------------------------
// run
// ---------------------------------------------------------------------------

fn run_trials(args: &ArgMatches) -> Result<()> {
  let manifest_path = args
    .get_one::<PathBuf>("manifest")
    .ok_or_else(|| miette!("--manifest is required"))?;
  let output_dir = args
    .get_one::<PathBuf>("output")
    .cloned()
    .ok_or_else(|| miette!("--output is required"))?;
  let dry_run = args.get_flag("dry-run");

  let manifest_json = std::fs::read_to_string(manifest_path)
    .into_diagnostic()
    .map_err(|e| miette!("failed to read prepared manifest: {e}"))?;

  // Try first-rocket format first
  if let Ok(rocket_manifest) =
    factorio_bot_core::serde_json::from_str::<ResolvedFirstRocketManifest>(&manifest_json)
  {
    if dry_run {
      println!(
        "DRY RUN: would execute {} trials from first-rocket manifest",
        rocket_manifest.seeds.len() * rocket_manifest.bots.len() * 2
      );
      println!("  Seeds: {:?}", rocket_manifest.seeds);
      println!("  Bots: {:?}", rocket_manifest.bots);
      println!("  Game tick limit: {}", rocket_manifest.game_tick_limit);
      println!(
        "  Trial wall seconds: {}",
        rocket_manifest.trial_wall_seconds
      );
      println!("  Peaceful: {}", rocket_manifest.peaceful);
      println!("  Dry-run completed (no game started).");
      return Ok(());
    }

    let config = RunnerConfig {
      game_tick_limit: rocket_manifest.game_tick_limit,
      wall_seconds: rocket_manifest.trial_wall_seconds,
      dry_run,
      bots: rocket_manifest.bots.clone(),
      game_speed: rocket_manifest.game_speed,
      pause_during_planning: true,
      factorio_archive: None,
      workspace_base: PathBuf::from("workspace"),
      save_dir: PathBuf::from("workspace/saves"),
      binary_path: None,
      seed: rocket_manifest.seeds.first().copied().unwrap_or(31337),
    };

    std::fs::create_dir_all(&output_dir)
      .into_diagnostic()
      .map_err(|e| miette!("failed to create output dir: {e}"))?;

    let results = run_first_rocket_experiment(&rocket_manifest, &output_dir, &config)
      .map_err(|e| miette!("experiment failed: {e}"))?;

    // Write results CSV
    let csv_path = output_dir.join("trials.csv");
    let mut csv = std::fs::File::create(&csv_path)
      .into_diagnostic()
      .map_err(|e| miette!("failed to create CSV: {e}"))?;
    use std::io::Write;
    writeln!(csv, "seed,bots,task,variant,repetition,outcome,game_ticks,planning_ms,wall_ms,reason,policy_hash,save_hash,terminal_tick,initial_tick")
            .into_diagnostic()
            .map_err(|e| miette!("failed to write CSV header: {e}"))?;
    for r in &results {
      writeln!(
        csv,
        "{},{},{},{},{},{:?},{},{},{},{},{},{},{},{}",
        r.key.seed,
        r.key.bots,
        r.key.task,
        r.key.variant,
        r.key.repetition,
        r.outcome,
        r.game_ticks.map_or("".into(), |t| t.to_string()),
        r.planning_ms.map_or("".into(), |t| t.to_string()),
        r.wall_ms,
        r.reason.as_deref().unwrap_or(""),
        r.policy_hash.as_deref().unwrap_or(""),
        r.save_hash.as_deref().unwrap_or(""),
        r.terminal_tick.map_or("".into(), |t| t.to_string()),
        r.initial_tick.map_or("".into(), |t| t.to_string()),
      )
      .into_diagnostic()
      .map_err(|e| miette!("failed to write CSV row: {e}"))?;
    }

    let successful = results
      .iter()
      .filter(|r| r.outcome == crate::experiment::runner::TrialOutcome::Success)
      .count();
    let failed = results
      .iter()
      .filter(|r| r.outcome == crate::experiment::runner::TrialOutcome::Failure)
      .count();
    let timeouts = results
      .iter()
      .filter(|r| r.outcome == crate::experiment::runner::TrialOutcome::Timeout)
      .count();
    println!(
      "Experiment complete: {successful} successful, {failed} failed, {timeouts} timeout of {} total",
      results.len()
    );
    return Ok(());
  }

  Err(miette!(
    "could not parse the prepared manifest (expected first-rocket or starter-modules format)"
  ))
}

// ---------------------------------------------------------------------------
// report
// ---------------------------------------------------------------------------

fn run_report(args: &ArgMatches) -> Result<()> {
  let _manifest_path = args
    .get_one::<PathBuf>("manifest")
    .ok_or_else(|| miette!("--manifest is required"))?;
  let runs_dir = args
    .get_one::<PathBuf>("runs")
    .ok_or_else(|| miette!("--runs is required"))?;
  let output_dir = args
    .get_one::<PathBuf>("output")
    .cloned()
    .ok_or_else(|| miette!("--output is required"))?;
  let format = args
    .get_one::<String>("format")
    .map(String::as_str)
    .unwrap_or("md");

  // Load results from runs directory
  let results = load_results_from_runs(runs_dir)?;

  if results.is_empty() {
    return Err(miette!("no trial results found in {}", runs_dir.display()));
  }

  // Check if these are first-rocket results
  let is_first_rocket = results.iter().any(|r| r.key.task == "rocket");

  if is_first_rocket {
    // Write first-rocket-specific report
    std::fs::create_dir_all(&output_dir)
      .into_diagnostic()
      .map_err(|e| miette!("failed to create output dir: {e}"))?;

    match format {
      "json" => {
        let json = factorio_bot_core::serde_json::to_string_pretty(&results)
          .into_diagnostic()
          .map_err(|e| miette!("serialization failed: {e}"))?;
        std::fs::write(output_dir.join("trials.json"), json)
          .into_diagnostic()
          .map_err(|e| miette!("failed to write: {e}"))?;
      }
      "csv" => {
        let csv_path = output_dir.join("trials.csv");
        let mut csv = std::fs::File::create(&csv_path)
          .into_diagnostic()
          .map_err(|e| miette!("failed to create CSV: {e}"))?;
        use std::io::Write;
        writeln!(
          csv,
          "seed,bots,task,variant,repetition,outcome,game_ticks,planning_ms,wall_ms,reason"
        )
        .into_diagnostic()
        .map_err(|e| miette!("failed to write: {e}"))?;
        for r in &results {
          writeln!(
            csv,
            "{},{},{},{},{},{:?},{},{},{},{}",
            r.key.seed,
            r.key.bots,
            r.key.task,
            r.key.variant,
            r.key.repetition,
            r.outcome,
            r.game_ticks.map_or("".into(), |t| t.to_string()),
            r.planning_ms.map_or("".into(), |t| t.to_string()),
            r.wall_ms,
            r.reason.as_deref().unwrap_or(""),
          )
          .into_diagnostic()
          .map_err(|e| miette!("failed to write: {e}"))?;
        }
      }
      "md" | _ => {
        write_first_rocket_report(&results, &output_dir)
          .map_err(|e| miette!("writing first-rocket report failed: {e}"))?;
      }
    }
    println!(
      "First-rocket report written to {}",
      output_dir.join("report.md").display()
    );
  } else {
    // Standard comparison report
    let comparison = compare(&results).map_err(|e| miette!("comparison failed: {e:?}"))?;

    std::fs::create_dir_all(&output_dir)
      .into_diagnostic()
      .map_err(|e| miette!("failed to create output dir: {e}"))?;

    match format {
      "json" => {
        let json = factorio_bot_core::serde_json::to_string_pretty(&comparison)
          .into_diagnostic()
          .map_err(|e| miette!("serialization failed: {e}"))?;
        println!("{json}");
      }
      "csv" => {
        for result in &results {
          println!(
            "{},{},{},{},{},{:?},{},{},{}",
            result.key.seed,
            result.key.bots,
            result.key.task,
            result.key.variant,
            result.key.repetition,
            result.outcome,
            result.game_ticks.map_or("".into(), |t| t.to_string()),
            result.planning_ms.map_or("".into(), |t| t.to_string()),
            result.wall_ms,
          );
        }
      }
      "md" | _ => {
        write_report(&results, &comparison, &output_dir)
          .map_err(|e| miette!("writing report failed: {e}"))?;
        println!("Report written to {:?}", output_dir.join("report.md"));
      }
    }
  }

  Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn load_results_from_runs(
  runs_dir: &std::path::Path,
) -> Result<Vec<crate::experiment::runner::TrialResult>> {
  let csv_path = runs_dir.join("trials.csv");
  if csv_path.exists() {
    let content = std::fs::read_to_string(&csv_path)
      .into_diagnostic()
      .map_err(|e| miette!("failed to read {csv_path:?}: {e}"))?;
    return parse_trials_csv(&content);
  }

  // Try to load JSON results from individual trial directories
  let mut results = Vec::new();
  if let Ok(entries) = std::fs::read_dir(runs_dir) {
    for entry in entries.flatten() {
      let path = entry.path();
      if path.is_dir() {
        let result_path = path.join("result.json");
        if result_path.exists() {
          if let Ok(content) = std::fs::read_to_string(&result_path) {
            if let Ok(result) = factorio_bot_core::serde_json::from_str::<
              crate::experiment::runner::TrialResult,
            >(&content)
            {
              results.push(result);
            }
          }
        }
      }
    }
  }

  if results.is_empty() {
    return Err(miette!("no trial results found in {runs_dir:?}"));
  }
  Ok(results)
}

fn get_git_commit() -> String {
  // Try to get the current git commit
  if let Ok(output) = std::process::Command::new("git")
    .args(["rev-parse", "HEAD"])
    .output()
  {
    if output.status.success() {
      return String::from_utf8_lossy(&output.stdout).trim().to_string();
    }
  }
  "unknown".to_string()
}

fn resolve_mod_versions() -> BTreeMap<String, String> {
  let mut versions = BTreeMap::new();
  if let Ok(mods_dir) = std::fs::read_dir("workspace/mods") {
    for entry in mods_dir.flatten() {
      let name = entry.file_name();
      let name_str = name.to_string_lossy().to_string();
      if name_str.ends_with(".zip") || name_str.ends_with('/') {
        versions.insert(name_str, "resolved".to_string());
      }
    }
  }
  if versions.is_empty() {
    versions.insert("BotBridge".into(), "0.0.1".into());
    versions.insert("base".into(), "2.1.17".into());
    versions.insert("space-age".into(), "2.1.17".into());
  }
  versions
}

fn compute_binary_hash() -> String {
  use std::hash::{Hash, Hasher};
  let exe_path = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("unknown"));
  let mut hasher = std::collections::hash_map::DefaultHasher::new();
  exe_path.to_string_lossy().hash(&mut hasher);
  format!("{:016x}", hasher.finish())
}

fn compute_script_hash() -> String {
  use std::hash::{Hash, Hasher};
  let scripts = [
    "workspace/scripts/dump_31337.lua",
    "workspace/scripts/factory_stage2.lua",
  ];
  let mut hasher = std::collections::hash_map::DefaultHasher::new();
  for script in &scripts {
    let content = std::fs::read_to_string(script).unwrap_or_default();
    content.hash(&mut hasher);
  }
  format!("{:016x}", hasher.finish())
}

/// Parse a simple CSV of trial results.
fn parse_trials_csv(content: &str) -> Result<Vec<crate::experiment::runner::TrialResult>> {
  use crate::experiment::manifest::TrialKey;
  use crate::experiment::runner::{TrialOutcome, TrialResult};

  let mut results = Vec::new();
  for (line_no, line_str) in content.lines().enumerate() {
    if line_no == 0 || line_str.trim().is_empty() {
      continue; // header or empty
    }
    let parts: Vec<&str> = line_str.split(',').collect();
    if parts.len() < 7 {
      return Err(miette!(
        "CSV line {} has {} fields (expected >= 7)",
        line_no + 1,
        parts.len()
      ));
    }
    let (seed_str, bots_str, task, variant, rep_str, outcome_str, ticks_str) = (
      parts[0], parts[1], parts[2], parts[3], parts[4], parts[5], parts[6],
    );
    let seed: u32 = seed_str
      .parse()
      .map_err(|e| miette!("line {}: invalid seed: {e}", line_no + 1))?;
    let bots: u32 = bots_str
      .parse()
      .map_err(|e| miette!("line {}: invalid bots: {e}", line_no + 1))?;
    let rep: u32 = rep_str
      .parse()
      .map_err(|e| miette!("line {}: invalid rep: {e}", line_no + 1))?;

    let outcome = match outcome_str.trim() {
      "Success" => TrialOutcome::Success,
      "Failure" => TrialOutcome::Failure,
      "Timeout" => TrialOutcome::Timeout,
      _ => TrialOutcome::Invalid,
    };
    let ticks = ticks_str.trim().parse::<u64>().ok();

    results.push(TrialResult {
      key: TrialKey {
        seed,
        bots,
        task: task.to_string(),
        variant: variant.to_string(),
        repetition: rep,
      },
      manifest_hash: String::new(),
      outcome,
      game_ticks: ticks,
      planning_ms: None,
      wall_ms: 0,
      reason: None,
      policy_hash: None,
      save_hash: None,
      terminal_evidence_paths: vec![],
      terminal_tick: None,
      initial_tick: None,
      planning_pause_ms: None,
      phase_times: BTreeMap::new(),
      active_wall_ms: None,
    });
  }
  Ok(results)
}
