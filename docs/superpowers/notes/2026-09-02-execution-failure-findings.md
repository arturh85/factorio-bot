# Execution failure of run-1788298772-73007 — findings

Run: `target/release/factorio-bot lua research_run.lua -c 4 -n`, 2026-09-01
23:37:45 → 23:45:39 (wall). Record: `workspace/runs/run-1788298772-73007/`.
Log: `scratchpad/run3.log`.

**Headline, measured:** the Factorio *server process exited* 9 seconds after the
script began, because a mod handler added earlier the same day
(`sample_bots`, commit `ceddda80`) raised on a Factorio API misuse the first
time a bot actually started mining. Everything else in the report — the six
minute stall, the `early eof`, the zero successes — is downstream of that. A
second, unrelated and much older mod bug crashed one of the four clients during
its join, which is why the roster was 3.

Throughout: **Measured** = read off a log line, a record line or source.
**Inferred** = a conclusion I drew from those.

---

## Q1 — What is `early eof`, mechanically, and what actually happened?

### What `early eof` is (measured)

It is not this workspace's string and not the `rcon` crate's either. It is
tokio's:

- `~/.cargo/registry/src/.../tokio-1.53.1/src/io/util/read_exact.rs:44`
  → `io::Error::new(io::ErrorKind::UnexpectedEof, "early eof")`

`read_exact` produces it when the stream ends before the buffer is full. The
only `read_exact` calls in the RCON path are in the vendored `rcon` crate:

- `~/.cargo/registry/src/.../rcon-0.6.0/src/packet.rs:85,87,89` — the 12-byte
  packet header (length / id / type)
- `rcon-0.6.0/src/packet.rs:103` — the two terminating NUL bytes

So `early eof` means exactly one thing here: **the RCON TCP connection returned
EOF mid-packet**, i.e. the peer closed the socket. It is *not* a length limit,
and there is no size check anywhere on that path that could produce this
wording. (Note `packet.rs:94` reads the body with `take(...).read_to_end(...)`,
which tolerates a short read silently — so on a truncated body the failure
surfaces at the trailing-NUL `read_exact` on line 103 rather than at the body.)

### What actually closed the socket (measured)

`workspace/server/factorio-current.log` (this run: header
`0.000 2026-09-01 23:37:49`):

```
110.207 Error MainLoop.cpp:1488: Exception at tick 5880: The mod BotBridge (0.0.1) caused a non-recoverable error.
Error while running event BotBridge::on_nth_tick(60)
Entity is not mining-drill.
stack traceback:
	[C]: in function '__index'
	__BotBridge__/control.lua:1326: in function 'sample_bots'
	__BotBridge__/control.lua:1346: in function <__BotBridge__/control.lua:1345>
110.207 Info ServerMultiplayerManager.cpp:85: MultiplayerManager failed: ...
110.214 Quitting: multiplayer error.
111.441 Goodbye
```

Server started 23:37:49, so `110.2 s` = **23:39:39**. The script started at
23:39:32 (`run3.log`). The server was gone **7 seconds into the script**.

All four Factorio processes hit the same raise at the same tick
(`client1/factorio-current.log:169`, `client3:167`, `client4:169`, all
`Exception at tick 5880`) — `on_nth_tick` runs on every peer, so a raise there
is fatal to the whole session, not just the server.

The offending line, `mods/BotBridge/control.lua:1325-1326`:

```lua
mining = character and character.mining_state.mining
    and character.mining_target and character.mining_target.name or nil,
```

`LuaEntity::mining_target` exists only on mining drills; on a character it
raises `Entity is not mining-drill.` — Factorio's own wording, quoted verbatim
in the log. `git blame` puts both lines in `ceddda80`
*"feat(mod): push world-state samples on the server only"*, dated **2026-09-01**
— i.e. added this session.

The guard `character.mining_state.mining` is why it took until tick 5880:
short-circuit `and` never evaluated `mining_target` while every bot was walking.
Every sample in `samples.jsonl` (ticks 5460…5820) carries `"mining":null`. Tick
5880 is the first 60-tick beat after the two mine dispatches at ticks 5846 and
5853 — the first beat at which a character was actually mining.

