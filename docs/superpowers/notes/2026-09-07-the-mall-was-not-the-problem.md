# The mall was not the problem: a ration that only falls

2026-09-07. Session `mall-demand`, branch `the-mall-pulls-something`, off
`67f0c487`. Oracle: the 6:39:53 Space Age rocket-launch save, dumped with
`entity.status` as `workspace/wrload/scripts/wr-census-status.json` (2.94 GB,
`game.tick` 1,443,169), against the game's own ten-minute production statistics
recorded in `docs/superpowers/notes/2026-09-06-what-the-record-base-knows.md`.

Predecessors: `2026-09-07-a-machine-standing-still.md` (the supply balance) and
`2026-09-07-a-full-consumer-stops-pulling.md` (the outlet cap and the two-phase
ration).

**The brief: the two-phase ration assumes a line whose product nothing in the
model consumes has no drain at all, which is as much an extreme as nameplate;
`advanced-circuit` at 0.61x is the cap's sharp edge, because the real base
sends ~444 adv/min into modules and roboports that phase one gives zero.
Estimate what the mall really pulls.**

**The premise did not survive contact with the data, and the answer is that
the mall pulls nothing that matters here.** What was wrong is a different
thing entirely: the ration was a **monotone descent**, so a consumer that
stopped pulling released its share and *nothing was allowed to take it up* --
which is the mechanism the function's own doc comment had claimed all along.

Mean absolute log error over the fourteen items the game reports:
**0.298 nameplate -> 0.184 descent -> 0.141**.

---

## 1. The premise, falsified in one command

`advanced-circuit` is **not outlet-limited**. Freeing its outlet cap completely
-- an infinite mall pull on that one item, which is strictly more than any
estimate of the mall could ever justify -- moves it from **0.61 to 0.61**.

That is not a subtle measurement. It is now permanent, as
`what_holds_each_line_back` (`#[ignore]`d, gated on `FACTORIO_BOT_WORLD_DUMP`),
which prints what actually binds each line at the converged allocation:

```
          copper-cable  {"outlet": 126}
              iron-ore  {"at nameplate": 550}
            iron-plate  {"short of iron-ore": 515}
          copper-plate  {"outlet": 519}
    electronic-circuit  {"short of iron-plate": 98}
           plastic-bar  {"outlet": 24}
           steel-plate  {"short of iron-plate": 212}
      advanced-circuit  {"short of electronic-circuit": 138}
       iron-gear-wheel  {"outlet": 23}
       processing-unit  {"short of electronic-circuit": 48}
```

**All 138 advanced-circuit lines are short of green circuits.** Its 0.61 was
never the outlet cap's sharp edge; it was an ingredient shortage arriving from
four hops upstream. Reach for this probe before attributing an error to a
mechanism -- it is the check that was missing when the brief was written, and
writing it cost less than the first experiment it would have prevented.

## 2. What I assumed the mall pulls, and why: nothing, and it is measured

Twenty-six settings, across four families, all against the same dump on the
same binary. Every one is worse than zero, and the error grows with the size
of the prior.

| family | what the mall does | prior | MALE |
|---|---|---|---|
| baseline | nothing (shipped descent) | -- | **0.184** |
| competes | pulls ingredients *and* relaxes outlets, one ration | 0.1 / 0.25 / 0.4 / 0.5 / 0.75 / 1.0 | 1.204 / 0.665 / 0.540 / 0.536 / 0.564 / 0.610 |
| outlet only, uniform | relaxes outlets, never competes | 0.1 / 0.2 / 0.3 / 0.5 / 0.75 / 1.0 | 0.219 / 0.250 / 0.268 / 0.269 / 0.267 / 0.265 |
| outlet only, split | buildings at `b`, consumed goods at `c` | b0c.25 / b0c.5 / b0c1 / b.05c1 / b.1c1 / b.25c1 / b.1c.5 | 0.189 / 0.195 / 0.204 / 0.219 / 0.237 / 0.267 / 0.228 |
| the same seven, on the fixed point | | | 0.153 / 0.143 / 0.153 / -- / 0.185 / 0.235 / -- |

