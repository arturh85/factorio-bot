//! **Adding a bot must not turn a plan into a refusal.**
//!
//! On the seed-31337 t=0 dump, `have:pumpjack:1` planned for one bot
//! (746,162 ticks) and for two (458,660), and refused for three and for four
//! with
//!
//! ```text
//! no method can satisfy goal: have 6 iron-ore (a share sized for bot 3)
//! ```
//!
//! Four bots is the default roster, so the roster the project actually runs
//! was the roster that could not plan. The oil survey found the same for
//! `researched:engine`, `researched:automation-2`,
//! `researched:fluid-handling`, `researched:oil-gathering` and
//! `have:storage-tank:1`.
//!
//! # The mechanism, measured
//!
//! Not the share arithmetic. `even_shares` divides a shortfall evenly and a
//! share of six is not harder to satisfy than a share of nine — it is
//! *later*. The ledger it draws on is what has run out.
//!
//! A mining claim used to spend a **whole tile**, permanently, whoever made
//! it and however little it took, and `PlanState::mining_tile_separation`
//! then crowds every tile within 3.69 of that claim out for every *other*
//! runner. Shares are per-bot, so a roster of `n` burns `n` tiles per
//! shortfall where a roster of two burns two — and a deep goal has hundreds
//! of shortfalls. At the refusal, on a charted iron field of 940 tiles
//! holding **522,467 ore**: 324 tiles claimed (still holding 134,734 ore
//! between them), the other 616 crowded, zero available. The goal needed
//! **six**.
//!
//! So a plan plainly existed and the planner failed to find it. The fix is in
//! `PlanState::claim_yields_to`: where the world states what a tile holds, the
//! runner that claimed it may draw from it again against what is left, which
//! is the argument `MiningClaim` already makes for crowding (one bot runs one
//! action at a time). Where the world states nothing, the old whole-tile rule
//! and its old reason stand.
//!
//! # Why the fixture, and not the dump
//!
//! The dump is 864 MB and lives outside the repository. The same cliff
//! reproduces on the fixture world at the same shape — the fixture's iron
//! field is 121 tiles rather than 940, so it takes a bigger bill to exhaust
//! it, and `transport-belt` is the smallest goal in reach that does.

use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::{Direction, FactorioEntity};
use factorio_bot_planner::goal::{Goal, Holder};
use factorio_bot_planner::{BotId, PlanState, expand, pick_chain_actor, registry_for};
use std::sync::Arc;

/// What a live iron tile holds where anybody has actually looked. The
/// seed-31337 dump's charted iron field averages 556 ore per tile; the point
/// of the number is only that the world *states* it, which is the condition
/// [`PlanState::claim_yields_to`] turns on.
const REPORTED: u32 = 500;

/// The fixture, with every iron tile re-delivered carrying an amount — the
/// re-delivery `tests/tile_capacity.rs` uses, which is how a real reading
/// arrives.
fn world_with_iron_holding(bots: &[BotId], amount: u32) -> PlanState {
    let world = fixture_world();
    let ore: Vec<FactorioEntity> = world
        .entity_graph
        .resource_patches("iron-ore")
        .into_iter()
        .flat_map(|patch| patch.elements)
        .map(|tile| {
            let mut entity = FactorioEntity::new_resource(&tile, Direction::North, "iron-ore");
            entity.amount = Some(amount);
            entity
        })
        .collect();
    assert!(!ore.is_empty(), "the fixture carries an iron field");
    world
        .update_chunk_entities(ore)
        .expect("re-delivering ore with amounts");
    PlanState::from_world(Arc::new(world), bots)
}

/// The number of actions a roster plans for `goal`, or the refusal.
fn plan(item: &str, count: u32, bots: &[BotId]) -> Result<usize, String> {
    let state = world_with_iron_holding(bots, REPORTED);
    let goals = vec![Goal::Have {
        item: item.into(),
        count,
        whose: Holder::Anyone,
        via: None,
    }];
    let actor = pick_chain_actor(&state, bots).expect("a roster has a chain actor");
    expand(&goals, &state, &registry_for(bots), actor)
        .map(|net| net.len())
        .map_err(|err| err.to_string())
}

fn roster(n: u8) -> Vec<BotId> {
    (1..=n).map(BotId).collect()
}

/// The shape, stated once: a goal that plans for `n` bots plans for `n + 1`
/// on the same world.
///
/// Before the fix this failed at four: one, two and three bots planned
/// (42, 84 and 118 actions) and four refused with `no method can satisfy
/// goal: have 25 iron-ore (a share sized for bot 4)`.
#[test]
fn a_bigger_roster_never_refuses_what_a_smaller_one_planned() {
    let mut previous: Option<(u8, usize)> = None;
    for n in 1..=4u8 {
        match plan("transport-belt", 200, &roster(n)) {
            Ok(actions) => previous = Some((n, actions)),
            Err(refusal) => {
                let (planned_with, actions) =
                    previous.expect("a one-bot roster is the base case and must plan");
                panic!(
                    "{planned_with} bots planned this goal in {actions} actions and \
                     {n} refused it on the same world: {refusal}"
                );
            }
        }
    }
}

/// The same world, said the other way round, so a regression cannot hide
/// behind an early refusal: every roster from one to four plans, and the
/// deeper the roster the more the plan has to do rather than less.
#[test]
fn every_roster_from_one_to_four_plans_the_same_goal() {
    let counts: Vec<usize> = (1..=4u8)
        .map(|n| plan("transport-belt", 200, &roster(n)).expect("every roster plans"))
        .collect();
    assert!(
        counts.windows(2).all(|w| w[0] <= w[1]),
        "a bigger roster splits the same bill into more chains, not fewer: {counts:?}"
    );
}

/// The control. The relaxation is gated on the world having *stated* what a
/// tile holds, so on the untouched fixture — where every amount is a
/// `DEFAULT_RESOURCE_PER_TILE` guess — nothing about tile selection changed.
/// If this ever starts refusing, the gate has been removed rather than the
/// ledger fixed.
#[test]
fn a_world_that_states_no_amounts_keeps_the_old_rule() {
    let bots = roster(2);
    let state = PlanState::from_world(Arc::new(fixture_world()), &bots);
    let goals = vec![Goal::Have {
        item: "iron-plate".into(),
        count: 40,
        whose: Holder::Anyone,
        via: None,
    }];
    let actor = pick_chain_actor(&state, &bots).expect("a roster has a chain actor");
    expand(&goals, &state, &registry_for(&bots), actor).expect("the guessed world still plans");
}
