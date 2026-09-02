# Overnight report — 2026-09-02

Priority order you set: **reliability first, watchability second.** Both moved.

## Where the ladder stands

| rung | state |
|---|---|
| 1 gather iron ore x20 | **satisfied, 1 iteration** |
| 2 gather copper ore x20 | **satisfied, 1 iteration** |
| 3 smelt iron plates x10 | **satisfied** |
| 4 research automation | **plans 114 steps**, executes deep into them, not yet closed |
| 5-7 power, belts, oil | not reached |

At midnight this system crashed on rung 1 because a guard compared whole
inventories. It now dispatches 200+ actions per run at ~99% success across
mining, smelting, crafting, placing, fuelling, inserting and researching.

**I did not launch a rocket and was never going to.** I said so before you slept
and it remains true.

## Twenty defects fixed, every one found by a live run

The ones worth knowing about:

1. **Telemetry killed the game.** `character.mining_target` belongs to
   `LuaEntity`, not a character — my own code, and it only fired the first tick a
   bot really mined, so every planning-only test passed.
2. **Bots did not walk.** The "stuck" check measured leg duration, so every path
   leg over ~9.2 tiles was teleported by an unbounded jump, and a sibling branch
   reported the walk *successful* while leaving the bot walking forever. Every
   walk duration in every earlier record was fiction.
3. **A mine action reported success at half its count** — `on_mined_entity`
   completed *any* bot's task matching the entity, so two bots on one tile
   decremented each other. Fixing it made rungs 1-3 twelve times faster.
4. **A forest read as open ground.** `is_area_clear` consulted the tree holding
   factory entities; trees and cliffs live in another whose only reader was the
   debug drawing code. 10,510 tree records arrived every run and nobody asked.
5. **Machine time was measured in game ticks and waited for in wall-clock
   seconds.** Equal only at 60 UPS, which our server does not sustain — because
   we screenshot six 1920x1080 cameras every five seconds.

## A tension you should decide, not me

**The watchability feature is degrading the reliability one.** Frame capture is
why the server runs at ~53 UPS instead of 60, which is what broke the smelt
timing. Fewer cameras, lower resolution or a longer interval each buy back
server speed at the cost of the video.

## Watchability: the record now diagnoses failures by itself

Run 13's placement failure was diagnosed **from the record alone** — the target
tile from `events.jsonl`, the model ruled out by `map.jsonl`'s keyframe
agreement, and the cause named by `samples.jsonl`: a bot parked motionless
inside the furnace's footprint. No code reading required.

Failures went from an 18-minute silent hang to
`expected coal at (-37.5/5.5), found crash-site-spaceship-wreck-medium-3`.

Both deep runs render at `http://127.0.0.1:7500/#/runs`:

| run | frames | samples | map | events |
|---|---|---|---|---|
| `run-1788320177-77989` | 1217 | 1218 | 20 | 304 |
| `run-1788319014-01846` | 260 | 313 | 4 | 93 |

`skipped: 0` on every stream.

## Open, ranked

1. **Rung 4's placement refusal** — third distinct cause behind one message,
   under diagnosis now.
2. **Nothing retires a mined-out tile**, so exhausted ore is offered forever.
3. **A refusal is discarded** — the replanner re-chose the same refused tile
   twice.
4. **`Researched` sizes its whole bill against one bot**; option 3 from the
   share-sizing spec (a real multi-bot decomposition) remains the honest fix.
5. **`MapKind::Removed` has no writer**, documented as reserved.
6. **`wall_ms` in `events.jsonl` is badly wrong** (33780 to 738866 while the
   tick moves 10).

## Two process notes

- `git reset --hard` and `git stash` both destroyed or endangered another
  agent's work in this shared checkout. Use a worktree.
- A spurious STALE warning about `workspace/plans` — a directory with no
  production reader — trained me past the real one about `workspace/scripts`,
  which made a "teleports: 0" I reported to you meaningless.


---

## Update (~10:30): a change of tack on rung 4

`cannot place item 'stone-furnace' because surface.can_place_entity said 'no'`
has now failed **four** runs, and **three distinct causes have been found and
fixed behind it**:

1. a forest — `is_area_clear` read the tree holding factory entities, not the
   one holding trees and cliffs;
2. a non-roster character parked in the footprint;
3. a roster character, after a filter whose premise turned out to be false
   (`BotState::position` never advances during expansion, so the observed
   position is the only position the planner has for *any* bot).

Run 16 hits it again with all four bots connected and 36 of 40 actions
succeeding. Each fix was correct; each exposed another cause.

**I stopped hunting cause four.** Three correct fixes behind one message is
evidence the model cannot reliably predict `can_place_entity` — not evidence
that a fourth fix finishes the job. The structural gap, flagged twice tonight
and deferred both times, is that **a refusal is information the planner
discards**: the replanner re-chose the same refused tile twice in one run, so a
single bad site can consume every iteration.

The open question I asked to be reasoned about rather than assumed: **how long
should a refusal be believed?** A tile refused because a bot stood on it is
valid the moment that bot walks away; a tile refused because a cliff is on it
never is, and the message does not always distinguish them.

## The phantom bot, now with three effects

`initiate_missing_players_with_default_inventory` invents a player at `(0,0)`
when fewer clients connect than requested — which happened in most runs tonight.
It shadows a collision box at the origin; it drags the start-of-run keyframe's
bounds to the origin, so the map covers the wrong region; and it sits in the
roster as a bot that can never act. Two agents flagged it and both said the real
fix is upstream. Run 16 had all four connect and did not have it.

