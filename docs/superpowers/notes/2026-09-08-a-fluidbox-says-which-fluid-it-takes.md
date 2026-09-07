# A fluidbox says which fluid it takes — and the pump does not

2026-09-08. Branch `a-fluidbox-says-which-fluid-it-takes`, off `a88308d1`.

The bridge now carries **which fluid each fluid box will ever accept**, read
off the prototype. It landed, it is proven live, and **it falsified the
premise it was commissioned on**. That reversal is the useful part of this
note; the field is the easy part.

## What the field is

`FactorioFluidBoxPrototype::filter`, a new `FluidFilter` on the type that
already carried `production_type` and `pipe_connections`.

It reads `LuaFluidBoxPrototype::filter`, which is an **attribute** and not a
method — checked in `runtime-api.json` for 2.1.17 rather than recalled, the
mistake that made `volume` two fields down need its `pcall` and that made
`crafting_speed` arrive nil for 1,028 prototypes.

**It is the prototype question, deliberately.** *"What will this machine ever
take at this port"*, not `LuaEntity::get_fluid_filter` (what one standing pump
is currently set to) and not the contents. The type it lives on names which
one it is, so the identifier needs no qualifier the container does not have.

## Three states, because two would have merged the interesting pair

```rust
pub enum FluidFilter {
    Only { fluid: String },   // "a boiler's output is steam"
    Any,                      // "no filter: the recipe decides"
    #[default] Unknown,       // nobody said
}
```

`Option<String>` has two slots for three states and merges the middle into the
last. `Any` is not an absence — it is the **majority** answer here (44 of 56
boxes) and the one that makes chemistry composable, because every crafting
machine's boxes are unfiltered and the recipe picks the fluid. Reading that as
"we could not tell" would have thrown away most of the field's value.

`Unknown` is the `Default`, so an archived record or an older mod parses as
*not captured* rather than asserting that every box in it is unfiltered.

Two predicates, `is_only` and `excludes`, both **positive** claims and both
false for `Any` and `Unknown` — a single `accepts(fluid) -> bool` cannot
distinguish its own two meanings.

## What the planner does with `Unknown`: nothing, and that is the point

In `method::pipe::attributable_to`, a named filter answers first: `is_only`
attributes a source outright, and *every* supplying box excluding the fluid
rejects it outright. `Any` and `Unknown` fall through to the three inferences
that ran before the field existed.

That is the only choice with no regression in it. Erring toward permitting
would make every unreadable box a phantom source; erring toward refusing would
make every archived dump — all of which predate the field, so every box in
them is `Unknown` — unplannable. **Measured, not argued**: all four offline
baselines are byte-identical before and after, on one binary, because
`map.json` carries no filters at all.

| goal | before | after |
|---|---|---|
| `researched:automation` | 176 / 21,784 | 176 / 21,784 |
| `producing:automation-science-pack:6` | 316 / 22,457 | 316 / 22,457 |
| `producing:logistic-science-pack:6` | 441 / 47,478 | 441 / 47,478 |
| `gathered:crude-oil` (explored map) | 2,115 / 317,283 | 2,115 / 317,283 |

**Where the three states are distinguishable to a reader is incomplete.** The
refusal that names a missing fluid source is built in `method::fabricate`,
which was off limits to this branch. The type keeps the states apart and the
message cannot yet say which one it hit. That is a stated gap, not a silent
one.

## It crosses the bridge — live

