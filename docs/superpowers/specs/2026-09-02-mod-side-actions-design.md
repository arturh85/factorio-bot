# Design: durative character actions inside the mod

Status: **design only, not implemented.** Nothing outside this file was
changed. No Factorio process was started.

**Line numbers are a moving target.** `mods/BotBridge/control.lua` and
`crates/core/src/factorio/rcon.rs` were both dirty and being edited by other
agents while this was written. Every `file:line` below was read at
`control.lua` md5 `1fbe607a3c6be5fee290030e4e3f5ebe`, working tree
`a5f5aaad` + uncommitted changes. Function names are stable; the numbers may
have shifted by the time you read this. Two functions moved by ~50 lines
*during* the reading (`rcon_action_start_crafting` 2875 → 2924,
`rcon_insert_to_inventory` 2597 → 2646), which is the evidence for the
warning rather than a guess at it.

---

## 1. What is actually true today

The brief's premise — "the executor steers over RCON and should stop" — is
**false for the two actions it is most often said about**. Walking and mining
are already mod-side `on_tick` state machines holding their state in
`storage`, exactly the shape the prior-art note recommends
(`docs/superpowers/notes/2026-09-02-prior-art-research.md:704-720`). What is
Rust-side is something narrower and, as it turns out, more damaging: the
**approach** — the corrective walk a mine, place, insert or take makes when
it is out of reach — is orchestrated from Rust as a *separate dispatch*, and
that is where run 9's six-minute plans went.

### 1.1 The table

| Action | Durative loop lives | State lives in | Completion signal | RCON round trips (best case) | What Rust still does |
|---|---|---|---|---|---|
| **walk** | **mod**, `on_tick` | `storage.p[idx].walking` | `writeout action_completed` from the mod | **2** (path request + start) | pathfind, arrival pre-check, wait |
| **mine** | **mod**, `on_tick` | `storage.p[idx].mining` | `on_player_mined_entity` → `action_completed` | **1** in reach, **3** out of reach | reach pre-check, corrective walk, post-walk re-check, wait |
| **craft** | **game** (its own crafting queue) | module-local `crafting_queue`, **not `storage`** | `on_player_crafted_item` → `action_completed` | **1** | wait |
| **place** | none — synchronous | — | RCON reply body, same tick | **1**, up to **11** on `§player_blocks_placement§` | reach pre-check, corrective walk, 8-point unblock probe, retry |
| **insert** | none — synchronous | — | RCON reply body, same tick | **1**, **3** out of reach | reach pre-check, corrective walk |
| **take** | none — synchronous | — | RCON reply body, same tick | **1**, **3** out of reach | reach pre-check, corrective walk |
| **research** | **game**, and **nobody waits for it** | — | none bound to an action | **1** | nothing — returns the moment the tech is *queued* |

### 1.2 Evidence, per row

**walk.** `RconActuator::walk` (`crates/executor/src/rcon_actuator.rs:263`) →
`FactorioRcon::move_player_timed` (`crates/core/src/factorio/rcon.rs:1470`).
That method asks the mod to pathfind
(`rcon_async_request_player_path`, `mods/BotBridge/control.lua:3233`; the
result comes back on stdout as `on_script_path_request_finished` and is
awaited by `sleep_for_path_request_result`, `rcon.rs:1397`, with up to four
synthesised-goal retries at `rcon.rs:2366-2379`), checks the path actually
reaches the caller's goal (`rcon.rs:1493-1508`), then sends **one**
`action_start_walk_waypoints` (`rcon.rs:2424`). The mod stores
`storage.p[player_id].walking = { idx, waypoints, action_id, idx_tick,
leg_timeout }` (`control.lua:2377`) and `on_tick` (`control.lua:720-828`) is
the follower: it aims `player.walking_state` at the current waypoint, advances
on a 0.3-by-0.3 arrival box, sizes each leg's own stuck timeout from
`character_running_speed` (`walk_leg_timeout_ticks`, `control.lua:281`),
teleports on a stuck intermediate leg (`control.lua:800-813`, with a
`teleport` writeout at `control.lua:343`), aborts on a stuck final leg, and
writes `action_completed` / `action_failed`. **Rust sends nothing per tick.**

