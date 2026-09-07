# A surface knows its own daylight

**2026-09-07.** The energy fields landed hours earlier
(`2026-09-07-a-machine-says-what-it-draws.md`) and stopped, correctly, at
solar: `get_max_energy_production()` answers **60 kW for a `solar-panel`** and
**300 kW for an `accumulator`**, and both are instantaneous maxima that no
prototype turns into an average. That agent's reason was the whole of this
task — *"the follow-up is a surface-state channel, not a bigger prototype"* —
and it was right.

There is a channel now, and it answers both halves.

## The shapes, checked and not recalled

Read out of this install's `workspace/factorio-api-docs/runtime-api.json`
(`application_version` 2.1.17), because the neighbouring pair got this wrong in
both directions within a week: `energy_usage` is an attribute,
`get_max_energy_production` is a method, and reading a method as an attribute
raises inside the mod's `pcall` and arrives as a field that is simply nil.

| what | class | shape in 2.1.17 |
|---|---|---|
| `ticks_per_day` | `LuaSurface` | **attribute**, `uint32`, not optional |
| `dawn`, `dusk`, `evening`, `morning` | `LuaSurface` | **attribute**, `double`, not optional |
| `daytime` | `LuaSurface` | **attribute**, `double`, `[0, 1)` |
| `solar_power_multiplier` | `LuaSurface` | **attribute**, `double`, not optional |
| `always_day`, `freeze_daytime` | `LuaSurface` | **attribute**, `boolean` |
| `solar_panel_performance_at_day` / `_at_night` | `LuaEntityPrototype` | **attribute**, `double`, `subclasses: ["SolarPanel"]` |
| `electric_energy_source_prototype` | `LuaEntityPrototype` | **attribute**, optional, returns a `LuaElectricEnergySourcePrototype` |
| `buffer_capacity` | `LuaElectricEnergySourcePrototype` | **attribute**, `double`, not optional |

Ten attributes and no methods, so the coin landed the same way ten times —
which is exactly why it had to be looked up rather than assumed.

**Accumulators were reachable after all**, and the previous session's "a
sub-prototype nothing sends" was the accurate statement of a gap rather than of
an impossibility. `electric_energy_source_prototype` is one ordinary optional
attribute, and `buffer_capacity` hangs off it. Nothing in this project had ever
read through a sub-prototype before; that is all that was in the way.

## Where it lives

`SurfaceDaylight` on `FactorioSurface`, in a `SyncMutex<Option<_>>` beside the
other ledgers. **This is the least ambiguous per-surface field on that type** —
`ticks_per_day` and `solar_power_multiplier` are what differ between planets,
which is the aliasing argument `FactorioWorld`'s doc makes, in its purest form.
So when the game- and force-global fields are finally lifted off the surface
and `SurfaceNotYetSeparable` can be retired, this one needs no argument: it
stays. Its row is in that table now.

Both transports carry it, because the two are meant to agree about shape and a
collector that exists in one is a collector that drifts:
`writeout_daylight` beside `writeout_forces` on stdout, and `daylight` on
`WorldSnapshot` for the `--connect` path. A `world.dump` carries it too, which
matters more than it sounds: an offline plan has no game to ask, so a dump that
dropped the curve would make every solar question unanswerable on exactly the
basis this project iterates in.

## The derivation

`daytime` runs `[0, 1)` with **0 at noon**, so the sunlit half straddles the
wrap. Four boundaries cut the day into four straight runs:

```
  0        0.25       0.45      0.55       0.75        1
  |  full   |   fall   |  night  |   rise   |   full    |
 noon      dusk     evening   morning     dawn        noon
```

Output at any instant is `at_night + (at_day - at_night) * brightness`, times
the surface's `solar_power_multiplier`. Averaging it is the integral of a
trapezoid — exact, not sampled, because it is piecewise linear with four
pieces:

```
0.50 * 1  +  0.20 * 0.5  +  0.10 * 0  +  0.20 * 0.5  =  0.7
```

**60 kW at noon, 42 kW averaged.** Nothing in the code contains 0.7 or 42:
the endpoints come from the panel's prototype, the weights from the surface's
boundaries. `0.7` appears in this repo only in doc comments, as a *result*.

