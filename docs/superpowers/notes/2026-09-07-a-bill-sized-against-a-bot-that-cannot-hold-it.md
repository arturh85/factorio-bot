# A bill sized against a bot that cannot hold it

2026-09-07. `gathered:crude-oil` composed with `producing:petroleum-gas:45`
refused on master with

```
Error: the goal did not expand: bot 1 owns chain ChainId(451) because its bill
       was sized against it, but `has 5 iron-ore` does not hold there
```

The refusal was right about every fact it stated and wrong about nothing. The
chain **was** bot 1's, its bill **was** sized against bot 1, and bot 1 **did
not** hold the ore. What it could not say is who took it.

## Three premises of the brief, corrected by measurement

- **"Composition is the trigger."** No. `Gathered{crude-oil}` +
  `Have{iron-plate,10}` and `Gathered{crude-oil}` + `Researched{automation}`
  both plan, and so does `gathered:crude-oil` alone. `Produced{petroleum-gas}`
  alone refuses for an honest reason (nothing on the map supplies crude oil).
  The trigger is narrower and is named below.
- **"It is an expansion failure, not a scheduling one."** The message says
  `the goal did not expand` because that is what `app/src-tauri/src/cli/plan.rs`
  wraps `plan_best` in, and `plan_best` is expand *and* schedule. The error is
  raised in `schedule.rs`, from `PlannerError::ChainOwnerInfeasible`. The
  *cause* is in expansion; the detection is not.
- **"A general ownership defect rather than one bad bill."** This one holds,
  and understated it: **three** methods had it, and the run walks through all
  three in order.

## The trigger: a hand-over that is not a convergence

A chain is opened when the goal states a holder (`Holder::Bot` / `Share`) or
when the method `converges` — "two or more produced things must meet in one
inventory" (`Method::converges`, `crates/planner/src/method/mod.rs`).

`run_steps` nevertheless sizes and simulates **every** action against
`ctx.chain_actor`, chain or no chain: `effect.apply(&mut ctx.state, binding)`,
and supplier edges drawn out of `ctx.stock[(binding, item)]`. So an action
expanded outside any chain has a bill in one named bot's name and no owner —
"sizing against a bot that nothing then commits to", the defect
`expand_goal_body` already records twice, one level further out.

`pipe` is **one iron-plate**. So `HandCraft::converges` was honestly `false`,
`Goal::Have{pipe, whose: Anyone}` opened no chain, and the whole sequence

```
mine 9 iron-ore -> fuel furnace -> insert 9 iron-ore -> take 9 iron-plate -> craft 9 pipe
```

was emitted unchained. Measured on the failing plan: `mine 9 iron-ore` and
`mine 6 iron-ore` went to **another bot**, while `insert 9 iron-ore` and
`insert 6 iron-ore` settled on **bot 1** — the only bot already holding ore,
because its own chains had mined some. Fifteen ore left bot 1's hands for a
sequence that had mined its own elsewhere. Chain 451's `insert 5 iron-ore`
then found bot 1 empty, and the plan died naming a chain whose bill had been
correct all along.

`converges` was not the question. **`Method::hands_over`** is: *does this
decomposition put an item into a hand and then take it out of that hand
again?* One produced ingredient is already that; two are also a convergence.
The two are one computation (`HandCraft::short_ingredients`) read at two
thresholds, so the weaker claim cannot drift from the stronger one it
contains. The chain it opens is deliberately **ownerless** — nothing named a
bot for a `Holder::Anyone` goal, so who runs the sequence stays the
scheduler's choice; all that is asserted is that it is *one* bot.

## The same defect, twice more, behind it

Fixing `HandCraft` moved the wall, and the next two walls were the same shape:

- **`Gather`** (the wellhead rig). The pumpjack, the buffer tank and all
  seventeen pipes are `Goal::Have{whose: Holder::Share(ctx.chain_actor)}`
  followed by a placement out of a hand. Nothing converges — each item is its
  own subgoal — so the rig went unchained. `craft 10 pipe` handed bot 2 ten
  pipes; the seventeen unchained `place pipe` actions settled on bot 2 too;
  `craft 1 pumpjack` — owned by bot 2, sized against it, right about what it
  needed — found none. One expansion attempt earlier the same run produced
  **the fluid session's own reported error verbatim**, `has 1
  small-electric-pole`, off the rig's three poles. That report was a second
  instance, not a different bug.
- **`Fabricate`** (the refinery rig). Identical: machine, buffer and pipes
  stated as `Goal::Have{whose}` and placed out of a hand. Its nine unchained
  `place pipe` actions spent the ten pipes bot 1 had crafted for `craft 1
  oil-refinery`.

## Two geometry defects the ownership fix uncovered

Both were behind the ownership wall and could not have been reached before.

- **The buffer stood on the machine it was catching from.** `place
  oil-refinery at [137.5, -352.5]` and `place storage-tank at [137.5,
  -352.5]`, the same tile. Neither is emitted when the other is sited, so
  `free_area_near_where` read the ground under the refinery as clear. The
  machine's footprint is now `taken` alongside the inbound run's tiles.
  Reproduced in `fabricate_fluid_tests`' own fixture, where it had been
  silently true.
- **Siting and routing never agreed on a site.** Excluding the machine's
  ground moved the buffer to the next fitting tile, and the outbound run then
  had to cross the inbound one to reach it — `no route to the storage-tank's
  connection ..., blocked by 4 tile(s)`, on eight of that module's fixtures at
  once. Neither half was wrong; they were two searches. The route is now the
  acceptance test inside `free_area_near_where`, so the first site that *fits
  and routes* wins, which is the nearest such site by construction.

## What moved

One binary, `9bd0c4d`-era master plus this branch, four bots, seed 31337.

| | before | after |
|---|---|---|
| `researched:automation` (`map.json`) | 176 / 21,784 | **176 / 21,784** |
| `producing:automation-science-pack:6` | 316 / 22,457 | **316 / 22,457** |
| `producing:logistic-science-pack:6` | 441 / 47,478 | **441 / 47,478** |
| `gathered:crude-oil` (explored map) | 2,115 / 317,283 | **2,117 / 325,138** |
| the repro | *refused* | **2,295 / 330,406** |

**`gathered:crude-oil` moved, and that is the price of the fix, not a
regression to absorb.** `Gather::hands_over` binds the wellhead rig to one bot
where it used to be spread across four: +2 actions, +7,855 ticks (+2.5%). This
is the cost `expand_goal_body` already priced once for `Holder::Share` — "a
slower correct run beats a faster crashing one" — paid a second time for the
same reason. The plan that got the old number was building the rig with four
pairs of hands and a bill written for one.

## Falsification, and the two experiments that came back green

Seven mutations, each asserted to match **exactly once** before the edit was
written, each run against the whole planner suite, each restored by file copy
and then `touch`ed (`cp -p` preserves mtime, so cargo re-runs the *mutated*
binary against restored source and reports a false red).

| mutation | killed |
|---|---|
| `HandCraft::hands_over` `>= 1` -> `>= 2` | `a_one_ingredient_craft_hands_over_without_converging` |
| driver stops asking `hands_over` | **nothing** -- see below |
| `Gather::hands_over` -> `false` | `the_wellhead_rig_hands_over_without_converging` |
| machine footprint not reserved for the buffer | `the_buffer_is_not_sited_on_the_machine_it_catches_from` |
| `Fabricate::hands_over` -> `false` | `the_fluid_rig_hands_over_without_converging` |
| the route is not the acceptance test | `the_buffer_is_not_sited_on_the_machine_it_catches_from` |
| the machine's other port is not reserved | **nothing** -- see below |

**The driver test was an accidental pass, and this is what caught it.**
`a_one_ingredient_craft_is_expanded_into_a_single_chain` expanded
`have:iron-gear-wheel:1` through the real registry and asserted one chain. It
passed with the `hands_over` clause deleted from the driver, because a
top-level `Holder::Anyone` goal is claimed by **`SplitAcrossBots`**, which
restates it as `Holder::Share(bot)` subgoals -- so `one_inventory` opened that
chain and the test never touched the thing it was named for. The real defect
reached `HandCraft` under `Holder::Anyone` from *inside* `Fabricate`, where
`SplitAcrossBots::claims` is false. Replaced with a stub-method test that
cannot be satisfied by anything else, plus the same stub answering `false` as
the control that shows the assertion can fail; deleting the clause now kills
exactly that test.

**The other-port reservation has no unit test, and a second test written for
it was redundant.** The fluid fixture's geometry does not collide, so the
mutation passes whatever the code does. The disjointness test written to cover
it turned out to be word-for-word what
`a_fluid_ingredient_is_met_by_a_pipe_run_from_a_standing_tank` already asserts
on the same fixture, and was deleted rather than kept.

**So the mutation was run against the CLI repro instead, and it is
load-bearing.** With `&other_port_tiles` replaced by `&[]` -- one match,
asserted -- the repro refuses with `pipe fits at [134.5, -354.5] does not hold`,
the exact tile the two runs had claimed. That is a green *unit* mutation over a
line whose absence is fatal, which is a statement about the fixture rather than
about the code: it is covered by the repro and by nothing in the suite. Said
plainly rather than papered over, and left as it is rather than fitted with a
fixture built to fail -- a test written to match code that already exists is
how `method::connect`'s geometry defect survived four reviews.

**One mutation killed a test another mutation had already killed.** Both the
footprint reservation and the routing acceptance test are load-bearing for
`the_buffer_is_not_sited_on_the_machine_it_catches_from`, which is honest --
the second exists because the first exposed the need for it -- but it means the
routing acceptance has no test of its own either.

## What was NOT fixed, and is left named

**`route_between` pushes its two ends' port tiles into the result outside the
search**, so no obstacle grid can keep two runs apart at their ports —
`reserved` is honoured for every routed tile and silently not for these. The
observed collision (both refinery runs claiming `[134.5, -354.5]`) is fixed at
the caller, by reserving the machine's *other* port before routing the first
run, and that is measured. The hole in `route_between` itself is not, so
nothing was changed there: a fix with no failing case behind it would be
exactly the kind of unmeasured change this repo keeps paying for.

Two smaller ones from the same reading, also unmeasured and untouched:
`free_area_near_where`'s `accept` is `Fn`, so the winning site is routed twice;
and `taken` is built with `filter_map(collision_area)`, which silently drops a
prototype the world has no box for.
