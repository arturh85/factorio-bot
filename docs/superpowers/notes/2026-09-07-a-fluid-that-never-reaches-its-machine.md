# A fluid that never reaches its machine — three defects, and the number that got worse

2026-09-07. Session `fluid-edges`, branch `a-fluid-that-never-reaches-its-machine`,
off `5a5a6cd7`. Oracle: the 6:39:53 Space Age rocket-launch save, dumped as
`workspace/wrload/scripts/wr-census-status.json` (2.94 GB, `game.tick` 1,443,169).

Predecessor: `2026-09-07-the-recipe-was-in-the-dump-all-along.md`, whose §7 handed
this over with a measured hole and one explicitly-inferred claim.

**Headline: the reachability hole is real and closed, and closing it took the
flow graph's mean absolute log error from 0.141 to 1.554 — which is the most
useful thing in this note.** The 0.141 was resting on the hole: an ingredient
with no modelled producer throttles nobody, so the fourteen-item table was
scoring a model that had quietly excused itself from the entire oil chain. With
refineries in the graph, `crude-oil` is modelled at **1,380/min against
13,333/min that machines the game has configured are eating** — a supply that is
present, wrong, and now binding. See §5.

---

## 1. It was three defects, not one, and the fourth is not a defect of ours

A probe was written first, because the census table that started this work
(`oil-refinery 55 -> 0`, `chemical-plant 146 -> 32`, `steam-engine 896 -> 0`,
`biolab 80 -> 0`) cannot tell "nothing points at it" from "the walk reached it
and pruned". `why_a_machine_never_reaches_the_flow_graph` counts, per entity,
how many are in the entity graph, how many reached the flow graph, and how many
have any incoming or outgoing entity-graph edge at all.

**Before** (this binary, `connect()` re-run on the dump — see §2):

```
        entity   count   in_flow    has_in has_out  incoming neighbour types
  oil-refinery      55         0         0       0
chemical-plant     146        32        37     122  long-handed-inserter:38 fast-inserter:5
  steam-engine     896         0         0       0
        biolab      80         0        80       0  fast-inserter:80
        boiler     462        60       462       0  inserter:423 burner-inserter:30 ...
```

**After:**

```
        entity   count   in_flow    has_in has_out  incoming neighbour types
  oil-refinery      55        55        55      55  pipe:110
chemical-plant     146       146       146     146  pipe:277 chemical-plant:182 ...
  steam-engine     896         0       692       0  pipe:482 boiler:452
        biolab      80         0        80       0  fast-inserter:80
        boiler     462       459       462     452  inserter:423 burner-inserter:30 ...
```

- **Refineries and chemical plants: one defect, fixed.** 55 of 55 and 146 of
  146 now reach the flow graph.
- **The predecessor's inferred number was close and not right.** It said "the
  same rule is the likely reason 114 of 146 chemical plants are missing".
  Measured: **109** of 146 had no incoming edge at all; the other 37 did (via
  inserters) and 5 of those still failed to reach the flow graph, because
  adjacency is not reachability from a root. Inheriting 114 would have been
  inheriting a guess.
- **Steam engines: the same defect PLUS a second one, and only the first is
  fixed here.** They now have 692 incoming edges (482 from pipes, 452 from
  boilers) and still 0 flow nodes, because `FlowGraph::update` has no
  `EntityType::Boiler` and no `EntityType::Generator` source arm — the walk
  reaches a boiler (459 of 462 are flow nodes now, against 60 before) and
  `_ => Control::Prune`s there. A boiler is a fluid conversion (water in, steam
  out) and a generator is a sink; both are ordinary work and neither is in this
  change.
- **Biolabs: not this defect at all.** All 80 have an incoming edge, from a
  `fast-inserter`, **before and after** — the number does not move. They are
  unreachable because the inserters that feed them are themselves never reached
  from a root. What feeds *those* is not established here; nothing about it is
  fluid.

## 2. A dump carries the edges of the binary that wrote it

