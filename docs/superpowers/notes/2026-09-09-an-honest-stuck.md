# An honest `stuck`, and the one blocker left

`run-1788920460-08860`, seed 31337, four headless character bots at 10x,
`--new`, release binary built 04:20 from master `f3785215`. **Nothing cheated.**
Same goal as its predecessor:

```lua
goal.all { goal.sustain("copper-plate", 15, 36000),
           goal.producing("automation-science-pack", 6) }
```

This run produced **no more science than the last one — and it is the more
useful of the two**, because both things that were invisible are now visible.

## The two fixes did exactly what they were for

**The milestone reports `stuck`, not `satisfied`.** Verbatim:

```
milestone 1: stuck after 1 iteration(s), best 1063 steps, refused:
a cell already makes copper-plate at [29.5, -46.5] and nothing can carry it
to the supply chest at [30.5, -24.5]
```

Its predecessor reported `satisfied` in the same situation, having made zero
packs and placed no assembling machine. That was the false green
(`a27811f1`): `expand_goal_body`'s `All` loop carried
`SustainSupplyNotStanding` out with `?` before the science conjunct was ever
expanded, and `supervisor.lua` mapped that code to `_close("satisfied")`.

**The truncation is counted.** `action verdicts: {success: 831, abandoned: 48,
failed: 2}`. Those 48 are the plan's tail, both `assembling-machine-1`
placements among them. In the predecessor **the identical 48 steps wrote
nothing at all** — `run_action` on `PredOutcome::Abandoned` logged nothing, on
the reasoning that "nothing was dispatched, so nothing is written" — so the plan
simply appeared to stop, indistinguishable from a killed run. `Status::Abandoned`
(`2e4f428e`) is what makes them countable.

## The blocker, and it is a perimeter

```
from the iron-chest at [29.5, -46.5]: no belt route, blocked by 4 tile(s):
    [28.5, -46.5] [29.5, -48.5] [29.5, -45.5] [30.5, -46.5]
```

**Those four tiles are exactly the four orthogonal neighbours of the chest.**
The chest's entire perimeter is consumed, so no belt can ever leave it — this is
a *perimeter budget*, refused by `first_free_perimeter` before `route_belt` ever
runs, not an obstacle in the way. The repo has met this shape before and wrote
the rule down: **read the tiles a refusal names before naming the mechanism;
adjacent-to-the-source is a perimeter, not an obstacle.**

The consumer is `method::sustain`'s own coal run, which takes three of that
chest's four sides. `connect`'s hug rule closes a route out of the sides of the
chest *it serves*, but this chest is a **bystander** to the run that boxes it in.

**The obvious fix has been tried twice and reverted twice**: reserving a side for
bystander containers boxes in a *later* run of the same expansion, five sustain
tests red both times. The standing conclusion is that the bystander question has
to be asked with knowledge of what runs come **next** — an ordering change, not a
grid change.

## What is unchanged from the predecessor, and worth stating

`copper-plate` again holds **15/min with the roster at 0% busy** from minute 20,
and coal likewise; the sustain half works. Still **entirely burner-driven** —
`work e/b` reads `0/161`, no generator ran. Still zero `automation-science-pack`,
zero `iron-gear-wheel`, zero `electronic-circuit`: the science chain never
started, because the run never got past the belt route.

`steps/bot {1: 520, 2: 136, 3: 119, 4: 106}`, fleet utilisation **25.1%**. The
`yields_at` fix moved `producing:iron-plate:261` from 27.0% to 44.2%, and moved
this goal not at all — so **there is a second concentrator and it is not
`Chop`/`Stockpile`.** Unowned.

## One instrument bug this run exposed

`tools/run_analysis.py` reports `48 settle(s) with no dispatch` under `join:` as
an anomaly. That is now *correct behaviour* — an abandoned step settles without
ever having been dispatched — and the analyser should name it `abandoned` rather
than flag a broken join.
