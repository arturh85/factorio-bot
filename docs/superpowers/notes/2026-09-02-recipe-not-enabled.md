# The recipe was locked for 16 ticks, and the plan could not say so

`workspace/runs/run-1788365280-15443` (run 30), milestone 6 (`craft automation
science packs x10`):

```
milestone 6: satisfied after 2 iteration(s), best 31 steps,
  last error: game rejected the command: Unexpected Output:
  Error: recipe automation-science-pack is not enabled for this force
```

The same sentence `21a1228a` was written to make impossible, appearing at
*execution* time. It is **not** that defect leaking through, it is **not** a
stale plan, and it is **not** benign. It is a third thing: a real ordering gap
between what the plan can express and when the game applies a Factorio 2.0
trigger technology.

Fix: `crates/executor` — the executor now checks the `Condition::Researched` it
was given instead of inferring it from a predecessor's success.

## The three candidates, and which the record picked

### The plan was right. `21a1228a` is holding.

`events.jsonl`, milestone 6's first `plan_created` (tick 29189), last two nodes:

```
{"id":18,"bot":2,"action":"craft 1 lab","deps":[19,21,22],"planned_start":23541}
{"id":0, "bot":2,"action":"craft 10 automation-science-pack","deps":[4,18,22],"planned_start":23661}
```

`deps` includes 18. This is exactly the shape
`docs/superpowers/notes/2026-09-02-craft-ingredients.md` says the fix
guarantees, and it is the shape the broken run did not have. Candidate 2 (a
stale plan surviving a replan) dies here too: the second iteration's plan
(tick 53478) states the same edge, `{"id":0,…,"deps":[4,17,21,22]}` with 17 =
`craft 1 lab`. Both plans say the same thing.

### The executor honoured the edge.

Iteration 2, from the same `events.jsonl`:

```
77797 action_dispatched 17 craft 1 lab
77917 action_settled    17 success
77921 action_dispatched  0 craft 10 automation-science-pack
80930 action_settled      0 success
```

Four ticks after the lab, not at the start of the milestone. The executor waits
on the edge; it is not dispatching early.

### The game applies the unlock 16 ticks late.

`workspace/server-log.txt` is the mod's own stdout, and it settles it outright:

```
§53469§action_completed§ok 44
§53469§on_player_main_inventory_changed§{"player_id":2,…"lab"…}
§53485§on_research_finished§
§0§force§{"name":"player",…"current_research":"automation-science-pack","research_progress":1,…}
```

- **53469** — the lab craft completes (`on_player_crafted_item`), the lab enters
  bot 2's inventory, and the mod settles the action. This is the tick
  `events.jsonl` records as `craft 1 lab … success`.
- **53485** — `on_research_finished` for `automation-science-pack`. **Sixteen
  ticks later.** The trigger does not complete inside the craft event; the game
  makes the technology the current research and finishes it a few ticks
  afterwards.

Iteration 2's four-tick gap between settle and dispatch says roughly where in
that window the dependent craft went out. It landed with the recipe still
disabled, `rcon_action_start_crafting` checked
`player.force.recipes[recipe].enabled`, and answered the sentence at the top.

So: **candidate 1, a genuine race the edges cannot cover.** The plan already
says everything the planner knows. "The lab has been crafted" and "the
technology the lab triggers has been applied" are two different facts sixteen
ticks apart, and only the first one is a fact about the plan.

### Why the failed settle has no dispatch beside it

`events.jsonl` has an `action_settled` for id 0 at tick 53469 with
`elapsed_ticks: null` and no matching `action_dispatched`. That is not a gap in
the record — `record.actions` documents it as a finding — and it is not evidence
that the action was never sent. `rcon_action_start_crafting` refuses **before**
`stamp_tick()`, so a rejected craft never gets a game tick at all; 53469 is the
recorder's own high-water mark (`RunRecorder::not_before`), which is why it
coincides with the lab's settle rather than measuring anything. The tick that
proves the ordering is `§53485§` in the server log, not this one.

## It is not benign

