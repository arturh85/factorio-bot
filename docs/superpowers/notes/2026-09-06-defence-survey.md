# Defence: a survey before anything is built

**2026-09-06, branch `defence-survey` from `bda147a8`. A survey, not an
implementation — no Rust, Lua or TypeScript was changed, no Factorio process
was started, no cargo was run.** Everything below is from the game's own
prototype data, the archived run records, the source, and one offline
`score-map` against `workspace/scripts/map.json`.

The roadmap names defence as item 4 (`docs/superpowers/plans/2026-09-03-closing-the-idle-gap.md:567-571`):
*"Production makes pollution, pollution brings biters, and every run so far
ends before that matters."* This note establishes how far away "matters"
actually is, what the system can and cannot see, and what the cheapest first
move is.

**The headline, stated up front so the rest can be read as evidence for it:**

> **Pollution is not the near-term threat. Walking is.** At the emission rate
> our largest run actually achieved (~38 pollution units/minute, measured
> indirectly — see below), the cloud does not come close to the nearest nest
> on seed `31337`, which is **more than 253 and at most 520 tiles** from
> spawn. But bots have already walked **156.5 tiles** from spawn, exploration
> is a named roadmap item, and a spawner's `call_for_help_radius` is **50
> tiles**. The gap between "furthest a bot has been" and "nearest nest" is
> under 100 tiles and the roadmap is actively trying to close it.

---

## 1. When does it actually start to bite?

### What our machines emit (from the game's own prototypes)

`workspace/data/base/prototypes/entity/` — `emissions_per_minute.pollution`, at
100% energy draw:

| entity | PU/min | placed by our runs? |
|---|---|---|
| `boiler` | **30** | yes (1) |
| `burner-mining-drill` | **12** | yes (up to 10) |
| `burner-generator` | 10 | no |
| `electric-mining-drill` | **10** | not yet (roadmap) |
| `pumpjack` | 10 | no |
| `oil-refinery` | 6 | no |
| `assembling-machine-1` | **4** | yes (4) |
| `steel-furnace` | 4 | no |
| `chemical-plant` | 4 | no |
| `centrifuge` | 4 | no |
| `assembling-machine-2` | 3 | no |
| `stone-furnace` | **2** | yes (up to 27) |
| `assembling-machine-3` | 2 | no |
| `electric-furnace` | **1** | not yet (roadmap) |

`lab`, `steam-engine`, chests, poles, belts and inserters have no
`emissions_per_minute` at all. **Hand mining and hand crafting emit nothing** —
there is no emission on the character prototype — which matters a lot here,
because CLAUDE.md's own counter note records that ~40% of all ore in a run is
hand-mined.

Note `electric-furnace` is 1 and `stone-furnace` is 2: **electric smelting, one
of the stated next targets, halves smelting pollution per machine** — but it
arrives with `electric-mining-drill` at 10 (vs. burner at 12, barely better)
and with far more of everything running at once.

### What our largest run actually built and produced

The biggest factory any archived run has stood up is
`workspace/runs/run-1788635061-85457` (seed `31337`, digest `c161fa3f437221d0`,
4 clients, 1x). Its final `machines` sample at tick 60,300 (**16.75 minutes of
game time**):

```
stone-furnace 27  burner-mining-drill 10  assembling-machine-1 4
lab 2  boiler 1  steam-engine 1  iron-chest 8  wooden-chest 2
```

Flat out, that is `27*2 + 10*12 + 4*4 + 1*30` = **220 PU/min**. Nothing ran
flat out — most furnaces sample as `no_ingredients`, and drills stop after one
fuel load.

So estimate from *output* instead. The same run's final `force` sample:
670 iron plates, 189 copper plates, 859 ore, 85 red science, 203 gears,
204 cable, 66 circuits, 34 inserters, and a network drawing 164 kW of 900 kW.

| source | arithmetic | PU |
|---|---|---|
| smelting | 859 plates × 3.2 s ÷ 60 × 2 PU/min | **92** |
| drilling | ≤859 ore × 4 s (0.25 mining speed) ÷ 60 × 12 PU/min, ×~0.6 for the hand-mined share | **~410** (upper bound 687) |
| assembling | ~1,270 machine-seconds at 0.5 crafting speed ÷ 60 × 4 PU/min | **≤85** |
| boiler | 30 PU/min × (164 kW ÷ 900 kW capacity) × ~10 min present | **~55** |
| **total** | | **~640 PU over 16.75 min ≈ 38 PU/min** |

