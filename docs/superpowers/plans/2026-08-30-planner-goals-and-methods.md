# Planner Goals and Methods — Implementation Plan (2 of 3)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add layers 1 and 2 to `crates/planner` — declarative `Goal`s, hand-written `Method`s, and an expansion driver — so that `Have(automation-science-pack, 10, Anyone)` decomposes into a schedulable action network across four bots, with no Factorio in the loop.

**Architecture:** A `Goal` is declarative and bot-agnostic. A `Method` expands one goal into `Step`s — subgoals, actions, or explicit lag edges — and a registry picks the first applicable method per goal. The driver recurses until only actions remain, advancing a simulated `PlanState` as it goes so that later siblings see earlier ones' effects, then runs `infer_edges` over the result. Everything feeds the existing `schedule()` unchanged.

**Tech Stack:** Rust 2021, `factorio-bot-core` (recipes, prototypes, entity graph), the `crates/planner` engine from plan 1.

**Spec:** `docs/superpowers/specs/2026-08-29-multi-agent-planner-design.md`

**Prior plan:** `docs/superpowers/plans/2026-08-29-planner-scheduling-engine.md` (complete; `PlanState`, `Action`/`Condition`/`Effect`, `ActionNetwork`, `schedule()`, rendering).

**Follow-on plan (not this one):** (3) RCON execution, `ExecutionLog`, the Lua `plan.*` rewrite, and deleting `core/plan` and `graph/task_graph.rs`.

## The central design decision, stated up front

**Methods emit actions with `Actor::Role` and `pinned: None`.** Nothing in this plan pins an action to a bot.

The obvious alternative — pin each per-bot chain so expansion can simulate one inventory — would leave the scheduler with nothing to assign and defeat layer 4 entirely. It is unnecessary because of how the scheduler now works: a chain's consumer carries `HasItem { who: Role, … }`, and only the bot that actually mined the ore satisfies it, so the scheduler's feasibility check skips every other bot and the chain follows its items. Chains stay coherent without being nailed down, and independent chains still spread across bots because each chain's *first* action is unconstrained and goes to whichever bot is cheapest.

Expansion still needs *some* binding to simulate against, so `ExpansionCtx` carries a `chain_actor: BotId` used **only** for simulating effects during expansion. It never reaches the emitted actions. The assumption this rests on — that bots start interchangeable, so simulating chain *j* against bot *j* predicts what any bot would experience — holds here because `PlanState::from_world` gives every unknown bot the same defaults. **If bots ever start with materially different inventories, this assumption breaks and the driver must be revisited.** Task 2 records that in a doc comment.

## Global Constraints

- Workspace root: `/home/arturh/projects/private/factorio-bot`. Rust edition 2021. Branch `master`.
- **Every cargo invocation must be wrapped:** `nix develop --command bash -c 'eval "$(mise env -s bash)"; <command>'`
- **`cargo fmt -p factorio-bot-planner` before every commit — never `cargo fmt --all`.** Other crates in this checkout belong to other workstreams.
- `cargo clippy --workspace --all-features --all-targets -- --deny warnings --deny deprecated` must pass. It is currently clean.
- **Another agent may be working in this checkout with files staged.** Commit with the partial-commit form: `git commit -m "<message>" -- crates/planner`. This builds the commit from the working tree for that path and ignores the index entirely, so another agent's staged work cannot be swept in.
- **New files are the one exception.** A pathspec commit cannot see an untracked file, so a task that creates one must first run `git add <that exact path>` — naming the file, never a directory — and then commit with the pathspec as usual. Staging one named new file cannot capture anyone else's work. **Never `git add -A`, never `git add .`, never `git commit -a`.**
- Never `git checkout`/`stash`/`reset`/`clean` outside `crates/planner/`, with the single exception of discarding `crates/scripting_lua/tests/` snapshot churn after a workspace-wide test run.
- `Ticks` is `u32`; 60 ticks = 1 second. No `f64` durations — `f64` is for distances and radii only.
- **Multiply durations and item counts with `saturating_mul`, never `*`.** `Goal::Have`'s `count` is an unbounded `u32`, so a plain multiply panics in debug and *silently wraps* in release — turning an absurd request into a plausible-looking but wrong plan. Saturating turns the same request into an obviously broken one (a duration of `u32::MAX` ticks) instead of a subtly wrong one.
- Build structs with functional-update syntax (`Foo { a, ..Default::default() }`), never `let mut x = Foo::default();` plus field assignments. An `#[allow(clippy::field_reassign_with_default)]` is the wrong fix.
- Large enum variants get `Box`ed — clippy denies `large_enum_variant`.
- Determinism is required end to end. Iterate `BTreeMap`/`BTreeSet`/`Vec`, never `HashMap`/`HashSet`, anywhere a result can influence output.
- **Never adjust an assertion to match observed output.** If a stated expectation fails, report the actual value and stop. This has already caught three controller errors across these plans.
- **RED phase for a task that creates a new file:** an orphaned `.rs` file with no `pub mod` line is silently excluded from the build, so the tests in it neither run nor fail. Add the `pub mod` declaration *first*, then run the tests, so the failure you observe is the real one — a missing type, not a missing module.

## Existing API this plan builds on (all committed and reviewed)

```rust
// factorio_bot_planner::
pub type Ticks = u32;  pub type ItemId = String;
pub struct BotId(pub u8);  pub struct ActionId(pub u32);
pub struct ActionIdGen;                  // ::new(), ::next(&mut self) -> ActionId
pub enum Actor { Bound(BotId), Role }
pub enum Condition { HasItem{who,item,count}, AtPosition{who,pos,radius}, EntityAt{pos,name},
                     PositionFree{pos}, Researched(String), ResourceAvailable{pos,item,count} }
pub enum Effect { GainItem{who,item,count}, LoseItem{who,item,count}, CreateEntity(Box<FactorioEntity>),
                  RemoveEntity{pos}, ConsumeResource{pos,item,count}, Researched(String) }
impl Effect { pub fn apply(&self, &mut PlanState, binding: BotId) -> Result<(), PlannerError> }
pub enum ActionKind { Mine{pos,item,count}, Craft{item,count}, Place{entity: Box<FactorioEntity>},
                      Insert{pos,item,count}, Remove{pos,item,count}, Research{tech} }
pub struct Action { pub id, pub kind, pub pre: Vec<Condition>, pub eff: Vec<Effect>,
                    pub duration: Ticks, pub pinned: Option<BotId>, pub label: String }
impl ActionNetwork {
    pub fn new() -> Self;  pub fn add(&mut self, Action) -> ActionId;
    pub fn link(&mut self, from: ActionId, to: ActionId, lag: Ticks);
    pub fn infer_edges(&mut self);  pub fn validate(&self) -> Result<(), PlannerError>;
    pub fn action(&self, ActionId) -> Option<&Action>;  pub fn len(&self) -> usize;
}
impl PlanState {
    pub fn from_world(base: Arc<FactorioWorld>, bots: &[BotId]) -> PlanState;
    pub fn fork(&self) -> PlanState;  pub fn base(&self) -> &Arc<FactorioWorld>;
    pub fn bot(&self, BotId) -> Option<&BotState>;
    pub fn inventory_count(&self, BotId, &str) -> u32;  pub fn total_count(&self, &str) -> u32;
    pub fn gain(&mut self, BotId, &str, u32);  pub fn lose(&mut self, BotId, &str, u32) -> Result<(),PlannerError>;
    pub fn set_position(&mut self, BotId, Position);
    pub fn entity_at(&self, &Position) -> Option<FactorioEntity>;
    pub fn is_position_free(&self, &Position) -> bool;
    pub fn create_entity(&mut self, FactorioEntity);
    pub fn resource_available(&self, &Position, &str) -> u32;
    pub fn resource_patches(&self, &str) -> Vec<ResourcePatch>;   // elements sorted by (x,y)
}
pub fn schedule(net: &ActionNetwork, state: &PlanState, bots: &[BotId]) -> Result<Schedule, PlannerError>;
```

Recipe data comes from `state.base().recipes: Arc<DashMap<String, FactorioRecipe>>`, where `FactorioRecipe { name, category, ingredients: Option<Vec<FactorioIngredient>>, products: Vec<FactorioProduct>, energy: Box<R64>, .. }`, `FactorioIngredient { name, amount }`. Entity prototypes come from `state.base().entity_prototypes`, where `FactorioEntityPrototype { name, entity_type, mining_time: Option<f64>, mine_result: Option<BTreeMap<String,u32>>, .. }`.

The fixture world (`factorio_bot_core::test_utils::fixture_world()`) provides the whole chain: `automation-science-pack` (category `crafting`, energy 5.0, needs 1 copper-plate + 1 iron-gear-wheel), `iron-gear-wheel` (`crafting`, 0.5, 2 iron-plate), `iron-plate` (`smelting`, 3.2, 1 iron-ore), `copper-plate` (`smelting`, 3.2, 1 copper-ore), `stone-furnace` (`crafting`, 0.5, 5 stone). Ore fields for iron, copper, coal and stone each have `mining_time` 1.0 and yield 1 per mine.

## File Structure

| File | Responsibility |
|---|---|
| `crates/planner/src/goal.rs` | `Goal`, `Holder` |
| `crates/planner/src/method/mod.rs` | `Step`, `Method` trait, `MethodRegistry`, `ExpansionCtx`, the `expand` driver |
| `crates/planner/src/method/have.rs` | `AlreadySatisfied`, `Mine`, `Smelt`, `HandCraft`, `SplitAcrossBots` |
| `crates/planner/src/method/util.rs` | Shared helpers: recipe lookup, tick conversion, free-tile search, nearest resource tile |
| `crates/planner/tests/red_science.rs` | The end-to-end slice |
| `crates/planner/src/lib.rs` | Module declarations and re-exports (modified) |

Methods live together in `have.rs` because they change together — every one of them is a way to satisfy `Goal::Have`, and they share the same shape. `util.rs` exists so the methods stay readable; anything used by two methods belongs there.

---

### Task 1: `Goal`, `Holder`, and the method vocabulary

**Files:**
- Create: `crates/planner/src/goal.rs`, `crates/planner/src/method/mod.rs`
- Modify: `crates/planner/src/lib.rs`
- Test: inline `#[cfg(test)] mod tests` in both new files

