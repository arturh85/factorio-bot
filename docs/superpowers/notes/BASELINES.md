# Baselines, and which world each one belongs to

**A baseline number is meaningless without its dump.** Three agents in one night
reported a baseline "moved" or "could not be reproduced" when they had measured a
different world. Neither number was wrong; the pairing was.

Re-derive rather than quoting this file — that is the standing rule for every
number in this repo, and it applies here too. What this file is *for* is the
**pairing**: which world a goal is measured on, and why the worlds differ.

## The worlds, and which fields crossed the bridge when

Measured 2026-09-09 by counting prototypes that carry each field:

| dump | size | `crafting_categories` | `supply_area_distance` |
|---|---:|---:|---:|
| `map.json` | 826M | 0 | 0 |
| `map-31337-t0.json` | 826M | 0 | 0 |
| `map-t0-baseline.json` | 824M | 0 | 0 |
| `map-31337-explored.json` | 1.4G | 0 | 0 |
| `map-31337-explored-with-categories.json` | 158M | 11 | 0 |
| `map-31337-water-and-oil.json` | 1.5G | 18 | 5 |

All live under `workspace/scripts/`.

**This is why the same goal plans differently on two dumps, and why a refusal on
one is not a defect.** A world declaring no `crafting_categories` has no machine
for `chemistry`, so `Categories::planner_runs` is `{crafting, smelting}` and every
chemistry rung refuses — correctly, and with a refusal that says so in as many
words: *"no prototype in this world declares any crafting category at all, so the
model predates `crafting_categories` — it did not say, which is not the same as
saying nothing crafts chemistry."*

**Absent is not a value**, at the scale of a whole capture.

## The four standing baselines

**A baseline needs a COMMIT as well as a world**, and the omission of one bit me
within an hour of writing this file. Take before and after on the same binary,
and record what that binary was built from.

| goal | world | actions / makespan | measured at |
|---|---|---|---|
| `researched:automation` | `map.json` | 176 / 21,784 | stable across the night, unchanged by no-chests |
| `producing:automation-science-pack:6` | `map.json` | **REFUSES** (`assembly_no_standing_source`) | no-chests, 2026-09-09; was 316 / 22,457 |
| `all{sustain:iron-plate:12, sustain:copper-plate:6, producing:automation-science-pack:6}` | `map.json` | **1,559 / 96,221** | no-chests, 2026-09-09 -- the number that replaces the row above |
| `all{sustain:iron-plate:30, sustain:copper-plate:15, producing:automation-science-pack:6}` | `map.json` | 1,997 / 95,899 | no-chests, 2026-09-09; **refused in `sustain` before it** |
| `producing:logistic-science-pack:6` | `map.json` | **REFUSES** (`assembly_no_standing_source`, its iron is belted) | no-chests, 2026-09-09; was 559 / 52,298 |
| `all{sustain:iron-plate:30, sustain:copper-plate:15, producing:logistic-science-pack:6}` | `map.json` | REFUSES (`no room ... within 12 tiles`) | no-chests, 2026-09-09 -- green is phase 2, see below |
| `producing:iron-plate:261` | `map.json` | 194 / 33,645 | `71f9227c`, unchanged by no-chests |
| `producing:transport-belt:6` | `map.json` | **REFUSES** (`assembly_no_standing_source`) | no-chests, 2026-09-09; was 319 / 119,396 |
| `all{sustain:iron-plate:30, producing:transport-belt:6}` | `map.json` | **1,501 / 73,252** | no-chests, 2026-09-09 |
| `sustain:iron-plate:30:36000` | `map.json` | **1,016 / 46,456** | no-chests, 2026-09-09 (was 1,272 / 63,905 at `6eb0fa7b`) -- see below |
| `sustain:copper-plate:15:36000` | `map.json` | **500 / 21,069** | no-chests, 2026-09-09 (was 479 / 20,904) -- see below |
| `gathered:crude-oil` | **`map-31337-explored.json`** | **2,216 / 319,933** | no-chests, 2026-09-09 (was 2,352 / 322,738 at `c385c409`) -- see below |

### No chests in the line (2026-09-09): four rows refuse, and that is the point

