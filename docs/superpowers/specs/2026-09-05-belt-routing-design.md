# Routed belts: connecting a drill to its furnaces, and the furnaces to a chest

2026-09-05. Owner decisions taken in brainstorming, recorded at the end.

## The problem

**Nothing in this project can draw a line between two entities.** The one
multi-tile run that exists is the power plant's, and it is not routed: `power.rs`
lays a fixed pump / pipe / pipe / boiler / pipe / engine shape from hardcoded
north-frame offsets turned by direction, with `PIPE_COUNT = 3` asserted by a
test. It works, and it can only ever build that one shape on ground that suits
it.

So every item that moves between two machines today moves in a bot's hands: a
`take` from the drill, a walk, an `insert` into the furnace. That is why bot 1
does roughly twice its share of the work, and it is the mechanism behind the
idle roster. A belt moves items with no bot at all.

**This is the first primitive on the rocket path.** Oil, bulk building and
anything factory-shaped all need it, and none of them can be attempted before
it exists.

## What is being built

`connect(from, to, item)` — the caller names a source entity, a destination
entity and what flows — and gets the belt path, its directions, and the
inserters at both ends, or a refusal that names what stopped it.

**First consumer: drill → furnace → chest.** The smelting column at scale is
the biggest throughput lever, and it matches what the world-record replays
show: drills feeding furnaces is the early game's whole economy.

Worth stating plainly, because it bounds the claim: in the record base 35 of 98
furnaces sit within 2.5 tiles of a drill and are fed *directly*, with no belt.
**A belt earns its place when one drill feeds many furnaces, or when output is
gathered to one chest** — not on the first drill-furnace pair.

## Architecture

**The grid search lives in core, the action emission in the planner.** This is
the split `enclosure` already uses (`core::graph::enclosure` fills, and
`planner::enclosure::check` delegates to it) and it exists for a reason this
repo has already paid for: when prevention and detection had two different
grids, the finer one found cracks the game's pathfinder could not use, and
nothing detected the disagreement. One grid, one definition of a blocked tile.

### `core::graph::route` — the search

Reuses `enclosure::rasterize` for the game's one-tile occupancy grid, then A*
from one connection point to the other with a **turn penalty**, so a run comes
out straight rather than staircased. Straightness is not cosmetic: every corner
is a belt whose direction differs from its neighbours', and the direction is
the part a caller gets wrong.

Where the surface is blocked it considers an **underground pair**, bounded by
the prototype's maximum span.

Returns an ordered list of `(tile, direction)` plus any underground spans, or a
refusal naming the tiles that blocked it.

### `planner::method::connect` — the actions

Turns a route into `ActionKind::Place` actions in build order: belt tiles,
underground pairs, then the inserters at each end.

Its **materials bill is stated as preconditions**, so the shortfall machinery
that already exists refuses the plan before the first belt is placed. That is
the dry-run check: a plan that cannot be finished is never started.

### One owner for inserter facing

`direction` names the side an inserter **picks up from**, established
empirically in this repo and recorded in CLAUDE.md. Getting it backwards
produces a layout that places 100% correctly, passes every geometry check, and
does absolutely nothing, because placement and function are separate concerns.

**A trap with that shape belongs in exactly one function**, with a test that
pins the convention and cites the empirical finding. Belt-into-machine and
machine-into-chest both take their facing from it.

## Failure is refusal, never a partial build

Four ways to fail, all of them before anything is placed:

- no surface route and no underground span that helps — names the blocking
  tiles;
- an underground span longer than the prototype allows;
- materials short — via the existing shortfall preconditions;
- a connection point that does not exist on the named entity.

**Staleness is already handled and is not this module's job.** A route is
computed against the plan world and the world moves. The executor's
`pre_place` check steps the builder aside or refuses by name, so a stale route
degrades into a named refusal rather than a silent mess. `only_ghosts = true`
validates nothing here, as everywhere: ghosts do not collide, so a
ghost-placement count is not evidence the geometry is legal.

## Testing

- **Fixture tests on obstacle grids**, asserting the exact tile sequence and
  directions, in the style of `crates/core/tests/enclosure_run73005.rs`.
- **The inserter-facing test**, pinning the convention by name.
- **Offline**, against the t=0 baseline dump: `factorio-bot plan` for the
  actions and makespan. Seconds, no game.
- **Live**, `just headless` at 5x with four character bots. A run now costs
  ~3 minutes of wall clock, which is why this step is affordable at all.

## Not in this version

No splitters. No balancing or weaving. No throughput sizing — the planner has
no throughput model and adding one is its own piece of work. No fluid pipes,
though the router is shaped so that pipes reuse the search unchanged; the
power plant's fixed geometry is the obvious second consumer once belts are
proven.

## Owner decisions (2026-09-05)

1. **Belts before the alternatives.** Chosen over naming the rocket horizon,
   snapshot recovery and bulk placement, as the primitive everything else on
   that path waits for.
2. **First consumer is drill → furnace → chest.**
3. **The primitive owns the belt run *and* the inserters at both ends** —
   chosen over a belt-only router, so the facing trap is absorbed once rather
   than repeated by every caller.
4. **Route around obstacles, and go underground when the surface is blocked** —
   chosen over surface-only-with-refusal and over straight-lines-only. This
   accepts the underground materials model and the maximum-span failure in
   exchange for removing a whole class of refusal.
