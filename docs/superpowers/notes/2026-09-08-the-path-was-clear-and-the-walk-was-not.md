# The path was clear and the walk was not — measuring the walk stall

2026-09-08. Branch `the-path-was-clear-and-the-walk-was-not`.

The prompt for this work was `run-1788833726-34821`, the only live attempt at
the oil milestone: **bot 3 lost 474 of its 482 actions to a walk that stalled on
a tree at tick 7,980**, nineteen thousand ticks before any bot was killed. The
hypothesis handed to me was that the game's pathfinder clears a straight
(often diagonal) line and the mod's follower walks it as an axis-aligned
**staircase**, so a tree just off the diagonal blocks a corridor that was
genuinely clear — widened by `prefer_straight_paths = true`, which makes legs
longer and gives the staircase more room to diverge.

**Every part of that is falsified.** So is the simpler story that replaced it.
What is left standing is a different defect entirely, in a different place, and
one measurement that says the largest cost of the whole failure was never the
walk.

---

## 1. How many stalls the archive actually holds: 18, and 17 of them are invisible

Counted by `tools/walk_stall_census.py`, over all 78 `events.jsonl` in
`workspace/*/runs/` and `workspace/runs/`.

```
walk_settled : 6,690
failed walks :    24
```

Of those 24 — read by the **wording of `error`**, not only by `failure.kind`,
because CLAUDE.md records that `classify_walk_failure` once filed 19 of 20
failures as `other`:

| kind | n | wording |
|---|---:|---|
| `no_path` | 10 | `failed to path find` / `no path to` |
| `no_character` | 7 | a bot died; respawns in ~600 ticks |
| `destination_blocked` | 3 | the walk would end inside a `transport-belt` |
| `other` | 3 | 2 × `player N not connected`, 1 × collision-box refusal |
| `stalled` | **1** | `stuck while walking … made no progress` |

**The classification is healthy today**: every `kind` matches the wording it was
derived from, and the three `other`s are genuinely three different things the
grammar does not cover. The under-reporting warning did not fire here.

**But one stall in the archive is not one stall in the archive.**
`FactorioRcon::move_player_timed` answers a stalled leg with a *fresh path from
where the character stands*, up to `WALK_ATTEMPTS` (3) times. A stall it
recovers from settles as an ordinary successful walk and **writes nothing to
`events.jsonl` at all** — only a `warn!` line, and only if a session log
happened to be kept. Counting those:

```
$ grep -h "asking the game for a fresh path" workspace/session-logs/*.log | wc -l
17
```

**18 stalls, 17 recovered, 1 not.** So the record under-reports stalls 18:1, and
this is another instance of the file's own *silence is not success* list: the
mechanism that fixes the problem is also the mechanism that erases the evidence
that the problem happened.

### What the 17 were blocked by — the finding that decided everything else

The retry's `warn!` names the blocker:

| blocker | n |
|---|---:|
| our own `stone-furnace` | 6 |
| our own `burner-mining-drill` | 4 |
| another bot's character (`bot #1`, `#2`, `#4`, an undriven one) | 6 |
| our own `pipe` | 1 |

**Every one of the seventeen recoverable stalls was blocked by something that
had not been there when the path was computed** — an entity this run placed, or
a character that walks. Not one was scenery. The one stall that was *not*
recoverable is the tree.

That is the shape of the problem: **a stall is a stale path**, and re-pathing
fixes a stale path. It is exactly what the `cache = false` comment in
`request_player_path` predicted ("we are what changes it").

---

## 2. The staircase hypothesis is dead, killed by the code and then by 8,029 tiles

### The follower does not walk a staircase

`control.lua`'s follower reduces `dx` and `dy` to signs **independently** and
combines them, so its steer is one of **eight** directions including the four
diagonals — the archived failure says `steering southwest` in as many words.
The comment above `WALK_STALL_PROBE_AHEAD` that says "the follower steers one
axis-aligned step at a time" is about the probe's offset, and reads as a claim
about the trajectory that the code does not make.

And the archived leg was not one the quantisation could have hurt. Leg origin
`(206.234, -190.266)`, waypoint `(205.5, -189.5)`: a vector of
`(-0.734, +0.766)`, which is **within 2° of exact southwest**. The character
stood 0.20 tiles off that segment — less than the follower's own 0.3-tile
arrival box, i.e. ordinary. There was no divergence to explain the tree with.

### Trees do not stall walks: 480 walks, 8,029 tiles, zero

`scripts/walk_forest_probe.lua`, headless, seed 31337, 10x, on ground **nobody
builds on** — the bots only walk, so every obstacle was generated with the map
and no path can go stale. Two phases on the same ground: long hops out into the
quadrant the oil run stalled in, then short 7-13 tile hops repeated in place,
which is what makes the pathfinder wind (the archived stall was `leg 2 of 10`
over a **14-tile** hop; ten waypoints in fourteen tiles is a path threading
obstacles, and a 40-tile hop over grass is the opposite kind of path).

