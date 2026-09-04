# Exploration: mechanisms, legitimacy, and what the world model already knows

2026-09-04. Written against `9ff680f0`, measured off `workspace/runs/*` and
`workspace/server-log.txt` from the run that finished at 19:15 the same day
(`run-1788538389-09170`, seed 31337). No Factorio was started for this document;
everything below is read off the archive or off the source.

---

## Summary: two of the three premises this work started from are wrong

The brief for this design said three things. The measurements say otherwise
about two of them, and the correction changes what should be built.

**1. "The planner cannot see crude oil on our maps." — False on this
workspace.** `provenance.json` for `run-1788538389-09170` records the resource
census `EntityGraph` held at run start:

```json
"tiles": { "coal": 1887, "copper-ore": 2435, "crude-oil": 12,
           "iron-ore": 3658, "stone": 1560, "uranium-ore": 559 }
```

Twelve crude-oil wells and 559 uranium tiles. The four-ore census the brief
quotes (`coal, copper-ore, iron-ore, stone`) is what a **fresh** workspace
holds; every archived run that started on a fresh map shows exactly it, and the
one run that resumed from a savepoint shows the list above. Charting grows, and
`ResourceFingerprint`'s own doc already said so.

**2. "Nothing in this system ever asks the game to reveal new ground, so oil is
invisible." — The first half is true and the second does not follow.** The
world model learns about ground continuously and needs no change to do so. What
it *does* need is a reason to distrust what it learns, because:

> **The world model already knows about ground no bot has ever been near, and
> nothing records that it does.** In `run-1788532631-48030` the furthest any bot
> reached was **63.8 tiles** from spawn (max `|position|` over every event in
> `events.jsonl`). The world model that run handed to its successor contains
> two crude-oil patches at **380** and **505** tiles, 559 uranium tiles, and
> **36 biter spawners and 28 worm turrets** out to 500 tiles.

That is the real finding. Exploration is not blocked by a missing mechanism; it
is *masked* by an existing one that is more generous than any player could be.

**3. "`plan --goal researched:oil-processing` cannot even begin [because there
is no crude oil]." — It cannot begin, but not for that reason.** From the live
technology dump:

```json
{"name": "oil-processing", "research_unit_count": 1, "research_unit_energy": 0,
 "research_unit_ingredients": {}, "prerequisites": ["oil-gathering"],
 "research_trigger": {"type": "mine-entity"}}
```

`oil-processing` is a **trigger technology**: it costs no science and completes
when you mine a crude-oil well. `mods/BotBridge/types.lua:265-291` deliberately
sends `mine-entity` triggers **without a payload** (the shipped prototypes write
`entities = {...}` where `runtime-api.json` documents a singular `entity`, so
the mod refuses to guess). The planner therefore raises
`PlannerError::UnsupportedResearchTrigger` — a refusal that happens whether or
not a single oil well is charted. **Charting the map does not unblock that
goal.** Fixing the trigger payload does, and that is a different, smaller task.

For completeness, the real oil ladder on 2.1.17, from the same dump:

| technology | cost | prerequisite | unlocks |
|---|---|---|---|
| `logistic-science-pack` | 75 × red | `automation-science-pack` ✅ | green science |
| `automation-2`, `engine` | red + green | | |
| `fluid-handling` | 50 × red+green | `automation-2`, `engine` | pipes to ground, storage tank |
| `oil-gathering` | 100 × red+green | `fluid-handling` | **pumpjack** |
| `oil-processing` | **trigger: mine-entity** | `oil-gathering` | refinery, chemical plant, basic oil processing |

Everything up to and including `oil-gathering` is pure science and needs no oil
at all. Oil is needed for exactly one step — putting a pumpjack on a well and
mining it — and that step needs a charted well.

---

## The measurements

### Charting grew without anyone exploring

`provenance.json` across the archive (`map.tiles`, recorded at run start):

| run | seed | start tick | resumed from | census |
|---|---|---|---|---|
| 9 runs, 1788479942 → 1788504490 | unknown | 3.4k–6.9k | — | coal 523, copper 426, iron 533, stone 434 |
| 1788509918 / 13716 / 28493 | 31337 | ~3.5k | — | coal 466, copper 462, iron 940, stone 387 |
| 1788532631 | 31337 | 3658 | — | coal 466, copper 468, iron 940, stone 387 |
| **1788538389** | 31337 | **75915** | `1788532631/milestone-2` | coal 1887, copper 2435, **crude-oil 12**, iron 3658, stone 1560, **uranium 559** |

