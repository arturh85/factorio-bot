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

Verified 2026-09-09 on `target/release/factorio-bot`, `--bots 1,2,3,4`.

| goal | world | actions / makespan |
|---|---|---|
| `researched:automation` | `map.json` | 176 / 21,784 |
| `producing:automation-science-pack:6` | `map.json` | 316 / 22,457 |
| `producing:logistic-science-pack:6` | `map.json` | 571 / 54,371 |
| `gathered:crude-oil` | **`map-31337-explored.json`** | 2,117 / 309,574 |

`gathered:crude-oil` is the one that keeps going wrong. On `map.json` it
**refuses** — no crude oil is charted there — and on
`map-31337-explored-with-categories.json` it is **2,449 / 357,372**. Both correct
for their world. Name the dump beside the number, always.

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
