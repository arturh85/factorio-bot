//! `score-map`: how good a starting map is, judged from a dumped world.
//!
//! # Why this exists
//!
//! **Walking was 20.3% of the reference run** — 14,330 ticks, 3.98 minutes,
//! for bot 1 alone. The target for `researched:automation` is under nine
//! minutes of game time against a single-player world record of 6:12, so four
//! minutes of walking is two thirds of the budget, and no planner change
//! compensates for a spawn whose ore is far away. Workstream 0b of
//! `docs/superpowers/plans/2026-09-03-closing-the-idle-gap.md` is the answer:
//! pick a *reasonable* map first, so that a comparison against a world-record
//! time is a comparison of play rather than of luck.
//!
//! # It is the resurrection of `roll-seed`, minus what killed it
//!
//! `roll-seed` (`crate::cli::roll_seed`) refuses to run, and its own module
//! comment names what a resurrection needs: "a fitness function for the
//! current planner — `Schedule::makespan` is the plausible candidate — and the
//! cross-boundary plumbing to read a `Schedule` back out of the handle-based
//! Lua runtime". Workstream 0's world dump removed the plumbing problem
//! entirely: this reads a file, calls `expand()` and `schedule()` directly and
//! prints the makespan. There is no Lua runtime, no RCON, no workspace and no
//! Factorio process anywhere in it.
//!
//! What is *not* here is the other half of `roll-seed`: generating the maps.
//! That needs a live Factorio, and it is deliberately left as a recipe (see
//! [`SCORE_AFTER_HELP`]) rather than automated, because `--seed` only takes
//! effect together with `--new`, and `--new` **deletes the workspace's map**.
//!
//! # Two tiers, and both are printed
//!
//! * **Distance** — `MapScore`, in `crates/planner/src/score.rs`. Nearest
//!   charted tile of each thing rung 1 needs, priced as walking ticks. Cheap,
//!   and it is what explains a refusal.
//! * **Makespan** — the real `expand()` + `schedule()` for the goal, reported
//!   as `PlanReport`. Strictly the better number, because it prices the plan
//!   rather than a proxy for it. **A refusal is a result, not a crash**: a map
//!   the planner will not plan is the strongest possible verdict on it, so the
//!   error is reported beside the distances and the command still exits 0.
//!
//! # Charting bounds all of it
//!
//! Everything read here comes out of `EntityGraph`, which holds **charted**
//! chunks. `MapScore::charting` probes the search disc and says how much of it
//! the dump has terrain for; read that line before believing any other. See
//! `factorio_bot_planner::score`'s module documentation for what a t=0 dump
//! actually knows (measured: 418 chunks, tiles spanning `[-320, 320)`).

use crate::cli::plan::{load_world, parse_goal, parse_roster, roster_from};
use crate::cli::{Subcommand, SubcommandCallback};
use crate::context::Context;
use clap::{Arg, ArgAction, ArgMatches, Command, value_parser};
use factorio_bot_core::miette::{IntoDiagnostic, Result, miette};
use factorio_bot_core::serde_json;
use factorio_bot_core::types::Position;
use factorio_bot_planner::method::have::registry_for;
use factorio_bot_planner::score::{DEFAULT_SEARCH_RADIUS, MapScore};
use factorio_bot_planner::{PlanReport, PlanState, pick_chain_actor, plan_best};
use serde::Serialize;
use std::path::PathBuf;

/// The goal a starting map is judged against, when the caller names none.
///
/// The milestone the whole plan is about. Nothing else is a fair test of a
/// *starting* map: `have:iron-plate:50` never asks for water, and a map with
/// no shoreline would score perfectly on it and then refuse the actual run.
const DEFAULT_GOAL: &str = "researched:automation";

pub(crate) const SCORE_AFTER_HELP: &str = "\
Two scores are printed for one dumped world:

  distance   nearest charted tile of iron-ore, copper-ore, coal, stone and
             water, priced as ticks of walking. Water is the one that
             disqualifies: past 128 tiles the planner raises
             PowerPlantNeedsWater and the goal does not expand at all.
  makespan   the real expand() + schedule() for --goal. Better, because it
             prices the plan and not a proxy. A refusal is reported and is
             itself a verdict on the map -- the command still exits 0.

