# The power plant — 2026-09-02

**Crates:** `crates/planner` (all of the work), plus three compile-forced edits
outside it, named in §8.
**Follows:** `2026-09-02-research-needs-power.md` (stage 1),
`2026-09-02-building-power.md` (stage 2a), `2026-09-02-water-is-solid.md` (the
gate).
**Status:** **rung 7 plans.** `Goal::Researched("automation")` against a fresh
map with a lake now returns a 171-action network ending in
`research automation`, having built an offshore pump, three pipes, a boiler, a
steam engine and a pole, and fuelled the boiler. It has never once done that in
this project's history.

The sentence that blocked it —

```
automation needs a lab with 60 kW of electric supply, and the plan can show only 0 kW
```

— is no longer reachable from `Researched::expand` at all. It is now the
*trigger* for building a plant rather than the end of the road, and the
refusals a caller can still get name the **water** instead (§4).

---

## 1. What a rung-7 plan looks like end to end

One bot, the shared fixture world (its lake is a 4x4 block of `water` tiles at
`38..=41` on both axes), `Goal::Researched("automation")`. 171 actions,
makespan **67,098 ticks**. The placements, in scheduled order, with the mining,
smelting and crafting between them elided:

```
 11,586  place small-electric-pole at [34.5, 29.5]
 18,802  place offshore-pump      at [39.5, 37.5]     facing south, into the lake
 24,855  place boiler             at [36.5, 36]       facing north
 24,885  fuel the boiler with 5 coal
 42,156  place pipe               at [36.5, 34.5]     boiler steam  -> engine
 42,186  place pipe               at [39.5, 36.5]     pump output   -> ...
 42,216  place pipe               at [38.5, 36.5]     ...           -> boiler water
 42,246  place steam-engine       at [36.5, 31.5]     facing south
 50,426  place lab                at [32.5, 27.5]
 61,088  insert 10 automation-science-pack into the lab
 61,098  research automation                          (6,000 ticks)
 67,098  makespan
```

Fifteen stone furnaces are placed along the way; that is the pre-existing
smelting path, not the plant.

Read the plant's geometry from the pump outwards. The pump stands on tile
`(39, 37)`, which is *land*, with the lake's tiles `(38..=40, 38..=39)` in front
of it — the pump faces the water, its body extends into it, and its output pipe
comes out behind. One pipe sits on that output, a second one step along the
shore, and the boiler's water inlet reaches that second pipe. The boiler faces
away from the water, so its steam comes out inland onto a third pipe, and the
engine's inlet reaches the same tile. The pole goes beside the engine, the lab
goes inside the pole's supply area, and the packs go into the lab.

**Nothing in that layout is hardcoded as a set of offsets.** The pump's
position and facing come from a shoreline search; everything below it is
*derived* from a table of fluid-connection offsets by subtracting the offset
from the joint it has to reach. Change a number in that table and the buildings
move.

---

## 2. The joint rule, which is the whole design

Two fluidboxes connect when each one's connection point is the other's target.
A **pipe**'s connection point is its own centre (vanilla `position = {0, 0}`,
four directions), so:

> **An entity connects to a pipe standing on that entity's target tile.**

So the layout does not have to reason about mutual adjacency at all. It puts a
pipe on a tile that *two* entities both name, and both are joined to it. Three
joints, three pipes, three iron plates.

The alternative — abutting the boiler and the engine directly, as vanilla
players do — works and saves one pipe, but needs the "A's target is inside B and
B's target is inside A" rule instead of the "both name this tile" rule, and it
is the harder one to be sure of without building it. The pipe is worth one iron
plate.

`the_pipes_stand_where_both_neighbours_reach` asserts the joints at **all four
facings**, because a wrong rotation breaks them *silently*: every building still
places, and the plant does nothing. That is the `only_ghosts = true` failure and
the inserter-direction failure in their third costume, and it is the failure
this test exists to make impossible.

---

## 3. **The connection geometry cannot be read from `fluidbox_prototypes`.** Two reasons, both checked

