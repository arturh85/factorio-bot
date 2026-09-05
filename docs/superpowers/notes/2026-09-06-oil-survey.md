# Oil: a survey before any code

Written 2026-09-06 against `master` at `3a809d44`, in a worktree, without
running Factorio and without building anything. Every recipe and technology
figure below is read out of the game's own data at
`workspace/data/base/prototypes/{recipe,technology,fluid.lua,entity/*}` — not
from memory — and every planner figure is `factorio-bot plan` /
`score-map` run offline against `workspace/scripts/map.json`, the seed-31337
t=0 dump (fingerprint `c161fa3f437221d0`).

The owner's target is electric smelting plus solar/battery power that expands
itself. Both halves are gated behind petroleum. `petroleum` appears nowhere in
the planner, core, mod or scripts today. This note prices the gate and orders
the work.

---

## 1. The dependency chain, priced

### The technology ladder to petroleum

`oil-processing`'s research trigger is `{type = "mine-entity", entities =
{"crude-oil"}}` (`technology.lua`, `oil-processing`) and the game fires that
natively for a pumpjack, so **the trigger is free**. What is not free is
getting to a pumpjack. `oil-gathering` is the tech that unlocks the pumpjack
recipe, and its prerequisite chain is:

```
automation-science-pack (trigger: craft a lab)      free
  └ logistic-science-pack            75 red
  └ automation                       10 red
  └ steel-processing                 50 red
      └ automation-2                 40 red + 40 green   (needs automation, steel, green)
      └ engine                      100 red + 100 green
          └ fluid-handling           50 red + 50 green   → storage-tank, pump, barrel
              └ oil-gathering       100 red + 100 green  → pumpjack
                  └ oil-processing   TRIGGER (mine crude-oil) → oil-refinery,
                                     chemical-plant, basic-oil-processing
