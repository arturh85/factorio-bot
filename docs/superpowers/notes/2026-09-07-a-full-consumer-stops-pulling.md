# A full consumer stops pulling: the demand side of idleness

2026-09-07. Session `demand-side`, branch `a-full-consumer-stops-pulling`, off
`fe4d387f`. Oracle: the 6:39:53 Space Age rocket-launch save, re-dumped with
`entity.status` as `workspace/wrload/scripts/wr-census-status.json` (2.94 GB,
`game.tick` 1,443,169, 39,237 entities in the model), against the game's own
ten-minute production statistics recorded in
`docs/superpowers/notes/2026-09-06-what-the-record-base-knows.md`.

Predecessors: `2026-09-07-a-machine-standing-still.md` (the supply balance) and
`2026-09-07-a-machine-says-what-it-is-doing.md` (the status census).

**The brief: the model charges every consumer at nameplate, so its demand is
fictional, and that is why the deepest chain items got *worse* when the supply
balance landed.** What shipped is an **outlet cap** and a **two-phase ration**,
both derived from recipes and rates alone.

---

## 1. The result

Mean absolute log error over all fourteen items the game reports:
**0.298 nameplate -> 0.216 supply balance -> 0.184 with the demand side.**

Both other columns were **re-measured by this session on this dump with this
binary**, not quoted: the `pull-always` column is `FlowGraph::ration` run once
over every line at once, which is exactly what `sustained_production_rates` did
before this change, and it reproduces the predecessor note's numbers to the
digit (654.9 for its 655, 1310.7 for its 1,311, 135.2 for its 135). That
reproduction is the evidence that the two columns are comparable at all.

| item | game /min | nameplate | ratio | pull-always | ratio | **sustained** | **ratio** |
|---|---:|---:|---:|---:|---:|---:|---:|
| copper-cable | 22,367 | 22,680.0 | 1.01 | 22,680.0 | 1.01 | 19,386.3 | **0.87** |
| iron-ore | 15,247 | 16,500.0 | 1.08 | 16,500.0 | 1.08 | 16,500.0 | 1.08 |
| iron-plate | 15,170 | 19,016.5 | 1.25 | 16,468.8 | 1.09 | 13,781.3 | **0.91** |
| copper-ore | 15,157 | 15,870.0 | 1.05 | 15,870.0 | 1.05 | 15,870.0 | 1.05 |
| copper-plate | 15,147 | 16,579.6 | 1.09 | 15,870.0 | 1.05 | 12,492.6 | **0.82** |
| electronic-circuit | 6,694 | 8,820.0 | 1.32 | 5,584.5 | 0.83 | 5,700.0 | **0.85** |
| coal | 4,096 | 5,130.0 | 1.25 | 5,130.0 | 1.25 | 5,130.0 | 1.25 |
| plastic-bar | 2,846 | 2,880.0 | 1.01 | 2,880.0 | 1.01 | 1,825.1 | **0.64** |
| stone | 1,775 | 1,740.0 | 0.98 | 1,740.0 | 0.98 | 1,740.0 | 0.98 |
| steel-plate | 1,213 | 1,590.0 | 1.31 | 1,006.7 | 0.83 | 1,118.5 | **0.92** |
| advanced-circuit | 942 | 1,035.0 | 1.10 | 654.9 | 0.70 | 571.6 | **0.61** |
| iron-gear-wheel | 578 | 2,070.0 | 3.58 | 1,310.7 | 2.27 | 573.7 | **0.99** |
| stone-brick | 450 | 1,453.9 | 3.23 | 317.1 | 0.70 | 658.9 | **1.46** |
| processing-unit | 249 | 360.0 | 1.45 | 135.2 | 0.54 | 203.7 | **0.82** |
| **mean abs log error** | | | **0.298** | | **0.216** | | **0.184** |

Six moved for the better, four for the worse, four are untouched. §5 is the
regressions; none of them is swept up.

## 2. What the demand side is, and how it is predicted without an oracle

`FactorioEntity::status` landed yesterday and says which machines are backed
up. **It is deliberately not read by any of this.** A plan is scored on
machines that do not exist yet, so a model that needs their status is an oracle
that evaporates the moment it is needed. The census validates the prediction; it
is never an input to it.

What the model can see instead is **whether anything it knows about drains a
line's output**. Two rules follow, and both are the rule the supply side
already used -- *unknown is not zero* -- applied to the other end of the
machine.

**The outlet cap.** A line making item X may not run faster than the modelled
consumption of X. That is the whole of "a full consumer stops pulling": the
chest fills, the inserters stop, the machine stops eating. Two exemptions:

