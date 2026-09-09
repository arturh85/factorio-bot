# No chests in the line, phase 1: landed

2026-09-09, on `8d3cbe27`. Implements the phase 1 of
`2026-09-09-no-chests-in-the-line.md`: **a red science cell has no chest but
the output one.** Both of its plates are belted straight into the machine
that eats them from a standing stage-1 cell each, and a `producing` goal with
no such source is refused by name. No live run was started; every number here
is an offline plan on `workspace/scripts/map.json`, four bots, one binary
built in this worktree from the commit that carries this note.

## What changed, module by module

- **`assemble.rs`.** `AssemblySpec::belted` names every *smelted* ingredient
  (`produce::cell_spec` can make it from ore -- decided from the recipes, never
  from what stands, so a cell has one layout on every replan). A belted
  ingredient has no `FeedChest`/`SupplyChest` and no arm in `layout_table`;
  its row is a `Mouth` -- the arm tile and the belt tile a chest used to stand
  on, kept clear by `fit`, so the pole at `POLE_OFFSET` lights the arm exactly
  as it lit the chest inserter. `belted_link_steps` lays
  `connect_steps_reserving(source_chest, MACHINE, ..)` into each mouth with
  every other tile of the cell closed to the route (the ring round both
  machines, the beacon flank, the lane, the other mouths, and the tile in
  front of every run already laid), and `ensure_powered` wires each arm.
  `assign_sources` spends a source's rate as cells are assigned and refuses
  `AssemblyNoStandingSource` by name. The structural claims that rode on the
  chest charges now ride on each electric part's own `Place`
  (`Condition::Powered`, which `powering_entities` orders after the pole) and
  on the product machine's `SetRecipe` (`EntityAt` and `Feeds` for the whole
  chain, the belted arms included). The cell is sited from the **centroid** of
  its sources, and where the power anchor is further from a source than one
  `connect` window allows, a wire of poles is run to the centroid first and
  the cell is sited off its last pole; siting prefers a candidate within
  `SOURCE_REACH` of every source and falls back to the plain search.
