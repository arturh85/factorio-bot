# Exploration: the primitive, the measured map, and the mechanism that did not work

2026-09-06, branch `bots-that-chart`, from `bda147a8`. Every number below was
measured on this box tonight — on a live headless 2.1.17 server on seed 31337
(`workspace/headless-n`, ports 4343/34223) or offline against
`workspace/scripts/map.json`. Nothing is quoted from an earlier document
without being re-measured, because two documented constants in this repo turned
out wrong the same day.

This continues `docs/superpowers/specs/2026-09-04-exploration-design.md`, whose
pieces 0 (the threat index), 1 (the free-vision measurement) and 3 (the
`NotCharted` refusal) had already landed. It builds that spec's Q4 step 3 — a
first-class exploration goal — and it **refutes the mechanism the whole
approach assumed**.

---

## Summary

* **The primitive is built and works offline.** `Goal::Charted { around,
  radius }` → `method::scout::Scout` → `ActionKind::Survey { to }`, a square-ring
  (Chebyshev) lattice walked outwards, skipping charted cells and cells within
  50 tiles of a charted nest, refusing loudly when it is boxed in.
* **The map holds far more than the model sees, and the gap is measured**: at
  t=0 the model holds 2,255 resource tiles and **no crude oil, no uranium and
  no nests at all**; the map within ±672 holds 11,779 resource tiles including
  43 crude-oil tiles and 559 uranium, plus 154 enemy structures.
* **The reveal radius is ±128 tiles (4 chunks)** — measured twice, exactly.
* **A server-side character charts nothing.** `force.is_chunk_charted` is false
  everywhere, including under the character's own feet, after 700 ticks.
* **A bot cannot walk into unexplored ground at all** -- the pathfinder
  refuses, `failed to path find`, because it cannot path into chunks that do
  not exist. So exploration is a **mod-side** capability, not a walking one
  (section 5).
* **With the mod verb that makes ground exist, it works end to end.** One ring
  survey on seed 31337 took the model from **no crude oil and no nests at all**
  to **7 crude-oil tiles, 559 uranium and 32 enemy structures**, and iron from
  940 to 2,452 tiles, for 3,609 planned ticks across four bots (section 6).
  The `NotCharted` refusal that blocked `researched:oil-processing` is gone.

---

## 1. What is charted at t=0, and what the map actually holds

Seed 31337, fresh `--new` server, before anything moves.

```
generated=400 charted=0
```

**400 chunks generated, zero charted.** The map is created with a 20×20-chunk
block — tiles `[-320, 320)` on both axes — and the player force has charted
none of it. Every resource the planner can see at t=0 is knowledge the mod
ingested from `on_chunk_generated`, which fires on *generation* and never
consults the force's charted area. That is the free vision
`EventKind::VisionMeasured` was added to disclose, and here it is 100% of what
the model knows.

The census inside those 400 chunks, live, matches `map.json` exactly:

| | t=0 model (±320) | the map within ±672 |
|---|---:|---:|
| iron-ore | 940 | 3,658 |
| copper-ore | 462 | 2,712 |
| coal | 466 | 1,887 |
| stone | 387 | 1,920 |
| **crude-oil** | **0** | **43** |
| **uranium-ore** | **0** | **559** |
| **enemy structures** | **0** | **154** |

Distances from spawn, measured:

* **nearest crude oil: (131.5, −348.5), 372.5 tiles**, 476,613 units.
* **nearest enemy structure: `biter-spawner` at (−236.5, 62.5), 244.6 tiles.**
* **nearest iron beyond the starting patches: (−325.5, −51.5), 329.5 tiles.**
* nearest charted copper at t=0: 54.9 tiles, against iron's 18.4 — so **copper
  is the first thing a standing goal is gated on**, and it is inside the
  starting block rather than beyond it.

**The nearest nest (244.6) is closer than the nearest oil (372.5).** Oil is
outside the nest-free radius, and any route to it passes the nest belt. That is
the argument for the stand-off below, and eventually for something more than
avoidance.

`score-map` on the t=0 dump, for the record:

```
walk score     1337 ticks (00:00:22)
charted        resources span x -80..224, y -210..122
charting       17/17 probes on charted ground -- the search disc is covered
fingerprint    c161fa3f437221d0
verdict        VIABLE for rung 1
```

### One thing I could not resolve

