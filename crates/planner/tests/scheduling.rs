use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_core::types::{FactorioEntity, Position};
use factorio_bot_planner::action::{Action, ActionKind, Actor, Condition, Effect};
use factorio_bot_planner::ids::ActionIdGen;
use factorio_bot_planner::{
    schedule, ActionNetwork, BotId, PlanState, PlannerError, StepKind, Ticks,
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
fn mine_at(gen: &mut ActionIdGen, pos: &Position, count: u32) -> Action {
    Action {
        id: gen.next(),
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
fn place_at(gen: &mut ActionIdGen, pos: &Position) -> Action {
    let furnace = FactorioEntity {
        name: "stone-furnace".into(),
        entity_type: "furnace".into(),
        position: pos.clone(),
        ..Default::default()
    };
    Action {
        id: gen.next(),
        kind: ActionKind::Place {
            entity: Box::new(furnace.clone()),
        },
        pre: vec![
            Condition::AtPosition {
                who: Actor::Role,
                pos: pos.clone(),
                radius: 10.0,
            },
            Condition::PositionFree { pos: pos.clone() },
        ],
        eff: vec![Effect::CreateEntity(Box::new(furnace))],
        duration: 30,
        pinned: None,
        label: "place stone-furnace".into(),
    }
}

fn free(gen: &mut ActionIdGen, duration: Ticks) -> Action {
    Action {
        id: gen.next(),
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

    let mut gen = ActionIdGen::new();
    let mut net = ActionNetwork::new();
    // Together these demand more ore than the tile holds.
    net.add(mine_at(&mut gen, &pos, available));
    net.add(mine_at(&mut gen, &pos, 1));

    let result = schedule(&net, &s, &bots);
    assert!(
        matches!(result, Err(PlannerError::PreconditionUnsatisfied { .. })),
        "the second miner must fail on an exhausted tile, got {:?}",
        result.map(|s| s.makespan)
    );
}

#[test]
fn two_bots_cannot_place_at_the_same_position() {
    let bots = [BotId(1), BotId(2)];
    let s = state(&bots);
    let pos = Position::new(5., 5.);

    let mut gen = ActionIdGen::new();
    let mut net = ActionNetwork::new();
    net.add(place_at(&mut gen, &pos));
    net.add(place_at(&mut gen, &pos));

    let result = schedule(&net, &s, &bots);
    assert!(
        matches!(result, Err(PlannerError::PreconditionUnsatisfied { .. })),
        "the second placement must fail on an occupied tile"
    );
}

#[test]
fn every_precondition_holds_at_its_scheduled_time() {
    // Replaying the schedule step by step must never hit a false precondition.
    let bots = [BotId(1), BotId(2)];
    let s = state(&bots);
    let pos = ore_tile(&s);

    let mut gen = ActionIdGen::new();
    let mut net = ActionNetwork::new();
    net.add(mine_at(&mut gen, &pos, 1));
    net.add(mine_at(&mut gen, &pos, 1));
    net.add(place_at(&mut gen, &Position::new(5., 5.)));

    let result = schedule(&net, &s, &bots).expect("schedulable");

    let mut replay = s.fork();
    for step in &result.steps {
        match &step.what {
            StepKind::Walk { to } => replay.set_position(step.bot, to.clone()),
            StepKind::Act { action, .. } => {
                let a = net.action(*action).expect("action exists");
                for condition in &a.pre {
                    assert!(
                        condition.holds(&replay, step.bot),
                        "precondition `{}` of `{}` failed on replay",
                        condition,
                        a.label
                    );
                }
                for effect in &a.eff {
                    effect.apply(&mut replay, step.bot).expect("effect applies");
                }
            }
        }
    }
}

#[test]
fn more_bots_never_increase_the_makespan() {
    let mut gen = ActionIdGen::new();
    let mut net = ActionNetwork::new();
    for _ in 0..12 {
        net.add(free(&mut gen, 50));
    }

    let mut previous = Ticks::MAX;
    for count in 1..=4u8 {
        let bots: Vec<BotId> = (1..=count).map(BotId).collect();
        let result = schedule(&net, &state(&bots), &bots).expect("schedulable");
        assert!(
            result.makespan <= previous,
            "{} bots gave makespan {} against {} for {} bots",
            count,
            result.makespan,
            previous,
            count - 1
        );
        previous = result.makespan;
    }
}

#[test]
fn a_cyclic_network_is_rejected_rather_than_looping() {
    let mut gen = ActionIdGen::new();
    let mut net = ActionNetwork::new();
    let a = net.add(free(&mut gen, 10));
    let b = net.add(free(&mut gen, 10));
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
    let mut gen = ActionIdGen::new();
    let mut net = ActionNetwork::new();
    let mut previous = net.add(free(&mut gen, 1));
    for _ in 0..199 {
        let next = net.add(free(&mut gen, 1));
        net.link(previous, next, 0);
        previous = next;
    }
    let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let result = schedule(&net, &state(&bots), &bots).expect("schedulable");
    assert_eq!(result.makespan, 200, "a chain cannot be parallelised");
}
