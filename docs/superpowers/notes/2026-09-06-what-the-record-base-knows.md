# What the record base knows: the flow graph's first falsification test

2026-09-06/07. Session `wr-rates`, branch `what-the-record-base-knows`.

The owner asked to keep loading the world-record saves and use them to improve
our understanding of the world -- "like the flow graph properly working and
maybe matching the production statistics from the save roughly".

This is the first time anything in this project has put a flow-graph number
beside a number the game reported. Until now the graph had no whole-base reader
at all, so its rates could not be validated by any caller -- the same shape that
let `method::connect`'s geometry defect survive four reviews.

**Everything below is on the 6:39:53 Space Age rocket-launch save**, loaded into
the isolated `workspace/wrload` instance (ports 4390/34290), Factorio 2.1.17, at
`game.tick` ~1,447,000. Every game-side number came from read-only RCON queries
against that running instance. Every model-side number came from
`FlowGraph::production_rates()` over `workspace/wrload/scripts/wr-census.json`,
a `world.dump` of the same save.

---

## The headline, and it is the opposite of what was expected

The brief said the honest expected result was that **our prediction understates
the save**, because a world-record base is beaconed and moduled and our model is
neither.

**It does not understate. It over-predicts, on every high-volume item, by
8% to 32%** -- and the reason is that a 6:39 Space Age speedrun base is *not* a
beaconed megabase. It is built fast and cheap:

- **every one of its 1,222 furnaces carries a `speed_bonus` of 0.000 and a
  `productivity_bonus` of 0.000**;
- **every one of its 559 iron drills carries a `speed_bonus` of 0.000**, and
  the whole 1,544-drill fleet averages 0.005;
- of 2,226 modules on Nauvis, **not one sits in a furnace and not one sits in a
  mining drill** (26 are in pumpjacks). They are in assemblers, chemical plants,
  biolabs, the rocket silos, centrifuges, and 498 in the 249 beacons.

So the module and beacon blind spot -- the candidate the brief named first --
**costs 0% on plates and 0% on ore**. What our model actually lacks is
**idleness**: 87% of the furnaces and 82% of the drills are working at any
instant, and the flow graph has no notion of a machine standing still.

---

## 1. What we can see: 98% of the base, not 12%

The previously recorded figure was "321,041 entities within 1,000 tiles of
spawn, and the model holds 39,237 -- about 12%". **The 12% is an artefact of the
denominator.** Counted by type over the same disc:

| within 1,000 tiles of spawn | count | share |
|---|---:|---:|
| **resource tiles** (ore entities) | 270,844 | 82.9% |
| trees | 10,746 | 3.3% |
| fish | 3,198 | 1.0% |
| rocks (`simple-entity`) | 1,712 | 0.5% |
| cliffs | 0 | 0% |
| **player-force built entities** | 39,974 | 12.2% |
| everything else | 251 | 0.1% |
| total | 326,725 | |

Ore tiles are not missing from our model: they live in `EntityGraph::resources`,
a `Pos`-keyed map, not in `entity_tree`, so a comparison against `entity_tree`
was never going to find them. Trees, rocks and fish are deliberately not
admitted.

Against the right denominator -- **the built base on Nauvis** -- the model holds
**39,184 of 40,000, or 98.0%**. And of the 816 it does not hold, **763 are
entities of types `EntityGraph::add`'s whitelist deliberately excludes**:

```
beacon 249 · logistic-robot 131 · entity-ghost 123 · roboport 117 · pump 90
cargo-bay 14 · cargo-pod 12 · rocket-silo 12 · rocket-silo-rocket 12
radar 2 · cargo-landing-pad 1                                    = 763
```

So of the entities the whitelist *admits*, the model holds **39,184 of 39,237 --
99.9%**. This is not an ingest ceiling and not a chunk-replay problem. **It is a
filter, and the filter is doing what it says.**

Two limits worth naming beside that:

- **The model holds one surface.** The base spans nine: Nauvis 39,951 entities,
  Fulgora 1,774, Aquilo 1,435, Vulcanus 1,399, Gleba 1,377, five platforms 261 --
  **46,197 in total, of which Nauvis is 86.5%.** For iron plate, copper plate and
  green circuits this costs about 1% (Nauvis is 15,060 of 15,209 iron plate/min),
  because the other planets make holmium, tungsten and bioflux, not plates. For
  anything off-Nauvis it costs everything.
- **249 beacons are invisible by construction**, so even if we wanted to model
  beacon effects the entities carrying them are not in the graph.

### A correction: "no ore found under miner" is not an ingest failure

The load logs 86 `no ore found under miner electric-mining-drill` warnings. The
game reports **72 drills with `status = no_minable_resources`** and 1,472 of
1,544 with a live `mining_target`. Those are depleted patches, not lost ingest.

---

## 2. What the save actually produced

`LuaFlowStatistics.get_flow_count` returns a value **already normalised to
per-minute** for item statistics (per-tick only for electric networks). An early
pass here multiplied by 3,600 and got 59 million iron plates a minute; the
lifetime total is the check that catches it -- 5,496,840 plates over 1,447,190
ticks is 13,674/min, and the ten-minute rate is 15,247/min. Consistent.

