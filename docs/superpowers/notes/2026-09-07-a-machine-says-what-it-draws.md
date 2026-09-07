# A machine says what it draws

**2026-09-07.** `crates/planner/src/state.rs` held every electrical number this
project owns as hand-typed Rust: `consumer_kw` with 15 rows and `generation_kw`
with 2. Nothing electrical had ever crossed the bridge from the game —
`FactorioEntityPrototype` had 17 fields and not one of them was about energy,
and the only `energy` the mod sent anywhere was a recipe's crafting time. Both
tables now read the prototype and keep their old numbers only as a documented
fallback, exactly as `pole_supply_half_extent` did on 2026-09-06.

## The shape of each field, checked and not recalled

Read out of this install's `workspace/factorio-api-docs/runtime-api.json`
(`application_version` 2.1.17), because getting this wrong is silent: an
attribute read against a method **raises, `pcall` swallows it, and the field
arrives nil with nothing saying it should not have**. That is how
`crafting_speed` was nil for all 1,028 prototypes of a live game.

| what | shape in 2.1.17 | notes |
|---|---|---|
| `energy_usage` | **attribute**, `double`, optional | absent on an inserter |
| `get_max_energy_production(quality?)` | **method**, `double`, not optional | answers `0.0` for a non-generator |

They are opposite shapes, and the pair `get_supply_area_distance` (method) /
`mining_speed` (attribute) already in this file shows the coin lands both ways.

**There is no `max_energy_production` attribute and no `fluid_usage_per_tick`
on `LuaEntityPrototype` at all**, so a generator's output cannot be reassembled
downstream from its physics: a `steam-engine`'s 900 kW is
`fluid_usage_per_tick * 60 * heat_capacity * (maximum_temperature -
default_temperature) * effectivity`, of which the runtime exposes
`maximum_temperature` and `effectivity` and not the fluid usage. The runtime
does the arithmetic and hands over the answer; the method is the only route.

## The unit, derived and tested

Both are **joules per tick**. Watts are per second and a Factorio second is 60
ticks, so

```
kW = J/tick * 60 / 1000
```

`FactorioEntityPrototype::energy_usage_kw` and `max_energy_production_kw` are
the only two places that scale, and `TICKS_PER_SECOND` is written once. It is
deliberately *not* scaled by `game.speed`: a prototype's draw per tick does not
change when the game runs faster, only how many ticks pass per wall second.

**Tested against machines whose draw is independently known**, which is what
makes the factor a measurement rather than an assumption: the live game reports
`electric-furnace` = 3,000 J/tick and `steam-engine` = 15,000 J/tick, and 180 kW
and 900 kW are the numbers on their tooltips and in the table this change
replaces. A missing or doubled 60 moves both by 60x.

## Table versus prototype: the deliverable

Measured end to end — mod → RCON → serde → world model — by a `world.dump` off
a live headless seed-31337 game on an isolated instance, then compared offline.
1,028 prototypes; **28 carry `electric_energy_usage`, all 1,028 carry
`max_energy_production`** (it is not optional and reads `0.0` for anything that
is not a generator, which is why the `> 0` filter and the `entity_type` gate
are both load-bearing).

### consumers

| prototype | table kW | game kW | |
|---|---:|---:|---|
| `assembling-machine-1` | 75 | 75 | match |
| `assembling-machine-2` | 150 | 150 | match |
| `assembling-machine-3` | 375 | 375 | match |
| `electric-mining-drill` | 90 | 90 | match |
| `pumpjack` | 90 | 90 | match |
| `lab` | 60 | 60 | match |
| `electric-furnace` | 180 | 180 | match |
| `chemical-plant` | 210 | 210 | match |
| `oil-refinery` | 420 | 420 | match |
| `radar` | 300 | 300 | match |
| `beacon` | 480 | 480 | match |
| `inserter`, `long-handed-inserter` | 13.0 | *field absent* | duty cycle |
| `fast-inserter` | 18.2 | *field absent* | duty cycle |
| `bulk-inserter` | 52.0 | *field absent* | duty cycle |

