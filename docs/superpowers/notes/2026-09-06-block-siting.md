# Siting a block: what it does, and what is still unproven

2026-09-06. Sub-project 2 of the blueprint work. Spec:
`docs/superpowers/specs/2026-09-05-block-siting-design.md`.

**Status: the live run has NOT happened yet.** Everything below is offline
evidence plus unit tests. The section that would say "entities stood where the
plan said" is empty on purpose, and this note should not be read as proof of a
working feature until it is filled in.

## What changed

`Goal::Built` used to take a bare `anchor: Position` and refuse if that exact
spot was occupied. It now takes a `Site`:

```lua
goal.built(bp, {x = 10, y = -20})        -- Site::At      exact, as before
goal.built(bp, {near = {x = 0, y = -30}}) -- Site::Near    search from a hint
goal.built(bp)                            -- Site::Anywhere search from a stable seed
```

Resolution runs in one fixed order on every expansion:

1. **Recover** the anchor from the block's own entities already standing.
2. Failing that, **search** rings outward from a seed, refusing any site whose
   mining drills would not sit on ore.

## The part that matters: recovery, and why the seed cannot move

`Goal::Built` is re-expanded on **every replan**. If siting recomputed the
anchor each time, and the world had changed because we had been building into
it, a block half-built at site A could restart at site B — two half-factories,
no error, and a production curve that still rises. That is this project's
signature failure and it has shipped twice.

Two mechanisms prevent it, and they are load-bearing together rather than
separately:

- **Recovery** reads the anchor back off standing entities, requiring **two**
  of the block's entities to agree before it will answer. One is not enough: a
  single unrelated entity of the same name silently mis-sited a real blueprint
  in an existing fixture, and a wrongly *asserted* anchor is worse than a
  searched one.
- **A replan-stable seed** covers the window that leaves. With the two-entity
  floor, a block with exactly ONE entity standing recovers nothing and falls
  back to the search — so the search must return the same answer. It does,
  because placements only ever ADD obstacles and the search skips the block's
  own entities, so every earlier ring stays blocked. **That argument only holds
  if the seed is fixed**, which is why `Site::Anywhere` seeds at the nearest
  extractable ore patch (for a block with drills) or the world origin, and
  never at the roster centroid: bots walk, and a moving centroid re-orders the
  rings.

A third hole was found by review: `occupant_of` counted **live characters** as
obstacles, so an unrelated bot standing in a candidate ring blocked it, walked
away, and a later expansion could pick that ring. Siting now asks about durable
ground only (`siting_occupant`), while `expand`'s fixed-anchor pre-check still
names a character — those are different questions. A bot in the way of *this
anchor now* is exactly what a caller needs told; a bot standing where a block
*might* go is noise.

## Ore-awareness, and why it is deliberately conservative

A drill on bare ground places perfectly, passes every geometry check, and
produces nothing. So a site is refused unless **every mining drill has at least
one ore tile under its own 3×3 footprint**.

That is a floor, not a measurement: a real electric mining drill mines a 5×5
area while colliding on 3×3, and no mining radius is available on the
prototypes. So it can refuse a site where the drill would in fact reach ore just
outside its box. **That direction is the safe one** — a false refusal costs a
site, a false acceptance is a drill that produces nothing.

## What is proven, and what is not

**Proven offline:**

- `FurnaceLine` (179 entities) sites and plans: 988 actions, makespan 55,547.
- Determinism: the same world produces a byte-identical anchor across runs.
- Replan stability, recovery-over-`Site::At`, the one-entity floor, and
  bystander-immunity all have tests that have been **seen to fail** — each was
  verified by deliberately breaking the code and watching the assertion mismatch
  before restoring it.

**NOT proven:**

- **Nothing has been built in a live game from a sited anchor.** Section below
  is empty.
- **`MinerLine` does not site at all**, at radius 48, 64, 96 or 128.

## `MinerLine`'s failure is a planner defect, not a limit of siting

This was nearly recorded as structural — its belt-and-pole corridor runs between
the two drill columns, over the same ore the drills need, and the planner
refuses non-drill placement on ore. One read-only query against a live game:

```
ore at (-42.5,-37.5)  belt=true pole=true drill=true
```

