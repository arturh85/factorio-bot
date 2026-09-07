# A fluid arrives by pipe

2026-09-07, branch `a-fluid-arrives-by-pipe`, off `874ebb63`. Sequel to
`2026-09-07-a-goal-that-names-its-recipe.md`, whose closing output is this
task's opening wall:

```
basic-oil-processing runs in oil-refinery (category oil-processing), and this
planner can name that machine now -- but the recipe wants 100 crude-oil, which
is a fluid. ... so the 100 crude-oil cannot be delivered to the oil-refinery.
```

## The owner's three rulings, and what each became in code

1. **A fluid ingredient is satisfied by CONNECTIVITY, not by a quantity** --
   *"you pipe crude to a refinery, you never carry it"*. No `Goal::Stored`, no
   tenth goal kind. `method::fabricate` resolves the machine's input fluidbox
   to something standing that can supply, and lays the pipe.
2. **The plan builds the run, and the topology is tank -> refinery.** The long
   trunk from the patch tank to a base tank is *not* built and is still the
   separate item it was.
3. **A fluid product requires a sink or the plan refuses by name.** One fluid
   out is given a buffer and a pipe run into it; several -- which is
   `advanced-oil-processing` -- refuses, because the multi-output rule is the
   owner's separate decision.

## What shipped

**`crates/planner/src/method/pipe.rs`**, a new module and *not* a method: it
claims no goal, because a fluid ingredient is not a goal. Everything in it
except `route_between`, `sources_of` and `port_is_placeable` was **moved
unchanged from `method::gather`**, which had built the wellhead's
pumpjack-to-tank run first; `gather` now calls the shared router. A second
router agreeing with the first until it did not was the alternative.

`method::fabricate` gained `plan_fluid_rig`, which sites the machine and
resolves both runs **before a single step is emitted** -- the promise
`method::connect` makes, for the same reason.

### The asymmetry that decides everything

**A source is adopted and never built; a sink is built when none stands.**
A buffer this plan places is *empty*, which is exactly right for catching an
output and exactly wrong for feeding an input. Building a tank and calling it
a crude supply would be the "factory that quietly stops" ruling 3 forbids, in
its purest form.

### `sources_of`: three tests, all derived from the world

The first version took the nearest fluidbox whose `production_type` supplies.
**On its first offline run it chose a `boiler`** -- `method::power` had sited
a plant at the wellhead, and a boiler's steam box supplies. A refinery piped
to a boiler builds 100% correctly and makes nothing. So a candidate must be
*attributable* to the fluid:

1. **the plan told it what to make** -- `entity.recipe` names a recipe whose
   products include the fluid;
2. **it stands on the resource** -- an extractor's footprint covers a charted
   tile of it, i.e. a pumpjack on a crude well;
3. **it is a buffer at that resource's field** -- a `storage-tank`-typed
   entity within `gather::FIELD_RADIUS` of a charted tile, which is precisely
   the tank `method::gather` stands up.

**A buffer outranks an extractor**, which is ruling 2 and not an optimisation:
*"the fluid tank the oil arrives in from far away should be connected to the
refineries"*. Ranking by distance alone ties one refinery to one well, and it
is what the code did before the test existed.

Rule 1 is what makes the chemistry rung compose with no new code: a refinery
this plan set to `basic-oil-processing` **is** a petroleum source for a
chemical plant, by exactly that test.

## Five things measured that the brief got wrong or did not say

Each was believed going in and did not survive contact with the data.

**1. `storage-tank` and `pipe` are `production_type: "none"`, not
`input-output`.** There is no `"input-output"` on any prototype in the live
capture. A box that neither produces nor consumes is one fluid passes through
either way -- which is what a buffer *is* -- so leaving `"none"` out of the
supplying set made every tank invisible as a source. It did: that is why the
first run picked a pumpjack.

**2. `volume` is `None` on every dump this project holds.** So the
capacity check (`SinkTooSmall`) cannot fire offline, and **`None` is "the
sender did not say", never zero** -- refusing on silence would break the whole
archive. There is a control test for the silent case beside the refusal one.

**3. An `oil-refinery` declares TWO input fluidboxes and THREE output ones**,
and `basic-oil-processing` uses one of each. Nothing on our wire says which.
The game assigns a recipe's fluid ingredients to input boxes **in order** and
its fluid products to output boxes in order, so the nth fluid belongs to the
nth box -- `PipeEnd::port_index`. Ranking those by distance would have been a
coin flip whose wrong answer builds perfectly and moves nothing.

**4. A machine sited flush against its source has its port INSIDE the
source.** A refinery beside its tank put its first input port on a tile the
tank stands on, and the run refused with *"the tile at ... cannot hold a
pipe"* -- a true statement about a site that should never have been chosen.
`pipe::port_is_placeable` is that refusal turned into a siting predicate.

**5. A pipe already standing is a JOIN, not an obstacle.** `gather` never met
this: it routes on virgin ground at a wellhead. The second run out of the tank
the first run filled refused on the first run's own pipes. Such a tile is now
dropped from the emission (it needs no placement) and unblocked on the search
grid -- which also makes a replan over a half-laid run a no-op rather than a
refusal.

