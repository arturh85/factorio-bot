# Costs change as research lands — measured first

2026-09-06. Branch `costs-change-as-research-lands`, worktree
`.worktrees/gettingfaster`. Every offline plan below was made with **one
binary**, built in that worktree at commit **`a9eb0a3f`** ("Merge master
before landing block-earns-electronics") as
`nix develop -c cargo build --release --no-default-features --features cli,lua`.

---

## The number

**On the two goals this project actually plans, the frozen-cost error is
exactly zero ticks — 0.0%. Not small: zero.**

Not because the planner gets it right, but because neither
`researched:automation` nor `producing:logistic-science-pack:6` researches
any technology that changes any rate the planner models. **This is a latent
bug, not an active one.** No number this project has ever quoted is wrong
because of it.

It stays zero the moment it stops being latent, too, and that is the finding
that matters more than the measurement: **even a plan that *did* research
`steel-axe` would not get faster.** `Effect::Researched` reaches
`PlanState::set_researched`, which inserts a name into a `BTreeSet` and
nothing else (`crates/planner/src/state.rs:4922-4924`), while
`PlanState::manual_mining_speed_modifier()` reads
`self.base.forces[force].manual_mining_speed_modifier`
(`crates/planner/src/state.rs:2009-2019`) — and `base: Arc<FactorioWorld>`
(`state.rs:927`) is exposed only through `fn base(&self) -> &Arc<…>`
(`state.rs:1971`) and is never mutated anywhere in the crate. The one rate
modifier the model carries is a property of the **snapshot**, never of the
plan.

And it could not be otherwise, because the datum is thrown away one layer
further out: `mods/BotBridge/types.lua:269` keeps a technology's effects only
`if effect.type == "unlock-recipe"`. `FactorioTechnology`
(`crates/core/src/types.rs:1135-1162`) therefore has `unlocked_recipes` and no
other effect at all. **The planner cannot know that `steel-axe` grants
`character-mining-speed +1`, because nothing ever told it.**

### What it would be worth, if it were not zero

Counterfactual, same binary, same dump, one byte different (§ *How*): set the
force's `manual_mining_speed_modifier` from `0.0` to `1.0` — exactly what
`steel-axe` grants — and re-plan. This is the **optimistic bound**: it is the
plan you would get if the research were free and complete at tick 0.

| goal | makespan, base | makespan, modifier=1 | Δ makespan | Δ planned bot-ticks |
|---|---|---|---|---|
| `have:iron-ore:200` | 6,253 | 3,253 | **−48.0%** | −48.0% |
| `have:coal:200` | 4,758 | 4,218 | −11.3% | — |
| `researched:automation` | 21,784 | 21,645 | −0.6% | **−20.5%** (35,871 → 28,507) |
| `producing:logistic-science-pack:6` | 47,542 | 42,935 | **−9.7%** | −5.7% (135,818 → 128,058) |
| `researched:steel-axe` | 89,612 | 80,321 | −10.4% | — |
| `researched:steel-axe` + `have:iron-ore:400` | 89,612 | 84,140 | −6.1% | **−29.6%** (141,735 → 99,839) |

**The error does not grow monotonically with plan length**, which the brief's
consequence 1 predicted and this table does not support. `researched:automation`
is the shortest goal and moves 0.6% on makespan while moving 20.5% on bot-ticks;
green is nearly the reverse. What actually drives the two columns apart is that
**makespan is a critical path and mining is often not on it** — four bots with
50,000 idle ticks each absorb a mining slowdown without the makespan noticing.
Any future work here should quote both columns; quoting only makespan would have
called `researched:automation` insensitive when a fifth of its work is
mis-costed.

### Where the ceiling comes from

The only durations that respond to the modifier are `mine` and `chop`
(`mining_ticks`, `crates/planner/src/method/util.rs:183`, which divides by
`character_mining_speed`, `util.rs:159`). Their share of each plan's summed
action durations, from `--steps` output:

| goal | mine+chop ticks | all action ticks | rate-sensitive share |
|---|---|---|---|
| `researched:automation` | 9,372 | 22,632 | 41.4% |
| `producing:logistic-science-pack:6` | 36,972 | 97,792 | 37.8% |
| `researched:steel-axe` | 30,612 | 61,322 | 49.9% |

So **roughly 40–50% of a plan's costed work is rate-sensitive today**, and a
technology that exists in vanilla halves it. The exposure is real; only the
trigger is absent.

### Why the shipped goals are at zero — three independent checks

1. **Position in the schedule.** `researched:automation` contains exactly one
   research action, `#12 research automation` at ticks 15,784–21,784, and
   21,784 *is* the makespan. Nothing follows it. Green contains two,
   `#317 research automation` (28,942–34,942) and
   `#145 research logistic-science-pack` (34,942–46,342) of a 47,542 makespan.
2. **The technologies' own effects.** Parsed from
   `workspace/server/data/base/prototypes/technology.lua` (base 2.1.17,
   `workspace/server/data/base/info.json:3`): `automation` (line 1914) and
   `logistic-science-pack` (line 1960) carry `unlock-recipe` effects and
   nothing else; `steel-processing` (line 1983) likewise. The one technology
   in the early tree with a rate effect is `steel-axe` (line 2007):
   `{ type = "character-mining-speed", modifier = 1 }` at lines 2012-2015,
   `research_trigger` craft 50 `steel-plate`.
3. **The archive.** All 21 directories under `workspace/runs/` were scanned for
   research action labels. Every run that researched anything researched only
   `automation` and/or `logistic-science-pack`; `grep -rl
   "steel-axe\|character-mining-speed\|manual_mining_speed" workspace/runs/`
   returns nothing. **No archived run has ever contained a rate-changing
   research**, so the planned-vs-executed ratio in `tools/run_analysis.py`
   cannot measure this effect — the effect is not present in any record.

That third point is why I did not use the archived-run route the brief offered
first. The ~0.97–1.18 planned/executed ratio is real, but it is measuring
something else: the largest documented single contribution to it is a power
ordering defect that put two researches 13,000 ticks over their planned
durations (`crates/planner/src/state.rs:4013-4022`), which has nothing to do
with rates.

### Ordering, measured

Consequence 2 of the brief — "the scheduler cannot see that ordering matters" —
is directly visible in the compound goal
`researched:steel-axe` + `have:iron-ore:400`. In that plan all 400 ore are
mined at 120 ticks each, finishing by tick ~50,869 (largest single action:
`#311 mine 100 iron-ore`, 38,869–50,869), while `steel-axe` is only earned by
the last steel plate at ~89,602. Mining after the research instead would cost
60 ticks each: **24,000 bot-ticks left on the table, on bots that are idle
50,000–60,000 ticks anyway.** The scheduler has no way to see that, because
both orderings cost the same in its model.

Note honestly that this particular reordering is makespan-neutral (89,612 both
ways) — the critical path is steel smelting. It saves roster capacity, not
wall time, on this goal.

---

## How it was measured, and why it is sound

Deterministic offline planning only. No Factorio was started, no code was
changed, no run was made. The box was at load average 21 throughout (other
agents building); **this does not touch the result**, because the quantity
measured is a planned tick count, which is a pure function of the dump, the
goal, the roster and the binary. `crates/planner` is documented pure and
deterministic, and the two arms produced byte-stable numbers across two
different binaries (see the control below).

The counterfactual dump:

```
sed 's/"manual_mining_speed_modifier": 0\.0/"manual_mining_speed_modifier": 1.0/' \
    workspace/scripts/map.json > scratch-map-axed.json
```

Four guards, following `2026-09-06-fixtures-agree-with-their-code.md`:

- **The substitution was asserted to match.** `grep -c` on the result: exactly
  `1`. (The whole 865 MB dump contains one force with that key.) An unmatched
  `str.replace` producing a reassuring green is the documented trap.
- **The break moved something.** Every one of the six goals changed its
  makespan, its action count, or both. The field is read end to end.
- **A control round-trip.** Substituting `1.0` back to `0.0` produced a file
  `cmp`-identical to the original `map.json`, and re-planning
  `have:iron-ore:200` against it returned 6,253 — the base figure. So the
  difference between the two arms is that one character and nothing about the
  copy.
- **An independent oracle.** `iron-ore` has `mining_time = 1`
  (`workspace/server/data/base/prototypes/entity/resources.lua:82`) and the
  character has `mining_speed = 0.5`
  (`…/prototypes/entity/entities.lua:976`), so 200 ore across 4 bots is
  200 × (1/0.5) × 60 / 4 = **6,000 ticks** of mining, and half that at
  modifier 1. Measured makespans 6,253 and 3,253 — a 3,000-tick delta and a
  253-tick walking remainder that is identical in both arms. The arithmetic
  was derived from prototype data, not from the planner.

**What the experiment does not show.** The two arms are not always the same
plan: green expands to 442 actions at modifier 0 and 537 at modifier 1, and
even researches a different set (17,400 research ticks against 13,500). A
cheaper mining rate changes what the expansion chooses, so the makespan deltas
are "plan under rate A versus plan under rate B", not "the same plan re-costed".
`have:iron-ore:200`, `have:coal:200` and the compound goal keep their action
counts within one and are the cleaner rows.

Scratch dumps were deleted; the two `sed` lines above regenerate them.

---

## Part 2 — what actually changes with research or modules, and what we model

Read from `workspace/server/data/base/` at version **2.1.17**
(`base/info.json:3`). Only the `base` mod was consulted; `space-age`,
`quality`, `elevated-rails` and `recycler` exist as siblings and were **not**
scanned — if any is enabled, tiers not listed here exist. All line numbers are
in `workspace/server/data/base/prototypes/`.

### The engine's rate-changing modifier types, and which base technologies use them

From the `ModifierType` union in `workspace/factorio-api-docs/runtime-api.json`,
crossed against a full `type =` census of `technology.lua`:

| modifier type | used by base technologies? |
|---|---|
| `character-mining-speed` | **yes** — 1 technology |
| `laboratory-speed` | **yes** — 6 |
| `inserter-stack-size-bonus` | **yes** — 2 |
| `bulk-inserter-capacity-bonus` | **yes** — 8 |
| `mining-drill-productivity-bonus` | **yes** — 4 |
| `worker-robot-speed` | **yes** — 6 |
| `gun-speed` | yes — 26 (combat, not factory) |
| `train-braking-force-bonus` | yes — 7 |
| `character-crafting-speed` | **no** — 0 hits in `technology.lua` |
| `character-running-speed` | **no** — 0 hits |
| `laboratory-productivity` | **no** — 0 hits |
| `change-recipe-productivity` | **no** — 0 hits |
| `beacon-distribution` | **no** — 0 hits |

**There is no vanilla technology that changes hand-crafting speed.** That
matters here: `recipe_ticks` (`util.rs:737`) costs a hand craft at speed 1 and
is *correct and permanently correct* in vanilla base. Hand crafting is 40% of
the green plan's action ticks and is not part of this problem at all.

### The specific numbers

**Character mining.** `steel-axe`, `character-mining-speed +1`,
`technology.lua:2007-2023` (effect at 2013), prerequisite `steel-processing`,
`research_trigger` craft 50 steel-plate. It is the **only**
`character-mining-speed` effect in the file. Character base `mining_speed = 0.5`
(`entity/entities.lua:976`).

**Research speed.** `research-speed-1..6`, `laboratory-speed`
+0.2/+0.3/+0.4/+0.5/+0.5/+0.6, `technology.lua:2302, 2324, 3618, 3643, 3668,
3694`. `lab` prototype `researching_speed = 1` (`entity/entities.lua:3913`).

**Machine tiers** (`crafting_speed`, fixed per prototype, no technology changes
one): `stone-furnace` 1 (`entities.lua:1076`), `steel-furnace` 2
(`entities.lua:4785`, unlocked by `advanced-material-processing`,
`technology.lua:2384`), `electric-furnace` 2 (`entities.lua:4332`),
`assembling-machine-1` 0.5 (`entities.lua:3186`), `-2` 0.75
(`entities.lua:3264`), `-3` 1.25 (`entities.lua:5463`). Drills:
`burner-mining-drill` `mining_speed` 0.25 (`entity/mining-drill.lua:1800`),
`electric-mining-drill` 0.5 (`mining-drill.lua:1749`).

**Inserters.** Swing is `rotation_speed`/`extension_speed`, fixed per tier:
burner 0.013/0.035 (`entities.lua:2649-2650`), `inserter` 0.014/0.035
(`2327-2328`), long-handed 0.02/0.05 (`2530-2531`), fast 0.04/0.1
(`2432-2433`), bulk 0.04/0.1 (`5506-5507`). Capacity is *not* a prototype field
in this version — it is 1 (2 for `bulk-inserter`, `bulk = true`,
`entities.lua:5480`) and scales through eight technologies:
`bulk-inserter` and `inserter-capacity-bonus-1..7`, `technology.lua:1697, 1724,
1749, 1778, 1804, 1831, 1858, 1885`, reaching stack-size 3 and bulk capacity 12.

**Belts** (`transport-belts.lua`, `speed` in tiles/tick, no technology changes a
built belt): transport 0.03125 (`:180`), fast 0.0625 (`:216`), express 0.09375
(`:252`), with matching underground and splitter tiers. **No
`turbo-transport-belt` in base** — the brief's 0.125 tier is a Space Age
prototype and was not found here. *Unverified:* the exact technology names
unlocking the fast/express tiers were not traced (they are `unlock-recipe`
entries in the logistics chain, not modifier effects).

**Modules** (`item.lua`, static per prototype; technology only unlocks the
recipe): `speed-module` speed +0.2 / consumption +0.5 (`:2542`), `-2` +0.3
(`:2565`), `-3` +0.5 (`:2589`); `productivity-module` productivity +0.04 and
**speed −0.05** (`:2686`), `-2` +0.06 / −0.1 (`:2708`), `-3` +0.1 / −0.15
(`:2732`); `efficiency-module` 1/2/3 consumption −0.3/−0.4/−0.5 (`:2613,
2637, 2661`). Beacon: `distribution_effectivity = 1.5`,
`supply_area_distance = 3`, `module_slots = 2` (`entities.lua:7701, 7682,
7705`), unlocked by `effect-transmission` (`technology.lua:3796`).

**Mining productivity.** `mining-productivity-1..4`,
`mining-drill-productivity-bonus` +0.1 each, level 4 infinite with
`count_formula = "2500*(L - 3)"`, `technology.lua:5222-5316`. This is extra
*output per operation*, not a faster swing — a different shape of the same
problem, and one the planner would feel as a smaller bill rather than a shorter
duration.

### What the model captures — the confirmation the brief asked for

The brief's finding is confirmed and is worse than stated.

| what changes a rate | does the world model carry it? | does the planner read it? |
|---|---|---|
| `manual_mining_speed_modifier` (force) | **yes** — `FactorioForce`, `crates/core/src/types.rs:1197` | **yes** — `character_mining_speed`, `util.rs:159`, into `mining_ticks`, `util.rs:183` |
| a machine's `crafting_speed` | yes — `FactorioEntityPrototype`, sent at `mods/BotBridge/types.lua:432` via `get_crafting_speed()` | yes — `machine_crafting_speed`, `util.rs:777`, into `smelting_ticks`, `util.rs:800` |
| an entity's `mining_speed` | yes — `mods/BotBridge/types.lua:419` | only as `character.mining_speed`; no drill rate is costed |
| **every other technology effect** | **no** — `types.lua:269` keeps `unlock-recipe` only; `FactorioTechnology` has no effects field (`types.rs:1135-1162`) | — |
| `laboratory-speed` / lab `researching_speed` | no | no — `research_ticks_in_labs` (`util.rs:1059`) is `unit_energy * ceil(count/labs)`, with no speed term at all |
| inserter capacity or swing | no | no — no planner file mentions `rotation_speed`, `extension_speed` or any stack bonus |
| belt tier speed | no | no — `method::connect` places belts but costs each at the flat `PLACE_TICKS = 30` (`method/have.rs:368`) and models no throughput |
| modules, beacons | **no** — the strings "module" and "beacon" appear nowhere in `crates/planner/src` or `crates/core/src/types.rs` as game concepts | no |
| mining productivity | no | no |

`FactorioForce` has exactly six fields (`types.rs:1176-1199`) and exactly one is
a rate modifier. The brief's alarm is correct.

Two more places where a rate is a hard-coded constant rather than data, found on
the way and worth knowing about:

- `crates/core/src/graph/flow_graph.rs:712-721` hard-codes smelting rates
  (`1/3.2` for iron/copper/stone, `1/16` for steel) as literals rather than
  reading `recipe.energy`. It happens to be right for 2.1.17 and would silently
  survive a rebalance. `crates/planner` does not use `FlowGraph` at all.
- `COAL_BURN_TICKS = 2666` (`method/have.rs:362`), `DRILL_BURN_TICKS = 1600`
  (`method/produce.rs:109`), `CELL_CHARGE_TICKS = 9_000`
  (`method/assemble.rs:203`), `PLACE_TICKS = 30`, `TRANSFER_TICKS = 10`. These
  are fixed by fuel value and by the executor's own round-trip cost, not by
  research, so they are out of scope — but they are the reason the *unmodelled*
  half of a plan's duration is as large as it is.

---

## Part 3 — design. Options, failure modes, one recommendation

The circularity the brief names is real: costing an action against the state at
the time it runs makes `duration` a function of schedule position, and the
schedule is chosen from the durations. But the measurement above changes which
problem is worth solving first, so the options are ordered accordingly.

### Option 0 — carry the effects, and do nothing else yet

**Not on the brief's list, and it is the precondition for every option that
is.** Today the planner is structurally unable to know that `steel-axe` changes
anything, because `mods/BotBridge/types.lua:269` filters the effects list to
`unlock-recipe`. Widening that filter to carry the modifier effects into
`FactorioTechnology` — name, type, value, and for `gun-speed`-shaped ones the
ammo category — is a data change with no scheduling consequence at all. Every
other option below is impossible without it, and none of them is possible
*with* only it, so it is separable, testable on its own, and cannot regress a
plan.

Failure mode: none that changes a number, but it grows the world snapshot and
the OpenAPI snapshot seam, and `FactorioTechnology` derives `Hash`/`Eq`, so the
effect value needs `R64` like its neighbours.

### Option A — fixed-point iteration

Expand, schedule, re-cost each action against the state at its scheduled start,
re-schedule, repeat until the schedule stops changing.

**Failure modes.** Termination is not free: re-costing shortens actions, which
moves research earlier, which shortens more actions — a descending sequence
that *usually* converges, but "usually" is not a determinism argument, and the
crate's contract requires one. It can also cycle between two schedules of equal
makespan that differ in bot assignment. A hard iteration cap with a
deterministic tie-break (keep the lexicographically smallest schedule among
those of minimal makespan) makes it terminate, but then the answer depends on
the cap, which is a constant nobody has measured — precisely the shape this
repository has been burned by three times in a day.

**And the cost is paid on every replan.** A green expansion is ~4.5 s release;
runs replan four to seven times. Three iterations would be 15–30 s per replan,
paid when things are already going wrong.

### Option B — cost against the state after all research the plan commits to

Optimistic and consistent: expand once, collect the technologies the network
already contains, apply their modifiers, cost everything at the post-research
rate.

**Failure mode.** It is exactly the second column of my table — the
`modifier = 1` arm — and it is wrong in the *unsafe* direction. A plan that
mines 400 ore before earning `steel-axe` would be costed as if all 400 were
mined after, understating by 24,000 bot-ticks. **The current model errs
pessimistic; this option errs optimistic.**

How bad that is depends on what reads the number, and I checked rather than
assumed. `Action::duration` is **not** an executor deadline: the executor waits
on per-action completion signals, and `duration` reaches it only as a recorded
estimate in the log (`crates/executor/src/log.rs:118-130`, `:842`
`planned_duration`, explicitly "the estimate, not a measurement of it"). What
*is* a deadline is a **lag edge** — `finish(pred) + lag`,
`crates/executor/src/run.rs:1083-1156` — and lag is machine time, which for a
furnace comes from `smelting_ticks` and therefore *is* rate-derived
(`method/have.rs:2464`). So under B an optimistic `duration` costs a wrong
gantt and a wrong `planned` column, which is tolerable; an optimistic **lag**
would dispatch a take before the furnace finished, which is not. Any option
that re-costs must treat `duration` and `lag` differently, and this is the
sharpest reason to keep the two apart.

B is also self-referential in a way that matters: whether the plan commits to
`steel-axe` depends on whether `steel-axe` looks worth it, which depends on the
costs.

### Option C — two-phase: schedule once, re-cost, re-order once

Expand and schedule at frozen costs. Then walk the schedule in order,
maintaining a running modifier state, re-cost each action against the state at
its own start, and re-schedule once with those durations. Stop.

**Failure modes.** Bounded and honest, but the second pass's ordering can
invalidate its own costs — an action that moved earlier is now costed at a rate
that no longer applies at its new position. The result is not a fixed point and
should not be described as one; it is "one Newton step". Whether one step
captures most of the available saving is unmeasured.

**But this is the shape the codebase already has.** `plan_best`
(`crates/planner/src/lib.rs:104-152`) already builds a plan under each of an
enumerated set of policies, keeps the smallest makespan, and terminates via a
stated proof rather than a cap. A two-arm version — one arm at frozen costs,
one at re-costed — reuses that machinery exactly, and its cost is one extra
expansion, which is a number the project already pays and has measured.

### Option D — leave durations static, disclose the error

Emit a `PlanReport` field: "this plan contains research granting *X*, and *N*
ticks of rate-sensitive work are scheduled after it; the plan is overstated by
up to *M* ticks."

**Failure modes.** It fixes nothing the owner asked about — consequence 3,
investment, remains inexpressible. But it is the only option that is *certainly*
correct, costs one pass over the schedule, cannot regress a plan, and turns an
invisible modelling gap into a number in every run record. And it is the thing
that would have let this note be written from the archive instead of from a
counterfactual.

### Recommendation

**Ship Option 0, then Option D, and hold A/B/C until D produces a non-zero
number on a real goal.**

The reasoning is the measurement. The error today is **zero**, on every goal we
plan and in every run we have archived. Building a fixed-point re-coster, or
even a two-phase one, is paying expansion time on every replan, and adding a
termination argument to a crate whose determinism is load-bearing, to correct an
error that does not currently exist. Option 0 makes the effects visible; Option
D makes the error countable; together they cost no scheduling risk and they turn
"nobody knows" into a field in `PlanReport`.

Then, when a goal that actually researches a rate bonus is planned — anything
reaching `steel-axe`, or any goal deep enough to want `research-speed-1` — D
reports the magnitude, and that number decides between C and nothing. My
expectation, from the table above, is that C is worth it once a plan's
post-research rate-sensitive work exceeds roughly 10% of its bot-ticks, and
that this happens well before a rocket.

**What would falsify this recommendation:**

- **A goal we plan today whose plan does contain a rate-changing research with
  substantial work after it.** I checked `researched:automation`,
  `producing:logistic-science-pack:6`, `researched:steel-axe`, and a compound
  steel-axe goal. If a rocket-directed goal (or `producing:` anything past
  green) puts `steel-axe` or `research-speed-1` mid-plan with 20%+ of the
  bot-ticks after it, D's disclosure is too slow and C should be built first.
- **A demonstration that one re-costing step captures most of the saving.**
  Option C's weakness is that nobody knows whether one step is enough. Build the
  two-arm `plan_best` variant, measure both arms on the compound goal, and if
  the re-costed arm lands within a few percent of the modifier=1 bound, C is
  cheap and proven and beats waiting.
- **A measurement that the extra expansion is cheap on a replan.** If a second
  arm costs under a second on green, the argument from replan cost dissolves and
  C becomes nearly free.
- **Evidence that an optimistic `duration` is harmless in practice.** I showed
  above that `Action::duration` is not a deadline, only a recorded estimate, and
  that lag edges are the real deadlines. If a run demonstrates that an
  optimistic `duration` with honest lags costs nothing but a wrong gantt, B
  becomes the cheapest option by a wide margin and this recommendation should
  change. I did not test that; **unverified**.

### Two constraints any option must respect

- `Action::duration` is a serialized `Ticks` on a struct the executor and the
  record both read (`crates/planner/src/action.rs:817-818`), and durations are
  also accumulated into a per-bot ledger during expansion
  (`PlanState::note_planned_ticks`, `method/mod.rs:1134`,
  `state.rs:4462-4472`). A re-costing pass has to rewrite that ledger too or the
  `planned` column in every report silently disagrees with the schedule.
- Determinism. Any iteration needs a stated termination proof of `plan_best`'s
  kind, not a cap.

---

## Defects found, not fixed

Written down as the brief asked, none touched.

1. **The mod discards every technology effect but `unlock-recipe`**
   (`mods/BotBridge/types.lua:269`). This is the root cause of everything above
   and is invisible from the Rust side, where `FactorioTechnology` simply has no
   effects field to be empty.
2. **`research_ticks_in_labs` (`crates/planner/src/method/util.rs:1059`) has no
   speed term.** It is `unit_energy * ceil(units / labs)` and ignores both the
   `lab` prototype's `researching_speed` (1 in vanilla, so currently harmless)
   and `laboratory-speed`, which six base technologies grant. This is the same
   defect as the mining one and is not documented in that function the way
   `machine_crafting_speed` documents its own gap.
3. **`flow_graph.rs:712-721` hard-codes smelting rates** as `1/3.2` and `1/16`
   instead of reading `recipe.energy`. Correct for 2.1.17; would survive a
   rebalance silently. Its own test asserts the constants, so the test cannot
   catch it — a fixture agreeing with its code, in the shape
   `2026-09-06-fixtures-agree-with-their-code.md` describes.
4. **Documentation drift worth one line:** `character_mining_speed`'s doc
   (`util.rs:144-158`) explains that the modifier "moves with research", and
   `FactorioForce::manual_mining_speed_modifier`'s doc (`types.rs:1186-1189`)
   says the same — but nothing in the planner can move it, because
   `set_researched` does not touch it and `base` is immutable. Both docs read as
   if the mechanism exists.

## Reproducing this

```bash
git worktree add .worktrees/gettingfaster costs-change-as-research-lands
cd .worktrees/gettingfaster
nix develop -c cargo build --release --no-default-features --features cli,lua

sed 's/"manual_mining_speed_modifier": 0\.0/"manual_mining_speed_modifier": 1.0/' \
    ../../workspace/scripts/map.json > axed.json
grep -c '"manual_mining_speed_modifier": 1.0' axed.json   # must print exactly 1

for w in ../../workspace/scripts/map.json axed.json; do
  ./target/release/factorio-bot plan --world "$w" \
      --goal producing:logistic-science-pack:6 --bots 1,2,3,4 --steps
done
```

`workspace/scripts/map.json` must still be the seed-31337 t=0 dump, fingerprint
`c161fa3f437221d0`; `map-31337-t0.json` beside it is the same bytes.
