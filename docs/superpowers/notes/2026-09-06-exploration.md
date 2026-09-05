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
* **And the mechanism failed in the live test.** Four bots walked a full ring
  out to ±256 tiles and the model's resource census did not change by a single
  tile. Walking did not teach the model anything. This is the finding that
  matters most and it is **not fixed**.

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

## 5. The live run: the mechanism did not work

`factorio-bot lua explore_ring.lua --headless --bots 4 --game-speed 5 --seed 31337 --new`

The script asks `researched:oil-processing` before and after running
`goal.charted(0, 0, 384)`. Before:

```
plannable before: false
refusal: no crude-oil is charted anywhere this plan can see ...
  the plan sees coal (466 tiles), copper-ore (462 tiles), iron-ore (940 tiles), stone (387 tiles);
  charted ground covers 17 of 17 probes within 256 tiles of [0.5, 0.5],
  so the uncharted ground is beyond that radius
```

The ring planned and ran: **makespan 3,609 ticks, 4 bots, 4 steps each** (two
walks and two surveys apiece — all eight ring-1 cells attempted), finished in
~13 s wall at 5×. Bots ended up where they were sent; one is at (−249.2, 255.2).

After:

```
plannable after: false
refusal: ... the plan sees coal (466 tiles), copper-ore (462 tiles),
  iron-ore (940 tiles), stone (387 tiles);
  charted ground covers 7 of 17 probes within 256 tiles of [-249.22, 255.23],
  ending soonest 128 tiles south-east at [-158.71, 345.74]
```

**The census is byte-identical: 466 / 462 / 940 / 387. Not one new tile.**

So the bots walked a full ring to ±256 tiles and **the world model learned
nothing at all**. The premise the whole approach rests on — that moving a
character causes the engine to generate chunks, which the mod writes out, which
the model ingests — did not hold in this run, even though §2 measured a
character generating an 81-chunk block when it was *created or teleported* into
virgin ground.

The distinction between those two observations is the open question, and I did
not settle it. The leading hypothesis is that the 9×9 block follows an entity
being **placed** into ungenerated space, and that a character **walking** does
not push the generation frontier outward at all — in which case walking is not
an exploration mechanism in this codebase and the honest options are a mod-side
`request_to_generate_chunks` around each bot (cheap, legitimate — it asks for
the ground the bot is standing on to exist), or radar.

Note also that the "after" refusal reports **7 of 17 probes** from the bot's new
position: the model genuinely does not have ground around (−249, 255), which is
consistent with nothing having been generated there.

**Do not read this note as "exploration works".** The planner primitive works,
is tested, and produces the right plan for the right cost. The step from that
plan to new knowledge is unproven and currently looks false.

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

1. **Walking does not appear to grow the model** (§5). The most important open
   item; it needs one run that watches `surface.get_chunks()` while a bot walks
   outward, to separate "walking generates nothing" from "the ring landed
   inside already-generated ground".
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