**Interfaces:**
- Consumes: `Action`, `ActionId`, `ActionIdGen`, `BotId`, `ItemId`, `Ticks`, `PlanState`, `PlannerError`.
- Produces: `Goal::{Have{item,count,whose}, Researched(String), Producing{item,rate}, All(Vec<Goal>)}`; `Holder::{Anyone, Bot(BotId)}`; `Step::{Subgoal(Goal), Act(Box<Action>), Link{from,to,lag}}`; `trait Method { fn name(&self) -> &'static str; fn applicable(&self, &Goal, &PlanState) -> bool; fn expand(&self, &Goal, &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError>; }`; `ExpansionCtx { state: PlanState, ids: ActionIdGen, chain_actor: BotId, depth: u32 }` with `ExpansionCtx::new(state, chain_actor)`; `MethodRegistry` with `::new()`, `::with(Box<dyn Method>)`, `::find(&Goal, &PlanState) -> Option<&dyn Method>`.

`Goal::Built` is deliberately absent — it needs blueprint types nothing uses yet. `Producing` is present as the documented functorio bridge but has no method in this plan.

- [ ] **Step 1: Write the failing tests**

Create `crates/planner/src/goal.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::BotId;

    #[test]
    fn holders_distinguish_anyone_from_a_named_bot() {
        assert_ne!(Holder::Anyone, Holder::Bot(BotId(1)));
        assert_eq!(Holder::Bot(BotId(1)), Holder::Bot(BotId(1)));
    }

    #[test]
    fn have_goals_compare_by_all_three_fields() {
        let a = Goal::Have { item: "iron-plate".into(), count: 2, whose: Holder::Anyone };
        let b = Goal::Have { item: "iron-plate".into(), count: 2, whose: Holder::Anyone };
        let c = Goal::Have { item: "iron-plate".into(), count: 3, whose: Holder::Anyone };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn goals_render_for_diagnostics() {
        let g = Goal::Have { item: "coal".into(), count: 4, whose: Holder::Bot(BotId(2)) };
        assert_eq!(g.to_string(), "have 4 coal (bot 2)");
        let g = Goal::Have { item: "coal".into(), count: 4, whose: Holder::Anyone };
        assert_eq!(g.to_string(), "have 4 coal (anyone)");
    }
}
```

Create `crates/planner/src/method/mod.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::goal::{Goal, Holder};
    use crate::ids::BotId;
    use crate::state::PlanState;
    use factorio_bot_core::test_utils::fixture_world;
    use std::sync::Arc;

    fn ctx() -> ExpansionCtx {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        ExpansionCtx::new(state, BotId(1))
    }

    /// A method that claims every `Have` goal and expands to nothing.
    struct Nothing;
    impl Method for Nothing {
        fn name(&self) -> &'static str {
            "nothing"
        }
        fn applicable(&self, goal: &Goal, _state: &PlanState) -> bool {
            matches!(goal, Goal::Have { .. })
        }
        fn expand(&self, _goal: &Goal, _ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
            Ok(vec![])
        }
    }

    #[test]
    fn the_registry_finds_an_applicable_method() {
        let reg = MethodRegistry::new().with(Box::new(Nothing));
        let c = ctx();
        let goal = Goal::Have { item: "coal".into(), count: 1, whose: Holder::Anyone };
        assert_eq!(reg.find(&goal, &c.state).map(|m| m.name()), Some("nothing"));
    }

    #[test]
    fn the_registry_returns_none_when_nothing_applies() {
        let reg = MethodRegistry::new().with(Box::new(Nothing));
        let c = ctx();
        assert!(reg.find(&Goal::Researched("automation".into()), &c.state).is_none());
    }

    #[test]
    fn the_registry_prefers_the_first_registered_applicable_method() {
        struct Other;
        impl Method for Other {
            fn name(&self) -> &'static str {
                "other"
            }
            fn applicable(&self, _goal: &Goal, _state: &PlanState) -> bool {
                true
            }
            fn expand(&self, _g: &Goal, _c: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
                Ok(vec![])
            }
        }
        let reg = MethodRegistry::new().with(Box::new(Nothing)).with(Box::new(Other));
        let c = ctx();
        let goal = Goal::Have { item: "coal".into(), count: 1, whose: Holder::Anyone };
        assert_eq!(reg.find(&goal, &c.state).map(|m| m.name()), Some("nothing"));
    }

    #[test]
    fn the_context_allocates_ascending_action_ids() {
        let mut c = ctx();
        let first = c.ids.next();
        let second = c.ids.next();
        assert!(first < second);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: FAIL to compile — `goal` and `method` are not modules of the crate, and none of these types exist.

- [ ] **Step 3: Write `goal.rs`**

Put this above the test module in `crates/planner/src/goal.rs`:

```rust
//! What we want, stated declaratively and without reference to any bot.

use crate::ids::{BotId, ItemId};

/// Who must end up holding the items.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Holder {
    /// Satisfied by the sum across every bot. This is what makes multi-bot
    /// gathering parallel: the count can be split into independent per-bot
    /// chains that never coordinate.
    Anyone,
    /// One named bot's inventory.
    Bot(BotId),
}

impl std::fmt::Display for Holder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Holder::Anyone => write!(f, "anyone"),
            Holder::Bot(id) => write!(f, "{}", id),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Goal {
    Have {
        item: ItemId,
        count: u32,
        whose: Holder,
    },
    Researched(String),
    /// The functorio bridge: `BusLane item rate` transcribed into Rust. No
    /// method satisfies this yet; blueprint generation is a later increment.
    Producing {
        item: ItemId,
        rate: f64,
    },
    All(Vec<Goal>),
}

impl std::fmt::Display for Goal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Goal::Have { item, count, whose } => write!(f, "have {} {} ({})", count, item, whose),
            Goal::Researched(tech) => write!(f, "research {}", tech),
            Goal::Producing { item, rate } => write!(f, "produce {} {}/min", rate, item),
            Goal::All(goals) => write!(f, "all of {} goals", goals.len()),
        }
    }
}
```

- [ ] **Step 4: Write `method/mod.rs`**

Put this above the test module in `crates/planner/src/method/mod.rs`:

```rust
//! How to get what we want: hand-written decompositions, and the driver that
//! runs them until only actions remain.

use crate::action::Action;
use crate::error::PlannerError;
use crate::goal::{Goal, Holder};
use crate::ids::{ActionId, ActionIdGen, BotId, Ticks};
use crate::state::PlanState;

/// One element of a method's expansion.
#[derive(Clone, Debug)]
pub enum Step {
    /// Recurse: this goal is expanded by whatever method claims it.
    Subgoal(Goal),
    /// A primitive action. Boxed because `Action` is large.
    Act(Box<Action>),
    /// An explicit ordering edge with a minimum lag, for dependencies that
    /// inference cannot see — a furnace's smelting time, above all. The ids
    /// come from actions the same method allocated via `ExpansionCtx::ids`.
    Link {
        from: ActionId,
        to: ActionId,
        lag: Ticks,
    },
}

/// State threaded through one expansion.
///
/// `chain_actor` is used **only** to simulate effects while expanding, so that
/// a later sibling goal sees what an earlier one produced. It never reaches an
/// emitted action: methods emit `Actor::Role` with `pinned: None`, and the
/// scheduler decides who actually runs each action.
///
/// This rests on bots being interchangeable at the start of planning — chain
/// *j* is simulated against bot *j* on the assumption that any bot would
/// experience the same thing. That holds while `PlanState::from_world` gives
/// unknown bots identical defaults. **If bots ever start with materially
/// different inventories, this driver must be revisited.**
pub struct ExpansionCtx {
    pub state: PlanState,
    pub ids: ActionIdGen,
    pub chain_actor: BotId,
    pub depth: u32,
}

impl ExpansionCtx {
    pub fn new(state: PlanState, chain_actor: BotId) -> Self {
        ExpansionCtx {
            state,
            ids: ActionIdGen::new(),
            chain_actor,
            depth: 0,
        }
    }
}

/// One way to satisfy one kind of goal. Hand-written and readable by design:
/// there is no search here, and adding a method should never require
/// understanding the others.
pub trait Method {
    fn name(&self) -> &'static str;

    /// Can this method satisfy `goal` given `state`? Consulted in registration
    /// order, so a cheaper method registered earlier wins.
    fn applicable(&self, goal: &Goal, state: &PlanState) -> bool;

    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError>;
}

/// Methods in preference order. The first applicable one wins.
#[derive(Default)]
pub struct MethodRegistry {
    methods: Vec<Box<dyn Method>>,
}

impl MethodRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, method: Box<dyn Method>) -> Self {
        self.methods.push(method);
        self
    }

    pub fn find(&self, goal: &Goal, state: &PlanState) -> Option<&dyn Method> {
        self.methods
            .iter()
            .find(|m| m.applicable(goal, state))
            .map(|m| m.as_ref())
    }
}
```

- [ ] **Step 5: Declare and re-export the modules**

In `crates/planner/src/lib.rs`, add `pub mod goal;` and `pub mod method;` to the module list (keeping it alphabetical), and add:

```rust
pub use goal::{Goal, Holder};
pub use method::{ExpansionCtx, Method, MethodRegistry, Step};
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: PASS — 7 new tests on top of the existing suite.

