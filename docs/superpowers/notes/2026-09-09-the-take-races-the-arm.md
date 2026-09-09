# The take was reading a slot the cell's own arm keeps empty

Follows `2026-09-09-thrashing-on-the-same-take.md`, which stated the symptom:
`run-1788949638-11792` replanned six times, retried `craft 2
assembling-machine-1` in every one, and lost it every time behind a `take
copper-plate from the cell` that came back `removed 0`. This note is what the
record says the mechanism was, the fix, and what the fix does not cover.

## What the record says, before any code was read

Every one of the eleven failures targets **one entity**, and it is not a
chest. The plan step for each carries `target: {x: 27.0, y: -46.0}` -- an
integer position, so a 2x2 `stone-furnace`, the copper cell's own furnace.
`produce::take_steps` emits `ActionKind::Remove { slot: FurnaceResult }` at
`cell.furnace`; it was never reading a chest, and nothing was re-sited. The
same furnace served the three takes that *succeeded* at ticks 23,505--25,241
(`took 10 copper-plate in 3 pieces ... the source held 8 when first asked`),
so the coordinates were right for the whole run.

`samples.jsonl`, the furnace at `[27,-46]`, status and `products_finished`:

```
tick    status           made   output
19200   working             0   {}
22200   working            13   {copper-plate: 9}     <- takes succeed here
27000   no_ingredients     33   {copper-plate: 8}
30900   no_ingredients     33   {}                    <- arm fuelled, slot drained
43500   working            33   {}
45000   no_ingredients     39   {}                    <- take 631 fails, tick 46,086
54300   working            40   {}
83700   no_ingredients    153   {}                    <- 120 plates, output {} at every sample
```

**The furnace worked the whole time.** Between 43,500 and 83,700 it smelted
120 plates, and its output slot read `{}` at every one of ~130 samples. The
plates went where the arrangement sends them:

```
burner-inserter [28.5,-46.5] dir 12   picks up from the furnace, drops into
iron-chest      [29.5,-46.5]          "hold the copper-plate the cell makes"
inserter        [30.5,-46.5] dir 12   onto a belt east, powered from 49,800
belt ... -> iron-chest [31.5,-30.5]   the science cell's SUPPLY chest
```

The plate chest held 3 -> 21 plates from 22,200 to 45,000, drained to 0 by
55,800 once the electric arm had power, and the supply chest at `[31.5,-30.5]`
then climbed 0 -> **133 copper-plate** by 84,300 with nothing ever taking one
out -- the assembling machines it was for were never built.

So: **not stale coordinates, not a supply gap.** The take targeted the right
furnace, the furnace was producing, and `sustain`'s offtake arm lifted every
plate out of the result slot within ticks of it being made. A hand reading
that slot finds zero by construction, and `take_in_pieces` (`473c7294`) --
written for a source that is *slower* than the ask -- waits its full 1,800
ticks for a slot that will never fill and fails as "the source is not
producing", which is the one thing it was not.

**It is the t=0 plan too, not only the replans.** Plan 1 placed the arm at
planned tick 16,736 (`#17 place burner-inserter at [28.5,-46.5] -- take
copper-plate out of the stone-furnace`) and scheduled hand-takes from the same
furnace at 20,641, 20,898, 21,378 and 40,222. The first three succeeded live
only because the arm had no fuel yet; the fourth ran after it did. Two methods
had planned the same furnace with contradictory beliefs about where its output
lives.

## Reproduced offline, on the run's own world

Master `c385c409`, release binary built from it:

```
plan --world map.json --bots 1,2,3,4 --all \
     --goal sustain:copper-plate:15:36000 --goal producing:automation-science-pack:6 --steps
  #17  17808  place burner-inserter at [28.5,-46.5] -- take copper-plate out of the stone-furnace
  #626 21412  take 10 copper-plate from the cell
  #638 21970  take 5 copper-plate from the cell
  #649 23341  take 2 copper-plate from the cell
  #631 31842  take 1 copper-plate from the cell        (909 actions / 54,890)

plan ... --standing-from-run workspace/runs/run-1788949638-11792 --at-tick 80314 --steps
  standing world ... at keyframe tick 80314: 187 placed
  #8   take 10 copper-plate from the cell
  #19  take 5 copper-plate from the cell
  #29  take 4 copper-plate from the cell                (71 actions / 22,096)
```

**One trap in the harness worth writing down**: `--at-tick 55000` on this run
answered `keyframe tick 410, 0 placed` -- an empty world -- and planned
happily on it. The run's `map.jsonl` has keyframes at 410 and then nothing
until 80,314, so the loader was right and the request was wrong; but nothing
refused, and a replan against t=0 ground reads exactly like a replan that
found no defect. Check the `standing world ... N placed` line before reading
a replan's output.

## The fix

