# A boxed-in source is tapped, not refused: the splitter lands in the dynamic router

Follows `2026-09-09-the-cell-fix-holds-live.md` and
`2026-09-09-the-fix-worked-and-the-predicted-blocker-fired.md`, which both end
on the same shape and call it a design question. The owner answered it
(2026-09-09): of *splice a splitter into the run the source already has*, *build
a second source chest*, or *avoid needing the source twice*, **"1 sounds
good."** This is (1), built, measured offline against the stuck run, and
committed with its apparatus.

## The problem, read off the record rather than restated

`run-1788946451-86723` (seed 31337, four headless bots at 10x, honest,
`5323e71a`) ended `stuck` at tick 64,053:

```
a cell already makes copper-plate at [29.5, -46.5] and nothing can carry it to
the supply chest at [30.5, -55.5]: from the iron-chest at [29.5, -46.5]: no belt
route, blocked by 4 tile(s): [28.5, -46.5] [29.5, -47.5] [29.5, -45.5] [30.5, -46.5]
```

The four tiles, off the keyframe at that tick: west `[28.5,-46.5]` is the
offtake arm (furnace → chest), north `[29.5,-47.5]` and south `[29.5,-45.5]`
are the cell's own coal arm and coal belt, east `[30.5,-46.5]` is the electric
`inserter` carrying the chest's plates onto the run plan 1 laid — east to
`x=36.5`, south to `y=-32.5`, west into the first science cell's supply chest.
The replan did not recognise that science cell (no machine with a recipe
stood), sited a fresh one to the north, and its supply chest needed a second
run out of a chest with no side left. **The refusal was right.** Two inserters
need two tiles; there is no tile.

Reproduced offline in ten seconds on `7665ffde`, byte for byte:

```
factorio-bot plan --world workspace/scripts/map.json \
    --standing-from-run workspace/runs/run-1788946451-86723 --at-tick 64053 \
    --all --goal sustain:copper-plate:15:36000 \
    --goal producing:automation-science-pack:6
```

## What a splitter is, before building one

The brief said "2 tiles long in the direction of travel, 1 tile wide". **That is
backwards**, and the prototype says so: `splitter`'s collision box is
`1.796875 x 0.796875` facing north (`crates/core/tests/entity-prototype-
fixtures.json`), i.e. **two tiles wide ACROSS the direction of travel and one
tile long ALONG it**. `entity_graph.rs`'s splitter arm agrees — its two output
tiles are `(-0.5, -1)` and `(0.5, -1)` turned to the facing, and its position is
the midpoint of its two tiles (whole number on one axis, half on the other:
`[37, -45.5]` for one facing south over `x=36.5` and `x=37.5`). Both lanes enter
on the back edge, both leave on the front edge, and with the defaults the
splitter divides by availability: everything goes to whichever output is not
backed up. That last property is what the fallback rests on — the run keeps its
whole flow when the branch is full, and the branch gets it all when the original
destination is.

Its recipe needs `logistics`, like `underground-belt`, so a plan that taps
carries that research; the `Goal::Have splitter` says so through the ordinary
shortfall path.

## What was built

All in `crates/planner/src/method/connect.rs` and `crates/core/src/graph/route.rs`;
`sustain.rs` gained one match arm; `assemble.rs` and `cellstock.rs` are untouched.

**`route_belt_launching`** (`route.rs`): the plain search with the first tile's
facing fixed. It seeds only the launch facing at `from` and offers only the step
(or jump) in that direction out of it, so the first belt continues the emitter's
way *by construction* — the same reason the underground exit has its
straight-tile rule. A splitter pushes items straight ahead; a sideways belt on
its output tile is side-loaded onto one lane.

**`tap_standing_run`** (`connect.rs`), reached from `connect_steps_reserving`
only when **the `from` end's perimeter is what refused** — a sink with no side
or a route with no path refuses exactly as before, and every plan that routed
before is byte-identical (the seven t=0 baselines below did not move by a
tick). It:

1. reads every belt chain leaving `from` through an arm whose facing says it
   picks up *from* `from` (`outbound_chains`; a run into the machine is not a
   door);
2. chooses the destination's end as a fresh run would (`first_free_perimeter`);
3. lists every splice (`tap_candidates`): three same-facing belts in a row, the
   middle one replaced; the side tile and the tile in front of it free on the
   caller's placement grid; nothing belt-connectable behind the side tile
   facing in (it would merge onto the run). Nearest branch start to the sink
   first;
4. routes the branch from the splitter's second output with
   `route_belt_launching`, on the same grids in the same order as the plain
   run — threats avoided first, and **surface from any splice before a tunnel
   from the nearest**, which the fixture forced: the nearest splice launched
   straight at the destination's standing arm and went under it;
5. emits the bill (one `splitter`, the branch's belts and pairs, **one**
   inserter — the load arm stands), the branch, the unload arm, then a `Chop`
   of the belt tile the splitter replaces and the splitter's `Place`, linked
   chop-before-place.

The belt is chopped rather than fast-replaced because the plan speaks in
`AreaFree` and `RemoveEntity`: the chop's effect makes the splitter's
precondition true in the overlay and the belt comes back as an item. The mod's
`rcon_place_entity` does pass `fast_replace`, so the game would have accepted
the splitter over the belt; the planner would not have.

