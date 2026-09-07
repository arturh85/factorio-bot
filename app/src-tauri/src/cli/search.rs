//! `search`: rank candidate layouts by the flow they would sustain, offline.
//!
//! The CLI half of `factorio_bot_planner::search`, which carries the argument
//! for what is optimised and what the ranking cannot see. This file only
//! reads a dump, builds the candidate set, times each evaluation, and prints
//! the table -- every number in it comes from the planner crate, and the
//! wall-clock cost per candidate is measured here because that crate is
//! deliberately clock-free.
//!
//! # A ranking has to be able to come out differently
//!
//! Three controls ride along by default, and the table says which rows they
//! are:
//!
//! - every generated block gets an **unfuelled twin** -- the same geometry
//!   with its coal branch deleted. In the game that block stops; in the model
//!   it scores the same. The tie is the point, and the table prints it rather
//!   than hiding it.
//! - an **off-ore** siting of the best generated block, which must read as
//!   `unreached` with a rate of zero.
//! - the `drills x furnaces` sweep itself contains the starved and the
//!   over-provisioned corners (one drill into many furnaces, many drills into
//!   one), which the balance must place below the balanced designs.
//!
//! `--measured NAME=RATE` puts a live number beside a candidate and prints
//! `model / measured`; `--emit NAME` prints a candidate's blueprint string so
//! a script can stand it up and measure it.

use crate::cli::plan::{load_world, parse_roster, roster_from};
use crate::cli::{Subcommand, SubcommandCallback};
use crate::context::Context;
use clap::{Arg, ArgAction, ArgMatches, Command, value_parser};
use factorio_bot_core::factorio::world::FactorioSurface;
use factorio_bot_core::miette::{Result, miette};
use factorio_bot_core::serde_json;
use factorio_bot_core::types::Position;
use factorio_bot_planner::goal::{Goal, Site};
use factorio_bot_planner::method::have::registry_for;
use factorio_bot_planner::search::{
  Fitness, Layout, OrePatch, Siting, evaluate, ore_to_plate, rank, site,
};
use factorio_bot_planner::{PlanReport, PlanState, pick_chain_actor, plan_best};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

const SEARCH_AFTER_HELP: &str = "\
Candidates come from two places:

  --drills A-B --furnaces C-D   every generated block in that grid, in the
                                geometry of the measured OreToPlateThree
  --blueprint NAME=TEXT         a blueprint string, by name (repeatable)
  --rcontest <lua file>         every `Name = \"0eN...\"` fixture in the file

Every candidate is sited on the iron patch nearest the origin so that as many
of its drills as possible stand on ore, then scored by the flow graph's
whole-base balance. A candidate the flow walk never reaches -- one with no
drill in it -- is reported as `unreached`, not ranked as bad: the model has no
number for it, and zero is not one.

  --measured NAME=RATE    a live plates/min for a candidate; the table then
                          prints model / measured beside it
  --emit NAME             print that candidate's blueprint string and exit
  --plan                  also plan `goal.built` at the chosen anchor and
                          report the plan's actions and makespan (seconds each)
  --json                  the whole table as JSON

The ranking is by sustained iron plates per minute, then by the fewest iron
plates spent to build the block. Payback is that cost over that rate.";