`always_day` and `freeze_daytime` short-circuit it, and they are the two ways a
surface can have perfectly ordinary-looking boundaries and a completely
different answer — the same shape as the `energy_usage`-is-90-kW-of-coal gate.

## 0.84 is not what the brief said it was, and that is the finding

The brief read the owner's **25 panels to 21 accumulators** as a statement that
average output is 0.84 of nameplate, and said to treat it as a check: if the
integral lands near 0.84, corroboration; if not, report the discrepancy rather
than tune.

It lands on **0.7**, and the discrepancy is not a discrepancy. **25:21 and the
average are answers to different questions about the same curve**, and 0.84 is
the *accumulator* one:

An array sized at its own average carries a flat load equal to that average. It
makes more than that around noon and less around midnight, and the *less* is
what a battery covers. Integrating `max(0, average - instantaneous)` over the
day — the area between the flat load line and the curve wherever the curve is
underneath — gives **0.168 of a full day's full output**:

```
ramp down  0.14 * 0.35  = 0.049
night      0.10 * 0.70  = 0.070
ramp up    0.14 * 0.35  = 0.049
                          -----
                          0.168
```

Times a panel's 1,000 J/tick times 25,000 ticks is **4.2 MJ**, against an
accumulator's 5 MJ: **0.84 accumulators per panel**, exactly 25:21.

So the one channel reproduces **both** vanilla ratios, by two independent
integrals, from data nobody here typed. That is stronger corroboration than the
brief asked for, and it also means conflating the two would size an array 20%
short — which is why both accessors exist and why a test pins them as
different numbers.

## A day is 25,200 ticks, not 25,000

**Measured, and it moves one of the two answers.** Every reference — the wiki,
this project's own arithmetic, and the unit tests as first written — says a
Factorio day is 25,000 ticks. A running 2.1.17 game says:

```
ticks_per_day  25200      (seven minutes exactly at 60 ticks per second)
dawn 0.75  dusk 0.25  evening 0.45  morning 0.55
solar_power_multiplier 1.0   always_day false   freeze_daytime false
```

The **average is unaffected** — it is a fraction of a day, whatever a day is —
so 0.7 and 42 kW stand. The **accumulator ratio is proportional** to it, so the
derived figure is `0.84 * 25200/25000` = **0.8467**, and the familiar 25:21 is
the number on a 25,000-tick day. A 0.8% disagreement between the rule of thumb
and the derivation, entirely explained by day length rather than tuned away.

The capture is `crates/core/tests/live-2.1.17-daylight.json`, lifted verbatim
from a `world.dump` on an isolated headless seed-31337 instance, and the test
asserts **both**: the 0.84 that a reader will check against, and the 0.8467 the
planner will use. This is the file's own failure mode caught in the act — a
type modelled on what the author already believed, invisible because no fixture
could contradict it — and it was caught within an hour of the fixture existing.

## Live, end to end

`world.dump` off a live headless game, mod through RCON through serde into the
world model, then read offline. 1,028 prototypes:

| | |
|---|---:|
| prototypes carrying `solar_panel_performance_at_day` | **1** (`solar-panel`) |
| prototypes carrying `electric_buffer_capacity` | **48** |
| `solar-panel` noon output | 1,000 J/tick = **60 kW** |
| `solar-panel` endpoints | 1.0 / **0.0** |
| `solar-panel` buffer | **0.0** — a real zero |
| `accumulator` `max_energy_production` | 5,000 J/tick = **300 kW** (a *rate*) |
| `accumulator` `buffer_capacity` | **5,000,000 J** (the *store*) |

Two of those are load-bearing. **A panel's buffer is a real 0.0**, so the
sizing code has to filter it rather than divide by it — the same
absent-versus-zero care the inserter rows needed yesterday, arriving from the
opposite direction. And **the endpoints appear on exactly one prototype**,
which is why their presence is the gate that says "this is a solar panel": a
steam engine credited a daylight average would be a silent 30% under-count of a
generator that runs all night.

## What is deliberately NOT wired up

**`electric_supply_kw` still credits nothing for solar**, and the reason has
changed even though the behaviour has not.

