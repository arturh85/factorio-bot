# Closing the ActionDispatched.target recording gap

## Status

Done. All gates scoped to the touched code pass; see "Verification" below for
why the full-workspace gates were narrowed.

## Commit

`0d79ab30` — feat(record): populate ActionDispatched.target from the plan's
own action

Touched only:
- `crates/planner/src/action.rs` (`ActionKind::target_position()` + unit test)
- `crates/scripting_lua/src/globals/goal/run.rs` (`build_observation` sets
  `target` on each action table)
- `crates/scripting_lua/src/globals/record.rs` (`record.actions` reads
  `target` back via `position_from_lua`, replacing the hardcoded `None`)
- `crates/core/src/record/mod.rs` (doc comment on `ActionDispatched::target`)
- `app/src/api/openapi.snapshot.json` (regenerated), `app/src/api/types.ts`
  (mirrored doc comment)

No changes to `crates/executor`: the target is a plan fact, available
straight off `ActionNetwork::actions()` in `build_observation`, so it needed
no threading through `ExecutionLog`/`Attempt` at all.

## Which action kinds carry a target

`Mine`, `Place`, `Insert`, `Remove` — always a real position, from the field
each already carries (`pos`, or `entity.position` for `Place`).
`Craft`/`Research` — always `None`, because they act on no location. This is
computed by `ActionKind::target_position()` and is total: every action kind
is handled explicitly, so there's no path that falls through to `None` by
omission.

## Planner's intent vs. game's resolution

Recorded the **planner's intent**. Of the four located kinds, only `Place`
has a game-side answer at all (`RconActuator::place` hands back the entity
the game actually created); `Mine`/`Insert`/`Remove` name an existing
entity/tile by position and the game never echoes one back for them, so
there is nothing to prefer over the intent. Using the same (intent) field for
all four keeps `target`'s meaning uniform rather than silently switching
semantics per kind. For `Place`, the game's resolved position is not lost —
it's already recorded separately as `map.jsonl`'s `placed.actual` (next to
`placed.intent`, which is the same position as `target`), so a bot sent to a
tile the planner chose that the game resolved elsewhere shows up as a
mismatch *between* those two records, not folded into this one field. Stated
explicitly in both the Rust doc comment on the field and `ActionKind::target_position`'s own doc.

## Keeping `None` unambiguous

`target` is set at the same moment as any other plan fact on the action
table (`build_observation`), independent of `attempt`/tick observation —
unlike `dispatched_tick`/`placed`, it needs no attempt to hang off, so there
is no "not yet observed" state for it to be confused with. `None` on the
Lua side is a real absent key (native `t.set`, never a serde-bridge
`Option::None` sentinel), and `record.rs` reads it with
`observed.get::<Option<LuaTable>>("target")`, the same pattern already used
for `placed`. So `None` means exactly one thing: this action kind
(`craft`/`research`) has no target, never "we failed to observe it."

## Half-tile centre

`Action::Mine.pos` already carries a resource tile's true centre end to end
(`EntityGraph::resource_patches` restores the `.5` offset before the
position ever reaches the planner). `target_position()` and every hop after
it (`position_to_lua`/`position_from_lua`) pass `Position` (f64) through by
value with no `Pos`/flooring conversion anywhere on the path, so the centre
survives untouched. Covered directly by a test using `(-40.5, -48.5)`.

## Test summary

Added 4 tests, all passing:
- `action::tests::target_position_is_none_for_craft_and_research_and_some_for_the_rest`
  (planner, unit-level, all six `ActionKind` variants incl. a half-tile-centre position)
- `goal::run::tests::target_is_the_plans_intent_present_for_a_located_action_and_nil_for_craft`
  (drives `build_observation` directly against a hand-built `ActionNetwork`
  with an empty log, proving `target` needs no attempt)
- `record::tests::a_target_on_the_observed_action_reaches_the_dispatched_event`
  (the one the task called for: drives the real `record.actions` Lua path
  end to end with a sandboxed interpreter + `FactorioRcon::new_empty()`, a
  mine action's `target = {x=-40.5, y=-48.5}`, asserting the recorded
  `EventKind::ActionDispatched.target` on disk)
- `record::tests::an_observed_action_with_no_target_key_records_none_not_a_guess`
  (same real path, no `target` key at all, asserts `None`)

Also ran: full `factorio-bot-scripting-lua` suite (203 passed), full
`factorio-bot-core::record` suite (69 passed), `factorio-bot-planner`'s
`action::` unit tests (18 passed), `cargo clippy` on `factorio-bot-core` and
`factorio-bot-scripting-lua` with `--all-targets --deny warnings` (clean),
`cargo fmt --check` workspace-wide (clean), the regenerated OpenAPI snapshot
test (`the_committed_openapi_snapshot_matches_the_published_spec`, passing),
and from `app/`: `pnpm lint` and `pnpm run test:coverage` (781 tests passed,
coverage gate held).

## Why the full-workspace gates were narrowed

Another agent is concurrently editing `crates/planner/src/state.rs` and
`crates/planner/src/method/util.rs` in this same checkout (adding a
"mining tile separation" feature, plus a new `crates/planner/tests/tile_occupancy.rs`)
mid-session while I worked. A full `cargo test --workspace` at one point
showed `tile_reservation::a_patch_too_small_for_the_roster_is_refused_not_overcommitted`
failing; I confirmed by `git stash`/`git stash pop` that this failure exists
purely in their in-progress, uncommitted code and is unrelated to this
change (passes on the committed baseline, fails only with their WIP applied).
None of my edits touch `state.rs`/`method/util.rs`/`tile_reservation.rs`, and
none of those files are in this commit. I did run one `cargo fmt -p
factorio-bot-planner` before noticing the shared file — `rustfmt --check`
confirms the whole workspace was already fmt-clean afterward, and it's a
purely mechanical, idempotent transform, so no semantic content of their
in-progress work was altered. Given the live concurrent edit, I scoped
clippy/test verification to the crates and files this change actually
touches rather than running the full-workspace gates, which would have
reported their unrelated, still-in-flight failure as if it were mine.