READ THE `charting` LINE FIRST. EntityGraph holds charted chunks, so a
resource reported as missing may only be unexplored. A t=0 dump was measured
at 418 chunks, tiles spanning [-320, 320) on both axes, which contains the
whole default 256-tile search disc -- but that is one measured save, which is
why every dump is probed and the answer printed.

SEARCHING FOR A SEED (this command does not do it -- it scores what you give
it, and generating a map needs a live Factorio):

  1. Dump a map at t=0. `scripts/dump_map.lua` does exactly this and nothing
     else; it needs no graphical client:

       factorio-bot lua dump_map.lua --clients 0 --bots 4 \\
         --seed <N> --new --workspace-path /tmp/seedsearch

     --seed ONLY takes effect together with --new, because it reaches
     Factorio as --map-gen-seed on a --create and --create only runs when
     level.zip is absent. --new DELETES that map. Point --workspace-path at a
     scratch directory, never at the primary workspace.

  2. Score the dump, with no game running:

       factorio-bot score-map --world /tmp/seedsearch/map-<N>.json \\
         --bots 1,2,3,4 --json > /tmp/seedsearch/score-<N>.json

     --bots explicitly: a --clients 0 run may leave the dump with no players,
     and a missing roster skips the makespan tier instead of failing.

  3. Repeat, then rank by `.score.walk_score` (lower is better) among the
     entries whose `.score.verdict` is \"Viable\", and break ties on
     `.plan.makespan`.

`tools/seed_search.sh` is that loop, written down. It launches Factorio, so do
not run it while another agent holds the workspace or the build.

No Factorio, no RCON, no workspace and no settings file are involved in THIS
command.";

impl Subcommand for ThisCommand {
  fn name(&self) -> &'static str {
    "score-map"
  }
  fn build_command(&self) -> Command {
    Command::new("score-map")
      .about("Score a dumped world as a starting map: resource distances, water, and a makespan")
      .after_help(SCORE_AFTER_HELP)
      .arg(
        Arg::new("world")
          .long("world")
          .short('w')
          .value_name("path")
          .required(true)
          .value_parser(value_parser!(PathBuf))
          .help("world dump to score, as written by world.dump()"),
      )
      .arg(
        Arg::new("from")
          .long("from")
          .value_name("x,y")
          .value_parser(value_parser!(String))
          .help("measure distances from here [default: 0,0, Factorio's map origin]"),
      )
      .arg(
        Arg::new("radius")
          .long("radius")
          .short('r')
          .value_name("tiles")
          .value_parser(value_parser!(f64))
          .help("how far out to look for a resource [default: 256]"),
      )
      .arg(
        Arg::new("goal")
          .long("goal")
          .short('g')
          .value_name("spec")
          .value_parser(value_parser!(String))
          .help("goal for the makespan tier [default: researched:automation]"),
      )
      .arg(
        Arg::new("bots")
          .long("bots")
          .short('b')
          .value_name("ids")
          .value_parser(value_parser!(String))
          .help("roster for the makespan tier, e.g. 1,2,3,4 [default: the players in the dump]"),
      )
      .arg(
        Arg::new("no-plan")
          .long("no-plan")
          .action(ArgAction::SetTrue)
          .help("distance scoring only; skip expand() and schedule()"),
      )
      .arg(
        Arg::new("json")
          .long("json")
          .action(ArgAction::SetTrue)
          .help("print the whole verdict as JSON, for ranking a batch of seeds"),
      )
  }

  fn build_callback(&self) -> SubcommandCallback {
    |args, context| Box::pin(std::future::ready(run(args, context)))
  }
}

/// Everything this command concluded about one map, as one JSON document.
///
/// One object rather than two, because ranking a batch of seeds means sorting
/// files by a field in them, and a shape that puts the distance score and the
/// makespan in separate documents makes a caller join them by filename.
#[derive(Debug, Serialize)]
struct MapVerdict {
  world: String,
  score: MapScore,
  /// The makespan tier, when it was asked for and the plan expanded.
  plan: Option<PlanReport>,
  /// Why there is no plan. **A refusal is a verdict on the map**, not a
  /// failure of this command: `PowerPlantNeedsWater` means the lab never gets
  /// power on this map, which is the strongest thing that can be said against
  /// a starting seed. It is reported and never raised.
  plan_error: Option<String>,
  /// Reasons a number here may be misleading, in the order a reader needs
  /// them. Carried in the JSON as well as printed, so a batch ranked by a
  /// script keeps the caveat attached to the number it qualifies.
  notes: Vec<String>,
}

