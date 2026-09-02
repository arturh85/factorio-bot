# Stage 1 of the starter factory: `Goal::Producing` gets a method

**Date:** 2026-09-03. `just test` green end to end (exit 0, 63 test binaries),
clippy clean with `--deny warnings`, and **every existing makespan pin passing
unchanged** — `red_science.rs`, `scheduling.rs`, `placement_occupancy.rs`,
`refusal_memory.rs`, `tile_capacity.rs`, `tile_occupancy.rs`,
`tile_reservation.rs`, `split_capacity.rs`, `smelt_roots.rs` and
`seeded_roster.rs` were not touched and did not move.

This is stage 1 of `docs/superpowers/specs/2026-09-03-starter-factory-design.md`
and nothing else. **Nothing here has been run against a game.** Every claim
below is source, tests, and the vanilla prototype definitions in
`workspace/data/`.

---

## 1. What a `Producing` plan looks like, end to end

`goal.producing("iron-plate", 15)` against the fixture world, one bot:

```
ActionId(0)  place burner-mining-drill at [-35, 35]
ActionId(1)  place stone-furnace at [-35, 33]
ActionId(2)  fuel the burner-mining-drill with 23 coal
ActionId(3)  fuel the stone-furnace with 14 coal
ActionId(4)  craft 1 burner-mining-drill
ActionId(5)  place stone-furnace at [-37, 33]
ActionId(6)  insert 3 iron-ore
ActionId(7)  fuel the furnace with 1 coal
ActionId(8)  take 3 iron-plate from the furnace
ActionId(9)  mine 3 iron-ore
ActionId(10) mine 1 coal
ActionId(11) craft 1 stone-furnace
ActionId(12) mine 5 stone
ActionId(13) craft 3 iron-gear-wheel
ActionId(14) place stone-furnace at [-39, 32]
...                                      (a second smelt, for the other 6 plates)
ActionId(26) mine 37 coal
```

27 actions, makespan **10,288 ticks** (~2.9 minutes of game time for one bot),
pinned by `the_whole_of_stage_one_costs_this_much`. The cell it builds makes 15
plates a minute — ~150 in its first ten minutes, against 9 iron plates and 10
stone to build and 37 coal to fuel.

The four cell actions carry ids 0–3 because `cell_steps` allocates them while
the bill is still a list of `Step::Subgoal`s the driver has not descended into
yet — the same emission order `power::plant_steps` has. That order fixes
`ActionId` allocation and therefore `schedule`'s tie-break, which is why it is
stated rather than incidental.

**Ordering is entirely inferred.** No `Step::Link` is emitted. Each fuel insert
carries `Condition::EntityAt` for the machine it loads, which is world-scoped,
so `ActionNetwork::infer_edges` draws the edge from the placement that created
it. The furnace's insert carries `EntityAt` for *both* machines plus the
`Condition::Feeds` between them, so by the time the scheduler checks the feed,
both placements have run.

### The bill, and a rationale that turned out to be false

`bill()` asks for the drills, then the furnaces, then the coal. I originally
wrote that this ordering was load-bearing — a `burner-mining-drill` costs one
`stone-furnace`, so asking for furnaces first looks like asking `HandCraft` to
eat the ones the cell is going to place.

**It is not, and a mutation is what said so.** Swapping the two lines changes no
plan and fails no test, because `run_steps` reserves each `Goal::Have`'s *whole
stated count* the moment that subgoal is satisfied
(`crates/planner/src/method/mod.rs`), so the drill's craft sees the placement
furnaces as spoken for and makes another. The doc comment now says that, and
`the_plan_crafts_a_furnace_for_the_drill_as_well_as_one_to_place` is labelled as
a test of the reservation ledger rather than of the ordering. Shipping the first
version would have left a confident wrong explanation in the file.

---

## 2. How it cannot be satisfied by a dead factory — and where it still can be

The spec splits satisfaction deliberately: the planner answers **structure**,
the supervisor answers **duration**. This change is the planner half, and the
honest boundary is worth stating in both directions.

