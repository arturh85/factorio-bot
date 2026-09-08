# A worm shoots a passer-by, and a peaceful one shoots nobody

2026-09-08. Branch `a-threat-is-not-only-at-the-target`, off `144603b2`.

The owner asked: *"seems like we need to take threats to avoid into account for
all actions?"* The 2026-09-07 guard covers **where a bot is sent to work** —
`threat_covering` is read in `error.rs`, `blueprint.rs`, `have.rs`, `scout.rs`,
`util.rs` and `state.rs`, and appears nowhere in `action.rs`,
`graph/route.rs`, `connect.rs`, `power.rs` or `assemble.rs`.

Before writing any of it, one question had to be answered, because the whole
design turns on it and nobody knew: **does a worm shoot a bot that is merely
passing through, or only one that lingers?** If passing is safe, walk routing
needs nothing and only placement matters — a much smaller change.

## The measurement

`scripts/threat_pass_probe.sh`, against a live headless seed-31337 server with
a `small-worm-turret` cheated in on open ground 200 tiles from spawn. A
character is teleported in (stationary trials) or walked past at its own
0.15 tiles/tick (transient trials); the lowest health seen is reported, not the
final one, because a bot that is shot and survives has still been shot. Every
window is bounded in **game ticks**. Character max health 250; the worm's
`attack_parameters.range`, read off the running game, is **25**.

### Hostile

```
-- stationary, 300 ticks --        -- transient pass, 120 tiles --
   dist   hp_min   damage             offset   hp_min   damage
     40    250.0      0.0                 40    250.0      0.0
     34    250.0      0.0                 34    250.0      0.0
     30    250.0      0.0                 30    250.0      0.0
     26    250.0      0.0                 26    250.0      0.0
     24     96.6    153.4                 24    250.0      0.0
     20    138.4    111.6                 20    210.4     39.6
     15    105.1    144.9                 15    174.4     75.6
     10    192.4     57.6                 10    170.8     79.2
      5    111.4    138.6                  5    210.4     39.6
                                            0    134.8    115.2
```

**A passer-by IS shot.** The owner's premise stands and the smaller answer is
not available: from 20 tiles inward a bot walking straight past loses 40–115 of
250 health without stopping. So walk routing genuinely is exposed.

**But standing is much worse, and the boundary moves.** Stationary damage
switches on exactly at the 25-tile attack range (nothing at 26, 153 at 24).
Transient damage switches on at **20**: at offset 24 the bot is inside the
25-tile radius for only `2*sqrt(25^2-24^2)/0.15 ~ 47 ticks`, less than the worm
needs to rear up and fire, and it takes **nothing**. At offset 20 the exposed
span is 30 tiles, ~200 ticks, and it is hit.

That asymmetry is the design input: **a building cannot walk on.** Standing
exposure is unbounded in time and is what this change guards.

### Peaceful

Same probe, same seed, `--peaceful`:

```
stationary 40..5 tiles:  damage 0.0 at every distance
transient  40..0 tiles:  damage 0.0 at every offset
provoked (worm struck for 50 by the player force, bot at 10 tiles): damage 0.0
```

**A peaceful worm does not shoot at all**, including one that has just been
damaged.

**Proved by a control, because "nothing was shot" is what a broken arena also
says.** In the same game, with the same worm, one variable flipped:

```
game.surfaces[1].peaceful_mode = false   ->  stationary at 10 tiles: 173.7 damage
game.surfaces[1].peaceful_mode = true    ->  stationary at 10 tiles:   0.0 damage
```

So peaceful mode is what stops it, not a missing worm. Since peaceful is now
the default for oil runs, **the threat guard costs those runs plans for
nothing** — worth knowing before anybody reads a threat refusal on a peaceful
run as a real hazard. It is *not* a reason to switch the guard off: nothing in
the planner reads `peaceful` today, and a run that flips to hostile mid-way (a
resumed savepoint, a settings change) would have no guard at all.

The earlier census surprise — same seed, both modes, identical enemy counts
(300 enemy, 169 spawners, 131 worms) and 1,408 enemy entities walking around a
peaceful world — is consistent with this: peaceful stops the shooting, not the
spawning.

## What was guarded

Two seams, both **prefer-then-fall-back**, so the guard can move a thing and
can never delete a plan:

- **`method::util::free_area_near_where`** — the one search every siting caller
  goes through (`power`, `assemble`, `fabricate`, `have`, `sustain`, `pipe`).
  Its rings are now walked twice: once skipping candidates inside a charted
  standoff, then, only if that found nothing at all, again exactly as before.
- **`method::connect::connect_steps_with`** — a belt is a standing structure,
  so the route is tried once on a grid with threatened cells marked blocked and
  falls back to the plain grid on a refusal. The threat list is taken **once**
  per window rather than per cell, because `EntityGraph::threats_from` clones
  and sorts the whole table on every call and 2,304 of those per belt is the
  shape that doubled the oil goal's planning time the day before.

