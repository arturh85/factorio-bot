//! The science cell must plan AGAIN from the world its own first plan built.
//!
//! # Why this test exists
//!
//! Every baseline in `docs/superpowers/notes/BASELINES.md` is an offline plan
//! from the t=0 dump, and a t=0 dump has no factory in it. On 2026-09-09
//! `9f549b1c` moved all eight of them correctly, was measured at 885 / 52,891
//! planning cleanly through to both assembling machines, was reviewed and
//! merged -- and refused in `run-1788923927-04849` with the byte-identical
//! blocker it had been written to remove:
//!
//! ```text
//! a cell already makes copper-plate at [29.5,-46.5] and nothing can carry it
//! to the supply chest at [30.5,-24.5]
//! ```
//!
//! *"A cell ALREADY MAKES copper-plate"* is a sentence only a replan can say.
//! The supervisor plans, executes, and replans whenever a batch truncates, and
//! the second expansion meets the first plan's cell standing as map facts.
//! Offline, from t=0, there is no cell, so the fixed path was never reached.
//! **The failure mode is not a wrong number; it is eight right numbers and a
//! broken run** -- and until this file, nothing in the tree planned against a
//! world with a factory standing in it.
//!
//! # What it does
//!
//! `continuous_supply.lua`'s goal -- `sustain copper-plate 15/min over 36,000
//! ticks` and `producing automation-science-pack 6/min` -- is planned on the
//! seed-31337 t=0 dump for four bots, exactly as the run plans it. The plan is
//! then applied to the world through `factorio_bot_planner::standing::
//! world_after` (placements, recipes, choppings and mined ore as map facts on a
//! fresh surface, the way the mod's writeout would leave them), and the same
//! goal is planned again on a **fresh** `PlanState` over that world -- which
//! is the supervisor's own shape: `plan_rounds` rebuilds its state from the
//! live world every round.
//!
//! The replan must plan, at each of four truncation points of the first plan
//! (a quarter, half, three quarters, all of it). That is the whole assertion,
//! and it is the one the baselines cannot make.
//!
//! # What it cannot see, stated rather than implied
//!
//! Inventories, bot positions, chest contents and research do not carry over
//! between the rounds -- the replan meets t=0 pockets beside a standing cell.
//! A live replan after a lost batch is close to that (its inventory is exactly
//! what the plan could not vouch for), but a replan after a *finished* batch
//! is not, and the refusal of `run-1788926478-07032` came on a THIRD plan
//! after a second one had executed. `plan --replan 2` reaches that round
//! offline; this test pins the first replan only, because that is the one a
//! merged fix has already failed. A savepoint-resumed live run remains the
//! check of record for the rest.
//!
//! # The dump, and why a miss FAILS
//!
//! `workspace/scripts/map.json` is 864 MB and not in the repository. Three
//! agents have reported "the planner suite is green" from worktrees where a
//! sibling test silently skipped for want of it, so this one **fails** when
//! the dump is absent -- with the symlink command in the message -- unless
//! `CI` is set, which GitHub sets and a worktree does not. Regenerate the dump
//! with `factorio-bot lua dump_31337.lua --headless --bots 4 --seed 31337 --new`.

use factorio_bot_core::factorio::world::FactorioSurface;
use factorio_bot_core::types::Position;
use factorio_bot_planner::method::produce::{cell_spec, standing_cells};
use factorio_bot_planner::standing::{survivors_of_failure, world_after};
use factorio_bot_planner::{
    ActionId, ActionNetwork, BotId, Goal, PlanState, PlannerError, Schedule, StepKind,
    pick_chain_actor, plan_best, registry_for,
};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

/// Where the dump is expected, relative to this crate.
const DUMP: &str = "../../workspace/scripts/map.json";

/// The seed-31337 t=0 dump, or a loud failure saying how to get it.
fn dump() -> Option<Arc<FactorioSurface>> {
    let path: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(DUMP);
    let Ok(path) = path.canonicalize() else {
        let message = format!(
            "workspace/scripts/map.json is not present at {}, so the replan check did NOT run. \
             In a worktree: `ln -s <main checkout>/workspace/scripts/map.json \
             <worktree>/workspace/scripts/map.json`. On a fresh clone regenerate it with \
             `factorio-bot lua dump_31337.lua --headless --bots 4 --seed 31337 --new`.",
            path.display()
        );
        if std::env::var_os("CI").is_some() {
            eprintln!("SKIPPED under CI: {message}");
            return None;
        }
        panic!("{message}");
    };
    let raw = std::fs::read_to_string(&path).expect("the dump is readable");
    let world: FactorioSurface =
        factorio_bot_core::serde_json::from_str(&raw).expect("map.json is a world dump");
    Some(Arc::new(world))
}