Stage 2a §5 recommended reading the offsets straight out of the prototype:

> nothing needs to hardcode "a boiler is 3x2 and its steam comes out the narrow
> end" — read the offsets from the prototype

That is right about the *shape* of the problem and wrong about the data. Checked
rather than trusted:

### 3.1 The two captures in this repo disagree about what `positions` means

| connection | `entity-prototype-fixtures.json` | `live-2.1.17-world-snapshot.json` |
| --- | --- | --- |
| `offshore-pump` output | `(0, 1)` | `(0, 0)` |
| `boiler` water #1 | `(-2, 0.5)` | `(-1, 0.5)` |
| `boiler` steam | `(0, -1.5)` | `(0, -0.5)` |
| `steam-engine` #1 | `(0, 3)` | `(0, 2)` |
| `pipe` (all four) | `(0, -1)`, `(1, 0)`, … | `(0, 0)` ×4 |

Every entry differs by **exactly one tile along that connection's own
direction**. The fixture holds Factorio 1.x's reading — the *target*, the tile
the connecting fluidbox must occupy. Live 2.1.17 holds 2.x's
`PipeConnectionDefinition::position`, documented as *"position relative to
entity's center where pipes can connect to this fluidbox"*, which is a point
**inside** the entity.

So code that reads `positions` as a target is one tile wrong against a real
game, and code that reads it as a point is one tile wrong against every test in
`crates/planner`. There is no reading that is right for both.

### 3.2 The direction is not sent, and cannot be recovered

Turning a 2.x point into a target needs
`PipeConnectionDefinition::direction`. `FactorioEntityPrototype`
(`crates/core/src/types.rs`) has no field for it, and the mod does not collect
it. It cannot be inferred geometrically either:

* the **pump**'s connection point is its own centre, so *all four* cardinals
  leave its collision box;
* the **boiler**'s water point `(-1, 0.5)` is 0.289 tiles from the west edge and
  0.289 tiles from the south edge — west and south are equally plausible and
  only one is right.

### 3.3 So the north-frame geometry is written down

In `crates/planner/src/method/power.rs`, from the vanilla prototype definitions
(`workspace/data/base/prototypes/entity/entities.lua`, readable in this
checkout), the same discipline as `crate::state`'s pole tables and
`COAL_BURN_TICKS`. What *is* read from the game is the **rotation**, and that is
pinned twice:

* `the_four_rotations_of_a_connection_are_the_prototypes_own_four` checks, for
  every fluidbox of every plant entity, that `positions[k]` really is
  `positions[0].turn(4k)`. That is what makes `Position::turn` the right
  rotation for these tables — and it is worth pinning, because `turn`'s sense is
  not obvious: `rotate_clockwise` is `(x, y) -> (y, -x)`, which is
  *anti*clockwise on a y-down screen.
* `the_connection_table_matches_the_prototype_the_tests_plan_against` ties every
  hardcoded entry to the fixture's own `positions[0]`, so the two halves cannot
  drift apart without a test saying so.

**The follow-up that deletes these tables** is sending `direction` on
`FactorioFluidBoxConnection` from the mod, exactly as sending
`supply_area_distance` / `max_energy_production` would delete stage 1's pole and
generator tables. Until then, do not "simplify" `PUMP_OUTPUT` and friends into a
read of `fluidbox_prototypes`: it compiles, it passes against the fixture, and
it is one tile wrong in every real run.

---

## 4. The siting rule, and its refusals

**Plant at the water, coal carried.** Stage 2a argued this from numbers and
nothing here changes them: `pipe-to-ground` is 15 iron plates per 10 tiles
against a whole rung-7 bill of ~98, so 60 tiles of pipe roughly doubles it,
while coal is five items in an inventory.

The consequence for the threshold is worth stating plainly, because it moves the
brief's own number:

