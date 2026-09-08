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