```
pass 1   1 bot    24 walks   1,065 tiles   0 failed   0 stalled
pass 2   4 bots  480 walks   8,029 tiles   0 failed   0 stalled
```

with tree densities up to **98 trees in a 16-tile disc** around the goal, and
**18 of the 480 paths carrying a waypoint the game itself flagged
`needs_destroy_to_reach`**. Not one stalled.

So: it is not the trees, it is not the diagonals, and it is not
`prefer_straight_paths` — none of which changed between those 480 walks and the
one that failed. What changed is that the failing run was **building things and
had four bots moving around each other**, which is the 17.

### What I will *not* claim

**The cause of that one tree stall is not established, and I am leaving it
unknown rather than replacing one story with another.** The probe's own doc
warns that a generous box "would name a furnace two tiles to the side and read
exactly like a furnace in the way"; a forest guarantees a tree inside any box
you draw, and 8,029 tiles of walking says a tree 0.75 tiles ahead is an
ordinary thing to have. The tree is what the probe *found*; that it is what
*stopped* the character is an inference, and it is now an inference with 480
counter-examples behind it. Candidates not eliminated: the run was not
`--peaceful` and a biter can shove a character off its line; another bot's
character had been at that spot; the game held the character for reasons run 9
already documented on open ground with nothing within twelve tiles.

---

## 3. The defect that actually cost 474 actions, and it is not in the follower

A stall reaching a **halt** has already been retried three times with three
fresh paths. So the interesting question was never "retry or re-plan": **retry
is already implemented, already spent, and demonstrably effective on 17 of 18
stalls.** By the time recovery is asked, re-walking is the only thing that
cannot work.

And re-walking is exactly what recovery would have proposed, forever:

- a walk carries **no `ActionId`** — it is keyed `(bot, step_index)` in
  `ExecutionLog` and never appears in `net.actions()`;
- so `recover::exhausted_tier_one`, which reads `net.actions()` for
  `Status::Failed` with three attempts, **cannot see a walk halt at all**;
- and what a halt leaves in the action log is the bot's remaining steps
  published `Status::Lost`, which that budget **deliberately excludes**
  (`crate::run::halt` says so: "`recover` counts failures towards its escalation
  budget and losses not at all").

The consequence: a bot that halts on the same walk every round loops in tier 1
forever. Every proposal is a valid schedule, every run halts at the same tile,
nothing ever reaches `Failed` three times, and the caller's budget runs out
having made no progress and never escalated.

**Fixed** in `crates/executor/src/recover.rs`: `walk_halted_on_a_stall` declines
tier 1 when some walk halted its bot with BotBridge's stalled-leg wording,
matched through `factorio_bot_core::factorio::rcon::walk_reports_stalled_leg` so
a reword in `control.lua` fails a test rather than quietly retiring the check.
Declining on the *first* occurrence is the same judgement `refused_by_the_game`
and `diverged_from_the_world` already make: a verdict about the ground is not a
circumstance.

**Only a stall.** A halt from `player N has no character` is a death that heals
in 600 ticks — the transient tier 1 exists for — and it is **7 of the archive's
24 failed walks**, nearly a third. Collapsing the two would trade one lost run
for another. Both cases, and the no-verdict case, are controls in
`a_bot_halted_by_a_stalled_walk_escalates_past_tier_one`.

### And the oil run would still not have recovered

`scripts/oil_milestone.lua` line 188 is `local obs = goal.run(plan)` and there
is no line 189. **It never calls `obs:recover()`.** The recovery machinery — the
three tiers, the `Rescheduled`/`Reexpanded` distinction, the log-carrying rules
— was all in place and was never asked. `scripts/supervisor.lua` is the only
script in the tree that loops on it.

So the 474 actions were lost twice over: once because the script did not ask for
recovery, and once because, if it had, recovery could not have escalated. The
second is fixed here. The first is a one-line change to a script and belongs to
whoever owns the oil milestone.

---

## 4. Numbers, and what moved

All four baselines re-measured on this branch's own debug binary (built off
`7faf1076` plus this change), and **all four are unmoved** — identical to the
figures the task carried:

| goal | world | actions | ticks |
|---|---|---:|---:|
| `researched:automation` | `map.json` | 176 | 21,784 |
| `producing:automation-science-pack:6` | `map.json` | 316 | 22,457 |
| `producing:logistic-science-pack:6` | `map.json` | 441 | 47,478 |
| `gathered:crude-oil` | `map-31337-explored.json` | 2,117 | 314,345 |

Expected, and stated as a check rather than a result: nothing in
`crates/planner` was touched, and `recover` has no caller on the `plan` path.
Measured anyway, because "it cannot have moved" is the sentence this project
keeps having to retract.

## 5. Apparatus, committed

- `tools/walk_stall_census.py` — the archive census, including the leg geometry
  extraction that would have tested the diagonal prediction had there been more
  than one sample to test it on.
- `scripts/walk_forest_probe.lua` — the 480-walk forest probe. Nothing is
  cheated and nothing is built, which is the whole point: it is the control the
  live runs do not have.
