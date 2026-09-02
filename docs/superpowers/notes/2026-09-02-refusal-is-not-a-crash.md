# A refusal is a verdict, not a crash — 2026-09-02

**Run:** 31, `workspace/runs/run-1788372605-35170/`.
**Files:** `crates/scripting_lua/src/globals/goal/mod.rs`,
`crates/scripting_lua/src/globals/goal/plan.rs`, `scripts/supervisor.lua`,
`scripts/research_run.lua`, `crates/core/src/record/splits.rs`.

## What happened

Run 31 satisfied six rungs in twelve minutes — the best result so far — and
then asked to research automation in a world whose plan could show no electric
supply at all. The planner refused:

```
RAISED: runtime error: goal: automation needs a lab with 60 kW of electric
        supply, and the plan can show only 0 kW
RUN FINISHED state=crashed id=run-1788372605-35170
```

The refusal is correct: no plan in any archived run has ever placed a
generator, and `generated_kw` was `0.0` in all 541 of run 30's force samples.
It is also the most informative failure this planner has produced.

**And it destroyed the record of itself.** `Sup:step()` raised out of the
`planning` branch, so `_close` never ran, so no history entry was made, so
`sup:report()` printed milestones 1–6 and **no `milestone 7` line at all**.
`splits.json` did get a milestone 7 — `scripts/research_run.lua`'s outer
`pcall` records one — with `outcome: "plan_error"`, a word outside the
documented vocabulary, and `state=crashed` on the manifest. A reader of the
best run so far could not tell what happened to the milestone it died on.

## The distinction

A planner **refusing to plan something impossible is a verdict about the
world**. Nothing is broken, nothing is retryable, and the honest thing for a
loop spanning several goals to do is record the reason and finish. A **fault**
— a method contradicting itself, a network it could not have built correctly, a
name that refers to nothing — must still end the run loudly, because a run that
carries on past one reports a world condition for what is really a bug.

A catch-all around `goal.plan` would erase that difference, so the
classification is made variant by variant, in Rust, where the type is
(`refusal_for`, `goal/mod.rs`). The match has **no wildcard arm**: a new
`PlannerError` variant fails the build rather than defaulting into either
bucket. Defaulting to "verdict" would let a new bug be recorded as a world
condition; defaulting to "fault" puts the next `ResearchNeedsPower` back on the
path that destroyed run 31's record.

### Verdicts (7)

| variant | why |
| --- | --- |
| `NoApplicableMethod` | "this world offers no route to that" — a reading of the map. |
| `NoRoomToWork` | "nowhere to stand while doing it", with the number of bots that could have shared it. |
| `ResearchNeedsPower` | a lab with no power researches nothing at all; the planner states the gap rather than costing it at zero. |
| `UnsupportedResearchTrigger` | the same shape: a trigger this planner cannot express, named. |
| `SelfUnlockingResearchTrigger` | its own help text says it — "cannot be reached from the current world state". |
| `PreconditionUnsatisfied` | `crates/planner`'s own doc calls it "the world was not as planned"; the planner's `two_bots_cannot_mine_the_same_exhausted_tile` shows what it carries — an exhausted ore tile, named by the condition. |
| `ChainOwnerInfeasible` | the same rejection with blame placed on the chain's owner; the `condition` is still a fact about the world that does not hold. Splitting the pair would be arbitrary. |

### Faults (9)

| variant | why |
| --- | --- |
| `InsufficientItems` | only reachable by *applying* an effect, which both `schedule` and the method driver do only after judging the same action feasible — so it means the feasibility check and the effect disagree. A world that is genuinely short reports `NoApplicableMethod` or `PreconditionUnsatisfied`. |
| `UnknownBot`, `NoBots` | the roster and the state disagree, or there is no roster. `goal.plan` refuses both before the planner is reached, so arriving here is a contract violation, not a shortage. |
| `CyclicNetwork`, `Deadlock` | a network whose edges cannot be run. No world makes that true or false. |
| `ChainConflict`, `UnownedHandover` | both say so in their own doc comments: "not about the world at all"; "a mistake in the method that wrote the step". |
| `ExpansionTooDeep` | the depth guard, on a method expanding into itself. |
| `UnknownTechnology` | **the borderline one.** The name refers to nothing, so no state of the world makes the goal meaningful — a typo, or a world that was never told about the technology. Both are broken inputs. `supervisor.lua` has called "an unknown item or technology" a construction error since it was written, and this keeps that promise. |

`PreconditionUnsatisfied` and `ChainOwnerInfeasible` are the two worth
revisiting if this proves wrong: they are the variants most likely to be
produced by a *method* defect while wearing a world condition's clothes. They
are recorded with their condition either way, so the cost of being wrong is a
milestone blamed on the world with the planner's own sentence attached, not
silence.

## The mechanism, and why it is a raise and not a return

`goal.plan` still **raises** a refusal. Returning one — an empty `PlanValue`
with a `refusal` field — would put a refusal one `#plan.steps == 0` away from
being read as "nothing left to do", which is the exact mistake `goal.holds` was
added to stop this loop making.

