//! `obs:recover()`: what to do about a run that did not finish its plan.
//!
//! The executor decides *what* recovery is possible ([`recover`], three tiers,
//! pure); this module is the seam that puts the decision in a script's hands.
//! Nothing here retries anything by itself, and that is the design: a
//! supervisor that retried inside `goal.run` would hide the policy in Rust,
//! change what `obs.failed` means, and let a run burn a long time on a goal
//! that cannot succeed. The loop belongs in the script, where its budget is
//! visible:
//!
//! ```lua
//! local obs, tries = goal.run(plan), 0
//! while obs.failed > 0 and tries < 3 do
//!   local next_plan = obs:recover()
//!   if next_plan == nil then break end
//!   obs = goal.run(next_plan)
//!   tries = tries + 1
//! end
//! ```
//!
//! What a script never sees is the [`ExecutionLog`]. A proposal is only
//! correct when run against the right one -- the previous run's for a tier-1
//! reschedule, a fresh one for a re-expansion -- and the two are not
//! interchangeable, so the proposal carries its own log inside the
//! [`PlanValue`] it comes back as (see [`PlanValue::from_recovery`]). There is
//! no argument for a script to get wrong and no `goal.run(plan, log)` overload
//! to reach for.

use super::plan::{PlanOrigin, PlanValue};
use super::{goal_error, refuse_unknown_bots};
use factorio_bot_core::mlua::prelude::*;
use factorio_bot_executor::{recover, ExecutionLog};
use factorio_bot_planner::{ActionNetwork, PlanState};
use std::sync::Arc;

/// Installs `recover` on one observation table.
///
/// The snapshot the closure captures -- this network, this log, this
/// `done` -- is the same one the counts beside it were read from, so a script
/// that looked at `obs.failed` and then asked for a proposal gets one about
/// the run it just looked at.
pub(crate) fn install_recover(
    lua: &Lua,
    obs: &LuaTable,
    net: Arc<ActionNetwork>,
    log: ExecutionLog,
    done: bool,
    origin: Option<Arc<PlanOrigin>>,
) -> LuaResult<()> {
    obs.set(
        "recover",
        // `Option<LuaValue>` for the receiver, exactly as `failures` takes it,
        // so `obs:recover()` and `obs.recover()` are the same call.
        lua.create_function(move |_, _self: Option<LuaValue>| {
            propose(&net, &log, done, origin.as_ref())
        })?,
    )?;
    Ok(())
}

