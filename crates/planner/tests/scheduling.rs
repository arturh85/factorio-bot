use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::{FactorioEntity, Position};
use factorio_bot_planner::action::{Action, ActionKind, Actor, Condition, Effect, InventorySlot};
use factorio_bot_planner::ids::ActionIdGen;
use factorio_bot_planner::schedule::arrival_point;
use factorio_bot_planner::{
    ActionId, ActionNetwork, BotId, PlanState, PlannerError, Schedule, ScheduledStep, StepKind,
    Ticks, schedule,
};
use std::sync::Arc;

fn state(bots: &[BotId]) -> PlanState {
    let mut s = PlanState::from_world(Arc::new(fixture_world()), bots);
    for bot in bots {
        s.set_position(*bot, Position::new(0., 0.));
    }
    s
}

fn ore_tile(s: &PlanState) -> Position {
    s.resource_patches("iron-ore")
        .first()
        .expect("fixture has iron ore")
        .elements
        .first()
        .expect("patch has tiles")
        .clone()
}

/// Mine `count` ore from `pos`, consuming it from the map.
fn mine_at(id_gen: &mut ActionIdGen, pos: &Position, count: u32) -> Action {
    Action {
        id: id_gen.next(),
        kind: ActionKind::Mine {
            pos: pos.clone(),
            item: "iron-ore".into(),
            count,
        },
        pre: vec![
            Condition::AtPosition {
                who: Actor::Role,
                pos: pos.clone(),
                radius: 3.0,
                min_radius: 0.0,
            },
            Condition::ResourceAvailable {
                pos: pos.clone(),
                item: "iron-ore".into(),
                count,
            },
        ],
        eff: vec![
            Effect::ConsumeResource {
                pos: pos.clone(),
                item: "iron-ore".into(),
                count,
            },
            Effect::GainItem {
                who: Actor::Role,
                item: "iron-ore".into(),
                count,
            },
        ],
        duration: 60,
        pinned: None,
        label: format!("mine {} iron-ore", count),
    }
}

/// Place a furnace at `pos`, requiring the tile to be free.
fn place_at(id_gen: &mut ActionIdGen, pos: &Position) -> Action {
    let furnace = FactorioEntity {
        name: "stone-furnace".into(),
        entity_type: "furnace".into(),
        position: pos.clone(),
        ..Default::default()
    };
    Action {
        id: id_gen.next(),
        kind: ActionKind::Place {
            entity: Box::new(furnace.clone()),
        },
        pre: vec![
            Condition::AtPosition {
                who: Actor::Role,
                pos: pos.clone(),
                radius: 10.0,
                min_radius: 0.0,
            },
            Condition::PositionFree { pos: pos.clone() },
        ],
        eff: vec![Effect::CreateEntity(Box::new(furnace))],
        duration: 30,
        pinned: None,
        label: "place stone-furnace".into(),
    }
}

/// Insert ore into the furnace at `pos`, which must already stand there.
fn insert_at(id_gen: &mut ActionIdGen, pos: &Position) -> Action {
    Action {
        id: id_gen.next(),
        kind: ActionKind::Insert {
            pos: pos.clone(),
            entity: "stone-furnace".into(),
            slot: InventorySlot::FurnaceSource,
            item: "iron-ore".into(),
            count: 1,
        },
        pre: vec![
            Condition::AtPosition {
                who: Actor::Role,
                pos: pos.clone(),
                radius: 10.0,
                min_radius: 0.0,
            },
            Condition::EntityAt {
                pos: pos.clone(),
                name: "stone-furnace".into(),
            },
        ],
        eff: vec![],
        duration: 20,
        pinned: None,
        label: "insert iron-ore".into(),
    }
}

fn free(id_gen: &mut ActionIdGen, duration: Ticks) -> Action {
    Action {
        id: id_gen.next(),
        kind: ActionKind::Craft {
            item: "iron-gear-wheel".into(),
            count: 1,
        },
        pre: vec![],
        eff: vec![],
        duration,
        pinned: None,
        label: "craft".into(),
    }
}