The first live query found **0 enemy structures within ±320**; after
`request_to_generate_chunks` + `force_generate_chunk_requests`, the same query
found **15 spawners and 11 worms** in the same box. Both are recorded above as
measured. I could not establish whether forcing generation *populated* chunks
that had been generated as terrain only, or whether something else changed. It
matters — it decides whether a t=0 model could ever know about a nest before
walking near it — and it needs its own run. Until then, treat "0 nests at t=0"
as what the model holds and not as a claim about the map.

---

## 2. The reveal radius: ±128 tiles, measured twice

The lattice pitch is the one number the pattern rests on, so it was measured
rather than assumed. A lone character created on empty ground, on an otherwise
quiet server:

| character at | chunks generated | box |
|---|---:|---|
| (1500, 1500) | 81 | x 42..50, y 42..50 |
| (−1500, 1500) | 81 | x −51..−43, y 42..50 |

A **9×9 block of chunks centred on the character**, both times. So the reveal is
**±4 chunks = ±128 tiles**, and a lattice of pitch **8 chunks = 256 tiles**
tiles the plane with one chunk of overlap and no gap: a point at chunk 0 covers
−4..4, its neighbour at chunk 8 covers 4..12.

That is 8× coarser than the one-chunk pitch I had assumed before measuring, and
it changes the economics completely: **ring 1 alone covers out to 384 tiles**,
which is past the nearest oil.

### A server-side character charts nothing

The same probe, on the same character:

```
charted=0 tick=6852
under char: false   origin: false
```

`force.is_chunk_charted` is false **everywhere, including the chunk the
character is standing in**, after 700 ticks of the game running. This does not
by itself break anything, because the world model is fed by generation and not
by charting — but it means:

* the honest word for what a survey buys is **generated ground**, not charted
  ground;
* any future check written against `is_chunk_charted` will read zero on a
  perfectly good run;
* `freeplay.set_chart_distance`, which `CLAUDE.md` points at for exploration,
  is irrelevant here twice over: it is one-shot at scenario init, and charting
  is not the channel the model learns through.

---

## 3. What was built

### `Goal::Charted { around, radius }`

The exploration primitive. Every other goal names something to end up *with*;
this one names ground to end up having *seen*. A disc rather than a resource
name, deliberately: "chart me some crude oil" is not a goal a planner can
honestly claim, because whether a well is out there is exactly what nobody
knows until the ground is generated — a method claiming it would promise an
outcome it cannot deliver and would never terminate on a map that has none.

**It is satisfied by looking, not by finding.** A survey that walks the whole
disc and finds bare grass has succeeded, and that is the useful outcome: it
converts `NotCharted` ("unexplored, so unknown") into `NoApplicableMethod`
("looked, and it is not there"), which are genuinely different answers.

Reachable from the CLI as `charted:<x>:<y>:<radius>` and from Lua as
`goal.charted(x, y, radius)`.

### `method::scout::Scout` — square rings, walked outwards

Cells are a lattice of pitch `REVEAL_PITCH` (256, measured above), visited in
**Chebyshev rings**: ring 0 is the centre, ring k is the 8k cells at Chebyshev
distance k. Squares because squares tile the plane exactly — no overlap, no
gaps — which a disc of any radius cannot.

Three properties fall out of ring order rather than being arranged:
nearest-first (so the first thing found is the nearest thing), a truncated plan
that has still looked at the near ground, and a roster split that balances
itself because one ring's cells are all about equally far out.

A cell is skipped when **standing there would teach the model nothing**, or
when it is within `THREAT_STANDOFF` of a charted enemy structure. Both are
counted; threat skips name the nest.

### Threat avoidance — the first non-test caller of the threat index

`EntityGraph::threats_from` / `nearest_threat` / `threat_census` have existed
since piece 0 with **zero non-test callers**. `Scout` is the first.
`PlanState::nearest_threat` lifts the query onto the state so a method can ask
it the way it asks `charting`.

**Stand-off: 50 tiles**, a biter spawner's `call_for_help_radius` — the radius
at which "walking past" becomes "being attacked". Stated as a starting scale,
not a proven-safe distance: nothing here has met a nest yet. It errs generous
on purpose, because skipping a cell costs one lattice point and being wrong
costs a bot, and **nothing in this project notices a bot dying** (piece 2 of the
design; the roster is computed once and never recomputed).

When *every* remaining cell is behind a nest, `expand` **refuses and says how
many**, rather than emitting an empty plan. An empty plan there is
indistinguishable from "already explored", which is exactly the silent-failure
shape this repo has paid for repeatedly.

