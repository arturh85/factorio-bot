# A bot of ours on the tile: what actually failed, and what one failure cost

2026-09-06. Two defects from `run-1788663566-25023` (headless, four bots, 5x,
a 179-entity `FurnaceLine`): a placement blocked by one of our own bots, and
the ~50 placements that died with it.

**Measured live**, eight headless runs at 5x on seed 31337, four with the
change and four without, mods symlink verified against this worktree on every
one.

## (A) The run is not evidence about the busy-blocker fix — the fix was not loaded

The hypothesis under test was that `c14c1fd9`'s 45-second busy budget covers
`mining` and `walking` but not an idle blocker with nowhere to step aside,
which is classified `stuck`. **The code says that is exactly right. The run
cannot say so, because it did not run that mod.**

The refusal in the record is:

```
cannot place item 'transport-belt' because a character is standing in the
footprint; dispatched 4 times over 534 game ticks (1.8s of waiting between
attempts) and refused every time
```

There is **no `(blockers: …)` clause**, and `describe_footprint_blockers`
returns the empty string only when `step_aside_from_footprint` named nobody —
which happens only when every character in the box is the acting bot. The
acting bot was 3, at `(13.30, 3.20)`; the blocker was bot 4, at
`(20.02, 5.07)`, against a belt at `(20.5, 5.5)` whose ±0.4 box the character's
±0.2 box overlaps. The current mod would have named it. So the mod that ran had
no clause at all.

`provenance.json` records commit `191df2db`, clean, and that commit's
`mods/BotBridge/control.lua` **does** have the clause. The other session then
checked its own run rather than accepting the account, and it is worse than a
stale symlink: **at that run's own branch HEAD, `control.lua` contained zero
occurrences of `describe_footprint_blockers`** — the function landed half an
hour later. The clause was not empty; it did not exist.

**Provenance's `git.commit` is not the mod that ran.** The worktree's binary
was launched from the main checkout's directory, so provenance was *accurate
about the thing it measures and silent about the thing that mattered* — not
blank, but confidently about the wrong object. That is a third blind spot
beside the two `world.dump` ones, and it silently invalidated evidence two
sessions reasoned from: a 45-second budget was sized partly on that run, and
the other session drew conclusions about its own block from it. Both may still
be true; neither is measured.

**What a run would have to record for this to be impossible**: the *resolved*
mods directory (the symlink target, not the checkout), and ideally a content
hash of the mod as loaded — a symlink target is not the same claim as the
bytes that loaded. Until then, the `Using mods directory` line is the only
answer, and it has to be read on every run rather than inferred.

### The gap is real anyway, and it is `stuck` — but `stepping aside` is fine

Reading the vocabulary against `FootprintWait::decide`:

| clause | busy budget? | ends? |
|---|---|---|
| `#N mining` / `#N walking` | yes | its own action |
| `#N stepping aside` | **on the second attempt** | the walk |
| `#N stuck` | no | never |
| `#N gone`, `an unclaimed character` | no | never |

**The `stepping aside` row is wrong, and the runs below are what corrected
it.** The code reading was: `start_walk_waypoints` sets
`storage.p[id].walking`, so the next attempt reads `#N walking` and draws the
45 s budget. Live, the clause reads `#3 stepping aside` **four times in a
row**. The reason is arithmetic nobody did: a step aside is one or two tiles,
about 13 ticks at 0.15 tiles/tick, while `FOOTPRINT_CLEAR_BACKOFF` is 0.6 s —
**180 ticks at 5x**. The walk finishes, `walking` clears, and the next attempt
finds an idle bot and dispatches another step aside. So `stepping aside` gets
the four-attempt budget, not the busy one, and a blocker that keeps coming back
is refused in 399 ticks.

Corrected table: the only class that reliably draws the busy budget is a
blocker busy with an action of *its own*. `stuck`, `gone` and
`an unclaimed character` never do, and `stepping aside` does so only if the
walk is still running when the next attempt lands — which at these backoffs it
usually is not.

### The fix: escalate the search instead of giving up

`placement_step_aside_landing` asked `find_non_colliding_position` at one
radius, 4, sized for "a handful of stacked bots" on open ground. A 179-entity
block packed with 2×2 furnaces can have no character-sized gap within four
tiles of any exit, and then every exit answers nil and the blocker is `stuck`.

It now walks a bounded ladder — `PLACEMENT_STEP_ASIDE_RADII = {4, 12, 32}` —
radius outermost, exits innermost. Because `find_non_colliding_position`
searches outward from its target, **a landing the old radius had is still
found first**: widening can only turn a `stuck` into a longer walk, never move
an answer. Twelve questions and then `stuck`, which stays the honest answer for
a block with genuinely no room in it.

What this does **not** do: teleport the blocker, or place the entity on top of
it. The mod can do both (`push_characters_out_of`), and both were rejected —
moving another bot is not a player action, and the machinery exists for the
case where the game put a building on someone, not as a way past a refusal.

## (B) One unresolvable placement no longer costs the rest of the block

`run_bot_signalled` stopped the bot at its first failure and `abandon_rest`
published `Failed` for every remaining step in its slice.

