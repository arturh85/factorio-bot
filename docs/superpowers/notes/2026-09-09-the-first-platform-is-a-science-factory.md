# The first platform is a science factory, and the wall is on Nauvis

*2026-09-09. Research only — nothing was built and no game was started.*

**Provenance.** Every prototype number below is read out of the prototype
section of `workspace/scripts/map-31337-water-and-oil.json` (662 recipes, 1,028
entity prototypes, 342 item prototypes, 277 technologies; Space Age 2.1.17 with
the repo's six mods). The offline plan probes ran `target/release/factorio-bot
plan` (binary of 2026-09-08 22:04) against that same dump. Nothing here is
quoted from a wiki.

---

## 1. The chain

```
space-science-pack   1 ice + 2 iron-plate + 1 carbon -> 5 packs
                     category "crafting", energy 15 s
```

`crafting` means assembling-machine-1/2/3 **or a character's own hands**
(`character.crafting_categories = ["crafting", "hand-crafting"]`). Only two
inputs actually need a machine that is not an assembler.

The three inputs, and where they come from in orbit:

| input | recipe | tech | machine | catalyst |
|---|---|---|---|---|
| `ice` | `oxide-asteroid-crushing` | `space-platform` | `crusher` | 1 oxide chunk -> 5 ice + chunk @0.3 |
| `carbon` | `carbonic-asteroid-crushing` | `space-platform` | `crusher` | 1 carbonic chunk -> 10 carbon + chunk @0.3 |
| `iron-plate` | `metallic-asteroid-crushing` then `iron-plate` | `space-platform` / start | `crusher` then any furnace | 1 metallic chunk -> 20 iron-ore + chunk @0.3 |

All three crushing recipes return their own chunk at p = 0.3, so a chunk is
crushed **1/(1−0.3) = 1.4286 times** before it is gone. Net yield per chunk
*consumed*: **7.14 ice**, **14.29 carbon**, **28.57 iron-ore**.

That fixes the asteroid mix. Per craft (= 5 packs): 0.140 oxide chunks, 0.070
carbonic, 0.070 metallic — **oxide : carbonic : metallic = 2 : 1 : 1**.

Machine-seconds per craft, and therefore the ratios:

| machine | s per craft | at 1 assembler |
|---|---:|---|
| `crusher` — oxide | 0.40 | |
| `crusher` — carbonic | 0.20 | |
| `crusher` — metallic | 0.20 | 0.04 crushers total |
| furnace (speed 2) | 3.20 | 0.16 furnaces |
| `assembling-machine-2` (0.75) | 20.00 | **15 packs/min** |

One `assembling-machine-1` gives 10 packs/min, AM2 15, AM3 25. **One crusher of
each type is ~25× oversized for one assembler** — the crushers are never the
limit. Asteroid arrival is, and the dump carries nothing about asteroid spawn
rates (see §6).

**Thrusters are not part of the first platform.** `thruster-fuel` and
`thruster-oxidizer` are unlocked by `space-platform-thruster`, which costs
**500 × space-science-pack**. You cannot fuel a thruster before you make space
science; the first platform sits in Nauvis orbit and does not move. Same for
`ice-melting` (water from ice) — also `space-platform-thruster`. Recorded for
completeness:

| recipe | tech | in -> out |
|---|---|---|
| `thruster-fuel` | `space-platform-thruster` | 2 carbon + 10 water -> 75 |
| `thruster-oxidizer` | `space-platform-thruster` | 2 iron-ore + 10 water -> 75 |
| `ice-melting` | `space-platform-thruster` | 1 ice -> 20 water |
| `advanced-*-asteroid-crushing` | `advanced-asteroid-processing` | needs Vulcanus (metallurgic science) |
| `*-asteroid-reprocessing` | `asteroid-reprocessing` | needs Vulcanus |

So the whole Vulcanus branch (reprocessing, advanced crushing, calcite, foundry)
is **behind** the first platform, not in front of it.

### Technology gate, in order

`space-science-pack` ← `space-platform` ← `rocket-silo`. Both space techs are
**trigger** techs with zero science cost:

| tech | cost | trigger |
|---|---|---|
| `rocket-silo` | 1000 × red+green+blue | — |
| `space-platform` | free | `create-space-platform` |
| `space-science-pack` | free | `build-entity: asteroid-collector` × 1 |

The closure of `rocket-silo` is **29 technologies**, four of them free triggers
(`electronics`, `steam-power`, `automation-science-pack`, `oil-processing`), and
the rest total:

> **4,225 automation + 4,090 logistic + 2,650 chemical science packs.**

Our best live run plateaus at ~85 red packs. That is the real distance.

---

## 2. The minimum platform

Derived, not copied from The Rook. Strictly required:

| entity | count | why | power |
|---|---:|---|---|
| `space-platform-hub` | 1 | arrives with the starter pack; has no recipe | not captured |
| `asteroid-collector` | ≥1 | **the only asteroid source**, and the thing whose placement fires the tech | not captured |
| `crusher` | 3 | one per chunk type; `crushing` runs in nothing else | 540 kW each |
| furnace | 1 | `smelting` runs in stone/steel/electric furnace only | see below |
| `assembling-machine-1/2/3` | 1 | or a character's hands | 75 / 150 / 375 kW |
| `space-platform-foundation` | ~60+ | the ground itself; a **tile**, not an entity | — |
| power + inserters + belts | — | solar 60 kW/panel, accumulator 5 MJ / 300 kW | |

**Optional, and worth dropping for a first platform:** `cargo-bay` (storage/
throughput only), `thruster` (cannot be fuelled — see §1), quality of any kind,
and every recycler/reprocessing loop.

**Take a `steel-furnace`, not an `electric-furnace`.** Both smelt at speed 2.
`electric-furnace` costs `advanced-material-processing-2` (250 × red+green+blue)
and 180 kW; `steel-furnace` costs `advanced-material-processing` (75 ×
red+green), draws no electricity, and burns the `carbon` the carbonic crusher is
already making (`carbon` fuel_value = 2 MJ). One tech tier and 180 kW saved, and
one less thing to ship. *Caveat:* the dump carries no `fuel_category`, so
"a furnace accepts carbon" is not established from this data.

**Power.** Continuous demand at 15 packs/min is roughly 150 kW (AM2, ~100% duty)
+ ~22 kW (three crushers at 4% duty) + collectors + inserters ≈ 200–300 kW —
about 5 solar panels' worth — but crusher *peak* is 1,620 kW, so either
accumulators or ~30 panels. Solar output in orbit is not something this dump can
answer (§6).

---

## 3. Nauvis-side prerequisites

`space-platform-starter-pack` (tech `rocket-silo`, 60 s):
**20 steel-plate + 20 processing-unit + 60 space-platform-foundation**, where
`space-platform-foundation` = 20 steel-plate + 20 copper-cable each.

Raw ore/fluid, Nauvis recipes only (no Gleba/Vulcanus/Fulgora branch):

| target | iron-ore | copper-ore | coal | crude-oil | water | stone |
|---|---:|---:|---:|---:|---:|---:|
| `space-platform-starter-pack` ×1 | 6,582 | 1,400 | 40 | 1,727 | 1,214 | — |
| `rocket-part` ×50 | 1,705 | 3,000 | 225 | 49,975 | 25,862 | — |
| `rocket-silo` ×1 | 12,220 | 8,600 | 400 | 29,273 | 28,136 | 1,000 |
| **one launch, total** | **20,507** | **13,000** | **665** | **80,975** | **55,212** | **1,000** |
| `asteroid-collector` ×1 | 408 | 624 | 60 | 2,730 | 1,452 | — |
| `crusher` ×1 | 360 | 430 | 50 | 2,418 | 1,209 | — |
| min-platform payload* | 3,256 | 3,198 | 275 | 15,624 | 11,487 | 20 |

\* 2 collectors, 3 crushers, 1 electric-furnace, 1 AM2, 20 solar, 10 accumulator,
12 inserters, 40 belt, 8 medium poles, 4 steel chests.

**We have never made steel.** `steel-plate` = 5 iron-plate, 16 s smelting, tech
`steel-processing` (50 red). It is the first thing on this list and it is
cheap.

**The 50 in "rocket-part ×50" is not from the dump.** `rocket_parts_required`
is not a field the dump carries; 50 is the number I assumed. Treat that row as
conditional.

**Ordering matters and is not obvious:** `asteroid-collector`, `crusher` and
`cargo-bay` are unlocked by `space-platform`, whose trigger is *creating a
platform*. So the sequence is: research `rocket-silo` → build silo → craft
starter pack → create platform + insert pack → **then** the collector/crusher
recipes exist → craft them on Nauvis → rocket them up → placing the collector
fires `space-science-pack`.

---

## 4. The gap list, ranked

Ranked by what blocks what. Kinds: **(a)** capability exists, nothing emits;
**(b)** genuinely missing; **(c)** bridge gap (game knows, mod does not send).

| # | gap | kind | evidence |
|---|---|---|---|
| 1 | **Nauvis-side production at 4,225/4,090/2,650 packs** | (b) — a factory, not code | §1; our runs plateau at ~85 red |
| 2 | **Tile placement.** `space-platform-foundation` is a tile; the platform *is* tiles | (b), with decode already done | decoder reads them `crates/core/src/blueprint.rs:964-988`, placement refuses `:1075`; `set_tiles` has **zero** occurrences in the whole tree; `rcon_place_entity` refuses tile items at `mods/BotBridge/control.lua:4559` because a tile item's `place_result` is nil |
| 3 | **`create_space_platform` has no emitter.** | **(a)** | `FactorioRcon::create_space_platform` exists and is proven live (`crates/core/src/factorio/rcon.rs:5988`, note `2026-09-07-a-space-platform-needs-one-new-verb.md`), exposed only to Lua (`crates/scripting_lua/src/globals/rcon.rs:426-440`). No `ActionKind`, no `Goal`, no planner method, no HTTP route. |
| 4 | **The planner cannot state the two space triggers.** | (b) | measured: `plan --goal researched:space-science-pack` → *"space-science-pack is unlocked by a build-entity trigger -- build 1 asteroid-collector -- which this planner cannot express as a goal"* (`PlannerError::UnsupportedResearchTrigger`, `crates/planner/src/error.rs:283`) |
| 5 | **The mod drops every non-Nauvis chunk.** | **(c)** | `mods/BotBridge/control.lua:2122-2128`, writes `surface_chunk_dropped`. Rust is ready: `FactorioWorld` is multi-surface (`world.rs:1260,1427`) and `OutputParser` already routes per surface (`output_parser.rs:110-120`). |
| 6 | **The planner has no surface concept at all.** | (b) | `PlanState { base: Arc<FactorioSurface> }` (`crates/planner/src/state.rs:1358`); `SurfaceId` appears in planner code only inside `#[cfg(test)]`. Neither `Goal` nor `ActionKind` carries a surface. |
| 7 | **`only_surface()` refuses once a second surface exists** — and one caller panics | (a)/(b) | `crates/server/src/game/mod.rs:29`, `crates/server/src/manage/execute.rs:182`, `crates/core/src/process/process_control.rs:53`, and `app/src-tauri/src/repl/run_script.rs:26` `.unwrap()` |
| 8 | **Mod reads hardcode `game.surfaces[1]`** — a bot on a platform gets well-formed answers about Nauvis | (c) | `control.lua:6020` (`find_entities_filtered`), `:6030` (`find_tiles_filtered`), `:5952` (`inventory_contents_at`), `:5653` (`generate_chunks`), `:6521` (`async_request_path`), `:6822` (spawn), `:7218` (respawn teleports a dead bot to Nauvis) |
| 9 | **Daylight is Nauvis-only** — a platform's solar curve is never emitted | (c) | `bridge_surface()` returns `game.surfaces['nauvis']`, `control.lua:1340-1344` |
| 10 | **Quality is not emitted anywhere** | (b) | no quality param on `place_entity` (`rcon.rs:5255`) or `rcon_place_entity` (`control.lua:4527`); `FactorioEntity` has no quality field. Read side exists (`InventoryItemWithQuality`, `types.rs:3522`). **Not a blocker** — a normal-quality platform works. |
| — | **Rocket launch** | *not a gap* | the silo launches itself; `launch_rocket` returned `false` throughout the run that produced a platform |

Gaps 2, 3, 5 and 6 are the four that must close for a planned platform. 3 is the
cheapest (an action kind over a call that already works). 2 and 6 are real work.

---

## 5. What is measurable without leaving Nauvis

Measured tonight against the dump, no game:

| goal | result |
|---|---|
| `researched:steel-processing` | plans — 290 actions, 56,145 ticks (15:35) |
| `have:steel-plate:5` | plans — 341 actions, 67,862 ticks (18:51) |
| `researched:oil-processing` | plans — 2,012 actions, 316,042 ticks (1:27:47) |
| `researched:rocket-silo` | **refuses**, and not for a space reason |
| `have:space-platform-starter-pack:1` | **refuses**, same reason |
| `researched:sulfur-processing` | plans — 2,469 actions, 409,807 ticks (1:53:50) |
| `researched:plastics` | plans — 2,509 actions, 423,904 ticks (1:57:45) |
| `researched:space-platform` | refuses: *"unlocked by a create-space-platform trigger -- create a space platform -- which this planner cannot express as a goal"* |
| `researched:space-science-pack` | refuses: `UnsupportedResearchTrigger` |

Both rocket-silo-class goals fail at the same place, far below anything to do
with space:

> `sulfur runs in chemical-plant, which needs 2 input and 0 output fluid port(s)
> all clear at once, and none of the 380 footprint(s) around [46.5, -8.5] can
> hold it: 86 had a port that could not take a pipe and 294 had every port clear
> but no pipe route back to the source`

**That is tonight's actual blocker on the path to space**, and it is a fluid-
siting refusal on Nauvis. It is offline-reproducible in ~90 seconds and needs no
platform, no surface work and no new verb.

**Can be validated offline once the planner can name them:** the whole Nauvis
prerequisite chain (steel → plastics → sulfur → chemical science → processing
unit → LDS → rocket fuel → silo → starter pack), and — if a platform surface
ever reaches a dump — the in-orbit chain too, since all its recipes and machines
are already in the prototype tables.

**Genuinely needs a launched platform:** asteroid arrival rate (nothing in the
dump), solar output in orbit, hub and collector power draw, whether
`space-science-pack`'s `build-entity` trigger fires for `create_entity` the way
the other build triggers do not (CLAUDE.md: the mod emulates `build-entity`
because `surface.create_entity` fires nothing), and whether the entity graph
behaves on a platform's coordinate frame.

---

## 6. What the dump does not carry — absent, not zero

- `electric_energy_usage` is **null** for `asteroid-collector`, `space-platform-hub`
  and `inserter`. These are not 0 kW; the field was not captured for them.
- No `fuel_category` on items, so "a steel furnace burns carbon" is unverified.
- No `rocket_parts_required` on `rocket-silo`.
- Nothing about asteroid spawn rates, platform speed, or orbit solar multiplier.
- `space-platform-hub` has **no recipe** in the table at all — it is not craftable;
  it arrives with the starter pack.
- `inventories: []` in this dump, as always for a `plan`-side dump.
