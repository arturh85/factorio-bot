# The flow graph has a reader, a refresh and derived rates

2026-09-06. Branch `a-flow-graph-with-a-reader`, worktree `.worktrees/flow`,
off `93eb8289`. This is the implementation of the design in
`2026-09-06-the-flow-graph-has-no-caller-and-no-refresh.md`, which said to do
the three parts together or not at all. All three landed.

**What is established, and what is not, stated first.** Established: a reader
exists, staleness is structurally unobservable, the rates are derived from the
game's own data, and every one of the twelve new tests has been seen to go red
for the right reason. **Not established: that any of it is right about a
running game.** No Factorio was started. Every number below comes from captured
prototype and recipe fixtures, and the three offline baselines are unchanged
byte for byte — which proves the change is inert on the paths that were
measured, not that it is correct on the path it was built for.

---

## 1. The refresh: a generation counter, and a rebuild rather than a patch

`EntityGraph` gains `generation: AtomicU64`, bumped by `add`, `remove`,
`connect` and `set_recipe` (`add_blueprint_entities` bumps through `add`). It is
not serialised: a loaded graph is generation 0, and `FlowGraph`'s own
`built_generation` starts at a sentinel of `u64::MAX` that no generation can
equal, so the first read after a load rebuilds.

Every public reader on `FlowGraph` — `throughput_at`, `node_at`, `inner_graph`,
`condense`, `graphviz_dot`, `graphviz_dot_condensed` — calls `ensure_current()`
first. `update()` now **claims the generation before it walks** and clears
`inner` and `flow_tree`, which makes it re-entrant: the `node_at` calls the walk
itself makes see a current graph and do not recurse. A `parking_lot::Mutex`
makes the *decision* to rebuild single without holding `inner`'s own lock across
the walk.

**Rebuild, not delta, and the reason is not effort.** `node_at` matches on
position alone and returns before the prototype is consulted, so an entity built
where another stood inherits the old node — its type, its name, its contents. A
refresh that visited only what changed would fix the stale nodes and keep that
silently, which is worse than the frozen graph, because it would look
maintained. `an_entity_built_where_another_stood_does_not_inherit_its_flow_node`
exists to forbid it, and its fixture is hostile on purpose: two 1x1 entities at
exactly the same position, the only arrangement in which the bug can express
itself.

## 2. The reader: `throughput_at`, and two things it cannot see

```rust
pub fn throughput_at(&self, position: &Position) -> FlowRates
```

Items per second arriving at the entity standing there, per item name.

**It does not model back-pressure, and its own doc says so at length.** Every
rate is computed forwards from a source; nothing reduces an upstream rate
because a downstream one cannot accept it. The measurement that forced the
wording is this project's own, from the same day: an electric smelter whose
unloading arm sat one tile outside pole coverage delivered **0 plates to the
sink with 61 stuck in the furnaces**, and this function would have reported the
full modelled rate at both ends. Back-pressure travels backwards, so the symptom
appears upstream of the cause, and this model can see neither end of it.

**Nor does it model buffers, which is what decides fairness.** Two furnaces of
six took 78% of the ore while two took three plates between them, because a fuel
slot caps at 5 and refuses more while an ore input has no small ceiling. Coal
balanced itself; ore did not. A pure rate model predicts an even split and is
wrong.

So the doc says: read a number from here as **an upper bound under ideal
distribution**, never as evidence that a line is working. That is the "state
loudly that it does not" branch of the constraint, taken deliberately —
modelling back-pressure needs a fixed point over the whole graph and a sink
model, and neither belongs in the first reader.

## 3. The caller: `method::sustain`, and the limit that shaped it

`fed_by_machine` is the predicate that makes `Sustain` idempotent — "is
something other than a bot's hands already delivering into this machine". It was
purely **structural**: it asks where an inserter stands and which tile it drops
on, and answers the same whether the belt behind it is running or was never
connected to anything. For a goal whose entire subject is what keeps turning
with no bot in the loop, that is the wrong shape of answer.

It now composes with `flow_reaches`, which asks `throughput_at`. The composition
is **one-directional on purpose**: the predicate can only become stricter, never
looser, so the worst a wrong flow answer can do is make a replan build a feed
that already exists — which the belt primitive refuses by name — and it can
never make the method skip a feed it should have built.

### The architectural fact that shaped this, and it is the finding worth keeping

**`PlanState` is an overlay; the flow graph is built from `base().entity_graph`.
Every entity an expansion is about to place is invisible to the flow graph.**

So a flow-graph consumer inside the planner can only ever ask about what
*already stands*. That rules out the design note's more ambitious first use —
"refuse before placing anything, if the belt cannot carry the coal the cells
will burn" — because the belt in question does not exist yet and never will
during the expansion that plans it. This matters well beyond `sustain`: the
owner's direction is planning by rates, and rates live in a graph that cannot
see the plan.

The consequence in code is the rule that **an arm the flow graph has no node for
reads as fed**. Ignorance is not evidence of a dead belt — the flow walk starts
only from offshore pumps and drills on ore, so a hand-fed arm has no node either.
The veto fires only where the graph positively models the arm *and* reports
nothing reaching it, which is what a belt whose source was mined out or removed
looks like. Flipping that `true` to `false` turns three sustain tests red, which
is what says the flow graph is genuinely on the predicate's path rather than
merely referenced there.

A second, smaller use: the `SustainSupplyNotStanding` refusal now carries what
the flow graph models arriving at each standing furnace, in items/min, labelled
as an upper bound and explicitly not a verdict. It is a number in an error
string rather than in a decision because that terminus is the only place in the
method where every entity is known to *stand* — so it is the only place the flow
graph is being asked about the world it was built from. Putting the model's
number beside the run is the only way the model ever gets validated.

