# Building a designed block, honestly, with four bots

2026-09-05. Sub-project 1 of three. Owner decisions recorded at the end.

## Why this and not more belt routing

A smelter array and a main bus are **designed** layouts, not routed ones. Nobody
pathfinds a belt through a bus; they stamp a block that was designed once and is
correct by construction. The belt router this repo just gained solves the
ad-hoc case — connect these two things around whatever is in the way — which is
exactly the case a blueprint removes. It keeps its value at the **joins between
blocks**, where the geometry is variable because the ore is where the map put
it, and that is later work.

The owner's framing, which this spec follows: smelter arrays give belt-based
throughput, those plates feed a bus-shaped medium base, and the multi-bot
advantage lives entirely **before construction robots**, because after that a
robot network builds from ghosts and one player is as fast as four bots.

Everything before robots is hand-placement, and stamping a two-hundred-entity
block by hand is mostly walking. **Walking parallelises.** That is where four
bots beat one player, and it is the same work the review already listed as
"place dozens of entities per plan instead of ones and twos", seen from the
blueprint side.

## What already exists, and why it cannot be used as-is

`rcon_place_blueprint` (`mods/BotBridge/control.lua`) already builds the ghosts,
walks them, finds each item in the inventories of a list of bots, revives the
ghost, and charges the item to whichever bot supplied it. It is exposed to Lua
as `rcon.place_blueprint`.

**It builds by teleport.** `ghost.revive()` from script needs nobody present and
respects no build reach, so a bot standing across the map pays for an entity
that appears instantly. That is acceptable in the `rcon` namespace, which is the
scripting escape hatch, and it is not something a measured run can use: the
executor's rule is that it issues only legitimate player actions.

**So ghosts are not used at all here.** They are a fine way to *say* what a
layout is and a bad way to *build* it, and they would prove nothing anyway —
ghosts do not collide, so a ghost layout that stamps perfectly can be
geometrically impossible.

## Architecture

### Decoding: `crates/core/src/blueprint.rs`

A blueprint string is base64 over zlib over JSON. Decoding yields entity names,
offsets and directions, and nothing else. **`base64`, `flate2` and `serde_json`
are already dependencies of `crates/core`** — this adds no new ones.

Pure data: no game, no I/O, no clock. A blueprint decodes in a unit test and a
block plans against a world dump offline, in seconds, with nothing running.

### The goal: a block stands at an anchor

`Goal::Built { blueprint, anchor }` — this blueprint stands at this position.

**It survives replanning, which is the whole reason it is shaped this way.**
Expanding it means *the set of its entities not yet standing*, re-derived
against the world every time. Every other goal here is item-shaped and
declarative for the same reason: this planner replans constantly, and a goal
naming particular machines would be invalidated the moment a replan sited a
different one. An anchored block is verifiable instead — and building it twice
is a no-op rather than a second factory.

### Emission: a new planner method

Ordinary `ActionKind::Place` actions, in the shape every other method emits:
`AtPosition`, `AreaFree` and `HasItem` preconditions, `LoseItem` and
`CreateEntity` effects, `state.create_entity`, and a materials bill as
`Goal::Have` subgoals so the existing shortfall machinery refuses before the
first placement.

Nothing new reaches the executor, so walking, reach, the pre-placement
step-aside that stops a bot sealing itself into a pocket, divergence handling
and recovery all come for free.

### The roster splits the block into vertical bands

Balanced by **entity count, not area**, so bands hold equal work, and assigned
deterministically. A bot never crosses another's band, which is the structural
reason two of them cannot trap each other — this repo has already lost a run to
a bot walled in by its own cell placements, and to a placement refused because
a character stood in the footprint.

## What the repo's own blueprints turn out to contain

Four real strings already live in `scripts/rcontest.lua`, and decoding them
settles several design questions that were guesses:

| blueprint | entities | contents |
|---|---|---|
| `FurnaceLine` | **179** | 87 transport-belt, 48 inserter, 24 stone-furnace, 13 pole, 3 lamp, **2 splitter**, **2 underground-belt** |
| `MinerLine` | 37 | 13 electric-mining-drill, 21 transport-belt, 3 pole |
| `StarterSteamEngineBoiler` | 6 | 2 steam-engine, 2 pole, boiler, pipe |

`FurnaceLine` is the smelter array this project wants, it already exists, and
it is 179 entities — which is exactly the scale at which four bots beat one
player, and far past what the planner places today.

**Splitters need no router.** A splitter inside a blueprint is just another
entity to place. The hard splitter problem is *deciding* where one goes, and a
designed block has already decided. This is the strongest argument for doing
blocks before more routing.

### Two traps found by decoding, both of which would place perfectly and do nothing

**1. Underground belts carry their half in the blueprint, and our placement
path cannot say it.** `FurnaceLine` contains
`{"name":"underground-belt","direction":2,"type":"input"}` and a matching
`"output"`. `FactorioEntity` has no field for that and
`rcon_place_entity(player_id, item_name, position, direction)` has no argument
for it, which is the same gap that made the belt router disable undergrounds.
**So it is a prerequisite here, not a later item**: a block containing an
underground pair cannot be built honestly until the placement path carries
`belt_to_ground_type`. Either that lands first, or v1 refuses blueprints
containing undergrounds by name — and `FurnaceLine` is such a blueprint.

**2. These are Factorio 1.x blueprints, and the direction scale changed.**
Their `version` field is `281474976710656`, i.e. 1.0.0.0, and every direction
in them is one of `0, 2, 4, 6` — the old **eight**-point scale where 2 is east.
Factorio 2.0 uses **sixteen** points, where east is 4 and 2 is a diagonal.
The mod never had to care because `import_stack` migrates on import; **decoding
in Rust bypasses that migration entirely.** Placing a 1.x blueprint's raw
directions into 2.1 turns every belt and inserter a half-turn, which is a
factory that places 100% correctly and moves nothing — this project's signature
failure, arriving through a new door. The decoder must convert by version, and
a test must pin east to east.

## Refusals, all before anything is placed

- an entity name the prototypes do not know;
- a blueprint carrying **tiles, circuit wiring, module requests or filters** —
  refused **by name**, never silently stripped, because a block that quietly
  builds two thirds of itself is worse than one that refuses;
- ground occupied by something that is not part of this blueprint;
- materials short, via the shortfall subgoals.

## Evidence

**Entities standing, not actions dispatched.** The live check reads the world
back and confirms every blueprint entity is present, at its position, with its
direction. A placement count is not evidence: this project has twice shipped
layouts that placed 100% correctly and did nothing.

Tests: a decode fixture (a real blueprint string against its expected entities);
band-split determinism; an offline plan against `workspace/scripts/map.json`
(seed 31337, fingerprint `c161fa3f437221d0`) showing the placements; then a
headless run at 5x with four bots, reading the world back.

## Out of scope

Where a block goes (sub-project 2). Getting materials into the right hands
(sub-project 3), which will dominate the clock. Belts, splitters and the joins
between blocks, which come after all three. Recipes, modules and circuit
networks — refused rather than half-built.

## Owner decisions (2026-09-05)

1. **Blueprint stamping over more belt work**, because a designed layout is not
   a routed one, and the multi-bot edge lives before construction robots.
2. **Decode the blueprint in Rust**, over stamping ghosts and reading them back,
   and over hand-written layouts in code — so blocks stay offline-plannable and
   a design can be pasted in from a game or a world record.
3. **Split the block into regions per bot**, over nearest-free-bot work
   stealing, because crowding is a failure this repo has already paid for twice.
4. This sub-project first, before siting and before material supply, because it
   is the one that produces a measurement.
