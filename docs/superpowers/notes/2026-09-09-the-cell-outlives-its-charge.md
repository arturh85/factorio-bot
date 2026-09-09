# The cell outlives its charge

`run-1788914717-24351`, seed 31337, four headless character bots at 10x,
`--new`, binary built 02:44 from `3dadc04e`. Goal:

```lua
goal.all { goal.sustain("copper-plate", 15, 36000),
           goal.producing("automation-science-pack", 6) }
```

**This run answers the question the whole cell line of work existed to ask, and
the answer is yes.** It also produces a false green that is worse than the
result is good, so both halves are recorded here.

## The result: a factory ran for eight minutes with nobody feeding it

`copper-plate` is attributed **`factory`** at the 20:00, 25:00 and 28:18 marks,
with **zero feeding dispatches in each of those intervals** and the roster at
**0% busy**. `coal` is `factory` across the same three. The rate is **15/min,
held flat** from 15:00 to 25:00 — which is exactly the `sustain` target the goal
asked for, met by machines rather than by hands.

| mark | copper-plate cum. | /min | feed acts | busy% | verdict |
|---|---:|---:|---:|---:|---|
| 5:00 | 32 | 6 | 362 | 87 | roster-fed |
| 10:00 | 84 | 10 | 156 | 34 | roster-fed |
| 15:00 | 113 | 6 | 25 | 15 | roster-fed |
| 20:00 | 188 | **15** | **0** | **2** | **factory** |
| 25:00 | 263 | **15** | **0** | **0** | **factory** |
| 28:18 | 269 | 2 | **0** | **0** | **factory** |

**The comparison that matters**: the charge-fed cell of 2026-09-08 earned this
project's first `factory` verdict and then **stopped dead at 9:48**, because it
was draining a chest a bot had filled by hand and `CELL_CHARGE_TICKS` is 9,000.
This cell was still producing at **28:18** — eighteen and a half minutes past
where its predecessor died — with no bot within reach of it. `3dadc04e` belts a
standing stage-1 cell into the assembly cell's supply chest, and that is the
whole difference.

**It is entirely burner-driven.** The `work e/b` column reads `0/165`: not one
electric producer, no generator all run. Burner mining drills feeding stone
furnaces, self-fuelling off the coal that flows through them — the shape
described under *"a burner block works exactly where coal flows THROUGH it"*.
The boiler and generator that stand from 15:00 never worked.

## The false green, which is the more urgent finding

The supervisor reported, verbatim:

```
milestone 1: satisfied after 1 iteration(s), best 1056 steps,
last error: game rejected the command: player blocks placement in all directions
```

**It was not satisfied.** `automation-science-pack` production is **0** at every
mark. **No assembling machine was ever placed** — the census has no
`assembling-machine` row at any mark, and the mod does sample that kind.

The plan contained the steps. `place assembling-machine-1 at [31.5, -27.5]`,
`place assembling-machine-1 at [35.5, -27.5]`, `set assembling-machine-1 to
automation-science-pack` and `set assembling-machine-1 to iron-gear-wheel` all
sit at `planned_start` 45,218–45,624 against a makespan of 50,709 — the last
tenth of the plan. **830 actions were dispatched of 1,056 steps: ~226 tail steps
never ran**, and the four that mattered were among them.

So a placement was rejected, the batch stopped, and the satisfaction check said
the goal was met anyway. Two independent defects, chased separately:

1. **The predicate answers "satisfied" with no machine standing.**
   `producing:X:N` means *standing capacity* (`cells_for` minus
   `cells_standing()`), by design — so the suspicion is that `cells_standing()`
   matched the copper chain's own 15 furnaces and 6 drills. If it does, **an
   `All` goal whose first conjunct builds machines can satisfy its second
   conjunct for free**, which would make every `producing:` green suspect.
2. **A truncated batch reports success.** `events.jsonl` for this run carries
   **no refusal event of any kind** — eleven event kinds, none of them a
   refusal — despite a fatal placement rejection. The record cannot see the
   thing that ended the run.

## `player blocks placement in all directions`

The error that cost the run its objective. `pre_place` judges only the character
*doing* the placing, and `method::blueprint` and `method::assemble` both have
evacuation machinery (`enclosure::check` emitting `ActionKind::Evacuate`). So
this is very likely the thirteen-times-repeated shape: **the capability exists
and nothing on this path emits it.**

## A second, cheaper cost: walks routed into our own belts

Dozens of these, all bot 1:

```
#1 was routed to a spot nobody can stand on (the walk to [36.5, -51.5] would end
at [35.5, -42.5], inside transport-belt at [35.5, -42.5] — a character cannot
stand there, so the walk could only stall), asking again for 36.5/-51.5 at a
radius of 0.5 instead of 10
```

The run placed 156 transport belts and then kept resolving walk destinations
into them. **It is not a failure — zero walks failed, the radius-0.5 retry
works** — but bot 1 spent 18,121 ticks walking against 21,378 acting.

## And the serialisation is confirmed again

`steps/bot {1: 517, 2: 136, 3: 119, 4: 106}` — bot 1 took **59% of the plan**.
Fleet utilisation **24.7%**. Same shape as the plate-rate run (bot 1: 231 steps,
bots 2–4: 8–14, utilisation 27%), so it is not an artefact of one goal.

## What this run does NOT show

Nothing here is evidence about science packs, assembling machines, or electric
power: none of the three existed in it. The proven claim is narrow and worth
stating narrowly — **a belt-fed burner cell sustains a target rate unattended
for at least eighteen minutes past its charge**, on one item, on Nauvis, at 10x.
