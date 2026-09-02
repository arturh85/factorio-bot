# A lost action could not be written down

`workspace/runs/run-1788347034-00981` recorded **179 `action_dispatched` lines
and 170 `action_settled` lines**. The nine missing settles were not dropped by
accident. They were unwritable: `record.actions` keyed the settle on a
*measured reply tick*, and an action the run lost track of never has one — that
is what "lost" means. The one outcome the record most needed to show was the
one outcome it structurally could not.

The consequence is worse than a gap. Milestone 7 reads as an unbroken run of
successes while the supervisor's own counters said an action was lost on every
single iteration. A reader trusting the record looks in the wrong place.

## 1. Verifying the account

Checked against the run's own files, not taken on trust.

**Confirmed.**

- Event kinds in `run-1788347034-00981/events.jsonl`: `action_dispatched` 179,
  `action_settled` 170, `plan_created` 14, `milestone_started` 7,
  `milestone_satisfied` 6, `teleport` 2, `run_started` 1, `placement_refused` 1.
  Nine more dispatches than settles.
- Exactly one settle is not `success`: id 22, bot 4, tick 11531,
  `can_place_entity said 'no'` — milestone 4, not milestone 7.
- Segmenting by `plan_created` and matching dispatch to settle by
  `(plan, id)`, the nine unsettled actions are:

  | plan | milestone | id | bot | action |
  |---|---|---|---|---|
  | 8  | 6 | 7  | 1 | craft 1 stone-furnace |
  | 9  | 6 | 0,1,2,3 | 1,2,3,4 | craft N automation-science-pack |
  | 10 | 7 | 13 | 1 | craft 1 stone-furnace |
  | 11 | 7 | 21 | 1 | craft 1 stone-furnace |
  | 12 | 7 | 12 | 1 | craft 1 stone-furnace |
  | 13 | 7 | 19 | 1 | craft 1 stone-furnace |

  **Every one is a `craft`.** Every one has a dispatch and nothing after it.
- The three quoted supervisor lines match three specific plans exactly, which
  is what makes the join safe to reason from:
  - `success=18 … pending=56 (+37 events)` → plan 11: 75 steps, 19 dispatched,
    18 settled. 19 + 18 = 37; 75 − 19 = 56.
  - `success=0 … pending=70 (+1 events)` → plan 12: 71 steps, 1 dispatched,
    0 settled. 71 − 1 = 70.
  - `success=14 … pending=55 (+29 events)` → plan 13: 70 steps, 15 dispatched,
    14 settled. 15 + 14 = 29.

**One correction to the reading that opened this.** *"An iteration reporting
`failed=1 lost=1` contributed +1 events … so the failed action produced a
`action_dispatched` and no `action_settled`, and the lost one produced
nothing."* That reads the line as naming **two** actions. It names one.

`scripts/supervisor.lua` builds the value `research_run.lua` prints under
`failed=` as a **sum**:

```lua
local failed = (obs.failed or 0) + (obs.lost or 0)
    + (obs.walks_failed or 0) + (obs.walks_lost or 0)
```

So `failed=1 lost=1` means `obs.lost == 1` and everything else zero — one lost
action, counted once and printed twice under two different labels. Nothing
failed anywhere in milestone 7. That also removes a hypothesis worth naming:
there was no invisible `Status::Failed` action in this run. The
`Dispatch::NotDispatched` path (`ActionTicks::UNKNOWN`, so neither tick) is
real and was equally unrecordable, but it is not what this run hit.

The `failed=` label is therefore a second, smaller defect, and it is the one
that misled the reading above. It is **not fixed here**: `scripts/research_run.lua`
owns the format string, `t.failed` has that summed meaning pinned by
`crates/scripting_lua/src/supervisor_lib.rs`, and both are outside this change's
boundary. Reported rather than expanded.

## 2. Where the action escaped

Not in the executor. `crates/executor/src/run.rs` already reaches a terminal
status for everything it dispatches: `Ok` → `succeed`, `Err` →
`lose_track` when `ActuatorError::NoVerdict`, `fail` otherwise, and
`lose_track_of_outstanding` sweeps anything still `Running` when the run
unwinds. `Status::Lost` was correct, present, and counted.

The escape is one line down, at the seam where the log becomes the record —
`crates/scripting_lua/src/globals/record.rs`, `record.actions`:

```rust
if let Some(replied) = replied {          // <- the hole
    …EventKind::ActionSettled { … }
}
```

`replied` is `Attempt::replied_tick`, a **measurement**: `game.tick` as the game
reported it, correctly `None` when the game said nothing. So the gate reads
"write the verdict once the game has timed it" — and for a lost action the game
never will. A `craft` that outlived `ACTION_RESULT_DEADLINE` therefore produced
a dispatch (it *was* acknowledged, so it has a dispatch tick) and could never
produce a settle, no matter how many times it happened.

The fix inverts what the settle is keyed on. It is keyed on the **verdict**:

- `success` / `failed` / `lost` → exactly one `action_settled`, always.
- `pending` / `running` → nothing. No verdict, nothing to report. This is the
  new failure mode to guard, and
  `an_action_with_no_verdict_yet_writes_nothing` is the control that guards it.