**A transport belt and an electric pole can both be built on an ore tile.**
Factorio's resources sit on the `resource` collision layer only — which is why a
character walks straight through them, as the mod's own walk-stall comment says.

The planner computes `resource_blocks = !stands_on_resources(name)` at every
placement check, so ore blocks everything that is not a mining drill.
`stands_on_resources` was added because `is_area_free` "refused a drill
everywhere on every map" — the right diagnosis, but it carved an exception for
drills instead of correcting the general rule.

This is not blueprint-specific. `is_area_free` and `placement_occupant` are what
`method::connect` routes belts through and what `method::assemble` sites cells
with, so **a belt route that would legally cross a patch is refused as
`NoRoute`, near any patch, on any map.** It fails in the safe direction —
refusing legal ground, never building on illegal ground — which is exactly why
nothing caught it: it produces "no route" and "no site", never a broken factory.

Being fixed separately (`ore-does-not-block`), from prototype collision masks
rather than from the single query above: *"a belt and a pole are legal on that
ore tile"* is an observation; *"nothing collides with resources"* is a rule, and
only the first was measured here.

## Cost: the 96 seconds is not siting's

A sited `FurnaceLine` plan takes ~96 s, and it is worth saying plainly that this
was misattributed twice before it was measured.

| | AFTER (siting, `Site::At`) | BEFORE (`293ec331`, bare anchor) |
|---|---|---|
| | 94.6 / 96.5 / 96.9 s | 52.0 / 51.9 / 51.8 s |

Interleaved, three runs a side, load recorded per run, on a floor deliberately
cleared. Spreads 2.5 % and 0.3 %, so the ~45 s gap is real. **The cause is
`plan_best`**, which runs the whole expand-and-schedule pipeline twice — once
per `DrainPolicy` — keeping the shorter schedule; 96/52 = 1.85. That feature
does not exist at `293ec331`. Siting's own contribution, `recover_anchor`, is
**under 10 ms per call**.

`recover_anchor` *was* genuinely broken — it scanned the whole entity tree once
per blueprint entity, 179 whole-world scans for `FurnaceLine`, worst observed
call 7.86 s — and is fixed by scanning once and bucketing by name. That fix is
worth keeping. It was simply never the 96 seconds.

**The method failure is the transferable part.** The first experiment showed a
fixed anchor still cost ~95 s and was read as exonerating the search and
implicating recovery. But a fixed anchor *also* calls recovery, so it separated
the search from everything-else and never separated recovery from the rest.
**Ruling out one of three candidates convicts neither of the others.**

## Siting is bounded by what the model has ingested — and that is now liftable

Ore-aware siting chooses among the resources the model knows, which at t=0 on
seed 31337 is a fraction of what exists: 940 iron tiles against 3,658 within
±672, 462 copper against 2,712, no crude oil against 43, and **no enemy
structures against 154**.

Since `Goal::Charted` landed this is no longer a hard bound:

```lua
goal.charted(0, 0, 384)   -- widen what the model knows
goal.built(FurnaceLine)   -- then site against the wider set
```

Two caveats must travel with that or the sentence misleads. The mechanism is a
**mod-side generate call clamped to 4 chunks, not walking** — a bot cannot cross
the generation frontier on foot at all. And the ground appears before any bot
arrives, so a run that charts must quote `ground_generate_calls` /
`ground_generated_chunks` / `ground_generate_failures`. "We made the ground
exist" is the honest sentence; "the bots explored" is not.

**Siting does not check for enemy structures.** A block can be sited into a
biter base and the planner will not know. One charting ring puts 32 of them into
the threat index, so the data now exists — the check does not.

## The live run

`run-1788663566-25023`, headless, four character bots, game speed 5, seed 31337
on a fresh map. `goal.built(FurnaceLine)` with **no anchor** — siting chose.

**Siting is proven. The build did not finish, for a reason that is not siting.**
Both halves are real and they are separable, so they are reported separately.

### What siting did — proven

- **Anchor chosen: `(0.00, 0.00)`**, with no anchor supplied by the caller.
- **179 of 179 planned placements were consistent with that one anchor**, none
  disagreed. The block was sited coherently, not approximately.
