# Surfaces: what happens when we move into space

A survey, not an implementation. Nothing was changed. Every claim below cites a
file and a line, read in this worktree at `0e411a34`.

## Verdict

**This is not a refactor of everything, and it is not contained either. It is a
contained change to the *containers* and a wide change to the *record*, sitting
on top of one genuinely hard problem the planner has never had: a walk that is
not a walk.**

Said precisely, in three parts, because the parts have very different prices:

1. **Geometry is fine and stays fine.** Nothing in this codebase needs a
   three-dimensional coordinate. Two positions are only ever comparable *within*
   a surface, which is exactly what `Position { x, y }`
   (`crates/core/src/types.rs:582`) already says. The belt router
   (`crates/core/src/graph/route.rs:140`, `route_belt(blocked, origin, from, to,
   max_underground)`), the blueprint placer, the inserter-facing rule, the
   footprint scans — all of them operate inside one surface and would keep
   working unchanged the day a second one exists. This is the cheap half and it
   is most of the code.

2. **The containers are single-surface and would alias silently.** There is one
   `EntityGraph`, one `FlowGraph`, one `players` map
   (`crates/core/src/factorio/world.rs:980-1000`), one `PlanState` overlay
   (`crates/planner/src/state.rs:926-1130`), one quadtree world of ±5120 tiles
   (`entity_graph.rs:290`). A chest at (10, 10) on Nauvis and a chest at (10, 10)
   on a platform are the same key everywhere. That is a bounded, enumerable
   change — the enumeration is section 2 — but it is bounded only because the
   mod currently refuses to send the second surface at all
   (`mods/BotBridge/control.lua:1865`). Remove that one guard and the aliasing
   starts, everywhere, at once, with no error.

3. **One thing is genuinely new and cannot be retrofitted: crossing.** Every
   cost model in the scheduler prices movement as
   `travel_ticks(from, to, min_radius, radius)` over
   `calculate_distance` (`crates/planner/src/schedule.rs:102-112`), and every
   dispatch is `Actuator::walk(bot, to: Position, …)`
   (`crates/executor/src/actuator.rs:145-148`). A bot on Nauvis and a target in
   orbit would be priced at their *coordinate* distance — five tiles, maybe
   zero — and the executor would dutifully walk five tiles on the wrong surface
   and report success. That is not a bug to be fixed in the walk code; it is a
   missing concept (rocket, cargo pod, platform trip) that has to be *added* to
   the action vocabulary, the cost model, and the scheduler's per-bot timeline.

So: **do not budget "refactor everything". Budget "key the world by surface,
disclose the surface in the record, and design one new action class."** The
third is the expensive one, and it is expensive because it is design work, not
because it touches many files.

**One thing worth knowing before anything else: Space Age is already enabled in
this workspace.** `workspace/mods/mod-list.json` has `space-age`, `quality`,
`elevated-rails` and `recycler` all `"enabled": true`, and
`workspace/server/data/space-age/prototypes/planet/planet.lua` defines
`vulcanus`, `gleba`, `fulgora`, `aquilo` alongside `base`'s `nauvis` — **five
planets**, not six (verified by reading the prototypes; there are additional
non-planet space locations). So every run this project has ever measured has
been a Space Age run that happened never to leave Nauvis. The second surface is
not a future modding decision; it is one rocket away.

## 1. What the system supports today

**One surface, Nauvis, by construction — and the construction is a single
`return`.**

`mods/BotBridge/control.lua:1856-1890`, `on_chunk_generated`:

```lua
1865:	if surface ~= game.surfaces['nauvis'] then -- TODO we only support one surface
1866:		print("unknown surface")
1867:		return
```

Everything downstream of that — `writeout_entities` (1880), `writeout_tiles`
(1884), the `storage.map_area` widening (1875-1878) — never runs for another
surface. So Rust's `EntityGraph` has never contained a non-Nauvis entity, which
is the only reason the aliasing in section 2 has never fired.

The guard is also **weaker than it looks**, in two ways:

- The initial-discovery replay does not go through it in a meaningful sense:
  `on_whoami` enumerates `for chunk in game.surfaces[1].get_chunks()` (1237) and
  the on-tick drain synthesises `on_chunk_generated({… surface=game.surfaces[1]})`
  (1269). The replay can only ever present surface 1, so the guard is
  unreachable on that path.
