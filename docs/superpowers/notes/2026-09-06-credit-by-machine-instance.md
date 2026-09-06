# Credit belongs to a machine, not to a prototype

2026-09-06, on `3d013ec9`. The hand-credit mass balance
(`tools/run_analysis.py`, note `2026-09-06-hand-credit-mass-balance.md`) prices
every hand delivery as a credit of output and refuses a window whose machine
production the roster's outstanding credit could explain. It works: it refused
`run-1788674059-90744`, which the older lead-in check passed off one charge of
coal.

**It pooled that credit by entity *prototype*.** All five stone furnaces in a
run were one bucket, so a plan that hand-smelts its own belts in four furnaces
charged all of their coal against a fifth, belted one.

## The wall this built

Measured on `run-1788679826-02267`, the first cell that ran with no bot in the
loop:

```
credit 333 from 90 delivery(ies)
  (burner-mining-drill/coal 47, stone-furnace/coal 333,
   stone-furnace/iron-ore 128, wooden-chest/iron-ore 79)
spent 235 before the window; outstanding 98
machines made 11 in the window -> -87 unexplained, 30 needed
```

98 of credit outstanding when the window opened, so a cell rated at 15/min
would have had to produce **64/min** to clear it. That is not a rounding error.
**No self-feeding cell of this size could ever have passed**, however perfectly
its belts worked, because the plan that builds one must hand-smelt its own
belts in a machine of the same prototype.

## The position was already in the record

`Delivery` (`crates/core/src/record/mod.rs`) carries `item`, `count`, `entity`
and `slot` — **no position**. But the position is on the *sibling field of the
same event*: `ActionDispatched::target`, whose own doc comment says "Every
`mine`, `place`, `insert` or `remove` action carries a real position here,
always". Both archived runs have it, including the older one whose deliveries
are read from prose labels rather than from the struct.

**So there was no blocking gap and no Rust change.** Worth stating because the
brief allowed for one: the field to reach for was not the one the data lived
in, and checking cost less than adding it would have.

The sampled machines carry `position` too, so position is the join. It is the
only identity available — the samples key on the game's `unit_number` and no
event records it — which means two machines that ever stood on the same tile
are one machine to this analyser.

## What the grouping is now

A stage is `(machine, position, delivered item)`. From there, two rules,
because two different questions are being asked:

- **Within one machine instance, by maximum.** Coal and ore into the same
  furnace are two alternative bounds on that furnace's output, not two supplies
  that add.
- **Across instances, by sum** — but only for instances whose own counter makes
  the goal item. Two furnaces are two suppliers.

And the balance is then done **one machine at a time**: `outstanding_M =
max(0, credit_M - spent_M)` against that machine's own pre-window production,
`unexplained_M = max(0, made_M - outstanding_M)`. Surplus credit stays with its
own machine rather than subsidising the one next door — coal in furnace A
cannot smelt furnace B's ore.

### The floating pool: what still cannot be attributed

Credit that names no machine that could have spent it does not disappear; it
becomes a **floating pool** that may explain any machine's output.

- **A chest's `stock`/`charge`.** It names a container, and nothing in the
  record says which machine an inserter later fed from it.
- **A drill's coal.** It bounds the plates *some other* machine makes from its
  ore, so it belongs to no plate-producing instance.
- **A `target` that joins to no sampled machine** — a machine loaded before the
  first sample, or a planner intent the game resolved elsewhere. Floating, not
  attributed to a guess.

Floating stages are combined by **maximum, not sum**: they are alternative
explanations of the same downstream output, and adding them would invent supply
that never existed. The pool is drawn down against *every* machine's production
of the item, because whichever machine it reached, that machine's output spent
it.

**Every one of those choices is the one that refuses.** Credit is an upper
bound; attributing floating credit to a specific machine, or draining the pool
faster, would push towards passing.

## Both archived runs, before and after

Same commit, same fixtures, `iron-plate:15:7200`.

| | `run-1788674059-90744` (roster-fed) | `run-1788679826-02267` (belted cell) |
|---|---|---|
| **verdict before** | `roster-fed` | `roster-fed` |
| **verdict after** | `roster-fed` | **`short`** |
| credit before | 194 | 333 |
| credit after | 348 (194 named + 153 floating) | 412 (333 named + 79 floating) |
| spent before window | 40 | 235 |
| outstanding before | 154 | 98 |
| outstanding after | 268 | 136 |
| machines made in window | 30 | 11 |
| unexplained before | −124 | −87 |
| unexplained after | −113 | **+11** |

The regression run's verdict is unchanged, which is the point of it. Its credit
*rose*, because the drill's coal and the furnace's coal are now summed as a
floating pool plus a named machine rather than max'd as two prototype stages —
more credit, which refuses harder.

The belted cell is now refused for the reason it deserves. Its own furnace at
`[-5, -27]` received one coal charge worth 69 plates and had made 107 by the
time the window opened, so it has nothing outstanding and its 11 plates stand
alone against the 30 asked for. **`short` is still a refusal, and nothing was
tuned to make it pass**: the run was `SHORT` on the lead-in check too, and the
cause is on the machine's own status line — `full_output`, because nothing
takes the plates away.

## Could a self-feeding cell pass now?

Yes, in principle, and that is the change. The wall was structural: hand credit
grew with every furnace the plan built, and the belted furnace's output was
measured against all of it. Now the belted furnace is measured against its own
charge, which its own early output spends, and the only credit that can still
cover it is the floating pool — 79 plates in this run, drawn to zero by 235
plates of production before the window ever opened.

What a cell now has to do is **make plates that leave**. This one throttled at
11 in 7,200 ticks; it needed 30. The next thing to fix is on the belt, not in
the ledger.

## Still unattributable, and worth saying out loud

- **A chest's contents.** 79 plates of credit in this run that name a container
  and no machine. An inserter's source and destination would settle it; the
  record has neither.
- **A machine loaded before the record began.** The ledger starts at the first
  event.
- **`BURN_TICKS` is approximate** — it ignores a partial burn carried between
  crafts, and the mod does not send `energy_usage`.
- **Two machines on one tile at different times** are one machine here.

## Two process notes

**The fixtures are not mine.** `tools/fixtures/run-1788679826-02267` is the
archived belted run, copied unchanged except that its 711 `bots` sample rows
were dropped, recorded on `0831cc3c` by a session that reported this defect
rather than fixing it. `tools/fixtures/run-1788674059-90744` predates the
balance entirely. The synthetic cases in `InstanceGroupingTest` *were* written
by the author of the change and say so in their class docstring.

Five substitutions were run against the implementation, each asserted to have
matched before its result was read: pooling by prototype
again, attributing every delivery to whatever stands on its tile, letting
surplus credit subsidise a neighbour, dropping the position off `target`, and
summing the floating stages instead of taking the largest. Each went red, and
the first put the belted run back to `roster-fed` — `AssertionError:
'roster-fed' != 'short'`, which is the defect reappearing by name.

**A silent redefinition is invisible to a suite that does not exercise the
loser.** The first version of this change defined `pos_key` 1,600 lines below
an existing `pos_key`. All 31 tests passed — none of them call `frozen_bots` —
and the CLI died on its first real archive with `the JSON object must be str …
not tuple`. `ModuleSanityTest` now walks the module's AST and refuses two
top-level definitions of one name.