**This is why the first attempt measured nothing at all.** `EntityGraph`'s
`Deserialize` restores `entity_graph` verbatim, and no offline probe is a live
parser, so nothing re-runs `connect_node`. The corrected rule was in the binary
and the census was byte-identical, which reads exactly like a fix that does not
work.

Every `#[ignore]`d probe in `flow_graph.rs` now calls
`surface.entity_graph.connect()` after loading. It is append-only and dedupes,
so it is a no-op on a graph already wired by this binary's rules and re-derives
the difference on one that is not. **The control matters:** with the new rule
switched off, `connect()` reproduces the published table to the last digit
(0.298 / 0.184 / 0.141 / 0.141), so the dump's stored edges and this binary's
old rules agree and the "before" column is honest.

## 3. The rule is prototype-driven, and the old one is deleted

`EntityType::is_fluid_input` — `Pipe | PipeToGround | StorageTank | Boiler` — is
**gone from `types.rs`**, not extended. So are `connect_node`'s hand-written
`Pipe`, `StorageTank` and `OffshorePump` arms and the `PipeToGround` arm's
hand-written surface hop.

What replaces them reads `FactorioEntityPrototype::fluidbox_prototypes`:

- **`positions[direction]`** gives the tile; **`production_type`** gives the
  direction of flow. A joint exists where one box reaches into the other's
  footprint **both ways**, and `production_type` decides which edges are drawn.
- **`production_type` is what keeps the flow walk's roots roots**: an offshore
  pump and a pumpjack each declare a single `output` box, so nothing draws an
  edge back into them.
- **The storage tank's four joints come back identical to the four tiles the
  deleted arm listed by hand** — `(-1,-2)`, `(-2,-1)`, `(2,1)`, `(1,2)`. That
  is the strongest evidence the geometry is right: two independent derivations
  of the same four tiles.

**Two conventions, told apart by the datum itself.** A 1.x `positions` entry
names the tile *outside* the entity; a 2.0 one names the tile *on* it, with the
facing moved into `PipeConnectionDefinition::direction`. Whether the tile falls
inside the footprint says which, per connection, with no version number to read.
This was not foresight: `crates/core/tests/entity-prototype-fixtures.json` is a
**1.x capture** — its `oil-refinery` says `(-1, 3)`, a tile beyond a 5x5
footprint, and its `connection_type` values are `input`/`output`/`input-output`,
which 2.0 replaced with `normal`/`underground`. Live 2.1.17 says `(-1, 2)` and
`normal`. Both are read and both are tested.

**Where the rule is a superset, and it says so.** `FluidBoxPorts::certain` is
false when the anchor sits on a corner tile with fewer connections than sides —
a chemical plant's four connections are all corners, so each could face either
of two ways. Two cases are exact and neither is a name list: as many connections
as sides (the storage tank, and an ordinary pipe), and a box that also carries an
`underground` connection at the same anchor (an underground connector faces
`direction`, keyed on `max_underground_distance` being present rather than on
the type being `PipeToGround`). **The one field that would make it always exact
is `PipeConnectionDefinition::direction`, which the mod does not send** — see §7.

## 4. Two latent defects the new edges exposed, both fixed here

Neither is about fluids. Both were reachable only once the graph got denser.

**`FlowGraph::update` picked its roots by `externals(Direction::Incoming)`** and
filtered that to pumps and ore drills — "nothing points at it" standing in for
"it is a source". An `electric-mining-drill` declares an `input-output` fluid
box, because that is how sulfuric acid reaches a uranium drill, so a pipe beside
one now correctly draws an edge into it and the proxy deleted the drill from the
roster. **Measured, with the fluid rule in and this not yet fixed: `iron-ore`
16,500/min -> 120, `copper-ore` 15,870 -> 0, mean absolute log error `inf`.**
Roots are chosen by the predicate now. The proxy had already been quietly
wrong — a burner drill fed by an inserter has an incoming edge too — which is
visible in `coal` moving 5,130 -> 5,580/min on the raw-mining row.