#[test]
fn two_bots_cannot_mine_the_same_exhausted_tile() {
    let bots = [BotId(1), BotId(2)];
    let s = state(&bots);
    let pos = ore_tile(&s);
    let available = s.resource_available(&pos, "iron-ore");
    assert!(available > 0);

    let mut id_gen = ActionIdGen::new();
    let mut net = ActionNetwork::new();
    // Together these demand more ore than the tile holds.
    net.add(mine_at(&mut id_gen, &pos, available));
    let second = net.add(mine_at(&mut id_gen, &pos, 1));

    let result = schedule(&net, &s, &bots);
    // This test is the whole evidence for having no reservation table, so it
    // names the action and the condition that must fail. Asserting only the
    // error variant would also be satisfied by an unrelated AtPosition
    // regression.
    match result {
        Err(PlannerError::PreconditionUnsatisfied {
            action, condition, ..
        }) => {
            assert_eq!(action, second, "the second miner is the one that fails");
            assert_eq!(
                condition,
                Condition::ResourceAvailable {
                    pos,
                    item: "iron-ore".into(),
                    count: 1,
                }
                .to_string(),
                "the exhausted tile must be what fails"
            );
        }
        other => panic!(
            "the second miner must fail on an exhausted tile, got {:?}",
            other.map(|s| s.makespan)
        ),
    }
}

#[test]
fn two_bots_cannot_place_at_the_same_position() {
    let bots = [BotId(1), BotId(2)];
    let s = state(&bots);
    let pos = Position::new(5., 5.);

    let mut id_gen = ActionIdGen::new();
    let mut net = ActionNetwork::new();
    net.add(place_at(&mut id_gen, &pos));
    let second = net.add(place_at(&mut id_gen, &pos));

    let result = schedule(&net, &s, &bots);
    // As above: the failing action and condition are the claim, not the variant.
    match result {
        Err(PlannerError::PreconditionUnsatisfied {
            action, condition, ..
        }) => {
            assert_eq!(action, second, "the second placement is the one that fails");
            assert_eq!(
                condition,
                Condition::PositionFree { pos }.to_string(),
                "the occupied tile must be what fails"
            );
        }
        other => panic!(
            "the second placement must fail on an occupied tile, got {:?}",
            other.map(|s| s.makespan)
        ),
    }
}

/// Replay a schedule in **time** order and assert the property the design asks
/// for: every precondition holds at the moment its action starts.
///
/// Replaying `result.steps` in vector order — which is assignment order — would
/// only re-run the checks `schedule()` already made, against the same
/// accumulated state, in the same sequence. Such a replay cannot fail. Ordering
/// by tick instead is what would expose two actions with no ordering edge
/// between them overlapping in time.
fn assert_preconditions_hold_over_time(
    net: &ActionNetwork,
    initial: &PlanState,
    result: &Schedule,
) {
    enum Event {
        Arrive(Position),
        Check(ActionId),
        Apply(ActionId),
    }

    // Phase 0 events land before phase 1 events at the same tick, so an effect
    // applied at tick t is visible to a precondition checked at tick t.
    let mut timeline: Vec<(Ticks, u8, BotId, Event)> = Vec::new();
    for step in &result.steps {
        match &step.what {
            StepKind::Walk { to, min_radius, .. } => {
                // A `Walk` names the annulus, not a point in it -- the point
                // is the game's to choose. What the *plan* believes it reached
                // is `arrival_point`, the same function `schedule()` advances
                // its own simulation with, so a replay that used anything else
                // would be checking a different plan than the one scheduled.
                timeline.push((
                    step.end,
                    0,
                    step.bot,
                    Event::Arrive(arrival_point(to, *min_radius)),
                ));
            }
            StepKind::Act { action, .. } => {
                timeline.push((step.end, 0, step.bot, Event::Apply(*action)));
                timeline.push((step.start, 1, step.bot, Event::Check(*action)));
            }
        }
    }
    timeline.sort_by_key(|(tick, phase, _, _)| (*tick, *phase));

    let mut replay = initial.fork();
    for (tick, _, bot, event) in &timeline {
        match event {
            Event::Arrive(to) => replay.set_position(*bot, to.clone()),
            Event::Check(id) => {
                let a = net.action(*id).expect("action exists");
                for condition in &a.pre {
                    assert!(
                        condition.holds(&replay, *bot),
                        "precondition `{}` of `{}` failed at tick {} for {}",
                        condition,
                        a.label,
                        tick,
                        bot
                    );
                }
            }
            Event::Apply(id) => {
                let a = net.action(*id).expect("action exists");
                for effect in &a.eff {
                    effect.apply(&mut replay, *bot).unwrap_or_else(|e| {
                        panic!("effect of `{}` failed at tick {}: {}", a.label, tick, e)
                    });
                }
            }
        }
    }
}

