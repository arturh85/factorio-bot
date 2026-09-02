# Stage 2 of the starter factory: the two things that had to be true first

**Date:** 2026-09-03. This is **not** the note that says red science is
automated. It is the note that says what stage 2 needs before it can be, what
of that is now in `master`, and — precisely — what is not.

**Read the title as a promise, not a report.** `docs/superpowers/specs/2026-09-03-starter-factory-design.md`
§14 says stage 2 is done when `Producing{automation-science-pack, 12}` holds
*and a witness shows packs accumulating with every bot idle*. Neither half is
in this change. **No assembling machine has been planned, placed or witnessed,
and nothing here has been run against a game.** Every claim below is source,
tests and the vanilla prototype definitions in `workspace/data/`.

What is here is the two prerequisites the brief singled out, each landed
red-first with its own mutation battery:

* **D5 — `Powered` is a network budget, not a per-consumer test.** Thirteen
  assembling machines on one 900 kW steam engine used to pass individually.
  They do not any more.
* **The inserter direction rule, made checkable.** An inserter's `direction`
  names the side it *picks up* from, and `PlanState::delivers_into` now models
  both ends of one so a backwards inserter is refused by the model rather than
  discovered in a dead factory.

Everything else stage 2 needs is still ahead, and §5 says what and why the
order is what it is.

---

## 1. Why stage 2 stops here and not further along

Stage 2 is an **assembling machine with a recipe set on it**, and

> **there is no `set_recipe` anywhere in this project.**

Not in `mods/BotBridge/control.lua`, not in `FactorioRcon`, not in
`ActionKind`. The 2026-09-03 spec §10 said so and it is still true: verified
again this pass by grep across `crates/` and `mods/`. An assembling machine
placed today has no recipe and does nothing — a placed-but-dead machine of
exactly the class the whole of 2026-09-02 went into eliminating.

Adding `ActionKind::SetRecipe` is a change to `crates/planner`, which this task
owns. It is **not** a change that can stand alone: `crates/executor`'s dispatch
match over `ActionKind` is exhaustive, so the variant does not compile without
an arm there, which needs an `Actuator::set_recipe`, which needs
`set_recipe_timed` in `crates/core`, which needs `rcon_set_recipe` in
`mods/BotBridge`. Three of those four crates are **report-before-editing** for
this task. So the honest shape of stage 2 is a staircase whose second step is a
cross-boundary change somebody has to agree to, and this note is the first step
plus the request.

The two things landed here were chosen because they are the two that are
(a) entirely inside `crates/planner`, (b) prerequisites rather than parts —
nothing about the cell layout can be trusted until they hold — and (c) each one
is a trap this project has already paid for once, one level down.

---

## 2. D5: `Powered` counts what is left, not what exists

### 2.1 The failure, quoted

The red, before anything was written:

```
---- action::power_budget_tests::the_thirteenth_assembler_on_one_engine_is_refused stdout ----

thread '...' panicked at crates/planner/src/action.rs:1090:9:
twelve assemblers already draw the whole 900 kW; the thirteenth has nothing
left to run on
```

Twelve `assembling-machine-1`s at 75 kW is exactly one steam engine's 900 kW.
On the old predicate the thirteenth asked "is there 75 kW of supply at this
ground", was told "900", and passed — as would the fiftieth. `Condition::Powered`
was a **per-consumer** test, and a per-consumer test is not a budget.

This is *coverage is not capacity* one level up. The comment on
`electric_supply_kw` already told the story one level down: run 30 read
`generated_kw = 0.0` in all 541 of its force samples while a coverage-only check
passed. For a single lab — the only electric consumer this planner had ever
placed — nobody could have hit the budget version. Stage 2 is the first design
with enough consumers to hit it, and the spec's own §6.1 table (three
assemblers, two drills, a lab and twelve inserters, **≈ 621 kW**) is more than
half an engine before anything is running.

**And it fails quietly.** Factorio degrades an under-supplied network
*proportionally*: 1,560 kW of demand on 900 kW of supply runs everything at
58 %, so every lag edge in the plan is wrong by 1.7× with no error anywhere.
(§15.6 of the spec lists "slow or dead?" as a run-settled question; D5 is right
either way, and this is the reading that argues it is *quiet* rather than loud.)

### 2.2 The shape

```rust
// crates/planner/src/state.rs
pub fn electric_demand_kw(&self, area: &Rect, except: Option<&Position>) -> f64

// crates/planner/src/action.rs, Condition::Powered
let headroom = state.electric_supply_kw(&area) - state.electric_demand_kw(&area, Some(pos));
headroom.total_cmp(kw).is_ge()
```