impl Subcommand for ThisCommand {
  fn name(&self) -> &'static str {
    "search"
  }
  fn build_command(&self) -> Command {
    Command::new("search")
      .about("Rank candidate layouts by the flow they would sustain, without Factorio")
      .after_help(SEARCH_AFTER_HELP)
      .arg(
        Arg::new("world")
          .long("world")
          .short('w')
          .value_name("path")
          .required(true)
          .value_parser(value_parser!(PathBuf))
          .help("world dump to site and score against, as written by world.dump()"),
      )
      .arg(
        Arg::new("drills")
          .long("drills")
          .value_name("A-B")
          .default_value("1-4")
          .help("range of burner drills for the generated family"),
      )
      .arg(
        Arg::new("furnaces")
          .long("furnaces")
          .value_name("C-D")
          .default_value("1-4")
          .help("range of stone furnaces for the generated family"),
      )
      .arg(
        Arg::new("blueprint")
          .long("blueprint")
          .value_name("NAME=TEXT")
          .action(ArgAction::Append)
          .help("a named blueprint string to score"),
      )
      .arg(
        Arg::new("rcontest")
          .long("rcontest")
          .value_name("path")
          .value_parser(value_parser!(PathBuf))
          .help("score every fixture in this Lua file (Name = \"0eN...\")"),
      )
      .arg(
        Arg::new("resource")
          .long("resource")
          .value_name("name")
          .default_value("iron-ore")
          .help("the ore patch to site on"),
      )
      .arg(
        Arg::new("measured")
          .long("measured")
          .value_name("NAME=RATE")
          .action(ArgAction::Append)
          .help("a live plates/min for a candidate, printed as model / measured"),
      )
      .arg(
        Arg::new("controls")
          .long("no-controls")
          .action(ArgAction::SetFalse)
          .help("omit the unfuelled twins and the off-ore control"),
      )
      .arg(
        Arg::new("plan")
          .long("plan")
          .action(ArgAction::SetTrue)
          .help("also plan goal.built at the chosen anchor for each candidate"),
      )
      .arg(
        Arg::new("bots")
          .long("bots")
          .short('b')
          .value_name("ids")
          .help("roster for --plan [default: the players in the dump]"),
      )
      .arg(
        Arg::new("emit")
          .long("emit")
          .value_name("NAME")
          .help("print this candidate's blueprint string and exit"),
      )
      .arg(
        Arg::new("json")
          .long("json")
          .action(ArgAction::SetTrue)
          .help("print the table as JSON"),
      )
  }

  fn build_callback(&self) -> SubcommandCallback {
    |args, context| Box::pin(std::future::ready(run(args, context)))
  }
}

/// `A-B` or `A`, inclusive.
fn parse_range(raw: &str) -> Result<(u32, u32)> {
  let bad = || miette!("`{raw}` is not a range like 2-5");
  let (a, b) = if let Some((a, b)) = raw.split_once('-') {
    let a: u32 = a.trim().parse().map_err(|_| bad())?;
    let b: u32 = b.trim().parse().map_err(|_| bad())?;
    (a, b)
  } else {
    let a: u32 = raw.trim().parse().map_err(|_| bad())?;
    (a, a)
  };
  if a == 0 || b < a {
    return Err(bad());
  }
  Ok((a, b))
}

/// Every `Name = "0eN..."` in a Lua fixture file.
pub(crate) fn fixtures_in(text: &str) -> Vec<(String, String)> {
  let re = factorio_bot_core::regex::Regex::new(
    r#"(?m)^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*"(0eN[A-Za-z0-9+/=]+)""#,
  )
  .expect("a literal regex compiles");
  re.captures_iter(text)
    .map(|c| (c[1].to_owned(), c[2].to_owned()))
    .collect()
}

/// `NAME=VALUE` pairs from a repeatable argument.
fn pairs(args: &ArgMatches, key: &str) -> Result<Vec<(String, String)>> {
  let mut out = vec![];
  for raw in args.get_many::<String>(key).into_iter().flatten() {
    let (name, value) = raw
      .split_once('=')
      .ok_or_else(|| miette!("`{raw}` is not NAME=VALUE"))?;
    out.push((name.trim().to_owned(), value.trim().to_owned()));
  }
  Ok(out)
}

#[derive(Serialize)]
struct Row {
  name: String,
  note: String,
  control: Option<String>,
  anchor: Option<String>,
  ore_under_drills: Option<String>,
  reached: bool,
  unscoreable: Option<String>,
  ore_per_min: f64,
  plates_per_min: f64,
  nameplate_plates_per_min: f64,
  entities: usize,
  iron_cost: f64,
  stone_cost: f64,
  payback_min: Option<f64>,
  eval_ms: f64,
  measured: Option<f64>,
  model_over_measured: Option<f64>,
  plan: Option<PlanRow>,
}

#[derive(Serialize)]
struct PlanRow {
  actions: Option<usize>,
  makespan: Option<u32>,
  refused: Option<String>,
  plan_ms: f64,
}

