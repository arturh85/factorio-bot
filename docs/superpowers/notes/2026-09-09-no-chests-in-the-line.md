# No chests in the line: what a chest-free science cell is, and how big it is

2026-09-09, survey only. Master `9bcf75de`. No code changed, no run started.
Everything below was read off the tree; where a claim is a guess it says so.

## The directive, and why it is a root cause rather than a preference

> "easy, have no chests, that's not something that is really ever used except
> to split off resources for the players to use. inside a factory every chest
> would be a huge bottleneck because the slow inserters at the beginning are
> way slower than a belt"

A chest is a 1x1 with four sides. Every run into or out of it spends one, and
every method that touches it has to know which sides the others still need:
`connect_steps_reserving`'s `reserved` argument, `container_sides`,
`Offtake::exit`, `product_exits`, `supply_chest_is_reachable`, the reservation
that "was blinding its own reader" (`7332b562`), "a run out of a chest leaves
by the exit kept for it" (`09edc838`), the sealed-sink message (`7762b65f`).
That is one night's work on **which of four tiles a belt may use**. None of it
exists for a 3x3 machine, whose perimeter is twelve tiles and whose sides an
inserter picks from directly.

## What is chest-shaped today, by module

Read off `assemble.rs` (6,085 lines, 51 tests -- not the ~3,000 the prompt
said), `sustain.rs`, `cellstock.rs`, `connect.rs`, `have.rs`.

**The assembly cell** (`layout_table`, north frame, `I` = intermediate machine
at the origin, `P` = product machine at `product_offset`, both 3x3):

```text
        x: -3  -2  -1   0   1
   y  0:  C   >   .  ###        FeedChest(0)  -> FeedInserter(0) -> I
      1: [C] [>]  .  ###        FeedChest(1)  -> FeedInserter(1) -> I   (green only)
      2:  .   .   P   v         Pole at (-1,2); LinkInserter I -> P
      3:  O   <   .  ###        OutputInserter P -> OutputChest
      4:  C   >   .  ###        SupplyChest   -> SupplyInserter -> P
```

Five roles are chests or arms on chests; **the gear->pack handoff is already
machine -> `LinkInserter` -> machine with no chest**, so question 3 of the
brief is answered by the tree: there is nothing to convert there, and it is
already the tightest form an inserter allows (reach 1, read off
`inserter_pickup_position`, `state.rs::inserter_reach`).

The three chests are:

| role | filled by | emptied by | mainline hop? |
|---|---|---|---|
| `FeedChest(n)` | a bot, once, `feed_charges()` | its feed arm | **yes** |
| `SupplyChest` | a bot once (ignition) **and**, since `8f43e633`, a belt from a standing stage-1 cell's plate chest | its supply arm | **yes** |
| `OutputChest` | the output arm | a bot, `cellstock::DrawFromCell`, carrying packs to a lab | a split-off for the roster -- legitimate until labs are belted |

**Upstream, `sustain`** ends in a chest too: furnace -> offtake arm ->
`BUFFER` (`iron-chest`), and `supply_link_steps` belts *out of that chest*
into the supply chest. So the copper path that ran unattended for 18 minutes
(`2026-09-09-the-cell-outlives-its-charge.md`) is

```text
drill -> furnace -> arm -> CHEST -> arm -> belt -> arm -> CHEST -> arm -> machine
```

two chests and four arms where the T-junction smelter has one arm. And the
coal side starts from a `wooden-chest` on the drill's drop tile because "a
burner mining drill has no inventory an inserter can reach into" -- true, but
a drill drops onto a **belt** just as well, so that chest is a choice, not a
law of the game.

## 1. Geometry: what the chest-free cell looks like

Red science: `automation-science-pack` = 1 `copper-plate` + 1
`iron-gear-wheel`; gear = 2 `iron-plate`. Two smelted inputs, one link.

```text
        x: -4  -3  -2  -1   0   1   2
   y  0:  =   =   >   ###             iron-plate belt ends at (-3,0); arm at (-2,0) picks west, drops into I
      1:  .   .   .   ###
      2:  .   .   P   v               pole (-1,2) unchanged; link arm I -> P unchanged
      3:  =   =   <   ###             OutputInserter (-2,3) drops onto a pack belt at (-3,3), running WEST
      4:  =   =   >   ###             copper-plate belt ends at (-3,4); arm at (-2,4) picks west, drops into P
```

Facts under it, each read off the tree rather than reasoned:

- **The pole already covers every arm in this picture.** `POLE_OFFSET`
  `(-1,2)` with `supply_area_distance` 2.5 lights `x in [-3.5, 1.5]`,
  `y in [-0.5, 4.5]`; rows 0, 3, 4 at `x = -2` are inside. That is the same
  proof `tests::the_pole_lights_the_output_mouth_but_no_third_feed_row`
  makes for the chest layout; only the thing at `x = -3` changes, from a
  chest to a belt tile, and a belt draws nothing.
