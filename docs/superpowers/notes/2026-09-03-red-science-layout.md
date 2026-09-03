# The stage-2 layout: an assembler making red science

**Date:** 2026-09-03. The cell, its geometry, and how each of the three ways it
could ship dead is refused. `docs/superpowers/specs/2026-09-03-starter-factory-design.md`
§14 says stage 2 is done when `Producing{automation-science-pack, ..}` holds
**and a witness shows packs accumulating with every bot idle**.

> **RUN STATUS: the cell was never built. No witness. Stage 2 is NOT done.**
>
> **Two runs, and neither reached the cell.** Run 2
> (`run-1788399150-53956`, §4.1) halted one rung *earlier* than run 1, on the
> same wood refusal, with the power plant spending the poles. The blocker was
> never the layout; it was that this planner could not obtain wood and could
> not hand an item between bots. The first half of that landed on `master`
> while this was being written (`b0e3e12e`, `Mine` fells a tree); the second
> half has not.
>
> Live run `run-1788396958-07935`, 2026-09-03 00:54–01:20 UTC, four graphical
> clients, video recorded. Rung 1 `researched("automation")` **SATISFIED** — the
> ladder reproducing a fourth time. Rung 2 then halted at tick 92,446:
>
> ```
> HALTED: stuck -- refused: no method can satisfy goal: have 1 wood
>                           (a share sized for bot 1)
> ```
>
> **The blocker is the pole's wood, and the model's premise is the bug.** From
> the run record: bot 1 performed all 235 action dispatches and *every* craft;
> bots 2, 3 and 4 did 10–11 actions each and crafted nothing. Bot 1 dispatched
> `craft 1 small-electric-pole` **once** and `place small-electric-pole`
> **twice** — the recipe yields two poles from one wood, so that single craft
> spent bot 1's entire starting wood on exactly the plant's two poles. The cell
> needs a third. Bot 1 has none; bots 2/3/4 never touched theirs, so **three
> wood sat idle in other pockets while the plan refused for want of one.**
>
> So §1's "four bots, four wood, eight poles ever" is a cap this planner
> invented. Factorio renews wood — you mine a tree — and the pieces are already
> here: `mods/BotBridge/control.lua:1158` *already* deforests (labelled a HACK),
> `crates/core/src/graph/entity_graph.rs:43` contemplates "Mining a tree or a
> rock", and tree prototypes are in the live capture. The §1 argument that a
> second pole per cell "would halve how many cells this project can ever build"
> rests on that invented cap and does not survive it.
>
> Two things this run did prove, both previously only modelled: **zero
> teleports** across both execution batches, so the teleport removal holds under
> a real game; and a live `cannot place item 'stone-furnace' because a character
> is standing in the footprint`, which is the first live evidence for the
> `PLACEMENT_STEP_ASIDE_ACTION_ID = 4712` item.

---

## 1. What was built, and what it is general over

```
   x:  -3   -2   -1    0    1
 y 0:  [C]  >    .   #####
   1:   .   .    .   # I #        C  iron-chest        #  assembling-machine-1
   2:   .   .   (P)   v           >  inserter, picks up from the west
   3:   .   .    .   #####        v  inserter, picks up from the north
   4:  [C]  >    .   # P #       (P) small-electric-pole
   5:   .   .    .   #####
```

(The machines are three tiles across and centred on `x = 0`, so they occupy
`x = -1 .. 1`; the drawing shows their centre column. The origin every offset
is measured from is the **intermediate machine**, `I`.)

Eight buildings, six links, two recipes, one pole:

| role | what | direction (north frame) | why there |
| --- | --- | --- | --- |
| feed chest | `iron-chest` | — | holds iron plates |
| feed inserter | `inserter` | **West** — picks up from the chest | drops east into the gear machine |
| intermediate | `assembling-machine-1`, recipe `iron-gear-wheel` | — | two iron plates a gear |
| link inserter | `inserter` | **North** — picks up from the gear machine | drops south into the pack machine |
| product | `assembling-machine-1`, recipe `automation-science-pack` | — | the terminal |
| supply inserter | `inserter` | **West** — picks up from the chest | drops east into the pack machine |
| supply chest | `iron-chest` | — | holds copper plates |
| pole | `small-electric-pole` | — | one pole reaches all five consumers |

