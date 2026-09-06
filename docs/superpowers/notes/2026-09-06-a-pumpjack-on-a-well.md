# A pumpjack on a well: siting works, and the reach is 64 tiles

2026-09-06, branch `a-pumpjack-on-a-well`, merged onto master at `7492b9b6`. The plan
numbers below were taken on a release build of `14c54bbf` plus this branch. Everything below
was measured offline against `workspace/scripts/map.json` (the seed-31337 t=0
dump) with a release binary, or against the planner's own suite. **No live run
was made.**

---

## Summary

* **`method::extract` sites the extractor now.** `Goal::Extracted` was a
  four-tier refusal and nothing else; it picks a charted well, bills the
  machine, carries electric power to it, places it, and hangs the technology's
  `Effect::Researched` on that placement.
* **`researched:oil-processing` plans end to end** on the seed-31337 dump with
  a crude-oil patch charted into it: **1,750 actions, 262,081 ticks (1:12:48)**
  over four bots, `place pumpjack at [79.5, -39.5]`, ten small electric poles
  from the power plant's pole to the well, and the seven researches on the path
  (`automation`, `automation-science-pack`, `logistic-science-pack`,
  `steel-processing`, `engine`, `fluid-handling`, `oil-gathering`).
* **The reach is ~64 tiles, and the ceiling is not in this module.**
  `crate::state`'s `POWER_SEARCH_RADIUS` is 64, so a pole run longer than that
  carries power `Condition::Powered` cannot see. A well 121 tiles from
  generation refuses.
* **No fluid is modelled**, on purpose: the `mine-entity` trigger fires when a
  pumpjack extracts, natively, and where the petroleum gas goes is a separate
  problem.
* **The one thing this cannot claim** is that any of it works in the game. The
  evidence is a plan, not a run.

---

## 1. What the method does

`Extract::applicable` is now true when the world can answer tiers 1 to 3 of the
old ladder — the resource is charted, something mines it, and that machine's
recipe is open or already being researched by this plan. Tiers 1 to 3 are
otherwise unchanged and are still what `Method::refusal` answers, which is what
keeps `method::have::Researched` asking them *before* it plans a hundred
science packs.

`expand` then:

1. re-checks the whole ladder (an `expand` reachable without `applicable`
   should refuse by name, not by accident);
2. refuses `ExtractionNotModelled` when the extractor's electrical draw is not
   in `crate::state`'s `consumer_kw` table — that table's unknown name errs
   towards *permitting*, so `Condition::Powered` would read the silence as
   "draws nothing" and pass;
3. bills the machine as a `Goal::Have`, so a shortfall refuses through the
   ordinary machinery before any ground is reserved;
4. sites it, carries power, places it.

### Siting: centred on the well, and on the machine's own grid

Candidates are **every charted tile of the resource**, ordered
`(distance from the acting bot, x, y)`. There is no radius bound, for the
reason `method::power`'s `PLANT_WATER_SCAN_RADIUS` doc sets out: the walk is
already priced by `crate::schedule`, so a distant well is a worse plan rather
than an impossible one.

Two facts decide the rest, and both were nearly got wrong:

* **A pumpjack works exactly one tile while standing on nine.** Its
  `mining_drill_radius` is 0.49 (captured live in `14c54bbf`), so it must be
  *centred* on the well; its 3×3 collision box is no guide. Centring is the
  choice that is right at every radius, which is why this code reads the field
  in its doc and not in its logic — the field is `None` on every dump written
  before today, and `None` means **unknown reach, never zero reach**.
* **A resource entity always sits at a tile centre** (`(-40.5, -48.5)`, never
  `(-41, -49)`), and an even-footprint machine's build grid is tile
  *boundaries*. So a `burner-mining-drill` (2×2) **cannot be centred on a well
  at all**, and is refused by name rather than placed half a tile off where the
  game would reject it. A pumpjack's 3×3 grid is tile centres, which is exactly
  where the wells are.

`the_sited_tile_is_a_resource_entitys_own_position` asserts the site against a
resource entity's own position rather than against this module's arithmetic;
rounding the site to integers makes it fail, along with two other tests.

### Power: a plant, then a run of poles

`power::supply_for` is asked from the **well**, so it adopts a standing network
with real headroom, finishes a half-built plant, or sites a new one at water —
all three tiers unchanged. Between whatever it finds and the well, this module
lays a run of small electric poles.

