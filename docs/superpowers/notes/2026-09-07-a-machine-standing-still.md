# A machine standing still: what the flow graph can and cannot say about idleness

2026-09-07. Session `backpressure`, branch `a-machine-standing-still`, off
`d86e7dae`. Oracle: the 6:39:53 Space Age rocket-launch save, dumped as
`workspace/wrload/scripts/wr-census.json`, against the game's own production
statistics recorded in
`docs/superpowers/notes/2026-09-06-what-the-record-base-knows.md`.

The brief: build a duty-cycle / back-pressure term, because idleness was
measured as the dominant and only *unbounded* term in the model's error
(+16% to +23%, against coverage at 1.6% and modules at 0%).

**What was built is a predictive whole-base supply balance, not an observed
duty cycle.** The reason is the first finding below, and it is not a
preference.

---

## 1. The oracle carries no per-machine state, so "measure it first" was not
   available

The brief's instruction was to measure the distribution of `LuaEntity.status`
across the 1,222 furnaces before modelling anything. **The dump cannot answer
that, and neither can any other dump this project has made.**

Checked, not assumed. Every furnace record in `wr-census.json` reads:

```
"output_inventory": null,
"fuel_inventory": null,
```

and there is **no `input_inventory` key and no `transport_lines` key anywhere
in the file** -- those fields landed in the mod on 2026-09-06/07 (`b85679b1`,
`70982977`), after the dump was taken. `FactorioEntity` has never carried
`status` at all. So the offline oracle holds geometry and recipes and nothing
whatever about what a machine was doing.

The mod *does* read `entity.status` (`mods/BotBridge/control.lua`,
`machine_row`), but it writes it into `samples.jsonl` on the sampling session
-- a per-run time series, not part of the world the flow graph is built from.

**Handover, since `mods/` is out of scope for this session** -- see §5.

## 2. A per-machine duty cycle from this graph is unsound, and the falsification
   is in the tests

The obvious implementation is per machine: divide what
`sum_incoming_edge_weights` says is arriving at a machine's tile by what its
recipe eats. That was built first, for furnaces and for assemblers, and
measured. It is wrong, and the measurement says how wrong:

`what_consumers_see_arriving_is_not_a_conserved_flow` (new, `#[ignore]`d,
gated on `FACTORIO_BOT_WORLD_DUMP`) puts, per item, what the graph says is
**produced** beside what its 2,042 consumers say is **arriving**. In a
conserved flow the second cannot exceed the first. Measured:

| item | produced /min | arriving /min | ratio |
|---|---:|---:|---:|
| coal | 5,040 | 6,016,020 | **1,193.65** |
| iron-ore | 16,500 | 1,910,474 | **115.79** |
| stone | 1,740 | 165,225 | 94.96 |
| copper-ore | 15,870 | 743,400 | 46.84 |
| iron-plate | 19,016 | 19,447 | 1.04 |
| **steel-plate** | 1,590 | 60 | **0.07** |
| **engine-unit** | 243 | 4.5 | **0.02** |

**From 0.02x to 1,194x.** It is not a bound in either direction, so a duty
cycle derived from it is noise, and the noise is not small: shipping the
per-machine version moved iron plate by 2% (16,500-fold excess ore at the
furnaces pins the duty at 1) and pushed steel plate from **31% high to 24%
low** (steel furnaces see 7% of the plate the base makes).

The cause is by design. `update_flow_edge` writes a machine's **whole** output
on **each** of its outgoing edges -- that is what makes `throughput_at` right
for any one consumer -- and `sum_incoming_edge_weights` then adds those up.
`production_rates` already compensates on the *producer* side by taking a
maximum rather than a sum; nothing does on the consumer side, and the graph
holds nothing that could. **Making edges a conserved flow is a real piece of
work and this is not it** -- `Splitter` is the only arm that divides
(`divide_flowrate`); belts and inserters do not.

## 3. What was shipped: `sustained_production_rates()`

The same question asked at the aggregation that *is* conserved -- the whole
surface. `production_rates()` is unchanged and still means **nameplate**: every
machine at 100%, ingredients assumed. The new sibling adds one term: a machine
may not consume more of an item than the base makes of it.

