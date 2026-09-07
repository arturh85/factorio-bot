# A machine chosen by what it crafts, and the wall one rung further on

2026-09-07, branch `a-machine-chosen-by-what-it-crafts`, off `6cb5f8fe`.
Sequel to `2026-09-07-a-recipe-the-planner-cannot-run.md`, which established
that the recipe-category "gate" was never a gate: it was the edge of what the
wire could name.

## What shipped

Three pieces, and the first is the one that matters.

**`method::machine::MachineTable` is the one place that answers "which machine
runs category X".** It reads `FactorioEntityPrototype::crafting_categories`,
which crossed the bridge earlier today, and it is the *same* computation
`products::Categories::planner_runs` is now built from — so the set a refusal
names and the set a method acts on cannot drift. `planner_runs()` was a
two-element constant; it takes a `&MachineTable` now.

The rule, in full:

| input | answer |
|---|---|
| `crafting` | `Machine::Hands` |
| `smelting` | `Machine::Entity("stone-furnace")` |
| a category the **character** prototype declares (`hand-crafting`) | `Machine::Hands` |
| a category exactly one placeable prototype declares | that prototype |
| a category several declare | `MachineRefusal::Ambiguous`, naming all of them |
| a category none declares | `MachineRefusal::NoMachine`, saying **which kind of nothing** |

**`method::fabricate::Fabricate` is the third method**, registered second to
last, immediately ahead of `NoProducer`. It names its machine from that table,
refuses before emitting anything, and otherwise stands the machine up:
`Have(machine, 1)`, place, `SetRecipe`, one insert per ingredient, one take,
with the machine's own `crafting_speed`-derived run time charged as a lag on
each insert.

**`substance::split_bill` has a production caller.** That was the whole point
of not wiring it speculatively last time.

## The two things this is careful about

**`None` is not an empty list, and the refusal says which it got.** Every dump
this project holds — `workspace/scripts/map.json`,
`map-31337-explored.json`, the checked-in 2.1.17 capture — predates the field,
so every prototype reads `None`. That is *the sender did not say*, and the
refusal says so in words:

```
no machine in this world model runs recipe category oil-processing: no
prototype in this world declares any crafting category at all, so the model
predates `crafting_categories` -- it did not say, which is not the same as
saying nothing crafts oil-processing
```

against, on a world that does carry the field:

```
... the world declares crafting categories and none of them is this one
```

**`parameters` is not a recipe category.** Four of the five vanilla crafting
machines report it at runtime; it appears in no data file and names no recipe.
It is filtered by name.

## The measurements

Release binary from this branch against a release binary built from `6cb5f8fe`
at the same hour, same toolchain, `--bots 1,2,3,4`.

### No baseline moved, and not by luck

| goal (`workspace/scripts/map.json`) | master | branch | branch, world annotated |
|---|---|---|---|
| `researched:automation` | 176 / 21,784 | **176 / 21,784** | **176 / 21,784** |
| `producing:automation-science-pack:6` | 316 / 22,457 | **316 / 22,457** | **316 / 22,457** |
| `producing:logistic-science-pack:6` | 441 / 47,478 | **441 / 47,478** | **441 / 47,478** |
| `gathered:crude-oil` (explored map) | 2,115 / 317,283 | **2,115 / 317,283** | — |
| `researched:oil-processing` (explored) | — | **2,012 / 308,577** | — |
| `have:oil-refinery:1` (explored) | — | **2,095 / 343,011** | — |

The third column is the strong one: even on a world where the categories *are*
declared, nothing moves. The structural reason is registration order —
`Fabricate` sits behind every existing method, so a goal that plans today
reaches an applicable method before this one is ever consulted. It can only
claim goals that previously refused.

### The wall moved

`produced:petroleum-gas:100` on the **unmodified** explored dump is byte-identical
before and after — correctly, because that world does not say what any machine
crafts:

```
5 recipes produce petroleum-gas -- ... -- and none is in a category this
planner runs (crafting, smelting). petroleum-gas is also a fluid, ...
```

So the change had to be measured on a world that carries the field. There is no
such dump and no Factorio was run for this: `scratch/map-31337-explored-with-categories.json`
is that dump with `crafting_categories` injected onto the eleven vanilla
crafting prototypes, transcribed from
`workspace/server/data/base/prototypes/entity/entities.lua` plus the
`parameters` the note above measured on a live game. **The master binary on the
same annotated file prints the old message**, which is the control that the
file changed nothing by itself.

`produced:petroleum-gas:100`, annotated world:

```
4 recipes this planner can run produce petroleum-gas -- advanced-oil-processing
(category oil-processing), basic-oil-processing (category oil-processing),
coal-liquefaction (category oil-processing), light-oil-cracking (category
chemistry) -- and nothing here can choose between them; ask for a recipe by
name rather than for the product
```