#[test]
fn every_precondition_holds_at_its_scheduled_time() {
    let bots = [BotId(1), BotId(2)];
    let s = state(&bots);
    let pos = ore_tile(&s);

    let mut id_gen = ActionIdGen::new();
    let mut net = ActionNetwork::new();
    net.add(mine_at(&mut id_gen, &pos, 1));
    net.add(mine_at(&mut id_gen, &pos, 1));
    net.add(place_at(&mut id_gen, &Position::new(5., 5.)));

    let result = schedule(&net, &s, &bots).expect("schedulable");
    assert_preconditions_hold_over_time(&net, &s, &result);
}

#[test]
fn the_property_still_holds_across_an_inferred_entity_edge() {
    // The design's Smelt shape: place a furnace, then insert ore into it. The
    // insert's EntityAt precondition is only satisfied by the place, and only
    // an inferred edge orders the two.
    let bots = [BotId(1), BotId(2)];
    let s = state(&bots);
    let furnace = Position::new(5., 5.);

    let mut id_gen = ActionIdGen::new();
    let mut net = ActionNetwork::new();
    let place = net.add(place_at(&mut id_gen, &furnace));
    let insert = net.add(insert_at(&mut id_gen, &furnace));
    net.infer_edges();
    assert_eq!(
        net.preds(insert),
        vec![(place, 0)],
        "the insert must be ordered after the place"
    );

    let result = schedule(&net, &s, &bots).expect("schedulable");
    assert_preconditions_hold_over_time(&net, &s, &result);
}

#[test]
#[should_panic(expected = "iron-ore available")]
fn the_time_ordered_replay_catches_an_overlapping_schedule() {
    // Proof that the property above has teeth. This hand-built schedule empties
    // the same tile twice, the second miner starting exactly when the first
    // finishes. `schedule()` never produces it. The old replay — which walked
    // `steps` in assignment order, re-running the checks the scheduler had
    // already made — would have accepted it; ordering by tick rejects it.
    let bots = [BotId(1), BotId(2)];
    let s = state(&bots);
    let pos = ore_tile(&s);
    let available = s.resource_available(&pos, "iron-ore");

    let mut id_gen = ActionIdGen::new();
    let mut net = ActionNetwork::new();
    let first = net.add(mine_at(&mut id_gen, &pos, available));
    let second = net.add(mine_at(&mut id_gen, &pos, available));

    let arrive = |bot| ScheduledStep {
        what: StepKind::Walk {
            to: pos.clone(),
            min_radius: 0.0,
            radius: 3.0,
        },
        bot,
        start: 0,
        end: 0,
    };
    let hand_built = Schedule {
        steps: vec![
            arrive(BotId(1)),
            ScheduledStep {
                what: StepKind::Act {
                    action: first,
                    label: "mine the tile dry".into(),
                },
                bot: BotId(1),
                start: 0,
                end: 60,
            },
            arrive(BotId(2)),
            ScheduledStep {
                what: StepKind::Act {
                    action: second,
                    label: "mine the same tile again".into(),
                },
                bot: BotId(2),
                start: 60,
                end: 120,
            },
        ],
        makespan: 120,
    };
    assert_preconditions_hold_over_time(&net, &s, &hand_built);
}

