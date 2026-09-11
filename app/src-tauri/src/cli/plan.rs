//! `plan`: expand and schedule a goal against a dumped world, offline.
//!
//! # Why this exists
//!
//! Until this landed, **a plan could not be made without launching Factorio**.
//! Evaluating a planner change therefore cost a live run — 20 minutes on the
//! reference run, plus archive extraction, plus a 90-second wait for graphical
//! clients that may or may not connect. That is the single biggest tax on
//! iteration in this repository, and it is what workstream 0 of
//! `docs/superpowers/plans/2026-09-03-closing-the-idle-gap.md` removes.
//!
//! A run writes a world with `world.dump("map.json")` (see
//! `crates/scripting_lua/src/globals/world.rs`); this reads it back and runs
//! the same `expand()` and `schedule()` the run would have. **No Factorio, no
//! RCON, no workspace, no settings file** — the only input is the file. It
//! runs in a fraction of a second.
//!
//! The plan made here is the plan that would have been made there:
//! `crates/planner/tests/world_round_trip.rs` asserts every action, every
//! chain owner and every scheduled step is identical across the round trip,
//! not merely the makespan.
//!
//! # What it cannot tell you
//!
//! Everything it reports is *planned*, not observed. The reference run's
//! headline finding is that 65.9% of its busiest bot's time was idle in ways
//! the plan does not price — a smelt's wait is nobody's action duration — so
//! two of these reports compare two plans honestly, and one of them compared
//! against a run's elapsed ticks compares two different things. See
//! `PlanReport`'s own doc.

use crate::cli::{Subcommand, SubcommandCallback};
use crate::context::Context;
use clap::{Arg, ArgAction, ArgMatches, Command, value_parser};
use factorio_bot_core::factorio::world::FactorioSurface;
use factorio_bot_core::miette::{IntoDiagnostic, Result, miette};
use factorio_bot_core::record::standing::StandingSnapshot;
use factorio_bot_core::serde_json;
use factorio_bot_core::types::Position;
use factorio_bot_planner::goal::{Goal, Holder};
use factorio_bot_planner::method::have::registry_for;
use factorio_bot_planner::standing::{Standing, survivors_of_failure, world_after, world_with};
use factorio_bot_planner::{
  ActionId, BotId, PlanReport, PlanState, PlannerError, StepKind, Ticks, pick_chain_actor,
  plan_best, plan_best_modules,
};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

const PLAN_AFTER_HELP: &str = "\
Goal specs (--goal, repeatable; every one is planned together):

  have:<item>:<count>        end up holding that many, across the whole roster
  produced:<item>:<count>    cause that many to come into existence
  have:<item>:<count>:<recipe>       ... made by that recipe, by name
  produced:<item>:<count>:<recipe>   ... made by that recipe, by name
  producing:<item>:<rate>    stand up machinery yielding that many per minute
  researched:<technology>    finish that research
  charted:<x>:<y>:<radius>   walk bots out until that disc has been looked at

  have:iron-plate:50   produced:stone-furnace:2   researched:automation
  charted:0:0:256      produced:petroleum-gas:100:basic-oil-processing

The trailing <recipe> is a RECIPE name, not a product and not a machine, and
it is optional: leaving it off means what it has always meant -- the planner
picks, and refuses when more than one recipe it can run makes the product.
A recipe that does not produce the item, or one no machine here runs, is
refused by name rather than quietly replaced.

Anything the shorthand cannot say -- a goal held by one named bot, a nested
Goal::All, a Produced that unlocks a technology -- goes in as --goal-json,
which takes the serde form of `Goal` itself:

  --goal-json '{\"Have\":{\"item\":\"iron-plate\",\"count\":50,\"whose\":{\"Bot\":1}}}'

