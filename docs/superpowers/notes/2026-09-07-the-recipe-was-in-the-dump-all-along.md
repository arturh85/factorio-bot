# The recipe was in the dump all along, and the missing supply is not off-map

2026-09-07. Session `off-map-supply`, branch `supply-that-comes-from-off-the-map`,
off `9da3447a`. Oracle: the 6:39:53 Space Age rocket-launch save, dumped with
`entity.status` as `workspace/wrload/scripts/wr-census-status.json` (2.94 GB,
`game.tick` 1,443,169).

Predecessor: `2026-09-07-the-mall-was-not-the-problem.md`. This corrects one
paragraph of it and leaves the rest standing.

**The brief: a predecessor reported that `recipe_making` charges `plastic-bar`
to `bioplastic` -- bioflux and yumako-mash, "which nothing on Nauvis makes".
The owner pointed out that in Space Age, goods are shipped to Nauvis from other
planets, so "nothing makes it here" does not mean "it is not available". Find
out whether the tie-break picked the recipe this factory really runs, and give
off-map supply its own answer.**

**The owner is right about the base and the correction does not save the
tie-break.** Bioflux does arrive by rocket -- this base has nine cargo landing
pads, ten rocket silos and 28 bioflux entities standing on Nauvis. It is not
what feeds the plastic. **All 24 chemical plants making plastic are set to the
`plastic-bar` recipe, and the dump says so on every one of them**: the recipe
each crafting machine is configured to run is a field on `FactorioEntity`, the
flow graph's own assembler arm already reads it to decide what comes *out* of
the machine, and `nameplate_lines` then threw it away and guessed it back from
the product's name to decide what goes *in*.

Mean absolute log error over the fourteen items the game reports: **0.141 ->
0.141**. Nothing in that table moves, and §4 says why that is the honest
result rather than a disappointing one.

---

## 1. Is bioflux feeding plastic? No, and one probe settles it

`what_recipe_each_machine_really_runs` (`#[ignore]`d, gated on
`FACTORIO_BOT_WORLD_DUMP`) censuses every crafting machine in the flow graph by
the recipe the **game** has set on it, and prints what
[`FlowGraph::recipe_making`] would have guessed instead.

```
      assembling-machine                  plastic-bar      24              bioplastic  <-- DISAGREES
      assembling-machine            uranium-processing      12   kovarex-enrichment-process  <-- DISAGREES
36 machines whose set recipe recipe_making disagrees with
-- plastic/bioplastic machine status --
                     full_output       5
        item_ingredient_shortage       1
                         working      18
```

**36 of 827 configured machines**, and those are the only two disagreements in
the whole base. Every other machine's set recipe is exactly what the name-based
guess would have picked, which is why nobody noticed for so long.

Two more facts from the same probe, both of which shape the fix:

- **Every assembling machine reports a recipe and no furnace does.** 827 report
  a name; all 1,215 furnaces (1,189 steel, 26 stone) report `null`. So
  `recipe_making` cannot be deleted -- a furnace still has to be charged by
  guessing, and the mixed-belt share `update`'s furnace arm computes is still
  the only signal there is about what a furnace spends its time on.
- **A recipe is layout, not status.** This file's standing rule is that
  `entity.status` validates a prediction and never feeds it. A crafting
  machine's recipe is not that kind of reading: it is a configuration somebody
  set, the same class of fact as where the machine stands and which way its
  inserters face. The flow graph has always used it for the machine's *output*;
  the change is that the ingredient side now agrees with the product side.

## 2. Why the tie-break could never have got it right

`recipe_making` prefers a candidate whose ingredients the base **all** supplies,
and falls back to alphabetical order when none qualifies. `plastic-bar` needs
`coal` and `petroleum-gas`. Coal is supplied. **`petroleum-gas` has no modelled
producer at all**, so `plastic-bar` failed the test, `bioplastic` failed it too,
and `bioplastic` sorts first.

The predecessor's diagnosis of the *mechanism* was exactly right and is
unchanged. What was wrong was the sentence about where the missing supply comes
from -- and the answer is not imports either.

## 3. `petroleum-gas` has no producer because the walk never reaches one

Measured, not inferred. The same probe counts each entity name in the **entity**
graph against the **flow** graph:

| entity | entity graph | flow graph |
|---|---:|---:|
| oil-refinery | 55 | **0** |
| chemical-plant | 146 | 32 |
| steam-engine | 896 | 0 |
| biolab | 80 | 0 |
| small-electric-pole | 3,162 | 0 |
| assembling-machine-2 | 752 | 665 |
| steel-furnace | 1,195 | 1,189 |