`input_counts` is **production** and `output_counts` is **consumption** -- the
statistics GUI's left/right, not the intuitive reading of the words.

Nauvis, ten-minute precision:

| item | made /min |
|---|---:|
| copper-cable | 22,367 |
| iron-ore | 15,247 |
| copper-ore | 15,157 |
| iron-plate | 15,170 |
| copper-plate | 15,147 |
| electronic-circuit | 6,694 |
| coal | 4,096 |
| plastic-bar | 2,846 |
| stone | 1,775 |
| steel-plate | 1,213 |
| advanced-circuit | 942 |
| iron-gear-wheel | 578 |
| stone-brick | 450 |
| processing-unit | 249 |

---

## 3. Predict, compare, attribute

`FlowGraph::production_rates()` is new: the whole-base aggregate the graph never
had. It counts only the four arms of `update()` that *originate* a rate -- mining
drills, furnaces, assembling machines, offshore pumps -- and takes the **maximum**
per item across a producer's outgoing edges rather than the sum, because
`update_flow_edge` writes a machine's whole output on *each* of its outgoing
edges.

| item | game /min | model /min | model / game |
|---|---:|---:|---:|
| copper-cable | 22,367 | 22,680 | **1.01** |
| plastic-bar | 2,846 | 2,880 | 1.01 |
| stone | 1,775 | 1,740 | 0.98 |
| copper-ore | 15,157 | 15,870 | 1.05 |
| iron-ore | 15,247 | 16,500 | 1.08 |
| advanced-circuit | 942 | 1,035 | 1.10 |
| copper-plate | 15,147 | 16,580 | 1.09 |
| iron-plate | 15,170 | 19,016 | 1.25 |
| steel-plate | 1,213 | 1,590 | 1.31 |
| electronic-circuit | 6,694 | 8,820 | 1.32 |
| processing-unit | 249 | 360 | 1.45 |
| stone-brick | 450 | 1,454 | 3.23 |
| iron-gear-wheel | 578 | 2,070 | 3.58 |

(The `copper-plate` and `stone-brick` columns are **after** the furnace fix in
§4; before it they were 19,425 and 7,125, i.e. 1.28 and **15.8**.)

### Iron ore: the 8% closes exactly, and it is two errors of ~10% not cancelling

The game makes **15,247 ore/min from 559 drills = 0.4545 ore/s per drill**. A
nominal electric mining drill on iron is `mining_speed 0.5 / mining_time 1.0` =
0.5/s. So:

| factor | multiplier on our prediction | measured from |
|---|---:|---|
| coverage: 550 of 559 iron drills modelled | ×0.984 | census vs `mining_target` census |
| force mining productivity **+10%**, not modelled | ×0.909 | `mining_drill_productivity_bonus` |
| duty cycle **82.6%**, we assume 100% | ×1.211 | 0.4545 / (0.5 × 1.1) |
| modules and beacons on iron drills | ×1.000 | `speed_bonus` 0.000 across all 559 |
| **product** | **×1.083** | observed 1.082 |

The arithmetic closes to a tenth of a percent. **Read the near-agreement as a
warning, not a validation**: a −9% productivity blind spot and a +21% idleness
blind spot happen to leave 8%.

### Iron plate: idleness and an over-count, modules contribute nothing

The game has **468 furnaces set to `iron-plate`**, all of them `steel-furnace`
(`crafting_speed` 2) on a 3.2 s recipe = 0.625 plate/s each, so **17,550/min of
standing capacity** against 15,170 made: **86.4% duty**.

| factor | multiplier | measured from |
|---|---:|---|
| the model gives an iron edge to 507 furnace-equivalents, not 468 | ×1.083 | 19,016 / (0.625 × 60) |
| duty cycle 86.4% | ×1.157 | 15,170 / 17,550 |
| coverage: 1,221 of 1,222 furnaces | ×0.999 | census |
| modules and beacons | ×1.000 | `speed_bonus` **0.000**, `productivity_bonus` **0.000**, all 1,222 |
| **product** | **×1.252** | observed 1.253 |

### Green circuits: the only place modules move the number, and they move it 3%

98 `assembling-machine-2` are set to `electronic-circuit`; the model's 8,820/min
is exactly 98 × (1 × 0.75 / 0.5) × 60, so the machine count is right.

- modules: those 98 carry `speed_bonus` **−0.10** and `productivity_bonus`
  **+0.08** -- one productivity module 1 each. Net ×0.972.
- duty: 75 of 98 working = ×0.765.
- 8,820 × 0.972 × 0.765 = **6,559** against **6,694** measured: within 2%.

So the module blind spot here is **−3%** and idleness is **−23%**.

### Copper cable: the match that means nothing

130 `assembling-machine-2` on `copper-cable`, `speed_bonus` **+0.12** (beacons),
`productivity_bonus` +0.03, 107 of 130 working. Predicted 22,680, measured
22,367 -- **1.4% apart**, the best agreement in the table.