Three decisions in that, each with a test:

**One walk, not two.** The three-step coverage → connectivity → capacity walk
that `electric_supply_kw` did inline is now a private `ElectricNetwork` built
once by `PlanState::electric_network` and asked twice. Supply and demand are
therefore, by construction, about the same network. Two independent walks would
be two notions of "the same network", and a demand subtracted from a supply
computed over a *different* network is worse than no demand at all. The
generator test and the consumer test are literally the same
`ElectricNetwork::carries(&footprint)` call, which is what makes a kilowatt of
supply and a kilowatt of demand commensurable.

**Excluding self is not optional.** A consumer this plan has already placed
must not be charged against its own budget, or the second check of an identical
plan refuses what the first accepted — a non-idempotent predicate, which in a
supervisor loop is an oscillation. The exclusion matches by **tile**, through
`Pos`, because that is how `create_entity` keys the overlay and two consumers
cannot stand on one tile anyway.

**`nearest_supply_anchor` uses headroom too.** Its own doc said "somewhere a
consumer could be built and actually run", and a pole whose network is already
spoken for is not that. Nothing is excluded from the demand there, because the
consumer being sited does not exist yet. Without this the anchor would site a
consumer on a full network and `Powered` would then refuse it — a coherent
outcome, but one that spends a search to arrive at a worse error message.

### 2.3 `consumer_kw`, and the one table in that file whose unknown name errs
   the *wrong* way

`generation_kw`, `pole_supply_half_extent`, `pole_wire_reach` and
`delivery_offset` all answer `None` for a name they do not know, and every one
of them thereby **under**-credits: an unknown pole covers nothing, an unknown
generator makes nothing, an unknown machine delivers into nothing. Each refuses
a plan rather than promising one.

`consumer_kw` is the opposite and it is stated in place rather than hidden: a
consumer it does not name draws **nothing**, so an unmodelled machine on the
network is headroom that is not there. That is why the table names every
electric consumer this planner can place plus the vanilla early ones, and why
this is listed in §6 as a residual rather than treated as closed.

Every figure except the inserters is the prototype's own `energy_usage`, read
this pass off `workspace/data/base/prototypes/entity/entities.lua` and
`mining-drill.lua` (base 2.1.17) rather than from memory:
assembling-machine-1/2/3 = 75/150/375, electric-mining-drill = 90,
pumpjack = 90, lab = 60, electric-furnace = 180, chemical-plant = 210,
oil-refinery = 420, radar = 300, beacon = 480.

**Burner machines are absent rather than zero.** A stone furnace draws 90 kW
*of coal*; an entry for it here would be a number in the wrong units that every
test would agree with. So is `offshore-pump`: its `energy_usage` says 60 kW and
its `energy_source` is `type = "void"`, which is the sort of thing that reads as
a bug in whoever wrote it down from memory. It draws nothing from a network.

**The inserter figure is not a prototype field and says so.** Vanilla gives an
`inserter` a 0.4 kW idle `drain` plus 5 kJ per movement and 5 kJ per rotation.
What a budget needs is what a *busy* one costs, which is a duty cycle.
`INSERTER_DUTY_KW = 13.0` is the spec §4.2 figure, kept as one constant with the
faster inserters scaled by their own per-swing energy against this one's 5 kJ —
so there is one duty cycle in the file and not four. Budgeting the 0.4 kW drain
for an inserter that never stops swinging is the same error as counting coverage
as capacity, one table down.

### 2.4 Evidence

**Red-first, genuinely, for the headline.** `the_thirteenth_assembler_on_one_engine_is_refused`
was written against the *existing* `Condition::Powered` and failed with the
output in §2.1. The other three tests in that group were written in the same
pass and passed on the old code too — they are the "must not over-refuse"
controls, and they became meaningful only once the budget existed. Two of them
go red under mutation (below); the fourth (`a_consumer_on_another_network_spends_nothing_here`)
does not, and that is said plainly here rather than counted.

The `state.rs` tests for `electric_demand_kw` itself could not be red-first:
a test for a function that does not exist is a compile error, not a red test.

**Mutation battery.** Each applied alone to the fixed tree, the whole
`-p factorio-bot-planner --lib` suite run (430 tests), the tree restored between
each.

