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
pub mod substance;

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
/// # The second pass is only paid when the policy actually decides something
///
/// The cost is one extra expansion and schedule *per policy that can differ*,
/// and on most goals none can. `DrainPolicy` is read in exactly one place --
/// `Drain::bound_from` -- and the two policies part only when a backlog
/// falls between one cell-build and one per standing cell, which needs two
/// cells standing at once and a backlog in that window. A goal that never
/// stands a second cell (`researched:automation`,
/// `producing:automation-science-pack:6`) and a goal that never smelts at all
/// (a `Goal::Built` block) expand identically under both, and the old code
/// paid full price to rediscover that.
///
/// So the expansion records whether any drain decision it took depended on
/// the policy ([`PlanState::drain_policy_mattered`]), and a completed
/// expansion that says no ends the search. That is a **proof, not a
/// heuristic**: expansion is a deterministic function of the state, the
/// policy is read nowhere else, and `schedule` does not read it at all, so
/// an expansion in which no drain decision turned on the policy is the
/// expansion every other policy would have produced, action for action.
/// Hence the chosen plan is byte-identical to what this returned before --
/// the saving is entirely in passes whose answer was already known.
///
/// A failed expansion proves nothing and ends nothing: the flag would then
/// describe the fragment of a plan that was built before the refusal, so a
/// policy that fails is skipped exactly as before and the next one is built
/// in full.
///
/// What remains is wall-clock time with the game's clock stopped
/// (`Planner::plan_pause`), not game time, and it buys a number the planner
/// could not otherwise see.
pub fn plan_best(
    goals: &[Goal],
    state: &PlanState,
    registry: &MethodRegistry,
    chain_actor: BotId,
    roster: &[BotId],
) -> Result<(ActionNetwork, Schedule), PlannerError> {
    let mut best: Option<(ActionNetwork, Schedule)> = None;
    let mut first_error: Option<PlannerError> = None;
    for policy in DrainPolicy::ALL {
        let under = state
            .clone()
            .with_drain_policy(policy)
            .with_fresh_policy_probe();
        let net = match expand(goals, &under, registry, chain_actor) {
            Ok(net) => net,
            Err(err) => {
                first_error.get_or_insert(err);
                continue;
            }
        };
        // Read before scheduling and only off a *complete* expansion: this
        // says every remaining policy expands to the network just built, so
        // whatever the schedule then makes of it, there is nothing left to
        // compare against. See this function's doc for why that is a proof.
        let settled = !under.drain_policy_mattered();
        match schedule(&net, &under, roster) {
            Ok(plan) => {
                if best
                    .as_ref()
                    .is_none_or(|(_, best)| plan.makespan < best.makespan)
                {
                    best = Some((net, plan));
                }
            }
            Err(err) => {
                first_error.get_or_insert(err);
            }
        }
        if settled {
            break;
        }
    }
    match (best, first_error) {
        (Some(found), _) => Ok(found),
        (None, Some(err)) => Err(err),
        (None, None) => unreachable!("the policy list is not empty"),
    }
}
