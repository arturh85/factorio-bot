# `set_recipe`, end to end

**Date:** 2026-09-03. The verb only. `docs/superpowers/notes/2026-09-03-red-science-automated.md`
§5 item 1 asked for exactly this chain and this note is its answer:

```
ActionKind::SetRecipe / Condition::RecipeSet / Effect::SetRecipe   crates/planner
  -> the dispatch arm in `perform`                                 crates/executor
  -> Actuator::set_recipe / RconActuator::set_recipe               crates/executor
  -> FactorioRcon::set_recipe_timed                                crates/core
  -> rcon_set_recipe                                               mods/BotBridge
```

**Nothing here builds the stage-2 layout, and nothing here has met a game.**
No method emits a `SetRecipe`; the planner gained vocabulary, exactly as it
gained `Condition::Powered` and `Condition::Feeds` before a method produced
either. Every claim below is source, tests, and the game's own
`workspace/factorio-api-docs/runtime-api.json` (Factorio 2.1.17).

---

## 1. What `LuaEntity.set_recipe` actually returns

**Not a success flag. An array of `ItemWithQualityCount`.** Read out of
`runtime-api.json`, `LuaEntity`, method `set_recipe`:

```json
"return_values": [{
  "description": "Any items removed from this entity as a result of setting the recipe.",
  "type": { "complex_type": "array", "value": "ItemWithQualityCount" },
  "optional": false
}]
```

`ItemWithQualityCount` is `{ name: string, quality: string, count: ItemCountType }`.
The signature is `set_recipe(recipe: RecipeID?, quality: QualityID?)`, and
`"takes_table": false` — so it is `entity.set_recipe(name)` positionally, not a
table call and not a colon call.

Two consequences, and both are why the brief said to check:

* **Discarding the return value destroys items.** Those are the old recipe's
  ingredients and products, evicted because they no longer belong in the
  machine. This is the same shape as the three discarded returns that already
  cost this project something — `remove_item`'s actual count, `create_entity`'s
  optional entity, `player.teleport`'s boolean. `rcon_set_recipe` hands them to
  the acting bot, who is standing at the machine to operate it, and
  `player.insert`'s own return value (how many it took) is checked in turn.
* **The verdict has to come from somewhere else.** A machine that ignores the
  call still answers it with an array — an empty one — so nothing in the
  documented return distinguishes success from a silent no-op. The handler
  reads the recipe back with `get_recipe()` and refuses if it is not the one
  asked for. Without that read this verb would report success on exactly the
  placed-but-dead machine it exists to prevent.
  `a_recipe_that_did_not_take_is_refused` drives a stub that takes the call and
  does nothing, which is the only way to state the difference.

**Also from the same file, and load-bearing:** `set_recipe` is listed under
`"subclasses": ["AssemblingMachine"]`. A stone furnace has `get_recipe` and no
`set_recipe`, so calling it on one raises *inside the remote call* — which
reaches the executor as an unreadable reply rather than as a refusal it can act
on. The handler checks `entity.type` first and names what the thing actually
is.