## A record semantics trap

`plan_created.bots` is derived from the **steps**, not the roster. `bots: [2]`
does not mean the roster was one bot. Anyone reading that field as a roster —
including a future diagnosis — will be wrong.


---

## Update (~11:30): run 17, the deepest run yet

`run-1788325660-10154`:

```
mine 128 · craft 57 · place 44 · fuel 42 · insert 16 · take 12 · research 1
```

**300 actions dispatched, 297 succeeded, 3 failed.** 157,080 ticks — about 44
minutes of game time — 2.5 MB of samples, 68 recorded teleports, 2 recorded
placement refusals. Rungs 1-3 each satisfied in one iteration.

**The refusal memory worked exactly as designed.** Iteration by iteration:

```
success=4  failed=0  (+0 refusals)
success=4  failed=0  (+0 refusals)
success=40 failed=1  (+1 refusals)   <- learned here
success=51 failed=0  (+0 refusals)
success=43 failed=1  (+0 refusals)
success=38 failed=0  (+0 refusals)
```

One refusal, then none across three further iterations. Every earlier run
re-chose the same refused ground until it ran out of attempts.

Rung 4 then crashed on something new:

```
bot 1 owns chain ChainId(2) because its bill was sized against it,
but between 1.2705824974445776 and 10 of [-16, 18] does not hold there
```

A **fractional lower bound on a resource count**. Counts are integers; a float
with that many digits is arithmetic, not a quantity. Under diagnosis, with the
already-twice-reported exhausted-tile gap as the leading suspect — this run
mined 128 times, which is exactly where a belief that mined-out tiles still hold
ore would surface.

Note the error message itself is one this work rewrote to be truthful: it says
*why* the chain is bound to that bot rather than claiming a caller named it.


---

## Update (~12:30): run 18, and the last known blocker

Run 17's crash was **one ulp**. `1.2705824974445776` was not a resource count —
it is `placement_clearance("stone-furnace")`, a distance in tiles, bit-exact
against the prototype fixtures. `arrival_point` returned `to.x() + min_radius`,
a rounded sum whose distance measured back out came to `...772` against an
inclusive inner bound of `...776`, so **the scheduler rejected the arrival point
it had just computed**. Fatal only because the share-binding fix gives a chain
an owner, leaving no second candidate. Two of tonight's own fixes interacting.

That diagnosis also killed my exhausted-tile hypothesis with evidence: 128 mine
dispatches at 128 *distinct* tiles, at most 5 each against a modelled 500.

**Run 18** (`run-1788329146-40305`): 175 actions, **172 succeeded**, rungs 1-3
satisfied, rung 4 running **8 iterations of 89-step plans** — the most
consistent run yet — before:

```
ERROR: could not start mining for 301 ticks: another character is standing on the copper-ore
```

That is a gap the obstructed-tiles work named and **deliberately left open**,
for a good reason: `resource_unclaimed` ignores characters because *a miner
legitimately stands on its own target*, and a blanket rule would fence every bot
off the tile it was sent to. The distinction is whose feet.

Likely our own doing again: split capacity sizes rung 4 to fewer bots than the
roster, so bots are **parked**, and a parked bot standing on ore is invisible to
a selector that only reasons about miners.

## `wall_ms` removed rather than left wrong

It stamped the moment `record()` was *called*. The supervisor flushes a whole
plan's events in one batch after execution finishes, so every event in a batch
carried the batch-flush time — the `33780 → 738866` across 10 ticks. Making it
truthful needs real timestamps captured in the executor and threaded across the
mlua boundary. Nothing read it, so it went.

## Update (~afternoon): the 2.1 audit closed, and rung 6 diagnosed

The Factorio 2.1 API audit is now fully acted on except one item.

- **A2 `build_mode`** — `force_build = true` maps to `defines.build_mode.forced`;
  the inherited default was `normal`, the opposite of intent. `build_blueprint`
  has no `force_build` parameter at all. Note that `forced` also deconstructs
  obstructing nature entities, which 1.1's `force_build` did not — a real
  behaviour change, not a rename.
- **A3 placement destroyed materials** — `remove_item` returns how many it
  *actually* removed and `create_entity` returns an *optional* entity; both
  return values were discarded, so a failed placement ate the item and a no-op
  removal built for free. Now create → charge → `destroy()` if the charge did not
  take exactly one. The reverse order needs to refund through `insert`, which
  also returns a count and can fall short: a refund that silently loses material
  is the same bug with more steps.
  `charge_item_to` did **not** fit — it spends from `get_main_inventory()` while
  the affordability guard above uses `get_item_count`, which spans every
  inventory. Pairing them would check one set and spend from another.
  The new failure line is deliberately in the *material* family and does not
  contain `can_place_entity said 'no'`, so the refusal memory added last night
  will not fence the planner off ground the game never refused.
- **B1 unfiltered entity queries** — the keyframe query is now filtered to the 16
  types `EntityGraph` models. `attach_world`, `is_area_empty` and the placement
  obstruction checks were deliberately left unfiltered: they feed `blocked_tree`,
  and narrowing them would reintroduce the forest-siting bug. Both calls moved to
  `remote_call_json`, closing a latent pool bug where a truncated reply left the
  pooled connection holding an unread remainder.
- **A6 `needs_destroy_to_reach`** — carried through the audit, still not acted on.

