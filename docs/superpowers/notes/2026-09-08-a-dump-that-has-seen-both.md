# A dump that has seen both — and the premise that water needed charting

2026-09-08, branch `a-dump-that-has-seen-both`, worktree
`.worktrees/water-and-oil`, master at `7faf1076`.

The task was: *seven of eight rungs of the rocket ladder refuse with "nothing
standing supplies WATER", and that is unmeasurable because no dump has charted
both water and oil — the explored dump was charted toward the oil, and water on
seed 31337 is near spawn.*

**Two of that premise's three clauses are wrong, and the third is right for a
different reason.** This note records what was measured, in the order it was
measured, so the next reader can tell which parts are data.

## 1. Water never needed charting, and every dump already had it

Read straight out of the archived dumps' `entity_graph.tile_tree`:

| dump | tiles | water tiles | nearest |
|---|---:|---:|---|
| `map-31337-t0.json` | 409,600 | 5,497 | `water` at [47, −8], **d = 47.7** |
| `map.json` | 409,600 | 5,497 | same |
| `map-31337-explored.json` | 640,000 | 42,790 | same |
| `map-31337-explored-with-categories.json` | 640,000 | 42,790 | same |

A fresh map is generated to ±320 tiles at creation and this seed's nearest
shore is at 47.7 — inside it by a factor of six. So **every dump this project
has ever taken has seen the water**, the t=0 ones included, and
`map-31337-explored.json` had seen water *and* oil since 2026-09-06.

`score-map` on the new dump agrees to the tenth: `water 48.1`, matching the
seed-scan table in `CLAUDE.md` that was sitting there the whole time.

**The charting script therefore charts the oil and nothing else.** It is
`chart_then_drill.lua`'s phase 1 with a `world.dump` at the end.

## 2. The blocker was the dump's AGE, and that is checkable in one command

`scratch/audit_dump.py` in this worktree (kept in the note below, since the
worktree dies) reads a dump and reports which of today's fields it carries.
Against the two dumps:

| field | `…-with-categories` (2026-09-07 23:44) | `map-31337-water-and-oil` (today) |
|---|---|---|
| `fluidbox_prototypes[].filter` | absent | present (`{"kind":"any"}` ×4 on a chemical plant) |
| `offshore-pump.fluid_source_offset` | absent | `{x: 0, y: −1}` |
| `FactorioTile.fluid` | absent | `{"kind":"yields","fluid":"water"}` |
| `crafting_categories` | 11 prototypes | **18 prototypes** |
| `maximum_wire_distance` / `supply_area_distance` | absent | 7.5 / 2.5 |
| `attack_range` | absent | present and **`None` for all 1,028** |

That last row is not a defect: `types.rs` says in place that *"nothing fills it
yet — `mods/BotBridge/types.lua` reads no attack parameter of any kind"*. This
measurement confirms the documented state rather than finding a new one.

**This dump is the first in the project whose tiles read `TileFluid::Yields`.**
Every archived dump reads `Unknown`, which is exactly why the by-name
`is_water()` fallback was load-bearing.

**And it explains the refusal the task was named after.** On the old dump,
`researched:rocket-silo` refuses with

```
Nothing standing on this map can be shown to supply water ... (considered and
rejected: the pipe at [45.5, -8.5], ..., the offshore-pump at [46.5, -8.5],
the boiler at [45, -5.5] -- a fluidbox that supplies SOMETHING is not a source
of water)
```

**An offshore pump was already in the plan, at the water, and was rejected** —
because nothing in that world file said what its output box yields. The list is
of the plan's own overlay (the dump has zero standing entities), so the
mechanism was: the planner sites a power plant at the shore, then cannot
attribute water to the pump it just planned. That is what
`fluid_source_offset` + `TileFluid` exist to answer.

## 3. The new dump

```
workspace/scripts/map-31337-water-and-oil.json     1.51 GB (gitignored)
fingerprint    a883ccef59bbcc6e
seed           31337, default settings, Factorio 2.1.17 Space Age
mode           --headless --bots 4 --game-speed 10 --peaceful --new
instance       workspace/headless-w.toml  (rcon 4370, game 34270, api 7570)
build          debug, master 7faf1076
script         scripts/chart_water_and_oil.lua
```