The last row is the same map as the row above it, 67,000 ticks later, with
4–5× the resource tiles and two resource kinds that did not exist before.

### Where the oil is (seed 31337)

Extracted from the `§tick§entities§` lines of `workspace/server-log.txt`, which
is the mod's own writeout:

| patch | wells | centroid | distance from spawn | amounts |
|---|---|---|---|---|
| **B (nearest)** | 7 | (144.8, −351.8) | **380** | 307k–982k |
| A | 5 | (−502.5, −49.1) | **505** | 356k–567k |

Patch A's westernmost well is at `x = −509.5`, **2.5 tiles inside the mod's
±512 wall.** Whether that patch continues past `x = −512` is unknowable from
the archive, and if it does, those wells can never enter the model.

### Where the nests are (seed 31337)

Same source. The mod has always written enemy entities out —
`writeout_entities` filters only `ent.type ~= "character"`.

* **36 unique `*-spawner`**, **28 unique `*-worm-turret`**, plus wandering units.
* **Nearest enemy structure to spawn: `biter-spawner` at (−237.5, 66.5), 246.6
  tiles.** No nest is charted inside ~246 tiles of origin.
* Nearest enemy structure to oil patch B: `spitter-spawner` at (43.5, −283.5),
  **122 tiles** from the patch and **≈66 tiles off the straight line from spawn
  to it**.
* Nearest enemy structure to oil patch A: `spitter-spawner` at (−487.5, −94.5),
  **48 tiles** from the patch.

So the nearest oil is outside the nest-free zone, and the direct route to it
passes within ~66 tiles of a nest cluster.

---

## Q1: does walking already chart? Yes — and the mod ingests more than walking earns

The chain is continuous and already works:

```
bot walks  →  engine generates + charts the chunk
           →  on_chunk_generated            (control.lua:1576)
           →  writeout_entities             (control.lua:2579)  every entity in the chunk
           →  §tick§entities§ on stdout     (control.lua:2619)
           →  OutputParser "entities" arm   (core/src/process/output_parser.rs:28-46)
           →  FactorioWorld::update_chunk_entities  (core/src/factorio/world.rs:976)
           →  EntityGraph::add              (core/src/graph/entity_graph.rs:970)
```

There is no kind filter anywhere on that path — `crude-oil`'s entity type is
`resource`, `serialize_entity` sends its `amount`, and `EntityGraph::add`
branches on the string `"resource"`. The `RUNG_1_ORES` array in
`crates/planner/src/score.rs:79` is a scoring convenience, not an ingest
filter.

**But the mod hooks generation, not charting.** `on_chunk_generated` fires when
the *engine* creates a chunk, for whatever reason — and the force's charted
area is never consulted. `force.is_chunk_charted` and the
`on_chunk_charted` event both exist in 2.1.17 and appear zero times in this
repo. The consequence is the 63.8-tiles-vs-505-tiles gap above.

**What generated ground 500 tiles from four bots that never left 64?** I could
not determine this from the archive and it needs a run to settle. What I can
report:

* The resumed run generated **zero** new chunks in 225,205 ticks — its 1,024
  entity writeouts are all from the `initial_discovery` replay, one per tick at
  ticks 71,196–72,219, and the client logs contain no entity writeouts at all.
  So generation happened during an *earlier* run, whose logs are overwritten.
* The generated region is bounded, and the bound is suspiciously exact. The mod
  reports 1,024 chunks — precisely the 32×32 grid the ±512 clamp admits — and
  `client1/script-output/tiles/` holds 828 non-void PNGs on the 33×33 grid from
  −512 to +512 in steps of 32, i.e. terrain exists out to at least `x = 544`.
* **The only ±512 loop in the codebase is `on_player_joined_game`**
  (`control.lua:2667-2677`), which on client1 screenshots every chunk from −512
  to +512 in steps of 32, then again in steps of 256. Its extent matches the
  generated extent exactly. CLAUDE.md records that per-camera screenshots were
  "retired on 2026-09-02"; the *cadence* was, this join-time sweep was not.

  I am not able to confirm from documentation that `game.take_screenshot`
  forces chunk generation — chunk generation is game state and must be
  deterministic across peers, which argues against it. The correlation is what
  it is, and the one-run test that settles it is at the end of this document.

Either way the conclusion for the design is unchanged and does not depend on
the cause: **the model is fed by generation, which is not something a bot
earned.**

---

## Q2: can exploration be reactive? Yes, with no world-model change at all