**mine.** `player_mine_timed` (`rcon.rs:1582`). Reach is pre-checked in Rust
against the value the game reported for that player; if short, Rust dispatches
a *whole separate walk* through `move_player_timed`
(`rcon.rs:1606-1613`) — its own action id, its own path request, its own
completion — then re-measures where the bot landed (`rcon.rs:1618-1637`) and
refuses if still short. Only then does `action_start_mining` go out
(`rcon.rs:2455`). Mod-side, `rcon_action_start_mining` (`control.lua:2387`)
stores `storage.p[player_id].mining = { entity, action_id, prototype, left }`
and `on_tick` (`control.lua:834-930`) sets `mining_state` every tick, guards
`resource_reach_distance` (`control.lua:850`), classifies every non-progressing
tick into a `blocked` reason and bounds it with `MINE_BLOCKED_TIMEOUT_TICKS =
300` (`control.lua:310`, `control.lua:899-909`). Completion is
`on_mined_entity` (`control.lua:999`), scoped to `event.player_index` and
decrementing by the *actual* `event.buffer`.

**craft.** `player_craft_timed` (`rcon.rs:1661`) sends one
`action_start_crafting` (`rcon.rs:2522`). The mod calls
`player.begin_crafting{}` (`control.lua:2926`) — **the game's own crafting
queue does the durative work** — and pushes `count` entries onto
`crafting_queue[player_id]`, only the last carrying the action id
(`control.lua:2930-2936`). `on_player_crafted_item` (`control.lua:2139`) pops
the head and completes. Two facts about that queue matter: it is a **module
local** (`control.lua:61`), not `storage`, so it does not survive a save/load
and `on_load` (`control.lua:2010`) restores nothing; and matching is
**positional** — `queue[1].recipe == event.recipe.name` — with no handler for
`on_player_cancelled_crafting` (the event exists; `grep` finds no registration).
A craft that is cancelled or can never start has **no timeout of any kind**.

**place / insert / take.** All three are synchronous: `rcon_place_entity`
(`control.lua:2411`), `rcon_insert_to_inventory` (`control.lua:2646`),
`rcon_remove_from_inventory` (`control.lua:2685`) run inside the RCON command
and answer in the reply body. No action id, no `storage`, no loop. Rust's
`place_entity_timed` (`rcon.rs:1839`), `insert_to_inventory_timed`
(`rcon.rs:2032`) and `remove_from_inventory_timed` (`rcon.rs:2105`) each
pre-check reach and call `move_player` — *not* `move_player_timed` — when
short. `place` additionally, on `§player_blocks_placement§`, probes eight
compass points with `is_area_empty` (one RCON round trip each), walks, and
retries (`rcon.rs:1901-1980`).

**research.** `add_research_timed` (`rcon.rs:1087`) sends one
`add_research` and returns. `rcon_add_research` (`control.lua:2840`) calls
`force.add_research`, which per `workspace/factorio-api-docs/runtime-api.json`
(`LuaForce.add_research`) *"adds this technology to the back of the research
queue"* and returns *"whether the technology was successfully added"* — not
whether it completed. `on_research_finished` (`control.lua:2302`) does write
out, but carries **no action id**, so nothing joins it to the action.
`Actuator::research` therefore reports success the instant the technology is
**queued**, and any plan step that depends on the technology being available
runs against a belief nothing established.

### 1.3 Where the executor waits, and on what

Two different waits, and they are often conflated:

- **Between actions** — `tokio::sync::watch` per action id
  (`crates/executor/src/run.rs:63-67`), released by the dispatching task. This
  is the DAG edge mechanism and it is **not** a completion signal from the
  game.
- **For the game's verdict on one action** — `sleep_for_action_result`
  (`rcon.rs:1342`), a **50 ms poll of an in-process `DashMap`**
  (`world.actions`, `crates/core/src/factorio/world.rs:214`) with a flat
  **`ACTION_RESULT_DEADLINE = 360 s`** (`rcon.rs:38`).

The map is filled by push, not by poll: the mod `print`s
`§tick§action_completed§ok <id>` (`writeout`, `control.lua:1998`;
`action_completed`, `control.lua:2076`), the server's stdout is read by
`read_output` (`crates/core/src/process/process_control.rs:393`), and
`OutputParser` (`crates/core/src/process/output_parser.rs:226-286`) inserts an
`ActionOutcome { tick, result }`. **Client peers' stdout is `Stdio::null()`**
(`process_control.rs:511`) or a log file, never parsed — only the server peer's
`writeout` lines reach Rust.

