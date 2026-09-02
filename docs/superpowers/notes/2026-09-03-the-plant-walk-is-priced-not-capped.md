# The power plant's distance bound was a borrowed constant, and it halted run 32

**Status: landed.** `crates/planner` only, plus two forced lines in
`crates/scripting_lua`. Every existing makespan pin passes unchanged. No live
run was made for this — the evidence is run 32's own record and the planner's
own cost model.

## What happened

Run `run-1788379071-00467` satisfied rungs 1-6 and refused rung 7:

```
HALTED: stuck -- refused: the nearest water is 67.8 tiles away, and a power
plant may not be sited more than 64 tiles from the bot that has to carry it there
```

Refused by **3.8 tiles**. The refusal machinery worked exactly as designed: the
run halted cleanly with a reason instead of crashing. The number was wrong.

## Why the number was wrong

### 1. It was borrowed, and its own doc said so

`PLANT_SITE_RADIUS`'s doc comment read, in place:

> The same bound `PlanState::electric_supply_kw` and `Researched`'s lab search
> already use, and for a sharper reason here: everything the plant needs is
> carried to it […]

Those two are about a **pole's supply area** — a physical constant of the game,
the ground a small electric pole actually energises. This one guarded a
**walk**. One number stood for two unrelated quantities, and the walk half of
it was never derived from anything. The "sharper reason" is a reason the bound
should *exist*; it is not a derivation of 64.

### 2. It refused a journey the same run was making routinely

Recomputed from `workspace/runs/run-1788379071-00467/samples.jsonl` (625 bot
samples, ticks 3,360–40,800), integrating successive positions for distance
travelled and taking the maximum `|position|` for displacement from spawn:

| bot | travelled | max displacement from spawn |
| --- | --- | --- |
| 1 | 1117 tiles | **68.8** |
| 2 | 750 tiles | **69.9** |
| 3 | 692 tiles | **72.5** |
| 4 | 510 tiles | **71.5** |

All four bots had already been further from spawn than the water was, in the
run the bound refused.

### 3. It measured from a point that means nothing