**Rung 6 was a locked recipe, not a split inventory.** The leading hypothesis was
that ingredients had landed on the wrong bot. `samples.jsonl` refuted it in one
query: every bot held both ingredients in its own inventory, and bot 2's is
byte-identical across the whole six-minute milestone — nothing needed moving and
nothing consumed anything. `events.jsonl` named the real defect instead: of four
`craft automation-science-pack` nodes, only one had `deps`.

`automation-science-pack` is `enabled = false` in 2.1.17, unlocked by a 2.0
trigger technology. The planner states that precondition for the *first* share
and applies `Effect::Researched` to its own overlay; the other three then asked
`recipe_gate`, which read the **union of world and overlay** and answered `Open`.
`Open` means "nothing has to happen first", so they carried no condition,
`infer_edges` had nothing to hang an edge on, and all three dispatched at tick 0.
`Open` was doing double duty — "already true in the world" and "will be true by
then" — and only the first is safe to skip an edge for.

One existing test had encoded the bug, asserting `Open` for an
overlay-researched unlocker. It is now split into an overlay case and a world
case, so the fix cannot be satisfied by never reporting `Open` at all.

## The next rung was never actually reachable

Independent of rung 6, the mod-side-actions spec found that **research is
durative and nobody waits for it**: `Actuator::research` reports success the
instant it sends the command, and `on_research_finished` carries no action id to
correlate a completion back. Rung 7 is "research automation" — so it could never
have worked, and a green rung 7 would have been a false green. Being blocked at
rung 6 hid that.

The same spec falsified a premise I had been carrying: walk and mine are
*already* mod-side `on_tick` state machines with push completion carrying a real
`game.tick`. Polling would be a step backwards. The real remaining defect there
is that the approach walk is a separate dispatch laundered through `move_player`
→ `into_report` across five call sites — which is where run 9's three
360-second plans went.

## Video, and the clock it needs

The question "can we just record a video?" is a good one, and the answer turns on
the clock. `take_screenshot` renders **synchronously inside the game's main
loop**, once per camera per capture; an external recorder captures the frame the
GPU already drew. So video is strictly cheaper on the game thread.

What it costs is the join. Everything in the viewer joins on `game.tick` —
frames are literally named `tick-<digits>-<camera>.jpg`, and `frameJoin.ts`
*verifies* the join rather than assuming it. Video frames are wall-clock, and UPS
is not constant (it sagged to ~53 under capture, exactly the condition you would
be recording in), so the two cannot be related by multiplying.

**Corrected an hour later, by the spec:** I wrote here that the sidecar should be
written *by the mod at a known `game.tick`*. The mod **cannot do that at all** —
Factorio's control stage has no clock. All 157 classes of `runtime-api.json` were
searched; the only real-time source is `LuaProfiler`, which explicitly refuses to
yield a number to Lua and can only be rendered into a `LocalisedString`. The
clock has to be built host-side, which turns out to be strictly better anyway:
ffmpeg and the tick sampler then share one monotonic clock with no cross-process
skew. Every RCON reply already carries a `game.tick` stamp (`rcon.rs:711`).

The `wall_ms` lesson survives the correction, but it is about *where* the stamp is
taken, not who takes it: `wall_ms` was removed because it stamped the moment
`record()` was called, and the supervisor flushes a whole plan's events in one
batch, so a batch's events all carried the flush time. Timestamp at the event,
not at the flush.

**Also corrected: frames ARE in the analysis path**, in exactly one place I had
not looked. `runTimeline.ts` `tickSources` pushes frame ticks into the `drawn`
set, and `tickBounds`/`leadInTicks` use `drawn` to decide where the axis starts —
the lead-in trimming built yesterday. So a video-only run moves the analysis axis
**silently**: no error, nothing marked. My claim that moving frames to video
costs the analysis path nothing was wrong. It is still true that no run has ever
been *diagnosed* from a picture; that is a different statement, and I generalised
it past what it supports.

**Unmeasured, and load-bearing:** nobody has A/B'd what screenshot capture
actually costs. The "~7 UPS from six cameras" figure is inference. Any argument
for video that leans on it is leaning on an unrun experiment.

## Update (~12:45): run 23 — four rungs faster, one rung spinning

`run-1788344167-58471`, the first run with all five of today's fixes together
(recipe ordering, tile retirement, roster, awaited research, teleport collision).
Killed at 31 minutes rather than left to burn its 90-minute timeout, because the
record showed it could not progress.

**What got better, measurably:**

| rung | before | after |
|---|---|---|
| smelt copper x20 | 3 iterations, 15 teleports | **2 iterations, 0 teleports** |
| craft gears x20 | 2 iterations (3 then 4 actions) | **1 iteration, 12 actions** |

The load-bearing unknown from the teleport fix is answered: Factorio's
`find_non_colliding_position("character", ..)` **does** count other characters as
obstacles. No two bots shared a tile at any point in this run. The stacking bug
is dead.

**What replaced it: a spin.**

```
1414 teleport events.  All bot 1.  All walk_stuck.
All to the identical destination (-22.0, 19.0).
From two alternating positions, every 61 ticks,
tick 20385 -> 108151 = 87,766 ticks on one action.
```

Rung 6 iterations read `success=1 pending=35 (+353 teleports)`.

The likely mechanism: the teleport now lands the bot at an *adjusted* position
while arrival is still judged against the *original* waypoint. Before, the
teleport put the character exactly on the waypoint and the next tick's arrival
check advanced the leg. Now it lands 0.5 tiles off, arrival never fires, the leg
times out again, `find_non_colliding_position` is deterministic and returns the
same spot, forever.