- [ ] **Step 7: Verify lints and commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-planner && cargo clippy --workspace --all-features --all-targets -- --deny warnings --deny deprecated'
git commit -m "feat(planner): add Goal, Holder and the method vocabulary" -- crates/planner
```

---

### Task 2: The expansion driver

**Files:**
- Modify: `crates/planner/src/method/mod.rs`, `crates/planner/src/error.rs`
- Test: the existing inline `mod tests` in `crates/planner/src/method/mod.rs`

**Interfaces:**
- Consumes: everything from Task 1.
- Produces: `pub const MAX_EXPANSION_DEPTH: u32 = 32;`; `pub fn expand(goals: &[Goal], state: &PlanState, registry: &MethodRegistry, chain_actor: BotId) -> Result<ActionNetwork, PlannerError>`; `PlannerError::{NoApplicableMethod{goal}, ExpansionTooDeep{goal}}`.

The driver advances `ctx.state` as it emits actions, so a later sibling's `AlreadySatisfied` check sees what an earlier sibling produced. That progression is what makes hand-written methods composable without them knowing about each other.

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `crates/planner/src/method/mod.rs`:

```rust
    use crate::action::{Action, ActionKind, Actor, Effect};
    use crate::network::ActionNetwork;

    fn gain_action(ctx: &mut ExpansionCtx, item: &str, count: u32) -> Action {
        Action {
            id: ctx.ids.next(),
            kind: ActionKind::Craft { item: item.into(), count },
            pre: vec![],
            eff: vec![Effect::GainItem { who: Actor::Role, item: item.into(), count }],
            duration: 60,
            pinned: None,
            label: format!("make {} {}", count, item),
        }
    }

    /// Expands `Have` into one action that produces the requested count.
    struct Produce;
    impl Method for Produce {
        fn name(&self) -> &'static str { "produce" }
        fn applicable(&self, goal: &Goal, _s: &PlanState) -> bool {
            matches!(goal, Goal::Have { .. })
        }
        fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
            let Goal::Have { item, count, .. } = goal else { unreachable!() };
            let a = gain_action(ctx, item, *count);
            Ok(vec![Step::Act(Box::new(a))])
        }
    }

    #[test]
    fn the_driver_turns_a_goal_into_a_network() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let reg = MethodRegistry::new().with(Box::new(Produce));
        let goal = Goal::Have { item: "coal".into(), count: 3, whose: Holder::Anyone };
        let net = expand(&[goal], &state, &reg, BotId(1)).unwrap();
        assert_eq!(net.len(), 1);
    }

    #[test]
    fn the_driver_advances_its_state_so_siblings_see_earlier_effects() {
        // `Produce` runs for the first goal; the second is already satisfied by
        // the first one's simulated effect, so `Satisfied` claims it and emits
        // nothing. Two identical goals must therefore yield ONE action.
        struct Satisfied;
        impl Method for Satisfied {
            fn name(&self) -> &'static str { "satisfied" }
            fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
                match goal {
                    Goal::Have { item, count, .. } => state.total_count(item) >= *count,
                    _ => false,
                }
            }
            fn expand(&self, _g: &Goal, _c: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
                Ok(vec![])
            }
        }
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let reg = MethodRegistry::new().with(Box::new(Satisfied)).with(Box::new(Produce));
        let goal = Goal::Have { item: "coal".into(), count: 3, whose: Holder::Anyone };
        let net = expand(&[goal.clone(), goal], &state, &reg, BotId(1)).unwrap();
        assert_eq!(net.len(), 1, "the second goal was already satisfied");
    }

    #[test]
    fn the_driver_recurses_through_subgoals() {
        struct ViaSubgoal;
        impl Method for ViaSubgoal {
            fn name(&self) -> &'static str { "via-subgoal" }
            fn applicable(&self, goal: &Goal, _s: &PlanState) -> bool {
                matches!(goal, Goal::Have { item, .. } if item == "iron-gear-wheel")
            }
            fn expand(&self, _g: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
                let a = gain_action(ctx, "iron-gear-wheel", 1);
                Ok(vec![
                    Step::Subgoal(Goal::Have {
                        item: "iron-plate".into(),
                        count: 2,
                        whose: Holder::Anyone,
                    }),
                    Step::Act(Box::new(a)),
                ])
            }
        }
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let reg = MethodRegistry::new().with(Box::new(ViaSubgoal)).with(Box::new(Produce));
        let goal = Goal::Have { item: "iron-gear-wheel".into(), count: 1, whose: Holder::Anyone };
        let net = expand(&[goal], &state, &reg, BotId(1)).unwrap();
        assert_eq!(net.len(), 2, "the subgoal's action and the gear itself");
    }

    #[test]
    fn an_unclaimed_goal_is_an_error() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let reg = MethodRegistry::new().with(Box::new(Produce));
        let result = expand(&[Goal::Researched("automation".into())], &state, &reg, BotId(1));
        assert!(matches!(result, Err(PlannerError::NoApplicableMethod { .. })));
    }

    #[test]
    fn producing_has_no_method_in_this_increment() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let reg = MethodRegistry::new().with(Box::new(Produce));
        let goal = Goal::Producing { item: "automation-science-pack".into(), rate: 150.0 };
        assert!(matches!(
            expand(&[goal], &state, &reg, BotId(1)),
            Err(PlannerError::NoApplicableMethod { .. })
        ));
    }

    #[test]
    fn runaway_recursion_is_an_error_not_a_hang() {
        struct Forever;
        impl Method for Forever {
            fn name(&self) -> &'static str { "forever" }
            fn applicable(&self, goal: &Goal, _s: &PlanState) -> bool {
                matches!(goal, Goal::Have { .. })
            }
            fn expand(&self, goal: &Goal, _c: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
                Ok(vec![Step::Subgoal(goal.clone())])
            }
        }
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let reg = MethodRegistry::new().with(Box::new(Forever));
        let goal = Goal::Have { item: "coal".into(), count: 1, whose: Holder::Anyone };
        assert!(matches!(
            expand(&[goal], &state, &reg, BotId(1)),
            Err(PlannerError::ExpansionTooDeep { .. })
        ));
    }

    #[test]
    fn a_bot_addressed_goal_rebinds_the_chain_actor() {
        // `Record` reports which actor the driver was simulating against.
        use std::cell::RefCell;
        use std::rc::Rc;
        struct Record(Rc<RefCell<Vec<BotId>>>);
        impl Method for Record {
            fn name(&self) -> &'static str { "record" }
            fn applicable(&self, goal: &Goal, _s: &PlanState) -> bool {
                matches!(goal, Goal::Have { .. })
            }
            fn expand(&self, _g: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
                self.0.borrow_mut().push(ctx.chain_actor);
                Ok(vec![])
            }
        }
        let seen = Rc::new(RefCell::new(Vec::new()));
        let reg = MethodRegistry::new().with(Box::new(Record(seen.clone())));
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1), BotId(2)]);
        let goals = vec![
            Goal::Have { item: "coal".into(), count: 1, whose: Holder::Bot(BotId(2)) },
            Goal::Have { item: "stone".into(), count: 1, whose: Holder::Anyone },
        ];
        expand(&goals, &state, &reg, BotId(1)).unwrap();
        assert_eq!(
            *seen.borrow(),
            vec![BotId(2), BotId(1)],
            "a Bot-addressed goal rebinds, and the binding is restored afterwards"
        );
    }

    #[test]
    fn all_expands_each_of_its_goals() {
        let state = PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)]);
        let reg = MethodRegistry::new().with(Box::new(Produce));
        let goal = Goal::All(vec![
            Goal::Have { item: "coal".into(), count: 1, whose: Holder::Anyone },
            Goal::Have { item: "stone".into(), count: 1, whose: Holder::Anyone },
        ]);
        let net = expand(&[goal], &state, &reg, BotId(1)).unwrap();
        assert_eq!(net.len(), 2);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: FAIL — `expand`, `PlannerError::NoApplicableMethod` and `PlannerError::ExpansionTooDeep` do not exist.

- [ ] **Step 3: Add the two error variants**

In `crates/planner/src/error.rs`, add to `PlannerError`:

```rust
    #[error("no method can satisfy goal: {goal}")]
    #[diagnostic(code(planner::no_applicable_method))]
    NoApplicableMethod { goal: String },

    #[error("expansion of {goal} exceeded {depth} levels; a method is probably expanding into itself")]
    #[diagnostic(code(planner::expansion_too_deep))]
    ExpansionTooDeep { goal: String, depth: u32 },
```

- [ ] **Step 4: Write the driver**

Append to `crates/planner/src/method/mod.rs`, above the test module:

```rust
use crate::network::ActionNetwork;

/// Recipe chains in this domain are shallow — science pack to gear to plate to
/// ore is four levels, plus one for a per-bot split. This bound exists to turn
/// a method that expands into itself into an error rather than a hang.
pub const MAX_EXPANSION_DEPTH: u32 = 32;

/// Expand `goals` into a schedulable network.
///
/// Each goal is expanded by the first applicable method, recursively, until
/// only actions remain. The context's state is advanced as actions are emitted,
/// so a later sibling sees what an earlier one produced — that progression is
/// what lets hand-written methods compose without knowing about each other.
///
/// Ordering edges are inferred at the end, on top of whatever explicit `Link`
/// steps the methods emitted for dependencies inference cannot see.
pub fn expand(
    goals: &[Goal],
    state: &PlanState,
    registry: &MethodRegistry,
    chain_actor: BotId,
) -> Result<ActionNetwork, PlannerError> {
    let mut ctx = ExpansionCtx::new(state.fork(), chain_actor);
    let mut net = ActionNetwork::new();
    for goal in goals {
        expand_goal(goal, &mut ctx, &mut net, registry)?;
    }
    net.infer_edges();
    net.validate()?;
    Ok(net)
}

fn expand_goal(
    goal: &Goal,
    ctx: &mut ExpansionCtx,
    net: &mut ActionNetwork,
    registry: &MethodRegistry,
) -> Result<(), PlannerError> {
    if ctx.depth >= MAX_EXPANSION_DEPTH {
        return Err(PlannerError::ExpansionTooDeep {
            goal: goal.to_string(),
            depth: MAX_EXPANSION_DEPTH,
        });
    }

    if let Goal::All(inner) = goal {
        ctx.depth += 1;
        for g in inner {
            expand_goal(g, ctx, net, registry)?;
        }
        ctx.depth -= 1;
        return Ok(());
    }

    // NOTE: as written below, the restore of `chain_actor` and the decrement of
    // `depth` sit after several `?` early-returns, so an error skips them. That
    // is inert today because `expand()` aborts on the first error and drops the
    // context — but the invariant is claimed unconditionally in the comments and
    // is not. Task 2's review corrected this; the shipped code brackets a single
    // fallible `expand_goal_body` call with the save and restore instead. Follow
    // the shipped code, not this listing, if you are re-deriving the driver.
    //
    // A goal addressed to one bot rebinds the chain actor for its whole
    // subtree, so that simulated effects land in the same inventory the
    // shortfall checks read. Without this the driver would credit a chain's
    // mining to one bot while asking whether a different one was satisfied.
    let previous_actor = ctx.chain_actor;
    if let Goal::Have { whose: Holder::Bot(bot), .. } = goal {
        ctx.chain_actor = *bot;
    }

    let method = registry
        .find(goal, &ctx.state)
        .ok_or_else(|| PlannerError::NoApplicableMethod {
            goal: goal.to_string(),
        })?;
    let steps = method.expand(goal, ctx)?;

    ctx.depth += 1;
    for step in steps {
        match step {
            Step::Subgoal(g) => expand_goal(&g, ctx, net, registry)?,
            Step::Act(action) => {
                // Simulate against the chain actor so later siblings see this
                // action's results. The emitted action stays unpinned.
                let binding = ctx.chain_actor;
                for effect in &action.eff {
                    effect.apply(&mut ctx.state, binding)?;
                }
                net.add(*action);
            }
            Step::Link { from, to, lag } => net.link(from, to, lag),
        }
    }
    ctx.depth -= 1;
    ctx.chain_actor = previous_actor;
    Ok(())
}
```