**All 11 checkable rows match exactly.** So the maintenance half of this change
found no bug, and CLAUDE.md's milestone arithmetic — 24 electric furnaces at
180 kW against a 1.8 MW plant, the whole argument for a second boiler — **stands
as written**. This is a mod-compatibility fix, not a correctness one, and the
comparison is the deliverable whether or not anything moved. It was worth doing
because a hand-kept table is only ever read by code that agrees with it: the
neighbouring `pole_wire_reach` had drifted 30 against the game's 32 and nobody
noticed for a Factorio major version.

### generators

| prototype | table kW | game kW | |
|---|---:|---:|---|
| `steam-engine` | 900 | 900 | match |
| `steam-turbine` | 5,800 | **5,820** | **+0.34%** |

The one real discrepancy, in an entity nothing here builds yet.
`vanilla_generation_kw` keeps the 5,800 deliberately — its job is to say what a
pre-field world *would have* answered — and any world recorded from now on gets
5,820 from the game.

## An inserter reports nothing, not zero — the brief and I both had this wrong

Going in, both the field's doc and the planner's said an inserter is electric
and "reports 0", so a zero had to be read as "the prototype does not answer".
**Measured: 28 prototypes carry the field and not one reports 0.** An
`inserter` has an electric energy source and passes the mod's gate, but
`energy_usage` is an *optional* attribute and is simply absent on it — its cost
is `energy_per_movement` and `energy_per_rotation`, per swing. So the four
inserter rows are reached through the ordinary absent-field fallback and need no
exception at all. The `> 0` filter is kept as a guard against a modded
prototype that states a standing draw of nothing, and the test asserts both
cases, because a reader who saw only the guard would conclude the wrong thing
about vanilla.

## What the tables never knew, which is the correctness half

`consumer_kw` is the one table in `state.rs` whose unknown name errs **towards
permitting**: a machine it does not carry draws nothing, so an unmodelled
consumer on the network is headroom that is not there. Its own doc has always
said so. The live game names **17 electric consumers it never carried**, and
some are not hypothetical:

| prototype | kW | |
|---|---:|---|
| `foundry` | 2,500 | |
| `electromagnetic-plant` | 2,000 | |
| `cryogenic-plant` | 1,500 | |
| `crusher` | 540 | |
| `centrifuge` | 350 | |
| `biolab`, `big-mining-drill` | 300 | |
| `rocket-silo` | 250 | |
| `recycler` | 180 | |
| `agricultural-tower` | 100 | |
| `roboport` | 50 | |
| `pump` | 29 | |
| **`small-lamp`** | **5** | the `FurnaceLine` fixture stands **three** and budgeted zero for all three |
| `programmable-speaker` | 2 | |
| three combinators | 1 each | |

Nobody has to have thought of any of them. That is the whole argument for
deriving rather than tabulating.

## Two gates, and each one is the safety of its field

**The mod sends `energy_usage` only for a prototype with an electric energy
source**, and the field is named `electric_energy_usage` so the name states the
gate. A `stone-furnace`'s `energy_usage` is 90 kW *of coal*; charged against an
electric budget it is a number in the wrong units that every test would agree
with. `state.rs` has always left burner machines out of `consumer_kw` rather
than zeroing them, for exactly this reason, and the gate now lives upstream
where the energy source is visible. Confirmed live: `stone-furnace`,
`steel-furnace`, `burner-mining-drill`, `boiler`, `offshore-pump` and
`burner-inserter` all carry no `electric_energy_usage`.

**`generation_kw` is gated on `entity_type`** —
`generator`, `burner-generator`, `fusion-generator` — and that gate is a
*determinism* gate, not a completeness one. It is what stopped this change
switching solar on by accident: `get_max_energy_production()` answers **60 kW
for a `solar-panel`** and **300 kW for an `accumulator`**, and crediting either
makes the same plan feasible or not according to what time of day the run
started. `solar-panel` was absent from the old table by omission, and omission
is not a gate. Everything the planner will now credit as generation, from the
live game:

| prototype | type | kW |
|---|---|---:|
| `steam-engine` | generator | 900 |
| `steam-turbine` | generator | 5,820 |
| `burner-generator` | burner-generator | 1,000 |
| `fusion-generator` | fusion-generator | 50,000 |

