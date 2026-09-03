# `Goal::Producing` — Design

**Status:** **PARTLY IMPLEMENTED — layers 2 and 3 ship, layer 1 does not.** Corrected 2026-09-03; this line read *"specified, not implemented"*. `Goal::Producing` is claimed by two methods (`crates/planner/src/method/have.rs:1992,1997`), answered by `holds` (`:227`), reachable from Lua as `goal.producing` (`crates/scripting_lua/src/globals/goal/value.rs:70`) and used by live scripts. The field is `per_minute: u32`, **not** the spec's `rate: f64` — changed deliberately for determinism (`crates/planner/src/goal.rs:118-128`). **Missing:** the ratio solver — no matrix, no rationals, just `ceil(per_minute * ticks_per_item / 3600)` (`crates/planner/src/method/produce.rs:150`). D1/D2's `Ax = b` solver is **not the blocker** for green science: `assembly_spec` (`crates/planner/src/method/assemble.rs:258-296`) rejects `logistic-science-pack` on *cell shape*, not on ratio arithmetic. **D4 is wrong — see the marker there.**
**Date:** 2026-09-01

## Why

`Producing { item, rate }` has existed since the first planner spec as the
documented bridge to functorio's `BusLane item rate`, with a test pinning that
no method claims it. It is the goal that means *build a factory*, and without it
the planner models manual labour: `Have` is satisfied by hand-mining and
hand-crafting, which is why a rocket is not reachable at any horizon.

`Goal::Produced` (`cf0d7bff`) is the piece that makes this tractable now.
Building a factory needs machines *made and placed*, and `Produced` already
means "cause this to come into existence" — so construction needs no new
production machinery, only somewhere to put the results.

## What it means

`Producing { item, rate }`: a standing arrangement that yields `item` at `rate`
per minute without further intervention. Distinct from both existing goals:

| Goal | Satisfied by |
|---|---|
| `Have { item, count }` | holding `count` — possession |
| `Produced { item, count }` | causing `count` to exist — one act |
| `Producing { item, rate }` | a structure that keeps producing |

## The three layers

Deliberately separated, because they fail differently and only one of them is
hard.

### Layer 1 — Ratios: rate to machine counts

**D1: solve a matrix, do not expand a recipe tree.** Tree expansion cannot
handle recipes that consume their own output — Kovarex enrichment, coal
liquefaction — and silently produces a plausible answer for them. Kirk
McDonald's method solves `Ax = b` over a recipes-by-items matrix, falling back
to a general solver for cyclic subgraphs; it is Apache-2.0 and documented in his
"Calculating Factorio" essay, so this is a port rather than a dependency.

**D2: exact rationals, never `f64`.** Rates are ratios of integers — a recipe
makes 2 items in 0.5 seconds — and stay exact under the arithmetic this needs.
`crates/planner` currently orders floats with `total_cmp` as a workaround for
`f64` not being `Ord`; rationals delete that whole class of problem from a crate
whose defining constraint is determinism. This is the one idea worth taking from
functorio, whose `Throughput` is a `Fraction` for exactly this reason.

Input: the recipe table already derived from the game. Output: for each recipe,
how many machines, and the input rates the arrangement demands.

### Layer 2 — Layout: where things go

**D3: a sparse cell grid, not a layout solver.** The optimality demand is what
makes Factorio layout exponential — the one serious free-form solver tops out
at ~100 tiles in minutes, and a CP-SAT balancer at 16x16 takes over five hours.
Dropping optimality collapses it to arithmetic: give each machine a generous
cell, lay cells in rows, run belts down the corridors between them.

This is **already validated in-game**, not hypothesised. A generated 168-entity
layout placed 168/168 against real 2.1.17 collision boxes, and a generated line
ran end to end — chest to belt to assembler to belt to chest, delivering iron
gear wheels. The cost is roughly 2.5-3x the area of a hand-packed design, which
in Factorio is free.

Three facts the spike paid for, which the layout code must encode rather than
rediscover:

- **An inserter's `direction` points at the side it picks up from**, not the
  side it drops into. Getting this backwards produces a layout that places
  100%, passes every geometry check, and moves nothing.
- **Power coverage is not power capacity.** Every consumer inside a pole's
  supply area, network connected, and the line still did nothing because
  generation was short. It does not degrade into "slow"; it reads as dead. The
  layout must check generation against demand, not just pole coverage.
- **Positions follow from one formula** — `pos = centre of the tile rect` —
  which handles the parity trap automatically: 3-wide lands on `n+1.5`, 2-wide
  on `n+1`. The same half-tile problem that made every ore mine fail.

### Layer 3 — Construction: building it

**D4: reuse `Produced`.**

> **OBSOLETE 2026-09-03 — D4 NAMES THE WRONG GOAL; DO NOT FOLLOW IT.** A bill of machines written with `Goal::Produced` **re-crafts every replan**: `Produced` deliberately ignores inventory — that is its entire reason to exist — so it does not see the machines the bot is already carrying. The shipped code uses `Goal::Have` and says so in place, naming this spec: `crates/planner/src/method/produce.rs:554-557`, *"`Goal::Have` and not `Goal::Produced`, which is where the 2026-09-01 design named the wrong goal"*. The `Place`-action half of D4 below is unaffected.

Each entity the layout calls for becomes a
`Goal::Produced { item, count }` subgoal and a `Place` action at the computed
position. Nothing new is needed to *make* the machines; `Produced` already
causes production by whichever method applies, and `Place` already exists.

The ordering falls out of the existing machinery: `Place` preconditions on
`HasItem`, and `infer_edges` turns the producing action's `GainItem` into the
edge that keeps the placement after it.

## Out of scope, and why

- **Belt routing between blocks.** Multi-net routing with no crossings is where
  both the SAT and CP formulations blow up, and no published Factorio-specific
  solution exists. v1 builds one block whose internal belts the cell grid
  determines. Single-net A* on a grid is a few hundred lines when it is needed.
- **Fluids.** Pipes cannot cross without underground pipes and throughput
  degrades with length in 2.0. A separate problem.
- **Beacons and modules.** They change the ratios, so they belong in layer 1
  once layer 1 exists, not before.
- **Verifying the built result against `FlowGraph`.** The original spec named
  this; it is the natural next step after a factory can be built at all.

## Testing

- **Ratio solver against known answers**, including at least one cyclic recipe
  (Kovarex) that tree expansion gets wrong — that case is the reason for D1 and
  should fail loudly if someone ever "simplifies" it back.
- **Exactness**: a rate whose decimal expansion repeats must round-trip, which
  is the property `f64` cannot offer.
- **Layout geometry**, mutation-proven as the spike's checks were: no tile
  overlap, no collision-box overlap, every inserter reaching both its source and
  its target, every consumer within a pole's supply area, and positions on the
  right parity for their footprint. The spike's checks needed six mutations to
  prove they discriminate; these inherit that bar.
- **Generation against demand**, as its own check. The spike had power coverage
  passing while the line was dead.
- **Live**: a `Producing` goal builds a working line and the item accumulates.
  The recording spine already measures this — splits, task lanes and frames make
  the failure legible when it does not.

## Known risk

The nearest live failure is already visible: `tried to remove 10 copper-plate
but removed 9`, a plan pulling ten plates from a furnace that made nine. That is
a smelting-time *estimate*, the same drift class as mining running 2x and walks
1.4-1.6x over prediction. A factory multiplies the number of such couplings, so
timing accuracy stops being a nuisance and becomes load-bearing. It is worth
fixing before layer 3, not after.
