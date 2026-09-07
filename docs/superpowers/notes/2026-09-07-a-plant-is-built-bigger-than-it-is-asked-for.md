# A plant is built bigger than it is asked for

*2026-09-07. Owner ruling, landed in `method::power::plant_size_for`.*

> *"We probably don't just want to build what we need in terms of power, but
> maybe at least 50% more, so that we can build stuff and not immediately lose
> power."*

## Why a margin at all

An under-supplied Factorio network does not degrade into "slow" — it reads as
**completely dead**. This repo already records that from the other side twice
(*"power coverage is not power capacity"*, and `PowerPlantTooSmall`'s own
"everything places, everything is wired, and the network browns out"). A plant
sized exactly to a block's draw is one inserter — 13 kW — away from taking the
whole network down, and the symptom is indistinguishable from a wiring fault.

## Rounding up to whole engines was NOT already doing this

The obvious objection is that `plant_size_for` already takes a ceiling over
900 kW engines, so most demands get slack for free. **Most do; the ones that do
not are the ones that matter.** Measured off the fixture, before the change:

| demand | engines | capacity | ratio |
|---|---|---|---|
| 60 kW (a lab) | 1 | 900 | 15.0x |
| 78 kW (the electric block) | 1 | 900 | 11.5x |
| **900 kW** | **1** | **900** | **1.00x** |
| **1,800 kW** | **2** | **1,800** | **1.00x** |
| 4,320 kW (24 electric furnaces) | 5 | 4,500 | 1.04x |

At every exact multiple of an engine the free margin is **zero**, and just above
one it is a rounding accident nobody chose. So "check whether the existing
arithmetic already has slack" answers **no**, and it fails exactly where the
ruling is aimed: the electric-smelter demands, where 4,320 kW got 4%.

## What changed

One constant, `PLANT_HEADROOM = 1.5`, used in one place —
`plant_size_for` sizes the engine row against `kw * PLANT_HEADROOM` instead of
`kw`. Everything else derives: `engines_for`, `plan_plant_for`,
`complete_plant`'s growth of a standing plant, and the plant's coal bill all go
through it.

**Where it deliberately does not apply:** `supply_for`'s adoption tiers and
`Condition::Powered`'s headroom test both go on asking for the true `kw`. Two
reasons, either sufficient:

* A plant that already stands with exactly enough headroom is still worth
  adopting; demanding 1.5x to adopt would build a *second* plant beside a
  working one, which is the cost `PLANT_ADOPT_RADIUS` exists to avoid.
* If the **test** asked for 1.5x too, the margin would be spent as fast as it
  was bought and the plant would grow without bound. The check must measure the
  load, not the allowance.

## Two consequences, both stated rather than hidden

**1. The largest demand this planner serves drops from 36 MW to 24 MW.** The
ceiling is the offshore pump's water (`BOILERS_PER_PUMP`), and it is unchanged
— but a plant is now built to 1.5x, so the largest *demand* that fits is
36,000 / 1.5. 24 MW is still 133 electric furnaces; the number is written as
`WHOLE_PLANT_KW_LITERAL / PLANT_HEADROOM` in the tests rather than as a literal,
so it moves with the constant.

**2. `PowerPlantTooSmall` carries two numbers now.** A demand can be refused
that would have fitted unmargined, and a message quoting only the sized figure
would report a number the caller never supplied — the confusion this project
keeps fixing elsewhere. So `needed_kw` is **what was asked for** and `sized_kw`
is **what the margin made this planner try to build**:

```
that needs 24000.5 kW -- sized with headroom to 36000.75 kW -- and the largest
plant this planner lays out generates 36000 kW
```

## What moved, and what did not

**No offline baseline moved.** Four goals, same release binary, before and
after, byte-identical:

| goal | world | actions / ticks |
|---|---|---|
| `researched:automation` | `map.json` | 176 / 21,784 |
| `producing:automation-science-pack:6` | `map.json` | 316 / 22,457 |
| `producing:logistic-science-pack:6` | `map.json` | 441 / 47,478 |
| `gathered:crude-oil` | `map-31337-explored.json` | 2,115 / 317,283 |

That is expected rather than lucky: every plant those four plans build is for a
lab (60 kW) or a red cell (189 kW), and both were one engine before the margin
and are one engine after it. **The margin only bites where demand approaches
plant capacity**, which is the electric-furnace case and nothing in today's
baselines.

Four existing tests moved, each for a reason the change makes true:

* `a_demand_is_sized_into_engines_and_a_bigger_one_refuses_by_name` — the
  boundary is now two thirds of an engine (600 kW), not one.
* `a_demand_one_plant_carries_is_still_built_rather_than_refused` — 1,500 kW is
  three engines, not two.
* `every_engine_of_a_grown_plant_is_covered_and_the_poles_are_one_network` and
  `every_boiler_in_the_chain_is_fuelled_and_the_bill_pays_for_it` — their
  largest case was 36 MW, which is now past the servable ceiling.
* `a_full_network_is_not_finished_but_the_half_built_plant_beside_it_is` needed
  its fixture rebalanced, and **the reason is worth recording**: it asked for
  900 kW against a network with 840 kW free. With the margin there is no demand
  that both exceeds 840 and still fits one engine, so the standing consumer had
  to grow with it — a beacon's 480 kW leaves 420 free, and 450 kW is above that
  while 450 x 1.5 fits one engine. The test's *subject* (a complete plant is not
  something to finish; the half-built one beside it is) is unchanged.

## Falsification

Three mutations, each substitution asserted to match exactly once, each restored
with `touch` rather than a copy that preserves mtime:

| mutation | tests killed |
|---|---|
| `PLANT_HEADROOM: 1.5 -> 1.0` | `a_demand_is_sized_into_engines...`, `a_demand_one_plant_carries...` |
| `let sized = kw * PLANT_HEADROOM` -> `let sized = kw` | those two **and** `every_demand_the_planner_serves_gets_at_least_the_headroom_margin` |
| `sized_kw: sized` -> `sized_kw: kw` in the refusal | `a_demand_is_sized_into_engines...` only |

**The first one is worth saying out loud: it does NOT kill the new test.** That
test derives its expected engine count *from* `PLANT_HEADROOM`, so it moves with
the constant and cannot see it change — by design. The constant's concrete
consequences are pinned by the older test, which the mutation does kill; what
the new test adds is the *property* across sixteen demands up to the ceiling,
including that each was actually **served** (a "the ratio is at least 1.5"
assertion passes for a function that refuses everything, so the same loop
asserts the exact engine and boiler counts as well).

## This is a second question about the same promise

It arrived alongside the diagnosis in
`2026-09-07-the-stamp-mines-the-pole-run.md`, and the two are separable on
purpose: that note is about whether `ensure_powered`'s promise is *delivered*,
this one is about *how much* it should promise. A bigger plant does not help a
block whose pole run was mined out from under it.
