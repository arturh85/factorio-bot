//! Shared helpers for the planner's integration tests.
//!
//! `tests/scheduling.rs` carries its own copy of
//! `assert_preconditions_hold_over_time`. That is deliberate: that file is
//! pinned byte-identical by the hardening pass, so the helper could not be
//! lifted out of it. The two must be kept in step — if you change the replay
//! semantics here, change them there too, and vice versa.

use factorio_bot_core::types::Position;
use factorio_bot_planner::schedule::StepKind;
use factorio_bot_planner::{ActionId, ActionNetwork, BotId, PlanState, Schedule, Ticks};

/// Replay a schedule in **time** order and assert the property the design asks
/// for: every precondition holds at the moment its action starts.
///
/// Replaying `result.steps` in vector order — which is assignment order — would
/// only re-run the checks `schedule()` already made, against the same
/// accumulated state, in the same sequence. Such a replay cannot fail. Ordering
/// by tick instead is what would expose two actions with no ordering edge
/// between them overlapping in time.
pub fn assert_preconditions_hold_over_time(
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
            StepKind::Walk { to, .. } => {
                timeline.push((step.end, 0, step.bot, Event::Arrive(to.clone())));
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
