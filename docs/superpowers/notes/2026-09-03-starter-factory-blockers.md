# The two blockers in front of the starter factory

**Date:** 2026-09-03. Both fixed, both committed, `crates/planner` untouched in
every other respect and **every existing makespan pin passing unchanged**.

* `ee623717` — fix(supervisor): a goal nothing can answer is not a goal that is done
* `cef7bd19` — fix(planner): a mining drill may stand on the ore it mines

These are D0 and D6 of
`docs/superpowers/specs/2026-09-03-starter-factory-design.md`, and its §12 lists
them as the two prerequisites that "block a live run outright". Neither is a
piece of `Goal::Producing`; both had to land **before** anything can construct
one, which is why they are their own change.

Nothing here was run against a game. Everything below is source, tests, and
`crates/core/tests/live-2.1.17-world-snapshot.json`.

---

## Blocker 1 — the absence of a verdict was recorded as success

### What it was

`scripts/supervisor.lua`'s empty-plan branch asked `goal.holds` and treated two
of its three answers as satisfaction:

| `goal.holds` | before | after |
|---|---|---|
| `true` | `satisfied`, reason `already_satisfied` | unchanged |
| `nil` | **`satisfied`, reason `plan_empty`** | **`stuck`**, `t.refusal.code = supervisor::unanswerable` |
| `false` | raises — "refusing to report it satisfied" | unchanged |

`plan_empty` is a **`SatisfiedReason`** (`crates/core/src/record/mod.rs:459`),
so that middle row closed the milestone *done*, wrote a `MilestoneSatisfied`
event, and produced a `satisfied` split. `holds` returns `nil` for exactly two
goal kinds today — `Goal::Produced` and `Goal::Producing`
(`crates/planner/src/method/have.rs:212`, whose own fixture at `have.rs:3125` is
named `unanswerable`) — and it returns `nil` because *possession cannot settle
those goals*, which is the honest answer, not a defect.

It cost nothing so far only because every goal a script can build today is a
`have` or a `researched`, both of which `holds` answers. It becomes a factory
reported built on nothing the moment a `Producing` method exists and returns an
empty step list for **any** reason — a site it believes is already built, a
shortfall it computed as zero, a refusal it swallowed. This is the same failure
shape as run 30's structural claim passing on a base with 0 kW, and the same one
`app/src/api/frameJoin.ts` refuses to make about a missing run id.

Worth stating plainly: **`Goal::Producing` is not reachable through this branch
today**, because no method claims it, so `expand` raises `NoApplicableMethod` —
which `goal.refusal` already classifies as a verdict and the loop already closes
`stuck`. The hole opens on the *first* method that claims the goal. That is the
next task, which is why this one is done first.

### What it reports now, and why

**`stuck`, with the reason on `t.refusal`, code `supervisor::unanswerable`.**
Four decisions, each against the existing vocabulary:

1. **Not `satisfied`.** Nothing established that the goal is met. `holds`
   answering `nil` is a statement about the planner's model, not about the
   world, and reading it as "done" is asserting something nobody checked.

2. **Not a raise, unlike `false`.** `false` is a *contradiction* between the
   planner and the world — a defect, with nothing a retry could change, so it
   ends the run loudly. `nil` is the planner correctly declining to model
   something. Nothing is broken, and a run must be able to end on it with its
   record intact. That is run 31's lesson (`7de7c6a0`): the run that produced
   the best result so far is the one whose record could not say what happened to
   the milestone it died on, because a legitimate verdict was raised past the
   loop.

3. **`stuck`, not `stuck_silent`, and not a seventh word.**
   `crates/core/src/record/splits.rs` already argues both halves of this for
   planner refusals and the argument transfers verbatim: `stuck_silent` means
   "no progress and *nothing to show for it*", and this has a sentence to show;
   and the outcome vocabulary says *how far a milestone got* — "as far as this
   loop can establish" — while *what kind* of stop it was is data the event
   already carries. A word nothing consumes is also a word the viewer has no
   style for.

