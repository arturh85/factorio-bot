# A chest has four sides, and that is the whole cap

**2026-09-08.** Branch `a-cell-that-can-grow-past-one-arm`, commit `aee2f845`.
Everything below is offline, against `workspace/scripts/map.json` (seed 31337,
t=0, fingerprint `c161fa3f437221d0`), on one debug binary built from that
commit. No live run. Where a number is a model's and not the game's, it says
so.

## The question

The strategy review found that nothing in this project can say *"keep
growing"*, and gave the evidence:

```
sustain:iron-plate:15:36000    PLANS (376 actions)
sustain:iron-plate:30:36000    refuses
sustain:iron-plate:60:36000    refuses
```

with the paraphrase *"cannot ask for more than one stone furnace"*.

## Where the cap actually is

The paraphrase is right and the sweep sharpens it. `cells_for` quantises, so
`:8` and `:15` are the **same plan** -- 376 actions, 20,128 ticks, byte for
byte. 16 is the first goal that asks for a second cell:

| goal | result |
|---|---|
| `sustain:iron-plate:8:36000` | 376 actions / 20,128 ticks |
| `sustain:iron-plate:15:36000` | 376 actions / 20,128 ticks |
| `sustain:iron-plate:16:36000` | refuses |
| ... every N above | the same refusal |

**The cap is one cell.** One burner mining drill, one stone furnace,
15 iron-plate/min. There is no boundary between 15 and 30 to look for; the
boundary is between the first cell and the second.

## What the cell costs at the boundary

Placements in the 376-action plan:

| entity | count |
|---|---:|
| `transport-belt` | **79** |
| `burner-inserter` | 11 |
| `stone-furnace` | 5 (one is the cell; four are hand-smelt) |
| `iron-chest` | 3 |
| `burner-mining-drill` | 2 (one on coal, one on iron) |
| `wooden-chest` | 1 |

**101 entities, 79 of them belt, for 15 plates/min.**

## The refusal was about the wrong entity, three times over

The premise I was handed -- and the one the module's own doc invites -- is that
this is the burner-inserter fuel ceiling: *an arm that moves plates cannot take
a plate as fuel*. The message says exactly that:

> the burner-inserter at [-7.5, -32.5] makes iron-plate and nothing within
> reach can take it away: the cell's coal runs laid no belt to branch the
> offtake arm's own fuel off

**It is not that.** Three defects sat behind it, each hiding the next. Each was
found by printing what the second cell actually chose, and each was falsified
by reverting it alone.

### 1. Cell 2 adopted cell 1's PLATE chest as its coal buffer

```
furnace=[-5, -27] arm=[-5.5, -28.5] local=[0.5, -31.5]  local_fed=false
furnace=[-7, -31] arm=[-7.5, -32.5] local=[-5.5, -29.5] local_fed=true
```

`[-5.5, -29.5]` is cell 1's `hold the iron-plate the cell makes`. The
local-buffer search excluded the source buffer and *this* cell's offtake sink;
an earlier cell's sink is the same `iron-chest` and passed the filter. Two
failures follow, and the message named the second: coal would have been belted
into the chest the plates come out of, and `fed_by_machine` read that chest as
already fed -- because cell 1's offtake arm delivers into it -- so no coal run
was laid and there was no belt left to tap.

**A predicate that asks "does anything arrive here" cannot answer "does coal
arrive here".** Same family as *absent is not a value*: two different questions
returning one answer.

### 2. The tap search saw only this cell's own slice of the plan

With (1) fixed, cell 2 correctly shares cell 1's **coal** chest -- and
therefore lays no haul, and therefore its slice of `steps` is empty. The belts
were there; the search was looking at a window that excluded them.

### 3. Siting is blind to the belts the same method lays

`plan_cells` sites every cell against a fork holding the previous cells' drill
and furnace **and nothing else** -- the belts do not exist yet when it is
called. So cell 2 was packed against cell 1 as tightly as two drills allow and
then walled in by it: `no belt route, blocked by 10 tile(s)`, and every one of
the ten was a belt cell 1 had just laid.