### What "placed" is no longer enough for

`cells_standing` counts a cell only when **all three** hold, and each has its own
test and its own mutation:

| Clause | Failure it refuses | Test |
|---|---|---|
| a **burner** drill, by name | an electric drill whose network nobody looked at — "coverage is not capacity", one level out | `an_electric_drill_is_not_a_stage_one_cell_however_well_it_is_placed` |
| standing **on the ore** it mines | a drill one tile off the patch: places 100 %, mines nothing | `a_drill_beside_the_patch_is_not_a_cell` |
| **delivering into** a stone furnace | a drill facing the wrong way: places 100 %, moves nothing — the inserter-direction trap in its stage-1 form | `a_furnace_the_drill_does_not_feed_is_not_a_cell` |

The same three gate *siting*: `fit` refuses a candidate that fails any of them,
so a plan cannot contain a cell that would not have counted.

And the claim is stated in the plan, not only checked at expansion:
`Condition::Feeds` sits on the furnace's fuel insert, and the scheduler checks
preconditions against the state as it advances. **A cell whose geometry were
wrong would not schedule at all** — which the combined mutation in §4 shows
directly.

### What it still cannot see, and nothing here pretends otherwise

A structurally satisfied cell can still be producing nothing:

* **the drill's fuel has run out.** 23 coal is ~10 minutes; a full slot is ~22.
  Nothing in this stack reads a fuel level, so after that the cell is silently
  dead and `holds` keeps saying yes.
* **the furnace's output has backed up.** `PlanState` models no container
  contents for this purpose, and stage 1 emits no `Remove` — a bot visit empties
  it, and a chest and an inserter are stage 3.
* **the ore under the drill is exhausted.** `Effect::ConsumeResource` is emitted
  by mining and by nothing else; a drill neither claims nor consumes the tiles it
  stands on, so a cell on a thin patch dies at an unmodelled time.

These are §13 rows 1, 3 and 4 of the spec, unchanged. **No test in this change
can catch any of them**, and the module doc says so in those words. The only
thing that can is the supervisor's witness — **which does not exist yet**
(§5).

### The narration, because an honest empty plan looks like no plan at all

Every other goal this planner takes is satisfied by *actions*, so a plan that
does something is visible in the run's own output. A `Producing` goal is
satisfied by *machines*, and machines persist: the second time a supervisor asks
for one, the honest plan is empty — identical to a plan nobody asked for, and
only one of the two is a defect. That is the buffer-refresh problem one level
out, so `goal.plan` now narrates it in the same shape and the same place
(`paris` on stdout, `plan.rs`, first re-siting round only):

```
no iron-plate cell stands yet: planning 2 of them for 30 a minute
1 of 2 iron-plate cell(s) already stand: planning the other 1
1 iron-plate cell(s) already stand and 15 a minute needs 1: nothing left to
build. They *stand*, which is not the same as producing -- only a furnace's
output rising says that
```

The wording is deliberate: it says **stand**, never **produce**. The decision
behind it is split into a pure `production_progress` so it can be tested without
reading stdout, and two mutations of it are red (§4).

---

## 3. `rate: f64` became `per_minute: u32`, and the count is integer throughout

D2′ of the spec, taken now while it costs nothing. `Goal::Producing { item,
per_minute: u32 }`.

The point was never the field alone — it was that **no float may touch the
machine count**. So the whole division is in integer ticks:

```
drill:   ceil(60 * ore.mining_time / drill.mining_speed)   = 240 ticks/ore  (15/min)
furnace: smelting_ticks(recipe, stone-furnace) / yield     = 192 ticks/plate (18.75/min)
cell:    max(drill, furnace)                               = 240 ticks/item
cells:   ceil(per_minute * 240 / 3600)                     -- u64, exact
```

The drill is the bottleneck, which is why a cell is 15/min and not 18.75.
`cells_for(15) == 1` and `cells_for(16) == 2` are pinned, and a mutation
replacing `div_ceil` with `/` is red on three tests.

