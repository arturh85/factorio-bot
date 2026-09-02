# Three reliability defects, 2026-09-02

Three independent defects reported from live runs and never fixed. All three
turned out to be real. One commit each, plus a fourth for a false doc comment.

---

## 1. Nothing retired a mined-out ore tile — **real**

### Cause

Two halves were missing, not one.

* **No depletion signal exists.** `mods/BotBridge/types.lua` sends
  `record.amount` for a resource when its chunk is written out, and never
  again. `mined_item`'s `writeout` is commented out in `control.lua`. There is
  no `on_resource_depleted` anywhere — not in the mod, not in this workspace —
  despite `EntityGraph::resource_amount`'s own doc comment asserting that one
  "removes it from this map". That sentence had been wrong for as long as it
  existed and is what made the gap invisible.
* **Nothing called the retirement that does exist.** `EntityGraph::remove`
  retires a resource tile correctly, and has a green test proving it
  (`removing_a_resource_delivered_twice_empties_the_tile`). No mine path ever
  called it, and no path debited the stored amount either.

So a tile the run mined to zero kept its pre-run reading, kept appearing in
`resource_patches`, and `PlanState::resource_available` kept reporting it as a
candidate. The planner re-chose it and the bot walked back to nothing. Within
one plan `PlanState::consumed` hides this; the failure is *across* re-plans,
which is the loop the supervisor actually runs.

### Change

`crates/core/src/graph/entity_graph.rs`

* `EntityGraph::resource_mined(name, position, mined) -> ResourceDepletion`
  debits the tile and retires it at zero.
* `EntityGraph::retire_resource(name, position) -> bool` takes it out outright,
  for when the game destroyed the entity and no arithmetic here can be trusted.
* `ResourceDepletion` is four-valued (`Absent`, `AmountUnknown`, `Remaining(u32)`,
  `Exhausted`) rather than an `Option<u32>`. "No such tile", "nobody ever said
  how much is in it" and "empty, and retired" are three different facts, and
  conflating the middle one with a number is the mistake
  `DEFAULT_RESOURCE_PER_TILE` already made: a tile holding twenty read as five
  hundred. A tile with no reported amount is left exactly as it was.

`crates/core/src/factorio/rcon.rs`

* `player_mine_timed` calls `resource_mined` on success (with the count the
  game took) and `retire_resource` when the verdict carries the mod's "was gone
  before mining finished". `player_mine_timed` is the single choke point — both
  the executor and the `rcon.*` Lua binding go through it — and it already
  holds the `world`, so nothing in `crates/executor` had to be touched.
* `mine_reports_target_gone` is a free function over the verdict text, because
  the verdict reaches Rust as one opaque string in `RconError`.

**The half-tile trap.** Every test uses a real tile centre `(-40.5, -48.5)`.
`resources` keys by a flooring `Pos`, and `retire_resource` has to rebuild the
centre through `resource_position_from_pos` to find the tile in `resource_tree`
at all. A fixture on integers round-trips losslessly through the flooring and
would have proved nothing — which is exactly how this class of bug shipped
once before with every test green.

### Tests

`crates/core/src/graph/entity_graph.rs`

* `mining_a_tile_dry_retires_it` — partial mine debits and leaves the tile;
  the mine that empties it removes it from `resources`, `resource_patches` and
  `snapshot_within`; a further report finds nothing to debit.
* `retiring_a_tile_leaves_its_neighbour_alone` — the adjacent-tile control.
* `mining_a_tile_of_unknown_amount_changes_nothing` — negative control.
* `a_vanished_target_is_retired_whatever_the_model_believed`.

`crates/core/src/factorio/rcon.rs`

* `a_vanished_mine_target_is_recognised_from_the_mods_own_wording` —
  `include_str!`s `mods/BotBridge/control.lua` through `repo_mods_path!` and
  asserts the phrase is still in it, so a reword fails here rather than
  silently switching the retirement off. Also checks the phrase survives the
  `RconError` wrapping, and that an out-of-reach refusal does **not** match.

Verified failing first: with the retirement disabled (the pre-fix state), 3 of
the 4 `entity_graph` tests fail; the fourth is the negative control and is
expected to pass either way.

### Not verifiable without a live run

* That the mod's "was gone before mining finished" is in fact what a
  *depletion* produces (as opposed to only a genuine race with another bot).
  The reasoning is that mining a tile to zero destroys the entity and the
  watchdog fires on `not ent.valid`, but only a run shows it.
* That `count` is what the game actually took on a success. The mod's
  `on_mined_entity` compares delivered against requested and can report success
  having delivered less; if that turns out to happen, the debit is an
  overestimate and the retirement fires early. It cannot fire *late*, which is
  the direction that costs a wasted walk.
* Whether the workspace copy of `BotBridge` on any given machine still matches
  the checkout the guard test reads.

---

## 2. A phantom bot at (0,0) — **real**

### Cause

`Planner::initiate_missing_players_with_default_inventory(bot_count)` built the
run's roster by **inventing** a player for every id in `1..=bot_count` the world
did not already have. An invented player takes `FactorioPlayer::default()`,
whose position is the origin. `lua_runner::run_lua` called it on every run.

The world's players come from the game — `attach_world` seeds them from
`connected_players()`, and `OutputParser` keeps them current — so a client that
failed to connect simply is not there, and the invention filled the hole. The
planner then sized and assigned real work to that bot. None of it could
complete: `RconActuator` checks its own roster against `connected_players()`
and refuses a player the game does not have, so every step that bot owned was
dead on dispatch, while the plan reported itself as covering four bots.