/// `--from x,y`.
///
/// # Why an origin has to be given at all
///
/// **A `FactorioSurface` carries no spawn point.** There is no field for it and
/// no way to derive one, so the default is Factorio's map origin `(0, 0)`,
/// which is where a fresh character appears. A mid-run dump has every player
/// somewhere else entirely, which is why the origin is echoed in the report
/// rather than left implicit.
fn parse_origin(raw: &str) -> Result<Position> {
  let (x, y) = raw
    .split_once(',')
    .ok_or_else(|| miette!("`{raw}` is not a position; expected `x,y`, e.g. `0,0`"))?;
  let parse = |part: &str, axis: &str| -> Result<f64> {
    part
      .trim()
      .parse::<f64>()
      .map_err(|_| miette!("`{}` in `{raw}` is not a {axis} coordinate", part.trim()))
  };
  Ok(Position::new(parse(x, "x")?, parse(y, "y")?))
}

/// Scores the dump and returns the verdict.
///
/// Split out of [`run`] for the same reason `plan`'s is: everything here is a
/// pure function of a file and some arguments, and a test that had to build a
/// `Context` would be testing the CLI harness instead.
fn score_dump(
  world_path: &std::path::Path,
  origin: Option<&str>,
  radius: Option<f64>,
  goal_spec: Option<&str>,
  roster: Option<&str>,
  skip_plan: bool,
) -> Result<MapVerdict> {
  let world = load_world(world_path)?;
  let origin = match origin {
    Some(raw) => parse_origin(raw)?,
    None => Position::new(0., 0.),
  };
  let radius = radius.unwrap_or(DEFAULT_SEARCH_RADIUS);
  if radius <= 0. || !radius.is_finite() {
    return Err(miette!("--radius must be a positive number, not {radius}"));
  }

  // The roster is only the makespan tier's business: distances from spawn do
  // not depend on how many bots walk them. So a dump with no players still
  // scores, and only the plan is skipped.
  let bots = if let Some(raw) = roster {
    Some(parse_roster(raw)?)
  } else {
    let found = roster_from(&world);
    (!found.is_empty()).then_some(found)
  };

  let state = PlanState::from_world(world.clone(), bots.as_deref().unwrap_or_default());
  let score = MapScore::of(&state, &origin, radius);

  let mut notes = Vec::new();
  if !score.charting.is_complete() {
    notes.push(format!(
      "{} of {} charting probes found no terrain: this dump has not charted \
       the whole search disc, so a resource reported as missing may only be \
       unexplored",
      score.charting.blind.len(),
      score.charting.probes
    ));
  }
  let far_players: Vec<String> = world
    .players
    .iter()
    .filter(|entry| {
      factorio_bot_core::factorio::util::calculate_distance(&entry.value().position, &origin) > 32.
    })
    .map(|entry| entry.key().to_string())
    .collect();
  if !far_players.is_empty() {
    notes.push(format!(
      "player(s) {} are more than 32 tiles from the origin, so this is \
       probably a mid-run dump rather than a fresh map; the origin scored \
       from is ({:.1}, {:.1})",
      far_players.join(", "),
      origin.x(),
      origin.y()
    ));
  }

  let mut plan = None;
  let mut plan_error = None;
  if !skip_plan {
    if let Some(bots) = &bots {
      let goal = parse_goal(goal_spec.unwrap_or(DEFAULT_GOAL))?;
      match plan_for(&state, &goal, bots) {
        Ok(report) => plan = Some(report),
        Err(err) => plan_error = Some(err),
      }
    } else {
      notes.push(
        "no makespan: the dump has no players and --bots was not given, so \
         there is no roster to plan for"
          .to_owned(),
      );
    }
  }

  Ok(MapVerdict {
    world: world_path.display().to_string(),
    score,
    plan,
    plan_error,
    notes,
  })
}

