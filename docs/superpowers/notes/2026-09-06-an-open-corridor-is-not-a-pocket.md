# An open corridor is not a pocket: a belt was being read as a wall

2026-09-06. **Reproduced**, and it is not the search window.

## The report

A live `TwoRowSmelter` run logged, fourteen times, for a bot that never moved:

```
pre-place check: the character is already walled in here; this placement does
not change that and is allowed  player=1 from=[0.5, 0.5] pocket_tiles=1.0
```

One reachable tile for a bot in a three-tile corridor open at both ends. 13 of
29 steps were never dispatched.

## The hypothesis that was wrong, and the one that was right

**Refuted: the fill is not clipped by its own window.** The window is a fixed
48x48-tile square (`SEARCH_RADIUS = 24`, `crates/core/src/graph/enclosure.rs`),
and reaching its edge is precisely what `Escape::Open` means — a corridor open
at both ends would reach the edge in 24 tiles and answer `Open`. `26498dee`
resized the *bot-selection* radius in `crates/planner/src/enclosure.rs`, not
the fill window, and is unrelated in either direction.

**Confirmed: `blocked_tree` is a buildability index and a belt is in it.**
`EntityGraph::add` files every entity with a collision box except resources and
rails, which is right for "does a furnace fit here" and wrong for "can a
character walk here". Read off the live 2.1.17 capture
(`crates/core/tests/live-2.1.17-world-snapshot.json`):

| prototype | collision mask |
|---|---|
| `transport-belt`, `underground-belt`, `splitter`, `loader`, `linked-belt` | `water_tile, floor, transport_belt, object, meltable` |
| `stone-furnace`, `iron-chest`, `burner-inserter`, `pipe` | `… object, **player**, meltable` |

No player layer on any belt. A character walks over them; the fill did not
know that, so **a belt row across a corridor sealed it**.

## The number is exactly the reported one

Decoded from the blueprint in `scripts/rcontest.lua`, `TwoRowSmelter` is
`stone-furnace` (2x2) rows at `y = -2` and `y = 3`, `burner-inserter` rows at
`y = -0.5` and `y = 1.5`, and nine `transport-belt` at `y = 0.5`. Grown by the
character's own half-box the furnaces block rows `-2.5/-1.5` and `2.5/3.5`,
leaving the three-tile corridor.

A bot on the belt row **between an input and an output inserter** — at
`[7.5, 0.5]` in blueprint coordinates — has an inserter north, an inserter
south and a belt either side. With a belt as a wall that is a pocket of exactly
one tile: `pocket_tiles=1.0`, the live reading, reproduced offline in
`crates/core/tests/enclosure_two_row_smelter.rs`.

The live log said `[0.5, 0.5]` because that block was **sited by search** from
the roster's centroid, so its world anchor is not the blueprint origin. The
shape reproduces; the coordinate does not, and does not need to.

## Why a false `Enclosed` is worse than a wrong number

`crates/executor/src/pre_place.rs` matches `(Escape::Enclosed, _)` and allows
the placement — correctly, since a bot already sealed in is not this
placement's doing. So **one false `Enclosed` makes every later placement near
that bot read as informed consent**, and each line looks harmless alone.

The same false reading also reached the planner: `PlanState::find_walled_in`
excludes a walled-in bot from gathering shares and from being a chain actor.
It requires the game's own path refusal as a second witness, so the belt bug
alone could not bench a bot — but on a bot the game had refused, it would have.

## The fix, and the shape it keeps

`graph::enclosure::blocks_character(prototypes, name)` reads the prototype's
own mask, in **both spellings** (`player` from a live 2.0 game, `player-layer`
from this repo's 1.x fixture — matching one would make it true in tests and
false in a run). A prototype with no mask, and a name with no prototype, both
block: an unknown entity is not something to wave a character through.

Three call sites:

* `grid_for` (core) subtracts the walkable boxes from `blocking_boxes_within`.
  The blocked tree's payload is a bare `is_minable` flag with no name in it, so
  the boxes come from the entity tree, matched as a multiset on Factorio's
  1/256 position grid (`drop_walkable`). Anything in `blocked_tree` the entity
  tree does not hold — a tree, a cliff, a unit, water — is untouched and keeps
  blocking. That is the conservative direction: the failure being fixed is a
  *false* enclosure.
* `PlanState::walkable_obstacles_within` (planner) does the same, filtering the
  plan's own `added` entities and the base world's by name and subtracting from
  the anonymous rectangles. Prevention and detection reading different walls is
  the disagreement `crates/planner/src/enclosure.rs`'s module doc exists to
  forbid.
* `judge_placement` (executor) declines by name for a placement a character
  does not collide with — it can seal nobody in, and standing on its tile is
  not a refusal, so a belt no longer costs a pointless step-aside walk.

## On the latch

The peer's framing — "once a bot reads as enclosed the guard is off for the
rest of the run" — is right about the consequence and **there is no latch in
the code to remove**. `escape_from` is recomputed from the live world on every
placement, and the planner's ledger already re-asks (`find_walled_in`'s "second
witness … re-asking is what un-excludes a bot without requiring it to move
first"). The persistence was the world's, not a stored flag: a genuinely sealed
bot stays sealed. So the false input was the whole bug, and nothing here needs
to become re-evaluable.

The one thing still worth doing, deliberately not done here: the
`already walled in` branch only `info!`s. It is an independent witness to a
condition the run record names elsewhere, and "silence is not success" applies
— it belongs in the ledger beside `walk_memory::note_enclosure`'s rows, not
only in a log line.

## Verification

* Five falsifications, one break at a time, each matching its predicted count
  exactly: removing `drop_walkable` (2 core tests fail, both
  `Enclosed { pocket_tiles: 1.0 }`); dropping the `player-layer` spelling (2
  fail); forcing the planner's `added` branch to block (1 fails, `Evacuate …
  pocket_tiles: 1.0`); forcing its base-world branch to block (1 fails,
  `Clear` — the guard silently off, the exact shape of the defect); removing
  the executor gate (1 fails, a step-aside for a belt).
* Every test above has a **control on the same geometry**: swap the nine belts
  for nine `iron-chest` and the same fill finds the same one-tile pocket. So
  the passes are about walkability, not about the fill going blind.
* The three offline baselines are **byte-identical** on
  `map-31337-t0`, same binary, `--bots 1,2,3,4`:
  `researched:automation` 176 / 21,784; `producing:automation-science-pack:6`
  316 / 22,463; `producing:logistic-science-pack:6` 442 / 47,542.
* `cargo test --workspace` green (97 `test result: ok`); clippy
  `--workspace --all-features --all-targets --deny warnings` clean.

## What this does not claim

Nothing here has been run live. The offline reproduction is of the *model*'s
answer for the block's own geometry; whether it clears the 13 undispatched
steps is unknown, and the peer's own account attributes that stall to a walk
resolving inside a furnace's collision box, which is a different defect and
still open.