**The shape is stated over recipes, not over the name `automation-science-pack`.**
`assembly_spec` asks: is this a *crafting* recipe with exactly two ingredients,
exactly one of which is itself a crafting recipe taking a single ingredient?
Red science is the case it exists for — copper plate is *smelted*, so nothing
in an assembling machine makes one and it has to arrive in a chest, while an
iron gear wheel is one craft away from iron plates so the cell builds a machine
for it. An `electronic-circuit` is the same shape with different numbers (three
copper cables per circuit, two cables a run), and
`only_a_two_ingredient_craft_with_exactly_one_makeable_half_is_a_cell` asserts
both, plus the four ways to *not* be one.

**Why the layout is an L and not a row.** A small pole's supply area is 5×5,
and the game supplies an entity whose bounding box *overlaps* it. Laid out
along one axis the cell is eleven tiles wide and needs two poles; folded so the
two machines face each other across a one-tile gap, one pole at `(-1, 2)`
reaches all five consumers at every facing —
`the_cells_own_pole_covers_every_consumer_in_it`. That is not tidiness: a small
electric pole costs **one wood**, and *this planner* cannot make wood — `Mine`
sources only `EntityGraph::resources` and a tree is not one — so the four wood
a four-bot run starts with is all it will ever have. **That is a limit of the
model, not of the game**, and the run-status block above is right to say so:
Factorio renews wood, the mod already deforests, and teaching `Mine` to fell a
tree would lift it. Until somebody does, a second pole per cell halves how many
cells a run can build, and the cell should not be spending one it does not
need — which is exactly what §4 turned out to be about.

**Rate.** From the live capture, not from memory: `automation-science-pack` is
5 s of recipe and `assembling-machine-1` has `crafting_speed` 0.5, so one
machine is 10 s a pack — **six a minute**, not twelve. The gear machine is
nowhere near the bottleneck (1 s a gear against 10 s a pack), so
`ticks_per_item` is 600 and `cells_for` is the same integer ceiling stage 1
uses: 6/min is one cell, 7/min is two.

---

## 2. How the inserter directions were decided

**Not by reasoning about them.** The rule was landed and checked against a
running game in `7a129420` (`docs/superpowers/notes/2026-09-03-red-science-automated.md`
§3): `pickup = pos + (0, -reach).turn(direction)`, `drop = pos + (0, +reach).turn(direction)`,
one rule turned by one direction, validated against CLAUDE.md's two
measurements — `direction = 12` moves items west to east, and a row fed from a
belt to its north uses `direction = 0` at both ends.

This layout **uses** that rule and does not re-derive it. Concretely:

* the two chest inserters are `West`, so they pick up from the chest one tile
  west and drop one tile east into a machine;
* the link inserter is `North`, so it picks up from the gear machine one tile
  north and drops one tile south into the pack machine.

And then the layout does not trust its own table either. `fit` places the whole
cell on a fork and asks `PlanState::delivers_into` about all six links — the
same predicate `Condition::Feeds` is checked with, not a restatement of it — so
a cell whose geometry does not actually deliver is **refused at planning time**
rather than built. `every_link_of_the_chain_delivers_at_every_facing` asserts
all six at all four facings; a rotation wrong by a quarter turn places all eight
buildings perfectly and moves nothing.

`an_inserter_turned_round_places_perfectly_and_feeds_nothing` is the trap stated
as a difference: it turns **one** inserter by a half turn, asserts the placement
is *still legal* (`is_area_free_facing` says yes — that is the whole trap), and
asserts that exactly two links stop holding.

---

## 3. Did 620 kW hold against real demand?

**Exactly, and the cell is much smaller than it.**
`the_specs_621_kw_is_what_the_demand_ledger_says` computes the spec's §6.1 bill
through `PlanState::consumer_draw_kw` — the very table `electric_demand_kw`
sums over — and gets **621.0 kW** on the nose: three assembling machines (225),
two electric mining drills (180), a lab (60), twelve inserters at the
`INSERTER_DUTY_KW` duty figure (156).

What this cell draws is **189 kW**: two assembling machines and three
inserters. The difference is not a discrepancy, it is the scope, and it is the
thing worth reporting:

> **The spec's stage-2 cell has two electric drills and three stone furnaces in
> it, and this one has neither. It cannot.** §8.4 of the same spec anchors
> stage 2's site "to the supplying pole, exactly as `lab_site` is", and §14
> lists two drills standing on ore. Those two sentences contradict each other
> on any map where the ore is not next to the water: a drill has to stand on
> the patch, an assembler has to stand in a pole's supply area, and a small
> pole reaches 7.5 tiles. Joining them needs a **pole line** from the plant to
> the patch, which is a routing problem this planner has no primitive for.