/// The makespan tier. `Err` carries a message, never a diagnostic to raise:
/// see [`MapVerdict::plan_error`].
fn plan_for(
  state: &PlanState,
  goal: &factorio_bot_planner::Goal,
  bots: &[factorio_bot_planner::BotId],
) -> std::result::Result<PlanReport, String> {
  let actor = pick_chain_actor(state, bots).ok_or_else(|| "no bots to plan for".to_owned())?;
  let (net, scheduled) = plan_best(
    std::slice::from_ref(goal),
    state,
    &registry_for(bots),
    actor,
    bots,
  )
  .map_err(|err| format!("the goal did not expand: {err}"))?;
  Ok(PlanReport::of(&net, &scheduled, bots, state))
}

/// The verdict as lines a person reads.
fn render(verdict: &MapVerdict) -> Vec<String> {
  let mut out = vec![format!("world          {}", verdict.world)];
  out.extend(verdict.score.lines());
  match (&verdict.plan, &verdict.plan_error) {
    (Some(report), _) => {
      out.push(String::new());
      out.extend(report.lines());
    }
    (None, Some(err)) => {
      out.push(String::new());
      out.push(format!("plan           REFUSED: {err}"));
      out.push(
        "               a map the planner will not plan is the strongest \
         verdict there is"
          .to_owned(),
      );
    }
    (None, None) => {}
  }
  for note in &verdict.notes {
    out.push(format!("note: {note}"));
  }
  out
}

fn run(args: &ArgMatches, _context: &mut Context) -> Result<()> {
  let world_path = args
    .get_one::<PathBuf>("world")
    .ok_or_else(|| miette!("--world is required"))?;
  let verdict = score_dump(
    world_path,
    args.get_one::<String>("from").map(String::as_str),
    args.get_one::<f64>("radius").copied(),
    args.get_one::<String>("goal").map(String::as_str),
    args.get_one::<String>("bots").map(String::as_str),
    args.get_flag("no-plan"),
  )?;

  if args.get_flag("json") {
    println!(
      "{}",
      serde_json::to_string_pretty(&verdict).into_diagnostic()?
    );
  } else {
    for line in render(&verdict) {
      println!("{line}");
    }
  }
  Ok(())
}

pub(crate) struct ThisCommand {}