> Stage 2a's break-even of **40–60 tiles** is the length at which a *pipe run*
> stops paying. A plant sited at the water has no pipe run — three pipes, three
> tiles — so that cost is gone. What is left is a **walk**: the pump, the pipes,
> the boiler, the engine, the pole, five coal, the lab and ten science packs are
> all carried to the shore from wherever the ore was. So the bound is the same
> 64 tiles `electric_supply_kw` and the lab search already use, and the refusal
> names the distance, which is what the brief actually asked for.

Three refusals, all carrying numbers:

| variant | when | message |
| --- | --- | --- |
| `PowerPlantTooFarFromWater` | water found, beyond 64 tiles | *"the nearest water is 78.5 tiles away, and a power plant may not be sited more than 64 tiles from the bot that has to carry it there"* |
| `PowerPlantNeedsWater` | no water within 128 tiles | *"a power plant needs water, and the plan can see none within 128 tiles"* |
| `PowerPlantNeedsShore` | water near enough, no buildable edge | *"the nearest water is N tiles away, but no shoreline within 10 tiles of it has room for a pump, a boiler, a steam engine and the pipes between them"* |

`PowerPlantNeedsWater` is **not** the same claim as "this map has no lakes", and
its `help` says so: a world attached from a snapshot fetches no tiles at all
(`attach_world`), and an owned run knows only the charted chunks.

### The wide search runs only on the failing path

`nearest_water_tile` is linear in the tiles inside its radius, and since
`fa8dabf3` a fully charted map carries ~410,000 water tiles. The first draft
searched 128 tiles unconditionally to buy the distance for the refusal — a
256x256 box, ~65,000 tiles cloned, on **every** successful expansion. It now
searches 64 first and widens to 128 only when that finds nothing. That is the
one place the extra 410,000 blocking boxes visibly bit; nothing else in the
suite got measurably slower.

---

## 5. The shoreline rule, and why the existing one is wrong for 2.x

Stage 2a §8 asked for the predicate in
`FactorioRcon::find_offshore_pump_placement_options` to be ported. **It was
deliberately not ported, because it is wrong for Factorio 2.x.** That function
keeps *"a **water** tile whose neighbour ahead is not water, while both lateral
neighbours are"* — i.e. the 1.x pump, which stands **in** the water with land in
front of it.

The 2.0 pump stands on **land** with the water in front. Its own
`tile_buildability_rules` say so, and they are in this checkout:

```lua
tile_width = 1, tile_height = 1,
tile_buildability_rules = {
  {area = {{-0.4, -0.4}, {0.4, 0.4}}, required_tiles = ground, colliding_tiles = water},
  {area = {{-1, -2},     {1, -1}},    required_tiles = water},
}
```

So the ported rule is: the pump's own tile is ground, and a **three-wide by
two-deep block ahead of it is water**. The block is the second rule's box,
rotated with the facing.

Three things about that, each of which cost something to establish:

* **`tile_width = 1, tile_height = 1` is stated explicitly**, against a collision
  box of 1.195 x 1.344 tiles that would otherwise round *up* to 2 x 2. It is the
  only real entity in vanilla `base` that overrides them (checked: the other
  hits are corpses and one train). The difference is not cosmetic — an even
  extent puts the entity's centre on a tile **corner**, from which the first
  buildability rule cannot be satisfied by any shoreline that exists. The mod
  sends neither field, so `explicit_tile_extent` in `method/util.rs` carries the
  one exception, documented; `no offshore-pump grid override` is a mutation and
  it is caught.
* **The rule is deliberately stricter than the game's.** The second box only
  *half* covers its outer columns, and whether the engine tests overlap or
  coverage is not settleable from this repo. Requiring all six tiles refuses
  shorelines the game might accept and accepts none it would refuse — the safe
  direction when the alternative is committing a bot to a walk and a placement
  that fails.
* **The pump is the one building allowed to overlap water**, and that had to be
  taught to `is_area_clear`. Since `fa8dabf3` every water tile is a blocking box,
  and a pump on a real shoreline has its body over two of them; without this the
  plant refuses every site it can find. The permission is read from the game —
  the pump's `collision_mask` has no water layer, and the vanilla prototype
  comments the reason in place — not named here. **Both spellings** are matched
  (`water-tile` and `water_tile`), because the fixture capture uses 1.x layer
  names and the live capture uses 2.x ones; matching one makes it true in tests
  and false in a run.

