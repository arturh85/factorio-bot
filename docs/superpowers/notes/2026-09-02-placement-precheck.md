# Ask the game before committing a plan to a site — 2026-09-02

## Status

Done. All gates green. Two commits (`d0db355c` the change, `24a211ee` an
unrelated pre-existing clippy failure that made the gate red for anyone in this
tree); the note itself is not committed.

## Is this the right move?

Yes, and the case for it is stronger than the brief's framing, because the
pre-check turns out to answer a question the dispatch-time refusal *cannot*.

The brief offered an alternative: the four causes were all model gaps, so a
fifth model source might be cheaper than a query. That is a real argument and
it was the right call four times. It is now the wrong one, for a reason the
runs themselves make: **each of the four model sources was found by forensics
on a refusal that named no cause.** A forest, a non-roster character, a roster
character, one ulp — each cost a run, a note, and a session of reading
`events.jsonl` backwards. Adding a fifth model source requires already knowing
what the fifth cause is, and nothing in the record says.

`can_place_entity` has room for an answer that the refusal message does not.
Asking it *before* dispatch means the mod still has the collision box in hand,
so it can run `find_entities_filtered` over exactly the box the game just
tested and name what is in it, plus the tile underneath. That lands in the
record as `blockers` / `tile`. So this is not only "refusals get cheaper"; it
is "the next unknown cause identifies itself instead of costing a run".

The prerequisite the refusal-memory author named is genuinely load-bearing. The
loop only converges because a refused site is written somewhere that survives
the next `PlanState::from_world`. Without it, re-expansion re-derives the same
site from the same inputs and the pre-check buys an RCON round trip per
iteration and nothing else.

## 1. Where the check lives

**In `goal.plan`, between scheduling and returning the plan**
(`plan_verified`, `crates/scripting_lua/src/globals/goal/plan.rs`).

The sequence is: expand (pure) → schedule (pure) → **ask** → write what the
game said into `FactorioWorld::placement_refusals` → expand again (pure, now
reading a world with one more fact in it). Every expansion is still a pure
function of a world snapshot and a roster, which is exactly what
`expansion_is_deterministic` pins. What changed between two of them is the
world, and a changing world is what `PlanState::from_world` is for.

The seam is a new `PlacementChecker` — a `Fn(Vec<PlacementQuery>) -> Future<…>`
built from RCON in `create_lua_goal`, `None` when there is no game. `None` is
the path every pre-existing test takes and the path a headless build takes, so
"no checker" is not a special planner.

**What I rejected:**

1. **The executor, immediately before dispatch.** Cheap and obvious, and it
   buys almost nothing. The plan is already committed and its dependents are
   already scheduled; converting "the game refused this" into "the game would
   have refused this" one round trip earlier still fails the action, still
   abandons its dependents, still escalates recovery. The saving is one RCON
   call. The brief's own bar — *a plan that would have been refused is not
   dispatched* — is not met, because by then it already was.
2. **A pre-planning region sweep.** `can_place_entity` is per-prototype: the
   collision box, direction and force are all part of the question, so "is this
   region clear" is not a thing you can ask. You would have to sweep per entity
   type over `free_area_near`'s 625-candidate window, i.e. hundreds of queries
   for candidates no plan will use, most of them stale before anything runs.
   Unbounded cost, speculative value.
3. **The supervisor, in Lua, between `goal.plan` and `goal.run`.** Visible in
   the script, which is a real virtue. But `obs:recover()`'s tier-2
   re-expansion builds and runs plans in Rust and would bypass it entirely,
   every driver would have to opt in, and the refusal list would have to be
   shuttled out through the observation surface and back. This is the same
   argument the refusal-memory note used against threading refusals through
   Lua, and it holds for the same reason: put the fact where the fact belongs
   and no call site changes.
4. **Filtering the refused steps out of the schedule at dispatch** (the design
   I got two thirds of the way through before backing out). `Replay::new`
   derives each walk's `bot_step_index` by counting that bot's steps *in
   schedule order*, and `run_bot_signalled` derives the same index the same
   way. Dropping a step from the schedule the executor runs, while showing the
   unfiltered one in the replay, silently shifts every later walk row onto the
   wrong observation. Catching this before writing it is most of why the check
   ended up in `goal.plan` instead.