## What was deliberately NOT guarded, and why

- **Walk routing.** The planner does not route walks: it emits a destination
  and the game's own pathfinder chooses the path. There is no planner-side
  route to steer, and steering one means waypointing through
  `crates/executor`, which is outside this branch. This is the owner's premise
  that survived measurement, and it is the open item — see below.
- **`crates/planner/src/action.rs`.** Actions carry positions but choose none;
  every position in them was chosen by a method. A guard there would be a
  second, later opinion about a decision already made, which is exactly the
  "two encodings of one fact" this repo keeps paying for.
- **`crates/core/src/graph/route.rs`.** Left pure. It takes a `blocked` grid
  and knows nothing about threats, which is why the threat overlay is built by
  its caller.

## The baselines, one binary, `144603b2` + this branch, debug

| goal | world | before | after |
|---|---|---|---|
| `researched:automation` | `map.json` | 176 / 21,784 | 176 / 21,784 |
| `producing:automation-science-pack:6` | `map.json` | 316 / 22,457 | 316 / 22,457 |
| `producing:logistic-science-pack:6` | `map.json` | 441 / 47,478 | 441 / 47,478 |
| `gathered:crude-oil` | `map-31337-explored.json` | 2,117 / **314,345** | 2,117 / **309,574** |

The three `map.json` goals cannot move and their being identical proves
nothing: that dump's `threats` is literally `{}`.

### The oil move, attributed

Controlled the way the target-side guard was: the siting pass was disabled **in
place** (`for avoid_threats in [false]`), rebuilt, and re-run —
**314,345 exactly.** So the −4,771 ticks (−1.5%, action count unchanged) is the
siting guard and nothing else.

The mechanism, from instrumenting the skip:

```
THREATSKIP stone-furnace [-346, -66] by spitter-spawner at [-298.5, -74.5] d=48.3 standoff=50.0
```

**Exactly one candidate, in the whole 2,117-action plan, is rejected.** Five
placements then shift by one to four tiles — two `burner-mining-drill`, two
`stone-furnace`, one `wooden-chest`, all in the same cell — because they are
sited relative to each other, and the shorter makespan falls out of that.
**The 1.5% is incidental, not a benefit of avoiding threats**; a cascade could
as easily have gone the other way.

### Planning cost

One extra `PlanState::threat_covering` per candidate that has already passed
every other test — which is normally the first one, because the search returns
on it. Over the entire oil plan the second pass was needed **three times**.
That is the honest instrument: a count, not a wall clock. (For what it is
worth the oil plan's wall time went 234 s -> 207 s across the two runs, but two
other cargo processes were on the box and no wall-clock claim is made.)

## The gap this opened, and it is a real one

Counting how many placements in each plan sit inside a charted standoff:

```
guard off:  1   stone-furnace       at (-346, -66)   48.3 tiles from a spitter-spawner
guard on:   2   stone-furnace       at (-346, -66)   48.3
                burner-mining-drill at (-346, -64)   48.6
```

**Exposure went UP, from one entity to two.** The guarded search refused that
furnace site three times and a furnace stands there anyway, because
**`method::produce::fit` sites a cell's drill and its furnace from fixed
offsets and never calls `free_area_near_where`** — it tests
`covers_resource`, `covers_claimed_resource`, `cell_yield` and
`is_area_free_facing`, and nothing about threats. The cascade then moved a
drill in beside it.

So: the guard is correct where it fires and is **bypassed by the method that
places the most entities on a threatened map**. `produce.rs` is outside this
branch's boundary, so this is reported rather than fixed. It is the first thing
to do next, and it is one call site.

Two smaller open items:

- **A footprint is tested at its centre only.** `threatened_tile` asks about
  one position; against a 25-to-50-tile standoff a 3-tile machine is noise, but
  it is an approximation and is written down rather than assumed.
- **The standoff table still answers 100% of calls.** `attack_range` is
  readable from the running game — this probe read `25` for
  `small-worm-turret` in one RCON call — and `mods/BotBridge` still does not
  send it, so `WORM_ATTACK_RANGE`'s hard-coded numbers remain load-bearing.

## Reproducing

```bash
# terminal 1 -- hostile, then repeat with --peaceful
factorio-bot lua hold_long.lua --headless --bots 1 --seed 31337 --new \
    --game-speed 1 --settings workspace/headless-e.toml

# terminal 2
scripts/threat_pass_probe.sh workspace/headless-e.toml
```

The apparatus is committed on purpose. An experiment whose apparatus lives only
in a worktree dies with it, and that has already cost this project three
unverifiable results.
