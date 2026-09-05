//! The plan value: an expanded action network plus the schedule assigning it
//! to bots, exposed to Lua as one inspectable object rather than a handle.
//!
//! `Schedule`'s own step shape is deliberately thin -- `ScheduledStep` carries
//! only `{ what, bot, start, end }`, and `StepKind::Act` carries only
//! `{ action, label }` -- because the planner does not repeat a `Mine`'s
//! `item`/`count` on every step that runs it; that payload lives once, on the
//! `Action` in the `ActionNetwork`. Building a Lua-facing step is therefore a
//! join: look the `ActionId` up in the network, read its `ActionKind`, and
//! flatten the two into one table. [`step_to_lua`] is that join, and it is
//! the single place the per-kind field shapes in the module doc table are
//! decided.
//!
//! `install_goal_plan` is `goal.plan`: it expands and schedules a goal value
//! in one call, against one roster, and hands back a [`PlanValue`]. See
//! `expand_goal`'s doc comment in `goal/mod.rs` for the defect that removes --
//! a network expanded for one roster only ever makes sense scheduled on that
//! same roster, and sharing one `roster` binding between the two calls here
//! is what makes the old mismatch unrepresentable.

use super::value::goal_from_lua;
use super::{
    BufferRefresher, PlacementChecker, expand_goal, goal_error, planner_error, refuse_unknown_bots,
};
use factorio_bot_core::factorio::rcon::PlacementQuery;
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::mlua::prelude::*;
use factorio_bot_core::types::Position;
use factorio_bot_executor::{ExecutionLog, Recovery};
use factorio_bot_planner::ids::{ActionId, BotId};
use factorio_bot_planner::method::produce::{cell_spec, cells_for, cells_standing};
use factorio_bot_planner::{
    ActionKind, ActionNetwork, Goal, InventorySlot, PlanState, Schedule, ScheduledStep, StepKind,
    Ticks, graphviz, mermaid_gantt, pick_chain_actor, schedule,
};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// The predicate keys `count`/`find` understand. `pos` and `to` are
/// deliberately absent -- they are tables, not comparable values, and are
/// rejected with a dedicated message rather than falling through to "unknown
/// field".
const KNOWN_PREDICATE_KEYS: &[&str] = &[
    "kind",
    "bot",
    "start",
    "finish",
    "id",
    "label",
    "entity",
    "item",
    "count",
    "tech",
    "slot",
    // A walk's tolerance. A plain number, so unlike `to` it compares; the
    // comparison is exact equality on an `f64`, which is what a caller asking
    // `{ radius = 10 }` means and all this predicate language offers.
    "radius",
    // And its inner bound, on the same terms. A walk serving a placement has
    // a non-zero one; every other walk has zero.
    "min_radius",
];

/// What a plan was planned *from*: everything a later recovery needs to
/// propose the next plan, and nothing a run needs to dispatch this one.
///
/// It travels plan -> run -> observation because that is the path the question
/// takes: `obs:recover()` is asked of a finished run, and answering it needs
/// the goal the plan came from (tier 2 re-expands it), the world to read
/// observed state off, and the roster to schedule against -- none of which the
/// executor keeps, since none of them is needed to dispatch a schedule.
///
/// The roster lives here rather than beside it on [`PlanValue`] so that the
/// three are never separated: expanding for one roster and scheduling for
/// another is the mismatch `goal.plan` exists to make unrepresentable, and a
/// recovery re-plans for exactly the roster the plan it recovers was made for.
pub(crate) struct PlanOrigin {
    pub(crate) goal: Goal,
    pub(crate) world: Arc<FactorioWorld>,
    pub(crate) roster: Vec<BotId>,
}

/// An expanded, scheduled plan: an [`ActionNetwork`] plus the [`Schedule`]
/// that assigned its actions to bots, held together so Lua can inspect either
/// side of the join `step_to_lua` performs.
///
/// `net` and `schedule` are `Arc`s rather than owned values because
/// `goal.start`/`goal.run` need exactly these two handed to the executor
/// without cloning the graph or the schedule.
pub(crate) struct PlanValue {
    net: Arc<ActionNetwork>,
    schedule: Arc<Schedule>,
    /// Where this plan came from; also the roster it was scheduled for, in
    /// roster order, which is what `plan.bots` reports.
    origin: Arc<PlanOrigin>,
    /// The [`ExecutionLog`] a run of this plan must start from.
    ///
    /// Empty for every plan a script builds with `goal.plan`, and empty for a
    /// re-expanded recovery. Non-empty for exactly one thing: a tier-1
    /// recovery, whose network is a *subset of the one that just ran* and
    /// whose ids therefore still mean what the previous run's log says they
    /// mean (see [`Recovery::Rescheduled`]).
    ///
    /// It is a field rather than an argument to `goal.run` on purpose. The
    /// pairing of a proposal with the log it must run against is the one
    /// mistake this surface cannot let a script make -- carrying the old log
    /// into a re-expanded plan joins unrelated work, and starting a fresh one
    /// for a rescheduled plan abandons the retries waiting behind actions that
    /// already succeeded -- so the pairing is decided once, inside
    /// [`PlanValue::from_recovery`], by a `match` on the variant. No caller,
    /// Lua or Rust, is ever handed both halves to put together.
    seed: ExecutionLog,
    /// Set the first time a [`RunSlot`] is actually taken. A plan may only be
    /// run once -- running it twice would dispatch every action against the
    /// game a second time -- and this flag is where that rule lives, which is
    /// why the run side (`RunValue`) needs no registry of its own to enforce
    /// it.
    ///
    /// An `Arc` rather than a bare flag so [`RunSlot`] can carry the right to
    /// set it *without* carrying a borrow of the plan's userdata. That is
    /// what lets `goal.start` defer the flip past its `.await` on the
    /// actuator: the reservation is made synchronously, the flip happens only
    /// once dispatch is certain.
    consumed: Arc<AtomicBool>,
}

/// A reservation on a plan: everything an about-to-start run needs, plus the
/// right to consume the plan -- and nothing that borrows the plan itself.
///
/// It exists because "a plan may be run once" and "a plan is spent" are two
/// different facts, and the second must only become true when a run is
/// actually going to dispatch. `goal.start` can fail after the reservation
/// for reasons that have nothing to do with the plan -- no `PendingWork`, no
/// actuator (which is what *every* plan-only script hits: `create_lua_goal`'s
/// factory refuses with "no rcon connection") -- and a plan burned by one of
/// those answers the next `goal.start` with "already taken for a run",
/// reporting an absent fact as a present one and hiding the real cause behind
/// it. So [`take`](RunSlot::take) is called last, after every check that can
/// fail has passed and the actuator is in hand.
pub(crate) struct RunSlot {
    net: Arc<ActionNetwork>,
    schedule: Arc<Schedule>,
    seed: ExecutionLog,
    origin: Arc<PlanOrigin>,
    consumed: Arc<AtomicBool>,
}

/// Everything a run needs, handed over by the plan that is spent to start it.
///
/// A struct rather than a tuple because [`seed`](Dispatch::seed) is the field
/// nobody may choose: it arrives already paired with the network it belongs to
/// (see [`PlanValue::seed`]), and a tuple invites a caller to build one from
/// parts.
pub(crate) struct Dispatch {
    pub(crate) net: Arc<ActionNetwork>,
    pub(crate) schedule: Arc<Schedule>,
    pub(crate) seed: ExecutionLog,
    pub(crate) origin: Arc<PlanOrigin>,
}

impl RunSlot {
    /// Consumes the plan and hands over what the run needs.
    ///
    /// The check is repeated here, not merely made at reservation time: two
    /// reservations can be outstanding at once (each `goal.start` awaits its
    /// actuator, and a script may have several coroutines in flight), and the
    /// `swap` is what makes exactly one of them win.
    pub(crate) fn take(self) -> LuaResult<Dispatch> {
        if self.consumed.swap(true, Ordering::SeqCst) {
            return Err(already_taken());
        }
        Ok(Dispatch {
            net: self.net,
            schedule: self.schedule,
            seed: self.seed,
            origin: self.origin,
        })
    }
}

/// The one refusal a spent plan gives, worded the same wherever it is raised.
///
/// It names no run, because a run is not a named thing here: `RunValue`
/// carries a log, a network and a completion signal, and no identity a second
/// caller could be pointed at. Saying "the run started at line 12" would be
/// inventing one.
fn already_taken() -> LuaError {
    goal_error(
        "plan has already been taken for a run; a plan may be executed at most once, so re-plan to retry",
    )
}

/// The word `obs:recover()` answers with, one per [`Recovery`] variant.
///
/// A `&'static str` and not a number or a boolean, because the four outcomes
/// are not ordered and not two: "the work is done" and "nothing mechanical is
/// left to try" both end a loop, and a script that stops on either still wants
/// to say which happened.
pub(crate) const RESCHEDULED: &str = "rescheduled";
pub(crate) const REEXPANDED: &str = "reexpanded";
pub(crate) const COMPLETE: &str = "complete";
pub(crate) const SURFACED: &str = "surfaced";

impl PlanValue {
    /// A plan as `goal.plan` builds it: nothing has run, so the log a run of
    /// it starts from is empty.
    pub(crate) fn new(
        net: Arc<ActionNetwork>,
        schedule: Arc<Schedule>,
        origin: Arc<PlanOrigin>,
    ) -> Self {
        Self {
            net,
            schedule,
            origin,
            seed: ExecutionLog::default(),
            consumed: Arc::new(AtomicBool::new(false)),
        }
    }

    /// A [`Recovery`] as the next plan to run -- **including which log that
    /// run must start from**.
    ///
    /// This is the only place in the crate where a network and a non-empty
    /// [`ExecutionLog`] are put together, and the pairing is not a parameter:
    /// `previous` is the log the run being recovered from produced, and which
    /// of the two arms below uses it is decided by the variant, here, by a
    /// `match` that a new variant breaks the build over.
    ///
    /// Both directions of the pairing are a defect, and only one of them is
    /// loud:
    ///
    /// - **`Reexpanded` with `previous` would join unrelated work.** `expand`
    ///   numbers a fresh network from zero, so `ActionId(0)` of the new plan
    ///   is an unrelated action that merely shares a number with `ActionId(0)`
    ///   of the old one. Every colliding id would inherit the old plan's
    ///   attempt count -- burning the tier-1 budget of a plan that has never
    ///   run -- and any of them the new schedule left unassigned would be
    ///   published `Success` straight from a log describing a plan that no
    ///   longer exists.
    /// - **`Rescheduled` with a fresh log is the silent one.** Tier 1 keeps
    ///   already-succeeded actions in its network for their lag edges and
    ///   leaves them out of its schedule; `run_into` reads their `Success` off
    ///   the log it is given. From an empty log they read as never-attempted,
    ///   so they are published `Failed`, the retries waiting behind them are
    ///   abandoned, and the run returns having dispatched nothing at all.
    ///
    /// `previous` is taken **by value** for that reason: at the one call site
    /// there is a single log, it is moved in, and there is no second use of it
    /// to get wrong.
    ///
    /// `Complete` and `Surfaced` produce no plan. The word is returned
    /// alongside either way, so the caller always learns which tier answered
    /// without inspecting a schedule's insides.
    pub(crate) fn from_recovery(
        recovery: Recovery,
        origin: Arc<PlanOrigin>,
        previous: ExecutionLog,
    ) -> (Option<Self>, &'static str) {
        match recovery {
            // The work is done. Deliberately not a `Rescheduled` with an empty
            // schedule -- see `Recovery::Complete` -- so a script looping on
            // `recover` has something to stop on.
            Recovery::Complete => (None, COMPLETE),
            // Nothing mechanical is left to try. Also no plan, and for the
            // same reason: proposing one here would loop forever.
            Recovery::Surfaced(_) => (None, SURFACED),
            Recovery::Rescheduled { net, sched } => (
                Some(Self::recovered(net, sched, origin, previous)),
                RESCHEDULED,
            ),
            Recovery::Reexpanded { net, sched } => (
                Some(Self::recovered(
                    net,
                    sched,
                    origin,
                    // Not `previous`, and not a filtered `previous`: a plan
                    // built from scratch is a different plan, so its progress
                    // is recorded from scratch too.
                    ExecutionLog::default(),
                )),
                REEXPANDED,
            ),
        }
    }

