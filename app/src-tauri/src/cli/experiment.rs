//! `experiment`: manage and analyse experiments.
//!
//! Subcommands:
//! - `experiment report <manifest> <results-dir>`: produce a comparison report
//!   from existing trial results.

use crate::cli::{Subcommand, SubcommandCallback};
use crate::context::Context;
use crate::experiment::{manifest::Manifest, report::{compare, write_report}};
use clap::{Arg, ArgMatches, Command, value_parser};
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
            .about("Manage and analyse experiments")
            .subcommand(
                Command::new("run")
                    .about("Run an offline experiment comparing module vs legacy planning")
                    .arg(
                        Arg::new("manifest")
                            .long("manifest")
                            .short('m')
                            .value_name("path")
                            .required(true)
                            .value_parser(value_parser!(PathBuf))
                            .help("path to the experiment manifest JSON"),
                    )
                    .arg(
                        Arg::new("maps")
                            .long("maps")
                            .short('M')
                            .value_name("dir")
                            .required(true)
                            .value_parser(value_parser!(PathBuf))
                            .help("directory containing map dumps (map-{seed}.json)"),
                    )
                    .arg(
                        Arg::new("output")
                            .long("output")
                            .short('o')
                            .value_name("path")
                            .value_parser(value_parser!(PathBuf))
                            .help("output directory for results [default: ./results]"),
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
                            .help("path to the experiment manifest JSON"),
                    )
                    .arg(
                        Arg::new("results")
                            .long("results")
                            .short('r')
                            .value_name("path")
                            .required(true)
                            .value_parser(value_parser!(PathBuf))
                            .help("path to results directory (trials.csv + comparison.json)"),
                    )
                    .arg(
                        Arg::new("output")
                            .long("output")
                            .short('o')
                            .value_name("path")
                            .value_parser(value_parser!(PathBuf))
                            .help("output directory for the report [default: results/report]"),
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
        Some(("run", run_args)) => run_offline(run_args),
        Some(("report", report_args)) => run_report(report_args),
        Some((name, _)) => Err(miette!("unknown experiment subcommand: {name}")),
        None => Err(miette!("experiment requires a subcommand (run|report)")),
    }
}

fn run_offline(args: &ArgMatches) -> Result<()> {
    let manifest_path = args
        .get_one::<PathBuf>("manifest")
        .ok_or_else(|| miette!("--manifest is required"))?;
    let maps_dir = args
        .get_one::<PathBuf>("maps")
        .ok_or_else(|| miette!("--maps is required"))?;
    let output_dir = args
        .get_one::<PathBuf>("output")
        .cloned()
        .unwrap_or_else(|| PathBuf::from("./results"));

    std::fs::create_dir_all(&output_dir)
        .into_diagnostic()
        .map_err(|e| miette!("failed to create output dir: {e}"))?;

    let manifest_json = std::fs::read_to_string(manifest_path)
        .into_diagnostic()
        .map_err(|e| miette!("failed to read manifest: {e}"))?;
    let _manifest: crate::experiment::manifest::Manifest = factorio_bot_core::serde_json::from_str(&manifest_json)
        .into_diagnostic()
        .map_err(|e| miette!("failed to parse manifest: {e}"))?;

    let results = crate::experiment::offline::run_offline_experiment(
        &manifest,
        maps_dir,
        &output_dir,
    );

    println!("Experiment complete: {}/{} successful",
        results.iter().filter(|r| r.outcome == crate::experiment::runner::TrialOutcome::Success).count(),
        results.len());

    Ok(())
}

