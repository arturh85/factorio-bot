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
use super::{expand_goal, goal_error, refuse_unknown_bots};
use factorio_bot_core::factorio::world::FactorioWorld;
use factorio_bot_core::mlua::prelude::*;
use factorio_bot_core::types::Position;
use factorio_bot_executor::{ExecutionLog, Recovery};
use factorio_bot_planner::ids::{ActionId, BotId};
use factorio_bot_planner::{
    ActionKind, ActionNetwork, Goal, InventorySlot, PlanState, Schedule, ScheduledStep, StepKind,
    Ticks, graphviz, mermaid_gantt, schedule,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// The predicate keys `count`/`find` understand. `pos` and `to` are
/// deliberately absent -- they are tables, not comparable values, and are
/// rejected with a dedicated message rather than falling through to "unknown
/// field".
const KNOWN_PREDICATE_KEYS: &[&str] = &[
    "kind", "bot", "start", "finish", "id", "label", "entity", "item", "count", "tech", "slot",
    // A walk's tolerance. A plain number, so unlike `to` it compares; the
    // comparison is exact equality on an `f64`, which is what a caller asking
    // `{ radius = 10 }` means and all this predicate language offers.
    "radius",
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
) -> LuaResult<()> {
    table.set(
        "plan",
        lua.create_function(move |_lua, (g, opts): (LuaTable, Option<LuaTable>)| {
            let goal = goal_from_lua(&g)?;
            let roster = resolve_roster(opts.as_ref(), &default_roster)?;
            // Built once and reused for the scheduler below; `expand_goal`
            // builds its own copy internally to run the same refusal, which
            // is redundant but harmless: `PlanState::from_world` is a pure
            // read of the world snapshot.
            let state = PlanState::from_world(world.clone(), &roster);
            refuse_unknown_bots(&state)?;
            let net = expand_goal(goal.clone(), &world, &roster)?;
            let scheduled = schedule(&net, &state, &roster).map_err(goal_error)?;
            // The goal, the world and the roster are kept together on the
            // plan, not because dispatching needs them -- it does not -- but
            // because `obs:recover()` will, one run later, and a recovery must
            // re-plan the same goal for the same roster or it is answering a
            // different question.
            Ok(PlanValue::new(
                Arc::new(net),
                Arc::new(scheduled),
                Arc::new(PlanOrigin {
                    goal,
                    world: world.clone(),
                    roster,
                }),
            ))
        })?,
    )?;
    Ok(())
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
        StepKind::Walk { to, radius } => {
            t.set("kind", "walk")?;
            t.set("to", position_to_lua(lua, to)?)?;
            // Additive: `to` keeps the meaning every existing script reads it
            // with. `radius` is the tolerance the walk's own precondition
            // asked for, so a script can tell "stand on this" from "stand
            // near this" -- which for a place/insert/remove is the difference
            // between a reachable request and the entity's own tile.
            t.set("radius", *radius)?;
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
        let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
        lua.set_app_data(crate::lua_runner::PendingWork::default());
        let table = create_lua_goal_with(
            &lua,
            seeded_world_for(roster),
            factory(Arc::new(StubActuator::new(Failure::Never))),
            roster.to_vec(),
        )
        .expect("goal table");
        lua.globals().set("goal", table).expect("install");
        lua
    }

    #[test]
    fn plan_defaults_to_the_whole_roster() {
        let lua = lua_with_world(&[1, 2, 3, 4]);
        lua.load(
            r#"
            local p = goal.plan(goal.have("iron-plate", 8))
            assert(#p.bots == 4, "defaults to every bot, got " .. #p.bots)
            for i = 1, 4 do assert(p.bots[i] == i, "roster is 1..4 in order") end
        "#,
        )
        .exec()
        .expect("script");
    }

    #[test]
    fn plan_honours_a_bot_subset() {
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
        .exec()
        .expect("script");
    }

    #[test]
    fn expansion_and_scheduling_always_share_one_roster() {
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
        .exec()
        .expect("script");
    }

    #[test]
    fn semantic_errors_raise_at_plan_time_not_construction() {
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
        .exec()
        .expect("script");
    }

    #[test]
    fn an_unknown_bot_raises_and_names_it() {
        let lua = lua_with_world(&[1, 2]);
        lua.load(
            r#"
            local ok, err = pcall(goal.plan, goal.have("iron-plate", 1), { bots = { 99 } })
            assert(not ok, "bot 99 is not a connected player")
            assert(tostring(err):find("99"), "the error names the bot: " .. tostring(err))
        "#,
        )
        .exec()
        .expect("script");
    }

    #[test]
    fn every_action_step_carries_its_predecessor_ids() {
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
        .exec()
        .expect("script");
    }

    #[test]
    fn an_empty_bot_list_raises() {
        let lua = lua_with_world(&[1, 2]);
        lua.load(
            r#"
            local ok, err = pcall(goal.plan, goal.have("iron-plate", 1), { bots = {} })
            assert(not ok, "an empty roster cannot plan anything")
            assert(tostring(err):find("bot"), "the error is about bots: " .. tostring(err))
        "#,
        )
        .exec()
        .expect("script");
    }
}
