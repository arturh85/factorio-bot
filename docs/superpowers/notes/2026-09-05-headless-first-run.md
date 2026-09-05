# Headless character bots: the first runs, and the settings bug they exposed

2026-09-05. Task 7 of `plans/2026-09-05-headless-character-bots.md`, picked up
after the session that wrote tasks 1-6 ended overnight without running the
feature once. Branch `headless-character-bots`, worktree `.worktrees/headless`,
own workspace and ports (34200 / 4324), never the defaults.

## What was already true before this session

The dead session left more evidence than its commits show. Its scratch server's
log holds a **character bot that walked**: a continuous run of
`on_player_changed_position` for bot 1 from (-7.5, -9.5) to (-10.5, -10.2),
ending in `action_completed ok 1` at tick 625. So the central risk the prior-art
note flagged as unverified -- that `LuaControl` on an unassociated character
answers the same calls a `LuaPlayer` does -- was already retired live. It never
reached a commit message or a note, which is why this file exists.

## The startup cost, measured

| | client run | headless run |
|---|---|---|
| launch to script running | ~26 s sprite load per client, then a connect wait bounded at 300 s | **12-13 s**, twice, consistently |

Both headless runs printed `Using bot mode characters (4 requested, ids
[1, 2, 3, 4])` and `Headless run: 4 character bot(s), no graphical client`, and
the four bots existed with `all_bots` reporting a roster of 4.

## A character bot is honestly equipped, and that broke goal_smoke

`goal_smoke.lua` asserts `p.makespan > 0` for `goal.have("iron-plate", 5)`. On
a headless run it fails, and **the failure is correct**.

`rcon_spawn_bots` gives each character the freeplay starting inventory, by
asking freeplay for it (`remote.call("freeplay", "get_created_items")`) rather
than by naming items -- the same 8 iron plates, stone furnace, burner drill and
wood a joining player gets. The goal is therefore already satisfied and an
empty plan is the honest answer.

The script's assumption comes from the other mode. With `--clients 0` there are
no players at all, so `Planner::initiate_missing_players_with_default_inventory`
**synthesises** a roster holding wood, a furnace and a drill -- and no iron
plates. That is the world in which "5 plates needs work" is true.

So this is not a regression, it is the difference between a fabricated roster
and a real one, and it is worth stating plainly: **`--clients 0` plans against
bots that do not exist, and `--headless` does not.** A script asserting a
positive makespan is asserting something about the fabrication.

## The bug this found, which predates the branch

Running any script by name against a second workspace was broken, and had been
on master.

`crate::scripting::run_script_file` called `load_app_settings()`, which reads
the **default** settings file and knows nothing about `--settings`. The rest of
the run honoured the override: the server started in the named workspace, and
the mods line named it. Only the script name resolved against the default
workspace's `scripts/`.

Both failure modes are bad, and the quiet one is worse:

- a name present in both workspaces (every script in the repo) **ran the wrong
  copy, silently** -- the file the operator edited was not the file that ran;
- a name present only in the named workspace failed with
  `path not found: headless_accept.lua`, naming a file that plainly existed in
  the workspace the run was using.

The second is how it was found. The first is how it survived: the acceptance
script for this very task was the first file to exist in one workspace and not
the other.

**Fixed by making the settings a required parameter** rather than something the
function fetches. `run_script_file` now takes `&AppSettings`, and the three call
sites (the CLI's two, the REPL's one) pass the settings they already resolved.
`crate::settings` no longer re-exports `load_app_settings` at all, so reaching
for a fresh load is no longer the easy thing in a module that already imports
that path. The REPL's command builder still loads it directly, and honestly:
it runs before a `Context` exists, so it has no resolved settings to consult,
and it only affects tab completion.

This is the `Silence is not success` pattern again, in a new place: the check
that would have caught it is "did the file I edited run", and nothing asked.

## The wall: a trigger technology never fires for a character bot

The second acceptance run (`run-1788596556-91172`, seed 31337, fingerprint
`c161fa3f437221d0`, 4 bots at 5x) executed properly -- **545 actions
dispatched, 544 settled `success`, 175 walks, 0 lost** -- and then halted
`stuck` at milestone 1 after 10 iterations, every iteration failing on
`recipe automation-science-pack is not enabled for this force`.

That message is the **mod's own** pre-flight check, not the game's, and it is
telling the truth. The chain, each link read from the record rather than
inferred:

1. `automation-science-pack` is not a recipe you start with on 2.1.17. It is
   unlocked by a **technology of the same name**, and that technology is
   trigger-based: `{"type": "craft-item", "item": "lab", "count": 1}`. The mod
   already sends this (`serialize_technology`, `types.lua`), and its own
   comment lists 32 such technologies live, including `electronics`,
   `steam-power` and `steel-axe`.
