# The oil is past where we have looked

2026-09-08. Branch `the-oil-is-past-where-we-have-looked`, off `b5376c94`.

`scripts/oil_milestone.lua` ran headless on a fresh seed-31337 map and refused:

```
PLAN REFUSED: no crude-oil is charted anywhere this plan can see ...
charted ground covers 17 of 17 probes within 256 tiles of [0.5, 0.5]
```

The refusal is correct. This note is what it took to clear it, what that cost,
and three things that were measured rather than assumed along the way.

## How far the oil actually is, and how that was measured

Read directly out of `workspace/scripts/map-31337-explored.json` — a dump of
the same seed already explored — rather than guessed:

```
crude-oil   n=7   nearest=372.5   farthest=390.3
```

All seven wells sit in one field at x 131..154, y -365..-340: north-north-east
of spawn, 372.5 to 390.3 tiles out. For comparison, from the same dump:
iron 18.4, coal 32.1, stone 33.3, copper 54.9, uranium 336.0.

**This confirms rather than discovers.** `scripts/explore_ring.lua` already
carried "the nearest well is at 372.5 tiles" in its own header; the measurement
was taken independently and agrees to the tile.

A fresh map is generated out to ±320 tiles at creation. The oil is outside
that by 52 tiles, which is why no amount of crafting, research or building
clears the refusal.

## The charting design, and whether `goal.charted`'s doc already described it

The loop is exactly what `method::scout`'s module doc says belongs one level
up:

> a supervisor that plans ring by ring -- widening the radius and re-planning
> -- stops the moment the goal it actually wanted stops raising `NotCharted`.

So `scripts/chart_then_drill.lua` invents nothing structurally. Radii 384,
640, 896; plan the disc, run it with the whole roster, replan the goal, stop
when it plans.

**But `goal.charted`'s own Lua doc did not describe it, and was wrong in a way
that would have picked the wrong radius.** It said:

> Planning it emits one `survey` step per blind probe: seventeen points are
> checked -- the centre, then the eight compass directions at half the radius
> and at the full radius

That is `PlanState::charting`'s seventeen-probe compass pattern — the thing a
`NotCharted` refusal *reports* — described as if it were what planning emits.
What planning actually emits is `method::scout::survey_plan`: Chebyshev rings
`0..floor(radius / 256)` of a **256-tile lattice**, where 256 is the measured
reveal pitch (a standing character generates ±4 chunks = ±128 tiles, so points
256 apart tile the plane).

The consequence is not cosmetic. Under the doc as written, `goal.charted(0, 0,
200)` would probe out to 200 tiles in eight directions. Under the code, it
plans **the centre cell alone** — and on a fresh map the centre is already
charted, so it plans *nothing* and is indistinguishable from a fully explored
disc. Fixed in place.

## Foreknowledge: none was used

The oil's position is known to whoever wrote this note. **It is not an input to
the run.** The script charts a disc centred on spawn and widens it blindly; it
never names the oil's coordinates, never calls `force.chart`, never reads the
map generator, never cheats anything in. Ring 1 was enough, but the loop would
have widened had it not been.

The one thing the executor does that a human player could not is
`generate_chunks`, issued before each survey walk because a bot cannot path
into ungenerated ground. It is clamped mod-side to exactly the reveal a
standing character gets, and counted into `EventKind::BatchProgress` as
`ground_generated`, so the run record discloses it.

## What the offline planner said before the run

Release binary at `25a01a4b`, four bots, against
`map-31337-explored-with-categories.json`:

| goal | actions | ticks |
|---|---:|---:|
| `gathered:crude-oil` | 2,117 | 325,138 (01:30:18) |
| `produced:petroleum-gas:45:basic-oil-processing` | REFUSED | — |
| both, as one `goal.all` | 2,295 | 330,406 (01:31:46) |

The middle row's refusal:

```
basic-oil-processing runs in oil-refinery (category oil-processing), and the
recipe wants 100 crude-oil -- a fluid, so it arrives by pipe rather than in a
hand. Nothing standing on this map can be shown to supply crude-oil, so there
is nothing to connect the oil-refinery to.
```

**This corrected a premise the script was first written on.** Seeing the
refinery half refuse alone, the first design split the milestone into three
phases — chart, gather, then replan the refinery against a world with a tank in
it. One measurement of the third row killed that: the composed goal plans, and
costs only 178 actions more than the gathering half alone, which is what says
the rig is planned once and shared rather than twice. The sequential split
survives only as a fallback taken if the composed goal refuses *live* with
`planner::no_fluid_source`.

Also worth recording: `map-31337-explored.json` (without `-with-categories`)
**predates `crafting_categories`** and refuses the refinery half for that
reason instead —

```
no prototype in this world declares any crafting category at all, so the model
predates `crafting_categories` -- it did not say, which is not the same as
saying nothing crafts oil-processing
```

— a clean instance of *absent is not a value*, handled correctly.