- **The footprint was actually clear**: the script's own pre-scan found **0
  non-resource, non-character entities inside the chosen footprint** before
  `goal.run`. So the site was genuinely empty ground, not merely a spot the
  planner failed to refuse — a distinction this project has been caught by
  before.
- Plan: 183 steps across 4 bots, makespan 1,405 ticks.

### What stood — proven, and the direction migration holds

**129 of 179 entities read back off the live surface at the right tile, with
the right name, facing the right way**, including both underground belts with
the correct half (`input`/`output`).

**Zero wrong directions. Zero wrong halves.** Everything that was built was
built correctly. That is the second independent confirmation of the 8-point to
16-point direction migration, and the first at a **sited** anchor rather than a
hand-chosen one.

### What did not stand — and why it is not siting

**50 entities are missing, and the cause is a single failed placement that
aborted the rest of the batch.**

```
failure[1] id=130 status=failed
  cannot place item 'transport-belt' because a character is standing in the
  footprint; dispatched 4 times over 534 game ticks and refused every time
```

One bot stood where another needed to build, the placement was refused four
times, and the batch stopped — leaving roughly fifty later placements never
dispatched. Two bots also stalled walking, **blocked by the block's own
entities** (`blocked by our own stone-furnace`, `blocked by our own inserter`).

Three things follow, none of them about siting:

1. **This is the footprint-refusal defect the speedrun session independently
   hit tonight** on a 1× client run, where it cost a replan at milestone 1. It
   was believed fixed for server-side characters by the mod's bot registry;
   this run shows it failing **headless too**, so it is failing in both modes,
   not one.
2. **A single unresolvable placement costs the whole remainder of a block.**
   Failing fast is defensible, but the blast radius here is 50 entities from
   one bot standing in one tile, and nothing retried or re-planned around it.
3. **The band split did not prevent bots blocking each other.** The spec's
   claim is that bands are the structural reason two bots cannot trap each
   other; `FurnaceLine` is 29 wide by 11 tall, so bands are vertical slabs ~7
   tiles wide, and bots working adjacent slabs still meet at the boundary.
   Bands stop bots *interleaving*; they do not stop them *colliding*.

### Reproduced three times, in separate processes and workspaces

The run above is one of **three independent live runs** — two of them in a
different workspace, on different ports, by a different process:

| run | stood correct / 179 | wrong direction | wrong half | wrong entity |
|---|---|---|---|---|
| A | 129 | 0 | 0 | 0 |
| B | 89 | 0 | 0 | 0 |
| C | 129 | 0 | 0 | 0 |

Two things this buys that one run could not:

- **The anchor was `(0.0, 0.0)` in all three live runs and in the offline plan
  against the world dump.** Deterministic across processes, workspaces and the
  offline path — which is the property the whole design rests on, and it is now
  evidence rather than an argument from the code.
- **Every mismatch in every run was a plain MISSING entity.** Not once a wrong
  direction, a wrong half, or a wrong entity. Whatever got built was built
  correctly, three times over.

### How much to trust this

The failures are **logical, not starvation**: a character occupying a tile is a
refusal the game computes, not a timeout, and it repeated across 534 game ticks
rather than expiring.

But the floor was **not** quiet, and the numbers say so: **delivered tick rate
was 63.8 % and 71.4 % of nominal**, both below the 80 % flag, with sibling
worktrees compiling at load 13-31 even though no other Factorio was running.

So the two halves of this result carry different weights, and the difference is
the point:

- **Siting and placement-correctness are trustworthy.** Three independent
  reproductions, identical anchor, zero incorrect placements. Starvation cannot
  make an entity stand in the wrong place.
- **The completion count is real but load-modulated.** The defect exists at any
  tick rate — 89/179 and 129/179 differ, and the *cause* is identical in all
  three — but **its severity is not established.** Nobody should quote a
  completion rate from these runs. A quiet floor would give a number; these give
  a diagnosis.

### Still not proven, and worth repeating

`FurnaceLine` carries 13 poles and **no generator at all**. The 87 belts, 48
inserters, 2 splitters and 2 underground belts are proven to **stand** and have
never been shown to **move a single item**. Nothing in this run could show it,
and nothing in it tried.
