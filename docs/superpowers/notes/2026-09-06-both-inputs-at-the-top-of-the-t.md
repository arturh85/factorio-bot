# Both inputs join at the top of the T

2026-09-06. `OreToPlateTee`, 29 entities, seed 31337.

The owner's correction, after a screenshot showed what was wrong:

> *"that's why i told you to build T junctions to join coal and ore, both should
> join at the top of the T"*

## What I had built was not a T

`OreToPlate` ran the drills **straight onto the main belt** and sideloaded coal
into it further down. A mining drill fills whichever lane has room, so ore took
**both** lanes, and by the time coal reached its junction there was no gap to
merge into. A sideload can only enter a lane that has room.

The screenshot is what showed it: the stem packed solid with ore top to bottom,
and the coal arm backed up behind a junction it could not enter, with 55 of its
100 coal still sitting in the chest.

**Moving the junction upstream of the drills barely helped: 17 -> 18 plates.**
That is the useful negative — it proves the problem was the *topology*, not the
ordering, and I would have gone on adjusting the ordering without the picture.

## A real T

Ore arrives on **its own belt** from the west; coal on **its own belt** from the
east; both sideload into the same stem tile from opposite sides. Each claims a
lane by geometry, so neither can crowd the other, whatever arrives first.

```
                 ore arm  --->  |  <---  coal arm
                                v
                              stem (south)
                                |
                             furnaces
```

| design | plates | ore mined | coal used |
|---|---|---|---|
| sideload downstream of the drills | 17 | 46 | 45 of 100 |
| sideload upstream of the drills | 18 | 35 | 73 of 100 |
| **both inputs at the top of the T** | **78** | **86** | **89 of 100** |

**4.6x the plates**, and the coal figure is the independent confirmation: the
broken design left 55 coal unused because it could not get past the junction.

It also did not stall — the polling loop ran out of iterations rather than
detecting a plateau, with 8 ore in transit against 29 stranded before.

## A verdict that fired on a working block

The old verdict said `THE LANE` whenever `mined > plates`, so **8 ore of normal
pipeline fill read as a jam**. A belt in use always holds some. It now needs a
threshold, and says `RUNNING` below it — 8 on a 13-tile stem is what a working
belt looks like; 29 was what a jam looked like.

That is the same shape as the rest of today: a check that answers a slightly
different question than the one being asked, and reads as bad news instead of
good.

## The screenshot is a tool, and it works

Nothing reads a transport line, which has now blocked four questions. A single
on-demand screenshot answered this one immediately.

- **A headless server cannot render.** The RCON call reaches the game and
  *creates* `script-output/`, then writes nothing — the empty directory is the
  tell.
- **A graphical client can**, and the file lands in the **client's**
  `script-output`, not the server's.
- `remote.call('botbridge','screenshot', {...})` forwards straight to
  `game.take_screenshot`. The script must hold the game open, since the server
  dies when the script returns.

This is not the retired per-beat capture — that was 2,164 JPEGs for 947 MB and
rendered synchronously inside the game loop. One diagnostic frame, taken by
hand, is a different thing and is worth reaching for the next time something is
invisible.
