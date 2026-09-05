//! A bot the game has benched receives no step that would make it walk.
//!
//! # The run
//!
//! `run-1788614781-38058`: eight headless character bots on seed 31337. Bot 6
//! ended up at `(-5.2, -29.1)`, overlapping a stone furnace another bot had
//! placed at `[-5, -28]`, and every path request from it was refused -- four
//! `walk_settled` rows with `failure.kind: no_path`, the later three all to
//! `(12.5, -33.5)`, a coal tile other bots reached without trouble.
//!
//! Two memories already existed and neither helped:
//!
//! * The walk-refusal ledger held the pair, and `schedule` consulted it -- as
//!   a *tier reordering*. The mine came from a gathering share bot 6 owned,
//!   so its candidate list had one bot in it, and reordering a one-element
//!   list is a no-op. `crates/planner/tests/unreachable_memory.rs` pins that
//!   exact limitation, on purpose.
//! * The walled-in fill said `Open`. It models no characters and seeds from
//!   the tile centre, and the furnace's box grown by the character's half-box
//!   did not cover that centre. So `PlanState::walled_in` stayed empty, and
//!   `even_shares` handed bot 6 a share on every one of seven plans.
//!
//! The bench is the game's own verdict, written by the executor after the
//! pathfinder refused a short hop in every direction, and this file is what
//! it buys: the bot is out of the share split, out of the chain-actor pick,
//! and refused every scheduler pairing that would make it walk -- while
//! still allowed to act where it stands.

use factorio_bot_core::factorio::world::{Bench, FactorioWorld, HOP_DISTANCE};
use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::{FactorioPlayer, Position};
use factorio_bot_planner::action::{Action, ActionKind, Actor, Condition, Effect};
use factorio_bot_planner::ids::ActionIdGen;
use factorio_bot_planner::{
    ActionNetwork, BotId, PlanState, PlannerError, Schedule, StepKind, pick_chain_actor, schedule,
};
use std::sync::Arc;

/// The benched spot. Not bot 6's own `(-5.2, -29.1)`: the fixture world is
/// not seed 31337, and the test needs a spot the fixture has coal *near*, so
/// that with no bench the nearest bot is one standing here. This is the
/// spot `unreachable_memory.rs` uses for the same reason.
const BENCHED_AT: (f64, f64) = (-56.2578125, 14.74609375);
/// Where a healthy bot is working, far from that coal.
const ELSEWHERE: (f64, f64) = (-23.1875, -37.90625);

fn at(p: (f64, f64)) -> Position {
    Position::new(p.0, p.1)
}

fn bench(player: u8, at_pos: (f64, f64)) -> Bench {
    Bench {
        tick: None,
        player,
        at: at(at_pos),
        refused_hops: 4,
        hop_tiles: HOP_DISTANCE,
    }
}

/// A fixture world whose players stand where the run had them, carrying
/// `benches` the way a run's `FactorioWorld` carries them into the next plan.
fn world_with(benches: &[Bench]) -> Arc<FactorioWorld> {
    let world = fixture_world();
    for (id, position) in [
        (1u8, ELSEWHERE),
        (2, BENCHED_AT),
        (3, BENCHED_AT),
        (4, ELSEWHERE),
    ] {
        world.players.insert(
            id,
            FactorioPlayer {
                player_id: id,
                position: at(position),
                ..Default::default()
            },
        );
    }
    for bench in benches {
        world.record_bench(bench.clone());
    }
    Arc::new(world)
}

fn roster() -> Vec<BotId> {
    vec![BotId(1), BotId(2), BotId(3), BotId(4)]
}

fn state_with(benches: &[Bench]) -> PlanState {
    PlanState::from_world(world_with(benches), &roster())
}