```

**425 red and 290 green science before a pumpjack exists**, and the planner
already plans all seven of those researches — see §5.

Past that gate:

| technology | cost | unlocks |
|---|---|---|
| `plastics` | 200 red + green | `plastic-bar` |
| `advanced-circuit` | 200 red + green | `advanced-circuit` |
| `sulfur-processing` | 150 red + green | `sulfur`, `sulfuric-acid` |
| `battery` | 150 red + green | `battery` |
| `chemical-science-pack` | 75 red + green | blue packs (needs `advanced-circuit` + `sulfur-processing`) |
| `advanced-material-processing` | 75 red + green | `steel-furnace` |
| `advanced-material-processing-2` | 250 red + green + **blue** | `electric-furnace` |
| `electric-energy-distribution-1` | 120 red + green | `medium-electric-pole`, `iron-stick` |
| `electric-energy-accumulators` | 150 red + green | `accumulator` (needs `battery`) |
| `solar-energy` | 250 red + green | `solar-panel` |

### (a) A battery

```
battery      = 20 sulfuric-acid (FLUID) + 1 iron-plate + 1 copper-plate   [chemistry, 4 s]
sulfuric-acid= 5 sulfur + 1 iron-plate + 100 water (FLUID)                [chemistry, 1 s] → 50 acid
sulfur       = 30 water (FLUID) + 30 petroleum-gas (FLUID)                [chemistry, 1 s] → 2 sulfur
petroleum-gas= 100 crude-oil (FLUID)                                      [oil-processing, 5 s] → 45 gas
crude-oil    ← pumpjack on a crude-oil well (90 kW electric, mining_speed 1)
```

New item classes: `sulfur`, `battery`, `sulfuric-acid`, `petroleum-gas`,
`crude-oil`, `water`. **Everything on this chain except `sulfur` and `battery`
is a fluid.** Pre-oil: nothing. This chain is entirely post-oil.

`accumulator` = 2 iron-plate + 5 battery. Pre-oil apart from the battery.

### (b) An electric furnace

```
electric-furnace = 10 steel-plate + 5 advanced-circuit + 10 stone-brick   [crafting, 5 s]
advanced-circuit = 2 electronic-circuit + 2 plastic-bar + 4 copper-cable  [crafting, 6 s]
plastic-bar      = 20 petroleum-gas (FLUID) + 1 coal                      [chemistry, 1 s] → 2 bars
```

Pre-oil: `steel-plate`, `stone-brick`, `electronic-circuit`, `copper-cable` —
all reachable today. Post-oil: `plastic-bar`, and therefore `advanced-circuit`.
And the *technology* `advanced-material-processing-2` needs **blue science**,
which needs `advanced-circuit` (plastic) and `sulfur` (petroleum) again. So the
electric furnace is gated by oil twice: once for its ingredients and once for
its research.

`chemical-science-pack` = 2 engine-unit + 3 advanced-circuit + 1 sulfur.

### (c) A solar panel

```
solar-panel = 5 steel-plate + 15 electronic-circuit + 5 copper-plate      [crafting, 10 s]
```

**A solar panel is entirely pre-oil.** So is its technology (`solar-energy`,
250 red+green, prerequisites `steel-processing` + `logistic-science-pack`).
This is the single most useful finding in §1: *solar generation does not need
oil at all; only the battery half of "solar + battery" does.*

The planner confirms it offline. On seed 31337, four bots:

```
researched:solar-energy   1501 actions, 223,667 ticks (1:02:07)
have:solar-panel:1        1550 actions, 249,137 ticks (1:09:12)
researched:steel-processing 273 actions,  30,659 ticks (0:08:30)
have:steel-plate:10        329 actions,  36,926 ticks (0:10:15)
researched:automation      176 actions,  21,776 ticks (0:06:02)
```

### The crafting-category wall, which is independent of fluids

`character.crafting_categories = {"crafting", "hand-crafting"}`
(`entity/entities.lua:715`). The planner's `HandCraft` already requires
`recipe.category == CRAFTING_CATEGORY` (`crates/planner/src/method/have.rs:3224`).
Therefore, *even with fluids fully modelled*:

- `plastic-bar`, `sulfur`, `sulfuric-acid`, `battery` are `chemistry` → need a
  **chemical plant**, never a bot's hands.
- `basic-oil-processing` is `oil-processing` → needs an **oil refinery**.
- `engine-unit` is `advanced-crafting` → needs an **assembling machine**
  (`assembling-machine-1.crafting_categories = {"crafting", "advanced-crafting"}`,
  `entities.lua:3185`). This one bites *before* oil: it is on the path to
  `chemical-science-pack` and to `fluid-handling`'s prerequisite `engine`.

So oil is not only a fluid problem. It is the first goal that **cannot be
hand-crafted at all** — every product past the pumpjack requires a powered
machine with a recipe set and inputs delivered. The assembly-cell machinery in
`method/assemble.rs` exists for exactly that shape, but it moves items with
inserters and chests.

---

## 2. What "fluid" breaks in the current model

The planner's quantity model is one sentence:
`pub type ItemId = String` (`crates/planner/src/ids.rs:6`), counted as `u32`,
held in a character's `BTreeMap<ItemId, u32>` inventory
(`crates/planner/src/state.rs:477`). Everything below follows from that.

| what | where | breaks how | new concept, or item-with-a-different-carrier? |
|---|---|---|---|
| `Goal::Have` | `goal.rs:83` | `available()` (`state.rs:2058`) sums character inventories only. A fluid is never in one, so `Have{item:"petroleum-gas"}` is permanently `available == 0` and permanently unsatisfiable. | **New concept.** "Have" means possession by a character; a fluid is possessed by a *fluidbox*. A `Goal::Stored{fluid, where}` is the honest analogue. |
| bill arithmetic | `HandCraft::expand`, `have.rs:3298` | Every ingredient becomes `Goal::Have{ingredient}` **and** `Condition::HasItem{who: Actor::Role}`. A fluid ingredient becomes "the bot must be carrying 20 sulfuric-acid". | New concept, but small: split the bill by `ingredient_type` before it becomes a subgoal. |
| the type is already there and discarded | `method/util.rs:616` | `ingredients_of` maps `(name, amount)` and **drops `ingredient_type`**. `FactorioIngredient.ingredient_type` and `FactorioProduct.product_type` (`crates/core/src/types.rs:322`, `:335`) already carry the game's `"item"`/`"fluid"`. | Item-with-a-carrier: stop discarding the field. This is the cheapest single change in the whole survey. |
| recipe lookup | `util.rs:95` vs `crates/core/src/factorio/world.rs:1365` | Recipes are **stored by recipe name** and **looked up by product name**. That works only because vanilla item recipes are named after their product. `recipe_for(state, "petroleum-gas")` returns `None` — there is no recipe with that name — so every method *silently declines* rather than refusing loudly. Multi-product recipes (`advanced-oil-processing` → 3 fluids) have no home in this index at all. | **New concept**: a product→recipes index, many-to-many. Also the reason a fluid goal fails quietly today. |
| `amount: u32` | `types.rs:308-312` | The game reports `Ingredient.amount` as a double; the field truncates. Vanilla fluid amounts are integers (100/45/55/20/30), so nothing breaks today, but the type is documented as a narrowing. | Watch it; not blocking. |
| `Withdraw` | `have.rs:2480`, `state.rs:654` | `withdraw_slot(entity_type)` maps only `furnace`→`FurnaceResult`, `container`→`Chest`, `assembling-machine`→`AssemblerOutput`. A **`storage-tank`, `pipe` or `pumpjack` returns `None`**, so its contents can never become a `Buffer` and can never be withdrawn. | New concept. There is no withdrawing a fluid to a character at all — a fluid leaves a tank by pipe or by barrel, and `barrel` is a `fluid-handling` unlock this project has never touched. |
| `InventorySlot` | `action.rs:549-583` | Seven variants: `Chest, FurnaceSource, FurnaceResult, Fuel, AssemblerInput, AssemblerOutput, LabInput`. **None can address a fluidbox**, and there is no Factorio API shape that would let one. | New concept: fluidboxes are addressed by index and connection, not by inventory define. |
| cell input/output | `method/produce.rs`, `method/assemble.rs` | Stage-1 cells move ore by the **drill's drop point**; stage-2 cells move items by **inserter + chest**, checked by `delivers_into` (`state.rs:2641`) from `pickup_position`/`drop_position`. Neither concept exists for fluids: a pipe has no drop position. `cell_spec` (`produce.rs:180`) additionally hard-refuses anything but a one-ingredient, amount-exactly-1, `smelting` recipe. | New concept: a *fluid link* predicate — two fluidboxes adjacent and facing each other — parallel to `delivers_into`. |
| the mod's `insert`/`take` | `control.lua` `rcon_insert_to_inventory` / `rcon_remove_from_inventory` | Item inventories only. `LuaPlayer.insert` cannot take a fluid, so there is nothing to extend. | Not extendable. Fluids move by geometry, not by a verb. |
| record samples | `crates/core/src/record/samples.rs:223` | `MachineSample.input/output/fuel` are all `BTreeMap<String, u32>` built from item inventories; `ProductionSample` reads `force.item_production_statistics` and never `fluid_production_statistics`. **There is no `get_fluid_contents` call anywhere in `mods/BotBridge/control.lua`.** So a run cannot prove it has any crude oil. | Item-with-a-carrier: add `fluids: BTreeMap<String, f64>` to `MachineSample` and a `machine_row` branch. Straightforward. |
| flow graph | `crates/core/src/types.rs:1785` | `EntityType::is_fluid_input()` covers `Pipe \| StorageTank \| PipeToGround \| Boiler`. A **refinery is `assembling-machine` and a pumpjack is `mining-drill`**, so neither is a fluid node; an oil chain would not connect in `flow_graph`. | Item-with-a-carrier: extend the predicate, per-entity rather than per-type. |
| fluid geometry data | `types.rs:1215-1277`, `mods/BotBridge/types.lua:169-192` | `FactorioFluidBoxPrototype`, `pipe_connections`, `fluidbox_prototypes`, `mining_fluid`, `resource_category` are **already captured and serialised**, and the planner reads none of them. But `FactorioFluidBoxConnection` has **no `direction` field**, and `method/power.rs:217-247` documents at length that the captured `positions` disagree with reality by one tile per connection, which is why the power plant's pipe offsets are three hand-written tuples. | Partly there. A refinery (1 fluid in, 3 out, direction-dependent) will hit exactly the wall `power.rs` documents. Expect either a new hand-written offset table or a `direction` on the connection type. |

**What does *not* break, and is worth knowing.** The entity graph already
charts crude oil generically: `resources: DashMap<String, BTreeMap<Pos, ...>>`
(`entity_graph.rs:191`) is keyed by name with no ore whitelist, and
`resource_patches("crude-oil")` is the documented example at `:121`. The
`entity_tree` whitelist already admits `Pipe, PipeToGround, StorageTank,
MiningDrill, AssemblingMachine`. The only ore-only filter in the codebase is
`RUNG_1_ORES = ["iron-ore","copper-ore","coal","stone"]`
(`crates/planner/src/score.rs:79`), and it is **reporting only** — it decides
what `score-map` prints, not what the graph holds.

---

## 3. What the executor and the mod cannot do today

The executor's verb set is nine `ActionKind`s
(`crates/planner/src/action.rs:599`), dispatched at
`crates/executor/src/run.rs:1239`: `Mine, Chop, Craft, Place, Insert, Remove,
Research, SetRecipe, Evacuate` (+ `walk`, emitted by the scheduler to satisfy
`AtPosition`). The mod exposes 33 remote functions
(`mods/BotBridge/control.lua:6426`) and exactly four `action_start_*`:
walk_waypoints, mining, crafting, research.

**Placing a pumpjack on a crude-oil patch: the machinery is already there.**
This was the surprise of the survey.

- The mod places with `build_check_type = defines.build_check_type.manual`
  (`control.lua:3910`), which is what lets a mining drill sit on a resource;
  resources collide only on the `resource` layer. **Nothing in the mod filters
  by resource at all**, so it would already accept a correctly-positioned
  pumpjack.
- `PlanState::stands_on_resources` (`state.rs:2455`) returns true for
  `entity_type == "mining-drill"`, and **a pumpjack is a `mining-drill`**
  (`entity/mining-drill.lua`, `resource_categories = {"basic-fluid"}`). Its own
  doc comment already names the pumpjack as an intended member.
- What is missing is a *method that sites one*. `crates/planner/src/method/extract.rs:51`
  — `Extract::applicable` returns `false` unconditionally; its module doc line
  10 says "No method sites a pumpjack yet". `Goal::Extracted` (`goal.rs:156`)
  exists precisely for this and is documented as fluid-motivated.
- A pumpjack is 3×3, `energy_usage = "90kW"`, `mining_speed = 1`, electric.
  `cell_yield` (`produce.rs:506`) assumes the mining area equals the footprint,
  which is a burner-drill assumption; a well is a single resource entity and
  crude oil is `infinite = true, minimum = 60000`, so yield arithmetic that
  counts a depleting `amount` does not apply. `control.lua:2621` already says
  so: "An infinite resource (crude oil) never decreases below its minimum
  yield, so a pumpjack's output is not countable this way at all."

**Missing verbs and capabilities, as a task list:**

1. **Chart the ground.** No mod function calls `force.chart` or freeplay's
   `set_chart_distance`, and `OutputParser` has **no `"resources"` branch** —
   the mod writes a `resources` line and Rust discards it
   (`output_reader.rs:91`), so resources reach the model only through the
   generic chunk-entity stream. Nothing in this system ever explores.
2. **Site a pumpjack** (`Extract` method): choose a well tile, snap to build
   grid, check `AreaFree` with the resource exemption, require `Powered`.
3. **Fluid link geometry**: a `connects_fluid(a, b)` predicate, plus offset
   tables for pumpjack output, storage-tank connections, refinery in/out and
   chemical-plant in/out. `power.rs:259-278` is the precedent and the warning.
4. **Pipe runs**: `Place` of `pipe` / `pipe-to-ground` works today as a bare
   entity placement — the only prototype-specific branch in the mod is
   `underground-belt`'s `type` field (`control.lua:4031`), and `pipe-to-ground`
   correctly needs none. What is missing is a *router*: `method/connect.rs`
   routes belts via `route_belt` and has no fluid sibling.
5. **A refinery's three outputs**: `set_recipe` already accepts a refinery —
   the mod's guard is `entity.type ~= "assembling-machine"`
   (`control.lua:4699`) and an `oil-refinery` and `chemical-plant` are both
   assembling machines. **But** the eviction path (`control.lua:4721-4747`)
   inserts what `set_recipe` returned into the player, and a recipe change on a
   refinery can evict *fluids*, which `LuaPlayer.insert` cannot take. That path
   is untested and would report a failure. Also `enabled` is checked
   (`:4685`), so the recipe must be researched first.
6. **Cracking**: nothing beyond items 3–5; `heavy-oil-cracking` and
   `light-oil-cracking` are chemistry recipes with two fluid ingredients and one
   fluid product, so they are the *same* problem as `sulfur`.
7. **A storage tank**: `Place` works; `StorageTank` is already an
   `EntityType` and already in the `entity_tree` whitelist and already
   `is_fluid_input`. What is missing is only the fluid link and the telemetry.
8. **Fluid levels in samples**: add a `storage-tank` entry to `MACHINE_TYPES`
   (`control.lua:1972` — the comment there deliberately excludes pipes as too
   numerous, and `MACHINE_SAMPLE_LIMIT = 400` at `:2027` is why), a
   `get_fluid_contents()` branch in `machine_row` (`control.lua:2736`, guarded
   — a bad attribute read in a sampler has taken a live run down before), and
   `fluids: BTreeMap<String, f64>` on `MachineSample` (`samples.rs:223`) behind
   `#[serde(default)]`. The OpenAPI snapshot seam
   (`app/src/api/openapi.snapshot.json`, `crates/server/tests/openapi.rs`) will
   fail until regenerated — by design.