**A soundness gap, stated rather than hidden:** "enemy" is inferred from the
entity *type* string (`["unit-spawner", "turret"]`) because **the mod never
serialises an entity's `force`**. `unit-spawner` is enemy-only in vanilla, so it
is safe by luck; `turret` is not — once this project builds defences, its own
gun turrets will read as threats and push the spiral away from its own base.
The fix is to serialise `force` in `mods/BotBridge/types.lua` and filter on it.

### Two bugs found by building it, both fixed and pinned

1. **Skipping on the centre tile skipped the cell that matters.**
   `is_charted(point)` asks whether the model has the one tile under the
   lattice point; what a visit buys is the whole ±128 block around it. On the
   t=0 dump these disagree exactly where it counts — the ring-1 cell at
   (256, −256) has a charted centre tile (it is inside ±320) while the ground a
   bot standing there would generate reaches y = −384, and **the nearest crude
   oil at (131.5, −348.5) sits in that block**. The centre-tile test skipped
   that cell, so the spiral could never have found the oil it exists to find.
   Now `is_covered` probes the centre and the four reveal edges.

2. **`holds` and `Scout` were two predicates and disagreed silently.**
   `holds` used `PlanState::charting`'s seventeen fixed probes, all of which
   land inside ±320 for any radius up to ~320. So `charted:0:0:256` read as
   *already satisfied* while the lattice still had all eight ring-1 cells to
   visit — and because `AlreadySatisfied` is registered ahead of `Scout`, the
   goal planned nothing and said nothing about it. `holds` now asks `Scout`'s
   own question.

Both have named regression tests.

### What was deliberately not built

* **No new mod verb.** Charting is the engine's response to a character
  standing somewhere new, so a survey asks the game for nothing beyond the walk
  its own `AtPosition` precondition already causes — exactly like
  `ActionKind::Evacuate`. There is therefore no `botbridge_*` stub test here:
  there is no new verb to test.
* **No cheat.** No `force.chart`, no `set_chart_distance`, no `cheat_*` on any
  path a run takes. The generation probes in §2 used `/c` on a scratch instance
  and are comparisons, not run behaviour.
* **No combat.** Nothing shoots, arms a bot or builds a turret.
* **No stop-on-find predicate inside the planner.** A plan is expanded before
  anything runs, so nothing at expansion time can know what a survey will
  reveal; baking a predicate in would be a promise about the future. It belongs
  one level up and is *already expressible*: `Goal::Charted` is idempotent and
  `PlanState` reads the live world, so a supervisor that widens the radius ring
  by ring and re-plans stops the moment the goal it actually wanted stops
  raising `NotCharted`. That same loop is what lets a **newly charted nest**
  exclude the cells behind it.

---

## 4. Cost, offline, on the real t=0 dump

`factorio-bot plan --world workspace/scripts/map.json --goal charted:0:0:<r> --bots 1,2,3,4`

| radius | rings | actions | makespan |
|---:|---:|---:|---|
| 255 | 0 | 0 | 0 |
| 256 | 0–1 | 8 | 5,868 (1:37) |
| 384 | 0–1 | 8 | 5,868 (1:37) |
| 512 | 0–2 | 24 | 14,355 (3:59) |

**Ring 1 costs 1:37 of four-bot game time and covers out to 384 tiles**, which
is past the nearest crude oil at 372.5. Exploration on this map is cheap — the
spiral is affordable, and radar is not needed to reach the first oil. That was
the open question the pitch measurement settled.

The three pinned baselines are unchanged, as required:

| goal | actions | makespan |
|---|---:|---:|
| `researched:automation` | 176 | 21,776 |
| `producing:automation-science-pack:6` | 324 | 26,990 |
| `producing:logistic-science-pack:6` | 569 | 52,819 |

---

## 5. The live runs: walking cannot cross the frontier

Two runs, in order. The first showed the symptom; the second found the cause.

### 5a. The ring taught the model nothing

`explore_ring.lua`, headless, seed 31337, `--new`: ask
`researched:oil-processing` before and after running `goal.charted(0, 0, 384)`.

Before, the refusal names the census `coal (466), copper-ore (462), iron-ore
(940), stone (387)` with 17 of 17 probes covered. The ring planned and ran --
makespan 3,609 ticks, 4 bots, 4 steps each, all eight ring-1 cells attempted.
After, the refusal names **exactly the same census**: 466 / 462 / 940 / 387.
Not one new tile.

### 5b. Why: the pathfinder will not path into ungenerated ground