So the smelting half of the chain is still bots: they charge the two chests by
hand. That is stated in `CELL_CHARGE_TICKS`' own doc comment and at the top of
`scripts/factory_stage2.lua` rather than left to be discovered — a green run
says *machines assembled red science with every bot idle*, which is what stage 2
is for, and it does not say *this factory runs by itself*.

Two further power facts the numbers settled:

* **189 kW leaves 711 of the engine's 900**, so one plant runs one cell and the
  lab beside it with room to spare. Two cells (12/min) is 378 kW and still
  fits; five would not.
* **The plant's five coal is the binding constraint, not the kilowatts.**
  `power::PLANT_COAL` is 5 coal = 20 MJ, which at 189 kW is **106 seconds**.
  The plant is fuelled during the research milestone and the cell is built
  minutes of game time later, so a witness run straight off the spec would have
  watched a cell standing on a dead boiler. The cell therefore tops the boiler
  up for the same window it charges its own chests, sized from the **whole
  network's** demand read off a fork with the cell standing (the lab counts),
  and capped at one stack because a boiler's fuel inventory is one slot.

---

## 4. What changed in response: the cell adopts supply it finds standing

The refusal above is a wood refusal, and there are two ways to answer it. Only
one of them is mine.

**The one that is: stop asking for the pole.** A stage-2 cell is sited *around
a supplying pole* by construction — `nearest_supply_anchor` picks the anchor and
the ring search runs from it — so a cell that lands inside that pole's own 5×5
supply area needs no pole at all. Asking for one anyway spends an item the
planner cannot replace in order to duplicate something already standing three
tiles away. That was a design error, not bad luck, and the run is what exposed
it.

So `POLE_OFFSET` left `LAYOUT` and became conditional:

* `layout(origin, facing, with_pole)` builds seven parts or eight;
* `plan_cell` runs its **whole ring search twice** — pass 1 for a cell some
  existing network already covers, pass 2 for one that pays for its own supply.
  Two full passes in a fixed order, not an interleave, because the preference is
  a *value* judgement and not a distance one: a cell twelve tiles out that needs
  no pole beats one beside the anchor that costs a wood. Determinism is
  untouched — two ordered passes are as deterministic as one;
* `bill` counts `cells.iter().filter(Cell::brings_pole)`, so a pole reaches the
  bill only when a cell will actually place it;
* `fuel_for` reads the network off the **product machine** rather than off the
  pole, because the cells this is for no longer have one.

`a_cell_inside_an_existing_supply_area_brings_no_pole_of_its_own` is the run's
finding as a test. Its fixture puts the generator seven tiles away on a second
wired pole, which is what leaves the first pole's supply area empty — the
difference between a plant whose engine sits in the ground a cell wants (the
`powered()` fixture, where adoption is impossible) and one whose does not. It
asserts the adopted cell is genuinely `Powered`, and that **no action in the
whole plan so much as mentions a pole** — asserting only on placements would
pass against a bill that still asks for one and never puts it down, which
spends the wood just the same.
`a_cell_no_existing_pole_reaches_brings_one` is its control: without it the test
would pass against a planner that never places a pole at all.

**The one that is not mine: the planner cannot hand an item from one bot to
another.** Three wood sat in bots 2/3/4's pockets. `Holder::Share(chain_actor)`
welds the cell's whole bill to one bot, and `worth_converging` is about
splitting *production* — mining and smelting — not about moving stock that
already exists, so `Have{wood, 1, Share(bot 1)}` has no applicable method with
the item in plain sight. `Step::Owned` is the mechanism a handover would use and
it exists. This is a gap in `Have`, `power.rs` has exactly the same exposure,
and it is not a change to make at the end of a night. **Reported, not
attempted.**

A third thing the run showed, also outside these files: **rung 1 built two
power plants.** `Researched` builds one inline whenever `lab_site` cannot show
60 kW, and the replan after a failed batch asked again. That is what turned "one
spare pole" into "none", and it is a separate finding.

### 4.1 Run 2, with the fix in: the same refusal, one rung earlier

`run-1788399150-53956`, same script, same four bots, 92,235 ticks. It did **not**
reach the cell: it halted on **rung 1**, `researched("automation")`, with the
identical message.