---

## 4. The rungs, ordered

Each rung is independently testable and each has a refusal that moves when it
lands. The ordering is forced by the fact that **the planner's current refusal
for oil is a charting refusal, not a modelling refusal** (§5).

**Rung O1 — chart crude oil.** A mod verb that charts a radius (or freeplay's
`set_chart_distance` plus `force.chart`), and an `OutputParser` branch (or the
existing entity stream) that lands the result in `EntityGraph::resources`.
*Test:* a dump taken after charting contains `entity_graph.resources["crude-oil"]`,
and `score-map` reports a crude-oil row. No planner change.

**Rung O2 — report oil in the survey.** Add crude oil to `score.rs` — either
by growing `RUNG_1_ORES` or, better, a separate oil section, since a missing
oil patch is not a rung-1 `NotViable`. *Test:* `score-map` on a charted dump
prints distance and well count; on the current dump it says so and does not
lie.

**Rung O3 — site and place a pumpjack (`Goal::Extracted`).** Implement
`Extract`. Needs power at the field (`Condition::Powered` and the existing
`method/power.rs` plant, or poles run out). *Test:* `plan --goal-json
'{"Extracted":{"entity":"crude-oil","unlocks":"oil-processing"}}'` against a
charted dump returns a plan containing a `place pumpjack` step at a well tile;
live, the game accepts the placement and `oil-processing` unlocks by its own
native trigger with no emulation.