`frontier_walk.lua` walks bot 1 east in steps, on a fresh seed-31337 map whose
generated block is 400 chunks, tiles `[-320, 320)`:

```
x=100  REACHED
x=200  REACHED
x=300  NO PATH -- the game's pathfinder returned no path: Error: failed to path find
x=400  NO PATH        x=440  NO PATH        x=480  NO PATH
x=560  NO PATH        x=600  NO PATH
```

An earlier five-leg tour to (600,0), (600,600), (-600,600), (-600,-600) and
(600,-600) refused **every leg** with the same error.

So a bot cannot be sent past the edge of the generated world: `rcon.move`
refuses before dispatching anything. The ring in 5a "worked" only because
+/-256 is *inside* the block the map is created with -- it walked over ground
the model already had, which is precisely why it learned nothing.

**A caveat on the boundary.** The refusal begins between x=200 and x=300, which
is inside the 320-tile frontier, so the x=300 refusal may be terrain (a lake)
rather than the frontier itself. That does not affect the conclusion: every
probe *beyond* the frontier refuses, in every direction tried.

**And it is why the raw walk verb must not be used for this.** Dispatched
straight at (600, 0), `action_start_walk_waypoints` does not path at all: the
bot walked into the first obstacle and stopped at x = 63.7, reporting nothing.
That would have read as "walking generates nothing" for entirely the wrong
reason.

### 5c. The two controls, on one server

Chunk counts and the eastern frontier, measured in sequence on a single fresh
seed-31337 server:

| step | chunks | eastern frontier |
|---|---:|---:|
| A. baseline (`--new`) | 400 | 320 |
| B. spawn one character bot at origin | 400 | 320 |
| C. **teleport** that bot to (1500, 0) | **481** (+81) | **1632** |
| D. **`request_to_generate_chunks`** at (3000, 0), r=3 | **683** (+202) | **3200** |

* **B**: placing a bot inside already-generated ground generates nothing.
* **C**: placing it in virgin ground generates the 9x9 block of section 2.
  Generation is **placement-driven**, which is why the teleport probes worked
  and why walking -- which cannot reach virgin ground -- never triggers it.
* **D**: `request_to_generate_chunks` does exactly what walking cannot, from
  the mod, with no character anywhere near it.

### What this means

**Exploration is a mod-side capability, not a walking capability.** The
`Goal::Charted` / `Scout` / `Survey` machinery is right and worth keeping -- it
decides *where to look*, nearest-first, avoiding nests, and it produces the
correct plan at the correct cost. What it cannot do is make the ground exist.
The missing piece is a mod verb that asks the engine to generate chunks around
a point, called when a survey runs or just ahead of it, so the bot then has
somewhere to walk to.

That verb is defensible: it asks for the ground the bot is heading to to
*exist*, which the engine does for a real player continuously as they move. It
is not `force.chart` -- it reveals nothing to the force, and the model would
still learn only what `on_chunk_generated` writes out. But the honesty question
deserves its own look before it ships: generating ground the bot has not
reached is a small piece of free vision, and the +/-128 reveal radius is the
natural bound to hold it to.

Radar remains the alternative, and is more attractive than section 2 suggested:
it is the only mechanism that works today with no new mod surface.

---

## 6. The capability, and the end-to-end proof

### `generate_chunks`, the one act a player cannot perform

`mods/BotBridge/control.lua` gains `rcon_generate_chunks(x, y, radius)`,
registered on the `botbridge` remote interface, bound in Rust as
`FactorioRcon::generate_chunks` and reached from the executor's `Survey`
dispatch, which now **generates before it walks**.

**It is clamped to 4 chunks, mod-side.** That is `GENERATE_CHUNKS_MAX_RADIUS`,
and it is the reveal a character standing there would have been given for free
-- the measured 9x9 block of section 2. A caller asking for more silently gets
four; the clamp lives in the mod so no caller can widen it. That bound is the
whole honesty argument, and it is what
`crates/core/tests/botbridge_generate_chunks.rs` tests hardest.

**The argument that it is legitimate.** A *human* player walks into unexplored
ground constantly: they hold a key, and the engine makes the ground as they go.
Our bots cannot, only because we steer them through `request_path`, which will
not path into chunks that do not exist. That is an artefact of how we drive a
character, not a rule of the game. Clamped to one character's own reveal, this
verb restores parity a player already has.