/// One candidate as the CLI carries it: the layout, and whether it is a
/// control (and why).
struct Candidate {
  layout: Layout,
  control: Option<String>,
}

impl Row {
  fn scored(
    fit: &Fitness,
    candidate: &Candidate,
    unscoreable: Option<String>,
    eval_ms: f64,
    measured: Option<f64>,
  ) -> Row {
    let plates = fit.plates_per_min();
    Row {
      name: fit.name.clone(),
      note: candidate.layout.note.clone(),
      control: candidate.control.clone(),
      anchor: Some(format!(
        "({}, {})",
        fit.siting.anchor.x(),
        fit.siting.anchor.y()
      )),
      ore_under_drills: Some(format!("{}/{}", fit.siting.covered, fit.siting.possible)),
      reached: fit.reached,
      unscoreable,
      ore_per_min: fit
        .sustained_per_min
        .get("iron-ore")
        .copied()
        .unwrap_or_default(),
      plates_per_min: plates,
      nameplate_plates_per_min: fit
        .nameplate_per_min
        .get("iron-plate")
        .copied()
        .unwrap_or_default(),
      entities: fit.entities,
      iron_cost: fit.iron_cost(),
      stone_cost: fit.cost.get("stone").copied().unwrap_or_default(),
      payback_min: fit.payback_minutes(),
      eval_ms,
      measured,
      model_over_measured: measured.and_then(|m| (m > 0.).then(|| plates / m)),
      plan: None,
    }
  }

  /// A candidate the flow walk has no root in.
  fn unsited(candidate: &Candidate, unscoreable: Option<String>, measured: Option<f64>) -> Row {
    Row {
      name: candidate.layout.name.clone(),
      note: candidate.layout.note.clone(),
      control: candidate.control.clone(),
      anchor: None,
      ore_under_drills: None,
      reached: false,
      unscoreable: Some(match unscoreable {
        Some(multi) => {
          format!("{multi}; and no drill in the layout, so the flow walk has no root here")
        }
        None => "no drill in the layout: the flow walk has no root here".to_owned(),
      }),
      ore_per_min: 0.,
      plates_per_min: 0.,
      nameplate_plates_per_min: 0.,
      entities: candidate.layout.entities.len(),
      iron_cost: 0.,
      stone_cost: 0.,
      payback_min: None,
      eval_ms: 0.,
      measured,
      model_over_measured: None,
      plan: None,
    }
  }
}

/// The candidate set, in a stable order: the generated grid (each block
/// followed by its unfuelled twin when controls are on), then the named
/// blueprints, then the fixtures of `--rcontest`.
fn candidates(args: &ArgMatches) -> Result<Vec<Candidate>> {
  let (d_lo, d_hi) = parse_range(args.get_one::<String>("drills").expect("default"))?;
  let (f_lo, f_hi) = parse_range(args.get_one::<String>("furnaces").expect("default"))?;
  let controls = args.get_flag("controls");
  let mut out: Vec<Candidate> = vec![];
  for d in d_lo..=d_hi {
    for f in f_lo..=f_hi {
      out.push(Candidate {
        layout: ore_to_plate(d, f, true),
        control: None,
      });
      if controls {
        out.push(Candidate {
          layout: ore_to_plate(d, f, false),
          control: Some("unfuelled twin: game stops, model cannot see fuel".to_owned()),
        });
      }
    }
  }
  for (name, text) in pairs(args, "blueprint")? {
    out.push(Candidate {
      layout: Layout::from_blueprint(&name, &text).map_err(|e| miette!("{name}: {e}"))?,
      control: None,
    });
  }
  if let Some(path) = args.get_one::<PathBuf>("rcontest") {
    let text = std::fs::read_to_string(path)
      .map_err(|err| miette!("could not read {}: {err}", path.display()))?;
    let found = fixtures_in(&text);
    if found.is_empty() {
      return Err(miette!(
        "no `Name = \"0eN...\"` fixtures in {}",
        path.display()
      ));
    }
    for (name, bp) in found {
      match Layout::from_blueprint(&name, &bp) {
        Ok(layout) => out.push(Candidate {
          layout,
          control: None,
        }),
        Err(err) => eprintln!("skipping fixture {name}: {err}"),
      }
    }
  }
  Ok(out)
}