Note also that the commit title is misleading about scope: "on the server only"
describes `write_sample`'s `helpers.write_file(..., 0)` target, not where the
handler runs. The table is *built* on every peer.

### How that became `goal: game rejected the command: early eof` (measured)

Chain, each link read from source:

1. `crates/scripting_lua/src/globals/goal/run.rs:604` registers `goal.run` via
   `create_async_function` — which is why the Lua traceback reads
   `[C]: in local 'poll'` / `[string "?"]:4: in field 'run'`. The raise came out
   of Rust, not out of a Lua chunk.
2. `run.rs:563` — `start_impl` does `let act = actuator().await.map_err(goal_error)?;`
   *before* anything is dispatched.
3. `crates/scripting_lua/src/globals/goal/mod.rs:85` — that factory is
   `RconActuator::new(rcon, world)`.
4. `crates/executor/src/rcon_actuator.rs:149-152` — `RconActuator::new` first
   calls `rcon.connected_players()` and maps any error to
   `ActuatorError::Rejected(e.to_string())`.
5. `crates/executor/src/actuator.rs:13` — `Rejected`'s Display is
   `"game rejected the command: {0}"`.
6. `goal_error` prefixes `"goal: "`.

So the message decomposes as
`goal: ` + `game rejected the command: ` + `early eof`, and the `early eof` is
the roster query failing on a socket the dead server had FIN'd.

**This wording is actively misleading and is worth fixing on its own.** The game
did not reject anything; the game did not exist. `Rejected` is documented
(`rcon_actuator.rs:246-255`) as deliberately under-claiming for transport
failures, which is the right call for `Lost`/`Rejected` classification but wrong
for the *text a human reads*.

### The oversized-response hypothesis: ruled out (measured)

- `record.keyframe` is the only caller of `find_entities_filtered` added this
  session (`crates/scripting_lua/src/globals/record.rs:711`).
- It is invoked from exactly one place: `scripts/supervisor.lua:158`, inside
  `Sup:_close()`.
- `_close` is only reached on a milestone closing. This run raised *inside*
  `Sup:step()`, so `_close` never ran. **`record.keyframe` was never called.**
- Even if it had been: `record.rs:697-704` returns `false` before any RCON call
  when `placed_bounds` is `None`, and `placed_bounds`
  (`crates/core/src/record/mod.rs:305`, updated only by `record_map` on a
  `placed` line) was `None` because nothing was ever placed this run.
- Corroborating: `map.jsonl` is 0 bytes and `manifest.json` says `"map": 0`.

The large `entities` payloads visible in `workspace/server-log.txt` are from the
**2026-08-31 17:58 run** — that file's mtime is `2026-08-31 17:59:45` and its
header line is `0.000 2026-08-31 17:58:15`. It has nothing to do with this run.
(`write_logs` was off here; this run's Factorio logs are the
`*/factorio-current.log` files.)

I did **not** establish that the RCON client is safe against genuinely oversized
Factorio responses — Factorio chunks large RCON replies across packets and I did
not audit whether `crates/core/src/factorio/rcon.rs` reassembles them. That
remains an open risk, just not this bug.

### Why the six-minute stall (measured)

`crates/core/src/factorio/rcon.rs:37`:

```rust
const ACTION_RESULT_DEADLINE: Duration = Duration::from_secs(360);
```

`sleep_for_action_result_until` (`rcon.rs:1034-1076`) polls only in-process
state (`world.actions.remove(...)`) every 50 ms. It has **no liveness check on
the RCON socket or the Factorio child process**. Two mine actions had been
acknowledged by the mod before the crash, so both waits ran the full deadline.

Measured corroboration: `events.jsonl` has `plan_created` at `wall_ms: 6` and
the first `action_dispatched` at `wall_ms: 367485` — 367 s, i.e. 360 s deadline
plus overhead. `run3.log` timestamps agree: `21:39:32` → `21:45:39`.

### Unknown