- `print("unknown surface")` (1866) is **not** a `writeout`. `writeout` is
  `print("§"..tick.."§"..key.."§"..tostring(value))` (control.lua:3247), and
  `crates/core/src/process/output_parser.rs` keys on that sentinel. So the drop
  reaches the server log and *no record artefact at all*. A reader of the run
  would never learn a surface was discarded. Per this repo's own "silence is not
  success" rule, that is the fifth instance of the pattern.

**Confirming the headline.** `pub struct Position { pub x: f64, pub y: f64 }`
(`crates/core/src/types.rs:582-585`) — no surface. `FactorioEntity`
(`types.rs:1476-1513`), `FactorioPlayer` (`types.rs:502-522`), `FactorioTile`
(`types.rs:901-906`), `Pos(pub i32, pub i32)` (`types.rs:610`) — none carries a
surface. `grep -rn "surface" --include='*.rs'` over `crates/` and
`app/src-tauri/src/` returns 232 hits and **every one of them is either the
English word or a doc comment quoting Lua's `surface.*` API**. There is no
surface concept in Rust at all.

Nor in the mod's wire types: `serialize_player` whitelists nine fields, none of
them a surface (`mods/BotBridge/types.lua:100-110`); `serialize_entity`
whitelists `{"name","direction","type","position","drop_position"}`
(`types.lua:494`); `serialize_tile` whitelists `{"name","position"}`
(`types.lua:564-568`). Nor in the frontend: `app/src/api/types.ts` and
`app/src/lib/runMap.ts` have zero surface hits.

**The one deliberate acknowledgement of space in the whole system** is a
refusal: `ResearchTrigger::CreateSpacePlatform` (`crates/core/src/types.rs:1097`)
is rejected by name in the planner as
`UnsupportedResearchTrigger { act }` (`crates/planner/src/error.rs:244`,
`method/util.rs:929`), because no action in this project performs the act. That
is the right call and it should stay until an action does.

**Something the mod does that the guard does not cover.** Three loops already
iterate `pairs(game.surfaces)` today and flatten the result:

- `power_totals(force)` — `control.lua:2152-2155`, every electric pole on every
  surface summed into one `generated/consumed/demanded` triple.
- `sample_machines_body` — `control.lua:2898-2901`, every machine on every
  surface into one flat `machines` map. The cost comment at 2862 says "One
  `find_entities_filtered` per surface per 300 ticks" — the author knew.
- `refresh_drill_registry` — `control.lua:2607-2610`, same shape.

These are not hypothetical. They run on every measured run today. They produce
correct answers only because there is exactly one surface.

## 2. Where the assumption lives, enumerated

Three classes, because they cost differently. The distribution is the finding:
**there is essentially nothing in class (b).**

### (a) Would silently alias two surfaces into one

**The world model.**

| Site | Evidence |
|---|---|
| `EntityGraph::resources: DashMap<String, BTreeMap<Pos, Option<u32>>>` | `entity_graph.rs:191`. `add()` at `1256-1272` treats a repeat delivery for a known `Pos` as **already known** and overwrites the amount. Two surfaces' ore at the same tile is one entry, and the second one's amount silently replaces the first's. The dedup was written on purpose for chunk-replay repeats (`1226-1254`) and would swallow a second surface with the same mechanism. |
| The four quadtrees | `entity_graph.rs:295-298`, all built on one `max_area` of `(-5120, -5120)` + `10240 × 10240` (`290`). Every surface shares one coordinate space. |
| `EntityGraph::entity_at` | `entity_graph.rs:2112-2132`. On multiple hits it `warn!`s and returns `results[0]` — an arbitrary pick. This is the closest thing to a loud failure anywhere, and it is a `paris` warning on stdout that still returns an answer. |
| `minables`, `threats` | `entity_graph.rs:225, 271`, same `BTreeMap<Pos, …>` shape. |
| `resource_fingerprint` | `entity_graph.rs:1069-1110`. FNV-1a over `name.as_bytes()` then `pos.0/pos.1.to_le_bytes()`, with no surface component. Two surfaces do not merely collide — overlapping tiles are **lost** from the tile census, so the count a reader trusts to say "same map" would undercount. Written into `provenance.map` at `crates/scripting_lua/src/globals/record.rs:1104`. |
| `FactorioWorld` | `crates/core/src/factorio/world.rs:980-1000`: one `players`, one `entity_graph`, one `flow_graph`. `Planner` holds one `real_world` and one `plan_world` (`crates/core/src/plan/planner.rs:152-161`). |
| `FactorioWorld`'s hand-written `Serialize` | `world.rs:1679-1700`, 14 named fields; `EntityGraph`'s at `entity_graph.rs:2207-2226`, 11 named fields with an explicit field enum in `Deserialize`. This is the dump format, and it has no surface slot. |