- **An item nothing eats has an unknown outlet, not a closed one**, and is not
  capped. Without this the deepest item in every chain is held at zero. On this
  base that is `processing-unit` exactly: its modelled consumption comes
  entirely from lines nothing drains.
- **A machine that takes its output from the ground is capped by nothing** --
  the same exemption `the_ground_is_not_a_recipe` gives it on the input side,
  and for the same reason. A burner's fuel is not a recipe ingredient, so the
  model sees **844 coal/min eaten against a real 4,096**: capping the coal
  drills at what it can see them feed would be a factor of five wrong, measured
  rather than argued.

**The two-phase ration.** A line whose product nothing in the model consumes --
on this base 42 items, the mall and the science block: roboports at 9/min, labs
at 22.5/min, six science packs, splitters, poles, solar panels -- has an unknown
drain. Its real customer is a bot request, a lab, or a player. Charging it at
nameplate for scarce iron plate is the fiction the brief named. So known drains
are rationed first and unknown drains get the residual.

**That is not an arbitrary tie-break; it is the mechanism itself.** A mall
assembler with a full output chest stops its input inserters, which releases its
share of the belt to the machines downstream that are still pulling.

Measured, this is not a small term. Per item, demand split by whose it is:

| item | made /min | wanted by known drains | wanted by unknown drains |
|---|---:|---:|---:|
| iron-gear-wheel | 2,070 | 1,143 | **2,718** |
| pipe | 900 | 486 | **1,834** |
| iron-plate | 19,016 | 24,332 | 2,942 |
| stone | 1,740 | 3,478 | **4,500** |
| processing-unit | 360 | **0** | 206 |

70% of the demand for gears, and 79% of the demand for pipe, is from lines
nothing drains.

**The whole-surface aggregate is preserved.** Nothing here reads
`sum_incoming_edge_weights`: the per-machine duty cycle that runs from 0.02x to
1,193.65x is still dead, and `what_consumers_see_arriving_is_not_a_conserved_flow`
still guards it.

## 3. Two phases, because a scale that only falls cannot recover

The iteration is monotone decreasing, which is what makes it converge rather
than oscillate between two allocations that each look feasible. That forbids
doing this in one pass: an unknown-drain line clamped to zero early, while the
known-drain lines were still at nameplate, could never take back the residual
they freed as they scaled down.

Phase one may **ignore** the unknown-drain lines rather than merely deprioritise
them, and that is exact rather than an approximation: a line is unknown-drain
precisely because no line's ingredients mention its product, so it can supply
nothing phase one is rationing.

The residual handed to phase two is keyed on what phase one **makes**, never on
what it merely eats. A residual of `-demand` for an item with no producer would
read as "every gram is spoken for" and stop a line dead on an ingredient the
model has simply never heard of -- a fluid off another surface, say. That is the
`have <= 0` guard's meaning, and it has to survive into phase two where zero can
mean either thing.

## 4. The snapshot is not a duty cycle, and the difference is bounded

The census's own caveat: 87.7% of furnaces `working` at tick 1,443,169 is an
instant, not "furnaces were working 87.7% of the time". **It is used here only
as a prior on where the error lives -- never as a number the model consumes --
and the validation in §1 is against a time-integrated quantity**, the game's
ten-minute production statistics.

The difference can be bounded on one fleet, from two independent measurements.
Every furnace product this graph models, summed:

```
model furnace nameplate  iron 19,016.5 + copper 16,579.6 + brick 1,453.9 + steel 1,590.0 = 38,640.0 /min
game, ten-minute average iron 15,170   + copper 15,147   + brick   450   + steel 1,213   = 31,980   /min
implied time-integrated duty                                                              82.8%
instantaneous `working` at tick 1,443,169                                                 87.7%
```

**Five percentage points apart; the snapshot reads 6% high in relative terms.**
That is the size of the steady-state assumption on this fleet, and it is small.
Two things it rests on: the model's furnace nameplate is itself the quantity
under test (though on furnaces it is the cleanest -- all 1,222 read `speed_bonus`
and `productivity_bonus` of exactly 0.000), and the ten-minute window and the
instant are not the same window.

**The drill fleet goes the other way and is not clean**, which is worth saying
rather than picking the flattering half: 40,605/min of modelled drill nameplate
against 36,275/min reported gives 89.3%, above the 83.7% instantaneous -- but
the drills carry a +10% mining-productivity force bonus the model omits, which
is roughly the size of the discrepancy. Only the furnace bound should be quoted.

## 5. The four regressions, and the one that is a warning