`produce::cell_ledger` now skips a cell whose furnace has an offtake --
`sustain::has_offtake`, which is `standing_offtake(..).is_some()`, the
predicate `sustain` already used to recognise its own arm. It reads the
overlay, so an arm this expansion has just planned is seen by a fragment
expanded after it, and an arm standing from an earlier plan is seen on the
replan. The fragment is then served the way one with no live cell is served:
another cell, or a hand smelt. What the arm carries away is `sustain`'s to
route, not a hand's to race.

Same binary, with the change:

```
t=0 bundle              923 / 57,797   0 takes from an armed furnace (was 4)
replan at 80,314         97 / 22,202   0 (was 3); both assembling machines planned
```

`take 50 iron-plate from the cell` survives in both -- a one-shot iron cell
of the plan's own, with no arm on it, which is the case the ledger is for.

Test: `sustain::tests::a_furnace_with_an_offtake_is_not_a_hands_source`.
Through `standing::world_after`, not a fork -- a fork carries the first
expansion's resource claims and would hide the cell from the ledger for the
wrong reason. With the `continue` removed the test fails on `take 10
iron-plate from the cell` drawn from the armed furnace (mutation by copy,
restored by copy plus `touch`).

**Two replan tests moved with it, and each says what it measures now.**
`replan_on_standing_world::a_failed_take_leaves_the_plate_chest_a_way_out`
failed the first `copper-plate from the cell` take -- a label that no longer
exists in the bundle. It fails the first copper take of any furnace now, and
that take is a late hand smelt (896 of 923 actions survive it), so the supply
link stands and the plate chest's kept exit holds the link's own arm -- the
exit being *used*. The invariant now reads "a free side, or a standing arm
whose pickup is this chest"; a chest walled in by anything else still trips
it. `replan_taps_the_run` counted every `Chop` as "exactly one belt comes
up"; the replan of run 86723 now chops a rock as well, for the stone of a
hand-smelt furnace, and the count is of belts. It still taps: one splitter,
one belt, no second arm on the chest.

**And the replan after a failed take now plans.** That test's own doc
records the replan refusing on master, further along, on two open items
(the half-built link's belt, the supply chest boxed in). On `6eb0fa7b` it
plans: 225 actions, makespan 22,403. Not chased here -- the failed take is a
different action now, so the world it leaves is a different world -- but it
is the first time that path has come back `Ok`, and the test's `match` can
be tightened to `expect` once someone confirms it holds for the cell's own
take failing too.

## Baselines, one binary each side, `c385c409` + `6eb0fa7b`

| goal | master | fix |
|---|---|---|
| `researched:automation` | 176 / 21,784 | same |
| `producing:automation-science-pack:6` | 316 / 22,457 | same |
| `producing:logistic-science-pack:6` | 559 / 52,298 | same |
| `producing:iron-plate:261` | 194 / 33,645 | same |
| `producing:transport-belt:6` | 319 / 119,396 | same |
| `sustain:iron-plate:30:36000` | 1,234 / 64,564 | **1,272 / 63,905** |
| `sustain:copper-plate:15:36000` | 479 / 20,904 | same |
| `gathered:crude-oil` (explored) | 2,352 / 322,738 | same |

**`sustain:iron-plate` moved, and it is the same defect.** On master that
plan has 11 `take .. iron-plate from the cell`; eight of them draw from the
two furnaces the plan itself puts offtake arms on. With the fix those eight
are gone (3 remain, on unarmed cells), 8 more hand inserts stand in for them,
and the makespan is 659 ticks shorter. Those eight takes would have raced the
arm live exactly as the copper ones did.

**`gathered:crude-oil` reads 2,352 / 322,738 on `c385c409`**, against the
2,330 / 354,699 `BASELINES.md` records at `c0e51463`. Identical on both
binaries here, so it moved between those commits, not with this change.

## What this does not do

**The 133 plates in the supply chest are still invisible to every replan.**
`BUFFER_ENTITIES` is `["stone-furnace", "wooden-chest"]`; the iron chest is
excluded on purpose, with the reason beside it (`crates/core/src/plan/
planner.rs`): a cell's three input chests are iron chests too, and a
name-keyed list cannot tell a store from a feed. So `Withdraw` cannot see the
plate chest or the supply chest, and a replan hand-smelts copper beside a
chest holding 133 of it. That is the design question the exclusion already
names -- *what a particular chest is for* is a property of the plan that
sited it -- and it is not answered here.

**The `too far too mine` at 72,107** is a separate, one-off walk failure on
bot 4 (`mine 1 coal` at `[28.5,-32.5]`): it cost 19 abandoned steps in plan 4
and did not recur. Not forced into this story.

**Nothing here touches `route.rs` or the splitter tap** (`23c30a3f`). The
splitter never fired in this run and this defect is upstream of routing.