**Assumptions, stated:** that emissions scale linearly with energy draw (they
do in 2.0); that ~60% of ore came from drills rather than hands; that the
boiler stood for the last ~10 minutes at its measured load. The honest range is
**25–60 PU/min average, 220 PU/min if everything ran continuously**.

### How far the nearest nest is on seed 31337

**Between 253 and 520 tiles.** Derived from `vision_measured` events across
every archived run on map digest `c161fa3f437221d0`:

- largest model radius with **zero** enemy structures: **253.0 tiles**
- smallest model radius with **any** enemy structures: **519.9 tiles**
- at full model extent (592 tiles) the model holds **64** enemy structures

The bracket is coarse because the model grows by whole generated chunks, not
smoothly. The exact figure is **not recorded anywhere** — see the query that
would settle it, at the end.

For comparison, the *old* map's nearest nest is documented precisely in
`crates/core/src/graph/entity_graph.rs:236-238`: 36 spawners, 28 worm turrets,
nearest at `(-237.5, 66.5)` = **246.6 tiles Euclidean**. Commit `2bf76bd7`
exists because that number was previously computed with a Manhattan method
wearing a `distance` name.

**The furthest any bot has ever walked is 156.5 tiles**
(`travelled_tiles` across all runs; typical runs 62–143).

### The mechanism, and the estimate

`workspace/data/base/prototypes/map-settings.lua:11-32` and
`entity/enemies.lua`:

- a chunk must hold **15 PU** before it diffuses at all (`min_to_diffuse`)
- it then sends **2%** per neighbour per second (`diffusion_ratio = 0.02`)
- tiles absorb continuously (`ageing = 1`); trees absorb heavily
- a spawner absorbs **20 PU/s absolute + 1% proportional**
  (`enemies.lua:174`) — so absorption at the nest is never the bottleneck
- a **small biter costs only 4 PU** to join an attack
  (`absorptions_to_join_attack`, `enemies.lua:103`); medium 20, big 80,
  behemoth 400

So the entire question is *does the cloud reach a nest chunk at all*, and once
it does, attacks follow almost immediately and cheaply.

**Estimate: at today's scale, never.** ~38 PU/min spread over the four or five
chunks the base occupies produces a cloud that stabilises well inside 253
tiles — roughly 2–4 chunks (64–128 tiles) by the usual vanilla behaviour of a
burner-era base. To push a cloud 8+ chunks you need an order of magnitude more
emission sustained for tens of minutes: **on the order of 500–1,500 PU/min for
45–90 minutes of game time.** That is exactly the regime the next three roadmap
targets (a factory that sustains a rate, electric smelting, self-expanding
solar) put us in — a few dozen electric drills at 10 PU/min each gets there on
its own.

Evolution meanwhile stays negligible: `time_factor = 4e-6`/s and
`pollution_factor = 9e-7`/PU, so an hour of play plus 640 PU leaves evolution
well under 0.2 — **small biters only**, which is the cheapest possible enemy to
defend against and the best possible time to learn how.

**This estimate is arithmetic, not measurement, and it does not have to stay
that way** — see §4 and the dispatch.

### Does anything in the record carry pollution? No.

**Finding, and it is the reason §1 is an estimate:** the `force` sample carries
exactly `research`, `techs_unlocked`, `production`, `power`
(`mods/BotBridge/control.lua:2321-2358`,
`crates/core/src/record/samples.rs:82-88`). `grep -ril "pollution\|evolution"`
over `workspace/runs/` and `workspace/headless-*/runs/` — **44 runs, every one
of them — returns nothing.** The strings do not occur in the Rust crates or in
the mod either. Pollution and evolution are invisible to this system end to
end.

---

## 2. What the system can and cannot see

**Enemy structures: yes, and better than expected.**
`EntityGraph` has a `threats: DashMap<String, BTreeMap<Pos, Position>>` field
(`crates/core/src/graph/entity_graph.rs:271`), fed by a constant rather than
the `EntityType` whitelist:

```rust
pub const ENEMY_STRUCTURE_TYPES: [&str; 2] = ["unit-spawner", "turret"];
```