**And the chop waits for the splitter to be in hand.** The first plan chopped at
tick 4,509 and placed the splitter at 37,140 — on the far side of `research
logistics` and the craft — leaving the run cut for 32,000 ticks. A `HasItem
splitter` precondition on the chop (spending nothing; the `Place` spends it)
links it to the splitter's producer like any consumer, and it now lands at
37,143 beside the placement at 37,155.

The refusal is honest about the fallback: a boxed-in source with no run out
refuses as `ConnectRefusal::TapRefused { blocked, why }`, whose text opens with
the exact `no belt route, blocked by 4 tile(s): ...` sentence it always had and
then says the tap was tried and why it could not be. Silence about a fallback is
how a fallback goes unmeasured.

## Measured, one binary, `7665ffde` plus this change

The stuck replan, same command as above:

| | before | after |
|---|---|---|
| `run-1788946451-86723` at tick 64,053 | **refuses**, the four tiles | **341 actions / 37,185 ticks** |

The tap it chose: chop `transport-belt` at `[36.5, -45.5]` (the run's southbound
leg), splitter at `[37, -45.5]` facing south, branch from `[37.5, -44.5]` south
one tile, east to `x=38.5`, north to `y=-57.5`, west to `[30.5, -57.5]`, unload
`inserter` at `[30.5, -56.5]` into the supply chest at `[30.5, -55.5]`. Why not
the nearer eastbound leg at `y=-46.5`: its north side is under a hand-smelt
`stone-furnace` at `[34, -48]` and its south side is the cell's westbound coal
belt — read off the keyframe, both honest obstacles. No underground pair. The
plan carries `research logistics` (9,000 ticks) because a standing snapshot
carries no research state; a live replan reads the force's own.

The seven t=0 baselines, before and after on the same binary — **unchanged to
the tick**, as the fallback shape promises:

| goal | before | after |
|---|---|---|
| `researched:automation` | 176 / 21,784 | 176 / 21,784 |
| `producing:automation-science-pack:6` | 316 / 22,457 | 316 / 22,457 |
| `producing:logistic-science-pack:6` | 559 / 52,298 | 559 / 52,298 |
| `producing:iron-plate:261` | 194 / 33,645 | 194 / 33,645 |
| `producing:transport-belt:6` | 319 / 119,396 | 319 / 119,396 |
| `sustain:iron-plate:30:36000` | 1,234 / 64,564 | 1,234 / 64,564 |
| `sustain:copper-plate:15:36000` | 479 / 20,904 | 479 / 20,904 |

`just replan-check` (bundle, `--replan 1`): 909 / 54,890 then everything
standing. With `--fail "copper-plate from the cell"`: second round **plans, 284
actions / 22,349 ticks, with no splitter** — the failed-take shape is finished
by `standing_run`, not tapped; the harness note recorded that round refusing on
an earlier master, and no before-figure was taken on this binary for it, so it
is reported and not claimed.

## Tests, and what each can fail on

- `connect::tests::a_source_with_no_free_side_is_tapped_where_its_run_already_leaves`
  — the miniature of the run: a 1x1 source with its run standing and its
  other three sides taken. One splitter facing the run's way over one chopped
  belt of the run and one free tile; branch starting on the second output
  tile, continuing straight, never on the run, no tunnel where the surface
  serves; one arm, at the destination; chop before splitter with a stated
  link; bill of one splitter and one arm; overlay agrees.
- `a_source_with_a_free_side_is_never_tapped` — the fallback is a fallback.
- `a_boxed_in_source_with_no_run_out_refuses_and_says_the_tap_was_tried` —
  the four tiles, the old opening sentence, nothing in the overlay.
- `route_grid::a_launched_route_leaves_in_its_launch_direction_before_it_turns`
  and `..._of_one_tile_faces_its_launch`, against the plain search as control.
- `replan_taps_the_run::the_replan_of_run_1788946451_86723_taps_the_standing_run`
  — the run's own keyframe, checked in as
  `crates/planner/tests/fixtures/run-1788946451-86723-tick64053.json`
  (regenerate with `--save-standing`): plans, one splitter over one chopped
  belt of the record facing that belt's way, no second arm on the plate chest.

Falsification by copy-and-touch (see the commit that follows this note): with
`outbound_chains`'s door test inverted the fixture test and the run test go
red on `TapRefused`; with the launch guard in `search_once` deleted the route
test goes red on a first tile facing north.

## Scope, stated

Phase 1. The branch is emitted flat under the chain actor (no bands); a
splice needs three same-facing belts in a row, so a run that turns every other
tile refuses by name; an underground pair in the standing run ends the chain;
priority and filter are left at the prototype's defaults. Nothing here can say
what the tapped run carries — it carries whatever the door's arm lifts out of
`from`, which for the chests and machines this module joins is the one thing
they hold. **Not run live.** The next measured run of `continuous_supply.lua`
is the check of record: the mod mines a belt through the same
`action_start_mining` a rock goes through, and places a splitter through the
same `rcon_place_entity` `FurnaceLine`'s two went through, but neither has been
asked to do it in this order on this cell.