/// The decision, as the pair Lua receives: the next plan (or nothing) and the
/// word for why.
///
/// Reading observed state happens **here**, not when the observation was
/// built: tier 1 re-schedules against the world as it is now, and the point of
/// recovery is that the world moved -- the ore is further away, a bot died, a
/// furnace ended up somewhere else. A `PlanState` frozen at the failing run's
/// start would re-propose the plan that just failed.
fn propose(
    net: &Arc<ActionNetwork>,
    log: &ExecutionLog,
    done: bool,
    origin: Option<&Arc<PlanOrigin>>,
) -> LuaResult<(Option<PlanValue>, &'static str)> {
    // A proposal about a run that is still dispatching is a proposal to run
    // the same actions twice: `recover` retires an action only on `Success`,
    // so everything a bot is working on right now comes back in the plan.
    // Refused rather than quietly waited on -- `run:wait()` is how a script
    // says it wants to wait, and doing it implicitly inside a call that reads
    // like a question would block a script that only meant to ask one.
    if !done {
        return Err(goal_error(
            "recover: this run has not finished; wait for it (`run:wait()`, or \
             `goal.run`, which waits) before asking what to do about it -- a \
             proposal made now would re-dispatch the actions still in flight",
        ));
    }
    let Some(origin) = origin else {
        return Err(goal_error(
            "recover: this run did not come from `goal.plan`, so there is no goal to \
             re-plan (internal error, not a script bug)",
        ));
    };
    let state = PlanState::from_world(origin.world.clone(), &origin.roster);
    // The same refusal `goal.plan` makes, for the same reason: a bot that has
    // left since the plan was made would otherwise be planned for with a
    // fabricated inventory and guessed reach distances. Tier 2 really does
    // expand against this state, so the hazard is the same one.
    refuse_unknown_bots(&state)?;
    let decision = recover(&origin.goal, net, &state, &origin.roster, log);
    Ok(PlanValue::from_recovery(
        decision,
        origin.clone(),
        // Moved, not borrowed: the one log in play goes in, and which of the
        // two proposing arms uses it is the variant's decision, not a
        // caller's. See `PlanValue::from_recovery`.
        log.clone(),
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::super::plan::{COMPLETE, REEXPANDED, RESCHEDULED, SURFACED};
    use super::super::tests::{
        exec_bounded, exec_bounded_err, lua_with_goal, seeded_world_for, Failure, StubActuator,
    };
    use super::*;
    use factorio_bot_core::tokio::sync::watch;
    use factorio_bot_executor::{Actuator, Recovery, Status};
    use factorio_bot_planner::{schedule, ActionId, BotId, Goal, Holder, Schedule};
    use std::sync::atomic::AtomicBool;

    const BOTS: [BotId; 2] = [BotId(1), BotId(2)];

    fn ore_goal(count: u32) -> Goal {
        Goal::Have {
            item: "iron-ore".into(),
            count,
            whose: Holder::Anyone,
        }
    }

    fn origin(goal: Goal) -> Arc<PlanOrigin> {
        Arc::new(PlanOrigin {
            goal,
            world: seeded_world_for(&[1, 2]),
            roster: BOTS.to_vec(),
        })
    }

    /// A network of two actions with a `Success` recorded against `ActionId(0)`
    /// and a `Failed` against `ActionId(1)` -- the ids a re-expansion of the
    /// same goal numbers its own actions with, which is the collision the log
    /// rule exists for.
    fn a_run_with_one_success_and_one_failure() -> (ActionNetwork, Schedule, ExecutionLog) {
        let origin = origin(ore_goal(20));
        let state = PlanState::from_world(origin.world.clone(), &origin.roster);
        let net = super::super::expand_goal(ore_goal(20), &origin.world, &origin.roster)
            .expect("the fixture world can be mined");
        let sched = schedule(&net, &state, &origin.roster).expect("schedulable");

        let mut log = ExecutionLog::default();
        let ids: Vec<ActionId> = net.actions().map(|a| a.id).collect();
        assert!(
            ids.len() >= 2,
            "this fixture needs two actions to make one succeed and one fail, got {}",
            ids.len()
        );
        log.start(ids[0], 0);
        log.succeed(ids[0], 60);
        log.start(ids[1], 60);
        log.fail(ids[1], 90, "the bot walked into a biter".to_string());
        (net, sched, log)
    }

    /// The mapping the whole surface rests on, asserted variant by variant.
    ///
    /// The pairing of a proposal with the log it runs against is not a
    /// parameter anywhere -- `from_recovery` takes the previous log and
    /// decides -- so this is the test that the decision is the one documented.
    #[test]
    fn which_log_a_recovered_plan_runs_against_is_decided_by_the_variant() {
        let (net, sched, log) = a_run_with_one_success_and_one_failure();
        let goal = ore_goal(20);

        let (rescheduled, why) = PlanValue::from_recovery(
            Recovery::Rescheduled {
                net: net.clone(),
                sched: sched.clone(),
            },
            origin(goal.clone()),
            log.clone(),
        );
        assert_eq!(why, RESCHEDULED);
        let rescheduled = rescheduled.expect("tier 1 proposes a plan");
        assert_eq!(
            rescheduled.seed().status(ActionId(0)),
            Status::Success,
            "a reschedule is a subset of the network that just ran, so the log \
             describing it must carry forward -- without it `run_into` publishes \
             the succeeded actions as failures and abandons the retries behind them"
        );

        let (reexpanded, why) = PlanValue::from_recovery(
            Recovery::Reexpanded {
                net: net.clone(),
                sched: sched.clone(),
            },
            origin(goal.clone()),
            log.clone(),
        );
        assert_eq!(why, REEXPANDED);
        let reexpanded = reexpanded.expect("tier 2 proposes a plan");
        assert!(
            reexpanded.seed().is_empty(),
            "a re-expansion numbers its actions from zero, so every id the old log \
             knows would be read against an unrelated action"
        );

        let (nothing, why) =
            PlanValue::from_recovery(Recovery::Complete, origin(goal.clone()), log.clone());
        assert!(nothing.is_none(), "there is nothing left to run");
        assert_eq!(why, COMPLETE);

        let (nothing, why) =
            PlanValue::from_recovery(Recovery::Surfaced(log.failed()), origin(goal), log.clone());
        assert!(
            nothing.is_none(),
            "nothing mechanical is left, so proposing a plan would loop forever"
        );
        assert_eq!(why, SURFACED);
    }

    /// The safety property, at the surface a script uses: a re-expanded plan
    /// runs against a log that has never heard of it.
    ///
    /// The old log calls `ActionId(0)` a success and has three attempts
    /// recorded against `ActionId(1)`; the re-expanded network numbers its own
    /// actions from zero and reaches exactly those ids. Carried forward, the
    /// new run's first attempt at each of them would be counted as its fourth,
    /// which spends the tier-1 budget of a plan that has never run -- and any
    /// action the new schedule left unassigned would be published `Success`
    /// from a log describing a plan that no longer exists.
    ///
    /// Asserted on `attempts`, because that is what the stale log would
    /// visibly write into the new run.
    #[tokio::test]
    async fn a_reexpanded_plan_runs_against_a_fresh_log() {
        let stub: Arc<dyn Actuator> = Arc::new(StubActuator::new(Failure::Always));
        let lua = lua_with_goal(stub);
        exec_bounded(
            &lua,
            r#"
            -- Tier 1 keeps proposing the same plan until an action has burned
            -- its attempts; the run below is what burns them.
            local obs = goal.run(goal.plan(goal.have("iron-ore", 20)))
            local plan, why
            local rounds = 0
            repeat
                plan, why = obs:recover()
                assert(plan ~= nil, "a failing but schedulable plan must keep proposing")
                rounds = rounds + 1
                assert(rounds <= 6, "tier 1 never escalated")
                if why == "reexpanded" then break end
                assert(why == "rescheduled", "unexpected tier " .. why)
                obs = goal.run(plan)
            until false

            -- Every action of the old plan had been dispatched three times.
            local stale = false
            for _, a in pairs(obs.actions) do
                if a.attempts >= 3 then stale = true end
            end
            assert(stale, "the run before the re-expansion had exhausted its attempts")

            local fresh = goal.run(plan)
            for id, a in pairs(fresh.actions) do
                assert(a.attempts <= 1,
                       "action " .. id .. " of a re-expanded plan inherited " ..
                       a.attempts .. " attempts from a plan it has nothing to do with")
            end
        "#,
        )
        .await;
    }

    /// The other direction, and the silent one: a tier-1 proposal must run
    /// against the log it was made from.
    ///
    /// Tier 1 leaves already-succeeded actions out of its schedule and keeps
    /// them in its network for their lag edges; `run_into` reads their
    /// `Success` off the log it is given. From a fresh log they read as
    /// never-attempted, are published `Failed`, and the retries waiting behind
    /// them are abandoned -- a run that dispatches nothing and reports success.
    #[tokio::test]
    async fn a_rescheduled_plan_runs_against_the_log_of_the_run_it_recovers() {
        let stub: Arc<dyn Actuator> =
            Arc::new(StubActuator::new(Failure::First(AtomicBool::new(false))));
        let lua = lua_with_goal(stub);
        exec_bounded(
            &lua,
            r#"
            local obs = goal.run(goal.plan(goal.have("iron-ore", 20)))
            local failed = nil
            for id, a in pairs(obs.actions) do
                if a.status == "failed" then failed = id end
            end
            assert(failed ~= nil, "the stub refuses the first action dispatched")
            assert(obs.actions[failed].attempts == 1, "one attempt so far")

            local plan, why = obs:recover()
            assert(why == "rescheduled", "the plan still fits the world, got " .. why)
            local retried = goal.run(plan)

            assert(retried.actions[failed].attempts == 2,
                   "the retry is the second attempt at that action, not the first: " ..
                   "the log of the run being recovered must carry forward")
            assert(retried.actions[failed].status == "success", "and this time it worked")
            assert(retried.success > 0, "the retry reached the game")
        "#,
        )
        .await;
    }

    /// The loop the surface exists for, run to its end: recover, run, recover,
    /// until there is nothing left. A recovery surface that cannot terminate
    /// is worse than none, so the bound is asserted from inside the loop.
    #[tokio::test]
    async fn a_recover_loop_ends_at_complete() {
        let stub: Arc<dyn Actuator> =
            Arc::new(StubActuator::new(Failure::First(AtomicBool::new(false))));
        let lua = lua_with_goal(stub);
        exec_bounded(
            &lua,
            r#"
            local obs = goal.run(goal.plan(goal.have("iron-ore", 20)))
            assert(obs.failed > 0, "the stub refuses the first action, so there is work to recover")

            local rounds, reason = 0, nil
            while true do
                local next_plan, why = obs:recover()
                reason = why
                if next_plan == nil then break end
                rounds = rounds + 1
                assert(rounds <= 5, "the loop did not terminate")
                obs = goal.run(next_plan)
            end

            assert(reason == "complete",
                   "the loop must end because the work is done, ended with " .. tostring(reason))
            assert(rounds == 1, "one retry was enough, took " .. rounds)
            assert(obs.failed == 0 and obs.pending == 0 and obs.running == 0,
                   "nothing is left outstanding")
        "#,
        )
        .await;
    }

    /// A finished run with nothing to recover answers `nil` on the first ask,
    /// which is what makes the loop above safe to enter unconditionally.
    #[tokio::test]
    async fn a_clean_run_has_nothing_to_recover() {
        let stub: Arc<dyn Actuator> = Arc::new(StubActuator::new(Failure::Never));
        let lua = lua_with_goal(stub);
        exec_bounded(
            &lua,
            r#"
            local obs = goal.run(goal.plan(goal.have("iron-ore", 20)))
            assert(obs.failed == 0, "nothing failed")
            local plan, why = obs:recover()
            assert(plan == nil, "a finished plan proposes nothing")
            assert(why == "complete", "and says so, got " .. tostring(why))
        "#,
        )
        .await;
    }

    /// Recovering from a run that is still dispatching would propose the
    /// actions its bots are working on right now.
    ///
    /// The gate is never opened, so the run cannot finish and "still in
    /// flight" is a fact rather than a race.
    #[tokio::test]
    async fn a_run_still_in_flight_refuses_to_be_recovered() {
        let (_gate_tx, gate_rx) = watch::channel(false);
        let stub = StubActuator {
            gate: Some(gate_rx),
            ..StubActuator::new(Failure::Never)
        };
        let lua = lua_with_goal(Arc::new(stub));
        let err = exec_bounded_err(
            &lua,
            r#"
            local run = goal.start(goal.plan(goal.have("iron-ore", 20)))
            local obs = run:progress()
            assert(not obs.done, "the gate is shut, so the run cannot have finished")
            obs:recover()
        "#,
        )
        .await;
        assert!(
            err.contains("has not finished"),
            "the script must be told why: {err}"
        );
    }
}
