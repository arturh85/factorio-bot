# Run recording — what landed, and what it found

Overnight of 2026-08-31 into 2026-09-01. Companion to
`2026-09-01-open-decisions-from-the-upgrade-sweep.md`, which covers the
dependency work done in parallel.

## What exists now

A run is a durable artifact. `record.start()` mints an id, creates
`<workspace>/runs/<id>/`, and starts frame capture with that same id so the log
and the frames cannot disagree about which run they belong to. The run writes
`events.jsonl` as it goes, and `record.finish()` derives `splits.json` and
`manifest.json` from that log and copies the run's frames out of the workspace
before the next run wipes them.

`/api/v1/runs` serves it, and `/runs` in the web app shows it: splits as a
speedrun table, a tick scrubber that plays or seeks, the frame at-or-before the
cursor per bot and camera, per-bot task lanes, and a delta column against
another run.

Specs: `2026-08-31-supervisor-loop-design.md`,
`2026-09-01-run-recording-and-replay-design.md`.

## The finding that matters: `stuck_silent` fired, and it was right

A four-milestone run (`workspace/scripts/showcase.lua`) gathered ore, smelted
plates, then spent 61345 ticks on `goal.researched("automation")` before the
supervisor halted it as **`stuck_silent`** — no progress, and *every run
reported success*.

That state exists precisely to catch "success reported for work that did not
happen". It fired on its first long run, and the log says what was happening:

    plans:  63 steps -> 26 -> 4 -> 4 -> 4   (stall 0, 0, 0, 1, 2)
    every run: failed=0

    dispatched, all reporting success:
       7x  mine 5 stone
       7x  craft 1 stone-furnace
       6x  mine 1 coal
       5x  research electronics
       5x  research steam-power

**`electronics` and `steam-power` are `research_trigger` technologies** — this
codebase names both of them in `crates/core/src/types.rs`, and they do not
consume science packs at all; they complete when their trigger is satisfied.
The run dispatched `research electronics` five times, was told success five
times, and the technology never became researched, so the planner planned it
again.

*Hypothesis, not verified:* a `Research` action cannot complete a
trigger-unlocked technology, so issuing one succeeds at the RCON level and
changes nothing in the world. The planner does model triggers — it has
`UnsupportedResearchTrigger` and states that only `craft-item` triggers can be
planned — so the gap is likely between planning the trigger and *performing*
it, not in recognising it. Someone should confirm before fixing.

### The planner fix needs a concept that does not exist yet

Attempted and **reverted**. For a `craft-item` trigger the planner does two
things, and both are wrong:

* it subgoals `Goal::Have { trigger_item }`, which is satisfied by
  **possession** -- a bot already carrying a lab crafts nothing, so the trigger
  never fires;
* it then emits an `ActionKind::Research`, which does nothing at all, because
  once the trigger fires the game researches the technology itself. That action
  is what looped: five dispatches, five reported successes, no research.

Replacing it with "emit the craft, carrying `Effect::Researched`" fixes both --
and breaks a third case. A trigger item can be produced by **smelting**, and a
hand-craft action for it cannot run. Guarding on `CRAFTING_CATEGORY` then
refuses trigger technologies the old model planned fine. Both models are wrong,
in opposite directions.

What it actually needs is a goal that demands **production** rather than
possession -- something like `Goal::Produced { item, count }` that every
production method (craft, smelt, mine) can satisfy, with the `Researched`
effect riding on whichever action ends up producing it. A method cannot attach
that effect to a subgoal's action after the fact: it never sees their ids, by
design (`have.rs`, "No explicit `Link` steps").

That is a spec-level change touching every production method, so it is written
down here rather than guessed at.

**The good news is that this no longer fails silently.** With the reply now
judged, a trigger technology refuses immediately and says why, instead of
looping for 61345 ticks while reporting success.

### `Goal::Produced` landed, and moved the failure

`cf0d7bff`. The trigger loop is fixed, and the live before/after on the same
four-milestone run is unambiguous:

    before   stuck_silent   plans 63 -> 26 -> 4 -> 4 -> 4   no errors at all
    after    stuck          plans 57 -> 37 -> 27 -> 26 -> 25   a named error