- Whether `crates/core`'s RCON layer handles multi-packet Factorio replies. Not
  implicated here; unaudited.

---

## Q2 — Why did the first six actions fail?

### The tally in the log is wrong; the real tally is benign (measured)

`run3.log`: `ran: success=0 failed=2 lost=2 pending=1`. That reads as five
outcomes for a three-action DAG. It is a double count:

- `scripts/supervisor.lua:286` — `local failed = (obs.failed or 0) + (obs.lost or 0)`
- `scripts/supervisor.lua:299-303` — returns **both** `failed = failed` (the
  sum) and `lost = obs.lost`
- `scripts/research_run.lua:92-94` prints both side by side

So the true observation was `success=0, failed=0, lost=2, pending=1` — three
actions, matching the three in `plan_created`. **Nothing failed.** Two actions
were dispatched and never got a verdict (the server died); one was never
reached.

This also explains the empty `first_error`: `build_observation`
(`crates/scripting_lua/src/globals/goal/run.rs:193`) only fills `failures` for
`Status::Failed`, and there were none — which is why `milestone_stuck.last_error`
carries the outer `pcall` traceback rather than an action error.

Console line "planned 6 steps" vs the record's `"steps":3` is also not a
discrepancy, but the two numbers mean different things: `supervisor.lua:198`
counts every step including walks; `record.plan_created`
(`crates/scripting_lua/src/globals/record.rs:460`) counts only what it recorded,
and `plan_for_record` (`supervisor.lua:70-88`) skips any step without an `id` —
i.e. all walks. So 6 steps = 3 walks + 3 mines.

### The bots did reach the ore (measured, then one inference)

Three independent pieces of evidence, none of which requires knowing where the
ore was:

1. **The mod accepted both mine dispatches.**
   `mods/BotBridge/control.lua:2044-2065` (`rcon_action_start_mining`) does
   `ent = player.surface.find_entity(name, position)` and, if that misses, calls
   `rcon.print("Error: no entity to mine")` **and** `action_failed(...)`
   immediately. An immediate `action_failed` would have come back through
   `sleep_for_action_result` as a *verdict* → `Dispatch::Refused` →
   `Status::Failed` with a `replied_tick`. We got `Lost` with no reply. So
   `find_entity` **hit**. The tile-centre / `Pos(i32,i32)` flooring trap did not
   bite here.
2. **The reach guard did not fire.**
   `control.lua:705-707` fails an action with `"ERROR: too far too mine"` once
   per tick while `distance(player.position, ent.position) > player.resource_reach_distance`.
   That also produces a verdict. None arrived.
3. **A character was actually mining.** The crash itself proves it: the raise at
   `control.lua:1326` is reachable *only* when
   `character.mining_state.mining` is true. At tick 5880 — 27 ticks after the
   last dispatch — at least one bot was mid-mine.

Bot positions from `samples.jsonl` are consistent with a normal approach: all
three walk together from spawn `(4.8, -3.9)` at tick 5460 out to `(29.8, -28.9)`
at 5700, then west along `y ≈ -32.2` to `x ≈ 17-19` by 5820. Inventories are
static across all seven samples (`burner-mining-drill 1, stone-furnace 1, wood 1`,
plus `iron-plate 8` for bots 2 and 3) — no mining output yet, as expected 27
ticks into the first mine.

**Conclusion (inferred, but on three independent measured legs): question 2's
premise does not hold. The plan was fine, the geometry was fine, and the bots
were in reach and mining. There is no separate action-failure bug here — the two
"lost" actions and the one "pending" are all consequences of the server dying at
tick 5880.**

### Record gaps this exposed — the actionable part

The record could *not* have answered this on its own. Specifically:

1. **`record.actions` writes nothing for an action with no `replied_tick`.**
   `crates/scripting_lua/src/globals/record.rs:594` gates `ActionSettled` on
   `if let Some(replied) = replied`. A `Lost` action has a dispatch tick and no
   reply, so it produces an `action_dispatched` line and **no settle line at
   all**. `events.jsonl` therefore shows two dispatches, zero settles, and no
   record-side trace that anything went wrong until `milestone_stuck`.
   *Wanted field:* emit `ActionSettled` for any action whose final status is not
   `Pending`, with `elapsed_ticks: None` when there is no reply tick. `Lost` is
   a status the enum already has (`crates/executor/src/run.rs:389`); it just
   never reaches the file.
