# No chests in the line, phase 3: the output goes into a lab, and the lab researches

2026-09-09, on master `868773e9` (phase 1 merged as `76628de5`). Owner:
*"why does it have an output chest? it should output into a lab"* and
*"btw. labs can be chained with inserters."* No live run was started; every
number is an offline plan on `workspace/scripts/map.json`, four bots, one
binary built in this worktree from the commit that carries this note.

## The shape

A science pack's cell sinks into a **lab** (`assemble::Sink::Labs(n)`):

```text
        x: -3  -2  -1   0   1   2
   y  0:  =   >   ###             iron-plate belt -> arm -> I
      2:  .   .   P   v           pole (-1,2); link arm I -> P
      4:  =   >   ###             copper-plate belt -> arm -> P
      6:          ^               output arm, facing north: picks from P, drops south
      8:         LAB      p       lab 0, its pole at (2,7)
     10:          ^               lab 0 -> lab 1 (only when the chain is longer)
     12:         LAB      p
```

**South, not west, where the chest stood.** The chest's tile `(-3, 3)` has
the copper mouth's belt tile `(-3, 4)` below it, and a 3x3 lab centred
anywhere the west-face arm at `(-2, 3)` could reach covers that tile. The
product machine's south face is the one face nothing else claims. Each lab
has a pole of its own one column east (`x = 2`, below the beacon flank's
last row, `y = 5`), 5.8 tiles from the cell's pole and 4 from the next, so
the chain is wired by construction. Every further lab is one `LAB_PITCH`
(4) further south with a north-facing inserter between it and the one
before -- labs pass packs to each other, so there is no belt and no chest
anywhere in the line.

`default_sink` is `Labs(1)` for any item a technology researches with and
`Chest` for anything else. A steel or belt cell keeps its chest and
`cellstock::DrawFromCell`, which is the one case that module still serves.

## The research is fed by the cell

`have::lab_fed_research`: a research whose **one** pack a cell of this shape
makes, with a standing source for every smelted ingredient, and which is not
the technology unlocking the cell's own machine, builds the cell **inline**
(`assemble::build_cells`, the whole of `BuildAssemblyCell::expand` as a
function -- a subgoal is expanded after the method returns, and the research
action has to name the labs the cell placed). No pack is crafted, carried or
inserted; the research's preconditions are `EntityAt` and `Powered` for each
lab of the chain plus the poles and generators that supply it, and it is
linked from the action the cell is complete at (the product machine's
`SetRecipe`) with lag `units x ticks_per_item` -- the cell's tempo times the
packs, the lower bound `cellstock` already states. Multi-pack technologies
(a unit needs both packs in the same lab; two chains put them in two), the
unlocker (the bootstrap cycle), and a research with no standing source take
the hand-fed path exactly as before.

## Chain length: what the arithmetic supports

`assemble::labs_fed_by(cell_per_minute, pack, tech)`: a lab researches one
unit per `research_unit_energy` **ticks** and eats the unit's `amount` of the
pack, so it burns `amount * 3600 / energy` packs a minute; a chain is fed
from one end, labs pass forward only, and a lab hands on only what it is not
using, so the chain the cell feeds is `floor(rate / per_lab)`, one at least.
`Researched` takes the fewer of that and `labs_worth_building`.

On shipped 2.1.17 a red cell makes **6 packs a minute** (600 ticks a pack at
0.5 speed):

| research | ticks / unit | one lab burns /min | labs the cell feeds |
|---|---:|---:|---:|
| `automation` | 600 | 6 | 1 |
| `logistics` | 900 | 4 | 1 |
| `logistic-science-pack` | 300 | 12 | 1 (cell-bound) |

**One lab, for every `automation`-era research.** A chain earns a second
lab only when one lab burns fewer than half the cell's rate -- a research
slower than 1,200 ticks a unit at this cell -- or when a second cell feeds
the same chain, which is not built. The far-end starvation the coordinator
flagged (the 91/43/7/0 % gradient) therefore never arises on the chains this
commit will build, and `a_research_fed_by_a_cell_puts_no_pack_in_anyones_hands`
pins `labs_fed_by(6, red, logistics) == 1`.

## Baselines, one binary, this worktree

| goal | before (phase 1) | after |
|---|---:|---:|
| `all{sustain:iron:12, sustain:copper:6, producing:red:6}` | 1,559 / 96,221 | **1,518 / 71,341** |
| `all{sustain:iron:12, sustain:copper:6, researched:logistics}` | -- | **1,519 / 108,330** (the script's goal) |
| `all{sustain:iron:30, sustain:copper:15, producing:red:6}` | 1,997 / 95,899 | 2,048 / 107,038 |
| `all{sustain:iron:30, sustain:copper:15, researched:logistic-science-pack}` | -- | 2,049 / 168,119 |
| `researched:automation` | 176 / 21,784 | 176 / 21,784 |
| `producing:iron-plate:261` | 194 / 33,645 | 194 / 33,645 |
| `sustain:iron-plate:30` / `sustain:copper-plate:15` | 1,016 / 500 | 1,016 / 500 |
| `all{sustain:iron:30, producing:transport-belt:6}` | 1,501 / 73,252 | 1,501 / 73,252 |
| `gathered:crude-oil` (explored dump) | 2,216 / 319,933 | 2,216 / 319,933 |

The research bundle is the producing bundle **plus one action** -- the
research -- and no pack insert anywhere: 108,330 ticks is 71,341 for the
cell, 12,000 of lag for twenty packs, and 18,000 of research in one lab,
plus scheduling. `planning_work_ceilings` re-keyed to 1,518 (forks 68,966,
ceiling 2x; the belt row unchanged).

## Falsification

Copy-backed, restored by copy plus `touch`, each red on its guard:

- `labs_fed_by` over-counting by five --
  `a_research_fed_by_a_cell_puts_no_pack_in_anyones_hands` red.
- the lab-fed path never taken (`sources_stand_for` forced false) -- same
  test red (packs inserted by hand again).
- the output arm's link into the lab dropped from `links` --
  `a_product_machine_nothing_empties_does_not_count` red.
- a pack sinking into a chest (`default_sink` forced `Chest`) --
  `a_red_cell_has_no_chest_but_the_output_one` red.

## What is deliberately not done, and what is open

- **Two cells into one chain** is not built: `labs_fed_by` is per cell and a
  chain longer than one cell feeds would starve. A research at a rate one
  cell cannot supply builds two cells with a lab each.
- **Multi-pack research** (red + green) stays hand-fed until a mixed chain
  exists; green's own cell is phase 2.
- **The `producing` goal alone** now builds the lab too and leaves the packs
  to pile up in a lab nobody set to work, which is the plan's shape rather
  than a defect; `continuous_supply.lua` asks for `researched:logistics` for
  that reason.
- **Nothing here has run live.** The first run says whether a lab placed on
  the south face by the cell reads `working`, and whether the research's
  `Powered` on the lab's own pole holds on the plant the cell is wired to.
