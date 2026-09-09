//! A replan against the world `run-1788946451-86723` left standing: the
//! source with no free side, and the run that already leaves it.
//!
//! The run (seed 31337, four headless bots at 10x, honest, release binary
//! from `5323e71a`) ended `stuck` at tick 64,053 on
//!
//! ```text
//! a cell already makes copper-plate at [29.5, -46.5] and nothing can carry
//! it to the supply chest at [30.5, -55.5]: from the iron-chest at
//! [29.5, -46.5]: no belt route, blocked by 4 tile(s): [28.5, -46.5]
//! [29.5, -47.5] [29.5, -45.5] [30.5, -46.5]; from the stone-furnace at
//! [27, -46]: no belt route, blocked by 8 tile(s): ...
//! ```
//!
//! Every one of those four tiles is honestly taken -- three by the cell's
//! own coal arms, `[30.5, -46.5]` by the arm carrying the cell's plates onto
//! the run plan 1 laid to its first supply chest -- and two arms cannot
//! share a tile. The owner's ruling (2026-09-09, "1 sounds good") was to
//! **splice a splitter into the run the source already has**, and
//! `method::connect::tap_standing_run` is that: the perimeter refusal
//! stands, and a boxed-in source whose run leaves through its one arm is
//! tapped rather than refused.
//!
//! Reproduced offline on `7665ffde` byte for byte with
//!
//! ```text
//! factorio-bot plan --world workspace/scripts/map.json \
//!     --standing-from-run workspace/runs/run-1788946451-86723 --at-tick 64053 \
//!     --all --goal sustain:copper-plate:15:36000 \
//!     --goal producing:automation-science-pack:6
//! ```
//!
//! and the fixture is what that command writes with `--save-standing
//! crates/planner/tests/fixtures/run-1788946451-86723-tick64053.json`.
//!
//! Like `replan_finishes_its_cell`, it FAILS rather than skips when
//! `workspace/scripts/map.json` is absent, unless `CI` is set.

use factorio_bot_core::factorio::world::FactorioSurface;
use factorio_bot_core::record::standing::StandingSnapshot;
use factorio_bot_core::types::{Pos, Position};
use factorio_bot_planner::standing::world_with;
use factorio_bot_planner::{
    ActionKind, ActionNetwork, BotId, Goal, PlanState, expand, pick_chain_actor, registry_for,
};
use std::path::PathBuf;
use std::sync::Arc;

/// The seed-31337 t=0 dump, or a loud failure saying how to get it.
fn dump() -> Option<Arc<FactorioSurface>> {
    let path: PathBuf =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../workspace/scripts/map.json");
    let Ok(path) = path.canonicalize() else {
        let message = format!(
            "workspace/scripts/map.json is not present at {}, so the replan against run \
             86723's world did NOT run. In a worktree: `ln -s <main checkout>/workspace/\
             scripts/map.json <worktree>/workspace/scripts/map.json`.",
            path.display()
        );
        if std::env::var_os("CI").is_some() {
            eprintln!("SKIPPED under CI: {message}");
            return None;
        }
        panic!("{message}");
    };
    let raw = std::fs::read_to_string(&path).expect("the dump is readable");
    Some(Arc::new(
        factorio_bot_core::serde_json::from_str(&raw).expect("map.json is a world dump"),
    ))
}

/// The run's record at the tick it replanned and refused.
fn fixture() -> StandingSnapshot {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/run-1788946451-86723-tick64053.json");
    StandingSnapshot::read_from(&path).expect("the fixture is in the tree and parses")
}

/// The plate chest with no free side, as the refusal names it.
const PLATE_CHEST: (f64, f64) = (29.5, -46.5);

fn standing_world(snapshot: &StandingSnapshot) -> Option<(PlanState, Vec<BotId>)> {
    let base = dump()?;
    let (world, standing) = world_with(&base, snapshot);
    assert_eq!(
        standing.placed,
        snapshot.entities.len(),
        "every entity of the keyframe stands, or this is a test of a different world: {standing:?}"
    );
    let bots: Vec<BotId> = snapshot.bots.iter().map(|b| BotId(b.id)).collect();
    Some((PlanState::from_world(world, &bots), bots))
}

