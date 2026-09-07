# A well knows its own yield — and the mod sends half of what that takes

2026-09-07. Session `well-yield`, branch `a-well-knows-its-own-yield`, off
`ee28485e`. Oracle: the 6:39:53 Space Age rocket-launch save, dumped as
`workspace/wrload/scripts/wr-census-status.json` (2.94 GB, `game.tick`
1,443,169).

Predecessor: `2026-09-07-a-fluid-that-never-reaches-its-machine.md`, which
connected the oil chain, watched the mean absolute log error go 0.141 → 1.554,
and handed over three things by name. Two of them are done here.

**Headline: 1.554 → 1.066, and every digit of that move is one missing factor
in a mining drill's rate.** `mining_speed / mining_time` counts mining
*operations*; this file emitted it as items per second. A solid ore yields one
item per operation, so the error was invisible for as long as the flow walk
could not reach an oil chain. `crude-oil` yields **ten**.

**And the next binding number is named and measured: water.** With crude oil
corrected, the model's water supply is 600/min against 68,250/min that machines
the game has configured are eating — a 114× gap, against crude oil's old 10×.

---

## 1. A pumpjack's rate, and where each factor comes from

```
items/second  =  mining_speed / mining_time  ×  products-per-operation  ×  yield
                 └── prototypes ──────────┘     └── prototypes ──────┘     └─ NOT
                                                                              ON
                                                                              THE
                                                                              WIRE
```

**The factor that was missing and is now read: `mine_result`.** The mod's
`products_to_dict` flattens a resource's `minable.results` into
`FactorioEntityPrototype::mine_result`, a `BTreeMap<String, u32>`, and it has
always been on the wire. Read off the dump itself:

| resource | `mine_result` | `mining_time` |
|---|---|---|
| `iron-ore`, `copper-ore`, `coal`, `stone` | 1 | 1.0 |
| **`crude-oil`** | **10** | 1.0 |

`base/prototypes/entity/resources.lua` agrees: `crude-oil`'s `minable` names
one product of `amount = 10`, and `pumpjack` is `mining_speed = 1`. So the old
code said one crude oil per second where the game's own data says ten.

`FlowGraph::mining_yield_per_operation` reads it, defaulting to 1 for a
resource that reports nothing — which is what this file assumed
unconditionally before, so a fixture or an old dump keeps the answer it had.
**Every solid-ore row in the table below is unchanged, by construction.**

## 2. The yield IS on the wire. The constant it is divided by is not.

`crude-oil` is an **infinite** resource, and Factorio scales an infinite
resource's yield by `amount / normal_resource_amount` — the percentage the game
shows on the well.

**The numerator is present, and the probe
`what_every_pumpjack_is_standing_on` proves it on real data**: the mod has
always sent `entity.amount` for a `type == "resource"` entity,
`EntityGraph::resources` keeps it per tile, and `resource_amount` reads it back.

```
24 pumpjacks, 24 standing on a well whose amount the mod reported
amounts from 1,144,230 to 5,517,850, mean 2,650,000
mean yield if normal is 300000: 8.83x
```

(The predecessor said 23 pumpjacks; measured here, it is **24**.)

**The denominator is absent.** `normal_resource_amount`, `infinite_resource`
and `minimum_resource_amount` are all *attributes* of `LuaEntityPrototype` on
the 2.1.17 runtime API, and `mods/BotBridge/types.lua` sends none of them. So
nothing in the model can tell an infinite resource from a finite one, let alone
divide by the right number. Writing `300_000` into `update` is exactly the
mod-compatibility defect this project rules against, so **the yield is handed
over, not guessed** (§6), and `mining_yield_per_operation` reports the
100%-yield rate with the gap written into its own doc.

The constant appears once in this repository, in the probe that prints, where
it is labelled as coming from the data files.

**What this means for the table: the remaining pumpjack error is a floor with a
known direction.** 8.83× more crude oil would not move a row, because 13,800/min
already exceeds the 13,333/min the base's refineries can eat — once supply
clears demand, rationing stops and the size of the surplus stops mattering.
**The oracle cannot see the yield factor, and saying so is the point**: it is
not measured here and must not be reported as closed.

## 3. `Boiler` and `Generator` arms: 452 steam engines in, and zero table movement

The walk used to `_ => Prune` at a boiler, so the record base's 896 steam
engines had 692 incoming entity-graph edges and no flow node. Both arms exist
now, carrying the incoming flow through unchanged.

```
                    before this session      after
steam-engine  in_flow          0              452     (of 896, 692 with an incoming edge)
boiler        in_flow        459              460
```

The 452 are exactly the engines a boiler feeds; the remaining 444 are reached
by pipe only and are not reached from a root.

**Two honest limits, both written into the arm's own doc.**

- The **magnitude** is right and is not a rate table: a boiler converts one
  unit of fluid into one of its output fluid, and a generator passes along what
  it does not burn.