```
milestone 1: stuck after 3 iteration(s), best 180 steps,
  refused: no method can satisfy goal: have 1 wood (a share sized for bot 1)
```

Its record shows the mechanism without the cell anywhere near it: three
dispatched `craft 1 small-electric-pole` and **five** `place
small-electric-pole` across the replans, at `[-44.5, 10.5]`, `[-45.5, 10.5]` and
`[-44.5, -2.5]` — the power plant re-sited three times, each siting spending
poles. Final inventories: bot 1 empty, **bots 2, 3 and 4 each still holding one
wood**.

So the run says three things plainly, and only one of them is about this cell:

1. **The cell's pole was never the whole problem.** `power.rs` spends poles on
   every re-site and hits the same wall one rung earlier. §4's change is still
   right — a cell should not spend a wood to duplicate supply three tiles away —
   but it was never going to be sufficient on its own.
2. **The blocker is the handover gap**, exactly as §4 said, and it is worth more
   than the pole fix: four wood in a roster and a plan that cannot reach three
   of them.
3. **Rung 1 is not as reproducible as run 1 made it look.** It closed in run 1
   and failed in run 2 on a different map, three transient placement refusals in
   a row (`a character is standing in the footprint`) driving the replans that
   spent the poles.

### 4.2 Runs 3 and 4

Recorded so the count is honest rather than flattering. **Run 3 aborted with no
bots**: the four graphical clients never obtained a character inside the 90 s
wait (`0/4 have a character`), the script's own roster guard caught it, and it
stopped without planning. It says nothing about the cell.

**Run 4 got one bot of four** — three clients again failed to obtain a
character, one succeeded, and `wait_for_roster`'s deliberate "proceed and say
who is missing" branch let the run go ahead with it. That is a *better* test of
this cell than a four-bot run, for a reason worth stating: with one bot there is
no handover to fail, so `Holder::Share(chain_actor)` is trivially satisfiable
and the wood question the first two runs died on cannot arise at all. It is a
worse test of everything else, because one bot does four bots' walking.

**A correction, since this file briefly said otherwise.** An earlier version of
this section recorded run 4 as never having started, on the strength of a
`Couldn't acquire exclusive lock` line at the top of its log. That line is one
client failing to take a lock, not the run failing to launch; the run went on to
plan and execute. The mistake is left visible here rather than quietly rewritten
because it is the same mistake this whole note is about — reading a partial
signal as a verdict.

**Wood stopped being a cap while this was being written.** Another agent landed
`b0e3e12e feat(planner): wood comes off a tree, so the eight-pole cap is gone`
on `master` — `Mine` can now fell a tree, which is the fix the run-status block
above pointed at. That removes the refusal both runs died on; it does not by
itself say the cell works, and this note does not claim it does.

## 5. What the witness watches, and whether it needed generalising

**It did not, and the reason is worth writing down because the obvious reading
says it should.**

`supervisor.witness` takes `from`/`into` — the two ends of *one*
machine-to-machine link — and red science is a chain of five. So:

```lua
supervisor.witness {
    item = "automation-science-pack",
    from = "inserter", into = "assembling-machine-1",
    near = { x = 0, y = 0 }, radius = 300,
    at_least = 1, within_ticks = 3600,
}
```

watches **both** machines, because an inserter drops into each. That is not a
fudge: **the terminal is chosen by the item, not by the watch set.**
`supervisor.count_item` sums one item name across the watched machines' *output*
inventories, and the gear machine's output holds gears and never packs — it
contributes zero to the before reading and zero to the after one, so the delta
is the pack machine's alone.

Where a generalisation *would* be needed: a chain whose terminal makes the same
item as an earlier stage. Then `from`/`into` would have to name positions rather
than entity kinds, and the caller would have to learn the site the planner
chose. No chain in this design has that shape, and inventing the API without one
to check it against would be inventing it — which is exactly the reason
`2026-09-03-witnessing-production.md` §2.2 gave for not generalising it then.

Two smaller points about the window. `within_ticks = 3600` is about five times
the cell's own predicted fill (60 ticks for the first gear + 600 for the first
pack + three inserter swings), the same margin stage 1 chose and for the same
reason: those numbers are a *model* read off prototypes, and the point of
witnessing is that the model can be optimistic. And the cost is asymmetric —
`at_least = 1` stops a working cell's wait at about 700 ticks, so only a dead
cell pays the whole minute.

