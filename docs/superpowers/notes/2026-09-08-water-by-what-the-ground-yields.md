# Water by what the ground yields, and what the name pair is worth

2026-09-08. Branch `water-by-what-the-ground-yields`, off `e4047aa3`.

The follow-up the previous session filed and deliberately did not do: swap the
three hard-coded-name water predicates onto `LuaTilePrototype::fluid` without
making every archived dump unplannable.

## Where the names actually lived, and who called them

`FactorioTile::is_water` — the pair `["deepwater", "water"]` — had exactly
three call sites in the workspace, and every water question in the planner
went through one of them:

| predicate | file | its callers |
|---|---|---|
| `EntityGraph::is_water_at` | `crates/core/src/graph/entity_graph.rs` | `PlanState::occupant_of` (twice: skip water for a pump, and label `Occupant::Water`) |
| `EntityGraph::nearest_water_tile` | same file | `PlanState::nearest_water_tile` → `method::power::plan_plant_for` (both scan tiers), `score.rs`, `method::have` |
| `PlanState::water_tiles_within` | `crates/planner/src/state.rs` | `method::power::plan_plant_for`, the **shoreline** read |

The two `power.rs` callers are the load-bearing ones and they are *different
questions asked of the same predicate*: `nearest_water_tile` picks **which
lake**, `water_tiles_within` finds **that lake's shore**. That distinction is
what the falsification sweep turned into a finding — see below.

## The widening, and its exact form

One predicate, `FactorioTile::yields_water`, used by all three:

```rust
match self.fluid {
    TileFluid::Unknown => self.is_water(),          // nobody said: fall back to the name
    _ => self.fluid.yields(Self::WATER_FLUID),      // the ground answered: believe it
}
```

Two things it buys that a straight `yields("water")` would not, and one it
gives up nothing for:

- **`Dry` outranks the name.** A charted tile called `water` that a pump draws
  nothing from is not water. `Dry` is a fact, so it wins — this is the branch
  a `bool is_water` could never have expressed.
- **`Yields` outranks the name in the other direction.** A tile yielding water
  under any name at all is water, with no list to extend. That is the point.
- **`Unknown` stays inert.** It adds no water the name denies and removes none
  the name allows — asserted directly in the truth-table test, not just
  implied by the four baselines.

## What the by-name fallback is worth: the counterfactual, measured

Not argued — built and run. A second binary with the `Unknown` arm deleted
(`self.fluid.yields("water")` and nothing else), same tree, same debug profile:

| goal | world | with fallback | fallback deleted |
|---|---|---|---|
| `researched:automation` | `map.json` | 176 / 21,784 | **`PowerPlantNeedsWater`** |
| `producing:automation-science-pack:6` | `map.json` | 316 / 22,457 | **`PowerPlantNeedsWater`** |
| `producing:logistic-science-pack:6` | `map.json` | 441 / 47,478 | **`PowerPlantNeedsWater`** |
| `gathered:crude-oil` | `map-31337-explored.json` | 2,117 / 325,138 | **`PowerPlantNeedsWater`** |

Four of four refuse. And the refusal is worse than a bare failure, because it
is *confident*:

```
a power plant needs water, and the plan can see none within 128 tiles of
(0.0, 0.0), where charted ground covers 17 of 17 probes
```

`17 of 17 probes` is the planner saying **the ground is charted and there is
no water on it** — the `charting` discriminator `plan_plant_for` added
precisely so a refusal could tell *dry* from *blind* reports **dry** for a map
with two lakes on it. Deleting the fallback would not have produced a
diagnosable failure; it would have produced a plausible lie about the map.

So the fallback is load-bearing, and its scope is exactly named: **senders that
never filled `fluid` in.** Both world dumps and every archived server log —
4,440,064 tile records — are such senders.

## The baselines did not move

Same debug binary before and after the widening, `master` at `e4047aa3`:

| goal | world | before | after |
|---|---|---|---|
| `researched:automation` | `map.json` | 176 / 21,784 | 176 / 21,784 |
| `producing:automation-science-pack:6` | `map.json` | 316 / 22,457 | 316 / 22,457 |
| `producing:logistic-science-pack:6` | `map.json` | 441 / 47,478 | 441 / 47,478 |
| `gathered:crude-oil` | `map-31337-explored.json` | 2,117 / 325,138 | 2,117 / 325,138 |

All four inherited numbers were confirmed rather than assumed, and the oil goal
was given a 600 s bound (it takes ~94 s wall).

**And they are not a vacuous pass.** All three `map.json` goals actually site a
steam plant — 7, 8 and 8 `offshore-pump`/`boiler`/`steam-engine` steps
respectively — so the unchanged numbers are the water path running, not the
water path being skipped.

## The other direction: a live world whose tiles carry `fluid`

The archived dumps only exercise the `Unknown` arm. One headless run on an
isolated instance (`workspace/headless-a.toml`, ports 4330/34210, four
character bots, 10x, seed 31337 `--new`, debug build) produced a dump that
exercises the other:

```
yields:  5,497   (all of them "water")
dry:   404,103
unknown:     0
```

Planning `researched:automation` against **that** world, with three binaries:

| binary | result |
|---|---|
| name-only (`master`) | 176 / 21,779 |
| widened (this branch) | 176 / 21,779 |
| **fluid-only, no fallback** | **176 / 21,779** |

The no-fallback build — which refuses all four archived baselines — succeeds
here. That is the clean two-by-two: the ground answers on a live world and the
name answers on an archived one, and the widening is the only form that gets
both.

End to end, the sited pump lands at `[46.5, -8.5]`, standing on `dirt-7`
(`dry`) with `water` (`yields: water`) at the tile to its east — shore on land,
fluid to draw from, both read off the prototype.

## Is `["deepwater", "water"]` even the right name set under Space Age?

**Measured, and on this map it is exactly right — which is not the same as
being the right rule.** Pairing every tile name in the live dump with its
declared fluid:

```
3,675  deepwater  -> water
1,822  water      -> water
  ...  dirt-*, grass-*, sand-*, dry-dirt -> dry
```

Every water-yielding tile on seed-31337 Nauvis is in the pair, and nothing else
yields anything. So the pair is not *wrong here*, and I am not claiming it is;
the honest statement is narrower:

- it is right for **Nauvis at t=0 on this seed**, which is the only surface the
  mod sends (`control.lua`'s Nauvis guard) and the only map we run;
- it is **not a rule**, because Space Age's `ammoniacal-ocean` and Vulcanus'
  lava are counterexamples the day a second surface crosses the bridge, and a
  mod can add a water tile under any name tomorrow;
- so it is now documented as **a fallback for old senders, not the definition
  of water**, and the right response to a missing name is never to add it to
  the list — it is that the sender should declare `fluid`. It cannot be deleted
  while any archived dump is still planned against.

## The green mutation, and what it exposed

Five substitutions, each asserted to match exactly once, each built and run
against the whole workspace. Four went red immediately. **The fifth was
green**, and it was the interesting one:

| mutation | result |
|---|---|
| M1 drop the `Unknown` fallback | RED — 3 tests, incl. the existing `water_charted_on_one_surface_is_not_water_on_the_other` |
| M2 ignore the ground, trust the name | RED — 3 tests |
| M3 `is_water_at` back to the name pair | RED — 2 tests |
| M4 `nearest_water_tile` back to the name pair | RED — 2 tests |
| **M5 `water_tiles_within` back to the name pair** | **GREEN — 3,064 tests, all passing** |

M5 is the shoreline half. Nothing in the workspace could see it revert, because
**every water fixture in the tree names its lake `water` and declares that it
yields water** — `test_utils::spawn_water` and `entity_graph::tests::terrain`
both derive `fluid` from the name, deliberately, so a fixture "cannot claim a
dry lake or a wet meadow by typo". The cost of that consistency is that no
fixture can tell the two predicates apart.

Live, that mutation is nastier than a plain miss: `nearest_water_tile` finds
the lake by fluid, `water_tiles_within` fails to find its shore by name, and
the plant refuses with `PowerPlantNeedsShore` — **geometry blamed for a naming
problem** on a map with a perfectly good shoreline. Same shape as the
`Occupant::Terrain` masking `Occupant::Refused` bug: a correct-looking refusal
naming the wrong cause.

Closed by `a_lake_the_name_pair_does_not_know_still_has_a_shore` in
`method::power`'s tests — the fixture lake in the same place, named
`wetland-green-slime`, declaring water. M5 re-run with it in place: **RED**.

The transferable part: **a fixture whose fields are derived from one another
cannot falsify a change that separates them.** The consistency that makes
fixtures trustworthy is exactly what blinds them here, and only the mutation
found it — four reviews of the same diff would not have.

## Verification

- `nix develop -c cargo test --workspace --no-fail-fast`, redirected to a file
  with cargo's own exit code read: **3,065 passed / 0 failed, exit 0**
  (3,060 inherited + 5 added).
- `cargo clippy --workspace --all-features --all-targets -- --deny warnings`:
  exit 0.
- Five mutations, all red (M5 after its test landed). Restored by file copy
  plus `touch`, never `git checkout --`; committed before the sweep.
- `workspace/mods/BotBridge` and `workspace/headless-a/mods/BotBridge` both
  `readlink`ed before and after the live run and restored to the main checkout.

## What I inherited and did not change

The brief's four baseline numbers were all correct as given, and the 600 s
bound was needed (94 s wall for the oil goal). The brief's proposed form
`yields("water") || (Unknown && by-name)` is what shipped, unmodified. The one
thing the brief asked that came back with a different answer than expected is
the Space Age name question: I went looking for water-yielding tiles outside
the pair and **found none**, so the case for the change rests on the rule being
wrong in principle and on `Dry` being expressible — not on the pair being
incomplete on any map we have.