| Mutation | Failed |
|---|---|
| M1 the demand subtraction removed from `Powered` (the feature, absent) | `the_thirteenth_assembler_on_one_engine_is_refused` |
| M2 `except` ignored | 2 — `a_standing_consumer_is_not_charged_against_itself`, `the_consumer_asked_about_is_the_one_left_out_of_its_own_budget` |
| M3 demand ignores network membership | 2 — `a_consumer_on_another_network_spends_nothing_here`, `demand_on_another_network_is_not_counted` |
| M4 `stone-furnace` given a 90 kW **electric** draw | `a_burner_machine_spends_none_of_the_electric_budget` |
| M4b `burner-mining-drill` given 150 kW electric | the same one |
| M5 an inserter budgeted at its 0.4 kW idle drain | `the_same_state_gives_the_same_demand_twice` |
| M6 `nearest_supply_anchor` back to nameplate | `the_supply_anchor_refuses_a_network_that_is_already_committed` |
| M7 (**negative control**) `beacon` 480 → 500 kW | **nothing** — no test names a beacon and none should; this measures that the table is pinned only where it is exercised |
| M8 `Powered` compares with `>` instead of `>=` | 2 — the two tests that sit exactly on the boundary |

**M4 found a test of mine passing for the wrong reason**, which is the thing
this battery exists to catch. The first draft of
`a_burner_machine_spends_none_of_the_electric_budget` put a stone furnace, a
burner drill and a burner inserter **all at `(9.5, 11.5)`**. `create_entity`
keys the overlay by floored `Pos`, so three machines at one position are one
machine: the furnace was never in the state at all, and the mutation that gives
it a 90 kW electric draw changed nothing. The fixed version puts them on
distinct tiles, asserts all three are readable by name first, and asserts that a
`lab` dropped in the same place *is* counted — so the zero is about units and
not about absence. It now fails M4 and M4b.

---

## 3. The inserter direction rule, checked against a game rather than itself

### 3.1 The trap

CLAUDE.md, established empirically and paid for twice:

> An inserter's `direction` points at the side it **PICKS UP** from, not the
> side it drops into. Getting this backwards produces a layout that places
> 100 % correctly, passes every geometry check, and does absolutely nothing —
> the failure is silent because placement and function are separate concerns.

Stage 2 is the first design in this project that needs inserters at all. The
spec's §13 failure table rates this row 5 — *detected in the model*, but the
model rests on a hand-written table, "so a wrong table is a wrong answer that
every test agrees with".

**The way out of that circularity is not another test of the table.** It is to
check the rule against measurements somebody made in a running game.
CLAUDE.md records two, and they are independent of each other:

1. chest / burner-inserter / chest: `direction = 12` ("west") is what moves
   items *west to east*;
2. for a row fed from a belt to its north, input and output inserters are
   **both** `direction = 0`.

### 3.2 The rule

One rule, turned by one direction, and both cases fall out of it:

```rust
fn inserter_reach(name: &str) -> Option<f64>          // 1 tile, or 2 long-handed
fn pickup_offset(name, direction) = (0., -reach).turn(direction)
// delivery_offset's inserter arm     = (0.,  reach).turn(direction)
```

`Direction::West = 12` and `Position::turn(West)` is one clockwise quarter turn,
`(x, y) -> (y, -x)`. So `(0, -1)` becomes `(-1, 0)` — one tile **west** — and the
drop is one tile east. Case 1. `Direction::North` is the identity, so pickup is
one tile north and drop one tile south. Case 2. Neither case was used to derive
the rule; both are asserted against it.

