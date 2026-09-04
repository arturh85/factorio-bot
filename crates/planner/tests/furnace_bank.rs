//! A smelt uses the furnaces that already stand, and spreads its runs over
//! them.
//!
//! # The gate
//!
//! `BuildCell` and `BuildAssemblyCell` have always subtracted what already
//! stands before billing for more — `needed.saturating_sub(cells_standing(..))`
//! in `produce.rs` and `assemble.rs`. **The hand-smelt path had no equivalent.**
//! Every `Smelt` expansion asked for one `stone-furnace` and placed it, however
//! many furnaces of its own an earlier plan had left beside the same ore.
//!
//! That does not merely waste stone; it does not converge. Milestone 2 of the
//! run recorded on 2026-09-03 placed 4, 8, 11, 11, 12, 11 and 9 furnaces over
//! seven plan epochs — 66 in all, three of those batches completing with zero
//! failures. Fifty-six minutes, `best` improving once. The work finished each
//! time and the goal re-derived a fresh furnace bill.
//!
//! So the first four tests here are a **fix**: a plan run twice against a world
//! that kept the first plan's furnaces must not bill for them again. The last
//! three are the optimisation on top — a bank of `k` furnaces waits
//! `ceil(runs / k)` cycles instead of `runs`, which is the 39.1% of
//! `run-1788459085-32452`'s milestone 1 that was one bot standing beside one
//! furnace.

mod common;

use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::{Direction, FactorioEntity, Position};
use factorio_bot_planner::action::{ActionKind, Effect};
use factorio_bot_planner::goal::{Goal, Holder};
use factorio_bot_planner::method::expand;
use factorio_bot_planner::method::have::registry_for;
use factorio_bot_planner::{ActionNetwork, BotId, PlanState, schedule};
use std::collections::BTreeSet;
use std::sync::Arc;

/// One stone furnace smelts one iron plate in 3.2 s.
const PER_RUN: u32 = 192;

/// Open ground east of the fixture's iron patch (centred `(-40, 40)`, 11 tiles
/// across), on the 2-tile grid a stone furnace is placed on, and inside the
/// patch-wide radius `adoptable_furnaces` scans.
fn bank_sites(count: usize) -> Vec<Position> {
    (0..count)
        .map(|i| Position::new(-34.0 + 2.0 * (i % 4) as f64, 40.0 + 2.0 * (i / 4) as f64))
        .collect()
}

/// `fixture_world()` with `count` idle stone furnaces standing beside the iron.
fn world_with_standing_furnaces(count: usize) -> Arc<FactorioWorldAlias> {
    let world = fixture_world();
    for site in bank_sites(count) {
        world
            .on_some_entity_created(FactorioEntity::new_stone_furnace(&site, Direction::North))
            .expect("the ground east of the patch is open");
    }
    Arc::new(world)
}

type FactorioWorldAlias = factorio_bot_core::factorio::world::FactorioWorld;

fn plan_for(world: Arc<FactorioWorldAlias>, plates: u32) -> (ActionNetwork, PlanState) {
    let bots = [BotId(1)];
    let state = PlanState::from_world(world, &bots);
    let net = expand(
        &[Goal::Have {
            item: "iron-plate".into(),
            count: plates,
            whose: Holder::Bot(BotId(1)),
        }],
        &state,
        &registry_for(&bots),
        BotId(1),
    )
    .expect("twenty plates plan");
    (net, state)
}

fn furnaces_placed(net: &ActionNetwork) -> Vec<Position> {
    net.actions()
        .filter_map(|a| match &a.kind {
            ActionKind::Place { entity } if entity.name == "stone-furnace" => {
                Some(entity.position.clone())
            }
            _ => None,
        })
        .collect()
}

/// Stone the plan commits to gathering, by whichever verb gathers it.
///
/// **A rock counts.** This looked at `ActionKind::Mine` alone until
/// 2026-09-04, when `Chop` moved ahead of `Mine` and the fixture's own
/// `rock-huge` became the cheaper way to get five stone. A helper blind to
/// that reads "no stone is owed" off a plan that is about to go and get
/// twenty-four of it, which is the opposite of what every caller here means.
/// A chop's yield is stated by its effects and never inferred from its
/// `count`, which is a number of entities.
fn stone_gathered(net: &ActionNetwork) -> u32 {
    net.actions()
        .map(|a| match &a.kind {
            ActionKind::Mine { item, count, .. } if item == "stone" => *count,
            ActionKind::Chop { .. } => a
                .eff
                .iter()
                .filter_map(|e| match e {
                    Effect::GainItem { item, count, .. } if item == "stone" => Some(*count),
                    _ => None,
                })
                .sum(),
            _ => 0,
        })
        .sum()
}