## What is left is arithmetic, and it is the honest ceiling

With all three fixed the refusal is:

```
nothing can carry coal from the buffer at [0.5, -31.5] to the
burner-mining-drill at [-9, -31]: no belt route, blocked by 4 tile(s):
[-0.5, -31.5] [0.5, -32.5] [0.5, -30.5] [1.5, -31.5]
```

**All four of the chest's neighbours.** A 1x1 chest has four perimeter tiles;
each belt run claims one plus the cell beyond it; a cell's chest already spends
three (haul in, drill, furnace) and the source chest spends two. Giving cell 2
its own chest does not help -- the haul to it leaves the source chest, which
has the same four tiles.

**Burner inserters self-fuelling was never the constraint.** It works. The
constraint is that chest-to-chest hauling is a distribution topology with a
hard fan-out of four, and no amount of siting changes arithmetic about a 1x1
footprint.

## So: coal-branching, electric, or more cells? None of the three

This is the part I did not expect, and it came from a tool that was already in
the tree.

`factorio-bot search` ranks generated `drills x furnaces` blocks --
`search::ore_to_plate`, one belt with N drills on it and M furnaces off it --
by the flow the graph says they would sustain. Against the same map:

| candidate | plates/min (model) | entities | iron | payback |
|---|---:|---:|---:|---:|
| `gen-1x1` | 15.0 | **22** | 56 | 3.8m |
| `gen-2x2` | 30.0 | 32 | 87 | 2.9m |
| `gen-3x3` | 45.0 | 42 | 118 | 2.6m |
| `gen-4x4` | **60.0** | **52** | 148 | 2.5m |

`gen-1x1` is the rate `Sustain` caps at, in **22 entities against Sustain's
101**. And `gen-4x4` -- 60 plates/min, the goal `sustain` refuses outright --
**plans today**, through `Goal::Built`:

```
gen-4x4  ... | plan 242 actions, 12199 ticks (1740 ms)
```

| | rate | actions | ticks |
|---|---:|---:|---:|
| `sustain:iron-plate:15` | 15/min | 376 | 20,128 (5:35) |
| `goal.built(gen-4x4)` | 60/min (model) | **242** | **12,199 (3:23)** |

**Four times the rate, for 64% of the actions and 61% of the ticks.**

So the growth primitive is not a bigger cell, not cells replicated with
`Site::Beside`, and not a coal-branching scheme. It is a **wider block** -- one
belt, N drills, M furnaces, coal branched once -- and the project already has
both halves: `search::ore_to_plate` generates it as `Vec<BlueprintEntity>`,
which is exactly what `method::blueprint`'s `BuildBlock` consumes. **Nothing
wires them together.**

### The caveat that keeps this honest

`search.rs`'s own doc says **fuel is invisible to the flow graph**: a burner's
coal is not a recipe ingredient, so a block scores identically with its coal
branch deleted. `gen-4x4` carries eight output inserters that move only plates
-- the exact *"an arm that touches only plates has no fuel source"* failure
this repo documents -- and the model cannot see them starve.

**That is the synthesis.** The two halves of this project each hold half the
answer: `sustain` knows the offtake arm needs coal and pays 79 belts to route
it; `ore_to_plate` knows the efficient shape and its model is blind to fuel.
Neither is right alone, and **60 plates/min is a model number until a live run
says otherwise.**

## And the electric route is far cheaper than its reputation

Priced on the same binary and map, from t=0:

| goal | actions | makespan |
|---|---:|---|
| `have:electronic-circuit:1` | 27 | 1:20 |
| `have:inserter:1` | 29 | — |
| `have:small-electric-pole:1` | 23 | — |
| `have:offshore-pump:1` | 9 | 3:46 |
| `have:boiler:1` | 8 | 3:47 |
| `have:steam-engine:1` | 42 | 3:47 |
| **all of it at once**: `have:inserter:8` + engine + boiler + pump | **154** | **3:52** |