It is a coincidence. 22,680 × 1.12 × 1.03 × 0.823 = 21,530: the beacon speed and
the productivity module *nearly exactly cancel the 18% idle*. This is the case
the brief warned about -- **a number that matches exactly is more suspicious than
one that is off by 3x** -- and here it is, measured.

### The gap, by cause, with each part's size

| cause | size on this base | bounded? |
|---|---|---|
| **coverage** | 1.6% on ore, 0.1% on furnaces, ~1% from the eight other surfaces | **yes, and small** |
| **modules and beacons** | **0% on plates and ore**, −3% on green circuits, −15% on copper cable | **yes** -- 2,226 modules censused, none in a furnace or ore drill |
| **force bonuses** | −9% on ore (mining productivity +10%); 0% on plates. Lab speed +2.5 and robot speed +2.4 touch nothing we predict | **yes**, one factor |
| **idle time / back-pressure** | **+21% on ore, +16% on plates, +23% on circuits** -- the dominant term everywhere | **no.** Nothing in the model represents it |
| **the flow graph's hard-coded rates** | already fixed before this session; the WR base is 98% `steel-furnace`, so the old `1/3.2` would have been **2.0x low** on every plate | -- |
| **the furnace's per-input double emission** | **up to 15.8x** (stone-brick), 28% (copper plate) | found here, fixed in §4 |

Two of the four candidates the brief named are **bounded and small**. One is
bounded and one-sided. The fourth -- idleness -- is unbounded and is what a
next piece of work should attack.

---

## 4. What the test found and what was fixed

### `smelting_output` was already right

The `1/3.2` hard-coding the CLAUDE.md entry describes had already been fixed in
an earlier session: `smelting_output` derives
`product.amount * crafting_speed / recipe.energy` from the game's own tables, and
three tests hold the derivation rather than the constants. This base is the
verification that was missing: **1,196 of its 1,222 furnaces are `steel-furnace`
(`crafting_speed` 2)**, so the old code would have reported every plate at
**exactly half rate** -- 8,775/min against a measured 15,170.

### A furnace ran every recipe at once

`update()`'s furnace arm added a **full-rate** output edge for **every**
smeltable input arriving. On a mixed belt that reports one furnace smelting iron
*and* copper *and* stone at 100% of its rate simultaneously.

Measured on this base: **`stone-brick` came out at 7,125/min against the game's
450**, a factor of 15.8, because stone reaches furnaces the game has set to iron.
`copper-plate` came out 28% high.

Fixed: the outputs now **share the furnace's time**, weighted by each input's
share of the smeltable ore arriving. A single-input furnace is unchanged (share
1.0); a mixed one now sums to exactly one furnace's capacity.

| after the fix | before | game |
|---|---:|---:|
| stone-brick | 7,125 → **1,454** | 450 |
| copper-plate | 19,425 → **16,580** | 15,147 |
| iron-plate | 19,312 → **19,016** | 15,170 |

The remaining 3.2x on `stone-brick` is idleness (36 brick furnaces at 33% duty)
plus a residual over-count; it is a small column and was not chased further.

### `production_rates()` -- the reader the file never had

`FlowGraph::production_rates() -> BTreeMap<String, f64>`. Three tests, each
falsified once with the substitution confirmed to match exactly once:

- `belts_and_inserters_carry_a_rate_and_do_not_add_to_it` -- fails when
  `TransportBelt` is admitted as a producer.
- `a_drill_feeding_two_things_is_counted_once` -- fails when the per-node
  maximum becomes a sum.
- `a_furnace_fed_two_ores_still_only_runs_one_at_a_time` -- fails when the time
  share reverts to a full edge per input.

And `production_rates_of_a_dumped_world`, `#[ignore]`d and gated on
`FACTORIO_BOT_WORLD_DUMP`, is the harness this note was written from:

```bash
FACTORIO_BOT_WORLD_DUMP=workspace/wrload/scripts/wr-census.json \
  cargo test -p factorio-bot-core --release --lib \
  production_rates_of_a_dumped_world -- --ignored --nocapture
```

31 seconds against a 2.8 GB dump, no Factorio required.

---

## What this says to do next

1. **Idleness is the whole remaining gap.** Modules, beacons, force bonuses and
   coverage together account for about a tenth of it; duty cycle accounts for the
   rest, and the graph is structurally blind to it because every rate is computed
   forwards from a source with nothing flowing backwards. Back-pressure is the
   next modelling piece, and this save is the oracle to test it against -- it has
   194 drills reading `waiting_for_space_in_destination` right now.
2. **Do not build module or beacon modelling first.** It is measurably worth
   0% on the columns that carry the volume in a speedrun base. It matters for
   circuits and for anything a megabase does, not for us yet.
3. **A WR base is a cheap base.** Unmoduled smelting, unmoduled mining, 87% duty.
   That is a fact about what the record actually optimises, and it is the sort of
   transferable lesson the reference saves are for.