**`sum_incoming_edge_weights` and `sum_incoming_edge_weights_by_side` unwrapped
`node_at`.** The `MiningDrill` arm warns and returns `Control::Continue` without
emitting an edge when the drill stands on nothing, so the walk descends past a
source that minted no flow node. It took a fluid edge into a refinery to reach
that on a real base and abort the process — `[profile.release]` sets
`panic = "abort"`. Empty is the honest answer and is what the rest of the file
already gives.

## 5. The table, and why the number got worse

Every column computed by `production_rates_of_a_dumped_world` on one binary and
one dump; the "before" column is the same binary with `fluid_boxes` returning
nothing, so nothing here is quoted from a note.

| item | game /min | before (sustained) | ratio | **after (sustained)** | **ratio** |
|---|---:|---:|---:|---:|---:|
| copper-cable | 22,367 | 22,379.7 | 1.00 | 1,552.0 | 0.07 |
| iron-ore | 15,247 | 16,500.0 | 1.08 | 16,500.0 | 1.08 |
| iron-plate | 15,170 | 16,494.0 | 1.09 | 8,278.1 | 0.55 |
| copper-ore | 15,157 | 15,870.0 | 1.05 | 15,870.0 | 1.05 |
| copper-plate | 15,147 | 14,345.4 | 0.95 | 886.7 | 0.06 |
| electronic-circuit | 6,694 | 6,570.5 | 0.98 | 494.5 | 0.07 |
| coal | 4,096 | 5,130.0 | 1.25 | 5,580.0 | 1.36 |
| plastic-bar | 2,846 | 2,102.7 | 0.74 | 52.0 | 0.02 |
| stone | 1,775 | 1,740.0 | 0.98 | 1,740.0 | 0.98 |
| steel-plate | 1,213 | 1,415.0 | 1.17 | 977.7 | 0.81 |
| advanced-circuit | 942 | 667.1 | 0.71 | 17.1 | 0.02 |
| iron-gear-wheel | 578 | 651.8 | 1.13 | 658.7 | 1.14 |
| stone-brick | 450 | 720.0 | 1.60 | 481.6 | 1.07 |
| processing-unit | 249 | 234.5 | 0.94 | 3.9 | 0.02 |
| **mean abs log error** | | | **0.141** | | **1.554** |

**The unconstrained column barely moves: `nameplate` 0.298 -> 0.306.** So the
machines, their recipes and their rates are fine. The whole of the damage is in
the *rationing*, and the provenance ledger says why in one line:

```
                        item    guessed/min      eaten/min         made/min
                   crude-oil        26666.7        13333.3           1380.0
                  heavy-oil       113400.0        13800.0           1500.0
```

**Before, `petroleum-gas` was `UNMODELLED` and throttled nobody.** Now the oil
chain has a modelled supply that is about a tenth of what its own machines eat,
and a modelled-but-low supply rations every line downstream of it — which on
this base is most of them. Outlet-bound lines go **19.4% -> 73.0%**.

So this is not a fix that made the model worse; it is a fix that **replaced an
excused unknown with a visible, quantified, wrong number**, and the wrong number
has a name: a pumpjack's rate comes from `mining_speed / mining_time` on the
`crude-oil` prototype, which **ignores the well's yield entirely**. 23 pumpjacks
at 1/s is 1,380/min. That is the next defect and it is measured, not inferred.

**`stone-brick` moves 1.60 -> 1.07 and that is NOT an improvement to bank.** It
is the same rationing that wrecked the other rows, landing by luck on the right
side of the truth for one item — the exact shape this file has recorded for
`copper-cable`'s 1.01 and `stone-brick`'s own 0.70.

## 6. Verification

- `nix develop -c cargo test --workspace` -> **exit 0**, 105 `test result: ok`
  blocks, 0 failed, with the exit code taken from the command and not from a
  pipeline. (105 is this branch's base, `5a5a6cd7`; master has since reached
  106.) `cargo clippy -p factorio-bot-core --all-targets -- --deny warnings`
  clean. `rustfmt --edition 2024` on the three files.
