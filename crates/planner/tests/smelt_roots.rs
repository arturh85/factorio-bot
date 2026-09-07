//! A smelt's roots, and where they land.
//!
//! `Smelt` decomposes into three roots that no ordering edge holds together:
//! the ore, the coal, and the furnace. Each is consumed by an action of its
//! own — one insert per ingredient, one place for the furnace — so nothing in
//! the network says the three have to be in the same pair of hands, and the
//! scheduler is free to mine the coal onto one bot and load the furnace from
//! another. With one bot that cannot show; with several smelts in flight over
//! four bots it is the plan's normal state, and the plan then dies on a
//! `HasItem` precondition it sized correctly and delivered to the wrong bot.

use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::Position;
use factorio_bot_planner::action::{ActionKind, Effect, InventorySlot};
use factorio_bot_planner::goal::{Goal, Holder};
use factorio_bot_planner::ids::ActionId;
use factorio_bot_planner::method::expand;
use factorio_bot_planner::method::have::registry_for;
use factorio_bot_planner::schedule::StepKind;
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

/// Two goals whose smelts differ in size, which is what makes the roots
/// scatter: identically sized subtrees can be paired off by luck, mismatched
/// ones cannot. Seven plates and nine gears is the smallest pair in this
/// fixture that produces smelts of three different sizes.
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
    let result = schedule(&net, &state, bots).expect("every smelt has a bot that can run it");
    (net, state, result)
}

/// Who works each **batch**, split by what the work *converges into*.
///
/// `.0` is the batch's **inventory-convergent** side: the ore that goes in and
/// the plates that come out. Those actions read and write one bot's pockets,
/// and that bot has to be the one the smelt was sized against.
///
/// `.1` is its **world-convergent** side: the placement and the fuel load.
/// Both produce map facts — the next action's precondition is
/// `Condition::EntityAt`, which names a position and no bot — so R3 hands them
/// to another bot when doing so takes real work off the taker. See
/// `crates/planner`'s `furnace_suppliers`.
///
/// # A batch, and not a furnace
///
/// It was keyed by furnace position until in-plan reuse: a plan committed a
/// furnace for its whole length, so one furnace *was* one batch and the two
/// keys could not be told apart. A later smelt may now queue behind an earlier
/// one in the same furnace, and the two are different smelts, sized against
/// different bots and legitimately run by different bots — so grouping by
/// position asserts something the plan never claimed, and this fixture really
/// does produce a furnace worked by bots 2 and 4.
///
/// A batch is **a take and the inserts feeding it**, which the network states
/// exactly: `smelt_steps` links every insert of a slot to that slot's own
/// removal and to no other. The placement is one edge further out — it feeds
/// the inserts, not the take — so it is found through them, and a batch that
/// *reused* a standing furnace finds none, which is correct: it did not build
/// one.
type BatchWorkers = (BTreeSet<BotId>, BTreeSet<BotId>);

fn by_batch(net: &ActionNetwork, result: &Schedule) -> BTreeMap<String, BatchWorkers> {
    let ran: BTreeMap<ActionId, BotId> = result
        .steps
        .iter()
        .filter_map(|step| match &step.what {
            StepKind::Act { action, .. } => Some((*action, step.bot)),
            _ => None,
        })
        .collect();
    let at = |id: ActionId| -> Option<Position> {
        match &net.action(id)?.kind {
            ActionKind::Place { entity } => Some(entity.position.clone()),
            ActionKind::Insert { pos, .. } | ActionKind::Remove { pos, .. } => Some(pos.clone()),
            _ => None,
        }
    };
    let mut out: BTreeMap<String, BatchWorkers> = BTreeMap::new();
    for action in net.actions() {
        let ActionKind::Remove { pos, .. } = &action.kind else {
            continue;
        };
        let mut workers = BatchWorkers::default();
        if let Some(bot) = ran.get(&action.id) {
            workers.0.insert(*bot);
        }
        for (insert, _) in net.preds(action.id) {
            if at(insert).as_ref() != Some(pos) {
                continue;
            }
            let Some(ActionKind::Insert { slot, .. }) = net.action(insert).map(|a| &a.kind) else {
                continue;
            };
            let side = if *slot == InventorySlot::Fuel {
                &mut workers.1
            } else {
                &mut workers.0
            };
            if let Some(bot) = ran.get(&insert) {
                side.insert(*bot);
            }
            // The placement, one edge further out: `smelt_steps` links a
            // furnace's own place to the inserts that need it to stand.
            //
            // **Only for the batch that built it.** A furnace's `Place` is a
            // predecessor of every insert ever made into it — `Condition::
            // EntityAt` is world-scoped, so `infer_edges` links it across
            // batches — and a batch that queued behind an earlier one did not
            // place anything. It is told apart by the release edge that let it
            // queue at all: an earlier `Remove` at this position among its
            // inserts' predecessors.
            let queued_behind = net.preds(insert).into_iter().any(|(earlier, _)| {
                matches!(
                    net.action(earlier).map(|a| &a.kind),
                    Some(ActionKind::Remove { .. })
                ) && at(earlier).as_ref() == Some(pos)
            });
            if queued_behind {
                continue;
            }
            for (place, _) in net.preds(insert) {
                if matches!(
                    net.action(place).map(|a| &a.kind),
                    Some(ActionKind::Place { .. })
                ) && at(place).as_ref() == Some(pos)
                    && let Some(bot) = ran.get(&place)
                {
                    workers.1.insert(*bot);
                }
            }
        }
        out.insert(format!("{pos} batch {:?}", action.id), workers);
    }
    out
}

