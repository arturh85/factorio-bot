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
| `researched:automation` | `map.json` | 176 / 21,784 | stable across the night |
| `producing:automation-science-pack:6` | `map.json` | 316 / 22,457 | stable across the night |
| `producing:logistic-science-pack:6` | `map.json` | 559 / 52,298 | `112e0fdf` |
| `producing:iron-plate:261` | `map.json` | 194 / 33,645 | `71f9227c` |
| `producing:transport-belt:6` | `map.json` | 319 / 119,396 | `71f9227c` |
| `sustain:iron-plate:30:36000` | `map.json` | 1,234 / 64,564 | `112e0fdf` |
| `sustain:copper-plate:15:36000` | `map.json` | 479 / 20,904 | `112e0fdf` |
| `gathered:crude-oil` | **`map-31337-explored.json`** | **2,330 / 354,699** | `c0e51463` |

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