### Change

* `Planner::roster(bot_count)` — new, `&self`, returns the ids the world
  actually has a player for. `lua_runner` uses it.
* `initiate_missing_players_with_default_inventory` keeps the invention
  unchanged and is now documented as the **simulation seam** it always was.

**Omit or fail loudly?** Omit, and name what was omitted. Three working clients
out of four are three working clients, and a run that can still do most of the
work should; raising would throw away a run that is mostly fine. But a silently
missing bot is its own failure mode, so every absent id is warned about by
name. The total failure is loud without any help from here: no client at all
means an empty roster, which `crates/planner`'s `schedule` refuses outright
with a sentence about having no bots.

**`--clients 0` is why this is not simply "delete the invention".** The
documented fast planning loop (`factorio-bot lua goal_smoke.lua --clients 0
--bots 4`) starts no Factorio client and legitimately wants four bots. So the
seeding moved to the one place that *knows* it is simulating — the `clients ==
0` branch in `app/src-tauri/src/cli/lua.rs`, which seeds before the script runs
so the roster comes back whole. Discriminating instead on "the world has no
players" would have been a guess, and it is precisely the signal that cannot
tell "simulating" from "every client failed to connect".

A roster is therefore no longer necessarily `1..=n`. `crates/planner`'s
`ids.rs` says so: `[1, 3]` is ordinary now, and packing it down to `[1, 2]`
would drive the wrong player for every step bot 3 owns.

Two fixtures (`goal_script_fixture`, `lua_runner::tests::test_script`) build a
world from nothing and now seed explicitly, the same way `--clients 0` does.

### Tests

`crates/core/src/plan/planner.rs`

* `a_client_that_never_connected_produces_no_bot` — roster of `[1,2,3]` from a
  world of three, and no player 4 is invented by the asking.
* `a_gap_in_the_middle_of_the_roster_stays_a_gap` — `[1, 3]`, not `[1, 2]`.
* `a_run_whose_clients_all_failed_gets_no_bots_at_all`.
* `the_simulation_seam_still_invents_the_bots_it_is_asked_for` — pins the
  planning-only path, which would otherwise silently plan for nobody.

Verified failing first by pointing the three roster tests at the old production
call: `[1,2,3,4]` against `[1,2,3]`, `[1,2,3]` against `[1,3]`, and a non-empty
roster where none was expected.

### Not verifiable without a live run

* That a *connected* client is reliably in `world.players` by the time the
  script starts. It reaches the world through the mod's
  `on_player_changed_position` writeouts (and through `attach_world` in
  `--connect` mode). If the mod turns out not to emit for a player who joined
  and has not moved, the roster would come back short for a client that did
  connect — which the warning would name, but a run is what would show it.
  Making `run_lua` refresh players from `connected_players()` before reading
  the roster is the obvious hardening if that happens.
* The behaviour of the REPL path (`app/src-tauri/src/repl/run_script.rs`),
  which was left alone and now gets the strict roster.

---

## 3. `plan_created.bots` derived from steps — **real**

### Cause

`record.plan_created`'s Lua binding built `bots` by walking the plan's steps
and collecting the distinct bot ids. A bot that got no work therefore did not
appear. A live four-bot run recorded `bots: [2]`, which reads exactly like a
run of one bot, and an empty plan recorded `bots: []`, indistinguishable from a
run with no bots at all. The bots that got nothing are the whole question the
record is asked.

### Change

`crates/scripting_lua/src/globals/record.rs` — `bots` now comes from
`create_lua_record`'s `all_bots`, the run's roster, which the binding already
closes over. **No Lua signature change**: `record.plan_created(index, plan)` is
what it was, so `scripts/research_run.lua` and `scripts/supervisor.lua` are
untouched, and a script cannot pass a roster that disagrees with the run.
`steps` and `makespan` stay derived from the plan.

Combined with defect 2, the field now means "the bots this run actually has",
because `all_bots` is `Planner::roster`'s output.

The wire shape is unchanged (`bots: number[]`), so only the field's description
moved in `app/src/api/openapi.snapshot.json`.

### Tests

* `plan_created_reports_the_runs_roster_and_not_the_bots_in_the_plan` — roster
  of four, plan naming two, `bots == [1,2,3,4]`.
* `plan_created_of_an_empty_plan_is_zero_steps_and_zero_makespan` — an empty
  plan still names who was available.

Verified failing first against the derived version: `[1,2]` against `[1,2,3,4]`
and `[]` against `[1,2]`.

### Not verifiable without a live run

Nothing here needs one. What a live run would *add* is the confirmation that
the two fixes together make a short roster visible end to end: a run started
with four clients where one fails should now record three bots in
`plan_created` and warn by name about the fourth, instead of recording four and
losing a quarter of the plan to dispatch refusals.

---

## Also: `ArchivedFrame.bot`'s doc comment was false

It claimed "Bots and clients are 1:1". The archive on disk disproves it:
`client3` holds the `follow`, `bot-1` and `area` cameras while `client1` holds
`bot-4`. The field is a **client** number despite its name; which bot a frame is
*of* is in `camera`, and only when the camera names one — `follow` and `area`
name none. Corrected, which moved one description in the OpenAPI snapshot.

---

## Verification

`cargo fmt --check`, `cargo clippy --workspace --all-features --all-targets --
--deny warnings` and `cargo test --workspace` all clean, plus the frontend
suite (48 files, 783 tests) since the OpenAPI snapshot moved. Every cargo
invocation through `nix develop -c` — a bare one cannot find Lua 5.4.
