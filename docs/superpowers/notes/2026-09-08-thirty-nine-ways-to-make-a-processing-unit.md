# Thirty-nine ways to make a processing unit

2026-09-08. Branch `thirty-nine-ways-to-make-a-processing-unit`, off `master`
at `089f1a63`. Every number here was measured on a **debug** binary against
`workspace/scripts/map-31337-water-and-oil.json`, fingerprint
`a883ccef59bbcc6e`, with `tools/ambiguity_probe.sh`.

## A more accurate world produced a worse plan

The new dump is the first whose tiles carry real fluid data and whose
prototypes report `crafting_categories` on **18** machines rather than 11. Seven
machines began declaring their categories, `recycler` among them.

`MachineTable::runnable_categories` admits any category exactly one machine
declares. Exactly one machine declares `recycling`. So the day the world got
more accurate, `recycling` became a category this planner runs, and **39
`*-recycling` recipes became candidates for `processing-unit` — every candidate
it had.** Eight rungs of the rocket ladder that had been refusing for want of a
fluid supplier began refusing as `Ambiguous` instead.

Nothing was broken. A recycler really does hand back a processing unit, and the
model really did learn something true. It learned a true thing into a place
that could not use it.

## Where the existing filter was, and why it did not cover this

`products.rs`, in `inputs_this_surface_cannot_supply` — written a day earlier,
against the same category, with the reason stated in its own doc: *"a recycling
recipe as evidence of how to obtain something is circular, since you must
already have it."*

It was never applied to **choosing** a recipe because it never had to be. Until
this dump, `categories.admits` threw every recycling recipe away one line below
`sole_recipe_producing`'s candidate collection. The filter and the hole were in
the same file, forty lines apart, and the hole was invisible for exactly as long
as no machine declared the category.

**The transferable shape**: a filter that is redundant with a second mechanism
is load-bearing the moment that mechanism's input changes. Neither the filter
nor the mechanism was wrong; their overlap was doing work nobody had written
down.

## The counts, after each rule separately

Rule 1 — drop recycling recipes from the candidate set. Measured live, by
re-running each goal against a build with rule 2 switched off:

| goal | before | after rule 1 |
|---|---|---|
| `have:processing-unit:1` | 39 ambiguous | 1 candidate, `processing-unit` |
| `have:plastic-bar:10` | 9 | 2 |
| `have:sulfur:10` | 4 | 3 |
| `have:rocket-fuel:1` | 3 | 2 |

**Rule 1 was not the whole fix**, and the brief was right to suspect it might
have been. Four of the eight rungs still refused as ambiguous after it, three of
them on sulfur's remaining three.

And `processing-unit`'s 39 → 1 is not a goal that plans. The one survivor is in
`crafting-with-fluid`, which **two** assembling machines declare, so
`machine_for` refuses to choose one and the category is not runnable. Rule 1
converts a nonsense refusal into a correct one; it does not open the rung.

Rule 2 — among several runnable candidates, prefer those whose ingredients this
surface can transitively obtain. Seven of the eight rungs then stop refusing on
ambiguity:

```
researched:rocket-silo         sulfur wants 30 petroleum-gas; nothing here supplies it
have:rocket-silo:1             same
have:rocket-part:1             same
have:low-density-structure:1   same
have:plastic-bar:10            wants 20 petroleum-gas; nothing here supplies it
have:sulfur:10                 wants 30 water; nothing here supplies it
have:processing-unit:1         crafting-with-fluid is not a category this planner runs
have:rocket-fuel:1             STILL AMBIGUOUS: ammonia-rocket-fuel vs rocket-fuel-from-jelly
```

The eighth is correct behaviour, not a miss. Neither survivor is fed here — one
wants ammonia (Aquilo), one wants jelly (Gleba) — and the base `rocket-fuel`
recipe is `crafting-with-fluid`, invisible for the machine reason above. The
preference refuses to choose rather than choosing wrongly.

## "Prefer the base recipe" without a mod list

`which mod shipped this` is not a field on anything, and a hard-coded mod list
is a mod-compatibility defect by the owner's standing rule. The ingredient list
**is** a field, and it separates sulfur's producers cleanly:

- `sulfur` ← petroleum gas, water — oil is charted here, the ground yields water
- `biosulfur` ← bioflux, which closes back to Gleba fruit nothing here grows
- `advanced-carbonic-asteroid-crushing` ← a chunk that arrives from orbit

So the rule is not *"the base one"*, it is *"the one this planet can feed"* —
which happens to be the base one on Nauvis and would correctly be `biosulfur` on
Gleba. A test asserts exactly that by charting yumako instead and watching the
answer flip, which is the evidence that the rule is about the ground rather than
about a preference somebody wrote down.

Two things the closure needs, each of which a plausible cheaper version gets
wrong:

**The seed is the CHARTED resources, not the declared ones.**
`PlanState::resource_names` reads the prototype table, where this install
declares twelve — `calcite`, `scrap`, `tungsten-ore`, `lithium-brine`,
`fluorine-vent`, `sulfuric-acid-geyser` among them. A Nauvis map has six. Seed
with the declared set and `casting-iron` becomes reachable (molten iron wants
calcite), so `iron-plate` stays ambiguous between smelting and a foundry. That
was measured, not reasoned: seeding with prototypes left `iron-plate`,
`copper-plate`, `steel-plate` and `low-density-structure` all ambiguous.
`EntityGraph::resource_names_present` is the new accessor; the map's answer is
`coal, copper-ore, crude-oil, iron-ore, stone, uranium-ore`.

**Fluids must be seeded from the ground, and cannot be waved through.** Granting
every fluid for free re-breaks `iron-plate` for the same reason. Granting none
loses `sulfur`, whose recipe is petroleum gas plus water. The seed comes from
`TileFluid::named` — `LuaTilePrototype::fluid`, *"the fluid an offshore pump
produces on this tile"* — over the tiles within 256 of the origin. Not
`yields_water` and not the tile-name pair, so a modded planet whose lakes are
something else seeds that instead.

**And that is why no archived baseline could move.** Every dump written before
tiles carried `fluid` has `TileFluid::Unknown` everywhere; `named()` is `None`
for that; the seed is resources only, the ingredient closure over a
water-needing recipe fails, no candidate survives, and the preference — which is
a preference and never a filter — hands back the whole set. An unknown supply
prefers nothing.

## The baselines did not move

Four goals, one debug binary, before and after, on the dumps each was taken on:

| goal | world | actions / ticks |
|---|---|---|
| `researched:automation` | `map.json` | 176 / 21,784 |
| `producing:automation-science-pack:6` | `map.json` | 316 / 22,457 |
| `producing:logistic-science-pack:6` | `map.json` | 441 / 47,478 |
| `gathered:crude-oil` | `map-31337-explored.json` | 2,117 / 309,574 |

Identical in both runs, and identical to the figures inherited in the brief.

## The gate judges what was chosen; it cannot do the choosing

Worth recording because it looks exactly like the right place and is not.

`RecipeGate::Unobtainable` means *"disabled and no technology unlocks it"*.
`biosulfur` has an unlocking technology, so the gate answers `NeedsResearch` and
it stays a live candidate — the planner will happily bill research for a recipe
whose ingredients nothing here can make. **That gap is real and is not closed by
this work.** A fifth verdict beside `Unobtainable`, naming the unreachable
ingredient the way `BlockDrillUnfed` names the drill, would close it.

It could not have closed *this* one, and the **call sites** say so rather than
the bodies. All eleven `recipe_gate` callers are handed a recipe that has
already been chosen — by `recipe_for`, which keys on a name, or by
`recipe_producing`, which is `sole_recipe_producing`'s own caller.
`fabricate::job_for` is the shape of every one of them: it selects on line 320
and gates on line 332. The refusal these goals died of was raised before
anything was selected, by a function that holds no `PlanState` to ask a gate
with — deliberately, so that one index is valid for a whole expansion.

Found with `grep -n 'recipe_gate('` before reasoning about the body, which is
the rule this repo wrote down after two sessions reasoned about
`drills_are_fed`'s logic while its one call site two frames up was the problem.

## Two fixtures that could not express what they claimed

From `tools/falsify_ambiguity.py`, which mutates each of the eight decisions in
turn. Six went red at once. Two came back green:

- `the_seed_is_what_is_charted_plus_what_the_ground_yields` asserted
  `!supply.contains("calcite")`. **No fixture world declares calcite**, so the
  right answer and the wrong one both satisfied it. The assertion named a real
  resource off the live install and tested nothing. The fixture world declares
  six resource prototypes and spawns four patches, so `uranium-ore` and
  `crude-oil` are the discriminator it can express.
- `reachable_here`'s fixpoint could be replaced by a **single pass** with every
  test still green. The closure walks `by_recipe`, sorted by name, and every
  fixture's chain happened to be in alphabetical dependency order, so one pass
  closed them by luck.

And the sweep's own detector was wrong first: it matched `error: test failed, to
rerun ...` and reported six live mutations as uncompilable. That is precisely
the *"a mutation that fails to compile reads as green"* trap, arriving through
the detector instead of the compiler. It now tells the two apart by whether a
test run happened at all.

## What is still open

- **`crafting-with-fluid` and `advanced-crafting` are not runnable**, because
  two and three assembling machines declare them and `machine_for` refuses an
  ambiguous machine. This is a *machine* ambiguity, structurally the same
  problem one layer down, and it is what actually blocks `processing-unit`,
  `rocket-fuel` and `engine-unit`. Nothing in this branch touches it.
- **No fluid supplier is ever planned** for a `Have` that needs one. Every rung
  that got past ambiguity now stops there.
- The reachability closure is **not** wired into `recipe_gate`, so a chosen
  recipe with unreachable ingredients is still gated only on research.
