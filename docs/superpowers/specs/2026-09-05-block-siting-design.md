# Siting a block: the planner chooses the anchor

2026-09-05, evening. Sub-project 2 of the blueprint work. Owner decisions at the
end.

## The problem, stated honestly

`Goal::Built { blueprint, anchor }` places a decoded blueprint at an anchor the
**caller** names, and nothing chooses that anchor. Sub-project 1 scoped siting
out on the assumption that "given an anchor, build the block" was complete work.
It is not, and the evidence is in the record: a 37-entity `MinerLine` was
attempted at three anchors and never got past planning, because one obstructed
tile anywhere along its 21-tile belt run makes the whole block infeasible. On
real terrain that is the normal case, not an edge case.

The sharpest statement of why it matters is not about the planner:
**a researcher's first pasted blueprint will refuse too.**

## The risk is replanning, not searching

Finding empty ground is the easy half. The design constraint that actually
shapes this is the one sub-project 1's spec was built around:

> It survives replanning, which is the whole reason it is shaped this way.
> An anchored block is verifiable — and building it twice is a no-op rather
> than a second factory.

A fixed anchor gave that property for free. **Siting can take it away.** If the
search re-runs on every replan and the world has changed underneath it — which
it has, because we have been building — a block half-built at site A can be
restarted at site B. The result is two half-factories, no error, and a
production curve that still rises. This project's signature failure, arriving
through a new door for the third time.

So the anchor must be **resolved once and then recovered**, not recomputed.

### Recovery needs no new state

`method::blueprint` already answers "is this blueprint entity already standing?"
(`already_stands`, three-valued `Standing`, comparing name, tile, direction and
underground half). That is enough to run backwards:

- For each entity standing in the world whose name matches some blueprint
  entity, the pair implies a candidate anchor (the standing entity's tile minus
  that blueprint entity's offset).
- Score each candidate anchor by how many of the blueprint's entities are
  satisfied at it.
- If any candidate satisfies at least one entity, the block is **already
  sited**: take the best-scoring anchor, ties broken by ordered `(x, y)`.
- Only when nothing stands is a fresh search run.

This means the site is chosen exactly once — at the moment the first entity of
the block goes down — and every later replan rediscovers it from the ground
itself. No persistence, no new field on the goal, nothing to get stale. It is
the same shape as the existing `replan-reuses-site` behaviour for plant, cell
and lab.

**The honest limit:** a blueprint whose first-placed entity is a `transport-belt`
sitting on a tile where an unrelated `transport-belt` already stands will
recover a wrong anchor. Scoring by *count* of satisfied entities makes that
vanishingly unlikely for a real block and impossible to rule out for a
one-entity blueprint. Recorded rather than hidden; a one-entity block is not a
case this sub-project claims.

## The search

Rings outward from a seed, first acceptable site wins.

- **Seed**: a caller-supplied `near` hint when present, otherwise the roster's
  centroid. Blocks should land near the bots that must walk to them — walking is
  the dominant cost in every measurement this repo has.
- **Candidate test**: reuse the footprint scan `expand()` already performs, so
  the acceptance predicate and the refusal predicate are the same code. Two
  predicates that are supposed to agree, and are written twice, eventually
  disagree.
- **Determinism**: rings in a fixed order, ties by ordered `(x, y)`, floats via
  `total_cmp`, ordered collections only. The planner is pure and deterministic
  and this must not be the thing that breaks that.
- **Bound**: a maximum radius. On exhaustion, a typed refusal naming how far it
  searched and the nearest obstruction, in the shape of `BlockGroundOccupied` —
  which names the tile and what is on it, and which replaced an error that
  blamed an internal scheduling decision for a fact about the ground.

## Ore-aware siting

`MinerLine` is 13 `electric-mining-drill`s. Clear ground is not merely
insufficient for it — it is **wrong**: a drill on bare ground is a machine that
places perfectly and produces nothing, which is the exact failure class this
repo keeps paying for.

So a candidate site is rejected unless **every mining drill in the blueprint has
ore under its mining area**. Among accepted sites, prefer the one covering the
most ore, ties by distance to seed, then by `(x, y)`.

Two facts from the record constrain the implementation:

- **Resource positions are tile centres** — `(-40.5, -48.5)`, never `(-41,-49)`.
  `EntityGraph` keys resources by `Pos(i32,i32)`, which floors, so anything
  reading a position back out of that map must restore the half-tile offset.
  Getting this wrong made mining fail for every ore on every map while every
  test passed, because the fixture built ore at integer positions — the one
  input for which the lossy round-trip is lossless. **A test for ore-aware
  siting must therefore use half-tile ore positions, or it proves nothing.**
- A drill's **mining area** is not its collision box. It must be read from the
  prototype, not assumed to equal the footprint.

## Evidence

**Entities standing, not actions dispatched**, and not a placement count — this
project has twice shipped layouts that placed 100% correctly and did nothing.

- Offline plans against `workspace/scripts/map.json` (seed 31337, fingerprint
  `c161fa3f437221d0`) for both fixtures, sited rather than anchored.
- Determinism: the same world plans the same site twice.
- Replan stability, which is the property most worth a test: plan, place part of
  the block, re-expand against the mutated world, assert the anchor is
  unchanged. This is the regression that would otherwise be found in a run.
- A headless 5x run with four bots reading every entity back off the live
  surface. Offline-only is what hid the belt-routing geometry defect through
  four clean reviews.

## Out of scope

Material supply (sub-project 3, and every live build so far has cheated the
materials in — no honest end-to-end block build has been measured). Spacing
between blocks, bus geometry, and where blocks go *relative to each other*.
Power: `FurnaceLine` carries no generator, so a sited furnace line is still a
block whose 87 belts and 48 inserters have never been shown to move an item.

## Owner decisions (2026-09-05)

1. **Siting only, done thoroughly**, over a combined siting + power-headroom
   night and over material supply.
2. **Clear ground *and* ore-aware**, accepting the larger night, because
   clear-ground-only sites just one of the two fixtures meaningfully.
3. **Land to master when the suite is green**, unattended.
4. **Headless 5x runs only**; 1x client runs stay with the speedrun session.

## Noted for later, not built here

The goal vocabulary is `Have`, `Researched`, `Produced`, `Built` — all one-shot.
Nothing means *keep this true*, which is the same fact the record reports as
"production plateaus at exactly the plan's bill". Managing and expanding
capacity — the owner's stated target — needs a fifth goal kind, not a change to
an existing one. Solar panels are reachable pre-oil; batteries and electric
furnaces are both hard-gated behind petroleum, which appears nowhere in the tree.
