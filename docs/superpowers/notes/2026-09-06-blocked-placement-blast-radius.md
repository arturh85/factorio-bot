# A bot of ours on the tile: what actually failed, and what one failure cost

2026-09-06. Two defects from `run-1788663566-25023` (headless, four bots, 5x,
a 179-entity `FurnaceLine`): a placement blocked by one of our own bots, and
the ~50 placements that died with it.

**Status: no live run yet.** Everything below is the record, the code, and unit
tests. The before/after on entities-standing is empty on purpose.

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
`mods/BotBridge/control.lua` **does** have the clause — but provenance records
the *checkout's* HEAD, and `workspace/mods/BotBridge` is a symlink to whichever
worktree set up last. `.worktrees/headless` is stale at `133802b6`, whose
`control.lua` has no `describe_footprint_blockers`, no `bot_of_character` and
no `placement_step_aside_landing` at all. Bot 4 did not move once between ticks
1380 and 1920, which is what a mod that never asks looks like.

**Provenance's `git.commit` is not the mod that ran.** That is a third blind
spot alongside the two `world.dump` ones, and nothing in the record closes it.

### The gap is real anyway, and it is `stuck` — but `stepping aside` is fine

Reading the vocabulary against `FootprintWait::decide`:

| clause | busy budget? | ends? |
|---|---|---|
| `#N mining` / `#N walking` | yes | its own action |
| `#N stepping aside` | **on the second attempt** | the walk |
| `#N stuck` | no | never |
| `#N gone`, `an unclaimed character` | no | never |

`stepping aside` looks uncovered and is not: `start_walk_waypoints` sets
`storage.p[id].walking`, so the *next* attempt reads `#N walking` and draws on
the 45 s budget. The classes waiting cannot fix are `stuck`, `gone` and
`an unclaimed character`, and only `stuck` is common.

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
published `Failed` for every remaining step in its slice. That is why one belt
took ~50 undispatched placements with it.

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
reading off one; only a live block run can settle it. The executor pair is
stronger: the independent-step test and the dependent-step test share one
fixture and differ by a single `net.link`, so a loop that ignored dependencies
would pass one and fail the other.

## Offline plans, on the merged tree (master `32282b11`)

Unchanged, as expected — nothing here touches the planner.

| goal | actions | makespan |
|---|---|---|
| `researched:automation` | 176 | 21,784 |
| `producing:automation-science-pack:6` | 316 | 22,463 |
| `producing:logistic-science-pack:6` | 442 | 47,542 |