/// Everything one run of the command produces before it is printed.
struct Outcome {
  ranked: Vec<Row>,
  unreached: Vec<Row>,
  load_ms: f64,
  patch_tiles: usize,
}

fn score_all(
  args: &ArgMatches,
  world: &Arc<FactorioSurface>,
  candidates: &[Candidate],
  measured: &BTreeMap<String, f64>,
  load_ms: f64,
) -> Result<Outcome> {
  let resource = args.get_one::<String>("resource").expect("default");
  let controls = args.get_flag("controls");
  let plan = args.get_flag("plan");
  let prototypes = world.globals.entity_prototypes.clone();
  let recipes = world.globals.recipes.clone();
  let Some(ore) = OrePatch::nearest(&world.entity_graph, resource) else {
    return Err(miette!(
      "no `{resource}` patch is charted in the dump -- nothing to site on"
    ));
  };

  let mut rows: Vec<Row> = vec![];
  let mut scored: Vec<Fitness> = vec![];
  let mut unreached: Vec<Row> = vec![];
  // The best generated, un-controlled, scoreable block so far, for the
  // off-ore control: its layout, siting, and rate.
  let mut best: Option<(Layout, Siting, f64)> = None;
  for candidate in candidates {
    let layout = &candidate.layout;
    let unscoreable = {
      let machines = layout.multi_output_machines(&prototypes, &recipes);
      (!machines.is_empty()).then(|| {
        let named: Vec<String> = machines.iter().map(ToString::to_string).collect();
        format!(
          "multi-output machine(s) {} would be scored per product; the game stops the whole machine",
          named.join("; ")
        )
      })
    };
    let live = measured.get(&layout.name).copied();
    let Some(siting) = site(layout, &ore, &prototypes) else {
      unreached.push(Row::unsited(candidate, unscoreable, live));
      continue;
    };
    let t = Instant::now();
    let fit = evaluate(layout, &siting, &ore, &prototypes, &recipes)
      .map_err(|e| miette!("{}: {e}", layout.name))?;
    let eval_ms = t.elapsed().as_secs_f64() * 1000.;
    let mut row = Row::scored(&fit, candidate, unscoreable.clone(), eval_ms, live);
    if plan {
      row.plan = Some(plan_row(world, layout, &siting, args)?);
    }
    let eligible = fit.reached
      && unscoreable.is_none()
      && candidate.control.is_none()
      && layout.name.starts_with("gen-");
    if eligible {
      let rate = fit.plates_per_min();
      let better = best.as_ref().is_none_or(|(_, s, prev)| {
        rate.total_cmp(prev).is_gt() || (rate.total_cmp(prev).is_eq() && s.covered < siting.covered)
      });
      if better {
        best = Some((layout.clone(), siting.clone(), rate));
      }
    }
    if fit.reached {
      scored.push(fit);
      rows.push(row);
    } else {
      unreached.push(row);
    }
  }

  // The off-ore control: the best generated block, anchored where no ore is.
  if controls && let Some((layout, siting, _)) = &best {
    let off = Siting {
      anchor: Position::new(siting.anchor.x() + 400., siting.anchor.y() + 400.),
      covered: 0,
      possible: siting.possible,
    };
    let candidate = Candidate {
      layout: layout.clone(),
      control: Some("off-ore siting of the best generated block: must read unreached".to_owned()),
    };
    let t = Instant::now();
    let mut fit =
      evaluate(layout, &off, &ore, &prototypes, &recipes).map_err(|e| miette!("{e}"))?;
    let eval_ms = t.elapsed().as_secs_f64() * 1000.;
    fit.name = format!("{}-off-ore", layout.name);
    let row = Row::scored(&fit, &candidate, None, eval_ms, None);
    if fit.reached {
      rows.push(row);
      scored.push(fit);
    } else {
      unreached.push(row);
    }
  }

  // Rank, and reorder the rows to match.
  let mut ordered: Vec<Row> = vec![];
  for fit in rank(scored) {
    if let Some(i) = rows.iter().position(|r| r.name == fit.name) {
      ordered.push(rows.remove(i));
    }
  }
  ordered.extend(rows);
  Ok(Outcome {
    ranked: ordered,
    unreached,
    load_ms,
    patch_tiles: ore.tiles.len(),
  })
}