/// Every furnace tile the plan puts iron ore into.
fn loaded_furnaces(net: &ActionNetwork) -> BTreeSet<String> {
    net.actions()
        .filter_map(|a| match &a.kind {
            ActionKind::Insert { pos, item, .. } if item == "iron-ore" => Some(format!("{pos}")),
            _ => None,
        })
        .collect()
}

/// The longest smelt lag in the plan — the wait the last take still serves.
fn longest_wait(net: &ActionNetwork) -> u32 {
    net.actions()
        .flat_map(|a| net.preds(a.id))
        .map(|(_, lag)| lag)
        .max()
        .expect("a smelt states lag edges")
}

// ---------------------------------------------------------------------------
// The fix: a plan does not re-bill what the last one built
// ---------------------------------------------------------------------------

/// The churn, reproduced and closed: plan, keep what it placed, plan again.
///
/// This is the property the live run violated 66 times. The second plan may do
/// anything it likes *except* pay for the furnaces the first one already
/// stood up.
#[test]
fn a_second_plan_does_not_rebuild_what_the_first_one_placed() {
    let world = fixture_world();
    let (first, _) = plan_for(Arc::new(world.clone()), 20);
    let built = furnaces_placed(&first);
    assert!(
        !built.is_empty(),
        "a bare world has to build the first furnace"
    );

    // The world the executor would hand the next plan: the same world, with
    // the furnaces the first plan placed now standing in it.
    for site in &built {
        world
            .on_some_entity_created(FactorioEntity::new_stone_furnace(site, Direction::North))
            .expect("the first plan sited these itself, so they fit");
    }
    let (second, _) = plan_for(Arc::new(world), 20);

    assert_eq!(
        furnaces_placed(&second),
        Vec::<Position>::new(),
        "the second plan built furnaces beside the ones the first one left"
    );
    assert_eq!(
        stone_gathered(&second),
        0,
        "no stone is owed for a furnace that already stands"
    );
}

/// The bill, not just the placements: adopting means no `stone-furnace`
/// subgoal at all, so nothing is crafted and no stone is mined for it.
#[test]
fn a_standing_furnace_takes_the_stone_out_of_the_bill() {
    let (bare, _) = plan_for(Arc::new(fixture_world()), 20);
    let (adopting, _) = plan_for(world_with_standing_furnaces(1), 20);

    assert_eq!(furnaces_placed(&bare).len(), 1);
    // Twenty-four, not five: since 2026-09-04 the cheapest five stone on this
    // fixture is one swing at a `rock-huge` (360 ticks against 5 * 120), and a
    // rock is indivisible -- see `have::Chop`. What this test is about is the
    // *adopting* plan owing nothing at all, and that is unchanged.
    assert_eq!(
        stone_gathered(&bare),
        24,
        "one rock is what a furnace's stone costs now"
    );

    assert!(furnaces_placed(&adopting).is_empty());
    assert_eq!(stone_gathered(&adopting), 0);
    assert!(
        !adopting
            .actions()
            .any(|a| matches!(&a.kind, ActionKind::Craft { item, .. } if item == "stone-furnace")),
        "nothing should be crafted for a furnace that already stands"
    );
}