## The three planner baselines, re-measured not quoted

Same binary, `workspace/scripts/map.json`, `--bots 1,2,3,4`:

| goal | actions | ticks |
|---|---:|---:|
| `researched:automation` | 176 | 21,784 (00:06:03) |
| `producing:automation-science-pack:6` | 316 | 22,457 (00:06:14) |
| `producing:logistic-science-pack:6` | 441 | 47,478 (00:13:11) |

All three match what was handed to me. The oil baseline
(`gathered:crude-oil` on `map-31337-explored.json`) also matches at
2,117 / 325,138, and took **74 s wall** — the 600 s bound is necessary and the
120 s one would have truncated it.

## The run

Headless, four character bots, `--game-speed 10`, seed 31337, `--new`, on the
isolated `headless-d` instance (RCON 4333, game 34213). Debug build at
`ed6db8e8`.

### Charting: one ring, nine seconds of walking, and the refusal is gone

Reproduced identically across three launches:

```
roster: 4 bot(s)
before any charting: REFUSED (planner::not_charted) no crude-oil is charted
  anywhere this plan can see ... charted ground covers 17 of 17 probes within
  256 tiles of [0.5, 0.5]
-- ring radius 384: goal.holds = false --
  survey: 16 step(s), 4 bot(s), makespan=3609
  survey run: done=true success=8 failed=0 lost=0 pending=0
  after ring: PLANNABLE
charting done: 1 ring(s) walked
```

**One ring was enough**, as the lattice arithmetic predicted: the cell at
(256, −256) reveals x ∈ [128, 384], y ∈ [−384, −128], and all seven wells sit
inside it. Nine seconds of wall time, 3,609 planned ticks, eight surveys, zero
failures. `crude-oil` appears in the world model's resource paths immediately
afterwards.

The *expensive* half is not the walking. Replanning the composed goal against
the newly charted world took **182 s with the game paused** (debug build), and
the same again for phase 2's plan.

### Provenance: the first non-null map-exchange string

`provenance.json` for these runs carries a **populated
`map_exchange_string`**. CLAUDE.md records that the plumbing had landed but
that "no run has written a non-null value, so the first one that does is the
confirmation". These runs are that confirmation. `map.digest` is
`c161fa3f437221d0`, matching the documented seed-31337 t=0 fingerprint, and
`mods` lists the six expected mods.

### Three script defects the run found, two of them latent in `oil_milestone.lua`

Each killed a launch after the charting had already succeeded, which is the
most expensive place to fail.

1. **`record.milestone_satisfied` has no reason for "the plan ran".** The enum
   is `already_satisfied` | `plan_empty` and raises on anything else. Every
   driver script in the tree records a completed plan as `plan_empty` —
   `furnace_run`, `block_run`, `starter_run`, `moving_block_live` — so the
   most common way a milestone is reached is indistinguishable in the record
   from the planner having found nothing to do. `oil_milestone.lua` passes
   `"plan_complete"` and would raise the moment it got there.
2. **`record.plan_created` takes a shaped step array, not the `PlanValue`.**
   `oil_milestone.lua` passes the plan; a `PlanValue` is userdata, so it fails
   with `bad argument #2: error converting Lua userdata to table`.
   `scripts/supervisor.lua::plan_for_record` has the canonical shaping.
3. **The reverse, and silent: `record.actions` wants `plan.steps` itself.** It
   reads `label`, and `kind`/`item`/`count`/`entity`/`slot` to reconstruct a
   hand delivery. The reshaped array `oil_milestone.lua` builds carries
   `detail` instead of `label` and none of the delivery fields, so every
   `action_dispatched` would record an empty action name and no delivery —
   with no error, because a missing field reads as "no delivery".

The first two are loud. The third is exactly the shape this repo keeps paying
for: a wrong record that looks like a right one.

### The plan, and the roster check first

```
plan_created.bots = [1, 2, 3, 4]        <- checked before anything else
PLAN: 2789 step(s), 4 bot(s), makespan=336332 ticks (1:33:26)
  bot 1: 994   bot 2: 627   bot 3: 601   bot 4: 567
recorded steps = 2295, tick = 4698
```

A connect stall does not abort a run, it proceeds with whoever showed up, so
no number here means anything until `bots == [1,2,3,4]`. It does, and all four
bots carry work.

**The live plan and the offline plan agree.** `record.plan_created` counts the
id-carrying steps — the actions — and gets **2,295**, which is the offline
figure on the fully explored dump to the action. The makespan is 336,332
against 330,406, +1.8%. So a world charted by *one ring* and a world charted
by a full exploration produce the same plan for this goal: the extra ground a
full exploration buys is ground this plan has no use for.

The 2,789 − 2,295 = 494 stepless-of-id remainder are walks, which carry no
action id.

One placement was re-sited before dispatch, deterministically across all three
launches: `stone-furnace at [-340, -70] (blocked by cliff; tile grass-3)`,
1 of 157 placements, round 1 of 2. The pre-dispatch `can_place_entity` pass
doing exactly its job.

