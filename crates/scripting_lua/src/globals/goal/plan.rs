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
//! Not yet reachable from Lua: nothing constructs a [`PlanValue`] outside this
//! module's own tests. A later task adds `goal.plan`, which builds one from an
//! expanded, scheduled goal.

use super::goal_error;
use factorio_bot_core::mlua::prelude::*;
use factorio_bot_core::types::Position;
use factorio_bot_planner::ids::{ActionId, BotId};
use factorio_bot_planner::{
    graphviz, mermaid_gantt, ActionKind, ActionNetwork, InventorySlot, Schedule, ScheduledStep,
    StepKind, Ticks,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// The predicate keys `count`/`find` understand. `pos` and `to` are
/// deliberately absent -- they are tables, not comparable values, and are
/// rejected with a dedicated message rather than falling through to "unknown
/// field".
#[allow(dead_code)]
const KNOWN_PREDICATE_KEYS: &[&str] = &[
    "kind", "bot", "start", "finish", "id", "label", "entity", "item", "count", "tech", "slot",
];

/// An expanded, scheduled plan: an [`ActionNetwork`] plus the [`Schedule`]
/// that assigned its actions to bots, held together so Lua can inspect either
/// side of the join `step_to_lua` performs.
///
/// `net` and `schedule` are `Arc`s rather than owned values because
/// `goal.execute` (a later task, mirroring today's `Plans::scheduled` in
/// `mod.rs`) needs exactly these two handed to the executor without cloning
/// the graph or the schedule.
#[allow(dead_code)]
pub(crate) struct PlanValue {
    net: Arc<ActionNetwork>,
    schedule: Arc<Schedule>,
    /// The bots the plan was scheduled for, in roster order. Exposed as
    /// `plan.bots` and walked by `for_bot`.
    roster: Vec<BotId>,
    /// Set the first time [`PlanValue::take_for_run`] succeeds. A plan may
    /// only be run once -- running it twice would dispatch every action
    /// against the game a second time -- and this flag is where that rule
    /// lives, which is why the run side (`RunValue`, a later task) needs no
    /// registry of its own to enforce it.
    consumed: AtomicBool,
}

impl PlanValue {
    #[allow(dead_code)]
    pub(crate) fn new(
        net: Arc<ActionNetwork>,
        schedule: Arc<Schedule>,
        roster: Vec<BotId>,
    ) -> Self {
        Self {
            net,
            schedule,
            roster,
            consumed: AtomicBool::new(false),
        }
    }

    /// Hands the plan's network and schedule to a caller about to execute it,
    /// refusing a second call with an error naming the reason.
    #[allow(dead_code)]
    pub(crate) fn take_for_run(&self) -> LuaResult<(Arc<ActionNetwork>, Arc<Schedule>)> {
        if self.consumed.swap(true, Ordering::SeqCst) {
            return Err(goal_error("plan has already been taken for a run"));
        }
        Ok((self.net.clone(), self.schedule.clone()))
    }
}

/// A `Position` as a Lua table `{ x = ..., y = ... }`.
///
/// `Position` has no `IntoLua` impl of its own (only `FromLuaMulti`, for the
/// `x, y` argument pairs elsewhere in this crate), so this is written by
/// hand rather than reused.
#[allow(dead_code)]
fn position_to_lua(lua: &Lua, pos: &Position) -> LuaResult<LuaTable> {
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
#[allow(dead_code)]
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
#[allow(dead_code)]
fn step_to_lua(lua: &Lua, net: &ActionNetwork, step: &ScheduledStep) -> LuaResult<LuaTable> {
    let t = lua.create_table()?;
    t.set("bot", step.bot.0)?;
    let start: Ticks = step.start;
    let finish: Ticks = step.end;
    t.set("start", start)?;
    t.set("finish", finish)?;
    match &step.what {
        StepKind::Walk { to } => {
            t.set("kind", "walk")?;
            t.set("to", position_to_lua(lua, to)?)?;
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
#[allow(dead_code)]
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
#[allow(dead_code)]
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
            for (i, bot) in this.roster.iter().enumerate() {
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
            },
            bot: BotId(2),
            start: 0,
            end: 34,
        });

        let schedule = Schedule {
            makespan: 60,
            steps,
        };
        PlanValue::new(Arc::new(net), Arc::new(schedule), vec![BotId(1), BotId(2)])
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
        plan.take_for_run().expect("first");
        let err = plan.take_for_run().expect_err("second").to_string();
        assert!(err.contains("already"), "{err}");
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
}