Both halves are **read from the world**, not named here: `mining_speed`,
`mining_time` and `crafting_speed` all come off the prototypes, following
`character_mining_speed`'s discipline. A rate too large to build refuses with
`TooManyCells` rather than walking a patch a million times.

**Cross-boundary fallout, reported not assumed:** the rename forced
`crates/executor/src/recover.rs`'s tier-2 fixture, which constructs a
`Goal::Producing` *because* nothing could decompose it. Since `BuildCell` now
claims the ones it understands, the fixture's item changed from `iron-plate` to
`automation-science-pack` — crafted, not smelted, so no machine this planner can
build makes it and `applicable` still says no. The test's premise is preserved
exactly; the file is outside the stated boundary and this is the only change to
it.

---

## 4. Evidence

### The one honest red, with its output

The Lua surface has a test that pins the `goal` table's exact contents, and it
caught the new binding before anything else did:

```
---- globals::goal::tests::the_goal_table_offers_exactly_the_new_surface stdout ----
thread '...' panicked at crates/scripting_lua/src/globals/goal/mod.rs:1600:10:
script: RuntimeError("crates/scripting_lua/src/globals/goal/mod.rs:1576:14:
unexpected function on the goal table: producing ...")
```

That is the mirror working in the direction that matters — a binding nobody
documented. `lua_docs.rs`'s `goal.lua` list is the other direction and had to be
extended too.

**Being plain about the rest: `crates/planner/src/method/produce.rs` is a new
module, and its tests were written alongside its implementation, not before it.**
A test for a file that does not exist is a compile error, not a red test, and
calling that "red-first" would be a decoration. What the tests were actually held
to is the battery below — and it found **four of them passing for the wrong
reason**, which is the failure mode red-first exists to prevent, caught by the
other end.

### The four tests that were passing for the wrong reason

Every one of these was green before *and* after its mutation, which is exactly
the pass this project has learned to miscount:

| Test | Why it passed regardless | Fix |
|---|---|---|
| `a_drill_beside_the_patch_is_not_a_cell` | the pair sat at the origin, which the patch-anchored search in `cells_standing` never reaches — the count was 0 for a completely different reason | moved to `(-33, 40)`, two tiles off the patch edge and well inside the search |
| `a_site_off_the_ore_does_not_fit_however_clear_the_ground_is` | at one tile off the edge, `fit` refuses because the *furnace* two tiles ahead stands on ore — the right answer, the wrong reason | moved to `(-32, 40)`, where both machines are clear of the patch |
| (none existed) — `delivery_position` preferring the game's reported `drop_position` | no fixture entity carries one, so the whole branch was unexercised | `the_games_own_drop_position_beats_the_table` |
| (none existed) — `cells_standing` excluding electric drills | the fixture has no electric drill standing anywhere | `an_electric_drill_is_not_a_stage_one_cell_however_well_it_is_placed`, which first asserts the pair really *does* feed and really *does* stand on ore, so the exclusion cannot pass through the geometry |

### Mutation table

Each mutation applied alone to the fixed tree, the whole `--lib` suite run, tree
restored between each.