**All 55 oil refineries are set to `advanced-oil-processing`, they are all on
Nauvis, they are all in the entity graph, and not one of them is in the flow
graph.** A flow node exists only for an entity `FlowGraph::update`'s walk
reaches from a root, and the walk follows entity-graph edges. A refinery is fed
by pipe, and `EntityGraph::connect_node`'s pipe arm only draws an edge into a
`EntityType::is_fluid_input()` type -- `Pipe`, `PipeToGround`, `StorageTank`,
`Boiler`. **An assembling machine is not in that list**, so a refinery has no
incoming edge and the walk stops before it.

So "no modelled producer" here is a **reachability hole in our own graph**, not
a fact about the base and not an import. Off-map supply is real, and it is a
third cause standing beside this one; nothing in the flow graph can tell them
apart, which is the whole point of §5.

## 4. The table, and why every column is identical

Every column computed by `production_rates_of_a_dumped_world` on **one binary on
one dump**, including the before column: `nameplate_lines_by_guessing` is the
old charging kept in the test module for exactly that, beside
`balance_by_descent`, so nothing here is quoted from a note.

| item | game /min | nameplate | ratio | descent | ratio | **guessed** | **ratio** | **set recipe** | **ratio** |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| copper-cable | 22,367 | 22,680.0 | 1.01 | 19,386.3 | 0.87 | 22,379.7 | 1.00 | 22,379.7 | 1.00 |
| iron-ore | 15,247 | 16,500.0 | 1.08 | 16,500.0 | 1.08 | 16,500.0 | 1.08 | 16,500.0 | 1.08 |
| iron-plate | 15,170 | 19,016.5 | 1.25 | 13,781.3 | 0.91 | 16,494.0 | 1.09 | 16,494.0 | 1.09 |
| copper-ore | 15,157 | 15,870.0 | 1.05 | 15,870.0 | 1.05 | 15,870.0 | 1.05 | 15,870.0 | 1.05 |
| copper-plate | 15,147 | 16,579.6 | 1.09 | 12,492.6 | 0.82 | 14,345.4 | 0.95 | 14,345.4 | 0.95 |
| electronic-circuit | 6,694 | 8,820.0 | 1.32 | 5,700.0 | 0.85 | 6,570.5 | 0.98 | 6,570.5 | 0.98 |
| coal | 4,096 | 5,130.0 | 1.25 | 5,130.0 | 1.25 | 5,130.0 | 1.25 | 5,130.0 | 1.25 |
| plastic-bar | 2,846 | 2,880.0 | 1.01 | 1,825.1 | 0.64 | 2,102.7 | 0.74 | 2,102.7 | 0.74 |
| stone | 1,775 | 1,740.0 | 0.98 | 1,740.0 | 0.98 | 1,740.0 | 0.98 | 1,740.0 | 0.98 |
| steel-plate | 1,213 | 1,590.0 | 1.31 | 1,118.5 | 0.92 | 1,415.0 | 1.17 | 1,415.0 | 1.17 |
| advanced-circuit | 942 | 1,035.0 | 1.10 | 571.6 | 0.61 | 667.1 | 0.71 | 667.1 | 0.71 |
| iron-gear-wheel | 578 | 2,070.0 | 3.58 | 573.7 | 0.99 | 651.8 | 1.13 | 651.8 | 1.13 |
| stone-brick | 450 | 1,453.9 | 3.23 | 658.9 | 1.46 | 720.0 | 1.60 | 720.0 | 1.60 |
| processing-unit | 249 | 360.0 | 1.45 | 203.7 | 0.82 | 234.5 | 0.94 | 234.5 | 0.94 |
| **mean abs log error** | | | **0.298** | | **0.184** | | **0.141** | | **0.141** |

**Nothing moves. Not one of the fourteen, to the printed precision.** That is
the correct answer and the reason is arithmetic, not luck: the recipe the
plastic lines were wrongly charged to needs `bioflux` and `yumako-mash`, and the
one they are now correctly charged to needs `coal` and `petroleum-gas`. Both of
the old ingredients and one of the new ones have **no modelled producer**, so
they constrain nobody either way; the one that does, coal, is in surplus by a
factor of two. `plastic-bar` is outlet-bound, and its outlet is what *consumes*
plastic, which this change does not touch.