/// A furnace a drill is feeding is a cell's furnace, and belongs to the cell.
///
/// Before this guard the solo `Researched("automation")` plan adopted the
/// furnace at `[-35, 33]` that the drill at `[-35, 35]` had been placed to
/// feed, five actions after placing it — and would then have taken plates
/// `PlaceDrill`'s own take had already been promised.
///
/// Since 2026-09-05 the cell itself is what serves the twenty: a standing
/// pair on the ore is topped up and drained by `PlaceDrill` rather than
/// left idle beside a furnace built for the hand. So the smelt builds
/// nothing here either — not because it adopted the furnace, but because it
/// was never asked. What the guard protects is unchanged and still pinned:
/// no hand loads ore into a furnace a drill feeds.
#[test]
fn a_furnace_a_drill_feeds_is_not_adopted() {
    let world = fixture_world();
    let furnace = Position::new(-34.0, 40.0);
    world
        .on_some_entity_created(FactorioEntity::new_stone_furnace(
            &furnace,
            Direction::North,
        ))
        .expect("open ground");
    world
        .on_some_entity_created(FactorioEntity::new_burner_mining_drill(
            &Position::new(-34.0, 42.0),
            Direction::North,
        ))
        .expect("open ground");
    let (net, state) = plan_for(Arc::new(world), 20);
    assert!(
        state.entity_at(&furnace).is_some(),
        "the fixture really stands a furnace there"
    );
    assert!(
        !loaded_furnaces(&net).contains(&format!("{furnace}")),
        "a drill-fed furnace belongs to its cell; no hand loads ore into it"
    );
    assert!(
        furnaces_placed(&net).is_empty(),
        "and the cell serves the plates, so the smelt is never asked to build its own"
    );
    assert!(
        net.actions().any(|a| matches!(
            &a.kind,
            ActionKind::Remove { pos, item, .. } if *pos == furnace && item == "iron-plate"
        )),
        "the plates come out of the cell's furnace"
    );
}

/// Nothing standing is the path every run starts on, and it must plan exactly
/// as it did before any of this existed: one furnace, one take, and a wait of
/// `192 * (runs + 1)`.
#[test]
fn a_bare_world_plans_exactly_as_it_did_before() {
    let (net, state) = plan_for(Arc::new(fixture_world()), 20);
    assert_eq!(furnaces_placed(&net).len(), 1);
    assert_eq!(loaded_furnaces(&net).len(), 1);
    assert_eq!(
        net.actions()
            .filter(|a| matches!(&a.kind, ActionKind::Remove { item, .. } if item == "iron-plate"))
            .count(),
        1
    );
    assert_eq!(
        longest_wait(&net),
        PER_RUN * 21,
        "twenty runs and one cycle of headroom, in one furnace"
    );
    assert!(
        schedule(&net, &state, &[BotId(1)]).is_ok(),
        "the unchanged path still schedules"
    );
}

// ---------------------------------------------------------------------------
// The optimisation: the runs spread over the bank
// ---------------------------------------------------------------------------

/// The point of the change. Twenty plates through one furnace is twenty serial
/// cycles; through four it is five, and the take waits for the slowest.
///
/// The `k` is [`bank_size`]'s, derived from the arithmetic and not chosen here
/// — so these are the numbers that derivation actually produces, listed so a
/// change to it has to be explained rather than absorbed.
///
/// [`bank_size`]: factorio_bot_planner
#[test]
fn the_wait_falls_as_the_bank_widens() {
    let waits: Vec<(usize, u32, usize)> = [0usize, 1, 2, 4, 8]
        .into_iter()
        .map(|standing| {
            let (net, _) = plan_for(world_with_standing_furnaces(standing), 20);
            (standing, longest_wait(&net), loaded_furnaces(&net).len())
        })
        .collect();

    assert_eq!(
        waits,
        vec![
            // Nothing to adopt: one furnace built, twenty runs in it.
            (0, PER_RUN * 21, 1),
            (1, PER_RUN * 21, 1),
            // ceil(20/2) = 10 runs each.
            (2, PER_RUN * 11, 2),
            // ceil(20/4) = 5.
            (4, PER_RUN * 6, 4),
            // Five, not eight, and the reason is the coal. Splitting rounds
            // each furnace's share up to a whole coal, so a `k`-wide bank
            // burns up to `k` coal where one furnace burned two, and coal is
            // mined at 120 ticks each against 192 saved per cycle removed.
            // `bank_size` totals `wait + 30*(k-1) + 120*extra_coal`:
            // 4032, 2142, 1716, 1482, **1440**, 1590, 1548, 1698 for k = 1..8.
            // Six is worse than five because `ceil(20/6)` is still 4 and the
            // sixth furnace buys a coal and no cycles.
            (8, PER_RUN * 5, 5),
        ],
        "the wait must fall with the bank, and by the arithmetic"
    );
}

