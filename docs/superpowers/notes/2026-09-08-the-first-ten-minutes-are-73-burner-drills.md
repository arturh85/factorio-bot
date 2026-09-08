# The first ten minutes are 73 burner drills

**The whole rate curve of a finished run is recoverable from its end-state
save.** `LuaFlowStatistics.get_flow_count` takes `sample_index` 1..300 and every
precision level holds 300 samples, so `ten_hours` is 300 windows of **2 minutes**
— the entire run of anything under 10 h, at 2-minute resolution, for items,
entities built, fluids and power. Our mod never used it. Apparatus:
`tools/rate_history_probe.lua`, `tools/rate_history_power_probe.lua`; raw output
in `notes/data/2026-09-08-*.csv`.

Validated: summing the 200 `ten_hours` samples of iron-plate gives 5,454,896
against a lifetime `get_input_count` of 5,462,637 — 99.86%, the remainder being
the nine non-Nauvis surfaces. **Nauvis only** below; timings are ±2 min.

Two references, both Space Age (`base`, `space-age`, `elevated-rails`,
`quality`), both loaded in `workspace/wrload`:
`any-wr-6-39-53.zip` (tick 1,440,582, 10 surfaces) and `RSNG_3_17_06.zip`
(tick 715,870, **15 surfaces, 161 technologies** — all four planets plus Aquilo,
so a *different and harder* category finished in half the time).

## The curves, at game-time marks

iron-plate /min. Ours = the best archived t=0 runs (4 headless character bots,
seed 31337, `run-1788650660-97962` / `-91919` / `-64852`), and **every mark of
every one of them is `roster-fed`** — bots hand-loading stone furnaces.

| min | any% 6:39:53 | RSNG 3:17:06 | ours | ratio |
|---:|---:|---:|---:|---:|
| 5 | 261 | 222 | 51 | 5x |
| 10 | 390 | 224 | 67 | 5.8x |
| 15 | 840 | 581 | 13–27 | 40x |
| 20 | 1,425 | 1,213 | — | — |
| 30 | 1,726 | 2,456 | — | — |
| 60 | 5,103 | 6,997 | — | — |
| 120 | 15,632 | 63,870 | — | — |

Copper: 90 / 138 / 165 / 163 / 422 (any%) against ours 19 / 19 / 0.
**Steel: first at minute 42–45 in both references; 0 in all 78 of our runs.**
Power (any%, main network, avg MW): 0.05 / 0.90 / 4.89 / 7.62 / 19.2 at
5/10/15/20/30 min, against our flat 900 kW generated and ≤164 kW drawn, first
generation at 6:23–11:28 and never grown.

## What they built, and when — the two runs agree to within 2 minutes

Cumulative built, any% / RSNG:

| | first | 5m | 10m | 15m | 20m | 30m |
|---|---|---:|---:|---:|---:|---:|
| burner-mining-drill | 2m / 3m | 44 / 24 | **73 / 73** | 73 / 73 | 73 / 73 | 73 / 73 |
| stone-furnace | 2m / 3m | 28 / 25 | 39 / 36 | 103 / 82 | 103 / 97 | 150 / 250 |
| offshore-pump+boiler+steam-engine | 8m / 9m | 0 | 1+1+1 | 3+6 | 15+31 | 15+31 |
| lab | 8m / 9m | 0 | 2 / 1 | 6 / 4 | 6 / 4 | 54 / 4 |
| assembling-machine-1 | 10m / 11m | 0 | 11 / 9 | 18 / 13 | 26 / 28 | 63 / 105 |
| inserter | 10m / 13m | **0** | 15 / 0 | 94 / 55 | 193 / 180 | 382 / 526 |
| transport-belt | 12m / 15m | **0** | **0** | 279 / 244 | 378 / 555 | 936 / 2,112 |
| electric-mining-drill | 12m / 15m | 0 | 0 | 27 / 26 | 61 / 67 | 100 / 165 |
| pumpjack / refinery | 54m / 49m | 0 | 0 | 0 | 0 | 0 |

**Exactly 73 burner drills by minute 10 in both runs, and never another one
afterwards in either.** Minute 0–10 is drills and furnaces with **no inserter,
no belt and no electricity at all** — a burner drill dropping straight into a
stone furnace needs none of them.

The opening arithmetic checks out: at minute 5 the any% run is making 261 iron +
169 coal + 90 copper per minute, and a burner drill yields 15/min, so 17 + 11 +
6 = 34 of its 44 drills are producing. **Early iron/min ≈ 15 × (iron burner
drills).** Nothing else is in the number.

Across **all 78 of our archived runs combined we have dispatched 168
burner-mining-drill placements** (best single run: 11) and 1,721 placements of
anything. One boiler, five steam engines, seven labs, zero electric drills,
ever. Our verb histograms are mine-dominated — best green run 146 `mine` against
121 `place`; the 98-minute oil run 1,005 `mine`.

## Where it diverges, and why

- **Minute 0.** They are 5x ahead by minute 5 and the whole difference is drill
  count. A `mine 8 ore` action is 960 planned ticks = 30 ore/min/bot, so four
  bots hand-mining full time cap at ~120 ore/min; 73 drills are ~1,100 ore/min
  *and run while the bots do something else*. A drill costs 9 iron-plate +
  5 stone and returns 15 ore/min: **payback ≈ 36 seconds.**
- **Minute 12.** Ours stops. A/B/C flatten at 635–670 plates at 11.6–12.8 min
  because the plan's bill was filled; theirs turns up 4x between minute 15 and
  20 as the first electric drills land.
- **Minute 8–9 is when power starts** in both references, one pump + one boiler
  + one engine, then 15 boilers and 31 engines by minute 20 (7.6 MW). Ours
  reaches 900 kW once and never grows it.
- **Oil is late.** First pumpjack at 54 min / 49 min, after 100+ electric drills
  and 35 MW. That is direct evidence on the open question of whether oil is a
  t=0 target: for a world-record human on their own map, it is not.

Disclosure, per the standing rule: this is a human with foreknowledge, and 44
drills by minute 5 implies they knew where the coal was. The *ordering* lesson
does not depend on that; the siting speed does.

## What to change, ranked

1. **A first milestone of N burner drills, each dropping into a stone furnace,
   N ≈ 60.** No power, no inserter, no belt, no research. `Site::Beside` is the
   tiling primitive; the unit is a 2-entity pair.
2. **Price hand-mining against a drill.** Hand-mine only what buys the next
   drill. A `mine` buys one item; a `place` buys a rate forever.
3. **`producing:X:N/min` must mean standing capacity ≥ N, not a bill of N.**
   Otherwise the planner has no reason to build drill 74, and (1) decays into a
   one-off script.
4. **Power as a growable thing from minute 8**, not a single 900 kW plant.
5. **Electric drills as the second milestone** (30 ore/min, no fuel logistics) —
   this is what bends their curve at minute 15, and it needs (4).
6. **Machine-feed a lab.** They have 2 at minute 8 and 41–54 by minute 30; in 41
   of our runs containing a lab, not one has an inserter within 2 tiles.
7. **Move oil behind steel and electric drills.**
8. **Adopt the flow-statistics history in our own sampler** — it would give us
   this curve for our own runs even where `samples.jsonl` has gaps, and for any
   reference save dropped in afterwards.

Box load average 8.5 during the probes; every number above is tick-bounded and
none of it is a wall-clock measurement.