When the game supplied no reply tick, the settle is stamped with
`RunRecorder::not_before(dispatched)` — the record's own high-water mark, i.e.
"no earlier than everything already written, including this action's own
dispatch". What is never done is reusing the dispatch tick, which would report
a real reply at a real instant with a duration of zero. `elapsed_ticks` is
`Some` only when **both** ends were measured, so a synthesized stamp always
carries a null duration and can never be mistaken for a timed span. That null
is also how a reader tells the two stamps apart.

One shape is now possible that was not before: an `action_settled` with **no**
`action_dispatched` beside it. That is a finding, not a gap — the action reached
a verdict before the game ever acknowledged a dispatch, so there is no dispatch
to record and inventing one would be a fabrication in the opposite direction.

## 3. `Lost` stays distinct from `Failed`

Three ways, none of which collapse:

- **`status`** carries the word itself. `status_name` in
  `crates/scripting_lua/src/globals/goal/run.rs` has always produced
  `"lost"` as its own spelling, distinct from `"failed"`; until now nothing
  downstream could ever see it, because the line carrying it was not written.
- **`error`** carries why the outcome is unknown, which is not a verdict.
  `ExecutionLog::lose_track` deliberately reuses the same field a failure uses
  and lets `status` be the only discriminator, so a reader never has to know
  which of two message fields to look in.
- **`failure.kind`** classifies the text. A lost action's `no action result
  received in time` classifies as `FailureKind::Timeout`, never `Rejected`,
  which is what a rejection gets.

`derive_lanes` now closes a lost lane as `lost`. Before this it left the lane
*open*, which is a third meaning again — an open lane says the run was
interrupted mid-dispatch — so all nine lost crafts were drawn as if the process
had died on each of them.

## 4. The same shape, a third time — and it was on actions

Twice today: `milestone_stuck` recorded `last_error: null` while an error was
being logged every iteration, fixed by adding `obs.walks_failed`; then that fix
missed `Status::Lost` walks, fixed by adding `obs.walks_lost`. Both about walks.
Actions were assumed covered.

They were not. In `build_observation`, `first_error` is built from `failures`,
and `failures` holds `Status::Failed` **only** — on purpose, and the test
`an_action_the_game_gave_no_verdict_for_is_reported_as_lost` pins it: losing the
thread is not a failure. But that left a lost action contributing *nothing* to
`first_error` either. In this run, every milestone-7 iteration had
`obs.failed == 0`, `obs.lost == 1`, and no failing walk — so `first_error` was
`nil` on every one, and if milestone 7 had halted, `milestone_stuck` would have
written `last_error: null` again, **after both walk fixes**.

`failures()` is left alone. A `first_lost_error` fallback is added instead, and
the chain is ordered most-specific-first:

```
failed action  ->  lost action  ->  failed-or-lost walk
```

Each term is a fallback, not a replacement, so every existing reading of
`first_error` is unchanged.

## 5. Also checked: `teleport.action_id` cannot be joined to
`action_dispatched.id`

Reported rather than fixed; it is not a one-line change and the shape of the
answer is not a rename.

The two fields name different things and look far more alike than they are.
`ActionDispatched::id` is the planner's `ActionId`, plan-local, restarting at 0
with every plan. `Teleport::action_id` is the **mod's** action id, minted by
`FactorioRcon` from `FactorioWorld::next_action_id`, run-global and wrapping at
1000 (`*next_action_id = (*next_action_id + 1) % 1000`). Joining them produces
confident nonsense.

Unifying them would not answer the question anyway. A `walk_stuck` teleport
belongs to a **walk leg**, and a walk is a `StepKind::Walk` with no `ActionId`
at all — that is why `build_observation` keys `obs.walks` by
`(bot, step_index)` instead. So "which action was this bot walking for" has no
answer in this id space regardless of what the fields are called.

Answering it needs two things this record does not have:

1. **Walks in the record.** There is no walk `EventKind`, so `obs.walks` — the
   ticks, destinations and per-walk statuses the executor already measures —
   never reaches `events.jsonl`. Walking is most of the wall-clock in these
   plans, and none of it is in the run record.
2. **The mod's action id carried back out** of `FactorioRcon::move_player` and
   attached to the walk it belonged to, so the teleport has something real to
   point at.

Neither is small. Both are worth doing, and (1) is the larger hole of the two.

## 6. Tests

Four regression tests, all verified failing before the fix:

- `a_lost_action_settles_even_though_the_game_never_replied` — was
  `written: 1, expected 2`.
- `a_failure_the_game_never_stamped_a_tick_for_still_settles` — was
  `written: 0, expected 1`.
- `every_dispatched_action_settles_exactly_once` — settled
  `[(0, success), (1, failed)]`, missing `(2, lost)` entirely.
- `an_action_the_run_lost_track_of_still_reports_its_error` — `first_error` was
  `nil`.

Plus one control that passed both before and after —
`an_action_with_no_verdict_yet_writes_nothing`, so the new status gate cannot
start inventing verdicts — and
`a_lost_action_closes_its_lane_as_lost_and_not_as_failed` in
`crates/core/src/record/lanes.rs`.

## 7. One thing deliberately not done

The account in §2/§3 belongs on `EventKind::ActionSettled` itself, but it is
written there as a plain `//` comment rather than a `///` doc comment. utoipa
copies doc comments into the published OpenAPI schema and
`app/src/api/openapi.snapshot.json` pins them, so documenting this variant would
require regenerating a file in the frontend's tree. None of it is a fact a
schema consumer needs. Worth knowing before someone "fixes" the comment style.
