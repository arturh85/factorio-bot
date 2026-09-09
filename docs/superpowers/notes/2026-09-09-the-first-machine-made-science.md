# The first machine-made science packs

`run-1788926478-07032`, seed 31337, four headless character bots at 10x, `--new`,
release binary built 06:00 from master `473c7294`. **Nothing cheated.**

## The result

```
automation-science-pack   0 → 0 → 14 → 15 → 15 → 15   at 5/10/15/20/25/end 27:58
why (15:00): machines made 14 of the 14 (100%) -- assembling-machine-1 x14
why (iron-gear-wheel at 15:00): machines made 15 of the 15 (100%) -- assembling-machine-1 x15
verdict: automation-science-pack  15:00 roster-fed, 20:00 factory
```

**Before tonight this project had never made a science pack with a machine.** The
owner's own statement of the bar was *"no science pack has ever been machine-made
without a bot feeding it, and no lab has ever been inserter-fed"*. Half of that
is now cleared: an `assembling-machine-1` made fourteen packs, and every one of
the fourteen came out of the machine rather than out of a bot's hands.

**And the first electric production, too.** The plant ran from ~10:33:

| mark | gen kW | cons kW | work e/b | busy% | feed acts |
|---|---:|---:|---|---:|---:|
| 5:00 | 0 | 0 | 0/171 | 85 | 383 |
| 10:00 | 0 | 0 | 0/378 | 41 | 133 |
| **15:00** | **900** | 162 | **32/345** | 33 | 107 |
| 20:00 | 900 | 22 | 0/219 | 2 | **0** |
| 25:00 | 900 | 22 | 0/180 | 0 | **0** |

`work e/b` at 15:00 is the first non-zero **electric** producer count this
project has recorded. Census: `assembling-machine 2 / 1 working`, `generator
1 / 1`, `boiler 1 / 1`.

## What is NOT claimed

**Not a sustained factory.** Fourteen packs arrive between 10:00 and 15:00 at
3/min, one more by 20:00, and then it stops: `0 working` electric at 20:00 and
25:00. The `factory` verdict at 20:00 covers **one pack** made with zero feeding
dispatches, which is a real but thin piece of evidence. The 15:00 interval is
honestly `roster-fed` — the machines made the packs, the bots carried the inputs.

**No lab ran.** A lab stands from 10:00 and reads `0 working` at every mark. The
second half of the owner's bar is untouched.

**The run still ended `stuck`, at 15:29**, and 80 steps were abandoned.

## What unblocked it, and it was not what I predicted

`473c7294`, a short take finished in pieces rather than treated as a verdict. The
note appears **four times** in this run's events. The diagnosis behind it was the
night's sharpest correction: the plan's timing had been right all along — the
furnace had made its ten plates on schedule, one every ~233 ticks — and the
plates were simply *in a different container*, because `sustain`'s offtake arm is
a **burner inserter touching only plates**, so it has no fuel and dies after
carrying seven. `produce`'s take reads the furnace's own result slot. CLAUDE.md
already stated the rule (*a burner block works exactly where coal flows THROUGH
it*); nobody had connected it to this.

Also visible in this run for the first time: `HALTED: stuck -- refused: ...` is
printed and recorded, from `2b649846`. Three earlier runs could not say why they
stopped.

## The next blocker, and it is one tile

```
nothing can carry coal from the buffer at [32.5,-41.5] to the iron-chest at
[27.5,-40.5]: no belt route, blocked by 1 tile(s): [30.5,-39.5]
```

**One tile.** The supervisor reached 2 iterations with a best plan of 231 steps —
most of the factory was already standing. Every previous run refused on four
tiles or eight; this is the smallest blocker this goal has ever produced, and
per the rule that keeps paying off here, **read what is actually on `[30.5,-39.5]`
and which run placed it before naming a mechanism.**