`PlanState` does not copy the world. `crates/planner/src/state.rs:666` holds
`base: Arc<FactorioWorld>` and every resource read goes through it at query
time — `resource_available` (:3079), `resource_patches` (:3662),
`has_resource_patches` (:3679) all reach the live `DashMap`. `Planner`'s
`plan_world` is the same `Arc` as `real_world`
(`crates/core/src/plan/planner.rs:123`), and the Lua bindings get that same
handle (`crates/scripting_lua/src/lua_runner.rs:139`); both were deep copies
once and were deliberately un-copied.

**A resource tile written at tick 100,000 is visible to a plan expanded at tick
100,001** — in fact to an expansion already in flight. Only eight overlay fields
(bot positions, inventories, refusals, buffers) are frozen per plan.

The one exception is the offline `plan` CLI
(`app/src-tauri/src/cli/plan.rs:221`), which deserialises a `world.dump()`
file. That is frozen by construction.

**So "walk somewhere, then re-plan" works today.** Exploration is a *planning*
problem, not an ingestion problem.

---

## Q3: the ±512 drop

`control.lua:1590-1593`:

```lua
if chunk_x < -512 then return end
if chunk_y < -512 then return end
if chunk_xend > 512 then return end
if chunk_yend > 512 then return end
```

* **Origin: the first commit that ever added the mod** (`850f8c19`, 2021-08-04).
  No rationale was recorded then and none has been added since.
* **It drops everything, not just tiles.** The `return` is before
  `writeout_entities`, before `writeout_tiles`, and before `storage.map_area` is
  widened. A chunk outside the box contributes nothing at all.
* **It admits exactly 1,024 chunks** — `left_top` from −512 to 480 — which is
  the 1,024 the archive shows, so on this workspace the clamp is the binding
  constraint on the model's extent, not the game's.
* **Yes, it bounds where oil can ever be found**: a 1,024×1,024-tile box. Patch
  A already touches the wall. Default map generation puts oil well outside that
  box on plenty of seeds.
* **It is recoverable.** `on_chunk_generated` never fires twice for a chunk, so
  widening the clamp does nothing to a live game — but the `initial_discovery`
  replay (`control.lua:1074-1104`) walks `surface.get_chunks()` on every server
  boot, so a **restart or a `--resume-from`** re-emits every chunk under the new
  bound. No data is permanently lost by having had the clamp; it is lost only
  while the clamp is in place.

**Recommendation.** Widen it, but do not remove it: it is the only thing
bounding the cost of the discovery replay (one chunk per tick, so 1,024 chunks
is ~17 seconds of game time, and a 2,048-tile box would be ~68 seconds). Make
the bound a named constant with the reason attached, and put the value into
`provenance.json` so a census is never read as a map fact again.

---

## The mechanisms, and whether each is legitimate in a measured run

The rule this is judged against: *final measured runs must be as cheat-free as
we can get them; cheating is fine while developing and must be recorded in
provenance.*

| mechanism | what it does | cost | legitimate in a measured run? |
|---|---|---|---|
| **A bot walks there** | Engine generates and charts as the character moves. Mod ingests for free. | Bot-time. ~380 tiles at ~9 tiles/s ≈ 42 s each way, plus the risk below. | **Yes, unreservedly.** This is what a speedrunner does. It is the reference mechanism, and the honest price is exactly the price. |
| **Radar** | Charts a 3×3-chunk area continuously and slowly sweeps a much larger one, from where it stands. | `radar` technology: **20 × red science, 600 energy** — prerequisite `automation-science-pack`, which we already reach. Recipe: 10 iron-plate, 5 iron-gear-wheel, 5 electronic-circuit. Needs **300 kW** of electric supply (`state.rs:260` already knows the figure). | **Yes.** A real building, a real research, a real power draw, all paid in-game. Cheap — cheaper than the brief assumed. See the argument below for why it may be *preferable* to walking. |
| **`force.chart(area)`** | Reveals arbitrary map with nobody there. | Free. | **No.** This is the god-mode reveal a player cannot perform. Development only, and any run that used it must say so in `provenance.json`. |
| **`freeplay.set_chart_distance`** | Sets `storage.chart_distance` (default **200**), which `chart_starting_area` uses **once**, at scenario init, to `force.chart` a `2r × 2r` box around spawn (`data/base/script/freeplay/freeplay.lua:42-47, 170`). | Free, but one-shot and before the run. | **A scenario setting, not a tool.** It cannot reveal anything after the game has started, so it cannot be used to find oil reactively; raising it enlarges the starting reveal. Treat it exactly like `starting_area` in `map-gen-settings`: a property of the map the run was launched on, legitimate if declared, and it belongs in `provenance.json`. Not a substitute for exploration. |
| **`on_chunk_generated` ingest (what we do today)** | Records every entity of every chunk the engine generates, charted or not. | Free. | **No — and this is the one nobody had counted.** It is `force.chart` with extra steps: the model gains complete knowledge of ground the force has never seen, including ore, water and nests. On the measured workspace it is worth 380–505 tiles of vision against 64 tiles of actual travel. |

