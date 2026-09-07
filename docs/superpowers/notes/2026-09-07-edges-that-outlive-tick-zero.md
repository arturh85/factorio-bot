# The entity graph's nodes were maintained and its edges were not

2026-09-07, branch `edges-that-outlive-tick-zero`, off `cd1a764a`.

`EntityGraph::add` inserted nodes and never wired them. `connect()` — the
method that draws every edge — ran from three places, none of which a *run*
reaches:

| caller | when |
|---|---|
| `process/output_parser.rs::on_init` | once, when Factorio logs `initial discovery done` |
| `factorio/snapshot.rs::attach_world` | the `--connect` path |
| `factorio/world.rs::FactorioSurface::import` | a bulk world install |

**Three, not two** — the brief's own note had said "two one-shot callers", and
the substance held while the count did not. Re-verified by `grep -rn
"connect()" crates/ --include='*.rs'` on this tree: the only other hits are
tests (`test_utils.rs`, `flow_graph.rs`, `sustain.rs`, `entity_graph.rs`).

So every machine, belt and inserter a run built was a node with no edges, for
ever. That was harmless while nothing read the edges. It is not harmless now:
`FlowGraph` is rebuilt off this graph's `generation` counter and has been
validated against a world-record base, and `PlanState` walks connectivity for
power. A flow graph perfectly refreshed over a wiring frozen at tick 0 is
precise about the wrong world — and a stale answer looks exactly like a current
one.

## The three questions, answered before choosing an approach

`FlowGraph` had the same defect and its fix was *not* "call `update()` again".
The same three questions decide this one, and here all three answer the other
way.

**Does `connect()` append or rebuild?** It **appends**. Nothing in it clears an
edge; it can only ever add.

**Does it dedupe?** **Yes, twice.** Every candidate is guarded by
`contains_edge` where it is gathered, and again where it is applied. Re-running
it on an unchanged world adds nothing. *(The second guard turned out to be
redundant with the first — see the falsification log below.)*

**Does anything delete?** **Yes — `remove()` does**, which is exactly what
`FlowGraph` lacked. It drops the node, both directions of its edges and the
`entity_nodes` mapping, over a `StableGraph` whose indices survive a removal.
And `node_at` resolves through `entity_tree`, which `remove` also empties, so
the position-reuse trap that forced `FlowGraph::update` to rebuild — a furnace
inheriting a chest's node because lookup matched on position alone — **cannot
happen here**.

Because all three answer that way, the fix is small: call the sweep from `add`,
over the nodes that changed rather than the whole world.

## What was done

`add` now ends with `connect_nodes_near(&added)`, before its single
`bump_generation()` — so `add` is still one mutation and the existing
`add bumps exactly once` test stays green.

