# Closing the Idle Gap — Implementation Plan

**Status:** planning only. **Live experiments are paused** at the owner's
instruction until these pieces are assembled. A rung-1 run costs 20+ minutes
and the current bottleneck is understood well enough that further runs buy
little.

**Target:** `researched("automation")` in **under 9 minutes**, ideally near
the single-player world record's **6:12**. Current: **21.4 minutes** with
four bots.

---

## A cell an earlier plan left standing is topped up and drained, not rebuilt

The open end from `bdec88af` — "cells standing from an earlier plan are not
drained across replans" — root-caused on `run-1788552801-73005` and closed.

**What the record says.** The run stood five cells (drills at `[-10,-33]`,
`[-14,-34]`, `[-8,-30]`, `[21,-53]`, `[21,-56]`), in plans 1 and 3. Each was
fuelled with 8–11 coal and every one had **burned it out within ~16,000
ticks**: at every replan that followed (ticks 141,930; 211,986; 232,935;
243,921) the standing drills read `no_fuel` — one `no_minable_resources`
with 3 coal left, the rim cell — and their furnaces `no_ingredients`,
holding 2, 5, 7 and 2 plates. Plans 5 and 6 then each **hand-mined 44 and 30
iron ore (27,360 and 24,720 planned mining ticks) and stood one more cell**,
beside two iron cells standing over roughly 77 and 166 ore (the live world
reads 143 and 232 under those sites before their takes). The furnace
leftovers *were* taken — plan 5 opens with `take 5 iron-plate from the
stone-furnace` and `take 2` — so the `Withdraw` half already worked; what
nobody did was put ten coal back in the drill. The live run
(`run-1788559688-08406`) shows the same shape at tick 135,000: two of its
three drills already `no_fuel`, plus drill 16 at `[-13,-14]` from the red
prelude, fuelless since the savepoint and ignored by every green plan.

**What in the brief was wrong.** The brief asked for a *renewing source*:
"count on its future output at the drill's rate for as long as its fuel and
ore last". At every replan in the data that number is **zero** — the fuel
is gone long before the batch ends. And "a cell with no fuel is not
counted" would have excluded every standing cell there was. A standing cell
is not a source; it is a **placed, unfuelled machine over ore**, worth
exactly one fuel visit. The other half, "take what its furnace holds now",
was already in the plan.

**The fix** (`crates/planner/src/method/produce.rs`, `state.rs`,
`crates/core/src/plan/planner.rs`). `cell_ledger` now admits a drill-on-ore
feeding a furnace with **no queue entry** — an earlier plan's cell — as a
live cell with nothing queued and `started_by: None`, provided no tile
under it is claimed by a hand and no hand-smelt has committed its furnace.
`drain_steps` tops it up and takes from it exactly as it would a cell this
plan stood, timing every take from **this plan's own first fuel visit** to
the drill. The top-up is sized for the job **net of the coal last read in
each slot** (`PlanState::fuelled`, the `fuel` half of the readings
`refresh_buffers` already brought back for furnaces; `FUELLED_ENTITIES`
adds the drill to the query), never less than one coal per machine, because
the visit is the anchor. Bounds: the ore under the drill
(`cell_yield`; a dry cell is not in the ledger); fuel is a credit, never a
count. Offline and for any machine nobody asked about the credit is zero
and the cell is fuelled in full — the safe direction. The chest-role lesson
does not arise: a stage-1 cell has no feed chest.

| solo `Have iron-plate N`, fixture with a standing fuelless cell | before | after |
|---|---|---|
| 20 | 8 actions / 8,065 (hand-smelt) | **4 / 5,924** (one top-up, one take) |
| 50 | 21 / 17,687, 1 drill placed | **4 / 13,124**, 0 drills |
| 100 | 22 / 30,047, 1 drill placed | **5 / 25,497**, 0 drills |

The three baseline goals on the t=0 dump are **unchanged** (136 / 28,023;
300 / 46,089; 402 / 217,749): a fresh map has no standing cell, and within
one plan the ledger already worked. Live, the effect lands at the **second
plan of a milestone and every replan after**: on run 3's shape, plans 5 and
6 would have refuelled two iron cells for ~20 coal instead of hand-mining
74 ore and crafting two drills and two furnaces. Not yet measured live.

Two things this does not fix, named: `cells_standing` (the `Producing`
rate predicate) still counts a fuelless cell as producing; and a standing
drill's `room` is read off the tile amounts as of chunk delivery, so a cell
that mined since is over-counted by what it dug — the short take fails
honestly, as before. Also fixed on the way: `scripting_lua`'s
`plan_cell` call was one argument behind `bdec88af` and failed
`--all-targets` clippy on master.

## A shared smelt builds its own furnace — red −10%, green −13% offline (`e2b8d9c9`)

Half of the "9,300 ticks bot 1 waits on its copper furnace" was not bot 1's:
bots 3 and 4 stood 2,914 and 4,248 ticks at that furnace waiting to *insert*
the shared smelt's ore behind bot 1's take, and the same shape repeated on
three iron furnaces. `b0ed1d25`'s own-first rule was written for the taker's
own loads; a shared smelt's inserts sit on the suppliers' timelines, so
queueing it behind the taker's batch put the wait it exists to avoid on three
bots at once. Fix beside `own_grow`: a smelt whose ore the roster supplies,
on a patch with no idle furnace, builds a furnace of its own.

| goal (four bots) | before | after |
|---|---|---|
| `researched:automation` | 207 / 21,818 | 209 / 21,903 (+0.4%) |
| `producing:automation-science-pack:6` | 358 / 35,239 | 370 / **31,675 (−10.1%)** |
| `producing:logistic-science-pack:6` | 610 / 95,237 | 616 / **82,742 (−13.1%, 22:59)** |

No supplier-insert wait over 800 ticks remains on red (there were nine);
utilisation 47.5% → 54.5%. **Pricing the bank against the taker's own waits
was built eight ways and rejected**: on a serial taker unhidden lag is
conserved under the lookahead key — every variant moved the copper waits
elsewhere on bot 1's timeline and added handling.

**A correction that touches every offline number in this record:**
`workspace/scripts/map.json` is byte-identical to `map-t0-baseline.json`
(fingerprint `dfac0f4caa0a7500`) — the *old* map's t=0 dump, not seed 31337
(`c161fa3f437221d0`). Every offline plan quoted since 2026-09-04 was made on
the old map while every live run was on 31337. The offline and live numbers
still moved together, but they are two maps. **Done, at the owner's instruction: `workspace/scripts/map.json` is now the
seed-31337 t=0 dump** (four headless character bots, honest freeplay
inventory, fingerprint `c161fa3f437221d0`), also kept as
`map-31337-t0.json`; the old baseline remains `map-t0-baseline.json`. On the
seed, the current planner plans automation **177 / 22,044 (6:07)**, red
328 / 28,885, green **623 / 71,167 (19:46)** — against 21,903 / 31,675 /
82,742 on the old map. A one-bot dump had read 8:48 for automation because
bots 2–4 were fabricated with empty inventories.

## The supplier ranking was never consulted; the leak was in the rehearsal (`be29d33d`)

My brief said `furnace_suppliers` hands other bots' furnaces to bot 1. It
hands nothing to anyone: on all three goals every furnace is placed by its own
taker, because since the rock forecast a taker's first coal swing covers its
whole demand and it holds the stone for every furnace after, so a handover
saves a 30-tick craft against a 310-tick detour. The ranking is consulted only
in the **rehearsal pass**, which has no forecast yet, hands a furnace away, and
leaves the supplier's 5 stone + 1 coal on the forecast — which then tipped bot
4 across the `chop_beats_mining` boundary (3 × 120 ties a 360 swing; 4 wins
it) and cost automation the 422 ticks that were blamed on the ranking. Fixed
where it lives: a rehearsal keeps every furnace with its taker, and the
ranking reads the bot's whole planned load on owned chains
(`PlanState::planned_ticks` replaces `planned_mining`). Plans byte-identical.
Red's +5.2% is bot 1 waiting ~9,300 ticks on its own copper furnace — a bank
width question, dispatched (`copper-bank`).

## Oil, first rung: a `mine-entity` trigger names its entity (`ffc56270`)

The mod sent trigger technologies as a bare type because runtime-api.json
documents a singular `entity` while the prototypes write `entities`. Read off
the shipped 2.1.17 prototypes: **`mine-entity` carries an `entities` list of
1–4 names and never a count** (9 techs; `oil-processing` → `["crude-oil"]`),
`craft-item` carries `item` and an optional `count`, `craft-fluid` and
`build-entity` are unused by any shipped tech. The mod now accepts a bare
name, a `{name}` filter or a list of either and always sends `entities` plus
`count`; Rust types default so old dumps load.

The planner walks the ladder by name instead of `UnsupportedResearchTrigger`:
`Goal::Extracted{entity}` behind a `Researched` with a `mine-entity` trigger;
refusals `UndescribedResearchTrigger` (a dump from the old mod — both baseline
dumps), `NotCharted` (the well is 380 tiles out), `NoExtractor`,
`ExtractorLocked` (names `oil-gathering`), `ExtractionNotModelled` — the last
is where the pumpjack cell and fluid routing will go. Charting and extractor
refusals fire *before* prerequisites, so 100 green packs are never planned in
front of a missing well. The three science goals are byte-identical.

**Verified live** (server-only run, `world.dump`): `oil-processing` arrives as
`{"type":"mine-entity","entities":["crude-oil"],"count":1}`,
`uranium-processing` as `["uranium-ore"]`, and `automation-science-pack` is
unchanged (`craft-item`, `lab`, 1). The two baseline dumps still predate this
and refuse as `UndescribedResearchTrigger`; a re-dump of the t=0 baseline is
a deliberate step, not a side effect, because the fingerprint is charting.

## The copper bank was the symptom; the queue behind bot 1's release was the loss (`copper-bank`)

The brief: bot 1 waits ~9,300 ticks on its own copper furnace at `[-51, 29]`,
bank width sized for one, price the bank against the taker's own waits, a
second furnace is 5 stone and 30 ticks. **Measured on the listing**: bot 1's
copper takes waited 384 + 1,818 + 1,344 + 2,615 + 384 = **6,545** on its own
timeline (1, 9, 6, 15 and 1 plates, serial through one furnace, `k = 1`
because `bank_size`'s `widest` is the *idle* count and an own-queued furnace
is not idle); the other **7,162** were bots 3 and 4 standing at that furnace
with the 15-plate smelt's shared ore, waiting for bot 1's take (2,914 and
4,248). The same shape on iron: bots 2-4 waited 3,002 each at `[-34, -32]`,
5,170 / 1,936 / 1,936 at `[-38, -16]`, 1,282 ×3 at `[-32, -33]` — every one
behind a release of bot 1's, because `b0ed1d25`'s own-queue rule sends a
*shared* smelt behind the taker's own batch, and a shared smelt's inserts are
on the suppliers' timelines.

**The remedy the brief named was measured eight ways and rejected.** Pocket
furnaces (stone already in hand, 60 ticks) priced against the wait; own-queued
furnaces counted as bank slots; a per-bot and a per-smelt bare-wait fraction
measured on a rehearsal schedule and on the plan's own schedule (a third
expansion pass); subset selection over priced slots; clustered siting. Every
variant took bot 1's copper waits out and put them back somewhere else on its
timeline: the freed time landed on the shared iron batches (one variant stood
4,026 at a 24-plate take that had been hidden under the research), the solo
rung-1 fixture went 26,770 → 27,867 … 31,878 in every variant that built for
the lag, and red on four bots ranged 30,913 … 41,588 across variants that
differed only in siting or pricing. **On a serial taker's timeline the
unhidden lag is conserved under the list scheduler's lookahead; a bank moves
it and adds handling and walking.** `bank_size`'s doc said this crate cannot
price the "bot is idle anyway" regime; that stands, and now it is measured
rather than argued.

**What was shipped is one line of mechanism**: a smelt whose ore the roster
supplies and whose patch has no idle furnace builds a furnace of its own
(`shared_grow`, beside `own_grow`) instead of queueing its suppliers behind
the taker's release. Same reasoning as the own-queue rule, applied to the
timelines the queue actually lands on.

Offline, `workspace/scripts/map.json` (fingerprint `dfac0f4caa0a7500` — the
t=0 baseline dump, byte-identical to `map-t0-baseline.json`, **not** seed
`31337`'s map), HEAD → fix:

| bots | goal | HEAD | fix |
|---|---|---|---|
| 1,2,3,4 | `researched:automation` | 207 / 21,818 | 209 / 21,903 (+0.4%) |
| 1,2,3,4 | `producing:automation-science-pack:6` | 358 / 35,239 | **370 / 31,675 (−10.1%)** |
| 1,2,3,4 | `producing:logistic-science-pack:6` | 610 / 95,237 | **616 / 82,742 (−13.1%)** |
| 1,2,3 | automation / red / green | 23,411 / 29,962 / 100,599 | 25,166 (+7.5%) / 29,786 / 84,810 |
| 1,2 | automation / red / green | 28,482 / 46,643 / 120,702 | 25,470 / 41,276 / 105,146 |
| 1 | all three | identical | identical (no shared smelt exists) |

Red, four bots: bot 1 idle 9,830 → 4,161, its research 28,364 → 25,513, no
supplier insert waits over 800 ticks (there were nine), utilisation 47.5% →
54.5%, 13 → 19 furnaces. The two regressions are the scheduler's reshuffle,
not the mechanism: on four bots the packs are inserted 85 ticks later; on
three bots one extra iron furnace (three actions) reorders bot 3's ready
work and its `take 10 copper-ore from the wooden-chest` moves from 8,860 to
13,473 with no furnace of its involved.

## ✅✅✅ RUN 17: GREEN IN 14:26 (`run-1788635061-85457`) — best on both measures

2026-09-05 21:19, master `a7e3b0eb`, four clients at 1x, seed 31337 `--new`,
100% tick delivery, planning excluded (27.2 s, clock stopped).

| | run 14 | run 16 | **run 17** |
|---|---|---|---|
| green cell | 17:20 | 15:19 | **14:26** (51,977) |
| green witness | 18:00 | 15:57 | **15:02** |
| plan | 569 / 57,752* | 569 / 52,554* | 569 / **52,819** |
| executed / planned | 1.046* | 1.050* | **0.984** |
| fleet utilisation | 60.9% | 67.4% | **69.5%** |
| iron plate /min at 5 / 10 / 15 | 32 / 57 / 43 | 36 / 64 / 34 | **40 / 73 / 22** |
| red packs /min at 5 / 10 | 0 / 8 | 0 / 13 | **0 / 17** |
| reach corrections | — | — | **0** |

\* plans before the walk model was corrected are ~22% under-priced; only run
17's ratio is meaningful against 1.0.

Fresh-world green: 64:22 → 36:48 → 31:35 → 26:32 → 29:09 → 23:38 → 20:59 →
17:20 → 18:46 → 15:19 → **14:26**. Zero failed actions, zero failed walks
across 402 walk events, zero reach corrections — the 0.6-tile margin held
in a client run as it did headless. Two paths reported a waypoint needing
something destroyed and were routed around.

The plateau is unchanged: production still stops at the plan's bill, and
this run reaches it faster and at a higher rate than any before it. The
self-fed cell remains the next objective.

## ✅ AND THE CREDIT MADE REAL: a walk stops where the action can reach (`b6f6777f`)

The other half of the walk RCA, and this one is a genuine speedup rather
than an honest price. **The margin was measured, not guessed**:
`scripts/walkprobe.lua`, 128 probe walks on seed 31337 across eight
bearings and radii 0.5–5, resting positions read off the mod. Worst
overshoot past the requested radius: **0.301 tiles**, and the mean is
negative at every radius — bots usually stop short. The `R + 1.1` this
codebase had carried since 2026-08-30, from a single walk, was about three
times too pessimistic. Margin set to **0.6**, twice the measured maximum,
because a stop box's diagonal worst case is 0.424 and 128 draws are not a
proof; a post-arrival reach check makes being wrong cost one short step
instead of a failed action.

One rule drives both sides now (`approach_aim`): the planner charges and
simulates to the same point the executor aims the game at, and a blocked
outer ring degrades inward to exactly the old behaviour.

| goal, `map.json` | 4 bots before → after | 8 bots |
|---|---|---|
| `researched:automation` | 21,943 → **21,776** (−0.8%) | 18,310 |
| `producing:automation-science-pack:6` | 28,107 → **26,990** (−4.0%) | 19,573 |
| `producing:logistic-science-pack:6` | 59,018 → **52,819** (−10.5%) | 49,229 |

Action counts identical everywhere: the same plan, walked less. The saving
scales with reach — a build's `(1.3, 10]` annulus gives up 6.1 tiles a
trip, a mine's 2.7 disc only 1.4. Live at 5x: green 56,360 → **55,253**,
executed/planned 0.987 → **0.999**, **zero failed actions and zero reach
corrections over 242 walks** on the verbs that would complain first (74
places, 69 inserts, 71 takes, 111 mines).

What is left of walking is the trips themselves: 201 walks, 43,638 planned
bot-ticks, mean 217 ticks ≈ 30 tiles. That is a siting and assignment
question, not a walking one. Also found: the mine's own corrective walk
(`too far away, moving first!`, 30 times in one run) exists only as a log
line and deserves an event.

## WHAT MULTIPLE BOTS ARE ACTUALLY WORTH, on the honest walk model

The mis-credit flattered multi-bot plans specifically — walking is the part
of a plan that does not parallelise, and every walk was a fifth too cheap —
so **every four-bot-versus-one speedup this record has quoted is overstated
by an unknown amount.** Rather than leave that as a caveat, here it is
measured on `map.json` (seed 31337, t=0, `3d45f40e`, plan makespans in
ticks; the fixture's own measured speedup fell 2.00× → 1.86× on the same
change):

| goal | 1 bot | 2 bots | 4 bots | 8 bots | 4-bot speedup |
|---|---|---|---|---|---|
| `researched:automation` | 27,348 | 23,115 | 21,943 | 18,416 | **1.25×** |
| `producing:automation-science-pack:6` | 59,328 | 38,667 | 28,107 | 20,520 | **2.11×** |
| `producing:logistic-science-pack:6` | 146,120 | 92,717 | 59,018 | 49,080 | **2.48×** |

So bots pay off in proportion to how much independent gathering and
crafting a goal contains, and barely at all for automation, which is short,
local and gated on one research. Eight bots add 16% over four for green and
27% for red. These are plan numbers on one map; the live four-bot green run
is 15:19 against a one-bot run nobody has made recently.

## THE WALK MODEL WAS 22–25% SHORT ON EVERY WALK (`3d45f40e`)

Not a tail of stalls: 792 walks across four four-bot runs, **zero walk
failures**, and the plan under-charged every one of them.

1. **The unrealised `radius` credit — 8,200–10,200 ticks a run, 70–82% of
   the error.** `travel_ticks` charged `(distance − radius) / speed`, but
   nothing stops the bot at `radius`: `arrival_point` and the executor's
   `approach_annulus` both stop it on the **inner** ring (`min_radius`, the
   centre for a disc). In the record it is two clean spikes: every disc of
   radius 2.7 came in exactly 18 ticks over, every disc of radius 10 exactly
   66–67.
2. **The speed constant — 2,200–3,500 ticks.** Implied speed pooled over
   196,717 measured ticks is **0.1413** tiles/tick, not the prototype's
   0.15: a polyline through tile centres, an 8-direction follower with a
   0.3-tile arrival box, one RCON round trip per walk. Now 0.14, rounded
   down so the model over-charges ~1% rather than under-charging.

Ruled out with numbers: obstacle detours (5–8% residual), stalls (one
step-aside in 192 walks), dispatch overhead.

**The plans got longer and nothing got slower.** Green four bots
52,554 → **59,018**; automation 21,735 → 21,943; eight-bot green 48,756 →
49,080; action counts identical everywhere. Live at 5x the run was the same
length to ten ticks (56,360 vs 56,370) and **executed/planned went 1.059 →
0.998**, the walk term from +29% to −4.5%. Applied to the four historical
runs the new model predicts each within ±3% where the old was 22–25% short.

Two consequences worth stating. **Every four-bot-versus-one speedup quoted
in this record was somewhat overstated** — the per-walk credit flattered
multi-bot plans specifically, and the fixture's measured speedup fell from
2.00× to 1.86×. And **the credit can be made real**: stopping a walk on the
outer ring is worth ~8,000 ticks a run, which is a genuine speedup rather
than an honest price. Dispatched as the `outerring` worktree; it needs the
bot's resting position measured rather than inferred, since a walk aimed at
the bound once rested 3.345 tiles out against a reach of 3.

## ✅✅ RUN 16: GREEN IN 15:19, AND AHEAD ON RATES (`run-1788625945-57257`)

2026-09-05 18:48, master `1e965ed7`, four clients at 1x, seed 31337 `--new`,
debug build, 100% tick delivery, planning excluded (25.1 s, clock stopped).
Best on both measures.

| | run 14 | run 15 | **run 16** |
|---|---|---|---|
| green cell | 17:20 | 18:46 | **15:19** (55,152) |
| green witness | 18:00 | 19:25 | **15:57** |
| plan | 569 / 57,752 | 569 / 59,476 | 569 / **52,554** |
| executed / planned | 1.046 | 1.137 | **1.050** |
| fleet utilisation | 60.9% | 58.4% | **67.4%** |
| failed | 1 walk | none | **none** |
| iron plate /min at 5 / 10 / 15 | 32 / 57 / 43 | 33 / 60 / 40 | **36 / 64 / 34** |
| red packs /min at 5 / 10 / 15 | 0 / 8 / 8 | 0 / 9 / 7 | **0 / 13 / 4** |

Fresh-world green: 64:22 → 36:48 → 31:35 → 26:32 → 29:09 → 23:38 → 20:59 →
17:20 → 18:46 → **15:19**. Automation's record for comparison is 6:11.

**The plateau is unchanged and is now the whole story.** Copper stops at
11:46 (189), red packs at 11:51 (85), circuits at 12:51 (66) — production
ends four minutes before the run does, at exactly the plan's bill, because
the cell is charged by hand. Run 16 reaches the same dead end sooner and
with less waste. The next objective is the self-fed cell (drills and
furnaces feeding the assemblers through belts and inserters, owned by the
peer session via its `connect` primitive), measured as a sustained rate
over a window rather than as six packs.

## ✅ THE FOUR-BOT REGRESSION WAS TWO SCHEDULER MECHANISMS (`99ee93c1`)

RCA of run 15 against run 14, traced round by round in `schedule.rs`:

1. **A bot could be committed past a gap another bot's finish would fill.**
   `schedule` commits one `(action, bot)` per round ranked by that bot's own
   finish over its own ready work. With the research hung off the steam
   engine (`c83c5906`), bot 4's `insert 4 iron-ore` — the fourth ore into
   the furnace whose plates open the engine block — carried bound 50,713
   while bot 1's cell takes carried 45,146 and won round after round; by the
   time the insert was committed, bot 1's `free_at` had moved to 47,436 and
   never moves back. Before the pole edge the insert's short tail got it
   committed early **by accident**. Now a candidate is committed no earlier
   than any other bot's choice that finishes before it starts acting.
2. **Two researches were modelled as concurrent.** A force researches one
   technology at a time; the plan dispatched `automation` while `logistic`
   held the labs, so it settled at the first research's end plus its own
   duration: 13,154 against a modelled 6,000 in run 15 (7,234 in run 14).
   **5,920 of run 15's ~7,090 loss was that.** The scheduler now keeps the
   force's research slot.

| goal, `map.json`, four bots | run 14 era | run 15 era | now |
|---|---|---|---|
| `researched:automation` | 22,044 | 21,765 | **21,735** |
| `producing:automation-science-pack:6` | 28,885 | 26,162 | **27,304** |
| `producing:logistic-science-pack:6` | 57,752 | 59,476 | **52,554** |
| eight bots, green | — | 53,326 | **48,756** |

Headless validation (`workspace/headless-i/runs/run-1788625111-28152`, 4
bots, 5x, quiet box): one plan 571 / 53,249, green at 56,370,
executed/planned **1.059** against hl-09's 1.105; `logistic` settled exactly
at its planned duration. Run 16 at 1x is the measured lane. **Neither mechanism ever failed an action.** No refusal, no lost action, no
walk failure: both were invisible to every check the project had except the
plan's own length, and a run that executed one of them faithfully looked
perfect. That is the class of defect the rate framing exists to stop hiding,
and it is the argument for comparing plans offline on every planner change,
which is now required of every agent here. Left on the
table: a strictly chronological commit order plans 51,303 but hands a shared
research to the busy chain owner on ties; walk overruns (~11–12k per
four-bot run) are the largest remaining slip and untouched; research
durations still assume the lab count the method saw.

## THE RATE VIEW (owner, 2026-09-05 19:20): production stops at minute 15

Owner: "maybe you should prioritize production rates at given times over raw
run time." Read off `samples.jsonl` (`force.production.made`, cumulative, at
fixed game minutes from `run_started`):

| minute | run 13 iron / copper / red / green | run 14 | run 15 |
|---|---|---|---|
| 5 | 80 / 69 / 0 / 0 | 159 / 94 / 0 / 0 | 165 / 100 / 0 / 0 |
| 10 | 326 / 133 / 18 / 0 | 442 / 146 / 42 / 0 | 463 / 152 / 47 / 0 |
| 15 | 677 / 189 / 55 / 0 | 657 / 189 / 82 / 0 | 661 / 189 / 83 / 0 |
| 20 | 677 / 189 / 85 / 0 | run ended at 18:00 | run ended at 19:25 |
| end | 677 / 189 / 85 / 4 (21:39) | 670 / 189 / 85 / 6 (18:00) | 670 / 189 / 85 / 4 (19:22) |

**Stated plainly: until a cell feeds itself, milestone time has been
measuring plan length, not factory output.** Every conclusion in this record
drawn on the makespan alone — including the run-to-run ordering of the last
two days — compares how long the bots took to build the same dead cell.

(Corrected by `just analyse --rates-md`: runs 14 and 15 never reached minute 20 from `run_started`; the row I first typed there was the end value. `--compare` now says: run 15 ahead at 5, 10 and 15; run 14 ahead at the end.)