The world file comes from a run: call world.dump(\"map.json\") from a Lua
script and take it out of the scripts directory. A dump taken at setup is a
fresh map; a dump taken after a milestone also carries what is inside every
furnace and chest the run looked in, and every site the game refused a build
at, which is what makes replanning from that milestone mean anything.

--bots defaults to the players present in the dump. Pass it explicitly to plan
the same map for a different roster -- that comparison is exactly what this
command is for, and it is free.

--replan N plans, then applies what the plan BUILT to the world -- placements,
recipes, choppings and mined ore, as map facts on a fresh surface, the way the
mod's writeout would have -- and plans the same goals again against it, N
times. A t=0 dump has no factory in it, so without this every offline plan
meets clean ground and the replan path (\"a cell ALREADY MAKES copper-plate\")
is never reached; a fix on that path was green on all eight baselines and
refused in the live run on 2026-09-09. A replan that refuses is this command
failing, with the round named. --done-by <tick> counts only the steps a
schedule finishes by that tick as executed, which is what a truncated batch
leaves behind. --dump-standing <path> writes the last standing world as a
dump, so `plan --world` and `score-map` can be pointed at it directly.
Inventories, positions, chest contents and research are NOT carried between
rounds; see `factorio_bot_planner::standing` for why that is a stated limit.

--standing-from-run <run-dir> --at-tick <tick> puts the world a FINISHED RUN's
own record says it had at that tick onto the dump before planning: every
non-resource entity in the last `map.jsonl` keyframe at or before the tick,
each bot where the nearest `samples.jsonl` reading had it with what it held,
and every machine recipe. That is how run-1788926478-07032's refusal was
reproduced byte for byte in seconds with no game. --save-standing <path>
writes that snapshot (a few tens of KB) so a test can check it in --
`workspace/runs/` is not in the repository, and a number a note quotes must
be reproducible from master; --standing <path> reads one back. Both compose
with --replan.

--researched <technology> (repeatable) marks it researched on the acting force
before planning -- a stated hypothesis, never a fact off a record, for a leg no
archived dump reaches. The electric offtake arm needs `electronics` open AND a
network standing: on the t=0 dump the flag alone changes nothing (20 burner
arms either way), on a world with a plant standing it turns two arms electric
and lays twelve poles, and the plant alone with the recipe closed keeps every
arm a burner.

No Factorio, no RCON, no workspace and no settings file are involved.";

impl Subcommand for ThisCommand {
  fn name(&self) -> &'static str {
    "plan"
  }
  fn build_command(&self) -> Command {
    Command::new("plan")
      .about("Expand and schedule a goal against a dumped world, without Factorio")
      .after_help(PLAN_AFTER_HELP)
      .arg(
        Arg::new("world")
          .long("world")
          .short('w')
          .value_name("path")
          .required(true)
          .value_parser(value_parser!(PathBuf))
          .help("world dump to plan against, as written by world.dump()"),
      )
      .arg(
        Arg::new("goal")
          .long("goal")
          .short('g')
          .value_name("spec")
          .action(ArgAction::Append)
          .value_parser(value_parser!(String))
          .help("a goal, e.g. have:iron-plate:50 or researched:automation"),
      )
      .arg(
        Arg::new("goal-json")
          .long("goal-json")
          .value_name("json")
          .action(ArgAction::Append)
          .value_parser(value_parser!(String))
          .help("a goal in the serde form of `Goal`, for what the shorthand cannot say"),
      )
      .arg(
        Arg::new("bots")
          .long("bots")
          .short('b')
          .value_name("ids")
          .value_parser(value_parser!(String))
          .help("roster to plan for, e.g. 1,2,3,4 [default: the players in the dump]"),
      )
      .arg(
        Arg::new("json")
          .long("json")
          .action(ArgAction::SetTrue)
          .help("print the report as JSON instead of a table"),
      )
      .arg(
        Arg::new("steps")
          .long("steps")
          .action(ArgAction::SetTrue)
          .help("also list every scheduled step, per bot, with its start and end tick"),
      )
      .arg(
        Arg::new("planner-mode")
          .long("planner-mode")
          .value_name("mode")
          .value_parser(["legacy", "modules"])
          .help("planning engine: legacy (native methods) or modules (module designs) [default: legacy]"),
      )
      .args(standing_args())
  }

  fn build_callback(&self) -> SubcommandCallback {
    |args, context| Box::pin(std::future::ready(run(args, context)))
  }
}

/// The replan and standing-world arguments, kept beside each other because
/// they are one feature: where the first plan starts, and what the next one
/// meets.
fn standing_args() -> Vec<Arg> {
  vec![
    Arg::new("all")
      .long("all")
      .action(ArgAction::SetTrue)
      .help("plan the goals as ONE bundle (a script's goal.all{...}) rather than in sequence"),
    Arg::new("replan")
      .long("replan")
      .value_name("rounds")
      .value_parser(value_parser!(u32))
      .help("after planning, apply what was built to the world and plan again, this many times"),
    Arg::new("done-by")
      .long("done-by")
      .value_name("tick")
      .value_parser(value_parser!(u32))
      .requires("replan")
      .help("with --replan: only steps a schedule finishes by this tick count as built"),
    Arg::new("fail")
      .long("fail")
      .value_name("label")
      .value_parser(value_parser!(String))
      .requires("replan")
      .conflicts_with("done-by")
      .help(
        "with --replan: the first action whose label contains this fails, abandoning \
       everything downstream of it; the rest counts as built",
      ),
    Arg::new("standing")
      .long("standing")
      .value_name("path")
      .value_parser(value_parser!(PathBuf))
      .conflicts_with("standing-from-run")
      .help("a standing snapshot (see --save-standing) to put on the dump before planning"),
    Arg::new("standing-from-run")
      .long("standing-from-run")
      .value_name("run-dir")
      .value_parser(value_parser!(PathBuf))
      .requires("at-tick")
      .help("a run directory whose record says what stood; needs --at-tick"),
    Arg::new("at-tick")
      .long("at-tick")
      .value_name("tick")
      .value_parser(value_parser!(u64))
      .requires("standing-from-run")
      .help("with --standing-from-run: the game tick to take the world at"),
    Arg::new("researched")
      .long("researched")
      .value_name("technology")
      .action(ArgAction::Append)
      .value_parser(value_parser!(String))
      .help(
        "mark this technology researched before planning (repeatable) -- a stated hypothesis, \
         for a leg no archived dump reaches",
      ),
    Arg::new("save-standing")
      .long("save-standing")
      .value_name("path")
      .value_parser(value_parser!(PathBuf))
      .help("write the standing snapshot the plan started from, for checking in beside a test"),
    Arg::new("dump-standing")
      .long("dump-standing")
      .value_name("path")
      .value_parser(value_parser!(PathBuf))
      .requires("replan")
      .help("with --replan: write the last standing world as a dump to this path"),
  ]
}

/// One goal from the `--goal` shorthand.
///
/// Deliberately narrow. Everything it accepts names a `Holder::Anyone`,
/// because a shorthand that silently picked a bot would be inventing the one
/// decision that most changes a plan — `Holder::Bot` and `Holder::Share` both
/// weld a whole subtree to one pair of hands, which is what the reference run
/// found gave bot 1 all 25 crafts. Naming a bot is possible and takes
/// `--goal-json`, where it is visible.
pub(crate) fn parse_goal(spec: &str) -> Result<Goal> {
  let parts: Vec<&str> = spec.split(':').collect();
  let count = |raw: &str| -> Result<u32> {
    raw
      .parse::<u32>()
      .map_err(|_| miette!("`{raw}` in `{spec}` is not a count"))
  };
  // Separate from `count` because a coordinate is signed and fractional and a
  // count is neither. Rejecting non-finite input here rather than letting an
  // `inf` radius reach `PlanState::charting`, where it would place every probe
  // at infinity and report the map as uniformly blind.
  let coord = |raw: &str| -> Result<f64> {
    match raw.parse::<f64>() {
      Ok(value) if value.is_finite() => Ok(value),
      _ => Err(miette!("`{raw}` in `{spec}` is not a finite coordinate")),
    }
  };
  match parts.as_slice() {
    ["researched", tech] if !tech.is_empty() => Ok(Goal::Researched((*tech).to_owned())),
    ["have", item, n] if !item.is_empty() => Ok(Goal::Have {
      item: (*item).to_owned(),
      count: count(n)?,
      whose: Holder::Anyone,
      via: None,
    }),
    ["produced", item, n] if !item.is_empty() => Ok(Goal::Produced {
      item: (*item).to_owned(),
      count: count(n)?,
      whose: Holder::Anyone,
      unlocks: None,
      via: None,
    }),
    // `have:<item>:<count>:<recipe>` and `produced:<item>:<count>:<recipe>`.
    //
    // The recipe goes in the POSITIONAL form rather than only in
    // `--goal-json`, deliberately: the refusal that sends a caller here says
    // *"ask for a recipe by name rather than for the product"*, and a message
    // whose remedy is "now rewrite your goal as JSON" is a worse seam than a
    // fourth field. The usual objection to another colon is mis-ordering, and
    // it does not apply: `<count>` is a number and a recipe name is not, so
    // `produced:petroleum-gas:basic-oil-processing:100` fails loudly on the
    // count rather than planning something.
    //
    // Arity keeps it unambiguous against every other shorthand -- `sustain`
    // is the only other four-part form and its tag differs.
    ["have", item, n, recipe] if !item.is_empty() && !recipe.is_empty() => Ok(Goal::Have {
      item: (*item).to_owned(),
      count: count(n)?,
      whose: Holder::Anyone,
      via: Some((*recipe).to_owned()),
    }),
    ["produced", item, n, recipe] if !item.is_empty() && !recipe.is_empty() => Ok(Goal::Produced {
      item: (*item).to_owned(),
      count: count(n)?,
      whose: Holder::Anyone,
      unlocks: None,
      via: Some((*recipe).to_owned()),
    }),
    // `gathered:<resource-entity>` -- note the argument is an ENTITY, not an
    // item: `crude-oil` here names the well in the ground, the same way
    // `Goal::Extracted` does, and what comes out of it is a fluid no
    // inventory can hold.
    ["gathered", entity] if !entity.is_empty() => Ok(Goal::Gathered {
      entity: (*entity).to_owned(),
      unlocks: None,
    }),
    ["producing", item, n] if !item.is_empty() => Ok(Goal::Producing {
      item: (*item).to_owned(),
      per_minute: count(n)?,
    }),
    // `sustain:<item>:<rate>:<window-ticks>`. The window is a positional part
    // and not an optional suffix: it has no default here for the same reason
    // it has none in Lua or in `sustained_rate` -- the window is what decides
    // what a failure means, and a shorthand that guessed one would hand back a
    // verdict nobody derived. Ticks, because they are the only clock the
    // record, the planner and the mod share.
    ["sustain", item, n, window] if !item.is_empty() => Ok(Goal::Sustain {
      item: (*item).to_owned(),
      per_minute: count(n)?,
      window_ticks: count(window)?,
    }),
    // `orbiting` on its own, or `orbiting:<planet>`. The only shorthand with
    // a zero-argument form, and the defaults are `method::orbit`'s constants
    // rather than literals typed here: the first platform does not move, so it
    // is Nauvis orbit and the shipped starter pack. A pack other than the
    // shipped one needs `--goal-json`, which is what that flag is for.
    ["orbiting"] => Ok(Goal::Orbiting {
      planet: factorio_bot_planner::method::orbit::FIRST_PLANET.to_owned(),
      starter_pack: factorio_bot_planner::method::orbit::STARTER_PACK.to_owned(),
      unlocks: None,
    }),
    ["orbiting", planet] if !planet.is_empty() => Ok(Goal::Orbiting {
      planet: (*planet).to_owned(),
      starter_pack: factorio_bot_planner::method::orbit::STARTER_PACK.to_owned(),
      unlocks: None,
    }),
    // `charted:<x>:<y>:<radius>`. The one shorthand whose arguments are
    // coordinates rather than an item name, and it takes all three because
    // there is no sane default for *where*: spawn is only the right centre
    // while nothing has moved, and this goal exists precisely for the plans
    // that have.
    ["charted", x, y, radius] => Ok(Goal::Charted {
      around: Position::new(coord(x)?, coord(y)?),
      radius: coord(radius)?,
    }),
    _ => Err(miette!(
      "`{spec}` is not a goal. Expected have:<item>:<count>[:<recipe>], \
       produced:<item>:<count>[:<recipe>], producing:<item>:<per-minute>, \
       sustain:<item>:<per-minute>:<window-ticks>, \
       gathered:<resource-entity>, orbiting[:<planet>], \
       charted:<x>:<y>:<radius> or researched:<technology> -- or --goal-json \
       for anything else."
    )),
  }
}

/// The roster, as `--bots 1,2,3,4`.
///
/// A `BotId` is the **Factorio player id**, never an index, so this parses
/// what it is given and never renumbers it. `[1, 3]` is an ordinary roster
/// (bot 2's client failed to connect) and packing it to `[1, 2]` would plan
/// for a player that is not there.
pub(crate) fn parse_roster(raw: &str) -> Result<Vec<BotId>> {
  let mut bots = Vec::new();
  for part in raw.split(',') {
    let part = part.trim();
    if part.is_empty() {
      continue;
    }
    let id: u8 = part
      .parse()
      .map_err(|_| miette!("`{part}` in `{raw}` is not a player id"))?;
    let bot = BotId(id);
    if !bots.contains(&bot) {
      bots.push(bot);
    }
  }
  bots.sort();
  if bots.is_empty() {
    return Err(miette!("--bots named no players"));
  }
  Ok(bots)
}

/// Whoever the dump has a player for, in id order.
///
/// Used when `--bots` is absent, so the default answer is "the roster the run
/// that took this dump actually had" rather than a number invented here.
pub(crate) fn roster_from(world: &FactorioSurface) -> Vec<BotId> {
  let mut bots: Vec<BotId> = world
    .globals
    .players
    .iter()
    .map(|entry| BotId(*entry.key()))
    .collect();
  bots.sort();
  bots
}

/// Reads a dump off disk, or says which file and why not.
///
/// `pub(crate)` because `score-map` reads the same file for the same reason
/// and a second `serde_json::from_str` with a different error message would
/// be a second answer to "is this a world dump".
pub(crate) fn load_world(world_path: &std::path::Path) -> Result<Arc<FactorioSurface>> {
  let raw = std::fs::read_to_string(world_path)
    .map_err(|err| miette!("could not read {}: {err}", world_path.display()))?;
  let world: FactorioSurface = serde_json::from_str(&raw)
    .map_err(|err| miette!("{} is not a world dump: {err}", world_path.display()))?;
  Ok(Arc::new(world))
}

/// Where the first round starts: the dump as it is, or a run's own record
/// of what stood at a tick, put on the dump first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Start {
  #[default]
  Dump,
  /// A snapshot already on disk.
  Snapshot(PathBuf),
  /// A run directory and the tick to read it at.
  Run { dir: PathBuf, tick: u64 },
}