    /// The plan half of [`from_recovery`](PlanValue::from_recovery), shared by
    /// its two proposing arms so that they can differ in exactly one thing --
    /// the log -- and nothing else.
    fn recovered(
        net: ActionNetwork,
        schedule: Schedule,
        origin: Arc<PlanOrigin>,
        seed: ExecutionLog,
    ) -> Self {
        Self {
            net: Arc::new(net),
            schedule: Arc::new(schedule),
            origin,
            seed,
            consumed: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The log a run of this plan would start from. Test-only: production
    /// never reads it, it only hands it to the run.
    #[cfg(test)]
    pub(crate) fn seed(&self) -> &ExecutionLog {
        &self.seed
    }

    /// Reserves the plan for a run that is about to be attempted, cloning out
    /// everything the attempt needs so no borrow of this userdata survives
    /// into the attempt's future.
    ///
    /// Reserving does **not** consume: [`RunSlot::take`] does, and only once
    /// dispatch is certain. An already-spent plan is refused here, before the
    /// caller wastes an actuator on it -- so a genuinely second run still
    /// hears "already taken", and only that case does.
    pub(crate) fn reserve_for_run(&self) -> LuaResult<RunSlot> {
        if self.consumed.load(Ordering::SeqCst) {
            return Err(already_taken());
        }
        Ok(RunSlot {
            net: self.net.clone(),
            schedule: self.schedule.clone(),
            seed: self.seed.clone(),
            origin: self.origin.clone(),
            consumed: self.consumed.clone(),
        })
    }
}

/// The roster `goal.plan` expands and schedules against -- and `goal.holds`
/// reads inventories for: `opts.bots` if given, otherwise `default_roster`
/// (the run's whole roster).
///
/// Deliberately a list of bot ids, never a count: a count could only mean
/// "some N of them", which is exactly the ambiguity `goal.plan` exists to
/// remove by fixing expansion and scheduling to the same, explicit roster.
///
/// An empty roster raises here, before either the planner or the scheduler
/// ever sees it -- `schedule` itself also refuses an empty slice
/// (`PlannerError::NoBots`), but that error does not mention "bot" at all, and
/// this call's own empty-roster test asserts on that word.
pub(super) fn resolve_roster(
    opts: Option<&LuaTable>,
    default_roster: &[BotId],
) -> LuaResult<Vec<BotId>> {
    let bots_value: LuaValue = match opts {
        Some(opts) => opts.get("bots")?,
        None => LuaValue::Nil,
    };
    let roster = match bots_value {
        LuaValue::Nil => default_roster.to_vec(),
        LuaValue::Table(bots) => {
            let len = bots.raw_len();
            let mut roster = Vec::with_capacity(len);
            for i in 1..=len {
                let value: LuaValue = bots.get(i)?;
                let n: i64 = match value {
                    LuaValue::Integer(n) if n >= 1 => n,
                    LuaValue::Number(n) if n >= 1.0 && n.fract() == 0.0 => n as i64,
                    other => {
                        return Err(goal_error(format!(
                            "opts.bots must be a list of positive integers; \
                             index {i} is a {}",
                            other.type_name()
                        )));
                    }
                };
                let bot = u8::try_from(n)
                    .map_err(|_| goal_error(format!("bot {n} does not fit a player id (0-255)")))?;
                roster.push(BotId(bot));
            }
            roster
        }
        other => {
            return Err(goal_error(format!(
                "opts.bots must be a table of bot ids, got a {}",
                other.type_name()
            )));
        }
    };
    if roster.is_empty() {
        return Err(goal_error(
            "opts.bots is empty; a plan needs at least one bot",
        ));
    }
    Ok(roster)
}

/// Installs `goal.plan` on `table`.
///
/// Expands and schedules in a single call, against one shared roster --
/// `expand_goal`'s own doc comment explains why the two cannot safely use
/// different ones. `world` and `default_roster` are captured by the closure,
/// exactly as every other `goal.*` binding in `mod.rs` captures them.
pub(crate) fn install_goal_plan(
    lua: &Lua,
    table: &LuaTable,
    world: Arc<FactorioWorld>,
    default_roster: Vec<BotId>,
    checker: Option<PlacementChecker>,
    refresher: Option<BufferRefresher>,
) -> LuaResult<()> {
    table.set(
        "plan",
        lua.create_async_function(move |_lua, (g, opts): (LuaTable, Option<LuaTable>)| {
            let world = world.clone();
            let default_roster = default_roster.clone();
            let checker = checker.clone();
            let refresher = refresher.clone();
            async move {
                let goal = goal_from_lua(&g)?;
                let roster = resolve_roster(opts.as_ref(), &default_roster)?;
                let (net, scheduled) =
                    plan_verified(&goal, &world, &roster, checker.as_ref(), refresher.as_ref())
                        .await?;
                narrate_work_split(&net, &scheduled);
                // The goal, the world and the roster are kept together on the
                // plan, not because dispatching needs them -- it does not --
                // but because `obs:recover()` will, one run later, and a
                // recovery must re-plan the same goal for the same roster or
                // it is answering a different question.
                Ok(PlanValue::new(
                    Arc::new(net),
                    Arc::new(scheduled),
                    Arc::new(PlanOrigin {
                        goal,
                        world: world.clone(),
                        roster,
                    }),
                ))
            }
        })?,
    )?;
    Ok(())
}

/// How many times `goal.plan` will re-site a plan the game says it would
/// refuse, before handing back whatever the last expansion produced.
///
/// Bounded rather than "until it converges", for two independent reasons.
/// Each round costs one RCON round trip *and* one full expansion, so an
/// unbounded loop is an unbounded stall inside a call a script thinks is
/// cheap; and a bug that stopped the ledger reaching the planner would turn
/// that loop into a hang rather than into the visible regression it should
/// be. Three expansions and three round trips is the worst case.
///
/// Exceeding the budget is not an error and does not raise. The plan is
/// returned with whatever sites it has, the dispatch-time refusal path
/// catches them exactly as it did before this existed, and every site the
/// game turned down along the way is in the ledger and in the record either
/// way. The pre-check is an improvement on the failure mode, never a new one.
const MAX_RESITE_ROUNDS: usize = 2;

/// Ask the game what is in the buffers, and say out loud what came back.
///
/// # Both outcomes are narrated, and that is the point
///
/// A refresh that found nothing and a refresh that never happened leave
/// **identical worlds and identical plans**: `FactorioWorld::inventories` is
/// empty either way, `PlanState::has_buffers` is false either way, and
/// `Withdraw` claims nothing either way. Only one of those is a defect, and
/// nothing about the resulting plan distinguishes them -- it re-mines, and if
/// the ore is gone it stalls, which is the exact failure buffers were made
/// visible to prevent. So the count is printed whether or not it is
/// interesting, and silence here means the refresh did not run.
///
/// # `paris` on stdout, not `tracing` on stderr
///
/// This is narration: a line a person reads *while the run happens*, about
/// what the run is doing next. The two `tracing::warn!`s in
/// [`plan_verified`] are diagnostics -- they explain a degraded pre-check to
/// whoever debugs the run later. A failed refresh is both, so it gets both:
/// `paris` says what it means for this run, `tracing` carries the error
/// string. See the logging note in `CLAUDE.md`; the colour markup below is
/// `paris` syntax and would print literally through `tracing`.
///
/// # Volume
///
/// One line per `goal.plan`, which the supervisor calls once per milestone
/// iteration -- tens of lines across a whole run, beside a Factorio server's
/// own stdout. Cheap enough to always print, and the whole value is in always.
///
/// # Warn and carry on
///
/// A refresh that cannot be made must not stop a run, exactly as an
/// unreachable pre-check does not. The plan that follows is the plan this call
/// would have returned before buffers were visible at all, so the cost of a
/// failed refresh is the behaviour we already had -- but it is *not* a free
/// failure the way a missed pre-check is, because the pre-check only narrows a
/// window while this one decides whether a plan re-mines ore that may be gone.
/// The warning says so in those terms rather than reporting an RPC error.
/// Say how much of each production goal is already standing, before planning
/// the rest.
///
/// # Why a `Producing` goal in particular needs a line
///
/// Every other goal this planner takes is satisfied by *actions*, so a plan
/// that does something is visible in the run's own output. A `Producing` goal
/// is satisfied by *machines*, and machines persist: the second time a
/// supervisor asks for one, the honest plan is empty. An empty plan and a plan
/// nobody asked for look exactly alike from outside — identical worlds,
/// identical output, and only one of them a defect. That is the same
/// indistinguishability [`narrate_buffer_refresh`] exists to remove, one level
/// out, and it is worth more here: the thing that "already stands" is a
/// factory, and a wrong belief about it is a milestone that closes satisfied
/// having built nothing.
///
/// # What it does not say
///
/// **That anything is coming out.** `cells_standing` counts structure — a
/// drill on the right ore, delivering into a furnace — and reads no fuel level
/// and no output inventory. The line below says "stand", never "produce", on
/// purpose, and a run that wants the other claim has to watch a furnace's
/// output rise with every bot idle.
///
/// # Volume
///
/// One line per production goal per `goal.plan`, and a run has one or two such
/// goals at most; a ladder that never asks for one prints nothing at all.
/// `paris` on stdout, because this is narration a person reads while the run
/// happens — see the logging note in `CLAUDE.md`.
fn narrate_production_goals(goal: &Goal, state: &PlanState) {
    for p in production_progress(goal, state) {
        if p.standing == 0 {
            factorio_bot_core::paris::info!(
                "no <bright-blue>{}</> cell stands yet: planning <bright-blue>{}</> of them for {} a minute",
                p.item,
                p.wanted,
                p.per_minute
            );
        } else if p.standing >= p.wanted {
            factorio_bot_core::paris::info!(
                "<bright-blue>{}</> <bright-blue>{}</> cell(s) already stand and {} a minute \
                 needs {}: nothing left to build. They *stand*, which is not the same as \
                 producing -- only a furnace's output rising says that",
                p.standing,
                p.item,
                p.per_minute,
                p.wanted
            );
        } else {
            factorio_bot_core::paris::info!(
                "<bright-blue>{}</> of <bright-blue>{}</> {} cell(s) already stand: planning \
                 the other {}",
                p.standing,
                p.wanted,
                p.item,
                p.wanted - p.standing
            );
        }
    }
}

/// Says how this plan divides across the roster, and how much of that division
/// happened *inside* somebody else's chain.
///
/// # Why the number needs saying out loud
///
/// `steps/bot` is the number the whole four-bot utilisation effort turns on,
/// and for a long time nothing said it while the run was happening. Two
/// separate classes of defect were found and fixed in one evening — bots
/// unable to work, and work being duplicated — and *neither moved it*, which
/// only became visible after the run, from
/// `tools/run_analysis.py --json <run-dir>`. A run that has already spent
/// twenty minutes is an expensive place to learn that the work never divided.
///
/// The second line is R3 specifically. A `Researched` chain is welded to one
/// bot by construction (`crates/planner`'s owner-binding comment), so the only
/// work that can leave it is work whose product is a fact about the **map**
/// rather than about an inventory: a furnace that stands, and a furnace that is
/// fuelled. When that happens the plan looks, from outside, exactly like a plan
/// where it did not — same actions, same goal, same roster — so it is said
/// here or it is invisible. `crates/planner` carries no logger on purpose;
/// this is where its decisions are narrated, for the same reason
/// [`narrate_walled_in_bots`] is.
///
/// # How a handover is recognised
///
/// Off the network, not off a flag: a `Place` of a stone furnace whose chain
/// owner is **not** the owner of the chain that takes the plates back out of
/// that same furnace. That is the fact itself rather than a report of it, so a
/// change that stopped handing furnaces over would silence this line rather
/// than keep printing a stale claim.
///
/// # Volume
///
/// One line per `goal.plan`, plus a second only when a handover happened —
/// the same budget [`narrate_production_goals`] keeps. `paris` on stdout,
/// because it is narration a person reads while the run happens.
fn narrate_work_split(net: &ActionNetwork, plan: &Schedule) {
    let mut steps: std::collections::BTreeMap<BotId, usize> = Default::default();
    let mut mined: std::collections::BTreeMap<BotId, u32> = Default::default();
    for step in &plan.steps {
        *steps.entry(step.bot).or_default() += 1;
        let StepKind::Act { action, .. } = step.what else {
            continue;
        };
        if let Some(ActionKind::Mine { count, .. }) = net.action(action).map(|a| &a.kind) {
            *mined.entry(step.bot).or_default() += count;
        }
    }
    if steps.is_empty() {
        // An empty plan is a legitimate answer -- the goal is already met --
        // and `narrate_production_goals` has already said so where it applies.
        return;
    }
    let split: Vec<String> = steps
        .iter()
        .map(|(bot, count)| {
            format!(
                "bot {}: {} step(s), {} raw unit(s)",
                bot.0,
                count,
                mined.get(bot).copied().unwrap_or(0)
            )
        })
        .collect();
    factorio_bot_core::paris::info!("this plan divides as <bright-blue>{}</>", split.join("; "));

    // Who takes the plates out of the furnace at each position: that chain's
    // owner is the bot the smelt was sized against.
    let mut taker_of: std::collections::BTreeMap<String, BotId> = Default::default();
    for action in net.actions() {
        let ActionKind::Remove { pos, entity, .. } = &action.kind else {
            continue;
        };
        if entity != "stone-furnace" {
            continue;
        }
        if let Some(owner) = net.chain_of(action.id).and_then(|c| net.owner_of(c)) {
            taker_of.insert(pos.to_string(), owner);
        }
    }
    let handovers: Vec<String> = net
        .actions()
        .filter_map(|action| {
            let ActionKind::Place { entity } = &action.kind else {
                return None;
            };
            if entity.name != "stone-furnace" {
                return None;
            }
            let builder = net.chain_of(action.id).and_then(|c| net.owner_of(c))?;
            let taker = taker_of.get(&entity.position.to_string()).copied()?;
            (builder != taker).then(|| format!("bot {} for bot {}", builder.0, taker.0))
        })
        .collect();
    if !handovers.is_empty() {
        factorio_bot_core::paris::info!(
            "<bright-blue>{}</> furnace(s) are built and fuelled by a bot other than the one \
             that will smelt in them (<bright-blue>{}</>). A furnace that stands is a fact \
             about the map, not about anybody's pockets, so its stone and its coal are sized \
             against the supplier *and* run by it -- the gathering divides without any chain \
             being sized for one bot and handed to another",
            handovers.len(),
            handovers.join(", ")
        );
    }
}

/// Says which bots this plan will not size a gathering share against, and why.
///
/// A share sized against a bot that cannot walk anywhere is work no other bot
/// can pick up (`crates/planner`'s `participants_that_can_work` argues the
/// whole case), so the planner leaves such a bot out of the split. That is a
/// visible change in how much of the map the run uses, and it has to be
/// visible in the run's own output too: `crates/executor`'s walk memory
/// originally shipped with no logging and nobody could tell whether it had
/// fired, which cost real diagnostic time. `crates/planner` carries no logger
/// on purpose -- it is a pure crate -- so the line is written here, where the
/// plan is actually made.
///
/// Both halves of the answer are narrated. Staying silent when a bot *stops*
/// being walled in would leave a reader unable to tell "the pocket opened"
/// from "nobody ever looked".
///
/// The chain actor is narrated here too, for the same reason and by the same
/// rule. A goal that names no bot -- `Researched`, `BuildCell`, `Producing` --
/// is sized against the chain actor's inventory *and* run by it, so
/// `expand_goal` skips a walled-in bot when it picks one
/// (`crates/planner`'s `pick_chain_actor`). That silently changes which bot
/// does the run's headline work and which inventory its bill is measured
/// against, which is exactly the class of decision this function exists to
/// stop being silent. Only the *move* is reported: saying "the chain actor is
/// bot 1" on every plan where nothing happened would bury the line that
/// matters.
fn narrate_walled_in_bots(state: &PlanState, roster: &[BotId]) {
    let walled_in = state.walled_in();
    if walled_in.is_empty() {
        return;
    }
    let working: Vec<String> = state
        .bot_ids()
        .into_iter()
        .filter(|bot| !state.is_walled_in(*bot))
        .map(|bot| bot.0.to_string())
        .collect();
    for (bot, pocket_tiles) in walled_in {
        factorio_bot_core::paris::warn!(
            "bot <bright-blue>{}</> is walled in at <bright-blue>{}</>: every point it can \
             reach is inside a pocket of {:.1} square tiles. It gets no gathering share this \
             plan -- a share names one owner and no other bot may take it over -- and it is \
             back in the split the moment the pocket opens",
            bot.0,
            state
                .bot(*bot)
                .map(|b| b.position.to_string())
                .unwrap_or_else(|| "an unknown position".into()),
            pocket_tiles,
        );
    }
    if working.is_empty() {
        factorio_bot_core::paris::warn!(
            "every bot is walled in, so the split is sized across all of them anyway: a plan \
             that dispatches and fails leaves a record and a recovery tier, one that was never \
             made leaves neither"
        );
    } else {
        factorio_bot_core::paris::info!(
            "gathering shares this plan are sized across bot(s) <bright-blue>{}</>",
            working.join(", ")
        );
    }
    // `expand_goal` asks the same question of the same state and the same
    // roster, so this reports the pick that plan is actually made with rather
    // than a second guess at it.
    if let (Some(preferred), Some(chosen)) =
        (roster.first().copied(), pick_chain_actor(state, roster))
        && preferred != chosen
    {
        factorio_bot_core::paris::warn!(
            "the chain actor moves from bot <bright-blue>{}</> to bot <bright-blue>{}</>: bot \
             {} is walled in, and a goal that names no bot is both sized against the chain \
             actor's inventory and welded to it, so leaving it there would be work no other \
             bot could take over",
            preferred.0,
            chosen.0,
            preferred.0,
        );
    }
}

/// How far along one `Goal::Producing` is: what it asks for, and what stands.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Production {
    item: String,
    per_minute: u32,
    /// Complete cells for `item` the state already carries.
    standing: u32,
    /// Complete cells `per_minute` needs in all.
    wanted: u32,
}

/// The production goals in `goal`, with their progress — the whole decision
/// [`narrate_production_goals`] reports, split out so it can be tested without
/// reading stdout.
///
/// A goal whose item no cell can make, or whose rate the planner refuses
/// outright, yields nothing: `expand` is about to refuse it by name, and
/// saying so twice would be noise. Recurses into `Goal::All` exactly as
/// `goal_from_lua` does, so a production goal inside a bundle is not silently
/// skipped.
fn production_progress(goal: &Goal, state: &PlanState) -> Vec<Production> {
    match goal {
        Goal::All(goals) => goals
            .iter()
            .flat_map(|member| production_progress(member, state))
            .collect(),
        Goal::Producing { item, per_minute } => {
            let Some(spec) = cell_spec(state, item) else {
                return Vec::new();
            };
            let Ok(wanted) = cells_for(*per_minute, spec.ticks_per_item) else {
                return Vec::new();
            };
            vec![Production {
                item: item.clone(),
                per_minute: *per_minute,
                standing: cells_standing(state, &spec),
                wanted,
            }]
        }
        _ => Vec::new(),
    }
}

async fn narrate_buffer_refresh(refresher: Option<&BufferRefresher>) {
    let Some(refresher) = refresher else {
        // No RCON: `goal.plan` called with no game behind it, which is a
        // legitimate mode (`--clients 0`) and not a failure. Nothing is said,
        // because there was nothing to ask and no run to mislead.
        return;
    };
    match refresher().await {
        Ok(0) => factorio_bot_core::paris::info!(
            "no buffers to read before planning: the world knows of no furnace or chest yet"
        ),
        Ok(asked) => factorio_bot_core::paris::info!(
            "read the contents of <bright-blue>{}</> buffer(s) before planning",
            asked
        ),
        Err(err) => {
            factorio_bot_core::paris::warn!(
                "<red>could not read what is in this world's buffers</>; planning as if every \
                 furnace and chest were empty. Anything a previous plan left in one is invisible \
                 to this one, so it will plan to make those materials again -- out of ore that \
                 may already have been mined for them"
            );
            factorio_bot_core::tracing::warn!(
                "buffer refresh failed before goal.plan, planning against an empty buffer map: {}",
                err
            );
        }
    }
}

/// Expands and schedules `goal`, asking the game about the placements the
/// plan chose and re-expanding around the ones it would refuse.
///
/// # The loop, and why it terminates
///
/// Expansion and scheduling are pure functions of the world snapshot and the
/// roster. The only thing that changes between two rounds is the world: a
/// refused site is written into `FactorioWorld::placement_refusals` by
/// [`FactorioRcon::can_place_entities`], and `PlanState::from_world` reads
/// that ledger on the next `from_world`, so `free_area_near` stops offering
/// the refused footprint. That is the whole reason a pre-check needed refusal
/// memory to exist first: without somewhere durable to put the answer, the
/// next expansion re-derives the same site from the same inputs and the loop
/// never moves.
///
/// [`MAX_RESITE_ROUNDS`] bounds it regardless, so a defect in that chain costs
/// two wasted round trips rather than a hang.
///
/// # Cost
///
/// One round trip for a plan whose sites are all legal — the overwhelmingly
/// common case, and the one the budget is chosen for. A plan with no `Place`
/// action at all costs none: the query list is empty and no call is made.
///
/// # What a green pre-check does not promise
///
/// Nothing about the moment the action runs. See the module-level note on
/// staleness in `docs/superpowers/notes/2026-09-02-placement-precheck.md`; in
/// short, the check narrows the window between "the site was chosen" and "the
/// build is attempted", and does not close it.
async fn plan_verified(
    goal: &Goal,
    world: &Arc<FactorioWorld>,
    roster: &[BotId],
    checker: Option<&PlacementChecker>,
    refresher: Option<&BufferRefresher>,
) -> LuaResult<(ActionNetwork, Schedule)> {
    // Once, before any expansion, and deliberately outside the loop below.
    //
    // What separates two rounds of that loop is the refusal ledger the
    // previous round's pre-check wrote; nothing in it changes what is standing
    // in a chest. A refresh per round would buy a second RCON round trip and
    // an answer the first one already gave.
    //
    // Before, rather than after, for a sharper reason: `PlanState::from_world`
    // reads the buffer map at the top of every round, and `Withdraw` is what
    // decides whether the plan mines ore or walks to a furnace. Refreshing
    // after expansion would refresh a fact the plan had already been built
    // without.
    narrate_buffer_refresh(refresher).await;
    for round in 0..=MAX_RESITE_ROUNDS {
        // Rebuilt every round, deliberately: this is the read that picks up
        // the refusals the previous round's query wrote.
        let state = PlanState::from_world(world.clone(), roster);
        refuse_unknown_bots(&state)?;
        // First round only: the re-siting rounds below re-expand against the
        // same standing machines, and saying it three times would be noise.
        if round == 0 {
            narrate_production_goals(goal, &state);
            narrate_walled_in_bots(&state, roster);
        }
        let net = expand_goal(goal.clone(), world, roster)?;
        let scheduled = schedule(&net, &state, roster).map_err(planner_error)?;

        let Some(checker) = checker else {
            return Ok((net, scheduled));
        };
        let queries = placement_queries(&net, &scheduled);
        if queries.is_empty() {
            return Ok((net, scheduled));
        }
        let verdicts = match checker(queries.clone()).await {
            Ok(verdicts) => verdicts,
            Err(err) => {
                // A pre-check that cannot be made must not stop a run. The
                // plan is exactly the plan this call would have returned
                // before the pre-check existed, and the dispatch-time refusal
                // path is untouched -- so the cost of an unreachable or
                // out-of-date mod is the behaviour we already had, reported
                // once, rather than a raise from a call that used to be
                // infallible.
                factorio_bot_core::tracing::warn!(
                    "could not ask the game whether this plan's {} placement(s) are legal, \
                     dispatching it unchecked: {}",
                    queries.len(),
                    err
                );
                return Ok((net, scheduled));
            }
        };
        // Length mismatches are rejected inside `can_place_entities`, which
        // is the only place the join by index is made; anything shorter than
        // the query list here would mean a stub, and zipping is then the
        // conservative read -- an unanswered site is not a refused one.
        let refused = verdicts.iter().filter(|v| v.is_durable_refusal()).count();
        if refused == 0 {
            return Ok((net, scheduled));
        }
        // The last round still *asks* -- it just does not re-expand. Every
        // site the game turned down is in the ledger either way, so the next
        // `goal.plan` (the supervisor replans every iteration) starts from
        // what this one learned rather than rediscovering it.
        if round == MAX_RESITE_ROUNDS {
            factorio_bot_core::tracing::warn!(
                "still {} refused placement(s) after {} re-sitings; dispatching the plan as it \
                 stands. The refused sites are on record and the dispatch-time refusal path is \
                 unchanged, so this is the behaviour that existed before the pre-check, not a \
                 new failure",
                refused,
                MAX_RESITE_ROUNDS,
            );
            return Ok((net, scheduled));
        }
        factorio_bot_core::tracing::info!(
            "the game would refuse {} of this plan's {} placement(s); re-siting rather than \
             dispatching (round {} of {})",
            refused,
            queries.len(),
            round + 1,
            MAX_RESITE_ROUNDS,
        );
    }
    // Unreachable: the loop above returns on its last round.
    Err(goal_error(
        "placement pre-check loop ended without a plan (internal error)",
    ))
}

/// Every placement this plan would make, as a question for the game.
///
/// Deduplicated, because a schedule may legitimately name the same site more
/// than once and asking twice would cost a round trip's worth of payload for
/// an answer already in hand. Ordered by `ActionId` -- `ActionNetwork::actions`
/// iterates a `BTreeMap` -- so two runs of the same plan ask the same
/// questions in the same order.
///
/// A `Place` the schedule assigns to no bot is skipped rather than guessed at:
/// `can_place_entity` needs a force and a surface, both of which come from the
/// acting player, and inventing one would ask about a placement nobody is
/// going to make.
fn placement_queries(net: &ActionNetwork, sched: &Schedule) -> Vec<PlacementQuery> {
    let mut seen: BTreeSet<(u8, String, String, u8)> = BTreeSet::new();
    let mut queries = Vec::new();
    for action in net.actions() {
        let ActionKind::Place { entity } = &action.kind else {
            continue;
        };
        let Some(bot) = sched.assignment(action.id) else {
            continue;
        };
        // A `BotId` *is* a Factorio player id; there is no mapping layer
        // anywhere in this stack and this must not become one.
        let key = (
            bot.0,
            entity.name.clone(),
            format!("{:?},{:?}", entity.position.x, entity.position.y),
            entity.direction,
        );
        if !seen.insert(key) {
            continue;
        }
        queries.push(PlacementQuery {
            player_id: bot.0,
            item_name: entity.name.clone(),
            position: entity.position.clone(),
            direction: entity.direction,
        });
    }
    queries
}

/// A `Position` as a Lua table `{ x = ..., y = ... }`.
///
/// `Position` has no `IntoLua` impl of its own (only `FromLuaMulti`, for the
/// `x, y` argument pairs elsewhere in this crate), so this is written by
/// hand rather than reused.
pub(super) fn position_to_lua(lua: &Lua, pos: &Position) -> LuaResult<LuaTable> {
    let t = lua.create_table()?;
    t.set("x", pos.x)?;
    t.set("y", pos.y)?;
    Ok(t)
}

/// The name an [`InventorySlot`] is exposed under in Lua.
///
/// Deliberately not [`InventorySlot::defines_key`]: that name is the game's
/// own `defines.inventory` key, which Factorio 2.0 unified so that
/// `FurnaceSource` and `AssemblerInput` both read `crafter_input` -- fine for
/// the executor, which only needs to open the right inventory, but a Lua
/// script asking "was this a furnace or an assembler" deserves the answer the
/// planner itself uses to tell them apart.
fn slot_name(slot: InventorySlot) -> &'static str {
    match slot {
        InventorySlot::Chest => "chest",
        InventorySlot::FurnaceSource => "furnace_source",
        InventorySlot::FurnaceResult => "furnace_result",
        InventorySlot::Fuel => "fuel",
        InventorySlot::AssemblerInput => "assembler_input",
        InventorySlot::AssemblerOutput => "assembler_output",
        InventorySlot::LabInput => "lab_input",
    }
}