2. **`PlannedStep` carries no position.** `crates/core/src/record/mod.rs:83-103`
   / the `PlannedStep` struct has `id, bot, action, deps, planned_start,
   planned_duration`. `"mine 7 iron-ore"` does not say *where*, and
   `ActionDispatched.target` is `None` because `record.rs:588` hardcodes
   `target: None`. So "were the bots near the ore" is unanswerable from the
   record. *Wanted field:* the mine/place target position on `PlannedStep`
   (the planner has it — `goal/plan.rs:500` sets `pos` on the Lua step, and
   `plan_for_record` drops it), and a real `target` on `ActionDispatched`.
3. **Walks are absent from the record entirely.** `plan_for_record` skips them,
   so the recorded DAG (3 nodes) does not describe the plan that ran (6 steps).
   A reader cannot tell a plan that was mostly walking from one that was not.
4. **Nothing in the record says the game died.** The strongest available signal
   is arithmetic: `manifest.json` has `elapsed_ticks: 448` against a wall span of
   367 s — ~1.2 ticks/s where 60 is normal. *Wanted field:* a wall-clock-to-tick
   ratio, or better, an explicit event when the Factorio child exits or the RCON
   connection drops.

### Unknown

- The ore's actual coordinates, and how far each bot walked relative to plan.
  Not recoverable from this record (see gap 2).

---

## Q3 — Why did only 3 of 4 clients connect?

### Established (measured)

All four clients launched, connected, downloaded the map and began catching up.
**Client 2 crashed during its own catch-up**, on a *different* BotBridge bug:

`workspace/client2/factorio-current.log:164-178`

```
49.555 Error MainLoop.cpp:1488: Exception at tick 2925: The mod BotBridge (0.0.1) caused a non-recoverable error.
Error while running event BotBridge::on_player_joined_game (ID 52)
__BotBridge__/control.lua:1715: attempt to index upvalue 'client_local_data' (a nil value)
49.555 Info ClientMultiplayerManager.cpp:608: UpdateTick(2925) changing state from(TryingToCatchUp) to(Failed)
```

Server side, `server/factorio-current.log`:
`81.247 Info ServerMultiplayerManager.cpp:1085: Disconnect notification for peer (3)`.

Peer↔client mapping, from matching the server's `Serving map(...) for peer(N)`
tick against each client's `ConnectedLoadingMap → TryingToCatchUp` tick:
peer 1 = client4 (2855), peer 2 = client1 (2916), **peer 3 = client2 (2925)**,
peer 4 = client3 (3000). The server logs `PlayerJoinGame` for peers 1, 2 and 4
only. Consistent with `workspace/runs/.../frames/` containing directories `1`,
`3`, `4` and no `2`.

### The mechanism (measured, plus one inference about ordering)

- `control.lua:55` — `local client_local_data = nil -- DO NOT USE, will cause desyncs`
- `control.lua:577-581` — it is lazily initialised **only inside `on_tick`**.
- `control.lua:1715` — `on_player_joined_game` dereferences it unguarded.

`git blame`: line 1715 is from `850f8c19` (2021-08-04). This is a **latent
pre-existing bug**, not something this session introduced.

Why only client2: a joining peer replays tick closures during
`TryingToCatchUp`, and Factorio runs events before `on_tick` within a tick.
Client2's *first* simulated tick was 2925, which is exactly the tick at which
the server processed `PlayerJoinGame peerID(1)`. So client2 ran
`on_player_joined_game` before it had ever run `on_tick`. Client4 started at
2855, client1 at 2916 and client3 at 3000 — each had at least one `on_tick`
before the first join event reached it. (The event-before-`on_tick` ordering is
the inferred half; the tick numbers are measured, and they line up exactly.)