- Mutations, each with the substitution **counted and confirmed to occur exactly
  once** before it was applied, applied one at a time (`scratch/mutate.py`):


  | mutation | kills |
  |---|---|
  | the two `positions` conventions are told apart backwards | all 7 fluid tests + `a_drill_with_a_pipe_on_it_is_still_a_root` |
  | an `input` box also gives fluid out | `a_pipe_reaches_the_assembling_machine_the_type_list_refused`, `a_corner_the_data_cannot_settle_is_reported_as_a_superset` |
  | an `output` box also takes fluid in | `a_pipe_reaches_the_assembling_machine_the_type_list_refused`, `nothing_draws_an_edge_back_into_a_fluid_producer` |
  | drop the symmetric footprint check | `a_pipe_against_a_machines_blank_side_joins_nothing`, `a_storage_tanks_joints_are_the_ones_the_hand_written_list_had`, `a_pipe_reaches_...` |
  | a shared corner is never exact | `a_corner_with_a_connection_per_side_is_resolved_exactly` (alone) |
  | an underground connector is an ordinary box | `an_underground_connector_faces_the_way_it_points` (alone) |
  | an `underground` connection is also a surface tile | `an_underground_connector_faces_the_way_it_points` (alone) |
  | the sum readers `unwrap` again | `a_drill_standing_on_nothing_stops_the_flow_rather_than_the_process` (alone) |

  **Nothing came back green**, which is the check. Four of the eight kill one
  test alone; the first kills eight, correctly -- inverting the convention
  discriminator disables the whole rule, so it is the "does this code run at
  all" mutation rather than a behavioural one, and the four that kill exactly
  one are where the behaviour is pinned. Every negative assertion in the new
  tests is paired with a positive one from the same call, so none of them can
  pass on a wiring pass that never ran.
- Offline baselines on one release binary, before and after -- **all four byte-identical**:

  | goal | world | before | after |
  |---|---|---|---|
  | `researched:automation` | `map.json` | 176 / 21,784 | **176 / 21,784** |
  | `producing:automation-science-pack:6` | `map.json` | 316 / 22,457 | **316 / 22,457** |
  | `producing:logistic-science-pack:6` | `map.json` | 441 / 47,478 | **441 / 47,478** |
  | `gathered:crude-oil` | `map-31337-explored.json` | 2,115 / 317,283 | **2,115 / 317,283** |

  **They prove nothing about this change and are reported for that reason.**
  The flow graph still has no planner caller, and the planner's power
  connectivity follows *wires*, not pipes -- `electric_entities` walks
  `ElectricPole` neighbours and no fluid box touches it. A fluid edge is
  invisible to every one of the four. What they do rule out is the thing
  worth ruling out: that the root-predicate change in §4, which is NOT
  fluid-specific, moved a plan.
- `wiring_one_entity_at_a_time_agrees_with_one_full_sweep` still passes, so the
  known one-edge incremental/full divergence is unchanged.

## 7. Handed over, not taken

**The mod does not send `PipeConnectionDefinition::direction`.** 2.0 moved the
facing of a fluid connection out of `positions` and into `direction` (and
`flow_direction`), and `mods/BotBridge/types.lua::serialize_fluidbox_connection`
sends `positions`, `connection_type` and `max_underground_distance` only. Without
it the facing has to be recovered from geometry, which is exact on an edge tile
and ambiguous on a corner — every chemical plant, two of five refinery boxes,
and an offshore pump. The superset is emitted and flagged (`certain: false`)
rather than one of two being picked. **One field would collapse it to exact.**

**`FlowGraph::update` needs a `Boiler` and a `Generator` arm**, or 896 steam
engines stay at zero with 692 incoming edges pointing at them. Not a mod
requirement — ours, in `flow_graph.rs`.

**A pumpjack's rate must come from the well's yield**, not from
`mining_speed / mining_time`. This is the single largest error in the table
above and it was invisible until refineries entered the graph.

**Off-map supply still needs a second surface** (unchanged from the
predecessor), and **the mod still does not send a furnace's recipe** (all 1,215
report `null`).