**154 actions buys eight electric inserters and the power station to run
them.** `Sustain` spends **376** to belt coal to *one* 15/min cell. The
electric conversion is not the expensive architectural rung this file's own
module doc implies -- on this map it is **41% of the price of the burner
workaround**, and it dissolves the offtake-arm fuel problem rather than routing
around it.

Two things stop this being a free lunch, and both are named rather than
guessed: `small-electric-pole` costs wood, which the roster happens to hold at
t=0 (`have:wood:2` plans in 0 actions) and which no method in this crate can
obtain once it runs out; and an electric *furnace* is behind oil however much
research is done, so the smelting half stays a burner regardless.

## What changed in the code, and what did not

Committed: the three defects above, in `crates/planner/src/method/sustain.rs`.
Not committed: two further changes that were tried and **reverted as
unproven** -- feeding later cells from a belt tap instead of a chest, and
choosing the tap by routability instead of by proximity. Neither reached a
two-cell plan, and unproven code with no live proof is a liability.

**A single-cell plan is byte-identical** -- 376 actions, 20,128 ticks -- which
is the control that says this is about the second cell.

All four baselines unmoved on this binary:

| goal | actions / ticks |
|---|---|
| `researched:automation` | 176 / 21,784 |
| `producing:automation-science-pack:6` | 316 / 22,457 |
| `producing:logistic-science-pack:6` | 441 / 47,478 |
| `gathered:crude-oil` (explored dump) | 2,117 / 314,345 |

## The falsification sweep, including its failure

`nix develop -c cargo test --workspace --no-fail-fast`: **exit 0, 114 suites,
3,109 passed, 0 failed.**

Each of the three fixes was reverted in place, one at a time, substitution
asserted to match exactly once:

| reverted | `cargo test -p factorio-bot-planner --lib sustain::` | `plan --world map.json --goal sustain:iron-plate:16:36000` |
|---|---|---|
| cumulative plate-chest exclusion | **GREEN** | catches it: coal buffer becomes `[-5.5, -29.5]`, cell 1's plate chest |
| whole-plan tap search | red, one test | catches it |
| siting after predecessors' belts | **GREEN** | catches it: back to `blocked by 10 tile(s)` |

**Two green mutations, and they are findings.** This module's fixture is not a
two-cell fixture -- it exercises the first cell thoroughly and the second
hardly at all, and its two patches sit differently enough that cell 2 never
reaches for cell 1's plate chest there. A fixture cannot be talked into a
geometry it does not have. The falsifier for those two is the offline `plan`
command against the real dump, and the test's own doc says so rather than
implying coverage it does not have.

## What `search.rs` should become

Not deleted, and not a second siting ranker. It should become the **generator
`Sustain` calls** when a goal asks for more than one cell's worth: rank
`(drills, furnaces)` for the requested rate, hand the winning `Layout` to
`Goal::Built`. Its ranking already has a live calibration point
(`OreToPlateThree`, 36.2 plates/min measured) and its `gen-3x2` **is** that
fixture entity for entity. The missing piece is one wire, plus a fuel model the
flow graph does not have.

That wire crosses `method::blueprint.rs` and `Site::*`, which belong to another
session, and it changes what `Goal::Sustain` decomposes to. **It is an owner
decision, not an overnight one.**

## Open, and deliberately not closed here

- **The two-cell chest ceiling is not fixed, only named.** It is arithmetic
  about a 1x1 footprint and the fix is a different topology, not a better
  search.
- **60 plates/min is a model number.** No live run. The flow graph cannot see
  the eight output inserters starve.
- **`fed_by_machine` answers "anything arrives" to the question "coal
  arrives".** Narrowing it needs the arm's cargo, which `PlanState` does not
  carry.
