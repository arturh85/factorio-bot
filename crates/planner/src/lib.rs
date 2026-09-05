pub mod action;
pub mod enclosure;
pub mod error;
pub mod goal;
pub mod ids;
pub mod method;
pub mod network;
pub mod render;
pub mod report;
pub mod schedule;
pub mod score;
pub mod state;

/// Test-only worlds. Not part of the crate's API: research needs a world with
/// a force, and the shared `fixture_world` has none.
#[cfg(test)]
mod test_world;

pub use action::{Action, ActionKind, Actor, Condition, Effect, InventorySlot};
pub use error::PlannerError;
pub use goal::{Goal, Holder, Site};
pub use ids::{ActionId, ActionIdGen, BotId, ChainId, ChainIdGen, ItemId, Ticks};
pub use method::have::default_registry;
pub use method::have::holds;
pub use method::have::registry_for;
pub use method::produce::DrainPolicy;
pub use method::{ExpansionCtx, GoalSite, Method, MethodRegistry, Step};
pub use method::{MAX_EXPANSION_DEPTH, expand, pick_chain_actor};
pub use network::{ActionNetwork, Edge};
pub use render::{graphviz, mermaid_gantt, ticks_to_timestamp};
pub use report::{BotReport, PlanReport};
pub use schedule::{
    Schedule, ScheduledStep, StepKind, WALK_TILES_PER_TICK, schedule, travel_ticks,
};
pub use score::{
    ChartingScore, MapScore, RUNG_1_ORES, ResourceScore, Verdict, WaterScore, WoodScore,
};
pub use state::{BotState, Buffer, PlanState};

/// Build the plan under every drain policy and keep the shorter schedule.
///
/// # The schedule is the arbiter, and nothing else can be
///
/// [`DrainPolicy`] decides whether a fragment waits on a cell this plan
/// already stood or mines its ore by hand, and that trade is only settled by
/// how much slack the roster has -- which is a property of the finished
/// schedule. Measured on `workspace/scripts/map.json` (2026-09-05):
/// `producing:logistic-science-pack:6` over four bots plans 72.0% busy and
/// wants the parallel policy (52,819 -> 49,051 ticks, 569 -> 452 actions,
/// 345 -> 273 ore mined by hand); the same goal over eight bots plans 51.8%
/// busy and wants the conservative one (49,229, against 55,093); and
/// `researched:automation` at 39.6% wants it so strongly that a flat bound
/// giving green its number cost automation 5,927 ticks. **No constant
/// available at expansion time separates those three**, so this asks the
/// question the only way it can be answered honestly: it builds both and
/// reads the makespans off.
///
/// Deterministic: policies are tried in `DrainPolicy`'s own order and a tie
/// keeps the earlier one, so [`DrainPolicy::Conservative`] -- the behaviour
/// every plan had before this existed -- wins any draw and this can never
/// return a longer plan than expansion alone would have.
///
/// A policy whose expansion or scheduling **fails** is skipped rather than
/// propagated, so a refusal under one policy cannot take down a run another
/// policy plans perfectly well; the error is returned only if every policy
/// fails, and then it is the first one's, which is the conservative
/// policy's and so the one a reader expects. That is not a hypothetical: a
/// raised drain bound has been seen to refuse `producing:logistic-science-pack:6`
/// over eight bots with `bot 2 has 27 iron-plate, needs 36` while the
/// conservative plan for the same goal expands and schedules.
///
/// The cost is one extra expansion and schedule per plan. That is wall-clock
/// time with the game's clock stopped (`Planner::plan_pause`), not game time,
/// and it buys a number the planner could not otherwise see.
pub fn plan_best(
    goals: &[Goal],
    state: &PlanState,
    registry: &MethodRegistry,
    chain_actor: BotId,
    roster: &[BotId],
) -> Result<(ActionNetwork, Schedule), PlannerError> {
    let mut best: Option<(ActionNetwork, Schedule)> = None;
    let mut first_error: Option<PlannerError> = None;
    for policy in [DrainPolicy::Conservative, DrainPolicy::Parallel] {
        let under = state.clone().with_drain_policy(policy);
        let attempt = expand(goals, &under, registry, chain_actor)
            .and_then(|net| schedule(&net, &under, roster).map(|plan| (net, plan)));
        match attempt {
            Ok((net, plan)) => {
                if best
                    .as_ref()
                    .is_none_or(|(_, best)| plan.makespan < best.makespan)
                {
                    best = Some((net, plan));
                }
            }
            Err(err) => {
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
        }
    }
    match (best, first_error) {
        (Some(found), _) => Ok(found),
        (None, Some(err)) => Err(err),
        (None, None) => unreachable!("the policy list is not empty"),
    }
}