## 4. The rates: derived, and the enum gate gone

```
items per second = product.amount * crafting_speed / recipe.energy
```

`furnace_output` — a free function matching on `EntityName` and returning
`1/3.2` for iron, copper and stone and `1/16` for steel — is deleted. In its
place `FlowGraph::smelting_output(machine, input)` searches the recipe table for
a `smelting` recipe taking `input`, and `FlowGraph::crafting_speed(machine)`
reads the furnace's own prototype. The assembler arm's `product.amount / 3.2`
and its `FIXME` go the same way.

Three defects closed, only one of them arithmetic:

- **`crafting_speed` is read.** A `steel-furnace` and an `electric-furnace` are
  both 2 and were reported at 1x. An `assembling-machine-1` is 0.5 and was
  reported at 2x-of-nothing (`/3.2` was iron's *smelting* time applied to every
  assembler recipe).
- **A modded ore smelts.** The enum match meant an unknown input was not
  "smelted slowly", it was `warn!("invalid furnace input")` and **no edge at
  all**. This is the mod-compatibility half of the owner's rule and is the
  reason the rate had to be derived rather than corrected.
- **Coal stops being special-cased.** It is an ingredient of no smelting recipe,
  so fuel falls out of the data rather than out of a name.

Ties are broken by recipe name and warned about: `self.recipes` is a `DashMap`
whose iteration order is not stable, so an input smelted by two recipes would
otherwise make the graph non-deterministic run to run. Vanilla has no such
input; a mod may.

### The old test asserted the constants the code read

`steel_smelts_five_times_slower_than_iron` is deleted, not extended. Four tests
replace it and none of them can agree with the code by construction:

- rates asserted against **captured game data** — `crafting_speed` 1 and 2 from
  `entity-prototype-fixtures.json`, `energy` 3.2 and 16 from
  `recipes-fixtures.json`;
- **the derivation, not the value**: double a recipe's `energy` in a table
  handed to the graph and require the reported rate to halve, which a
  hard-coded rate cannot satisfy;
- **a modded ore** whose name `EntityName::from_str` genuinely rejects (asserted
  in the test) smelting at 3 products x speed 2 / 6 s;
- `steel-plate` was **not in the fixture** and was added **verbatim from
  `workspace/scripts/map.json`**, the seed-31337 t=0 dump of a live 2.1.17 game.
  Not typed.

`entity_graph_from` in `test_utils` now builds graphs with the captured recipe
table instead of an empty `DashMap`. An empty table makes every machine report
no output, which is a fixture disagreeing with the game rather than with the
code.

## Falsification: nine breaks, one at a time, each substitution asserted

Every substitution asserted it matched **exactly one** occurrence before the
suite was run, and every new test was confirmed present by name with `grep -c`
over the full un-truncated output rather than by reading a verdict.

| break | went red |
|---|---|
| `ensure_current` returns immediately | all four refresh/reader tests |
| `update()` does not clear | the removal test and the position-reuse test |
| `add` does not bump | the generation test |
| `remove` does not bump | the generation test and the removal test |
| `crafting_speed` always 1.0 | the two-speed test and the modded-ore test |
| the smelting rate is a literal again | all four rate tests |
| the recipe category filter changed | all four rate tests plus `test_furnace` |
| `flow_reaches` treats ignorance as a dead belt | **three sustain tests** |
| `flow_reaches` never refuses | the dead-arm test |

**One falsification came back green and is reported rather than absorbed.**
Removing the `&& flow_reaches(...)` clause from `fed_by_machine` entirely leaves
the whole planner suite green. So: the call is proven to be on the predicate's
path (the ignorance-rule break turns three tests red), and the refusal branch is
proven to work (`an_arm_on_a_furnace_that_only_gets_fuel_is_refused` goes red
when `flow_reaches` stops refusing) — but **no test proves the end-to-end
effect**, that a vetoed arm makes `sustain` plan a feed. Building that fixture
needs a dead arm standing at exactly a position `sustain` interrogates, and it
was not built. Per this project's own rule, a green falsification is a statement
about the test, not about the code.

## What was not done, and one thing found on the way

- **No live run.** Nothing here has been asked a question by a running game.
  The first one will find something no fixture could, the way `method::connect`'s
  first caller found an unbuildable materials bill after four clean reviews.
- **The assembler arm's derivation has no test.** There is no
  `FactorioEntity::new_assembling_machine` constructor, so no fixture reaches
  that arm. Its shared half — `crafting_speed()` — is covered; the
  `product.amount * speed / energy` expression in that arm is not.
- **Back-pressure and buffers are documented as absent, not modelled.**
- **The end-to-end veto effect is untested**, as above.

### And `EntityGraph::connect()` is as one-shot as the flow walk used to be

Found while writing the dead-arm fixture, which failed on its own precondition:
`update_chunk_entities` calls `add` and **not** `connect`. Across the whole
workspace `EntityGraph::connect()` has exactly two callers —
`OutputParser::on_init` and `factorio::snapshot` — and both fire once at world
initialisation.

So the entity graph's **edges** are frozen at tick 0 exactly as the flow graph's
walk was. An entity a run builds gets a node from `on_some_entity_created` and
**never gets an edge**, which means the flow graph, refreshed or not, will not
see the belt it was refreshed to see.

This does not make anything here wrong: a rebuild over an edgeless region
produces no flow, `flow_reaches` reads absence as ignorance rather than as a
dead belt, and the composition is one-directional. But it does mean **the flow
graph's usefulness during a run is capped one layer below this change**, and
whoever wants a live rate answer has to solve it. It is deliberately left alone:
`connect()` is a full sweep over every node, so calling it per placement is not
the fix, and the fix is a piece of work with its own design.
