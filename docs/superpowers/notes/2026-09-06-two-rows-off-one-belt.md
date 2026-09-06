# Two rows off one belt: it works, and it does not share

2026-09-06. `TwoRowSmelter`, 27 entities, headless 2 bots at 5x, seed 31337,
second instance.

`TJunctionSmelter` fed two furnaces on one side of a mixed belt. This is the
shape the owner described and `FurnaceLine` uses: furnaces on **both** sides of
a central belt, three per side.

**Deliberately input-starved.** One burner loader arm supplies far less than six
stone furnaces consume (six need 1.875 ore/s). That is the design, not a
shortfall: a belt with more ore than anyone needs distributes perfectly however
badly it is built. Scarcity is what exposes unfairness. **Nothing here is a
throughput or bandwidth number** — 48 stone furnaces saturate a yellow belt, so
six are nowhere near any belt limit.

## The result

```
NORTH row (arms see the ORE lane first):  59 + 3 + 1 = 63
SOUTH row (arms see the COAL lane first): 58 + 27 + 2 = 87
west -> east, both rows: N 59/3/1   S 58/27/2
ore consumed 150 of 150, coal consumed 60 of 60
```

**It works.** Both rows produced, so the starting lane asymmetry resolves. The
two rows draw opposite lanes first — an inserter prefers the far lane, and the
rows sit on opposite sides — so southern arms meet coal first and northern arms
meet ore. That is a preference, not a restriction: an arm takes from **both**
lanes, so a furnace whose ore slot is full forces its arm onto the other one.
(Confirmed by the owner, who plays the game, and matched by the run.)

**It does not share.** The two westmost furnaces took **117 of 150 plates —
78%** — and the two eastmost took 3 between them.

## Why, and this is the useful part

The final reading names the mechanism:

```
tick 13833   ore=0  coal=0  N=59/3/1  S=58/27/2  fuelN=2/5/5  fuelS=2/4/5
```

The starved furnaces are **not** short of fuel. They end with full fuel slots
and no ore.

**Coal self-balances because its buffer is small; ore does not because its
buffer is big.** A furnace's fuel slot fills at 5 and then refuses more coal, so
the near arm cannot take any and the coal rides on to the next takeoff. A
furnace's ore input has no such small ceiling, so the near arm keeps accepting
ore and none of it ever reaches the far end.

The consequence for scaling is sharp, and it is not "add more furnaces
carefully":

> **A long smelter line fed from one end must have a SATURATED ore belt.** Any
> shortfall is absorbed entirely by the near end, and the far end dies with a
> full fuel slot. This is why real designs run a full belt into 48 furnaces
> rather than a partial belt into a few.

## Two defects this run found in my own work

### Search-based siting can wall the roster in

The first attempt used `goal.built(bp)` with no site, which searches outward
from the **roster's centroid**. Siting deliberately treats a character as
non-blocking — a bot can walk away, so it should not veto a site — and for the
small blocks so far that held. This block is 27 entities spanning roughly 16x8
with furnace rows above and below a central corridor, and it was sited around
the bots:

```
pre-place check: the character is already walled in here; this placement does
not change that and is allowed  player=1 from=[0.5, 0.5] pocket_tiles=1.0
walk failure ... would end at [-0.5, -1.5], inside a collision box spanning
[-0.7, -2.7] to [0.7, -1.3]
build: done=true failed=0 lost=0 pending=13
```

One reachable tile. 13 of 29 steps never ran; 2 of 6 furnaces stood.

Only the explicit-anchor form of `goal.built` refuses for a bot on the
footprint. **Both searching forms ignore characters**, which is right for a
block a bot can step out of and wrong for one that encloses them. Worked around
here with a `near` hint 40 tiles east; that is a workaround, not a fix, and the
gap is open.

### I dropped a guard I had already written

`build: done=true failed=0 lost=0 pending=13`. `done` beside a non-zero
`pending` cannot both be true, and nothing said so — because the
`BUILD INCOMPLETE` check present in `smelting_live.lua` and
`belt_smelter_live.lua` was simply not carried into this script. The run was
read as "no furnaces found" when the real answer was "two thirds of the block
was never built".

Restored, and widened to `lost` as well as `failed` and `pending`. The lesson is
not "add the check" — it is that **a check written once for a class of failure
has to travel with the class, not with the file it was born in.**

## Open

- The siting enclosure gap above.
- Whether the far-lane-then-near-lane correction costs a swing cycle. Not
  visible under starvation; would need a saturated belt to see.
- Nothing here has been run with a saturated ore belt, which is precisely the
  regime the scaling conclusion says matters.
