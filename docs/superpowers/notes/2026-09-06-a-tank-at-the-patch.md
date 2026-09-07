# A tank at the patch

2026-09-06. The rung above `a pumpjack on a well`: `method::gather`, a new
planner method claiming a new `Goal::Gathered`, which stands a pumpjack on the
nearest charted well, sites a **storage tank on the field's centroid**, and
runs **pipe** between the two. Offline only — no live run.

The specification is the owner's own description of how oil is played
(`2026-09-06-how-oil-is-actually-played.md`), and following it made the work
*smaller* in three specific ways, each recorded here because each was a thing
this lane was about to build and did not need.

## What it plans

`gathered:crude-oil` on `workspace/scripts/map-31337-explored.json`, four bots,
alongside `researched:oil-gathering` (which the goal needs and does not itself
plan — stated directly, an unresearched pumpjack recipe is tier 2 of
`method::extract`'s ladder and refuses by name):

```
actions        2156
makespan       342088 ticks (01:35:01)
```

against `researched:oil-processing` alone at **2012 / 308,577**, which is
unchanged by this work. The gathering itself is 19 actions of that: 1 pumpjack,
1 tank, 17 pipes.

The geometry it produced, which is the part worth checking by hand:

```
pumpjack  (131.5, -348.5)   the nearest charted well to spawn
tank      (141.5, -354.5)   the centroid of the 5-well field, exactly
pipes     (133.5,-349.5) (132.5,-350.5) (133.5,-350.5)   <- the pumpjack's port
          (133.5,-351.5) ... (139.5,-356.5)              <- 13 route tiles
          (139.5,-355.5) (140.5,-356.5)                  <- the tank's port
```

## The three things it does NOT do, and why that is the note

**No power plant at the well.** A wellhead is a pumpjack and a tank. No
refinery, so no water, so no plant. The refusal this lane spent a day on —
*"a power plant needs water, and the plan can see none within 128 tiles"* —
was the planner siting a plant at the consumer. `method::power::ensure_powered`
already stays where the water is and crosses the distance on poles, and this
module calls it through `method::extract` rather than reimplementing anything.

**No flow model.** 2.1's pipe bandwidth carries a patch down one trunk, so the
problem is connectivity and direction. There is no rate arithmetic anywhere in
`gather.rs`.

**No pump.** A pumpjack pushes into whatever is connected to its output. A pump
would only be needed to lift fluid or to enforce a direction, and **a pump's
two pipe connections are its directionality** — one placed backwards builds
100% correctly and moves nothing, the silent class this repo already paid for
with inserters. The cheapest way not to get a convention wrong is not to need
it. So the convention is still unestablished, and nothing in the tree depends
on it.

## The finding: two captures in this repo disagree about `positions`

This is the one thing here that would have produced a silent failure, and it
was found by reading both captures rather than by reasoning.

`fluidbox_prototypes[].pipe_connections[].positions` is four positions, one per
direction the entity can face, north first. **What each position means is not
the same in the two captures this repo holds:**

| capture | pumpjack output, north | storage tank, north |
|---|---|---|
| `crates/core/tests/entity-prototype-fixtures.json` | `(1,-2)` | `(-1,-2) (2,-1) (1,2) (-2,1)` |
| `crates/core/tests/live-2.1.17-world-snapshot.json`, and every world dump | `(1,-1)` | `(-1,-1) (1,1) (1,1) (-1,-1)` |

A pumpjack is 3x3, so `(1,-1)` is a tile **inside** its footprint and `(1,-2)`
is the neighbour **outside** it. The old fixture names the pipe tile outright.
The live capture names the interior tile the connection sits on — and Factorio's
own prototype pairs that with a `direction` saying which neighbour the pipe goes
on, **which the mod does not send**.

Read one as the other and every pipe lands on the wrong tile, builds perfectly,
and moves nothing. Worse: the *fixture* uses the old convention and every real
dump uses the new one, so a module written and tested against the fixture alone
would have been wrong in exactly the case that matters and green in the case
that does not. That is
`2026-09-06-fixtures-agree-with-their-code.md` with the polarity reversed.

**What the module does about it:** decides per connection, by whether the offset
leaves the footprint. Outside → that is the pipe tile. Inside → every neighbour
outside the footprint is a *candidate*, and on a corner there are two, so it
places **both plus the diagonal tile that touches both**. Whichever candidate is
the real connection is joined to the run; the other is an inert stub costing one
iron plate. Guessing was not an option and version-sniffing would have been a
guess with extra steps.

The two captures are also the test oracle, which is the part I would keep:
`the_two_captures_agree_on_where_a_pumpjacks_pipe_goes` asserts that **the tile
the 1.x capture names outright is one of the candidates the 2.x reading
produces**. Neither number comes from this module's arithmetic, so the test
cannot agree with whatever the code happened to do.

**Unsettled, and deliberately left so:** the two captures describe different
*tanks* — four corners with one connection each, versus two corners with two
each. Both are handled on their own terms and the answer would only ever save
an iron plate.

## The field is not the graph's patch

The spec says to anchor the tank on the patch centroid. The obvious
implementation — average `PlanState::resource_patches` — **is wrong for oil, and
silently so**. `EntityGraph`'s patches come from a flood fill over *adjacent*
tiles and oil wells are never adjacent, so on the explored dump the seven
charted wells are seven patches of one tile each and a "patch centroid" is just
the well. The whole idea collapses into "put the tank next to the pumpjack" and
nothing announces it.

So the module defines its own neighbourhood: the **field** is every charted tile
of the resource within `FIELD_RADIUS` (20 tiles) of the wellhead.

**`FIELD_RADIUS` is chosen against the routing window, not against geology.** A
pipe run is searched on one `enclosure::window`, which reaches 24 tiles; the
centroid of a set within `R` of the wellhead is itself within `R`, so `R` must
leave room for the tank's footprint and for a route that is not a straight line.
A nearer tank is a worse plan; an unroutable one is no plan at all.

## What the tests caught

Two real defects, both found by a test rather than by review:

**A plan holds pipes this module did not lay.** `ensure_powered` sites a power
plant when there is none, and a plant is an offshore pump, a boiler, a steam
engine *and the pipe between them* — three more `Place { pipe }` actions in the
same `Vec<Step>`. The first connectivity test read all eight pipes as one run
and reported it in seven pieces. The test now filters on the placement label,
and asserts that the filter removed something, so it cannot quietly go back to
measuring everything.

**The pole run could take a pipe's tile.** The pipe run is computed *before*
anything is emitted (so a refusal leaves nothing behind), which means no pipe is
in the overlay when `ensure_powered` sites its poles. A pole was free to stand
where a pipe was about to go; the plan would have read as good and the pipe's own
`AreaFree` would have failed at execution. `extractor_steps` now takes the run as
`reserved` occupants.

## Falsification found two tests that tested nothing

Thirteen tests, each broken on purpose one at a time, asserting that the break
is caught by **exactly** the test that claims it. Eleven were caught first time.
Two were not, and both were the same shape — *the test was answered by a
different mechanism than the one it named*:

**`the_pipe_run_joins_the_pumpjack_to_the_tank` did not need the source port at
all.** Dropping the pumpjack's own port tiles from the run broke nothing,
because the fixture world speaks the 1.x convention where that port is one tile
— and that tile is also the route's start, which `route_belt` emits anyway. So
every test in the module was exercising the *unambiguous* half of the port code
and the two-candidate half that every real dump takes was reached by nothing.
Fixed by `a_run_on_the_live_capture_places_both_candidates_and_their_junction`,
which expands end to end against a world carrying the live fluidbox numbers.

**`an_unroutable_tank_refuses_and_leaves_nothing_behind` never needed the
obstacle grid.** Unblocking every cell of the search grid broke no test: the
walled-in pumpjack is refused by the *per-tile* `is_area_free` check that runs
after a route is found, which is a second, independent guard. The grid — the
thing that makes a route go *around* an obstacle — was untested. Fixed by
`a_route_goes_around_an_obstacle_rather_than_through_it`, which fails as a
refusal (the straight route hits the wall and the per-tile check rejects it)
rather than as a bad layout.

Both mutations are caught now, each by exactly one test.

**One thing is deliberately left unpinned and is stated rather than claimed:**
handing the pipe run to `ensure_powered` as reserved ground. Removing that line
breaks no test, because the fixture's pole run never wants a tile the pipe run
wants. It is in the code because the collision is possible, not because a test
demands it; a test that forced the geometry would be a test of the fixture.

## What is left

* **One pumpjack, not all of them.** The tank is sited for the field — that is
  what the centroid is for — but only the nearest well is worked. Standing a
  pumpjack on every well is a loop over this same code with a pole run each.
* **No trunk.** The second tank at the base and the long run between them is the
  other half of the topology, and it has a real design question this one does
  not: hundreds of tiles cannot be routed on a single 48x48 window, so it needs
  a coarser search or a chain of windows.
* **Thirteen unit tests and no live build.** See the falsification section
  above for what they do and do not cover.
* **Nothing has been run.** Every claim here is a plan and a unit test. The
  tank standing and the pipes standing would be shown by a live build; **crude
  actually moving** needs the tank's `fluidbox` read back non-empty, and that is
  the only evidence that would settle the candidate-pair question above.

## Baselines

`target/release/factorio-bot plan --world workspace/scripts/map.json --bots
1,2,3,4`, one release binary before and after:

| goal | before | after |
|---|---|---|
| `researched:automation` | 176 / 21,784 | 176 / 21,784 |
| `producing:automation-science-pack:6` | 316 / 22,463 | 316 / 22,463 |
| `producing:logistic-science-pack:6` | 442 / 47,542 | 442 / 47,542 |
| `researched:oil-processing` (explored map) | 2012 / 308,577 | 2012 / 308,577 |

Identical, as they must be: `Goal::Gathered` is a new goal that nothing else
expands to, and `method::extract` was refactored — `expand` split into a pure
`site_extractor` and an emitting `extractor_steps` — without changing what it
emits or the order it allocates ids in.

`gathered:crude-oil` against `workspace/scripts/map.json` (t=0, no oil charted)
refuses `NotCharted`, which is the correct answer there.
