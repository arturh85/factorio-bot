# Closing the Idle Gap — Implementation Plan

**Status:** planning only. **Live experiments are paused** at the owner's
instruction until these pieces are assembled. A rung-1 run costs 20+ minutes
and the current bottleneck is understood well enough that further runs buy
little.

**Target:** `researched("automation")` in **under 9 minutes**, ideally near
the single-player world record's **6:12**. Current: **21.4 minutes** with
four bots.

---

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

## Sequencing

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
