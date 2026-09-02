# A parked bot is a permanent blocker — 2026-09-02

## Status

Done, mod-only, no Rust production change. All gates green: `cargo fmt --check`
clean, `cargo clippy --workspace --all-features --all-targets -- --deny
warnings` exit 0, `cargo test --workspace` exit 0 with **1321 passed, 0
failed**, `luac -p mods/BotBridge/control.lua` clean. 15 new tests, the two
defect ones verified red first and every fix verified non-vacuous by mutation.

Another agent was committing to `scripts/supervisor.lua` and its Rust wrapper
in this same checkout while these ran (`db5cc0bd`); the gates were green with
that work present and none of it is in this commit.

## What run 27 actually says

Re-read from `workspace/runs/run-1788353986-24634/` rather than taken from the
brief, and the brief is right on every count:

- 154 `action_dispatched`, 154 `action_settled`, **zero** `placement_refused`
  events, three failures, all identical:
  `cannot place item 'stone-furnace' because a character is standing in the
  footprint` — the transient wording `537adf30` introduced, so nothing entered
  the refusal ledger, exactly as designed.
- Milestone 4 recovered from that message after 3 iterations; milestone 6 did
  not, and its last three `plan_created` events are **byte-identical**
  30-step plans: `place stone-furnace at [2, 31]`, `[-22, 18]`, `[-23, 16]`.
- The failure at tick 27281 is action 23, `place stone-furnace at [-23, 16]`,
  from the plan built at tick 22713.

And `samples.jsonl` names the blocker without any forensics:

| tick | bot 1 | bot 2 | bot 3 | bot 4 |
|---|---|---|---|---|
| 18720 | moving | (-23.45, 25.73) | **(-23.5078125, 16.203125)** | moving |
| 31380 | moving | (-23.45, 25.73) | **(-23.5078125, 16.203125)** | (-25.44, 11.79) |

Bot 3 held that exact position from tick 18240 to the end of the run — 13,000
ticks — and every `action_dispatched` after tick 18700 names bot 1. Bots 2, 3
and 4 were parked with nothing to do for the whole of milestone 6.

A stone furnace's collision box is `±0.69921875` and a character's is
`±0.19921875` (both read off this run's own `entity_prototypes` writeout, not
from memory). The footprint at `[-23, 16]` is `[-23.699, -22.301] x [15.301,
16.699]`; bot 3's box is `[-23.707, -23.309] x [16.004, 16.402]`. Squarely
inside. The game was right every time.

## Cause seven: the planner was not wrong, it was misinformed

The brief's diagnosis stops at "the replanner correctly re-chooses the same
site — the ground is good". That is true of the *outcome* and wrong about the
*mechanism*, and the difference decides the fix. `PlanState` already has a
`characters` occupancy source, added for runs 19 and 21, and it already
excludes roster bots' own boxes. It should have refused `[-23, 16]` outright.

It did not, because of a defect one layer further out. From
`workspace/server-log.txt`, bot 3's **last** position event of the entire run:

```text
§18187§on_player_changed_position§{"player_id":3,"position":{"y":16.96484375,"x":-23.19140625}}
```

Nothing followed it. **Factorio raises `on_player_changed_position` once per
tile a character crosses, not once per position change** — visible in that same
log, where consecutive events for one walking bot are ~1.0 tiles and ~7 ticks
apart, one per tile boundary. Bot 3 crossed into tile `(-24, 16)` at tick 18187
and came to rest 0.825 tiles further on, inside the same tile. No further event
was possible.

So `FactorioWorld` believed bot 3 was at `(-23.19140625, 16.96484375)` for
13,000 ticks. And that is the difference between a plan and a stuck milestone:

- at the **believed** position the character's box is `y [16.766, 17.164]`,
  which clears the furnace footprint by 0.067 tiles. `free_area_near` offers
  the site and is reasoning correctly from what it was told.
- at the **real** position it is `y [16.004, 16.402]` — inside.

`crates/planner/tests/placement_occupancy.rs` now pins both halves of that
arithmetic. The planner needed no change at all.

This is a general fault, not one bot's bad luck. The same log's last events for
bots 2 and 4 are 0.445 and 0.297 tiles short of where `samples.jsonl` has them
standing. Every parked bot in every run has been up to a tile out, always in
the direction of its last leg, and always permanently — because a stationary
character raises no events to correct it.

## The other half: a transient with no end

`537adf30` is right and is not enough. It stopped reporting a character in a
footprint as a verdict about the ground, on the argument that *a character
moves on its own*. That argument holds for a bot that has work. It does not
hold for a bot that finished its last insert and stopped: nothing in the system
has ever asked an idle bot to move, so "transient" was a promise nobody kept.

`docs/superpowers/notes/2026-09-02-placement-precheck.md` predicted exactly
this — *"the fix makes each occurrence cost a reschedule instead of a site, but
does not make occurrences rarer"* — and run 27 is the case where the cost is
not a reschedule but the milestone, because the blocker never leaves.