fn run(args: &ArgMatches, _context: &mut Context) -> Result<()> {
  let world_path = args.get_one::<PathBuf>("world").expect("required");
  let resource = args.get_one::<String>("resource").expect("default");
  let json = args.get_flag("json");
  let plan = args.get_flag("plan");

  let mut measured: BTreeMap<String, f64> = BTreeMap::new();
  for (name, rate) in pairs(args, "measured")? {
    let rate: f64 = rate
      .parse()
      .map_err(|_| miette!("`{rate}` for `{name}` is not a rate"))?;
    measured.insert(name, rate);
  }

  let candidates = candidates(args)?;
  if let Some(name) = args.get_one::<String>("emit") {
    let Some(candidate) = candidates.iter().find(|c| &c.layout.name == name) else {
      return Err(miette!("no candidate named `{name}`"));
    };
    println!("{}", candidate.layout.blueprint_text());
    return Ok(());
  }

  let started = Instant::now();
  let world = load_world(world_path)?;
  let load_ms = started.elapsed().as_secs_f64() * 1000.;
  world
    .entity_graph
    .connect()
    .map_err(|err| miette!("the dump's entity graph could not be rewired: {err}"))?;
  let outcome = score_all(args, &world, &candidates, &measured, load_ms)?;

  if json {
    #[derive(Serialize)]
    struct Out<'a> {
      world: String,
      resource: &'a str,
      patch_tiles: usize,
      load_ms: f64,
      ranked: &'a [Row],
      unreached: &'a [Row],
    }
    println!(
      "{}",
      serde_json::to_string_pretty(&Out {
        world: world_path.display().to_string(),
        resource,
        patch_tiles: outcome.patch_tiles,
        load_ms: outcome.load_ms,
        ranked: &outcome.ranked,
        unreached: &outcome.unreached,
      })
      .expect("the report serialises")
    );
    return Ok(());
  }

  println!(
    "world {} loaded in {:.0} ms; siting on the {} patch nearest the origin ({} tiles)",
    world_path.display(),
    outcome.load_ms,
    resource,
    outcome.patch_tiles
  );
  println!();
  println!("{}", header());
  for r in &outcome.ranked {
    print_row(r, plan);
  }
  if !outcome.unreached.is_empty() {
    println!();
    println!("unreached -- the model has NO number for these, and zero is not one:");
    for r in &outcome.unreached {
      println!(
        "{:<16} {:>9} {:>7} {:>9} {:>10} {:>10} {:>6}  {}",
        r.name,
        r.anchor.as_deref().unwrap_or("-"),
        r.ore_under_drills.as_deref().unwrap_or("-"),
        "-",
        "unreached",
        "-",
        r.entities,
        r.unscoreable
          .as_deref()
          .or(r.control.as_deref())
          .unwrap_or(&r.note)
      );
    }
  }
  Ok(())
}

/// The column headings, padded to the widths `print_row` uses.
fn header() -> String {
  let cols: [(&str, usize, bool); 13] = [
    ("candidate", 16, false),
    ("anchor", 9, true),
    ("ore", 7, true),
    ("ore/min", 9, true),
    ("plates/min", 10, true),
    ("nameplate", 10, true),
    ("ents", 6, true),
    ("iron", 7, true),
    ("payback", 8, true),
    ("eval ms", 8, true),
    ("measured", 9, true),
    ("m/meas", 7, true),
    ("note", 0, false),
  ];
  cols
    .iter()
    .map(|(label, width, right)| {
      if *right {
        format!("{label:>width$}")
      } else {
        format!("{label:<width$}")
      }
    })
    .collect::<Vec<_>>()
    .join(" ")
}