Two attributes were looked at and deliberately **not** used:
`recipe_locked` ("the recipe in this assembling machine can't be changed by the
player") and `disabled_by_recipe`. Neither is what "locked recipe" means in
this task — that is the force's `enabled` flag — and modelling machine-side
recipe locking with no caller that produces one would be inventing a
requirement. Named here as a residual rather than left unmentioned.

## 2. How a locked recipe is refused

**By name, in the reply body, before the game is touched, and force-scoped.**

```lua
local known = player.force.recipes[recipe]
if known == nil then
    rcon.print("Error: no such recipe: " .. tostring(recipe))
    return
end
if not known.enabled then
    rcon.print("Error: recipe " .. tostring(recipe) .. " is not enabled for this force")
    return
end
```

The flag lives on `player.force.recipes[name]`, not on the prototype, which is
the whole reason the wording says *for this force*. It is the same check
`rcon_action_start_crafting` already makes and deliberately the same wording:
one refusal a reader learns to recognise, not two.

Three distinctions this keeps apart, each with its own sentence and its own
test:

| what happened | message | what a reader should do |
|---|---|---|
| the name refers to nothing | `no such recipe: X` | fix the spelling |
| the force has not unlocked it | `recipe X is not enabled for this force` | research the unlocker first |
| nothing is standing there | `cannot set a recipe on nonexisting entity X at (a/b)` | the plan aimed at empty ground |
| it is not an assembling machine | `... it is a furnace, not an assembling-machine` | the plan aimed at the wrong machine |

**Refusals reach the executor as `Dispatch::Refused`, not `NotDispatched`.**
`set_recipe_timed` judges the reply with `judge_set_recipe_reply`, which is the
same rule `judge_transfer_reply` uses and is strong for the same reason: the
handler prints **only** its `§tick§` stamp when the recipe is set, so an empty
reply once the stamp is off is the mod asserting all of it — player, recipe,
force, machine, type, the game taking the recipe, and the evicted items
reaching the bot. Anything left over is a verdict the game gave, carried
through verbatim.

**Nothing is written to a durable-refusal ledger, and that is the distinction
the placement path already makes.** `PlacementRefusal` exists because ground
the game refuses stays refused and the planner must route around it;
`PlacementVerdict::is_durable_refusal` separates that from "a character was
standing there" (transient) and "we could not ask" (nothing observed). A locked
recipe is none of those three: it is a fact the planner *already models*, from
the same `enabled` flag the world sends it (`crates/planner`'s `recipe_gate`).
Recording it again would be a second copy of a judgement that can disagree with
the first. What the mod's own line separates is "we could not ask" — no such
player, which is `ActionFailure::not_dispatched` — from every refusal above,
which is `refused` and keeps the dispatch tick.

### 2.1 The planner half: `PlannedResearch` is still not `Open`

`Condition::RecipeSet` says nothing about whether the force may *use* the
recipe. It is a statement about one machine, and its doc says so in place: a
method emitting `ActionKind::SetRecipe` must gate the recipe with
`recipe_gate` and honour all four answers.

That obligation is pinned by two tests in `network.rs` rather than by prose:

* `a_planned_unlock_orders_the_recipe_after_the_research_that_enables_it` — a
  `SetRecipe` carrying `Condition::Researched(tech)` gets an inferred edge from
  the research action.
* `a_recipe_that_states_no_gate_is_ordered_by_nothing` — the control. An
  identical action that states no gate gets **no** edge and would be dispatched
  at tick zero, which is precisely what `run-1788338409-63794` did with three
  crafts of a locked recipe.

So the cost of reading `PlannedResearch` as `Open` is visible as a missing edge
in a test, not as a sentence somebody has to remember. `BOT_FORCE = "player"`
is untouched; nothing here sorts forces.

## 3. Determinism: no pin moved

**Every makespan pin passes unchanged and none was edited.** `red_science.rs`
(7), `scheduling.rs` (9), `placement_occupancy.rs` (9), `refusal_memory.rs`
(10), `tile_capacity.rs` (7), `tile_occupancy.rs` (4), `tile_reservation.rs`
(5), `split_capacity.rs` (5), `smelt_roots.rs` (3), `seeded_roster.rs` (3),
`buffers.rs` (12), `ore_underfoot.rs` (4), `recipe_probability.rs` (3) — all
green, plus 444 planner unit tests. The ladder's `8/8/37/40/16/52/111` is
untouched, and it could hardly be otherwise: no method emits a `SetRecipe`, so
no expansion reaches any of the new code.

The new code is deterministic by construction: `PlanState::set_recipe` writes
one `BTreeMap` entry keyed by `Pos`, `Condition::RecipeSet` compares two
strings, and `Effect::satisfies` compares a `Pos` and a string. No floats, no
iteration order, no I/O.

Three enum variants were added, each at the **end** of its enum, so no existing
variant's serialised form moved. The replay wire format is untouched:
`crates/executor/src/replay.rs` carries `{"kind":"act","action":N,"label":".."}`
and never an `ActionKind`, so `app/src/api/replay.ts` and the OpenAPI snapshot
needed no change. Four exhaustive matches over `ActionKind` outside the planner
had to grow an arm — the executor's `perform` (the one the earlier note
predicted), `crates/scripting_lua`'s plan renderer, and two test helpers — and
one over `PlannerError` in `refusal_for`, where `NoMachineForRecipe` is a
**fault** rather than a verdict, alongside `BufferShort` for the same reason:
it says an expansion contradicted itself, not that the world is unfavourable.

## 4. Red-first, with the actual output

### 4.1 The mod: a genuine red

Lua is dynamically typed, so a test that calls a function which does not exist
compiles and *fails*. All eleven did:

```
---- setting_a_recipe_replies_with_only_a_tick_stamp stdout ----
thread 'setting_a_recipe_replies_with_only_a_tick_stamp' panicked at
crates/core/tests/botbridge_set_recipe.rs:224:10:
handler call: RuntimeError("[string \"call\"]:1: attempt to call a nil value
(global 'rcon_set_recipe')\nstack traceback:\n\t[C]: in global
'rcon_set_recipe'\n\t[string \"call\"]:1: in main chunk")

test result: FAILED. 0 passed; 11 failed
```

### 4.2 The planner: red on the behaviour, not on the vocabulary

A Rust test naming a variant that does not exist is a **compile error, not a
red test**, and saying so is the honest answer. What can be red is the
behaviour: the three variants were added first with the minimal compiling body
— `holds` returning `false`, `apply` returning `Ok(())`, and no arm in
`satisfies` — and the suite was run against that:

```
---- action::tests::a_set_recipe_effect_satisfies_the_matching_condition ----
assertion failed: eff.satisfies(&Condition::RecipeSet { pos, recipe: "automation-science-pack".into() })

---- action::tests::setting_a_recipe_replaces_the_one_before_it ----
assertion `left == right` failed
  left: Some("iron-gear-wheel")
 right: Some("automation-science-pack")

---- action::tests::setting_a_recipe_where_there_is_no_machine_is_an_error ----
nothing stands there: ()

---- action::tests::setting_a_recipe_makes_the_condition_hold ----
the overlay must carry what the effect claims

---- action::tests::a_machine_already_carrying_the_recipe_satisfies_it ----
assertion failed: Condition::RecipeSet { .. }.holds(&s, BotId(1))

test result: FAILED. 436 passed; 5 failed
```

The four negative tests (`a_machine_with_no_recipe_...`,
`a_machine_carrying_another_recipe_...`, `an_empty_tile_...`,
`a_set_recipe_effect_satisfies_nothing_else`) passed against the stub, because
a stub that always says no is right about every no. They became meaningful only
once the yes existed, and the battery below is what says whether they earn
their place.

### 4.3 One red nobody wrote

`every_remote_call_the_client_sends_is_exported_by_the_mod`
(`crates/scripting_lua/src/globals/rcon.rs`) caught the missing line in
`remote.add_interface("botbridge", ..)`:

```
the RCON client sends ["set_recipe"], which `remote.add_interface("botbridge", ..)`
in mods/BotBridge/control.lua does not export -- every such call fails against
a running game.
```

Worth recording because it is the failure mode this whole chain is most exposed
to: every layer would have compiled, every test above would have passed, and
the verb would have failed only against a real game.

## 5. Mutation battery

Each applied alone to the fixed tree, the whole crate's suite run with
`--no-fail-fast`, the tree restored between each. `--no-fail-fast` matters: the
first pass without it attributed M3 to one test because `cargo test` stopped
before the integration binaries ran.

| # | Mutation | Failed |
|---|---|---|
| P1 | `Condition::RecipeSet::holds` always `true` | 4 |
| P2 | `holds` checks only that *some* recipe is set | 2 — `a_machine_carrying_another_recipe_...`, `setting_a_recipe_replaces_...` |
| P3 | `Effect::SetRecipe::apply` a no-op (the feature, absent) | 3 |
| P4 | `PlanState::set_recipe` returns `Ok(())` on empty ground | `setting_a_recipe_where_there_is_no_machine_is_an_error` |
| P5 | `satisfies` ignores the recipe name | `a_set_recipe_effect_satisfies_nothing_else` |
| P6 | `satisfies` ignores the position | the same one |
| P7 | `target_position` answers `None` for `SetRecipe` | `a_set_recipe_acts_at_the_machine` |
| P8 | **negative control** — reword `NoMachineForRecipe`'s `help(..)` | **nothing** — the wording is pinned only where a test reads it |
| P9 | the `satisfies` arm removed entirely | 2 — the unit test **and** `inference_orders_a_load_after_the_recipe_that_decides_it` |
| P10 | the pre-existing `(Researched, Researched)` arm removed | 5, including `a_planned_unlock_orders_the_recipe_after_the_research_that_enables_it` — so the gate test is not vacuous |
| P11 | `set_recipe` appends instead of assigning | 2 — `setting_the_same_recipe_twice_changes_nothing`, `setting_a_recipe_replaces_...` |
| E1 | the dispatch arm swaps `entity` and `recipe` | `set_recipe_dispatches_the_machine_and_the_recipe_the_action_named` |
| E2 | `refused_by_the_game` escalates a failed `SetRecipe` | `a_failed_set_recipe_is_retried_at_tier_one` |
| M1 | the mod discards `set_recipe`'s return value | 2 — both eviction tests |
| M2 | the `get_recipe()` verification skipped | `a_recipe_that_did_not_take_is_refused` |
| M3 | the `enabled` check removed | 2 — the mod test **and** the core seam test |
| M4 | the `entity.type` check removed | `a_machine_that_is_not_an_assembler_is_refused_by_type` |
| M5 | `player.insert`'s return value assumed to be the full count | `items_the_player_cannot_carry_are_reported_as_lost` |
| M6 | one `rcon.print` of narration added to the success path | **6** — every success test in both files |
| M7 | **negative control** — reword the eviction-loss message | **nothing** |
| M8 | `judge_set_recipe_reply` ignores the reply lines | `a_locked_recipe_is_refused_and_the_reply_names_it` |
| M9 | the unknown-recipe check removed | `an_unknown_recipe_is_refused_by_name` |
| M10 | the unknown-player check removed | `an_unknown_player_is_refused` |
| M11 | the missing-entity refusal made silent | `a_missing_machine_is_refused_and_names_the_tile` |

**M6 is the one worth reading twice.** Six tests die from a single narration
line, because `rcon.print` *is* the reply body and the executor reads the reply
body as the action's result. That is the rule CLAUDE.md states and this is it
priced: a debug line on this path turns every success into a reported failure.

**Two tests are killed by no mutation, and both are named rather than
counted.**

* `a_machine_already_carrying_the_recipe_satisfies_it` is the positive control
  for `holds`. The mutation that kills it is `holds => false`, which *is* the
  red-first state in §4.2 — it is listed there with its output instead of here.
* `a_recipe_that_states_no_gate_is_ordered_by_nothing` is a control by
  construction: it asserts an absence, so only a mutation that invents edges
  would kill it, and no such mutation is a plausible edit of this change.

## 6. Residuals, stated rather than closed

* **Nothing has met a game.** Every mod-side claim rests on `runtime-api.json`
  and a stub. In particular: whether `set_recipe` on a recipe the machine's
  `crafting_categories` do not cover raises or silently declines is **not**
  established — the `get_recipe()` read catches the second, and only a run
  catches the first.
* **`recipe_locked` and `disabled_by_recipe` are unmodelled.** §1.
* **No method emits a `SetRecipe`**, so `Condition::RecipeSet` and
  `Effect::SetRecipe` are exercised only by hand-built networks. The gating
  obligation in §2.1 is documented and pinned at the network level; it is not
  yet *executed* by anything.
* **The evicted-items path is a refusal on overflow.** A bot with no room for
  what the recipe change threw out gets the action reported as failed even
  though the recipe is set. That is safe only because the verb is idempotent
  (the retry evicts nothing), it is stated in the message, and it under-claims
  in the direction this codebase chooses everywhere else. A bot with a full
  inventory in a stage-2 cell would retry three times and then escalate, which
  is a correct-but-slow answer to a problem whose real fix is not filling the
  bot up.
* **A machine reachable from two tiles.** `PlanState::set_recipe` keys the
  overlay by the tile it was *asked* about, matching `create_entity` and
  `remove_entity`. Every method names a machine by its centre, so this is
  currently a distinction without a difference; a caller that named a corner
  would write an overlay entry `entity_at(centre)` does not read.

## 7. Files

**Mod half:** `mods/BotBridge/control.lua` (`rcon_set_recipe`, and the missing
line in `remote.add_interface`), `crates/core/tests/botbridge_set_recipe.rs`
(new, 11 tests).

**Rust half:** `crates/planner/src/action.rs` (`Condition::RecipeSet`,
`Effect::SetRecipe`, `ActionKind::SetRecipe`, `holds`/`apply`/`satisfies`/
`target_position`/`Display`, 10 tests), `crates/planner/src/state.rs`
(`PlanState::set_recipe`), `crates/planner/src/error.rs`
(`NoMachineForRecipe`), `crates/planner/src/network.rs` (3 tests),
`crates/core/src/factorio/rcon.rs` (`set_recipe_timed`,
`judge_set_recipe_reply`, 2 tests), `crates/executor/src/actuator.rs`
(`Actuator::set_recipe`), `crates/executor/src/rcon_actuator.rs` (the
implementation), `crates/executor/src/run.rs` (the dispatch arm, 1 test),
`crates/executor/src/recover.rs` (1 test),
`crates/scripting_lua/src/globals/goal/{mod,plan}.rs` (the two exhaustive
matches), plus arms in three test helpers.

**Untouched:** `app/src/`, `scripts/`, the OpenAPI snapshot, every makespan
pin.