**Two defects, not one.** The adjusted-position mismatch is the trigger, but the
reason it cost 87,766 ticks instead of one wasted attempt is that **nothing caps
the retries**. The abort arm added with the fix covers "nothing free"; it does
not cover "teleported and it did not help".

**And the new diagnostic could not see it.** `obs.walks_failed` and the
`first_error` fallback both key off a walk *failing*. This walk never failed — it
never terminated. A spin is invisible to an instrument that only fires on
failure, which is the same shape as `stuck_silent` reporting no error: the
absence of a verdict read as the absence of a problem.

**Honest ladder position: still 5 of 7.** Four fixes verified good, one fix
traded a permanent brick for an unbounded loop. That is progress — a spin is
recoverable and a brick is not — but it is not a rung.

## Correction: why 30 of 33 steps land on one bot

I have said twice today that this is the reported gap where `Researched` sizes
its whole bill against a single bot. That is a true description of the code and a
false account of the cause, and it was measured rather than argued.

Counterfactuals on a four-bot fixture:

| scenario | distribution | makespan |
|---|---|---|
| as-is, technology locked | 49 / 12 / 12 / 12 | 15922 |
| technology already researched in the world | 12 / 12 / 12 / 12 | 2304 |
| `Goal::Researched` hoisted to a top-level sibling | 49 / 12 / 12 / 12 | 16832 |

Hoisting the goal clear of the share moves the distribution by **zero actions**
and makes the makespan *worse*, because the hoisted chain stops sharing
intermediates. Rewriting those `Holder::Share`s to `Anyone` would change nothing
— no method claims them at that site.

What actually binds it: the chain is opened by the first share to expand
(`SplitAcrossBots` emits in ascending `BotId`, so bot 1), `HandCraft` meets
`NeedsResearch` inside that chain, and nothing beneath a chain can re-own itself.
The sizing is downstream of that, not the cause of it.

**The real constraint is that a lab is one craft.** Its ~50 iron and ~16 copper
plates must be in one inventory at one moment, and this planner has no way for
one bot to hand an item to another. 86% of the fixture's makespan is the unlock;
8280 of bot 1's 15922 ticks are mining, while bots 2-4 finish around tick 4400
and idle for eleven thousand.

Also corrected: I cited "option 3 in the share-sizing spec". It is not there —
that spec explicitly puts research out of scope. The options are in
`2026-09-02-rung-3-4-findings.md`, which already called option 3 "a larger change
than a one-night fix". This work is the measurement that note asked for.

**Not fixed, deliberately.** Two designs would work — a furnace-as-buffer split
(supplier and taker need not be the same bot, since `Smelt`'s `Remove` carries no
`HasItem`), or a bot-to-bot transfer primitive. Both are real changes, and this
is a parallelism problem, not a reliability one. Reliability is the stated
priority and the ladder is not blocked on this. It is a decision to put to the
project owner, not a ruling to make at 13:00 while a run is in flight.

## Update (~13:35): run 24 — rung 6 falls, and rung 7 is reached for the first time

`run 24`, built from a clean worktree at `7da3d5e9` (the teleport-spin fix)
because another agent was mid-edit in `crates/planner/src/method/have.rs` —
building from the shared tree would have compiled unknown in-progress work into
the run.

**The ladder is at 6 of 7.** Rung 6, `craft automation science packs x10`, is
satisfied for the first time since it was added:

```
ran: success=32 failed=4 lost=0 pending=0
ran: success= 8 failed=1 lost=1 pending=35
ran: success= 0 failed=4 lost=4 pending=0
SATISFIED
```

Rung 7, `research automation`, planned **114 steps** and is executing. It has
never been reached before, and until this morning it could not have worked at
all — `Actuator::research` reported success the instant it sent the command, so a
green rung 7 would have been a false green.

**The teleport numbers, before and after:**

| | run 23 | run 24 |
|---|---|---|
| teleports, whole run | 1414 | **2** |
| iterations with zero teleports | — | 8 of 10 |
| ticks spent on one spinning action | 87,766 | none |

Rungs 1-5 kept run 23's improved profile: copper in 2 iterations, gears in 1.

**What is still failing, and it is the old acquaintance:**

```
first error: cannot place item 'stone-furnace' because surface.can_place_entity said 'no'
```

That is the failure that has now ended or damaged six runs across five distinct
causes. The refusal memory added last night keeps a replan from re-choosing the
same refused site, but nothing yet asks the game *before* committing — the
pre-check the refusal-memory note argued had to come second, because the planner
is pure and needs somewhere to put the answer. That somewhere now exists.

## Update (~14:15): run 24 ended at rung 7, and cause six is ours

Run 24 (`run-1788347034-00981`) reached rung 7 and ground at it: plan size fell
114 → 113 → 103 → 102 → 100 → 99 → 97 across seven iterations, which **alternated**
between roughly fifteen successful actions and *zero*. I stopped it 17 minutes
short of its timeout once the pattern was established and its cause fixed.

**Six rungs of seven, and rung 7 reached for the first time.**

### The alternating dead iterations

Every zero-success iteration contributed exactly `+1 events` — one dispatch, no
settlement. The cause is now understood, and it is our own:

The planner sites furnaces on a **2-tile grid**, which leaves a **0.2-tile gap**
against a **0.4-wide character**. A bot servicing its own furnace therefore
cannot stand between them. At tick 11520 bot 3 sat at `(-21.47, 23.73)` — inside
the footprint bot 4 was about to build on, parked since servicing its own
furnace. **The plan manufactured its own blocker, 2,400 ticks after the
pre-check passed on that ground.**