**The planner.**

| Site | Evidence |
|---|---|
| `PlanState` overlay maps | `crates/planner/src/state.rs:982-1120`: `added: BTreeMap<Pos, FactorioEntity>`, `removed: BTreeSet<Pos>`, `consumed: BTreeMap<Pos, u32>`, `claimed: BTreeMap<Pos, MiningClaim>`, `committed_machines: BTreeSet<Pos>`, `machine_queue: BTreeMap<Pos, MachineQueue>`, `machine_load: BTreeMap<Pos, Ticks>`. Every one is keyed by a bare tile. A furnace committed on Nauvis would mark the same tile committed on Vulcanus. |
| `BotState` | `state.rs:505-511`: `position: Position` and three reach distances. A bot's surface is not representable in the plan's model of a bot. |
| Every positional `Condition` | `crates/planner/src/action.rs:31-160`: `AtPosition{pos}`, `EntityAt{pos}`, `PositionFree{pos}`, `AreaFree{pos, entity, direction}`, `Powered{pos}`, `ResourceAvailable`, `Feeds`. None carries a surface, so every precondition is a question about "the" tile. |
| `travel_ticks` | `crates/planner/src/schedule.rs:102-112`, called from `376`, `447`, `726` — the three places the scheduler prices a bot moving. Pure `calculate_distance` between two `Position`s. **This is the single most dangerous site in the survey**: a cross-surface target reads as a short walk. |
| Water siting | `state.rs:2854` → `entity_graph.rs:933` `nearest_water_tile`, used by `method/power.rs:974-978` to site a boiler and by `score.rs:323` to judge a map. Would happily site a Nauvis boiler on a Fulgora shoreline. |
| `MapScore` / `score-map` | `crates/planner/src/score.rs` and `app/src-tauri/src/cli/score_map.rs:42-49`. Would report a Vulcanus coal patch as 40 tiles of walking from a Nauvis spawn. |

**The mod (observation path).** These are hard-wired to `game.surfaces[1]`
while the action path follows the bot — the two halves disagree:

| Site | Evidence |
|---|---|
| `rcon_find_entities_filtered` | `control.lua:5348-5349` |
| `rcon_find_tiles_filtered` | `control.lua:5358-5359` |
| `rcon_inventory_contents_at` | `control.lua:5310-5316`; it echoes the *request's* name and position back (5340-5341), so a coincidental match on the wrong surface is reported under the caller's own coordinates |
| `rcon_generate_chunks` | `control.lua:5011-5012` |
| `rcon_async_request_path` | `control.lua:5816-5817` — a path computed across Nauvis terrain for a platform route |
| force production statistics | `control.lua:2374` and `6389`: `force.get_item_production_statistics(game.surfaces[1])`. **Verified against the API**: `LuaForce::get_item_production_statistics` takes a `SurfaceIdentification` and is per-surface in 2.x (`workspace/factorio-api-docs/runtime-api.json`, `application_version` 2.1.17). So `production.made` is Nauvis-only, in the *same sample line* as `power` and `machines`, which are all-surface. Internally inconsistent today, undisclosed. |
| bot respawn and spawn | `control.lua:6293` `create_bot_character(game.surfaces[1], …)` and `6117`. A bot that dies on a platform respawns on Nauvis and the `player_respawned` writeout (6296-6299) carries only `player_id` and `x/y`. |
| `resource_key` | `control.lua:2551-2552`, `resource.name .. "@" .. x .. "," .. y`. Its own comment two lines above reads *"one tile holds one resource entity, so `name@x,y` cannot collide"* — a single-surface assumption written down as a proof. |
| `machine_key` fallback | `control.lua:2440-2446`. `unit_number` is engine-global and safe; the `name@x,y` fallback is not. |
| walk waypoints | `control.lua:3800`, `3832-3834`: `tmp[i] = {x=waypoints[i][1], y=waypoints[i][2]}`. The wire format is a 2-tuple. **A cross-surface walk target is not expressible**, so it degrades to walking to those local coordinates. |
| `writeout` envelope | `control.lua:3247`, `print("§"..tick.."§"..key.."§"..tostring(value))` — three fields, no surface slot. `writeout_tiles` (3161), `writeout_entities` (3206) and `writeout_resources` (3175) each *take* `surface` as a parameter and drop it from the header (3163, 3208, 3177). |