`PlanState::delivers_into(from, to)` gains a second disjunct: as well as the
**push** it already modelled (`from` has a drop point that lands on a tile `to`
covers — a mining drill, and an inserter's drop side), it now accepts a
**pull** (`to` is an inserter whose pickup tile lands on a tile `from` covers).
Both readings are "the thing at `from` ends up in the thing at `to`", which is
the predicate's name. A whole chain — furnace → inserter → assembler — is
therefore two `Condition::Feeds` and needs **no new `Condition` variant**.

`pickup_position` answers `None` for everything that is not an inserter, so the
pull disjunct widens nothing else. A stone furnace does not reach out and take
from the chest beside it.

### 3.3 Evidence

**Red-first, and the output.** Four of the six tests were written before the
rule existed and failed:

```
---- state::inserter_geometry_tests::an_inserter_facing_west_moves_items_west_to_east stdout ----
thread '...' panicked at crates/planner/src/state.rs:4204:9:
direction 12 picks up from the west chest

---- state::inserter_geometry_tests::a_row_fed_from_the_north_takes_from_the_north_and_drops_to_the_south stdout ----
thread '...' panicked at crates/planner/src/state.rs:4239:9:
direction 0 picks up one tile north
```

The other two (`only_an_inserter_takes_from_the_machine_beside_it`,
`a_furnace_does_not_reach_back_into_a_drill_that_faces_away`) passed on the
unmodified tree, because with no inserter model at all every `delivers_into`
involving one is `false`. They are guards against the *new* disjunct widening
`Feeds` for everything, and only a mutation can show whether they earn their
place. One of them did not — see below.

**Mutation battery**, same discipline.

| Mutation | Failed |
|---|---|
| N1 pickup and drop **swapped** (the trap itself) | 4 — every direction test |
| N2 the pull disjunct removed | 4 |
| N3 a long-handed inserter reaches one tile | `a_long_handed_inserter_reaches_two_tiles_on_each_side` |
| N4 `covers_tile` uses the collision **box** instead of the tile | **19** — see below |
| N5 `inserter_reach` names every entity | 3 |
| N6 (**negative control**) `filter-inserter` 1 → 2 tiles | **nothing** — no test names one; the table is pinned only where it is exercised |

**N5 found a second test of mine passing for the wrong reason.** The first
`only_an_inserter_takes_from_the_machine_beside_it` used a chest and a **2×2**
stone furnace, and a 2×2 machine's own hypothetical pickup tile lands *inside
itself* — so the refusal held whatever `inserter_reach` said. The fixed version
uses a 1×1 `small-electric-pole` one tile from the chest, and first asserts that
an **inserter** in the pole's place *would* take from it, so the refusal is
about the entity kind and not about the distance. It now fails N5.

**N4 is the 1/1280 finding, reproduced at a new call site.** Factoring the tile
test out of `delivers_into` into a shared `covers_tile` created a second place
where somebody could "tidy" tile containment into box containment. A burner
drill's drop point misses a stone furnace's collision box by **0.00078125 of a
tile**, so that tidy-up rejects the one layout stage 1 exists to build: 19 of
`method::produce`'s tests go red, including `the_whole_of_stage_one_costs_this_much`
and every cell-counting test. The measurement is now guarded from both files.

---

## 4. Determinism

The pins did not move, and that is the point rather than a footnote.

* **Every existing makespan pin passes unchanged.** `red_science.rs`,
  `scheduling.rs`, `placement_occupancy.rs`, `refusal_memory.rs`,
  `tile_capacity.rs`, `tile_occupancy.rs`, `tile_reservation.rs`,
  `split_capacity.rs`, `smelt_roots.rs`, `seeded_roster.rs` and
  `produce.rs`'s own `the_whole_of_stage_one_costs_this_much` were not touched
  and did not move. The ladder's byte-identical `8/8/37/40/16/52/111` is
  untouched: the only electric consumer it places is one lab, whose demand
  excludes itself, so its headroom is the 900 kW its supply always was.
* **`electric_demand_kw` sums in `entities_within`'s fixed order**, the same
  order `electric_supply_kw` sums generation in. A sum of `f64`s is
  order-dependent, so `the_same_state_gives_the_same_demand_twice` compares
  `to_bits()` and not values.
* **`ElectricNetwork::root` does no path compression**, deliberately: `carries`
  takes `&self` so both callers can iterate `nearby` while asking. Correctness
  does not depend on compression and the construction pass has already
  flattened the tree.
* **`consumer_kw`, `inserter_reach` and `pickup_offset` are matches on a
  name**, not lookups in a hash map whose iteration order could leak.

---

## 5. What stage 2 still needs, in the order it needs it

Nothing below is done. The list is shorter than the spec's because two items
came off it today.

1. **`set_recipe`, end to end** — `ActionKind::SetRecipe` + `Effect::SetRecipe`
   + `Condition::RecipeSet` in `crates/planner`, one dispatch arm in
   `crates/executor/src/run.rs`, `Actuator::set_recipe`, `set_recipe_timed` in
   `crates/core/src/factorio/rcon.rs`, `rcon_set_recipe` in
   `mods/BotBridge/control.lua`. **Three of those five files are outside this
   task's boundary** and this is the report. Three mod-side rules the new
   command must obey, all already paid for elsewhere in that file: `rcon.print`
   is the verdict (a debug line turns a success into a reported failure — use
   `writeout`), check `LuaEntity.set_recipe`'s return value, and refuse a
   recipe the force has not unlocked **by name** rather than setting nothing and
   reporting success. `SetRecipe` is also the one new action that is
   **idempotent**, which makes it safe under `recover.rs`'s tier 1 where
   `Place`/`Insert`/`Remove` are not — worth a test asserting exactly that,
   since its neighbours are the counter-examples.
2. **The red-science cell layout** in `method/produce.rs`: two AM1s on the pack
   recipe, one on gears, three stone furnaces, two electric drills, ~12
   inserters. A rigid body of derived offsets sited by one search, the way
   `power.rs` and stage 1's cell already are. The inserter half of its geometry
   is now modelled (§3); the recipe half is blocked on item 1.
3. **A servicing lane, asserted.** Stage 1 measured the gap between its two
   machines (0.6015625, the very gap that produced 18 of 18 walk stalls, eight
   at 1/256 of a tile) and declared it unwalkable rather than claiming a lane.
   A cell with twelve inserters in it has to be refuelled, so stage 2 is the
   first design that must *state* where a bot stands and assert every lane tile
   admits a character. Not attempted here.
4. **The coal buffer.** A boiler's fuel inventory is one slot and at stage 2's
   ≈ 621 kW one stack is **5.4 minutes**. `iron-chest` (8 iron plates) plus a
   `burner-inserter` (1 plate + 1 gear) — both enabled from the start, no
   research, no electricity — turns that into ~2.9 hours. Whether a burner
   inserter refuels *itself* from the coal it moves is still unverified
   (spec §15.2); I found no prototype flag either way this pass either.
5. **A witness that can name a chain end.** `supervisor.witness` takes
   `from`/`into` — the two ends of a *single* machine-to-machine link, which is
   the whole of stage 1's shape. Red science is drill → furnace → inserter →
   assembler → inserter → assembler, and choosing which end is terminal is the
   caller's decision because only the caller knows which end the goal was
   about. Generalising it without the chain to check it against would be
   inventing an answer.

**Verified from game data this pass, not from memory** (`crates/core/tests/live-2.1.17-world-snapshot.json`,
0 of 277 technologies researched, so its `enabled` flags *are* the answer to
"available with no research"):

* `automation` — 10 × `automation-science-pack`, `research_unit_energy = 600`
  (10 s), prerequisite `automation-science-pack`, unlocks **`assembling-machine-1`**
  and `long-handed-inserter`.
* `automation-science-pack` — `research_unit_count = 1`,
  `research_unit_energy = 0`, **no unit ingredients**: a *trigger* technology,
  needing no lab. Prerequisites `steam-power` and `electronics`, both of which
  are triggers too.
* So the chain to an assembling machine is: craft 50 iron plate → `steam-power`;
  craft 10 copper plate → `electronics`; craft 1 lab → `automation-science-pack`;
  then **one** lab research of 10 red packs → `automation`. One lab research,
  not a ladder of them.
* `assembling-machine-1` is `enabled: false` at start and costs 9 iron plate,
  5 iron gear wheel, 3 electronic circuit.
* Solar is untouched and stays that way: `solar-energy` sits above
  `logistic-science-pack` above `automation-science-pack` — the lab it would
  power — and `generation_kw` still has no `solar-panel` arm.

---

## 6. Residuals, stated rather than closed

* **`consumer_kw` over-states headroom for a machine it does not name.** §2.3.
  The only real fix is the mod sending `energy_usage`, which is the same
  follow-up that deletes `pole_supply_half_extent`, `pole_wire_reach`,
  `generation_kw`, `delivery_offset` and `inserter_reach` — now **five**
  hand-written tables in one file, up from four, and the case for that change
  is a table stronger than it was yesterday.
* **A boiler with no fuel still reads as 900 kW.** `electric_supply_kw` counts
  nameplate, because nothing in `FactorioWorld` says whether a steam engine has
  steam. D5 makes the *demand* side honest and changes nothing about that.
  Spec §13 row 2, unchanged.
* **Nothing here has met a game.** The inserter rule reproduces two
  measurements CLAUDE.md made in a running game, which is the strongest check
  available without one, and it is still a check of a rule against two data
  points. The three-tile reach of a long-handed inserter, the faster inserters'
  duty figures and every `consumer_kw` entry beyond `assembling-machine-1` and
  `lab` are exercised by no test and confirmed by no run.
* **`only_ghosts = true` would validate none of this**, and neither does a
  passing test suite. A cell that assembles but cannot be witnessed producing
  is not finished; stage 2 is not finished.

---

## 7. Files

**Owned and changed:** `crates/planner/src/state.rs` (the `ElectricNetwork`
extraction, `consumer_kw`, `electric_demand_kw`, `nearest_supply_anchor`'s
headroom, `inserter_reach`, `pickup_offset`, `pickup_position`, `covers_tile`,
`delivers_into`'s pull disjunct, and 12 new tests),
`crates/planner/src/action.rs` (`Condition::Powered`'s budget and its doc, and
4 new tests).

**Nothing outside `crates/planner` was touched.** No `crates/core`, no
`crates/executor`, no `mods/`, no `app/src/`, no `scripts/`.
