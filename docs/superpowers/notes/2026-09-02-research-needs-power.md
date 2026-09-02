# Research needs a lab, and a lab needs power — 2026-09-02

**Crate:** `crates/planner` (plus one Lua test fixture, named below).
**Blocker addressed:** rung 7 of the milestone ladder, `research automation`,
which has never once been satisfied.
**Status:** stage 1 landed. The lab is now placed, fed and required to be
powered; **nothing here builds the power**, so a world that cannot show the
supply is refused by name rather than planned. What remains is in *Stage 2*
below.

## What was wrong

Two independent defects, both in `crates/planner`, both fatal to rung 7 on
their own.

### 1. `Researched(tech)` was satisfied by crafting a lab

`Researched::expand` emitted the science-pack bill and one `Research` action
carrying `Condition::HasItem` / `Effect::LoseItem` for the packs. Nothing
placed a lab, nothing put a pack into one, and nothing said anything about
power. Run 30 (`workspace/runs/run-1788365280-15443/`) is the whole story:

- all five milestone-7 plans read `… craft 1 lab … research automation`;
- **all 17 `placed` records in the run are stone furnaces** — the lab was
  crafted and never put down;
- **`generated_kw` is 0.0 in all 541 force samples**, and no plan in any
  archived run has ever placed a generator;
- no science pack was inserted into anything;
- `samples.jsonl` shows `research: automation, progress 0.0` from tick 105,300
  to the end — **60,661 ticks, unmoved** — after which the action timed out as
  `lost` and every later `add_research` was refused because the technology was
  already the current research.

The old doc comment named the approximation and got its direction wrong: "in
the game a lab consumes them, not the bot, and nothing here yet moves packs
from a bot into a lab … the plan debits the bot, which is the conservative
direction". Debiting the bot is conservative about the *count*. It is not
conservative about whether the research can finish, and that is what the
milestone needed.

### 2. The planner was reasoning about the `enemy` force

Reported by another agent in `2026-09-02-recipe-not-enabled.md` and fixed here.
`PlanState::from_world` chose the acting force as
`base.forces.keys().min()`, on the stated premise that "every world this plans
against has a single force". `writeout_forces` emits all of `game.forces`, so
the world gains `enemy` and `neutral` at the first research completion of every
run — tick 26,449 in run 30 — and `min()` returns **`enemy`**, a force that
never researches anything. The `player` force finished
`automation-science-pack` at tick 53,485; reading `enemy`, the planner did not
believe it, and all five milestone-7 plans re-derived its trigger.

Worse, it was inconsistent rather than uniformly wrong: the mod's
`collect_recipes` hardcodes `game.forces["player"]`, so recipe gating read the
right force while technology questions read `enemy` — one `PlanState` giving
two different answers about the same game.

## What `Researched` requires now

For a technology unlocked by science packs (the trigger path is unchanged and
returns before any of this):

1. **A lab, placed.** `Have { lab, 1, Share(chain_actor) }` plus an
   `ActionKind::Place`, with the same `AreaFree` / `AtPosition` annulus /
   `HasItem` preconditions a furnace placement carries.
2. **The packs, in the lab.** One `ActionKind::Insert` per ingredient into
   `InventorySlot::LabInput`, at the lab's position, carrying the `HasItem`
   precondition and the `LoseItem` effect that used to sit on the research.
   The research action no longer holds or spends packs at all.
3. **The lab powered.** The research action states
   `Condition::Powered { pos, entity: "lab", kw: 60.0 }`, and expansion
   **refuses** with `PlannerError::ResearchNeedsPower` when the state cannot
   show that supply.
4. **The ordering, stated.** `Condition::EntityAt` orders the inserts and the
   research after the placement by inference, but no *effect* of an insert
   satisfies any *condition* of the research, so `infer_edges` cannot draw
   that edge. The method holds both ids and states it as `Step::Link`.

A lab already standing and powered is reused rather than built again, so a plan
covering three technologies places one lab, not three.

### The site