The milestone did satisfy on the next iteration, which is why this looked like
it might be a speculative probe or a designed retry. It is neither. The failure
abandoned iteration 1 at tick 53469 with the whole chain complete — 50 iron ore
mined, smelted, gears, cable, circuits, belts and the lab all done — and
iteration 2 started over from mining coal and stone:

| | iteration 1 | iteration 2 |
| --- | --- | --- |
| started | 29189 | 53478 |
| lab crafted | 53469 | 77917 |
| outcome | rejected | satisfied at 80931 |

**27,462 ticks — 7.6 minutes of game time — redone because of a 16-tick race**,
on a run that then ran out of budget on milestone 7 and finished `stuck`.

## The fix

`crates/executor/src/run.rs`: `await_preds` now ends with `await_research`,
which reads the action's own `Condition::Researched(tech)` preconditions and
waits until the game confirms each one before the action is dispatched.

- `Actuator::technology_researched(tech) -> Result<Option<bool>, _>`, defaulted
  to `Ok(None)`. **`None` is "this actuator cannot say", not "no".** A caller
  that collapses the two waits out its whole budget on every craft for an
  answer that will never change — the same distinction `NoVerdict` keeps from
  `Rejected`, for the same reason.
- `RconActuator` answers from `FactorioWorld`, not over RCON. The mod's
  `on_research_finished` calls `writeout_recipes()` and `writeout_forces()`
  *before* it settles anything, so by the time a technology has landed
  `OutputParser` has already written it into the world. The answer is in memory;
  a round trip would be asking a question we can already read.
- Bounded, and it never fails the action: `RESEARCH_SETTLE_BUDGET` is 5s (the
  one measurement is 16 ticks ≈ 0.27s at 60 UPS, so an order of magnitude of
  headroom), after which the dispatch goes out anyway. The action's own verdict
  is then the report — the same trade, and the same wording, as
  `LAG_CHASE_BUDGET`. A technology that is never coming costs one wait, not a
  stalled run.

This is the discipline `wait_out_lag` already applies to machine time. A lag
edge says the furnace keeps working after the bot walks away; this says the
force keeps researching after the craft that triggered it settles. Both are the
world catching up with an ordering the plan got right.

### Tests, all verified red first

`crates/executor/src/run.rs`:

- `a_craft_waits_for_the_unlock_its_predecessor_triggers` — the run-30 shape.
  Without `await_research`:
  ```
  assertion `left == right` failed: the craft must go out only after the game
  confirmed the unlock, not merely after the craft that triggers it succeeded
    left: Some(0)
   right: Some(3)
  ```
  `Some(0)` is "dispatched having asked the game nothing", which is run 30.
- `an_unlock_that_never_lands_still_dispatches_after_the_budget` — drop the
  deadline and this reports `run did not terminate: some bot is waiting on a
  signal nobody will send`.
- `an_actuator_that_cannot_answer_is_asked_once_and_not_waited_on` — treat
  `Ok(None)` as `Some(false)` and this fails.
- `a_technology_the_world_already_has_costs_one_question` and
  `an_action_with_no_research_precondition_asks_nothing` — the common cases stay
  free, and the wait keys on the plan's stated condition rather than on "this is
  a craft".

`crates/executor/src/rcon_actuator.rs`: four tests on
`technology_researched_in`, including
`the_other_forces_in_the_world_do_not_get_a_vote` (below) and
`a_technology_the_force_does_not_list_is_unknown_rather_than_unresearched`.

`cargo test -p factorio-bot-executor`: 124 + 5 passed.
`cargo clippy -p factorio-bot-executor --all-targets -- --deny warnings`: clean.
`cargo test -p factorio-bot-scripting-lua`: 239 passed — it holds the only two
other `Actuator` impls, and a defaulted trait method left both untouched.

## Found on the way: the planner is planning for the *enemy* force

**Not fixed here — `crates/planner` belongs to another agent right now. This is
the report.**

`PlanState::from_world` (`crates/planner/src/state.rs:574`) picks the force it
plans for as:

```rust
let force = base.forces.iter().map(|entry| entry.key().clone()).min();
```

with the field doc arguing "Every world this plans against has a single force,
so the tie-break decides nothing in practice".

**That premise is false from the first research completion of every run.**
`writeout_forces` (`mods/BotBridge/control.lua:806`) loops over all of
`game.forces` — the audit note
`2026-09-02-factorio-21-api-audit.md` already flagged the ~430kB of waste this
causes, but not this. `OutputParser` writes each one into `FactorioWorld::forces`
by name, so after the first `on_research_finished` the world holds `enemy`,
`neutral` and `player`, and `min()` returns **`enemy`**.

Run 30's own force dumps, parsed from `server-log.txt`:

| dump | force | steam-power | electronics | automation-science-pack |
| --- | --- | --- | --- | --- |
| first (tick 0) | player | false | false | false |
| last (tick 53485) | player | **true** | **true** | **true** |
| last (tick 53485) | enemy | false | false | false |
| last (tick 53485) | neutral | false | false | false |

The first `on_research_finished` was at tick 26449. From there to the end of the
run — milestones 6 and 7 in full — every `PlanState` answered every technology
question from a force that never researches anything.

The visible consequence is in milestone 7's plans. `automation` has
`prerequisites = {"automation-science-pack"}`
(`workspace/data/base/prototypes/technology.lua`), which the *player* force
finished at 53485. Reading `enemy`, the planner does not believe it, so every
one of the five milestone-7 plans re-derives the trigger:

```
{"id":18,"bot":2,"action":"craft 1 lab","deps":[19,20,30,31]}
{"id":0, "bot":2,"action":"research automation","deps":[18]}
```

A second lab, crafted to trigger a technology the force already has, never
placed, on the milestone that consumed 85,030 ticks and ended the run `stuck`.

`recipe_gate` is mostly shielded from this by accident: it checks
`recipe.enabled` first, and `world.recipes` comes from `collect_recipes`, which
*does* hardcode `game.forces["player"]`. So recipe gating reads the right force
and technology questions read the wrong one. Two answers about the same game
from one `PlanState`.

Two independent fixes, either of which closes it, and they are worth doing
together:

1. **`mods/BotBridge/control.lua:806`** — `writeout_forces` should emit
   `collect_player_force()` only. The function already exists, is already used
   by the two RCON paths, and its own comment (`control.lua:755`) says
   `game.forces` "also holds `enemy` and `neutral` … [which] describe nobody the
   planner plans for". It was simply never applied here. Saves ~287kB of stdout
   per research completion as well.
2. **`crates/planner/src/state.rs:574`** — name the force rather than sorting
   for it. `min()` is reproducible, which is what it was chosen for, but
   reproducibly wrong is not better. `crates/executor/src/rcon_actuator.rs`'s
   `BOT_FORCE` now names `"player"` for exactly this reason, with a test that
   fails if the lookup is changed back to a sort.

Doing only (1) leaves the planner one stray `writeout_forces` away from the same
state; doing only (2) leaves the bandwidth. Neither is this note's to make.

## Not investigated

Milestone 7's own terminal failure is a separate question and is not touched
here. For whoever picks it up: at tick 158217 the mod refused with `cannot
research automation: researched=false enabled=true trigger=false
unmet_prerequisites=[]` — i.e. `LuaForce.add_research` returned false with every
reason `start_research` knows how to name ruled out. The lab is crafted and
never placed, which is the more likely story, but nothing here establishes it.

## Evidence hygiene

Run 30 postdates `fcb4ed68`, so its record can be read for absence as well as
presence (`docs/superpowers/specs/2026-09-02-material-convergence-design.md`
§2.3.1). Runs 28 (`run-1788358260-07659`, four occurrences) and 29
(`run-1788361433-78052`, three) carry the same fingerprint; runs 1–27 do not,
but they predate the record that could show it, so **nothing here claims this
started recently**. The 16-tick measurement stands on the server log, which is
the game's own event ticks and is not subject to that caveat at all.
