# Rung 7 is not blocked by an unreachable ore field — 2026-09-02

**Run:** 30, `workspace/runs/run-1788365280-15443/` (plus `workspace/server-log.txt`,
which is still that run's).
**Status:** analysis only. No production code changed. Two fixes named, neither
of them mine to make; one hardening named that is mine, and deliberately not
landed yet (see *What I did not do*).

## The short version

The message that ended run 30 —

```
ERROR: stuck while walking, the destination is unreachable:
the game's pathfinder found no path from (6.90234375/30.09765625)
                                      to (-22.30078125/18.22265625)
```

— is **true, honest, and not the reason rung 7 is stuck.** Two separate things
are going on and the record settles both without a new run.

1. **The route is passable.** `(-22.30078125, 18.22265625)` is not a place on
   the western ore field. It is a point **inside the collision box of the stone
   furnace at `(-22, 18)`** that this same run placed at tick 33,342. The walk
   was refused because its *destination* was inside a building, not because the
   ore field was walled off. Eight ticks after the refusal the same bot asked
   for a path from the same tile to the same ore field, got one, walked it, and
   mined copper at `(-23.5, 18.5)`.

2. **Rung 7's actual blocker is that the plan for `research automation` cannot
   finish.** Every one of its five plans is `… craft 1 lab … research
   automation`. **The lab is never placed. No power is ever generated. No
   science pack is ever inserted.** `automation` was successfully started at
   tick 105,028 and sat at `progress 0.0` for the remaining 60,661 ticks of the
   run, with `generated_kw = 0.0` in all 541 force samples. The action waited
   out the 360-second deadline and was recorded `lost`;
   every later attempt was refused by `force.add_research` because the
   technology was already the force's current research.

The walk error is in `milestone_stuck.last_error` because `first_error` is the
*earliest* error of the milestone (`crates/scripting_lua/src/globals/goal/run.rs`
prefers a failed action, then a lost one, then a walk). It came from iteration 2
of 4, at tick 81,661. The thing that stopped iteration 4 is in the same file's
event stream 76,000 ticks later, and says something else entirely.

## Evidence: the route is passable

- **Bot 2 crossed `x = 0` fourteen times during rung 7**, `samples.jsonl` (2,698 bot samples),
  x ranging from `-26.22` in the west to `+66.28` in the east.
- **The very next path request succeeded.** `server-log.txt`:

  ```
  §81661§action_completed§fail 72 ERROR: stuck while walking, the destination is unreachable: …
  §81669§on_script_path_request_finished§33#[{"x":6.90234375,"y":30.09765625,…} … {"x":-23.5,"y":19.5,…}]
  ```

  Same start tile, 33 waypoints, into the western ore field. `events.jsonl` then
  has `81919 action_dispatched id 6 mine 1 copper-ore target (-23.5, 18.5)` and
  `82039 action_settled … success`.
- Rung 7 successfully mined at `(-23.5, 18.5)`, `(-25.5, 24.5)`, `(-22.5, 21.5)`,
  `(-22.5, 22.5)` and `(-26.5, 25.5)` — the ore field the walk supposedly could
  not reach — while also mining coal at `(55.5, 2.5)` and stone at `(62.5, -29.5)`
  sixty tiles the other way.

So: **not terrain.** No water, no cliff, no walled-off patch.

## Evidence: rung 7's blocker is the research plan

`events.jsonl`, milestone 7, all five `plan_created` events, filtered to the
steps that mention a lab, a science pack, a pole or research:

| plan tick | steps | lab/power/research steps |
| --- | --- | --- |
| 80931 | 32 | `craft 1 lab`, `research automation` |
| 81644 | 30 | `craft 1 lab`, `research automation` |
| 88990 | 28 | `craft 1 lab`, `research automation` |
| 126616 | 32 | `craft 1 lab`, `research automation` |
| 158217 | 31 | `craft 1 lab`, `research automation` |

`map.jsonl` has 17 `placed` records for the whole run and **every one of them is
a `stone-furnace`.** No lab, no boiler, no steam engine, no solar panel, no pole.

`samples.jsonl`, `kind: "force"` — the research field changes exactly once in
all 541 force samples:

```
tick 3900    research null                                        generated_kw 0.0
tick 105300  research {"name":"automation","progress":0.0,…}      generated_kw 0.0
```

and then never again, for 60,661 ticks to the end of the run. All 541 force
samples read `generated_kw 0.0, consumed_kw 0.0`.

The two rung-7 action failures follow from that directly:

```
105028 id 0  lost    the game reported no readable outcome: no action result received in time
158217 id 0  failed  Unexpected Output: Error: cannot research automation:
                     researched=false enabled=true trigger=false unmet_prerequisites=[]
```

The mod's refusal message (`control.lua`, `start_research`) enumerates four
reasons `force.add_research` can answer `false` and all four are negative here.
The fifth reason is the one it does not name: **the technology is already the
current research or already queued** — which the sample stream confirms it was,
from tick 105,300 onwards.

This is a hard blocker. Nothing in any of the five plans could ever advance
`automation` past `progress 0.0`, so rung 7 could not have closed no matter how
the walks went.

## Evidence: why the walk failed anyway

Worth fixing on its own account — it cost run 30 three failed walks (756 ticks
spent stalling inside them, and three abandoned executor slices behind that),
and its error message is what sent this investigation (and
the brief) looking at terrain.

### The destination was inside a furnace we built

Character half-extent `0.19921875`, stone-furnace half-extent `0.69921875`, both
read off this run's own `entity_prototypes` writeout. All three failed walks:

| walk | fails at tick | destination | inside furnace | placed at tick |
| --- | --- | --- | --- | --- |
| 72 | 81661 | `(-22.30078125, 18.22265625)` | `(-22, 18)` | 33342 |
| 96 | 89002 | `(-22.2890625, 23.3359375)` | `(-22, 24)` | 87493 |
| 179 | 165964 | `(-19.5, 19.5)` | `(-19, 20)` | 161648 |

Three for three. No other walk in the run failed.

That destination is **the last waypoint of the path Factorio returned**
(`start_walk_repath` takes `w.waypoints[#w.waypoints]`), and Factorio returned
a path that runs through the furnace. Parsing every
`on_script_path_request_finished` in `server-log.txt` against the furnaces the
run had placed by that tick: **12 of 75 returned paths contain at least one
waypoint strictly inside a stone-furnace collision box**, and every one of those
waypoints carries `needs_destroy_to_reach: false` — the pathfinder believed the
tile was clear. Examples: `(6.5, 30.5)` inside `(6, 31)`; `(8.5, 30.5)` inside
`(8, 30)`; `(4.5, 34.5)` and `(5.5, 34.5)` inside `(5, 35)`.

`move_player_timed` accepted it because the only thing it checks is
`walk_arrives(goal, radius, end)` — distance from the **caller's** goal. Walk 72
was a mine's corrective walk toward the copper the next iteration mined at
`(-23.5, 18.5)`; against `approach_radius(2.7) = 1.35` the tolerance is `2.35`
and the returned endpoint is `1.2309` away, so it passes comfortably. Walk 179
is the cleaner illustration: its destination `(-19.5, 19.5)` is `0.707` from the
centre of the furnace at `(-19, 20)`, which is what a `take from the furnace`
step's `AtPosition` target *is* — the entity's own position, inside its own
collision box, exactly the case `approach_radius`'s doc comment already names.
Nothing anywhere asks whether a character can *stand* on the endpoint.

### Then the re-path could not possibly succeed

`WALK_REPATH_RADIUS = 0.5`. To stand clear of a stone furnace a character's
centre must be at least `0.69921875 + 0.19921875 = 0.8984375` from the furnace
centre on one axis. For the goal `(-22.30078125, 18.22265625)` against the
furnace at `(-22, 18)` the nearest standable point is `x = -22.8984375`, which is
`0.59765625` away. **The request was unsatisfiable by `0.09765625` tiles**, by
arithmetic, before the pathfinder ran. `walk_repath_finished` then reported
exactly what the game told it — no path — and the message names a destination
the caller never asked for.

The same holds for any goal near a furnace centre: `0.8984375 > 0.5`. **A walk
whose terminal waypoint lands inside a stone furnace is guaranteed to end in
"the destination is unreachable", whatever the terrain.**

### And the narrow gap is real, and it is where every stall happens

The brief's third candidate is confirmed and can be quantified exactly.

- Furnaces on the planner's 2-tile grid leave `2 - 2 x 0.69921875 = 0.6015625`
  tiles between collision boxes.
- A character is `0.3984375` wide.
- Slack: `0.203125` total, `0.1015625` per side.

Now the measurement. For each of the 18 walk-stall detections in run 30, the
separation between the character's box and the nearest furnace box on the freer
axis:

```
 5/40   (  6.90234, 31.99219)   0.09375   furnace(6,31)
 4/39   (  6.79688, 31.92969)   0.03125   furnace(6,31)
 4/39   (  6.79688, 31.91016)   0.01172   furnace(6,31)
 4/39   (  6.90234, 30.09766)   0.00391   furnace(6,31)
33/34   (  7.20312, 30.90234)   0.00391   furnace(8,30)
 3/35   (  7.09766, 30.92578)   0.02734   furnace(8,30)     x4
 3/51   (  8.79688, 30.90625)   0.00781   furnace(8,30)
 5/53   (  8.79688, 30.95312)   0.05469   furnace(8,30)
75/82   (  4.09766, 34.09766)   0.00391   furnace(5,35)
 3/87   (-22.90234, 24.90234)   0.00391   furnace(-22,24)
52/59   (  8.79688, 30.90234)   0.00391   furnace(8,30)
 5/80   (  5.90234, 34.09766)   0.00391   furnace(5,35)
14/96   (-18.20312, 19.09766)   0.00391   furnace(-19,20)
69/78   (  4.09766, 34.09766)   0.00391   furnace(5,35)
 9/36   (  7.09766, 30.95703)   0.05859   furnace(8,30)
```

**18 of 18.** Maximum clearance `0.094` tiles; eight of them at `0.00391`, which
is `1/256` — one position unit, the character physically scraping the box. The
mod's follower steers with eight-way `walking_state` toward a waypoint that is
inside the furnace, and the character wedges against it. `(4.09766, 34.09766)`
is squarely in the `0.6015625` gap between the furnaces at `(5, 33)` and
`(5, 35)`; `(7.09766, 30.92578)` is in the gap between `(6, 31)` and `(8, 30)`.

### The re-path machinery itself works

12 stall episodes, **9 recovered on the first re-path**, 3 failed — and all 3
failures are exactly the walks whose terminal waypoint was inside a furnace,
i.e. the case no re-path can fix. The 4-re-path bound was reached once. The
`98895500` change is doing its job; what it exposed is upstream of it.

## What the record cannot settle

**Why Factorio returned those paths.** The request is right: `request_player_path`
passes the character's own `collision_box` and `collision_mask`
(`is_object, player, train`), and a stone furnace's mask includes `player`, so
the two do collide; `runtime-api.json` confirms `request_path`'s `collision_mask`
parameter is the same `CollisionMask` type `LuaEntityPrototype.collision_mask`
returns. The pathfinder also answered "no path" correctly on two of the three
occasions it was asked afresh, so it is not simply blind to our buildings.

The leading explanation is documented in the shipped 2.1.17 docs and is one line
to test: **`PathfinderFlags.cache` defaults to `true`**, and its own description
is *"Enables path caching. This can be more efficient, but might fail to respond
to changes in the environment."* `request_player_path` sets only
`allow_destroy_friendly_entities` and `prefer_straight_paths`, so caching is on —
in a run whose entire job is changing the environment along the two corridors it
walks. The signature fits: identical requests returning identical
through-a-furnace answers (walk 96 re-pathed four times from the *exact* same
position and got the same 35-waypoint answer each time), and a fresh answer from
a start 1.9 tiles away correctly reporting no path.

**What a run would have to capture to settle it, and nothing shorter will:** the
mod already has `rcon_async_request_path`. Issue the *same* start/goal twice back
to back, once with `pathfind_flags.cache = false` and once with the default, at a
moment when a furnace stands on the straight line between them, and write both
answers out. If the `cache = false` answer detours and the default one does not,
it is the cache. Nothing in the existing record can distinguish that from "the
unit pathfinder's answer is not collision-exact at this resolution", because
every archived path answer was taken with the same flags.

## Whose fixes these are

### 1. `crates/planner` — rung 7's actual blocker. Report only; not edited.

The `research` method must produce a plan that can actually finish. As it stands
it emits `craft 1 lab` followed by `research automation`, and `research
automation` is an `add_research` call that the executor then waits on for
`on_research_finished`. With no lab on the map and `generated_kw = 0.0`, that
event can never be raised.

Minimally the expansion needs: place the lab; insert the automation science
packs into it; and provide power — which for rung 7 means the boiler / steam
engine / offshore pump / small pole chain, or a burner-only alternative if one
exists. Precisely what has to change:

- the `research` method's decomposition, so `Researched(tech)` depends on
  `LabPowered` and `LabFed` and not only on `HasItem(lab)`;
- a power sub-goal, which the planner currently has no notion of at all — no
  plan in any archived run places a generator;
- the `state.rs` overlay would need to read `power` from the world snapshot to
  know whether that sub-goal is already satisfied. `samples.jsonl` already
  carries `power.generated_kw` / `consumed_kw` / `satisfaction`, so the
  measurement exists; whether `FactorioWorld` carries it is a separate question.

Until then rung 7 is unreachable by construction and no amount of walk fixing
changes that.

### 2. `mods/BotBridge/control.lua` — two one-liners. Report only; not edited.

- `request_player_path` should carry `cache = false` in `pathfind_flags`, on the
  evidence above, and that change is exactly the experiment that proves or
  disproves the cache theory.
- `start_research`'s refusal message should name the fifth reason. It currently
  prints `researched / enabled / trigger / unmet_prerequisites`, all four of
  which were negative in run 30, leaving the caller with a message that rules
  everything out and explains nothing. Adding whether the technology is the
  force's `current_research` or already in `research_queue` would have named
  run 30's second failure outright.

### 3. `crates/core/src/factorio/rcon.rs` — hardening. Mine, specified, not landed.

`move_player_timed` judges the returned path only by
`walk_arrives(goal, radius, end)` — a statement about the **caller's** goal. It
should also refuse a path whose last waypoint is somewhere a character cannot
stand, because that waypoint becomes the walk's non-negotiable destination in
the mod and therefore guarantees a stall, four wasted re-paths, and a failure
message that blames terrain.

The oracle already exists and is in-bounds:
`EntityGraph::blocking_boxes_within` (`crates/core/src/graph/entity_graph.rs`),
which is documented as the ground truth for buildability — every entity with a
non-zero collision box except resources and rails, plus player-collidable
tiles — and `FactorioWorld` holds an `Arc<EntityGraph>`, so `move_player_timed`
can reach it with no new plumbing. The shape:

- probe `blocking_boxes_within` around the terminal waypoint, expanded by the
  character's half-extent;
- test each returned rectangle exactly (the quad-tree query is a narrowing pass,
  as its doc comment says);
- on an overlap, either trim back to the last standable waypoint **if that still
  satisfies `walk_arrives(goal, radius, ·)`** — which keeps the arrival contract
  exactly as it is and is not the teleport's substitution, because the contract
  was always "within tolerance of the caller's goal" — or refuse with a new
  `NotDispatched` variant naming the obstruction.

For run 30's walk 72 the trim would not have helped (the previous two waypoints
are also inside the furnace and the one before that is 3.0 tiles from the goal
against a 2.35 tolerance), so it would have become an immediate honest refusal
naming `stone-furnace at (-22, 18)` instead of a 280-tick stall blaming the
pathfinder. That is worth having.

