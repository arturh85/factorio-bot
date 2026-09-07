# A goal that names its recipe

2026-09-07, branch `a-goal-that-names-its-recipe`, rebased onto `3399efed`.
Sequel to `2026-09-07-a-machine-chosen-by-what-it-crafts.md`, whose closing
section is this task: the planner's own refusal said *"ask for a recipe by name
rather than for the product"*, and there was no way to do it.

The owner's ruling, given three options, was **name the recipe in the goal** —
not a planner policy (*"the reason lives in code rather than in the goal"*), and
not a caller who must build every rung by hand.

## What shipped

`Goal::Have` and `Goal::Produced` gain `via: Option<RecipeName>`.

**A qualifier, not a tenth `Goal` variant**, and the code supported that reading
rather than merely permitting it: `via` is read in exactly the places
`method::have::demand` already unified the two kinds, and a variant would have
had to restate the item, the count, the holder and the unlock while adding an
arm to every exhaustive match.

### Which kinds took it, and which did not

**`Have` and `Produced`. Nothing else — and the brief's expectation was wrong
here, which is worth recording because the reason is structural.**

The brief said *"`Produced` and `Producing` obviously; `Sustain` probably;
`Have` maybe."* Measured: `Producing` and `Sustain` never reach a
product→recipe lookup at all. Every call site that resolves a product to a
recipe is one of two:

| call site | reached from |
|---|---|
| `Fabricate::job_for` → `ProductIndex::sole_recipe_producing` | `demand()`, i.e. `Have` / `Produced` only |
| `products::NoProducer::refusal` | `Have` / `Produced` only |

`method::produce` (which serves `Producing`) and `method::sustain` reach
`method::util::recipe_for`, a **recipe-name-keyed** lookup that never sees an
ambiguity and has no way to be told about one. Putting `via` on them would have
been a field nothing reads — the exact defect this repo's falsification passes
keep finding. `Charted`, `Built`, `Researched`, `Extracted`, `Gathered` and
`All` name no product at all.

So: two kinds, both of them the ones whose refusal asks for the vocabulary.

### `ProductIndex::recipe_producing`

`sole_recipe_producing` with the caller's choice honoured. **`via: None`
delegates on the first line** — the literal first statement of the body is
`return self.sole_recipe_producing(...)`, so a goal that names no recipe cannot
take a different path, and that is asserted over every product in the fixture
under two category sets rather than argued.

Four new `ProductRefusal` variants, each reachable **only** from `Some(name)`:

| variant | says |
|---|---|
| `NoSuchRecipe` | no recipe of that name here — and lists what *does* produce the product |
| `RecipeDoesNotProduce` | the recipe exists and makes something else, named |
| `NamedRecipeNotRunnable` | it produces the product; its category has no machine — quoting `MachineTable`'s own words, **the machine reason, never the ambiguity one** |
| `NamedRecipeNotHonoured` | it is `crafting`/`smelting`, whose methods pick by product name and cannot be told |

`ProductRefusal::named_recipe()` answers `Some` for exactly those four and
`None` for the three that exist without a caller naming anything — **absent is
not a value**, checked from both sides.

### It refuses before any method is asked

`method::named_recipe_refusal` sits in `expand_goal_body` beside
`fluid_have_refusal`, and for the same stated reason. A `Method::refusal` is
consulted only when *nobody* claimed the goal, and a goal like
`have:iron-gear-wheel:5 via casting-iron-gear-wheel` is claimed instantly by
`HandCraft`, which would plan the ordinary gear recipe and hand back a
schedule — the caller asking for one recipe and getting another with no
diagnostic. There is a test for the sharpest form of this: a goal the roster
*already satisfies* (200 iron plates in hand, zero actions unqualified) is still
refused when its named recipe cannot answer it.

### The surface: positional, not JSON-only

`produced:petroleum-gas:100:basic-oil-processing` — a fourth colon-separated
field on `have` and `produced`, plus `--goal-json` for free.

**Deliberate, and the usual objection does not apply.** The refusal that sends a
caller here says "ask for a recipe by name"; a remedy of "now rewrite your goal
as JSON" is a worse seam than one more colon. Mis-ordering is caught **by
type**: a count is a number and a recipe name is not, so
`produced:petroleum-gas:basic-oil-processing:100` fails loudly on the count.
Arity keeps it unambiguous — `sustain` is the only other four-part form and its
tag differs.

Lua: `goal.have(item, count, { via = "recipe-name" })`, in the `opts` table
`bot` already lives in. An **empty** `via` raises on the line that built the
goal rather than travelling to the planner as a recipe no world has. There is
no `goal.produced` in the Lua bindings at all, so `have` is the whole Lua
surface.

## Where the refusal lands once a recipe can be named

**Exactly where the brief predicted: the fluid ingredient.**

