# What two world-record replays say about our first ten minutes

2026-09-04. Two replay saves the owner dropped into `workspace/`:
`any-wr-6-39-53.zip` (Space Age any%, 6 h 39 m, map 2.0.66, seed 1856629886)
and `RSNG_3_17_06.zip` (Space Age, 3 h 17 m, 410 rockets, map 2.0.77, seed
3278808720). **Not replicated, read.** The replay stream itself is an
undocumented binary; what was read is the *final world* of each save, loaded
on a scratch headless server (own write directory, ports 34198/34199, no
BotBridge) and queried over RCON. The owner's instruction: lessons for us, not
their build.

## How the numbers were taken, and one trap

`LuaFlowStatistics.get_flow_count{..., precision_index, sample_index}`. The
precisions are **windows**, not sample sizes: `ten_minutes` is the last ten
minutes at 2 s per sample; `ten_hours` is 300 samples of 2 minutes and covers
both runs from tick 0. The first pass used `ten_minutes` and read the runs'
last minutes as if they were the first — flat curves of ~550 iron plates per
sample, obviously the late base. Everything below is `ten_hours`, 2-minute
bins, chronological.

**Rock yields are not in the production statistics.** `stone/input` and
`coal/input` read zero through minute 6 in both runs while 100+ stone furnaces
(5 stone each) and 40–70 burner drills were crafted. The stone and the coal
came from rocks, and the game does not count a rock's yield as production.
Read the crafted-item rows, not the raw-material rows, for the first minutes.

## The curves

Two-minute bins, items **crafted** (or smelted) in that bin.

**RSNG 3:17:06**

| bin | iron ore | iron plate | copper plate | stone furnace | burner drill | steam engine | lab | AM1 | red sci | green sci |
|---|---|---|---|---|---|---|---|---|---|---|
| 0–2 | 67 | 62 | 0 | 55 | 4 | | | | | |
| 2–4 | 455 | 445 | 0 | 18 | 29 | | | | | |
| 4–6 | 716 | 695 | 86 | 30 | 29 | | | | | |
| 6–8 | 689 | 699 | 330 | 3 | 9 | 1 | 1 | | 8 | |
| 8–10 | 552 | 449 | 290 | | | | | 9 | 2 | |
| 10–12 | 702 | 772 | 341 | 64 | | 7 | 3 | 4 | 27 | |
| 12–14 | 1257 | 1163 | 330 | | | | | | 30 | |
| 16–18 | 2388 | 2395 | 620 | 8 | | 30 | | | 23 | |
| 22–24 | 4186 | 3445 | 665 | 103 | | | | | 36 | |
| 30–32 | 5996 | 5985 | 2441 | | | 10 | | | 160 | 122 |

Electric drills from minute 14 at ~35 per 2 minutes; steel from minute 22;
10 red packs **consumed** by the bin ending at 12 min, so `automation` lands
at 10–12 minutes.

**any% 6:39:53** has the same shape: 109 stone furnaces and 41 burner drills
by minute 6, copper only from minute 6, steam engine + lab + first red pack in
the 6–8 bin, `automation` by 10–12 min, green science from minute 28.

**Ours** (`run-1788528493-60555`, automation at 8.28 min, seed 31337,
cumulative from `samples.jsonl`):

| min | iron ore | iron plate | copper plate | stone furnace | burner drill | red sci |
|---|---|---|---|---|---|---|
| 4 | 76 | 73 | 26 | 7 | 0 | 0 |
| 8 | 96 | 96 | 26 | 7 | 0 | 10 |
| 16 | 267 | 207 | 56 | 7 | 0 | 11 |

## The lessons, in order of size

1. **They automate mining before they research anything.** Both runs put the
   first six minutes into burner drills — 62 by minute 6 in RSNG, 41 in the
   any% — and 100+ stone furnaces, all from rock stone and rock coal. By
   minute 8 they have ~1,200 iron plates and every ore flowing by machine; we
   have 96 plates, 7 furnaces, zero drills, and four bots hand-mining. The
   previous green run spent **148,304 ticks (41 minutes) of bot time
   hand-mining** (`mine n=366` in `just analyse`). Our `PlaceDrill` cell is
   the right shape (RSNG's surviving starter base has 72 burner drills near
   spawn — 30 on coal, 24 iron, 11 copper, 7 stone — and 35 of its 98 stone
   furnaces sit within 2.5 tiles of a drill: drill-feeds-furnace pairs); the
   plan places it in ones and twos. **The lever is count, not design.**
2. **Rocks are the whole early stone and coal economy**, not a shortcut for
   a 37-coal bill. `130b3bde` was right and undersized. The runs never
   hand-mine stone or coal at all in the first ten minutes; a plan that does
   is paying 120 ticks per unit against 360 per 24+24.
3. **Automation is not rushed.** Their `automation` lands at 10–12 minutes,
   later than our 8:17, *with* 1,200 plates banked and mining automated. For a
   3 h 17 m horizon the right first metric is **iron plates per minute at
   minute 10**, not the automation timestamp. Ours is ~10/min at minute 10
   (103 plates cumulative); theirs is ~350/min.
4. **Copper is late and small.** Zero copper before minute 4–6 in both runs;
   ~330 plates per 2 minutes until minute 16. Our plan mines copper in the
   first minute. Iron first, copper only when circuits and red science need it.
5. **Science is made by machines from the start of a milestone**, never by
   hand: red at 27–40 per 2 minutes from minute 10, green at 120–160 per 2
   minutes from minute 30. Our green plan hand-crafts 75 red packs on one bot
   for 22,500 ticks while three bots stand idle. The WR's science is a rate
   from the first pack.
6. **Coal is the largest single drill target** (30 of 72). Every burner
   machine burns it, and it is what our plan runs out of first when the fuel
   for a furnace comes from a rock chopped far away.

## What this does not tell us

The replay input stream would give the exact order and the walking; the final
world does not. Nothing here says *where* they stood, how far they walked, or
how they bootstrapped the first drill from the crash-site plates. It also does
not transfer directly: one human with a mouse against four bots that cannot
place a blueprint and must walk between every action. The count lesson
transfers; the timings do not.

Servers and RCON client: `scratchpad/wr/`, `scratchpad/wr2/` (session
scratch, not kept). To repeat on a new save: `factorio --config <own
config.ini> --mod-directory <mods> --start-server <save.zip> --port P
--rcon-port R --rcon-password X --server-settings <json>`, then
`get_flow_count` with `precision_index = ten_hours`.