/// How many times to replan, and what counts as built between rounds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Replan {
  /// What the first round plans against.
  pub start: Start,
  /// Where to write the snapshot the first round started from, if anywhere.
  pub save_standing: Option<PathBuf>,
  /// Technologies to mark researched before the first round.
  pub researched: Vec<String>,
  /// Rounds *after* the first plan. Zero is the plain command.
  pub rounds: u32,
  /// Which planning engine to use: "legacy" (default) or "modules".
  pub planner_mode: String,
  /// Only steps a schedule finishes by this tick are applied to the world
  /// before the next round; `None` applies the whole plan.
  pub done_by: Option<Ticks>,
  /// The first action whose label contains this fails, and its dependency
  /// cone is abandoned -- the executor's rule, and the shape the live
  /// replans met. See `standing::survivors_of_failure`.
  pub fail: Option<String>,
  /// Where to write the last standing world, if anywhere.
  pub dump_standing: Option<PathBuf>,
}

/// One planning round's outcome, kept so a replan can be reported beside the
/// plan it followed.
pub(crate) struct Round {
  /// `None` when the round found the whole arrangement standing and emitted
  /// nothing -- `PlannerError::SustainSupplyNotStanding`, which the driver
  /// reads as a satisfied milestone and this command reads the same way.
  /// Only a replan can end here; the first round on a t=0 dump has
  /// everything left to build.
  pub report: Option<PlanReport>,
  /// What the previous round left standing on the world this one planned
  /// against; `None` for the first round, which meets the dump as it is.
  pub standing: Option<Standing>,
  pub listing: Vec<String>,
}

