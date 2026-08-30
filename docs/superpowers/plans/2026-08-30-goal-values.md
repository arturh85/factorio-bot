# Goal Values Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the handle-based `goal.*` Lua API with one where goals, plans and observations are inspectable values, so long-horizon goals can be scripted, asserted on, and tested without a running game.

**Architecture:** Goals become plain Lua tables built by validating constructors. `goal.plan` does expansion and scheduling in a single call and returns a `PlanValue` userdata holding both the `ActionNetwork` and the `Schedule`; its `steps` field is a join over the two. `goal.start` consumes a plan and returns a `RunValue` userdata. The `Plans` and `Runs` handle registries are deleted — userdata carries the state that integer handles were indexing.

**Tech Stack:** Rust, `mlua` (Lua 5.4) with `LuaUserData`, `factorio_bot_planner`, `factorio_bot_executor`, `tokio`.

**Spec:** `docs/superpowers/specs/2026-08-30-goal-values-design.md`

## Global Constraints

- `crates/planner` is **pure**: no I/O, no async, no wall-clock, no randomness. This plan does not change it.
- Ordered collections only where iteration order is observable: `BTreeMap` / `BTreeSet` / `Vec`. Floats compare with `total_cmp`.
- The executor issues only legitimate player actions. No `cheat_*` RCON calls, ever.
- **Do not touch** `crates/server/**`, `app/src/**`, or `crates/scripting_lua/src/run_script.rs` — a peer session owns them.
- Tick fields exposed to Lua are named `planned_start` / `planned_end`. **Never** `observed_*`: they are scheduler estimates, and `crates/executor/src/log.rs` documents why a measurement is not available yet.
- `finish`, never `end`, for a step's end tick — `end` is a Lua keyword and `step.end` does not parse.
- Every Lua-facing error goes through `goal_error` so scripts see one error shape.
- Commit after each task. The final script-migration commit carries a `BREAKING CHANGE:` footer.
- Workspace must be green after every task: `cargo clippy --workspace --all-features --all-targets -- --deny warnings` and `cargo nextest run` (or `cargo test`).

## File Structure

`crates/scripting_lua/src/globals/goal.rs` (1992 lines) becomes a module directory. The split is by responsibility, and each new file is created by the task that needs it:

- `goal/mod.rs` — `create_lua_goal`, `create_lua_goal_with`, the doc table, `goal_error`, `expand_goal`, `refuse_unknown_bots`. The registration surface and nothing else.
- `goal/value.rs` — the goal constructors and `Goal` conversion (Task 1).
- `goal/plan.rs` — `PlanValue`, the step join, predicates, renderers (Tasks 2-3).
- `goal/run.rs` — `RunValue`, observations, the actuator drive loop (Task 4).

Tests live beside their subject in each file's `mod tests`.

---

### Task 1: Goal values

**Files:**
- Create: `crates/scripting_lua/src/globals/goal/value.rs`
- Move: `crates/scripting_lua/src/globals/goal.rs` → `crates/scripting_lua/src/globals/goal/mod.rs`

Do the move with `git mv` as the **first** action of this task, before any edit, so the
history follows the file:

```bash
mkdir -p crates/scripting_lua/src/globals/goal
git mv crates/scripting_lua/src/globals/goal.rs crates/scripting_lua/src/globals/goal/mod.rs
```

Then add `mod value;` to it. Nothing else in `mod.rs` changes in this task.

**Interfaces:**
- Consumes: `factorio_bot_planner::{Goal, Holder, ids::BotId}`; `goal_error` from `goal/mod.rs`.
- Produces:
  - `pub(crate) fn install_goal_constructors(lua: &Lua, table: &LuaTable) -> LuaResult<()>` — sets `have`, `researched`, `all` on the goal table.
  - `pub(crate) fn goal_from_lua(value: &LuaTable) -> LuaResult<Goal>` — converts a goal table to a planner `Goal`, raising on a malformed one.

**Background.** A goal table has a `kind` field (`"have"` / `"researched"` / `"all"`) and a metatable carrying `__tostring`. Constructors validate eagerly so a mistake raises on the line that contains it. `goal_from_lua` validates again, because a Lua table is open and a script may hand-build one; both raise the same shape of error.

- [ ] **Step 1: Write the failing tests**

Create `crates/scripting_lua/src/globals/goal/value.rs` with only a `mod tests` and these tests. Use a bare `Lua` with the constructors installed.