```
$ factorio-bot plan --world <annotated> --bots 1,2,3,4 \
      --goal produced:petroleum-gas:100:basic-oil-processing

Error: × the goal did not expand: basic-oil-processing runs in oil-refinery
      │ (category oil-processing), and this planner can name that machine now --
      │ but the recipe wants 100 crude-oil, which is a fluid. No character
      │ inventory holds a fluid, no `InventorySlot` addresses a fluidbox, and no
      │ action in this planner moves one, so the 100 crude-oil cannot be delivered
      │ to the oil-refinery. it would come from empty-crude-oil-barrel (category
      │ crafting-with-fluid), which no character can craft
```

`FabricateRefusal::FluidIngredient`, which is `Goal::Stored`-shaped and §3 of
`2026-09-07-decisions-waiting-for-the-owner.md`. **No fluid routing was built,
as instructed.**

The `100` is the load-bearing half: it is `basic-oil-processing`'s own
ingredient amount out of the live capture, so the bill really was walked with
the recipe already chosen. And naming a *different* recipe for the same product
reaches a *different* wall — `light-oil-cracking` with its fluid inputs dropped
lands on `FluidProduct` ("nowhere for it to land"), which is what makes this a
choice rather than one path with a decoration.

Unqualified, on the same world and the same binary, the message is **byte-identical
to master's**:

```
Error: × the goal did not expand: 4 recipes this planner can run produce petroleum-
      │ gas -- advanced-oil-processing (category oil-processing), basic-oil-
      │ processing (category oil-processing), coal-liquefaction (category oil-
      │ processing), light-oil-cracking (category chemistry) -- and nothing here
      │ can choose between them; ask for a recipe by name rather than for the
      │ product
```

Master, given the qualified spec, says `is not a goal` — which is the honest
before-picture: the vocabulary did not exist.

**And this particular ambiguity is not a mod artefact.** CLAUDE.md's Space Age
section makes the distinction: three of sulfur's four producers are Space Age,
so *"produce sulfur"* has one answer in base Factorio — but petroleum gas's four
runnable producers (`advanced-oil-processing`, `basic-oil-processing`,
`coal-liquefaction`, `light-oil-cracking`) are all base game. The goal this
vocabulary was built for is ambiguous regardless of the mod set.

## Baselines: nothing moved

Re-measured by me on **one binary each**, release, `--bots 1,2,3,4`, master at
`3399efed` and the branch rebased onto it. (Also measured against `76709a2b`
before master moved twice under me; identical both times.)

| goal | master `3399efed` | branch |
|---|---|---|
| `researched:automation` (`map.json`) | 176 / 21,784 | **176 / 21,784** |
| `producing:automation-science-pack:6` | 316 / 22,457 | **316 / 22,457** |
| `producing:logistic-science-pack:6` | 441 / 47,478 | **441 / 47,478** |
| `gathered:crude-oil` (`map-31337-explored.json`) | 2,115 / 317,283 | **2,115 / 317,283** |

`gathered:crude-oil` on `map.json` refuses on both — no oil is charted there —
which is correct and is why it is measured on the explored dump.

Three unqualified refusals on the annotated world were also diffed
master-vs-branch and are identical, including the two the peer's
`unreachable_inputs` work just landed:
`have:agricultural-science-pack:1` (the *supply* message naming bioflux and
pentapod-egg), `have:plastic-bar:2`, `have:sulfur:5`. **The coordinator's
concern that a qualifier might change which recipe is "nearest" does not
materialise**: the qualified path is a separate branch that never runs for
`via: None`, and the `NoRunnableCategory` construction it *can* reach has no
`PlanState` and leaves `unreachable_inputs` empty — the field's documented
"nothing filled it in" — rather than guessing.

## The offline basis cannot exercise the new path, so it was annotated

Every dump this project holds predates `crafting_categories`, so on them no
machine is nameable, the old messages stand, and a named recipe would refuse
with `NamedRecipeNotRunnable`'s *"it did not say"*. That is correct and it means
the archive cannot test this.

The measurements above therefore use
`scratch/map-31337-explored-with-categories.json`, the previous agent's
annotated copy, **verified rather than trusted**: diffed field-by-field against
`workspace/scripts/map-31337-explored.json`, it differs in exactly eleven
prototypes and in no other field, and each one matches
`workspace/server/data/base/prototypes/entity/entities.lua` plus the
`parameters` pseudo-category measured on a live game.

**The control is the master binary on the same file**, which prints the old
message — so the file changed nothing by itself.

## Tests

`nix develop -c cargo test --workspace` — **exit 0** captured from the command
itself, not from a pipeline: 111 test blocks, 2,977 tests, 0 failed (110 blocks
on master; this branch adds one file). `cargo clippy --workspace --all-features
--all-targets -- --deny warnings` — exit 0.

