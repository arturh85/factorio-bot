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

## The open finding

**`goal.plan` can raise `PreconditionUnsatisfied` on a large goal after earlier
milestones have run.** Reproduced live: a four-milestone run gathered iron ore,
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
