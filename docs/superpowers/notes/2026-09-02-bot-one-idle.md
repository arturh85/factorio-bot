# Bot 1 was idle because freeplay hid it for 750 ticks — 2026-09-02

**Runs:** 30 (`workspace/runs/run-1788365280-15443/`) primarily, plus
`run-1788361433-78052`, `run-1788334911-41961`, `run-1788322836-81715` as the
other three instances, and `run-1788372605-35170` (run 31) as the control.
**Status:** analysis only. No production code changed. Three fixes named, none
of them in my files; the reason I did not land the one that is nearly mine is
in *What I did not do*.

## The claim holds, and it is bigger than one bot

The rung-7 note's aside —

> bot 1 never moved from `(0,0)` and got **0 of 161 dispatches**

— is exactly true. In run 30 all 161 `action_dispatched` and all 163
`action_settled` events name `bot 2`; bot 1 appears in **all 2,698 `bots`
samples**, from tick 4,320 to tick 166,140, at exactly `(0.0, 0.0)`, and in no
event of any kind. What the note missed is that **bots 3 and 4 were idle too**:
one distinct position each for the whole run, zero dispatches. Three of the
four connected clients did nothing for 162,158 ticks.

**Bot 1 is not a phantom.** `samples.jsonl` is written by the *mod*, from
`game.connected_players` (`sample_bots_body`, `mods/BotBridge/control.lua`), so
every bot in it is a player the game itself has. The server's own log agrees:
`workspace/server/factorio-previous.log` **is** run 30's, and shows four
`PlayerJoinGame … mode(create)` lines. Bot 1 was a real, connected, alive
Factorio player that was never asked to do anything.

**And it is not a recording artefact either** — but one field does lie, see
*Whose fix* §4.

## The trap first: `(0, 0)` plus that inventory is *not* a phantom signature

This is the thing to remember, because it is what made the observation look
like a live defect. `Planner::initiate_missing_players_with_default_inventory`
seeds an invented player at `(0, 0)` with
`{burner-mining-drill: 1, stone-furnace: 1, wood: 1}`. The **real** first
player of a freeplay game holds *exactly* that, at *exactly* that position, and
every run in `workspace/runs/` shows it:

| bot | position at first sample | main inventory |
| --- | --- | --- |
| 1 | `(0.0, 0.0)` | `burner-mining-drill 1, stone-furnace 1, wood 1` |
| 2, 3, 4 | `(±0.59765625, ±0.59765625)` | the same **plus `iron-plate 8`** |

`workspace/data/base/script/freeplay/freeplay.lua` explains both halves. Every
player gets `created_items` (`iron-plate 8, wood 1, pistol 1,
firearm-magazine 10, burner-mining-drill 1, stone-furnace 1`), but for the
**first** player only, `on_player_created` then runs the crash-site block:

```lua
crash_site.create_crash_site(surface, {-5,-6}, …)
util.remove_safe(player, storage.crashed_ship_items)   -- firearm-magazine 8
util.remove_safe(player, storage.crashed_debris_items) -- iron-plate 8
```

so player 1 alone loses the eight iron plates into the debris, and the pistol
and magazines never showed anyway (the sampler reads
`defines.inventory.character_main`, and guns and ammo are not in it). `(0, 0)`
is simply the spawn point; players 2–4 are offset because Factorio will not
stack characters.

**So a bot at the origin holding those three items is what a healthy first
player looks like.** The seeded phantom is observationally identical to it in
`samples.jsonl`. Neither position nor inventory can distinguish them; only the
roster warning, or the mod-sourced sample existing at all, can.

## What actually happened: the crash-site cutscene