4. **On `t.refusal`, not a new field.** `t.refusal` is already the channel a
   driver reads for "why the milestone closed with no action dispatched":
   `scripts/research_run.lua:227` computes
   `why = (t.refusal and t.refusal.message) or sup.first_error` and hands it to
   `record.milestone_stuck`. A new field would have to be taught to every
   driver, and a driver that had not learnt it would record this halt with **no
   reason at all** — which is precisely the defect `7de7c6a0` closed. The
   `code` is what keeps the two kinds apart for a reader without matching on a
   sentence: a planner refusal carries the planner's own miette code
   (`planner::research_needs_power`), and this carries `supervisor::unanswerable`,
   which no `PlannerError` can produce.

The halt carries **no `reason`** field. `reason` is a `SatisfiedReason`, and
putting one on a halt would put `plan_empty` back on the record through a
different door.

### What was deliberately left alone

`SatisfiedReason::PlanEmpty` stays in `crates/core`. No live writer emits it
now, but archived runs on disk contain it and deserialising those has to keep
working. Its doc comment at `record/mod.rs:476-480` is now **stale** — it says
"the planner exposes no way for a script to check whether a goal already holds
independently of planning it", which `goal.holds` has not been true of since it
landed. `crates/core/src/record/` is outside this task's boundary; flagged, not
edited.

### Red-first, with the actual output

Tests written first, `scripts/supervisor.lua` restored to `HEAD`, then
`cargo test -p factorio-bot-scripting-lua --lib supervisor_lib`:

```
---- a_milestone_satisfied_by_an_empty_plan_records_which_it_was stdout ----
assertion `left == right` failed: a goal possession cannot settle was never
satisfied by anything, so no satisfaction may be recorded for it
  left: Some("plan_empty")
 right: None

---- an_unanswerable_goal_still_gets_its_work_done_first stdout ----
assertion `left == right` failed
  left: "done"
 right: "stuck"

---- an_unanswerable_goal_with_an_empty_plan_is_not_a_satisfaction stdout ----
assertion `left == right` failed: nothing established that this goal is met,
so the loop must not say it is
  left: "done"
 right: "stuck"

---- an_unanswerable_halt_carries_its_own_code_not_the_planners stdout ----
driver runs: RuntimeError("attempt to index a nil value (local 'seen')")

test result: FAILED. 32 passed; 4 failed
```

The fourth is red *by absence*: there is no `halted` transition to capture, so
the driver indexes `nil`. After the fix: `36 passed; 0 failed`.

### Mutation evidence

Each mutation applied to the fixed `scripts/supervisor.lua` alone, the whole
`supervisor_lib` suite run, tree restored between each.

| Mutation | Failed |
|---|---|
| `code = "planner::unanswerable"` | `an_unanswerable_halt_carries_its_own_code_not_the_planners` |
| `_close("stuck_silent")` instead of `_close("stuck")` | the same one |
| `if held ~= true` → fires for every empty plan | **11 tests**, incl. `an_empty_plan_for_a_goal_that_does_hold_is_still_satisfied`, `a_satisfied_milestone_never_runs_anything`, `the_source_is_asked_again_after_a_milestone_is_satisfied` |
| the halt also carries `reason = "plan_empty"` | `an_unanswerable_halt_carries_its_own_code_not_the_planners` |
| the refusal is not stored on `self.refusal` | `an_unanswerable_goal_with_an_empty_plan_is_not_a_satisfaction`, `an_unanswerable_halt_carries_its_own_code_not_the_planners` |

The third row is the one that matters. **`an_empty_plan_for_a_goal_that_does_hold_is_still_satisfied` passes both before and after the fix and cannot go red from the change itself** — it is a negative control, not a regression test, and left at that it would be exactly the kind of pass this project has learned to miscount. The over-reach mutation is what gives it teeth: it is red the moment the halt widens from "unanswerable" to "empty", which is the one way this fix could break every run that finishes.

