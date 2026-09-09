# The ring was the other end

Three measured runs on 2026-09-09 died on the same sentence, and the sentence
was read the same way each time: *the plate cell's four coal runs draw a
closed ring round the cell, so a route out must tunnel, and the tunnel is
longer than the belt allows.* This note is what the runs' own keyframes say
when they are put back under the planner (`plan --standing-from-run`, ten
seconds, no game), and it is not that.

```
run-1788920460-08860   blocked by 4 tile(s)                      the chest's own perimeter (fixed a69ae64c)
run-1788923927-04849   blocked by 4 tile(s)                      a hand furnace ON the kept exit (fixed a69ae64c)
run-1788924859-39900   underground span of 11 tiles, allows 5    a genuine ring -- on THAT binary's layout
run-1788936524-99544   underground span of 7 tiles, allows 5     the SINK: engine | belt | boiler
```

## What the number means

`RouteError::SpanTooLong { needed }` is `widest_blocked_run + 1` measured on
the **Bresenham line from source to sink** after the search has failed. Its
own doc says it *"only ever looks at the direct line"*. So "7" is the widest
run of anything the straight line from `[29.5,-46.5]` to `[42.5,-26.5]`
happens to cross -- the plate cell, a hand furnace, the coal haul -- and says
nothing about which end is shut. Three notes took it as the thickness of a
wall round the source. **Now a sink with all four neighbours blocked is
refused by those four tiles** (`route::walls_around`), which is what the
search had already proved: no jump lands on a cell it cannot leave.

## Run 99544, cell by cell

`run-1788936524-99544` (seed 31337, four headless bots at 10x, honest,
release from `c1fd2620`) built its first plan whole except the cone of
`take 10 copper-plate from the cell` (failed with 3 of 10), and replanned at
tick 62,222. On the keyframe, with the science bundle expanded exactly as
the supervisor does:

- **The plate chest's kept exit is open.** `[30.5,-46.5] [31.5,-46.5]` are
  free, as `a69ae64c` promised. They open into a three-tile pocket sealed by
  the first plan's own supply link (its belt along `y=-48.5`, its pole at
  `[30.5,-47.5]`), a hand-smelt furnace at `[33,-47]`, and the furnace's coal
  run along `y=-45.5`. **Not by the coal runs**: on this binary the branch
  to the offtake arm runs west of the cell and along `y=-49.5` only as far as
  `x=28`, leaving the north-east open. A 2-span pair north over the link
  belt gets out, and `connect` finds it: asked to route from the plate chest
  to a bare chest at the replan's own supply-chest position, with nothing
  else of that cell standing, **it routes** (test
  `the_plate_chest_of_run_1788936524_99544_has_a_way_out`).
- **The link's destination was unreachable.** The half-built science cell
  has no assembler (its placements were in the abandoned cone), so
  `complete_cell` finds nothing to finish and `plan_cell` sites a fresh one
  from the nearest powered pole -- which put it on the shore beside the
  standing plant. Its supply chest at `[42.5,-26.5]`: north the cell's own
  supply inserter, west the cell's feed chest, east a pipe, south the only
  free pair -- an arm tile and then `[42.5,-24.5]`, **the one-tile gap
  between the steam engine and the boiler**, whose neighbours are engine,
  boiler, pipe, and the arm's own tile. `first_free_perimeter` accepts a pair
  when both tiles are free; nothing asked whether the belt tile can be
  entered.

So the mechanism is neither of the two hypotheses put to this session. Not
"the coal runs are sited blind to the reserved exit" (they keep it, and on
this layout they do not ring it), and not "the underground reach is the wrong
number" (the span is a label, not a measurement, and the pocket is crossed
by a 2-span pair). It is the repo's recurring shape: `belt_reaches_open_ground`
existed and was composed with the plate chest's exit (`choose_exit`) and with
nothing else. **The supply chest is the one part of a cell that another
method's belt has to reach, and siting never asked whether one could.**

## What ships

- `assemble::supply_chest_is_reachable`: a cell is sited only where one side
  of its supply chest has the arm's tile and the belt's tile free **with the
  arm standing** and the belt tile reaching the window's edge on the surface.
  The arm has to stand for the question to be the right one: in 99544 the
  belt tile's only free neighbour was the arm's tile.
- `route::walls_around`: a sealed sink is named by its walls, never by a span.

Run 99544's replan now **plans**: 416 actions, the science cell re-sited on
open ground east (`x = 42..46, y = -35..-28`), the link leaving the plate
chest by the **reserved east exit** for the first time, tunnelling north with
one pair, and `research logistics` in the plan because a pair needs it.
`just replan-check --fail "copper-plate from the cell"` -- the offline
reproduction that refused on the science cell's supply chest boxed in by four
tiles -- now plans its second round too: 462 actions / 36,629 ticks.

## A baseline moved, and why

