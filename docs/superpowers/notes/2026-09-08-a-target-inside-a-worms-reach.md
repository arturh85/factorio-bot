# A target inside a worm's reach

2026-09-08. Branch `a-target-inside-a-worms-reach`, off `a367faf9`.

`entity_graph.threats` was populated and had no reader in target selection.
A live run sent bot 1 to chop a rock with three `small-worm-turret`s at 20.4,
20.7 and 20.7 tiles and lost it at tick 53,619; all three were already in the
dump the planner read. Bot 4 was killed by a worm at 34,873.

The rock is identifiable in the dump: **`huge-rock` at (-259.75, 76.75)**,
with worms at exactly 20.4, 20.7 and 20.7 tiles and a `biter-spawner` at 27.3.

## Two premises in the brief were wrong

Both were flagged as likely-wrong by the person who wrote them, and both were.

**"`threats` has no reader anywhere in `crates/planner`" is false.**
`method/scout.rs` reads it — `THREAT_STANDOFF`, 50 tiles, applied to survey
lattice cells, with `Skip::Threat` naming the nest and a `is_boxed_in()`
refusal. The gap was in **target** selection specifically, not in the planner.
This matters beyond pedantry: scout's constant is documented as a deliberately
generous guess, and it turns out to be *exactly* the game's own
`call_for_help_radius` for a spawner (see below). That is a coincidence and is
recorded as one.

**"`gather`/`extract` choose which rock or ore patch to send a bot to" is
false.** `method/gather.rs` is the **oil wellhead rig** — pumpjack, tank, pipe —
and `method/extract.rs` sites a pumpjack on a well. Neither picks a rock.

Targets are actually chosen in two places, found by following call sites rather
than names:

| what | where | how |
|---|---|---|
| rocks and trees | `Chop::expand`, `crates/planner/src/method/have.rs` | sorts `minable_sources` by distance from the chain actor, takes nearest first |
| ore tiles | `nearest_resource_tile`, `resource_tiles_for`, `resource_supply_at_least`, `resource_seats` — all in `crates/planner/src/method/util.rs` | same, over `resource_patches` |

`Chop::expand` is the one that chose that rock. It consulted nothing but
distance, so the plan was correct by its own lights.

## A worm's range does NOT cross the bridge

Checked rather than assumed, from both ends:

- `FactorioEntityPrototype` (`crates/core/src/types.rs`) has 26 fields and
  none is an attack parameter.
- `mods/BotBridge/types.lua` reads none — no `attack_parameters`, no `range`.
- In `map-31337-explored.json`, `small-worm-turret`'s prototype carries
  **exactly four non-null fields**: `name`, `entity_type`, `collision_box`,
  `collision_mask`. Same for `medium-worm-turret`, `biter-spawner` and
  `spitter-spawner`.

So the range had to be **named as a gap**, not derived. Both halves were done:

1. **The type was widened.** `FactorioEntityPrototype::attack_range:
   Option<f64>`, `#[serde(default)]`, read **first** by
   `PlanState::threat_standoff`. Nothing fills it, so the seam is inert today —
   but the day the mod sends a range, it wins with no other change. A test
   injects one and asserts it beats the table.
2. **The fallback is named, and its provenance travels with every number.**
   `ThreatStandoff { tiles, source }` where `source` is `Prototype`,
   `NamedFallback` or `UnknownKind`, and `Display` renders
   `25 tiles (named fallback; the mod sends no attack range)`. A refusal that
   quotes an assumption as though the game said it is worse than no refusal.

**The fallback answers 100% of calls today.** That is stated in its own doc, the
way the by-name water fallback is, rather than quietly relied on.

The numbers are read off the **installed game's own data files**, cited line by
line, not from a wiki and not invented:

```
workspace/data/base/prototypes/entity/enemy-constants.lua:134-137
    range_worm_small = 25   _medium = 30   _big = 38   _behemoth = 48
workspace/data/base/prototypes/entity/enemies.lua
    biter-spawner: call_for_help_radius = 50
```

`prepare_range` (+8 for a small worm, so it stands up at 33) was **deliberately
not added**. A worm that has noticed a bot at 30 tiles still cannot hit it, and
inflating the avoided area by a third on an unmeasured argument would cost
walks for a margin this session cannot demonstrate. Recorded so the next reader
knows it was considered.

## Spawners are modelled differently from worms, on purpose

They are different risks and collapsing them would be the defect, not the
simplification:

- **A worm is a static gun with a range.** Stand outside 25 and that worm
  cannot touch you. Its standoff *is* its attack range.
- **A spawner shoots nothing.** Its `attack_range` would be **zero** — and zero
  is precisely the reading that sends a bot to mine a rock in the middle of a
  nest. What it does is produce roamers, whose reach is not a radius at all but
  wherever the unit walks. Its standoff is therefore its own
  `call_for_help_radius`, the game's nearest statement of "how far this nest's
  units answer a disturbance".

Neither is ever zero, and an **unrecognised** enemy structure gets the widest
known standoff rather than none — `absent-is-not-a-value`, in the one place
where getting it wrong costs a bot. A test asserts a spawner and a worm never
collapse onto one number and that neither is zero.

**What this does not address, and cannot:** the third death (bot 2) was a
roamer 166 tiles from anything static. No standoff reaches that, routing buys
nothing, and it is already handled by the standing return-fire order from
`ffb6e67e`. This change addresses only the part that is positional.

## Shape: prefer, then refuse

`PlanState::minable_sources_by_safety` splits standing sources into safe and
covered. `Chop::expand` walks past the covered ones while a safe one stands,
and refuses with `PlannerError::TargetInsideThreat` — naming the threat, its
position, its distance and its standoff-with-provenance — only when none is
left. The filter runs *before* the distance sort, so `steps.is_empty()` keeps
meaning "no source at all" and cannot absorb a threat refusal into a
`NoApplicableMethod`, which would report a map with a rock on it as having no
route to wood: true of the plan, a lie about the world.

`chop_beats_mining` prices over the **safe** sources only, so chopping cannot
win a goal on the strength of rocks `expand` is about to pass over. Combined
with `applicable` still asking the *unfiltered* `has_minable_source`, this puts
the refusal in the right place: when ore exists, a zero safe supply drops
through and `Mine` gets its turn; when it does not — wood — `Chop` still claims
and refuses by name.

Ore tiles are passed over **silently**, with no refusal, through one shared
`threatened_tile` predicate that all four selectors read. They must share it:
`resource_seats` sizes a split that `resource_tiles_for` then has to fill, and
a seat counted in one and refused in the other is the disagreement the claim
ledger exists to prevent.

The asymmetry is measured, not assumed. On `map-31337-explored.json`:

| resource | charted tiles | inside a standoff |
|---|---:|---:|
| iron-ore | 2,452 | 374 (15.3%) |
| coal, copper-ore, stone, uranium-ore, crude-oil | 3,714 | **0** |
| minables (rocks/trees) | 17,431 | 1,465 (8.4%) |

Ore comes in fields of thousands of tiles and always has a safe neighbour here;
a rock is one of a handful of discrete entities, any of which may be the only
one. Hence a named refusal for one and a silent skip for the other.

### The known gap, from a test that failed

Two attempts at the ore test failed before the third passed, and the failure is
the finding. `test_utils::fixture_world`'s iron field is 11×11, about 14 tiles
across, and a small worm reaches 25 — so **a worm anywhere near a small patch
covers all of it**, and there is no safe tile at all. Ore then falls through to
`NoApplicableMethod` rather than to a refusal naming the worm.

That is left open rather than fixed, because on the maps measured here it does
not arise (0% for five of six resources, 15.3% for iron out of a 2,452-tile
field). It is written into the test's own doc so it is not rediscovered.

## Baselines

One debug binary, `a367faf9` plus this change, all four measured before and
after on the *same* build. Goal strings as given.

| goal | world | before | after |
|---|---|---|---|
| `researched:automation` | `map.json` | 176 / 21,784 | **176 / 21,784** |
| `producing:automation-science-pack:6` | `map.json` | 316 / 22,457 | **316 / 22,457** |
| `producing:logistic-science-pack:6` | `map.json` | 441 / 47,478 | **441 / 47,478** |
| `gathered:crude-oil` | `map-31337-explored.json` | 2,117 / 325,138 | **2,117 / 314,345** |

The first three are unchanged, and **unchanged is the only correct answer**:
`map.json` holds zero threats, so nothing in this change can be reached. The
explored map holds 13 `biter-spawner`, 13 `small-worm-turret` and 6
`spitter-spawner`, and that is where the change shows up.

### The mechanism of the oil move, established by a control

The guard was disabled in place (`threat_covering` forced to answer `None`) and
the plan re-made: **325,138 exactly**. So the guard alone accounts for the
−10,793 ticks, and the two plans are comparable target-for-target.