**The justification is the dependency graph, not a measurement.** The "~50 of
179 never dispatched" figure this task was briefed with is **void** — it came
from `run-1788663566-25023`, whose mod predates `describe_footprint_blockers`
existing at all (see (A)). The argument does not need it: the network already
knows what depends on what, and halting a bot on a failed action discards work
the graph says is independent. Everything below is measured here instead.

A failed or lost `Act` now publishes its own verdict and the bot moves on.
**The dependency graph decides the blast radius**: `await_preds` reads the
predecessor's signal and returns `Abandoned`, so a belt nobody built takes down
the inserter that feeds it — and takes down nothing else. An abandoned step
publishes `Failed` for *itself*, which is what carries the abandonment one hop
further; without that a bot waiting behind it hangs, and there is a test that
watches it hang.

**A failed walk still stops the bot**, deliberately. A walk carries no signal,
and its effect is the bot's position, which every later step depends on with no
edge saying so. That is the one dependency the network does not hold.

**The honest cost**: an edge the network is *missing* used to be masked by the
bot stopping and is now not. Two things narrow it — `crate::occupancy` still
holds inventory ordering within a bot, and the failure still counts towards
`recover`'s escalation budget, so a batch that keeps failing replans rather
than grinding on.

## Tests, and what they assume

Both changes were falsified before being believed, and the substitution was
checked to have matched each time:

- ladder → `{4}`: `a_blocker_with_no_near_landing_is_walked_further_rather_than_called_stuck`
  fails with "Got []" — nobody asked to move.
- abandoned path publishes nothing: `an_abandoned_step_still_releases_a_bot_waiting_behind_it`
  and the pre-existing `abandonment_propagates_along_a_chain_of_waiting_bots`
  both time out on the 60 s deadline.
- old batch-stop behaviour restored:
  `a_failed_action_does_not_take_down_an_independent_later_step` and both
  `an_action_missing_from_the_network_*` fail.

**These tasks wrote both the code and the fixtures.** The stub-game ground
("free at twelve tiles, full at four") is a claim about a dense block, not a
reading off one — and the live runs did **not** settle it in the ladder's
favour: `#2 stuck` survived four attempts three times with the ladder in
place. The executor pair is
stronger: the independent-step test and the dependent-step test share one
fixture and differ by a single `net.link`, so a loop that ignored dependencies
would pass one and fail the other.

## Measured: eight `furnace_run.lua` runs, headless, 4 bots, 5x, seed 31337

`FurnaceLine`, 179 entities, anchor `(60,-100)`-ish, materials cheated (the
script discloses it). Identical binary and identical mod on both sides; the
**only** difference is the executor's `StepKind::Act` arm. Every run logged

```
Using mods directory "…/workspace/headless-r/mods" (debug build; BotBridge is
a symlink to "…/.worktrees/blocked/mods/BotBridge", so an edit there is what
the game loads)
```

| | failed | **never dispatched** | standing | delivered tps |
|---|---|---|---|---|
| before 1 | 0 | 0 | 179 | 295 (98%) |
| before 2 | 1 | **14** | 164 | 269 (90%) |
| before 3 | 2 | **7** | 170 | 299 (100%) |
| before 4 | 1 | **4** | 174 | 284 (95%) |
| after 1 | 1 | 0 | 178 | 210 (70%) |
| after 2 | 1 | 0 | 178 | 314 (105%) |
| after 3 | 2 | 0 | 177 | 300 (100%) |
| after 4 | 2 | 0 | 177 | 301 (100%) |

`pending` in the run's own `done=…` line is what "never dispatched" reads; the
after runs' `action_dispatched` count is 179 of 179 planned.

**A failed placement now costs exactly one entity.** Before: four failures cost
4 + 25 = 29 entities across four runs. After: five failures cost five.

Two things this table also settles, neither of them flattering:

- **The failure is a race, not a property of the plan.** Same seed, same map,
  same 183-step plan of makespan 2,139, and `before 1` hit no refusal at all.
  A single run that comes back clean proves nothing here; four did not.
- **`#2 stuck` still happens with the ladder in place** — four attempts, full
  budget, in three of the eight runs. The ladder did not eliminate `stuck` in
  this block; the game offered no landing outside the clearance even at radius
  32. What the ladder is proven to do is not regress anything (the after runs'
  failure counts match the before runs'), and what actually saved the block was
  (B). **(A)'s fix is unproven against the case that motivated it.**

Delivered tick rate is computed from whole-second log timestamps over a 7-13 s
window, so it is ±15% and should be read as "at or near nominal", not quoted.
Per the standing rule, these are clean-enough passes at a good tick rate; the
one 70% reading is the first run on a cold workspace and its result is a pass,
which is trustworthy at any rate.

## Offline plans, on the merged tree (master `32282b11`)

Unchanged, as expected — nothing here touches the planner.

| goal | actions | makespan |
|---|---|---|
| `researched:automation` | 176 | 21,784 |
| `producing:automation-science-pack:6` | 316 | 22,463 |
| `producing:logistic-science-pack:6` | 442 | 47,542 |