- The **name** is the input fluid's, so a boiler's steam is still labelled
  `water`. The output fluid's identity is `LuaFluidBoxPrototype::filter`, which
  the mod does not send; a generator's consumption needs `energy_usage`,
  `effectivity` and `maximum_temperature`, which it does not send either. The
  test pins the misnomer rather than hiding it.

**It moves no number in the fourteen-item table, and that was measured, not
assumed.** With the arm ablated to an unreachable `EntityType::Lab` and
everything else unchanged, the table is **byte-identical to the last digit** in
all four columns (`scratch/ablate_boiler.log`). So §5's whole move belongs to
§1. This is inert because `producer_nameplates` counts only drills, furnaces,
assemblers and pumps.

## 4. What is binding now: water, by 114×

The provenance ledger, measured on this binary in both states (the "before"
column is this branch with both changes mutated out, not quoted from a note):

```
                        item    guessed/min      eaten/min      made BEFORE      made AFTER
                   crude-oil        26666.7        13333.3           1380.0         13800.0
                  heavy-oil       113400.0        13800.0           1500.0          1500.0
                  light-oil         1170.0        59970.0           9900.0          9900.0
              petroleum-gas        36000.0        36000.0           9300.0          9300.0
                      water        59250.0        68250.0            600.0           600.0
```

Crude oil now covers its own demand. **Water does not, and never did — it was
simply not the tightest constraint while crude was ten times short.** 600/min is
10 offshore pumps at the `1.` per second the `OffshorePump` arm has hard-coded
since the file was written. The base game says `pumping_speed = 20` per tick,
i.e. **1,200/s, 72,000/min per pump** — a factor of 1,200.

**This is not fixed here, for the same reason the yield is not**: the runtime
API exposes it as `LuaEntityPrototype::get_pumping_speed()`, **a method, not an
attribute** (subclasses `OffshorePump`, `Pump`, taking a `quality` argument),
and the mod sends nothing for it. Writing `1200.` into the arm would be the
defect, not the fix. Handed over in §6.

Every fluid the residual error runs through needs water: `plastic-bar` is
`short of petroleum-gas`, `advanced-circuit` is `short of plastic-bar`, and
`processing-unit` is `short of sulfuric-acid`. Outlet-bound lines are
**65.8%**, against the 73.0% the predecessor recorded (its measurement, not
re-taken here).

## 5. The table

Every column computed by `production_rates_of_a_dumped_world`, one binary, one
dump, `entity_graph.connect()` re-run after loading. The "before" column is
the control and **it reproduces the predecessor's published figures to the last
digit** (0.306 nameplate / 1.554 sustained), which is what makes the delta
readable.

| item | game /min | before (sustained) | ratio | **after (sustained)** | **ratio** |
|---|---:|---:|---:|---:|---:|
| copper-cable | 22,367 | 1,552.0 | 0.07 | **2,992.1** | **0.13** |
| iron-ore | 15,247 | 16,500.0 | 1.08 | 16,500.0 | 1.08 |
| iron-plate | 15,170 | 8,278.1 | 0.55 | **9,230.5** | **0.61** |
| copper-ore | 15,157 | 15,870.0 | 1.05 | 15,870.0 | 1.05 |
| copper-plate | 15,147 | 886.7 | 0.06 | **1,973.5** | **0.13** |
| electronic-circuit | 6,694 | 494.5 | 0.07 | **857.2** | **0.13** |
| coal | 4,096 | 5,580.0 | 1.36 | 5,580.0 | 1.36 |
| plastic-bar | 2,846 | 52.0 | 0.02 | **319.7** | **0.11** |
| stone | 1,775 | 1,740.0 | 0.98 | 1,740.0 | 0.98 |
| steel-plate | 1,213 | 977.7 | 0.81 | **1,096.2** | **0.90** |
| advanced-circuit | 942 | 17.1 | 0.02 | **105.1** | **0.11** |
| iron-gear-wheel | 578 | 658.7 | 1.14 | 658.2 | 1.14 |
| stone-brick | 450 | 481.6 | 1.07 | **549.5** | **1.22** |
| processing-unit | 249 | 3.9 | 0.02 | **11.7** | **0.05** |
| **mean abs log error** | | | **1.554** | | **1.066** |
| *nameplate, unconstrained* | | | *0.306* | | *0.306* |
| *descent* | | | *1.913* | | *1.253* |
| *guessed bill* | | | *1.552* | | *0.893* |

**The mechanism is one, and it is the same for every row that moved**: crude
oil supply 1,380 → 13,800/min stops rationing the oil chain, and every line
below it — directly (plastic, advanced circuit, processing unit) or through the
outlet-bound coupling (copper plate, copper cable, iron plate) — gets more.
**Nothing was tuned; the unconstrained `nameplate` column does not move at
all**, which is the check that no machine, recipe or rate was touched.

**Two rows got worse and they are in the same table as the wins.**