**The argument against, stated rather than buried.** The ground appears
*before* the bot arrives rather than as it does, so a plan can see one reveal
further than a player would at that instant. It is small, it is bounded, and it
is **not nothing**. So it is disclosed:
`EventKind::BatchProgress` now carries `ground_generate_calls`,
`ground_generated_chunks` and `ground_generate_failures`, the way
`research_trigger_emulated` discloses its own emulation. It rides on
`BatchProgress` rather than `provenance.json` because provenance is written at
run start, before any survey has run, and the manifest only exists for runs
that finished -- while a killed run still generated its ground.

**Is it fit for a measured speedrun run?** My answer: **yes for research and
capability work, and I would not quote a speedrun time from a run that used it
without saying so beside the number.** The three counters make that possible
rather than requiring anyone to remember. If the owner wants a run with *no*
asterisk at all, the honest alternative is **radar** -- 20 red science, 10 iron
plate, 5 gears, 5 circuits, 300 kW, all paid in game -- which reveals ground
with nobody standing in it and needs no new mod surface. Radar is the right
answer for a headline speedrun; this verb is the right answer for everything
else, and the counters are what keep the two from being confused.

The failure counter exists because this crate has no logger. A server whose
BotBridge predates the verb answers "no such function", and without the third
counter that run would read as one that simply found no new ground.

### The proof

`explore_ring.lua`, headless, four bots, 5x, seed 31337, `--new`, with
`FACTORIO_BOT_REFRESH_MODS=1` so the release binary ships the edited mod.

**Before** -- `researched:oil-processing` refuses:

```
no crude-oil is charted anywhere this plan can see, so crude-oil has nowhere
to come from; the plan sees coal (466 tiles), copper-ore (462 tiles),
iron-ore (940 tiles), stone (387 tiles); charted ground covers 17 of 17
probes within 256 tiles of [0.5, 0.5]
```

`goal.charted(0, 0, 384)` planned 8 surveys, makespan **3,609 ticks**, four
bots, four steps each. **After**:

```
a power plant needs water, and the plan can see none within 128 tiles
```

**The `NotCharted` refusal is gone.** The goal now fails several rungs further
up the ladder, for a reason that has nothing to do with oil -- which is exactly
what "the bots went and looked" is supposed to change.

The census confirms it directly:

| | t=0 | after one ring |
|---|---:|---:|
| iron-ore | 940 | **2,452** |
| copper-ore | 462 | **1,400** |
| coal | 466 | **853** |
| stone | 387 | **895** |
| **crude-oil** | **0** | **7** |
| **uranium-ore** | **0** | **559** |
| **enemy structures** | **0** | **32** |

Cost: 3,609 planned ticks across four bots, ~14 s wall at 5x. The copper the
standing-goal lane is gated on triples; the threat index -- which had no
non-test caller at all until this work -- now has 32 real structures in it, so
the stand-off has something to avoid on the next ring.

**This is the acceptance test the whole exploration lane exists for, and it
passes.**

---

## Verification

* `nix develop -c cargo test --workspace` — green, 81 test binaries, exit 0.
* `nix develop -c cargo clippy --workspace --all-features --all-targets -- --deny warnings` — clean.
* 11 new tests in `crates/planner/src/method/scout.rs`, each failing before its
  own change: ring disjointness and size, nearest-first ordering, the empty
  plan for covered ground, charted-cell skips, the centre-tile regression, the
  `holds`/`Scout` agreement regression, threat skip naming, boxed-in refusal,
  boxed-in vs finished, no-effect/labelling, registry routing.
* The three offline baselines above, unchanged.

## What I found and did not fix

1. **Radar is still unbuilt**, and it is the mechanism a no-asterisk speedrun
   run wants (section 6). `generate_chunks` is disclosed rather than free, and
   for a headline time that disclosure is a cost radar does not have.
2. **No entity carries a `force`**, so "enemy" is a type-string guess and this
   project's own turrets will eventually read as threats (§3).
3. **A bot death is still not an event.** The roster is computed once and never
   recomputed, and the mod returns a bare `false` for `character == nil`. Every
   stand-off in this work is a guess that avoids the need to detect this;
   nothing should send a bot past the nest belt until it exists.
4. **The ±512 clamp in `on_chunk_generated`** still silently drops everything
   outside a 1,024-tile box, still with no recorded rationale. It does not bind
   the ring-1 spiral (384 < 512), and it would bind a bigger one.
5. **The nest census discrepancy** in §1, unresolved.
6. **`REVEAL_PITCH` could be 288 rather than 256** (9 chunks exactly tiles with
   no overlap). 256 was chosen because overlap is the safe direction to err in
   and the difference is one ring at the radii that matter.
