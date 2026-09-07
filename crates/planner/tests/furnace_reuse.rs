//! A later smelt in the *same* plan queues behind an earlier one's furnace.
//!
//! # The gate
//!
//! `tests/furnace_bank.rs` closed the half of this that crosses plans: a plan
//! run twice must not re-bill the furnaces the first run left standing. The
//! half *inside* one plan stayed open, and it was open on purpose —
//! `smelt_steps` committed every furnace it touched
//! (`PlanState::commit_machine`) and nothing ever released the commitment, so
//! a plan needed **as many furnaces as it had `Smelt` goals**.
//!
//! Measured on `run-1788497495-79997`: 28 independent hand-smelts for 276 iron
//! ore and 46 copper, several of them smelting a single ore, each with a
//! furnace of its own. Cross-plan reuse still worked, so an epoch added ~13
//! rather than starting over — which is why it read as churn rather than as a
//! leak.
//!
//! It became the binding constraint when red science alone put **42 stone
//! furnaces** on seed `31337`, spread `x −18..33, y −49..−12`, on a map whose
//! iron ore is 18.4 tiles from spawn. They landed on and around the patch the
//! green-science cell then needed, and `Producing` refused after one iteration
//! with *no room for a iron-ore cell within 12 tiles of the patch* —
//! `produce::CELL_SITES_RESERVED` reserves six sites and 42 furnaces overwhelm
//! six.
//!
//! # What makes reuse legal
//!
//! One stated edge and one withheld promise. The edge runs from the earlier
//! batch's **take** to every insert of the later one, so the furnace is empty
//! before anything goes back into it; `infer_edges` cannot supply it, because
//! a take's effect is `GainItem`, which satisfies no precondition of an insert,
//! and no condition in this crate says "this machine is empty". The promise is
//! `PlanState::machine_queue`, which is written only for a batch whose take
//! provably drains the furnace and is read only by a smelt of the same item.

mod common;

use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::Position;
use factorio_bot_planner::action::{ActionKind, InventorySlot};
use factorio_bot_planner::goal::{Goal, Holder};
use factorio_bot_planner::ids::ActionId;
use factorio_bot_planner::method::expand;
use factorio_bot_planner::method::have::registry_for;
use factorio_bot_planner::{ActionNetwork, BotId, PlanState, Schedule, schedule};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

fn have(item: &str, count: u32) -> Goal {
    Goal::Have {
        item: item.into(),
        count,
        whose: Holder::Anyone,
        via: None,
    }
}

/// The same pair `tests/smelt_roots.rs` uses, and for the same reason: two
/// goals whose smelts differ in size produce several independent smelts on one
/// ore patch, which is exactly the shape that used to cost a furnace each.
fn mismatched_smelts() -> Goal {
    Goal::All(vec![have("iron-plate", 7), have("iron-gear-wheel", 9)])
}

