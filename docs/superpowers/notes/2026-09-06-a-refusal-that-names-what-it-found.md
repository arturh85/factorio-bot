# A refusal that names what it found

2026-09-06. Follow-up to
`2026-09-06-a-failed-placement-blames-a-tree.md`, whose measurement stands and
whose mechanism was deliberately left open. This closes the readable half and
leaves the other half open **on purpose**, with what was learned about why.

## What was wrong, and it was structural

`PlanState::occupant_of` consulted six sources in a fixed order, and the one
that **cannot say what it found** was asked third:

```
added entities -> base entities -> BLOCKING BOXES -> characters -> refusals
```

`blocked_tree` stores a bare `is_minable` flag and no name, so every box in it
came back `Occupant::Terrain`, whose message recited *"occupied by a tree,
cliff, rock or unit"* -- four things, of which a reader takes the first, and
none of which it had read. `Occupant::Refused` existed, read *"a footprint the
game already refused a build at"*, and was **unreachable whenever a box covered
the same ground**.

Three changes, all in `crates/planner/src/state.rs` and
`crates/core/src/graph/entity_graph.rs`.

**1. The order puts what is named ahead of what is anonymous.** Refusals and
characters are now asked before the blocking boxes:

```
added entities -> base entities -> characters -> refusals -> BLOCKING BOXES
```

Clearness is unchanged -- `is_area_clear_of` is still `occupant_of(..).is_none()`
and every source still blocks. Only which one gets to speak has changed.

**2. A refusal carries the game's own evidence.** `PlacementRefusal` has always
held `blockers` (what the mod found in the box it had judged) and `tile` (the
ground under the refused centre), plus the entity name; the planner reduced all
of that to a `Rect` and threw the rest away. It keeps it now
(`RefusedFootprint`), and `Occupant::Refused { entity, blockers, tile }` says
it:

```
a footprint the game already refused a burner-mining-drill at,
where it found iron-ore on tile dirt-4
```

An empty blocker list *with* a tile reads "where it found no entity at all on
tile X, so the ground itself is the answer"; empty with no tile reads "and it
named nothing it found there" -- the signature of a refusal from a mod that
appended nothing, which is not the same as clear ground.

**3. `Occupant::Terrain` says only what it knows.** `is_minable` is the one bit
`blocked_tree` stores, and `blocking_boxes_within` discarded it;
`blocking_boxes_within_minable` keeps it. `Terrain { minable: true }` is
*"occupied by a tree or rock"*; `Terrain { minable: false }` is *"occupied by
something with a collision box that this model cannot name -- a cliff, a unit,
or anything else the entity tree does not hold"*. That second sentence is the
honest unknown the old message was pretending not to be.

## What put a box there: NOT ESTABLISHED

The handover asked for this and said an honest "I could not" is a good outcome.
It is that.

**One latent defect of exactly the right shape was found and fixed.**
`EntityGraph::add` filed **every** entity with a non-zero collision box into
`blocked_tree` except resources and rails -- **including `entity-ghost`**. Both
of `occupant_of`'s entity loops skip ghosts by name, unconditionally, with a doc
saying no caller should ever want a ghost to collide (measured live: a real
placement consumes the ghost beneath it rather than being refused by it). The
blocked-box loop is name-blind, so **a ghost that reaches the graph defeats that
skip and reports as terrain.** `GHOST_ENTITY_TYPES` now keeps ghosts out of the
tree, which also takes them out of `enclosure::grid_for` and the belt router's
obstacle grid, where a ghost is equally not an obstacle.

**That is not asserted to be the cause of the reported refusal**, and the
evidence against it is same-day and specific: nothing printed inside an
RCON-invoked mod function reaches stdout, so `rcon_place_blueprint`'s ghost
writeouts were measured arriving **zero** times, with a non-ghost control that
also never arrived (CLAUDE.md, "Nothing printed inside an RCON-invoked mod
function reaches stdout"). `stamp_ghosts` discards the ghosts `place_blueprint`
returns, so the reply is not a second path either. A ghost reaching the graph is
therefore not demonstrated on any live path today.

Two candidates from the original note remain untested: a failed placement
recorded as blocked ground, and a box whose extent exceeds its source. One
observation narrows the search for whoever picks this up: the game's census at
that tile held **entity-ghost, iron-ore and transport-belt and nothing else**;
ore never enters `blocked_tree`, and a belt is in `entity_tree` too so the
entity loop would have named it first. Whatever the box was, it was **not named
by any source the planner had**.

**Nothing in the Lua bindings enumerates `blocked_tree`**, which is why the
peer could not see it from a script and why the next step is a binding that
does -- that is the mod and it belongs to its owner, not to this branch.

## A refused tile still cannot be rebuilt on replan, and that is now visible

The property `goal.built` exists for -- a second pass finishes a partial
build -- is **still defeated**, and the refusal ledger is what defeats it.
`PlanState::refused` is read from `FactorioSurface::placement_refusals`, which
is **never expired**, so a footprint the game turned down once is excluded for
the rest of the run whatever later happens to the ground. A refusal whose cause
was transient -- a bot standing on the tile, the commonest one -- permanently
strands the block it was part of.

That trade was made deliberately (four runs re-chose a refused tile, one twice
in a single milestone) and it is not this branch's to reverse. What changed is
that the reader can now see which case they are in: the refusal names the
entity, the blockers and the tile, so "a bot was standing here" and "the ground
is unbuildable" are distinguishable without a run. **Expiring a refusal, or
re-asking the game about one before a replan, is the open question this leaves
behind.**

## Verification

- **The three offline baselines are byte-identical**: 176 / 21,784 ·
  316 / 22,463 · 442 / 47,542, on `workspace/scripts/map.json`, measured
  **twice** on release binaries built from the same tree with and without the
  change -- once at the branch's original base `297785e2`, and again after
  rebasing onto `870ead7e`, since master moved while this was in flight.
- **Six tests, each falsified one at a time**, and each substitution asserted to
  have matched before the result was read:
  - ghosts out of `blocked_tree` -- filter removed, `1 != 0`, one test red;
  - the `is_minable` flag -- hard-coded `false` in the accessor, one test red;
  - the refusal/box order -- the refusal overlap disabled, and the failure is
    the **reported symptom reproduced**: `Terrain { minable: true }` where the
    game had said `iron-ore` on `dirt-4`;
  - `Terrain`'s flag -- hard-coded `true`, then `false`, one test red each way.
- **The control caught a bad fixture.** The first draft placed its test site at
  the run's own (-16, -14), which has a tree on it *in the shared fixture* -- a
  different map and a coincidence. `the_site_starts_clear` failed, and without
  it every later assertion would have been about a world with two obstacles in
  it.
- `cargo check --workspace --all-targets` and
  `cargo clippy --workspace --all-features --all-targets -- --deny warnings`
  are clean, and `cargo test --workspace` is green on the rebased branch.

  Before the rebase it had one failure,
  `method::power::block_headroom_tests::a_draw_past_the_layout_is_refused_by_the_layout_and_not_by_the_poles`,
  and **the obvious reading of that was wrong**. A first check ran it on a
  worktree at the main checkout's `HEAD` -- which was **not** this branch's
  base, master having moved -- where it passed, so the failure looked like
  this change's doing. Run at the actual base commit `297785e2` it fails
  identically with no change applied: pre-existing, and fixed by a master
  commit that landed in between. **A baseline is only a baseline against a
  stated commit**, and "HEAD" is not a stated commit when two worktrees
  disagree about it.