**Rung O4 — a fluid sink: pipe the pumpjack into a storage tank.** The fluid
link predicate plus the two offset tables. *Test:* a live headless run ends
with a storage tank whose fluidbox is non-empty.

**Rung O5 — fluid telemetry.** The `get_fluid_contents` path end to end.
*Test:* `samples.jsonl` from that run carries a rising `crude-oil` figure for
the tank, and `just analyse` shows it. **Until O5 lands, O4 cannot be proved
from the record** — only by an ad-hoc `rcon` query.

**Rung O6 — fluid as a planning quantity.** `ingredients_of` stops discarding
`ingredient_type`; a product→recipe index; a fluid-shaped goal
(`Goal::Stored` or similar) with its own `available`. *Test:* `plan --goal
have:plastic-bar:2` stops answering "no method can satisfy" and starts
answering with a refinery-and-chemical-plant plan or a *specific* refusal.

**Rung O7 — refinery and chemistry cells.** `set_recipe` on a refinery
including the fluid-eviction path; a chemical plant fed by pipes and inserters;
then plastic → advanced circuit → electric furnace, and sulfur → sulfuric acid
→ battery.

**The smallest thing that would put crude oil in a tank on seed 31337** is
O1+O3+O4 — three rungs, of which O1 is the only one with no prior art in the
codebase and O3 is the only one that touches the planner's method registry.
O2 and O5 are the *observability* of O1 and O4 and should not be skipped: this
repo's record has four separate instances of a mechanism reporting nothing
while broken.