| Mutation | Failed |
|---|---|
| `FURNACE_OFFSET` `(0,-2)` → `(0,-3)` | 17 tests, incl. `the_drill_drops_into_the_furnace_at_every_facing`, `a_cells_two_machines_never_overlap...` |
| `delivery_offset`'s drill entry `(-0.35,-1.3)` → `(-0.35, 1.3)` | 18 tests, incl. `a_drill_facing_away_from_its_furnace_feeds_nothing` |
| `delivers_into` tests the collision **box** instead of the tile | 17 tests — see §6, this is the 0.00078125 finding |
| `delivery_position` ignores the reported `drop_position` | `the_games_own_drop_position_beats_the_table` |
| `fit` stops requiring the drill to stand on ore | `a_site_off_the_ore_does_not_fit_however_clear_the_ground_is` |
| `cells_standing` counts a drill that feeds nothing | `a_furnace_the_drill_does_not_feed_is_not_a_cell` |
| `cells_standing` counts a drill beside the patch | `a_drill_beside_the_patch_is_not_a_cell` |
| `cells_standing` counts any `mining-drill`, not just burners | `an_electric_drill_is_not_a_stage_one_cell_however_well_it_is_placed` |
| `cells_for` truncates instead of ceiling | `the_cell_count_is_integer_arithmetic_end_to_end`, `a_bigger_rate_builds_more_cells`, `a_standing_cell_holds_its_own_rate_and_not_a_larger_one` |
| the cell rate takes `min` (the furnace) instead of `max` (the bottleneck) | `a_cell_is_a_burner_drill_dropping_into_a_stone_furnace` + 2 |
| `plan_cells` stops reserving each cell as it sites it | `two_cells_never_land_on_one_site` |
| `expand` ignores the cells that already stand | `a_half_built_factory_is_finished_rather_than_doubled` |
| `holds` goes back to `None` for `Producing` | `a_produced_goal_has_no_answer_from_possession_but_a_producing_one_does` |
| the plan stops stating `Condition::Feeds` | `the_plan_states_that_the_drill_feeds_the_furnace` |
| `BuildCell` is not registered | 9 tests |
| the narration reports a half-built factory as done | `production_progress_reports_what_stands_against_what_is_wanted` |
| the narration skips a production goal inside a `Goal::All` | the same one |
| `bill()` asks for furnaces before drills | **nothing** — see §1, the rationale was wrong and the doc now says so |
| `fit` stops re-checking the delivery on a trial fork | **nothing** — see below |

**The last row is a genuine negative control and is stated as one.** The trial
check in `fit` is belt-and-braces against the layout constants: while they are
right it cannot fail. What it buys shows up only in combination — with
`FURNACE_OFFSET` wrong **and** the trial check removed, siting succeeds and

```
test method::produce::tests::a_production_plan_schedules_end_to_end ... FAILED
```

because `Condition::Feeds` does not hold at its scheduled time. So the two
checks are a pair: `fit` refuses the site, and if it did not, the *plan* refuses
to schedule. Neither is redundant with the other, and a cell that produces
nothing cannot get past both.

### Determinism

`the_same_inputs_give_the_same_plan_twice` compares every action's id, label and
precondition list across two expansions of `Producing{iron-plate, 30}`;
`a_production_plan_schedules_end_to_end` schedules twice and compares makespans.
`cells_standing` counts through a `BTreeSet` of tiles and never reads the order
`resource_patches` returns equal-sized patches in.

The ladder's own determinism is untouched: `BuildCell` is registered **last** in
both registries, claims `Goal::Producing` and nothing else, and keeps the default
`concurrency` and `split_probe` (both `None`), so it contributes nothing to
`MethodRegistry::concurrency`'s minimum and cannot change which method claims any
existing goal.

---

## 5. What is **not** done, and why stage 1 is not finished

**The spec's own "done when" for stage 1 is not met.** It reads: *"`Producing{iron-plate, 15}` plans, builds, and a `witness` milestone with all bots idle shows the furnace's output rising."* Only the first of the three is in this change.

* **`supervisor.witness` does not exist.** The durative half — a milestone that
  dispatches no actions, waits, and asserts an output inventory rose — is a
  `scripts/supervisor.lua` addition, outside this task's boundary and not
  attempted. Until it exists **there is no evidence anywhere that a cell
  produces anything**, and a `Producing` milestone in a ladder would be a claim
  about ground only. The spec's rule that every `Producing` milestone is
  followed by its `witness` cannot be honoured yet.
* **No cell has been built in a game.** §15.1 of the spec — does a burner
  drill's output actually land in a stone furnace two tiles away — is still the
  cheapest thing to check first. It is now *better* founded than it was:
  `vector_to_place_result` was read out of
  `workspace/data/base/prototypes/entity/mining-drill.lua` rather than taken
  from the spec's table (`{-0.35, -1.3}` burner, `{0, -1.85}` electric — the
  spec was right), and `delivery_position` prefers the game's own reported
  `drop_position` over the table whenever an entity carries one. It is still
  hand-written geometry that no run has confirmed.