fn plan(bots: &[BotId]) -> (ActionNetwork, PlanState, Schedule) {
    let state = PlanState::from_world(Arc::new(fixture_world()), bots);
    let net = expand(
        &[mismatched_smelts()],
        &state,
        &registry_for(bots),
        BotId(1),
    )
    .expect("the goals expand");
    let result = schedule(&net, &state, bots).expect("the plan schedules");
    (net, state, result)
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

/// Where an action acts, for the three kinds that address a machine.
fn site(net: &ActionNetwork, id: ActionId) -> Option<Position> {
    match &net.action(id)?.kind {
        ActionKind::Place { entity } => Some(entity.position.clone()),
        ActionKind::Insert { pos, .. } | ActionKind::Remove { pos, .. } => Some(pos.clone()),
        _ => None,
    }
}

/// One batch: the take, and the inserts the network says feed *it*.
///
/// `smelt_steps` links every insert of a bank slot to that slot's own removal
/// and to no other, so the take is the batch's identity and its predecessors
/// are its membership. Nothing here reads emission order or `ActionId`
/// arithmetic — the ids of a reused furnace's two batches are not adjacent.
struct Batch {
    pos: Position,
    take: ActionId,
    inserts: Vec<ActionId>,
}

fn batches(net: &ActionNetwork) -> Vec<Batch> {
    net.actions()
        .filter_map(|action| {
            let ActionKind::Remove { pos, .. } = &action.kind else {
                return None;
            };
            let inserts = net
                .preds(action.id)
                .into_iter()
                .map(|(id, _)| id)
                .filter(|id| site(net, *id).as_ref() == Some(pos))
                .filter(|id| {
                    matches!(
                        net.action(*id).map(|a| &a.kind),
                        Some(ActionKind::Insert { .. })
                    )
                })
                .collect();
            Some(Batch {
                pos: pos.clone(),
                take: action.id,
                inserts,
            })
        })
        .collect()
}

fn when(result: &Schedule, id: ActionId) -> (u32, u32) {
    result
        .steps
        .iter()
        .find_map(|step| match &step.what {
            factorio_bot_planner::schedule::StepKind::Act { action, .. } if *action == id => {
                Some((step.start, step.end))
            }
            _ => None,
        })
        .expect("every action in the net is scheduled")
}

// ---------------------------------------------------------------------------
// The fix
// ---------------------------------------------------------------------------

/// **One bot, one furnace, several smelts.** The whole point.
///
/// `have::patch_furnace_budget` is one furnace per bot per ore patch, so a solo
/// plan builds exactly one however many smelts it contains, and every one of
/// them queues in it.
#[test]
fn a_solo_plan_smelts_everything_through_one_furnace() {
    let (net, _, _) = plan(&[BotId(1)]);
    let built = furnaces_placed(&net);
    assert_eq!(
        built.len(),
        1,
        "one bot, one ore patch, one furnace; got {built:?}"
    );
    let loaded: BTreeSet<String> = batches(&net)
        .iter()
        .map(|batch| batch.pos.to_string())
        .collect();
    assert_eq!(
        loaded.len(),
        1,
        "every batch runs in the one furnace; got {loaded:?}"
    );
    assert!(
        batches(&net).len() >= 2,
        "the fixture is supposed to produce several smelts, got {}",
        batches(&net).len()
    );
}

/// **The budget is the roster**, so four bots get four furnaces and their
/// independent smelts do not queue behind each other.
///
/// A bot loads and unloads one furnace at a time, which is the whole reason the
/// bound is the roster rather than a constant. Measured the other way round on
/// `workspace/scripts/map.json`: four independent ten-plate smelts, one per
/// bot, cost 10,899 ticks on one furnace, 7,090 on two and 4,971 on four.
#[test]
fn the_furnace_budget_grows_with_the_roster() {
    let solo = furnaces_placed(&plan(&[BotId(1)]).0).len();
    let fleet = furnaces_placed(&plan(&[BotId(1), BotId(2), BotId(3), BotId(4)]).0).len();
    assert_eq!(solo, 1, "a roster of one gets one furnace at the patch");
    assert!(
        fleet > solo,
        "four bots must get more than one bot's furnace, got {fleet} against {solo}"
    );
    assert!(
        fleet <= 4,
        "and no more than one per bot, got {fleet} furnaces"
    );
}

/// **The edge that makes it legal, asserted on the network.**
///
/// Every batch but the first in a furnace has each of its inserts ordered after
/// an earlier take *in that same furnace*. Stated on the edges rather than on
/// the schedule, because a schedule that happens to come out in the right order
/// is not the same claim: without the edge the scheduler is free to reorder,
/// and the reordering is legal by every other constraint in the plan.
#[test]
fn every_queued_batch_waits_for_the_take_that_empties_the_furnace() {
    let (net, _, result) = plan(&[BotId(1)]);
    let all = batches(&net);
    let mut by_furnace: BTreeMap<String, Vec<&Batch>> = BTreeMap::new();
    for batch in &all {
        by_furnace
            .entry(batch.pos.to_string())
            .or_default()
            .push(batch);
    }
    let mut queued_batches = 0;
    for (pos, mut group) in by_furnace {
        // In the order they actually run, which is what "earlier" has to mean.
        group.sort_by_key(|batch| when(&result, batch.take));
        for (index, batch) in group.iter().enumerate().skip(1) {
            queued_batches += 1;
            let earlier: BTreeSet<ActionId> =
                group[..index].iter().map(|batch| batch.take).collect();
            for insert in &batch.inserts {
                let waits = net
                    .preds(*insert)
                    .into_iter()
                    .any(|(pred, _)| earlier.contains(&pred));
                assert!(
                    waits,
                    "the insert {insert:?} of batch {:?} in the furnace at {pos} is not \
                     ordered after any earlier take there ({earlier:?}); without that edge \
                     the plan may load a furnace that still holds the batch before it",
                    batch.take
                );
            }
        }
    }
    assert!(
        queued_batches > 0,
        "this fixture is supposed to make at least one batch queue behind another"
    );
}

/// **And what the edge buys, asserted on the schedule.**
///
/// No furnace is loaded while a batch is still in it. This is the property the
/// game enforces and the plan used to guarantee by never reusing anything: an
/// insert of copper into a furnace holding iron plates is refused outright, and
/// a refusal is what the permanent commitment existed to avoid.
#[test]
fn no_furnace_is_loaded_before_the_batch_in_it_is_taken() {
    let (net, _, result) = plan(&[BotId(1), BotId(2), BotId(3), BotId(4)]);
    let all = batches(&net);
    let mut by_furnace: BTreeMap<String, Vec<&Batch>> = BTreeMap::new();
    for batch in &all {
        by_furnace
            .entry(batch.pos.to_string())
            .or_default()
            .push(batch);
    }
    for (pos, mut group) in by_furnace {
        group.sort_by_key(|batch| when(&result, batch.take));
        for pair in group.windows(2) {
            let taken = when(&result, pair[0].take).1;
            let first_load = pair[1]
                .inserts
                .iter()
                .map(|id| when(&result, *id).0)
                .min()
                .expect("a batch has at least one insert");
            assert!(
                first_load >= taken,
                "the furnace at {pos} is loaded again at {first_load} but the batch \
                 before it is not taken until {taken}"
            );
        }
    }
}

/// **Every precondition still holds at the tick its action starts.**
///
/// The replay check, run over the reuse path. A release edge wired to the wrong
/// furnace's take, or dropped by `infer_edges` closing a cycle, is invisible to
/// every count above and shows up here as an `EntityAt` or `HasItem` that does
/// not hold when the action starts.
#[test]
fn a_reusing_plan_replays_in_time_order() {
    for roster in [vec![BotId(1)], vec![BotId(1), BotId(2), BotId(3), BotId(4)]] {
        let (net, state, result) = plan(&roster);
        common::assert_preconditions_hold_over_time(&net, &state, &result);
    }
}

/// Determinism: reuse reads mutable state — which furnace is committed, how
/// much is queued in it — so the same world must plan the same queue twice.
#[test]
fn a_reusing_plan_is_identical_on_a_second_expansion() {
    let render = |net: &ActionNetwork| {
        let mut out: Vec<String> = net
            .actions()
            .map(|a| format!("{:?} {} {:?}", a.id, a.label, a.pre))
            .collect();
        for action in net.actions() {
            for (pred, lag) in net.preds(action.id) {
                out.push(format!("{pred:?} -> {:?} lag {lag}", action.id));
            }
        }
        out
    };
    let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let (first, _, first_plan) = plan(&bots);
    let (second, _, second_plan) = plan(&bots);
    assert_eq!(render(&first), render(&second));
    assert_eq!(first_plan.makespan, second_plan.makespan);
    assert_eq!(first_plan.steps, second_plan.steps);
}

/// **A furnace with fuel in it is still one furnace.** The fuel load is ordered
/// with the rest of the batch rather than left free, so a bank slot's whole
/// membership travels together.
#[test]
fn a_queued_batchs_fuel_waits_with_its_ore() {
    let (net, _, result) = plan(&[BotId(1)]);
    let all = batches(&net);
    let fuel: Vec<ActionId> = all
        .iter()
        .flat_map(|batch| batch.inserts.iter().copied())
        .filter(|id| {
            matches!(
                net.action(*id).map(|a| &a.kind),
                Some(ActionKind::Insert {
                    slot: InventorySlot::Fuel,
                    ..
                })
            )
        })
        .collect();
    assert!(!fuel.is_empty(), "a smelt fuels the furnace it loads");
    // Every fuel load is a member of exactly one batch, so it is covered by
    // `every_queued_batch_waits_…` above; this asserts the membership itself,
    // which is what makes that coverage true.
    for id in fuel {
        let owner = all
            .iter()
            .filter(|batch| batch.inserts.contains(&id))
            .count();
        assert_eq!(owner, 1, "the fuel load {id:?} belongs to one batch");
        let _ = when(&result, id);
    }
}

// ---------------------------------------------------------------------------
// Whose queue a smelt joins
// ---------------------------------------------------------------------------

/// A furnace's whole queue, per position: the batches in it in the order they
/// run, with the bot that takes each.
fn queues(net: &ActionNetwork, result: &Schedule) -> BTreeMap<String, Vec<(BotId, ActionId)>> {
    let taker = |id: ActionId| {
        result
            .steps
            .iter()
            .find_map(|step| match &step.what {
                factorio_bot_planner::schedule::StepKind::Act { action, .. } if *action == id => {
                    Some(step.bot)
                }
                _ => None,
            })
            .expect("every take is scheduled")
    };
    let mut by_furnace: BTreeMap<String, Vec<(BotId, ActionId)>> = BTreeMap::new();
    let mut all = batches(net);
    all.sort_by_key(|batch| when(result, batch.take));
    for batch in all {
        by_furnace
            .entry(batch.pos.to_string())
            .or_default()
            .push((taker(batch.take), batch.take));
    }
    by_furnace
}

fn plan_goals(goals: Vec<Goal>, bots: &[BotId]) -> (ActionNetwork, PlanState, Schedule) {
    let state = PlanState::from_world(Arc::new(fixture_world()), bots);
    let net = expand(&[Goal::All(goals)], &state, &registry_for(bots), BotId(1))
        .expect("the goals expand");
    let result = schedule(&net, &state, bots).expect("the plan schedules");
    (net, state, result)
}

fn have_for(item: &str, count: u32, bot: BotId) -> Goal {
    Goal::Have {
        item: item.into(),
        count,
        whose: Holder::Bot(bot),
        via: None,
    }
}

/// **A bot with no furnace of its own on the patch builds one**, even when
/// the patch is already at the roster's budget, rather than queueing behind
/// another bot's batch.
///
/// The defect this pins, measured on `producing:logistic-science-pack:6`
/// against `workspace/scripts/map.json`: bot 1's first smelts built every
/// furnace the budget allowed, so bot 4's nine drill plates queued 9,848
/// ticks behind bot 1's ladder on the furnace at `[-34, -32]`, its drill
/// stood at 44,658 instead of ~33,000, and its 78-plate cell take -- the
/// step green's research waits on -- slid with it. A stone furnace is five
/// stone and thirty ticks; the queue it replaces is thousands.
///
/// Two bots, budget two. Bot 1's two smelts fill the budget greedily, as
/// they always did; bot 2's smelt then gets a furnace of its own instead of
/// a place in bot 1's queue, and that furnace is bot 2's own errand -- the
/// point of it is independence from the other bot's timeline, so its
/// placement is never handed over.
#[test]
fn a_bot_with_no_furnace_of_its_own_builds_one_rather_than_queueing() {
    let bots = [BotId(1), BotId(2)];
    let (net, _, result) = plan_goals(
        vec![
            have_for("iron-plate", 7, BotId(1)),
            have_for("iron-gear-wheel", 9, BotId(1)),
            have_for("iron-plate", 6, BotId(2)),
        ],
        &bots,
    );
    let built = furnaces_placed(&net);
    assert_eq!(
        built.len(),
        3,
        "two for bot 1's bank at the budget, one of bot 2's own; got {built:?}"
    );
    let by_furnace = queues(&net, &result);
    let bot_2s: Vec<(&String, &Vec<(BotId, ActionId)>)> = by_furnace
        .iter()
        .filter(|(_, queue)| queue.iter().any(|(bot, _)| *bot == BotId(2)))
        .collect();
    assert_eq!(
        bot_2s.len(),
        1,
        "bot 2 smelts in one furnace: {by_furnace:?}"
    );
    let (pos, queue) = bot_2s[0];
    assert!(
        queue.iter().all(|(bot, _)| *bot == BotId(2)),
        "bot 2's furnace at {pos} carries nobody else's batch: {queue:?}"
    );
    let placed_by = result
        .steps
        .iter()
        .find_map(|step| match &step.what {
            factorio_bot_planner::schedule::StepKind::Act { action, .. }
                if site(&net, *action).map(|p| p.to_string()).as_ref() == Some(pos)
                    && matches!(
                        net.action(*action).map(|a| &a.kind),
                        Some(ActionKind::Place { .. })
                    ) =>
            {
                Some(step.bot)
            }
            _ => None,
        })
        .expect("bot 2's furnace is placed");
    assert_eq!(
        placed_by,
        BotId(2),
        "a furnace built so that a bot need not wait on another is that bot's own errand"
    );
}

/// **A smelt queues behind its own batch before anybody else's**, whatever
/// the load. Waiting on a take the same bot performs costs that bot nothing
/// it was not already paying -- its actions are serial -- while a batch of
/// another bot's puts the wait on that bot's timeline, which the expansion
/// cannot see and the schedule then pays for.
///
/// Bot 2's first smelt gets a furnace of its own and loads it heavily; its
/// second is small, and every one of bot 1's furnaces carries less. Least-
/// loaded alone would send it to bot 1's; it goes behind its own.
#[test]
fn a_smelt_queues_behind_its_own_batch_before_a_lighter_furnace_of_another_bots() {
    let bots = [BotId(1), BotId(2)];
    let (net, _, result) = plan_goals(
        vec![
            have_for("iron-plate", 4, BotId(1)),
            have_for("iron-gear-wheel", 3, BotId(1)),
            have_for("iron-plate", 20, BotId(2)),
            have_for("copper-plate", 1, BotId(2)),
            have_for("iron-plate", 22, BotId(2)),
        ],
        &bots,
    );
    let by_furnace = queues(&net, &result);
    let bot_2s: Vec<(&String, &Vec<(BotId, ActionId)>)> = by_furnace
        .iter()
        .filter(|(_, queue)| {
            queue
                .iter()
                .any(|(bot, take)| *bot == BotId(2) && is_iron_take(&net, *take))
        })
        .collect();
    assert_eq!(
        bot_2s.len(),
        1,
        "both of bot 2's iron smelts run in bot 2's own furnace: {by_furnace:?}"
    );
    let (_, queue) = bot_2s[0];
    let iron: Vec<_> = queue
        .iter()
        .filter(|(_, take)| is_iron_take(&net, *take))
        .collect();
    assert_eq!(iron.len(), 2, "two batches of bot 2's, queued: {queue:?}");
    assert!(
        iron.iter().all(|(bot, _)| *bot == BotId(2)),
        "and nobody else's between them: {queue:?}"
    );
}

fn is_iron_take(net: &ActionNetwork, id: ActionId) -> bool {
    matches!(
        net.action(id).map(|a| &a.kind),
        Some(ActionKind::Remove { item, .. }) if item == "iron-plate"
    )
}

/// **The two rules above keep a plan deterministic**: which furnace is a
/// bot's own is read off mutable state, so the same goals must queue the same
/// way twice.
#[test]
fn whose_queue_a_smelt_joins_is_identical_on_a_second_expansion() {
    let bots = [BotId(1), BotId(2)];
    let goals = || {
        vec![
            have_for("iron-plate", 7, BotId(1)),
            have_for("iron-gear-wheel", 9, BotId(1)),
            have_for("iron-plate", 6, BotId(2)),
            have_for("iron-plate", 3, BotId(2)),
        ]
    };
    let (first, _, first_plan) = plan_goals(goals(), &bots);
    let (second, _, second_plan) = plan_goals(goals(), &bots);
    assert_eq!(queues(&first, &first_plan), queues(&second, &second_plan));
    assert_eq!(first_plan.steps, second_plan.steps);
}