**The record and the tools.**

| Site | Evidence |
|---|---|
| `Provenance` | `crates/core/src/record/provenance.rs:58-128` — 15 fields, no surface, no planet. |
| `MapRecord` / `EntitySnapshot` / `Bounds` | `crates/core/src/record/map.rs:15-77`. A keyframe is one 2D rectangle. |
| `Sample` / `MachineSample` / `PowerSample` | `crates/core/src/record/samples.rs:47-362`. |
| Keyframe divergence | `crates/scripting_lua/src/globals/record.rs:163-198`. The `game` half resolves through `rcon_find_entities_filtered` → **surface 1**; the `model` half is `snapshot_within` over the merged graph. On two surfaces the two halves are drawn from different populations and `divergence` fills with spurious `only_in: "model"` rows — noise, not an error. |
| `tools/run_analysis.py` power inference | `run_analysis.py:1862-1892`. `any_generation` drives the documented inference *"if the force generated nothing at any beat of the interval, nothing electric ran, so whatever was produced came out of a burner machine"* — a steam engine on a platform falsifies that for Nauvis. |
| `run_analysis.py` production vs machines | `production_at`/`_made` at `1707-1755` read surface-1-only production; `machines_at` accumulates all-surface machine rows. The two halves of the report would disagree with no way to see why. |
| `app/src/lib/runMap.ts` | `samePlace(a, b)` at `18-20` is `name && x && y`. Its own doc comment (13-16) argues that position alone is insufficient *for name disambiguation* — the identical argument applies to surfaces and is not made. A `removed` record could delete the wrong entity. |
| `MapPanel.vue` | props at `56-62`, projection frame at `128` — one 2D frame, no layer selector. |

### (b) Would fail loudly

**Effectively empty.** Two candidates, and neither is really loud:

- `control.lua:1865-1867` prints `"unknown surface"` to the server log — no
  `§tick§` envelope, no parser arm, no record entry. Loud to a human staring at
  stdout; silent to every artefact.
- `EntityGraph::entity_at` (`entity_graph.rs:2124-2131`) `warn!`s on multiple
  quadtree hits and then returns `results[0]` anyway.

**There is no path in this codebase that refuses to proceed on a second
surface.** That is the finding that should drive the staging in section 3.

### (c) Genuinely surface-agnostic — needs nothing

- **All the geometry.** `route_belt` (`crates/core/src/graph/route.rs:140`) takes
  a rasterised `blocked` grid and an origin; it does not know what a surface is
  and does not need to. `method::connect`'s `inserter_facing`,
  `method::blueprint`'s `BuildBlock`, `enclosure::rasterize`,
  `method::util::tile_alignment` — all operate inside one surface's grid.
- **The mod's action path**, by accident rather than design: ten RCON entry
  points resolve the surface from `player.surface` and are therefore correct for
  a bot wherever it stands — `rcon_action_start_mining` (`control.lua:3864`),
  `rcon_place_entity` (`3916`, and `can_place_entity` at `3941`,
  `create_entity` at `4065`), `rcon_can_place_entities` (`4593`),
  `rcon_insert_to_inventory` (`4649`), `rcon_remove_from_inventory` (`4710`),
  `rcon_set_recipe` (`4797`), `rcon_revive_ghost` (`5446`),
  `rcon_place_blueprint` (`5580`), `rcon_cheat_blueprint` (`5719`),
  `rcon_async_request_player_path` (`5777`).
- **The whole stall / step-aside / footprint family**, which already threads
  `surface` as an explicit parameter: `walk_tile_is_clear(surface, …)` (`664`),
  `walk_stall_cause` (`589`), `walk_step_clear_landing` (`692`),
  `push_characters_out_of(surface, …)` (`4162`),
  `character_in_footprint(surface, …)` (`4259`), `scan_footprint(surface, …)`
  (`4274`), `placement_step_aside_landing(surface, …)` (`4396`),
  `step_aside_from_footprint(surface, …)` (`4466`). This is what the rest of the
  mod should look like.
- **`rcon_world_snapshot`** (`control.lua:4958-4966`) — prototypes, item
  prototypes, recipes, forces. Carries nothing spatial by design; its own doc at
  `4948-4951` says entities are excluded because *"a whole surface would be
  unbounded"*. It is also the natural home for a surface roster and does not
  carry one.
