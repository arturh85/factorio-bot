# Leaving room for the beacons

2026-09-07. Owner's ask: *"Maybe leave space for beacons from the beginning so
we can cheaply improve the production rates/productivity later."*

Cheap-now, expensive-later. Empty ground costs nothing today; a beacon lane
retrofitted into a built and belted base is a teardown, and this project has
already established that smelters force one teardown while belts upgrade in
place. The point is to avoid a second.

## 1. Which beacon fields we have, and which we do not

**We have the beacon's footprint and nothing else.** Every field on a beacon
that reaches `FactorioEntityPrototype` (`crates/core/tests/entity-prototype-fixtures.json`,
the live 2.1.17 capture):

```
name  entity_type  collision_mask  collision_box  mine_result  mining_time
mining_speed=null  crafting_speed=null  max_underground_distance=null
fluidbox_prototypes=null
```

`collision_box` is `+/-1.19921875` — the 3x3 footprint, which is real data we
can derive from.

Absent, by exact name, with the runtime accessor the mod would use:

| what it decides | prototype field | runtime accessor | present? |
|---|---|---|---|
| how far a beacon reaches | `supply_area_distance` | `LuaEntityPrototype::get_supply_area_distance()` — a **method** in 2.0, not an attribute | **no** |
| what a shared module effect is worth | `distribution_effectivity` | `LuaEntityPrototype.distribution_effectivity` | **no** |
| the n-th beacon's diminishing return | `profile` | `LuaEntityPrototype.profile` | **no** |
| how many modules a beacon holds | `module_slots` | `LuaEntityPrototype.module_inventory_size` | **no** |
| what a module actually does | (item side) | `LuaItemPrototype.module_effects` | **no** |

`FactorioItemPrototype` carries `name, item_type, stack_size, fuel_value,
place_result, group, subgroup` — a `speed-module` and a `productivity-module`
are indistinguishable in our data.

This is the same gap `crates/planner/src/state.rs`'s `pole_supply_half_extent`
already documents ("the mod does not send `supply_area_distance`"). The mod
requirement was handed to the agent owning `mods/BotBridge/` and is not
repeated here.

### A trap in the two conventions

**A pole's `supply_area_distance` and a beacon's are different geometry**, and
reading one off the other gets a lane wrong by three tiles. The prototype docs
say a pole's "corresponds to *half* of the supply area" — 2.5 gives 5x5 on a
1x1 pole, i.e. `2d`. A beacon's is "the maximum distance that this beacon can
supply its neighbors" — 3 on a 3x3 footprint gives the 9x9 the game shows,
i.e. `footprint + 2d`, **not** `2d`.

## 2. The thing nobody had checked: our machines refuse beacon effects

Read out of `workspace/server/data/base/prototypes/entity/entities.lua`, each
line inside its own entity's block:

| entity | line | `effect_receiver` |
|---|---|---|
| `stone-furnace` (1024) | 1040 | `uses_module_effects = false, uses_beacon_effects = false` |
| `assembling-machine-1` (3161) | 3197 | `uses_module_effects = false, uses_beacon_effects = false` |
| `steel-furnace` (4743) | 4757 | `uses_module_effects = false, uses_beacon_effects = false` |
| `electric-furnace` (4303) | — | defaults (`uses_beacon_effects` defaults **true**), `module_slots = 2` |
| `assembling-machine-2` (3209) | — | defaults, `module_slots = 2` |

**Every machine this project currently builds is on the refusing list.**
`method::smelt` places a `stone-furnace`; `method::assemble::MACHINE` is
`assembling-machine-1`. A beacon standing next to either does *nothing at all*
— not "a bit less", nothing.

