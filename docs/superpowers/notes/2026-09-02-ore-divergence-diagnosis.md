# The ore divergence, diagnosed: one defect and one artefact

Answers `2026-09-02-keyframe-ore-divergence.md`, which recorded that the model
and the game disagree about ore **in both directions at the same tick** and
declined to call it. Both directions are now settled from the archive alone —
21 run directories in `workspace/runs/`, 55 keyframes — with no new run needed.

The short version:

| direction | what it is | verdict |
| --- | --- | --- |
| game has ore the model lacks (`iron-ore` short by 9) | the model deletes an ore tile on the **first mining swing**, while the game still holds hundreds of ore in it | **defect**, and it predates `e562847a` |
| model has ore the game lacks (`copper-ore` over by 10) | the model reports a row of tiles lying **wholly outside** the keyframe bounds, touching the left or top edge | **artefact** of the comparison |

Neither is retirement failing. **`e562847a` has never once executed against a
live game**, and was reported as landed and working; that is a correction to
something on the record, and it has its own section below.

## The two objections first: ruled in and ruled out

**"The two sides may not be asking the same question" — mostly ruled out, with
one named exception that is not ore.** `keyframe_relevant_types` (the `type`
filter sent to `find_entities_filtered`) and `keyframe_relevant` (the Rust-side
pass) list exactly the sixteen `EntityType`s that `EntityGraph::add` inserts
into `entity_tree`, plus `Resource`, plus the two named rocks out of
`simple-entity`. The lists agree. The evidence that they agree is in the record:
across every keyframe, **the only non-ore divergences are six
`crash-site-spaceship*` entities**, always `only_in: "game"`, always the same
six, never varying with anything the bots do. They are not a filter mismatch —
their types are ones the filter admits (that is how they reached the `game`
array at all) and `EntityGraph::add` tracks those types too. They are a
*population* gap: `writeout_entities` runs from the mod's chunk handlers, so
entities that were map-generated before the mod's handler existed never reach
the model. Furnaces the bots place match exactly, 12 against 12. That gap is
real, it is constant, and it is out of scope here; it is recorded at the end.

**"Tile centres" — ruled out.** `resource_position_from_pos` is doing its job.
Every model-side resource in every keyframe comes back on a half-tile (`-42.5`,
`12.5`), never on an integer, and every game-side resource does too. The
game-only ore tiles match the executor's `mine` targets to the *exact* float,
not to within a tile. If the half-tile were being lost anywhere, no ore position
would ever match on either side, and every ore tile in the box would be
reported as diverging instead of the handful that are.

**Evidence hygiene (`fcb4ed68`), verified rather than trusted.** `fcb4ed68`
changed only how `action_settled` is keyed — it made a `Status::Lost` action
recordable at all. This analysis reads `keyframe` lines and `action_dispatched`
lines only. Both are written at the moment they describe, neither depends on a
reply arriving, so both sides of `fcb4ed68` are usable here. No claim below
rests on an `action_settled` line or on the absence of one.

## Direction 1 — the model is missing ore the game has. A defect.

### The finding

**Every ore tile the game has and the model lacks is a tile a bot mined at.**
Not "plausibly"; exactly. Across the 37 keyframes that report any game-only
ore:

```
unexplained game-only ore tiles: 0 of 583
```

For each keyframe, take the game-only ore entities and the `action_dispatched`
`mine` targets that fell inside that keyframe's bounds at an earlier tick. The
two sets match, keyframe by keyframe, run by run, with **not one** game-only ore
tile that no bot ever mined at. The counts track step for step as a run
proceeds:

```
run-1788344167-58471   t= 9553   game-only ore  8   mines inside bounds so far  8
                       t=16116                 15                              15
                       t=19835                 16                              16
run-1788353986-24634   t=12099                 12                              12
                       t=21546                 19                              19
                       t=31329                 21                              21
```

One tile leaves the model per mine action. `mine 1 iron-ore` costs a tile
exactly as `mine 5 iron-ore` does — so this is not the model debiting an amount
to zero.

### The mechanism

`control.lua:2589`:

```lua
script.on_event(defines.events.on_player_mined_entity,
    function (event) on_mined_entity(event); on_some_entity_deleted(event) end)
```