/// Builds one step's Lua table.
///
/// `bot`, `start` and `finish` come straight off [`ScheduledStep`]. `finish`,
/// never `end`: `end` is a Lua keyword, so `step.end` does not parse, and a
/// field reachable only as `step["end"]` is a trap rather than an API. Every
/// other field is looked up by joining `StepKind::Act`'s bare [`ActionId`]
/// against `net`, per the table in the module doc.
fn step_to_lua(lua: &Lua, net: &ActionNetwork, step: &ScheduledStep) -> LuaResult<LuaTable> {
    let t = lua.create_table()?;
    t.set("bot", step.bot.0)?;
    let start: Ticks = step.start;
    let finish: Ticks = step.end;
    t.set("start", start)?;
    t.set("finish", finish)?;
    match &step.what {
        StepKind::Walk {
            to,
            min_radius,
            radius,
        } => {
            t.set("kind", "walk")?;
            t.set("to", position_to_lua(lua, to)?)?;
            // Additive: `to` keeps the meaning every existing script reads it
            // with. `radius` is the tolerance the walk's own precondition
            // asked for, so a script can tell "stand on this" from "stand
            // near this" -- which for a place/insert/remove is the difference
            // between a reachable request and the entity's own tile.
            t.set("radius", *radius)?;
            // And `min_radius` is the other half of the same precondition:
            // how close is *too* close. A placement's target is ground the
            // acting bot must not be standing on, and a step that published
            // only `radius` said the opposite -- that distance zero was fine.
            t.set("min_radius", *min_radius)?;
        }
        StepKind::Act { action, label } => {
            let action_id: ActionId = *action;
            let act = net.action(action_id).ok_or_else(|| {
                goal_error(format!(
                    "plan: action {action_id:?} referenced by the schedule is missing from its own network"
                ))
            })?;
            t.set("id", action_id.0)?;
            t.set("label", label.clone())?;
            // Predecessor ids, ascending -- what `record.plan_created` needs
            // to draw the DAG rather than just a list of steps. Only actions
            // have ids to depend on or to be depended on, so a walk (handled
            // above, before this arm) carries no `deps` key at all rather
            // than an empty one: the two would otherwise be indistinguishable
            // to a reader, and only one of them is "this step waits on
            // nothing".
            let deps: Vec<u32> = net
                .preds(action_id)
                .into_iter()
                .map(|(id, _)| id.0)
                .collect();
            t.set("deps", deps)?;
            match &act.kind {
                ActionKind::Mine { pos, item, count } => {
                    t.set("kind", "mine")?;
                    t.set("pos", position_to_lua(lua, pos)?)?;
                    t.set("item", item.clone())?;
                    t.set("count", *count)?;
                }
                // Its own kind rather than a second `"mine"`, and it carries
                // `entity` as well as `item`: for an ore the two are one name
                // and a script reading `item` learns everything, for a tree
                // they differ and collapsing them would tell the script either
                // the wrong name or the wrong item. `count` is entities here,
                // which is why the kind has to be distinguishable at all.
                ActionKind::Chop {
                    pos,
                    entity,
                    item,
                    count,
                } => {
                    t.set("kind", "chop")?;
                    t.set("pos", position_to_lua(lua, pos)?)?;
                    t.set("entity", entity.clone())?;
                    t.set("item", item.clone())?;
                    t.set("count", *count)?;
                }
                ActionKind::Craft { item, count } => {
                    t.set("kind", "craft")?;
                    t.set("item", item.clone())?;
                    t.set("count", *count)?;
                }
                ActionKind::Place { entity } => {
                    t.set("kind", "place")?;
                    t.set("entity", entity.name.clone())?;
                    t.set("pos", position_to_lua(lua, &entity.position)?)?;
                }
                ActionKind::Insert {
                    pos,
                    entity,
                    slot,
                    item,
                    count,
                } => {
                    t.set("kind", "insert")?;
                    t.set("pos", position_to_lua(lua, pos)?)?;
                    t.set("entity", entity.clone())?;
                    t.set("slot", slot_name(*slot))?;
                    t.set("item", item.clone())?;
                    t.set("count", *count)?;
                }
                ActionKind::Remove {
                    pos,
                    entity,
                    slot,
                    item,
                    count,
                } => {
                    t.set("kind", "remove")?;
                    t.set("pos", position_to_lua(lua, pos)?)?;
                    t.set("entity", entity.clone())?;
                    t.set("slot", slot_name(*slot))?;
                    t.set("item", item.clone())?;
                    t.set("count", *count)?;
                }
                ActionKind::Research { tech } => {
                    t.set("kind", "research")?;
                    t.set("tech", tech.clone())?;
                }
                ActionKind::SetRecipe {
                    pos,
                    entity,
                    recipe,
                } => {
                    t.set("kind", "set_recipe")?;
                    t.set("pos", position_to_lua(lua, pos)?)?;
                    t.set("entity", entity.clone())?;
                    t.set("recipe", recipe.clone())?;
                }
                // Its own kind rather than folded into a `walk` step: a walk
                // has no action id and is never dispatched on its own (see
                // `StepKind::Walk`'s own doc), while this is a real,
                // individually-scheduled action -- `crate::enclosure::check`'s
                // evacuation -- that happens to ask the game for nothing but
                // the walk its own `AtPosition` precondition already caused.
                ActionKind::Evacuate { to } => {
                    t.set("kind", "evacuate")?;
                    t.set("pos", position_to_lua(lua, to)?)?;
                }
            }
        }
    }
    Ok(t)
}