### Why a transient became permanent

`rcon_place_entity` recognised only the **acting** player standing in a
footprint. Any *other* character fell through to the generic
`can_place_entity said 'no'` — which is exactly the wording
`note_placement_refusal` matches. So good ground entered the never-expiring
refusal ledger.

The worse half: **recording the refusal suppressed the one recovery that fits.**
`recover`'s tier-1 reschedule is skipped for a refused footprint, and "wait, the
bot will walk away" is precisely what tier 1 is *for*. A refusal that disables
the correct recovery is worse than no refusal at all.

The pre-check path had this right all along (`rec.character` /
`is_durable_refusal`); the dispatch path drew a narrower line. **Two call sites,
one concept, different answers** — cause six lived in the gap. Now three
branches: acting player (walk aside and retry), any other character (transient),
anything else (a real verdict about the ground).

### What the fix does not do

It makes each occurrence cost a **reschedule** instead of a **site**. It does not
make occurrences rarer — the 2-tile grid still manufactures them. The honest fix
for that is a stand-point model, not a wider spacing constant, and it is not
written.

### The pre-check was already built

I dispatched an agent to build it. It had been committed at 09:32 (`d0db355c`)
and was live in run 24 — it answered correctly and the failure happened
downstream of it. I did not check the log before dispatching. The agent verified
rather than rebuilt, which is the only reason that cost an hour of one agent
instead of a duplicate implementation.

## Update (~14:20): the record could not report a lost action, by construction

The settlement hole is fixed (`fcb4ed68`), and both the mechanism and my reading
of it needed correcting.

**My reading was wrong in one specific way.** I said `failed=1 lost=1` meant two
things went wrong. It names **one** action. `scripts/supervisor.lua` sets
`t.failed` to a *sum* — `obs.failed + obs.lost + obs.walks_failed +
obs.walks_lost` — and `research_run.lua` prints that sum under the label
`failed=`. So those lines mean `obs.lost == 1` and everything else zero. There
was no invisible `Status::Failed` in that run at all. The mislabelled `failed=`
is its own defect, reported and not yet fixed.

**Where the action escaped: not the executor.** `crates/executor/src/run.rs`
already reaches a terminal status for everything it dispatches. The escape was
one layer out, in `record.rs`: the settle was written inside
`if let Some(replied) = replied`. `replied_tick` is a *measurement*, and it is
correctly `None` when the game never said anything — so **a `Status::Lost` action
could never produce a settle, by construction.** Not a missed branch; a shape
that made the case unrepresentable.

The fix keys the settle on the **verdict** instead: success, failed and lost each
write exactly one line; pending and running write none. With no reply tick the
stamp is `not_before(dispatched)` rather than the dispatch tick, which would fake
a zero-duration reply, and `elapsed_ticks` is `Some` only when both ends were
measured — so a synthesized stamp always carries a null duration and cannot be
misread as a timed span.

**Third instance of the same shape today, and this time it was actions.**
`build_observation`'s `first_error` came from `failures`, which holds
`Status::Failed` only — deliberately, pinned by a test saying "losing the thread
is not a failure". So a lost *action* contributed nothing to `first_error`
either. Every milestone-7 iteration had `obs.failed == 0, obs.lost == 1` and no
failing walk, so `first_error` was `nil`: had milestone 7 halted, `milestone_stuck`
would have written `last_error: null` **again, after both walk fixes**. Now
`first_lost_error` falls back: failed action → lost action → failed-or-lost walk.

**And the finding that matters most: all nine unsettled actions were `craft`.**

```
plan  8  ms 6   craft 1 stone-furnace
plan  9  ms 6   craft N automation-science-pack   (x4)
plan 10-13 ms 7 craft 1 stone-furnace             (x4)
```

No mine, no place, no insert, no walk. Crafting is the one action class that
never reports completion, so each costs a full 360-second deadline before being
written off. That is almost certainly what rung 7 has been grinding against, and
it is the same defect research had this morning — durative work nobody waits for.

**Not fixable: joining a teleport to an action.** `teleport.action_id` is the
mod's run-global id; `action_dispatched.id` is the planner's plan-local
`ActionId`. Worse, a `walk_stuck` teleport belongs to a *walk leg*, and walks
have no `ActionId` at all — there is no walk `EventKind`, and `obs.walks` never
reaches `events.jsonl` even though walking is most of the wall-clock. The
question has no answer in this id space. Renaming would hide that rather than fix
it.

## Update (~14:45): run 25 — the cleanest rungs yet, and a shallower finish

Run 25 (`run-1788351494-76427`), built at `537adf30` (the parked-bot transient
fix). **5 of 7**, which is one rung *worse* than run 24, and simultaneously the
best-behaved run there has been:

```
gather iron ore x20      3 actions   0 failed   1 iteration
gather copper ore x20    3 actions   0 failed   1 iteration
smelt iron plates x50   18 actions   0 failed   1 iteration
smelt copper plates x20 22 actions   0 failed   1 iteration
craft iron gear wheels  11 actions   0 failed   1 iteration
```

**Every one of rungs 1-5 in a single iteration with zero failures, and zero
teleports across all 16 iterations** — against 1414 teleports two runs ago and
rung 4 needing three iterations with a refusal.

### What actually happened

```
12:18:14  client 4 times out; roster plans for 3 of 4 and names the absent id
12:18:32  rungs 1-5 satisfied
12:29:30  rung 6: could not have player client3 craft 4 automation-science-pack (but only 0)
12:43:16  RAISED: bot(s) 1, 2, 3 are not connected players in this world
```