**`copper-cable` 1.01 -> 0.87 is the one the brief warned about, and it moved.**
Its nameplate 1.01 was documented as a coincidence -- +12% beacon speed and +3%
productivity nearly exactly cancelling 18% idle -- and it is now a different
quantity entirely. Cable is no longer read off capacity at all; it is read off
demand, `3 x electronic-circuit + 4 x advanced-circuit`, and it inherits their
under-prediction exactly. A coincidence was replaced by a derived number that is
13% low. That is a worse ratio and a better model, and both halves of that
sentence are meant.

**`copper-plate` 1.05 -> 0.82 and `plastic-bar` 1.01 -> 0.64** are the same
mechanism one and two hops further up: both are now capped by what the circuit
lines take, and the circuit lines are low.

**`advanced-circuit` 0.70 -> 0.61** is the honest cost of the outlet cap's
sharpest edge. The real base sends about 444 adv circuits a minute into modules,
roboports and electric furnaces -- lines whose own drain the model cannot see,
which phase one therefore gives **zero**. Assuming the mall pulls nothing is as
much an extreme as assuming it pulls nameplate; this change swapped one extreme
for the other, and picked the one that fixes gears.

**`stone-brick` 0.70 -> 1.46 is not really a regression, and calling it one
would be the copper-cable error in reverse.** Its 0.70 came from landfill --
1,800 stone/min of pure fiction -- starving the brick furnaces of stone in the
model. Removing the fiction leaves brick reading close to its capacity, which is
honest and 46% high, rather than reading close to the game by way of an error.

**The one unambiguous win is `iron-gear-wheel`, 3.58 -> 2.27 -> 0.99.** It was
the worst item in the table in both previous columns by a wide margin, and its
outlet cap is the single largest contributor to the error reduction.

## 6. What is still missing

The residual error is now concentrated in one place: the model cannot see the
mall's real pull, and it has to choose between charging it at nameplate (the
old bug, which inflated iron demand by 1.8x) and at zero (this one, which
under-feeds `advanced-circuit`). A third answer would need an estimate of the
mall's throughput, and nothing in a dumped world supplies one.

The outer consistency question was checked and is a non-issue: feeding phase
two's *allocated* pull back into phase one's outlet converges in one round on
this base, because the leaf-mall lines mostly end at zero residual anyway.

## 7. What is in the tree, and what was verified

`crates/core/src/graph/flow_graph.rs` only. No planner, mod, `types.rs` or
frontend change; nothing outside the file was touched.

- `ProductionLine`, and `nameplate_lines()` factored out of
  `sustained_production_rates`.
- `balance()` -- pure, testable with no world -- and `ration()`, the one
  monotone fixed point both phases use.
- `production_rates_of_a_dumped_world` now prints nameplate, pull-always and
  sustained side by side, so the before column is *computed*, not remembered.
- `what_the_model_thinks_is_consumed`, `#[ignore]`d: the §2 demand split.

Four unit tests, each falsified with the substitution **counted and confirmed
to occur exactly once** in the file before it was applied, and applied one at a
time:

| mutation | breaks |
|---|---|
| drop the outlet cap | `a_producer_is_held_to_what_its_consumers_can_take` (alone) |
| cap the ground too | `the_ground_is_not_capped_by_the_customers_the_model_can_see` (+2) |
| treat an unseen outlet as a closed one | `an_item_nothing_here_eats_has_an_unknown_outlet` (+2) |
| let an unknown drain compete in phase one | `an_unknown_drain_does_not_displace_a_known_one` (+2) |

Each test is broken by the mutation of its own subject and by no other, which
is the property that matters; three mutations break neighbours as well, which
is over-coverage rather than a hole. `a_producer_is_held_to_what_its_consumers_can_take`
uses a **three-deep** chain deliberately: with a two-deep one its middle line
would be exempt under the unknown-outlet rule and the test would pass without
exercising the cap at all -- the same shape as the invented ingredient that
made `the_ground_is_not_a_recipe` vacuous on its first substitution.

Verification, with the exit code taken from the command and not from a
pipeline:

- `nix develop -c cargo test --workspace` -> exit 0, **2,749 passed, 0 failed**.
- `cargo clippy -p factorio-bot-core --all-targets -- --deny warnings` clean.
- `rustfmt --edition 2024` on the one file.
- **The three offline baselines are byte-identical** on a release binary built
  from this branch: `researched:automation` 176 / 21,784,
  `producing:automation-science-pack:6` 316 / 22,457,
  `producing:logistic-science-pack:6` 441 / 47,478. The flow graph still has no
  planner caller, so nothing should have moved, and nothing did.