So today's completion path is **push over the wire, poll in-process**. The only
poll that costs a round trip is the one that does not exist.

---

## 2. The proposed contract

The design is *not* "move the loops into the mod" — two of them are already
there. It is four things, in this order of value:

1. **One action registry** replacing four ad-hoc state holders, with a total
   transition function so "no verdict at all" becomes structurally impossible.
2. **The approach moves into the mod**, so an out-of-reach mine/place/insert/
   take is one dispatch instead of three, and the five `move_player` call sites
   that launder a failure's phase and ticks disappear.
3. **Research becomes awaited.**
4. **Push stays the completion path**; a poll is added *only* as a reconciler
   on the deadline, never as the primary signal.

### 2.1 Starting an action

`remote.call('botbridge', 'action_start', spec)` where `spec` is one table:

```lua
{ id = 42, player = 3, kind = "mine",
  target = { name = "iron-ore", position = {x=-40.5, y=-48.5} },
  count = 8,
  approach = { radius = 1.35 },   -- optional; nil means "must already be in range"
  deadline_ticks = 1800 }         -- optional; the mod computes a default per kind
```

The reply body carries **only** `§tick§<game.tick>` (`stamp_tick`,
`control.lua:1994`) on acceptance, or one plain-text refusal line on rejection.
That is exactly today's convention and it is load-bearing: `Dispatch`
(`rcon.rs:~585`) treats "the game answered the RPC" as the only evidence of a
dispatch, and every caller judges the reply by shape. **No diagnostics may ever
go into that body** — `rcon.print` output *is* the action's result to the
executor.

### 2.2 What the mod holds

```lua
storage.actions[id] = {
  id, player, kind, phase,            -- "approaching" | "acting" | "settled"
  started_tick, deadline_tick,        -- absolute game ticks, never seconds
  blocked_since, blocked_reason,      -- nil while progressing
  payload = { ... },                  -- kind-specific: waypoints, entity, recipe, left
}
storage.live_by_player[player] = { [id] = true, ... }
```

Three properties this must have that today's four holders do not all have:

- **In `storage`, not a module local.** `crafting_queue` and
  `recent_item_additions` (`control.lua:61-62`) are locals and are lost on
  save/load with nothing in `on_load` to rebuild them. `client_local_data` is
  marked *"DO NOT USE, will cause desyncs"* (`control.lua:55`) and that warning
  applies to any state the action machinery reads.
- **Internal sub-actions get ids from a separate namespace.** The mining
  handler's step-aside walk is dispatched today with the hardcoded id `4711`
  (`control.lua:884`), which leaks an immortal entry into `world.actions`
  (reported, unfixed: throughput note, "What is reported, not fixed" item 4).
  Sub-actions must be `{ parent = id, seq = n }` and must never write an
  `action_completed` line at all.
- **One sweep, not per-kind branches.** `on_tick` iterates
  `storage.actions`, not `game.players` — O(live actions), not O(players) — and
  the per-kind body is a function returning exactly one of
  `progressing | blocked(reason) | settled(ok|fail, reason)`. A branch that
  returns nothing is a compile-time-ish impossibility rather than a 360-second
  silence.

### 2.3 The completion signal, and whether polling is a step backwards

**Polling as the primary completion path would be a step backwards. Polling as
a deadline reconciler is a step forwards. These are different proposals and the
prior-art note recommends the first because FLE has no choice.**

FLE long-polls `get_walking_queue_length` every 0.5 s
(`prior-art-research.md:106-118`) because its **only** mod→client channel is
the RCON reply body. This project has a second, better one: `writeout` on the
server's stdout, already parsed, already carrying the real `game.tick`, and
deliberately kept out of the reply body precisely so the reply body can mean
"this action's result".

Concretely, replacing push with poll would:

- **Add a round trip per poll per action.** Four bots at 2 Hz is 8 RCON commands
  a second. Every RCON `/silent-command` is a replicated input action executed
  on every peer — the same UPS budget the inventory-shortfall note measured at
  **~53.6 instead of 60** under four clients plus frame capture
  (`2026-09-02-inventory-shortfall.md`). Spending it on polling is spending it
  on the thing already short.