`crates/planner/tests/goal_names_its_recipe.rs` carries ten tests, built on the
same live-capture-plus-injection fixture as `machine_named_by_category.rs`, with
the undeclared control where the contrast is the claim. Three more in
`products.rs` and three in the Lua bindings; the CLI parse test gained the
positional form, the mis-ordering case and the empty-recipe case.

One clippy finding worth keeping: the new variants pushed `PlannerError` past
`clippy::result_large_err`'s 128-byte threshold, so `ProductNotMakeable` is now
boxed like `CannotFabricate` beside it. That is a cost the refusal path was
already imposing on every function in the crate that never refuses.

### Falsification

Twelve breaks, one at a time, each asserting its substitution matched **exactly
once**, each restored with a `touch` (`cp -p` preserves mtime and cargo would
re-run the mutant against restored source — CLAUDE.md, today).

| # | break | red |
|---|---|---|
| 1 | `via: None` no longer delegates to `sole_recipe_producing` | 8 |
| 2 | the pre-method guard is removed | 4 |
| 3 | a missing recipe is reported as a missing *product* | 2 |
| 4 | a recipe that does not produce the product is accepted | 3 |
| 5 | an unrunnable named recipe falls back to the ambiguity path | 3 |
| 6 | a `crafting` recipe the hand methods cannot run is accepted | 1 |
| 7 | `Fabricate` ignores the recipe the caller named | 2 |
| 8 | `Display` drops the named recipe | 1 |
| 9 | an empty Lua `via` is treated as absent | 1 |
| 10 | the CLI positional recipe field is gone | 1 |
| 11 | a share drops the recipe the caller named | 1 |
| 12 | `skip_serializing_if` is dropped | 1 |

**Three results worth keeping.**

**Breaks 8 and 11 were GREEN on the first pass, and both were real gaps.**
`Goal::Display`'s ` via <recipe>` suffix was asserted nowhere in Rust — the Lua
binding's `render_goal` was pinned and the planner's own `Display` was not, and
that string is quoted verbatim into `PlannerError::NoApplicableMethod`, so a
goal whose recipe vanished from its rendering would refuse under a description
of a *different* goal. And the `via.clone()` forwarding into `SplitAcrossBots`'
shares had no witness at all. Both now have one, and both breaks kill exactly
one test.

**The harness lied twice before it told the truth, and both lies had the same
shape — a pipeline reporting something other than what it measured.** First
`cargo test` stops at the first failing *target* unless `--no-fail-fast`, so
break 3 read "1 red" when the answer was 2 and break 1 read "2" when it was 8:
undercounts that look exactly like a narrowly-scoped test. Then the failure
regex was written as `^(\S+) \.\.\. FAILED$` when cargo prints `test <name>
... FAILED`, so *every* kill list was silently empty and the verdict came from
the exit code alone. This is the repo's own standing warning about pipelines,
found again in a new place.

**Break 6 kills exactly one test, and that one test needs an invented world.**
`NamedRecipeNotHonoured` is **unreachable on the game this project actually
runs** — Factorio 2.1.17 **with Space Age**, per CLAUDE.md's standing correction
that nothing here is base Factorio. Measured rather than assumed: over
`crates/core/tests/live-2.1.17-world-snapshot.json`, no product made by a
`crafting` or `smelting` recipe lacks a same-named one, and none has two, so
`method::util::recipe_for` is accidentally exact there, exactly as the
`products` module doc says. Space Age's own extra producers do not create the
case — `casting-*`, `*-recycling` and the asteroid crushers are all in other
categories, which is why they land on `NamedRecipeNotRunnable` instead. The test
injects a second `crafting` recipe to make the modded case exist.



## What this does not prove

* **That anything now plans that did not before.** It cannot: naming a recipe
  answers "which one", and every recipe on this install in a newly-nameable
  category still has a fluid somewhere in its bill. The qualifier moves a
  refusal one rung; it does not build a factory.
* **That the emitted plan would execute.** `Fabricate` still does not call
  `power::ensure_powered`, and a chemical plant is electric — unchanged from
  the predecessor's note, and unaddressed here.
* **That `via` is honoured by every method.** It is honoured by `Fabricate`,
  forwarded through `SplitAcrossBots`' shares and the three same-goal
  restatements in `method::have`, and **refused** where the hand-craft methods
  cannot honour it. No method below `Fabricate` was taught to select a recipe.

## For the owner

**The multi-output sink rule is still open and was not built**, per the brief:
`advanced-oil-processing` makes three fluids, and a plan must require a sink for
every output or refuse. Naming the recipe makes it possible to *ask* for
`advanced-oil-processing` specifically, which is the first time that rule has a
caller who can reach it.