`producing:logistic-science-pack:6` on `map.json`, four bots, one binary:
**571 / 54,371 -> 559 / 52,298**. One site is refused that used to be
accepted -- the sixth cell at `[43.5,-11.5]`, whose supply chest at
`[43.5,-8.5]` opened onto the plan's own plant gap at `[43.5,-6.5]`, the
99544 shape at t=0 -- and the cells pack differently from there. Green never
lays a belt into that chest (its supply is a bot's stockpile), so the old
plan was not broken; the rule refuses a cell that could never be belted,
which is the direction the project has chosen. Shorter by chance, stated as
chance. `planning_work_ceilings` pins green's action count and was updated
to 559 with this reason beside it; the other six t=0 baselines and the
science bundle are byte-identical.

## What this does not settle

- **Whether a run gets past it.** Offline the replan plans; the pair costs
  `logistics` off a hand charge (measured earlier at ~9,000 research ticks).
  Only a run says whether the link is laid and plates arrive.
- **The link ignores the exit it was kept.** In plan 0 `first_free_perimeter`
  took the plate chest's north side, and the link's own belt and pole are
  what sealed the reserved east exit into a pocket. The reservation keeps two
  tiles free; it does not keep a way out of them, and the plan's own later
  placements (link, pole, hand furnace) closed it. Preferring the reserved
  pair for the run it was kept for is a one-line change in `connect` that
  nobody has measured.
- **A replan still throws away a half-built science cell** whose assembler
  is missing -- `complete_cell` finishes only around a standing machine.
  The re-siting works around it at the price of a second set of chests and
  inserters.
- Run 39900's ring was real on `a69ae64c`'s layout (double walls at
  `x=24.5/25.5`, `34.5/35.5`, `y=-51.5/-52.5`, read off its keyframe), and
  the layout has moved since. CLAUDE.md's "every kept exit is sealed" was a
  measurement of that binary and is amended.

Fixture: `crates/planner/tests/fixtures/run-1788936524-99544-tick62222.json`,
regenerated by the command in `tests/replan_sealed_supply.rs`.

## Closed the same day: the link takes the exit it was kept

The "one-line change in `connect`" above was two changes, and the second was
the one that mattered. Reproduced offline from the run's own keyframe with
the link taken back out of it -- `map.jsonl` has no keyframe between 410 and
62,222, and `events.jsonl` dispatches exactly 35 placements from tick 56,300
to the keyframe, all of them the link and its wire (poles 533-536, arms 502
and 532, belts 503-531) -- so the tick-62,222 fixture minus those 35 is the
world at tick 56,299 (`replan_sealed_supply::fixture_before_the_link`). On
`ff291941` that world plans the link's arm at `[29.5, -47.5]`, the north
side, exactly where the run put it at tick 60,611.

- **`first_free_perimeter` could not see the reservation.** It took the
  obstacle grid, the origin and the footprint, and walked N/E/S/W for the
  first free pair. The call site exempted a kept tile from the grid, so the
  exit was *usable*, and nothing ranked it. Now the `from` end is handed
  the kept pair (`kept`), a candidate whose inserter AND belt cells are both
  kept goes first, and a kept pair the route cannot leave by falls back to
  the plain order rather than refusing. The `to` end keeps the plain order:
  a reservation is a product exit, a run INTO the chest is not what it was
  kept for, and `keeper` is prose with no direction in it.
- **And sustain reserved the exit twice, on two sides.** `Sustain::expand`
  reserves every standing cell's exit before its cell loop and reads the
  offtake again inside it; the second read went through `product_exits`,
  which counts a side only when both tiles are free, and the tiles the first
  read had just reserved were `Occupant::Reserved`. So the east exit no
  longer counted, north ranked first, and a second pair was kept -- the
  reservation blinded its own author. With two kept sides `connect` took the
  first in scan order, north, and the preference alone changed nothing (the
  mutation sweep showed each half necessary: drop either and the derived
  world's link is back on the north side). `standing_offtake` now reads the
  state's own promise first (`kept_exit_of`, both tiles in a line).

Measured on one binary, `map.json`, bots 1,2,3,4: the seven canonical
baselines are byte-identical (a t=0 plan sites nothing on an exit before the
link is laid, and the link is the last thing the bundle lays). What moved:

```
science bundle, t=0            913 / 55,177  ->  909 / 54,890   link leaves by [30.5,-46.5]
replan-check --fail, 2nd round 462 / 36,629  ->  408 / 23,659   no pair, no logistics
derived world at tick 56,299   287 / 21,298  ->  287 / 21,298   arm [29.5,-47.5] -> [30.5,-46.5]
replan at tick 62,222          416 / 34,806  ->  416 / 34,806   unchanged: north already stood
```

The replan-check's second round no longer tunnels: the first plan's link
left by the east exit, so nothing sealed it. What it does instead is lay a
**second** link from the north side, because the replan does not recognise
its own standing link (the open item above), and with the east exit now
occupied by that link's arm the standing chest honestly reports north as the
side it has left. That is the next thing to close, and it is not this one.
`planning_work_ceilings` forks are unchanged at 48,801 of 60,756.
