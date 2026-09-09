# The route stood on its own tile, and called it an obstacle

`run-1788926478-07032` made the project's first machine-made science packs
(14 `automation-science-pack`, all by `assembling-machine-1`) and then ended
`stuck` at 15:29 with 80 abandoned steps on

```
nothing can carry coal from the buffer at [32.5,-41.5] to the iron-chest at
[27.5,-40.5]: no belt route, blocked by 1 tile(s): [30.5,-39.5]
```

The record showed `[30.5,-39.5]` **empty** in every keyframe covering it, in
both the `game` and `model` arrays, and no bounding box of any standing
entity covers it either (the nearest, a `burner-mining-drill` at `[28,-39]`,
ends at x = 29). No `placement_refused` event exists in the run, so no stale
refused-footprint memo could have been consulted. The tile is grass.

## What is at the tile, read off a reproduction rather than reasoned about

The replan was reproduced offline in about two seconds
(`crates/planner/tests/replan_haul.rs`): the t=0 dump, plus the 224
non-resource entities of the run's own keyframe at tick 56,168 — the tick the
supervisor replanned — plus the four bots where `samples.jsonl` put them. The
expansion refuses **byte for byte** the same sentence. So does the haul asked
of `connect_steps_reserving` on its own, **with nothing reserved at all**.

That last measurement is the one that mattered. The fresh suspect was
`a69ae64c`'s reserved ground (`PlanState::reserve_ground`, `Occupant::Reserved`),
landed hours earlier and consulted by every `is_area_free`. It is innocent:
the reservations in the expansion are `[30.5,-46.5]` and `[31.5,-46.5]`, the
copper cell's product exit five tiles north, and the bare haul refuses
identically with an empty reservation list.

The tile came from `route_belt_with_tunnels`. Two facts, both printed from
the search before the fix:

**The buffer's belt tile sits in a pocket the cell's own belts seal on the
surface.** The surface search died against 33 tiles, all of them belts of
the cell's coal ring: the column at x = 29.5 from y = -42.5 to -33.5, the
rows at y = -43.5 and y = -32.5, the column at x = 36.5. That is the ring the
open item names, and the haul out of it must tunnel.

**The tunnel search found a route and the route stood on `[30.5,-39.5]`
twice:**

```
[30.5,-41.5] S belt   [30.5,-40.5] S belt   [30.5,-39.5] S belt
[30.5,-38.5] E belt   [31.5,-38.5] N belt   [31.5,-39.5] W belt
[30.5,-39.5] W underground ENTRY  ->  [26.5,-39.5] W underground EXIT
[25.5,-39.5] N belt   [25.5,-40.5] N belt
```

A jump is only launched straight, so a run heading south that must dive west
turns a three-tile hook to arrive facing west — and the hook that turns
north re-enters the column on the tile it left. The state space is
`(cell, facing, surfaced)`; the same cell with a different facing is a
different state, and A* has no memory of its path. `self_crossing` caught the
doubled tunnel ground, returned it as `NoPath { blocked }`, and `connect`
printed it as an obstacle. **The tile named as blocked was the route's own.**

## The fix, and the fix that did not work first

`route_belt_with_tunnels` now repairs a self-meeting route instead of
refusing it: the move that landed on the doubled cell is forbidden and the
search runs again. Bounded (`SELF_CROSSING_REPAIRS`, 8 rounds; the measured
case needs one). A route that never met itself is the first search's answer,
untouched, which is what keeps every route the planner made before this
byte-identical — the seven canonical baselines did not move.

The first version closed the doubled *cell* instead. Measured, it moved the
hook one row south per round and did not converge in five: the hook that
turns south is exactly as cheap as the one that turns north and only loses
the tie, so closing the cell pushed the exit down and the search rebuilt the
same illegal hook one tile further along. Forbidding the one step that lands
on the doubled cell is what lets the tied legal hook win. Whack-a-mole on
cells was the wrong unit; the unit is the move.

The refusal is honest either way now: when the rounds run out or a round
finds nothing, the tiles named are the obstacles the frontier touched **on
the caller's grid**, never the search's own bookkeeping.

## What it buys, and what it does not

The haul routes: 15 tiles, one underground pair west under the belt column.
The replan then proceeds to the next refusal, which is already on record as
open and is **not** closed here:

```
a cell already makes copper-plate at [29.5,-46.5] and nothing can carry it to
the supply chest at [21.5,-32.5]: from the iron-chest at [29.5,-46.5]: the
obstacle needs an underground span of 10 tiles and the belt allows 5; from
the stone-furnace at [27,-46]: no belt route, blocked by 8 tile(s): ...
```

That is the coal ring sealing the plate cell's exit two belts deep, and the
replan not recognising its own half-built supply link — the two items the
brief listed as open. This change removes the blocker in front of them and
nothing more. A measured re-run would now hit one of those, not this.

## The method, which is the transferable part

- **Read the tile before naming the mechanism.** Every hypothesis in the
  brief was about something *on* the tile. Nothing was on it; the planner had
  put itself there. Only a reproduction that printed the route could say so.
- **An offline plan from t=0 could never reach this.** The refusal is a
  replan's, against a standing factory. The fixture is 28 KB of the run's own
  keyframe and the test runs in seconds; it is the first regression in the
  tree that plans against a half-built world, which
  `2026-09-09-offline-cannot-see-the-replan.md` said nothing enforced.
- **A refusal that names a tile must name a tile on the map.** The error
  path had a variant for "this route collides with itself" and reused the
  one for "the map is in the way", so the message was grammatical, specific,
  and false. The two are now told apart by never letting the first reach
  the caller.
