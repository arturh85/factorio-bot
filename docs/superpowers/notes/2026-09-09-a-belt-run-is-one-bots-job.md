# The second concentrator is the belt run

Derived entirely from `run-1788920460-08860`'s own plan record. No build, no
run, no code change — the artifact had the answer.

## The measurement

Fleet utilisation **25.1%**, `steps/bot {1: 520, 2: 136, 3: 119, 4: 106}`. The
`yields_at` fix (`76540e32`) moved `producing:iron-plate:261` from 27.0% to
44.2% and moved **this** goal not at all, so a second concentrator had to exist
and had to be something other than `Chop`/`Stockpile`.

Broken down by verb, the plan is not lopsided in the way the step count suggests
— it is lopsided in *what kind of work* each bot does:

| verb | bot 1 | bot 2 | bot 3 | bot 4 |
|---|---:|---:|---:|---:|
| place | **200** | 9 | 5 | 3 |
| take | 91 | 0 | 0 | 6 |
| craft | 76 | 12 | 4 | 5 |
| fuel | 62 | 7 | 4 | 4 |
| insert | 53 | 11 | 11 | 14 |
| mine | 23 | 50 | 50 | 42 |
| stock | 0 | 39 | 40 | 30 |

**Gathering is divided four ways. Construction is not divided at all** — bot 1
does 200 of the plan's 217 placements, 91% of them.

By *planned ticks* it looks tamer (28,262 / 24,326 / 15,410 / 14,560) because
mining is slow and placing is fast. That is exactly the trap this repo has
recorded before: **a verb histogram cannot see waiting.** The other three bots
finish their mining and then have nothing to do while bot 1 builds.

## And 156 of those 200 are one belt run

```
bot 1: 200 placements -> transport-belt 156, burner-inserter 11,
                         stone-furnace 9, burner-mining-drill 5,
                         small-electric-pole 5, iron-chest 3
```

**A belt run is emitted as one sequence of `Place` steps owned by one bot.**
156 belts, laid end to end, by one pair of hands, while three bots stand idle.

## Why this is the fifteenth instance of the dominant defect class

The capability to split this exists **twice over**, and the belt run uses
neither:

- `method::blueprint` bands entities across bots — its own first line is *"a
  blueprint, an anchor, and one band per bot"*, and a band is a region precisely
  so that *"a bot never crosses another's band"*. `FurnaceLine`'s 179 entities
  split 45/45/45/44.
- `method::assemble` has `deal_bundles` — *"Deal a cell's bundles across
  `builders`, heaviest first, each to whoever is lightest at that moment"* —
  written for exactly this symptom: `run-1788604520-39283` left bot 1 alone for
  the last 47 actions and ~35,000 ticks while the others had finished.
  `grep` gives it **exactly one call site**, inside `assemble.rs` itself.

`method::connect`'s belt run calls neither.

## Why a belt run should be easier to split than a cell

`BuildAssemblyCell::converges` is `true` for a stated and correct reason: *"Ten
buildings, two recipes and two chest charges have to meet in one pair of hands:
three parts of a cell arriving on three bots is a cell nobody can assemble."*

**A belt run has no such requirement.** Each belt is placed from the placer's
own inventory; nothing downstream needs them to have arrived in one pair of
hands. And `deal_bundles`' own safety argument transfers directly: *"a placed
chest, inserter or machine is a **map fact**. Everything that comes after it
names a position and no bot."* A placed belt is a map fact in exactly that
sense.

The shape a fix probably wants is `blueprint.rs`'s, not `assemble.rs`'s: a belt
run is a **line**, which is the easiest thing there is to cut into contiguous
bands, and contiguity is what keeps two bots from walking through each other.

## What is NOT claimed here

That splitting it will make the run faster. Three bots that currently idle would
start walking to their segments, and this project has already measured that
contention on the ground is real — eight bots produced a *shorter plan* and a
*longer run* (1.42x against 1.18x for four). The claim is only that **156
sequential placements by one bot while three stand idle is the concentrator**,
and that the two mechanisms which would address it were both written for other
callers.

Unowned. `method/connect.rs` is currently held by the perimeter-refusal work.

## Outcome (same day): the run is banded, and what the baselines say

`connect_steps_reserving` now cuts a route into **contiguous bands in path
order** (`route_bands`, `method/connect.rs`), one `Step::Owned` per band,
exactly the shape `blueprint.rs` gives a block band -- with the axis question
already answered, because a route's only axis is its path. A band must be
worth its walk: `HANDOVER_WALK_TICKS / PLACE_TICKS` belts (ten), so a 26-tile
run is two bands and a 156-tile run is four. A cut never separates an
underground pair. Bands are dealt heaviest-first to the lightest bot on
`PlanState::planned_ticks`, `deal_bundles`' own rule. A one-band run (one bot,
or a short run) is emitted flat, byte for byte the old plan.

Measured offline on one binary each side of the change, `map.json`, bots
1,2,3,4, HEAD `7a5bdfe4` against this commit:

| goal | before | after |
|---|---:|---:|
| `researched:automation` | 176 / 21,784 | 176 / 21,784 |
| `producing:automation-science-pack:6` | 316 / 22,457 | 316 / 22,457 |
| `producing:logistic-science-pack:6` | 571 / 54,371 | 571 / 54,371 |
| `producing:iron-plate:261` | 194 / 33,645 | 194 / 33,645 |
| `producing:transport-belt:6` | 319 / 119,396 | 319 / 119,396 |
| `sustain:iron-plate:30:36000` | 1,111 / 62,064 (util 64.0%) | 1,234 / 64,564 (util 61.2%) |
| `sustain:copper-plate:15:36000` | 470 / 28,126 (util 39.5%) | 479 / 20,904 (util 58.5%) |

Three things the table says that the diagnosis above could not:

- **`producing:transport-belt:6` did not move, and it could not have.** That
  goal *crafts* belts; it lays none. The 156-belt run in
  `run-1788920460-08860` was `sustain`'s, and the two `sustain` goals are the
  only baselines that route a belt. Five of seven baselines are unchanged to
  the tick, which is the one-band path's promise kept.
- **The deal's load key undercounts the chain actor.** On the iron plan every
  run is 22-28 tiles, so every run is two bands -- and every band 0 went to
  bot 1 and every band 1 to bot 2, never 3 or 4. `planned_ticks` counts only
  owned-chain work, by its own doc "a ranking key, not a cost model": bots 3
  and 4 carry provable mining claims while the chain actor's flat work is
  invisible, so the taker reads lightest. Belts moved 134/0/0/0 to
  85/49/0/0. That is `deal_bundles`' rule, faithfully applied, and its limit.
- **Iron is +4% makespan, copper is -26%, and neither is a run.** The iron
  plan's critical path is still bot 1, now with more feeding (`take`
  117 -> 125, `insert` 64 -> 69) beside its remaining 85 belts; copper's
  four-way idle fell from 5,460/18,925/21,279/22,401 to
  2,414/7,148/11,121/14,043. Both are planned ticks. Contention on the ground
  is the thing this project has measured that a plan cannot see (eight bots:
  a shorter plan, a longer run), so no speed claim is made here.