- **The no-position RCON calls**: `rcon_savepoint` (`3033`),
  `rcon_sampling_start/stop` (`2986`/`3007`), `rcon_session_reset` (`3078`),
  `rcon_whoami` (`4856`), `rcon_player_force` (`4936`), research and crafting
  starts, `rcon_store/retrieve_map_data`.

**The asymmetry is the headline of this section: the action path is
surface-correct because it follows the bot; the observation path is pinned to
surface 1 almost everywhere.** A bot on a platform could mine and place
correctly while Rust's world model, its production numbers and its pathfinder
all described Nauvis.

## 3. The staged cost

Four rungs, cheapest useful increment first. Each says what it buys and what
could break silently if it is skipped.

### Rung 0 — Make the assumption *stated* rather than *assumed*. (Hours of work; buys correctness of belief.)

Do nothing structural. Add:

- a `surface` field to the mod's `serialize_entity`, `serialize_tile`,
  `serialize_player` and the bot sample rows — **carried but not used**;
- an assertion, once at session start, that `game.surfaces[1] == game.surfaces['nauvis']`
  (today `control.lua:1237/1269` and `1865` assume this and never check it);
- a `surfaces` roster in `rcon_world_snapshot` (index → name), which costs one
  loop and immediately tells the record what world it was in;
- a `surface` field in `Provenance` naming the run's single surface;
- disclose the inconsistency at `control.lua:2374`: either take production per
  surface, or record that it is Nauvis-only, because
  `run_analysis.py`'s `any_generation` inference currently rests on numbers from
  two different populations.

**Buys**: every artefact from this day forward can be *read* on a multi-surface
run, and the next rung's migration has data to test against. Costs a serde field
each — and this repo has the precedent: `FactorioEntity::underground_half` is
`#[serde(default)]` precisely so "every archived run record and world dump
written before this field existed still deserialises"
(`crates/core/src/types.rs:1507-1512`).

**Skipping it**: every number measured between now and rung 2 is unlabelled, and
per this repo's own experience with `--seed` (silently ignored until `61ec7364`,
retroactively unidentifiable), unlabelled measurements cannot be backfilled.

### Rung 1 — Key the world by `(surface, position)`. (The bounded structural change.)

Introduce `SurfaceId` (see section 4 for the shape). Then:

- `EntityGraph` becomes per-surface. **Two shapes are possible and the choice
  matters**: (i) `BTreeMap<SurfaceId, EntityGraph>` — every quadtree, every
  `BTreeMap<Pos, …>`, `resource_fingerprint`, `nearest_water_tile`,
  `resource_patches` keep working unchanged inside one surface, and the
  `±5120` bound (`entity_graph.rs:290`) stops being shared. (ii) widen every key
  to `(SurfaceId, Pos)`. **(i) is strictly better** because it preserves every
  existing invariant by construction and makes "compare two positions" a
  type-level question of *which graph you asked*. It also makes
  `resource_fingerprint` naturally per-surface, which is what a map identity
  should be.
- `PlanState` follows: its seven `Pos`-keyed maps become per-surface, or the
  whole `PlanState` becomes single-surface with a named surface. The latter is
  cheaper and is probably right for rung 1 — see rung 2.
- The mod's writeouts start carrying the surface (rung 0 already added it) and
  the `1865` guard is replaced by routing rather than dropping.
- `machine_key`'s fallback and `resource_key` (`control.lua:2440-2446`, `2551`)
  become surface-qualified; the comment at `2549-2550` gets corrected.

**Buys**: a world model that can hold two surfaces without lying. Nothing plans
across them yet.

**Skipping it and opening the mod guard anyway**: the aliasing in section 2(a)
fires with no error anywhere, and the first symptom would be a keyframe
divergence full of `only_in: "model"` rows that reads as a mod bug.

### Rung 2 — Teach the planner that a walk cannot cross surfaces. (The rung that makes it safe.)

Make `travel_ticks` and every `Condition` carrying a `pos` surface-aware, and —
before anything can plan a crossing — make a cross-surface requirement a
**typed refusal**, in the same family as `ConnectRefusal::NoRoute` and
`PlannerError::BlockGroundOccupied`. Something like
`PlannerError::CrossSurface { bot, bot_surface, target, target_surface }`.