Three things worth keeping from that table, because each cost an experiment.

**A mall that competes for ingredients is a death spiral, not a correction.**
Letting the unknown-drain lines eat starves the known ones, whose reduced
output caps their own suppliers, whose reduced output rations them again: at a
prior of 0.1 the whole base collapses to a sixth of the game's rates
(MALE 1.204), *worse* than at 1.0. Non-monotonic in the prior, which is the
signature of a feedback loop rather than of a bias.

**A fractional prior turns an exemption into a tight cap, which is the
opposite of what it is for.** The outlet cap exempts an item nothing eats --
"unknown outlet, not a closed one". Give the mall a pull of 0.1 and
`processing-unit`, whose entire modelled consumption is from mall lines, goes
from exempt to capped at a tenth of nameplate: 0.82 -> 0.07. Any mall prior has
to be added to outlets that already exist and never used to create one.

**The building/consumed split is a good idea that does not pay here.** An item
that is itself a placeable entity (belts, inserters, machines) is spent on
*construction*, a stock the model has no rate for; an item that is not (science
packs, ammunition, modules, rocket parts) is spent at the rate it is made. That
distinction is derivable from `entity_prototypes` with no oracle, and it splits
the 42 unknown-drain items 32/11. It is the best of the priors -- and it is
still worse than zero, because relaxing gear's cap costs more than relaxing
advanced circuit's gains: `iron-gear-wheel` goes 0.99 -> 1.37 while
`advanced-circuit` does not move at all, for the reason in §1.

## 3. What was actually wrong: a ration that could only fall

`ration` iterated a scale map downward and stopped when nothing had fallen.
That made it converge, and it made the second half of its own sentence
impossible:

> A mall assembler with a full output chest stops its input inserters, which
> **releases its share of the belt to the machines downstream that are still
> pulling.**

The release happened. Nothing was ever allowed to climb back up and take it.
`a_line_that_stops_pulling_releases_its_share_to_one_that_has_not` is that in
seven lines: two consumers of one 10/s plate supply, one of them
output-blocked at a tenth of its nameplate. Both were first rationed to half
the plate; when the blocked one fell to 1, the other was **frozen at the 5 it
had already been written down to**, and nine plates a second went nowhere.

And the descent compounds with the outlet cap. A shortage rations every
consumer proportionally, including the ones really running flat out; their
reduced pull caps their own suppliers, whose reduced output rations them again.
On the record base the whole chain settles with **supply exactly equal to
demand for every modelled item** -- iron plate 13,781.3 made against 13,781.3
eaten, gears 573.7 against 573.7, advanced circuit 571.6 against 571.6, copper
plate 12,492.6 against 12,492.6.

That is worth stating plainly, because it also answers the brief's question at
the root: **the residual left over for anything the model cannot see is
exactly zero, by construction.** "The mall pulls nothing" was never an
assumption the previous change chose. It was forced by the ratchet, and phase
two was handing the mall a residual that the outlet cap had already guaranteed
would be empty.

So the scale is now solved as a **fixed point a line may climb back up to**:
each round recomputes every line's target from the current allocation and
steps a fraction of the way there, up or down, never above the ceiling it
started at.

### The damping is not a tuning knob, and the sweep is the evidence

| damping | 0.05 | 0.1 | 0.25 | 0.5 | 0.9 |
|---|---|---|---|---|---|
| MALE | 0.141 | **0.141** | 0.143 | 0.150 | 0.210 |