Two of those lines are fixes from today working exactly as designed:

- `planning for 3 of 4 bot(s): the game has no player for [4], so nothing will be
  assigned to it. A client that never connected is not a bot.` — the phantom-bot
  fix, naming the absent id instead of inventing a bot at `(0,0)`.
- The raise at 12:43 is the guard **refusing to plan against a fabricated
  inventory and guessed reach distances** once the remaining clients dropped.
  Failing loudly there is right; the alternative is a plan built on invented
  bodies.

### The real defect, and it is not in the bot

**The three surviving clients died mid-run**, somewhere between 12:29 and 12:43.
Nothing in this repository knows why, because `--logs` was not passed, so no
client log was written — `workspace/client1-log.txt` is from 31 August. A
Factorio client dying during a run is a first-class reliability problem and it is
currently **undiagnosable by construction**.

*Every subsequent run passes `--logs`.* That is the cheapest possible fix and it
should have been on from the start.

### Rung 6's own failure is separate and still open

`could not have player client3 craft 4 automation-science-pack (but only 0)` with
a three-bot roster. Bot 3 ended holding 3 copper-plate and 3 iron-gear-wheel —
enough for three packs, not four — while bot 1 held a finished `lab` and 20
iron-plate. That is the "a lab is one craft, and no bot can hand an item to
another" constraint measured earlier today, now reached from the other side: the
shares were sized against a four-bot roster's materials and executed by three.

Also still true in this build: **102 dispatched against 98 settled, and all 98
settled are `success`.** The four lost actions are invisible, because this binary
predates `fcb4ed68`. The next run is the first to carry the settlement fix.

## Update (~15:10): run 27 — two fixes proven, and the blocker moves to geometry

Run 27 (`run-1788353986-24634`), built at `d0a5e094` on a quiet machine (load
1.71 at run start, with a load gate in the run script after run 26). **5 of 7**,
stuck at milestone 6.

### Two fixes proven live

**Awaited crafts work.** `lost=0` in every iteration of the run. The previous run
had eleven lost crafts, each burning a 360-second deadline. Zero now.

**Every action settles.** 154 dispatched, 154 settled. Run 25 had 102 dispatched
against 98 settled with all 98 reported `success` — four actions simply absent.
That hole is closed.

**And the transient classification is right.** `placement_refused: 0`. The three
failures are all the new message:

```
cannot place item 'stone-furnace' because a character is standing in the footprint
```

so a parked bot no longer poisons the durable refusal ledger, and tier-1
reschedule stays available. Milestone 4 **recovered** from exactly this and was
satisfied after three iterations.

### Why milestone 6 still stuck, and why every layer was correct

Milestone 6's last three iterations read `success=0 failed=1 lost=0 pending=30`.
One placement fails and thirty actions wait behind it.

Every layer behaved as designed, which is the point:

- The mod correctly says *a character is standing here*, not *this ground is bad*.
- Because that is not a refusal, the site is not blacklisted.
- The replanner therefore re-chooses the same site — **correctly**, the ground is
  good.
- **But the blocking bot is idle.** It parked after servicing its own furnace and
  has no reason to move, ever.

So the retry is right, the site is right, and nothing changes. A perfect
transient that never resolves.

The placement note predicted this in as many words: *"the fix makes each
occurrence cost a reschedule instead of a site, but does not make occurrences
rarer."* The 2-tile furnace grid leaves a 0.2-tile gap against a 0.4-wide
character, so a servicing bot **must** stand where the next furnace goes. The
layout manufactures the collision.

That is now the top item, and it is the first blocker in days that is a design
question rather than a bug.

## Update (~15:50): every parked bot has been up to a tile wrong, always

The parked-bot blocker is fixed (`c99e2ce2`), and the cause was one layer below
where I had stopped looking.

**`on_player_changed_position` fires once per tile crossed, not per position
change.** Consecutive events for a walking bot are ~1.0 tiles and ~7 ticks apart.
So a bot that enters a tile and comes to rest partway into it reports its
*entry*, and nothing corrects it — no further event is possible while it stands
still.

In run 27, bot 3 sat at `(-23.5078125, 16.203125)` from tick 18240 to the end,
13,000 ticks, while its last reported position was `(-23.19140625, 16.96484375)`.
At the believed position its box clears the `[-23, 16]` furnace footprint by
0.067 tiles; at the real one it is squarely inside. Bots 2 and 4 were 0.445 and
0.297 tiles out the same way.

**`PlanState`'s `characters` occupancy source was never broken.** It exists, it
works, and probed directly with bot 3's *real* position it correctly reports
`is_area_free("stone-furnace", [-23, 16]) == false`. It was being fed stale
input. Every parked bot in every run has been up to a tile wrong.

That is why the stand-point model was the wrong fix and was rejected: it models
where a bot *will* park, when the planner already had a source for where one
*had* parked and that source was lying. Adding a second model to compensate for a
broken input buys two wrong answers.

The fix reports `player.character.position` at every point the walker lets a
character stop, and widens the mod's existing acting-player recovery — walk aside
and retry — to any bot standing in a refused footprint. The same widening
`537adf30` made to the classification, now made to the recovery.

### A correction to what I told the project owner

I said that recording a parked bot as a refusal "suppressed `recover`'s tier-1
reschedule, which is exactly the recovery that fits". **`scripts/supervisor.lua`
never calls `obs:recover()` at all.** It re-plans through `goal.plan` each
iteration and stops after three non-improving plans. Tier 1 was never in this
loop, so it was never the thing being suppressed. The classification fix in
`537adf30` was still right — a character says nothing about the ground — but my
account of what it saved was wrong.

