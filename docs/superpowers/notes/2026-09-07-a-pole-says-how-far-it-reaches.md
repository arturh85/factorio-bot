# A pole says how far it reaches

2026-09-07. `crates/planner/src/state.rs`, `crates/planner/src/method/util.rs`.

`FactorioEntityPrototype::supply_area_distance`,
`distribution_effectivity` and `beacon_profile` landed on 2026-09-06 with
nothing reading them, deliberately: *"consuming the datum moves plans,
carrying it does not."* This is the consumption.

Two readers now exist. `method::util::beacon_supply_area_distance` — the
seam that returned a hard-coded `None` — reads the field. And
`state::pole_supply_half_extent`, which was a hand-kept table of four vanilla
pole names, derives from the same field.

## The hand-kept table was right, all four numbers

The deliverable was the comparison, not the change. Base 2.1.17's own
`workspace/server/data/base/prototypes/entity/entities.lua`, against the
table that had never been checked against it:

| prototype | table said | base 2.1.17 says | |
|---|---:|---:|---|
| `small-electric-pole` | 2.5 | 2.5 | agrees |
| `medium-electric-pole` | 3.5 | 3.5 | agrees |
| `big-electric-pole` | 2.0 | 2 | agrees |
| `substation` | 9.0 | 9 | agrees |
| `beacon` | (not in the table) | 3 | — |

So this validates a table nobody had checked rather than finding an error in
it, and **every offline baseline is unmoved**, measured on one binary before
and after (`151e6d00`, `map.json` = seed-31337 t=0, `--bots 1,2,3,4`):

| goal | before | after |
|---|---|---|
| `researched:automation` | 176 / 21,784 | 176 / 21,784 |
| `producing:automation-science-pack:6` | 316 / 22,457 | 316 / 22,457 |
| `producing:logistic-science-pack:6` | 441 / 47,478 | 441 / 47,478 |
| `gathered:crude-oil` (explored map) | 2,115 / 317,283 | 2,115 / 317,283 |

**But the wire table beside it is wrong.** `pole_wire_reach` is a second
hand-kept table, of `maximum_wire_distance`, and it says
`big-electric-pole => 30.0` where base 2.1.17 says **32**. Factorio 2.0 moved
it. That one cannot be derived — the mod does not send
`maximum_wire_distance` and `FactorioEntityPrototype` has no field for it —
so it is reported here rather than quietly corrected, and it is the answer to
"is a hand-kept table of game data a real defect": **two tables, one of them
already drifted.**

## Deleting the fallback refuses every plan we have

The obvious reading of "derive it from data" is to delete the table. Measured,
on a release build with the fallback removed and nothing else changed:

```
researched:automation              Error: the goal did not expand: the nearest
producing:logistic-science-pack:6         water is 48.1 tiles away, but no
                                          shoreline within 10 tiles of it has
                                          room for a pump, a boiler, a steam
                                          engine and the pipes between them
gathered:crude-oil                 Error: a power plant needs water, and ...
```

Not a moved baseline — **no plan at all**, on all three goals. Every archived
world predates the field, `workspace/scripts/map.json` included, so every pole
reads as supplying nothing, `pole_would_supply` is false everywhere, and
`ensure_powered` can find no shoreline arrangement that works. Note that the
refusal blames *the water*: an unrelated message, one layer downstream, with
no mention of poles. A blind planner does not announce that it is blind.

So the table survives, renamed `vanilla_pole_supply_half_extent`, as a
compatibility shim for pre-2026-09-06 senders and documented as one. A world
that declares the field overrides it before it is ever consulted; a pole name
it does not know still answers `None`. The precedent is
`VANILLA_CHARACTER_RESOURCE_CATEGORIES` — the game's own shipped value, not a
guess, standing in for a sender that said nothing.

## The two conventions are different and share no helper

The same prototype field answers for `ElectricPole` and for `Beacon`, and it
means different things:

* a **pole**'s is half the side of its supply square — 2.5 is a 5x5;
* a **beacon**'s is distance *beyond its own footprint* — 3 on a 3x3 is 9x9,
  i.e. `b + 2d` and not `2d`.

Both readers gate on `entity_type` before believing the number, from opposite
sides. Without the pole-side gate a beacon standing in a plan reads as
supplying 6x6 of electricity it was never wired to carry; without the
beacon-side gate a small pole's 2.5 becomes a `b + 2d` that
`BeaconGeometry` has no business computing. Each gate has its own test and
each test was falsified by removing exactly that gate.

## `beacon_profile` is an array, and the geometry is all that is wired

`distribution_effectivity` and `beacon_profile` are read by nothing here, and
that is a stopping point rather than an omission. A receiver's share is
`distribution_effectivity * beacon_profile[n]` where **n is how many beacons
reach that receiver** — vanilla's array is 100 entries beginning
`1, 0.7071, 0.5773, 0.5` (it is `1/sqrt(n)`), so the second beacon on a
machine is worth 71% of what the first was. Anything treating
`distribution_effectivity` as the whole answer is right for exactly `n = 1`
and silently wrong for every other count. Pricing a beacon needs a caller
that knows `n`, which is a siting decision nothing in the planner makes yet.
The reach is wired; the worth is not.

## How an absent field reads

`None`, everywhere, and `None` means *the sender did not say* — never zero
reach, and never a default. The two sides differ only in what they do next,
and the difference is argued rather than assumed: the pole side falls back
because deleting the table refuses every plan (above), and the beacon side
does **not**, because nothing consumes a beacon's `d` yet, so honesty there
is free and is taken.

## Falsification

Seven new tests, seven mutations, one at a time, each mutation asserting its
own substitution count is exactly 1 *in the breaking edit* — the rule from
`2026-09-06-fixtures-agree-with-their-code.md`. All seven killed; the harness
also asserts the target test actually ran under the mutation, which is the
half a peer session found unguarded.