/// `continuous_supply.lua`'s goal, verbatim: PER_MINUTE is 6 there.
fn continuous_supply() -> Goal {
    Goal::All(vec![
        Goal::Sustain {
            item: "copper-plate".to_string(),
            per_minute: 15,
            window_ticks: 36_000,
        },
        Goal::Producing {
            item: "automation-science-pack".to_string(),
            per_minute: 6,
        },
    ])
}

const BOTS: [BotId; 4] = [BotId(1), BotId(2), BotId(3), BotId(4)];

/// Where a batch is cut, as a share of the first plan's makespan.
///
/// The live replans of 2026-09-09 followed a batch truncated at a failed
/// `take` some 47,000 ticks into a 46,985-tick plan -- nearly everything
/// stood except the science tail behind it. One cut cannot stand in for every
/// truncation, and a test that pinned the one tick the last run failed at
/// would be a test of that run; four cuts across the plan meet the copper
/// cell half-built, built, built with the link laid, and finished.
const CUTS: [(u32, u32); 4] = [(1, 4), (1, 2), (3, 4), (1, 1)];

fn plan(state: &PlanState, goal: &Goal) -> Result<(ActionNetwork, Schedule), PlannerError> {
    let chain_actor = pick_chain_actor(state, &BOTS).expect("four bots");
    plan_best(
        std::slice::from_ref(goal),
        state,
        &registry_for(&BOTS),
        chain_actor,
        &BOTS,
    )
}

#[test]
fn the_science_cell_plans_again_from_the_world_it_built() {
    let Some(world) = dump() else {
        return;
    };
    let goal = continuous_supply();

    let first = PlanState::from_world(world, &BOTS);
    let (net, scheduled) = plan(&first, &goal)
        .expect("the science cell plans from t=0 -- that is the baseline every session takes");
    eprintln!(
        "first plan: {} actions, makespan {}",
        net.len(),
        scheduled.makespan
    );

    let mut failures = Vec::new();
    for (num, den) in CUTS {
        let cut = scheduled.makespan / den * num;
        let done: BTreeSet<ActionId> = scheduled
            .steps
            .iter()
            .filter(|step| step.end <= cut)
            .filter_map(|step| match &step.what {
                StepKind::Act { action, .. } => Some(*action),
                StepKind::Walk { .. } => None,
            })
            .collect();
        let (standing_world, standing) = world_after(&first, &net, |id| done.contains(&id))
            .expect("the plan has a topological order");
        assert!(
            standing.placed >= 1,
            "by tick {cut} ({num}/{den} of the plan) something must stand, or this cut \
             measures the t=0 dump again: {standing:?}"
        );

        // A FRESH state, not a fork: the live supervisor starts every round
        // from the world and nothing else, and a fork would carry the first
        // expansion's reservations into the second -- masking exactly the
        // defect `a69ae64c` fixed.
        let second = PlanState::from_world(standing_world, &BOTS);
        match plan(&second, &goal) {
            Ok((net, scheduled)) => eprintln!(
                "cut {num}/{den} at tick {cut}, {standing:?}: replanned {} actions, makespan {}",
                net.len(),
                scheduled.makespan
            ),
            // The driver's reading (`goal/mod.rs` in `scripting_lua`): the
            // whole arrangement stands and nothing is left to build, so the
            // milestone is satisfied. Expected at the full cut and nowhere
            // earlier -- an earlier cut has the science tail still to build.
            Err(PlannerError::SustainSupplyNotStanding { .. }) if den == num => eprintln!(
                "cut {num}/{den} at tick {cut}, {standing:?}: everything stands, nothing to build"
            ),
            Err(err) => failures.push(format!(
                "cut {num}/{den} at tick {cut} ({standing:?}): {err}"
            )),
        }
    }
    assert!(
        failures.is_empty(),
        "the science cell planned from t=0 ({} actions, makespan {}) and REFUSED on a replan \
         against the world that plan left standing:\n  {}\n\
         This is the replan-only refusal class that swallowed a reviewed fix on 2026-09-09; \
         the offline baselines cannot see it and this test exists so something does. \
         Reproduce with `factorio-bot plan --world workspace/scripts/map.json --bots 1,2,3,4 \
         --all --replan 1 --done-by <tick> --goal sustain:copper-plate:15:36000 \
         --goal producing:automation-science-pack:6`.",
        net.len(),
        scheduled.makespan,
        failures.join("\n  ")
    );
}

/// The four tiles round a chest, the only places a belt out of it can start.
fn neighbours(of: &Position) -> [Position; 4] {
    [
        Position::new(of.x() + 1., of.y()),
        Position::new(of.x() - 1., of.y()),
        Position::new(of.x(), of.y() + 1.),
        Position::new(of.x(), of.y() - 1.),
    ]
}