/// The whole bill still arrives, and no more: the ore is dealt out over the
/// bank, not multiplied by it.
#[test]
fn a_bank_loads_and_unloads_exactly_the_goal() {
    let (net, _) = plan_for(world_with_standing_furnaces(4), 20);
    let loaded: u32 = net
        .actions()
        .filter_map(|a| match &a.kind {
            ActionKind::Insert { item, count, .. } if item == "iron-ore" => Some(*count),
            _ => None,
        })
        .sum();
    let taken: u32 = net
        .actions()
        .filter_map(|a| match &a.kind {
            ActionKind::Remove { item, count, .. } if item == "iron-plate" => Some(*count),
            _ => None,
        })
        .sum();
    assert_eq!(loaded, 20, "one ore per plate, dealt over the bank");
    assert_eq!(taken, 20, "and every plate comes back out");
    // **The claim is "no furnace was built", and it is now asserted as that.**
    // It read `stone_gathered(&net) == 0` until 2026-09-04. Stone is no longer
    // a proxy for a furnace: `Chop` swings at a `rock-huge` for this plan's
    // *coal*, and the same swing hands over twenty-four stone the plan never
    // asked for -- so the old spelling failed on a plan that adopts every
    // furnace it uses, for a reason that has nothing to do with furnaces. See
    // `have::Chop` on why the whole bill is credited.
    assert!(
        furnaces_placed(&net).is_empty(),
        "the bank was adopted, not built"
    );
    assert!(
        !net.actions()
            .any(|a| matches!(&a.kind, ActionKind::Craft { item, .. } if item == "stone-furnace")),
        "and nothing was crafted towards one"
    );
}

/// A wider bank must be a shorter *schedule*, not only a shorter lag — the lag
/// is a number in the network and the makespan is what a run actually costs.
#[test]
fn a_bank_shortens_the_schedule_and_never_lengthens_it() {
    let makespans: Vec<u32> = [0usize, 2, 4]
        .into_iter()
        .map(|standing| {
            let (net, state) = plan_for(world_with_standing_furnaces(standing), 20);
            schedule(&net, &state, &[BotId(1)])
                .expect("schedulable")
                .makespan
        })
        .collect();
    assert!(
        makespans[1] < makespans[0] && makespans[2] < makespans[1],
        "one bot, twenty plates: {makespans:?} should fall as the bank widens"
    );
}

/// **The single-bot path is not regressed, stated as its own property.**
///
/// The change can only ever *remove* work from a solo run: with nothing
/// standing it plans identically, and with furnaces standing it plans strictly
/// less. There is no input for which it plans more.
#[test]
fn the_single_bot_path_never_gets_more_work() {
    for plates in [1u32, 5, 20, 50] {
        let (bare, bare_state) = plan_for(Arc::new(fixture_world()), plates);
        let (bank, bank_state) = plan_for(world_with_standing_furnaces(4), plates);
        let bare_span = schedule(&bare, &bare_state, &[BotId(1)])
            .expect("schedulable")
            .makespan;
        let bank_span = schedule(&bank, &bank_state, &[BotId(1)])
            .expect("schedulable")
            .makespan;
        assert!(
            bank_span <= bare_span,
            "{plates} plates: a standing bank made the solo run longer, \
             {bank_span} against {bare_span}"
        );
        assert!(
            stone_gathered(&bank) <= stone_gathered(&bare),
            "{plates} plates: a standing bank should never cost more stone"
        );
    }
}

/// **Every precondition of a bank plan holds at the tick its action starts**,
/// replayed in time order rather than in assignment order.
///
/// The property a bank puts most at risk, because it is the first plan in this
/// crate where one chain holds several machines at once: `k` places, `3k`
/// transfers and `k` independent lags, whose only ordering is the edges
/// `smelt_steps` states. A take wired to the wrong furnace's inserts, or a lag
/// on the wrong edge, is invisible to every count above and shows up here as an
/// `EntityAt` or `HasItem` that does not hold when the action starts.
#[test]
fn a_bank_plan_replays_in_time_order() {
    for standing in [0usize, 2, 4] {
        let (net, state) = plan_for(world_with_standing_furnaces(standing), 20);
        let plan = schedule(&net, &state, &[BotId(1)]).expect("schedulable");
        common::assert_preconditions_hold_over_time(&net, &state, &plan);
    }
}

/// Determinism: the same world plans the same bank, twice, byte for byte.
#[test]
fn a_bank_plans_identically_twice() {
    let render = |net: &ActionNetwork| {
        net.actions()
            .map(|a| format!("{:?} {} {:?}", a.id, a.label, a.pre))
            .collect::<Vec<_>>()
    };
    let (first, _) = plan_for(world_with_standing_furnaces(4), 20);
    let (second, _) = plan_for(world_with_standing_furnaces(4), 20);
    assert_eq!(render(&first), render(&second));
}
