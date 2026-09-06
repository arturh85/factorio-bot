# The flow graph: no caller, no refresh, and one derived rate out of three

2026-09-06. Branch `a-rate-is-not-a-constant`, worktree `.worktrees/bonuses`.
Design only — **no code in `flow_graph.rs` was changed**, deliberately, and the
last section says why that was the right answer rather than a shortfall.

This note exists because `crates/core/src/graph/flow_graph.rs` hard-codes
smelting rates that a rule in this project forbids, and because fixing that
arithmetic first would have been the wrong move by a wide margin.

---

## The rule the violation breaks

The owner's, and it is about mods before it is about correctness:

> rates get derived from the game's own data, **because that is what survives
> mods**.

A hard-coded rate is therefore a mod-compatibility defect, not merely a number
that might drift. It is the same rule that makes
`FactorioTechnologyEffect::kind` a `&str` rather than a 51-variant enum, and
the same rule that this branch's other two commits are about: a real rate is
`prototype x force bonus x module effect` and the model carried one factor.

## The violation, verified in place

Three arms in one file, and **the file already does it right in one of them**.

| arm | line | what it does |
|---|---|---|
| mining drill | `flow_graph.rs:150` | `mining_speed / mining_time` — **derived from prototypes, correct** |
| assembling machine | `flow_graph.rs:178` | `product.amount as f64 / 3.2`, with a `FIXME` on the line |
| furnace | `flow_graph.rs:719` (`furnace_output`) | `1/3.2` for iron, copper and stone; `1/16` for steel — literals, matched on `EntityName` |

So the furnace arm is wrong three ways at once, and only the third is the
arithmetic:

1. **It ignores `crafting_speed`.** A `steel-furnace` and an
   `electric-furnace` are both `crafting_speed = 2` and are reported at 1x.
2. **It is keyed on the `EntityName` enum**, so a modded ore is not "smelted
   slowly", it is `warn!("invalid furnace input")` and no edge at all.
3. **Its own test asserts the constants the function reads**
   (`steel_smelts_five_times_slower_than_iron`). That is the fourth failure
   shape in `2026-09-06-fixtures-agree-with-their-code.md`: under any
   rebalance the code and the test move together and the suite stays green.

The derivation that replaces all three is one line and both fields are in the
live 2.1.17 dump:

```
rate = product.amount * machine.crafting_speed / recipe.energy
```

`FactorioRecipe::energy` is "how long one craft takes, in seconds"
(`crates/core/src/types.rs`), `self.recipes` is already a field of `FlowGraph`,
and `method::util::machine_crafting_speed` already reads the machine side for
the planner. Nothing here is hard.

## Why fixing it now would have been a mistake — two facts, both checked

**1. Nothing reads the result.** `condense()`, `node_at()`, `inner_graph()`,
`graphviz_dot()` and `graphviz_dot_condensed()` have no caller outside
`flow_graph.rs` and its own tests — not `crates/planner`, not
`crates/executor`, not `crates/server`, not a Lua binding.

Corrected arithmetic in a function nobody calls is exactly the `method::connect`
shape this repository has already paid for: **it had no caller until 2026-09-06,
and that absence is what hid a geometry defect through four reviews** — while
the first real caller immediately exposed a materials bill that could not be
built at t=0. A fixture cannot discover what a caller discovers in one run.

**2. The graph never refreshes, and would lie without saying so.** `update()`
has exactly two callers, and both are one-shot at world initialisation:

- `OutputParser::on_init` (`output_parser.rs:580`), fired once when Factorio
  logs `initial discovery done`;
- `snapshot.rs:197`, the `--connect` path, once per snapshot.

Nothing calls it when an entity is created or destroyed, although
`FactorioWorld::on_some_entity_created` / `on_some_entity_deleted`
(`world.rs:1355`, `:1366`) maintain `entity_graph` continuously through the
parser. **The entity graph is live; the graph built from it is frozen at
initialisation.** So every machine a run builds is invisible to the flow graph,
and the first reader would get the world as it was at tick 0 — with no error,
no warning, and an answer indistinguishable in shape from a current one.

That is precisely the pattern this repo already has three entries for under
"Silence is not success": archived samples that stopped at the last closed
milestone while the mod kept sampling; the `Using mods directory` line that
printed on no run at all; walk failures archived as `kind: "other"`.