The same `on_player_created` block ends with `crash_site.create_cutscene(player,
{-5, -4})`, and `lib.is_crash_site_cutscene` gates the exit on
`event.player_index == 1`. **Only player 1 ever gets it.** Its waypoints
(`workspace/data/core/lualib/crash-site.lua`) are `transition_time 450` +
`time_to_wait 150` + `transition_time 150` = **750 ticks, 12.5 seconds**, during
which `player.set_controller{type = defines.controllers.cutscene}` is in force
and `LuaPlayer::character` is **nil** — the character is parked in
`cutscene_character` ("the character this player would be using once the
cutscene is over").

And `rcon_players()` in `mods/BotBridge/control.lua` filters exactly on that:

```lua
for player_id, player in pairs(game.players) do
    if player.connected and player.character then
```

So for 750 ticks after joining, **player 1 is invisible to `rcon.players()`**
while players 2, 3 and 4 are visible the moment they spawn.

`scripts/research_run.lua` builds the roster it uses for the entire run from
the **first non-empty** answer to that call:

```lua
local ok, ids = pcall(function() return rcon.players() end)
if ok and ids ~= nil then
    best = ids
    if #ids > 0 then … return ids end
end
…
local BOTS = wait_for_roster(600)
…
{ bots = BOTS, stall_limit = 3, max_iterations = 10 }
```

One poll, whoever is there, frozen for 162,000 ticks. In run 30 that answer was
`[2]`.

Normally this is masked: `crates/core/src/process/process_control.rs` waits for
`rcon.connected_player_count()` to reach `client_count` before the script runs
at all, and that count goes through the same character filter — so a successful
wait *guarantees* player 1 is out of its cutscene. **The wait times out after
90 seconds**, and when it does, the mask comes off.

## Evidence: the roster the plan was made for is legible in the plan

`SplitAcrossBots` emits one share per bot in the roster, so the first
milestone's step count *counts the roster* — and it disagrees with the roster
the record reports:

| run | first milestone, 20 iron ore | roster the plan used | `plan_created.bots` |
| --- | --- | --- | --- |
| 31 (20:10) | 4 steps: `1:mine 5`, `2:mine 5`, `3:mine 5`, `4:mine 5` | `[1,2,3,4]` | `[1,2,3,4]` |
| 16:11 | 4 steps, one per bot | `[1,2,3,4]` | `[1,2,3,4]` |
| 17:03 | 3 steps: `4:mine 6`, `2:mine 7`, `3:mine 7` | `[2,3,4]` | `[1,2,3,4]` |
| **30 (18:08)** | 2 steps: `2:mine 3`, `2:mine 17` | `[2]` | `[1,2]` |

Milestone 2 (20 copper ore) repeats it exactly: four `mine 5` in run 31, `6/7/7`
at 17:03, and a single `2:mine 20` in run 30. A one-bot roster, arithmetically.

## Evidence: the timing, from the server's own log

Run 30 — `factorio-previous.log`, server started 18:06:17:

```
 94.252  UpdateTick (3511)  PlayerJoinGame peerID(1) playerIndex(0) mode(create)
101.038  UpdateTick (3694)  PlayerJoinGame peerID(2) playerIndex(1)
106.604  UpdateTick (4019)  PlayerJoinGame peerID(3) playerIndex(2)
109.620  UpdateTick (4195)  PlayerJoinGame peerID(4) playerIndex(3)
```

The first `plan_created` is at **tick 3,803**. At that moment: player 1 joined
292 ticks earlier and was 458 ticks from the end of its cutscene; player 2 had
just spawned; **players 3 and 4 had not joined at all.** The clients needed
94–110 s from server start to join, the 90-second wait had already given up, and
the script polled into that gap. Both the CLI roster (`[1,2]`, which is
`Planner::roster` reading a world that genuinely had two players) and the
script's roster (`[2]`) are honest reports of a game that was still assembling
itself.

Run 31 is the control, and it is almost too neat —
`factorio-current.log`, all four clients joined within 1.2 s:

```
 68.184  UpdateTick (3411)  PlayerJoinGame peerID(1) …
 69.424  UpdateTick (3486)  PlayerJoinGame peerID(4) …
```

Player 1's cutscene therefore ended at tick `3411 + 750 = 4161`. The connect
wait, which needs four *characters*, could not break before that tick — and run
31's first `plan_created` is at tick **4,174**, thirteen ticks later, with all
four bots in it. The wait was gated by the cutscene, released by it, and the
script planned for the full roster 0.2 s afterwards.

## How many runs

**Four of the nineteen runs with samples**, and all four carry the identical
signature — bot 1 excluded from the plan while sitting at `(0, 0)`:

| run | first-milestone shares | roster the plan used | bot 1 |
| --- | --- | --- | --- |
| 06:20 `run-1788322836` | 3 | `[2,3,4]` | idle, 1 position |
| 09:41 `run-1788334911` | 1 | `[2]` | idle, 1 position |
| 17:03 `run-1788361433` | 3 | `[2,3,4]` | idle, 1 position |
| 18:08 `run-1788365280` (30) | 1 | `[2]` | idle, 1 position |

Every other run planned for bots starting at 1. It is **not** the inverse of the
over-concentration story: at 16:11 bot 1 took 188 of 256 dispatches and at 17:03
bot 2 took 175 of 241 — the concentration is unchanged, the roster it
concentrates *within* is what lost its first entry. Two of the four (09:41 and
run 30) additionally lost bots 3 and 4, which is the same timeout with more
clients still loading.

Only the two later runs are readable as roster evidence: `plan_created.bots`
became the run roster in `b356f8ee` (12:00), so at 06:20 and 09:41 that field is
the step bots. The share arithmetic above does not depend on it and works on all
four.

## Whose fix

Three latent defects and one trigger. Any single one of them, removed, prevents
this.

1. **`scripts/research_run.lua` freezes the first answer it gets** — owner:
   `scripts/`. `wait_for_roster` returns on `#ids > 0`, so it is a race by
   construction, and the result is the roster for the whole run. The script
   already has what it needs to do better: `globals.all_bots` is the run's
   roster, so it can poll until `rcon.players()` covers it (and only then fall
   back to whoever showed up, saying so). This is the smallest and safest of the
   three.
2. **The crash-site cutscene should not apply to a bot run** — owner:
   `mods/BotBridge/control.lua`. `remote.call("freeplay", "set_disable_crashsite",
   true)` before the first client joins, or `player.exit_cutscene()` from
   `on_player_joined_game`, removes the 12.5-second hole in `rcon.players()`
   permanently. It would also give player 1 its eight iron plates back, which
   makes bot 1 stop looking like a phantom in every future record.
3. **The 90-second connect wait is the trigger** —
   `crates/core/src/process/process_control.rs:214-249`, **mine**. It gives up
   on a *fixed* wall-clock budget while progress is still being made: run 30's
   clients were joining at 94–110 s and the wait quit at ~90 s, four seconds
   before the second-to-last one arrived. The fix is not simply a bigger number
   — it should keep waiting while the count is still *rising* and give up on a
   stall, which would both have saved run 30 and abandoned a genuinely dead
   client sooner than today. It should also name who is missing when it quits.
4. **`plan_created.bots` over-reports, and it lies precisely here** — owner:
   `crates/scripting_lua/src/globals/record.rs`. `b356f8ee` changed the field
   from the bots in the steps to the run's `all_bots`, because "the bots that got
   nothing are the whole question". But a script may pass its own roster to
   `goal.plan{bots = …}`, and then `all_bots` is not the roster the plan was made
   for either: run 30 records `[1,2]` for a plan made for `[2]`, and 17:03
   records `[1,2,3,4]` for a plan made for `[2,3,4]`. The record now says a bot
   was offered work and refused it when it was never offered any. The honest
   field is the roster `goal.plan` actually used, which means carrying it out on
   the plan table rather than recovering it in the recorder.

`Planner::roster` (`4a400120`) has **no hole**. In run 30 it returned `[1,2]`
against a world that had two players, which is what it promises, and it is not
what the script planned with.

## What I did not do

I did not touch the connect wait, though (3) is in my files. Three other agents
are building in this checkout, the load average was 13.8, and `just test` is the
gate for a change to it — landing an unverified behavioural change to the
startup path while other people are doing live runs is worse than a precise
handoff. The change wanted is small and is described above.

I also did not add the `(0, 0)`-is-not-a-phantom warning to
`Planner::roster`'s documentation for the same reason. It belongs there
eventually: that doc comment is the one place a future reader will look, and it
currently says a bot at the origin is the defect's signature without saying that
a healthy first player has the same signature.