The plan now genuinely converges, and the halt is `stuck` -- failures reported
-- rather than `stuck_silent`, which is the state for "no progress while
everything claims to work". Trigger technologies plan as pure production:
`researched(automation-science-pack)` is 34 steps with **zero** research
actions, where it used to emit one that did nothing.

**It does not yet complete**, and the thing now blocking it is different in kind:

    tried to remove 10 copper-plate but removed 9

That is a *correct guard*, in `rcon_remove_from_inventory`, catching a real
discrepancy: the plan pulls ten plates from a furnace that has produced nine.
Not a logic error -- a smelting-time estimate, the same drift class as mining
running 2x and walks 1.4-1.6x over prediction. The supervisor absorbs it by
replanning (57 down to 25) but does not converge inside the iteration cap, and
the final iteration ran 25 planned steps while dispatching **zero** actions,
which is its own question.

So: the research critical path is now blocked on *estimation*, not on a silent
lie. That is a much better place to be stuck, and a different piece of work.

### The furnace lag, and why it was the only one of its kind

`tried to remove 10 copper-plate but removed 9` was not a rate error. Measured
against a real stone furnace, plates arrive about 3.2 s apart -- exactly what
`smelting_ticks` models. The mistake was using that number as a **point
estimate** where it functions as a **schedule constraint**: a removal placed at
the predicted completion is right half the time by construction.

The two sides are not symmetric, which is what settles the fix. Being early
costs a whole replan cycle. Being late costs scheduled slack the bot spends on
other work anyway -- that is what the lag is *for*. So the lag now carries one
craft cycle of headroom, covering a real mechanism rather than padding a guess:
the furnace cannot begin before the ore lands, and the insert action's reply
tick is when the *mod* returned, not when the furnace next looked at its input
slot.

**Audited for the same shape and found exactly one instance**, which is worth
knowing. The smelt lag is the only `Step::Link` carrying a lag in the planner.
Everything else is a `duration`, and the difference is what made this one
fragile: a duration is a makespan estimate while the executor waits on an
actual completion signal, so under-estimating it costs accuracy in a prediction.
A lag gates a *dependent action on elapsed time* with no signal behind it, so
under-estimating it dispatches into a world that is not ready. One is a
prediction, the other is a promise.

### A measurement instrument that read a constant

Reading the game clock from a script was not possible, so `rcon.last_tick()` is
new. The first probe built on it reported all ten plates arriving at `+0 ticks`
-- a result that would have been written up as "smelting is instantaneous" if
the wall-clock timestamps in the log had not been three seconds apart.

The clock was frozen. `last_tick` is advanced by *timed* calls, and the query
calls -- `inventory_contents_at`, `find_entities_in_radius` -- use the untimed
`remote_call` and never take a stamp off the reply. Polling with one leaves the
clock at whatever the last action reported.

Worse: the documentation I had written for `last_tick` an hour earlier
recommended `inventory_contents_at` as a timestamped sample. The doc now names
which calls advance it and warns about the frozen loop specifically. A wrong
doc on a new primitive is a trap laid for whoever uses it next.

### Ghosts and the entity graph: checked, not a defect

`EntityGraph` filters `FlyingText` and `Fish` by `entity_type` but not
`entity-ghost`, which looked like it would let a ghost furnace be reasoned
about as a real one. **It does not.** Probed live: place two furnace ghosts,
then ask the graph what is there and try to build on them.

    placed 2 ghost(s); first at -1.0,24.0
    graph sees NOTHING at the ghost position
    building a real furnace on the ghost -> ACCEPTED

Both halves are right: ghosts never reach the graph, and they do not block a
real build -- which matches the game, where a ghost is buildable-over.

Worth recording how nearly this went the other way. The live world snapshot
contains five `entity-ghost` records, which looks like proof that ghosts do
reach the world. They are **entity prototypes** -- `entity-ghost`,
`entity-unknown`, `tile-proxy`, all with zero-size bounding boxes -- not placed
entities. Concluding from them would have produced a confident answer about
placed ghosts from data that contains none.

### What was ruled out

`rcon_add_research` discards the return value of `LuaForce.add_research`, which
is documented as a boolean saying whether the technology entered the queue.
That looked like the answer -- the same shape as the discarded `remove()` count
fixed in `place_blueprint` earlier the same night -- so a guard was added and
then **reverted**, because it could not be shown to fire.