---

## 6. Determinism: no pin moved

**Every makespan pin passes unchanged and none was edited.** `red_science.rs`,
`scheduling.rs`, `placement_occupancy.rs`, `refusal_memory.rs`,
`tile_capacity.rs`, `tile_occupancy.rs`, `tile_reservation.rs`,
`split_capacity.rs`, `smelt_roots.rs`, `seeded_roster.rs`, `buffers.rs`,
`ore_underfoot.rs`, `recipe_probability.rs` and `produce.rs`'s
`the_whole_of_stage_one_costs_this_much` are all green, together with 467
planner unit tests and the whole workspace (`cargo test --workspace`, zero
failures; `cargo clippy --workspace --all-features --all-targets --deny
warnings`, clean).

It could hardly be otherwise for the *existing* ladder — no method it expands
reaches this code — and the two shared things it does touch were kept
conservative on purpose:

* `have::holds` for `Goal::Producing` became a disjunction of the two cell
  shapes. They are disjoint by construction: one wants a smelting recipe taking
  one ore, the other a crafting recipe taking two ingredients, so no item is
  answered by both and the stage-1 answer is unchanged for every item that has
  one.
* `PlanState::consumer_draw_kw` is a public reader over the existing private
  `consumer_kw` table and changes nothing about what it contains.

The new code is deterministic by construction: the anchor is the pole
`nearest_supply_anchor` orders first (distance, then `x`, then `y`), the
candidates come out of a ring search in a fixed order, the four facings are
tried north/east/south/west, the layout is a `const` array of offsets turned by
`Position::turn`, the cell count and both charge quantities are integer
arithmetic end to end, and `boiler_coal` rounds the demand up to a whole
kilowatt before dividing so no float reaches the answer.
`the_same_state_sites_the_same_cell_twice` pins it.

---

## 7. Red-first, with the actual output

**The headline red needed no new API at all**, which is the strongest form
available here: `Goal::Producing { item: "automation-science-pack" }` was a goal
no method claimed.

```
---- a_producing_goal_for_red_science_builds_an_assembly_cell stdout ----
thread '...' panicked at crates/planner/tests/red_science_cell.rs:241:23:
a powered world can build a red-science cell: NoApplicableMethod
  { goal: "produce 6 automation-science-pack/min" }

---- the_cell_count_follows_the_rate_and_nothing_else stdout ----
thread '...' panicked at crates/planner/tests/red_science_cell.rs:280:14:
a powered world can build a red-science cell: NoApplicableMethod
  { goal: "produce 6 automation-science-pack/min" }

test result: FAILED. 2 passed; 2 failed
```