So a beacon is worth ground only once the cell's machine becomes
`assembling-machine-2` or a furnace becomes `electric-furnace`, which is
exactly the milestone the owner already named ("electric smelters are the
milestone"). That makes the reservation *more* worth making, not less: those
two machines are `fast_replaceable_group` upgrades in place, so a cell built
today with the lane already clear upgrades without a teardown, which is the
whole ask.

## 3. The spacing rule, derived (`crates/planner/src/method/util.rs`)

`BeaconGeometry`, with `b` the footprint and `d` the supply area distance:

```
     row A          lane           row B
...####|<-- gA -->|#####|<-- gB -->|####...
                    b
```

The supply area is the footprint grown by `d` on every side, so **row A is
reached iff `gA < d`**, and a beacon earns its ground only when it reaches
both rows. Centring it makes the gaps equal, giving

* `lane_tiles() = b` — the narrowest lane that can ever work;
* `max_row_separation_tiles() = b + 2d` — the widest two rows may sit apart;
* `reaches_gap(g) = g < d`.

**The reservation decision does not need `d`.** Set `gA = gB = 0` — machines
flush against the beacon — and the reach condition is `0 < d`, true of every
beacon that supplies anything. So the cheapest lane is exactly `b`, `b` comes
from `collision_box`, which we have, and `d` bounds only how much *wider* a
lane may usefully be. That is why this is a landable deliverable rather than a
blocked one.

Everything that needs `d` returns `None`. The single seam is
`beacon_supply_area_distance()`, which returns `None` for every world today
and is one function body to change when the field lands. It is deliberately
**not** a table of vanilla names in the manner of `pole_supply_half_extent`:
that table is honest about being a stopgap and is still a mod-compat defect,
and one is enough.

Nothing here models what a beacon is *worth* — that needs
`distribution_effectivity` and `profile`, and inventing them from a remembered
table is the defect this whole approach exists to avoid. `rates.rs` still
lists `beacon-distribution` as disclosed-but-not-re-costed, correctly.

## 4. The ground reserved (`crates/planner/src/method/assemble.rs`)

Three columns immediately east of the cell's machine column, spanning the
seven rows the two 3x3 machines occupy — 21 tiles, checked in `fit()`.

**East, and one column, both forced.** The two machines sit four tiles apart
and that distance is not a choice: `Role::LinkInserter` at `(0, 2)` carries
the intermediate's output into the product machine and an inserter reaches
exactly one tile, so a lane *between* them would break the one link the cell
is built around. Both machines therefore present their east faces on the same
line, and one beacon column reaches both — the "does it reach both rows"
question does not even arise. West is taken by the chests (`x = -3`) and by
`LANE`, the ground a bot stands on to fill them (`x = -4`).

Width is `BeaconGeometry::lane_tiles()`, so a world with no beacon prototype
reserves **nothing** and a mod with a 5x5 beacon gets five columns.

Checked in `fit()` and deliberately **not** in `fit_partial()`: a cell already
standing was sited before this rule existed, and refusing to finish it would
strand a half-built cell to protect ground that is already gone.

## 5. What it cost

Same binary before and after, seed-31337 t=0 dump, `--bots 1,2,3,4`:

| goal | before | after |
|---|---|---|
| `researched:automation` | 176 / 21,784 | 176 / 21,784 (unchanged — no cell) |
| `producing:automation-science-pack:6` | 316 / 22,463 | 316 / **22,457** |
| `producing:logistic-science-pack:6` | 442 / 47,542 | **441** / **47,478** |

**Two baselines moved, and both got slightly shorter.** The cell re-sites to
satisfy the reservation and the new site happens to be marginally cheaper.
That is luck, not a benefit — the honest claim is *nothing refused*, and a
different map could move them the other way by the same mechanism.

Sweeping the reserved width with a temporary env override (removed before
commit; the shipped width is derived):

| lane width | red pack | green pack |
|---|---|---|
| 0 | 316 / 22,463 | 442 / 47,542 |
| 1–2 | 316 / 22,457 | 441 / 48,168 |
| 3 (derived) | 316 / 22,457 | 441 / 47,478 |
| 4, 5, 6, 8, 10, 12 | 316 / 22,457 | 441 / 47,478 |

**Nothing refuses anywhere up to twelve tiles**, and the numbers are flat from
three onward. So the lane is free at any plausible `supply_area_distance`,
which is the answer that survives the field being absent.

**What this measurement cannot see**, and it is the case that matters: a
*fresh* map is nearly all open ground. Every refusal this project has actually
suffered came on a **replan into a partly-built base**, and a t=0 dump is
blind to that by construction. Read the sweep as "the reservation is not
intrinsically expensive", never as "siting will never refuse".

## 6. What is still open

* the five absent prototype fields, above — handed to the mod owner;
* pricing a beacon (needs `distribution_effectivity` and `profile`);
* `method::smelt`'s furnace line reserves nothing — a `stone-furnace` cannot
  use a beacon at all, and the electric-furnace line that could does not exist
  yet. The block blueprints in `scripts/rcontest.lua` were a peer session's.