Similarly, `a_milestone_with_work_to_do_is_never_asked_whether_it_already_holds`
(pre-existing) and its new sibling `an_unanswerable_goal_still_gets_its_work_done_first`
pin that `holds` is consulted **once**, on the empty re-plan — a change that
asked earlier, or halted on the answer rather than on the answer *plus* an empty
plan, stops a run before it does anything.

---

## Blocker 2 — resource tiles were occupancy for everything, including drills

### What it was

`PlanState::is_area_clear_of` ended:

```rust
!tiles_under(area).iter().any(|tile| self.base.entity_graph.any_resource_at(tile))
```

Unconditional, for every entity. A mining drill is *defined* by standing on a
resource, so this refused a drill **every site it could ever have, on every
map** — not rarely, not on crowded patches: totally. Stage 1 of the starter
factory is a burner drill dropping into a stone furnace and could not have
placed its first machine.

Nothing had hit it because the only entity this planner has ever placed is a
stone furnace (all 329 entities across 21 archived runs), which wants to be
*near* ore and never on it.

### The fix, and where it departs from its precedent

`is_area_clear_of` gains `resource_blocks: bool` beside the existing
`water_blocks`, consulted by exactly one of the six occupancy sources.
`is_area_free_facing` passes `!self.stands_on_resources(name)`;
`is_area_clear`/`is_position_free` pass `true`. Everything else keeps blocking:
an entity, a character or a refused footprint standing on an ore patch still
blocks a drill.

Two arguments rather than one bundled "what is being sited", because
`is_position_free` names no entity — it is the tile-granularity question
`Condition::PositionFree` asks — and must get the strict answer to both.

**Where it does not follow the pump, and this is the load-bearing part.**
`collides_with_water` reads the prototype's `collision_mask`. The mask cannot
decide this one:

```
stone-furnace   collision_mask ["is_lower_object","is_object","water_tile","item","object","player","meltable"]
burner-mining-drill              ["is_lower_object","is_object","water_tile","item","object","player","meltable"]
iron-ore                         ["resource"]
```

(from `crates/core/tests/live-2.1.17-world-snapshot.json`; the 1.x fixture
agrees, in its own layer spelling.) **No building carries the `resource`
layer**, and a resource entity collides on that layer alone — so *Factorio
itself would let a stone furnace be built on an ore patch*. A mask-derived
predicate would have answered "does not collide with resources" for **every
building** and silently deleted the rule instead of exempting one machine
from it.

