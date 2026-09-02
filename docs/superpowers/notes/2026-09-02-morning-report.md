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