- [ ] **Step 5: Re-export from `lib.rs`**

Add to `crates/planner/src/lib.rs`:

```rust
pub use method::{expand, MAX_EXPANSION_DEPTH};
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: PASS — 7 more tests.

- [ ] **Step 7: Verify lints and commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-planner && cargo clippy --workspace --all-features --all-targets -- --deny warnings --deny deprecated'
git commit -m "feat(planner): add the goal expansion driver" -- crates/planner
```

---

### Task 3: Shared method helpers

**Files:**
- Create: `crates/planner/src/method/util.rs`
- Modify: `crates/planner/src/method/mod.rs` (add `pub mod util;`)
- Test: inline `#[cfg(test)] mod tests` in `crates/planner/src/method/util.rs`

**Interfaces:**
- Consumes: `PlanState`, `PlannerError`, `Ticks`.
- Produces: `pub fn seconds_to_ticks(seconds: f64) -> Ticks`; `pub fn recipe_for(state: &PlanState, item: &str) -> Option<FactorioRecipe>`; `pub fn nearest_resource_tile(state: &PlanState, item: &str, from: &Position, need: u32) -> Option<Position>`; `pub fn free_tile_near(state: &PlanState, from: &Position) -> Option<Position>`; `pub fn mining_ticks(state: &PlanState, item: &str) -> Ticks`.

Everything two methods would otherwise duplicate lives here, so the methods stay readable.

- [ ] **Step 1: Write the failing tests**

Create `crates/planner/src/method/util.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::BotId;
    use crate::state::PlanState;
    use factorio_bot_core::test_utils::fixture_world;
    use factorio_bot_core::types::Position;
    use std::sync::Arc;

    fn state() -> PlanState {
        PlanState::from_world(Arc::new(fixture_world()), &[BotId(1)])
    }

    #[test]
    fn seconds_convert_to_ticks_and_round_up() {
        assert_eq!(seconds_to_ticks(1.0), 60);
        assert_eq!(seconds_to_ticks(3.2), 192);
        assert_eq!(seconds_to_ticks(0.5), 30);
        // Never round a positive duration down to nothing.
        assert_eq!(seconds_to_ticks(0.001), 1);
        assert_eq!(seconds_to_ticks(0.0), 0);
    }

    #[test]
    fn recipes_are_found_by_name() {
        let s = state();
        let r = recipe_for(&s, "iron-gear-wheel").expect("fixture has iron-gear-wheel");
        assert_eq!(r.category, "crafting");
        assert!(recipe_for(&s, "nonexistent-thing").is_none());
    }

    #[test]
    fn smelting_and_crafting_recipes_are_distinguishable() {
        let s = state();
        assert_eq!(recipe_for(&s, "iron-plate").unwrap().category, "smelting");
        assert_eq!(
            recipe_for(&s, "automation-science-pack").unwrap().category,
            "crafting"
        );
    }

    #[test]
    fn the_nearest_resource_tile_is_in_the_patch_and_holds_enough() {
        let s = state();
        let origin = Position::new(0., 0.);
        let tile = nearest_resource_tile(&s, "iron-ore", &origin, 5).expect("fixture has iron ore");
        assert!(s.resource_available(&tile, "iron-ore") >= 5);
        // The iron field sits around x -45..-35, y 35..45.
        assert!(tile.x <= -35.0 && tile.x >= -45.0, "unexpected x: {}", tile.x);
        assert!(tile.y >= 35.0 && tile.y <= 45.0, "unexpected y: {}", tile.y);
    }

    #[test]
    fn the_nearest_resource_tile_is_deterministic() {
        let s = state();
        let origin = Position::new(0., 0.);
        let a = nearest_resource_tile(&s, "iron-ore", &origin, 1).unwrap();
        let b = nearest_resource_tile(&s, "iron-ore", &origin, 1).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn a_missing_resource_has_no_tile() {
        let s = state();
        assert!(nearest_resource_tile(&s, "uranium-ore", &Position::new(0., 0.), 1).is_none());
    }

    #[test]
    fn a_free_tile_is_found_and_is_actually_free() {
        let s = state();
        let pos = free_tile_near(&s, &Position::new(0., 0.)).expect("origin area is open");
        assert!(s.is_position_free(&pos));
    }

    #[test]
    fn a_free_tile_avoids_an_occupied_one() {
        let mut s = state();
        let origin = Position::new(0., 0.);
        let first = free_tile_near(&s, &origin).unwrap();
        let furnace = factorio_bot_core::types::FactorioEntity {
            name: "stone-furnace".into(),
            entity_type: "furnace".into(),
            position: first.clone(),
            ..Default::default()
        };
        s.create_entity(furnace);
        let second = free_tile_near(&s, &origin).unwrap();
        assert_ne!(first, second);
        assert!(s.is_position_free(&second));
    }

    #[test]
    fn mining_a_fixture_ore_takes_one_second() {
        let s = state();
        assert_eq!(mining_ticks(&s, "iron-ore"), 60);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: FAIL to compile — `crate::method::util` does not exist.

- [ ] **Step 3: Write the helpers**

Put this above the test module in `crates/planner/src/method/util.rs`:

```rust
//! Helpers shared by more than one method.

use crate::ids::Ticks;
use crate::state::PlanState;
use factorio_bot_core::factorio::util::calculate_distance;
use factorio_bot_core::num_traits::ToPrimitive;
use factorio_bot_core::types::{FactorioRecipe, Position};

const TICKS_PER_SECOND: f64 = 60.0;

/// How far out `free_tile_near` will search before giving up, in tiles.
const FREE_TILE_SEARCH_RADIUS: i32 = 12;

/// Convert a recipe's or prototype's seconds into ticks, rounding up so that a
/// positive duration never becomes zero.
pub fn seconds_to_ticks(seconds: f64) -> Ticks {
    if seconds <= 0.0 {
        return 0;
    }
    (seconds * TICKS_PER_SECOND).ceil() as Ticks
}

pub fn recipe_for(state: &PlanState, item: &str) -> Option<FactorioRecipe> {
    state.base().recipes.get(item).map(|r| r.clone())
}

/// Ticks to mine one unit of `item`, from its entity prototype. Defaults to one
/// second when the prototype carries no mining time.
pub fn mining_ticks(state: &PlanState, item: &str) -> Ticks {
    let seconds = state
        .base()
        .entity_prototypes
        .get(item)
        .and_then(|p| p.mining_time)
        .unwrap_or(1.0);
    seconds_to_ticks(seconds)
}

/// The tile of `item` nearest `from` that still holds at least `need`.
///
/// Ties on distance are broken by `(x, y)`, so the result depends only on the
/// tile set and the origin — never on the order `resource_patches` happens to
/// return patches in, which is not stable across processes for patches of
/// equal size (their ids come from a `HashMap`-seeded loop in `core`, and the
/// size sort is stable).
pub fn nearest_resource_tile(
    state: &PlanState,
    item: &str,
    from: &Position,
    need: u32,
) -> Option<Position> {
    let mut best: Option<(f64, Position)> = None;
    for patch in state.resource_patches(item) {
        for tile in patch.elements {
            if state.resource_available(&tile, item) < need {
                continue;
            }
            let distance = calculate_distance(from, &tile);
            let better = match &best {
                None => true,
                Some((best_distance, best_tile)) => matches!(
                    distance
                        .total_cmp(best_distance)
                        .then(tile.x.total_cmp(&best_tile.x))
                        .then(tile.y.total_cmp(&best_tile.y)),
                    std::cmp::Ordering::Less
                ),
            };
            if better {
                best = Some((distance, tile));
            }
        }
    }
    best.map(|(_, tile)| tile)
}

