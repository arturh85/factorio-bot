# A replan you can run offline, and what it found on the first day

Follows `2026-09-09-offline-cannot-see-the-replan.md`, which stated the gap:
every baseline is a plan from the t=0 dump, a t=0 dump has no factory in it,
and `9f549b1c` moved all eight baselines correctly and refused in the live run
with the blocker it had been written to remove. **Nothing in the tree closed
it.** This note is what closes it, how it was shown able to fail, and the two
things it found before it was finished.

## Two ways to a standing world, one seam

Both return a **new surface** with the entities as ordinary map facts -- the
same `on_some_entity_created` the mod's writeout reaches -- so that a fresh
`PlanState::from_world` over it is what the supervisor's next round builds.
`plan_rounds` (`crates/scripting_lua/src/globals/goal/plan.rs`) rebuilds its
state from the live world every round; so does this.

| | what it applies | where |
|---|---|---|
| `standing::world_after(state, net, done)` | a plan the planner made: placements, recipes, choppings, mined ore | `crates/planner/src/standing.rs` |
| `standing::world_with(base, snapshot)` | a finished run's record: keyframe entities, bot positions and inventories, machine recipes | same file; the snapshot type is `crates/core/src/record/standing.rs` |

**Not a fork, and that is the whole point.** The sustain tests' `built_world`
forks the plan state and creates the placements in its overlay. Right for
"what did this expansion build"; wrong for a replan, because a fork carries
every reservation the first expansion made into the second and masks exactly
the defect `a69ae64c` fixed. The live replan starts from the world and nothing
else.

**What neither carries, stated**: chest and furnace contents, research, ground
charted after t=0. `world_after` also carries no inventories or positions --
a replan after a *lost* batch is a replan whose inventory the plan cannot
vouch for anyway. `world_with` carries both, read off `samples.jsonl`.

## The commands

```bash
# plan, apply what was built, plan again from the standing world
just replan-check                                  # the science bundle, seed 31337
factorio-bot plan --world workspace/scripts/map.json --bots 1,2,3,4 --all \
    --replan 1 --goal sustain:copper-plate:15:36000 \
    --goal producing:automation-science-pack:6

#   --done-by <tick>   only steps finished by then count as built
#   --fail <label>     that action fails; its dependency cone is abandoned
#   --dump-standing    write the standing world as a full dump

# plan against the world a finished run's own record says it had
factorio-bot plan --world workspace/scripts/map.json \
    --standing-from-run workspace/runs/run-1788926478-07032 --at-tick 56168 \
    --all --goal ... --save-standing crates/planner/tests/fixtures/<run>-tick<T>.json

#   --standing <file>  read a saved snapshot back
```

`--all` matters: `goal.all{a, b}` holds one conjunct's "already standing"
refusal back while the other expands, and two sequential `--goal`s end at it.
A replan of a script's bundle has to be planned as the bundle, or the second
round refuses on the conjunct that stands, for a reason the run never sees.

A replan that finds the whole arrangement standing is read the way the driver
reads it (`SustainSupplyNotStanding` => satisfied), and any other refusal on a
replan fails the command **naming the round and what stood**.

## Tests, and what each can fail on

`crates/planner/tests/replan_on_standing_world.rs` -- **fails, not skips,
when `map.json` is absent** (unless `CI` is set), because three agents have
reported a green planner suite from worktrees where the sibling test silently
skipped. Symlink the dump into a worktree.

- *the science cell plans again from the world it built*: four cuts of the
  first plan (1/4, 1/2, 3/4, all), replan must plan or find everything
  standing. Green on `71f9227c` and on `a69ae64c` reverted -- **a tick cut
  cannot produce the live shape** (below).
- *a failed take leaves the plate chest a way out*: `take 10 copper-plate
  from the cell` fails, its dependency cone is abandoned
  (`survivors_of_failure`, the executor's rule), everything else stands.
  Asserts the plate chest keeps a free side, and that whatever the replan
  says it does not name all four of that chest's neighbours.

`crates/planner/tests/replan_haul.rs` (peer session, `71f9227c`) now goes
through `world_with` and a fixture the tool writes, instead of its own loader.

## Falsification, both ways

**`a69ae64c` reverted by copy+touch, `--replan 1 --fail "copper-plate from
the cell"`, ~10 s, no game:**

```
a cell already makes copper-plate at [29.5, -46.5] and nothing can carry it to
the supply chest ...: from the iron-chest at [29.5, -46.5]: no belt route,
blocked by 4 tile(s): [28.5, -46.5] [29.5, -48.5] [29.5, -45.5] [30.5, -46.5]
```

Those are `run-1788923927-04849`'s four tiles, byte for byte. Restored, the
chest keeps its exit and the refusal moves (next section). The exit-invariant
test goes red on the same swap.

**`71f9227c`'s `route.rs` swapped for its parent, `replan_haul` through the
shared seam:** both tests red; restored, green. So the refactor kept the
peer's falsification.

## Two findings before the harness was finished

**1. A tick cut is the wrong knife.** At every fraction of the makespan the
replan on the pre-`a69ae64c` planner was green. The live batch was not a
suffix: the failed take's *dependents* were abandoned while every other bot's
work went on, so the supply link's first belt at `[27.5,-43.5]` stood while
its arm did not. `--fail` reproduces that; `--done-by` cannot. The unit of a
truncation is the dependency cone.

**2. The replan of the science bundle on master still refuses after a failed
take, on two items already on record as open** -- and the harness says which:

```
from the iron-chest at [29.5,-46.5]: ... blocked by 4 tile(s): [29.5,-28.5]
[31.5,-30.5] [31.5,-27.5] [32.5,-28.5]        <- the science cell's SUPPLY
                                                 chest, boxed in
from the stone-furnace at [27,-46]: ... blocked by 8 tile(s): ... [27.5,-43.5]
                                              <- the half-built link's belt
```

Not new, not fixed here (`connect`/`sustain` are held by another agent);
now reproducible in ten seconds instead of a run.

**And one the record-driven path found: the recipe decides.** The hand-built
fixture for run 07032 carried entities and bots and no recipes; against it the
replan refuses ("a cell already makes copper-plate..."). The same tick read
off the run's own `samples.jsonl` carries four recipes -- the assembler at
`[31.5,-27.5]` set to `automation-science-pack` among them -- and against that
the replan **plans**, 175 actions, 26,698 ticks. `producing` recognises a
standing science cell by its recipe, and a snapshot without recipes is a
world where the machine stands empty. The fixture is regenerated by the tool
and carries them now.

## Should a fixture be checked in

Yes, when a test or a note relies on it: `workspace/runs/` is not in the
repository, and a number quoted from a run must be reproducible from master.
The rule this repo already has. Size: about 140 bytes per entity as written
(`run-1788926478-07032` at 224 entities is 31 KB); anything under a few
thousand entities is fine as text under `crates/planner/tests/fixtures/`. A
keyframe lists **every** entity in its rectangle -- 1,803 for that run, ore
included -- so the tool filters resources out by prototype; without that the
same snapshot is 233 KB and puts ore the dump already has back on it. A
world-record base is a save, not a snapshot; `--resume-from` is for it.

## What this does not replace

A savepoint-resumed live run. `world_with` is what the *model* held, at a
60-tick beat, inside the keyframe's rectangle; it does not carry what was in
a chest, and the executor's own behaviour (short takes, walks, the mod's
refusals) is not in any offline plan. The record says which kind of question
each tool answers; use the one whose blind spot is not where you are looking.