This is a **race, not a determinism problem**: it fires when a client's
map-load tick coincides with a tick carrying a join event. With four clients
joining within ~3 seconds of each other, the odds are not small.

Note that `rcon.whoami(...)` — the only thing that sets
`client_local_data.whoami` — is sent from `process_control.rs:255` *after* the
90-second connect wait, so it cannot help.

### Relation to the execution failure (measured)

**Independent.** Client2 died at server-clock 49.5 s; the run's fatal crash was
at 110.2 s, in different code, from different commits, on all remaining peers.
The only coupling is cost: `crates/core/src/process/process_control.rs:242`
waits a flat 90 s for a client count that could never be reached, and there is
no check on the client child processes' liveness, so the run burned 90 s
(23:38:01 → 23:39:31) waiting for a client that had already failed. The run then
proceeded with 3 bots, which is correct behaviour
(`app/src-tauri/src/cli/start.rs:118-119` documents that it is deliberate).

### Unknown

- Whether the crashed client's process stayed alive showing an error dialog.
  Its log has no `Goodbye` line, unlike the server's, which suggests it did —
  but no process survived to check, and nothing in the record says.

---

## Recommended fix order

1. **`mods/BotBridge/control.lua:1325-1326` — remove `character.mining_target`.**
   *Confidence: certain.* This is the whole failure. Characters have
   `mining_state` (`{mining, position}`) and no `mining_target`. Report
   `mining = character.mining_state.mining or nil` (a boolean), or resolve a
   name via `player.selected` / `surface.find_entity(nil, character.mining_state.position)`
   if the name is genuinely wanted. Remember `workspace/mods/` is the copy that
   runs — the CLI already warns that it is stale; refresh with
   `FACTORIO_BOT_REFRESH_MODS=1` or edit the workspace copy directly.

2. **Wrap `sample_bots` and `sample_force` in `pcall`.**
   *Confidence: high.* These handlers run inside the deterministic simulation on
   every peer; any raise kills the entire multiplayer session. An observability
   feature must never be able to do that. Log once and disable itself rather
   than propagate. Fix 1 without fix 2 leaves the class of failure open.

3. **`mods/BotBridge/control.lua:1715` — guard `client_local_data`.**
   *Confidence: certain about the bug, high about the fix.* Either hoist the
   lazy init into a small `ensure_client_local_data()` called from both
   `on_tick` and `on_player_joined_game`, or make line 1715 tolerate nil. This
   costs one of four clients on most multi-client runs.

4. **Fail fast when the game is gone.**
   *Confidence: high on the need, medium on the shape.* Two places:
   `sleep_for_action_result_until` (`crates/core/src/factorio/rcon.rs:1042`)
   should abort its 360 s wait when the RCON connection or the Factorio child
   has died; the 90 s client-connect loop
   (`crates/core/src/process/process_control.rs:214-249`) should stop early when
   a spawned client process has exited. Turning 6 minutes of silence into an
   immediate, accurate error is worth more than any record improvement below.

5. **Fix the double-counted tally.**
   *Confidence: certain.* `scripts/supervisor.lua:286` folds `lost` into
   `failed`, and lines 299-303 return both, so `research_run.lua` prints
   `failed=2 lost=2` for two lost actions and zero failures. Either stop folding
   or stop returning both. This one line sent this investigation looking for an
   action bug that does not exist.

6. **Reword `ActuatorError::Rejected` for transport failures.**
   *Confidence: high.* `crates/executor/src/actuator.rs:13` prints "game
   rejected the command" for an error where the game was never reached. Keep the
   `Rejected` *classification* (the under-claim is deliberate and correct) and
   change the text, or add a separate variant with the same classification.

7. **Record gaps, in the order they cost most.**
   *Confidence: medium — these are judgement calls about the format.*
   a. Settle every non-`Pending` action, not only those with a `replied_tick`
      (`crates/scripting_lua/src/globals/record.rs:594`).
   b. Carry target positions on `PlannedStep` and `ActionDispatched.target`.
   c. Record walks, or say explicitly that the DAG omits them.
   d. Emit an event when the Factorio process exits or RCON drops.