* **No servicing lane is modelled.** The two machines are 0.6015625 apart —
  the very gap that produced 18 of 18 walk stalls, eight at 1/256 of a tile —
  and `a_cells_two_machines_never_overlap_and_leave_no_walkable_lane_between_them`
  states that as a measurement so nobody later mistakes it for a lane. The cell
  is serviced from its flanks, and that the flanks are walkable is **not**
  asserted.
* **`Producing` is reachable from Lua but no script uses it.**
  `goal.producing(item, per_minute)` is installed, documented and round-trips
  through `goal_from_lua`; `scripts/` is outside the boundary and untouched.

### What stage 2 and 3 still need, unchanged from the spec

Stage 2: rung 7 closed live, then `ActionKind::SetRecipe` + `Effect::SetRecipe`
+ `Condition::RecipeSet` in the planner, one dispatch arm and one actuator
method in `crates/executor`, `set_recipe_timed` in `crates/core`, and
`rcon_set_recipe` in `mods/BotBridge` — **there is no `set_recipe` anywhere
today**, so an assembling machine placed now has no recipe and does nothing.
Plus D5's uncommitted-capacity ledger (`electric_demand_kw`), without which
thirteen assemblers on one 900 kW engine each pass `Powered` individually.

Stage 3: the ratio solver, exact rationals, and a coal drill feeding the boiler
— §6.2 proves hand-mined coal cannot sustain it.

---

## 6. One finding worth keeping: the delivery point misses the box by 1/1280

`delivers_into` compares **tiles**, not boxes, and that is not a convenience.

A burner drill at an integer position facing north drops at `(-0.35, -1.3)` from
its own centre. The stone furnace two tiles north of it — the vanilla starter
pair, the entire content of stage 1 — has a collision box of `±0.69921875`, so
its near edge sits at `-1.30078125`. The drop point misses it by
**0.00078125 of a tile**, one part in 1280.

Box containment would therefore have rejected the one layout stage 1 exists to
build, and any layout derived to satisfy it would have been derived from the
wrong rule. A drop point resolves to the tile it lands in; the furnace covers
both of its tiles; the 1/1280 never comes up. The mutation that swaps tile
containment for box containment fails 17 tests, which is the measurement kept as
a test.

There is one tile of slack in **one direction only**, and the asymmetry is the
reason to check the delivery rather than the distance: a furnace one tile *nearer*
is also fed, but its box overlaps the drill's, so `is_area_free_facing` refuses
that site first. `a_site_whose_furnace_would_not_be_fed_does_not_fit` says so in
place — the first version of that test asserted the near case, went red, and was
wrong.

---

## 7. Files

**Owned:** `crates/planner/` (`method/produce.rs` new, plus `goal.rs`,
`action.rs`, `state.rs`, `error.rs`, `method/have.rs`, `method/mod.rs`), and
`crates/scripting_lua/src/globals/goal/`.

**Outside the stated boundary, reported rather than assumed:**

* `crates/executor/src/recover.rs` — one test fixture, forced by the field
  rename. §3.
* `crates/scripting_lua/src/globals/goal/mod.rs`'s `refusal_for` — forced: the
  match over `PlannerError` is exhaustive, so four new variants had to be
  classified. `NoCellProduces`, `NoPatchForCell` and `NoRoomForCell` are
  **verdicts** (facts a different map makes false, and each actionable: craft it
  by hand, chart more map, aim at another patch); `TooManyCells` is a **fault**,
  by `UnknownTechnology`'s test — the bound is a constant of this planner, so no
  state of the world makes the goal meaningful and a retry meets the same number.
* `crates/scripting_lua/src/globals/goal/plan.rs` and
  `crates/scripting_lua/src/lua_docs.rs` — the narration of §2, asked for
  mid-task, and the doc-surface list that pins it.