/// A coal tile nearest the benched spot, so that with no bench the cheapest
/// bot for it is one standing there.
fn ore_near_the_bench(state: &PlanState) -> Position {
    let mut tiles: Vec<Position> = state
        .resource_patches("coal")
        .into_iter()
        .flat_map(|patch| patch.elements)
        .collect();
    tiles.sort_by(|a, b| {
        let d = |p: &Position| (p.x - BENCHED_AT.0).hypot(p.y - BENCHED_AT.1);
        d(a).total_cmp(&d(b)).then(a.x.total_cmp(&b.x))
    });
    tiles.first().expect("the fixture has a coal patch").clone()
}

fn mine_at(id_gen: &mut ActionIdGen, pos: &Position) -> Action {
    Action {
        id: id_gen.next(),
        kind: ActionKind::Mine {
            pos: pos.clone(),
            item: "coal".into(),
            count: 1,
        },
        pre: vec![Condition::AtPosition {
            who: Actor::Role,
            pos: pos.clone(),
            radius: 3.0,
            min_radius: 0.0,
        }],
        eff: vec![Effect::GainItem {
            who: Actor::Role,
            item: "coal".into(),
            count: 1,
        }],
        duration: 60,
        pinned: None,
        label: "mine 1 coal".into(),
    }
}

fn craft(id_gen: &mut ActionIdGen) -> Action {
    Action {
        id: id_gen.next(),
        kind: ActionKind::Craft {
            item: "iron-gear-wheel".into(),
            count: 1,
        },
        pre: vec![],
        eff: vec![Effect::GainItem {
            who: Actor::Role,
            item: "iron-gear-wheel".into(),
            count: 1,
        }],
        duration: 30,
        pinned: None,
        label: "craft 1 iron-gear-wheel".into(),
    }
}

fn walker(schedule: &Schedule, to: &Position) -> Option<BotId> {
    schedule
        .steps
        .iter()
        .find(|step| match &step.what {
            StepKind::Walk { to: dest, .. } => {
                dest.x.total_cmp(&to.x).is_eq() && dest.y.total_cmp(&to.y).is_eq()
            }
            StepKind::Act { .. } => false,
        })
        .map(|step| step.bot)
}

fn walks_of(schedule: &Schedule, bot: BotId) -> usize {
    schedule
        .steps
        .iter()
        .filter(|step| step.bot == bot && matches!(step.what, StepKind::Walk { .. }))
        .count()
}

fn schedule_one_mine(benches: &[Bench]) -> (PlanState, Position, Schedule) {
    let state = state_with(benches);
    let pos = ore_near_the_bench(&state);
    let mut id_gen = ActionIdGen::new();
    let mut net = ActionNetwork::new();
    net.add(mine_at(&mut id_gen, &pos));
    let scheduled = schedule(&net, &state, &roster()).expect("one mine, four bots");
    (state, pos, scheduled)
}

/// The control: with no bench the nearest bot wins, which is a bot standing
/// on the furnace.
#[test]
fn the_nearest_bot_wins_when_nobody_is_benched() {
    let (state, pos, scheduled) = schedule_one_mine(&[]);
    assert!(state.benched().is_empty());
    let sent = walker(&scheduled, &pos).expect("the mine needs a walk");
    assert!(
        sent == BotId(2) || sent == BotId(3),
        "the bots at the furnace are nearest this tile; got {sent}"
    );
}

/// The point: the bench reaches the plan, and the plan sends somebody else.
#[test]
fn a_benched_bot_is_not_sent_even_when_it_is_nearest() {
    let (state, pos, scheduled) = schedule_one_mine(&[bench(2, BENCHED_AT), bench(3, BENCHED_AT)]);
    assert!(state.is_benched(BotId(2)) && state.is_benched(BotId(3)));
    let sent = walker(&scheduled, &pos).expect("the mine still needs a walk");
    assert!(
        sent == BotId(1) || sent == BotId(4),
        "the game said 2 and 3 cannot move; got {sent}"
    );
    assert_eq!(walks_of(&scheduled, BotId(2)), 0);
    assert_eq!(walks_of(&scheduled, BotId(3)), 0);
}

