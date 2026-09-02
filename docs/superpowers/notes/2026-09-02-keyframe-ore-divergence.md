# The model and the game disagree about ore, in both directions

An observation from `run-1788344167-58471` (run 23), **not** a diagnosis. Recorded
because it bears on a fix I have been describing as landed.

## What the keyframes say

`map.jsonl`, `kind: keyframe`, comparing the game's entities against
`EntityGraph`'s within the same bounds:

```
tick   4507   game   2   model   0   divergence  2
tick   9553   game 486   model 478   divergence  8
tick  16116   game 828   model 824   divergence 38
tick  19835   game 829   model 824   divergence 39
```

Furnaces match exactly — 12 in the game, 12 in the model, at the final keyframe.
The world model is **not** blind to what the bots build, which rules out the
first thing I assumed when a bot walked into a furnace it had placed.

Ore is where they disagree, and it disagrees **both ways** at the same tick:

```
tick 19835   copper-ore   game 329   model 339     model has 10 MORE
             iron-ore     game 482   model 473     model has  9 FEWER
```

## Why this matters

This run was built at `c5c937d5`, which includes `e562847a` — "retire an ore tile
the moment a mine empties it". If retirement were working, the model should not
be holding **more** copper-ore tiles than the game has. Ten surplus tiles is
exactly the shape of the bug that fix was for: tiles mined to nothing that the
model still offers to the planner.

I have been describing that fix as landed. It is committed and its tests are
green, but this is the first live evidence about it and the evidence does not
obviously agree. That is worth saying plainly rather than leaving the fix on the
"verified" pile.

## What would have to be checked before calling it a defect

Two things could make this reading wrong, and neither has been ruled out:

1. **The two sides may not be asking the same question.** `B1` filtered the
   keyframe's game-side query to the 16 types `EntityGraph` models. If the filter
   and the model's own membership rule differ even slightly, the counts differ
   for a reason that has nothing to do with retirement.
2. **The iron-ore direction is unexplained by retirement at all.** Retirement can
   only ever remove tiles from the model, so it cannot produce a model holding
   *fewer* iron-ore than the game. Something else is also going on, and until
   that is understood, attributing the copper surplus to retirement is a guess
   that happens to fit one of the two numbers.

The honest summary is that the model and the game disagree about ore in two
directions at once, that only one direction is consistent with retirement
failing, and that the `divergence` array in each keyframe already names the
specific entities — so this is answerable from the record without another run.