A science cell has no chest but the output one any more: every smelted
ingredient (`produce::cell_spec` can make it from ore -- iron and copper
plate) is belted straight into the machine that eats it from a standing
stage-1 cell's plate chest, and a `producing` goal with no such source is
refused by name (`PlannerError::AssemblyNoStandingSource`) rather than planned
as a cell that stops when a hand charge runs out. So `producing:automation-
science-pack:6`, `producing:logistic-science-pack:6` and
`producing:transport-belt:6` refuse on a t=0 dump, and the number to report
is the composed bundle -- `continuous_supply.lua`'s goal, with both sustains
at the cell's own demand. Design: `2026-09-09-no-chests-in-the-line.md`;
result: `2026-09-09-no-chests-landed.md`.

Three rows moved for reasons that are not the cell:

- **`sustain:iron-plate:30` 1,272 -> 1,016 and `gathered:crude-oil` 2,352 ->
  2,216**: `Researched` no longer asks for a science cell whose sources do not
  stand (`have::machine_made_packs` is gated on
  `assemble::sources_stand_for`), so the red packs a research inside those
  plans needs are hand-crafted as they were before cells existed, instead of a
  whole chest-fed cell being built for them. `researched:automation` never
  built one (it is the unlocker) and is byte-identical.
- **`sustain:copper-plate:15` 479 -> 500, and part of the iron move**: the
  offtake arm's fuel branch now taps the standing belt under an unload arm
  the expansion placed as well as the belts it laid (`sustain::nearest_belt_of`).
  That is what lets a SECOND sustain in one plan branch off the first's coal
  haul -- every two-sustain bundle refused with `laid no belt to branch the
  offtake arm's own fuel off` before, in both orders -- and it also moves the
  tap point of a lone sustain.

**Green is not a phase-1 result.** Its gears and inserters still arrive in
hand-filled chests (nothing here makes them), its iron is belted, and on
`map.json` the composed bundle finds no room in the siting ring. That is the
gear cell the design note names as phase 2, not a regression to chase here.

### `sustain:iron-plate` moved at `6eb0fa7b`, and it is a fix, not drift

`produce::cell_ledger` stopped offering a furnace that an offtake arm empties
to a hand (`docs/superpowers/notes/2026-09-09-the-take-races-the-arm.md`). On
`c385c409` that plan had eleven `take .. iron-plate from the cell`, eight of
them from the two furnaces the plan itself puts arms on -- takes that would
have come back `removed 0` live, exactly as the copper ones did in
`run-1788949638-11792`. With the fix those eight are hand smelts: +38 actions,
-659 ticks. Seven other baselines unchanged on the same binary pair.

`gathered:crude-oil` was re-measured at the same time and had **already
moved** on master between `c0e51463` and `c385c409` -- identical on both
binaries of that pair, so not this change's. Whoever moved it did not record
it here; the row now carries the commit it was last seen at.

### The sustain baselines drifted for hours because they lived only in prose

**Never in this table until `112e0fdf`.** They were quoted from session to
session in `/loop` prompts across an entire night — `1,111 / 62,064` and
`470 / 28,126` — and nobody re-derived them from the binary in between. By the
time an agent building an unrelated fix (`sealed-supply-chest`) checked, they
read **1,234 / 64,564** and **479 / 20,904**: moved by commits between
`a69ae64c` and `c713da57`, none of which touched sustain's own logic — the
cell-siting and belt-banding work upstream of it moved what a sustain plan
costs. The agent's own change did not move them further; it only *found* that
they had already moved.

This is "a number in prose is a cache" at the scale of a whole night: every
`/loop` prompt copied the figure forward as fact, and the figure was stale for
an unknown number of iterations before anyone checked. **The rule the rest of
this file states now has a concrete cost attached**: re-derive from the binary
at the point of use, and put a re-measured number in this table — not just in
a prompt — the moment it is checked, so the next reader has somewhere to find
it besides another prompt.

Also legitimately moved and disclosed as a finding, not silently re-pinned:
`producing:logistic-science-pack:6` **571 → 559** at the same commit. A sixth
science cell used to be sited where its supply chest could never be belted at
all; `supply_chest_is_reachable` now refuses that site, so the plan sites five
cells honestly rather than a sixth it could never have delivered from. Shorter
by correctness, not by chance.

### `gathered:crude-oil` is the one that keeps going wrong, twice over

**By world.** It **refuses** on `map.json` (no crude oil charted there) and reads
**2,449 / 357,372** on `map-31337-explored-with-categories.json`. Both correct for
their world.