Two smaller facts that fall out of reading `update()` and that the design has
to handle:

- **It accumulates and never clears.** The walk writes into `self.inner`, which
  is never emptied, so calling `update()` a second time on a changed world adds
  nodes beside the stale ones rather than replacing them. A refresh is not
  "call `update()` more often".
- **It walks from source roots only** — graph externals that are an offshore
  pump, or a mining drill with ore under it. A world with neither produces an
  empty flow graph, which is also why the one-shot call at initialisation costs
  nothing measurable. (An earlier version of this brief said the cost was worth
  reclaiming. It is not; there is no cost.)

**A reader without the refresh is worse than no reader.** That is what makes
"add a caller" and "refresh on entity change" one piece of work.

---

## What the graph is actually for, and why it is early rather than wrong

The planner has only ever asked **"how many plates does this recipe want"** — a
bill of materials. It has never asked **"how fast can this line deliver
them"**. Every result this project produced on 2026-09-06 is the second
question, and every one of them was obtained by hand or by burning a run:

- **48 stone furnaces saturate a yellow belt** (24 if steel) — belt speed over
  smelt rate, derived by hand.
- A shared, under-supplied line gave **117 of 150 plates to the two westmost
  furnaces and 3 to the two eastmost**, because a fuel slot caps at 5 and an
  ore slot does not, so the near arm absorbs everything. That is a
  *distribution* result and it cost a live run.
- Community tables distinguish the machine count that **empties an input belt**
  from the count that **fills an output belt**; they differ whenever a recipe
  is not one-to-one. We model neither.

And the owner's standing objective is production rates over completion times,
with the rocket as the goal — which makes "where is this factory's bottleneck"
the central question rather than a nice-to-have. The abstraction is right. It
has simply never been asked anything.

---

## The design

### The smallest question worth asking first

Not "where is the bottleneck" — that needs the whole graph to be trustworthy at
once. The smallest question with a real consumer is:

> **Is this belt saturated by the machines drawing on it, and if not, how many
> more would it take?**

Concretely, one function:

```rust
/// Items per second flowing into `position`, per item name, as the standing
/// arrangement would deliver them.
pub fn throughput_at(&self, position: &Position) -> FlowRates
```

built on the `sum_incoming_edge_weights` that `update()` already uses, plus a
belt-capacity term from the prototype (`transport-belt` `speed` is
tiles/tick; 8 items per tile per lane, two lanes).

It is the smallest question because it is answerable from **one node and its
incoming edges**, so a wrong answer is localised and checkable against the
game, and because it is the exact question both of this project's measured belt
results were about.

### The consumer

`crates/planner/src/method/sustain.rs`. `Goal::Sustain{item, rate, window}` is
**already a rate over a window**, and today it proves that rate by building the
arrangement and running the game for the window. Two uses, in order of value:

1. **Refuse early.** `sustain` sites capacity and then a coal supply belt to
   feed it. If `throughput_at` the cell's input says the belt cannot carry the
   coal the cells will burn, the arrangement is refused *before* anything is
   placed — the same promise `method::connect` already makes ("it refuses
   before placing anything, because a half-built belt run is worse than none").
2. **Size the cell.** `sustain` currently takes the machine count from the
   bill. "How many burners does this coal belt feed" is the same arithmetic as
   the 48-furnaces-per-yellow-belt number, and it is the first thing the flow
   graph could tell a planner that a bill cannot.

`method::assemble` is the natural second consumer and should not be the first:
it has more geometry in it and would confound a wrong flow answer with a wrong
placement.

### The refresh, which ships with the reader or not at all

`update()` cannot simply be called more often — it accumulates. Two options,
and the second is the recommendation:

### The append is more specific than "stale nodes", and the specifics pick the fix

Verified by the peer session (`5eb090d2`) after this note was written, and it
**rules out the cheaper option**.

Re-running `update()` on an **unchanged** world is **idempotent**:
`get_or_create_flow_node` checks `node_at` first and reuses the node at that
position, and `update_flow_edge` uses petgraph's `update_edge`, which replaces a
weight rather than adding a parallel edge. **That is presumably why nobody
noticed** — the obvious test, call it twice and compare, comes back clean.

It is wrong on a **changed** world, in two ways, and only one of them is staleness:

1. **A removed entity's node and edges stay forever.** `self.inner` is built once
   in `new()` and nothing ever clears or deletes.
2. **A position reused by a different entity keeps the old `FlowNode`.**
   `node_at` matches on **position alone** and returns *before* the prototype is
   consulted — so a furnace built where a chest stood **inherits the chest's flow
   node**, with the chest's type and the chest's contents.

**(2) is why the cheap refresh is not available.** A rebuild-on-read fixes both.
A "patch the delta" refresh — visit what changed, leave the rest — fixes (1) and
**silently keeps (2)**, which is worse than the current state because it would
look maintained. Anyone optimising this later will reach for exactly that.

- **A. Rebuild on demand, cached by an entity-graph generation counter.**
  `EntityGraph` gains a monotonic `generation: AtomicU64` bumped by `add`,
  `remove` and `connect`. `FlowGraph` records the generation it was built at;
  every reader (`throughput_at`, `condense`, `node_at`) checks it and rebuilds
  from scratch if stale. `update()` clears `self.inner` first.
- **B. Incremental invalidation on entity change** — recompute only the
  affected component when `on_some_entity_created` / `on_some_entity_deleted`
  fires.

**A**, for three reasons. It is correct by construction rather than by getting
an invalidation set right; a full rebuild is cheap at the sizes this project
builds (the largest block placed to date is 179 entities, against a walk that
starts only from pumps and ore drills); and it makes staleness **structurally
impossible to observe**, which is the property the whole note is about — where
B leaves a bug class in which a reader gets a stale answer that looks current.
If a rebuild ever shows up in a profile, B is the optimisation and A is the
oracle to test it against.

The generation counter belongs to `EntityGraph`, not to `FlowGraph`, because
the entity graph is the thing that changes and it is already the single point
every mutation passes through.

### Then, and only then, the rates

With a reader and a refresh in place:

1. `furnace_output` becomes a method on `FlowGraph`, taking `&str` and the
   furnace entity, and returns `product.amount * crafting_speed / recipe.energy`
   from `self.recipes`. The `EntityName` match goes, so a modded ore works.
2. The assembler arm's `/ 3.2` and its `FIXME` go the same way — it is the same
   expression with the same two fields.
3. Coal stops being special-cased as "not a smelting input": with the recipe
   table as the authority, an item that is an ingredient of no smelting recipe
   is simply not an input, and fuel falls out of that for free rather than by
   name.

**The falsification bar is higher here, not lower**, because there is still no
game in the loop for a unit test:

- **Never assert against the constant the code reads.** The existing test
  (`steel_smelts_five_times_slower_than_iron`) does exactly that and must be
  replaced, not extended.
- Assert against **game data**: `crates/core/tests/recipes-fixtures.json` is a
  live capture and already carries `iron-plate` (`category: "smelting"`,
  `energy: 3.2`, one `iron-ore` in, one plate out) and `stone-brick`. Steel is
  not in that fixture and should be added from a capture rather than typed.
- Assert the **derivation, not the value**: feed the same graph a recipe table
  with `energy` doubled and require the reported rate to halve. A test that
  only checks `1/3.2` cannot tell a derived rate from a hard-coded one — which
  is how the current one came to certify the bug it was written beside.
- Assert `crafting_speed` is read: the same furnace with a `steel-furnace`
  prototype must report exactly 2x a `stone-furnace`.

## Recommendation

**Do it as one piece — reader, refresh and rates together — or not at all.**

Nothing in this note is difficult; the reason it was not done in this task is
that the three parts are not separable without producing something worse than
the current state. A corrected rate with no caller is unverifiable. A caller
with no refresh answers about tick 0 and says nothing about it. A refresh with
no caller is work nobody can check.

Estimated shape if picked up: the generation counter and rebuild is small and
self-contained; `throughput_at` plus a belt-capacity term is small; the rate
derivation is three expressions; the `sustain` consumer and its refusal is the
real work and is where the first live surprise will come from — as it did the
first time `method::connect` was called for real.

## What is deliberately not claimed here

- No number in this note was produced by running the flow graph. It has never
  answered a question in this project and this note does not pretend otherwise.
- The 48-furnaces-per-belt and the 117-of-150 distribution figures are from the
  2026-09-06 live notes, not from this design.
- Whether `throughput_at` is the right first question is a judgement. What is
  established rather than judged: there is no reader, there is no refresh, and
  the furnace arm derives nothing.