Per **output**, not per machine: a furnace this graph credits with copper plate
and stone brick at once has two independent products, and a stone shortage must
not throttle its copper. Gating on the minimum over everything a machine
touches took copper plate from 9% high to 12% low while its ore was never
short.

**Summing over the surface is also what makes it robust to the graph's
inability to route.** Ore that reaches a smelter by train, by bot, or through a
chest nothing feeds is counted in the base's supply even though no edge carries
it -- which is exactly the class of failure §2 is about.

### It throttles on shortage and never on surplus, deliberately

Demand here is only what *modelled producers* consume. A wall, a rocket, a lab,
a chest somebody fills are all invisible, so demand is systematically
understated and a surplus proves nothing. A shortage is different: if the model
can only see 16,500 ore/min being mined, consumers needing 19,016 cannot all be
running.

So **this closes the `no_ingredients` half of idleness and not the other half**.
The 194 drills reading `waiting_for_space_in_destination` on this base are
still modelled at full rate, and iron ore accordingly does not move at all
(1.08 before and after).

### Two traps found while building it, both silent

- **`casting-iron` sorts before `iron-plate`.** Space Age gives most items a
  foundry recipe, and charging an item's ingredients through "the recipe that
  makes it, by name" billed every plate to **molten iron** -- which nothing on
  Nauvis makes, and which this function treats as unknown rather than absent.
  The entire iron and copper constraint switched itself off and **nothing
  looked wrong**: the rates simply stayed at nameplate. Fixed by preferring,
  among candidates, a recipe whose ingredients the base actually supplies --
  data-driven, so it survives a mod adding a third route.
- **`coal` has a synthesis recipe, and a drill is not a machine.** Charging
  every producer through its recipe billed 559 coal drills for carbon and
  sulfur they never touch: the model reported **46 coal/min against a real
  4,096**. A drill and an offshore pump take their output from the ground and
  are never charged.

## 4. The result, on the oracle

Nameplate is byte-identical to the pre-change baseline -- the change is
additive.

| item | game /min | nameplate | ratio | **sustained** | **ratio** |
|---|---:|---:|---:|---:|---:|
| copper-cable | 22,367 | 22,680 | 1.01 | 22,680 | **1.01** |
| copper-plate | 15,147 | 16,580 | 1.09 | 15,870 | **1.05** |
| iron-plate | 15,170 | 19,016 | 1.25 | 16,469 | **1.09** |
| electronic-circuit | 6,694 | 8,820 | 1.32 | 5,584 | **0.83** |

And the rest of the table the game reports:

| item | game /min | nameplate | ratio | sustained | ratio |
|---|---:|---:|---:|---:|---:|
| iron-ore | 15,247 | 16,500 | 1.08 | 16,500 | 1.08 |
| copper-ore | 15,157 | 15,870 | 1.05 | 15,870 | 1.05 |
| coal | 4,096 | 5,040 | 1.23 | 5,040 | 1.23 |
| plastic-bar | 2,846 | 2,880 | 1.01 | 2,880 | 1.01 |
| stone | 1,775 | 1,740 | 0.98 | 1,740 | 0.98 |
| steel-plate | 1,213 | 1,590 | 1.31 | 1,007 | 0.83 |
| advanced-circuit | 942 | 1,035 | 1.10 | 655 | **0.70** |
| iron-gear-wheel | 578 | 2,070 | 3.58 | 1,311 | 2.27 |
| stone-brick | 450 | 1,454 | 3.23 | 317 | 0.70 |
| processing-unit | 249 | 360 | 1.45 | 135 | **0.54** |

Mean absolute log error over all fourteen items: **0.297 -> 0.216**, a 27%
reduction. Six items improve, six are untouched, **two get worse**.

### Copper cable did not move, and that is the honest outcome

Its 1.01 agreement is a **coincidence** -- +12% beacon speed and +3%
productivity nearly exactly cancel 18% idle -- and the brief was right to warn
that a number improving for the wrong reason is worse than one staying wrong.
It is unchanged here because copper plate supply (15,870/min) exceeds what its
consumers demand, so nothing throttles it. The coincidence is preserved intact
rather than papered over.