Charted: **one ring**, radius 384 — 16 planned steps, 8 surveys, **0 failures,
4,156 ticks** (the earlier oil run measured 4,159; same ring, same map).
Coverage x ∈ [−384, 415], y ∈ [−384, 415], 640,000 tiles; 42,790 water tiles;
resources `coal 853 · copper-ore 1400 · crude-oil 7 · iron-ore 2452 ·
stone 895 · uranium-ore 559`.

**Name it for what it is**: it has seen water and oil *and* carries today's
schema. It is a **mid-run dump**, not t=0 — the four bots stand at the survey
lattice points (`[±249…±255, ±249…±255]`), and `score-map` says so itself.
Those positions are identical to both explored dumps, so plans made against it
are comparable with them on that axis.

Obsolete for any fluid question: `map-31337-explored.json` (no
`crafting_categories` either) and `map-31337-explored-with-categories.json`.
Still fine as t=0 baselines: `map.json` / `map-31337-t0.json`, which no fluid
goal reaches anyway.

## 4. The ladder on the new dump — the wall MOVED, it did not open

Every goal string below, 4 bots, 600 s bound, debug build at `7faf1076`.

| goal | old dump | **new dump** |
|---|---|---|
| `researched:rocket-silo` | water wall | **sulfur is ambiguous (4 recipes)** |
| `have:rocket-silo:1` | water wall | sulfur is ambiguous (4) |
| `have:rocket-part:1` | *category `rocket-building` not runnable* | sulfur is ambiguous (4) |
| `have:low-density-structure:1` | water wall | sulfur is ambiguous (4) |
| `have:processing-unit:1` | — | **ambiguous, 39 recipes, every one `recycling`** |
| `have:rocket-fuel:1` | — | ambiguous (3) |
| `have:rocket-fuel:1:rocket-fuel` | — | **two machines run `crafting-with-fluid` and nothing chooses** |
| `have:plastic-bar:10` | — | ambiguous (9) |
| `have:plastic-bar:10:plastic-bar` | petroleum wall | petroleum wall (unchanged) |
| `have:sulfur:10` | water wall | ambiguous (4) |
| `have:sulfur:10:sulfur` | water wall | water wall (unchanged) |

Exact text of the two that still stand, from the new dump:

```
sulfur runs in chemical-plant (category chemistry), and the recipe wants 30
water -- a fluid, so it arrives by pipe rather than in a hand. Nothing standing
on this map can be shown to supply water, so there is nothing to connect the
chemical-plant to. it would come from empty-water-barrel, ice-melting,
steam-condensation (category chemistry, crafting-with-fluid), which no
character can craft
```

```
plastic-bar runs in chemical-plant (category chemistry), and the recipe wants
20 petroleum-gas -- a fluid, ... Nothing standing on this map can be shown to
supply petroleum-gas, ... it would come from advanced-oil-processing,
basic-oil-processing, coal-liquefaction, empty-petroleum-gas-barrel,
light-oil-cracking
```

Note the **"considered and rejected" list is now empty** on both. On the old
dump it named a pump. Here nothing is planned that would stand one — so the new
fields have not yet been *exercised* by a goal that reaches them, and this run
does **not** establish that the pump would now be accepted. Unknown, not fixed.

### The new wall is product ambiguity, and it grew because the dump got richer

The old dump reported `crafting_categories` for 11 prototypes; the new one for
18, adding `biochamber`, `captive-biter-spawner`, `crusher`, `cryogenic-plant`,
`electromagnetic-plant`, `foundry`, `recycler`. Every one of those makes recipes
*runnable* that were previously invisible, so the candidate set for a product
explodes: `processing-unit` now has **39 producers and all 39 are recycling
recipes** — none of which is a way to *make* a processing unit, they are ways to
destroy something that contains one.

**A named recipe does not reach the recursion.** `researched:rocket-silo` +
`have:sulfur:10:sulfur` still refuses on ambiguous sulfur, because the ambiguity
arises inside `rocket-silo`'s own expansion where nothing carries the name. So
the ladder cannot be unblocked by composing named goals at the top; the choice
has to be made where the recursion makes it.