## 2. What is asked, and how the API was verified

`surface.can_place_entity{ name, position, direction, force,
build_check_type = defines.build_check_type.manual }` — **the same argument
table the real placement uses**, built by one shared helper
(`placement_check_args`, `mods/BotBridge/control.lua`) that both call sites go
through. A pre-check asking a different question than the placement is worse
than no pre-check, because its green would be believed.

Verified by reading `workspace/factorio-api-docs/runtime-api.json`
(`application_version` 2.1.17, runtime `api_version` 6), not from memory:

- `LuaSurface.can_place_entity` takes `{name, position, direction, force,
  build_check_type, forced, inner_name}`, `takes_table: true`.
- **`build_check_type` is optional and defaults to `ghost_revive`, not
  `manual`.** This is the documented trap in its local form. `only_ghosts =
  true` validates nothing because ghosts do not collide; defaulting to a
  ghost-flavoured build check is the same mistake with the same shape — a call
  that looks like it validated a build and did not. `manual` is what a player
  building by hand runs, which is what a bot placing an entity is.
- `forced` is read **only** for `manual_ghost` / `script_ghost` /
  `blueprint_ghost`, so it is irrelevant here and is not passed.
- There is **no** `force_build` and **no** `build_mode` parameter on this
  method at all. The 2.0 rename the API audit found belongs to blueprint
  building, not to this call, so the audit's finding does not apply — checked
  rather than assumed, since assuming it did would have meant passing a
  parameter the game ignores.

`force` is the acting player's force (the default is `"neutral"`), which is why
each query carries the player id the schedule assigned — a `BotId` *is* a
player id, and this does not become a mapping layer.

The guarantee is pinned by
`the_pre_check_asks_can_place_entity_exactly_what_a_real_placement_asks`, which
loads the repo's own `control.lua` into a real Lua 5.4 interpreter, drives both
handlers against a stub that records **every argument table
`can_place_entity` was handed**, and compares them. Changing
`build_check_type` to `ghost_revive` in the mod makes it fail, which was
checked.

## 3. How many questions

**One round trip per plan.** Every placement in the plan goes out in a single
`remote.call('botbridge', 'can_place_entities', {...})` and comes back as one
JSON document, joined to the queries by index.

- A plan whose sites are all legal: **1** call. This is the overwhelmingly
  common case and the one the budget is sized for.
- A plan with no `Place` at all: **0** calls — the query list is empty and no
  call is made.
- A plan needing re-siting: 1 call per expansion, capped at
  `MAX_RESITE_ROUNDS + 1 = 3` calls and 3 expansions.

Run 19's rung-4 plans were 103 steps; a plan that size makes well over a
hundred RCON round trips as it executes, so one more before it starts is under
1%. The questions are the *plan's chosen sites*, deduplicated — not one per
candidate `free_area_near` considered, which is the version that would have
been dozens.

Exhausting the budget is deliberately **not** an error. The plan is returned as
it stands, the dispatch-time refusal path catches it exactly as it did before
this existed, and every site the game turned down along the way is on record
either way — the last round still asks, it just does not re-expand, so the next
`goal.plan` starts from what this one learned. A pre-check must never be able to
fail a run that would otherwise merely have stumbled.

## 4. What staleness remains

**A green pre-check is what the game said at the tick it was asked. It is not a
promise about the tick the build happens.** The window narrows from "whenever
the plan reaches this step" to "between the plan being returned and that step
being dispatched", and it does not close. Still possible:

1. **A bot walks into the footprint.** The executor's own walk-aside retry
   handles the *acting* bot (`§player_blocks_placement§`); another bot in the
   footprint at dispatch time is still an ordinary refusal.
2. **A biter, a spawner expansion, or anything else the game moves.**
3. **An earlier step of the same plan changes the ground.** Every site is
   checked against the world as it is *before* any of the plan has run. Today
   the planner does not site a build on ground it plans to clear, so this is a
   possibility rather than an observed case — but it is the one that could
   produce a false **red**, which is the direction that matters: a false red
   enters a never-expired ledger. It is bounded by the refused box and by the
   fact that only sites the game itself turned down ever enter, but it is not
   impossible, and it is the first thing to suspect if the planner is ever seen
   avoiding ground nothing is wrong with.