---

## 6. Rotation, and the condition that had to learn about it

**No planner method had ever emitted a non-zero `direction`.** The plant emits
four. That turned two latent assumptions into bugs:

1. **`collision_area` returned the north-facing box.** A boiler is 3x2 tiles
   facing north and **2x3 facing east**; a placement checked against the
   unrotated box is checking the wrong ground. `collision_area_facing` and
   `is_area_free_facing` are the rotated pair, and `create_entity` /
   `footprint_of` now read the entity's own `direction` when they fill a
   footprint in.
2. **`Condition::AreaFree` could not express it.** It carries a `direction: u8`
   now. Three call sites needed a `direction: 0` (two in the planner, one test
   helper in `crates/executor`), and `an_area_free_check_turns_with_the_building`
   pins the difference: an obstacle 1.05 tiles east of a site is inside a
   north-facing boiler's box (half-width 1.289) and clear of an east-facing one
   (half-width 0.789), so the same `pos` and `entity` must answer differently.

The whole plant is a **rigid body rotated about the pump's tile centre**, which
is what keeps every building on its own build grid at all four facings: a
quarter turn about a tile centre maps tile centres to tile centres and tile
corners to tile corners, and each building's direction turns with the body.
`every_facing_puts_every_building_on_its_own_grid` is that claim as a test, and
`the_plants_own_buildings_never_overlap_each_other` is what makes it sound for
`fit` to check each building against the world rather than against its siblings.

---

## 7. Fuel, and what still is not detected

Five coal into the boiler's `InventorySlot::Fuel`, through the same path the
stone furnaces already use. The arithmetic is pinned by a test rather than
asserted in a comment
(`the_boilers_fuel_bill_covers_the_research_several_times_over`): coal carries
4 MJ, a lab draws `LAB_POWER_KW`, `research_ticks(automation)` is 6,000 ticks =
100 s, so the research alone is **6 MJ — one and a half coal**, and the test
also asserts that *one* coal is genuinely short, without which the bound would
be satisfied by any number at all.

**If it runs dry, nothing detects it.** `electric_supply_kw` counts nameplate
capacity, so a boiler with an empty fuel slot still reads as 900 kW, and
`crates/executor` waits on the game's own `on_research_finished` with no
modelled duration — it would sit for ever at whatever percentage the research
reached. That is stated in `PLANT_COAL`'s own doc and it is the residual this
change does not close. Closing it wants a fuel monitor, not a bigger constant.

**Wood is still the hard cap.** The plant spends **one** pole, which is one
craft (`wood 1 + copper-cable 2`, yields 2). Every bot starts with exactly one
wood and the planner cannot make more, so the roster's lifetime budget is four
wood. The fixture's bots start empty, so the research tests seed the one wood a
real bot has; `unpowered_lakeside_state`'s doc says why, at length, because a
test fixture that silently differs from the game on this would make the cap
invisible.

---

## 8. Ordering: stated, not inferred

No `Effect` satisfies `Condition::Powered` — it is a statement about the world,
checked against the state — so `ActionNetwork::infer_edges` can draw **no** edge
from any part of the plant to the research. The method states them, exactly as
it already states the pack-insert-before-research edges.

**Every** part, not just the generator and the pole. `Powered` counts nameplate
capacity, so as far as the model is concerned an engine and a pole are enough —
but an engine with no steam produces nothing, and a boiler with no water or no
coal produces no steam. The scheduler would probably get there anyway by
rejecting the research until `Powered` holds; "probably" is not an ordering.

The plant's sites are also **reserved in `ctx.state` as each `Place` is
emitted**, the same discipline the lab placement uses and for the same reason:
`expand` returns its whole step list before `run_steps` executes any of it, so an
unreserved site is chosen twice by two subtrees of the same plan. `plant sites
are not reserved during expansion` is a mutation and it takes five tests.