## What I did not do, and why

I did not land the `rcon.rs` change. It sits on the path of **every** walk, and
its correctness depends on `EntityGraph` never believing a building is somewhere
it is not — a false positive there refuses a good walk, which is a worse failure
than the one being fixed and would show up in no test that does not run the
game. It also does not unblock rung 7, which is (1) above. It should land with a
live run behind it, and preferably in the same run that tests `cache = false`,
since if the cache is the cause then paths stop terminating inside buildings and
this guard should fire approximately never.

## One more thing the record does not say

**Three walk failures, zero of them in `events.jsonl`.** `EventKind` has
`RunStarted, MilestoneStarted, MilestoneSatisfied, MilestoneStuck, PlanCreated,
ActionDispatched, ActionSettled, Frame, Teleport` and no walk. A corrective walk
inside `player_mine_timed` is a separate dispatch with its own action id, and it
is recorded nowhere; the only trace of run 30's three failed walks in the run
directory is one `last_error` string on `milestone_stuck`, and the only place the
other two exist at all is `workspace/server-log.txt`, which is overwritten by the
next run.

`obs.walks` already carries `bot`, `step_index`, `to`, `status`, the two planned
ticks, the two observed ticks and `error` — everything a `WalkSettled` event
would need. This diagnosis needed the server log; the next one may not have it.
That is `crates/core/src/record/` plus a `record.walks()` drain, owned elsewhere
right now — flagged, not touched.

## Also visible in run 30, not chased

**Bot 1 never moved.** It stood at `(0.0, 0.0)` in all 2,698 bot samples of the run
and received zero of the 161 `action_dispatched` events. Every action in a
two-bot run went to bot 2. This is the same all-work-on-one-bot pattern runs 21,
24 and 27 showed, and it is why rung 7 took 85,030 ticks.