## Before and after, real output

Both on the annotated world (see below), release, `--bots 1,2,3,4`.

**`produced:petroleum-gas:45:basic-oil-processing`**

```
master   basic-oil-processing runs in oil-refinery (category oil-processing), and
         this planner can name that machine now -- but the recipe wants 100
         crude-oil, which is a fluid. No character inventory holds a fluid, no
         `InventorySlot` addresses a fluidbox, and no action in this planner moves
         one, so the 100 crude-oil cannot be delivered to the oil-refinery.

branch   basic-oil-processing runs in oil-refinery (category oil-processing), and
         the recipe wants 100 crude-oil -- a fluid, so it arrives by pipe rather
         than in a hand. Nothing standing on this map can be shown to supply
         crude-oil, so there is nothing to connect the oil-refinery to.
```

**`have:plastic-bar:2`** -- the coordinator's headline goal:

```
master   ... but the recipe wants 20 petroleum-gas, which is a fluid. No character
         inventory holds a fluid, no `InventorySlot` addresses a fluidbox, and no
         action in this planner moves one ...

branch   ... and the recipe wants 20 petroleum-gas -- a fluid, so it arrives by
         pipe rather than in a hand. Nothing standing on this map can be shown to
         supply petroleum-gas, so there is nothing to connect the chemical-plant
         to.
```

**`have:sulfur:5`** moves furthest, from a fluid it cannot carry to the thing
that is actually undecidable:

```
master   ... the recipe wants 30 water, which is a fluid ...

branch   sulfur runs in chemical-plant and takes 2 fluids in -- water and
         petroleum-gas -- and no field this planner receives says which of the
         chemical-plant's input fluidboxes accepts which, so a pipe would be a
         guess
```

**Both refusals are correct on that world: nothing is standing.** The map has
never had a tank on it.

### Where the pipe run actually gets built, offline

Chaining the rung below it does it -- the tank `gathered:crude-oil` stands up
is visible to the next goal through the plan overlay:

```
$ factorio-bot plan --world <annotated> --bots 1,2,3,4 \
      --goal gathered:crude-oil \
      --goal produced:petroleum-gas:45:basic-oil-processing

master   ... the 100 crude-oil cannot be delivered to the oil-refinery
branch   bot 1 owns chain ChainId(451) because its bill was sized against it,
         but has 5 iron-ore does not hold there
```

**The branch is past every fluid wall and into the scheduler.** That last
refusal is **pre-existing and not about fluids**, verified with a control:
`--goal gathered:crude-oil --goal have:oil-refinery:1` produces the same class
of failure, byte-identical on master and on the branch:

```
bot 2 owns chain ChainId(659) because its bill was sized against it,
but has 1 small-electric-pole does not hold there
```

So **`gathered:crude-oil` composed with any second goal fails in scheduling
today**, on master too. That is the next thing in the way of an end-to-end
offline oil plan, and it is somebody's separate bug.

The end-to-end path *is* exercised, in `method::fabricate`'s own tests: on a
world where the crude has arrived in a tank, the goal expands into a refinery,
its recipe, a pipe run from the tank to the refinery's first input box, a
buffer, and a pipe run from its first output box into that buffer.

## Baselines: nothing moved

Re-measured by me, **one release binary each**, `--bots 1,2,3,4`, master
`874ebb63` and this branch on top of it.

| goal | master | branch |
|---|---|---|
| `researched:automation` (`map.json`) | 176 / 21,784 | **176 / 21,784** |
| `producing:automation-science-pack:6` | 316 / 22,457 | **316 / 22,457** |
| `producing:logistic-science-pack:6` | 441 / 47,478 | **441 / 47,478** |
| `gathered:crude-oil` (`map-31337-explored.json`) | 2,115 / 317,283 | **2,115 / 317,283** |

`gathered:crude-oil` on `map.json` refuses on both -- no oil is charted there,
which is correct and is why it is measured on the explored dump. The
`gathered:crude-oil` figure is the one that matters here: `method::gather`'s
router was moved out from under it wholesale, and the plan is identical.

## The offline basis cannot exercise the new path, so it was annotated

Every dump this project holds predates `crafting_categories`, so no machine is
nameable on them and the old messages stand. The measurements above use
`scratch/map-31337-explored-with-categories.json`, the previous agent's
annotated copy (eleven vanilla crafting prototypes transcribed from
`workspace/server/data/base/prototypes/entity/entities.lua`), **with the
master binary run on the same file as a control** -- so the file changed
nothing by itself. That control is what the "before" column above is.

## Tests

`nix develop -c cargo test --workspace --no-fail-fast` -- **exit 0** captured
from the command itself, not from a pipeline: 111 blocks, **3,000** tests, 0
failed (2,985 on master). `cargo clippy --workspace --all-features
--all-targets -- --deny warnings` -- exit 0.

Fifteen new tests: twelve in `method::fabricate`, three in `method::pipe`,
built on `test_world::world_with_oil` -- the oil ladder's own fixture, written
for `method::gather` a day before this code existed.