/// The nearest unoccupied tile to `from`, searched in rings so the result is
/// close and reproducible.
pub fn free_tile_near(state: &PlanState, from: &Position) -> Option<Position> {
    let base_x = from.x.floor() as i32;
    let base_y = from.y.floor() as i32;
    for radius in 0..=FREE_TILE_SEARCH_RADIUS {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                // Only the ring at exactly this radius; inner ones were done.
                if dx.abs() != radius && dy.abs() != radius {
                    continue;
                }
                let candidate = Position::new((base_x + dx) as f64, (base_y + dy) as f64);
                if state.is_position_free(&candidate) {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// Ingredients of `item`, or an empty vector when the recipe has none.
pub fn ingredients_of(recipe: &FactorioRecipe) -> Vec<(String, u32)> {
    recipe
        .ingredients
        .as_ref()
        .map(|list| {
            list.iter()
                .map(|i| (i.name.clone(), i.amount))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// How many of `item` one execution of `recipe` yields. Defaults to 1.
pub fn output_per_craft(recipe: &FactorioRecipe, item: &str) -> u32 {
    recipe
        .products
        .iter()
        .find(|p| p.name == item)
        .map(|p| p.amount.max(1))
        .unwrap_or(1)
}

/// A recipe's energy in ticks.
pub fn recipe_ticks(recipe: &FactorioRecipe) -> Ticks {
    seconds_to_ticks(recipe.energy.to_f64().unwrap_or(0.5))
}
```

- [ ] **Step 4: Declare the module**

In `crates/planner/src/method/mod.rs`, add near the top:

```rust
pub mod util;
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: PASS — 9 more tests.

- [ ] **Step 6: Verify lints and commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-planner && cargo clippy --workspace --all-features --all-targets -- --deny warnings --deny deprecated'
git commit -m "feat(planner): add shared helpers for methods" -- crates/planner
```

---

### Task 4: `AlreadySatisfied` and `Mine`

**Files:**
- Create: `crates/planner/src/method/have.rs`
- Modify: `crates/planner/src/method/mod.rs` (add `pub mod have;`), `crates/planner/src/lib.rs`
- Test: inline `#[cfg(test)] mod tests` in `crates/planner/src/method/have.rs`

**Interfaces:**
- Consumes: Tasks 1-3.
- Produces: `pub struct AlreadySatisfied;` and `pub struct Mine;`, both implementing `Method`; `pub fn default_registry() -> MethodRegistry` returning them in that order.

After this task, `Have(iron-ore, 5, Anyone)` expands and schedules end to end.

- [ ] **Step 1: Write the failing tests**

Create `crates/planner/src/method/have.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::BotId;
    use crate::method::expand;
    use crate::schedule::schedule;
    use crate::state::PlanState;
    use factorio_bot_core::test_utils::fixture_world;
    use std::sync::Arc;

    fn state(bots: &[BotId]) -> PlanState {
        PlanState::from_world(Arc::new(fixture_world()), bots)
    }

    #[test]
    fn an_already_held_item_expands_to_nothing() {
        let mut s = state(&[BotId(1)]);
        s.gain(BotId(1), "iron-ore", 10);
        let net = expand(
            &[Goal::Have { item: "iron-ore".into(), count: 5, whose: Holder::Anyone }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();
        assert_eq!(net.len(), 0, "nothing to do");
    }

    #[test]
    fn mining_produces_one_action_that_yields_the_requested_count() {
        let s = state(&[BotId(1)]);
        let net = expand(
            &[Goal::Have { item: "iron-ore".into(), count: 5, whose: Holder::Anyone }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();
        assert_eq!(net.len(), 1);
        let action = net.actions().next().unwrap();
        match &action.kind {
            ActionKind::Mine { item, count, .. } => {
                assert_eq!(item, "iron-ore");
                assert_eq!(*count, 5);
            }
            other => panic!("expected a mine action, got {:?}", other),
        }
        // One second per ore in the fixture.
        assert_eq!(action.duration, 300);
    }

    #[test]
    fn mining_only_asks_for_what_is_missing() {
        let mut s = state(&[BotId(1)]);
        s.gain(BotId(1), "iron-ore", 3);
        let net = expand(
            &[Goal::Have { item: "iron-ore".into(), count: 5, whose: Holder::Anyone }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();
        let action = net.actions().next().unwrap();
        match &action.kind {
            ActionKind::Mine { count, .. } => assert_eq!(*count, 2, "only the shortfall"),
            other => panic!("expected a mine action, got {:?}", other),
        }
    }

    #[test]
    fn a_mine_action_carries_its_reach_and_resource_preconditions() {
        let s = state(&[BotId(1)]);
        let net = expand(
            &[Goal::Have { item: "coal".into(), count: 2, whose: Holder::Anyone }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();
        let action = net.actions().next().unwrap();
        assert!(action.pre.iter().any(|c| matches!(c, Condition::AtPosition { .. })));
        assert!(action
            .pre
            .iter()
            .any(|c| matches!(c, Condition::ResourceAvailable { item, count, .. } if item == "coal" && *count == 2)));
        assert!(action
            .eff
            .iter()
            .any(|e| matches!(e, Effect::GainItem { item, count, .. } if item == "coal" && *count == 2)));
        assert!(action
            .eff
            .iter()
            .any(|e| matches!(e, Effect::ConsumeResource { .. })));
    }

    #[test]
    fn emitted_actions_are_unpinned_and_use_the_role_actor() {
        let s = state(&[BotId(1)]);
        let net = expand(
            &[Goal::Have { item: "coal".into(), count: 1, whose: Holder::Anyone }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();
        let action = net.actions().next().unwrap();
        assert_eq!(action.pinned, None, "methods must never pin");
        assert!(action.eff.iter().all(|e| match e {
            Effect::GainItem { who, .. } | Effect::LoseItem { who, .. } => *who == Actor::Role,
            _ => true,
        }));
    }

    #[test]
    fn a_mined_goal_schedules() {
        let bots = [BotId(1), BotId(2)];
        let s = state(&bots);
        let net = expand(
            &[Goal::Have { item: "iron-ore".into(), count: 4, whose: Holder::Anyone }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();
        let plan = schedule(&net, &s, &bots).expect("schedulable");
        assert_eq!(plan.steps.len(), 2, "a walk and the mine");
        assert!(plan.makespan > 240, "walking to the patch plus four seconds mining");
    }

    #[test]
    fn an_unobtainable_item_has_no_method() {
        let s = state(&[BotId(1)]);
        let result = expand(
            &[Goal::Have { item: "uranium-ore".into(), count: 1, whose: Holder::Anyone }],
            &s,
            &default_registry(),
            BotId(1),
        );
        assert!(matches!(result, Err(PlannerError::NoApplicableMethod { .. })));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: FAIL to compile — `crate::method::have` does not exist.

- [ ] **Step 3: Write the two methods**

Put this above the test module in `crates/planner/src/method/have.rs`:

```rust
//! Methods that satisfy `Goal::Have`.
//!
//! Every method here emits actions with `Actor::Role` and `pinned: None`. The
//! scheduler decides who runs each one, and a chain stays with one bot because
//! its `HasItem` preconditions are only satisfiable by the bot holding the
//! items — see the plan's note on why nothing is pinned.

use crate::action::{Action, ActionKind, Actor, Condition, Effect};
use crate::error::PlannerError;
use crate::goal::{Goal, Holder};
use crate::method::util::{mining_ticks, nearest_resource_tile};
use crate::method::{ExpansionCtx, Method, MethodRegistry, Step};
use crate::state::PlanState;

/// How much of `item` still needs producing, given what is already held.
fn shortfall(state: &PlanState, item: &str, count: u32, whose: &Holder) -> u32 {
    let held = match whose {
        Holder::Anyone => state.total_count(item),
        Holder::Bot(id) => state.inventory_count(*id, item),
    };
    count.saturating_sub(held)
}

/// The goal is already met. Emits nothing.
pub struct AlreadySatisfied;

impl Method for AlreadySatisfied {
    fn name(&self) -> &'static str {
        "already-satisfied"
    }

    fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
        match goal {
            Goal::Have { item, count, whose } => shortfall(state, item, *count, whose) == 0,
            _ => false,
        }
    }

    fn expand(&self, _goal: &Goal, _ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        Ok(vec![])
    }
}

/// Mine the shortfall straight out of the ground.
pub struct Mine;

impl Method for Mine {
    fn name(&self) -> &'static str {
        "mine"
    }

    fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
        let Goal::Have { item, count, whose } = goal else {
            return false;
        };
        let need = shortfall(state, item, *count, whose);
        if need == 0 {
            return false;
        }
        // Position-independent on purpose: applicability asks only whether a
        // tile with enough left exists anywhere. Which one is nearest is
        // `expand`'s business, and depends on the chain actor it has and this
        // method does not.
        state.resource_patches(item).iter().any(|patch| {
            patch
                .elements
                .iter()
                .any(|tile| state.resource_available(tile, item) >= need)
        })
    }

    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        let Goal::Have { item, count, whose } = goal else {
            return Err(PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            });
        };
        let need = shortfall(&ctx.state, item, *count, whose);
        let from = ctx
            .state
            .bot(ctx.chain_actor)
            .map(|b| b.position.clone())
            .unwrap_or_default();
        let pos = nearest_resource_tile(&ctx.state, item, &from, need).ok_or_else(|| {
            PlannerError::NoApplicableMethod {
                goal: goal.to_string(),
            }
        })?;
        let reach = ctx
            .state
            .bot(ctx.chain_actor)
            .map(|b| b.resource_reach_distance)
            .unwrap_or(3.0);

        let action = Action {
            id: ctx.ids.next(),
            kind: ActionKind::Mine {
                pos: pos.clone(),
                item: item.clone(),
                count: need,
            },
            pre: vec![
                Condition::AtPosition {
                    who: Actor::Role,
                    pos: pos.clone(),
                    radius: reach,
                },
                Condition::ResourceAvailable {
                    pos: pos.clone(),
                    item: item.clone(),
                    count: need,
                },
            ],
            eff: vec![
                Effect::ConsumeResource {
                    pos,
                    item: item.clone(),
                    count: need,
                },
                Effect::GainItem {
                    who: Actor::Role,
                    item: item.clone(),
                    count: need,
                },
            ],
            duration: mining_ticks(&ctx.state, item).saturating_mul(need),
            pinned: None,
            label: format!("mine {} {}", need, item),
        };
        Ok(vec![Step::Act(Box::new(action))])
    }
}

/// The methods this crate ships, in preference order.
pub fn default_registry() -> MethodRegistry {
    MethodRegistry::new()
        .with(Box::new(AlreadySatisfied))
        .with(Box::new(Mine))
}
```

- [ ] **Step 4: Declare and re-export**

In `crates/planner/src/method/mod.rs` add `pub mod have;`. In `crates/planner/src/lib.rs` add:

```rust
pub use method::have::default_registry;
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: PASS — 7 more tests.