- **`cellstock.rs`.** The `BufferGain` count is the `SupplyHorizon`, not
  `charge_products`; the loop seed names what the cell eats whether belted or
  charged. The lag edge is unchanged and now stated as the lower bound it is
  (a source slower than the cell's tempo makes the wait longer).
- **`have.rs`.** `machine_made_packs` asks for a cell only where
  `assemble::sources_stand_for` says its sources stand; otherwise the packs
  are hand-crafted as they were before cells existed. This is why
  `researched:automation` is byte-identical and why `sustain:iron-plate:30`
  and `gathered:crude-oil` got shorter (each used to build a chest-fed science
  cell for a research inside the plan).
- **`sustain.rs`, two small things outside the four modules, because the
  composed bundle refused inside `sustain` on master.** (1) Every furnace's
  plate chest, whatever it smelts, is excluded from the coal-buffer search --
  a second `Sustain` in one plan ranked the first's plate chest as its nearest
  coal buffer. (2) `nearest_belt_of` also taps the standing belt under an
  unload arm the expansion placed: the second sustain's haul met the first's
  coal belts already running past its chest's door, `connect::standing_run`
  finished it with two arms and no belt, and the offtake arm's fuel branch
  found nothing to tap. Both were measured in both orders before the fix
  (`laid no belt to branch the offtake arm's own fuel off`, at copper's arm
  `[28.5,-46.5]` and iron's `[-5.5,-28.5]`). Neither touches sustain's chests.
- **`scripts/continuous_supply.lua`** composes both sustains ahead of the
  cell, at the cell's own demand (12 iron and 6 copper a minute for six packs).
- **`error.rs`**: `AssemblyNoStandingSource`, a verdict about the world.
- **`crates/planner/tests/common`**: `with_sources`, the standing-source
  fixture the integration tests re-key on.

## What replaces `CELL_CHARGE_TICKS`

`SupplyHorizon`: the fewest products any belted input's source will deliver
before its ore is dug out (`SupplySource::yield_left / per_product`, off
`produce::cell_yield`), and that many of the cell's own cycles. It sizes the
ledger `cellstock` draws against, the boiler top-up and a burner product
machine's coal -- the last two capped at one fuel slot, which is all a hand
fills. Where a chest remains (green's gears and inserters) the hand charge
still bounds the cell and the horizon is the smaller of the two. The rate a
source must cover is the **goal's rate**, not the cell's tempo: sized at the
tempo, `producing:transport-belt:6` asked one furnace for 120 iron a minute.

## The side-load risk, checked in code and pinned by a test

`route_belt` gives a run's last tile the direction it arrived with, and
nothing in `route.rs` or `connect.rs` models that against a foreign belt.
`run_end_is_isolated` checks both directions after every run: the tile the
last belt pours into holds no belt (and is then closed to every later run of
the cell), and no standing belt outside the run faces into any of its tiles.
Either refuses as `AssemblyNoRouteForSupply` with the tiles named.
`tests::two_runs_into_one_cell_do_not_pour_into_each_other` lays iron and
copper into one red cell on the fixture and reads every belt off the built
world: no belt is fed from more than one side, each unload arm's belt pours
into nothing, and each arm is lit. Neither `route.rs` nor the splitter tap was
touched.

## Baselines, one binary, this worktree at `8d3cbe27` plus this change

| goal | before | after |
|---|---:|---:|
| `researched:automation` | 176 / 21,784 | 176 / 21,784 |
| `producing:automation-science-pack:6` | 316 / 22,457 | **refuses**: `eats 12 iron-plate/min by belt and no standing cell delivers it` |
| `all{sustain:iron-plate:12, sustain:copper-plate:6, producing:red:6}` | refuses in `sustain` | **1,559 / 96,221** |
| `all{sustain:iron-plate:30, sustain:copper-plate:15, producing:red:6}` | refuses in `sustain` | 1,997 / 95,899 |
| `all{sustain:copper-plate:15, producing:red:6}` (the old script) | 923 / 57,797 | refuses (no iron source) |
| `producing:logistic-science-pack:6` | 559 / 52,298 | refuses (its iron is belted) |
| `producing:iron-plate:261` | 194 / 33,645 | 194 / 33,645 |
| `producing:transport-belt:6` | 319 / 119,396 | refuses; `all{sustain:iron-plate:30, ..}` 1,501 / 73,252 |
| `sustain:iron-plate:30:36000` | 1,272 / 63,905 | 1,016 / 46,456 |
| `sustain:copper-plate:15:36000` | 479 / 20,904 | 500 / 21,069 |
| `gathered:crude-oil` (explored dump) | 2,352 / 322,738 | 2,216 / 319,933 |

**The composed bundle is the number now**, and it is a much bigger plan than
the 923 it replaces: two sustains with their coal hauls, two belt runs with
wires to their load arms, and no hand charge -- 96,221 ticks against 57,797,
for a cell that is bounded by ore rather than by fifteen packs. Nothing here
says the live run will hold that; it says the plan exists and refuses
nothing.

**`planning_work_ceilings`** was re-keyed to the red bundle and the belt
bundle (green refuses the composition: no room in the siting ring, and it is
phase 2). The ceilings are twice the first measurement on this binary.

## Falsification

Mutations, each backed up by copy and restored by copy plus `touch`, each
red on exactly the test that guards it:

- `laid_forward` not reserved and `run_end_is_isolated` returning `Ok` --
  `two_runs_into_one_cell_do_not_pour_into_each_other` stays green on the
  fixture (the copper run arrives from the west and never crosses the iron
  run's front), which is honest: the guard is a refusal for a geometry the
  fixture does not produce, and the test pins the property, not the code
  path. The property is also checked in code on every plan.
- `belted_among` returning empty -- every chest test goes red the other way
  (`a_red_cell_has_no_chest_but_the_output_one`, `the_charge_is_integer...`).
- `assign_sources` never refusing -- `a_cell_with_no_standing_source_is_refused_by_name`
  and `a_source_with_no_rate_left_is_not_belted_twice` red.
- `horizon_for` returning `charge_products()` -- `the_ledger_is_the_sources_ore_and_not_a_constant`
  and `the_plants_boiler_is_topped_up_for_the_horizon` red.

## What is deliberately not done, and what is open

- **Green** keeps its gear and inserter chests and refuses the composition on
  `map.json` (no room). Phase 2 is a gear cell.
- **`sustain`'s own two chests and the output chest** stand, as the design
  said. `DrawFromCell` still reads the output chest.
- **Five replan tests are `#[ignore]`d with the reason in the attribute**
  (`replan_finishes_its_cell`, `replan_taps_the_run`, `replan_sealed_supply`):
  their checked-in worlds are half-built CHEST cells from live runs, a layout
  no plan produces any more. They want a run of the belted layout and
  `plan --standing-from-run <run> --save-standing`.
- **The plan has never run live.** The first run will say whether two
  `ensure_powered` wires to burner cells and two runs into one machine column
  build cleanly; the side-load check refuses rather than guesses, so a
  refusal there is the next thing to read.