### The recommendation on the last row

Do **not** rip it out. Do this instead, in order:

1. **Measure it and record it.** Add the charted-vs-known comparison to
   `provenance.json`: the force's actual charted area alongside the model's
   census. A run whose model extends past what the force charted is a run whose
   number carries an asterisk, and today nothing prints that asterisk.
2. **Then make it optional**, defaulting to the current behaviour so no run
   changes silently. A mod-side `force.is_chunk_charted(surface, chunk)` guard
   in `on_chunk_generated`, plus an `on_chunk_charted` handler so ground the
   force charts later is ingested when it is charted rather than never.
3. **Only then flip the default**, once exploration can actually supply what
   the guard removes. Flipping it first reduces the world model to the ~418
   starting chunks and breaks every current milestone.

Step 2 is where the "ingest charted-but-not-generated chunks" idea in the brief
actually lands: the gap is the reverse of what was assumed. Everything
generated is ingested; nothing is ingested *because* it was charted.

---

## Enemies: what happens when a bot meets something that fights back

### What is modelled: almost nothing, and one thing that now is

Before this document, nothing in the codebase modelled enemies. The mod sends
them — spawners as `entity_type: "unit-spawner"`, worms as `"turret"`, biters as
`"unit"` — and `EntityType` (`crates/core/src/types.rs:1397`) has no variant for
any of the three, so `EntityType::from_str` returned `Err` and
`EntityGraph::add`'s whitelist was never consulted. All that survived was one
anonymous rectangle in `blocked_tree`, which can say *something is in the way*
and cannot say what, whose, or where its centre is.

**This is fixed** (see "What was implemented"). `EntityGraph` now keeps a
`threats` map — nests and worms by name and position — with `nearest_threat`,
`threats_from` and `threat_census`. Live biters are deliberately excluded: a
unit walks, a chunk is written out once, so a recorded biter is a permanent
phantom.

### What is not modelled, and should be said plainly

* **Nothing notices a bot has died.** The roster is computed once, at script
  start (`crates/scripting_lua/src/lua_runner.rs:130` →
  `Planner::roster`, `crates/core/src/plan/planner.rs:277`), from
  `world.players`. It is never recomputed. On the mod side, a dead player keeps
  its `LuaPlayer` and loses its `character`, and
  `start_walk_waypoints` (`control.lua:3078-3081`) returns a bare `false` for
  `character == nil` — indistinguishable from "not connected" and from "no such
  player". So a killed bot degrades into an ordinary stream of action failures
  with no distinguishing kind, and `just analyse` has no concept of it.
  **This is exactly the silent-roster-degradation failure this project has
  already paid for once**, and it would be a new instance of it.
* **Nothing avoids a nest when routing.** Walks go through the game's own
  pathfinder, which routes around collision, not around danger.

### The four questions, answered

**1. Does a legitimate route need to avoid nests, and how would it know?**
Partly, and less reactively than feared. Nests are visible in the model *before*
a bot goes anywhere, because charting today runs ahead of travel — and after
the honest-ingest change of step 3 above, they would be visible for exactly the
ground already charted, which is the ground behind the frontier. The right shape
is therefore **incremental**: chart a little, re-read `nearest_threat`, extend.
That is available today; the frontier is never more than one leg ahead of what
is known. Reactive turn-back is a fallback, not the design.

**2. What happens when a bot dies?** Established above: nothing. **Before any
run sends a bot past the safe radius, "a bot died" must become a recorded event
that ends or re-plans the run.** This is a prerequisite for exploration, not a
follow-up to it, and it is a milestone of its own — see the decomposition.

**3. Is there a cheap safety bound?** Yes, and it is worth a lot. Default
generation leaves a nest-free starting area; on seed 31337 the nearest charted
enemy structure is **246.6 tiles** from spawn. So:

> **Explore freely inside the charted nest-free radius; treat the distance to
> the nearest charted enemy structure as the point past which a bot is at
> risk.** Compute it, do not assume it: `nearest_threat(spawn)` now answers it
> per map, and an empty answer means *nothing charted*, never *safe*.

The bound is generous but not generous enough — 246 tiles does not reach oil at
380. It does cover the whole `DEFAULT_SEARCH_RADIUS` of 256 that scoring already
uses, so every rung-1 decision is inside it.

**4. Does this change the mechanism choice? Yes — it promotes radar.**
A radar reveals ground with nobody standing in it. That removes the entire
failure mode: no bot is exposed, no roster can degrade, nothing has to be
detected after the fact. Against that it costs 20 red science (which the project
already produces), ~10 iron-plate/5 gears/5 circuits, and 300 kW — and the
project already builds and powers assembling-machine cells, so a 300 kW draw is
a known quantity, not a new capability.

**Recommendation: walk inside the safe radius, radar beyond it.** Walking is the
reference mechanism and is free of research and power; use it where it is free
of risk too. Past the nearest charted nest, the honest comparison is not
"radar's research cost vs. free walking" but "radar's research cost vs. a bot's
life and a silently different plan", and radar wins that comfortably.

---

## Q4: how would a plan ask for this?

Exploration has the shape the idle-work problem has: **it is not demanded by
anything.** Every `Goal` variant is about items, technology, or rates —
`Have`, `Researched`, `Produced`, `Producing`, `All`
(`crates/planner/src/goal.rs:75-134`) — and the registry is first-match-wins
with no backtracking (`method/mod.rs:305`). Nothing asks "where is the oil",
because a goal that needs oil fails before it can ask.

Four facts constrain the answer:

* **No new executor verb is needed.** `Actuator::walk`
  (`crates/executor/src/actuator.rs:144`) exists, and `ActionKind::Evacuate`
  (`crates/planner/src/action.rs:687`) already demonstrates an action whose
  entire payload is a position and whose dispatch is one line
  (`run.rs:1277` → `act.walk(...)`). An exploration action is `Evacuate` with a
  different name and a different reason.
* **Movement is normally implicit.** Methods emit `Condition::AtPosition` and
  the *scheduler* materialises `StepKind::Walk` (`schedule.rs:142`). Walks are
  not network nodes.
* **There is no `Condition` for "this region is charted" and no `Effect` for
  "ground became charted"** (`action.rs:32-219`, `:354-458`), and
  `PlanState` has no charting query at all — `score.rs::probe_charting`
  reaches past it into `entity_graph.tiles_within`.
* **The refusal that exploration should replace is already ambiguous.** An
  absent resource makes `resource_patches` warn and return empty →
  `Mine::applicable` false → `NoApplicableMethod`. The crate's own convention
  is to give an actionable refusal its own variant, and `NoPatchForCell` and
  `PowerPlantNeedsWater` already blame charting by name in their help text.

**The shape I would build**, cheapest first:

1. **A refusal that says "unexplored, not absent", plus where to look.** Lift a
   charting query onto `PlanState` and add a `NotCharted`-style
   `PlannerError` carrying the charted extent and the frontier direction. This
   costs nothing at runtime and turns a dead end into an instruction. It is the
   single highest-value step and it needs no new goal, action, or verb.
2. **A script-level scout, outside the planner.** `goal.plan` already fails with
   a typed error a Lua script can branch on. A supervisor script that catches
   "not charted", picks a frontier point, issues a walk, and re-plans is
   *entirely expressible today* — because of Q2, the re-plan sees the new
   ground. This is the honest minimum and needs no planner change at all.
3. **Only then, a first-class `Goal::Explored`** with an `ActionKind::Survey
   { to }` modelled on `Evacuate`, a `Condition::Charted` / `Effect::Charted`
   pair (and the `Effect::satisfies` entry, or `infer_edges` draws no edge),
   and a `Scout` method. This is worth doing when something needs exploration
   *scheduled against other work* rather than *before* it — e.g. sending one bot
   to look while three keep smelting. That is the real prize and it is also the
   first place where the concurrency machinery has to understand a goal nobody
   demanded.

**Do not build 3 before 1 and 2.** A `Goal::Explored` with no consumer is the
idle-work problem again.

---

## What was implemented here

`crates/core/src/graph/entity_graph.rs` — enemy structures are no longer
discarded on arrival.