/// Reads the dump, plans, and returns every round's report plus the lines to
/// print ahead of them.
///
/// Split out of [`run`] so it is testable without a `Context` — everything
/// this command does is a pure function of a file and some arguments, and a
/// test that had to build a `Context` would be testing the CLI harness.
///
/// With `replan.rounds > 0` the plan is applied to the world through
/// [`factorio_bot_planner::standing::world_after`] and the goals are planned
/// again on a **fresh** state over that world -- the supervisor's own shape
/// (`plan_rounds` rebuilds its state from the live world every round), and
/// the only offline path that reaches the replan-only refusals. A round that
/// refuses is an error naming the round, because a goal that plans from t=0
/// and refuses from its own standing world is the failure this flag exists
/// to make visible.
// Eight parameters, and the eighth is the point: `notes` is an out-parameter
// so a refusal still reports what stood, which a return value cannot do.
#[allow(clippy::too_many_arguments)]
fn plan_from_dump(
  world_path: &std::path::Path,
  specs: &[String],
  goal_json: &[String],
  roster: Option<&str>,
  steps: bool,
  bundle: bool,
  replan: &Replan,
  notes: &mut Vec<String>,
) -> Result<Vec<Round>> {
  let world = load_world(world_path)?;

  let goals = goals_from(specs, goal_json, bundle)?;
  let bots = roster_for(roster, &world)?;
  // Through the out-parameter, so a refusal still says what stood: a
  // replan refusal with the standing line lost would read as a t=0 refusal.
  let (mut world, mut standing) = starting_world(world, replan, notes)?;
  let mut rounds = Vec::new();
  for round in 0..=replan.rounds {
    let state = PlanState::from_world(world.clone(), &bots);
    // Not an error. A dump can legitimately be planned for a roster it has
    // no players for -- comparing one map's plan across roster sizes is
    // exactly what this command is for -- but a bot the world knows nothing
    // about is seeded with an empty inventory and guessed reach distances,
    // and a report that did not say so would read as a measurement of the
    // roster rather than of four defaults.
    if round == 0 {
      let unknown: Vec<String> = state
        .unknown_bots()
        .iter()
        .map(|bot| bot.0.to_string())
        .collect();
      if !unknown.is_empty() {
        notes.push(format!(
          "note: the dump has no player for bot(s) {} -- they are planned with an \
           empty inventory and default reach",
          unknown.join(", ")
        ));
      }
    }

    let chain_actor = pick_chain_actor(&state, &bots)
      .ok_or_else(|| miette!("no bots to plan for; a roster needs at least one"))?;
    // `plan_best`, not `expand` + `schedule`: the drain policy is settled by
    // reading finished schedules, so the CLI has to make the same choice a
    // run makes or the offline loop stops predicting it.
    // Counted, not timed: see `factorio_bot_core::plan_work` for why a wall
    // clock cannot be the regression guard on a box whose load ranged from 1
    // to 80 in one day.
    let (planned, work) = factorio_bot_core::plan_work::measure(|| {
      match replan.planner_mode.as_str() {
        "modules" => plan_best_modules(&goals, &state, &registry_for(&bots), chain_actor, &bots),
        _ => plan_best(&goals, &state, &registry_for(&bots), chain_actor, &bots),
      }
    });
    let (net, scheduled, _memory) = match planned {
      Ok(plan) => plan,
      // The driver's own reading of this code (`goal/mod.rs` in
      // `scripting_lua`): the whole arrangement stands and there is nothing
      // left to build, so the milestone is satisfied. On a replan that is the
      // expected end of a plan applied in full, not a refusal -- and it can
      // only happen on a replan, because a t=0 dump has everything to build.
      Err(PlannerError::SustainSupplyNotStanding { .. }) if round > 0 => {
        notes.push(format!(
          "replan {round}: the whole arrangement stands and nothing is left to build, which \
           the driver reads as a satisfied milestone"
        ));
        rounds.push(Round {
          report: None,
          standing: standing.take(),
          listing: Vec::new(),
        });
        break;
      }
      // "did not PLAN", not "did not expand". `plan_best` covers expansion
      // *and* scheduling, and the two fail for different reasons -- a
      // `ChainOwnerInfeasible` is raised in `schedule.rs`, never in `expand`.
      // The old wording sent a session into `expand()` looking for a
      // scheduling failure, the same way `occupant_of` once reported
      // `Terrain` for a refusal that was `Refused`.
      Err(err) if round == 0 => return Err(miette!("the goal did not plan: {err}")),
      Err(err) => {
        return Err(miette!(
          "the goal planned from the dump but REFUSED on replan {round} of {}, against \
           the world its own previous plan left standing ({}): {err}",
          replan.rounds,
          describe_standing(standing.as_ref())
        ));
      }
    };

    let listing = if steps {
      step_lines(&scheduled, &bots)
    } else {
      Vec::new()
    };
    rounds.push(Round {
      report: Some(PlanReport::of(&net, &scheduled, &bots, &state).with_work(work)),
      standing: standing.take(),
      listing,
    });

    if round == replan.rounds {
      break;
    }
    let done = done_after(&net, &scheduled, replan, round, notes)?;
    let (after, built) = world_after(&state, &net, |id| done.contains(&id))
      .map_err(|err| miette!("could not apply plan {round} to the world: {err}"))?;
    standing = Some(built);
    world = after;
  }

  if let Some(path) = &replan.dump_standing {
    world.dump_to(path).map_err(|err| {
      miette!(
        "could not write the standing world to {}: {err}",
        path.display()
      )
    })?;
    notes.push(format!(
      "wrote the standing world after {} round(s) to {}",
      replan.rounds,
      path.display()
    ));
  }
  Ok(rounds)
}

/// The goals, as one bundle when asked.
fn goals_from(specs: &[String], goal_json: &[String], bundle: bool) -> Result<Vec<Goal>> {
  let mut goals: Vec<Goal> = Vec::new();
  for spec in specs {
    goals.push(parse_goal(spec)?);
  }
  for json in goal_json {
    goals.push(serde_json::from_str(json).into_diagnostic()?);
  }
  if goals.is_empty() {
    return Err(miette!(
      "nothing to plan: pass at least one --goal or --goal-json"
    ));
  }
  // `goal.all { a, b }` is not `a; b`: a bundle holds one conjunct's
  // "already standing" refusal back while the others expand, and a sequence
  // ends at it. A replan of a script's bundle has to be planned as the
  // bundle or the second round refuses on the first conjunct that stands.
  if bundle {
    goals = vec![Goal::All(goals)];
  }
  Ok(goals)
}

/// `--bots`, or the players the dump has.
fn roster_for(roster: Option<&str>, world: &FactorioSurface) -> Result<Vec<BotId>> {
  if let Some(raw) = roster {
    return parse_roster(raw);
  }
  let found = roster_from(world);
  if found.is_empty() {
    return Err(miette!(
      "the dump has no players, so there is no default roster -- pass \
       --bots, e.g. --bots 1,2,3,4"
    ));
  }
  Ok(found)
}