One correction to the brief while here, because it changes what "recovery"
means: **`recover`'s tier 1 is not in this loop at all.** `scripts/supervisor.lua`
never calls `obs:recover()` (grep `recover` under `scripts/`: one comment). It
re-plans the whole goal through `goal.plan` every iteration and stops after
`stall_limit = 3` non-improving plans. So the thing that "re-chooses the same
site" is the planner, from `PlanState::from_world`, and the only lever on it is
what that world holds. Which is cause seven.

## The three options, and why the third one wins twice

**1. A stand-point model in the planner — rejected.**

The honest version of it is real and the precheck note is right that it beats a
wider grid constant. But it does not fit this defect in three ways. It is a
model of where a bot *will* park; run 27 needed to know where one *had* parked,
and the planner already has a source for that which was being fed stale data —
adding a second model to compensate for a broken input is how a system acquires
two wrong answers instead of one. It only covers bots parked by servicing a
machine the planner sited, and bot 3 could equally have stopped there because a
walk was abandoned. And it is a change to `crates/planner`, which is pure and
deterministic and whose site selection is currently correct; the cheapest true
statement about run 27 is that no planner change was needed.

The geometry complaint underneath it stands and is **not** fixed here: furnaces
on a 2-tile grid leave a 0.2-tile gap against a 0.4-wide character, so a bot
servicing a furnace must stand where a neighbour is going to go. Options 2 and
3 make each occurrence cost a short walk instead of a milestone, which is the
right cost; they do not make the row less crowded. If furnace-row throughput is
ever measured and the step-asides show up in it, a stand-point model is the fix
and a 3-tile constant still is not.

**2. Re-site for this plan only — rejected.**

It reopens the loop the pre-check closed. A character-caused exclusion that
lives for one expansion makes the planner flee one tile per iteration against a
stall limit of 3, and it asserts something false in the process: the site is
fine, and the plan that avoids it has learned nothing it can act on next time.
It also cannot help run 27, where the planner never knew there was anything to
re-site around. Once the position is accurate, the ordinary `characters` source
does this correctly and permanently, for free, with no new concept.

**3. Step aside — chosen, together with fixing the input.**

Two changes, both in `mods/BotBridge/control.lua`, both small:

*Report where a character comes to rest.* Every point the mod's walker lets a
character stop — an arrival, a zero-waypoint no-op, a stuck-abort, a successful
stuck-recovery teleport — now writes an `on_player_changed_position` record
carrying `player.character.position`. That is the moment a resting position
becomes a fact and the only moment anything is in a position to say so. It goes
out under the key the Rust output reader already parses, so
`FactorioWorld::player_changed_position` consumes it unchanged and **no Rust
code moved**. A walk still under way writes nothing: the game is already
raising an event per tile for a moving character, and a second source for the
same fact would be a stdout line per bot per tick for no new information.

*Ask the blocker to move.* When `rcon_place_entity` finds a character in the
footprint that is not the acting player, it now asks that bot to walk out
before reporting the refusal. This is the acting-player recovery
(`§player_blocks_placement§`, walk the actor around eight compass points and
retry) widened from "the bot I happen to be holding" to "any bot I can reach" —
the same widening `537adf30` made to the *classification*, so both halves of
that branch finally draw the line in the same place.

Four guards, each with a cost behind it:

- **Only a bot the mod is not already steering.** `storage.p[idx].walking` and
  `.mining` are how `on_tick` drives a bot through an action the executor is
  waiting on; replacing either would strand that action until the 360-second
  `ACTION_RESULT_DEADLINE` calls it lost, which is worse than the refusal being
  fixed. It is also unnecessary — a bot that is walking or mining is going to
  leave. **Only a bot with nothing to do is a permanent blocker**, and "the mod
  is not steering it" is exactly that condition. Decided and acted on inside a
  single RCON command, so it races nothing: the executor's next dispatch for
  that bot is a later command and would simply replace a walk that is nobody's
  request and has nothing waiting on it.
- **A legitimate walk, never a teleport.** The blocker goes through the same
  `walking_state` machinery as every other walk, under action id 4712 — outside
  the executor's `% 1000` id space, the same trick the mining step-aside
  already uses with 4711, so an `action_completed` for one can never be
  mistaken for a reply to a dispatched action.
- **Nothing extra reaches the RCON reply.** `place_entity_timed` reads the
  reply body as the placement's entire verdict, so a `get_player` error or a
  second tick stamp would turn a refusal that names its cause into `Unexpected
  Response`. `rcon_action_start_walk_waypoints` was therefore split: a
  `start_walk_waypoints` body that prints nothing, and the RCON wrapper that
  adds `get_player`'s reporting and the stamp. This is the same trap as
  debugging the mod with `rcon.print`, reached by accident instead of on
  purpose.
- **The transient/refusal distinction is untouched.** The reply is the same
  string it was, outside the `can_place_entity said 'no'` family
  `note_placement_refusal` matches. The placement still fails and still teaches
  the ledger nothing; the difference is that by the time anything asks again,
  the blocker is somewhere else.