---

## 9. Edits outside `crates/planner`

Three, all compile- or test-forced, all flagged so they can be reviewed in
isolation:

| file | why | size |
| --- | --- | --- |
| `crates/executor/src/recover.rs` | `Condition::AreaFree` gained a field; this is a test helper that builds one | 1 line |
| `crates/scripting_lua/src/globals/goal/mod.rs` | `refusal_for` matches `PlannerError` exhaustively; the three new variants are **verdicts** (facts about the map, false on another map), classified with `ResearchNeedsPower` | 3 lines + a doc bullet, and one test retargeted |
| `crates/scripting_lua/tests/goal_script.lua` | it asserted the refusal that no longer happens | see below |

The Lua fixture is the interesting one, and it is the thing stage 1 wrote down
in advance:

> **What is lost** is the end-to-end "a research plan reaches Lua with a
> research action and mining in it" coverage. It comes back for free the day the
> whitelist admits poles.

It is back, and better than predicted: the world does not have to *carry* a
plant, the plan *builds* one. `goal_script.lua` now asserts a positive makespan,
exactly one research action and some mining — the assertions stage 1 removed.

The Rust twin, `a_research_goal_with_no_power_raises_a_recognisable_refusal`,
still exists and still pins the same seam (a planner verdict reaches Lua
carrying its own diagnostic code, so a milestone loop can tell "this world
cannot do that" from "this planner is broken" without matching message text).
What changed is which verdict: it now puts the bot 80 tiles from the fixture's
lake and checks `planner::power_plant_too_far_from_water`. It is renamed to say
so.

---

## 10. Red first, with the failure output

The behaviour change was written as five tests against the un-wired planner, so
the first run is the red evidence, and it is rung 7's own sentence:

```
---- method::have::tests::rung_seven_builds_the_power_it_needs ----
a world with a lake can build its own power:
ResearchNeedsPower { technology: "automation", needed_kw: 60.0, supply_kw: 0.0 }

---- method::have::tests::the_research_waits_for_every_piece_of_the_plant ----
---- method::have::tests::the_plant_stands_on_the_shore_and_the_lab_stands_by_the_plant ----
---- method::have::tests::a_rung_seven_plan_is_identical_on_a_second_expansion ----
---- method::have::tests::a_research_with_no_water_anywhere_refuses_for_want_of_water ----
        the control must plan: ResearchNeedsPower { … }

test result: FAILED. 369 passed; 5 failed
```

Wiring `Researched::expand` to the plant turned those five green and turned two
*existing* tests red, which is the honest shape of the behaviour change:

```
---- method::have::tests::research_refuses_when_the_lab_would_have_no_power ----
expected ResearchNeedsPower, got NoApplicableMethod { goal: "have 1 wood (a share sized for bot 1)" }

---- method::have::tests::a_pole_with_nothing_generating_is_not_power ----
got NoApplicableMethod { goal: "have 1 wood (a share sized for bot 1)" }
```

Both asserted "no power ⇒ refuse" through `expand`, which now answers "no power
⇒ build a plant". They were **not** deleted: they were moved down to `lab_site`,
which is where the refusal now lives and is still load-bearing — a `lab_site`
that quietly stopped refusing would stop the plant being built at all *and* put
the lab back where run 30 had it. Their doc comments say so.

**Honest gap:** `crates/planner/src/method/power.rs` itself was written
test-and-implementation together, so its eleven module tests have no red-first
evidence beyond a compile failure — which `2026-09-02-water-is-solid.md` §4
rightly calls weak. Every behavioural claim in that module is therefore pinned
by a mutation below instead, and four of them were *only* pinned after a
mutation survived and showed the test set was decorative (§11).

---

## 11. Mutations: 22, applied one at a time, each reverted after

| # | mutation | caught by |
| --- | --- | --- |
| 1 | boiler takes the other water connection | `the_pipes_stand_where_both_neighbours_reach`, `the_plants_own_buildings_never_overlap_each_other`, `a_rung_seven_plan_is_identical_on_a_second_expansion` |
| 2 | engine takes the other steam connection | the same three |
| 3 | no lateral step before the boiler | the same three |
| 4 | boiler faces the water instead of away from it | all nine plant tests |
| 5 | shoreline looked for behind the pump | all nine, plus `a_shoreline_needs_water_ahead_and_ground_underfoot` |
| 6 | a pump may stand in the lake | `a_shoreline_needs_water_ahead_and_ground_underfoot` **(alone)** |
| 7 | everything collides with water | all eight end-to-end plant tests |
| 8 | no `offshore-pump` grid override | `every_facing_puts_every_building_on_its_own_grid` **(alone)** |
| 9 | tile alignment ignores the rotation | `every_facing_puts_every_building_on_its_own_grid` **(alone)** |
| 10 | collision boxes are never rotated | `the_plants_own_buildings_never_overlap_each_other`, `a_rung_seven_plan_is_identical_on_a_second_expansion` |
| 11 | one coal instead of five | `the_boilers_fuel_bill_covers_the_research_several_times_over` **(alone)** |
| 12 | the research is not ordered after the plant | `the_research_waits_for_every_piece_of_the_plant` **(alone)** |
| 13 | any pole supplies anything | `a_pole_only_supplies_what_its_supply_area_reaches` **(alone)** |
| 14 | plant sites not reserved during expansion | the five end-to-end tests |
| 15 | water is never too far away | `water_beyond_the_siting_radius_is_refused_with_the_distance` **(alone)** |
| 16 | water always blocks, whatever the mask says | the eight end-to-end plant tests |
| 17 | `AreaFree` checks the unrotated box | `an_area_free_check_turns_with_the_building` **(alone)** |
| 18 | the plant does not fuel the boiler at all | the five end-to-end tests |
| 19 | the coal is billed as zero | the five end-to-end tests |
| 20 | the shoreline search never widens past the nearest water tile | the six siting tests |
| 21 | the plant emits no `Place` actions at all | the five end-to-end tests |
| 22 | *(15 re-run after the search was reordered)* | as above |

Eight of the twenty-two are caught by exactly one test, which is the shape that
says the test is about the thing it names.

### The four that survived a first pass, and what each one showed

This is the part worth more than the table.

* **6 — "a pump may stand in the lake" survived.** The negative control was
  built on a lake only two rows deep, so a water tile inside it had *no* water
  ahead of it either: both halves of the predicate refused, and dropping the
  ground check changed nothing. Fixed by deepening the lake to four rows, so
  that the tile at `(1, 3)` has a perfect block of water ahead and the **only**
  thing refusing it is the ground under the pump.
* **11 — "one coal instead of five" survived.** The end-to-end test asserted
  `count == PLANT_COAL`, which follows the constant wherever it goes and pins
  nothing about the quantity. Replaced by the arithmetic, with its own control.
* **13 — "any pole supplies anything" survived.** `fit` chooses the pole's site
  by that predicate alone; a predicate that says yes to everything sites the
  pole on the first free tile and the plant reads as powered by luck. It
  survived because on the fixture's open shoreline the first free tile *is* in
  range. Pinned directly now, including that a stone furnace supplies nothing.
* **17 — "`AreaFree` checks the unrotated box" survived.** The fixture's
  shoreline is open enough that the rotation never changes the verdict, so the
  whole reason the field was added was unpinned. Fixed by constructing the case
  where the two orientations differ (§6).

Four survivors out of eighteen on the first pass, and every one of them was a
test that looked like it covered something and did not.

### One thing that could not be made to go red, and what it pins instead

`the_connection_table_matches_the_prototype_the_tests_plan_against` cannot fail
under any mutation *of this change*, because it compares two constants that this
change wrote together. What it pins is the **future**: it is the tripwire on the
finding in §3. Anyone who replaces the hardcoded tables with a read of
`fluidbox_prototypes` will make it pass trivially and will have shipped a plant
that is one tile wrong in every real run; anyone who *edits* one table entry
without the other gets a failure naming both numbers. It is a seam, not a
behaviour test, and it is labelled as one.

### Determinism

`a_rung_seven_plan_is_identical_on_a_second_expansion` expands the whole rung-7
goal six times and compares every action label **and** the schedule's makespan;
`the_same_world_sites_the_same_plant_twice` does eleven expansions of the plant
alone and compares the whole `Plant` structure. Every ordering in the new code
is explicit: the water tile is the one `EntityGraph::nearest_water_tile` orders
first, the shoreline candidates come out of a ring search in a fixed order, the
four facings are tried north/east/south/west, and the water set is a
`BTreeSet<Pos>`. Nothing reads a quad tree's own order, and every float
comparison is `total_cmp`.

---

## 12. Verification

* `nix develop -c cargo test -p factorio-bot-planner` — **377 lib + 69
  integration**, from 358 + 69. **Every existing makespan pin passed unchanged**;
  no pin was touched or justified away.
* `nix develop -c cargo test -p factorio-bot-scripting-lua` — 260, from 259.
* `nix develop -c cargo test --workspace` — 62 suites, 0 failed.
* `nix develop -c cargo clippy --workspace --all-targets -- --deny warnings` —
  clean.
* `nix develop -c rustfmt --edition 2024` on the ten touched files only.
  **`cargo fmt --all` was not run, and neither was `cargo fmt -p`**: it rewrites
  a whole crate and other agents are editing the same checkout.
* **No Factorio process was launched.**

---

## 13. What is not settled without a live build, and what was shipped anyway

Named explicitly, because stage 2a asked for exactly this and because the two
items below are the ones that fail *silently* if they are wrong — the layout
places 100%, every geometry check passes, and the plant produces nothing.

1. **The joint rule.** "An entity connects to a pipe standing on its target
   tile" is derived from the vanilla prototypes (a pipe's connection point is
   its own centre; an entity's target is its point plus one tile in the
   connection's direction) and is consistent with both data captures under their
   respective readings. It has **not** been observed in a running game.
2. **The three-by-two water block.** Whether Factorio's `tile_buildability_rules`
   test tiles that *overlap* the rule's box or tiles the box *covers* is not
   settleable from this repo. The strict reading was shipped, which can only
   refuse a legal site and never accept an illegal one.

Both are cheap to check in one run: build the plant, then read back
`entity.fluidbox`'s neighbours, or simply look at whether the engine produces.
Until somebody does, they are assumptions, and they are the assumptions.

Two more, less sharp:

3. **The plant is welded to one bot.** Every subgoal is
   `Holder::Share(chain_actor)` — the same weld the lab and the packs already
   have — so one bot mines, smelts, crafts and carries ~45 iron plates of plant
   on top of the lab and the packs. Whether that fits a real inventory is not
   something the planner checks, and a real multi-bot decomposition for
   `Researched` is still stage 1's open item 6.
4. **Fuel is still a one-shot insert**, per §7.

---

## 14. What remains, in order

1. **Run it.** Everything in §13.
2. **Send `direction` on `FactorioFluidBoxConnection`** from the mod, and delete
   the tables in `power.rs`. Same shape as sending the electrical prototype
   fields and deleting stage 1's.
3. **Fuel as a standing obligation**, with a condition or a monitor on the
   boiler's remaining fuel.
4. **A real multi-bot decomposition for `Researched`**, now with a plant welded
   to `chain_actor` as well as a lab and ten packs.
5. Still open from stage 1: unify `BOT_FORCE` in `crates/core`; send the
   electrical prototype fields.
6. **One stale doc comment left alone.** `PlanState::entities_within`'s doc
   still says `electric-pole` and `generator` "are not on" `EntityGraph::add`'s
   whitelist, which `8f40b6cf` made false. Not corrected here, to keep this
   change to one subject; it is three sentences in `crates/planner/src/state.rs`.