Note that O1 through O5 need **no fluid concept in the planner at all**. A
pumpjack is placed like a mining drill, a pipe and a tank are placed like any
entity, and nothing has to be counted. That is why the first rung is small.

---

## 5. What the map gives us — and it gives us no oil

`score-map --world workspace/scripts/map.json --bots 1,2,3,4`, seed 31337,
fingerprint `c161fa3f437221d0`:

```
iron-ore   18.4    copper-ore 54.9    coal 32.1    stone 33.3    water 48.1
walk score 1337 ticks    charting 17/17 probes on charted ground
charted resources span x -80..224, y -210..122       verdict VIABLE for rung 1
```

The dump's `entity_graph.resources` map contains **exactly four keys**: `coal`,
`copper-ore`, `iron-ore`, `stone`. There is **no crude-oil resource entity in
the dump at all**. The 46 occurrences of the string `crude-oil` in the 826 MB
file are all prototype and recipe definitions (`crude-oil-barrel`,
`empty-crude-oil-barrel`, the `crude-oil` resource *prototype*, the barrel
recipes) — none is a placed entity.

The planner already says this, well:

```
$ factorio-bot plan --world scripts/map.json --goal researched:oil-processing
Error: the goal did not expand: no crude-oil is charted anywhere this plan can
  see, so crude-oil has nowhere to come from; the plan sees coal (466 tiles),
  copper-ore (462 tiles), iron-ore (940 tiles), stone (387 tiles); charted
  ground covers 17 of 17 probes within 256 tiles of [0, 0], so the uncharted
  ground is beyond that radius
```