- **The `MAX_FEED = 2` bound was a bound on chests, and its own doc says so**
  ("It is an artifact of CHESTS, and does not survive a belt-fed layout").
  The doc's remaining objection -- "it needs a lane model that does not
  exist" -- **does not apply here**: every run `connect_steps_with` lays
  carries exactly one item, so a belt in this cell never has two commodities
  on it. The lane model is needed for a *shared* mainline (ore + coal), which
  is `sustain`'s coal belt and not this cell.
- **Belts as sources already count as "fed".** `loaded_feeders` asks
  `delivers_into(source, inserter)`, which is "an entity covers the arm's
  pickup tile"; a `transport-belt` is an entity. `is_drained` likewise
  accepts any entity under the drop. So `cells_standing`, and with it
  `holds_assembling`, need **no change** for a belt-fed cell. Neither reads
  what a belt carries, but neither reads what a chest holds today.
- **The arms would be placed by `connect`, not by the layout table.**
  `connect_steps_with(from, to, item, inserter)` takes two `FactorioEntity`s
  and derives the arm tile from `to`'s footprint perimeter; a 3x3 gives it
  twelve candidates and it picks the first free one. `supply_link_steps`
  already does exactly this for the supply chest (a 1x1 end). Pointing the
  same call at the machine instead of a chest is the whole mechanical change
  on the feed side; the `FeedChest`/`SupplyChest`/`FeedInserter`/
  `SupplyInserter` roles then have nothing to place.
- **One risk grep cannot clear: two runs ending on adjacent tiles.** Rows 3
  and 4 above put two belt tiles at `(-3,3)` and `(-3,4)`. A belt whose
  *front* is another belt's tile side-loads into it. Neither
  `graph/route.rs` nor `connect.rs` contains the word "side-load" outside the
  underground-entry rule (`route.rs:228`), and nothing fixes a run's **last**
  tile's direction relative to a foreign neighbour. If the copper run ends
  facing north its front is the pack belt and copper rides off with the
  packs. This has to be pinned by a test before either row is belted --
  or the output goes on P's east face with a second pole, which costs a wood
  and moves nothing else.

**Green** (`logistic-science-pack` = `inserter` + `transport-belt`; belt =
`iron-plate` + `iron-gear-wheel`): the intermediate eats iron **and gears**,
and nothing makes gears -- `assembly_spec` refuses one-ingredient recipes
(`2026-09-08-there-is-no-mall`, §3) -- and the inserters are hand-crafted
into the supply chest by design. A chest-free green cell needs a gear cell
first. **Red is the honest phase-1 target; green stays partly hand-fed
whatever happens to chests.**

## 2. What replaces `CELL_CHARGE_TICKS`

Today `charge_products() = 9000 / ticks_per_item = 15` sizes every hand
charge, the boiler top-up (`fuel_for`, `cell_demand_kw` over 9,000 ticks), the
furnace-cell coal (`furnace_charge_coal`), and the output-chest `BufferGain`
ledger `cellstock` draws against. **It is a duration standing in for a
supply.** With the feed belted the supply *is* the upstream cell's rate, and
the goal already has a place to say it: `Goal::Sustain{item, rate, window}`.

So the honest replacement is not a bigger constant but a **demand**: a cell
at `per_minute` eats `per_minute * amount` of each smelted ingredient a
minute, and either

- (a) **requires** a standing source for each -- what `8f43e633` does for the
  supplied ingredient: `standing_supply_source` finds a stage-1 cell in the
  overlay, refuses by name when there is none -- so the script composes
  `goal.all{ sustain(iron-plate, 12, W), sustain(copper-plate, 6, W),
  producing(automation-science-pack, 6) }` and `All`'s in-order expansion
  puts the sources in the overlay before the cell is sited from them; or