What is new is that the raise is *recognisable*. A verdict is raised as
`mlua::Error::external(PlanRefusal { code, message })`; `goal.refusal(err)`
hands back `{ code = "planner::research_needs_power", message = ... }` for one
and `nil` for everything else. The code is the planner's own miette diagnostic
code, so nothing matches on message text. `Display` keeps the `goal: ` prefix
every other error on this surface carries, so the raise reads exactly as it did
before; the carried `message` is unprefixed, so a report line can put its own
word in front of it.

Both places that call the planner go through one seam (`planner_error`), so a
refusal cannot reach Lua marked by one path and unmarked by the other.

**`nil` is the safe answer, and everything unclassifiable gets it.** No `goal`
table, no `goal.refusal` on it, a classifier that itself raised, an answer that
is not a table — all read as *fault*: loud, out of the loop. The opposite
default would let an old or stubbed binding turn every planner bug into a
quietly recorded world condition. This is the same rule `goal.plan`'s placement
pre-check follows: an absent checker must never look like a green answer.

## What the milestone line reads now

```
   HALTED: stuck -- refused: automation needs a lab with 60 kW of electric supply, and the plan can show only 0 kW
RUN FINISHED state=stuck id=run-...
supervisor: stuck
  milestone 1: satisfied after 1 iteration(s), best 12 steps
  ...
  milestone 7: stuck after 0 iteration(s), best - steps, refused: automation needs a lab with 60 kW of electric supply, and the plan can show only 0 kW
```

`after 0 iteration(s), best - steps` is the honest shape of a milestone nothing
ever ran for.

Three deliberate choices in that line:

- **`stuck`, not a new word.** The outcome vocabulary
  (`crates/core/src/record/splits.rs`) says *how far a milestone got*, and "as
  far as this world allows" is `stuck`. Not `stuck_silent` — that one means "no
  progress and nothing to show for it", and a refusal has the planner's reason
  to show. What *kind* of stuck it was is `h.refusal`, which is data the event
  already carries; a seventh word would also be a word the viewer has no style
  for and nothing consumes.
- **`refused:`, not `last error:`.** No action was dispatched and nothing
  failed. Calling the refusal an error would invent a failure that never
  happened — the same defect as the `failed` counter that used to carry a sum
  of four.
- **`first_error` is untouched.** It means the first failed *action*'s own
  text. A milestone can have both a failed run and a later refusal, and they
  are different facts; collapsing them would lose whichever was written second.
  `research_run.lua` records `t.refusal.message` when present and
  `sup.first_error` otherwise, so the record carries the reason that actually
  closed the milestone.

Not retried, for a reason of its own: re-planning asks the same question of the
same world — nothing ran, so nothing changed — and gets the same answer, until
the iteration cap turns a stated reason into `exhausted`.

## Evidence

Red first, with the actual failure:

- `a_research_goal_with_no_power_raises_a_recognisable_refusal` builds the
  research force on the shared fixture world, plans `researched("automation")`
  through the real bindings, and gets run 31's own sentence. Red as
  `attempt to call a nil value (field 'refusal')`.
- `a_refusal_closes_the_milestone_instead_of_killing_the_run` drove the real
  `supervisor.lua` and went red with the raise propagating straight out of the
  loop — the same shape as run 31.

Mutations, each failing its own tests and nothing unrelated:

| mutation | fails |
| --- | --- |
| `ResearchNeedsPower` moved to the fault arm | `a_research_goal_with_no_power_raises_a_recognisable_refusal` |
| `UnknownTechnology` moved to the verdict arm | `a_technology_no_force_defines_is_a_fault_carrying_no_refusal` |
| `refusal_of` returns a table for every raise (the catch-all) | 7, including all three "a fault still ends the run" controls and `research_run_lib`'s `a_raise_during_planning_is_recorded_with_the_callers_error_text` |
| report says `last error:` for a refusal | 2 refusal-report tests |
| the refusal is written into `first_error` | `a_refusal_reaches_the_driver_...`, `a_refusal_after_a_failed_run_keeps_both_reasons_apart` |
| closed as `stuck_silent` | the two tests that read the outcome |
| the transition drops `refusal` | `a_refusal_reaches_the_driver_...` |

The catch-all mutation is the one that matters: the two negative controls
(`a_raise_the_classifier_does_not_vouch_for_still_ends_the_run`,
`a_goal_table_with_no_classifier_treats_a_raise_as_a_fault`) pass both before
and after this change by construction — they pin behaviour that must *not*
move — so their value is only demonstrated by going red under exactly the
mutation they exist to catch. They do.

## What is still open

- `Split::outcome`'s doc lists five words and a real run has already produced a
  sixth, `plan_error`, from the driver's fault path. Correcting that doc changes
  the OpenAPI snapshot, which lives in `app/src/api/` — owned elsewhere today —
  so the vocabulary is explained in the module doc instead and a test pins that
  an unknown outcome reaches the split unchanged.
- The planner still cannot build a generator, which is the actual reason rung 7
  cannot be reached. This change makes the refusal survivable; it does not make
  the research possible.
