# The block earns its own upgrade

2026-09-06. `block_earns_electronics.lua`, headless 2 bots at 5x, seed 31337,
second instance (`scratch/blocks.toml`, rcon 4360, `workspace/blocks`).

`TJunctionSmelter` runs on burner inserters, belts and stone furnaces and
**cannot grow an output side**: an arm carrying iron plates has no fuel source,
so it stops when its hand charge burns out. The missing part is the electric
`inserter`, gated behind `electronics`.

`electronics` is a **trigger technology fired by 10 copper plates** — no lab, no
science packs, `research_unit_count: 1` with empty ingredients. So the question
is whether the block can pay for its own upgrade by running on copper.

## It can

```
CONTROL (before):  failed: recipe copper-cable is not enabled for this force
  >> the block has smelted 10 copper plates at tick 2204
trigger technology electronics earned at tick 2220 (craft-item 10/10)
RETRY   (after):   SUCCEEDED
took the block's own copper: furnaces held 20, now hold 4 (moved 16)
  craft copper-cable         x2  -> ok
  craft iron-gear-wheel      x1  -> ok
  craft electronic-circuit   x1  -> ok
  craft inserter             x1  -> ok
```

**2,220 ticks — about 37 seconds of game time — from a bare force to an electric
inserter**, built out of copper the block smelted. No lab was placed, no science
pack was made, and no research action appeared in the plan.

## The control is the result

A "then it worked" with no control proves nothing here, because the craft could
fail for two different reasons and only one of them is interesting.

So five copper plates are cheated to the bot **before** anything smelts, and the
craft is attempted with materials already in hand. The failure that came back
names the reason:

```
Error: recipe copper-cable is not enabled for this force
```

Not "insufficient materials". The same craft, on the same materials, succeeded
after the block ran. **Five is deliberately below the trigger's threshold of
ten**, so the cheat cannot fire the technology the block is supposed to earn —
a control that hands over the answer is not a control.

The script refuses to continue if that first craft succeeds, and says the run is
void rather than reporting a result it cannot support.

## The metal is the block's own

The chain could have run on the five cheated plates and looked identical. So the
plates are pulled out of the furnaces first (`remove_from_inventory` with
`inventory_type` 3, a furnace's result slot) and the furnace output is **re-read
afterwards**: 20 → 4, so 16 moved. Had the removal silently done nothing, the
cheated plates would have covered the craft and the claim would have been wrong
in a way nothing in the output would have shown.

## What this makes possible

The two prerequisites of `automation-science-pack` are both trigger
technologies, and a burner block earns both from ordinary smelting:

| trigger | fired by | unlocks |
|---|---|---|
| `steam-power` | 50 iron plates | `boiler`, `steam-engine`, `offshore-pump` |
| `electronics` | 10 copper plates | `inserter`, `small-electric-pole`, `electronic-circuit`, `lab`, `copper-cable` |

The iron run of `TJunctionSmelter` already fired `steam-power` unprompted
(`craft-item 50/50` at tick 5,880). A copper run fires `electronics`. Between
them the block unlocks **the electric inserter, the pole, and the entire steam
power chain** — which is exactly the bill for an output side that does not
starve.

## Caveat, stated rather than buried

The `trigger technology ... earned` line is the **mod's emulation sweep**, not
the game's own check. That is not a grant: `craft-item` does fire for machine
output, and the record already measures the sweep as running at most ~400 ticks
ahead of the game (`docs/superpowers/notes/2026-09-05-research-triggers.md`).
So the unlock is genuine and the *tick* is up to ~400 early. Nothing here would
change if the sweep were off; the technology would land slightly later.

## Two smaller facts

- **`remove_from_inventory` had no call site anywhere** — not in a script, not
  in the planner. `inventory_type` 3 works for a furnace's result slot; the mod
  passes the number straight to `entity.get_inventory`.
- The executor's walk recovery earned its keep. A bot was routed to
  `[10.5, 0.5]`, which resolved to a tile inside a collision box; it re-asked at
  radius 0.5 instead of 10 and the build completed with 0 failed and 0 lost.

## Next

The output side is now buildable: electric inserters plus a `small-electric-pole`
and a boiler/engine/pump for power — every one of them unlocked by the block
itself. That is the first block in this project that could run indefinitely
rather than until its hand charge burns out.