`lab_site` searches around the **nearest supplying pole**, not around the bot.
`free_area_near` reaches 12 tiles, so a bot-anchored search would only ever find
supply the bot happened to be standing in.

The site is also **taken during expansion**, by `PlanState::create_entity`, at
the moment it is chosen. `expand` returns its whole step list before
`run_steps` executes any of it, so a technology's prerequisites — `Researched`
subgoals expanded afterwards — would each call `lab_site` against a state where
the site was still empty. `military` came out of that with three
`place lab at [8.5, 8.5]` actions, only the first of which the game would
accept. Reserving it is the same thing `Mine` already does when it claims a
resource tile mid-expansion, at the same moment and for the same reason.

### The geometry

The brief asked for spacing that does not reproduce run 30's walk stalls, where
18 of 18 detections had the character pressed against a furnace on the
planner's 2-tile grid (`2 − 2×0.69921875 = 0.6015625` between boxes against a
`0.3984375`-wide character: `0.1015625` of slack per side, eight of them
measured at 1/256 of a tile).

Two things here:

- **One lab, sited by power, not by a grid.** There is no lab *array* and no
  fixed spacing to get wrong; `free_area_near`'s ring search takes the first
  site that both fits (`is_area_free`, box against box) and is supplied.
- **`tile_alignment`, new in `method/util.rs`.** A lab covers three tiles on
  each axis, so its centre belongs at a tile **centre** (`n + 0.5`), not on the
  integer grid every placement used before — that grid is right for an
  even-sized entity like a stone furnace (1.3984 tiles across, covering two)
  and wrong for an odd-sized one. Read from the prototype's own
  `collision_box`, so nothing hardcodes "a lab is 3×3", and a stone furnace's
  chosen sites are byte-identical to what they were.

## How power is modelled

`PlanState::electric_supply_kw(area) -> f64`: how much generation is wired to
whatever occupies `area`. Three steps, each load-bearing:

1. **Coverage** — which poles' supply areas overlap `area`. Overlap, not
   containment, because that is the game's rule.
2. **Connectivity** — which poles those reach transitively by copper wire, as a
   union-find. Two poles are wired when they are within the *smaller* of their
   two maximum wire distances, as the game does it.
3. **Capacity** — the generators covered by a pole in one of *those*
   components, summed.

**Coverage alone is the check that passes on a base with no generator at all**,
which is the base run 30 actually had. CLAUDE.md records why that matters and
why it is not a small error: an under-supplied network does not degrade into
"slow", it reads as completely dead. Both directions have their own test
(`a_pole_with_no_generator_supplies_nothing`,
`a_generator_with_no_pole_supplies_nothing`), and so does the third
(`a_generator_on_another_network_does_not_supply_the_site`, which also bridges
the two networks with two poles and watches the same engine start counting).

### What is deliberately not covered

- **Solar panels and accumulators are not credited.** A panel's output is a
  function of the in-game clock, so crediting it would make the same goal
  feasible or not according to when the run started — exactly the trap
  CLAUDE.md names. A solar-powered base therefore reads as unpowered: a false
  refusal, which is the safe direction. A steam engine's 900 kW is the same at
  every hour, and is what the tables carry.
- **Nameplate capacity, not throughput.** Nothing in `FactorioWorld` says
  whether a steam engine has steam, so a boiler that has run out reads as
  generating. This is the residual the function does *not* close.
- **Nothing builds the power.** An offshore pump, a boiler, a steam engine and
  the pipes between them are a subsystem of their own — shoreline geometry,
  fluid connections, pole placement — and none of it is modelled. That is
  Stage 2.
- **A 64-tile search bound.** `EntityGraph` offers no "every entity" query and
  an unbounded scan on every condition check would be a full map pass. A power
  plant beyond that radius reads as absent.
- **Two hardcoded tables** — pole supply/wire distances and generator output —
  because the mod does not send `supply_area_distance`, `maximum_wire_distance`
  or `max_energy_production`; `FactorioEntityPrototype` carries `collision_box`,
  mining and crafting fields and nothing electrical. Same discipline as
  `COAL_BURN_TICKS`, which hardcodes a stone furnace's 90 kW for the same
  reason. Sending the fields is the follow-up that deletes the tables.