### The execution: it did NOT reach the rig

`run-1788833726-34821`, outcome `incomplete`.

```
run returned (tick_before=4706 tick_after=53631 elapsed_ticks=48925)
done=true success=246 failed=7 lost=1 pending=2041 running=0
first_error: player 2 has no character: died at tick 27065 killed by small-biter
```

**246 of 2,295 actions settled. 2,041 were never dispatched.** `done=true`
here is the executor judging that no further progress was possible, which is
why the script records `milestone_stuck` and not satisfaction: `pending == 0`
is the part that would have made the claim true, and it is 2,041.

The seven failures: two bot deaths' worth of `no character`, four
`no entity to mine`, one timeout. **Two bots were killed** — bot 2 by a
small-biter at tick 27,065, bot 1 by a small-worm-turret at 53,619, each
respawning 600 ticks later. The model knows of **32 enemy structures** out at
the frontier it charted.

No pumpjack, no tank, no refinery. `entities placed: {stone-furnace: 8,
burner-mining-drill: 3}` — the plan never got past its own scaffolding.

### The attribution verdict, stated as what it is

```
mark     bots  busy%  feed%  feed acts  gen kW  cons kW  work e/b  note
5:00        4     78     49        121       0        0      0/65  no generator: hand-fed
10:00       4     51     25         51       0        0      0/10  no generator: hand-fed
15:00       4      8      0          1       0        0       0/0  no generator (nothing made)
end 19:43   4      0      0          0       0        0       0/0  no generator (nothing made)

  iron-plate    5:00 roster-fed   10:00 roster-fed
  copper-ore    5:00 hand-made    10:00 hand-made
  iron-ore      5:00 mixed        10:00 mixed
```

**There is no petroleum row because no petroleum was made.** The milestone's
own question — is the petroleum `factory` or `roster-fed`? — does not arise:
there was never a refinery. What the run did produce is 53 iron plates and 382
iron ore, and the analyser calls it `roster-fed` / `hand-made` / `mixed`, which
is correct and is not the milestone.

All three items plateau before minute 9 with `THE MACHINES STOPPED` and
`mostly no_fuel` after it. No generation at any point in the run.

### What is genuinely good here

* **`plan_created.bots == [1, 2, 3, 4]`**, checked first.
* **Milestone 1, the charting, is clean**: satisfied at 1:10 game time over
  4,159 ticks at **87.3% fleet utilisation** — 84.0 / 84.7 / 86.1 / 94.4% per
  bot. A committed break is exactly the shape that parallelises: eight cells
  of one ring, all roughly equidistant, dealt across four bots with almost no
  idle. That is the best-utilised window this run has by a wide margin (the
  whole run is 34.8%).
* **Free vision ratio 1.0x** — model reach 448.7 tiles against bot travel
  430.1. The bots went where the model learned; nothing was given for free.
* **The seed is confirmed by exchange string and digest**, the first non-null
  `map_exchange_string` this project has recorded.

### The record has a hole this run put there, and it is mine

`just analyse` reports **"BOT DEATHS AND ROSTER CHANGES: none recorded"** over
a run in which two bots were killed. The events are real — the run's own
`paris` lines narrate both deaths and both respawns on stdout — but
`record.deaths()` was never called, because the script never called it. The
analyser's own caveat is exactly right and could not have been better written:

> 4 failure(s) ARE classified `no_character`, so this build knows the wording;
> a missing character with no death recorded is a cutscene, a controller
> switch, or a death the mod did not see.

It was none of those three. It was a recorder nobody called. Fixed in the
script; **this run's archived record still has the hole**, and anyone reading
`run-1788833726-34821` should take the deaths from this note.

### What this says about the next piece of work, stated as a question and not an answer

The charting worked, the plan is right, and the execution died in biter
territory 370 tiles from spawn with no combat model of any kind.
`method::scout`'s own doc already says this, under "What it does not do":

> **Combat.** Nothing here shoots, arms a bot or builds a turret. If avoidance
> alone cannot reach something the plan needs, the honest outcome is the
> refusal and the skip census, not a fight.

Avoidance is implemented for the *survey* — `THREAT_STANDOFF` skips a lattice
cell within 50 tiles of a charted nest — and the survey did in fact complete
with zero failures. What has no avoidance at all is **everything the plan does
afterwards**, out on ground the survey just revealed 32 enemy structures on.
The deaths were not in the exploration; they were in the work.

Whether the answer is a threat-aware planner, a defended forward base, or a
policy that oil is simply not reachable at t=0 on this seed is **not decided
by this run** and should not be inferred from it. What the run establishes is
narrower and solid: *the refusal that blocked the oil milestone was a charting
refusal, one ring of surveys clears it in nine seconds of walking and 4,159
game ticks, and the wall behind it is not planning — it is surviving the
walk.*