The predecessor said this in advance -- "it is left alone here because it moves
nothing in this table" -- and it was right. The change is worth making anyway
because it replaces a guess with a reading, and because the demand ledger in §5
was lying about 4,800 items a minute.

**So off-map supply does not explain `plastic-bar` 0.74 or `advanced-circuit`
0.71.** `what_holds_each_line_back` still says what does, unchanged for all
fourteen: all 138 advanced-circuit lines are short of `electronic-circuit`, four
hops upstream, and plastic is held by its outlet (24 lines).

**One number does move, and it is the right one.** That probe's headline count
of outlet-bound lines goes **980 of 2,173 (45.1%) -> 960 (44.2%)**: the 36
re-charged machines are no longer bound the way a fictional bill made them.
Against the **21.7%** of assemblers observed `full_output` in the same dump, the
model still over-predicts output-blocking by about a factor of two, and this
change moves that gap by a twentieth of it. It is the honest size of the effect,
not a fix for the gap.

## 5. The third state: what the model cannot account for

`sustained_production_rates` has always treated an item with no modelled
producer as **unknown rather than absent** -- otherwise every machine fed a
fluid would read as stopped. But the fact was only ever a `None` arm inside a
private function, so nothing could report it, and a reader had no way to tell
"the base makes plenty of this" from "the model has never heard of it".

Three things now say it out loud, and none of them invents a rate:

- **`Share`** replaces `supply_ratio`'s `Option<f64>`. `Share::Of(0.0)` is a
  real shortage; `Share::Unmodelled` is the admission. The two had the same
  representation before, which is the shape this repository has now recorded
  seven times -- `consumer_kw`'s unknown name, `occupant_of`'s ordering,
  `blocked_tree`'s anonymous boxes, `pole_would_supply`'s two falses.
- **`Supply`** and **`InputProvenance`**, and `FlowGraph::input_provenance()`:
  per item that modelled producers eat, how much, and either the rate modelled
  producers make or `Supply::Unmodelled`. **`Unmodelled` carries no number on
  purpose.** Saying "this comes from somewhere I cannot see and I will not
  guess how much" is the deliverable; a fabricated supply would be
  indistinguishable downstream from a measured one.
- The probe `what_the_model_cannot_account_for`, which prints the ledger with
  the old charging beside the new one.

On the record base, **6 of 45 eaten items have no modelled producer**, and the
before/after column is the whole evidence for §1:

```
                        item    guessed/min      eaten/min         made/min
                     bioflux          960.0              -                -
                 yumako-mash         3840.0              -                -
               petroleum-gas              -        28800.0       UNMODELLED
                        coal          843.8         2283.8           5130.0
                 uranium-235          218.5          160.0            224.0
                 uranium-238           27.3           20.0             68.0
                 uranium-ore              -          600.0           1365.0
                   lubricant         1890.0         1890.0       UNMODELLED
               sulfuric-acid         3600.0         3600.0       UNMODELLED
  depleted-uranium-fuel-cell          113.3          113.3       UNMODELLED
                      sulfur           75.0           75.0       UNMODELLED
                        wood           90.0           90.0       UNMODELLED
```

**4,800 items a minute of fictional demand are gone** -- 960 bioflux and 3,840
yumako-mash the base was recorded as "wanting" and makes none of. The coal row
is the arithmetic check: +1,440/min is exactly 2,880 plastic a minute at one
coal per two bars. The uranium rows are the other disagreement, twelve machines
charged `uranium-235` and `uranium-238` for what they really make out of
`uranium-ore`.

`petroleum-gas` at 28,800/min is now the largest single thing the model cannot
account for, and §3 says it is ours to fix.

## 6. What is in the tree

`crates/core/src/graph/flow_graph.rs` only. No planner, mod, `types.rs`,
`entity_graph.rs` or frontend change.

- `ProducerNameplate`, carrying the recipe the game set on each machine.
- `FlowGraph::recipe_charged_for`, which prefers that recipe when it is the
  recipe's **first** product and otherwise leaves `recipe_making` alone. The
  first-product condition is not fussiness: one `advanced-oil-processing`
  refinery is three lines here, and charging heavy oil, light oil and petroleum
  gas each the full water and crude would treble both demands.
- `Share`, `Supply`, `InputProvenance`, `FlowGraph::input_provenance` and its
  pure half `provenance_of`.
- Two probes, `#[ignore]`d: `what_recipe_each_machine_really_runs` and
  `what_the_model_cannot_account_for`.
