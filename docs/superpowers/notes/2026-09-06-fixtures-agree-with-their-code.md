# A test written beside the code agrees with the code

2026-09-06. Four instances in one night, across two sessions, each found
only by measuring against the running game. This is not four coincidences;
it is one failure mode with four faces, and it is the most expensive thing
in this project's history apart from unrepeatable measurements.

## The four

1. **A blueprint fixture placed a stone furnace where a 2×2 entity cannot
   legally stand.** The belt-routing primitive passed four clean task
   reviews and a full suite while being unable to connect any real machine.
   Only a whole-branch review that checked the fixture against real
   prototype geometry caught it.
2. **A resource fixture gave its ore tile a `unit_number`.** Resource
   entities do not have one. The drill accumulator keyed on it, credited
   nothing to any drill, and reported `produced: 0` for a drill that had
   just mined 133 ore — which was then written into the plan record as a
   finding about the *planner* before it was retracted.
3. **An obstruction test placed a decoy facing the blueprint's own default
   direction**, so the rule "an entity standing as designed is not an
   obstruction" read the decoy as the block already standing. The tests
   would have passed while asserting nothing.
4. **A replan-stability test never moved the roster**, so it would have
   passed under the roster-centroid seed it exists to forbid.

## Why it keeps happening

In each case **the same task wrote the code and the fixture**. A fixture is
a statement about the world; when its author is the code's author, it is
written to be the world the code expects. Nothing in a green suite can
detect this, because the suite is exactly the thing that has been made to
agree.

Two constants had the same shape without a fixture: `R + 1.1` as a walk
margin, carried from one observation in August against a measured worst
case of 0.301; and a walk speed of 0.15 taken from the prototype against a
measured 0.1413 over 196,717 ticks. Documented prose is a fixture too.

## The move that would have caught all four, in seconds

**Make the test fail on purpose before believing it.** Swap in the value the
test exists to forbid, watch it fail with the actual mismatch, revert, watch
it pass. The siting session's implementer did exactly that for the
replan-stability Critical — substituted the forbidden roster-centroid seed,
watched `Pos(0,0)` against `Pos(500,500)`, restored — and its re-reviewer
then **independently reproduced the experiment rather than trusting the
report**, doing the same for a second finding by flipping a flag back and
watching the bystander test fail.

Every one of the four above would have been caught this way. The drill
accumulator reporting `produced: 0` would have been obvious the moment
someone asked it for a number it should have been unable to produce; the
decoy furnace would have failed the instant it faced a direction the
blueprint did not; the 2×2 furnace fixture would have failed on any legal
position.

A green test proves nothing until it has been seen to go red for the right
reason. Treat "it passes" as an unverified claim about the test.

## The sibling: a crate-scoped check cannot see a crate it does not compile

Same night, same shape, different surface. A `PlannerError::NoSiteFound`
variant was added three tasks before anyone noticed that
`crates/scripting_lua/src/globals/goal/mod.rs:355` matches `PlannerError`
exhaustively. **Three tasks passed and two reviews signed off**, because
every brief said `cargo test -p factorio-bot-planner` — so the crate that
breaks was never compiled once. The Global Constraints did say
`--workspace`, but no task *step* ever did, which made the constraint
decorative.

**A task that adds a variant to a shared enum must build every crate that
matches on it.** A crate-scoped check cannot see a break in a crate it does
not compile, and neither can a crate-scoped review. If a brief names a
crate-scoped command in its steps, that is the command that will be run,
whatever the preamble says.

Known exhaustive matches in this workspace, as of 2026-09-06: over
`PlannerError`, only `goal/mod.rs:355` — everything else uses a wildcard or
`matches!`. Over `Goal`, three: `method/have.rs`'s `holds()`,
`scripting_lua/globals/goal/value.rs`, and `server/game/control.rs`.

## What actually caught them

Measurement against the live game, every time. Not review, not more tests,
not more careful reading. The ladder this project already keeps — offline
plan in seconds, headless run in minutes, 1x client run for a number — is
what makes that affordable, and the middle rung is where all four surfaced.

## The rules that follow

- **Build test worlds from prototype data or a real dump**, or assert the
  fixture against one. `workspace/scripts/map.json` is a real world; a
  hand-typed position is a guess.
- **A new instrument's first reading is evidence about the instrument.**
  Validate a counter, a margin or a classifier against something that
  already knows the answer — the force's own production statistics, a
  second query, a hand count — before believing what it says about the
  system.
- **A constant with no measurement behind it is a defect waiting.** If a
  number in this repo has no note saying who measured it and when, treat
  quoting it as a claim you are making, not a fact you are citing.
- **Prefer measuring to quoting this repository's own prose.** Three
  documented constants were wrong in one day (`R + 1.1`, the walk speed,
  the MinerLine block dimensions).
- **When a task writes both the code and its fixture, say so in the report**
  and name what the fixture assumes. That sentence is cheap and is the only
  warning a later reader gets.