- **`stone-brick` 1.07 → 1.22.** It moved the *other* way for the same reason
  the predecessor refused to bank it: 1.60 → 1.07 was rationing landing by luck
  on the right side of the truth, and relieving that rationing walks it back
  towards its honest over-prediction. The nameplate figure for stone-brick is
  **3.33**; nothing here brought that closer or further. This is the third time
  this file has recorded the same coincidence, after `copper-cable`'s 1.01 and
  `stone-brick`'s own 0.70.
- **`iron-gear-wheel` 1.14 → 1.14** and `steel-plate` 0.81 → 0.90 are the
  boundary cases: gears barely move, steel improves.

**The honest reading of 1.066 is that it is still dominated by one number we
cannot compute** — water, §4 — and that the fourteen items are not independent:
ten of them sit downstream of the two fluids.

## 6. Handed over, not taken

Three requirements, all of them fields the mod does not send. Two of them are
this session's, one is inherited.

**A resource's infinite-yield triple**, so a pumpjack's rate can carry the
well's yield: `LuaEntityPrototype::normal_resource_amount`, `infinite_resource`
and `minimum_resource_amount` — all three **attributes** on 2.1.17. The
numerator (`FactorioEntity::amount`) is already sent and already stored; this is
the denominator and the flag that says whether the ratio applies at all.
Measured effect if supplied: `crude-oil` would rise by roughly 8.83× on this
base, which the fourteen-item table cannot see because the surplus is already
sufficient.

**`LuaEntityPrototype::get_pumping_speed()` — A METHOD, NOT AN ATTRIBUTE**
(subclasses `OffshorePump` and `Pump`, takes a `quality`), so the offshore pump
arm can stop saying `1.` where the game says 1,200/s. This is now the largest
single wrong number in the model: 600/min supplied against 68,250/min eaten.
Reading it as an attribute raises, `pcall` swallows it, and the field goes
missing in silence — the trap that has caught four field pairs this week.

**`LuaFluidBoxPrototype::filter`** (and, for a generator's consumption,
`energy_usage` / `effectivity` / `maximum_temperature`), so a boiler's output
can be named `steam` rather than carried through as `water`.

Still open from the predecessor and untouched here:
`PipeConnectionDefinition::direction`; a furnace's recipe (all 1,215 report
`null`); off-map supply needing a second surface.

## 7. Verification

- `nix develop -c cargo test --workspace` → **exit 0** taken from the command
  and not from a pipeline; **107** `test result: ok` blocks, 0 failed.
  `cargo clippy -p factorio-bot-core --all-targets -- --deny warnings` clean.
  `rustfmt --edition 2024` on the one file.
- Mutations, each with the substitution **counted and refused unless it occurred
  exactly once**, applied one at a time and reverted (`scratch/mutate.py`):

  | mutation | kills |
  |---|---|
  | drop the yield-per-operation factor from the drill's rate | `a_pumpjack_yields_ten_crude_oil_per_operation` (alone) |
  | a resource that reports nothing yields three, not one | `a_resource_that_reports_no_mine_result_yields_one_per_operation` (alone) |
  | the `Boiler`/`Generator` arm is an unreachable `Lab` arm | `a_boiler_carries_its_water_through_to_the_steam_engine` (alone) |
  | the boiler passes an empty flow on | `a_boiler_carries_its_water_through_to_the_steam_engine` (alone) |
  | look the product up by iteration order, not by name | `a_resource_with_two_products_is_credited_with_the_one_being_mined` (alone) |

  **The fifth came back GREEN the first time, and that was the useful one.**
  Every resource in the base game and on the record base declares exactly one
  product keyed by its own name, so `products.get(resource)` and
  `products.values().next()` coincide on every fixture — the behaviour was
  real, unexercised, and would have broken on a modded resource with a
  byproduct. A test was written for it and the mutation now kills that test
  alone. Green is a broken experiment, not a pass.

  Every assertion is paired with a non-accidental one from the same
  computation: the pumpjack's 10.0 against an electric drill's 0.5 in the same
  test (a blanket multiplier fails), the default 1 against the same map's
  untouched `crude-oil` at 10 (the lookup must actually run), the fed engine
  against a stranded one that must stay out (the walk admits it, not its type),
  and the two-product resource's own 4 against a name that is a product but no
  resource.

- **Offline baselines on this branch's release binary, all four matching the
  published figures exactly** — `researched:automation` 176 / 21,784,
  `producing:automation-science-pack:6` 316 / 22,457,
  `producing:logistic-science-pack:6` 441 / 47,478 on `map.json`, and
  `gathered:crude-oil` 2,115 / 317,283 on `map-31337-explored.json`, with
  `map.json` refusing that goal as it should.

  **They prove nothing about this change and are reported for that reason.**
  The flow graph still has no planner caller, and nothing here touched a root
  predicate, an entity-graph rule or a recipe. What they rule out is the thing
  worth ruling out: that a change inside `update` leaked into a plan.