`on_player_mined_entity` fires **per mining swing**, not on depletion. The mod's
own completion handler proves it: `on_mined_entity` does
`mining.left = mining.left - delivered` and completes the action when `left <= 0`
(`control.lua:1197-1219`). If the event only fired when the tile was destroyed,
a `mine 5` on a tile holding hundreds could never complete — and the record
shows it completing in 604 ticks, which is five swings of ~120 ticks (`mine 1`
settles in 120, `mine 2` in 241, `mine 4` in 483, `mine 5` in 604).

So every swing writes `on_some_entity_deleted`, which reaches
`FactorioWorld::on_some_entity_deleted` → `EntityGraph::remove`, whose resource
branch takes the tile out of `resource_tree` **and** out of the `resources` map
unconditionally. First swing, tile gone from the model, hundreds of ore still in
the ground.

The mod even sends the remaining amount and it is thrown away:
`serialize_entity` sets `record.amount = entity.amount` for a resource
(`types.lua:391-392`), so the payload that deletes the tile is carrying the
proof that the tile still exists.

### Correction to the record: `e562847a` has never fired in a live run

**This is a correction, not a footnote.** `e562847a` was reported to the project
owner as landed and working. It was neither — it is *committed*, its unit tests
are sound, and it has **never once executed against a live game**. Anything said
on the strength of "retirement is in place" needs re-checking against that.

It could not have fired. `player_mine_timed` calls
`resource_mined(name, position, count)` after the action settles, by which time
the delete events from that action's own swings have already removed the tile —
so `resource_mined` returns `ResourceDepletion::Absent` and there is nothing to
debit. Its own tests could not catch this and are not at fault for missing it:
they call `resource_mined` directly, which is exactly the call the live path
never reaches with a tile still in the model.

The keyframes agree. The surplus direction that would have been retirement's
signature is entirely accounted for by the bounds artefact below, and the
deficit direction is present in runs built *hours before* `e562847a` landed
(11:42):

```
run-1788317597-64759  04:53   game-only ore  5   all 5 mined
run-1788319014-01846  05:16                 23   all 23 mined
run-1788323755-24892  06:35                 25   all 25 mined
```

The original note's worry — "the fix is on the verified pile and the evidence
does not obviously agree" — was right to be raised and wrong in its guess. The
fix is not broken. It is unreachable, because something older removes the tile
first. It becomes reachable for the first time with the change below, which is
the point at which `e562847a` starts to deserve the description it was already
given.

### The fix

`EntityGraph::remove` now treats a resource payload that carries a **positive
`amount`** as an amount report rather than a removal, and leaves the tile
standing. `on_resource_depleted` (routed to the same writeout, with the entity
emptied) and `retire_resource` (which builds its entity with no amount at all)
both still delete, so the two paths that mean "this tile is really gone" are
unaffected — and `e562847a`'s arithmetic becomes reachable for the first time.

The reported amount is deliberately **not** written through to the stored one.
`resource_mined` is the debit authority; applying both the game's reading and
the model's subtraction would take the same ore out twice and could retire a
tile that still holds a few units — the same class of error in a smaller
costume.

## Direction 2 — the model reports ore the game does not. An artefact.

### The fingerprint

**Learn this signature; it identifies the cause without redoing the analysis.**

```
model-only ore tiles, by the bound they abut:

    left    443
    top     248
    right     0
    bottom    0
    interior  0
                     total 691 of 691, across 30 keyframes
```

Left and top leak, right and bottom do not, and nothing is ever interior. **A
half-open containment test — `min <= p < max` — produces exactly that, and
nothing else does.** Inclusive-on-all-sides would leak all four bounds about
equally. A rounding or half-tile error would scatter and would hit interior
tiles too. A population difference between the two sides would not respect the
bounds at all. Two edges leaking while their mirror images do not is the
fingerprint of a containment predicate closed at its minimum corner and open at
its maximum one.

There has never been a model-only divergence that was not ore:

```
run-1788344167-58471  t=19835  bounds left=-42.0
    model-only: copper-ore at x=-42.5, y=12.5 .. 28.5   (17 tiles, one column)
```

A tile centred at `-42.5` occupies `[-43, -42]`. The bounds start at `-42`. The
tile is outside and merely abuts the boundary line. The game does not return it,
correctly. The model does.

### Where that predicate is

`crates/core/src/aabb_quadtree.rs`:

```rust
fn my_intersects<S>(a: Rect<S>, b: Rect<S>) -> bool {
    a.intersects(&b) || a.contains(b.origin) || a.contains(b.max())
}
```

`euclid`'s `Rect::contains` is half-open — `min <= p < max`. So the query
rectangle **includes** a box whose maximum corner lands exactly on the query's
left or top edge, and **excludes** one whose minimum corner lands on the right
or bottom edge — the fingerprint above, derived rather than fitted. It is not a
resource-specific bug and it is not `resource_position_from_pos`; it is the
query being inclusive at two of its four edges.

The unit test reproduces the fingerprint in miniature. With the clip reverted,
`snapshot_within_excludes_a_tile_that_only_abuts_the_bounds` reports the tile
outside the left edge and the tile outside the top edge, and *not* the ones
outside the right and bottom edges:

```
  left: [Position { x: -42.5, y: 12.5 }, Position { x: 12.5, y: -2.5 },
         Position { x: -41.5, y: 12.5 }]
 right: [Position { x: -41.5, y: 12.5 }]
```

This is an artefact of the comparison, and a diagnostic that cries wolf is worse
than none — this project spent today being trained past a permanently-true STALE
warning. So `snapshot_within` now drops anything whose box does not genuinely
overlap `bounds`. `snapshot_within` has exactly one caller, the keyframe
(`crates/scripting_lua/src/globals/record.rs:177`), so the clip changes the
diagnostic and nothing the planner reads.

## What is left over, and deliberately not fixed here

**The six `crash-site-spaceship*` entities.** Present in every keyframe whose
bounds reach the spawn area, always game-only, never varying. The model has
never heard of them: the mod writes a chunk's entities out from its chunk
handlers, and the starting area was generated before those handlers existed, so
no map-generated entity in the starting chunks ever reaches `EntityGraph`. This
is a real blind spot — it means the model does not know about pre-existing
containers, and a bot could be planned into one — but the fix is a mod-side
initial sweep, in files owned elsewhere. It is a constant six-entity offset in
every keyframe until then.

**Runs before the `add` deduplication.** `run-1788307982-79011` and
`run-1788319014-01846` show the model holding *twice* the game's ore
(`iron-ore 417/900`). That is the duplicate-insert defect the `already_known`
guard in `add` was written for, visible in the archive and absent from every run
after it. Recorded so a later reader does not mistake those two runs for the
same phenomenon as this note's.

## Verification

Both guards were reverted by hand and the tests re-run, to check that they fail
for the reason claimed rather than passing by luck. `cargo test -p
factorio-bot-core --lib graph::entity_graph`:

```
snapshot_within_excludes_a_tile_that_only_abuts_the_bounds ... FAILED
a_mined_resource_that_still_holds_ore_is_not_removed ... FAILED

---- a_mined_resource_that_still_holds_ore_is_not_removed ----
a tile the game still holds ore in must stay in the model

---- snapshot_within_excludes_a_tile_that_only_abuts_the_bounds ----
assertion `left == right` failed: only the tile actually inside the bounds is
inside the bounds
  left: [Position { x: -42.5, y: 12.5 }, Position { x: 12.5, y: -2.5 },
         Position { x: -41.5, y: 12.5 }]
 right: [Position { x: -41.5, y: 12.5 }]
```

**The third test, `an_emptied_resource_is_still_removed`, passes with the guard
reverted, and that is what it is for.** It is the negative control: before the
guard existed, removal was unconditional, so it could not possibly have gone
red. Saying "all three fail" would have been the comfortable claim and the false
one. What it actually pins was checked separately, by mutating the guard to the
over-broad form a careless fix would produce —

```rust
if entity.entity_type == EntityType::Resource.to_string() {
    return Ok(());   // no amount check at all
}
```

— against which it fails, along with four existing tests that would otherwise
have been the only thing standing between this fix and a model that never
retires a tile at all:

```
an_emptied_resource_is_still_removed ... FAILED
mining_a_tile_dry_retires_it ... FAILED
a_vanished_target_is_retired_whatever_the_model_believed ... FAILED
removing_a_resource_delivered_twice_empties_the_tile ... FAILED
retiring_a_tile_leaves_its_neighbour_alone ... FAILED
```

With both guards restored the file is byte-identical to the committed version
(`git diff` is empty), and the suite is green: 30 passed, 0 failed.
