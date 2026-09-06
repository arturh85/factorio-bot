# Ore does not block

2026-09-06. **In Factorio a resource entity's entire collision mask is the
single `resource` layer, and no buildable prototype carries that layer — so ore
blocks nothing, and only a mining drill *wants* to be on it.** The planner
encoded the opposite for months, in the one place that decides whether a
placement is legal.

## The rule, and what it rests on

Not one query. Three sources, and they agree.

1. **The game's own prototype data.** `data/core/lualib/collision-mask-defaults.lua`
   gives `["resource"] = {layers={resource=true}}` — a resource entity's whole
   mask. Grepping `resource *= *true` across `base`, `core`, `space-age`,
   `quality` and `elevated-rails` finds the layer named only by *tiles* (a
   water tile carries it so ore does not generate on water) and by one
   decorative. **No entity prototype in the shipped game collides with it.**
2. **A live capture.** `crates/core/tests/live-2.1.17-world-snapshot.json` is
   1,028 prototypes read out of a running 2.1.17 game. 579 declare a mask;
   exactly 12 name `resource`; all 12 *are* resources, each with `["resource"]`
   and nothing else. The 2022 1.x fixture
   (`crates/core/tests/entity-prototype-fixtures.json`) agrees across the
   `resource-layer` → `resource` rename. Now asserted, against the capture, in
   `live_2_1_payloads.rs::nothing_buildable_collides_with_the_resource_layer`.
3. **The running game.** A peer session's `can_place_entity` at the ore tile
   `(-42.5, -37.5)`: `belt = true`, `pole = true`, `drill = true`.

The control that makes the rule readable: **a stone furnace's mask is character
for character a burner mining drill's** — `is_lower_object`, `is_object`,
`water_tile`, `item`, `object`, `player`, `meltable`. Whether a machine needs
ore *underfoot* is not expressible in a collision mask at all, which is why
`stands_on_resources` reads `entity_type == "mining-drill"` — and exactly why
it must not be asked what blocks a placement.

## The defect

`crates/planner/src/state.rs` computed `resource_blocks = !stands_on_resources(name)`
at every placement check, so ore refused everything but a mining drill. One
predicate answered two questions: *does this machine need ore underfoot* and
*does ore block this*. The first is real. The second the game never asks.

`stands_on_resources` came from a correct diagnosis — `is_area_free` "refused a
drill everywhere on every map" — and a wrong generalisation: it carved an
exception for drills instead of correcting the rule. What survived was a
**policy** ("do not bury the patch you are about to mine") stated as a
collision rule, and the function's own doc comment said as much while the code
went on enforcing it as physics.

**Nothing caught it because it fails in the safe direction.** It refused legal
ground and never built on illegal ground, so it surfaced as `NoRoute` and
`NoSiteFound`, never as a broken factory:

* `method::connect` routes belts through `is_area_free` / `is_position_free`, so
  a belt run that would legally cross a patch was refused;
* `method::assemble` sites cells with the same predicates, so a cell near a
  patch was refused a site;
* `MinerLine` could not site at any radius, because its own belt-and-pole
  corridor runs over the ore its drills need — a peer session was about to
  record that as structural.

## What changed