Where it steps to: out through the **nearest** edge of the footprint, plus the
character's own half-width, plus a 0.4 margin, then
`find_non_colliding_position` to snap it somewhere the character actually fits.
Nearest because a step aside should be a step — run 24's blocker was 0.43 tiles
from one edge and 1.37 from the other. A `nil` answer, or a landing back inside
the footprint, dispatches no walk at all: a walk to a spot the character cannot
occupy buys a leg timeout instead of an answer.

## Determinism

`crates/planner` was **not modified** — the only change under it is three new
tests. So purity and determinism are preserved by not being touched, which is
the strongest form of that claim available. `the_same_world_sites_the_same_furnace_twice`
asserts it anyway, because the fix makes character positions arrive *more
often* than they used to and ordering is what would break first if `characters`
were ever collected off the `players` `DashMap` instead of into a `BTreeMap`.

The mod side is deterministic too, and deliberately: `placement_step_aside_target`
is pure geometry that makes no query and reads no state, and its ties resolve
west, east, north, south in that order rather than by table iteration.

## Tests

**15 new, across three files.**

`crates/core/tests/botbridge_rest_position.rs` (3, new file) — drives the real
`control.lua` in a Lua 5.4 interpreter against a stub that never moves the
character itself, so what is asserted is the mod's own choice of what to say
and when. A walk that arrives reports the **character's** position, not its
waypoint, to the digit — run 27's `(-23.5078125, 16.203125)`. A walk the walker
gives up on owes the same record. A walk still under way writes nothing.

`crates/core/tests/botbridge_placement_material.rs` (+9) — the step-aside. The
defect; the reply is unchanged (both the wording and that the body is still one
line); a blocker already walking keeps its walk **with its original action id
intact**; a blocker that is mining is untouched; a tree is asked nothing; a
character with no player behind it is skipped without raising; the acting-player
branch asks nobody to move; nowhere to stand means no walk; and the target
handed to the game is outside the footprint, on the nearest edge.

`crates/planner/tests/placement_occupancy.rs` (+3) — run 27's arithmetic. The
reported position clears the site and the real one does not, which is cause
seven in one assertion; once told the truth the site search walks past; and the
determinism control.

**Verified red first.** Before the mod change, exactly the two defect tests
failed and every control passed:

```text
a_walk_that_arrives_reports_where_the_character_came_to_rest ... FAILED   (Got [])
a_walk_that_is_abandoned_reports_where_the_character_stopped ... FAILED   (Got [])
a_walk_still_under_way_reports_nothing ....................... ok

a_parked_bot_in_the_footprint_is_asked_to_walk_out ........... FAILED   (Got [])
the_step_aside_aims_out_of_the_nearest_edge .................. FAILED   (Got [])
(the other 7 step-aside tests) ............................... ok
```

The three planner tests were green from the start and are documentation of
arithmetic rather than a regression: they say the planner was never the thing
that was broken, and `run27s_reported_position_clears_the_site_and_its_real_one_does_not`
is the test that would fail if this whole diagnosis were wrong.

**Non-vacuous, checked four ways.** Each mutation fails exactly its own tests
and nothing else:

| mutation | fails |
|---|---|
| drop the `walking == nil and mining == nil` guard | the two busy-blocker controls |
| always leave by the east edge | `the_step_aside_aims_out_of_the_nearest_edge` |
| report the waypoint instead of the character | `a_walk_that_arrives_reports_where_the_character_came_to_rest` |
| remove the `step_aside_from_footprint` call | the two step-aside defect tests |

## What I could not verify, and what is still open

- **That run 28 completes.** Everything here is from run 27's record, its
  server log, and Lua-driven tests of the real mod. A run needs
  `FACTORIO_BOT_REFRESH_MODS=1` to pick this up.
- **That `find_non_colliding_position("character", ..)` counts other characters
  as obstacles.** The same load-bearing assumption the stuck-walk teleport
  already rests on; only a live run settles it.
- **The event's exact semantics.** "Once per tile" is read off run 27's log,
  not off the API documentation, which says only that the event fires when a
  player's position changes. The evidence is unambiguous (~1.0 tiles, ~7 ticks
  between consecutive events for one walker) and the fix does not depend on the
  rule being *exactly* per-tile — only on it not being per-tick, which the log
  settles.
- **A bot that stops for a reason the mod does not drive** is still only
  tile-accurate. Walking and teleporting are the only two ways a character
  moves here, and both are covered, but a human-driven client would not be.
- **`find_entities_filtered` is now called twice** on the refusal path — once
  by `character_in_footprint` to classify and once by `step_aside_from_footprint`
  to act. Two scans of a 1.4-tile box, only when a placement was already
  refused. Left as it is so `character_in_footprint` stays the single
  classifier its two call sites share.
- **The geometry is unchanged.** See option 1 above: furnace rows still
  manufacture these collisions, and this change makes each one cost a walk
  rather than a milestone. That is the right cost, not the absence of a cost.
- **Milestone 6 also had every step on one bot.** Bots 2, 3 and 4 got no
  `action_dispatched` at all after tick 18700, which is the same
  all-work-on-one-bot pattern runs 21 and 24 showed. Not touched here, and it
  is why the blockers had nothing to do in the first place.