/// The plate chests of every standing copper cell: an `iron-chest` beside
/// the furnace that a `burner-inserter` carries the furnace's output into.
/// The cell's COAL chests are not here -- those legitimately spend every
/// side they have (see the `connect` entry in `CLAUDE.md`).
fn plate_chests(state: &PlanState) -> Vec<Position> {
    let spec = cell_spec(state, "copper-plate").expect("a stone furnace smelts copper");
    let mut chests = Vec::new();
    for cell in standing_cells(state, &spec) {
        for chest in state.entities_within(&cell.furnace, 3.5) {
            if chest.name != "iron-chest" {
                continue;
            }
            let carried = state
                .entities_within(&chest.position, 1.5)
                .into_iter()
                .filter(|arm| arm.name == "burner-inserter")
                .any(|arm| {
                    state.delivers_into(&cell.furnace, &arm.position)
                        && state.delivers_into(&arm.position, &chest.position)
                });
            if carried {
                chests.push(chest.position.clone());
            }
        }
    }
    chests
}

/// **The live shape, and the one that falsifies `a69ae64c`.** The first
/// take of copper plates out of the cell fails and everything downstream of
/// it is abandoned -- `survivors_of_failure`, the executor's own rule -- so
/// the cell and every coal run stand finished while the science tail does
/// not, exactly as `run-1788923927-04849` left the world at tick 48,017.
///
/// Two assertions, in order of what they are evidence of:
///
/// 1. **The plate chest keeps a way out.** With the reservation a local of
///    `sustain`'s expansion (before `a69ae64c`) the hand-smelt furnace of a
///    `have copper-plate` subgoal landed on the kept exit, and all four of
///    the chest's sides were spent. Measured on this test's own world with
///    that commit reverted by copy+touch: the replan refused on the run's
///    four tiles byte for byte, `[28.5,-46.5] [29.5,-48.5] [29.5,-45.5]
///    [30.5,-46.5]`. This is the invariant that commit states, asked of the
///    standing world directly.
/// 2. **Whatever the replan says, it does not blame that chest's exit.**
///    The replan is NOT asserted to plan: on `71f9227c` it refuses further
///    along, on two items already on record as open -- the half-built
///    supply link's first belt on the furnace end, and the science cell's
///    supply chest boxed in on the other (`docs/superpowers/notes/
///    2026-09-09-a-replan-you-can-run-offline.md`). Reproduce with
///    `just replan-check --fail "copper-plate from the cell"`. When that
///    refusal is fixed, tighten this to `expect`.
#[test]
fn a_failed_take_leaves_the_plate_chest_a_way_out() {
    let Some(world) = dump() else {
        return;
    };
    let goal = continuous_supply();
    let first = PlanState::from_world(world, &BOTS);
    let (net, scheduled) = plan(&first, &goal).expect("the science cell plans from t=0");
    let failed = scheduled
        .steps
        .iter()
        .find_map(|step| match &step.what {
            StepKind::Act { action, label } if label.contains("copper-plate from the cell") => {
                Some(*action)
            }
            _ => None,
        })
        .expect(
            "the plan takes copper plates out of the cell -- if it no longer does, the \
                 live shape this test reproduces has changed and the label needs updating",
        );
    let done = survivors_of_failure(&net, failed);
    let (standing_world, standing) =
        world_after(&first, &net, |id| done.contains(&id)).expect("a topological order");
    assert!(standing.placed >= 10, "{standing:?}");

    let second = PlanState::from_world(standing_world, &BOTS);
    let chests = plate_chests(&second);
    assert!(
        !chests.is_empty(),
        "a copper cell stands with a chest its arm fills, or this test measures nothing"
    );
    for chest in &chests {
        let open = neighbours(chest)
            .iter()
            .filter(|tile| second.is_area_free("transport-belt", tile))
            .count();
        assert!(
            open > 0,
            "the plate chest at {chest} has all four sides spent by the plan that built it, so \
             no belt can ever leave it -- the exit `sustain` reserves has to be state every \
             siting reads (`a69ae64c`), and it is not"
        );
    }

    match plan(&second, &goal) {
        Ok((net, scheduled)) => eprintln!(
            "after a failed take, {standing:?}: replanned {} actions, makespan {}",
            net.len(),
            scheduled.makespan
        ),
        Err(err) => {
            let text = err.to_string();
            eprintln!(
                "after a failed take the replan refuses further along, as recorded open: {text}"
            );
            // Before `a69ae64c` the refusal listed the plate chest's own four
            // neighbours as the blockers. A refusal naming some OTHER chest's
            // four sides (the science cell's supply chest, on record as open)
            // is not that, so the check is on which tiles are named.
            for chest in &chests {
                let names_every_side = neighbours(chest)
                    .iter()
                    .all(|tile| text.contains(&tile.to_string()));
                assert!(
                    !names_every_side,
                    "the replan blames the plate chest at {chest}'s own four sides again: {text}"
                );
            }
        }
    }
}