The router was a straight line for exactly one measurement. On the seed-31337
dump with a well at `[80.5, -39.5]`, the line from the plant's pole hit ground
it could not stand on at `[54.3, -24.1]` and the whole cell refused — on a map
that is covered in trees, rocks and water, which is every real map. **A router
that only works on a billiard table always refuses.** It is now a bounded
best-first search over pole sites: each node offers sixteen compass directions
at three step lengths, the frontier is ordered by distance to the head, and
every candidate is a real free tile checked against the state the placements
will be made in.

It is still not a good router. It knows nothing about what the wood costs, it
will take a long way round, and it cannot share a run with another consumer.
What it has is the property that matters: it either returns a run every hop of
which stands on free ground, or it returns nothing.

**The run is verified by the game's rule, not by this module's arithmetic.**
The last check is `Condition::Powered` — a *headroom* test, since coverage is
not capacity — evaluated against a fork carrying every pole, which walks
`PlanState`'s own union-find over the poles' real wire distances.

---

## 2. What was measured

### On the real map, unperturbed

```
factorio-bot plan --world workspace/scripts/map.json \
    --goal researched:oil-processing --bots 1,2,3,4
```

```
no crude-oil is charted anywhere this plan can see ... charted ground covers
17 of 17 probes within 256 tiles of [0, 0]
```

Unchanged, and correct: the t=0 dump charts no oil. Tier 1 is preserved.

### On the real map with a crude-oil patch charted into it

`tools/inject_oil.py` writes seven `crude-oil` tiles into
`entity_graph.resources` and changes nothing else — the terrain, the entity
graph, the prototypes, the recipes and the whole technology tree are the real
dump's. Seven is the count one exploration ring actually charted on seed 31337
(`2026-09-06-exploration.md` §6); **that ring's world was never dumped**, so
this stands in for it until it is.

| well tile | plant pole → well | result |
|---|---:|---|
| `[80.5, -39.5]` | ~52 tiles | **plans**: 1,750 actions, 262,081 ticks, 10 poles, pumpjack placed |
| `[150.5, 40.5]` | ~121 tiles | refuses, `ExtractionNotModelled` |
| `[170.5, -59.5]` | ~140 tiles | refuses, `ExtractionNotModelled` |
| `[120.5, 90.5]` | ~130 tiles | refuses, `ExtractionNotModelled` |
| `[200.5, 60.5]` | — | refuses, `NoSiteFound`: *"occupied by a tree, cliff, rock or unit"* — every one of the seven tiles was under real terrain |
| `[300.5, 100.5]` | ~281 tiles | refuses, `PowerPlantNeedsWater`: *"the plan can see none within 128 tiles"* |

The last row is **the same refusal the exploration lane measured live** after
its ring charted real oil. That the perturbed dump reproduces it is the best
evidence available here that the perturbation is faithful.

The `[80.5, -39.5]` plan's pole run, read off `--steps`:

```
[38.5, -7.5] (the plant's own pole) → [39.5, -14.5] → [45.5, -16.5] →
[47.5, -22.5] → [45.5, -28.5] → [47.5, -34.5] → [53.5, -36.5] →
[57.5, -40.5] → [63.5, -42.5] → [69.5, -42.5] → [75.5, -42.5] →
[77.5, -41.5] → place pumpjack at [79.5, -39.5]
```

Every consecutive gap is between 2.24 and 7.07 tiles, inside a small pole's
7.5. The route bends around obstacles twice (the westward step to
`[45.5, -28.5]`, and the flat run along y = -42.5).

---

## 3. The ceiling, and what it would take to lift it

**`crate::state`'s `POWER_SEARCH_RADIUS` is 64 tiles.** It bounds the entities
`electric_supply_kw` looks at when answering `Condition::Powered`, and its own
doc calls it a cost bound rather than a physical limit — `EntityGraph` has no
"every entity" query and an unbounded scan per condition check would be a full
pass over the map.

What that doc does not say is what it costs, and here is what it costs: **a
pole run is useless past it, however many poles are in it.** The generator at
the far end is outside the window, `Condition::Powered` answers no, and the
placement is refused — correctly, because the scheduler checks the same
condition and would refuse it too.

So `method::extract` reaches ~64 tiles, not the ~380 that `MAX_POLE_RUN` would
allow. Seed 31337's first crude oil is charted at 256–384 tiles.