/// The world the first round plans against: the dump, or the dump with a
/// run's record put on it, with what that put down and the lines to say so.
fn starting_world(
  world: Arc<FactorioSurface>,
  replan: &Replan,
  notes: &mut Vec<String>,
) -> Result<(Arc<FactorioSurface>, Option<Standing>)> {
  let snapshot = match &replan.start {
    Start::Dump => None,
    Start::Snapshot(path) => Some(
      StandingSnapshot::read_from(path)
        .map_err(|err| miette!("could not read the standing snapshot: {err}"))?,
    ),
    Start::Run { dir, tick } => {
      // The dump already carries the ore; a keyframe lists it too.
      let prototypes = world.globals.entity_prototypes.clone();
      let built = |name: &str| {
        prototypes
          .get(name)
          .is_none_or(|prototype| prototype.entity_type != "resource")
      };
      Some(
        StandingSnapshot::from_run(dir, *tick, built)
          .map_err(|err| miette!("could not read {} at tick {tick}: {err}", dir.display()))?,
      )
    }
  };
  let snapshot = match (snapshot, replan.researched.is_empty()) {
    (Some(mut snapshot), _) => {
      snapshot
        .researched
        .extend(replan.researched.iter().cloned());
      Some(snapshot)
    }
    (None, false) => Some(StandingSnapshot {
      researched: replan.researched.clone(),
      ..StandingSnapshot::default()
    }),
    (None, true) => None,
  };
  if let Some(snapshot) = &snapshot {
    let (next, built) = world_with(&world, snapshot);
    notes.push(format!(
      "standing world from {} at keyframe tick {} (bots sample {}, {} divergence(s) between \
       game and model): {}",
      snapshot.run.as_deref().unwrap_or("an unnamed run"),
      snapshot.keyframe_tick,
      snapshot
        .bots_sample_tick
        .map_or_else(|| "none".to_string(), |t| t.to_string()),
      snapshot.divergences,
      describe_standing(Some(&built))
    ));
    if built.researched < snapshot.researched.len() {
      return Err(miette!(
        "--researched named a technology the dump's force does not have ({})",
        describe_standing(Some(&built))
      ));
    }
    if built.placed == 0 && !snapshot.entities.is_empty() {
      return Err(miette!(
        "the standing snapshot put nothing on the dump ({}) -- a plan against it would be a \
         plan against t=0 wearing a run's name",
        describe_standing(Some(&built))
      ));
    }
    if let Some(path) = &replan.save_standing {
      snapshot
        .write_to(path)
        .map_err(|err| miette!("could not write the snapshot to {}: {err}", path.display()))?;
      notes.push(format!("wrote the standing snapshot to {}", path.display()));
    }
    return Ok((next, Some(built)));
  }
  if let Some(path) = &replan.save_standing {
    return Err(miette!(
      "--save-standing needs a standing world to save: pass --standing-from-run <dir> \
       --at-tick <tick>; {} was not written",
      path.display()
    ));
  }
  Ok((world, None))
}

/// Which of a round's actions count as executed before the next round.
///
/// `done_by` reads the SCHEDULE, not the network: a step's end tick is the
/// only place "finished by tick T" is defined. `fail` reads the network,
/// because an abandoned subtree is a dependency cone, not a suffix.
fn done_after(
  net: &factorio_bot_planner::ActionNetwork,
  scheduled: &factorio_bot_planner::Schedule,
  replan: &Replan,
  round: u32,
  notes: &mut Vec<String>,
) -> Result<BTreeSet<ActionId>> {
  let done: BTreeSet<ActionId> = if let Some(needle) = &replan.fail {
    let failed = scheduled
      .steps
      .iter()
      .find_map(|step| match &step.what {
        StepKind::Act { action, label } if label.contains(needle.as_str()) => Some(*action),
        _ => None,
      })
      .ok_or_else(|| {
        miette!("--fail: no action in plan {round} has a label containing `{needle}`")
      })?;
    notes.push(format!(
      "plan {round}: `{}` fails, abandoning everything downstream of it",
      net.action(failed).map_or("?", |a| a.label.as_str())
    ));
    survivors_of_failure(net, failed)
  } else {
    scheduled
      .steps
      .iter()
      .filter(|step| replan.done_by.is_none_or(|tick| step.end <= tick))
      .filter_map(|step| match &step.what {
        StepKind::Act { action, .. } => Some(*action),
        StepKind::Walk { .. } => None,
      })
      .collect()
  };
  Ok(done)
}

/// One line saying what a replan met, for the refusal and the round header.
fn describe_standing(standing: Option<&Standing>) -> String {
  match standing {
    Some(s) => format!(
      "{} placed, {} recipe(s) set, {} chopped, {} ore tile(s) mined, {} researched, {} not \
       applied",
      s.placed, s.recipes_set, s.chopped, s.mined, s.researched, s.unapplied
    ),
    None => "the dump as it is".to_string(),
  }
}

/// Every scheduled step, per bot, with the gap that precedes it.
///
/// The report says a bot idled 39,892 ticks; this says *where*. Idle in a
/// schedule is always a wait on somebody else's finish or on a lag edge, and
/// which one it is decides whether the fix is more bots or a shorter lag --
/// a distinction the aggregate cannot make.
fn step_lines(scheduled: &factorio_bot_planner::Schedule, bots: &[BotId]) -> Vec<String> {
  let mut out = Vec::new();
  for bot in bots {
    out.push(format!("--- {bot} ---"));
    let mut previous_end = 0u32;
    for step in scheduled.steps_for(*bot) {
      let gap = step.start.saturating_sub(previous_end);
      let label = match &step.what {
        factorio_bot_planner::StepKind::Act { action, label } => format!("#{} {label}", action.0),
        factorio_bot_planner::StepKind::Walk { to, .. } => format!("walk to {to}"),
      };
      out.push(format!(
        "{:>7} {:>7} {:>7}  idle {:>6}  {}",
        step.start,
        step.end,
        step.end.saturating_sub(step.start),
        gap,
        label
      ));
      previous_end = step.end;
    }
  }
  out
}

