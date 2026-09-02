# Research is durative, and now somebody waits for it

Stage 1 of `docs/superpowers/specs/2026-09-02-mod-side-actions-design.md`
(§2 item 3, §3.5, step 4 of the staged migration). Nothing else from that spec
was implemented: no action registry, no approach folding, no tick-denominated
deadlines, no failure codes.

## The defect, re-verified

The spec's account is **correct in every particular**. Confirmed by reading,
not by trust:

- `mods/BotBridge/control.lua`, `rcon_add_research` — called
  `force.add_research(technology_name)` and, on `true`, `stamp_tick()`. The
  reply body was therefore a bare tick stamp the moment the technology entered
  the queue.
- `LuaForce.add_research` is documented as adding a technology "to the back of
  the research queue" and returning "whether the technology was successfully
  added". Queueing, not finishing.
- `on_research_finished` wrote out recipes, distances, a bare
  `on_research_finished` marker and the forces. **No action id anywhere**, so
  no line it produced could be joined to the action that asked for the
  research.
- `crates/core/src/factorio/rcon.rs`, `add_research_timed` — one
  `remote_call_timed`, then `Ok(ActionTicks::at(tick))`. Both ends of the
  returned `ActionTicks` were the *queue* tick.
- `crates/executor/src/rcon_actuator.rs`, `Actuator::research` — called that
  and mapped the error. No wait of any kind.

So the executor reported research a success the instant it was requested. Rung
7 of the ladder ("research automation") could not have worked as claimed: it
would report done while the technology was still being researched, and any step
depending on the unlocked recipe would run against a belief nobody established.

**Both halves of that were pinned red before the fix.**
`crates/core/tests/research_is_awaited.rs`, pointed at the old
`add_research_timed`, failed on `a queued technology is not a researched one;
the action must still be outstanding` — the call had already returned with
nothing completed. `crates/core/tests/botbridge_research_action.rs` failed 8 of
its 9 cases against the unmodified mod (the ninth is the unchanged
queue-only path, and passed, which is the control).

## What changed

### Mod (`mods/BotBridge/control.lua`)

- `rcon_add_research` became `start_research(technology_name, action_id)` with
  two thin entry points: `rcon_add_research` (no id, unchanged behaviour) and
  the new `rcon_action_start_research(action_id, technology_name)`, registered
  on the remote interface as `action_start_research`. All the refusal
  diagnostics are shared rather than duplicated.
- `research_actions()` — `storage.research_actions[technology_name]` is an
  **array of action ids**.
- `settle_research_actions(event)`, called from `on_research_finished`, looks
  the technology up by name, clears the entry, and writes one
  `action_completed` per waiting id at `event.tick`.

**How the mod remembers, and why that way.**

- *Keyed by technology name*, because that is the only join the game offers:
  `on_research_finished` carries the technology and nothing else — no request
  id, no queue position. `add_research` appends to the *back* of a queue that
  may already hold other technologies, so completions do not arrive in the
  order they were asked for. The positional match `crafting_queue` uses
  (`queue[1].recipe == event.recipe.name`) would settle the wrong action here;
  a name key cannot.
- *An array per name, not one id*, because two actions may ask for the same
  technology. Overwriting would leave the first waiting out the whole
  `ACTION_RESULT_DEADLINE` — six minutes of silence for something the game
  finished. `another_technology_finishing_settles_nothing` and
  `two_actions_waiting_on_one_technology_both_settle` pin both directions.
- *In `storage`, not a module local.* `crafting_queue` and
  `recent_item_additions` are module locals, `on_load` rebuilds nothing, and a
  craft spanning a save/load never settles. Research is the action kind most
  likely to be in flight across a save, so the same mistake here would be its
  most expensive version. `the_registry_lives_in_storage` asserts the location
  directly.
- *Created lazily, not in `on_init`.* `on_init` runs only for a save that never
  had this mod, and nothing registers `on_configuration_changed`, so a save
  gaining this version would otherwise reach the handlers with the key absent.
- *Registered before `add_research`, removed on refusal.* Nothing documented
  says `on_research_finished` cannot be raised inside that call; if it ever is,
  the handler must find the id already there. The refusal path takes the entry
  back out, because leaving it would let an unrelated research settle a refused
  action as a success — the exact overclaim being removed.
- *Settled last*, after `writeout_recipes` / `writeout_forces`. stdout is
  ordered, so the executor has the newly unlocked recipes before it is told the
  research succeeded. The other order lets the next plan step run against a
  world snapshot that does not know about the thing it just waited for.
  `the_completion_follows_the_world_data_it_unlocked` pins it.

**No `on_tick` work was added and no `on_nth_tick` handler was registered.**
Research needs neither: the game does the durative work itself and announces
the end of it, exactly as crafting does. The walk/mine followers exist because
those need `walking_state`/`mining_state` re-aimed every tick; research does
not. `on_nth_tick(n, f)` replaces the handler for `n`, and 60 and 300 are
already taken — nothing here goes near them.

### Rust

- `FactorioRcon::research_timed(&world, technology_name)` — allocates an action
  id from `world.next_action_id` the way walk/mine/craft do, dispatches
  `action_start_research`, and waits on `sleep_for_action_result`. Shaped
  exactly like `player_craft_timed`. **No polling of the game was added**: the
  wait is on the existing push channel (mod `writeout` → `OutputParser` →
  `world.actions`), so the reply tick is the game's own `game.tick` at the
  moment the research finished.
