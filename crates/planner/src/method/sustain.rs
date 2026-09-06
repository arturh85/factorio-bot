//! The method that claims [`Goal::Sustain`] — the capacity half, and a named
//! refusal for the half that does not exist yet.
//!
//! Design note: `docs/superpowers/notes/2026-09-06-standing-goals.md`.
//!
//! # What a standing goal decomposes to, and what is missing
//!
//! ```text
//! Sustain{item, rate, window}
//! ├── capacity   Producing{item, rate}            — reuse, unchanged
//! ├── supply     for each input of each machine: a STANDING deliverer
//! ├── power      Condition::Powered for every electric machine
//! └── source     enough resource under the drills to last `window`
//! ```
//!
//! **Only the first line exists.** This method plans it, and then refuses the
//! rest by name. That is deliberate and it is the whole content of the first
//! rung: every input of a stage-1 burner cell arrives as an `insert` action,
//! and an `insert` is a bot's hands. The primitive that would belt them in —
//! [`crate::method::connect`]'s `connect_steps` — exists, is tested, and **has
//! no caller anywhere in the tree**; giving it one is the second rung and it
//! is a real risk to state, because nothing but its own fixtures has ever
//! exercised its geometry.
//!
//! So this method's contract is narrow and honest:
//!
//! * capacity does not stand → plan it (the `Producing` subgoal, unchanged),
//!   and say nothing about supply yet, because a cell that is not built cannot
//!   have its supply planned around it;
//! * capacity stands → [`PlannerError::SustainSupplyNotStanding`], naming the
//!   inputs with no standing deliverer.
//!
//! Neither branch is ever an empty network, which is the obligation
//! [`crate::method::have::holds`] places on this method by answering `None`:
//! an empty network is this planner's word for *done*, and a standing rate is
//! exactly what it cannot know is done. `crates/planner/tests/standing_goals.rs`
//! pins it.
//!
//! # Idempotence
//!
//! The capacity subgoal is `Goal::Producing`, whose method
//! ([`crate::method::produce::BuildCell`]) already subtracts what stands
//! before planning anything, so a replan over a half-built cell builds the
//! remainder rather than a second cell. `tests/standing_site_reuse.rs` is the
//! precedent and the warning; this method adds no siting of its own, so it
//! inherits that property rather than having to re-establish it.

use crate::error::PlannerError;
use crate::goal::Goal;
use crate::method::{ExpansionCtx, Method, Step};
use crate::state::PlanState;

/// Claims [`Goal::Sustain`].
pub struct Sustain;

/// The inputs a stage-1 cell for `item` consumes that nothing standing
/// delivers.
///
/// Two, always, and both are hand-carried today: the **ore** the drill mines
/// (delivered drill→furnace by the drill's own drop point, which *is*
/// standing — but the drill's own fuel is not) and the **coal** both machines
/// burn. Fuel is an input like any other, and it is the one the first rung is
/// really about: one coal is ~1,600 ticks in a drill and ~2,666 in a furnace,
/// so a burner cell can satisfy no window longer than a single fuel load
/// unless coal is belted in.
///
/// Returned as a rendered list rather than a structure because the only
/// consumer is a refusal message; when `method::connect` gains a caller this
/// becomes a real query and should be rewritten then, not decorated now.
fn unfed_inputs(state: &PlanState, item: &str) -> String {
    match crate::method::produce::cell_spec(state, item) {
        Some(spec) => format!("{} and coal", spec.ore),
        None => "its inputs".to_string(),
    }
}

impl Method for Sustain {
    fn name(&self) -> &'static str {
        "sustain"
    }

    /// Claims [`Goal::Sustain`] and nothing else.
    ///
    /// Unconditionally — including for an item no cell can make and for a
    /// window of zero. A false answer here would produce
    /// `NoApplicableMethod`, which says only "nobody understood this"; the
    /// refusals this method raises instead say *which* half is missing, and
    /// the supervisor turns either into a `stuck` milestone carrying the
    /// planner's own code. The same reasoning as
    /// [`crate::method::produce::BuildCell`]'s, one rung further out.
    fn applicable(&self, goal: &Goal, _state: &PlanState) -> bool {
        matches!(goal, Goal::Sustain { .. })
    }

    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        let Goal::Sustain {
            item,
            per_minute,
            window_ticks,
        } = goal
        else {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        };
        // The capacity half, asked of the world exactly as `Goal::Producing`'s
        // own `holds` does -- one predicate in one place, so the two cannot
        // drift apart and report different amounts of factory standing.
        let capacity_stands =
            crate::method::produce::holds_producing(&ctx.state, item, *per_minute)
                || crate::method::assemble::holds_assembling(&ctx.state, item, *per_minute);
        if capacity_stands {
            return Err(PlannerError::SustainSupplyNotStanding {
                item: item.clone(),
                per_minute: *per_minute,
                window_ticks: *window_ticks,
                inputs: unfed_inputs(&ctx.state, item),
            });
        }
        Ok(vec![Step::Subgoal(Goal::Producing {
            item: item.clone(),
            per_minute: *per_minute,
        })])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goal::Goal;
    use crate::ids::BotId;
    use crate::{PlanState, expand, holds, registry_for};
    use factorio_bot_core::test_utils::fixture_world;
    use std::sync::Arc;

    fn state() -> PlanState {
        PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)])
    }

    fn goal() -> Goal {
        Goal::Sustain {
            item: "iron-plate".into(),
            per_minute: 15,
            window_ticks: 7200,
        }
    }

    /// A sustain goal is never `already-satisfied`, so the method is always
    /// reached and always answers.
    #[test]
    fn the_method_is_reached_because_nothing_claims_the_goal_holds() {
        assert_eq!(holds(&goal(), &state()), None);
        assert!(Sustain.applicable(&goal(), &state()));
    }

    /// Capacity that does not stand is planned, not refused.
    #[test]
    fn a_cell_that_does_not_stand_is_planned() {
        let roster = [BotId(1)];
        let net = expand(&[goal()], &state(), &registry_for(&roster), BotId(1))
            .expect("the fixture world has iron ore and no cell on it");
        assert!(
            !net.is_empty(),
            "an empty network is this planner's word for `done`"
        );
    }

    /// And capacity that stands is refused **by name**, naming the fuel.
    ///
    /// This is the branch the first rung produces once its cell is built, and
    /// the sentence is the deliverable: the planner saying which input has no
    /// standing deliverer, rather than planning a factory whose measurement
    /// will read `roster-fed`.
    #[test]
    fn a_cell_that_stands_is_refused_by_the_input_that_nothing_delivers() {
        let world = state();
        // Rather than fabricating a standing cell in the overlay -- a fixture
        // agreeing with its code -- ask for a rate of zero, which
        // `holds_producing` answers `true` for on any world: zero cells are
        // needed and zero stand. The refusal path is the same one.
        let goal = Goal::Sustain {
            item: "iron-plate".into(),
            per_minute: 0,
            window_ticks: 7200,
        };
        assert!(
            crate::method::produce::holds_producing(&world, "iron-plate", 0),
            "premise: a rate of zero needs no cells, so capacity stands"
        );
        let roster = [BotId(1)];
        let err = expand(&[goal], &world, &registry_for(&roster), BotId(1))
            .expect_err("capacity stands, so the supply half is what is missing");
        let msg = err.to_string();
        assert!(msg.contains("coal"), "the refusal names the fuel: {msg}");
        assert!(
            msg.contains("not modelled"),
            "and says the supply is not modelled rather than blaming the world: {msg}"
        );
    }
}