2. **A lab was crafted and placed.** `samples.jsonl` shows `lab: 1` in a bot's
   inventory; `map.jsonl` holds `{"name":"lab","position":{"x":36.5,"y":-9.5}}`.
3. **The technology stayed `researched: false`** in every force writeout of
   that run, across 9 replans and ~95,000 ticks.
4. In the client workspace's log the same technology reaches
   `researched: true`.

So: **crafting an item with a server-side character does not fire Factorio's
research trigger.** The trigger system keys off the player crafting event, and
a character with no player behind it never raises one. Nothing in the mod or
this repo can be blamed for it, and no amount of retrying helps -- the run's 9
identical replans are the executor doing exactly what it should with a
precondition that can never become true.

**This gates the whole early game**, because those 32 technologies are the
early game. A headless run can plan, walk, mine, chop, smelt, craft, place,
fuel, stock, take and insert -- all verified above -- and cannot cross the
first trigger.

### Closed: the owner took the decision, and the run went green

*"I think it sounds ok to trigger the technology when we would have gotten it
anyways."* So the mod now completes a trigger technology **when the force has
already done the thing the trigger names** -- never before it, which is the
whole of the honesty argument. `emulate_research_triggers` sweeps every 60
ticks, only while character bots exist (with real players the game does this
itself), and writes a `research_trigger_emulated` event naming the technology,
the item and the counts that earned it.

**It took two attempts, and the first one's failure is the interesting part.**
The sweep originally read only the force's production statistics. That
unlocked `electronics` (10 copper plates) and `steam-power` (50 iron plates)
and never `automation-science-pack` -- because **a hand craft does not appear
in production statistics at all**. Measured: at tick 55,200 of
`run-1788597675-93375` the force had made 202 iron plates and 71 copper
plates, both smelted, while three labs sat in bot inventories and
`production.made.lab` was absent. Smelting is counted; crafting by hand is
not.

So the mod also tallies what bots finish crafting (`tally_crafted_products`,
off the recipe's own products, at the point the craft is already detected),
and the sweep takes **the larger of the two counters, never their sum**.
Adding them would double-count anything appearing in both and could fire a
trigger before the work was done; the maximum can only fire late. A
probabilistic product counts as zero for the same reason: undercounting
delays an unlock, overcounting invents one.

Only `craft-item` is emulated. The other live types -- 11 `mine-entity`
(including `oil-processing`), `build-entity`, `capture-spawner`,
`create-space-platform` -- send no payload from `serialize_technology`,
because the shipped prototypes and the runtime API disagree about the field's
shape. Emulating a condition that cannot be read is granting it, so a headless
run still cannot cross those, and **oil is behind one of them**.

### The acceptance run (`run-1788597952-96167`)

`factory_stage2.lua`, four character bots, 5x, fresh seed-31337 map,
`outcome: done`.

| milestone | game time | iters |
|---|---|---|
| m1 automation researched | 8.5 m | 1 |
| m2 red-science cell producing 6/min | 5.5 m | 1 |
| m3 witness: 5 packs to the output chest in 90 s | 0.9 m | 0 |
| **total** | **14.9 m game time in 3.0 min wall** | |

Measured `4.98x realtime`, three milestone savepoints written, and the first
batch settled **132 of 132 with nothing failed and nothing lost**. Provenance
carries `bot_mode: characters` and `game_speed: 5.0`; the map fingerprint is
`c161fa3f437221d0`, the 31337 baseline.

The free-vision asterisk is unchanged by any of this and still applies: the
model saw 253 tiles against a furthest bot travel of 88.3, a ratio of 2.9x.

### The decision, as it was put

The only way to close it is for the mod to complete a trigger technology
itself when a character bot performs the qualifying action -- watch for the
craft, then set `researched`. That is **emulating what the game would have
done for a player**, not granting free research, but it is the mod deciding a
technology is complete, and this project's rule is that a measured run must be
as cheat-free as it can be. So it wants an explicit decision, and if taken it
wants a provenance field, so no headless number is ever quoted beside a client
one without it.

Until then the honest description of the mode is: **headless is for planning,
for pre-trigger execution, and for iterating on either -- not for a milestone
that crosses a trigger technology.**

## Status

- Tasks 1-6: implemented by the prior session, committed.
- Task 7 step 1 (build): done.
- Task 7 step 2 (four bots, roster, plan): **done for spawn, roster and
  startup**; the plan assertion is discussed above.
- Task 7 step 3 (a full executing run at 5x, with `just analyse`): **not run
  yet.** Held while the speedrun session measured run 12 on the same machine --
  a 5x server with four bots competes for CPU the way a cargo build does, which
  this repo has already paid for once.
- The settings fix compiles in `--features cli,lua`; the `--all-features` and
  test runs are held for the same reason.