`world.dump` on a fresh seed-31337 headless run (four character bots, 10x,
isolated instance `headless-a`, debug build, mod symlink confirmed pointing at
this worktree by the run's own `Using mods directory` line):

```
prototypes with fluidboxes: 28    boxes: 56
filter kinds: any 44, only 12, absent 0
```

**Zero absent**, so this is not the uniform-absence signature of a stale
binary dropping a field. Both definite states occur in real data. The third,
`Unknown`, occurs in real data too — every fluid box in every dump this
project already holds, including `map.json`.

## The premise that failed: the offshore pump is unfiltered

`method::pipe::sources_of` had written down its own unblocking condition:
water has no rule because a pump carries no recipe and water is not a charted
resource, so *"it becomes reachable the day the fluidbox's accepted fluid
crosses the bridge"*. My brief repeated it. Both assumed a pump's output box
is filtered to water.

It is not:

```
offshore-pump   output any
boiler          input only=water    output only=steam
heat-exchanger  input only=water    output only=steam
chemical-plant  input any  input any  output any  output any
oil-refinery    input any  input any  output any  output any  output any
pumpjack        output any
storage-tank    none any
```

A 2.0 offshore pump takes its fluid from the **tile it stands on**
(`LuaEntity::get_fluid_source_fluid`), so `Any` is the honest answer. The only
two `only=water` boxes in the entire mod set are boiler and heat-exchanger
**inputs**, and an input supplies nothing.

**So water is exactly as unsolved as it was**, and closing it needs a tile
reading this bridge does not carry. Naming a sufficient condition for a fix is
not the same as checking it — the condition sat in a doc comment for days,
looking like a plan.

## What did close

Four prototypes have supplying boxes that are *all* filtered: `boiler` and
`heat-exchanger` (steam), `fusion-generator` (fluoroketone-hot),
`fusion-reactor` (fusion-plasma). For those, both directions now answer on a
fact:

- **steam gains a rule it never had**, for precisely water's reason — a boiler
  has no recipe, steam is not a charted resource, and it is not a buffer, so
  all three inferences failed on it and always would have;
- **the boiler is refused by name.** The bug that motivated the three
  inferences was a refinery piped to a boiler, chosen by distance because a
  boiler's steam box supplies. A box filtered `steam` definitely does not
  supply petroleum gas.

## Does sulfur get closer? Not much, and here is the honest accounting

Sulfur wants water and petroleum gas into a chemical plant.

- the plant's two input boxes are both `Any`, so **this field does not
  disambiguate them** — `PipeEnd::port_index` still assigns the nth fluid of a
  recipe to the nth box positionally, exactly as before;
- water still has no source rule;
- what sulfur does gain is a **negative**: a boiler or heat-exchanger standing
  near the site can no longer be chosen as its water or petroleum source. That
  removes a silent wrong build, which is this repo's standing failure class,
  but it does not add a plan that did not exist.

**The wall is water, and it is a tile question, not a fluidbox question.**

## Falsification

Seven new tests, each mutated one at a time, restored with `touch` (a
preserved mtime re-runs the mutated binary against restored source).

Six killed exactly one test each. **M4 — making `Any` the default — killed
two**, and the overlap is real rather than a harness fault: both
`a_record_predating_the_field_reads_as_unknown` and
`a_fluid_box_whose_filter_cannot_be_read_says_unknown_rather_than_any` assert
that an *absent* field reads as `Unknown`, reached via two different producers
(an archived record, and a mod whose read raised). No mutation separates them,
because they share that one wire fact. M3 isolates the mod half on its own.

Two harness faults worth not repeating, both of which read as a result:

- **`grep -F -c` on a multi-line pattern counts lines matching *either* line**,
  because grep splits it into alternatives. It refused two substitutions that
  were in fact unique. Count multi-line patterns in perl over the whole file.
- **A mutation that does not compile reads as green.** Removing `#[default]`
  from a `derive(Default)` enum is a compile error; a harness that greps for
  `FAILED` lines sees none and reports the test survived. A "green after a
  mutation" must be investigated, never accepted — this one was a broken
  experiment twice before it was a real one.

## Verification

- `cargo test --workspace --no-fail-fast`: **3,009 passed, 0 failed**, exit
  code taken from the command itself.
- `cargo clippy --workspace --all-features --all-targets -- --deny warnings`:
  exit 0.
- OpenAPI snapshot regenerated; `FactorioFluidBoxPrototype` is not bound in the
  TypeScript contract, so no `types.ts` mirror was needed.
- **`provenance.mods` unchanged and captured**, verified by a live run rather
  than assumed: `base 2.1.17 · elevated-rails · quality · recycler ·
  space-age · BotBridge 0.0.1`, on map digest `c161fa3f437221d0`. Note the
  field is named **`mods`**, not `mod_set` — CLAUDE.md and my brief both used a
  name the code does not carry.

## Next

1. **Water needs the tile, not the box.** `LuaEntity::get_fluid_source_fluid`
   answers what an offshore pump will produce where it stands, and
   `get_fluid_source_tile` says which tile it read. That is the fourth rule
   `sources_of` wants, and it is a different bridge crossing from this one.
2. **The refusal message should name which of the three states it hit**, which
   needs `method::fabricate`.
3. The **input** filters now crossing (`boiler` input `only=water`) have no
   reader yet. A consumer-side check — *"is this pipe delivering the fluid this
   port accepts"* — is now expressible and would catch a whole class of
   silently-correct-looking plumbing.