- **Lose the tick.** A poll answers "at the moment you asked"; the writeout
  answers "at tick N", which is the executor's only real game clock
  (`output_parser.rs:23-25`) and the axis the whole replay/frames join hangs on.
- **Not remove the in-process poll anyway.** `sleep_for_action_result` polls a
  `DashMap` every 50 ms (`rcon.rs:1362`); that is free and orthogonal, and
  should become a `watch`/`Notify` per action id as a separate, unrelated
  cleanup.

What polling *should* be used for is the one question push cannot answer:
**"is this action still alive?"** Today, when the 360 s deadline expires, Rust
asserts `Dispatch::NoVerdict` → `Status::Lost` on the strength of silence.
Add one cheap query, called **only on that path**:

```
action_status(ids) -> [{ id, phase, blocked_reason, deadline_tick }]  -- absent id = settled or never existed
```

That converts an inference into an observation, costs one round trip per
*timeout* rather than per action per second, and is the smallest possible
version of "poll a scalar".

### 2.4 Settlement

Unchanged wire format — `action_completed` / `action_failed` writeouts
(`control.lua:2076`, `control.lua:2086`) — with one addition: the failure
reason gains a **leading machine code** from a closed vocabulary, before the
prose.

```
§4512§action_completed§fail 42 out_of_reach: 3.34 tiles from iron-ore, reach 2.7
```