4. **The check is skipped entirely** when there is no RCON, when the mod is
   older than this binary (`can_place_entities` is a new remote function), or
   when the call fails. All three log a warning and plan exactly as before.

The one thing that is *not* stale is a refusal: it goes into the ledger and is
believed for the life of the run, for the reasons argued in
`2026-09-02-refusal-memory.md`.

**A character in the footprint is deliberately not learned**, and this is the
one place the pre-check and the dispatch path deliberately differ in reach. A
character moves on its own, `PlanState::from_world` re-reads every character
from the world on every plan, and at pre-check time the acting bot has not
walked to the site yet — so a character standing there now says nothing about
whether the ground is buildable when the plan arrives. Remembering it would
fence the planner off ground that is fine; re-siting on it would reopen exactly
the loop this closes, with the planner fleeing one tile per iteration against
the supervisor's stall limit of 3. This is the mod's own
`§player_blocks_placement§` distinction, widened from the acting player to every
character because at plan time there is no acting player standing anywhere yet.

## 5. How it appears in the record

`EventKind::PlacementRefused` gains three fields:

```jsonl
{"tick":6198,"kind":"placement_refused","entity":"stone-furnace",
 "position":{"x":-16.0,"y":-58.0},"source":"pre_check",
 "blockers":["tree-01","tree-02"],"tile":"grass-3"}
```

- **`source`** is `"dispatch"` or `"pre_check"`, and the two must not be
  confused. A `dispatch` refusal has an `action_settled` failure beside it at
  about the same tick. A `pre_check` refusal has **none**, because no action for
  that site was ever created — and a reader who could not tell them apart would
  go looking for a line that is absent by design.
- **`blockers`** names the distinct entities the game found in the box it just
  tested, sorted. Always empty for `dispatch`, which has no way to ask. Empty
  for `pre_check` means something different and is genuinely informative: no
  entity intersected the footprint at all, so the ground itself is the answer —
  which is what `tile` is for.
- **`tile`** is the tile under the refused centre; `null` for `dispatch`.

Both refusal kinds are written by the same `record.refusals()` at the same
cadence (once per `ran` transition in `scripts/research_run.lua`), so nothing in
the driver changed. The console gets one `tracing::warn` per newly-learned site
naming the entity, the position, and the cause; repeats are silent because the
ledger dedupes. `goal.plan` logs one `info` per re-siting round and one `warn`
if the budget runs out.

Snapshot regenerated (`UPDATE_OPENAPI_SNAPSHOT=1 …`, +21 lines), mirrored in
`app/src/api/types.ts` and `app/src/api/openapi.contract.spec.ts`. Nothing in
the frontend switches exhaustively over `EventKind`, so no renderer changed.

## What else changed, and what did not

- **`goal.plan` is now an async Lua function.** It has to be: it awaits an RCON
  round trip. Scripts are already executed with `exec_async`, so no script
  changes; seven of `plan.rs`'s tests moved from `.exec()` to
  `#[tokio::test]` + `.exec_async().await`. Nothing else about the binding
  moved — same arguments, same `PlanValue`, same errors.
- **The executor is untouched.** No change to `run_into`, `Schedule`,
  `Replay`, `Actuator` or recovery. `recover`'s existing tier-1 skip for a
  refused footprint (`refused_by_the_game`) now also sees pre-check refusals,
  for free, because they land in the same ledger.
- **`PlacementRefusal` gained a constructor**, `at_dispatch(tick, entity,
  position)`, with deliberately no way to pass blockers or a tile — that path
  has nothing to put in them, and an optional argument would invite a caller to
  fill them from somewhere else, making a guess indistinguishable from an
  observation.
- **Ledger identity is unchanged**: still `(entity, position)`. A site learned
  by the pre-check and then refused again at dispatch is one entry, and the
  first recorded wins — which in practice means the pre-check's, the one
  carrying the evidence.

## Reported, not fixed

1. **`crates/scripting_lua/src/research_run_lib.rs` had a clippy failure on
   HEAD** (`manual_repeat_n`), unrelated to this change and red for anyone
   working in this tree. Fixed in its own commit (`24a211ee`) rather than
   folded in.