fn run_report(args: &ArgMatches) -> Result<()> {
    let manifest_path = args
        .get_one::<PathBuf>("manifest")
        .ok_or_else(|| miette!("--manifest is required"))?;
    let results_path = args
        .get_one::<PathBuf>("results")
        .ok_or_else(|| miette!("--results is required"))?;
    let output_dir = args
        .get_one::<PathBuf>("output")
        .cloned()
        .unwrap_or_else(|| results_path.join("report"));

    let manifest_json = std::fs::read_to_string(manifest_path)
        .into_diagnostic()
        .map_err(|e| miette!("failed to read manifest: {e}"))?;
    let manifest: Manifest = factorio_bot_core::serde_json::from_str(&manifest_json)
        .into_diagnostic()
        .map_err(|e| miette!("failed to parse manifest: {e}"))?;

    // Load trial results from results directory.
    let results_csv = results_path.join("trials.csv");
    let results_json = results_path.join("comparison.json");

    let mut results = Vec::new();

    // Try CSV first.
    if results_csv.exists() {
        let csv_content = std::fs::read_to_string(&results_csv)
            .into_diagnostic()
            .map_err(|e| miette!("failed to read {results_csv:?}: {e}"))?;
        results = parse_trials_csv(&csv_content)?;
    } else if results_json.exists() {
        let json_content = std::fs::read_to_string(&results_json)
            .into_diagnostic()
            .map_err(|e| miette!("failed to read {results_json:?}: {e}"))?;
        // For JSON, parse each line as a TrialResult
        for line in json_content.lines().filter(|l| !l.trim().is_empty() && !l.trim().starts_with('[') && !l.trim().starts_with(']') && !l.trim().starts_with('{')) {
        }
        return Err(miette!("JSON results not yet supported; use CSV format"));
    } else {
        return Err(miette!("no trial results found at {results_path:?}"));
    }

    let comparison = compare(&results)
        .map_err(|e| miette!("comparison failed: {e:?}"))?;

    let format = args.get_one::<String>("format").map(String::as_str).unwrap_or("md");
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
            // Write report to output directory.
            write_report(&results, &comparison, &output_dir)
                .map_err(|e| miette!("writing report failed: {e}"))?;
            println!("Report written to {:?}", output_dir.join("report.md"));
        }
    }

    Ok(())
}

/// Parse a simple CSV of trial results.
fn parse_trials_csv(content: &str) -> Result<Vec<crate::experiment::runner::TrialResult>> {
    #[allow(unused_variables)]
    use crate::experiment::manifest::TrialKey;
    use crate::experiment::runner::{TrialOutcome, TrialResult};

    let mut results = Vec::new();
    for (line_no, _line) in content.lines().enumerate() {
        if line_no == 0 || line.trim().is_empty() {
            continue; // header or empty
        }
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() < 7 {
            return Err(miette!("CSV line {} has {} fields (expected >= 7)", line_no + 1, parts.len()));
        }
        let (seed_str, bots_str, task, variant, rep_str, outcome_str, ticks_str) = (parts[0], parts[1], parts[2], parts[3], parts[4], parts[5], parts[6]);
        let _ = variant; let _ = outcome_str;
        let seed: u32 = seed_str.parse().map_err(|e| miette!("line {}: invalid seed: {e}", line_no + 1))?;
        let bots: u32 = bots_str.parse().map_err(|e| miette!("line {}: invalid bots: {e}", line_no + 1))?;
        let rep: u32 = rep_str.parse().map_err(|e| miette!("line {}: invalid rep: {e}", line_no + 1))?;

        let outcome = match outcome_str.trim() {
            "Success" => TrialOutcome::Success,
            "Failure" => TrialOutcome::Failure,
            "Timeout" => TrialOutcome::Timeout,
            _ => TrialOutcome::Invalid,
        };
        let ticks = ticks_str.trim().parse::<u64>().ok();

        results.push(TrialResult {
            key: TrialKey { seed, bots, task: task.to_string(), variant: variant.to_string(), repetition: rep },
            manifest_hash: String::new(),
            outcome,
            game_ticks: ticks,
            planning_ms: None,
            wall_ms: 0,
            reason: None,
        });
    }
    Ok(results)
}