#[test]
fn more_bots_never_increase_the_makespan() {
    // 12 identical 50-tick actions divide evenly over 1, 2, 3 and 4 bots, so
    // the makespan is pinned exactly rather than bounded. `<=` would also be
    // satisfied by a scheduler that ignored every bot after the first.
    let mut id_gen = ActionIdGen::new();
    let mut net = ActionNetwork::new();
    for _ in 0..12 {
        net.add(free(&mut id_gen, 50));
    }

    for (count, expected) in [(1u8, 600 as Ticks), (2, 300), (3, 200), (4, 150)] {
        let bots: Vec<BotId> = (1..=count).map(BotId).collect();
        let result = schedule(&net, &state(&bots), &bots).expect("schedulable");
        assert_eq!(
            result.makespan, expected,
            "{} bots must fully share 600 ticks of work",
            count
        );
    }
}

#[test]
fn unequal_durations_are_shared_rather_than_split_by_count() {
    // 100 + 90 + 80 + 70 + 60 + 50 = 450 ticks of work over two bots. Splitting
    // the six actions three-and-three in id order would give 270; greedy list
    // scheduling interleaves them instead and lands at 240.
    let mut id_gen = ActionIdGen::new();
    let mut net = ActionNetwork::new();
    for duration in [100, 90, 80, 70, 60, 50] {
        net.add(free(&mut id_gen, duration));
    }

    let one = [BotId(1)];
    assert_eq!(
        schedule(&net, &state(&one), &one)
            .expect("schedulable")
            .makespan,
        450
    );

    let two = [BotId(1), BotId(2)];
    assert_eq!(
        schedule(&net, &state(&two), &two)
            .expect("schedulable")
            .makespan,
        240
    );
}

#[test]
fn a_cyclic_network_is_rejected_rather_than_looping() {
    let mut id_gen = ActionIdGen::new();
    let mut net = ActionNetwork::new();
    let a = net.add(free(&mut id_gen, 10));
    let b = net.add(free(&mut id_gen, 10));
    net.link(a, b, 0);
    net.link(b, a, 0);

    let bots = [BotId(1)];
    assert!(matches!(
        schedule(&net, &state(&bots), &bots),
        Err(PlannerError::CyclicNetwork(_))
    ));
}

#[test]
fn scheduling_terminates_on_a_deep_chain() {
    // A 200-long chain must not blow up or hang.
    let mut id_gen = ActionIdGen::new();
    let mut net = ActionNetwork::new();
    let mut previous = net.add(free(&mut id_gen, 1));
    for _ in 0..199 {
        let next = net.add(free(&mut id_gen, 1));
        net.link(previous, next, 0);
        previous = next;
    }
    let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let result = schedule(&net, &state(&bots), &bots).expect("schedulable");
    assert_eq!(result.makespan, 200, "a chain cannot be parallelised");
}

/// An action on `bot`, needing nothing, taking `duration` ticks.
fn pinned(id_gen: &mut ActionIdGen, label: &str, bot: BotId, duration: Ticks) -> Action {
    let mut action = free(id_gen, duration);
    action.pinned = Some(bot);
    action.label = label.into();
    action
}

fn act_start(plan: &Schedule, action: ActionId) -> Ticks {
    plan.steps
        .iter()
        .find_map(|s| match &s.what {
            StepKind::Act { action: id, .. } if *id == action => Some(s.start),
            _ => None,
        })
        .expect("the action is scheduled")
}