* New `threats: DashMap<String, BTreeMap<Pos, Position>>`, populated in
  `EntityGraph::add` for the two entity types in the new
  `ENEMY_STRUCTURE_TYPES` (`"unit-spawner"`, `"turret"`), keyed by floored tile
  and holding the game's own position so it can be handed straight back to the
  mod. Cleared in `remove` the way `minables` is.
* `nearest_threat(from)`, `threats_from(from)` (nearest first, ties broken on
  position then name, floats via `total_cmp`), `threat_census()`.
* Recorded as a *separate map*, not by adding variants to `EntityType`.
  Admitting `unit-spawner` to `EntityType` would put nests into `entity_tree`,
  the petgraph and `snapshot_within`'s keyframes — a much larger claim than
  "remember where the nests are". A test pins `EntityType::from_str` returning
  `Err` for all three enemy spellings, so anyone who later adds the variant
  finds this rather than a silent duplicate.
* Live biters (`"unit"`) are deliberately not recorded: a unit moves and a chunk
  is written out once, so a stored biter is a permanent phantom.
* Five tests: the ingest and the biter exclusion; nearest-first ordering with
  the game's own position; `None` means *nothing charted, not safe*; removal;
  clone + serde round trip **and** a dump with no `threats` key still loading
  (defaulted like `minables`, so yesterday's world dumps are unaffected).

Nothing in `mods/BotBridge/control.lua` was changed. Every mod-side change this
document recommends needs a run to verify and none of them could be verified
here.

---

## What could not be determined without a run

1. **What actually generates chunks 500 tiles out.** The join-time ±512
   screenshot sweep is the leading candidate on extent alone; I have no
   documentation that `take_screenshot` forces generation, and determinism
   argues against it. **Test:** start a fresh map, take a `resource_fingerprint`
   before any client joins and again 60 s after client1 joins; if the census
   jumps from ~418 chunks' worth to ~1,024, the sweep is the cause. If it does
   not, run the same test across a 20-minute idle server to check enemy
   expansion.
2. **Whether oil patch A continues past `x = −512`**, and more generally how
   much the clamp is hiding. Answered by widening the clamp and re-booting onto
   the same savepoint.
3. **What a bot actually does when it meets a worm** — whether it dies, whether
   the walk fails with a distinguishable message, whether
   `classify_walk_failure` has wording for it. Nothing in the archive contains a
   bot death.
4. **`peaceful_mode` and evolution factor on the live workspace.**
   `map-gen-settings.example.json` ships `"peaceful_mode": false` and
   `"starting_area": 1`, but I could not confirm what this workspace's
   `level.zip` was created with.
5. **Radar's real sweep rate and range in 2.1.17.** The recipe, technology and
   power draw are measured above; the reveal geometry is not, and it decides
   whether radar is a 380-tile answer or a 100-tile one.

---

## Scale and decomposition

This is roughly a week, and it splits into five pieces that land independently.
Only the first is done.

| # | piece | size | depends on |
|---|---|---|---|
| 0 | **Threat index in `EntityGraph`** — nests are no longer discarded | done | — |
| 1 | **Honesty instrumentation** — the force's charted area vs. the model's census, in `provenance.json`; the ±512 bound recorded as a number | ~half a day, needs one run | — |
| 2 | **A bot death is an event** — recompute the roster, give the mod a distinguishable failure for `character == nil`, teach `just analyse` about it | ~1 day, needs one run | — |
| 3 | **"Unexplored, not absent"** — charting query on `PlanState`, typed `PlannerError`, frontier direction in the help text | ~1 day | — |
| 4 | **Scout-and-replan supervisor script** — catch the refusal, walk to the frontier inside the safe radius, re-plan | ~1 day, needs runs | 2, 3 |
| 5 | **Optional charted-only ingest** — `is_chunk_charted` guard plus `on_chunk_charted`, off by default | ~1 day, needs runs | 1, 4 |

Two things that look like they belong here and do not:

* **Widening the ±512 clamp** is 4 lines and can go with piece 1. It is not
  exploration; it is a bound that was never justified.
* **`researched:oil-processing`** is unblocked by fixing the `mine-entity`
  trigger payload in `mods/BotBridge/types.lua`, not by any of the above.
  Everything up to `oil-gathering` is science-only. Oil itself is needed for the
  pumpjack and the trigger — and for that, exploration and fluid modelling both
  have to exist. **The planner will currently plan hand-mining crude oil**
  (`Mine::applicable` gates only on `has_resource_patches`, and the resource
  name and item name are both `crude-oil`), which is not a thing a player can
  do. That trap is now *reachable*, because the model has oil in it.