### Geometry, still deliberately unchanged

Furnaces on a 2-tile grid still leave 0.2 tiles against a 0.4-wide character, so
the layout still manufactures these collisions. This fix makes each one cost a
short walk instead of a milestone — the right cost, not the absence of one.

### Still open

Milestone 6 gave every step to bot 1; bots 2, 3 and 4 received no dispatch after
tick 18700, which is *why* they were parked in the first place. That is the
"a lab is one craft, and no bot can hand an item to another" constraint again,
and it remains a decision for the project owner rather than a bug.

## Update (~16:45): run 28 — 6 of 7, and one blocker left

Run 28, built at `0011f40c` on a quiet machine (load 2.42 at start). **Matches the
record and is by far the cleanest route to it:**

```
milestone 1  satisfied  1 iteration    8 steps
milestone 2  satisfied  1 iteration    8 steps
milestone 3  satisfied  1 iteration   37 steps
milestone 4  satisfied  2 iterations  40 steps
milestone 5  satisfied  1 iteration   16 steps
milestone 6  satisfied  3 iterations  46 steps
milestone 7  stuck after 5 iterations
```

Rung 6's closing iteration was **44 actions with zero failures of any of the four
kinds** — the best single pass there has been. And the four-count line earned its
keep immediately: every rung-7 iteration reads `actions(failed=0 lost=0)
walks(failed=1 lost=0)`, which under the old summed label would have printed as
`failed=1` and told nobody which of four different things went wrong.

### One blocker, named plainly

```
milestone 7: last error: ERROR: stuck while walking,
             aborted before reaching last waypoint
```

Not the planner, not crafting, not placement, not research. **A walk.**

### The teleport contributed nothing, again

```
planned  84   success=46 pending=11  walks(failed=1)  +13 teleports
planned  55   success=19 pending=19  walks(failed=1)   +2 teleports
planned 119   success= 1 pending=78  walks(failed=1)   +0 teleports
planned 118   success= 0 pending=78  walks(failed=1)   +0 teleports
```

Thirteen teleports in the first iteration and the walk failed anyway; none at all
in the last two and the same walk still failed. The project owner has directed
its unconditional removal, replaced by re-pathing with an honest *unreachable*
failure. That work is in flight.

Note the plan growing — 84, 55, 119, 118 — while the last two iterations
accomplished 1 and 0 actions against 78 pending. Whatever the walk cannot reach
blocks a large fraction of the plan behind it, which is the argument for failing
a walk *fast and honestly* rather than retrying it.

### Where the ladder stands

**6 of 7.** Rung 7 has now been reached in three consecutive runs and satisfied in
none. Every rung below it closes reliably, most in a single iteration. The
remaining distance is one walk that cannot reach its destination and a planner
that cannot route around it because a teleport keeps telling it the walk arrived.

## Update (~17:45): run 29 — the teleport is gone and nothing missed it

Run 29 (`run-1788361433-78052`), built at `4bdf5565`. **6 of 7**, and the first run
in the project's history with **zero teleports** — 26 iterations, `+0 teleports`
in every one.

```
milestone 1  satisfied  1 iteration   6 steps
milestone 2  satisfied  1 iteration   6 steps
milestone 3  satisfied  2 iterations 27 steps
milestone 4  satisfied  2 iterations 30 steps
milestone 5  satisfied  1 iteration   3 steps
milestone 6  satisfied  3 iterations 31 steps
milestone 7  stuck after 4 iterations
```

Rung 5 closed in a single iteration of **three steps**. Plans are markedly
smaller than run 28's (27/30/31 against 37/40/46), which is what the ore fixes
should do: a model that stops forgetting a tile on the first mining swing does
not re-plan the mining it already did.

### The re-path replacement works, and failed honestly once

```
milestone 6: last error: ERROR: stuck while walking,
             gave up after 4 re-paths on one walk
```

Milestone 6 **survived that and closed anyway**. That is the entire argument for
the change: the walk failed, said so, and the run routed around it — where a
teleport would have reported arrival and left the planner choosing the same
unreachable site.

### Rung 7's blocker has changed

```
milestone 7: last error: the game reported no readable outcome:
             no action result received in time
```

Not a walk any more. An action that never got a verdict — `actions(failed=0
lost=1)`. That is a different defect and the next thing to diagnose.

### The first video capture failed, and the interesting part is why

`video.mp4` was **0 bytes after 26 minutes** while `ticks.jsonl` grew to 158 KB.
ffmpeg was a zombie; `video.json` said `status: recording` throughout and only
recorded `ffmpeg_exit: 234` at stop, 37 minutes late, with `reason: null`.

The immediate cause is one token (`8482d560`): ffmpeg 9.0.1 rejects
`default_base_is_moof`; the flag is `default_base_moof`. Verified both ways
against the live client window — the old flag exits 234 with zero bytes, the
corrected one records 1,148,279 bytes in 8 seconds and emits the progress lines
the calibration reads. **First measured bitrate: ~143 KB/s at 706x854@15fps**,
about 8.6 MB/min, on a largely static window.

**The design gap is worth more than the bug.** The spec deliberately chose a
one-frame trial grab over a version string — the right instinct — and it still
let this through, because a single PNG frame never touches `-movflags`. The probe
proved x11grab worked and proved nothing about the command that actually runs.