- (b) **emits** `Goal::Sustain` for each smelted ingredient itself. Cleaner
  for the caller, but a subgoal expands *after* the method returns and the
  cell is sited **from** the source (`expand`'s own comment on why the plant
  is built inline: "a subgoal is expanded after this method returns, and the
  cell's site is chosen from the pole"). So (b) means expanding `Sustain`
  inline, which is a 3,741-line method with its own siting. Not phase 1.

What each consumer of the constant becomes:

| today | belted |
|---|---|
| `feed_charges`, `supply_charge` | gone; nothing to charge. An **ignition** charge -- one hand insert straight into the machine's input, the shape of `sustain`'s one coal -- only if the witness window (2,400 ticks) turns out shorter than a run's fill time. A 30-tile yellow belt fills in ~1,000 ticks; measure before keeping any charge |
| `fuel_for` / boiler top-up over 9,000 ticks | over the sustain **window** of the sources it links to; `Sustain` carries `window` already |
| `cellstock`'s `BufferGain = charge_products` | the cell's output over that window, `window / ticks_per_item`, capped by the slowest source's `rate * window / amount`; `cellstock`'s own doc names this as "whoever belts a cell has to revisit `BufferGain`", and says the lag edge (`ticks_per_item * take`) stays |
| a lone `producing:X:N` | **refuses by name** (no standing source for `<ingredient>`), instead of planning a cell that dies at 9:48. That is a baseline move, and it is the point |

**Does this touch `a27811f1`?** No, and explicitly: that fix is about `All`
holding a `SustainSupplyNotStanding` back until the other conjuncts have
expanded, and it is *exactly* the mechanism (a) relies on -- the sustain reads
standing, the cell expands anyway and links to it. `cells_standing` stays
structural; the false-green it closed (satisfied with no machine placed)
cannot reopen because nothing here changes what a cell must have standing.
What changes is the **class** of dead-but-standing cell: "chest empty" cannot
happen for a belted feed; "upstream sustain stalled" still can, and is
invisible to structure exactly as it is today. The witness stays.

**`producing:X:N` becomes continuous** once its machines stand and its
sources run -- bounded by the upstream rate rather than by 9,000 ticks -- but
it is *not* "no standing-cell shortcut needed": the structural predicate is
still what says a replan need not build a second cell. That is the correct
reading and it does not move.

## 3. The gear->pack handoff

Already direct (`LinkInserter` at `(0,2)`, picks north from I, drops south
into P). Nothing to do. If anything, the belt-fed layout should keep it: a
gear belt would spend a face on both machines for an item that never leaves
the cell.

## 4. How big it is, honestly

Not one file. The chest is a *protocol* between four modules and one script:

| where | what knows about the chest | phase |
|---|---|---|
| `assemble.rs` | `Role::{FeedChest, SupplyChest, FeedInserter, SupplyInserter}`, `lane()` (where a bot stands to charge), `fit`'s lane checks, `supply_chest_is_reachable`, `cell_steps`' charge inserts and `BufferGain`, `fuel_for`, `standing_supply_source` (one item), `supply_link_steps` (one chest), `chest_count`, `chest_role_name`; ~20 of 51 tests are named for a charge or a chest | **1** |
| `cellstock.rs` | the ledger bound (`charge_products`) | **1** |
| `sustain.rs` | the plate `BUFFER` and its `Offtake::exit`, `product_exits`, the exit reservation (`09edc838`, hours old) | 2: furnace -> arm -> belt, no chest; the reservation machinery becomes unnecessary rather than fixed |
| `sustain.rs` | the coal `wooden-chest` on the drill's drop tile | 2/3: drill drops onto the coal belt; arms take from the belt into every burner including the drill |
| `have.rs` / `cellstock.rs` | `OutputChest` as the lab's pack source | 3: pack belt -> arm -> lab; `lab_site_with_pole` moves |
| `connect.rs` | nothing chest-specific in the path phase 1 needs -- but **`.worktrees/replan-owns-link` has 235 uncommitted lines in it** | wait |
| `scripts/continuous_supply.lua` | composes only `sustain(copper-plate)`; needs the iron sustain | **1** |
| `tests/replan_sealed_supply.rs`, `standing_site_reuse.rs`, `planning_work_ceilings.rs` | assert on chest sites and on the fork count (80% of ceiling tonight) | **1** |

**And the in-flight worktree is on the same lines.** `replan-owns-link`
(uncommitted, `git diff --stat`: `assemble.rs +242`, `connect.rs +235`) rewrites
`fit_partial` to seed a cell from a standing chest or arm, `complete_cell` to
rank chests as seeds, and `supply_chest_is_reachable` to accept a standing
link. All three are functions phase 1 deletes or reshapes. Starting phase 1
now means one of the two lands as a conflict the other has to re-derive.

## Recommended phase 1, and why it is the slice

**Belt every smelted ingredient straight into its machine, from a standing
source, and delete the feed/supply chests. Leave `sustain`'s chests and the
output chest where they are.**

Concretely:

1. `standing_supply_source` takes the ingredient and is asked for each
   smelted input of the cell (red: `iron-plate` for I, `copper-plate` for P).
2. `supply_link_steps` calls `connect_steps_with(source_chest, machine, ..)`
   -- the machine, not a chest -- and `ensure_powered` for each arm, exactly
   as it does now. `FeedChest`, `SupplyChest` and their arms leave
   `layout_table`; `lane()` shrinks to the output chest's tile.
3. No feed charge. `fuel_for` and `BufferGain` size off the sources'
   `window`. A lone `producing:` with no source refuses by name.
4. `continuous_supply.lua` composes both sustains.
5. Tests: the side-load pin from §1 **first**, on a fixture with two runs
   ending on adjacent rows; then the layout tests re-keyed off machines.

What it buys: the 15-pack plateau goes (it is `9000/600`); two chests and two
arms per cell go; the `supply_chest_is_reachable` siting rule goes with the
chest. What it leaves: `sustain`'s two chests, which are the next bottleneck
the owner's sentence names, and the output chest as a split-off for the
roster until labs are belted.

**Do it after `replan-owns-link` lands**, from a worktree with its own
`CARGO_TARGET_DIR` and `map.json` symlinked, and expect
`producing:automation-science-pack:6` to **refuse** on `map.json` (no
source) while the composed bundle becomes the number to report. Take the
canonical table from `BASELINES.md` on the same binary before and after.