(`entity_graph.rs:282`, ingest at `:1318-1323`). It is deliberately placed
*before* the `EntityType::from_str` whitelist, because that enum
(`crates/core/src/types.rs:1741-1781`) has no military variant — which is why,
before this landed, every nest survived only as an anonymous rectangle in
`blocked_tree`. Nests round-trip through `world.dump`
(`entity_graph.rs:2223`), so an offline plan sees them.

**Live biters: deliberately not.** `entity_graph.rs:257-263` — "a unit walks,
so its position is true for the tick it was serialised in and a lie
thereafter."

**Force ownership: not at all.** `serialize_entity`
(`mods/BotBridge/types.lua:478-479`) sends `name`, `direction`, `type`,
`position`, `drop_position`, bounding box and inventories — **no `force`, no
`health`, no `unit_number`**. `ENEMY_STRUCTURE_TYPES` is the only thing that
makes an entity "enemy", and it works only because the player's own turrets are
`ammo-turret`/`electric-turret`/`fluid-turret` and do not match `"turret"`.

**Pollution, evolution, unit positions, attack events, entity health: none.**

**The threat API exists and nothing calls it.** Three methods —
`threats_from` (`entity_graph.rs:517`), `nearest_threat` (`:542`),
`threat_census` (`:552`). Repo-wide, **the only callers are that module's own
tests** plus prose in two design docs. Nothing in `crates/planner`,
`crates/executor`, `crates/scripting_lua`, `crates/server`, `app/` or
`score-map` reads a threat. The Manhattan/Euclidean fix (`2bf76bd7`) corrected a
number that no production code consumes.

Its own docs are careful about the asymmetry (`entity_graph.rs:265-270`): *"A
nest in ground nobody has looked at is absent from this map, and absence here
is never evidence of safety."* Confirmed in practice — the seed-`31337` t=0
dump `workspace/scripts/map.json` has `"threats": {}` while the same map at
full model extent holds 64 enemy structures.

**What a run does record:** `EventKind::VisionMeasured` carries
`model_enemy_structures` (`crates/core/src/record/mod.rs:984`) and, when the
furthest model entity happens to be a nest, its position — that is the only
enemy coordinate in any record. And `EventKind::BotDied { bot, position, cause,
cause_type, respawn_in }` (`mod.rs:802-817`). Exactly **one** bot death exists
across all 44 archived runs (`run-1788556166-27937`, tick 85,828, at
`(20.4, -52.3)` — right next to base, `cause: null` under the old schema, so it
was almost certainly not a biter). The mod's own comment
(`control.lua:3324`) says the death-event field shapes come from the API docs,
not from evidence.

**One live door nobody uses:** the mod's `find_entities_filtered` passes the
filter table straight through (`control.lua:5168-5175`), so
`{force = "enemy", type = "unit-spawner"}` is answerable over RCON today. The
Rust wrapper (`crates/core/src/factorio/rcon.rs:5037-5042`) only ever sets
`area_filter`, `search_name`, `search_type` — it never sets `force`.

---

## 3. What the system cannot do

The planner's whole verb vocabulary is nine `ActionKind` variants
(`crates/planner/src/action.rs:599-689`): `Mine`, `Chop`, `Craft`, `Place`,
`Insert`, `Remove`, `Research`, `SetRecipe`, `Evacuate`. Walking is not a verb;
it is emitted to satisfy an `AtPosition` precondition.