**Two candidate fixes, both cheap, neither attempted here** (planner code was
out of scope for this session): exclude `recycling` from *production* candidates
outright, and prefer a non-Space-Age recipe when one exists. `CLAUDE.md` already
names this class — *"it is why product ambiguity is everywhere"* — but the
39-recycler case is new and is caused by the dump improving.

### Compositions

| goals | world | result |
|---|---|---|
| `gathered:crude-oil` | old | **2,117 / 314,345** |
| `gathered:crude-oil` | new | **2,117 / 314,345** (identical) |
| `gathered:crude-oil` + `produced:petroleum-gas:45:basic-oil-processing` | old | **plans: 2,295 / 353,358** |
| `gathered:crude-oil` + `produced:petroleum-gas:45:basic-oil-processing` | new | **refuses** |
| `produced:petroleum-gas:45:basic-oil-processing` alone | new | refuses (no crude source) |
| `have:plastic-bar:10:plastic-bar` + the two above | old **and** new | petroleum wall |
| `have:sulfur:10:sulfur` + `producing:automation-science-pack:6` | new | water wall |

The refusal on the new dump:

```
bot 1 owns chain ChainId(593) because its bill was sized against it, but
oil-refinery fits at [143.5, -358.5] does not hold there
```

**This is stated as a difference, not as a regression.** The two dumps are not a
single-variable comparison: their `blocked_tree` holds **65,642 boxes on the old
one and 60,804 on the new**, so the ground itself differs (units wander, trees
were mined) and the refinery's site being unavailable may be about that rather
than about any new field. What is established is that the composed petroleum
goal plans on one world and not the other; **why is unknown** and needs the same
world charted twice, or a dump diff that names what those 4,838 boxes are.

`gathered:crude-oil` reproducing to the action on both dumps is the control that
says the new fields did not perturb everything.

### `gathered:water` is not the composition, and its refusal is misleading

```
no water is charted anywhere this plan can see, so water has nowhere to come
from; the plan sees coal (853 tiles), ..., uranium-ore (559 tiles)
```

Water is charted — 42,790 tiles, nearest at 47.7. `Goal::Gathered` names a
**resource entity**, and water is a *tile*, so it is absent from
`EntityGraph::resources` and the not-charted refusal fires on an absence that
means "wrong kind of thing", not "not looked at". This is the *absent is not a
value* shape again: a lookup that cannot answer returns what a lookup answering
zero returns.

## 5. Where this leaves the oil milestone

The milestone is *a headless run that builds the rig and where petroleum appears
in the production samples*. Unchanged by this work, and this session did **not**
attempt it.

- `gathered:crude-oil` plans on the new dump at 2,117 actions / 314,345 ticks,
  same as it always did. The rig half is not the blocker.
- The composed rig+refinery goal, which is the milestone's actual goal, **plans
  on the old dump and not on the new one**. That is the next obstacle and it is
  a *scheduling/siting* refusal, not a fluid one — one rung further in than the
  brief expected.
- Above that sits product ambiguity, which is what stands between here and every
  rung of the rocket ladder, and which is planner work rather than dump work.

So: closer, and the wall that was named in the brief is gone. What replaced it
is two walls that were previously hidden behind it.

## 6. Baselines

Unmoved on `7faf1076`, debug build, 4 bots:

```
researched:automation                   map.json                 176 / 21,784
producing:automation-science-pack:6     map.json                 316 / 22,457
producing:logistic-science-pack:6       map.json                 441 / 47,478
gathered:crude-oil                      map-31337-explored.json  2117 / 314,345
gathered:crude-oil                      map-31337-water-and-oil  2117 / 314,345
```

No Rust was changed in this session, so `cargo test --workspace` was not run.

## 7. Apparatus

- `scripts/chart_water_and_oil.lua` — committed, produces the dump.
- `workspace/headless-w.toml` — the isolated instance (not the default ports).
- The field audit is small enough to restate rather than commit: read
  `entity_prototypes["chemical-plant"].fluidbox_prototypes[0]` for `filter`,
  `entity_prototypes["offshore-pump"].fluid_source_offset`, and any element of
  `entity_graph.tile_tree.elements` for `fluid`. If those three are absent, the
  dump predates 2026-09-08 and **a fluid refusal taken against it says nothing
  about the planner**.