/// The bug, stated as the property it breaks: one furnace, one pair of hands
/// **on the side of it that reads an inventory**.
///
/// Asserted per furnace rather than over the plan as a whole, because "the
/// plan schedules" is also true of a fix that welds every action onto a single
/// bot — see the parallelism assertion below, which that fix would fail.
///
/// # Why the placement and the fuel load are asserted separately
///
/// The original claim was "its ore, its coal and the hands that placed it are
/// one bot's", and two thirds of that is still exactly right: the ore comes
/// out of the inserting bot's pockets and the plates go into the taker's, so a
/// plan that mines onto one bot and loads from another dies on a `HasItem`
/// precondition it sized correctly and delivered to the wrong bot.
///
/// The other third was never load-bearing and R3 spends it. A furnace that
/// *stands* and a furnace that is *fuelled* are facts about the map, not about
/// anybody's inventory; nothing downstream reads the placer's pockets. So the
/// placement and the coal may be a different bot's errand — but still **one**
/// other bot's, because they are emitted as a single `Step::Owned` block whose
/// own bill was sized against that bot.
#[test]
fn every_furnace_is_placed_loaded_and_emptied_by_one_bot() {
    let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let (net, _, result) = plan(&bots);
    let batches = by_batch(&net, &result);
    assert!(
        batches.len() >= 3,
        "the fixture is supposed to produce several smelts, got {}",
        batches.len()
    );
    for (pos, (consumers, builders)) in &batches {
        assert_eq!(
            consumers.len(),
            1,
            "the {pos} has its ore put in and its plates taken out by \
             {consumers:?}; that side of a furnace is one inventory's, so its ore \
             and its output are one bot's"
        );
        assert!(
            builders.len() <= 1,
            "the {pos} was built and fuelled by {builders:?}; that side \
             may be somebody else's errand, but it is emitted as one owned block \
             and so is one bot's"
        );
    }
}

/// The roots themselves: whatever a bot puts into a furnace, that bot dug up.
///
/// This is the half `every_furnace_…` cannot see. A single bot could still be
/// loading a furnace out of ore another bot mined and handed over — which no
/// action in this plan does, because there is no hand-over action — so the
/// arithmetic has to close per bot: mined plus carried in, minus inserted,
/// never negative at any point of that bot's own sequence.
#[test]
fn no_bot_loads_a_furnace_with_ore_another_bot_dug() {
    let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let (net, state, result) = plan(&bots);

    let mut held: BTreeMap<(BotId, String), i64> = BTreeMap::new();
    for bot in bots {
        for item in ["iron-ore", "coal", "stone"] {
            held.insert(
                (bot, item.to_string()),
                i64::from(state.inventory_count(bot, item)),
            );
        }
    }

    let mut acts: Vec<&_> = result
        .steps
        .iter()
        .filter(|s| matches!(s.what, StepKind::Act { .. }))
        .collect();
    acts.sort_by_key(|s| (s.start, s.end));

    for step in acts {
        let StepKind::Act { action, .. } = &step.what else {
            unreachable!("filtered above")
        };
        let action = net.action(*action).expect("scheduled action is in the net");
        // **A chop is a gather too, and it gathers more than one item.** This
        // read `ActionKind::Mine` alone until 2026-09-04; `Chop` then moved
        // ahead of `Mine` and a bot's coal started arriving off a `rock-huge`
        // -- twenty-four coal and twenty-four stone in one action -- so the
        // ledger saw the insert and not the gather and reported a bot loading
        // a furnace out of nothing. The yield is read off the action's effects,
        // never from its `count`, which is a number of entities.
        let deltas: Vec<(String, i64)> = match &action.kind {
            ActionKind::Mine { item, count, .. } => vec![(item.clone(), i64::from(*count))],
            ActionKind::Chop { .. } => action
                .eff
                .iter()
                .filter_map(|e| match e {
                    Effect::GainItem { item, count, .. } => Some((item.clone(), i64::from(*count))),
                    _ => None,
                })
                .collect(),
            ActionKind::Insert { item, count, .. } => vec![(item.clone(), -i64::from(*count))],
            _ => continue,
        };
        for (item, delta) in deltas {
            let Some(running) = held.get_mut(&(step.bot, item.clone())) else {
                continue;
            };
            *running += delta;
            assert!(
                *running >= 0,
                "{} put {item} into a furnace that it never dug and was never given: \
                 running total {running} after `{}`",
                step.bot,
                action.label
            );
        }
    }
}

/// The other side of the ledger: welding a smelt's roots must not weld the
/// *plan*.
///
/// A fix that put every action on one bot would satisfy both assertions above
/// and destroy the only thing this planner exists for. Four bots, four
/// independent smelts, so at least three of them have work.
#[test]
fn welding_each_smelt_still_leaves_the_bots_working_in_parallel() {
    let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let (_, _, result) = plan(&bots);
    let working: BTreeSet<BotId> = result
        .steps
        .iter()
        .filter(|s| matches!(s.what, StepKind::Act { .. }))
        .map(|s| s.bot)
        .collect();
    assert!(
        working.len() >= 3,
        "only {working:?} were given work; welding a smelt's roots to one bot \
         must not weld the whole plan to one bot"
    );

    let (_, _, alone) = plan(&[BotId(1)]);
    assert!(
        result.makespan < alone.makespan,
        "four bots finished in {} ticks and one bot in {}; the roster is buying nothing",
        result.makespan,
        alone.makespan
    );
}