/// A predicate table's key, as a plain `String`. `count`/`find` predicates are
/// always string-keyed field names, never anything else.
fn require_predicate_key(key: &LuaValue) -> LuaResult<String> {
    match key {
        LuaValue::String(s) => Ok(s.to_string_lossy()),
        other => Err(goal_error(format!(
            "plan predicate keys must be strings, got a {}",
            other.type_name()
        ))),
    }
}

/// Does `step` (a table built by [`step_to_lua`]) match every key in
/// `predicate`?
///
/// Every key is validated before any comparison result is trusted: a step
/// that already fails to match on one key still has its remaining keys
/// checked against [`KNOWN_PREDICATE_KEYS`], so a typo'd key raises
/// regardless of where in the predicate it sits or whether an earlier key
/// already decided the match.
///
/// A step lacking a field the predicate asks about (e.g. `item` on a `walk`
/// step) is simply not a match -- not an error -- exactly like a normal Lua
/// table with a missing key reading `nil`.
fn step_matches(step: &LuaTable, predicate: &LuaTable) -> LuaResult<bool> {
    let mut matches = true;
    for pair in predicate.pairs::<LuaValue, LuaValue>() {
        let (key, want) = pair?;
        let key = require_predicate_key(&key)?;
        if key == "pos" || key == "to" {
            return Err(goal_error(format!(
                "plan predicate: \"{key}\" is a table and cannot be compared; filter on another field instead"
            )));
        }
        if !KNOWN_PREDICATE_KEYS.contains(&key.as_str()) {
            return Err(goal_error(format!(
                "plan predicate: unknown field \"{key}\""
            )));
        }
        let have: LuaValue = step.get(key.as_str())?;
        if have != want {
            matches = false;
        }
    }
    Ok(matches)
}