/// Not `async`, and the callback below wraps it in `std::future::ready` --
/// the same shape `roll_seed` uses. There is nothing here to await: the whole
/// command is a file read and a pure computation, which is precisely the claim
/// this subcommand makes about itself.
fn run(args: &ArgMatches, _context: &mut Context) -> Result<()> {
  let world_path = args
    .get_one::<PathBuf>("world")
    .ok_or_else(|| miette!("--world is required"))?;
  let specs: Vec<String> = args
    .get_many::<String>("goal")
    .map(|values| values.cloned().collect())
    .unwrap_or_default();
  let goal_json: Vec<String> = args
    .get_many::<String>("goal-json")
    .map(|values| values.cloned().collect())
    .unwrap_or_default();
  let roster = args.get_one::<String>("bots").map(String::as_str);

  let start = if let Some(path) = args.get_one::<PathBuf>("standing") {
    Start::Snapshot(path.clone())
  } else if let Some(dir) = args.get_one::<PathBuf>("standing-from-run") {
    Start::Run {
      dir: dir.clone(),
      tick: *args
        .get_one::<u64>("at-tick")
        .ok_or_else(|| miette!("--standing-from-run needs --at-tick"))?,
    }
  } else {
    Start::Dump
  };
  let replan = Replan {
    start,
    save_standing: args.get_one::<PathBuf>("save-standing").cloned(),
    researched: args
      .get_many::<String>("researched")
      .map(|values| values.cloned().collect())
      .unwrap_or_default(),
    rounds: args.get_one::<u32>("replan").copied().unwrap_or(0),
    done_by: args.get_one::<u32>("done-by").copied(),
    fail: args.get_one::<String>("fail").cloned(),
    dump_standing: args.get_one::<PathBuf>("dump-standing").cloned(),
    planner_mode: args.get_one::<String>("planner-mode").cloned().unwrap_or_else(|| "legacy".to_string()),
  };

  let mut notes = Vec::new();
  let planned = plan_from_dump(
    world_path,
    &specs,
    &goal_json,
    roster,
    args.get_flag("steps"),
    args.get_flag("all"),
    &replan,
    &mut notes,
  );
  for note in &notes {
    eprintln!("{note}");
  }
  let rounds = planned?;
  let several = rounds.len() > 1;
  for (index, round) in rounds.iter().enumerate() {
    if several {
      println!(
        "=== plan {index}: against {} ===",
        describe_standing(round.standing.as_ref())
      );
    }
    for line in &round.listing {
      println!("{line}");
    }
    let Some(report) = &round.report else {
      println!("(nothing left to build: the whole arrangement stands)");
      continue;
    };
    if args.get_flag("json") {
      println!(
        "{}",
        serde_json::to_string_pretty(report).into_diagnostic()?
      );
    } else {
      for line in report.lines() {
        println!("{line}");
      }
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

  /// The plain, one-round command, which is what every test below that is
  /// not about replanning asks for. Shadows the outer function on purpose: a
  /// local item wins over the glob import, and the tests read as they did.
  fn plan_from_dump(
    world_path: &std::path::Path,
    specs: &[String],
    goal_json: &[String],
    roster: Option<&str>,
    steps: bool,
  ) -> Result<(PlanReport, Vec<String>, Vec<String>)> {
    let mut notes = Vec::new();
    let mut rounds = super::plan_from_dump(
      world_path,
      specs,
      goal_json,
      roster,
      steps,
      false,
      &Replan::default(),
      &mut notes,
    )?;
    let round = rounds.remove(0);
    let report = round.report.expect("a first round always has a plan");
    Ok((report, notes, round.listing))
  }

  /// `--replan 1`: the second round meets what the first one built, as map
  /// facts, and says so.
  #[test]
  fn a_replan_meets_what_the_first_plan_built() {
    let dir = tempfile::tempdir().unwrap();
    let path = dumped_world(&dir);
    let goal = ["have:iron-plate:5".to_string()];
    let mut notes_ignored = Vec::new();
    let rounds = super::plan_from_dump(
      &path,
      &goal,
      &[],
      Some("1"),
      false,
      false,
      &Replan {
        rounds: 1,
        ..Replan::default()
      },
      &mut notes_ignored,
    )
    .expect("plans twice");
    assert_eq!(rounds.len(), 2);
    assert!(
      rounds[0].standing.is_none(),
      "the first round meets the dump"
    );
    let standing = rounds[1]
      .standing
      .as_ref()
      .expect("the second round reports what it met");
    assert!(
      standing.placed > 0,
      "a five-plate plan on the fixture builds a furnace, so the replan must meet one: {standing:?}"
    );
  }

  /// `--done-by 0`: nothing finishes by tick zero, so the replan meets the
  /// dump unchanged -- and the report says zero placed rather than nothing.
  #[test]
  fn done_by_zero_leaves_nothing_standing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dumped_world(&dir);
    let goal = ["have:iron-plate:5".to_string()];
    let mut notes_ignored = Vec::new();
    let rounds = super::plan_from_dump(
      &path,
      &goal,
      &[],
      Some("1"),
      false,
      false,
      &Replan {
        rounds: 1,
        done_by: Some(0),
        ..Replan::default()
      },
      &mut notes_ignored,
    )
    .expect("plans twice");
    let standing = rounds[1].standing.as_ref().unwrap();
    assert_eq!(standing.placed, 0, "{standing:?}");
    assert_eq!(
      rounds[0].report.as_ref().map(|r| r.actions),
      rounds[1].report.as_ref().map(|r| r.actions),
      "with nothing built the replan is the same plan"
    );
  }

  /// `--standing-from-run`: the run's keyframe stands on the dump before the
  /// first plan, `--save-standing` writes it, and `--standing` reads it back
  /// to the same world.
  #[test]
  fn a_runs_record_puts_its_world_on_the_dump_before_the_first_plan() {
    let dir = tempfile::tempdir().unwrap();
    let path = dumped_world(&dir);
    let run = dir.path().join("run-test");
    std::fs::create_dir(&run).unwrap();
    // One keyframe with a furnace the dump does not have, and one bots
    // sample moving bot 1 -- the two things a snapshot exists to carry.
    std::fs::write(
      run.join("map.jsonl"),
      concat!(
        r#"{"tick":900,"kind":"keyframe","bounds":{"left":0,"top":0,"right":1,"bottom":1},"#,
        r#""game":[],"model":[{"name":"stone-furnace","position":{"x":10.5,"y":10.5},"direction":0}],"#,
        r#""divergence":[]}"#,
        "\n"
      ),
    )
    .unwrap();
    let goal = ["have:iron-plate:5".to_string()];
    let saved = dir.path().join("standing.json");
    let mut notes = Vec::new();
    let rounds = super::plan_from_dump(
      &path,
      &goal,
      &[],
      Some("1"),
      false,
      false,
      &Replan {
        start: Start::Run {
          dir: run.clone(),
          tick: 1_000,
        },
        save_standing: Some(saved.clone()),
        ..Replan::default()
      },
      &mut notes,
    )
    .expect("plans against the run's world");
    let standing = rounds[0]
      .standing
      .as_ref()
      .expect("the first round says what the record put down");
    assert_eq!(standing.placed, 1, "{standing:?}");
    assert!(
      notes.iter().any(|n| n.contains("keyframe tick 900")),
      "{notes:?}"
    );
    assert!(saved.exists(), "the snapshot was written for checking in");

    let mut again_notes = Vec::new();
    let again = super::plan_from_dump(
      &path,
      &goal,
      &[],
      Some("1"),
      false,
      false,
      &Replan {
        start: Start::Snapshot(saved),
        ..Replan::default()
      },
      &mut again_notes,
    )
    .expect("plans against the saved snapshot");
    assert_eq!(
      again[0].standing, rounds[0].standing,
      "the saved snapshot puts the same world on the dump"
    );

    let before = StandingSnapshot::from_run(&run, 100, |_| true).unwrap_err();
    assert!(
      before
        .to_string()
        .contains("no keyframe at or before tick 100"),
      "{before}"
    );
  }

  /// `--dump-standing` writes a world `plan --world` can read back, with the
  /// first plan's furnace in it.
  #[test]
  fn the_standing_world_can_be_dumped_and_planned_against() {
    let dir = tempfile::tempdir().unwrap();
    let path = dumped_world(&dir);
    let standing_path = dir.path().join("standing.json");
    let goal = ["have:iron-plate:5".to_string()];
    let mut notes = Vec::new();
    let _rounds = super::plan_from_dump(
      &path,
      &goal,
      &[],
      Some("1"),
      false,
      false,
      &Replan {
        rounds: 1,
        dump_standing: Some(standing_path.clone()),
        ..Replan::default()
      },
      &mut notes,
    )
    .expect("plans twice");
    assert!(
      notes.iter().any(|n| n.contains("wrote the standing world")),
      "{notes:?}"
    );
    let standing = load_world(&standing_path).expect("the standing dump reads back");
    let state = PlanState::from_world(standing, &[BotId(1)]);
    assert!(
      !state.entities_named("stone-furnace").is_empty(),
      "the furnace the first plan placed stands in the dumped world"
    );
  }

  /// Writes a fixture world where a dump would be, and hands back the path.
  fn dumped_world(dir: &tempfile::TempDir) -> PathBuf {
    let world = fixture_world();
    let path = dir.path().join("world.json");
    world.dump_to(&path).expect("a dump is written");
    path
  }

  #[test]
  fn the_shorthand_covers_the_four_goal_shapes() {
    assert_eq!(
      parse_goal("researched:automation").unwrap(),
      Goal::Researched("automation".into())
    );
    assert_eq!(
      parse_goal("have:iron-plate:50").unwrap(),
      Goal::Have {
        item: "iron-plate".into(),
        count: 50,
        whose: Holder::Anyone,
        via: None,
      }
    );
    assert_eq!(
      parse_goal("produced:stone-furnace:2").unwrap(),
      Goal::Produced {
        item: "stone-furnace".into(),
        count: 2,
        whose: Holder::Anyone,
        unlocks: None,
        via: None,
      }
    );
    // The fourth field is the recipe, and it is OPTIONAL: the two assertions
    // above carry `via: None` and are the shape every existing script and
    // baseline uses.
    assert_eq!(
      parse_goal("have:iron-gear-wheel:5:iron-gear-wheel").unwrap(),
      Goal::Have {
        item: "iron-gear-wheel".into(),
        count: 5,
        whose: Holder::Anyone,
        via: Some("iron-gear-wheel".into()),
      }
    );
    assert_eq!(
      parse_goal("produced:petroleum-gas:100:basic-oil-processing").unwrap(),
      Goal::Produced {
        item: "petroleum-gas".into(),
        count: 100,
        whose: Holder::Anyone,
        unlocks: None,
        via: Some("basic-oil-processing".into()),
      }
    );
    // The mis-ordering objection to a fourth colon, answered by types rather
    // than by care: a recipe name does not parse as a count, so the swapped
    // form is refused loudly instead of planning something.
    let err = parse_goal("produced:petroleum-gas:basic-oil-processing:100")
      .unwrap_err()
      .to_string();
    assert!(err.contains("is not a count"), "{err}");
    // An empty recipe is not "no recipe": it falls through to the catch-all
    // rather than becoming `via: Some("")`, which would reach the planner as
    // a recipe nothing has.
    assert!(parse_goal("have:iron-plate:5:").is_err());

    assert_eq!(
      parse_goal("producing:iron-plate:30").unwrap(),
      Goal::Producing {
        item: "iron-plate".into(),
        per_minute: 30
      }
    );
    assert_eq!(
      parse_goal("sustain:iron-plate:15:7200").unwrap(),
      Goal::Sustain {
        item: "iron-plate".into(),
        per_minute: 15,
        window_ticks: 7200
      }
    );
  }

  /// The window is not optional, and a `sustain` missing it is not silently
  /// read as a `producing`.
  #[test]
  fn a_sustain_goal_without_a_window_is_refused() {
    let err = parse_goal("sustain:iron-plate:15").unwrap_err().to_string();
    assert!(
      err.contains("sustain:<item>:<per-minute>:<window-ticks>"),
      "the refusal names the shape it wanted: {err}"
    );
    assert!(parse_goal("sustain:iron-plate:15:forever").is_err());
    assert!(
      parse_goal("sustain::15:7200").is_err(),
      "an item is required"
    );
  }

  /// A misspelled goal is refused, not guessed at.
  #[test]
  fn an_unrecognised_goal_names_what_was_expected() {
    let err = parse_goal("mine:iron-ore:50").unwrap_err().to_string();
    assert!(err.contains("have:<item>:<count>"), "{err}");
    assert!(
      parse_goal("have:iron-plate").is_err(),
      "a count is required"
    );
    assert!(parse_goal("have:iron-plate:lots").is_err());
    assert!(parse_goal("have::4").is_err(), "an item is required");
    assert!(parse_goal("researched:").is_err());
  }

  /// A roster is player ids, and they are never renumbered.
  #[test]
  fn a_roster_keeps_the_player_ids_it_was_given() {
    assert_eq!(parse_roster("1,3").unwrap(), vec![BotId(1), BotId(3)]);
    assert_eq!(parse_roster(" 4 , 2 ").unwrap(), vec![BotId(2), BotId(4)]);
    assert_eq!(parse_roster("2,2").unwrap(), vec![BotId(2)]);
    assert!(parse_roster("").is_err());
    assert!(parse_roster("one").is_err());
  }

  /// A dump of the same world with a furnace holding plates in it.
  ///
  /// Sited clear of the fixture's iron patch, the same tile
  /// `crates/planner/tests/world_round_trip.rs` and `tests/buffers.rs` use.
  fn dumped_world_with_a_full_furnace(dir: &tempfile::TempDir) -> PathBuf {
    use factorio_bot_core::types::{
      Direction, FactorioEntity, InventoryItemWithQuality, InventoryResponse, Position,
    };
    let at = Position::new(-34.0, 40.0);
    let world = fixture_world();
    world
      .on_some_entity_created(FactorioEntity::new_stone_furnace(&at, Direction::North))
      .expect("the furnace is placed");
    world.observe_inventories(vec![InventoryResponse {
      name: "stone-furnace".into(),
      position: at,
      output_inventory: Box::new(Some(vec![InventoryItemWithQuality {
        name: "iron-plate".into(),
        quality: "normal".into(),
        count: 40,
      }])),
      fuel_inventory: Box::new(None),
      input_inventory: Box::new(None),
    }]);
    let path = dir.path().join("full.json");
    world.dump_to(&path).expect("a dump is written");
    path
  }

  /// **A dump that carries container contents plans withdrawals from them.**
  ///
  /// The other end of the seam `world.dump` closes. Until the dump asked the
  /// game what was in its buffers, `inventories` came out `[]` for every
  /// script that had not also planned, and so the entire `Withdraw` path --
  /// furnaces and chests handing their contents over -- was unreachable from
  /// this command, which is the loop planner changes are evaluated in. A
  /// defect living there would have read as absent.
  ///
  /// The negative half is the point: the *same* fixture with no reading must
  /// not take anything, or a `take` line would prove nothing about the file.
  #[test]
  fn a_dump_carrying_buffer_contents_plans_to_withdraw_them() {
    let dir = tempfile::tempdir().expect("a directory");
    let goal = ["have:iron-plate:8".to_string()];

    let full = dumped_world_with_a_full_furnace(&dir);
    let (_, _, listing) =
      plan_from_dump(&full, &goal, &[], Some("1"), true).expect("plans from a full furnace");
    assert!(
      listing
        .iter()
        .any(|line| line.contains("from the stone-furnace")),
      "the dump carried a furnace holding 40 plates and the plan smelted its own: {listing:?}"
    );

    let empty = dumped_world(&dir);
    let (_, _, bare) =
      plan_from_dump(&empty, &goal, &[], Some("1"), true).expect("plans from a bare map");
    assert!(
      !bare
        .iter()
        .any(|line| line.contains("from the stone-furnace")),
      "the bare map withdrew too, so the assertion above is not about the file: {bare:?}"
    );
  }

  /// The headline: a file in, a plan out, no game anywhere.
  #[test]
  fn a_dump_and_a_goal_produce_a_plan() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dumped_world(&dir);
    let (report, notes, _) = plan_from_dump(
      &path,
      &["have:automation-science-pack:4".to_string()],
      &[],
      Some("1,2"),
      false,
    )
    .expect("plans");
    assert!(report.actions > 10, "{} actions", report.actions);
    assert!(report.makespan > 0);
    assert_eq!(report.bots.len(), 2);
    assert!(
      notes.iter().any(|note| note.contains("no player")),
      "the fixture has no players and the report should say so: {notes:?}"
    );
  }

  /// More bots is a different plan, and it costs nothing to ask.
  ///
  /// This is the iteration loop the whole workstream exists to create: two
  /// plans over the same map, compared, without a Factorio process.
  #[test]
  fn the_same_map_plans_differently_for_a_different_roster() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dumped_world(&dir);
    let goal = ["have:automation-science-pack:10".to_string()];
    let (solo, _, _) = plan_from_dump(&path, &goal, &[], Some("1"), false).expect("plans for one");
    let (four, _, _) =
      plan_from_dump(&path, &goal, &[], Some("1,2,3,4"), false).expect("plans for four");
    assert_ne!(
      solo.makespan, four.makespan,
      "four bots planned exactly like one, which is the finding, not the test"
    );
    assert_eq!(solo.bots.len(), 1);
    assert_eq!(four.bots.len(), 4);
  }

  /// `--goal-json` reaches the shapes the shorthand deliberately cannot.
  ///
  /// Asserted by comparing against the shorthand's own `Holder::Anyone`
  /// version of the same goal rather than by looking for bot 2's name in the
  /// schedule: naming a holder welds a chain to a bot, and what that does to
  /// the resulting steps is the planner's business and has changed before.
  /// That the two plans differ at all is what proves the holder was read and
  /// not dropped on the floor.
  #[test]
  fn goal_json_can_name_a_holder_the_shorthand_cannot() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dumped_world(&dir);
    let (anyone, _, _) = plan_from_dump(
      &path,
      &["have:iron-plate:8".to_string()],
      &[],
      Some("1,2"),
      false,
    )
    .expect("plans");
    let (bot_two, _, _) = plan_from_dump(
      &path,
      &[],
      &[r#"{"Have":{"item":"iron-plate","count":8,"whose":{"Bot":2}}}"#.to_string()],
      Some("1,2"),
      false,
    )
    .expect("plans");
    assert_ne!(
      anyone.bots, bot_two.bots,
      "naming bot 2 as the holder changed nothing, so --goal-json is not \
       reaching the planner"
    );
  }

  /// `--steps` is the diagnostic this command was missing.
  ///
  /// The report says a bot idled 39,892 ticks; only a step listing says
  /// *where*, and that distinction is what the buffer-chest workstream was
  /// read off -- the reference plan's 12,240-tick gap turned out to be one
  /// wait on one lag edge, in front of which sat every raw unit the chain
  /// owner had to mine itself.
  #[test]
  fn steps_lists_every_scheduled_step_and_the_gap_before_it() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dumped_world(&dir);
    let goal = ["have:iron-plate:8".to_string()];
    let (_, _, listing) =
      plan_from_dump(&path, &goal, &[], Some("1,2"), true).expect("plans with steps");
    assert!(
      listing.iter().any(|line| line.starts_with("--- bot 1")),
      "the listing is grouped by bot: {listing:?}"
    );
    assert!(
      listing.iter().any(|line| line.contains("idle")),
      "every step names the gap that precedes it: {listing:?}"
    );

    let (_, _, quiet) =
      plan_from_dump(&path, &goal, &[], Some("1,2"), false).expect("plans without steps");
    assert!(
      quiet.is_empty(),
      "the listing is opt-in and costs nothing when it is not asked for"
    );
  }

  #[test]
  fn a_missing_file_says_which_file() {
    let err = plan_from_dump(
      std::path::Path::new("/nonexistent/world.json"),
      &["have:iron-plate:1".to_string()],
      &[],
      Some("1"),
      false,
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("/nonexistent/world.json"), "{err}");
  }

  #[test]
  fn a_file_that_is_not_a_world_says_so() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("nope.json");
    std::fs::write(&path, "{}").expect("written");
    let err = plan_from_dump(
      &path,
      &["have:iron-plate:1".to_string()],
      &[],
      Some("1"),
      false,
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("is not a world dump"), "{err}");
  }

  #[test]
  fn no_goal_at_all_is_refused_rather_than_planned_as_nothing() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dumped_world(&dir);
    let err = plan_from_dump(&path, &[], &[], Some("1"), false)
      .unwrap_err()
      .to_string();
    assert!(err.contains("nothing to plan"), "{err}");
  }

  /// A dump with no players and no `--bots` refuses instead of planning for
  /// an invented roster.
  #[test]
  fn a_playerless_dump_asks_for_a_roster() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dumped_world(&dir);
    let err = plan_from_dump(&path, &["have:iron-plate:1".to_string()], &[], None, false)
      .unwrap_err()
      .to_string();
    assert!(err.contains("--bots"), "{err}");
  }

  /// The subcommand is reachable from the assembled CLI.
  #[test]
  fn the_command_parses_off_the_root_app() {
    let matches = crate::cli::build_app()
      .try_get_matches_from([
        "factorio-bot",
        "plan",
        "-w",
        "map.json",
        "-g",
        "have:coal:4",
      ])
      .expect("parses");
    let plan = matches.subcommand_matches("plan").expect("the subcommand");
    assert_eq!(
      plan.get_one::<PathBuf>("world"),
      Some(&PathBuf::from("map.json"))
    );
    assert_eq!(
      plan
        .get_many::<String>("goal")
        .expect("a goal")
        .cloned()
        .collect::<Vec<_>>(),
      vec!["have:coal:4".to_string()]
    );
  }
}