### The two that got worse say what is still missing

`advanced-circuit` (1.10 -> 0.70) and `processing-unit` (1.45 -> 0.54) are the
deepest items in the chain, and they under-predict because **the demand side
still assumes every machine runs at nameplate.** The base has assemblers making
pipe at 900/min and rail at 540/min that are mostly idle; charged at full rate,
they eat an iron plate shortage that is largely fictional, and the shortfall is
spread onto everything downstream. We removed the idleness assumption from the
supply side and left it standing on the demand side.

That is the next piece of work and it is the same piece as the other half of
idleness: a machine's *demand* should be its duty cycle times its nameplate,
which is exactly the reading §5 asks for.

## 5. The mod reading to hand over

**One field, on the entity record, not on the sample stream.**

`mods/BotBridge/types.lua`, in the entity serialiser that already writes
`output_inventory` / `fuel_inventory` / `input_inventory`:

```lua
-- `LuaEntity.status` is subclasses:None in 2.1.17, so it is safe on any
-- entity, and optional:true, so write it only when it is not nil.
local status = entity.status
if status ~= nil then
    record.status = ENTITY_STATUS_NAMES[status] or ("unmapped_" .. tostring(status))
end
```

`ENTITY_STATUS_NAMES` already exists in `control.lua` and would need to move or
be shared; the **name** crosses the wire, never the number, for the reason
stated there -- a status id is meaningless without the table that resolved it.

The matching half is `pub status: Option<String>` on `FactorioEntity` in
`crates/core/src/types.rs`, `#[serde(default)]` so every archived record and
world dump still deserialises, and `None` meaning *the sender did not say* --
never `working`.

**What it buys, in order of value:**

1. An **observed** duty cycle, which is what the brief asked for and what no
   amount of modelling substitutes for. `production_rates()` could be multiplied
   by the measured working fraction per machine and be *right* rather than
   argued.
2. The `waiting_for_space_in_destination` half of idleness, which §3 explicitly
   does not model and which nothing derivable from this graph can.
3. The demand-side fix in §4 -- a machine's ingredient demand at its real duty
   rather than at nameplate, which is what is dragging `processing-unit` to
   0.54.

Until then, `sustained_production_rates()` is a prediction with its blind spots
written on it, and `production_rates()` remains the honest upper bound.

## 6. What is in the tree

`crates/core/src/graph/flow_graph.rs` only.

- `sustained_production_rates()`, `producer_nameplates()`, `recipe_making()`.
- `production_rates()` refactored onto `producer_nameplates()`; its output is
  unchanged, verified against the oracle.
- Three unit tests, each falsified once with the substitution confirmed to
  break exactly one test:
  - `a_base_makes_only_what_its_ore_supply_supports` -- fails when the throttle
    is removed.
  - `the_ground_is_not_a_recipe` -- fails when a drill is charged like a
    crafting machine. **This one passed while testing nothing on its first
    substitution**: the fixture's synthesis recipe used an invented ingredient,
    so the drill escaped the bill for the unrelated reason that an unknown
    ingredient never constrains. Changed to iron plate, which the fixture base
    really makes and really is short of. Six ways a check comes back green
    while testing nothing are catalogued in
    `2026-09-06-fixtures-agree-with-their-code.md`; this is a seventh.
  - `a_recipe_whose_ingredients_the_base_never_makes_is_not_the_one_charged` --
    fails when recipe choice reverts to sorting, i.e. to `casting-iron`.
- `what_consumers_see_arriving_is_not_a_conserved_flow`, `#[ignore]`d, the
  measurement behind §2.
- The dumped-world harness now prints nameplate and sustained side by side.

Verification: `nix develop -c cargo test --workspace` green (101 `ok` results,
0 failed), `cargo clippy -p factorio-bot-core --all-targets -- --deny warnings`
clean. The three offline planner baselines are **byte-identical** on the same
binary before and after -- `researched:automation` 176 / 21,784,
`producing:automation-science-pack:6` 316 / 22,457,
`producing:logistic-science-pack:6` 441 / 47,478 -- as expected, since the flow
graph still has no planner caller.
