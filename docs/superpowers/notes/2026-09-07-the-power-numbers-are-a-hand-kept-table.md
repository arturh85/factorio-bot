# The power numbers are a hand-kept table, and the milestone rests on them

*2026-09-07. Found while checking whether the electric-smelter milestone is
blocked by the anchor-recovery decision. It is not blocked by that — it is
blocked by this.*

## What is there

Every electrical number this planner uses is hard-coded in `state.rs`:

```rust
fn consumer_kw(name: &str) -> Option<f64> {      // 15 entries
    "assembling-machine-1" => Some(75.0),
    "electric-mining-drill" => Some(90.0),
    "electric-furnace" => Some(180.0),
    "beacon" => Some(480.0),
    ...
}

fn generation_kw(name: &str) -> Option<f64> {    // 2 entries
    "steam-engine" => Some(900.0),
    "steam-turbine" => Some(5800.0),
    _ => None,
}
```

`solar-panel` and `accumulator` are in neither, which is why
`a_solar_block_reads_as_unpowered_on_purpose` exists and why the owner
overruled it: *"for solar it should just assume the average output, we have
batteries to smooth out the power generation later"*, at **25 solar panels to
21 accumulators**.

## Why adding two rows is the wrong fix

The owner's standing preference is explicit — derive ratios from prototypes so
they survive mods; a hard-coded rate is a mod-compatibility defect. And this
repo has just paid for the general version of that: `pole_supply_half_extent`
was a hand-kept table, the real `supply_area_distance` replaced it, and **a big
pole reaches 32, not the 30 the table said** (`59ab9ca3`). Every pole placement
before that was slightly wrong, silently, because the only code reading the
table was code that agreed with it.

`consumer_kw` is fifteen rows of exactly that shape, and it decides whether a
block browns out.

## Why it cannot be derived today

**The mod sends no energy data at all.** `FactorioEntityPrototype` has 17
fields and not one of them is electrical:

```
name, entity_type, collision_mask, collision_box, mine_result, mining_time,
mining_speed, crafting_speed, max_underground_distance, fluidbox_prototypes,
mining_drill_radius, resource_category, resource_categories, mining_fluid,
supply_area_distance, distribution_effectivity, beacon_profile
```

The only `energy` the mod sends anywhere is a *recipe's* crafting time
(`mods/BotBridge/types.lua`), which is a different quantity.

So this is not a taste question about where a constant should live. The data
required to derive it has never crossed the bridge.

## The shape of the fix, and it is a shape this repo has already run

The last four fields in that list — `mining_drill_radius`,
`supply_area_distance`, `distribution_effectivity`, `beacon_profile` — were all
added recently by exactly this route, and each replaced a guess or a table with
a fact. The route is known to work:

1. **mod** (`mods/BotBridge/types.lua`): send `max_energy_production` and
   `energy_usage` per entity prototype.
2. **types** (`crates/core/src/types.rs`): two `Option<f64>` fields. The
   snapshot seam then fails from both ends until the TypeScript side mirrors
   them, which is the point of it.
3. **planner** (`crates/planner/src/state.rs`): `consumer_kw` and
   `generation_kw` read the prototype, keeping the table only as a fallback
   for a dump that predates the fields — `None` means *unknown*, never zero,
   the same distinction `mining_drill_radius` already documents.

Solar then needs one derived judgement rather than a constant: **average
output, not nameplate**, per the owner. A panel's nameplate is its noon figure,
and the accumulator ratio the owner gave — 25:21 — is itself the statement that
average is 0.84 of what the panels would otherwise promise. **That inference is
WRONG — see the correction at the end of this note.** Deriving the ratio
from the two prototypes rather than writing 0.7 or 42 kW anywhere is what makes
it survive a mod that changes either.

## Why it matters now rather than later

The milestone is electric smelting. CLAUDE.md already does the arithmetic:
24 electric furnaces at 180 kW is **4,320 kW**, against a plant that tops out
at 1.8 MW with two engines — which is the whole reason a second boiler and then
solar are on the path at all.

**That 180 is a hand-typed number in a fifteen-row table, and the 900 beside it
is one of two.** So the obvious worry is that one of them has drifted the way
the pole table did, in a direction nobody would notice: a block that passes its
own power check and browns out.