One thing that did work: Hyprland refused the 1280x720 resize, and because the
recorder records the geometry it *observes* rather than the one it requested, the
file is honestly described as 706x854 instead of silently mislabelled.

## Update (~18:55): run 30 — video works, and rung 7 names its blocker

Run 30 (`run-1788365280-15443`), built at `86c2d28e`. **6 of 7**, and the
best-behaved run there has been:

```
milestone 1  satisfied  1 iteration   4 steps
milestone 2  satisfied  1 iteration   2 steps
milestone 3  satisfied  1 iteration  11 steps
milestone 4  satisfied  1 iteration  12 steps
milestone 5  satisfied  1 iteration   1 step
milestone 6  satisfied  2 iterations 31 steps
milestone 7  stuck after 6           39 steps
```

Rungs 1-5 each in a single iteration, against run 29's 6/6/27/30/3 steps and run
28's 8/8/37/40/16. The plans are collapsing as the world model stops lying to the
planner.

### Video capture works, and the join is verified rather than assumed

```
290 MB · h264 · 700x854 · 45 min 16 s
ffmpeg_exit: 0   status: stopped   rate_ok: true
calibration points: 2   ticks.jsonl: 5,226 lines
measured: 6.4 MB/min
```

`rate_ok: true` is the important field: the two-point calibration read ffmpeg's
`-progress` stream and **confirmed the playback rate is 1.0**, so the video/tick
join is checked, not trusted — the same discipline `frameJoin.ts` applies to
frames.

**First real measurements**, replacing estimates that have been carried since the
spec was written: **6.4 MB/min at 700x854@15fps**. Still unmeasured: the
encoder's UPS cost, and the screenshot cost it would be compared against. No
claim that video is cheaper than screenshots.

### Rung 7's blocker is now a sentence instead of a mystery

```
ERROR: stuck while walking, the destination is unreachable:
the game's pathfinder found no path from (6.90234375/30.09765625)
                                      to (-22.30078125/18.22265625)
```

Three runs ago this was a silent teleport that told the planner the bot had
arrived. Two runs ago it was `stuck while walking, aborted before reaching last
waypoint`. Now it names both endpoints and attributes the verdict to the game's
own pathfinder. That is the entire value of failing honestly: the next question
is answerable.

The next question being: **why is that route unreachable?** The bots are at
`(6.9, 30.1)` — the eastern furnace cluster — and the target is `(-22.3, 18.2)`,
the western ore field both earlier runs died near. Something between them is
impassable, and the candidates are water, cliffs, or a wall of our own furnaces.

### Also seen, and not yet explained

```
milestone 6: last error: Error: recipe automation-science-pack
             is not enabled for this force
```

Milestone 6 satisfied anyway on the next iteration, but that message is the
recipe-gate defect's fingerprint (`21a1228a`) appearing at execution time rather
than plan time. Worth a look before assuming it is benign.

## Update (~20:20): run 31 — the fastest run yet, and rung 7 fails honestly in seconds

Run 31 (`run-1788372605-35170`), built at `075df7c1`. **12 minutes**, against run 30's
45. Rungs 1-6 all satisfied:

```
milestone 1  satisfied  1 iteration   8 steps
milestone 2  satisfied  1 iteration   8 steps
milestone 3  satisfied  1 iteration  37 steps
milestone 4  satisfied  2 iterations 40 steps
milestone 5  satisfied  1 iteration  16 steps
milestone 6  satisfied  1 iteration  52 steps   <- first time in one pass
```

**Screenshots are retired and the samplers survived**: `frames: 0`, `samples: 755`,
`events: 301`. That was the trap in the change — `sample_force` and `sample_bots`
both return early when the capture session is nil, so deleting the session would
have silently emptied three viewer panels. The session still starts and registers
no camera.

### Rung 7 now refuses in seconds, and the refusal is true

```
automation needs a lab with 60 kW of electric supply,
and the plan can show only 0 kW
```

Run 30 spent **85,030 ticks** discovering nothing on this milestone. Run 31 says
it at plan time. The refusal is correct: no plan in any archived run has ever
placed a generator, and `generated_kw` was 0.0 in all 541 of run 30's force
samples.

### But a correct refusal should not crash the run

```
RAISED: runtime error: goal: automation needs a lab with 60 kW ...
RUN FINISHED state=crashed
```

`ResearchNeedsPower` propagates out of `goal.plan` as a Lua error, through
`supervisor.lua:197`, and terminates the run. There is no `milestone 7` line in
the summary at all — the run record cannot say what happened to the milestone it
died on.

That is the wrong shape for a *planner refusing to plan something impossible*.
`NoApplicableMethod` and its relatives are verdicts about the world, not faults:
the milestone should read `stuck` with that reason and the run should finish. As
written, the most informative failure the planner has ever produced is also the
one that destroys the record of itself.

### The window size is still not honoured, and it moved

```
video window is 1426x1728, not the 1280x720 requested -- recording what it is
```

`window-size=1280x720` in `config.ini` did not take either — and the observed
geometry *changed*, from run 30's 706x854 to 1426x1728, which is almost exactly
2x of 713x864. That smells like display scaling rather than the setting being
ignored outright.

What this establishes: **on a tiling compositor, neither `xdotool windowsize` nor
Factorio's own `window-size` fixes the geometry.** The window would have to be
floated by a compositor rule. Worth saying plainly rather than trying a third
lever.

The safeguard did its job both times: the recorder records the geometry it
**observes**, so the file is honestly described at 1426x1728 rather than
mislabelled 1280x720.