The old reason was determinism: a noon figure makes the same plan feasible or
not according to when the run started, and a planner whose output must be
identical for identical inputs cannot credit that. **That objection is now
answered** — an average is a function of surface constants and is perfectly
deterministic.

What is not answered is **storage**. An array credited its average keeps a base
alive only if the accumulators to carry the night are actually standing, and
nothing checks that. Crediting the average before that check exists trades a
*false refusal* — the safe direction — for a base that dies at midnight, which
is the direction this repo has twice recorded as reading like a base that is
completely dead rather than slow. `accumulators_per_panel` is the arithmetic
that check needs; making it is a solar arm of `method::power`, which is that
arm's work and not this table's. Both `DETERMINISTIC_GENERATOR_TYPES` and
`electric_supply_kw` say so in place.

So the deliverable here is a channel and two derivations with honest
`None`s — not a behaviour change.

## Verification

- `nix develop -c cargo test --workspace` — exit **0** taken from cargo and
  redirected to a file, never from a pipe. 104 `ok` results.
- `cargo clippy --workspace --all-features --all-targets -- --deny warnings` —
  exit 0.
- `cd app && pnpm lint` and `pnpm run test:coverage` — 53 files, 942 tests,
  both green. The OpenAPI snapshot was regenerated; the diff is **purely
  additive** and each new field carries its own `description`, with
  `electric_energy_usage` and `supply_area_distance` either side unchanged, so
  **no doc-comment theft**.
- **Eighteen falsification mutations**, one at a time, each asserting its
  substitution matched **exactly once in the breaking edit** and re-run after
  `rustfmt --edition 2024` had touched the files (`scratch/falsify.py`). All
  eighteen red.

**One came back green, and the investigation found something worth keeping.**
The mutation for "an unreported world answers unknown" replaced
`self.base.daylight()?` with `unwrap_or_default()` — and the test still passed,
because a default `SurfaceDaylight` is all-`None` and `runs()` refuses it
anyway. **The property is held by two independent guards**, and only the outer
one is what the tempting change would remove: a *vanilla fallback table*,
exactly the shape `vanilla_pole_supply_half_extent` and `vanilla_generation_kw`
already have next door. The mutation had to become that change to bite, and
does now. A green falsification is a broken experiment, and this one was.

### Baselines: nothing moved

Same release binary, this branch, `plan --bots 1,2,3,4`:

| goal | actions | ticks |
|---|---:|---:|
| `researched:automation` | 176 | 21,784 |
| `producing:automation-science-pack:6` | 316 | 22,457 |
| `producing:logistic-science-pack:6` | 441 | 47,478 |
| `gathered:crude-oil` (on `map-31337-explored.json`) | 2,115 | 317,283 |

Byte-identical to master's, as they must be: nothing that any of those goals
touches reads the new accessors, and every archived dump answers `None` for the
curve exactly as it did before the field existed.

**Which is also why the offline basis cannot exercise the new path**, and why
the live dump above is the check that matters. The previous session handled the
same gap by injecting live prototypes into `map.json`; here the equivalent is
the verbatim capture, which is stronger — it is a fixture the tests keep
running rather than a one-off.

## Follow-ups this leaves named

- **A solar arm of `method::power`**, which is what turns these two numbers
  into a plan: size the array on `solar_average_kw`, size the bank on
  `accumulators_per_panel`, and only then credit either in `electric_supply_kw`.
  Do not credit generation without the storage check.
- **Discharge rate is not modelled.** `accumulators_per_panel` sizes *energy*.
  An accumulator's 300 kW is a per-unit ceiling on delivery, and a bank with
  enough joules can still fail to hand them over fast enough. Nobody has asked
  that question yet.
- **`ticks_per_day` should be re-read on any other planet**, and this is the
  one field here that makes a second surface bite: the accumulator ratio is
  proportional to it and `solar_power_multiplier` scales the average, so a
  Vulcanus array is a different size. Which is the argument for where this
  field lives.
- `pole_wire_reach` and `delivery_offset` remain the two hand-kept tables,
  waiting on `maximum_wire_distance` and `vector_to_place_result`.