| defence capability | verdict |
|---|---|
| **craft `stone-wall` / `gun-turret` / `firearm-magazine` / `repair-pack`** | **already works.** No whitelist gates any recipe — `HandCraft` admits anything in the `crafting` category (`method/have.rs:3214-3226`), and a military-tech-gated recipe emits `Goal::Researched` as a subgoal automatically. `have:gun-turret:2` would plan today. |
| **place a wall or turret** | **existing verb, missing decision.** `ActionKind::Place` carries a whole `FactorioEntity` and names nothing; `rcon_place_entity` resolves any item with a `place_result` (`control.lua:3869`). `Goal::Built` → `blueprint::BuildBlock` already builds an arbitrary blueprint *and* states its bill as `Have` subgoals. What is missing is anything that **chooses an anchor** — every existing `Place` emitter hardcodes its entity. |
| **feed a turret ammunition** | **one enum variant.** `Insert` is entity- and item-generic; the mod hands `inventory_type` straight to `get_inventory`. `InventorySlot` (`action.rs:549-593`) has no `TurretAmmo` — yet `turret_ammo` is already in the executor's defines snapshot (`rcon_actuator.rs:115`) and in the mod's legacy name map. From Lua it works today, since `rcon.insert_to_inventory` takes a raw numeric index. |
| **repair** | **genuinely new, end to end.** No `ActionKind`, no `Actuator` method, no RCON binding, no mod function, and no entity-health state in `PlanState`. |
| **shoot / choose to fight** | **genuinely new, end to end.** No `shooting_state` anywhere; `character_bot_proxy` (`control.lua:5726-5727`) deliberately covers walking, mining, position, force and inventory and **not** guns. |
| **flee / cancel a walk in flight** | **half-present, and the half that exists is unreachable.** `ActionKind::Evacuate` is a real "walk clear and stand there" verb, pinned to its one caller in `enclosure.rs`. There is **no `action_stop_walk` RCON** — mining has a `"stop"` cancel (`control.lua:3849`), walking does not; a new walk silently overwrites the old one, leaving the previous `action_id` with no verdict. And nothing on the Rust side can interrupt a running action: dispatch awaits the actuator's reply or a deadline. Fleeing needs a mod-side cancel *and* an executor-side pre-emption path. |

The reactive half is already built and has never fired: `on_player_died`
writes the event and `fail_bot_actions` (`control.lua:3377`) fails the in-flight
walk, mine and every craft waiter. The executor's four recovery tiers
(`crates/executor/src/recover.rs`) have **no tier for "a bot is under attack"**.

---

## 4. The cheapest thing that would change the outcome

Ranked by cost against benefit. "Cost" is honest about which crate moves.

1. **Sample pollution and evolution.** *Mod + one record struct.* Two calls
   verified against `workspace/factorio-api-docs/runtime-api.json`:
   `LuaSurface.get_total_pollution()` / `get_pollution(position)`,
   `LuaGameScript.get_pollution_statistics(surface)` (a `LuaFlowStatistics`
   with the **per-prototype** breakdown, which would replace all of §1's
   arithmetic with measurement), and `LuaForce.get_evolution_factor(surface)`
   plus its `_by_time` / `_by_pollution` / `_by_killing_spawners` split. This is
   the cheapest item on the list by a wide margin and it is the only one that
   turns an estimate into a number. **Highest ratio.**
2. **Disclose the threats we already hold.** *Rust only, read-only.* Put
   `threat_census()` into `provenance.json`'s `map` object and `nearest_threat`
   into `score-map`'s output. Zero new perception; it makes three unused
   functions load-bearing and makes "is this map dangerous" answerable offline.
   Cheap, and a prerequisite for everything below.