```
chop targets      pre 64   post 64      common 60, dropped 4, added 4
THREATENED        pre  4   post  0
```

The four dropped, with the nest that covered each:

```
huge-rock (-167.13, -333.25)   biter-spawner at  5.3 tiles
huge-rock (-187.31, -336.75)   biter-spawner at 19.9
huge-rock (-191.25, -338.81)   biter-spawner at 23.8
huge-rock (-190.06, -358.63)   biter-spawner at 30.2
```

and the four that replaced them, by distance from spawn:

```
dropped:  372.8  385.3  389.1  405.9
added:    213.5  215.7  227.7  513.8
```

**The makespan gain is incidental and is not claimed as an optimisation.**
`Chop::expand` sorts by distance from the chain actor at expansion time, not
from spawn; removing four candidates sent that sort elsewhere, and three of the
four replacements happen to be 150-190 tiles nearer spawn. The critical path
shortened by luck. A 3.3% move in the other direction would have been equally
correct — avoiding a threatened rock legitimately costs a longer walk — and the
thing that matters is that **no baseline refused and none moved by much**.

One cost worth stating: the oil plan's wall time went from ~93 s to ~184 s.
`threat_covering` calls `threats_from`, which is O(threats) and is now asked per
candidate tile. It is offline planning, not a run, so nothing was optimised;
but a map with hundreds of nests would want a spatial index.

## Would a bot still be sent to that rock?

No. `huge-rock` at (-259.75, 76.75) is covered by three `small-worm-turret`s at
20.4, 20.7 and 20.7 tiles — all inside the 25-tile standoff — and by a
`biter-spawner` at 27.3, inside its 50. `Chop` now passes it over while any safe
rock stands, and refuses by name if none does.

Across the whole `gathered:crude-oil` plan, **0 of 64 chop targets are inside
any standoff**, against 4 before.

## Verification

- `nix develop -c cargo test --workspace --no-fail-fast`, cargo's own exit code
  **0**, **3,088 passed / 0 failed** (baseline `a367faf9` was 3,079 — nine new
  tests).
- `cargo clippy --workspace --all-features --all-targets -- --deny warnings`
  clean. It caught `result_large_err` on the new variant; the payload is boxed,
  the way `NotCharted` already boxes its `ChartingSummary`.
- The OpenAPI snapshot seam fired as designed when `attack_range` was added and
  was regenerated. `FactorioEntityPrototype` is not declared in
  `app/src/api/types.ts`, so the TypeScript half had nothing to mirror.
- **Falsification sweep: 8 mutations, 9 tests, CLEAN.** Every test dies to at
  least one mutation; every mutation compiled (a mutation that fails to compile
  reads as green and is reported separately); every substitution was asserted to
  match exactly once. Backup by file copy, restore by copy **plus `touch`** —
  never `git checkout`. Committed before the sweep.

  One mutation came back **green** on the first run and was a finding about the
  *expectation*, not the test: shrinking the worm range 25 → 7 does not kill
  `a_tree_inside_a_worms_reach_is_passed_over_while_a_safe_one_stands`, because
  that fixture puts its tree 2 tiles from the worm — deliberately deep inside,
  since the test is about preference and not about where the boundary falls.
  The boundary is `a_position_is_covered_inside_the_standoff_and_free_outside_it`,
  which the mutation does kill.

  The negative control (`the_same_tree_is_chopped_when_the_worm_is_out_of_range`,
  which guards against over-refusing) is falsified by its own mutation —
  `threat_covering` forced to answer `Some` always. A control no mutation can
  falsify is a control that may assert nothing.

No live run. The defect is a *choice*, not an execution, so the offline plans
are the strong evidence; a run would add cost and no discrimination.

## Open

- **The mod sends no attack range.** One field in
  `mods/BotBridge/types.lua` (`attack_parameters.range`) retires the fallback
  entirely. Not done here: another agent was live in that mod.
- **A patch smaller than a worm's reach has no safe tile**, and ore then refuses
  as `NoApplicableMethod` rather than naming the worm. See above.
- **`scout.rs`'s `THREAT_STANDOFF` is still its own 50** and is not derived from
  `threat_standoff`. The two answer different questions — where to *walk to
  look*, versus where to *stand and work* — and were deliberately not merged.
  Whether they should be is an open call.
- **Threat lookups are linear.** Fine at 32 nests, not at hundreds.