**What I could not establish offline, and the query that settles it:** whether
crude oil exists just outside the charted disc on seed 31337, and at what
distance. A dump measures what has been *seen*, and the bots never explore. The
question needs a running seed-31337 server and one read-only RCON call:

```
factorio-bot rcon -s localhost -- '/c local s=game.surfaces[1]
  s.request_to_generate_chunks({0,0}, 20) s.force_generate_chunk_requests()
  local e = s.find_entities_filtered{name="crude-oil", position={0,0}, radius=640}
  local best = 1e9 for _,x in pairs(e) do
    local d = math.sqrt(x.position.x^2 + x.position.y^2) if d < best then best = d end end
  rcon.print(#e .. " wells, nearest " .. string.format("%.1f", best))'
```

(`request_to_generate_chunks` mutates a live world by generating chunks, so run
it on a savepoint-resumed world or a throwaway server, never during a measured
run.)

**What the absence implies for rung ordering.** It forces charting to be rung
one. Not because charting is interesting, but because every downstream rung is
untestable without it: `Extract` cannot be exercised offline against a dump
that has no well, and a live run cannot place a pumpjack on ground the bots
have never looked at. It also means the walk cost of oil is **currently
unknown** — vanilla map generation excludes oil from the starting area, so the
nearest well is very likely outside the 256-tile probe disc, and that walk is a
cost this project has never paid. `score-map`'s `walk score` of 1337 ticks
covers rung-1 ore only.

The mod already has the lever: CLAUDE.md records that freeplay exposes
`set_chart_distance`, "relevant to exploration, which this mod otherwise never
does."

---

## 6. Steel

`steel-plate` = 5 `iron-plate`, category `smelting`, `energy_required = 16`.
A `stone-furnace` has `crafting_speed = 1` and `energy_usage = "90kW"`, so:

| | iron ore | furnace-ticks |
|---|---|---|
| 1 iron-plate | 1 | 192 (3.2 s) |
| 1 steel-plate | 5 | 5×192 + 960 = **1,920** |
| 10 steel (an electric furnace) | 50 | **19,200** (5:20 of one furnace) |
| 5 steel (a solar panel) | 25 | 9,600 |
| 15 steel (an oil refinery) | 75 | 28,800 |

**The current smelting model reaches steel with no new concepts.** `steel-plate`
is a one-ingredient, `amount = 5`, `smelting` recipe, so `Smelt`
(`have.rs:1108`) handles it: place furnace, `Insert` into `FurnaceSource`,
`Insert` coal into `Fuel`, `Remove` from `FurnaceResult`. It is proved offline:

```
researched:steel-processing   273 actions,  30,659 ticks (0:08:30)
have:steel-plate:10           329 actions,  36,926 ticks (0:10:15)
```