**The wall moved from "no machine" to "which recipe".** That was not predicted.
The brief expected the fluid ingredient, and the fluid ingredient is real — but
`petroleum-gas` is produced by *four* recipes whose categories are now all
runnable, so `ProductRefusal::Ambiguous` fires one tier earlier than
`Fabricate` is ever reached. The predecessor's tier-3 refusal, measured empty
in vanilla for crafting+smelting and kept anyway "so that a modded world says
so instead of picking the alphabetically first one", is the first thing the
widened set hits. It was right to keep.

`have:plastic-bar:2`, annotated world — the fluid wall, reached for real:

```
plastic-bar runs in chemical-plant (category chemistry), and this planner can
name that machine now -- but the recipe wants 20 petroleum-gas, which is a
fluid. No character inventory holds a fluid, no `InventorySlot` addresses a
fluidbox, and no action in this planner moves one, so the 20 petroleum-gas
cannot be delivered to the chemical-plant. it would come from
advanced-oil-processing, basic-oil-processing, coal-liquefaction,
empty-petroleum-gas-barrel, light-oil-cracking ...
```

Master, same world, same goal: *"9 recipes produce plastic-bar ... and none is
in a category this planner runs"*. **The machine is named, the bill was walked,
and 20 is the recipe's own ingredient amount** — the predecessor's oracle,
reproduced without editing a recipe.

### Wall three did not materialise

The predecessor's warning was that opening the category would make the message
*worse*, because `NoProducer` declines the moment a runnable recipe exists and
the driver falls through to `no method can satisfy goal`. It does not, because
`Fabricate` claims the goal and refuses by name. That is why it is a method
rather than a refusal-only observer.

## Tests, and what falsification showed

`nix develop -c cargo test --workspace` — **exit 0**, 110 test blocks
(109 on master; this branch adds one file). `cargo clippy --workspace
--all-features --all-targets -- --deny warnings` — exit 0.

`crates/planner/tests/machine_named_by_category.rs` carries six tests, each
with its **undeclared control**: the same world, the same goal, with the field
absent. Without that, "the refusal names a fluid" would be equally explained by
a world that refuses everything.

Eight breaks, one at a time, each asserting its substitution matched exactly
once and `touch`ing the restored file (`cp` preserves mtime and cargo would
otherwise re-run the mutant against restored source — CLAUDE.md, today).

| # | break | red |
|---|---|---|
| 1 | `parameters` is counted as a recipe category | 1 |
| 2 | ambiguity resolved by taking the first machine | 3 |
| 3 | an undeclared world claims it declared | 2 |
| 4 | the character does not map to hands | 2 |
| 5 | the fluid-ingredient wall removed | 1 |
| 6 | the fluid-product wall removed | 1 |
| 7 | `planner_runs` ignores the world | 4 |
| 8 | `Fabricate` unregistered in both registries | 3 |

Two results worth keeping:

**Break 1 kills only one test, and the reason is a finding.** Counting
`parameters` is nearly harmless on vanilla — four machines declare it, so it
comes out `Ambiguous` and is excluded from the runnable set anyway. The filter
only bites on a world where exactly one crafting machine exists. So the guard
is right and its blast radius is small; the test that catches it asks
`machine_for("parameters")` directly, which is the only place the difference is
visible.

**Break 8 leaves `petroleum_gas_refuses_for_a_different_reason...` green**, and
that is correct rather than a gap: that test's new message comes from
`NoProducer` reading the widened category set, not from `Fabricate`. The two
mechanisms have separate witnesses, which is what breaks 7 and 8 together
demonstrate.

## What this does not prove

* **That anything now plans that did not before.** No vanilla recipe in a
  newly-nameable category has an all-item bill: `oil-processing` and
  `chemistry` are fluids throughout, `centrifuging`'s `uranium-processing`
  needs uranium ore a character cannot hand-mine (sulfuric acid), and
  `advanced-crafting` / `crafting-with-fluid` are ambiguous. **The emit path is
  exercised only by the experiment** — `plastic-bar` with its petroleum-gas
  dropped, which places a chemical plant, sets its recipe, loads it and empties
  it. That is a real end-to-end test and it is not a live run.
* **That the emitted plan would execute.** A chemical plant is electric and
  `Fabricate` does **not** call `power::ensure_powered`. On any world where the
  emit path becomes reachable, that is the next thing to add, and it is stated
  here rather than discovered live.
* **That `oil-refinery` is the only machine a modded world would return.** It
  is the only one in vanilla; `MachineRefusal::Ambiguous` is what a modded
  world gets.

## For the owner

Two decisions, neither taken here.

1. **A fluid ingredient needs a way to reach a machine's fluidbox.** This is
   `Goal::Stored`-shaped and is already §3 of
   `2026-09-07-decisions-waiting-for-the-owner.md`. Nothing was invented: the
   refusal names the machine, the fluid, the amount and where the fluid would
   come from, and stops.
2. **`produced:<product>` cannot name a recipe.** The petroleum-gas refusal
   now says *"ask for a recipe by name rather than for the product"*, and there
   is no goal shape that lets a caller do that. Every rung above this one —
   basic versus advanced oil processing, cracking versus not — is a choice the
   planner has no vocabulary for. That is a goal-shape decision, not a method.