3. **Avoid known nests when walking.** *Planner.* `threats_from` already
   returns sorted `(name, position, distance)`. A keep-out radius that costs or
   refuses a destination inside it is a scoring change, not a new verb — and it
   defends against the threat that is actually near-term (§1's headline).
   Depends on (2) being honest about "empty means unknown, not safe".
4. **Gun turrets with ammunition.** *Planner + one `InventorySlot` variant.*
   Crafting works; placing works; the missing pieces are `TurretAmmo` and a
   method that sites a turret. Only worth it once (1) says pollution is
   climbing.
5. **A wall line.** *Planner, and the most expensive planner work here.* Walls
   craft and place today, but a *line* needs a route, and `enclosure.rs` exists
   precisely because sealing a footprint can trap a bot — `Evacuate` was
   invented for that. High cost, and against small biters a turret beats a wall.
6. **Pollution-aware siting.** *Planner, and it wants (1) first.* Attractive in
   principle — put the factory where it provokes less — but siting is already
   contested by ore distance and walk score, and with the nearest nest 253+
   tiles away there is nothing to gain until emissions are an order of
   magnitude higher.
7. **Bots that flee.** *Mod + executor + planner.* Needs a walk cancel, an
   interrupt path through dispatch, and a decision. The most plumbing of
   anything on this list. Defer.

---

## 5. The enemy use case

The owner wants a bot able to play as **enemy** in a human's game. Beyond
defence it needs almost nothing conceptually new — it is the same perception
with the opposite intent:

- **the same perception, pointed the other way.** Instead of "where are the
  nests", "where are *their* structures" — and that is a strictly easier query,
  because a human's factory is charted, static and force-tagged.
- **the one real gap is force.** Nothing in this system knows which force owns
  an entity: the mod never serialises `entity.force` and `FactorioEntity` has
  no field for it. An adversarial bot must distinguish its own belt from the
  target's, and today it cannot. **This is the single change that unlocks the
  enemy case**, and it is one field in `types.lua:478` plus one in
  `crates/core/src/types.rs`. It would also make `ENEMY_STRUCTURE_TYPES` an
  honest filter instead of a lucky one.
- **combat verbs, which defence needs anyway** — the shoot/target verb from §3
  is the same verb whether it is aimed at a biter or a player's power pole.
- **nothing in the design forbids it.** The executor's stated constraint is
  that it issues only *legitimate player actions* — no `cheat_*` — and shooting
  a gun is a legitimate player action. `crates/scripting_lua/src/sandbox.rs`
  restricts the filesystem, not the game. The mod's action family is small but
  not principled about non-violence.

Worth stating plainly: an enemy bot is **cheaper** than a defending bot, because
it does not need pollution modelling, retreat, or repair — only perception,
pathing to a target, and one attack verb.

---

## Recommended first dispatch

**Add pollution and evolution to the `force` sample.**

Mod: in `sample_force_body` (`mods/BotBridge/control.lua:2321-2358`) add
`surface.get_total_pollution()`, `surface.get_pollution(spawn)`,
`force.get_evolution_factor(surface)` and the three `_by_*` components, plus the
`game.get_pollution_statistics(surface)` input table (the per-prototype
breakdown). Rust: two or three **optional** fields on `ForceSample`
(`crates/core/src/record/samples.rs:82-88`), so every archived run still parses.
`tools/run_analysis.py`: report the peak and the final value.

**Why this one.** It is the smallest change on the list, it touches nothing the
planner or executor depends on, it cannot regress a run, and it converts §1 —
the part of this survey that is arithmetic rather than measurement — into a
recorded number on every future run, including the long ones the roadmap is
about to start producing. Everything else in §4 wants to know that number
first.

**Acceptance test, as a sentence someone can check:** *after any run of any
length, `workspace/runs/<run>/samples.jsonl` contains `force` samples carrying a
non-negative `pollution_total` that rises while machines are running and an
`evolution` value, `just analyse` prints the peak of each, and an archived run
recorded before the change still loads without error.*

---

## Where I was uncertain, and the query that settles it

1. **The nearest nest on seed `31337` is bracketed at 253–520 tiles, not
   known.** The bracket comes from `vision_measured`'s
   `model_enemy_structures` crossing zero between two model radii, which is
   chunk-granular. **The query:** against a resumed savepoint (e.g.
   `workspace/runs/run-1788641738-65147/savepoints/milestone-1.zip`), one RCON
   call —
   `factorio-bot rcon -s localhost -- '/c rcon.print(serpent.line(game.surfaces[1].find_entities_filtered{force="enemy", type="unit-spawner", position={0,0}, radius=600, limit=5}))'`
   — or, once item (2) of §4 lands, `score-map` printing `nearest_threat`
   straight off the dump. I did not run it: the instructions forbid starting a
   server tonight, and three other agents share this box.
2. **The pollution figure (~38 PU/min) is arithmetic from production counts,
   not a measurement**, and its largest term (drilling, ~410 of ~640 PU) rests
   on an assumed 60/40 drill-to-hand split. **The query:** the dispatch above —
   `game.get_pollution_statistics(surface).input_counts` gives the per-prototype
   truth directly.
3. **"The cloud stabilises inside 253 tiles" is a judgement, not a
   simulation.** The diffusion constants are quoted so the reasoning is
   checkable, but I did not model tile absorption. **The query:**
   `surface.get_pollution({x, y})` sampled on a ring at 64/128/256 tiles during
   a long run — which is the same mod change as the dispatch, one extra line.
4. **The single archived bot death has `cause: null`** and cannot be attributed.
   No query will recover it; the schema that would have answered it postdates
   the run.