One caveat: `cell_spec` (`produce.rs:180`) refuses any recipe whose single
ingredient has `amount != 1`, so **steel cannot be produced by a stage-1
production cell** — a `Producing{item:"steel-plate"}` goal has no cell to build
and falls back to `Smelt`'s hand-fed furnaces. Steel is reachable, steel *at a
rate* is not. That is a pre-existing gap and it is on the path to solar panels,
which need 5 steel each.

---

## An incidental defect found while surveying

**Goals that plan for one or two bots refuse for three or four.** Every plan
below is the same goal against the same dump:

| goal | 1 bot | 2 bots | 3 bots | 4 bots |
|---|---|---|---|---|
| `have:pumpjack:1` | 776 acts, 3:27:16 | 986 acts, 2:07:24 | **refused** | **refused** |
| `researched:engine` | 343 acts, 1:26:02 | — | — | **refused** |

The refusal is `no method can satisfy goal: have 6 iron-ore (a share sized for
bot 3)` — i.e. `SplitAcrossBots` sizes a share and then nothing can satisfy
that share, although the whole goal is satisfiable. `researched:automation-2`,
`researched:fluid-handling`, `researched:oil-gathering`, `have:storage-tank:1`
all fail the same way at four bots and are the *default* roster. This is not an
oil problem — it will block any deep goal — and it is reproducible in four
seconds offline. It is not in scope for this survey and I have not diagnosed
it.

The 2-bot `have:pumpjack:1` plan is otherwise correct and encouraging: it
schedules exactly the seven researches derived in §1 —

```
automation, logistic-science-pack, steel-processing,
automation-2, engine, fluid-handling, oil-gathering
```

— and ends holding a pumpjack. **The planner can already reach the pumpjack
item.** It cannot put it anywhere.

---

## Recommended first dispatch

**Chart the ground, so crude oil enters the world model.** One task: add a mod
verb that charts a radius around spawn (`force.chart`, or freeplay's
`set_chart_distance` followed by a chart call), expose it to Lua, and make sure
the resulting resource entities reach `EntityGraph::resources` — plus a
`crude-oil` row in `score-map` so the result is visible. No planner method, no
fluid concept, no new `ActionKind`.

*Acceptance test, as a sentence someone could check:* **after a headless
seed-31337 run calls the new charting verb and then `world.dump`s, running
`factorio-bot score-map` on that dump prints a `crude-oil` row with a finite
distance and a well count, and `factorio-bot plan --goal
researched:oil-processing` against the same dump no longer refuses with "no
crude-oil is charted anywhere this plan can see" but with
`ExtractionNotModelled` instead** — the refusal moving one rung down the ladder
is the proof that the rung landed.

Two things I could not settle without the game, restated so they are not lost:
whether seed 31337 has oil within a reasonable walk (the RCON query is in §5),
and whether the mod's `set_recipe` eviction path survives a refinery whose
fluids get evicted (`control.lua:4721`) — the query for that is a savepoint
world, a placed refinery with `basic-oil-processing` running, and a
`set_recipe` to something else.

---

## The roster refusal

Diagnosed and fixed 2026-09-06, on branch `shares-refuse-with-more-bots`. The
survey above found it ("An incidental defect found while surveying") and did
not diagnose it; this section is the answer, written here because this is
where it was found.

**It is wrong, not badly worded.** A plan existed and the planner failed to
find it, so a better message would have been a better-worded mistake.

### The mechanism

Not the share arithmetic. `even_shares` divides a shortfall evenly, and a
share of six is not harder to satisfy than a share of nine — it is *later*.
What has run out by then is the tile ledger.

A mining claim spent a **whole tile** (`PlanState::claimed`, read through
`resource_unclaimed_for` in `crates/planner/src/state.rs`), permanently,
whoever made it and however little it took; and `mining_tile_separation` then
crowds every tile within 3.69 of that claim out for every *other* runner.
Shares are per-bot, so a roster of `n` burns `n` tiles per shortfall where two
bots burn two — and a deep goal has hundreds of shortfalls. Instrumented at
the exact refusal of `have:pumpjack:1` at three bots, on this dump's charted
iron field:

```
tiles=940  claimed=324  crowded=616  free=0
physical=522,467 ore    of which 134,734 sat on the 324 claimed tiles
need=6
```

Half a million ore in the ground, six wanted, none offered. The four-bot run
reached the same wall one bot later (412 claimed, 528 crowded).

### The fix