So the rule `is_area_clear_of` enforces is a **policy of this planner's**, not a
reading of the game's collision rules: do not bury the patch you are about to
mine. `cf89b493` is where it came from ("Siting moved off the copper patch
edge"). A policy has to be exempted exactly as narrowly as it is stated, so:

```rust
pub fn stands_on_resources(&self, name: &str) -> bool {
    match self.base.entity_prototypes.get(name) {
        Some(proto) => proto.entity_type == "mining-drill",
        None => false,
    }
}
```

`entity_type` is the game's own classification and is the one field both
captures in this repo agree on (the mask layer *names* differ between 1.x and
2.x, which `collides_with_water` already has to work around by matching both
spellings). It admits `burner-mining-drill`, `electric-mining-drill`,
`big-mining-drill` and `pumpjack` — every one of which stands on a resource,
crude oil included — and nothing else.

`None => false`: an unknown prototype gets **no** exemption, the same direction
`collides_with_water` falls in. The two mistakes do not cost the same. A wrongly
exempted entity is a machine buried on an ore patch that nothing later can
explain; a wrongly blocked one is a refusal with a site one tile over. (In
practice an unknown name never reaches the question — `is_area_free` has already
refused it for having no size.)

### What this does *not* fix

* **A drill and the `Mine` method still compete for the same tiles.** A drill
  standing on ore does not claim that ore in `PlanState`, and nothing
  decrements a patch for machine consumption (`Effect::ConsumeResource` is
  emitted by `Mine` alone). §13 rows 3 of the spec, unchanged.
* **A furnace can still be sited one tile off a patch edge**, exactly as
  before. This changes nothing for any entity that is not a mining drill.
* **Nothing has placed a drill in a game.** The spec's §15.1 — does a burner
  drill's output actually land in a stone furnace two tiles away — is still
  unanswered and is still the cheapest thing to check first.

### Red-first, with the actual output

Tests written first against a `stands_on_resources` stub returning `false`
(a bare test would not have compiled, which is a compile error and not a red
test):

```
---- state::tests::a_mining_drill_may_stand_on_the_ore_it_mines stdout ----
panicked at crates/planner/src/state.rs:2777:9:
a burner drill on iron ore is the whole of stage 1

---- state::tests::an_unknown_entity_gets_no_exemption stdout ----
assertion failed: s.stands_on_resources("burner-mining-drill")

test result: FAILED. 51 passed; 2 failed
```

`ore_still_blocks_everything_that_is_not_a_drill` and
`the_drills_exemption_is_ore_and_only_ore` **passed in the red run and pass
now**. They cannot go red by construction from this change, because the
behaviour they pin is the behaviour that existed before it — they are the
controls on the fix not widening, and they are stated as such. What they pin is
below.

After the fix: **464 planner tests pass** (`cargo test -p factorio-bot-planner`,
15 binaries, all ok), clippy clean with `--deny warnings`, and every makespan
pin in `red_science.rs`, `scheduling.rs`, `placement_occupancy.rs`,
`refusal_memory.rs`, `tile_capacity.rs`, `tile_occupancy.rs`,
`tile_reservation.rs`, `split_capacity.rs` and `smelt_roots.rs` untouched.

### Mutation evidence

Each applied to the fixed `crates/planner/src/state.rs` alone, `state::tests::`
run, tree restored between each.

| Mutation | Failed |
|---|---|
| `entity_type == "furnace"` instead of `"mining-drill"` | `a_mining_drill_may_stand_on_the_ore_it_mines`, `an_unknown_entity_gets_no_exemption`, **`ore_still_blocks_everything_that_is_not_a_drill`** |
| `None => true` — an unknown prototype is exempted | `an_unknown_entity_gets_no_exemption` |
| `!stands_on_resources(name)` → `true` — nobody exempted (the original bug) | `a_mining_drill_may_stand_on_the_ore_it_mines` |
| `if resource_blocks &&` → `if false &&` — ore stops being occupancy at all | **`ore_still_blocks_everything_that_is_not_a_drill`**, `a_tile_holding_ore_is_not_free` |
| `is_area_clear` passes `resource_blocks = false` — the tile question gets the exemption | the same two |
| the exemption also suppresses water for a drill | **`the_drills_exemption_is_ore_and_only_ore`** |

The three bold rows are the answer to "does the exemption weaken resource
occupancy for anything else". Widen it to furnaces, to every entity, to the
tile-granularity question, or to water, and a control goes red for each. That is
what makes the two controls worth having despite never being able to fail from
the fix itself.

---

## One pre-existing failure, not mine

`factorio-bot-scripting-lua`'s
`globals::goal::tests::a_research_that_cannot_reach_the_water_raises_a_recognisable_refusal`
fails on `master` at `b3d4b343`, before either change here. Confirmed by
restoring both of my files to `HEAD` and running that single test:

```
panicked at crates/scripting_lua/src/globals/goal/mod.rs:2232:9:
a plant that cannot be sited cannot be planned; the call must not succeed
```

It expects a refusal for a plant sited too far from water, and `e3ea04fe`
("a distant power plant is a longer walk, not a refusal") made that no longer a
refusal. The test's expectation is stale, not the code. `globals/goal/mod.rs` is
outside this task's boundary; flagged, not touched.

## Not done, on purpose

`holds(Producing)` still returns `None`. The spec's D0 says it must stop doing
so *in the same change that gives `Producing` a method* — and that method is the
next task. Blocker 1 is the half that had to land first: with it, an
unanswerable goal is a clean halt with a reason instead of a silent success,
whatever `Producing` grows into.
