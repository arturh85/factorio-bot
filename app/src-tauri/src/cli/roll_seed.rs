//! `roll-seed`, which is declared but does not work, and now says so.
//!
//! # What it was meant to do
//!
//! Generate maps from one map-exchange string with random seeds, score each
//! one, and report the best. The scoring lived in
//! `factorio_bot_scripting_lua::roll_best_seed::score_seed`: it ran a plan
//! script against a freshly generated map and scored the seed as
//! `-planner.graph().shortest_path()` -- a shorter critical path through the
//! old task graph meant a cheaper start -- minus 10000 for every resource type
//! not found within 3000 tiles of spawn.
//!
//! # Why it does not
//!
//! `score_seed` was deleted with the old task-graph planner. It had no live
//! caller even then: the worker loop that called it had been commented out for
//! a long time, so `roll_seed` pushed nothing onto its join-handle vector,
//! joined an empty vector, and returned `Ok(None)` -- "no seed found" -- for
//! every possible input. Before it could even get that far it canonicalized
//! `plans/{name}.lua`, a path relative to the process CWD, and there is no
//! `plans/` directory in the repo (`workspace/plans` was a duplicate
//! extraction, removed as dead in `15d49278`). So the command's real behaviour
//! was: prepare `--parallel` Factorio instances (default four, minutes of
//! archive extraction each), then fail on a `canonicalize` of a directory that
//! does not exist -- and had that path been fixed, spend the same minutes to
//! print "no seed found".
//!
//! # Why this refuses instead of resolving the path
//!
//! Because the path was the *second* thing wrong with it. Pointing it at the
//! workspace scripts directory would have made a dead loop read a file it
//! never uses, and turned a loud failure into a silent one. Resurrecting seed
//! rolling means choosing a fitness function for the current planner --
//! `Schedule::makespan` is the plausible candidate -- and building the
//! cross-boundary plumbing to read a `Schedule` back out of the handle-based
//! Lua runtime. That is real design work, not a path fix, so the subcommand is
//! gated here rather than left looking usable.
//!
//! The clap surface below is deliberately kept: it is the interface any
//! resurrection would implement, and `--help` is where a reader looks first.
//!
//! # Half of it now exists as `score-map`
//!
//! Workstream 0b built the scoring half: `factorio-bot score-map`
//! (`crate::cli::score_map`) takes a dumped world and reports resource
//! distances, water, a verdict and the real `expand()` + `schedule()`
//! makespan. The plumbing this comment called for turned out not to be
//! needed at all — a world dump is read straight into `PlanState`, so there
//! is no Lua runtime to read a `Schedule` back out of.
//!
//! What is still missing is exactly what this command was: the loop that
//! *generates* the maps, which needs a live Factorio per seed. That is
//! written down as a recipe in `tools/seed_search.sh` and in `score-map
//! --help` rather than automated here, because `--seed` only takes effect
//! together with `--new` and `--new` deletes the workspace's map. Anyone
//! resurrecting this command should make it drive `score-map`, not
//! reimplement it.

use crate::cli::{SETTINGS_PRECEDENCE_HELP, Subcommand, SubcommandCallback};
use clap::{Arg, Command, value_parser};

use factorio_bot_core::miette::{Result, miette};

impl Subcommand for ThisCommand {
  fn name(&self) -> &'static str {
    "roll-seed"
  }
  fn build_command(&self) -> Command {
    Command::new("roll-seed")
      .arg(
        Arg::new("map")
          .long("map")
          .value_name("map")
          .required(true)
          .value_parser(value_parser!(String))
          .help("use given map exchange string"),
      )
      .arg(
        Arg::new("seconds")
          .short('s')
          .long("seconds")
          .value_name("seconds")
          .default_value("360")
          .value_parser(value_parser!(u64))
          .help("limits how long to roll seeds"),
      )
      .arg(
        Arg::new("parallel")
          .short('p')
          .long("parallel")
          .value_name("parallel")
          .default_value("4")
          .value_parser(value_parser!(u8))
          .help("how many rolling servers to run in parallel"),
      )
      .arg(
        Arg::new("name")
          .long("name")
          .value_name("name")
          .required(true)
          .value_parser(value_parser!(String))
          .help("name of plan without .lua extension"),
      )
      .arg(
        Arg::new("rolls")
          .short('r')
          .long("rolls")
          .value_name("rolls")
          .value_parser(value_parser!(u64))
          .help("how many seeds to roll"),
      )
      .arg(
        Arg::new("clients")
          .short('c')
          .long("clients")
          .default_value("1")
          .value_parser(value_parser!(u8))
          .help("number of bots to plan for (no client process is started)"),
      )
      .about("UNIMPLEMENTED: roll good seed for given map-exchange-string based on heuristics")
      .after_help(SETTINGS_PRECEDENCE_HELP)
  }

  fn build_callback(&self) -> SubcommandCallback {
    // `std::future::ready` rather than an `async fn`: there is nothing to
    // await, and an async wrapper around an immediate error would only
    // suggest work is being done.
    |_args, _context| Box::pin(std::future::ready(refuse()))
  }
}

/// Refuses before doing anything at all.
///
/// The refusal is first, ahead of settings loading and ahead of any instance
/// preparation, because the previous order was the whole user-visible defect:
/// several minutes of archive extraction bought a failure that had been
/// certain since the process started. Failing in milliseconds, with the reason
/// in the message, is the only honest thing this can do until the scoring
/// function exists again.
fn refuse() -> Result<()> {
  Err(miette!(
    "roll-seed is not implemented: its scoring function was deleted with the \
     old task-graph planner and the loop that called it had already been \
     disabled, so this command could never report a seed. See the module \
     comment in app/src-tauri/src/cli/roll_seed.rs for what resurrecting it \
     needs."
  ))
}

struct ThisCommand {}
pub fn build() -> Box<dyn Subcommand> {
  Box::new(ThisCommand {})
}