`PlanState::claim_yields_to`: **where the world states what a tile holds, the
runner that already claimed it may draw from it again**, against what is left
(`resource_available` = the game's `resource_amount` less `consumed`). Every
other runner is still refused the tile, and crowding is untouched.

Two reasons it is the right relaxation rather than a loosened bound:

* `MiningClaim` already makes exactly this argument for crowding — a bot runs
  one action at a time, an owned chain is offered to one bot and an unowned
  one binds to a single bot at its first action — so two draws by one runner
  are provably disjoint in time whatever the schedule turns out to be.
* Whole-tile exclusivity's own stated reason was that the planner "cannot know
  what a tile really holds", `DEFAULT_RESOURCE_PER_TILE` standing in for a
  reading nobody took. Where the world *does* state the amount that reason is
  absent — and where it does not, the old rule stands verbatim, which is why
  every hand-built fixture behaves exactly as before.

The defect whole-tile exclusivity was introduced for — four *different* bots
sent to one tile, and the game reporting `the target stone was gone before
mining finished` — is untouched: the relaxation is same-runner only.

### Before and after, `factorio-bot plan` against this dump

| goal | roster | before | after |
|---|---|---|---|
| `have:pumpjack:1` | 1 | 776 acts, 746,162 (3:27:16) | 707 acts, 753,322 (3:29:15) |
| | 2 | 986 acts, 458,660 (2:07:24) | 969 acts, 460,480 (2:07:54) |
| | 3 | **refused**, 6 iron-ore | **1,601 acts, 335,805 (1:33:16)** |
| | 4 | **refused**, 6 iron-ore | **refused**, 17 iron-plate — different cause, below |
| `researched:automation` | 4 | 176 acts, 21,776 | 176 acts, 21,784 |
| `producing:automation-science-pack:6` | 4 | 324 acts, 26,990 | 324 acts, **22,547** |
| `producing:logistic-science-pack:6` | 4 | 569 acts, 52,819 | 570 acts, **52,224** |
| `researched:engine` | 4 | **refused**, 14 iron-plate | **1,206 acts, 126,590 (0:35:09)** |
| `researched:automation-2` | 4 | **refused**, 20 iron-plate | **904 acts, 93,675 (0:26:01)** |
| `researched:fluid-handling` | 4 | **refused**, 7 iron-ore | **1,843 acts, 205,042 (0:56:57)** |
| `have:storage-tank:1` | 4 | **refused**, 8 iron-ore | **1,743 acts, 205,836 (0:57:10)** |
| `researched:oil-gathering` | 4 | **refused**, 27 iron-ore | **refused**, 24 iron-plate — same second cause |

Four of the survey's five goals plan at the default roster where none did.
Nothing regressed except `researched:automation` by **8 ticks** (21,776 →
21,784, +0.04%, same 176 actions: two fewer walks for bots 1 and 2, one more
each for 3 and 4), and the one-bot pumpjack by 7,160 ticks on 69 *fewer*
actions. Both are stated rather than rounded away.

### The second cliff, found and NOT fixed

`have:pumpjack:1` at four bots and `researched:oil-gathering` at four still
refuse — one layer deeper, on `have N iron-plate`, and for an unrelated
reason. Evidence: with the method registry instrumented, `MethodRegistry::find`
never returns `None` for that goal, so the error comes from *inside* a
method's `expand`; in `smelt_steps` (`crates/planner/src/method/have.rs`) the
only reachable `NoApplicableMethod` for iron-plate is the
`free_area_near(&trial, &anchor, &furnace_entity).ok_or_else(...)` site — **no
free ground for another furnace** within `FREE_TILE_SEARCH_RADIUS` (12 tiles,
`crates/planner/src/method/util.rs`).

The three-bot pumpjack plan that now succeeds **places 75 stone furnaces**.
That is the real finding: a smelt builds its own furnace rather than queueing
behind one, so furnaces scale with the bill and with the roster, and the
12-tile ring around the anchor fills. Raising the radius is not the fix —
`method::power::PLANT_ADOPT_RADIUS` is derived from it — reuse is. It wants
its own task.

Eight bots was not settled: the expansion runs for many minutes at that
roster (before the fix it refused in seconds on `have 5 copper-ore`), which is
itself worth a look — expansion cost grows with the number of shares.