`plan_plant(&ctx.state, &from)` takes `from` = `BotState::position` of the
chain actor. That is seeded from `base.players` once and **never advances
during expansion** (`method::have`'s furnace siting says so in place). So the
origin of the measurement is wherever the previous milestone's last action
happened to leave the bot. At tick 40,800 that was:

```
bot 1: (9.75, 29.25)   |from spawn| = 30.8
bot 2: (6.26, 35.69)   |from spawn| = 36.2
bot 3: (12.23, 27.30)  |from spawn| = 29.9
bot 4: (3.32, 29.74)   |from spawn| = 29.9
```

Move bot 1 four tiles and the same map, the same lake and the same plan pass
the bound. A policy whose verdict turns on an accident is not a policy.

## Why no cap replaced it

**The walk is already priced.** Every part of the plant carries a
`Condition::AtPosition` at the plant site, so `schedule` emits a `Walk` and
`travel_ticks` charges `distance / WALK_TILES_PER_TICK` — 0.15 tiles per tick.
67.8 tiles is 452 ticks.

Against run 32's own clock — 40,775 ticks to reach rung 7, six rungs done —
one round trip to a plant costs:

| plant distance | one way | round trip | share of run 32 so far |
| --- | --- | --- | --- |
| 64 tiles | 427 ticks | 853 | 2.1% |
| 128 tiles | 853 ticks | 1,707 | 4.2% |
| 256 tiles | 1,707 ticks | 3,413 | 8.4% |
| 512 tiles | 3,413 ticks | 6,827 | 17% |

None of those is "the bot spends the run walking". A refusal on top of a priced
walk charges the same distance twice — once as ticks in the makespan, once as a
veto — and the veto has no measurement behind it. **A distant plant is a worse
plan, and a worse plan is what a makespan is for.**

So `PlannerError::PowerPlantTooFarFromWater` is **deleted**, not raised.
Raising it to a bigger round number would have been the same borrowed-constant
mistake with a different digit, and the test below is built so that it cannot
be made to pass that way.

## What the two remaining radii are, and why

`plan_plant` keeps its two-tier scan, and the second tier changes job: it used
to measure a distance in order to write a better epitaph, and now it **finds
the water the plant is built against**.

* **`PLANT_WATER_SCAN_RADIUS = 64`** — the cheap scan, run on every expansion.
  Its meaning changed from "the furthest a plant may be" to "the furthest we
  look before paying for a wider read". The number is unchanged because nothing
  about the cheap scan changed; raising it would only move work from the second
  tier into the first. It is now a **cost** number and needs no policy
  derivation.

* **`PLANT_WATER_WIDE_SCAN_RADIUS = 128`** — run only when the cheap scan finds
  nothing. This is now the *only* bound on where a plant may go, so it has to
  carry its own justification rather than inherit the refusal's.

  The coordinator asked whether 128 has the same problem as the 64. **Partly,
  and the answer is worth stating precisely.** It was chosen alongside the 64,
  but for a genuinely different and written-down reason — read cost, not pole
  supply — so it is not a borrowed physical constant. What *is* new is that it
  has been promoted from a diagnostic radius to a policy one by this change,
  and it is documented as such.

  Its derivation is cost: `nearest_water_tile` is linear in the tiles the quad
  tree holds inside a `2R`-by-`2R` box, and a fully charted map carries
  ~410,000 water tiles since `fa8dabf3`. At 128 that box is 65,536 tiles, read
  once, and only on the path that would otherwise have nothing to offer; at 256
  it is 262,144. Cost grows as `R²`; the chance of a lake appearing in the new
  ring does not — and water further out is often water the game has not charted
  at all, since the planner only ever sees the chunks it has been sent.

**The wide scan still runs only on the failing path.** That was a deliberate
earlier fix (the first draft charged every successful plan ~65,000 tile clones
for a sentence) and it is untouched: the cheap tier is tried first and the wide
one only when it returns `None`.

## What survives

* **`PowerPlantNeedsWater`** — nothing within 128 tiles. This is the genuinely
  impossible case, and its message now says what was *looked at* rather than
  what is *allowed*: "the plan can see no water within 128 tiles" is true and
  actionable, where "a plant may not be sited more than 64 tiles away" was a
  claim the code could not support. Its doc also keeps the two readings that
  are not "this map is dry": a world attached from a snapshot fetches no tiles
  at all, and an owned run knows only charted chunks.
* **`PowerPlantNeedsShore`** — water near enough, no piece of its edge with
  room behind it. Untouched.

## Red-first, and the actual failure

```
---- method::power::tests::water_past_the_cheap_scan_is_built_against_rather_than_refused stdout ----
thread '…' panicked at crates/planner/src/method/power.rs:1023:14:
80 tiles of water is a longer walk, not an impossible plant:
PowerPlantTooFarFromWater { distance: 78.50159234053791, limit: 64.0 }
```

The fixture's lake sits at about (40, 40); from (-40, 40) it is ~78.5 tiles
away — run 32's shape at a slightly larger number.

## Mutation evidence

Each mutation applied to the fixed code, then reverted.

| mutation | tests that failed | isolated? |
| --- | --- | --- |
| **M1** the wide tier never runs (refuse on the cheap scan, as before) | `water_past_the_cheap_scan_is_built_against_rather_than_refused`, `the_wide_scan_is_anchored_on_the_bot`, `the_wide_scan_finds_the_same_lake_the_cheap_one_does` | no — all three are far-anchor tests and this removes the mechanism all three rest on. Correct, not a flaw. |
| **M2** `PLANT_WATER_SCAN_RADIUS` raised 64 → 128 (the "bigger round number" fix) | `water_past_the_cheap_scan_is_built_against_rather_than_refused` **only** | yes |
| **M3** wide tier anchored on the origin instead of the bot | `the_wide_scan_is_anchored_on_the_bot` **only** | yes |
| **M4** dry-map refusal changed to `PowerPlantNeedsShore` | `a_world_with_no_water_refuses_by_name` **only** | yes |
| **M5** scan wide first, cheap tier dead | **nothing failed** | see below |

**M2 is the important one.** The test asserts
`distance(pump, from) > PLANT_WATER_SCAN_RADIUS`, so it cannot be satisfied by
enlarging the constant — only by the fallback actually working. That is the
"do not raise it to a bigger round number" instruction encoded rather than
promised.

### The mutation that cannot go red, and what the tests do pin

**M5 — swapping the tier order — is undetectable, by construction.**
`nearest_water_tile` sorts candidates by distance and returns the first, so
`nearest_water_tile(from, 128)` equals `nearest_water_tile(from, 64)` whenever
the latter is `Some`. Scanning wide first is *behaviourally identical* and
differs only in how much terrain is read. No unit test can observe it, because
there is nothing to observe.

What guards it is the constant's doc and the comment in `plan_plant`, not a
test. If tier order ever needs pinning, the thing to pin is a read count, which
would mean instrumenting `PlanState`; that is a bigger change than the property
is worth today, and this paragraph is the record that it was considered rather
than missed.

**`the_wide_scan_finds_the_same_lake_the_cheap_one_does` also cannot go red on
the shared fixture alone**, which is why `the_wide_scan_is_anchored_on_the_bot`
exists next to it and builds a second lake. With one lake, an origin-anchored
scan and a bot-anchored scan pick the same tile and nothing distinguishes them
(this is exactly what M3 demonstrated: the one-lake control passed the mutated
code). What the one-lake control *does* pin is that a plant reached by the
second tier is still built on the fixture's water rather than somewhere the
layout wandered off to — it fires on a siting bug, not on an anchoring one.

Note that the two plants in that control are deliberately **not** identical:
from (0, 0) the nearest tile of the lake is its north-west corner and the plant
goes on the north shore; from (-40, 40) it is the west edge and the plant goes
on the west shore. The plant is built on the side the bot approaches from,
which is the behaviour to want and the reason the control asserts a *lake*
rather than a `Plant`.

## Blast radius

`crates/planner/src/method/power.rs`, `crates/planner/src/error.rs`, and two
forced lines in `crates/scripting_lua/src/globals/goal/mod.rs` —
`refusal_for`'s match over `PlannerError` is exhaustive with no `_` arm, so
deleting a variant is a compile error there. The alternative (keep a public
error variant nothing ever constructs) would leave that function's own doc
comment promising callers a refusal — "the water is too far to carry a plant
to" — that the planner no longer issues.

Full `cargo test -p factorio-bot-planner`: 378 lib tests plus every integration
file green, including every makespan pin in `red_science.rs`, `scheduling.rs`,
`smelt_roots.rs`, `seeded_roster.rs` and `split_capacity.rs`. Nothing moved:
the cheap tier is unchanged, so every plan that used to be planned is planned
identically.

## What this does not claim

It does not claim rung 7 now succeeds. It claims rung 7 is now *planned* rather
than refused before it starts. Whether the plan then executes — a bot walking
68 tiles carrying a pump, a boiler, an engine, five coal, a lab and ten science
packs, and a lab that has to stand in the new pole's supply area — is a
question for the next run, and `PLANT_COAL`'s own doc already names the
residual nobody detects (a boiler that runs dry still reads as 900 kW).
