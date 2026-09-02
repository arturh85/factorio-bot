# A walk whose destination is inside a building is refused, not walked — 2026-09-02

**Follows:** `docs/superpowers/notes/2026-09-02-rung-7-unreachable.md`, which
established the finding from run 30's record and deliberately did not land the
fix. This landed it.
**Changed:** `crates/core/src/factorio/rcon.rs` only. No new query on
`EntityGraph`, no change to the mod, no change to the planner.

## What was wrong

`move_player_timed` judged a returned path by exactly one question —
`walk_arrives(goal, radius, end)`, the distance from the **caller's** goal —
and never asked whether a character could stand on the endpoint. The last
waypoint is the walk's destination *inside the mod*: its follower steers at that
point until it arrives or the leg times out.

Run 30 returned 12 paths (of 75) with a waypoint strictly inside a stone furnace
that same run had built, every one flagged `needs_destroy_to_reach: false`. All
three of the run's failed walks are exactly the ones whose **last** waypoint was
one of those:

| walk | tick | destination | furnace | placed |
| --- | --- | --- | --- | --- |
| 72 | 81661 | `(-22.30078125, 18.22265625)` | `(-22, 18)` | 33342 |
| 96 | 89002 | `(-22.2890625, 23.3359375)` | `(-22, 24)` | 87493 |
| 179 | 165964 | `(-19.5, 19.5)` | `(-19, 20)` | 161648 |

Each cost four re-paths and a leg timeout, and then reported the *terrain* as
unreachable — while the re-path could not have succeeded either:
`WALK_REPATH_RADIUS` is `0.5` and standing clear of a stone furnace needs
`0.69921875 + 0.19921875 = 0.8984375`. Unsatisfiable by arithmetic, before the
pathfinder ran. The 9 stall episodes whose destination was *not* inside a
furnace all recovered on the first re-path; the machinery was never the problem.

## What it does now

`judge_path` — a new free function holding everything `move_player_timed`
decides before it dispatches — asks two questions in this order:

1. **Does the path reach the caller's goal?** Unchanged, still
   `RconWalkFallsShort`, still first. A walk that does not arrive is refused for
   not arriving, whatever else is true of its endpoint.
2. **Can a character stand where it ends?** New. A provable no is
   `RconWalkEndsWhereNobodyCanStand`, `Dispatch::NotDispatched`,
   `ActionTicks::UNKNOWN` — the walk genuinely did not happen, so the executor
   renders it `Failed` rather than `Lost`, exactly as the arrival refusal does.

For run 30's walk 72 the message is:

```
the walk to [-23.5, 18.5] would end at [-22.30078125, 18.22265625],
inside stone-furnace at [-22, 18] — a character cannot stand there,
so the walk could only stall
```

instead of a 280-tick stall blaming the pathfinder. It names the obstruction,
where the obstruction is, and where the walk would have ended.

## The part that had to be got right: blocked vs. cannot tell

`EntityGraph::blocking_boxes_within` is an **in-bounds** oracle. It answers
about entities and player-collidable tiles the graph has been told about, so an
empty answer covers two different situations — "the ground is clear" and "the
graph has never seen this ground" — and nothing here can tell them apart. This
guard sits on the path of **every** walk, so a false positive is worse than the
stall it prevents.

`StandingVerdict` therefore has two variants, not three, and the asymmetry is
the point:

- `Blocked { blocker }` — a box the graph has actually seen overlaps the
  character's footprint. Only this may refuse.
- `NotProvablyBlocked` — everything else, *including* every case where the real
  answer is "I cannot tell". Always walked.

That is the same distinction `PlacementVerdict::is_durable_refusal` makes about
the game's build refusals, and the same one the mod makes between a character
standing in a footprint and a verdict about the ground: an observation that
something *is* there is a fact; the absence of an observation is not.

Three more places the same rule is applied rather than a guess:

- **Touching is not colliding.** `boxes_overlap` is strict on all four
  comparisons, so a character whose box abuts a furnace's exactly — at
  `0.8984375`, the standing-clear distance — is walked. One 1/256th nearer is
  a real overlap and is refused.
- **An unknown character extent narrows the question instead of inventing a
  number.** The footprint comes from the world's own `character` prototype;
  with no such prototype it collapses to the single point, so the question
  becomes "is this exact point strictly inside a building" — strictly weaker,
  still catches all three run-30 destinations, and never refuses on a made-up
  half-extent.
- **An empty path is not judged.** The arrival check falls back to the bot's
  current position when there are no waypoints (the mod completes such a walk
  next tick without moving); the standing check reads `waypoints.last()` only.
  Refusing a walk because the bot is *already* standing somewhere the graph
  calls blocked would be a refusal about the past.

A blocker that cannot be *named* is still refused: `blocking_boxes_within`
returns bare rectangles, and the name comes from the entity tree, which holds
only the types that tree tracks. A tree, a rock or a water tile blocks and is
reported as its box. That naming query is deliberately unfiltered by name and
type — narrowing it is what reintroduced the forest-siting bug `1b2b2149` was
careful to leave alone.

## Evidence

**Red first.** With the second check removed from `judge_path` and nothing else
changed, the three run-30 walks fail and only those three:

```
run_30_walk_72_is_refused_before_dispatch_and_names_the_furnace ... FAILED
run_30_walk_96_is_refused_before_dispatch ... FAILED
run_30_walk_179_is_refused_even_though_it_lands_where_it_was_asked_to ... FAILED
test result: FAILED. 7 passed; 3 failed
```

The seven that pass in that state are the ones pinning what must **not** be
refused — they hold in both worlds, which is what they are for.

Each run-30 test also asserts `walk_arrives` accepts its path first, so the
tests pin that the new check catches what the old one could not, rather than
duplicating it. The collision boxes come from `tests/entity-prototype-fixtures.json`,
which carries the game's real `0.19921875` character and `0.69921875` stone
furnace half-extents — the arithmetic in the tests is the arithmetic the run
did.

**Mutation.** Six mutations, each caught, five by exactly the one test that
owns them:

| mutation | fails |
| --- | --- |
| `NotProvablyBlocked` becomes a refusal | the four "must not refuse" tests, and none of the run-30 ones |
| `boxes_overlap` admits touching (`<` → `<=`) | the 0.8984375 boundary test |
| unknown character extent guessed as 0.3984375 | the no-prototype test |
| standing check reads the fallback position too | the empty-path test |
| standing check preempts the arrival check | the falls-short test |
| unnameable blocker reported without its box | the tree test |

The first is the regression that would matter, and it fails four tests without
touching the three that motivated the change — which is the shape it should
have.

## What this does not do

- **It does not unblock rung 7.** That is the research plan and power, handled
  elsewhere; see the previous note's section 1.
- **It does not explain why Factorio returned those paths.** The
  `pathfind_flags.cache = false` experiment named in the previous note is still
  the way to settle that, and is still the mod's to make. If the cache turns out
  to be the cause, this guard should fire approximately never — which is the
  outcome to hope for, not a reason to leave it out.
- **It has no live run behind it yet.** Every test here is a unit test against
  archived coordinates. What a run would add is the one thing no test can: a
  count of how often `RconWalkEndsWhereNobodyCanStand` fires, and whether any of
  those were walks that would have succeeded. That number is worth reading on
  the next run.