## The fallback stays, and the counterfactual is measured

Deleting the pole fallback on 2026-09-06, with nothing else changed, made **all
three offline goals refuse to expand at all** — and the refusal blamed the
water, one layer downstream, with no mention of poles. Every archived world here
predates every electrical field, `workspace/scripts/map.json` included, so the
same deletion here would blind the demand ledger the same way. Two of the nine
falsification mutations below demonstrate it: removing either fallback turns
**54 and 165 tests red respectively**, across power, blueprints, research and
extraction.

## Baselines: nothing moved, and that was checked twice

Same release binary, `plan --bots 1,2,3,4`, this branch:

| goal | actions | ticks |
|---|---:|---:|
| `researched:automation` | 176 | 21,784 |
| `producing:automation-science-pack:6` | 316 | 22,457 |
| `producing:logistic-science-pack:6` | 441 | 47,478 |
| `gathered:crude-oil` (on `map-31337-explored.json`) | 2,115 | 317,283 |

All four are identical to master's. That was expected — every archived dump
predates the fields, so offline planning runs the fallback path, which
reproduces the old tables exactly.

**Which means the offline basis cannot exercise the new path, so it was
exercised deliberately**: the live probe's prototypes were injected into
`map.json` and the three `map.json` goals re-planned against a world that *does*
carry the fields. **176 / 21,784, 316 / 22,457, 441 / 47,478 — identical
again.** So the derived values agree with the table on everything these goals
touch, and the change is behaviour-preserving on the whole measured basis while
being correct-by-derivation for everything beyond it.

## Solar: a stop, not a result

The owner's ruling of 25 panels to 21 accumulators is itself the statement that
the average is **0.84** of the panels' promise, and the instruction was to
derive that ratio rather than write `0.7` or `42 kW` anywhere. **It is not
derivable from prototype data, so this stops here**, the same answer the beacon
work correctly gave.

What the prototypes carry is the **noon** figure and nothing else:
`solar-panel` = 60 kW, `accumulator` = 300 kW of discharge limit. The day/night
curve that turns noon into an average lives on **`LuaSurface`** —
`ticks_per_day`, `dawn`, `dusk`, `evening`, `morning`, `solar_power_multiplier`
— and the accumulator sizing additionally needs `buffer_capacity`, which is on
`LuaElectricEnergySourcePrototype`, a sub-prototype nothing sends.
`solar_panel_performance_at_day` and `..._at_night` are on the entity prototype
but give only the endpoints of the curve, never the fraction of the day spent at
each.

So the follow-up is not a bigger prototype: **it is surface state, and
`FactorioEntityPrototype` is the wrong home for it.** Until it exists, solar
reads as unpowered and a solar base is falsely refused — the safe direction, and
`state.rs` says so in place.

## Verification

- `nix develop -c cargo test --workspace` — 104 `ok` results, exit 0, redirected
  to a file with the exit code taken from cargo rather than from a pipe.
- `cargo clippy --workspace --all-features --all-targets -- --deny warnings` —
  exit 0.
- `cd app && pnpm lint` **and** `pnpm run test:coverage` — 53 files, 942 tests,
  both green. The OpenAPI snapshot was regenerated twice, once after the doc
  comments were rewritten.
- **Nine falsification mutations**, one at a time, each asserting its
  substitution matched **exactly once in the breaking edit** and re-run after
  `rustfmt` had touched the files (`scratch/falsify.py`). All nine red, each
  with the intended test among the failures.
- **No doc-comment theft**: the snapshot diff is purely additive and both new
  fields carry their own `description`, with `entity_type` and
  `max_underground_distance` either side unchanged.

## Follow-ups this leaves named

- `pole_wire_reach` and `delivery_offset` are the two hand-kept tables left,
  waiting on `maximum_wire_distance` and `vector_to_place_result`.
- The four inserter rows would fall to `energy_per_movement` and
  `energy_per_rotation`, both attributes on `LuaEntityPrototype`. That deletes
  the last non-prototype number in `consumer_kw` — though **13 kW is a duty
  cycle, a choice about how busy an inserter is**, and the per-swing energies
  give the scale factor, not the cycle.
- Solar needs a surface-state channel, per above.