**Four tests on master were superseded and rewritten**, not deleted, because
the fact each asserted has moved:

| test | asserted | now asserts |
|---|---|---|
| `naming_the_recipe_moves_the_refusal_to_the_fluid_ingredient` | "no action moves a fluid" | "nothing standing can be shown to supply it" |
| `naming_a_different_recipe_..._reaches_a_different_wall` | `light-oil-cracking` with its fluids dropped lands on a fluid *product* | it lands on **two fluids in**, which is a genuinely different wall |
| `a_fluid_ingredient_is_refused_with_the_machine_named` | "is a fluid" | "a fluid, so it arrives by pipe" + the missing source |
| `a_fluid_product_is_refused_with_the_machine_named` | one fluid product has nowhere to land | **several** do; one is given a buffer |

### Falsification

Thirteen breaks, one at a time, each substitution asserted to match **exactly
once** in the breaking direction, each restored with a `touch` (`cp -p` and
`shutil.copy2` preserve mtime, and cargo then re-runs the *mutant* binary
against restored source -- CLAUDE.md). Scope: `cargo test -p
factorio-bot-planner --no-fail-fast`.

| # | break | red |
|---|---|---|
| 1 | attribution ignored -- any supplying fluidbox is a source | 1 |
| 2 | a `none` fluidbox is not a source, so tanks are invisible | 7 |
| 3 | a buffer no longer outranks an extractor | 1 |
| 4 | `port_index` ignored -- every box is a candidate | 2 |
| 5 | a standing pipe is an obstacle again | 1 |
| 6 | **siting no longer asks whether the machine's ports fit** | **0 -- see below** |
| 7 | the machine is sited at the bot, not at its source | 7 |
| 8 | a fluid product is taken into a hand like an item | 1 |
| 9 | no buffer is built for a fluid product | 1 |
| 10 | two fluids in are no longer refused | 2 |
| 11 | several fluids out are no longer refused | 2 |
| 12 | a stated volume too small is accepted | 1 |
| 13 | the pipe runs are resolved but never emitted | 1 |

**Break 6 is GREEN and I could not close it. Reported rather than papered
over.** `port_is_placeable` is pinned by its own test; **that siting uses it is
pinned by nothing**, because the fixture cannot reach the case. Measured, not
assumed: with the predicate and without it, `free_area_near` returns the *same*
site -- `Some(28.5, 23.5)` -- and it does so for a structural reason.
`free_area_near` walks rings outward and takes the **first** free candidate in
each ring, which is always a far corner, and a far corner's port never lands
inside the source. The live failure needed a *partially occupied* ring, where
every corner-ward candidate was taken and the survivor sat at `(+2, -4)` from
the tank -- the one offset whose port falls inside a 3x3 footprint. Two attempts
to arrange that in the fixture (moving the tank clear of the wells, blocking the
ring with furnaces) both landed on a far corner again.

So the wiring rests on the live measurement in "Five things measured" above and
on the predicate's own test, and not on a falsified one. Whoever next touches
siting here should know that removing the predicate breaks nothing in CI.

**Break 13's first form was a COMPILE-ERROR** (`for run in [None, None]` has no
inferable type), which is not a result -- a mutation that does not build tests
nothing. Re-run with a mutation that compiles: 1 red.

## What this does not prove

* **That anything has been executed.** No run was made. Every claim here is
  about a plan.
* **That a tank the plan pipes from holds the right fluid.** Nothing in
  `PlanState` models fluid contents and no dump carries any. Rule 3 attributes
  a tank by *where it stands*, which is an inference from the map and not an
  observation. Closing it needs either fluid contents on the wire or a
  `Goal::Gathered` that records what its tank is for.
* **That water can be sourced.** Water is not a charted resource and an
  offshore pump carries no recipe, so none of the three rules answers for it.
  That is deliberate rather than forgotten: every recipe needing water also
  needs a second fluid, and those refuse before a source is looked for -- a
  water rule today would be code with no reachable caller.
* **That the emitted plan would execute.** `Fabricate` still does not call
  `power::ensure_powered`, and a refinery is electric. Unchanged from the
  predecessor, and unaddressed here.

## For the owner

**The chemistry rung should fall out, and here is exactly what it needs.**
`plastic-bar` and `sulfur` are the reason a sink exists, and:

* **`plastic-bar` needs one thing**: a standing petroleum source. Rule 1
  already recognises a refinery running `basic-oil-processing` as one, so a
  plan that builds the refinery first and the chemical plant second composes
  with **no new code** -- once the scheduling failure above is fixed.
* **`sulfur` needs a field on the wire**: which fluid each input fluidbox
  accepts. Two fluids in is refused by name today, and no amount of geometry
  closes it. That is a mod-side change (`LuaFluidBox.get_filter` /
  the prototype's `filter`), not a planner one.

**And the adoption of a consumer as a sink is the next rung after that.** Today
a fluid product always gets a fresh buffer. A chemical plant that consumes
petroleum *is* a sink for a refinery, one hop along -- the same primitive with
`["input", "input-output"]` instead of the supplying set. It is deliberately
not written yet, because it would be a constant nothing reads.