```rust
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn lua_with_goal() -> Lua {
        let lua = Lua::new();
        let table = lua.create_table().unwrap();
        install_goal_constructors(&lua, &table).unwrap();
        lua.globals().set("goal", table).unwrap();
        lua
    }

    #[test]
    fn have_builds_an_inspectable_table() {
        let lua = lua_with_goal();
        lua.load(r#"
            local g = goal.have("iron-plate", 5)
            assert(g.kind == "have", "kind")
            assert(g.item == "iron-plate", "item")
            assert(g.count == 5, "count")
            assert(g.bot == nil, "no bot means anyone")
        "#).exec().expect("script");
    }

    #[test]
    fn have_targets_a_named_bot() {
        let lua = lua_with_goal();
        lua.load(r#"
            local g = goal.have("coal", 2, { bot = 3 })
            assert(g.bot == 3, "bot")
        "#).exec().expect("script");
    }

    #[test]
    fn shape_errors_raise_at_construction() {
        let lua = lua_with_goal();
        for (src, want) in [
            (r#"goal.have("iron-plate", 0)"#, "count"),
            (r#"goal.have("iron-plate", -1)"#, "count"),
            (r#"goal.have("", 1)"#, "item"),
            (r#"goal.researched("")"#, "technology"),
            (r#"goal.all({})"#, "at least one"),
            (r#"goal.all({ 42 })"#, "goal"),
            (r#"goal.have("iron-plate", 1, { bot = 0 })"#, "bot"),
        ] {
            let err = lua.load(src).exec().expect_err(src).to_string();
            assert!(err.contains(want), "{src}: {err} lacks {want}");
        }
    }

    #[test]
    fn goals_nest_and_render() {
        let lua = lua_with_goal();
        lua.load(r#"
            local g = goal.all { goal.have("iron-plate", 5), goal.researched("automation") }
            assert(g.kind == "all", "kind")
            assert(#g.goals == 2, "two sub-goals")
            assert(tostring(g):find("automation"), "tostring mentions the technology")
        "#).exec().expect("script");
    }

    #[test]
    fn conversion_maps_every_shape_to_the_planner() {
        let lua = lua_with_goal();
        let g: LuaTable = lua.load(r#"
            return goal.all {
                goal.have("iron-plate", 5),
                goal.have("coal", 2, { bot = 3 }),
                goal.researched("automation"),
            }
        "#).eval().expect("script");
        let converted = goal_from_lua(&g).expect("converts");
        assert_eq!(converted, Goal::All(vec![
            Goal::Have { item: "iron-plate".into(), count: 5, whose: Holder::Anyone },
            Goal::Have { item: "coal".into(), count: 2, whose: Holder::Bot(BotId(3)) },
            Goal::Researched("automation".into()),
        ]));
    }

    #[test]
    fn conversion_refuses_a_hand_built_table() {
        let lua = lua_with_goal();
        let t: LuaTable = lua.load(r#"return { kind = "have", item = "iron-plate" }"#)
            .eval().expect("script");
        let err = goal_from_lua(&t).expect_err("no count").to_string();
        assert!(err.contains("count"), "{err}");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p factorio-bot-scripting-lua goal::value -- --nocapture`
Expected: FAIL — `install_goal_constructors` and `goal_from_lua` do not exist.

- [ ] **Step 3: Implement the constructors and conversion**

Write the implementation above the tests. Shape rules, all raising via `goal_error`:

| rule | message must contain |
| --- | --- |
| `item` a non-empty string | `item` |
| `count` an integer `>= 1` | `count` |
| `tech` a non-empty string | `technology` |
| `opts.bot` an integer `>= 1` when present | `bot` |
| `goal.all` given `>= 1` element, each a goal table | `at least one`, `goal` |

Set a shared metatable with `__tostring` producing `have 5 iron-plate`, `researched automation`, `all { … }` (sub-goals joined by `, `). Recursion in `__tostring` and in `goal_from_lua` mirrors `Goal::All`.