`connect()` was split into `connect_nodes(nodes)` (the sweep, no generation
bump) and `connect_node(node)` (one node's rules). `connect()` is now
`connect_nodes(all nodes)` plus the bump.

**The neighbourhood, not just the new node.** An edge is drawn while visiting
its *source*, so a belt built downstream of one already standing needs the
**standing** belt re-visited. The candidate set is the new nodes plus, for each,
everything within 3 tiles of its footprint, plus any `underground-belt` or
`pipe-to-ground` within the longest `max_underground_distance` in the prototype
table (derived, not tabled — 10 in vanilla). Deduplicated through a
`BTreeSet`, so the work is proportional to what changed.

Two smaller changes fell out of calling this on every `add`:

- **`connect_node` no longer holds an `entity_tree` read guard across its
  body.** `node_at` takes that lock itself, and parking_lot documents a
  recursive read as a deadlock hazard: a writer queued between the outer and
  inner acquisition blocks both. Survivable while the sweep ran twice in a
  process; not on every `add`, on the parser thread, beside readers on others.
  The two positions it needs are copied out under a guard released at once.
- **The two "could not find entity at Drop/Pickup position" lines are `debug!`,
  not `error!`.** A drill dropping ore on the ground is ordinary. They read as
  errors only because the sweep ran twice per process; on every `add` they
  would be thousands of false alarms per run.

## Cost, measured

`the_cost_of_wiring_a_recorded_base` (ignored, gated on
`FACTORIO_BOT_WORLD_DUMP`) replays a dump's entities back through `add` in
50-entity batches — the shape the parser delivers them in — then runs one full
`connect` on top. Against `workspace/wrload/scripts/wr-census-status.json`
(2.94 GB, the world-record base), release build, same binary either side:

| | before | after |
|---|---:|---:|
| entities replayed | 39,191 | 39,191 |
| replay (784 × `add`) | 66 ms | 289 ms |
| edges after replay | 0 | 41,670 |
| one full `connect` on top | 26 ms | 28 ms |
| edges the sweep still found | 41,669 | **0** |

**+223 ms over 39,191 entities: 5.7 µs per entity, 285 µs per 50-entity
batch.** For scale, a run at eight bots spends ~308 µs per *tick* in the mod, so
one chunk batch costs about one tick's worth of that — and a run builds
hundreds of entities, not 39,000. A full sweep over the finished graph is 28 ms, so
calling `connect()` per entity would have cost on the order of 9 minutes
(28 ms x 39,191, halved for the graph growing as it goes) for the same work;
the neighbourhood is what makes this affordable, not a debounce.

**"Edges the sweep still found: 0" is the correctness result, not the timing
one.** On a real 39,191-entity base, wiring incrementally reaches the same edge
set as one full sweep. Nothing was left for the sweep to discover.

## The one place the two orders disagree — 41,670 against 41,669

Incremental wiring produced exactly **one edge more** than a from-scratch sweep,
reproducibly -- 41,670 on two runs after the change, 41,669 on three runs
with it backed out. The first hypothesis — `entity_at` answers
`results[0]` when a 0.1-tile query hits more than one entity, and quad-tree
order is not a promise — was **measured and killed**: the base has **zero**
ambiguous positions. The probe stayed in the test.

The real mechanism, reduced to three entities and asserted in
`a_half_dropped_into_a_tunnel_leaves_the_long_pair_behind`:

`connect_node`'s underground arm pairs a half with the **nearest** matching half
behind it and stops. Build an entrance and an exit four tiles apart and the pair
is drawn. Drop a third half into the gap afterwards and the two short pairs are
drawn as well — and nothing removes the long one, **because this graph only ever
appends**. A world built in one sweep has two edges there; a world built in the
order a run builds it has three.

Three things about that:

- It is **bounded**: it needs an underground half placed *between* an
  already-paired one, which is a placement that severs the pair in the game.
  One occurrence in 41,670 edges on the largest base available.
- It is **strictly better than what it replaces**. Before this change the same
  belt run had *no* edges at all.
- It is **not fixed here, deliberately.** Severing the stale pair means teaching
  the underground and pipe arms to remove an edge their own approximation
  invalidated — and the pipe arm's pairing rule (two halves facing each other,
  each searching backwards along its own direction) has never been validated
  against the game's actual pairing. Adding precision to a model whose ground
  truth is unestablished is the wrong order of work. The test names the
  divergence so it cannot be discovered again by surprise; whoever removes the
  stale edge should delete that test rather than update it.

## Falsification

Every new test was broken on purpose, one at a time, with the substitution
**asserted to match exactly once** before the run (`scratch/falsify.py`). Two of
the six attempts came back green, and per
`2026-09-06-fixtures-agree-with-their-code.md` both were treated as broken
experiments rather than findings — investigated, and both turned out to be
redundancy in the code:

| mutation | result |
|---|---|
| `add` stops calling `connect_nodes_near` | 6 tests fail — the defect restored |
| wire only the new nodes, not the neighbourhood | the same 6 fail — the neighbourhood is load-bearing |
| drop the apply-time `contains_edge` guard | **green** — redundant with the gather-time guard |
| drop *both* dedupe guards on the belt rule | 6 tests fail |
| `remove` stops removing edges explicitly | **green** — `StableGraph::remove_node` takes incident edges with it |
| `remove` stops removing edges *and* nodes | 1 test fails |

Two facts worth keeping from that: the apply-time dedupe guard and the explicit
edge-removal loop in `remove` are both dead weight, each covered by the code
next to it. Neither was touched — they are correct, just unreachable as the only
line of defence.

## Baselines

Byte-identical before and after, on release binaries built from this branch
either side of the change, on `workspace/scripts/map.json` (seed 31337 t=0,
fingerprint `c161fa3f437221d0`) except where noted:

| goal | actions | makespan |
|---|---:|---:|
| `researched:automation` | 176 | 21,784 |
| `producing:automation-science-pack:6` | 316 | 22,457 |
| `producing:logistic-science-pack:6` | 441 | 47,478 |
| `gathered:crude-oil` (`map-31337-explored.json`) | 2,115 | 317,283 |

Nothing moved, and that is the expected answer for a reason worth stating: the
offline `plan` path **deserialises** a `FactorioSurface`, edges and all, and
never calls `add`. So these baselines exercise the `connect()` refactor (which
must not change the edge set) and cannot exercise the incremental path at all.
The 39,191-entity replay above is what covers that, and it is the reason the
replay test exists rather than a smaller fixture.

## Verification

- `nix develop -c cargo test --workspace` — redirected to a file, exit code
  taken from the command: **0**, 602 passing in `factorio-bot-core` alone.
- `nix develop -c cargo clippy --workspace --all-features --all-targets --
  --deny warnings` — exit 0.
- `rustfmt --edition 2024` on the one file.