## MEASURED, and the worry is falsified

**The table has not drifted.** Checked against the game's own prototype files,
which are on disk (`workspace/server/data/base/prototypes/`) and need no run:

```
assembling-machine-1/2/3   75 / 150 / 375   MATCH
electric-mining-drill 90   pumpjack 90      MATCH
lab 60   electric-furnace 180               MATCH
chemical-plant 210   oil-refinery 420       MATCH
radar 300   beacon 480                      MATCH
```

All eleven checkable `consumer_kw` rows agree exactly, `electric-furnace` 180
included. **The milestone arithmetic is sound as written** and the second boiler
really is needed. So this is a maintainability fix, not a bug fix, and the case
for it rests on mod compatibility alone — which was the owner's reason in the
first place, and did not need my embellishment.

**The generator side is the part that changes the implementation.** A generator
carries no production field at all — only `fluid_usage_per_tick`,
`maximum_temperature` and `effectivity`:

```
kW = fluid_usage_per_tick * 60 * heat_capacity * (max_temperature - default_temperature) * effectivity
steam: heat_capacity 0.2kJ, default_temperature 15

steam-engine   0.5 * 60 * 0.2 * (165-15) = 900 kW      table 900     exact
steam-turbine  1.0 * 60 * 0.2 * (500-15) = 5,820 kW    table 5,800   off by 20
```

So asking the mod for `max_energy_production` is the wrong shape for generators:
the field does not exist at the data stage. Send the four inputs and derive —
which is the honest form of "derive, do not tabulate", because the derivation is
the physics and survives a mod changing a temperature or a fluid.

**The one real discrepancy is the turbine: 5,800 against a derived 5,820**,
0.34%, in an entity nothing builds yet. Small, and exactly the class a
derivation removes.

**Solar, with numbers**: `solar-panel` has `production = "60kW"` and no
`energy_usage`; `accumulator` has `buffer_capacity 5MJ` with `input_flow_limit`
and `output_flow_limit` both `300kW`. 60 kW is the noon nameplate, so the
owner's "assume the average" needs the day/night factor — and their 25:21 ratio
*is* that statement, since 21/25 = 0.84.

## CORRECTION: 25:21 is the accumulator ratio, not the average

I asserted twice above that the owner's 25:21 states "average is 0.84 of
nameplate", and passed that to the peer, who put it in an agent brief. **It is
wrong.** The agent measured it rather than inheriting it. They are two different
integrals of the same curve:

```
average       0.70 of nameplate    60 kW noon -> 42 kW
              0.50*1 + 0.20*0.5 + 0.10*0 + 0.20*0.5
accumulators  0.84 per panel       integral of max(0, average - instantaneous)
              0.168 of a day's full output -> 4.2 MJ against a 5 MJ buffer
```

**Sizing an array on 0.84 rather than 0.70 comes out 20% short.** The two
numbers are close enough to look interchangeable and are not, which is exactly
why the coincidence was worth checking rather than reasoning from.

And the live day is **25,200 ticks, not the 25,000 every reference gives** —
seven minutes exactly. The average is a fraction of a day and so is unaffected;
the accumulator ratio is proportional, making it 0.8467. The entire 0.8% gap
from the owner's 0.84 is day length. That is the third number this week that
everyone quotes and the game does not hold, after the pole's 30-against-32 and
the turbine's 5,800-against-5,820.

The mistake has the shape this note is otherwise about: I had two numbers that
nearly matched and concluded they were the same quantity, instead of asking what
each was an integral *of*. **A match is more suspicious than a mismatch** — this
repo already says so about the flow graph's copper-cable 1.01 — and I did not
apply it to my own arithmetic.

**A note on the instrument, because the first version of it lied.** Scanning a
fixed 6,000-character window after each `name = "..."` let fields bleed across
entity boundaries: it reported `steam-engine` at 60 kW (solar's `production`)
and gave solar the accumulator's buffer and flow limits. Bounding each block at
the *next* `name =` fixed it. The tell was a value that made no sense for the
entity it was attached to — and had the contamination landed on a plausible
number instead, this note would have shipped a wrong table verdict.

## Ownership

`state.rs` and `mods/BotBridge/` are the peer's files; this note is the handoff,
not the change.