The two that passed in that same run are the **premises**, and they pass
because they are claims about data rather than about code:
`the_red_science_chain_is_what_the_live_game_says_it_is` reads the chain out of
`crates/core/tests/live-2.1.17-world-snapshot.json` (0 of 277 technologies
researched, so its `enabled` flags *are* the answer to "available with no
research"), and `the_specs_621_kw_is_what_the_demand_ledger_says` reads the
spec's figure out of `consumer_kw`. The 1.1 recipe fixture is used only as the
*shape* of a world to plan against; every claim about the chain comes from the
live capture, and the `assembling-machine-1` recipe the 1.1 capture does not
have at all is added from it.

**The geometry tests could not be red-first** and saying so is the honest
answer: a Rust test naming `Role::LinkInserter` before that type exists is a
compile error, not a red test. What pins them is §7.

**One red nobody wrote, and it was mine.**
`the_charge_is_integer_arithmetic_from_the_recipe` failed on its first run:

```
assertion `left == right` failed: 9000 ticks / 30 a circuit
  left: 100
 right: 300
```

I had written the electronic-circuit case from arithmetic done in my head — a
circuit at 60 ticks — and the code's answer was 90, because three cables at a
60-tick two-cable run make the *cable* machine the bottleneck and not the
circuit machine. The code was right and the test was wrong. It is worth
recording because it is the same class of error the whole design is against:
a number derived rather than read.

---

## 8. Mutation battery

Each applied alone to the fixed tree, the whole `-p factorio-bot-planner` suite
run with `--no-fail-fast` (467 unit tests plus every integration binary), the
tree restored between each. Test names are abbreviated; all are in
`method::assemble::tests` unless marked.

| # | Mutation | Failed |
|---|---|---|
| M1 | the link inserter turned round — **the trap itself** | **16**, incl. the plan tests, `every_link_of_the_chain_delivers_at_every_facing`, `an_inserter_turned_round_places_perfectly_and_feeds_nothing` |
| M2 | `compose` ignores the cell's facing | 2 — the two direction tests |
| M3 | the pole moved one tile | 2 — `the_cells_own_pole_covers_every_consumer_in_it`, `no_two_buildings_of_a_cell_overlap_at_any_facing` |
| M4 | `fit` stops checking headroom | `a_cell_is_refused_on_a_network_that_is_already_spent` |
| M5 | `fit` stops checking the links | **nothing** — see below |
| M6 | `cells_standing` ignores the recipe on the machine | `a_cell_whose_product_machine_has_no_recipe_does_not_hold` |
| M7 | `cells_standing` counts an inserter nothing feeds | `a_product_machine_short_of_a_feeder_does_not_count` |
| M8 | `cells_standing` stops checking power | `a_cell_that_stands_on_a_dead_network_does_not_hold` |
| M9 | `feed_charge` rounds the run count down | `the_last_run_of_a_charge_is_paid_for_in_full` |
| M10 | `feed_charge` ignores the intermediate recipe's yield | 2 — both charge tests |
| M11 | `boiler_coal` drops the one-slot cap | `the_coal_bill_is_bounded_by_the_one_slot_it_goes_in` |
| M12 | no boiler is ever found | `the_plants_boiler_is_topped_up_for_the_charge` |
| M13 | every consumer claimed at an assembler's 75 kW | `every_powered_condition_states_the_draw_the_ledger_charges` |
| M14 | only the product machine gets a recipe | `a_producing_goal_for_red_science_builds_an_assembly_cell` |
| M15 | the charge inserts stop asserting the recipes | `the_charge_inserts_assert_the_whole_chain` |
| M16 | the charge inserts stop asserting the links | the same one |
| M17 | the servicing lane runs over the chests | `every_lane_tile_admits_a_character` |
| M18 | `have::holds` forgets the stage-2 shape | `a_cell_that_stands_with_its_recipes_on_it_holds` |
| M19 | an intermediate need not be *crafted* | **18** — every test in the file that plans or holds |
| M20 | **negative control** — boiler search 16 → 20 tiles | **nothing** |
| M21 | **negative control** — `NoRoomForCellNearPower`'s help reworded | **nothing** |
| M22 | the charge window moved 9,000 → 9,600 ticks | 2 — the charge and the coal bill |
| M23 | one link stated backwards in `links()` | **16**, as M1 |
| M24 | the cell's own pole left out of the bill | 6 |
| M1+M5 | **the trap, with the check that would catch it also removed** | **6**, incl. `every_link_of_the_chain_delivers_at_every_facing` and every `holds` test |

A second battery for the pole change of §4, same discipline:

| # | Mutation | Failed |
|---|---|---|
| M25 | the cheap pass removed — every cell brings a pole | `a_cell_inside_an_existing_supply_area_brings_no_pole_of_its_own` |
| M26 | the paying pass removed — no cell may bring one | **15**, incl. `a_cell_no_existing_pole_reaches_brings_one` and every plan and `holds` test |
| M27 | the bill asks for a pole per cell regardless of `brings_pole` | `a_cell_inside_an_existing_supply_area_brings_no_pole_of_its_own` — and only because that test reads *labels*, not placements |
| M28 | `brings_pole` always answers no | 7 |
| M30 | **negative control** — `POLE_OFFSET`'s doc reworded | **nothing** |

M27 is the one worth reading twice: it is the mutation the first draft of that
test did **not** catch. Asserting "no pole is placed" passes against a bill that
crafts one and leaves it in a pocket, which spends the wood exactly as a
placement would. Asserting that no action's label mentions a pole catches both.

**M5 kills nothing on its own, and that is stated rather than counted.**
`fit`'s link check is a re-statement of a geometry the `LAYOUT` constant already
guarantees, so with a correct layout removing it changes no answer. What it buys
is *where* a wrong layout fails: under M1 or M23 it turns a dead factory into a
planning refusal, and every one of those 16 failures goes through it. M1+M5 is
the combined mutation that says the trap still has teeth without it — the
geometry test and all four `holds` tests still die.

**M19 is the one that says the shape rule is load-bearing.** Dropping the
"crafting" requirement from `intermediate_for` makes `copper-plate` (smelted
from one ore) read as a makeable intermediate too, so red science has *two*
makeable halves, `assembly_spec` refuses it, and eighteen tests die. The rule is
not decoration around a hardcoded recipe.

---

## 9. Residuals, stated rather than closed

* **A cell may still need a pole, and then it is still stuck.** §4 removes the
  pole in the case the cell is usually in — sited around an existing supply
  area — and changes nothing about the case where the plant's own engine
  occupies the ground a pole-less cell would need. Then pass 2 asks for a pole,
  and on a run whose acting bot has no wood that refusal is the same one run 1
  hit, one layer earlier. The real fix is a handover method in `Have`, or
  teaching `Mine` to fell a tree; both are named in §4 and neither is here.
* **The chests are filled by hand.** §3. `CELL_CHARGE_TICKS` is 9,000 ticks —
  two and a half minutes, fifteen packs, thirty iron plates and fifteen copper
  plates — and nothing refills them. The structural predicate keeps holding
  after they run dry, exactly as stage 1's does after its drill's fuel runs out;
  only the next witness catches it.
* **`cells_standing` sweeps 512 tiles from the world origin.** `holds` gets no
  position, and `EntityGraph` offers no "every entity" query, so the sweep needs
  a centre and the origin is it. A cell built further out is not counted and its
  goal is re-planned — over-building rather than over-claiming, the direction
  this crate chooses everywhere, and named in `CELL_SCAN_RADIUS`' own doc.
* **`cells_standing` cannot tell which item an inserter carries.** It counts
  *one loaded inserter per ingredient of the recipe*, which is the only handle
  this crate has: nothing models item routing. A machine ringed by three
  inserters all fed with iron would count. What stops that in practice is that
  the planner built the cell and knows what it charged each chest with; what
  would catch it in general is the witness.
* **The boiler top-up finds "a boiler within sixteen tiles of the anchor".** On
  a map with an unrelated boiler nearer than the plant's, the wrong one gets
  the coal. The cost of being wrong is a few coal in the wrong machine rather
  than a wrong plan, and it is named as a heuristic in
  `BOILER_SEARCH_RADIUS`' doc.
* **Inserter throughput is unmodelled.** A vanilla inserter moves roughly 0.83
  items a second and this cell needs 0.1, so it is nowhere near binding here —
  but `ticks_per_item` is the slower of the two *machines* and knows nothing
  about the three inserters between them. A cell whose rate approached an
  inserter's would be over-claimed.
* **`feed_charge`'s `div_ceil` is exercised by no reachable recipe.** 300
  cables is exactly 150 runs, so floor and ceiling agree everywhere the game can
  reach. `the_last_run_of_a_charge_is_paid_for_in_full` asks it of a hand-built
  spec with an odd count, and says in its own doc that this is why.
* **A boiler with no fuel still reads as 900 kW.** `electric_supply_kw` counts
  nameplate. Unchanged from `power.rs`; spec §13 row 2. The top-up makes it less
  likely, not less possible.
* **Green science is not reachable from here.** `logistic-science-pack` costs
  **75 red packs** and this cell's charge is fifteen. Feeding a lab from the
  cell needs either a much larger charge (which is a smelting test, not a
  factory one) or the smelting half automated, which is stage 3.

---

## 10. Files

**Owned and changed:** `crates/planner/src/method/assemble.rs` (new — the
layout, the siting, `cells_standing`, `holds_assembling`, the steps, the
conditional pole of §4, and 26 tests), `crates/planner/tests/red_science_cell.rs` (new — 4 tests: the two
game-data premises and the two plan tests that were the red),
`crates/planner/src/method/mod.rs` (one `pub mod`),
`crates/planner/src/method/have.rs` (both registries, and `holds`'s second
disjunct), `crates/planner/src/state.rs` (`consumer_draw_kw`),
`crates/planner/src/error.rs` (`NoRoomForCellNearPower`, and `NoCellProduces`'
help, which named stage 2 as future work),
`crates/scripting_lua/src/globals/goal/mod.rs` (`refusal_for`'s verdict arm),
`scripts/factory_stage2.lua` (new — the three-rung ladder and its witness).

**Untouched:** `crates/core/`, `crates/executor/`, `mods/`, `app/src/`, the
OpenAPI snapshot, and every makespan pin.
