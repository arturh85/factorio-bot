# `writeout` cannot escape an RCON-invoked function

2026-09-06. **A fix I shipped, measured, and then removed**, because it was
inert. The finding is worth more than the fix would have been.

## The gap, which is real and still open

A blueprint's ghosts never reach the planner's world model.
`rcon_place_blueprint` returns them in the **RCON reply body**, which the
executor reads as the action's result and nothing else ever sees. The model is
fed by `writeout` on **stdout**. A script-driven `build_blueprint` raises no
`on_built_entity`, so `on_some_entity_created` never fires for a ghost.

Measured live on a 9-entity block, three numbers from one run:

| | count |
|---|---:|
| entries in the RCON reply body | 9 |
| ghosts standing in the **game** | 9 |
| ghosts visible to the **planner** | **0** |

The cost is that `method::blueprint::recover_anchor_from_ghosts` — correct
code, with passing unit tests — has been unable to fire since it landed.

## What I shipped, and why it did nothing

`7842f7a5` added a loop in `rcon_place_blueprint` that called `writeout` for
each remaining ghost, with five tests proving the loop runs and passes the
right record on the right key. **All five were true and all five were
useless**: they capture `print` inside a Lua stub, where `print` works.

## The measurement that killed it

The peer session had already eliminated a race (25 retries with RCON round
trips), the bounding box (a live ghost reports 0.7969 x 0.7969, comfortably
non-zero), deserialization (zero `failed to deserialize entity` lines) and the
filter itself. What remained was the channel.

**The control is the whole experiment.** I wrote a plain **non-ghost** record —
a `stone-furnace` at a fixed position — on the same channel, from the same
loop, in the same call. It also never arrived.

That single control removes every hypothesis about the *record* at once. It is
not the ghost shape, not `ghost_name`, not the filter, not the graph's
whitelist. **`writeout` cannot escape a function invoked through `remote.call`
from an RCON command.**

## Why, and the warning that already existed for one case

This is the same mechanism CLAUDE.md already documents for `rcon.print`:

> do not add `rcon.print` *inside* a mod function the executor calls — that
> output lands in the RCON reply body and the executor reads it as the
> action's result.

Factorio redirects console output to the RCON client for the duration of the
command. The existing warning was written about one function and is really
about the whole channel: **`print` and `rcon.print` both go to the caller, not
to stdout, while an RCON command is in flight.** `writeout` is exactly one
`print`.

## The deferral, which also failed

`todo_next_tick_other` is drained inside `on_tick`, an ordinary event handler
with ordinary stdout, so queueing the writeout there should escape. Tried, with
the same non-ghost control: **also did not arrive.** The negative is therefore
about the channel and not the record, but *why* is not established. One
candidate, unverified: that queue is drained in an `elseif`, so a non-empty
`todo_next_tick` starves it indefinitely.

## What is in the tree now

**Nothing.** The loop is removed and replaced by a comment stating the finding
where the next person will look for it. Leaving a `writeout` there would have
left a call that reads as a fix, passes its tests, and does nothing — the exact
defect this repo has spent the day cataloguing, committed by the person
cataloguing it.

The five tests went with it. A test suite for a mechanism that is not there is
worse than no suite.

## What to try next

A channel **known** to escape:

* a real event handler — have the mod notice the ghosts on a later tick through
  an event it already receives, rather than being told about them by RCON;
* the sampling session's own writer, which demonstrably reaches the record;
* `on_chunk_generated`'s path, which is proven — the exploration run's census
  grew, so writeouts from event handlers arrive.

The last one is the useful control for whoever picks this up: **event-handler
writeouts demonstrably work in the same process, in the same run.** So the
channel is not broken in general, only from RCON context.

## The shape, for the pattern collection

The peer's original finding was **a reader with nothing to read**: ghost
recovery verified against a `PlanState` hand-populated with ghosts, which is
precisely the step the live path never performs.

Mine was the mirror image: **a writer whose output goes nowhere**, verified
against a stub where `print` is captured. Each half was proven correct against
a fixture that supplied what the other half never delivers. Neither test was
wrong; both were about a world that does not exist.