fn print_row(r: &Row, plan: bool) {
  let anchor = r.anchor.as_deref().unwrap_or("-").replace(' ', "");
  let payback = r
    .payback_min
    .map_or_else(|| "-".to_owned(), |m| format!("{m:.1}m"));
  let measured = r
    .measured
    .map_or_else(|| "-".to_owned(), |m| format!("{m:.1}"));
  let ratio = r
    .model_over_measured
    .map_or_else(|| "-".to_owned(), |m| format!("{m:.3}"));
  let mut note = String::new();
  if let Some(c) = &r.control {
    note.push_str("CONTROL: ");
    note.push_str(c);
  } else if let Some(u) = &r.unscoreable {
    note.push_str("UNSCOREABLE: ");
    note.push_str(u);
  } else {
    note.push_str(&r.note);
  }
  if let Some(p) = &r.plan {
    match (&p.refused, p.actions, p.makespan) {
      (Some(why), _, _) => {
        let _ = write!(note, " | plan refused: {why} ({:.0} ms)", p.plan_ms);
      }
      (None, Some(a), Some(m)) => {
        let _ = write!(note, " | plan {a} actions, {m} ticks ({:.0} ms)", p.plan_ms);
      }
      _ => {}
    }
  } else if plan {
    note.push_str(" | plan: not attempted");
  }
  println!(
    "{:<16} {:>9} {:>7} {:>9.1} {:>10.1} {:>10.1} {:>6} {:>7.0} {:>8} {:>8.1} {:>9} {:>7}  {}",
    r.name,
    anchor,
    r.ore_under_drills.as_deref().unwrap_or("-"),
    r.ore_per_min,
    r.plates_per_min,
    r.nameplate_plates_per_min,
    r.entities,
    r.iron_cost,
    payback,
    r.eval_ms,
    measured,
    ratio,
    note
  );
}

fn plan_row(
  world: &Arc<FactorioSurface>,
  layout: &Layout,
  siting: &Siting,
  args: &ArgMatches,
) -> Result<PlanRow> {
  let bots = if let Some(raw) = args.get_one::<String>("bots") {
    parse_roster(raw)?
  } else {
    let found = roster_from(world);
    if found.is_empty() {
      return Err(miette!("the dump has no players; pass --bots for --plan"));
    }
    found
  };
  let t = Instant::now();
  let state = PlanState::from_world(world.clone(), &bots);
  let chain_actor =
    pick_chain_actor(&state, &bots).ok_or_else(|| miette!("no bots to plan for"))?;
  let goal = Goal::Built {
    blueprint: layout.blueprint_text(),
    site: Site::At(siting.anchor.clone()),
  };
  let outcome = plan_best(&[goal], &state, &registry_for(&bots), chain_actor, &bots);
  let plan_ms = t.elapsed().as_secs_f64() * 1000.;
  Ok(match outcome {
    Ok((net, scheduled)) => {
      let report = PlanReport::of(&net, &scheduled, &bots, &state);
      PlanRow {
        actions: Some(report.actions),
        makespan: Some(report.makespan),
        refused: None,
        plan_ms,
      }
    }
    Err(err) => PlanRow {
      actions: None,
      makespan: None,
      refused: Some(err.to_string()),
      plan_ms,
    },
  })
}

pub(crate) struct ThisCommand {}

pub fn build() -> Box<dyn Subcommand> {
  Box::new(ThisCommand {})
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn ranges_parse_inclusively_and_refuse_zero() {
    assert_eq!(parse_range("2-5").unwrap(), (2, 5));
    assert_eq!(parse_range("3").unwrap(), (3, 3));
    assert!(parse_range("0-2").is_err());
    assert!(parse_range("5-2").is_err());
  }

  #[test]
  fn fixtures_are_found_by_name_in_a_lua_table() {
    let lua = "local BP = {\n    OreToPlate = \"0eNabc+/=\",\n    Other = \"0eNxyz\",\n}\nlocal x = \"0eNnot\"\n";
    let found = fixtures_in(lua);
    assert_eq!(
      found,
      vec![
        ("OreToPlate".to_owned(), "0eNabc+/=".to_owned()),
        ("Other".to_owned(), "0eNxyz".to_owned())
      ]
    );
  }
}