/// A bench is a fact about a position. A bot found somewhere else is not
/// the bot the game answered for, and is back without anyone lifting the row.
#[test]
fn a_bench_earned_somewhere_else_does_not_apply() {
    let stale = bench(2, (BENCHED_AT.0 - 30., BENCHED_AT.1));
    let state = state_with(&[stale.clone(), bench(3, BENCHED_AT)]);
    assert!(
        !state.is_benched(BotId(2)),
        "bot 2 is 30 tiles from where it was benched; got {:?}",
        state.benched()
    );
    assert!(state.is_benched(BotId(3)));
    let (_, pos, scheduled) = schedule_one_mine(&[stale, bench(3, BENCHED_AT)]);
    assert_eq!(
        walker(&scheduled, &pos),
        Some(BotId(2)),
        "bot 2 is nearest and its bench is about somewhere else"
    );
}

/// The exclusion is about walking, not about working: a benched bot may
/// still do what needs no position.
#[test]
fn a_benched_bot_may_still_act_where_it_stands() {
    let state = state_with(&[bench(2, BENCHED_AT)]);
    let mut id_gen = ActionIdGen::new();
    let mut net = ActionNetwork::new();
    let craft = craft(&mut id_gen);
    let craft_id = craft.id;
    net.add(craft);
    // Only bot 2 in the roster, so the craft is its or nobody's.
    let scheduled = schedule(&net, &state, &[BotId(2)]).expect("a craft needs no walk");
    let step = scheduled
        .steps
        .iter()
        .find(|s| matches!(&s.what, StepKind::Act { action, .. } if *action == craft_id))
        .expect("the craft was scheduled");
    assert_eq!(step.bot, BotId(2));
    assert_eq!(walks_of(&scheduled, BotId(2)), 0);
}

/// When the benched bot is the only bot, the plan is refused and the error
/// names the bench -- rather than dispatching the walk the game already
/// refused, which is what seven plans of the run did.
#[test]
fn a_plan_with_only_benched_bots_is_refused_by_name() {
    let state = state_with(&[bench(2, BENCHED_AT)]);
    let pos = ore_near_the_bench(&state);
    let mut id_gen = ActionIdGen::new();
    let mut net = ActionNetwork::new();
    net.add(mine_at(&mut id_gen, &pos));
    let err = schedule(&net, &state, &[BotId(2)]).expect_err("nobody can walk there");
    match err {
        PlannerError::PreconditionUnsatisfied { bot, condition, .. } => {
            assert_eq!(bot, BotId(2));
            assert!(
                condition.contains("benched"),
                "the error must say why: {condition}"
            );
        }
        other => panic!("expected the bench to be named, got {other:?}"),
    }
}

/// The chain actor -- the bot a goal naming no holder is sized against and
/// welded to -- is not a benched bot either, for the reason
/// `pick_chain_actor`'s walled-in rule gives: an owned chain has no fallback.
#[test]
fn the_chain_actor_is_not_a_benched_bot() {
    let state = state_with(&[bench(2, BENCHED_AT)]);
    assert_eq!(
        pick_chain_actor(&state, &[BotId(2), BotId(3), BotId(1)]),
        Some(BotId(3)),
        "bot 2 is benched; the next preference that can move is bot 3"
    );
    let all_benched = state_with(&[bench(2, BENCHED_AT), bench(3, BENCHED_AT)]);
    assert_eq!(
        pick_chain_actor(&all_benched, &[BotId(2), BotId(3)]),
        Some(BotId(2)),
        "with every preference benched the first is taken, exactly as for walled-in"
    );
}

/// The bench survives a dump: a plan made offline against a world dumped with
/// a benched bot must not re-send it, or the offline loop would reproduce
/// the live defect and call it fixed.
#[test]
fn a_bench_survives_the_world_round_trip() {
    let live = world_with(&[bench(2, BENCHED_AT)]);
    let json = serde_json::to_string(&*live).expect("a world serialises");
    let loaded: FactorioWorld = serde_json::from_str(&json).expect("and loads");
    assert_eq!(loaded.benches(), live.benches());
    let state = PlanState::from_world(Arc::new(loaded), &roster());
    assert!(state.is_benched(BotId(2)));
}