`occupant_of` lost its sixth source, its `resource_blocks` parameter and
`Occupant::Resource`. `is_area_clear_of` takes `water_blocks` alone;
`is_area_free_facing`, `placement_occupant` and `siting_occupant` no longer
consult `stands_on_resources`, which keeps only its real job and its real
callers (`method::blueprint`'s `drills_are_fed` and `nearest_ore_seed`).

**The policy is kept — as a policy, where ground is chosen.** Two sites, both
now saying so in the code rather than inheriting it from a rule about physics:

* `method::util::free_area_near_where` skips a candidate whose footprint covers
  ore (`PlanState::covers_any_resource`). This is the ring search every
  hand-smelt furnace, lab and pole is sited by.
* `method::produce::fit` does the same for a cell's furnace, which is sited at a
  fixed offset from its drill and never goes through that search.

**Both were measured, not assumed.** Dropping either one fails
`seventy_five_packs_are_crafted_on_several_bots_and_each_delivers_to_a_lab`
with `NoApplicableMethod { goal: "have 40 iron-ore" }`: a furnace standing on a
patch tile makes that tile unminable (`PlanState::resource_tile_blocked`, doing
its job correctly), and the plan eats the ore it was about to mine. A first
attempt made ore a *ranked preference* with a fallback onto the patch when
clear ground ran out; that is the shape that fails, because the fallback fires
exactly when the plan is under pressure. **Running out of ground is
recoverable** — `smelt_steps` queues into a furnace that already stands, which
is what `crates/planner/tests/furnace_ground.rs` is about — **and running out of
ore is not.**

So the honest summary of the fix is narrower than "ore stops blocking": ore
stops blocking *placements*, and stays refused by *two named siting searches
that had a reason*.

## Measurements

Offline against `workspace/scripts/map.json` (seed 31337, fingerprint
`c161fa3f437221d0`), with two release binaries built from this same tree — the
"after" one from the branch, the "before" one after checking `state.rs`,
`method/util.rs` and `method/produce.rs` back to master. Nothing is quoted from
elsewhere.

| goal | bots | before (actions / ticks) | after |
|---|---|---|---|
| `researched:automation` | 4 | 176 / 21,784 | **176 / 21,784** |
| `producing:automation-science-pack:6` | 4 | 316 / 22,463 | **316 / 22,463** |
| `producing:logistic-science-pack:6` | 4 | 442 / 47,542 | **442 / 47,542** |
| `have:pumpjack:1` | 4 | 1,674 / 263,432 | **1,674 / 263,432** |
| `researched:oil-gathering` | 4 | 1,671 / 260,115 | **1,671 / 260,115** |
| `researched:automation` | 8 | 382 / 18,291 | **382 / 18,291** |
| `producing:logistic-science-pack:6` | 8 | 913 / 47,603 | **913 / 47,603** |

Four further probes, chosen to look for any difference at all — none:
`producing:iron-plate:30` (21 / 5,822), `producing:copper-plate:30`
(21 / 7,313), `have:electronic-circuit:50` (131 / 10,368),
`researched:logistics` (271 / 26,187).

Planning wall-clock at eight bots: **3 s** for automation, **6–8 s** for green,
against the 864 MB dump. Unchanged before to after.

**Nothing moved, and that is the result rather than a disappointment.** The
whole crate's makespan and plan-shape pins are unchanged too. The refusals this
removes are on paths the goal ladder does not yet walk: `method::connect` has no
caller in the tree, `Goal::Built` is only reachable from `goal.built`, and every
furnace and cell on this map still had non-ore ground within its search radius,
where the surviving policy sites them exactly as before. **Do not read the table
as "the fix does nothing" — read it as "the fix costs nothing, and what it
unblocks has no caller yet."** The things it unblocks are belt routes across a
patch, a cell sited beside one, and `MinerLine`; the first two are provable only
once `connect` and `assemble` are called on this map, and the third is the peer
session's to confirm.

## Tests, and the falsifications

Every new assertion was watched going red for the right reason, and each
substitution was checked to have actually matched (the edit asserts its own
match and prints `substitution matched`), per
`2026-09-06-fixtures-agree-with-their-code.md`.

* **`nothing_buildable_collides_with_the_resource_layer`** (core, against the
  live capture): expected count 12 → 13. Red, printing the twelve real names —
  `coal, copper-ore, iron-ore, stone, uranium-ore, calcite, tungsten-ore,
  scrap, crude-oil, sulfuric-acid-geyser, lithium-brine, fluorine-vent`. The
  test reads the data, not a constant beside it.
* **`ore_blocks_nothing_because_nothing_collides_with_it`** and
  **`a_tile_holding_ore_is_free_ground`** (state): the forbidden rule put back
  into `occupant_of`. Red — *"the game allows a furnace on ore, so the planner
  must"* — along with nine others, including every drill test, since the
  restored rule was cruder than the one it replaced.
* **`siting_steps_off_the_ore_that_no_longer_blocks_it`** (util): the ore filter
  removed from `free_area_near_where`. Red on its `covers_any_resource`
  assertion, and with it the five plan-shape tests, including the ore
  starvation above. The filter is load-bearing and now provably so.
* **`the_fixture_leaves_nowhere_to_site_a_furnace`** (furnace_ground): the water
  removed. Red, naming 91 newly free sites — **while the file's other three
  tests stayed green**, which is precisely the "passes by accident" this guard
  exists to catch.

`cargo test --workspace` green; `cargo clippy --workspace --all-features
--all-targets -- --deny warnings` clean.

**This task wrote both the code and its fixtures**, so: the state and util tests
assume `test_utils::fixture_world`'s iron patch is where the rest of the crate
already believes it is, and the core test assumes the 2.1.17 capture is a
faithful `world_snapshot` reply — it is the same file four other tests in that
module already read.

## Found and not fixed

* **A cell's furnace still cannot stand on ore, and the rim is thin.**
  `produce::fit`'s policy pushes every cell out to the patch edge, where seed
  `31337` holds 3–10 ore a tile against 200+ three tiles in — the exact
  shortfall `fit`'s own item 5 was written for after `run-1788552801-73005`
  fuelled a rim cell for 66 ore and got 40 plates. Lifting it needs a
  reservation model — "ore I will need later" — not a permission change;
  without one, the plan starves itself. Named at the check.
* **`resource_tile_blocked` is now the only thing between a plan and its own
  ore**, and it only sees entities already added to the overlay. `have.rs`
  already orders its placements before the ore blocks that depend on them; that
  ordering is now load-bearing in more places than when ore refused every
  placement outright.
* **The unblocked capabilities are still unexercised.** `method::connect` has no
  caller anywhere in the tree, so "a belt may now cross a patch" is asserted by
  a unit test and by the game's own prototypes, and by nothing that has run.