**And by commit.** It read **2,117 / 309,574** earlier on 2026-09-09 — measured on
a `target/release/factorio-bot` built at 22:04 the previous evening, while master
had moved through a dozen merges since. An agent building fresh at `c0e51463`
measured **2,330 / 354,699** and correctly reported that the older figure "does
not reproduce at HEAD… stale, not something I moved."

So the older number was not wrong when taken; it was quoted past its build. **A
stale binary is as much a source of a phantom regression as a stale dump**, and it
is harder to notice because `git log` looks right while `target/` does not.

Check the binary's mtime against `git log -1` before quoting anything from it.

## Every baseline here is an OFFLINE plan from t=0, and that is a blind spot

**A baseline cannot see the replan path.** Every number in this file is
`factorio-bot plan --world <dump>`, and every dump is t=0-shaped, so the planner
always meets a clean world. A live run does not: the supervisor plans, executes,
and *replans* whenever a batch truncates — and the second expansion meets a world
where the first plan's work already stands as map facts.

**This is not hypothetical. It swallowed a reviewed, merged fix on 2026-09-09.**
`9f549b1c` was written to remove a specific refusal, moved all eight baselines
correctly, and was measured at **885 / 52,891** planning cleanly through to both
assembling machines. The live run on that exact binary refused with the
**byte-identical** blocker:

```
a cell already makes copper-plate at [29.5,-46.5] and nothing can carry it to the
supply chest at [30.5,-24.5]
```

**"a cell ALREADY MAKES copper-plate" is a sentence only a replan can say.**
Offline there is no cell, so the case never arises and the fix looks perfect.

So when a change touches anything reachable on a replan — siting, routing,
reservations, recovery, anything that reads standing entities — **the numbers in
this file are necessary and not sufficient, and saying so is part of reporting
them.** The failure mode is not a wrong number; it is eight right numbers and a
broken run.

**Two things now do see it, offline, in seconds** (2026-09-09, see
`2026-09-09-a-replan-you-can-run-offline.md`):

- `just replan-check` / `plan --replan 1 [--fail <label> | --done-by <tick>]`
  plans, applies what was built to the world as map facts, and plans again
  from a fresh state over it. `--fail "copper-plate from the cell"` is the
  live shape (a failed take abandons its dependency cone); reverting
  `a69ae64c` under it reproduces `run-1788923927-04849`'s four tiles byte
  for byte.
- `plan --standing-from-run <run> --at-tick <T>` plans against the world a
  finished run's own `map.jsonl` keyframe and `samples.jsonl` say it had;
  `--save-standing` writes the snapshot a test can check in.

Both are pinned by `crates/planner/tests/replan_on_standing_world.rs` and
`replan_haul.rs`. **Run the first beside the baselines for any change that
reads standing entities.** `--resume-from <run>[:<milestone>]` remains the
check of record for what no offline plan carries -- chest contents, the
executor's own behaviour, the mod's refusals.

### Re-measured 2026-09-09 on `7665ffde` plus the belt tap, unchanged to the tick

The seven `map.json` rows above were taken before and after
`connect::tap_standing_run` landed (a boxed-in source is tapped with a
splitter spliced into the run it already has -- see
`2026-09-09-a-boxed-in-source-is-tapped.md`), same binary, and **none moved**:
the tap is a fallback that only runs where the plain search refused. The
number that did move is replan-shaped and belongs beside them: the replan of
`run-1788946451-86723` at tick 64,053 went from **refuses** to **341 /
37,185**, pinned by `crates/planner/tests/replan_taps_the_run.rs` on the
checked-in fixture.

## Goals that only exist on a categories world

`have:sulfur:10`, `have:battery:1`, `have:plastic-bar:10`,
`produced:petroleum-gas:45`, and the four rocket-silo rungs all need a world that
declares `crafting_categories`. Measuring them on `map.json` proves nothing about
the planner.

## When a baseline legitimately moves

It is a **finding**, not a failure — report which goal, by how much, and whether
the new behaviour is correct. Never re-baseline quietly. Two cases seen so far:

- a change that makes a plan genuinely cheaper or dearer (a cell costs its build
  up front and pays back over a campaign, which one plan cannot show);
- a change that alters bot ranking, so expansions differ. `note_planned_ticks`
  reads `duration`, so anything touching a duration can reshuffle bots — that is
  a *different plan*, not the same plan scheduled better, and should be described
  as such.

## The rule that prevents all of this

**Take before and after on the SAME binary, and state the commit and the dump
beside them.** A baseline compared across two builds measures the builds. A
baseline compared across two dumps measures the dumps.