/// **`run-1788621697-14165`'s 1,724 plan ticks, as a fixture.** Four clients
/// at 1x: bot 1 stood free at 29,867 with nothing ready but a cell take whose
/// plates would exist at 35,706, and took it -- committing its timeline past
/// a 5,839-tick gap -- while bot 4's fourth ore insert into the furnace at
/// `[-6, -25]`, ready since 21,016, was still uncommitted in the round order
/// because its bound (50,713, the research hangs off that furnace's plates)
/// lost every round to bot 1's 45,146. When the insert was finally
/// committed, its successor -- bot 1's `take 16 iron-plate`, the start of the
/// steam-engine block, ready at 30,796 -- found bot 1 free at 47,436, and the
/// research waited on the engine until 48,026.
///
/// In miniature: bot 1's far take `F` waits on a 1,000-tick lag from its own
/// first step `L`; bot 2's short `P` gates bot 1's `E`, which carries the
/// long `R`. Ranked by bound alone, `F` (bound 1,020: bot 1 has nothing
/// else) is committed before `P` (bound 2,110: `R` hangs off it), `E` then
/// finds bot 1 free at 1,020, and the plan is 1,020 + 100 + 2,000 = 3,120.
/// Committing `P` first, because it finishes before `F` starts, puts `E` in
/// the gap and the plan is 2,110.
#[test]
fn a_bot_is_not_committed_past_a_gap_another_bots_finish_could_fill() {
    let bots = [BotId(1), BotId(2)];
    let mut id_gen = ActionIdGen::new();
    let mut net = ActionNetwork::new();
    let l = net.add(pinned(&mut id_gen, "L: start the far lag", BotId(1), 10));
    let f = net.add(pinned(&mut id_gen, "F: the far take", BotId(1), 10));
    net.link(l, f, 1000);
    let p = net.add(pinned(
        &mut id_gen,
        "P: the short predecessor",
        BotId(2),
        10,
    ));
    let e = net.add(pinned(&mut id_gen, "E: the engine block", BotId(1), 100));
    net.link(p, e, 0);
    let r = net.add(free(&mut id_gen, 2000));
    net.link(e, r, 0);

    let plan = schedule(&net, &state(&bots), &bots).expect("schedulable");
    assert!(
        act_start(&plan, e) < act_start(&plan, f),
        "E (ready at 10) belongs in the gap before F (ready at 1,010), got E at {} and F at {}",
        act_start(&plan, e),
        act_start(&plan, f)
    );
    assert_eq!(plan.makespan, 2110, "{plan:#?}");
}

/// **`run-1788621697-14165`'s 7,454 execution ticks, as a fixture.** The plan
/// held `research logistic-science-pack` (11,400) from 48,026 and `research
/// automation` (6,000) from 52,063, on two bots; a force researches one
/// technology at a time, so the second waited in the game's queue until
/// 61,346 and settled 13,154 after dispatch against its 6,000. Run 14 had
/// the same overlap for 1,534 ticks and paid the same way. Two researches
/// are a sequence, whoever runs them.
#[test]
fn two_researches_never_overlap_whoever_runs_them() {
    let bots = [BotId(1), BotId(2)];
    let mut id_gen = ActionIdGen::new();
    let mut net = ActionNetwork::new();
    let mut ids = Vec::new();
    for (tech, duration) in [("automation", 500 as Ticks), ("logistic-science-pack", 300)] {
        let mut action = free(&mut id_gen, duration);
        action.kind = ActionKind::Research { tech: tech.into() };
        action.label = format!("research {tech}");
        ids.push(net.add(action));
    }

    let plan = schedule(&net, &state(&bots), &bots).expect("schedulable");
    let spans: Vec<(Ticks, Ticks)> = plan
        .steps
        .iter()
        .filter_map(|s| match &s.what {
            StepKind::Act { action, .. } if ids.contains(action) => Some((s.start, s.end)),
            _ => None,
        })
        .collect();
    assert_eq!(spans.len(), 2);
    let (a, b) = (spans[0], spans[1]);
    assert!(
        a.1 <= b.0 || b.1 <= a.0,
        "the labs run one research at a time, got {a:?} and {b:?}"
    );
    assert_eq!(plan.makespan, 800, "{plan:#?}");
}
