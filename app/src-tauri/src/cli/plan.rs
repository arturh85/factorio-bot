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
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::miette::{IntoDiagnostic, Result, miette};
use factorio_bot_core::serde_json;
use factorio_bot_core::types::Position;
use factorio_bot_planner::goal::{Goal, Holder};
use factorio_bot_planner::method::have::registry_for;
use factorio_bot_planner::{BotId, PlanReport, PlanState, pick_chain_actor, plan_best};
use std::path::PathBuf;
use std::sync::Arc;

const PLAN_AFTER_HELP: &str = "\
Goal specs (--goal, repeatable; every one is planned together):

  have:<item>:<count>        end up holding that many, across the whole roster
  produced:<item>:<count>    cause that many to come into existence
  producing:<item>:<rate>    stand up machinery yielding that many per minute
  researched:<technology>    finish that research
  charted:<x>:<y>:<radius>   walk bots out until that disc has been looked at

  have:iron-plate:50   produced:stone-furnace:2   researched:automation
  charted:0:0:256

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
  }

  fn build_callback(&self) -> SubcommandCallback {
    |args, context| Box::pin(std::future::ready(run(args, context)))
  }
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
      _ => Err(miette!(
        "`{raw}` in `{spec}` is not a finite coordinate"
      )),
    }
  };
  match parts.as_slice() {
    ["researched", tech] if !tech.is_empty() => Ok(Goal::Researched((*tech).to_owned())),
    ["have", item, n] if !item.is_empty() => Ok(Goal::Have {
      item: (*item).to_owned(),
      count: count(n)?,
      whose: Holder::Anyone,
    }),
    ["produced", item, n] if !item.is_empty() => Ok(Goal::Produced {
      item: (*item).to_owned(),
      count: count(n)?,
      whose: Holder::Anyone,
      unlocks: None,
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
      "`{spec}` is not a goal. Expected have:<item>:<count>, \
       produced:<item>:<count>, producing:<item>:<per-minute>, \
       sustain:<item>:<per-minute>:<window-ticks>, \
       gathered:<resource-entity>, \
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
pub(crate) fn roster_from(world: &FactorioWorld) -> Vec<BotId> {
  let mut bots: Vec<BotId> = world
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
pub(crate) fn load_world(world_path: &std::path::Path) -> Result<Arc<FactorioWorld>> {
  let raw = std::fs::read_to_string(world_path)
    .map_err(|err| miette!("could not read {}: {err}", world_path.display()))?;
  let world: FactorioWorld = serde_json::from_str(&raw)
    .map_err(|err| miette!("{} is not a world dump: {err}", world_path.display()))?;
  Ok(Arc::new(world))
}

/// Reads the dump, plans, and returns the report plus the lines to print
/// ahead of it.
///
/// Split out of [`run`] so it is testable without a `Context` — everything
/// this command does is a pure function of a file and some arguments, and a
/// test that had to build a `Context` would be testing the CLI harness.
fn plan_from_dump(
  world_path: &std::path::Path,
  specs: &[String],
  goal_json: &[String],
  roster: Option<&str>,
  steps: bool,
) -> Result<(PlanReport, Vec<String>, Vec<String>)> {
  let world = load_world(world_path)?;

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

  let bots = if let Some(raw) = roster {
    parse_roster(raw)?
  } else {
    let found = roster_from(&world);
    if found.is_empty() {
      return Err(miette!(
        "the dump has no players, so there is no default roster -- pass \
         --bots, e.g. --bots 1,2,3,4"
      ));
    }
    found
  };

  let state = PlanState::from_world(world, &bots);
  // Not an error. A dump can legitimately be planned for a roster it has no
  // players for -- comparing one map's plan across roster sizes is exactly
  // what this command is for -- but a bot the world knows nothing about is
  // seeded with an empty inventory and guessed reach distances, and a report
  // that did not say so would read as a measurement of the roster rather than
  // of four defaults.
  let mut notes = Vec::new();
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

  let chain_actor = pick_chain_actor(&state, &bots)
    .ok_or_else(|| miette!("no bots to plan for; a roster needs at least one"))?;
  // `plan_best`, not `expand` + `schedule`: the drain policy is settled by
  // reading finished schedules, so the CLI has to make the same choice a run
  // makes or the offline loop stops predicting it.
  let (net, scheduled) = plan_best(&goals, &state, &registry_for(&bots), chain_actor, &bots)
    .map_err(|err| miette!("the goal did not expand: {err}"))?;

  let listing = if steps {
    step_lines(&scheduled, &bots)
  } else {
    Vec::new()
  };
  Ok((PlanReport::of(&net, &scheduled, &bots), notes, listing))
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

  let (report, notes, listing) = plan_from_dump(
    world_path,
    &specs,
    &goal_json,
    roster,
    args.get_flag("steps"),
  )?;

  for note in &notes {
    eprintln!("{note}");
  }
  for line in &listing {
    println!("{line}");
  }
  if args.get_flag("json") {
    println!(
      "{}",
      serde_json::to_string_pretty(&report).into_diagnostic()?
    );
  } else {
    for line in report.lines() {
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
        whose: Holder::Anyone
      }
    );
    assert_eq!(
      parse_goal("produced:stone-furnace:2").unwrap(),
      Goal::Produced {
        item: "stone-furnace".into(),
        count: 2,
        whose: Holder::Anyone,
        unlocks: None
      }
    );
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
    assert!(parse_goal("sustain::15:7200").is_err(), "an item is required");
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