impl LuaUserData for PlanValue {
    fn add_fields<F: LuaUserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("makespan", |_, this| -> LuaResult<Ticks> {
            Ok(this.schedule.makespan)
        });
        fields.add_field_method_get("bots", |lua, this| {
            let t = lua.create_table()?;
            for (i, bot) in this.origin.roster.iter().enumerate() {
                t.set(i + 1, bot.0)?;
            }
            Ok(t)
        });
        fields.add_field_method_get("steps", |lua, this| {
            let t = lua.create_table()?;
            for (i, step) in this.schedule.steps.iter().enumerate() {
                t.set(i + 1, step_to_lua(lua, &this.net, step)?)?;
            }
            Ok(t)
        });
    }

    fn add_methods<M: LuaUserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("count", |lua, this, predicate: LuaTable| {
            let mut n: i64 = 0;
            for step in &this.schedule.steps {
                let t = step_to_lua(lua, &this.net, step)?;
                if step_matches(&t, &predicate)? {
                    n += 1;
                }
            }
            Ok(n)
        });
        methods.add_method("find", |lua, this, predicate: LuaTable| {
            let out = lua.create_table()?;
            let mut i = 1i64;
            for step in &this.schedule.steps {
                let t = step_to_lua(lua, &this.net, step)?;
                if step_matches(&t, &predicate)? {
                    out.set(i, t)?;
                    i += 1;
                }
            }
            Ok(out)
        });
        methods.add_method("for_bot", |lua, this, bot: i64| {
            let out = lua.create_table()?;
            let mut i = 1i64;
            // A bot id that does not fit a `BotId` (negative, or beyond
            // `u8::MAX`) simply matches no step -- exactly like a valid but
            // absent bot id (`for_bot(99)` in the tests) -- rather than an
            // error: "no such bot" and "this bot has no steps" are the same
            // outcome from a script's point of view.
            if let Ok(bot) = u8::try_from(bot) {
                for step in this.schedule.steps_for(BotId(bot)) {
                    out.set(i, step_to_lua(lua, &this.net, step)?)?;
                    i += 1;
                }
            }
            Ok(out)
        });
        methods.add_method("graphviz", |_, this, ()| Ok(graphviz(&this.net)));
        methods.add_method("gantt", |_, this, title: String| {
            Ok(mermaid_gantt(&this.schedule, &title))
        });
        methods.add_meta_method(LuaMetaMethod::ToString, |_, this, ()| {
            Ok(format!(
                "plan: {} steps, makespan {}",
                this.schedule.steps.len(),
                this.schedule.makespan
            ))
        });
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::globals::goal::create_lua_goal_with;
    use crate::globals::goal::tests::{
        Failure, StubActuator, factory, seeded_world_for, test_origin,
    };
    use factorio_bot_core::factorio::rcon::PlacementVerdict;
    use factorio_bot_core::factorio::world::{PlacementRefusal, RefusalSource};
    use std::future::Future;
    use std::pin::Pin;

    /// The three branches of the production narration, chosen by the two
    /// numbers and nothing else.
    ///
    /// The line itself goes to stdout through `paris` and is not read back
    /// here -- what is worth pinning is the *decision*, and that is the part a
    /// change could get wrong: reporting "nothing left to build" for a factory
    /// that is half there is exactly the indistinguishability this narration
    /// exists to remove.
    #[test]
    fn production_progress_reports_what_stands_against_what_is_wanted() {
        use factorio_bot_planner::method::produce::{DRILL, FURNACE, plan_cell};
        use factorio_bot_planner::{BotId, PlanState};

        let world = seeded_world_for(&[1]);
        let mut state = PlanState::from_world(world, &[BotId(1)]);
        let goal = |per_minute| Goal::Producing {
            item: "iron-plate".into(),
            per_minute,
        };

        // Nothing stands.
        assert_eq!(
            production_progress(&goal(30), &state),
            vec![Production {
                item: "iron-plate".into(),
                per_minute: 30,
                standing: 0,
                wanted: 2,
            }]
        );

        // One cell stands: the partial branch, and the one that must not read
        // as "done".
        let spec = cell_spec(&state, "iron-plate").expect("iron plate smelts from one ore");
        // `want = 1`: this test asks where a cell stands, not how much ore
        // it must sit on (`bdec88af` made siting yield-aware).
        let cell = plan_cell(
            &state,
            &factorio_bot_core::types::Position::new(0., 0.),
            &spec,
            1,
        )
        .expect("the fixture has iron ore");
        for (name, position, facing) in [
            (DRILL, cell.drill.clone(), cell.facing),
            (
                FURNACE,
                cell.furnace.clone(),
                factorio_bot_core::types::Direction::North,
            ),
        ] {
            let entity_type = state
                .base()
                .entity_prototypes
                .get(name)
                .map(|p| p.entity_type.clone())
                .expect("the fixture carries both prototypes");
            state.create_entity(factorio_bot_core::types::FactorioEntity {
                name: name.into(),
                entity_type,
                position,
                direction: factorio_bot_core::num_traits::ToPrimitive::to_u8(&facing).unwrap_or(0),
                ..Default::default()
            });
        }
        assert_eq!(
            production_progress(&goal(30), &state)[0].standing,
            1,
            "one of two, which must narrate as partial rather than as done"
        );
        assert_eq!(
            production_progress(&goal(15), &state)[0],
            Production {
                item: "iron-plate".into(),
                per_minute: 15,
                standing: 1,
                wanted: 1,
            },
            "and the same cell satisfies the smaller rate outright"
        );

        // A goal no cell can make says nothing at all: `expand` refuses it by
        // name a moment later, and saying so twice is noise.
        assert!(production_progress(&goal_for("iron-gear-wheel"), &state).is_empty());
        // So does a bundle of goals with no production in it.
        assert!(
            production_progress(
                &Goal::All(vec![Goal::Researched("automation".into())]),
                &state
            )
            .is_empty()
        );
        // And a production goal nested in a bundle is still found.
        assert_eq!(
            production_progress(&Goal::All(vec![goal(15)]), &state).len(),
            1
        );
    }

    fn goal_for(item: &str) -> Goal {
        Goal::Producing {
            item: item.into(),
            per_minute: 15,
        }
    }

    /// A hand-built network and schedule covering every step kind, so the
    /// step-shape test does not depend on what the planner happens to emit.
    ///
    /// Bot 1 takes every action step; bot 2 takes the walk, which also gives
    /// `for_bot` two bots to separate.
    fn every_kind() -> PlanValue {
        use factorio_bot_core::types::{Direction, FactorioEntity, Position};
        use factorio_bot_planner::{Action, ActionKind, InventorySlot, ScheduledStep, StepKind};

        let kinds = vec![
            ActionKind::Mine {
                pos: Position::new(1.0, 1.0),
                item: "iron-ore".into(),
                count: 3,
            },
            ActionKind::Craft {
                item: "iron-plate".into(),
                count: 2,
            },
            ActionKind::Place {
                entity: Box::new(FactorioEntity::new_stone_furnace(
                    &Position::new(2.0, 2.0),
                    Direction::North,
                )),
            },
            ActionKind::Insert {
                pos: Position::new(2.0, 2.0),
                entity: "stone-furnace".into(),
                slot: InventorySlot::FurnaceSource,
                item: "iron-ore".into(),
                count: 3,
            },
            ActionKind::Remove {
                pos: Position::new(2.0, 2.0),
                entity: "stone-furnace".into(),
                slot: InventorySlot::FurnaceResult,
                item: "iron-plate".into(),
                count: 2,
            },
            ActionKind::Research {
                tech: "automation".into(),
            },
        ];

        let mut net = ActionNetwork::default();
        let mut steps = Vec::new();
        for (i, kind) in kinds.into_iter().enumerate() {
            let id = ActionId(i as u32);
            let label = format!("step {i}");
            net.add(Action {
                id,
                kind,
                pre: vec![],
                eff: vec![],
                duration: 10,
                pinned: None,
                label: label.clone(),
            });
            steps.push(ScheduledStep {
                what: StepKind::Act { action: id, label },
                bot: BotId(1),
                start: (i as Ticks) * 10,
                end: (i as Ticks) * 10 + 10,
            });
        }
        steps.push(ScheduledStep {
            what: StepKind::Walk {
                to: Position::new(5.0, 5.0),
                min_radius: 1.25,
                radius: 7.5,
            },
            bot: BotId(2),
            start: 0,
            end: 34,
        });

        let schedule = Schedule {
            makespan: 60,
            steps,
        };
        PlanValue::new(
            Arc::new(net),
            Arc::new(schedule),
            test_origin(&[BotId(1), BotId(2)]),
        )
    }

    /// Installs one plan as the global `p` in a sandboxed interpreter.
    fn lua_with_plan(plan: PlanValue) -> Lua {
        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        lua.globals().set("p", plan).expect("install p");
        lua
    }

    #[test]
    fn every_step_kind_carries_its_documented_fields() {
        let lua = lua_with_plan(every_kind());
        lua.load(
            r#"
            local by_kind = {}
            for _, s in ipairs(p.steps) do by_kind[s.kind] = s end
            for _, k in ipairs{"walk","mine","craft","place","insert","remove","research"} do
                assert(by_kind[k], "missing kind " .. k)
                local s = by_kind[k]
                assert(s.bot and s.start and s.finish, k .. " lacks common fields")
                assert(s["end"] == nil, "end is a Lua keyword and must not be a field")
            end
            assert(by_kind.walk.to.x, "walk carries to")
            -- The tolerance the walk exists to satisfy, beside the thing it
            -- must get near. Read as a number, not merely present: a `to`
            -- with no radius is how "stand within 7.5 of the furnace" used to
            -- reach the game as "stand on the furnace".
            assert(by_kind.walk.radius == 7.5,
              "walk carries its radius, got " .. tostring(by_kind.walk.radius))
            -- And its inner bound. Publishing only the outer one said a
            -- placement could be made from distance zero, which is the tile
            -- the placement is going on.
            assert(by_kind.walk.min_radius == 1.25,
              "walk carries its min_radius, got " .. tostring(by_kind.walk.min_radius))
            assert(by_kind.walk.id == nil, "a walk is not an action")
            assert(by_kind.mine.item and by_kind.mine.count and by_kind.mine.pos)
            assert(by_kind.craft.item and by_kind.craft.count)
            assert(by_kind.place.entity and by_kind.place.pos)
            assert(by_kind.insert.slot and by_kind.insert.entity and by_kind.insert.item)
            assert(by_kind.remove.slot and by_kind.remove.entity and by_kind.remove.item)
            assert(by_kind.research.tech)
            assert(by_kind.place.id ~= nil and by_kind.place.label ~= nil)
        "#,
        )
        .exec()
        .expect("script");
    }

    #[test]
    fn count_and_find_match_on_every_predicate_key() {
        let lua = lua_with_plan(every_kind());
        lua.load(
            r#"
            assert(p:count{ kind = "place" } == 1)
            assert(p:count{ kind = "place", entity = "stone-furnace" } == 1)
            assert(p:count{ kind = "place", entity = "iron-chest" } == 0)
            assert(#p:find{ kind = "mine" } == 1)
            assert(p:find{ kind = "mine" }[1].item == "iron-ore")
        "#,
        )
        .exec()
        .expect("script");
    }

    #[test]
    fn every_field_a_step_actually_has_is_accepted_as_a_predicate_key() {
        // `KNOWN_PREDICATE_KEYS` is a hand-written mirror of the fields
        // `step_to_lua` sets, and mirrors go stale silently: nothing forces
        // a field added to a step to also be added to the accepted-key
        // list, and the two never disagreeing has never actually been
        // checked before this test. `every_kind()` covers all seven step
        // kinds, so the union of its steps' own keys *is* the ground truth
        // for what a predicate should be allowed to name.
        let lua = lua_with_plan(every_kind());
        lua.load(
            r#"
            -- Collect one sample value for every key that appears on any step.
            local sample = {}
            for _, s in ipairs(p.steps) do
                for k, v in pairs(s) do
                    if sample[k] == nil then sample[k] = v end
                end
            end

            local checked = 0
            for k, v in pairs(sample) do
                -- pos and to are tables and are documented as non-comparable.
                if type(v) ~= "table" then
                    local ok, err = pcall(function() return p:count{ [k] = v } end)
                    assert(ok, "field '" .. k .. "' appears on a step but is refused "
                               .. "as a predicate key: " .. tostring(err))
                    checked = checked + 1
                end
            end
            assert(checked >= 8, "expected to check most step fields, only saw " .. checked)
        "#,
        )
        .exec()
        .expect("script");
    }

    #[test]
    fn an_unknown_predicate_key_raises() {
        let lua = lua_with_plan(every_kind());
        let err = lua
            .load(r#"p:count{ kind = "place", entty = "stone-furnace" }"#)
            .exec()
            .expect_err("typo")
            .to_string();
        assert!(err.contains("entty"), "{err} must name the bad key");
    }

    #[test]
    fn for_bot_returns_that_bots_steps_in_start_order() {
        let lua = lua_with_plan(every_kind());
        lua.load(
            r#"
            local one = p:for_bot(1)
            assert(#one == 6, "bot 1 has every action step, got " .. #one)
            for i = 2, #one do
                assert(one[i].start >= one[i - 1].start, "steps come back in start order")
                assert(one[i].bot == 1, "for_bot(1) returns only bot 1")
            end
            assert(#p:for_bot(2) == 1, "bot 2 has only the walk")
            assert(p:for_bot(2)[1].kind == "walk")
            assert(#p:for_bot(99) == 0, "a bot with no steps is empty, not an error")
        "#,
        )
        .exec()
        .expect("script");
    }

    #[test]
    fn makespan_and_bots_are_readable_fields() {
        let lua = lua_with_plan(every_kind());
        lua.load(
            r#"
            assert(p.makespan == 60, "makespan, got " .. tostring(p.makespan))
            assert(#p.bots == 2 and p.bots[1] == 1 and p.bots[2] == 2, "roster")
            assert(#p.steps == 7, "six actions and one walk, got " .. #p.steps)
        "#,
        )
        .exec()
        .expect("script");
    }

    #[test]
    fn a_plan_can_only_be_taken_for_a_run_once() {
        let plan = every_kind();
        plan.reserve_for_run()
            .expect("first reservation")
            .take()
            .expect("first take");
        let err = plan
            .reserve_for_run()
            .err()
            .expect("a second reservation must be refused")
            .to_string();
        assert!(err.contains("already"), "{err}");
    }

    /// A reservation that is never taken leaves the plan runnable.
    ///
    /// This is the whole reason [`RunSlot`] exists: `goal.start` reserves,
    /// then can still fail on something that is not the plan, and the plan
    /// must survive that untouched.
    #[test]
    fn a_reservation_that_is_dropped_does_not_spend_the_plan() {
        let plan = every_kind();
        drop(plan.reserve_for_run().expect("reservation"));
        plan.reserve_for_run()
            .expect("the plan is still runnable")
            .take()
            .expect("and can still be taken");
    }

    #[test]
    fn renderers_are_lazy_and_produce_output() {
        let lua = lua_with_plan(every_kind());
        lua.load(
            r#"
            assert(type(p.graphviz) == "function", "graphviz is a method, not a string")
            assert(p:graphviz():find("digraph"))
            assert(p:gantt("t"):find("gantt"))
        "#,
        )
        .exec()
        .expect("script");
    }

    /// `slot_name`'s seven strings are a public API contract: a script writes
    /// `plan:count{ slot = "furnace_source" }` against them, and nothing but
    /// this test pins the exact spelling. The match itself is exhaustive, so
    /// the compiler already refuses a build that adds an `InventorySlot`
    /// variant without a `slot_name` arm -- that direction needs no test.
    /// Only the string *values* are unguarded, and a later refactor could
    /// rename one silently with the rest of the suite green.
    ///
    /// These are the Rust enum's own names, deliberately not the game's
    /// `defines.inventory` names: Factorio 2.1 maps both `FurnaceSource` and
    /// `AssemblerInput` to the single define `crafter_input`, so using the
    /// game's names here would collide two distinct slots into one string and
    /// lose exactly the information a script asks `slot` for.
    #[test]
    fn slot_names_are_pinned_to_their_exact_strings() {
        assert_eq!(slot_name(InventorySlot::Chest), "chest");
        assert_eq!(slot_name(InventorySlot::FurnaceSource), "furnace_source");
        assert_eq!(slot_name(InventorySlot::FurnaceResult), "furnace_result");
        assert_eq!(slot_name(InventorySlot::Fuel), "fuel");
        assert_eq!(slot_name(InventorySlot::AssemblerInput), "assembler_input");
        assert_eq!(
            slot_name(InventorySlot::AssemblerOutput),
            "assembler_output"
        );
        assert_eq!(slot_name(InventorySlot::LabInput), "lab_input");
    }

    // ---------------------------------------------------------------- goal.plan

    /// The real `goal` table over a world seeded with `roster`, in a sandbox.
    ///
    /// Reuses `seeded_world_for`, `factory` and `StubActuator` from
    /// `goal::tests` (bumped to `pub(crate)` for this). The table is the one
    /// `create_lua_goal_with` returns, unmodified: it now carries the
    /// value-based `have`/`researched`/`all` that `goal.plan` consumes, so
    /// nothing has to be installed on top of it any more.
    fn lua_with_world(roster: &[u8]) -> Lua {
        lua_with_world_and_checker(seeded_world_for(roster), roster, None)
    }

    /// [`lua_with_world`] over a chosen world and with a chosen pre-flight
    /// placement checker.
    ///
    /// `None` is what every test that predates the pre-check passes, and is
    /// also what production uses when there is no game to ask -- so those
    /// tests exercise the same code path a headless run takes, not a special
    /// one.
    fn lua_with_world_and_checker(
        world: Arc<FactorioWorld>,
        roster: &[u8],
        checker: Option<PlacementChecker>,
    ) -> Lua {
        lua_with_world_checker_and_refresher(world, roster, checker, None)
    }

    fn lua_with_world_checker_and_refresher(
        world: Arc<FactorioWorld>,
        roster: &[u8],
        checker: Option<PlacementChecker>,
        refresher: Option<BufferRefresher>,
    ) -> Lua {
        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        lua.set_app_data(crate::lua_runner::PendingWork::default());
        let table = create_lua_goal_with(
            &lua,
            world,
            factory(Arc::new(StubActuator::new(Failure::Never))),
            roster.to_vec(),
            checker,
            refresher,
        )
        .expect("goal table");
        lua.globals().set("goal", table).expect("install");
        lua
    }

    /// A [`BufferRefresher`] that counts its calls and answers `outcome`.
    fn stub_refresher(
        outcome: Result<usize, &'static str>,
    ) -> (BufferRefresher, Arc<std::sync::atomic::AtomicUsize>) {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = calls.clone();
        let refresher: BufferRefresher = Arc::new(move || {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let outcome = outcome.map_err(|e| e.to_string());
            Box::pin(async move { outcome })
                as Pin<Box<dyn Future<Output = Result<usize, String>> + Send>>
        });
        (refresher, calls)
    }

    // ------------------------------------------- goal.plan's buffer refresh

    /// **The wiring, stated as a test.** `goal.plan` asks the game what is in
    /// the buffers before it plans.
    ///
    /// Without this call every piece of the buffer machinery is correct,
    /// tested and inert: `FactorioWorld::inventories` stays empty,
    /// `PlanState::has_buffers` answers `false`, `Withdraw` claims nothing,
    /// and the replan mines ore that may already be gone. That failure looks
    /// exactly like the code working, which is why the call is pinned rather
    /// than left to the wiring being obviously there.
    #[tokio::test]
    async fn planning_reads_the_buffers_first() {
        let world = seeded_world_for(&[1, 2]);
        let (refresher, calls) = stub_refresher(Ok(3));
        let lua = lua_with_world_checker_and_refresher(world, &[1, 2], None, Some(refresher));
        lua.load(r#"p = goal.plan(goal.have("iron-plate", 8))"#)
            .exec_async()
            .await
            .expect("plan");
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "one refresh per goal.plan"
        );
    }

    /// Once per `goal.plan`, not once per re-siting round.
    ///
    /// The cost claim, and the reason the call sits outside the loop. What
    /// separates two rounds is the refusal ledger the previous round wrote,
    /// and nothing in that changes what is standing in a chest -- so a second
    /// round trip would buy an answer the first one already gave.
    /// `RefuseEverything` drives the loop to its full budget, so a refresh
    /// mistakenly placed inside it would be called three times here.
    #[tokio::test]
    async fn the_buffers_are_read_once_however_many_times_the_plan_is_re_sited() {
        let world = seeded_world_for(&[1, 2]);
        let (checker, log) = stub_checker(world.clone(), Policy::RefuseEverything);
        let (refresher, calls) = stub_refresher(Ok(2));
        let lua =
            lua_with_world_checker_and_refresher(world, &[1, 2], Some(checker), Some(refresher));
        lua.load(r#"p = goal.plan(goal.have("iron-plate", 8))"#)
            .exec_async()
            .await
            .expect("plan");
        assert_eq!(
            log.lock().unwrap_or_else(|e| e.into_inner()).calls.len(),
            MAX_RESITE_ROUNDS + 1,
            "the fixture must really drive the re-site loop, or this proves nothing"
        );
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the buffers are read once, before the loop, not once per round"
        );
    }

    /// A refresh that cannot be made does not stop a run.
    ///
    /// The plan that comes back is the plan this call would have returned
    /// before buffers were visible at all. It is *not* a free failure the way
    /// a missed pre-check is -- the pre-check narrows a window, while this
    /// decides whether a plan re-mines ore that may be gone -- so it is
    /// narrated in those terms. What is pinned here is only that it does not
    /// raise: `goal.plan` answers with a plan or not at all, and a script that
    /// caught a raise here would treat an unreachable game as an unsatisfiable
    /// goal.
    #[tokio::test]
    async fn a_refresh_that_fails_still_hands_back_a_plan() {
        let world = seeded_world_for(&[1, 2]);
        let (refresher, calls) = stub_refresher(Err("rcon is not connected"));
        let lua = lua_with_world_checker_and_refresher(world, &[1, 2], None, Some(refresher));
        let steps: usize = lua
            .load(r#"p = goal.plan(goal.have("iron-plate", 8)) return #p.steps"#)
            .eval_async()
            .await
            .expect("a failed refresh must not raise");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(steps > 0, "the plan is the one we would have had anyway");
    }

    /// No RCON, no refresh, no complaint.
    ///
    /// `goal.plan` is callable with no game behind it (`--clients 0` plans
    /// against invented players). An absent refresher is that mode, not a
    /// failure, and it must not narrate a warning about buffers to a run that
    /// never had a game to ask.
    #[tokio::test]
    async fn planning_without_a_refresher_is_not_a_failure() {
        let world = seeded_world_for(&[1, 2]);
        let lua = lua_with_world_checker_and_refresher(world, &[1, 2], None, None);
        let steps: usize = lua
            .load(r#"p = goal.plan(goal.have("iron-plate", 8)) return #p.steps"#)
            .eval_async()
            .await
            .expect("no refresher is a mode, not an error");
        assert!(steps > 0);
    }

    // ------------------------------------------- goal.plan's placement pre-check

    /// The question list is the plan's own placements: deduplicated, ordered,
    /// and silent about a `Place` nobody is scheduled to make.
    ///
    /// Order matters because the whole reply is joined back by index, and
    /// determinism matters because two runs of the same plan must ask the same
    /// questions -- otherwise the round-trip count, and which sites get
    /// learned, would depend on iteration order.
    #[test]
    fn the_query_list_is_the_plans_own_placements_deduplicated_and_ordered() {
        use factorio_bot_core::types::{FactorioEntity, Position};
        use factorio_bot_planner::{Action, ActionKind, ScheduledStep, StepKind};

        let furnace = |x: f64, y: f64| ActionKind::Place {
            entity: Box::new(FactorioEntity::new_stone_furnace(
                &Position::new(x, y),
                factorio_bot_core::types::Direction::North,
            )),
        };
        let mut net = ActionNetwork::default();
        // 0 and 1 are the same site scheduled twice; 2 is a second site; 3 is
        // a placement no bot is scheduled to make.
        for (id, kind) in [
            (0u32, furnace(1., 1.)),
            (1, furnace(1., 1.)),
            (2, furnace(5., 5.)),
            (3, furnace(9., 9.)),
            (
                4,
                ActionKind::Craft {
                    item: "iron-plate".to_string(),
                    count: 1,
                },
            ),
        ] {
            net.add(Action {
                id: ActionId(id),
                kind,
                pre: vec![],
                eff: vec![],
                duration: 10,
                pinned: None,
                label: format!("step {id}"),
            });
        }
        let sched = Schedule {
            steps: [0u32, 1, 2, 4]
                .iter()
                .map(|id| ScheduledStep {
                    what: StepKind::Act {
                        action: ActionId(*id),
                        label: format!("step {id}"),
                    },
                    bot: BotId(3),
                    start: 0,
                    end: 10,
                })
                .collect(),
            makespan: 10,
        };

        let queries = placement_queries(&net, &sched);
        assert_eq!(
            queries.len(),
            2,
            "one question per distinct site, and none for the unscheduled one: {queries:?}"
        );
        assert_eq!(queries[0].position, Position::new(1., 1.));
        assert_eq!(queries[1].position, Position::new(5., 5.));
        assert!(
            queries.iter().all(|q| q.player_id == 3),
            "a BotId is a player id; there is no mapping layer"
        );
    }

    /// What a stub checker answers.
    #[derive(Clone, Copy, PartialEq)]
    enum Policy {
        /// Every site is fine. The common case, and the one the cost claim is
        /// about: one round trip, no re-expansion.
        Allow,
        /// The first distinct site ever asked about is refused; everything
        /// else is allowed. Models a single unmodelled obstacle.
        RefuseFirstSite,
        /// Every site is refused, forever. Models the pathological case the
        /// round budget exists for.
        RefuseEverything,
        /// Every site is refused *because a character is standing in it* --
        /// which is not a fact about the ground and must change nothing.
        CharacterEverywhere,
        /// The game cannot be reached.
        Fail,
    }

    /// Everything one stub checker remembers about how it was used.
    #[derive(Default)]
    struct CheckerLog {
        /// One entry per call, holding that call's queries. Its length is the
        /// round-trip count, which is the cost claim this whole design rests
        /// on.
        calls: Vec<Vec<PlacementQuery>>,
        /// The first site ever asked about, for [`Policy::RefuseFirstSite`].
        first_site: Option<Position>,
    }

    /// A [`PlacementChecker`] that answers by `policy` and records what it was
    /// asked.
    ///
    /// It writes durable refusals into `world` itself, because that is the
    /// contract the production checker has (see [`PlacementChecker`], and
    /// `FactorioRcon::can_place_entities`, which does the same thing at the
    /// point the game's answer arrives). A stub that answered but recorded
    /// nothing would let `plan_verified` loop forever on a world that never
    /// learns, and would be testing a checker nobody has.
    fn stub_checker(
        world: Arc<FactorioWorld>,
        policy: Policy,
    ) -> (PlacementChecker, Arc<std::sync::Mutex<CheckerLog>>) {
        let log = Arc::new(std::sync::Mutex::new(CheckerLog::default()));
        let checker_log = log.clone();
        let checker: PlacementChecker = Arc::new(move |queries: Vec<PlacementQuery>| {
            let world = world.clone();
            let log = checker_log.clone();
            let mut verdicts = Vec::with_capacity(queries.len());
            {
                let mut log = log.lock().unwrap_or_else(|e| e.into_inner());
                if log.first_site.is_none() {
                    log.first_site = queries.first().map(|q| q.position.clone());
                }
                let first = log.first_site.clone();
                log.calls.push(queries.clone());
                for query in &queries {
                    let refuse = match policy {
                        Policy::Allow | Policy::Fail => false,
                        Policy::RefuseEverything | Policy::CharacterEverywhere => true,
                        Policy::RefuseFirstSite => first.as_ref() == Some(&query.position),
                    };
                    let verdict = PlacementVerdict {
                        ok: !refuse,
                        character: refuse && policy == Policy::CharacterEverywhere,
                        blockers: if refuse {
                            vec!["tree-01".to_string()]
                        } else {
                            Vec::new()
                        },
                        tile: refuse.then(|| "grass-1".to_string()),
                        error: None,
                    };
                    if verdict.is_durable_refusal() {
                        world.record_placement_refusal(PlacementRefusal {
                            tick: Some(6198),
                            entity: query.item_name.clone(),
                            position: query.position.clone(),
                            direction: Some(query.direction),
                            source: RefusalSource::PreCheck,
                            blockers: verdict.blockers.clone(),
                            tile: verdict.tile.clone(),
                        });
                    }
                    verdicts.push(verdict);
                }
            }
            Box::pin(async move {
                if policy == Policy::Fail {
                    return Err("rcon is not connected".to_string());
                }
                Ok(verdicts)
            })
                as Pin<Box<dyn Future<Output = Result<Vec<PlacementVerdict>, String>> + Send>>
        });
        (checker, log)
    }

    /// The `x` of every `place` step in the plan bound to `p`, as the script
    /// sees them.
    const PLACE_SITES: &str = r#"
        local sites = {}
        for _, s in ipairs(p:find{ kind = "place" }) do
            sites[#sites + 1] = s.pos.x .. "," .. s.pos.y
        end
        table.sort(sites)
        return table.concat(sites, " ")
    "#;

    async fn plan_sites(lua: &Lua, goal: &str) -> String {
        lua.load(format!("p = goal.plan({goal})\n{PLACE_SITES}"))
            .eval_async()
            .await
            .expect("plan")
    }

    /// **The cost claim.** A plan whose sites are all legal costs exactly one
    /// round trip, whatever it contains.
    ///
    /// One call, and its queries are precisely the placements the plan chose
    /// -- not one per candidate the search considered, and not one per step.
    #[tokio::test]
    async fn a_legal_plan_costs_one_round_trip_naming_only_the_sites_it_chose() {
        let world = seeded_world_for(&[1, 2]);
        let (checker, log) = stub_checker(world.clone(), Policy::Allow);
        let lua = lua_with_world_and_checker(world, &[1, 2], Some(checker));
        let sites = plan_sites(&lua, r#"goal.have("iron-plate", 8)"#).await;
        assert!(!sites.is_empty(), "the fixture plan must place something");

        let log = log.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(
            log.calls.len(),
            1,
            "one round trip for a plan with no problem"
        );
        let asked: Vec<String> = {
            let mut asked: Vec<String> = log.calls[0]
                .iter()
                .map(|q| format!("{:?},{:?}", q.position.x, q.position.y))
                .collect();
            asked.sort();
            asked
        };
        assert_eq!(
            asked.join(" "),
            sites,
            "the questions are exactly the plan's own placements"
        );
    }

    /// A plan with nothing to place asks nothing at all.
    #[tokio::test]
    async fn a_plan_with_no_placement_makes_no_round_trip() {
        let world = seeded_world_for(&[1, 2]);
        let (checker, log) = stub_checker(world.clone(), Policy::Allow);
        let lua = lua_with_world_and_checker(world, &[1, 2], Some(checker));
        lua.load(r#"p = goal.plan(goal.have("iron-ore", 2))"#)
            .exec_async()
            .await
            .expect("plan");
        assert!(
            log.lock()
                .unwrap_or_else(|e| e.into_inner())
                .calls
                .is_empty(),
            "an empty query list must not become an empty call"
        );
    }

    /// **The defect this exists to remove.** A site the game would refuse is
    /// never handed to the executor.
    ///
    /// Before the pre-check, the refusal cost a dispatched action, its
    /// dependents, and a recovery escalation -- five separate runs ended that
    /// way. Now it costs one extra round trip and one re-expansion, and the
    /// plan `goal.plan` returns sites somewhere else.
    #[tokio::test]
    async fn a_site_the_game_would_refuse_never_reaches_the_returned_plan() {
        let world = seeded_world_for(&[1, 2]);
        let (checker, log) = stub_checker(world.clone(), Policy::RefuseFirstSite);
        let lua = lua_with_world_and_checker(world.clone(), &[1, 2], Some(checker));
        let sites = plan_sites(&lua, r#"goal.have("iron-plate", 8)"#).await;

        let log = log.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(log.calls.len(), 2, "asked, re-sited, asked again");
        let refused = log.first_site.clone().expect("a site was asked about");
        assert!(
            !sites.contains(&format!("{:?},{:?}", refused.x, refused.y)),
            "the refused site {refused:?} is still in the returned plan: {sites}"
        );
        assert!(
            !sites.is_empty(),
            "re-siting produces a plan, not an empty one"
        );

        let learned = world.placement_refusals();
        assert_eq!(learned.len(), 1, "and the site is remembered: {learned:?}");
        assert_eq!(learned[0].position, refused);
        assert_eq!(learned[0].source, RefusalSource::PreCheck);
        assert_eq!(learned[0].blockers, vec!["tree-01".to_string()]);
    }

    /// Re-siting is bounded, and running out of budget is not an error.
    ///
    /// A world where every site is refused would otherwise loop forever. The
    /// budget turns that into three round trips and a plan handed back
    /// unchanged, which is exactly the behaviour that existed before the
    /// pre-check: the dispatch-time refusal path is still there and still
    /// catches it. A pre-check must never be able to fail a run that would
    /// otherwise have merely stumbled.
    #[tokio::test]
    async fn re_siting_is_bounded_and_running_out_of_rounds_still_returns_a_plan() {
        let world = seeded_world_for(&[1, 2]);
        let (checker, log) = stub_checker(world.clone(), Policy::RefuseEverything);
        let lua = lua_with_world_and_checker(world, &[1, 2], Some(checker));
        lua.load(r#"p = goal.plan(goal.have("iron-plate", 8))"#)
            .exec_async()
            .await
            .expect("a plan still comes back");
        assert_eq!(
            log.lock().unwrap_or_else(|e| e.into_inner()).calls.len(),
            MAX_RESITE_ROUNDS + 1,
            "one query per expansion, and no more expansions than the budget allows"
        );
    }

    /// A character in the footprint changes nothing: not the ledger, not the
    /// plan, not the number of round trips.
    ///
    /// It is the one blocker that moves on its own, `PlanState::from_world`
    /// already re-reads every character from the world on every plan, and at
    /// pre-check time the acting bot has not walked to the site yet. Re-siting
    /// on it would also reopen the very loop this design closes: the planner
    /// would flee ground that is fine, one tile per iteration, against the
    /// supervisor's stall limit.
    #[tokio::test]
    async fn a_character_in_the_footprint_neither_re_sites_nor_is_remembered() {
        let world = seeded_world_for(&[1, 2]);
        let (unchecked, _) = stub_checker(world.clone(), Policy::Allow);
        let baseline = lua_with_world_and_checker(world.clone(), &[1, 2], Some(unchecked));
        let expected = plan_sites(&baseline, r#"goal.have("iron-plate", 8)"#).await;

        let world = seeded_world_for(&[1, 2]);
        let (checker, log) = stub_checker(world.clone(), Policy::CharacterEverywhere);
        let lua = lua_with_world_and_checker(world.clone(), &[1, 2], Some(checker));
        let sites = plan_sites(&lua, r#"goal.have("iron-plate", 8)"#).await;

        assert_eq!(
            sites, expected,
            "the plan is the one that would have been made"
        );
        assert_eq!(
            log.lock().unwrap_or_else(|e| e.into_inner()).calls.len(),
            1,
            "no re-expansion: nothing was learned to re-expand around"
        );
        assert!(
            world.placement_refusals().is_empty(),
            "a bot standing there is not a fact about the ground"
        );
    }

    /// A pre-check that cannot be made is not a planning failure.
    ///
    /// The plan is the plan this call would have returned before the
    /// pre-check existed, and the dispatch-time refusal path is untouched. An
    /// unreachable game, or a `workspace/mods` copy older than this binary,
    /// therefore costs the behaviour we already had rather than a raise from
    /// a call that used to be infallible.
    #[tokio::test]
    async fn a_checker_that_cannot_reach_the_game_does_not_stop_planning() {
        let world = seeded_world_for(&[1, 2]);
        let (unchecked, _) = stub_checker(world.clone(), Policy::Allow);
        let baseline = lua_with_world_and_checker(world.clone(), &[1, 2], Some(unchecked));
        let expected = plan_sites(&baseline, r#"goal.have("iron-plate", 8)"#).await;

        let world = seeded_world_for(&[1, 2]);
        let (checker, _) = stub_checker(world.clone(), Policy::Fail);
        let lua = lua_with_world_and_checker(world.clone(), &[1, 2], Some(checker));
        let sites = plan_sites(&lua, r#"goal.have("iron-plate", 8)"#).await;
        assert_eq!(sites, expected);
        assert!(world.placement_refusals().is_empty());
    }

    /// The same plan, with and without a checker installed, is the same plan.
    ///
    /// The control for everything above: a build with no game to ask takes
    /// the `None` path, and it must not be a different planner.
    #[tokio::test]
    async fn no_checker_plans_exactly_as_an_all_clear_checker_does() {
        let world = seeded_world_for(&[1, 2]);
        let (checker, _) = stub_checker(world.clone(), Policy::Allow);
        let checked = lua_with_world_and_checker(world, &[1, 2], Some(checker));
        let with = plan_sites(&checked, r#"goal.have("iron-plate", 8)"#).await;

        let bare = lua_with_world(&[1, 2]);
        let without = plan_sites(&bare, r#"goal.have("iron-plate", 8)"#).await;
        assert_eq!(with, without);
    }

    #[tokio::test]
    async fn plan_defaults_to_the_whole_roster() {
        let lua = lua_with_world(&[1, 2, 3, 4]);
        lua.load(
            r#"
            local p = goal.plan(goal.have("iron-plate", 8))
            assert(#p.bots == 4, "defaults to every bot, got " .. #p.bots)
            for i = 1, 4 do assert(p.bots[i] == i, "roster is 1..4 in order") end
        "#,
        )
        .exec_async()
        .await
        .expect("script");
    }

    #[tokio::test]
    async fn plan_honours_a_bot_subset() {
        let lua = lua_with_world(&[1, 2, 3, 4]);
        lua.load(
            r#"
            local p = goal.plan(goal.have("iron-plate", 8), { bots = { 1, 2 } })
            assert(#p.bots == 2, "two bots asked for, got " .. #p.bots)
            assert(#p.steps > 0, "a two-bot plan is still a plan")
            for _, s in ipairs(p.steps) do
                assert(s.bot == 1 or s.bot == 2, "no step may land on bot " .. s.bot)
            end
        "#,
        )
        .exec_async()
        .await
        .expect("script");
    }

    #[tokio::test]
    async fn expansion_and_scheduling_always_share_one_roster() {
        // The regression this design exists for. A four-bot world, planned
        // for one bot, must schedule every action of its own network onto
        // that bot and satisfy every precondition -- not fail on a
        // precondition about an item three other bots were carrying.
        let lua = lua_with_world(&[1, 2, 3, 4]);
        lua.load(
            r#"
            local p = goal.plan(goal.have("iron-plate", 8), { bots = { 1 } })
            assert(#p.bots == 1 and p.bots[1] == 1)
            for _, s in ipairs(p.steps) do assert(s.bot == 1, "every step on bot 1") end
            assert(#p.steps > 0, "a one-bot plan is still a plan")
        "#,
        )
        .exec_async()
        .await
        .expect("script");
    }

    #[tokio::test]
    async fn semantic_errors_raise_at_plan_time_not_construction() {
        let lua = lua_with_world(&[1, 2]);
        lua.load(
            r#"
            -- Constructing is pure: an unknown item is not a shape error.
            local g = goal.have("not-a-real-item", 1)
            assert(g.item == "not-a-real-item", "construction succeeds")
            local ok, err = pcall(goal.plan, g)
            assert(not ok, "planning an unknown item must raise")
            assert(tostring(err):find("not%-a%-real%-item"), "the error names it: " .. tostring(err))

            local t = goal.researched("no-such-technology")
            local ok2, err2 = pcall(goal.plan, t)
            assert(not ok2, "planning an unknown technology must raise")
            assert(tostring(err2):find("no%-such%-technology"), "names it: " .. tostring(err2))
        "#,
        )
        .exec_async().await
        .expect("script");
    }

    #[tokio::test]
    async fn an_unknown_bot_raises_and_names_it() {
        let lua = lua_with_world(&[1, 2]);
        lua.load(
            r#"
            local ok, err = pcall(goal.plan, goal.have("iron-plate", 1), { bots = { 99 } })
            assert(not ok, "bot 99 is not a connected player")
            assert(tostring(err):find("99"), "the error names the bot: " .. tostring(err))
        "#,
        )
        .exec_async()
        .await
        .expect("script");
    }

    #[tokio::test]
    async fn every_action_step_carries_its_predecessor_ids() {
        // `record.plan_created` needs the DAG, not just a step list: an
        // action step's `deps` is the network's own `preds`, and a smelt
        // chain (mine ore -> place furnace -> insert -> remove) has real
        // edges to report. A walk carries no `deps` key at all -- it has no
        // action id to be a predecessor of, or to depend on one.
        let lua = lua_with_world(&[1, 2, 3, 4]);
        lua.load(
            r#"
            local p = goal.plan(goal.have("iron-plate", 8))
            local any_deps = false
            for _, s in ipairs(p.steps) do
                if s.kind == "walk" then
                    assert(s.deps == nil, "a walk has no deps key")
                else
                    assert(type(s.deps) == "table", s.kind .. " must carry a deps table")
                    if #s.deps > 0 then any_deps = true end
                end
            end
            assert(any_deps, "a smelt chain has at least one real dependency edge")
        "#,
        )
        .exec_async()
        .await
        .expect("script");
    }

    #[tokio::test]
    async fn an_empty_bot_list_raises() {
        let lua = lua_with_world(&[1, 2]);
        lua.load(
            r#"
            local ok, err = pcall(goal.plan, goal.have("iron-plate", 1), { bots = {} })
            assert(not ok, "an empty roster cannot plan anything")
            assert(tostring(err):find("bot"), "the error is about bots: " .. tostring(err))
        "#,
        )
        .exec_async()
        .await
        .expect("script");
    }
}