/// The run's goal, as `scripts/continuous_supply.lua` states it.
fn goal() -> Goal {
    Goal::All(vec![
        Goal::Sustain {
            item: "copper-plate".into(),
            per_minute: 15,
            window_ticks: 36_000,
        },
        Goal::Producing {
            item: "automation-science-pack".into(),
            per_minute: 6,
        },
    ])
}

/// Every `Place` of `name`, as `(position, direction)`, in a fixed order.
fn placed(net: &ActionNetwork, name: &str) -> Vec<(Position, u8)> {
    let mut out: Vec<(Position, u8)> = net
        .actions()
        .filter_map(|a| match &a.kind {
            ActionKind::Place { entity } if entity.name == name => {
                Some((entity.position.clone(), entity.direction))
            }
            _ => None,
        })
        .collect();
    out.sort_by(|a, b| a.0.x.total_cmp(&b.0.x).then(a.0.y.total_cmp(&b.0.y)));
    out
}

/// Every `Chop`, as `(position, entity)`.
fn chopped(net: &ActionNetwork) -> Vec<(Position, String)> {
    net.actions()
        .filter_map(|a| match &a.kind {
            ActionKind::Chop { pos, entity, .. } => Some((pos.clone(), entity.clone())),
            _ => None,
        })
        .collect()
}

/// The replan that refused now plans, by tapping the run: exactly one
/// splitter, over exactly one chopped belt of the standing run, facing the
/// way that belt faced -- and no second arm on the plate chest, because
/// there is no tile for one.
#[test]
fn the_replan_of_run_1788946451_86723_taps_the_standing_run() {
    let snapshot = fixture();
    let Some((state, bots)) = standing_world(&snapshot) else {
        return;
    };
    let chain_actor = pick_chain_actor(&state, &bots).expect("the roster has a chain actor");
    let net = expand(&[goal()], &state, &registry_for(&bots), chain_actor)
        .unwrap_or_else(|refusal| panic!("the replan of run 86723 refused again: {refusal}"));

    let splitters = placed(&net, "splitter");
    assert_eq!(splitters.len(), 1, "exactly one splitter: {splitters:?}");
    // Belts only: the same plan may chop a rock for a hand-smelt furnace's
    // stone (it does since `produce::cell_ledger` stopped offering a furnace
    // an arm empties to a hand -- the copper the tail needs is hand-smelted
    // now rather than read out of a slot the offtake keeps at zero), and a
    // rock coming up says nothing about the tap.
    let chops: Vec<(Position, String)> = chopped(&net)
        .into_iter()
        .filter(|(_, name)| name == "transport-belt")
        .collect();
    assert_eq!(chops.len(), 1, "exactly one belt comes up: {chops:?}");
    let (belt_at, belt_name) = &chops[0];
    assert_eq!(belt_name, "transport-belt");

    // The chopped tile is a belt of the run that stood, and the splitter
    // faces the way that belt faced.
    let standing_belt = snapshot
        .entities
        .iter()
        .find(|e| e.name == "transport-belt" && Pos::from(&e.position) == Pos::from(belt_at))
        .unwrap_or_else(|| panic!("{belt_at} is not a standing belt of the record"));
    let (splitter_at, splitter_facing) = &splitters[0];
    assert_eq!(
        *splitter_facing, standing_belt.direction,
        "the splitter faces the run's way"
    );
    assert_eq!(
        (splitter_at.x() - belt_at.x()).abs() + (splitter_at.y() - belt_at.y()).abs(),
        0.5,
        "the splitter straddles the chopped tile and the one beside it: splitter {splitter_at}, \
         belt {belt_at}"
    );

    // No arm is placed on the plate chest's perimeter: it has no tile for
    // one, which is the whole reason the run was tapped.
    let chest = Position::new(PLATE_CHEST.0, PLATE_CHEST.1);
    let arms_on_chest: Vec<Position> = placed(&net, "inserter")
        .into_iter()
        .chain(placed(&net, "burner-inserter"))
        .map(|(at, _)| at)
        .filter(|at| (at.x() - chest.x()).abs() + (at.y() - chest.y()).abs() < 1.5)
        .collect();
    assert!(
        arms_on_chest.is_empty(),
        "a second arm was put on the plate chest: {arms_on_chest:?}"
    );
    eprintln!(
        "the replan plans: {} actions; splitter at {splitter_at} facing {splitter_facing}, over \
         the belt at {belt_at}",
        net.len()
    );
}
