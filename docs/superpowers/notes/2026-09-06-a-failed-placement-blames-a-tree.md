# A failed placement makes the tile unbuildable, and blames a tree that is not there

2026-09-06. Reproducible with `scripts/tree_at_the_drill.lua`, seed 31337.

## The symptom

`OreToPlate` loses one placement on pass 1, and the replan then refuses:

```
pass 1: done=true failed=1 pending=0
replan REFUSES: cannot build burner-mining-drill at tile (-16, -14):
                occupied by a tree, cliff, rock or unit
```

**That defeats the property replanning exists for.** `goal.built` re-derives the
entities not yet standing, so a second pass should finish a partial build. Here
the second pass cannot, and the block can never be completed.

## There is no tree

The discriminator is `rcon.*` for the game against `world.*` for the model,
asked **before** the build as well as after — the reading the earlier probe of
this shape could not get:

```
before the build   GAME=13  MODEL=0
    game : iron-ore x13
    model: (nothing)

after pass 1       GAME=26  MODEL=6
    game : entity-ghost x7, iron-ore x13, transport-belt x6
    model: transport-belt x6
```

**No tree, cliff, rock or unit at any point**, in either. And the refusal
appears only *after* a placement fails at that tile.

## Why the message says "tree"

`PlanState::occupant_of` consults its sources in order, and
`blocking_boxes_within` comes **before** the refused-footprint check. So
anything in `blocked_tree` is reported as `Occupant::Terrain` — whose `Display`
is *"occupied by a tree, cliff, rock or unit"* — regardless of what actually put
it there. `Occupant::Refused` exists and reads *"a footprint the game already
refused a build at"*, which is the message this case wants, and it is
unreachable whenever a blocking box covers the same ground.

**The boxes are anonymous by construction**: `blocked_tree` keeps an
`is_minable` flag and no name, which the code documents as deliberate. That is
defensible for a real tree and is what makes this case unreadable — the verdict
cannot say what it is about.

## What is NOT established

What put a box there. Nothing in the Lua bindings enumerates `blocked_tree`, so
from a script it is invisible: `world.find_entities_in_radius` reads the
entity graph's *entities*, and blocked boxes are a separate structure.

Candidates, none tested: the failed placement being recorded as blocked ground;
a stamped ghost entering the blocked tree; or a box whose extent reaches further
than its source. **I am not guessing between them** — the earlier version of
this investigation asserted a mechanism and had to be retracted, and the useful
half was always the measurement.

## What is established

- No tree, cliff, rock or unit is at that tile, before or after.
- The refusal follows a failed placement at that tile and did not occur on the
  run of the same blueprint that built cleanly.
- The message names the wrong cause, because blocked boxes are checked before
  refusals and are anonymous.
- **A partial build cannot be completed by replanning**, which is the recovery
  `goal.built` is designed around.

## Why it matters beyond this block

Ore-sited blocks make it likely rather than rare. A block sited on ore has its
footprint chosen by `nearest_ore_seed` rather than by clear ground, so a
placement failure anywhere in it strands the block — and the same blueprint on
the same seed built cleanly once and lost a placement the next time, so **the
build is not deterministic**.