This is the rung that converts the entire class (b)-is-empty finding into
class (b)-is-nonempty. **It is worth doing on its own even if nobody ever builds
a rocket**, because the alternative to a refusal is not "it doesn't work" — it
is a five-tick walk to the wrong planet, dispatched, settled, and recorded as a
success. This repo has already paid for that shape twice: the inserter direction
that "places 100% correctly and does absolutely nothing", and
`only_ghosts = true` validating nothing.

The cheapest honest version of rung 2 is: **one surface per plan**. A
`PlanState` names its surface; `from_world` takes it; a goal whose target is on
another surface refuses by name. Multi-surface *planning* then becomes rung 3's
problem and rung 2 is a few dozen signatures plus one error variant.

**Buys**: a run can go to space and the planner will say so instead of
pretending. Also makes the record honest: per-surface production, per-surface
power, per-surface map fingerprint.

### Rung 3 — Rockets, cargo pods and platforms. (Design work, not plumbing.)

New action classes with no analogue in the current vocabulary: launch, transfer
between surfaces, a platform that *moves between space locations* while entities
sit on it. Concretely:

- `Actuator` needs verbs that are not `walk`/`place`/`mine`/`insert`.
- `schedule()`'s per-bot timeline needs a cost for a crossing that is not a
  distance — and crossings are *scheduled events with capacity*, not free.
- **A space platform's surface moves.** `LuaSpacePlatform.space_location` is
  *"the space location this space platform is stopped at or `nil`"*
  (verified in `runtime-api.json`, 2.1.17). So a surface's *identity* is stable
  but its *position in the solar system* is not, and any model that treats
  "which surface" as sufficient to locate something will be wrong for platforms
  in transit.
- **A planet's surface may not exist yet.** `LuaPlanet.surface` is documented as
  *"the surface for this planet if one currently exists… Planets do not default
  generate their surface"* (same source). So "enumerate the surfaces" and
  "enumerate the planets" are different questions with different answers, and a
  roster keyed on `LuaSurface.index` cannot name a planet nobody has visited.

Nothing below rung 3 needs to be right about any of this. Rungs 0-2 are worth
doing on their own merits.

## 4. What is cheap NOW and expensive in a year

This is the section that matters. Five things, in order of ratio.

**1. Do NOT put a surface field on `Position`.** This is the tempting move and it
is wrong. Mechanically it is nearly free — 1,433 `Position::new(` call sites
against only 18 struct literals in `crates/` and `app/src-tauri/src/`, so a
two-argument `new` defaulting to Nauvis would compile. But `Position` is a
*value* used in arithmetic: `manhattan_distance` (`types.rs:587`),
`calculate_distance`, `Sub`, `From<Position> for Point2D`, the whole belt
grid. Putting a surface on it forces every one of those to answer "what is
`p1 − p2` across surfaces?", and the honest answer — "undefined" — cannot be
expressed by a subtraction that returns an `f64`. It also changes `PartialEq`
semantics under 2,591 mentions, and it ripples straight into
`app/src/api/openapi.snapshot.json` (`Position` is a published schema with
`required: [x, y]`), `app/src/api/types.ts`, and the generated `docs/lua/types.lua`
— three failing contract tests for a type that should not change.

**The right shape: keep coordinates 2D and make the *container* surface-scoped.**
Coordinates are only comparable within a surface; that is a fact about the
domain, and `Position { x, y }` already models it correctly.

**2. Add `SurfaceId` as a newtype today, unused.** One file,
`crates/core/src/types.rs`, next to `PlayerId`:

- a newtype over the surface **name** (`String`), not the index. Verified:
  `LuaSurface.index` is *"assigned when a surface is created, and remains so
  until it is deleted"* while `LuaSurface.name` is *"unique among surfaces"* —
  so the index is stable within a save and the name is stable across records.
  For an artefact somebody reads next year, the name is the identity.
- with a `nauvis()` constructor and a `Default`.

Cost today: near zero. Value later: every one of the ~30 sites in section 2 that
needs a surface has a type to name, and the diff that adds them is mechanical
instead of also being a design argument.

**3. Add the surface to the mod's four wire types now, defaulted, ignored.**
`serialize_entity` (`types.lua:494`), `serialize_tile` (`types.lua:564`),
`serialize_player` (`types.lua:102`), and the bot sample rows
(`control.lua:2304-2314`). On the Rust side each is `#[serde(default)]`, which
this codebase already does for exactly this reason (`types.rs:1507-1512`).