- [ ] **Step 6: Verify lints and commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-planner && cargo clippy --workspace --all-features --all-targets -- --deny warnings --deny deprecated'
git commit -m "feat(planner): add the already-satisfied and mine methods" -- crates/planner
```

---

### Task 5: `Smelt`

**Files:**
- Modify: `crates/planner/src/method/have.rs`
- Test: the existing inline `mod tests` in `crates/planner/src/method/have.rs`

**Interfaces:**
- Consumes: Tasks 1-4.
- Produces: `pub struct Smelt;` implementing `Method`, registered in `default_registry()` after `AlreadySatisfied` and before `Mine`; `pub const PLATES_PER_COAL: u32 = 13;`.

Smelting is where the lag edge earns its place: the removal cannot start until the furnace has run, but the bot is free to walk away in the meantime.

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `crates/planner/src/method/have.rs`:

```rust
    #[test]
    fn smelting_emits_place_insert_insert_remove() {
        let mut s = state(&[BotId(1)]);
        s.gain(BotId(1), "stone-furnace", 1);
        let net = expand(
            &[Goal::Have { item: "iron-plate".into(), count: 2, whose: Holder::Anyone }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();
        let kinds: Vec<&str> = net
            .actions()
            .map(|a| match &a.kind {
                ActionKind::Mine { .. } => "mine",
                ActionKind::Craft { .. } => "craft",
                ActionKind::Place { .. } => "place",
                ActionKind::Insert { .. } => "insert",
                ActionKind::Remove { .. } => "remove",
                ActionKind::Research { .. } => "research",
            })
            .collect();
        assert_eq!(kinds.iter().filter(|k| **k == "place").count(), 1);
        assert_eq!(kinds.iter().filter(|k| **k == "insert").count(), 2, "ore and fuel");
        assert_eq!(kinds.iter().filter(|k| **k == "remove").count(), 1);
        assert_eq!(kinds.iter().filter(|k| **k == "mine").count(), 2, "iron ore and coal");
    }

    #[test]
    fn the_removal_waits_for_the_smelting_time() {
        let mut s = state(&[BotId(1)]);
        s.gain(BotId(1), "stone-furnace", 1);
        let net = expand(
            &[Goal::Have { item: "iron-plate".into(), count: 2, whose: Holder::Anyone }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();
        let remove = net
            .actions()
            .find(|a| matches!(a.kind, ActionKind::Remove { .. }))
            .expect("a removal");
        // Identify each insert by what it inserts, then check its own edge to
        // the removal — a blind max() over all predecessors would pass even if
        // the ore and fuel lags were swapped, which is the whole invariant.
        let lag_from = |item: &str| -> Ticks {
            let insert = net
                .actions()
                .find(|a| matches!(&a.kind, ActionKind::Insert { item: i, .. } if i == item))
                .unwrap_or_else(|| panic!("expected an insert of {}", item));
            net.preds(remove.id)
                .into_iter()
                .find(|(from, _)| *from == insert.id)
                .unwrap_or_else(|| panic!("expected an edge from the {} insert to the removal", item))
                .1
        };

        // iron-plate is 3.2 s each, so two plates lag 2 * 192 = 384 ticks.
        assert_eq!(lag_from("iron-ore"), 384, "the ore insert carries the smelting time");
        // Fuel must be in before the removal, but does not itself take smelting time.
        assert_eq!(lag_from("coal"), 0, "the fuel insert carries no lag");
    }

    #[test]
    fn smelting_without_a_furnace_crafts_one_first() {
        let s = state(&[BotId(1)]);
        let net = expand(
            &[Goal::Have { item: "iron-plate".into(), count: 1, whose: Holder::Anyone }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();
        assert!(
            net.actions().any(|a| matches!(&a.kind, ActionKind::Craft { item, .. } if item == "stone-furnace")),
            "the bot has no furnace, so it must make one"
        );
        assert!(
            net.actions().any(|a| matches!(&a.kind, ActionKind::Mine { item, .. } if item == "stone")),
            "and mine the stone for it"
        );
    }

    #[test]
    fn a_smelted_goal_schedules() {
        let bots = [BotId(1), BotId(2)];
        let mut s = state(&bots);
        s.gain(BotId(1), "stone-furnace", 1);
        s.gain(BotId(2), "stone-furnace", 1);
        let net = expand(
            &[Goal::Have { item: "iron-plate".into(), count: 2, whose: Holder::Anyone }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();
        let plan = schedule(&net, &s, &bots).expect("schedulable");
        assert!(plan.makespan > 384, "at least the smelting time");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: FAIL — `Smelt` does not exist, so `iron-plate` falls through to `Mine`, which finds no iron-plate resource patch, and expansion errors with `NoApplicableMethod`.

- [ ] **Step 3: Write the method**

Add to `crates/planner/src/method/have.rs`, above the test module. Extend the `use` lines with `crate::method::util::{ingredients_of, output_per_craft, recipe_for, recipe_ticks, free_tile_near}` and `factorio_bot_core::types::FactorioEntity`.

```rust
/// How many plates one coal will smelt in a stone furnace.
///
/// A coal carries 4 MJ and a stone furnace draws 90 kW, so one coal sustains
/// about 44 seconds of smelting — roughly 13 plates at 3.2 s each. This is an
/// approximation: it ignores partial burns carried between smelts, and it
/// assumes stone-furnace speed. Calibrating it against observed burn rates is
/// follow-up work for the execution increment.
pub const PLATES_PER_COAL: u32 = 13;

/// Time to put items into or take them out of a machine.
const TRANSFER_TICKS: Ticks = 10;

/// Time to place an entity.
const PLACE_TICKS: Ticks = 30;

/// Smelt the shortfall in a stone furnace.
pub struct Smelt;

impl Method for Smelt {
    fn name(&self) -> &'static str {
        "smelt"
    }

    fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
        let Goal::Have { item, count, whose } = goal else {
            return false;
        };
        if shortfall(state, item, *count, whose) == 0 {
            return false;
        }
        matches!(recipe_for(state, item), Some(r) if r.category == "smelting")
    }

    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        let Goal::Have { item, count, whose } = goal else {
            return Err(PlannerError::NoApplicableMethod { goal: goal.to_string() });
        };
        let need = shortfall(&ctx.state, item, *count, whose);
        let recipe = recipe_for(&ctx.state, item).ok_or_else(|| PlannerError::NoApplicableMethod {
            goal: goal.to_string(),
        })?;
        let per_craft = output_per_craft(&recipe, item);
        let runs = need.div_ceil(per_craft);
        let coal = runs.div_ceil(PLATES_PER_COAL).max(1);

        let from = ctx
            .state
            .bot(ctx.chain_actor)
            .map(|b| b.position.clone())
            .unwrap_or_default();
        let pos = free_tile_near(&ctx.state, &from).ok_or_else(|| {
            PlannerError::NoApplicableMethod { goal: goal.to_string() }
        })?;
        let build = ctx.state.bot(ctx.chain_actor).map(|b| b.build_distance).unwrap_or(10.0);
        let reach = ctx.state.bot(ctx.chain_actor).map(|b| b.reach_distance).unwrap_or(10.0);

        let furnace = FactorioEntity {
            name: "stone-furnace".into(),
            entity_type: "furnace".into(),
            position: pos.clone(),
            ..Default::default()
        };

        let mut steps: Vec<Step> = Vec::new();

        // Ingredients, fuel, and the furnace itself, as subgoals.
        for (ingredient, amount) in ingredients_of(&recipe) {
            steps.push(Step::Subgoal(Goal::Have {
                item: ingredient,
                count: amount.saturating_mul(runs),
                whose: whose.clone(),
            }));
        }
        steps.push(Step::Subgoal(Goal::Have {
            item: "coal".into(),
            count: coal,
            whose: whose.clone(),
        }));
        steps.push(Step::Subgoal(Goal::Have {
            item: "stone-furnace".into(),
            count: 1,
            whose: whose.clone(),
        }));

        let place_id = ctx.ids.next();
        steps.push(Step::Act(Box::new(Action {
            id: place_id,
            kind: ActionKind::Place { entity: Box::new(furnace.clone()) },
            pre: vec![
                Condition::AtPosition { who: Actor::Role, pos: pos.clone(), radius: build },
                Condition::PositionFree { pos: pos.clone() },
                Condition::HasItem { who: Actor::Role, item: "stone-furnace".into(), count: 1 },
            ],
            eff: vec![
                Effect::LoseItem { who: Actor::Role, item: "stone-furnace".into(), count: 1 },
                Effect::CreateEntity(Box::new(furnace)),
            ],
            duration: PLACE_TICKS,
            pinned: None,
            label: format!("place stone-furnace at {}", pos),
        })));

        let mut insert_ids = Vec::new();
        for (ingredient, amount) in ingredients_of(&recipe) {
            let total = amount.saturating_mul(runs);
            let id = ctx.ids.next();
            insert_ids.push(id);
            steps.push(Step::Act(Box::new(Action {
                id,
                kind: ActionKind::Insert { pos: pos.clone(), item: ingredient.clone(), count: total },
                pre: vec![
                    Condition::AtPosition { who: Actor::Role, pos: pos.clone(), radius: reach },
                    Condition::EntityAt { pos: pos.clone(), name: "stone-furnace".into() },
                    Condition::HasItem { who: Actor::Role, item: ingredient.clone(), count: total },
                ],
                eff: vec![Effect::LoseItem { who: Actor::Role, item: ingredient.clone(), count: total }],
                duration: TRANSFER_TICKS,
                pinned: None,
                label: format!("insert {} {}", total, ingredient),
            })));
        }

        let fuel_id = ctx.ids.next();
        insert_ids.push(fuel_id);
        steps.push(Step::Act(Box::new(Action {
            id: fuel_id,
            kind: ActionKind::Insert { pos: pos.clone(), item: "coal".into(), count: coal },
            pre: vec![
                Condition::AtPosition { who: Actor::Role, pos: pos.clone(), radius: reach },
                Condition::EntityAt { pos: pos.clone(), name: "stone-furnace".into() },
                Condition::HasItem { who: Actor::Role, item: "coal".into(), count: coal },
            ],
            eff: vec![Effect::LoseItem { who: Actor::Role, item: "coal".into(), count: coal }],
            duration: TRANSFER_TICKS,
            pinned: None,
            label: format!("fuel the furnace with {} coal", coal),
        })));

        let remove_id = ctx.ids.next();
        steps.push(Step::Act(Box::new(Action {
            id: remove_id,
            kind: ActionKind::Remove { pos: pos.clone(), item: item.clone(), count: need },
            pre: vec![
                Condition::AtPosition { who: Actor::Role, pos: pos.clone(), radius: reach },
                Condition::EntityAt { pos: pos.clone(), name: "stone-furnace".into() },
            ],
            eff: vec![Effect::GainItem { who: Actor::Role, item: item.clone(), count: need }],
            duration: TRANSFER_TICKS,
            pinned: None,
            label: format!("take {} {} from the furnace", need, item),
        })));

        // The furnace runs between the last insert and the removal. The bot is
        // free to do other work across this lag — that is what it is for.
        let smelt_lag = recipe_ticks(&recipe).saturating_mul(runs);
        for id in insert_ids {
            let lag = if id == fuel_id { 0 } else { smelt_lag };
            steps.push(Step::Link { from: id, to: remove_id, lag });
        }

        Ok(steps)
    }
}
```

Add `use crate::ids::Ticks;` to the file's imports, and register the method in `default_registry()` between `AlreadySatisfied` and `Mine`:

```rust
pub fn default_registry() -> MethodRegistry {
    MethodRegistry::new()
        .with(Box::new(AlreadySatisfied))
        .with(Box::new(Smelt))
        .with(Box::new(Mine))
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: PASS — 4 more tests.

If `smelting_without_a_furnace_crafts_one_first` fails with `NoApplicableMethod` for `stone-furnace`, that is expected until Task 6 adds `HandCraft` — in that case **report it and move to Task 6 rather than weakening the test**, then confirm it passes at the end of Task 6.

- [ ] **Step 5: Verify lints and commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-planner && cargo clippy --workspace --all-features --all-targets -- --deny warnings --deny deprecated'
git commit -m "feat(planner): add the smelting method with its furnace lag" -- crates/planner
```

---

### Task 6: `HandCraft`

**Files:**
- Modify: `crates/planner/src/method/have.rs`
- Test: the existing inline `mod tests` in `crates/planner/src/method/have.rs`

**Interfaces:**
- Consumes: Tasks 1-5.
- Produces: `pub struct HandCraft;` implementing `Method`, registered after `Smelt` and before `Mine`.

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `crates/planner/src/method/have.rs`:

```rust
    #[test]
    fn hand_crafting_expands_its_ingredients() {
        let mut s = state(&[BotId(1)]);
        s.gain(BotId(1), "iron-plate", 4);
        let net = expand(
            &[Goal::Have { item: "iron-gear-wheel".into(), count: 2, whose: Holder::Anyone }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();
        assert_eq!(net.len(), 1, "the plates are already held, so just the craft");
        let action = net.actions().next().unwrap();
        match &action.kind {
            ActionKind::Craft { item, count } => {
                assert_eq!(item, "iron-gear-wheel");
                assert_eq!(*count, 2);
            }
            other => panic!("expected a craft, got {:?}", other),
        }
        // 0.5 s per gear, two gears.
        assert_eq!(action.duration, 60);
    }

    #[test]
    fn hand_crafting_consumes_its_ingredients() {
        let mut s = state(&[BotId(1)]);
        s.gain(BotId(1), "iron-plate", 4);
        let net = expand(
            &[Goal::Have { item: "iron-gear-wheel".into(), count: 2, whose: Holder::Anyone }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();
        let action = net.actions().next().unwrap();
        assert!(action.pre.iter().any(
            |c| matches!(c, Condition::HasItem { item, count, .. } if item == "iron-plate" && *count == 4)
        ));
        assert!(action.eff.iter().any(
            |e| matches!(e, Effect::LoseItem { item, count, .. } if item == "iron-plate" && *count == 4)
        ));
    }

    #[test]
    fn the_whole_science_chain_expands() {
        let mut s = state(&[BotId(1)]);
        s.gain(BotId(1), "stone-furnace", 1);
        let net = expand(
            &[Goal::Have {
                item: "automation-science-pack".into(),
                count: 1,
                whose: Holder::Anyone,
            }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();
        let labels: Vec<String> = net.actions().map(|a| a.label.clone()).collect();
        assert!(labels.iter().any(|l| l.contains("mine") && l.contains("iron-ore")), "{:?}", labels);
        assert!(labels.iter().any(|l| l.contains("mine") && l.contains("copper-ore")), "{:?}", labels);
        assert!(labels.iter().any(|l| l.contains("iron-gear-wheel")), "{:?}", labels);
        assert!(labels.iter().any(|l| l.contains("automation-science-pack")), "{:?}", labels);
    }

    #[test]
    fn the_science_chain_schedules_without_a_precondition_failure() {
        let bots = [BotId(1), BotId(2)];
        let mut s = state(&bots);
        s.gain(BotId(1), "stone-furnace", 2);
        s.gain(BotId(2), "stone-furnace", 2);
        let net = expand(
            &[Goal::Have {
                item: "automation-science-pack".into(),
                count: 1,
                whose: Holder::Anyone,
            }],
            &s,
            &default_registry(),
            BotId(1),
        )
        .unwrap();
        let plan = schedule(&net, &s, &bots).expect("the chain must be schedulable");
        assert_eq!(
            plan.steps.iter().filter(|s| matches!(s.what, crate::schedule::StepKind::Act { .. })).count(),
            net.len(),
            "every action is scheduled"
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: FAIL — `iron-gear-wheel` has no applicable method, so expansion errors.

- [ ] **Step 3: Write the method**

Add to `crates/planner/src/method/have.rs`, above the test module:

```rust
/// Craft the shortfall by hand, expanding each ingredient as a subgoal.
pub struct HandCraft;

impl Method for HandCraft {
    fn name(&self) -> &'static str {
        "hand-craft"
    }

    fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
        let Goal::Have { item, count, whose } = goal else {
            return false;
        };
        if shortfall(state, item, *count, whose) == 0 {
            return false;
        }
        matches!(recipe_for(state, item), Some(r) if r.category == "crafting")
    }

    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        let Goal::Have { item, count, whose } = goal else {
            return Err(PlannerError::NoApplicableMethod { goal: goal.to_string() });
        };
        let need = shortfall(&ctx.state, item, *count, whose);
        let recipe = recipe_for(&ctx.state, item).ok_or_else(|| PlannerError::NoApplicableMethod {
            goal: goal.to_string(),
        })?;
        let runs = need.div_ceil(output_per_craft(&recipe, item));

        let mut steps: Vec<Step> = Vec::new();
        let mut pre = Vec::new();
        let mut eff = Vec::new();

        for (ingredient, amount) in ingredients_of(&recipe) {
            let total = amount.saturating_mul(runs);
            steps.push(Step::Subgoal(Goal::Have {
                item: ingredient.clone(),
                count: total,
                whose: whose.clone(),
            }));
            pre.push(Condition::HasItem {
                who: Actor::Role,
                item: ingredient.clone(),
                count: total,
            });
            eff.push(Effect::LoseItem {
                who: Actor::Role,
                item: ingredient,
                count: total,
            });
        }
        eff.push(Effect::GainItem {
            who: Actor::Role,
            item: item.clone(),
            count: runs.saturating_mul(output_per_craft(&recipe, item)),
        });

        steps.push(Step::Act(Box::new(Action {
            id: ctx.ids.next(),
            kind: ActionKind::Craft {
                item: item.clone(),
                count: runs,
            },
            pre,
            eff,
            duration: recipe_ticks(&recipe).saturating_mul(runs),
            pinned: None,
            label: format!("craft {} {}", runs, item),
        })));

        Ok(steps)
    }
}
```

Register it in `default_registry()` between `Smelt` and `Mine`:

```rust
pub fn default_registry() -> MethodRegistry {
    MethodRegistry::new()
        .with(Box::new(AlreadySatisfied))
        .with(Box::new(Smelt))
        .with(Box::new(HandCraft))
        .with(Box::new(Mine))
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: PASS — 4 more tests, plus `smelting_without_a_furnace_crafts_one_first` from Task 5 if it was deferred.

- [ ] **Step 5: Verify lints and commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-planner && cargo clippy --workspace --all-features --all-targets -- --deny warnings --deny deprecated'
git commit -m "feat(planner): add the hand-crafting method" -- crates/planner
```

---

### Task 7: `SplitAcrossBots` and the end-to-end slice

**Files:**
- Modify: `crates/planner/src/method/have.rs`
- Create: `crates/planner/tests/red_science.rs`
- Test: both of the above

**Interfaces:**
- Consumes: Tasks 1-6.
- Produces: `pub struct SplitAcrossBots { pub bots: Vec<BotId> }` implementing `Method`; `pub fn registry_for(bots: &[BotId]) -> MethodRegistry`.

`SplitAcrossBots` divides a `Holder::Anyone` count into `min(bot_count, count)` shares as evenly as possible, each an independent per-bot chain. It is registered first among the producing methods so it claims a multi-unit `Anyone` goal before the others see it, and it emits `Holder::Bot(_)` subgoals that the other methods then handle without recursing back into it.

- [ ] **Step 1: Write the failing tests**

Append to `mod tests` in `crates/planner/src/method/have.rs`:

```rust
    #[test]
    fn a_shared_goal_splits_into_one_chain_per_bot() {
        let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
        let mut s = state(&bots);
        for b in bots {
            s.gain(b, "iron-ore", 0);
        }
        let net = expand(
            &[Goal::Have { item: "iron-ore".into(), count: 8, whose: Holder::Anyone }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .unwrap();
        assert_eq!(net.len(), 4, "one mine per bot");
        for action in net.actions() {
            match &action.kind {
                ActionKind::Mine { count, .. } => assert_eq!(*count, 2, "8 split four ways"),
                other => panic!("expected mines, got {:?}", other),
            }
        }
    }

    #[test]
    fn an_uneven_split_distributes_the_remainder() {
        let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
        let s = state(&bots);
        let net = expand(
            &[Goal::Have { item: "iron-ore".into(), count: 10, whose: Holder::Anyone }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .unwrap();
        let mut counts: Vec<u32> = net
            .actions()
            .map(|a| match &a.kind {
                ActionKind::Mine { count, .. } => *count,
                other => panic!("expected mines, got {:?}", other),
            })
            .collect();
        counts.sort_unstable();
        assert_eq!(counts, vec![2, 2, 3, 3], "10 across four bots");
        assert_eq!(counts.iter().sum::<u32>(), 10);
    }

    #[test]
    fn a_count_smaller_than_the_roster_uses_only_as_many_chains_as_needed() {
        let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
        let s = state(&bots);
        let net = expand(
            &[Goal::Have { item: "iron-ore".into(), count: 2, whose: Holder::Anyone }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .unwrap();
        assert_eq!(net.len(), 2, "two chains for two units");
    }

    #[test]
    fn a_single_unit_goal_is_not_split() {
        let bots = [BotId(1), BotId(2)];
        let s = state(&bots);
        let net = expand(
            &[Goal::Have { item: "iron-ore".into(), count: 1, whose: Holder::Anyone }],
            &s,
            &registry_for(&bots),
            BotId(1),
        )
        .unwrap();
        assert_eq!(net.len(), 1);
    }
```

Create `crates/planner/tests/red_science.rs`:

```rust
//! The first vertical slice: ten red science packs, four bots, no Factorio.

use factorio_bot_core::test_utils::fixture_world;
use factorio_bot_planner::goal::{Goal, Holder};
use factorio_bot_planner::method::expand;
use factorio_bot_planner::method::have::registry_for;
use factorio_bot_planner::schedule::StepKind;
use factorio_bot_planner::{mermaid_gantt, schedule, BotId, PlanState};
use std::sync::Arc;

fn world_with_furnaces(bots: &[BotId]) -> PlanState {
    let mut state = PlanState::from_world(Arc::new(fixture_world()), bots);
    for bot in bots {
        // Every bot starts the way `Planner` seeds a fresh player.
        state.gain(*bot, "stone-furnace", 2);
    }
    state
}

fn goal(count: u32) -> Goal {
    Goal::Have {
        item: "automation-science-pack".into(),
        count,
        whose: Holder::Anyone,
    }
}

#[test]
fn ten_red_science_across_four_bots_expands_and_schedules() {
    let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let state = world_with_furnaces(&bots);
    let net = expand(&[goal(10)], &state, &registry_for(&bots), BotId(1)).expect("expands");
    assert!(net.len() > 10, "a real chain, not a stub: {} actions", net.len());

    let plan = schedule(&net, &state, &bots).expect("schedulable");
    let acted: usize = plan
        .steps
        .iter()
        .filter(|s| matches!(s.what, StepKind::Act { .. }))
        .count();
    assert_eq!(acted, net.len(), "every action is scheduled exactly once");
}

#[test]
fn every_bot_is_used() {
    let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let state = world_with_furnaces(&bots);
    let net = expand(&[goal(10)], &state, &registry_for(&bots), BotId(1)).unwrap();
    let plan = schedule(&net, &state, &bots).unwrap();
    for bot in bots {
        assert!(
            !plan.steps_for(bot).is_empty(),
            "{} was given nothing to do",
            bot
        );
    }
}

#[test]
fn more_bots_finish_sooner() {
    let state_one = world_with_furnaces(&[BotId(1)]);
    let net_one = expand(&[goal(4)], &state_one, &registry_for(&[BotId(1)]), BotId(1)).unwrap();
    let one = schedule(&net_one, &state_one, &[BotId(1)]).unwrap().makespan;

    let four = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let state_four = world_with_furnaces(&four);
    let net_four = expand(&[goal(4)], &state_four, &registry_for(&four), BotId(1)).unwrap();
    let many = schedule(&net_four, &state_four, &four).unwrap().makespan;

    assert!(
        many < one,
        "four bots ({} ticks) must beat one ({} ticks)",
        many,
        one
    );
}

#[test]
fn the_plan_renders_as_a_gantt_chart() {
    let bots = [BotId(1), BotId(2)];
    let state = world_with_furnaces(&bots);
    let net = expand(&[goal(2)], &state, &registry_for(&bots), BotId(1)).unwrap();
    let plan = schedule(&net, &state, &bots).unwrap();
    let chart = mermaid_gantt(&plan, "Red science");
    assert!(chart.starts_with("gantt"));
    assert!(chart.contains("section bot 1"));
    assert!(chart.contains("section bot 2"));
    assert!(chart.contains("mine"));
}

#[test]
fn expansion_is_deterministic() {
    let bots = [BotId(1), BotId(2), BotId(3), BotId(4)];
    let state = world_with_furnaces(&bots);
    let first = expand(&[goal(10)], &state, &registry_for(&bots), BotId(1)).unwrap();
    let second = expand(&[goal(10)], &state, &registry_for(&bots), BotId(1)).unwrap();
    let labels_a: Vec<String> = first.actions().map(|a| a.label.clone()).collect();
    let labels_b: Vec<String> = second.actions().map(|a| a.label.clone()).collect();
    assert_eq!(labels_a, labels_b);

    let plan_a = schedule(&first, &state, &bots).unwrap();
    let plan_b = schedule(&second, &state, &bots).unwrap();
    assert_eq!(plan_a.makespan, plan_b.makespan);
    assert_eq!(plan_a.steps, plan_b.steps);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: FAIL to compile — `SplitAcrossBots` and `registry_for` do not exist.

- [ ] **Step 3: Write the method**

Add to `crates/planner/src/method/have.rs`, above the test module:

```rust
/// Split a shared goal into one independent chain per bot.
///
/// The chains never coordinate: each mines, smelts and crafts its own share.
/// They are emitted as `Holder::Bot(_)` subgoals so the other methods handle
/// them without recursing back into this one.
pub struct SplitAcrossBots {
    pub bots: Vec<BotId>,
}

impl Method for SplitAcrossBots {
    fn name(&self) -> &'static str {
        "split-across-bots"
    }

    fn applicable(&self, goal: &Goal, state: &PlanState) -> bool {
        let Goal::Have { item, count, whose } = goal else {
            return false;
        };
        if !matches!(whose, Holder::Anyone) {
            return false;
        }
        // Only worth splitting when there is more than one share to give out.
        self.bots.len() > 1 && shortfall(state, item, *count, whose) > 1
    }

    fn expand(&self, goal: &Goal, ctx: &mut ExpansionCtx) -> Result<Vec<Step>, PlannerError> {
        let Goal::Have { item, count, whose } = goal else {
            return Err(PlannerError::NoApplicableMethod { goal: goal.to_string() });
        };
        let need = shortfall(&ctx.state, item, *count, whose);
        let chains = (self.bots.len() as u32).min(need);
        let base = need / chains;
        let remainder = need % chains;

        let mut steps = Vec::new();
        for (index, bot) in self.bots.iter().take(chains as usize).enumerate() {
            let share = base + if (index as u32) < remainder { 1 } else { 0 };
            steps.push(Step::Subgoal(Goal::Have {
                item: item.clone(),
                count: share,
                whose: Holder::Bot(*bot),
            }));
        }
        Ok(steps)
    }
}

/// The registry to use for a given bot roster.
pub fn registry_for(bots: &[BotId]) -> MethodRegistry {
    MethodRegistry::new()
        .with(Box::new(AlreadySatisfied))
        .with(Box::new(SplitAcrossBots { bots: bots.to_vec() }))
        .with(Box::new(Smelt))
        .with(Box::new(HandCraft))
        .with(Box::new(Mine))
}
```

Note that `default_registry()` stays as it is — single-bot callers and the earlier tests use it — and `registry_for` is the multi-bot entry point. Add `use crate::ids::BotId;` if it is not already imported.

**A subtlety to get right:** a per-bot share is expanded with `whose: Holder::Bot(b)`, so `shortfall` consults that bot's own inventory rather than the total. Because `ExpansionCtx` simulates every chain against `chain_actor`, a later chain would otherwise see the earlier chain's items and think itself satisfied. `Holder::Bot(b)` is what prevents that: bot *b*'s simulated inventory stays empty until *its* chain produces something. Do not "simplify" the subgoals back to `Holder::Anyone`.

- [ ] **Step 4: Re-export from `lib.rs`**

Add to `crates/planner/src/lib.rs`:

```rust
pub use method::have::registry_for;
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test -p factorio-bot-planner'`
Expected: PASS — 4 unit tests plus 5 integration tests in `red_science.rs`.

If `more_bots_finish_sooner` fails, **report the two makespans and stop.** It is the plan's headline claim, and a failure means either the split or the scheduler is not delivering parallelism — that is a finding, not an assertion to relax.

- [ ] **Step 6: Run the whole workspace suite**

Run: `nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo test --workspace'`
Expected: PASS. Then discard the snapshot churn this produces:

```bash
git checkout -- crates/scripting_lua/tests/
```

- [ ] **Step 7: Verify lints and commit**

```bash
nix develop --command bash -c 'eval "$(mise env -s bash)"; cargo fmt -p factorio-bot-planner && cargo clippy --workspace --all-features --all-targets -- --deny warnings --deny deprecated'
git commit -m "feat(planner): split shared goals across bots and add the red science slice" -- crates/planner
```

---

## Self-Review

**Spec coverage.** Layer 1 `Goal` and `Holder` (T1), including `Producing` as the documented functorio bridge with a test pinning that it has no method yet. Layer 2 `Method`, the registry and the expansion driver with state progression and a depth bound (T1, T2). The spec's named methods: `AlreadySatisfied` and `Mine` (T4), `Smelt` with the insert-to-removal lag that models a furnace (T5), `HandCraft` (T6), `SplitAcrossBots` dividing into `min(bot_count, count)` shares as evenly as possible (T7). The first slice — `Have(automation-science-pack, N)` across four bots — is T7's integration test.

**Deliberate deviations from the spec, flagged.**

1. **`Goal::Built` is omitted.** It needs `BlueprintRef` and `Placement` types that nothing in this increment uses. It arrives with blueprint work.
2. **`Holder::Chest` is omitted, and with it `TakeFromChest` and `Consolidate`.** The spec names both as first-slice methods, but they cannot be written yet: `PlanState` models no container inventory, so neither can express "two actions must not spend the same stack" — the spec's third reservation category, which its own implementation note records as unmodelled. The red-science slice does not need them, because `SplitAcrossBots` gives each bot a self-contained chain and `Holder::Anyone` is satisfied by the sum across bots without anything being physically consolidated. **Consolidation becomes unavoidable for `Researched`**, since science must reach one lab, which is why that goal has no method here either.
3. **`Goal::Researched` has a variant but no method**, for the reason just given. The spec's first slice is `Have`, not `Researched`, so this is deferred rather than missing.
4. **Fuel is modelled approximately.** `PLATES_PER_COAL = 13` is derived from a coal's 4 MJ against a stone furnace's 90 kW, ignores partial burns carried between smelts, and assumes stone-furnace speed. The constant is documented where it is defined.

**Placeholder scan.** No TBD or TODO entries. Every code step carries its code; every test step carries its assertions. Two places where a downstream task's absence could cause a failure — Task 5's furnace-crafting test before `HandCraft` exists, and Task 7's parallelism claim — say explicitly what to do, and in both cases the instruction is to report rather than weaken.

**Type consistency.** `shortfall(state, item, count, whose) -> u32` is used identically by all five methods. `Step::{Subgoal, Act(Box<Action>), Link{from,to,lag}}` is constructed the same way in T2's tests and every method. `ExpansionCtx { state, ids, chain_actor, depth }` has the same four fields wherever it appears. `Method::{name, applicable, expand}` signatures are unchanged from T1 through T7. `default_registry()` gains members across T4-T6 and is shown in full each time it changes; `registry_for(bots)` is introduced once, in T7, and the plan states explicitly that `default_registry()` remains for single-bot callers. `recipe_for`, `ingredients_of`, `output_per_craft`, `recipe_ticks`, `mining_ticks`, `nearest_resource_tile`, `free_tile_near` and `seconds_to_ticks` are defined once in T3 and used with those exact names afterwards.

**A type check worth recording.** `BotId` derives `Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize` but **not** `Default`, so `Option<BotId>::unwrap_or_default()` does not compile. An earlier draft of `Mine::applicable` relied on it to pick a reference position; it now asks the position-independent question instead — does any tile hold enough — which is both compilable and more honest about what applicability means. `Position` *does* implement `Default` (`core/src/types.rs:326`), so `..Default::default()` on `FactorioEntity` is fine.