The undamped iteration converges too, and to a *worse* fixed point (0.184, the
descent's own answer): a full step overshoots downward and settles in a lower
basin. The system has many fixed points -- an outlet cap and a supply share
hold each other consistent at any level -- and the physical one is the
**greatest**, because a factory does not choose to run slower than it can.
Stepping down from the ceiling in small steps stops at the first fixed point
below it, which is that one. A halving of the step that moves the answer by
0.2% is a converged solution. Shipped at 0.1, converging in 2,406 rounds for
phase one and 4,353 for phase two against a cap of 20,000.

### Being short of a thing is not the same as not wanting it

The fixed point alone scores **0.128** -- better than what shipped. It is also
wrong, and the way it is wrong is instructive.

Charging an item's outlet at what its consumers are *currently drawing* counts
a consumer's own shortage **of that very item** as evidence that it has
stopped pulling. That makes a producer and its consumer neutrally stable at
any level: each is consistent with the other at 90% of nameplate and equally
consistent at 73%, so the answer becomes whatever the transient happened to
leave, and the fixed point stops being a property of the base at all. Measured
on the unit fixture: 7.31 where the plate supply allows 9.

So `relieved_pull` has a consumer contribute what it would run at with every
constraint it has **except** its share of this item -- its other ingredients,
its own outlet, and its ceiling. A machine genuinely output-blocked still
contributes only its reduced pull, which is the whole point of the cap; a
machine merely starved contributes what it would take.

**That costs 0.128 -> 0.141, and the trade is deliberate.** A better number
from a model whose answer depends on the path it took there is the
`copper-cable` mistake in another form: right for a reason that will not hold
the next time.

## 4. The table

Every column computed by `production_rates_of_a_dumped_world` on **one binary
on one dump**, including the before column -- `balance_by_descent` is the old
solver kept in the test module for exactly that, so nothing here is quoted
from a note.

| item | game /min | nameplate | ratio | descent | ratio | **fixed point** | **ratio** |
|---|---:|---:|---:|---:|---:|---:|---:|
| copper-cable | 22,367 | 22,680.0 | 1.01 | 19,386.3 | 0.87 | 22,379.7 | **1.00** |
| iron-ore | 15,247 | 16,500.0 | 1.08 | 16,500.0 | 1.08 | 16,500.0 | 1.08 |
| iron-plate | 15,170 | 19,016.5 | 1.25 | 13,781.3 | 0.91 | 16,494.0 | **1.09** |
| copper-ore | 15,157 | 15,870.0 | 1.05 | 15,870.0 | 1.05 | 15,870.0 | 1.05 |
| copper-plate | 15,147 | 16,579.6 | 1.09 | 12,492.6 | 0.82 | 14,345.4 | **0.95** |
| electronic-circuit | 6,694 | 8,820.0 | 1.32 | 5,700.0 | 0.85 | 6,570.5 | **0.98** |
| coal | 4,096 | 5,130.0 | 1.25 | 5,130.0 | 1.25 | 5,130.0 | 1.25 |
| plastic-bar | 2,846 | 2,880.0 | 1.01 | 1,825.1 | 0.64 | 2,102.7 | **0.74** |
| stone | 1,775 | 1,740.0 | 0.98 | 1,740.0 | 0.98 | 1,740.0 | 0.98 |
| steel-plate | 1,213 | 1,590.0 | 1.31 | 1,118.5 | 0.92 | 1,415.0 | **1.17** |
| advanced-circuit | 942 | 1,035.0 | 1.10 | 571.6 | 0.61 | 667.1 | **0.71** |
| iron-gear-wheel | 578 | 2,070.0 | 3.58 | 573.7 | 0.99 | 651.8 | **1.13** |
| stone-brick | 450 | 1,453.9 | 3.23 | 658.9 | 1.46 | 720.0 | **1.60** |
| processing-unit | 249 | 360.0 | 1.45 | 203.7 | 0.82 | 234.5 | **0.94** |
| **mean abs log error** | | | **0.298** | | **0.184** | | **0.141** |

Eight improve, three get worse, three are untouched.

### The regressions, each with its mechanism

**`iron-gear-wheel` 0.99 -> 1.13.** The largest, and the honest cost of the
change. Its 0.99 was the descent's headline win, and it came from gear being
capped at exactly what its visible consumers were drawing -- consumers who
were themselves ratcheted down. Twenty-three of its lines are still
outlet-bound; they are now bound by what those consumers would take rather
than by what the ratchet had left them at. 1.13 is a worse ratio from a
better-defined quantity, which is the same sentence the predecessor wrote
about `copper-cable`, and it is meant the same way.

**`stone-brick` 1.46 -> 1.60.** Same mechanism, one column down. Its brick
furnaces are outlet-bound (202 lines) and the outlet is now read at the
relieved pull. Brick has been over-predicted in every column this project has
produced and remains the least-explained item in the table: it was 3.23 at
nameplate, 0.70 with a fictional landfill demand, 1.46 without it, 1.60 now.

**`steel-plate` 0.92 -> 1.17.** Steel's consumers are largely short of steel
rather than output-blocked, so the relief raises steel's outlet toward its
nameplate demand and the cap stops binding.

### And `copper-cable` moved to 1.00, which is not the coincidence returning

The predecessor documented `copper-cable`'s nameplate 1.01 as a coincidence --
+12% beacon speed and +3% productivity nearly cancelling 18% idle -- and
warned that a number improving for the wrong reason is worse than one staying
wrong. It then went to 0.87 by being read off demand instead of capacity.

**It is now 1.00, and it is still read off demand.** Nothing about the beacon
coincidence returned: cable is `3 x electronic-circuit + 4 x advanced-circuit`
and it inherits those two exactly, as it did at 0.87. What moved is green
circuits, 0.85 -> 0.98, because the descent had them ratcheted below what the
iron plate actually available supports. Cable is 1.00 because green is 0.98
and green is 0.98 for a reason this note can state. That is the mechanism, and
it is the check the warning asks for -- but it lands on 1.00 rather than 0.98
by the same arithmetic as before, so **do not read cable's agreement as
validation of anything.** Green circuits and iron plate are the columns
carrying the claim.

## 5. What this is still not, and one defect found on the way

**Nothing here reads `FactorioEntity::status`.** The census's 21.7% of
assemblers reading `full_output` validates the prediction and is never an
input to it: a plan is scored on machines that do not exist yet.

**The prediction against that observable, and it does not flatter the model.**
`what_holds_each_line_back` counts, at the converged allocation, how many
lines are held by their outlet rather than by an ingredient:

```
980 of 2173 crafting lines are outlet-bound (45.1%), out of 3722 lines in all
```

Against **21.7%** of assemblers observed `full_output` at tick 1,443,169, the
model says output-blocking is about **twice as common as it is**. That is a
real over-prediction, it is the first time this quantity has been comparable
at all -- the descent's answer was not, since it capped things at a level that
had itself been ratcheted -- and it is the obvious next thread to pull.

Three reasons the two are not quite the same measurement, and they do not
close a factor of two between them: a *line* is a machine-product pair, so
2,173 crafting lines cover roughly 2,042 crafting machines; furnaces are in
the model's denominator and not in the census's assembler row, and the census
puts furnaces at only 5.9% `full_output`, which pulls the observed figure
*down* rather than up; and a snapshot is not a duty cycle -- the furnace
fleet's own bound puts the instant about 6% high in relative terms against the
ten-minute integral, which again cuts the wrong way for the model. **45.1%
against 21.7% is a gap to explain, not a validation.**

**A real defect, found and deliberately not fixed here.** `recipe_making`
charges `plastic-bar` to **`bioplastic`** -- bioflux and yumako-mash, neither
of which anything on Nauvis makes -- so the model never charges plastic for
the coal and petroleum gas it really takes. This is the `casting-iron` trap
from the predecessor note, surviving in a second place: the tie-break prefers a
candidate whose ingredients the base *all* supply, and `plastic-bar` fails it
because `petroleum-gas` has no modelled producer, so it falls through to
alphabetical order and `bioplastic` sorts first. Confirmed in the demand probe,
which shows the base "wanting" 960 bioflux/min and 3,840 yumako-mash/min it
makes none of. Fixing it needs the tie-break to prefer the candidate with the
*most* supplied ingredients rather than all of them. It is left alone here
because it moves nothing in this table -- coal is in surplus 5,130 against 630
eaten -- and mixing it into a change about the solver would make both harder to
attribute.

## 6. What is in the tree, and what was verified

`crates/core/src/graph/flow_graph.rs` only. No planner, mod, `types.rs` or
frontend change; nothing outside the file was touched.

- `ration` is a damped fixed point rather than a monotone descent.
- `supply_ratio` and `relieved_pull`, both pure and both new.
- `RATION_DAMPING`, `RATION_TOLERANCE`, `RATION_ROUNDS`, each with its
  measurement in its own doc.
- `production_rates_of_a_dumped_world` now prints the ratio table and the mean
  absolute log error itself, with `balance_by_descent` beside it so the before
  column is computed rather than remembered.
- `what_holds_each_line_back`, `#[ignore]`d: §1.
- The four `balance` unit tests now assert to 1e-9 rather than exactly, since
  a fixed point is reached in the limit. The tolerance is a thousand times the
  convergence residual and a billionth of any rate they assert.

Six mutations, each with the substitution **counted and confirmed to occur
exactly once** in the file before it was applied, and applied one at a time:

| mutation | breaks |
|---|---|
| make the ration a monotone descent again | `a_line_that_stops_pulling_releases_its_share_to_one_that_has_not` (alone) |
| charge the outlet at what consumers currently draw | `a_line_that_stops_pulling_releases_its_share_to_one_that_has_not` (alone) |
| drop the outlet cap | `a_producer_is_held_to_what_its_consumers_can_take` (+1) |
| cap the ground too | `the_ground_is_not_capped_by_the_customers_the_model_can_see` (+2) |
| treat an unseen outlet as a closed one | `an_item_nothing_here_eats_has_an_unknown_outlet` (+3) |
| let an unknown drain compete in phase one | `an_unknown_drain_does_not_displace_a_known_one` (+3) |

Every test is broken by the mutation of its own subject. **The first two
mutations break the same single test, and that is a real gap rather than a
tidy result**: the relief has an effect only inside a fixed point, so a
fixture separating "solved as a fixed point" from "relieved" is not
constructible -- any case where the relief matters is a case where the descent
already gives a third answer. It is one test holding two claims, and it holds
them by asserting the exact value 9 rather than "more than 5", which
distinguishes the descent's 5, the unrelieved fixed point's 7.31 and the
correct 9.

Three mutations break neighbours as well, which is over-coverage rather than a
hole. The first sweep of substitutions caught something else worth recording:
three of the six matched **twice**, because the test module's own
`balance_by_descent` copy and the new probe contain the same lines. A
substitution counted before it is applied is what turned that into a
re-anchoring rather than a false result.

Verification, with the exit code taken from the command and not from a
pipeline:

- `nix develop -c cargo test --workspace` -> exit 0, **2,846 passed, 0 failed**
  across 104 result lines.
- `cargo clippy -p factorio-bot-core --all-targets -- --deny warnings` clean.
- `rustfmt --edition 2024` on the one file.
- **The four offline baselines are byte-identical** on a release binary built
  from this branch: `researched:automation` 176 / 21,784,
  `producing:automation-science-pack:6` 316 / 22,457,
  `producing:logistic-science-pack:6` 441 / 47,478, and `gathered:crude-oil`
  2,115 / 317,283 on `map-31337-explored.json`, with the refusal on `map.json`
  intact. **They prove only that nothing regressed**: the flow graph still has
  no planner caller, so not one of them exercises a line of this change. The
  record base and the unit fixtures are the only things that do.