## The thing that makes this a refusal rather than a plan

**`EntityGraph::add` does not admit poles or generators.** Its whitelist
(`crates/core/src/graph/entity_graph.rs`) inserts furnaces, inserters, boilers,
labs, offshore pumps, drills, tanks, containers, splitters, belts, pipes and
assemblers into the entity tree. `electric-pole` and `generator` are not on it.
They reach `blocked_tree`, so they block placements, and nothing can read their
name.

So a pole and a steam engine a *live* world already contains are invisible to
`electric_supply_kw`, which scores them zero and refuses. Entities a **plan**
places go through `PlanState::create_entity` and are visible immediately, which
is how the powered path is tested and is exactly the path a Stage-2
power-building method will take.

**This is a one-line change in `crates/core`, and it is not mine to make** —
the brief scopes this work to `crates/planner`. Adding `ElectricPole`,
`Generator` and `SolarPanel` to that match is the whole fix, and it should land
with a test that a world carrying a hand-built power plant reads as powered.

## Before and after, rung 7's fixture

`Goal::Researched("automation")`, one bot, the research fixture world.

**Before** (run 30's five plans, and what the fixture reproduced):

```
craft 1 lab
research automation          <- pre: has 10 automation-science-pack
                                eff: lose 10 …, researched automation
```

The lab is never placed, the packs never leave the bot, and the research action
is dispatched into a world with no lab and no power.

**After**, against a state that can show 900 kW:

```
Have { lab, 1, Share(bot 1) }
Have { automation-science-pack, 10, Share(bot 1) }
place lab at [8.5, 8.5]              pre: within build range (annulus), lab fits, has 1 lab
                                     eff: lose 1 lab, create lab
insert 10 automation-science-pack into the lab
                                     pre: within reach, lab at [8.5, 8.5], has 10 packs
                                     eff: lose 10 packs
research automation                  pre: lab at [8.5, 8.5],
                                          lab at [8.5, 8.5] has 60 kW of supply
                                     eff: researched automation
link: insert -> research
```

**After**, against any state that cannot — which is every live world today, for
the whitelist reason above:

```
planner::research_needs_power
automation needs a lab with 60 kW of electric supply, and the plan can show only 0 kW
help: the planner can place and feed a lab but cannot yet build a generator; a lab
      with no power researches nothing at all rather than researching slowly
```

So rung 7 now fails **at plan time, with a named reason, in seconds**, instead
of consuming 85,030 ticks and ending `stuck` behind a walk error that had
nothing to do with it. That is the stage that landed. It does not make rung 7
pass; it makes rung 7's failure true and immediate, and it is what the next
stage builds on.

## The force

`PlanState::from_world` now looks the acting force up by name:
`BOT_FORCE = "player"`, matching `crates/executor`'s `rcon_actuator::BOT_FORCE`
and the mod's `collect_player_force` / `collect_recipes`. Reproducibility was
the sort's whole justification and a key lookup keeps it — a `DashMap::get` does
not depend on the hash seed either.

The field's doc comment was rewritten rather than left in place: the sentence
asserting a single force is what made the sort look reasonable, and leaving it
would re-earn the bug.

**The two constants are deliberately not unified yet.** The only place both
crates can see is `crates/core`, which is outside this work's scope. Both carry
a test that fails if the lookup is changed back to a sort
(`the_other_forces_in_the_world_do_not_get_a_vote`, in both crates, by
coincidence of naming and not of code). Unifying them is a one-line follow-up.

A world with forces but none of them `player` now acts for **no** force and
knows no technology — refusing to plan beats planning for somebody else, which
is precisely what the sort did.

## What I tried and reverted, with the evidence

**Counting the lab in `Researched::converges`.** It is one more thing that has
to land in one pair of hands, so counting it looks right. It is not.
`automation` needs one pack type, so the lab tips it over the two-producer
threshold; a `Researched` goal that converges opens a chain **at the top of its
own subtree**; and `expand_goal_body`'s `ctx.chain.is_none()` guard then stops
every `Holder::Share` subgoal below it from opening a chain of its own — and
with it from recording its owner. That owner binding is the fix
`2026-09-02-rung-3-4-findings.md` landed for a live four-bot crash.

`the_live_four_bot_research_run_plans_and_schedules` caught it immediately: the
trigger's smelt went from one `insert 50 iron-ore` welded to `mine 42 iron-ore`
to a three-way split across the roster, with no owner on the chain. Reverting
the single `+ lab` term restored it exactly, which is what identifies the term
as the cause rather than something else in the change.

Nothing is lost by not counting it: every subgoal this method emits names
`Holder::Share(ctx.chain_actor)`, so the lab and the packs are welded to one
bot by the holder they state — a stronger guarantee than a convergence chain,
and where the ownership comes from. `converges` is for decompositions where
*nothing* names a bot. The reasoning is written at the call site and pinned by
`a_lab_that_must_be_crafted_is_not_a_convergence_on_its_own`.

## Tests

`cargo test -p factorio-bot-planner`: **356 lib + 69 integration**, from 339 +
69. Every existing makespan pin passed unchanged — `tests/red_science.rs` and
`tests/scheduling.rs` plan against `fixture_world`, which carries no forces and
so no research, and none of their numbers moved.

### Red first, with the failure output

The behaviour change was made before the fixtures were adjusted, so the first
run of the suite is the red evidence. 24 tests failed; the representative one:

```
---- method::have::tests::a_research_goal_expands_and_schedules stdout ----
automation must be reachable in the fixture world:
ResearchNeedsPower { technology: "automation", needed_kw: 60.0, supply_kw: 0.0 }
```

and the bills, before the lab subgoal was expected:

```
---- method::have::tests::a_research_asks_for_one_unit_bill_times_the_unit_count ----
  left: [Have { item: "lab", … }, Have { item: "automation-science-pack", count: 10, … }]
 right: [Have { item: "automation-science-pack", count: 10, … }]
```

and the duplicate-lab defect, found by a test written for reuse and failing for
a reason worth more than the one it was written for:

```
---- method::have::tests::a_standing_powered_lab_is_reused_rather_than_built_again ----
but only one lab, got [ … "place lab at [8.5, 8.5]" ×3 … ]
  left: 3
 right: 1
```

### Mutations, each fed back through the whole suite

Eleven, applied one at a time to the landed code and reverted after:

| # | mutation | caught by |
| --- | --- | --- |
| 1 | force back to `forces.keys().min()` | `the_other_forces_in_the_world_do_not_get_a_vote`, `a_world_without_the_player_force_acts_for_no_force`, `the_acting_force_does_not_depend_on_map_order` |
| 2 | craft the lab, never place it (run 30's shape) | `a_research_places_its_lab_feeds_it_and_waits_for_both` (+9 others, since the plan becomes infeasible) |
| 3 | never insert the packs (run 30's other half) | `a_research_places_its_lab_feeds_it_and_waits_for_both`, `a_research_asks_for_one_unit_bill_times_the_unit_count`, `a_pack_researched_technology_is_untouched_by_the_trigger_path` |
| 4 | drop `Condition::Powered` from the research | `a_research_places_its_lab_feeds_it_and_waits_for_both` (alone) |
| 5 | no power? site the lab on bare ground anyway | `research_refuses_when_the_lab_would_have_no_power`, `a_pole_with_nothing_generating_is_not_power` |
| 6 | treat coverage as capacity | `a_pole_with_no_generator_supplies_nothing`, `a_solar_panel_is_not_counted_as_generation`, `a_generator_on_another_network_does_not_supply_the_site`, `a_pole_with_nothing_generating_is_not_power` |
| 7 | count every generator in range, wired or not | `a_generator_on_another_network_does_not_supply_the_site` (alone) |
| 8 | credit solar panels at 60 kW | `a_solar_panel_is_not_counted_as_generation` (alone) |
| 9 | put every entity on the integer grid | `an_odd_sized_entity_is_centred_on_a_tile_centre` (alone) |
| 10 | do not reserve the site during expansion | `a_standing_powered_lab_is_reused_rather_than_built_again` (alone) |
| 11 | drop the stated insert → research link | `a_research_places_its_lab_feeds_it_and_waits_for_both` (alone) |

Seven of the eleven are caught by exactly one test, which is the shape that
says the test is about the thing it names.

### Determinism

`a_research_plan_is_identical_on_a_second_expansion` expands ten times and
compares the actions (id, label, preconditions, effects) **and** the schedule
(makespan and every step). All three new places determinism could have leaked
are behind it: the pole scan reads a quad tree whose query order is undefined
(`entities_within` sorts by `(x, y, name)` with `total_cmp`), the network
components come out of a union-find over that scan, and the site search's first
acceptable candidate fixes an `ActionId` allocation and therefore `schedule`'s
`(end, ActionId, BotId)` tie-break. `entities_within_comes_back_in_a_fixed_order`
pins the first of those on its own, twenty times.

### One test whose negative control I could not build

`the_supply_anchor_is_a_pole_with_generation_behind_it` asserts a `None` for a
demand of 1,000 kW against a 900 kW network. It cannot go red by construction
in the useful direction, because nothing in the crate can currently produce a
network of more than one steam engine's worth without the test building it by
hand — which is what it does. What it pins is that the `kw` argument is read at
all rather than ignored, and mutation 6 (which returns a constant 900) is what
demonstrates that the surrounding arithmetic is load-bearing.

### One test converted, and what was lost

`crates/scripting_lua/tests/goal_script.lua` planned
`goal.researched("automation")` and asserted a positive makespan, a research
action and a mine. Its world has no power and — because of the whitelist gap
above — **cannot be given any** through the world, only through a
`PlanState` overlay the Lua harness has no access to. It now asserts the
refusal instead, checking that the error names the technology and says
"electric supply".

That still proves what the test exists for (the research method is reachable
from Lua at all — it was once bound to a method that did not exist), because
the refusal is raised from inside that method. **What is lost** is the
end-to-end "a research plan reaches Lua with a research action and mining in
it" coverage. It comes back for free the day the whitelist admits poles: the
fixture world can then carry a pole and a steam engine and the original
assertions can be restored. That is the one edit outside `crates/planner` in
this change, and it is a test fixture.

## Stage 2 — what remains

In rough order of how much rung 7 needs it:

1. **Widen `EntityGraph::add`'s whitelist** to `ElectricPole`, `Generator` and
   `SolarPanel` (`crates/core`). Until this lands, a hand-built power plant is
   invisible and `Researched` refuses against every live world.
2. **A power-building method.** Offshore pump on a shoreline tile, pipes,
   boiler, steam engine, small pole — with the pole sited so its supply area
   covers the lab, and the boiler fuelled. This is the subsystem, and the
   offshore pump alone needs water-edge geometry and a direction nothing in the
   planner models. Deterministic by construction: boiler and steam engine, not
   solar.
3. **Send the electrical prototype fields** (`supply_area_distance`,
   `maximum_wire_distance`, `max_energy_production`, `energy_usage`) from the
   mod, and delete the three hardcoded tables in `state.rs` and `have.rs`.
4. **Fuel as a plan obligation.** `electric_supply_kw` counts nameplate
   capacity; a boiler with no coal reads as generating. Once a method builds
   the boiler, the coal belongs in the same bill.
5. **Unify `BOT_FORCE`** between `crates/planner` and `crates/executor`, in
   `crates/core`.
6. **A real multi-bot decomposition for `Researched`**, still undone and now
   with one more reason to want it: the lab, the packs and the placement are
   all welded to `chain_actor`.

## Verification

- `nix develop -c cargo fmt --check` — clean.
- `nix develop -c cargo clippy --workspace --all-features --all-targets -- --deny warnings` — clean.
- `nix develop -c cargo test --workspace` — green, no failures.
- `cargo fmt -p factorio-bot-planner` only; `cargo fmt --all` was not run, per
  the standing instruction about file ownership.
