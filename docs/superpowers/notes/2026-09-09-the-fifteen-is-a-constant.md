# The 15-pack plateau is `CELL_CHARGE_TICKS / ticks_per_item`, by construction

Answered entirely from the record — `run-1788926478-07032` and
`run-1788931904-77495` — with no code change and no run.

## The number

`crates/planner/src/method/assemble.rs`: `CELL_CHARGE_TICKS = 9_000`;
`charge_products()` is `9000 / ticks_per_item`. An `assembling-machine-1`
makes a red pack every 600 ticks: **9000/600 = 15.** The gear feed charge is
`30 iron-plate` — also exactly 15 gears. Both runs' plateau is the constant
agreeing with itself, not a property of the map, the ring, or the belts.

## The ring hypothesis is killed as the cause of this plateau

The copper belt from the sustain cell **works the whole run**: the supply
chest climbs 7 → 53 and keeps climbing past the plateau tick. Copper
accumulates **unused** in the science cell while it starves on the *other*
input.

## The actual limiter is iron gears, and the feed chest has no belt at all

Machine status samples, identical in both runs: the gear feed chest
(`[35.5,-30.5]`) empties at tick 51,000 and is never refilled;
the gear assembler stops with `item_ingredient_shortage`, empty input;
the pack assembler reaches 15 at tick 54,600 and stops the same way,
holding unused copper-plate and no gears.

**Why nothing refills it**: `assemble.rs` says so in its own comment — *"the
feed chests are filled by hand … nothing refills them"*. The supply chest
(copper) is the one exception, belted since `8f43e633`; the gear feed chest
never got the same treatment, and iron-plate has no `sustain` in this goal at
all.

## What was ruled out along the way

- **"The driver stopped dispatching"** — narrowly false. Production stops at
  54,600; the last dispatch is 56,008 / 64,084. But one level up it is *nearly*
  right: plans 2 and 3 contain no second `charge the feed chest`, because the
  `producing` conjunct reads a standing cell as satisfied and nothing asks for
  more.
- **The take-in-pieces bound (`473c7294`)** — unrelated to the machine
  plateau. Its two failures target the *sustain* furnace, whose plates are
  taken off by a belt arm before a bot's hand-take can find them in the
  result slot — same shape as the original diagnosis, a different container.
  Feeds only the lab thread; the abandoned counts are mostly an unrelated
  walk failure on bot 3.

## Classification

**A design consequence, not a bug and not a capacity limit.** The cell is
charge-fed by construction; an honest run makes one charge and stops. Two
runs agreeing is the constant agreeing with itself — this was never evidence
of a shared map feature, and going looking for one (the ring, coal, a belt
run) would have been chasing a coincidence.

The paths forward are owner decisions: **more cells**, **a charge sized from
demand rather than a fixed tick budget**, or **extending the `8f43e633` belt
pattern to the feed chests** (which needs an iron `sustain` in the composed
goal). None chosen here.

**One cheap secondary item**: for a belted produce cell, the "source is not
producing" message on a failed hand-take is misleading — the source *is*
producing, into a chest the take never looks at.

## Why the coal-ring hypothesis was still worth having

It was wrong for this plateau, and cheap to kill: the agent read the copper
supply chest's own curve before touching any code, which is exactly what
prevented an hour spent "fixing" a ring that was never the problem. Read the
artifact before naming the mechanism — again.