Why now specifically: **the mod already has the value in hand at every one of
these sites** (`entity.surface.name`, `player.surface.name` — the character
proxy at `control.lua:5938` explicitly forwards `surface`), and adding it later
means a wire-format change *plus* a migration of every archived run. Adding it
now means archived runs simply lack the field and read as Nauvis, which is true.

**4. Make `resource_key` and `machine_key`'s fallback surface-qualified now**
(`control.lua:2551`, `2440-2446`). These are two string concatenations. The
comment at `2549-2550` currently states a false theorem — *"one tile holds one
resource entity, so `name@x,y` cannot collide"* — which is true per surface and
false across surfaces, and a reader will believe it. Fixing the key is three
lines; fixing the counters after they have merged two machines' lifetime
production is not fixable at all, because the record does not retain the
inputs.

**5. Turn the `1865` drop into a recorded event.** Today a discarded surface
reaches the server log and no artefact. Making it a `writeout` with a key —
counted per surface, once, not per chunk — costs a few lines and means that on
the first day somebody accidentally generates a platform, the run record says
so. Per the "silence is not success" rule this is the cheapest possible
insurance and it is worth more than the guard itself.

**What is not worth doing now**: splitting `EntityGraph`, touching `PlanState`,
or restructuring the quadtrees. Those are rung 1, they are mechanical once
`SurfaceId` exists, and doing them speculatively would churn the most
heavily-tested code in the repo for a capability nobody is exercising.

## 5. Honest unknowns

- **Whether one `EntityGraph` per surface is affordable.** The quadtrees are
  `10240 × 10240` each and there are four of them per graph
  (`entity_graph.rs:290-298`). Five planets plus platforms is 6+ graphs. I did
  not measure the memory of an empty `QuadTree` at that extent, and I did not
  read whether the trees allocate lazily. **Unverified.** If they allocate
  eagerly this is the first thing rung 1 has to answer.
- **What the `±5120` bound means for a space platform.** Platforms are small and
  near-origin, so they fit — but I did not verify what a platform's coordinate
  range actually is, in the data or live. **Unverified; my own knowledge only.**
- **Whether a character can exist on a platform surface at all**, and whether
  `create_bot_character` would succeed there. `control.lua:6234-6235` uses
  `force.get_spawn_position(surface)`, and I do not know what that returns for a
  platform. **Unverified.** This gates the entire "bots in space" premise and is
  cheap to answer with one `rcon -s localhost` query against a live game.
- **How a bot actually crosses.** In Space Age a player rides a rocket or a
  cargo pod. Whether that is expressible through the legitimate-player-actions
  constraint the executor holds itself to (`crates/executor`, "no `cheat_*`
  calls") is unknown to me. **Unverified.** If it is not, rung 3 has a
  constraint problem before it has a scheduling problem.
- **Whether trigger technologies fire per-surface.** `control.lua:6389` reads
  Nauvis production for the research-trigger emulation, so a trigger earned by
  producing on another surface would not fire today. Whether the *game's own*
  trigger check is per-surface or per-force I did not test. **Unverified**, and
  testable the way the 2026-09-05 trigger work was: switch the sweep off and
  watch.
- **Whether `game.surfaces[1]` is always Nauvis.** `control.lua:1237/1269` and
  `1865` assume it; I found no assertion. It is almost certainly true in a fresh
  save and I did not verify what happens after `LuaSurface` deletion. **Assumed,
  not verified.**
- **The planet count.** Five planets are defined in the shipped data (`nauvis`
  plus `vulcanus`, `gleba`, `fulgora`, `aquilo`). The owner said "around 6".
  There are additional non-planet space locations; whether the sixth is one of
  those or a mod is not something I checked.

## One defect found, not fixed

`control.lua:2374` (and `6389`) take `force.get_item_production_statistics(game.surfaces[1])`
while `power_totals` (`2152`) and `sample_machines_body` (`2898`) iterate every
surface. **Today, with one surface, these agree.** The moment a second surface
exists — a platform, a Vulcanus outpost — a single `samples.jsonl` line would
carry Nauvis-only production alongside all-surface power and machines, and
`tools/run_analysis.py`'s attribution verdict (`roster-fed` / `factory` /
`unclear`) would be computed across the mismatch with no way to detect it. It is
not a bug now; it is a trap that arms itself. Worth fixing at rung 0, when it
costs one function signature.