Probed live: `electronics`, `steam-power`, `automation`, `automation` a second
time, and a technology that does not exist. **All five were accepted.** A
nonexistent technology being accepted is not something this explanation
survives, and the mod under test was confirmed to be the edited one --
`workspace/mods` is the single copy, symlinked from both the server and the
clients.

So: the discarded boolean is still a latent defect worth fixing on its own
merits, but it is **not** the cause of the research loop, and the guard was
removed rather than shipped as a check that never fires. Whoever picks this up
starts from "why does `add_research` accept a technology that does not exist",
which is a much sharper question than the one this night started with.

Archive: `workspace/runs/run-1788225725-75943/` — 190 events, 666 frames,
4 splits, and the whole loop visible in the task lanes.

## A second finding, seen once and not reproduced



**`goal.plan` can raise `PreconditionUnsatisfied` on a large goal after earlier
milestones have run.** Seen on one run and *not* on the re-run, which reached
milestone 4 and planned 63 steps — so it is world-state dependent and may be a
symptom of the same research loop rather than a separate defect. Reproduced live: a four-milestone run gathered iron ore,
gathered copper ore and smelted iron plates, then raised

    precondition has 50 iron-plate of action ActionId(10) does not hold for bot 1

on `goal.researched("automation")`.

The same goal plans **fine** in isolation — 103 steps, makespan 43857, against a
world where the bot holds nothing. It fails once the bot is carrying the output
of three completed milestones.

Two things make this worth someone's attention rather than a shrug:

- `crates/planner/src/schedule.rs:1067` states that `PreconditionUnsatisfied`
  means *"the world was not as planned"* and that re-planning from observed
  state is the response. But this arrives from `goal.plan` itself, which
  **already plans from observed state**. Either the expansion under-produces
  the actions that would satisfy its own precondition, or the precondition is
  stated against a world the scheduler does not have.
- It is inventory-dependent, which is why no test catches it: fixtures start
  from an empty bot.

Not investigated further — found at the end of the night, and it wants a fresh
head rather than a guess.

## Defects found and fixed, all of them silent

Every one of these produced a system that looked healthy. None was caught by a
test; all were caught by reading real output.

- **`get_contents()` changed shape in Factorio 2.0** — an array of
  `{name, count, quality}`, not a `name -> count` dict. Every in-Lua read still
  indexed it as a dict, so `rcon_place_blueprint` could never build anything and
  `rcon_revive_ghost` refused every request. Invisible because the *Rust* side
  had already adapted, so inventories crossing the wire were correct while the
  mod could not act on them.
- **An inserter's `direction` points at the side it picks up from**, not the
  side it drops into. Every inserter in a generated blueprint was reversed. The
  layout placed 168/168 and moved nothing.
- **`only_ghosts = true` validates nothing.** Ghosts do not collide, so a
  deliberately overlapping blueprint places exactly as many ghosts as a correct
  one. A "168/168, geometry confirmed" result was a false positive that only a
  negative control caught.
- **Power coverage is not power capacity.** Every consumer inside a pole's
  supply area, network connected, and the line still did nothing because
  generation was short. It does not degrade into "slow"; it reads as dead.
- **A fabricated tick.** `record.start()` recorded before sending any command,
  so `last_tick()` was honestly `None` and an `unwrap_or(0)` turned that into
  tick 0 while every later event sat near 59000.
- **An absolute tick reported as a duration.** A run lasting 871 ticks reported
  60246 — seventeen minutes instead of fifteen seconds.
- **A milestone finishing before its own work.** Milestones were stamped from
  the last RCON reply, but an action's *completion* arrives through the mod's
  stdout and never touches that clock. A split read 381 ticks for a span of 864.

## The shape worth remembering

Two sessions independently landed on the same thing tonight: **the check that
works is the same operation against a known-good reference, not an assertion
about the new thing.** Generating `types.lua` on the old schemars before the
rewrite and diffing. `grep -c "listening on"` before and after installing a
tracing subscriber. Serving one `dist` from two tower-http versions. A
deliberately-overlapping blueprint next to a correct one.

A test asserts what you already believed. A reference comparison does not.

Its companion: **a number that fits two stories.** A `000` from curl that was a
shell quoting bug and looked like a broken endpoint; a 435-byte asset that
looked like an HTML fallback and was just a small file. In both cases the wrong
story was the more alarming one, so the pull was toward reporting it.