`a_pole_chain_past_the_power_search_radius_is_not_seen` pins the fact against
`PlanState` rather than against `method::extract`, and names 64 nowhere: it
asserts that a 30-tile chain carries and a 150-tile chain does not. If somebody
makes the network walk unbounded, that test fails — which is the right way
round, because that change is what would let this method reach a real oil
field.

**Three ways past it, none of them taken here:**

1. **Widen or remove `POWER_SEARCH_RADIUS`** (`crates/planner/src/state.rs`).
   The cheapest, and the one that actually unblocks oil at 300 tiles. It is a
   change to a file this task did not own. Note the cost is per
   `Condition::Powered` check, of which a plan makes many.
2. **Big electric poles.** 30-tile wire reach against a small pole's 7.5, so a
   300-tile run is 10 poles instead of 50 — but they need steel and copper
   plate, and the model still cannot *see* the far end, so this alone changes
   nothing.
3. **A second power plant at the well.** `supply_for` already tries this and it
   is what `PowerPlantNeedsWater` refuses when there is no lake. Piping water to
   a dry well is priced in `2026-09-02-building-power.md` §5 at about 15 iron
   plates per 10 tiles, which is not obviously wrong for a one-off oil cell but
   is a whole design of its own.

---

## 4. What this does not claim

* **No live run.** Every number here is a plan. Nothing has stood a pumpjack up
  in a game, and the claim that the `mine-entity` trigger fires when one
  extracts is quoted from `2026-09-05-research-triggers.md`, not re-measured.
* **The crude oil is injected.** `workspace/scripts/map.json` charts none, so
  the siting path cannot be reached on the unperturbed dump at all. When
  somebody dumps a world after an exploration ring, every row of the table in
  §2 should be re-run against it — and the `[300.5, 100.5]` row says what will
  probably happen.
* **The fixture tests share an author with the code**, and two of the fixtures
  are mine: the decoy chests in the occupied-well tests, and the engine-and-poles
  worlds in `power_reach_tests` and `a_run_whose_power_the_model_cannot_see...`.
  `world_with_oil` and its twelve wells are not — they predate this work by a
  day and belong to the refusal ladder's tests.
* **The final `Condition::Powered` check inside `pole_run` cannot be reached
  through `expand` on any fixture available**, so it is tested by calling
  `pole_run` directly with a distant anchor. Without that test the check was
  provably dead weight — removing it left the suite green, which is how it was
  found.
* **`ExtractionNotModelled` does not say *why*.** When a pole run cannot carry
  power, the refusal names the entity and the extractor and stops. It has no
  field for the distance, and adding a variant means touching
  `crates/planner/src/error.rs` and the exhaustive match in
  `scripting_lua/globals/goal/mod.rs:355`. Worth doing; not done here.

---

## 5. Falsification

Nine substitutions, each asserting its own match count before compiling —
`tools/pumpjack_falsify.py`. A `str.replace` that matches nothing leaves the suite
green and reads exactly like "the forbidden value changes nothing", which is
the trap `2026-09-06-fixtures-agree-with-their-code.md` names.

| broken on purpose | tests that went red |
|---|---|
| site rounded to an integer position | 3 |
| pole spacing widened past the wire reach | 7 |
| the build-grid guard removed | 1 |
| the route's final `Condition::Powered` check removed | 1 |
| the placement no longer states `Condition::Powered` | 1 |
| power placements no longer ordered before the extractor | 1 |
| the goal claimed whatever the recipe gate says | 1 |
| the unlock dropped instead of riding on the placement | 1 |
| an occupied tile accepted as a site | 3 |

**Two of these were hollow on the first pass** and the script said so rather
than passing: the run filter was `extract_siting_tests`, which does not match
the two later test modules, so the grid guard and the route check both came
back green with nothing having run. Fixing the filter is what turned them into
the two rows above. The check that caught it is the same one the note above
prescribes — count what ran, do not read the verdict.

---

## 6. One test moved, in a file this task did not own

`method::have::tests::an_extraction_goal_with_everything_in_place_names_the_unmodelled_cell`
asserted `ExtractionNotModelled` for a world where the well is charted, a
pumpjack mines it and its recipe is open. That is precisely the state this work
makes plannable, so the assertion could not survive. It is now
`..._is_claimed_and_bills_the_machine`: it asserts the goal is **claimed**, and
that what refuses on that fixture is the pumpjack's own bill (`fixture_world`
cannot make a steel plate). That is the only edit to `have.rs`, and it is the
only test in the workspace that broke.