- A refusal is classified `Dispatch::Refused` explicitly rather than falling
  through `?` to `NotDispatched`: the game did see the command and did judge
  it. The mod registers nothing for a technology it would not queue, so there
  is no completion coming and nothing to wait for — a refusal must not become a
  360-second silence. `a_refused_research_fails_at_once_rather_than_waiting`.
- `Actuator::research` calls it. That is the whole executor change; it issues
  no new command kinds and no `cheat_*` call.
- Doc corrections on `add_research` / `add_research_timed` and on the Lua
  binding `rcon.add_research`, all saying plainly that they return at *queue*
  time.

## Rejected

- **Making `add_research` itself await.** It is the Lua binding and the REST
  endpoint (`POST /api/v1/game/add_research`). A script that starts a research
  and carries on would suddenly block for minutes, and an HTTP request would
  hang. Two named calls — queue it, or research it — is the honest split, and
  `queueing_research_still_returns_at_once` keeps the queue-only path from
  quietly acquiring a wait.
- **Deleting `add_research_timed`.** Still the implementation of
  `add_research`, and its doc now carries the warning that used to be missing.
- **Polling `force.current_research` or `research_progress` over RCON.**
  Spec §2.3: this project has a push channel that already carries the game
  tick; every `/silent-command` is a replicated input action on every peer, and
  UPS is the resource already short.
- **A mod-side deadline for research.** Spec §3.5: duration depends on lab
  count, science supply and speed modules, none of which the mod can bound
  without modelling the factory. The executor's deadline is the only honest
  one.
- **`on_research_started` as an `action_progress` line.** That is spec §2.4,
  which is stage 3, and there is no `action_progress` line kind yet. Adding one
  here would change the wire format for the whole action machinery in a commit
  about research.

## Tests

- `crates/core/tests/botbridge_research_action.rs` — 9 tests, the mod half.
  Loads the real `control.lua` into mlua on a stub game (the
  `botbridge_placement_material.rs` pattern), starts research actions and
  raises `on_research_finished` at the handler. Captures **both** channels:
  `rcon.print` (the reply body the executor reads as the action's result) and
  `print` (the stdout `writeout` lands on).
- `crates/core/tests/research_is_awaited.rs` — 4 tests, the Rust half. Speaks
  the RCON wire protocol to a fake server (the `rcon_oversized_reply.rs`
  harness, extended to record the commands it was sent), so the dispatch and
  the reply are real and "has the call returned yet" is observable. The
  completion is delivered as `OutputParser` delivers it.

Verified at `2c238391` plus only this change, in a clean worktree:
`cargo fmt --all --check` clean, `cargo clippy --workspace --all-features
--all-targets -- --deny warnings` clean, `cargo test --workspace` **1253
passed, 0 failed, exit 0**.

## Could not be verified without a live run

1. **That `on_research_finished` carries `event.research` with a `.name`
   matching what was passed to `add_research`.** Taken from the API docs and
   from the existing handler's signature; the stub supplies that shape, so the
   test proves the join *given* the shape, not the shape.
2. **Whether a technology can be dropped from the research queue without ever
   raising `on_research_finished`.** Cancelling the current research, clearing
   the queue, or a technology becoming unresearchable would leave the registry
   entry and the action forever. The action then costs the full 360-second
   `ACTION_RESULT_DEADLINE` and reports `NoVerdict`, which is the correct claim
   but a slow one. No timeout was added mod-side, per §3.5. `storage` keeps a
   stale entry in that case, which would settle a *later* research of the same
   technology as this action — bounded, but wrong. Worth revisiting once the
   spec's action registry exists.
3. **Whether real research fits inside the 360-second deadline.** This is the
   spec's own step-4 risk ("goes from ~0 to minutes") and it is now live.
   `automation` is 10× red science at 30 s; with two labs that is ~150 s of lab
   time, inside the deadline, but only if science is actually being supplied.
   A research that starves reads as `NoVerdict` after six minutes rather than
   as "no science".
4. **The effect on the executor's schedule shape and lag edges.** A research
   action that used to return in one tick now occupies a bot-independent action
   for minutes. Nothing in `crates/executor` was changed to account for that,
   and nothing obviously needs to be — research takes no bot — but the
   supervisor's stuck detector has not been observed against a run where one
   action legitimately runs for minutes.
5. **Whether every peer's `on_research_finished` agrees.** Only the server
   peer's stdout is parsed, so a divergence would be invisible. Spec §8 item 2.

## One process note

`crates/core/src/factorio/rcon.rs` shows as unmodified in this commit because
another agent's commit `e562847a` ("fix(core): retire an ore tile the moment a
mine empties it") swept up this change's uncommitted `rcon.rs` edits from the
shared checkout. The content is correct and present; it simply landed under
another commit's message. This is the failure mode CLAUDE.md's committing rules
exist to prevent, and it has now happened twice.

---

## Correction, same day

Lines above cite `crafting_queue` as a live module local and use its positional
match as a contrast. Both are now historical: `d0a5e094` removed the module local
in favour of `storage.craft_actions[player][recipe]`.

The contrast drawn here was right for the wrong reason. This note says the
positional match "would settle the wrong action". It does something worse — a
non-matching craft was ignored **while leaving the head in place**, so a single
entry that will never be crafted silenced every later craft for that bot
permanently. The research fix's choice to key by name avoided that, so the
decision recorded here stands; only the description of what it was avoiding was
too mild.