- `nameplate_lines_by_guessing` in the test module, so the table's before column
  is computed on this binary rather than remembered.

Five mutations, each with the substitution **counted and confirmed to occur
exactly once** in the file before it was applied, and applied one at a time
(`scratch/mutate.py` in the worktree):

| mutation | breaks |
|---|---|
| never read the machine's own recipe | `the_bill_a_machine_is_charged_comes_off_the_machine` (alone) |
| ignore the set recipe in `recipe_charged_for` | `a_machine_is_charged_the_recipe_the_game_set_on_it` (+1) |
| charge a by-product the whole bill too | `a_machine_is_charged_the_recipe_the_game_set_on_it` (alone) |
| an unmodelled ingredient is a supply of zero | `an_ingredient_nothing_here_makes_is_unaccounted_for_and_not_short` (alone) |
| an unaccounted-for supply is reported as zero | `an_ingredient_nothing_here_makes_is_unaccounted_for_and_not_short` (alone) |

**The first mutation killed NOTHING on the first sweep, and that was a real
hole rather than a tidy result.** `the_bill_a_machine_is_charged_comes_off_the_
machine` charged its correct recipe to `iron-ore` -- which the fixture's own
drill mines -- so `recipe_making`'s tie-break, which prefers a candidate whose
ingredients are all supplied, picked the right recipe unaided and deleting the
read from `producer_nameplates` changed nothing at all. **The test was asserting
the right outcome for the wrong reason, exactly like the code it was written to
protect.** Changing the fixture's ingredient to one nothing there produces makes
the tie-break fall through to alphabetical order, which is the situation the
production code exists for, and the mutation then kills it alone. Nothing but
the sweep would have found that.

The first two rows are the point of having two tests rather than one: the rule
and the *reaching* of the rule are separately falsifiable, which is what
`method::connect`'s four passing reviews taught this repository to check.

Both new tests pair an accidental-value assertion with a non-accidental one from
the same call, so neither can pass because the computation did not run:
`an_ingredient_nothing_here_makes_is_unaccounted_for_and_not_short` asserts
`Unmodelled` for the import **and** `Modelled(3.0)` with the right `eaten`
beside it, and then that the import throttles nobody;
`a_machine_is_charged_the_recipe_the_game_set_on_it` asserts that the guess it
replaces really does pick the wrong recipe on that fixture before asserting that
the new rule picks the right one.

Verification, with the exit code taken from the command and not from a pipeline:

- `nix develop -c cargo test --workspace` -> exit 0.
- `cargo clippy -p factorio-bot-core --all-targets -- --deny warnings` clean.
- `rustfmt --edition 2024` on the one file.
- **The four offline baselines are byte-identical** on a release binary built
  from this branch. **They prove nothing about this change**: the flow graph
  still has no planner caller, so not one of them executes a line of it. The
  record base and the unit fixtures are the only things that do.

## 7. Handed over, not taken

Three requirements this change deliberately did not act on, because agents are
live in the files they need.

**`EntityGraph::connect_node` cannot wire a fluid into a crafting machine**
(`crates/core/src/graph/entity_graph.rs`, and `EntityType::is_fluid_input` in
`crates/core/src/types.rs`). Its pipe arm draws an edge only into `Pipe`,
`PipeToGround`, `StorageTank` or `Boiler`. **Measured:** 55 of 55 oil refineries
are absent from the flow graph, and `petroleum-gas` -- 28,800/min of demand --
therefore has no producer. **Inferred, not measured:** the same rule is the
likely reason 114 of 146 chemical plants are missing too; the 32 that are
present are reachable through their solid-ingredient inserters, and nobody has
checked the other 114 one by one. This is the single biggest gap the provenance
ledger exposes and it is entirely ours.

**The mod does not send a furnace's recipe.** All 1,215 furnaces on the record
base report `recipe: null` while every assembling machine reports a name. If
`mods/BotBridge` sent it, the mixed-belt time-share heuristic in `update`'s
furnace arm would become checkable against the game's own answer rather than
being the only signal available. Not needed for anything here; worth knowing
before someone tunes that heuristic again.

**Off-map supply cannot be modelled without a second surface.**
`mods/BotBridge/control.lua` drops every non-Nauvis chunk and `FactorioWorld`
refuses a second surface by name, so an item that genuinely arrives by rocket is
unknowable in principle today. `input_provenance` records that it is
unaccounted for and stops there, which is the right amount of claim until
surfaces land.