`classify_failure` (`crates/scripting_lua/src/globals/record.rs:341`) currently
re-parses prose and the inventory-shortfall note calls that out; a code it can
match first, with the prose parser as fallback, is a strict improvement and
does not break the existing `transfer_guarantee_tests` contract (which asserts
the mod's *wording* whole, `rcon.rs:3600+`).

Add a third line kind, `action_progress`, for phase transitions
(`approaching → acting`), so folding the corrective walk into the action does
not erase it from the timeline (see §4).

---

## 3. Failure, timeout, and the states that freeze today

### 3.1 Every live action has a deadline, in game ticks

Per-kind, computed at start from the action's own shape — never a flat
constant:

| kind | deadline |
|---|---|
| walk | Σ `walk_leg_timeout_ticks` over legs (`control.lua:281`), which already exists |
| mine | `count × mining_time × 60 / speed`, ×3 headroom — plus the existing `MINE_BLOCKED_TIMEOUT_TICKS` blocked clock, which is *not* a mining timeout and must stay separate |
| craft | Σ recipe energy × count × 60, ×3 |
| approach phase | the walk deadline for its own path |
| research | none — see §3.5 |

**Ticks, not seconds, and the Rust side must follow.** `ACTION_RESULT_DEADLINE`
is 360 wall-clock seconds (`rcon.rs:38`). A game running at 53.6 UPS burns that
deadline 11% faster than the mod's tick budget; a paused game burns it while the
mod's budget does not advance at all. This is the same defect the
inventory-shortfall note fixed for lag edges: *a tick budget slept on the wall
clock*. The Rust deadline should become "no verdict **and** `game_tick()`
(`rcon_actuator.rs`, `Actuator::game_tick`) has passed the mod's
`deadline_tick`", with the wall clock demoted to an estimate of how long to
sleep between readings — exactly the shape `run::wait_out_lag` already uses.

### 3.2 The nil-character freeze — the reported, unfixed gap

`on_tick` gates the **entire** per-player body on
`if player.connected and player.character then` (`control.lua:716`, comment
`-- TODO FIXME`). A bot that dies, disconnects or is otherwise characterless
freezes both state machines with no verdict, and every outstanding action for it
costs the full 360 s. The throughput note lists this as item 3 of "reported, not
fixed" and says it is the shape that best fits bot 2 failing three times running
across two action types.

**Invert the gate.** The registry sweep runs unconditionally; *actuation* is what
is gated. A tick on which a live action's player has no character is a `blocked`
tick with reason `no_character`, and the existing blocked-clock machinery bounds
it. Verified on the class that actually returns it: `LuaPlayer.character` is
documented as *"Returns `nil` when the player is disconnected"*
(`runtime-api.json`, `LuaPlayer.character`) — and note that
`resource_reach_distance`, `build_distance`, `walking_state`, `mining_state`,
`mining_progress`, `selected` and `update_selected_entity` are all on
**`LuaControl`**, `LuaPlayer`'s parent, *not* on `LuaPlayer`. Reading them off
the wrong class is the mistake the brief warns about.

### 3.3 Disconnect and death are edge-triggered, not waited out

A timeout is the wrong instrument when the game will *tell* you. All of these
events exist in `runtime-api.json` and **none of them is registered today**
(`control.lua:2322-2354`):

- `on_pre_player_left_game` / `on_player_left_game` — the latter *is* registered
  but only decrements `storage.n_clients` (`control.lua:2059-2064`); it must
  settle every live action for that player with `player_left`.
- `on_player_died`, `on_pre_player_died`, `on_player_respawned` — a death voids
  everything in progress; `LuaPlayer.ticks_to_respawn` says when it comes back.
- `on_player_cancelled_crafting` — today a cancelled craft never answers.
- `on_player_controller_changed`, `on_player_changed_surface` — both invalidate
  a walk's waypoints and a mine's target.

### 3.4 A paused game

`game.tick_paused` (`LuaGameScript.tick_paused`) stops entity update, and with it
`on_tick`. Nothing progresses, nothing times out, and nothing completes. Because
every deadline above is in **game ticks**, this is correct behaviour and needs no
special case *in the mod*. It needs one in Rust, and §3.1 is it. Do not add a
wall-clock escape hatch: a paused game is a legitimate state (FLE's whole design
depends on it) and the executor should simply not be counting.

### 3.5 Research has no useful deadline and should not get one

Research duration depends on lab count, science supply and speed modules — none
of which the mod can bound in advance without modelling the factory. Bind the
action to `on_research_finished` (`control.lua:2302`) matched by technology name,
and let the *executor's* supervisor decide it has waited too long. `add_research`
raises `on_research_started` per `runtime-api.json`, which is the honest
`action_progress` signal for the transition from "queued" to "running".

### 3.6 `on_nth_tick` is not available

`script.on_nth_tick(n, f)` **replaces** the handler for `n`. 300 belongs to frame
capture (`control.lua:2354`, guarded by a comment at `control.lua:2351`), 60 to
bot sampling (`control.lua:1635`, guarded at `control.lua:1631`). The action
sweep must live **inside the existing `on_tick`**, not on a new cadence — which
is right anyway, because the walk follower needs per-tick resolution to re-aim
`walking_state`.

---

## 4. What the record gains and loses

**Gains**

- **A settle line for cases that today produce none.** Every branch of the sweep
  returns a verdict, so the class of failure that cost run 9 three
  `ACTION_RESULT_DEADLINE`s (throughput note §3) cannot be written.
- **A machine-readable failure code** in front of the prose, ending the
  re-parsing `classify_failure` does.
- **The approach becomes part of the action.** Today an out-of-reach mine is two
  dispatches, and when the walk half fails through `move_player` the `Dispatch`
  phase and `ActionTicks` are destroyed by `into_report` — the throughput note
  fixed this for `player_mine_timed` and explicitly left **five more call sites**
  doing it (place / insert / remove / build paths). Folding the approach into the
  mod deletes those sites rather than fixing them one at a time.
- **Research finally has a settle tick.**
- **Disconnect and death become named events** instead of six minutes of silence.

**Loses, unless deliberately preserved**

- **Sub-action granularity.** Today the corrective walk has its own
  `action_dispatched`/`action_settled` pair. Folding it in erases the walk leg
  from the timeline unless `action_progress` (§2.4) is emitted for the
  `approaching → acting` transition. **This is the one thing most likely to be
  dropped during implementation and it must not be.**
- **Rust-side refusals become mod-side refusals.** `walk_arrives`
  (`rcon.rs:1493`), `within_resource_reach` and the placement pre-check are
  currently Rust decisions with Rust error types
  (`RconWalkFallsShort`, `RconOutOfResourceReach`, `note_placement_refusal` and
  the refusal memory). Every one of those that moves must become a writeout, or
  the reason disappears. A mod-side decision that only `print`s is invisible.
- **`Dispatch::NotDispatched` gets rarer and `Refused` more common**, because the
  mod now judges things Rust used to judge before sending. That is a *more*
  honest classification (the game really did see it), but any dashboard counting
  `Rejected` vs `Lost` will shift and should be told.
- **The teleport record must survive.** `teleport_writeout` (`control.lua:343`)
  is the only thing that distinguishes a teleport from walking, since
  `on_player_changed_position` fires identically for both.

---

## 5. What this fixes from the three diagnoses, and what it does not

### `2026-09-02-throughput-regression.md`

| Item | Fixed? |
|---|---|
| §3 branches that neither complete nor fail | **Yes, structurally** — the sweep is a total function. (Already fixed for mining specifically; this generalises it and makes regression impossible rather than unlikely.) |
| §2 step-aside waypoint vs. reach guard | Already fixed. The design **prevents the class**: one rule for approach distance, in one place, used by both the walk and the guard that judges it. |
| Reported item 1 — five `move_player` sites laundering phase and ticks | **Yes** — the sites are deleted, not patched. |
| Reported item 3 — nil-character freeze | **Yes** (§3.2, §3.3). |
| Reported item 4 — `4711` leaks into `world.actions` | **Yes** (§2.2). |
| Reported item 2 — `record.actions` hardcodes `target: None` | **No.** Unrelated; it is a plumbing gap in `crates/scripting_lua/src/globals/record.rs`. |
| §4 — tile selector has no reachability or occupancy notion | **No, and this is important.** Moving the loop mod-side converts a planner-level collision into a mod-level `blocked` reason reported in 5 s instead of 360. That is *reporting*, not fixing. Two bots will still be sent to adjacent tiles over the same ore. |

### `2026-09-02-inventory-shortfall.md`

**Not fixed, and mostly not addressed.** That failure is a *lag edge between*
actions — a furnace smelting — not a character action, and the fix (wait on
`game_tick()`, not on converted seconds) already landed in `run::wait_out_lag`.
The one thing this design takes from it is the **unit discipline**: §3.1 applies
the same lesson to `ACTION_RESULT_DEADLINE`, which is still 360 wall-clock
seconds against a game that was measured at 53.6 UPS.

A mod-side `wait_until_tick(T)` action would be strictly better than the Rust
chase loop — one dispatch, one settle, zero polling — and is a natural extension.
It is **out of scope** here because it is a new action kind, not a relocation of
an existing one.

### `2026-09-02-mine-completion-fix.md`

**Already fixed; the design must not un-fix it.** `on_mined_entity` is scoped to
`event.player_index` and decrements by the real `event.buffer`
(`control.lua:999-1029`). Under a registry keyed by action id, the lookup becomes
`storage.live_by_player[event.player_index]` filtered to `kind == "mine"` with a
matching entity — which preserves the scoping *by construction*, but a naive
"find the action whose entity matches" sweep over `storage.actions` would
reintroduce the exact bug. Call it out in the step-2 review.

---

## 6. Alternatives rejected

1. **Poll a scalar over RCON as the primary completion path (FLE's shape).**
   Rejected — §2.3. FLE polls because it has no push channel; this project has
   one, it carries the game tick, and the RCON channel is a shared, replicated,
   UPS-consuming resource that is already scarce.
2. **Compile the whole plan into the mod and run it on `on_tick`**
   (Factorio-TAS-Generator's shape, `prior-art-research.md:416`). Rejected — no
   recovery, and replanning is this project's main asset.
3. **Pause-and-step with `game.tick_paused` / `ticks_to_run` / `game.speed`**
   (FLE's real-time model). Genuinely attractive and **orthogonal**: it would
   make every deadline exact and every measurement repeatable. Rejected *here*
   because it changes the meaning of every wall-clock number in the record and
   raises a multi-peer question (which peer pauses?) that this spec has no
   evidence about. It deserves its own spec.
4. **Keep the Rust-side approach and just re-check reach afterwards.** That is
   what exists (`rcon.rs:1618-1637`). It costs three round trips, still leaves
   the five laundering sites, and still splits one logical action across two
   records.
5. **A new `on_nth_tick(N)` cadence for the action sweep.** Rejected — §3.6.
6. **Teleporting `fast` mode as an escape hatch** (FLE has one). Rejected: the
   executor's contract is *only legitimate player actions*, and the mod already
   teleports on a stuck leg (`control.lua:800-813`), which is a bounded recovery
   with a record, not a mode.

---

## 7. Staged migration

Each step is independently reviewable, lands alone, and is verifiable by a live
run. Riskiest last.

**Remember the mod-resolution trap** (CLAUDE.md): a debug build uses
`workspace/mods` once it exists and never refreshes it. Every live verification
below must first confirm the *"Using mods directory …"* line names the copy under
test.

### Step 0 — a harness that can drive `on_tick` (no behaviour change)

`transfer_guarantee_tests` (`crates/core/src/factorio/rcon.rs:3600+`) already
loads the real `control.lua` into mlua against a stubbed Factorio API and runs a
real handler. Extend that stub with a scriptable clock and player set so a test
can drive `on_tick` over a sequence of ticks and assert the writeout lines.

- **Why first:** right now *no step below can be tested without a Factorio run*,
  which is why the throughput note's own verification section says the mod change
  "cannot be exercised by a test".
- **Verify:** `cargo test --workspace`. No live run needed.
- **Risk:** none.

### Step 1 — settle outstanding actions on disconnect, death and nil character

Register `on_player_died` / `on_pre_player_left_game`; make
`on_player_left_game` settle live actions instead of only decrementing
`n_clients`; move the `player.connected and player.character` gate from around
the whole body to around actuation, with `no_character` as a bounded blocked
reason.

- **Contract:** unchanged. **Rust:** unchanged.
- **Verify live:** start a plan, kill a client mid-walk. The action must settle
  within ~5 s naming `player_left`, not after 360 s.
- **Risk:** low. Pure addition; the failure mode is settling something that would
  have recovered, which is visible immediately.

### Step 2 — one registry in `storage`

`storage.actions` + `storage.live_by_player` replacing `storage.p[idx].walking`,
`storage.p[idx].mining` and the module-local `crafting_queue`. Sub-action id
namespace; `4711` deleted. Writeout format **unchanged**.

- **Verify live:** rerun a plan of the shape run 7 executed. `events.jsonl` must
  be identical in shape and comparable in timing; `world.actions` must end empty.
- **Verify offline:** step 0's harness, including a save/load of the stub state
  and the `on_mined_entity` player-scoping property from §5.
- **Risk:** medium. Touches every action's state. Contract-preserving, so it can
  be reverted without touching Rust.

### Step 3 — tick-denominated deadlines, failure codes, `action_progress`

Per-kind `deadline_tick`; failure codes from a closed vocabulary;
`action_progress` lines. Rust: `classify_failure` matches the code first and
keeps the prose parser; `ACTION_RESULT_DEADLINE` becomes tick-aware via
`Actuator::game_tick`.

- **Verify live:** three deliberate failures — a mine whose ore another bot takes,
  a craft with missing ingredients, a walk to an unreachable tile — each settling
  with the right code and a plausible tick.
- **Risk:** medium. First step that changes the wire format; the OpenAPI snapshot
  seam and its TypeScript mirror will need regenerating if a `FailureKind`
  variant is added.

### Step 4 — await research

`action_start{kind="research"}`; settle on `on_research_finished` matched by
technology; `on_research_started` as progress.

- **Verify live:** a `research automation` milestone. Compare the settle tick
  against `on_research_finished` in the same log.
- **Risk:** medium-high, and **not because of the mod.** A research action goes
  from ~0 to minutes, which changes the executor's schedule shape, its lag edges
  and the supervisor's stuck detector. Check those before landing.

### Step 5 — the approach moves into the mod (riskiest, last)

`approach = { radius }` on every action spec. The mod pathfinds (it already owns
`request_path`, `control.lua:3233`), walks via the existing follower as the
`approaching` phase, re-checks reach **on the tick it acts**, then acts. Rust
deletes the reach pre-checks, the corrective `move_player` calls (all five
laundering sites), and the post-walk re-measure.

- **Verify live:** a four-bot gather plan on a contested patch — the exact input
  that produced run 9. Compare against run 7's 6.1 s per successful action.
- **Risk:** highest. Subsumes the most Rust logic, touches every action kind, and
  a bug here reproduces run 9 exactly. It goes last so that when it does, steps
  1–3 mean the failure arrives in 5 s with a reason instead of in 6 minutes with
  nothing.

### Step 6 — the reconciler query (optional, separable)

`action_status(ids)`, called only when the Rust deadline expires, so
`Dispatch::NoVerdict` is a checked claim rather than an inference from silence.

- **Risk:** low, but it buys nothing until steps 1–3 have made silence rare.

---

## 8. What could not be determined without running the game

Listed because each one would change a decision above, not as hedging.

1. **Whether a per-action sweep is cheaper than the current per-player sweep.**
   `on_tick` iterates `game.players` every tick today (`control.lua:706`); the
   registry iterates live actions. Almost certainly smaller at 4 bots, but the
   constant factor of table lookups in `storage` versus a local is unmeasured,
   and `on_tick` runs on every peer.
2. **Whether every peer's `on_tick` agrees.** Only the *server* peer's stdout is
   parsed (`process_control.rs:511`), so a divergence between peers would be
   invisible until it produced a desync. Determinism says they agree; the
   `client_local_data` warning (`control.lua:55`) says the mod already has state
   that must not be read from a shared path. Whether any handler in the proposed
   sweep is peer-conditional is a fact only a run establishes.
3. **Whether `LuaControl.mining_progress` is usable as the mine's scalar.** It
   exists, is read/write `double`, and is documented as *"between 0 and 1"* for
   characters (verified on `LuaControl`, **not** `LuaEntity`, where it does not
   exist). It is the natural progress signal and the natural stall detector — but
   its behaviour when `mining_state` is aimed at an out-of-reach target is
   undocumented, and the game is known to fail that case *silently*.
4. **The real cost of an RCON round trip on this rig.** The inventory-shortfall
   note measured ~53.6 UPS under four clients plus six-camera frame capture.
   Whether a 2 Hz poll would matter — the whole basis of §2.3's second half — is a
   measurement, not a deduction.
5. **Factorio's crafting-queue semantics under `begin_crafting`.** The current
   completion match is positional (`queue[1].recipe == event.recipe.name`,
   `control.lua:2141`). Whether the game ever reorders, coalesces or interleaves
   intermediate crafts in a way that breaks positional matching was not
   established. `on_player_cancelled_crafting` exists and is unhandled.
6. **`add_research` queue ordering.** It adds to the *back of the queue* and
   returns "successfully added", so a technology queued behind another settles
   only when the queue drains. Whether `on_research_finished` arrives in an order
   the registry can match one-to-one was not verified.
7. **Whether the mod's own pathfinder can be called from `on_tick`.**
   `request_path` is asynchronous and answers via
   `on_script_path_request_finished` (`control.lua:2211`), which today writes the
   result out for Rust to consume. Step 5 needs the mod to consume its own
   answer. Nothing forbids it; nothing in this repo does it yet.

---

## Superseded: the craft rows, 2026-09-02 afternoon

Everything this spec says about **craft** describes code that no longer exists.
Kept rather than edited in place, because the spec's argument is why the fix
happened and rewriting it would erase the reasoning. Read the craft entries in
§1.1, §1.2, §3.3, §5 and open question §8.5 as *historical*.

What changed, in `d0a5e094`:

- The module-local `crafting_queue` is gone. State lives in
  `storage.craft_actions[player][recipe]`, an array of `{id, remaining}` — so it
  survives save/load, which the spec correctly flagged as the smell.
- The spec assumed the defect was the module local. **It was not.** The join was
  **positional** — `queue[1].recipe == event.recipe.name` — and a non-matching
  craft was ignored *while leaving the head in place*. That head-of-line entry
  blocks permanently: one craft that will never complete silences every later
  craft for that bot, with no timeout and no log line. Two live ways to create
  one, both now closed: a partial `begin_crafting` (which complained, refused the
  action, **and pushed all `count` entries anyway**), and cancellation
  (`on_player_cancelled_crafting` was never registered at all).
- Matching is now **counted per recipe bucket**, not positional, because
  `on_player_crafted_item` carries no request id and cannot tell two requests for
  one recipe apart. Crafts are attributed FIFO, which the code states is an
  *attribution*, not a measurement.

The evidence that made this findable: in `run-1788347034-00981` the eleven
unsettled actions were all crafts, **and every one of those crafts had
succeeded** — `samples.jsonl` shows bot 1 at 19 stone / 0 furnaces on one sample
and 15 stone / 1 furnace on the next. The game did the work and raised its event.
Only the join failed.