`goal_from_lua` maps `bot` present → `Holder::Bot(BotId(n))`, absent → `Holder::Anyone`. It never produces `Holder::Share` — that is the expansion's marker, not a caller's word.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p factorio-bot-scripting-lua goal::value`
Expected: PASS, 6 tests.

- [ ] **Step 5: Prove the tests discriminate**

Mutate `count >= 1` to `count >= 0` and re-run: `shape_errors_raise_at_construction` must fail. Mutate `Holder::Bot` to `Holder::Anyone` in the conversion: `conversion_maps_every_shape_to_the_planner` must fail. Revert both.

- [ ] **Step 6: Commit**

```bash
git add crates/scripting_lua/src/globals/goal.rs crates/scripting_lua/src/globals/goal/value.rs
git commit -m "feat(lua): goals as validating, inspectable values"
```

---

### Task 2: The plan value

**Files:**
- Create: `crates/scripting_lua/src/globals/goal/plan.rs`
- Modify: `crates/scripting_lua/src/globals/goal/mod.rs` (add `mod plan;`)

**Interfaces:**
- Consumes: `factorio_bot_planner::{ActionNetwork, Schedule, ScheduledStep, StepKind, ActionKind, ids::{ActionId, BotId}}`.
- Produces: `pub(crate) struct PlanValue { net: Arc<ActionNetwork>, schedule: Arc<Schedule>, roster: Vec<BotId>, consumed: AtomicBool }` implementing `LuaUserData`, plus `impl PlanValue { pub(crate) fn new(net, schedule, roster) -> Self; pub(crate) fn take_for_run(&self) -> LuaResult<(Arc<ActionNetwork>, Arc<Schedule>)> }`.

**Background.** `ScheduledStep` carries only `{ what: StepKind, bot, start, end }`, and `StepKind::Act` carries only `{ action: ActionId, label }`. The per-kind fields live on the `Action` in the network, so building a step table is a join: look the `ActionId` up in the network, read its `ActionKind`, flatten. `ActionKind::Place` holds a `Box<FactorioEntity>`; expose its `name` as `entity` and its `position` as `pos`.

`take_for_run` flips `consumed` and returns `Err` if it was already set — this is where "running one plan twice raises" lives, and it is why `RunValue` needs no registry.

- [ ] **Step 1: Write the failing tests**

```rust
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
        use factorio_bot_planner::{Action, ActionKind, InventorySlot, ScheduledStep, StepKind};
        use factorio_bot_core::types::{Direction, FactorioEntity, Position};

        let kinds = vec![
            ActionKind::Mine { pos: Position::new(1.0, 1.0), item: "iron-ore".into(), count: 3 },
            ActionKind::Craft { item: "iron-plate".into(), count: 2 },
            ActionKind::Place {
                entity: Box::new(FactorioEntity::new_stone_furnace(&Position::new(2.0, 2.0), Direction::North)),
            },
            ActionKind::Insert {
                pos: Position::new(2.0, 2.0), entity: "stone-furnace".into(),
                slot: InventorySlot::FurnaceSource, item: "iron-ore".into(), count: 3,
            },
            ActionKind::Remove {
                pos: Position::new(2.0, 2.0), entity: "stone-furnace".into(),
                slot: InventorySlot::FurnaceResult, item: "iron-plate".into(), count: 2,
            },
            ActionKind::Research { tech: "automation".into() },
        ];

        let mut net = ActionNetwork::default();
        let mut steps = Vec::new();
        for (i, kind) in kinds.into_iter().enumerate() {
            let id = ActionId(i as u32);
            let label = format!("step {i}");
            net.add(Action {
                id, kind, pre: vec![], eff: vec![],
                duration: 10, pinned: None, label: label.clone(),
            });
            steps.push(ScheduledStep {
                what: StepKind::Act { action: id, label },
                bot: BotId(1),
                start: (i as Ticks) * 10,
                end: (i as Ticks) * 10 + 10,
            });
        }
        steps.push(ScheduledStep {
            what: StepKind::Walk { to: Position::new(5.0, 5.0) },
            bot: BotId(2), start: 0, end: 34,
        });

        let schedule = Schedule { makespan: 60, steps };
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
        lua.load(r#"
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
        "#).exec().expect("script");
    }

    #[test]
    fn count_and_find_match_on_every_predicate_key() {
        let lua = lua_with_plan(every_kind());
        lua.load(r#"
            assert(p:count{ kind = "place" } == 1)
            assert(p:count{ kind = "place", entity = "stone-furnace" } == 1)
            assert(p:count{ kind = "place", entity = "iron-chest" } == 0)
            assert(#p:find{ kind = "mine" } == 1)
            assert(p:find{ kind = "mine" }[1].item == "iron-ore")
        "#).exec().expect("script");
    }

    #[test]
    fn an_unknown_predicate_key_raises() {
        let lua = lua_with_plan(every_kind());
        let err = lua.load(r#"p:count{ kind = "place", entty = "stone-furnace" }"#)
            .exec().expect_err("typo").to_string();
        assert!(err.contains("entty"), "{err} must name the bad key");
    }

    #[test]
    fn for_bot_returns_that_bots_steps_in_start_order() {
        let lua = lua_with_plan(every_kind());
        lua.load(r#"
            local one = p:for_bot(1)
            assert(#one == 6, "bot 1 has every action step, got " .. #one)
            for i = 2, #one do
                assert(one[i].start >= one[i - 1].start, "steps come back in start order")
                assert(one[i].bot == 1, "for_bot(1) returns only bot 1")
            end
            assert(#p:for_bot(2) == 1, "bot 2 has only the walk")
            assert(p:for_bot(2)[1].kind == "walk")
            assert(#p:for_bot(99) == 0, "a bot with no steps is empty, not an error")
        "#).exec().expect("script");
    }

    #[test]
    fn makespan_and_bots_are_readable_fields() {
        let lua = lua_with_plan(every_kind());
        lua.load(r#"
            assert(p.makespan == 60, "makespan, got " .. tostring(p.makespan))
            assert(#p.bots == 2 and p.bots[1] == 1 and p.bots[2] == 2, "roster")
            assert(#p.steps == 7, "six actions and one walk, got " .. #p.steps)
        "#).exec().expect("script");
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
        lua.load(r#"
            assert(type(p.graphviz) == "function", "graphviz is a method, not a string")
            assert(p:graphviz():find("digraph"))
            assert(p:gantt("t"):find("gantt"))
        "#).exec().expect("script");
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p factorio-bot-scripting-lua goal::plan`
Expected: FAIL — `PlanValue` does not exist.

- [ ] **Step 3: Implement `PlanValue`**

`impl LuaUserData for PlanValue`:

- `add_field_method_get("makespan", …)` → `schedule.makespan`
- `add_field_method_get("bots", …)` → roster as a Lua array of integers
- `add_field_method_get("steps", …)` → array of step tables, in `schedule.steps` order
- `add_method("count", …)`, `add_method("find", …)`, `add_method("for_bot", …)`
- `add_method("graphviz", …)`, `add_method("gantt", |_, this, title: String|…)`
- `add_meta_method(LuaMetaMethod::ToString, …)` → `plan: <n> steps, makespan <m>`

Step construction (one function, `step_to_lua`), joining `StepKind::Act { action, label }` to `net.action(action)`:

| `ActionKind` | `kind` | fields set |
| --- | --- | --- |
| — (`StepKind::Walk`) | `walk` | `to` |
| `Mine { pos, item, count }` | `mine` | `pos`, `item`, `count` |
| `Craft { item, count }` | `craft` | `item`, `count` |
| `Place { entity }` | `place` | `entity` = `entity.name`, `pos` = `entity.position` |
| `Insert { pos, entity, slot, item, count }` | `insert` | all five, `slot` as its string name |
| `Remove { … }` | `remove` | all five |
| `Research { tech }` | `research` | `tech` |

Predicate matching: for each key in the predicate table, if the key is not one of the known field names (`kind`, `bot`, `start`, `finish`, `id`, `label`, `entity`, `item`, `count`, `tech`, `slot`) raise naming the key; otherwise compare with the step's value, treating a step that lacks the field as not matching. `pos` and `to` are tables and are **not** comparable — raise if used as a predicate key, saying so.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p factorio-bot-scripting-lua goal::plan`
Expected: PASS, 7 tests.

- [ ] **Step 5: Prove discrimination**

Mutate `step_to_lua` to name the end field `end` instead of `finish`: `every_step_kind_carries_its_documented_fields` must fail. Mutate the unknown-key check to accept anything: `an_unknown_predicate_key_raises` must fail. Mutate `take_for_run` to always succeed: `a_plan_can_only_be_taken_for_a_run_once` must fail. Revert all three.

- [ ] **Step 6: Commit**

```bash
git add crates/scripting_lua/src/globals/goal/plan.rs crates/scripting_lua/src/globals/goal/mod.rs
git commit -m "feat(lua): plans as inspectable values with one filterable step collection"
```

---

### Task 3: `goal.plan` — expand and schedule in one call

**Files:**
- Modify: `crates/scripting_lua/src/globals/goal/plan.rs`, `crates/scripting_lua/src/globals/goal/mod.rs`

**Interfaces:**
- Consumes: `expand_goal(goal, world, bots)` and `refuse_unknown_bots` from `goal/mod.rs`; `goal_from_lua` from Task 1; `PlanValue::new` from Task 2.
- Produces: `goal.plan(g, opts?) -> PlanValue` installed on the goal table.

**Background — the defect this removes.** `expand_goal`'s own doc comment describes it: `SplitAcrossBots` sizes each share against the bot it names, so a network expanded for four bots only makes sense on those four. The old API let `goal.have` expand over the full roster and `goal.schedule` assign over a different one. Doing both in one call makes the mismatch unrepresentable. **The test named below is the point of this task.**

- [ ] **Step 1: Write the failing tests**

```rust
/// The real `goal` table over a world seeded with `roster`, in a sandbox.
/// Reuses `seeded_world_for` and `factory` from the existing tests.
fn lua_with_world(roster: &[u8]) -> Lua {
    let lua = crate::sandbox::new_sandboxed_lua().expect("sandbox");
    lua.set_app_data(crate::lua_runner::PendingWork::default());
    let table = create_lua_goal_with(
        &lua,
        seeded_world_for(roster),
        factory(Arc::new(StubActuator::new(Failure::Never))),
        roster.to_vec(),
    ).expect("goal table");
    lua.globals().set("goal", table).expect("install");
    lua
}

#[test]
fn plan_defaults_to_the_whole_roster() {
    let lua = lua_with_world(&[1, 2, 3, 4]);
    lua.load(r#"
        local p = goal.plan(goal.have("iron-plate", 8))
        assert(#p.bots == 4, "defaults to every bot, got " .. #p.bots)
        for i = 1, 4 do assert(p.bots[i] == i, "roster is 1..4 in order") end
    "#).exec().expect("script");
}

#[test]
fn plan_honours_a_bot_subset() {
    let lua = lua_with_world(&[1, 2, 3, 4]);
    lua.load(r#"
        local p = goal.plan(goal.have("iron-plate", 8), { bots = { 1, 2 } })
        assert(#p.bots == 2, "two bots asked for, got " .. #p.bots)
        for _, s in ipairs(p.steps) do
            assert(s.bot == 1 or s.bot == 2, "no step may land on bot " .. s.bot)
        end
    "#).exec().expect("script");
}

#[test]
fn expansion_and_scheduling_always_share_one_roster() {
    // The regression this design exists for. A four-bot world, planned for
    // one bot, must schedule every action of its own network onto that bot
    // and satisfy every precondition -- not fail on a precondition about an
    // item three other bots were carrying.
    let lua = lua_with_world(&[1, 2, 3, 4]);
    lua.load(r#"
        local p = goal.plan(goal.have("iron-plate", 8), { bots = { 1 } })
        assert(#p.bots == 1 and p.bots[1] == 1)
        for _, s in ipairs(p.steps) do assert(s.bot == 1, "every step on bot 1") end
        assert(#p.steps > 0, "a one-bot plan is still a plan")
    "#).exec().expect("script");
}

#[test]
fn semantic_errors_raise_at_plan_time_not_construction() {
    let lua = lua_with_world(&[1, 2]);
    lua.load(r#"
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
    "#).exec().expect("script");
}

#[test]
fn an_unknown_bot_raises_and_names_it() {
    let lua = lua_with_world(&[1, 2]);
    lua.load(r#"
        local ok, err = pcall(goal.plan, goal.have("iron-plate", 1), { bots = { 99 } })
        assert(not ok, "bot 99 is not a connected player")
        assert(tostring(err):find("99"), "the error names the bot: " .. tostring(err))
    "#).exec().expect("script");
}

#[test]
fn an_empty_bot_list_raises() {
    let lua = lua_with_world(&[1, 2]);
    lua.load(r#"
        local ok, err = pcall(goal.plan, goal.have("iron-plate", 1), { bots = {} })
        assert(not ok, "an empty roster cannot plan anything")
        assert(tostring(err):find("bot"), "the error is about bots: " .. tostring(err))
    "#).exec().expect("script");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p factorio-bot-scripting-lua goal::plan`
Expected: FAIL — `goal.plan` is not installed.

- [ ] **Step 3: Implement `goal.plan`**

```
1. goal_from_lua(g)                          -- shape re-check
2. roster = opts.bots (list of integers) or the run's full roster
   - empty list -> raise
   - not a list of positive integers -> raise
3. state = PlanState::from_world(world, &roster); refuse_unknown_bots(&state)
4. net = expand_goal(goal, &world, &roster)  -- maps planner errors via goal_error
5. schedule = schedule(&net, &state, &roster).map_err(goal_error)
6. PlanValue::new(Arc::new(net), Arc::new(schedule), roster)
```

Read `schedule`'s exact signature from `crates/planner/src/schedule.rs` before calling it; do not assume the argument order from this plan.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p factorio-bot-scripting-lua goal::plan`
Expected: PASS, 13 tests (7 from Task 2 + 6 here).

- [ ] **Step 5: Prove discrimination**

Change `goal.plan` to expand against the full roster while scheduling against the subset — the old bug, reintroduced deliberately. `expansion_and_scheduling_always_share_one_roster` must fail. Revert.

- [ ] **Step 6: Commit**

```bash
git add crates/scripting_lua/src/globals/goal/plan.rs crates/scripting_lua/src/globals/goal/mod.rs
git commit -m "feat(lua): goal.plan expands and schedules against one roster"
```

---

### Task 4: Runs and observations

**Files:**
- Create: `crates/scripting_lua/src/globals/goal/run.rs`
- Modify: `crates/scripting_lua/src/globals/goal/mod.rs` (add `mod run;`)

**Interfaces:**
- Consumes: `PlanValue::take_for_run`; `factorio_bot_executor::{run_into, ExecutionLog, Status, Attempt}`; the existing `ActuatorFactory` type in `goal/mod.rs`.
- Produces: `goal.start(plan) -> RunValue`, `goal.run(plan) -> observation`, and `RunValue` userdata with `:progress()` and `:wait()`.

**Test infrastructure.** The old `goal.rs` tests already carry what these need — reuse
them rather than writing new ones: `seeded_world_for(&[u8])`, `seed_players`,
`factory(stub)`, `lua_with_goal(stub)`, the `StubActuator` with its `Failure` modes, and
the timeout runner that turns a hang into a named assertion failure. Two details from
`lua_with_goal` that are easy to lose in the move and fatal to lose: it builds the
interpreter with `crate::sandbox::new_sandboxed_lua()`, and it sets
`crate::lua_runner::PendingWork::default()` as app data — **without that app data a run
cannot start at all**. If no `AlwaysOk` stub exists, add one as the trivial `Actuator`
that returns `Ok(())`.

**Background.** Lift the run machinery from the old `Runs`/`RunEntry` — the `tokio::sync::watch` completion signal and the reason `wait_for_run` clones the receiver rather than holding a lock across the await. That reasoning still applies; only the handle indirection goes. `RunValue` holds `Arc<Mutex<ExecutionLog>>`, the `watch::Receiver<bool>`, and the network (to size the observation).

- [ ] **Step 1: Write the failing tests**

Drive the real bindings against the existing actuator stub (see the old `goal.rs` tests for `Failure` and `factory`; carry that infrastructure over rather than rewriting it).

```rust
#[test]
fn run_reports_a_finished_observation() {
    let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::Never)));
    lua.load(r#"
        local p = goal.plan(goal.have("iron-ore", 2))
        local obs = goal.run(p)
        assert(obs.done, "a returned run is finished")
        assert(obs.failed == 0, "nothing failed, got " .. obs.failed)
        assert(obs.success > 0, "something succeeded")
        assert(obs.pending == 0 and obs.running == 0, "nothing left outstanding")
        assert(obs.first_error == nil, "no error on a clean run")
    "#).exec().expect("script");
}

#[test]
fn an_observation_carries_per_action_outcomes_keyed_by_step_id() {
    let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::Never)));
    lua.load(r#"
        local p = goal.plan(goal.have("iron-ore", 2))
        local obs = goal.run(p)
        local checked = 0
        for _, s in ipairs(p.steps) do
            if s.kind ~= "walk" then
                local a = obs.actions[s.id]
                assert(a, "no outcome for action " .. tostring(s.id))
                assert(a.status == "success", "status for " .. s.id .. ": " .. a.status)
                assert(a.attempts == 1, "one attempt")
                assert(type(a.planned_start) == "number", "planned_start")
                assert(type(a.planned_end) == "number", "planned_end once finished")
                checked = checked + 1
            end
        end
        assert(checked > 0, "the plan had action steps to check")
    "#).exec().expect("script");
}

#[test]
fn tick_fields_are_named_planned_not_observed() {
    let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::Never)));
    lua.load(r#"
        local obs = goal.run(goal.plan(goal.have("iron-ore", 2)))
        for id, a in pairs(obs.actions) do
            assert(a.planned_start ~= nil, "planned_start")
            assert(a.observed_start == nil, "these are estimates, not measurements")
            assert(a.observed_end == nil, "these are estimates, not measurements")
        end
    "#).exec().expect("script");
}

#[test]
fn failures_are_reported_with_their_errors() {
    // `Failure::First` is the existing stub mode that fails the first action
    // dispatched and succeeds thereafter.
    let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::First(AtomicBool::new(false)))));
    lua.load(r#"
        local obs = goal.run(goal.plan(goal.have("iron-ore", 2)))
        assert(obs.failed >= 1, "the stub failed an action, got " .. obs.failed)
        assert(type(obs.first_error) == "string", "first_error is set")
        assert(#obs.first_error > 0, "first_error is not empty")
        local fs = obs:failures()
        assert(#fs == obs.failed, "failures() agrees with the count")
        assert(type(fs[1].error) == "string" and #fs[1].error > 0, "each failure carries its error")
        assert(fs[1].id ~= nil, "each failure names its action")
    "#).exec().expect("script");
}

#[test]
fn start_is_non_blocking_and_progress_reads_it() {
    let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::Never)));
    lua.load(r#"
        local run = goal.start(goal.plan(goal.have("iron-ore", 2)))
        -- Returning at all is the assertion: a blocking start could not reach
        -- this line before the run finished.
        local snap = run:progress()
        assert(type(snap.done) == "boolean", "progress answers with an observation")
        assert(type(snap.success) == "number", "and it carries counts")
        local obs = run:wait()
        assert(obs.done, "wait returns only once the run is over")
        assert(obs.failed == 0, "clean run")
    "#).exec().expect("script");
}

#[test]
fn wait_is_idempotent() {
    let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::Never)));
    lua.load(r#"
        local run = goal.start(goal.plan(goal.have("iron-ore", 2)))
        local a = run:wait()
        local b = run:wait()
        assert(a.done and b.done, "both waits return a finished observation")
        assert(a.success == b.success, "and they agree")
    "#).exec().expect("script");
}

#[test]
fn running_one_plan_twice_raises() {
    let err = lua.load(r#"goal.run(p); goal.run(p)"#).exec().expect_err("twice").to_string();
    assert!(err.contains("already"), "{err}");
}

#[test]
fn goal_run_equals_start_then_wait() {
    let lua = lua_with_goal(Arc::new(StubActuator::new(Failure::Never)));
    lua.load(r#"
        local direct = goal.run(goal.plan(goal.have("iron-ore", 2)))
        local staged = goal.start(goal.plan(goal.have("iron-ore", 2))):wait()
        assert(direct.done == staged.done, "done")
        assert(direct.success == staged.success, "success")
        assert(direct.failed == staged.failed, "failed")
    "#).exec().expect("script");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p factorio-bot-scripting-lua goal::run`
Expected: FAIL — `goal.start` is not installed.

- [ ] **Step 3: Implement runs and observations**

Observation table:

| field | type |
| --- | --- |
| `done` | boolean |
| `pending`, `running`, `success`, `failed` | integers |
| `first_error` | string or nil — the error of the lowest-id failed action, for assert messages |
| `actions` | map `ActionId` → `{ status, attempts, planned_start, planned_end, error }` |
| `failures` | function → array of `{ id, error, status, attempts, planned_start, planned_end }` |

`status` is the lowercase name of `Status` (`"pending"`, `"running"`, `"success"`, `"failed"`). `planned_end` is nil while an attempt is unfinished, mirroring `Attempt::planned_end_tick: Option<Ticks>`.

`goal.run(plan)` is literally `goal.start(plan)` followed by that run's `wait` — implement it by calling the same two functions, not by duplicating the drive loop.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p factorio-bot-scripting-lua goal::run`
Expected: PASS, 8 tests.

- [ ] **Step 5: Prove discrimination**

Rename `planned_start` to `observed_start` in the observation builder: `tick_fields_are_named_planned_not_observed` must fail. Make `first_error` always `nil`: `failures_are_reported_with_their_errors` must fail. Revert both.

- [ ] **Step 6: Commit**

```bash
git add crates/scripting_lua/src/globals/goal/run.rs crates/scripting_lua/src/globals/goal/mod.rs
git commit -m "feat(lua): runs and observations as values"
```

---

### Task 5: Delete the handle surface

**Files:**
- Modify: `crates/scripting_lua/src/globals/goal/mod.rs`

**Interfaces:**
- Produces: a goal table with exactly `have`, `researched`, `all`, `plan`, `run`, `start` (plus the two `__doc__` keys).

**Background.** `Plans`, `Runs`, `PlanEntry`, `RunEntry`, `Progress`, `wait_for_run(runs, handle)` and the eight old binding closures exist only to make integer handles work. Userdata replaced all of it. Anything still referenced by the new modules moves rather than dies — check each before deleting.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn the_goal_table_offers_exactly_the_new_surface() {
    let lua = /* full goal table installed */;
    lua.load(r#"
        for _, name in ipairs{"have","researched","all","plan","run","start"} do
            assert(type(goal[name]) == "function", name .. " must exist")
        end
        for _, gone in ipairs{"schedule","graphviz","gantt","execute","progress","wait"} do
            assert(goal[gone] == nil, gone .. " must be gone, not merely deprecated")
        end
    "#).exec().expect("script");
}
```

- [ ] **Step 2: Run to verify failure**

Expected: FAIL — `goal.schedule` still exists.

- [ ] **Step 3: Delete the old surface**

Remove `Plans`, `Runs`, `PlanEntry`, `RunEntry`, `Progress`, the old `wait_for_run`, and the eight closures. Rewrite `__doc__header` to describe values rather than handles — the current text says "Every function that takes a `plan` takes the number returned by `goal.have`", which is now false. Update each remaining function's `__doc__` entry.

`expand_goal`, `refuse_unknown_bots`, `goal_error` and `lock` stay if still used; delete any that are not.

- [ ] **Step 4: Run the whole crate**

Run: `cargo test -p factorio-bot-scripting-lua`
Expected: PASS. Then `cargo clippy --workspace --all-features --all-targets -- --deny warnings` — expected: no warnings. Dead code left behind will show up here.

- [ ] **Step 5: Commit**

```bash
git add crates/scripting_lua/src/globals/goal/mod.rs
git commit -m "refactor(lua): delete the handle-based goal surface"
```

---

### Task 6: Migrate the scripts

**Files:**
- Modify: `scripts/lib.lua`, `scripts/example.lua`, `scripts/api_test.lua`, `scripts/goal_smoke.lua`, `scripts/exec_smoke.lua`, `scripts/attach_smoke.lua`, `scripts/test_phase_2_1.lua`, `scripts/test_phase_2_2.lua`

**Background.** These are the only `goal.*` callers; the HTTP surface calls `run_lua`, not `goal.*`. Each script fails at *runtime*, not at build time, so the migration is not compiler-checked — read every `goal.` line in each file.

Per-file notes:
- `lib.lua:24-25` — `goal.have` then `goal.schedule(plan, #bots)` becomes one `goal.plan(goal.have(...))`, which already defaults to the whole roster; drop the `#bots` argument entirely.
- `test_phase_2_1.lua:48-50` — asserts `goal.gantt` refuses an *unscheduled* plan. That state no longer exists: a `PlanValue` is always scheduled. Replace with an assertion that carries weight in the new model — that `goal.plan` refuses an unknown item, naming it.
- `test_phase_2_2.lua` — `goal.execute` / `goal.progress` / `goal.wait` become `goal.start` / `:progress()` / `:wait()`.
- `goal_smoke.lua:47-51` — the comment explains that `goal.schedule` re-expands for the bots asked for. Rewrite it to say that `goal.plan` does both at once and the mismatch is now unrepresentable.

- [ ] **Step 1: Migrate every call site**

Grep first: `grep -rn "goal\." scripts/` — 8 files, and comments mention the old names too. Update prose comments as well as code; a comment describing a deleted API is worse than no comment.

- [ ] **Step 2: Verify each script parses**

Run: `for f in scripts/*.lua; do luac -p "$f" || echo "FAILED $f"; done` (or `luajit -bl`). If no Lua binary is available, run each planning-only script through the CLI in the next step and treat a parse error as a failure there.

- [ ] **Step 3: Run the planning-only scripts headless**

Run: `cargo run --no-default-features --features cli,lua -- lua scripts/goal_smoke.lua --clients 0 --bots 4`
Expected: completes with no error; assertions inside the script are the check. Repeat for `test_phase_2_1.lua` and `api_test.lua`.

Scripts needing a live game (`exec_smoke.lua`, `test_phase_2_2.lua`, `attach_smoke.lua`) are exercised in Task 7 — note that in the report rather than skipping silently.

- [ ] **Step 4: Add one assertion the old API could not express**

In `goal_smoke.lua`, add a plan-shape assertion — the capability this whole increment buys:

```lua
local p = goal.plan(goal.all { goal.have("iron-plate", 5), goal.researched("automation") })
assert(p:count { kind = "place", entity = "stone-furnace" } <= #p.bots,
       "no more furnaces than bots")
for _, s in ipairs(p:find { kind = "mine" }) do
    assert(s.count > 0, "a mine step for nothing is a planner bug")
end
```

- [ ] **Step 5: Commit with the breaking-change footer**

```bash
git add scripts/
git commit -F - <<'MSG'
feat(lua)!: goals, plans and observations are values

Plans are inspectable rather than opaque: a script can assert on the steps a
goal expands to without a running game, and planning is ~1s headless. Goals
compose through goal.all. goal.plan expands and schedules against one roster,
so a plan expanded for four bots can no longer be scheduled onto one.

BREAKING CHANGE: the handle-based goal API is replaced.

  goal.have(i, c)       -> goal.have(i, c)          returns a value, not a handle
  goal.researched(t)    -> goal.researched(t)       returns a value
  goal.schedule(h, n)   -> goal.plan(g, { bots = ... })
  goal.graphviz(h)      -> plan:graphviz()
  goal.gantt(h, t)      -> plan:gantt(t)
  goal.execute(h)       -> goal.start(plan)
  goal.progress(r)      -> run:progress()
  goal.wait(r)          -> run:wait()
  (new)                    goal.all { ... }
  (new)                    goal.run(plan)           = start then wait
  (new)                    plan:count / plan:find / plan:for_bot
  (new)                    plan.steps / plan.makespan / plan.bots

A plan may be run only once; re-plan to retry. Observation tick fields are
planned_start and planned_end -- scheduler estimates, not measurements.
MSG
```

---

### Task 7: Documentation and a live run

**Files:**
- Modify: `docs/` Lua API pages that name the old functions; `CLAUDE.md` if it describes the goal API.

- [ ] **Step 1: Find every doc reference**

Run: `grep -rn "goal\.\(schedule\|graphviz\|gantt\|execute\|progress\|wait\)" docs/ CLAUDE.md README.md`
Update each. The generated Lua API docs come from the `__doc__` entries rewritten in Task 5 — regenerate rather than hand-editing if a generator exists.

- [ ] **Step 2: Full workspace check**

Run: `just test`
Expected: clippy clean, all tests pass, build succeeds.

- [ ] **Step 3: Drive it against a live game**

Start a server and run `exec_smoke.lua`. **Execution has never completed a real action**, so treat a failure as the expected outcome and the diagnostics as the deliverable. Record in the report: which action failed, its `error` string from the observation, and whether the failure is in the actuator, the mod, or the plan.

Do not fix live-execution defects inside this task — file them in the ledger with their evidence. They are their own increment.

- [ ] **Step 4: Commit**

```bash
git add docs/ CLAUDE.md
git commit -m "docs: describe the values-based goal API"
```