pub fn build() -> Box<dyn Subcommand> {
  Box::new(ThisCommand {})
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
  use super::*;
  use factorio_bot_core::test_utils::fixture_world;

  fn dumped_world(dir: &tempfile::TempDir) -> PathBuf {
    let world = fixture_world();
    let path = dir.path().join("world.json");
    world.dump_to(&path).expect("a dump is written");
    path
  }

  #[test]
  fn an_origin_is_two_numbers_or_an_error_naming_the_bad_one() {
    assert_eq!(parse_origin("0,0").unwrap(), Position::new(0., 0.));
    assert_eq!(
      parse_origin(" -12.5 , 8 ").unwrap(),
      Position::new(-12.5, 8.)
    );
    assert!(parse_origin("0").unwrap_err().to_string().contains("x,y"));
    let err = parse_origin("0,north").unwrap_err().to_string();
    assert!(err.contains("north"), "{err}");
  }

  /// The headline: a file in, a verdict out, no game anywhere.
  #[test]
  fn a_dump_is_scored_without_a_factorio() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dumped_world(&dir);
    let verdict = score_dump(&path, None, None, None, Some("1,2"), true).expect("scores");
    assert_eq!(verdict.score.origin, Position::new(0., 0.));
    for entry in &verdict.score.resources {
      assert!(entry.distance.is_some(), "{} not found", entry.name);
    }
    assert!(verdict.plan.is_none(), "--no-plan skips the makespan tier");
  }

  /// The distance tier does not need a roster; the makespan tier does, and
  /// says so rather than inventing one.
  #[test]
  fn a_playerless_dump_still_scores_and_explains_the_missing_makespan() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dumped_world(&dir);
    let verdict = score_dump(&path, None, None, None, None, false).expect("scores");
    assert!(verdict.score.walk_score.is_some(), "distances need no bots");
    assert!(verdict.plan.is_none());
    assert!(
      verdict.notes.iter().any(|note| note.contains("--bots")),
      "{:?}",
      verdict.notes
    );
  }

  /// A refused plan is reported, not raised.
  ///
  /// The fixture world has no forces, so `researched:automation` cannot
  /// expand — which stands in for the real case this protects: a map with no
  /// shoreline, where `plan_plant` raises `PowerPlantNeedsWater` and a
  /// command that propagated the error would abort a seed sweep on the first
  /// bad map instead of recording it as bad.
  #[test]
  fn a_map_the_planner_refuses_is_a_verdict_and_not_an_error() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dumped_world(&dir);
    let verdict = score_dump(&path, None, None, None, Some("1,2,3,4"), false)
      .expect("a refusal must not propagate");
    assert!(verdict.plan.is_none());
    let err = verdict.plan_error.clone().expect("a reason is given");
    assert!(!err.is_empty(), "a refusal with no reason is not a verdict");
    let rendered = render(&verdict).join("\n");
    assert!(rendered.contains("REFUSED"), "{rendered}");
  }

  /// The makespan tier runs when the goal can expand.
  #[test]
  fn a_goal_that_expands_gets_a_makespan_beside_the_distances() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dumped_world(&dir);
    let verdict = score_dump(
      &path,
      None,
      None,
      Some("have:iron-plate:8"),
      Some("1,2"),
      false,
    )
    .expect("scores");
    let report = verdict.plan.as_ref().expect("iron plate expands");
    assert!(report.makespan > 0);
    assert!(verdict.plan_error.is_none());
    let rendered = render(&verdict).join("\n");
    assert!(rendered.contains("makespan"), "{rendered}");
    assert!(rendered.contains("iron-ore"), "{rendered}");
  }

  /// Moving the origin moves the score, which is the whole mechanism.
  #[test]
  fn scoring_from_the_ore_beats_scoring_from_spawn() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dumped_world(&dir);
    let spawn = score_dump(&path, None, None, None, Some("1"), true).expect("scores");
    let on_copper = score_dump(&path, Some("-40,0"), None, None, Some("1"), true).expect("scores");
    assert!(on_copper.score.walk_score.unwrap() < spawn.score.walk_score.unwrap());
    assert_eq!(on_copper.score.origin, Position::new(-40., 0.));
  }

  /// A charting caveat travels in the JSON, not only on the terminal.
  #[test]
  fn an_undercharted_dump_carries_its_caveat_into_the_json() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dumped_world(&dir);
    let verdict = score_dump(&path, None, None, None, Some("1"), true).expect("scores");
    assert!(
      verdict
        .notes
        .iter()
        .any(|note| note.contains("charting probes")),
      "the fixture charts 16 tiles and must say so: {:?}",
      verdict.notes
    );
    let json = serde_json::to_string(&verdict).expect("serialises");
    assert!(json.contains("charting probes"), "{json}");
    assert!(
      json.contains("walk_score"),
      "the ranking key is in the JSON"
    );
  }

  #[test]
  fn a_radius_of_zero_is_refused_rather_than_scored_as_nothing_nearby() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dumped_world(&dir);
    let err = score_dump(&path, None, Some(0.), None, Some("1"), true)
      .unwrap_err()
      .to_string();
    assert!(err.contains("--radius"), "{err}");
  }

  #[test]
  fn a_file_that_is_not_a_world_says_so() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("nope.json");
    std::fs::write(&path, "{}").expect("written");
    let err = score_dump(&path, None, None, None, Some("1"), true)
      .unwrap_err()
      .to_string();
    assert!(err.contains("is not a world dump"), "{err}");
  }

  /// The subcommand is reachable from the assembled CLI.
  #[test]
  fn the_command_parses_off_the_root_app() {
    let matches = crate::cli::build_app()
      .try_get_matches_from(["factorio-bot", "score-map", "-w", "map.json", "--json"])
      .expect("parses");
    let scored = matches
      .subcommand_matches("score-map")
      .expect("the subcommand");
    assert_eq!(
      scored.get_one::<PathBuf>("world"),
      Some(&PathBuf::from("map.json"))
    );
    assert!(scored.get_flag("json"));
  }

  /// The recipe for a seed search is in `--help`, where a reader looks, and
  /// it names the trap that has already cost this project a map.
  #[test]
  fn the_help_names_the_seed_trap() {
    assert!(SCORE_AFTER_HELP.contains("--seed ONLY takes effect together with --new"));
    assert!(SCORE_AFTER_HELP.contains("DELETES"));
    assert!(SCORE_AFTER_HELP.contains("--workspace-path"));
  }
}
