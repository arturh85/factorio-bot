# A replan finishes what it began

Three runs and the offline harness died on one sentence tonight, and it was
read as a belt-routing problem: *a replan does not recognise its own
half-built link, and either routes round it or lays a second one.* Put back
under the planner from their own keyframes (`plan --standing-from-run`, ten
seconds each), the mechanism is one level up from the belt, and there was
no ownership anywhere to lose.

## What actually decides "is this mine"

Nothing tracks provenance. A placed belt is a belt; a chest is a chest. A
replan recognises standing work only **functionally, and only at a sink**:

| what | recognised by | where |
|---|---|---|
| a science cell | a standing `assembling-machine-1` on a layout tile | `assemble::complete_cell` -> `fit_partial` |
| a supply link | an inserter whose DROP lands in the supply chest | `assemble::already_filled_by_machine` |
| a coal haul | an inserter delivering into the chest (plus a flow veto) | `sustain::fed_by_machine` |

`Occupant::Reserved` (`a69ae64c`) is not related: it is a promise about
ground inside one `PlanState`, re-derived from the standing offtake on every
round, and says nothing about entities.

So the question "half-built link" reduces to which parts of a cell stand
when a batch is cut. Read off `run-1788941729-70024`'s record: the failed
action was `take 1 copper-plate from the cell`, and its dependency cone was
`craft copper-cable -> craft small-electric-pole -> place pole -> research
automation -> craft 2 assembling-machine-1 -> place both machines, set both
recipes, both charges`. **Everything else stood** at the replan (tick
53,546): three chests, four inserters, four poles, the lab, and the whole
supply link -- load arm on the kept east exit `[30.5,-46.5]`, 25 belts,
unload arm at `[31.5,-31.5]`. The machines are the LAST parts of a cell to
stand, because both need `automation` and every electric arm needs a
circuit off the same take. `complete_cell` keyed on the one part that was
missing, saw no cell, sited a fresh one at `[44.5,-32.5]`, and the fresh
cell's supply chest needed a second link out of a plate chest whose fourth
side the first link's own arm holds. That is the refusal, byte for byte.

The harness (`plan --replan 1 --fail "copper-plate from the cell"`) cuts a
different cone: **every inserter of the link and of the cell** (each needs a
circuit) and **no belt** (iron only). That is the genuinely half-built link
-- 25 belts from the kept exit to the supply chest's door, no arm at either
end -- and there the belts were obstacles, the exit's belt tile read as
taken, and the chest reported north as the side it had left.

## What ships

- **`assemble::complete_cell` recovers a cell from ANY standing part**, not
  only a machine. A seed entity is taken as each role its name can hold
  (chest: feed/supply/output; inserter: the four arms; pole: the cell's own),
  at each facing, and the layout with the most parts on their tiles wins.
  **Two parts is the floor for a non-machine seed** -- a lone chest anywhere
  is never a cell -- and a machine still seeds on its own, machines tried
  first, so every plan with a machine standing is byte-identical.
- Two checks that were right for siting and wrong for finishing:
  `supply_chest_is_reachable` now answers yes for a chest a machine already
  fills or a chest with a belt chain ENDING at its door (without this the true
  layout of 70024's cell was rejected -- its free sides all opened onto the
  coal run -- and a *mirror* layout with the feed chest as supply chest was
  recovered instead); and the lane behind a supply chest a machine fills may
  hold that machine's arm (plan 1 put the link's unload arm there).
- **`connect::standing_run`**: before routing, a belt chain that leaves a
  perimeter side of `from` (arm tile free, belt standing) and, followed by
  belt direction through standing belts, ends on the belt tile of a
  perimeter side of `to` (arm tile free) is this run. `connect` then emits
  the two arms and nothing else. No ownership: a chain from this door to
  that door is a link between the two, whoever laid it. A chain ending
  anywhere else is an obstacle as before; a chain with an underground pair
  in it is followed only to the pair.

## Measured, one binary, `map.json`, bots 1,2,3,4

All seven canonical baselines and the science bundle are byte-identical
(176/21,784 · 316/22,457 · 559/52,298 · 194/33,645 · 319/119,396 ·
1,234/64,564 · 479/20,904 · bundle 909/54,890). `planning_work_ceilings`
forks for green: 48,801 of 60,756, unchanged. A t=0 plan has nothing
standing, so nothing here can reach it.

The replan-shaped worlds moved, all in the direction of finishing:

| world | before | after | what the after-plan places |
|---|---:|---:|---|
| run 70024 at tick 53,546 | **refused** (4 tiles round the plate chest) | 91 / 22,161 | the two machines, `research automation` |
| harness `--fail`, round 2 | 408 / 23,659, a second cell and a second link from the north side | 284 / 22,349 | the two machines, the cell's four arms, the link's two arms on `[30.5,-46.5]` and `[31.5,-31.5]`; no belt |
| run 99544 at tick 62,222 | 416 / 34,806, a pair and `logistics` | 128 / 21,237 | the two machines; no pair, no `logistics` |

`replan_sealed_supply::the_replan_of_run_1788936524_99544_plans` pinned
"the link tunnels with a pair" and is re-pinned to "the half-built cell is
finished and nothing tunnels"; the direct `connect` assertion beside it (the
plate chest has a way out) is unchanged.

## Tests, and what each is shown to fail on

Mutation sweep, backed up by copy and restored by copy plus `touch`:

| mutation | red |
|---|---|
| `least = 1` for every seed | `a_lone_chest_does_not_seed_a_cell` |
| only machines seed (the old filter) | `a_cell_whose_chests_and_arms_stand_is_finished_around_them`, `the_replan_of_run_1788941729_70024_finishes_its_half_built_cell` |
| `standing_run` never fires | `a_standing_run_missing_its_arms_is_finished_with_two_arms`, `a_link_missing_only_its_arms_is_finished_with_two_arms` |
| any chain tail accepted as the sink | `a_chain_that_ends_short_of_the_sink_is_not_taken_as_the_run` |
| a machine-filled chest not reachable by that fact | the 70024 replan test -- **green on the first sweep**, because the door-belt clause then accepted any standing belt and the coal run passing the chest's west side at `[29.5,-30.5]` counted; tightened to a chain whose flow ends at the door, and red on the second |
| the lane behind a machine-filled supply chest must be free | the 70024 replan test |

Fixture: `crates/planner/tests/fixtures/run-1788941729-70024-tick53546.json`,
regenerated by the command in `tests/replan_finishes_its_cell.rs`.

## What this does not settle

- **Whether the second run gets past it live.** Every number here is an
  offline plan. The one thing only a run says is whether the finished
  cell's machines land and the standing link delivers into them.
- **A partial belt chain** -- belts placed one by one, a bot dying mid-run
  -- is not this shape. `standing_run` matches a chain that reaches the
  sink's door; a chain that stops short is an obstacle as before, and
  extending it needs the router to enter a standing belt in its own
  direction. Not built; the two observed shapes are both whole-chain.
- **A link whose unload arm stands but load arm does not** reads as complete
  (`already_filled_by_machine` is sink-only) and gets no load arm. Both arms
  share the circuit dependency, so it has not been observed; stated.
- **Plan 1 laid the link's unload arm on the cell's lane** (`[31.5,-31.5]`
  behind the supply chest). Tolerated on the replan; not closed to the
  route in `supply_link_steps`, because closing it re-sites the link on a
  chest with no other side and that is a siting question.
- Recovery by geometry has the ambiguity CLAUDE.md records for
  `Goal::Built` -- two layouts sharing a sub-layout. The two-part floor and
  the machine-first order bound it; they do not remove it.
