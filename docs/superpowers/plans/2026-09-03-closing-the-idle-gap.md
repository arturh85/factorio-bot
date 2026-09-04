# Closing the Idle Gap — Implementation Plan

**Status:** planning only. **Live experiments are paused** at the owner's
instruction until these pieces are assembled. A rung-1 run costs 20+ minutes
and the current bottleneck is understood well enough that further runs buy
little.

**Target:** `researched("automation")` in **under 9 minutes**, ideally near
the single-player world record's **6:12**. Current: **21.4 minutes** with
four bots.

---

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
| `researched("automation")` | 21.34 min | **8:17** measured on seed `31337`; **planned 8:01** since rocks (`130b3bde`) |
| red science cell standing | never satisfied, `stuck_silent` | satisfied in 1 iteration |
| red science producing **once** | never observed | witnessed six times, all inside 780 ticks |
| red science producing **at a rate** | impossible to claim | **5 packs in 3,240 ticks (~5.5/min)**, twice |
| green science, planning | did not expand at all | plans end to end |
| green science, live | never run | **executes, `exhausted` at best 200 steps** |
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

### What blocks green now

**Capacity.** A furnace's output slot holds exactly one stack, and the planner
sizes transfers from demand. `735efeba` bounded `PlaceDrill`'s take only; the
smelt bank, the ore insert and seven fuel sites remain unbounded, and **5 of
green's 11 failures are still divergence.** Already specified in
`docs/superpowers/specs/2026-09-04-world-model-divergence-design.md`.

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