Three things the makespan table cannot show. (1) **Runs 14 and 15 are the
same run on rates**, run 15 a little ahead through minute 12; the 17:20 vs
18:46 difference is the last two minutes of a factory that has already
stopped. (2) **Every run's production plateaus at minute ~15** at exactly
the plan's bill — 670 iron plates, 189 copper, 85 red packs — because the
planner builds a cell and then charges it by hand ("charge the feed chest
with 6 iron-plate"); nothing feeds the cell after the charge, so the rate
at minute 20 is zero. "Producing 6/min" is witnessed as six packs, not as
a rate. (3) The world-record replays' curves rise through the same window
(`docs/superpowers/notes/2026-09-04-world-record-replays.md`).

Consequences, in order: the analyser reports production at fixed marks and
per-minute rates over a trailing window, and runs are compared on that
table first (dispatched as `rates`); `producing:X:N` must mean a sustained
rate verified over a window, with the cell fed by drills and furnaces
through inserters and belts rather than by hand — which is the consumer
the peer's `connect` primitive has been waiting for; and the record carries both, each where it fits: the curve for anything
about sustained output, game ticks for a genuine first event such as a
technology landing or a build-time comparison (owner: "continue using game
time where it makes more sense than production / throughput rates").

## ⚠️ RUN 15: 18:46 (`run-1788621697-14165`) — slower than run 14, honestly

2026-09-05 17:20, master `917fdcd2` (everything of the afternoon merged),
four clients at 1x, seed 31337 `--new`, debug build, 100% tick delivery,
**clock stopped for both plannings (26.4 s, 0 ticks charged)** — the first
measured run under the 17cdd5f7 rule, so its number excludes planning and
run 14's included ~1,983 ticks of it.

| | run 14 | run 15 |
|---|---|---|
| green cell | 17:20 (62,408) | **18:46 (67,598)** |
| plan | 569 / 57,752 (+ 9-step replan) | 569 / 59,476, one plan |
| executed / planned | 1.046 (planning removed) | **1.137** |
| failed | 1 walk | none |
| fleet utilisation | 60.9% | 58.4% |

So the pole-edge change (`c83c5906`) that took eight bots from 71,936 to
67,636 at 5x costs four bots ~1,700 ticks of plan and ~3,900 ticks of
execution at 1x. The number stands and 17:20 remains the best; the RCA is
dispatched as the `fourbot` worktree: where the extra execution slip sits
per bot against run 14, and whether the pole condition should be an edge
(order) without being a scheduling tie the cell's plate take wins.

## The afternoon after 17:20: what headless mode found (2026-09-05, 15:00–18:00)

Full log in `docs/superpowers/notes/2026-09-05-headless-experiments.md`. On
master since run 14: aim avoids standing bots (`0f5f5170`); new workspaces
seed scripts (`3cd57c70`) and are created on first run (`675f93b7`);
character spawn spread, pinned-walker step-clear, push-out record
(`f24c02a6`); boxed-in bench (`efc1931b`); **research names the poles and
generator it draws through (`c83c5906`)** — the eight-bot execution gap was
labs sitting dark for 8,500 ticks because `Condition::Powered` had no edge
to the pole that powers them; the same hole was in the four-bot plan and
landed by luck. Offline green for four bots is now **569 / 59,476**
(+1,724 on a scheduler tie-break, the model being honest); eight bots
53,326. Pending merge: the game clock stops while the planner thinks
(planning wall time was the whole speed tax: 942 ticks at 5x, 1,837 at 10x,
6,438 for green at 5x), gated to servers this process owns, recorded as
`planning_timed`. Run 15 at 1x follows the merge; its number will exclude
planning time and say so.

## ✅ GREEN FROM A FRESH WORLD IN 17:20 (`run-1788612263-27812`) — the merged afternoon, measured

Run 14, 2026-09-05 14:44, master `53949434` (tail, character-identity,
walk-into-rock, replan-reuses-site, belt-routing all merged; workspace tests
green), four clients at 1x, seed 31337 `--new`, debug build. Validated first
headless at 5x (hl-04: one plan, 0 failed, 18:54 at 5x) — the first run of
the day to follow the offline → headless → 1x sequence.

| | |
|---|---|
| green cell producing 6/min | **17:20** (62,408 ticks) |
| green witness | **17:59** |
| plans | 569 / 57,752, then 9 / 75 after one failed walk at tick 62,191 |
| failed / lost | 0 / 0 actions; 1 walk |
| planned vs executed | 57,752 vs 62,408 — 8.1% |
| fleet utilisation | **60.9%** (was 44.9%) |
| steps/bot | {1: 195, 2: 152, 3: 125, 4: 97} (was {278, 123, 115, 107}) |

Fresh-world green: **64:22 → 36:48 → 31:35 → 26:32 → 29:09 → 23:38 → 20:59 →
17:20.** The failed walk: bot 1 was aimed at (34.73, -7.89) for the
assembler at (36.5, -5.5) while bot 3 stood idle at (34.25, -7.70) — the
aim sits on a standing bot, the entity graph holds no characters, and the
game answered no path. Dispatched as the `aimbots` worktree: the approach
aim must avoid the roster's known positions the way it now avoids boxes.

## ✅ THE TAIL IS DEALT: offline green 71,167 → 57,752 (`f5f273bb`, branch `tail`)

RCA of run 13's one-bot tail, three mechanisms, each with a number from its
plan: (1) `Researched::expand` pushed the `research` action inline and
`run_steps` stamped it with the cell's chain, whose owner is bot 1, so a
world-scoped action (`Researched`, `EntityAt`, `Powered`; no `HasItem`, no
`AtPosition`) was welded to one bot — `research automation` had its deps met
at ~38,107 and started at 53,423 while bot 2 idled from 36,027. (2) The pack
deal was 11 / 25 / 39 because `deal_by_load`'s preload charged the lead two
17,232-tick lab bills for one lab and seeded nobody with `planned_ticks`;
bot 4's 39-pack chain ran to 59,089 and gated `research
logistic-science-pack`. (3) The cell's 22 placements and all its crafting
were bot 1's through `BuildAssemblyCell::converges` + `Holder::Share`.

Fixes, general: `Action::tied_to_runner` — an action naming `Actor::Role` in
no condition or effect belongs to nobody, is left unstamped and off the load
ledger, and the scheduler gives it to the earliest finisher; `block_bill_ticks`
prices each item once per bot; the preload is seeded from `planned_ticks`;
`cell_steps` deals every cell's steps as `Step::Owned` bundles across the
roster (a roster of one emits the old sequence byte for byte). Two new
tests pin (1) and (2); planner tests and clippy green.

| goal (map.json, seed 31337) | before | after |
|---|---|---|
| `researched:automation` | 177 / 22,044 | 176 / 21,765 |
| `producing:automation-science-pack:6` | 328 / 28,885 | 324 / 26,162 |
| `producing:logistic-science-pack:6` | 623 / 71,167 | **569 / 57,752 (−18.8%)** |

Per-bot planned ends 71,167 / 36,027 / 46,079 / 59,367 → 57,732 / 57,722 /
44,137 / 57,752. Open from this RCA: research is modelled as occupying a bot
for its whole duration (it is a lag); bundles are dealt without pricing the
builder's walk; `produce.rs`'s plate cells still bill the chain actor; pricing
the deal in hand time helps green (54,847) and hurts red (31,296). Validated
offline only at the time of writing; the headless 5x run of the merge is the
next section.

## ✅ GREEN FROM A FRESH WORLD IN 20:59 (`run-1788604520-39283`) — copper bank merged, and the tail is one bot

Run 13, 2026-09-05 12:31, master `cf73a47f` (copper `shared_grow` `48847c80`
merged), four clients at 1x, seed 31337 `--new`, debug build, log in
`workspace/session-logs/run13.log`.

| | |
|---|---|
| green cell producing 6/min | **20:59** (75,543 ticks) |
| green witness | **21:38** |
| plan | one — 816 steps, 623 actions, 0 failed, 0 lost, 193 of 193 walks |
| planned vs executed | 71,167 vs 75,543 — 6.1% |
| fleet utilisation | **44.9%** (bot 1 the whole window; bots 2–4 idle for the last 12.6 / 10.0 / 7.4 min) |
| steps/bot | {1: 278, 2: 123, 3: 115, 4: 107}; planned ticks {1: 40,276, 2: 17,166, 3: 20,800, 4: 24,660} |

Fresh-world green: **64:22 → 36:48 → 31:35 → 26:32 → 29:09 → 23:38 → 20:59.**
Three walk stalls at tick ~9,000 where our own furnace at ≈(-12.8,-11.5) and
drill at ≈(-12.6,-13.6) pinch the walkway next to spawn and bots 1, 3, 4 met
there together; one `failed to path find` for bot 4's mining approach near
(35.7,-52.8). All resolved by the walker's steering and the recovery tiers;
no walk failed. Bot 1's idle by verb waited for: `take` 13,947 ticks,
`research` 2,927.

**The mechanism now in the way is the tail.** The plan ends the other three
bots at ticks 36,027 / 46,079 / 59,367 and leaves bot 1 alone for 47 actions
and 35,000 ticks: all 22 `place` steps of the green cell, all crafting of the
cell's parts (copper-cable 1,530 ticks, gears 1,020, circuits 1,020, inserters
1,020 — 4,600 ticks a free bot could have taken), **and both researches**.
`research automation` (6,000 ticks) starts at 53,423, behind bot 1's queue,
while bot 2 has been idle since 36,027; `research logistic-science-pack`
(11,400) then gates the recipe set and the charging. Research is chained to
the crafter because the lab and the packs are in bot 1's hands, and the cell
build is one bot's sequence. Two levers, both planner: hand the research to
the earliest free bot (a materials handover through the cell's chest is one
action) and split the cell's crafting and placing across the idle roster the
way `Holder::Share` splits research. Dispatched as the `tail` worktree.

## ✅ GREEN FROM A FRESH WORLD IN 23:38 (`run-1788594774-55056`) — the furnace-slot fix, executed

Git `b0ed1d25`: a bot with no furnace of its own on the patch builds one as its
own errand, and least-loaded reads the whole queue (the proxy had been reset
on every adoption). The RCA found **one furnace at [-34,-32] carrying 26
batches from all four bots (53 visits)** while two furnaces two tiles away sat
idle after one batch each; three mechanisms, none of them bank width. The
brief's remedy — price the queue wait — was measured and rejected: a release
is an action, not a tick, and pricing it made green worse (108,170).

| | |
|---|---|
| green cell producing 6/min | **23:38** (85,090 ticks) |
| green witness | **24:16** |
| plan | one — 811 steps, 617 actions, 0 failed, 0 lost, 194 of 194 walks |
| planned vs executed | 83,311 vs 85,090 — 2.1% |
| fleet utilisation | **43.0%** (bot 1 77.9%, bots 2–4 26–37%) |

**Disclosure (2026-09-05, from the peer session):** during this run the peer
issued several `factorio-bot rcon --settings <its file> -s localhost` queries
meant for its scratch server on 4324; because of the bug fixed in `47c087fe`
they dialled this run's port 4321 instead. All were read-only prints (a
recipe's enabled flag, a technology's researched flag, an entity count) and
all returned empty, so they most likely never landed, but the record cannot
prove that. Nothing odd was found in the run's record; the number stands with
this note attached.

Fresh-world green over the night: **64:22 → 36:48 → 31:35 → 26:32 → 29:09 →
23:38.** Offline the same change moved red +5.2% (33,487 → 35,239) because
red is bot-1-bound and bots 3/4 now stand furnaces of their own; automation
held (21,818). Open from this RCA: `furnace_suppliers` ranks by
`planned_mining`, which does not count rock swings, so it hands other bots'
furnaces to bot 1.

## The stalled walk was a starved server, and the stall clock counted from the wrong place (`2e705d0b`)

Every hypothesis in my brief was wrong, and the record had the answer.
`from` in the wording is the *stall position*, not the leg's origin: leg 2
was 1.42 tiles, the character had walked 1.04 of it in 7 ticks and then stood
still for 54. Nothing physical was there — run 11's chunk write-out for the
same seed shows zero entities within 12 tiles and 1,024 tiles of `dirt-3`.
**`video/ticks.jsonl` shows the server at 2.3–10 ticks per second through
the exact windows in which the character froze** (60 tps before and after);
run 9 had 35 slow spans, runs 10 and 11 had none and no failed walks. Across
1,581 in-flight beats under a healthy tick rate a steered character never once
stood still.

**Those slow spans are mine.** Run 9 was launched at 02:50 and I ran the
merged planner/core/executor/scripting-lua suites and a CLI build in
`.worktrees/chop` from 02:53 — a starved server during a measured run. The
CLAUDE.md rule "never start a live run while an agent is building" holds in
the other direction too. Memory written.

What landed anyway, because the clock was wrong on its own: the walker's
`walk_leg_timeout_ticks` (from the leg's start) is now `WALK_STALL_TICKS`
counting from the last tick the distance to the waypoint shrank; consecutive
duplicate waypoints (60 of 5,131 legs in run 11, the only sub-half-tile legs)
are dropped at the request; the verdict now says `moved 1.04 tiles of a
1.42-tile leg that began at (x/y) … steering east at 0.150 tiles/tick,
walking_state read back walking=true`, so the next such line tells whether
the game held a steered character or something overwrote the steer. Five
stub-runtime walker tests, offline plans untouched.

## The shared furnace: least-loaded compared the newest batch, and the budget starved every bot but the first — green planned 26:27

The RCA behind run 11's +5,173, done offline against `workspace/scripts/map.json`
with a listing that now names the furnace on every fuel, insert and take (`at
[-34, -32]`). **Three mechanisms, and my brief named none of them exactly.**

1. **"Least-loaded first" compared the newest batch, not the queue.**
   `smelt_steps` unqueues a furnace while it emits and queues it again behind
   its own take; `MachineQueue::queued` lived only on that entry, so the cycle
   reset it to the last batch. On green the furnace at `[-34, -32]` read
   576–2,304 while carrying **26 batches from all four bots** (53 visits); the
   furnaces at `[-33, -30]` and `[-38, -16]`, two tiles away, sat at 3,072 and
   3,840 with one long shared batch each and were never chosen again. The
   total now lives in `PlanState::machine_load`, which only `queue_machine`
   writes.
2. **The budget counted cells.** `patch_furnace_budget` is one furnace per bot,
   compared against *every* stone furnace near the patch — bot 2's starter cell
   stood there before the first hand-smelt was expanded, so "four" was three
   hand furnaces, all bot 1's, and by the time bot 4 asked, cells had made it
   six against four. It now bounds hand-smelt furnaces (`PatchFurnaces::hand`);
   ground is protected where a furnace is sited (`cell_room_to_spare`).
3. **The budget was first-come.** Bot 1's first smelts built every furnace it
   allowed, so bot 4's nine drill plates queued 9,848 ticks behind bot 1's
   ladder; its drill stood at 44,658 (37,100 → 42,273 on the fuel that opened
   the queue, exactly the brief's numbers), its 78-plate cell take at 63,668,
   research behind it. **A bot with no furnace of its own on the patch now
   builds one, as its own errand** (`own_grow`), and a smelt queues behind
   *its own* batch before a lighter furnace of somebody else's.

The critical path, before: bot 4's drill plates (3 + 6) queued on `[-34, -32]`
behind bot 3's 10/2/4 and bot 2's 8 → drill placed 44,658 → 78 plates at 240
each → 63,668 → 39 packs crafted → 76,548 → research → cell. After: bot 4
smelts its nine on its own furnace at `[-38, -18]` (takes 32,405 / 33,779),
cell take **53,009**, and the makespan is bot 1's chain.

**What the numbers refused.** Fix 1 alone: green 102,405 → **108,170** — the
queues spread evenly and every bot then waited on a batch some *other* bot
would insert late (bot 3's insert at 41,028 held bot 1's 16-plate take to
44,302). Fix 1 + 2: 113,330. "Own furnace first, and only then the budget"
(a bot chains everything on one furnace): automation 27,265, red 52,557 —
bot 1's bank overlap is real. Fix 3 with the own furnace handed to a
supplier: automation 22,240, because `furnace_suppliers` ranks by
`planned_mining`, which does not count rock swings, and so hands bot 3's
furnace to bot 1 — **open**, and a separate RCA.

| goal | before | after | furnaces | util |
|---|---|---|---|---|
| `researched:automation` | 197 / 21,883 | 207 / **21,818** | 8 → 12 | 42.0 → 45.2% |
| `producing:automation-science-pack:6` | 347 / 33,487 | 358 / **35,239** (+5.2%) | 8 → 13 | 47.2 → 47.5% |
| `producing:logistic-science-pack:6` | 628 / 102,405 | 610 / **95,237 (26:27)** | 18 → 24 | 33.2 → 35.8% |

Green per bot, steps / planned / idle: before `{1: 355/58,432/43,973, 2:
168/22,897/79,508, 3: 148/24,857/77,548, 4: 148/29,910/72,495}`; after `{1:
370/62,396/32,841, 2: 162/22,265/72,972, 3: 141/24,249/70,988, 4:
138/27,577/67,660}`. Bot 1 hand-mines 36 more iron ore (a 24-ore smelt that
was shared is now its own) and is the whole critical path.

**Red moves up 1,752**, and it is bot 1's timeline: bots 3 and 4 each stand a
furnace of their own (+330 ticks each, from a rock), bot 1 builds a fourth
(the budget no longer counts the cell), and its `research automation` starts
at 28,364 instead of 26,715. The plates are the same plates; the red plan is
bot-1-bound and every tick bot 1 spends before the research is makespan.

Pins moved, each with its reason in place: the unlock fixtures 7,278 → 7,156
and 6,955 → 7,654; the fleet's rung-1 bill coal 54 → 78 and stone 48 → 93
(three suppliers fuel furnaces of their own off rocks — gathered, not dug
twice; iron and copper do not move). `every_expansion_replays_in_time_order`
lost its cross-chain edges — every furnace in its fixtures was a first
furnace — and gained a fourth case where bot 1 smelts first, so R3 still
fires on a taker's second furnace.

## Run 11: the rock forecast's +5% on green reproduced live — 29:09 (`run-1788583161-11653`)

Same script, seed and roster as run 10, git `cbf5ae01`. One plan, 605 of 605,
zero failures; green cell at **29:09** (104,990 ticks), witness 29:48. Run 10
on the previous planner: 26:32 (95,527). Offline the forecast predicted
97,232 → 102,405; live it was 95,527 → 104,990 — **the +5% is real and the
plan is the whole cause** (execution again within 1%). Hand-mined coal is
zero; iron hand-mining is unchanged at 86 actions / 23,828 ticks; bot 1 busy
52.6%.

The agent's own trace: bot 1 now swings its rock first instead of placing its
furnace at 209 and digging six single coal; its furnace queue shifts, and
because the hand-smelt furnace is shared as a *slot*, bot 4's crafted-drill
plates queue 37,100 → 42,273 behind it, its 78-plate cell take 58,495 →
63,668, research behind that — the same 5,173 at every step. Planned
bot-ticks fell; the makespan rose through one shared furnace. That is the
next RCA: a second furnace for a shared bank is 5 stone against 5,000 ticks
of queue.

Net for the night on the fresh-world ladder: **automation 6:11 (record 6:12),
green 26:32 best / 29:09 current**, both one plan, zero failures.

## ✅ AUTOMATION IN 6:11 — one second under the pre-Space-Age record (`run-1788582657-14978`)

`just bench automation_speedrun.lua --clients 4 --bots 4 --logs`: release
build, `--seed 31337 --new`, fresh map, git `cbf5ae01`, roster `[1,2,3,4]`,
no resume, no cheats.

| | |
|---|---|
| `researched("automation")` | **22,271 ticks = 6:11 game time** |
| the record (pre-SA, same recipes) | 6:12 = 22,320 ticks |
| planned makespan | 21,985 (6:06) — executed within 1.3% |
| plan | **1**; 167 actions, 217 steps, 0 failed, 0 lost, 100 of 100 walks |
| fleet utilisation | 38.5% (bot 1 65.6%, bots 2–4 27–33%) |

Yesterday morning this was 8:17 on an unidentifiable map; last night 7:04 on
this one; now 6:11. **Every second of it is the planner: the schedule
executed as written.** Provenance says `dirty: true` — the only uncommitted
change in the tree was another session's docs note
(`docs/superpowers/notes/2026-09-05-remote-control-approach-review.md`), no
code.

The comparison still flatters us three ways and the record says so: four bots
against one human, a map picked for short walks, and a run that does only
automation. The number to beat honestly is the same runner's split on our
map, which nobody has. What it does establish: the planner and executor
agree to 1%, and the roster is now the whole gap.

## A rock is judged over the bot's whole demand — automation planned 6:05 (`7e330a2c`)

Both halves of my brief were wrong. **No starter drill placement has any
incoming edge** — `de765de3`'s same-chain skip already removed the ordering;
three of four starter drills stay in inventory on automation and red because
`cell_setup_bot_ticks` prices the held drill and furnace as nine plates mined
and smelted from nothing. Pricing them free was built and measured: every
starter drill on ore in the first minute, and automation 25,886 → 32,693,
green 97,232 → 127,801 — a burner drill is 240 ticks per plate against 120 by
hand, so a one-drill share ends *later* unless the bot has other work. The
wrong price was keeping single-drill cells off the critical path by
accident; the true price is the count rung. Reverted, documented on the
function.

What shipped: `expand` rehearses once, recording per `(bot, item)` how much
`Mine`/`Chop` were asked for, and the real pass prices a rock over the bot's
whole demand while coverage stays per fragment. Per bot, not roster-wide — a
roster-wide forecast made four bots swing four rocks for one bot's coal.

| goal | before | after |
|---|---|---|
| `researched:automation` | 207 / 25,886 | 197 / **21,883 (6:05)** |
| `producing:automation-science-pack:6` | 359 / 39,118 | 347 / **33,487** |
| `producing:logistic-science-pack:6` | 644 / 97,232 | 628 / 102,405 (+5.3%) |

Hand-mined coal: 13/15/16 actions → 4/4/**0**. Green's +5,173 is a scheduling
shift through the shared hand-smelt furnace slot (bot-ticks *fell* 137,504 →
136,096); the 86 iron hand-mines are drill work rocks cannot touch. **6:05
planned is under the 6:12 record; it is a plan, not a run.**

## ✅ GREEN FROM A FRESH WORLD IN 26:32, ONE PLAN, ZERO FAILURES (`run-1788578779-80166`)

Git `5c5087fb` (everything of the night: rocks, reach, fuel lag, recovery,
deadlines, bot death, enclosure step-aside, drain, lookahead scheduler,
ore-under-drill graph fix, research split with two labs, turned-box
footprints, cell ladder). Fresh seed-31337 world, debug build, no resume.

| | |
|---|---|
| green cell producing 6/min | **26:32** (95,527 ticks) |
| green witness, 5 packs in 2,280 ticks | **27:10** |
| plans | **1** — 799 steps, 619 actions, **0 failed, 0 lost, 360 of 360 walks** |
| planned vs executed | 95,192 vs 95,527 — **0.35%** |
| fleet utilisation | **36.0%** (bot 1 59.7%, bots 2–4 23–32%) |

Fresh-world green over one night: **64:22 → 36:48 → 31:35 → 26:32**, and the
last one needed no replan, no recovery and no refusal. Where the ticks go
now: red-pack crafting 25,582 over six actions, hand-mining iron 86 actions /
23,831 ticks (fragments still hand-mined beside 11 drills), research 12,599
on two labs, copper 11,711. The named opens from the ladder agent are next:
the starter drill ordered behind a crafted one by `infer_edges`, and
`chop_beats_mining` answering per fragment.

## ✅ GREEN FROM A FRESH WORLD IN 31:35 (`run-1788576604-65414`) — footprint fix confirmed live

Git `399c041c` (footprint fix in; cell ladder not yet), fresh seed-31337 world.
**Green cell at 31:35, witnessed at 32:15**; 470 of 470 actions; 140 of 140
walks bar one stall; fleet utilisation 26.5%. No steam-engine refusal this
time — the turned-box check classified the actor and the plant stood where
the plan put it. The one stall (`leg 2 of 86 made no progress for 61 ticks …
moved 1.04 tiles … nothing findable`) was **recovered in place by tier 1 —
`planned 252 steps (best 606) -- recovered: rescheduled 1` — and the
rescheduled batch finished 291 of 291**: the first time a tier-1 recovery
after a walk refusal demonstrably reassigned work and completed. The walker's
sub-tile-leg stall is under RCA (`walker-short-leg`).

Fresh-world green across the night: **64:22 → 36:48 → 31:35**; planned for
the next binary (cell ladder): 27:00.

## The cell ladder was three mechanisms, none of them "the previous cell's plates" (`de765de3`)

My brief said each drill was built from the previous cell's plates. Wrong:
bot 4's drill was hand-smelted and waited 27,000 ticks on bot 1's *furnace
queue*. What serialised green:

1. **`Drain::cap` tripped at five drills** because `cells_stood` counted any
   furnace near the patch with an iron-plate queue — the three hand-smelt
   furnaces too — and after that every cell was eligible whatever its backlog,
   so 12-plate drill bills queued 21,830 ticks behind a 78-plate science share.
2. **`infer_edges` paired every `HasItem` consumer with every earlier producer
   on its chain**, so bot 1's cells were placed one per take: place, wait
   8,400, place, wait 8,400. Its own doc admitted this.
3. **Stated supply edges used the oldest stock (FIFO)**, not the stock the
   subgoal had just produced for that consumer.

Measured one at a time: the cap fix alone 142,092 (worse), LIFO alone
195,894, inference alone 142,092, **all three 97,232**. They are one
mechanism. "Cells built breadth-first by other bots via handover" was built,
measured and rejected on all three goals.

| goal | before | after |
|---|---|---|
| `researched:automation` | 207 / 25,886 | 207 / 25,886 |
| `producing:automation-science-pack:6` | 359 / 39,791 | 359 / 39,118 |
| `producing:logistic-science-pack:6` | 503 / 132,507, util 23% | 644 / **97,232 (27:00)**, util 35%, 11 drills, bot 1 idle −44k |

**Open, named:** a bot's starter drill is still ordered after a *crafted*
drill by `infer_edges` (bot 4's copper cell stood at 56k instead of ~10k);
`chop_beats_mining` answers per fragment, so a solo bank plan hand-mines 1+3
coal before swinging a rock; count-ahead-of-demand is still a rate-rung
decision.

## ✅ GREEN FROM A FRESH WORLD IN 36:48 — the research split, executed (`run-1788574143-35250`)

Same script and seed as run 7, git `1502629c` (research shared across the
roster, two labs), debug, no resume, no cheats.

| | run 7 | **run 8** |
|---|---|---|
| green cell producing 6/min | 63:42 | **36:48** (132,537 ticks) |
| green witness | 64:22 | **37:26**, 5 packs in 2,220 ticks |
| plans | 407 → 86 | 470 → 183 |
| actions | 454 / 1 failed | **626 / 1 failed** (the same engine refusal; fix not yet in this binary) |
| research logistic-science-pack | 20,643 ticks, one lab | **7,499**, two labs |
| fleet utilisation | 12.9% | **30.5%** — bots 2/3/4 at 24%, 28%, 30% (were 3–4%) |

The executed 36:48 is the offline plan's 36:48 to the minute. **The day's
green number went 60:07 planned / never executed → 36:48 executed**, and the
whole ladder from an empty map to witnessed green science now fits in 37
minutes and two plans.

The 51,159 ticks of red-pack crafting are now spread over 12 actions on four
bots, and the remaining single-bot item is the serial cell ladder the research
agent named — the `cell-ladder` RCA is in flight.

## The refused engine site: the mod tested footprints with an unturned box (`1b544f94`)

My brief was wrong on every count. Plan 1 sited nothing inside anything —
its four poles were at [33.5,−14.5] [38.5,−7.5] [35.5,−7.5] [40.5,−11.5] and
none overlapped the engine's east box; the pole at (42.5,−5.5) belonged to
**plan 2**, emitted 345 ticks after the refusal. What blocked the engine was
**bot 1 itself**, parked at (42.24, −6.77) where its walk to the pipe left it
— inside the *east-facing* 3 × 5 footprint by 0.41 tiles. The mod's
`rcon_place_entity`, `character_in_footprint`, `step_aside_from_footprint`
and `rcon_can_place_entities` all tested the prototype's collision box
**unturned**, so the game said no for the actor, every branch saw nobody, and
the generic `said 'no'` went into the refusal ledger as a bad site; plan 2
moved the whole plant 10 tiles east.

Fixed where it lives: every footprint question in the mod uses the box turned
to the placement's direction; the refusal names what was in the footprint and
the tile; the record carries the direction and a step-aside reason; the
planner excludes the *turned* refused box; the executor's pre-place check
steps the actor out of its own footprint before any RPC. Verified live,
read-only, on bots 3 and 4 that a character alone in the box makes
`can_place_entity` false. Offline plans unchanged. One planner-side gap named
for later: `arrival_point` models a walk as landing on the annulus's inner
edge along +x while the walker stops anywhere in a small box.

## Research work shared across the roster — green planned 60:07 → 36:48 (`34d3773b`)

Three mechanisms serialised research on bot 1, and my brief named one:

1. **`Holder::Share(chain_actor)` on every research subgoal**, made a hard
   owner by `c470388b`: the 75-pack craft and its gears, cable and circuits
   were bot 1's because the *bill* was stated for bot 1. The scheduler was
   right; the decomposition was wrong.
2. **The trigger technology.** `automation-science-pack` is a 2.0 trigger
   tech fired by crafting a lab, so every pack craft carried
   `Researched("automation-science-pack")`; bot 1's `craft 1 lab` fell at
   tick 101,473 behind the cell's whole bill while bots 2–4 sat on ready
   ingredients from 46,448. Invisible in the live record, where automation
   was already researched.
3. **One lab**, hard-coded by `lab_site` taking the first standing lab.

Landed: the pack bill dealt across the roster as owned `Holder::Share(b)`
blocks (each share sized against and bound to its bot — `c470388b`'s guarantee
kept), width from `worth_converging`; the trigger craft and first lab on a
lead supplier; `labs_worth_building` adds a lab while it saves more than the
lab's from-raw bill (17,232 ticks, not "10 gears + 10 circuits") shared over
the roster — two labs for green on four bots, one for automation, never alone.

| goal | before | after |
|---|---|---|
| `researched:automation` | 136 / 26,212, util 32% | 207 / **25,886**, util 41% |
| `producing:automation-science-pack:6` | 300 / 44,641 | 359 / **39,791** |
| `producing:logistic-science-pack:6` | 402 / 217,105, util 13% | 503 / **132,507 (36:48)**, util 23% |

**Corrections:** the two big actions were 22% of the makespan, not the
problem — **~60% of green's planned time is bot 1 *waiting* on a serial
ladder of burner cells, each drill built from the previous cell's plates
(~86,000 idle ticks, `produce.rs`)**; and the second lab pays only because the
roster shares its bill. Green's planned number is chaotic at ±20k under small
changes for the same reason. That ladder is the next RCA.

## ✅ GREEN FROM A FRESH WORLD IN 64:22 — the first end-to-end, non-resumed number

`run-1788569499-05724`: `factory_stage3.lua --seed 31337 --new`, debug binary at
`790e2c63`, roster `[1,2,3,4]`, no resume, no cheats. One 498-step plan for
the whole ladder (automation, the red cell, the green cell), one replan.

| | game time from tick 0 |
|---|---|
| automation researched (first tech in the samples) | ~8:00 |
| green cell producing 6/min | **63:42** (229,357 ticks) |
| green witness: 6 packs in 2,341 ticks | **64:22** |

Actions **454 success, 1 failed** (a refused steam-engine site — see the RCA
below), walks 103 of 103, plans 407 → 86 steps. Fleet utilisation 12.9%: bot
1 ran 338 of 455 actions, busy 41%; bots 2–4 busy 3–4%.

Two things the run showed working for the first time live: the pre-placement
enclosure check **stepped bot 1 out of a 19-tile pocket** before it placed an
assembler at [55.5, 2.5] (`bot_stepped_aside`, tick 236,884), and the ore
tiles under standing drills stayed off the hand-mining list. One label is
wrong: the driver prints `WALLED IN: 1 bot(s)` for that step-aside, because
`record.enclosures()` now drains both. Fixed below.

### The one failure: the plan's own pole inside its engine's footprint

`place steam-engine at [40.5, -5.5]` refused by `can_place_entity`, recorded
with `blockers: []`. Live: `small-electric-pole@42.5,-5.5` and `pipe@43.5,-5.5`
from the same plan; `can_place(north) = true`, `can_place(east) = false` — the
east-facing 3 × 5 footprint contains the pole. One placement of a plan sat
inside another's future footprint and the area check saw neither. RCA agent
dispatched (`footprint-overlay`).

## Green in ONE plan: 56.75 min after the resume, 264 of 264 (`run-1788565721-53126`)

Same savepoint, git `5bfcd9e0` (every fix of the night in the binary).
**One plan, 264 actions, 0 failed, 0 lost, 134 of 134 walks**; planned makespan
202,262, executed 204,295 — within 1%. Cell satisfied at **56.75 min** (run 5:
75.73), witness 5 packs in 2,040 ticks, `state=done`. Hand-mined iron: 24
actions (run 3: 184; run 5: 64). No step-aside was needed.

**Utilisation 10.0%**: bot 1 ran 205 of 264 actions and was busy 34.9%; bots
2, 3, 4 were busy 1.8%, 2.0%, 1.3%. On bot 1's serial timeline: `craft 75
automation-science-pack` 22,575 ticks, `research logistic-science-pack`
22,499, gears 4,919. That is 25% of the makespan on two actions three bots
could have shared or halved. The record already names the cause: a
`Researched` chain's trigger and pack subtrees are bound to one bot
(`c470388b`, "22,072 ticks on a live four-bot run"), and the plan builds one
lab. RCA agent dispatched (`research-parallelism`).

## ✅ AUTOMATION IN 7:04 — `just bench` run for the first time, seed 31337 validated

`run-1788565090-80288`: `just bench automation_speedrun.lua --clients 4 --bots
4 --logs` — **release build, `--seed 31337 --new`, a fresh map**, git
`b297bbb8`, roster `[1,2,3,4]`, not resumed, no cheats.

| | |
|---|---|
| `researched("automation")` | **25,461 ticks = 7:04 game time** |
| planned makespan | 24,940 (6:56) — execution within 2% of the plan |
| plans | **1**; 132 actions, 170 steps, 0 failed, 0 lost, 76 of 76 walks |
| fleet utilisation | 31.0% (bot 1 75.6%, bots 2–4 14–18%) |
| map | digest `cb1032c7fa39cb2a`; iron 940, copper 803, coal 466, stone 397 tiles charted |

Previous best 8:17 (unidentifiable build, run 3 days ago); the bar is 6:12.
**A 73-second improvement from the day's planner work executed exactly as
planned**: rocks stood beside, reach measured to the box, the lookahead
scheduler, yield-aware drill siting. The seed is now validated — the water
fits the pump, the shoreline was never refused — and every earlier caveat
about `just bench` having never run is closed.

The remaining minute against the record is in the roster: bot 1 does 87 of
132 actions and is busy 75% of the span while the others are busy 15%.

## Ore under a standing drill: the graph forgot the drill's ground (`3c05e0a3`)

My brief said the model does not know an entity stands on an ore tile. **Wrong:
`PlanState::resource_tile_blocked` has read `blocking_boxes_within` since
`6a6221b6`** and every "tile under a drill/furnace/chest is skipped" test
passes on the old code. The mechanism was one level down. When a drill in run
5 mined a tile under itself dry, the mod reported the ore entity deleted, and
`EntityGraph::remove` swept `blocked_tree` with the removed entity's box,
deleting **every overlapping box** — the ore's 0.2-tile box sits inside the
drill's, so the drill's went too. Six of seven plan-1 drills had such a
deletion before plan 2, which then hand-mined `(-7.5, -29.5)` and
`(28.5, -47.5)` and found drills. Fix where it lives: a resource removal no
longer touches `blocked_tree` (it never added to it), and any other removal
takes only the box whose centre lies inside its own bounds. Two tests red on
the old code, offline plans unchanged. Run 3 never hit it only because its
plans 6–7 mined 88 tiles that happened to be clear.

## ✅ GREEN SCIENCE WITNESSED — the first time in this project's history

`run-1788559688-08406`, seed `31337`, resumed from `run-1788528493-60555:3`
(red rate-witnessed), git `b560af5c` (drills, reach, recovery, craft deadline,
bot death; the enclosure, drain, scheduler and research-deadline fixes landed
on master after it started), roster `[1,2,3,4]`, `--resume-force`.

```
WITNESSED: logistic-science-pack in 5 watched machine(s) went 0 -> 5 (+5, wanted 5)
in 1985 of 3600 ticks, 1844 polls
RUN FINISHED state=done
```

| rung | game time from the resume |
|---|---|
| 1 — green cell producing 6/min | **75.73 min** (272,636 ticks), 3 plans: 302 → 303 → 149 steps |
| 2 — witness, 5 packs in 90 s | +0.6 min; the packs arrived in **1,985 ticks (33 s)** |

The red prelude ended at tick 64,465 in the run it was resumed from, so the
whole ladder from a fresh world would be about **94 game-minutes** — a
resumed run is not comparable with a fresh one and `--compare` will say so.

Actions: **538 success, 2 failed, 1 lost**; walks 118 of 118. The three
failures are the two classes named in the section below (ore under a
standing drill; the research deadline), both fixed on master since.
Hand-mining iron ore fell from 184 actions / 83,267 ticks (run 3) to
**64 / 17,909**.

**What the number says next: utilisation was 10.1%.** Bot 1 ran 389 of 541
actions and was busy 29.7% of the span; bots 2, 3 and 4 were busy **4.2%,
4.8% and 1.9%**. The two largest single items are still on bot 1 and still
serial: `craft 75 automation-science-pack` (22,707 ticks) and `research
logistic-science-pack` (20,643). Those are the world-record lessons — science
made by machines, a rate rung ahead of the research — and they are the next
lever, not another defect.

## Run 5, first batch: 293 of 301, no rock, no siting, no divergence — and two new classes

`run-1788559688-08406` (green, same savepoint, drills + reach + recovery +
deadline + bot-death in the binary; enclosure, drain and scheduler landed on
master after it started). First batch: **293 successes, 0 failed, 0 walk
failures**, 57 game-minutes; the 64-plate and 100-plate cell takes both
succeeded where run 3 got 40 of 64. The one loss was `research
logistic-science-pack` — 75 units × 5 s in one lab = 375 s against the flat
360 s deadline, the same defect as the craft, one action kind over. Fixed at
the general layer (`fdbf8d8c`): `Actuator::research` takes the plan's own
duration and both crafts and research now wait `sized_deadline(ticks)`.

Second batch, 96 of 301, two failures of a class no run had shown:

```
could not start mining for 301 ticks: expected iron-ore at (-7.5/-29.5), found burner-mining-drill
```

A hand-mine aimed at an ore tile under a **drill an earlier plan placed**.
Within a plan `bdec88af` claims a drill's tiles; across plans the world
model's resource tiles do not know an entity stands on them. RCA agent
dispatched (`ore-under-entities`).

## The scheduler learns to look one step ahead — automation planned at 7:17 (`51c7f695`)

The greedy list scheduler's key was `(end, action, bot)`: whatever finishes
soonest goes first. The trace on the four-bot fixture showed why the coal trip
came last (every nearer place/mine *ends* sooner) and why the take then waited
beside a furnace that had only just started. **My proposed fix — a static
critical-path priority — made every real plan worse** (automation 28,023 →
29,210): bot 1 walked to the coal patch three separate times, because a
priority computed on the network alone cannot see where the bot stands.

What landed instead: a **lookahead bound** — for a candidate on a bot, the
worst of "its own end plus what follows it" and "the other ready work on this
bot, reached by walking from it, plus that work's remaining path" — as the
primary key, old tie-breaks preserved, integer ticks, deterministic.

| goal | before | after |
|---|---|---|
| `researched:automation` | 136 / 28,023 | 136 / **26,212 (7:17)** |
| `producing:automation-science-pack:6` | 300 / 46,089 | 300 / 44,641 |
| `producing:logistic-science-pack:6` | 402 / 217,749 | 402 / 217,105 |
| four-bot fixture | 2,682 | 2,543 |

Utilisation on automation 29.3% → 32.0%; bot 1's idle 7,916 → 6,383. Two new
pins fail under the old key (a far trip gating a lag goes before nearer work;
co-located work is finished before walking away). Four method-layer pins moved
down with it. **Correction:** the 2,063 the earlier note wanted back was never
honest — it took plates from a furnace that had not started; the floor is bot
work plus walking, and each bot now idles 22–29 ticks on that fixture.

**Unmeasured live.** 7:17 planned against 8:17 measured on the previous
planner; the next automation run says what the schedule is worth executed.

## The walled-in bot, root-caused: the stand-point is chosen blind, and the fill was too fine (`79f3f9d3`)

Bot 1 walked to place `assembling-machine-1 [31.5, -4.5]` arriving from the
east, so `approach_annulus` aimed the walk toward where it came from and it
landed at **(34.418, -4.629)** — a 2 × 2 pocket inside an **older cell**: chest
column at x = 33.5, inserters at x = 34.5, pole at [35.5, -4.5], machines at
x = 36.5. Four ticks later it placed the new assembler (box x 30.3..32.7) on
the ground it had walked in across. What was left were a 0.5-tile and a
0.65-tile crack; Factorio's `request_path` at the mod's resolution is 1 × 1
tiles centred on tile centres, and both neighbouring tiles are occupied at
their centres. **Its own placement, from a stand-point the actuator chose
blind.**

**Why the enclosure record was silent:** not a precondition — the fill itself
returned `Open`. Its eighth-tile configuration-space grid found the two cracks
the game's one-tile grid cannot use. The module doc's "incomplete in the safe
direction" was wrong in this direction. `crates/core/src/graph/enclosure.rs`
now works on the game's grid (`CELL = 1`, tile-centre box test); the same
fixture reads `Enclosed { 4 }` after the placement and `Open` before
(`crates/core/tests/enclosure_run73005.rs`, with the run's entities as a JSON
fixture). The planner's `enclosure::check` delegates to core so prevention and
detection share one grid.

**The system fix, executor side** (`crates/executor/src/pre_place.rs`):
before every `place`, the same fill with the footprint added; if it would
close, walk to the nearest reachable tile that stays open, is clear of the
footprint and within build reach, place from there, and record
`EventKind::BotSteppedAside`. Refuse by name only if no such tile exists. The
planner is the wrong layer: it names an annulus, never a point, and its check
correctly said Clear with the bot 50 tiles away at plan time. A lane
reservation around cells would make this fire less often; not needed for
correctness. No-path walk records now carry `from`/`to`
(`-- found no path from (x/y) to (x/y)`).

**Corrections to my brief:** every suspect I listed (pole, tree line, water,
`judge_path`) was wrong; `record.enclosures()` *is* called by the drivers; and
the fill was too *fine*, not too coarse. Offline plans unchanged.

## Run 4 died with the disk, and the drills work landed (`b98fe587`)

`run-1788556703-31339` (same savepoint, master with recovery, deadline and
reach fixes): batch 1 went 269 of 306 with one partial transfer, then every
player vanished — `player not found (id 1)`, then the planner's refusal
*"bots 1–4 are not connected players in this world; refusing to plan against
a fabricated inventory"*. **The root filesystem had hit 100%**: `target/` had
grown to 371 GB across sessions and this session's eight agent worktrees
added 86 GB. An environment failure, not a code one; the refusal is the right
behaviour. `target/debug` and `target/release` were removed and rebuilt,
merged worktrees removed, 439 GB free after. Memory written so the next
session checks `df` before a run.

### Drills over hands (`bdec88af`), measured and merged

| goal | before | after |
|---|---|---|
| `researched:automation` | 136 / 28,918 | 136 / **28,023** (−3%) |
| `producing:automation-science-pack:6` | 299 / 43,871 | 300 / 46,089 (**+5%**) |
| `producing:logistic-science-pack:6` | 443 / 216,322, mine 101 / 42,000 ticks | 402 / 217,749, mine **73 / 19,560** |
| ladder `producing:iron-plate:60` → automation | — | 151 / 41,111, 4 drills, **128 plates by tick 36,000** (was 101) |

What landed: **yield-aware siting** — a cell is sited where the drill's 2×2
holds the ore its takes will draw (the `removed 40 of 64` root cause; pinned
on a thin-rim fixture), takes consume the drill's tiles; **drains** — later
fragments of the same plan are served from the least-backlogged live cell;
**rock-priced gate** (crossover 50 → ~32 plates); and **stated supply edges**
in `run_steps`, because `infer_edges` dropped the real producer→consumer edge
once drains existed. Rejected with numbers: opening cells on backlog within
one goal took red 43,871 → 78,240 — on a roster 15–35% busy, wall-clock is
the scarce resource and one cell at 240 ticks/plate loses to four hands.
**Count ahead of demand is a ladder decision**, which is why the WR lesson
shows up as a rate rung (`producing:iron-plate:60`) and not inside
`researched:automation`.

**Corrections to the brief:** `PlaceDrill` never claims ore goals — it claims
plate goals, and the `mine` actions are `Smelt` decomposing plates in ~4-ore
fragments, so no per-goal gate could ever see the aggregate demand; and the
baseline dump's chosen sites hold 154–570 ore, so siting was not the blocker
*offline* — the dry cell came from a world the dump does not describe.
**Open:** cells standing from an earlier plan are not drained across replans.

## Run 3 finished `stuck` at 50 game-minutes — and its four failure classes are all named

`run-1788552801-73005` (green, seed 31337, resumed from the red witness, first
run with both rock fixes): **649 successes, 3 failed, 1 lost, 3 refused walks;
fleet utilisation 22.2%; hand-mining iron ore 184 actions / 83,267 ticks (23
min of bot time, 51% of the span)**. Plans: 386 → 302 → 275 → 363 → 313 steps.
The rocks are gone from the failure list for good. What remains:

| class | count | fix |
|---|---|---|
| `removed 40 of 64` — the cell's drill mined its tiles dry | 3 | drills agent: amount-aware siting (in flight) |
| tier-1 re-issued the identical take | 1 | `c1c41412` — divergence is not rescheduled |
| `craft 75 automation-science-pack` **lost** at the flat 360 s deadline while still crafting | 1 | `fffdb37e` — deadline sized from the recipe (packs alone are 375 s) |
| `failed to path find`, bot 1, three different targets | 3 | **open** — bot 1 stood at `(34.4, -4.6)` for all three, from tick 211,958 to 248,685, right after placing the green cell's `assembling-machine-1 [43.5, -2.5]`, `inserter [43.5, -0.5]` and `iron-chest [43.5, 0.5]`. It walled itself in with its own placements, 37,000 ticks frozen, and `record.enclosures()` recorded nothing. This is the "nothing checks a walk's landing spot against footprints the plan needs later" item from the open list, now with a position. Run 4 replays the same plan; the live world will be queried when it recurs. |

The lost craft is the worst of these: the supervisor refuses to recover a run
with a lost action, so it replanned around a craft the game finished anyway.

### Bot death, bench-verified (`a002efc3`)

Scratch run `run-1788556166-27937`, `game.players[2].character.die("enemy")`
mid-batch while bot 2 was mining: the mod wrote `player_died` at tick 76,342,
failed the mining action in flight with `player 2 has no character: died at
tick 76342`, and `player_respawned` 600 ticks later. The record shows the
failure as `no_character` and carries `bot_died` / `bot_respawned` — stamped
at the flush tick (85,828), not the death tick, which is a small inaccuracy
to fix when someone touches the recorder. No `roster_changed`, correctly: the
bot was back before the next plan.

**A defect of the merge, found by the agent that shipped the recovery fix and
by the core suite:** the new guard refuses a player without a `character`, and
every BotBridge stub test built players without one — 30-odd tests across four
files went red on master for an hour. Fixed by giving the stubs a character
(`fffdb37e`, `07b6e6a9`, `1b80112c`).

## The cell's drill mines its four tiles dry — the `removed 40 of 64` divergence, root-caused live

Run 3 (`run-1788552801-73005`), batch 3: `take 64 iron-plate from the cell`
→ `removed 40`; tier-1 recovery re-issued the identical take → `removed 0`.
Asked the running game, read-only, while the next cell was mid-wait:

```
burner-mining-drill@-10,-33  status=21 (no_minable_resources)  fuel=3 coal left  ore=NONE
stone-furnace@-10,-35        status=18 (no_ingredients)        products_finished=40
neighbouring cells: products_finished = 36, 37
iron tiles within 6 of the cells: n=45  min=9  median=271  max=511
```

**Not fuel, not rate.** Bot 1 handed over the coal (178 → 162 across the
drill's and furnace's fuelling) and the drill still has 3 left. The drill
mined its 2 × 2 area dry after 40 ore: `PlaceDrill` sites the drill on the
*nearest* tiles of the patch, which on seed 31337 are the thin edge at ~10 ore
per tile, and sizes the take from the coal it gave the drill (10 coal = 267 s
≈ 66 ore) with no check that the mining area holds that much. The model has
the number — `EntityGraph::resource_amount` carries the mod's reported
`amount` per tile — and nothing reads it when siting. Handed to the drills
agent as the first fix, ahead of the count lever: on this seed it is the
largest single source of the divergence class.

Two smaller things from the same query: `defines.inventory.furnace_source` is
gone in 2.x (`crafter_input` / `crafter_output`), and a tier-1 retry of a
partial transfer is exactly the "identical command to the identical
container" the recovery design warned about — the recovery agent is closing
it.

## Two world-record replays, read for lessons — the lever is drill COUNT

Full note: `docs/superpowers/notes/2026-09-04-world-record-replays.md`. Final
worlds of both saves loaded on a scratch server and their production history
read at 2-minute resolution. By minute 8: **they have ~1,200 iron plates,
40–70 burner drills and 100+ stone furnaces, all stone and coal from rocks;
we have 96 plates, 7 furnaces, no drills, and four bots hand-mining.** Their
`automation` lands at 10–12 min — later than our 8:17 — with mining already
automated. Copper is untouched until minute 4–6. Science is machine-made from
the first pack. The first metric for a 3 h horizon is plates per minute at
minute 10 (ours ~10, theirs ~350), not the automation timestamp. Our
`PlaceDrill` cell is the right shape; the plan builds it in ones.

Trap found on the way: `flow_precision_index` names a *window* (`ten_minutes`
= the last 10 min at 2 s), and rock yields are not counted as production.

## Rocks work live; the next failure is a furnace fuelled 8,000 ticks after it was fed (`330a9a39`)

`run-1788552801-73005`, same savepoint, with both rock fixes and `--resume-force`
(the mod digest changed). **All nine chops succeeded** — one tree, one big-rock,
seven huge-rocks including every far one — and all six corrective walks landed
within reach. First batch: 140 of 306 succeeded, 0 walks failed, **1 action
failed**, and it is a different class:

```
take 5 copper-plate from the furnace        tried to remove 5 copper-plate but removed 3
```

The trace: bot 2 inserted 5 copper ore at tick 75,932 through the shared-ore
path; bot 1's own fuel landed at 84,023 — after every rock it chopped for the
coal — and the take fired 26 ticks after that. The furnace had made three
plates on **residual fuel from the red prelude** and gone cold. And **the plan
itself** had the take 40 ticks after the fuel and 6,800 after the insert,
which no furnace can do: the taker's own fuel edge carried zero lag, on the
assumption that it sits one action after the ore insert on one serial
timeline. That held only while the ore was the taker's too.

Fix: every fuel edge carries the smelting lag. The executor's rule is
`max over preds (finish + lag)`, so this is exact whichever of ore and fuel
lands last and cannot double-count. Four pins moved and say why; the four-bot
fixture ceiling in `red_science.rs` went 2,310 → 2,950 (measured 2,682) because
on that fixture the coal is mined *after* the ore, so the old 2,063 took plates
from a furnace that had not started. **Scheduling the coal ahead of the ore
would win most of that back** — a planning improvement, queued.

| goal, baseline map | before | after |
|---|---|---|
| `researched:automation` | 136 / 28,918 | **unchanged** |
| `producing:automation-science-pack:6` | 299 / 43,821 | 299 / 43,871 |
| `producing:logistic-science-pack:6` | 443 / 216,172 | 443 / 216,322 |

Not yet in a live binary: run 3 was left running on the previous build to see
how far green gets with bot 1 no longer halted. **Residual fuel in a reused
furnace is a second, unmodelled fact** — `refresh_buffers` reads contents the
plan can draw on, not the fuel slot's state — and it is what let three plates
exist at all.

### What run 3 looks like mid-way, and why it is not a stall

At tick 111,292 the heartbeat read 151 of 254 settled, nothing in flight, three
bots waiting on bot 1's `take 12 iron-plate` and bot 1 waiting on a
`lag_deadline` for `take 34 iron-plate` from the same furnace at `[-10, -15]`.
The live game confirmed four idle characters. **That is a legitimate wait**: a
34-plate smelt is 6,528 ticks of one furnace, and the plan queues every bot's
ore insert behind the take that empties the slot. Eight furnaces stand on this
map and the plan serialises the whole roster on one. That is the shape of
green's idle problem now that the defects in front of it are gone.

## Reach is measured to the box, not the centre — the second rock defect (`76f3e153`)

`run-1788551693-66583`, same savepoint as the run below, with `98ca85d8`
(stand beside the rock). **The walks all succeeded — 109 of 109** — and six
chops went through: four huge-rocks, one big-rock, one tree. Then the far
huge-rocks failed, now as *actions*:

```
too far away, moving first!
the walk to [53.625, -109.4375] would end at [53.5, -108.5], inside a
collision box spanning [52.13, -110.54] to [55.13, -108.34]
```

Two rules measured reach to the entity's **centre**: `within_resource_reach`
in `rcon.rs` and the mod's `on_tick` guard. The game measures to the
**collision box** (`can_reach_entity`). For a 3 × 2.2 rock the planned annulus
is (2.14, 2.7], the walker's stop box is 0.3, and a landing at 2.72 read as out
of reach — after which the corrective walk aimed a plain disc at the centre,
inside the rock, and `judge_path` refused it. 5 plans, 11 game-minutes, 4
rejected actions, killed once the fix was built.

Now `mining_distance` is to the blocking box when the target has one and to the
centre for ore; the corrective walk is an annulus (clearance, reach + shorter
half-side] beside a boxed target; the mod's guard asks `can_reach_entity`, with
the centre rule as fallback for a player without a character. Three tests pin
the Rust side against a huge-rock with its live box. **The mod side is
untested until run 3** (`--resume-force`, because the mod digest changed).

### Two agents landed in the same hour, and both found their brief wrong

- **`7ad5cba9` — a hand cannot mine crude oil, and an unseen ore is
  "unexplored, not absent".** The discriminator is categorical and **was not
  in the data**: `resource_category` (`basic-fluid`) against the character's
  `resource_categories` (`basic-solid`); the mod never sent either. Both now
  travel end to end, `#[serde(default)]` so old dumps load. `NotHandMinable`
  and `NotCharted` (with a resource census and a compass frontier) are
  verdicts, not faults. Offline plans byte-identical. **Corrections:**
  `refusal_for` lives in `crates/scripting_lua/src/globals/goal/mod.rs`, not
  the planner; the `schedule.rs:543` reclassification cannot touch an
  expansion-time refusal; and `an_unobtainable_item_has_no_method` had pinned
  the very ambiguity being removed.
- **`febdeead` — the Stockpile cost model never inverted on the map.** With
  the chest: red 28,897, red-producing 43,797, green 216,370; without it:
  32,172 / 54,952 / 221,253. **The chest pays on every goal** (10.2%, 20.3%,
  2.2%). The fixture inversion was **mining seats**: one five-ore stockpile put
  three runners on a nine-seat patch, the smelt behind it failed `G6` and bot 1
  hand-mined 20 ore. Rocks were irrelevant. `worth_stockpiling` now reserves a
  whole further convergence (`seats >= k + 2·known`); the map is unchanged
  because 3 + 2·4 = 12 is exactly the seat cap, and the signed assertion is
  back. **The record above that says "the Stockpile cost model is now
  questionable" was wrong.**

## Rocks had never been chopped live — the first run to try lost bot 1 in every batch (`98ca85d8`)

`run-1788549906-13347`, seed `31337`, resumed from `run-1788528493-60555:3`
(red rate-witnessed, 8 furnaces), `factory_stage3.lua`, roster `[1,2,3,4]`.
The first live run since rocks landed (`130b3bde`), and therefore **the first
run in which a bot was ever sent to chop a rock**. The "8:01 planned since
rocks" figure had never been executed.

Every batch ended the same way:

```
ran: success=100 pending=207 actions(failed=0 lost=0) walks(failed=1 lost=0)
first error: game rejected the command: the walk to [-7, 16.375] would end at
[-6.5, 15.5], inside a collision box spanning [-8, 15.48] to [-6, 17.38] —
a character cannot stand there, so the walk could only stall
```

The box is the rock's own. The step was `chop big-rock at [-7, 16.38] for 20
stone`, and `Chop`'s `AtPosition` carried **`min_radius: 0.0`**, so the walk
asked the game for a disc centred on the rock; the pathfinder's last waypoint
landed inside it and `judge_path` refused before dispatch — correctly. Trees
never showed this because a tree's box is 0.8 wide and the path happens to end
beside it. A `big-rock` is 2 × 1.9, a `huge-rock` 3 × 2.2.

**12 plans in 24.9 game-minutes, zero action failures, one refused walk per
batch, always bot 1's** — the bot holding 198 of 307 steps. Tier-1 recovery
fired three times and each time re-issued the identical walk to the identical
rock, which answers a question the recovery section left open: *a tier-1
reschedule after a walk refusal does not reassign the work* when the chain
owner pins the bot. Rule 4 did not catch it because `obs.success` kept rising —
the other three bots were working. Killed deliberately at 25.9 min wall once
the fix was built; the record is complete up to the last batch.

### The fix, and what it moves

The inner radius is now the entity's **placement clearance** — half its
collision diagonal plus half the character's — the same bound `Place` uses,
for the same reason. Pinned by `a_chop_stands_beside_the_rock_not_on_it`, which
also asserts the annulus is non-empty against the character's reach.

| goal, baseline map | before | after |
|---|---|---|
| `researched:automation` | 136 / 28,897 | 136 / **28,918** (+21) |
| `producing:automation-science-pack:6` | 299 / 43,797 | 299 / 43,821 (+24) |
| `producing:logistic-science-pack:6` | 443 / 216,370 | 443 / 216,172 (−198) |

The ticks are the schedule's simulated arrival point moving off the rock's
centre; the old figures priced a stand-point the game refuses. Two exact pins
moved and say why (`38,606 → 38,620`, `5,773 → 5,800`).

### Corrections to the brief I was handed

- **Green's offline baseline is 443 actions / 216,370 ticks**, not the
  619 / 216,474 the record still quotes further down — that figure predates
  rocks.
- **"Planned 8:01 since rocks" was an offline number only.** Nothing had run
  it. A planner change that alters *where a bot stands* cannot be verified by
  the 4-second loop, which never asks the pathfinder anything.

### What the plan itself says about green, before any execution

First plan: 307 steps, makespan 191,189 (53 min). Bot 1: 198 steps, 66,316
planned ticks, **last step ends at 191,189**; bots 2–4: ~36 steps, ~8,500
planned ticks each, done by 42,613. The tail is `craft 75
automation-science-pack` (22,500 ticks, starting at 146,076) followed by
`research logistic-science-pack` (22,500). **Bot 1 waits ~125,000 ticks inside
its own plan** — on smelt lags feeding the packs — while three bots stand idle
from tick 42,613. That is the shape the previous green run showed too (207 of
406 steps on bot 1) and it is the number to move once green completes at all.

## A stalled walk now names what blocked it (`52d35716`) — and the message was lying

The mod probes at the instant the leg gives up and appends the cause:

```
ERROR: stuck while walking, leg 9 of 10 made no progress for 61 ticks
from (-9.90625/-18.171875) to (-10.5/-18.5), moved 0.02 tiles,
blocked at (-10.437/-18.702) by character #3 (mining) on tile 'grass-1'
```

Causes: `character #N (walking|mining|idle)`, `entity '<name>' (ours|theirs)`,
`tree`, `rock`, `cliff`, `nothing findable`, and
`blocker unknown (probe failed: <err>)` — it runs under `pcall` inside
`on_tick` and **says when the probe itself failed** rather than falling back to
something that reads like the old message. The Rust side distinguishes three
answers where two would hide a regression: **no clause** (older mod),
**`Nothing`** (looked, tile clear), **`Unknown`** (a wording this build does
not know, carrying the mod's own words).

Two deliberate refusals: it never names a **resource** — a bot stuck on ore
stands in a solid block of them, so that would be a confident wrong answer on
the most common terrain — and it does not widen the box until something is
always found. **`nothing findable` is a real answer.**

### `"made no progress"` has never measured progress

The check is `event.tick - w.idx_tick > w.leg_timeout` **and nothing else** — a
leg **timeout**. It fires just as readily for a leg walked *slowly* as for one
that is wedged. **The record has been asserting the stronger claim for both.**
The wording could not be renamed (three consumers, and every archived run
matches that string), so the mod now stamps the leg's origin and reports
`moved <d> tiles` beside it. Without that number `nothing findable` is
unreadable: **0.00 tiles is a pathfinder problem, 3.40 tiles means
`walk_leg_timeout_ticks` is wrong.**

### Stalls are invisible to the record

`grep -rl "made no progress" workspace/runs/` returns **zero of 22 runs**,
while the most recent run's *log* contains one. A stall that recovers via a
fresh path settles the walk `success`, so the error never reaches
`events.jsonl` — the whole class exists only in stderr and has never been
analysable. This is the fifth mechanism found here reporting nothing while
working.

### Corrections to my brief

- **`classify_walk_failure` is in `crates/scripting_lua/src/globals/record.rs`**,
  not `rcon.rs` as I said.
- **The structured record field is not done** — `WalkFailure` lives in
  `crates/core/src/record/`, which was another agent's territory. The cause
  reaches `events.jsonl` verbatim inside `WalkSettled.error`, but there is no
  `blocked_by` column. Adoption is two lines and `WalkBlocker` already derives
  `Serialize`/`Deserialize` for it.

**Untested geometry:** Factorio has never loaded this `control.lua`. Whether
0.75 tiles ahead with a 0.35 half-box is where a wedged character's obstruction
sits is a guess sized from the character's 0.2 half-width. If wrong, the
symptom is a flood of `nothing findable` with `moved 0.00 tiles` — legible
rather than silent.

## The divergence bug is a container capacity limit — found on a bench in minutes

The owner suggested checking *"if you can pick up more than 1 stack size at
once"*. That was the answer.

### Measured live, on a scratch instance

```
stone-furnace  output slots=1   accepted 100 iron-plate   (stack 100)
stone-furnace  fuel   slots=1   accepted  50 coal         (stack  50)
stone-furnace  input  slots=1   accepted  70 iron-ore     (stack  50)
iron-chest            slots=32  accepted 300 iron-plate
```

**`tried to remove 141 iron-plate but removed 100` is a furnace output slot
holding exactly one stack.** Not staleness, not double-counting — **a furnace
cannot hold 141 plates and never could at any moment**. A raw
`remove_item{count=141}` from a chest holding 300 removes 141 correctly, so it
is not a cap on the transfer either.

**And a furnace whose output slot is full stops smelting**, which is where the
`full_output` machine status in archived samples comes from. An over-sized
batch does not merely fail at the take — it stalls the furnace partway and
wastes the whole wait.

### The planner has no notion of stack size anywhere

Nothing in `have.rs` or `state.rs` mentions it. `withdraw_slot` knows *which*
slot to draw from and never *how much fits in it*.

| item | stack |
|---|---|
| iron-plate, copper-plate, iron-gear-wheel, transport-belt | 100 |
| electronic-circuit | 200 |
| stone, coal, iron-ore, copper-ore, inserter, stone-furnace | **50** |

### The naive fix would be wrong, which is why this was worth measuring

**The input slot is not one stack** — it took **70** ore where the stack is 50.
Factorio lets a machine's ingredient slot exceed a stack by a recipe-derived
margin, while output and fuel are exactly one. So "bound every transfer by
`stack_size`" **under-fills the input** and is right for the other two only by
coincidence. **70 must not be hardcoded either** — it is one measurement of one
recipe on one machine, and the rule behind it is unknown. Ask the game.

### What this says about method

This took **four minutes on a bench instance** and pre-empted an hour of design
work aimed at "staleness versus accounting" — neither of which it was. The
capability that made it possible is `factorio-bot rcon -s localhost`, which
already existed and which nothing documented until today. **A scratch instance
where cheating is allowed is a different tool from a measured run**, and the
questions it answers cheaply are exactly the ones that have cost whole runs.

## Dumps now carry container contents (`8d07af4b`) — and `iron-chest` must NOT be whitelisted

`world.dump` asks the game what is in the containers before writing the file.
Chosen over "the caller refreshes" for one reason: contents are **pulled, never
pushed**, so a dump that writes `inventories: []` is **byte-identical to one
that says nobody looked**. That file then misleads every offline plan made from
it, possibly days later. The side effect is narrated on stdout, skipped with no
RCON (`--clients 0`), and non-fatal — a failed read still writes the file,
since a dump is usually taken at a milestone you cannot reproduce without
repeating the run. The path is resolved **before** the read, so an escaping
path never reaches the game.

### My instruction to add `iron-chest` was wrong, and following it would have broken stage 2

I read `BUFFER_ENTITIES`' comment — *"add it the day something places one"* —
as a standing instruction whose day had come. It had not.

**`plan_cell` places three iron chests per cell and none of them is a store.**
`FeedChest` "holds one of the things the intermediate machine eats";
`SupplyChest` "holds the ingredient nothing in the cell makes". All three are
**inputs**, hand-filled with `CELL_CHARGE_TICKS` of ingredients before the cell
is switched on. The product never enters a chest — it sits in the assembler's
output slot, which `withdraw_slot` already reaches.

So whitelisting the name would not recover stranded items; **it would let a
replan drain a running cell**, and quietly, because `plan_cell`'s own doc
records that *"After it runs out, nothing detects it."* The whitelist's rule is
"entities this planner builds **and unloads itself**" — the planner builds iron
chests and never unloads one. The distinction needed is *what a particular
chest is for*, which a name-keyed list cannot express. Now written into the
`BUFFER_ENTITIES` comment so nobody derives it a third time.

### A correction to how I stated the gap

*"A dump's `inventories` is `[]`"* is **not unconditional**. `goal.plan`
refreshes into the same world the dump binding holds, so a script that plans
and *then* dumps was already carrying contents. The failing case is a dump
taken **before the first plan** — which is exactly `dump_map.lua`, the script
that produces every map this project measures against. The conclusion stood;
the mechanism was narrower than I described.

### What this unlocks

**The `Withdraw` path is now reachable offline.** The divergence failure that
just stopped green (`tried to remove 141 iron-plate but removed 100`) may now
be reproducible in seconds rather than by paying for a 19-minute prelude.

Red verified byte-identical the strong way: binaries built with and without the
change, `--steps` listings diffed — **281 lines identical**.

## Divergence is now the dominant failure class

`run-1788517971-48257`, seed `31337`, with furnace reuse:

| | |
|---|---|
| red cell satisfied | 18.53 min |
| red witness | **18.75 min** — sixth witness |
| green | **`stuck`, 6 iterations, best 92 steps** |
| furnaces placed | **19** (42 → 21 → 19 across three runs) |
| plans | 7 |

```
last error: tried to remove 141 iron-plate but removed 100
```

**4 of 5 action failures are world-model divergence** (`removed N of M`). The
planner believed a container held 141 iron plates; the game had 100.

This is the gap that was deliberately held back as "only ever found by
accident" — it is now the thing stopping green, and it is the same class as
`cells_standing` reading three producing cells as zero. The recovery design
independently found it at **7 of 28 action failures across 21 runs** and noted
that retrying "issues the identical command to the identical container", so
tier-1 recovery would not help without a `divergence_observed` predicate.

### Furnace reuse: the trade shows up in execution

**Red took 18.53 min here against 13.63 in the previous `31337` run**, while
its *planned* makespan moved only +8 ticks. Fewer furnaces means more
serialisation, and the reuse agent said so — *"the saving is ground and stone,
not time"*. The runs differ in collision history so this is not a clean
before/after, but **it is the first sign that the trade has a real execution
cost**, and it deserves a controlled comparison rather than an assumption.

Green did converge much further than before (6 iterations, best 92 steps,
against 1 iteration last run), and 19 furnaces left room for the ore cell.

## Self-sustaining green: composition rejected, and the target was 0.44% (`31c8d579`)

**Both load-bearing premises of my brief were wrong.**

1. *"an inserter cell avoids the pole geometry"* — it does not.
   `assembly_spec` requires the **product** recipe to have exactly two
   ingredients, and `inserter` has three, **so an inserter cell does not exist
   today either.** Composition moves the three-ingredient problem from the
   intermediate to the product; it does not remove it.
2. *"reuses machinery that works"* — `Stockpile` is **bot-mediated**: a
   supplier deposits, the owner withdraws. **Nothing in the planner ever takes
   items out of a cell's product machine.** Routing one cell's output into
   another's chest needs an output inserter and a composed layout — new
   geometry, not `Stockpile`.

### The arithmetic loses, and nothing amortises

Per 9,000-tick charge: saves ~**700** ticks of hand-crafting, costs ~**1,640**
in placements and recipes, plus thousands of ticks mining and smelting ~66 iron
and ~8 copper. **Four of the inserters it costs are a third of the twelve it
exists to save.** And there is exactly **one** charge — `CELL_CHARGE_TICKS` is
9,000, nothing refills the chest, and `Producing` is satisfied once the cell
stands, so the plan ends.

**The general form, which is the real finding: chest-fed cells compose into
*more* hand-fill points, never fewer.** One chest of 12 inserters becomes three
chests. **Autonomy comes only from attaching an input to something that
renews** — a drill on ore, a furnace — which is `produce`'s stage-1 shape.

### The thing I called "the next step" is 0.44% of the makespan

Green baseline (`producing:logistic-science-pack:6`): **619 actions, makespan
216,474 (1:00:07), utilisation 20.2%.** Bot 1: 360 steps, 91,704 planned,
**124,770 idle**. Bots 2-4: ~28,000 planned, **~189,000 idle each**.

Bot 1's craft time: `automation-science-pack` **25,500** (85 packs) ·
`iron-gear-wheel` 5,580 · `copper-cable` 2,520 · `electronic-circuit` 1,620 ·
**`inserter` 960**.

**`craft 32 inserter` is 960 of 216,474 ticks — 0.44%.** The whole inserter
supply chain is ~2.0%. The makespan is **85 hand-crafted red packs (11.8%)
plus 28,500 ticks of lab time, on one bot, while three bots idle.**

### One new fact, measured and pinned

At all four facings, on the cell's own pole: the intermediate's third feed mouth
is **dark**, but the **product's west `y = 3` mouth is POWERED and unused**, on
free ground already serviced by the existing lane.

**A three-ingredient *product* costs no second pole; a third *feed* chest
does.** `MAX_FEED = 2` is a bound on the feed side alone — the module doc's
phrasing implied a bound on the whole cell, which is what produced my wrong
premise. Pinned by
`the_pole_lights_a_third_product_mouth_but_no_third_feed_row`; previously only
the positive half was tested, so a `POLE_OFFSET` change could have made
`MAX_FEED` wrong in either direction silently.

This also corrects the earlier finding that chest-feeding circuits into an
inserter machine is *geometrically out of reach*: out of reach as an
**intermediate**, in reach as a **product**.

### A pin correction that would have misled the next reader

`actions 202, makespan 30085, utilisation 36.4%` is **`score-map`'s** output.
`plan --goal producing:automation-science-pack:6` is **369 actions / 47,330
ticks / 37.0%**. Anyone diffing the wrong one will conclude red moved when it
did not.

### Where the real increment is

**Not inserters.** Getting the 85 red science packs and the 28,500 ticks of lab
time off the single converging bot. Genuine green self-sustenance is a stage-3
factory — machine-to-machine routing, ~6 assembling machines, 2 smelting lines
— not a next step.

## Recovery: wired, deliberate, and aimed at the wrong failure class (`33dc079f`)

Design at `docs/superpowers/specs/2026-09-04-recovery-instead-of-replan-design.md`.
**Four things I asserted were wrong.**

1. **`obs:recover()` is fully wired to Lua** (`66404580`, five end-to-end Lua
   tests, the log-pairing hazard made unrepresentable at the boundary).
   **There is no API work.** The gap is entirely in `scripts/supervisor.lua`.
2. **It is not an oversight — it is decision D2** of
   `2026-08-31-supervisor-loop-design.md`: *"Replan on completion or failure
   only"*, justified by *"planning is ~1s, cheap enough to redo constantly."*
   `supervisor.lua` was written **3.5 hours after** recovery existed. **The
   premise is true and the conclusion does not follow**: a replan is not only a
   recomputation, it is a **re-decision of layout**, and half the layout is
   already built.
3. **The dominant failure class is a walk, and recovery cannot see walks.**
   Across 21 runs: **28 failed/lost actions against 76 failed walks**, and
   **28 of 57 replans (49%) had no action failure at all.** `recover()` reads
   only `net.actions()`; `ExecutionLog` keeps walks in a separate map it never
   touches.
4. **The agent retracted its own first cost figure.** Last-settle→next-plan
   gaps looked like 143 game-minutes of planning; measured against
   `batch_progress` heartbeats they are a median of 1,396 ticks. The big gaps
   were the executor's **deadline waits**. D2's cost premise stands.

### The latent defect this would have hit

**Tier 1 has no loop breaker for a walk-only failure.** A failed walk halts the
bot; `abandon_rest` publishes `Failed` on the watch channels but writes
**nothing to the log**, so abandoned actions stay `Pending`.
`exhausted_tier_one` scans for `Failed` with `attempts >= 3`, and nothing is
ever dispatched, so no attempt count rises. **`recover()` would propose
`Rescheduled` forever**, and `MAX_TIER_ONE_ATTEMPTS` cannot stop it because the
budget is denominated in a unit the failure never produces. Latent only because
nothing calls `recover`.

### The replan's cost, proven in one line

`run-1788481380-80843`: 194 steps, 154 succeeded, one placement failed on a
standing character **that cleared 53 ticks later**, and the replan discarded
**11,966 ticks**. The pole placed at tick 13,813 at `[10.5, -41.5]` was
stranded — the replan re-sited the whole power plant **65 tiles away** to
`[-5.5, -57.5]`. No amount of cheap planning offsets that.

### 38 of 57 replans (67%) had a trigger tier 1 is designed for

| n | trigger | tier 1 |
|---:|---|---|
| 28 | walk failed only | yes, **but needs a budget** |
| 10 | character in placement footprint | yes — the canonical case |
| 8 | **no failure at all** | **no** — replan is correct |
| 7 | `tried to remove N but removed 0` | costs 2 wasted dispatches |
| 3 | action `lost` | **dangerous** — retries a possibly-executed action |
| 2 | mining blocked by a character | yes |

### The trade is symmetric, which corrects my framing

I argued a replan is *robust* because it re-derives from the world. But
`propose()` already reads a fresh `PlanState` off the live world and
`schedule()` re-checks every precondition — continuation inherits only **site
choices, quantities and chain bindings**. So: continuing risks finishing a
layout the world outgrew; **replanning strands the half that already stands,
unconditionally, every time.** And the `cells_standing` incident proves
re-deriving layout from a *wrong* world model is not robustness either.

### Recommendation, in three sizes

- **S0 (~half a day)** — `cause` on `PlanCreated` plus a discarded-step count.
  **Nothing today distinguishes a first plan from a replan**, so the
  justification for the whole change is currently unmeasurable.
- **S1 (~2-3 days, `scripts/supervisor.lua` only, no Rust change)** —
  tier-1-only recovery with all three defects fixed inside it: refuse
  `reexpanded`, refuse any run with `obs.lost > 0`, cap at 2 recoveries per
  plan lineage, and **abandon when `obs.success` does not strictly increase**,
  which catches the walk loop on its first repetition in three lines.
- **S2 (deferred, `crates/planner`)** — site affinity in expansion, the actual
  fix for the stranded pole.

**Savepoints do not change the calculus**: `--resume-from` restores the
Factorio world only — new run dir, ladder from milestone 1, no supervisor
state, no plan, no log.

**Never executed in a live run:** the walk-refusal demotion
(`schedule.rs:421-438`) has never picked a different bot, because nothing has
reached it.

## The crash is fixed; the furnace reuse gap is now the binding constraint

`run-1788513716-64336`, seed `31337`, with the double-spend fix:

| rung | result |
|---|---|
| 1 — red cell | satisfied |
| 2 — red witness | **WITNESSED** — fourth time |
| 3 — green cell | **`stuck` after 1 iteration** |

No crash — `3b79eb20` held. The refusal is the ore-cell one again:

```
no room for a iron-ore cell within 12 tiles of the patch: a drill needs to
stand on the ore with a furnace two tiles ahead of it standing off it
```

**But it now fails on the FIRST iteration, where the old map took six.**

### Why: red alone places 42 furnaces, and the seed put the ore underneath them

Red's cell and witness placed **42 stone-furnaces**, spread `x −18..33,
y −49..−12`. On `31337` iron ore is **18.4 tiles from spawn** — so those
furnaces land on and around the very patch green needs. `3ba0d441` reserves six
cell sites; 42 furnaces overwhelm that.

**The seed exposed a tension nobody had stated.** `31337` was chosen for *short
walks* — iron at 18.4 tiles against 40.4 — and that inadvertently made
*crowding worse*, because everything then competes for the same ground. Short
walks and room to build are not the same objective, and `score-map` only scores
the first. Whether `31337` is net better is now **an open question, not a
settled one**; the old map's green run reached six iterations before crowding
out, this one reached one — though the runs are not directly comparable, since
this one built red's whole factory first.

### This makes the reuse gap the thing to fix

`smelt_steps` commits a furnace and **never releases the commitment**, so a plan
needs one furnace per `Smelt` goal — 28 furnaces for 276 ore in an earlier run,
several smelting a single ore. It has now been named as the disease twice and
deferred twice as "its own piece of work". It is the reason 42 furnaces exist,
and fewer furnaces helps **both** halves of the tension above.

Dispatched, with red explicitly allowed to move — unlike every other change
this week — provided the reason is stated and the before/after reported.

## It was not a shortfall — the plan double-spent its own stock (`3b79eb20`)

**Bot 1 held ZERO iron plates**, not 141. `samples.jsonl` at tick 56,460:
`{"coal":5,"copper-cable":1,"small-electric-pole":2,"wood":3}`. The 141 was a
number in the planner's own ledger, and the nine-plate gap was **the same stock
counted twice**. There was nothing to mine, so my framing — "green's expansion
meeting a bot nine plates short" — pointed the fix in exactly the wrong
direction.

### The mechanism

- Green asks `Have{iron-plate, 150, Share(bot 1)}` for `craft 75 iron-gear-wheel`.
- `Withdraw` empties 17 furnaces into bot 1 and recurses.
- The recursion sizes itself at `150 − 17 = 133` and builds a cell for the rest.
- That cell needs a drill, and **the drill's own `Have{iron-plate,3}` and
  `Have{iron-plate,6}` subgoals see those 17 plates sitting unreserved**, plan
  nothing, and spend 9 of them.
- `take 133 from the cell` lands on the 8 remaining → **141**. The craft demands
  its promised 150. `PlanState::lose` refuses.

`shortfall` credits stock in hand towards a `Have` goal, but **nothing recorded
that credit**, so the goal's *own subtree* could spend it. This is the
shared-intermediate defect the reservation machinery exists to prevent,
arriving through a door it did not cover.

### The fix, and why it is not a refusal

`expand_goal_body` now reserves `min(count, available)` for a `Goal::Have`
across its own expansion — measured **before** `method.expand` (that is the
number the shortfall was computed from) and applied **after** it (reserving
first would hide the credit from the sizing that describes it). Released on
every exit path including errors.

**I was wrong to ask for this to become a named refusal.** `refusal_for`
deliberately classifies `InsufficientItems` as a *fault*, not a verdict, because
"it means the feasibility check and the effect disagree" — which is exactly what
happened. Filing it as a refusal would have recorded a planner defect as a fact
about the map.

Red byte-identical: 290-line `--steps` listing, md5 `d097106e…`, and structurally
unable to move since no bot holds an iron plate at t=0.

### The retry window: do NOT widen it, and my suggestion was refuted

At the refusal, `samples.jsonl` puts **bot 2 at (31.34, −47.34)** — dead centre
of the footprint — **mining copper ore**, its action running ticks
20,689→21,172 (483 ticks). And `step_aside_from_footprint` only steers a
blocker when `state.walking == nil and state.mining == nil`, so **it skipped bot
2 entirely.**

The retry's premise — "the blocker has been asked to move; has it gone yet?" —
was **false**. No clock fixes a blocker nobody asked to move. That also refutes
my own suggestion of waiting until the step-aside walk settles: **there was no
step-aside walk.** It would fix the `c2b2698a` case and do nothing here.

What would justify a number is a different *signal*, not a longer clock: the mod
already knows whether it steered the blocker and does not say so in the reply.
Left unfixed.

**And the record cannot tell the difference**: the settle carries
`elapsed_ticks: 0`, so "retried four times and failed" is indistinguishable from
"failed instantly" — another instance of the project's own "silence is not
success" pattern.

### A real limitation of the offline loop, found here

**`world.dump` never calls `Planner::refresh_buffers`**, so a dump's
`inventories` is `[]`, and the `plan` CLI does not refresh either — only the Lua
`goal.plan` path does. **The entire `Withdraw` path is unreachable offline.**
A plain resume of the failing world plans fine (530 actions); reproducing this
needed the sampled inventories *and* 26 furnaces holding a plate each, injected
by hand. The 4-second loop has a blind spot, and it is exactly where this bug
lived.

## RED WITNESSED ON THE BENCHMARK SEED — and green crashes on nine plates

`run-1788509918-33958`, **seed `31337`**, roster `[1,2,3,4]`. The first result
in this project on a map anyone can regenerate.

| rung | result |
|---|---|
| 1 — red cell producing 6/min | satisfied, 2 iterations (363 steps) |
| 2 — red witness | **WITNESSED** — third time, first reproducible |
| 3 — green cell | **CRASHED** |

```
RAISED: runtime error: goal: bot 1 has 141 iron-plate, needs 150
RUN FINISHED state=crashed
```

**A nine-plate shortfall raised a Lua runtime error and killed the run.** That
is the defect. A shortfall is an ordinary planning condition — mining and
smelting nine plates is work this planner does constantly. It should be planned
for, or at worst **refused by name** the way `PowerPlantNeedsWater`,
`ResearchNeedsRoom` and "no room for a iron-ore cell" are. Crashing loses the
run and everything after it.

Note what preceded it: rungs 1 and 2 **succeeded**. This is not a broken world,
it is green's expansion meeting a bot nine plates short.

### The placement collision recurred, and the retry was not enough

```
first error: cannot place item 'stone-furnace' because a character is
standing in the footprint          actions(failed=1)
```

`c2b2698a` retries 4 times over 1.8 s, a window sized at **twice the single
measurement available** (a blocker that cleared in 53 ticks). One sample was
never much of a basis, and here the window was too short. The replan recovered
and red still satisfied, so this cost iterations rather than the run.

### What the seed change did and did not do

Red satisfied in **2 iterations / 363 steps** here against 1 iteration / 263
steps on the old map — the collision cost the extra iteration, not the map. The
planned makespan for `researched:automation` on `31337` is **29,000** against
30,077, so the map is better on paper; **no clean end-to-end timing on it exists
yet**, because this run crashed before finishing.

## AFTER GREEN: oil, and why exploration comes first

The owner asked whether oil processing is the next hard milestone and whether
we are ready for it. **We are not, and the first blocker is exploration** —
their own suggestion, confirmed by evidence in seconds rather than by a run.

### The planner cannot see oil, and never will as things stand

Resource kinds visible on the reference map:

```
coal, copper-ore, iron-ore, stone
```

**No crude oil.** `force.chart`, `chart_area` and `request_to_generate` appear
**zero times** in `mods/BotBridge/control.lua`. The mod replays the chunks that
already exist once at `whoami("server")` and thereafter only reacts to
`on_chunk_generated` — measured at **418 chunks, tiles `[-320, 320)`** — with a
hard drop beyond ±512 at `control.lua:1311`. **Nothing in this system ever asks
the game to reveal new ground.** Crude oil is normally further out than that.

### The gaps, in order

| gap | why it blocks |
|---|---|
| **Exploration / charting** | Oil is invisible, so nothing downstream can be planned at all. |
| **Fluids** | `basic-oil-processing` takes **100 crude-oil**, a fluid. The cell model moves *items* with inserters; the only fluid handling anywhere is the fixed pump→boiler→engine chain. Pumpjack→pipe→refinery is a different shape. |
| **Steel** | Refinery 15, pumpjack 5, chemical plant 5. `steel-plate` (5 iron → 1) is in no ladder. |
| **Power** | A refinery is ~420 kW against one steam engine's 900, before pumpjack and chemical plant. |

**One gap that does *not* apply yet:** `basic-oil-processing` is
**single-output** in 2.0 (100 crude → 45 petroleum-gas), so the multi-output
problem that would break `assembly_spec` outright waits for *advanced* oil
processing.

### Exploration deserves to be its own milestone

It is independently valuable before oil is even considered: better ore patches,
more furnace ground, and an escape from the crowding that halted the first
green run — `3ba0d441` reserves cell sites on a patch, but a bot that can reach
a *second* patch does not need the reservation.

## Recovery S1 shipped (`ba5211d3`) — and its own spec would have broken twice

A run that does not finish its plan now asks `obs:recover()` before replanning,
and takes **only tier 1**. Implemented as a **second transition** rather than a
mutated one, for a reason the design missed.

**Two defects in the design, each of which would have negated it:**

1. **The design's snippet would have recorded nothing at all.** It mutates the
   `ran` transition to `t.action = "planned"` and returns it, claiming drivers
   record it as a plan. But the mutated table has **no `t.plan`**, so
   `plan_created` never fires — and because the word changed, the `ran` branch
   that writes `record.actions` / `walks` / `teleports` / `refusals` is skipped
   too. **Both recordings lost.**
2. **Rule 4 as literally spelled refuses the class it was written for.**
   `obs.success <= self.chain_success` with `chain_success` starting at 0 means
   a walk-only failure on the first run has `success == 0`, so `0 <= 0`
   **refuses the first recovery — killing all 28 walk-only cases**, which are
   49% of replans. The baseline must be `nil` until a recovery is accepted: the
   rule is about a recovery that achieved nothing, not a first run that did.

All four rules pinned by 15 tests driving the shipped file via `include_str!`.
Walks are handed to a driver once per lineage, keyed
`(bot, step_index, dispatched_tick)`, so a genuine re-walk is still offered
while a survivor of the carried log is not.

Also established **by reading rather than by running**: `obs.success` *is*
cumulative (`build_observation` counts over `net.actions()` against the handed-in
log, and tier 1 keeps succeeded actions in `net` for their lag edges). And
**nothing copies `supervisor.lua`** — release extracts only when
`workspace/scripts` is absent, debug creates it empty and never seeds it, so
both copies needed updating.

Red unmoved: a Lua file on a path `score-map` never loads.

**S1's effect is hard to measure until S0** (`PlanCreated` has no `cause`, so a
recovery and a first plan are indistinguishable). Two mitigations shipped: the
drivers print `-- recovered: rescheduled N`, and **`recovery_limit = 0`
restores pre-change behaviour byte for byte**, so an A/B on one seed is
possible without S0.

**Never executed live:** whether a tier-1 reschedule after a walk refusal
actually reassigns the work. A chain owner may pin the same bot, in which case
rule 4 fires on the first repeat and we are back to today's behaviour — safe,
but worth nothing.

## ✅ RED SCIENCE PRODUCES AT A RATE — and recovery fired for the first time ever

`run-1788528493-60555`, seed `31337`, `factory_stage2.lua`.

```
WITNESSED: automation-science-pack in 1 watched machine(s)
went 0 -> 5 (+5, wanted 5) in 3240 of 5400 ticks, 3134 polls
```

**Five packs in 3,240 ticks — 54 seconds, about 5.5/min against a 6/min
claim.** This is the first evidence in the project's history that a bot-built
cell *sustains* production. The six previous witnesses each proved a single
pack inside 780 ticks and could prove no more, because the terminal was a
machine slot that jammed at four.

| milestone | game time |
|---|---|
| 1 — `researched("automation")` | **8.28 min** |
| 2 — red cell producing 6/min | 15.97 min |
| 3 — **rate witness** | **16.88 min** |

5 plans · **8 furnaces** (42 → 21 → 19 → **8** across four runs, as reuse and
the capacity work landed) · 5 failed actions.

### Recovery ran live for the first time

```
planned 77 steps (best 199) -- recovered: rescheduled 1
```

`obs:recover()` had **never executed in a live run** before this. The tiers
have existed since `66404580` and the supervisor never called them. A failed
plan was recovered into a 77-step remainder instead of discarding 199 steps and
re-deciding the layout.

### What this settles, and what it does not

- **Settled:** a cell the bots built assembles science packs continuously, and
  the milestone that claims it is now backed by a rate rather than a single
  observation. My earlier "red science works" was true of one pack and
  overstated for a rate; it is now true as stated.
- **Not settled:** 5.5/min is *below* the 6/min the rung claims. The witness
  floor (5 in 5,400 ticks) is deliberately weaker than the goal, so the goal's
  own number remains unverified.
- **Not settled:** green. The capacity defect — a furnace output slot holding
  one stack — is **designed (`97922af5`) and not implemented**, and it is what
  stops green.

## The cell now has an output path, and the witness is a rate claim (`2aca0f10`, `911ec3c4`)

**Measured on the bench, which settles it rather than arguing it.** Two
identical assembling machines, same recipe, same chest-and-inserter feed, one
drained and one not, after 17,130 ticks:

| | drained | undrained |
|---|---|---|
| `products_finished` | **28** | **4** |
| `status` | `working` | **`full_output`** |

28 crafts in 17,130 ticks is **exactly nameplate** (600 ticks/pack). The
undrained machine reproduces the reference run's jam precisely.

The cell gains `Role::OutputInserter` at `(-2, 3)` facing east and
`Role::OutputChest` at `(-3, 3)`, appended so the plan is the old sequence plus
two placements rather than a renumbering.

### Three consequences worth stating

1. **`cells_standing` now requires the drain.** Existing standing cells stop
   counting and a replan builds a new one. Deliberate — over-build rather than
   over-claim — and the alternative is a predicate that keeps returning `true`
   for exactly the arrangement being fixed. Tested through
   `update_chunk_entities`, not only the overlay.
2. **The fix broke the old witness, which had to be fixed with it.** Rung 3
   read the *machine's* `output_inventory`; with a drain attached that slot
   sits at zero (bench: 94 packs made, output slot 0, `working`), so the old
   witness would have **halted a healthy cell**. It now watches the output
   chest.
3. **A rate assertion became possible for free, and only now.** `at_least = 1`
   was the strongest claim *possible* while the terminal was a machine slot
   capped at four items — that is why six witnesses all fired inside 780 ticks.
   A chest accumulates monotonically, so rung 3 is now **5 packs in 5,400
   ticks**: one more than the four crafts an undrained machine manages, making
   it specifically a claim **the defective cell could not satisfy however long
   it waited**.

### Red

`researched:automation` **identical** (it builds no cell).
`producing:...:6` goes 369 → **372 actions**, 47,330 → **43,288 ticks**. The +3
is the intended change; **the −4,042 is not claimed as an improvement** — the
extra bill perturbs the crafting subgoals and the critical path happened to
land shorter.

### The queued long-inserter item's premise has expired

`31c8d579` established that a three-ingredient *product* costs no second pole,
because the `(-2, 3)` mouth was powered and unused. **That mouth is now the
output path.** The test was renamed and deliberately inverted on those two
tiles rather than quietly updated. Anyone reaching for a three-ingredient
product must now find a different opening or pay for a pole.

## ⚠ STAGE 2 HAS ARGUABLY NEVER PRODUCED AT A RATE

Found while designing the capacity fix (`97922af5`), and it is the most
important thing on this page.

**1,249 of 1,281 `full_output` samples in the reference run belong to the two
stage-2 red-science assemblers, not to any furnace.** The science-pack
assembler **jammed at tick 74,100 holding four packs, never held more, and
finished four crafts in the entire run.**

**`plan_cell` gives the product machine no output inserter — by design.** Its
roles are `Pole, Intermediate, Product, FeedChest, SupplyChest, FeedInserter,
LinkInserter, SupplyInserter`. **Nothing removes the product.** An assembling
machine stops when its output backs up, which for an assembler is three or four
items, not a stack.

`milestone_satisfied` was recorded anyway.

### What this does and does not invalidate

- **The witness is still true.** It asserts a pack appeared while every bot
  stood still, and one did — inside the first 780 ticks, before the jam. **A
  bot-built machine did assemble science.** That claim stands.
- **`producing("automation-science-pack", 6)` does not.** The rung's own doc
  says it means the cell *stands*, and it does stand — but the number **6/min**
  has never been achieved, and could not be, because the cell physically cannot
  run for a minute.
- **Six witnesses across six runs all fired in the same 780 ticks.** They were
  never evidence of a rate and never claimed to be. **I have been reporting
  them as "red science works", which overstates what was shown.**

### The fix is a cell-design change, not a bug

The cell needs an output path — an inserter and a chest, or a consumer. That is
`assemble.rs` geometry, and the product machine's **third mouth is already
known to be powered and unused** (`31c8d579`), which is where it would go.

Until then, **"produces 6/min" should not be quoted for red or green.**

## Green: resumed from a savepoint, exhausted, and it oscillates rather than converges

`run-1788538389-09170`, resumed from `run-1788532631-48030:2` (red witnessed),
seed `31337`. **First real use of `--resume-from`** — it skipped red's
16-minute prelude and announced its own limitation unprompted: *"this run is
NOT comparable with a fresh-world run, and --compare will refuse to try."*

`exhausted` after 10 iterations, best 200 steps.

### I mis-called the convergence, twice

Plan step counts across the run:

```
406, 295, 219, 264, 231, 210, 134, 213, 172, 136, 189, 114
```

**That oscillates.** It trends downward but bounces up repeatedly — 219→264,
134→213, 136→189. I reported "converging steadily" twice from partial views.
The honest description is a downward trend with repeated regressions, which is
a different and less encouraging shape.

### The remaining failures are the capacity sites we did not fix

11 failures: **5 divergence**, 6 other. The capacity increment deliberately
covered only `PlaceDrill`'s take (`735efeba`); the design named three more
emitters — the smelt bank (`have.rs:1898`), the ore insert (`have.rs:1553/1706`)
and seven fuel sites — and they are still unbounded. **That is the clear next
lever for green**, and it is already specified.

### Two things that did work

- **Provenance is populated for the first time**: `seed 31337`,
  `factorio 2.1.17`, `git 0db94f6f`. Every number from this run is traceable to
  the map and the code that made it.
- **The waiting field earned itself**: a 351/406 pause resolved as
  `take 100 iron-plate · waiting_on=lag_deadline · deadline=187650`, a
  legitimate 20,000-tick wait for one drill-and-furnace cell. Yesterday that was
  frozen counters and a guess — and a guess of that kind once killed a healthy
  run.

`max_iterations` raised 10 → 25 for stage 3, matching the starter.

## THE BENCHMARK, SETTLED — 6:12 is the bar, 9:12 is a strategy

We run **Space Age** (`factorio-space-age_linux_2.1.17.tar.xz`, `elevated-rails`
in the data directory). But **nothing in the automation chain changed**, verified
against our own recipe table:

| | |
|---|---|
| `automation-science-pack` | 1 copper-plate + 1 iron-gear-wheel, 5 s |
| `iron-gear-wheel` | 2 iron-plate, 0.5 s |
| plate smelt | 3.2 s |
| `lab` | 10 gears + 10 circuits + 4 belts |
| boiler · steam-engine · offshore-pump · circuit · pole | unchanged |

So the Space Age run's **9:12 is not a speed limit** — that run front-loads
infrastructure for a 3h17m game and spends more here on purpose. **6:12 is the
demonstrated-achievable bar**, and it stands.

| | automation |
|---|---|
| demonstrated achievable (pre-SA WR, same recipes) | **6:12** |
| Space Age WR split (a strategy, not a ceiling) | 9:12 |
| **ours, seed `31337`** | **8:17** |
| ours, old unidentified map | 8:41 / 8:43 |

**We are about two minutes behind achievable**, not ahead of anything.

### I got this wrong twice in one turn, in opposite directions

First measuring against 6:12 without checking which game we ran; then swinging
to 9:12 as though it were a speed bar because it was the Space Age number. The
resolution is neither: **the version differs, the recipes do not, so the older
record's time is still the thing to beat.**

And the flattery in our own number is unchanged and still real: **four bots
against one human, on a map chosen for short walks, doing only automation.**

### What it means for the horizon

3h17m for a rocket remains the long-range target, and the capability gaps —
**exploration** (nothing here ever charts, so oil is invisible), **fluids**,
**steel**, **multi-output recipes** — are what stand between us and it. Another
minute off rung one is not.

## Capacity finished (`edf6d6fa`) — and it probably does NOT unblock green

A furnace is a slot, so a long smelt is now a **sequence of visits**:
`runs_per_load()` takes the minimum across input, output and fuel, and
`BankFurnace` emits one (insert, fuel, take) cycle per load, each load ordered
behind the take that emptied the slot. Fuel splits into stack-sized visits
chained by burn time, timed from the **first** visit — the machine starts there
and runs across refuels.

`have:steel-plate:150` now plans entirely within capacity: `insert 750` → 7×110
(cap 120), `take 150 steel-plate` → 7×22 (cap 100), `fuel …113 coal` →
50/50/13 (cap 50).

**All three bounds bind, and the input is tightest for iron** — capping output
at one stack of 100 plates still asks 100 ore into a source slot that takes 70.
**An output-only cap would have looked correct and fixed nothing.**

### Red did not move, and my brief was wrong to expect it to

`score-map` identical at **136 / 28,897 / `dfac0f4caa0a7500`**;
`producing:automation-science-pack:6` identical at 299 / 43,797; green's
`--steps` **byte-identical**. Reason rather than luck: every transfer either
plan makes is already inside its cap, so every `loads` and `fuel_visits` vector
has length one and the ids, lags and emission order are the ones that were
always there.

### The correction that matters: green may not have been blocked on this

**Green's offline plan was already entirely within capacity before this
change** — max take 100, insert 12, fuel 23 — and is byte-identical after it.
`735efeba` had already fixed the one site green hits.

So **"5 of green's 11 failures are divergence from these sites" is
unsupported.** Those failures came from a **mid-run replan against a different
world**, not from the baseline plan, and that cannot be verified without a run.
**The next action is to run green, not to assume it is fixed.**

### Three more corrections

- **"Seven fuel sites" is five.** Two of the design's are inside `#[cfg(test)]`
  and one is not a fuel insert. Of the five real ones, **only two were
  genuinely unbounded**; the rest were already bounded by construction or
  arithmetic. All five now route through `slot_capacity` anyway, so a future
  constant bump cannot reintroduce it silently.
- **The shared-ore excess branch is unreachable in practice** — measured, not
  assumed: it fires on none of the four baseline plans and none of ~600 tests.
- The design's `have.rs` line numbers no longer resolve; the sites are
  `smelt_steps`' ingredient, shared-ore and take loops.

### `SlotOverflow` deliberately not shipped

One thing can still trip it: **`Researched`'s lab insert**, sized from the
technology's unit count against a slot holding one stack per science type.
Splitting it needs a lag for "when has the lab consumed a stack", and research
is a single modelled duration a bot is bound to — later inserts would schedule
*after* the research, which is useless. **That is a scheduler change, not a
sizing one.** Shipping the refusal now would turn a latent mis-size into a loud
refusal of any research over 200 packs.

## Rocks: automation's planned makespan drops to 8:01 (`130b3bde`)

Your speedrun trick, implemented. **A `huge-rock` gives 24 coal AND 24 stone in
360 ticks; hand-mining is 120 ticks per unit.**

| goal | before | after |
|---|---|---|
| `researched:automation` | 202 acts / 30,085 (8:21) | **136 acts / 28,897 (8:01)** |
| `producing:automation-science-pack:6` | 372 / 43,288 | **299 / 43,797** |

Red loses **40 hand-mined stone and 24 of 28 coal**. The sharpest pin:
`the_whole_of_stage_one_costs_this_much` **11,025 → 5,773 ticks (−48%)** —
"mine 37 coal" was 4,440 of them.

### It was a method-ordering bug, not a missing capability

**Everything already existed.** `EntityGraph::minables` is filled by `add` for
any minable entity, `minables_yielding` reads `mine_result`, `perform` routes
`Chop` through `act.mine`, and a live test already asserted a `big-rock` yields
20 stone. **`Chop` simply sat *after* `Mine` in the registry**, so mining always
claimed the goal first. Its guard was its position in a list; it is now an
explicit tick comparison, `chop_beats_mining`.

### Two decisions worth keeping

- **The two-item yield**: one `Effect::GainItem` per bill entry, applied to
  `ctx.state` as steps are emitted — so a swing taken for coal credits its
  stone and the next `Have{stone}` is `AlreadySatisfied`. **Surplus is
  credited, never aimed at.**
- **The range**: `products_to_dict` already resolves `24-50` to `amount_min`,
  so `mine_result` is the **floor** of reality. Sized on the average you are
  short on half of all swings and a short delivery forces a replan; on the
  floor you are never short and pay at most one extra swing.

### Three things it corrected

1. **Rock counts** — nearest `big-rock` is **22.6** tiles, not 17.8; nearest
   `huge-rock` is **63.5** tiles, so *coal* costs a real walk.
2. **Two sites still use Factorio-1.1 rock names** (`rock-big`, `rock-huge`)
   and are dead code on 2.x. They are dead on *both* sides of the keyframe
   comparison so they cancel — **but that is luck, not design.**
3. `spawn_rocks` in `test_utils` has an off-by-population bug.

### Two guards caught their own decay, and one inverted

`every_expansion_replays_in_time_order` found its cross-chain edges gone — a
bot holding a rock's surplus is short of nothing, so `worth_handing_a_furnace_over`
correctly refuses.

More importantly, **`the_chest_makes_the_plan_shorter_than_it_is_without_one`
INVERTED**: 30,136 with the chest against 28,951 without, where it used to save
5,338. **The `Stockpile` cost model is now questionable** — the chest is built
for iron ore, which no rock yields. Rocks exposed it rather than caused it, and
the test now bounds the cost at 5% and asserts neither sign.

### One genuine regression

Bot 1 runs **106 of 142 steps** (was 90 of 173). A goal claimed by one chain
takes its welded crafts with it, and **a rock is indivisible**. The plan is 29%
faster and every bot's absolute load fell, but the roster is used less evenly.
`Chop` behind `Stockpile` was measured as the alternative and is much worse.

## The oil chain, measured on a bench — and I got it wrong first

**Owner challenged the claim and was right.** I said crude oil is hand-minable,
reading `mineable_properties.minable = true`. That flag is what a **pumpjack**
uses. Tested directly:

```
character.mine_entity(crude-oil) -> false      wells remaining: 5 (unchanged)
```

**A character cannot mine crude oil.** I asserted a surprising thing from a
prototype field instead of doing it, and it took one query to disprove.

### The actual chain

```
fluid-handling    50 x (red+green)   prereq automation-2, engine  -> storage-tank, pump, barrels
      |
oil-gathering    100 x (red+green)                                -> pumpjack
      |
oil-processing   trigger: mine-entity crude-oil, i.e. a PUMPJACK  -> oil-refinery, chemical-plant,
                 extracting; 0 science                               basic-oil-processing
```

**Oil is gated behind GREEN SCIENCE — 150 green packs before a pumpjack
exists.** So "green and oil in parallel" was wrong as stated: the *tech* path
depends on green. Oil **infrastructure** work can still proceed in parallel
using cheated techs, which is the accelerant; the honest path cannot.

**And the exploration agent was right where I overruled it.** It flagged that
the planner will plan hand-mining crude oil as *a trap now reachable*. I
reframed that as "the required mechanism". It is a trap: the action is
impossible, and `Mine::applicable` gating only on `has_resource_patches` will
emit it.

### Space Age gates early recipes behind trigger technologies

On a fresh world, `pipe`, `boiler`, `offshore-pump`, `lab` and `inserter` are
**all disabled**, while `iron-gear-wheel`, `stone-furnace` and `transport-belt`
are enabled. The early progression is trigger-driven:

| tech | trigger |
|---|---|
| `steam-power` | craft **50 iron plates** -> boiler, pipe, offshore pump |
| `electronics` | craft **10 copper plates** |
| `automation-science-pack` | craft **1 lab** |
| `steel-axe` | craft 50 steel plates |
| `oil-processing`, `uranium-processing`, `calcite-processing`, … | mine a named entity |

**Our runs trip these incidentally and the planner models none of them.** That
is why a fresh world shows the recipes disabled and our runs still work.

### The development accelerant is proven

On a bench: cheating `oil-gathering` / `oil-processing` / `fluid-handling`
enables pumpjack, refinery, chemical plant and storage tank; five crude-oil
wells created; **a pumpjack placed successfully**. So the oil chain can be
built and tested now, against cheated tech and cheated oil, with the honest
path required only for a measured run.

## Owner decisions (2026-09-04, on the chunk-ingest finding)

1. **Measure the free vision now, flip the default later.** Record how much
   ground each run was given without visiting it, make charted-only an option,
   and flip once milestones survive it. Flipping first would collapse the model
   to ~418 chunks and break every milestone.
2. **Green and oil in parallel.** Green's remaining blocker is three specified
   capacity sites; oil's is a trigger payload the mod deliberately omits. They
   are in different crates.
3. **The 8:17 automation result stands, with a footnote.** Everything
   automation needs — iron 18.4, copper 54.9, coal 32.1, stone 33.3, water 48.1
   — is **within ~68 tiles**, well inside what a bot's own walking would
   legitimately reveal. The free vision reached 505 tiles and was not needed
   for this milestone. **Record the caveat, not a doubt**; do not mark past
   numbers provisional.

## The world model gets free vision — a disclosure item, not an incident

`918f0d6d`. **Owner's calibration (2026-09-04): do not over-weight this.**
Cheating during exploration and development is fine and carries no penalty;
only the **"real" runs** need to achieve everything as honestly as we can. So
this is something to *disclose and eventually flip*, not something to stop for.

**And it cuts the other way, usefully:** because cheats are fine in
development, a capability can be built and tested *before* its honest
prerequisite exists. **Oil does not have to wait for exploration** — cheat oil
into view, develop and test the whole oil chain against it, and require the
legitimate route only when a run is being measured. That decouples the two
biggest items on the rocket path from each other.

**`on_chunk_generated` ingest — what this project does today — is
`force.chart` with extra steps.** In `run-1788532631-48030` the **furthest any
bot ever travelled was 63.8 tiles**, and the world model it produced holds
crude oil at **380 and 505 tiles**, 559 uranium tiles, and 36 biter spawners
out to 500. The hook catches chunk *generation* and never consults the force's
charted area, so the model sees ground nobody has been near.

**Do not rip it out.** Flipping to charted-only collapses the model to ~418
chunks and breaks every milestone. The order is: **measure it in provenance →
make it optional (`is_chunk_charted` + `on_chunk_charted`) → then flip the
default.**

### Three things I asserted that were wrong

1. **"The planner cannot see crude oil."** False for a *resumed* workspace.
   `provenance.json` for `run-1788538389-09170` records `crude-oil: 12` and
   `uranium-ore: 559`. The four-resource list is what a **fresh** map holds;
   charting grows, and `ResourceFingerprint`'s own doc already said so.
2. **"`researched:oil-processing` cannot begin because oil is invisible."** It
   cannot begin — **for a different reason**. `oil-processing` is a **trigger
   technology** (`research_trigger: {type: "mine-entity"}`, zero science), and
   `mods/BotBridge/types.lua:265-291` deliberately sends `mine-entity` triggers
   **without a payload**, so the planner raises `UnsupportedResearchTrigger`
   whether or not a well is charted. **Charting does not unblock oil; fixing
   the trigger payload does.** Everything up to `oil-gathering` (100 red+green)
   is science-only.
3. **"Exploration is the missing mechanism."** The mechanism is not missing —
   the honesty is. See above.

### Legitimacy, settled

| mechanism | verdict |
|---|---|
| bot walks there | **legitimate** — the reference; the honest price is the price |
| **radar** | **legitimate, and cheaper than assumed**: 20 red science (prereq already met), 10 iron / 5 gears / 5 circuits, 300 kW |
| `force.chart` | **cheat** — development only, must be recorded |
| `freeplay.set_chart_distance` | **neither** — sets `storage.chart_distance` once at scenario init; a map property, declare it in provenance |
| `on_chunk_generated` ingest | **cheat, uncounted** — see above |

### Two prerequisite defects, not follow-ups

- **Nothing notices a bot has died.** The roster is computed once at script
  start and never recomputed, and `start_walk_waypoints` returns a bare `false`
  for `character == nil` — **indistinguishable from "not connected"**. Nearest
  charted enemy structure is **246.6 tiles**; oil is at **380**, so any oil
  route leaves the safe radius. **This promotes radar above walking**: it risks
  no bot at all.
- **The planner will plan hand-mining crude oil.** `Mine::applicable` gates
  only on `has_resource_patches`, and that trap is *newly reachable* because
  the model now contains oil.

### Implemented

`EntityGraph` gains a `threats` map (`unit-spawner` + `turret`, by name and
tile) with `nearest_threat` / `threats_from` / `threat_census`. **The mod
always sent these** — `EntityType::from_str` returned `Err` for all three enemy
spellings, so they left only an anonymous rect in `blocked_tree`. Live biters
excluded deliberately: a stored unit is a permanent phantom.

### And the ±512 wall

From the mod's **first commit in 2021, with no rationale**. It drops entities,
tiles *and* the map-area update, admitting exactly 1,024 chunks — **the binding
constraint on this workspace, not the game**. Oil patch A's westernmost well
sits **2.5 tiles inside** it. Recoverable: `initial_discovery` re-emits
everything on the next boot.

## WHERE THIS STANDS (read this first)

**Sections below are reverse-chronological — newest first.** The plan began as
a set of workstreams; most are now landed and the document is as much a record
as a plan.

### Results

**Benchmark: 6:12** — the demonstrated-achievable automation time. We run Space
Age, but **the automation chain's recipes are unchanged** (verified against our
own table), so the Space Age record's 9:12 reflects that run front-loading
infrastructure for a 3h17m game, not a harder milestone.

| goal | before | now |
|---|---|---|
| `researched("automation")` | 21.34 min | **6:11** measured on a fresh seed-`31337` map (`run-1788582657-14978`, release, one plan, zero failures); the record is 6:12 |
| red science cell standing | never satisfied, `stuck_silent` | satisfied in 1 iteration |
| red science producing **once** | never observed | witnessed six times, all inside 780 ticks |
| red science producing **at a rate** | impossible to claim | **5 packs in 3,240 ticks (~5.5/min)**, twice |
| green science, planning | did not expand at all | plans end to end |
| green science, live | never run | **WITNESSED thirteen times** — fresh world **14:26 / 15:02** (`run-1788635061-85457`), one 569-step plan, zero failures, 69.5% utilisation, executed/planned 0.984; 64:22 → … → 15:19 → 14:26 |
| furnaces per run | 42 | **8** |
| recovery (`obs:recover`) | never executed in any run | **fires live** |

**We are ~2 minutes behind achievable**, and the comparison flatters us three
ways: four bots against one human, a map picked for short walks, and a run that
does only automation.

### The two claims that were overstated and are now correct

- **"Red science works"** was true of *one pack*. The cell had **no output
  path**, so its assembler jammed at four items and every witness fired in the
  same 780-tick window before the jam. Fixed (`2aca0f10`); the witness is now a
  rate claim the defective cell could not have satisfied.
- **"Green converges"** — it does not. Step counts oscillate: `406, 295, 219,
  264, 231, 210, 134, 213, 172, 136, 189, 114`. Downward overall, with repeated
  regressions.

### What blocks green now (refreshed 2026-09-05 00:50)

**Not capacity.** Runs 3–5 on the red-witness savepoint, with rocks executing
live, showed the real classes and each has a mechanism fix on master:

| class | what it was | fix |
|---|---|---|
| walk refused at a rock | Chop aimed at the rock's own centre | `98ca85d8` stand beside it; `76f3e153` reach measured to the box |
| `removed 40 of 64` from a cell | drill sited on the patch's thin rim, mined its four tiles dry | `bdec88af` yield-aware siting; `7c93d57e` refuel and drain standing cells at replans |
| `removed 5 but 3` from a furnace | fuel landed 8,000 ticks after a shared insert; fuel edge carried no lag | `330a9a39` |
| tier-1 re-issued an identical take | recovery could not see divergence | `c1c41412` |
| `craft 75 packs` lost at 360 s | flat deadline shorter than the craft | `fffdb37e` |
| bot 1 walled in by its own placement | stand-point chosen blind; enclosure fill on an eighth-tile grid found cracks the game cannot use | `79f3f9d3` |
| bot idle 24 min waiting on one furnace | the list scheduler's `(end, id)` key | `51c7f695` lookahead key |

What remains structural: three bots idle while bot 1 hand-crafts 75 red packs
and researches; science is not machine-made across milestones; copper is
mined in minute one. Those are the world-record lessons
(`docs/superpowers/notes/2026-09-04-world-record-replays.md`), and they are
ladder decisions (a rate rung such as `producing:iron-plate:60`), not fixes.

### Landed

| what | commit |
|---|---|
| Lag clock starts at the predecessor's finish (F) | `c8ef0d91` |
| Per-bot concurrency for background actions (C) | `421de15c` |
| Shared chest so gathering can move between bots (B) | `0069b41c` |
| Offline planning from a dumped world (0) | `db612be9` |
| Map scorer (0b) | `a5b31c80` |
| Milestone savepoints and resume (G) | `a5dc71da`, `7e9fabc6` |
| Retry a placement a bot is standing in | `c2b2698a` |
| **A recipe set over RCON reaches the world model** | `30b28846` |
| Run provenance, `--seed` fix, `--compare` | `61ec7364`, `ccaf562f`, `a1c1316a` |
| Furnace handover (R3) | `c0c3bc5c` |
| Cell intermediate with two mouths (green) | `240d3efb` |
| A cycle costs an item edge, never a placement | `374d7aa3` |
| A lab brings its own pole | `38869772` |
| `automation_speedrun.lua`, `factory_stage3.lua` | `c6620b24`, `abae10ab` |

### The tools that made the difference

- **Offline planning in ~4 s** against a real world dump
  (`workspace/scripts/map.json`), instead of a 20-minute run. It established
  the green-science gap, the 13:05 ceiling, and B's whole result.
- **`automation_speedrun.lua`** — one goal, for timing.
  **`research_run.lua`** — seven rungs, for diagnosis. Both are worth keeping;
  the ladder costs ~53% and buys a named failure.
- **Milestone savepoints** — `milestone-7.zip` is a world with automation
  already researched.

### Open, and why each is not being rushed

- **`BUFFER_ENTITIES` omits `iron-chest`**, which stage 2 places two of per
  cell, so the planner cannot see what a cell's supply chests hold. Changes
  planning behaviour that cannot be verified offline.
- **`supervisor.lua` never calls `obs:recover()`** — the recovery tiers exist
  and no live run has ever reached them; every iteration replans from scratch
  and discards completed work. The single largest untouched lever.
- **Nothing checks a walk's landing spot against footprints the plan needs
  later** — the root of the collision fixed in `c2b2698a`, which was treated
  at the retry end rather than the cause.
- **Seed `20260903` has never been run or scored.** The map every result here
  rests on is unidentified and survives only as
  `workspace/known-good-map/level.zip`.
- **The flaky determinism test** never reproduced in 18 attempts and no error
  text was ever captured.
- **The 34-furnace bank** crowds out the ore cell that feeds it. Under
  investigation; may be a costing defect rather than a spatial one.
- **A long-handed inserter may unlock the third feed mouth — unmeasured.**
  Owner's point, queued rather than actioned. The mechanism is real and
  **already modelled**: `state.rs:311` gives `long-handed-inserter` a reach of
  `2.`, the doc says it "skips a tile on both sides", and
  `a_long_handed_inserter_reaches_two_tiles_on_each_side` pins it. It is
  craftable from iron plate + gear + inserter with no tech gate.

  **`MAX_FEED = 2` is a bound on pole *coverage*, not on inserter reach** — the
  intermediate's third west mouth is unpowered and an inserter there will not
  run however far it reaches. But **a long-handed inserter need not occupy that
  dark tile**: it can sit in a powered tile two away and still reach the
  machine. `the_pole_lights_a_third_product_mouth_but_no_third_feed_row`
  measured the *standard adjacent row* at all four facings; **whether a powered
  tile at distance 2 can host a long-handed feed has never been measured.**

  **Payoff for green is 0.44%** (`craft 32 inserter` = 960 of 216,474 ticks; the
  whole inserter chain ~2%), which is why it is queued and not urgent. It
  becomes **structural at stage 3+** — oil refineries, chemical plants and
  anything else with three or more ingredients. Whoever picks this up should
  start from `crates/planner/src/method/assemble.rs`'s module doc, which already
  works through the three-ingredient case on the *product* side; this extends it
  to the *feed* side by reach rather than coverage.

- **Nothing checks a cell's servicing lane when siting the lab**, and
  `enclosure::check` runs for cells and plants but not for the lab. Does not
  bite on this map; the check simply does not exist.

## Owner decisions (2026-09-03, before an unattended night)

1. **Success is game time, from run start to milestone satisfied.** Not
   roster-ready, not wall clock. The reference run is 21.34 min on that
   clock. The target is **under 9 minutes**; the single-player world record
   researches automation at **6:12**.
2. **Workstream C is approved for full implementation**, including the
   executor's per-bot concurrency change — not design-only.
3. **Savepoints are preferred over cheat hatches.** Cheating and teleporting
   are **fine during development**, to reach an untested stage quickly. The
   **final measured runs must be as cheat-free as we can get them.** Any run
   that used a cheat path must say so in provenance, and `--compare` must
   refuse to compare it against an honest run. This promotes workstream G
   (milestone saves) from "queued" to the supported way of making runs cheap.

## The measurement that reframes the problem

From `workspace/runs/run-1788465258-49050` (21.34 min game time, roster
`[1,2,3,4]` confirmed via `plan_created`).

### Counting rules — read these before quoting a number

Three quantities here are easily confused, and confusing them has already
produced two wrong versions of this section:

- **Action ids are per-plan, not global.** This run replanned twice
  (`plan_created` at ticks 4,642 / 23,092 / 81,514) and ids are reused
  across plans: id 38 is `craft 2 iron-gear-wheel` in the first plan and
  `craft 1 offshore-pump` in the third. **44 of 147 ids collide.** Keying any
  aggregate on `id` silently drops those 44 settles and understates executing
  time by 8,067 ticks (2.24 min). An earlier version of this plan did exactly
  that. **Aggregate over settle *events*, never over ids.**
- **147 dispatch events = 147 distinct actions**, across three plans. There
  are no repeated dispatches of the same action, and no wasted walks to
  already-satisfied actions — a hypothesis that looked strong until the id
  collision explained it away.
- **"Planned steps" (96) ≠ dispatched actions (147).** A replan re-emits
  remaining work.

### Bot 1, over a span of 70,616 ticks (19.62 min)

| | ticks | time | share |
|---|---|---|---|
| **executing actions** | 31,464 | **8.74 min** | 44.6% |
| **walking** (`walk_settled`) | 14,330 | **3.98 min** | 20.3% |
| **neither** — acting nor walking | 24,822 | **6.89 min** | 35.2% |

Executing time by verb:

| verb | actions | ticks | time |
|---|---|---|---|
| `mine` | 39 | 18,890 | **5.25 min** |
| `craft` | 34 | 6,575 | 1.83 min |
| `research` | 1 | 5,999 | 1.67 min (lab time — a hard floor) |
| `place` / `fuel` / `insert` / `take` | 61 | **0** | free |

**Mining is the single largest activity in the run**, larger than crafting
and research combined.

### The whole roster

| bot | executing | walking | busy |
|---|---|---|---|
| 1 | 31,464 | 14,330 | **12.72 min** |
| 2 | 1,691 | 435 | 0.59 min |
| 3 | 1,691 | 613 | 0.64 min |
| 4 | 1,693 | 591 | 0.63 min |

**14.59 bot-minutes of work across 85.4 bot-minutes available — 17.1% roster
utilisation.** All 34 crafts went to bot 1; bots 2–4 crafted nothing.

### What this means

Against a single-player world record that researches automation at **6:12**,
bot 1 is *busy* for 12.72 minutes and the whole roster does 14.59 bot-minutes
of work. So there are two distinct gaps, and they need different fixes:

1. **Bot 1 idles 6.89 minutes.** Workstream F establishes that essentially
   **all** of it is a single executor defect — the F diagnosis independently
   measured 24,583 ticks of lag-clock waiting against the 24,822 ticks of
   "neither" computed here, i.e. ~99%. This is a bug, not a capacity problem.
2. **Three bots are idle almost the entire run.** That is the
   `Holder::Share` ceiling, addressed by R3 (`c0c3bc5c`) — which landed
   **after** this run and has therefore never been measured live.

Two structural facts explain why nothing fills the idle:

- **The executor gives each bot exactly one action at a time.** Measured: 0
  of 99 consecutive same-bot dispatch pairs overlap.
- **The planner cannot express work that has no consumer.** Every `Goal`
  variant (`Have`, `Researched`, `Produced`, `Producing`, `All`) is
  demand-driven.

## THE STARTER FACTORY: red witnessed again, green wants a second plant

`run-1788504490-09380`, `scripts/factory_starter.lua`, roster `[1,2,3,4]` —
red and green science in one run, four rungs.

| rung | result |
|---|---|
| 1 — red cell producing 6/min | **satisfied, 13.63 min** |
| 2 — red witness | **WITNESSED, 13.85 min** |
| 3 — green cell producing 6/min | **`stuck`**, 1 iteration, best 736 steps |
| 4 — green witness | not reached |

**Red's witness reproduced.** Same line as `run-1788489532-62404`:
`automation-science-pack … went 0 -> 1 (+1, wanted 1) in 780 of 3600 ticks`.
Twice now, on two different runs. It is not a fluke.

Red built a complete working plant on the way: **1 offshore-pump, 1 boiler, 1
steam-engine, 3 pipe, 1 lab, 2 assembling-machine-1, 4 poles, 3 inserters, 2
iron-chests, 1 drill**, plus 21 furnaces and 5 wooden chests.

### Why one script rather than two runs — measured

| goal | planned |
|---|---|
| red alone | 30,077 ticks |
| green alone | 191,105 |
| **both** | **190,654** |

The pair is **cheaper than green by itself**: red's cell is nearly free once
green's power plant, lab and research have to exist anyway.

### Green's refusal

```
a power plant needs water, and the plan can see none within 128 tiles
```

The rung's own name is *"on the plant red already stood up"* — the intent is
**reuse**. Green asked for a **new** plant instead. That is the defect to
explain, and the water is a symptom of wherever it decided to put one: this map
has water **46.7 tiles** from spawn, and green plans fine at t=0.

Two candidate causes needing opposite fixes, which the error text cannot tell
apart: the search origin moved (green's cells are sited from resource patches,
not spawn), or the shoreline red's pump now occupies stopped counting.

### The savepoints paid for themselves here

`savepoints/milestone-2.zip` **is the world at the moment red was witnessed** —
exactly the state green failed from. With `--resume-from` and `dump_map.lua`,
that world can be dumped and planned against **offline in ~4 seconds**. Every
previous failure of this kind had to be reproduced by paying for the whole
prelude again. Under investigation that way now.

## There was no bank of 34 (`3ba0d441`)

**My brief was wrong about the central fact.** The 34 furnaces in plan 2 are
**6 cell furnaces plus 28 independent hand-smelts, each exactly one furnace
wide** — several smelting as little as **one ore** (`insert 1 copper-ore` /
`take 1 copper-plate`). 276 iron ore and 46 copper, across 28 furnaces.

**`bank_size` is not implicated, and it is in `have.rs`, not `produce.rs`
where I sent the investigation.** It computes
`widest = runs.min(MAX_BANK).min(standing).max(1)`, so with nothing standing it
returns 1 and never builds for lag. **The `bdbde0c9` verdict — that building
furnaces for the lag loses — is intact and already enforced in code.** My
suspicion that a 34-furnace bank was a costing defect was unfounded.

### The real mechanism: a reuse gap, and a perimeter collision

`smelt_steps` calls `commit_machine` on whatever furnace it uses,
`adoptable_furnaces` refuses a committed machine, and **a commitment is never
released** — so a plan needs as many furnaces as it has `Smelt` goals.
Cross-plan reuse does work (plan 3 had 47 smelts and only 13 new furnaces); it
just adds ~13 every epoch, forever.

Those furnaces land on the **patch perimeter**, because ore is the one thing a
furnace may not stand on and `free_area_near` searches outward from the nearest
resource tile. That perimeter ring is *exactly* where a cell's furnace must go
— two tiles ahead of a drill standing on the ore. Measured on the run's own
world:

| iron patch | cells that pack at once |
|---|---|
| clean | **15** |
| + the run's 44 standing furnaces | **7** |

About **0.6 cell sites destroyed per hand-smelt furnace, monotonically.**

### The fix reserves ground, and treats the symptom by design

`smelt_steps` gains a tier: while the patch has room to spare
(`CELL_SITES_RESERVED = 6`), siting is unchanged; once it does not, the search
steps around ground where a cell really fits *now*, with the old unguarded
search still the fallback so nothing becomes a refusal.

Six, not `MAX_CELLS` (12): 12 was tried first and **moved red** by one copper
furnace, because this map's copper patch packs 12-16.

Paving the map one furnace at a time, cell count decays
`15, 11, 10, 8, 6, 4, 3, 2, 1` without the reserve and
`15, 11, 10, 8, 6, 5, 5, 5, 5` with it — **a floor of five, against the four
green asks for and the zero the run reached.**

Red byte-identical (290-line `--steps` listing, `diff -q` clean). 72 suites,
1,977 tests.

### Two limitations, and one disease left untreated

- **The exact refusal was not reproduced offline.** Reconstructing the halt
  world from `map.json` plus the 44 recorded furnace positions and expanding
  green there **succeeds** (832 actions) — the live epoch-7 state carries more
  than those furnaces. So the fix is proven to arrest the erosion, **not proven
  to make that run finish.**
- **Green's t=0 plan is byte-identical after the fix** — a fresh patch has room
  to spare and the gate never opens. It engages only on the crowded worlds a
  *replan* meets, which is where the run died.
- **The reuse gap itself is untouched.** 28 furnaces for 276 ore is the
  disease; this treats the symptom that halts runs. Closing it means letting a
  later smelt adopt a furnace an earlier smelt committed, which needs an
  ordering edge plus a serialisation lag — a scheduler change that would
  certainly move red.

## Green science ran live and halted on a 34-furnace bank

`run-1788497495-79997` (`scripts/factory_stage3.lua`), 29.6 min game time:

```
HALTED: stuck -- refused: no room for a iron-ore cell within 12 tiles of the
patch: a drill needs to stand on the ore with a furnace two tiles ahead of it
standing off it
```

### What the record shows

| | |
|---|---|
| plans | 6 |
| **`stone-furnace` placed** | **44, all at distinct tiles** |
| `burner-mining-drill` placed | **0** |
| furnaces per iteration | **iteration 2 placed 34**, iteration 6 placed 10 |

**A single plan wanted a bank of 34 furnaces.** This is not replan
accumulation — every furnace is at its own tile, nothing was rebuilt on top of
itself, and the step counts were *falling* (637 → 475 → 472 → 454), so progress
was being kept. The bank then occupies the ground around the ore patch, and the
drill+furnace ore cell that must stand **on** the ore has nowhere left to go.

**Same shape as the lab conflict fixed in `38869772`**: something sited inline,
and a subgoal expanded afterwards finds the ground taken.

### The refusal was named, not silent

This project has four separate mechanisms on record that reported nothing while
broken. This one stopped, said which cell, which patch, and the exact geometric
requirement it could not meet. That is the "silence is not success" discipline
paying off — the failure took one run to localise instead of several.

### The open question, and a prior measurement that bears on it

Is 34 a sensible answer to green's plate demand, or a runaway? Green's cell
chest-feeds **12 hand-crafted inserters per cell per charge** across two cells,
each dragging a circuit, gear and plate behind it, so demand is genuinely
large. But `bdbde0c9` measured *building extra furnaces to cover smelting lag*
and found it **lost** — 126 actions / 49,743 ticks against 110 / 46,164 — with
only reuse of standing furnaces paying. If that verdict still holds, a bank of
34 is a **costing** defect wearing a spatial failure's clothes.

Under investigation.

## The chain-owner error was a bystander (`374d7aa3`)

Green science now plans as **research**: 313 actions, makespan **122,356
(33:59)**, utilisation 21.7%. Red is byte-identical.

### The error named the wrong thing

`bot 1 owns chain ChainId(50) … but burner-mining-drill at [-44, -11] does not
hold there` is `Condition::EntityAt` — **world state, satisfiable by any bot.**
It failed for *every* bot. Bot 1 was named only because `schedule.rs` picks the
cheapest rejected candidate and then **`schedule.rs:543` upgrades
`PreconditionUnsatisfied` to `ChainOwnerInfeasible` whenever that candidate's
chain has an owner.** The sizing-versus-binding invariant was intact and never
violated.

**So a plain world-state failure inside an owned chain always arrives dressed
as an ownership problem.** Worth remembering the next time this variant appears
— I took the framing at face value in the brief I wrote, and said "something
sizes a bill against bot 1 and then requires an entity bot 1 does not hold".
Nothing was mis-sized. The entity was required of everyone and existed for
no one, because the action that creates it had been unlinked.

### The real defect: a cycle broke the wrong edge

`ActionNetwork::infer_edges` matches items **by name, ignoring counts**, so it
linked `take 150 iron-plate from the cell → craft 1 burner-mining-drill`,
closing the loop `place → fuel → take → craft → place`. One edge had to go, and
the loop popped whichever it happened to be adding when `validate()` first
failed — **a fact about iteration order, not about the plan.** The placement
edge lost, so `fuel the drill` ended up with no predecessors and the scheduler
stalled at 297 of 313 actions.

The fix: **two passes over the same pair loop — world-scoped pairings first,
then inventory-scoped.** A world-scoped edge (`EntityAt`, `Feeds`, `Researched`,
`BufferHas`) states something only its producer can make true. An item edge is
an over-approximation by construction, and the stock its consumer was sized
against is still in the inventory the scheduler per-bot-checks. **So only an
item edge can ever be what a cycle costs.** Nine lines of logic.

**Why red never hit it:** `researched:automation` places its one drill from a
bot's *starting inventory* — every bot begins with one — so there is no
`craft drill` node and no loop. Green needs two more cells after bot 1's
starting drill is spent, and chain 50 is owned by bot 1, so the spares bots 2-4
still hold are out of reach.

Red was verified the strong way rather than by the pin: a 290-line `--steps`
listing captured before and after, diffed **empty**.

### Corrections to my brief

- **`PlaceDrill` was not the participant.** The cell here is built by the
  `Goal::Have` one-off method that emits `take N <item> from the cell`, not by
  `BuildCell`/`cell_steps`. Its cost model was not involved — **the defect is
  ordering, not costing.**
- **The `craft_ticks`/wood trap does not bear on this.** `no resource patch
  found for 'wood'` appears identically on the *passing* `researched:automation`
  run. Background noise on this map, not a signal — I flagged it as possibly
  relevant and it was not.

## Green science: the cell now resolves, blocked by two further walls (`240d3efb`)

**Red is byte-identical** — 205 actions, makespan 30,077, utilisation 38.8%.

### The design: one intermediate machine with two mouths, not two machines

The cell is still **two machines and never three**; what widened is how many
ingredients the *intermediate* may take (`MAX_FEED = 2`).

**Two is forced by geometry, not chosen.** The intermediate presents three
tiles to its west, and the 5x5 supply area of the cell's single small pole
reaches only two of them. A third feed chest needs a second pole, and a pole is
one wood out of the four a run has. So **"allow a second intermediate machine"
was not the smaller change I proposed in the brief — the version with `inserter`
as an intermediate is not buildable at all**, needing three feed chests under a
pole that lights two.

Consequently green resolves with no tie-break: `transport-belt` fits an
intermediate, `inserter` (three ingredients) does not. **Whole inserters are
chest-fed and hand-crafted** — one level coarser than the chest-fed
`electronic-circuit` I floated, and strictly more hand-work: **12 inserters per
cell per charge** (24 for a `:6` plan), each dragging a circuit, gear and plate
behind it. This is a first green cell, not a green factory.

### Two things the widening made load-bearing

- **`bill` now merges by item.** Green's supply chest holds `inserter`s, which
  is also what three of the cell's own links are made of — two `Goal::Have`
  subgoals naming one item are satisfied by the *same* inventory, so the cell
  would have been built out of the inserters its chest was meant to hold.
  Nothing in red's bill repeats, which is why red does not move.
- **`cells_standing` now asks whether a feeder's source has anything to give.**
  A belt machine fed iron plates and no gears stands, is powered, delivers
  nothing, and used to count as a whole cell. A one-feed cell cannot express
  that case, which is why nobody hit it.

### Wall 1: the green cell takes the tiles red's lab needs

```
logistic-science-pack needs a lab, and no ground within 12 tiles of [7.5, -40.5]
is both free and inside a supply area (117 powered tiles are built on,
351 free ones have no supply)
```

Red's lab goes at `[5.5, -40.5]`; the green cell's **second feed chest and
inserter sit on exactly those tiles.** The cell is sited inline — it must be,
the site comes from the pole — while the research that unlocks green is a
subgoal expanded afterwards. A red cell is one row narrower and leaves the
hole. Named rather than papered over: `PlannerError::ResearchNeedsRoom`, with
the two counts separated because powered-but-built-on and free-but-unpowered
want opposite fixes.

### Wall 2: a pre-existing scheduler defect, and it blocks green regardless

```
$ factorio-bot plan --world workspace/scripts/map.json \
      --goal researched:logistic-science-pack --bots 1,2,3,4
Error: the plan did not schedule: bot 1 owns chain ChainId(50) because its bill
       was sized against it, but burner-mining-drill at [-44, -11] does not hold there
```

`researched:automation` plans fine at 30,077 on the same world, so this is
specific to the path green takes. It is the **sizing-versus-binding invariant**
speaking — R3 established that a chain owner is a hard single-candidate
constraint with no fallback tier, deliberately. Something on this path sizes a
bill against bot 1 and then requires an entity bot 1 does not hold.

**`assemble.rs` is not on this path** — no method in the workspace emits a
`Goal::Producing` subgoal, and `researched:logistic-science-pack` alone fails
identically. Under investigation.

### Still true and still untouched

`BUFFER_ENTITIES` omits `iron-chest`, so the planner cannot see what a cell's
chests hold — **including the 12 hand-made inserters it just put in one.**

## STAGE 3: green science does not plan at all — a capability gap, not a defect

```
$ factorio-bot plan --world workspace/scripts/map.json \
      --goal producing:logistic-science-pack:6 --bots 1,2,3,4
Error: the goal did not expand: no method can satisfy goal:
       produce 6 logistic-science-pack/min
```

**Established in 4 seconds rather than a 20-minute run.**

### The constraint, precisely

`assembly_spec` (`crates/planner/src/method/assemble.rs`) requires a recipe
with **exactly two ingredients, exactly one of which is craftable**. That
craftable one becomes the cell's single intermediate machine; the other is
chest-supplied. The module doc says so outright, and notes that a second
craftable ingredient "would need a second intermediate machine".

| recipe | ingredients | verdict |
|---|---|---|
| `automation-science-pack` (red) | `copper-plate`, `iron-gear-wheel` | one craftable — **the shape the model was built for** |
| `logistic-science-pack` (green) | `transport-belt`, `inserter` | **both craftable** |
| `transport-belt` | `iron-plate`, `iron-gear-wheel` | fine as an intermediate |
| `inserter` | `iron-plate`, `iron-gear-wheel`, `electronic-circuit` | **three ingredients** |
| `electronic-circuit` | `copper-cable`, `iron-plate` | another level down |

So green breaks the model twice: it needs **two** intermediates rather than
one, and an intermediate with **three** ingredients, one of which is itself
crafted. Red's cell is two machines; green is genuinely deeper.

This is a **capability gap, not a defect** — nothing is broken, the planner
simply cannot express this shape. That distinction matters for how it is
approached: red science was months of chasing defects, green starts by
designing a thing that does not exist.

### The trap that must not be repeated

The cell that works and the cell the model *believes* works are different
things — that was tonight's decisive bug, and every `holds_assembling` test
missed it because they stood their cells through the **overlay**, which a
replan never has. Any new cell shape needs a seam test that pushes it through
`update_chunk_entities`, the door the mod's events actually use, or it will
pass every test and fail every run.

## STAGE 2 IS DONE — THE FIRST WITNESS IN THIS PROJECT'S HISTORY

`run-1788489532-62404`, roster `[1,2,3,4]`, `scripts/factory_stage2.lua`.
Verbatim:

```
WITNESSED: automation-science-pack in 2 watched machine(s) went 0 -> 1
(+1, wanted 1) in 780 of 3600 ticks, 756 polls
```

| rung | satisfied | game time from run start |
|---|---|---|
| 1 — `researched("automation")` | 1 iteration, 263 steps | **8.73 min** |
| 2 — `producing("automation-science-pack", 6)` | **1 iteration**, 290 steps | **14.78 min** |
| 3 — **the witness** | 0 iterations (dispatches nothing) | **15.00 min** |

**Rung 2 went from `stuck_silent` after 4 iterations to satisfied on the
first**, on the strength of `30b28846` alone — one field the world model never
wrote.

### Why this is evidence and not another "it stands"

The witness **dispatches no actions**. It reads the fed machines' output
inventories, waits, reads again, and asserts the count rose while every bot
stood still. A pack that appears under those conditions can only have been
assembled by the machine. Every earlier claim about this cell was structural —
"nine actions succeeded", "both recipes set", "the cell stands" — and the
project's own notes were careful to say **standing is not producing**. This is
the first time the distinction has been settled in the right direction.

It took 780 of its 3,600-tick budget, so the machine was working comfortably
inside the window rather than scraping it.

### The whole night, end to end

| | |
|---|---|
| automation, first measured | 21.34 min |
| automation, now | **8.69 / 8.72 min** (two runs) |
| red science cell, before | never satisfied; `stuck_silent` |
| red science cell, now | **satisfied in 1 iteration, witnessed producing** |

## THE CELLS WERE WORKING ALL ALONG (`30b28846`)

**"No cell has ever produced anything" was false.** The run record shows three
complete cells, all on `network: 1`, **two of them `full_output` with
`products_finished: 4` and `7`**. Red science was being assembled. The world
model read zero and the supervisor kept building more cells.

### The defect: a predicate checked against a field nothing writes

`Goal::Producing` is answered by `assemble::cells_standing`, whose second
clause is `machine.recipe.as_deref() == Some(spec.recipe.name)`. **That field
is `None` on every assembling machine in the live world, always:**

- A machine is **built empty** and given its recipe by a *separate* RCON call,
  so `on_some_entity_created` — the only event that puts an entity into
  `EntityGraph` — carries `recipe: None`.
- `EntityGraph::add` **refuses a tile something already stands on**, so a later
  re-observation cannot correct it.
- `FactorioWorld::on_some_entity_updated` is an explicit no-op, and the mod
  raises it only from `on_player_rotated_entity` anyway.

The recipe therefore lived only in that iteration's `PlanState` overlay and
died with it. **Rung 1 satisfied on its first replan because research *is*
recorded in the world model — that contrast is the whole defect.**

### What the record shows, at 400-tick resolution

| tick | event |
|---|---|
| 56,700 | cell 1's science machine **has its recipe**, per the game |
| 57,102 | supervisor replans → **builds a second complete cell elsewhere** |
| 57,600 | cell 1 finishes its first `automation-science-pack` |
| 81,900 | cell 2's machine has its recipe |
| 82,047 | replan → **a third cell** |
| 105,300 | cell 3's machine has its recipe |
| 105,577 | `stuck_silent`, 4 iterations, zero failures |

The plans were not diverging — they were **repeating**, each building a whole
new cell at a fresh site.

### The fix, and the test that was missing

`EntityGraph::set_recipe` records the recipe after a successful RCON reply.
Not an assumption: `rcon_set_recipe` reads `get_recipe()` back and refuses in
the reply body unless it names the recipe asked for, so a refusal never reaches
the write-back.

**Every existing `holds_assembling` test stood its cell through the overlay**
(`create_entity` + `PlanState::set_recipe`) — which a replan never has. The new
test pushes the same cell through `update_chunk_entities`, the door the mod's
events actually use, and asserts `false` without recipes (the state every
replan saw) and `true` with them.

### A second gap, found and deliberately not fixed

`BUFFER_ENTITIES` is `["stone-furnace", "wooden-chest"]`, and its own doc says
*"Not `iron-chest` … Add it the day something places one."* **Stage 2 places
two per cell.** So the planner can never see what a cell's supply chests hold —
exactly the "ran dry vs. broken" case the mod cites. It was left alone because
it changes planning behaviour that cannot be verified offline.

### Corrections to my own framing

- **"The planner produces a plan whose successful execution does not satisfy
  the goal" was half right.** The plan was fine and the *world* satisfied the
  goal after iteration 1. What failed was the model's *reading* of the world.
- **The 4-second offline loop could not reproduce this**, and in the opposite
  direction to the obvious guess: a t=0 world has no cell to mis-read. The
  diagnosis came from `samples.jsonl` plus a code trace.



`run-1788485718-45723` (`scripts/factory_stage2.lua`, roster `[1,2,3,4]`):

- **Rung 1** `researched("automation")` — **satisfied**, 1 iteration, 263 steps.
- **Rung 2** `producing("automation-science-pack", 6)` — **`stuck_silent`
  after 4 iterations**, best 290 steps.

### The symptom is now as sharp as it has ever been

Each of the four iterations reported:

```
ran: success=194 pending=0 actions(failed=0 lost=0) walks(failed=0 lost=0)
ran: success=216 pending=0 actions(failed=0 lost=0) walks(failed=0 lost=0)
ran: success=243 pending=0 actions(failed=0 lost=0) walks(failed=0 lost=0)
ran: success=231 pending=0 actions(failed=0 lost=0) walks(failed=0 lost=0)
```

**Every action succeeded. Nothing failed, nothing was lost. The goal was never
satisfied.** So this is not an execution defect: **the planner produces a plan
whose fully successful execution does not satisfy the goal it was made for.**

That is a much better-posed question than "no cell has ever produced anything",
which is where this stood at the start of the night. Tonight's fixes removed
the execution failures that used to mask it — the lag clock, per-bot
serialisation and the placement collision — and rung 1 now completes in 8.7
minutes with zero failures.

Note the step counts do **not** converge: 194, 216, 243, 231. Combined with the
finding that `supervisor.lua` never calls `obs:recover()` and re-plans from
scratch every iteration, each pass may be rebuilding rather than continuing —
and `BUFFER_ENTITIES` decides what a replan can even see.

Under investigation offline, where it reproduces in **4 seconds** rather than a
20-minute run:

```
factorio-bot plan --world workspace/scripts/map.json \
    --goal producing:automation-science-pack:6 --bots 1,2,3,4 --steps
```

## TARGET MET AND CONFIRMED: 8.69 and 8.72 min

**Two runs, both under target, 0.3% apart.**

| run | ticks | game time | plan |
|---|---|---|---|
| `run-1788483599-83227` | 31,295 | **8.69 min** | 194 steps, makespan 30,268 |
| `run-1788484671-91355` | 31,406 | **8.72 min** | 194 steps, makespan 30,268 |

Identical plans, so planning is deterministic on a live world as well as
offline. Both: one plan, one iteration, no replan, `failed=0 lost=0`.



`run-1788483599-83227`, roster `[1,2,3,4]`, `automation_speedrun.lua`.

```
run_started              @6,944
plan                     @7,173   194 steps, makespan 30,268
milestone 1 satisfied    @38,239
RUN START -> SATISFIED:  31,295 ticks = 8.69 min GAME TIME   (target: under 9.00)
```

**One plan. One iteration. `failed=0 lost=0` on both actions and walks, zero
refusals.** Execution came in **2.6% over plan** (31,066 against 30,268).

### Tonight's progression

| run | time | what changed |
|---|---|---|
| reference (`…465258`) | **21.34 min** | before any of tonight's work |
| ladder (`…479942`) | **13.76 min** | + lag clock (F), concurrency (C), chest (B), savepoints (G) |
| single goal (`…481380`) | **10.52 min** | + one goal instead of seven rungs |
| **verified (`…483599`)** | **8.69 min** | + placement retry (`c2b2698a`) |

**A 59% reduction**, and each step is attributable to a named change rather
than to variance.

### What each fix was actually worth

- **F, C, B, G together**: 21.34 -> 13.76 (−35%). Execution stopped diverging
  from the plan; the chest moved a lag's start earlier so it overlapped.
- **Single goal instead of a ladder**: 13.76 -> 10.52 (−24%). Seven
  independently-planned rungs cannot overlap.
- **Retrying a placement whose footprint a bot stands in**: 10.52 -> 8.69
  (−17%). One collision had been discarding 11,966 ticks of progress.

### Caveats, stated rather than buried

- **Two runs, not one** — 8.69 and 8.72, a 0.3% spread. The margin to 9.00 is
  ~3.3%, so this is met but not comfortably; a slower map or a worse roll would
  eat it.
- **The map is the known-good one and is unidentified** — it predates the
  `--seed` fix, so no seed reproduces it. It survives as
  `workspace/known-good-map/level.zip`. A number from this map is not
  comparable to one from another.
- **The owner's ~9-minute manual solo baseline was set on a random map**, and
  the justfile already notes that optimising ours would make our numbers less
  comparable to it. This run did not optimise the map — it used what was there.
- **`researched("automation")` only.** Red and green science are a different
  and larger problem.

## The collision, diagnosed and fixed (`c2b2698a`)

**The blocker was another bot, parked and idle — not the placing bot.**

At tick 15,727 bot 1 was refused `place stone-furnace at [-39, -12]` while
standing at `(-35.27, -17.5)`, nowhere near it. **Bot 4** was at
`(-38.25, -11.2)`, overlapping the furnace's collision box by about a third of
a tile — and had been **motionless there since tick 11,760, for 4,000 ticks**.
It had walked toward `(-38, -16)` to load a furnace bot 1 had not built yet,
stopped ~4.8 tiles short (inside build reach), and waited.

**The mod already handled the hard half and nothing used it.**
`rcon_place_entity` classifies this apart from the `can_place_entity` family
(so nothing durable is learned) and `step_aside_from_footprint` dispatches a
legitimate walk for the blocker. Its own comment says "by the time anything
asks again, the blocker is somewhere else". **Nothing asked again.** Bot 4 was
clear **53 ticks later** — by which time bot 1's chain was abandoned (39 of 194
steps never dispatched) and the milestone re-planned from scratch.

The fix re-issues such a placement up to 4 times over 1.8 s. No planner change,
so the offline makespan is unmoved at 30,077.

### Two reporting defects found alongside it

- The run recorded this as `kind: "rejected", detail: null` — indistinguishable
  from a full chest. Now `FailureKind::Blocked`.
- **`tools/run_analysis.py` matched `another character is standing`, which is
  the *mining* refusal's wording.** It derived `rejected` and agreed with the
  record **for the wrong reason** — the fourth instance in this project of a
  classifier silently missing wording it did not recognise.

### The larger lever, deliberately untouched: recovery is never invoked

**`scripts/supervisor.lua` never calls `obs:recover()`.** The recovery tiers in
`crates/executor/src/recover.rs` exist and are correct, but no live run reaches
them — after every run the supervisor sets `state = "planning"` and calls
`goal.plan` fresh. Proven from the record: the two `plan_created` events share
**zero id+action pairs** among their 102 common ids, and the whole power plant
relocated from `[9.5, -45.5]` to `[-5.5, -57.5]`.

So the replan was not a decision made *about* this failure — **it is what the
supervisor always does**, and it discards every completed step. That is a
bigger lever on the 9-minute target than the collision was, and it changes
behaviour well beyond this defect, so it is its own workstream.

### Corrections to the brief I wrote

- **The planner already avoids characters.** `PlanState::is_area_free`
  (`state.rs:2210`) refuses any site overlapping a character's box, roster bots
  included. It could not have helped: at plan time bot 4 was ~40 tiles and
  ~12,000 ticks away from where it would eventually stand. Planner-side
  avoidance is not the fix.
- **`min_radius` is not involved.** The blocked tile was not the placement's
  approach position; it was where a *different* action's walk left a bot. **The
  real gap is that nothing checks a walk's landing spot against footprints the
  plan will need later** — a genuine defect, and a separate one.

## SINGLE-GOAL RUN: 10.52 min, and one collision is the whole remaining gap

`run-1788481380-80843`, roster `[1,2,3,4]`, `automation_speedrun.lua` (one
goal). **10.52 min game time (37,854 ticks).**

Progression tonight: **21.34 -> 13.76 -> 10.52 min.**

| | tick | makespan |
|---|---|---|
| run start | 3,511 | |
| first plan | 3,797 | **30,268** -> would finish 34,065 = **8.49 min** |
| forced replan | 15,763 | 25,291 -> finished 41,365 = 10.52 min |

**The ladder hypothesis is confirmed.** The single-goal plan came out at 30,268
against the offline prediction of 30,077 — a 0.6% match — versus ~45,984 for
the seven-rung ladder.

### One placement collision cost the target

```
cannot place item 'stone-furnace' because a character is standing in the
footprint
```

A bot stood where another needed to build. The failure forced a replan at tick
15,763 that **discarded 11,966 ticks of completed progress**.

But for it, the run was on course for **8.49 min planned**; at the 4.6-7.2%
execution overhead measured on the previous run, that lands around
**8.9-9.1 min** — at or just under target. **This single defect is the
difference between meeting the goal and missing it**, which is why it is now
the top open item rather than a footnote.

A full replan is a disproportionate response to a transient collision, and
CLAUDE.md already flags a related concern about `run.rs:258` abandoning a
bot's whole chain on a transient refusal.

### What is now established about where the time goes

- **Execution tracks the plan** (~7%, and 0.6% on the plan/offline comparison).
  Not the bottleneck.
- **Planning structure was worth 53%** — fixed by planning one goal.
- **Transient failures that force replans are expensive** — a mid-run replan
  discards everything done so far and re-plans from the current world. One cost
  ~2 minutes.

## FIRST MEASURED RUN: 13.76 min, and the executor is no longer the problem

`run-1788479942-45523`, roster `[1,2,3,4]` confirmed, all seven rungs
satisfied including `researched("automation")`. Carries F, C, B and G.

**13.76 min game time (49,541 ticks) from run start to the last milestone**,
against 21.34 min for the reference run. Over the 9-minute target.

### Execution now tracks the plan to within 7.2%

| milestone | planned | actual | over |
|---|---|---|---|
| 6 (science packs x10) | 16,894 | 17,700 | 4.8% |
| 7 (research automation) | 17,307 | 18,104 | 4.6% |
| all seven | ~45,984 | 49,541 | **7.2%** |

**This is F and C validated live.** Before them, the executor's dispatch
condition could diverge from the plan by the entire span a bot spent doing
other work; a 3.6-minute single gap was one instance. It now tracks within
7%, so **execution slack is no longer where the minutes are**.

### The remaining gap is planning structure, not execution

`research_run.lua` climbs **seven rungs** (`have(iron-ore,20)` … `have
(automation-science-pack,10)`, then `researched(automation)`). Each rung is
planned independently and **cannot overlap the next**. The ladder plans
~45,984 ticks; the same end state as a **single** goal plans at **30,077**.
The ladder pays roughly **53%** for its legibility.

That legibility is deliberate and worth keeping — the script's own comment
explains that a failure should name which step broke rather than "research did
not happen". So the ladder is the right shape for diagnosis and the wrong
shape for a stopwatch. `scripts/automation_speedrun.lua` is the timed variant:
identical machinery, one goal.

### Milestone savepoints paid off within the hour

Every rung wrote one, 222-641 ms each. `milestone-7.zip` (3,518,669 bytes) **is
a world with automation researched** — exactly the starting point red and green
science need, so those experiments need not re-derive this run. `session_reset`
was confirmed live (`{"walking":0,"mining":0,"crafts":0,"research":0}`).

Also established: **`workspace/server/saves/level.zip` is byte-identical before
and after a run.** Runs do not write back to it, so the map is pristine and
repeatable, and the "accumulated world" worry was unfounded.

### One real error in the run

Milestone 3 needed two iterations: `tried to remove 5 iron-plate but removed
0`. Every other rung was satisfied first time.

## TARGET MET OFFLINE: 30,077 ticks (8:21)

Workstream B landed as `0069b41c`. Verified independently, and **byte-identical
across two invocations** — determinism held.

| | before | after |
|---|---|---|
| makespan | 44,548 (12:22) | **30,077 (8:21)** |
| utilisation | 24.7% | **38.8%** |
| bot 1 planned / idle | 27,999 / 16,549 | 24,709 / **5,368** |
| bots 2/3/4 planned | 6,502 / 4,656 / 4,878 | 8,175 / 6,768 / 7,077 |
| actions | 109 | 205 |

Solo is **byte-identical** (53,692 both sides, 103 actions). Two bots:
45,045 → 39,553.

**This is the plan, not a run.** Nothing has been executed. The measured run is
the next step and it may not track the plan.

### I had the mechanism wrong, and it matters

This plan predicted "if bot 1's share were balanced across four bots … roughly
27,500". **Right in magnitude, wrong in mechanism** — and the wrong mechanism
would have sent the next workstream in the wrong direction.

Bot 1's planned ticks fell only **12%** (27,999 → 24,709) while the makespan
fell **32.5%**. Balancing load is not what paid. The 44,548 was 21,560 ticks of
bot-1 prefix **followed by** the 12,240-tick cell lag, because the lag only
starts at `fuel the burner-mining-drill` — which needed 13 coal that bot 1
mined at the *end* of its prefix. Putting that coal in a chest moved the fuel
load to tick **7,071**, and the lag then **overlapped** the prefix instead of
queueing behind it.

**The lever was moving a lag's start time earlier, not spreading work.** A lag
that overlaps costs nothing; the same lag queued behind a prefix costs its full
length.

### B required D — this plan had the sequencing backwards

The plan said "D is a re-measurement that only becomes meaningful after A and
B". It is the opposite: `worth_converging` refuses **every** bill in this plan,
so the first working chest bought exactly nothing — 44,548, unchanged. The
largest single bill is thirteen coal at 1,560 ticks against a break-even near
2,200, because the model charges an **idle** supplier's detour at the *taker's*
rate. B needed the re-pricing to do anything at all.

Related: `even_shares` deals the taker a share, and here that costs 6,800 ticks
(36,877 vs 30,077) — the taker's share is exactly the work the chest exists to
remove.

### The new binding constraint

Bot 1 is now **82% busy** (24,709 of 30,077), idling only 5,368. The cell lag no
longer binds — it is fuelled at 7,071 and the take waits 1,733. What binds is
bot 1's own serial chain:

| | ticks |
|---|---|
| walking | 7,789 |
| research | 6,000 (hard floor) |
| crafting | 5,760 |
| mining | 4,200 |
| transfers | 960 |

**The movable half is crafting**, and it is currently immovable: `Stockpile::site`
requires `has_resource_patches`, so only *raw* materials route through a chest.
Every crafted intermediate — gears, cable, circuits, pipes, and the 3,000-tick
`craft 10 automation-science-pack` — still converges on the chain owner's
inventory. Extending `Stockpile` to crafted items (supplier crafts its share,
deposits, owner withdraws) is the next cut, and the machinery is now in place.

### Corrections from this workstream

- **Most of the brief's "at minimum" list already existed.** `Condition::BufferHas`,
  `Effect::BufferLose`, `PlanState::buffered`/`take_from_buffer` and `Withdraw`
  all shipped with R3's read-only buffer work. Only the *deposit* side was
  missing. **No executor change was needed** — `Insert`/`Remove` with
  `InventorySlot::Chest` is already the assembly cell's path.
- **The real executor-side gap was elsewhere**: `BUFFER_ENTITIES` in `crates/core`
  whitelists which entities `refresh_buffers` asks the game about, and listed
  only `stone-furnace`. Without `wooden-chest`, a replan is blind to every chest
  the previous plan built — the exact discontinuity that list exists to bridge.
- **`iron-chest` was the wrong guess** (a comment predicted it). Wooden is 8x
  cheaper here: 372 planned ticks against 2,965.
- **`produce::craft_ticks` prices wood at zero** — it checks for a resource patch
  then a recipe, and a tree is neither, so a wooden chest costed 60 ticks and the
  affordability gate stopped guarding. Fixed locally via `chest_ticks`, not
  globally (that would move `PlaceDrill`'s crossover). **Latent trap for anything
  else costed through `craft_ticks` that bottoms out in a minable.**
- **A buffer-scoped condition needs a real edge where a role-scoped one did not.**
  The role-scoped omission is justified by the scheduler re-deriving it from one
  bot's ordered slice; a stockpile's depositors and its withdrawer are different
  bots by construction, so no such slice exists. Demonstrated, not asserted:
  removing the arm makes a whole-plan test find a chest drawn on before it was
  filled.

## The offline loop is real: 4 seconds per experiment

A live dump of the known-good map now exists at `workspace/scripts/map.json`
(864 MB, produced by `factorio-bot lua dump_map.lua --clients 0 --bots 4` in
**16 seconds**). `score-map` loads and plans against it in **~4 s**.

**This is the loop to iterate in.** A planner change is now measurable ~300x
faster than a run.

### The authoritative baseline (live dump, current `master`)

```
makespan     44,548 ticks (12:22)      target: under 32,400 (9:00)
utilisation  24.7% of 178,192 bot-ticks
actions      109
bot   steps  acts  walks  planned   idle
1        90    68     22    27999   16549
2        26    17      9     6502   38046
3        21    12      9     4656   39892
4        21    12      9     4878   39670
```

Resource distances on this map: iron 40.4, copper 58.3, coal 54.0, stone 32.7,
water 46.7 (inside the plant's cheap 64-tile scan), wood 45.2. Verdict
`VIABLE`, fingerprint `dfac0f4caa0a7500`, 17/17 charting probes covered.

**This supersedes the log-reconstructed figures above** (47,127 ticks, iron
24.9): those came from replaying `server-log.txt`, this from the game.

### What the baseline says to do

**Bot 1 holds 27,999 of the roster's 44,035 planned ticks — 64%** — and the
makespan *is* bot 1's chain: 27,999 of work plus 16,549 of waiting. Bots 2-4
idle ~39,000 ticks each.

If bot 1's share were balanced across four bots **and every lag stayed exactly
as long**, the critical path would fall to roughly 27,500 ticks — **7:38, under
target**. Balancing is the lever; shortening lags is not required to hit 9
minutes. That is workstream B, now dispatched with these numbers as its target.

## Measured offline: the PLAN is 13:05, so execution fixes alone cannot reach 9

`a5b31c80` added `factorio-bot score-map`. Run against the reference run's own
map (reconstructed offline by replaying `workspace/server-log.txt`):

```
iron-ore 24.9   copper-ore 29.8   coal 54.5   stone 67.8   water 40.8   (tiles)
walk score 1,454 ticks   charting 17/17   VIABLE
makespan 47,127 ticks (13:05) over bots 1-4, 23.7% utilisation
```

**The current planner's own makespan for `researched:automation` on this map is
13:05.** That is what a *perfect* executor would achieve. The target is under
9:00. So no amount of execution fixing gets there — **the plan itself has to get
shorter**, and its roster utilisation of 23.7% is the reason it does not.

This promotes the plan-side workstreams (B, A, D, E) from "margin" to
"necessary", and workstream 0 makes each of them measurable in **0.26 s**
instead of a 20-minute run. That is the loop to run now.

### Two findings that argue against workstream 0b

1. **The reference map is already a good one.** Everything rung 1 needs is
   inside 68 tiles and the verdict is `Viable`. A seed search is therefore not
   where the remaining minutes are.
2. **The justfile already argued this**, in as many words: searching for a good
   seed "stops being comparable to the ~9 minute manual solo baseline, which was
   not run on an optimised map". The owner's benchmark was set on a random map.
   Optimising ours would make our number better *and* less comparable. Recorded,
   not decided — this is the owner's call.

**`walk_score` ranks maps; it does not predict a run.** 1,454 ticks against bot
1's measured 14,330 — ~10x loose, because rung 1 walks to each resource
repeatedly and between them, which a nearest-tile proxy cannot see.

### The known-good map is UNIDENTIFIED and was at risk

No `map-gen-seed.txt` or `map-exchange-string.txt` exists for it — it predates
the `--seed` fix (`61ec7364`), so nothing recorded what it was. It survives
**only** as `workspace/server/saves/level.zip`, and `just bench` passes `--new`,
which deletes the map. **Copied to `workspace/known-good-map/level.zip`**
(gitignored). Before any seed work runs, capture this map's exchange string
from a live game so it can be regenerated at will.

### Seed `20260903` is still unscored

No map exists for it and creating one is a live run. It remains what the
justfile says it is: a date, chosen for being written down. Do not treat it as
validated.

### Corrections from this workstream

- **"Trees are a bonus, not a requirement" was wrong.** Wood *is* on the power
  plant's bill (1, for a `small-electric-pole`). It is not a *map* requirement
  only because every bot starts holding one wood and a four-bot run has four.
- **`roll-seed`'s stated blocker no longer exists.** Its doc asks for plumbing
  to read a `Schedule` out of the Lua runtime; a world dump loads straight into
  `PlanState`, so no Lua runtime is involved. Only the map-generating loop is
  still missing.
- **Water's bound is the planner's own**: `plan_plant` scans 64 then 128 tiles
  and raises `PowerPlantNeedsWater` beyond that, so water at 200 tiles is
  reported *missing* — "too far to use" and "not found" are the same outcome.
- **Charting does not invalidate the scorer.** The mod charts 418 chunks
  spanning `[-320, 320)` at startup, so a 256-radius search disc fits inside
  what a t=0 dump already knows. A t=0 census remains a *lower bound*, so a
  miss is `Incomplete`, never "absent".

## The path to under 9 minutes — a budget, not a hope

Bot 1's 19.62-minute span is the run. Spending it down:

| step | what it removes | bot 1 span |
|---|---|---|
| baseline | — | **19.62 min** |
| **F** — lag clock starts at the predecessor's finish | 6.89 min of idle (F accounts for ~99% of it) | **12.72 min** |
| **B + R3** — split the mining across four bots | `mine` is 5.25 min, the largest single activity; four ways is ~1.31 | **~8.8 min** |
| **C** — overlap `research` (1.67 min) and `craft` (1.83) with walking | up to ~2 min more, minus what cannot overlap — and the footprint rule below forbids some of it | **~7 min** |
| **0b** — a seed with ore near spawn | some of the 3.98 min of walking | **lower** |

**F plus splitting the mining is what reaches the target.** Everything else is
margin. That is a materially different conclusion from this plan's first
draft, which ranked "fill idle time" first on a mis-measured 12.5-minute idle
figure.

### The floor, and why it is not zero

Two costs are irreducible without more machines:

- **Research is 5,999 ticks (1.67 min) of lab time.** A hard floor for the
  milestone itself.
- **Machine time.** The cell's modelled lag alone is 12,240 ticks (3.4 min) —
  `51 * 240`, the drill's 240 ticks/ore. The milestone cannot complete before
  that elapses, no matter how many bots wait for it.

So roughly **5 minutes of the 6:12 world record is machine time and lab
time**. A human speedrunner is not working faster than us during it; they are
*doing something else* while it runs. That is exactly what F stops us
throwing away, what A fills, and what D/E shorten by adding machines.

**This reorders the plan's own premise.** After F, bot 1 has little idle left
to fill — so **A's value is not bot 1 at all**, it is bots 2, 3 and 4, which
are idle for 48,504 ticks apiece while doing 0.6 minutes of work each. A is
about the roster, not about the busiest bot.

## Workstreams, in value order

### 0. Offline planning on a real map — **DONE** (`db612be9`)

**Landed.** `world.dump(path)` (a Lua binding, bounded by the same
`resolve_write_path` that gates `world.draw`) writes a world; `factorio-bot
plan --world map.json --goal researched:automation --bots 1,2,3,4` plans
against it. **Measured at 0.26 s against a ~20-minute run.** No Factorio,
RCON, workspace or settings file needed. `PlanReport`
(`crates/planner/src/report.rs`) reports actions, makespan, steps/acts/walks
per bot, planned and idle ticks per bot, and roster utilisation — it lives in
the planner because 0b's makespan scoring needs the same numbers.

The Lua binding was chosen over a CLI flag because **the dump that matters is
mid-run**: at t=0 all the ledgers are empty, so a setup-time dump is trivially
faithful and worth little. Only a script knows when a milestone closed.

#### This plan's central premise was FALSE, and it matters

I wrote that "`FactorioWorld` already round-trips through serde", citing the
two `impl` blocks. **Both halves existed and neither had ever been run against
a world containing anything.** `EntityGraph`'s `resources` and `minables` key
their inner maps by `Pos`, a two-field tuple struct, and `serde_json` refuses
the whole document with `key must be a string` the moment one appears in key
position. **Serializing any world that had ever seen a single ore tile or one
tree failed outright.** A test comment in `entity_graph.rs` even recorded JSON
as impossible there.

It went unnoticed for exactly the reason this plan gave for the feature being
easy: nothing wrote a world to disk, so the only worlds ever serialized were
empty ones. **The presence of an `impl` is not evidence that it works** —
I checked that the code existed and reported that as a working round-trip.
Both maps now travel as sorted `[pos, value]` pair lists, with a regression
test and a byte-stability test.

#### Four ledgers were missing, not two

This plan named `inventories` and `placement_refusals`. `PlanState::from_world`
also reads `walk_refusals` (`refused_walks`) and `enclosures`
(`find_walled_in`). All four are now serialized — `inventories` through
`observed_inventories()` in tile order, which determinism requires *and* which
is required at all, since `Pos` cannot be a key. All four load as optional,
because absent is what a pre-change dump means by empty, and that is what a
t=0 world genuinely is.

#### The identical-plan property holds

`crates/planner/tests/world_round_trip.rs` compares every `Action` (id, kind,
pre, eff, duration, label), every action's chain, every chain's owner, and
every `ScheduledStep` with bot/start/end — not merely the makespan. Kept from
being vacuous by a sibling test that strips the four ledger fields back out
(exactly what the old serialization wrote) and asserts the plan **moves**.

#### Caveat: a dump is not byte-stable across processes

The maps the planner depends on are sorted, but `players`, `forces`,
`graphics`, `item_prototypes`, `actions`, `path_requests`,
`entity_prototypes` and `recipes` are still `DashMap`s written in hash order —
stable within a process, not across two. Two dumps of one world can differ as
bytes while loading to the same world and giving an identical plan. **Do not
checksum a dump to decide whether two runs saw the same map**;
`EntityGraph::resource_fingerprint` answers that.

<details><summary>Original workstream text (retained)</summary>


**Today a plan cannot be made without launching Factorio**, so every planner
change costs a 20-minute run to evaluate. This is the single biggest tax on
iteration and it is nearly free to remove.

**`FactorioWorld` already round-trips through serde.** Both halves are
hand-written and present (`crates/core/src/factorio/world.rs:1080` and
`:1099`). What is serialized: `players`, `forces`, `recipes`,
`entity_prototypes`, `item_prototypes`, `actions`, `path_requests`, and
`entity_graph` — which itself carries `entity_graph`, `blocked_tree`,
`entity_tree`, `tile_tree`, `entity_nodes`, `resources`, `resource_tree`
and `minables` (`crates/core/src/graph/entity_graph.rs:1775-1784`).

**Nothing writes one to disk.** That is the whole gap: add a dump after
world setup, and a load that feeds `PlanState::from_world`. Then
`expand()` + `schedule()` run against a real map, offline, in
milliseconds.

**This supersedes the archive-replay design (`ec735a59`).** That design
declared three gaps it could not close from a run archive — resource
*amounts* absent from `EntitySnapshot`, trees/water/cliffs structurally
absent from keyframes, `techs_unlocked` a count rather than a list.
Dumping `FactorioWorld` has none of them, because it captures the actual
world the planner reads rather than a lossy derived snapshot: `resources`
carries amounts, `tile_tree` carries water and terrain, `forces` carries
research state. Keep `ec735a59` as the record of why the archive route was
rejected.

**Two fields are not in the serialized set and both are read by
`PlanState::from_world`:**
- `inventories` — observed container contents (`DashMap<Pos,
  ObservedInventory>`)
- `placement_refusals` — sites the game has refused a build at

At t=0 on a fresh map both are empty, so a setup-time dump is already
faithful. Dumping mid-run — which is what makes replanning from a
milestone useful — needs both added to the impl. Do that as part of this
workstream rather than discovering it later.

**Payoff:** planner iteration drops from ~20 minutes to seconds, and every
workstream below can be evaluated before a run is spent on it. It also
makes the paused experiments cheap to resume.

</details>

### 0b. A reasonable starting seed — the scorer, **DONE**; the search, **NOT RUN**

**Part 1 landed: `factorio-bot score-map`.** Part 2 — actually generating
maps and choosing a seed — is deliberately not done, because it needs a live
Factorio and live experiments are paused. **No seed has been chosen, scored or
validated.**

#### What landed

`crates/planner/src/score.rs` (`MapScore`, pure and deterministic, beside
`PlanReport` and for the same reason) plus `factorio-bot score-map`
(`app/src-tauri/src/cli/score_map.rs`), which prints both tiers for one dumped
world and involves no game:

- **Distance** — nearest charted tile of `iron-ore`, `copper-ore`, `coal`,
  `stone` and **water**, priced as ticks of walking at the planner's own
  `WALK_TILES_PER_TICK`, so the proxy is in the same units as the makespan it
  is a proxy for. `walk_score` is their sum; lower is better.
- **Makespan** — the real `expand()` + `schedule()` for
  `researched:automation`, reported as `PlanReport`. **A refusal is a verdict,
  not a crash**: a map the planner will not plan is the strongest thing that
  can be said against it, so the error is printed and the command exits 0.
  A seed sweep that aborted on its first bad map would be useless.

`scripts/dump_map.lua` is the t=0 dump (it does nothing else), and
`tools/seed_search.sh` is the loop, written down and not run.

#### Water is the constraint, and it is the planner's own number

`plan_plant` scans 64 tiles and then 128; past 128 it raises
`PowerPlantNeedsWater`, the goal does not expand, and the lab never gets
power. So **water beyond 128 tiles disqualifies a map outright**, and
`score-map` reports it as missing even when a tile was found — "found, but too
far to use" and "not found" are the same outcome for a run. Both constants
were made `pub` rather than copied.

#### Wood is a requirement, but not a *map* requirement

This section said trees were "a bonus". They are on the plant's bill: a
`small-electric-pole` is 1 wood + 2 copper cable. But every bot starts holding
one wood and a four-bot run has exactly four, so a treeless map still
researches automation and a tree is only the fallback. `score-map` reports the
nearest tree and never lets it change a verdict.

#### Charting bounds the whole approach, and the bound was checked

`EntityGraph` holds **charted** chunks, so a t=0 score is of what has been
*seen*. Nothing in `mods/BotBridge` calls `force.chart`; the mod replays
`surface.get_chunks()` once at `whoami("server")` and then reacts to
`on_chunk_generated`. Measured off `workspace/server-log.txt`: **418 chunks,
tiles spanning `[-320, 320)` on both axes**. Every point inside a disc of
radius 256 has `|x| <= 256 < 320`, so the default search disc fits inside what
a t=0 dump already knows — which is what makes scoring a fresh map worth
doing.

That is one measured save and not a guarantee, so `MapScore::charting` probes
17 points across the disc per dump and the report leads with the answer. Two
residual limits it cannot repair: `control.lua:1311` drops any chunk outside
`[-512, 512]` for ever, and the discovery pass has been seen returning empty
chunks whose contents arrived thousands of ticks later
(`notes/2026-09-02-resource-double-count.md`) — so a t=0 census is a **lower
bound**. A missing resource is `Incomplete`, never "absent".

#### The one real map scored so far

Not a seed search: the world in `workspace/server/saves/level.zip`,
reconstructed offline by replaying `workspace/server-log.txt` through
`OutputParser` and dumping the result. **Its seed is unknown** — the log says
`Loading map`, not creating one, and `--seed` was silently ignored before
`61ec7364`.

```
iron-ore  24.9    copper-ore  29.8    coal  54.5    stone  67.8    water  40.8
walk score 1454 ticks (00:00:24)      charting 17/17 probes      VIABLE
makespan  47,127 ticks (13:05) over bots 1-4, 23.7% utilisation
```

Two things to read off it. The map the reference run was on is already a
*good* one by this measure — everything inside 68 tiles, water inside the
cheap 64-tile scan — which weakens the case that a seed search is where the
remaining minutes are. And `walk_score` of 1,454 ticks against bot 1's
**measured** 14,330 ticks of walking shows how loose the proxy is: rung 1
walks to each resource repeatedly and between them, so the sum of one-way
distances is roughly a tenth of the real cost. It ranks maps; it does not
predict a run.

#### Seed `20260903` has NOT been scored

It cannot be, offline: no map exists for it. `just bench` would create one,
and creating one is a live run. It remains what the justfile says it is — a
date, chosen for being written down, not for being good.

#### The tension with `BENCHMARK_SEED`, unresolved

The justfile argues the opposite of this workstream in as many words:
searching for a seed that scores well "stops being comparable to the ~9 minute
manual solo baseline, which was not run on an optimised map". Both positions
are defensible and they cannot both hold. **Whoever picks a seed owns that
decision**, and the number quoted afterwards has to say which seed it came
from either way.

<details><summary>Original workstream text (retained)</summary>

**Walking is 21% of bot 1's run.** Against a 6:12 target, 4 minutes of
walking would consume two thirds of the entire budget. No amount of
planner improvement compensates for a spawn whose ore is far away, so a
comparison against WR times is meaningless until the map is a fair one.

**Scope: deliberately modest.** Not a search for a global optimum — a
reasonable seed where everything rung 1 needs is close to spawn. Required
within a short radius: **iron ore, copper ore, coal, stone**, and **water**
(the run places a boiler and steam engine to power the lab). Trees nearby
are a bonus, for the wood the owner wants gathered during idle time.

**Workstream 0 makes this cheap, and removes the blocker that gated the
existing tool.** `roll-seed` exists as a CLI surface but is deliberately
disabled (`app/src-tauri/src/cli/roll_seed.rs`): its old fitness function
(`-shortest_path()`, minus 10,000 per resource type not found within 3,000
tiles) died with the task-graph planner, and its doc says resurrection
needs "a fitness function for the current planner — `Schedule::makespan` is
the plausible candidate — and the cross-boundary plumbing to read a
`Schedule` back out of the handle-based Lua runtime."

**That plumbing is exactly what a world dump removes.** Generate a map →
dump `FactorioWorld` → `expand()` + `schedule()` → read `makespan`. A
direct function call on a deserialized world; no Lua runtime involved.

Two tiers, and the cheap one is probably enough:
- **Distance scoring** — nearest patch of each required resource from
  spawn, read straight off `EntityGraph::resources` on a dumped world. No
  planner needed at all.
- **Makespan scoring** — free once workstream 0 lands, and strictly better,
  since it prices the actual plan rather than a proxy.

**Record the chosen seed and freeze it.** Note that `--seed` was silently
ignored until `61ec7364`, so every earlier run used an uncontrolled map;
`20260903` is documented as the benchmark seed but has never been run and
has not been scored by any of the above. Scoring it is part of this
workstream — it may well not be a good map.

</details>

### A. Fill idle time — for bots 2-4, not bot 1 (see the budget above)

The largest single lever, and the one with no existing mechanism.

Needs a way to express opportunistic work: things always worth doing that no
goal demands. The owner's list: keep furnaces and miners stocked with coal,
collect wood, clear areas for later building, handcraft ahead.

Design questions to settle before implementation:
- A new `Goal` variant, or a separate "filler" pass that runs after
  `schedule()` and packs idle windows? The latter keeps `expand()` purely
  demand-driven and leaves the determinism contract untouched.
- Filler work must never delay demanded work. It needs a preemption or
  window-fitting rule: only schedule filler into a gap whose end is already
  known.
- Filler output must not confuse the planner's inventory reasoning — an
  extra 20 coal in a bot's pocket changes later `available()` sizing.

**Determinism is a hard constraint**: `crates/planner` is pure, no I/O, no
wall-clock, ordered collections only, floats via `total_cmp`. Filler
selection must be a deterministic function of plan state.

### B. Shared chest as a material buffer — **DONE** (`0069b41c`, see above)

The R3 agent could hand a furnace's stone, craft, placement and coal to
another bot, but **could not hand over the ore**: ore is
*inventory-convergent* — a downstream action reads the producing bot's
inventory, so it must land in one specific bot's hands. Placements move
freely because their precondition names a *position*, a world fact any bot
can satisfy.

**A shared chest converts inventory-convergent work into world-convergent
work.** "50 plates in bot 1's inventory" becomes "50 plates in the chest at
P" — any bot can fill it, any bot can draw from it.

This is the enabler for parallel mining and parallel crafting. It is worth
more than the crafting arithmetic alone suggests, because it removes the
structural reason work cannot move between bots.

Needs: a `Condition` naming chest contents; planner methods that route
through it; executor support for take/insert against a buffer chest;
a placement rule for where the buffer lives.

### C. Overlap crafting AND research with other work — **LANDED**

**Implemented.** The executor's invariant is now "one *exclusive* action in
flight per bot" rather than "one action". `crates/executor/src/occupancy.rs`
holds the classification in one exhaustive `match` — `Craft` and `Research`
are background, everything else is exclusive — and `run_bot_signalled`
queues a background action and moves the bot on to its next step instead of
standing still for it.

**Ordering is untouched.** A background action still runs `await_preds`
before it dispatches, and every consumer still waits on that action's own
completion signal, published when the game *settles* it rather than when it
was queued. The wait edges the loop imposes are a strict subset of the ones
`check_wait_graph` approved before the run started, so no schedule that used
to terminate can now hang.

**The inventory hazard, and the narrow rule taken for it.** Hand-crafting
spends materials, and two mechanisms in `crates/planner` reason about a
bot's inventory by walking its steps *in order*: `PlanState::available`'s
reservations, and `ActionNetwork::infer_edges`, which deliberately omits the
producer→consumer edge for a role-scoped `HasItem` across chains on the
stated grounds that the scheduler's per-bot feasibility check re-derives it.
So schedule order really is load-bearing for some material dependencies.

The rule chosen is **disjoint item footprints**: every item an action names
in its `pre`, its `eff` or its own `ActionKind`, and two of a bot's actions
may overlap only if those sets do not meet. It is sufficient rather than
necessary and it forbids real wins — a craft spending iron plates blocks a
later *take* of iron plates, which is harmless in fact because Factorio
removes a craft's ingredients the moment `begin_crafting` accepts it.
Distinguishing those needs the recipe, which the executor does not have.

**What that permits in practice**: a queued craft or research overlaps every
`Walk` step (footprint empty — this is the bulk of it) and every action
naming other items. What it forbids: two crafts sharing an ingredient, and a
transfer of an item a queued craft names.

**Not claimed**: any wall-clock saving on a real run. What is verified
offline is the concurrency invariant, the ordering guarantee and the
footprint rule, each pinned by a test that fails under the pre-change
behaviour (checked by mutation: exclusive-everything fails 3 run tests,
always-disjoint fails 1, background-everything fails the exclusivity test).

Note this interacts with A — a bot with a filler task and a queued craft is
doing two things, which is correct and was impossible before.

**One measurement caveat this creates.** `just analyse`'s per-bot
`busy_ticks` is a sum of intervals, not a union, so it can now exceed the
window span and report `busy% > 100`. That is a real reading — the bot did
two things at once — and the overlap-aware figure is `idle_gaps`, which
merges the intervals first. Commented in place in `tools/run_analysis.py`.

### D. Stop pricing an idle bot's time as scarce

`bdbde0c9` measured building extra furnaces to cover smelting lag and it
**lost**: 126 actions / 49,743 ticks against 110 / 46,164. That verdict is
why only *reuse* of standing furnaces was kept.

**That measurement is not wrong, but it answers a different question than
the one that matters.** It prices ~630 ticks of craft-and-place against a
bot whose time is assumed scarce. The bot has 45,117 idle ticks. Once A and
B give bots slack, this must be re-measured — the owner's instinct ("while
the bot is waiting, why not keep building more miners and furnaces") is
consistent with the idle data and inconsistent with the cost model.

### E. Burner drills and miner+furnace pairs — R2

`PlaceDrill` exists in `crates/planner/src/method/produce.rs` with a cost
model. Exactly **one** burner mining drill was crafted in the reference run,
at tick 26,704. A drill is *slower per ore than hands* (240 vs 120 ticks) —
the win is freed bot-time, which only pays once there is something else for
the bot to do. **So E depends on A.**

The owner's framing: "usually one starts with miner+furnace pairs" — a drill
feeding a furnace directly, which removes both the mining and the hauling
from a bot's critical path.

### F. The lag clock starts too late — **FIXED** (`c8ef0d91`)

**Landed.** `await_preds` now accumulates `finish(pred) + lag(pred)` per
predecessor and takes the max; `wait_out_lag` waits
`deadline.saturating_sub(now)`, zero when already past. The executor's
dispatch condition is now `deps_ready` from `schedule.rs:285` evaluated
against *observed* finish ticks rather than planned ones — previously the two
could differ by the entire span a bot spent doing other work.

Route: the log was passed into `await_preds` rather than widening
`Status::Success(Ticks)` — `Status` is `Copy`, serde-serialised, and reaches
the browser through the OpenAPI seam, so widening it would change a wire
shape to move a number `replied_tick` already holds. Race-free by
construction: the settle path writes under the log guard and only then
publishes `Success`.

Verified by mutation three times: arrival-based timing fails all three new
tests and none of the four old ones — exactly the gap that let this survive.
A half-fix keeping an absolute deadline but collapsing the predecessor
pairing also fails all three. 145 executor tests green.

**A fixture bug was found and fixed on the way**: `RecordingAct` reported a
fixed `(900_001, 900_002)` tick pair for every dispatch while `game_tick()`
counted from zero — two clocks free to disagree. Invisible to a wait measured
from arrival, and nonsense for one measured from a predecessor's finish, so
no origin test could have meant anything until it was fixed.

**Not claimed:** any wall-clock saving on a real run. What is verified is
plan-versus-execution agreement.

<details><summary>Original diagnosis (retained)</summary>


**This was "the unexplained 3.6-minute gap". It is a defect, and it is worth
~32% of the run.** Full diagnosis:
`docs/superpowers/notes/2026-09-03-the-lag-clock-starts-too-late.md`
(`6cee0143`).

**The mechanism.** `wait_out_lag` (`crates/executor/src/run.rs:601`) starts
the lag countdown at the tick the bot **arrives** at the action:

```rust
let deadline = started.saturating_add(u64::from(lag));   // started == NOW
```

The planner's semantics (`crates/planner/src/schedule.rs:285`) is
`finished[predecessor] + lag`. The two agree **only** if the bot arrives on
the exact tick its predecessor settled. They diverge by exactly the amount of
parallelism achieved — and `schedule.rs:225-227` states that walking across a
lag is *where multi-bot parallelism comes from*.

**So the better the plan overlaps a lag, the more the executor throws away.**
Parallelism is not merely unrewarded here; it is actively punished.

**The arithmetic, exact to 23 ticks.** `#24 fuel the stone-furnace` settled at
26,833, every predecessor done. Modelled lag 12,240 (`produce.rs:2051`,
`51 * 240` — the drill's 240 ticks/ore is the cell bottleneck, not the
furnace's 192). Correct deadline: **39,073**. Bot 1 did 17,371 ticks of other
useful work, walked to the cell, and arrived at **44,204** — already 5,131
ticks *after* the plates were due. `wait_out_lag` then set the deadline to
44,204 + 12,240, dispatching at **56,467**. The take succeeded with
`elapsed_ticks: 0`.

**Independent proof the wait was empty.** `samples.jsonl` cumulative
`iron-plate` production reads **76 at tick 40,200 and 76 at tick 56,400** —
zero plates produced anywhere on the map during the entire wait. Production
stopped near 40,000 when the drill's 8 coal ran out at 39,631, exactly as the
planner sized it (predicted 39,073, delivered by 40,200 — within 3%). Bot 1's
position is frozen at `(-26.2, -24.8)` in every sample from 44,400 to 56,460.

**It is seven gaps, not one.** All ten of bot 1's idle gaps over 400 ticks are
`take`s waiting out lag edges. **24,583 of 27,853 idle ticks — 88%, about 6.8
minutes, roughly 32% of the run — were spent waiting for machine time that had
already been spent.**

**Why nothing caught it.** Invisible to a verb histogram, because `take`
settles in its dispatch tick. Invisible to the test suite, because all four
`wait_out_lag` tests use `cross_bot_fixture`, where arrival time and finish
time are the same number — they pin the wait's *duration* and never its
*origin*.

**The fix.** `ExecutionLog::AttemptRecord.replied_tick`
(`crates/executor/src/log.rs:192`) already holds the missing number. Either
carry it in the completion signal (`Status::Success(Ticks)`) or pass the log
into `await_preds`, then compute
`deadline = max over preds (finish(pred) + lag(pred))` and wait
`deadline.saturating_sub(now)`. Two cautions from the diagnosis:
`await_preds` currently collapses `max_lag` across predecessors independently
of which predecessor each lag belongs to (`run.rs:446-468`) — harmless today,
wrong under finish-based timing; and the change must not undo the
tick-versus-wall-clock fix (`run-1788320177-77989`). The walk-before-wait
ordering (`run.rs:280-322`) is corrected for free by the same change.

**Not claimed:** that fixing this cuts 6.8 minutes off the run. Bot 1 holds 90
of 96 steps so it is probably close to the critical path, but that is a
prediction and must be measured on the benchmark seed.

</details>

**The other three bots are idle from 32,793 to the end (48,504 ticks) but are
not blocked by this** — the plan gave them two steps each and they finished.
That is the separate `Holder::Share` ceiling.

---

## Already landed (no work needed)

- **R3** (`c0c3bc5c`) — a furnace's stone, craft, placement and coal can be
  handed to another bot. Fixture: busiest bot 86% → 52% of steps, makespan
  −16.5%. Solo plan byte-identical. 563 planner tests green.
- **Provenance** (`61ec7364`, `1390840d`) — `provenance.json` at run start:
  seed, map-exchange string, game version, git commit + dirty flag, build
  profile, roster, `resumed_from`, map fingerprint.
- **`--seed` was silently ignored** (`61ec7364`, `ccaf562f`) — it only
  reaches Factorio via `--map-gen-seed` on `--create`, which only runs when
  `level.zip` is absent, which only `--new` causes. **Every run before this
  fix used an uncontrolled map.** Now warns loudly and writes
  `map-gen-seed.txt` at every creation.
- **`--compare`** (`a1c1316a`) — refuses comparisons across differing seed,
  commit, Factorio version, build profile or `resumed_from`; flags absent
  provenance, dirty tree, degraded roster, runs ending on `plan_created`.
- **Offline replan harness** (`ec735a59`) — design only, and now
  **superseded by workstream 0**. Its three gaps are artifacts of replaying
  a run *archive*; serializing `FactorioWorld` avoids all three. Keep the
  document as the record of why the archive route was rejected.

---

## Open items not on the critical path

- **Oil's trigger is free.** Measured 2026-09-05 (`1f296b47`): the game fires
  `mine-entity` triggers for a drill or pumpjack on the named resource, in
  headless mode too, so `oil-processing` unlocks the moment a pumpjack
  extracts crude. The oil rungs still open are the planner's: rate, fluid
  cell, steel, exploration/radar, power — not the trigger.

- **`FactorioEntity::new_stone_furnace` uses a 1.8 collision box**
  (`crates/core/src/types.rs` ~1655) while the repo's prototype fixture
  records 1.3984375 for the furnace's box; nothing was changed. Reported by
  the peer session 2026-09-05 from the belt-routing review; unowned. A test
  world built from the constructor is therefore 0.4 tiles wider than the
  game's, which is the class of fixture lie that let a routing primitive pass
  four reviews while unable to connect any real machine.
- **A fixture written by the same task as the code can lie in the code's
  favour.** The belt-routing branch's first version passed every review and
  the whole suite with a furnace placed where a 2×2 entity cannot stand; only
  a whole-branch review that checked the fixture against real prototype
  geometry caught it. Rule to carry: build test worlds from prototype data
  (the dump, `map.json`) or assert the fixture against it, never from
  hand-typed positions alone.

- **Tick-based waits exist; their behaviour on a starved server above 1x is untested.**
  Corrected 2026-09-05 after the peer session read the code: `Actuator::game_tick`
  reads `game.tick` over RCON and the lag wait in `run.rs` polls it against an
  absolute deadline in ticks (the wall clock only estimates the sleep between
  readings), motivated by a run that waited a modelled 4,032 ticks while ~3,599
  passed. The remaining gaps: (1) an actuator that cannot report a clock falls
  back to wall clock × game speed, the old behaviour, so mocks and attached
  servers without a clock still have it; (2) `Actuator::game_speed` was stubbed
  at 1.0 until the peer's branch plumbed the real value, and that path has never
  been exercised on a starved box at 5x. Shape of the item: a test or headless
  experiment that starves the server at 5x and checks no wait expires early,
  plus the delivered tick rate recorded per run. Nobody owns it yet.

- **Flaky test**: `crates/planner/tests/red_science.rs::every_expansion_replays_in_time_order`
  failed once under a full `--workspace` run, reported by the provenance
  agent. **Not reproduced in 18 subsequent attempts** — 12 isolated runs and
  6 full workspace passes (70 `ok` result lines each, zero failures) at
  `20323fd0`.

  **This is unresolved, not resolved.** No error text was captured from the
  original failure, so there is nothing to diagnose from, and the test body
  contains no wall-clock, no randomness and no shared state — reading it
  suggests nothing that *could* vary. In a crate whose whole contract is
  byte-identical determinism, "it stopped happening" is not an answer. If it
  recurs, **capture the failure output before doing anything else**; that is
  the missing input.

  One hypothesis worth recording rather than testing blindly: the failure
  was seen under full-workspace parallelism, and the only load-sensitive
  failure mode available to a pure test is stack depth in `expand()`'s
  recursion, since test threads get a smaller default stack than main. That
  would abort the process rather than fail one test, which does not match —
  so it is a weak hypothesis, offered only so the next reader does not start
  from zero.
- **Benchmark seed `20260903`** is documented but **has never been run**.
  The recipe is destructive (`--new` deletes the map) and unvalidated.
- Mod-side walk re-path defect (`walk_repath_finished` quoting the original
  path's last waypoint).
- Provenance is not surfaced in `RunSummary` or the viewer — deliberate, it
  would move the OpenAPI snapshot for no current consumer.
- `ls` is aliased to `eza` in this shell, which makes `ls -t` fail with an
  `--time` error and silently breaks globs; `cat` is aliased to `bat`.
  Belongs in CLAUDE.md's alias traps.

---

## Sequencing (superseded — kept for the record)

**Every workstream named below has landed.** The order it argued for was also
wrong in two places, both corrected above: B required D rather than following
it, and F — added later — was the largest single item rather than a footnote.
What follows is the plan as written before any of it ran.

**F first — it is a bug fix, not a capability, and it is the largest single
item.** It is confined to `crates/executor` and blocks nothing else, so it
can land immediately and in parallel with the rest.

**0 before everything else, then 0b.** Offline planning is what makes the rest
cheap to evaluate; without it each workstream below costs a 20-minute run
to judge. Seed selection (0b) comes straight after, because every timing
below is measured against a map, and comparisons across different maps are
worthless — that error was already made once this session.

**B before A before E.** The chest (B) is what makes work movable; filler
work (A) is what consumes the freed capacity; drills (E) only pay once a
bot has something else to do. C is independent and can land any time. D is
a re-measurement that only becomes meaningful after A and B. F should be
diagnosed early — it is 17% of the run and might be a defect, in which case
it changes the arithmetic above.

**No live runs until F, 0, 0b, B, A and C are in.** The next run should be the
first honest before/after: on the seed chosen and scored in 0b, with
provenance recorded and `--compare` guarding the comparison.

Note that this plan's reference run (`run-1788465258-49050`) was itself made
on an **uncontrolled map**, since `--seed` did nothing before `61ec7364`.
Its *proportions* — 34% acting, 21% walking, 45% waiting — are what the plan
rests on, and those are structural. Its absolute 21.4 minutes is not a
baseline any later run can be compared against, and `--compare` will refuse
to try.