2. **A `Place` the schedule assigns to no bot is not asked about.**
   `can_place_entity` needs a force and a surface, both of which come from the
   acting player; inventing one would ask about a placement nobody is going to
   make. Such an action is already published `Failed` by `run_into` before the
   run starts, so nothing is lost — noted because the skip is silent.
3. **The plan-internal ordering gap (staleness item 3) is not guarded.** A
   cheap guard exists — suppress recording when a `Mine` in the same plan
   targets a position inside the refused footprint — and was left out because
   nothing observed needs it and the planner does not currently site on ground
   it plans to clear. If a false red is ever seen, that is the fix.
4. **`Mine` / `Insert` / `Remove` get no pre-check.** Only `Place` has a
   game-side "would this work" question with a cheap answer. `Mine`'s
   equivalent failure ("no entity to mine") has a different shape and a
   different cause and would need its own design.
5. **The three items carried forward from `2026-09-02-placement-refusal-3.md`
   are still open**: `record.plan_created`'s `bots` field reads like a roster
   and is not one; a run that never places anything gets no map coverage where
   it failed; and the phantom player invented by
   `initiate_missing_players_with_default_inventory` shadows the origin
   unconditionally.

## Test summary

**17 new tests across four layers.**

`crates/core/src/factorio/rcon.rs`, `transfer_guarantee_tests` (6) — driving
the **real** `control.lua` in a Lua 5.4 interpreter: the pre-check and the real
placement ask `can_place_entity` the identical argument table (the
`build_check_type` guard); an allowed site is green and carries no cause; a
refused site names the sorted distinct blockers and the tile; a character in the
footprint is reported and is not a durable refusal; an item with no
`place_result` is an error rather than a refusal; and a batch comes back in the
order it was asked, including a filled slot for a query the mod could not judge
(without which every later verdict joins to the wrong query).

`crates/core/src/factorio/rcon.rs`, `placement_precheck_tests` (4) — the
ledger filter. A four-site batch in which only one verdict is a fact about the
ground; a reply of the wrong length is rejected whole and records nothing; a
site already in the ledger is not recorded twice and keeps the first (evidence-
carrying) entry; and the warning's cause text distinguishes "nothing was in the
footprint" from "we did not look".

`crates/scripting_lua/src/globals/goal/plan.rs` (7) — the loop. The cost claim
(a legal plan costs exactly one round trip, whose queries are precisely the
plan's own placements); a plan with no placement makes no call at all; **the
defect** (a site the game would refuse never reaches the returned plan, is
remembered, and carries its blockers); re-siting is bounded and running out of
rounds still returns a plan; a character neither re-sites nor is remembered; a
checker that cannot reach the game does not stop planning; no checker plans
exactly as an all-clear checker does; and `placement_queries` deduplicates,
orders and skips a `Place` nobody is scheduled to make.

`crates/scripting_lua/src/globals/record.rs` (1) — a pre-check refusal records
its `source`, `blockers` and `tile`.

**Confirmed non-vacuous**, three ways:

- With `MAX_RESITE_ROUNDS` set to 0 (never re-expand), exactly one test fails —
  `a_site_the_game_would_refuse_never_reaches_the_returned_plan` — and all 24
  controls still pass, including the round-trip-count and character tests.
- With the `!self.character` guard removed from `is_durable_refusal`, exactly
  three fail: the two that assert a character is not a durable refusal, and the
  plan-level one that asserts it neither re-sites nor is remembered.
- With `build_check_type` changed to `ghost_revive` in the mod, the
  same-question test fails naming the build check.

**No existing test moved and no existing expectation changed.** The seven
`plan.rs` tests that became `#[tokio::test]` assert exactly what they asserted
before; they run the same scripts through the same bindings.

## Gates

- `cargo fmt --all -- --check` — clean.
- `cargo clippy --workspace --all-features --all-targets -- --deny warnings` —
  exit 0 (after `24a211ee`; red on HEAD before it, for an unrelated reason).
- `cargo test --workspace --all-features` — exit 0, 1213 passed, 0 failed.
- `luac -p mods/BotBridge/control.lua` — clean.
- `pnpm lint` (tsc + vue-tsc + eslint) — clean.
- `pnpm run test:coverage` — 48 files, 781 tests, 0 failed; coverage gate met.

All Rust gates through `nix develop --command`; a bare `cargo` cannot build
`mlua-sys` in this checkout.
